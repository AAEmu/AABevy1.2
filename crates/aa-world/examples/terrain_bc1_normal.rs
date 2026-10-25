//! Original BC1 terrain XYZ, copied-source routing and raw RG detail controls.
//! --world [--off] [--unload-check] selects the authored Palace dirt001b receiver.
use aa_cry::texture::{Dds, DdsFormat};
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
        aa_dev::default_plugins("Native BC1 terrain normals"),
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
    let built = LayerLibrary::build("main_world", [(13, 10), (12, 14)].into_iter()).unwrap();
    let arrays = built.images.map(|mut image| {
        image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::Repeat,
            address_mode_v: ImageAddressMode::Repeat,
            min_filter: ImageFilterMode::Nearest,
            mag_filter: ImageFilterMode::Nearest,
            mipmap_filter: ImageFilterMode::Nearest,
            ..default()
        });
        images.add(image)
    });
    let surface = images.add(aa_world::terrain_layers::surface_image(&[0]));
    let mut controls = Vec::new();
    for (row, (cell, sid, source_axis, path, point)) in [
        (
            (13, 10),
            6,
            0,
            "canyon/tr_canyon_rock3d_703a_ddn",
            UVec2::new(137, 68),
        ),
        (
            (13, 10),
            0,
            2,
            "field/tr_field_dirt_001a_ddn",
            UVec2::new(10, 311),
        ),
        (
            (13, 10),
            8,
            2,
            "field/tr_field_dirt_001b_ddn",
            UVec2::new(208, 428),
        ),
        (
            (12, 14),
            3,
            2,
            "wasteland/w_forest_white_sand_a_ddn",
            UVec2::new(336, 250),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let original = built.library.params(cell, Vec3::ONE).unwrap();
        let source = (sid * 3 + source_axis) * 4;
        let code = original.layers[source].y;
        assert_eq!((code / 1000.0).floor() as u32, 8);
        let bytes = aa_cry::read(&format!("textures/aa_terrain/{path}.dds")).unwrap();
        let dds = Dds::parse(&bytes).unwrap();
        assert_eq!(dds.format, DdsFormat::Bc1);
        assert_eq!(dds.width, 512);
        let texels = dds.decode_rgba8(0, 0);
        let i = ((point.y * dds.width + point.x) * 4) as usize;
        let raw = Vec3::new(texels[i] as f32, texels[i + 1] as f32, texels[i + 2] as f32) / 255.0;
        let uv = (point.as_vec2() + Vec2::splat(0.5)) / 512.0;
        for column in 0..4 {
            let p = [0, 1, 2, 2][column];
            let copied = column == 1;
            let detail = if column == 3 { 0.7 } else { 0.0 };
            let center = Vec2::new(column as f32 * 2.2 - 3.3, 2.8 - row as f32 * 1.85);
            let mut params = TerrainParams::new(Vec3::ONE);
            params.inspect_normal = 1;
            params.tint.w = 1e9;
            params.detail_distance_z = 1e9;
            let e = p * 4;
            params.layers[e..e + 4].copy_from_slice(&original.layers[source..source + 4]);
            params.layers[e].y = code + if copied { 1000.0 } else { 0.0 };
            params.layers[e].z = 0.0;
            params.layers[e].w = 0.0;
            let base_uv = uv * if copied { 2.0 } else { 1.0 };
            params.layers[e + 1].x = base_uv.x;
            params.layers[e + 1].y = base_uv.y;
            params.layers[e + 1].w = detail;
            params.layers[e + 2].x = if detail != 0.0 { code } else { -1.0 };
            params.layers[e + 2].y = 0.0;
            params.layers[e + 2].z = 0.0;
            params.layers[e + 3].x = uv.x;
            params.layers[e + 3].y = uv.y;
            let bump = raw * 2.0 - Vec3::ONE;
            // Common.cfi retains BC1 XYZ; FragLib adds raw RG detail after base Z selection.
            let v = Vec3::new(bump.x * (1.0 + detail), bump.y * (1.0 + detail), bump.z);
            let n = match p {
                0 => Vec3::new(v.z, -v.y, -v.x),
                1 => Vec3::new(v.x, -v.y, -v.z),
                _ => Vec3::new(v.x, v.z, v.y),
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
            let mut mesh = Mesh::from(Rectangle::new(1.8, 1.3));
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
                Mesh3d(meshes.add(Rectangle::new(1.8, 0.16))),
                MeshMaterial3d(standards.add(StandardMaterial {
                    base_color: Color::linear_rgb(linear.x, linear.y, linear.z),
                    unlit: true,
                    ..default()
                })),
                Transform::from_xyz(center.x, center.y - 0.8, 0.0),
            ));
            controls.push((center, expected));
            info!("BC1 row{row} col{column} {path} raw{raw:?} expected{expected:?}");
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
                viewport_height: 9.0,
            },
            ..OrthographicProjection::default_3d()
        }),
    ));
    commands.spawn((Text::new("Original BC1 XYZ normals / independent CPU swatches. Four actual rock/dirt/sand textures.\nColumns: signed X / Y + copy flag / Z / Z + raw RG detail .7. No reconstructed base Z."),TextFont{font_size:FontSize::Px(18.0),..default()},
        Node{position_type:PositionType::Absolute,left:px(12.0),top:px(10.0),..default()}));
}
fn pixels(shot: On<ScreenshotCaptured>, controls: Res<Controls>) {
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
            "BC1 normal{i}: GPU{gpu:?} CPU{cpu:?}"
        );
        info!("BC1 normal{i} PASSED GPU{gpu:?} CPU{cpu:?}");
    }
}
fn world_camera(mut commands: Commands, terrain: Res<aa_world::TerrainQuery>) {
    let target = Vec3::new(13822.0, 10878.0, 166.29433);
    let eye = target + Vec3::new(10.0, -14.0, 11.0);
    let cell = terrain.cell(13, 10).unwrap();
    assert_eq!(cell.surface[319 * 513 + 255], 8);
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
            if flag("--off") {
                for entry in m.extension.params.layers.chunks_mut(4) {
                    if entry[0].y >= 0.0 && ((entry[0].y / 1000.0).floor() as u32) & 8 != 0 {
                        entry[0].y = -1.0;
                    }
                }
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
        info!("BC1 normal unload PASSED: cells/tasks/patches/materials/surface maps/meshes=0");
    }
}
