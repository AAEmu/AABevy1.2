//! Original entrance getters and local queue controls over separately received battlefield state.
use super::api::{HudBridge, HudData, Shared};
use aa_net::{Battlefield, BattlefieldCommand, BattlefieldEvent, Event, NetCommand, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, MultiValue, Table, Value};

mod catalog;
#[allow(dead_code)]
pub(super) mod fixture;
mod invitation;
mod notifications;
mod progress;
mod slots;
pub(super) use catalog::Tables;
pub(super) use slots::refresh as refresh_slots;

#[derive(Default)]
pub(super) struct View {
    active: bool,
    owner: u32,
    level: Option<i32>,
    buffs: Option<Vec<u32>>,
    received: Battlefield,
    clock: aa_net::ServerClock,
    interacted_npc: u32,
    // Native local request state precedes the ACK. It never creates queue counts or admission.
    local_request: Option<(u32, u8)>,
    cancelled: bool,
    requests: Vec<(u32, u32, BattlefieldCommand)>,
    invitation: invitation::View,
    ordinary_slots: std::collections::HashMap<u32, super::panels::SlotSnap>,
    slots_dirty: bool,
}

impl View {
    fn joined(&self) -> (f64, f64, f64) {
        if self.cancelled {
            return (0., 0., -1.);
        }
        let (count, size, corps) = self.received.joined_infos();
        let corps = self.local_request.map_or(corps, |(_, corps)| i32::from(corps));
        (f64::from(count as f32), f64::from(size as f32), f64::from(corps as f32))
    }

    fn busy(&self) -> bool {
        self.local_request.is_some() || self.received.requesting.is_some() || self.received.applied.is_some() || self.received.cancel_pending
    }
}

pub(super) fn refresh(world: &World, data: &mut HudData, spawn: &aa_net::PlayerSpawn) {
    let owner = spawn.character().id;
    let view = &mut data.battlefield;
    if view.owner != owner {
        *view = View::default();
        view.owner = owner;
    }
    view.level = Some(i32::from(
        world
            .get_resource::<aa_net::state::Units>()
            .and_then(|units| units.get(spawn.obj_id))
            .map_or(spawn.unit.level, |unit| unit.level),
    ));
    view.buffs = world
        .get_resource::<aa_net::state::Buffs>()
        .and_then(|buffs| buffs.native.of(spawn.obj_id))
        .map(|buffs| buffs.iter().map(|entry| entry.buff.buff_id).collect());
    if !view.active {
        view.received = world.get_resource::<Battlefield>().cloned().unwrap_or_default();
        view.clock = world.get_resource::<aa_net::ServerClock>().cloned().unwrap_or_default();
    }
}

fn field(data: &HudData, npc: u32) -> Option<&catalog::Field> {
    data.battlefield.level?;
    data.game.get()?.battlefield.field(npc)
}

fn permitted(data: &HudData) -> Option<bool> {
    let blocked = &data.game.get()?.battlefield.blocked_buffs;
    Some(!data.battlefield.buffs.as_ref()?.iter().any(|buff| blocked.contains(buff)))
}

fn cancel(lua: &Lua, bridge: &Shared) {
    {
        let mut data = bridge.borrow_mut();
        let view = &mut data.battlefield;
        if view.owner == 0 || view.cancelled || view.received.cancel_pending || view.received.in_instant_game() {
            return;
        }
        view.local_request = None;
        view.interacted_npc = 0;
        view.cancelled = true;
        view.requests.push((view.owner, 0, BattlefieldCommand::Cancel));
    }
    // Native reset and callback are synchronous, before its empty e2 write.
    aa_ui::lua::fire_ui_event(lua, "INSTANT_GAME_JOIN_CANCEL", vec![]);
    if let Err(error) = lua.load("HUD.battlefield_close_invitation()").exec() {
        warn!("hud: cancel battlefield invitation: {error}");
    }
}

fn number(value: i32) -> Value {
    Value::Number(f64::from(value as f32))
}

