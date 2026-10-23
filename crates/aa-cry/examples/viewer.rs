//! Model viewer: `cargo run -p aa-cry --example viewer -- --model objects/<...>.cgf [--shot shots/aa-cry/x.png]`
//!
//! Flags (besides the aa-dev ones `--shot`, `--shot-after`, `--exit-after`, `--size`):
//! * `--model <pak path>`   `.cgf` / `.cga` / `.chr` (repeatable: models are placed side by side)
//! * `--attach <pak path>`  skin (`.chr`) bound to the skeleton of the previous `--model`
//! * `--material <.mtl>`    material override for the last `--model`
//! * `--lod <n>`            level of detail
//! * `--full`               native Illum / skin / hair shading (all models)
//! * `--stage`              login lighting rig (camera at --yaw 180 faces characters)
//! * `--yaw <deg>` `--pitch <deg>` `--dist <m>`  initial camera (distance defaults to fit)
//! * `--focus <x,y,z>`      camera target (Bevy space, metres; defaults to the bounds centre)
//! * `--bones`              draw the skeleton
//!
//! Mouse: left drag orbits, wheel zooms, right drag pans.

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::light::CascadeShadowConfigBuilder;
use bevy::prelude::*;

use aa_cry::scene::{CryAssets, CryJoint, CryModel, CryPlugin, ModelOptions, SkeletonBinding};

#[derive(Resource)]
struct Args {
    /// (path, options, attach to the previous model's skeleton)
    models: Vec<(String, ModelOptions, bool)>,
    yaw: f32,
    pitch: f32,
    dist: Option<f32>,
    focus: Option<Vec3>,
    bones: bool,
    stage: bool,
}

fn parse_args() -> Args {
    let args: Vec<String> = std::env::args().collect();
    let mut a = Args { models: Vec::new(), yaw: 35.0, pitch: 15.0, dist: None, focus: None, bones: false, stage: false };
    let mut lod = 0;
    let mut i = 1;
    while i < args.len() {
        let v = args.get(i + 1).cloned().unwrap_or_default();
        match args[i].as_str() {
            "--model" | "--attach" => {
                a.models.push((v, ModelOptions { lod, ..Default::default() }, args[i] == "--attach"));
                i += 1;
            }
            "--material" => {
                if let Some(m) = a.models.last_mut() {
                    m.1.material = Some(v);
                }
                i += 1;
            }
            "--lod" => {
                lod = v.parse().unwrap_or(0);
                for m in &mut a.models {
                    m.1.lod = lod;
                }
                i += 1;
            }
            "--yaw" => {
                a.yaw = v.parse().unwrap_or(a.yaw);
                i += 1;
            }
            "--pitch" => {
                a.pitch = v.parse().unwrap_or(a.pitch);
                i += 1;
            }
            "--dist" => {
                a.dist = v.parse().ok();
                i += 1;
            }
            "--focus" => {
                let v: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                if v.len() == 3 {
                    a.focus = Some(Vec3::new(v[0], v[1], v[2]));
                }
                i += 1;
            }
            "--bones" => a.bones = true,
            "--stage" => a.stage = true,
            _ => {}
        }
        i += 1;
    }
    if a.models.is_empty() {
        a.models.push(("objects/env/01_nuia/001_ncom/ncom_city_signpost_c.cgf".into(), ModelOptions::default(), false));
    }
    if args.iter().any(|arg| arg == "--full") {
        for (_, options, _) in &mut a.models {
            options.shading = aa_cry::Shading::Full;
        }
    }
    a
}

#[derive(Component)]
struct Orbit {
    focus: Vec3,
    yaw: f32,
    pitch: f32,
    dist: f32,
}

fn main() {
    App::new()
        .add_plugins((aa_dev::default_plugins("aa-cry viewer"), aa_dev::DevPlugin, CryPlugin))
        .insert_resource(parse_args())
        .insert_resource(ClearColor(Color::srgb(0.32, 0.38, 0.46)))
        .insert_resource(GlobalAmbientLight { color: Color::srgb(0.75, 0.8, 0.9), brightness: 900.0, ..default() })
        .add_systems(Startup, setup)
        .add_systems(Update, (orbit, draw_bones))
        .run();
}

