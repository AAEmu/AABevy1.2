//! The local player: spawned from the server's data, moved by the keyboard, snapped to the
//! terrain and reported to the server.
//!
//! Keys (ArcheAge defaults): W / S forward / back, A / D strafe, Q / E turn, Space jump, Shift
//! walk. The mouse drives the camera ([`super::camera`]); with the right button held the character
//! faces where the camera looks, otherwise it turns to the camera heading when it starts to move.

use aa_anim::pose::Rig;
use aa_anim::{Cal, CharacterAssets, ClipLibrary, spawn_character};
use aa_cry::Skeleton;
use aa_cry::scene::CryAssets;
use aa_net::model::stance;
use aa_net::state::Player;
use aa_net::{Command, Event, MoveCommand, MoveState, NetCommand, NetEvent, Pos};
use aa_world::{TerrainQuery, cry_to_bevy};
use bevy::prelude::*;

use super::anim::{ClipSet, Locomotion, Motion, RUN_SPEED, WALK_SPEED};
use super::appearance;
use super::camera::OrbitCamera;
use super::data::Db;
use super::dev::DevOptions;
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
}

/// What the player's input did in the last step: moving, jumping and turning cancel some casts ([`super::combat`]).
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerInput {
    /// The character walked or ran (a movement key was held and it went somewhere).
    pub moving: bool,
    /// The jump started in this step.
    pub jumped: bool,
    /// The facing changed (Q / E, the right mouse button, the swing to the camera heading).
    pub turned: bool,
}

/// Gravity and jump speed (the actor model has `physics_living_gravity` 25, `game_jump_height` 1.3).
const GRAVITY: f32 = 25.0;
const JUMP_SPEED: f32 = 8.06;
/// Turning with Q / E (radians per second); the actor models turn at 360 degrees per second.
const TURN_RATE: f32 = 2.4;
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

/// Terrain height under a game position, or `fallback` outside the map.
pub fn height_or(terrain: &TerrainQuery, x: f32, y: f32, fallback: f32) -> f32 {
    terrain.height_at(x, y).unwrap_or(fallback)
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
    let pos = Vec3::new(px, py, ground);
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
    let assets = match desc.resolve(&game) {
        Ok(a) => a,
        Err(e) => {
            error!("cannot resolve the player character: {e}");
            return;
        }
    };
    drop(game);
    let carried = weapons::carried(&db, equipment);
    let Some(root) = spawn_with_locomotion(&mut commands, &mut cry, &clips, assets, placement(pos, facing, 1.0), carried) else { return };
    commands.entity(root).insert((
        Name::new(format!("player {}", spawn.name())),
        LocalPlayer { obj_id: spawn.obj_id },
        Mover { pos, facing, vz: 0.0, grounded: true, velocity: Vec3::ZERO },
    ));
}

