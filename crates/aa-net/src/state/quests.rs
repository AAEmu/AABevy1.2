//! The local player's quests: the active ones and which quests are done.

use crate::quest::{status, CompletedBlock, Quest};
use crate::session::Event;

resource! {
    /// What the server says about the player's quests. Filled when the character spawns
    /// (`SCQuests`, `SCCompletedQuests`) and kept current from the quest packets.
    pub struct Quests {
        /// Quests being worked on (including those ready to be handed in).
        pub active: Vec<Quest>,
        /// Blocks of 64 completed flags, see [`CompletedBlock`].
        pub completed: Vec<CompletedBlock>,
    }
}

impl Quests {
    /// The active quest `template_id` (`quest_contexts.id`), if the player has it.
    pub fn active_quest(&self, template_id: u32) -> Option<&Quest> {
        self.active.iter().find(|q| q.template_id == template_id)
    }

    /// Whether the player has finished quest `quest_id` (and not reset it since).
    pub fn is_completed(&self, quest_id: u32) -> bool {
        self.completed.iter().any(|b| b.has(quest_id))
    }

    fn set_block(&mut self, block: CompletedBlock) {
        match self.completed.iter_mut().find(|b| b.id == block.id) {
            Some(b) => *b = block,
            None => self.completed.push(block),
        }
    }

    fn put(&mut self, quest: &Quest) {
        match self.active.iter_mut().find(|q| q.template_id == quest.template_id) {
            Some(q) => *q = quest.clone(),
            None => self.active.push(quest.clone()),
        }
    }

    /// Returns true when the data changed.
    pub fn apply(&mut self, e: &Event) -> bool {
        match e {
            Event::QuestsLoaded(list) => self.active = list.clone(),
            Event::CompletedQuests(blocks) => self.completed = blocks.clone(),
            Event::QuestStarted(q) => self.put(q),
            Event::QuestUpdated(q) => match q.status {
                // a dropped quest comes as an update with its status
                status::DROPPED | status::COMPLETED => {
                    let before = self.active.len();
                    self.active.retain(|a| a.template_id != q.template_id);
                    if self.active.len() == before {
                        return false;
                    }
                }
                _ => self.put(q),
            },
            Event::QuestCompleted { quest_id, block } => {
                self.active.retain(|a| a.template_id != *quest_id);
                self.set_block(*block);
            }
            Event::QuestReset { block, .. } => self.set_block(*block),
            Event::CharacterSelected(_) | Event::LeftWorld { .. } | Event::Disconnected { .. } => {
                if self.active.is_empty() && self.completed.is_empty() {
                    return false;
                }
                *self = Self::default();
            }
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quest(template_id: u32, status: u8) -> Quest {
        Quest { template_id, status, ..Quest::default() }
    }

    #[test]
    fn quests_start_progress_and_complete() {
        let mut q = Quests::default();
        assert!(q.apply(&Event::QuestsLoaded(vec![quest(330, status::PROGRESS)])));
        assert!(q.apply(&Event::CompletedQuests(vec![CompletedBlock { id: 0, bits: 1 << 5 }])));
        assert!(q.is_completed(5) && !q.is_completed(6));
        assert!(q.apply(&Event::QuestStarted(quest(331, status::PROGRESS))));
        assert_eq!(q.active.len(), 2);
        assert!(q.apply(&Event::QuestUpdated(quest(331, status::READY))));
        assert_eq!(q.active_quest(331).unwrap().status, status::READY);
        // handing in: no longer active, its flag is set in the block the server sends
        assert!(q.apply(&Event::QuestCompleted { quest_id: 331, block: CompletedBlock { id: 5, bits: 1 << 11 } }));
        assert!(q.active_quest(331).is_none() && q.is_completed(331));
        // a drop comes as an update
        assert!(q.apply(&Event::QuestUpdated(quest(330, status::DROPPED))));
        assert!(q.active.is_empty());
        assert!(!q.apply(&Event::QuestUpdated(quest(330, status::DROPPED))));
        // a reset block clears the flag
        assert!(q.apply(&Event::QuestReset { quest_id: 331, block: CompletedBlock { id: 5, bits: 0 } }));
        assert!(!q.is_completed(331));
        assert!(q.apply(&Event::LeftWorld { target: 1 }));
        assert!(q.completed.is_empty());
    }
}
