//! Authored zone-179 distant clouds, native motion / far depth. --night / --off are controls.
//! --glass --behind-origin checks translucent and opaque foreground ordering.
//! --motion-check asserts the real authored speed over frame time; --unload-check retires clouds.
//! --reference uses reference/ingame/spawn/camera.json's pose/FOV/hour for sky comparison.
use aa_world::sky_clouds::{CloudSettings, CloudSprite};
use aa_world::{AaWorldPlugin, WorldClock, WorldConfig, cry_to_bevy};
use bevy::prelude::*;

fn main() {
    let flag = |name| std::env::args().any(|a| a == name);
    let (glass, behind, reference) = (flag("--glass"), flag("--behind-origin"), flag("--reference"));
    let mut config = WorldConfig::new("main_world");
    config.load_radius = 180.0;
    config.unload_radius = 300.0;
    config.objects = false;
    config.vegetation = false;
    config.terrain_detail = false;
    config.shadows = false;
    App::new()
        .add_plugins(aa_dev::default_plugins("aa-world native clouds"))
        .add_plugins(aa_dev::DevPlugin)
        .add_plugins(AaWorldPlugin { config })
        .insert_resource(WorldClock {
            hour: if flag("--night") {
                0.0
            } else if reference {
                19.166172
            } else {
                10.0
            },
            zone: Some(179),
            ..default()
        })
        .insert_resource(CloudSettings {
            enabled: !flag("--off"),
        })
        .add_systems(
            Startup,
            move |mut commands: Commands,
                  mut meshes: ResMut<Assets<Mesh>>,
                  mut materials: ResMut<Assets<StandardMaterial>>| {
                let at = cry_to_bevy(if behind {
                    Vec3::new(30000.0, 10000.0, 800.0)
                } else if reference {
                    Vec3::new(4315.49316, 4121.85449, 127.979904)
                } else {
                    Vec3::new(11764.0, 11541.0, 150.0)
                });
                // The native (0,-2000,400) direction of the original bearing-180 cloud.
                let look = cry_to_bevy(if reference {
                    Vec3::new(-0.362598211, -0.931945682, 0.0)
                } else {
                    Vec3::new(0.0, -2000.0, 400.0)
                })
                .normalize();
                let camera = Transform::from_translation(at).looking_to(look, Vec3::Y);
                if glass {
                    commands.spawn((
                        Mesh3d(meshes.add(Rectangle::new(10.0, 7.0))),
                        MeshMaterial3d(materials.add(StandardMaterial {
                            base_color: Color::linear_rgba(0.02, 0.2, 0.01, 0.75),
                            alpha_mode: AlphaMode::Blend,
                            unlit: true,
                            cull_mode: None,
                            ..default()
                        })),
                        Transform::from_translation(at + look * 15.0).with_rotation(camera.rotation),
                    ));
                    commands.spawn((
                        Mesh3d(meshes.add(Rectangle::new(3.0, 7.0))),
                        MeshMaterial3d(materials.add(StandardMaterial {
                            base_color: Color::linear_rgb(0.1, 0.1, 0.1),
                            unlit: true,
                            cull_mode: None,
                            ..default()
                        })),
                        Transform::from_translation(at + look * 14.0 - camera.right() * 5.0)
                            .with_rotation(camera.rotation),
                    ));
                }
                commands.spawn((
                    Camera3d::default(),
                    camera,
                    Projection::Perspective(PerspectiveProjection {
                        fov: if reference { 42.75_f32 } else { 55.0_f32 }.to_radians(),
                        near: 0.1,
                        far: 20000.0,
                        ..default()
                    }),
                ));
            },
        )
        .add_systems(PostUpdate, verify.after(bevy::transform::TransformSystems::Propagate))
        .run();
}

fn verify(
    time: Res<Time>,
    sprites: Query<&CloudSprite>,
    mut config: ResMut<WorldConfig>,
    mut start: Local<Option<(f32, f32)>>,
    mut checked: Local<bool>,
) {
    let flag = |name| std::env::args().any(|a| a == name);
    if flag("--unload-check") && time.elapsed_secs() > 10.0 && config.environment {
        assert_eq!(sprites.iter().count(), 4);
        config.environment = false;
        info!("cloud world unload: disabled environment");
    }
    if flag("--unload-check") && time.elapsed_secs() > 14.0 && !*checked {
        assert_eq!(sprites.iter().count(), 0);
        *checked = true;
        info!("cloud world unload: verified no cloud entities remain");
    }
    if !flag("--motion-check") || *checked {
        return;
    }
    let Some(sprite) = sprites.iter().find(|s| s.definition().bearing == 180.0) else {
        return;
    };
    let Some((at, angle)) = *start else {
        *start = Some((time.elapsed_secs(), sprite.bearing()));
        return;
    };
    if time.elapsed_secs() - at > 10.0 {
        let expected = (time.elapsed_secs() - at) * sprite.definition().speed;
        let actual = sprite.bearing() - angle;
        assert!(
            (actual - expected).abs() < 0.02,
            "native cloud angle: {actual} versus {expected}"
        );
        *checked = true;
        info!(
            "cloud motion check: {actual:.6} degrees over {:.3} frame-seconds",
            time.elapsed_secs() - at
        );
    }
}
