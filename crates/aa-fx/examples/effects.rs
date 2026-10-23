//! A row of particle effects on a dark ground with an orbit camera.
//!
//! ```text
//! cargo run -p aa-fx --example effects -- --shot shots/aa-fx/effects.png
//! cargo run -p aa-fx --example effects -- --effect X2_EFFECTS.fire_smoke.torchlight_b --effect X2_EFFECTS.e_effect.smoke
//! cargo run -p aa-fx --release --example effects -- --grid 100 --exit-after 12     # 100 torches: logs the particle cost
//! ```
//!
//! Flags (plus the aa-dev ones `--shot --shot-after --exit-after --size`):
//!
//! * `--effect <lib.group.name>`  repeatable; default: torch, campfire, candle, smoke, waterfall mist, magic glow
//! * `--grid <n>`                 `n` copies in a grid, cycling through the `--effect` list (a cost test); labels off
//! * `--spacing <m>`              distance between the effects (default 3.2)
//! * `--dist <m>` `--height <m>` `--yaw <deg>` `--orbit <deg/s>`   the camera (orbit defaults to 12 deg/s, 0 with `--shot`)
//! * `--gain <x>` `--light <x>` `--size-scale <x>` `--sprite <half>` `--alpha-scale <x>` `--no-soft` `--max-particles <n>`   `FxSettings`
//! * `--scale <x>`               scale of every effect (the cell entities scale them, e.g. 0.65)
//! * `--fx-yaw <deg>`            turns every effect about the vertical axis (effects emit along their own forward, -Z)
//! * `--day`                      a brighter ground (default: dark, so that additive glows show)
//! * `--loop`                     start every effect again when it has finished (skill effects are one shots)
//! * `--stagger <s>`             with `--grid n`: copy `i` of the effect starts `i * s` seconds after the first (a film strip of one
//!   effect in a single shot; add `--loop` and `--no-prime`)
//! * `--beam <m>`               stretches the effect entity to this length on its Z, like the combat code's beam anchor
//! * `--sweep <m>`               moves every effect on a circle of this radius, 8 rad/s, at 1.2 m (a sword swing: trails and tails)
//! * `--swing <s>`              a sword slash: the effect starts at 3 s, and the entity swings like a hand with a blade (the effect's
//!   forward -Z is the blade) through 170 degrees in the plane facing the camera, in `<s>` seconds (0.3 is a fast cut)
//! * `--backdrop`                a bright checkered wall behind the effects (refraction shows on it)
//! * `--fly <m/s>` `--fly-len <m>`   with `--list`: the effects fly along their forward axis (-Z; turn with `--fx-yaw`) like a projectile,
//!   stopping after `--fly-len` metres (default 4)
//! * `--lift <m>`                 raises every effect (the pedestal stays): a cast orb at hand height
//! * `--no-body`                 the effects are on no unit (the real client's `Particle.SpawnEffect`): `AttachType` emitters emit from the
//!   origin instead of a human sized capsule standing on it
//! * `--no-prime`                 do not run the effects ahead when they start (skill effects start from nothing)
//! * `--list <file>`             a contact sheet: one effect per line (`#` comments), laid out `--cols n` wide (default 4) with a
//!   1.8 m marker beside each; all start together at 4 s (assets preloaded) and are not looped. `--times a,b,c` seconds after
//!   the start, `--out <prefix>` writes `<prefix>_<i>.png` and exits (one window for a whole batch of skill effects)
//! * `--real <dist>`             the camera of the real client captures (`data/extract/aa-fx/r6/real`): 1.68 m above the floor,
//!   `<dist>` metres from the effect, looking horizontally, vertical field of view 42.75 degrees; use with `--lift 0.4 --size 1600x900`
//! * `--bumps <m>`               a rolling ground of this amplitude, known to the effects (`FxSettings::ground`): decals follow it
//! * `--max-lights <n>` `--lux <x>`   the particle lights (`FxSettings::max_lights`, `light_lux`); `--hdr <x>` the HDR multiplier
//! * `--sun-light <x>`             wrapped sunlight on lit sprites (default 0); camera `--yaw` changes the facing
//! * `--ground`                   the ground plane is solid: particles with `CollideTerrain` bounce on it and start their
//!   `ParentCollide` children (`Effect::ground_y`)
//!
//! `aa-fx: ...` is logged every 2 s: effects, active effects, particles, quads and the main thread time of the simulation and
//! mesh upload passes.

