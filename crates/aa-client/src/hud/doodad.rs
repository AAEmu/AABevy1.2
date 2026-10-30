//! Using doodads (a Memory Tome, a tree, a navi portrait ...): the generic "doodad func -> use" path.
//!
//! The client knows from the game database what each phase (`doodad_func_groups.id`, the doodad's current
//! `func_group_id`) of a doodad can do: its `doodad_funcs` rows. A func with a `func_skill_id` is used by casting that
//! skill at the doodad (`CSStartSkill` with a doodad target, [`aa_net::Command::UseDoodadSkill`]); the server's
//! `InteractionEffect` of the skill then runs `Doodad.Use(caster, skill)`, which finds the func of the current phase
//! with that skill and executes it (a Memory Tome's `DoodadFuncBinding` sets the recall point and answers
//! `SCCharacterReturnDistricts`, then the phase changes: `SCDoodadPhaseChanged`). The navi funcs have no skill and
//! their own packets (`CSNaviTeleport`, `CSNaviOpenPortal`).
//!
//! Original 1.2 capture (2026-10-30, `tools/evidence/doodad_affordance/capture-results.txt`): the bar offers a phase's
//! `func_skill_id` (the Narcissus' Gather 13990, a chair's 15857) and the reach is the skill's `max_range` ([`reach`]).
//! The lamp 2319's fake-use skill 13166 (offered by the original too) comes from the func's own detail row (`gather.rs`),
//! as do the decal portals' 17557 (not seen: the capture stood 5.5 m from one, outside its 4 m reach). Not listed here yet:
//! the Mailbox's 11400 "Open Mailbox" (no `func_skill_id` and no detail row; the mail window opens with Ctrl-E, `mail.rs`).
//!
//! What the player sees is the original's:
//!
//! * the **dynamic action bar** (`dynamic_action_bar.lua`): while a usable doodad is in reach, its skills as 48 px
//!   buttons in the lower middle of the screen with the key under each (F, G, H, J: `do_interaction_1..4`).
//!   `DYNAMIC_ACTION_BAR_SHOW("doodad", id)` / `DYNAMIC_ACTION_BAR_HIDE`; the scripts ask `X2:GetDynamicActionCount`,
//!   `GetDynamicActionSkill(i)`, `IsDynamicActionSkillToggled(i)`, `X2Interaction:GetInteractionSkillIconPath(skill)`,
//!   `IsValidIconPath(path)`; a click is `X2:ExecuteDynamicAction(i)`. The key does the same.
//! * a **right click** on a doodad uses it: one use at once, several (a navi sign) through the interaction list of
//!   `interaction.lua` (`INTERACTION_START`, `X2Interaction:GetInteractionSkillCount/Id/Name`, `SelectInteraction`,
//!   `CancelInteraction`).
//! * **hovering** a usable doodad shows its plate (`DRAW_DOODAD_TOOLTIP`): the name, the owner (from
//!   `SCDoodadOriginator`) and the growth time left (`displayTime`).
//! * the doodad the bar is about gets the native use-target ring (`doodad_almighties.use_target_decal`, drawn by
//!   `ingame::doodad_affordance`) and the `use_target_highlight` glow (`ingame::doodad_visual`).
//! * `SCDoodadPhaseMsg` becomes `DOODAD_PHASE_MSG(text)` (the chat scripts print it), with the phase's `phase_msg`.
//!
//! The cast bar, the cast animation and the result come from the server's answers like for any skill (`cast.rs`,
//! `ingame::combat`, `portal.rs` for the new recall point).
//!
//! F goes to the doodad only when no loot is in reach (F loots) and no NPC is nearer than it (F talks).
//!
//! Dev flags: `--doodad-key N T` presses `do_interaction_N` T seconds after the HUD is up; `--doodad-rclick T` right
//! clicks the nearest usable doodad (as if the pointer were on it).

use std::collections::{HashMap, HashSet};

use crate::ingame::doodad_affordance::{DoodadDecal, DoodadTargetDecal};
use crate::ingame::doodad_visual::{DoodadHighlights, DoodadVisual};
use crate::ingame::units::{Views, doodad_world};
use aa_net::interaction::Interact;
use aa_net::CofferCommand;
use aa_net::state::{Doodads, Inventory, Units};
use aa_net::{Command, NetCommand};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use mlua::{Lua, Table, Value};
use rusqlite::Connection;

use super::api::{HudBridge, HudData, Shared};
use super::{HudKeyboardFocus, HudPointerOverUi};

mod gather;

/// Skills the dynamic action bar leaves out (`dynamic_action_bar.lua`, `ShoulPassShow`).
const HIDDEN_SKILLS: [u32; 2] = [0, 10994];
/// Keys of the dynamic action bar (`do_interaction_1` ... ) that use a doodad.
const KEYS: usize = 4;
/// Reach of a doodad use without a skill range, metres (not captured). A skill's own `max_range` is used as is
/// ([`skill_reach`]): the 2026-10-30 original-client capture showed no bar entry for Gather (13990, `max_range` 4) at
/// 4.5 m, so there is no extra slack.
const BASE_REACH: f32 = 4.0;
/// How much further away a doodad right behind the player may be than one in front of it and still be preferred, metres.
const BEHIND: f32 = 3.0;
/// How far a right click or the pointer picks doodads, metres.
const PICK_DISTANCE: f32 = 40.0;
/// A press that moves more than this many pixels before it is released was a camera drag.
const CLICK_SLOP: f32 = 6.0;
/// How far from a coffer its window stays open (the server serves it within 5 m), metres.
const COFFER_RANGE: f32 = 6.0;
/// The ids of the navi uses in the interaction list (skill ids are below this).
const NAVI_ID: i64 = 1 << 32;
/// How long the walk to a doodad may go without getting nearer before it is given up, seconds.
const STUCK_SECS: f32 = 3.0;
/// The keys that end the walk to a doodad (the player takes over).
const MOVE_ACTIONS: [&str; 5] = ["moveforward", "moveback", "moveleft", "moveright", "jump"];

/// A use without a skill.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Navi {
    Teleport,
    OpenPortal,
    /// A house safe: `CSCofferInteraction` opens its window.
    Coffer,
}

impl Navi {
    fn label(self) -> &'static str {
        match self {
            Navi::Teleport => "Teleport",
            Navi::OpenPortal => "Open Worldgate",
            Navi::Coffer => "Open",
        }
    }
}

/// What a doodad phase can be used for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Uses {
    /// The skills of its funcs, in func order, without repeats.
    pub skills: Vec<u32>,
    pub navi: Vec<Navi>,
}

#[derive(Default)]
pub(super) struct Tables {
    /// `doodad_func_groups.id` -> its uses.
    uses: HashMap<u32, Uses>,
    /// `doodad_almighties.id` -> English name.
    names: HashMap<u32, String>,
    /// The doodads that do not light up when they are in reach or under the pointer (`use_target_highlight` off).
    dark: HashSet<u32>,
    /// The doodads drawn as a silhouette when they are the use target (`use_target_silhouette` on).
    silhouettes: HashSet<u32>,
    /// `doodad_almighties.id` -> `target_decal_size` of the use-target decal, when `use_target_decal` is on.
    decals: HashMap<u32, f32>,
    /// `doodad_func_groups.id` -> the phase message shown by `DOODAD_PHASE_MSG`.
    messages: HashMap<u32, String>,
    /// The phase messages with `is_msg_to_zone`: shown even when the doodad itself is not known.
    zone_messages: HashSet<u32>,
    /// Attachment funcs require a known-free doodad bond and the separate mount gate.
    attachment_skills: HashSet<(u32, u32)>,
}

impl Tables {
    pub(super) fn load(conn: &Connection) -> rusqlite::Result<Self> {
        let mut t = Tables::default();
        let detail = gather::DetailSkills::load(conn)?;
        let mut q = conn.prepare("SELECT doodad_func_group_id, func_skill_id, actual_func_type, actual_func_id FROM doodad_funcs ORDER BY doodad_func_group_id, id")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            let group = super::data::any_int(r, 0)? as u32;
            let skill = super::data::any_int(r, 1)? as u32;
            let kind: Option<String> = r.get(2)?;
            let func_id = super::data::any_int(r, 3)? as u32;
            t.add_resolved(
                group,
                skill,
                kind.as_deref().unwrap_or(""),
                func_id,
                &detail,
            );
        }
        let mut q = conn.prepare("SELECT id, use_target_highlight, use_target_silhouette, use_target_decal, target_decal_size FROM doodad_almighties")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            let id = super::data::any_int(r, 0)? as u32;
            if !super::data::any_flag(r, 1)? {
                t.dark.insert(id);
            }
            if super::data::any_flag(r, 2)? {
                t.silhouettes.insert(id);
            }
            if super::data::any_flag(r, 3)? {
                t.decals.insert(id, any_float(r, 4)?);
            }
        }
        let mut q = conn.prepare("SELECT idx, en_us FROM localized_texts WHERE tbl_name = 'doodad_almighties' AND tbl_column_name = 'name' AND en_us <> ''")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            t.names.insert(super::data::any_int(r, 0)? as u32, r.get(1)?);
        }
        let mut localized = HashMap::new();
        let mut q = conn.prepare("SELECT idx, en_us FROM localized_texts WHERE tbl_name = 'doodad_func_groups' AND tbl_column_name = 'phase_msg' AND en_us <> ''")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            localized.insert(super::data::any_int(r, 0)? as u32, r.get::<_, String>(1)?);
        }
        // Native 395b1350 falls back to the raw column when the translation is missing or empty.
        let mut q = conn.prepare("SELECT id, is_msg_to_zone, phase_msg FROM doodad_func_groups WHERE phase_msg IS NOT NULL AND phase_msg <> ''")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            let id = super::data::any_int(r, 0)? as u32;
            let raw = r.get::<_, String>(2)?;
            let text = localized
                .get(&id)
                .map(|s| cstr(s).to_owned())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| cstr(&raw).to_owned());
            if text.is_empty() {
                continue;
            }
            if super::data::any_flag(r, 1)? {
                t.zone_messages.insert(id);
            }
            t.messages.insert(id, text);
        }
        Ok(t)
    }

    /// [`Tables::add`] for a `doodad_funcs` row whose own detail table (or, for the legacy gather
    /// funcs, the server adapter) names the skill when `func_skill_id` is empty.
    fn add_resolved(
        &mut self,
        group: u32,
        skill: u32,
        kind: &str,
        func_id: u32,
        detail: &gather::DetailSkills,
    ) {
        let skill = if skill != 0 {
            skill
        } else {
            detail.skill(kind, func_id).unwrap_or(0)
        };
        self.add(group, skill, kind);
    }

    fn add(&mut self, group: u32, skill: u32, kind: &str) {
        if skill != 0 && kind == "DoodadFuncAttachment" {
            self.attachment_skills.insert((group, skill));
        }
        let navi = match kind {
            "DoodadFuncNaviTeleport" => Some(Navi::Teleport),
            "DoodadFuncNaviOpenPortal" => Some(Navi::OpenPortal),
            "DoodadFuncCoffer" => Some(Navi::Coffer),
            _ => None,
        };
        if skill == 0 && navi.is_none() {
            return;
        }
        let uses = self.uses.entry(group).or_default();
        if skill != 0 {
            if !HIDDEN_SKILLS.contains(&skill) && !uses.skills.contains(&skill) {
                uses.skills.push(skill);
            }
        } else if let Some(n) = navi
            && !uses.navi.contains(&n)
        {
            uses.navi.push(n);
        }
    }

    /// What the phase `group` can be used for (`None`: nothing).
    pub(super) fn uses(&self, group: u32) -> Option<&Uses> {
        self.uses.get(&group).filter(|u| !u.skills.is_empty() || !u.navi.is_empty())
    }

    fn allows_attachment(&self, group: u32, skill: u32, bond_free: bool, mount_free: bool) -> bool {
        !self.attachment_skills.contains(&(group, skill)) || (bond_free && mount_free)
    }

    /// Does the doodad light up (`use_target_highlight`)?
    pub(super) fn glows(&self, template_id: u32) -> bool {
        !self.dark.contains(&template_id)
    }

    /// Is the doodad drawn as a silhouette when it is the use target (`use_target_silhouette`)? No 1.2 doodad sets
    /// the column (0 of 7124 rows); the mapping is kept for the column test and for models that may.
    #[allow(dead_code)]
    pub(super) fn silhouettes(&self, template_id: u32) -> bool {
        self.silhouettes.contains(&template_id)
    }

    /// The use-target decal (`target_decal_size`, 0 = the client's default) when the template has `use_target_decal`.
    pub(super) fn decal_size(&self, template_id: u32) -> Option<f32> {
        self.decals.get(&template_id).copied()
    }

    /// The phase message of `doodad_func_groups.id` (`DOODAD_PHASE_MSG`).
    pub(super) fn message(&self, func_group_id: u32) -> Option<&str> {
        self.messages.get(&func_group_id).map(String::as_str)
    }

    /// Is the phase message for the whole zone (`is_msg_to_zone`) rather than only for the doodad in view?
    pub(super) fn zone_message(&self, func_group_id: u32) -> bool {
        self.zone_messages.contains(&func_group_id)
    }

    pub(super) fn name(&self, template_id: u32) -> Option<&str> {
        self.names.get(&template_id).map(String::as_str)
    }
}

