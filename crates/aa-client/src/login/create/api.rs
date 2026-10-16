//! The native side of the character-create scripts: the `CC` table of Rust functions that
//! `game_create.lua` builds `X2LoginCharacter`, `X2Customizer` and the customizing unit from.
//!
//! The scripts and the Bevy systems share one [`Shared`] (behind a mutex): the natives read and
//! change the look, and queue [`Request`]s (leave a step, create the character, ...) that the
//! systems of [`super`] carry out.

use std::sync::{Arc, Mutex, MutexGuard};

use aa_anim::{Gender, Race};
use mlua::{Lua, Table};

use super::look::{Customizer, Style};
use super::saved::saved_looks;

// `CDR_*` of the client's custom data natives (the values the constants of aa-ui carry).
const CDR_LOAD_SUCCESS: u32 = 1;
const CDR_SAVE_SUCCESS: u32 = 2;
const CDR_DELETE_SUCCESS: u32 = 3;
const CDR_ERROR_AS_NO_PATH: u32 = 4;
const CDR_ERROR_AS_NO_FILE: u32 = 5;
const CDR_ERROR_AS_SAME_FILE: u32 = 6;
const CDR_ERROR_AS_INVALID_DATA: u32 = 7;

/// What the scripts ask the screen to do (answered by the systems of the plugin).
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// `EndCharacterCreate / Customize / Ability(forward)` from step `from`.
    Step { from: u8, forward: bool },
    /// `GotoCharacterSelect()`
    GotoSelect,
    /// `ShowSkillAction(ability)`
    SkillAction(u32),
    /// `NewCharacterCBT2`: create the character.
    Create { name: String, ability: u32 },
}

pub struct Shared {
    pub cz: Customizer,
    /// 1 race, 2 appearance, 3 skillset and name.
    pub step: u8,
    pub requests: Vec<Request>,
    /// `Characters::race_congestion` (index = race id).
    pub race_congestion: [u8; 9],
    /// Camera zoom of step 2 (mouse wheel), 0 = default.
    pub zoom: f32,
    /// The skillset picked in step 3 (ability id, 0 = none).
    pub ability: u32,
    pub create_pending: bool,
    pub locale: String,
}

pub type SharedRef = Arc<Mutex<Shared>>;

pub fn lock(s: &SharedRef) -> MutexGuard<'_, Shared> {
    s.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn race_from(name: &str) -> Race {
    match name {
        "elf" => Race::Elf,
        "ferre" => Race::Ferre,
        "hariharan" => Race::Hariharan,
        _ => Race::Nuian,
    }
}

pub fn gender_from(name: &str) -> Gender {
    if name == "female" { Gender::Female } else { Gender::Male }
}

pub fn race_name(r: Race) -> &'static str {
    r.folder()
}

pub fn gender_name(g: Gender) -> &'static str {
    g.folder()
}

fn style(name: &str) -> mlua::Result<Style> {
    Style::parse(name).ok_or_else(|| mlua::Error::runtime(format!("unknown style {name}")))
}

