//! Map clicks request a server ping; only the echoed position reaches either map.
use std::{cell::RefCell, rc::Rc};

use aa_net::{
    party::{Party, PartyCommand, PartyEvent},
    wire::Pos,
};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, Table};

use super::super::{
    api::{HudBridge, Request},
    data::GameData,
};

#[derive(Clone, Copy, Debug)]
enum Req {
    Place { level: u8, id: u32, u: f32, v: f32 },
    Remove,
}

#[derive(Clone, Debug, PartialEq)]
struct DisplayKey {
    ping: Option<(Pos, u32)>,
    instance: u32,
    loaded: bool,
    version: u64,
}

#[derive(Default)]
struct State {
    requests: Vec<Req>,
    displayed: Option<DisplayKey>,
    version: u64,
}

#[derive(Clone, Default)]
struct Queue(Rc<RefCell<State>>);

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        sync.after(super::sync)
            .before(super::super::api::handle_requests)
            .run_if(super::super::hud_up)
            .run_if(in_state(crate::Screen::InGame)),
    );
}

pub(super) fn install(lua: &Lua, hud: &Table) -> mlua::Result<()> {
    let q = Queue::default();
    lua.set_app_data(q.clone());
    hud.set("party_ping", false)?;
    let place = q.clone();
    hud.set(
        "place_ping",
        lua.create_function(move |_, (level, id, u, v): (u8, u32, f32, f32)| {
            let mut state = place.0.borrow_mut();
            // A pointer action has one destination; repeated callbacks in one frame keep the latest.
            state.requests.clear();
            state.requests.push(Req::Place { level, id, u, v });
            Ok(true)
        })?,
    )?;
    hud.set(
        "remove_ping",
        lua.create_function(move |_, ()| {
            let mut state = q.0.borrow_mut();
            state.requests.clear();
            state.requests.push(Req::Remove);
            Ok(true)
        })?,
    )
}

fn allowed(party: &Party, own: u32) -> bool {
    own != 0 && (party.id == 0 || party.owner_id == own || party.marks.contains(&(1, own)))
}

fn rect(game: &GameData, level: u8, id: u32) -> Option<[f32; 4]> {
    match level {
        0 if id == 2 => game.world_rects.get(&id).copied(),
        1 if matches!(id, 3..=5) => game.world_rects.get(&id).copied(),
        2 => game.zone_groups.get(&id).map(|g| g.rect),
        3 => game.sub_zone_rects.get(&id).copied(),
        _ => None,
    }
    .filter(|r| r.iter().all(|v| v.is_finite()) && r[2] > 0.0 && r[3] > 0.0)
}

fn position(r: [f32; 4], u: f32, v: f32) -> Option<Vec2> {
    if !u.is_finite() || !v.is_finite() || !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
        return None;
    }
    let p = Vec2::new(r[0] + u * r[2], r[1] + (1.0 - v) * r[3]);
    p.is_finite().then_some(p)
}

/// Named projections retain every overlapping zone/city instead of guessing a unique map.
fn projections(game: &GameData, p: Pos) -> Vec<(u8, u32, f32, f32)> {
    let mut maps = vec![(0, 2)];
    maps.extend((3..=5).map(|id| (1, id)));
    maps.extend(game.zone_groups.keys().map(|id| (2, *id)));
    maps.extend(game.sub_zone_rects.keys().map(|id| (3, *id)));
    maps.sort_unstable();
    maps.into_iter()
        .filter_map(|(level, id)| {
            let r = rect(game, level, id)?;
            let (u, v) = ((p.x - r[0]) / r[2], 1.0 - (p.y - r[1]) / r[3]);
            (u.is_finite() && v.is_finite() && (0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v))
                .then_some((level, id, u, v))
        })
        .take(256)
        .collect()
}

