//! Checked CS089 type-2 (land vehicle) wire construction; no driver or
//! controller admission.
//!
//! `FUN_397c0470` case 2 (case 2 ~rel. 169-226) is the serializer; the object
//! id in front of it is the controlled vehicle (`FUN_394045f0` / `FUN_394064b0`
//! send path, opcode 0x089). The 100 ms pump interval, the velocity scale
//! (client 30/32767 = m/s*1092 vs the server's 2048) and the wheel angular
//! velocity units are not confirmed on the wire yet; see `docs/aa-net.md`.

use crate::vehicle::codec::FixedPos;
use crate::vehicle::move_unit::{MAX_WHEELS, MoveUnit};
use crate::wire::{Pos, Writer, coord_to_long};

/// A checked `CSMoveUnit` type-2 body: a nonzero BC3 vehicle object followed by
/// the native vehicle movement.
///
/// The object is the controlled slave (cart, car, tractor), not its driver.
/// A future Session adapter must recheck the current own driver's received
/// attachment and the slave incarnation before writing, as for
/// [`crate::ShipControlRequest`]. Ownership alone is insufficient.
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleMoveRequest {
    slave_obj_id: u32,
    body: MoveUnit,
}

impl VehicleMoveRequest {
    /// Requires a nonzero 24-bit object id, `body.move_type() == 2` and at most
    /// [`MAX_WHEELS`] wheel angular velocities. The serializer writes the count
    /// byte before the wheels, so a larger count would desynchronise the body.
    pub fn new(slave_obj_id: u32, body: MoveUnit) -> Result<Self, &'static str> {
        if slave_obj_id == 0 || slave_obj_id > 0x00ff_ffff {
            return Err("vehicle move requires a nonzero 24-bit vehicle object ID");
        }
        if body.move_type() != 2 {
            return Err("vehicle move requires a type-2 move body");
        }
        if usize::from(body.wheel_count()) > MAX_WHEELS {
            return Err("vehicle move carries more than 18 wheel angular velocities");
        }
        if !body.fixed_pos().z.is_finite() {
            return Err("vehicle move requires a finite z position");
        }
        if body
            .velocity()
            .into_iter()
            .chain(body.rotation())
            .chain(body.vehicle_ang_vel())
            .chain(core::iter::once(body.vehicle_steering()))
            .chain((0..usize::from(body.wheel_count())).map(|i| body.wheel_ang_vel(i)))
            .any(|v| !v.is_finite())
        {
            return Err("vehicle move requires finite vehicle values");
        }
        Ok(Self { slave_obj_id, body })
    }

    /// Builds a type-2 body from world values: position in metres, a quaternion
    /// `[x, y, z, w]`, velocity in m/s, angular velocity in rad/s, steering in
    /// [-1, 1] and wheel angular velocities. The world-position rounding goes
    /// through [`coord_to_long`]; the original `FUN_3961ddd0` conversion still
    /// needs a live check (see `docs/aa-net.md`).
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        slave_obj_id: u32,
        time: u32,
        pos: Pos,
        rotation: [f32; 4],
        velocity_mps: [f32; 3],
        angular_velocity: [f32; 3],
        steering: f32,
        wheel_ang_vel: &[f32],
    ) -> Result<Self, &'static str> {
        if wheel_ang_vel.len() > MAX_WHEELS {
            return Err("vehicle move carries more than 18 wheel angular velocities");
        }
        let mut body = MoveUnit::default();
        body.set_move_type(2);
        body.set_time(time);
        body.set_fixed_pos(FixedPos {
            x: coord_to_long(pos.x),
            y: coord_to_long(pos.y),
            z: pos.z,
        });
        body.set_velocity(velocity_mps);
        body.set_rotation(rotation);
        body.set_vehicle_ang_vel(angular_velocity);
        body.set_vehicle_steering(steering);
        body.set_wheel_count(wheel_ang_vel.len() as u8);
        for (i, v) in wheel_ang_vel.iter().enumerate() {
            body.set_wheel_ang_vel(i, *v);
        }
        Self::new(slave_obj_id, body)
    }

    pub fn slave_obj_id(&self) -> u32 {
        self.slave_obj_id
    }

    /// The native type-2 body.
    pub fn body(&self) -> &MoveUnit {
        &self.body
    }

    pub fn time(&self) -> u32 {
        self.body.time()
    }

    /// The session fills the client physics time immediately before sending.
    pub fn set_time(&mut self, time: u32) {
        self.body.set_time(time);
    }

    /// Original397c0470 case 2: BC3, type/header, position, velocity, rotation,
    /// angular velocity, steering, wheel count and wheel angular velocities.
    pub(crate) fn write(&self, w: &mut Writer) {
        w.bc(self.slave_obj_id);
        self.body.write_to(w);
    }
}

#[cfg(test)]
mod tests;
