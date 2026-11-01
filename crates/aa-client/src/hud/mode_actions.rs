//! Received buff skills and the original mode bar. Native refresh publishes the
//! buff vector before539, then rebuilds the capped actions and enables mode1 before95.
use std::collections::{HashMap, HashSet, VecDeque};

use aa_net::{Event, buff::BuffKind, state::BuffEntry};
use aa_ui::UiRuntime;
use mlua::{Lua, Table, Value};
use rusqlite::Connection;

use super::api::{HudData, Shared};

pub(super) mod fixture;
#[cfg(test)]
mod tests;

/// A skill of a slave's mode bar: the combat skill and its `mount_skills` row (the id a slave cast carries).
/// The capture's Farm Wagon (slave template 60) reads Owner's Mark 20704, First Round in New Morning 20291
/// (10 s cooldown), Sound Horn 15622, Water Plant 15450 and Create Water 15546.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SlaveSkill {
    /// The skill the bar shows (for a gunner this is the `mount_attached_skills` rider skill,
    /// 14372/24501/20161).
    pub skill: u32,
    /// The `mount_skills` row a slave cast carries.
    pub mount_skill: u32,
    /// The `mount_skills.skill_id` the bar's slot actually casts. For a gunner this is the real
    /// fire skill (10473/24500/20160), not the rider presentation skill the bar shows; `None`
    /// for a normal slave bar, whose `skill` is cast directly.
    pub fire: Option<u32>,
}

impl SlaveSkill {
    const fn new(skill: u32, mount_skill: u32) -> Self {
        Self {
            skill,
            mount_skill,
            fire: None,
        }
    }

    const fn gunner(skill: u32, mount_skill: u32, fire: u32) -> Self {
        Self {
            skill,
            mount_skill,
            fire: Some(fire),
        }
    }

    /// The skill the packet names (`StartSiegeWeaponSkill` sends the mount skill's own skill).
    pub fn cast_skill(&self) -> u32 {
        self.fire.unwrap_or(self.skill)
    }
}

/// A native mode-bar entry of kind 3 (`FUN_39204320`): a function button with a click stub,
/// not a skill. The icon and the `ui_texts` key are the strings `FUN_39204500` /
/// `FUN_392045f0` pass in; the text category is the third argument (`0x21` = 33,
/// `0x13` = 19).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Function {
    /// Slot 6 of any slave seat: `CSDiscardSlave` 0x032 (`FUN_39345aa0`).
    Unbind,
    /// Slot 7 of the driver seat (two undecoded gates): UI event `SHOW_SLAVE_INFO`.
    ShowSlaveInfo,
    /// First slot while riding a mate: mate command 9 (`FUN_39361e80`), the pet-bar dismount.
    CommandUnmount,
}

impl Function {
    /// The `ui_texts` key `FUN_39204320` is called with.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Function::Unbind => "unbind",
            Function::ShowSlaveInfo => "showSlaveInfo",
            Function::CommandUnmount => "command_unmount",
        }
    }

    /// The `ui_texts.category_id` of the key.
    fn category(self) -> i64 {
        match self {
            Function::Unbind | Function::ShowSlaveInfo => 33,
            Function::CommandUnmount => 19,
        }
    }

    /// The icon path the native passes (`strings-xrefs.tsv`: 0x399bab3c / 0x399bab70).
    pub(crate) fn icon(self) -> &'static str {
        match self {
            Function::Unbind | Function::CommandUnmount => "ui/icon/icon_skill_vehicle03.dds",
            Function::ShowSlaveInfo => "ui/icon/icon_skill_info_view.dds",
        }
    }

    /// A stable slot id for the HUD's cached slot table (not a skill).
    pub(crate) fn id(self) -> u32 {
        match self {
            Function::Unbind => 1,
            Function::ShowSlaveInfo => 2,
            Function::CommandUnmount => 3,
        }
    }
}

/// One slot of the mode bar as the native builds it (`FUN_39204500` / `FUN_392045f0`):
/// the buff skills, a slave's mount skills, or a function button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Entry {
    /// A received buff skill (kind 1, `FUN_392044a0`).
    Player(u32),
    /// A slave's mount skill (kind 2, `FUN_39204500`); cast as a `Mount` caster.
    Skill(SlaveSkill),
    /// A function button (kind 3, `FUN_39204320`).
    Function(Function),
}