use aa_fx::{Effect, FxPlugin, FxSettings, FxStats};
use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn args(name: &str) -> Vec<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().enumerate().filter(|(_, a)| *a == name).filter_map(|(i, _)| args.get(i + 1).cloned()).collect()
}

fn flag(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}

fn num(name: &str, default: f32) -> f32 {
    arg(name).and_then(|v| v.parse().ok()).unwrap_or(default)
}

const DEFAULT_EFFECTS: &[&str] = &[
    "X2_EFFECTS.fire_smoke.torchlight_b",
    "X2_EFFECTS_P.doodad.quest_cook_fire",
    "X2_EFFECTS.e_effect.candle_a",
    "X2_EFFECTS.e_effect.smoke",
    "X2_EFFECTS.water_etc.maha_waterfall_step_01",
    "X2_EFFECTS.e_effect.abbys_purple_spore",
];

/// The rolling ground of `--bumps`.
fn bump_height(amp: f32, x: f32, z: f32) -> f32 {
    amp * ((x * 0.7).sin() + (z * 0.9).cos() - 1.0) * 0.5
}

#[derive(Resource)]
struct Orbit {
    yaw: f32,
    dist: f32,
    height: f32,
    speed: f32,
    target: Vec3,
}

#[derive(Component)]
struct Label(Vec3);

/// An effect moved on a circle about its place.
#[derive(Component)]
struct Sweep {
    centre: Vec3,
    radius: f32,
}

/// An effect swung like a hand with a sword in the plane facing the camera: the blade (the effect's -Z) turns through
/// `from..to` degrees in `duration` seconds from `t0`, the hand riding on a circle of 0.45 m round `centre`.
#[derive(Component)]
struct Swing {
    t0: f32,
    duration: f32,
    centre: Vec3,
}

fn swing(time: Res<Time>, mut q: Query<(&Swing, &mut Transform)>) {
    for (s, mut t) in &mut q {
        let k = ((time.elapsed_secs() - s.t0) / s.duration).clamp(0.0, 1.0);
        let k = k * k * (3.0 - 2.0 * k);
        let phi = (100.0 + (-70.0 - 100.0) * k).to_radians();
        let dir = Vec3::new(phi.cos(), phi.sin(), 0.0);
        t.translation = s.centre + Vec3::Y * 1.2 + dir * 0.45;
        t.rotation = Quat::from_rotation_arc(Vec3::NEG_Z, dir);
    }
}

/// An effect that flies straight along its forward axis (-Z) like a projectile: `speed` m/s from `t0`, stopping after `len` metres.
#[derive(Component)]
struct Fly {
    t0: f32,
    speed: f32,
    len: f32,
    from: Vec3,
    dir: Vec3,
}

fn fly(time: Res<Time>, mut q: Query<(&Fly, &mut Transform)>) {
    for (f, mut t) in &mut q {
        let d = ((time.elapsed_secs() - f.t0) * f.speed).clamp(0.0, f.len);
        t.translation = f.from + f.dir * d;
    }
}

fn sweep(time: Res<Time>, mut q: Query<(&Sweep, &mut Transform)>) {
    let a = time.elapsed_secs() * 8.0;
    for (s, mut t) in &mut q {
        t.translation = s.centre + Vec3::new(a.cos() * s.radius, 1.2, a.sin() * s.radius);
    }
}

/// An effect that starts `at` seconds after the program.
#[derive(Component)]
struct Later {
    at: f32,
    effect: Effect,
    place: Transform,
    swing: Option<Swing>,
    fly: Option<(f32, f32)>,
}

fn start_later(mut commands: Commands, time: Res<Time>, later: Query<(Entity, &Later)>) {
    for (e, l) in &later {
        if time.elapsed_secs() >= l.at {
            let mut spawned = commands.spawn((l.effect.clone(), l.place));
            if let Some(s) = &l.swing {
                spawned.insert(Swing { t0: s.t0, duration: s.duration, centre: s.centre });
            }
            if let Some((speed, len)) = l.fly {
                spawned.insert(Fly { t0: l.at, speed, len, from: l.place.translation, dir: l.place.rotation * Vec3::NEG_Z });
            }
            commands.entity(e).despawn();
        }
    }
}

