//! Free-fly camera over an ArcheAge world.
//!
//! ```text
//! cargo run -p aa-world --example fly -- --world main_world --pos 9420,14480,330 --shot shots/aa-world/nuian_start.png
//! cargo run -p aa-world --example fly -- --world login2 --shot shots/aa-world/login2.png
//! ```
//!
//! Flags (plus the `aa-dev` ones `--shot --shot-after --exit-after --size`):
//!
//! * `--world <name>`      `main_world` (default) or `login2`
//! * `--pos x,y[,z]`       camera position in game world (cry) coordinates (without z: 1.8 m above the terrain)
//! * `--spawn <race>`     the new-character spawn of `nuian`, `elf`, `hariharan` or `ferre`, seen from the default
//!                         third person camera (4 m behind, 1.7 m above the player)
//! * `--yaw <deg>`         compass heading, 0 = north (+Y), 90 = east (+X)
//! * `--pitch <deg>`       up positive
//! * `--look x,y,z`       look at this cry-space point instead of yaw / pitch
//! * `--cinema <name>`     camera of `worlds/<world>/cinemas/<name>.xml` (login2 default `01_world_select_e1`),
//!                         `--t <s>` evaluates it at that time, `--play` runs the move
//! * `--camera-json <f>`   pos / dir / fov_deg of a `reference/<screen>/camera.json` capture
//! * `--shift x,y,z`       added to that position; the in-game captures are client-local, the client
//!                         renders main_world 11 cells (11264 m) closer to the origin: `--shift 11264,11264,0`
//! * `--radius <m>`        cell load radius
//! * `--hour <h>`          time of day (0..24, default 10), `--speed <h/s>` lets it run
//! * `--zone <id>`         force a zone's time of day, `--layer <0|1>` the `relic_layer` block of its files
//! * `--sun-scale <lux>`, `--ambient-scale <x>`   lighting calibration (see `WorldConfig`)
//! * `--terrain-tint r,g,b`, `--fog-visibility <m>`, `--tree-distance <m>`
//! * `--bench <s>`         fly forward for that many seconds (`--bench-speed <m/s>`, default 80) and print frame
//!                         time statistics (average, 1 % low, worst frame, frames over 20 ms), then exit
//! * `--fx <lib.group.name>`  a particle effect (`aa-fx`, e.g. `abillity_skill_table_m.system_skill.return_skill_form`, the Return
//!                         orb) in front of the camera: `--fx-dist <m>` (default 3), `--fx-drop <m>` below the view axis
//!                         (default 0.3), `--fx-scale <x>` (default 1)
//! * `--no-sprites`        retain vegetation meshes for sprite comparisons
//! * `--no-objects`        terrain only, `--no-shadows`, `--no-env`, `--no-post` (no HDR / fog / film curve camera setup), `--no-terrain-detail` (cover colour only)
//!
//! Controls: WASD move, Space/Ctrl up/down, Shift fast, hold right mouse button to look.

use aa_world::cinema::CinemaCamera;
use aa_world::{AaWorldPlugin, WorldConfig, bevy_to_cry, cry_to_bevy};
use bevy::input::mouse::MouseMotion;
use bevy::prelude::*;
use bevy::window::{PresentMode, WindowResolution};

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn vec3_arg(name: &str) -> Option<Vec3> {
    let v: Vec<f32> = arg(name)?.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    (v.len() == 3).then(|| Vec3::new(v[0], v[1], v[2]))
}

#[derive(Resource)]
struct Start {
    pos: Vec3,
    yaw: f32,
    pitch: f32,
    look: Option<Vec3>,
    fov: f32,
    /// cry space `(forward, up)` from a cinema / capture.
    dir: Option<(Vec3, Vec3)>,
}

#[derive(Resource)]
struct Bench {
    seconds: f32,
    speed: f32,
    elapsed: f32,
    frames: Vec<f32>,
    slow_logged: usize,
}

