//! Vehicles and ships drawn as units are (body parts and wheels); no server connection.
//! `cargo run -p aa-client --example vehicles -- [--models 657,1008,...] [--gap 14] --shot shots/aa-client/vehicles.png`
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

pub(crate) fn lua_int(n: i64) -> mlua::Value {
    mlua::Value::Integer(n)
}

/// The airship, a tractor, a race car, a carriage, a siege tank, a cart, a rowboat and a galleon.
const MODELS: [u32; 8] = [657, 1008, 1293, 653, 892, 565, 129, 393];

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn models() -> Vec<u32> {
    arg("--models").map(|m| m.split(',').filter_map(|v| v.trim().parse().ok()).collect()).unwrap_or(MODELS.to_vec())
}

fn gap() -> f32 {
    arg("--gap").and_then(|g| g.parse().ok()).unwrap_or(14.0)
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("Vehicles and ships"),
            aa_dev::DevPlugin,
            aa_cry::CryPlugin,
            aa_anim::AnimPlugin,
            aa_world::pipelines::build,
        ))
        .insert_resource(GlobalAmbientLight { color: Color::WHITE, brightness: 300.0, ..default() })
        .add_systems(Startup, (stage, scene))
        .run();
}

fn stage(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    let width = models().len() as f32 * gap();
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(width * 0.5, width * 0.28, width * 0.55).looking_at(Vec3::new(width * 0.5 - gap() * 0.5, 2.0, 0.0), Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight { illuminance: 10000.0, shadow_maps_enabled: false, ..default() },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, -0.5, 0.0)),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(width + 40.0, 80.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.16, 0.19, 0.21))),
        Transform::from_xyz(width * 0.5, 0.0, 0.0),
    ));
}

fn scene(mut commands: Commands, mut cry: aa_cry::scene::CryAssets, clips: Res<aa_anim::ClipLibrary>) {
    let db = ingame::data::Db::open().expect("game database");
    let loader = cry.loader();
    for (i, model) in models().into_iter().enumerate() {
        let Some(look) = db.vehicle_look(model) else {
            warn!("vehicles: model {model} is not a vehicle or ship");
            continue;
        };
        let pieces = ingame::vehicle::prepare(&loader, &look);
        info!("vehicles: model {model} {} -> {} parts", look.normal, pieces.len());
        // turned side-on to the camera
        let transform = Transform::from_xyz(i as f32 * gap(), 0.0, 0.0).with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2));
        ingame::vehicle::spawn(&mut commands, &mut cry, &clips, pieces, transform);
    }
}
