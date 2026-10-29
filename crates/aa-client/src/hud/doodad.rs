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
//! * **hovering** a usable doodad shows its name (`DRAW_DOODAD_TOOLTIP({name})`).
//!
//! The cast bar, the cast animation and the result come from the server's answers like for any skill (`cast.rs`,
//! `ingame::combat`, `portal.rs` for the new recall point).
//!
//! F goes to the doodad only when no loot is in reach (F loots) and no NPC is nearer than it (F talks).
//!
//! Dev flags: `--doodad-key N T` presses `do_interaction_N` T seconds after the HUD is up; `--doodad-rclick T` right
//! clicks the nearest usable doodad (as if the pointer were on it).

use std::collections::{HashMap, HashSet};

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

/// Skills the dynamic action bar leaves out (`dynamic_action_bar.lua`, `ShoulPassShow`).
const HIDDEN_SKILLS: [u32; 2] = [0, 10994];
/// Keys of the dynamic action bar (`do_interaction_1` ... ) that use a doodad.
const KEYS: usize = 4;
/// Reach of a doodad use without a skill range, and the slack on top of a skill's `max_range` (the range is to the
/// doodad's origin, a big model is touched from further away), metres.
const BASE_REACH: f32 = 4.0;
const REACH_SLACK: f32 = 1.0;
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
    /// Attachment funcs require a known-free doodad bond and the separate mount gate.
    attachment_skills: HashSet<(u32, u32)>,
}

impl Tables {
    pub(super) fn load(conn: &Connection) -> rusqlite::Result<Self> {
        let mut t = Tables::default();
        let mut q = conn.prepare("SELECT doodad_func_group_id, func_skill_id, actual_func_type FROM doodad_funcs ORDER BY doodad_func_group_id, id")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            let group = super::data::any_int(r, 0)? as u32;
            let skill = super::data::any_int(r, 1)? as u32;
            let kind: Option<String> = r.get(2)?;
            t.add(group, skill, kind.as_deref().unwrap_or(""));
        }
        let mut q = conn.prepare("SELECT id, use_target_highlight FROM doodad_almighties")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            if !super::data::any_flag(r, 1)? {
                t.dark.insert(super::data::any_int(r, 0)? as u32);
            }
        }
        let mut q = conn.prepare("SELECT idx, en_us FROM localized_texts WHERE tbl_name = 'doodad_almighties' AND tbl_column_name = 'name' AND en_us <> ''")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            t.names.insert(super::data::any_int(r, 0)? as u32, r.get(1)?);
        }
        Ok(t)
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

    pub(super) fn name(&self, template_id: u32) -> Option<&str> {
        self.names.get(&template_id).map(String::as_str)
    }
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

/// The character walks to a doodad that was right clicked out of reach and uses it there (the 1.2 client's move to the
/// interaction target: `InteractionManager` keeps the pending use, the character runs to the doodad, the use follows).
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

/// The reach of a doodad whose phase has `uses`, metres.
fn reach(data: &HudData, uses: &Uses) -> f32 {
    let range = data.game.get().and_then(|g| uses.skills.first().and_then(|s| g.skills.get(s))).map_or(0.0, |s| s.max_range as f32);
    if range > 0.0 { range.max(2.0) + REACH_SLACK } else { BASE_REACH }
}

fn to_vec(p: aa_net::Pos) -> Vec3 {
    Vec3::new(p.x, p.y, p.z)
}

/// The doodad in reach whose phase has a skill to use that the bar is about: the one last right clicked, else the
/// nearest, a doodad behind the player counting up to [`BEHIND`] metres further away than one in front (an inn's
/// chairs around the Memory Tome).
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

