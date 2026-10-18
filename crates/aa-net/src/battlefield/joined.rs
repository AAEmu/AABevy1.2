use crate::wire::{Reader, Result};

/// Exact received SC190 fields shared by original 1.2 and the active lane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BattlefieldJoined {
    pub zone_id: u32,
    pub instance_id: u32,
    pub corps: u8,
    pub victory_score: i32,
    pub victory_kill_count: u16,
    pub victory_kill_corps_head: u32,
    pub phase_times_ms: [i32; 3],
    pub factions: [u32; 2],
    pub killstreak_skills: [u32; 24],
}
impl BattlefieldJoined {
    pub(super) fn read(r: &mut Reader) -> Result<Self> {
        let zone_id = r.u32()?;
        let instance_id = r.u32()?;
        let corps = r.u8()?;
        let victory_score = r.i32()?;
        let victory_kill_count = r.u16()?;
        let victory_kill_corps_head = r.u32()?;
        let phase_times_ms = [r.i32()?, r.i32()?, r.i32()?];
        let factions = [r.u32()?, r.u32()?];
        let mut killstreak_skills = [0; 24];
        for skill in &mut killstreak_skills {
            *skill = r.u32()?;
        }
        Ok(Self {
            zone_id,
            instance_id,
            corps,
            victory_score,
            victory_kill_count,
            victory_kill_corps_head,
            phase_times_ms,
            factions,
            killstreak_skills,
        })
    }
}
