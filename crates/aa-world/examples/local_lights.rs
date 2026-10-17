//! Original dock Light_116 at night, with the original streetlamp and surrounding world.
//! `cargo run -p aa-world --example local_lights -- --shot shots/aa-world/lights/on.png --shot-after 16`
//! `--off` provides a matching control; `--stream-check --exit-after 18` verifies actual unload.

use aa_world::objects::lights::{LocalLight, LocalLightSettings, LocalLightStats};
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
    config.shadows = !flag("--no-shadows");
    App::new()
        .add_plugins(aa_dev::default_plugins("aa-world native local lights"))
        .add_plugins(aa_dev::DevPlugin)
        .add_plugins(AaWorldPlugin { config })
        .insert_resource(WorldClock { hour: 0.0, ..default() })
        .insert_resource(LocalLightSettings {
            enabled: !flag("--off"),
            ..default()
        })
        .add_systems(Startup, |mut commands: Commands| {
            commands.spawn((
                Camera3d::default(),
                Transform::from_translation(cry_to_bevy(Vec3::new(18961.0, 7745.0, 105.5)))
                    .looking_at(cry_to_bevy(Vec3::new(18967.62, 7734.7, 104.0)), Vec3::Y),
                Projection::Perspective(PerspectiveProjection {
                    fov: 50.0_f32.to_radians(),
                    near: 0.1,
                    far: 20000.0,
                    ..default()
                }),
            ));
        })
        .add_systems(Update, (stats, stream_check))
        .run();
}

fn stats(time: Res<Time>, stats: Res<LocalLightStats>, mut elapsed: Local<f32>) {
    *elapsed += time.delta_secs();
    if *elapsed > 2.0 {
        *elapsed = 0.0;
        info!(
            "local lights: authored={} rendered={} shadowed={} unsupported={}",
            stats.authored, stats.rendered, stats.shadowed, stats.unsupported
        );
    }
}

fn stream_check(
    time: Res<Time>,
    stats: Res<LocalLightStats>,
    slots: Query<&LocalLight>,
    lights: Query<&PointLight, With<LocalLight>>,
    mut camera: Query<&mut Transform, With<Camera3d>>,
    mut stage: Local<u8>,
) {
    if !std::env::args().any(|a| a == "--stream-check") || *stage == 2 || time.elapsed_secs() < 10.0 {
        return;
    }
    if *stage == 0
        && let Ok(mut transform) = camera.single_mut()
    {
        assert!(stats.rendered > 0 && !lights.is_empty());
        assert!(stats.rendered <= 64 && stats.shadowed <= 2);
        info!("local light unload: {} render slots", stats.rendered);
        transform.translation = cry_to_bevy(Vec3::new(-100000.0, -100000.0, 1000.0));
        *stage = 1;
    } else if *stage == 1 && time.elapsed_secs() > 14.0 {
        assert_eq!(stats.rendered, 0);
        assert!(
            slots.is_empty() && lights.is_empty(),
            "all authored local-light entities must unload"
        );
        info!("local light unload: verified no authored or rendered lights remain");
        *stage = 2;
    }
}
