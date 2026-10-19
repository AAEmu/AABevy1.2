//! Rope bones in motion: characters in a long robe, a cloak and a dress run in circles, stop, and run again, so the
//! robes, cloaks and skirts swing (see `aa_anim::rope`).
//!
//! ```text
//! cargo run -p aa-anim --example rope -- --shot shots/aa-anim/rope/all.png --shot-after 9
//!     [--only N]       one character close up (0 robe, 1 cloak, 2 dress)
//!     [--crowd N]      N characters (the cloak) on a grid, for the frame-time cost
//!     [--no-rope]      the same without rope simulation (bind-relative rope bones)
//!     [--cam x,y,z --look x,y,z]   camera override
//!     [--record dir --record-fps 15 --record-from 6 --record-secs 2]   frame sequence (aa-dev)
//! ```
//!
//! Every character runs 3 s around a circle (turning all the time), stands 1.5 s, and starts again. The console logs
//! `rope: <n> updates, <µs> per frame` every 2 s next to aa-dev's fps.

use aa_anim::{AnimPlugin, Animator, CharacterDesc, ClipLibrary, Costume, GameDb, Gender, NoRope, Race, RopeSettings, SampleOptions, spawn_character};
use aa_cry::scene::{CryAssets, CryPlugin};
use bevy::prelude::*;

const RUN_SECS: f32 = 3.0;
const STAND_SECS: f32 = 1.5;
/// Ground speed of the run clip (m/s).
const SPEED: f32 = 5.0;

fn main() {
    let mut app = App::new();
    app.add_plugins((aa_dev::default_plugins("aa-anim rope"), aa_dev::DevPlugin, CryPlugin, AnimPlugin))
        .insert_resource(ClearColor(Color::srgb(0.07, 0.075, 0.09)))
        .insert_resource(GlobalAmbientLight { color: Color::srgb(0.75, 0.8, 0.9), brightness: 700.0, ..default() })
        .add_systems(Startup, setup)
        .add_systems(Update, (run, log_stats));
    if has("--no-rope") {
        app.insert_resource(RopeSettings { enabled: false, ..default() });
    }
    app.run();
}

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn has(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}

fn vec3(name: &str, d: Vec3) -> Vec3 {
    arg(name)
        .and_then(|v| {
            let p: Vec<f32> = v.split(',').filter_map(|c| c.trim().parse().ok()).collect();
            (p.len() == 3).then(|| Vec3::new(p[0], p[1], p[2]))
        })
        .unwrap_or(d)
}

/// Runs around `center` and stops now and then.
#[derive(Component)]
struct Runner {
    center: Vec3,
    radius: f32,
    /// Seconds into the run / stand cycle, and the angle on the circle.
    clock: f32,
    angle: f32,
    folder: String,
    running: Option<bool>,
}

struct Look {
    race: Race,
    gender: Gender,
    meshes: &'static [&'static str],
}

const LOOKS: [Look; 3] = [
    // long robe (back flaps on ropeRobe01/02)
    Look { race: Race::Nuian, gender: Gender::Male, meshes: &["objects/characters/nuian/male/parts/ar/cloth/ar_cloth147/nu_m_ar_cloth147.cdf"] },
    // cloak (ropeCloak02/03) over a chest piece
    Look {
        race: Race::Nuian,
        gender: Gender::Male,
        meshes: &[
            "objects/characters/nuian/male/parts/ar/cloth/ar_cloth101/nu_m_ar_cloth101.cdf",
            "objects/characters/nuian/male/parts/cp/cloth/cp_cloth016/nu_m_cp_cloth016.cdf",
        ],
    },
    // dress costume (ropeRobe01/02)
    Look { race: Race::Nuian, gender: Gender::Female, meshes: &["objects/characters/nuian/female/costume/nu_f_sk_party001/nu_f_sk_party001_v101.cdf"] },
];

