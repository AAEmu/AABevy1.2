//! Original terrain layer distance fade: --legacy numeric expectations for the old build only.
//! --world [--unload-check]: original forest cell12,14 from800m above, unchanged environment.
//! --quality [--axis-x|--axis-y]: four native projection distance levels (ground/Z by default).
//! --shared-distance: explicit former shared-XY control; --world --quality [--low|--restore].
//! --near: ground quality at96m; --denominator: tiny positive distances and native .001 clamp.
//! --shared-distance --expect-native is an intentional RED control for the former approximation.
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
use std::sync::{Arc, Mutex};

/// Kept outside App as well: the window runner consumes the app before returning.
#[derive(Resource, Clone, Default)]
struct Completion(Arc<Mutex<Completed>>);

#[derive(Clone, Copy, Default)]
struct Completed {
    loaded: bool,
    low: bool,
    restored: bool,
    unloaded: bool,
    numeric: bool,
}

fn checked_exit(exit: AppExit, required: Completed, done: Completed) -> AppExit {
    if exit.is_error() { return exit; }
    for (required, done, label) in [
        (required.loaded, done.loaded, "loaded terrain quality"),
        (required.low, done.low, "applied low distance"),
        (required.restored, done.restored, "applied restored distance"),
        (required.unloaded, done.unloaded, "unloaded terrain"),
        (required.numeric, done.numeric, "numeric GPU readback"),
    ] {
        if required && !done {
            eprintln!("terrain_fade: incomplete requested check: {label}; allow more time before capture/exit");
            return AppExit::error();
        }
    }
    exit
}