fn bench(
    time: Res<Time>,
    mut state: ResMut<Bench>,
    mut cam: Query<&mut Transform, With<Camera3d>>,
    timings: Res<aa_world::WorldTimings>,
    objects: Res<aa_world::objects::ObjectState>,
    streamer: Res<aa_world::stream::Streamer>,
    mut exit: MessageWriter<AppExit>,
) {
    let dt = time.delta_secs();
    if state.elapsed > 8.0 && (dt > 0.05 || (dt > 0.02 && state.slow_logged < 6)) && state.slow_logged < 40 {
        state.slow_logged += 1;
        info!(
            "slow frame {:.1} ms: cells {:.1} receive {:.1} lod {:.1} objects {:.1}",
            dt * 1000.0,
            timings.cells_ms,
            timings.receive_ms,
            timings.lod_ms,
            timings.objects_ms
        );
    }
    state.elapsed += dt;
    if let Ok(mut t) = cam.single_mut() {
        let fwd = Vec3::new(t.forward().x, 0.0, t.forward().z).normalize_or_zero();
        t.translation += fwd * state.speed * dt;
    }
    // the first seconds are the initial load of the surroundings
    if state.elapsed > 8.0 {
        state.frames.push(dt);
    }
    if state.elapsed >= state.seconds + 8.0 {
        let mut f = state.frames.clone();
        f.sort_by(|a, b| b.total_cmp(a));
        let n = f.len().max(1);
        let avg = f.iter().sum::<f32>() / n as f32;
        let low1: f32 = f.iter().take((n / 100).max(1)).sum::<f32>() / (n / 100).max(1) as f32;
        let over = f.iter().filter(|d| **d > 0.02).count();
        info!(
            "BENCH frames={} avg_fps={:.0} 1%_low_fps={:.0} worst_ms={:.1} over_20ms={}",
            n,
            1.0 / avg,
            1.0 / low1,
            f.first().copied().unwrap_or(0.0) * 1000.0,
            over
        );
        info!(
            "BENCH scene: terrain cells {}, object sectors {}, object part entities {}, water cells {}, failed models {}",
            streamer.loaded_cells().count(),
            objects.stats.sectors,
            objects.stats.entities,
            objects.stats.water_cells,
            objects.stats.models_failed
        );
        info!("BENCH vegetation sprites {} using {} shared models", objects.stats.sprites, objects.stats.sprite_models);
        exit.write(AppExit::Success);
    }
}

#[derive(Resource)]
struct Play {
    cinema: CinemaCamera,
    time: f32,
}

fn reference_camera(path: &str) -> Option<(Vec3, Vec3, f32)> {
    let text = std::fs::read_to_string(path).ok()?;
    let num = |obj: &str, key: &str| -> Option<f32> {
        let i = text.find(&format!("\"{obj}\""))?;
        let rest = &text[i..];
        let j = rest.find(&format!("\"{key}\""))?;
        let rest = &rest[j + key.len() + 2..];
        let rest = rest.trim_start().strip_prefix(':')?.trim_start();
        let end = rest.find(|c: char| !(c.is_ascii_digit() || "+-.eE".contains(c)))?;
        rest[..end].parse().ok()
    };
    let pos = Vec3::new(num("pos", "x")?, num("pos", "y")?, num("pos", "z")?);
    let dir = Vec3::new(num("dir", "x")?, num("dir", "y")?, num("dir", "z")?);
    let fov = {
        let i = text.find("\"fov_deg\"")?;
        let rest = text[i + 9..].trim_start().strip_prefix(':')?.trim_start();
        let end = rest.find(|c: char| !(c.is_ascii_digit() || "+-.eE".contains(c)))?;
        rest[..end].parse().ok()?
    };
    Some((pos, dir, fov))
}