fn setup(mut commands: Commands, mut cry: CryAssets, clips: Res<ClipLibrary>) {
    let db = GameDb::open().expect("game database");
    let only: Option<usize> = arg("--only").and_then(|v| v.parse().ok());
    let crowd: Option<usize> = arg("--crowd").and_then(|v| v.parse().ok());

    let spawn = |commands: &mut Commands, cry: &mut CryAssets, look: &Look, center: Vec3, radius: f32, clock: f32| {
        let mut desc = CharacterDesc::from_preset(&db, look.race, look.gender, 0).expect("preset");
        desc.costume = Costume::Meshes(look.meshes.iter().map(|s| s.to_string()).collect());
        match desc.resolve(&db).and_then(|a| spawn_character(commands, cry, &clips, a, Transform::from_translation(center))) {
            Ok(e) => {
                let folder = format!("animations/{}/{}", look.race.folder(), look.gender.folder());
                commands.entity(e).insert(Runner { center, radius, clock, angle: clock, folder, running: None });
                if has("--no-rope") {
                    commands.entity(e).insert(NoRope);
                }
            }
            Err(e) => error!("{:?} {:?}: {e}", look.race, look.gender),
        }
    };

    let (cam, look_at) = if let Some(n) = crowd {
        let side = (n as f32).sqrt().ceil() as usize;
        for i in 0..n {
            let (gx, gz) = ((i % side) as f32, (i / side) as f32);
            let center = Vec3::new((gx - (side as f32 - 1.0) / 2.0) * 4.0, 0.0, -gz * 4.0);
            spawn(&mut commands, &mut cry, &LOOKS[1], center, 1.5, i as f32 * 0.37);
        }
        (Vec3::new(0.0, 9.0, 12.0), Vec3::new(0.0, 0.5, -(side as f32) * 1.6))
    } else if let Some(o) = only {
        spawn(&mut commands, &mut cry, &LOOKS[o.min(2)], Vec3::ZERO, 2.0, 0.0);
        (Vec3::new(0.0, 2.2, 7.5), Vec3::new(0.0, 0.9, 0.0))
    } else {
        for (i, look) in LOOKS.iter().enumerate() {
            spawn(&mut commands, &mut cry, look, Vec3::new((i as f32 - 1.0) * 5.0, 0.0, 0.0), 1.8, i as f32 * 0.8);
        }
        (Vec3::new(0.0, 3.0, 12.0), Vec3::new(0.0, 0.9, 0.0))
    };

    let floor = cry.meshes.add(Cylinder::new(40.0, 0.04));
    let floor_mat = cry.materials.add(StandardMaterial { base_color: Color::srgb(0.16, 0.17, 0.18), perceptual_roughness: 1.0, reflectance: 0.0, ..default() });
    commands.spawn((Mesh3d(floor), MeshMaterial3d(floor_mat), Transform::from_xyz(0.0, -0.02, 0.0)));
    commands.spawn((
        DirectionalLight { illuminance: 18000.0, shadow_maps_enabled: true, color: Color::srgb(1.0, 0.96, 0.9), ..default() },
        Transform::from_xyz(2.0, 6.0, 6.0).looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight { illuminance: 6000.0, color: Color::srgb(0.6, 0.7, 1.0), ..default() },
        Transform::from_xyz(-5.0, 2.5, -4.0).looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y),
    ));
    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection { fov: 35f32.to_radians(), ..default() }),
        Transform::from_translation(vec3("--cam", cam)).looking_at(vec3("--look", look_at), Vec3::Y),
    ));
}

/// Moves the runners around their circles and switches between the run and the idle clip.
fn run(time: Res<Time>, clips: Res<ClipLibrary>, mut runners: Query<(&mut Runner, &mut Transform, &mut Animator)>) {
    let dt = time.delta_secs();
    for (mut r, mut t, mut a) in &mut runners {
        r.clock = (r.clock + dt) % (RUN_SECS + STAND_SECS);
        let running = r.clock < RUN_SECS;
        if r.running != Some(running) {
            let name = if running { "fist/fist_mo_normal_run_f" } else { "fist/fist_ba_relaxed_idle" };
            let rig = a.rig.clone();
            match clips.bound(&rig, &format!("{}/{name}.caf", r.folder), SampleOptions::default()) {
                Ok(clip) => a.play(clip, true, 0.2),
                Err(e) => error!("{name}: {e}"),
            }
            r.running = Some(running);
        }
        if running {
            r.angle += SPEED / r.radius * dt;
        }
        // counter-clockwise seen from above; the model faces -Z in Bevy space
        let (s, c) = r.angle.sin_cos();
        t.translation = r.center + Vec3::new(c, 0.0, -s) * r.radius;
        let forward = Vec3::new(-s, 0.0, -c);
        t.rotation = Quat::from_rotation_arc(Vec3::NEG_Z, forward);
    }
}

fn log_stats(time: Res<Time>, mut since: Local<f32>, mut frames: Local<u32>) {
    *since += time.delta_secs();
    *frames += 1;
    if *since >= 2.0 {
        let (n, nanos) = aa_anim::rope::take_stats();
        let (_, anim) = aa_anim::animator::take_stats();
        let f = (*frames).max(1) as f32;
        info!("rope: {n} updates, {:.1} us per frame (whole animation system {:.1} us)", nanos as f32 / 1000.0 / f, anim as f32 / 1000.0 / f);
        *since = 0.0;
        *frames = 0;
    }
}
