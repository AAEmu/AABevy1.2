//! Received telescope snapshots feed the original Map widget and Lua observers.
use std::{collections::HashMap, sync::Arc};

use aa_net::{Event, FishRadar, NetEvent, ShipRadar, TransferRadar, radar::RadarEvent};
use aa_ui::{
    UiRuntime,
    lua::radar::{self, Contact, MapBounds, ShipTooltip, Snapshot, Symbol, Tooltip},
};
use bevy::prelude::*;
use mlua::{Lua, MultiValue, Table, Value};
use rusqlite::Connection;

use super::{
    api::{HudBridge, HudData, Shared},
    data::GameData,
};

#[allow(dead_code)]
pub(super) mod fixture;

#[derive(Default)]
pub(super) struct Tables {
    transfers: HashMap<u32, Symbol>,
    kinds: HashMap<u32, &'static str>,
    fish_names: HashMap<u32, String>,
}

impl Tables {
    pub(super) fn load(db: &Connection) -> rusqlite::Result<Self> {
        let mut out = Self::default();
        let mut query = db.prepare("SELECT t.id,m.name FROM transfers t JOIN models m ON m.id=t.model_id")?;
        for row in query.query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, Option<String>>(1)?)))? {
            let (id, name) = row?;
            if let Some(name) = name {
                out.transfers.insert(
                    id,
                    match name.as_str() {
                        "airship" => Symbol::Airship,
                        "carriage_main" => Symbol::Carriage,
                        _ => Symbol::Landship,
                    },
                );
            }
        }
        let mut query = db.prepare("SELECT id,slave_kind_id FROM slaves")?;
        for row in query.query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, Option<u32>>(1)?)))? {
            let (id, kind) = row?;
            out.kinds.insert(
                id,
                match kind.unwrap_or(0) {
                    1 => "big_sailing_ship",
                    2 => "small_sailing_ship",
                    3 => "speedboat",
                    4 => "boat",
                    5 => "tank",
                    6 => "machine",
                    7 => "siege_weapon",
                    8 => "slave_equipment",
                    9 => "fishboat",
                    _ => "invalid slave_kind",
                },
            );
        }
        let mut query = db.prepare("SELECT d.id,coalesce(l.en_us,d.name,'') FROM doodad_almighties d LEFT JOIN localized_texts l ON l.tbl_name='doodad_almighties' AND l.tbl_column_name='name' AND l.idx=d.id AND l.en_us IS NOT NULL AND l.en_us<>''")?;
        for row in query.query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, String>(1)?)))? {
            let (id, name) = row?;
            out.fish_names.insert(id, name);
        }
        Ok(out)
    }
}

/// Once active, retain packet order rather than replacing this with the frame's final resources.
#[derive(Default)]
pub(super) struct View {
    owner: Option<(u32, u32)>,
    active: bool,
    faction: u32,
    ships: ShipRadar,
    transfers: TransferRadar,
    fish: FishRadar,
    observed: [Vec<u32>; 3],
}

pub(super) fn refresh(world: &World, data: &mut HudData, spawn: &aa_net::session::PlayerSpawn) {
    let owner = (spawn.character().id, spawn.obj_id);
    if data.radar.active && data.radar.owner == Some(owner) {
        return;
    }
    data.radar = View {
        owner: Some(owner),
        faction: spawn.character().faction_id,
        ships: world.get_resource::<ShipRadar>().cloned().unwrap_or_default(),
        transfers: world.get_resource::<TransferRadar>().cloned().unwrap_or_default(),
        fish: world.get_resource::<FishRadar>().cloned().unwrap_or_default(),
        ..default()
    };
}

fn maps(game: &GameData) -> Vec<MapBounds> {
    let mut out = Vec::new();
    for (&id, &rect) in &game.world_rects {
        if matches!(id, 2..=5) {
            out.push(MapBounds {
                level: u8::from(id != 2),
                id,
                rect,
            });
        }
    }
    out.extend(game.zone_groups.iter().map(|(&id, g)| MapBounds { level: 2, id, rect: g.rect }));
    out.extend(game.sub_zone_rects.iter().map(|(&id, &rect)| MapBounds { level: 3, id, rect }));
    out.retain(|m| m.rect.iter().all(|v| v.is_finite()) && m.rect[2] > 0. && m.rect[3] > 0.);
    out.sort_unstable_by_key(|m| (m.level, m.id));
    out
}

