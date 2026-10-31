//! Remote vehicle motion, replayed from a recorded drive: the drawn pose of another player's cart
//! must follow the recorded type-2 packets smoothly, the 1.2 default 300 ms behind them, and hold
//! when the packets stop (the 1.2 vehicle controller: `FUN_390b2420` / `FUN_390b0040` /
//! `FUN_390ad150`).
//!
//! The fixture is the S6/S7 live capture case `c_left`
//! (`crates/aa-client/tests/fixtures/remote-vehicle/cart_c_left.tsv`): the original 1.2 client
//! driving a Farm Wagon. Each row is a recorded `CSMoveUnit` 0x089 type-2 body; the lane server
//! rebroadcasts the same `MoveType` in `SCOneUnitMovement`, so it is also the received body. The
//! example drives the real `ingame::units::receive_moves` and `move_units` systems twice: once
//! with a pending `Slave` actor (no model) and once with a drawn entity (the render query), and
//! reads the drawn pose through `Views::unit_drawn` / `RemoteUnit::drawn_pose`.
//!
//! The run passes when, over the moving part of the recording:
//! * every frame moves (no stall) and no frame jumps (speed matches the recorded path),
//! * the drawn position stays within 5 cm of the recorded polyline,
//! * the drawn rotation is within 3 degrees of the recorded segment's rotation,
//! * the drawn path length matches the recorded one within 2 %,
//! * the playback is within 80 ms of the decompile's 300 ms default (an independent literal),
//! * after the last packet the pose holds, within 5 cm of the recorded end, and a stream cut
//!   while moving holds at its last packet.
//!
//! It exits non-zero on any failure. No game files are needed.
//!
//! `cargo run -p aa-client --example remote_vehicle_motion -- --headless`

#![allow(dead_code)]
#[path = "../src/hud/mod.rs"]
mod hud;
#[path = "../src/ingame/mod.rs"]
mod ingame;
#[path = "../src/login/mod.rs"]
mod login;
#[path = "../src/lua_value.rs"]
mod lua_value;
#[path = "../src/script/mod.rs"]
mod script;
#[path = "../src/settings.rs"]
mod settings;
#[path = "../src/sound/mod.rs"]
mod sound;
#[path = "../src/task.rs"]
mod task;

use std::time::Duration;

use aa_net::model::{Equipment, ModelParams, Movement, Unit, UnitKind};
use aa_net::{Event, NetEvent};
use bevy::prelude::*;
use bevy::time::TimeUpdateStrategy;
use ingame::units::{RemoteUnit, Views, move_units, receive_moves};
use lua_value::lua_int;

#[derive(States, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Screen {
    #[default]
    Login,
    WorldSelect,
    CharacterSelect,
    CharacterCreate,
    Loading,
    InGame,
}

/// The captured cart (`slave 79`, Farm Wagon template 60); the fixture bodies carry this id.
const OBJ: u32 = 33993;
/// The playback frame the example drives.
const FRAME: f64 = 1.0 / 60.0;
/// The only default in the 1.2 image for the vehicle delay: 300 ms when the `DAT_39ae4e90`
/// getter is null (`x2game.dll.c:756212`). Used as the oracle, not read from
/// `ingame::vehicle_track::DELAY`, so a wrong code constant fails.
const DECOMPILED_DELAY_MS: f64 = 300.0;
/// The recorded cart brakes to a standstill at the end of the capture; the motion checks stop
/// before it (the hold check covers the rest).
const MOVING_SECONDS: f64 = 5.0;
/// Tolerances: the recorded path is the oracle; the clock's 1 ms nudges may shift the playback.
const PATH_TOLERANCE: f32 = 0.05;
const ROTATION_TOLERANCE: f32 = 3.0f32.to_radians();
const LENGTH_TOLERANCE: f32 = 0.02;
const HOLD_TOLERANCE: f32 = 0.05;
/// How far the measured playback point may sit from the 300 ms target (ms).
const DELAY_TOLERANCE_MS: f64 = 80.0;

struct Row {
    /// The packet `time` (ms), as recorded.
    time: u32,
    /// The arrival on the example's local clock (s from the first packet).
    arrival: f64,
    movement: Movement,
}

