//! S14 self-check: the Experimental Glider (quest 1065's reward, item 14677, buff 1029) through the real systems.
//!
//! The example drives `glide::track_flight`, `glide::request_deploy`, `glide::control_glide` and
//! `player::control_player` on a deterministic 60 Hz clock over the real `main_world` terrain (AA_PAK and AA_DB are
//! required). It injects buff 1029 as the server's `SCBuffCreated` would, removes it as `remove_on_land` would after the
//! character touches the ground, and folds it in the air. The wire check requires every flight packet to be the captured
//! stance 1 / actor 0 shape, the touchdown to send `CSRemoveBuff` (index, reason 9) before the captured landing packet
//! (stance 1, flags 6, actor 0x14), and the fold not to require a landing wire.
//!
//! It asserts: the opening climbs at 3 m/s for 0.7 s (about 2.1 m, `FUN_3912d860`'s double sign flip), 4 s of gliding
//! at the data's 6.00 m/s forward and -1.50 m/s down, the landing slide ends within its 1.00 s and hands control back,
//! and an in-air fold hands the vertical speed to gravity. A 20 s simulated budget fails a run that never gets there
//! (a never-ending slide, a broken fold), so mutations are caught instead of hanging.
//!
//! `cargo run -p aa-client --example glider_flight -- --mute --offscreen-shot shots/aa-client/glider_flight.png --shot-after 15`
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

use aa_net::state::{BuffEntry, Buffs};
use aa_net::{Command, NetCommand};
use bevy::ecs::system::RunSystemOnce;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use ingame::glide::{self, Phase};
use ingame::player::{self, LocalPlayer, Mover, PlayerInput, SendState};
use lua_value::lua_int;

#[derive(States, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Screen {
    #[default]
    WorldSelect,
    Login,
    CharacterSelect,
    CharacterCreate,
    Loading,
    InGame,
}

const DT: f32 = 1.0 / 60.0;
const BUDGET: f32 = 20.0;
const START_ABOVE_GROUND: f32 = 10.0;
const FOLD_ABOVE_GROUND: f32 = 20.0;
/// A flat, solid main_world point (the lifecycle test's `solid` sample).
const SPAWN: (f32, f32) = (21188.25, 9030.5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Deploy,
    Climb,
    Glide,
    LandWait,
    SlideWait,
    Sliding,
    FoldWait,
    FoldStart,
    FoldGlide,
    FoldFall,
    Done,
}

#[derive(Resource)]
struct Sim {
    stage: Stage,
    time: f32,
    accumulator: f32,
    ground: f32,
    start_z: f32,
    slide_start: f32,
    fold_z: f32,
    fold_vz: f32,
    release: f32,
    buff_on: bool,
    keep_open: bool,
    done: bool,
    glide_wire: bool,
    landing_wire: bool,
    remove_sent: bool,
}

#[derive(Component)]
struct Pilot;
#[derive(Component)]
struct Wing;
#[derive(Component)]
struct FollowCam;

