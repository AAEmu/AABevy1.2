//! The original public-farm windows (`x2ui/commonfarm`: `info.alb`, `my_farm_info.alb`,
//! unchanged Lua) over aa-net's farm protocol and the static `farm_groups` / `farm_group_doodads` /
//! `common_farms` tables.
//!
//! `X2` getters (`FUN_394445B0` .. `FUN_394447C0` in `x2game.dll.c`): `GetFarmGroups` answers
//! `{ type, name }` entries in data order, `GetFarmGorupInfo(kind)` answers `(name, count,
//! description)` (the client's typo is the native name), `GetFarmGroupDoodadList(kind)` the allowed doodads' names,
//! `GetCommonFarmInfo(id)` `(name, guard_time, farm_group_id)` and the runtime getters the player's
//! planted crops. `SCResponseCommonFarmList` is kept as the runtime list; the window's refresh asks
//! for it with `RequestRuntimeCommonFarmDoodadInfo` -> `CSRequestCommonFarmList` (0x10b).
//!
//! `SCShowCommonFarm` (the OpenFarmInfo doodad func) raises `OPEN_COMMON_FARM_INFO(farmType)`, which
//! opens the original info window; `INTERACTION_END` closes it. The farm-area map drawable of
//! `worldmap.ShowCommonFarm` / `SetCommonFarmDrawable` and `SCIsUnitInFarm` are not drawn yet (the
//! packet only updates the farm type state here); see `docs/aa-client-farm.md`.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use aa_net::farm::{CommonFarmEntry, CommonFarmList, FarmEvent};
use aa_net::{Command, Event, NetCommand, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, Table, Value};

use super::api::{HudBridge, Shared};
use super::data::GameData;

/// The original refreshes the runtime list at most this often (`FUN_39035560`, `_DAT_3999d848` =
/// 2.0 s).
const REFRESH_DEBOUNCE_SECS: f64 = 2.0;
/// How far the player may walk from the farm doodad before the info window closes. The native ends
/// the interaction with the doodad; the exact reach is not recovered (capture-pending).
const FARM_LEAVE: f32 = 10.0;

/// The runtime list as received, with the receipt clock the growth/protect counters count down from.
#[derive(Clone, Debug)]
struct Received {
    list: CommonFarmList,
    at: Instant,
    unix_ms: i64,
}

/// The farm doodad whose info window is open (`SCShowCommonFarm`).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Shown {
    farm_type: i32,
    pos: Vec3,
}

#[derive(Default)]
struct View {
    list: Option<Received>,
    show: Option<Shown>,
    /// The last `SCIsUnitInFarmPacket` value (the native's unit farm type at `DAT_39adb270`).
    unit_farm_type: Option<i32>,
    last_request: Option<Instant>,
    commands: Vec<Command>,
}

#[derive(Clone, Default)]
pub(super) struct State(Rc<RefCell<View>>);