/// A REAL column that may also arrive as text in hand-made databases.
fn any_float(r: &rusqlite::Row, i: usize) -> rusqlite::Result<f32> {
    use rusqlite::types::Value;
    Ok(match r.get::<_, Value>(i)? {
        Value::Real(f) => f as f32,
        Value::Integer(n) => n as f32,
        Value::Text(t) => t.trim().parse().unwrap_or(0.0),
        _ => 0.0,
    })
}

/// The native text helpers stop at an embedded NUL.
fn cstr(s: &str) -> &str {
    s.split('\0').next().unwrap_or_default()
}

/// The usable doodad in reach.
#[derive(Clone, Debug, PartialEq)]
struct Focus {
    obj_id: u32,
    skills: Vec<u32>,
    dist: f32,
}

/// One row of the interaction list.
#[derive(Clone, Debug, PartialEq)]
struct Entry {
    id: i64,
    name: String,
}

#[derive(Default)]
pub(super) struct View {
    focus: Option<Focus>,
    /// F goes to the doodad (it is nearer than any NPC).
    claims_key: bool,
    /// The interaction list: the doodad and its rows.
    menu: Option<(u32, Vec<Entry>)>,
    /// The ids picked from the list.
    chosen: Vec<i64>,
    /// The doodad under the pointer (its name tooltip is up).
    hover: Option<u32>,
    /// What the doodad tooltip shows now.
    tip: Option<Tip>,
    /// The doodad last right clicked: the bar is about it while it is in reach.
    preferred: Option<u32>,
    /// The walk to a doodad that was right clicked out of reach.
    approach: Option<Approach>,
    /// The coffer window was closed.
    coffer_close: bool,
    pointer_pick: Option<u32>,
    arrived: Option<u32>,
}

/// The doodad tooltip (`DRAW_DOODAD_TOOLTIP` of `interaction.lua`): the name, `Owner: ...` and the time the growth has left.
#[derive(Clone, Debug, PartialEq)]
struct Tip {
    obj_id: u32,
    name: String,
    owner: Option<String>,
    secs: u32,
}

/// The content of the hover plate (`DRAW_DOODAD_TOOLTIP` of `interaction.lua`): name, owner while it has one, and the
/// whole seconds of growth left (the script's `displayTime` -> `MakeTimeString`). Capture-pending: the 2026-10-30
/// capture read no plate text (the driver sees no tooltip and the seedling it stood at offered no action), so when the
/// native passes `owner` and `displayTime` to the script is still unseen.
fn plate(obj_id: u32, name: String, owner: Option<String>, remaining_ms: f32) -> Tip {
    Tip {
        obj_id,
        name,
        owner,
        secs: ((remaining_ms / 1000.0).max(0.0)).ceil() as u32,
    }
}

/// The character walks to a doodad that was right clicked out of reach and uses it there (the 1.2 client's move to the
/// interaction target: `InteractionManager` keeps the pending use, the character runs to the doodad, the use follows).
/// Capture-pending: the 2026-10-30 capture never right clicked a far doodad. Its only far right clicks, on a Farm Wagon
/// unit at 7.8 m and 27.8 m, targeted it and did not walk (no packet, no movement in 6 s), which argues against this.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Approach {
    obj_id: u32,
    /// The nearest the character has been, metres, and when it last got nearer (`Time::elapsed_secs`).
    best: f32,
    best_at: f32,
}

/// How near the character walks to a doodad whose use reaches `reach` metres: well inside it.
fn stop_distance(reach: f32) -> f32 {
    (reach - 1.0).clamp(1.5, 3.0)
}

impl View {
    pub(super) fn has_menu(&self) -> bool { self.menu.is_some() }
    /// F (`do_interaction_1`) uses the doodad in reach instead of talking to an NPC.
    pub(super) fn claims_key(&self) -> bool {
        self.claims_key
    }
}

/// The reach of a doodad whose phase has `uses`, metres: [`skill_reach`] of the first skill's `max_range`.
fn reach(data: &HudData, uses: &Uses) -> f32 {
    let range = data.game.get().and_then(|g| uses.skills.first().and_then(|s| g.skills.get(s))).map_or(0.0, |s| s.max_range as f32);
    skill_reach(range)
}

/// A skill's `max_range` (at least 2 m, the minimum is ours), else [`BASE_REACH`]. The range is to the doodad's origin
/// with no slack. The 2026-10-30 1.2 capture: Gather 13990 (`max_range` 4) has a bar entry at 3 m and none at 4.5 m or
/// 8 m (`tools/evidence/gather/capture-results.txt`, `tools/evidence/doodad_affordance/capture-results.txt`).
fn skill_reach(max_range: f32) -> f32 {
    if max_range > 0.0 {
        max_range.max(2.0)
    } else {
        BASE_REACH
    }
}

/// The 1.2 client refuses a doodad use that costs more labor than the player has, before it asks the server: the
/// 2026-10-30 capture printed "Insufficient Labor Points." and sent no `CSStartSkill` (Gather 13990, 1 LP). A skill that
/// costs no labor has nothing to refuse, so a labor below zero does not stop it (a free skill below zero LP was not
/// sampled in the capture).
fn lacks_labor(rules: Option<&super::usable::Rules>, skill: u32, labor: i32) -> bool {
    rules.is_some_and(|r| {
        let cost = r.labor_cost(skill);
        cost > 0 && cost > labor
    })
}

fn to_vec(p: aa_net::Pos) -> Vec3 {
    Vec3::new(p.x, p.y, p.z)
}

/// The doodad in reach whose phase has a skill to use that the bar is about: the one last right clicked, else the
/// nearest, a doodad behind the player counting up to [`BEHIND`] metres further away than one in front (an inn's
/// chairs around the Memory Tome). The capture's bar beside tome 851 (a chair 0.7 m away, the tome 4.9 m) was the
/// chair's skill 15857, as here; beside tome 850 (the Mailbox 1.5 m away, the tome 2.7 m) it was 11400, the Mailbox's
/// "Open Mailbox", which is not listed here yet (see the module doc). The tomes' own 14602 was not observed.
fn find_focus(data: &HudData, doodads: &Doodads, units: &Units) -> Option<Focus> {
    let game = data.game.get()?;
    let forward = Vec2::new(-data.yaw.sin(), data.yaw.cos());
    let candidates = doodads.by_id.values().filter_map(|o| {
        let uses = game.doodad.uses(o.func_group_id)?;
        if uses.skills.is_empty() {
            return None;
        }
        let pos = doodad_world(o, units)?;
        let dist = data.pos.distance(pos);
        if dist > reach(data, uses) {
            return None;
        }
        let toward = (pos - data.pos).truncate().normalize_or_zero();
        let score = if data.doodad.preferred == Some(o.obj_id) { f32::MIN } else { dist + BEHIND * 0.5 * (1.0 - forward.dot(toward)) };
        Some((score, Focus { obj_id: o.obj_id, skills: uses.skills.clone(), dist }))
    });
    candidates.min_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, f)| f)
}

