//! Native zone-179 / zone-210 ToD blending at a real Solzreed / Silent Sea sector border.
//! --offset <metres> selects a fixed border position; --motion-check crosses it over 12s.
//! --zone179 / --zone210 force single-zone controls. --unload-check disables the environment.
//! --layer-check switches the authored relic sky at 5s. --fade-check verifies shared sky/cloud alpha.
use std::sync::Arc;

use aa_world::sky::{Environment, update_environment};
use aa_world::sky_clouds::CloudMaterial;
use aa_world::sky_clouds::CloudSprite;
use aa_world::sky_material::SkyMaterial;
use aa_world::sky_transition::SkySelection;
use aa_world::stream::WorldInfo;
use aa_world::{AaWorldPlugin, World, WorldClock, WorldConfig, WorldFocus, cry_to_bevy};
use bevy::prelude::*;

#[derive(Component)]
struct BorderCamera;

fn flag(name: &str) -> bool {
    std::env::args().any(|arg| arg == name)
}

fn offset() -> f32 {
    let args: Vec<_> = std::env::args().collect();
    args.iter()
        .position(|arg| arg == "--offset")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0)
}

fn position(offset: f32) -> Vec3 {
    cry_to_bevy(Vec3::new(15712.0, 15808.0 + offset, 200.0))
}

fn main() {
    let mut world = World::load("main_world").expect("original world");
    world
        .load_sector_zones()
        .expect("original sector ownership");
    let mut config = WorldConfig::new("main_world");
    config.load_radius = 180.0;
    config.unload_radius = 400.0;
    config.objects = false;
    config.vegetation = false;
    config.terrain_detail = false;
    config.shadows = false;
    App::new()
        .add_plugins(aa_dev::default_plugins(
            "aa-world native environment border",
        ))
        .add_plugins(aa_dev::DevPlugin)
        .add_plugins(AaWorldPlugin { config })
        .insert_resource(WorldInfo(Arc::new(world)))
        .insert_resource(WorldClock {
            hour: 19.166172,
            zone: if flag("--zone179") {
                Some(179)
            } else if flag("--zone210") {
                Some(210)
            } else {
                None
            },
            ..default()
        })
        .add_systems(Startup, |mut commands: Commands| {
            commands.spawn((
                Camera3d::default(),
                BorderCamera,
                WorldFocus,
                Transform::from_translation(position(if flag("--motion-check") {
                    -120.0
                } else {
                    offset()
                }))
                .looking_to(cry_to_bevy(Vec3::new(0.0, 1.0, 0.12)).normalize(), Vec3::Y),
                Projection::Perspective(PerspectiveProjection {
                    fov: 55.0_f32.to_radians(),
                    far: 20000.0,
                    ..default()
                }),
            ));
        })
        .add_systems(Update, advance.before(update_environment))
        .add_systems(PostUpdate, verify)
        .run();
}

fn advance(
    time: Res<Time>,
    mut cameras: Query<(&mut Transform, &mut GlobalTransform), With<BorderCamera>>,
    mut config: ResMut<WorldConfig>,
    mut clock: ResMut<WorldClock>,
) {
    if flag("--motion-check") {
        let travelled = ((time.elapsed_secs() - 2.0) / 12.0).clamp(0.0, 1.0);
        for (mut camera, mut global) in &mut cameras {
            camera.translation = position(-120.0 + travelled * 240.0);
            // The production environment reads GlobalTransform. Keep this scripted fixture's
            // sample pose current in Update, before ordinary PostUpdate propagation.
            *global = GlobalTransform::from(*camera);
        }
    }
    if flag("--unload-check") && time.elapsed_secs() > 10.0 {
        config.environment = false;
    }
    if flag("--layer-check") && time.elapsed_secs() > 5.0 {
        clock.layer = 1;
    }
}

