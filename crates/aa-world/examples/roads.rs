//! Original road inspection. Defaults to an elevated view of the roads in cell (14,11).
//! `cargo run -p aa-world --example roads -- --shot shots/aa-world/roads/cell14_11.png --shot-after 18`
//! `--reference` uses the Nuian spawn reference camera/time; `--no-vegetation` exposes the banks.
//! `--memoria` inspects the narrow forest road; `--stream-check --exit-after 18` checks despawning.

use aa_world::{AaWorldPlugin, WorldClock, WorldConfig, cry_to_bevy};
use bevy::prelude::*;

fn main() {
    let reference = std::env::args().any(|a| a == "--reference");
    let memoria = std::env::args().any(|a| a == "--memoria");
    let mut config = WorldConfig::new("main_world");
    config.load_radius = 450.0;
    config.unload_radius = 650.0;
    config.brush_distance = 450.0;
    config.tree_distance = 450.0;
    config.vegetation = !std::env::args().any(|a| a == "--no-vegetation");
    App::new()
        .add_plugins(aa_dev::default_plugins("aa-world roads"))
        .add_plugins(aa_dev::DevPlugin)
        .add_plugins(AaWorldPlugin { config })
        .insert_resource(WorldClock {
            hour: if reference { 19.166172 } else { 10.0 },
            ..default()
        })
        .add_systems(Startup, move |mut commands: Commands| {
            let transform = if reference {
                Transform::from_translation(cry_to_bevy(Vec3::new(15579.493, 15385.854, 127.979904)))
                    .looking_to(cry_to_bevy(Vec3::new(-0.3625982, -0.9319457, 0.0)), Vec3::Y)
            } else if memoria {
                Transform::from_translation(cry_to_bevy(Vec3::new(10490.0, 15160.0, 255.0)))
                    .looking_at(cry_to_bevy(Vec3::new(10447.0, 15130.0, 220.0)), Vec3::Y)
            } else {
                Transform::from_translation(cry_to_bevy(Vec3::new(14780.0, 11520.0, 161.0)))
                    .looking_at(cry_to_bevy(Vec3::new(14710.0, 11480.0, 117.0)), Vec3::Y)
            };
            commands.spawn((
                Camera3d::default(),
                transform,
                Projection::Perspective(PerspectiveProjection {
                    fov: if reference { 42.75_f32 } else { 60.0_f32 }.to_radians(),
                    near: 0.3,
                    far: 20000.0,
                    ..default()
                }),
            ));
        })
        .add_systems(Update, (stats, stream_check))
        .run();
}

fn stream_check(
    time: Res<Time>,
    objects: Res<aa_world::objects::ObjectState>,
    names: Query<&Name>,
    mut camera: Query<&mut Transform, With<Camera3d>>,
    mut stage: Local<u8>,
) {
    if !std::env::args().any(|a| a == "--stream-check") || *stage == 2 || time.elapsed_secs() < 10.0 {
        return;
    }
    if *stage == 0 && let Ok(mut transform) = camera.single_mut() {
        assert!(objects.stats.roads > 0, "stream check must load roads before leaving");
        transform.translation = cry_to_bevy(Vec3::new(-100000.0, -100000.0, 1000.0));
        *stage = 1;
        info!("road stream check: moved outside world; road count must fall to zero");
    } else if *stage == 1 && time.elapsed_secs() > 14.0 {
        assert_eq!(objects.stats.roads, 0, "unloaded placement cells must release roads");
        assert!(!names.iter().any(|n| n.as_str() == "Terrain road"), "road entities must despawn");
        info!("road stream check: verified no road entities remain");
        *stage = 2;
    }
}

fn stats(time: Res<Time>, objects: Res<aa_world::objects::ObjectState>, mut elapsed: Local<f32>) {
    *elapsed += time.delta_secs();
    if *elapsed > 2.0 {
        *elapsed = 0.0;
        info!(
            "roads={} object_parts={} models_pending={}",
            objects.stats.roads, objects.stats.entities, objects.stats.models_pending
        );
    }
}