fn main() -> AppExit {
    if !aa_cry::pak_available() {
        eprintln!("glider_flight: skipped, the game pak is missing (AA_PAK=<1.2 client>/game_pak)");
        return AppExit::Success;
    }
    if !aa_anim::GameDb::available() {
        eprintln!(
            "glider_flight: skipped, the game database is missing (AA_DB=<server>/compact.sqlite3)"
        );
        return AppExit::Success;
    }
    let terrain = aa_world::TerrainQuery::new("main_world");
    let Some(ground) = terrain.ground_at(SPAWN.0, SPAWN.1).map(|g| g.height) else {
        eprintln!("glider_flight: skipped, no terrain at {SPAWN:?}");
        return AppExit::Success;
    };
    let args = aa_dev::args();
    let keep_open = args.iter().any(|a| {
        matches!(
            a.as_str(),
            "--shot" | "--offscreen-shot" | "--exit-after" | "--record"
        )
    });
    let mut app = App::new();
    app.add_plugins((
        aa_dev::default_plugins("aa-client: glider flight"),
        aa_dev::DevPlugin,
    ))
    .init_state::<Screen>()
    .insert_state(Screen::InGame)
    .insert_resource(terrain.clone())
    .insert_resource(aa_world::WorldCollision::new("main_world", terrain.clone()))
    .insert_resource(aa_world::WaterQuery::new("main_world"))
    .init_resource::<Buffs>()
    .init_resource::<glide::Gliding>()
    .init_resource::<SendState>()
    .init_resource::<PlayerInput>()
    .init_resource::<ingame::LocalJump>()
    .init_resource::<ingame::LocalMount>()
    .init_resource::<ingame::units::Views>()
    .init_resource::<crate::settings::Actions>()
    .init_resource::<ButtonInput<KeyCode>>()
    .init_resource::<ButtonInput<MouseButton>>()
    .init_resource::<AccumulatedMouseMotion>()
    .insert_resource(ingame::DevOptions::from_args())
    .add_message::<NetCommand>()
    .insert_resource(Sim {
        stage: Stage::Deploy,
        time: 0.0,
        accumulator: 0.0,
        ground,
        start_z: 0.0,
        slide_start: 0.0,
        fold_z: 0.0,
        fold_vz: 0.0,
        release: 0.0,
        buff_on: false,
        keep_open,
        done: false,
        glide_wire: false,
        landing_wire: false,
        remove_sent: false,
    })
    .insert_resource(ClearColor(Color::srgb(0.62, 0.74, 0.88)))
    .add_systems(Startup, stage)
    .add_systems(Update, (step, follow).chain());
    app.world_mut().resource_mut::<SendState>().settle_now();
    app.run()
}

fn stage(
    mut commands: Commands,
    sim: Res<Sim>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let ground = sim.ground;
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(80.0, 80.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.24, 0.35, 0.17),
            perceptual_roughness: 1.0,
            ..default()
        })),
        Transform::from_xyz(SPAWN.0, ground, SPAWN.1),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 12_000.0,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, -0.5, 0.0)),
    ));
    commands.spawn((
        FollowCam,
        Camera3d::default(),
        ingame::camera::OrbitCamera::default(),
        Transform::from_xyz(
            SPAWN.0 - 12.0,
            ground + START_ABOVE_GROUND + 6.0,
            SPAWN.1 + 14.0,
        )
        .looking_at(
            Vec3::new(SPAWN.0, ground + START_ABOVE_GROUND, SPAWN.1),
            Vec3::Y,
        ),
    ));
    commands.spawn((
        Pilot,
        LocalPlayer { obj_id: 7 },
        Mover {
            pos: Vec3::new(SPAWN.0, SPAWN.1, ground + START_ABOVE_GROUND),
            facing: 0.0,
            vz: 0.0,
            grounded: false,
            velocity: Vec3::ZERO,
            swim: None,
            wading: 0.0,
        },
        Transform::from_xyz(SPAWN.0, ground + START_ABOVE_GROUND, SPAWN.1),
        ingame::Motion::default(),
        Mesh3d(meshes.add(Capsule3d::new(0.35, 1.1))),
        MeshMaterial3d(materials.add(Color::srgb(0.35, 0.45, 0.75))),
    ));
    commands.spawn((
        Wing,
        Mesh3d(meshes.add(Cuboid::new(4.5, 0.15, 2.2))),
        MeshMaterial3d(materials.add(Color::srgb(0.85, 0.85, 0.9))),
        Transform::from_xyz(SPAWN.0, ground + START_ABOVE_GROUND + 2.4, SPAWN.1),
    ));
    info!(
        "glider_flight: buff 1029 test over main_world at ({}, {}) ground {ground:.2}",
        SPAWN.0, SPAWN.1
    );
}

