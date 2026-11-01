//! Self-checking walk to the mount: the real HUD binding (`X2Mate:MountMate()`), the real `walk_to_mount` /
//! `update` systems and the real on-foot controller over received mate state, with no socket.
//!
//! Cases, all through the real binding:
//! * a mate 30 m away is refused at the request ("Target is too far.", no walk, no request, no cooldown), the
//!   `FUN_3935aa10` 25 m rule;
//! * a mate 4 m away starts the walk, never queues `CSMountMate` while the distance is still over the 0.8 m arrival
//!   window (`FUN_390aa730`, `0x399A3240` squared), sends exactly one at the window and releases the forward key;
//!   the ride cooldown keeps the instant stamped at the accepted request (`FUN_393642f0` line 701802), so a second
//!   in-window press 2 s after the accept sends at once while the send itself is younger than 2 s;
//! * a mate 2.5 m away starts the walk too and sends at the window (the port used to send at once under 3 m).
//!
//! `cargo run -p aa-client --example mount_walk -- --headless --headless-fps 240` (needs `AA_PAK` and `AA_DB`).
//! Add `--mute` in a shared session. The run exits non-zero when a case is wrong.
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

/// The 1.2 arrival window (`0x399A3240` = 0.64 squared) and the `FUN_3909b070` 3 m send gate (`0x39AC3878` = 9.0).
const STOP: f32 = 0.8;
const SEND_GATE: f32 = 3.0;
/// The mate distances (m) for the walk, the in-window press and the refused request.
const FAR: f32 = 4.0;
const NEAR: f32 = 2.5;
const TOO_FAR: f32 = 30.0;
/// The native ride cooldown (`1999 < now - _DAT_3a169944`).
const RIDE_COOLDOWN: f32 = 2.0;
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

/// The staged positions: the player at [`AT`] on the terrain, the mate placed by [`place`].
#[derive(Resource, Clone, Copy)]
struct Staged {
    player: Vec3,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    /// Waiting for the collision settle, then the refused 30 m request and the 4 m walk.
    Waiting,
    /// The 4 m walk.
    Far,
    /// Waiting for the accepted request's 2 s cooldown, then pressing again at the arrival window.
    AfterFar,
    /// Waiting for that in-window request to reach `commands`.
    Probe,
    /// The 2.5 m walk.
    Near,
    /// Every case passed.
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
    saw_between: bool,
    arrived_at: f32,
    accepted: Option<web_time::Instant>,
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
            saw_between: false,
            arrived_at: 0.0,
            accepted: None,
            failures: Vec::new(),
        }
    }
}

impl Check {
    fn fail(&mut self, failure: String) {
        self.failures.push(failure);
    }
}

