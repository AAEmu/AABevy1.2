//! Original ranking receipts. Names, readiness, localization and reward delivery remain independent producers.
use crate::{
    session::{Event, Phase},
    wire::{Reader, Result, WireError},
};
use std::collections::BTreeMap;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RankCharacterRow {
    pub account_id: u32,
    pub account: Vec<u8>,
    pub character_id: u32,
    pub rank_id: u32,
    pub world_id: u8,
    pub v1: i32,
    pub raw_sum_v1: u8,
    pub v2: i32,
    pub raw_sum_v2: u8,
    pub recorded_raw: u64,
}
impl RankCharacterRow {
    fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            account_id: r.u32()?,
            account: r.blob()?.to_vec(),
            character_id: r.u32()?,
            rank_id: r.u32()?,
            world_id: r.u8()?,
            v1: r.i32()?,
            raw_sum_v1: r.u8()?,
            v2: r.i32()?,
            raw_sum_v2: r.u8()?,
            recorded_raw: r.u64()?,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RankSnapshotEntry {
    pub character_id: u32,
    pub rank_id: u32,
    pub world_id: u8,
    pub v1: i32,
    pub v2: i32,
    pub updated_raw: u64,
    pub ranking: i32,
}
impl RankSnapshotEntry {
    fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            character_id: r.u32()?,
            rank_id: r.u32()?,
            world_id: r.u8()?,
            v1: r.i32()?,
            v2: r.i32()?,
            updated_raw: r.u64()?,
            ranking: r.i32()?,
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RankSnapshotScope {
    pub scope_id: i32,
    pub v1: i32,
    pub v2: i32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RankSnapshot {
    pub board_id: i64,
    pub updated_raw: u64,
    pub reset_raw: u64,
    pub entry_count: i32,
    pub entries: Vec<RankSnapshotEntry>,
    pub scope_count: i32,
    pub scopes: Vec<RankSnapshotScope>,
}
fn count(r: &Reader, n: i32, min_bytes: usize) -> Result<usize> {
    let n = n.max(0) as usize;
    if n > r.remaining() / min_bytes {
        return Err(WireError::Invalid("ranking count exceeds remaining packet bytes".into()));
    }
    Ok(n)
}
impl RankSnapshot {
    fn read(r: &mut Reader) -> Result<Self> {
        let board_id = r.i64()?;
        let updated_raw = r.u64()?;
        let reset_raw = r.u64()?;
        let entry_count = r.i32()?;
        let n = count(r, entry_count, 29)?;
        let mut entries = Vec::with_capacity(n);
        for _ in 0..n {
            entries.push(RankSnapshotEntry::read(r)?);
        }
        let scope_count = r.i32()?;
        let n = count(r, scope_count, 12)?;
        let mut scopes = Vec::with_capacity(n);
        for _ in 0..n {
            scopes.push(RankSnapshotScope {
                scope_id: r.i32()?,
                v1: r.i32()?,
                v2: r.i32()?,
            });
        }
        Ok(Self {
            board_id,
            updated_raw,
            reset_raw,
            entry_count,
            entries,
            scope_count,
            scopes,
        })
    }
    fn admitted(&self) -> bool {
        self.entry_count <= 100
            && self.scope_count <= 100
            && self.entries.len() == self.entry_count.max(0) as usize
            && self.scopes.len() == self.scope_count.max(0) as usize
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RankingEvent {
    Personal {
        type_id: u32,
        error: i16,
        ack_time_raw: u64,
        count: i32,
        rows: Vec<RankCharacterRow>,
    },
    Snapshot {
        rank_id: u32,
        error: i16,
        board: RankSnapshot,
    },
    RewardMail {
        rank_id: u32,
    },
    Alarm {
        rank_id: u32,
        raw_kind: u8,
    },
}
impl RankingEvent {
    pub fn handles(op: u16) -> bool {
        matches!(op, 0x1f9 | 0x1fa | 0x1fb | 0x205)
    }
    /// Full lane-declared framing. Cache admission is separate from unsafe native over-cap readers.
    pub fn read(op: u16, r: &mut Reader) -> Result<Self> {
        Ok(match op {
            0x1f9 => Self::RewardMail { rank_id: r.u32()? },
            0x1fa => {
                let type_id = r.u32()?;
                let error = r.i16()?;
                let ack_time_raw = r.u64()?;
                let raw_count = r.i32()?;
                let n = count(r, raw_count, 33)?;
                let mut rows = Vec::with_capacity(n);
                for _ in 0..n {
                    rows.push(RankCharacterRow::read(r)?);
                }
                Self::Personal {
                    type_id,
                    error,
                    ack_time_raw,
                    count: raw_count,
                    rows,
                }
            }
            0x1fb => Self::Snapshot {
                rank_id: r.u32()?,
                error: r.i16()?,
                board: RankSnapshot::read(r)?,
            },
            0x205 => Self::Alarm {
                rank_id: r.u32()?,
                raw_kind: r.u8()?,
            },
            _ => return Err(WireError::Invalid("unknown ranking opcode".into())),
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RankBoard {
    pub personal: RankCharacterRow,
    pub snapshot: Option<RankSnapshot>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RankingNotice {
    /// Native error792 precedes PlayerRecords when an accepted personal receipt leaves no boards.
    Error(i32),
    /// RANK_PLAYER_RECORDS(), after all rows and acknowledgement time are assigned.
    PlayerRecords,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct Rankings {
    /// Actual selected persistent identity only; never inferred from a packet row.
    pub character_id: Option<u32>,
    pub by_id: BTreeMap<u32, RankBoard>,
    pub ack_time_raw: Option<u64>,
    pub last_receipt: Option<RankingEvent>,
}
impl Rankings {
    pub fn apply(&mut self, event: &Event) -> bool {
        self.apply_internal(event, &mut |_, _| {})
    }
    pub fn apply_with_notices(&mut self, event: &Event) -> (bool, Vec<RankingNotice>) {
        let mut notices = Vec::new();
        let changed = self.apply_internal(event, &mut |notice, _| notices.push(notice));
        (changed, notices)
    }
    /// Use callback-time views when presenting the native empty-map error before acknowledgement assignment.
    pub fn apply_with_dispatch(&mut self, event: &Event) -> (bool, Vec<(RankingNotice, Rankings)>) {
        let mut notices = Vec::new();
        let changed = self.apply_internal(event, &mut |notice, view| notices.push((notice, view.clone())));
        (changed, notices)
    }
    fn apply_internal(&mut self, event: &Event, emit: &mut impl FnMut(RankingNotice, &Self)) -> bool {
        match event {
            Event::LoginSucceeded(_) | Event::Disconnected { .. } | Event::Kicked { .. } | Event::PhaseChanged(Phase::Authenticating | Phase::Connecting) => {
                let changed = self.character_id.is_some() || !self.by_id.is_empty() || self.last_receipt.is_some() || self.ack_time_raw.is_some();
                *self = Self::default();
                return changed;
            }
            Event::CharacterSelected(s) => return self.bind(s.character.id),
            Event::PlayerSpawn(s) => return self.bind(s.state.character.id),
            Event::Ranking(receipt) => {
                let mut changed = self.last_receipt.as_ref() != Some(receipt);
                self.last_receipt = Some(receipt.clone());
                match receipt {
                    RankingEvent::Personal {
                        error: 0,
                        ack_time_raw,
                        count,
                        rows,
                        ..
                    } if *count <= 16 && rows.len() == (*count).max(0) as usize && rows.iter().all(|row| row.account.len() <= 16) => {
                        for row in rows {
                            match self.by_id.get_mut(&row.rank_id) {
                                Some(board) => {
                                    changed |= board.personal != *row;
                                    board.personal = row.clone();
                                }
                                None => {
                                    self.by_id.insert(
                                        row.rank_id,
                                        RankBoard {
                                            personal: row.clone(),
                                            snapshot: None,
                                        },
                                    );
                                    changed = true;
                                }
                            }
                        }
                        if self.by_id.is_empty() {
                            emit(RankingNotice::Error(792), self);
                        }
                        changed |= self.ack_time_raw != Some(*ack_time_raw);
                        self.ack_time_raw = Some(*ack_time_raw);
                        emit(RankingNotice::PlayerRecords, self);
                    }
                    RankingEvent::Snapshot { rank_id, error: 0, board } if board.admitted() => {
                        if let Some(current) = self.by_id.get_mut(rank_id) {
                            changed |= current.snapshot.as_ref() != Some(board);
                            current.snapshot = Some(board.clone());
                        }
                    }
                    _ => {}
                }
                return changed;
            }
            _ => {}
        }
        false
    }
    /// Explicit application isolation on actual character replacement; native reset caller remains unclosed.
    fn bind(&mut self, id: u32) -> bool {
        if self.character_id == Some(id) {
            return false;
        }
        *self = Self {
            character_id: Some(id),
            ..Self::default()
        };
        true
    }
}

#[cfg(test)]
mod tests;


/// Explicit ranking requests; tab selection, local locks and readiness remain the caller's policy.
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum RankingCommand { RequestPlayerRecords,RequestSnapshots{rank_id:u32} }
impl RankingCommand {
 pub fn opcode(self)->u16 {match self {Self::RequestPlayerRecords=>0x12f,Self::RequestSnapshots{..}=>0x130}}
 pub(crate) fn write(self,w:&mut crate::wire::Writer){if let Self::RequestSnapshots{rank_id}=self{w.u32(rank_id);}}
}
