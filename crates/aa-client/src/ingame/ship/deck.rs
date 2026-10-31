//! Standing and walking on a ship deck (S10).
//!
//! No server packet puts a unit on a deck: the on-foot `CSMoveUnit` type 1
//! carries it. While the local player stands on a hull floor, the movement is
//! in the ship's frame (world = ship * local) and every packet has the captured
//! support block: header [`aa_net::model::move_flags::STANDING_ON_OBJECT`] (0x40), actor flag
//! 0x20 plus the ground/jump bits, `gcFlags` 1, `gcPartId` 1, `gcId` = ship,
//! primary `pos` = ship-local (right, forward, up), auxiliary position = world.
//!
//! The two rotation bytes are captured (2026-10-29 rowboat): the primary
//! direction is the player's facing relative to the hull, and the auxiliary
//! `z` is the player's world facing, both as truncated [`radians_to_direction`]
//! bytes. A jump keeps the ride: the feet rise and fall in the ship frame and
//! the jump packet is the captured 0x44 / 0x34 body with the block.
//!
//! Walking or jumping off the hull clears the component, so the next
//! `control_player` packet is an ordinary on-foot movement without 0x40/0x20
//! (the captured transition). The world position and velocity (the ship's
//! included) are carried over. The exit frame's step (and a jump-off's air
//! step) is taken here once: `control_player` skips that frame
//! ([`SendState::hand_over_from_deck`]) and takes over on the next one. A higher floor ahead (a riser, hatch or rail
//! brush) blocks an airborne step while the ride and the vertical motion stay.
//! A dead, skill-moved, seated or mounted player is left to those paths.
//! Walking uses the on-foot modes: the `.` toggle, Shift, auto-run and the
//! Dash/buff speed multiplier.
//!
//! The floor is the salvaged hull-brush deck (`ship/floor.rs`); `board_deck`
//! only asks for it after a cheap ship-local bounds check, and the floor is
//! preloaded by unit preparation so the frame does not load the mesh.

use aa_net::model::{MoveSupport, UnitKind, radians_to_direction, stance};
use aa_net::state::{Player, PlayerStats, Units};
use aa_net::{Command, MoveCommand, MoveState, NetCommand, Pos};
use aa_world::WorldCollision;
use bevy::prelude::*;

use super::super::anim::{LocalJump, Motion};
use super::super::camera::OrbitCamera;
use super::super::combat::Dead;
use super::super::data::Db;
use super::super::mount::{DismountMove, LocalMount, Rider};
use super::super::player::{
    CarriedVelocity, FaceCamera, HEIGHT, LocalPlayer, MoveKeys, Mover, PlayerInput, RADIUS, STEP_UP,
    SendState, TURN_RATE, TURN_STEP_CAP, air_step, face_camera, forward, ground_velocity, placement,
    resolve_keys, right, walking_mode,
};
use super::super::seating::Seated;
use super::super::skill_motion::SkillMove;
use super::super::sprint::Sprint;
use super::super::swim::ActorParams;
use super::super::units::{RemoteUnit, Views};
use super::floor::ShipFloor;

/// How far above the feet a hull floor may be to board it, and how far a
/// walking player steps up onto the next bit of deck (a stair, a hatch).
const DECK_STEP: f32 = 1.5;
const WALK_STEP_UP: f32 = 0.6;
/// A surface this far above the feet in the step's XY is a riser/wall the body hits,
/// not a roof to walk under.
const RISER_BAND: f32 = 2.0;
/// How far down a walking player steps onto the lower deck before the edge
/// counts as a fall; the on-foot controller's DROP (0.6 m), so a bigger
/// lower deck starts a real deck fall instead of snapping the feet down.
const WALK_STEP_DOWN: f32 = 0.6;
/// How far outside a hull's floor bounds the feet may be before the cheap
/// range gate skips the ship (the brush does not reach the rail).
const HULL_MARGIN: f32 = 2.0;
/// The on-foot movement cadence, as `player::SEND_INTERVAL`.
const SEND_INTERVAL: f32 = 0.1;

/// The drawn hull pose of a ship (the smoothed `RemoteUnit` the client shows).
#[derive(bevy::ecs::system::SystemParam)]
pub struct DeckHull<'w, 's> {
    views: Res<'w, Views>,
    hulls: Query<'w, 's, &'static RemoteUnit, Without<LocalPlayer>>,
}

impl DeckHull<'_, '_> {
    /// The drawn hull position and full game-space rotation (S8: the drawn quaternion; yaw,
    /// pitch and roll). #49's `DeckGround::pose` uses the same tuple shape.
    pub(crate) fn pose(&self, ship: u32) -> Option<(Vec3, Quat)> {
        self.views
            .unit_entity(ship)
            .and_then(|entity| self.hulls.get(entity).ok())
            .map(|hull| (hull.pos, hull.game_rotation()))
    }
}

/// The yaw of a game-space (z up) rotation quaternion, as the camera and facing use it.
pub fn yaw_of(rotation: Quat) -> f32 {
    (2.0 * (rotation.w * rotation.z + rotation.x * rotation.y))
        .atan2(1.0 - 2.0 * (rotation.y * rotation.y + rotation.z * rotation.z))
}

