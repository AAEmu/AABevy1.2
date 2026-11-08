//! A mount's jump on the horse graph's clips: the Lilyut horse (model 139) runs in place and jumps every 3 s on its
//! 1.3 m arc at gravity 30 (0.59 s in the air). The script writes what `units::move_units` writes for a remote mate
//! (`Motion` with its speed kept in the air, `AirStart` for the take-off) and lifts the horse along the arc;
//! `anim::drive_locomotion` picks `fist_mo_jump_f_start`, holds it to 0.98 of its 0.83 s (longer than the air time, so
//! on flat ground the landing `fist_mo_jump_f_land` follows it), then the run. The console logs every clip change.
//!
//! It needs `AA_PAK` and `AA_DB`, and exits non-zero when the take-off clip did not play by the shot:
//!
//! `cargo run -p aa-client --example mount_jump -- --mute --shot shots/aa-client/mount_jump.png --shot-after 6`
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
use ingame::anim::{AirStart, Motion};
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

/// The horse's run speed (m/s, `MountSpeed::run` of the Lilyut horse) and its jump: `sqrt(2 * 30 * 1.3)` up, gravity 30.
const RUN: f32 = 9.0;
const TAKE_OFF: f32 = 8.83;
const GRAVITY: f32 = 30.0;
/// The first take-off (s after start) and the time between two: the default `--shot-after 6` falls in the second jump.
const FIRST_JUMP: f32 = 2.85;
const EVERY: f32 = 3.0;

#[derive(Component)]
struct Horse;

/// The clips the horse played, in order.
#[derive(Resource, Default)]
struct Played(Vec<String>);

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("Mount jump"),
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
        .add_systems(Update, (jump_cycle, ingame::anim::drive_locomotion, log_clips).chain())
        .add_systems(Last, check_take_off)
        .run();
}

fn scene(mut commands: Commands, mut cry: aa_cry::scene::CryAssets, clips: Res<aa_anim::ClipLibrary>) {
    let db = ingame::data::Db::open().expect("game database (AA_DB)");
    let file = db.actor_model_file(139).expect("model 139 (horse) has an actor model file");
    let horse = ingame::creature::spawn(&mut commands, &mut cry, &clips, &file, None, Transform::from_rotation(Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2)))
        .unwrap_or_else(|| panic!("{file} cannot be spawned"));
    commands.entity(horse).insert((Horse, AirStart::default()));
    commands.spawn((
        Mesh3d(cry.meshes.add(Plane3d::default().mesh().size(30.0, 30.0))),
        MeshMaterial3d(cry.materials.add(StandardMaterial { base_color: Color::srgb(0.3, 0.36, 0.24), perceptual_roughness: 1.0, ..default() })),
    ));
    commands.spawn((
        DirectionalLight { illuminance: 12000.0, shadow_maps_enabled: true, ..default() },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, -0.5, 0.0)),
    ));
    commands.spawn((Camera3d::default(), Transform::from_xyz(0.0, 2.2, 9.0).looking_at(Vec3::new(0.0, 1.6, 0.0), Vec3::Y)));
}

/// What a remote mate's jump writes: the run, then 0.59 s in the air with the speed kept and the take-off set.
fn jump_cycle(time: Res<Time>, mut horse: Query<(&mut Motion, &mut AirStart, &mut Transform), With<Horse>>) {
    let Ok((mut motion, mut start, mut transform)) = horse.single_mut() else { return };
    let t = time.elapsed_secs() - FIRST_JUMP;
    let p = t.rem_euclid(EVERY);
    let height = if t >= 0.0 { TAKE_OFF * p - 0.5 * GRAVITY * p * p } else { 0.0 };
    let airborne = height > 0.0;
    *motion = Motion { speed: RUN, local_dir: Vec2::Y, airborne, climb: None };
    start.0 = airborne;
    transform.translation.y = height.max(0.0);
}

fn log_clips(time: Res<Time>, mut played: ResMut<Played>, horse: Query<&aa_anim::Animator, With<Horse>>) {
    let Ok(animator) = horse.single() else { return };
    let Some(file) = animator.current.as_ref().and_then(|p| ingame::anim::clip_file(&p.clip)) else { return };
    if played.0.last().is_none_or(|last| **last != *file) {
        info!("mount_jump: {:.2} s: {file}", time.elapsed_secs());
        played.0.push(file.to_string());
    }
}

/// By the default shot (6 s) the first jump played its take-off and landing.
fn check_take_off(time: Res<Time>, played: Res<Played>, mut checked: Local<bool>) {
    if *checked || time.elapsed_secs() < 5.5 {
        return;
    }
    *checked = true;
    for clip in ["fist_mo_jump_f_start.caf", "fist_mo_jump_f_land.caf"] {
        if !played.0.iter().any(|p| p == clip) {
            error!("mount_jump: {clip} did not play: {:?}", played.0);
            std::process::exit(1);
        }
    }
    info!("mount_jump: the jump played {:?}", played.0);
}
