//! Driving a ship at the helm (attach point 1 of a `ShipModel` slave).
//!
//! The server owns the hull: the seated driver only sends the two helm bytes.
//! The local controller is the ship branch of `FUN_3909d8c0`:
//!
//! * throttle is the key level clamped to -1..=1 and written at once
//!   (`FUN_3909da60`: `round(ctrl+0x148 * 127)`);
//! * steering ramps toward the pressed direction at the ship model's
//!   `ship_models.steer_vel` and decays to 0 when no key is held; the wire sign
//!   is A = -127, D = +127 (the capture's `c_left` / `c_right` rows).
//!
//! The 2026-10-29 rowboat capture sent CSMoveUnit type 4 continuously from the
//! 100 ms non-actor pump (`FUN_394045f0`), also 0/0, with header flags 0. The
//! captured ramp samples are frame-quantised, so the rate is fitted to them:
//! D 65, A -98, release 127 -> 62 -> 0 and 127 -> 94 -> 0. The rowboat's
//! `ship_models.steer_vel` is 6.0 axis/s (762 wire units/s) and reproduces
//! every sample with the input-to-frame delays 85/129/85/43 ms. The decompile
//! multiplies the row by a 0.01-scaled game option whose value was not
//! captured; the fit therefore folds that factor into the rowboat's row. Other
//! models use their own row (a galleon's is 1.0), and [`STEER_RATE`] is the
//! fallback when a model has no ship row.
//!
//! The capture's last type-4 at the leave is 0/0, sent on the same wall tick
//! just before `CSDiscardSlave`; nothing follows it. The HUD slave leave writes
//! that final 0/0 immediately before its unbind request, and [`drive_ship`]
//! sends nothing more until `SCUnitDetached` clears the seat (nor after it). A
//! teleport's loading screen skips [`drive_ship`], so [`reset_on_teleport`]
//! clears the ramp and pump meanwhile.
//!
//! Visuals: the smallboat (rowboat) prefab has only the `$helms`, `$driver`,
//! `$passenger0`, `$ladder_*`, `$aimpoint`, `$heal_point0` and `$lamp_rear`
//! helper nodes and the static `smallboat_helms.cgf` brush; it has no rudder
//! part or steering-wheel bone. `FUN_390d2990` (UpdateSteeringWheel) returns
//! early for models whose path contains `smallboat`, `soloship` or
//! `coupleduckship`, so the rowboat has no wheel animation in the original
//! either. `FUN_390c7410` (UpdateRudder) needs a child model named `rudder`,
//! which this prefab does not have.

pub(crate) mod deck;
pub(crate) mod floor;

pub use deck::{DeckRide, board_deck, deck_control};

use aa_net::model::UnitKind;
use aa_net::state::{PlayerStats, Units};
use aa_net::{Command, NetCommand, ShipControlRequest};
use bevy::prelude::*;

use super::cart::Pump;
use super::data::Db;
use super::mount::LocalMount;
use super::player::LocalPlayer;

/// The native full input level: `FUN_3909da60` writes `round(axis * 127)`.
pub const FULL: i8 = 127;

/// The fallback steering ramp (axis/s, 762 wire units/s) when a model has no
/// `ship_models` row: the rowboat's 6.0. The captured ramp was fitted at this
/// rate, with the game option the decompile multiplies in folded into the fit.
pub const STEER_RATE: f32 = super::data::DEFAULT_SHIP_STEER_VEL;

/// The helm state the type-4 body is built from. Throttle is already a level,
/// steering is a continuous -1..=1 axis.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShipController {
    pub throttle: f32,
    pub steering: f32,
}

impl ShipController {
    /// `FUN_3909d8c0`: throttle follows the key at once; steering ramps at
    /// `rate` (the model's `ship_models.steer_vel`) while a direction is held
    /// (through zero when it flips) and decays at the same rate when released.
    pub fn step(&mut self, dt: f32, throttle: f32, steer: f32, rate: f32) {
        if dt <= 0.0 {
            return;
        }
        self.throttle = throttle.clamp(-1.0, 1.0);
        let target = steer.clamp(-1.0, 1.0);
        let step = rate * dt;
        self.steering = if target == 0.0 {
            approach(self.steering, 0.0, step)
        } else {
            (self.steering + target.signum() * step).clamp(-1.0, 1.0)
        };
    }

    /// `FUN_3909da60`: round the axes into the signed wire levels.
    pub fn request(&self, obj_id: u32, time: u32) -> Option<ShipControlRequest> {
        let level = |v: f32| (v.clamp(-1.0, 1.0) * f32::from(FULL)).round() as i8;
        ShipControlRequest::new(obj_id, time, 0, None, level(self.throttle), level(self.steering)).ok()
    }
}

fn approach(value: f32, target: f32, step: f32) -> f32 {
    if value < target {
        (value + step).min(target)
    } else {
        (value - step).max(target)
    }
}

/// Point 1 of a slave is the driver's seat; the server ignores ship requests
/// from any other seat (`movement.rs` checks the sender's attach point).
pub fn driver_of(kind: &UnitKind, point: u8) -> bool {
    point == 1 && matches!(kind, UnitKind::Slave { .. })
}

