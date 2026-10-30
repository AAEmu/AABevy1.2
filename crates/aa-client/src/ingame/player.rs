//! The local player: spawned from the server's data, moved by the keyboard, standing on the terrain and on
//! the world's brushes (bridges, stairs), stopped by walls, swimming in deep water, and reported to the server.
//!
//! Keys (ArcheAge's `default_binding.g`), as 1.2's on-foot input reads them ([`resolve_keys`]): W / S forward / back, Q / E
//! strafe left / right, A / D turn a quarter turn a second (strafe while a mouse button is held), Space jump (swim up in the
//! water), X swim down, `.` toggles walking, NumLock auto-run ([`AutoRun`]: once W, S and both mouse buttons were let go, the
//! next press of any of them ends it). Both mouse buttons held run forward. Shift held also walks (a convenience, the real
//! client has none). The mouse drives the camera ([`super::camera`]); with the right button held the character faces where the
//! camera looks; the left button turns the camera alone, also on the run (free look: [`face_camera`]); without a button the
//! character turns to the camera heading at once when it starts to move (1.2's `fold_yaw_offset`). Dash's double tap and speed are [`super::sprint`]'s.
//!
//! Collision ([`aa_world::WorldCollision`]): every step slides the character's capsule along the walls, then stands it on the
//! highest surface under it, terrain or brush, within a step height. Terrain steeper than the actor's climb angle cannot be
//! walked up. The server stays authoritative; the client sends where it thinks it is.
//!
//! Swimming ([`super::swim`]): water deeper than [`ActorParams::swim_depth`] (1.3 m) over the ground makes the character swim
//! at the surface; the packets then carry the swim stance and no ground flag.
//!
//! Riding ([`super::mount`]): on the driver's seat of its own mate the keys move the mount at the mount's speed (no jumping, no
//! swimming) and the packets carry the mate's object id; a passenger is carried along.

use aa_anim::pose::Rig;
use aa_anim::{Cal, CharacterAssets, ClipLibrary, spawn_character_with_facial};
use aa_cry::Skeleton;
use aa_cry::scene::CryAssets;
use aa_net::model::stance;
use aa_net::state::Player;
use aa_net::{Command, Event, MoveCommand, MoveState, NetCommand, NetEvent, Pos};
use aa_world::{TerrainQuery, WaterQuery, WorldCollision, cry_to_bevy};
use bevy::prelude::*;

use super::anim::{ClipSet, LocalJump, Locomotion, Motion, RUN_SPEED};
use super::appearance;
use super::camera::OrbitCamera;
use super::data::Db;
use super::dev::DevOptions;
use super::swim::{self, ActorParams, Swim, SwimClips, Wet};
use super::weapons::{self, CarriedItem};

/// Horizontal world velocity carried from a ship deck after stepping or jumping off it;
/// control_player adds it while airborne and removes it on landing.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq)]
pub struct CarriedVelocity(pub Vec2);

/// The HUD's keyboard/pointer focus, grouped to keep control_player within Bevy's
/// system-parameter limit.
#[derive(bevy::ecs::system::SystemParam)]
pub struct HudFocus<'w> {
    typing: Option<Res<'w, crate::hud::HudKeyboardFocus>>,
    over_ui: Option<Res<'w, crate::hud::HudPointerOverUi>>,
}

/// Marks the local player's character entity.
#[derive(Component)]
pub struct LocalPlayer {
    pub obj_id: u32,
}

/// Where the player is, in game coordinates (x east, y north, z up) and facing (radians, 0 = north,
/// counter-clockwise).
#[derive(Component)]
pub struct Mover {
    pub pos: Vec3,
    pub facing: f32,
    /// Vertical velocity (m/s).
    pub vz: f32,
    pub grounded: bool,
    /// World velocity of the last step (m/s), for the movement packets.
    pub velocity: Vec3,
    /// In the water and swimming (`Some`), else walking or in the air.
    pub swim: Option<Swim>,
    /// Water over the ground under the feet (m): wading slows the character down.
    pub wading: f32,
}

/// What the player's input did in the last step: moving, jumping and turning cancel some casts ([`super::combat`]).
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerInput {
    /// The character walked or ran (a movement key was held and it went somewhere).
    pub moving: bool,
    /// The jump started in this step.
    pub jumped: bool,
    /// The facing changed (A / D, the right mouse button, the swing to the camera heading).
    pub turned: bool,
}

/// Gravity and jump speed come from the actor model (`physics_living_gravity` 25, `game_jump_height` 1.3: [`ActorParams`]).
/// The capsule that collides with the walls (m), the tallest step it climbs, and how far it may drop before it falls.
pub(super) const RADIUS: f32 = 0.35;
pub(super) const HEIGHT: f32 = 1.8;
pub(super) const STEP_UP: f32 = 0.5;
const DROP: f32 = 0.6;
/// `physics_living_max_climb_angle` 50 degrees: terrain that rises faster than this over [`CLIMB_LOOK`] metres ahead cannot be
/// walked up (tan 50).
const MAX_CLIMB_TAN: f32 = 1.19;
const CLIMB_LOOK: f32 = 0.5;
/// Turning with A / D (radians per second): 1.2's `input::KeyboardTurnRate` (x2game+0x412e00) turns pi x `keyboard_rotate_speed`
/// (0.5) a second on foot, a quarter turn, by the frame's time capped at [`TURN_STEP_CAP`].
pub(super) const TURN_RATE: f32 = std::f32::consts::FRAC_PI_2;
pub(super) const TURN_STEP_CAP: f32 = 0.033;
/// Seconds after the character settles or is teleported during which a landing is not reported (the server applies fall damage
/// from it, and a spawn above a bridge that is not built yet must not hurt).
const LANDING_GRACE: f32 = 1.5;
/// The longest the first step waits for the collision around the spawn to be built (seconds).
const COLLISION_WAIT: f32 = 5.0;
/// Interval between movement packets while moving.
const SEND_INTERVAL: f32 = 0.1;

/// Unit vectors of a game yaw: forward (north at 0, counter-clockwise) and right.
pub fn forward(yaw: f32) -> Vec2 {
    Vec2::new(-yaw.sin(), yaw.cos())
}

pub fn right(yaw: f32) -> Vec2 {
    Vec2::new(yaw.cos(), yaw.sin())
}

/// Heightmap cells within this many metres of the player are loaded ahead of use.
const PREFETCH_RADIUS: f32 = 600.0;

/// Physical terrain under a game position, or `fallback` over holes/outside the map.
pub fn height_or(terrain: &TerrainQuery, x: f32, y: f32, fallback: f32) -> f32 {
    terrain.ground_at(x, y).map_or(fallback, |ground| ground.height)
}

pub fn placement(pos: Vec3, yaw: f32, scale: f32) -> Transform {
    Transform { translation: cry_to_bevy(pos), rotation: Quat::from_rotation_y(yaw), scale: Vec3::splat(scale) }
}

/// The player character was built (or could not be) for the current spawn; reset when the game screen is left.
#[derive(Resource, Default)]
pub struct PlayerBuilt(pub bool);