fn main() {
    let mut effects = args("--effect");
    let sheet = arg("--list").map(|p| {
        let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{p}: {e}"));
        let list: Vec<String> = text.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
        let cols = num("--cols", 4.0) as usize;
        let times = arg("--times").map(|t| t.split(',').filter_map(|v| v.parse().ok()).collect()).unwrap_or_else(|| vec![0.3, 0.8, 1.5, 3.0]);
        Sheet { list, cols: cols.max(1), times, out: arg("--out").unwrap_or_else(|| "shots/aa-fx/sheet".into()) }
    });
    if let Some(sheet) = &sheet {
        effects = sheet.list.clone();
    }
    if effects.is_empty() {
        effects = DEFAULT_EFFECTS.iter().map(|s| s.to_string()).collect();
    }
    let shot = arg("--shot").is_some() || sheet.is_some();
    let grid = if sheet.is_some() { 0 } else { num("--grid", 0.0) as usize };
    let spacing = num("--spacing", if sheet.is_some() { 3.6 } else { 3.2 });
    let n = if grid > 0 { grid } else { effects.len() };
    let width = if grid > 0 { (grid as f32).sqrt().ceil() * spacing } else { n as f32 * spacing };
    let real: Option<f32> = arg("--real").and_then(|v| v.parse().ok());
    let (sheet_dist, sheet_height) = sheet.as_ref().map_or((0.0, 0.0), |s| {
        let rows = n.div_ceil(s.cols) as f32;
        let (w, d) = (s.cols.min(n) as f32 * spacing, rows * spacing);
        let dist = (w * 0.5 + 1.0) / 0.74 + d * 0.15;
        (dist, dist * 0.4)
    });
    let orbit = Orbit {
        yaw: num("--yaw", 0.0).to_radians(),
        dist: num("--dist", if let Some(d) = real { d } else if sheet.is_some() { sheet_dist } else if grid > 0 { width * 0.9 } else { (width * 0.74).max(8.0) }),
        height: num("--height", if real.is_some() { 0.0 } else if sheet.is_some() { sheet_height } else if grid > 0 { width * 0.35 } else { 1.8 }),
        speed: num("--orbit", if shot || grid > 0 { 0.0 } else { 12.0 }).to_radians(),
        target: Vec3::new(0.0, if real.is_some() { 1.68 } else { 1.0 }, 0.0),
    };
    let mut settings = FxSettings::default();
    settings.gain = num("--gain", settings.gain);
    settings.light = num("--light", settings.light);
    settings.sun_light = num("--sun-light", settings.sun_light);
    settings.size_scale = num("--size-scale", settings.size_scale);
    settings.sprite_half = num("--sprite", settings.sprite_half);
    settings.alpha_scale = num("--alpha-scale", settings.alpha_scale);
    settings.max_particles = num("--max-particles", settings.max_particles as f32) as usize;
    if flag("--no-soft") {
        settings.soft_distance = 0.0;
    }
    settings.max_lights = num("--max-lights", settings.max_lights as f32) as usize;
    settings.light_lux = num("--lux", settings.light_lux);
    settings.hdr_base = num("--hdr", settings.hdr_base);
    let bumps = num("--bumps", 0.0);
    if bumps != 0.0 {
        settings.ground = Some(aa_fx::Ground(std::sync::Arc::new(move |p: Vec3| Some(bump_height(bumps, p.x, p.z)))));
    }

    App::new()
        .add_plugins(aa_dev::default_plugins("aa-fx effects"))
        .add_plugins((aa_dev::DevPlugin, FxPlugin))
        .insert_resource(settings)
        .insert_resource(orbit)
        .insert_resource(Spawn { effects, grid, spacing, sheet: sheet.clone() })
        .insert_resource(ClearColor(Color::srgb(0.01, 0.012, 0.02)))
        .add_systems(Startup, setup)
        .add_systems(Update, (orbit_camera, labels, log_stats, start_later, sweep, swing, fly, sheet_shots))
        .run();
}