#[derive(Default)]
pub(crate) struct Tables {
    skills: HashMap<u32, Vec<u32>>,
    /// `slave_mount_skills` per slave template id, in row order: the driver's mode bar (1.2's
    /// `GetSlaveMountSkillList`; the capture's Farm Wagon order matches the row ids).
    slave_skills: HashMap<u32, Vec<SlaveSkill>>,
    /// `(slave template, mount skill, attach point)` -> its `mount_attached_skills.skill_id`.
    /// Point 1 is the driver/gunner seat; the same table has point-2 rows for the tank seats
    /// (`FUN_39347260` filters the list by the seat's point).
    attached: HashMap<(u32, u32, u8), u32>,
    /// `(slave template, mount skill)` -> the attach point of the row's FIRST
    /// `mount_attached_skills` entry (lowest id). A row with an attached list is counted only at
    /// that point; a row without one only at point 1 (`FUN_39347260` / `FUN_39347130`).
    first_attached: HashMap<(u32, u32), u8>,
    /// `slaves.id` -> `slaves.level`, the slave unit's level byte `+0x4b4` in the bar's gate.
    /// Empty when the schema has no `level` column (test fixtures): the gate then passes.
    slave_levels: HashMap<u32, u8>,
    /// `skills.id` -> `skills.ability_level`, the skill's `+8` in the same gate.
    skill_levels: HashMap<u32, u8>,
    /// Slave templates whose point-1 seat is a turret (kind 7/8, `vehicle_models.velocity == 0`
    /// and a non-zero turret range): only their rows show the point-1 attached skill. This is
    /// the same rule as `ingame::turret::turret_seat_model`, kept here so the HUD can build a
    /// bar without the engine's seat resource.
    turret_slaves: HashSet<u32>,
    /// `slaves.id` -> `slaves.model_id`.
    slave_models: HashMap<u32, u32>,
    /// Model ids with a `model_bindings` row at attach point 2 (`$passenger0`).
    point2_models: HashSet<u32>,
    /// The function buttons' `ui_texts` text, keyed by (category, key).
    texts: HashMap<(i64, String), String>,
}

