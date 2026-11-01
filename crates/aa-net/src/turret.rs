//! Turret aim: the `CSTurretState` (0x036) request and the posture type 8 body.
//!
//! The 1.2 client's aim editors (`39425490` for models without an installed turret, `39425580`
//! for installed-turret models) clamp in the `vehicle_models` turret limits and then call
//! `FUN_39356990`, which writes `bc unitId, f32 pitch, f32 yaw` at most once per 200 ms
//! (`DAT_3a169940 + 200 <= now`). The server reads exactly those three fields
//! (`packets/c2g_misc.rs::CSTurretStatePacket`) and relays the stored aim as posture type 8
//! (`SCUnitModelPostureChangedPacket` / `SCUnitStatePacket`), whose body is
//! `u8 8, bool has_loot_left, f32 pitch, f32 yaw`.
//!
//! The angles are radians in the slave model's own frame. Which object the client names as
//! `unitId` (the cannon slave, the gunner or the hull) is not yet captured (S12-C1); the server
//! expects the unit it lets the sender man, so this client sends the manned slave.

use crate::wire::{Reader, Result, Writer};

/// An aim in a vehicle model's frame: `pitch` up from the model's horizontal plane and `yaw`
/// counter-clockwise from the model's forward axis, radians.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TurretAim {
    pub pitch: f32,
    pub yaw: f32,
}

impl TurretAim {
    pub const fn new(pitch: f32, yaw: f32) -> Self {
        Self { pitch, yaw }
    }

    pub fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            pitch: r.f32()?,
            yaw: r.f32()?,
        })
    }
}

/// `CSTurretStatePacket` (0x036): the manned slave, then its model-frame aim.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TurretState {
    pub unit_id: u32,
    pub aim: TurretAim,
}

impl TurretState {
    pub const fn new(unit_id: u32, pitch: f32, yaw: f32) -> Self {
        Self {
            unit_id,
            aim: TurretAim::new(pitch, yaw),
        }
    }

    /// The server reader (`CSTurretStatePacket::read`): `read_bc`, `read_f32`, `read_f32`.
    pub fn write(&self, w: &mut Writer) {
        w.bc(self.unit_id).f32(self.aim.pitch).f32(self.aim.yaw);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::ClientMsg;
    use crate::opcodes::cs;

    /// The server's reader (`packets/c2g_misc.rs`): a `bc` id, then the pitch and the yaw as
    /// little-endian `f32`. The object `0xaabbcc` is three bytes, so a non-24-bit id would move
    /// the floats.
    #[test]
    fn turret_state_body_is_the_servers_reader_layout() {
        let state = TurretState::new(0xaa_bbcc, 0.25, -0.5);
        let mut expected = vec![0xcc, 0xbb, 0xaa];
        expected.extend_from_slice(&0.25f32.to_le_bytes());
        expected.extend_from_slice(&(-0.5f32).to_le_bytes());
        let mut w = Writer::new();
        state.write(&mut w);
        assert_eq!(w.finish(), expected);
        assert_eq!(expected.len(), 11);

        let message = ClientMsg::TurretState(state);
        assert_eq!(message.opcode(), cs::CSTurretStatePacket);
        assert_eq!(message.opcode(), 0x036);
        assert_eq!(message.encode_body(), expected);

        // The 1.2 serializer `FUN_397c7640` calls the fields "unitId", "pitch", "yaw": a body
        // shorter than 11 bytes cannot hold all three, and the server's reader would fail.
        let mut r = Reader::new(&expected);
        assert_eq!(r.bc().unwrap(), 0xaa_bbcc);
        assert_eq!(r.f32().unwrap(), 0.25);
        assert_eq!(r.f32().unwrap(), -0.5);
        assert!(r.is_empty());
    }
}
