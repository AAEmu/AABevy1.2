use crate::wire::{Reader, Result};

/// Native manager states 5 through 8; all four count as being in an instant game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BattlefieldPhase {
    Joined,
    Opening,
    Playing,
    Ending,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BattlefieldStart {
    pub zone_id: u32,
    pub instance_id: u32,
    /// Received clock fields, not a replacement for the client's mapped server clock.
    pub now_ms: u32,
    pub start_ms: u32,
}
impl BattlefieldStart {
    pub(super) fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            zone_id: r.u32()?,
            instance_id: r.u32()?,
            now_ms: r.u32()?,
            start_ms: r.u32()?,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BattlefieldMemberResult {
    pub character_id: u32,
    pub name: String,
    pub score: i32,
    pub bonus: i32,
    pub bonus_set: u32,
    pub killstreak: u16,
    pub kills: u16,
    pub assists: u16,
    pub deaths: u16,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BattlefieldTeamResult {
    pub state: u8,
    pub faction_id: u32,
    pub score: i32,
    pub total_kills: i16,
    pub total_deaths: i16,
    pub members: Vec<BattlefieldMemberResult>,
}
impl BattlefieldTeamResult {
    fn read_lane(r: &mut Reader) -> Result<Self> {
        let state = r.u8()?;
        let faction_id = r.u32()?;
        let score = r.i32()?;
        let total_kills = r.i16()?;
        let total_deaths = r.i16()?;
        let count = r.u8()?;
        // The lane writes the full u8 count. Native inline storage clamps this to 50;
        // do not apply that clamp to the active lane stream and misalign its tail.
        let mut members = Vec::with_capacity(usize::from(count));
        for _ in 0..count {
            members.push(BattlefieldMemberResult {
                character_id: r.u32()?,
                name: r.string()?,
                score: r.i32()?,
                bonus: r.i32()?,
                bonus_set: r.u32()?,
                killstreak: r.u16()?,
                kills: r.u16()?,
                assists: r.u16()?,
                deaths: r.u16()?,
            });
        }
        Ok(Self {
            state,
            faction_id,
            score,
            total_kills,
            total_deaths,
            members,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BattlefieldEnd {
    pub zone_id: u32,
    pub instance_id: u32,
    pub reason: u8,
    pub teams: [BattlefieldTeamResult; 2],
}
impl BattlefieldEnd {
    pub(super) fn read_lane(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            zone_id: r.u32()?,
            instance_id: r.u32()?,
            reason: r.u8()?,
            teams: [BattlefieldTeamResult::read_lane(r)?, BattlefieldTeamResult::read_lane(r)?],
        })
    }
}