impl Tables {
    pub(crate) fn load(db: &Connection) -> rusqlite::Result<Self> {
        let ids = |table: &str| -> rusqlite::Result<HashSet<u32>> {
            db.prepare(&format!("SELECT id FROM {table}"))?
                .query_map([], |row| row.get(0))?
                .collect()
        };
        let buffs = ids("buffs")?;
        let skills = ids("skills")?;
        let mut out = Self::default();
        // Original395b16e0: SQL order is observable, including duplicate pairs.
        let mut query = db.prepare("SELECT buff_id, skill_id FROM buff_skills")?;
        for row in query.query_map([], |row| Ok((row.get::<_, u32>(0)?, row.get::<_, u32>(1)?)))? {
            let (buff, skill) = row?;
            if buffs.contains(&buff) && skills.contains(&skill) {
                out.skills.entry(buff).or_default().push(skill);
            }
        }
        // ServeGameData.GetSlaveMountSkillList: the slave's rows in table order, each through mount_skills.
        // Rows without a skill are dropped here, so the bar's slots and this list stay index-aligned.
        let mut query = db.prepare(
            "SELECT sm.slave_id, ms.skill_id, sm.mount_skill_id FROM slave_mount_skills sm
             JOIN mount_skills ms ON ms.id = sm.mount_skill_id
             WHERE sm.slave_id IS NOT NULL AND ms.skill_id > 0 ORDER BY sm.id",
        )?;
        for row in query.query_map([], |row| {
            Ok((
                row.get::<_, u32>(0)?,
                row.get::<_, u32>(1)?,
                row.get::<_, u32>(2)?,
            ))
        })? {
            let (slave, skill, mount_skill) = row?;
            out.slave_skills
                .entry(slave)
                .or_default()
                .push(SlaveSkill::new(skill, mount_skill));
        }
        // The per-row attached skill, for every seat point: point 1 is the operator/gunner, and
        // the point-2 rows are the passenger skills (`FUN_39347260` filters by the seat point).
        // A row without one keeps its mount skill (the server's `GetMountAttachedSkills` is per
        // attach point). The row id order makes `first_attached` the native row's FIRST entry.
        let mut query = db.prepare(
            "SELECT sm.slave_id, sm.mount_skill_id, mas.attach_point_id, mas.skill_id
             FROM slave_mount_skills sm
             JOIN mount_attached_skills mas ON mas.mount_skill_id = sm.mount_skill_id
             WHERE sm.slave_id IS NOT NULL AND mas.skill_id > 0
             ORDER BY sm.id, mas.id",
        )?;
        for row in query.query_map([], |row| {
            Ok((
                row.get::<_, u32>(0)?,
                row.get::<_, u32>(1)?,
                row.get::<_, u8>(2)?,
                row.get::<_, u32>(3)?,
            ))
        })? {
            let (slave, mount_skill, point, skill) = row?;
            out.attached.insert((slave, mount_skill, point), skill);
            out.first_attached
                .entry((slave, mount_skill))
                .or_insert(point);
        }
        // The native level gate in `FUN_39347260`: the skill's level (`+8`,
        // `skills.ability_level`) must not exceed the slave unit's level byte (`+0x4b4`,
        // `slaves.level`). The unit there is the slave itself (`FUN_39345f10`); one 1.2 row is
        // above it (slave 44's skill 13572, ability level 99 against a level-50 slave).
        // A schema without these columns left the gate open (test fixtures).
        if let Ok(mut query) = db.prepare("SELECT id, ability_level FROM skills") {
            for row in query.query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, i64>(1)?)))? {
                let (skill, level) = row?;
                out.skill_levels.insert(skill, level.clamp(0, 255) as u8);
            }
        }
        if let Ok(mut query) = db.prepare("SELECT id, level FROM slaves") {
            for row in query.query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, i64>(1)?)))? {
                let (slave, level) = row?;
                out.slave_levels.insert(slave, level.clamp(0, 255) as u8);
            }
        }
        // The same gate as `ingame::turret::turret_seat_model`: kind 7/8, a numeric
        // `vehicle_models.velocity` of 0, and a non-zero parsed pitch/yaw range. The text
        // `f`/`t` values in other columns are not angles and are excluded by `typeof`.
        let mut query = db.prepare(
            "SELECT s.id FROM slaves s
             JOIN models m ON m.id = s.model_id
             JOIN vehicle_models v ON v.id = m.sub_id AND m.sub_type = 'VehicleModel'
             WHERE s.slave_kind_id IN (7, 8)
               AND typeof(v.velocity) IN ('integer', 'real') AND v.velocity = 0
               AND typeof(v.turret_pitch_angle_min) IN ('integer', 'real')
               AND typeof(v.turret_pitch_angle_max) IN ('integer', 'real')
               AND typeof(v.turret_yaw_angle_min) IN ('integer', 'real')
               AND typeof(v.turret_yaw_angle_max) IN ('integer', 'real')
               AND typeof(v.turret_yaw_angvel) IN ('integer', 'real')
               AND typeof(v.turret_pitch_angvel) IN ('integer', 'real')
               AND v.turret_pitch_angle_min <= v.turret_pitch_angle_max
               AND v.turret_yaw_angle_min <= v.turret_yaw_angle_max
               AND (v.turret_pitch_angle_min != v.turret_pitch_angle_max
                    OR v.turret_yaw_angle_min != v.turret_yaw_angle_max)",
        )?;
        for row in query.query_map([], |row| row.get::<_, u32>(0))? {
            out.turret_slaves.insert(row?);
        }
        // The 1.2 client's slave-candidate builder asks the model's attach-point table for point
        // 2 (`FUN_390c9f70`): the only per-model attach list is `model_bindings`. A test or
        // partial database without these tables simply gets no passenger offer.
        if let Ok(mut query) = db.prepare("SELECT id, model_id FROM slaves") {
            for row in query.query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, u32>(1)?)))? {
                let (slave, model) = row?;
                out.slave_models.insert(slave, model);
            }
        }
        if let Ok(mut query) = db.prepare(
            "SELECT owner_id FROM model_bindings WHERE owner_type = 'Model' AND attach_point_id = 2",
        ) {
            for row in query.query_map([], |r| r.get::<_, u32>(0))? {
                out.point2_models.insert(row?);
            }
        }
        // The mode bar's function tooltips: the keys are the ones `FUN_39204320` receives.
        // A database without the text tables keeps the key as the text.
        if let Ok(mut query) = db.prepare(
            "SELECT u.category_id, u.key, coalesce(l.en_us, u.text) FROM ui_texts u
             LEFT JOIN localized_texts l ON l.tbl_name = 'ui_texts'
                 AND l.tbl_column_name = 'text' AND l.idx = u.id
             WHERE (u.category_id = 33 AND u.key IN ('unbind', 'showSlaveInfo'))
                 OR (u.category_id = 19 AND u.key = 'command_unmount')",
        ) {
            for row in query.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            })? {
                let (category, key, text) = row?;
                if let Some(text) = text {
                    out.texts.insert((category, key), text);
                }
            }
        }
        Ok(out)
    }

    /// Whether the slave's model has a `$passenger0` attach point (`FUN_390c9f70(2)`).
    pub(crate) fn has_passenger_point(&self, template: u32) -> bool {
        self.slave_models
            .get(&template)
            .is_some_and(|model| self.point2_models.contains(model))
    }

    /// The function button's `ui_texts` text, or its key when the database has no row.
    pub(crate) fn text(&self, function: Function) -> String {
        self.texts
            .get(&(function.category(), function.key().to_string()))
            .cloned()
            .unwrap_or_else(|| function.key().to_string())
    }

    #[cfg(test)]
    pub(crate) fn fixture_passenger_point(&mut self, template: u32, model: u32, has: bool) {
        self.slave_models.insert(template, model);
        if has {
            self.point2_models.insert(model);
        }
    }

    fn turret_slave(&self, template: u32) -> bool {
        self.turret_slaves.contains(&template)
    }

    /// The slave's bar, one entry per matching `slave_mount_skills` row. `FUN_39347260` /
    /// `FUN_39347130` filter every seat, the driver's included: a row without an attached list
    /// counts only at point 1, and a row with one only when its FIRST `mount_attached_skills`
    /// entry is at `point`; the row's skill level must also not exceed the slave's level. On a
    /// turret point-1 seat a kept row with a point-1 attached skill shows that skill and casts
    /// the mount skill; a passenger seat shows the row's attached skill the same way. `None` when
    /// the slave has no rows at all (the native still appends its function slots; the caller uses
    /// `unwrap_or_default`).
    pub(crate) fn bar(&self, template: u32, point: u8, turret: bool) -> Option<Vec<SlaveSkill>> {
        let rows = self.slave_skills.get(&template)?;
        if rows.is_empty() {
            return None;
        }
        let level = self.slave_levels.get(&template).copied().unwrap_or(u8::MAX);
        Some(
            rows.iter()
                .filter(|row| {
                    let skill_level = self.skill_levels.get(&row.skill).copied().unwrap_or(1);
                    if skill_level > level {
                        return false;
                    }
                    match self.first_attached.get(&(template, row.mount_skill)) {
                        Some(&first) => first == point,
                        None => point == 1,
                    }
                })
                .map(|row| {
                    if turret
                        && point == 1
                        && let Some(skill) = self.attached.get(&(template, row.mount_skill, 1))
                    {
                        return SlaveSkill::gunner(*skill, row.mount_skill, row.skill);
                    }
                    if point > 1
                        && let Some(skill) = self.attached.get(&(template, row.mount_skill, point))
                    {
                        return SlaveSkill::gunner(*skill, row.mount_skill, row.skill);
                    }
                    *row
                })
                .collect(),
        )
    }

    /// The turret-seat view of [`Tables::bar`], used by the fire-path tests and the example.
    #[allow(dead_code)]
    pub(crate) fn gunner_bar(&self, template: u32) -> Option<Vec<SlaveSkill>> {
        self.bar(template, 1, true)
    }

    fn has(&self, buff: u32) -> bool {
        self.skills.get(&buff).is_some_and(|v| !v.is_empty())
    }

    fn collect(&self, buffs: &[BuffEntry], exclude: u32) -> Vec<u32> {
        [BuffKind::Good, BuffKind::Bad, BuffKind::Hidden]
            .into_iter()
            .flat_map(|kind| {
                buffs
                    .iter()
                    .filter(move |b| b.buff.kind == kind && b.buff.index != exclude)
            })
            .flat_map(|b| {
                self.skills
                    .get(&b.buff.buff_id)
                    .into_iter()
                    .flatten()
                    .copied()
            })
            .collect()
    }
}

