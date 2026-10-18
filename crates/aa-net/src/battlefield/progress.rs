use crate::wire::{Reader, Result};

/// SC193 contains cumulative team scores as well as the notification's point delta.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BattlefieldPoint {
    pub zone_id: u32,
    pub instance_id: u32,
    pub corps: u8,
    pub points: i32,
    pub scores: [i32; 2],
    pub character_name: String,
}
impl BattlefieldPoint {
    pub(super) fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            zone_id: r.u32()?,
            instance_id: r.u32()?,
            corps: r.u8()?,
            points: r.i32()?,
            scores: [r.i32()?, r.i32()?],
            character_name: r.string()?,
        })
    }
}
/// The signed killstreak byte is an index into the 24 skills supplied by SC190.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BattlefieldSkillEnable {
    pub zone_id: u32,
    pub instance_id: u32,
    pub killstreak: i8,
    pub skill_id: u32,
    pub enabled: bool,
}
impl BattlefieldSkillEnable {
    pub(super) fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            zone_id: r.u32()?,
            instance_id: r.u32()?,
            killstreak: r.i8()?,
            skill_id: r.u32()?,
            enabled: r.bool()?,
        })
    }
}
