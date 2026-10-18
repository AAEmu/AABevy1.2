//! Controlled authored zone-179 curve fixture. Use --hour <game hour> (default 2.7).
//! Fixed original spawn camera + world-origin shift, no objects/vegetation/color retuning.
use aa_world::{AaWorldPlugin, WorldClock, WorldConfig, cry_to_bevy};
use bevy::prelude::*;

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let hour = args.iter().position(|arg| arg == "--hour").and_then(|i| args.get(i + 1))
        .and_then(|value| value.parse::<f32>().ok()).unwrap_or(2.7);
    let mut config = WorldConfig::new("main_world");
    config.load_radius = 450.0;
    config.unload_radius = 600.0;
    config.objects = false;
    config.vegetation = false;
    config.terrain_detail = false;
    config.shadows = false;
    App::new()
        .add_plugins(aa_dev::default_plugins("aa-world native ToD curves"))
        .add_plugins(aa_dev::DevPlugin)
        .add_plugins(AaWorldPlugin { config })
        .insert_resource(WorldClock { hour, zone: Some(179), ..default() })
        .add_systems(Startup, |mut commands: Commands| {
            commands.spawn((
                Camera3d::default(),
                Transform::from_translation(cry_to_bevy(Vec3::new(15579.49316, 15385.85449, 127.979904)))
                    .looking_to(cry_to_bevy(Vec3::new(-0.362598211, -0.931945682, 0.0)), Vec3::Y),
                Projection::Perspective(PerspectiveProjection { fov: 42.75_f32.to_radians(), far: 20000.0, ..default() }),
            ));
        })
        .run();
}
