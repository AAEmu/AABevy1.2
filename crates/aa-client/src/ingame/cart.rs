//! Driving a cart (a `UnitKind::Slave` land vehicle) at attach point 1.
//!
//! The original client's local vehicle controller (`FUN_390c1b80`, `FUN_390abe10`)
//! runs a CryPhysics wheeled simulation and the non-foot send pump
//! (`FUN_394045f0`) writes `CSMoveUnit` type 2 for the cart once its 100 ms gate
//! is due, with no change test, so a still cart is resent as well. The 1.2
//! serializer (`FUN_397c0470` case 2) writes position, velocity at 1/1092 m/s,
//! the quaternion, the vehicle angular velocity, the steering and the wheel
//! block. The live capture (2026-10-29, Farm Wagon model 1008) confirmed:
//!
//! * 100 ms gate evaluated once per frame (143 ms median at the hidden
//!   client's 22.5 FPS), also while still;
//! * flags 0, no scType/phase block, wheel count 0;
//! * steering is the raw input (+1 left, -1 right), ramped by about 0.245 per
//!   22.5 FPS frame (5.5/s, full lock in ~4 frames): the captured
//!   intermediates 0.484 (two frames in), -0.752 (one frame after release)
//!   and 0.269 (three frames after release) are that ramp sampled by the
//!   143 ms send gate;
//! * `angVel.z = 0.4 * steering` rad/s and the velocity is world-space.
//!
//! The wheeled simulation itself is **not** ported: a kinematic stand-in
//! integrates the yaw at `0.4 * steering` (faded out below [`TURN_FULL_SPEED`]
//! and mirrored in reverse; the wire `angVel` stays the captured value) and the forward speed at
//! [`ACCEL`] toward [`TOP_SPEED`], fitted to the capture's wire speed curve
//! (0 to 4.0 m/s in 1.73 s, 4.0 to 0 in 1.77 s, both a 2.5 m/s² slope). The
//! cart keeps its spawn height (no terrain contact or slope z velocity), and
//! pitch/roll are not modelled; the wheel and spindle visuals are in `vehicle.rs`.

use aa_net::model::UnitKind;
use aa_net::state::Units;
use aa_net::vehicle::VehicleMoveRequest;
use aa_net::{Command, NetCommand, Pos};
use bevy::prelude::*;

use super::mount::LocalMount;
use super::player::{LocalPlayer, Mover, placement};
use super::units::{RemoteUnit, Views};

/// `FUN_394045f0`: the send pump's interval object reads `100 ms`.
pub const SEND_INTERVAL_MS: i32 = 100;

/// Fitted to the capture's forward run: 4.0 m/s reached in 1.6 s of motion
/// (1.73 s wall clock including the input/frame delay).
pub const TOP_SPEED: f32 = 4.0;

/// Fitted to the captured forward run, reverse and both coast-downs: the wire
/// speed changes by 2.5 m/s² on every 144 ms send.
pub const ACCEL: f32 = 2.5;

/// `angVel.z = 0.4 * steering` rad/s (capture: 0.4 exactly at steering 1.0).
pub const YAW_RATE_PER_STEER: f32 = 0.4;

/// The captured steering intermediates are the frame-quantised ramp: 0.484
/// after two 22.5 FPS frames, -0.752 one frame after release and 0.269 three
/// frames after release. That is about 0.245 per 44.4 ms frame = 5.5/s, full
/// lock in ~4 frames (the task's 1-2 frame note was pre-capture).
pub const STEER_RATE: f32 = 5.5;

/// Below this forward speed (m/s) the stand-in's turn fades out, and it turns the other way in
/// reverse: wheels do not turn a cart on the spot. **Not a captured value**: the captures turn
/// only forward, at 0.96 and 4.0 m/s, which both get the full `0.4 * steering`.
pub const TURN_FULL_SPEED: f32 = 0.5;

/// The controller state the wire body is built from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CartController {
    pub pos: Pos,
    /// Quaternion `[x, y, z, w]`.
    pub rotation: [f32; 4],
    /// World velocity in m/s.
    pub velocity: [f32; 3],
    /// Signed forward speed in m/s.
    pub speed: f32,
    /// Raw input in -1..=1, ramped by [`STEER_RATE`].
    pub steering: f32,
    pub yaw: f32,
}

impl CartController {
    pub fn new(pos: Pos, yaw: f32) -> Self {
        let (s, c) = (yaw * 0.5).sin_cos();
        Self {
            pos,
            rotation: [0.0, 0.0, s, c],
            velocity: [0.0; 3],
            speed: 0.0,
            steering: 0.0,
            yaw,
        }
    }

