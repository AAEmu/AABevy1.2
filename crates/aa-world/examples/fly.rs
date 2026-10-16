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
//! * `--pos x,y,z`         camera position in game world (cry) coordinates
//! * `--yaw <deg>`         compass heading, 0 = north (+Y), 90 = east (+X)
//! * `--pitch <deg>`       up positive
//! * `--look x,y,z`       look at this cry-space point instead of yaw / pitch
//! * `--radius <m>`        cell load radius
//!
//! Controls: WASD move, Space/Ctrl up/down, Shift fast, hold right mouse button to look.

use aa_world::{AaWorldPlugin, WorldConfig, bevy_to_cry, cry_to_bevy};
use bevy::input::mouse::MouseMotion;
use bevy::prelude::*;

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
}

fn main() {
    let world = arg("--world").unwrap_or_else(|| "main_world".into());
    let default_pos = if world == "login2" { Vec3::new(1652.5, 465.7, 164.8) } else { Vec3::new(9420.0, 14480.0, 330.0) };
    let start = Start {
        pos: vec3_arg("--pos").unwrap_or(default_pos),
        yaw: arg("--yaw").and_then(|v| v.parse().ok()).unwrap_or(if world == "login2" { 270.0 } else { 0.0 }),
        pitch: arg("--pitch").and_then(|v| v.parse().ok()).unwrap_or(-10.0),
        look: vec3_arg("--look"),
    };
    let mut plugin = AaWorldPlugin::new(&world);
    if let Some(r) = arg("--radius").and_then(|v| v.parse().ok()) {
        plugin.config = WorldConfig { load_radius: r, unload_radius: r * 1.2, ..plugin.config };
    }
    App::new()
        .add_plugins(aa_dev::default_plugins("aa-world fly"))
        .add_plugins(aa_dev::DevPlugin)
        .add_plugins(plugin)
        .insert_resource(start)
        .add_systems(Startup, setup)
        .add_systems(Update, fly)
        .run();
}

fn setup(mut commands: Commands, start: Res<Start>) {
    let pos = cry_to_bevy(start.pos);
    let mut transform = Transform::from_translation(pos);
    if let Some(look) = start.look {
        transform.look_at(cry_to_bevy(look), Vec3::Y);
    } else {
        let (yaw, pitch) = (start.yaw.to_radians(), start.pitch.to_radians());
        // compass yaw: 0 = north = bevy -Z, 90 = east = +X
        transform.rotation = Quat::from_rotation_y(-yaw) * Quat::from_rotation_x(pitch);
    }
    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection { fov: 60f32.to_radians(), near: 0.3, far: 20000.0, ..default() }),
        transform,
    ));
    commands.spawn((
        DirectionalLight { illuminance: 12000.0, ..default() },
        Transform::from_rotation(Quat::from_euler(EulerRot::YXZ, 0.6, -0.9, 0.0)),
    ));
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