pub(super) fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    let api: Table = lua.globals().get("X2BattleField")?;
    let state = bridge.clone();
    api.set(
        "GetJoinedInfos",
        lua.create_function(move |_, _: Value| Ok(state.borrow().battlefield.joined()))?,
    )?;
    let state = bridge.clone();
    api.set(
        "GetInteractedNpcType",
        lua.create_function(move |_, _: Value| Ok(f64::from(state.borrow().battlefield.interacted_npc as f32)))?,
    )?;
    let state = bridge.clone();
    // Genuine matching Joined is state5; later match phases are owned by the received net fold.
    api.set(
        "IsInInstantGame",
        lua.create_function(move |_, _: Value| Ok(state.borrow().battlefield.received.in_instant_game()))?,
    )?;
    let state = bridge.clone();
    api.set(
        "GetEntranceBasicInfo",
        lua.create_function(move |lua, (_, npc): (Value, u32)| {
            let data = state.borrow();
            let Some(field) = field(&data, npc) else { return Ok(MultiValue::new()) };
            let out = lua.create_table()?;
            out.set("type", number(field.id as i32))?;
            out.set("name", field.name.as_str())?;
            out.set("desc", field.desc.as_str())?;
            Ok(MultiValue::from_iter([Value::Table(out)]))
        })?,
    )?;
    let state = bridge.clone();
    api.set(
        "GetEntranceMaximumPlayers",
        lua.create_function(move |_, (_, npc): (Value, u32)| {
            let data = state.borrow();
            let result = field(&data, npc).and_then(|field| field.rule(data.battlefield.level?));
            Ok(result.map_or_else(MultiValue::new, |rule| MultiValue::from_iter([number(rule.size)])))
        })?,
    )?;
    let state = bridge.clone();
    api.set(
        "GetEntranceLevelSection",
        lua.create_function(move |_, (_, npc): (Value, u32)| {
            let data = state.borrow();
            let Some(field) = field(&data, npc) else { return Ok(MultiValue::new()) };
            let (minimum, maximum) = field.band(data.battlefield.level.unwrap());
            Ok(MultiValue::from_iter([number(minimum), number(maximum)]))
        })?,
    )?;
    let state = bridge.clone();
    api.set(
        "GetEntranceVictoryConditionInfo",
        lua.create_function(move |lua, (_, npc): (Value, u32)| {
            let data = state.borrow();
            let Some(rule) = field(&data, npc).and_then(|field| field.rule(data.battlefield.level?)) else {
                return Ok(MultiValue::new());
            };
            let out = lua.create_table()?;
            out.set("victoryScore", number(rule.score))?;
            out.set("victoryKillCount", number(rule.kills))?;
            out.set("victoryTarget", number(rule.target))?;
            Ok(MultiValue::from_iter([Value::Table(out)]))
        })?,
    )?;
    let state = bridge.clone();
    api.set(
        "GetEntranceFactionInfo",
        lua.create_function(move |lua, (_, npc, corps): (Value, u32, i32)| {
            let data = state.borrow();
            let Some(rule) = field(&data, npc).and_then(|field| field.rule(data.battlefield.level?)) else {
                return Ok(MultiValue::new());
            };
            let text = match corps {
                0 | 1 => data.game.get().unwrap().factions.get(&rule.factions[corps as usize]).map_or("", String::as_str),
                2 => "ANY_CORPS",
                _ => "",
            };
            Ok(MultiValue::from_iter([Value::String(lua.create_string(text)?)]))
        })?,
    )?;
    let state = bridge.clone();
    api.set(
        "RequestInstanceGame",
        lua.create_function(move |lua, (_, npc, corps): (Value, u32, i32)| {
            let mut data = state.borrow_mut();
            let Some(field_id) = field(&data, npc).map(|field| field.id) else {
                return Ok(());
            };
            if data.battlefield.owner == 0 || corps < 0 {
                return Ok(());
            }
            match permitted(&data) {
                Some(false) => {
                    drop(data);
                    cancel(lua, &state);
                    return Ok(());
                }
                None => return Ok(()),
                Some(true) => {}
            }
            let view = &mut data.battlefield;
            if view.busy() || view.cancelled {
                return Ok(());
            }
            let corps = corps.min(2) as u8;
            view.local_request = Some((field_id, corps));
            view.interacted_npc = npc;
            view.requests.push((
                view.owner,
                npc,
                BattlefieldCommand::Apply {
                    battlefield_id: field_id,
                    corps,
                },
            ));
            Ok(())
        })?,
    )?;
    let state = bridge.clone();
    api.set(
        "CancelInstanceGame",
        lua.create_function(move |lua, _: Value| {
            cancel(lua, &state);
            Ok(())
        })?,
    )?;
    invitation::install(lua, bridge)?;
    slots::install(lua, bridge)?;
    progress::install(lua, bridge)
}

