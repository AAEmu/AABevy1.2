//! Original cave Terrain base pass. --off hides only voxel surfaces; physics is unchanged.
//! --unload-check moves the camera and verifies cell/GPU-asset release.
//! --colour-check compares the actual native vertex colour against its numeric linear control.

use std::collections::HashSet;

use aa_world::objects::voxels::{
    VoxelDetail, VoxelExtension, VoxelMaterial, VoxelMaterialPlugin, VoxelSurface,
};
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
    exterior: bool,
    detail_off: bool,
    normal_off: bool,
    detail_normal_off: bool,
    meshes: HashSet<AssetId<Mesh>>,
    materials: HashSet<AssetId<VoxelMaterial>>,
    images: HashSet<AssetId<Image>>,
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
            .add_plugins(VoxelMaterialPlugin)
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
                exterior: flag("--exterior"),
                detail_off: flag("--detail-off"),
                normal_off: flag("--normal-off"),
                detail_normal_off: flag("--detail-normal-off"),
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

fn world_camera(mut commands: Commands, fixture: Res<Fixture>) {
    // Original compiled-proxy floor: (21354.6667,9055.3333,454.91738).
    let position = vector_arg("--pos").unwrap_or(if fixture.exterior {
        Vec3::new(22310.0, 8180.0, 560.0)
    } else {
        Vec3::new(21354.0, 9055.0, 456.7)
    });
    let target = vector_arg("--look").unwrap_or(if fixture.exterior {
        Vec3::new(22368.0, 8224.0, 510.0)
    } else {
        Vec3::new(21335.0, 9044.0, 458.5)
    });
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
        &MeshMaterial3d<VoxelMaterial>,
        &mut Visibility,
    )>,
    mut camera: Query<&mut Transform, With<Camera3d>>,
    mut details: Query<
        (
            &VoxelDetail,
            &Mesh3d,
            &MeshMaterial3d<VoxelMaterial>,
            &mut Visibility,
        ),
        Without<VoxelSurface>,
    >,
    meshes: Res<Assets<Mesh>>,
    mut materials: ResMut<Assets<VoxelMaterial>>,
    images: Res<Assets<Image>>,
) {
    for (_, mesh, material, mut visibility) in &mut surfaces {
        fixture.meshes.insert(mesh.id());
        fixture.materials.insert(material.id());
        if let Some(image) = materials
            .get(material.id())
            .and_then(|m| m.extension.cover.as_ref())
        {
            fixture.images.insert(image.id());
        }
        if fixture.off {
            *visibility = Visibility::Hidden;
        }
    }
    for (_, mesh, material, mut visibility) in &mut details {
        fixture.meshes.insert(mesh.id());
        fixture.materials.insert(material.id());
        if let Some(material) = materials.get(material.id()) {
            for image in [
                &material.extension.cover,
                &material.extension.diffuse,
                &material.extension.normal,
                &material.extension.detail_normal,
            ]
            .into_iter()
            .flatten()
            {
                fixture.images.insert(image.id());
            }
        }
        let change_normal = materials.get(material.id()).is_some_and(|m| {
            (fixture.normal_off && m.extension.params.normals.x != 0.0)
                || (fixture.detail_normal_off && m.extension.params.normals.y != 0.0)
        });
        if change_normal && let Some(mut material) = materials.get_mut(material.id()) {
            if fixture.normal_off {
                material.extension.params.normals.x = 0.0;
            }
            if fixture.detail_normal_off {
                material.extension.params.normals.y = 0.0;
            }
        }
        if fixture.off || fixture.detail_off {
            *visibility = Visibility::Hidden;
        }
    }
    if fixture.exterior
        && !fixture.ray_checked
        && surfaces.iter().filter(|(v, ..)| v.cell == (21, 8)).count() == 2
    {
        let target_materials: HashSet<_> = surfaces
            .iter()
            .filter(|(v, ..)| v.cell == (21, 8))
            .map(|(_, _, m, _)| m.id())
            .collect();
        assert_eq!(target_materials.len(), 1, "shared native cover material");
        assert!(
            materials
                .get(*target_materials.iter().next().unwrap())
                .unwrap()
                .extension
                .cover
                .is_some(),
            "original cover tile"
        );
        fixture.ray_checked = true;
        info!("original neighbour meshes share one original cover tile/material");
    }
    if !fixture.exterior
        && !fixture.ray_checked
        && objects.stats.voxels >= 7
        && collision.cell(20, 8).is_some()
    {
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
        let normal_materials = fixture
            .materials
            .iter()
            .filter_map(|id| materials.get(*id))
            .filter(|m| m.extension.normal.is_some())
            .count();
        let detail_materials = fixture
            .materials
            .iter()
            .filter_map(|id| materials.get(*id))
            .filter(|m| m.extension.detail_normal.is_some())
            .count();
        info!(
            "voxel normal resources: captured_images={} normal_materials={} detail_normal_materials={}",
            fixture.images.len(),
            normal_materials,
            detail_materials
        );
        info!(
            "voxel base: meshes={} triangles={} unsupported={} captured_mesh_assets={} captured_material_assets={} models_pending={} failed={} layers={} layer_unsupported={}",
            objects.stats.voxels,
            objects.stats.voxel_triangles,
            objects.stats.voxels_unsupported,
            fixture.meshes.len(),
            fixture.materials.len(),
            objects.stats.models_pending,
            objects.stats.models_failed,
            objects.stats.voxel_layers,
            objects.stats.voxel_layers_unsupported,
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
        assert!(fixture.images.iter().all(|id| images.get(*id).is_none()));
        assert_eq!(details.iter().count(), 0);
        assert_eq!((collision.stats().cells, collision.stats().loading), (0, 0));
        info!(
            "voxel base unload PASSED: entities/cell collision/jobs/owned mesh+material+cover/diffuse/normal/detail assets = 0"
        );
        fixture.verified = true;
    }
}

fn colour_control(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<VoxelMaterial>>,
) {
    let source = aa_world::placement::load_cell_objects("main_world", 20, 8);
    let native = source.voxels[0].compiled_base_mesh().unwrap().colors[0].map(|v| v as f32 / 255.0);
    assert_eq!(native, [21.0 / 255.0, 14.0 / 255.0, 11.0 / 255.0, 1.0]);
    for (x, vertex) in [(-1.3, true), (1.3, false)] {
        let mut mesh = Mesh::from(Rectangle::new(2.0, 2.0));
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_COLOR,
            vec![if vertex { native } else { [1.0; 4] }; 4],
        );
        commands.spawn((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(materials.add(VoxelMaterial {
                base: StandardMaterial {
                    base_color: if vertex {
                        Color::WHITE
                    } else {
                        Color::linear_rgba(21.0 / 255.0, 14.0 / 255.0, 11.0 / 255.0, 1.0)
                    },
                    unlit: true,
                    ..default()
                },
                extension: VoxelExtension::new(None),
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
