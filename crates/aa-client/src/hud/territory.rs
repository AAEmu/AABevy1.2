//! Original territory service over received Dominion, faction and world state.
use super::api::{HudBridge, HudData, Shared};
use aa_net::{Event, NetEvent};
use bevy::prelude::*;
use mlua::{Lua, MultiValue, Table, Value};

mod catalog;
#[allow(dead_code)]
pub(super) mod fixture;
mod getters;
mod guard;
#[cfg(test)]
mod tests;
pub(super) use catalog::Tables;

#[derive(Default)]
pub(super) struct View {
    generation: u64,
    context: Option<Context>,
    guard: Option<guard::Inputs>,
    requests: Vec<TaxRequest>,
    hide_pending: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Context {
    npc: (u32, u32),
    zone: u16,
}

#[derive(Clone, Copy)]
struct TaxRequest {
    generation: u64,
    character: u32,
    actor: u32,
    context: Context,
    rate: i32,
}

fn queue(data: &mut HudData, key: Value, rate: Value) {
    let (Some(id), Some(rate)) = (integer(key), integer(rate)) else {
        return;
    };
    let id = id as u16;
    let Some(context) = data.territory.context.filter(|c| c.zone == id) else {
        return;
    };
    let Some((character, actor)) = data.expedition.identity() else {
        return;
    };
    if !(0..=50).contains(&rate)
        || !row(data, Value::Integer(i64::from(id))).is_some_and(|row| owner(data, row))
    {
        return;
    }
    data.territory.requests.push(TaxRequest {
        generation: data.territory.generation,
        character,
        actor,
        context,
        rate,
    });
}

fn flush(data: &mut HudData, player: &aa_net::state::Player) -> Vec<aa_net::Command> {
    let requests = std::mem::take(&mut data.territory.requests);
    requests
        .into_iter()
        .filter_map(|request| {
            let spawn = player.spawn.as_ref()?;
            (request.generation == data.territory.generation
                && data.territory.context == Some(request.context)
                && data.expedition.identity() == Some((request.character, request.actor))
                && spawn.obj_id == request.actor
                && spawn.character().id == request.character
                && row(data, Value::Integer(i64::from(request.context.zone)))
                    .is_some_and(|row| owner(data, row)))
            .then_some(aa_net::Command::Dominion {
                character_id: request.character,
                command: aa_net::DominionCommand::ChangeTaxRate {
                    zone_group_id: request.context.zone,
                    tax_rate: request.rate,
                },
            })
        })
        .collect()
}

fn reset(data: &mut HudData) {
    data.territory.generation = data.territory.generation.wrapping_add(1);
    data.territory.context = None;
    data.territory.guard = None;
    data.territory.requests.clear();
    data.territory.hide_pending = true;
}

fn invalidate(data: &mut HudData, event: &Event) {
    // Every load retires dependent interaction work. This does not assert that
    // an unknown destination accepted a fresh population baseline.
    let removed = match event {
        Event::UnitsRemoved(ids) => {
            data.territory
                .context
                .is_some_and(|c| ids.contains(&c.npc.0))
                || data
                    .territory
                    .guard
                    .as_ref()
                    .is_some_and(|guard| ids.contains(&guard.tower))
                || data
                    .expedition
                    .identity()
                    .is_some_and(|(_, actor)| ids.contains(&actor))
        }
        _ => false,
    };
    if removed
        || matches!(
            event,
            Event::CharacterSelected(_)
                | Event::PlayerSpawn(_)
                | Event::LoadInstance(_)
                | Event::LeftWorld { .. }
                | Event::Kicked { .. }
                | Event::Disconnected { .. }
                | Event::LoginFailed { .. }
                | Event::GameDecodeFailed { .. }
        )
        || matches!(event,Event::UnhandledGamePacket{level,len,..} if *level==aa_net::game::LEVEL_COMPRESSED && *len>0)
    {
        reset(data);
    }
}

fn track(mut events: MessageReader<NetEvent>, bridge: NonSend<HudBridge>) {
    let mut data = bridge.0.borrow_mut();
    for NetEvent(event) in events.read() {
        invalidate(&mut data, event);
    }
}

fn hide_retired(rt: NonSend<aa_ui::UiRuntime>, bridge: NonSend<HudBridge>) {
    let pending = std::mem::take(&mut bridge.0.borrow_mut().territory.hide_pending);
    if pending {
        let _ = rt.exec(
            "@territory_retired",
            "if territoryFrame then territoryFrame:Show(false) end",
        );
    }
}

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        track
            .after(super::expedition::track)
            .before(super::sync_hud)
            .before(super::prebuild_hud),
    );
    app.add_systems(
        Update,
        hide_retired
            .after(super::sync_hud)
            .before(aa_ui::UiFrame)
            .run_if(super::hud_up),
    );
}

/// Original numeric thunk: f32 -> CVTTSS2SI, followed by a u16 lookup key.
fn integer(value: Value) -> Option<i32> {
    let n = match value {
        Value::Integer(n) => n as f32,
        Value::Number(n) => n as f32,
        _ => return None,
    };
    Some(
        if n.is_finite() && (-2147483648.0..2147483648.0).contains(&n) {
            n as i32
        } else {
            i32::MIN
        },
    )
}

fn row(data: &HudData, key: Value) -> Option<&aa_net::DominionData> {
    let id = integer(key)? as u16;
    let received = data.expedition.dominion();
    (!received.coverage_gap)
        .then(|| received.rows.get(&id))
        .flatten()
}

fn owner(data: &HudData, row: &aa_net::DominionData) -> bool {
    let own = data.expedition.own_id();
    !data.disconnected
        && own != 0
        && row.expedition_id == own
        && data
            .expedition
            .identity()
            .is_some_and(|(character, _)| character != 0 && character == data.character_id)
        && data
            .expedition
            .faction(own)
            .is_some_and(|faction| faction.owner_id == data.character_id)
}

fn text(lua: &Lua, bytes: &[u8]) -> mlua::Result<mlua::LuaString> {
    lua.create_string(bytes.split(|b| *b == 0).next().unwrap_or_default())
}

/// Correct the original visible-house-event closure's shadowed nil upvalue.
fn repair_window_events(rt: &aa_ui::UiRuntime) -> mlua::Result<()> {
    rt.lua
        .load(include_str!("territory/window.lua"))
        .set_name("@territory/window.lua")
        .exec()
}

pub(super) fn install(lua: &Lua, state: &Shared) -> mlua::Result<()> {
    getters::install(lua, state)?;
    guard::install(lua, state)?;
    let api: Table = lua.globals().get("X2Dominion")?;
    let state = state.clone();
    api.set(
        "ChangeDominionTaxRate",
        lua.create_function(move |_, (_, key, rate): (Value, Value, Value)| {
            queue(&mut state.borrow_mut(), key, rate);
            Ok(())
        })?,
    )
}
