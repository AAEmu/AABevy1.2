//! Original star catalogue and native star-sprite math above the main-world landscape.
//! `cargo run -p aa-world --example stars -- --shot shots/aa-world/night/stars.png --shot-after 14`
//! `--off` is a same-camera control, `--day` evaluates the authored daytime star intensity.

use aa_world::night_sky::{StarSettings, StarStats};
use aa_world::{AaWorldPlugin, WorldClock, WorldConfig, cry_to_bevy};
use bevy::prelude::*;

fn main() {
    let flag = |name| std::env::args().any(|a| a == name);
    let mut config = WorldConfig::new("main_world");
    config.load_radius = 180.0;
    config.unload_radius = 300.0;
    config.brush_distance = 200.0;
    config.water_distance = 220.0;
    config.tree_distance = 200.0;
    config.vegetation = false;
    config.shadows = false;
    App::new()
        .add_plugins(aa_dev::default_plugins("aa-world original stars"))
        .add_plugins(aa_dev::DevPlugin)
        .add_plugins(AaWorldPlugin { config })
        .insert_resource(WorldClock {
            hour: if flag("--day") { 12.0 } else { 0.0 },
            ..default()
        })
        .insert_resource(StarSettings {
            enabled: !flag("--off"),
        })
        .add_systems(Startup, |mut commands: Commands| {
            commands.spawn((
                Camera3d::default(),
                Transform::from_translation(cry_to_bevy(Vec3::new(15579.5, 15385.85, 137.0)))
                    .looking_to(cry_to_bevy(Vec3::new(0.0, -0.94, 0.342)), Vec3::Y),
                Projection::Perspective(PerspectiveProjection {
                    fov: 65.0_f32.to_radians(),
                    near: 0.1,
                    far: 20000.0,
                    ..default()
                }),
            ));
        })
        .add_systems(Update, stats)
        .run();
}

fn stats(time: Res<Time>, stars: Res<StarStats>, mut elapsed: Local<f32>) {
    *elapsed += time.delta_secs();
    if *elapsed > 2.0 {
        *elapsed = 0.0;
        info!(
            "star catalogue={} authored intensity={:.4}",
            stars.catalogue, stars.intensity
        );
    }
}