/// Runs the real systems on a fixed 60 Hz clock, a few steps per rendered frame.
fn step(world: &mut World) {
    let Some(mut sim) = world.remove_resource::<Sim>() else {
        return;
    };
    if sim.done {
        world.insert_resource(sim);
        return;
    }
    let real_dt = world.resource::<Time>().delta_secs().min(0.1);
    sim.accumulator += real_dt;
    let original = *world.resource::<Time>();
    let mut steps = 0;
    while sim.accumulator >= DT && steps < 8 && !sim.done {
        sim.accumulator -= DT;
        sim.time += DT;
        steps += 1;
        let mut fixed: Time = Time::default();
        fixed.advance_by(Duration::from_secs_f32(DT));
        world.insert_resource(fixed);
        before(world, &mut sim);
        if !sim.done {
            world
                .run_system_once(glide::track_flight)
                .expect("track_flight");
            world
                .run_system_once(glide::request_deploy)
                .expect("request_deploy");
            world
                .run_system_once(glide::control_glide)
                .expect("control_glide");
            world
                .run_system_once(player::control_player)
                .expect("control_player");
            after(world, &mut sim);
        }
        let mut saw_remove = false;
        for NetCommand(command) in world.resource_mut::<Messages<NetCommand>>().drain() {
            let Command::RemoveOwnBuff { buff_index, reason } = command else {
                if let Command::Move(movement) = command {
                    let wire = movement.to_movement(0);
                    match sim.stage {
                        Stage::Climb | Stage::Glide | Stage::LandWait
                            if wire.stance == 1 && wire.actor_flags == 0 && wire.flags & !0x02 == 0 =>
                        {
                            sim.glide_wire = true;
                        }
                        Stage::LandWait
                            if wire.stance == 1 && wire.flags == 6 && wire.actor_flags == 0x14 =>
                        {
                            assert!(
                                saw_remove || sim.remove_sent,
                                "glider_flight: FAILED landing packet before CSRemoveBuff"
                            );
                            sim.landing_wire = true;
                        }
                        Stage::Climb | Stage::Glide | Stage::LandWait => {
                            panic!(
                                "glider_flight: FAILED wire in {:?}: stance {} flags {} actor 0x{:02x}",
                                sim.stage, wire.stance, wire.flags, wire.actor_flags
                            );
                        }
                        _ => {}
                    }
                }
                continue;
            };
            assert_eq!(buff_index, 9, "glider_flight: wrong buff index removed");
            assert_eq!(reason, 9, "glider_flight: glider end must use CSRemoveBuff reason 9");
            assert!(!sim.remove_sent, "glider_flight: duplicate CSRemoveBuff");
            sim.remove_sent = true;
            saw_remove = true;
        }
        if sim.time > BUDGET && !sim.done {
            let phase = world
                .resource::<glide::Gliding>()
                .active
                .map(|a| a.flight.phase);
            let grounded = mover_state(world).is_some_and(|m| m.3);
            panic!(
                "glider_flight: FAILED: 20 s budget spent in stage {:?} (glide {:?}, grounded {grounded}, buff {})",
                sim.stage, phase, sim.buff_on
            );
        }
    }
    world.insert_resource(original);
    world.insert_resource(sim);
}