#[allow(clippy::too_many_arguments)]
/// Builds the player character from the server's spawn data, once, as soon as the spawn is known: on the loading screen
/// (the world is drawn behind it, see `camera::spawn_camera`), or on the game screen for `--enter-world`.
pub fn spawn_player(
    mut commands: Commands,
    player: Res<Player>,
    mut built: ResMut<PlayerBuilt>,
    db: Option<Res<Db>>,
    mut cry: CryAssets,
    clips: Res<ClipLibrary>,
    terrain: Res<TerrainQuery>,
    dev: Res<DevOptions>,
) {
    let Some(spawn) = &player.spawn else { return };
    if built.0 {
        return;
    }
    built.0 = true;
    let Some(db) = db else {
        error!("the game database is missing: no player character");
        return;
    };
    let character = spawn.character();
    let p = spawn.position();
    let (px, py) = dev.pos.map_or((p.x, p.y), |d| (d.x, d.y));
    terrain.prefetch(px, py, PREFETCH_RADIUS);
    let ground = height_or(&terrain, px, py, p.z);
    // the server's height is kept when it is above the terrain by a bridge deck's worth (the first steps put the character on
    // what is under it once the collision around is built); `--pos x,y,z` gives one, `--pos x,y` stands on the terrain
    let saved = match dev.pos {
        Some(d) if d.z.is_finite() => d.z,
        Some(_) => ground,
        None if p.z.is_finite() && p.z >= ground && p.z - ground < 40.0 => p.z,
        None => ground,
    };
    let pos = Vec3::new(px, py, saved);
    let facing = dev.yaw.unwrap_or_else(|| spawn.yaw());
    info!(
        "player {} race {} gender {} model {} hair color {} skin color {} at ({:.1}, {:.1}, {:.1}) terrain {:.2} facing {:.1} deg",
        spawn.name(),
        character.race,
        character.gender,
        spawn.unit.model_id,
        character.model_params.hair_color,
        character.model_params.skin_color,
        p.x,
        p.y,
        p.z,
        ground,
        facing.to_degrees()
    );
    debug!(
        "model params: unit kind {} hair {} skin {} | character kind {} hair {} skin {}",
        spawn.unit.model_params.kind,
        spawn.unit.model_params.hair_color,
        spawn.unit.model_params.skin_color,
        character.model_params.kind,
        character.model_params.hair_color,
        character.model_params.skin_color
    );
    let mut params = if spawn.unit.model_params.kind > 0 { spawn.unit.model_params.clone() } else { character.model_params.clone() };
    if let Some(color) = dev.hair_color {
        params.hair_color = color;
        params.kind = params.kind.max(aa_net::model::ModelParams::HAIR);
    }
    let params = &params;
    let equipment = if spawn.unit.equipment.template_ids().iter().any(|&t| t != 0) { &spawn.unit.equipment } else { &character.equipment };
    for (i, item) in equipment.slots.iter().enumerate() {
        let Some(item) = item else { continue };
        let (image, dye) = match &item.detail {
            aa_net::model::ItemDetail::Equip(d) => (d.image_item_template_id, d.dye_item_id),
            _ => (0, 0),
        };
        debug!("equipment slot {i}: item {} (look {image}, dye item {dye})", item.template_id);
    }
    let game = db.game.lock().unwrap();
    let Some((race, gender)) = appearance::race_gender(spawn.unit.model_id, character.race, character.gender) else {
        error!("unsupported race {} / model {}", character.race, spawn.unit.model_id);
        return;
    };
    let Some(desc) = appearance::describe(&game, race, gender, equipment, params) else {
        error!("cannot describe the player character");
        return;
    };
    debug!("player look: {desc:?}");
    let assets = match appearance::resolve(&game, &desc, equipment) {
        Ok(a) => a,
        Err(e) => {
            error!("cannot resolve the player character: {e}");
            return;
        }
    };
    drop(game);
    let carried = weapons::carried(&db, equipment);
    let swim_clips = SwimClips::load(&assets.skeleton, &assets.cal, &clips);
    let actor = ActorParams::load(spawn.unit.model_id).unwrap_or_default();
    debug!("actor params: {actor:?}, swim clips: {}", swim_clips.is_some());
    let Some(root) = spawn_with_locomotion(&mut commands, &mut cry, &clips, assets, placement(pos, facing, 1.0), carried) else { return };
    commands.entity(root).insert((
        Name::new(format!("player {}", spawn.name())),
        LocalPlayer { obj_id: spawn.obj_id },
        // the time of day areas follow the player (the real client registers Mirage Isle's `tod_area_shopping_land` at
        // weight 1 for a player standing in it, `ss_debug_ui 4`; reference/ingame/mirage_isle/notes.md)
        aa_world::AreaToDSubject,
        Mover { pos, facing, vz: 0.0, grounded: true, velocity: Vec3::ZERO, swim: None, wading: 0.0 },
        actor,
        weapons::base_wield(&db, equipment),
        super::gear_refresh::Applied::new(spawn.unit.model_id, race, gender, equipment, params),
    ));
    if spawn.unit.hp <= 0 {
        commands.entity(root).insert(super::combat::Dead { at: f64::MIN / 4.0 });
    }
    if let Some(swim_clips) = swim_clips {
        commands.entity(root).insert(swim_clips);
    }
}

/// The local character's authored facial playback plus in-world locomotion clips.
pub fn spawn_with_locomotion(
    commands: &mut Commands,
    cry: &mut CryAssets,
    clips: &ClipLibrary,
    assets: CharacterAssets,
    transform: Transform,
    carried: Vec<CarriedItem>,
) -> Option<Entity> {
    let skeleton = assets.skeleton.clone();
    let cal = assets.cal.clone();
    let t = web_time::Instant::now();
    let root = match spawn_character_with_facial(commands, cry, clips, assets, transform) {
        Ok(r) => r,
        Err(e) => {
            error!("cannot spawn a character: {e}");
            return None;
        }
    };
    let t_character = t.elapsed().as_secs_f32() * 1000.0;
    // the animator's own rig is private to the entity; clip bindings only depend on the skeleton
    let set = Skeleton::load(&skeleton)
        .ok()
        .map(|s| std::sync::Arc::new(Rig::new(s)))
        .zip(Cal::load(&cal).ok().map(std::sync::Arc::new))
        .and_then(|(rig, cal)| ClipSet::load(&rig, cal, clips));
    debug!("spawn_character {t_character:.0} ms, locomotion clips {:.0} ms", t.elapsed().as_secs_f32() * 1000.0 - t_character);
    let mut entity = commands.entity(root);
    entity.insert(Motion::default());
    if !carried.is_empty() {
        entity.insert(weapons::Carried(carried));
    }
    if let Some(set) = set {
        entity.insert(Locomotion::new(set));
    }
    Some(root)
}

/// What is under a point and whether it is a brush (a deck, a stair) rather than the terrain.
#[derive(Clone, Copy, Debug)]
struct Support {
    height: f32,
    brush: bool,
}

/// The highest surface under `(x, y)` that is not above `z + step_up`: a brush (bridge, stairs, floor) or the terrain; the terrain
/// also where it is higher than that (the character is inside the ground and comes out on top of it). `None` off the map.
fn support(terrain: &TerrainQuery, collision: &WorldCollision, x: f32, y: f32, z: f32, step_up: f32) -> Option<Support> {
    let terr = terrain.ground_at(x, y).map(|ground| ground.height);
    match (terr, collision.ground_below(x, y, z, step_up)) {
        (terrain, Some(g)) if terrain.is_none_or(|height| g.height > height + 0.02) => Some(Support { height: g.height, brush: true }),
        (Some(height), _) => Some(Support { height, brush: false }),
        _ => None,
    }
}

/// The highest supported surface, without the on-foot "keep the current height"
/// fallback: `None` when neither the collision data nor the terrain has one.
pub(super) fn support_at(
    terrain: &TerrainQuery,
    collision: &WorldCollision,
    pos: Vec3,
    step_up: f32,
) -> Option<f32> {
    support(terrain, collision, pos.x, pos.y, pos.z, step_up).map(|support| support.height)
}

pub(super) fn floor_at(
    terrain: &TerrainQuery,
    collision: &WorldCollision,
    pos: Vec3,
    step_up: f32,
) -> f32 {
    support_at(terrain, collision, pos, step_up).unwrap_or_else(|| {
        if terrain.is_hole_at(pos.x, pos.y) == Some(true) {
            f32::NEG_INFINITY
        } else {
            pos.z
        }
    })
}

