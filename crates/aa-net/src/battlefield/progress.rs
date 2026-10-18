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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BattlefieldKill {
    pub zone_id: u32,
    pub instance_id: u32,
    pub killer_id: u32,
    pub victim_id: u32,
    pub killer_corps: u8,
    pub victim_corps: u8,
    pub killer_killstreak: i8,
    pub killer_name: String,
    pub victim_name: String,
    pub killer_corps_kills: i32,
    pub victim_corps_deaths: i32,
}
impl BattlefieldKill {
    pub(super) fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            zone_id: r.u32()?,
            instance_id: r.u32()?,
            killer_id: r.u32()?,
            victim_id: r.u32()?,
            killer_corps: r.u8()?,
            victim_corps: r.u8()?,
            killer_killstreak: r.i8()?,
            killer_name: r.string()?,
            victim_name: r.string()?,
            killer_corps_kills: r.i32()?,
            victim_corps_deaths: r.i32()?,
        })
    }
}