/// A ship deck under a glider's feet: the hull and the point in its frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DeckContact {
    pub ship: u32,
    pub local: Vec3,
    /// The hull pose the local point was found at (updated from [`DeckGround::pose`] while the ride lasts).
    pub root: Vec3,
    /// The hull's full drawn rotation at that pose.
    pub rotation: Quat,
    /// The player's facing in the hull frame: it is carried through the landing wait and the slide, and composed
    /// with the current hull yaw (the ship's turn turns the player, it does not steer the slide).
    pub facing: f32,
}

impl DeckContact {
    pub(crate) fn world(self) -> Vec3 {
        ship_to_world(self.root, self.rotation, self.local)
    }

    /// The hull yaw of the contact, for the facing arithmetic.
    pub(crate) fn yaw(self) -> f32 {
        yaw_of(self.rotation)
    }
}

/// The deck floor under a point, for a controller that is not on a deck yet (a glider landing): the same hull-floor
/// lookup `board_deck` uses, at the drawn hull pose when it is known.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct DeckGround<'w, 's> {
    db: Option<Res<'w, Db>>,
    units: Option<Res<'w, Units>>,
    hull: DeckHull<'w, 's>,
}

impl DeckGround<'_, '_> {
    /// The highest ship deck under `at` (world z). A rising glider passes over a deck; a descending one lands on a floor
    /// it is close above, or that the fall crossed since the previous frame (`vz`, `dt`).
    pub(crate) fn floor_at(&self, at: Vec3, vz: f32, dt: f32) -> Option<f32> {
        self.contact(at, vz, dt).map(|contact| contact.world().z)
    }

    /// The highest ship deck under `at`, with the hull and the ship-local landing point, for a glider that must stay
    /// attached to a moving hull through its landing and slide.
    pub(crate) fn contact(&self, at: Vec3, vz: f32, dt: f32) -> Option<DeckContact> {
        self.contact_within(at, vz, dt, 0.5)
    }

    /// [`contact`](Self::contact) with the highest surface `reach` metres above the feet still counting (the stop probe's
    /// `land_height + 0.2`, or the touchdown's 0.5 m).
    pub(crate) fn contact_within(
        &self,
        at: Vec3,
        vz: f32,
        dt: f32,
        reach: f32,
    ) -> Option<DeckContact> {
        let db = self.db.as_deref()?;
        let units = self.units.as_deref()?;
        if vz > 0.0 {
            return None;
        }
        let mut best: Option<(DeckContact, f32)> = None;
        for unit in units.iter() {
            if !matches!(unit.kind, UnitKind::Slave { .. }) || !db.is_ship(unit.model_id) {
                continue;
            }
            let Some(bounds) = db.ship_bounds(unit.model_id) else { continue };
            let wire = (
                Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z),
                Quat::from_rotation_z(unit.yaw()),
            );
            let (root, rotation) = self.hull.pose(unit.obj_id).unwrap_or(wire);
            let local = world_to_ship(root, rotation, at);
            if !in_hull_bounds(bounds, local, HULL_MARGIN) {
                continue;
            }
            let Some(floor) = db.ship_floor(unit.model_id) else { continue };
            let previous = local.z - vz * dt;
            let next = local.z + vz * dt;
            let height = floor
                .height(local.x, local.y, previous.max(local.z) + 0.01)
                .filter(|h| previous >= *h && next <= *h)
                .or_else(|| {
                    floor
                        .height(local.x, local.y, local.z + reach.max(DECK_STEP))
                        .filter(|h| (-0.05..=reach).contains(&(local.z - *h)))
                });
            if let Some(height) = height {
                let contact = DeckContact {
                    ship: unit.obj_id,
                    local: Vec3::new(local.x, local.y, height),
                    root,
                    rotation,
                    facing: 0.0,
                };
                let world = contact.world();
                if best.is_none_or(|(_, best)| world.z > best) {
                    best = Some((contact, world.z));
                }
            }
        }
        best.map(|(contact, _)| contact)
    }

    /// The drawn hull pose of `ship`, else the pose in the last movement packet.
    pub(crate) fn pose(&self, ship: u32) -> Option<(Vec3, Quat)> {
        self.hull.pose(ship).or_else(|| {
            self.units
                .as_deref()
                .and_then(|units| units.by_id.get(&ship))
                .map(|unit| {
                    (
                        Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z),
                        Quat::from_rotation_z(unit.yaw()),
                    )
                })
        })
    }

    /// The hull's floor mesh, if the ship is still in view.
    pub(crate) fn ship_floor(&self, ship: u32) -> Option<std::sync::Arc<ShipFloor>> {
        let db = self.db.as_deref()?;
        let unit = self.units.as_deref()?.by_id.get(&ship)?;
        db.ship_floor(unit.model_id)
    }

    /// The ship's last reported velocity (zero when it is gone).
    pub(crate) fn ship_velocity(&self, ship: u32) -> Vec3 {
        self.units
            .as_deref()
            .and_then(|units| units.by_id.get(&ship))
            .map_or(Vec3::ZERO, ship_velocity)
    }
}

/// The keyboard, mouse and HUD focus the deck reads, as `player::control_player` does.
#[derive(bevy::ecs::system::SystemParam)]
pub struct DeckInput<'w> {
    actions: Res<'w, crate::settings::Actions>,
    keyboard: Res<'w, ButtonInput<KeyCode>>,
    mouse: Res<'w, ButtonInput<MouseButton>>,
    typing: Option<Res<'w, crate::hud::HudKeyboardFocus>>,
    over_ui: Option<Res<'w, crate::hud::HudPointerOverUi>>,
    dev: Res<'w, crate::ingame::DevOptions>,
    /// A glider that is touching or sliding on the hull moves the ride itself; the walker must not fight it.
    gliding: Option<Res<'w, super::super::glide::Gliding>>,
}