pub(super) fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    let x2: Table = lua.globals().get("X2")?;
    let state = bridge.clone();
    x2.set("GetDynamicActionCount", lua.create_function(move |_, _: Value| Ok(state.borrow().dynamic_actions.count()))?)?;
    let state = bridge.clone();
    x2.set(
        "GetDynamicActionSkill",
        lua.create_function(move |_, (_, i): (Value, usize)| {
            Ok(match state.borrow().dynamic_actions.skill(i) {
                Some(skill) => mlua::MultiValue::from_vec(vec![crate::lua_int(i64::from(skill))]),
                None => mlua::MultiValue::new(),
            })
        })?,
    )?;
    x2.set("IsDynamicActionSkillToggled", lua.create_function(|_, _: mlua::MultiValue| Ok(false))?)?;
    let state = bridge.clone();
    x2.set(
        "ExecuteDynamicAction",
        lua.create_function(move |_, (_, i): (Value, usize)| {
            state.borrow_mut().dynamic_actions.request(i);
            Ok(())
        })?,
    )?;

    let interaction: Table = lua.globals().get("X2Interaction")?;
    let state = bridge.clone();
    interaction.set(
        "GetInteractionSkillIconPath",
        lua.create_function(move |_, (_, skill): (Value, u32)| {
            let d = state.borrow();
            Ok(d.game.get().and_then(|g| g.skills.get(&skill)).map(|s| s.named.icon.clone()).filter(|p| !p.is_empty()).unwrap_or_else(|| "ui/icon/icon_skill_none.dds".into()))
        })?,
    )?;
    interaction.set("IsValidIconPath", lua.create_function(|_, (_, path): (Value, String)| Ok(aa_pak::Pak::global().exists(&path)))?)?;
    let state = bridge.clone();
    interaction.set("GetInteractionSkillCount", lua.create_function(move |_, _: Value| Ok(state.borrow().doodad.menu.as_ref().map_or(0, |m| m.1.len())))?)?;
    let state = bridge.clone();
    interaction.set(
        "GetInteractionSkillId",
        lua.create_function(move |_, (_, k): (Value, usize)| Ok(state.borrow().doodad.menu.as_ref().and_then(|m| m.1.get(k.wrapping_sub(1)).map(|e| e.id))))?,
    )?;
    let state = bridge.clone();
    interaction.set(
        "GetInteractionSkillName",
        lua.create_function(move |_, (_, k): (Value, usize)| Ok(state.borrow().doodad.menu.as_ref().and_then(|m| m.1.get(k.wrapping_sub(1)).map(|e| e.name.clone()))))?,
    )?;
    let state = bridge.clone();
    interaction.set(
        "SelectInteraction",
        lua.create_function(move |_, (_, id): (Value, i64)| {
            state.borrow_mut().doodad.chosen.push(id);
            Ok(())
        })?,
    )?;
    let prior: Option<mlua::Function> = interaction.get("CancelInteraction")?;
    let state = bridge.clone();
    interaction.set(
        "CancelInteraction",
        lua.create_function(move |_, args: mlua::MultiValue| {
            if state.borrow_mut().doodad.menu.take().is_some() {
                return Ok(());
            }
            match &prior {
                Some(f) => f.call::<()>(args),
                None => Ok(()),
            }
        })?,
    )?;
    // the coffer window (`coffer.lua`) tells when it was closed
    let state = bridge.clone();
    let hud: Table = lua.globals().get("HUD")?;
    hud.set(
        "coffer_close",
        lua.create_function(move |_, ()| {
            state.borrow_mut().doodad.coffer_close = true;
            Ok(())
        })?,
    )?;
    lua.load(include_str!("coffer.lua")).set_name("@coffer.lua").exec()
}

/// `--doodad-key N T` and `--doodad-rclick T`.
#[derive(Resource, Default)]
struct Scripted {
    keys: Vec<(f32, usize)>,
    rclicks: Vec<f32>,
    /// `--doodad-near T`: log the doodads within 10 m at T seconds.
    near: Vec<f32>,
    /// `--doodad-hover T`: from T on the pointer is on the nearest usable doodad.
    hovers: Vec<f32>,
    hovering: bool,
    since: f32,
}

impl Scripted {
    fn from_args() -> Self {
        let args: Vec<String> = aa_dev::args();
        let num = |i: usize| args.get(i).and_then(|v| v.parse::<f32>().ok());
        let mut s = Scripted::default();
        for (i, a) in args.iter().enumerate() {
            match a.as_str() {
                "--doodad-key" => {
                    if let (Some(n), Some(t)) = (num(i + 1), num(i + 2)) {
                        s.keys.push((t, n as usize));
                    }
                }
                "--doodad-rclick" => s.rclicks.extend(num(i + 1)),
                "--doodad-near" => s.near.extend(num(i + 1)),
                "--doodad-hover" => s.hovers.extend(num(i + 1)),
                _ => {}
            }
        }
        s.keys.sort_by(|a, b| b.0.total_cmp(&a.0));
        s.rclicks.sort_by(|a, b| b.total_cmp(a));
        s.near.sort_by(|a, b| b.total_cmp(a));
        s.hovers.sort_by(|a, b| b.total_cmp(a));
        s
    }
}

/// The pointer over the 3D view.
#[derive(bevy::ecs::system::SystemParam)]
struct Pointer<'w, 's> {
    mouse: Res<'w, ButtonInput<MouseButton>>,
    over_ui: Res<'w, HudPointerOverUi>,
    windows: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
    cameras: Query<'w, 's, (&'static Camera, &'static GlobalTransform), (With<Camera3d>, Without<crate::hud::ModelViewCamera>)>,
}

impl Pointer<'_, '_> {
    /// The view ray under the cursor (not over the HUD).
    fn ray(&self) -> Option<Ray3d> {
        if self.over_ui.0 {
            return None;
        }
        let cursor = self.windows.single().ok()?.cursor_position()?;
        let (camera, transform) = self.cameras.iter().find(|(c, _)| c.is_active)?;
        camera.viewport_to_world(transform, cursor).ok()
    }
}

/// How far along `ray` it passes within `radius` of `centre` (game coordinates), if it does.
fn hit(ray: Ray3d, centre: Vec3, radius: f32) -> Option<f32> {
    let centre = aa_world::cry_to_bevy(centre);
    let along = (centre - ray.origin).dot(*ray.direction);
    (along > 0.0 && (ray.origin + *ray.direction * along).distance(centre) <= radius).then_some(along)
}

/// The usable doodad the ray picks, unless an NPC is in front of it.
fn pick(ray: Ray3d, data: &HudData, doodads: &Doodads, units: &Units, own: u32) -> Option<u32> {
    let game = data.game.get()?;
    let mut best: Option<(f32, u32)> = None;
    for o in doodads.by_id.values() {
        let Some(pos) = doodad_world(o, units) else { continue };
        if (game.doodad.uses(o.func_group_id).is_none() && o.owner_id == 0) || data.pos.distance(pos) > PICK_DISTANCE {
            continue;
        }
        let scale = if o.scale > 0.0 { o.scale } else { 1.0 };
        let centre = pos + Vec3::Z * 0.8 * scale;
        if let Some(along) = hit(ray, centre, 1.3 * scale.max(0.7))
            && best.is_none_or(|(a, _)| along < a)
        {
            best = Some((along, o.obj_id));
        }
    }
    let (along, obj_id) = best?;
    let npc_in_front = units.iter().filter(|u| u.obj_id != own && matches!(u.kind, aa_net::model::UnitKind::Npc { .. })).any(|u| {
        hit(ray, to_vec(u.pos) + Vec3::Z * 0.9 * u.scale.max(0.5), 1.1 * u.scale.max(0.7)).is_some_and(|a| a < along)
    });
    (!npc_in_front).then_some(obj_id)
}

/// The system that decides the doodad in reach, the plate and the walk target; `ingame::doodad_affordance` draws the
/// target decal after it (it reads the `DoodadTargetDecal` this writes).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct TrackDoodads;

