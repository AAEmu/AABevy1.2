//! S14 self-check: the Experimental Glider (quest 1065's reward, item 14677, buff 1029) opens, glides, and lands.
//!
//! The flight is the real [`ingame::glide::Flight`] state machine with the buff-1029 row from the game database
//! (`GlideParams::EXPERIMENTAL` is the same row, checked against `buffs` by the unit test). The scene is a stand-in
//! (a capsule under a wing) because a real character and glider need a server; the numbers are the rule.
//!
//! After 4 s of gliding it asserts the data's normal tier: 6 m/s forward, 1.5 m/s down, and a lower position than the
//! start. Then it runs the stop probe within `land_height + 0.2` of the ground and asserts the 1.0 s slide finishes.
//! Any broken rule panics (a non-zero exit); `--exit-after`/`--shot` are left to `DevPlugin`.
//!
//! `cargo run -p aa-client --example glider_flight -- --mute --offscreen-shot shots/aa-client/glider_flight.png --shot-after 6`
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

use bevy::prelude::*;
use ingame::glide::{Flight, GlideParams, Input, Phase};
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
const GLIDE_SECONDS: f32 = 4.0;
const START_HEIGHT: f32 = 60.0;

#[derive(Resource)]
struct Sim {
    flight: Flight,
    elapsed: f32,
    accumulator: f32,
    x: f32,
    z: f32,
    checked: bool,
    landed: bool,
    done: bool,
    keep_open: bool,
}

#[derive(Component)]
struct Pilot;
#[derive(Component)]
struct Wing;
#[derive(Component)]
struct FollowCam;

fn main() -> AppExit {
    let args = aa_dev::args();
    let keep_open = args.iter().any(|a| {
        matches!(
            a.as_str(),
            "--shot" | "--offscreen-shot" | "--exit-after" | "--record"
        )
    });
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-client: glider flight"),
            aa_dev::DevPlugin,
        ))
        .init_state::<Screen>()
        .insert_state(Screen::InGame)
        .insert_resource(Sim {
            flight: Flight::start(GlideParams::EXPERIMENTAL),
            elapsed: 0.0,
            accumulator: 0.0,
            x: 0.0,
            z: START_HEIGHT,
            checked: false,
            landed: false,
            done: false,
            keep_open,
        })
        .insert_resource(ClearColor(Color::srgb(0.62, 0.74, 0.88)))
        .add_systems(Startup, stage)
        .add_systems(Update, fly)
        .run()
}

fn stage(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(80.0, 80.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.24, 0.35, 0.17),
            perceptual_roughness: 1.0,
            ..default()
        })),
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
        Transform::from_xyz(-14.0, START_HEIGHT + 6.0, 16.0)
            .looking_at(Vec3::new(0.0, START_HEIGHT, 0.0), Vec3::Y),
    ));
    commands.spawn((
        Pilot,
        Mesh3d(meshes.add(Capsule3d::new(0.35, 1.1))),
        MeshMaterial3d(materials.add(Color::srgb(0.35, 0.45, 0.75))),
        Transform::from_xyz(0.0, START_HEIGHT, 0.0),
    ));
    commands.spawn((
        Wing,
        Mesh3d(meshes.add(Cuboid::new(4.5, 0.15, 2.2))),
        MeshMaterial3d(materials.add(Color::srgb(0.85, 0.85, 0.9))),
        Transform::from_xyz(0.0, START_HEIGHT + 2.4, 0.0),
    ));
    info!("glider_flight: Experimental Glider deployed at {START_HEIGHT:.0} m");
}

fn fly(
    time: Res<Time>,
    mut sim: ResMut<Sim>,
    mut pilot: Query<&mut Transform, (With<Pilot>, Without<Wing>, Without<FollowCam>)>,
    mut wing: Query<&mut Transform, (With<Wing>, Without<Pilot>, Without<FollowCam>)>,
    mut camera: Query<&mut Transform, (With<FollowCam>, Without<Pilot>, Without<Wing>)>,
) {
    if sim.done {
        return;
    }
    sim.accumulator += time.delta_secs().min(0.1);
    while sim.accumulator >= DT {
        if sim.done {
            break;
        }
        sim.accumulator -= DT;
        sim.elapsed += DT;
        if !sim.landed {
            let step = sim.flight.step(DT, Input::default(), false);
            sim.z += step.vz * DT;
            sim.x += step.speed * DT;
            if sim.elapsed >= GLIDE_SECONDS && !sim.checked {
                let passed = (step.speed - 6.0).abs() < 0.05
                    && (step.vz + 1.5).abs() < 0.05
                    && sim.z < START_HEIGHT;
                assert!(
                    passed,
                    "glider_flight: FAILED after {:.2} s: speed {:.3} m/s (data 6), sink {:.3} m/s (data 1.5), z {:.2} (start {START_HEIGHT})",
                    sim.elapsed, step.speed, step.vz, sim.z
                );
                info!(
                    "glider_flight: {:.2} s at {:.2} m/s forward, {:.2} m/s down, z {:.2}",
                    sim.elapsed, step.speed, step.vz, sim.z
                );
                sim.checked = true;
                let phase = sim.flight.stop(step.vz, Some(0.0));
                assert_eq!(
                    phase,
                    Phase::Sliding,
                    "the stop probe within land_height + 0.2 must slide"
                );
                sim.landed = true;
                info!(
                    "glider_flight: touched down, sliding {:.2} s",
                    sim.flight.params.sliding_time
                );
            }
        } else {
            let done = sim.flight.slide(DT);
            sim.x += sim.flight.speed * DT;
            if done {
                sim.done = true;
                info!(
                    "glider_flight: PASS: opened, glided 4 s at the data's normal tier and landed"
                );
                if !sim.keep_open {
                    std::process::exit(0);
                }
            }
        }
    }
    for mut transform in &mut pilot {
        transform.translation = Vec3::new(sim.x, sim.z, 0.0);
    }
    for mut transform in &mut wing {
        transform.translation = Vec3::new(sim.x, sim.z + 2.4, 0.0);
    }
    for mut transform in &mut camera {
        *transform = Transform::from_xyz(sim.x - 12.0, sim.z + 5.0, 14.0)
            .looking_at(Vec3::new(sim.x, sim.z + 1.0, 0.0), Vec3::Y);
    }
}
