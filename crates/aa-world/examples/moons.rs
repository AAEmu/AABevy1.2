//! Native zone-133 moon discs/coronas above main-world hills. `--off` and `--day` are controls.
//! `--natural` uses the camera's own zone instead of the visible-disc fixture's authored zone.
//! `cargo run -p aa-world --example moons -- --shot shots/aa-world/night/moons.png --shot-after 14`

use aa_world::moons::{MoonSettings, MoonStats};
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
        .add_plugins(aa_dev::default_plugins("aa-world original moons"))
        .add_plugins(aa_dev::DevPlugin)
        .add_plugins(AaWorldPlugin { config })
        .insert_resource(WorldClock {
            hour: if flag("--day") { 12.0 } else { 0.0 },
            zone: if flag("--natural") { None } else { Some(133) },
            ..default()
        })
        .insert_resource(MoonSettings {
            enabled: !flag("--off"),
        })
        .add_systems(Startup, |mut commands: Commands| {
            let (bearing, elevation) = (150.0_f32.to_radians(), 35.0_f32.to_radians());
            let look = Vec3::new(
                bearing.sin() * elevation.cos(),
                bearing.cos() * elevation.cos(),
                elevation.sin(),
            );
            commands.spawn((
                Camera3d::default(),
                Transform::from_translation(cry_to_bevy(Vec3::new(15579.5, 15385.85, 137.0)))
                    .looking_to(cry_to_bevy(look), Vec3::Y),
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

fn stats(
    time: Res<Time>,
    moons: Res<MoonStats>,
    env: Res<aa_world::sky::Environment>,
    mut elapsed: Local<f32>,
) {
    *elapsed += time.delta_secs();
    if *elapsed > 2.0 {
        *elapsed = 0.0;
        info!(
            "moons authored={} rendered={} zone={:?} colors={:?}",
            moons.authored,
            moons.rendered,
            env.zone,
            env.sample
                .as_ref()
                .map(|s| [s.moons[0].color, s.moons[1].color])
        );
    }
}