/// A player standing on a ship deck (not seated).
#[derive(Component, Clone, Debug)]
pub struct DeckRide {
    ship: u32,
    /// Ship-local (right, forward, up) point on the hull floor, or the feet
    /// while jumping.
    local: Vec3,
    /// The hull floor height under the feet (ship-local z), for landing.
    ground: f32,
    /// Facing relative to the ship's.
    facing: f32,
    /// The hull yaw last frame: the ship's turn is added to the player's world facing.
    last_yaw: f32,
    /// Vertical speed in the ship frame (m/s).
    vz: f32,
    /// The feet are in the air after a deck jump.
    airborne: bool,
    since_sent: f32,
}

impl DeckRide {
    /// The hull-local point the ride keeps (the glider's touchdown and slide update it; the
    /// glide tests read it).
    #[allow(dead_code)]
    pub(crate) fn local(&self) -> Vec3 {
        self.local
    }

    /// A ride at a known hull-local floor point (a landing, not a walk).
    pub(crate) fn new(ship: u32, local: Vec3, facing: f32, last_yaw: f32) -> DeckRide {
        DeckRide {
            ship,
            local,
            ground: local.z,
            facing,
            last_yaw,
            vz: 0.0,
            airborne: false,
            since_sent: SEND_INTERVAL,
        }
    }

    /// A ride starting a deck fall (a glider slide over a lower deck): the feet stay at `local`, the floor below is
    /// `ground`, and `deck_control` applies gravity, the landing and the fall-damage report.
    pub(crate) fn falling(ship: u32, local: Vec3, ground: f32, facing: f32, last_yaw: f32) -> DeckRide {
        let mut ride = DeckRide::new(ship, local, facing, last_yaw);
        ride.ground = ground;
        ride.airborne = true;
        ride.vz = 0.0;
        ride
    }
}

#[cfg(test)]
impl DeckRide {
    pub(crate) fn fixture(
        ship: u32,
        local: Vec3,
        ground: f32,
        facing: f32,
        airborne: bool,
        vz: f32,
        last_yaw: f32,
    ) -> DeckRide {
        DeckRide {
            ship,
            local,
            ground,
            facing,
            last_yaw,
            vz,
            airborne,
            since_sent: SEND_INTERVAL,
        }
    }
}

/// Dead, skill-moved and seated players do not stand on a deck.
pub fn deck_allowed(dead: bool, skill_moving: bool, seated: bool) -> bool {
    !(dead || skill_moving || seated)
}

/// The local player's stats say it died: the server's death event sets them
/// before combat adds [`Dead`], and `control_player` stops on either.
fn stats_dead(stats: Option<&PlayerStats>, obj_id: u32) -> bool {
    stats.is_some_and(|s| s.obj_id == obj_id && s.dead)
}

/// Whether a ship-local point is inside the hull floor's XY bounds plus
/// `margin`: the cheap gate before [`Db::ship_floor`].
pub fn in_hull_bounds(bounds: (Vec2, Vec2), local: Vec3, margin: f32) -> bool {
    let (min, max) = bounds;
    local.x >= min.x - margin
        && local.x <= max.x + margin
        && local.y >= min.y - margin
        && local.y <= max.y + margin
}

/// A ship-local point (right, forward, up) to a world point with the hull's full rotation
/// (the drawn quaternion: yaw, pitch and roll).
pub fn ship_to_world(root: Vec3, rotation: Quat, local: Vec3) -> Vec3 {
    root + rotation * local
}

/// Inverse of [`ship_to_world`].
pub fn world_to_ship(root: Vec3, rotation: Quat, world: Vec3) -> Vec3 {
    rotation.inverse() * (world - root)
}

/// The deck a get-off lands on, with the drawn hull's full rotation (yaw, pitch and roll):
/// the hull floor under `at` (the seat's get-off point), as a [`DeckRide`] and the world feet
/// point. `None` when the point is not over a ship floor, so the ordinary get-off placement
/// runs.
pub(crate) fn deck_landing(
    db: &Db,
    units: &Units,
    ship: u32,
    at: Vec3,
    facing: f32,
    drawn: Option<(Vec3, Quat)>,
) -> Option<(DeckRide, Vec3)> {
    let unit = units.get(ship)?;
    if !matches!(unit.kind, UnitKind::Slave { .. }) || !db.is_ship(unit.model_id) {
        return None;
    }
    let bounds = db.ship_bounds(unit.model_id)?;
    let wire = (
        Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z),
        Quat::from_rotation_z(unit.yaw()),
    );
    let (root, rotation) = drawn.unwrap_or(wire);
    let yaw = yaw_of(rotation);
    let local = world_to_ship(root, rotation, at);
    if !in_hull_bounds(bounds, local, HULL_MARGIN) {
        return None;
    }
    let height = db
        .ship_floor(unit.model_id)?
        .height(local.x, local.y, local.z + DECK_STEP)
        .filter(|h| local.z - h <= DECK_STEP)?;
    let local = Vec3::new(local.x, local.y, height);
    Some((DeckRide::new(ship, local, facing - yaw, yaw), ship_to_world(root, rotation, local)))
}