/// `spawn_character` plus the in-world locomotion clips.
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
    let root = match spawn_character(commands, cry, clips, assets, transform) {
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

/// Keyboard movement, jumping, terrain following and the movement packets.
#[allow(clippy::too_many_arguments)]
pub fn control_player(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut cameras: Query<&mut OrbitCamera>,
    mut player: Query<(&LocalPlayer, &mut Mover, &mut Transform, &mut Motion)>,
    terrain: Res<TerrainQuery>,
    mut out: MessageWriter<NetCommand>,
    mut state: ResMut<SendState>,
    mut input_state: ResMut<PlayerInput>,
    dev: Res<DevOptions>,
    typing: Option<Res<crate::hud::HudKeyboardFocus>>,
    over_ui: Option<Res<crate::hud::HudPointerOverUi>>,
) {
    let Ok((local, mut mover, mut transform, mut motion)) = player.single_mut() else { return };
    let Ok(mut orbit) = cameras.single_mut() else { return };
    let dt = time.delta_secs().min(0.1);
    state.since_spawn += dt;
    // scripted keys (`--walk`) count like pressed ones
    let t = state.since_spawn;
    // an edit box of the HUD (the chat input) has the keyboard: no movement, no jump; a drag that began on the HUD does not turn us
    let typing = typing.is_some_and(|f| f.0);
    let over_ui = over_ui.is_some_and(|o| o.0);
    let pressed = |k: KeyCode, c: char| (!typing && keys.pressed(k)) || dev.scripted(t, c);
    let key = |k: KeyCode, c: char| pressed(k, c) as i32 as f32;

    let (fwd, strafe) = (key(KeyCode::KeyW, 'W') - key(KeyCode::KeyS, 'S'), key(KeyCode::KeyD, 'D') - key(KeyCode::KeyA, 'A'));
    let turn = key(KeyCode::KeyQ, 'Q') - key(KeyCode::KeyE, 'E');
    let before_facing = mover.facing;
    if turn != 0.0 {
        let d = turn * TURN_RATE * dt;
        mover.facing += d;
        orbit.yaw += d; // the camera turns with the character
    }
    let input = Vec2::new(strafe, fwd);
    if mouse.pressed(MouseButton::Right) && !over_ui {
        mover.facing = orbit.yaw;
    } else if input != Vec2::ZERO {
        mover.facing = approach_angle(mover.facing, orbit.yaw, ALIGN_RATE * dt);
    }

    // horizontal velocity from the input (backward and walking are slower, as in the actor model)
    let walking = pressed(KeyCode::ShiftLeft, 'H') || (!typing && keys.pressed(KeyCode::ShiftRight));
    let local_dir = input.normalize_or_zero();
    let mut speed = if walking { WALK_SPEED } else { RUN_SPEED };
    if local_dir.y < -0.1 {
        speed *= 0.65;
    }
    let step = (forward(mover.facing) * local_dir.y + right(mover.facing) * local_dir.x) * speed;
    let mut pos = mover.pos;
    let mut candidate = pos;
    candidate.x += step.x * dt;
    candidate.y += step.y * dt;
    // load the neighbouring heightmap cells in the background when a cell border is crossed
    let cell = ((pos.x / aa_world::CELL_SIZE).floor() as i32, (pos.y / aa_world::CELL_SIZE).floor() as i32);
    if state.cell != cell {
        state.cell = cell;
        terrain.prefetch(pos.x, pos.y, PREFETCH_RADIUS);
    }
    // stay on the map
    if terrain.height_at(candidate.x, candidate.y).is_some() {
        pos = candidate;
    }

    // vertical: terrain following and jumping
    let ground = height_or(&terrain, pos.x, pos.y, mover.pos.z);
    let mut jumped = false;
    if mover.grounded && ((!typing && keys.just_pressed(KeyCode::Space)) || (dev.scripted(t, 'J') && !dev.scripted(t - dt, 'J'))) {
        mover.vz = JUMP_SPEED;
        mover.grounded = false;
        jumped = true;
    }
    if mover.grounded {
        // follow the slope; walking off a ledge turns into a fall
        if pos.z - ground > 0.6 {
            mover.grounded = false;
            mover.vz = 0.0;
        } else {
            pos.z = ground;
        }
    }
    // impact speed (m/s) of a landing in this step: sent once, in the movement packet of the landing
    let mut landed = None;
    if !mover.grounded {
        mover.vz -= GRAVITY * dt;
        pos.z += mover.vz * dt;
        if pos.z <= ground && mover.vz <= 0.0 {
            pos.z = ground;
            landed = Some(-mover.vz);
            mover.vz = 0.0;
            mover.grounded = true;
        }
    }

    let velocity = Vec3::new(step.x, step.y, if mover.grounded { 0.0 } else { mover.vz });
    mover.velocity = velocity;
    mover.pos = pos;
    *transform = placement(pos, mover.facing, 1.0);
    let moving = local_dir != Vec2::ZERO && step.length() > 0.0;
    *motion = Motion { speed: if moving { step.length() } else { 0.0 }, local_dir, airborne: !mover.grounded };
    let turned = (mover.facing - before_facing).abs() > 1e-4;
    let input = PlayerInput { moving, jumped, turned };
    if *input_state != input {
        *input_state = input;
    }

    // movement packets: every SEND_INTERVAL while moving, facing or airborne; one more when it stops,
    // and one at once when the character lands
    state.since_sent += dt;
    let active = moving || !mover.grounded || turned;
    let due = state.since_sent >= SEND_INTERVAL;
    let first = !state.sent_first && state.since_spawn > 0.5;
    if (active && due) || (!active && state.was_active) || first || landed.is_some() {
        let to_wire = |v: f32| (v * 1024.0).round().clamp(-32767.0, 32767.0) as i16;
        // pressed direction relative to the facing, [right, forward, up]; forward is [0, 127, 0]
        let delta = [(local_dir.x * 127.0).round() as i8, (local_dir.y * 127.0).round() as i8, 0];
        // rising after a jump, falling otherwise in the air, on the ground when standing on the terrain
        let move_state = if mover.grounded {
            MoveState::Ground
        } else if mover.vz > 0.0 {
            MoveState::Jump
        } else {
            MoveState::Fall
        };
        let mut cmd = MoveCommand {
            pos: Pos::new(pos.x, pos.y, pos.z),
            yaw: mover.facing,
            moving: active,
            stance: stance::RELAXED,
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
    state.was_active = active;
}

/// Bookkeeping of the movement packets (reset when the game screen is left).
#[derive(Resource, Default)]
pub struct SendState {
    since_sent: f32,
    since_spawn: f32,
    sent_first: bool,
    was_active: bool,
    moves_sent: u32,
    cell: (i32, i32),
}

/// Rotates `from` towards `to` (radians) by at most `max`, the short way round.
pub fn approach_angle(from: f32, to: f32, max: f32) -> f32 {
    let d = (to - from + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    from + d.clamp(-max, max)
}

/// The server moved the player (a teleport): follow it.
pub fn follow_server(
    mut events: MessageReader<NetEvent>,
    mut player: Query<(&mut Mover, &mut Transform), With<LocalPlayer>>,
    mut cameras: Query<&mut OrbitCamera>,
    mut out: MessageWriter<NetCommand>,
    mut state: ResMut<SendState>,
) {
    for NetEvent(event) in events.read() {
        let Event::Teleported { pos } = event else { continue };
        let Ok((mut mover, mut transform)) = player.single_mut() else { continue };
        info!("teleported to ({:.1}, {:.1}, {:.1})", pos.x, pos.y, pos.z);
        mover.pos = Vec3::new(pos.x, pos.y, pos.z);
        mover.vz = 0.0;
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
