//! Passenger seats on a cart: board point 2 from the HUD's 13863 interaction request, switch to the driver's seat after
//! leaving, have a remote passenger follow the drawn cart, and leave. Synthetic receipts through aa-net's state and
//! the game's own `mount` systems; no socket. Every wait has a timeout and exits non-zero.
//!
//! The run passes when
//! * the local player is on attach point 2 and drawn at the body's `$passenger0` helper,
//! * after `SCUnitDetached` and a driver attach it sits at `$driver`,
//! * a remote character attached at point 2 sits at `$passenger0` and follows a moved cart `Transform`,
//! * both leave and the local seat and the remote `Rider` are gone.
//!
//! The example exits non-zero on a failed check or a staging timeout. Needs `AA_PAK` and `AA_DB`; without them it says
//! so and exits 0.
//!
//! `cargo run -p aa-client --example passenger_seats -- --mute --offscreen-shot shots/aa-client/passenger_seats.png`

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

use aa_net::interaction::InteractEvent;
use aa_net::model::{CharacterState, Equipment, ModelParams, Unit, UnitKind};
use aa_net::state::{Player, Units};
use aa_net::{Event, NetEvent, Pos};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use ingame::LocalMount;
use ingame::data::Db;
use ingame::player::{LocalPlayer, Mover, SendState, placement};
use ingame::units::{RemoteUnit, Views};
use ingame::{MountReason, player};
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

const LOCAL_OBJ: u32 = 100;
const REMOTE_OBJ: u32 = 101;
const CART_OBJ: u32 = 20;
const CART_TEMPLATE: u32 = 146;
const CART_MODEL: u32 = 1553;
const CART_POS: Vec3 = Vec3::new(100.0, 200.0, 5.0);
const CART_YAW: f32 = 0.4;
const REMOTE_AT: Vec3 = Vec3::new(104.0, 203.0, 5.0);
const SETTLE: f32 = 0.5;
const STAGING_TIMEOUT: f32 = 15.0;
const SAVING_TIMEOUT: f32 = 30.0;

fn main() -> AppExit {
    if !aa_cry::pak_available() {
        eprintln!(
            "passenger_seats: skipped, the game pak is missing (AA_PAK=<1.2 client>/game_pak)"
        );
        return AppExit::Success;
    }
    if !aa_anim::GameDb::available() {
        eprintln!(
            "passenger_seats: skipped, the game database is missing (AA_DB=<server>/compact.sqlite3)"
        );
        return AppExit::Success;
    }
    let Some(db) = Db::open() else {
        eprintln!("passenger_seats: skipped, the database cannot be opened (AA_DB)");
        return AppExit::Success;
    };
    let args = aa_dev::DevArgs::from_env();
    let sequence = Sequence::new(&args);
    let terrain = aa_world::TerrainQuery::new("main_world");
    let collision = aa_world::WorldCollision::new("main_world", terrain.clone());
    App::new()
        .set_error_handler(bevy::ecs::error::warn)
        .add_plugins((
            aa_dev::default_plugins("Passenger seats"),
            aa_dev::DevPlugin,
            aa_cry::CryPlugin,
            aa_anim::AnimPlugin,
            aa_anim::facial::FacialPlugin,
            aa_world::pipelines::build,
        ))
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
        .init_resource::<SendState>()
        .init_resource::<MountReason>()
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
        .add_systems(PreUpdate, (drive, deliver).chain())
        .add_systems(
            Update,
            (
                ingame::mount::track_local_mount,
                ingame::mount::drive_mount,
                ingame::mount::pose_local_rider,
                ingame::mount::start_stop_riding,
                ingame::mount::pose_riders,
                ingame::mount::step_dismounts,
            )
                .chain(),
        )
        .run()
}

#[derive(Resource, Default)]
struct Inbox(Vec<Event>);

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

fn stage(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let cart = aa_world::cry_to_bevy(CART_POS);
    let eye = cart + Vec3::new(-5.0, 4.0, 8.0);
    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(eye).looking_at(cart + Vec3::Y, Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 9000.0,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, -0.5, 0.0)),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(60.0, 60.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.2, 0.3, 0.16))),
        Transform::from_translation(aa_world::cry_to_bevy(Vec3::new(100.0, 200.0, 0.0))),
    ));
}

