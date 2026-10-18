//! Native five-bit terrain materials. Default: all 32 GPU table slots. --legacy aliases
//! high slots to their former low nibble. --world [--forest] [--legacy] [--unload-check].
use aa_world::terrain_layers::{TerrainExt, TerrainMaterial, TerrainMaterialPlugin, TerrainParams};
use bevy::{
    asset::{AssetId, RenderAssetUsages},
    core_pipeline::tonemapping::{DebandDither, Tonemapping},
    image::ImageSampler,
    prelude::*,
    render::{
        render_resource::{
            Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
        },
        view::screenshot::ScreenshotCaptured,
    },
};
use std::collections::HashSet;
fn flag(s: &str) -> bool {
    std::env::args().any(|a| a == s)
}
fn palette(s: usize) -> [u8; 4] {
    [16 + 7 * s as u8, 230 - 5 * s as u8, 50 + 3 * s as u8, 255]
}
fn alias(params: &mut TerrainParams) {
    let original = params.layers;
    for sid in 16..32 {
        params.layers[sid * 12..(sid + 1) * 12]
            .copy_from_slice(&original[(sid & 15) * 12..((sid & 15) + 1) * 12]);
    }
}
#[derive(Resource)]
struct Controls(Vec<(Vec2, [u8; 3])>);
#[derive(Resource, Default)]
struct State {
    mats: HashSet<AssetId<TerrainMaterial>>,
    meshes: HashSet<AssetId<Mesh>>,
    images: HashSet<AssetId<Image>>,
    moved: bool,
    verified: bool,
    last: f32,
}
fn main() {
    let mut app = App::new();
    app.add_plugins((
        aa_dev::default_plugins("Native five-bit terrain surfaces"),
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
) {
    let mut bytes = Vec::new();
    for sid in 0..32 {
        for _ in 0..16 {
            bytes.extend_from_slice(&palette(sid));
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 32,
        },
        TextureDimension::D2,
        bytes,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..default()
    });
    image.sampler = ImageSampler::nearest();
    let array = images.add(image);
    let mut controls = Vec::new();
    for sid in 0..32 {
        let center = Vec2::new((sid % 8) as f32 * 1.5 - 5.25, 2.25 - (sid / 8) as f32 * 1.5);
        let mut params = TerrainParams::new(Vec3::ONE);
        params.tint.w = 1e9;
        for slot in 0..31 {
            params.layers[slot * 12 + 8] = Vec4::new(slot as f32, -1.0, 0.0, 0.0);
            params.layers[slot * 12 + 9] = Vec4::new(0.0, 0.0, 1.0, 0.0);
        }
        if flag("--legacy") {
            alias(&mut params);
        }
        let mut mesh = Mesh::from(Rectangle::new(1.25, 1.2));
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; 4]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0]; 4]);
        commands.spawn((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(materials.add(TerrainMaterial {
                base: StandardMaterial {
                    base_color: Color::linear_rgb(82.0 / 255.0, 85.0 / 255.0, 128.0 / 255.0),
                    unlit: true,
                    ..default()
                },
                extension: TerrainExt::new(
                    params,
                    std::array::from_fn(|_| array.clone()),
                    images.add(aa_world::terrain_layers::surface_image(&[sid as u8])),
                ),
            })),
            Transform::from_xyz(center.x, center.y, 0.0),
        ));
        // Independent Common.cfi cover and terrain.cfx layer blend at full native vertex weight.
        let r = (82.0_f64 / 255.0 + 0.001012) * 31.0 / 30.0;
        let g = 85.0 / 255.0;
        let light = 128.0 / 255.0;
        let mut gamma = [r, g, 1.0 - r - g].map(|v| (v * 3.0 * light).clamp(0.0, 1.0));
        let selected = if flag("--legacy") { sid & 15 } else { sid };
        if selected != 31 {
            let texel = palette(selected);
            for c in 0..3 {
                gamma[c] = (gamma[c] + f64::from(texel[c]) / 255.0 - 0.5).clamp(0.0, 1.0);
            }
        }
        let expected = gamma.map(|v| {
            let v = v.powf(2.2);
            ((if v <= 0.0031308 {
                v * 12.92
            } else {
                1.055 * v.powf(1.0 / 2.4) - 0.055
            }) * 255.0)
                .round() as u8
        });
        controls.push((center, expected));
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
                viewport_height: 8.0,
            },
            ..OrthographicProjection::default_3d()
        }),
    ));
    commands.spawn((Text::new("Native terrain surface slots: rows 0-7 / 8-15 / 16-23 / 24-31. Slot31 is a hole: cover only.\nControlled palette, native table indexing and existing terrain blend; --legacy restores low-nibble alias."),TextFont{font_size:FontSize::Px(18.0),..default()},Node{position_type:PositionType::Absolute,left:px(12.0),top:px(10.0),..default()}));
}
fn pixels(shot: On<ScreenshotCaptured>, controls: Res<Controls>) {
    let image = shot.image.clone().try_into_dynamic().unwrap().to_rgb8();
    let (w, h) = image.dimensions();
    for (sid, (p, cpu)) in controls.0.iter().enumerate() {
        let x = (w as f32 * 0.5 + p.x * h as f32 / 8.0) as u32;
        let y = (h as f32 * 0.5 - p.y * h as f32 / 8.0) as u32;
        let gpu = image.get_pixel(x, y).0;
        assert!(
            gpu.iter().zip(cpu).all(|(a, b)| a.abs_diff(*b) <= 2),
            "Native slot{sid} GPU{gpu:?} expected{cpu:?}"
        );
        info!("Native surface slot{sid} pixel PASSED GPU{gpu:?} expected{cpu:?}");
    }
}
fn world_camera(mut commands: Commands, terrain: Res<aa_world::TerrainQuery>) {
    let target = if flag("--forest") {
        Vec3::new(12948.0, 14896.0, 190.68614)
    } else {
        Vec3::new(21114.0, 8574.0, 377.5)
    };
    let h = terrain
        .cell((target.x / 1024.0) as i32, (target.y / 1024.0) as i32)
        .unwrap();
    let ix = ((target.x % 1024.0) / 2.0) as usize;
    let iy = ((target.y % 1024.0) / 2.0) as usize;
    assert_eq!(
        h.surface[iy * 513 + ix],
        if flag("--forest") { 19 } else { 16 }
    );
    let eye = target + Vec3::new(10.0, -14.0, 11.0);
    info!("Native high-ID terrain camera Cry{eye:?} target{target:?}");
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
    mut materials: ResMut<Assets<TerrainMaterial>>,
    meshes: Res<Assets<Mesh>>,
    images: Res<Assets<Image>>,
    mut camera: Query<&mut Transform, With<Camera3d>>,
) {
    for (mesh, handle) in &patches {
        state.meshes.insert(mesh.id());
        if state.mats.insert(handle.id())
            && let Some(mut material) = materials.get_mut(handle.id())
        {
            state.images.insert(material.extension.surface.id());
            if flag("--legacy") {
                alias(&mut material.extension.params);
            }
        }
    }
    if time.elapsed_secs() > state.last + 2.0 {
        state.last = time.elapsed_secs();
        info!(
            "High-ID terrain: loaded={} patches={} cell_materials={}",
            streamer.loaded_count(),
            patches.iter().count(),
            state.mats.len()
        );
    }
    if flag("--unload-check") && !state.moved && time.elapsed_secs() > 12.0 {
        assert!(!state.mats.is_empty());
        state.moved = true;
        camera.single_mut().unwrap().translation =
            aa_world::cry_to_bevy(Vec3::new(-100000.0, -100000.0, 1000.0));
    }
    if state.moved && !state.verified && time.elapsed_secs() > 17.0 {
        assert_eq!(streamer.loaded_count(), 0);
        assert!(!streamer.is_loading());
        assert!(patches.is_empty());
        assert!(state.mats.iter().all(|id| materials.get(*id).is_none()));
        assert!(state.meshes.iter().all(|id| meshes.get(*id).is_none()));
        assert!(state.images.iter().all(|id| images.get(*id).is_none()));
        state.verified = true;
        info!(
            "High-ID terrain unload PASSED: cells/tasks/patches/materials/surface maps/meshes = 0; shared world arrays retained"
        );
    }
}