fn snapshot(data: &HudData) -> Snapshot {
    let Some(game) = data.game.get() else { return Snapshot::default() };
    let view = &data.radar;
    let ships = view
        .ships
        .contacts
        .iter()
        .flatten()
        .map(|row| {
            // The app's existing static faction relation is partial: native39838880 additionally
            // consults teams, force attack and polymorphic ownership. Keep that gap explicit.
            let enemy = game.relation(view.faction, row.faction_id) == "hostile";
            let symbol = match (row.raw_unit_type == 2, enemy) {
                (true, false) => Symbol::Ship,
                (true, true) => Symbol::EnemyShip,
                (false, false) => Symbol::Shipyard,
                (false, true) => Symbol::EnemyShipyard,
            };
            Contact {
                obj_id: row.obj_id,
                position: [row.pos.x, row.pos.y],
                symbol,
                tooltip: Some(Tooltip::Ship(ShipTooltip {
                    name: Some(row.name.clone()),
                    owner: Some(row.summoner_name.clone()),
                    kind: game.radar.kinds.get(&row.template_id).map(|s| (*s).to_owned()),
                    // An expedition ID alone is not a received expedition name.
                    expedition: None,
                    buff: Some(row.raw_tail == 3554),
                    hp: Some(row.hp.min(row.max_hp) as f32),
                    max_hp: Some(row.max_hp as f32),
                })),
            }
        })
        .collect();
    let transfers = view
        .transfers
        .contacts
        .iter()
        .flatten()
        .filter_map(|row| {
            Some(Contact {
                obj_id: row.obj_id,
                position: [row.pos.x, row.pos.y],
                symbol: *game.radar.transfers.get(&row.template_id)?,
                tooltip: Some(Tooltip::Text(row.name.clone())),
            })
        })
        .collect();
    let fish = view
        .fish
        .contacts
        .iter()
        .flatten()
        .map(|row| Contact {
            obj_id: row.obj_id,
            position: [row.pos.x, row.pos.y],
            symbol: Symbol::FishSchool,
            tooltip: game.radar.fish_names.get(&row.template_id).cloned().map(Tooltip::Text),
        })
        .collect();
    Snapshot {
        maps: Arc::new(maps(game)),
        ships: Arc::new(ships),
        transfers: Arc::new(transfers),
        fish: Arc::new(fish),
    }
}

pub(super) fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    publish(lua, bridge);
    let state = bridge.clone();
    let map: Table = lua.globals().get("X2Map")?;
    map.set(
        "GetTelescopeUnitHealth",
        lua.create_function(move |_, (_, value): (Value, Value)| {
            let id = match value {
                Value::Integer(n) => u32::try_from(n).ok(),
                Value::Number(n) if n.is_finite() && n.fract() == 0. && (0.0..=u32::MAX as f64).contains(&n) => Some(n as u32),
                _ => None,
            };
            let data = state.borrow();
            let row = id.and_then(|id| data.radar.ships.contacts.as_ref()?.iter().find(|r| r.obj_id == id));
            Ok(row.map_or_else(MultiValue::new, |row| {
                MultiValue::from_vec(vec![Value::Number(f64::from(row.hp as f32)), Value::Number(f64::from(row.max_hp as f32))])
            }))
        })?,
    )
}

pub(super) fn publish(lua: &Lua, bridge: &Shared) {
    radar::set_snapshot(lua, snapshot(&bridge.borrow()));
}

const AREA: [&str; 3] = ["UPDATE_TELESCOPE_AREA", "UPDATE_TRANSFER_TELESCOPE_AREA", "UPDATE_FISH_SCHOOL_AREA"];
const UPDATE: [&str; 3] = ["UPDATE_TELESCOPE_INFO", "UPDATE_TRANSFER_TELESCOPE_INFO", "UPDATE_FISH_SCHOOL_INFO"];
const REMOVE: [&str; 3] = ["REMOVE_TELESCOPE_INFO", "REMOVE_TRANSFER_TELESCOPE_INFO", "REMOVE_FISH_SCHOOL_INFO"];
const CLEAR: [&str; 3] = ["CLEAR_TELESCOPE_INFO", "CLEAR_TRANSFER_TELESCOPE_INFO", "CLEAR_FISH_SCHOOL_INFO"];

