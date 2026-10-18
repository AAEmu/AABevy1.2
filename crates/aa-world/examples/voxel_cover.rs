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
    let four = u16::from_le_bytes(block[0..2].try_into().unwrap()) > u16::from_le_bytes(block[2..4].try_into().unwrap());
    let rgb = match index {
        0 => a,
        1 => b,
        2 if !four => std::array::from_fn(|i| ((a[i] as u16 + b[i] as u16) / 2) as u8),
        3 if !four => [0;3],
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
        info!(
            "native cover GPU control selector={} PASSED: shader={shader:?} CPU={cpu:?}",
            i as f32 * 0.5
        );
    }
}
