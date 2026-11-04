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
//! `func_skill_id` (the Narcissus' Gather 13990, a chair's 15857). The candidate distance is the client setting
//! `max_interaction_doodad_distance` that `FUN_3902f010` passes to its range predicate ([`DOODAD_REACH`]), not the
//! skill's `max_range`: the capture bounds it at 3.0 m in and 4.5 m out for Gather (whose `max_range` is also 4),
//! and storage skill 15307 (`max_range` 20) must still clear at the same distance.
//! The lamp 2319's fake-use skill 13166 (offered by the original too) comes from the func's own detail row (`gather.rs`),
//! as do the decal portals' 17557 (not seen: the capture stood 5.5 m from one, outside its 4 m reach). The Mailbox's
//! 11400 "Open Mailbox" is the native func-type mapping of `DoodadFuncNaviOpenMailbox`; its use opens the mail window
//! locally and sends nothing (`local.rs`).
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

use crate::ingame::data::Db;
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

mod fish;
mod gather;
mod local;

/// Skills the dynamic action bar leaves out (`dynamic_action_bar.lua`, `ShoulPassShow`).
const HIDDEN_SKILLS: [u32; 2] = [0, 10994];
/// The action id of a `DoodadFuncCraftStart` / `DoodadFuncCraftPack` phase in the shared action
/// list. Native `FUN_393bd240` stores the func id as the action key; a bench has no skill, so the
/// port uses a sentinel above the skill range and resolves its icon separately. The workbench
/// callback (`FUN_393b7ba0` -> `FUN_394dc360`) is the local open, not a skill cast.
pub(super) const CRAFT_ACTION: u32 = 0xffff_fffe;
/// `doodad_func_permission` ids (native `FUN_393b3300`). The 1.2 data uses every value except 9;
/// the craft rows are 0, 1, 6, 7 and 8.
pub(super) mod craft_perm {
    pub const ANY: u8 = 0;
    pub const OWNER: u8 = 1;
    pub const OWNER_OR_FRIEND: u8 = 2;
    pub const EXPEDITION: u8 = 3;
    pub const OWNER_OR_PARTY: u8 = 4;
    pub const OWNER_OR_RAID: u8 = 5;
    pub const SAME_ACCOUNT: u8 = 6;
    pub const SAME_NATION: u8 = 7;
    pub const ZONE_RESIDENTS: u8 = 8;
}
/// Keys of the dynamic action bar (`do_interaction_1` ... ) that use a doodad.
const KEYS: usize = 4;
/// The dynamic action bar's candidate distance (`FUN_3902f010`), as a **centre distance**,
/// independent of the skill's `max_range`. The 2026-10-30 original-client capture bounds it:
/// Gather 13990 (`max_range` 4) is offered at 3.0 m and absent at 4.5 m, and storage skill 15307
/// (`max_range` 20) must still clear at the doodad distance. The surface gap below is only the
/// ordering key, so a non-zero player radius does not push the prompt past 4 m. `FUN_3902f010`
/// passes `max_interaction_doodad_distance` to its range predicate, whose default is the 2.1 m of
/// [`INTERACTION_DISTANCE`] measured as a surface gap to the doodad's shape; how that maps to this
/// capture-bounded centre distance is not resolved (the bar's own shape distance is not ported).
pub(super) const DOODAD_REACH: f32 = 4.0;
/// `max_interaction_doodad_distance` (`x2game` config +0x2a0, default f32 2.0999999046325684 at `0x399a7660`):
/// the reach of the right-click doodad use, as the 3D surface gap (`FUN_39093800` requires the offset strictly
/// greater than the geometry distance, so 2.1 m is already out). The skill's `max_range` is not the use's reach.
const INTERACTION_DISTANCE: f32 = 2.099_999_9;
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
/// `ErrorMessageType` 0x9a (`FUN_39428370`): the pending use is still out of reach when the walk
/// gives up.
const TOO_FAR_AWAY: i16 = 0x9a;
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
    /// The `perm_id` of every `DoodadFuncCraftStart` / `DoodadFuncCraftPack` func in this phase,
    /// in func order. Native `FUN_393bd290` checks each func's permission separately; the
    /// workbench window opens locally (native `FUN_394dc360`) and the recipe list is the
    /// template's static CraftStart/CraftPack lists (`FUN_394dc210`). `FUN_398a9480` checks a
    /// recipe against that template-wide union (every function group on the `doodad_almighties`
    /// row), not only the current phase; `crafting::data::Tables::benches` is built the same way.
    pub craft: Vec<u8>,
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
    /// `doodad_almighties.id` -> simulation radius in metres (`max(sim_radius / 1000, 0.1)`, the same
    /// shape radius `ingame::data::Db` uses; the native `FUN_3989bf90` uses the doodad's real collider).
    radii: HashMap<u32, f32>,
    /// `doodad_func_groups.id` -> the phase message shown by `DOODAD_PHASE_MSG`.
    messages: HashMap<u32, String>,
    /// The phase messages with `is_msg_to_zone`: shown even when the doodad itself is not known.
    zone_messages: HashSet<u32>,
    /// Attachment funcs require a known-free doodad bond and the separate mount gate.
    attachment_skills: HashSet<(u32, u32)>,
    /// `(doodad_func_group_id, skill)` -> the fish-stand func and its `actual_func_id`.
    fish_funcs: HashMap<(u32, u32), fish::FishFunc>,
    /// `doodad_func_buy_fish_id` / `doodad_func_convert_fish_id` -> accepted item templates.
    fish_buy_items: HashMap<u32, HashSet<u32>>,
    fish_convert_items: HashMap<u32, HashSet<u32>>,
    /// `fish_details.item_id` -> `max_weight` (the refund denominator).
    fish_max_weights: HashMap<u32, f32>,
    /// `(doodad_func_group_id, action id)` -> a window the client opens itself and its permission.
    locals: HashMap<(u32, u32), local::LocalUse>,
    /// `(doodad_func_group_id, action id)` -> `DoodadFuncInsertCounter` requirement and permission.
    insert_counters: HashMap<(u32, u32), local::CounterUse>,
}

impl Tables {
    pub(super) fn load(conn: &Connection) -> rusqlite::Result<Self> {
        let mut t = Tables::default();
        let detail = gather::DetailSkills::load(conn)?;
        let local = local::LocalFuncs::load(conn);
        let mut q = conn.prepare("SELECT id, doodad_func_group_id, func_skill_id, actual_func_type, actual_func_id, perm_id, next_phase FROM doodad_funcs ORDER BY doodad_func_group_id, id")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            let row_id = super::data::any_int(r, 0)? as u32;
            let group = super::data::any_int(r, 1)? as u32;
            let skill = super::data::any_int(r, 2)? as u32;
            let kind: Option<String> = r.get(3)?;
            let func_id = super::data::any_int(r, 4)? as u32;
            let perm = super::data::any_int(r, 5)? as u8;
            let next_phase = super::data::any_int(r, 6)? as i32;
            t.add_resolved_local(
                group,
                skill,
                kind.as_deref().unwrap_or(""),
                func_id,
                row_id,
                perm,
                next_phase,
                &detail,
                &local,
            );
        }
        (t.fish_buy_items, t.fish_convert_items, t.fish_max_weights) = fish::load_items(conn);
        let mut q = conn.prepare("SELECT id, use_target_highlight, use_target_silhouette, use_target_decal, target_decal_size, sim_radius FROM doodad_almighties")?;
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
            t.radii.insert(id, (any_float(r, 5)? * 0.001).max(0.1));
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
    #[cfg(test)]
    fn add_resolved(
        &mut self,
        group: u32,
        skill: u32,
        kind: &str,
        func_id: u32,
        perm: u8,
        detail: &gather::DetailSkills,
    ) {
        self.add_resolved_local(group, skill, kind, func_id, func_id, perm, 0, detail, &local::LocalFuncs::default());
    }

    #[allow(clippy::too_many_arguments)]
    fn add_resolved_local(
        &mut self,
        group: u32,
        skill: u32,
        kind: &str,
        func_id: u32,
        row_id: u32,
        perm: u8,
        next_phase: i32,
        detail: &gather::DetailSkills,
        local: &local::LocalFuncs,
    ) {
        let skill = if skill != 0 {
            skill
        } else if kind == "DoodadFuncNaviOpenMailbox" {
            // Native FUN_397aa970 maps the func type to 11400 "Open Mailbox"; the func row itself
            // has no `func_skill_id`.
            local::MAILBOX_ACTION
        } else if kind == "DoodadFuncEvidenceItemLoot" {
            // Native FUN_393b7eb0 case 0x34 reads detail+8, the row's `skill_id`.
            local.evidence_action(func_id).unwrap_or(0)
        } else {
            detail.skill(kind, func_id).unwrap_or(0)
        };
        self.add_fish_func(group, skill, kind, func_id);
        self.add_full(group, skill, kind, perm, func_id, row_id, next_phase, local);
    }

    #[cfg(test)]
    fn add(&mut self, group: u32, skill: u32, kind: &str) {
        self.add_full(group, skill, kind, 0, 0, 0, 0, &local::LocalFuncs::default());
    }

    #[allow(clippy::too_many_arguments)]
    fn add_full(&mut self, group: u32, skill: u32, kind: &str, perm: u8, func_id: u32, row_id: u32, next_phase: i32, local_funcs: &local::LocalFuncs) {
        if skill != 0 && kind == "DoodadFuncAttachment" {
            self.attachment_skills.insert((group, skill));
        }
        let navi = match kind {
            "DoodadFuncNaviTeleport" => Some(Navi::Teleport),
            "DoodadFuncNaviOpenPortal" => Some(Navi::OpenPortal),
            "DoodadFuncCoffer" => Some(Navi::Coffer),
            _ => None,
        };
        // Native `FUN_394dc210` collects recipes from `DoodadFuncCraftStart` (kind 0x0c) and
        // `DoodadFuncCraftPack` (kind 0x3a); both open the same window through `FUN_394dc360`.
        let craft = matches!(kind, "DoodadFuncCraftStart" | "DoodadFuncCraftPack").then_some(perm);
        if skill == 0 && navi.is_none() && craft.is_none() {
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
        if let Some(perm) = craft {
            uses.craft.push(perm);
        }
        if skill != 0 {
            if let Some(action) = local_funcs.local(kind, func_id, row_id, next_phase) {
                self.locals
                    .insert((group, skill), local::LocalUse { action, perm });
            }
            if kind == "DoodadFuncInsertCounter"
                && let Some((item, count)) = local_funcs.insert_counter(func_id)
            {
                self.insert_counters.insert(
                    (group, skill),
                    local::CounterUse {
                        item,
                        count,
                        perm,
                    },
                );
            }
        }
    }

    /// What the phase `group` can be used for (`None`: nothing).
    pub(super) fn uses(&self, group: u32) -> Option<&Uses> {
        self.uses.get(&group).filter(|u| !u.skills.is_empty() || !u.navi.is_empty() || !u.craft.is_empty())
    }

    /// The window the client opens itself for action `action` of phase `group`, and its permission.
    pub(super) fn local(&self, group: u32, action: u32) -> Option<local::LocalUse> {
        self.locals.get(&(group, action)).copied()
    }

    /// The `DoodadFuncInsertCounter` requirement of action `action` and its permission.
    pub(super) fn insert_counter(&self, group: u32, action: u32) -> Option<local::CounterUse> {
        self.insert_counters.get(&(group, action)).copied()
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

    /// The doodad's simulation radius as the shape the interaction gap measures against (`sim_radius` millimetres,
    /// at least 0.1 m). Unknown templates keep a point.
    fn radius(&self, template_id: u32) -> f32 {
        self.radii.get(&template_id).copied().unwrap_or(0.0)
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
    /// The workbench action's profession icon (`actability_groups.icon_path`), when the phase
    /// offers a craft action; the dynamic bar resolves it for [`CRAFT_ACTION`].
    craft_icon: Option<String>,
    /// The native ordering key (see `slave::native_key`): lower sorts first.
    key: f32,
    /// Centre distance, metres (the HUD's NPC-talk priority uses it).
    dist: f32,
}

/// The context `FUN_393bb090` stores at `DAT_39aff990+0x140..0x14c` for `X2Trial.ReportCrime`:
/// doodad object id, the action/skill id, the func's next phase and the func row id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CrimeReport {
    pub obj_id: u32,
    pub skill_id: u32,
    pub next_phase: i32,
    pub func_id: u32,
}

/// One row of the interaction list.
#[derive(Clone, Debug, PartialEq)]
struct Entry {
    id: i64,
    name: String,
}

/// The local interaction `FUN_393b7b10(state, obj_id)` remembers: `DAT_3a17e6a0` and
/// `DAT_3a17e6a4`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct LocalState {
    pub state: u8,
    pub obj_id: u32,
}

#[derive(Default)]
pub(super) struct View {
    focus: Option<Focus>,
    /// The local player sits on a bonded seat doodad (`player+0x3b94`, `FUN_393635e0`
    /// `BondLazyDoodad`): 1.2 builds no interaction indicator candidate then, so neither the
    /// doodad bar nor the slave Grab Machine prompt is shown.
    pub(super) seated: bool,
    /// The profession icon of the craft action the bar currently shows (see [`CRAFT_ACTION`]).
    pub(super) craft_icon: Option<String>,
    /// F goes to the doodad (it is nearer than any NPC).
    claims_key: bool,
    /// The interaction list: the doodad and its rows.
    menu: Option<(u32, Vec<Entry>)>,
    /// The list was shown to the player (`INTERACTION_START`), not only carried as a single
    /// action: another interaction (`FUN_393b7b10`) must close it first.
    menu_shown: bool,
    /// The ids picked from the list.
    chosen: Vec<i64>,
    /// The doodad under the pointer (its name tooltip is up).
    hover: Option<u32>,
    /// A right click on the climb doodad under the pointer when it is the nearest doodad and in
    /// grab reach: `(distance, id)`, handled by the dialog's unit comparison.
    climb_click: Option<(f32, u32)>,
    /// What the doodad tooltip shows now.
    tip: Option<Tip>,
    /// The doodad last right clicked: the bar is about it while it is in reach.
    preferred: Option<u32>,
    /// The walk to a doodad that was right clicked out of reach.
    approach: Option<Approach>,
    /// The use that is waiting for that walk to arrive.
    pending: Option<Pending>,
    /// The walk stopped with no progress while the pending use was still out of the measured
    /// reach: pointer_use answers it with TOO_FAR_AWAY (0x9a).
    too_far: bool,
    /// `obj_id` -> the faction from `SCDoodadOriginator` (native doodad+0x350; `FUN_393b3300` case 3).
    factions: HashMap<u32, u32>,
    /// The coffer window was closed.
    coffer_close: bool,
    /// The `DoodadFuncEvidenceItemLoot` context for `X2Trial.ReportCrime`.
    pub(super) crime_report: Option<CrimeReport>,
    /// `X2Trial.ReportCrime` calls since the last `execute`: context and report text.
    pub(super) crime_reports: Vec<(CrimeReport, String)>,
    /// The interaction state (`DAT_3a17e6a0`) of an open local window (1, 2, 7, 8, 0xd, 0xe) and
    /// the object it belongs to (`DAT_3a17e6a4`, `FUN_393b7b10`); the next `end_interaction`
    /// (`FUN_393b7aa0`) ends it.
    pub(super) local_state: Option<LocalState>,
    /// Purchase dialogs confirmed since the last `execute`; each sends its native packet.
    pub(super) purchases: Vec<local::PurchaseOrder>,
    /// `obj_id` -> the drawn model's prefab-local Cry bounds, for the interaction gap (the native
    /// `FUN_3989bf90` uses the doodad's real collider; the model bounds are the recovered shape).
    bounds: HashMap<u32, (Vec3, Vec3)>,
    pointer_pick: Option<u32>,
    arrived: Option<u32>,
    /// `Houses::my_houses` tl_id -> the zone group its position resolves to, kept per own house so
    /// `doodad_func_permission` 8 survives the house leaving view and the world being unloaded
    /// (native `FUN_3932aaf0` walks the own house list `DAT_3a167ec4`).
    own_house_zones: HashMap<u16, u32>,
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

/// A doodad use that waits for the walk to arrive (native `FUN_393b3ec0` keeps the action through
/// `FUN_390a6af0`). The phase and view generation it was picked from are stored with it: a changed
/// phase or a recreated doodad on arrival must not run the old action.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Pending {
    obj_id: u32,
    action: i64,
    phase: u32,
    generation: Option<u64>,
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

/// Starts the walk to a doodad that was right clicked out of reach; a mount walk and a talk walk give way.
pub(super) fn start_walk(data: &mut HudData, obj_id: u32, dist: f32, now: f32) {
    super::pets::cancel_mount_walk(data);
    super::dialog::cancel_walk(data);
    data.doodad.pending = None;
    data.doodad.too_far = false;
    data.doodad.approach = Some(Approach {
        obj_id,
        best: dist,
        best_at: now,
    });
}

impl View {
    /// Ends a doodad walk because a mount walk takes over: no use follows. Returns whether a walk was running.
    pub(super) fn cancel_walk(&mut self) -> bool {
        self.pending = None;
        self.approach.take().is_some()
    }

    /// A doodad walk is running (it owns the scripted forward key).
    pub(super) fn walking(&self) -> bool {
        self.approach.is_some()
    }

    pub(super) fn has_menu(&self) -> bool { self.menu.is_some() }
    /// F (`do_interaction_1`) uses the doodad in reach instead of talking to an NPC.
    pub(super) fn claims_key(&self) -> bool {
        self.claims_key
    }
    /// The right-clicked climb doodad (nearest doodad, in grab reach): distance and id.
    pub(super) fn climb_click(&self) -> Option<(f32, u32)> {
        self.climb_click
    }

    /// The bar's F claim, for the dialog tests (`claims_key` is set by the doodad frame system).
    #[cfg(test)]
    pub(super) fn claim_key_for_test(&mut self) {
        self.claims_key = true;
    }
}

/// What the permission check can resolve from the world (`FUN_393b3300`): houses for the house
/// owner / account and the terrain zone map for the nation / resident cases.
#[derive(Clone, Copy, Default)]
pub(super) struct PermEnv<'a> {
    pub houses: Option<&'a aa_net::Houses>,
    pub world: Option<&'a aa_world::World>,
}

/// The local player's facts for `FUN_393b3300`.
struct PlayerFacts<'a> {
    character_id: u32,
    account_id: u32,
    expedition_id: u32,
    /// The player's nation (native `DAT_39d44b94`, the CharacterState faction).
    nation: u32,
    /// The received Dominion rows, owner expedition per zone group.
    dominion: &'a aa_net::Dominion,
    /// The received faction records, for the expedition -> nation lookup (`FUN_392f8180`).
    expeditions: &'a aa_net::Expeditions,
    /// `system_factions.id` -> `mother_id` from the game data: the static nation rows the
    /// received faction list does not carry.
    faction_mother: Option<&'a HashMap<u32, u32>>,
    party: &'a aa_net::Party,
    friends: &'a aa_net::Friends,
    /// Zone groups of the account's houses; `None` when the client cannot know them.
    house_zone_groups: Option<Vec<u32>>,
}

/// The doodad's facts for `FUN_393b3300`.
struct DoodadFacts {
    owner_id: u32,
    house_owner_id: u32,
    house_account_id: u32,
    /// `SCDoodadOriginator.new_faction` (native doodad+0x350).
    faction: u32,
    /// The zone group at the doodad; `None` when the world does not know it.
    zone_group: Option<u32>,
}

/// The zone group at a world position (the terrain's zone key through the zone table).
fn zone_group(data: &HudData, world: Option<&aa_world::World>, pos: Vec3) -> Option<u32> {
    let key = u32::from(world?.zone_at(pos)?);
    data.game.get()?.zones.get(&key).map(|z| z.0)
}