fn dispatch(rt: &UiRuntime, bridge: &Shared, event: &Event) {
    notifications::advance(rt, bridge);
    let progress_before = notifications::before_receive(rt, bridge, event);
    let (before, was_joined) = {
        let data = bridge.borrow();
        (data.battlefield.received.invitation, data.battlefield.received.joined.is_some())
    };
    let notify = {
        let mut data = bridge.borrow_mut();
        let valid_field = match event {
            Event::Battlefield(BattlefieldEvent::QueueState(queue)) => {
                data.game.get().is_some_and(|g| g.battlefield.fields.contains_key(&queue.battlefield_id))
            }
            _ => false,
        };
        let view = &mut data.battlefield;
        let accepted_queue = matches!(event, Event::Battlefield(BattlefieldEvent::QueueState(queue))
            if valid_field && !view.cancelled && !view.received.cancel_pending && view.received.invitation.is_none() && view.received.joined.is_none()
                && view.received.applied.is_some_and(|(id,_)| id==queue.battlefield_id));
        view.clock.apply(event);
        let now = view.clock.mapped_now();
        if matches!(event, Event::Battlefield(BattlefieldEvent::Started(_))) {
            // Preserve state6 during its synchronous callback, including SC191's immediate branch.
            view.received.apply(event);
            if progress_before.phase == Some(aa_net::battlefield::BattlefieldPhase::Joined)
                && view.received.phase == Some(aa_net::battlefield::BattlefieldPhase::Opening)
            {
                view.received.opening_at_ms = now.map(|now| now.wrapping_add(6000));
            }
        } else {
            view.received.apply_with_time(event, now);
        }
        match event {
            Event::Battlefield(BattlefieldEvent::Applied { battlefield_id, corps, error })
                if view.local_request == Some((*battlefield_id, *corps)) && !view.cancelled =>
            {
                view.local_request = None;
                if *error != 0 {
                    view.interacted_npc = 0;
                }
            }
            Event::Battlefield(BattlefieldEvent::Cancelled { .. }) => {
                view.cancelled = false;
                view.local_request = None;
                view.interacted_npc = 0;
            }
            Event::Battlefield(BattlefieldEvent::RequestRejected { .. }) => {
                // Transport failure is not a server reset. Preserve the native local busy state.
            }
            Event::CharacterSelected(_) | Event::LeftWorld { .. } | Event::Disconnected { .. } | Event::Kicked { .. } => *view = View::default(),
            _ => {}
        }
        accepted_queue
    };
    if notify {
        rt.fire_event("UPDATE_INSTANT_GAME_STATE", ());
    }
    slots::push(
        rt,
        bridge,
        matches!(
            event,
            Event::Battlefield(BattlefieldEvent::Joined(_) | BattlefieldEvent::Started(_) | BattlefieldEvent::Ended(_) | BattlefieldEvent::SkillEnable(_))
                | Event::LoadInstance(_)
        ),
    );
    notifications::after_receive(rt, bridge, event, progress_before);
    invitation::received(rt, bridge, before, was_joined);
}

