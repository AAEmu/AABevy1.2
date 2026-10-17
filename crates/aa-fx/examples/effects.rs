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
//! * `--grid <n>`                 `n` copies of the first effect in a grid (a cost test); labels off
//! * `--spacing <m>`              distance between the effects (default 3.2)
//! * `--dist <m>` `--height <m>` `--yaw <deg>` `--orbit <deg/s>`   the camera (orbit defaults to 12 deg/s, 0 with `--shot`)
//! * `--gain <x>` `--light <x>` `--size-scale <x>` `--alpha-scale <x>` `--no-soft` `--max-particles <n>`   `FxSettings`
//! * `--scale <x>`               scale of every effect (the cell entities scale them, e.g. 0.65)
//! * `--fx-yaw <deg>`            turns every effect about the vertical axis (effects emit along their own forward, -Z)
//! * `--day`                      a brighter ground (default: dark, so that additive glows show)
//! * `--loop`                     start every effect again when it has finished (skill effects are one shots)
//! * `--stagger <s>`             with `--grid n`: copy `i` of the effect starts `i * s` seconds after the first (a film strip of one
//!                                effect in a single shot; add `--loop` and `--no-prime`)
//! * `--no-prime`                 do not run the effects ahead when they start (skill effects start from nothing)
//!
//! `aa-fx: ...` is logged every 2 s: effects, active effects, particles, quads and the main thread time of the simulation and
//! mesh upload passes.

use aa_fx::{Effect, FxPlugin, FxSettings, FxStats};
use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::prelude::*;

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

/// An effect that starts `at` seconds after the program.
#[derive(Component)]
struct Later {
    at: f32,
    effect: Effect,
    place: Transform,
}

fn start_later(mut commands: Commands, time: Res<Time>, later: Query<(Entity, &Later)>) {
    for (e, l) in &later {
        if time.elapsed_secs() >= l.at {
            commands.spawn((l.effect.clone(), l.place));
            commands.entity(e).despawn();
        }
    }
}

fn main() {
    let mut effects = args("--effect");
    if effects.is_empty() {
        effects = DEFAULT_EFFECTS.iter().map(|s| s.to_string()).collect();
    }
    let shot = arg("--shot").is_some();
    let grid = num("--grid", 0.0) as usize;
    let spacing = num("--spacing", 3.2);
    let n = if grid > 0 { grid } else { effects.len() };
    let width = if grid > 0 { (grid as f32).sqrt().ceil() * spacing } else { n as f32 * spacing };
    let orbit = Orbit {
        yaw: num("--yaw", 0.0).to_radians(),
        dist: num("--dist", if grid > 0 { width * 0.9 } else { (width * 0.74).max(8.0) }),
        height: num("--height", if grid > 0 { width * 0.35 } else { 1.8 }),
        speed: num("--orbit", if shot || grid > 0 { 0.0 } else { 12.0 }).to_radians(),
        target: Vec3::new(0.0, 1.0, 0.0),
    };
    let mut settings = FxSettings::default();
    settings.gain = num("--gain", settings.gain);
    settings.light = num("--light", settings.light);
    settings.size_scale = num("--size-scale", settings.size_scale);
    settings.alpha_scale = num("--alpha-scale", settings.alpha_scale);
    settings.max_particles = num("--max-particles", settings.max_particles as f32) as usize;
    if flag("--no-soft") {
        settings.soft_distance = 0.0;
    }

    App::new()
        .add_plugins(aa_dev::default_plugins("aa-fx effects"))
        .add_plugins((aa_dev::DevPlugin, FxPlugin))
        .insert_resource(settings)
        .insert_resource(orbit)
        .insert_resource(Spawn { effects, grid, spacing })
        .insert_resource(ClearColor(Color::srgb(0.01, 0.012, 0.02)))
        .add_systems(Startup, setup)
        .add_systems(Update, (orbit_camera, labels, log_stats, start_later))
        .run();
}

#[derive(Resource)]
struct Spawn {
    effects: Vec<String>,
    grid: usize,
    spacing: f32,
}

fn setup(
    mut commands: Commands,
    spawn: Res<Spawn>,
    orbit: Res<Orbit>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let day = flag("--day");
    let ground = if day { Color::srgb(0.25, 0.27, 0.2) } else { Color::srgb(0.035, 0.037, 0.045) };
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(120.0, 120.0))),
        MeshMaterial3d(materials.add(StandardMaterial { base_color: ground, perceptual_roughness: 1.0, ..default() })),
    ));
    commands.spawn((DirectionalLight { illuminance: if day { 12_000.0 } else { 1_500.0 }, ..default() }, Transform::from_xyz(4.0, 8.0, 3.0).looking_at(Vec3::ZERO, Vec3::Y)));
    commands.insert_resource(GlobalAmbientLight { color: Color::srgb(0.5, 0.55, 0.7), brightness: if day { 500.0 } else { 60.0 }, ..default() });

    let pedestal = meshes.add(Cylinder::new(0.18, 0.12));
    let pedestal_mat = materials.add(StandardMaterial { base_color: Color::srgb(0.3, 0.28, 0.26), perceptual_roughness: 0.9, ..default() });
    let pillar = meshes.add(Cuboid::new(0.5, 3.0, 0.5));
    let pillar_mat = materials.add(StandardMaterial { base_color: Color::srgb(0.2, 0.2, 0.22), perceptual_roughness: 0.9, ..default() });

    let mut positions = Vec::new();
    if spawn.grid > 0 && num("--stagger", 0.0) > 0.0 {
        // a film strip: one row
        for i in 0..spawn.grid {
            positions.push((spawn.effects[0].clone(), Vec3::new((i as f32 - (spawn.grid as f32 - 1.0) * 0.5) * spawn.spacing, 0.0, 0.0)));
        }
    } else if spawn.grid > 0 {
        let side = (spawn.grid as f32).sqrt().ceil() as usize;
        for i in 0..spawn.grid {
            let (x, z) = ((i % side) as f32, (i / side) as f32);
            positions.push((
                spawn.effects[0].clone(),
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
        let effect = Effect { looping: flag("--loop"), prime: !flag("--no-prime"), ..Effect::new(name.clone()).with_scale(num("--scale", 1.0)) };
        let place = Transform::from_translation(pos + Vec3::Y * 0.12).with_rotation(Quat::from_rotation_y(num("--fx-yaw", 0.0).to_radians()));
        if stagger > 0.0 {
            // the copies start once the first one has loaded everything (an identical effect behind the camera loads it)
            if i == 0 {
                commands.spawn((effect.clone(), Transform::from_xyz(0.0, 0.0, orbit.dist * 2.0 + 5.0)));
            }
            commands.spawn(Later { at: 3.0 + i as f32 * stagger, effect, place });
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
    commands.spawn((Camera3d::default(), DepthPrepass, Transform::from_translation(eye).looking_at(orbit.target, Vec3::Y)));
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
        "aa-fx: effects={} active={} emitters={} particles={} quads={} sim_ms={:.3} upload_ms={:.3} manage_ms={:.3}",
        stats.effects, stats.active, stats.emitters, stats.particles, stats.quads, stats.sim_ms, stats.upload_ms, stats.manage_ms
    );
}