/// The shortest signed turn from `from` to `to` (radians).
fn angle_delta(to: f32, from: f32) -> f32 {
    (to - from + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
        - std::f32::consts::PI
}

/// One walking step in the ship frame: `facing` is relative to the hull and
/// `velocity` the on-foot ground velocity (`player::ground_velocity`).
pub fn deck_step(facing: f32, velocity: Vec2, dt: f32) -> Vec2 {
    (forward(facing) * velocity.y + right(facing) * velocity.x) * dt
}

/// The horizontal world step for a ship-local step along the deck plane. The rotated step's
/// z is dropped by the collision slide, which would shorten a pitched or rolled walk by cos²;
/// scale the horizontal part so the inverse transform keeps the requested ship-local length.
pub(crate) fn deck_world_step(rotation: Quat, step: Vec2) -> Vec2 {
    let flat = (rotation * Vec3::new(step.x, step.y, 0.0)).truncate();
    let local = rotation.inverse() * Vec3::new(flat.x, flat.y, 0.0);
    let length = local.truncate().length();
    if length <= 1e-6 {
        Vec2::ZERO
    } else {
        flat * (step.length() / length)
    }
}

/// One frame of a deck jump: rises from `z` at `vz` and lands back on the
/// `ground` floor when it comes down.
pub fn deck_air_step(z: f32, vz: f32, ground: f32, gravity: f32, dt: f32) -> (f32, f32, bool) {
    let (z, vz) = air_step(z, vz, gravity, dt, f32::INFINITY);
    if vz <= 0.0 && z <= ground {
        (ground, 0.0, true)
    } else {
        (z, vz, false)
    }
}

/// What an airborne step finds at its target XY: a floor at or below the feet,
/// a higher floor (a riser, hatch or rail brush) that blocks the step, or no
/// hull floor at all (the step carries the feet off the deck).
enum AirStep {
    Floor(f32),
    /// A lower hull floor beyond the walk step: start a deck fall onto it.
    Drop(f32),
    Blocked,
    OffHull,
}

fn airborne_step(floor: Option<&ShipFloor>, x: f32, y: f32, feet: f32) -> AirStep {
    let Some(floor) = floor else {
        return AirStep::OffHull;
    };
    if let Some(h) = floor.height(x, y, feet + DECK_STEP) {
        return if h <= feet {
            AirStep::Floor(h)
        } else {
            AirStep::Blocked
        };
    }
    if floor.height(x, y, f32::INFINITY).is_some() {
        AirStep::Blocked
    } else {
        AirStep::OffHull
    }
}

/// A grounded step onto the next bit of deck: a surface within the walk step is
/// walked onto, a surface in the body band above the feet (a riser, hatch or rail
/// brush) blocks, and no surface at that XY leaves the hull. Only the surfaces
/// within [RISER_BAND] count: a roof, cabin top or upper deck above head height
/// is walked under, not through.
fn walking_step(floor: Option<&ShipFloor>, x: f32, y: f32, feet: f32) -> AirStep {
    let Some(floor) = floor else {
        return AirStep::OffHull;
    };
    let Some(height) = floor.height(x, y, feet + RISER_BAND) else {
        return AirStep::OffHull;
    };
    if height > feet + WALK_STEP_UP {
        AirStep::Blocked
    } else if feet - height <= WALK_STEP_DOWN {
        AirStep::Floor(height)
    } else {
        AirStep::Drop(height)
    }
}

/// What one glider slide step on a hull found.
pub(crate) enum DeckSlide {
    /// The feet moved to this ship-local floor point.
    Moved(Vec3),
    /// The next surface is more than the deck step down: start a deck fall at `local` onto `ground`.
    Drop { local: Vec3, ground: f32 },
    /// A riser, hatch or rail brush: the slide stops here.
    Blocked,
    /// The step carried the feet past the hull: the point and the ship-local velocity of the fall.
    OffHull { local: Vec3, local_velocity: Vec2 },
}

/// One glider slide step in the hull frame, through the same world collision, floor lookup and walk step as
/// `deck_control`: a wall/riser blocks, a lower deck is stepped onto, and leaving the hull reports the fall point.
#[allow(clippy::too_many_arguments)]
pub(crate) fn slide_step(
    floor: Option<&ShipFloor>,
    collision: &WorldCollision,
    root: Vec3,
    rotation: Quat,
    local: Vec3,
    local_facing: f32,
    speed: f32,
    dt: f32,
) -> DeckSlide {
    let local_velocity = deck_step(local_facing, Vec2::new(0.0, speed), 1.0);
    let step = local_velocity * dt;
    let world_step = deck_world_step(rotation, step);
    let world = ship_to_world(root, rotation, local);
    let slid = collision.slide(world, world_step, RADIUS, HEIGHT, STEP_UP).pos;
    let slid_local = world_to_ship(root, rotation, slid);
    let (x, y) = (slid_local.x, slid_local.y);
    match walking_step(floor, x, y, local.z) {
        AirStep::Floor(h) => DeckSlide::Moved(Vec3::new(x, y, h)),
        AirStep::Drop(h) => DeckSlide::Drop {
            local: Vec3::new(x, y, local.z),
            ground: h,
        },
        AirStep::Blocked => DeckSlide::Blocked,
        AirStep::OffHull => DeckSlide::OffHull {
            local: Vec3::new(x, y, local.z),
            local_velocity,
        },
    }
}

/// The world velocity of a ship from its last received type-3 movement.
pub fn ship_velocity(ship: &aa_net::model::Unit) -> Vec3 {
    ship.movement
        .as_ref()
        .filter(|m| m.move_type == 3)
        .map_or(Vec3::ZERO, |m| Vec3::from_array(m.velocity_mps()))
}

/// Leaves the deck the way an on-foot step off a ledge does: the ship-local
/// point and velocity become world ones (the ship's velocity added), and
/// `control_player` continues the fall from the next frame (this frame's step
/// is already taken, so it skips this one).
#[allow(clippy::too_many_arguments)]
pub(crate) fn leave_deck(
    commands: &mut Commands,
    entity: Entity,
    mover: &mut Mover,
    transform: &mut Transform,
    motion: &mut Motion,
    input_state: &mut PlayerInput,
    root: Vec3,
    rotation: Quat,
    yaw: f32,
    facing: f32,
    local: Vec3,
    local_velocity: Vec2,
    vz: f32,
    ship_velocity: Vec3,
    moving: bool,
    walk_off: bool,
    state: &mut SendState,
    local_jump: Option<&mut LocalJump>,
) {
    let world = ship_to_world(root, rotation, local);
    let step = rotation * Vec3::new(local_velocity.x, local_velocity.y, 0.0);
    let velocity = step + ship_velocity;
    mover.pos = world;
    mover.facing = yaw + facing;
    if walk_off {
        // stepping off is an on-foot drop: let control_player's DROP rule decide
        mover.grounded = true;
        mover.vz = 0.0;
    } else {
        // a jump-off keeps its take-off clip and air motion
        mover.grounded = false;
        mover.vz = vz + ship_velocity.z;
        state.take_off();
        if let Some(local_jump) = local_jump {
            local_jump.rising = true;
        }
    }
    mover.swim = None;
    mover.velocity = Vec3::new(velocity.x, velocity.y, mover.vz);
    // only the hull's horizontal motion is carried; the held keys' own speed is
    // added by control_player, as on an on-foot jump
    commands
        .entity(entity)
        .insert(CarriedVelocity(Vec2::new(ship_velocity.x, ship_velocity.y)));
    *transform = placement(world, mover.facing, 1.0);
    *motion = if walk_off {
        Motion::default()
    } else {
        Motion {
            speed: local_velocity.length(),
            local_dir: local_velocity.normalize_or_zero(),
            airborne: true,
        }
    };
    *input_state = PlayerInput {
        moving,
        jumped: !walk_off,
        ..PlayerInput::default()
    };
    commands.entity(entity).remove::<DeckRide>();
    state.hand_over_from_deck();
}

/// The captured deck movement: ship-local primary position (and relative
/// facing), world auxiliary position (and world facing) and the `gcFlags` /
/// `gcPartId` / `gcId` support block.
#[allow(clippy::too_many_arguments)]
pub fn deck_move(
    ship: u32,
    local: Vec3,
    relative_facing: f32,
    world: Vec3,
    world_facing: f32,
    moving: bool,
    state: MoveState,
    delta_movement: [i8; 3],
) -> MoveCommand {
    MoveCommand {
        pos: Pos::new(local.x, local.y, local.z),
        yaw: relative_facing,
        moving,
        stance: stance::RELAXED,
        delta_movement,
        state,
        support: Some(MoveSupport {
            obj_id: ship,
            kind: 1,
            part: 1,
            world: Some((
                Pos::new(world.x, world.y, world.z),
                [0, 0, radians_to_direction(world_facing)],
            )),
            climb: None,
        }),
        ..MoveCommand::default()
    }
}

/// Puts an on-foot player who stands on an owned (or any) ship's hull floor on
/// its deck. Seated, dead and skill-moved players are left to those paths.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn board_deck(
    player: Res<Player>,
    net: Res<Units>,
    db: Option<Res<Db>>,
    mount: Option<Res<LocalMount>>,
    gliding: Option<Res<super::super::glide::Gliding>>,
    stats: Option<Res<PlayerStats>>,
    hull: DeckHull,
    time: Res<Time>,
    mut commands: Commands,
    locals: Query<
        (
            Entity,
            &LocalPlayer,
            &Mover,
            Has<Dead>,
            Option<&SkillMove>,
            Has<Seated>,
            Has<DismountMove>,
            Option<&Rider>,
            Option<&DeckRide>,
        ),
        With<LocalPlayer>,
    >,
) {
    if player
        .spawn
        .as_ref()
        .is_some_and(|p| p.unit.attached_to.is_some())
        || mount.is_some_and(|m| m.0.is_some())
        || gliding.as_deref().is_some_and(|g| g.flying())
    {
        return;
    }
    let Some(db) = db.as_deref() else { return };
    let Ok((entity, local, mover, dead, skill_move, seated, dismounting, riding_rider, deck)) =
        locals.single()
    else {
        return;
    };
    // the server's death event marks the stats before combat adds `Dead` (as on foot)
    let dead = dead || stats_dead(stats.as_deref(), local.obj_id);
    // a get-off clip playing over the hull may already stand on the deck: the
    // ride starts at the landing point and the clip keeps playing
    if deck.is_some()
        || !deck_allowed(dead, skill_move.is_some(), seated)
        || dismounting
        || riding_rider.is_some_and(|rider| !rider.dismounting())
    {
        return;
    }
    let dt = time.delta_secs().min(0.1);
    // the highest floor under the feet among the ships in view
    let mut best: Option<(u32, Vec3, f32, f32, f32, f32)> = None;
    for unit in net.iter() {
        if !matches!(unit.kind, UnitKind::Slave { .. }) || !db.is_ship(unit.model_id) {
            continue;
        }
        // cheap gate: skip ships whose hull is far from the feet before the
        // floor lookup (the cache is warmed by unit preparation)
        let Some(bounds) = db.ship_bounds(unit.model_id) else {
            continue;
        };
        // the drawn hull, as deck_control places the player; the wire pose is the fallback
        let wire = (
            Vec3::new(unit.pos.x, unit.pos.y, unit.pos.z),
            Quat::from_rotation_z(unit.yaw()),
        );
        let (root, rotation) = hull.pose(unit.obj_id).unwrap_or(wire);
        let yaw = yaw_of(rotation);
        let local = world_to_ship(root, rotation, mover.pos);
        if !in_hull_bounds(bounds, local, HULL_MARGIN) {
            continue;
        }
        let Some(floor) = db.ship_floor(unit.model_id) else {
            continue;
        };
        let height = if mover.grounded {
            let Some(height) = floor
                .height(local.x, local.y, local.z + DECK_STEP)
                .filter(|h| local.z - h <= DECK_STEP)
            else {
                continue;
            };
            (height, local.z)
        } else {
            // a jumper still rising passes over the deck
            if mover.vz > 0.0 {
                continue;
            }
            // a fast fall can cross the whole floor window between frames: sweep the
            // segment from the previous to the predicted next feet height
            let previous = local.z - mover.vz * dt;
            let next = local.z + mover.vz * dt;
            let swept = floor
                .height(local.x, local.y, previous.max(local.z) + 0.01)
                .filter(|h| previous >= *h && next <= *h);
            if let Some(height) = swept {
                (height, height)
            } else {
                // otherwise land only once the feet are close above the floor (a swimmer
                // under the hull is not lifted onto the deck)
                let Some(height) = floor
                    .height(local.x, local.y, local.z + DECK_STEP)
                    .filter(|h| (-0.05..=0.5).contains(&(local.z - h)))
                else {
                    continue;
                };
                (height, local.z)
            }
        };
        let (height, feet_z) = height;
        let rise = (feet_z - height).abs();
        if best.is_none_or(|(_, _, _, b, _, _)| rise < b) {
            best = Some((
                unit.obj_id,
                Vec3::new(local.x, local.y, height),
                mover.facing - yaw,
                rise,
                feet_z,
                yaw,
            ));
        }
    }
    let Some((ship, local, facing, _, feet_z, ship_yaw)) = best else {
        return;
    };
    let mut ride = DeckRide::new(ship, local, facing, ship_yaw);
    if !mover.grounded {
        // carry the fall into the ride instead of snapping the feet to the floor
        let ship_vz = net.get(ship).map_or(0.0, |unit| ship_velocity(unit).z);
        ride.local.z = feet_z;
        ride.airborne = true;
        ride.vz = mover.vz - ship_vz;
    }
    commands
        .entity(entity)
        .insert(ride)
        .remove::<DismountMove>()
        .remove::<CarriedVelocity>();
    info!(
        "standing on ship {} deck at local ({:.1}, {:.1}, {:.1})",
        ship, local.x, local.y, local.z
    );
}

