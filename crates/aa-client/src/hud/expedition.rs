//! Original expedition screens over received native membership and policy caches.
use super::api::{HudBridge, Shared};
use aa_net::{Event, ExpeditionEvent, ExpeditionRolePolicy, Expeditions, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, MultiValue, Table, Value};
mod actions;
mod creation;
mod tasks;

#[allow(dead_code)]
pub(super) mod fixture;
mod members;
pub(super) use members::Tables;
#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct View {
    cache: Expeditions,
    requests: Vec<(u32, u32, aa_net::ExpeditionCommand)>,
    tasks: tasks::Tasks,
    dominion: aa_net::Dominion,
    creation_npc: Option<(u32, u32)>,
}

impl View {
    pub(super) fn faction_name(&self, id: u32) -> Option<&[u8]> {
        self.cache.factions.get(&id).map(|faction| faction.name.as_slice())
    }
}

fn text(lua: &Lua, bytes: &[u8]) -> mlua::Result<mlua::LuaString> {
    lua.create_string(bytes.split(|byte| *byte == 0).next().unwrap_or_default())
}

/// Native392faa00 omits the wire dismiss flag and reorders the remaining permissions.
fn policy(lua: &Lua, received: &ExpeditionRolePolicy) -> mlua::Result<Table> {
    let out = lua.create_table()?;
    out.set(1, f64::from(received.role))?;
    out.set(2, text(lua, &received.name)?)?;
    for (column, index) in [1, 2, 3, 8, 7, 0, 5, 6].into_iter().enumerate() {
        out.set(column + 3, received.permissions[index] != 0)?;
    }
    Ok(out)
}

fn optional_policy(lua: &Lua, received: Option<&ExpeditionRolePolicy>) -> mlua::Result<MultiValue> {
    Ok(match received {
        Some(received) => MultiValue::from_iter([Value::Table(policy(lua, received)?)]),
        None => MultiValue::new(),
    })
}

/// The native ScriptAnyValue adapter accepts a number, converts through f32 and CVTTSS2SI,
/// then looks up a signed key. Out-of-byte roles cannot match received policy roles.
fn role(value: Value) -> Option<u8> {
    let number = match value {
        Value::Number(number) => number as f32,
        Value::Integer(number) => number as f32,
        _ => return None,
    };
    let role = number.trunc();
    (role.is_finite() && (0.0..=255.0).contains(&role)).then_some(role as u8)
}

pub(super) fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    let faction: Table = lua.globals().get("X2Faction")?;
    let shared = bridge.clone();
    faction.set(
        "GetMyExpeditionId",
        lua.create_function(move |_, _: Value| Ok(f64::from(shared.borrow().expedition.cache.own_id as f32)))?,
    )?;
    let shared = bridge.clone();
    faction.set(
        "IsExpedInfoLoaded",
        lua.create_function(move |_, _: Value| Ok(shared.borrow().expedition.cache.info_loaded()))?,
    )?;
    let shared = bridge.clone();
    faction.set(
        "GetMyExpeditionName",
        lua.create_function(move |lua, _: Value| {
            let data = shared.borrow();
            let cache = &data.expedition.cache;
            let bytes = if cache.own_id == 0 {
                None
            } else {
                cache.factions.get(&cache.own_id).map(|row| row.name.as_slice())
            };
            text(lua, bytes.unwrap_or_default())
        })?,
    )?;
    let shared = bridge.clone();
    faction.set(
        "GetExpeditionMyRolePolicy",
        lua.create_function(move |lua, _: Value| optional_policy(lua, shared.borrow().expedition.cache.my_policy()))?,
    )?;
    let shared = bridge.clone();
    faction.set(
        "GetAllExpeditionRolePolicies",
        lua.create_function(move |lua, _: Value| {
            let data = shared.borrow();
            let out = lua.create_table()?;
            for received in data.expedition.cache.own_policies() {
                out.raw_push(policy(lua, received)?)?;
            }
            Ok(out)
        })?,
    )?;
    let shared = bridge.clone();
    faction.set(
        "GetExpeditionRolePolicy",
        lua.create_function(move |lua, (_, value): (Value, Value)| {
            let data = shared.borrow();
            let cache = &data.expedition.cache;
            optional_policy(lua, role(value).and_then(|role| cache.policies.get(&(cache.own_id, role))))
        })?,
    )?;
    let shared = bridge.clone();
    faction.set(
        "GetExpeditionMemberCount",
        lua.create_function(move |_, _: Value| {
            let data = shared.borrow();
            let members = &data.expedition.cache.members;
            Ok((
                f64::from(members.values().filter(|row| row.online != 0).count() as f32),
                f64::from(members.len() as f32),
            ))
        })?,
    )?;
    // Verified original runtime39ac729c; independent of the ten visible Lua rows.
    faction.set("GetExpeditionMembersPerPage", lua.create_function(|_, _: Value| Ok(50.0))?)?;
    members::install(lua, &faction, bridge)?;
    actions::install(lua, &faction, bridge)?;
    creation::install(lua, &faction, bridge)?;
    tasks::install(lua, bridge)
}