/// Keyboard movement, jumping, standing on the ground, swimming and the movement packets.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn control_player(
    time: Res<Time>,
    (keys, actions): (Res<ButtonInput<KeyCode>>, Res<crate::settings::Actions>),
    mouse: Res<ButtonInput<MouseButton>>,
    mut cameras: Query<&mut OrbitCamera>,
    mut player: Query<(
        Entity,
        &LocalPlayer,
        &mut Mover,
        &mut Transform,
        &mut Motion,
        Option<&ActorParams>,
        Has<super::combat::Dead>,
        Option<&super::skill_motion::SkillMove>,
        Has<super::seating::Seated>,
        Option<&mut super::mount::DismountMove>,
        Option<&super::mount::Rider>,
        Has<super::ship::DeckRide>,
        Option<&mut CarriedVelocity>,
    )>,
    stats: Option<Res<aa_net::state::PlayerStats>>,
    terrain: Res<TerrainQuery>,
    collision: Res<WorldCollision>,
    water: Res<WaterQuery>,
    mut out: MessageWriter<NetCommand>,
    mut commands: Commands,
    mut state: ResMut<SendState>,
    mut input_state: ResMut<PlayerInput>,
    dev: Res<DevOptions>,
    focus: HudFocus,
    (mut trace, sprint, mut local_jump, mount, mut dismount_trace): (Local<(f32, Option<bool>)>, Option<Res<super::sprint::Sprint>>, ResMut<LocalJump>, Option<Res<super::mount::LocalMount>>, Local<DismountTrace>),
) {
    let Ok((entity, local, mut mover, mut transform, mut motion, actor, dead, skill_moving, seated, dismount, rider, deck, carried)) = player.single_mut() else { return };
    let dt = time.delta_secs().min(0.1);
    // the scripted clock runs for every controller path, including the deck
    state.since_spawn += dt;
    // on a ship deck `ship::deck_control` walks, carries and reports the player
    // (and owns the walk / autorun / turn state while it rides); on the frame it
    // leaves the hull it has already taken that frame's step and input
    if deck || std::mem::take(&mut state.deck_exit) {
        return;
    }
    // a passenger goes where the mount takes it (`mount::drive_mount`); a driver's keys steer the mount: its speed, no jumping
    // or swimming, and the packets name the mount
    let mount = mount.as_deref().copied().unwrap_or_default();
    // `AA_DISMOUNT_TRACE=1`: the local rider's position every 100 ms for 1.5 s after a detach, with the mount it left
    // and the mount's facing, so a replay can measure the landing in the mount's frame.
    if dismount_trace.at.is_none()
        && dismount.is_some()
        && *dismount_trace.on.get_or_insert_with(|| std::env::var_os("AA_DISMOUNT_TRACE").is_some())
    {
        let step = dismount.as_deref().expect("just checked");
        dismount_trace.at = Some(0.0);
        dismount_trace.next = 0.0;
        dismount_trace.mount = step.mount();
        dismount_trace.yaw = step.yaw();
    }
    if let Some(at) = dismount_trace.at {
        let at = at + time.delta_secs();
        dismount_trace.at = Some(at);
        while dismount_trace.next <= 1.5 + 1e-4 && dismount_trace.next <= at + 1e-4 {
            let pos = dismount.as_deref().map_or(mover.pos, super::mount::DismountMove::pos);
            let (mount, yaw) = (dismount_trace.mount, dismount_trace.yaw);
            info!(
                "dismount-trace: t {:.1} pos ({:.3}, {:.3}, {:.3}) mount ({:.3}, {:.3}, {:.3}) yaw {:.4}",
                dismount_trace.next, pos.x, pos.y, pos.z, mount.x, mount.y, mount.z, yaw
            );
            dismount_trace.next += 0.1;
        }
        if at >= 1.5 {
            dismount_trace.at = None;
        }
    }
    // the get-off placement (`FUN_3909b4a0` / `FUN_390aaff0`) owns the character until it reaches the landing: the seat is
    // let go, the get-off clips play on the rider, and the 100 ms hand-off is the controller's own milestone. Its camera,
    // packets and input resume when the move ends.
    if mount.0.is_none()
        && let Some(mut step) = dismount
    {
        let dt = time.delta_secs().min(0.1);
        let handed_off = step.handed_off();
        let gravity = actor.copied().unwrap_or_default().gravity;
        let done = step.step(
            dt,
            gravity,
            |p| floor_at(&terrain, &collision, p, STEP_UP),
            |p, delta| collision.slide(p, delta, RADIUS, HEIGHT, STEP_UP).pos,
        );
        // after 100 ms the normal movement pipeline reports the placement, not the unmount controller
        if !handed_off && step.handed_off() {
            state.send_now();
        }
        mover.pos = step.pos();
        *transform = placement(mover.pos, mover.facing, 1.0);
        *motion = Motion::default();
        *input_state = PlayerInput::default();
        mover.velocity = Vec3::ZERO;
        state.autorun = AutoRun::default();
        if done {
            mover.grounded = true;
            mover.vz = 0.0;
            mover.swim = None;
            mover.velocity = Vec3::ZERO;
            state.send_now();
            // `track_local_mount` drops the component on its next run (this system is at Bevy's 16-parameter limit)
        }
        return;
    }
    // the get-off clips play on where the placement left the rider (`mount::step_dismounts`): the keys take over
    // once it is on foot
    if mount.passenger() || rider.is_some_and(|r| r.dismounting()) {
        *motion = Motion::default();
        *input_state = PlayerInput::default();
        mover.velocity = Vec3::ZERO;
        state.autorun = AutoRun::default();
        return;
    }
    // a cart driver's keys go to `cart::drive_cart` and a ship driver's to `ship::drive_ship`; the character
    // sends no on-foot movement and stays in its seat
    if mount.cart_driver().is_some() || mount.ship_driver().is_some() {
        *motion = Motion::default();
        *input_state = PlayerInput::default();
        mover.velocity = Vec3::ZERO;
        mover.vz = 0.0;
        mover.grounded = true;
        mover.swim = None;
        state.autorun = AutoRun::default();
        state.was_active = false;
        return;
    }
    let driving = mount.driving();
    // a skill moves the character (a charge, a back step, a knockback): `skill_motion::drive_local` plays it and sends the packets
    if let Some(skill_move) = skill_moving.filter(|_| !dead) {
        // A declined parented Blink/impulse does not swallow genuine exit keys.
        if !seated || skill_move.controller_activity() {
            *input_state = PlayerInput { moving: skill_move.controller_activity(), ..default() };
            state.autorun = AutoRun::default();
            return;
        }
    }
    if dead || stats.is_some_and(|s| s.obj_id == local.obj_id && s.dead) {
        *motion = Motion::default();
        *input_state = PlayerInput::default();
        mover.velocity = Vec3::ZERO;
        mover.vz = 0.0;
        mover.swim = None;
        state.autorun = AutoRun::default();
        state.was_active = false;
        return;
    }
    let Ok(mut orbit) = cameras.single_mut() else { return };
    let actor = actor.copied().unwrap_or_default();
    // scripted keys (`--walk`) count like pressed ones
    let t = state.since_spawn;
    // the first step waits for the collision around the character (a deck or a house is not there before it is built)
    if !state.settled && !seated {
        state.wait += dt;
        if collision.ready_around(mover.pos.x, mover.pos.y, 24.0) || state.wait > COLLISION_WAIT {
            state.settled = true;
            state.grace = LANDING_GRACE;
        } else {
            return;
        }
    }
    state.grace = (state.grace - dt).max(0.0);
    // an edit box of the HUD (the chat input) has the keyboard: no movement, no jump; a drag that began on the HUD does not turn us
    let typing = focus.typing.as_ref().is_some_and(|f| f.0);
    let over_ui = focus.over_ui.as_ref().is_some_and(|o| o.0);
    let pressed = |action: &str, c: char| (!typing && actions.pressed(action, 0)) || dev.scripted(t, c);
    let tapped = |action: &str| !typing && actions.just_pressed(action, 0);
    let key = |action: &str, c: char| pressed(action, c) as i32 as f32;

    // the walking toggle (`.`), the turn key pressed last, the world mouse buttons
    if tapped("toggle_walk") {
        state.walk = !state.walk;
    }
    if tapped("turnleft") {
        state.last_turn = 1;
    } else if tapped("turnright") {
        state.last_turn = -1;
    }
    let left_mouse = mouse.pressed(MouseButton::Left) && !over_ui;
    let right_mouse = mouse.pressed(MouseButton::Right) && !over_ui;
    let held = MoveKeys {
        forward: pressed("moveforward", 'W'),
        back: pressed("moveback", 'S'),
        left: pressed("moveleft", 'Q'),
        right: pressed("moveright", 'E'),
        turn_left: pressed("turnleft", 'A'),
        turn_right: pressed("turnright", 'D'),
    };
    // auto-run (NumLock): W, S or both mouse buttons end it once they were let go
    state.autorun.update(tapped("autorun"), held.forward || held.back || (left_mouse && right_mouse));
    let (fwd, strafe, turn) = resolve_keys(MoveKeys { forward: held.forward || state.autorun.on, ..held }, left_mouse, right_mouse, state.last_turn);
    let jump_held = pressed("jump", 'J') && driving.is_none();
    let jump_tapped = state.jump_tapped(&dev, t, tapped("jump")) && driving.is_none();
    let vertical = jump_held as i32 as f32 - key("down", 'X');

    // Read movement/character-yaw intent before HoldPosition prevents motion. A
    // successful UnbondRequested completion releases the seat; enqueue alone does not.
    // Space and camera-only orbit have no independently proven seat-exit binding.
    if seated {
        let moving = fwd != 0.0 || strafe != 0.0;
        let snap = face_camera(left_mouse, right_mouse, moving, state.asked_before) == FaceCamera::Snap;
        *input_state = PlayerInput {
            moving,
            jumped: false,
            turned: turn != 0.0 || (snap && (orbit.yaw - mover.facing).sin().atan2((orbit.yaw - mover.facing).cos()).abs() > 1e-4),
        };
        *motion = Motion::default();
        *local_jump = LocalJump::default();
        mover.velocity = Vec3::ZERO;
        state.autorun = AutoRun::default();
        return;
    }

    let before_facing = mover.facing;
    if turn != 0.0 {
        let d = turn * TURN_RATE * dt.min(TURN_STEP_CAP);
        mover.facing += d;
        orbit.yaw += d; // the camera turns with the character
    }
    let local_dir = Vec2::new(strafe, fwd).normalize_or_zero();
    // 1.2 turns the unit by the camera's offset at once and folds it into the heading (`camera::fold_yaw_offset`): the camera
    // stays where it is and the character faces where it looks
    if face_camera(left_mouse, right_mouse, local_dir != Vec2::ZERO, state.asked_before) == FaceCamera::Snap {
        mover.facing = orbit.yaw;
    }
    state.asked_before = local_dir != Vec2::ZERO;

    let old = mover.pos;
    let swimming = mover.swim.is_some();
    // local velocity (x right, y forward): walking and running as the actor model says, swimming at the swim speed
    let walking = walking_mode(
        state.walk,
        dev.scripted(t, 'H')
            || (!typing && (keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight))),
    );
    let local_vel = if let Some((_, speed)) = driving {
        // the mount's buffs (a gallop) speed it up
        local_dir * speed.of(local_dir, walking) * sprint.as_ref().map_or(1.0, |s| s.mount_speed_mul)
    } else if swimming {
        swim::local_velocity(local_dir)
    } else {
        // the own buffs' speed (Dash: 1.384 at its first level)
        ground_velocity(
            &actor,
            local_dir,
            walking,
            sprint.as_ref().map_or(1.0, |s| s.speed_mul),
            mover.wading,
        )
    };
    // a deck leave's hull speed: also on the first on-foot frame after a walk-off, which starts grounded at
    // the rail (the DROP rule below turns it into the fall); a landing removes it
    let carried_velocity = if mover.swim.is_some() {
        Vec2::ZERO
    } else {
        carried.as_ref().map_or(Vec2::ZERO, |carried| carried.0)
    };
    let step = forward(mover.facing) * local_vel.y
        + right(mover.facing) * local_vel.x
        + carried_velocity;
    let mut pos = old;
    // load the neighbouring heightmap cells in the background when a cell border is crossed
    let cell = ((pos.x / aa_world::CELL_SIZE).floor() as i32, (pos.y / aa_world::CELL_SIZE).floor() as i32);
    if state.cell != cell {
        state.cell = cell;
        terrain.prefetch(pos.x, pos.y, PREFETCH_RADIUS);
    }
    // walls: slide along them; stay on the map
    if step != Vec2::ZERO {
        let candidate = collision.slide(pos, step * dt, RADIUS, HEIGHT, STEP_UP).pos;
        if terrain.height_at(candidate.x, candidate.y).is_some() {
            // terrain steeper than the climb angle is not walked up: looked at over half a metre ahead, the facets of the
            // mesh are rougher than that (a brush under the feet, stairs, is not terrain)
            let ahead = Vec2::new(candidate.x, candidate.y) + step.normalize_or_zero() * CLIMB_LOOK;
            let rise = terrain.ground_at(ahead.x, ahead.y).map_or(0.0, |ground| ground.height - pos.z);
            let climbing = !swimming && mover.grounded && rise > CLIMB_LOOK * MAX_CLIMB_TAN;
            let on_brush = climbing && support(&terrain, &collision, candidate.x, candidate.y, pos.z, STEP_UP).is_some_and(|s| s.brush);
            if !climbing || on_brush {
                pos.x = candidate.x;
                pos.y = candidate.y;
            }
        }
    }

    // what is under the feet and the water around
    let step_up = if mover.grounded || swimming { STEP_UP } else { 0.05 };
    let ground = floor_at(&terrain, &collision, Vec3::new(pos.x, pos.y, old.z), step_up);
    let wet = water.water_at(pos).map(|(surface, kind)| Wet { surface, kind, depth: (surface - ground).max(0.0) });
    mover.wading = if swimming { 0.0 } else { wet.map_or(0.0, |w| w.depth) };
    let rising = !mover.grounded && mover.vz > 0.0 && !swimming;
    let mut jumped = false;
    // swimming starts and stops with the depth of the water over the ground (a body that rises out of it does not start)
    let want_swim = swim::should_swim(swimming, pos.z, wet, actor.swim_depth()) && (swimming || !rising) && driving.is_none();
    if want_swim && !swimming {
        mover.grounded = false;
        let w = wet.expect("swimming needs water");
        mover.swim = Some(Swim { surface: w.surface, kind: w.kind, local_dir, speed: 0.0, vertical: 0.0, deep: false });
        info!("swimming in {:?} water at ({:.1}, {:.1}, {:.2}), surface {:.2}, ground {:.2}", w.kind, pos.x, pos.y, pos.z, w.surface, ground);
    } else if !want_swim && swimming {
        mover.swim = None;
        mover.grounded = false;
        mover.vz = mover.vz.min(0.0);
        info!("left the water at ({:.1}, {:.1}, {:.2}), ground {:.2}", pos.x, pos.y, pos.z, ground);
    }

    // impact speed (m/s) of a landing in this step: sent once, in the movement packet of the landing
    let mut landed = None;
    if let (Some(w), true) = (wet, mover.swim.is_some()) {
        // swimming: float at the surface, dive with X, surface with Space; a jump near the surface leaves the water
        let rest = w.surface - actor.swim_depth();
        if jump_tapped && pos.z >= rest - swim::JUMP_RANGE {
            mover.swim = None;
            mover.vz = actor.jump_speed();
            jumped = true;
        } else {
            let wanted = swim::wanted_vz(vertical, pos.z, rest);
            mover.vz = swim::approach(mover.vz, wanted, swim::WATER_DRAG, dt);
            pos.z = (pos.z + mover.vz * dt).max(ground + 0.05);
            if pos.z <= ground + 0.05 {
                mover.vz = mover.vz.max(0.0);
            }
            mover.swim = Some(Swim { surface: w.surface, kind: w.kind, local_dir, speed: local_vel.length(), vertical, deep: rest - pos.z > swim::DEEP });
        }
    }
    if mover.swim.is_none() {
        if mover.grounded && jump_tapped {
            mover.vz = actor.jump_speed();
            mover.grounded = false;
            jumped = true;
            state.took_off = true;
        }
        if mover.grounded {
            // follow the slope, the stairs and the decks; walking off a ledge turns into a fall
            if pos.z - ground > DROP {
                mover.grounded = false;
                mover.vz = 0.0;
            } else {
                pos.z = ground;
            }
        }
        if !mover.grounded {
            // water slows a fall down; the splash is no landing
            let in_water = wet.is_some_and(|w| pos.z < w.surface);
            let gravity = actor.gravity * if in_water { swim::WATER_GRAVITY } else { 1.0 };
            (pos.z, mover.vz) = air_step(pos.z, mover.vz, gravity, dt, if in_water { 4.0 } else { f32::INFINITY });
            if pos.z <= ground && mover.vz <= 0.0 {
                pos.z = ground;
                if state.grace <= 0.0 && !wet.is_some_and(|w| w.depth > swim::WADE_LO) {
                    landed = Some(-mover.vz);
                }
                mover.vz = 0.0;
                mover.grounded = true;
            }
        }
    }

    if mover.grounded || mover.swim.is_some() {
        // the carried ship speed lasts through the fall, not after landing
        if carried.is_some() {
            commands.entity(entity).remove::<CarriedVelocity>();
        }
    }

    // the step that was taken, not the one asked for: a wall takes the speed away
    let actual = if dt > 0.0 { Vec2::new(pos.x - old.x, pos.y - old.y) / dt } else { Vec2::ZERO };
    let swimming = mover.swim.is_some();
    let airborne = !mover.grounded && !swimming;
    let velocity = Vec3::new(actual.x, actual.y, if swimming || airborne { mover.vz } else { 0.0 });
    mover.velocity = velocity;
    mover.pos = pos;
    *transform = placement(pos, mover.facing, 1.0);
    let moving = local_dir != Vec2::ZERO && actual.length() > 0.05;
    // the jump's clips (`super::jump`): a take-off, walking, Dash and the keys' direction
    if !airborne {
        state.took_off = false;
    }
    // 1.2's freefall (move mode 1): fast and high over the terrain, held until the ground
    let terrain_height = terrain.ground_at(pos.x, pos.y).map_or(ground, |g| g.height);
    state.freefall = super::jump::freefall(state.freefall, airborne, -mover.vz, pos.z - terrain_height, state.took_off);
    let jump_now = LocalJump {
        freefall: state.freefall,
        rising: airborne && state.took_off,
        walking,
        sprint: sprint.as_ref().is_some_and(|s| s.dashing),
        dir: local_dir,
        speed_mul: sprint.as_ref().map_or(1.0, |s| s.speed_mul),
        sprint_motion: sprint.as_ref().is_some_and(|s| s.sprint_motion),
    };
    if local_jump.rising != jump_now.rising
        || local_jump.walking != jump_now.walking
        || local_jump.sprint != jump_now.sprint
        || local_jump.dir != jump_now.dir
        || local_jump.speed_mul != jump_now.speed_mul
        || local_jump.sprint_motion != jump_now.sprint_motion
        || local_jump.freefall != jump_now.freefall
    {
        *local_jump = jump_now;
    }
    // the swim clips are driven by `swim::drive_swim`; the locomotion sees a character standing still
    *motion = if swimming { Motion::default() } else { Motion { speed: if moving { actual.length() } else { 0.0 }, local_dir, airborne } };
    let turned = (mover.facing - before_facing).abs() > 1e-4;
    let input = PlayerInput { moving, jumped, turned };
    if *input_state != input {
        *input_state = input;
    }

    // movement packets: every SEND_INTERVAL while moving, facing, airborne or swimming about; one more when it stops,
    // one at once when the character lands and when it starts or stops swimming
    state.since_sent += dt;
    let swim_active = swimming && (moving || vertical != 0.0 || mover.vz.abs() > 0.05);
    let active = moving || airborne || turned || swim_active;
    let due = state.since_sent >= SEND_INTERVAL;
    let first = !state.sent_first && state.since_spawn > 0.5;
    let stance_changed = swimming != state.swimming;
    if (active && due) || (!active && state.was_active) || first || landed.is_some() || stance_changed {
        let to_wire = |v: f32| (v * 1024.0).round().clamp(-32767.0, 32767.0) as i16;
        // pressed direction relative to the facing, [right, forward, up]; forward is [0, 127, 0]
        let delta = [(local_dir.x * 127.0).round() as i8, (local_dir.y * 127.0).round() as i8, if swimming { (vertical * 127.0) as i8 } else { 0 }];
        // rising after a jump, falling otherwise in the air (and swimming: nothing under the feet), on the ground when standing
        let move_state = if mover.grounded {
            MoveState::Ground
        } else if mover.vz > 0.0 && !swimming {
            MoveState::Jump
        } else {
            MoveState::Fall
        };
        let mut cmd = MoveCommand {
            pos: Pos::new(pos.x, pos.y, pos.z),
            yaw: mover.facing,
            moving: active,
            stance: if swimming { stance::SWIM } else { stance::RELAXED },
            velocity: if active { [to_wire(velocity.x), to_wire(velocity.y), to_wire(velocity.z)] } else { [0; 3] },
            delta_movement: delta,
            ..MoveCommand::default()
        }
        .with_state(move_state);
        if let Some(speed) = landed {
            cmd = cmd.landing(speed);
        }
        out.write(NetCommand(match driving {
            Some((mate, _)) => Command::MoveUnit { obj_id: mate, movement: cmd },
            None => Command::Move(cmd),
        }));
        state.since_sent = 0.0;
        state.sent_first = true;
        state.moves_sent += 1;
        if state.moves_sent % 50 == 1 {
            debug!("move #{} to ({:.1}, {:.1}, {:.1}) for obj {}", state.moves_sent, pos.x, pos.y, pos.z, local.obj_id);
        }
    }
    state.swimming = swimming;
    state.was_active = active;
    // `AA_MOVE_TRACE=1`: where the character is, twice a second
    trace.0 += dt;
    if trace.0 >= 0.5 && *trace.1.get_or_insert_with(|| std::env::var_os("AA_MOVE_TRACE").is_some()) {
        trace.0 = 0.0;
        info!(
            "move: ({:.2}, {:.2}, {:.2}) ground {ground:.2} {} vz {:.2} speed {:.2}{}",
            pos.x,
            pos.y,
            pos.z,
            if swimming { "swimming" } else if mover.grounded { "grounded" } else { "airborne" },
            mover.vz,
            actual.length(),
            wet.map_or(String::new(), |w| format!(" water {:.2} deep {:.2}", w.surface, w.depth))
        );
    }
}

