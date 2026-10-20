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

use super::quests::QuestTables;
use super::stats::StatTables;

mod recharge;
pub(crate) use recharge::item_recharge_catalog;

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
    /// `item_groups.id` the item's category belongs to (the bag tabs filter by it), 0 when unknown.
    pub group: u32,
    /// `item_categories.category_order`, the native inventory-view sort key.
    pub category_order: i64,
    /// The item category's English name ("Book"), for the tooltip.
    pub category: String,
    /// `items.impl_id` (`ItemImplEnum`): the tooltip's `item_impl` ("portal", "weapon", ...).
    pub impl_id: i64,
    pub sellable: bool,
    pub price: i64,
    pub refund: i64,
    pub honor_price: i64,
    pub living_price: i64,
    pub level_limit: i64,
    /// Native ItemDesc tag29 membership, independent of display category.
    pub pet_only: bool,
}

#[derive(Clone, Debug, Default)]
pub struct SkillInfo {
    pub named: Named,
    /// `skills.cooldown_time` (ms) and `cooldown_tag_id`.
    pub cooldown_ms: u32,
    pub tag: u32,
    /// All `tagged_skills` rows in native SQL order, including duplicates.
    pub tags: Vec<u32>,
    /// The global cooldown casting it starts (ms): `custom_gcd`, or one second for `default_gcd`.
    pub gcd_ms: u32,
    /// `skills.ignore_global_cooldown`: the global cooldown does not hold it back.
    pub ignore_gcd: bool,
    /// `skills.ability_id` (0 general, 1 fight ... 10 love: the tooltip's ability colour and name).
    pub ability: u8,
    pub mana: i64,
    /// Casting and channeling time (ms), range (m): the tooltip's lines.
    pub casting_ms: i64,
    pub channeling_ms: i64,
    pub min_range: i64,
    pub max_range: i64,
    /// `skills.desc` with the placeholders filled in as far as the database allows.
    pub desc: String,
    /// `skills.weapon_slot_for_autoattack_id` is set: the auto attack (the combat text prints its hits white).
    pub auto_attack: bool,
}

/// A buff: name, icon, description and its list (`buffs.kind_id`: 1 good, 2 bad, 3 hidden).
#[derive(Clone, Debug, Default)]
pub struct BuffInfo {
    pub named: Named,
    pub desc: String,
    pub kind: i64,
    /// Native descriptor+0x1a5: prevents manual cancellation with error224.
    pub system: bool,
    pub stack_rule: u32,
}

/// A title (`appellations`): its name and the buff it gives.
#[derive(Clone, Debug, Default)]
pub struct AppellationInfo {
    pub name: String,
    pub buff_id: u32,
}

