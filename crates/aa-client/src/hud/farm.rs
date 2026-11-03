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
use web_time::Instant;

use aa_net::farm::{CommonFarmList, FarmEvent};
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
/// `FUN_390324c0` / `FUN_3983c0e0`: at most 50 growth phases are summed (`local_8 = 0x32`).
const GROWTH_PHASE_LIMIT: u32 = 50;

/// One runtime crop as `FUN_390324c0` builds it. The times are the seconds left when the list
/// arrived; `FUN_39035620` subtracts whole seconds since that receipt.
#[derive(Clone, Debug, Default)]
struct RuntimeCrop {
    /// `farm_groups.id`, the key `GetRuntimeCommonFarmDoodadsInfo` filters by.
    farm_group_id: u32,
    /// `common_farms.id`, the script's `farmType`.
    farm_type: u32,
    name: String,
    current_phase_name: String,
    doodad_name: String,
    item_type: u32,
    left_growth: i64,
    left_protect: i64,
    location: String,
    vpos: (f32, f32),
    grow_up: bool,
}

/// The runtime list as received, with the receipt clock the growth/protect counters count down from.
#[derive(Clone, Debug)]
struct Received {
    entries: Vec<RuntimeCrop>,
    at: Instant,
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

fn unix_secs() -> i64 {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
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

    let s = state.clone();
    x2.set(
        "GetRuntimeCommonFarmDoodadsInfo",
        lua.create_function(move |lua, (_self, kind): (Value, u32)| runtime_info(lua, &s, kind))?,
    )?;

    let s = state.clone();
    x2.set(
        "GetRuntimeCommonFarmDoodadCount",
        lua.create_function(move |_, (_self, kind): (Value, u32)| {
            let v = s.0.borrow();
            Ok(v.list.as_ref().map_or(0, |r| {
                r.entries.iter().filter(|e| e.farm_group_id == kind).count()
            }) as i64)
        })?,
    )?;

    let (s, b) = (state.clone(), bridge.clone());
    x2.set(
        "RequestRuntimeCommonFarmDoodadInfo",
        lua.create_function(move |lua, _: mlua::MultiValue| {
            let now = Instant::now();
            // `FUN_39035560`: an instant zone clears the list, shows
            // `cannot_request_commonfarm_info_in_instant_zone` (0x242) and raises
            // `COMMON_FARM_UPDATED`; a request within 2 s only refreshes the window.
            if b.borrow().snapshot.instance_id != 0 {
                s.0.borrow_mut().list = None;
                // `FUN_39428370(0x242)`: `announce_error`'s path, with its CHAT_FAILED fallback.
                let code: i16 = 0x242;
                let name = super::alerts::error_name(code).unwrap_or("UNKNOWN");
                match super::alerts::localized_lua(lua, "ERROR_MSG", name) {
                    Some(text) => aa_ui::lua::queue_ui_event(
                        lua,
                        "SYSMSG",
                        vec![Value::String(lua.create_string(&text)?)],
                    ),
                    None => aa_ui::lua::queue_ui_event(
                        lua,
                        "CHAT_FAILED",
                        vec![
                            Value::String(lua.create_string(format!("Server error {code}"))?),
                            Value::String(lua.create_string("")?),
                        ],
                    ),
                }
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
            // A sent request clears the list before sending (`FUN_39034b10`).
            v.list = None;
            v.last_request = Some(now);
            v.commands.push(Command::RequestCommonFarmList);
            Ok(())
        })?,
    )?;

    Ok(())
}

/// `GetRuntimeCommonFarmDoodadsInfo`: one table per planted crop of `kind`, or nil when there is
/// none (`FUN_39444630` returns nil for an empty list; `FUN_39035620`).
fn runtime_info(lua: &Lua, state: &State, kind: u32) -> mlua::Result<Value> {
    let v = state.0.borrow();
    let Some(received) = v.list.as_ref() else {
        return Ok(Value::Nil);
    };
    let elapsed = received.at.elapsed().as_secs() as i64;
    let mut crops = received.entries.iter().filter(|e| e.farm_group_id == kind);
    if crops.clone().next().is_none() {
        return Ok(Value::Nil);
    }
    let out = lua.create_table()?;
    for (i, crop) in crops.by_ref().enumerate() {
        let t = lua.create_table()?;
        let left_growth = crop.left_growth - elapsed;
        t.set("name", crop.name.as_str())?;
        t.set("currentPhaseName", crop.current_phase_name.as_str())?;
        t.set("doodadName", crop.doodad_name.as_str())?;
        t.set("itemType", i64::from(crop.item_type))?;
        t.set("leftGrowthTime", left_growth)?;
        t.set("leftProtectTime", (crop.left_protect - elapsed).max(0))?;
        t.set("location", crop.location.as_str())?;
        t.set("vPosX", f64::from(crop.vpos.0))?;
        t.set("vPosY", f64::from(crop.vpos.1))?;
        t.set("farmType", i64::from(crop.farm_type))?;
        t.set("growUp", crop.grow_up)?;
        out.raw_set(i + 1, t)?;
    }
    Ok(Value::Table(out))
}

/// Build the runtime list of `FUN_390324c0`: each packet entry is keyed by its **common farm id**,
/// whose `farm_group_id` selects the group; unknown common farms, farm groups, doodads and phase
/// groups are skipped.
fn build_runtime(
    game: Option<&GameData>,
    world: Option<&aa_world::World>,
    list: &CommonFarmList,
    now: i64,
) -> Vec<RuntimeCrop> {
    let Some(game) = game else { return Vec::new() };
    let mut out = Vec::new();
    for entry in &list.entries {
        let Some(common) = game
            .housing
            .common_farms
            .iter()
            .find(|f| f.id == entry.farm_type)
        else {
            continue;
        };
        let Some(group) = game.housing.farm_groups.get(&common.farm_group_id) else {
            continue;
        };
        let Some(doodad_name) = game.housing.farm_doodad_name(entry.template_id) else {
            continue;
        };
        if !game.housing.farm_has_phase(entry.func_group_id) {
            continue;
        }
        let item = game.housing.farm_doodad_item(group.id, entry.template_id);
        let name = game
            .items
            .get(&item)
            .map(|i| i.named.name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| doodad_name.to_string());
        let zone_key = world.and_then(|w| {
            w.zone_at(Vec3::new(entry.pos.x, entry.pos.y, entry.pos.z))
                .map(u32::from)
        });
        let location = zone_key
            .and_then(|key| game.zone(key))
            .map(|(_, zone, _)| zone.text.clone())
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| "unknown".to_string());
        let climate_flag = climate_reduces_growth(game, zone_key, entry.template_id);
        let growth_ms =
            growth_remaining_ms(game, entry.func_group_id, entry.time_left, climate_flag);
        let guard_secs = (common.guard_time as f32 * 0.001) as i64;
        let left_protect = (guard_secs - (now - entry.plant_time)).max(0);
        out.push(RuntimeCrop {
            farm_group_id: group.id,
            farm_type: common.id,
            name,
            current_phase_name: game
                .housing
                .farm_phase_name(entry.func_group_id)
                .unwrap_or_default()
                .to_string(),
            doodad_name: doodad_name.to_string(),
            item_type: item,
            left_growth: (growth_ms / 1000) as i64,
            left_protect,
            location,
            vpos: (entry.pos.x, entry.pos.y),
            // `FUN_390324c0`: `growUp = FUN_3983c170(func_group) == DAT_39adb320`.
            grow_up: game
                .housing
                .farm_growth_step(entry.func_group_id)
                .is_none_or(|step| step.next_phase == 0),
        });
    }
    out
}

/// `FUN_39032470`: climate 1 ("all") always scales growth; otherwise the doodad's climate must be
/// in the zone's `zone_climate_elems` list.
fn climate_reduces_growth(game: &GameData, zone_key: Option<u32>, template_id: u32) -> bool {
    let climate = game.housing.farm_climate(template_id);
    climate == 1
        || (climate != 0
            && zone_key
                .and_then(|key| game.zone_climate.get(&key))
                .and_then(|id| game.zone_climate_elems.get(id))
                .is_some_and(|ids| ids.contains(&climate)))
}

/// `FUN_3983c1b0`: the packet's remaining growth plus the rounded times of the phases after it
/// (`FUN_3983c0e0`), each scaled by the zone-climate flag: 1.0 normally, 0.7 when the doodad's
/// climate is in the zone's climate list (`FUN_39032470`).
fn growth_remaining_ms(
    game: &GameData,
    func_group_id: u32,
    time_left: u32,
    climate_flag: bool,
) -> u32 {
    let scale = if climate_flag { 0.7f32 } else { 1.0 };
    let mut budget = GROWTH_PHASE_LIMIT;
    let mut next = game
        .housing
        .farm_growth_step(func_group_id)
        .map_or(0, |step| step.next_phase);
    let mut sum = 0.0f64;
    while budget > 0 && next != 0 {
        let Some(step) = game.housing.farm_growth_step(next) else {
            break;
        };
        budget -= 1;
        sum += f64::from(step.delay_ms as f32 * scale);
        next = step.next_phase;
    }
    (sum.round() as i64 + i64::from(time_left)).max(0) as u32
}

/// Folds aa-net's farm events into the runtime list and the info window.
fn sync(
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<HudBridge>,
    world: Option<Res<aa_world::stream::WorldInfo>>,
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
                let entries = {
                    let data = bridge.0.borrow();
                    build_runtime(
                        data.game.get(),
                        world.as_ref().map(|w| w.0.as_ref()),
                        list,
                        unix_secs(),
                    )
                };
                state.0.borrow_mut().list = Some(Received {
                    entries,
                    at: Instant::now(),
                });
                aa_ui::lua::queue_ui_event(&rt.lua, "COMMON_FARM_UPDATED", Vec::new());
            }
            Event::Farm(FarmEvent::Show(show)) => {
                let pos = Vec3::new(
                    show.position().x,
                    show.position().y,
                    show.position().z,
                );
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