    /// A controller seeded at an observed pose: the byte tests rebuild the
    /// captured bodies from the decoded values, no stepping involved.
    #[cfg(test)]
    pub fn from_pose(
        pos: Pos,
        rotation: [f32; 4],
        velocity: [f32; 3],
        speed: f32,
        steering: f32,
    ) -> Self {
        Self {
            pos,
            rotation,
            velocity,
            speed,
            steering,
            yaw: 2.0 * rotation[2].atan2(rotation[3]),
        }
    }

    /// Takes the server's pose after a teleport or a server-side move, keeping the speed and
    /// steering the driver still holds.
    pub fn resync(&mut self, pos: Pos, yaw: f32) {
        let (s, c) = (yaw * 0.5).sin_cos();
        self.pos = pos;
        self.yaw = yaw;
        self.rotation = [0.0, 0.0, s, c];
    }

    /// `FUN_397c0470` case 2 values; the session fills the time.
    pub fn request(&self, obj_id: u32) -> Option<VehicleMoveRequest> {
        VehicleMoveRequest::from_parts(
            obj_id,
            0,
            self.pos,
            self.rotation,
            self.velocity,
            self.angular_velocity(),
            self.steering,
            &[],
        )
        .ok()
    }

    pub fn angular_velocity(&self) -> [f32; 3] {
        [0.0, 0.0, YAW_RATE_PER_STEER * self.steering]
    }

    /// One frame of the kinematic stand-in: `throttle` and `steer` in -1..=1.
    pub fn step(&mut self, dt: f32, throttle: f32, steer: f32) {
        if dt <= 0.0 {
            return;
        }
        self.steering = approach(self.steering, steer.clamp(-1.0, 1.0), STEER_RATE * dt);
        let target = throttle.clamp(-1.0, 1.0) * TOP_SPEED;
        self.speed = approach(self.speed, target, ACCEL * dt);
        let turn = YAW_RATE_PER_STEER * self.steering * (self.speed / TURN_FULL_SPEED).clamp(-1.0, 1.0) * dt;
        self.yaw = wrap_angle(self.yaw + turn);
        // the captured quaternions yaw about the cart's own up axis (post-multiply)
        let delta = Quat::from_rotation_z(turn);
        self.rotation = (Quat::from_array(self.rotation) * delta).to_array();
        let mut forward = Quat::from_array(self.rotation) * Vec3::Y;
        forward.z = 0.0;
        let forward = forward.normalize_or_zero();
        self.pos.x += forward.x * self.speed * dt;
        self.pos.y += forward.y * self.speed * dt;
        self.velocity = [forward.x * self.speed, forward.y * self.speed, 0.0];
    }
}

fn approach(value: f32, target: f32, step: f32) -> f32 {
    if value < target {
        (value + step).min(target)
    } else {
        (value - step).max(target)
    }
}

pub(crate) fn wrap_angle(a: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    (a + std::f32::consts::PI).rem_euclid(tau) - std::f32::consts::PI
}

/// `FUN_394045f0`'s timer: increased with the frame time, reset on a send. It
/// has no change test, so a still cart keeps sending.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pump {
    pub timer: f32,
}

impl Pump {
    pub fn tick(&mut self, dt: f32) -> bool {
        self.timer += dt;
        self.timer * 1000.0 >= SEND_INTERVAL_MS as f32
    }

    pub fn sent(&mut self) {
        self.timer = 0.0;
    }
}

/// Point 1 of a slave is the driver's seat; a mate and a passenger seat do not
/// drive a cart (`FUN_390c1b80` is entered through the slave attach).
pub fn driver_of(kind: &UnitKind, point: u8) -> bool {
    point == 1 && matches!(kind, UnitKind::Slave { .. })
}

/// The movement keys as the vehicle controller reads them: W/S throttle,
/// A/D steering (A is +1, left).
pub fn controls(forward: bool, back: bool, left: bool, right: bool) -> (f32, f32) {
    let throttle = i8::from(forward) - i8::from(back);
    let steer = i8::from(left) - i8::from(right);
    (f32::from(throttle), f32::from(steer))
}

/// The controller's input: the ordinary bindings plus the script runner's held
/// keys (`--walk` and the script `hold` command), exactly as `player::control_player`
/// reads them on foot.
pub fn keyed_controls(
    actions: &crate::settings::Actions,
    dev: &super::DevOptions,
    t: f32,
    typing: bool,
) -> (f32, f32) {
    let pressed = |action: &str, key: char| !typing && (actions.pressed(action, 0) || dev.scripted(t, key));
    controls(pressed("moveforward", 'W'), pressed("moveback", 'S'),
             pressed("turnleft", 'A'), pressed("turnright", 'D'))
}

/// A server pose this far (m) from the one the controller last wrote is the server moving the
/// cart (a teleport or a correction), not the echo of our own last packet.
pub const RESYNC_DISTANCE: f32 = 2.0;

/// A server yaw this far (rad) from the controller's is a server-side turn.
pub const RESYNC_YAW: f32 = 0.35;

