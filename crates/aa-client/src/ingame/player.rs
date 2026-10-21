//! The local player: spawned from the server's data, moved by the keyboard, standing on the terrain and on
//! the world's brushes (bridges, stairs), stopped by walls, swimming in deep water, and reported to the server.
//!
//! Keys (ArcheAge's `default_binding.g`): W / S forward / back, Q / E strafe left / right, A / D turn, Space jump (swim
//! up in the water), X swim down, `.` toggles walking, NumLock auto-run (S or a movement-less release of it cancels).
//! Both mouse buttons held run forward. Shift held also walks (a convenience, the real client has none). The mouse drives the
//! camera ([`super::camera`]); with the right button held the character faces where the camera looks, otherwise it turns
//! to the camera heading when it starts to move.
//!
//! Collision ([`aa_world::WorldCollision`]): every step slides the character's capsule along the walls, then stands it on the
//! highest surface under it, terrain or brush, within a step height. Terrain steeper than the actor's climb angle cannot be
//! walked up. The server stays authoritative; the client sends where it thinks it is.
//!
//! Swimming ([`super::swim`]): water deeper than [`ActorParams::swim_depth`] (1.3 m) over the ground makes the character swim
//! at the surface; the packets then carry the swim stance and no ground flag.

use aa_anim::pose::Rig;
use aa_anim::{Cal, CharacterAssets, ClipLibrary, spawn_character_with_facial};
use aa_cry::Skeleton;
use aa_cry::scene::CryAssets;
use aa_net::model::stance;
use aa_net::state::Player;
use aa_net::{Command, Event, MoveCommand, MoveState, NetCommand, NetEvent, Pos};
use aa_world::{TerrainQuery, WaterQuery, WorldCollision, cry_to_bevy};
use bevy::prelude::*;

use super::anim::{ClipSet, Locomotion, Motion, RUN_SPEED};
use super::appearance;
use super::camera::OrbitCamera;
use super::data::Db;
use super::dev::DevOptions;
use super::swim::{self, ActorParams, Swim, SwimClips, Wet};
use super::weapons::{self, CarriedItem};

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
const RADIUS: f32 = 0.35;
const HEIGHT: f32 = 1.8;
const STEP_UP: f32 = 0.5;
const DROP: f32 = 0.6;
/// `physics_living_max_climb_angle` 50 degrees: terrain that rises faster than this over [`CLIMB_LOOK`] metres ahead cannot be
/// walked up (tan 50).
const MAX_CLIMB_TAN: f32 = 1.19;
const CLIMB_LOOK: f32 = 0.5;
/// Turning with A / D (radians per second); the actor models turn at 360 degrees per second.
const TURN_RATE: f32 = 2.4;
/// Seconds after the character settles or is teleported during which a landing is not reported (the server applies fall damage
/// from it, and a spawn above a bridge that is not built yet must not hurt).
const LANDING_GRACE: f32 = 1.5;
/// The longest the first step waits for the collision around the spawn to be built (seconds).
const COLLISION_WAIT: f32 = 5.0;
/// How fast the character swings to the camera heading when it starts to move (radians per second).
const ALIGN_RATE: f32 = 12.0;
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
        Mover { pos, facing, vz: 0.0, grounded: true, velocity: Vec3::ZERO, swim: None, wading: 0.0 },
        actor,
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
    let t = std::time::Instant::now();
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

pub(super) fn floor_at(terrain: &TerrainQuery, collision: &WorldCollision, pos: Vec3, step_up: f32) -> f32 {
    support(terrain, collision, pos.x, pos.y, pos.z, step_up).map_or_else(
        || if terrain.is_hole_at(pos.x, pos.y) == Some(true) { f32::NEG_INFINITY } else { pos.z },
        |support| support.height,
    )
}