/// Finds the doodad in reach, shows / hides the bar, reads the keys, the right click and the pointer.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn track(
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<HudBridge>,
    doodads: Res<Doodads>,
    units: Res<Units>,
    player: Res<aa_net::state::Player>,
    actions: Res<crate::settings::Actions>,
    focus_kb: Res<HudKeyboardFocus>,
    pointer: Pointer,
    time: Res<Time>,
    mut script: ResMut<Scripted>,
    mut pressed_at: Local<Option<Vec2>>,
    mut cameras: Query<&mut crate::ingame::camera::OrbitCamera>,
    mut dev: ResMut<crate::ingame::DevOptions>,
    (mut glows, names, views, visuals, mut target_decal): (
        ResMut<DoodadHighlights>,
        Res<aa_net::CharacterNames>,
        Res<Views>,
        Query<&DoodadVisual>,
        ResMut<DoodadTargetDecal>,
    ),
    mut out: MessageWriter<NetCommand>,
    mut asked: Local<std::collections::HashSet<u32>>,
) {
    let Some(rt) = rt else { return };
    let own = player.spawn.as_ref().map_or(0, |s| s.obj_id);
    script.since += time.delta_secs();
    let mut d = bridge.0.borrow_mut();
    let focus = find_focus(&d, &doodads, &units);
    // the bar
    let wanted = focus.as_ref().and_then(|f| {
        let phase = doodads.by_id.get(&f.obj_id)?.func_group_id;
        let generation = views.doodad_generation(f.obj_id)?;
        Some(super::dynamic_actions::Shown {
            source: super::dynamic_actions::Source::Doodad { obj_id: f.obj_id, phase, generation },
            skills: f.skills.clone(),
        })
    });
    let directing = d.dialog.directing;
    d.doodad.claims_key = focus.as_ref().is_some_and(|f| {
        !units.iter().any(|u| u.obj_id != own && !u.dead && matches!(u.kind, aa_net::model::UnitKind::Npc { .. }) && d.pos.distance(to_vec(u.pos)) < f.dist)
    });
    d.doodad.focus = focus;
    if !directing && d.dynamic_actions.replace(wanted.clone()) {
        drop(d);
        match &wanted {
            Some(super::dynamic_actions::Shown { source: super::dynamic_actions::Source::Doodad { obj_id, .. }, skills }) => {
                info!("doodad: {obj_id} in reach, uses {skills:?}");
                rt.fire_event("DYNAMIC_ACTION_BAR_SHOW", ("doodad", *obj_id));
            }
            None => rt.fire_event("DYNAMIC_ACTION_BAR_HIDE", ()),
            Some(_) => unreachable!("the doodad producer publishes only doodad sources"),
        }
        d = bridge.0.borrow_mut();
    }
    // the keys
    let typing = focus_kb.0 || rt.keyboard_captured();
    let moving = !typing && MOVE_ACTIONS.iter().any(|a| actions.pressed(a, 0));
    let loot = d.loot.nearest.is_some();
    for n in 1..=KEYS {
        if !typing && !loot && !directing && d.doodad.claims_key && actions.just_pressed(&format!("do_interaction_{n}"), 0) {
            d.dynamic_actions.request(n);
        }
    }
    while script.keys.last().is_some_and(|k| script.since >= k.0) {
        let (_, n) = script.keys.pop().unwrap();
        info!("doodad: --doodad-key {n}");
        d.dynamic_actions.request(n);
    }
    if script.near.last().is_some_and(|t| script.since >= *t) {
        script.near.pop();
        log_near(&d, &doodads, &units);
    }
    // the pointer: a right click uses, a hover names
    let blocked = super::blocks_look_pointer(&d, MouseButton::Right);
    if blocked { *pressed_at = None; }
    let ray = pointer.ray();
    let cursor = pointer.windows.single().ok().and_then(|w| w.cursor_position());
    if !blocked && pointer.mouse.just_pressed(MouseButton::Right) {
        *pressed_at = if pointer.over_ui.0 { None } else { cursor };
    }
    let mut clicked = None;
    if !blocked && pointer.mouse.just_released(MouseButton::Right)
        && let (Some(start), Some(end), Some(ray)) = (pressed_at.take(), cursor, ray)
        && start.distance(end) <= CLICK_SLOP
    {
        clicked = pick(ray, &d, &doodads, &units, own);
    }
    if script.rclicks.last().is_some_and(|t| script.since >= *t) {
        script.rclicks.pop();
        clicked = (!blocked).then(|| nearest_usable(&d, &doodads, &units)).flatten();
        info!("doodad: --doodad-rclick on {clicked:?}");
    }
    if script.hovers.last().is_some_and(|t| script.since >= *t) {
        script.hovers.pop();
        script.hovering = true;
    }
    let hover = ray.and_then(|r| pick(r, &d, &doodads, &units, own)).or_else(|| script.hovering.then(|| nearest_usable(&d, &doodads, &units)).flatten());
    d.doodad.hover = hover;
    let arrived = walk_to(&mut d, &doodads, &units, time.elapsed_secs(), moving, &mut cameras, &mut dev);
    d.doodad.pointer_pick = clicked;
    d.doodad.arrived = arrived;
    // the glow: the doodad the bar is about and the one under the pointer
    let glow: Vec<u32> = d.doodad.focus.iter().map(|f| f.obj_id).chain(hover).filter(|id| doodads.by_id.get(id).is_some_and(|o| d.game.get().is_some_and(|g| g.doodad.glows(o.template_id)))).collect();
    if glow != glows.0 {
        glows.0 = glow;
    }
    // the native target decal manager's "doodad" layer: the ring under the doodad the use bar is about
    let decal = d.doodad.focus.as_ref().and_then(|f| {
        let o = doodads.by_id.get(&f.obj_id)?;
        let size = d
            .game
            .get()
            .and_then(|g| g.doodad.decal_size(o.template_id))?;
        Some(DoodadDecal {
            obj_id: f.obj_id,
            size,
            scale: if o.scale > 0.0 { o.scale } else { 1.0 },
        })
    });
    if target_decal.0 != decal {
        target_decal.0 = decal;
    }
    // the tooltip: name, owner and the growth time left
    let tip = hover.and_then(|id| doodads.by_id.get(&id)).map(|o| {
        let name = d.game.get().and_then(|g| g.doodad.name(o.template_id)).unwrap_or("?").to_string();
        let own_char = player.spawn.as_ref().map(|s| (s.state.character.id, s.state.character.name.as_str()));
        let owner = (o.owner_id != 0).then(|| owner_name(o.owner_id, own_char, &units, &names)).flatten();
        if o.owner_id != 0 && owner.is_none() && asked.insert(o.owner_id) {
            out.write(NetCommand(Command::QueryCharacterName { character_id: own_char.map_or(0, |c| c.0), requested_id: o.owner_id }));
        }
        let remaining = views.doodad_entity(o.obj_id).and_then(|e| visuals.get(e).ok()).map_or(0.0, |v| v.remaining_ms);
        plate(o.obj_id, name, owner, remaining)
    });
    let tip_changed = tip != d.doodad.tip;
    d.doodad.tip = tip.clone();
    drop(d);
    if tip_changed {
        match tip {
            Some(tip) => {
                let info = rt.lua.create_table().and_then(|t| {
                    t.set("name", tip.name)?;
                    if let Some(owner) = tip.owner {
                        t.set("owner", owner)?;
                    }
                    if tip.secs > 0 {
                        t.set("displayTime", tip.secs)?;
                    }
                    Ok(t)
                });
                if let Ok(info) = info {
                    rt.fire_event("DRAW_DOODAD_TOOLTIP", info);
                }
            }
            None => rt.fire_event("DRAW_DOODAD_SIGN_TAG", ()),
        }
    }
}

fn pointer_use(
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<HudBridge>,
    doodads: Res<Doodads>,
    units: Res<Units>,
    time: Res<Time>,
    mut dev: ResMut<crate::ingame::DevOptions>,
) {
    let mut data = bridge.0.borrow_mut();
    let clicked = data.doodad.pointer_pick.take();
    let clicked = (!super::blocks_look_pointer(&data, MouseButton::Right))
        .then_some(clicked)
        .flatten();
    let arrived = data.doodad.arrived.take();
    let menu = clicked
        .or(arrived)
        .is_some_and(|id| right_click(&mut data, &doodads, &units, id, time.elapsed_secs(), &mut dev));
    drop(data);
    if menu && let Some(rt) = rt {
        rt.fire_event("INTERACTION_START", ());
    }
}

/// The name of the character `owner_id` as far as it is known: the player, a character in view, a name received on the
/// stream.
fn owner_name(owner_id: u32, own: Option<(u32, &str)>, units: &Units, names: &aa_net::CharacterNames) -> Option<String> {
    if let Some((id, name)) = own
        && id == owner_id
    {
        return Some(name.to_string());
    }
    if let Some(u) = units.iter().find(|u| matches!(u.kind, aa_net::model::UnitKind::Character { char_id } if char_id == owner_id)) {
        return Some(u.name.clone());
    }
    names.by_id.get(&owner_id).map(|n| String::from_utf8_lossy(n).into_owned()).filter(|n| !n.is_empty())
}

/// What one frame of the walk decides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Walk {
    /// Keep running at the doodad.
    Run,
    /// The doodad is inside [`stop_distance`]: use it.
    Arrived,
    /// It has not got nearer for [`STUCK_SECS`]: give up (the use may still be in reach).
    Stuck,
    /// The doodad is gone or the player took over.
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Step {
    walk: Walk,
    best: f32,
    best_at: f32,
}

/// The walk's state machine for one frame: the nearest distance so far and when it was reached (0.3 m counts as
/// progress), then arrived / stuck / run / cancel. Pure, so the timing is tested without a world.
fn advance(
    target_gone: bool,
    moving: bool,
    dist: f32,
    reach: f32,
    best: f32,
    best_at: f32,
    now: f32,
) -> Step {
    if target_gone || moving {
        return Step {
            walk: Walk::Cancel,
            best,
            best_at,
        };
    }
    let (best, best_at) = if dist < best - 0.3 {
        (dist, now)
    } else {
        (best, best_at)
    };
    let walk = if dist <= stop_distance(reach) {
        Walk::Arrived
    } else if now - best_at > STUCK_SECS {
        Walk::Stuck
    } else {
        Walk::Run
    };
    Step {
        walk,
        best,
        best_at,
    }
}

/// One frame of the walk to a doodad that was right clicked out of reach: the character runs towards it (the camera
/// turns to it and W is held, like the script runner's `walk`) until it is near enough, and the doodad is returned to be
/// used. A key of the player's, a doodad that is gone, no progress for [`STUCK_SECS`] or a use that is in reach already
/// end the walk.
fn walk_to(
    d: &mut HudData,
    doodads: &Doodads,
    units: &Units,
    now: f32,
    moving: bool,
    cameras: &mut Query<&mut crate::ingame::camera::OrbitCamera>,
    dev: &mut crate::ingame::DevOptions,
) -> Option<u32> {
    let mut a = d.doodad.approach?;
    let stop = |dev: &mut crate::ingame::DevOptions| dev.script_keys.retain(|c| c != 'W');
    let target = doodads
        .by_id
        .get(&a.obj_id)
        .and_then(|o| doodad_world(o, units));
    let reach = d
        .game
        .get()
        .zip(doodads.by_id.get(&a.obj_id))
        .and_then(|(g, o)| g.doodad.uses(o.func_group_id))
        .map_or(BASE_REACH, |u| reach(d, u));
    let dist = target.map_or(f32::INFINITY, |t| d.pos.distance(t));
    let step = advance(
        target.is_none(),
        moving,
        dist,
        reach,
        a.best,
        a.best_at,
        now,
    );
    a.best = step.best;
    a.best_at = step.best_at;
    match step.walk {
        Walk::Cancel => {
            stop(dev);
            d.doodad.approach = None;
            return None;
        }
        Walk::Arrived => {
            stop(dev);
            d.doodad.approach = None;
            return Some(a.obj_id);
        }
        Walk::Stuck => {
            stop(dev);
            d.doodad.approach = None;
            info!("doodad: stopped {dist:.1} m from {} (no progress)", a.obj_id);
            // close enough for the server's range check: use it from here
            return (dist <= reach).then_some(a.obj_id);
        }
        Walk::Run => {}
    }
    let target = target?;
    d.doodad.approach = Some(a);
    let toward = (target - d.pos).truncate();
    for mut o in cameras.iter_mut() {
        o.yaw = (-toward.x).atan2(toward.y);
    }
    if !dev.script_keys.contains('W') {
        dev.script_keys.push('W');
    }
    None
}

/// `--doodad-near`: the doodads around the player, their phase and uses.
fn log_near(d: &HudData, doodads: &Doodads, units: &Units) {
    let Some(game) = d.game.get() else { return };
    let mut near: Vec<_> = doodads.by_id.values().filter_map(|o| Some((d.pos.distance(doodad_world(o, units)?), o))).filter(|(dist, _)| *dist <= 10.0).collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (dist, o) in near {
        let Some(pos) = doodad_world(o, units) else { continue };
        let name = game.doodad.name(o.template_id).unwrap_or("?");
        info!("doodad: near {dist:.1} m: {name} ({}, template {}) at ({:.1}, {:.1}, {:.1}) phase {} uses {:?}", o.obj_id, o.template_id, pos.x, pos.y, pos.z, o.func_group_id, game.doodad.uses(o.func_group_id));
    }
}

/// The nearest doodad with a use within the pick distance (`--doodad-rclick`).
fn nearest_usable(d: &HudData, doodads: &Doodads, units: &Units) -> Option<u32> {
    let game = d.game.get()?;
    doodads
        .by_id
        .values()
        .filter(|o| game.doodad.uses(o.func_group_id).is_some())
        .filter_map(|o| Some((d.pos.distance(doodad_world(o, units)?), o.obj_id)))
        .filter(|(dist, _)| *dist <= PICK_DISTANCE)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, id)| id)
}