/// Bookkeeping of the movement packets and the movement toggles (reset when the game screen is left).
#[derive(Resource, Default)]
pub struct SendState {
    since_sent: f32,
    since_spawn: f32,
    sent_first: bool,
    was_active: bool,
    swimming: bool,
    moves_sent: u32,
    cell: (i32, i32),
    /// NumLock auto-run; `.` walking is on.
    pub(super) autorun: AutoRun,
    pub(super) walk: bool,
    /// The turn key pressed last (1 A, -1 D): it wins when both are held with a mouse button.
    pub(super) last_turn: i8,
    /// A direction was asked last frame ([`face_camera`] turns the character to the camera when one starts).
    pub(super) asked_before: bool,
    /// The air time began with a jump (not a fall off a ledge).
    took_off: bool,
    /// The scripted jump key was held last frame (the `hold J` / `--walk` edge).
    script_jump_held: bool,
    /// `ship::deck_control` took this frame's step off the hull ([`SendState::hand_over_from_deck`]).
    deck_exit: bool,
    /// In a freefall ([`super::jump::freefall`]).
    freefall: bool,
    /// The collision around the character is built (or the wait for it ran out), see [`COLLISION_WAIT`].
    settled: bool,
    wait: f32,
    /// Seconds left of the grace period after settling or a teleport in which a landing is not reported.
    grace: f32,
}

