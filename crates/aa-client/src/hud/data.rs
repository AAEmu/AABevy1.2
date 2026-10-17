//! What the HUD knows: the game database tables it needs, held in memory, and the snapshot of the
//! live game state the scripts read.
//!
//! The database (`compact.sqlite3`, read-only) has no indexes: one lookup is a table scan, and the
//! English names live in `localized_texts` (260k rows). So everything the HUD asks for (names and
//! icons of skills, items, buffs, NPC templates, zones, factions, the exp table) is read once by a
//! background thread at start-up ([`GameData::load_in_background`]) and queried from memory.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use bevy::prelude::*;
use rusqlite::{Connection, OpenFlags};

/// Name and icon of a skill, item or buff (`ui/icon/<icon>`).
#[derive(Clone, Debug, Default)]
pub struct Named {
    pub name: String,
    pub icon: String,
}

#[derive(Clone, Debug, Default)]
pub struct ItemInfo {
    pub named: Named,
    pub max_stack: i64,
    /// `items.level_requirement`.
    pub level: i64,
    pub fixed_grade: i64,
}

#[derive(Clone, Debug, Default)]
pub struct NpcInfo {
    pub name: String,
    pub level: i64,
    /// `npcs.npc_grade_id` (1 = normal, 2 = ... 7 = ...).
    pub grade: i64,
    pub kind: i64,
    pub faction_id: i64,
    pub show_name_tag: bool,
}

/// A zone group (what the HUD calls a zone: "Solzreed Peninsula") and where it is on the map.
#[derive(Clone, Debug, Default)]
pub struct ZoneGroup {
    /// English name.
    pub text: String,
    /// `zone_groups.name` (`w_solzreed`): the key of the map images and forms.
    pub key: String,
    /// Map rectangle in metres: west, south, width, height.
    pub rect: [f32; 4],
}

/// The static game data the HUD queries.
#[derive(Default)]
pub struct GameData {
    pub skills: HashMap<u32, Named>,
    pub items: HashMap<u32, ItemInfo>,
    pub buffs: HashMap<u32, Named>,
    pub npcs: HashMap<u32, NpcInfo>,
    /// `zones.zone_key` -> (zone group id, protecting faction id).
    pub zones: HashMap<u32, (u32, u32)>,
    /// `zone_groups.id` -> name, map rectangle.
    pub zone_groups: HashMap<u32, ZoneGroup>,
    /// `system_factions.id` -> English name.
    pub factions: HashMap<u32, String>,
}

/// Loaded once, in the background; `get()` is `None` until it is done (a fraction of a second).
#[derive(Clone, Default)]
pub struct SharedGameData(pub Arc<OnceLock<GameData>>);

impl SharedGameData {
    pub fn get(&self) -> Option<&GameData> {
        self.0.get()
    }

    pub fn load_in_background(&self) {
        let slot = self.0.clone();
        std::thread::Builder::new()
            .name("hud-db".into())
            .spawn(move || {
                let t = std::time::Instant::now();
                let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
                match GameData::load(&path) {
                    Ok(d) => {
                        info!(
                            "hud: game data in {:.0} ms ({} skills, {} items, {} buffs, {} npcs)",
                            t.elapsed().as_secs_f32() * 1000.0,
                            d.skills.len(),
                            d.items.len(),
                            d.buffs.len(),
                            d.npcs.len()
                        );
                        let _ = slot.set(d);
                    }
                    Err(e) => {
                        warn!("hud: game database {path}: {e}");
                        let _ = slot.set(GameData::default());
                    }
                }
            })
            .ok();
    }
}