/// A right click on doodad `obj_id` at `now` (`Time::elapsed_secs`): one use is made at once, several open the interaction
/// list (`true`); out of reach the character walks to it first ([`walk_to`]). A new target (or a click that is already
/// in reach) drops the old walk, whose held W key nothing else would release.
fn right_click(d: &mut HudData, doodads: &Doodads, units: &Units, obj_id: u32, now: f32, dev: &mut crate::ingame::DevOptions) -> bool {
    let Some(o) = doodads.by_id.get(&obj_id) else { return false };
    let Some(pos) = doodad_world(o, units) else { return false };
    let Some(game) = d.game.get() else { return false };
    let Some(uses) = game.doodad.uses(o.func_group_id).cloned() else { return false };
    let name = game.doodad.name(o.template_id).unwrap_or("?").to_string();
    let dist = d.pos.distance(pos);
    let reach = reach(d, &uses);
    d.doodad.preferred = Some(obj_id);
    if d.doodad.approach.take().is_some() {
        dev.script_keys.retain(|c| c != 'W');
    }
    if dist > reach {
        info!("doodad: {name} ({obj_id}) is {dist:.1} m away (reach {reach:.1} m): walking there");
        d.doodad.approach = Some(Approach { obj_id, best: dist, best_at: now });
        return false;
    }
    let mut entries: Vec<Entry> = uses
        .skills
        .iter()
        .map(|s| Entry { id: i64::from(*s), name: game.skills.get(s).map(|i| i.named.name.clone()).unwrap_or_else(|| format!("skill {s}")) })
        .collect();
    entries.extend(uses.navi.iter().map(|n| Entry { id: NAVI_ID + *n as i64, name: n.label().into() }));
    if entries.len() == 1 {
        d.doodad.menu = Some((obj_id, entries.clone()));
        d.doodad.chosen.push(entries[0].id);
        return false;
    }
    info!("doodad: {name} ({obj_id}) offers {:?}", entries.iter().map(|e| &e.name).collect::<Vec<_>>());
    d.doodad.menu = Some((obj_id, entries));
    true
}

/// The uses the bar, the keys and the list asked for become packets.
#[allow(clippy::too_many_arguments)]
fn execute(
    bridge: NonSend<HudBridge>, doodads: Res<Doodads>, mut inventory: ResMut<Inventory>,
    seats: Option<Res<crate::ingame::seating::Seats>>, player: Option<Res<aa_net::state::Player>>,
    mount: Option<Res<crate::ingame::LocalMount>>, views: Res<Views>, mut out: MessageWriter<NetCommand>,
    mut failed: MessageWriter<super::alerts::UseFailed>,
) {
    let mut d = bridge.0.borrow_mut();
    if std::mem::take(&mut d.doodad.coffer_close) && inventory.coffer.is_open() {
        let obj_id = inventory.coffer.obj_id;
        info!("doodad: coffer {obj_id} closed");
        out.write(NetCommand(Command::Coffer(CofferCommand::Interact { obj_id, open: false })));
        inventory.coffer.close();
    }
    let execute = if d.dynamic_actions.shown().is_some_and(|s| matches!(s.source, super::dynamic_actions::Source::Doodad { .. })) {
        d.dynamic_actions.take_requested()
    } else {
        Vec::new()
    };
    let chosen = std::mem::take(&mut d.doodad.chosen);
    let mut commands = Vec::new();
    for pick in execute {
        let super::dynamic_actions::Source::Doodad { obj_id, phase, generation } = pick.source else { continue };
        let current = doodads.by_id.get(&obj_id).is_some_and(|o| o.func_group_id == phase)
            && views.doodad_generation(obj_id) == Some(generation)
            && d.doodad.focus.as_ref().is_some_and(|f| f.obj_id == obj_id && f.skills.get(pick.index - 1) == Some(&pick.skill));
        if current {
            commands.push((obj_id, Command::UseDoodadSkill { skill_id: pick.skill, doodad_obj_id: obj_id }));
        }
    }
    if !chosen.is_empty()
        && let Some((obj_id, _)) = d.doodad.menu.take()
    {
        for id in chosen {
            let command = match id - NAVI_ID {
                x if x == Navi::Teleport as i64 => Command::Interact(Interact::NaviTeleport { doodad_obj_id: obj_id }),
                x if x == Navi::OpenPortal as i64 => Command::Interact(Interact::NaviOpenPortal { doodad_obj_id: obj_id }),
                x if x == Navi::Coffer as i64 => Command::Coffer(CofferCommand::Interact { obj_id, open: true }),
                _ => Command::UseDoodadSkill { skill_id: id as u32, doodad_obj_id: obj_id },
            };
            commands.push((obj_id, command));
        }
    }
    // No bag-space check: every herb gather of the 2026-10-30 capture ran with a full 50/50 bag. The use was sent, the
    // cast ran and the phase changed to 4514; the server only added `SCErrorMsg` BAG_FULL after it
    // (`tools/evidence/gather/capture-results.txt`).
    let game = d.game.get();
    for (obj_id, command) in commands {
        if let Command::UseDoodadSkill { skill_id, .. } = &command {
            if lacks_labor(game.map(|g| &g.rules), *skill_id, d.snapshot.labor) {
                debug!(
                    "doodad: skill {skill_id} needs more labor than {}",
                    d.snapshot.labor
                );
                failed.write(super::alerts::UseFailed {
                    skill: *skill_id,
                    result: super::usable::code::NEED_LABOR_POWER,
                    value: 0,
                    server: false,
                });
                continue;
            }
            let group = doodads.by_id.get(&obj_id).map(|o| o.func_group_id);
            let mount_free = player.as_ref().and_then(|p| p.spawn.as_ref())
                .is_some_and(|spawn| spawn.unit.attached_to.is_none())
                && mount.as_ref().is_none_or(|mount| mount.0.is_none());
            let bond_free = seats.as_ref().is_some_and(|seats| seats.own_free());
            if group.zip(game).is_some_and(|(group, game)| !game.doodad.allows_attachment(group, *skill_id, bond_free, mount_free)) {
                debug!("doodad: attachment use {skill_id} requires known-free bond and mount");
                continue;
            }
        }
        let name = doodads.by_id.get(&obj_id).and_then(|o| game.and_then(|g| g.doodad.name(o.template_id))).unwrap_or("?");
        info!("doodad: use {name} ({obj_id}): {command:?}");
        out.write(NetCommand(command));
    }
}

/// The coffer window follows the coffer: it opens when the first contents arrive (`SHOW_COFFER_INVEN`), redraws when they change
/// (`COFFER_UPDATE`) and closes (`INTERACTION_END`) when the character walks off, as the server only serves the coffer within
/// [`COFFER_RANGE`].
fn coffer_window(rt: Option<NonSend<UiRuntime>>, bridge: NonSend<HudBridge>, inventory: Res<Inventory>, doodads: Res<Doodads>, units: Res<Units>, mut shown: Local<u32>) {
    let Some(rt) = rt else { return };
    let open = inventory.coffer.obj_id;
    if open != *shown {
        *shown = open;
        if open != 0 {
            info!("doodad: coffer {open} opens ({} slots)", inventory.coffer.slots.len());
            rt.fire_event("SHOW_COFFER_INVEN", ());
        }
    } else if open != 0 && inventory.is_changed() {
        rt.fire_event("COFFER_UPDATE", (-1i64, -1i64));
    }
    if open == 0 {
        return;
    }
    let far = doodads
        .by_id
        .get(&open)
        .and_then(|o| doodad_world(o, &units))
        .is_none_or(|pos| bridge.0.borrow().pos.distance(pos) > COFFER_RANGE);
    if far {
        rt.fire_event("INTERACTION_END", ());
        bridge.0.borrow_mut().doodad.coffer_close = true;
    }
}

/// What the server made of a use, for the log: the cast at a doodad, the doodad's next phase, a new recall point.
fn results(mut events: MessageReader<aa_net::NetEvent>, player: Res<aa_net::state::Player>, doodads: Res<Doodads>) {
    let own = player.spawn.as_ref().map_or(0, |s| s.obj_id);
    for aa_net::NetEvent(e) in events.read() {
        match e {
            aa_net::Event::SkillStarted(s) if s.caster.obj_id == own && s.target.kind == aa_net::skill::target_kind::DOODAD => {
                info!("doodad: cast {} at doodad {} (result {}, {} ms)", s.skill_id, s.target.obj_id, s.result, s.cast_ms);
            }
            aa_net::Event::DoodadPhaseChanged { obj_id, func_group_id, time_left, .. } if doodads.by_id.contains_key(obj_id) => {
                debug!("doodad: {obj_id} is in phase {func_group_id} now ({time_left} ms left)");
            }
            aa_net::Event::Portal(aa_net::portal::PortalEvent::ReturnDistricts { portals, return_point_id }) => {
                info!("doodad: recall point is {return_point_id} ({} districts in the book)", portals.len());
            }
            _ => {}
        }
    }
}

/// `SCDoodadPhaseMsg`: the phase's `phase_msg` as `DOODAD_PHASE_MSG(text)` (the native handler 391dc720 ->
/// 3902e0f0 resolves the message and fires the event the chat scripts print). `is_msg_to_zone` messages show even when
/// the doodad is not known; the others only for a doodad the client has.
fn phase_messages(
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<HudBridge>,
    doodads: Res<Doodads>,
    mut events: MessageReader<aa_net::NetEvent>,
) {
    let Some(rt) = rt else { return };
    // the Lua handler may call back into the HUD: collect first, then fire without the borrow
    let texts: Vec<(u32, u32, String)> = {
        let data = bridge.0.borrow();
        let Some(game) = data.game.get() else { return };
        events
            .read()
            .filter_map(|aa_net::NetEvent(event)| {
                let aa_net::Event::DoodadPhaseMsg { obj_id, message_id } = event else {
                    return None;
                };
                let group = u32::try_from(*message_id).ok()?;
                let text = game.doodad.message(group)?;
                if !game.doodad.zone_message(group) && !doodads.by_id.contains_key(obj_id) {
                    return None;
                }
                Some((*obj_id, group, text.to_owned()))
            })
            .collect()
    };
    for (obj_id, group, text) in texts {
        info!("doodad: {obj_id} phase message {group}: {text}");
        rt.fire_event("DOODAD_PHASE_MSG", text);
    }
}

/// The world left: nothing is in reach.
fn reset(bridge: NonSend<HudBridge>) {
    let mut data = bridge.0.borrow_mut();
    data.doodad = View::default();
    data.dynamic_actions = Default::default();
}