fn sync(
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<HudBridge>,
    terrain: Option<Res<aa_world::TerrainQuery>>,
    water: Option<Res<aa_world::WaterQuery>>,
    mut events: MessageReader<aa_net::NetEvent>,
) {
    let echoes = events
        .read()
        .filter(|aa_net::NetEvent(e)| matches!(e, aa_net::Event::Party(PartyEvent::Ping { .. })))
        .count();
    let Some(rt) = rt else { return };
    let Some(q) = rt.lua.app_data_ref::<Queue>().map(|q| q.clone()) else {
        return;
    };
    let requests = std::mem::take(&mut q.0.borrow_mut().requests);
    let mut d = bridge.0.borrow_mut();
    for request in requests {
        if !allowed(&d.party, d.character_id) {
            continue;
        }
        let command = match request {
            Req::Remove => Some(PartyCommand::SetPing {
                team_id: d.party.id,
                has_ping: false,
                pos: Pos::default(),
                instance_id: 0,
            }),
            Req::Place { level, id, u, v } => {
                let p = d
                    .game
                    .get()
                    .and_then(|g| rect(g, level, id))
                    .and_then(|r| position(r, u, v));
                p.and_then(|p| {
                    // Native queries two surfaces from Z=1000 and takes max+2. Our terrain and
                    // authored water queries reproduce ground/sea/lake placement. Brush roofs and
                    // instance-specific collision surfaces are not represented by these queries yet.
                    let ground = terrain.as_ref().and_then(|q| q.ground_at(p.x, p.y)).map(|ground| ground.height);
                    let surface = water
                        .as_ref()
                        .and_then(|q| q.water_at(Vec3::new(p.x, p.y, 1000.0)))
                        .map(|(z, _)| z);
                    let z = match (ground, surface) {
                        (Some(a), Some(b)) => Some(a.max(b)),
                        (a, b) => a.or(b),
                    }?;
                    let s = &d.snapshot;
                    let own_map = if s.sub_map != 0 {
                        (3, s.sub_map)
                    } else {
                        (2, s.zone_group)
                    };
                    Some(PartyCommand::SetPing {
                        team_id: d.party.id,
                        has_ping: true,
                        pos: Pos::new(p.x, p.y, z + 2.0),
                        instance_id: if (level, id) == own_map {
                            s.instance_id
                        } else {
                            0
                        },
                    })
                })
            }
        };
        if let Some(command) = command {
            d.requests.push(Request::Party(command));
        }
    }
    let mut state = q.0.borrow_mut();
    state.version = state.version.wrapping_add(echoes as u64);
    let key = DisplayKey {
        ping: d.party.ping,
        instance: d.snapshot.instance_id,
        loaded: d.game.get().is_some(),
        version: state.version,
    };
    if state.displayed.as_ref() == Some(&key) {
        return;
    }
    let Ok(hud) = rt.lua.globals().get::<Table>("HUD") else {
        return;
    };
    let update = (|| -> mlua::Result<()> {
        let points = key
            .ping
            .filter(|(_, instance)| *instance == 0 || *instance == key.instance)
            .and_then(|(p, _)| d.game.get().map(|g| projections(g, p)))
            .unwrap_or_default();
        if let Some(&(level, id, u, v)) = points.first() {
            let ping = rt.lua.create_table()?;
            ping.set("level", level)?;
            ping.set("id", id)?;
            ping.set("u", u)?;
            ping.set("v", v)?;
            ping.set("version", key.version)?;
            let maps = rt.lua.create_table()?;
            for (i, (level, id, u, v)) in points.iter().enumerate() {
                let point = rt.lua.create_table()?;
                point.set("level", *level)?;
                point.set("id", *id)?;
                point.set("u", *u)?;
                point.set("v", *v)?;
                maps.set(i + 1, point)?;
            }
            ping.set("maps", maps)?;
            hud.set("party_ping", ping)
        } else {
            hud.set("party_ping", false)
        }
    })();
    if let Err(e) = update {
        warn!("hud: party ping: {e}");
        return;
    }
    state.displayed = Some(key);
    drop(state);
    drop(d);
    rt.fire_event("UPDATE_PING_INFO", ());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_ping_waits_for_echo_and_uses_surface_height_and_map_instance()
    -> Result<(), Box<dyn std::error::Error>> {
        if !std::path::Path::new(&std::env::var("AA_PAK").unwrap_or_else(|_| aa_pak::DEFAULT_PAK.into())).exists() {
            return Ok(());
        }
        let rt = UiRuntime::new("en_us", 1280.0, 720.0)?;
        rt.lua.load("HUD={state={zone_key_name='w_test'}}; HUD_ROAD_FRAMES={[2]={},[3]={}}; AAUI.WorldMap=function() return {} end; AAUI.RoadMap=function() return {level=2,zone=42} end").exec()?;
        let hud: Table = rt.lua.globals().get("HUD")?;
        install(&rt.lua, &hud)?;
        rt.lua.load(include_str!("../road_projection.lua")).exec()?;
        rt.lua.load(include_str!("../party_ping.lua")).exec()?;
        let mut game = GameData::default();
        let rect = [15_572.3, 15_374.9, 10.0, 10.0];
        game.zone_groups.insert(
            42,
            super::super::super::data::ZoneGroup { rect, ..default() },
        );
        game.world_rects.insert(2, [0.0, 0.0, 32768.0, 32768.0]);
        let mut data = super::super::super::api::HudData {
            character_id: 11,
            ..default()
        };
        data.snapshot.instance_id = 17;
        data.snapshot.zone_group = 42;
        assert!(data.game.0.set(game).is_ok());
        let bridge = Rc::new(RefCell::new(data));
        let terrain = aa_world::TerrainQuery::new("main_world");
        let water = aa_world::WaterQuery::with_terrain("main_world", terrain.clone());
        let mut app = App::new();
        app.add_message::<aa_net::NetEvent>();
        app.add_systems(Update, sync);
        app.insert_resource(terrain);
        app.insert_resource(water);
        app.insert_non_send(HudBridge(bridge.clone()));
        app.insert_non_send(rt);
        app.world()
            .non_send::<UiRuntime>()
            .lua
            .load("assert(AAUI.PlaceMapPing(2,42,0.5,0.5)); assert(AAUI.WorldMap().ping==false)")
            .exec()?;
        app.update();
        let (pos, instance) = match bridge.borrow_mut().requests.pop() {
            Some(Request::Party(PartyCommand::SetPing {
                team_id: 0,
                has_ping: true,
                pos,
                instance_id,
            })) => (pos, instance_id),
            other => panic!("wrong ping request: {other:?}"),
        };
        assert!((pos.x - 15_577.3).abs() < 0.01 && (pos.y - 15_379.9).abs() < 0.01);
        assert!((pos.z - 129.0).abs() < 0.01, "lake surface+2: {pos:?}");
        assert_eq!(instance, 17);
        assert!(bridge.borrow().party.ping.is_none());
        app.world()
            .non_send::<UiRuntime>()
            .lua
            .load("assert(AAUI.WorldMap().ping==false)")
            .exec()?;
        bridge.borrow_mut().party.ping = Some((pos, instance));
        app.world_mut()
            .write_message(aa_net::NetEvent(aa_net::Event::Party(PartyEvent::Ping {
                has_ping: true,
                pos,
                instance_id: instance,
            })));
        app.update();
        app.world().non_send::<UiRuntime>().lua.load("local p=AAUI.RoadMap().ping; assert(p and math.abs(p.u-0.5)<0.001 and math.abs(p.v-0.5)<0.001); oldVersion=p.version; assert(AAUI.WorldMap().ping)").exec()?;
        app.world_mut()
            .write_message(aa_net::NetEvent(aa_net::Event::Party(PartyEvent::Ping {
                has_ping: true,
                pos,
                instance_id: instance,
            })));
        app.update();
        app.world().non_send::<UiRuntime>().lua.load("assert(AAUI.RoadMap().ping.version>oldVersion); assert(AAUI.PlaceMapPing(0,2,0.5,0.5))").exec()?;
        app.update();
        assert!(matches!(
            bridge.borrow_mut().requests.pop(),
            Some(Request::Party(PartyCommand::SetPing { instance_id: 0, .. }))
        ));
        bridge.borrow_mut().snapshot.instance_id = 99;
        app.update();
        app.world().non_send::<UiRuntime>().lua.load("assert(AAUI.WorldMap().ping==false and AAUI.RoadMap().ping==false); assert(AAUI.RemoveMapPing())").exec()?;
        app.update();
        assert!(
            matches!(bridge.borrow_mut().requests.pop(),Some(Request::Party(PartyCommand::SetPing {has_ping:false,pos,instance_id:0,..})) if pos==Pos::default())
        );
        Ok(())
    }

    #[test]
    fn ping_coordinates_keep_named_overlaps_and_invert_map_y() {
        let mut game = GameData::default();
        game.world_rects.insert(2, [0.0, 0.0, 100.0, 100.0]);
        for id in 1..=9 {
            game.sub_zone_rects.insert(id, [10.0, 20.0, 20.0, 40.0]);
        }
        assert_eq!(
            position([10.0, 20.0, 20.0, 40.0], 0.25, 0.75),
            Some(Vec2::new(15.0, 30.0))
        );
        assert!(position([0.0, 0.0, 1.0, 1.0], f32::NAN, 0.0).is_none());
        assert!(position([0.0, 0.0, 1.0, 1.0], 1.1, 0.0).is_none());
        let points = projections(&game, Pos::new(15.0, 30.0, 8.0));
        assert_eq!(points.len(), 10);
        assert_eq!(points.last(), Some(&(3, 9, 0.25, 0.75)));
        let mut party = Party {
            id: 3,
            owner_id: 11,
            ..default()
        };
        assert!(allowed(&party, 11));
        assert!(!allowed(&party, 12));
        party.marks[8] = (1, 12);
        assert!(allowed(&party, 12));
        party.marks[8] = (2, 12);
        assert!(!allowed(&party, 12));
        assert!(!allowed(&Party::default(), 0));
    }
}