fn main() {
    let world = arg("--world").unwrap_or_else(|| "main_world".into());
    let default_pos = if world == "login2" { Vec3::new(1652.5, 465.7, 164.8) } else { Vec3::new(9420.0, 14480.0, 330.0) };
    let mut start = Start {
        pos: vec3_arg("--pos").unwrap_or(default_pos),
        yaw: arg("--yaw").and_then(|v| v.parse().ok()).unwrap_or(if world == "login2" { 270.0 } else { 0.0 }),
        pitch: arg("--pitch").and_then(|v| v.parse().ok()).unwrap_or(-10.0),
        look: vec3_arg("--look"),
        fov: 60.0,
        dir: None,
    };
    // `--pos x,y` without a height: 1.8 m above the terrain there
    if let Some(v) = arg("--pos").map(|v| v.split(',').filter_map(|s| s.trim().parse::<f32>().ok()).collect::<Vec<f32>>()).filter(|v| v.len() == 2) {
        let ground = aa_world::TerrainQuery::new(&world).height_at(v[0], v[1]).unwrap_or(0.0);
        start.pos = Vec3::new(v[0], v[1], ground + 1.8);
    }
    let flag = |name: &str| std::env::args().any(|a| a == name);
    // (x, y, z, server yaw in degrees counter-clockwise from north): `docs/research/world-spots.md`
    let spawns = [
        ("nuian", (15578.042, 15382.122, 126.484, 160.0)),
        ("elf", (10388.576, 15982.466, 367.606, 45.0)),
        ("hariharan", (19304.874, 7646.06, 212.704, 210.0)),
        ("ferre", (23881.218, 10495.782, 589.784, 200.0)),
    ];
    if let Some((_, (x, y, z, yaw))) = arg("--spawn").and_then(|r| spawns.iter().find(|s| s.0 == r)) {
        let a = f32::to_radians(*yaw);
        let forward = Vec3::new(-a.sin(), a.cos(), 0.0);
        start.pos = Vec3::new(*x, *y, *z + 1.73) - forward * 4.0;
        start.yaw = -yaw;
        start.pitch = 0.0;
        start.fov = 42.75; // the real client's in-game field of view
    }
    let t: f32 = arg("--t").and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let cinema_name = arg("--cinema").or_else(|| (world == "login2" && vec3_arg("--pos").is_none() && arg("--camera-json").is_none()).then(|| "01_world_select_e1".to_string()));
    let mut play = None;
    if let Some(name) = cinema_name {
        match CinemaCamera::load(&world, &name) {
            Some(cinema) => {
                let pose = cinema.pose(t);
                start.pos = pose.position;
                start.fov = pose.fov_deg;
                start.dir = Some((pose.rotation * Vec3::Y, pose.rotation * Vec3::Z));
                if flag("--play") {
                    play = Some(Play { cinema, time: t });
                }
            }
            None => eprintln!("cinema {name} not found"),
        }
    }
    if let Some(path) = arg("--camera-json") {
        match reference_camera(&path) {
            Some((pos, dir, fov)) => {
                start.pos = pos + vec3_arg("--shift").unwrap_or(Vec3::ZERO);
                start.fov = fov;
                start.dir = Some((dir, Vec3::Z));
            }
            None => eprintln!("cannot read {path}"),
        }
    }
    let mut plugin = AaWorldPlugin::new(&world);
    if let Some(r) = arg("--radius").and_then(|v| v.parse().ok()) {
        plugin.config = WorldConfig { load_radius: r, unload_radius: r * 1.2, ..plugin.config };
    }
    if let Some(v) = arg("--sun-scale").and_then(|v| v.parse().ok()) {
        plugin.config.sun_lux_scale = v;
    }
    if let Some(v) = arg("--ambient-scale").and_then(|v| v.parse().ok()) {
        plugin.config.ambient_scale = v;
    }
    if let Some(v) = arg("--tree-distance").and_then(|v| v.parse().ok()) {
        plugin.config.tree_distance = v;
    }
    if let Some(v) = vec3_arg("--terrain-tint") {
        plugin.config.terrain_tint = v;
    }
    if let Some(v) = arg("--fog-visibility").and_then(|v| v.parse().ok()) {
        plugin.config.fog_visibility = v;
    }
    plugin.config.objects = !flag("--no-objects");
    plugin.config.vegetation = !flag("--no-objects");
    plugin.config.vegetation_sprites = !flag("--no-sprites");
    plugin.config.shadows = !flag("--no-shadows");
    plugin.config.environment = !flag("--no-env");
    plugin.config.post = !flag("--no-post");
    plugin.config.terrain_detail = !flag("--no-terrain-detail");
    let size = aa_dev::DevArgs::from_env().size;
    let window = WindowPlugin {
        primary_window: Some(Window {
            title: "aa-world fly".into(),
            // physical size = the requested size, whatever the desktop scaling
            resolution: WindowResolution::new(size.0, size.1).with_scale_factor_override(1.0),
            present_mode: PresentMode::AutoNoVsync,
            ..default()
        }),
        ..default()
    };
    let mut clock = aa_world::WorldClock::default();
    if let Some(h) = arg("--hour").and_then(|v| v.parse().ok()) {
        clock.hour = h;
    }
    if let Some(s) = arg("--speed").and_then(|v| v.parse().ok()) {
        clock.speed = s;
    }
    clock.zone = arg("--zone").and_then(|v| v.parse().ok());
    if let Some(l) = arg("--layer").and_then(|v| v.parse().ok()) {
        clock.layer = l;
    }
    let mut app = App::new();
    // experiments: AA_COMPUTE_THREADS=N sizes the ECS task pool (the default takes whatever the IO pools leave)
    let mut task_pools = bevy::app::TaskPoolOptions::default();
    if let Some(n) = std::env::var("AA_COMPUTE_THREADS").ok().and_then(|v| v.parse::<usize>().ok()) {
        task_pools.compute = bevy::app::TaskPoolThreadAssignmentPolicy { min_threads: n, max_threads: n, percent: 1.0, on_thread_spawn: None, on_thread_destroy: None };
    }
    // Bevy compiles the render pipelines on the async compute pool (3 threads on 12 hardware threads)
    if let Some(n) = std::env::var("AA_ASYNC_COMPUTE_THREADS").ok().and_then(|v| v.parse::<usize>().ok()) {
        task_pools.async_compute = bevy::app::TaskPoolThreadAssignmentPolicy { min_threads: n, max_threads: n, percent: 1.0, on_thread_spawn: None, on_thread_destroy: None };
    }
    app.add_plugins(aa_dev::default_plugins("aa-world fly").build().set(window).set(bevy::app::TaskPoolPlugin { task_pool_options: task_pools }))
        .insert_resource(clock)
        .add_plugins(aa_dev::DevPlugin)
        .add_plugins(plugin)
        .insert_resource(start);
    if aa_world::profile::requested() {
        app.add_plugins(aa_world::profile::ProfilePlugin);
    }
    app.add_systems(Update, log_pipelines);
    if let Some(play) = play {
        app.insert_resource(play).add_systems(Update, play_cinema);
    }
    if let Some(seconds) = arg("--bench").and_then(|v| v.parse::<f32>().ok()) {
        let speed = arg("--bench-speed").and_then(|v| v.parse().ok()).unwrap_or(80.0);
        app.insert_resource(Bench { seconds, speed, elapsed: 0.0, frames: Vec::new(), slow_logged: 0 }).add_systems(Update, bench);
    }
    app.add_systems(Startup, setup).add_systems(Update, fly).run();
}