#[derive(Default)]
pub(super) struct View {
    context: Option<(u32, u32)>,
    epoch: u64,
    buff_skills: Option<Vec<u32>>,
    /// A slave seat's bar while the local player sits on any of its seats; it replaces the buff
    /// skills until the player leaves (`FUN_39345f10` returns any slave seat, not only point 1).
    vehicle: Option<Vec<Entry>>,
    /// The player rides a mate that is not their own: its `command_unmount` function slot comes
    /// first (`FUN_392045f0` adds it only when `FUN_3933b1b0(parent) == 0`).
    mate: bool,
    actions: Vec<Entry>,
    slots: Vec<super::panels::SlotSnap>,
    slots_dirty: bool,
    ui_dirty: bool,
    pending: VecDeque<Pending>,
    /// A function click asked for the Summon Info window: `ready` fires `SHOW_SLAVE_INFO` once.
    show_slave_info: bool,
    /// Refusals of function clicks (`ErrorMessageType` ids): `ready` shows them as `SYSMSG`.
    refusals: Vec<i16>,
}

/// Which bar a queued payload is: only buff payloads replace the buff skills; a seat payload is the slave
/// bar and must not be mistaken for buff state (the `lost` check and the restore on leaving read the buffs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    Buffs,
    Seat,
}

struct Pending {
    epoch: u64,
    skills: Option<Vec<u32>>,
    notify: bool,
    source: Source,
}

pub(super) fn actions(data: &HudData) -> &[Entry] {
    &data.mode_actions.actions
}

pub(super) fn removal(data: &HudData, event: &Event) -> Option<u32> {
    let Event::BuffRemoved { obj_id, index } = event else {
        return None;
    };
    let (_, actor) = super::user_music::own_identity(data)?;
    if *obj_id != actor {
        return None;
    }
    let buffs = super::user_music::own_buffs(data)?;
    let first = [BuffKind::Good, BuffKind::Bad, BuffKind::Hidden]
        .into_iter()
        .find_map(|kind| {
            buffs
                .iter()
                .find(|b| b.buff.kind == kind && b.buff.index == *index)
        })?;
    data.game
        .get()?
        .mode_actions
        .has(first.buff.buff_id)
        .then_some(*index)
}

fn current(data: &HudData, exclude: u32) -> Option<Vec<u32>> {
    Some(
        data.game
            .get()?
            .mode_actions
            .collect(super::user_music::own_buffs(data)?, exclude),
    )
}

