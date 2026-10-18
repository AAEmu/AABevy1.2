//! Native per-instance vegetation COLOR_VARIATION: shared material GPU controls and original forest.
//! --world [--off] [--unload-check] : original main_world cell12,14. Default: numeric; --fog adds fog order.

use aa_world::vegetation_color::{
    self, VegetationColor, VegetationColorPart, VegetationColorPlugin, VegetationMaterial,
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
    reference: bool,
    unload: bool,
    moved: bool,
    done: bool,
    parts: HashSet<Entity>,
    material_ids: HashSet<AssetId<VegetationMaterial>>,
    bases: HashMap<AssetId<VegetationMaterial>, Handle<StandardMaterial>>,
    last: f32,
}
fn main() {
    let flag = |v: &str| std::env::args().any(|a| a == v);
    let mut app = App::new();
    app.add_plugins((
        aa_dev::default_plugins("Native vegetation instance colour"),
        aa_dev::DevPlugin,
    ));
    if flag("--world") {
        let mut config = aa_world::WorldConfig::new("main_world");
        config.load_radius = if flag("--reference") { 450.0 } else { 100.0 };
        config.unload_radius = if flag("--reference") { 650.0 } else { 220.0 };
        config.brush_distance = 150.0;
        config.vegetation_distance = 100.0;
        config.tree_distance = 150.0;
        config.water_distance = 180.0;
        app.add_plugins(aa_world::AaWorldPlugin { config })
            .insert_resource(aa_world::WorldClock {
                hour: if flag("--reference") { 19.166172 } else { 10.0 },
                ..default()
            })
            .insert_resource(Fixture {
                reference: flag("--reference"),
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
    mut materials: ResMut<Assets<VegetationMaterial>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
) {
    let fog = std::env::args().any(|a| a == "--fog");
    let bgrs = [[0, 0, 255], [255, 128, 0], [69, 208, 233], [127; 3]];
    let mut controls = Vec::new();
    let mesh = meshes.add(Rectangle::new(1.5, 0.85));
    for row in 0..3 {
        // A shared handle for four different tags. Row1 is genuinely lit emissive, not albedo.
        let base = StandardMaterial {
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
        };
        let shared = materials.add(VegetationMaterial {
            base,
            extension: VegetationColor,
        });
        for (i, bgr) in bgrs.into_iter().enumerate() {
            let p = Vec2::new(i as f32 * 2.0 - 3.0, 2.5 - row as f32 * 2.2);
            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(shared.clone()),
                MeshTag::with_type::<VegetationColor>(vegetation_color::pack(bgr)),
                Transform::from_xyz(p.x, p.y, 0.0),
            ));
            // Independent native CPU formula, not the production packing implementation.
            let q = bgr.map(|v| (f64::from(v) * 63.0 / 255.0).floor().clamp(1.0, 63.0) as f32);
            let gain = Vec3::new(q[2], q[1], q[0]) * (0.4 / 63.0) + Vec3::splat(0.8);
            let mut color = Vec3::new(0.25, 0.4, 0.6) * gain;
            if fog {
                let f = Vec3::new(p.x, p.y, -6.0).length() / 12.0;
                color = color.lerp(Vec3::new(0.6, 0.1, 0.2), f);
            }
            if row == 2 {
                color *= 0.5;
            }
            let expected = color.to_array().map(|v| {
                ((if v <= 0.0031308 {
                    v * 12.92
                } else {
                    1.055 * v.powf(1.0 / 2.4) - 0.055
                }) * 255.0)
                    .round() as u8
            });
            controls.push((p, expected));
            // Small independent reference swatch below each production panel; fog already included.
            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(standard.add(StandardMaterial {
                    base_color: Color::linear_rgb(color.x, color.y, color.z),
                    unlit: true,
                    fog_enabled: false,
                    ..default()
                })),
                Transform::from_xyz(p.x, p.y - 0.65, 0.0).with_scale(Vec3::new(1.0, 0.35, 1.0)),
            ));
        }
    }
    commands.insert_resource(Controls(controls));
    let mut camera = commands.spawn((
        Camera3d::default(),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: bevy::camera::ScalingMode::FixedVertical {
                viewport_height: 8.0,
            },
            ..OrthographicProjection::default_3d()
        }),
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
        Transform::from_xyz(0.0, 0.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    if fog {
        camera.insert(DistanceFog {
            color: Color::linear_rgb(0.6, 0.1, 0.2),
            falloff: FogFalloff::Linear {
                start: 0.0,
                end: 12.0,
            },
            ..default()
        });
    }
    commands.spawn((
        Text::new(
            "Native COLOR_VARIATION: four tags share each material
Rows: unlit RGB / lit emissive / alpha 0.5. Thin swatches: independent CPU.
BGR: (0,0,255), (255,128,0), authored (69,208,233), default127",
        ),
        TextFont {
            font_size: FontSize::Px(18.0),
            ..default()
        },
        Node {
            position_type: PositionType::Absolute,
            left: px(20.0),
            top: px(10.0),
            ..default()
        },
    ));
}
fn pixels(shot: On<ScreenshotCaptured>, controls: Res<Controls>) {
    let image = shot.image.clone().try_into_dynamic().unwrap().to_rgb8();
    let (w, h) = image.dimensions();
    for (i, (p, expected)) in controls.0.iter().enumerate() {
        let x = (w as f32 * 0.5 + p.x * h as f32 / 8.0) as u32;
        let y = (h as f32 * 0.5 - p.y * h as f32 / 8.0) as u32;
        let gpu = image.get_pixel(x, y).0;
        for c in 0..3 {
            assert!(
                gpu[c].abs_diff(expected[c]) <= 2,
                "vegetation case{i} GPU={gpu:?} CPU={expected:?}"
            );
        }
        info!("native vegetation GPU case{i} PASSED GPU={gpu:?} CPU={expected:?}");
    }
}
fn world_setup(
    mut commands: Commands,
    fixture: Res<Fixture>,
    terrain: Res<aa_world::TerrainQuery>,
    water: Res<aa_world::WaterQuery>,
) {
    let target = Vec3::new(12812.47, 14575.70, 158.28);
    info!(
        "original forest12,14 authored group799 target={target:?} ground={:?} water={:?}",
        terrain.height_at(target.x, target.y),
        water.water_at(target)
    );
    let transform = if fixture.reference {
        // Reference camera uses the original engine's 11-cell (11264m) floating origin.
        Transform::from_translation(aa_world::cry_to_bevy(Vec3::new(
            15579.493, 15385.854, 127.979904,
        )))
        .looking_to(
            aa_world::cry_to_bevy(Vec3::new(-0.3625982, -0.9319457, 0.0)),
            Vec3::Y,
        )
    } else {
        Transform::from_translation(aa_world::cry_to_bevy(Vec3::new(12807.0, 14566.0, 162.0)))
            .looking_at(aa_world::cry_to_bevy(target + Vec3::Z * 0.6), Vec3::Y)
    };
    commands.spawn((
        Camera3d::default(),
        transform,
        Projection::Perspective(PerspectiveProjection {
            fov: (if fixture.reference {
                42.75_f32
            } else {
                50.0_f32
            })
            .to_radians(),
            near: 0.1,
            far: 20000.0,
            ..default()
        }),
    ));
}
#[allow(clippy::too_many_arguments)]
fn world_check(
    mut commands: Commands,
    time: Res<Time>,
    objects: Res<aa_world::objects::ObjectState>,
    fresh: Query<
        (Entity, &MeshMaterial3d<VegetationMaterial>),
        Added<MeshMaterial3d<VegetationMaterial>>,
    >,
    parts: Query<(Entity, &VegetationColorPart)>,
    all: Query<()>,
    materials: Res<Assets<VegetationMaterial>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
    mut fixture: ResMut<Fixture>,
    mut camera: Query<&mut Transform, With<Camera3d>>,
) {
    for (e, m) in &fresh {
        fixture.material_ids.insert(m.0.id());
        fixture.parts.insert(e);
        if fixture.off {
            let h = if let Some(h) = fixture.bases.get(&m.0.id()) {
                h.clone()
            } else {
                let h = standard.add(materials.get(&m.0).unwrap().base.clone());
                fixture.bases.insert(m.0.id(), h.clone());
                h
            };
            commands
                .entity(e)
                .remove::<MeshMaterial3d<VegetationMaterial>>()
                .insert(MeshMaterial3d(h));
        }
    }
    if time.elapsed_secs() - fixture.last > 2.0 {
        fixture.last = time.elapsed_secs();
        info!(
            "forest colour: parts={} nonneutral={} unique_materials={} objects={} pending={} failed={}",
            parts.iter().count(),
            parts.iter().filter(|(_, p)| p.bgr != [127; 3]).count(),
            fixture.material_ids.len(),
            objects.stats.entities,
            objects.stats.models_pending,
            objects.stats.models_failed
        );
    }
    if fixture.unload && !fixture.moved && time.elapsed_secs() > 16.0 {
        assert!(
            parts
                .iter()
                .any(|(_, p)| p.cell == (12, 14) && p.bgr != [127; 3]),
            "must draw actual varied forest plants first"
        );
        assert!(
            fixture.parts.len() > fixture.material_ids.len() * 10,
            "instances must share materials"
        );
        camera.single_mut().unwrap().translation = aa_world::cry_to_bevy(Vec3::splat(-100000.0));
        fixture.moved = true;
    }
    if fixture.moved && !fixture.done && time.elapsed_secs() > 21.0 {
        assert!(
            parts.is_empty(),
            "all qualified cell vegetation parts must despawn"
        );
        assert_eq!(objects.stats.entities, 0);
        assert!(
            fixture.parts.iter().all(|&e| all.get(e).is_err()),
            "every recorded mesh-part entity must be gone"
        );
        info!(
            "native vegetation unload PASSED: {} seen parts removed, {} shared materials retain existing model cache lifetime",
            fixture.parts.len(),
            fixture.material_ids.len()
        );
        fixture.done = true;
    }
}
