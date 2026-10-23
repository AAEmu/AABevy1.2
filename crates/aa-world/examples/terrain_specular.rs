//! Authored terrain gloss/normal/Phong controls. --dark disables the sun; --flat removes normals.
//! --off shows prior lighting; normal mode uses a known 100-unit probe light (not a material gain).
//! --unit uses one unit; --mapless checks the native white-gloss fallback with authored colours/powers.
use aa_cry::{
    material::{Material as CryMaterial, TextureMap},
    texture::{Dds, DdsFormat},
};
use aa_world::terrain_layers::{
    LayerLibrary, TerrainExt, TerrainMaterial, TerrainMaterialPlugin, TerrainParams,
};
use bevy::{
    camera::Exposure,
    core_pipeline::tonemapping::{DebandDither, Tonemapping},
    image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor},
    prelude::*,
    render::view::screenshot::ScreenshotCaptured,
};

fn flag(s: &str) -> bool {
    std::env::args().any(|a| a == s)
}
fn probe_light() -> f32 {
    if flag("--unit") { 1.0 } else { 100.0 }
}
#[derive(Resource)]
struct Controls(Vec<(Vec2, [u8; 3])>);
fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("Authored terrain specular"),
            aa_dev::DevPlugin,
            TerrainMaterialPlugin,
        ))
        .insert_resource(ClearColor(Color::BLACK))
        .add_systems(Startup, setup)
        .add_observer(pixels)
        .run();
}
fn sample(path: &str, uv: Vec2, normal: bool) -> Vec3 {
    let bytes = aa_cry::read(path).unwrap();
    let mut dds = Dds::parse(&bytes).unwrap();
    let swapped = dds.swapped_rg;
    dds.swapped_rg = false;
    let mip = if dds.width > 512 {
        (dds.width / 512).trailing_zeros()
    } else {
        0
    };
    let width = dds.width >> mip;
    let p = (uv.rem_euclid(Vec2::ONE) * width as f32)
        .floor()
        .as_uvec2()
        .min(UVec2::splat(width - 1));
    let rgba = dds.decode_rgba8(0, mip);
    let i = ((p.y * width + p.x) * 4) as usize;
    let mut v = Vec3::new(rgba[i] as f32, rgba[i + 1] as f32, rgba[i + 2] as f32) / 255.0;
    if matches!(dds.format, DdsFormat::Bc1 | DdsFormat::Bc3) {
        // The generic CPU RGBA8 decoder truncates interpolated BC colours. At low
        // gloss values that one-count error is magnified by this 100-unit probe.
        // Decode the original block independently, rounding its interpolation as the GPU does.
        let bytes = if dds.format == DdsFormat::Bc1 { 8 } else { 16 };
        let src = &dds.data[dds.surface_range(0, mip)];
        let b = ((p.y / 4) * (width / 4) + p.x / 4) as usize * bytes + bytes - 8;
        let block = &src[b..b + 8];
        let a = u16::from_le_bytes([block[0], block[1]]);
        let z = u16::from_le_bytes([block[2], block[3]]);
        let unpack = |q: u16| {
            let (r, g, b) = ((q >> 11) & 31, (q >> 5) & 63, q & 31);
            Vec3::new(
                ((r << 3) | (r >> 2)) as f32,
                ((g << 2) | (g >> 4)) as f32,
                ((b << 3) | (b >> 2)) as f32,
            )
        };
        let (first, last) = (unpack(a), unpack(z));
        let bits = u32::from_le_bytes(block[4..8].try_into().unwrap());
        let index = (bits >> (2 * ((p.y % 4) * 4 + p.x % 4))) & 3;
        v = match index {
            0 => first,
            1 => last,
            2 if a > z || bytes == 16 => ((first * 2.0 + last) / 3.0).round(),
            3 if a > z || bytes == 16 => ((first + last * 2.0) / 3.0).round(),
            2 => ((first + last) * 0.5).round(),
            _ => Vec3::ZERO,
        } / 255.0;
    }
    if normal {
        if swapped {
            v = Vec3::new(v.y, v.x, v.z);
        }
        v = v * 2.0 - Vec3::ONE;
        if dds.format != DdsFormat::Bc1 {
            v.z = (1.0 - v.xy().length_squared()).max(0.0).sqrt();
        }
    }
    v
}
fn rgb(v: Vec3) -> [u8; 3] {
    v.to_array().map(|v| {
        ((if v <= 0.0031308 {
            v * 12.92
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        }) * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8
    })
}
fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<TerrainMaterial>>,
) {
    let built =
        LayerLibrary::build("main_world", [(13, 10), (15, 15), (21, 8)].into_iter()).unwrap();
    let arrays = built.images.map(|mut i| {
        i.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::Repeat,
            address_mode_v: ImageAddressMode::Repeat,
            mag_filter: ImageFilterMode::Nearest,
            min_filter: ImageFilterMode::Nearest,
            mipmap_filter: ImageFilterMode::Nearest,
            ..default()
        });
        images.add(i)
    });
    let surface = images.add(aa_world::terrain_layers::surface_image(&[0]));
    let mut controls = Vec::new();
    // Real cells/materials, including the original three-projection rock and a zero-specular projection.
    for (row, (cell, sid, p)) in [
        ((13, 10), 13, 2),
        ((15, 15), 6, 2),
        ((21, 8), 7, 0),
        ((21, 8), 7, 2),
    ]
    .into_iter()
    .enumerate()
    {
        let original = built.library.params(cell, Vec3::ONE).unwrap();
        let xml = aa_pak::Pak::global()
            .read_string(&format!(
                "worlds/main_world/cells/{:03}_{:03}/client/leveldata.xml",
                cell.0, cell.1
            ))
            .unwrap();
        let entries = aa_world::terrain_layers::parse_surface_types(&xml);
        let entry = &entries[sid];
        let mtl = CryMaterial::load(&format!("{}.mtl", entry.material)).unwrap();
        let m = if mtl.sub_materials.len() == 3 {
            &mtl.sub_materials[p]
        } else {
            &mtl
        };
        let local = Vec2::new(12.3731, 91.6237);
        let from = (sid * 3 + p) * 4;
        let si = (sid * 3 + p) * 2;
        let a = original.layers[from];
        let b = original.layers[from + 1];
        let uv = local * a.zw() + b.xy();
        let dt = original.layers[from + 2];
        let dc = original.layers[from + 3];
        let detail_uv = local * dt.yz() + dc.xy();
        let normal_transform = original.normal_uv[sid * 3 + p];
        let normal_uv = uv * normal_transform.xy() + normal_transform.zw();
        let spec = original.specular[si];
        let gloss = if original.specular[si + 1].x >= 0.0 && !flag("--mapless") {
            sample(
                &m.texture(&TextureMap::Specular).unwrap().pak_path(),
                uv,
                false,
            )
        } else {
            Vec3::ONE
        };
        for column in 0..3 {
            let mut bindings = arrays.clone();
            let center = Vec2::new(column as f32 * 3.0 - 3.0, 2.7 - row as f32 * 1.8);
            let mut params = TerrainParams::new(Vec3::ONE);
            params.tint.w = 1e9;
            params.specular_light.y = if flag("--off") { 2.0 } else { 1.0 };
            params.layers[p * 4..p * 4 + 4].copy_from_slice(&original.layers[from..from + 4]);
            params.layers[p * 4].z = 0.0;
            params.layers[p * 4].w = 0.0;
            params.layers[p * 4 + 1].x = uv.x;
            params.layers[p * 4 + 1].y = uv.y;
            params.layers[p * 4 + 2].y = 0.0;
            params.layers[p * 4 + 2].z = 0.0;
            params.layers[p * 4 + 3].x = detail_uv.x;
            params.layers[p * 4 + 3].y = detail_uv.y;
            params.normal_uv[p] = normal_transform;
            params.specular[p * 2..p * 2 + 2].copy_from_slice(&original.specular[si..si + 2]);
            if flag("--mapless") {
                params.specular[p * 2 + 1].x = -1.0;
            } else if row == 2 && column == 1 {
                // Route the same authored map through the second array; this is a metadata control.
                params.specular[p * 2 + 1].x += 2000.0;
                bindings[6] = arrays[5].clone();
            }
            // Third column is matte, including occlusion of an earlier hypothetical highlight.
            if column == 2 {
                params.specular[p * 2] = Vec4::ZERO;
            }
            let flat = flag("--flat") || column == 1;
            if flat {
                params.layers[p * 4].y = -1.0;
            }
            let mut n = match p {
                0 => Vec3::X,
                1 => Vec3::NEG_Z,
                _ => Vec3::Y,
            };
            let mesh_normal = n;
            if !flat && a.y >= 0.0 {
                let normal = m
                    .texture(&TextureMap::Normal)
                    .or_else(|| m.texture(&TextureMap::Bump))
                    .unwrap();
                let mut nt = sample(&normal.pak_path(), normal_uv, true);
                if dt.x >= 0.0 {
                    let detail = sample(
                        &m.texture(&TextureMap::Detail).unwrap().pak_path(),
                        detail_uv,
                        false,
                    ) * 2.0
                        - Vec3::ONE;
                    nt.x += detail.x * b.w;
                    nt.y += detail.y * b.w;
                }
                n = match p {
                    0 => Vec3::new(nt.z, -nt.y, -nt.x),
                    1 => Vec3::new(nt.x, -nt.y, -nt.z),
                    _ => Vec3::new(nt.x, nt.z, nt.y),
                }
                .normalize();
            }
            let light = Vec3::NEG_Z;
            let reflected = -light + 2.0 * n * n.dot(light);
            let phong = Vec3::Z.dot(reflected).clamp(0.0, 1.0).powf(spec.w);
            let expected = if flag("--dark") || column == 2 {
                Vec3::ZERO
            } else {
                spec.xyz() * gloss * phong * Vec3::new(1.0, 0.6, 0.25) * probe_light()
            };
            let mut mesh = Mesh::from(Rectangle::new(2.5, 1.35));
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![mesh_normal.to_array(); 4]);
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0]; 4]);
            commands.spawn((
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(materials.add(TerrainMaterial {
                    base: StandardMaterial {
                        base_color: Color::linear_rgb(82.0 / 255.0, 85.0 / 255.0, 128.0 / 255.0),
                        ..default()
                    },
                    extension: TerrainExt::new(params, bindings, surface.clone()),
                })),
                Transform::from_xyz(center.x, center.y, 0.0),
            ));
            info!(
                "terrain spec row{row} col{column} material={} projection={p} spec={spec:?} gloss={gloss:?} normal={n:?} expected={:?}",
                entry.material,
                rgb(expected)
            );
            controls.push((center, rgb(expected)));
        }
    }
    commands.insert_resource(Controls(controls));
    commands.spawn((
        DirectionalLight {
            color: Color::linear_rgb(1.0, 0.6, 0.25),
            illuminance: if flag("--dark") {
                0.0
            } else {
                std::f32::consts::PI * probe_light()
            },
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_rotation_y(std::f32::consts::PI)),
    ));
    commands.spawn((
        Camera3d::default(),
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
        Exposure {
            ev100: -(1.2_f32).log2(),
        },
        Transform::from_xyz(0.0, 0.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: bevy::camera::ScalingMode::FixedVertical {
                viewport_height: 9.0,
            },
            ..OrthographicProjection::default_3d()
        }),
    ));
    commands.spawn((Text::new("Authored terrain specular: authored normals / flat normals / matte.\nPaving / wet sand / rock X / rock Z. Linear gloss x authored colour x native forward Phong. DDS/CPU checks."),TextFont{font_size:FontSize::Px(17.0),..default()},Node{position_type:PositionType::Absolute,left:px(12.0),top:px(12.0),..default()}));
}
fn pixels(shot: On<ScreenshotCaptured>, controls: Res<Controls>) {
    if flag("--off") {
        return;
    }
    let image = shot.image.clone().try_into_dynamic().unwrap().to_rgb8();
    let (w, h) = image.dimensions();
    for (i, (p, cpu)) in controls.0.iter().enumerate() {
        let gpu = image
            .get_pixel(
                (w as f32 * 0.5 + p.x * h as f32 / 9.0) as u32,
                (h as f32 * 0.5 - p.y * h as f32 / 9.0) as u32,
            )
            .0;
        assert!(
            gpu.iter().zip(cpu).all(|(a, b)| a.abs_diff(*b) <= 3),
            "terrain specular{i} GPU{gpu:?} CPU{cpu:?}"
        );
        info!("terrain specular{i} PASSED GPU{gpu:?} sourceDDS_CPU{cpu:?}");
    }
}