/// The bar of the slave seat the local player sits on, built like `FUN_39204500`: the mount
/// skills attached to that seat point, then `unbind` on any seat, then `showSlaveInfo` on the
/// driver seat. A slave with no `slave_mount_skills` rows (a carriage, a rowboat) still gets the
/// function slots: the native appends `unbind` after the skill loop whether or not it ran.
/// `None` on foot, on a mate, or with the catalog not loaded yet.
fn seat_bar(data: &HudData) -> Option<Vec<Entry>> {
    let tables = &data.game.get()?.mode_actions;
    let (slave, template, kind, point) = data.mounted.slave_seat()?;
    // A turret seat is this slave when the engine's seat flag names it, or, before that flag
    // reaches the HUD (the attach event can arrive first), when the template passes the same
    // kind/velocity/range gate as `turret_seat_model`.
    let turret = point == 1
        && match data.turret_seat {
            Some(seated) if seated == slave => true,
            _ => tables.turret_slave(template),
        };
    let mut out: Vec<Entry> = tables
        .bar(template, point, matches!(kind, 7 | 8) && turret)
        .unwrap_or_default()
        .into_iter()
        .map(Entry::Skill)
        .collect();
    out.push(Entry::Function(Function::Unbind));
    if point == 1 {
        // The native gates this on `local_8 == 1` (the driver) and two undecoded conditions
        // (`slave+0x3af0 == DAT_39d1fb98`, `FUN_3901be50() == 0`); the driver seat is the
        // evidence-backed part, and the owner's capture shows it on the Farm Wagon.
        out.push(Entry::Function(Function::ShowSlaveInfo));
    }
    Some(out)
}

/// The function entry at `slot` (0 based) of the shown bar.
pub(super) fn function_at(data: &HudData, slot: usize) -> Option<Function> {
    match actions(data).get(slot) {
        Some(Entry::Function(function)) => Some(*function),
        _ => None,
    }
}

/// The slot snapshot of a function button: `kind` "function" is what the original
/// `GetBindedType` returns, and `desc` is the `ui_texts` text the script shows on hover.
fn function_slot(tables: Option<&Tables>, function: Function) -> super::panels::SlotSnap {
    let text = tables.map_or_else(
        || function.key().to_string(),
        |tables| tables.text(function),
    );
    super::panels::SlotSnap {
        kind: "function",
        id: function.id(),
        name: text.clone(),
        icon: function.icon().to_string(),
        desc: text,
        ..Default::default()
    }
}

/// Carries out a function slot. `Unbind` is the `FUN_39345aa0` path (the driver's 5 m/s refusal,
/// the passenger's 1500 ms throttle); `ShowSlaveInfo` queues the `SHOW_SLAVE_INFO` event;
/// `CommandUnmount` is the existing mate unmount (command 9). A refusal is queued for `ready`.
pub(super) fn run_function(
    data: &mut HudData,
    function: Function,
    now: web_time::Instant,
) -> super::slave::Outcome {
    let outcome = match function {
        Function::Unbind => super::slave::discard_at(data, now),
        Function::ShowSlaveInfo => {
            data.mode_actions.show_slave_info = true;
            super::slave::Outcome::Sent
        }
        Function::CommandUnmount => {
            super::pets::owned::unmount(data, false);
            super::slave::Outcome::Sent
        }
    };
    if let super::slave::Outcome::Refused(code) = outcome {
        refuse(data, code);
    }
    outcome
}

/// Queues an `ErrorMessageType` refusal for `ready` to show as `SYSMSG` (`slave::announce`).
pub(super) fn refuse(data: &mut HudData, code: i16) {
    data.mode_actions.refusals.push(code);
}

fn queue(data: &mut HudData, next: Option<Vec<u32>>, notify: bool, source: Source) {
    let context = super::user_music::own_identity(data);
    let view = &mut data.mode_actions;
    if view.context != context {
        view.context = context;
        view.epoch = view.epoch.wrapping_add(1);
        view.pending.clear();
        view.actions.clear();
        view.slots.clear();
        view.slots_dirty = true;
        view.buff_skills = None;
        data.input_modes.set(1, 0);
    }
    view.pending.push_back(Pending {
        epoch: view.epoch,
        skills: next,
        notify,
        source,
    });
}

