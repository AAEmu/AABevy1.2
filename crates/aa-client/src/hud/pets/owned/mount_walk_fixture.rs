//! Self-checking walk to the mount: the real HUD binding (`X2Mate:MountMate()`), the real `walk_to_mount` / `update`
//! systems and the real on-foot controller over received mate state, with no socket.
//!
//! A mate 10 m north is mounted: the press starts the walk, the scripted forward key moves the player in, the walk
//! stops at the 1.2 controller's 3 m (`0x39AC3878 = 9.0`, squared) and queues exactly one `CSMountMate`. A mate 2.5 m
//! north is mounted at once: no walk, no forward key, one request. The check fails the run when either case is wrong.
//!
//! `cargo run -p aa-client --example mount_walk -- --headless --headless-fps 240` (needs `AA_PAK` and `AA_DB`).
use crate::hud::api::{self, HudBridge, HudData, Shared};
use crate::hud::data::GameData;
use crate::hud::{HudBuilt, HudKeyboardFocus, HudPointerOverUi};
use crate::ingame::camera::OrbitCamera;
use crate::ingame::player::{LocalPlayer, Mover, PlayerInput, SendState};
use crate::ingame::teleport::Teleport;
use crate::ingame::{DevOptions, LocalJump, Motion, MountReason};
use crate::settings::Actions;
use aa_net::mate::{MateCommand, Mates};
use aa_net::state::Units;
use aa_net::{Command, NetCommand, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

/// The 1.2 mount controller's stop distance (`0x39AC3878` = 9.0, squared).
const STOP: f32 = 3.0;
/// The first mate's distance (m) and the second's.
const FAR: f32 = 10.0;
const NEAR: f32 = 2.5;
/// The player and the mate live on the main world near the character spawn (terrain exists there).
const AT: (f32, f32) = (15_600.0, 15_380.0);
/// `control_player` waits up to 5 s for the collision around the spawn; press after that.
const PRESS_AFTER: f32 = 5.5;
/// Within this long the walk must have got this much nearer before an arrival is believable.
const PROGRESS_SECS: f32 = 1.0;
const PROGRESS: f32 = 1.0;
const TIMEOUT: f32 = 30.0;
/// The local player's object and character ids (the active pet fixture's player and mate).
const PLAYER_OBJ: u32 = 100;
const MATE_OBJ: u32 = 201;

/// The staged positions: the player at [`AT`] on the terrain, the mate [`FAR`] then [`NEAR`] metres north.
#[derive(Resource, Clone, Copy)]
struct Staged {
    player: Vec3,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    /// Waiting for the collision settle before the first press.
    Waiting,
    /// The 10 m walk.
    Walking,
    /// The 2.5 m press at once.
    Close,
    /// Both cases passed.
    Done,
}

#[derive(Resource)]
struct Check {
    phase: Phase,
    pressed_at: f32,
    sent: u32,
    frames: u32,
    start: f32,
    last: f32,
    saw_w: bool,
    failures: Vec<String>,
}

impl Default for Check {
    fn default() -> Self {
        Check {
            phase: Phase::Waiting,
            pressed_at: 0.0,
            sent: 0,
            frames: 0,
            start: 0.0,
            last: 0.0,
            saw_w: false,
            failures: Vec::new(),
        }
    }
}

impl Check {
    fn fail(&mut self, failure: String) {
        self.failures.push(failure);
    }
}

pub(crate) fn run() {
    if !aa_cry::pak_available() {
        eprintln!("mount_walk: skipped, no game_pak (set AA_PAK)");
        return;
    }
    let db = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
    if !Path::new(&db).exists() {
        eprintln!("mount_walk: skipped, no game database at {db} (set AA_DB)");
        return;
    }
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let (rt, bridge) = runtime(width as f32, height as f32, &db)
        .unwrap_or_else(|error| panic!("mount_walk: the HUD could not be built: {error}"));
    let (player, units, mates) = super::super::active_fixture::received();
    let terrain = aa_world::TerrainQuery::new("main_world");
    let z = terrain.height_at(AT.0, AT.1).unwrap_or(126.0);
    let staged = Staged {
        player: Vec3::new(AT.0, AT.1, z),
    };
    let mut app = App::new();
    app.add_plugins((
        aa_dev::default_plugins("aa-client: mount walk"),
        aa_dev::DevPlugin,
    ));
    app.insert_non_send(rt)
        .insert_non_send(HudBridge(bridge))
        .insert_resource(player)
        .insert_resource(units)
        .insert_resource(mates)
        .insert_resource(staged)
        .insert_resource(terrain.clone())
        .insert_resource(aa_world::WorldCollision::new("main_world", terrain.clone()))
        .insert_resource(aa_world::WaterQuery::with_terrain("main_world", terrain))
        .init_state::<crate::Screen>()
        .insert_state(crate::Screen::InGame)
        .insert_resource(HudBuilt {
            up: true,
            ..default()
        })
        .init_resource::<HudKeyboardFocus>()
        .init_resource::<HudPointerOverUi>()
        .init_resource::<Actions>()
        .init_resource::<DevOptions>()
        .init_resource::<SendState>()
        .init_resource::<PlayerInput>()
        .init_resource::<LocalJump>()
        .init_resource::<MountReason>()
        .init_resource::<Teleport>()
        .init_resource::<Check>()
        .add_message::<NetEvent>()
        .add_message::<NetCommand>()
        .add_systems(Startup, stage)
        .add_systems(
            Update,
            crate::ingame::player::control_player.before(super::super::super::sync_hud),
        );
    app.add_systems(Update, super::super::super::sync_hud);
    super::super::plugin(&mut app);
    app.add_systems(Update, check.after(super::super::update));
    let code = app.run();
    if code != AppExit::Success {
        eprintln!("mount_walk: FAILED");
    }
}

/// The HUD scripts and the pet API over a real `GameData`, as the app builds them (no socket).
fn runtime(
    width: f32,
    height: f32,
    db: &str,
) -> Result<(UiRuntime, Shared), Box<dyn std::error::Error>> {
    let mut rt = UiRuntime::new("en_us", width, height)?;
    rt.load_ui_texts(Path::new(db))?;
    let bridge: Shared = Rc::new(RefCell::new(HudData::default()));
    {
        let data = bridge.borrow_mut();
        data.game
            .0
            .set(GameData::load(db)?)
            .map_err(|_| "the game data is already set")?;
    }
    api::install(&mut rt, &bridge)?;
    rt.run_stage(aa_ui::Stage::World);
    if !rt.errors.is_empty() {
        return Err(format!("the HUD scripts reported {:?}", rt.errors).into());
    }
    api::enter_world(&rt, &bridge, &aa_net::state::Chat::default());
    Ok((rt, bridge))
}

/// The local player the HUD finds by name and the camera the walk steers.
fn stage(
    mut commands: Commands,
    staged: Res<Staged>,
    mut units: ResMut<Units>,
    mut mates: ResMut<Mates>,
) {
    let unit = units
        .by_id
        .get_mut(&PLAYER_OBJ)
        .expect("the active pet fixture's player");
    unit.pos = aa_net::Pos::new(staged.player.x, staged.player.y, staged.player.z);
    let mate = units
        .by_id
        .get_mut(&MATE_OBJ)
        .expect("the active pet fixture's mate");
    let player = staged.player;
    mate.pos = aa_net::Pos::new(player.x, player.y + FAR, player.z);
    if let Some(own) = mates.own.as_mut() {
        own.obj_id = Some(MATE_OBJ);
    }
    commands.spawn((
        Name::new("player Expansion fixture"),
        LocalPlayer { obj_id: PLAYER_OBJ },
        Mover {
            pos: player,
            facing: 0.0,
            vz: 0.0,
            grounded: true,
            velocity: Vec3::ZERO,
            swim: None,
            wading: 0.0,
        },
        Transform::from_translation(aa_world::cry_to_bevy(player)),
        Motion::default(),
    ));
    commands.spawn(OrbitCamera::default());
}

/// Moves the player and mate to `mate_distance` metres north of the staged player; the HUD state is updated too so a
/// press in this frame reads them.
#[allow(clippy::too_many_arguments)]
fn place(
    staged: Staged,
    mate_distance: f32,
    data: &mut HudData,
    movers: &mut Query<(&mut Mover, &mut Transform), With<LocalPlayer>>,
    units: &mut Units,
) {
    let player = staged.player;
    let mate = Vec3::new(player.x, player.y + mate_distance, player.z);
    if let Ok((mut mover, mut transform)) = movers.single_mut() {
        mover.pos = player;
        mover.facing = 0.0;
        mover.velocity = Vec3::ZERO;
        *transform = Transform::from_translation(aa_world::cry_to_bevy(player));
    }
    if let Some(unit) = units.by_id.get_mut(&MATE_OBJ) {
        unit.pos = aa_net::Pos::new(mate.x, mate.y, mate.z);
    }
    data.pos = player;
    data.pets.owned.pet_pos = Some((mate, 0.0));
    data.pets.owned.approach = None;
    data.pets.owned.last_ride = None;
    data.pets.owned.requests.clear();
    data.pets.release_w = false;
}

/// Press the real `X2Mate:MountMate()` binding.
fn press(rt: &UiRuntime) -> Result<(), String> {
    rt.lua
        .load("X2Mate:MountMate()")
        .set_name("@mount_walk_press")
        .exec()
        .map_err(|error| error.to_string())
}

/// Runs both cases: the 10 m walk (forward key, shrinking distance, 3 m stop, one `CSMountMate`) and the 2.5 m press.
#[allow(clippy::too_many_arguments)]
fn check(
    time: Res<Time>,
    mut check: ResMut<Check>,
    mut exit: MessageWriter<AppExit>,
    mut sent: MessageReader<NetCommand>,
    bridge: NonSendMut<HudBridge>,
    rt: Option<NonSend<UiRuntime>>,
    dev: Res<DevOptions>,
    staged: Res<Staged>,
    mut movers: Query<(&mut Mover, &mut Transform), With<LocalPlayer>>,
    mut units: ResMut<Units>,
) {
    for NetCommand(command) in sent.read() {
        if matches!(command, Command::Mate(MateCommand::Mount { .. })) {
            check.sent += 1;
        }
    }
    let Some(rt) = rt else {
        check.fail("no HUD runtime: the mount press cannot run".into());
        return finish(&mut check, &mut exit);
    };
    let now = time.elapsed_secs();
    match check.phase {
        Phase::Waiting => {
            if now < PRESS_AFTER {
                return;
            }
            {
                let mut data = bridge.0.borrow_mut();
                if data.pets.owned.current.is_none() || data.game.get().is_none() {
                    return;
                }
                if data.pets.owned.pet_pos.is_none() {
                    return;
                }
                place(*staged, FAR, &mut data, &mut movers, &mut units);
                check.start = FAR;
                check.last = FAR;
            }
            if let Err(error) = press(&rt) {
                check.fail(format!("the far mount press failed: {error}"));
                return finish(&mut check, &mut exit);
            }
            check.pressed_at = now;
            check.frames = 0;
            check.saw_w = false;
            check.sent = 0;
            let walking = bridge.0.borrow().pets.owned.approach.is_some();
            if !walking {
                check.fail("a mate 10 m away did not start the walk".into());
                return finish(&mut check, &mut exit);
            }
            check.phase = Phase::Walking;
        }
        Phase::Walking => {
            let (sent, start, last) = (check.sent, check.start, check.last);
            check.frames += 1;
            if now - check.pressed_at > TIMEOUT {
                check.fail(format!("the 10 m walk did not arrive within {TIMEOUT} s"));
                return finish(&mut check, &mut exit);
            }
            let (approach, dist, requests) = {
                let data = bridge.0.borrow();
                let dist = data
                    .pets
                    .owned
                    .pet_pos
                    .map_or(f32::INFINITY, |(mate, _)| data.pos.distance(mate));
                (
                    data.pets.owned.approach.is_some(),
                    dist,
                    data.pets.owned.requests.len(),
                )
            };
            if check.sent == 0 {
                if dev.script_keys.contains('W') {
                    check.saw_w = true;
                }
                if !check.saw_w && check.frames >= 3 {
                    check.fail("the walk did not hold the scripted forward key".into());
                    return finish(&mut check, &mut exit);
                }
                if now - check.pressed_at >= PROGRESS_SECS && start - dist < PROGRESS {
                    check.fail(format!(
                        "the walk did not get nearer: {dist:.2} m from {start:.2} m after {PROGRESS_SECS} s"
                    ));
                    return finish(&mut check, &mut exit);
                }
            }
            if approach {
                check.last = dist;
                return;
            }
            if check.sent != 1 || requests != 0 {
                check.fail(format!(
                    "the arrival sent {} CSMountMate ({} queued), not exactly one",
                    sent, requests
                ));
                return finish(&mut check, &mut exit);
            }
            if !check.saw_w {
                check.fail("the walk never held the scripted forward key".into());
            }
            if !(check.last > STOP && dist <= STOP) {
                check.fail(format!(
                    "the walk stopped at {dist:.2} m (last frame {last:.2} m), not across the {STOP} m stop"
                ));
            }
            if dev.script_keys.contains('W') {
                check.fail("the arrival left the scripted forward key held".into());
            }
            if !check.failures.is_empty() {
                return finish(&mut check, &mut exit);
            }
            // second case: the mate is already inside the send range
            {
                let mut data = bridge.0.borrow_mut();
                place(*staged, NEAR, &mut data, &mut movers, &mut units);
                check.start = NEAR;
                check.last = NEAR;
            }
            if let Err(error) = press(&rt) {
                check.fail(format!("the close mount press failed: {error}"));
                return finish(&mut check, &mut exit);
            }
            check.pressed_at = now;
            check.frames = 0;
            check.saw_w = false;
            check.sent = 0;
            let data = bridge.0.borrow();
            if data.pets.owned.approach.is_some() {
                check.fail("a mate 2.5 m away started a walk".into());
            }
            if dev.script_keys.contains('W') {
                check.fail("the close press held the scripted forward key".into());
            }
            drop(data);
            if !check.failures.is_empty() {
                return finish(&mut check, &mut exit);
            }
            check.phase = Phase::Close;
        }
        Phase::Close => {
            if now - check.pressed_at > TIMEOUT {
                check.fail(format!("the close mount did not send within {TIMEOUT} s"));
                return finish(&mut check, &mut exit);
            }
            if check.sent == 0 {
                return;
            }
            let sent = check.sent;
            if sent != 1 {
                check.fail(format!("the close mount sent {sent} CSMountMate, not one"));
            }
            if bridge.0.borrow().pets.owned.approach.is_some() {
                check.fail("the close mount left a walk running".into());
            }
            if dev.script_keys.contains('W') {
                check.fail("the close mount left the scripted forward key held".into());
            }
            check.phase = Phase::Done;
            finish(&mut check, &mut exit);
        }
        Phase::Done => finish(&mut check, &mut exit),
    }
}

fn finish(check: &mut Check, exit: &mut MessageWriter<AppExit>) {
    if check.phase == Phase::Done {
        info!(
            "mount-walk: passed: walked 10 m to {STOP} m and sent one CSMountMate; 2.5 m sent one at once"
        );
        exit.write(AppExit::Success);
    } else if !check.failures.is_empty() {
        for failure in &check.failures {
            error!("mount-walk: {failure}");
        }
        exit.write(AppExit::error());
        check.phase = Phase::Done;
    }
}