/// Walks the on-deck player in the ship frame with the on-foot input and
/// camera rules, follows the authoritative hull and reports the captured
/// support movement (also through a jump).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn deck_control(
    time: Res<Time>,
    input: DeckInput,
    player: Res<Player>,
    net: Res<Units>,
    db: Option<Res<Db>>,
    (mount, stats): (Option<Res<LocalMount>>, Option<Res<PlayerStats>>),
    sprint: Option<Res<Sprint>>,
    mut state: ResMut<SendState>,
    mut commands: Commands,
    mut out: MessageWriter<NetCommand>,
    mut input_state: ResMut<PlayerInput>,
    mut cameras: Query<&mut OrbitCamera>,
    hull: DeckHull,
    collision: Res<WorldCollision>,
    mut local_jump: ResMut<LocalJump>,
    mut query: Query<
        (
            Entity,
            &LocalPlayer,
            &mut Mover,
            &mut Transform,
            &mut Motion,
            &mut DeckRide,
            Option<&ActorParams>,
            Has<Dead>,
            Option<&SkillMove>,
            Has<Seated>,
            Has<DismountMove>,
            Option<&Rider>,
        ),
        With<LocalPlayer>,
    >,
) {
    if input.gliding.as_deref().is_some_and(|glide| glide.flying()) {
        return;
    }
    let Ok((
        entity,
        player_unit,
        mut mover,
        mut transform,
        mut motion,
        mut deck,
        actor,
        dead,
        skill_move,
        seated,
        dismounting,
        riding_rider,
    )) = query.single_mut()
    else {
        return;
    };
    // the server's death event marks the stats before combat adds `Dead` (as on foot)
    let dead = dead || stats_dead(stats.as_deref(), player_unit.obj_id);
    // the get-off clip may play while the ride already follows the hull
    if !deck_allowed(dead, skill_move.is_some(), seated)
        || mount.is_some_and(|m| m.0.is_some())
        || dismounting
        || riding_rider.is_some_and(|rider| !rider.dismounting())
    {
        commands.entity(entity).remove::<DeckRide>();
        *motion = Motion::default();
        *input_state = PlayerInput::default();
        *local_jump = LocalJump::default();
        return;
    }
    if player
        .spawn
        .as_ref()
        .is_some_and(|p| p.unit.attached_to.is_some())
    {
        commands.entity(entity).remove::<DeckRide>();
        *local_jump = LocalJump::default();
        return;
    }
    let Some(ship) = net.get(deck.ship) else {
        commands
            .entity(entity)
            .remove::<DeckRide>()
            .remove::<CarriedVelocity>();
        *local_jump = LocalJump::default();
        return;
    };
    let Ok(mut orbit) = cameras.single_mut() else {
        return;
    };
    let Some(db) = db.as_deref() else { return };
    let floor: Option<std::sync::Arc<ShipFloor>> = db.ship_floor(ship.model_id);
    // the drawn hull (the smoothed RemoteUnit), so the feet do not jitter against
    // the hull the client shows; the support block's ship-local point stays the wire one
    let wire = (
        Vec3::new(ship.pos.x, ship.pos.y, ship.pos.z),
        Quat::from_rotation_z(ship.yaw()),
    );
    let (root, rotation) = hull.pose(deck.ship).unwrap_or(wire);
    let yaw = yaw_of(rotation);
    let dt = time.delta_secs().min(0.1);
    if dt <= 0.0 {
        return;
    }
    // the on-foot input: key/mouse resolution, A / D turning the camera with
    // the character, and the camera snap of `player::control_player`
    let typing = input.typing.as_ref().is_some_and(|f| f.0);
    let over_ui = input.over_ui.as_ref().is_some_and(|o| o.0);
    // the on-foot sources: the real binding and the scripted keys (`--walk`, `hold`)
    let t = state.since_spawn();
    let pressed = |action: &str, key: char| {
        (!typing && input.actions.pressed(action, 0)) || input.dev.scripted(t, key)
    };
    let tapped = |action: &str| !typing && input.actions.just_pressed(action, 0);
    // the walking toggle (`.`) and the turn key pressed last, as on foot
    if tapped("toggle_walk") {
        state.walk = !state.walk;
    }
    if tapped("turnleft") {
        state.last_turn = 1;
    } else if tapped("turnright") {
        state.last_turn = -1;
    }
    let left_mouse = input.mouse.pressed(MouseButton::Left) && !over_ui;
    let right_mouse = input.mouse.pressed(MouseButton::Right) && !over_ui;
    let held = MoveKeys {
        forward: pressed("moveforward", 'W'),
        back: pressed("moveback", 'S'),
        left: pressed("moveleft", 'Q'),
        right: pressed("moveright", 'E'),
        turn_left: pressed("turnleft", 'A'),
        turn_right: pressed("turnright", 'D'),
    };
    // auto-run (NumLock): W, S or both mouse buttons end it once they were let go
    state.autorun.update(
        tapped("autorun"),
        held.forward || held.back || (left_mouse && right_mouse),
    );
    let (fwd, strafe, turn) = resolve_keys(
        MoveKeys {
            forward: held.forward || state.autorun.on,
            ..held
        },
        left_mouse,
        right_mouse,
        state.last_turn,
    );
    let asked = fwd != 0.0 || strafe != 0.0;
    // the world facing is authoritative: another system may have turned us (FaceTarget,
    // a ground-target cast), and the hull's own turn carries the player with it
    mover.facing = yaw + angle_delta(mover.facing, deck.last_yaw);
    deck.last_yaw = yaw;
    deck.facing = angle_delta(mover.facing, yaw);
    let before_facing = deck.facing;
    if turn != 0.0 {
        let d = turn * TURN_RATE * dt.min(TURN_STEP_CAP);
        mover.facing += d;
        orbit.yaw += d;
        deck.facing = angle_delta(mover.facing, yaw);
    }
    if face_camera(left_mouse, right_mouse, asked, state.asked_before) == FaceCamera::Snap {
        mover.facing = orbit.yaw;
        deck.facing = angle_delta(mover.facing, yaw);
    }
    state.asked_before = asked;
    let turned = (deck.facing - before_facing).abs() > 1e-4;
    let actor = actor.copied().unwrap_or_default();
    // a deck jump keeps `DeckRide`: the feet rise and fall in the ship frame
    // and every packet keeps the support block (captured 0x44 / 0x34)
    let jump_tapped = state.jump_tapped(&input.dev, t, tapped("jump"));
    if jump_tapped && !deck.airborne {
        deck.vz = actor.jump_speed();
        deck.airborne = true;
        deck.since_sent = SEND_INTERVAL;
    }
    let shift = !typing
        && (input.dev.scripted(t, 'H')
            || input.keyboard.pressed(KeyCode::ShiftLeft)
            || input.keyboard.pressed(KeyCode::ShiftRight));
    let walking = walking_mode(state.walk, shift);
    let local_dir = Vec2::new(strafe, fwd).normalize_or_zero();
    let local_vel = ground_velocity(
        &actor,
        local_dir,
        walking,
        sprint.as_ref().map_or(1.0, |s| s.speed_mul),
        0.0,
    );
    let start = deck.local.truncate();
    let mut local = deck.local;
    let mut walked = false;
    let mut landed: Option<f32> = None;
    let ship_local_vel = deck_step(deck.facing, local_vel, 1.0);
    let step = ship_local_vel * dt;
    if step != Vec2::ZERO {
        // world collision on the same capsule as on foot; the own hull is not world
        // collision, and the deck floor stays authoritative for the vertical step.
        // The ship's masts/cabin brushes are not static collision either: only the
        // deck floor and its risers model the ship itself.
        let world_step = deck_world_step(rotation, step);
        let world = ship_to_world(root, rotation, local);
        let slid = collision.slide(world, world_step, RADIUS, HEIGHT, STEP_UP).pos;
        let slid_local = world_to_ship(root, rotation, slid);
        let (x, y) = (slid_local.x, slid_local.y);
        if deck.airborne {
            match airborne_step(floor.as_deref(), x, y, local.z) {
                AirStep::Floor(h) | AirStep::Drop(h) => {
                    local.x = x;
                    local.y = y;
                    deck.ground = h;
                }
                AirStep::Blocked => {
                    // a riser, hatch or rail brush ahead: keep the ride and the
                    // vertical motion, but do not step into it
                }
                AirStep::OffHull => {
                    // the jump carried the feet past the hull: leave with the
                    // world position and velocity (the ship's included) and
                    // let the on-foot controller fall on
                    let (z, vz) = air_step(local.z, deck.vz, actor.gravity, dt, f32::INFINITY);
                    leave_deck(
                        &mut commands,
                        entity,
                        &mut mover,
                        &mut transform,
                        &mut motion,
                        &mut input_state,
                        root,
                        rotation,
                        yaw,
                        deck.facing,
                        Vec3::new(x, y, z),
                        ship_local_vel,
                        vz,
                        ship_velocity(ship),
                        true,
                        false,
                        &mut state,
                        Some(&mut local_jump),
                    );
                    return;
                }
            }
        } else {
            match walking_step(floor.as_deref(), x, y, local.z) {
                AirStep::Floor(h) => {
                    local = Vec3::new(x, y, h);
                    deck.ground = h;
                    walked = true;
                }
                AirStep::Drop(h) => {
                    // a lower deck beyond the walk step: keep the new XY and fall
                    // onto it in the ship frame (the landing reports its impact)
                    local.x = x;
                    local.y = y;
                    deck.ground = h;
                    deck.airborne = true;
                    deck.vz = 0.0;
                }
                AirStep::Blocked => {
                    // a riser, hatch or rail brush ahead: keep the ride and do not step in
                }
                AirStep::OffHull => {
                    // walking off the deck: fall where the step would have gone and
                    // let the on-foot controller send the unparented packets
                    leave_deck(
                        &mut commands,
                        entity,
                        &mut mover,
                        &mut transform,
                        &mut motion,
                        &mut input_state,
                        root,
                        rotation,
                        yaw,
                        deck.facing,
                        Vec3::new(x, y, local.z),
                        ship_local_vel,
                        0.0,
                        ship_velocity(ship),
                        true,
                        true,
                        &mut state,
                        Some(&mut local_jump),
                    );
                    *local_jump = LocalJump::default();
                    return;
                }
            }
        }
    }
    if deck.airborne {
        let impact = deck.vz;
        let (z, vz, touched) = deck_air_step(local.z, deck.vz, deck.ground, actor.gravity, dt);
        local.z = z;
        deck.vz = vz;
        if touched {
            deck.airborne = false;
            // the landing packet carries the impact, as control_player reports a fall
            landed = Some(-impact);
        }
    }
    deck.local = local;
    let world = ship_to_world(root, rotation, local);
    let facing = yaw + deck.facing;
    mover.pos = world;
    mover.facing = facing;
    mover.velocity = Vec3::ZERO;
    mover.vz = if deck.airborne { deck.vz } else { 0.0 };
    mover.grounded = !deck.airborne;
    mover.swim = None;
    mover.wading = 0.0;
    *transform = placement(world, facing, 1.0);
    *motion = Motion {
        speed: if walked || deck.airborne {
            local_vel.length()
        } else {
            0.0
        },
        local_dir,
        airborne: deck.airborne,
    };
    // the same jump/walk/Dash facts the on-foot controller writes for the clips
    let jump_now = LocalJump {
        freefall: false,
        rising: deck.airborne && deck.vz > 0.0,
        walking,
        sprint: sprint.as_ref().is_some_and(|s| s.dashing),
        dir: local_dir,
        speed_mul: sprint.as_ref().map_or(1.0, |s| s.speed_mul),
        sprint_motion: sprint.as_ref().is_some_and(|s| s.sprint_motion),
    };
    if *local_jump != jump_now {
        *local_jump = jump_now;
    }
    // the step that was taken, not the one asked for (as `control_player`): keys that
    // carry a jumper across the deck move it, a riser or wall that stops the feet does not
    let moved = (local.truncate() - start).length() / dt;
    let moving_input = local_dir != Vec2::ZERO && moved > 0.05;
    // the packet's moving flag stays the captured one: grounded and asked to move
    let sent_moving = !deck.airborne && (walked || local_dir != Vec2::ZERO);
    *input_state = PlayerInput {
        moving: moving_input,
        jumped: jump_tapped,
        turned,
    };
    if landed.is_some() {
        deck.since_sent = SEND_INTERVAL;
    }
    deck.since_sent += dt;
    if deck.since_sent >= SEND_INTERVAL {
        let state = if deck.airborne {
            if deck.vz > 0.0 {
                MoveState::Jump
            } else {
                MoveState::Fall
            }
        } else {
            MoveState::Ground
        };
        let delta = [
            (strafe * 127.0).round() as i8,
            (fwd * 127.0).round() as i8,
            0,
        ];
        let mut command = deck_move(
            deck.ship,
            local,
            deck.facing,
            world,
            facing,
            sent_moving,
            state,
            delta,
        );
        if let Some(speed) = landed {
            command = command.landing(speed);
        }
        out.write(NetCommand(Command::Move(command)));
        deck.since_sent = 0.0;
    }
}