/// Registers the global `CC` table.
pub fn install(lua: &Lua, shared: &SharedRef) -> mlua::Result<()> {
    let cc: Table = lua.create_table()?;
    macro_rules! def {
        ($name:literal, |$lua:tt, $s:ident, $args:tt : $ty:ty| $body:expr) => {{
            let shared_ref = shared.clone();
            cc.raw_set(
                $name,
                lua.create_function(move |$lua, $args: $ty| {
                    let $s = &shared_ref;
                    $body
                })?,
            )?;
        }};
    }

    // ---- race / gender of the preview ------------------------------------------------------
    def!("show_preview", |_, s, (race, gender): (String, String)| {
        let mut g = lock(s);
        if let Err(e) = g.cz.select(race_from(&race), gender_from(&gender)) {
            bevy::log::error!("character create: {e:#}");
        }
        g.zoom = 0.0;
        Ok(())
    });
    def!("edit_race", |_, s, _: ()| Ok(race_name(lock(s).cz.cat.race).to_string()));
    def!("edit_gender", |_, s, _: ()| Ok(gender_name(lock(s).cz.cat.gender).to_string()));
    def!("remove_unit", |_, s, _: ()| {
        lock(s).cz.reset();
        Ok(())
    });
    def!("race_congestions", |lua, s, _: ()| {
        let t = lua.create_table()?;
        let g = lock(s);
        for race in 1..=8usize {
            t.raw_set(race, g.race_congestion.get(race).copied().unwrap_or(0))?;
        }
        Ok(t)
    });

    // ---- styles ---------------------------------------------------------------------------
    def!("count", |_, s, key: String| Ok(lock(s).cz.count(style(&key)?)));
    def!("get", |_, s, key: String| {
        let (i, w) = lock(s).cz.get(style(&key)?);
        Ok((i, w))
    });
    def!("set", |_, s, (key, index, weight): (String, Option<f64>, Option<f64>)| {
        let index = index.unwrap_or(0.0).max(0.0) as usize;
        lock(s).cz.set(style(&key)?, index, weight.map(|w| w as f32));
        Ok(())
    });
    def!("get_rgb", |_, s, key: String| {
        let c = lock(s).cz.color(style(&key)?);
        Ok((c[0], c[1], c[2]))
    });
    def!("set_rgb", |_, s, (key, r, g, b): (String, f64, f64, f64)| {
        lock(s).cz.set_color(style(&key)?, [r as f32, g as f32, b as f32]);
        Ok(())
    });
    def!("set_scar", |_, s, (i, x, y, scale, rotate): (Option<f64>, Option<f64>, Option<f64>, Option<f64>, Option<f64>)| {
        lock(s).cz.set_scar(
            i.unwrap_or(0.0).max(0.0) as usize,
            x.unwrap_or(0.0) as f32,
            y.unwrap_or(0.0) as f32,
            scale.unwrap_or(1.0) as f32,
            rotate.unwrap_or(0.0) as f32,
        );
        Ok(())
    });
    def!("scar", |_, s, _: ()| {
        let sc = lock(s).cz.look.scar;
        Ok((sc.x, sc.y, sc.rotate, sc.scale))
    });
    def!("hair_icon", |_, s, index: f64| {
        let g = lock(s);
        let n = (index as usize).wrapping_sub(1);
        Ok(g.cz.cat.hair.get(n).map(|h| format!("ui/icon/{}", h.icon)))
    });

    // ---- premade faces, sliders -------------------------------------------------------------
    def!("apply_preset", |_, s, (part, index, fixed): (f64, f64, Option<bool>)| {
        lock(s).cz.apply_preset(part.max(0.0) as usize, index as i32, fixed.unwrap_or(false));
        Ok(())
    });
    def!("save_slot", |_, s, slot: f64| {
        lock(s).cz.save_slot(slot as u32);
        Ok(())
    });
    def!("load_slot", |_, s, slot: f64| {
        lock(s).cz.load_slot(slot as u32);
        Ok(())
    });
    def!("set_mixed", |_, s, index: f64| {
        lock(s).cz.set_mixed(index.max(0.0) as usize);
        Ok(())
    });
    def!("mix", |_, s, (part, value): (f64, f64)| {
        lock(s).cz.mix(part.max(0.0) as usize, value as f32);
        Ok(())
    });
    def!("num_targets", |_, s, part: f64| Ok(lock(s).cz.cat.targets_of(part.max(0.0) as usize).len()));
    def!("target_index", |_, s, (part, order): (f64, f64)| {
        let g = lock(s);
        Ok(g.cz.cat.targets_of(part.max(0.0) as usize).get(order.max(0.0) as usize).map(|t| t.idx))
    });
    def!("target_name", |_, s, idx: f64| Ok(lock(s).cz.slider_name(idx as usize).map(str::to_string)));
    def!("target_range", |_, s, idx: f64| Ok(lock(s).cz.slider_range(idx as usize)));
    def!("slider", |_, s, idx: f64| Ok(lock(s).cz.slider(idx as usize)));
    def!("set_slider", |_, s, (idx, v): (f64, f64)| {
        lock(s).cz.set_slider(idx as usize, v as f32);
        Ok(())
    });
    def!("set_smile", |_, s, smile: Option<bool>| {
        let mut g = lock(s);
        let smile = smile.unwrap_or(false);
        if g.cz.look.smile != smile {
            g.cz.look.smile = smile;
            g.cz.revision += 1;
        }
        Ok(())
    });
    def!("smile", |_, s, _: ()| Ok(lock(s).cz.look.smile));
    def!("set_fix", |_, s, fix: Option<bool>| {
        lock(s).cz.fix = fix.unwrap_or(false);
        Ok(())
    });
    def!("fix", |_, s, _: ()| Ok(lock(s).cz.fix));
    def!("zoom", |_, s, dir: f64| {
        let mut g = lock(s);
        g.zoom = (g.zoom + dir as f32 * 0.15).clamp(-0.3, 1.0);
        Ok(())
    });

    // ---- race advantages, saved looks ---------------------------------------------------------
    def!("race_buffs", |lua, s, (race, gender): (String, String)| {
        let t = lua.create_table()?;
        let g = lock(s);
        for (i, id) in g.cz.race_buffs(race_from(&race), gender_from(&gender)).into_iter().enumerate() {
            t.raw_set(i + 1, id)?;
        }
        Ok(t)
    });
    def!("buff_info", |_, s, id: f64| {
        let g = lock(s);
        Ok(g.cz.buff_info(id as u32, &g.locale).map(|(n, p, d)| mlua::Variadic::from_iter([n, p, d])).unwrap_or_default())
    });
    def!("saved_count", |_, _s, _: ()| Ok(saved_looks().len()));
    def!("saved_info", |_, _s, index: f64| {
        let list = saved_looks();
        Ok(list.get((index as usize).wrapping_sub(1)).map(|(n, p)| mlua::Variadic::from_iter([n.clone(), p.display().to_string()])).unwrap_or_default())
    });
    def!("saved_reload", |_, _s, _: ()| Ok(()));
    def!("saved_exists", |_, _s, name: String| Ok(Customizer::look_exists(&name)));
    def!("saved_save", |_, s, name: String| {
        let r = lock(s).cz.save_look(&name, false);
        Ok(match r {
            Ok(true) => (CDR_SAVE_SUCCESS, name),
            Ok(false) => (CDR_ERROR_AS_SAME_FILE, name),
            Err(e) => {
                bevy::log::warn!("saving the look: {e}");
                (CDR_ERROR_AS_NO_PATH, name)
            }
        })
    });
    def!("saved_overwrite", |_, s, index: f64| {
        let list = saved_looks();
        let Some((name, _)) = list.get((index as usize).wrapping_sub(1)) else { return Ok(CDR_ERROR_AS_NO_FILE) };
        Ok(match lock(s).cz.save_look(name, true) {
            Ok(_) => CDR_SAVE_SUCCESS,
            Err(_) => CDR_ERROR_AS_NO_PATH,
        })
    });
    def!("saved_load", |_, s, index: f64| {
        let list = saved_looks();
        let Some((_, path)) = list.get((index as usize).wrapping_sub(1)) else { return Ok(CDR_ERROR_AS_NO_FILE) };
        Ok(if lock(s).cz.load_look(path) { CDR_LOAD_SUCCESS } else { CDR_ERROR_AS_INVALID_DATA })
    });
    def!("saved_delete", |_, _s, index: f64| {
        let list = saved_looks();
        let Some((_, path)) = list.get((index as usize).wrapping_sub(1)) else { return Ok(CDR_ERROR_AS_NO_FILE) };
        Ok(if std::fs::remove_file(path).is_ok() { CDR_DELETE_SUCCESS } else { CDR_ERROR_AS_NO_FILE })
    });

    // ---- flow -------------------------------------------------------------------------------
    def!("end_step", |_, s, (from, forward): (f64, Option<bool>)| {
        lock(s).requests.push(Request::Step { from: from as u8, forward: forward.unwrap_or(false) });
        Ok(())
    });
    def!("goto_select", |_, s, _: ()| {
        lock(s).requests.push(Request::GotoSelect);
        Ok(())
    });
    def!("skill_action", |_, s, ability: Option<f64>| {
        lock(s).requests.push(Request::SkillAction(ability.unwrap_or(0.0) as u32));
        Ok(())
    });
    def!("create", |_, s, (name, ability): (String, f64)| {
        let mut g = lock(s);
        if g.create_pending {
            return Ok(false);
        }
        g.create_pending = true;
        g.requests.push(Request::Create { name, ability: ability as u32 });
        Ok(true)
    });
    def!("characters", |_, s, _: ()| {
        // number of characters on the world (for the "character slots full" checks): see mod.rs
        let _ = s;
        Ok(0)
    });

    lua.globals().raw_set("CC", cc)?;
    Ok(())
}
