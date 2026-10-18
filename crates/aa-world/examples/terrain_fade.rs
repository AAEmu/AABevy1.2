//! Original terrain layer distance fade: --legacy numeric expectations for the old build only.
//! --world [--unload-check]: original forest cell12,14 from800m above, unchanged environment.
use aa_world::terrain_layers::{TerrainExt, TerrainMaterial, TerrainMaterialPlugin, TerrainParams};
use bevy::{
    asset::RenderAssetUsages,
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

#[derive(Resource)]
struct Controls(Vec<(Vec2, [u8; 3])>);
#[derive(Resource, Default)]
struct Fixture {
    unload: bool,
    moved: bool,
    done: bool,
    last: f32,
    parts: HashSet<Entity>,
}
fn flag(v: &str) -> bool {
    std::env::args().any(|a| a == v)
}
fn main() {
    let mut app = App::new();
    app.add_plugins((
        aa_dev::default_plugins("Native terrain layer distance fade"),
        aa_dev::DevPlugin,
    ));
    if flag("--world") {
        let mut config = aa_world::WorldConfig::new("main_world");
        config.load_radius = 650.0;
        config.unload_radius = 950.0;
        config.objects = false;
        config.vegetation = false;
        app.add_plugins(aa_world::AaWorldPlugin { config })
            .insert_resource(aa_world::WorldClock {
                hour: 10.0,
                ..default()
            })
            .insert_resource(Fixture {
                unload: flag("--unload-check"),
                ..default()
            })
            .add_systems(Startup, world_setup)
            .add_systems(Update, world_check);
    } else {
        app.add_plugins(TerrainMaterialPlugin)
            .insert_resource(ClearColor(Color::BLACK))
            .add_systems(Startup, numeric)
            .add_observer(pixels);
    }
    app.run();
}
fn array_image(images: &mut Assets<Image>) -> Handle<Image> {
    let mut image = Image::new_fill(
        Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[255, 0, 64, 255],
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2Array),
        ..default()
    });
    image.sampler = ImageSampler::nearest();
    images.add(image)
}
fn numeric(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut mats: ResMut<Assets<TerrainMaterial>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
    args: Res<aa_dev::DevArgs>,
) {
    let texture = array_image(&mut images);
    let mut controls = Vec::new();
    for row in 0..3 {
        let surface = images.add(Image::new(
            Extent3d {
                width: 2,
                height: 2,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            match row {
                0 => vec![0, 0, 0, 0],
                1 => vec![0, 0, 1, 1],
                _ => vec![0, 1, 1, 1],
            },
            TextureFormat::R8Uint,
            RenderAssetUsages::RENDER_WORLD,
        ));
        for (i, ratio) in [0.0_f32, 0.5, 0.8, 0.9999, 1.1].into_iter().enumerate() {
            let p = Vec2::new(i as f32 * 1.8 - 3.6, 2.2 - row as f32 * 2.2);
            // Match the readback pixel centre, including the near-cutoff regression.
            let (w, h) = (args.size.0 as f32, args.size.1 as f32);
            let x = (w * 0.5 + p.x * h / 8.0).floor();
            let y = (h * 0.5 - p.y * h / 8.0).floor();
            let sample = Vec2::new((x + 0.5 - w * 0.5) * 8.0 / h, (h * 0.5 - y - 0.5) * 8.0 / h);
            let distance = Vec3::new(sample.x, sample.y, -6.0).length();
            let mut params = TerrainParams::new(Vec3::ONE);
            params.tint.w = if ratio == 0.0 { 1e9 } else { distance / ratio };
            let entry = 2 * 4;
            params.layers[entry] = Vec4::new(0.0, -1.0, 1.0, 1.0);
            params.layers[entry + 1] = Vec4::new(0.0, 0.0, 1.0, 0.0);
            let mut mesh = Mesh::from(Rectangle::new(1.5, 0.9));
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; 4]);
            // All fragments sample the same cell-grid midpoint; surface0 has weight1/.5/.25.
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.5 / 512.0, 0.5 / 512.0]; 4]);
            let material = mats.add(TerrainMaterial {
                base: StandardMaterial {
                    base_color: Color::linear_rgb(82.0 / 255.0, 85.0 / 255.0, 128.0 / 255.0),
                    unlit: true,
                    ..default()
                },
                extension: TerrainExt::new(
                    params,
                    std::array::from_fn(|_| texture.clone()),
                    surface.clone(),
                ),
            });
            commands.spawn((
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(material),
                Transform::from_xyz(p.x, p.y, 0.0),
            ));
            let weight = [1.0_f64, 0.5, 0.25][row];
            let fade = 1.0 - (f64::from(ratio).min(1.0)).powi(4);
            let alpha = if flag("--legacy") {
                weight.sqrt() * fade
            } else {
                (weight * fade).sqrt()
            };
            let r = (82.0_f64 / 255.0 + 0.001012) * 31.0 / 30.0;
            let g = 85.0 / 255.0;
            let intensity = 128.0 / 255.0;
            let gamma = [r, g, 1.0 - r - g].map(|v| (v * 3.0 * intensity).clamp(0.0, 1.0));
            let texel = [1.0, 0.0, 64.0 / 255.0];
            let linear = std::array::from_fn::<_, 3, _>(|c| {
                let base = gamma[c].powf(2.2);
                let layer = (gamma[c] + texel[c] - 0.5).clamp(0.0, 1.0).powf(2.2);
                base * (1.0 - alpha) + layer * alpha
            });
            let rgb = linear.map(|v| {
                ((if v <= 0.0031308 {
                    12.92 * v
                } else {
                    1.055 * v.powf(1.0 / 2.4) - 0.055
                }) * 255.0)
                    .round() as u8
            });
            controls.push((p, rgb));
            commands.spawn((
                Mesh3d(meshes.add(Rectangle::new(1.5, 0.22))),
                MeshMaterial3d(standard.add(StandardMaterial {
                    base_color: Color::linear_rgb(
                        linear[0] as f32,
                        linear[1] as f32,
                        linear[2] as f32,
                    ),
                    unlit: true,
                    ..default()
                })),
                Transform::from_xyz(p.x, p.y - 0.65, 0.0),
            ));
        }
    }
    commands.insert_resource(Controls(controls));
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 0.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: bevy::camera::ScalingMode::FixedVertical {
                viewport_height: 8.0,
            },
            ..OrthographicProjection::default_3d()
        }),
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
    ));
    commands.spawn((Text::new("Original terrain layer fade: rows surface weight1 /0.5 /0.25\nColumns: distance/D 0 /0.5 /0.8 /0.9999 /1.1. Thin swatches: independent CPU\nNative sqrt(weight*(1-min(d/D,1)^4)); --legacy expects previous fade outside sqrt"),TextFont{font_size:FontSize::Px(18.0),..default()},Node{position_type:PositionType::Absolute,left:px(20.0),top:px(10.0),..default()}));
}
fn pixels(shot: On<ScreenshotCaptured>, controls: Res<Controls>) {
    let image = shot.image.clone().try_into_dynamic().unwrap().to_rgb8();
    let (w, h) = image.dimensions();
    for (i, (p, cpu)) in controls.0.iter().enumerate() {
        let x = (w as f32 * 0.5 + p.x * h as f32 / 8.0) as u32;
        let y = (h as f32 * 0.5 - p.y * h as f32 / 8.0) as u32;
        let gpu = image.get_pixel(x, y).0;
        assert!(
            gpu.iter().zip(cpu).all(|(a, b)| a.abs_diff(*b) <= 2),
            "terrain fade{i} GPU{gpu:?} CPU{cpu:?}"
        );
        info!("terrain fade GPU case{i} PASSED GPU={gpu:?} CPU={cpu:?}");
    }
}
fn world_setup(mut commands: Commands) {
    let target = Vec3::new(12812.47, 14575.70, 158.27849);
    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(aa_world::cry_to_bevy(target + Vec3::Z * 800.0))
            .looking_at(aa_world::cry_to_bevy(target), -Vec3::Z),
        Projection::Perspective(PerspectiveProjection {
            fov: 45.0_f32.to_radians(),
            near: 0.1,
            far: 20000.0,
            ..default()
        }),
    ));
}
fn world_check(
    time: Res<Time>,
    stream: Res<aa_world::stream::Streamer>,
    patches: Query<(Entity, &aa_world::stream::TerrainPatch)>,
    all: Query<()>,
    mut camera: Query<&mut Transform, With<Camera3d>>,
    mut f: ResMut<Fixture>,
) {
    f.parts.extend(patches.iter().map(|(e, _)| e));
    if time.elapsed_secs() - f.last > 2.0 {
        f.last = time.elapsed_secs();
        info!(
            "terrain fade: cells={} patches={} loading={} seen_patches={}",
            stream.loaded_count(),
            patches.iter().count(),
            stream.is_loading(),
            f.parts.len()
        );
    }
    if f.unload && !f.moved && time.elapsed_secs() > 16.0 {
        assert!(patches.iter().any(|(_, p)| p.cell == (12, 14)));
        assert!(!stream.is_loading());
        camera.single_mut().unwrap().translation = aa_world::cry_to_bevy(Vec3::splat(-100000.0));
        f.moved = true;
    }
    if f.moved && !f.done && time.elapsed_secs() > 21.0 {
        assert!(patches.is_empty());
        assert_eq!(stream.loaded_count(), 0);
        assert!(!stream.is_loading());
        assert!(f.parts.iter().all(|e| all.get(*e).is_err()));
        info!(
            "terrain fade unload PASSED: {} original patch entity IDs gone",
            f.parts.len()
        );
        f.done = true;
    }
}