impl SendState {
    /// Reports the position in the next step, moving or not (getting on and off a mount).
    pub fn send_now(&mut self) {
        self.sent_first = false;
    }

    /// The script clock the `--walk` time windows use.
    pub(crate) fn since_spawn(&self) -> f32 {
        self.since_spawn
    }

    /// The next air time started with a take-off (a deck jump-off), not a fall.
    pub(crate) fn take_off(&mut self) {
        self.took_off = true;
    }

    /// The deck controller integrated this frame's step off the hull (the horizontal step and, on a
    /// jump-off, the air step) and read this frame's keys: `control_player` skips this frame once, so
    /// the exit frame moves the player exactly once, and reports the first on-foot movement on its next
    /// frame without waiting for the send interval.
    pub(crate) fn hand_over_from_deck(&mut self) {
        self.deck_exit = true;
        self.since_sent = SEND_INTERVAL;
    }

    /// A jump edge from the real binding or a scripted key (`hold J`, `--walk ...:J`); the
    /// scripted letter is remembered across frames, so a static hold is one tap, not one per frame.
    pub(crate) fn jump_tapped(&mut self, dev: &DevOptions, t: f32, input_tapped: bool) -> bool {
        let held = dev.scripted(t, 'J');
        let tapped = input_tapped || (held && !self.script_jump_held);
        self.script_jump_held = held;
        tapped
    }

