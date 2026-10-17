//! Which NPCs have something to do with the player's quests: the marker over their head.
//!
//! The server does not say which NPC starts a quest: the client works it out from its quest tables,
//! the way the original does:
//!
//! * an NPC starts the quests whose *Start* component (`quest_components.component_kind_id` 2) has a
//!   `QuestActConAcceptNpc` act for it. The quest is on offer (a yellow "!") when the player has not
//!   done or taken it (a repeatable quest may be done before) and the component's `unit_reqs` hold:
//!   level, race, which other quests are done, items owned ... (all of them, or any when
//!   `or_unit_reqs`);
//! * an NPC is where a quest is handed in when the quest's *Ready* component (kind 6) has a
//!   `QuestActConReportNpc` act for it: a yellow "?" when the player's quest is ready, a grey "?"
//!   while it is still being worked on.
//!
//! The requirement kinds are the server's (`unit_reqs.rs`, `UnitReqsKindType`); one the client cannot
//! check from what it knows holds, as it does there.

use std::collections::{HashMap, HashSet};

use aa_net::quest::status;
use aa_net::state::Quests;
use rusqlite::Connection;

use super::data::{any_flag, any_int};

/// What marker an NPC carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// A quest to take ("!").
    Available,
    /// A quest ready to be handed in ("?").
    Ready,
    /// A quest being worked on that is handed in here (grey "?").
    InProgress,
}

/// A Start component: the quest it starts and what the player needs.
#[derive(Clone, Debug, Default)]
struct StartComponent {
    quest: u32,
    any_req: bool,
    hidden: bool,
    /// `(kind, value1, value2)` of `unit_reqs`.
    reqs: Vec<(u32, u32, u32)>,
}

/// The tables the markers come from.
#[derive(Default)]
pub struct QuestTables {
    components: HashMap<u32, StartComponent>,
    /// NPC template -> Start components it gives out.
    starters: HashMap<u32, Vec<u32>>,
    /// NPC template -> quests handed in to it.
    reporters: HashMap<u32, Vec<u32>>,
    repeatable: HashSet<u32>,
}

/// What the requirements read of the player.
#[derive(Default)]
pub struct PlayerFacts<'a> {
    pub level: u8,
    pub race: u8,
    pub gender: u8,
    pub faction: u32,
    pub honor: i32,
    pub vocation: i32,
    /// Item templates in the bag and worn.
    pub items: HashSet<u32>,
    pub skills: HashSet<u32>,
    pub buffs: HashSet<u32>,
    pub equipped: HashSet<u32>,
    pub quests: Option<&'a Quests>,
}

impl PlayerFacts<'_> {
    fn completed(&self, quest: u32) -> bool {
        self.quests.is_some_and(|q| q.is_completed(quest))
    }

    fn status_of(&self, quest: u32) -> Option<u8> {
        self.quests.and_then(|q| q.active_quest(quest)).map(|q| q.status)
    }

    /// One `unit_reqs` row; the kinds are `UnitReqsKindType`.
    fn holds(&self, kind: u32, v1: u32, v2: u32) -> bool {
        let level = u32::from(self.level);
        match kind {
            0x01 => level >= v1 && (v2 == 0 || level <= v2),
            0x03 => u32::from(self.race) == v1,
            0x04 => u32::from(self.gender) == v1,
            0x09 => self.equipped.contains(&v1),
            0x0A => self.items.contains(&v1),
            0x0B => self.skills.contains(&v1),
            0x0F => self.buffs.contains(&v1),
            0x16 => !self.buffs.contains(&v1),
            0x1F => self.completed(v1),
            0x20 => self.status_of(v1) == Some(status::PROGRESS),
            0x21 => self.status_of(v1) == Some(status::READY),
            0x24 => !self.completed(v1),
            0x25 => !self.completed(v1) && self.status_of(v1).is_some(),
            0x28 | 0x37 => self.faction == v1,
            0x3A => self.faction != v1,
            0x2D => (v1 as i64..=v2 as i64).contains(&i64::from(self.honor)),
            0x32 => (v1 as i64..=v2 as i64).contains(&i64::from(self.vocation)),
            0x45 => level <= v1,
            0x48 => !self.completed(v1) && self.status_of(v1) != Some(status::PROGRESS),
            0x49 => !self.completed(v1) && self.status_of(v1) != Some(status::READY),
            _ => true,
        }
    }
}

