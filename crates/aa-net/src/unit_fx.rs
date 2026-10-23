//! Small per-unit changes: a physical push (knockback), the weapon set in hand, the idle (away)
//! status and the gear visibility options.
//!
//! The layouts are the lane server's writers, checked against the 1.2 client's readers:
//!
//! | packet | body |
//! |---|---|
//! | `SCActiveWeaponChanged` 0x06f | `bc unit, u8 weapon set` |
//! | `SCUnitImpulse` 0x079 | `bc unit`, `u8 0, bc unit` (the lane writes these 4 bytes as one float), `f32 x3` velocity, angular velocity, impulse, angular impulse |
//! | `SCUnitVisualOptions` 0x1cb | `bc unit, u8 flags`, then by flag: 1 `[u8; 6]`, 2 helmet, 4 back holdable, 8 cosplay, 16 cosplay backpack (`bool` each) |
//! | `SCUnitIdleStatus` 0x1f5 | `bc unit, bool idle` |
//!
//! The 1.2 client (`OnImpulseUnit` 391d7750) hands the impulse to the unit's physical entity
//! when physics runs for it; `OnUnitIdleStatus` (391e23a0) stores the flag on the unit and, for
//! the own character, tells the player controller.
use crate::wire::{Reader, Result, WireError};

pub mod op {
    pub const ACTIVE_WEAPON_CHANGED: u16 = 0x06f;
    pub const UNIT_IMPULSE: u16 = 0x079;
    pub const UNIT_VISUAL_OPTIONS: u16 = 0x1cb;
    pub const UNIT_IDLE_STATUS: u16 = 0x1f5;
}

/// A unit's gear visibility options (`CharacterVisualOptions`); `None` = not in this packet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnitVisualOptions {
    pub stp: Option<[u8; 6]>,
    pub helmet: Option<bool>,
    pub back_holdable: Option<bool>,
    pub cosplay: Option<bool>,
    pub cosplay_backpack: Option<bool>,
}