fn scene(
    mut commands: Commands,
    mut cry: aa_cry::scene::CryAssets,
    clips: Res<aa_anim::ClipLibrary>,
    db: Res<Db>,
    mut views: ResMut<Views>,
    mut inbox: ResMut<Inbox>,
) {
    let look = db
        .vehicle_look(CART_MODEL)
        .expect("the red ox cart's vehicle look");
    let pieces = ingame::vehicle::prepare(&cry.loader(), &look);
    let cart = ingame::vehicle::spawn(
        &mut commands,
        &mut cry,
        &clips,
        pieces,
        placement(CART_POS, CART_YAW, 1.0),
    );
    let cart_unit = fixture_unit(
        CART_OBJ,
        UnitKind::Slave {
            id: CART_TEMPLATE,
            tl_id: 75,
            template_id: CART_TEMPLATE,
            summoner_id: LOCAL_OBJ,
        },
        "Red Ox Cart",
        CART_MODEL,
        CART_POS,
    );
    commands
        .entity(cart)
        .insert(RemoteUnit::announced(&cart_unit));
    views.track_unit_entity(CART_OBJ, cart);

    let game_db = aa_anim::db::GameDb::open().expect("the animation database");
    let local_assets = aa_anim::CharacterDesc::from_preset(
        &game_db,
        aa_anim::db::Race::Nuian,
        aa_anim::db::Gender::Male,
        0,
    )
    .expect("the local preset")
    .resolve(&game_db)
    .expect("the local assets");
    let local = player::spawn_with_locomotion(
        &mut commands,
        &mut cry,
        &clips,
        local_assets,
        Transform::default(),
        Vec::new(),
    )
    .expect("the local character");
    commands.entity(local).insert((
        LocalPlayer { obj_id: LOCAL_OBJ },
        Mover {
            pos: CART_POS,
            facing: CART_YAW,
            vz: 0.0,
            grounded: true,
            velocity: Vec3::ZERO,
            swim: None,
            wading: 0.0,
        },
    ));

    let remote_assets = aa_anim::CharacterDesc::from_preset(
        &game_db,
        aa_anim::db::Race::Nuian,
        aa_anim::db::Gender::Female,
        0,
    )
    .expect("the remote preset")
    .resolve(&game_db)
    .expect("the remote assets");
    let remote_unit = fixture_unit(
        REMOTE_OBJ,
        UnitKind::Character { char_id: 102 },
        "Remote Passenger",
        remote_assets.model_id,
        REMOTE_AT,
    );
    let remote = player::spawn_with_locomotion(
        &mut commands,
        &mut cry,
        &clips,
        remote_assets,
        placement(REMOTE_AT, 0.0, 1.0),
        Vec::new(),
    )
    .expect("the remote character");
    commands
        .entity(remote)
        .insert(RemoteUnit::announced(&remote_unit));
    views.track_unit_entity(REMOTE_OBJ, remote);

    inbox
        .0
        .push(Event::PlayerSpawn(Box::new(aa_net::PlayerSpawn {
            obj_id: LOCAL_OBJ,
            state: CharacterState {
                character: aa_net::Character {
                    id: 47,
                    level: 30,
                    ..default()
                },
                ..default()
            },
            unit: fixture_unit(
                LOCAL_OBJ,
                UnitKind::Character { char_id: 47 },
                "Passenger fixture",
                0,
                CART_POS,
            ),
        })));
    inbox.0.push(Event::UnitSpawned(Box::new(cart_unit)));
    inbox.0.push(Event::UnitSpawned(Box::new(remote_unit)));
}

fn fixture_unit(obj_id: u32, kind: UnitKind, name: &str, model_id: u32, pos: Vec3) -> Unit {
    Unit {
        obj_id,
        name: name.into(),
        kind,
        owner_name: String::new(),
        pos: Pos::new(pos.x, pos.y, pos.z),
        rotation: [0.0; 3],
        scale: 1.0,
        level: 30,
        model_id,
        equipment: Equipment::default(),
        model_params: ModelParams::default(),
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
        bounty_money: None,
        pvp_honor: 0,
        pvp_kills: 0,
        skills: vec![],
        passive_buffs: vec![],
        ability_exp: vec![],
        active_abilities: vec![],
        buffs: vec![],
        anim_action: 0,
        attached_to: None,
        doodad_bond: None,
        house_posture: None,
        turret: None,
        state_attachments: None,
    }
}

fn attach(inbox: &mut Inbox, obj_id: u32, point: u8) {
    inbox.0.push(Event::UnitAttached {
        obj_id,
        mount: CART_OBJ,
        point,
        reason: 0,
    });
}

fn detach(inbox: &mut Inbox, obj_id: u32) {
    inbox.0.push(Event::UnitDetached { obj_id, reason: 1 });
}

fn helper(cart: &Transform, helpers: &ingame::vehicle::SeatHelpers, name: &str) -> Option<Vec3> {
    let local = helpers.0.get(name)?.translation;
    Some(cart.transform_point(aa_world::cry_to_bevy(local.into())))
}