    /// Skips the collision-settle wait in the flow tests (no world cells are built there).
    #[cfg(test)]
    pub(crate) fn settle_for_test(&mut self) {
        self.settled = true;
    }
}

/// `AA_DISMOUNT_TRACE=1`: where the local rider is every 100 ms for 1.5 s after a detach, with the mount it left
/// and its facing, so a replay can measure the landing in the mount's frame.
#[derive(Default)]
pub struct DismountTrace {
    /// Whether the environment asked for the trace.
    on: Option<bool>,
    /// Seconds since the detach; `None` outside a trace.
    at: Option<f32>,
    /// Seconds at which the next sample is due.
    next: f32,
    mount: Vec3,
    yaw: f32,
}

/// The movement keys held this frame (W, S, Q, E, A, D).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MoveKeys {
    pub forward: bool,
    pub back: bool,
    pub left: bool,
    pub right: bool,
    pub turn_left: bool,
    pub turn_right: bool,
}

/// 1.2's on-foot mapping of the keys and the world mouse buttons (x2game+0x412590, called by `input::UpdateMovement`): returns
/// (forward, right, turn left), each -1, 0 or 1. Either mouse button makes A / D strafe instead of turning (with both A and D
/// held, the one pressed last, `last_turn` 1 for A and -1 for D); both buttons also ask for forward. Opposite keys cancel.
pub fn resolve_keys(keys: MoveKeys, left_mouse: bool, right_mouse: bool, last_turn: i8) -> (f32, f32, f32) {
    let mouse = left_mouse || right_mouse;
    let (mut turn_left, mut turn_right) = (keys.turn_left, keys.turn_right);
    if mouse && turn_left && turn_right && last_turn != 0 {
        turn_left = last_turn > 0;
        turn_right = last_turn < 0;
    }
    let axis = |plus: bool, minus: bool| plus as i32 as f32 - minus as i32 as f32;
    let forward = axis(keys.forward || (left_mouse && right_mouse), keys.back);
    let right = axis(keys.right || (mouse && turn_right), keys.left || (mouse && turn_left));
    let turn = if mouse { 0.0 } else { axis(turn_left, turn_right) };
    (forward, right, turn)
}

/// How the character's facing follows the camera this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaceCamera {
    /// Faces where the camera looks at once (the right button, and both).
    Snap,
    /// Keeps its heading.
    Keep,
}

/// Whether the character turns to the camera this frame (1.2's `input::UpdateMovement`). The right
/// button (alone or with the left) turns it to the camera every frame; the left button alone turns the
/// camera only, standing or on the run (1.2: left drag moves the camera only), and its release leaves the heading as
/// it is; without a button the character turns to the camera heading once, at once, in the frame a direction is first asked for.
pub fn face_camera(left_mouse: bool, right_mouse: bool, asked: bool, asked_before: bool) -> FaceCamera {
    if right_mouse {
        return FaceCamera::Snap;
    }
    if left_mouse || !asked || asked_before {
        return FaceCamera::Keep;
    }
    FaceCamera::Snap
}

/// NumLock auto-run as 1.2's `input::UpdateMovement` (x2game+0x414d50) keeps it: its cancelling is armed once forward, back and
/// both mouse buttons are all let go after it starts, and the next press of any of them then ends it. Turning it on while W or S
/// is held does not end it until that key was let go and pressed again.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AutoRun {
    pub on: bool,
    armed: bool,
}

impl AutoRun {
    /// One frame: `toggled` the auto-run key was pressed, `held` forward, back or both mouse buttons are held.
    pub fn update(&mut self, toggled: bool, held: bool) {
        if toggled {
            self.on = !self.on;
            self.armed = false;
        }
        if !self.on {
            return;
        }
        if !held {
            self.armed = true;
        } else if self.armed {
            *self = AutoRun::default();
        }
    }
}

/// The speed factor of a pressed direction (x right, y forward, unit length). 1.2 scales the whole direction by one factor that
/// it interpolates between the actor model's eight directions (x2game+0x122690); the keys give only those eight, and every player
/// row has its diagonal factors equal to the straight ones (forward diagonal 1, backward diagonal 0.65; walking 0.45 and 0.2925),
/// so the straight, strafe and backward factors are 1.2's. Walking takes the walk factors of the run speed.
pub fn direction_factor(actor: &ActorParams, local_dir: Vec2, walking: bool) -> f32 {
    let sideways = local_dir.y.abs() <= 0.1 && local_dir.x != 0.0;
    match (walking, local_dir.y < -0.1, sideways) {
        (false, true, _) => actor.backward,
        (false, _, true) => actor.strafe,
        (false, _, _) => 1.0,
        (true, true, _) => actor.walk_backward,
        (true, _, true) => actor.walk_strafe,
        (true, _, _) => actor.walk,
    }
}

/// The walk mode of an on-foot or deck step: the `.` toggle, or Shift held (the walk modifier).
pub fn walking_mode(toggled: bool, shift: bool) -> bool {
    toggled != shift
}

/// The ground velocity (m/s, x right / y forward) of an on-foot step: the actor model's direction
/// factors, the walk mode and the Dash/buff speed multiplier, slowed by wading.
pub fn ground_velocity(
    actor: &ActorParams,
    local_dir: Vec2,
    walking: bool,
    speed_mul: f32,
    wading: f32,
) -> Vec2 {
    local_dir
        * (RUN_SPEED
            * speed_mul
            * direction_factor(actor, local_dir, walking)
            * swim::wading_factor(wading))
}

/// One frame of a body in the air: gravity, then the frame's mean vertical speed moves it, so a jump peaks at v^2 / 2g (1.2's
/// 1.3 m) at any frame rate. The vertical speed does not fall below `-max_fall` (water holds a falling body back).
pub fn air_step(z: f32, vz: f32, gravity: f32, dt: f32, max_fall: f32) -> (f32, f32) {
    let after = (vz - gravity * dt).max(-max_fall);
    (z + (vz + after) * 0.5 * dt, after)
}