impl GameData {
    pub fn load(path: &str) -> rusqlite::Result<GameData> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
        // English texts: (table, column) -> id -> text
        let mut text: HashMap<(&'static str, u32), String> = HashMap::new();
        {
            let mut q = conn.prepare(
                "SELECT tbl_name, idx, en_us FROM localized_texts WHERE en_us IS NOT NULL AND en_us <> '' AND (\
                 (tbl_name IN ('skills','items','buffs','npcs','system_factions','sub_zones') AND tbl_column_name = 'name') OR \
                 (tbl_name = 'zone_groups' AND tbl_column_name = 'display_text'))",
            )?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let table: String = r.get(0)?;
                let idx: i64 = r.get(1)?;
                let table = match table.as_str() {
                    "skills" => "skills",
                    "items" => "items",
                    "buffs" => "buffs",
                    "npcs" => "npcs",
                    "system_factions" => "system_factions",
                    "sub_zones" => "sub_zones",
                    _ => "zone_groups",
                };
                text.insert((table, idx as u32), r.get(2)?);
            }
        }
        let name = |table: &'static str, id: u32, fallback: Option<String>| text.get(&(table, id)).cloned().or(fallback).unwrap_or_default();
        let mut icons: HashMap<i64, String> = HashMap::new();
        {
            let mut q = conn.prepare("SELECT id, filename FROM icons")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                icons.insert(r.get(0)?, r.get::<_, Option<String>>(1)?.unwrap_or_default());
            }
        }
        let icon = |id: Option<i64>| id.and_then(|i| icons.get(&i)).map(|f| format!("ui/icon/{}", f.replace('\\', "/").to_ascii_lowercase())).unwrap_or_default();
        let mut data = GameData::default();
        {
            let mut q = conn.prepare("SELECT id, name, icon_id FROM skills")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let id: u32 = r.get(0)?;
                data.skills.insert(id, Named { name: name("skills", id, r.get(1)?), icon: icon(r.get(2)?) });
            }
        }
        {
            let mut q = conn.prepare("SELECT id, name, icon_id, max_stack_size, level_requirement, fixed_grade FROM items")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let id: u32 = r.get(0)?;
                data.items.insert(
                    id,
                    ItemInfo {
                        named: Named { name: name("items", id, r.get(1)?), icon: icon(r.get(2)?) },
                        max_stack: r.get::<_, Option<i64>>(3)?.unwrap_or(1),
                        level: r.get::<_, Option<i64>>(4)?.unwrap_or(0),
                        fixed_grade: r.get::<_, Option<i64>>(5)?.unwrap_or(0),
                    },
                );
            }
        }
        {
            let mut q = conn.prepare("SELECT id, name, icon_id FROM buffs")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let id: u32 = r.get(0)?;
                data.buffs.insert(id, Named { name: name("buffs", id, r.get(1)?), icon: icon(r.get(2)?) });
            }
        }
        {
            let mut q = conn.prepare("SELECT id, name, level, npc_grade_id, npc_kind_id, faction_id, show_name_tag FROM npcs")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let id: u32 = r.get(0)?;
                data.npcs.insert(
                    id,
                    NpcInfo {
                        name: name("npcs", id, r.get(1)?),
                        level: r.get::<_, Option<i64>>(2)?.unwrap_or(1),
                        grade: r.get::<_, Option<i64>>(3)?.unwrap_or(1),
                        kind: r.get::<_, Option<i64>>(4)?.unwrap_or(1),
                        faction_id: r.get::<_, Option<i64>>(5)?.unwrap_or(0),
                        show_name_tag: r.get::<_, Option<String>>(6)?.as_deref() != Some("f"),
                    },
                );
            }
        }
        {
            let mut q = conn.prepare("SELECT zone_key, group_id, faction_id FROM zones WHERE group_id IS NOT NULL")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                data.zones.insert(r.get(0)?, (r.get(1)?, r.get::<_, Option<u32>>(2)?.unwrap_or(0)));
            }
        }
        {
            let mut q = conn.prepare("SELECT id, name, display_text, x, y, w, h FROM zone_groups")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let id: u32 = r.get(0)?;
                let f = |i: usize| -> rusqlite::Result<f32> { Ok(r.get::<_, Option<f64>>(i)?.unwrap_or(0.0) as f32) };
                data.zone_groups.insert(
                    id,
                    ZoneGroup {
                        text: name("zone_groups", id, r.get(2)?),
                        key: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                        rect: [f(3)?, f(4)?, f(5)?, f(6)?],
                    },
                );
            }
        }
        {
            let mut q = conn.prepare("SELECT id, name FROM system_factions")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let id: u32 = r.get(0)?;
                data.factions.insert(id, name("system_factions", id, r.get(1)?));
            }
        }
        Ok(data)
    }

    /// Zone group of a `zones.zone_key` ("Solzreed Peninsula") and the faction protecting the zone.
    pub fn zone(&self, zone_key: u32) -> Option<(u32, &ZoneGroup, u32)> {
        let (group, faction) = *self.zones.get(&zone_key)?;
        Some((group, self.zone_groups.get(&group)?, faction))
    }
}

/// Everything the scripts read about the local player and the world, as of this frame. Compared
/// with the last one pushed to the Lua state: only a difference raises UI events.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    /// The unit id the scripts use for the player (`X2Unit:GetUnitId("player")`).
    pub unit_id: String,
    pub name: String,
    pub level: u8,
    pub race: u8,
    pub gender: u8,
    pub hp: i32,
    pub max_hp: i32,
    pub mp: i32,
    pub max_mp: i32,
    pub dead: bool,
    pub in_combat: bool,
    /// Percent of the current level (0..100) and the exp of the level / needed for the next.
    pub exp_percent: f64,
    pub exp_cur: i64,
    pub exp_need: i64,
    pub recoverable_exp: i32,
    pub labor: i32,
    pub max_labor: i32,
    pub money: i64,
    pub bm_point: i64,
    pub honor: i32,
    pub vocation: i32,
    pub zone: String,
    /// `zone_groups.id` / `.name` / map rectangle of the zone the player is in.
    pub zone_group: u32,
    pub zone_key_name: String,
    pub zone_rect: [f32; 4],
    /// The faction protecting the zone (id, 0 = none) and its name.
    pub zone_faction: u32,
    pub zone_faction_name: String,
    pub sub_zone: String,
    /// The player's faction name.
    pub faction: String,
    /// Game clock: hour 0..24 and minute.
    pub hour: u8,
    pub minute: f32,
}
