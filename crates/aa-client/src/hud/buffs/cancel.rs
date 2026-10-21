//! Original self-buff ordinal cancellation over the known native Good list.
use super::super::api::{HudBridge, HudData, Shared};
use aa_net::{Command, Event, NetCommand, NetEvent, PlayerSpawn, buff::BuffKind, state::Buffs};
use bevy::prelude::*;
use mlua::{Lua, MultiValue, ObjectLike, Table, Value};
use web_time::Instant;
#[allow(dead_code)]
pub(in crate::hud) mod fixture;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Row {
    index: u32,
    template: u32,
    received: Instant,
}
#[derive(Clone, Copy)]
struct Captured {
    actor: (u32, u32),
    row: Row,
}
#[derive(Default)]
pub(in crate::hud) struct View {
    waiting_spawn: bool,
    actor: Option<(u32, u32)>,
    /// Unknown history differs from a known empty Good list.
    good: Option<Vec<Row>>,
    requests: Vec<Captured>,
}

fn refresh(data: &mut HudData, spawn: Option<&PlayerSpawn>, buffs: Option<&Buffs>) {
    let actor = spawn
        .filter(|spawn| {
            !data.buff_cancel.waiting_spawn
                && !data.disconnected
                && data.character_id != 0
                && spawn.character().id == data.character_id
                && spawn.obj_id != 0
        })
        .map(|spawn| (spawn.character().id, spawn.obj_id));
    if data.buff_cancel.actor != actor {
        data.buff_cancel.requests.clear();
    }
    data.buff_cancel.actor = actor;
    data.buff_cancel.good = actor.and_then(|(_, obj_id)| buffs?.native.of(obj_id)).map(|entries| {
        entries
            .iter()
            .filter(|entry| entry.buff.kind == BuffKind::Good)
            .map(|entry| Row {
                index: entry.buff.index,
                template: entry.buff.buff_id,
                received: entry.received,
            })
            .collect()
    });
}

// ScriptAnyValue type4 -> f32 CVTTSS2SI, followed by native wrapping subtraction.
fn ordinal(value: Option<&Value>) -> Option<usize> {
    let value = match value? {
        Value::Integer(value) => *value as f32,
        Value::Number(value) => *value as f32,
        _ => return None,
    };
    let index = if value.is_finite() && (-2147483648.0..2147483648.0).contains(&value) {
        value as i32
    } else {
        i32::MIN
    };
    usize::try_from(index.wrapping_sub(1)).ok()
}

/// Return true only for the original system-buff error; all other refusals are silent.
fn request(data: &mut HudData, index: usize) -> bool {
    let Some(actor) = data
        .buff_cancel
        .actor
        .filter(|(id, _)| !data.disconnected && *id == data.character_id)
    else {
        return false;
    };
    let Some(row) = data.buff_cancel.good.as_ref().and_then(|rows| rows.get(index)).copied() else {
        return false;
    };
    // Refuse a click on stale displayed geometry instead of applying its ordinal to a different row.
    let shown = data.panels_pushed.as_ref().and_then(|panels| panels.buffs.good.get(index));
    if !shown.is_some_and(|shown| shown.id == row.template && shown.instance_index == row.index) {
        return false;
    }
    let Some(info) = data.game.get().and_then(|game| game.buffs.get(&row.template)) else {
        return false;
    };
    if info.system {
        return true;
    }
    data.buff_cancel.requests.push(Captured { actor, row });
    false
}

pub(in crate::hud) fn install(lua: &Lua, state: &Shared) -> mlua::Result<()> {
    let api: Table = lua.globals().get("X2Ability")?;
    let state = state.clone();
    api.raw_set(
        "CancelPlayerBuff",
        lua.create_function(move |lua, args: MultiValue| {
            let error = ordinal(args.get(1)).is_some_and(|index| request(&mut state.borrow_mut(), index));
            if error {
                let locale: Table = lua.globals().get("X2Locale")?;
                let group: Value = lua.globals().get("ERROR_MSG")?;
                let text: Value = locale.call_method("LocalizeUiText", (group, "CANNOT_REMOVE_SYSTEM_BUFF"))?;
                aa_ui::lua::fire_ui_event(lua, "SYSMSG", vec![text]);
            }
            Ok(())
        })?,
    )
}

fn flush(data: &mut HudData) -> Vec<Command> {
    std::mem::take(&mut data.buff_cancel.requests)
        .into_iter()
        .filter(|capture| {
            !data.disconnected
                && data.character_id == capture.actor.0
                && data.buff_cancel.actor == Some(capture.actor)
                && data.buff_cancel.good.as_ref().is_some_and(|rows| rows.contains(&capture.row))
                && data
                    .game
                    .get()
                    .and_then(|game| game.buffs.get(&capture.row.template))
                    .is_some_and(|info| !info.system)
        })
        .map(|capture| Command::RemoveBuff {
            character_id: capture.actor.0,
            obj_id: capture.actor.1,
            buff_index: capture.row.index,
        })
        .collect()
}

fn prepare(
    mut events: MessageReader<NetEvent>,
    state: NonSend<HudBridge>,
    player: Option<Res<aa_net::state::Player>>,
    buffs: Option<Res<Buffs>>,
) {
    let mut data = state.0.borrow_mut();
    for NetEvent(event) in events.read() {
        if matches!(
            event,
            Event::CharacterSelected(_)
                | Event::ReadyToConnect { .. }
                | Event::LoginSucceeded(_)
                | Event::LeftWorld { .. }
                | Event::Disconnected { .. }
                | Event::Kicked { .. }
                | Event::PhaseChanged(aa_net::Phase::Authenticating | aa_net::Phase::Connecting)
        ) {
            data.buff_cancel = View {
                waiting_spawn: true,
                ..default()
            };
        } else if matches!(event, Event::PlayerSpawn(_)) {
            data.buff_cancel.waiting_spawn = false;
            data.buff_cancel.requests.clear();
        }
    }
    refresh(
        &mut data,
        player.as_deref().and_then(|player| player.spawn.as_ref()),
        buffs.as_deref(),
    );
}
fn write(state: NonSend<HudBridge>, mut commands: MessageWriter<NetCommand>) {
    commands.write_batch(flush(&mut state.0.borrow_mut()).into_iter().map(NetCommand));
}
pub(in crate::hud) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        prepare
            .before(aa_ui::UiInput)
            .before(super::super::sync_hud)
            .before(super::super::prebuild_hud)
            .before(super::super::build_hud),
    )
    .add_systems(Update, write.after(aa_ui::UiFrame).after(super::super::sync_hud));
}

#[cfg(test)]
mod tests;