fn dispatch(rt: Option<&UiRuntime>, bridge: &Shared, event: &Event) {
    let mut data = bridge.borrow_mut();
    data.expedition.dominion.apply(event);
    let cache = &mut data.expedition.cache;
    let previous = cache.own_id;
    let (_, notices) = cache.apply_with_notices(event);
    let reset = previous != cache.own_id
        || matches!(
            event,
            Event::CharacterSelected(_)
                | Event::LeftWorld { .. }
                | Event::Kicked { .. }
                | Event::Disconnected { .. }
                | Event::LoginFailed { .. }
        );
    if reset {
        data.expedition.requests.clear();
        data.expedition.creation_npc = None;
        tasks::clear(&mut data.expedition);
    }
    drop(data);
    if let Event::Expedition(receipt) = event {
        tasks::received(rt.map(|rt| &rt.lua), bridge, receipt);
    }
    if let Some(rt) = rt {
        if reset {
            let _ = rt.exec("@expedition_reset", "HUD.expedition_close_tasks()");
        }
        for notice in notices {
            aa_ui::lua::fire_ui_event(&rt.lua, notice.event_name(), vec![]);
        }
        if let Event::ExpeditionRequestRejected { character_id, message, .. } = event {
            if bridge.borrow().expedition.cache.local_character_id == Some(*character_id) {
                rt.fire_event("SYSMSG", message.clone());
            }
        }
    }
}

fn track(mut events: MessageReader<NetEvent>, rt: Option<NonSend<UiRuntime>>, built: Res<super::HudBuilt>, bridge: NonSend<HudBridge>) {
    let rt = if built.up { rt.as_deref() } else { None };
    for NetEvent(event) in events.read() {
        dispatch(rt, &bridge.0, event);
    }
}

pub(super) fn plugin(app: &mut App) {
    app.add_systems(Update, track.before(super::sync_hud).before(super::prebuild_hud))
        .add_systems(
            Update,
            update
                .after(super::sync_hud)
                .after(super::dialog::handle_dialog)
                .before(aa_ui::UiFrame)
                .run_if(super::hud_up),
        );
}

fn update(
    rt: NonSend<UiRuntime>,
    bridge: NonSend<HudBridge>,
    player: Res<aa_net::state::Player>,
    units: Res<aa_net::state::Units>,
    mut events: MessageReader<NetEvent>,
    mut out: MessageWriter<aa_net::NetCommand>,
) {
    for NetEvent(event) in events.read() {
        creation::service(&rt, &bridge.0, &player, &units, event);
    }
    creation::check_service(&rt, &bridge.0, &units);
    tasks::show(&rt.lua, &bridge.0);
    for command in actions::flush(&mut bridge.0.borrow_mut(), &player) {
        out.write(aa_net::NetCommand(command));
    }
}
