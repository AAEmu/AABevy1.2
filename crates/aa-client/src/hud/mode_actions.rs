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

#[derive(Default)]
pub(super) struct Tables {
    skills: HashMap<u32, Vec<u32>>,
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
    actions: Vec<u32>,
    slots: Vec<super::panels::SlotSnap>,
    slots_dirty: bool,
    ui_dirty: bool,
    pending: VecDeque<(u64, Option<Vec<u32>>, bool)>,
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

fn queue(data: &mut HudData, next: Option<Vec<u32>>, notify: bool) {
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
    view.pending.push_back((view.epoch, next, notify));
}

/// Called after folding each actual receipt, including ordered catalog-startup replay.
pub(super) fn reduce(data: &mut HudData, event: &Event, removed: Option<u32>) {
    let own = super::user_music::own_identity(data).map(|(_, actor)| actor);
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
    if data.game.get().is_some()
        && (snapshot || created || removed.is_some() || gap || retired || lost)
    {
        let next = current(data, removed.unwrap_or(0));
        queue(data, next, snapshot || created || removed.is_some());
    } else if matches!(event, Event::OwnUnitSnapshot(u) if Some(u.obj_id) == own)
        && data.mode_actions.buff_skills.is_none()
        && data.game.get().is_some()
    {
        // Existing native objects ignore repeated SCUnitState. The remake uses
        // a matched snapshot only to recover unknown history, without 539/95.
        let next = current(data, 0);
        queue(data, next, false);
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
        let Some((epoch, skills, notify)) = pending else {
            break;
        };
        {
            let mut data = state.borrow_mut();
            if data.mode_actions.epoch != epoch {
                continue;
            }
            data.mode_actions.buff_skills = skills;
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
            if let Some(skills) = &data.mode_actions.buff_skills {
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
        let mut data = state.borrow_mut();
        if data.mode_actions.buff_skills.is_none()
            && data.mode_actions.pending.is_empty()
            && let Some(next) = current(&data, 0)
        {
            queue(&mut data, Some(next), false);
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
            let mut slot = super::panels::skill_slot(
                data.game.get(),
                Some(&data.cooldowns),
                skill,
                data.start,
            );
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
    let slots = actions(data)
        .iter()
        .map(|skill| {
            let mut slot = super::panels::skill_slot(data.game.get(), skills, *skill, data.start);
            slot.blocked = env.slot_blocked(&slot);
            slot
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

/// Mode slots use the ordinary skill target/validation path, not an item or forced-self cast.
pub(super) fn use_slot(
    data: &HudData,
    slot: usize,
    env: &super::usable::Env,
    flight: &super::usable::InFlight,
    target: &aa_net::state::Target,
    pressed: &mut bevy::prelude::MessageWriter<crate::ingame::ground_target::SkillPressed>,
    failed: &mut bevy::prelude::MessageWriter<super::alerts::UseFailed>,
) {
    let Some(skill) = actions(data).get(slot).copied() else {
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
}