/// Rotates `from` towards `to` (radians) by at most `max`, the short way round.
pub fn approach_angle(from: f32, to: f32, max: f32) -> f32 {
    let d = (to - from + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    from + d.clamp(-max, max)
}

/// The server moved the player (a teleport): follow it.
pub fn follow_server(
    mut events: MessageReader<NetEvent>,
    mut commands: Commands,
    mut player: Query<(Entity, &LocalPlayer, &mut Mover, &mut Transform)>,
    mut cameras: Query<&mut OrbitCamera>,
    mut out: MessageWriter<NetCommand>,
    mut state: ResMut<SendState>,
) {
    for NetEvent(event) in events.read() {
        let (pos, yaw, own) = match event {
            // `SCTeleportUnit` carries the facing at the destination (`z2`, radians)
            Event::Teleported { pos, yaw, .. } => (pos, Some(*yaw), None),
            // `SCLoadInstance` names the destination before the server lets the player move: put the
            // player (and with it the world's streaming focus) there while the loading screen is up.
            Event::LoadInstance(load) => (&load.pos, Some(load.angles[2]), None),
            Event::Resurrected { obj_id, pos, yaw } => (pos, Some(*yaw), Some(*obj_id)),
            _ => continue,
        };
        let Ok((entity, local, mut mover, mut transform)) = player.single_mut() else { continue };
        if own.is_some_and(|id| id != local.obj_id) {
            continue;
        }
        // the destination is no longer a place on the old hull
        commands
            .entity(entity)
            .remove::<super::ship::DeckRide>()
            .remove::<CarriedVelocity>();
        info!("teleported to ({:.1}, {:.1}, {:.1})", pos.x, pos.y, pos.z);
        mover.pos = Vec3::new(pos.x, pos.y, pos.z);
        if let Some(yaw) = yaw {
            mover.facing = yaw;
        }
        mover.vz = 0.0;
        mover.swim = None;
        // the new place is stood on once its collision is built
        state.settled = false;
        state.wait = 0.0;
        *transform = placement(mover.pos, mover.facing, 1.0);
        if let Ok(mut orbit) = cameras.single_mut() {
            orbit.yaw = mover.facing;
        }
        // aa-net already answered with `CSTeleportEnded`; a standing movement packet at the new position makes the server's
        // idea of where we stand (and its view of us for the other players) agree with ours
        out.write(NetCommand(Command::Move(
            MoveCommand {
                pos: Pos::new(mover.pos.x, mover.pos.y, mover.pos.z),
                yaw: mover.facing,
                moving: false,
                stance: stance::RELAXED,
                velocity: [0; 3],
                delta_movement: [0; 3],
                ..MoveCommand::default()
            }
            .with_state(if mover.grounded { MoveState::Ground } else { MoveState::Fall }),
        )));
        mover.velocity = Vec3::ZERO;
        state.since_sent = 0.0;
        state.was_active = false;
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;

    #[test]
    fn native_hole_height_is_not_a_player_floor_and_preserves_the_received_spawn_height() {
        if !aa_cry::pak_available() {
            return;
        }
        let terrain = TerrainQuery::new("main_world");
        let collision = WorldCollision::new("main_world", terrain.clone());
        // Executed original 300C91B0/300CAB30: a real 20,8 leaf, not synthetic missing terrain.
        let hole = Vec3::new(21204.25, 9026.5, 446.40103);
        assert_eq!(terrain.is_hole_at(hole.x, hole.y), Some(true));
        assert!((terrain.height_at(hole.x, hole.y).unwrap() - hole.z).abs() < 0.001);
        assert!(support(&terrain, &collision, hole.x, hole.y, hole.z + 10.0, STEP_UP).is_none());
        assert_eq!(floor_at(&terrain, &collision, hole, STEP_UP), f32::NEG_INFINITY);
        assert_eq!(height_or(&terrain, hole.x, hole.y, 430.0), 430.0);
        let solid = Vec3::new(21188.25, 9030.5, 498.16614);
        assert_eq!(terrain.is_hole_at(solid.x, solid.y), Some(false));
        let ground = support(&terrain, &collision, solid.x, solid.y, solid.z + 10.0, STEP_UP).unwrap();
        assert!(!ground.brush && (ground.height - solid.z).abs() < 0.001);
        assert!((floor_at(&terrain, &collision, solid, STEP_UP) - solid.z).abs() < 0.001);
        // Missing cells retain the application's streaming/map fallback; only known holes permit a fall.
        let outside = Vec3::new(-1.0, -1.0, 123.0);
        assert_eq!(floor_at(&terrain, &collision, outside, STEP_UP), outside.z);
        // The same genuine hole can expose a lower compiled cave/brush floor.
        assert!(collision.build_blocking(20, 8).is_some());
        let cave = Vec3::new(21270.5, 8960.5, 474.8673);
        assert_eq!(terrain.is_hole_at(cave.x, cave.y), Some(true));
        let ground = support(&terrain, &collision, cave.x, cave.y, cave.z + 5.0, STEP_UP).unwrap();
        assert!(ground.brush && (ground.height - 464.76614).abs() < 0.001);
        assert!((floor_at(&terrain, &collision, cave, STEP_UP) - 464.76614).abs() < 0.001);
    }

    #[test]
    fn only_the_local_resurrection_moves_the_player_and_restarts_collision_wait() {
        let mut app = App::new();
        app.init_resource::<SendState>().add_message::<NetEvent>().add_message::<NetCommand>().add_systems(Update, follow_server);
        let entity = app.world_mut().spawn((LocalPlayer { obj_id: 5 }, Mover {
            pos: Vec3::ZERO, facing: 0.0, vz: -3.0, grounded: true, velocity: Vec3::X, swim: None, wading: 0.0,
        }, Transform::default())).id();
        app.world_mut().resource_mut::<SendState>().settled = true;
        let pos = Pos::new(10.0, 20.0, 30.0);
        app.world_mut().write_message(NetEvent(Event::Resurrected { obj_id: 6, pos, yaw: 1.0 }));
        app.update();
        assert_eq!(app.world().get::<Mover>(entity).unwrap().pos, Vec3::ZERO);
        app.world_mut().write_message(NetEvent(Event::Resurrected { obj_id: 5, pos, yaw: 1.0 }));
        app.update();
        let mover = app.world().get::<Mover>(entity).unwrap();
        assert_eq!((mover.pos, mover.facing, mover.vz, mover.velocity), (Vec3::new(10.0, 20.0, 30.0), 1.0, 0.0, Vec3::ZERO));
        assert_eq!(app.world().get::<Transform>(entity).unwrap().translation, cry_to_bevy(mover.pos));
        assert!(!app.world().resource::<SendState>().settled);
    }

    #[test]
    fn a_teleport_clears_the_carried_deck_speed() {
        let mut app = App::new();
        app.init_resource::<SendState>()
            .add_message::<NetEvent>()
            .add_message::<NetCommand>()
            .add_systems(Update, follow_server);
        let entity = app
            .world_mut()
            .spawn((
                LocalPlayer { obj_id: 5 },
                Mover {
                    pos: Vec3::ZERO,
                    facing: 0.0,
                    vz: -3.0,
                    grounded: false,
                    velocity: Vec3::X,
                    swim: None,
                    wading: 0.0,
                },
                Transform::default(),
                CarriedVelocity(Vec2::X),
            ))
            .id();
        app.world_mut().write_message(NetEvent(Event::Teleported {
            pos: Pos::new(10.0, 20.0, 30.0),
            yaw: 0.0,
            reason: 0,
        }));
        app.update();
        assert!(
            app.world().get::<CarriedVelocity>(entity).is_none(),
            "a teleport left the old deck's carried speed"
        );
    }
}

/// `--hide-part N`: hides a part of the player character, to see what is under the armour.
pub fn dev_hide_part(
    dev: Res<DevOptions>,
    characters: Query<&aa_anim::Character, (With<LocalPlayer>, Added<aa_anim::Character>)>,
    mut commands: Commands,
    children: Query<&Children>,
    meshes: Query<&Mesh3d>,
    assets: Res<Assets<Mesh>>,
) {
    for c in &characters {
        for (i, &part) in c.parts.iter().enumerate() {
            let mut n = 0;
            let mut verts = Vec::new();
            for e in children.iter_descendants(part) {
                if let Ok(m) = meshes.get(e) {
                    n += 1;
                    // meshes shared with the character select stage may already be in the render world
                    verts.push(assets.get(&m.0).map_or(0, |m| m.try_attribute(Mesh::ATTRIBUTE_POSITION).map_or(0, |a| a.len())));
                }
            }
            debug!("part {i} {:?} {}: {n} meshes {verts:?}", c.assets.parts[i].kind, c.assets.parts[i].path);
        }
    }
    let Some(n) = dev.hide_part else { return };
    for c in &characters {
        if let Some(&part) = c.parts.get(n) {
            commands.entity(part).insert(Visibility::Hidden);
        }
    }
}

#[cfg(test)]
mod movement_rules_tests {
    use super::*;

    const NONE: MoveKeys = MoveKeys { forward: false, back: false, left: false, right: false, turn_left: false, turn_right: false };

    #[test]
    fn a_and_d_turn_without_a_mouse_button_and_strafe_with_one() {
        let a = MoveKeys { turn_left: true, ..NONE };
        let d = MoveKeys { turn_right: true, ..NONE };
        assert_eq!(resolve_keys(a, false, false, 1), (0.0, 0.0, 1.0));
        assert_eq!(resolve_keys(d, false, false, -1), (0.0, 0.0, -1.0));
        assert_eq!(resolve_keys(MoveKeys { turn_left: true, turn_right: true, ..NONE }, false, false, 1), (0.0, 0.0, 0.0));
        // either button: A strafes left, D right, and nothing turns
        assert_eq!(resolve_keys(a, true, false, 1), (0.0, -1.0, 0.0));
        assert_eq!(resolve_keys(d, false, true, -1), (0.0, 1.0, 0.0));
        // Q with the mouse's A is one strafe, not two
        assert_eq!(resolve_keys(MoveKeys { left: true, ..a }, false, true, 1), (0.0, -1.0, 0.0));
    }

    #[test]
    fn with_a_mouse_button_the_turn_key_pressed_last_wins() {
        let both = MoveKeys { turn_left: true, turn_right: true, ..NONE };
        assert_eq!(resolve_keys(both, false, true, 1), (0.0, -1.0, 0.0), "A pressed last");
        assert_eq!(resolve_keys(both, true, false, -1), (0.0, 1.0, 0.0), "D pressed last");
        assert_eq!(resolve_keys(both, true, false, 0), (0.0, 0.0, 0.0));
    }

    #[test]
    fn both_buttons_run_forward_and_keep_back_and_strafe() {
        assert_eq!(resolve_keys(NONE, true, true, 0), (1.0, 0.0, 0.0));
        assert_eq!(resolve_keys(MoveKeys { left: true, ..NONE }, true, true, 0), (1.0, -1.0, 0.0));
        assert_eq!(resolve_keys(MoveKeys { back: true, ..NONE }, true, true, 0), (0.0, 0.0, 0.0), "S cancels the mouse's forward");
        assert_eq!(resolve_keys(MoveKeys { forward: true, left: true, ..NONE }, false, false, 0), (1.0, -1.0, 0.0));
    }

    #[test]
    fn a_turn_key_turns_a_quarter_turn_a_second_and_no_more_after_a_long_frame() {
        let mut facing = 0.0;
        for _ in 0..60 {
            facing += TURN_RATE * (1.0f32 / 60.0).min(TURN_STEP_CAP);
        }
        assert!((facing - std::f32::consts::FRAC_PI_2).abs() < 1e-4, "{facing}");
        assert!((TURN_RATE * 0.1f32.min(TURN_STEP_CAP) - std::f32::consts::FRAC_PI_2 * 0.033).abs() < 1e-6);
    }

    #[test]
    fn the_left_button_looks_around_without_turning_the_character() {
        // running with the left button held: the camera turns alone, every frame
        assert_eq!(face_camera(true, false, true, false), FaceCamera::Keep);
        assert_eq!(face_camera(true, false, true, true), FaceCamera::Keep);
        // let go mid-run: the heading stays (nothing turns it to where the camera was left)
        assert_eq!(face_camera(false, false, true, true), FaceCamera::Keep);
        // the right button, alone or with the left, faces the camera
        assert_eq!(face_camera(false, true, true, true), FaceCamera::Snap);
        assert_eq!(face_camera(true, true, true, true), FaceCamera::Snap);
        // starting to move without a button turns to the camera heading at once, once (1.2's fold of the yaw offset)
        assert_eq!(face_camera(false, false, true, false), FaceCamera::Snap);
        assert_eq!(face_camera(false, false, true, true), FaceCamera::Keep);
        // standing still: nothing turns
        assert_eq!(face_camera(false, false, false, false), FaceCamera::Keep);
    }

    #[test]
    fn forward_back_and_both_buttons_end_auto_run_once_let_go() {
        let mut run = AutoRun::default();
        run.update(true, false);
        assert!(run.on);
        run.update(false, false);
        run.update(false, true);
        assert!(!run.on, "W pressed after it started ends it");

        // turned on with S held: S has to be let go and pressed again
        let mut run = AutoRun::default();
        run.update(true, true);
        run.update(false, true);
        assert!(run.on, "S held from before does not end it");
        run.update(false, false);
        assert!(run.on);
        run.update(false, true);
        assert!(!run.on);

        // the key toggles it off too, and a new start disarms
        let mut run = AutoRun::default();
        run.update(true, false);
        run.update(true, false);
        assert!(!run.on);
        run.update(true, true);
        run.update(false, true);
        assert!(run.on);
    }

    #[test]
    fn every_key_direction_runs_at_1_2s_speed() {
        let actor = ActorParams::default();
        let speed = |x: f32, y: f32, walking: bool| RUN_SPEED * direction_factor(&actor, Vec2::new(x, y).normalize(), walking);
        let close = |a: f32, b: f32| (a - b).abs() < 0.005;
        assert!(close(speed(0.0, 1.0, false), 5.4));
        assert!(close(speed(1.0, 1.0, false), 5.4), "forward diagonal");
        assert!(close(speed(-1.0, 0.0, false), 5.4), "strafe");
        assert!(close(speed(0.0, -1.0, false), 3.51), "back");
        assert!(close(speed(1.0, -1.0, false), 3.51), "back diagonal: the whole direction at 0.65, not only its back part");
        assert!(close(speed(0.0, 1.0, true), 2.43), "walk");
        assert!(close(speed(1.0, 1.0, true), 2.43));
        assert!(close(speed(1.0, 0.0, true), 2.43));
        assert!(close(speed(0.0, -1.0, true), 1.5795));
        assert!(close(speed(-1.0, -1.0, true), 1.5795));
    }

    #[test]
    fn a_jump_peaks_at_1_2s_height_at_any_frame_rate() {
        let actor = ActorParams::default();
        let v = actor.jump_speed();
        assert!((v - 8.062).abs() < 0.001);
        let jump = |dt: f32, step: &dyn Fn(f32, f32) -> (f32, f32)| {
            let (mut z, mut vz, mut peak, mut t) = (0.0f32, v, 0.0f32, 0.0f32);
            while !(z <= 0.0 && vz <= 0.0) {
                (z, vz) = step(z, vz);
                peak = peak.max(z);
                t += dt;
            }
            (peak, t)
        };
        for fps in [30.0f32, 60.0, 144.0] {
            let dt = 1.0 / fps;
            let (peak, air) = jump(dt, &|z, vz| air_step(z, vz, actor.gravity, dt, f32::INFINITY));
            assert!((peak - 1.2994).abs() < 0.01, "{fps} fps: peak {peak}");
            assert!((air - 2.0 * v / actor.gravity).abs() <= dt + 1e-4, "{fps} fps: {air} s in the air");
        }
        // the step it replaces (speed, then position) fell short of it at 60 fps
        let dt = 1.0 / 60.0;
        let (old, _) = jump(dt, &|z, vz| {
            let vz = vz - actor.gravity * dt;
            (z + vz * dt, vz)
        });
        assert!(old < 1.24, "{old}");
        // water holds a fall back
        assert_eq!(air_step(0.0, -10.0, 7.5, 0.016, 4.0).1, -4.0);
    }

    #[test]
    fn a_scripted_jump_is_one_tap_across_held_frames() {
        let dev = DevOptions {
            script_keys: "J".into(),
            ..Default::default()
        };
        let mut state = SendState::default();
        assert!(state.jump_tapped(&dev, 0.0, false), "the first held frame taps");
        assert!(!state.jump_tapped(&dev, 0.1, false), "a held key is not a second tap");
        let released = DevOptions::default();
        assert!(!state.jump_tapped(&released, 0.2, false));
        assert!(state.jump_tapped(&released, 0.3, true), "the real binding still taps");
    }
}