/// The real press: the server's `SCNpcInteractionSkillList` 0x07b for the cart offers 13863, and the HUD acts on the
/// first skill at once. Returns whether it queued the exact `CSStartSkill` request; the caller then plays the server's
/// `SCUnitAttached` reply.
fn board_passenger_through_hud(units: &Units) -> bool {
    let mut data = hud::api::HudData::default();
    data.pos = CART_POS;
    data.player_radius = 0.0;
    let event = Event::Interaction(InteractEvent::SkillList {
        npc_obj_id: CART_OBJ,
        obj_id: 0,
        extra_info: 1,
        pick_id: 0,
        mouse_button: 3,
        skills: vec![13863],
        interactable: true,
        modifier_keys: 0,
    });
    let _ = hud::slave::receive(&mut data, units, &event);
    data.requests.iter().any(|request| {
        matches!(
            request,
            hud::api::Request::UseSkill {
                skill_id: 13863,
                target_obj_id: CART_OBJ
            }
        )
    })
}

#[derive(Clone, Copy)]
enum Stage {
    BoardPassenger,
    PassengerSeated { since: f32 },
    LeavePassenger,
    PassengerLeft { since: f32 },
    BoardDriverAndRemote { since: f32 },
    Seated { since: f32 },
    MoveCart,
    CartMoved { since: f32 },
    Leave { since: f32 },
    Over,
}

#[derive(Resource)]
struct Sequence {
    path: Option<PathBuf>,
    after: f32,
    stage: Stage,
    saved: usize,
    failed: bool,
}

impl Sequence {
    fn new(args: &aa_dev::DevArgs) -> Self {
        Sequence {
            path: args
                .offscreen_shot
                .as_ref()
                .or(args.shot.as_ref())
                .map(PathBuf::from),
            after: args.shot_after.max(1.0),
            stage: Stage::BoardPassenger,
            saved: 0,
            failed: false,
        }
    }
}

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
                    error!("passenger_seats: cannot write {}: {error}", path.display());
                    sequence.failed = true;
                }
            }
        },
    );
}

