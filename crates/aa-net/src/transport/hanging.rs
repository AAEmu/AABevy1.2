//! Fixed original Hang/Unhang wire requests. These do not establish admission.

use crate::{opcodes::cs, wire::Writer};

/// A checked packet body, not an authorized hanging action.
///
/// Callers must establish the current own actor, native eligibility and target
/// lifetime before submission. Unhang has no wire target: a future session
/// adapter must fence a captured target against replacement before writing.
/// There is deliberately no Session command until those inputs are available.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HangingRequest {
    unit_obj_id: u32,
    action: Action,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Hang { target_obj_id: u32 },
    Unhang { reason: i32 },
}

impl HangingRequest {
    /// CS0cb: two BC3 IDs. Nonzero IDs are an outbound application boundary;
    /// accepting them proves neither actor ownership nor target eligibility.
    pub fn hang(unit_obj_id: u32, target_obj_id: u32) -> Result<Self, &'static str> {
        check_id(unit_obj_id)?;
        check_id(target_obj_id)?;
        Ok(Self {
            unit_obj_id,
            action: Action::Hang { target_obj_id },
        })
    }

    /// CS0cc: BC3 actor followed by the complete signed i32 reason bits.
    /// No target, reason conversion, default reason or success is supplied.
    pub fn unhang(unit_obj_id: u32, reason: i32) -> Result<Self, &'static str> {
        check_id(unit_obj_id)?;
        Ok(Self {
            unit_obj_id,
            action: Action::Unhang { reason },
        })
    }

    pub fn unit_obj_id(&self) -> u32 {
        self.unit_obj_id
    }

    /// Only Hang sends a target. This is not Unhang's expected-current-target guard.
    pub fn target_obj_id(&self) -> Option<u32> {
        match self.action {
            Action::Hang { target_obj_id } => Some(target_obj_id),
            Action::Unhang { .. } => None,
        }
    }

    pub fn reason(&self) -> Option<i32> {
        match self.action {
            Action::Unhang { reason } => Some(reason),
            Action::Hang { .. } => None,
        }
    }

    pub(crate) fn opcode(&self) -> u16 {
        match self.action {
            Action::Hang { .. } => cs::CSHangPacket,
            Action::Unhang { .. } => cs::CSUnhangPacket,
        }
    }

    pub(crate) fn write(&self, w: &mut Writer) {
        w.bc(self.unit_obj_id);
        match self.action {
            Action::Hang { target_obj_id } => {
                w.bc(target_obj_id);
            }
            Action::Unhang { reason } => {
                w.i32(reason);
            }
        }
    }
}

fn check_id(id: u32) -> Result<(), &'static str> {
    if id == 0 || id > 0x00ff_ffff {
        return Err("hanging request requires nonzero 24-bit object IDs");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
