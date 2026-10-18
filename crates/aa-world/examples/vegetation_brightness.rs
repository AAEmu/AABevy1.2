//! Authored non-terrain vegetation brightness. Default GPU controls; --world [--off] [--unload-check].
use aa_world::vegetation_color::{
    self, VegetationBrightnessPart, VegetationColor, VegetationColorPlugin, VegetationMaterial,
};
use bevy::{
    asset::AssetId,
    core_pipeline::tonemapping::{DebandDither, Tonemapping},
    mesh::MeshTag,
    prelude::*,
    render::view::screenshot::ScreenshotCaptured,
};
use std::collections::{HashMap, HashSet};
#[derive(Resource)]
struct Controls(Vec<(Vec2, [u8; 3])>);
#[derive(Resource, Default)]
struct Fixture {
    off: bool,
    unload: bool,
    moved: bool,
    done: bool,
    last: f32,
    parts: HashSet<Entity>,
    meshes: HashSet<AssetId<Mesh>>,
    materials: HashSet<AssetId<VegetationMaterial>>,
    controls: HashMap<AssetId<VegetationMaterial>, Handle<VegetationMaterial>>,
}
fn flag(s: &str) -> bool {
    std::env::args().any(|a| a == s)
}
fn main() {
    let mut app = App::new();
    app.add_plugins((
        aa_dev::default_plugins("Native vegetation group brightness"),
        aa_dev::DevPlugin,
    ));
    if flag("--world") {
        let mut config = aa_world::WorldConfig::new("main_world");
        config.load_radius = 100.0;
        config.unload_radius = 220.0;
        config.brush_distance = 150.0;
        config.vegetation_distance = 100.0;
        config.tree_distance = 150.0;
        config.water_distance = 180.0;
        app.add_plugins(aa_world::AaWorldPlugin { config })
            .insert_resource(aa_world::WorldClock {
                hour: 10.0,
                ..default()
            })
            .insert_resource(Fixture {
                off: flag("--off"),
                unload: flag("--unload-check"),
                ..default()
            })
            .add_systems(Startup, world_setup)
            .add_systems(Update, world_check);
    } else {
        app.add_plugins(VegetationColorPlugin)
            .insert_resource(ClearColor(Color::BLACK))
            .add_systems(Startup, numeric)
            .add_observer(pixels);
    }
    app.run();
}
fn numeric(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<VegetationMaterial>>,
    mut stdm: ResMut<Assets<StandardMaterial>>,
) {
    let mut controls = Vec::new();
    for row in 0..4 {
        for (i, raw) in [0.0f32, 0.4, 1.0, 10.0].into_iter().enumerate() {
            let brightness = raw.min(1.0); // native upper cap, independently asserted below.
            let alpha = [255u8, 153, 128, 255][row];
            let shared = mats.add(VegetationMaterial {
                base: StandardMaterial {
                    base_color: if row == 1 {
                        Color::BLACK
                    } else {
                        Color::linear_rgba(0.25, 0.4, 0.6, if row == 2 { 0.5 } else { 1.0 })
                    },
                    unlit: row != 1,
                    reflectance: 0.0,
                    emissive: if row == 1 {
                        LinearRgba::rgb(0.25, 0.4, 0.6)
                    } else {
                        LinearRgba::BLACK
                    },
                    emissive_exposure_weight: 0.0,
                    alpha_mode: if row == 2 {
                        AlphaMode::Blend
                    } else {
                        AlphaMode::Opaque
                    },
                    ..default()
                },
                extension: VegetationColor { brightness, ..default() },
            });
            let p = Vec2::new(i as f32 * 2.0 - 3.0, 3.0 - row as f32 * 2.0);
            let mut mesh = Mesh::from(Rectangle::new(1.5, 0.9));
            // Last row has no AO stream: the existing COLOR_VARIATION path must remain unchanged.
            if row != 3 {
                mesh.insert_attribute(
                    Mesh::ATTRIBUTE_UV_1,
                    vec![[f32::from(alpha) / 255.0, 0.0]; 4],
                );
            }
            let gradient = row == 2 && i == 1;
            if gradient {
                let bevy::mesh::VertexAttributeValues::Float32x3(pos) =
                    mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap()
                else {
                    panic!()
                };
                let uv: Vec<_> = pos.iter().map(|v| [(v[0] + 0.75) / 1.5, 0.0]).collect();
                mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, uv);
            }
            let tag = if row >= 2 {
                vegetation_color::pack([69, 208, 233])
            } else {
                vegetation_color::NO_COLOR_VARIATION
            };
            commands.spawn((
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(shared.clone()),
                MeshTag::with_type::<VegetationColor>(tag),
                Transform::from_xyz(p.x, p.y, 0.0),
            ));
            let ao = if row == 3 {
                1.0
            } else if gradient {
                0.5
            } else {
                f32::from(alpha) / 255.0
            };
            let mut expected = Vec3::new(0.25, 0.4, 0.6)
                * ao
                * if i == 0 {
                    0.0
                } else if i == 1 {
                    0.4
                } else {
                    1.0
                };
            if row >= 2 {
                let q = [233u8, 208, 69].map(|v| (f64::from(v) * 63.0 / 255.0).floor() as f32);
                expected *= Vec3::from_array(q) * (0.4 / 63.0) + Vec3::splat(0.8);
            }
            if flag("--fog") {
                expected = expected.lerp(
                    Vec3::new(0.6, 0.1, 0.2),
                    Vec3::new(p.x, p.y, -6.0).length() / 12.0,
                );
            }
            if row == 2 {
                expected *= 0.5;
            }
            let rgb = expected.to_array().map(|v| {
                ((if v <= 0.0031308 {
                    v * 12.92
                } else {
                    1.055 * v.powf(1.0 / 2.4) - 0.055
                }) * 255.0)
                    .round() as u8
            });
            controls.push((p, rgb));
            commands.spawn((
                Mesh3d(meshes.add(Rectangle::new(1.5, 0.22))),
                MeshMaterial3d(stdm.add(StandardMaterial {
                    base_color: Color::linear_rgb(expected.x, expected.y, expected.z),
                    unlit: true,
                    fog_enabled: false,
                    ..default()
                })),
                Transform::from_xyz(p.x, p.y - 0.65, 0.0),
            ));
        }
    }
    commands.insert_resource(Controls(controls));
    let mut cam = commands.spawn((
        Camera3d::default(),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: bevy::camera::ScalingMode::FixedVertical {
                viewport_height: 9.0,
            },
            ..OrthographicProjection::default_3d()
        }),
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
        Transform::from_xyz(0.0, 0.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    if flag("--fog") {
        cam.insert(DistanceFog {
            color: Color::linear_rgb(0.6, 0.1, 0.2),
            falloff: FogFalloff::Linear {
                start: 0.0,
                end: 12.0,
            },
            ..default()
        });
    }
    commands.spawn((Text::new("Native group brightness: columns raw0 /0.4 /1 /10(cap1)\nRows unlit /lit emissive+AO /opacity0.5+AO+variation /missing AO+variation\nThin swatches: independent CPU. Exact scalar multiplies final lit RGB before fog, alpha unchanged."),TextFont{font_size:FontSize::Px(18.0),..default()},Node{position_type:PositionType::Absolute,left:px(20.0),top:px(10.0),..default()}));
}
fn pixels(shot: On<ScreenshotCaptured>, controls: Res<Controls>) {
    let image = shot.image.clone().try_into_dynamic().unwrap().to_rgb8();
    let (w, h) = image.dimensions();
    for (i, (p, cpu)) in controls.0.iter().enumerate() {
        let x = (w as f32 * 0.5 + p.x * h as f32 / 9.0) as u32;
        let y = (h as f32 * 0.5 - p.y * h as f32 / 9.0) as u32;
        let gpu = image.get_pixel(x, y).0;
        assert!(
            gpu.iter().zip(cpu).all(|(a, b)| a.abs_diff(*b) <= 2),
            "brightness case{i} GPU{gpu:?} CPU{cpu:?}"
        );
        info!("native vegetation brightness GPU case{i} PASSED GPU={gpu:?} CPU={cpu:?}");
    }
}
fn world_setup(mut commands: Commands) {
    // Original group618 flower patch; no authored lighting/material changes.
    let target = Vec3::new(13632.293, 10354.819, 194.68315);
    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(aa_world::cry_to_bevy(target + Vec3::new(-4.0, -5.0, 2.5)))
            .looking_at(aa_world::cry_to_bevy(target + Vec3::Z * 0.45), Vec3::Y),
        Projection::Perspective(PerspectiveProjection {
            fov: 50.0f32.to_radians(),
            near: 0.1,
            far: 20000.0,
            ..default()
        }),
    ));
}
fn world_check(
    mut commands: Commands,
    time: Res<Time>,
    objects: Res<aa_world::objects::ObjectState>,
    fresh: Query<
        (
            Entity,
            &Mesh3d,
            &VegetationBrightnessPart,
            &MeshMaterial3d<VegetationMaterial>,
        ),
        Added<VegetationBrightnessPart>,
    >,
    parts: Query<&VegetationBrightnessPart>,
    all: Query<()>,
    mut mats: ResMut<Assets<VegetationMaterial>>,
    mut f: ResMut<Fixture>,
    mut camera: Query<&mut Transform, With<Camera3d>>,
) {
    for (e, mesh, b, material) in &fresh {
        assert_eq!(b.group, 618);
        assert_eq!(b.brightness, 0.4);
        assert_eq!(mats.get(&material.0).unwrap().extension.brightness, 0.4);
        f.parts.insert(e);
        f.meshes.insert(mesh.0.id());
        f.materials.insert(material.0.id());
        if f.off {
            let handle = if let Some(h) = f.controls.get(&material.0.id()) {
                h.clone()
            } else {
                let mut control = mats.get(&material.0).unwrap().clone();
                control.extension.brightness = 1.0;
                let h = mats.add(control);
                f.controls.insert(material.0.id(), h.clone());
                h
            };
            commands.entity(e).insert(MeshMaterial3d(handle));
        }
    }
    if time.elapsed_secs() - f.last > 2.0 {
        f.last = time.elapsed_secs();
        info!(
            "vegetation brightness: seen={} live={} meshes={} materials={} off={} objects={} pending={} failed={}",
            f.parts.len(),
            parts.iter().count(),
            f.meshes.len(),
            f.materials.len(),
            f.off,
            objects.stats.entities,
            objects.stats.models_pending,
            objects.stats.models_failed
        );
    }
    if f.unload && !f.moved && time.elapsed_secs() > 16.0 {
        assert!(f.parts.len() > 20);
        assert!(f.parts.len() > f.meshes.len());
        assert!(f.parts.len() > f.materials.len());
        camera.single_mut().unwrap().translation = aa_world::cry_to_bevy(Vec3::splat(-100000.0));
        f.moved = true;
    }
    if f.moved && !f.done && time.elapsed_secs() > 22.0 {
        assert!(parts.is_empty());
        assert_eq!(objects.stats.entities, 0);
        assert!(f.parts.iter().all(|e| all.get(*e).is_err()));
        info!(
            "native vegetation brightness unload PASSED: {} original part IDs gone; shared model cache lifetime unchanged",
            f.parts.len()
        );
        f.done = true;
    }
}
