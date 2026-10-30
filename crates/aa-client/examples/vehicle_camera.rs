//! The driven Farm Wagon's camera distance, from the received attachment; no server connection.
//!
//! The 2026-10-30 captures of the original 1.2 client read the driver's camera in camera mode 1 (option 65):
//! wanted = max = 16 m in the Farm Wagon (`groupA2`, model 1008 normal 20 / wide 16) and 25 m at the Eznan
//! Cutter's helm (`doodads`, model 393 normal 40 / wide 25), and 10 m after leaving. This example plays the same
//! receipts the live session folds (`PlayerSpawn`, `UnitSpawned` of the slave, `UnitAttached` on its driver seat,
//! `UnitDetached` after the check) through aa-net's state and the game's own `mount::track_local_mount`,
//! `camera::vehicle_camera` and `camera::follow_player` systems. The local player itself is a plain capsule: the
//! character assembly is not what this check is about.
//!
//! The run passes when the orbit camera's wanted distance and the vehicle limit are 16 m while the player drives
//! the wagon and 10 m (`VehicleCamera` cleared) after leaving, in camera mode 1. The example writes its own camera
//! mode 1 settings file (`AA_SETTINGS_FILE`, a temporary file) so a machine whose options say classic still checks
//! the captured mode. Needs `AA_PAK` and `AA_DB`; without them it says so and exits 0.
//!
//! `cargo run -p aa-client --example vehicle_camera -- --mute --offscreen-shot shots/aa-client/vehicle_camera.png`
//! writes the driver view. `--shot-after S` is the least time before the scene is staged (default 3).

#![allow(dead_code)]
#[path = "../src/hud/mod.rs"]
mod hud;
#[path = "../src/ingame/mod.rs"]
mod ingame;
#[path = "../src/login/mod.rs"]
mod login;
#[path = "../src/lua_value.rs"]
mod lua_value;
#[path = "../src/script/mod.rs"]
mod script;
#[path = "../src/settings.rs"]
mod settings;
#[path = "../src/sound/mod.rs"]
mod sound;
#[path = "../src/task.rs"]
mod task;

use std::path::PathBuf;

use aa_net::model::UnitKind;
use aa_net::state::{Player, Units};
use aa_net::{Event, NetEvent};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use ingame::camera::{CameraShake, OrbitCamera, VehicleCamera};
use ingame::data::Db;
use ingame::player::{LocalPlayer, Mover};
use ingame::units::Views;
use ingame::{LocalMount, MountReason};
use lua_value::lua_int;

#[derive(States, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Screen {
    #[default]
    WorldSelect,
    Login,
    CharacterSelect,
    CharacterCreate,
    Loading,
    InGame,
}

/// The fixture player, its Farm Wagon (slave template 60) and the model whose camera row is checked.
const PLAYER_OBJ: u32 = 100;
const WAGON_OBJ: u32 = 20;
const WAGON_TEMPLATE: u32 = 60;
const WAGON_MODEL: u32 = 1008;
/// The capture's camera mode 1 values: the Farm Wagon's wide distance, and the action mode's own limit after leaving.
const DRIVING_DISTANCE: f32 = 16.0;
const MODE_LIMIT: f32 = 10.0;
/// Seconds the settled driver state must hold before the picture, and the bounds of the whole run.
const SETTLE: f32 = 1.0;
const STAGING_TIMEOUT: f32 = 180.0;
const SAVING_TIMEOUT: f32 = 30.0;

