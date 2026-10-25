//! Fixed transport receipts and checked hanging/ship-control wire requests, without prediction.
//!
//! Original serializers397c1ea0/397c11d0,397b37a0,397b5960 and397b5a00
//! supply the field order. Raw bool bytes use the accepted existing Rust/lane
//! one-byte wire convention. Concrete native archive normalization remains open,
//! including for0/1; these bytes are not post-archive native bool values.

use crate::opcodes::sc;
use crate::wire::{Reader, Result, WireError};

mod hanging;
pub use hanging::HangingRequest;
mod ship_control;
pub use ship_control::ShipControlRequest;

#[derive(Clone, Debug, PartialEq)]
pub enum TransportReceipt {
    ControllerState {
        obj_id: u32,
        sc_type: u8,
        /// Present only for controller type0. Unknown nonzero types have no suffix.
        rope: Option<ReceivedRopeState>,
    },
    FlyingState {
        obj_id: u32,
        flying_wire_byte: u8,
    },
    Hung {
        unit_obj_id: u32,
        target_obj_id: u32,
    },
    Unhung {
        unit_obj_id: u32,
        target_obj_id: u32,
        reason: i32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReceivedRopeState {
    pub length: f32,
    pub teared_wire_byte: u8,
    pub cutouted_wire_byte: u8,
}

impl TransportReceipt {
    pub fn handles(opcode: u16) -> bool {
        matches!(
            opcode,
            sc::SCSkillControllerStatePacket
                | sc::SCUnitFlyingStateChangedPacket
                | sc::SCHungPacket
                | sc::SCUnhungPacket
        )
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::ControllerState { .. } => "SCSkillControllerState",
            Self::FlyingState { .. } => "SCUnitFlyingStateChanged",
            Self::Hung { .. } => "SCHung",
            Self::Unhung { .. } => "SCUnhung",
        }
    }

    pub fn read(opcode: u16, r: &mut Reader) -> Result<Self> {
        Ok(match opcode {
            sc::SCSkillControllerStatePacket => {
                let obj_id = r.bc()?;
                let sc_type = r.u8()?;
                let rope = if sc_type == 0 {
                    Some(ReceivedRopeState {
                        length: r.f32()?,
                        teared_wire_byte: r.u8()?,
                        cutouted_wire_byte: r.u8()?,
                    })
                } else {
                    None
                };
                Self::ControllerState {
                    obj_id,
                    sc_type,
                    rope,
                }
            }
            sc::SCUnitFlyingStateChangedPacket => Self::FlyingState {
                obj_id: r.bc()?,
                flying_wire_byte: r.u8()?,
            },
            sc::SCHungPacket => Self::Hung {
                unit_obj_id: r.bc()?,
                target_obj_id: r.bc()?,
            },
            sc::SCUnhungPacket => Self::Unhung {
                unit_obj_id: r.bc()?,
                target_obj_id: r.bc()?,
                reason: r.i32()?,
            },
            _ => {
                return Err(WireError::Invalid(format!(
                    "unknown fixed transport opcode {opcode:03x}"
                )));
            }
        })
    }
}

#[cfg(test)]
pub(crate) mod tests;
