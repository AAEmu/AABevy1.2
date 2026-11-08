//! A mount swimming on the horse graph's clips: the Lilyut horse (model 139) floats with its origin its swim level
//! (`MountSpeed::swim_level`, 2.14 m) under a water plane, as `player::control_player` floats a driven mount. The script
//! writes what `units::move_units` writes for a remote mate in the swim stance (`Motion`, `Swimming` at `waterLevel` 0):
//! the swim idle for 2 s, then a swim forward at the horse's 1.8 m/s, a strafe to the right, and forward again (in place:
//! the horse's swim clips have no root travel). `anim::drive_locomotion` picks `fist_ba_idle_swim`, `fist_mo_swim_f` and
//! `fist_mo_swim_r`; the console logs every clip change.
//!
//! It needs `AA_PAK` and `AA_DB`, and exits non-zero when one of the three clips did not play by the shot:
//!
//! `cargo run -p aa-client --example mount_swim -- --mute --shot shots/aa-client/mount_swim.png --shot-after 8`
#![allow(dead_code)]
#[path = "../src/graphics.rs"]
mod graphics;
#[path = "../src/hud/mod.rs"]
mod hud;
#[path = "../src/ingame/mod.rs"]
mod ingame;
#[path = "../src/logging.rs"]
mod logging;
#[path = "../src/login/mod.rs"]
mod login;
#[path = "../src/priority.rs"]
mod priority;
#[path = "../src/script/mod.rs"]
mod script;
#[path = "../src/skilltrace/mod.rs"]
mod skilltrace;
#[path = "../src/settings.rs"]
mod settings;
#[path = "../src/sound/mod.rs"]
mod sound;
#[path = "../src/task.rs"]
mod task;
#[path = "../src/lua_value.rs"]
mod lua_value;
use bevy::prelude::*;
use ingame::anim::{AirStart, Motion, Swimming};
use lua_value::lua_int;

#[derive(States, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Screen {
    #[default]
    Login,
    WorldSelect,
    CharacterSelect,
    CharacterCreate,
    Loading,
    InGame,
}

/// The script: until 2 s the swim idle, then forward, a strafe to the right from 4.5 s, forward again from 6 s.
const SWIM_FROM: f32 = 2.0;
const STRAFE_FROM: f32 = 4.5;
const FORWARD_AGAIN: f32 = 6.0;

#[derive(Component)]
struct Horse;

/// The clips the horse played, in order.
#[derive(Resource, Default)]
struct Played(Vec<String>);

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("Mount swim"),
            aa_dev::DevPlugin,
            aa_cry::CryPlugin,
            aa_anim::AnimPlugin,
            aa_world::pipelines::build,
        ))
        .insert_resource(GlobalAmbientLight { color: Color::WHITE, brightness: 400.0, ..default() })
        .insert_resource(ClearColor(Color::srgb(0.32, 0.4, 0.5)))
        .init_resource::<ingame::anim::LocalJump>()
        .init_resource::<Played>()
        .add_systems(Startup, scene)
        .add_systems(Update, (swim_cycle, ingame::anim::drive_locomotion, log_clips).chain())
        .add_systems(Last, check_clips)
        .run();
}

fn scene(mut commands: Commands, mut cry: aa_cry::scene::CryAssets, clips: Res<aa_anim::ClipLibrary>) {
    let db = ingame::data::Db::open().expect("game database (AA_DB)");
    let file = db.actor_model_file(139).expect("model 139 (horse) has an actor model file");
    let level = db.mount_speed(139).swim_level;
    info!("mount_swim: swim level {level:.3} m");
    let horse = ingame::creature::spawn(&mut commands, &mut cry, &clips, &file, None, Transform::from_rotation(Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2)))
        .unwrap_or_else(|| panic!("{file} cannot be spawned"));
    commands.entity(horse).insert((Horse, AirStart::default(), Swimming::default()));
    // the water surface the horse floats under, see-through so the body under it shows
    commands.spawn((
        Mesh3d(cry.meshes.add(Plane3d::default().mesh().size(30.0, 30.0))),
        MeshMaterial3d(cry.materials.add(StandardMaterial {
            base_color: Color::srgba(0.12, 0.3, 0.38, 0.55),
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 0.2,
            ..default()
        })),
        Transform::from_xyz(0.0, level, 0.0),
    ));
    commands.spawn((
        DirectionalLight { illuminance: 12000.0, shadow_maps_enabled: true, ..default() },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, -0.5, 0.0)),
    ));
    commands.spawn((Camera3d::default(), Transform::from_xyz(0.0, level + 1.6, 7.5).looking_at(Vec3::new(0.0, level - 0.2, 0.0), Vec3::Y)));
}

/// What a remote mate in the swim stance writes: standing, then swimming forward, to the right and forward.
fn swim_cycle(time: Res<Time>, mut horse: Query<&mut Motion, With<Horse>>) {
    let Ok(mut motion) = horse.single_mut() else { return };
    let t = time.elapsed_secs();
    let local_dir = match t {
        t if t < SWIM_FROM => Vec2::ZERO,
        t if (STRAFE_FROM..FORWARD_AGAIN).contains(&t) => Vec2::X,
        _ => Vec2::Y,
    };
    *motion = Motion { speed: if local_dir == Vec2::ZERO { 0.0 } else { 1.8 }, local_dir, airborne: false, climb: None };
}

fn log_clips(time: Res<Time>, mut played: ResMut<Played>, horse: Query<&aa_anim::Animator, With<Horse>>) {
    let Ok(animator) = horse.single() else { return };
    let Some(file) = animator.current.as_ref().and_then(|p| ingame::anim::clip_file(&p.clip)) else { return };
    if played.0.last().is_none_or(|last| **last != *file) {
        info!("mount_swim: {:.2} s: {file}", time.elapsed_secs());
        played.0.push(file.to_string());
    }
}

/// By the default shot (8 s) the swim idle, the forward swim and the strafe played.
fn check_clips(time: Res<Time>, played: Res<Played>, mut checked: Local<bool>) {
    if *checked || time.elapsed_secs() < 7.5 {
        return;
    }
    *checked = true;
    for clip in ["fist_ba_idle_swim.caf", "fist_mo_swim_f.caf", "fist_mo_swim_r.caf"] {
        if !played.0.iter().any(|p| p == clip) {
            error!("mount_swim: {clip} did not play: {:?}", played.0);
            std::process::exit(1);
        }
    }
    info!("mount_swim: the swim played {:?}", played.0);
}
