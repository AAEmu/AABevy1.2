//! Shows the collision of the world's brushes as a wireframe over the rendered scene, and the third person
//! camera arm clipped by it.
//!
//! ```text
//! cargo run -p aa-world --example collide -- --player 21131,10594 --yaw 90 --arm 8 --shot shots/aa-world/collide_lutesong.png --shot-after 12
//! ```
//!
//! * `--world <name>`        `main_world` (default) or `login2`
//! * `--player x,y`          the player's feet (game coordinates; the height is the ground under it)
//! * `--yaw <deg>`           compass heading the player looks to (0 = north, 90 = east); the camera sits behind
//! * `--arm <m>` `--pitch <deg>` `--radius <m>`   length and elevation of the camera arm (default 6 m, 15 deg), sphere radius (0.3)
//! * `--no-clip`             put the camera where the arm ends even inside a wall (what the client did without collision)
//! * `--cam x,y,z`          free camera instead (with `--look x,y,z`)
//! * `--draw <m>`            draw the collision triangles within this many metres of the player (default 40), `--no-draw` hides them
//! * `--xray`                draw the wireframe over everything (default: hidden behind surfaces)
//! * `--no-objects`          do not render the brushes (collision only)
//! * `--bench <n>`           time `n` raycasts / sweeps / ground queries around the player and print them
//!
//! Colours: cyan = structures, orange = props, pink = vegetation, yellow line = the wanted camera arm,
//! green = the clipped one, a red cross at the hit.

use aa_world::collide::{Mask, WorldCollision};
use aa_world::{AaWorldPlugin, cry_to_bevy};
use bevy::prelude::*;
use bevy::window::{PresentMode, WindowResolution};
use bevy::gizmos::retained::Gizmo;
use bevy::gizmos::GizmoAsset;
use bevy::gizmos::config::GizmoLineConfig;

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn nums(name: &str) -> Option<Vec<f32>> {
    Some(arg(name)?.split(',').filter_map(|s| s.trim().parse().ok()).collect())
}

fn flag(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}

#[derive(Resource)]
struct Setup {
    player: Vec2,
    yaw: f32,
    pitch: f32,
    arm: f32,
    radius: f32,
    clip: bool,
    free_cam: Option<(Vec3, Vec3)>,
    draw: f32,
    xray: bool,
}

#[derive(Component)]
struct Wire;

fn main() {
    let world = arg("--world").unwrap_or_else(|| "main_world".into());
    let player = nums("--player").filter(|v| v.len() >= 2).map_or(Vec2::new(21_131.0, 10_594.0), |v| Vec2::new(v[0], v[1]));
    let free_cam = nums("--cam").filter(|v| v.len() == 3).map(|c| {
        let look = nums("--look").filter(|v| v.len() == 3).map_or(Vec3::new(player.x, player.y, c[2]), |l| Vec3::new(l[0], l[1], l[2]));
        (Vec3::new(c[0], c[1], c[2]), look)
    });
    let setup = Setup {
        player,
        yaw: arg("--yaw").and_then(|v| v.parse().ok()).unwrap_or(90.0),
        pitch: arg("--pitch").and_then(|v| v.parse().ok()).unwrap_or(15.0),
        arm: arg("--arm").and_then(|v| v.parse().ok()).unwrap_or(6.0),
        radius: arg("--radius").and_then(|v| v.parse().ok()).unwrap_or(0.3),
        clip: !flag("--no-clip"),
        free_cam,
        draw: if flag("--no-draw") { 0.0 } else { arg("--draw").and_then(|v| v.parse().ok()).unwrap_or(40.0) },
        xray: flag("--xray"),
    };
    let mut plugin = AaWorldPlugin::new(&world);
    plugin.config.objects = !flag("--no-objects");
    plugin.config.vegetation = !flag("--no-objects");
    plugin.config.shadows = false;
    plugin.config.load_radius = 700.0;
    plugin.config.unload_radius = 900.0;
    let size = aa_dev::DevArgs::from_env().size;
    let window = WindowPlugin {
        primary_window: Some(Window {
            title: "aa-world collide".into(),
            resolution: WindowResolution::new(size.0, size.1).with_scale_factor_override(1.0),
            present_mode: PresentMode::AutoNoVsync,
            ..default()
        }),
        ..default()
    };
    let mut app = App::new();
    app.add_plugins(aa_dev::default_plugins("aa-world collide").build().set(window))
        .add_plugins(aa_dev::DevPlugin)
        .add_plugins(plugin)
        .insert_resource(setup)
        .add_systems(Startup, spawn)
        .add_systems(Update, (place_camera, redraw).chain());
    if let Some(n) = arg("--bench").and_then(|v| v.parse::<usize>().ok()) {
        app.insert_resource(Bench(n)).add_systems(Update, bench);
    }
    app.run();
}