fn setup(mut commands: Commands, mut cry: CryAssets, args: Res<Args>) {
    let mut x = 0.0f32;
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    // (root, binding) of the last plain --model
    let mut base: Option<(Entity, Option<SkeletonBinding>)> = None;
    let mut base_offset = 0.0f32;
    for (path, options, attach) in &args.models {
        let t0 = std::time::Instant::now();
        let mut options = options.clone();
        if *attach {
            let Some((base_root, Some(binding))) = &base else {
                error!("aa-cry viewer: --attach {path}: previous model has no skeleton");
                continue;
            };
            options.skeleton = Some(binding.clone());
            options.cry_space = true;
            match cry.spawn_model_full(&mut commands, path, &options) {
                Ok(s) => {
                    commands.entity(s.root).insert(ChildOf(*base_root));
                    let loaded = cry.load_model(path, &options).unwrap();
                    if let Some((a, b)) = loaded.model.bounds() {
                        let (a, b) = (Vec3::new(a.x, a.z, -b.y), Vec3::new(b.x, b.z, -a.y));
                        lo = lo.min(a + Vec3::X * base_offset);
                        hi = hi.max(b + Vec3::X * base_offset);
                        x = x.max(hi.x + 0.5);
                    }
                    info!("aa-cry viewer: attached {path}: mtl {:?}, {:.1} ms", loaded.material_path, t0.elapsed().as_secs_f64() * 1000.0);
                }
                Err(e) => error!("aa-cry viewer: {path}: {e}"),
            }
            continue;
        }
        let options = &options;
        match cry.spawn_model_full(&mut commands, path, options) {
            Ok(spawned) => {
                let root = spawned.root;
                let loaded = cry.load_model(path, options).unwrap();
                base = Some((root, loaded.skeleton.clone().map(|skeleton| SkeletonBinding { skeleton, joints: spawned.joints.clone() })));
                let (a, b) = loaded.model.bounds().unwrap_or((Vec3::ZERO, Vec3::ZERO));
                // CryEngine (x, y, z) -> Bevy (x, z, -y)
                let (a, b) = (Vec3::new(a.x, a.z, -b.y), Vec3::new(b.x, b.z, -a.y));
                let width = (b.x - a.x).max(0.5);
                let offset = x - a.x;
                base_offset = offset;
                commands.entity(root).insert(Transform::from_xyz(offset, 0.0, 0.0).with_rotation(aa_cry::CRY_TO_BEVY));
                lo = lo.min(a + Vec3::X * offset);
                hi = hi.max(b + Vec3::X * offset);
                x += width + 0.5;
                let parts: usize = loaded.parts.iter().map(|p| p.1.len()).sum();
                info!(
                    "aa-cry viewer: {path}: {} nodes, {} meshes, {parts} parts, skeleton {:?}, mtl {:?}, {:.1} ms",
                    loaded.model.nodes.len(),
                    loaded.model.meshes.len(),
                    loaded.skeleton.as_ref().map(|s| s.len()),
                    loaded.material_path,
                    t0.elapsed().as_secs_f64() * 1000.0
                );
            }
            Err(e) => error!("aa-cry viewer: {path}: {e}"),
        }
    }
    if !cry.cache.missing_textures.is_empty() {
        warn!("aa-cry viewer: missing textures: {:?}", cry.cache.missing_textures);
    }
    if lo.x > hi.x {
        lo = Vec3::ZERO;
        hi = Vec3::ONE;
    }
    let center = args.focus.unwrap_or((lo + hi) * 0.5);
    let radius = ((hi - lo).length() * 0.5).max(0.5);
    let dist = args.dist.unwrap_or(radius * 2.4);

    if !args.stage {
        // ground
        commands.spawn((
            Mesh3d(cry.meshes.add(Plane3d::default().mesh().size(radius * 8.0, radius * 8.0))),
            MeshMaterial3d(cry.materials.add(StandardMaterial { base_color: Color::srgb(0.35, 0.33, 0.3), perceptual_roughness: 1.0, ..default() })),
            Transform::from_xyz(center.x, lo.y.min(0.0) - 0.001, center.z),
        ));
        // sun
        commands.spawn((
            DirectionalLight { illuminance: 9000.0, shadow_maps_enabled: true, ..default() },
            // from the camera side, a bit to the left and above
            Transform::from_rotation(Quat::from_euler(EulerRot::YXZ, args.yaw.to_radians() - 0.7, -0.75, 0.0)),
            CascadeShadowConfigBuilder { maximum_distance: radius * 10.0, first_cascade_far_bound: radius * 2.0, ..default() }.build(),
        ));
    }
    let camera = commands.spawn((
        Camera3d::default(),
        Orbit { focus: center, yaw: args.yaw.to_radians(), pitch: args.pitch.to_radians(), dist },
        Transform::default(),
    )).id();
    if args.stage {
        aa_cry::stage::StageLighting::login().spawn(&mut commands, &mut cry, camera, Quat::IDENTITY);
        commands.entity(camera).insert(aa_cry::stage::login_color_grading());
    }
}

fn orbit(
    mut q: Query<(&mut Orbit, &mut Transform)>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
) {
    for (mut o, mut t) in &mut q {
        if buttons.pressed(MouseButton::Left) {
            o.yaw -= motion.delta.x * 0.005;
            o.pitch = (o.pitch + motion.delta.y * 0.005).clamp(-1.5, 1.5);
        }
        if buttons.pressed(MouseButton::Right) {
            let right = t.right();
            let up = t.up();
            let k = o.dist * 0.0015;
            o.focus += (-right * motion.delta.x + up * motion.delta.y) * k;
        }
        if scroll.delta.y != 0.0 {
            o.dist = (o.dist * (1.0 - scroll.delta.y * 0.1)).max(0.05);
        }
        let rot = Quat::from_euler(EulerRot::YXZ, o.yaw, -o.pitch, 0.0);
        *t = Transform::from_translation(o.focus + rot * Vec3::new(0.0, 0.0, o.dist)).looking_at(o.focus, Vec3::Y);
    }
}

fn draw_bones(args: Res<Args>, joints: Query<(&GlobalTransform, &ChildOf), With<CryJoint>>, all: Query<&GlobalTransform>, mut gizmos: Gizmos, _m: Query<&CryModel>) {
    if !args.bones {
        return;
    }
    for (gt, parent) in &joints {
        if let Ok(pt) = all.get(parent.0) {
            if joints.get(parent.0).is_ok() {
                gizmos.line(pt.translation(), gt.translation(), Color::srgb(1.0, 0.9, 0.1));
            }
        }
        gizmos.sphere(Isometry3d::from_translation(gt.translation()), 0.008, Color::srgb(1.0, 0.3, 0.1));
    }
}
