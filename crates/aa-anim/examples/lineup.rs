//! Every playable race and gender side by side in the login costume, playing the stage idle.
//!
//! ```text
//! cargo run -p aa-anim --example lineup -- --shot shots/aa-anim/lineup.png [--shot-after 12]
//!     [--preset N]       character-create face preset 0..8 (default 0)
//!     [--costume none]   nude instead of the login outfit
//! ```

use aa_anim::{AnimPlugin, CharacterDesc, ClipLibrary, Costume, GameDb, Gender, Race, spawn_character};
use aa_cry::scene::{CryAssets, CryPlugin};
use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins((aa_dev::default_plugins("aa-anim lineup"), aa_dev::DevPlugin, CryPlugin, AnimPlugin))
        .insert_resource(ClearColor(Color::srgb(0.07, 0.075, 0.09)))
        .insert_resource(GlobalAmbientLight { color: Color::srgb(0.75, 0.8, 0.9), brightness: 350.0, ..default() })
        .add_systems(Startup, setup)
        .run();
}

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn setup(mut commands: Commands, mut cry: CryAssets, clips: Res<ClipLibrary>) {
    let preset: usize = arg("--preset").and_then(|v| v.parse().ok()).unwrap_or(0);
    let nude = arg("--costume").is_some_and(|c| c == "none");
    let db = GameDb::open().expect("game database");

    let mut x = -4.0;
    for race in Race::ALL {
        for gender in Gender::ALL {
            let mut desc = CharacterDesc::from_preset(&db, race, gender, preset).expect("preset");
            if nude {
                desc.costume = Costume::None;
            }
            match desc.resolve(&db).and_then(|a| {
                spawn_character(&mut commands, &mut cry, &clips, a, Transform::from_xyz(x, 0.0, 0.0))
            }) {
                Ok(_) => {}
                Err(e) => error!("{race:?} {gender:?}: {e}"),
            }
            x += 1.15;
        }
    }

    // showroom: dark floor disc, key + fill + rim light, camera in front
    let floor = cry.meshes.add(Cylinder::new(7.0, 0.04));
    let floor_mat = cry.materials.add(StandardMaterial {
        base_color: Color::srgb(0.16, 0.17, 0.2),
        perceptual_roughness: 0.9,
        ..default()
    });
    commands.spawn((Mesh3d(floor), MeshMaterial3d(floor_mat), Transform::from_xyz(0.0, -0.02, 0.0)));

    commands.spawn((
        DirectionalLight { illuminance: 9000.0, shadow_maps_enabled: true, color: Color::srgb(1.0, 0.96, 0.9), ..default() },
        Transform::from_xyz(2.0, 5.0, 6.0).looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight { illuminance: 2500.0, color: Color::srgb(0.6, 0.7, 1.0), ..default() },
        Transform::from_xyz(-5.0, 2.5, 4.0).looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight { illuminance: 4000.0, color: Color::srgb(1.0, 0.9, 0.8), ..default() },
        Transform::from_xyz(0.0, 3.0, -6.0).looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y),
    ));
    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection { fov: 28f32.to_radians(), ..default() }),
        Transform::from_xyz(0.0, 1.15, 10.5).looking_at(Vec3::new(0.0, 0.95, 0.0), Vec3::Y),
    ));
}