/// The house a doodad belongs to: `owner_db_id` is its database id, `parent_obj_id` its object.
fn house_of<'a>(o: &aa_net::model::Doodad, houses: Option<&'a aa_net::Houses>) -> Option<&'a aa_net::HouseSnapshot> {
    houses?
        .by_tl_id
        .values()
        .filter(|h| !h.demolished)
        .map(|h| &h.snapshot)
        .find(|h| (o.owner_db_id != 0 && h.db_id == o.owner_db_id) || (o.parent_obj_id != 0 && h.obj_id == o.parent_obj_id))
}

/// Keeps [`View::own_house_zones`] in step with the account's own houses: a zone group that
/// resolves once stays cached while the house is still owned, even after its unit leaves view
/// or the world it stands in is unloaded.
fn sync_own_house_zones(
    data: &mut HudData,
    houses: Option<&aa_net::Houses>,
    world: Option<&aa_world::World>,
) {
    let Some(houses) = houses else {
        data.doodad.own_house_zones.clear();
        return;
    };
    data.doodad
        .own_house_zones
        .retain(|tl, _| houses.my_houses.contains_key(tl));
    let Some(world) = world else { return };
    for (tl, house) in &houses.my_houses {
        if data.doodad.own_house_zones.contains_key(tl) {
            continue;
        }
        if let Some(zone) = zone_group(data, Some(world), to_vec(house.position())) {
            data.doodad.own_house_zones.insert(*tl, zone);
        }
    }
}

fn player_facts<'a>(data: &'a HudData, env: PermEnv<'a>) -> PlayerFacts<'a> {
    let house_zone_groups = env
        .houses
        .map(|_| data.doodad.own_house_zones.values().copied().collect());
    PlayerFacts {
        character_id: data.character_id,
        account_id: data.account_id,
        expedition_id: data.expedition.own_id(),
        nation: data.expedition.received().current_faction_id,
        dominion: data.expedition.dominion(),
        expeditions: data.expedition.received(),
        faction_mother: data.game.get().map(|game| &game.faction_mother),
        party: &data.party,
        friends: &data.friends,
        house_zone_groups,
    }
}

fn doodad_facts(data: &HudData, o: &aa_net::model::Doodad, pos: Vec3, env: PermEnv<'_>) -> DoodadFacts {
    let house = house_of(o, env.houses);
    DoodadFacts {
        owner_id: house.map_or(o.owner_id, |h| h.owner_id),
        house_owner_id: house.map_or(0, |h| h.owner_id),
        house_account_id: house.map_or(0, |h| h.account_id),
        faction: data.doodad.factions.get(&o.obj_id).copied().unwrap_or(0),
        zone_group: zone_group(data, env.world, pos),
    }
}

/// The nation a faction belongs to, the native `FUN_392f8180` lookup: the top of its `mother_id`
/// chain, from the received expedition records or the static `system_factions` rows. `None` when
/// neither source knows the faction.
fn nation_of(player: &PlayerFacts, faction: u32) -> Option<u32> {
    let mut id = faction;
    let mut budget =
        player.expeditions.factions.len() + player.faction_mother.map_or(0, HashMap::len) + 1;
    while budget > 0 {
        budget -= 1;
        let mother = match (
            player.expeditions.factions.get(&id),
            player.faction_mother.and_then(|rows| rows.get(&id)),
        ) {
            (Some(row), _) => row.mother_id,
            (None, Some(&mother)) => mother,
            (None, None) => return None,
        };
        if mother == 0 {
            return Some(id);
        }
        id = mother;
    }
    None
}

/// Whether the owner is in the same five-person party group as `me` (native `FUN_39308e40`: it
/// walks the player's five-slot block of the 50-slot raid list).
fn same_party_group(party: &aa_net::Party, me: u32, owner: u32) -> bool {
    if party.id == 0 || owner == 0 {
        return false;
    }
    if party.is_party {
        return party.member(owner).is_some();
    }
    match (party.member(me), party.member(owner)) {
        (Some(a), Some(b)) => a.slot / 5 == b.slot / 5,
        _ => false,
    }
}

/// `FUN_393b3300`: each craft func's `perm_id` is checked on its own. A house zone the client
/// cannot resolve yet is inferred as allowed; the server rechecks it on `CSExecuteCraft`.
fn craft_permitted(perm: u8, player: &PlayerFacts, doodad: &DoodadFacts) -> bool {
    use craft_perm::*;
    let owner = if doodad.owner_id != 0 { doodad.owner_id } else { doodad.house_owner_id };
    let me = player.character_id;
    match perm {
        ANY => true,
        OWNER => owner != 0 && owner == me,
        OWNER_OR_FRIEND => owner == me || (owner != 0 && player.friends.friend(owner).is_some()),
        // Native: the player must be in an expedition; a doodad with no faction (0) is open to it,
        // otherwise its `SCDoodadOriginator` faction must be the player's expedition id.
        EXPEDITION => player.expedition_id != 0 && (doodad.faction == 0 || doodad.faction == player.expedition_id),
        OWNER_OR_PARTY => owner == me || same_party_group(player.party, me, owner),
        OWNER_OR_RAID => owner == me || player.party.member(owner).is_some(),
        // Native `FUN_393b32c0`: the owner is me, or the doodad's house is my account's.
        SAME_ACCOUNT => {
            owner == me
                || (doodad.house_account_id != 0 && doodad.house_account_id == player.account_id)
        }
        // Native `FUN_393b3300` case 7 -> `FUN_39053360(zoneGroup, player nation, 0)`: the zone
        // group's Dominion owner expedition must belong to the player's nation. A zone, owner or
        // faction the client has not received is refused (the native record/lookup is missing).
        SAME_NATION => {
            let Some(zone) = doodad.zone_group.and_then(|zone| u16::try_from(zone).ok()) else {
                return false;
            };
            let Some(owner) = player.dominion.rows.get(&zone).map(|row| row.expedition_id) else {
                return false;
            };
            player.nation != 0 && owner != 0 && nation_of(player, owner) == Some(player.nation)
        },
        // "My house in this zone": the account has a house in the doodad's zone group. Unknown
        // house zones are inferred as allowed.
        ZONE_RESIDENTS => match (&player.house_zone_groups, doodad.zone_group) {
            (Some(groups), Some(zone)) => groups.contains(&zone),
            _ => true,
        },
        // Native's default refuses unknown permissions.
        _ => false,
    }
}

/// The 3D surface gap from the player to a doodad, the distance `FUN_39093800` compares against
/// [`INTERACTION_DISTANCE`], and whether it measured the drawn model's bounds (`true`) rather than
/// the `sim_radius` fallback (`false`). The native predicate takes the minimum over the doodad's
/// real colliders, which are not recovered; that fallback is a centre distance and is 0 for many
/// templates, so it must not be treated as the native geometry test.
fn interaction_gap(
    data: &HudData,
    doodads: &Doodads,
    units: &Units,
    obj_id: u32,
) -> Option<(f32, bool)> {
    let o = doodads.by_id.get(&obj_id)?;
    let pos = doodad_world(o, units)?;
    let scale = if o.scale > 0.0 { o.scale } else { 1.0 };
    // The native predicate measures to the doodad's real colliders. Use the drawn model's bounds when the
    // model is ready; the sim radius (often 0) is only a fallback while it is not, and for parented
    // doodads whose parent transform the HUD does not compose here.
    if o.parent_obj_id == 0
        && let Some(bounds) = data.doodad.bounds.get(&obj_id).copied()
    {
        let (transform, lo, hi) = crate::ingame::units::doodad_bounds_local(o, bounds, scale);
        let player = aa_world::cry_to_bevy(data.pos);
        // the doodad's own (oriented) frame: an axis-aligned world box is too lenient on a rotated model
        let local = transform.to_matrix().inverse().transform_point3(player);
        let nearest = transform.transform_point(local.clamp(lo, hi));
        return Some((player.distance(nearest) - data.player_radius, true));
    }
    let radius = data
        .game
        .get()
        .map_or(0.0, |g| g.doodad.radius(o.template_id) * scale);
    Some((
        super::slave::surface_gap(data.pos, data.player_radius, pos, radius),
        false,
    ))
}

/// `FUN_393b7e40` (774405): the interaction ended. A real measured gap at or past the 2.1 m reach
/// ends it; a fallback gap (no drawn bounds, a parented doodad) is a centre distance, and the
/// action was offered up to [`DOODAD_REACH`], so only a gap past that proves the player left. A
/// gone doodad always ends it. This is the close rule all the local interactions share.
pub(super) fn left_interaction(
    data: &HudData,
    doodads: &Doodads,
    units: &Units,
    obj_id: u32,
) -> bool {
    if !doodads.by_id.contains_key(&obj_id) {
        return true;
    }
    match interaction_gap(data, doodads, units, obj_id) {
        Some((gap, true)) => gap >= INTERACTION_DISTANCE,
        Some((gap, false)) => gap >= DOODAD_REACH,
        // The doodad exists but has no composed world position: `FUN_39093800` cannot measure it
        // either, and the interaction must not stay open forever.
        None => true,
    }
}

/// Closes the parts of the active interaction that live in the HUD data: the bench, a shown
/// interaction list and the coffer (state 5, `FUN_394d08c0` sends the `CSCofferInteraction` close
/// packet at the top of the next `execute`), and takes the local window state. The caller raises
/// the events once its borrow of the data ends (see [`finish_interaction`]).
fn end_windows(data: &mut HudData, inventory: &Inventory) -> (Option<LocalState>, bool) {
    let bench = super::crafting::close_bench(data);
    // The local window states FUN_393b7b10 ends (`DAT_3a17e6a0`: open paper 1, renew 2, stamp 7,
    // purchase 8, coffer permission 0xd, auction 0xe).
    let state = data.doodad.local_state.take();
    // Only a list that was shown (`other_interaction`) is an active interaction; a single action
    // kept in `menu` for the immediate-use path is not.
    let menu = std::mem::take(&mut data.doodad.menu_shown);
    if menu {
        data.doodad.menu = None;
    }
    let coffer = inventory.coffer.is_open();
    if coffer {
        data.doodad.coffer_close = true;
    }
    (state, bench || menu || coffer || state.is_some())
}

/// The event half of the native `FUN_393b7aa0` (774053): `NPC_INTERACTION_END` (0x6c) for states
/// 0xe/0xf/0x11 first, then `INTERACTION_END` (0xd2) once when something was active. The
/// store/bank that `store::end_interaction` closes raises its own `NPC_INTERACTION_END`.
fn finish_interaction(
    rt: Option<&UiRuntime>,
    bridge: &Shared,
    state: Option<LocalState>,
    windows: bool,
) -> bool {
    let store = rt.is_some_and(|rt| super::store::end_interaction(rt, bridge));
    let ended = windows || store;
    if let Some(rt) = rt {
        if state.is_some_and(|state| matches!(state.state, 0xe | 0xf | 0x11)) {
            rt.fire_event("NPC_INTERACTION_END", ());
        }
        if ended {
            rt.fire_event("INTERACTION_END", ());
        }
    }
    ended
}

/// `end_windows` once per `execute`: `FUN_393b7b10` ends the active interaction only before the
/// first new bench or coffer of the frame.
fn end_once(
    data: &mut HudData,
    inventory: &Inventory,
    done: &mut bool,
    state: &mut Option<LocalState>,
    windows: &mut bool,
) {
    if *done {
        return;
    }
    *done = true;
    let (taken, ended) = end_windows(data, inventory);
    *state = taken;
    *windows = ended;
}

/// Ends the interaction that is active before a new local window, the native `FUN_393b7aa0`
/// (774053): the bench and the shown interaction list, the coffer (state 5), the store/bank and
/// the local window state. Raises `NPC_INTERACTION_END` first for states 0xe/0xf/0x11, then
/// `INTERACTION_END` (0xd2) once when something was active. Returns the taken state.
pub(super) fn end_interaction(
    rt: Option<&UiRuntime>,
    bridge: &Shared,
    inventory: &Inventory,
) -> Option<LocalState> {
    let (state, windows) = {
        let mut data = bridge.borrow_mut();
        end_windows(&mut data, inventory)
    };
    finish_interaction(rt, bridge, state, windows);
    state
}