/// `SCBuffCreated` / `SCBuffRemoved` and the test's stage transitions.
fn before(world: &mut World, sim: &mut Sim) {
    let active = world.resource::<glide::Gliding>().active;
    let mover = mover_state(world);
    match sim.stage {
        Stage::Deploy if sim.time >= 0.5 => {
            add_buff(world);
            sim.buff_on = true;
            sim.stage = Stage::Climb;
            info!("glider_flight: buff 1029 created (deploy)");
        }
        Stage::LandWait => {
            if let Some((pos, _vz, _velocity, grounded)) = mover {
                if grounded && sim.buff_on {
                    remove_buff(world);
                    sim.buff_on = false;
                    info!("glider_flight: touched the ground, buff 1029 removed (remove_on_land)");
                    sim.stage = Stage::SlideWait;
                } else if !grounded && pos.z > sim.ground + 0.5 {
                    // go to just above the floor so the real landing check runs now, not after a 30 m descent
                    set_mover_z(world, sim.ground + 0.5);
                }
            }
        }
        Stage::SlideWait if active.is_some_and(|a| a.flight.phase == Phase::Sliding) => {
            sim.slide_start = sim.time;
            sim.stage = Stage::Sliding;
        }
        Stage::FoldWait => {
            if !sim.buff_on && active.is_none() {
                sim.stage = Stage::FoldStart;
                sim.release = sim.time;
            }
        }
        Stage::FoldStart if sim.time - sim.release > 0.1 => {
            set_mover(world, |mover| {
                mover.pos.z = sim.ground + FOLD_ABOVE_GROUND;
                mover.vz = 0.0;
                mover.grounded = false;
                mover.velocity = Vec3::ZERO;
            });
            add_buff(world);
            sim.buff_on = true;
            sim.stage = Stage::FoldGlide;
            info!("glider_flight: buff 1029 created again for the in-air fold");
        }
        Stage::FoldGlide if active.is_some_and(|a| a.flight.elapsed >= 3.0) => {
            if let Some((pos, vz, _velocity, _grounded)) = mover {
                sim.fold_z = pos.z;
                sim.fold_vz = vz;
            }
            remove_buff(world);
            sim.buff_on = false;
            sim.release = sim.time;
            info!("glider_flight: buff 1029 removed in the air, folding");
            sim.stage = Stage::FoldFall;
        }
        _ => {}
    }
}

/// The assertions after the real systems have run.
fn after(world: &mut World, sim: &mut Sim) {
    let active = world.resource::<glide::Gliding>().active;
    let glide_owns = world.resource::<SendState>().glide_owns;
    let mover = mover_state(world);
    match sim.stage {
        Stage::Climb => {
            if let Some(active) = active {
                if sim.start_z == 0.0 {
                    sim.start_z = mover.map_or(0.0, |m| m.0.z);
                }
                if active.flight.elapsed >= 0.7 {
                    let climb = mover.map_or(f32::NAN, |m| m.0.z) - sim.start_z;
                    assert!(
                        (1.9..=2.3).contains(&climb),
                        "glider_flight: FAILED startup: climbed {climb:.2} m in 0.70 s (data: 3 m/s, about 2.1 m)"
                    );
                    info!("glider_flight: startup climbed {climb:.2} m in 0.70 s");
                    sim.stage = Stage::Glide;
                }
            }
        }
        Stage::Glide => {
            if let Some(active) = active
                && active.flight.elapsed >= 4.0
            {
                let (speed, vz) = (active.flight.speed, active.flight.vz);
                assert!(
                    (speed - 6.0).abs() < 0.05 && (vz + 1.5).abs() < 0.10,
                    "glider_flight: FAILED glide: {speed:.2} m/s forward, {vz:.2} m/s down (data 6 / 1.5)"
                );
                info!("glider_flight: 4 s at {speed:.2} m/s forward, {vz:.2} m/s down");
                sim.stage = Stage::LandWait;
            }
        }
        Stage::SlideWait => {}
        Stage::Sliding => {
            if active.is_none() {
                let took = sim.time - sim.slide_start;
                let stopped = mover.is_some_and(|m| m.2.length() < 0.2);
                assert!(
                    (0.9..=1.3).contains(&took),
                    "glider_flight: FAILED slide: ended after {took:.2} s (data: 1.00 s)"
                );
                assert!(
                    !glide_owns,
                    "glider_flight: FAILED slide: the glider still owns the character"
                );
                assert!(
                    stopped,
                    "glider_flight: FAILED slide: the character is still moving"
                );
                info!("glider_flight: slide ended after {took:.2} s, control_player took over");
                sim.stage = Stage::FoldWait;
                sim.release = sim.time;
            }
        }
        Stage::FoldGlide => {}
        Stage::FoldFall if active.is_none() && !glide_owns => {
            if sim.release == 0.0 {
                sim.release = sim.time;
            }
            if sim.time - sim.release >= 0.5 {
                let vz = mover.map_or(0.0, |m| m.1);
                let z = mover.map_or(sim.fold_z, |m| m.0.z);
                assert!(
                    vz < sim.fold_vz - 3.0 && z < sim.fold_z - 1.0,
                    "glider_flight: FAILED fold: vz {vz:.2} m/s, z {z:.2} (fold {:.2} m, {:.2} m/s)",
                    sim.fold_z,
                    sim.fold_vz
                );
                info!(
                    "glider_flight: fold handed {:.2} m/s to gravity, now {vz:.2} m/s at z {z:.2}",
                    sim.fold_vz
                );
                assert!(
                    sim.glide_wire,
                    "glider_flight: FAILED wire: no captured flight packet"
                );
                assert!(
                    sim.remove_sent && sim.landing_wire,
                    "glider_flight: FAILED landing wire: remove {} / landing {}",
                    sim.remove_sent,
                    sim.landing_wire
                );
                info!(
                    "glider_flight: wire captured: flight stance 1/actor 0; CSRemoveBuff 9; landing 1/6/0x14"
                );
                info!(
                    "glider_flight: PASS: startup climb, glide speeds, landing slide and in-air fold"
                );
                sim.done = true;
                if !sim.keep_open {
                    std::process::exit(0);
                }
            }
        }
        _ => {}
    }
}

