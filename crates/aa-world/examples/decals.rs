//! Original static decals in cell (18,7): terrain dirt and projections onto village props.
//! `cargo run -p aa-world --example decals -- --shot shots/aa-world/decals/village.png --shot-after 18`
//! `--straw` visits cell (14,11); `--planar` visits the Ferre decals; `--stream-check --exit-after 18`
//! verifies that unloading removes every decal entity. Uses only the original placement/assets.
//! `--siege` visits the 81 authored deferred decals; `--glass` adds a near translucent test pane.

use aa_world::{AaWorldPlugin, WorldClock, WorldConfig, cry_to_bevy};
use bevy::prelude::*;

fn main() {
    let flag = |name: &str| std::env::args().any(|a| a == name);
    let (mut pos, mut look) = if flag("--siege") {
        let objects = aa_world::placement::load_cell_objects("main_world", 2, 29);
        let decal = objects
            .decals
            .iter()
            .find(|d| d.deferred)
            .expect("original siegefield decal");
        let center = aa_world::coords::cell_origin(2, 29) + decal.pos;
        (center + Vec3::new(10.0, -14.0, 12.0), center)
    } else if flag("--straw") {
        (Vec3::new(14575.0, 11385.0, 150.0), Vec3::new(14575.5, 11391.0, 134.7))
    } else if flag("--planar") {
        let objects = aa_world::placement::load_cell_objects("main_world", 23, 10);
        let decal = objects
            .decals
            .iter()
            .find(|d| d.projection == 0 && !d.deferred)
            .expect("original planar decal");
        let center = aa_world::coords::cell_origin(23, 10) + decal.pos;
        (
            center + decal.basis.z_axis.normalize() * (decal.radius * 0.8) - decal.basis.y_axis * (decal.radius * 0.25),
            center,
        )
    } else {
        (Vec3::new(19088.0, 8148.0, 224.0), Vec3::new(19064.0, 8172.0, 203.5))
    };
    let vector = |name: &str| -> Option<Vec3> {
        let args: Vec<String> = std::env::args().collect();
        let values: Vec<f32> = args
            .get(args.iter().position(|a| a == name)? + 1)?
            .split(',')
            .filter_map(|v| v.parse().ok())
            .collect();
        (values.len() == 3).then(|| Vec3::new(values[0], values[1], values[2]))
    };
    if let Some(value) = vector("--pos") {
        pos = value;
    }
    if let Some(value) = vector("--look") {
        look = value;
    }
    let mut config = WorldConfig::new("main_world");
    let glass = flag("--glass");
    config.load_radius = 300.0;
    config.unload_radius = 450.0;
    config.brush_distance = 350.0;
    config.tree_distance = 350.0;
    config.vegetation = !flag("--no-vegetation");
    App::new()
        .add_plugins(aa_dev::default_plugins("aa-world decals"))
        .add_plugins(aa_dev::DevPlugin)
        .add_plugins(AaWorldPlugin { config })
        .insert_resource(WorldClock {
            hour: 10.0,
            ..default()
        })
        .add_systems(
            Startup,
            move |mut commands: Commands,
                  mut meshes: ResMut<Assets<Mesh>>,
                  mut materials: ResMut<Assets<StandardMaterial>>| {
                let transform = Transform::from_translation(cry_to_bevy(pos)).looking_at(cry_to_bevy(look), Vec3::Y);
                if glass {
                    commands.spawn((
                        Mesh3d(meshes.add(Rectangle::new(5.0, 3.0))),
                        MeshMaterial3d(materials.add(StandardMaterial {
                            base_color: Color::linear_rgba(0.02, 0.2, 0.01, 0.75),
                            alpha_mode: AlphaMode::Blend,
                            unlit: true,
                            cull_mode: None,
                            ..default()
                        })),
                        Transform::from_translation(cry_to_bevy(pos).lerp(cry_to_bevy(look), 0.25))
                            .with_rotation(transform.rotation),
                    ));
                }
                commands.spawn((
                    Camera3d::default(),
                    transform,
                    Projection::Perspective(PerspectiveProjection {
                        fov: if std::env::args().any(|a| a == "--planar") {
                            75.0_f32
                        } else {
                            50.0_f32
                        }
                        .to_radians(),
                        near: 0.3,
                        far: 20000.0,
                        ..default()
                    }),
                ));
            },
        )
        .add_systems(Update, (stats, stream_check))
        .run();
}

fn stats(time: Res<Time>, objects: Res<aa_world::objects::ObjectState>, names: Query<&Name>, mut elapsed: Local<f32>) {
    *elapsed += time.delta_secs();
    if *elapsed > 2.0 {
        *elapsed = 0.0;
        info!(
            "decals={} deferred={} roads={} object_parts={} models_pending={}",
            objects.stats.decals,
            names
                .iter()
                .filter(|n| n.as_str() == "Deferred terrain/object decal")
                .count(),
            objects.stats.roads,
            objects.stats.entities,
            objects.stats.models_pending
        );
    }
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
    if *stage == 0
        && let Ok(mut transform) = camera.single_mut()
    {
        assert!(objects.stats.decals > 0, "load decals before leaving");
        transform.translation = cry_to_bevy(Vec3::new(-100000.0, -100000.0, 1000.0));
        *stage = 1;
        info!("decal stream check: moved outside world");
    } else if *stage == 1 && time.elapsed_secs() > 14.0 {
        assert_eq!(objects.stats.decals, 0);
        assert!(
            !names
                .iter()
                .any(|n| matches!(n.as_str(), "Terrain/object decal" | "Deferred terrain/object decal"))
        );
        info!("decal stream check: verified no decal entities remain");
        *stage = 2;
    }
}