fn fail(exit: &mut MessageWriter<AppExit>, sequence: &mut Sequence, why: String) {
    error!("passenger_seats: {why}");
    exit.write(AppExit::error());
    sequence.stage = Stage::Over;
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn drive(
    mut commands: Commands,
    time: Res<Time>,
    mut sequence: ResMut<Sequence>,
    mut inbox: ResMut<Inbox>,
    mut exit: MessageWriter<AppExit>,
    target: Option<Res<aa_dev::OffscreenTarget>>,
    seat: Res<LocalMount>,
    units: Res<Units>,
    mut carts: Query<
        (&mut Transform, &ingame::vehicle::SeatHelpers),
        (
            With<RemoteUnit>,
            Without<LocalPlayer>,
            Without<ingame::mount::Rider>,
        ),
    >,
    players: Query<&Transform, With<LocalPlayer>>,
    riders: Query<(Entity, &RemoteUnit, &Transform), With<ingame::mount::Rider>>,
) {
    let now = time.elapsed_secs();
    if sequence.failed {
        fail(
            &mut exit,
            &mut sequence,
            "the picture could not be written".into(),
        );
        return;
    }
    let cart = carts.iter_mut().next();
    let Ok(player) = players.single() else { return };
    match sequence.stage {
        Stage::BoardPassenger => {
            if now < sequence.after {
                return;
            }
            if !board_passenger_through_hud(&units) {
                fail(
                    &mut exit,
                    &mut sequence,
                    "the HUD did not queue the 13863 CSStartSkill request".into(),
                );
                return;
            }
            info!("passenger_seats: the HUD 13863 request is queued");
            attach(&mut inbox, LOCAL_OBJ, 2);
            sequence.stage = Stage::PassengerSeated { since: now };
        }
        Stage::PassengerSeated { since } => {
            let Some((cart, helpers)) = cart else { return };
            let expected = helper(&cart, helpers, "passenger0");
            let seated = seat.0.is_some_and(|s| s.point == 2 && !s.drives);
            match (seated, expected) {
                (true, Some(expected)) if player.translation.distance(expected) < 1e-3 => {
                    if now - since >= SETTLE {
                        info!("passenger_seats: seated as passenger at {expected:?}");
                        sequence.stage = Stage::LeavePassenger;
                    }
                }
                _ if now - since > STAGING_TIMEOUT => {
                    fail(
                        &mut exit,
                        &mut sequence,
                        format!(
                            "passenger seat: {seated}, player at {:?}, expected {expected:?}",
                            player.translation
                        ),
                    );
                }
                _ => {}
            }
        }
        Stage::LeavePassenger => {
            detach(&mut inbox, LOCAL_OBJ);
            sequence.stage = Stage::PassengerLeft { since: now };
        }
        Stage::PassengerLeft { since } => {
            if seat.0.is_none() {
                attach(&mut inbox, LOCAL_OBJ, 1);
                attach(&mut inbox, REMOTE_OBJ, 2);
                sequence.stage = Stage::BoardDriverAndRemote { since: now };
            } else if now - since > STAGING_TIMEOUT {
                fail(
                    &mut exit,
                    &mut sequence,
                    format!(
                        "the passenger detach did not clear the local seat after {STAGING_TIMEOUT} s"
                    ),
                );
            }
        }
        Stage::BoardDriverAndRemote { since } => {
            if seat.0.is_some() {
                sequence.stage = Stage::Seated { since: now };
            } else if now - since > STAGING_TIMEOUT {
                fail(
                    &mut exit,
                    &mut sequence,
                    format!("the driver attach did not set LocalMount after {STAGING_TIMEOUT} s"),
                );
            }
        }
        Stage::Seated { since } => {
            let Some((cart, helpers)) = cart else { return };
            let driver = helper(&cart, helpers, "driver");
            let passenger = helper(&cart, helpers, "passenger0");
            let local_ok = seat.0.is_some_and(|s| s.point == 1)
                && seat.cart_driver() == Some(CART_OBJ)
                && driver.is_some_and(|p| player.translation.distance(p) < 1e-3);
            let remote = riders
                .iter()
                .find(|(_, unit, _)| unit.obj_id == REMOTE_OBJ)
                .map(|(_, _, transform)| transform.translation);
            let remote_ok = remote.is_some_and(|p| passenger.is_some_and(|e| p.distance(e) < 1e-3));
            if local_ok && remote_ok {
                if now - since < SETTLE {
                    return;
                }
                info!("passenger_seats: driver at {driver:?}, remote passenger at {passenger:?}");
                if sequence.path.is_some() && sequence.saved == 0 {
                    if let Some(path) = sequence.path.clone() {
                        shoot(&mut commands, target.as_deref(), path);
                    }
                    sequence.stage = Stage::MoveCart;
                    return;
                }
                sequence.stage = Stage::MoveCart;
            } else if now - since > STAGING_TIMEOUT {
                fail(
                    &mut exit,
                    &mut sequence,
                    format!(
                        "driver/remote seat: local {local_ok}, remote {remote_ok} ({remote:?})"
                    ),
                );
            }
        }
        Stage::MoveCart => {
            if sequence.path.is_some() && sequence.saved == 0 {
                if now - sequence.after > SAVING_TIMEOUT {
                    fail(
                        &mut exit,
                        &mut sequence,
                        "the picture was not written".into(),
                    );
                }
                return;
            }
            let Some((mut cart, _)) = cart else { return };
            cart.translation += Vec3::new(3.0, -1.5, 0.5);
            sequence.stage = Stage::CartMoved { since: now };
        }
        Stage::CartMoved { since } => {
            let Some((cart, helpers)) = cart else { return };
            let driver = helper(&cart, helpers, "driver");
            let passenger = helper(&cart, helpers, "passenger0");
            let local_ok = driver.is_some_and(|p| player.translation.distance(p) < 1e-3);
            let remote = riders
                .iter()
                .find(|(_, unit, _)| unit.obj_id == REMOTE_OBJ)
                .map(|(_, _, transform)| transform.translation);
            let remote_ok = remote.is_some_and(|p| passenger.is_some_and(|e| p.distance(e) < 1e-3));
            if local_ok && remote_ok {
                info!("passenger_seats: the drawn cart moved and both riders followed");
                detach(&mut inbox, LOCAL_OBJ);
                detach(&mut inbox, REMOTE_OBJ);
                sequence.stage = Stage::Leave { since: now };
            } else if now - since > STAGING_TIMEOUT {
                fail(
                    &mut exit,
                    &mut sequence,
                    format!(
                        "follow the drawn cart: local {local_ok}, remote {remote_ok} ({remote:?})"
                    ),
                );
            }
        }
        Stage::Leave { since } => {
            let remote_gone = !riders.iter().any(|(_, unit, _)| unit.obj_id == REMOTE_OBJ);
            if seat.0.is_none() && remote_gone {
                info!("passenger_seats: both seats cleared");
                exit.write(AppExit::Success);
                sequence.stage = Stage::Over;
            } else if now - since > STAGING_TIMEOUT {
                fail(
                    &mut exit,
                    &mut sequence,
                    format!(
                        "the final detach did not clear both seats after {STAGING_TIMEOUT} s (local {}, remote rider gone {remote_gone})",
                        seat.0.is_none()
                    ),
                );
            }
        }
        Stage::Over => {}
    }
}