#[derive(Clone, Debug, Default)]
pub struct NpcInfo {
    pub name: String,
    #[allow(dead_code)] // for the quest markers and the level text of the target frame
    pub level: i64,
    /// `npcs.npc_grade_id`, named by [`GameData::grade_name`].
    pub grade: i64,
    /// `npcs.npc_kind_id`.
    pub kind: i64,
    /// `npcs.npc_template_id`: with the grade and kind the key of the HP formula's variables.
    pub template: i64,
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
    pub(super) expedition: super::expedition::Tables,
    pub(super) achievements: super::achievement::Tables,
    pub(super) ranking: super::ranking::Tables,
    pub skills: HashMap<u32, SkillInfo>,
    pub items: HashMap<u32, ItemInfo>,
    pub buffs: HashMap<u32, BuffInfo>,
    /// `appellations`: the titles by id.
    pub appellations: HashMap<u32, AppellationInfo>,
    pub npcs: HashMap<u32, NpcInfo>,
    /// `zones.zone_key` -> (zone group id, protecting faction id).
    pub zones: HashMap<u32, (u32, u32)>,
    /// `zone_groups.id` -> name, map rectangle.
    pub zone_groups: HashMap<u32, ZoneGroup>,
    /// Item grades by the number the server and the icons use (`item_grades.grade_order`): English name and
    /// colour (`FFBA976D`).
    pub grades: HashMap<i64, (String, String)>,
    /// `world_groups.id` -> map rectangle: 2 the world, 3 / 4 / 5 the western / eastern / origin continent.
    pub world_rects: HashMap<u32, [f32; 4]>,
    /// `system_factions.id` -> English name.
    pub factions: HashMap<u32, String>,
    /// `system_factions.mother_id` (0: none): the faction a faction belongs to; the root factions 1 (friendly to
    /// all), 2 (neutral) and 3 (hostile) decide how everything under them looks to everyone.
    pub faction_mother: HashMap<u32, u32>,
    /// `system_faction_relations`: (faction, other faction) -> 1 hostile, 2 neutral, 3 friendly.
    pub relations: HashMap<(u32, u32), u8>,
    /// `sub_zones.id` -> English name.
    pub sub_zone_names: HashMap<u32, String>,
    /// `sub_zones.id` -> `parent_sub_zone_id` (a city's districts have the city as parent).
    pub sub_zone_parent: HashMap<u32, u32>,
    /// `sub_zones.id` -> rectangle (west, south, width, height) of the cities, caves and mines.
    pub sub_zone_rects: HashMap<u32, [f32; 4]>,
    /// The sub zone polygons of the world (the database rectangles are mostly empty).
    pub sub_zone_shapes: Vec<SubZoneShape>,
    /// `zones.group_id` -> the faction protecting the group's zones (0 = none).
    pub group_faction: HashMap<u32, u32>,
    /// The formulas behind maximum HP / MP.
    pub stats: StatTables,
    /// Which NPCs start and take in which quests.
    pub quests: QuestTables,
    /// What the tooltips fill in: the numbers of skill and buff descriptions, the stats of weapons and armor.
    pub tips: super::tipcalc::TipTables,
    /// When a skill can be used (mana, range, target, requirements, buff restrictions): `usable.rs`.
    pub rules: super::usable::Rules,
    /// Ability trees, passive buffs and the skill points the player may spend.
    pub book: super::skillbook::Book,
    pub stores: super::store::Tables,
    pub mail: super::mail::Tables,
    /// What each doodad phase can be used for, doodad names (`doodad.rs`).
    pub(super) doodad: super::doodad::Tables,
    pub expansions: Vec<aa_net::bank::ExpansionRule>,
    pub(super) repair: super::repair::Tables,
    pub(super) crafting: super::crafting::Tables,
    pub(super) pets: super::pets::Tables,
    pub(super) radar: super::radar::Tables,
    pub(super) battlefield: super::battlefield::Tables,
    pub(super) native_buffs: HashMap<u32,aa_net::state::NativeBuffRule>,
    pub(super) enchant: super::enchant::Tables,
    pub(super) specialties: super::specialty::Tables,
    /// The conversations of the `bubbles` rows (`speech.rs`).
    pub(super) speech: super::speech::Tables,
}

