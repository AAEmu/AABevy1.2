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
    /// `items.use_skill_id`: the skill using the item casts (0 = not usable).
    pub use_skill: u32,
    /// `items.description`.
    pub desc: String,
}

#[derive(Clone, Debug, Default)]
pub struct SkillInfo {
    pub named: Named,
    /// `skills.cooldown_time` (ms) and `cooldown_tag_id`.
    pub cooldown_ms: u32,
    pub tag: u32,
}

/// A buff: name, icon, description and its list (`buffs.kind_id`: 1 good, 2 bad, 3 hidden).
#[derive(Clone, Debug, Default)]
pub struct BuffInfo {
    pub named: Named,
    pub desc: String,
    pub kind: i64,
}

#[derive(Clone, Debug, Default)]
pub struct NpcInfo {
    pub name: String,
    #[allow(dead_code)] // for the quest markers and the level text of the target frame
    pub level: i64,
    /// `npcs.npc_grade_id` (1 = normal, 2 = ... 7 = ...).
    pub grade: i64,
    #[allow(dead_code)]
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

/// A sub zone ("Desiree Peak") as the level designers drew it: a polygon on the ground.
#[derive(Clone, Debug, Default)]
pub struct SubZoneShape {
    /// `sub_zones.id`.
    pub id: u32,
    /// West, south, east, north of the polygon, game metres.
    pub bounds: [f32; 4],
    pub points: Vec<[f32; 2]>,
}

impl SubZoneShape {
    fn contains(&self, x: f32, y: f32) -> bool {
        let b = self.bounds;
        if x < b[0] || x > b[2] || y < b[1] || y > b[3] {
            return false;
        }
        let mut inside = false;
        let mut j = self.points.len().wrapping_sub(1);
        for i in 0..self.points.len() {
            let (pi, pj) = (self.points[i], self.points[j]);
            if (pi[1] > y) != (pj[1] > y) && x < (pj[0] - pi[0]) * (y - pi[1]) / (pj[1] - pi[1]) + pi[0] {
                inside = !inside;
            }
            j = i;
        }
        inside
    }

    fn area(&self) -> f32 {
        (self.bounds[2] - self.bounds[0]) * (self.bounds[3] - self.bounds[1])
    }
}

/// The value of attribute `name` in the first tag of `text` that has it (`name="..."`).
fn attr<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let key = format!(" {name}=\"");
    let at = text.find(&key)? + key.len();
    let end = text[at..].find('"')?;
    Some(&text[at..at + end])
}

fn pair(text: &str) -> Option<(f32, f32)> {
    let mut it = text.split(',');
    Some((it.next()?.trim().parse().ok()?, it.next()?.trim().parse().ok()?))
}

/// `worlds/<w>/level_design/zone/<id>/client/subzone_area.xml`: one `AreaShape` entity per polygon, the
/// points relative to the entity, the entity relative to its cell in the zone, the zone to its origin
/// cell in `world.xml` (cells are 1024 m).
fn parse_sub_zone_areas(text: &str, origin: (f32, f32), out: &mut Vec<SubZoneShape>) {
    for entity in text.split("<Entity ").skip(1) {
        let (Some(pos), Some(cx), Some(cy)) = (attr(entity, "Pos").and_then(pair), attr(entity, "cellX"), attr(entity, "cellY")) else {
            continue;
        };
        let (Ok(cx), Ok(cy)) = (cx.parse::<f32>(), cy.parse::<f32>()) else { continue };
        let Some(id) = entity.find("<Area Id=\"").and_then(|a| entity[a + 10..].split('"').next()).and_then(|v| v.parse::<u32>().ok()) else {
            continue;
        };
        let gx = (origin.0 + cx) * 1024.0 + pos.0;
        let gy = (origin.1 + cy) * 1024.0 + pos.1;
        let points: Vec<[f32; 2]> = entity
            .split("<Point Pos=\"")
            .skip(1)
            .filter_map(|p| pair(p.split('"').next()?))
            .map(|(x, y)| [gx + x, gy + y])
            .collect();
        if points.len() < 3 {
            continue;
        }
        let mut bounds = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for p in &points {
            bounds = [bounds[0].min(p[0]), bounds[1].min(p[1]), bounds[2].max(p[0]), bounds[3].max(p[1])];
        }
        out.push(SubZoneShape { id, bounds, points });
    }
}

