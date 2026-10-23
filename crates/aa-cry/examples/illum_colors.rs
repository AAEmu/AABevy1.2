//! Illum interpolation/alpha fixture, independent of the pak. `--before` reproduces omitted colours.
//! Left: black-to-white gradient. Middle: red-to-blue gradient. Right: constant 128/255 RGB.
//! Source alpha is 7/255 on every vertex; the masked white surfaces must remain visible.
//! `--emission` exercises native Full composition with MatEmissiveColor=0.5 and no scene lights.

use aa_cry::mesh::Subset;
use aa_cry::shade::illum::{IllumExt, IllumMaterial, IllumParams};
use aa_cry::{CryPlugin, MeshData};
use bevy::asset::RenderAssetUsages;
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-cry: Illum vertex colours"),
            aa_dev::DevPlugin,
            CryPlugin,
        ))
        .insert_resource(ClearColor(Color::srgb(0.12, 0.14, 0.17)))
        .insert_resource(GlobalAmbientLight {
            brightness: 0.0,
            ..default()
        })
        .add_systems(Startup, setup)
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<IllumMaterial>>,
) {
    let before = std::env::args().any(|a| a == "--before");
    let emission = std::env::args().any(|a| a == "--emission");
    let mut params = IllumParams::from_material(
        &aa_cry::material::Material {
            shader: "Illum".into(),
            emissive: Vec3::splat(0.5),
            ..default()
        },
        false,
        false,
    );
    params.vertex = Vec4::new(
        if before { 0.0 } else { 1.0 },
        if emission { 1.0 } else { 0.0 },
        0.0,
        0.0,
    );
    let material = materials.add(IllumMaterial {
        base: StandardMaterial {
            base_color: Color::WHITE,
            unlit: !emission,
            alpha_mode: AlphaMode::Mask(0.5),
            ..default()
        },
        extension: IllumExt {
            params,
            ..default()
        },
    });
    let subset = Subset {
        first_index: 0,
        index_count: 6,
        first_vertex: 0,
        vertex_count: 4,
        material_id: 0,
        radius: 1.0,
        center: Vec3::ZERO,
    };
    for (i, (left, right)) in [
        ([0, 0, 0, 7], [255, 255, 255, 7]),
        ([255, 0, 0, 7], [0, 0, 255, 7]),
        ([128, 128, 128, 7], [128, 128, 128, 7]),
    ]
    .into_iter()
    .enumerate()
    {
        let source = MeshData {
            positions: vec![
                Vec3::new(-0.6, -0.8, 0.0),
                Vec3::new(0.6, -0.8, 0.0),
                Vec3::new(0.6, 0.8, 0.0),
                Vec3::new(-0.6, 0.8, 0.0),
            ],
            normals: vec![Vec3::Z; 4],
            colors: vec![left, right, right, left],
            indices: vec![0, 1, 2, 0, 2, 3],
            ..default()
        };
        let (mut mesh, _) =
            aa_cry::scene::subset_mesh(&source, &subset, false, RenderAssetUsages::RENDER_WORLD)
                .unwrap();
        assert!(aa_cry::scene::insert_illum_vertex_colors(
            &mut mesh, &source, &subset, false
        ));
        let mut entity = commands.spawn((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(material.clone()),
            Transform::from_xyz((i as f32 - 1.0) * 1.5, 0.0, 0.0),
        ));
        if std::env::args().any(|arg| arg == "--dissolve") {
            entity.insert(bevy::camera::visibility::VisibilityRange {
                start_margin: 0.0..0.0,
                end_margin: 4.0..6.0,
                use_aabb: false,
            });
        }
    }
    commands.spawn((
        Camera3d::default(),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: bevy::camera::ScalingMode::FixedVertical {
                viewport_height: 3.0,
            },
            ..OrthographicProjection::default_3d()
        }),
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
        Transform::from_xyz(0.0, 0.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    info!(
        "illum_colors: before={before}; midpoint expected linear RGB=0.25 (sRGB 137), constant=(128/255)^2 (sRGB 137)"
    );
}
