//! Original placed instrument prefabs and retained bond helpers; no server connection.
#![allow(dead_code)]
#[path = "../src/hud/mod.rs"]
mod hud;
#[path = "../src/ingame/mod.rs"]
mod ingame;
#[path = "../src/login/mod.rs"]
mod login;
#[path = "../src/script/mod.rs"]
mod script;
#[path = "../src/settings.rs"]
mod settings;
#[path = "../src/sound/mod.rs"]
mod sound;
use bevy::prelude::*;
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
fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("Placed instrument prefabs"),
            aa_dev::DevPlugin,
            aa_cry::CryPlugin,
            aa_world::pipelines::build,
        ))
        .insert_resource(GlobalAmbientLight {
            color: Color::WHITE,
            brightness: 200.0,
            ..default()
        })
        .add_systems(Startup, (stage, scene))
        .add_systems(Update, report)
        .run();
}

fn report(
    time: Res<Time<Real>>,
    status: Res<aa_world::pipelines::PipelineStatus>,
    meshes: Query<&ViewVisibility, With<Mesh3d>>,
    mut done: Local<bool>,
) {
    if !*done && time.elapsed_secs() > 10.0 {
        *done = true;
        info!(
            "instrument-control: mesh entities {} visible {} pipelines {} waiting {}",
            meshes.iter().count(),
            meshes.iter().filter(|v| v.get()).count(),
            status.total(),
            status.waiting()
        );
    }
}
fn stage(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(8.0, 6.0, 11.0).looking_at(Vec3::new(0.0, 0.8, 0.0), Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 10000.0,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, -0.5, 0.0)),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(18.0, 14.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.16, 0.19, 0.21))),
    ));
}

fn scene(mut commands: Commands, mut cry: aa_cry::scene::CryAssets) {
    let before = std::env::args().any(|a| a == "--before");
    let loader = cry.loader();
    for (x, name) in [(-4.0, "piano_a"), (0.0, "piano_b"), (4.0, "percussion_a")] {
        let reference = format!("prefab://prefabs/interaction_c.xml/furniture.{name}");
        // The former first-object_Model path finds none of these brush-only layouts.
        if before {
            info!("instrument-control: {name} omitted by former object_Model-only lookup");
            continue;
        }
        let prepared = ingame::doodad_model::prepare(&loader, &reference)
            .expect("complete authored instrument prefab");
        let spawned = ingame::doodad_model::spawn(&mut commands, &mut cry, prepared);
        commands
            .entity(spawned.entity)
            .insert(Transform::from_xyz(x, 0.0, 0.0));
        let helper = spawned.helpers.get("bond0").expect("authored chair helper");
        info!(
            "instrument-control: {name} bond0 Cry {:?}",
            helper.translation
        );
    }
}