/// The static game data the HUD queries.
#[derive(Default)]
pub struct GameData {
    pub skills: HashMap<u32, SkillInfo>,
    pub items: HashMap<u32, ItemInfo>,
    pub buffs: HashMap<u32, BuffInfo>,
    pub npcs: HashMap<u32, NpcInfo>,
    /// `zones.zone_key` -> (zone group id, protecting faction id).
    pub zones: HashMap<u32, (u32, u32)>,
    /// `zone_groups.id` -> name, map rectangle.
    pub zone_groups: HashMap<u32, ZoneGroup>,
    /// `system_factions.id` -> English name.
    pub factions: HashMap<u32, String>,
    /// `system_faction_relations`: (faction, other faction) -> 1 friendly, 2 neutral, 3 hostile.
    pub relations: HashMap<(u32, u32), u8>,
    /// `sub_zones.id` -> English name.
    pub sub_zone_names: HashMap<u32, String>,
    /// The sub zone polygons of the world (the database rectangles are mostly empty).
    pub sub_zone_shapes: Vec<SubZoneShape>,
    /// `zones.group_id` -> the faction protecting the group's zones (0 = none).
    pub group_faction: HashMap<u32, u32>,
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
                            "hud: game data in {:.0} ms ({} skills, {} items, {} buffs, {} npcs, {} sub zone shapes)",
                            t.elapsed().as_secs_f32() * 1000.0,
                            d.skills.len(),
                            d.items.len(),
                            d.buffs.len(),
                            d.npcs.len(),
                            d.sub_zone_shapes.len()
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

/// A column the table declares as text or integer, whichever the row holds (0 for null).
fn any_int(r: &rusqlite::Row, i: usize) -> rusqlite::Result<i64> {
    use rusqlite::types::Value;
    Ok(match r.get::<_, Value>(i)? {
        Value::Integer(n) => n,
        Value::Real(f) => f as i64,
        Value::Text(t) => t.trim().parse().unwrap_or(0),
        _ => 0,
    })
}

impl GameData {
    pub fn load(path: &str) -> rusqlite::Result<GameData> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
        // English texts: (table, column) -> id -> text
        let mut text: HashMap<(&'static str, u32), String> = HashMap::new();
        {
            let mut q = conn.prepare(
                "SELECT tbl_name, idx, en_us, tbl_column_name FROM localized_texts WHERE en_us IS NOT NULL AND en_us <> '' AND (\
                 (tbl_name IN ('skills','items','buffs','npcs','system_factions','sub_zones') AND tbl_column_name = 'name') OR \
                 (tbl_name = 'buffs' AND tbl_column_name = 'desc') OR                  (tbl_name = 'items' AND tbl_column_name = 'description') OR \
                 (tbl_name = 'zone_groups' AND tbl_column_name = 'display_text'))",
            )?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let table: String = r.get(0)?;
                let idx: i64 = r.get(1)?;
                let column: String = r.get(3)?;
                let table = match table.as_str() {
                    "buffs" if column == "desc" => "buffs_desc",
                    "items" if column == "description" => "items_desc",
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
            let mut q = conn.prepare("SELECT id, name, icon_id, cooldown_time, cooldown_tag_id FROM skills")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let id: u32 = r.get(0)?;
                data.skills.insert(
                    id,
                    SkillInfo {
                        named: Named { name: name("skills", id, r.get(1)?), icon: icon(r.get(2)?) },
                        cooldown_ms: any_int(r, 3)?.max(0) as u32,
                        tag: any_int(r, 4)?.max(0) as u32,
                    },
                );
            }
        }
        {
            let mut q = conn.prepare("SELECT id, name, icon_id, max_stack_size, level_requirement, fixed_grade, use_skill_id FROM items")?;
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
                        use_skill: r.get::<_, Option<i64>>(6)?.unwrap_or(0).max(0) as u32,
                        desc: name("items_desc", id, None),
                    },
                );
            }
        }
        {
            let mut q = conn.prepare("SELECT id, name, icon_id, desc, kind_id FROM buffs")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let id: u32 = r.get(0)?;
                data.buffs.insert(
                    id,
                    BuffInfo {
                        named: Named { name: name("buffs", id, r.get(1)?), icon: icon(r.get(2)?) },
                        desc: name("buffs_desc", id, r.get(3)?),
                        kind: r.get::<_, Option<i64>>(4)?.unwrap_or(1),
                    },
                );
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
        {
            let mut q = conn.prepare("SELECT faction1_id, faction2_id, state_id FROM system_faction_relations")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                data.relations.insert((r.get(0)?, r.get(1)?), r.get::<_, i64>(2)? as u8);
            }
        }
        {
            let mut q = conn.prepare("SELECT id, name FROM sub_zones")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let id: u32 = r.get(0)?;
                data.sub_zone_names.insert(id, name("sub_zones", id, r.get(1)?));
            }
        }
        for (group, faction) in data.zones.values() {
            if *faction != 0 {
                data.group_faction.entry(*group).or_insert(*faction);
            }
        }
        data.load_sub_zone_shapes();
        Ok(data)
    }

    /// Reads the sub zone polygons of `main_world` from the pak (about 90 small files).
    fn load_sub_zone_shapes(&mut self) {
        let pak = aa_pak::Pak::global();
        let world = "worlds/main_world";
        let Ok(world_xml) = pak.read_string(&format!("{world}/world.xml")) else { return };
        let mut origins: HashMap<u32, (f32, f32)> = HashMap::new();
        for tag in world_xml.split("<Zone ").skip(1) {
            let tag = format!(" {}", tag.split('>').next().unwrap_or(""));
            let (Some(id), Some(x), Some(y)) = (attr(&tag, "id"), attr(&tag, "originX"), attr(&tag, "originY")) else {
                continue;
            };
            if let (Ok(id), Ok(x), Ok(y)) = (id.parse(), x.parse(), y.parse()) {
                origins.insert(id, (x, y));
            }
        }
        let prefix = format!("{world}/level_design/zone/");
        let files: Vec<String> = pak.list(&prefix).filter(|p| p.ends_with("/client/subzone_area.xml")).map(str::to_string).collect();
        for file in files {
            let zone: Option<u32> = file.split("level_design/zone/").nth(1).and_then(|r| r.split('/').next()).and_then(|z| z.parse().ok());
            let Some(origin) = zone.and_then(|z| origins.get(&z)).copied() else { continue };
            if let Ok(text) = pak.read_string(&file) {
                parse_sub_zone_areas(&text, origin, &mut self.sub_zone_shapes);
            }
        }
    }

    /// The name of the sub zone at a game position: the smallest polygon around it.
    pub fn sub_zone_at(&self, x: f32, y: f32) -> Option<&str> {
        let shape = self.sub_zone_shapes.iter().filter(|s| s.contains(x, y)).min_by(|a, b| a.area().total_cmp(&b.area()))?;
        self.sub_zone_names.get(&shape.id).map(String::as_str).filter(|n| !n.is_empty())
    }

    /// A zone group by id: its data and the faction protecting it.
    pub fn zone_by_group(&self, group: u32) -> Option<(&ZoneGroup, u32)> {
        Some((self.zone_groups.get(&group)?, self.group_faction.get(&group).copied().unwrap_or(0)))
    }

    /// How a unit of faction `other` looks to a unit of faction `own`: what the unit frames and
    /// name plates colour by (`"friendly"`, `"neutral"`, `"hostile"`).
    pub fn relation(&self, own: u32, other: u32) -> &'static str {
        if own == other {
            return "friendly";
        }
        match self.relations.get(&(own, other)).or_else(|| self.relations.get(&(other, own))) {
            Some(1) => "friendly",
            Some(3) => "hostile",
            _ => "neutral",
        }
    }

    /// `npcs.npc_grade_id` as the scripts name the grade (`GetUnitGradeColor`, the target frame's
    /// ornaments). The table that names them is not in the database: 1 is most NPCs (normal),
    /// the rest follow the order of strength; a guess.
    pub fn grade_name(grade_id: i64) -> &'static str {
        match grade_id {
            2 => "strong",
            3 => "elite",
            4 => "boss_c",
            5 => "boss_b",
            8 => "boss_s",
            _ => "normal",
        }
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
