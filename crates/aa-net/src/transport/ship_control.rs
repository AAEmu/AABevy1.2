//! Fixed CS089 type4 wire construction; no driver or controller admission.

use crate::{model::move_flags, wire::Writer};

/// A checked ship-control packet body, not an authorized driving action.
///
/// The object is the controlled slave, not its driver or persistent owner.
/// A future Session adapter must recheck the current own driver's received
/// attachment at point1, the slave incarnation and connection/world generation
/// immediately before writing. Ownership alone is insufficient. Controller
/// fields must come from an actual existing controller; this codec creates none.
/// There is deliberately no Session command or conversion to `Movement`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShipControlRequest {
    slave_obj_id: u32,
    time: u32,
    flags: u8,
    controller: Option<(u32, u8)>,
    throttle: i8,
    steering: i8,
}

impl ShipControlRequest {
    /// Requires a nonzero BC3 slave ID and `controller` iff flags has bit0x10.
    /// `controller` is `(sc_type, phase)`: the type is a full u32, unlike the
    /// separate controller-state packet's u8. Explicit zero values are retained.
    ///
    /// All flag/time/control bits are preserved without defaults or clamping.
    /// The signed byte domain is a wire boundary, not a physical throttle range.
    pub fn new(
        slave_obj_id: u32,
        time: u32,
        flags: u8,
        controller: Option<(u32, u8)>,
        throttle: i8,
        steering: i8,
    ) -> Result<Self, &'static str> {
        if slave_obj_id == 0 || slave_obj_id > 0x00ff_ffff {
            return Err("ship control requires a nonzero 24-bit slave object ID");
        }
        if controller.is_some() != (flags & move_flags::HAS_SC_TYPE_AND_PHASE != 0) {
            return Err("ship control requires controller fields exactly when flags has bit0x10");
        }
        Ok(Self {
            slave_obj_id,
            time,
            flags,
            controller,
            throttle,
            steering,
        })
    }

    pub fn slave_obj_id(&self) -> u32 {
        self.slave_obj_id
    }

    pub fn time(&self) -> u32 {
        self.time
    }

    pub fn flags(&self) -> u8 {
        self.flags
    }

    /// The supplied full-width controller type and phase, or no wire block.
    pub fn controller(&self) -> Option<(u32, u8)> {
        self.controller
    }

    pub fn throttle(&self) -> i8 {
        self.throttle
    }

    pub fn steering(&self) -> i8 {
        self.steering
    }

    /// Original397c52f0 ->397c0470: BC3, then the shared move serializer's
    /// type-4 case (header, throttle, steering). The codes live only in
    /// [`crate::vehicle::move_unit`]; this wrapper adds the BC3 and the checks.
    pub(crate) fn write(&self, w: &mut Writer) {
        w.bc(self.slave_obj_id);
        let mut mv = crate::vehicle::MoveUnit::default();
        mv.set_move_type(4);
        mv.set_time(self.time);
        mv.set_flags(self.flags);
        if let Some((sc_type, phase)) = self.controller {
            mv.set_sc_type(sc_type);
            mv.set_phase(phase);
        }
        mv.set_request_throttle(self.throttle);
        mv.set_request_steering(self.steering);
        mv.write_to(w);
    }
}

#[cfg(test)]
mod tests;