fn main() -> AppExit {
    if !aa_cry::pak_available() {
        eprintln!("vehicle_camera: skipped, the game pak is missing (AA_PAK=<1.2 client>/game_pak)");
        return AppExit::Success;
    }
    if !aa_anim::GameDb::available() {
        eprintln!(
            "vehicle_camera: skipped, the game database is missing (AA_DB=<server>/compact.sqlite3)"
        );
        return AppExit::Success;
    }
    let Some(db) = Db::open() else {
        eprintln!("vehicle_camera: skipped, the game database cannot be opened (AA_DB)");
        return AppExit::Success;
    };
    // The capture ran in camera mode 1 (option 65); the example must not depend on the machine's saved options.
    let settings_file = camera_mode_one_settings();
    let args = aa_dev::DevArgs::from_env();
    let sequence = Sequence::new(&args);
    let terrain = aa_world::TerrainQuery::new("main_world");
    let collision = aa_world::WorldCollision::new("main_world", terrain.clone());
    let exit = App::new()
        .set_error_handler(bevy::ecs::error::warn)
        .add_plugins((
            aa_dev::default_plugins("Vehicle camera"),
            aa_dev::DevPlugin,
            aa_cry::CryPlugin,
            aa_anim::AnimPlugin,
            aa_anim::facial::FacialPlugin,
            aa_world::pipelines::build,
        ))
        // `Sequence` takes the picture and ends the run: DevPlugin's own shot would exit before the check
        .insert_resource(aa_dev::DevArgs {
            shot: None,
            offscreen_shot: None,
            ..args
        })
        .init_state::<Screen>()
        .insert_state(Screen::InGame)
        .insert_resource(db)
        .insert_resource(sequence)
        .init_resource::<Inbox>()
        .init_resource::<Player>()
        .init_resource::<Units>()
        .init_resource::<Views>()
        .init_resource::<LocalMount>()
        .init_resource::<ingame::player::SendState>()
        .init_resource::<MountReason>()
        .init_resource::<VehicleCamera>()
        .init_resource::<CameraShake>()
        .insert_resource(terrain)
        .insert_resource(collision)
        .insert_resource(ClearColor(Color::srgb(0.62, 0.74, 0.88)))
        .insert_resource(GlobalAmbientLight {
            color: Color::WHITE,
            brightness: 300.0,
            ..default()
        })
        .add_message::<NetEvent>()
        .add_message::<aa_net::NetCommand>()
        .add_systems(Startup, (stage, scene))
        // where `NetPlugin` folds and publishes every receipt: before the systems that read them
        .add_systems(PreUpdate, (drive, deliver).chain())
        .add_systems(
            Update,
            (
                ingame::track_local_mount,
                ingame::camera::vehicle_camera,
                ingame::camera::follow_player,
            )
                .chain(),
        )
        .run();
    let _ = std::fs::remove_file(&settings_file);
    exit
}

/// A settings file holding the captured camera mode (option 65 = 1, action), selected through `AA_SETTINGS_FILE`
/// before anything reads the options. The store is a process-wide `OnceLock`, so this has to happen first.
fn camera_mode_one_settings() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "aabevy-vehicle-camera-{}.json",
        std::process::id()
    ));
    if let Err(error) = std::fs::write(&path, br#"{"options":{"n:65":1}}"#) {
        panic!("vehicle_camera: cannot write {}: {error}", path.display());
    }
    // SAFETY: no other thread reads the environment yet; the example starts the process.
    unsafe { std::env::set_var("AA_SETTINGS_FILE", &path) };
    path
}

/// What a live session would hand over in this frame.
#[derive(Resource, Default)]
struct Inbox(Vec<Event>);

/// Folds the receipts into aa-net's state as the live session does, then publishes them as `NetEvent`s.
fn deliver(
    mut inbox: ResMut<Inbox>,
    mut player: ResMut<Player>,
    mut units: ResMut<Units>,
    mut out: MessageWriter<NetEvent>,
) {
    for event in inbox.0.drain(..) {
        player.apply(&event);
        units.apply(&event);
        out.write(NetEvent(event));
    }
}

/// The camera, the ground and the driver's stand-in; its rig and clips are not the point of this check.
fn stage(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Name::new("orbit camera"),
        Camera3d::default(),
        OrbitCamera {
            yaw: 0.0,
            pitch: 0.08,
            distance: MODE_LIMIT,
            wanted: MODE_LIMIT,
        },
        Transform::from_xyz(0.0, 2.0, 10.0),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 8000.0,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, -0.5, 0.0)),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(80.0, 80.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.20, 0.28, 0.16))),
    ));
    commands.spawn((
        Name::new("local player"),
        LocalPlayer { obj_id: PLAYER_OBJ },
        Mover {
            pos: Vec3::ZERO,
            facing: 0.0,
            vz: 0.0,
            grounded: true,
            velocity: Vec3::ZERO,
            swim: None,
            wading: 0.0,
        },
        Transform::default(),
        Visibility::default(),
        Mesh3d(meshes.add(Capsule3d::new(0.35, 1.1))),
        MeshMaterial3d(materials.add(Color::srgb(0.65, 0.55, 0.40))),
    ));
}