/// Called after folding each actual receipt, including ordered catalog-startup replay.
pub(super) fn reduce(data: &mut HudData, event: &Event, removed: Option<u32>) {
    let own = super::user_music::own_identity(data).map(|(_, actor)| actor);
    // a slave seat's bar replaces the buff skills; a seat change (or leaving) republishes it
    let game = data.game.get().is_some();
    // Kept only once the catalog is loaded: a seat or mate taken before it would otherwise look
    // unchanged on the next receipt and its bar would never be queued.
    let seat = seat_bar(data);
    let seat_changed = game && data.mode_actions.vehicle != seat;
    if seat_changed {
        data.mode_actions.vehicle = seat;
    }
    let mate = data.mounted.mate_unmount();
    let mate_changed = game && data.mode_actions.mate != mate;
    if mate_changed {
        data.mode_actions.mate = mate;
    }
    let snapshot = matches!(event, Event::PlayerSpawn(_));
    let created = matches!(event, Event::BuffCreated { obj_id, buff }
        if Some(*obj_id) == own && data.game.get().is_some_and(|g| g.mode_actions.has(buff.buff_id))
            && super::user_music::own_buffs(data).is_some_and(|rows|
                rows.iter().any(|row| row.buff.index == buff.index && row.buff.buff_id == buff.buff_id)));
    let gap = matches!(event, Event::GameDecodeFailed { .. })
        || matches!(event, Event::UnhandledGamePacket { level, len, .. }
            if *level == aa_net::game::LEVEL_COMPRESSED && *len > 0);
    let retired = matches!(
        event,
        Event::CharacterSelected(_)
            | Event::ReadyToConnect { .. }
            | Event::LoginSucceeded(_)
            | Event::LeftWorld { .. }
            | Event::Disconnected { .. }
            | Event::Kicked { .. }
    ) || matches!(event, Event::UnitsRemoved(ids) if data.mode_actions.context.is_some_and(|(_, actor)| ids.contains(&actor)));
    let lost =
        data.mode_actions.buff_skills.is_some() && super::user_music::own_buffs(data).is_none();
    // Buff state changes on its own, whether or not a seat bar is shown; this keeps `buff_skills`
    // current for the restore on leaving and for the `lost` check (which must not repeat once it
    // has cleared).
    if game && (snapshot || created || removed.is_some() || gap || retired || lost) {
        let next = current(data, removed.unwrap_or(0));
        queue(
            data,
            next,
            snapshot || created || removed.is_some(),
            Source::Buffs,
        );
    } else if matches!(event, Event::OwnUnitSnapshot(u) if Some(u.obj_id) == own)
        && data.mode_actions.buff_skills.is_none()
        && game
    {
        // Existing native objects ignore repeated SCUnitState. The remake uses
        // a matched snapshot only to recover unknown history, without 539/95.
        let next = current(data, 0);
        queue(data, next, false, Source::Buffs);
    }
    // The shown bar follows the slave seat (or a mate's function slot) and the buff skills.
    if game && (seat_changed || mate_changed) {
        if data.mode_actions.vehicle.is_some() {
            // `flush` recomposes the seat bar from `vehicle`, so no payload is needed here.
            queue(data, None, true, Source::Seat);
        } else {
            let next = current(data, 0);
            queue(data, next, true, Source::Buffs);
        }
    }
    // BuffUpdated changes charges/stacks but native391d5f20 does not refresh this list.
}

fn append(actions: &mut Vec<Entry>, entries: &[Entry]) {
    for entry in entries {
        if actions.len() >= 15 {
            break;
        }
        match entry {
            Entry::Player(0) => {}
            _ => actions.push(*entry),
        }
    }
}

pub(super) fn flush(rt: Option<&UiRuntime>, state: &Shared) {
    loop {
        let pending = state.borrow_mut().mode_actions.pending.pop_front();
        let Some(Pending {
            epoch,
            skills,
            notify,
            source,
        }) = pending
        else {
            break;
        };
        {
            let mut data = state.borrow_mut();
            if data.mode_actions.epoch != epoch {
                continue;
            }
            if source == Source::Buffs {
                data.mode_actions.buff_skills = skills;
            }
        }
        if notify && let Some(rt) = rt {
            rt.fire_event("BUFF_SKILL_CHANGED", ());
        }
        {
            let mut data = state.borrow_mut();
            if data.mode_actions.epoch != epoch {
                continue;
            }
            // Read again after Lua: reentry may replace the current buff vector/context.
            let mut next = Vec::new();
            if let Some(entries) = &data.mode_actions.vehicle {
                append(&mut next, entries);
            } else {
                let mut entries = Vec::new();
                if data.mode_actions.mate {
                    entries.push(Entry::Function(Function::CommandUnmount));
                }
                if let Some(skills) = &data.mode_actions.buff_skills {
                    entries.extend(skills.iter().copied().map(Entry::Player));
                }
                append(&mut next, &entries);
            }
            data.input_modes.set(1, u32::from(!next.is_empty()));
            data.mode_actions.actions = next;
            data.mode_actions.ui_dirty = true;
        }
        if notify && let Some(rt) = rt {
            state.borrow_mut().mode_actions.ui_dirty = false;
            rt.fire_event("MODE_ACTIONS_UPDATE", ());
        }
    }
    // Loading may have consumed real receipts while its HUD was suspended.
    // Repaint once on return; do not replay539/95 or promote the input mode.
    let dirty = state.borrow().mode_actions.ui_dirty;
    if dirty && let Some(rt) = rt {
        state.borrow_mut().mode_actions.ui_dirty = false;
        let shown = rt.lua.load("local w=modeSkillActionBar;if w then w:ShowModeSkillActionBar();return true end;return false").eval::<bool>();
        match shown {
            Ok(true) => {}
            Ok(false) => state.borrow_mut().mode_actions.ui_dirty = true,
            Err(error) => {
                state.borrow_mut().mode_actions.ui_dirty = true;
                bevy::log::warn!("mode action recovery: {error}");
            }
        }
    }
}