/// A contact sheet (`--list`).
#[derive(Clone)]
struct Sheet {
    list: Vec<String>,
    cols: usize,
    times: Vec<f32>,
    out: String,
}

/// Seconds the assets get to load before the sheet's effects start.
const SHEET_START: f32 = 4.0;

#[derive(Resource)]
struct Spawn {
    effects: Vec<String>,
    grid: usize,
    spacing: f32,
    sheet: Option<Sheet>,
}

fn sheet_shots(mut commands: Commands, time: Res<Time>, spawn: Res<Spawn>, mut next: Local<usize>, mut exit: MessageWriter<AppExit>) {
    let Some(sheet) = &spawn.sheet else { return };
    let t = time.elapsed_secs();
    if *next < sheet.times.len() {
        if t >= SHEET_START + sheet.times[*next] {
            let path = format!("{}_{}.png", sheet.out, *next);
            commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path.clone()));
            info!("sheet: {path} at +{:.2}s", t - SHEET_START);
            *next += 1;
        }
    } else if t >= SHEET_START + sheet.times[sheet.times.len() - 1] + 2.0 {
        exit.write(AppExit::Success);
    }
}

fn setup(
    mut commands: Commands,
    spawn: Res<Spawn>,
    orbit: Res<Orbit>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let day = flag("--day");
    let ground = if day { Color::srgb(0.25, 0.27, 0.2) } else { Color::srgb(0.035, 0.037, 0.045) };
    let bumps = num("--bumps", 0.0);
    let mut floor = Plane3d::default().mesh().size(120.0, 120.0).subdivisions(if bumps != 0.0 { 239 } else { 0 }).build();
    if bumps != 0.0 {
        if let Some(bevy::mesh::VertexAttributeValues::Float32x3(v)) = floor.attribute_mut(Mesh::ATTRIBUTE_POSITION) {
            for p in v.iter_mut() {
                p[1] = bump_height(bumps, p[0], p[2]);
            }
        }
        floor.compute_normals();
    }
    commands.spawn((
        Mesh3d(meshes.add(floor)),
        MeshMaterial3d(materials.add(StandardMaterial { base_color: ground, perceptual_roughness: 1.0, ..default() })),
    ));
    commands.spawn((DirectionalLight { illuminance: if day { 12_000.0 } else { 1_500.0 }, ..default() }, Transform::from_xyz(4.0, 8.0, 3.0).looking_at(Vec3::ZERO, Vec3::Y)));
    commands.insert_resource(GlobalAmbientLight { color: Color::srgb(0.5, 0.55, 0.7), brightness: if day { 500.0 } else { 60.0 }, ..default() });

    if flag("--backdrop") {
        let size = 64u32;
        let mut data = Vec::with_capacity((size * size * 4) as usize);
        for y in 0..size {
            for x in 0..size {
                let on = ((x / 4) + (y / 4)) % 2 == 0;
                let v = if on { 200u8 } else { 70u8 };
                data.extend_from_slice(&[v, v, (v as f32 * 0.9) as u8, 255]);
            }
        }
        let mut image = Image::new(
            bevy::render::render_resource::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
            bevy::render::render_resource::TextureDimension::D2,
            data,
            bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
            bevy::asset::RenderAssetUsages::RENDER_WORLD,
        );
        image.sampler = bevy::image::ImageSampler::nearest();
        let wall = materials.add(StandardMaterial { base_color_texture: Some(images.add(image)), unlit: true, ..default() });
        commands.spawn((
            Mesh3d(meshes.add(Plane3d::new(Vec3::Z, Vec2::new(14.0, 5.0)))),
            MeshMaterial3d(wall),
            Transform::from_xyz(0.0, 4.0, -3.5),
        ));
    }

    let pedestal = meshes.add(Cylinder::new(0.18, 0.12));
    let pedestal_mat = materials.add(StandardMaterial { base_color: Color::srgb(0.3, 0.28, 0.26), perceptual_roughness: 0.9, ..default() });
    let pillar = meshes.add(Cuboid::new(0.5, 3.0, 0.5));
    let marker = meshes.add(Cuboid::new(0.08, 1.8, 0.08));
    let pillar_mat = materials.add(StandardMaterial { base_color: Color::srgb(0.2, 0.2, 0.22), perceptual_roughness: 0.9, ..default() });

    let mut positions = Vec::new();
    if let Some(sheet) = &spawn.sheet {
        let rows = spawn.effects.len().div_ceil(sheet.cols);
        for (i, name) in spawn.effects.iter().enumerate() {
            let (c, r) = ((i % sheet.cols) as f32, (i / sheet.cols) as f32);
            let cols = sheet.cols.min(spawn.effects.len()) as f32;
            positions.push((name.clone(), Vec3::new((c - (cols - 1.0) * 0.5) * spawn.spacing, 0.0, (r - (rows as f32 - 1.0) * 0.5) * spawn.spacing)));
        }
    } else if spawn.grid > 0 && num("--stagger", 0.0) > 0.0 {
        // a film strip: one row
        for i in 0..spawn.grid {
            positions.push((spawn.effects[0].clone(), Vec3::new((i as f32 - (spawn.grid as f32 - 1.0) * 0.5) * spawn.spacing, 0.0, 0.0)));
        }
    } else if spawn.grid > 0 {
        let side = (spawn.grid as f32).sqrt().ceil() as usize;
        for i in 0..spawn.grid {
            let (x, z) = ((i % side) as f32, (i / side) as f32);
            positions.push((
                spawn.effects[i % spawn.effects.len()].clone(),
                Vec3::new((x - (side as f32 - 1.0) * 0.5) * spawn.spacing, 0.0, (z - (side as f32 - 1.0) * 0.5) * spawn.spacing),
            ));
        }
    } else {
        let n = spawn.effects.len();
        for (i, name) in spawn.effects.iter().enumerate() {
            positions.push((name.clone(), Vec3::new((i as f32 - (n as f32 - 1.0) * 0.5) * spawn.spacing, 0.0, 0.0)));
        }
    }
    let stagger = num("--stagger", 0.0);
    for (i, (name, pos)) in positions.into_iter().enumerate() {
        // `--effect game/objects/effects/crow/crow.chr`: a character-model effect (FxChr), `--chr-life <s>` ends it
        if name.ends_with(".chr") {
            let life = arg("--chr-life").and_then(|v| v.parse().ok());
            commands.spawn((aa_fx::ChrEffect { scale: num("--scale", 1.0), life, ..aa_fx::ChrEffect::new(name) }, Transform::from_translation(pos + Vec3::Y * (0.12 + num("--lift", 0.0))).with_rotation(Quat::from_rotation_y(num("--fx-yaw", 0.0).to_radians()))));
            continue;
        }
        let effect = Effect { force_loop: flag("--loop"), prime: !flag("--no-prime"), ground_y: flag("--ground").then_some(0.0), no_body: flag("--no-body"), ..Effect::new(name.clone()).with_scale(num("--scale", 1.0)) };
        let place = Transform::from_translation(pos + Vec3::Y * (0.12 + num("--lift", 0.0))).with_rotation(Quat::from_rotation_y(num("--fx-yaw", 0.0).to_radians())).with_scale(Vec3::new(1.0, 1.0, num("--beam", 1.0)));
        if spawn.sheet.is_some() {
            // loaded behind the camera first, the real one starts with every other at SHEET_START
            commands.spawn((Effect { force_loop: false, prime: false, ..effect.clone() }, Transform::from_xyz(0.0, 0.0, orbit.dist * 2.0 + 5.0)));
            commands.spawn(Later { at: SHEET_START, effect: Effect { force_loop: false, prime: false, ..effect }, place, swing: None, fly: arg("--fly").and_then(|v| v.parse().ok()).map(|speed| (speed, num("--fly-len", 4.0))) });
            commands.spawn((Mesh3d(marker.clone()), MeshMaterial3d(pillar_mat.clone()), Transform::from_translation(pos + Vec3::new(-1.7, 0.9, 0.0))));
            let short: String = name.rsplit('.').next().unwrap_or(&name).chars().take(16).collect();
            commands.spawn((
                Text::new(format!("{i} {short}")),
                TextFont { font_size: bevy::text::FontSize::Px(13.0), ..default() },
                TextColor(Color::srgb(0.85, 0.85, 0.9)),
                Node { position_type: PositionType::Absolute, ..default() },
                Label(pos + Vec3::Y * 0.3),
            ));
            continue;
        }
        if num("--swing", 0.0) > 0.0 {
            // loads the effect behind the camera first, then slashes at 3 s
            commands.spawn((effect.clone(), Transform::from_xyz(0.0, 0.0, orbit.dist * 2.0 + 5.0)));
            let swing = Swing { t0: 3.0, duration: num("--swing", 0.3), centre: pos };
            commands.spawn(Later { at: 3.0, effect, place, swing: Some(swing), fly: None });
        } else if num("--sweep", 0.0) > 0.0 {
            commands.spawn((effect, place, Sweep { centre: pos, radius: num("--sweep", 0.0) }));
        } else if stagger > 0.0 {
            // the copies start once the first one has loaded everything (an identical effect behind the camera loads it)
            if i == 0 {
                commands.spawn((effect.clone(), Transform::from_xyz(0.0, 0.0, orbit.dist * 2.0 + 5.0)));
            }
            commands.spawn(Later { at: 3.0 + i as f32 * stagger, effect, place, swing: None, fly: None });
        } else {
            commands.spawn((effect, place));
        }
        commands.spawn((Mesh3d(pedestal.clone()), MeshMaterial3d(pedestal_mat.clone()), Transform::from_translation(pos + Vec3::Y * 0.06)));
        if spawn.grid == 0 {
            // a pillar behind each effect, to see the soft particles and the depth sorting
            commands.spawn((Mesh3d(pillar.clone()), MeshMaterial3d(pillar_mat.clone()), Transform::from_translation(pos + Vec3::new(0.0, 1.5, -1.6))));
            let short = name.rsplit('.').next().unwrap_or(&name).to_string();
            commands.spawn((
                Text::new(short),
                TextFont { font_size: bevy::text::FontSize::Px(16.0), ..default() },
                TextColor(Color::srgb(0.85, 0.85, 0.9)),
                Node { position_type: PositionType::Absolute, ..default() },
                Label(pos),
            ));
        }
    }

    let eye = orbit.target + Vec3::new(orbit.yaw.sin() * orbit.dist, orbit.height, orbit.yaw.cos() * orbit.dist);
    commands.spawn((Camera3d::default(), Projection::Perspective(PerspectiveProjection { fov: if flag("--real") || arg("--real").is_some() { 0.746 } else { std::f32::consts::FRAC_PI_4 }, ..default() }), DepthPrepass, Transform::from_translation(eye).looking_at(orbit.target, Vec3::Y)));
}