impl UnitVisualOptions {
    fn read(r: &mut Reader) -> Result<Self> {
        let flag = r.u8()?;
        let mut o = Self::default();
        if flag & 1 != 0 {
            o.stp = Some(r.bytes(6)?.try_into().expect("six bytes"));
        }
        let mut field = |bit: u8| -> Result<Option<bool>> { if flag & bit != 0 { r.bool().map(Some) } else { Ok(None) } };
        o.helmet = field(2)?;
        o.back_holdable = field(4)?;
        o.cosplay = field(8)?;
        o.cosplay_backpack = field(16)?;
        Ok(o)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UnitFxEvent {
    ActiveWeaponChanged { obj_id: u32, weapon_set: u8 },
    /// A push (knockback, pull): velocity and angular velocity changes and the impulses, in world
    /// axes (x east, y north, z up).
    Impulse { obj_id: u32, velocity: [f32; 3], angular_velocity: [f32; 3], impulse: [f32; 3], angular_impulse: [f32; 3] },
    VisualOptions { obj_id: u32, options: UnitVisualOptions },
    /// The unit went away from keyboard (`true`) or came back.
    IdleStatus { obj_id: u32, idle: bool },
}

impl UnitFxEvent {
    pub fn handles(opcode: u16) -> bool {
        matches!(opcode, op::ACTIVE_WEAPON_CHANGED | op::UNIT_IMPULSE | op::UNIT_VISUAL_OPTIONS | op::UNIT_IDLE_STATUS)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::ActiveWeaponChanged { .. } => "SCActiveWeaponChanged",
            Self::Impulse { .. } => "SCUnitImpulse",
            Self::VisualOptions { .. } => "SCUnitVisualOptions",
            Self::IdleStatus { .. } => "SCUnitIdleStatus",
        }
    }

    pub fn obj_id(&self) -> u32 {
        match *self {
            Self::ActiveWeaponChanged { obj_id, .. }
            | Self::Impulse { obj_id, .. }
            | Self::VisualOptions { obj_id, .. }
            | Self::IdleStatus { obj_id, .. } => obj_id,
        }
    }

    pub fn read(opcode: u16, r: &mut Reader) -> Result<Self> {
        let vec3 = |r: &mut Reader| -> Result<[f32; 3]> { Ok([r.f32()?, r.f32()?, r.f32()?]) };
        Ok(match opcode {
            op::ACTIVE_WEAPON_CHANGED => Self::ActiveWeaponChanged { obj_id: r.bc()?, weapon_set: r.u8()? },
            op::UNIT_IMPULSE => {
                let obj_id = r.bc()?;
                // the client's target union: kind 0 = a unit id
                let kind = r.u8()?;
                if kind != 0 {
                    return Err(WireError::Invalid(format!("impulse target kind {kind}")));
                }
                r.bc()?;
                Self::Impulse { obj_id, velocity: vec3(r)?, angular_velocity: vec3(r)?, impulse: vec3(r)?, angular_impulse: vec3(r)? }
            }
            op::UNIT_VISUAL_OPTIONS => Self::VisualOptions { obj_id: r.bc()?, options: UnitVisualOptions::read(r)? },
            op::UNIT_IDLE_STATUS => Self::IdleStatus { obj_id: r.bc()?, idle: r.bool()? },
            opcode => return Err(WireError::Invalid(format!("not a unit fx packet: {opcode:#05x}"))),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::Writer;

    fn read(opcode: u16, w: &Writer) -> UnitFxEvent {
        let body = w.as_bytes();
        for n in 0..body.len() {
            assert!(UnitFxEvent::read(opcode, &mut Reader::new(&body[..n])).is_err(), "{opcode:#x} cut {n}");
        }
        UnitFxEvent::read(opcode, &mut Reader::new(body)).unwrap()
    }

    #[test]
    fn the_lane_impulse_decodes() {
        // the lane's SCUnitImpulsePacket: bc id, then f32::from_bits(id << 8) for the target union
        let mut w = Writer::new();
        w.bc(0x0a0b0c).f32(f32::from_bits(0x0a0b0c << 8));
        for v in [1.0, 2.0, 3.0, 0.0, 0.0, 0.5, 10.0, 0.0, 4.0, 0.0, 0.0, 0.0] {
            w.f32(v);
        }
        assert_eq!(
            read(op::UNIT_IMPULSE, &w),
            UnitFxEvent::Impulse {
                obj_id: 0x0a0b0c,
                velocity: [1.0, 2.0, 3.0],
                angular_velocity: [0.0, 0.0, 0.5],
                impulse: [10.0, 0.0, 4.0],
                angular_impulse: [0.0; 3]
            }
        );
    }

    #[test]
    fn weapon_idle_and_visual_options_decode() {
        let mut w = Writer::new();
        w.bc(5).u8(1);
        assert_eq!(read(op::ACTIVE_WEAPON_CHANGED, &w), UnitFxEvent::ActiveWeaponChanged { obj_id: 5, weapon_set: 1 });
        let mut w = Writer::new();
        w.bc(5).bool(true);
        assert_eq!(read(op::UNIT_IDLE_STATUS, &w), UnitFxEvent::IdleStatus { obj_id: 5, idle: true });
        let mut w = Writer::new();
        w.bc(5).u8(2 | 8).bool(false).bool(true);
        let options = UnitVisualOptions { helmet: Some(false), cosplay: Some(true), ..UnitVisualOptions::default() };
        assert_eq!(read(op::UNIT_VISUAL_OPTIONS, &w), UnitFxEvent::VisualOptions { obj_id: 5, options });
        let mut w = Writer::new();
        w.bc(5).u8(1 | 16).raw(&[1, 2, 3, 4, 5, 6]).bool(true);
        let options = UnitVisualOptions { stp: Some([1, 2, 3, 4, 5, 6]), cosplay_backpack: Some(true), ..UnitVisualOptions::default() };
        assert_eq!(read(op::UNIT_VISUAL_OPTIONS, &w), UnitFxEvent::VisualOptions { obj_id: 5, options });
    }
}
