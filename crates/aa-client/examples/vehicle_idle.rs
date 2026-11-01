//! The Farm Wagon's engine idle (`X2::PrefabUnitModel::UpdateIdleAnim`, `FUN_390d6690`): the body's
//! `fist_ba_relaxed_idle` loops, so `Bone_engine01` moves while the cart stands still.
//!
//! It exits non-zero when the bone has not moved between 1 s and 2 s (the vehicle idle is not
//! driving the body), and it needs the real pak and database:
//!
//! `cargo run -p aa-client --example vehicle_idle -- --headless --mute --exit-after 4`
//! `cargo run -p aa-client --example vehicle_idle -- --offscreen-shot shots/aa-client/vehicle_idle.png --shot-after 3`
#![allow(dead_code)]
#[path = "../src/graphics.rs"]
mod graphics;
#[path = "../src/hud/mod.rs"]
mod hud;
#[path = "../src/ingame/mod.rs"]
mod ingame;
#[path = "../src/logging.rs"]
mod logging;
#[path = "../src/login/mod.rs"]
mod login;
#[path = "../src/priority.rs"]
mod priority;
#[path = "../src/script/mod.rs"]
mod script;
#[path = "../src/settings.rs"]
mod settings;
#[path = "../src/sound/mod.rs"]
mod sound;
#[path = "../src/task.rs"]
mod task;
#[path = "../src/lua_value.rs"]
mod lua_value;
use lua_value::lua_int;
use bevy::prelude::*;

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

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn model() -> u32 {
    arg("--model").and_then(|m| m.parse().ok()).unwrap_or(1008)
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("Vehicle idle"),
            aa_dev::DevPlugin,
            aa_cry::CryPlugin,
            aa_anim::AnimPlugin,
            aa_world::pipelines::build,
            ingame::vehicle::VehiclePlugin,
        ))
        .insert_resource(GlobalAmbientLight { color: Color::WHITE, brightness: 300.0, ..default() })
        .init_resource::<IdleCheck>()
        .add_systems(Startup, scene)
        .add_systems(Update, check_idle)
        .run();
}

fn scene(mut commands: Commands, mut cry: aa_cry::scene::CryAssets, clips: Res<aa_anim::ClipLibrary>) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(5.5, 2.6, 6.5).looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight { illuminance: 10000.0, shadow_maps_enabled: false, ..default() },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, -0.5, 0.0)),
    ));
    let model = model();
    let db = ingame::data::Db::open().expect("game database (AA_DB)");
    let look = db.vehicle_look(model).unwrap_or_else(|| panic!("model {model} is not a vehicle or ship"));
    let loader = cry.loader();
    let pieces = ingame::vehicle::prepare(&loader, &look);
    assert!(!pieces.is_empty(), "no part of model {model} could be loaded");
    ingame::vehicle::spawn(&mut commands, &mut cry, &clips, pieces, Transform::default());
}

/// The engine bone at 1 s and at 2 s: 1.2's idle loop has to move it with no driver and no speed.
#[derive(Resource, Default)]
struct IdleCheck {
    first: Option<Quat>,
    done: bool,
}

fn check_idle(time: Res<Time>, mut check: ResMut<IdleCheck>, bones: Query<(&Name, &Transform)>) {
    if check.done {
        return;
    }
    let t = time.elapsed_secs();
    let bone = bones.iter().find(|(name, _)| name.as_str() == "Bone_engine01").map(|(_, transform)| transform.rotation);
    match check.first {
        None if t >= 1.0 => match bone {
            Some(rotation) => {
                info!("vehicle_idle: Bone_engine01 at {t:.1}s: {rotation:?}");
                check.first = Some(rotation);
            }
            None => fail(&mut check, "no Bone_engine01: the wagon body was not rigged".into()),
        },
        Some(first) if t >= 2.0 => match bone {
            Some(rotation) => {
                let moved = first.angle_between(rotation);
                if moved <= 0.02 {
                    fail(&mut check, format!("Bone_engine01 did not move ({moved:.4} rad): the engine idle is not playing"));
                } else {
                    info!("vehicle_idle: the engine moved {moved:.3} rad between 1.0 s and {t:.1} s");
                    check.done = true;
                }
            }
            None => fail(&mut check, "Bone_engine01 disappeared".into()),
        },
        _ => {}
    }
}

fn fail(check: &mut IdleCheck, message: String) {
    error!("vehicle_idle: {message}");
    check.done = true;
    // aa-dev's `--exit-after` exits 0 later; a broken self-check must fail the process at once
    std::process::exit(1);
}