/// Whether a use must walk first: the gap measured the drawn model's real bounds and is out of the
/// 2.1 m reach. A fallback gap (no bounds yet, a parented doodad, a template with no sim radius) is
/// not the native collider shape, so it never defers: the use is sent or opened and the server
/// checks the reach.
fn needs_walk(data: &HudData, doodads: &Doodads, units: &Units, obj_id: u32) -> bool {
    interaction_gap(data, doodads, units, obj_id)
        .is_some_and(|(gap, real)| real && gap >= INTERACTION_DISTANCE)
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

/// The doodad in reach whose phase has a skill to use that the bar is about: the one last right clicked among
/// doodads, else the lowest native interaction key (`slave::native_key`: surface gap minus `1 + facing`, the same
/// key the slave producer uses and the one `FUN_3902ed20` orders the combined unit/doodad candidate set by).
/// The right-click preference is a port behaviour that only ranks doodads: it must not force the doodad past a
/// slave in [`arbitrate`], because 1.2 has one candidate set ordered by the native key. The capture's bar beside
/// tome 851 (a chair 0.7 m away, the tome 4.9 m ahead) was the chair's skill 15857; beside tome 850 (the Mailbox
/// 1.5 m away, the tome 2.7 m) it was 11400, the Mailbox's "Open Mailbox" (the func-type mapping of
/// `DoodadFuncNaviOpenMailbox`, see `local.rs`). The tomes' own 14602 was not observed. A doodad has no recovered collision shape, so its gap is
/// the centre distance minus the player's radius; the doodad's own radius is 0 here.
fn find_focus(data: &HudData, doodads: &Doodads, units: &Units, env: PermEnv<'_>) -> Option<Focus> {
    if data.doodad.seated {
        return None;
    }
    let game = data.game.get()?;
    let world = super::slave::replicated_pose(units);
    let from = super::slave::player_centre(data, &world);
    let forward = super::slave::player_forward(data, &world);
    let player = player_facts(data, env);
    let mut best: Option<(bool, f32, Focus)> = None;
    for o in doodads.by_id.values() {
        let Some(uses) = game.doodad.uses(o.func_group_id) else {
            continue;
        };
        let Some(pos) = doodad_world(o, units) else {
            continue;
        };
        let dist = from.distance(pos);
        if dist >= DOODAD_REACH {
            continue;
        }
        // Native `FUN_393bd290` checks each craft func's permission separately and lists the
        // action through the same shared action list, so the bar's candidate distance is the
        // same as a skill's; the 2.1 m use range is checked when the action is used.
        let facts = doodad_facts(data, o, pos, env);
        let craft = uses.craft.iter().any(|perm| craft_permitted(*perm, &player, &facts));
        // Native `FUN_393bd290` checks every func's permission before it lists the action
        // (`FUN_393b3300`); the local windows and the insert-counter check are funcs too.
        let mut skills = Vec::with_capacity(uses.skills.len() + 1);
        for skill in &uses.skills {
            let local = game.doodad.local(o.func_group_id, *skill);
            let counter = game.doodad.insert_counter(o.func_group_id, *skill);
            let permitted = local.is_none_or(|l| craft_permitted(l.perm, &player, &facts))
                && counter.is_none_or(|c| craft_permitted(c.perm, &player, &facts));
            if permitted {
                skills.push(*skill);
            }
        }
        if craft {
            skills.push(CRAFT_ACTION);
        }
        if skills.is_empty() {
            continue;
        }
        let gap = super::slave::surface_gap(from, data.player_radius, pos, 0.0);
        let key = super::slave::native_key(from, forward, pos, gap);
        let preferred = data.doodad.preferred == Some(o.obj_id);
        let focus = Focus {
            obj_id: o.obj_id,
            skills,
            craft_icon: craft.then(|| super::crafting::bench_icon(data, o.template_id)).flatten(),
            key,
            dist,
        };
        let better = best
            .as_ref()
            .is_none_or(|(best_preferred, best_key, _)| match preferred.cmp(best_preferred) {
                std::cmp::Ordering::Greater => true,
                std::cmp::Ordering::Less => false,
                std::cmp::Ordering::Equal => key < *best_key,
            });
        if better {
            best = Some((preferred, key, focus));
        }
    }
    best.map(|(_, _, focus)| focus)
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
            if skill == CRAFT_ACTION {
                return Ok(d.doodad.craft_icon.clone().unwrap_or_else(|| "ui/icon/icon_skill_none.dds".into()));
            }
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
            let mut data = state.borrow_mut();
            data.doodad.menu_shown = false;
            data.doodad.chosen.push(id);
            Ok(())
        })?,
    )?;
    let prior: Option<mlua::Function> = interaction.get("CancelInteraction")?;
    let state = bridge.clone();
    interaction.set(
        "CancelInteraction",
        lua.create_function(move |_, args: mlua::MultiValue| {
            let mut data = state.borrow_mut();
            if data.doodad.menu.take().is_some() {
                data.doodad.menu_shown = false;
                return Ok(());
            }
            drop(data);
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
    lua.load(include_str!("coffer.lua")).set_name("@coffer.lua").exec()?;
    fish::install(lua)
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

/// The usable doodad the ray picks, unless an NPC is in front of it. Returns the distance along
/// the ray with the object id, so a climb doodad can be ordered against it.
fn pick(
    ray: Ray3d,
    data: &HudData,
    doodads: &Doodads,
    units: &Units,
    own: u32,
    world: &impl Fn(&aa_net::model::Unit) -> Option<Vec3>,
) -> Option<(f32, u32)> {
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
    (!npc_in_front(&ray, along, units, own, world)).then_some((along, obj_id))
}

/// Whether an NPC's pick-sphere middle is nearer along the ray than the doodad's middle (`along`).
/// Both are middles, so a vendor 0.5 m behind a stall does not hide it. `world` is the unit's
/// drawn world pose, so an NPC attached to a ship is tested where it is drawn, not at its
/// parent-local packet position.
fn npc_in_front(
    ray: &Ray3d,
    along: f32,
    units: &Units,
    own: u32,
    world: &impl Fn(&aa_net::model::Unit) -> Option<Vec3>,
) -> bool {
    units
        .iter()
        .filter(|u| u.obj_id != own && matches!(u.kind, aa_net::model::UnitKind::Npc { .. }))
        .any(|u| super::pick::unit_middle_hit(ray, u, world(u)).is_some_and(|a| a < along))
}

/// The bar the two interaction producers ask for: the one with the lower [`super::slave::native_key`]
/// (`FUN_3902ed20` orders the native unit/doodad set by it). A tie keeps the doodad: the native
/// tie-break is in `FUN_393bd9e0`, which is not decoded. The returned number is the chosen
/// candidate's centre distance, which F's NPC-talk priority compares against.
fn arbitrate(
    doodad: Option<(super::dynamic_actions::Shown, f32, f32)>,
    slave: Option<(super::dynamic_actions::Shown, f32, f32)>,
) -> (Option<super::dynamic_actions::Shown>, Option<f32>) {
    match (doodad, slave) {
        (Some((doodad, doodad_key, doodad_dist)), Some((slave, slave_key, slave_dist))) => {
            if slave_key < doodad_key {
                (Some(slave), Some(slave_dist))
            } else {
                (Some(doodad), Some(doodad_dist))
            }
        }
        (Some((doodad, _, dist)), None) => (Some(doodad), Some(dist)),
        (None, Some((slave, _, dist))) => (Some(slave), Some(dist)),
        (None, None) => (None, None),
    }
}

/// The climb doodad under the ray, tested along the whole `$ladder` segment. A ship ladder's
/// packet origin can be below the waterline while its rungs are at deck height, so a sphere at
/// the origin would never light it up. `segments` holds the drawn `(obj_id, bottom, top)` in
/// Bevy world space.
fn pick_climb(
    ray: Ray3d,
    data: &HudData,
    doodads: &Doodads,
    units: &Units,
    segments: &[(u32, Vec3, Vec3)],
) -> Option<(f32, u32)> {
    let mut best: Option<(f32, u32)> = None;
    for &(obj_id, a, b) in segments {
        let Some(pos) = doodads.by_id.get(&obj_id).and_then(|o| doodad_world(o, units)) else {
            continue;
        };
        if data.pos.distance(pos) > PICK_DISTANCE {
            continue;
        }
        let steps = ((a.distance(b) / 0.4).ceil() as usize).clamp(2, 24);
        for i in 0..=steps {
            let p = a.lerp(b, i as f32 / steps as f32);
            if let Some(along) = hit(ray, aa_world::bevy_to_cry(p), 0.45)
                && best.is_none_or(|(x, _)| along < x)
            {
                best = Some((along, obj_id));
            }
        }
    }
    best
}

/// The nearer of two ray hits, `(distance, obj_id)`; `None` when neither hit.
fn nearest_hit(a: Option<(f32, u32)>, b: Option<(f32, u32)>) -> Option<(f32, u32)> {
    match (a, b) {
        (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
        (a, b) => a.or(b),
    }
}

/// The system that decides the doodad in reach, the plate and the walk target; `ingame::doodad_affordance` draws the
/// target decal after it (it reads the `DoodadTargetDecal` this writes).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct TrackDoodads;

/// `player+0x3b94` (`FUN_393635e0` `BondLazyDoodad`): the local player sits on a bonded seat
/// doodad. 1.2 then builds no interaction indicator candidate, so [`track`] gates both the doodad
/// and the slave producer on [`View::seated`].
fn sync_seated(bridge: NonSend<HudBridge>, seats: Option<Res<crate::ingame::seating::Seats>>) {
    bridge.0.borrow_mut().doodad.seated = seats.as_ref().is_some_and(|seats| seats.own_seated());
}

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
    (
        mut glows,
        names,
        views,
        poses,
        mut boxes,
        visuals,
        mut target_decal,
        ladder_claim,
        db,
        ladder_visuals,
        houses,
        world_info,
    ): (
        ResMut<DoodadHighlights>,
        Res<aa_net::CharacterNames>,
        Res<Views>,
        super::pick::UnitPoses,
        super::pick::ModelBoxes,
        Query<&DoodadVisual>,
        ResMut<DoodadTargetDecal>,
        Option<Res<crate::ingame::ladder::LadderClaim>>,
        Option<Res<Db>>,
        Query<(&DoodadVisual, &GlobalTransform)>,
        Option<Res<aa_net::Houses>>,
        Option<Res<aa_world::stream::WorldInfo>>,
    ),
    mut out: MessageWriter<NetCommand>,
    mut asked: Local<std::collections::HashSet<u32>>,
) {
    let Some(rt) = rt else { return };
    let own = player.spawn.as_ref().map_or(0, |s| s.obj_id);
    let world = |u: &aa_net::model::Unit| poses.get(u, &units).map(|(pos, _)| pos);
    script.since += time.delta_secs();
    let mut d = bridge.0.borrow_mut();
    let env = PermEnv {
        houses: houses.as_deref(),
        world: world_info.as_deref().map(|w| w.0.as_ref()),
    };
    sync_own_house_zones(&mut d, houses.as_deref(), env.world);
    let focus = find_focus(&d, &doodads, &units, env);
    let slave = super::slave::candidate(&d, &units);
    // The bar: 1.2 puts units (`FUN_393601a0`) and doodads (`FUN_3902f010`) in one set keyed by
    // the same `native_key` (surface gap minus `1 + facing`, `FUN_3902ed20` orders it ascending).
    // The two producers are arbitrated on that key, not on centre distance: a Farm Wagon's four
    // storage boxes (doodads 3446 at attach points 9-12) are often nearer in centre terms but
    // behind or beside the player, while the wagon is in front.
    let doodad_wanted = focus.as_ref().and_then(|f| {
        let phase = doodads.by_id.get(&f.obj_id)?.func_group_id;
        let generation = views.doodad_generation(f.obj_id)?;
        Some((
            super::dynamic_actions::Shown {
                source: super::dynamic_actions::Source::Doodad {
                    obj_id: f.obj_id,
                    phase,
                    generation,
                },
                skills: f.skills.clone(),
            },
            f.key,
            f.dist,
        ))
    });
    let slave_wanted = slave.as_ref().map(|candidate| {
        (
            super::dynamic_actions::Shown {
                source: super::dynamic_actions::Source::Npc {
                    obj_id: candidate.obj_id,
                    template: candidate.template,
                },
                skills: candidate.skills.clone(),
            },
            candidate.key,
            candidate.dist,
        )
    });
    let (wanted, key_dist) = arbitrate(doodad_wanted, slave_wanted);
    let directing = d.dialog.directing;
    // The climb doodad owns F separately from the use bar: it blocks talk, loot and the bar
    // (`claims_key` stays the bar's own "the doodad is nearer than any NPC" claim).
    let ladder_owns_key = ladder_claim
        .as_deref()
        .is_some_and(|claim| claim.0.is_some());
    d.doodad.claims_key = key_dist.is_some_and(|dist| {
        !units.iter().any(|u| {
            u.obj_id != own
                && !u.dead
                && matches!(u.kind, aa_net::model::UnitKind::Npc { .. })
                && d.pos.distance(to_vec(u.pos)) < dist
        })
    });
    d.doodad.focus = focus;
    d.doodad.craft_icon = d.doodad.focus.as_ref().and_then(|f| f.craft_icon.clone());
    if !directing && d.dynamic_actions.replace(wanted.clone()) {
        drop(d);
        match &wanted {
            Some(super::dynamic_actions::Shown { source: super::dynamic_actions::Source::Doodad { obj_id, .. }, skills }) => {
                info!("doodad: {obj_id} in reach, uses {skills:?}");
                rt.fire_event("DYNAMIC_ACTION_BAR_SHOW", ("doodad", *obj_id));
            }
            Some(super::dynamic_actions::Shown {
                source: super::dynamic_actions::Source::Npc { obj_id, .. },
                skills,
            }) => {
                info!("slave: {obj_id} in reach, uses {skills:?}");
                rt.fire_event("DYNAMIC_ACTION_BAR_SHOW", ("npc", *obj_id));
            }
            None => rt.fire_event("DYNAMIC_ACTION_BAR_HIDE", ()),
        }
        d = bridge.0.borrow_mut();
    }
    // the keys
    let typing = focus_kb.0 || rt.keyboard_captured();
    let moving = !typing && MOVE_ACTIONS.iter().any(|a| actions.pressed(a, 0));
    let loot = d.loot.nearest.is_some();
    for n in 1..=KEYS {
        if !typing
            && !loot
            && !directing
            && !ladder_owns_key
            && d.doodad.claims_key
            && actions.just_pressed(&format!("do_interaction_{n}"), 0)
        {
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
    let ladder_segments: Vec<(u32, Vec3, Vec3)> = db.as_deref().map_or_else(Vec::new, |db| {
        doodads
            .by_id
            .values()
            .filter(|o| db.ladder_kind(o.func_group_id).is_some())
            .filter_map(|o| {
                let entity = views.doodad_entity(o.obj_id)?;
                let (visual, global) = ladder_visuals.get(entity).ok()?;
                let helper = visual.ladder?;
                Some((
                    o.obj_id,
                    global.transform_point(helper.bottom),
                    global.transform_point(helper.top),
                ))
            })
            .collect()
    });
    let climb = ray.and_then(|r| pick_climb(r, &d, &doodads, &units, &ladder_segments));
    let usable = ray.and_then(|r| pick(r, &d, &doodads, &units, own, &world));
    // the ladder must be the nearest doodad along the ray, not one behind a closer one
    let nearest = nearest_hit(climb, usable);
    // ... and it must be within grab reach: a far click falls through to the unit/doodad pick. The reach is
    // to the ladder's segment, like the grab's `nearest_ladder` (a ship ladder's origin can sit at the
    // waterline with its rungs at deck height); the doodad origin only stands in without a drawn helper.
    let ladder_click = climb.filter(|(_, id)| {
        let in_reach = match ladder_segments.iter().find(|(seg, ..)| seg == id) {
            Some((_, bottom, top)) => {
                crate::ingame::ladder::segment_point(
                    d.pos,
                    aa_world::bevy_to_cry(*bottom),
                    aa_world::bevy_to_cry(*top),
                )
                .0 <= crate::ingame::ladder::REACH
            }
            None => doodads
                .by_id
                .get(id)
                .and_then(|o| doodad_world(o, &units))
                .is_some_and(|p| d.pos.distance(p) <= crate::ingame::ladder::REACH),
        };
        nearest.is_some_and(|(_, n)| n == *id) && in_reach
    });
    let mut clicked = None;
    let mut climb_click = None;
    if !blocked && pointer.mouse.just_released(MouseButton::Right)
        && let (Some(start), Some(end)) = (pressed_at.take(), cursor)
        && start.distance(end) <= CLICK_SLOP
    {
        // The ladder grab is decided in the dialog against the unit/ship hit, so this only
        // records the ladder click. A climb doodad never goes to the doodad skill bar.
        climb_click = ladder_click;
        // a ladder click in grab reach is a new target action: a running talk or doodad walk must not keep
        // steering (and holding W) while the player takes the ladder
        if climb_click.is_some() && (d.doodad.cancel_walk() | super::dialog::cancel_walk(&mut d)) {
            dev.script_keys.retain(|c| c != 'W');
        }
        clicked =nearest.map(|(_, id)| id).filter(|id| {
            !doodads.by_id.get(id).is_some_and(|o| {
                db.as_deref()
                    .is_some_and(|db| db.ladder_kind(o.func_group_id).is_some())
            })
        });
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
    // Only a ladder is unit-ordered on click (a non-ladder doodad is used directly), so only a
    // ladder hover applies the unit/hull ordering. A ladder parented to the picked slave beats
    // that slave's loose box.
    let hover_is_ladder = nearest.is_some_and(|(_, id)| {
        db.as_deref().is_some_and(|db| {
            doodads
                .by_id
                .get(&id)
                .is_some_and(|o| db.ladder_kind(o.func_group_id).is_some())
        })
    });
    let ridden = player
        .spawn
        .as_ref()
        .and_then(|s| s.unit.attached_to.map(|a| a.obj_id));
    let unit_hit = if hover_is_ladder {
        ray.and_then(|r| {
            let (entry, id) = super::pick::pick_unit_hit(
                &r,
                &units,
                own,
                ridden,
                |u| {
                    matches!(
                        u.kind,
                        aa_net::model::UnitKind::Npc { .. }
                            | aa_net::model::UnitKind::Slave { .. }
                    )
                },
                |unit, ray| {
                    (
                        poses.get(unit, &units).map(|(pos, _)| pos),
                        boxes.get_near(unit, ray),
                    )
                },
            )?;
            let unit = units.get(id)?;
            let (at, model) = (
                poses.get(unit, &units).map(|(pos, _)| pos),
                boxes.get_near(unit, &r),
            );
            let along = super::pick::unit_compare_hit(&r, unit, at, model).unwrap_or(entry);
            Some((along, id))
        })
    } else {
        None
    };
    let hover = nearest
        .filter(|(_, id)| {
            if !hover_is_ladder {
                return true;
            }
            let parent = doodads
                .by_id
                .get(id)
                .map(|o| o.parent_obj_id)
                .filter(|parent| *parent != 0);
            super::pick::doodad_beats_unit(nearest, unit_hit, parent)
        })
        .map(|(_, id)| id)
        .or_else(|| {
            script
                .hovering
                .then(|| nearest_usable(&d, &doodads, &units))
                .flatten()
        });
    d.doodad.hover = hover;
    d.doodad.climb_click = climb_click;
    let arrived = walk_to(&mut d, &doodads, &units, time.elapsed_secs(), moving, &mut cameras, &mut dev);
    // The interaction gap measures to the drawn model's bounds; cache them for the doodads the pointer,
    // the bar or the walk is about (the clicked one may be far and not in `focus`).
    for id in [clicked, arrived, d.doodad.hover, d.doodad.preferred, d.doodad.focus.as_ref().map(|f| f.obj_id)].into_iter().flatten() {
        // a changed model or a reused obj_id must not keep the old model's bounds
        match views.doodad_bounds(id) {
            Some(bounds) => { d.doodad.bounds.insert(id, bounds); }
            None => { d.doodad.bounds.remove(&id); }
        }
    }
    d.doodad.bounds.retain(|id, _| doodads.by_id.contains_key(id));
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

#[allow(clippy::too_many_arguments)]
fn pointer_use(
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<HudBridge>,
    doodads: Res<Doodads>,
    units: Res<Units>,
    views: Res<Views>,
    time: Res<Time>,
    mut dev: ResMut<crate::ingame::DevOptions>,
    houses: Option<Res<aa_net::Houses>>,
    world_info: Option<Res<aa_world::stream::WorldInfo>>,
    mut errors: MessageWriter<super::alerts::ShowError>,
) {
    let mut data = bridge.0.borrow_mut();
    if std::mem::take(&mut data.doodad.too_far) {
        // FUN_393b3ec0's pending action never got into reach: FUN_39428370(0x9a).
        errors.write(super::alerts::ShowError(TOO_FAR_AWAY));
    }
    let clicked = data.doodad.pointer_pick.take();
    let clicked = (!super::blocks_look_pointer(&data, MouseButton::Right))
        .then_some(clicked)
        .flatten();
    let arrived = data.doodad.arrived.take();
    let env = PermEnv {
        houses: houses.as_deref(),
        world: world_info.as_deref().map(|w| w.0.as_ref()),
    };
    // A use that started the walk (an action out of the 2.1 m range) runs on arrival instead of
    // rebuilding the right-click decision (`FUN_390a6af0`'s pending action). The phase, the view
    // generation and the permission are re-checked: a doodad that changed on the way drops it.
    // A fresh click wins even on the frame the previous walk arrives. Otherwise the arrival
    // consumes its pending action below and swallows the new target.
    if clicked.is_some() && data.doodad.cancel_walk() {
        dev.script_keys.retain(|c| c != 'W');
    }
    let pending = if data
        .doodad
        .pending
        .is_some_and(|pending| Some(pending.obj_id) == arrived)
    {
        data.doodad.pending.take()
    } else {
        None
    };
    let menu = if let Some(pending) = pending {
        if pending_use_allowed(&data, &doodads, &units, &views, &pending, env) {
            data.doodad.menu = Some((pending.obj_id, Vec::new()));
            data.doodad.chosen.push(pending.action);
        } else {
            info!(
                "doodad: {:?} changed while walking, dropped",
                pending.obj_id
            );
        }
        false
    } else {
        clicked
            .or(arrived)
            .is_some_and(|id| right_click(&mut data, &doodads, &units, id, time.elapsed_secs(), &mut dev, env))
    };
    if menu {
        data.doodad.menu_shown = true;
    }
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
    /// The doodad is inside [`INTERACTION_DISTANCE`]: use it.
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
    stop: f32,
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
    let walk = if dist < stop {
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
pub(super) fn walk_to(
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
    let measured = interaction_gap(d, doodads, units, a.obj_id);
    let gap = measured.map_or(f32::INFINITY, |(gap, _)| gap);
    let fallback = measured.is_some_and(|(_, real)| !real);
    let step = advance(
        target.is_none(),
        moving,
        gap,
        INTERACTION_DISTANCE,
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
            d.doodad.pending = None;
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
            info!("doodad: stopped {gap:.1} m from {} (no progress)", a.obj_id);
            // close enough for the interaction reach: use it from here
            if gap < INTERACTION_DISTANCE {
                return Some(a.obj_id);
            }
            if d.doodad.pending.is_some() {
                if fallback {
                    // the sim-radius fallback cannot prove the doodad out of reach: run the use
                    return Some(a.obj_id);
                }
                d.doodad.pending = None;
                d.doodad.too_far = true;
            }
            return None;
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
fn right_click(d: &mut HudData, doodads: &Doodads, units: &Units, obj_id: u32, now: f32, dev: &mut crate::ingame::DevOptions, env: PermEnv<'_>) -> bool {
    let Some(o) = doodads.by_id.get(&obj_id) else { return false };
    let Some(pos) = doodad_world(o, units) else { return false };
    let (name, uses) = {
        let Some(game) = d.game.get() else { return false };
        let Some(uses) = game.doodad.uses(o.func_group_id).cloned() else { return false };
        (game.doodad.name(o.template_id).unwrap_or("?").to_string(), uses)
    };
    let (gap, real) = interaction_gap(d, doodads, units, obj_id).unwrap_or((f32::INFINITY, false));
    let dist = d.pos.distance(pos);
    d.doodad.preferred = Some(obj_id);
    // a doodad use is a new target action: a mount walk to the pet or a talk walk must not keep steering
    super::pets::cancel_mount_walk(d);
    super::dialog::cancel_walk(d);
    // a click that replaces a running doodad walk takes the forward key it was holding
    if d.doodad.cancel_walk() {
        dev.script_keys.retain(|c| c != 'W');
    }
    let Some(game) = d.game.get() else { return false };
    let player = player_facts(d, env);
    let facts = doodad_facts(d, o, pos, env);
    // `FUN_393bd290` lists only the funcs `FUN_393b3300` permits: the local windows and the
    // insert-counter check are funcs too.
    let mut listed_skills = 0;
    let mut entries: Vec<Entry> = Vec::with_capacity(uses.skills.len() + 1);
    for skill in &uses.skills {
        let local = game.doodad.local(o.func_group_id, *skill);
        let counter = game.doodad.insert_counter(o.func_group_id, *skill);
        let permitted = local.is_none_or(|l| craft_permitted(l.perm, &player, &facts))
            && counter.is_none_or(|c| craft_permitted(c.perm, &player, &facts));
        if !permitted {
            continue;
        }
        listed_skills += 1;
        entries.push(Entry {
            id: i64::from(*skill),
            name: game
                .skills
                .get(skill)
                .map(|i| i.named.name.clone())
                .unwrap_or_else(|| format!("skill {skill}")),
        });
    }
    // Each craft func's permission is checked on its own (native `FUN_393bd290` calls
    // `FUN_393b3300` per func); the action is listed at the bar's distance, not the 2.1 m use range.
    let craft = uses.craft.iter().any(|perm| craft_permitted(*perm, &player, &facts));
    if craft {
        entries.push(Entry {
            id: i64::from(CRAFT_ACTION),
            name: "Craft".into(),
        });
    }
    entries.extend(uses.navi.iter().map(|n| Entry { id: NAVI_ID + *n as i64, name: n.label().into() }));
    // A phase whose only use was refused offers nothing: never open an empty interaction list.
    if entries.is_empty() {
        return false;
    }
    // Native `FUN_393b3ec0` checks `FUN_39093800` for every action it is asked to run, so a phase
    // that offers a skill or a navi use walks to the doodad first when it is out of the 2.1 m
    // surface gap. Only the drawn model's bounds measure that gap; the sim-radius fallback (often
    // 0) is not the native collider shape and must not walk. A craft-only phase is listed out to
    // the bar's centre distance (`DOODAD_REACH`) and its use walks on its own; the two distances
    // are not compared with each other.
    let only_craft = listed_skills == 0 && uses.navi.is_empty();
    if real && gap >= INTERACTION_DISTANCE && (!only_craft || dist >= DOODAD_REACH) {
        info!("doodad: {name} ({obj_id}) is {gap:.1} m away (interaction reach {INTERACTION_DISTANCE:.1} m): walking there");
        start_walk(d, obj_id, gap, now);
        return false;
    }
    if entries.len() == 1 {
        d.doodad.menu = Some((obj_id, entries.clone()));
        d.doodad.chosen.push(entries[0].id);
        return false;
    }
    info!("doodad: {name} ({obj_id}) offers {:?}", entries.iter().map(|e| &e.name).collect::<Vec<_>>());
    d.doodad.menu = Some((obj_id, entries));
    true
}

/// Starts the walk to `obj_id` for `action` and keeps it as the pending use (native
/// `FUN_393b3ec0`: `FUN_39093800` out of range installs `FUN_390a6af0`'s pending action). The
/// phase and view generation are stored so a use that changed on the way is dropped.
#[allow(clippy::too_many_arguments)]
fn defer_action(
    d: &mut HudData,
    doodads: &Doodads,
    units: &Units,
    obj_id: u32,
    action: i64,
    phase: u32,
    generation: Option<u64>,
    now: f32,
) {
    let gap = interaction_gap(d, doodads, units, obj_id).map_or(f32::INFINITY, |(gap, _)| gap);
    info!("doodad: use {obj_id} ({action}) is {gap:.1} m away: walking there");
    start_walk(d, obj_id, gap, now);
    d.doodad.pending = Some(Pending {
        obj_id,
        action,
        phase,
        generation,
    });
}

/// Whether a pending use still matches the doodad on arrival: the phase and view generation it was
/// picked from, the phase still offers the skill, and a craft still passes a craft permission
/// (native `FUN_393b3ec0` runs the stored action; the checks the pick passed are re-run here).
fn pending_use_allowed(
    data: &HudData,
    doodads: &Doodads,
    units: &Units,
    views: &Views,
    pending: &Pending,
    env: PermEnv<'_>,
) -> bool {
    let Some(o) = doodads.by_id.get(&pending.obj_id) else {
        return false;
    };
    if o.func_group_id != pending.phase {
        return false;
    }
    if let Some(generation) = pending.generation
        && views.doodad_generation(pending.obj_id) != Some(generation)
    {
        return false;
    }
    let Some(game) = data.game.get() else {
        return false;
    };
    let Some(uses) = game.doodad.uses(o.func_group_id) else {
        return false;
    };
    if pending.action == i64::from(CRAFT_ACTION) {
        let Some(pos) = doodad_world(o, units) else {
            return false;
        };
        let player = player_facts(data, env);
        let facts = doodad_facts(data, o, pos, env);
        uses.craft
            .iter()
            .any(|perm| craft_permitted(*perm, &player, &facts))
    } else {
        let Some(pos) = doodad_world(o, units) else {
            return false;
        };
        let player = player_facts(data, env);
        let facts = doodad_facts(data, o, pos, env);
        u32::try_from(pending.action).is_ok_and(|skill| {
            uses.skills.contains(&skill)
                && game
                    .doodad
                    .local(o.func_group_id, skill)
                    .is_none_or(|local| craft_permitted(local.perm, &player, &facts))
                && game
                    .doodad
                    .insert_counter(o.func_group_id, skill)
                    .is_none_or(|counter| craft_permitted(counter.perm, &player, &facts))
        })
    }
}

/// Uses a workbench: opens it inside the 2.1 m range, else walks there and opens on arrival
/// (native `FUN_393b3ec0` checks `FUN_39093800`, then `FUN_390a6af0` keeps the pending action).
fn use_bench(
    d: &mut HudData,
    doodads: &Doodads,
    units: &Units,
    obj_id: u32,
    now: f32,
    generation: Option<u64>,
) -> Option<(u32, usize, bool)> {
    let o = doodads.by_id.get(&obj_id)?;
    if needs_walk(d, doodads, units, obj_id) {
        defer_action(
            d,
            doodads,
            units,
            obj_id,
            i64::from(CRAFT_ACTION),
            o.func_group_id,
            generation,
            now,
        );
        return None;
    }
    super::crafting::open_bench(d, obj_id, o.template_id).map(|(count, replaced)| (obj_id, count, replaced))
}

/// The world/house/seating state the doodad use permission needs; bundled to keep the execute
/// system under Bevy's parameter limit.
#[derive(bevy::ecs::system::SystemParam)]
pub(super) struct InteractionEnv<'w> {
    houses: Option<Res<'w, aa_net::Houses>>,
    world_info: Option<Res<'w, aa_world::stream::WorldInfo>>,
    seats: Option<Res<'w, crate::ingame::seating::Seats>>,
    target: Option<Res<'w, aa_net::state::Target>>,
}

/// The uses the bar, the keys and the list asked for become packets.
#[allow(clippy::too_many_arguments)]
pub(super) fn execute(
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<HudBridge>,
    doodads: Res<Doodads>,
    units: Res<Units>,
    mut inventory: ResMut<Inventory>,
    env_res: InteractionEnv,
    player: Option<Res<aa_net::state::Player>>,
    mount: Option<Res<crate::ingame::LocalMount>>,
    views: Res<Views>,
    poses: super::pick::UnitPoses,
    mut dev: ResMut<crate::ingame::DevOptions>,
    mut out: MessageWriter<NetCommand>,
    mut failed: MessageWriter<super::alerts::UseFailed>,
    time: Option<Res<Time>>,
    mut fish: Option<ResMut<fish::FishDialog>>,
    mut errors: Option<MessageWriter<crate::hud::ShowError>>,
) {
    let now = time.as_ref().map_or(0.0, |t| t.elapsed_secs());
    let target_id = env_res.target.as_ref().and_then(|t| t.obj_id).unwrap_or(0);
    let mut d = bridge.0.borrow_mut();
    // Dialog confirmations queued by the scripts: a purchase OK (`FUN_393bc4e0` /
    // `FUN_393bac20`) and `X2Trial.ReportCrime` both send here.
    let mut coin_notices: Vec<(String, i32)> = Vec::new();
    for (context, msg) in std::mem::take(&mut d.doodad.crime_reports) {
        out.write(NetCommand(Command::ReportCrime {
            obj_id: context.obj_id,
            skill_id: context.skill_id,
            next_phase: context.next_phase,
            func_id: context.func_id,
            msg,
        }));
    }
    for order in std::mem::take(&mut d.doodad.purchases) {
        match local::purchase_command(order, &inventory, d.game.get(), d.snapshot.money) {
            Ok(command) => {
                out.write(NetCommand(command));
            }
            Err(local::PurchaseRefusal::InvalidItem) => {
                if let Some(errors) = errors.as_mut() {
                    errors.write(crate::hud::ShowError(local::STORE_INVALID_ITEM));
                }
            }
            Err(local::PurchaseRefusal::NotEnoughMoney) => {
                if let Some(errors) = errors.as_mut() {
                    errors.write(crate::hud::ShowError(local::NOT_ENOUGH_MONEY));
                }
            }
            Err(local::PurchaseRefusal::UnknownCoin) => {
                if let Some(errors) = errors.as_mut() {
                    errors.write(crate::hud::ShowError(local::NOT_ENOUGH_ITEM));
                }
            }
            Err(local::PurchaseRefusal::NotEnoughCoin { coin_item, count }) => {
                let name = d
                    .game
                    .get()
                    .and_then(|g| g.items.get(&coin_item))
                    .map(|i| i.named.name.clone())
                    .unwrap_or_default();
                coin_notices.push((name, count));
            }
        }
    }
    if std::mem::take(&mut d.doodad.coffer_close) && inventory.coffer.is_open() {
        let obj_id = inventory.coffer.obj_id;
        info!("doodad: coffer {obj_id} closed");
        out.write(NetCommand(Command::Coffer(CofferCommand::Interact { obj_id, open: false })));
        inventory.coffer.close();
    }
    // Native `FUN_393b7b10` runs `FUN_393b7aa0` first when any interaction is active, not only
    // another bench: the shown interaction list and the coffer window are interactions too.
    let other_interaction = d.doodad.menu_shown || inventory.coffer.is_open();
    let execute = match d.dynamic_actions.shown().map(|s| s.source) {
        Some(super::dynamic_actions::Source::Doodad { .. })
        | Some(super::dynamic_actions::Source::Npc { .. }) => d.dynamic_actions.take_requested(),
        _ => Vec::new(),
    };
    let chosen = std::mem::take(&mut d.doodad.chosen);
    // A different bar or list pick cancels the pending open (native `FUN_393b3ec0` replaces the
    // stored action and `FUN_393b7b10` closes the active interaction): a use picked while a walk
    // to another action is running must not open the old one on arrival.
    if let Some(pending) = d.doodad.pending {
        let menu_obj = d.doodad.menu.as_ref().map(|menu| menu.0);
        let bar_differs = execute.iter().any(|pick| match pick.source {
            super::dynamic_actions::Source::Doodad { obj_id, .. } => {
                (obj_id, i64::from(pick.skill)) != (pending.obj_id, pending.action)
            }
            super::dynamic_actions::Source::Npc { .. } => true,
        });
        let chosen_differs = menu_obj.is_some_and(|obj_id| {
            obj_id != pending.obj_id || chosen.iter().any(|id| *id != pending.action)
        });
        if (bar_differs || chosen_differs) && d.doodad.cancel_walk() {
            dev.script_keys.retain(|c| c != 'W');
        }
    }
    let mut commands = Vec::new();
    let mut opens: Vec<(u32, usize, bool)> = Vec::new();
    for pick in execute {
        match pick.source {
            super::dynamic_actions::Source::Doodad {
                obj_id,
                phase,
                generation,
            } => {
                let current = doodads
                    .by_id
                    .get(&obj_id)
                    .is_some_and(|o| o.func_group_id == phase)
                    && views.doodad_generation(obj_id) == Some(generation)
                    && d.doodad.focus.as_ref().is_some_and(|f| {
                        f.obj_id == obj_id && f.skills.get(pick.index - 1) == Some(&pick.skill)
                    });
                if current && pick.skill == CRAFT_ACTION {
                    if let Some(open) =
                        use_bench(&mut d, &doodads, &units, obj_id, now, Some(generation))
                    {
                        opens.push(open);
                    }
                } else if current {
                    // Native `FUN_393b3ec0` walks for every action measured against the doodad's
                    // real colliders; a fallback gap sends the skill and lets the server check.
                    if !needs_walk(&d, &doodads, &units, obj_id) {
                        commands.push((
                            obj_id,
                            Command::UseDoodadSkill {
                                skill_id: pick.skill,
                                doodad_obj_id: obj_id,
                            },
                        ));
                    } else {
                        defer_action(
                            &mut d,
                            &doodads,
                            &units,
                            obj_id,
                            i64::from(pick.skill),
                            phase,
                            Some(generation),
                            now,
                        );
                    }
                }
            }
            super::dynamic_actions::Source::Npc { obj_id, .. } => {
                // The take validated the revision and the shown skill: `FUN_393b1de0` ->
                // `FUN_393be280`. 12076 is the driver bind (`CSBindSlave` 0x031); 13863 is the
                // plain passenger `CSStartSkill`. Other interaction skills are not ported.
                let outcome = match pick.skill {
                    12076 => {
                        // the bind range is measured from the drawn poses (a child slave on a ship)
                        let world = |id: u32| units.get(id).and_then(|u| poses.get(u, &units));
                        super::slave::bind(&mut d, &units, obj_id, &world)
                    }
                    13863 => super::slave::passenger(&mut d, &units, obj_id),
                    _ => super::slave::Outcome::Silent,
                };
                if let super::slave::Outcome::Refused(code) = outcome {
                    super::mode_actions::refuse(&mut d, code);
                }
            }
        }
    }
    if !chosen.is_empty()
        && let Some((obj_id, _)) = d.doodad.menu.take()
    {
        for id in chosen {
            if id == i64::from(CRAFT_ACTION) {
                let generation = views.doodad_generation(obj_id);
                if let Some(open) = use_bench(&mut d, &doodads, &units, obj_id, now, generation) {
                    opens.push(open);
                }
                continue;
            }
            let navi = match id - NAVI_ID {
                x if x == Navi::Teleport as i64 => Some(Command::Interact(Interact::NaviTeleport { doodad_obj_id: obj_id })),
                x if x == Navi::OpenPortal as i64 => Some(Command::Interact(Interact::NaviOpenPortal { doodad_obj_id: obj_id })),
                x if x == Navi::Coffer as i64 => Some(Command::Coffer(CofferCommand::Interact { obj_id, open: true })),
                _ => None,
            };
            if let Some(command) = navi {
                commands.push((obj_id, command));
                continue;
            }
            // An entry picked from the list is an action like any other: a measured gap out of
            // reach walks first (`FUN_393b3ec0`) and runs through the pending path.
            if u32::try_from(id).is_ok() && needs_walk(&d, &doodads, &units, obj_id) {
                let Some(o) = doodads.by_id.get(&obj_id) else { continue };
                defer_action(
                    &mut d,
                    &doodads,
                    &units,
                    obj_id,
                    id,
                    o.func_group_id,
                    views.doodad_generation(obj_id),
                    now,
                );
                continue;
            }
            commands.push((
                obj_id,
                Command::UseDoodadSkill {
                    skill_id: id as u32,
                    doodad_obj_id: obj_id,
                },
            ));
        }
    }
    // No bag-space check: every herb gather of the 2026-10-30 capture ran with a full 50/50 bag. The use was sent, the
    // cast ran and the phase changed to 4514; the server only added `SCErrorMsg` BAG_FULL after it
    // (`tools/evidence/gather/capture-results.txt`).
    if !opens.is_empty() {
        if other_interaction && inventory.coffer.is_open() {
            d.doodad.coffer_close = true;
        }
        d.doodad.menu_shown = false;
    }
    let game_slot = d.game.clone();
    let labor = d.snapshot.labor;
    drop(d);
    if let Some(rt) = rt.as_deref() {
        for (coin, count) in coin_notices {
            let text = super::alerts::localized_args(
                rt,
                "ERROR_MSG",
                "not_enough_coin",
                &[coin, count.to_string()],
            )
            .unwrap_or_else(|| "not_enough_coin".to_owned());
            rt.fire_event("SYSMSG", text);
        }
    }
    let game = game_slot.get();
    let env = PermEnv {
        houses: env_res.houses.as_deref(),
        world: env_res.world_info.as_deref().map(|w| w.0.as_ref()),
    };
    // The native `FUN_393b3300` gate, with the same player/doodad facts as the listing. `None`
    // means the doodad cannot be measured, so the listing would not offer it either: skip it
    // silently instead of showing a permission error.
    let permitted = |obj_id: u32, perm: u8| -> Option<bool> {
        let data = bridge.0.borrow();
        let player = player_facts(&data, env);
        doodads
            .by_id
            .get(&obj_id)
            .and_then(|o| doodad_world(o, &units).map(|pos| (o, pos)))
            .map(|(o, pos)| craft_permitted(perm, &player, &doodad_facts(&data, o, pos, env)))
    };
    for (obj_id, command) in commands {
        if let Command::UseDoodadSkill { skill_id, .. } = &command {
            let group = doodads.by_id.get(&obj_id).map(|o| o.func_group_id);
            // Native `FUN_393bc260` handles these funcs before `FUN_393b3ec0` sends the skill: the
            // original window opens locally and the server never sees the use.
            if let Some(local) = group
                .zip(game)
                .and_then(|(g, game)| game.doodad.local(g, *skill_id))
            {
                match permitted(obj_id, local.perm) {
                    Some(true) => {}
                    Some(false) => {
                        if let Some(errors) = errors.as_mut() {
                            errors.write(crate::hud::ShowError(local::NO_PERM));
                        }
                        continue;
                    }
                    None => continue,
                }
                let phase = local::run(
                    rt.as_deref(),
                    &bridge.0,
                    &doodads,
                    &units,
                    &inventory,
                    env_res.houses.as_deref(),
                    target_id,
                    obj_id,
                    *skill_id,
                    local.action,
                );
                if let Some(command) = phase {
                    out.write(NetCommand(command));
                }
                continue;
            }
            if let Some(counter) = group
                .zip(game)
                .and_then(|(g, game)| game.doodad.insert_counter(g, *skill_id))
            {
                match permitted(obj_id, counter.perm) {
                    Some(true) => {}
                    Some(false) => {
                        if let Some(errors) = errors.as_mut() {
                            errors.write(crate::hud::ShowError(local::NO_PERM));
                        }
                        continue;
                    }
                    None => continue,
                }
                // Native case 0x54 returns 1 with no local player: handled, not sent.
                if player.as_ref().is_none_or(|p| p.spawn.is_none()) {
                    continue;
                }
                if local::missing_counter_item(&inventory, counter.item, counter.count) {
                    debug!(
                        "doodad: skill {skill_id} needs {} of item {}",
                        counter.count, counter.item
                    );
                    if let Some(errors) = errors.as_mut() {
                        errors.write(crate::hud::ShowError(local::NOT_ENOUGH_ITEM));
                    }
                    continue;
                }
            }
            if lacks_labor(game.map(|g| &g.rules), *skill_id, labor) {
                debug!("doodad: skill {skill_id} needs more labor than {labor}");
                failed.write(super::alerts::UseFailed {
                    skill: *skill_id,
                    result: super::usable::code::NEED_LABOR_POWER,
                    value: 0,
                    server: false,
                });
                continue;
            }
            let mount_free = player.as_ref().and_then(|p| p.spawn.as_ref())
                .is_some_and(|spawn| spawn.unit.attached_to.is_none())
                && mount.as_ref().is_none_or(|mount| mount.0.is_none());
            let bond_free = env_res.seats.as_ref().is_some_and(|seats| seats.own_free());
            if group.zip(game).is_some_and(|(group, game)| !game.doodad.allows_attachment(group, *skill_id, bond_free, mount_free)) {
                debug!("doodad: attachment use {skill_id} requires known-free bond and mount");
                continue;
            }
            // Without the UI runtime (or the FishDialog resource) the skill is sent unconfirmed.
            let fishing = group.and_then(|group| game.and_then(|game| game.doodad.fish_func(group, *skill_id)));
            if let (Some(func), Some(rt), Some(dialog)) = (fishing, rt.as_deref(), fish.as_deref_mut()) {
                let game = game.expect("a fish func needs the game data");
                let items = game.doodad.fish_items(func.kind, func.func_id);
                fish::request(Some(rt), game, items, &inventory, func, *skill_id, obj_id, dialog, errors.as_mut());
                continue;
            }
        }
        let name = doodads.by_id.get(&obj_id).and_then(|o| game.and_then(|g| g.doodad.name(o.template_id))).unwrap_or("?");
        info!("doodad: use {name} ({obj_id}): {command:?}");
        out.write(NetCommand(command));
    }
    if let Some(rt) = rt {
        // Native `FUN_393b7b10` runs `FUN_393b7aa0` first when an interaction is already active,
        // so opening a second bench fires `INTERACTION_END` before the new `CRAFTING_START`.
        // `FUN_394dc360` then fires event 0x78 with the doodad object id and the unique recipe
        // count. The open itself is local (no packet); the window script's SetDoodad(doodadId,
        // count) treats either argument nil/zero as the general recipe book.
        for (object, count, replaced) in opens {
            if replaced || other_interaction {
                rt.fire_event("INTERACTION_END", ());
            }
            rt.fire_event("CRAFTING_START", (object, count));
        }
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
fn results(mut events: MessageReader<aa_net::NetEvent>, player: Res<aa_net::state::Player>, doodads: Res<Doodads>, bridge: NonSend<HudBridge>) {
    let own = player.spawn.as_ref().map_or(0, |s| s.obj_id);
    for aa_net::NetEvent(e) in events.read() {
        match e {
            aa_net::Event::SkillStarted(s) if s.caster.obj_id == own && s.target.kind == aa_net::skill::target_kind::DOODAD => {
                info!("doodad: cast {} at doodad {} (result {}, {} ms)", s.skill_id, s.target.obj_id, s.result, s.cast_ms);
            }
            aa_net::Event::DoodadPhaseChanged { obj_id, func_group_id, time_left, .. } if doodads.by_id.contains_key(obj_id) => {
                debug!("doodad: {obj_id} is in phase {func_group_id} now ({time_left} ms left)");
            }
            // Native `OnDoodadOriginator` stores the packet's faction in doodad+0x350, which
            // `FUN_393b3300` case 3 compares with the player's expedition id.
            aa_net::Event::DoodadOriginator { obj_id, new_faction, .. } => {
                bridge.0.borrow_mut().doodad.factions.insert(*obj_id, *new_faction);
            }
            aa_net::Event::DoodadsRemoved(ids) => {
                let mut data = bridge.0.borrow_mut();
                for id in ids {
                    data.doodad.factions.remove(id);
                }
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
        .init_resource::<fish::FishDialog>()
        .add_systems(
            Update,
            (
                sync_seated,
                track.in_set(TrackDoodads),
                coffer_window,
                results,
                phase_messages,
            )
                .chain()
                .after(crate::ingame::seating::track)
                .after(crate::ingame::units::sync_views)
                .after(super::sync_hud)
                .after(super::pets::walk_to_mount)
                .after(aa_ui::UiInput)
                .before(super::dialog::talk_input)
                .run_if(super::hud_up)
                .run_if(in_state(crate::Screen::InGame)),
        )
        .add_systems(Update, (pointer_use, execute, fish::tick).chain().after(super::PointerFinished).after(track).before(super::dialog::talk_pointer).run_if(super::hud_up).run_if(in_state(crate::Screen::InGame)))
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
        assert_eq!(t.uses(13066), Some(&Uses { skills: vec![14602], navi: vec![], craft: vec![] }));
        assert_eq!(t.uses(13067), None);
        assert_eq!(t.uses(1351), Some(&Uses { skills: vec![], navi: vec![Navi::Teleport, Navi::OpenPortal], craft: vec![] }));
        assert_eq!(t.uses(7851), Some(&Uses { skills: vec![15193], navi: vec![], craft: vec![] }));
        assert_eq!(t.uses(9), None, "the bar hides skill 10994");
    }

    #[test]
    fn a_safe_opens_without_a_skill() {
        let mut t = Tables::default();
        t.add(5602, 0, "DoodadFuncCoffer");
        assert_eq!(t.uses(5602), Some(&Uses { skills: vec![], navi: vec![Navi::Coffer], craft: vec![] }));
        assert_eq!(Navi::Coffer.label(), "Open");
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
        find_focus(data, doodads, &Units::default(), PermEnv::default()).map(|focus| focus.skills)
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

    /// The bar's candidate distance is the doodad interaction setting, not the skill's range:
    /// storage skill 15307 has `max_range` 20, but its button must clear at 4.5 m and 10 m from
    /// the box (2026-11-01 owner retest: a "store a trade pack" icon stayed on screen 10 m away).
    #[test]
    fn a_doodad_bar_uses_the_interaction_distance_not_the_skill_range() {
        let mut game = GameData::default();
        game.doodad.add(8264, 15307, "DoodadFuncConsumeChanger");
        game.skills.insert(
            15307,
            SkillInfo {
                max_range: 20,
                ..default()
            },
        );
        let mut data = HudData::default();
        assert!(data.game.0.set(game).is_ok());
        let mut doodads = Doodads::default();
        let storage = world_doodad(3446, 850, 8264, Vec3::new(10990.0, 10123.0, 102.5));
        assert!(doodads.apply(&aa_net::Event::DoodadsSpawned(vec![storage])));
        assert_eq!(bar_skills(&mut data, &doodads, 3.0), Some(vec![15307]));
        assert_eq!(
            bar_skills(&mut data, &doodads, 4.5),
            None,
            "the bar clears at the interaction distance, not the 20 m skill range"
        );
        assert_eq!(bar_skills(&mut data, &doodads, 10.0), None);
        // The capture's edge is a centre distance: a 0.55 m player radius must not stretch the
        // 4 m prompt to 4.5 m.
        data.player_radius = 0.55;
        assert_eq!(bar_skills(&mut data, &doodads, 3.9), Some(vec![15307]));
        assert_eq!(bar_skills(&mut data, &doodads, 4.0), None);
        assert_eq!(bar_skills(&mut data, &doodads, 4.5), None);
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
            // inside the 2.1 m use gap (the bar entry itself reaches 4 m)
            d.pos = Vec3::new(10990.0, 10121.5, 102.5);
            d.snapshot.labor = labor;
            d.doodad.focus = Some(Focus {
                obj_id: 2075,
                skills: vec![13990],
                craft_icon: None,
                key: -3.0,
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
            .insert_resource(Units::default())
            .insert_resource(inventory)
            .insert_resource(views)
            .insert_resource(crate::ingame::DevOptions::default())
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
            "CREATE TABLE doodad_almighties (id INTEGER, use_target_highlight TEXT, use_target_silhouette TEXT, use_target_decal TEXT, target_decal_size REAL, sim_radius REAL);
             CREATE TABLE doodad_funcs (id INTEGER, doodad_func_group_id INTEGER, func_skill_id INTEGER, actual_func_type TEXT, actual_func_id INTEGER, perm_id INTEGER, next_phase INTEGER);
             CREATE TABLE skills (id INTEGER);
             CREATE TABLE doodad_func_uses (id INTEGER, skill_id INTEGER);
             CREATE TABLE doodad_func_fake_uses (id INTEGER, fake_skill_id INTEGER);
             CREATE TABLE localized_texts (tbl_name TEXT, tbl_column_name TEXT, idx INTEGER, en_us TEXT);
             CREATE TABLE doodad_func_groups (id INTEGER, is_msg_to_zone TEXT, phase_msg TEXT);",
        )
        .unwrap();
        conn.execute_batch(
            "INSERT INTO doodad_almighties VALUES (7, 'f', 't', 'f', 0.0, 500.0);
             INSERT INTO doodad_almighties VALUES (8, 't', 'f', 't', 2.5, 0.0);
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
        let start = advance(false, false, 10.0, INTERACTION_DISTANCE, 10.0, 0.0, 0.0);
        assert_eq!(start.walk, Walk::Run);
        assert_eq!((start.best, start.best_at), (10.0, 0.0));
        let closer = advance(false, false, 6.0, INTERACTION_DISTANCE, 10.0, 0.0, 1.0);
        assert_eq!(closer.walk, Walk::Run);
        assert_eq!((closer.best, closer.best_at), (6.0, 1.0));
        assert_eq!(
            advance(false, false, 2.0, INTERACTION_DISTANCE, 6.0, 1.0, 5.0).walk,
            Walk::Arrived
        );
        assert_eq!(
            advance(false, true, 6.0, INTERACTION_DISTANCE, 6.0, 1.0, 5.0).walk,
            Walk::Cancel,
            "the player took over"
        );
        assert_eq!(
            advance(true, false, 6.0, INTERACTION_DISTANCE, 6.0, 1.0, 5.0).walk,
            Walk::Cancel,
            "the doodad is gone"
        );
        assert_eq!(
            advance(false, false, 5.9, INTERACTION_DISTANCE, 6.0, 1.0, 4.0).walk,
            Walk::Run,
            "5.9 is progress"
        );
        assert_eq!(
            advance(false, false, 5.9, INTERACTION_DISTANCE, 5.9, 4.0, 7.1).walk,
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

    #[test]
    fn a_vendor_half_a_metre_behind_the_stall_does_not_hide_it() {
        // the ray runs along bevy -z from the origin; game (x, y, z) -> bevy (x, z, -y), and the
        // pick middle is 0.9 m above the feet
        let ray = Ray3d::new(Vec3::ZERO, Dir3::NEG_Z);
        let npc = |along: f32| {
            let mut unit = crate::hud::fixture_spawn().unit;
            unit.obj_id = 21;
            unit.kind = aa_net::model::UnitKind::Npc {
                template_id: 1,
                owner_id: 0,
            };
            unit.pos = aa_net::Pos::new(0.0, along, -0.4);
            unit
        };
        let units = |along| {
            let mut all = Units::default();
            all.by_id.insert(21, npc(along));
            all
        };
        let stall = 6.0;
        let raw = |u: &aa_net::model::Unit| Some(Vec3::new(u.pos.x, u.pos.y, u.pos.z));
        // 0.5 m beyond the stall's middle: both are middles, so this is not "in front"
        assert!(!npc_in_front(&ray, stall, &units(6.5), 0, &raw));
        // 0.5 m before it: the vendor really is in front
        assert!(npc_in_front(&ray, stall, &units(5.5), 0, &raw));
    }

    #[derive(Resource, Default)]
    struct FrontOut(bool);

    fn read_npc_in_front(
        poses: crate::hud::pick::UnitPoses,
        units: Res<Units>,
        mut out: ResMut<FrontOut>,
    ) {
        let ray = Ray3d::new(Vec3::ZERO, Dir3::NEG_Z);
        out.0 = npc_in_front(&ray, 6.0, &units, 0, &|u| {
            poses.get(u, &units).map(|(pos, _)| pos)
        });
    }

    #[test]
    fn an_npc_attached_to_a_ship_is_tested_at_its_world_position() {
        // the packet position is the parent-local (0, -5, 0), which puts the NPC behind the
        // camera; its drawn root is a child of the ship at bevy (0, 0, -5), 5 m in front
        let mut npc = crate::hud::fixture_spawn().unit;
        npc.obj_id = 21;
        npc.kind = aa_net::model::UnitKind::Npc {
            template_id: 1,
            owner_id: 0,
        };
        npc.pos = aa_net::Pos::new(0.0, -5.0, 0.0);
        let mut units = Units::default();
        units.by_id.insert(21, npc);

        let mut app = App::new();
        app.init_resource::<Views>()
            .insert_resource(units)
            .init_resource::<FrontOut>()
            .add_systems(Update, read_npc_in_front);
        let ship = app.world_mut().spawn(Transform::default()).id();
        let root = app
            .world_mut()
            .spawn((
                Transform::from_translation(Vec3::new(0.0, 0.0, -5.0)),
                ChildOf(ship),
            ))
            .id();
        app.world_mut()
            .resource_mut::<Views>()
            .track_unit_entity(21, root);
        app.update();
        assert!(
            app.world().resource::<FrontOut>().0,
            "the drawn NPC is in front of the stall even though its packet position is behind"
        );
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

    /// The fish stands (`tool.fish_scale_a`: 6507, the Mirage Isle contest 7133, the test 6668) sell a fish with
    /// `DoodadFuncBuyFish` skill 21904 and convert it with `DoodadFuncConvertFish` 22201. Both are ordinary phase
    /// skills on the dynamic bar (`UseDoodadSkill` at the doodad), not a window of their own.
    #[test]
    fn the_fish_stands_offer_their_sell_and_convert_skills() {
        let Some(conn) = real_db() else { return };
        let t = Tables::load(&conn).unwrap();
        for (group, skills) in [(17262, vec![21_904, 22_201]), (19948, vec![21_904, 22_201]), (17729, vec![22_201])] {
            let uses = t.uses(group).unwrap_or_else(|| panic!("fish stand group {group}"));
            assert_eq!(uses.skills, skills, "fish stand group {group}");
            assert!(uses.navi.is_empty(), "the fish stand has no window");
        }
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
        find_focus(&data, &state, &Units::default(), PermEnv::default())
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
        // The capture stood 5.5 m away, past the 4 m interaction distance, so the portal's fake
        // use offered nothing.
        let mailbox_skill = conn
            .query_row(
                "SELECT func_skill_id FROM doodad_funcs WHERE doodad_func_group_id = 107",
                [],
                |r| r.get::<_, Option<i64>>(0),
            )
            .unwrap();
        assert_eq!(
            mailbox_skill, None,
            "the Mailbox row has no func_skill_id: 11400 is the native func-type mapping"
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
            right_click(&mut d, &doodads, &units, id, now, &mut dev, PermEnv::default());
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
            right_click(&mut d, &doodads, &units, id, now, &mut dev, PermEnv::default());
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
        let mut game = GameData {
            doodad: tables,
            ..Default::default()
        };
        // The bar's reach is the skill's; the right-click use must not take a long skill range as its own
        // interaction reach (2026-11-01: a 100 m doodad skill let the click use it from any distance).
        game.skills.insert(14602, SkillInfo { max_range: 100, ..Default::default() });
        let game = game;
        let data = HudData::default();
        let _ = data.game.0.set(game);
        let shared: Shared = std::rc::Rc::new(std::cell::RefCell::new(data));
        let mut state = Doodads::default();
        for doodad in doodads {
            // The walk tests need a measured gap: a drawn 1 x 1 x 1 m model box, not the sim-radius
            // fallback (which never defers now).
            shared.borrow_mut().doodad.bounds.insert(
                doodad.obj_id,
                (Vec3::new(-0.5, -0.5, 0.0), Vec3::new(0.5, 0.5, 1.0)),
            );
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
    fn a_large_workbench_measures_to_its_model_bounds_not_its_centre() {
        // sim_radius is 0 for most templates (Owl Mailbox, warehouses, workbenches); a large model
        // whose body keeps the player 4 m from its centre is still inside the 2.1 m interaction gap
        // when the gap is measured to its drawn model bounds (the collider approximation).
        let mut data = HudData {
            pos: Vec3::new(0.0, -4.0, 0.0),
            player_radius: 0.5,
            ..HudData::default()
        };
        data.doodad
            .bounds
            .insert(7, (Vec3::new(-3.0, -2.0, 0.0), Vec3::new(3.0, 2.0, 3.0)));
        let mut doodads = Doodads::default();
        doodads
            .by_id
            .insert(7, world_doodad(7, 850, 1, Vec3::ZERO));
        let (gap, real) = interaction_gap(&data, &doodads, &Units::default(), 7).unwrap();
        assert!(real, "the drawn model's bounds measured the gap");
        assert!(
            (gap - 1.5).abs() < 1e-4,
            "the 3 x 2 m body leaves 1.5 m from its side (centre distance would be 4 m): {gap}"
        );
        assert!(gap < INTERACTION_DISTANCE);
    }

    #[test]
    fn a_rotated_doodad_is_measured_in_its_own_frame_not_a_world_box() {
        // A 8 x 2 m body turned 45 degrees. The point (4.8, 4.8) is inside the axis-aligned box around
        // the model (that box would report a gap of 1.78 m) but 2.79 m from the real, rotated body, so
        // the right click must walk, not use.
        let mut data = HudData {
            pos: Vec3::new(4.8, 4.8, 1.5),
            ..HudData::default()
        };
        data.doodad
            .bounds
            .insert(7, (Vec3::new(-4.0, -1.0, 0.0), Vec3::new(4.0, 1.0, 3.0)));
        let mut doodad = world_doodad(7, 850, 1, Vec3::ZERO);
        doodad.rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_4).to_array();
        let mut doodads = Doodads::default();
        doodads.by_id.insert(7, doodad);
        let (gap, real) = interaction_gap(&data, &doodads, &Units::default(), 7).unwrap();
        assert!(real, "the drawn model's bounds measured the gap");
        assert!(
            (gap - 2.789).abs() < 0.01,
            "the rotated corner leaves its real 2.79 m gap: {gap}"
        );
        assert!(
            gap >= INTERACTION_DISTANCE,
            "the axis-aligned box's 1.78 m would have used it from out of reach"
        );
    }

    #[test]
    fn the_right_click_use_range_is_the_interaction_gap_not_the_skill_range() {
        // `max_interaction_doodad_distance` (2.1 m surface gap, `FUN_39093800`) is the right-click use reach;
        // `skills.max_range` only decides the bar's offer. A 100 m tome skill clicked at 3 m walks instead of
        // using the tome from there.
        let (mut app, shared) = approach_app(vec![test_doodad(7, 0.0, 3.0)]);
        {
            let mut h = app.world_mut().resource_mut::<Harness>();
            h.now = 0.0;
            h.click = Some(7);
        }
        app.update();
        assert!(
            shared.borrow().doodad.approach.is_some(),
            "3 m is beyond the 2.1 m interaction gap"
        );
        assert!(w_held(&app));
        assert!(
            shared.borrow().doodad.chosen.is_empty() && shared.borrow().doodad.menu.is_none(),
            "no use is queued before the walk arrives"
        );
        assert!(app.world().resource::<Harness>().acted.is_empty());

        app.world_mut().resource_mut::<Harness>().now = 1.0;
        app.world_mut().resource_mut::<Harness>().player = Vec3::new(0.0, 1.6, 0.0);
        app.update();
        assert_eq!(app.world().resource::<Harness>().acted, vec![7]);
        assert_eq!(shared.borrow().doodad.chosen.len(), 1, "one use on arrival");
        assert!(!w_held(&app));
    }

    #[test]
    fn a_doodad_inside_the_interaction_gap_is_used_at_once() {
        let (mut app, shared) = approach_app(vec![test_doodad(7, 0.0, 1.0)]);
        {
            let mut h = app.world_mut().resource_mut::<Harness>();
            h.now = 0.0;
            h.click = Some(7);
        }
        app.update();
        assert!(shared.borrow().doodad.approach.is_none());
        assert!(!w_held(&app));
        assert_eq!(shared.borrow().doodad.chosen.len(), 1, "the single use");
        assert!(app.world().resource::<Harness>().acted.is_empty());
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
        step(&mut app, 3.0, Vec3::new(0.0, 9.0, 0.0), false, None);
        step(&mut app, 4.0, Vec3::new(0.0, 8.8, 0.0), false, None);
        step(&mut app, 5.0, Vec3::new(0.0, 8.6, 0.0), false, None);
        assert_eq!(
            app.world().resource::<Harness>().acted,
            vec![7],
            "the use follows the arrival"
        );
        assert!(shared.borrow().doodad.approach.is_none());
        assert!(!w_held(&app));
        assert_eq!(shared.borrow().doodad.chosen.len(), 1, "one single use");
        step(&mut app, 6.0, Vec3::new(0.0, 8.6, 0.0), false, None);
        assert_eq!(
            app.world().resource::<Harness>().acted,
            vec![7],
            "no second action"
        );
        assert_eq!(shared.borrow().doodad.chosen.len(), 1);
    }

    #[test]
    fn an_in_reach_doodad_use_cancels_a_running_npc_walk() {
        let (mut app, shared) = approach_app(vec![test_doodad(7, 0.0, 1.0)]);
        crate::hud::dialog::start_walk(&mut shared.borrow_mut(), (20, 42), 5.0, 0.0, None);
        assert!(shared.borrow().dialog.approach.is_some());
        {
            let mut h = app.world_mut().resource_mut::<Harness>();
            h.now = 0.0;
            h.player = Vec3::ZERO;
            h.click = Some(7);
        }
        app.update();
        assert!(
            shared.borrow().dialog.approach.is_none(),
            "the in-reach doodad use ends the talk walk"
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

    /// F (`do_interaction_1`) on a slave prompt goes `FUN_393b1de0` -> `FUN_393be280` ->
    /// `CSBindSlave` 0x031. The bar is published with the native `"npc"` source and its first
    /// skill 12076; the request goes out through the real `handle_requests` packet path.
    #[test]
    fn a_slave_f_prompt_queues_cs_bind_slave() {
        let bridge = Shared::default();
        let mut units = Units::default();
        let mut cart = crate::hud::expansion::fixture::spawn().unit;
        cart.obj_id = 20;
        cart.name = "Farm Wagon".into();
        cart.kind = aa_net::model::UnitKind::Slave {
            id: 700,
            tl_id: 8,
            template_id: 60,
            summoner_id: 100,
        };
        cart.pos = aa_net::Pos::new(0.0, 1.0, 0.0);
        units.by_id.insert(20, cart);
        {
            let mut data = bridge.borrow_mut();
            data.pos = Vec3::ZERO;
            data.player_radius = 0.0;
            data.dynamic_actions.replace(Some(Shown {
                source: Source::Npc {
                    obj_id: 20,
                    template: 60,
                },
                skills: vec![12076],
            }));
            data.dynamic_actions.request(1);
        }
        let mut app = App::new();
        app.insert_non_send(HudBridge(bridge))
            .insert_resource(units)
            .insert_resource(Doodads::default())
            .insert_resource(Views::default())
            .insert_resource(crate::ingame::DevOptions::default())
            .insert_resource(Inventory::default())
            .init_resource::<NextState<crate::Screen>>()
            .init_resource::<aa_net::state::ActionBar>()
            .init_resource::<aa_net::state::Target>()
            .init_resource::<aa_net::state::Skills>()
            .init_resource::<aa_net::state::Buffs>()
            .init_resource::<crate::hud::InFlight>()
            .add_message::<NetCommand>()
            .add_message::<UseFailed>()
            .add_message::<crate::ingame::ground_target::SkillPressed>()
            .add_systems(Update, (execute, crate::hud::api::handle_requests).chain());
        app.update();
        let sent = app
            .world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .map(|message| message.0)
            .collect::<Vec<_>>();
        assert!(matches!(
            sent.as_slice(),
            [Command::Slave(aa_net::slave_control::SlaveCommand::Bind {
                tl_id: 8
            })]
        ));
    }

    /// The Farm Wagon's storage box (a doodad at its rear) is nearer in centre terms but stands
    /// behind or beside the player, while the wagon is in front: the native key (surface gap
    /// minus `1 + facing`) picks the wagon, which is the Grab Machine prompt the owner sees.
    /// Both producers are real (`find_focus` and `slave::candidate`), and the real `arbitrate`
    /// decides between them.
    #[test]
    fn the_native_key_arbitrates_the_rear_storage_box_against_the_grab_machine() {
        let mut game = GameData::default();
        game.doodad.add(1, 13990, "DoodadFuncUse");
        game.skills.insert(
            13990,
            SkillInfo {
                max_range: 4,
                ..default()
            },
        );
        let (mut data, doodads) = herb_field(game);
        // The player stands at the rear of the wagon (10, 10) facing north (+y), 1 m behind it.
        // The storage box is at (10, 8.6), 0.4 m behind the player: nearer, but behind.
        data.pos = Vec3::new(10.0, 9.0, 0.0);
        data.yaw = 0.0;
        let box_doodad = world_doodad(3446, 850, 1, Vec3::new(10.0, 8.6, 0.0));
        let mut doodads = doodads;
        doodads.by_id.insert(3446, box_doodad);
        let mut units = Units::default();
        let mut wagon = crate::hud::expansion::fixture::spawn().unit;
        wagon.obj_id = 20;
        wagon.kind = aa_net::model::UnitKind::Slave {
            id: 700,
            tl_id: 8,
            template_id: 60,
            summoner_id: 100,
        };
        wagon.model_id = 1008;
        wagon.pos = aa_net::Pos::new(10.0, 10.0, 0.0);
        units.by_id.insert(20, wagon);
        // A right click makes the box the preferred doodad; that must not force it past the
        // wagon in the shared candidate set (1.2 orders units and doodads on the native key).
        data.doodad.preferred = Some(3446);

        let focus = find_focus(&data, &doodads, &units, PermEnv::default()).expect("the storage box in reach");
        assert_eq!(focus.obj_id, 3446);
        assert!(
            (focus.dist - 0.4).abs() < 1e-4,
            "the box is the nearer centre: {}",
            focus.dist
        );
        let candidate = crate::hud::slave::candidate(&data, &units).expect("the wagon in reach");
        assert_eq!(candidate.obj_id, 20);
        assert_eq!(candidate.dist, 1.0);
        assert!(
            candidate.key < focus.key,
            "wagon key {} vs box key {}",
            candidate.key,
            focus.key
        );
        let doodad = (
            Shown {
                source: Source::Doodad {
                    obj_id: focus.obj_id,
                    phase: 1,
                    generation: 0,
                },
                skills: focus.skills,
            },
            focus.key,
            focus.dist,
        );
        let slave = (
            Shown {
                source: Source::Npc {
                    obj_id: candidate.obj_id,
                    template: candidate.template,
                },
                skills: candidate.skills,
            },
            candidate.key,
            candidate.dist,
        );
        let (wanted, dist) = arbitrate(Some(doodad), Some(slave));
        let wanted = wanted.expect("a bar");
        assert!(
            matches!(
                wanted.source,
                Source::Npc {
                    obj_id: 20,
                    template: 60
                }
            ),
            "the wagon wins over the nearer but behind storage box"
        );
        assert_eq!(dist, Some(1.0));
    }

    /// The right-click preference ranks doodads only. The preferred one keeps a native key, so a
    /// slave in front can still win `arbitrate`; without the click the native key chooses among
    /// the doodads.
    #[test]
    fn a_right_click_prefers_a_doodad_only_among_doodads() {
        let mut tables = Tables::default();
        tables.add(1, 13990, "DoodadFuncUse");
        let mut game = GameData {
            doodad: tables,
            ..Default::default()
        };
        game.skills.insert(
            13990,
            SkillInfo {
                max_range: 4,
                ..default()
            },
        );
        let mut data = HudData::default();
        let _ = data.game.0.set(game);
        data.pos = Vec3::ZERO;
        data.yaw = 0.0;
        let mut doodads = Doodads::default();
        // id 1 is 2 m behind (key 2), id 2 is 2 m in front (key 0).
        doodads
            .by_id
            .insert(1, world_doodad(1, 850, 1, Vec3::new(0.0, -2.0, 0.0)));
        doodads
            .by_id
            .insert(2, world_doodad(2, 850, 1, Vec3::new(0.0, 2.0, 0.0)));
        data.doodad.preferred = Some(1);
        let focus = find_focus(&data, &doodads, &Units::default(), PermEnv::default()).expect("a doodad");
        assert_eq!(
            focus.obj_id, 1,
            "the right-clicked doodad stays the bar's among doodads"
        );
        assert!(
            focus.key > 0.0,
            "its key is the native one, not f32::MIN: {}",
            focus.key
        );
        data.doodad.preferred = None;
        let focus = find_focus(&data, &doodads, &Units::default(), PermEnv::default()).expect("a doodad");
        assert_eq!(
            focus.obj_id, 2,
            "without a click the native key picks the front one"
        );
    }

    /// A player standing inside a vehicle's body box has a negative surface gap. The slave's
    /// bar key is clamped at 0 so a nearer storage box the player faces still wins the F prompt.
    #[test]
    fn a_player_inside_a_hauler_box_still_gets_its_facing_storage_prompt() {
        let mut game = GameData::default();
        game.doodad.add(1, 13990, "DoodadFuncUse");
        game.skills.insert(
            13990,
            SkillInfo {
                max_range: 4,
                ..default()
            },
        );
        game.set_vehicle_box(
            1553,
            Some((Vec3::new(-1.0, -4.5, 0.0), Vec3::new(1.0, 1.5, 2.0))),
        );
        let mut data = HudData::default();
        assert!(data.game.0.set(game).is_ok());
        // In the cargo bed, 3 m behind the origin, facing the rear storage box 2.1 m ahead.
        data.pos = Vec3::new(10.0, 7.0, 1.0);
        data.yaw = std::f32::consts::PI;
        data.player_radius = 0.55;
        let mut doodads = Doodads::default();
        doodads.by_id.insert(
            3446,
            world_doodad(3446, 850, 1, Vec3::new(10.0, 4.9, 1.0)),
        );
        let mut units = Units::default();
        let mut hauler = crate::hud::expansion::fixture::spawn().unit;
        hauler.obj_id = 20;
        hauler.kind = aa_net::model::UnitKind::Slave {
            id: 700,
            tl_id: 8,
            template_id: 146,
            summoner_id: 100,
        };
        hauler.model_id = 1553;
        hauler.pos = aa_net::Pos::new(10.0, 10.0, 0.0);
        units.by_id.insert(20, hauler);

        let focus = find_focus(&data, &doodads, &units, PermEnv::default()).expect("the facing storage box");
        assert_eq!(focus.obj_id, 3446);
        let candidate = crate::hud::slave::candidate(&data, &units).expect("the hauler");
        assert_eq!(candidate.obj_id, 20);
        assert!(
            focus.key < candidate.key,
            "storage key {} must beat the inside-box slave key {}",
            focus.key,
            candidate.key
        );
        let (wanted, _) = arbitrate(
            Some((
                Shown {
                    source: Source::Doodad {
                        obj_id: 3446,
                        phase: 1,
                        generation: 0,
                    },
                    skills: focus.skills,
                },
                focus.key,
                focus.dist,
            )),
            Some((
                Shown {
                    source: Source::Npc {
                        obj_id: 20,
                        template: 146,
                    },
                    skills: candidate.skills,
                },
                candidate.key,
                candidate.dist,
            )),
        );
        assert!(matches!(
            wanted.map(|w| w.source),
            Some(Source::Doodad { obj_id: 3446, .. })
        ));
    }

    /// The same with the real Red Farm Hauler body box (model 1553) and its storage box: the
    /// owner's "Store Trade Pack" prompt must win over the Grab Machine while facing it.
    #[test]
    #[ignore = "requires the game pak and game database (AA_PAK, AA_DB)"]
    fn the_real_hauler_box_does_not_hide_its_facing_storage_prompt()
    -> Result<(), Box<dyn std::error::Error>> {
        let pak = std::env::var("AA_PAK").unwrap_or_else(|_| aa_pak::DEFAULT_PAK.into());
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        if !std::path::Path::new(&pak).exists() || !std::path::Path::new(&path).exists() {
            eprintln!("skipping: the game pak {pak} or database {path} is not there");
            return Ok(());
        }
        let mut game = GameData::load(&path)?;
        let (lo, hi) = game.vehicle_box(1553).expect("the hauler body box");
        assert!(lo.y < -3.0, "the cargo bed extends behind: {lo:?}..{hi:?}");
        game.doodad.add(8264, 15307, "DoodadFuncConsumeChanger");
        game.skills.insert(
            15307,
            SkillInfo {
                max_range: 20,
                ..default()
            },
        );
        let mut data = HudData::default();
        assert!(data.game.0.set(game).is_ok());
        // In the cargo bed (3 m behind the origin), facing the rear storage box 2.1 m ahead.
        data.pos = Vec3::new(100.0, 197.0, 6.0);
        data.yaw = std::f32::consts::PI;
        data.player_radius = 0.55;
        let mut doodads = Doodads::default();
        let storage = world_doodad(3446, 850, 8264, Vec3::new(100.0, 194.9, 6.0));
        doodads.by_id.insert(3446, storage);
        let mut units = Units::default();
        let mut hauler = crate::hud::expansion::fixture::spawn().unit;
        hauler.obj_id = 20;
        hauler.kind = aa_net::model::UnitKind::Slave {
            id: 700,
            tl_id: 8,
            template_id: 146,
            summoner_id: 100,
        };
        hauler.model_id = 1553;
        hauler.pos = aa_net::Pos::new(100.0, 200.0, 5.0);
        units.by_id.insert(20, hauler);

        let focus = find_focus(&data, &doodads, &units, PermEnv::default()).expect("the facing storage box");
        let candidate = crate::hud::slave::candidate(&data, &units).expect("the hauler");
        assert!(
            focus.key < candidate.key,
            "storage key {} must beat the real-box slave key {}",
            focus.key,
            candidate.key
        );
        Ok(())
    }

    /// `player+0x3b94`: sitting on a bonded seat doodad shows no doodad bar either.
    #[test]
    fn no_doodad_bar_while_sitting_on_a_seat_doodad() {
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
        data.doodad.seated = true;
        assert_eq!(bar_skills(&mut data, &doodads, 3.0), None);
    }

    /// One F press with a climbable doodad in reach: the ladder grabs it and the dynamic action
    /// bar does not also use it (F3; the CSHang half is the ladder tests' grab test).
    #[test]
    fn a_ladder_claims_f_from_the_doodad_bar() {
        if !aa_cry::pak_available() {
            eprintln!("no pak: skipped");
            return;
        }
        use crate::ingame::DevOptions;
        use crate::ingame::Motion;
        use crate::ingame::data::Db;
        use crate::ingame::doodad_model::LadderHelper;
        use crate::ingame::ladder::{LadderClaim, LadderGrab, control_ladder};
        use crate::ingame::player::{LocalPlayer, Mover, PlayerInput, SendState};
        use crate::ingame::units::{RemoteDoodad, Views};

        let mut game = GameData::default();
        game.doodad.add(15013, 13990, "DoodadFuncUse");
        game.skills.insert(
            13990,
            SkillInfo {
                max_range: 4,
                ..default()
            },
        );
        let mut doodads = Doodads::default();
        assert!(
            doodads.apply(&aa_net::Event::DoodadsSpawned(vec![world_doodad(
                77,
                303,
                15013,
                Vec3::ZERO
            )]))
        );
        let shared = Shared::default();
        {
            let mut d = shared.borrow_mut();
            assert!(d.game.0.set(game).is_ok());
            d.pos = Vec3::new(0.5, 0.0, 0.0);
        }
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(ButtonInput::<MouseButton>::default())
            .insert_resource(crate::settings::Actions::default())
            .insert_resource(super::super::HudKeyboardFocus::default())
            .insert_resource(super::super::HudPointerOverUi::default())
            .insert_resource(DevOptions::default())
            .insert_resource(Db::ladder_fixture(15013, 1).expect("the ladder fixture"))
            .insert_resource(doodads)
            .insert_resource(Units::default())
            .insert_resource(Views::default())
            .insert_resource(aa_net::state::Player::default())
            .insert_resource(Inventory::default())
            .insert_resource(DoodadHighlights::default())
            .insert_resource(DoodadTargetDecal::default())
            .insert_resource(aa_net::CharacterNames::default())
            .insert_resource(LadderClaim::default())
            .insert_resource(LadderGrab::default())
            .insert_resource(SendState::default())
            .insert_resource(PlayerInput::default())
            .insert_resource(aa_anim::ClipLibrary::default())
            .insert_resource(Scripted::default())
            .add_message::<NetCommand>()
            .add_message::<super::super::alerts::UseFailed>()
            .add_systems(Update, (control_ladder, track, execute).chain());
        app.insert_non_send(UiRuntime::new("en_us", 1280., 720.).expect("the UI runtime"));
        app.insert_non_send(HudBridge(shared.clone()));
        let doodad = app
            .world_mut()
            .spawn((
                RemoteDoodad { obj_id: 77 },
                DoodadVisual {
                    path: String::new(),
                    model: None,
                    func_group_id: 15013,
                    rendered_func_group_id: 15013,
                    bond_helpers: Default::default(),
                    ladder: Some(LadderHelper {
                        bottom: Vec3::ZERO,
                        top: Vec3::new(0.0, 5.0, 0.0),
                        axis: Vec3::Y,
                        forward: Vec3::NEG_Z,
                    }),
                    growth: None,
                    remaining_ms: 0.0,
                    total_ms: 0.0,
                    base_scale: 1.0,
                },
                Transform::default(),
                GlobalTransform::default(),
            ))
            .id();
        app.world_mut()
            .resource_mut::<Views>()
            .fixture_doodad(77, doodad);
        app.world_mut().spawn((
            LocalPlayer { obj_id: 100 },
            Mover {
                pos: Vec3::new(0.5, 0.0, 0.0),
                facing: 0.0,
                vz: 0.0,
                grounded: true,
                velocity: Vec3::ZERO,
                swim: None,
                wading: 0.0,
            },
            Transform::default(),
            GlobalTransform::default(),
            Motion::default(),
        ));
        app.world_mut()
            .resource_mut::<crate::settings::Actions>()
            .tap_for_test("do_interaction_1");
        app.update();
        let commands: Vec<Command> = app
            .world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .map(|v| v.0)
            .collect();
        assert!(
            commands
                .iter()
                .any(|c| matches!(c, Command::Hang { target_obj_id: 77 })),
            "the ladder grabbed: {commands:?}"
        );
        assert!(
            !commands
                .iter()
                .any(|c| matches!(c, Command::UseDoodadSkill { .. })),
            "the ladder owns F, the doodad bar does not use it: {commands:?}"
        );
        assert!(
            shared.borrow().doodad.focus.is_some(),
            "the doodad is in the bar's reach"
        );
        assert_eq!(app.world().resource::<LadderClaim>().0, Some(77));
        let requested = shared.borrow_mut().dynamic_actions.take_requested();
        assert!(
            requested.is_empty(),
            "F did not request the bar: {requested:?}"
        );
    }

    /// A climb doodad has no use skills, but its template sets `use_target_highlight`; the pick
    /// must still resolve it. A ship ladder's packet origin is below the waterline while its
    /// rungs are at deck height, so the pick follows the whole `$ladder` segment.
    #[test]
    fn a_climb_doodad_is_picked_for_the_highlight() {
        let mut doodads = Doodads::default();
        doodads
            .by_id
            .insert(77, world_doodad(77, 1187, 15013, Vec3::new(0.0, 0.0, 0.0)));
        let data = HudData {
            pos: Vec3::new(0.0, -5.0, 0.0),
            ..Default::default()
        };
        // the ray crosses the rung at 3 m up (the packet origin is at the foot, z 0)
        let ray = Ray3d::new(
            aa_world::cry_to_bevy(Vec3::new(0.0, -5.0, 3.0)),
            bevy::math::Dir3::new(aa_world::cry_to_bevy(Vec3::new(0.0, 1.0, 0.0))).unwrap(),
        );
        // the drawn segment: the `$ladder` helper in the doodad root at the origin
        let segments = [(77, Vec3::ZERO, Vec3::new(0.0, 5.0, 0.0))];
        assert_eq!(
            pick_climb(ray, &data, &doodads, &Units::default(), &segments).map(|(_, id)| id),
            Some(77),
            "the visible rungs are picked"
        );
        // a ray from above the top pointing down: the bottom sample is the first and the
        // farthest; the nearest sample (the top) must win.
        let from_above = Ray3d::new(Vec3::new(0.0, 11.0, 0.4), bevy::math::Dir3::NEG_Y);
        let long = [(77, Vec3::ZERO, Vec3::new(0.0, 10.0, 0.0))];
        let (along, id) = pick_climb(from_above, &data, &doodads, &Units::default(), &long)
            .expect("the segment is under the ray");
        assert_eq!(id, 77);
        assert!(along < 2.0, "nearest sample, not the first: {along}");
    }

    #[test]
    fn the_nearest_hit_keeps_a_closer_doodad_over_a_ladder_behind_it() {
        assert_eq!(
            nearest_hit(Some((5.0, 77)), Some((3.0, 78))),
            Some((3.0, 78))
        );
        assert_eq!(
            nearest_hit(Some((3.0, 77)), Some((5.0, 78))),
            Some((3.0, 77))
        );
        assert_eq!(nearest_hit(None, Some((5.0, 78))), Some((5.0, 78)));
        assert_eq!(nearest_hit(None, None), None);
    }

    /// The Eznan Cutter's ladder doodads must light up: the owner hovered one and saw no
    /// highlight. `slave_doodad_bindings` for owner 21 lists templates 1187/3299/3300; their
    /// `doodad_almighties.use_target_highlight` is on.
    #[test]
    fn the_eznan_cutter_ladder_templates_glow() {
        let Some(conn) = real_db() else { return };
        let tables = Tables::load(&conn).unwrap();
        let mut q = conn
            .prepare(
                "SELECT d.id, d.model FROM slave_doodad_bindings b
                 JOIN doodad_almighties d ON d.id = b.doodad_id
                 WHERE b.owner_id = 21 AND b.owner_type = 'Slave' AND d.model LIKE '%ladder%'",
            )
            .unwrap();
        let rows = q
            .query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, String>(1)?)))
            .unwrap();
        let mut found = 0;
        for row in rows {
            let (template, model) = row.unwrap();
            found += 1;
            assert!(
                tables.glows(template),
                "the Eznan Cutter ladder template {template} ({model}) does not glow"
            );
        }
        assert!(found > 0, "the Eznan Cutter has a ladder doodad binding");
    }

    /// A workbench phase (Alchemy Table 568 / `DoodadFuncCraftPack` group 1088) has no skill:
    /// native `393bd290` lists its craft action, `393b7ba0` -> `394dc360` opens the local bench and
    /// emits `CRAFTING_START(objectID, uniqueRecipeCount)` (event 0x78). The right-click use goes
    /// through the same local action, sends no packet, and opens the original window script.
    #[test]
    fn a_craft_bench_use_fires_crafting_start_and_opens_the_bench() {
        let pak = std::env::var("AA_PAK").unwrap_or_else(|_| aa_pak::DEFAULT_PAK.to_string());
        if !std::path::Path::new(&pak).is_file() {
            eprintln!("skip: no game_pak at {pak}");
            return;
        }
        let Some(conn) = real_db() else { return };
        let mut game = GameData::default();
        game.doodad = Tables::load(&conn).unwrap();
        game.crafting = crate::hud::crafting::Tables::load(&conn).unwrap();
        let bridge = Shared::default();
        {
            let mut d = bridge.borrow_mut();
            d.character_id = 47;
            d.pos = Vec3::ZERO;
            d.player_radius = 0.0;
            assert!(d.game.0.set(game).is_ok());
        }
        let mut doodads = Doodads::default();
        doodads.by_id.insert(300, world_doodad(300, 568, 1088, Vec3::ZERO));
        let mut units = Units::default();
        units.by_id.clear();
        {
            let mut d = bridge.borrow_mut();
            let mut dev = crate::ingame::DevOptions::default();
            assert!(
                !right_click(&mut d, &doodads, &units, 300, 0.0, &mut dev, PermEnv::default()),
                "a single craft action uses the doodad at once"
            );
            assert_eq!(d.doodad.chosen, vec![i64::from(CRAFT_ACTION)]);
        }
        let rt = UiRuntime::new("en_us", 800.0, 600.0).unwrap();
        rt.lua
            .load(
                "CRAFT_ARGS={};local w=UIParent:CreateWidget('emptywidget','craftProbe','UIParent');                 w:RegisterEvent('CRAFTING_START');                 w:SetHandler('OnEvent',function(_,event,id,count) CRAFT_ARGS={id,count} end)",
            )
            .exec()
            .unwrap();
        let mut app = App::new();
        app.insert_non_send(rt)
            .insert_non_send(HudBridge(bridge.clone()))
            .insert_resource(units)
            .insert_resource(doodads)
            .insert_resource(Views::default())
            .insert_resource(crate::ingame::DevOptions::default())
            .insert_resource(Inventory::default())
            .init_resource::<NextState<crate::Screen>>()
            .init_resource::<aa_net::state::ActionBar>()
            .init_resource::<aa_net::state::Target>()
            .init_resource::<aa_net::state::Skills>()
            .init_resource::<aa_net::state::Buffs>()
            .init_resource::<crate::hud::InFlight>()
            .add_message::<NetCommand>()
            .add_message::<UseFailed>()
            .add_message::<crate::ingame::ground_target::SkillPressed>()
            .add_systems(Update, execute);
        app.update();
        let d = bridge.borrow();
        assert_eq!(
            crate::hud::crafting::active_bench(&d),
            Some((300, 568)),
            "the bench is the live CURRENT workbench"
        );
        assert_eq!(crate::hud::crafting::current_recipe_count(&d), 71, "Alchemy Table 568 contributes 71 recipes");
        drop(d);
        let rt = app.world().non_send::<UiRuntime>();
        rt.lua
            .load("assert(CRAFT_ARGS[1]==300 and CRAFT_ARGS[2]==71,'CRAFTING_START(doodadId,count)')")
            .exec()
            .unwrap();
        let sent = app
            .world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .map(|message| message.0)
            .collect::<Vec<_>>();
        assert!(sent.is_empty(), "opening a bench is local (no packet): {sent:?}");
    }
    fn craft_tables(group: u32, perm: u8) -> GameData {
        let mut tables = Tables::default();
        tables.add_full(group, 0, "DoodadFuncCraftPack", perm, 0, 0, 0, &local::LocalFuncs::default());
        GameData {
            doodad: tables,
            crafting: crate::hud::crafting::fixture_tables([(568, vec![5557]), (2241, vec![6666])]),
            ..Default::default()
        }
    }

    fn test_player<'a>(
        party: &'a aa_net::Party,
        friends: &'a aa_net::Friends,
        dominion: &'a aa_net::Dominion,
        expeditions: &'a aa_net::Expeditions,
        faction_mother: &'a HashMap<u32, u32>,
    ) -> PlayerFacts<'a> {
        PlayerFacts {
            character_id: 47,
            account_id: 7,
            expedition_id: 900,
            nation: 148,
            dominion,
            expeditions,
            faction_mother: Some(faction_mother),
            party,
            friends,
            house_zone_groups: Some(vec![5]),
        }
    }

    fn house_snapshot(tl_id: u16, obj_id: u32, x: f32, y: f32) -> aa_net::HouseSnapshot {
        aa_net::HouseSnapshot {
            tl_id,
            db_id: u32::from(tl_id),
            obj_id,
            template_id: 1,
            model_id: 1,
            co_owner_id: 0,
            owner_id: 47,
            owner_name: "Me".into(),
            account_id: 7,
            permission: 0,
            all_actions: 0,
            current_actions: 0,
            tax: 0,
            x: aa_net::wire::coord_to_long(x),
            y: aa_net::wire::coord_to_long(y),
            z: 0.0,
            name: "House".into(),
            allow_recover: false,
            sell_price: 0,
            sell_to_player_id: 0,
            sell_to_name: String::new(),
        }
    }

    fn perm_doodad(owner_id: u32, house_owner_id: u32, house_account_id: u32, faction: u32, zone_group: Option<u32>) -> DoodadFacts {
        DoodadFacts {
            owner_id,
            house_owner_id,
            house_account_id,
            faction,
            zone_group,
        }
    }

    /// Native `FUN_393b3300` cases 0-8 and the default, each craft func checked on its own.
    #[test]
    fn craft_permissions_follow_native_393b3300() {
        let mut friends = aa_net::Friends::default();
        friends.friends.push(aa_net::FriendInfo { id: 99, ..Default::default() });
        let party = aa_net::Party::default();
        // Zone 5's Dominion owner is expedition 900 (Nuia, 148); zone 6's is 901 (Haranya, 149).
        let mut expeditions = aa_net::Expeditions::default();
        expeditions.current_faction_id = 148;
        for (id, mother) in [(900, 148), (901, 149)] {
            expeditions.factions.insert(
                id,
                aa_net::ExpeditionFaction {
                    id,
                    mother_id: mother,
                    ..Default::default()
                },
            );
        }
        let faction_mother = HashMap::from([(148, 0), (149, 0)]);
        let mut dominion = aa_net::Dominion::default();
        dominion.rows.insert(
            5,
            aa_net::DominionData {
                zone_group_id: 5,
                expedition_id: 900,
                ..Default::default()
            },
        );
        dominion.rows.insert(
            6,
            aa_net::DominionData {
                zone_group_id: 6,
                expedition_id: 901,
                ..Default::default()
            },
        );
        let player = test_player(&party, &friends, &dominion, &expeditions, &faction_mother);
        assert!(craft_permitted(craft_perm::ANY, &player, &perm_doodad(5, 0, 0, 0, Some(6))));
        assert!(!craft_permitted(craft_perm::OWNER, &player, &perm_doodad(5, 0, 0, 0, Some(5))));
        assert!(craft_permitted(craft_perm::OWNER, &player, &perm_doodad(47, 0, 0, 0, Some(5))));
        assert!(
            craft_permitted(craft_perm::OWNER, &player, &perm_doodad(0, 47, 0, 0, Some(5))),
            "the house owner is the doodad's owner"
        );
        assert!(craft_permitted(craft_perm::OWNER_OR_FRIEND, &player, &perm_doodad(99, 0, 0, 0, Some(5))));
        assert!(!craft_permitted(craft_perm::OWNER_OR_FRIEND, &player, &perm_doodad(5, 0, 0, 0, Some(5))));
        assert!(
            craft_permitted(craft_perm::EXPEDITION, &player, &perm_doodad(5, 0, 0, 0, Some(5))),
            "a doodad with no faction is open to an expedition member"
        );
        assert!(craft_permitted(craft_perm::EXPEDITION, &player, &perm_doodad(5, 0, 0, 900, Some(5))));
        assert!(!craft_permitted(craft_perm::EXPEDITION, &player, &perm_doodad(5, 0, 0, 901, Some(5))));
        let mut no_expedition =
            test_player(&party, &friends, &dominion, &expeditions, &faction_mother);
        no_expedition.expedition_id = 0;
        assert!(!craft_permitted(craft_perm::EXPEDITION, &no_expedition, &perm_doodad(5, 0, 0, 0, Some(5))));
        assert!(
            !craft_permitted(
                craft_perm::SAME_ACCOUNT,
                &player,
                &perm_doodad(48, 0, 0, 0, Some(5))
            ),
            "an alt of the login list is not the owner: native FUN_393b32c0 only takes the house account"
        );
        assert!(
            craft_permitted(
                craft_perm::SAME_ACCOUNT,
                &player,
                &perm_doodad(5, 0, 7, 0, Some(5))
            ),
            "the house's account"
        );
        assert!(!craft_permitted(craft_perm::SAME_ACCOUNT, &player, &perm_doodad(5, 0, 8, 0, Some(5))));
        assert!(
            craft_permitted(
                craft_perm::SAME_NATION,
                &player,
                &perm_doodad(5, 0, 0, 0, Some(5))
            ),
            "the zone's Dominion owner (expedition 900) is of the player's nation (148)"
        );
        assert!(
            !craft_permitted(
                craft_perm::SAME_NATION,
                &player,
                &perm_doodad(5, 0, 0, 0, Some(6))
            ),
            "the zone's Dominion owner (expedition 901) is of nation 149"
        );
        assert!(
            !craft_permitted(
                craft_perm::SAME_NATION,
                &player,
                &perm_doodad(5, 0, 0, 0, Some(7))
            ),
            "a zone with no received Dominion row is refused"
        );
        assert!(
            !craft_permitted(
                craft_perm::SAME_NATION,
                &player,
                &perm_doodad(5, 0, 0, 0, None)
            ),
            "an unknown zone is refused"
        );
        let mut no_nation = test_player(&party, &friends, &dominion, &expeditions, &faction_mother);
        no_nation.nation = 0;
        assert!(
            !craft_permitted(
                craft_perm::SAME_NATION,
                &no_nation,
                &perm_doodad(5, 0, 0, 0, Some(5))
            ),
            "a player with no nation is refused"
        );
        let mut foreign_faction = aa_net::Expeditions::default();
        foreign_faction.current_faction_id = 148;
        foreign_faction.factions.insert(
            902,
            aa_net::ExpeditionFaction {
                id: 902,
                mother_id: 148,
                ..Default::default()
            },
        );
        let mut unknown_owner = aa_net::Dominion::default();
        unknown_owner.rows.insert(
            5,
            aa_net::DominionData {
                zone_group_id: 5,
                expedition_id: 999,
                ..Default::default()
            },
        );
        let unknown = test_player(
            &party,
            &friends,
            &unknown_owner,
            &foreign_faction,
            &faction_mother,
        );
        assert!(
            !craft_permitted(
                craft_perm::SAME_NATION,
                &unknown,
                &perm_doodad(5, 0, 0, 0, Some(5))
            ),
            "an owner the client has no faction record for is refused"
        );
        assert!(craft_permitted(craft_perm::ZONE_RESIDENTS, &player, &perm_doodad(5, 0, 0, 0, Some(5))));
        assert!(!craft_permitted(craft_perm::ZONE_RESIDENTS, &player, &perm_doodad(5, 0, 0, 0, Some(6))));
        let mut no_houses = test_player(&party, &friends, &dominion, &expeditions, &faction_mother);
        no_houses.house_zone_groups = Some(Vec::new());
        assert!(!craft_permitted(craft_perm::ZONE_RESIDENTS, &no_houses, &perm_doodad(5, 0, 0, 0, Some(5))));
        no_houses.house_zone_groups = None;
        assert!(
            craft_permitted(craft_perm::ZONE_RESIDENTS, &no_houses, &perm_doodad(5, 0, 0, 0, Some(5))),
            "unknown house zones are inferred as allowed"
        );
        assert!(!craft_permitted(9, &player, &perm_doodad(5, 0, 0, 0, Some(5))), "the default refuses");

        let mut raid = aa_net::Party { id: 3, is_party: false, ..Default::default() };
        raid.members.push(aa_net::PartyMember { id: 47, slot: 0, ..Default::default() });
        raid.members.push(aa_net::PartyMember { id: 50, slot: 4, ..Default::default() });
        raid.members.push(aa_net::PartyMember { id: 51, slot: 5, ..Default::default() });
        let raider = test_player(&raid, &friends, &dominion, &expeditions, &faction_mother);
        assert!(craft_permitted(craft_perm::OWNER_OR_PARTY, &raider, &perm_doodad(50, 0, 0, 0, Some(5))));
        assert!(!craft_permitted(craft_perm::OWNER_OR_PARTY, &raider, &perm_doodad(51, 0, 0, 0, Some(5))));
        assert!(craft_permitted(craft_perm::OWNER_OR_RAID, &raider, &perm_doodad(51, 0, 0, 0, Some(5))));
        assert!(!craft_permitted(craft_perm::OWNER_OR_RAID, &raider, &perm_doodad(52, 0, 0, 0, Some(5))));
    }

    fn pending_app(bridge: Shared, doodads: Doodads, units: Units, views: Views) -> App {
        let mut app = App::new();
        app.insert_non_send(HudBridge(bridge))
            .insert_resource(doodads)
            .insert_resource(units)
            .insert_resource(views)
            .insert_resource(Inventory::default())
            .insert_resource(Time::<()>::default())
            .insert_resource(crate::ingame::DevOptions::default())
            .add_message::<NetCommand>()
            .add_message::<UseFailed>()
            .add_message::<crate::hud::alerts::ShowError>()
            .add_systems(Update, (pointer_use, execute).chain());
        app
    }

    fn sent_commands(app: &mut App) -> Vec<Command> {
        app.world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .map(|message| message.0)
            .collect()
    }

    fn mixed_tables(group: u32, skill: u32, perm: u8) -> GameData {
        let mut game = craft_tables(group, perm);
        game.doodad.add_full(group, skill, "DoodadFuncUse", 0, 0, 0, 0, &local::LocalFuncs::default());
        game
    }

    /// Native `FUN_393b3ec0` runs `FUN_39093800` for every action, so a phase that offers a skill
    /// and a craft walks first at 2.1-4 m; only a craft-only phase is listed from the bar distance.
    #[test]
    fn a_mixed_skill_and_craft_phase_walks_before_listing_at_3_m() {
        let mut data = HudData {
            character_id: 47,
            pos: Vec3::ZERO,
            player_radius: 0.0,
            ..HudData::default()
        };
        assert!(
            data.game
                .0
                .set(mixed_tables(1088, 14602, craft_perm::ANY))
                .is_ok()
        );
        data.doodad
            .bounds
            .insert(300, (Vec3::new(-0.5, -0.5, 0.0), Vec3::new(0.5, 0.5, 1.0)));
        let mut doodads = Doodads::default();
        doodads
            .by_id
            .insert(300, world_doodad(300, 568, 1088, Vec3::new(0.0, 3.0, 0.0)));
        let mut dev = crate::ingame::DevOptions::default();
        assert!(!right_click(
            &mut data,
            &doodads,
            &Units::default(),
            300,
            0.0,
            &mut dev,
            PermEnv::default()
        ));
        assert!(
            data.doodad.approach.is_some(),
            "the skill entry is out of the 2.1 m surface gap: walk before the list"
        );
        assert!(data.doodad.menu.is_none(), "no list before the walk");
        assert!(data.doodad.chosen.is_empty(), "no use before the walk");
    }

    /// A skill picked from the bar out of the 2.1 m surface gap walks and keeps the pending use
    /// (native `FUN_393b3ec0` -> `FUN_390a6af0`), then sends it on arrival.
    #[test]
    fn a_bar_skill_picked_out_of_reach_walks_and_runs_on_arrival() {
        let bridge = Shared::default();
        {
            let mut d = bridge.borrow_mut();
            d.character_id = 47;
            d.pos = Vec3::ZERO;
            d.player_radius = 0.0;
            let mut game = GameData::default();
            game.doodad.add_full(1, 14602, "DoodadFuncUse", 0, 0, 0, 0, &local::LocalFuncs::default());
            assert!(d.game.0.set(game).is_ok());
            d.doodad.focus = Some(Focus {
                obj_id: 300,
                skills: vec![14602],
                craft_icon: None,
                key: -3.0,
                dist: 3.0,
            });
            d.doodad
                .bounds
                .insert(300, (Vec3::new(-0.5, -0.5, 0.0), Vec3::new(0.5, 0.5, 1.0)));
            d.dynamic_actions.replace(Some(Shown {
                source: Source::Doodad {
                    obj_id: 300,
                    phase: 1,
                    generation: 1,
                },
                skills: vec![14602],
            }));
            d.dynamic_actions.request(1);
        }
        let mut doodads = Doodads::default();
        doodads
            .by_id
            .insert(300, world_doodad(300, 850, 1, Vec3::new(0.0, 3.0, 0.0)));
        let mut views = Views::default();
        views.fixture_doodad(300, Entity::PLACEHOLDER);
        let mut app = pending_app(bridge.clone(), doodads, Units::default(), views);
        app.update();
        {
            let d = bridge.borrow();
            assert!(d.doodad.approach.is_some());
            assert_eq!(
                d.doodad.pending,
                Some(Pending {
                    obj_id: 300,
                    action: 14602,
                    phase: 1,
                    generation: Some(1),
                })
            );
        }
        assert!(
            sent_commands(&mut app).is_empty(),
            "no skill before the walk"
        );

        // arrive: pointer_use takes the pending action, execute sends it once
        {
            let mut d = bridge.borrow_mut();
            d.pos = Vec3::new(0.0, 1.0, 0.0);
            d.doodad.arrived = Some(300);
        }
        app.update();
        let sent = sent_commands(&mut app);
        assert!(
            matches!(
                sent.as_slice(),
                [Command::UseDoodadSkill {
                    skill_id: 14602,
                    doodad_obj_id: 300,
                }]
            ),
            "{sent:?}"
        );
        assert!(bridge.borrow().doodad.pending.is_none());
    }

    /// The bar request used by the fallback tests: skill 14602 on doodad 300 in phase 1 at 3 m.
    fn fallback_skill_app(bounded: bool) -> (App, Shared) {
        let bridge = Shared::default();
        {
            let mut d = bridge.borrow_mut();
            d.character_id = 47;
            d.pos = Vec3::ZERO;
            d.player_radius = 0.0;
            let mut game = GameData::default();
            game.doodad.add_full(1, 14602, "DoodadFuncUse", 0, 0, 0, 0, &local::LocalFuncs::default());
            assert!(d.game.0.set(game).is_ok());
            d.doodad.focus = Some(Focus {
                obj_id: 300,
                skills: vec![14602],
                craft_icon: None,
                key: -3.0,
                dist: 3.0,
            });
            if bounded {
                d.doodad
                    .bounds
                    .insert(300, (Vec3::new(-0.5, -0.5, 0.0), Vec3::new(0.5, 0.5, 1.0)));
            }
            d.dynamic_actions.replace(Some(Shown {
                source: Source::Doodad {
                    obj_id: 300,
                    phase: 1,
                    generation: 1,
                },
                skills: vec![14602],
            }));
            d.dynamic_actions.request(1);
        }
        let mut doodads = Doodads::default();
        doodads
            .by_id
            .insert(300, world_doodad(300, 850, 1, Vec3::new(0.0, 3.0, 0.0)));
        let mut views = Views::default();
        views.fixture_doodad(300, Entity::PLACEHOLDER);
        (pending_app(bridge.clone(), doodads, Units::default(), views), bridge)
    }

    /// The sim-radius fallback (no drawn model bounds, or a parented doodad) is not the native
    /// collider shape: FUN_393b3ec0's range check must not defer. The use is sent at once and the
    /// server checks the reach.
    #[test]
    fn a_doodad_without_drawn_bounds_is_used_at_once() {
        let (mut app, bridge) = fallback_skill_app(false);
        app.update();
        {
            let d = bridge.borrow();
            assert!(
                d.doodad.approach.is_none(),
                "the fallback gap must not walk"
            );
            assert!(d.doodad.pending.is_none(), "no pending use");
        }
        let sent = sent_commands(&mut app);
        assert!(
            matches!(
                sent.as_slice(),
                [Command::UseDoodadSkill {
                    skill_id: 14602,
                    doodad_obj_id: 300,
                }]
            ),
            "{sent:?}"
        );
    }

    /// The same bar request against a drawn model box does defer: the gap is real and out of reach.
    #[test]
    fn a_bounded_doodad_at_3_m_walks_first() {
        let (mut app, bridge) = fallback_skill_app(true);
        app.update();
        {
            let d = bridge.borrow();
            assert!(d.doodad.approach.is_some(), "the measured gap walks");
            assert_eq!(
                d.doodad.pending,
                Some(Pending {
                    obj_id: 300,
                    action: 14602,
                    phase: 1,
                    generation: Some(1),
                })
            );
        }
        assert!(sent_commands(&mut app).is_empty(), "no skill before the walk");
    }

    /// A walk that stops with no progress on a real measured gap cannot run the pending use: it
    /// answers TOO_FAR_AWAY (FUN_39428370(0x9a)) instead of dropping it silently.
    #[test]
    fn a_stuck_walk_on_a_measured_gap_shows_too_far_away() {
        let (mut app, shared) = approach_app(vec![test_doodad(7, 0.0, 3.0)]);
        {
            let mut h = app.world_mut().resource_mut::<Harness>();
            h.now = 0.0;
            h.click = Some(7);
        }
        app.update();
        assert!(shared.borrow().doodad.approach.is_some(), "the bounded doodad walks");
        shared.borrow_mut().doodad.pending = Some(Pending {
            obj_id: 7,
            action: 14602,
            phase: 1,
            generation: None,
        });

        {
            let mut h = app.world_mut().resource_mut::<Harness>();
            h.now = 4.0;
            h.click = None;
        }
        app.update();
        let d = shared.borrow();
        assert!(d.doodad.too_far, "the stuck pending use is refused");
        assert!(d.doodad.pending.is_none());
        assert!(d.doodad.approach.is_none());
        assert!(app.world().resource::<Harness>().acted.is_empty());
    }

    /// The same stop on a fallback gap runs the pending use instead: the sim radius cannot prove
    /// that the doodad is out of reach.
    #[test]
    fn a_stuck_walk_on_a_fallback_gap_uses_the_pending_action() {
        let (mut app, shared) = approach_app(vec![test_doodad(7, 0.0, 3.0)]);
        shared.borrow_mut().doodad.bounds.clear();
        {
            let mut d = shared.borrow_mut();
            d.doodad.approach = Some(Approach {
                obj_id: 7,
                best: 3.0,
                best_at: 0.0,
            });
            d.doodad.pending = Some(Pending {
                obj_id: 7,
                action: 14602,
                phase: 1,
                generation: None,
            });
        }
        {
            let mut h = app.world_mut().resource_mut::<Harness>();
            h.now = 4.0;
            h.click = None;
        }
        app.update();
        assert_eq!(
            app.world().resource::<Harness>().acted,
            vec![7],
            "the use follows the stuck walk"
        );
        assert!(!shared.borrow().doodad.too_far);
    }

    /// The refusal from the stuck walk is shown through the alerts system.
    #[test]
    fn the_stuck_walk_refusal_is_shown() {
        let (mut app, bridge) = fallback_skill_app(false);
        bridge.borrow_mut().doodad.too_far = true;
        {
            let mut d = bridge.borrow_mut();
            d.dynamic_actions.replace(None);
        }
        app.update();
        let errors = app
            .world_mut()
            .resource_mut::<Messages<crate::hud::alerts::ShowError>>()
            .drain()
            .map(|error| error.0)
            .collect::<Vec<_>>();
        assert_eq!(errors, vec![0x9a], "TOO_FAR_AWAY");
    }

    /// The pending use is re-checked on arrival: the phase, the view generation and a craft
    /// permission that changed on the way drop it (native `FUN_393b3ec0` stores only the action;
    /// the port stores what the pick was checked against).
    #[test]
    fn a_pending_use_that_changed_on_the_way_is_dropped() {
        let arrival =
            |phase: u32, generation: Option<u64>, craft_perm_id: u8, changed_phase: u32| {
                let bridge = Shared::default();
                {
                    let mut d = bridge.borrow_mut();
                    d.character_id = 47;
                    d.pos = Vec3::ZERO;
                    d.player_radius = 0.0;
                    assert!(d.game.0.set(craft_tables(1088, craft_perm_id)).is_ok());
                    d.doodad.pending = Some(Pending {
                        obj_id: 300,
                        action: i64::from(CRAFT_ACTION),
                        phase,
                        generation,
                    });
                    d.doodad.arrived = Some(300);
                }
                let mut doodads = Doodads::default();
                doodads
                    .by_id
                    .insert(300, world_doodad(300, 568, changed_phase, Vec3::ZERO));
                let mut views = Views::default();
                views.fixture_doodad(300, Entity::PLACEHOLDER);
                let mut app = pending_app(bridge.clone(), doodads, Units::default(), views);
                app.update();
                (bridge, app)
            };

        let (bridge, mut app) = arrival(1088, Some(1), craft_perm::ANY, 1088);
        assert_eq!(
            crate::hud::crafting::active_bench(&bridge.borrow()),
            Some((300, 568)),
            "matching pending opens"
        );
        assert!(sent_commands(&mut app).is_empty(), "the open is local");

        let (bridge, _) = arrival(1088, Some(7), craft_perm::ANY, 1088);
        assert!(
            crate::hud::crafting::active_bench(&bridge.borrow()).is_none(),
            "a recreated doodad drops it"
        );

        let (bridge, _) = arrival(1088, Some(1), craft_perm::ANY, 4403);
        assert!(
            crate::hud::crafting::active_bench(&bridge.borrow()).is_none(),
            "a changed phase drops it"
        );

        let (bridge, _) = arrival(1088, Some(1), craft_perm::OWNER, 1088);
        assert!(
            crate::hud::crafting::active_bench(&bridge.borrow()).is_none(),
            "a permission that no longer passes drops it"
        );
    }

    /// A different bar or list pick cancels the pending open and the walk it was holding.
    #[test]
    fn a_different_pick_cancels_the_pending_open() {
        let bridge = Shared::default();
        {
            let mut d = bridge.borrow_mut();
            d.character_id = 47;
            d.pos = Vec3::ZERO;
            d.player_radius = 0.0;
            assert!(
                d.game
                    .0
                    .set(mixed_tables(1088, 14602, craft_perm::ANY))
                    .is_ok()
            );
            d.doodad.pending = Some(Pending {
                obj_id: 300,
                action: i64::from(CRAFT_ACTION),
                phase: 1088,
                generation: Some(1),
            });
            d.doodad.approach = Some(Approach {
                obj_id: 300,
                best: 3.0,
                best_at: 0.0,
            });
            d.doodad.menu = Some((300, Vec::new()));
            d.doodad.chosen.push(14602);
        }
        let mut doodads = Doodads::default();
        doodads
            .by_id
            .insert(300, world_doodad(300, 568, 1088, Vec3::ZERO));
        let mut views = Views::default();
        views.fixture_doodad(300, Entity::PLACEHOLDER);
        let mut app = pending_app(bridge.clone(), doodads, Units::default(), views);
        app.update();
        {
            let d = bridge.borrow();
            assert!(d.doodad.pending.is_none(), "the pending bench is cancelled");
            assert!(d.doodad.approach.is_none(), "its walk is cancelled");
            assert!(crate::hud::crafting::active_bench(&d).is_none());
        }
        assert!(
            !app.world()
                .resource::<crate::ingame::DevOptions>()
                .script_keys
                .contains('W')
        );
        let sent = sent_commands(&mut app);
        assert!(
            matches!(
                sent.as_slice(),
                [Command::UseDoodadSkill {
                    skill_id: 14602,
                    doodad_obj_id: 300,
                }]
            ),
            "{sent:?}"
        );
    }

    #[test]
    fn a_new_click_wins_over_a_pending_use_arriving_this_frame() {
        let bridge = Shared::default();
        {
            let mut d = bridge.borrow_mut();
            d.character_id = 47;
            d.pos = Vec3::ZERO;
            let mut game = craft_tables(1088, craft_perm::ANY);
            game.doodad.add_full(2, 14602, "DoodadFuncUse", 0, 0, 0, 0, &local::LocalFuncs::default());
            assert!(d.game.0.set(game).is_ok());
            d.doodad.pending = Some(Pending {
                obj_id: 300,
                action: i64::from(CRAFT_ACTION),
                phase: 1088,
                generation: None,
            });
            d.doodad.arrived = Some(300);
            d.doodad.pointer_pick = Some(400);
        }
        let mut doodads = Doodads::default();
        doodads.by_id.insert(300, world_doodad(300, 568, 1088, Vec3::ZERO));
        doodads.by_id.insert(400, world_doodad(400, 1, 2, Vec3::ZERO));
        let mut app = pending_app(bridge.clone(), doodads, Units::default(), Views::default());
        app.update();
        assert!(crate::hud::crafting::active_bench(&bridge.borrow()).is_none());
        assert!(matches!(sent_commands(&mut app).as_slice(), [Command::UseDoodadSkill {
            skill_id: 14602, doodad_obj_id: 400,
        }]));
        assert!(bridge.borrow().doodad.pending.is_none());
    }

    /// Native `FUN_3932aaf0` walks the player's own house list (`DAT_3a167ec4`); the port used
    /// `Houses::by_tl_id`, which `UnitsRemoved` empties when a house leaves view. The house's zone
    /// group is kept per own house, so permission 8 still matches out of view.
    #[test]
    fn an_own_house_keeps_its_zone_after_it_leaves_view() {
        let mut data = HudData {
            character_id: 47,
            account_id: 7,
            ..HudData::default()
        };
        let mut game = GameData::default();
        game.zones.insert(129, (5, 148));
        assert!(data.game.0.set(game).is_ok());
        let world =
            aa_world::World::parse_g("name test\ncellX 1\ncellY 1\ncell (x 0, y 0, zones 129)");
        let own = house_snapshot(12, 0x010300, 100.0, 100.0);
        let mut houses = aa_net::Houses::default();
        assert!(
            houses.apply(&aa_net::Event::Housing(aa_net::HousingEvent::MyHouse(
                own.clone()
            )))
        );
        sync_own_house_zones(&mut data, Some(&houses), Some(&world));
        assert_eq!(data.doodad.own_house_zones.get(&12), Some(&5));

        // the house unit leaves view: the in-view table loses it, the own list and zone stay
        assert!(houses.apply(&aa_net::Event::UnitsRemoved(vec![own.obj_id])));
        assert!(houses.by_tl_id.is_empty());
        sync_own_house_zones(&mut data, Some(&houses), Some(&world));
        let env = PermEnv {
            houses: Some(&houses),
            world: Some(&world),
        };
        let player = player_facts(&data, env);
        assert_eq!(player.house_zone_groups, Some(vec![5]));
        assert!(
            craft_permitted(
                craft_perm::ZONE_RESIDENTS,
                &player,
                &perm_doodad(5, 0, 0, 0, Some(5))
            ),
            "the resident keeps the zone with the house out of view"
        );

        // the world the house stands in is unloaded: the cached zone stays
        sync_own_house_zones(&mut data, Some(&houses), None);
        assert_eq!(data.doodad.own_house_zones.get(&12), Some(&5));
    }

    /// Native `393bd290` lists the craft action through the bar's candidate set; the 2.1 m use
    /// range (`FUN_393b3ec0` -> `FUN_39093800`) only decides whether the use walks.
    #[test]
    fn a_workbench_craft_is_offered_at_the_bar_distance_and_its_use_walks_then_opens() {
        let mut data = HudData {
            character_id: 47,
            pos: Vec3::ZERO,
            player_radius: 0.0,
            ..HudData::default()
        };
        assert!(data.game.0.set(craft_tables(1088, craft_perm::ANY)).is_ok());
        data.doodad
            .bounds
            .insert(300, (Vec3::new(-0.5, -0.5, 0.0), Vec3::new(0.5, 0.5, 1.0)));
        let mut doodads = Doodads::default();
        doodads.by_id.insert(300, world_doodad(300, 568, 1088, Vec3::new(0.0, 3.0, 0.0)));
        let units = Units::default();
        let focus = find_focus(&data, &doodads, &units, PermEnv::default()).expect("the bench is on the bar at 3 m");
        assert_eq!(focus.skills, vec![CRAFT_ACTION]);
        let mut dev = crate::ingame::DevOptions::default();
        assert!(!right_click(&mut data, &doodads, &units, 300, 0.0, &mut dev, PermEnv::default()));
        assert_eq!(data.doodad.chosen, vec![i64::from(CRAFT_ACTION)]);
        assert!(data.doodad.approach.is_none(), "the list is offered before the walk");
        assert!(use_bench(&mut data, &doodads, &units, 300, 1.0, None).is_none());
        assert!(data.doodad.approach.is_some(), "the out-of-range use walks");
        assert_eq!(
            data.doodad.pending,
            Some(Pending {
                obj_id: 300,
                action: i64::from(CRAFT_ACTION),
                phase: 1088,
                generation: None,
            })
        );
        assert!(crate::hud::crafting::active_bench(&data).is_none(), "the bench opens on arrival");
        data.pos = Vec3::new(0.0, 1.0, 0.0);
        assert!(use_bench(&mut data, &doodads, &units, 300, 2.0, None).is_some());
        assert_eq!(crate::hud::crafting::active_bench(&data), Some((300, 568)));
    }

    /// A phase whose only craft func is refused offers nothing: no empty interaction list.
    #[test]
    fn a_refused_craft_only_phase_does_not_open_an_empty_menu() {
        let mut data = HudData {
            character_id: 47,
            pos: Vec3::ZERO,
            player_radius: 0.0,
            ..HudData::default()
        };
        assert!(data.game.0.set(craft_tables(1088, craft_perm::OWNER)).is_ok());
        let mut doodads = Doodads::default();
        doodads.by_id.insert(300, world_doodad(300, 568, 1088, Vec3::ZERO));
        let mut dev = crate::ingame::DevOptions::default();
        assert!(!right_click(&mut data, &doodads, &Units::default(), 300, 0.0, &mut dev, PermEnv::default()));
        assert!(data.doodad.menu.is_none(), "no empty interaction list");
        assert!(data.doodad.chosen.is_empty());
        assert!(data.doodad.approach.is_none(), "nothing to walk for");
    }

    /// Native `FUN_393b7b10` runs `FUN_393b7aa0` first, so a second bench fires `INTERACTION_END`
    /// before its `CRAFTING_START`.
    #[test]
    fn opening_a_second_bench_fires_interaction_end_first() {
        let pak = std::env::var("AA_PAK").unwrap_or_else(|_| aa_pak::DEFAULT_PAK.to_string());
        if !std::path::Path::new(&pak).is_file() {
            eprintln!("skip: no game_pak at {pak}");
            return;
        }
        let rt = UiRuntime::new("en_us", 800.0, 600.0).unwrap();
        rt.lua
            .load(
                "EVENTS='';local w=UIParent:CreateWidget('emptywidget','benchProbe','UIParent');                 w:RegisterEvent('INTERACTION_END');w:RegisterEvent('CRAFTING_START');                 w:SetHandler('OnEvent',function(_,event) EVENTS=EVENTS..event..';' end)",
            )
            .exec()
            .unwrap();
        let bridge = Shared::default();
        {
            let mut d = bridge.borrow_mut();
            d.character_id = 47;
            d.pos = Vec3::ZERO;
            d.player_radius = 0.0;
            assert!(d.game.0.set(craft_tables(1088, craft_perm::ANY)).is_ok());
        }
        let mut doodads = Doodads::default();
        doodads.by_id.insert(300, world_doodad(300, 568, 1088, Vec3::ZERO));
        doodads.by_id.insert(400, world_doodad(400, 2241, 1088, Vec3::ZERO));
        let mut app = craft_app(rt, bridge.clone(), doodads, Units::default());
        let open = |app: &mut App, bridge: &Shared, id: u32| {
            let mut d = bridge.borrow_mut();
            d.doodad.menu = Some((id, Vec::new()));
            d.doodad.chosen.push(i64::from(CRAFT_ACTION));
            drop(d);
            app.update();
        };
        open(&mut app, &bridge, 300);
        let rt = app.world().non_send::<UiRuntime>();
        rt.lua.load("assert(EVENTS=='CRAFTING_START;','first open: '..EVENTS)").exec().unwrap();
        open(&mut app, &bridge, 400);
        assert_eq!(crate::hud::crafting::active_bench(&bridge.borrow()), Some((400, 2241)));
        let rt = app.world().non_send::<UiRuntime>();
        rt.lua
            .load("assert(EVENTS=='CRAFTING_START;INTERACTION_END;CRAFTING_START;','second open: '..EVENTS)")
            .exec()
            .unwrap();
    }

    /// Native `FUN_393b7b10` closes any active interaction (`FUN_393b7aa0` -> event 0xd2) before
    /// installing kind 3; a shown doodad interaction list or an open coffer is one, not only
    /// another bench.
    #[test]
    fn opening_a_bench_closes_a_shown_interaction_list_or_coffer_first() {
        let bridge = Shared::default();
        {
            let mut d = bridge.borrow_mut();
            d.character_id = 47;
            d.pos = Vec3::ZERO;
            d.player_radius = 0.0;
            assert!(d.game.0.set(craft_tables(1088, craft_perm::ANY)).is_ok());
        }
        let mut doodads = Doodads::default();
        doodads.by_id.insert(300, world_doodad(300, 568, 1088, Vec3::ZERO));
        let mut inventory = Inventory::default();
        inventory.coffer.obj_id = 900;
        let mut app = App::new();
        app.insert_non_send(HudBridge(bridge.clone()))
            .insert_resource(doodads)
            .insert_resource(Units::default())
            .insert_resource(Views::default())
            .insert_resource(crate::ingame::DevOptions::default())
            .insert_resource(inventory)
            .add_message::<NetCommand>()
            .add_message::<UseFailed>()
            .add_systems(Update, execute);
        {
            let mut d = bridge.borrow_mut();
            d.doodad.menu_shown = true;
            d.doodad.menu = Some((300, Vec::new()));
            d.doodad.chosen.push(i64::from(CRAFT_ACTION));
        }
        app.update();
        assert_eq!(
            crate::hud::crafting::active_bench(&bridge.borrow()),
            Some((300, 568))
        );
        assert!(
            !bridge.borrow().doodad.menu_shown,
            "the list is closed by the bench"
        );
        assert!(
            bridge.borrow().doodad.coffer_close,
            "the coffer is closed too"
        );

        app.update();
        let sent = app
            .world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .map(|message| message.0)
            .collect::<Vec<_>>();
        assert!(
            matches!(
                sent.as_slice(),
                [Command::Coffer(CofferCommand::Interact {
                    obj_id: 900,
                    open: false,
                })]
            ),
            "{sent:?}"
        );
        assert!(!app.world().resource::<Inventory>().coffer.is_open());
    }

    /// Picking a list entry closes the list (`interaction.lua` picks one row), so a later bench
    /// open must not treat it as still shown and fire a spurious INTERACTION_END.
    #[test]
    fn picking_an_interaction_entry_clears_the_shown_list() -> mlua::Result<()> {
        let lua = aa_lua::new_lua()?;
        for name in ["X2", "X2Interaction", "HUD", "X2Coffer", "X2Bag"] {
            lua.globals().set(name, lua.create_table()?)?;
        }
        let bridge = Shared::default();
        install(&lua, &bridge)?;
        {
            let mut d = bridge.borrow_mut();
            d.doodad.menu = Some((
                300,
                vec![Entry {
                    id: 14602,
                    name: "Gather".into(),
                }],
            ));
            d.doodad.menu_shown = true;
        }
        lua.load("X2Interaction:SelectInteraction(14602)").exec()?;
        let d = bridge.borrow();
        assert_eq!(d.doodad.chosen, vec![14602]);
        assert!(!d.doodad.menu_shown, "the pick closes the list");
        assert!(
            d.doodad.menu.is_some(),
            "execute still needs the menu until it consumes it"
        );
        Ok(())
    }

    fn craft_app(rt: UiRuntime, bridge: Shared, doodads: Doodads, units: Units) -> App {
        let mut app = App::new();
        app.insert_non_send(rt)
            .insert_non_send(HudBridge(bridge))
            .insert_resource(units)
            .insert_resource(doodads)
            .insert_resource(Views::default())
            .insert_resource(crate::ingame::DevOptions::default())
            .insert_resource(Inventory::default())
            .init_resource::<NextState<crate::Screen>>()
            .init_resource::<aa_net::state::ActionBar>()
            .init_resource::<aa_net::state::Target>()
            .init_resource::<aa_net::state::Skills>()
            .init_resource::<aa_net::state::Buffs>()
            .init_resource::<crate::hud::InFlight>()
            .add_message::<NetCommand>()
            .add_message::<UseFailed>()
            .add_message::<crate::ingame::ground_target::SkillPressed>()
            .add_systems(Update, execute);
        app
    }
}