pub(crate) fn run() -> AppExit {
    if !aa_cry::pak_available() {
        eprintln!("mount_walk: skipped, no game_pak (set AA_PAK)");
        return AppExit::Success;
    }
    let db = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
    if !Path::new(&db).exists() {
        eprintln!("mount_walk: skipped, no game database at {db} (set AA_DB)");
        return AppExit::Success;
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
    code
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
/// press in this frame reads them. The ride cooldown is left alone: it belongs to the accepted request, not to the
/// staging.
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

/// Runs the 30 m refusal, the 4 m walk, the cooldown check at the window and the 2.5 m walk through the real
/// `X2Mate:MountMate` binding, `walk_to_mount`, `pets::update` and `control_player`.
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
                place(*staged, TOO_FAR, &mut data, &mut movers, &mut units);
            }
            if let Err(error) = rt
                .lua
                .load("CAPTURED={} SYSMSG=function(text) CAPTURED[#CAPTURED+1]=text end")
                .set_name("@mount_walk_messages")
                .exec()
            {
                check.fail(format!("the system-message capture failed: {error}"));
                return finish(&mut check, &mut exit);
            }
            if let Err(error) = press(&rt) {
                check.fail(format!("the 30 m mount press failed: {error}"));
                return finish(&mut check, &mut exit);
            }
            {
                let data = bridge.0.borrow();
                if data.pets.owned.approach.is_some() {
                    check.fail("a mate 30 m away started the walk".into());
                }
                if !data.pets.owned.requests.is_empty() {
                    check.fail("a mate 30 m away queued a mount request".into());
                }
                if data.pets.owned.last_ride.is_some() {
                    check.fail("a refused request stamped the ride cooldown".into());
                }
            }
            let text = rt
                .lua
                .load("return CAPTURED[1]")
                .eval::<Option<String>>()
                .ok()
                .flatten();
            if text.as_deref() != Some("Target is too far.") {
                check.fail(format!("the 30 m refusal showed {text:?}, not \"Target is too far.\""));
            }
            {
                let mut data = bridge.0.borrow_mut();
                place(*staged, FAR, &mut data, &mut movers, &mut units);
            }
            if let Err(error) = press(&rt) {
                check.fail(format!("the 4 m mount press failed: {error}"));
                return finish(&mut check, &mut exit);
            }
            {
                let data = bridge.0.borrow();
                if data.pets.owned.approach.is_none() {
                    check.fail("a mate 4 m away did not start the walk".into());
                }
                check.accepted = data.pets.owned.last_ride;
                if check.accepted.is_none() {
                    check.fail("the accepted 4 m request did not stamp the ride cooldown".into());
                }
            }
            check.pressed_at = now;
            check.frames = 0;
            check.start = FAR;
            check.last = FAR;
            check.saw_w = false;
            check.saw_between = false;
            check.sent = 0;
            check.phase = Phase::Far;
            if !check.failures.is_empty() {
                finish(&mut check, &mut exit);
            }
        }
        Phase::Far | Phase::Near => {
            check.frames += 1;
            if now - check.pressed_at > TIMEOUT {
                check.fail(format!("the walk did not arrive within {TIMEOUT} s"));
                return finish(&mut check, &mut exit);
            }
            let (approach, dist, requests, ride) = {
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
                    data.pets.owned.last_ride,
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
                if now - check.pressed_at >= PROGRESS_SECS && check.start - dist < PROGRESS {
                    let start = check.start;
                    check.fail(format!(
                        "the walk did not get nearer: {dist:.2} m from {start:.2} m after {PROGRESS_SECS} s"
                    ));
                    return finish(&mut check, &mut exit);
                }
                if dist <= SEND_GATE && dist > STOP + 0.05 {
                    check.saw_between = true;
                    if requests != 0 {
                        check.fail(format!("the walk queued CSMountMate at {dist:.2} m, before the 0.8 m window"));
                        return finish(&mut check, &mut exit);
                    }
                }
            }
            if approach {
                check.last = dist;
                return;
            }
            if check.sent != 1 || requests != 0 {
                let sent_count = check.sent;
                check.fail(format!(
                    "the arrival sent {sent_count} CSMountMate ({requests} queued), not exactly one"
                ));
                return finish(&mut check, &mut exit);
            }
            if !check.saw_between {
                check.fail("the walk jumped over the 0.8..3 m band: the no-3 m send was not sampled".into());
            }
            if !check.saw_w {
                check.fail("the walk never held the scripted forward key".into());
            }
            if !(check.last > STOP && dist <= STOP + 0.2) {
                let last = check.last;
                check.fail(format!(
                    "the walk stopped at {dist:.2} m (last frame {last:.2} m), not across the {STOP} m window"
                ));
            }
            if dev.script_keys.contains('W') {
                check.fail("the arrival left the scripted forward key held".into());
            }
            if check.accepted != ride {
                let a = check.accepted.map_or(-1.0, |t| t.elapsed().as_secs_f32());
                let r = ride.map_or(-1.0, |t| t.elapsed().as_secs_f32());
                check.fail(format!(
                    "the send restamped the ride cooldown (accept {a:.3} s ago, send {r:.3} s ago); FUN_3909b070 stamps nothing"
                ));
            }
            if ride.is_none_or(|t| t.elapsed().as_secs_f32() < 0.2) {
                check.fail("the ride cooldown was not stamped at the accepted request".into());
            }
            if !check.failures.is_empty() {
                return finish(&mut check, &mut exit);
            }
            if check.phase == Phase::Near {
                check.phase = Phase::Done;
                return finish(&mut check, &mut exit);
            }
            check.arrived_at = now;
            check.phase = Phase::AfterFar;
        }
        Phase::AfterFar => {
            let ride = bridge.0.borrow().pets.owned.last_ride;
            if ride.is_some_and(|t| t.elapsed().as_secs_f32() < RIDE_COOLDOWN) {
                return;
            }
            let since_arrival = now - check.arrived_at;
            if since_arrival >= RIDE_COOLDOWN {
                check.fail(format!(
                    "the 4 m walk took {since_arrival:.2} s, so this press cannot separate an accept stamp from a send stamp"
                ));
                return finish(&mut check, &mut exit);
            }
            if let Err(error) = press(&rt) {
                check.fail(format!("the in-window mount press failed: {error}"));
                return finish(&mut check, &mut exit);
            }
            {
                let data = bridge.0.borrow();
                if data.pets.owned.approach.is_some() {
                    check.fail("an in-window press started a walk".into());
                }
                if data.pets.owned.requests.is_empty() {
                    check.fail("the press after the accept's cooldown queued no CSMountMate".into());
                }
            }
            check.pressed_at = now;
            check.sent = 0;
            check.phase = Phase::Probe;
            if !check.failures.is_empty() {
                finish(&mut check, &mut exit);
            }
        }
        Phase::Probe => {
            if now - check.pressed_at > TIMEOUT {
                check.fail(format!("the in-window mount did not send within {TIMEOUT} s"));
                return finish(&mut check, &mut exit);
            }
            if check.sent == 0 {
                return;
            }
            if check.sent != 1 {
                let sent_count = check.sent;
                check.fail(format!("the in-window press sent {sent_count} CSMountMate, not one"));
                return finish(&mut check, &mut exit);
            }
            if ride_not_rested(&bridge) {
                return;
            }
            {
                let mut data = bridge.0.borrow_mut();
                place(*staged, NEAR, &mut data, &mut movers, &mut units);
            }
            if let Err(error) = press(&rt) {
                check.fail(format!("the 2.5 m mount press failed: {error}"));
                return finish(&mut check, &mut exit);
            }
            {
                let data = bridge.0.borrow();
                if data.pets.owned.approach.is_none() {
                    check.fail("a mate 2.5 m away did not start the walk".into());
                }
                if !data.pets.owned.requests.is_empty() {
                    check.fail("a mate 2.5 m away sent CSMountMate at once".into());
                }
                check.accepted = data.pets.owned.last_ride;
            }
            check.pressed_at = now;
            check.frames = 0;
            check.start = NEAR;
            check.last = NEAR;
            check.saw_w = false;
            check.saw_between = false;
            check.sent = 0;
            check.phase = Phase::Near;
            if !check.failures.is_empty() {
                finish(&mut check, &mut exit);
            }
        }
        Phase::Done => finish(&mut check, &mut exit),
    }
}

fn ride_not_rested(bridge: &HudBridge) -> bool {
    bridge
        .0
        .borrow()
        .pets
        .owned
        .last_ride
        .is_some_and(|t| t.elapsed().as_secs_f32() < RIDE_COOLDOWN)
}

fn finish(check: &mut Check, exit: &mut MessageWriter<AppExit>) {
    if !check.failures.is_empty() {
        for failure in &check.failures {
            error!("mount-walk: {failure}");
        }
        exit.write(AppExit::error());
        check.phase = Phase::Done;
        return;
    }
    if check.phase == Phase::Done {
        info!(
            "mount-walk: passed: 30 m refused; 4 m walked past 3 m to {STOP} m (one CSMountMate, cooldown at the accept); 2.5 m walked to {STOP} m"
        );
        exit.write(AppExit::Success);
    }
}