/// W/S throttle (+127 / -127), A/D steering (-127 / +127).
pub fn controls(forward: bool, back: bool, left: bool, right: bool) -> (f32, f32) {
    let throttle = f32::from(i8::from(forward) - i8::from(back));
    let steer = f32::from(i8::from(right) - i8::from(left));
    (throttle, steer)
}

/// The ordinary bindings plus the script runner's held keys, read exactly as
/// `cart::keyed_controls` does on foot.
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

/// The helm controller, its send pump and the ship it belongs to.
#[derive(Resource, Default)]
pub struct ShipDrive {
    obj_id: u32,
    /// The ship model's `ship_models.steer_vel` (axis/s).
    rate: f32,
    controller: ShipController,
    pump: Pump,
    /// The ship whose leave request is in flight: no helm frame is sent again
    /// until `SCUnitDetached` clears the seat.
    leaving: u32,
}

impl ShipDrive {
    fn stop(&mut self) {
        *self = Self::default();
    }

    /// Ends a helm session with the capture's last request: one 0/0 for the
    /// ship. Nothing when no ship is driven or the leave is already in flight.
    pub(crate) fn leave(&mut self, obj_id: u32) -> Option<ShipControlRequest> {
        let request = (obj_id != 0 && self.leaving != obj_id)
            .then(|| ShipController::default().request(obj_id, 0))
            .flatten();
        if request.is_some() {
            *self = ShipDrive { leaving: obj_id, ..Self::default() };
        }
        request
    }
}

/// The final helm 0/0 that must precede `CSDiscardSlave` when the local driver
/// unbinds the ship whose timeline id is `tl_id`; nothing for another slave or
/// when the leave is already in flight.
pub(crate) fn leave_before_unbind(
    drive: &mut ShipDrive,
    seat: &LocalMount,
    units: &Units,
    tl_id: u16,
) -> Option<ShipControlRequest> {
    let obj_id = seat.ship_driver()?;
    let UnitKind::Slave { tl_id: ship_tl, .. } = &units.by_id.get(&obj_id)?.kind else {
        return None;
    };
    (*ship_tl == tl_id).then(|| drive.leave(obj_id)).flatten()
}

/// Clears the helm ramp and pump while a teleport's loading screen is up:
/// [`drive_ship`] is skipped then, so it would otherwise resume the ramp from
/// before the jump. A leave in flight stays latched, so no helm frame follows
/// its final 0/0.
pub fn reset_on_teleport(tp: Res<super::teleport::Teleport>, mut drive: ResMut<ShipDrive>) {
    if tp.loading() {
        *drive = ShipDrive { leaving: drive.leaving, ..ShipDrive::default() };
    }
}

/// Sends CSMoveUnit type 4 for the ship the local player drives, on the same
/// 100 ms gate as the cart and also when both bytes are 0. The hull is placed
/// by the server's type-3 echo; this system never moves it.
#[allow(clippy::too_many_arguments)]
pub fn drive_ship(
    time: Res<Time>,
    actions: Res<crate::settings::Actions>,
    dev: Res<super::DevOptions>,
    seat: Res<LocalMount>,
    db: Option<Res<Db>>,
    net: Res<Units>,
    player: Query<(&LocalPlayer, Has<super::combat::Dead>)>,
    stats: Option<Res<PlayerStats>>,
    mut out: MessageWriter<NetCommand>,
    mut drive: ResMut<ShipDrive>,
    typing: Option<Res<crate::hud::HudKeyboardFocus>>,
) {
    let dt = time.delta_secs().min(0.1);
    let Some(obj_id) = seat.ship_driver() else {
        drive.stop();
        return;
    };
    if drive.leaving == obj_id {
        return;
    }
    let Some(unit) = net.by_id.get(&obj_id) else {
        drive.stop();
        return;
    };
    if !driver_of(&unit.kind, seat.0.map_or(0, |s| s.point)) {
        drive.stop();
        return;
    }
    let Ok((local, dead)) = player.single() else {
        drive.stop();
        return;
    };
    if dead || stats.as_deref().is_some_and(|s| s.obj_id == local.obj_id && s.dead) {
        drive.stop();
        return;
    }
    if drive.obj_id != obj_id {
        let rate = db.as_deref().map_or(STEER_RATE, |db| db.ship_steer_vel(unit.model_id));
        *drive = ShipDrive { obj_id, rate, ..ShipDrive::default() };
    }
    let typing = typing.is_some_and(|f| f.0);
    let (throttle, steer) = keyed_controls(&actions, &dev, time.elapsed_secs(), typing);
    let rate = drive.rate;
    drive.controller.step(dt, throttle, steer, rate);
    if drive.pump.tick(dt) {
        if let Some(request) = drive.controller.request(obj_id, 0) {
            out.write(NetCommand(Command::ShipControl(request)));
        }
        drive.pump.sent();
    }
}

#[cfg(test)]
mod tests;