pub(super) fn ready(rt: Option<&UiRuntime>, state: &Shared) {
    {
        let seat = seat_bar(&state.borrow());
        let mut data = state.borrow_mut();
        let mate = data.mounted.mate_unmount();
        // the catalog may finish loading while the player is already driving: publish the seat bar then
        if data.mode_actions.vehicle != seat || data.mode_actions.mate != mate {
            data.mode_actions.vehicle = seat;
            data.mode_actions.mate = mate;
            if data.mode_actions.vehicle.is_some() {
                queue(&mut data, None, false, Source::Seat);
            } else {
                let next = current(&data, 0);
                queue(&mut data, next, false, Source::Buffs);
            }
        }
        // Seed the buff state once even while a seat bar is shown: the bar is composed from the seat,
        // but `buff_skills` is what the `lost` check and the restore on leaving read, so leaving it
        // None would queue the bar again every frame (`ready` runs from `enchant::track` each frame).
        if data.mode_actions.buff_skills.is_none()
            && data.mode_actions.pending.is_empty()
            && let Some(next) = current(&data, 0)
        {
            queue(&mut data, Some(next), false, Source::Buffs);
        }
    }
    flush(rt, state);
    if let Some(rt) = rt {
        let (show, refusals) = {
            let mut data = state.borrow_mut();
            (
                std::mem::take(&mut data.mode_actions.show_slave_info),
                std::mem::take(&mut data.mode_actions.refusals),
            )
        };
        for code in refusals {
            super::slave::announce(rt, code);
        }
        if show {
            rt.fire_event("SHOW_SLAVE_INFO", ());
        }
    }
}

pub(super) fn install(lua: &Lua, state: &Shared) -> mlua::Result<()> {
    let unit: Table = lua.globals().get("X2Unit")?;
    let shared = state.clone();
    unit.set(
        "GetModeActionsCount",
        lua.create_function(move |_, _: Value| Ok(actions(&shared.borrow()).len()))?,
    )?;
    let hud: Table = lua.globals().get("HUD")?;
    let shared = state.clone();
    hud.set(
        "mode_slot",
        lua.create_function(move |lua, index: usize| {
            let data = shared.borrow();
            let Some(entry) = index
                .checked_sub(1)
                .and_then(|i| actions(&data).get(i))
                .copied()
            else {
                return Ok(None);
            };
            let mut slot = match entry {
                Entry::Function(function) => {
                    function_slot(data.game.get().map(|game| &game.mode_actions), function)
                }
                Entry::Skill(skill) => match data.mounted.slave_seat() {
                    Some((slave, ..)) => super::panels::slave_skill_slot(
                        data.game.get(),
                        Some(&data.cooldowns),
                        slave,
                        skill.skill,
                        data.start,
                    ),
                    None => super::panels::skill_slot(
                        data.game.get(),
                        Some(&data.cooldowns),
                        skill.skill,
                        data.start,
                    ),
                },
                Entry::Player(skill) => super::panels::skill_slot(
                    data.game.get(),
                    Some(&data.cooldowns),
                    skill,
                    data.start,
                ),
            };
            // The cache is the snapshot Lua last saw: it may still be the previous bar's slot
            // for one frame (a seat change clears the list after the swap), so only its own
            // kind and id may stand in for the slot computed here.
            if let Some(current) = data
                .mode_actions
                .slots
                .get(index - 1)
                .filter(|current| current.kind == slot.kind && current.id == slot.id)
            {
                slot = current.clone();
            }
            super::panels::slot_table(lua, &slot).map(Some)
        })?,
    )?;
    Ok(())
}

pub(super) fn refresh_slots(world: &bevy::prelude::World, data: &mut HudData) {
    let skills = world.get_resource::<aa_net::state::Skills>();
    let inventory = world.get_resource::<aa_net::state::Inventory>();
    let buffs = world
        .get_resource::<aa_net::state::Buffs>()
        .map_or(&[][..], |b| b.of(skills.map_or(0, |s| s.obj_id)));
    let env = super::usable::Env::of(data, skills, buffs, inventory);
    let seat = data.mounted.slave_seat().map(|(slave, ..)| slave);
    let tables = data.game.get().map(|game| &game.mode_actions);
    let slots = actions(data)
        .iter()
        .map(|entry| match entry {
            Entry::Function(function) => function_slot(tables, *function),
            Entry::Skill(skill) => match seat {
                Some(slave) => super::panels::slave_skill_slot(
                    data.game.get(),
                    skills,
                    slave,
                    skill.skill,
                    data.start,
                ),
                None => super::panels::skill_slot(data.game.get(), skills, skill.skill, data.start),
            },
            Entry::Player(skill) => {
                let mut slot =
                    super::panels::skill_slot(data.game.get(), skills, *skill, data.start);
                slot.blocked = env.slot_blocked(&slot);
                slot
            }
        })
        .collect();
    if data.mode_actions.slots != slots {
        data.mode_actions.slots = slots;
        data.mode_actions.slots_dirty = true;
    }
}