/// The wagon visual and the driver receipts.
fn scene(
    mut commands: Commands,
    mut cry: aa_cry::scene::CryAssets,
    clips: Res<aa_anim::ClipLibrary>,
    db: Res<Db>,
    mut inbox: ResMut<Inbox>,
) {
    let look = db
        .vehicle_look(WAGON_MODEL)
        .expect("the Farm Wagon's vehicle look");
    let pieces = ingame::vehicle::prepare(&cry.loader(), &look);
    ingame::vehicle::spawn(&mut commands, &mut cry, &clips, pieces, Transform::default());

    inbox.0.push(Event::PlayerSpawn(Box::new(player_spawn())));
    let mut wagon = player_spawn().unit;
    wagon.obj_id = WAGON_OBJ;
    wagon.name = "Farm Wagon".into();
    wagon.model_id = WAGON_MODEL;
    wagon.kind = UnitKind::Slave {
        id: WAGON_TEMPLATE,
        tl_id: 75,
        template_id: WAGON_TEMPLATE,
        summoner_id: PLAYER_OBJ,
    };
    inbox.0.push(Event::UnitSpawned(Box::new(wagon)));
    inbox.0.push(Event::UnitAttached {
        obj_id: PLAYER_OBJ,
        mount: WAGON_OBJ,
        point: 1,
        reason: 1,
    });
}

/// The local player the server would spawn (name, level and a character kind are all this check needs).
fn player_spawn() -> aa_net::PlayerSpawn {
    aa_net::PlayerSpawn {
        obj_id: PLAYER_OBJ,
        state: aa_net::model::CharacterState {
            character: aa_net::Character {
                id: 47,
                level: 30,
                ..default()
            },
            ..default()
        },
        unit: aa_net::Unit {
            bounty_money: None,
            pvp_honor: 0,
            pvp_kills: 0,
            obj_id: PLAYER_OBJ,
            name: "Vehicle camera fixture".into(),
            kind: UnitKind::Character { char_id: 47 },
            owner_name: String::new(),
            pos: aa_net::Pos::default(),
            rotation: [0.0; 3],
            scale: 1.0,
            level: 30,
            model_id: 0,
            equipment: default(),
            model_params: default(),
            hp: 100,
            mp: 100,
            max_hp: 100,
            max_mp: 100,
            dead: false,
            race_gender: 1,
            movement: None,
            faction_id: 0,
            expedition_id: 0,
            appellation: 0,
            flags: 0,
            gm_modes: [0; 4],
            skills: vec![],
            passive_buffs: vec![],
            ability_exp: vec![],
            active_abilities: vec![],
            buffs: vec![],
            anim_action: 0,
            attached_to: None,
            doodad_bond: None,
            house_posture: None,
            state_attachments: None,
        },
    }
}

/// Where the run is.
#[derive(Clone, Copy)]
enum Stage {
    /// Waiting for the driver state and the pipelines; `ready_since` is when both first held.
    Staging { ready_since: Option<f32> },
    /// The picture was asked for; the detach follows once it is written.
    Shot { since: f32 },
    /// The player left the seat; the mode's own limit must come back.
    Leaving { since: f32 },
    Over,
}

#[derive(Resource)]
struct Sequence {
    /// `--offscreen-shot` / `--shot`: where the picture is written. None: no picture.
    path: Option<PathBuf>,
    /// `--shot-after`: the clock before which the scene is not staged.
    after: f32,
    stage: Stage,
    saved: usize,
    /// Writing the picture failed (the capture still arrived); the run must not pass.
    failed: bool,
}

impl Sequence {
    fn new(args: &aa_dev::DevArgs) -> Sequence {
        Sequence {
            path: args
                .offscreen_shot
                .as_ref()
                .or(args.shot.as_ref())
                .map(PathBuf::from),
            after: args.shot_after,
            stage: Stage::Staging { ready_since: None },
            saved: 0,
            failed: false,
        }
    }
}