/// The server's experience table (`levels.total_exp`, levels 1..=55), which replaces the client's
/// hardcoded 1.2 curve in aa-net (51-55 are the 3.0 retail values on a patched server).
pub(crate) fn load_exp_table(path: &str) -> Result<aa_net::model::exp::ExpTable, String> {
    use aa_net::model::exp::{ExpTable, LEVELS};
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(|e| format!("open: {e}"))?;
    let mut query = conn
        .prepare("SELECT id, total_exp FROM levels WHERE id BETWEEN 1 AND 55 ORDER BY id")
        .map_err(|e| format!("levels: {e}"))?;
    let rows = query
        .query_map([], |r| Ok((any_int(r, 0)?, any_int(r, 1)?)))
        .map_err(|e| format!("levels: {e}"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| format!("levels: {e}"))?;
    if rows.len() != LEVELS {
        return Err(format!("levels: {} of {LEVELS} rows", rows.len()));
    }
    let mut totals = [0i64; LEVELS];
    for (index, (id, total)) in rows.into_iter().enumerate() {
        if id != index as i64 + 1 {
            return Err(format!("levels: row {} has id {id}", index + 1));
        }
        totals[index] = total;
    }
    Ok(ExpTable::from_totals(totals))
}

/// Installs [`load_exp_table`]'s table into aa-net once, at startup. Without the database (or with a
/// broken one) aa-net keeps its hardcoded 1.2 fallback.
pub(crate) fn install_exp_table() {
    let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
    match load_exp_table(&path) {
        Ok(table) => {
            let top = table.total_for_level(aa_net::model::exp::MAX_LEVEL);
            if aa_net::model::exp::install(table) {
                info!("hud: experience table from {path} (level 55 total {top})");
            }
        }
        Err(e) => warn!("hud: experience table {path}: {e}; using the 1.2 table"),
    }
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
pub(super) fn any_int(r: &rusqlite::Row, i: usize) -> rusqlite::Result<i64> {
    use rusqlite::types::Value;
    Ok(match r.get::<_, Value>(i)? {
        Value::Integer(n) => n,
        Value::Real(f) => f as i64,
        Value::Text(t) => t.trim().parse().unwrap_or(0),
        _ => 0,
    })
}

/// A description with the client's `#{name}` placeholders filled in by `values`; the ones the database
/// cannot work out (damage ranges: they depend on the player's gear) become `?`. CRLF becomes LF.
fn fill_placeholders(text: &str, values: impl Fn(&str) -> Option<String>) -> String {
    let text = text.replace("\r\n", "\n");
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while let Some(at) = rest.find("#{") {
        out.push_str(&rest[..at]);
        let after = &rest[at + 2..];
        let Some(end) = after.find('}') else {
            rest = &rest[at..];
            break;
        };
        out.push_str(&values(&after[..end]).unwrap_or_else(|| "?".into()));
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// A boolean column the database stores as `'t'` / `'f'` text or as a number.
pub(super) fn any_flag(r: &rusqlite::Row, i: usize) -> rusqlite::Result<bool> {
    use rusqlite::types::Value;
    Ok(match r.get::<_, Value>(i)? {
        Value::Integer(n) => n != 0,
        Value::Text(t) => matches!(t.trim(), "t" | "T" | "1" | "true"),
        _ => false,
    })
}

/// The global cooldown of a skill with `default_gcd` (the server: 1000 ms for a player, 1500 for an NPC).
pub const DEFAULT_GCD_MS: u32 = 1000;

impl GameData {
    pub fn load(path: &str) -> rusqlite::Result<GameData> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
        // English texts: (table, column) -> id -> text
        let mut text: HashMap<(&'static str, u32), String> = HashMap::new();
        {
            let mut q = conn.prepare(
                "SELECT tbl_name, idx, en_us, tbl_column_name FROM localized_texts WHERE en_us IS NOT NULL AND en_us <> '' AND (\
                 (tbl_name IN ('skills','items','buffs','npcs','system_factions','sub_zones','item_categories','item_grades','appellations') AND tbl_column_name = 'name') OR \
                 (tbl_name = 'buffs' AND tbl_column_name = 'desc') OR (tbl_name = 'skills' AND tbl_column_name = 'desc') OR                  (tbl_name = 'items' AND tbl_column_name = 'description') OR \
                 (tbl_name = 'zone_groups' AND tbl_column_name = 'display_text'))",
            )?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let table: String = r.get(0)?;
                let idx: i64 = r.get(1)?;
                let column: String = r.get(3)?;
                let table = match table.as_str() {
                    "buffs" if column == "desc" => "buffs_desc",
                    "skills" if column == "desc" => "skills_desc",
                    "item_categories" => "item_categories",
                    "item_grades" => "item_grades",
                    "items" if column == "description" => "items_desc",
                    "skills" => "skills",
                    "items" => "items",
                    "buffs" => "buffs",
                    "npcs" => "npcs",
                    "appellations" => "appellations",
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
            // `#{damage_multiplier}` of a description: the damage effect's DPS multiplier in percent
            let mut multipliers: HashMap<u32, i64> = HashMap::new();
            let wanted: Vec<String> = text.iter().filter(|((t, _), d)| *t == "skills_desc" && d.contains("#{damage_multiplier}")).map(|((_, id), _)| id.to_string()).collect();
            if !wanted.is_empty() {
                let mut q = conn.prepare(&format!(
                    "SELECT se.skill_id, d.dps_multiplier FROM skill_effects se JOIN effects e ON e.id = se.effect_id AND e.actual_type = 'DamageEffect' \
                     JOIN damage_effects d ON d.id = e.actual_id WHERE se.skill_id IN ({}) ORDER BY se.id DESC",
                    wanted.join(",")
                ))?;
                let mut rows = q.query([])?;
                while let Some(r) = rows.next()? {
                    multipliers.insert(r.get(0)?, (r.get::<_, Option<f64>>(1)?.unwrap_or(1.0) * 100.0).round() as i64);
                }
            }
            let mut q = conn.prepare("SELECT id, name, icon_id, cooldown_time, cooldown_tag_id, custom_gcd, default_gcd, ignore_global_cooldown, ability_id, mana_cost, casting_time, channeling_time, min_range, max_range, weapon_slot_for_autoattack_id FROM skills")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let id: u32 = r.get(0)?;
                data.skills.insert(
                    id,
                    SkillInfo {
                        named: Named { name: name("skills", id, r.get(1)?), icon: icon(r.get(2)?) },
                        cooldown_ms: any_int(r, 3)?.max(0) as u32,
                        tag: any_int(r, 4)?.max(0) as u32,
                        tags: Vec::new(),
                        gcd_ms: if any_flag(r, 6)? { DEFAULT_GCD_MS } else { any_int(r, 5)?.max(0) as u32 },
                        ignore_gcd: any_flag(r, 7)?,
                        ability: any_int(r, 8)?.clamp(0, 10) as u8,
                        mana: any_int(r, 9)?.max(0),
                        casting_ms: any_int(r, 10)?.max(0),
                        channeling_ms: any_int(r, 11)?.max(0),
                        min_range: any_int(r, 12)?.max(0),
                        max_range: any_int(r, 13)?.max(0),
                        auto_attack: any_int(r, 14)? > 0,
                        desc: fill_placeholders(&name("skills_desc", id, None), |key| if key == "damage_multiplier" { multipliers.get(&id).map(|m| m.to_string()) } else { None }),
                    },
                );
            }
        }
        {
            // Native LoadSkillTagsRelation395d15a0 appends every row, not just cooldown_tag_id.
            let mut q = conn.prepare("SELECT skill_id, tag_id FROM tagged_skills")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                if let Some(skill) = data.skills.get_mut(&r.get::<_, u32>(0)?) {
                    skill.tags.push(r.get(1)?);
                }
            }
        }
        {
            let mut q = conn.prepare("SELECT i.id, i.name, i.icon_id, i.max_stack_size, i.level_requirement, i.fixed_grade, i.use_skill_id, c.item_group_id, i.category_id, i.bind_id, i.sellable, i.price, i.refund, i.level_limit, i.impl_id, i.honor_price, i.living_point_price, c.category_order FROM items i LEFT JOIN item_categories c ON c.id = i.category_id")?;
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
                        desc: fill_placeholders(&name("items_desc", id, None), |_| None),
                        group: r.get::<_, Option<i64>>(7)?.unwrap_or(0).max(0) as u32,
                        category_order: any_int(r, 17)?,
                        category: name("item_categories", any_int(r, 8)?.max(0) as u32, None),
                        impl_id: any_int(r, 14)?,
                        sellable: any_flag(r, 10)?,
                        price: any_int(r, 11)?,
                        refund: any_int(r, 12)?,
                        honor_price: any_int(r, 15)?,
                        living_price: any_int(r, 16)?,
                        level_limit: any_int(r, 13)?,
                        pet_only: false,
                    },
                );
            }
        }
        {
            let mut q = conn.prepare("SELECT item_id FROM tagged_items WHERE tag_id=29")?;
            let mut rows = q.query([])?;
            while let Some(row) = rows.next()? {
                if let Some(item) = data.items.get_mut(&row.get::<_, u32>(0)?) {
                    item.pet_only = true;
                }
            }
        }
        {
            let mut q = conn.prepare("SELECT id, name, icon_id, desc, kind_id, system, stack_rule_id FROM buffs")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let id: u32 = r.get(0)?;
                data.buffs.insert(
                    id,
                    BuffInfo {
                        named: Named { name: name("buffs", id, r.get(1)?), icon: icon(r.get(2)?) },
                        desc: fill_placeholders(&name("buffs_desc", id, r.get(3)?), |_| None),
                        kind: r.get::<_, Option<i64>>(4)?.unwrap_or(1),
                        system: any_flag(r, 5)?,
                        stack_rule: any_int(r, 6)? as u32,
                    },
                );
            }
        }
        {
            let mut q = conn.prepare("SELECT id, name, buff_id FROM appellations")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let id: u32 = r.get(0)?;
                data.appellations.insert(id, AppellationInfo { name: name("appellations", id, r.get(1)?), buff_id: any_int(r, 2)?.max(0) as u32 });
            }
        }
        {
            let mut q = conn.prepare("SELECT id, name, level, npc_grade_id, npc_kind_id, faction_id, show_name_tag, npc_template_id FROM npcs")?;
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
                        template: r.get::<_, Option<i64>>(7)?.unwrap_or(0),
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
            let mut q = conn.prepare("SELECT id, grade_order, color_argb FROM item_grades")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let id: u32 = r.get(0)?;
                data.grades.insert(any_int(r, 1)?, (name("item_grades", id, None), r.get::<_, Option<String>>(2)?.unwrap_or_default()));
            }
        }
        {
            let mut q = conn.prepare("SELECT id, x, y, w, h FROM world_groups WHERE w > 0")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let f = |i: usize| -> rusqlite::Result<f32> { Ok(r.get::<_, Option<f64>>(i)?.unwrap_or(0.0) as f32) };
                data.world_rects.insert(r.get(0)?, [f(1)?, f(2)?, f(3)?, f(4)?]);
            }
        }
        {
            let mut q = conn.prepare("SELECT id, name, mother_id FROM system_factions")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let id: u32 = r.get(0)?;
                data.factions.insert(id, name("system_factions", id, r.get(1)?));
                data.faction_mother.insert(id, any_int(r, 2)?.max(0) as u32);
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
            let mut q = conn.prepare("SELECT id, name, parent_sub_zone_id, x, y, w, h FROM sub_zones")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let id: u32 = r.get(0)?;
                data.sub_zone_names.insert(id, name("sub_zones", id, r.get(1)?));
                let parent = r.get::<_, Option<i64>>(2)?.unwrap_or(0).max(0) as u32;
                if parent != 0 {
                    data.sub_zone_parent.insert(id, parent);
                }
                let f = |i: usize| r.get::<_, Option<f64>>(i).map(|v| v.unwrap_or(0.0) as f32);
                let rect = [f(3)?, f(4)?, f(5)?, f(6)?];
                // the cities, caves and mines (the client has a road map of): the rest has no rectangle
                if rect[2] > 0.0 && rect[3] > 0.0 {
                    data.sub_zone_rects.insert(id, rect);
                }
            }
        }
        for (group, faction) in data.zones.values() {
            if *faction != 0 {
                data.group_faction.entry(*group).or_insert(*faction);
            }
        }
        data.stats = StatTables::load(&conn)?;
        data.quests = QuestTables::load(&conn)?;
        data.tips = super::tipcalc::TipTables::load(&conn, &text)?;
        data.rules = super::usable::Rules::load(&conn)?;
        data.book = super::skillbook::Book::load(&conn)?;
        data.stores = super::store::Tables::load(&conn)?;
        data.mail = super::mail::Tables::load(&conn)?;
        data.doodad = super::doodad::Tables::load(&conn)?;
        data.expansions = super::expansion::load(&conn)?;
        data.repair = super::repair::Tables::load(&conn)?;
        data.crafting = super::crafting::Tables::load(&conn)?;
        data.pets = super::pets::Tables::load(&conn)?;
        data.radar = super::radar::Tables::load(&conn)?;
        data.battlefield = super::battlefield::Tables::load(&conn)?;
        data.expedition = super::expedition::Tables::load(&conn)?;
        data.achievements = super::achievement::Tables::load(&conn)?;
        data.ranking = super::ranking::Tables::load(&conn)?;
        data.native_buffs = super::buffs::load(&conn)?;
        data.enchant = super::enchant::Tables::load(&conn)?;
        data.specialties = super::specialty::Tables::load(&conn)?;
        data.speech = super::speech::Tables::load(&conn).unwrap_or_default();
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

    /// The sub zone at a game position: the smallest polygon around it, then up to the top of its
    /// parents. A city is a sub zone ("Ezna") with its districts and landmarks ("Royal Palace") drawn
    /// inside it as sub zones of their own; the zone text of the HUD and the map are the city's.
    /// The sub zones the point is in, the smallest polygon first, then its parents up to the root (`CSNotifySubZone`
    /// reports them: the server's recall points are on leaves as well as on roots).
    pub fn sub_zone_chain_at(&self, x: f32, y: f32) -> Vec<u32> {
        let Some(shape) = self.sub_zone_shapes.iter().filter(|s| s.contains(x, y)).min_by(|a, b| a.area().total_cmp(&b.area())) else {
            return Vec::new();
        };
        let mut chain = vec![shape.id];
        for _ in 0..8 {
            match self.sub_zone_parent.get(chain.last().unwrap()) {
                Some(p) if *p != 0 && !chain.contains(p) => chain.push(*p),
                _ => break,
            }
        }
        chain
    }

    pub fn sub_zone_root_at(&self, x: f32, y: f32) -> Option<u32> {
        let shape = self.sub_zone_shapes.iter().filter(|s| s.contains(x, y)).min_by(|a, b| a.area().total_cmp(&b.area()))?;
        let mut id = shape.id;
        for _ in 0..8 {
            match self.sub_zone_parent.get(&id) {
                Some(p) if *p != id => id = *p,
                _ => break,
            }
        }
        Some(id)
    }

    /// A zone group by id: its data and the faction protecting it.
    pub fn zone_by_group(&self, group: u32) -> Option<(&ZoneGroup, u32)> {
        Some((self.zone_groups.get(&group)?, self.group_faction.get(&group).copied().unwrap_or(0)))
    }

    /// How a unit of faction `other` looks to a unit of faction `own`: what the unit frames and
    /// name plates colour by (`"friendly"`, `"neutral"`, `"hostile"`).
    ///
    /// The server's rule (`SystemFaction.GetRelationState`): a faction under a root faction (`mother_id` 1, 2 or 3)
    /// is friendly, neutral or hostile to everyone; otherwise the same group is friendly, and the stored
    /// relation of the mother faction (or of the faction) decides: state 1 hostile, 2 neutral, 3 friendly.
    pub fn relation(&self, own: u32, other: u32) -> &'static str {
        let (Some(_), Some(_)) = (self.factions.get(&own), self.factions.get(&other)) else { return "neutral" };
        let mother = |f: u32| self.faction_mother.get(&f).copied().unwrap_or(0);
        let group = |f: u32| if mother(f) != 0 { mother(f) } else { f };
        let (mine, theirs) = (group(own), group(other));
        for root in [mine, theirs] {
            match root {
                1 => return "friendly",
                2 => return "neutral",
                3 => return "hostile",
                _ => {}
            }
        }
        if mine == theirs {
            return "friendly";
        }
        let stored = |a: u32, b: u32| self.relations.get(&(a, b)).or_else(|| self.relations.get(&(b, a))).copied();
        let state = if mother(own) != 0 && self.factions.contains_key(&mother(own)) { stored(mother(own), theirs).or(Some(3)) } else { stored(own, theirs) };
        match state {
            Some(1) => "hostile",
            Some(3) => "friendly",
            _ => "neutral",
        }
    }

    /// `npcs.npc_grade_id` as the scripts name the grade (`UNIT_GRADE_TEXT`, `GetUnitGradeColor`, the
    /// target frame's ornaments). The server database has no `npc_grades` table and the client keeps
    /// its own in the encrypted pak database, so the order is derived:
    ///
    /// * the scripts know exactly eight grades (`locale_helper.alb`'s `UNIT_GRADE_TEXT`: weak, normal,
    ///   strong, elite, boss_c, boss_b, boss_a, boss_s, weakest to strongest) and the NPCs use the ids 1 to 8;
    /// * every `npc_grade` variable of `unit_formula_variables` (HP, mana, kill experience: the
    ///   multipliers of a grade) puts the ids in the same order, from the weakest: 6, 1, 7, 2, 5, 4, 3, 8
    ///   (HP of the NPC formula: 0.3, 1, 2, 3, 15, 25, 50, 100; kill experience 0.15, 1, 2, 4, 5, 10, 15, 20).
    ///   Id 1 is the 11,483 ordinary NPCs, 8 the five world bosses (the Red Dragon).
    pub fn grade_name(grade_id: i64) -> &'static str {
        match grade_id {
            6 => "weak",
            7 => "strong",
            2 => "elite",
            5 => "boss_c",
            4 => "boss_b",
            3 => "boss_a",
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
    pub instance_id: u32,
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
    /// The labor changes the server announced (`PlayerStats::labor_log`: `(amount, total)`, the last few) and
    /// how many there were: a difference of the count between two snapshots says which are new.
    pub labor_log: Vec<(i32, i32)>,
    pub labor_seq: u32,
    pub money: i64,
    pub bank_money: i64,
    pub bm_point: i64,
    pub honor: i32,
    /// The player unit's honor gained in combat and hostile faction kills.
    pub pvp_honor: i32,
    pub pvp_kills: i32,
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
    /// The top of the sub zone the player is in (`sub_zones.id`, 0 for none) when it has a rectangle (a
    /// city, a cave: the road map is its own), and the rectangle.
    pub sub_map: u32,
    pub sub_map_rect: [f32; 4],
    /// The player's faction name.
    pub faction: String,
    /// The active abilities (archetype ids 1..=10) in slot order, and the experience of every ability (index =
    /// id, 0 is the general one): what `X2Ability` and the character window's title ("Archery Novice") answer from.
    pub abilities: Vec<u8>,
    pub ability_exp: [i32; 11],
    /// Game clock: hour 0..24 and minute.
    pub hour: u8,
    pub minute: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exp_table_matches_the_game_database() {
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        if !std::path::Path::new(&path).exists() {
            return;
        }
        let table = load_exp_table(&path).expect("levels table");
        let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let mut query = conn.prepare("SELECT total_exp FROM levels ORDER BY id LIMIT 55").unwrap();
        let rows = query
            .query_map([], |r| any_int(r, 0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(table.totals().as_slice(), rows.as_slice());
    }
}
