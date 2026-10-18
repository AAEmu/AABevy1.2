//! GPU check of native encoded cover decoding and interpolation with baked voxel RGB.
//! Top: shader cover/halfway/baked; bottom: independent CPU linear-colour controls.

use aa_world::objects::voxels::{VoxelExtension, VoxelMaterial, VoxelMaterialPlugin};
use bevy::asset::RenderAssetUsages;
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::render::view::screenshot::ScreenshotCaptured;

#[derive(Resource)]
struct Controls([[u8; 3]; 3]);

fn main() {
    App::new()
        .add_plugins(aa_dev::default_plugins(
            "Native voxel cover decode / selector control",
        ))
        .add_plugins((aa_dev::DevPlugin, VoxelMaterialPlugin))
        .insert_resource(ClearColor(Color::BLACK))
        .add_systems(Startup, setup)
        .add_observer(pixels)
        .run();
}

// Independent first-pixel BC1 decode. Production keeps the original compressed 128x128 tile.
fn first_pixel(block: &[u8]) -> [u8; 4] {
    let endpoint = |off| {
        let c = u16::from_le_bytes(block[off..off + 2].try_into().unwrap());
        let r = ((c >> 11) & 31) as u8;
        let g = ((c >> 5) & 63) as u8;
        let b = (c & 31) as u8;
        [r << 3 | r >> 2, g << 2 | g >> 4, b << 3 | b >> 2]
    };
    let a = endpoint(0);
    let b = endpoint(2);
    let index = block[4] & 3;
    let four = u16::from_le_bytes(block[0..2].try_into().unwrap())
        > u16::from_le_bytes(block[2..4].try_into().unwrap());
    let rgb = match index {
        0 => a,
        1 => b,
        2 if !four => std::array::from_fn(|i| ((a[i] as u16 + b[i] as u16) / 2) as u8),
        3 if !four => [0; 3],
        2 => std::array::from_fn(|i| ((2 * a[i] as u16 + b[i] as u16) / 3) as u8),
        _ => std::array::from_fn(|i| ((a[i] as u16 + 2 * b[i] as u16) / 3) as u8),
    };
    [rgb[0], rgb[1], rgb[2], 255]
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<VoxelMaterial>>,
) {
    if std::env::args().any(|a| a == "--layer-check") {
        return layer_controls(commands, meshes, images, materials);
    }
    let bytes = aa_pak::Pak::global()
        .read("game/worlds/main_world/cells/021_008/client/terrain/cover.ctc")
        .unwrap();
    let source = aa_world::cover::Cover::parse(&bytes).unwrap();
    assert_eq!(source.layers[0].format, aa_world::cover::CoverFormat::Bc1);
    let raw = first_pixel(source.tile(0, 4, 13, 0).unwrap());
    let image = images.add(Image::new_fill(
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &raw,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::default(),
    ));
    let baked = [21.0 / 255.0, 14.0 / 255.0, 11.0 / 255.0];
    let red = (raw[0] as f32 / 255.0 + 0.001012) * (31.0 / 30.0);
    let green = raw[1] as f32 / 255.0;
    let brightness = 3.0 * raw[2] as f32 / 255.0;
    let cover = [red, green, 1.0 - red - green].map(|v| (v * brightness).clamp(0.0, 1.0).powf(2.2));
    let mut expected = [[0; 3]; 3];
    for (i, selector) in [0.0, 0.5, 1.0].into_iter().enumerate() {
        let linear: [f32; 3] =
            std::array::from_fn(|c| cover[c] * (1.0 - selector) + baked[c] * selector);
        expected[i] = linear.map(|v| {
            let srgb = if v <= 0.0031308 {
                12.92 * v
            } else {
                1.055 * v.powf(1.0 / 2.4) - 0.055
            };
            (srgb * 255.0).round() as u8
        });
        for shader in [true, false] {
            let mut mesh = Mesh::from(Rectangle::new(1.5, 1.5));
            mesh.insert_attribute(
                Mesh::ATTRIBUTE_COLOR,
                vec![
                    if shader {
                        [baked[0], baked[1], baked[2], selector]
                    } else {
                        [1.0; 4]
                    };
                    4
                ],
            );
            commands.spawn((
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(materials.add(VoxelMaterial {
                    base: StandardMaterial {
                        unlit: true,
                        base_color: if shader {
                            Color::WHITE
                        } else {
                            Color::linear_rgb(linear[0], linear[1], linear[2])
                        },
                        ..default()
                    },
                    extension: VoxelExtension::new(shader.then(|| image.clone())),
                })),
                Transform::from_xyz((i as f32 - 1.0) * 2.0, if shader { 1.0 } else { -1.0 }, 0.0),
            ));
        }
    }
    info!("native cover numeric control raw={raw:?} linear={cover:?} expected_srgb={expected:?}");
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
    commands.spawn((Text::new("Original cover / halfway selector / original baked RGB\nTop: native shader; bottom: independent numeric linear controls\nEncoded cover texel from original cell 21,8 / node 13,0"), TextFont { font_size: FontSize::Px(20.0), ..default() }, TextColor(Color::WHITE), Node { position_type: PositionType::Absolute, top: px(12.0), left: px(24.0), ..default() }));
}