fn fixture() -> Vec<Row> {
    let mut rows = Vec::new();
    for line in include_str!("../tests/fixtures/remote-vehicle/cart_c_left.tsv").lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let (ms, hex) = line.split_once('\t').expect("ms and body");
        let bytes: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect();
        let mut r = aa_net::wire::Reader::new(&bytes);
        assert_eq!(r.bc().unwrap(), OBJ, "fixture object id");
        let movement = Movement::read(&mut r).expect("recorded movement");
        assert!(r.is_empty());
        assert_eq!(movement.move_type, 2, "type-2 cart body");
        rows.push(Row {
            time: movement.time,
            arrival: f64::from(ms.parse::<u32>().unwrap()) / 1000.0,
            movement,
        });
    }
    rows
}

/// The unit the server announces before the first move: a level Farm Wagon at the first sample.
fn unit(row: &Row) -> Unit {
    Unit {
        obj_id: OBJ,
        name: "Farm Wagon".into(),
        kind: UnitKind::Slave {
            id: 79,
            tl_id: 79,
            template_id: 60,
            summoner_id: 0,
        },
        owner_name: String::new(),
        pos: row.movement.pos,
        rotation: [0.0, 0.0, row.movement.yaw()],
        scale: 1.0,
        level: 1,
        model_id: 1008,
        equipment: Equipment::default(),
        model_params: ModelParams::default(),
        hp: 1,
        mp: 0,
        race_gender: 0,
        movement: None,
        faction_id: 0,
        expedition_id: 0,
        appellation: 0,
        flags: 0,
        gm_modes: [0; 4],
        bounty_money: None,
        pvp_honor: 0,
        pvp_kills: 0,
        skills: Vec::new(),
        passive_buffs: Vec::new(),
        ability_exp: Vec::new(),
        active_abilities: Vec::new(),
        buffs: Vec::new(),
        anim_action: 0,
        house_posture: None,
        turret: None,
        state_attachments: None,
        attached_to: None,
        doodad_bond: None,
        max_hp: 1,
        max_mp: 0,
        dead: false,
    }
}

fn quat(m: &Movement) -> Quat {
    Quat::from_array(m.rotation_quat().expect("type-2 rotation"))
}

/// The segment nearest `p` on the recorded (x, y) polyline and the fraction along it.
fn nearest(rows: &[Row], p: Vec3) -> (usize, f32, f32) {
    let mut best = (0, 0.0, f32::MAX);
    for (i, pair) in rows.windows(2).enumerate() {
        let a = Vec2::new(pair[0].movement.pos.x, pair[0].movement.pos.y);
        let b = Vec2::new(pair[1].movement.pos.x, pair[1].movement.pos.y);
        let d = b - a;
        let len2 = d.length_squared();
        let t = if len2 > 1e-9 {
            ((Vec2::new(p.x, p.y) - a).dot(d) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let distance = Vec2::new(p.x, p.y).distance(a + d * t);
        if distance < best.2 {
            best = (i, t, distance);
        }
    }
    best
}

/// Runs the real receive / move systems with the packets arriving at their recorded times;
/// `feed` is the last arrival that is sent, `until` the local time to stop drawing. `rendered`
/// spawns a drawn `RemoteUnit` (the real query path) instead of a pending actor.
fn replay(rows: &[Row], feed: f64, until: f64, rendered: bool) -> Vec<(f64, Vec3, Quat)> {
    let mut app = App::new();
    app.add_plugins((
        aa_dev::default_plugins("Remote vehicle motion"),
        aa_dev::DevPlugin,
    ))
    .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
        FRAME,
    )))
    .insert_state(Screen::InGame)
    .init_resource::<Views>()
    .init_resource::<Time>()
    .add_message::<NetEvent>()
    .insert_resource(aa_world::TerrainQuery::new("main_world"))
    .add_systems(Update, (receive_moves, move_units).chain());
    let unit = unit(&rows[0]);
    let entity = if rendered {
        let entity = app
            .world_mut()
            .spawn((
                ingame::player::placement(
                    Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z),
                    unit.yaw(),
                    1.0,
                ),
                RemoteUnit::announced(&unit),
                Visibility::default(),
            ))
            .id();
        app.world_mut()
            .resource_mut::<Views>()
            .track_entity(&unit, entity);
        Some(entity)
    } else {
        app.world_mut()
            .write_message(NetEvent(Event::UnitSpawned(Box::new(unit))));
        None
    };
    app.finish();
    app.cleanup();

    let mut sent = 0usize;
    let mut next = 0usize;
    let mut drawn = Vec::new();
    while (next as f64) * FRAME <= until {
        let local = next as f64 * FRAME;
        while sent < rows.len() && rows[sent].arrival <= local.min(feed) {
            let movement = rows[sent].movement.clone();
            app.world_mut().write_message(NetEvent(Event::UnitMoved {
                obj_id: OBJ,
                movement,
            }));
            sent += 1;
        }
        app.update();
        next += 1;
        let now = app.world().resource::<Time>().elapsed_secs_f64();
        let pose = match entity {
            Some(entity) => app
                .world()
                .get::<RemoteUnit>(entity)
                .map(|unit| unit.drawn_pose()),
            None => app.world().resource::<Views>().unit_drawn(OBJ),
        };
        let Some((pos, rot)) = pose else { continue };
        drawn.push((now, pos, rot));
    }
    drawn
}