#[derive(Resource)]
struct Controls(Vec<(Vec2, [u8; 3])>);
#[derive(Resource, Default)]
struct Fixture {
    unload: bool,
    moved: bool,
    done: bool,
    last: f32,
    parts: HashSet<Entity>,
    stage: u8,
    changed: f32,
    retained: Vec<(Entity, bevy::asset::AssetId<Mesh>, bevy::asset::AssetId<TerrainMaterial>)>,
}
fn flag(v: &str) -> bool {
    std::env::args().any(|a| a == v)
}
fn main() -> AppExit {
    let mut app = App::new();
    let completion = Completion::default();
    let required = Completed {
        loaded: flag("--world") && flag("--quality"),
        low: flag("--low") || flag("--restore"),
        restored: flag("--restore"),
        unloaded: flag("--unload-check"),
        numeric: !flag("--world"),
    };
    app.insert_resource(completion.clone());
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
        if flag("--shared-distance") { config.terrain_detail_distance_z = config.terrain_detail_distance; }
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
            .add_systems(Last, world_check);
        if flag("--quality") {
            // Keep a changed brightness measurement from affecting the restoration oracle.
            let mut tuning = aa_world::sky::AtmosphereTuning::from_env();
            tuning.adapt = 0.0;
            app.insert_resource(tuning);
        }
    } else {
        app.add_plugins(TerrainMaterialPlugin)
            .insert_resource(ClearColor(Color::BLACK))
            .add_systems(Startup, numeric)
            .add_observer(pixels);
    }
    let exit = app.run();
    let done = *completion.0.lock().unwrap();
    checked_exit(exit, required, done)
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
    let projection = if flag("--axis-x") { 0 } else if flag("--axis-y") { 1 } else { 2 };
    let scene_scale = if flag("--denominator") { 0.0001 } else { 1.0 };
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
                1 => vec![1, 0, 1, 1],
                _ => vec![0, 1, 1, 1],
            },
            TextureFormat::R8Uint,
            RenderAssetUsages::RENDER_WORLD,
        ));
        let values = if flag("--denominator") { vec![(0.0001_f32,0.0002), (0.001,0.001), (0.002,0.002), (1.0,1.0)] }
            else if flag("--quality") { vec![(256.0_f32,64.0), (512.0,128.0), (1024.0,128.0), (1024.0,256.0)] }
            else { vec![(0.0_f32,0.0), (0.5,0.5), (0.8,0.8), (0.9999,0.9999), (1.1,1.1)] };
        for (i, (xy, z_distance)) in values.into_iter().enumerate() {
            let p = Vec2::new(i as f32 * 1.8 - 3.6, 2.2 - row as f32 * 2.2);
            // Match the readback pixel centre, including the near-cutoff regression.
            let (w, h) = (args.size.0 as f32, args.size.1 as f32);
            let x = (w * 0.5 + p.x * h / 8.0).floor();
            let y = (h * 0.5 - p.y * h / 8.0).floor();
            let sample = Vec2::new((x + 0.5 - w * 0.5) * 8.0 / h, (h * 0.5 - y - 0.5) * 8.0 / h);
            let z = if flag("--quality") { if projection == 2 { if flag("--near") { -90.0 } else { -186.0 } } else { -378.0 } } else { 0.0 };
            let distance = Vec3::new(sample.x, sample.y, z - 6.0).length() * scene_scale;
            let selected = if projection == 2 && (!flag("--shared-distance") || flag("--expect-native")) { z_distance } else { xy };
            let ratio = if flag("--quality") || flag("--denominator") { distance / selected.max(0.001) } else { xy };
            let mut params = TerrainParams::new(Vec3::ONE);
            params.tint.w = if flag("--quality") || flag("--denominator") { xy }
                else if ratio == 0.0 { 1e9 } else { distance / ratio };
            params.detail_distance_z = if (flag("--quality") || flag("--denominator")) && !flag("--shared-distance") { z_distance }
                else { params.tint.w };
            let entry = projection * 4;
            params.layers[entry] = Vec4::new(0.0, -1.0, 1.0, 1.0);
            params.layers[entry + 1] = Vec4::new(0.0, 0.0, 1.0, 0.0);
            let mut mesh = Mesh::from(Rectangle::new(1.5, 0.9));
            let normal = match projection { 0 => [1.0,0.0,0.0], 1 => [0.0,0.0,-1.0], _ => [0.0,1.0,0.0] };
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![normal; 4]);
            // Native lower triangle: (x=.5,y=.25) gives A=.25,B=.5,D=.25.
            // The row masks retain controlled surface0 weights1/.5/.25.
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.25 / 512.0, 0.5 / 512.0]; 4]);
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
                Transform::from_translation(Vec3::new(p.x,p.y,z)*scene_scale).with_scale(Vec3::splat(scene_scale)),
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
                Transform::from_translation(Vec3::new(p.x, p.y - 0.65, 0.0)*scene_scale).with_scale(Vec3::splat(scene_scale)),
            ));
        }
    }
    commands.insert_resource(Controls(controls));
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 0.0, 6.0*scene_scale).looking_at(Vec3::ZERO, Vec3::Y),
        Projection::Orthographic(OrthographicProjection {
            near: 0.01*scene_scale,
            far: 1000.0*scene_scale,
            scaling_mode: bevy::camera::ScalingMode::FixedVertical {
                viewport_height: 8.0*scene_scale,
            },
            ..OrthographicProjection::default_3d()
        }),
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Off,
    ));
    let label = if flag("--denominator") {
        "Native .001m denominator minimum: distances .0001/.0002, .001, .002, 1m\nScene scaled1e-4; default ground/Z, --axis-x / --axis-y cliff. Rows weight1 /.5 /.25.\nThin swatches: independent CPU at each position; first two denominators clamp to .001m."
    } else if flag("--quality") {
        "Native terrain detail levels1 /2 /3 /4: XY256 /512 /1024 /1024; Z64 /128 /128 /256\nGround/Z receiver192m (--near96m); --axis-x / --axis-y cliffs384m. Rows weight1 /.5 /.25.\nThin swatches: independent CPU; --shared-distance explicitly selects former shared XY."
    } else {
        "Original terrain layer fade: rows surface weight1 /0.5 /0.25\nColumns: distance/D 0 /0.5 /0.8 /0.9999 /1.1. Thin swatches: independent CPU\nNative sqrt(weight*(1-min(d/D,1)^4)); --legacy expects previous fade outside sqrt"
    };
    commands.spawn((Text::new(label),TextFont{font_size:FontSize::Px(18.0),..default()},Node{position_type:PositionType::Absolute,left:px(20.0),top:px(10.0),..default()}));
}
fn pixels(shot: On<ScreenshotCaptured>, controls: Res<Controls>, completion: Res<Completion>) {
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
    completion.0.lock().unwrap().numeric = true;
}
fn world_setup(mut commands: Commands) {
    let target = Vec3::new(12812.47, 14575.70, 158.27849);
    let height = if flag("--quality") { 192.0 } else { 800.0 };
    let mut camera = commands.spawn((
        Camera3d::default(),
        Transform::from_translation(aa_world::cry_to_bevy(target + Vec3::Z * height))
            .looking_at(aa_world::cry_to_bevy(target), -Vec3::Z),
        Projection::Perspective(PerspectiveProjection {
            fov: 45.0_f32.to_radians(),
            near: 0.1,
            far: 20000.0,
            ..default()
        }),
    ));
    if flag("--quality") {
        // GTAO's per-frame sample sequence is independent of the frozen world clock.
        camera.insert(aa_world::post::Post { ssao: false, ..default() });
    }
}
#[allow(clippy::too_many_arguments)]
fn world_check(
    time: Res<Time>,
    stream: Res<aa_world::stream::Streamer>,
    patches: Query<(Entity, &aa_world::stream::TerrainPatch, &Mesh3d, &MeshMaterial3d<TerrainMaterial>)>,
    materials: Res<Assets<TerrainMaterial>>,
    mut cfg: ResMut<aa_world::WorldConfig>,
    all: Query<()>,
    mut camera: Query<&mut Transform, With<Camera3d>>,
    mut f: ResMut<Fixture>,
    completion: Res<Completion>,
) {
    f.parts.extend(patches.iter().map(|(e, _, _, _)| e));
    if flag("--quality") && !patches.is_empty() {
        for (_, _, _, material) in &patches {
            let params = &materials.get(&material.0).unwrap().extension.params;
            assert_eq!(params.tint.w, cfg.terrain_detail_distance);
            assert_eq!(params.detail_distance_z, cfg.terrain_detail_distance_z);
        }
        // Record observations after production updated the materials, never merely
        // the frame that requested a new config value.
        if f.stage > 0 {
            let mut done = completion.0.lock().unwrap();
            done.low |= cfg.terrain_detail_distance == 256.0 && cfg.terrain_detail_distance_z == 64.0;
            done.restored |= f.stage == 2 && cfg.terrain_detail_distance == 1024.0 && cfg.terrain_detail_distance_z == 256.0;
        }
        if f.stage == 0 && time.elapsed_secs() > 12.0 && !stream.is_loading() {
            f.retained = patches.iter().map(|(e, _, mesh, mat)| (e, mesh.0.id(), mat.0.id())).collect();
            f.stage = 1;
            f.changed = time.elapsed_secs();
            completion.0.lock().unwrap().loaded = true;
            if flag("--low") || flag("--restore") {
                cfg.terrain_detail_distance = 256.0;
                cfg.terrain_detail_distance_z = 64.0;
            }
            info!("terrain-quality loaded check: patches={} saved_xy={} saved_z={} roots/materials retained", f.retained.len(), cfg.terrain_detail_distance, cfg.terrain_detail_distance_z);
        } else if f.stage > 0 {
            for (entity, mesh, material) in &f.retained {
                let (_, _, current_mesh, current_material) = patches.get(*entity).unwrap();
                assert_eq!(current_mesh.0.id(), *mesh);
                assert_eq!(current_material.0.id(), *material);
            }
            if f.stage == 1 && time.elapsed_secs() - f.changed > 4.0 && flag("--restore") {
                cfg.terrain_detail_distance = 1024.0;
                cfg.terrain_detail_distance_z = 256.0;
                f.stage = 2;
                info!("terrain-quality restored loaded world to native default XY1024/Z256 without rebuilding geometry/materials");
            }
        }
    }
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
    let unload_at = if flag("--quality") { 28.0 } else { 16.0 };
    if f.unload && !f.moved && time.elapsed_secs() > unload_at {
        assert!(patches.iter().any(|(_, p, _, _)| p.cell == (12, 14)));
        assert!(!stream.is_loading());
        camera.single_mut().unwrap().translation = aa_world::cry_to_bevy(Vec3::splat(-100000.0));
        f.moved = true;
    }
    if f.moved && !f.done && time.elapsed_secs() > unload_at + 5.0 {
        assert!(patches.is_empty());
        assert_eq!(stream.loaded_count(), 0);
        assert!(!stream.is_loading());
        assert!(f.parts.iter().all(|e| all.get(*e).is_err()));
        info!(
            "terrain fade unload PASSED: {} original patch entity IDs gone",
            f.parts.len()
        );
        f.done = true;
        completion.0.lock().unwrap().unloaded = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn returned_exit(required: Completed, done: Completed, exit: AppExit) -> AppExit {
        let completion = Completion::default();
        *completion.0.lock().unwrap() = done;
        let mut app = App::new();
        app.insert_resource(completion.clone());
        // A timed or window-close runner may return success without any checks.
        app.set_runner(move |_| exit);
        let exit = app.run();
        let done = *completion.0.lock().unwrap();
        checked_exit(exit, required, done)
    }

    #[test]
    fn early_runner_success_cannot_pass_any_missing_requested_stage() {
        for required in [
            Completed { loaded: true, ..default() },
            Completed { low: true, ..default() },
            Completed { restored: true, ..default() },
            Completed { unloaded: true, ..default() },
            Completed { numeric: true, ..default() },
        ] {
            assert!(returned_exit(required, Completed::default(), AppExit::Success).is_error());
        }
        let restore_unload = Completed { loaded: true, low: true, restored: true, unloaded: true, ..default() };
        let loaded_low = Completed { loaded: true, low: true, ..default() };
        assert!(returned_exit(restore_unload, loaded_low, AppExit::Success).is_error());
        let restored = Completed { restored: true, ..loaded_low };
        assert!(returned_exit(restore_unload, restored, AppExit::Success).is_error());
        assert!(returned_exit(restore_unload, restore_unload, AppExit::Success).is_success());
    }

    #[test]
    fn completed_modes_pass_and_existing_runner_error_is_preserved() {
        let done = Completed { loaded: true, low: true, restored: true, unloaded: true, numeric: true };
        assert!(returned_exit(done, done, AppExit::Success).is_success());
        assert!(returned_exit(Completed::default(), Completed::default(), AppExit::Success).is_success());
        assert_eq!(returned_exit(done, done, AppExit::from_code(17)), AppExit::from_code(17));
    }
}