fn pixels(shot: On<ScreenshotCaptured>, controls: Res<Controls>) {
    let image = shot.image.clone().try_into_dynamic().unwrap().to_rgb8();
    let (w, h) = image.dimensions();
    for i in 0..3 {
        let x = (w as i32 / 2 + (i as i32 - 1) * h as i32 / 3) as u32;
        let shader = image.get_pixel(x, h / 3).0;
        let cpu = image.get_pixel(x, 2 * h / 3).0;
        for c in 0..3 {
            assert!(
                shader[c].abs_diff(cpu[c]) <= 1,
                "cover selector {i} shader={shader:?} CPU={cpu:?}"
            );
            assert!(
                shader[c].abs_diff(controls.0[i][c]) <= 1,
                "cover selector {i} shader={shader:?} expected={:?}",
                controls.0[i]
            );
        }
        info!("native voxel GPU control case={i} PASSED: shader={shader:?} CPU={cpu:?}");
    }
}

fn layer_controls(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<VoxelMaterial>>,
) {
    use aa_cry::material::{Material, TextureMap};
    use aa_cry::texture::Dds;
    use bevy::mesh::VertexAttributeValues;
    let mtl = Material::load("materials/aa_terrain/canyon/tr_canyon_rock3d_406b.mtl").unwrap();
    let mtl = &mtl.sub_materials[0];
    let bytes = aa_cry::read(&mtl.texture(&TextureMap::Diffuse).unwrap().pak_path()).unwrap();
    let source = Dds::parse(&bytes).unwrap();
    let rgba = source.decode_rgba8(0, 0);
    let raw = &rgba[..4];
    let image = images.add(Image::new_fill(
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        raw,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::default(),
    ));
    let strength = mtl
        .public_param("DetailTextureStrength")
        .unwrap()
        .parse::<f32>()
        .unwrap();
    let baked = Vec3::new(21.0, 14.0, 11.0) / 255.0;
    let gamma = baked.powf(1.0 / 2.2);
    let detail = Vec3::new(raw[0] as f32, raw[1] as f32, raw[2] as f32) / 255.0;
    let colour = (gamma + (detail - Vec3::splat(0.5)) * strength)
        .clamp(Vec3::ZERO, Vec3::ONE)
        .powf(2.2)
        * mtl.diffuse;
    let camera = Vec3::new(0.0, 0.0, 96.0);
    let mut expected = [[0; 3]; 3];
    for (i, mode) in [0, 1, 2].into_iter().enumerate() {
        let center = Vec3::new((i as f32 - 1.0) * 2.0, 1.0, 0.0);
        let mut mesh = Mesh::from(Rectangle::new(1.5, 1.5));
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_COLOR,
            vec![[baked.x, baked.y, baked.z, 1.0]; 4],
        );
        let base = meshes.add(mesh.clone());
        let base_mat = materials.add(VoxelMaterial {
            base: StandardMaterial {
                unlit: true,
                ..default()
            },
            extension: VoxelExtension::new(None),
        });
        commands.spawn((
            Mesh3d(base),
            MeshMaterial3d(base_mat),
            Transform::from_translation(center),
        ));
        let Some(VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!()
        };
        let weights: Vec<_> = positions
            .iter()
            .map(|p| {
                if mode == 0 {
                    0.0
                } else if mode == 2 || p[0] > 0.0 {
                    1.0
                } else {
                    0.0
                }
            })
            .collect();
        let tri: Vec<_> = mesh.indices().unwrap().iter().take(3).collect();
        let p: Vec<_> = tri
            .iter()
            .map(|&index| Vec2::new(positions[index][0], positions[index][1]))
            .collect();
        let v0 = p[1] - p[0];
        let v1 = p[2] - p[0];
        let v2 = -p[0];
        let denominator = v0.dot(v0) * v1.dot(v1) - v0.dot(v1).powi(2);
        let b = (v1.dot(v1) * v2.dot(v0) - v0.dot(v1) * v2.dot(v1)) / denominator;
        let c = (v0.dot(v0) * v2.dot(v1) - v0.dot(v1) * v2.dot(v0)) / denominator;
        let bary = [1.0 - b - c, b, c];
        let weight: f32 = tri
            .iter()
            .zip(bary)
            .map(|(&index, bary)| {
                let point = Vec3::from_array(positions[index]) + center;
                let fade = 1.0 - (point.distance(camera) / 128.0).min(1.0).powi(4);
                bary * weights[index] * fade
            })
            .sum();
        let alpha = weight.clamp(0.0, 1.0).sqrt();
        let linear = baked.lerp(colour, alpha);
        expected[i] = linear.to_array().map(|v| {
            ((if v <= 0.0031308 {
                v * 12.92
            } else {
                1.055 * v.powf(1.0 / 2.4) - 0.055
            }) * 255.0)
                .round() as u8
        });
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_UV_1,
            weights.into_iter().map(|w| [w, 128.0]).collect::<Vec<_>>(),
        );
        let mut extension = VoxelExtension::new(None);
        extension.diffuse = Some(image.clone());
        extension.params.flags = Vec4::new(0.0, 1.0, strength, 128.0);
        extension.params.colour = mtl.diffuse.extend(1.0);
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
        let mut cpu = Mesh::from(Rectangle::new(1.5, 1.5));
        cpu.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0; 4]; 4]);
        commands.spawn((
            Mesh3d(meshes.add(cpu)),
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
            "native layer GPU control mode={mode} original_texel={raw:?} strength={strength} material_colour={:?} vertex_weight_fade={weight} alpha={alpha} expected_srgb={:?}",
            mtl.diffuse, expected[i]
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
        Transform::from_translation(camera).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((Text::new("Native projected diffuse layer: zero / mixed / full surface weight\nTop: original shader formula with vertex distance fade; bottom: independent CPU controls\nOriginal rock diffuse texel + strength + material colour; native 128 m fade, camera at 96 m"),TextFont { font_size:FontSize::Px(20.0),..default() },TextColor(Color::WHITE),Node { position_type:PositionType::Absolute,top:px(12.0),left:px(24.0),..default() }));
}