pub(super) fn plugin(app: &mut App) {
    app.insert_resource(Scripted::from_args())
        .add_systems(
            Update,
            (track.in_set(TrackDoodads), coffer_window, results, phase_messages)
                .chain()
                .after(crate::ingame::seating::track)
                .after(crate::ingame::units::sync_views)
                .after(super::sync_hud)
                .after(aa_ui::UiInput)
                .before(super::dialog::talk_input)
                .run_if(super::hud_up)
                .run_if(in_state(crate::Screen::InGame)),
        )
        .add_systems(Update, (pointer_use, execute).chain().after(super::PointerFinished).after(track).before(super::dialog::talk_pointer).run_if(super::hud_up).run_if(in_state(crate::Screen::InGame)))
        .add_systems(OnExit(crate::Screen::InGame), reset);
}

/// A world doodad as `SCDoodadsCreated` announces it (tests): unowned, unparented, in phase `func_group_id`.
#[cfg(test)]
fn world_doodad(
    obj_id: u32,
    template_id: u32,
    func_group_id: u32,
    pos: Vec3,
) -> aa_net::model::Doodad {
    aa_net::model::Doodad {
        obj_id,
        template_id,
        owner_obj_id: 0,
        parent_obj_id: 0,
        attach_point: 255,
        pos: aa_net::Pos::new(pos.x, pos.y, pos.z),
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: 1.0,
        has_loot_item: false,
        func_group_id,
        owner_id: 0,
        ucc_id: 0,
        item_template_id: 0,
        type2: 0,
        time_left: 0,
        plant_time: 0,
        quest_glow: 0,
        family: 0,
        puzzle_group: -1,
        owner_type: 255,
        owner_db_id: 0,
        data: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hud::alerts::UseFailed;
    use crate::hud::data::{GameData, SkillInfo};
    use crate::hud::dynamic_actions::{Shown, Source};
    use crate::hud::usable::{self, Rules};

    #[test]
    fn phases_list_their_skills_and_navi_uses() {
        let mut t = Tables::default();
        t.add(13066, 14602, "DoodadFuncBinding");
        t.add(13066, 14602, "DoodadFuncBinding");
        t.add(13067, 0, "DoodadFuncTimer");
        t.add(1351, 0, "DoodadFuncNaviTeleport");
        t.add(1351, 0, "DoodadFuncNaviOpenPortal");
        t.add(1351, 0, "DoodadFuncNaviNaming");
        t.add(7851, 15193, "DoodadFuncNaviTeleport");
        t.add(9, 10994, "DoodadFuncUse");
        assert_eq!(t.uses(13066), Some(&Uses { skills: vec![14602], navi: vec![] }));
        assert_eq!(t.uses(13067), None);
        assert_eq!(t.uses(1351), Some(&Uses { skills: vec![], navi: vec![Navi::Teleport, Navi::OpenPortal] }));
        assert_eq!(t.uses(7851), Some(&Uses { skills: vec![15193], navi: vec![] }));
        assert_eq!(t.uses(9), None, "the bar hides skill 10994");
    }

    #[test]
    fn a_safe_opens_without_a_skill() {
        let mut t = Tables::default();
        t.add(5602, 0, "DoodadFuncCoffer");
        assert_eq!(t.uses(5602), Some(&Uses { skills: vec![], navi: vec![Navi::Coffer] }));
        assert_eq!(Navi::Coffer.label(), "Open");
    }

    #[test]
    fn the_walk_stops_well_inside_the_reach() {
        assert_eq!(stop_distance(4.0), 3.0);
        assert_eq!(stop_distance(3.0), 2.0);
        assert_eq!(stop_distance(1.0), 1.5, "never closer than this");
        assert_eq!(stop_distance(30.0), 3.0, "never further than this");
    }

    #[test]
    fn a_skill_reach_is_its_max_range() {
        // 2026-10-30 1.2 capture: Gather 13990 (`skills.max_range` 4) has the bar at 3 m and not at 4.5 m.
        assert_eq!(skill_reach(4.0), 4.0);
        assert!(skill_reach(4.0) > 3.0 && skill_reach(4.0) < 4.5);
        assert_eq!(
            skill_reach(0.0),
            BASE_REACH,
            "no skill range keeps the base reach"
        );
        assert_eq!(skill_reach(1.0), 2.0, "the shortest skill reach is 2 m");
    }

    #[test]
    fn a_doodad_skill_over_the_players_labor_is_refused_locally() {
        // 2026-10-30 1.2 capture: Gather 13990 (1 LP) at -2946 LP printed "Insufficient Labor Points."
        // and sent no CSStartSkill; the fake-use lamp 13166 costs no labor.
        let rules = super::super::usable::Rules::default()
            .with_labor(13990, 1)
            .with_labor(13166, 0);
        assert!(lacks_labor(Some(&rules), 13990, -2946));
        assert!(lacks_labor(Some(&rules), 13990, 0));
        assert!(!lacks_labor(Some(&rules), 13990, 1));
        assert!(!lacks_labor(Some(&rules), 13166, 0));
        assert!(
            !lacks_labor(None, 13990, 0),
            "unknown data is left to the server"
        );
    }

    #[test]
    fn a_doodad_skill_that_costs_no_labor_is_never_refused_for_labor() {
        // The refusal is about a cost the player cannot pay. A free skill has none, even below zero LP (the 2026-10-30
        // capture forced -2946 LP for the herb): the fake-use lamp 13166 costs 0 LP, and a skill the rules do not know
        // costs none the HUD knows of.
        let rules = Rules::default().with_labor(13990, 1).with_labor(13166, 0);
        assert!(!lacks_labor(Some(&rules), 13166, -2946));
        assert!(!lacks_labor(Some(&rules), 13166, -1));
        assert!(!lacks_labor(Some(&rules), 13166, 0));
        assert!(!lacks_labor(Some(&rules), 9, -2946), "unknown skill");
        assert!(
            lacks_labor(Some(&rules), 13990, -2946),
            "a skill with a cost is still refused"
        );
    }

    /// The capture's field spot: the player at (10990, 10120, 102.5) and the herb, world doodad 2075 of template 2274
    /// in its Gather phase 4513, 3 m north of it.
    fn herb_field(game: GameData) -> (HudData, Doodads) {
        let data = HudData::default();
        assert!(data.game.0.set(game).is_ok());
        let mut doodads = Doodads::default();
        let herb = world_doodad(2075, 2274, 4513, Vec3::new(10990.0, 10123.0, 102.5));
        assert!(doodads.apply(&aa_net::Event::DoodadsSpawned(vec![herb])));
        (data, doodads)
    }

    /// The skills the bar offers a player `metres` south of the herb.
    fn bar_skills(data: &mut HudData, doodads: &Doodads, metres: f32) -> Option<Vec<u32>> {
        data.pos = Vec3::new(10990.0, 10123.0 - metres, 102.5);
        find_focus(data, doodads, &Units::default()).map(|focus| focus.skills)
    }

    #[test]
    fn the_herb_bar_is_up_at_3_m_and_gone_at_4_5_m_and_8_m() {
        // 2026-10-30 1.2 capture: Gather (13990, `skills.max_range` 4) was offered at 3.0 m; at 4.5 m and 8.0 m the bar
        // was empty and F sent nothing (`tools/evidence/gather/capture-results.txt`). Before, the reach was 5 m.
        let mut game = GameData::default();
        game.doodad.add(4513, 13990, "DoodadFuncUse");
        game.skills.insert(
            13990,
            SkillInfo {
                max_range: 4,
                ..default()
            },
        );
        let (mut data, doodads) = herb_field(game);
        assert_eq!(bar_skills(&mut data, &doodads, 3.0), Some(vec![13990]));
        assert_eq!(bar_skills(&mut data, &doodads, 4.5), None);
        assert_eq!(bar_skills(&mut data, &doodads, 8.0), None);
    }

    #[test]
    fn the_real_herb_bar_and_labor_follow_the_capture() {
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        if !std::path::Path::new(&path).exists() {
            eprintln!("skip: no client database at {path}");
            return;
        }
        let game = GameData::load(&path).unwrap();
        assert_eq!(game.rules.labor_cost(13990), 1, "Gather costs 1 LP");
        assert_eq!(game.rules.labor_cost(13166), 0, "the lamp costs no labor");
        assert_eq!(game.skills[&13990].max_range, 4, "Gather reaches 4 m");
        let (mut data, doodads) = herb_field(game);
        assert_eq!(bar_skills(&mut data, &doodads, 3.0), Some(vec![13990]));
        assert_eq!(bar_skills(&mut data, &doodads, 4.5), None);
        assert_eq!(bar_skills(&mut data, &doodads, 8.0), None);
    }

    /// The bar's first button (F) is pressed for the herb 2075: what the HUD sends and refuses.
    fn press_gather(labor: i32, bag_full: bool) -> (Vec<Command>, Vec<UseFailed>) {
        let mut game = GameData::default();
        game.doodad.add(4513, 13990, "DoodadFuncUse");
        game.rules = Rules::default().with_labor(13990, 1);
        let bridge = Shared::default();
        let mut views = Views::default();
        views.fixture_doodad(2075, Entity::PLACEHOLDER);
        let generation = views.doodad_generation(2075).unwrap();
        {
            let mut d = bridge.borrow_mut();
            assert!(d.game.0.set(game).is_ok());
            d.snapshot.labor = labor;
            d.doodad.focus = Some(Focus {
                obj_id: 2075,
                skills: vec![13990],
                dist: 3.0,
            });
            d.dynamic_actions.replace(Some(Shown {
                source: Source::Doodad {
                    obj_id: 2075,
                    phase: 4513,
                    generation,
                },
                skills: vec![13990],
            }));
            d.dynamic_actions.request(1);
        }
        let mut doodads = Doodads::default();
        let herb = world_doodad(2075, 2274, 4513, Vec3::new(10990.0, 10123.0, 102.5));
        assert!(doodads.apply(&aa_net::Event::DoodadsSpawned(vec![herb])));
        let mut inventory = Inventory::default();
        if bag_full {
            inventory.bag_slots = 50;
            inventory.bag = (0..50)
                .map(|_| aa_net::model::Item::template_only(27264))
                .collect();
        }
        let mut app = App::new();
        app.insert_non_send(HudBridge(bridge))
            .insert_resource(doodads)
            .insert_resource(inventory)
            .insert_resource(views)
            .add_message::<NetCommand>()
            .add_message::<UseFailed>()
            .add_systems(Update, execute);
        app.update();
        let world = app.world_mut();
        let sent = world
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .collect::<Vec<_>>();
        let failed = world
            .resource_mut::<Messages<UseFailed>>()
            .drain()
            .collect();
        (sent.into_iter().map(|command| command.0).collect(), failed)
    }

    #[test]
    fn a_herb_gather_is_sent_even_with_a_full_bag() {
        // 2026-10-30 1.2 capture: every herb gather ran with a 50/50 bag. F still sent CSStartSkill 13990 at the herb,
        // the cast ran and the phase changed to 4514; the server only added SCErrorMsg BAG_FULL afterwards.
        let (commands, failed) = press_gather(7056, true);
        assert!(
            matches!(
                commands.as_slice(),
                [Command::UseDoodadSkill {
                    skill_id: 13990,
                    doodad_obj_id: 2075
                }]
            ),
            "{commands:?}"
        );
        assert!(failed.is_empty(), "{failed:?}");
    }

    #[test]
    fn a_herb_gather_without_labor_is_refused_and_not_sent() {
        // 2026-10-30 1.2 capture: at -2946 LP the same press printed "Insufficient Labor Points." and sent no
        // CSStartSkill (no cast started).
        let (commands, failed) = press_gather(-2946, true);
        assert!(commands.is_empty(), "{commands:?}");
        let [refusal] = failed.as_slice() else {
            panic!("{failed:?}");
        };
        assert_eq!(refusal.skill, 13990);
        assert_eq!(refusal.result, usable::code::NEED_LABOR_POWER);
        assert!(!refusal.server, "the client's own check");
    }

    #[test]
    fn dark_doodads_do_not_glow() {
        let mut t = Tables::default();
        t.dark.insert(7);
        assert!(!t.glows(7));
        assert!(t.glows(8));
    }

    #[test]
    fn the_template_columns_map_highlight_silhouette_and_decal() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE doodad_almighties (id INTEGER, use_target_highlight TEXT, use_target_silhouette TEXT, use_target_decal TEXT, target_decal_size REAL);
             CREATE TABLE doodad_funcs (id INTEGER, doodad_func_group_id INTEGER, func_skill_id INTEGER, actual_func_type TEXT, actual_func_id INTEGER);
             CREATE TABLE skills (id INTEGER);
             CREATE TABLE doodad_func_uses (id INTEGER, skill_id INTEGER);
             CREATE TABLE doodad_func_fake_uses (id INTEGER, fake_skill_id INTEGER);
             CREATE TABLE localized_texts (tbl_name TEXT, tbl_column_name TEXT, idx INTEGER, en_us TEXT);
             CREATE TABLE doodad_func_groups (id INTEGER, is_msg_to_zone TEXT, phase_msg TEXT);",
        )
        .unwrap();
        conn.execute_batch(
            "INSERT INTO doodad_almighties VALUES (7, 'f', 't', 'f', 0.0);
             INSERT INTO doodad_almighties VALUES (8, 't', 'f', 't', 2.5);
             INSERT INTO doodad_func_groups VALUES (11, 'f', 'raw one');
             INSERT INTO doodad_func_groups VALUES (12, 't', 'raw two');
             INSERT INTO localized_texts VALUES ('doodad_func_groups', 'phase_msg', 11, 'English one');
             INSERT INTO localized_texts VALUES ('doodad_func_groups', 'phase_msg', 12, '');",
        )
        .unwrap();
        let t = Tables::load(&conn).unwrap();
        assert!(!t.glows(7) && t.silhouettes(7) && t.decal_size(7).is_none());
        assert!(t.glows(8) && !t.silhouettes(8) && t.decal_size(8) == Some(2.5));
        assert_eq!(t.message(11), Some("English one"));
        assert_eq!(
            t.message(12),
            Some("raw two"),
            "an empty translation falls back to the raw text"
        );
        assert!(!t.zone_message(11) && t.zone_message(12));
    }

    #[test]
    fn the_plate_shows_the_name_owner_and_growth_seconds() {
        assert_eq!(
            plate(7, "Apple Tree".into(), Some("Farmer".into()), 3_600_000.0),
            Tip {
                obj_id: 7,
                name: "Apple Tree".into(),
                owner: Some("Farmer".into()),
                secs: 3600
            }
        );
        assert_eq!(
            plate(8, "Rock".into(), None, 999.0).secs,
            1,
            "the timer rounds up"
        );
        assert_eq!(plate(9, "Crate".into(), None, 0.0).secs, 0);
    }

    #[test]
    fn the_walk_keeps_running_until_it_is_inside_the_reach() {
        let start = advance(false, false, 10.0, 4.0, 10.0, 0.0, 0.0);
        assert_eq!(start.walk, Walk::Run);
        assert_eq!((start.best, start.best_at), (10.0, 0.0));
        let closer = advance(false, false, 6.0, 4.0, 10.0, 0.0, 1.0);
        assert_eq!(closer.walk, Walk::Run);
        assert_eq!((closer.best, closer.best_at), (6.0, 1.0));
        assert_eq!(
            advance(false, false, 3.0, 4.0, 6.0, 1.0, 5.0).walk,
            Walk::Arrived
        );
        assert_eq!(
            advance(false, true, 6.0, 4.0, 6.0, 1.0, 5.0).walk,
            Walk::Cancel,
            "the player took over"
        );
        assert_eq!(
            advance(true, false, 6.0, 4.0, 6.0, 1.0, 5.0).walk,
            Walk::Cancel,
            "the doodad is gone"
        );
        assert_eq!(
            advance(false, false, 5.9, 4.0, 6.0, 1.0, 4.0).walk,
            Walk::Run,
            "5.9 is progress"
        );
        assert_eq!(
            advance(false, false, 5.9, 4.0, 5.9, 4.0, 7.1).walk,
            Walk::Stuck,
            "no progress for 3.1 s"
        );
    }

    #[test]
    fn an_owner_is_found_by_id() {
        let units = Units::default();
        let mut names = aa_net::CharacterNames::default();
        names.by_id.insert(9, b"Farmer".to_vec());
        assert_eq!(owner_name(3, Some((3, "Me")), &units, &names).as_deref(), Some("Me"));
        assert_eq!(owner_name(9, Some((3, "Me")), &units, &names).as_deref(), Some("Farmer"));
        assert_eq!(owner_name(4, None, &units, &names), None);
    }

    #[test]
    fn a_ray_hits_what_it_passes_near() {
        // game (x, y, z) -> bevy (x, z, -y): a ray along bevy -z from the origin passes game (0, 10, 0)
        let ray = Ray3d::new(Vec3::ZERO, Dir3::NEG_Z);
        assert!(hit(ray, Vec3::new(0.0, 10.0, 0.5), 1.0).is_some_and(|a| (a - 10.0).abs() < 1e-3));
        assert!(hit(ray, Vec3::new(3.0, 10.0, 0.0), 1.0).is_none());
        assert!(hit(ray, Vec3::new(0.0, -10.0, 0.0), 1.0).is_none(), "behind the camera");
    }

    /// The server database at `AA_DB` (else `compact.sqlite3` in the working directory), read-only;
    /// `None` (the test skips) without one.
    fn real_db() -> Option<Connection> {
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let conn = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok();
        if conn.is_none() {
            eprintln!("skip: no client database at {path}");
        }
        conn
    }

    #[test]
    fn attachment_funcs_need_known_free_bond_and_mount_without_blocking_other_uses() {
        let Some(conn) = real_db() else { return };
        let tables = Tables::load(&conn).unwrap();
        for (group, skill) in [(1951, 20847), (16414, 20847), (22863, 20847), (22477, 26592), (23046, 26592)] {
            assert!(tables.uses(group).unwrap().skills.contains(&skill));
            for (bond_free, mount_free) in [(false, false), (false, true), (true, false), (true, true)] {
                assert_eq!(tables.allows_attachment(group, skill, bond_free, mount_free), bond_free && mount_free);
            }
        }
        assert!(tables.allows_attachment(13066, 14602, false, false), "ordinary Memory Tome use stays on its existing path");
    }

    #[test]
    fn the_real_tables_know_the_memory_tomes() {
        let Some(conn) = real_db() else { return };
        let t = Tables::load(&conn).unwrap();
        assert_eq!(t.uses(13066).map(|u| u.skills.clone()), Some(vec![14602]));
        assert_eq!(t.name(850), Some("Desireen Memory Tome"));
    }

    /// `find_focus` for a player at `player` among `doodads`; `skills` are (id, `max_range`) rows.
    fn focus_at(
        tables: Tables,
        skills: &[(u32, i64)],
        doodads: Vec<aa_net::model::Doodad>,
        player: Vec3,
    ) -> Option<Focus> {
        let mut game = GameData {
            doodad: tables,
            ..Default::default()
        };
        for &(id, max_range) in skills {
            game.skills.insert(
                id,
                SkillInfo {
                    max_range,
                    ..Default::default()
                },
            );
        }
        let mut data = HudData::default();
        let _ = data.game.0.set(game);
        data.pos = player;
        let mut state = Doodads::default();
        for doodad in doodads {
            state.by_id.insert(doodad.obj_id, doodad);
        }
        find_focus(&data, &state, &Units::default())
    }

    /// The capture's `tome_851` reading: standing at (15379.922, 13853.5, 159.6), 4.9 m south of the tome, the bar offered
    /// one action, skill 15857. That is the Practical Chair 0.7 m away (a `DoodadFuncAttachment`), not the tome (14602):
    /// chairs, beds and a mailbox stand around it (positions: `doodad_spawns.json`).
    #[test]
    fn the_capture_spot_at_tome_851_offers_the_chair_beside_it() {
        let mut tables = Tables::default();
        tables.add(302, 14602, "DoodadFuncBinding");
        tables.add(10870, 15857, "DoodadFuncAttachment");
        tables.add(10585, 15857, "DoodadFuncAttachment");
        tables.add(107, 0, "DoodadFuncNaviOpenMailbox");
        let doodads = vec![
            world_doodad(1, 851, 302, Vec3::new(15379.9, 13858.4, 159.6)),
            world_doodad(2, 4429, 10870, Vec3::new(15379.2, 13853.6, 159.6)),
            world_doodad(3, 4429, 10870, Vec3::new(15378.8, 13856.0, 159.6)),
            world_doodad(4, 4429, 10870, Vec3::new(15381.3, 13855.0, 159.6)),
            world_doodad(5, 4429, 10870, Vec3::new(15382.4, 13855.8, 159.6)),
            world_doodad(6, 4310, 10585, Vec3::new(15377.8, 13859.7, 165.1)),
            world_doodad(7, 320, 107, Vec3::new(15375.3, 13859.5, 159.1)),
        ];
        let focus = focus_at(
            tables,
            &[(14602, 4), (15857, 4)],
            doodads,
            Vec3::new(15379.922, 13853.5, 159.6),
        )
        .unwrap();
        assert_eq!((focus.obj_id, focus.skills), (2, vec![15857]));
    }

    /// The capture's other readings against the database the lane ran on (skips without one): the tomes' own use is 14602,
    /// so the 11400 read at the Desireen tome and the 15857 at the Crescent Throne tome are their neighbours' (the Mailbox's
    /// "Open Mailbox", which has no `func_skill_id`, and the chairs' and beds' "Sitting/Lying"); the decal portal's fake use
    /// (17557, `max_range` 4) and the dirt mound (no funcs) offer nothing at the 5+ m the capture stood; the herb's Gather
    /// phase is skill 13990.
    #[test]
    fn the_real_tables_explain_the_capture_readings() {
        let Some(conn) = real_db() else { return };
        let t = Tables::load(&conn).unwrap();
        let skills = |group| t.uses(group).map(|u| u.skills.clone());
        assert_eq!(skills(301), Some(vec![14602]), "tome 850");
        assert_eq!(skills(302), Some(vec![14602]), "tome 851");
        assert_eq!(skills(10870), Some(vec![15857]), "Practical Chair");
        assert_eq!(skills(10585), Some(vec![15857]), "Bed");
        assert_eq!(skills(4513), Some(vec![13990]), "Narcissus, Gather phase");
        assert_eq!(
            skills(11368),
            Some(vec![17557]),
            "decal portal 4608: its fake use's detail skill"
        );
        for group in [2590, 2591, 6157] {
            assert_eq!(skills(group), None, "Dirt Mound 1429, group {group}");
        }
        let text = |sql: &str| conn.query_row(sql, [], |r| r.get::<_, String>(0)).unwrap();
        let skill_name = |id: u32| {
            text(&format!(
                "SELECT en_us FROM localized_texts WHERE tbl_name = 'skills' AND tbl_column_name = 'name' AND idx = {id}"
            ))
        };
        assert_eq!(skill_name(11400), "Open Mailbox");
        assert_eq!(skill_name(15857), "Sitting/Lying");
        assert_eq!(skill_name(17557), "Enter Continental Worldgate");
        let portal_range = conn
            .query_row("SELECT max_range FROM skills WHERE id = 17557", [], |r| {
                crate::hud::data::any_int(r, 0)
            })
            .unwrap();
        assert_eq!(portal_range, 4);
        assert!(
            skill_reach(portal_range as f32) < 5.5,
            "out of reach at the 5.5 m the capture stood"
        );
        let mailbox_skill = conn
            .query_row(
                "SELECT func_skill_id FROM doodad_funcs WHERE doodad_func_group_id = 107",
                [],
                |r| r.get::<_, Option<i64>>(0),
            )
            .unwrap();
        assert_eq!(
            mailbox_skill, None,
            "the Mailbox's func has no skill: 11400 is native-only"
        );
    }

    /// The only four templates with `use_target_decal` (the continent "space gap" portals) have a ring at the client's default
    /// size and nothing but a `DoodadFuncFakeUse`: the bar lists its skill through the detail fallback of `gather.rs`, so
    /// their ring has a trigger (skips without a database). No 1.2 doodad sets `use_target_silhouette`.
    #[test]
    fn the_decal_portals_have_a_ring_and_a_fake_use_skill_on_the_bar() {
        let Some(conn) = real_db() else { return };
        let t = Tables::load(&conn).unwrap();
        assert_eq!(t.decals.len(), 4, "only four templates draw the ring");
        assert!(t.silhouettes.is_empty(), "no template is a silhouette");
        for (template, group, skill) in [
            (4608, 11368, 17557),
            (4610, 11373, 17558),
            (4611, 11372, 17556),
            (4612, 11370, 17541),
        ] {
            assert_eq!(t.decal_size(template), Some(0.0), "template {template}");
            assert_eq!(
                t.uses(group).map(|u| u.skills.clone()),
                Some(vec![skill]),
                "template {template}, group {group}"
            );
        }
    }

    /// Drives `walk_to` and `right_click` through an ECS app: the approach sequences need the real resources and the
    /// script-held key, not only the pure [`advance`] decision.
    #[derive(Default, Resource)]
    struct Harness {
        now: f32,
        moving: bool,
        player: Vec3,
        click: Option<u32>,
        /// Whether W was held right after a queued click and before the walk's frame.
        click_w: Vec<bool>,
        /// Doodads `walk_to` arrived at, one entry per arrival.
        acted: Vec<u32>,
    }

    fn harness_tick(
        bridge: NonSend<HudBridge>,
        doodads: Res<Doodads>,
        units: Res<Units>,
        mut dev: ResMut<crate::ingame::DevOptions>,
        mut harness: ResMut<Harness>,
        mut cameras: Query<&mut crate::ingame::camera::OrbitCamera>,
    ) {
        let mut d = bridge.0.borrow_mut();
        d.pos = harness.player;
        let now = harness.now;
        if let Some(id) = harness.click.take() {
            right_click(&mut d, &doodads, &units, id, now, &mut dev);
            harness.click_w.push(dev.script_keys.contains('W'));
        }
        if let Some(id) = walk_to(
            &mut d,
            &doodads,
            &units,
            now,
            harness.moving,
            &mut cameras,
            &mut dev,
        ) {
            right_click(&mut d, &doodads, &units, id, now, &mut dev);
            harness.acted.push(id);
        }
    }

    /// A Memory Tome (850) in phase 1, the one `approach_app` gives a use, `(x, y)` metres from the origin.
    fn test_doodad(obj_id: u32, x: f32, y: f32) -> aa_net::model::Doodad {
        world_doodad(obj_id, 850, 1, Vec3::new(x, y, 0.0))
    }

    fn w_held(app: &App) -> bool {
        app.world()
            .resource::<crate::ingame::DevOptions>()
            .script_keys
            .contains('W')
    }

    fn approach_app(doodads: Vec<aa_net::model::Doodad>) -> (App, Shared) {
        let mut tables = Tables::default();
        tables.add(1, 14602, "DoodadFuncUse");
        let game = GameData {
            doodad: tables,
            ..Default::default()
        };
        let data = HudData::default();
        let _ = data.game.0.set(game);
        let shared: Shared = std::rc::Rc::new(std::cell::RefCell::new(data));
        let mut state = Doodads::default();
        for doodad in doodads {
            state.by_id.insert(doodad.obj_id, doodad);
        }
        let mut app = App::new();
        app.insert_resource(state)
            .insert_resource(Units::default())
            .insert_resource(crate::ingame::DevOptions::default())
            .insert_resource(Harness::default())
            .insert_non_send(HudBridge(shared.clone()))
            .add_systems(Update, harness_tick);
        app.world_mut()
            .spawn(crate::ingame::camera::OrbitCamera::default());
        (app, shared)
    }

    #[test]
    fn a_second_approach_after_a_cancel_acts_once_at_arrival() {
        let (mut app, shared) = approach_app(vec![test_doodad(7, 0.0, 10.0)]);
        let step = |app: &mut App, now: f32, player: Vec3, moving: bool, click: Option<u32>| {
            {
                let mut h = app.world_mut().resource_mut::<Harness>();
                h.now = now;
                h.player = player;
                h.moving = moving;
                h.click = click;
            }
            app.update();
        };
        step(&mut app, 0.0, Vec3::ZERO, false, Some(7));
        assert!(shared.borrow().doodad.approach.is_some());
        assert!(w_held(&app));

        // the player moves: cancel, and the held key is released
        step(&mut app, 1.0, Vec3::ZERO, true, None);
        assert!(shared.borrow().doodad.approach.is_none());
        assert!(!w_held(&app));

        // walk again and arrive
        step(&mut app, 2.0, Vec3::ZERO, false, Some(7));
        assert!(shared.borrow().doodad.approach.is_some());
        step(&mut app, 3.0, Vec3::new(0.0, 8.0, 0.0), false, None);
        step(&mut app, 4.0, Vec3::new(0.0, 6.0, 0.0), false, None);
        step(&mut app, 5.0, Vec3::new(0.0, 2.9, 0.0), false, None);
        assert_eq!(
            app.world().resource::<Harness>().acted,
            vec![7],
            "the use follows the arrival"
        );
        assert!(shared.borrow().doodad.approach.is_none());
        assert!(!w_held(&app));
        assert_eq!(shared.borrow().doodad.chosen.len(), 1, "one single use");
        step(&mut app, 6.0, Vec3::new(0.0, 2.9, 0.0), false, None);
        assert_eq!(
            app.world().resource::<Harness>().acted,
            vec![7],
            "no second action"
        );
        assert_eq!(shared.borrow().doodad.chosen.len(), 1);
    }

    #[test]
    fn a_doodad_removed_while_approaching_does_not_act_and_releases_w() {
        let (mut app, shared) = approach_app(vec![test_doodad(7, 0.0, 10.0)]);
        {
            let mut h = app.world_mut().resource_mut::<Harness>();
            h.now = 0.0;
            h.click = Some(7);
        }
        app.update();
        assert!(shared.borrow().doodad.approach.is_some());
        assert!(w_held(&app));

        app.world_mut().resource_mut::<Doodads>().by_id.clear();
        app.world_mut().resource_mut::<Harness>().now = 1.0;
        app.update();
        assert!(shared.borrow().doodad.approach.is_none());
        assert!(!w_held(&app));
        assert!(app.world().resource::<Harness>().acted.is_empty());
        assert!(shared.borrow().doodad.chosen.is_empty());
    }

    #[test]
    fn re_targeting_releases_w_and_keeps_only_the_new_approach() {
        let (mut app, shared) =
            approach_app(vec![test_doodad(7, 0.0, 10.0), test_doodad(8, 10.0, 0.0)]);
        {
            let mut h = app.world_mut().resource_mut::<Harness>();
            h.now = 0.0;
            h.click = Some(7);
        }
        app.update();
        assert_eq!(shared.borrow().doodad.approach.map(|a| a.obj_id), Some(7));
        assert!(w_held(&app));

        {
            let mut h = app.world_mut().resource_mut::<Harness>();
            h.now = 1.0;
            h.click = Some(8);
        }
        app.update();
        assert_eq!(
            app.world().resource::<Harness>().click_w.last(),
            Some(&false),
            "the old walk's W was released"
        );
        assert_eq!(shared.borrow().doodad.approach.map(|a| a.obj_id), Some(8));
        assert!(w_held(&app), "the new walk holds W");
        assert!(app.world().resource::<Harness>().acted.is_empty());
        assert!(shared.borrow().doodad.chosen.is_empty());
    }
}