/// The checks of one replay: the drawn pose against the recorded path, the 300 ms playback
/// delay, the smoothness and the hold after the last (or cut) packet.
fn check(
    name: &str,
    rows: &[Row],
    drawn: &[(f64, Vec3, Quat)],
    truncated: &[(f64, Vec3, Quat)],
) -> Vec<String> {
    let last = rows.len() - 1;
    let end_pos = Vec3::new(
        rows[last].movement.pos.x,
        rows[last].movement.pos.y,
        rows[last].movement.pos.z,
    );
    let delay = DECOMPILED_DELAY_MS / 1000.0;
    let mut failures = Vec::new();
    let mut path_length = 0.0f32;
    let mut drawn_length = 0.0f32;
    let mut previous: Option<(f64, Vec3)> = None;
    // Local elapsed at the first update (the first packet's arrival); the clock offset is set
    // from it, so the playback time is measurable from here.
    let first_now = drawn.first().map_or(0.0, |(now, _, _)| *now);
    for (index, (now, pos, rot)) in drawn.iter().enumerate() {
        if let Some((last_now, last_pos)) = previous {
            drawn_length += pos.distance(last_pos);
            let _ = last_now;
        }
        previous = Some((*now, *pos));
        // Sender time relative to the first packet: the playback runs the 300 ms delay behind it.
        let sender = *now - first_now - delay;
        if sender < 0.0 || sender > MOVING_SECONDS {
            continue;
        }
        let game = aa_world::bevy_to_cry(*pos);
        let (segment, t, distance) = nearest(rows, game);
        let a = &rows[segment];
        let b = &rows[segment + 1];
        let expected_rot = aa_world::coords::cry_rot_to_bevy(ingame::vehicle_track::nlerp(
            quat(&a.movement),
            quat(&b.movement),
            t,
        ));
        let angle = rot.angle_between(expected_rot);
        // The recorded path between the two nearest packet rows.
        let recorded_speed = Vec3::new(
            b.movement.pos.x - a.movement.pos.x,
            b.movement.pos.y - a.movement.pos.y,
            0.0,
        )
        .length()
            / ((b.time.wrapping_sub(a.time)) as f32 / 1000.0).max(1e-3);
        if distance > PATH_TOLERANCE {
            failures.push(format!(
                "{name} {now:.2}s: {distance:.3} m off the recorded path"
            ));
        }
        if angle > ROTATION_TOLERANCE {
            failures.push(format!(
                "{name} {now:.2}s: rotation {:.1} deg off",
                angle.to_degrees()
            ));
        }
        // The point of the recorded path the cart is on must be 300 ms behind the sender's
        // clock (the decompile default, independent of the code's delay constant).
        let segment_length = Vec3::new(
            b.movement.pos.x - a.movement.pos.x,
            b.movement.pos.y - a.movement.pos.y,
            0.0,
        )
        .length();
        if segment_length > 0.1 {
            let projected =
                f64::from(a.time) + f64::from(t) * f64::from(b.time.wrapping_sub(a.time));
            let expected = f64::from(rows[0].movement.time) + (*now - first_now - delay) * 1000.0;
            if (projected - expected).abs() > DELAY_TOLERANCE_MS {
                failures.push(format!(
                    "{name} {now:.2}s: playing {:.0} ms from the sender's clock",
                    projected - expected
                ));
            }
        }
        if let Some((last_now, last_pos, _)) = drawn.get(index.wrapping_sub(1)) {
            let dt = (*now - *last_now) as f32;
            let step = pos.distance(*last_pos);
            if dt > 0.0 && recorded_speed > 0.5 {
                let expected = recorded_speed * dt;
                if step < expected * 0.25 {
                    failures.push(format!(
                        "{name} {now:.2}s: stalled ({step:.4} m vs {expected:.4} m)"
                    ));
                }
                if step > expected * 2.0 + 0.02 {
                    failures.push(format!(
                        "{name} {now:.2}s: jumped ({step:.3} m vs {expected:.3} m)"
                    ));
                }
            }
        }
    }
    for pair in rows.windows(2) {
        path_length += Vec3::new(
            pair[1].movement.pos.x - pair[0].movement.pos.x,
            pair[1].movement.pos.y - pair[0].movement.pos.y,
            0.0,
        )
        .length();
    }
    let length_error = (drawn_length - path_length).abs() / path_length.max(1.0);
    if length_error > LENGTH_TOLERANCE {
        failures.push(format!(
            "{name}: drawn path {drawn_length:.2} m vs recorded {path_length:.2} m"
        ));
    }
    // After the packets stopped the pose must hold (no extrapolation, FUN_390b2420).
    let tail_start = rows[last].arrival + delay + 0.3;
    let tail: Vec<Vec3> = drawn
        .iter()
        .filter(|(now, _, _)| *now >= tail_start)
        .map(|(_, pos, _)| *pos)
        .collect();
    if tail.len() > 1 {
        let held = tail
            .windows(2)
            .map(|w| w[1].distance(w[0]))
            .fold(0.0f32, f32::max);
        if held > 1e-3 {
            failures.push(format!("{name}: moved {held:.4} m after the last packet"));
        }
    }
    let final_pos = drawn
        .last()
        .map(|(_, pos, _)| aa_world::bevy_to_cry(*pos))
        .unwrap_or(Vec3::ZERO);
    if final_pos.distance(end_pos) > HOLD_TOLERANCE {
        failures.push(format!(
            "{name}: ended {:.3} m from the recorded end",
            final_pos.distance(end_pos)
        ));
    }
    // A stream cut while the cart is still moving must hold where it stopped: the last fed
    // sample, not an extrapolation of the last two.
    if let Some(fed) = rows.iter().rfind(|row| row.arrival <= 3.0) {
        let expected = Vec3::new(fed.movement.pos.x, fed.movement.pos.y, fed.movement.pos.z);
        let end = truncated
            .last()
            .map(|(_, pos, _)| aa_world::bevy_to_cry(*pos))
            .unwrap_or(Vec3::ZERO);
        if end.distance(expected) > HOLD_TOLERANCE {
            failures.push(format!(
                "{name}: after a cut stream the cart stood {:.2} m from the last packet",
                end.distance(expected)
            ));
        }
    }
    failures
}

