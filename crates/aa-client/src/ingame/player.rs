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
use aa_net::{Command, Event, MoveCommand, NetCommand, NetEvent, Pos};
use aa_world::cry_to_bevy;
use bevy::prelude::*;

use super::anim::{ClipSet, Locomotion, Motion, RUN_SPEED, WALK_SPEED};
use super::appearance;
use super::camera::OrbitCamera;
use super::data::Db;
use super::dev::DevOptions;
use super::terrain::Terrain;

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

pub fn placement(pos: Vec3, yaw: f32, scale: f32) -> Transform {
    Transform { translation: cry_to_bevy(pos), rotation: Quat::from_rotation_y(yaw), scale: Vec3::splat(scale) }
}

/// Builds the player character from the server's spawn data (`OnEnter(Screen::InGame)`).
pub fn spawn_player(
    mut commands: Commands,
    player: Res<Player>,
    db: Option<Res<Db>>,
    mut cry: CryAssets,
    clips: Res<ClipLibrary>,
    mut terrain: ResMut<Terrain>,
    dev: Res<DevOptions>,
) {
    let (Some(spawn), Some(db)) = (&player.spawn, db) else {
        error!("in game without a player spawn or the game database");
        return;
    };
    let character = spawn.character();
    let p = spawn.position();
    let (px, py) = dev.pos.map_or((p.x, p.y), |d| (d.x, d.y));
    let ground = terrain.height_or(px, py, p.z);
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
    let params = if spawn.unit.model_params.kind > 0 { &spawn.unit.model_params } else { &character.model_params };
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
    let Some(root) = spawn_with_locomotion(&mut commands, &mut cry, &clips, assets, placement(pos, facing, 1.0)) else { return };
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
        .zip(Cal::load(&cal).ok())
        .and_then(|(rig, cal)| ClipSet::load(&rig, &cal, clips));
    debug!("spawn_character {t_character:.0} ms, locomotion clips {:.0} ms", t.elapsed().as_secs_f32() * 1000.0 - t_character);
    let mut entity = commands.entity(root);
    entity.insert(Motion::default());
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
    mut terrain: ResMut<Terrain>,
    mut out: MessageWriter<NetCommand>,
    mut state: Local<SendState>,
    dev: Res<DevOptions>,
) {
    let Ok((local, mut mover, mut transform, mut motion)) = player.single_mut() else { return };
    let Ok(mut orbit) = cameras.single_mut() else { return };
    let dt = time.delta_secs().min(0.1);
    state.since_spawn += dt;
    // scripted keys (`--walk`) count like pressed ones
    let t = state.since_spawn;
    let pressed = |k: KeyCode, c: char| keys.pressed(k) || dev.scripted(t, c);
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
    if mouse.pressed(MouseButton::Right) {
        mover.facing = orbit.yaw;
    } else if input != Vec2::ZERO {
        mover.facing = approach_angle(mover.facing, orbit.yaw, ALIGN_RATE * dt);
    }

    // horizontal velocity from the input (backward and walking are slower, as in the actor model)
    let walking = pressed(KeyCode::ShiftLeft, 'H') || keys.pressed(KeyCode::ShiftRight);
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
    // stay on the map
    if terrain.height(candidate.x, candidate.y).is_some() {
        pos = candidate;
    }

    // vertical: terrain following and jumping
    let ground = terrain.height_or(pos.x, pos.y, mover.pos.z);
    if mover.grounded && (keys.just_pressed(KeyCode::Space) || (dev.scripted(t, 'J') && !dev.scripted(t - dt, 'J'))) {
        mover.vz = JUMP_SPEED;
        mover.grounded = false;
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
    if !mover.grounded {
        mover.vz -= GRAVITY * dt;
        pos.z += mover.vz * dt;
        if pos.z <= ground && mover.vz <= 0.0 {
            pos.z = ground;
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

    // movement packets: every SEND_INTERVAL while moving, facing or airborne; one more when it stops
    state.since_sent += dt;
    let active = moving || !mover.grounded || (mover.facing - before_facing).abs() > 1e-4;
    let due = state.since_sent >= SEND_INTERVAL;
    let first = !state.sent_first && state.since_spawn > 0.5;
    if (active && due) || (!active && state.was_active) || first {
        let to_wire = |v: f32| (v * 1024.0).round().clamp(-32767.0, 32767.0) as i16;
        out.write(NetCommand(Command::Move(MoveCommand {
            pos: Pos::new(pos.x, pos.y, pos.z),
            yaw: mover.facing,
            moving: active,
            stance: stance::RELAXED,
            velocity: if active { [to_wire(velocity.x), to_wire(velocity.y), to_wire(velocity.z)] } else { [0; 3] },
        })));
        state.since_sent = 0.0;
        state.sent_first = true;
        state.moves_sent += 1;
        if state.moves_sent % 50 == 1 {
            debug!("move #{} to ({:.1}, {:.1}, {:.1}) for obj {}", state.moves_sent, pos.x, pos.y, pos.z, local.obj_id);
        }
    }
    state.was_active = active;
}

/// Bookkeeping of the movement packets.
#[derive(Default)]
pub struct SendState {
    since_sent: f32,
    since_spawn: f32,
    sent_first: bool,
    was_active: bool,
    moves_sent: u32,
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
                    verts.push(assets.get(&m.0).map_or(0, |m| m.count_vertices()));
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