fn unix_ms() -> i64 {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

pub fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    let state = State::default();
    lua.set_app_data(state.clone());
    let x2: Table = lua.globals().get("X2")?;

    let b = bridge.clone();
    x2.set(
        "GetFarmGroups",
        lua.create_function(move |lua, _: mlua::MultiValue| {
            let out = lua.create_table()?;
            let data = b.borrow();
            if let Some(game) = data.game.get() {
                for (i, id) in game.housing.farm_group_ids.iter().enumerate() {
                    let Some(group) = game.housing.farm_groups.get(id) else {
                        continue;
                    };
                    // `FUN_394447C0`: each entry is `{ type = id, name = name }`.
                    let entry = lua.create_table()?;
                    entry.set("type", i64::from(*id))?;
                    entry.set("name", group.name.as_str())?;
                    out.raw_set(i + 1, entry)?;
                }
            }
            Ok(out)
        })?,
    )?;

    let b = bridge.clone();
    x2.set(
        "GetFarmGorupInfo",
        lua.create_function(
            move |lua, (_self, kind): (Value, u32)| -> mlua::Result<mlua::MultiValue> {
                let data = b.borrow();
                let Some(group) = data
                    .game
                    .get()
                    .and_then(|g| g.housing.farm_groups.get(&kind))
                else {
                    return Ok(mlua::MultiValue::new());
                };
                // `FUN_394445F0` -> `FUN_39441450`: name at +0xc, count at +4, description at +8.
                Ok(mlua::MultiValue::from_vec(vec![
                    Value::String(lua.create_string(&group.name)?),
                    Value::Integer(i64::from(group.count)),
                    Value::String(lua.create_string(&group.description)?),
                ]))
            },
        )?,
    )?;

    let b = bridge.clone();
    x2.set(
        "GetFarmGroupDoodadList",
        lua.create_function(move |lua, (_self, kind): (Value, u32)| {
            let data = b.borrow();
            let Some(game) = data.game.get() else {
                return Ok(Value::Nil);
            };
            if !game.housing.farm_groups.contains_key(&kind) {
                return Ok(Value::Nil);
            }
            let out = lua.create_table()?;
            for (i, d) in game
                .housing
                .farm_group_doodads
                .iter()
                .filter(|d| d.farm_group_id == kind)
                .enumerate()
            {
                // `FUN_39445BA0` returns the doodad template's name, in row order.
                let name = game.doodad.name(d.doodad_id).unwrap_or_default();
                out.raw_set(i + 1, name)?;
            }
            Ok(Value::Table(out))
        })?,
    )?;

    let b = bridge.clone();
    x2.set(
        "GetCommonFarmInfo",
        lua.create_function(
            move |lua, (_self, id): (Value, u32)| -> mlua::Result<mlua::MultiValue> {
                let data = b.borrow();
                let Some(farm) = data
                    .game
                    .get()
                    .and_then(|g| g.housing.common_farms.iter().find(|f| f.id == id))
                else {
                    return Ok(mlua::MultiValue::new());
                };
                // `FUN_394445B0` -> `FUN_39441370`: name at +0x10, guard_time at +0xc,
                // farm_group_id at +8.
                Ok(mlua::MultiValue::from_vec(vec![
                    Value::String(lua.create_string(&farm.name)?),
                    Value::Integer(i64::from(farm.guard_time)),
                    Value::Integer(i64::from(farm.farm_group_id)),
                ]))
            },
        )?,
    )?;

    let (s, b) = (state.clone(), bridge.clone());
    x2.set(
        "GetRuntimeCommonFarmDoodadsInfo",
        lua.create_function(move |lua, (_self, kind): (Value, u32)| {
            runtime_info(lua, &s, &b, kind)
        })?,
    )?;

    let s = state.clone();
    x2.set(
        "GetRuntimeCommonFarmDoodadCount",
        lua.create_function(move |_, (_self, kind): (Value, u32)| {
            let v = s.0.borrow();
            Ok(v.list.as_ref().map_or(0, |r| {
                r.list
                    .entries
                    .iter()
                    .filter(|e| e.farm_type == kind)
                    .count()
            }) as i64)
        })?,
    )?;

    let (s, b) = (state.clone(), bridge.clone());
    x2.set(
        "RequestRuntimeCommonFarmDoodadInfo",
        lua.create_function(move |lua, _: mlua::MultiValue| {
            let now = Instant::now();
            // `FUN_39035560`: an instant zone refuses the request with
            // `cannot_request_commonfarm_info_in_instant_zone` (0x242), and a request within 2 s
            // only refreshes the window; both still raise `COMMON_FARM_UPDATED`.
            if b.borrow().snapshot.instance_id != 0 {
                aa_ui::lua::queue_ui_event(lua, "COMMON_FARM_UPDATED", Vec::new());
                return Ok(());
            }
            let mut v = s.0.borrow_mut();
            if v.last_request
                .is_some_and(|t| now.duration_since(t).as_secs_f64() < REFRESH_DEBOUNCE_SECS)
            {
                aa_ui::lua::queue_ui_event(lua, "COMMON_FARM_UPDATED", Vec::new());
                return Ok(());
            }
            v.last_request = Some(now);
            v.commands.push(Command::RequestCommonFarmList);
            Ok(())
        })?,
    )?;

    Ok(())
}

/// `GetRuntimeCommonFarmDoodadsInfo`: one table per planted crop of `kind`, or nil when there is
/// none (`FUN_39444630` returns nil for an empty list; `FUN_39035620`).
fn runtime_info(lua: &Lua, state: &State, bridge: &Shared, kind: u32) -> mlua::Result<Value> {
    let v = state.0.borrow();
    let Some(received) = v.list.as_ref() else {
        return Ok(Value::Nil);
    };
    if !received.list.entries.iter().any(|e| e.farm_type == kind) {
        return Ok(Value::Nil);
    }
    let data = bridge.borrow();
    let game = data.game.get();
    let snapshot = &data.snapshot;
    let at = received.at;
    let unix = received.unix_ms;
    let out = lua.create_table()?;
    for (i, entry) in received
        .list
        .entries
        .iter()
        .filter(|e| e.farm_type == kind)
        .enumerate()
    {
        let t = runtime_entry(lua, game, snapshot, entry, at, unix)?;
        out.raw_set(i + 1, t)?;
    }
    Ok(Value::Table(out))
}