fn spawn(mut commands: Commands) {
    commands.spawn((Camera3d::default(), Projection::Perspective(PerspectiveProjection { fov: 60f32.to_radians(), near: 0.1, far: 20000.0, ..default() }), Transform::default()));
}

/// The player's feet: the ground under the player (collision when built, else the terrain).
fn feet(collision: &WorldCollision, p: Vec2) -> Vec3 {
    let h = collision.ground_below(p.x, p.y, 10_000.0, 0.0).map_or(0.0, |g| g.height);
    Vec3::new(p.x, p.y, h)
}

/// The compass heading as a game space direction (0 = +Y north, 90 = +X east).
fn heading(yaw_deg: f32) -> Vec3 {
    let a = yaw_deg.to_radians();
    Vec3::new(a.sin(), a.cos(), 0.0)
}

fn place_camera(
    setup: Res<Setup>,
    collision: Res<WorldCollision>,
    mut cam: Query<&mut Transform, With<Camera3d>>,
    mut gizmos: Gizmos,
    mut log: Local<f32>,
    time: Res<Time>,
) {
    let Ok(mut t) = cam.single_mut() else { return };
    // keep the cells around the player built (the plugin follows the camera, which starts far away)
    collision.request_around(setup.player.x, setup.player.y, 120.0);
    let feet = feet(&collision, setup.player);
    let pivot = feet + Vec3::Z * 1.9;
    let pitch = setup.pitch.to_radians();
    let back = -heading(setup.yaw) * pitch.cos() + Vec3::Z * pitch.sin();
    let wanted = pivot + back * setup.arm;
    let reach = if setup.clip { collision.sweep_sphere(pivot, back, setup.arm, setup.radius, Mask::CAMERA) } else { None };
    let eye = reach.map_or(wanted, |h| pivot + back * h.t);
    *log += time.delta_secs();
    if *log > 3.0 {
        *log = 0.0;
        let what = reach.map_or("free".to_string(), |h| format!("hit at {:.2} m ({}, normal {:.2},{:.2},{:.2})", h.t, if h.terrain { "terrain" } else { "brush" }, h.normal.x, h.normal.y, h.normal.z));
        info!("collide: player feet {:.2},{:.2},{:.2}; arm {:.1} m wanted, {what} -> camera {:.1},{:.1},{:.1}", feet.x, feet.y, feet.z, setup.arm, eye.x, eye.y, eye.z);
    }
    // the arm: wanted (yellow), clipped (green), contact (red); the player's axis in white
    gizmos.line(cry_to_bevy(pivot), cry_to_bevy(wanted), Color::srgb(1.0, 0.9, 0.1));
    gizmos.line(cry_to_bevy(pivot), cry_to_bevy(eye), Color::srgb(0.1, 1.0, 0.2));
    if let Some(h) = reach {
        let c = cry_to_bevy(h.point);
        for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
            gizmos.line(c - axis * 0.4, c + axis * 0.4, Color::srgb(1.0, 0.1, 0.1));
        }
    }
    gizmos.sphere(Isometry3d::from_translation(cry_to_bevy(pivot)), 0.15, Color::srgb(1.0, 0.9, 0.1));
    gizmos.line(cry_to_bevy(feet), cry_to_bevy(pivot), Color::srgb(1.0, 1.0, 1.0));
    let (eye, look) = setup.free_cam.unwrap_or((eye, pivot));
    *t = Transform::from_translation(cry_to_bevy(eye)).looking_at(cry_to_bevy(look), Vec3::Y);
}