fn verify(
    time: Res<Time>,
    config: Res<WorldConfig>,
    info: Res<WorldInfo>,
    env: Res<Environment>,
    clock: Res<WorldClock>,
    selection: Res<SkySelection>,
    sky_assets: Res<aa_world::sky::SkyAssets>,
    sky_materials: Res<Assets<SkyMaterial>>,
    cloud_materials: Res<Assets<CloudMaterial>>,
    cameras: Query<&Transform, With<BorderCamera>>,
    clouds: Query<(&CloudSprite, &MeshMaterial3d<CloudMaterial>)>,
    mut previous: Local<Option<(f32, Vec3, Vec3)>>,
    mut crossed: Local<bool>,
    mut logged: Local<bool>,
    mut unloaded: Local<bool>,
    mut previous_sky: Local<Option<(Option<u16>, u32, f32)>>,
) {
    let Some(camera) = cameras.iter().next() else {
        return;
    };
    if flag("--unload-check") && time.elapsed_secs() > 14.0 && !*unloaded {
        assert!(!config.environment && env.sample.is_none());
        assert_eq!(clouds.iter().count(), 0);
        assert_eq!((selection.zone, selection.opacity), (None, 0.0));
        assert!(
            sky_materials
                .get(&sky_assets.dome_material)
                .unwrap()
                .box_12
                .is_none()
        );
        *unloaded = true;
        info!("environment unload: no active sample or cloud entities");
    }
    let Some(sample) = &env.sample else { return };
    let pos = aa_world::bevy_to_cry(camera.translation);
    let weights = info.0.environment_weights(pos);
    if flag("--fade-check") {
        let weights = if let Some(zone) = clock.zone {
            vec![(zone, 1.0)]
        } else {
            weights.clone()
        };
        let total: f32 = weights.iter().map(|(_, weight)| weight).sum();
        let strongest = weights
            .iter()
            .map(|(_, weight)| *weight)
            .fold(0.0, f32::max);
        let spatial = (2.0 * strongest - total).clamp(0.0, 1.0);
        assert!(selection.opacity <= spatial + 1e-5);
        if let Some((zone, layer, opacity)) = *previous_sky {
            if zone != selection.zone || layer != selection.layer {
                assert!(
                    selection.opacity < 1e-5,
                    "native asset changes occur at zero opacity"
                );
            } else {
                // Fixed poses isolate the native two-second real frame-time fade.
                if !flag("--motion-check") {
                    assert!(
                        (selection.opacity - opacity).abs() <= time.delta_secs() * 0.501 + 1e-5
                    );
                }
            }
        }
        *previous_sky = Some((selection.zone, selection.layer, selection.opacity));
        for (_, material) in &clouds {
            let cloud = cloud_materials.get(&material.0).unwrap();
            assert!((cloud.params.shade_sky.w - selection.opacity).abs() < 1e-5);
        }
        if selection.opacity == 0.0 {
            assert_eq!(
                sky_materials
                    .get(&sky_assets.dome_material)
                    .unwrap()
                    .params
                    .box_sky
                    .w,
                0.0
            );
        }
        if time.elapsed_secs() > 11.0 && flag("--layer-check") {
            assert_eq!(selection.layer, 1);
            assert!((selection.opacity - spatial).abs() < 1e-4);
        }
    }
    if time.elapsed_secs() > 5.0 && !*logged {
        info!(
            "environment border: at {pos:?}, sector {:?}, weights {weights:?}, sky {:?}, sun {:?}, fog {:?}",
            env.zone,
            (selection.zone, selection.layer, selection.opacity),
            sample.sun,
            sample.fog
        );
        *logged = true;
    }
    if !flag("--motion-check") {
        return;
    }
    if let Some((y, sun, fog)) = *previous {
        let distance = (pos.y - y).abs();
        // The source weights are Lipschitz over distance. A sector ownership switch must
        // not produce the old full-zone color jump, even on a low-FPS frame.
        if distance < 2.0 {
            assert!((sample.sun - sun).length() < 0.1);
            assert!((sample.fog - fog).length() < 0.1);
        }
        if y < 15808.0 && pos.y >= 15808.0 {
            *crossed = true;
            info!(
                "environment border crossing: {distance:.6}m, sun delta {:.6}, fog delta {:.6}",
                (sample.sun - sun).length(),
                (sample.fog - fog).length()
            );
        }
    }
    *previous = Some((pos.y, sample.sun, sample.fog));
    if time.elapsed_secs() > 15.0 {
        assert!(*crossed, "scripted camera must cross the real zone border");
    }
}