fn main() -> AppExit {
    let rows = fixture();
    assert!(rows.len() > 40, "the recorded drive is complete");
    let last = rows.len() - 1;
    let end_pos = Vec3::new(
        rows[last].movement.pos.x,
        rows[last].movement.pos.y,
        rows[last].movement.pos.z,
    );
    let mut failures = Vec::new();
    let mut drawn_lengths = Vec::new();
    // Both the pending actor (no model, the example's headless path) and a drawn entity (the
    // real render query) replay the same packets.
    for (name, rendered) in [("pending", false), ("rendered", true)] {
        let drawn = replay(&rows, f64::INFINITY, rows[last].arrival + 2.0, rendered);
        let truncated = replay(&rows, 3.0, 5.0, rendered);
        let length: f32 = drawn.windows(2).map(|w| w[1].1.distance(w[0].1)).sum();
        drawn_lengths.push(length);
        failures.extend(check(name, &rows, &drawn, &truncated));
    }
    if failures.is_empty() {
        info!(
            "remote_vehicle_motion: ok, {} samples, drawn path {:.2} m / {:.2} m (pending / rendered), final ({:.2}, {:.2}, {:.2})",
            rows.len(),
            drawn_lengths[0],
            drawn_lengths[1],
            end_pos.x,
            end_pos.y,
            end_pos.z
        );
        AppExit::Success
    } else {
        for failure in failures.iter().take(20) {
            eprintln!("remote_vehicle_motion: FAIL: {failure}");
        }
        eprintln!("remote_vehicle_motion: {} failures", failures.len());
        AppExit::error()
    }
}