fn orbit_camera(time: Res<Time>, mut orbit: ResMut<Orbit>, mut cameras: Query<&mut Transform, With<Camera3d>>) {
    let step = orbit.speed * time.delta_secs();
    orbit.yaw += step;
    for mut t in &mut cameras {
        t.translation = orbit.target + Vec3::new(orbit.yaw.sin() * orbit.dist, orbit.height, orbit.yaw.cos() * orbit.dist);
        t.look_at(orbit.target, Vec3::Y);
    }
}

fn labels(cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>, mut labels: Query<(&Label, &mut Node)>) {
    let Ok((camera, gt)) = cameras.single() else { return };
    for (label, mut node) in &mut labels {
        if let Ok(p) = camera.world_to_viewport(gt, label.0 - Vec3::Y * 0.25) {
            node.left = Val::Px(p.x - 50.0);
            node.top = Val::Px(p.y);
        }
    }
}

fn log_stats(time: Res<Time>, stats: Res<FxStats>, mut since: Local<f32>) {
    *since += time.delta_secs();
    if *since < 2.0 {
        return;
    }
    *since = 0.0;
    info!(
        "aa-fx: effects={} active={} emitters={} particles={} quads={} sim_ms={:.3} upload_ms={:.3} manage_ms={:.3} lights={}/{} light_ms={:.3}",
        stats.effects, stats.active, stats.emitters, stats.particles, stats.quads, stats.sim_ms, stats.upload_ms, stats.manage_ms, stats.lights, stats.light_candidates, stats.light_ms
    );
}