fn observe(rt: &UiRuntime, bridge: &Shared, channel: usize) {
    let (on, ids, old) = {
        let mut data = bridge.borrow_mut();
        let v = &mut data.radar;
        let (on, ids) = match channel {
            0 => (
                v.ships.toggle.is_some_and(|t| t.on),
                v.ships.contacts.iter().flatten().map(|r| r.obj_id).collect(),
            ),
            1 => (
                v.transfers.toggle.is_some_and(|t| t.on),
                v.transfers.contacts.iter().flatten().map(|r| r.obj_id).collect(),
            ),
            _ => (
                v.fish.toggle.is_some_and(|t| t.on),
                v.fish.contacts.iter().flatten().map(|r| r.obj_id).collect(),
            ),
        };
        let old = std::mem::replace(&mut v.observed[channel], ids);
        (on, v.observed[channel].clone(), old)
    };
    if on {
        for id in old {
            if !ids.contains(&id) {
                rt.fire_event(REMOVE[channel], f64::from(id));
            }
        }
        rt.fire_event(UPDATE[channel], ());
    } else {
        rt.fire_event(CLEAR[channel], ());
    }
}

fn dispatch(rt: &UiRuntime, bridge: &Shared, event: &Event) {
    let notice = match event {
        Event::Radar(RadarEvent::ShipToggled(_)) => Some((0, true)),
        Event::Radar(RadarEvent::TransferToggled(_)) => Some((1, true)),
        Event::Radar(RadarEvent::FishToggled(_)) => Some((2, true)),
        Event::Radar(RadarEvent::ShipContacts { last: true, .. }) => Some((0, false)),
        Event::Radar(RadarEvent::TransferContacts { last: true, .. }) => Some((1, false)),
        Event::Radar(RadarEvent::FishContacts { last: true, .. }) => Some((2, false)),
        _ => None,
    };
    let changed = {
        let mut data = bridge.borrow_mut();
        let v = &mut data.radar;
        let a = v.ships.apply(event);
        let b = v.transfers.apply(event);
        let c = v.fish.apply(event);
        a || b || c
    };
    if !changed {
        return;
    }
    publish(&rt.lua, bridge);
    if let Some((channel, area)) = notice {
        if area {
            rt.fire_event(AREA[channel], ());
        }
        observe(rt, bridge, channel);
    } else if matches!(
        event,
        Event::CharacterSelected(_) | Event::LeftWorld { .. } | Event::Disconnected { .. } | Event::Kicked { .. }
    ) {
        for channel in 0..3 {
            observe(rt, bridge, channel);
        }
    }
}

fn route(mut events: MessageReader<NetEvent>, rt: Option<NonSend<UiRuntime>>, bridge: NonSend<HudBridge>) {
    let Some(rt) = rt else {
        events.clear();
        return;
    };
    let first = {
        let mut data = bridge.0.borrow_mut();
        let first = !data.radar.active;
        data.radar.active = true;
        first
    };
    if first {
        events.clear();
        publish(&rt.lua, &bridge.0);
        for channel in 0..3 {
            let known = {
                let data = bridge.0.borrow();
                match channel {
                    0 => data.radar.ships.toggle.is_some(),
                    1 => data.radar.transfers.toggle.is_some(),
                    _ => data.radar.fish.toggle.is_some(),
                }
            };
            if known {
                observe(&rt, &bridge.0, channel);
            }
        }
    } else {
        for NetEvent(event) in events.read() {
            dispatch(&rt, &bridge.0, event);
        }
    }
}

pub(super) fn plugin(app: &mut App) {
    app.add_systems(Update, route.after(super::sync_hud).before(aa_ui::UiFrame).run_if(super::hud_up));
    app.add_systems(OnExit(crate::Screen::InGame), |bridge: NonSend<HudBridge>| {
        bridge.0.borrow_mut().radar = default();
    });
}

#[cfg(test)]
mod tests;