impl QuestTables {
    pub fn load(conn: &Connection) -> rusqlite::Result<QuestTables> {
        let mut t = QuestTables::default();
        let mut kind_of: HashMap<u32, (u32, u32)> = HashMap::new(); // component -> (quest, kind)
        {
            let mut q = conn.prepare("SELECT id, quest_context_id, component_kind_id, or_unit_reqs, hide_quest_marker FROM quest_components WHERE component_kind_id IN (2, 6)")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let (id, quest, kind) = (any_int(r, 0)? as u32, any_int(r, 1)? as u32, any_int(r, 2)? as u32);
                kind_of.insert(id, (quest, kind));
                if kind == 2 {
                    t.components.insert(id, StartComponent { quest, any_req: any_flag(r, 3)?, hidden: any_flag(r, 4)?, reqs: Vec::new() });
                }
            }
        }
        {
            let mut q = conn.prepare(
                "SELECT qa.quest_component_id, n.npc_id FROM quest_acts qa JOIN quest_act_con_accept_npcs n ON n.id = qa.act_detail_id WHERE qa.act_detail_type = 'QuestActConAcceptNpc'",
            )?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let (component, npc) = (any_int(r, 0)? as u32, any_int(r, 1)? as u32);
                if t.components.contains_key(&component) {
                    t.starters.entry(npc).or_default().push(component);
                }
            }
        }
        {
            let mut q = conn.prepare(
                "SELECT qa.quest_component_id, n.npc_id FROM quest_acts qa JOIN quest_act_con_report_npcs n ON n.id = qa.act_detail_id WHERE qa.act_detail_type = 'QuestActConReportNpc'",
            )?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let (component, npc) = (any_int(r, 0)? as u32, any_int(r, 1)? as u32);
                if let Some(&(quest, 6)) = kind_of.get(&component) {
                    let list = t.reporters.entry(npc).or_default();
                    if !list.contains(&quest) {
                        list.push(quest);
                    }
                }
            }
        }
        {
            let mut q = conn.prepare("SELECT owner_id, kind_id, value1, value2 FROM unit_reqs WHERE owner_type = 'QuestComponent'")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                if let Some(c) = t.components.get_mut(&(any_int(r, 0)? as u32)) {
                    c.reqs.push((any_int(r, 1)? as u32, any_int(r, 2)? as u32, any_int(r, 3)? as u32));
                }
            }
        }
        {
            let mut q = conn.prepare("SELECT id, REPEATABLE FROM quest_contexts")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                if any_flag(r, 1)? {
                    t.repeatable.insert(any_int(r, 0)? as u32);
                }
            }
        }
        Ok(t)
    }

    /// The marker of NPC template `npc` for this player: a quest to hand in first, then one to take,
    /// then one in progress; `None` when the NPC has nothing to do with the player's quests.
    pub fn mark(&self, npc: u32, player: &PlayerFacts) -> Option<Mark> {
        let reported = self.reporters.get(&npc).map_or(&[][..], Vec::as_slice);
        let statuses = || reported.iter().filter_map(|q| player.status_of(*q));
        if statuses().any(|s| s == status::READY) {
            return Some(Mark::Ready);
        }
        let offered = self.starters.get(&npc).is_some_and(|list| {
            list.iter().filter_map(|c| self.components.get(c)).any(|c| {
                if c.hidden || player.status_of(c.quest).is_some() || (player.completed(c.quest) && !self.repeatable.contains(&c.quest)) {
                    return false;
                }
                if c.reqs.is_empty() {
                    return true;
                }
                let holds = |r: &(u32, u32, u32)| player.holds(r.0, r.1, r.2);
                if c.any_req { c.reqs.iter().any(holds) } else { c.reqs.iter().all(holds) }
            })
        });
        if offered {
            return Some(Mark::Available);
        }
        statuses().any(|s| s == status::PROGRESS).then_some(Mark::InProgress)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aa_net::quest::{CompletedBlock, Quest};

    fn tables() -> Option<QuestTables> {
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
        QuestTables::load(&conn).ok()
    }

    #[test]
    fn maude_offers_the_first_quest_to_a_new_character() {
        let Some(t) = tables() else { return };
        // Maude (npcs.id 3597) starts quest 330 for a level 1 character; Maison (10688) starts nothing
        let quests = Quests::default();
        let new = PlayerFacts { level: 1, race: 1, quests: Some(&quests), ..PlayerFacts::default() };
        assert_eq!(t.mark(3597, &new), Some(Mark::Available));
        assert_eq!(t.mark(10688, &new), None);
        // taken: no longer on offer, and Maude is not where it is handed in (or: it is, in progress)
        let taken = Quests { active: vec![Quest { template_id: 330, status: status::PROGRESS, ..Quest::default() }], ..Quests::default() };
        let facts = PlayerFacts { level: 1, race: 1, quests: Some(&taken), ..PlayerFacts::default() };
        assert_ne!(t.mark(3597, &facts), Some(Mark::Available));
        // done: gone
        let done = Quests { completed: vec![CompletedBlock { id: 330 / 64, bits: 1 << (330 % 64) }], ..Quests::default() };
        let facts = PlayerFacts { level: 1, race: 1, quests: Some(&done), ..PlayerFacts::default() };
        assert_eq!(t.mark(3597, &facts), None);
    }
}