/// Keyboard movement, jumping, standing on the ground, swimming and the movement packets.
#[allow(clippy::too_many_arguments)]
pub fn control_player(
    time: Res<Time>,
    (keys, actions): (Res<ButtonInput<KeyCode>>, Res<crate::settings::Actions>),
    mouse: Res<ButtonInput<MouseButton>>,
    mut cameras: Query<&mut OrbitCamera>,
    mut player: Query<(&LocalPlayer, &mut Mover, &mut Transform, &mut Motion, Option<&ActorParams>, Has<super::combat::Dead>, Has<super::skill_motion::SkillMove>)>,
    stats: Option<Res<aa_net::state::PlayerStats>>,
    terrain: Res<TerrainQuery>,
    collision: Res<WorldCollision>,
    water: Res<WaterQuery>,
    mut out: MessageWriter<NetCommand>,
    mut state: ResMut<SendState>,
    mut input_state: ResMut<PlayerInput>,
    dev: Res<DevOptions>,
    typing: Option<Res<crate::hud::HudKeyboardFocus>>,
    over_ui: Option<Res<crate::hud::HudPointerOverUi>>,
    mut trace: Local<(f32, Option<bool>)>,
) {
    let Ok((local, mut mover, mut transform, mut motion, actor, dead, skill_moving)) = player.single_mut() else { return };
    // a skill moves the character (a charge, a back step, a knockback): `skill_motion::drive_local` plays it and sends the packets
    if skill_moving && !dead {
        *input_state = PlayerInput::default();
        state.autorun = false;
        return;
    }
    if dead || stats.is_some_and(|s| s.obj_id == local.obj_id && s.dead) {
        *motion = Motion::default();
        *input_state = PlayerInput::default();
        mover.velocity = Vec3::ZERO;
        mover.vz = 0.0;
        mover.swim = None;
        state.autorun = false;
        state.was_active = false;
        return;
    }
    let Ok(mut orbit) = cameras.single_mut() else { return };
    let actor = actor.copied().unwrap_or_default();
    let dt = time.delta_secs().min(0.1);
    state.since_spawn += dt;
    // scripted keys (`--walk`) count like pressed ones
    let t = state.since_spawn;
    // the first step waits for the collision around the character (a deck or a house is not there before it is built)
    if !state.settled {
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
    let typing = typing.is_some_and(|f| f.0);
    let over_ui = over_ui.is_some_and(|o| o.0);
    let pressed = |action: &str, c: char| (!typing && actions.pressed(action, 0)) || dev.scripted(t, c);
    let tapped = |action: &str| !typing && actions.just_pressed(action, 0);
    let key = |action: &str, c: char| pressed(action, c) as i32 as f32;

    // auto-run (NumLock) and the walking toggle (`.`); S stops the auto-run
    if tapped("autorun") {
        state.autorun = !state.autorun;
    }
    if tapped("toggle_walk") {
        state.walk = !state.walk;
    }
    let back = key("moveback", 'S');
    if back > 0.0 {
        state.autorun = false;
    }
    // both mouse buttons run forward
    let both_buttons = mouse.pressed(MouseButton::Left) && mouse.pressed(MouseButton::Right) && !over_ui;
    let ahead = key("moveforward", 'W') > 0.0 || state.autorun || both_buttons;
    let fwd = ahead as i32 as f32 - back;
    let strafe = key("moveright", 'E') - key("moveleft", 'Q');
    let turn = key("turnleft", 'A') - key("turnright", 'D');
    let jump_held = pressed("jump", 'J');
    let jump_tapped = tapped("jump") || (dev.scripted(t, 'J') && !dev.scripted(t - dt, 'J'));
    let vertical = jump_held as i32 as f32 - key("down", 'X');

    let before_facing = mover.facing;
    if turn != 0.0 {
        let d = turn * TURN_RATE * dt;
        mover.facing += d;
        orbit.yaw += d; // the camera turns with the character
    }
    let local_dir = Vec2::new(strafe, fwd).normalize_or_zero();
    if mouse.pressed(MouseButton::Right) && !over_ui {
        mover.facing = orbit.yaw;
    } else if local_dir != Vec2::ZERO {
        mover.facing = approach_angle(mover.facing, orbit.yaw, ALIGN_RATE * dt);
    }

    let old = mover.pos;
    let swimming = mover.swim.is_some();
    // local velocity (x right, y forward): walking and running as the actor model says, swimming at the swim speed
    let walking = state.walk != (dev.scripted(t, 'H') || (!typing && (keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight))));
    let local_vel = if swimming {
        swim::local_velocity(local_dir)
    } else {
        let sideways = local_dir.y.abs() <= 0.1 && local_dir.x != 0.0;
        let mult = match (walking, local_dir.y < -0.1, sideways) {
            (false, true, _) => actor.backward,
            (false, _, true) => actor.strafe,
            (false, _, _) => 1.0,
            (true, true, _) => actor.walk_backward,
            (true, _, true) => actor.walk_strafe,
            (true, _, _) => actor.walk,
        };
        local_dir * (RUN_SPEED * mult * swim::wading_factor(mover.wading))
    };
    let step = forward(mover.facing) * local_vel.y + right(mover.facing) * local_vel.x;
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
    let want_swim = swim::should_swim(swimming, pos.z, wet, actor.swim_depth()) && (swimming || !rising);
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
            mover.vz -= actor.gravity * if in_water { swim::WATER_GRAVITY } else { 1.0 } * dt;
            if in_water {
                mover.vz = mover.vz.max(-4.0);
            }
            pos.z += mover.vz * dt;
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

    // the step that was taken, not the one asked for: a wall takes the speed away
    let actual = if dt > 0.0 { Vec2::new(pos.x - old.x, pos.y - old.y) / dt } else { Vec2::ZERO };
    let swimming = mover.swim.is_some();
    let airborne = !mover.grounded && !swimming;
    let velocity = Vec3::new(actual.x, actual.y, if swimming || airborne { mover.vz } else { 0.0 });
    mover.velocity = velocity;
    mover.pos = pos;
    *transform = placement(pos, mover.facing, 1.0);
    let moving = local_dir != Vec2::ZERO && actual.length() > 0.05;
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
        out.write(NetCommand(Command::Move(cmd)));
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
    /// NumLock auto-run is on; `.` walking is on.
    autorun: bool,
    walk: bool,
    /// The collision around the character is built (or the wait for it ran out), see [`COLLISION_WAIT`].
    settled: bool,
    wait: f32,
    /// Seconds left of the grace period after settling or a teleport in which a landing is not reported.
    grace: f32,
}

/// Rotates `from` towards `to` (radians) by at most `max`, the short way round.
pub fn approach_angle(from: f32, to: f32, max: f32) -> f32 {
    let d = (to - from + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    from + d.clamp(-max, max)
}

/// The server moved the player (a teleport): follow it.
pub fn follow_server(
    mut events: MessageReader<NetEvent>,
    mut player: Query<(&LocalPlayer, &mut Mover, &mut Transform)>,
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
        let Ok((local, mut mover, mut transform)) = player.single_mut() else { continue };
        if own.is_some_and(|id| id != local.obj_id) {
            continue;
        }
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