fn flush(data: &mut HudData) -> Vec<BattlefieldCommand> {
    let mut out = Vec::new();
    for (owner, npc, command) in std::mem::take(&mut data.battlefield.requests) {
        if owner == 0
            || owner != data.character_id
            || owner != data.battlefield.owner
            || out.contains(&command)
            || data.battlefield.received.validate_command(command).is_err()
        {
            continue;
        }
        if let BattlefieldCommand::Apply { battlefield_id, .. } = command {
            if permitted(data) != Some(true)
                || field(data, npc).is_none_or(|field| field.id != battlefield_id)
                || data.battlefield.received.requesting.is_some()
                || data.battlefield.received.applied.is_some()
                || data.battlefield.received.cancel_pending
            {
                continue;
            }
        }
        if matches!(
            command,
            BattlefieldCommand::RespondToInvitation { .. } | BattlefieldCommand::EnteredWorld { .. }
        ) && (data.battlefield.cancelled || (matches!(command, BattlefieldCommand::RespondToInvitation { .. }) && permitted(data) != Some(true)))
        {
            continue;
        }
        out.push(command);
    }
    out
}

fn recruiter(rt: &UiRuntime, bridge: &Shared, event: &Event, units: Option<&aa_net::state::Units>) {
    let Event::Interaction(aa_net::interaction::InteractEvent::SkillList {
        npc_obj_id,
        obj_id,
        skills,
        interactable: true,
        ..
    }) = event
    else {
        return;
    };
    if !skills.contains(&12660) {
        return;
    }
    let valid = (|| {
        let data = bridge.borrow();
        if data.snapshot.unit_id.parse::<u32>().ok() != Some(*obj_id) {
            return None;
        }
        let (_, template) = data.dialog.npc.filter(|(id, _)| id == npc_obj_id)?;
        if !units?
            .get(*npc_obj_id)
            .is_some_and(|unit| matches!(unit.kind, aa_net::model::UnitKind::Npc { template_id, .. } if template_id==template))
        {
            return None;
        }
        let field = data.game.get()?.battlefield.field(template)?;
        if data.battlefield.level? < field.minimum() {
            return None;
        }
        let near_target = data
            .panels
            .target
            .as_ref()
            .is_some_and(|target| target.obj_id == *npc_obj_id && target.npc_type == Some(template))
            && data.target_pos.is_some_and(|pos| pos.distance(data.pos) < 2.5);
        Some(near_target || data.battlefield.interacted_npc == template)
    })()
    .unwrap_or(false);
    if valid {
        rt.fire_event("NPC_INTERACTION_START", ("recruiter", "", npc_obj_id.to_string()));
    }
}

fn update(
    mut events: MessageReader<NetEvent>,
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<HudBridge>,
    units: Option<Res<aa_net::state::Units>>,
    mut out: MessageWriter<NetCommand>,
) {
    let Some(rt) = rt else {
        events.clear();
        return;
    };
    let first = {
        let mut data = bridge.0.borrow_mut();
        !std::mem::replace(&mut data.battlefield.active, true)
    };
    notifications::advance(&rt, &bridge.0);
    if first {
        events.clear();
        invitation::show(&rt, &bridge.0);
    } else {
        for NetEvent(event) in events.read() {
            recruiter(&rt, &bridge.0, event, units.as_deref());
            dispatch(&rt, &bridge.0, event);
        }
    }
    slots::push(&rt, &bridge.0, false);
    for command in flush(&mut bridge.0.borrow_mut()) {
        out.write(NetCommand(aa_net::Command::Battlefield(command)));
    }
}

pub(super) fn entered_world(rt: &UiRuntime, bridge: &Shared) {
    invitation::world_ready(rt, bridge);
}

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        update
            .after(super::sync_hud)
            .after(super::dialog::handle_dialog)
            .before(aa_ui::UiFrame)
            .run_if(super::hud_up),
    );
    app.add_systems(OnExit(crate::Screen::InGame), |bridge: NonSend<HudBridge>| {
        // The native manager outlives HUD Lua states and preserves pre-join world transfers.
        let mut data = bridge.0.borrow_mut();
        data.battlefield.active = false;
        invitation::reset_ui(&mut data.battlefield.invitation);
    });
}

#[cfg(test)]
mod invitation_tests;
#[cfg(test)]
mod progress_tests;
#[cfg(test)]
mod tests;
