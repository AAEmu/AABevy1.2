//! Native BC1/ATI2 normal decoding and projected vertex-frame GPU oracle.
//! Top: production shader; bottom: independently calculated original shader vectors.

use aa_cry::texture::{Dds, DdsOptions, dds_to_image};
use aa_world::objects::voxels::{VoxelExtension, VoxelMaterial, VoxelMaterialPlugin};
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use bevy::render::view::screenshot::ScreenshotCaptured;

#[derive(Resource)]
struct Controls([[u8; 3]; 3]);

fn main() {
    App::new()
        .add_plugins(aa_dev::default_plugins("Native projected voxel normals"))
        .add_plugins((aa_dev::DevPlugin, VoxelMaterialPlugin))
        .insert_resource(ClearColor(Color::BLACK))
        .add_systems(Startup, setup)
        .add_observer(pixels)
        .run();
}

fn raw_texture(
    path: &str,
    point: UVec2,
    images: &mut Assets<Image>,
) -> (Handle<Image>, Vec4, Vec2) {
    let bytes = aa_cry::read(path).unwrap();
    let mut dds = Dds::parse(&bytes).unwrap();
    dds.swapped_rg = false;
    let pixel_index = ((point.y * dds.width + point.x) * 4) as usize;
    let pixel: [u8; 4] = dds.decode_rgba8(0, 0)[pixel_index..pixel_index + 4]
        .try_into()
        .unwrap();
    let texel = Vec4::from_array(pixel.map(|c| c as f32 / 255.0));
    let uv = (point.as_vec2() + Vec2::splat(0.5)) / Vec2::new(dds.width as f32, dds.height as f32);
    let image = dds_to_image(&dds, DdsOptions::linear()).unwrap();
    (images.add(image), texel, uv)
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<VoxelMaterial>>,
) {
    let (ati2, raw, uv) = raw_texture(
        "textures/aa_terrain/canyon/tr_canyon_rock3d_406b01_ddn.dds",
        UVec2::ZERO,
        &mut images,
    );
    let (detail, raw_detail, detail_uv) = raw_texture(
        "textures/aa_terrain/canyon/cliff_grey_blue_with_dry_bushes_ddn.dds",
        UVec2::ZERO,
        &mut images,
    );
    let (bc1, raw_bc1, bc1_uv) = raw_texture(
        "textures/aa_terrain/field/tr_field_dirt_001a_ddn.dds",
        UVec2::new(10, 311),
        &mut images,
    );
    let mut expected = [[0; 3]; 3];
    for i in 0..3 {
        let center = Vec3::new((i as f32 - 1.0) * 2.0, 1.0, 0.0);
        let mut mesh = Mesh::from(Rectangle::new(1.5, 1.5));
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0; 4]; 4]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, vec![[1.0, 1024.0]; 4]);
        // Varying native normals make vertex projection-before-interpolation measurable.
        // These are directions only; the original renderer packs and expands without normalization.
        let directions = [
            Vec3::new(0.1, -0.9, 0.4),
            Vec3::new(0.6, -0.7, -0.2),
            Vec3::new(-0.3, -0.8, 0.5),
            Vec3::new(-0.5, -0.7, -0.3),
        ];
        let native: Vec<_> = directions
            .map(|n| {
                aa_world::cry_to_bevy(Vec3::from_array(
                    n.normalize()
                        .to_array()
                        .map(|c| (c * 127.5 + 128.0) as u8 as f32 * (2.0 / 255.0) - 1.0),
                ))
            })
            .into();
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, native.clone());
        let Some(VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!()
        };
        let tri: Vec<_> = mesh.indices().unwrap().iter().take(3).collect();
        let p: Vec<_> = tri
            .iter()
            .map(|&j| Vec2::new(positions[j][0], positions[j][1]))
            .collect();
        let v0 = p[1] - p[0];
        let v1 = p[2] - p[0];
        let v2 = -p[0];
        let d = v0.dot(v0) * v1.dot(v1) - v0.dot(v1).powi(2);
        let b = (v1.dot(v1) * v2.dot(v0) - v0.dot(v1) * v2.dot(v1)) / d;
        let c = (v0.dot(v0) * v2.dot(v1) - v0.dot(v1) * v2.dot(v0)) / d;
        let bary = [1.0 - b - c, b, c];
        let mut frame = [Vec3::ZERO; 3];
        for (&j, w) in tri.iter().zip(bary) {
            let n = native[j];
            frame[0] += (Vec3::X - n * Vec3::X.dot(n)).normalize() * w;
            frame[1] += (Vec3::NEG_Y - n * Vec3::NEG_Y.dot(n)).normalize() * w;
            frame[2] += n * w;
        }
        let mut bump = if i == 2 {
            raw_bc1.truncate() * 2.0 - Vec3::ONE
        } else {
            let xy = Vec2::new(raw.y, raw.x) * 2.0 - Vec2::ONE;
            Vec3::new(xy.x, xy.y, (1.0 - xy.length_squared()).max(0.0).sqrt())
        };
        if i == 1 {
            bump.x += raw_detail.x * 2.0 - 1.0;
            bump.y += raw_detail.y * 2.0 - 1.0;
        }
        let world = (frame[0] * bump.x + frame[1] * bump.y + frame[2] * bump.z).normalize();
        let linear = world * 0.5 + Vec3::splat(0.5);
        expected[i] = linear.to_array().map(|v| {
            ((if v <= 0.0031308 {
                v * 12.92
            } else {
                1.055 * v.powf(1.0 / 2.4) - 0.055
            }) * 255.0)
                .round() as u8
        });
        let base = materials.add(VoxelMaterial {
            base: StandardMaterial {
                base_color: Color::BLACK,
                unlit: true,
                ..default()
            },
            extension: VoxelExtension::new(None),
        });
        commands.spawn((
            Mesh3d(meshes.add(mesh.clone())),
            MeshMaterial3d(base),
            Transform::from_translation(center),
        ));
        let mut extension = VoxelExtension::new(None);
        extension.params.flags.y = 1.0;
        extension.params.u = Vec4::X;
        extension.params.v = Vec4::new(0.0, 0.0, -1.0, 0.0);
        extension.params.normals = Vec4::new(
            if i == 2 { 1.0 } else { 2.0 },
            if i == 1 { 1.0 } else { 0.0 },
            1.0,
            1.0,
        );
        extension.normal = Some(if i == 2 { bc1.clone() } else { ati2.clone() });
        extension.detail_normal = Some(detail.clone());
        let uv = if i == 2 { bc1_uv } else { uv };
        extension.params.bump_u = Vec4::new(0.0, 0.0, 0.0, uv.x);
        extension.params.bump_v = Vec4::new(0.0, 0.0, 0.0, uv.y);
        extension.params.detail_u = Vec4::new(0.0, 0.0, 0.0, detail_uv.x);
        extension.params.detail_v = Vec4::new(0.0, 0.0, 0.0, detail_uv.y);
        commands.spawn((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(materials.add(VoxelMaterial {
                base: StandardMaterial {
                    unlit: true,
                    alpha_mode: AlphaMode::Blend,
                    ..default()
                },
                extension,
            })),
            Transform::from_translation(center),
        ));
        let mut control = Mesh::from(Rectangle::new(1.5, 1.5));
        control.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0; 4]; 4]);
        commands.spawn((
            Mesh3d(meshes.add(control)),
            MeshMaterial3d(materials.add(VoxelMaterial {
                base: StandardMaterial {
                    base_color: Color::linear_rgb(linear.x, linear.y, linear.z),
                    unlit: true,
                    ..default()
                },
                extension: VoxelExtension::new(None),
            })),
            Transform::from_xyz(center.x, -1.0, 0.0),
        ));
        info!(
            "native voxel normal case={i}: raw_ATI2={raw:?} raw_detail={raw_detail:?} BC1={raw_bc1:?} tangent={bump:?} world={world:?} expected_srgb={:?}",
            expected[i]
        );
    }
    commands.insert_resource(Controls(expected));
    commands.spawn((
        Camera3d::default(),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: bevy::camera::ScalingMode::FixedVertical {
                viewport_height: 6.0,
            },
            ..OrthographicProjection::default_3d()
        }),
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
        Transform::from_xyz(0.0, 0.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((Text::new("Original ATI2 / ATI2 + raw detail XY / original BC1 XYZ\nTop: production projected normal shader; bottom: independent panel-centre vectors\nVarying packed vertex normals; signed projection U=X, V=-Z (Cry coordinates)"),TextFont {font_size:FontSize::Px(20.0),..default()},TextColor(Color::WHITE),Node {position_type:PositionType::Absolute,top:px(12.0),left:px(24.0),..default()}));
}

fn pixels(shot: On<ScreenshotCaptured>, controls: Res<Controls>) {
    let image = shot.image.clone().try_into_dynamic().unwrap().to_rgb8();
    let (w, h) = image.dimensions();
    for i in 0..3 {
        let x = (w as i32 / 2 + (i as i32 - 1) * h as i32 / 3) as u32;
        let gpu = image.get_pixel(x, h / 3).0;
        let cpu = image.get_pixel(x, 2 * h / 3).0;
        for c in 0..3 {
            assert!(
                gpu[c].abs_diff(cpu[c]) <= 2,
                "normal case{i} GPU={gpu:?} CPU={cpu:?}"
            );
            assert!(
                gpu[c].abs_diff(controls.0[i][c]) <= 2,
                "normal case{i} GPU={gpu:?} expected={:?}",
                controls.0[i]
            );
        }
        info!("native projected normal GPU oracle case{i} PASSED GPU={gpu:?} CPU={cpu:?}");
    }
}