/// The driver is dead when the actor is dead or the player stats say so.
pub fn driver_dead(dead: bool, stats: Option<&aa_net::state::PlayerStats>, local_id: u32) -> bool {
    dead || stats.is_some_and(|s| s.obj_id == local_id && s.dead)
}

/// Whether `server` is far enough from the pose the controller last wrote to resync.
pub fn server_moved(written: (Pos, f32), server: Pos, server_yaw: f32) -> bool {
    let (pos, yaw) = written;
    let distance =
        ((server.x - pos.x).powi(2) + (server.y - pos.y).powi(2) + (server.z - pos.z).powi(2))
            .sqrt();
    distance > RESYNC_DISTANCE || wrap_angle(server_yaw - yaw).abs() > RESYNC_YAW
}

/// The local cart's controller, its send pump and the slave it belongs to.
#[derive(Resource, Default)]
pub struct CartDrive {
    obj_id: u32,
    controller: Option<CartController>,
    pump: Pump,
    /// The pose written to the unit last frame, to tell a server move from our own write.
    written: Option<(Pos, f32)>,
}

impl CartDrive {
    /// The driven cart and the controller's forward speed (m/s), while driving.
    pub fn speed(&self) -> Option<(u32, f32)> {
        Some((self.obj_id, self.controller?.speed))
    }

    /// Stops driving (dead driver, left the seat, no cart): no controller, no sends.
    fn stop(&mut self) {
        *self = Self::default();
    }
}

/// Drives the cart the local player sits on at point 1: the keys move the
/// controller, the cart's unit and transform follow it, and a type-2 body goes
/// out on the 100 ms gate (also when still). The player's own on-foot movement
/// is stopped by `player::control_player`.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn drive_cart(
    time: Res<Time>,
    actions: Res<crate::settings::Actions>,
    dev: Res<super::DevOptions>,
    seat: Res<LocalMount>,
    mut net: ResMut<Units>,
    views: Res<Views>,
    mut carts: Query<(&mut RemoteUnit, &mut Transform), Without<LocalPlayer>>,
    mut player: Query<(&LocalPlayer, &mut Mover, Has<super::combat::Dead>), With<LocalPlayer>>,
    stats: Option<Res<aa_net::state::PlayerStats>>,
    mut out: MessageWriter<NetCommand>,
    mut drive: ResMut<CartDrive>,
    typing: Option<Res<crate::hud::HudKeyboardFocus>>,
) {
    let dt = time.delta_secs().min(0.1);
    let Some(s) = seat.0.filter(|s| s.cart) else {
        drive.stop();
        return;
    };
    let Some(unit) = net.by_id.get(&s.mount) else {
        drive.stop();
        return;
    };
    if !driver_of(&unit.kind, s.point) {
        drive.stop();
        return;
    }
    let Ok((local, mut mover, dead)) = player.single_mut() else {
        drive.stop();
        return;
    };
    if driver_dead(dead, stats.as_deref(), local.obj_id) {
        drive.stop();
        return;
    }
    let server_pos = Pos::new(unit.pos.x, unit.pos.y, unit.pos.z);
    let server_yaw = unit.yaw();
    if drive.obj_id != s.mount || drive.controller.is_none() {
        drive.obj_id = s.mount;
        drive.controller = Some(CartController::new(server_pos, server_yaw));
        drive.pump = Pump::default();
        drive.written = None;
    } else if let Some(written) = drive.written
        && server_moved(written, server_pos, server_yaw)
        && let Some(controller) = drive.controller.as_mut()
    {
        controller.resync(server_pos, server_yaw);
    }
    let typing = typing.is_some_and(|f| f.0);
    let (throttle, steer) = keyed_controls(&actions, &dev, time.elapsed_secs(), typing);
    let Some(mut controller) = drive.controller else {
        return;
    };
    controller.step(dt, throttle, steer);
    drive.controller = Some(controller);
    let (pos, yaw) = (controller.pos, controller.yaw);

    if let Some(unit) = net.by_id.get_mut(&s.mount) {
        unit.pos = pos;
        unit.rotation[2] = yaw;
    }
    if let Some(entity) = views.unit_entity(s.mount)
        && let Ok((mut runit, mut transform)) = carts.get_mut(entity)
    {
        runit.pos = Vec3::new(pos.x, pos.y, pos.z);
        runit.facing = yaw;
        *transform = placement(runit.pos, yaw, runit.scale());
    }
    mover.pos = Vec3::new(pos.x, pos.y, pos.z);
    mover.facing = yaw;
    mover.velocity = Vec3::ZERO;
    mover.vz = 0.0;
    mover.grounded = true;
    mover.swim = None;
    drive.written = Some((pos, yaw));

    if drive.pump.tick(dt) {
        if let Some(request) = controller.request(s.mount) {
            out.write(NetCommand(Command::MoveVehicle(request)));
        }
        drive.pump.sent();
    }
}

#[cfg(test)]
mod tests;
