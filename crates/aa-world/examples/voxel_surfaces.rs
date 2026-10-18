//! Original cave Terrain base pass. --off hides only voxel surfaces; physics is unchanged.
//! --unload-check moves the camera and verifies cell/GPU-asset release.
//! --colour-check compares the actual native vertex colour against its numeric linear control.

use std::collections::HashSet;

use aa_world::objects::voxels::VoxelSurface;
use aa_world::{AaWorldPlugin, WorldClock, WorldConfig, cry_to_bevy};
use bevy::asset::AssetId;
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::prelude::*;
use bevy::render::view::screenshot::ScreenshotCaptured;

#[derive(Resource, Default)]
struct Fixture {
    off: bool,
    unload: bool,
    moved: bool,
    verified: bool,
    ray_checked: bool,
    meshes: HashSet<AssetId<Mesh>>,
    materials: HashSet<AssetId<StandardMaterial>>,
    last: f32,
}

fn main() {
    let flag = |name: &str| std::env::args().any(|arg| arg == name);
    let mut app = App::new();
    app.add_plugins(aa_dev::default_plugins(
        "aa-world original voxel base surfaces",
    ))
    .add_plugins(aa_dev::DevPlugin);
    if flag("--colour-check") {
        app.insert_resource(ClearColor(Color::BLACK))
            .add_systems(Startup, colour_control)
            .add_observer(colour_pixels);
    } else {
        let mut config = WorldConfig::new("main_world");
        config.load_radius = 180.0;
        config.unload_radius = 300.0;
        config.brush_distance = 280.0;
        config.tree_distance = 280.0;
        config.vegetation = false;
        app.add_plugins(AaWorldPlugin { config })
            .insert_resource(WorldClock {
                hour: 10.0,
                ..default()
            })
            .insert_resource(Fixture {
                off: flag("--off"),
                unload: flag("--unload-check"),
                ..default()
            })
            .add_systems(Startup, world_camera)
            .add_systems(Update, verify);
    }
    app.run();
}

fn vector_arg(name: &str) -> Option<Vec3> {
    let args: Vec<_> = std::env::args().collect();
    let v = args.get(args.iter().position(|arg| arg == name)? + 1)?;
    let values: Vec<f32> = v
        .split(',')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    (values.len() == 3 && values.iter().all(|v| v.is_finite())).then(|| Vec3::from_slice(&values))
}

fn world_camera(mut commands: Commands) {
    // Original compiled-proxy floor: (21354.6667,9055.3333,454.91738).
    let position = vector_arg("--pos").unwrap_or(Vec3::new(21354.0, 9055.0, 456.7));
    let target = vector_arg("--look").unwrap_or(Vec3::new(21335.0, 9044.0, 458.5));
    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(cry_to_bevy(position)).looking_at(cry_to_bevy(target), Vec3::Y),
        Projection::Perspective(PerspectiveProjection {
            fov: 60.0_f32.to_radians(),
            near: 0.1,
            ..default()
        }),
    ));
}