/// Rebuilds the wireframe of the collision triangles near the player when the cells change.
fn redraw(
    mut commands: Commands,
    setup: Res<Setup>,
    collision: Res<WorldCollision>,
    mut assets: ResMut<Assets<GizmoAsset>>,
    old: Query<Entity, With<Wire>>,
    mut shown: Local<usize>,
) {
    if setup.draw <= 0.0 {
        return;
    }
    let stats = collision.stats();
    if stats.cells == 0 || stats.cells == *shown {
        return;
    }
    *shown = stats.cells;
    for e in &old {
        commands.entity(e).despawn();
    }
    let centre = feet(&collision, setup.player) + Vec3::Z * 2.0;
    let mut asset = GizmoAsset::default();
    let mut count = 0;
    for (mask, color) in [(Mask::STRUCTURES, Color::srgb(0.1, 0.9, 1.0)), (Mask::PROPS, Color::srgb(1.0, 0.6, 0.1)), (Mask::VEGETATION, Color::srgb(1.0, 0.3, 0.7))] {
        for t in collision.triangles_near(centre, setup.draw, mask) {
            let [a, b, c] = t.map(cry_to_bevy);
            asset.line(a, b, color);
            asset.line(b, c, color);
            asset.line(c, a, color);
            count += 1;
        }
    }
    info!("collide: drawing {count} triangles near {:.0},{:.0},{:.0}; {} cells, {:.1} MB, build {:.0} ms in total", centre.x, centre.y, centre.z, stats.cells, stats.bytes as f32 / 1e6, stats.build_ms);
    commands.spawn((Wire, Gizmo { handle: assets.add(asset), line_config: GizmoLineConfig { width: 1.0, ..default() }, depth_bias: if setup.xray { -1.0 } else { -0.0005 } }));
}

#[derive(Resource)]
struct Bench(usize);

/// Once the cells are built: times the queries around the player.
fn bench(bench: Res<Bench>, setup: Res<Setup>, collision: Res<WorldCollision>, mut done: Local<bool>, mut exit: MessageWriter<AppExit>) {
    if *done || !collision.ready_around(setup.player.x, setup.player.y, 60.0) {
        return;
    }
    *done = true;
    let centre = feet(&collision, setup.player) + Vec3::Z * 1.9;
    let time = |label: &str, f: &dyn Fn(Vec3, Vec3)| {
        let t = std::time::Instant::now();
        for i in 0..bench.0 {
            let a = i as f32 * 2.399;
            let p = centre + Vec3::new(a.cos() * (i % 50) as f32, a.sin() * (i % 50) as f32, 0.0);
            f(p, Vec3::new(a.cos(), a.sin(), 0.3).normalize());
        }
        info!("BENCH {label}: {:.2} us per call", t.elapsed().as_secs_f32() * 1e6 / bench.0 as f32);
    };
    time("raycast 10 m", &|o, d| {
        collision.raycast(o, d, 10.0);
    });
    time("sweep_sphere 10 m", &|o, d| {
        collision.sweep_sphere(o, d, 10.0, 0.3, Mask::CAMERA);
    });
    time("ground_below", &|o, _| {
        collision.ground_below(o.x, o.y, o.z - 1.9, 0.5);
    });
    time("push_out", &|o, _| {
        collision.push_out(o - Vec3::Z * 1.9, 0.35, 1.8, 0.5);
    });
    exit.write(AppExit::Success);
}
