//! Native `entities.xml` static slots: default streetlamp in cell (18,7); `--elevator` in (10,14).
//! `cargo run -p aa-world --example entity_props -- --shot shots/aa-world/entities/streetlamp.png --shot-after 18`
//! `--stream-check --exit-after 18` asserts sector unload removes their actual mesh parts.

use aa_world::objects::{ObjectState, StaticEntityPart};
use aa_world::{AaWorldPlugin, WorldClock, WorldConfig, cry_to_bevy};
use bevy::prelude::*;

fn main() {
    let elevator = std::env::args().any(|a| a == "--elevator");
    let (pos, look) = if elevator {
        (Vec3::new(10455.0, 15100.0, 226.0), Vec3::new(10435.48, 15097.45, 224.0))
    } else {
        (Vec3::new(18961.0, 7745.0, 105.5), Vec3::new(18967.62, 7734.7, 104.0))
    };
    let mut config = WorldConfig::new("main_world");
    config.load_radius = 180.0;
    config.unload_radius = 300.0;
    config.brush_distance = 200.0;
    config.water_distance = 220.0;
    config.tree_distance = 200.0;
    config.vegetation = false;
    App::new()
        .add_plugins(aa_dev::default_plugins("aa-world entity props"))
        .add_plugins(aa_dev::DevPlugin)
        .add_plugins(AaWorldPlugin { config })
        .insert_resource(WorldClock { hour: 10.0, ..default() })
        .add_systems(Startup, move |mut commands: Commands| {
            commands.spawn((
                Camera3d::default(),
                Transform::from_translation(cry_to_bevy(pos)).looking_at(cry_to_bevy(look), Vec3::Y),
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

fn stats(time: Res<Time>, objects: Res<ObjectState>, mut elapsed: Local<f32>) {
    *elapsed += time.delta_secs();
    if *elapsed > 2.0 {
        *elapsed = 0.0;
        info!(
            "entity_props={} total_parts={} pending={}",
            objects.stats.entity_props, objects.stats.entities, objects.stats.models_pending
        );
    }
}

fn stream_check(
    time: Res<Time>,
    objects: Res<ObjectState>,
    parts: Query<&StaticEntityPart>,
    mut camera: Query<&mut Transform, With<Camera3d>>,
    mut stage: Local<u8>,
) {
    if !std::env::args().any(|a| a == "--stream-check") || *stage == 2 || time.elapsed_secs() < 10.0 {
        return;
    }
    if *stage == 0
        && let Ok(mut transform) = camera.single_mut()
    {
        assert!(objects.stats.entity_props > 0, "must draw a native entity prop before unloading");
        assert!(!parts.is_empty());
        info!("entity prop unload: {} drawn parts", objects.stats.entity_props);
        transform.translation = cry_to_bevy(Vec3::new(-100000.0, -100000.0, 1000.0));
        *stage = 1;
    } else if *stage == 1 && time.elapsed_secs() > 14.0 {
        assert_eq!(objects.stats.entity_props, 0);
        assert!(parts.is_empty(), "unloaded static entity mesh parts must despawn");
        info!("entity prop unload: verified no mesh parts remain");
        *stage = 2;
    }
}