/// Finds the doodad in reach, shows / hides the bar, reads the keys, the right click and the pointer.
#[allow(clippy::too_many_arguments)]
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
    (mut glows, names, views, visuals): (ResMut<DoodadHighlights>, Res<aa_net::CharacterNames>, Res<Views>, Query<&DoodadVisual>),
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
    // the tooltip: name, owner and the growth time left
    let tip = hover.and_then(|id| doodads.by_id.get(&id)).map(|o| {
        let name = d.game.get().and_then(|g| g.doodad.name(o.template_id)).unwrap_or("?").to_string();
        let own_char = player.spawn.as_ref().map(|s| (s.state.character.id, s.state.character.name.as_str()));
        let owner = (o.owner_id != 0).then(|| owner_name(o.owner_id, own_char, &units, &names)).flatten();
        if o.owner_id != 0 && owner.is_none() && asked.insert(o.owner_id) {
            out.write(NetCommand(Command::QueryCharacterName { character_id: own_char.map_or(0, |c| c.0), requested_id: o.owner_id }));
        }
        let secs = views.doodad_entity(o.obj_id).and_then(|e| visuals.get(e).ok()).map_or(0, |v| (v.remaining_ms / 1000.0).ceil() as u32);
        Tip { obj_id: o.obj_id, name, owner, secs }
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
) {
    let mut data = bridge.0.borrow_mut();
    let clicked = data.doodad.pointer_pick.take();
    let clicked = (!super::blocks_look_pointer(&data, MouseButton::Right))
        .then_some(clicked)
        .flatten();
    let arrived = data.doodad.arrived.take();
    let menu = clicked
        .or(arrived)
        .is_some_and(|id| right_click(&mut data, &doodads, &units, id, time.elapsed_secs()));
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

/// One frame of the walk to a doodad that was right clicked out of reach: the character runs towards it (the camera
/// turns to it and W is held, like the script runner's `walk`) until it is near enough, and the doodad is returned to be
/// used. A key of the player's, a doodad that is gone, no progress for [`STUCK_SECS`] or a use that is in reach already
/// end the walk.
fn walk_to(d: &mut HudData, doodads: &Doodads, units: &Units, now: f32, moving: bool, cameras: &mut Query<&mut crate::ingame::camera::OrbitCamera>, dev: &mut crate::ingame::DevOptions) -> Option<u32> {
    let Some(mut a) = d.doodad.approach else { return None };
    let stop = |dev: &mut crate::ingame::DevOptions| dev.script_keys.retain(|c| c != 'W');
    let target = doodads.by_id.get(&a.obj_id).and_then(|o| doodad_world(o, units));
    let (Some(target), false) = (target, moving) else {
        stop(dev);
        d.doodad.approach = None;
        return None;
    };
    let dist = d.pos.distance(target);
    let reach = d.game.get().zip(doodads.by_id.get(&a.obj_id)).and_then(|(g, o)| g.doodad.uses(o.func_group_id)).map_or(BASE_REACH, |u| reach(d, u));
    if dist <= stop_distance(reach) {
        stop(dev);
        d.doodad.approach = None;
        return Some(a.obj_id);
    }
    if dist < a.best - 0.3 {
        (a.best, a.best_at) = (dist, now);
    }
    if now - a.best_at > STUCK_SECS {
        stop(dev);
        d.doodad.approach = None;
        info!("doodad: stopped {dist:.1} m from {} (no progress)", a.obj_id);
        // close enough for the server's range check: use it from here
        return (dist <= reach).then_some(a.obj_id);
    }
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
/// list (`true`); out of reach the character walks to it first ([`walk_to`]).
fn right_click(d: &mut HudData, doodads: &Doodads, units: &Units, obj_id: u32, now: f32) -> bool {
    let Some(o) = doodads.by_id.get(&obj_id) else { return false };
    let Some(pos) = doodad_world(o, units) else { return false };
    let Some(game) = d.game.get() else { return false };
    let Some(uses) = game.doodad.uses(o.func_group_id).cloned() else { return false };
    let name = game.doodad.name(o.template_id).unwrap_or("?").to_string();
    let dist = d.pos.distance(pos);
    let reach = reach(d, &uses);
    d.doodad.preferred = Some(obj_id);
    d.doodad.approach = None;
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
fn execute(
    bridge: NonSend<HudBridge>, doodads: Res<Doodads>, mut inventory: ResMut<Inventory>,
    seats: Option<Res<crate::ingame::seating::Seats>>, player: Option<Res<aa_net::state::Player>>,
    mount: Option<Res<crate::ingame::LocalMount>>, views: Res<Views>, mut out: MessageWriter<NetCommand>,
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
    let game = d.game.get();
    for (obj_id, command) in commands {
        if let Command::UseDoodadSkill { skill_id, .. } = &command {
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
            (track, coffer_window, results)
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn dark_doodads_do_not_glow() {
        let mut t = Tables::default();
        t.dark.insert(7);
        assert!(!t.glows(7));
        assert!(t.glows(8));
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

    #[test]
    fn attachment_funcs_need_known_free_bond_and_mount_without_blocking_other_uses() {
        let conn = Connection::open_with_flags(aa_anim::db::DEFAULT_DB, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
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
        let Ok(conn) = Connection::open_with_flags(aa_anim::db::DEFAULT_DB, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) else { return };
        let t = Tables::load(&conn).unwrap();
        assert_eq!(t.uses(13066).map(|u| u.skills.clone()), Some(vec![14602]));
        assert_eq!(t.name(850), Some("Desireen Memory Tome"));
    }
}
