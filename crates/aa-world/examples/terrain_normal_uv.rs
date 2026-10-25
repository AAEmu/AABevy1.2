//! Native independent terrain bump UVs. Actual ATI2 texels + native/legacy numeric controls;
//! --world [--rock] [--legacy] [--positive-v] [--unload-check].
//! --positive-v mirrors texture coordinates only; it retains the native signed normal frame.
use aa_cry::{
    material::{Material as CryMaterial, TextureMap},
    texture::{Dds, DdsFormat},
};
use aa_world::terrain_layers::{
    LayerLibrary, TerrainExt, TerrainMaterial, TerrainMaterialPlugin, TerrainParams,
};
use bevy::{
    asset::AssetId,
    core_pipeline::tonemapping::{DebandDither, Tonemapping},
    image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor},
    prelude::*,
    render::view::screenshot::ScreenshotCaptured,
};
use std::collections::HashSet;
fn flag(s: &str) -> bool {
    std::env::args().any(|a| a == s)
}
#[derive(Resource)]
struct Controls(Vec<(Vec2, [u8; 3])>);
#[derive(Resource, Default)]
struct State {
    mats: HashSet<AssetId<TerrainMaterial>>,
    meshes: HashSet<AssetId<Mesh>>,
    images: HashSet<AssetId<Image>>,
    moved: bool,
    done: bool,
}
fn main() {
    let mut app = App::new();
    app.add_plugins((
        aa_dev::default_plugins("Native terrain normal UVs"),
        aa_dev::DevPlugin,
    ));
    if flag("--world") {
        let mut config = aa_world::WorldConfig::new("main_world");
        config.load_radius = 180.0;
        config.unload_radius = 350.0;
        config.objects = false;
        config.vegetation = false;
        app.add_plugins(aa_world::AaWorldPlugin { config })
            .insert_resource(aa_world::WorldClock {
                hour: 10.0,
                ..default()
            })
            .init_resource::<State>()
            .add_systems(Startup, world_camera)
            .add_systems(Update, world_check);
    } else {
        app.add_plugins(TerrainMaterialPlugin)
            .insert_resource(ClearColor(Color::BLACK))
            .add_systems(Startup, numeric)
            .add_observer(pixels);
    }
    app.run();
}
fn numeric(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<TerrainMaterial>>,
    mut standards: ResMut<Assets<StandardMaterial>>,
) {
    let built = LayerLibrary::build("main_world", [(15, 15), (21, 8)].into_iter()).unwrap();
    let arrays = built.images.map(|mut image| {
        image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::Repeat,
            address_mode_v: ImageAddressMode::Repeat,
            mag_filter: ImageFilterMode::Nearest,
            min_filter: ImageFilterMode::Nearest,
            mipmap_filter: ImageFilterMode::Nearest,
            ..default()
        });
        images.add(image)
    });
    let surface = images.add(aa_world::terrain_layers::surface_image(&[0]));
    let mut controls = Vec::new();
    for (row, (cell, sid, p)) in [
        ((15, 15), 6, 2),
        ((15, 15), 8, 2),
        ((15, 15), 9, 2),
        ((21, 8), 7, 0),
        ((21, 8), 7, 1),
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
        let material = CryMaterial::load(&format!("{}.mtl", entry.material)).unwrap();
        let m = if material.sub_materials.len() == 3 {
            &material.sub_materials[p]
        } else {
            &material
        };
        let bump = m
            .texture(&TextureMap::Normal)
            .or_else(|| m.texture(&TextureMap::Bump))
            .unwrap();
        let diffuse = m.texture(&TextureMap::Diffuse).unwrap();
        let tile = |t: &aa_cry::material::Texture| {
            t.tex_mod
                .as_ref()
                .map_or(Vec2::ONE, |m| Vec2::new(m.tile_u, m.tile_v))
        };
        let normal_tile = tile(bump);
        let diffuse_tile = tile(diffuse);
        let bytes = aa_cry::read(&bump.pak_path()).unwrap();
        let mut dds = Dds::parse(&bytes).unwrap();
        assert_eq!(dds.format, DdsFormat::Bc5);
        dds.swapped_rg = false;
        let mip = if dds.width > 512 {
            (dds.width / 512).trailing_zeros()
        } else {
            0
        };
        let size = dds.width >> mip;
        let texels = dds.decode_rgba8(0, mip);
        // Stay away from nearest-sampler discontinuities while exercising noninteger world phase.
        let local = Vec2::new(12.3731, 91.6237);
        let source = [cell.0 as f64 * 1024.0, cell.1 as f64 * 1024.0];
        let origin = match p {
            0 => [source[1], 0.0],
            1 => [source[0], 0.0],
            _ => source,
        };
        let from = (sid * 3 + p) * 4;
        let a = original.layers[from];
        let b = original.layers[from + 1];
        let diffuse_uv = local * a.zw() + b.xy();
        for column in 0..2 {
            let center = Vec2::new(column as f32 * 3.0 - 1.5, 3.7 - row as f32 * 1.85);
            let mut params = TerrainParams::new(Vec3::ONE);
            params.inspect_normal = 1;
            params.tint.w = 1e9;
            params.detail_distance_z = 1e9;
            let to = p * 4;
            params.layers[to..to + 4].copy_from_slice(&original.layers[from..from + 4]);
            params.layers[to].z = 0.0;
            params.layers[to].w = 0.0;
            params.layers[to + 1].x = diffuse_uv.x;
            params.layers[to + 1].y = diffuse_uv.y;
            params.layers[to + 2].x = -1.0; // Isolate the base normal; production detail remains untouched.
            if column == 0 {
                params.normal_uv[p] = original.normal_uv[sid * 3 + p];
            }
            let uv = Vec2::from_array(std::array::from_fn(|axis| {
                let tile = if column == 0 {
                    normal_tile[axis]
                } else {
                    diffuse_tile[axis]
                };
                let sign = if axis == 0 { 1.0 } else { -1.0 };
                ((origin[axis] + f64::from(local[axis]))
                    * f64::from(sign * entry.scale[axis] * tile))
                .rem_euclid(1.0) as f32
            }));
            let point = (uv * size as f32)
                .floor()
                .as_uvec2()
                .min(UVec2::splat(size - 1));
            let i = ((point.y * size + point.x) * 4) as usize;
            // Native ATI2 GetNormalMap reads raw YX; reconstruct Z before the native frame.
            let xy = Vec2::new(texels[i + 1] as f32, texels[i] as f32) * (2.0 / 255.0) - Vec2::ONE;
            let z = (1.0 - xy.length_squared()).max(0.0).sqrt();
            let n = match p {
                0 => Vec3::new(z, -xy.y, -xy.x),
                1 => Vec3::new(xy.x, -xy.y, -z),
                _ => Vec3::new(xy.x, z, xy.y),
            }
            .normalize();
            let linear = n * 0.5 + Vec3::splat(0.5);
            let expected = linear.to_array().map(|v| {
                ((if v <= 0.0031308 {
                    v * 12.92
                } else {
                    1.055 * v.powf(1.0 / 2.4) - 0.055
                }) * 255.0)
                    .round() as u8
            });
            let normal = match p {
                0 => [1.0, 0.0, 0.0],
                1 => [0.0, 0.0, -1.0],
                _ => [0.0, 1.0, 0.0],
            };
            let mut mesh = Mesh::from(Rectangle::new(2.4, 1.3));
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![normal; 4]);
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0]; 4]);
            commands.spawn((
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(materials.add(TerrainMaterial {
                    base: StandardMaterial {
                        unlit: true,
                        ..default()
                    },
                    extension: TerrainExt::new(params, arrays.clone(), surface.clone()),
                })),
                Transform::from_xyz(center.x, center.y, 0.0),
            ));
            commands.spawn((
                Mesh3d(meshes.add(Rectangle::new(2.4, 0.16))),
                MeshMaterial3d(standards.add(StandardMaterial {
                    base_color: Color::linear_rgb(linear.x, linear.y, linear.z),
                    unlit: true,
                    ..default()
                })),
                Transform::from_xyz(center.x, center.y - 0.8, 0.0),
            ));
            info!(
                "Normal UV row{row} col{column} path={} native_uv={uv:?} texel={point:?} expected{expected:?}",
                bump.pak_path()
            );
            controls.push((center, expected));
        }
    }
    commands.insert_resource(Controls(controls));
    commands.spawn((
        Camera3d::default(),
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
        Transform::from_xyz(0.0, 0.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: bevy::camera::ScalingMode::FixedVertical {
                viewport_height: 11.0,
            },
            ..OrthographicProjection::default_3d()
        }),
    ));
    commands.spawn((Text::new("Native signed normal UV / former diffuse UV. Original ATI2 texels + CPU swatches.\nRows: wet sand 5x / coral 2.5x / dry sand .5x / rock X,Y .7x. All three signed projection frames."),TextFont{font_size:FontSize::Px(18.0),..default()},Node{position_type:PositionType::Absolute,left:px(12.0),top:px(10.0),..default()}));
}
fn pixels(shot: On<ScreenshotCaptured>, controls: Res<Controls>) {
    let image = shot.image.clone().try_into_dynamic().unwrap().to_rgb8();
    let (w, h) = image.dimensions();
    for (i, (p, cpu)) in controls.0.iter().enumerate() {
        let gpu = image
            .get_pixel(
                (w as f32 * 0.5 + p.x * h as f32 / 11.0) as u32,
                (h as f32 * 0.5 - p.y * h as f32 / 11.0) as u32,
            )
            .0;
        assert!(
            gpu.iter().zip(cpu).all(|(a, b)| a.abs_diff(*b) <= 3),
            "normal UV{i} GPU{gpu:?} CPU{cpu:?}"
        );
        info!("Native normal UV{i} PASSED GPU{gpu:?} CPU{cpu:?}");
    }
}
fn world_camera(mut commands: Commands, terrain: Res<aa_world::TerrainQuery>) {
    // Original owned 5x5 SID regions, selected from heightmap.dat; camera sits outside the sampled face.
    let target = if flag("--rock") {
        Vec3::new(22162.0, 8686.0, 605.37744)
    } else {
        Vec3::new(13822.0, 10878.0, 166.29433)
    };
    let cell = terrain
        .cell((target.x / 1024.0) as i32, (target.y / 1024.0) as i32)
        .unwrap();
    let (ix, iy) = (
        ((target.x % 1024.0) / 2.0) as usize,
        ((target.y % 1024.0) / 2.0) as usize,
    );
    assert_eq!(
        cell.surface[iy * 513 + ix],
        if flag("--rock") { 7 } else { 8 }
    );
    let normal = terrain.normal_at(target.x, target.y).unwrap();
    let eye = if flag("--rock") {
        target + normal * 18.0 + Vec3::Z * 4.0
    } else {
        target + Vec3::new(10.0, -14.0, 11.0)
    };
    info!("Original normal-UV camera Cry{eye:?} target{target:?}");
    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(aa_world::cry_to_bevy(eye))
            .looking_at(aa_world::cry_to_bevy(target), Vec3::Y),
        Projection::Perspective(PerspectiveProjection {
            fov: 50_f32.to_radians(),
            near: 0.1,
            far: 20000.0,
            ..default()
        }),
    ));
}
fn world_check(
    time: Res<Time>,
    mut state: ResMut<State>,
    streamer: Res<aa_world::stream::Streamer>,
    patches: Query<
        (&Mesh3d, &MeshMaterial3d<TerrainMaterial>),
        With<aa_world::stream::TerrainPatch>,
    >,
    mut mats: ResMut<Assets<TerrainMaterial>>,
    meshes: Res<Assets<Mesh>>,
    images: Res<Assets<Image>>,
    mut camera: Query<&mut Transform, With<Camera3d>>,
) {
    for (mesh, handle) in &patches {
        state.meshes.insert(mesh.id());
        if state.mats.insert(handle.id())
            && let Some(mut m) = mats.get_mut(handle.id())
        {
            state.images.insert(m.extension.surface.id());
            if flag("--positive-v") {
                // UV-only control; the production native frame stays unchanged.
                let params = &mut m.extension.params;
                for p in 0..params.normal_uv.len() {
                    let e = p * 4;
                    let ratio = params.normal_uv[p].y;
                    let normal_offset = params.layers[e + 1].y * ratio + params.normal_uv[p].w;
                    params.layers[e].w = -params.layers[e].w;
                    params.layers[e + 1].y = (-params.layers[e + 1].y).rem_euclid(1.0);
                    params.normal_uv[p].w =
                        (-normal_offset).rem_euclid(1.0) - params.layers[e + 1].y * ratio;
                    params.layers[e + 2].z = -params.layers[e + 2].z;
                    params.layers[e + 3].y = (-params.layers[e + 3].y).rem_euclid(1.0);
                }
            }
            if flag("--legacy") {
                m.extension
                    .params
                    .normal_uv
                    .fill(Vec4::new(1.0, 1.0, 0.0, 0.0));
            }
        }
    }
    if flag("--unload-check") && !state.moved && time.elapsed_secs() > 12.0 {
        assert!(!state.mats.is_empty());
        state.moved = true;
        camera.single_mut().unwrap().translation =
            aa_world::cry_to_bevy(Vec3::new(-100000.0, -100000.0, 1000.0));
    }
    if state.moved && !state.done && time.elapsed_secs() > 17.0 {
        assert_eq!(streamer.loaded_count(), 0);
        assert!(!streamer.is_loading());
        assert!(patches.is_empty());
        assert!(state.mats.iter().all(|id| mats.get(*id).is_none()));
        assert!(state.meshes.iter().all(|id| meshes.get(*id).is_none()));
        assert!(state.images.iter().all(|id| images.get(*id).is_none()));
        state.done = true;
        info!("Normal UV unload PASSED: cells/tasks/patches/materials/surface maps/meshes=0");
    }
}
