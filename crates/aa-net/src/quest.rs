//! Quests as the server lists them: the active ones (`SCQuests`, `SCQuestContextStarted` /
//! `Updated`) and the completed ones as blocks of 64 flags (`SCCompletedQuests`).
//!
//! Layouts follow the server's `Quest.Write` (`ActiveQuest::write`) and `CompletedQuest`.

use crate::wire::{Reader, Result};

/// `QuestStatus` of an active quest.
pub mod status {
    pub const PROGRESS: u8 = 1;
    pub const FAILED: u8 = 2;
    /// All objectives done: the quest waits to be handed in.
    pub const READY: u8 = 3;
    pub const DROPPED: u8 = 4;
    pub const COMPLETED: u8 = 5;
    pub const DAILY_COMPLETED: u8 = 6;
}

/// An active quest.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Quest {
    /// The server's id of this instance of the quest.
    pub id: i64,
    /// `quest_contexts.id`: the quest.
    pub template_id: u32,
    /// See [`status`].
    pub status: u8,
    /// How far the objectives of the current step are.
    pub objectives: [i32; 5],
    /// Milliseconds left of a quest with a time limit, -1 for none.
    pub time_left_ms: i32,
    /// `quest_components.id` of the current step.
    pub component_id: u32,
    /// Unix seconds when it was accepted.
    pub accept_time: i64,
    /// Who gave it (`QuestAcceptorType`: 1 NPC, 2 doodad ...) and its template id.
    pub acceptor_type: u8,
    pub acceptor_id: u32,
}

impl Quest {
    pub fn read(r: &mut Reader) -> Result<Quest> {
        let id = r.i64()?;
        let template_id = r.u32()?;
        let status = r.u8()?;
        let mut objectives = [0; 5];
        for o in &mut objectives {
            *o = r.i32()?;
        }
        r.skip(1)?; // isCheckSet
        r.bc()?; // object id
        r.u32()?; // type
        r.bc()?;
        r.bc()?;
        let time_left_ms = r.i32()?;
        let component_id = r.u32()?;
        r.i64()?; // doodad id
        let accept_time = r.i64()?;
        let acceptor_type = r.u8()?;
        let acceptor_id = r.u32()?;
        Ok(Quest { id, template_id, status, objectives, time_left_ms, component_id, accept_time, acceptor_type, acceptor_id })
    }
}

/// 64 completed-quest flags: block `id` holds the quests `id * 64 ..= id * 64 + 63` (bit `n` of
/// `bits`, the first byte of the packet being the lowest).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CompletedBlock {
    pub id: u16,
    pub bits: u64,
}

impl CompletedBlock {
    pub fn read(r: &mut Reader) -> Result<CompletedBlock> {
        let id = r.u16()?;
        let bytes: [u8; 8] = r.bytes(8)?.try_into().expect("8 bytes");
        Ok(CompletedBlock { id, bits: u64::from_le_bytes(bytes) })
    }

    /// The block quest `quest_id` lives in.
    pub fn block_of(quest_id: u32) -> u16 {
        (quest_id / 64) as u16
    }

    pub fn has(&self, quest_id: u32) -> bool {
        Self::block_of(quest_id) == self.id && self.bits >> (quest_id % 64) & 1 == 1
    }
}

/// The active quests of a list packet (`SCQuests`): a count, then one quest each.
pub fn read_quests(r: &mut Reader) -> Result<Vec<Quest>> {
    let n = r.i32()?.max(0) as usize;
    let mut out = Vec::with_capacity(n.min(256));
    for _ in 0..n {
        out.push(Quest::read(r)?);
    }
    Ok(out)
}

/// The completed blocks of `SCCompletedQuests`.
pub fn read_completed(r: &mut Reader) -> Result<Vec<CompletedBlock>> {
    let n = r.i32()?.max(0) as usize;
    let mut out = Vec::with_capacity(n.min(1024));
    for _ in 0..n {
        out.push(CompletedBlock::read(r)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::Writer;

    pub(crate) fn write_quest(w: &mut Writer, q: &Quest) {
        w.i64(q.id).u32(q.template_id).u8(q.status);
        for o in q.objectives {
            w.i32(o);
        }
        w.bool(false).bc(0).u32(0).bc(0).bc(0).i32(q.time_left_ms).u32(q.component_id).i64(0).i64(q.accept_time).u8(q.acceptor_type).u32(q.acceptor_id);
    }

    #[test]
    fn a_quest_round_trips_the_servers_layout() {
        let q = Quest {
            id: 77,
            template_id: 330,
            status: status::PROGRESS,
            objectives: [1, 2, 0, 0, 0],
            time_left_ms: -1,
            component_id: 1520,
            accept_time: 1_700_000_000,
            acceptor_type: 1,
            acceptor_id: 3597,
        };
        let mut w = Writer::new();
        w.i32(2);
        write_quest(&mut w, &q);
        write_quest(&mut w, &Quest { template_id: 331, ..q.clone() });
        let bytes = w.finish();
        let list = read_quests(&mut Reader::new(&bytes)).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0], q);
        assert_eq!(list[1].template_id, 331);
    }

    #[test]
    fn completed_blocks_hold_64_quests_each() {
        let mut w = Writer::new();
        w.i32(1).u16(5).raw(&[0b0000_0100, 0, 0, 0, 0, 0, 0, 0x80]);
        let bytes = w.finish();
        let blocks = read_completed(&mut Reader::new(&bytes)).unwrap();
        assert_eq!(blocks.len(), 1);
        // block 5 holds 320..=383: bit 2 is 322, bit 63 is 383
        assert!(blocks[0].has(322) && blocks[0].has(383));
        assert!(!blocks[0].has(321) && !blocks[0].has(66));
    }
}