/// A picture of what the window (or the `--offscreen-shot` image) shows now, saved when the GPU hands it back.
fn shoot(commands: &mut Commands, target: Option<&aa_dev::OffscreenTarget>, path: PathBuf) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let shot = target.map_or_else(Screenshot::primary_window, |t| {
        Screenshot::image(t.0.clone())
    });
    commands.spawn(shot).observe(
        move |captured: On<ScreenshotCaptured>, mut sequence: ResMut<Sequence>| {
            let written = captured
                .image
                .clone()
                .try_into_dynamic()
                .map_err(|error| error.to_string())
                .and_then(|image| image.save(&path).map_err(|error| error.to_string()));
            match written {
                Ok(()) => sequence.saved += 1,
                Err(error) => {
                    error!("vehicle_camera: cannot write {}: {error}", path.display());
                    sequence.failed = true;
                }
            }
        },
    );
}

/// Runs the sequence: waits for the driven camera, takes the picture, leaves the seat and checks the restore.
#[allow(clippy::too_many_arguments)]
fn drive(
    mut commands: Commands,
    time: Res<Time>,
    status: Option<Res<aa_world::pipelines::PipelineStatus>>,
    target: Option<Res<aa_dev::OffscreenTarget>>,
    mut sequence: ResMut<Sequence>,
    mut inbox: ResMut<Inbox>,
    camera: Query<&OrbitCamera>,
    limit: Res<VehicleCamera>,
    mut exit: MessageWriter<AppExit>,
) {
    let now = time.elapsed_secs();
    let orbit = camera.single().ok();
    let wanted = orbit.map(|orbit| orbit.wanted);
    let limit = limit.0;
    match sequence.stage {
        Stage::Staging { ready_since } => {
            let ready = now >= sequence.after
                && status.as_ref().is_none_or(|status| status.waiting() == 0)
                && wanted == Some(DRIVING_DISTANCE)
                && limit == Some(DRIVING_DISTANCE);
            if !ready {
                sequence.stage = Stage::Staging { ready_since };
                if now > STAGING_TIMEOUT {
                    error!(
                        "vehicle_camera: the driver camera did not reach {DRIVING_DISTANCE} m \
                         (wanted {wanted:?}, limit {limit:?}) within {STAGING_TIMEOUT} s"
                    );
                    exit.write(AppExit::error());
                    sequence.stage = Stage::Over;
                }
                return;
            }
            let since = ready_since.unwrap_or(now);
            if now - since >= SETTLE {
                info!(
                    "vehicle_camera: driving at {DRIVING_DISTANCE} m (wanted {wanted:?}, limit {limit:?})"
                );
                sequence.stage = match sequence.path.clone() {
                    Some(path) => {
                        shoot(&mut commands, target.as_deref(), path);
                        Stage::Shot { since: now }
                    }
                    None => Stage::Leaving { since: now },
                };
                if matches!(sequence.stage, Stage::Leaving { .. }) {
                    leave(&mut inbox);
                }
            } else {
                sequence.stage = Stage::Staging {
                    ready_since: Some(since),
                };
            }
        }
        Stage::Shot { since } => {
            if sequence.failed {
                error!("vehicle_camera: the picture could not be written");
                exit.write(AppExit::error());
                sequence.stage = Stage::Over;
            } else if sequence.saved > 0 {
                leave(&mut inbox);
                sequence.stage = Stage::Leaving { since: now };
            } else if now - since > SAVING_TIMEOUT {
                error!("vehicle_camera: the picture was not written");
                exit.write(AppExit::error());
                sequence.stage = Stage::Over;
            }
        }
        Stage::Leaving { since } => {
            if wanted == Some(MODE_LIMIT) && limit.is_none() {
                info!(
                    "vehicle_camera: after leaving the seat the mode limit {MODE_LIMIT} m returns \
                     (wanted {wanted:?}, limit {limit:?})"
                );
                exit.write(AppExit::Success);
                sequence.stage = Stage::Over;
            } else if now - since > STAGING_TIMEOUT {
                error!(
                    "vehicle_camera: leaving did not restore {MODE_LIMIT} m \
                     (wanted {wanted:?}, limit {limit:?})"
                );
                exit.write(AppExit::error());
                sequence.stage = Stage::Over;
            }
        }
        Stage::Over => {}
    }
}

/// The captured leaving receipt: the server detaches the driver from the slave's driver seat.
fn leave(inbox: &mut Inbox) {
    inbox.0.push(Event::UnitDetached {
        obj_id: PLAYER_OBJ,
        reason: 1,
    });
}
