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
pub(super) struct SlaveSkill {
    pub skill: u32,
    pub mount_skill: u32,
}

impl SlaveSkill {
    const fn new(skill: u32, mount_skill: u32) -> Self {
        Self { skill, mount_skill }
    }
}

#[derive(Default)]
pub(super) struct Tables {
    skills: HashMap<u32, Vec<u32>>,
    /// `slave_mount_skills` per slave template id, in row order: the driver's mode bar (1.2's
    /// `GetSlaveMountSkillList`; the capture's Farm Wagon order matches the row ids).
    slave_skills: HashMap<u32, Vec<SlaveSkill>>,
}

impl Tables {
    pub(super) fn load(db: &Connection) -> rusqlite::Result<Self> {
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
        Ok(out)
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
    /// The driven slave's bar while the local player sits on its driver seat; it replaces the buff skills
    /// until the player leaves (the capture: 7 slots while driving, none after).
    driver: Option<Vec<SlaveSkill>>,
    actions: Vec<u32>,
    slots: Vec<super::panels::SlotSnap>,
    slots_dirty: bool,
    ui_dirty: bool,
    pending: VecDeque<Pending>,
}

/// Which bar a queued payload is: only buff payloads replace the buff skills; a driver payload is the slave
/// bar and must not be mistaken for buff state (the `lost` check and the restore on leaving read the buffs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    Buffs,
    Driver,
}

struct Pending {
    epoch: u64,
    skills: Option<Vec<u32>>,
    notify: bool,
    source: Source,
}

pub(super) fn actions(data: &HudData) -> &[u32] {
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

/// The slave skill bar of the slave the local player drives, from `slave_mount_skills`; `None` on foot, on a
/// passenger seat, for a mate, or with the catalog not loaded yet.
fn driver_skills(data: &HudData) -> Option<Vec<SlaveSkill>> {
    let template = data.mounted.driver_slave()?.1;
    data.game
        .get()?
        .mode_actions
        .slave_skills
        .get(&template)
        .cloned()
        .filter(|skills| !skills.is_empty())
}

/// The mode bar's skills this frame: the driven slave's in its row order, else the received buff skills.
fn bar_skills(data: &HudData, exclude: u32) -> Option<Vec<u32>> {
    match &data.mode_actions.driver {
        Some(skills) => Some(skills.iter().map(|skill| skill.skill).collect()),
        None => current(data, exclude),
    }
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
    // a vehicle/ship driver's bar replaces the buff skills; a seat change (or leaving) republishes
    let driver = driver_skills(data);
    let driver_changed = data.mode_actions.driver != driver;
    if driver_changed {
        data.mode_actions.driver = driver;
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
    let game = data.game.get().is_some();
    // Buff state changes on its own, whether or not a driver bar is shown; this keeps `buff_skills` current
    // for the restore on leaving and for the `lost` check (which must not repeat once it has cleared).
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
    // The shown bar follows the driver seat.
    if game && driver_changed {
        let (next, source) = if data.mode_actions.driver.is_some() {
            (bar_skills(data, 0), Source::Driver)
        } else {
            (current(data, 0), Source::Buffs)
        };
        queue(data, next, true, source);
    }
    // BuffUpdated changes charges/stacks but native391d5f20 does not refresh this list.
}

fn append(actions: &mut Vec<u32>, skills: &[u32]) {
    for skill in skills.iter().copied().filter(|id| *id != 0) {
        if actions.len() >= 15 {
            break;
        }
        actions.push(skill);
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
            if let Some(skills) = &data.mode_actions.driver {
                let skills: Vec<u32> = skills.iter().map(|skill| skill.skill).collect();
                append(&mut next, &skills);
            } else if let Some(skills) = &data.mode_actions.buff_skills {
                append(&mut next, skills);
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
        let driver = driver_skills(&state.borrow());
        let mut data = state.borrow_mut();
        // the catalog may finish loading while the player is already driving: publish the slave bar then
        if data.mode_actions.driver != driver {
            data.mode_actions.driver = driver;
            let (next, source) = if data.mode_actions.driver.is_some() {
                (bar_skills(&data, 0), Source::Driver)
            } else {
                (current(&data, 0), Source::Buffs)
            };
            queue(&mut data, next, false, source);
        }
        // Seed the buff state once even while a driver bar is shown: the bar is composed from the driver, but
        // `buff_skills` is what the `lost` check and the restore on leaving read, so leaving it None would
        // queue the bar again every frame (`ready` runs from `enchant::track` each frame).
        if data.mode_actions.buff_skills.is_none()
            && data.mode_actions.pending.is_empty()
            && let Some(next) = current(&data, 0)
        {
            queue(&mut data, Some(next), false, Source::Buffs);
        }
    }
    flush(rt, state);
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
            let Some(skill) = index
                .checked_sub(1)
                .and_then(|i| actions(&data).get(i))
                .copied()
            else {
                return Ok(None);
            };
            let mut slot = match data.mounted.driver_slave() {
                Some((slave, _)) if data.mode_actions.driver.is_some() => {
                    super::panels::slave_skill_slot(
                        data.game.get(),
                        Some(&data.cooldowns),
                        slave,
                        skill,
                        data.start,
                    )
                }
                _ => super::panels::skill_slot(
                    data.game.get(),
                    Some(&data.cooldowns),
                    skill,
                    data.start,
                ),
            };
            if let Some(current) = data
                .mode_actions
                .slots
                .get(index - 1)
                .filter(|slot| slot.id == skill)
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
    let driver = data
        .mode_actions
        .driver
        .as_ref()
        .and_then(|_| data.mounted.driver_slave());
    let slots = actions(data)
        .iter()
        .map(|skill| match driver {
            Some((slave, _)) => {
                super::panels::slave_skill_slot(data.game.get(), skills, slave, *skill, data.start)
            }
            None => {
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
    let Some(skill) = actions(data).get(slot).copied() else {
        return;
    };
    if let Some(driver) = &data.mode_actions.driver {
        // The bar belongs to the driven slave: without it (a seat that has just ended) nothing is sent
        // rather than a player-cast skill the server refuses for a slave skill.
        let Some((slave, _)) = data.mounted.driver_slave() else {
            return;
        };
        let Some(entry) = driver.get(slot).filter(|entry| entry.skill == skill) else {
            return;
        };
        // The player-centric range/mana/cast checks do not describe the slave; its recorded own
        // cooldown shows the refusal, while its global cooldown and an unanswered press are
        // dropped silently, like the player's.
        if let Some(fail) = env.mount_cooldown(slave, skill) {
            failed.write(super::alerts::UseFailed {
                skill,
                result: fail.result,
                value: fail.value,
                server: false,
            });
            return;
        }
        if env.mount_held_back(slave, skill, flight) {
            return;
        }
        cast.write(aa_net::NetCommand(aa_net::Command::UseMateSkill {
            skill_id: skill,
            mate_obj_id: slave,
            mount_skill_id: entry.mount_skill,
            target_obj_id: target.obj_id.unwrap_or(0),
        }));
        if env.uses_gcd(skill) {
            flight.sent_mount(slave, web_time::Instant::now());
        }
        return;
    }
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
}
