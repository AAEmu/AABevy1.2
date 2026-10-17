//! The small slice of the game database the world sounds need: which sound an item makes, which voice an NPC has and what
//! kind of quest a quest is. Read on a thread at startup (when something can be heard or logged).

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use bevy::prelude::*;
use rusqlite::{Connection, OpenFlags};

/// What an NPC sounds like.
#[derive(Clone, Debug, PartialEq)]
pub struct NpcVoice {
    /// `sound_packs.name`, lower case; empty when the NPC has no pack.
    pub pack: String,
    /// The part of the pack's `<role>_interaction_start` / `_end` items its job picks: `store`, `blacksmith`, `bank` ...
    /// `talker` for an NPC that only talks.
    pub role: &'static str,
    /// `npcs.npc_kind_id` 1: a person (the NPC select sound, not the target select sound).
    pub person: bool,
}

#[derive(Default)]
pub struct Data {
    /// `items.id` -> (pickup sound id, use / equip sound id), the item's own or its category's.
    items: HashMap<u32, (i64, i64)>,
    npcs: HashMap<u32, NpcVoice>,
    /// `quest_contexts.id` -> `detail_id`.
    quests: HashMap<u32, u32>,
}

/// The job of an NPC, in the order the original picks the interaction sound: its first flag.
const ROLES: [(&str, &str); 13] = [
    ("skill_trainer", "skill_trainer"),
    ("ability_changer", "ability_change"),
    ("auctioneer", "auction"),
    ("priest", "priest_recover_exp"),
    ("teleporter", "teleporter"),
    ("blacksmith", "blacksmith"),
    ("banker", "bank"),
    ("stabler", "stabler"),
    ("repairman", "repairman"),
    ("trader", "trader"),
    ("specialty", "specialty_store"),
    ("expedition", "expedition"),
    ("merchant", "store"),
];

impl Data {
    pub fn load() -> rusqlite::Result<Data> {
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
        let mut data = Data::default();

        let mut q = conn.prepare(
            "SELECT i.id, COALESCE(NULLIF(i.pickup_sound_id, 0), c.pickup_sound_id, 0), \
             COALESCE(NULLIF(i.use_or_equipment_sound_id, 0), c.use_or_equipment_sound_id, 0) \
             FROM items i LEFT JOIN item_categories c ON c.id = i.category_id",
        )?;
        for r in q.query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))? {
            let (id, pickup, usage) = r?;
            if pickup != 0 || usage != 0 {
                data.items.insert(id, (pickup, usage));
            }
        }

        let flags = ROLES.iter().map(|(col, _)| format!("n.{col}")).collect::<Vec<_>>().join(", ");
        let mut q = conn.prepare(&format!("SELECT n.id, n.npc_kind_id, p.name, {flags} FROM npcs n LEFT JOIN sound_packs p ON p.id = n.sound_pack_id"))?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            let id: u32 = r.get(0)?;
            let kind: Option<u32> = r.get(1)?;
            let pack: Option<String> = r.get(2)?;
            let mut role = "talker";
            for (i, (_, name)) in ROLES.iter().enumerate() {
                if r.get::<_, Option<String>>(3 + i)?.is_some_and(|v| v == "t") {
                    role = name;
                    break;
                }
            }
            data.npcs.insert(id, NpcVoice { pack: pack.unwrap_or_default().to_ascii_lowercase(), role, person: kind == Some(1) });
        }

        let mut q = conn.prepare("SELECT id, detail_id FROM quest_contexts")?;
        for r in q.query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, Option<u32>>(1)?)))? {
            let (id, detail) = r?;
            data.quests.insert(id, detail.unwrap_or(0));
        }
        Ok(data)
    }

    /// (`items.pickup_sound_id`, `items.use_or_equipment_sound_id`) of an item template; 0 for none.
    pub fn item_sounds(&self, template: u32) -> (i64, i64) {
        self.items.get(&template).copied().unwrap_or((0, 0))
    }

    pub fn npc(&self, template: u32) -> Option<&NpcVoice> {
        self.npcs.get(&template)
    }

    /// The suffix of the `global` pack's `event_quest_<started|completed|failed|dropped>_<kind>` items for a quest.
    pub fn quest_kind(&self, quest: u32) -> &'static str {
        match self.quests.get(&quest).copied().unwrap_or(0) {
            2 => "main",
            5 => "hidden",
            6 => "task",
            7 | 10 | 13 => "daily",
            8 | 11 => "livelihood",
            9 | 12 => "group",
            _ => "normal",
        }
    }
}

/// The tables, loading on a thread.
#[derive(Resource, Clone)]
pub struct Tables(Arc<OnceLock<Data>>);

impl Tables {
    pub fn start(wanted: bool) -> Tables {
        let cell = Arc::new(OnceLock::new());
        if wanted {
            let c = cell.clone();
            std::thread::Builder::new()
                .name("sound-tables".into())
                .spawn(move || {
                    let started = std::time::Instant::now();
                    match Data::load() {
                        Ok(d) => {
                            debug!("sound: {} item, {} NPC and {} quest rows read in {:.0} ms", d.items.len(), d.npcs.len(), d.quests.len(), started.elapsed().as_secs_f32() * 1000.0);
                            let _ = c.set(d);
                        }
                        Err(e) => warn!("sound: cannot read the item / NPC sound tables: {e}"),
                    }
                })
                .ok();
        }
        Tables(cell)
    }

    pub fn get(&self) -> Option<&Data> {
        self.0.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quest_kinds() {
        let mut d = Data::default();
        d.quests.extend([(1, 2), (2, 7), (3, 8), (4, 1), (5, 9), (6, 5), (7, 6)]);
        assert_eq!(
            [1, 2, 3, 4, 5, 6, 7, 99].map(|q| d.quest_kind(q)),
            ["main", "daily", "livelihood", "normal", "group", "hidden", "task", "normal"]
        );
    }
}
