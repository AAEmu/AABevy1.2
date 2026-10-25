//! Recovered kind-1 candidate composition, separate from received SC07b offers.
//! Callers must supply qualified runtime facts; this module does not select an
//! actor, establish metadata membership, or turn missing providers into success.
use std::collections::HashMap;

use rusqlite::Connection;

const SERVICES: [(&str, u32); 15] = [
    ("skill_trainer", 12081),
    ("ability_changer", 12082),
    ("banker", 13238),
    ("auctioneer", 12083),
    ("trader", 14866),
    ("priest", 17063),
    ("teleporter", 12085),
    ("stabler", 12195),
    ("expedition", 13736),
    ("repairman", 15297),
    ("specialty", 16376),
    ("recruiting_battle_field_id", 12660),
    ("merchant", 12087),
    ("blacksmith", 12086),
    ("look_converter", 23478),
];

#[derive(Clone, Debug, Default)]
pub(super) struct Row {
    pub(super) enabled: [bool; 15],
    pub(super) custom: Vec<u32>,
}

#[derive(Default)]
pub(in crate::hud) struct Catalog {
    loaded: bool,
    rows: HashMap<u32, Row>,
}

#[derive(Clone, Copy)]
pub(super) enum Entry<'a> {
    Unknown,
    Missing,
    Loaded(&'a Row),
}

impl Catalog {
    pub(in crate::hud) fn load(conn: &Connection) -> rusqlite::Result<Self> {
        use crate::hud::data::{any_flag, any_int};
        let mut sets = HashMap::<u32, Vec<u32>>::new();
        let mut query = conn.prepare("SELECT id FROM npc_interaction_sets")?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            sets.insert(any_int(row, 0)? as u32, Vec::new());
        }
        // These are the native SQLs: return order, rather than numeric ID order,
        // determines custom slots. Resolve only sets that were actually loaded.
        let mut query =
            conn.prepare("SELECT id, npc_interaction_set_id, skill_id FROM npc_interactions")?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            if let Some(set) = sets.get_mut(&(any_int(row, 1)? as u32)) {
                set.push(any_int(row, 2)? as u32);
            }
        }
        let columns = SERVICES.iter().map(|s| s.0).collect::<Vec<_>>().join(",");
        let mut query = conn.prepare(&format!(
            "SELECT id,{columns},npc_interaction_set_id FROM npcs"
        ))?;
        let mut rows = query.query([])?;
        let mut out = Self::default();
        while let Some(row) = rows.next()? {
            let mut enabled = [false; 15];
            for (i, (column, _)) in SERVICES.iter().enumerate() {
                enabled[i] = if *column == "recruiting_battle_field_id" {
                    any_int(row, i + 1)? as u32 != 0
                } else {
                    any_flag(row, i + 1)?
                };
            }
            let set_id = any_int(row, 16)? as u32;
            out.rows.insert(
                any_int(row, 0)? as u32,
                Row {
                    enabled,
                    custom: sets.get(&set_id).cloned().unwrap_or_default(),
                },
            );
        }
        out.loaded = true;
        Ok(out)
    }

    pub(super) fn get(&self, id: u32) -> Entry<'_> {
        if !self.loaded {
            Entry::Unknown
        } else {
            self.rows.get(&id).map_or(Entry::Missing, Entry::Loaded)
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum MerchantRole {
    Ordinary,
    Dominion { authorized: bool },
    Defense { authorized: bool },
    Offense { authorized: bool },
}

/// Each Option is a provider result, not a default. In particular availability
/// is the known actor byte after native archive processing, not an arbitrary
/// noncanonical bool byte observed on the wire.
pub(super) struct Facts<'a> {
    pub(super) excluded: Option<bool>,
    pub(super) dead: bool,
    pub(super) loot: Option<bool>,
    pub(super) admitted: Option<bool>,
    pub(super) availability: Option<u8>,
    pub(super) metadata: Option<bool>,
    pub(super) row: Entry<'a>,
    pub(super) quest: Option<bool>,
    pub(super) role: Option<MerchantRole>,
    pub(super) specialty_extra: Option<bool>,
    pub(super) greeting: Option<bool>,
}

fn append(out: &mut Vec<u32>, skill: u32) {
    if !out.contains(&skill) {
        out.push(skill);
    }
}

fn services(out: &mut Vec<u32>, row: &Row, specialty_extra: Option<bool>) -> Option<()> {
    for (i, (_, skill)) in SERVICES.iter().enumerate() {
        if !row.enabled[i] {
            continue;
        }
        append(out, *skill);
        if *skill == 16376 && specialty_extra? {
            append(out, 17135);
        }
    }
    for &skill in &row.custom {
        append(out, skill);
    }
    Some(())
}

/// None means an unsupported provider; Some(empty) is a known empty result.
/// Native first-insertion uniqueness applies here, never to SC07b's raw list.
pub(super) fn compute(facts: Facts<'_>) -> Option<Vec<u32>> {
    if facts.excluded? {
        return Some(Vec::new());
    }
    if facts.dead && facts.loot? {
        return Some(vec![14047, 12079]);
    }
    if !facts.admitted? || facts.availability? == 0 || facts.dead {
        return Some(Vec::new());
    }
    if !facts.metadata? {
        return Some(Vec::new());
    }
    let row = match facts.row {
        Entry::Unknown => return None,
        Entry::Missing => return Some(Vec::new()),
        Entry::Loaded(row) => row,
    };
    let quest = facts.quest?;
    let mut out = Vec::new();
    if quest {
        append(&mut out, 13877);
    }
    let allowed = match facts.role? {
        MerchantRole::Ordinary => true,
        MerchantRole::Dominion { authorized } => {
            append(&mut out, 16912);
            authorized
        }
        MerchantRole::Defense { authorized } | MerchantRole::Offense { authorized } => authorized,
    };
    if allowed {
        services(&mut out, row, facts.specialty_extra)?;
    }
    if !quest && facts.greeting? {
        append(&mut out, 13356);
    }
    Some(out)
}

/// Native context refresh uses equality admission, unlike the 500 ms action gate.
#[derive(Default)]
pub(super) struct RefreshClock {
    pub(super) deadline: u32,
}
impl RefreshClock {
    pub(super) fn due(&mut self, now: u32) -> bool {
        if self.deadline == 0 {
            self.deadline = now.wrapping_add(350);
            return false;
        }
        if self.deadline > now {
            return false;
        }
        self.deadline = now.wrapping_add(350);
        true
    }
    pub(super) fn after_selection(&mut self, now: u32) {
        self.deadline = now.wrapping_add(350);
    }
}

#[cfg(test)]
mod tests;