fn runtime_entry(
    lua: &Lua,
    game: Option<&GameData>,
    snapshot: &super::data::Snapshot,
    entry: &CommonFarmEntry,
    at: Instant,
    unix_now: i64,
) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    let item = game.map_or(0, |g| {
        g.housing
            .farm_doodad_item(entry.farm_type, entry.template_id)
    });
    let doodad_name = game
        .and_then(|g| g.doodad.name(entry.template_id))
        .unwrap_or_default();
    let name = game
        .and_then(|g| g.items.get(&item))
        .map(|i| i.named.name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| doodad_name.to_string());
    let phase = game
        .and_then(|g| g.doodad.message(entry.func_group_id))
        .unwrap_or_default();
    // The runtime tree counts down from its own receipt; the list carries the growth time left at
    // send time, and the guard time comes from `common_farms` (`FarmGroupKind` fallback when the
    // zone is not carried in the packet).
    let elapsed = at.elapsed().as_millis() as i64;
    let left_growth = i64::from(entry.time_left) - elapsed;
    let guard = game.map_or(0, |g| g.housing.farm_guard_time(entry.farm_type, 0));
    let left_protect = (entry.plant_time * 1000 + i64::from(guard) - unix_now).max(0);
    let location = if snapshot.sub_zone.is_empty() {
        snapshot.zone.clone()
    } else {
        snapshot.sub_zone.clone()
    };
    t.set("name", name)?;
    t.set("currentPhaseName", phase)?;
    t.set("doodadName", doodad_name)?;
    t.set("itemType", i64::from(item))?;
    t.set("leftGrowthTime", left_growth)?;
    t.set("leftProtectTime", left_protect)?;
    t.set("location", location)?;
    t.set("vPosX", f64::from(entry.pos.x))?;
    t.set("vPosY", f64::from(entry.pos.y))?;
    t.set("farmType", i64::from(entry.farm_type))?;
    t.set("growUp", left_growth > 0)?;
    Ok(t)
}

/// Folds aa-net's farm events into the runtime list and the info window.
fn sync(
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<HudBridge>,
    mut events: MessageReader<NetEvent>,
    mut net: MessageWriter<NetCommand>,
) {
    let Some(rt) = rt else { return };
    let Some(state) = rt.lua.app_data_ref::<State>().map(|s| s.clone()) else {
        return;
    };
    let mut closes = false;
    for NetEvent(event) in events.read() {
        match event {
            Event::Farm(FarmEvent::List(list)) => {
                state.0.borrow_mut().list = Some(Received {
                    list: list.clone(),
                    at: Instant::now(),
                    unix_ms: unix_ms(),
                });
                aa_ui::lua::queue_ui_event(&rt.lua, "COMMON_FARM_UPDATED", Vec::new());
            }
            Event::Farm(FarmEvent::Show(show)) => {
                let pos = aa_world::cry_to_bevy(Vec3::new(
                    show.position().x,
                    show.position().y,
                    show.position().z,
                ));
                state.0.borrow_mut().show = Some(Shown {
                    farm_type: show.farm_type,
                    pos,
                });
                // `info.alb` raises the window from `OPEN_COMMON_FARM_INFO(farmType)`.
                aa_ui::lua::queue_ui_event(
                    &rt.lua,
                    "OPEN_COMMON_FARM_INFO",
                    vec![Value::Integer(i64::from(show.farm_type))],
                );
            }
            Event::Farm(FarmEvent::IsUnitInFarm(kind)) => {
                state.0.borrow_mut().unit_farm_type = Some(*kind);
            }
            Event::CharacterSelected(_) | Event::LeftWorld { .. } | Event::Disconnected { .. } => {
                let mut v = state.0.borrow_mut();
                v.list = None;
                v.show = None;
                v.unit_farm_type = None;
                v.last_request = None;
                v.commands.clear();
            }
            _ => {}
        }
    }
    // `INTERACTION_END` closes the info window; walking away from the farm doodad ends it.
    let away = {
        let v = state.0.borrow();
        v.show
            .is_some_and(|show| bridge.0.borrow().pos.distance(show.pos) > FARM_LEAVE)
    };
    if away {
        state.0.borrow_mut().show = None;
        closes = true;
    }
    if closes {
        aa_ui::lua::queue_ui_event(&rt.lua, "INTERACTION_END", Vec::new());
    }
    let commands = std::mem::take(&mut state.0.borrow_mut().commands);
    for command in commands {
        net.write(NetCommand(command));
    }
}

pub fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        sync.after(super::sync_hud)
            .before(aa_ui::UiFrame)
            .run_if(in_state(crate::Screen::InGame))
            .run_if(super::hud_up),
    );
}

#[cfg(test)]
mod tests;