fn setup(mut commands: Commands, start: Res<Start>) {
    let pos = cry_to_bevy(start.pos);
    let mut transform = Transform::from_translation(pos);
    if let Some((forward, up)) = start.dir {
        transform = Transform::from_translation(pos).looking_to(cry_to_bevy(forward), cry_to_bevy(up));
    } else if let Some(look) = start.look {
        transform.look_at(cry_to_bevy(look), Vec3::Y);
    } else {
        let (yaw, pitch) = (start.yaw.to_radians(), start.pitch.to_radians());
        // compass yaw: 0 = north = bevy -Z, 90 = east = +X
        transform.rotation = Quat::from_rotation_y(-yaw) * Quat::from_rotation_x(pitch);
    }
    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection { fov: start.fov.to_radians(), near: 0.3, far: 20000.0, ..default() }),
        transform,
    ));
    if let Some(name) = arg("--fx") {
        let num = |flag: &str, default: f32| arg(flag).and_then(|v| v.parse().ok()).unwrap_or(default);
        let at = pos + *transform.forward() * num("--fx-dist", 3.0) - Vec3::Y * num("--fx-drop", 0.3);
        commands.spawn((aa_fx::Effect::new(name).with_scale(num("--fx-scale", 1.0)), Transform::from_translation(at)));
    }
}