/// `(position, vz, velocity, grounded)` of the local player, copied out before other resources are touched.
fn mover_state(world: &mut World) -> Option<(Vec3, f32, Vec3, bool)> {
    let mut query = world.query_filtered::<&Mover, With<LocalPlayer>>();
    query
        .single(world)
        .ok()
        .map(|m| (m.pos, m.vz, m.velocity, m.grounded))
}

fn set_mover(world: &mut World, change: impl FnOnce(&mut Mover)) {
    let mut query = world.query_filtered::<&mut Mover, With<LocalPlayer>>();
    if let Ok(mut mover) = query.single_mut(world) {
        change(&mut mover);
    }
}

fn set_mover_z(world: &mut World, z: f32) {
    set_mover(world, |mover| {
        mover.pos.z = z;
        mover.vz = 0.0;
        mover.velocity = Vec3::ZERO;
    });
}

fn add_buff(world: &mut World) {
    world.resource_mut::<Buffs>().by_unit.insert(
        7,
        vec![BuffEntry {
            buff: aa_net::buff::Buff {
                index: 9,
                buff_id: 1029,
                duration_ms: 60_000,
                ..Default::default()
            },
            received: web_time::Instant::now(),
        }],
    );
}

fn remove_buff(world: &mut World) {
    world.resource_mut::<Buffs>().by_unit.remove(&7);
}

/// Follows the character's transform on the stand-in wing and camera.
#[allow(clippy::type_complexity)]
fn follow(
    pilot: Query<&Transform, (With<Pilot>, Without<Wing>, Without<FollowCam>)>,
    mut wing: Query<&mut Transform, (With<Wing>, Without<Pilot>, Without<FollowCam>)>,
    mut camera: Query<&mut Transform, (With<FollowCam>, Without<Pilot>, Without<Wing>)>,
) {
    let Ok(pilot) = pilot.single() else { return };
    let p = pilot.translation;
    for mut transform in &mut wing {
        transform.translation = p + Vec3::Y * 2.4;
    }
    for mut transform in &mut camera {
        *transform =
            Transform::from_xyz(p.x - 12.0, p.y + 5.0, p.z + 14.0).looking_at(p + Vec3::Y, Vec3::Y);
    }
}
