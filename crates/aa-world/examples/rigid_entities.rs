//! Authored Mari windmill: original CGA controller, entities.xml placement/loop/Speed=.1.
//! `cargo run -p aa-world --example rigid_entities -- --shot shots/aa-world/rigid/windmill.png --shot-after 14`
//! `--freeze` keeps the native first pose; `--stream-check --exit-after 18` verifies unload.

use aa_world::objects::ObjectState;
use aa_world::objects::rigid::{RigidEntity, RigidPart, RigidSettings};
use aa_world::{AaWorldPlugin, WorldClock, WorldConfig, cry_to_bevy};
use bevy::prelude::*;

fn main() {
    let freeze = std::env::args().any(|a| a == "--freeze");
    let mut config = WorldConfig::new("main_world");
    config.load_radius = 180.0;
    config.unload_radius = 300.0;
    config.brush_distance = 220.0;
    config.water_distance = 220.0;
    config.tree_distance = 220.0;
    config.vegetation = false;
    App::new()
        .add_plugins(aa_dev::default_plugins("aa-world rigid entity animation"))
        .add_plugins(aa_dev::DevPlugin)
        .add_plugins(AaWorldPlugin { config })
        .insert_resource(WorldClock {
            hour: 10.0,
            ..default()
        })
        .insert_resource(RigidSettings { enabled: !freeze })
        .add_systems(Startup, |mut commands: Commands| {
            commands.spawn((
                Camera3d::default(),
                Transform::from_translation(cry_to_bevy(Vec3::new(11610.0, 11992.0, 122.0)))
                    .looking_at(cry_to_bevy(Vec3::new(11648.0, 11998.0, 121.0)), Vec3::Y),
                Projection::Perspective(PerspectiveProjection {
                    fov: 55.0_f32.to_radians(),
                    near: 0.1,
                    far: 20000.0,
                    ..default()
                }),
            ));
        })
        .add_systems(Update, (stats, stream_check))
        .run();
}

fn stats(
    time: Res<Time>,
    objects: Res<ObjectState>,
    roots: Query<&RigidEntity>,
    mut elapsed: Local<f32>,
) {
    *elapsed += time.delta_secs();
    if *elapsed > 2.0 {
        *elapsed = 0.0;
        info!(
            "rigid instances={} parts={} total_parts={} pending={}",
            roots.iter().count(),
            objects.stats.animated_props,
            objects.stats.entities,
            objects.stats.models_pending
        );
    }
}

fn stream_check(
    time: Res<Time>,
    objects: Res<ObjectState>,
    roots: Query<&RigidEntity>,
    parts: Query<&RigidPart>,
    mut camera: Query<&mut Transform, With<Camera3d>>,
    mut stage: Local<u8>,
) {
    if !std::env::args().any(|a| a == "--stream-check") || *stage == 2 || time.elapsed_secs() < 10.0
    {
        return;
    }
    if *stage == 0
        && let Ok(mut transform) = camera.single_mut()
    {
        assert!(
            objects.stats.animated_props > 0 && !roots.is_empty() && !parts.is_empty(),
            "must draw a native playing CGA before unloading"
        );
        info!("rigid unload: {} parts", objects.stats.animated_props);
        transform.translation = cry_to_bevy(Vec3::new(-100000.0, -100000.0, 1000.0));
        *stage = 1;
    } else if *stage == 1 && time.elapsed_secs() > 14.0 {
        assert_eq!(objects.stats.animated_props, 0);
        assert!(
            roots.is_empty() && parts.is_empty(),
            "unloaded CGA roots / descendants must despawn"
        );
        info!("rigid unload: verified no instances remain");
        *stage = 2;
    }
}