fn play_cinema(
    time: Res<Time>,
    mut play: ResMut<Play>,
    mut cam: Query<(&mut Transform, &mut Projection), With<Camera3d>>,
) {
    let Ok((mut t, mut projection)) = cam.single_mut() else { return };
    play.time = (play.time + time.delta_secs()).min(play.cinema.duration.max(0.1));
    let pose = play.cinema.pose(play.time);
    *t = Transform::from_translation(cry_to_bevy(pose.position))
        .looking_to(cry_to_bevy(pose.rotation * Vec3::Y), cry_to_bevy(pose.rotation * Vec3::Z));
    if let Projection::Perspective(p) = &mut *projection {
        p.fov = pose.fov_deg.to_radians();
    }
}

fn fly(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut motion: MessageReader<MouseMotion>,
    mut cam: Query<&mut Transform, With<Camera3d>>,
    mut log: Local<f32>,
) {
    let Ok(mut t) = cam.single_mut() else { return };
    if mouse.pressed(MouseButton::Right) {
        let d: Vec2 = motion.read().map(|m| m.delta).sum();
        let (mut yaw, mut pitch, _) = t.rotation.to_euler(EulerRot::YXZ);
        yaw -= d.x * 0.003;
        pitch = (pitch - d.y * 0.003).clamp(-1.55, 1.55);
        t.rotation = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0);
    } else {
        motion.clear();
    }
    let mut dir = Vec3::ZERO;
    let fwd = t.forward().as_vec3();
    let right = t.right().as_vec3();
    for (key, v) in [
        (KeyCode::KeyW, fwd),
        (KeyCode::KeyS, -fwd),
        (KeyCode::KeyD, right),
        (KeyCode::KeyA, -right),
        (KeyCode::Space, Vec3::Y),
        (KeyCode::ControlLeft, -Vec3::Y),
    ] {
        if keys.pressed(key) {
            dir += v;
        }
    }
    let speed = if keys.pressed(KeyCode::ShiftLeft) { 400.0 } else { 40.0 };
    t.translation += dir.normalize_or_zero() * speed * time.delta_secs();
    *log += time.delta_secs();
    if *log > 4.0 {
        *log = 0.0;
        let p = bevy_to_cry(t.translation);
        info!("camera at {:.0},{:.0},{:.0}", p.x, p.y, p.z);
    }
}

/// Logs once when the render pipelines of the first frames are compiled (nothing waiting, the count steady for 30 frames).
fn log_pipelines(time: Res<Time>, status: Res<aa_world::pipelines::PipelineStatus>, mut last: Local<(usize, u32, bool)>) {
    if last.2 {
        return;
    }
    let total = status.total();
    if total == last.0 && status.waiting() == 0 && total > 0 {
        last.1 += 1;
    } else {
        last.1 = 0;
        last.0 = total;
    }
    if last.1 >= 30 {
        last.2 = true;
        info!("fly: render pipelines settled after {:.2} s ({total} pipelines)", time.elapsed_secs() - 0.5);
    }
}