pub(super) fn push_slots(rt: &UiRuntime, state: &Shared) {
    let changed = std::mem::take(&mut state.borrow_mut().mode_actions.slots_dirty);
    if changed && let Err(error) = rt.refresh_slots() {
        state.borrow_mut().mode_actions.slots_dirty = true;
        bevy::log::warn!("mode action slots: {error}");
    }
}

/// The packet a driven-slave bar slot writes. A gunner's bar shows `mount_attached_skills`
/// (14372 etc.) but casts the linked mount skill (10473 etc.); when that cast skill is
/// `BallisticPos` (target type 11) and the turret system published a sight for the same slave,
/// the target is the aimed ground point. Every other slave skill keeps the unit/no-target cast.
pub(crate) fn slave_cast(
    entry: &SlaveSkill,
    game: Option<&super::data::GameData>,
    sight: Option<crate::ingame::turret::Sight>,
    slave: u32,
    target_obj_id: u32,
) -> aa_net::Command {
    let cast = entry.cast_skill();
    let ballistic = game
        .and_then(|game| game.skills.get(&cast))
        .is_some_and(|info| info.target_type == 11);
    if let Some(sight) = sight.filter(|sight| ballistic && sight.slave == slave) {
        aa_net::Command::UseMateSkillAt {
            skill_id: cast,
            mate_obj_id: slave,
            mount_skill_id: entry.mount_skill,
            pos: sight.pos,
            rot: sight.rot,
        }
    } else {
        aa_net::Command::UseMateSkill {
            skill_id: cast,
            mate_obj_id: slave,
            mount_skill_id: entry.mount_skill,
            target_obj_id,
        }
    }
}

/// Mode slots use the ordinary skill target/validation path, not an item or forced-self cast; while the
/// driven slave's bar is shown, a slot is that slave's skill and goes out as a `Mount` caster
/// (`FUN_39399630` -> `FUN_393989e0`, the same `CSStartSkill` a mate's bar sends).
#[allow(clippy::too_many_arguments)]
pub(super) fn use_slot(
    data: &HudData,
    slot: usize,
    env: &super::usable::Env,
    flight: &mut super::usable::InFlight,
    target: &aa_net::state::Target,
    pressed: &mut bevy::prelude::MessageWriter<crate::ingame::ground_target::SkillPressed>,
    cast: &mut bevy::prelude::MessageWriter<aa_net::NetCommand>,
    failed: &mut bevy::prelude::MessageWriter<super::alerts::UseFailed>,
) {
    let Some(entry) = actions(data).get(slot).copied() else {
        return;
    };
    let Entry::Skill(skill) = entry else {
        // A function slot is run by `run_function` (the callers split it out before building
        // the skill env); a player/buff slot keeps the ordinary path below.
        let Entry::Player(skill) = entry else {
            return;
        };
        if env.held_back(skill, flight) {
            return;
        }
        if let Some(fail) = env.precheck(skill) {
            failed.write(super::alerts::UseFailed {
                skill,
                result: fail.result,
                value: fail.value,
                server: false,
            });
            return;
        }
        pressed.write(crate::ingame::ground_target::SkillPressed {
            skill,
            item: None,
            target_obj_id: target.obj_id.unwrap_or(0),
            gcd: env.uses_gcd(skill),
        });
        return;
    };
    // The bar belongs to the slave seat (driver or passenger): without it (a seat that has just
    // ended) nothing is sent rather than a player-cast skill the server refuses for a slave skill.
    let Some((slave, ..)) = data.mounted.slave_seat() else {
        return;
    };
    // The player-centric range/mana/cast checks do not describe the slave; its recorded own
    // cooldown shows the refusal, while its global cooldown and an unanswered press are
    // dropped silently, like the player's. A gunner's cooldown is on the cast skill, which
    // the server answers (`SCSkillStarted` names 10473, not the 14372 the bar shows).
    let skill_id = skill.skill;
    let cast_skill = skill.cast_skill();
    if let Some(fail) = env.mount_cooldown(slave, cast_skill) {
        failed.write(super::alerts::UseFailed {
            skill: skill_id,
            result: fail.result,
            value: fail.value,
            server: false,
        });
        return;
    }
    if env.mount_held_back(slave, cast_skill, flight) {
        return;
    }
    cast.write(aa_net::NetCommand(slave_cast(
        &skill,
        data.game.get(),
        data.turret,
        slave,
        target.obj_id.unwrap_or(0),
    )));
    if env.uses_gcd(cast_skill) {
        flight.sent_mount(slave, web_time::Instant::now());
    }
}