#[allow(clippy::too_many_arguments)]
fn verify(
    mut fixture: ResMut<Fixture>,
    time: Res<Time>,
    objects: Res<aa_world::objects::ObjectState>,
    collision: Res<aa_world::WorldCollision>,
    mut surfaces: Query<(
        &VoxelSurface,
        &Mesh3d,
        &MeshMaterial3d<StandardMaterial>,
        &mut Visibility,
    )>,
    mut camera: Query<&mut Transform, With<Camera3d>>,
    meshes: Res<Assets<Mesh>>,
    materials: Res<Assets<StandardMaterial>>,
) {
    for (_, mesh, material, mut visibility) in &mut surfaces {
        fixture.meshes.insert(mesh.id());
        fixture.materials.insert(material.id());
        if fixture.off {
            *visibility = Visibility::Hidden;
        }
    }
    if !fixture.ray_checked && objects.stats.voxels >= 7 && collision.cell(20, 8).is_some() {
        let point = Vec3::new(21354.6667, 9055.3333, 454.91738);
        let ground = collision
            .ground_below(point.x, point.y, point.z + 0.3, 0.0)
            .unwrap();
        assert!((ground.height - point.z).abs() < 0.01);
        info!(
            "original cave floor verified: {:?}; rendered voxel tris={}",
            ground, objects.stats.voxel_triangles
        );
        fixture.ray_checked = true;
    }
    if time.elapsed_secs() > fixture.last + 2.0 {
        fixture.last = time.elapsed_secs();
        info!(
            "voxel base: meshes={} triangles={} unsupported={} captured_mesh_assets={} captured_material_assets={} models_pending={} failed={}",
            objects.stats.voxels,
            objects.stats.voxel_triangles,
            objects.stats.voxels_unsupported,
            fixture.meshes.len(),
            fixture.materials.len(),
            objects.stats.models_pending,
            objects.stats.models_failed
        );
    }
    if fixture.unload && !fixture.moved && time.elapsed_secs() > 12.0 {
        assert!(fixture.ray_checked && !fixture.meshes.is_empty());
        camera.single_mut().unwrap().translation =
            cry_to_bevy(Vec3::new(-100000.0, -100000.0, 1000.0));
        fixture.moved = true;
    }
    if fixture.moved && !fixture.verified && time.elapsed_secs() > 16.0 {
        assert_eq!(
            (
                objects.stats.voxels,
                objects.stats.voxel_triangles,
                surfaces.iter().count()
            ),
            (0, 0, 0)
        );
        assert!(fixture.meshes.iter().all(|id| meshes.get(*id).is_none()));
        assert!(
            fixture
                .materials
                .iter()
                .all(|id| materials.get(*id).is_none())
        );
        assert_eq!((collision.stats().cells, collision.stats().loading), (0, 0));
        info!(
            "voxel base unload PASSED: entities/cell collision/jobs/owned mesh+material assets = 0"
        );
        fixture.verified = true;
    }
}

fn colour_control(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let source = aa_world::placement::load_cell_objects("main_world", 20, 8);
    let native = source.voxels[0].compiled_base_mesh().unwrap().colors[0].map(|v| v as f32 / 255.0);
    assert_eq!(native, [21.0 / 255.0, 14.0 / 255.0, 11.0 / 255.0, 1.0]);
    for (x, vertex) in [(-1.3, true), (1.3, false)] {
        let mut mesh = Mesh::from(Rectangle::new(2.0, 2.0));
        if vertex {
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![native; 4]);
        }
        commands.spawn((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: if vertex {
                    Color::WHITE
                } else {
                    Color::linear_rgba(21.0 / 255.0, 14.0 / 255.0, 11.0 / 255.0, 1.0)
                },
                unlit: true,
                ..default()
            })),
            Transform::from_xyz(x, 0.0, 0.0),
        ));
    }
    commands.spawn((
        Camera3d::default(),
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
        Transform::from_xyz(0.0, 0.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((Text::new("Native baked vertex RGB (left) / numeric linear control (right)\nOriginal (COLORS.r, COLORS.b, COLORS2.b) = (21,14,11)/255\nTerrain.Layer projected detail is not included in this base-pass slice"),
        TextFont { font_size: FontSize::Px(22.0), ..default() }, TextColor(Color::WHITE),
        Node { position_type: PositionType::Absolute, top: px(20.0), left: px(24.0), ..default() }));
}

fn colour_pixels(shot: On<ScreenshotCaptured>) {
    let image = shot.image.clone().try_into_dynamic().unwrap().to_rgb8();
    let (w, h) = image.dimensions();
    let left = image.get_pixel(w * 35 / 100, h / 2).0;
    let right = image.get_pixel(w * 65 / 100, h / 2).0;
    assert_eq!(
        left, right,
        "native linear vertex-colour vs uniform control"
    );
    for (actual, expected) in left.into_iter().zip([81u8, 66, 59]) {
        assert!(
            actual.abs_diff(expected) <= 1,
            "native linear-to-sRGB channel {actual} vs {expected}"
        );
    }
    info!("native voxel colour GPU control PASSED: {left:?}");
}
