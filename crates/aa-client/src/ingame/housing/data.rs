//! The static housing data the world side needs: what a house design looks like at each build step, which items enter
//! the builder, and the housing land of the world (`housing_area.xml` polygons with their group's allowed categories).
//!
//! Loaded once on a thread of its own ([`load_in_background`]); [`get`] is `None` until it is done. The land polygons
//! are the client's own files, the same the server validates against (`systems/npcs/subzone.rs`): every
//! `worlds/main_world/level_design/zone/<key>/client/housing_area.xml` area, its points relative to the entity's
//! `Pos` and to the 1024 m cell `zone origin + (cellX, cellY)` of `world.xml`, its `value1` the `housing_areas` row.

use std::collections::HashMap;
use std::sync::OnceLock;

use bevy::log::{info, warn};
use bevy::math::Vec2;
use rusqlite::{Connection, OpenFlags};

/// A house design (`housings`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Design {
    pub id: u32,
    pub category_id: u32,
    /// Half the side of the square of land the house owns (metres).
    pub garden_radius: f32,
    /// The finished house (`models.id`).
    pub main_model: u32,
    /// `housing_build_steps` in step order.
    pub steps: Vec<Step>,
}

/// One construction stage (`housing_build_steps`) and what its build action (`skill_id`) consumes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Step {
    /// The model the house shows during this stage.
    pub model: u32,
    /// The build action: cast at the house (`SkillTargetType.Building`), consumes the item and labor.
    pub skill: u32,
    /// Build actions this stage takes.
    pub actions: i32,
    /// `skill_effects.consume_item_id` / `consume_item_count` of the skill (0: labor only).
    pub item: u32,
    pub count: i32,
    /// `skills.consume_lp`.
    pub labor: i32,
}

impl Design {
    /// The stage `actions` build actions into the construction are in (`None`: finished).
    pub fn step_at(&self, actions: i32) -> Option<usize> {
        let mut done = 0;
        for (i, s) in self.steps.iter().enumerate() {
            done += s.actions;
            if actions < done {
                return Some(i);
            }
        }
        None
    }
}

/// A housing land polygon in game coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Land {
    /// `housing_areas.id` (`value1`).
    pub area_id: u32,
    pub points: Vec<Vec2>,
    min: Vec2,
    max: Vec2,
}

impl Land {
    pub fn new(area_id: u32, points: Vec<Vec2>) -> Land {
        let min = points.iter().fold(Vec2::splat(f32::INFINITY), |a, p| a.min(*p));
        let max = points.iter().fold(Vec2::splat(f32::NEG_INFINITY), |a, p| a.max(*p));
        Land { area_id, points, min, max }
    }

    /// The server's crossing test (`subzone::is_inside`): horizontal edges never cross.
    pub fn contains(&self, p: Vec2) -> bool {
        if self.points.len() < 3 || p.cmplt(self.min).any() || p.cmpgt(self.max).any() {
            return false;
        }
        let (x, y) = (f64::from(p.x), f64::from(p.y));
        let mut inside = false;
        for (a, b) in self.points.iter().zip(self.points.iter().cycle().skip(1)) {
            let (ax, ay, bx, by) = (f64::from(a.x), f64::from(a.y), f64::from(b.x), f64::from(b.y));
            if (ay > y) != (by > y) && x < (bx - ax) * (y - ay) / (by - ay) + ax {
                inside = !inside;
            }
        }
        inside
    }
}

/// `housing_groups`: who may build there.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Group {
    /// Only for an account without a house.
    pub houseless: bool,
    /// Not for an account that has a house of this category already (0: no such rule).
    pub existing_category_id: u32,
    /// `housing_group_categories`: the designs (by category) allowed.
    pub categories: Vec<u32>,
}

#[derive(Debug, Default)]
pub struct HousingData {
    pub designs: HashMap<u32, Design>,
    /// `item_housings`: a design item -> its design.
    pub design_by_item: HashMap<u32, u32>,
    /// `item_housing_decorations` -> `housing_decorations.doodad_id`: a furniture item -> (design, doodad template).
    pub decoration_by_item: HashMap<u32, (u32, u32)>,
    /// `models.id` -> the prefab element of its first (alive) state (`prefab://file/name`).
    pub prefabs: HashMap<u32, String>,
    /// `housing_areas.id` -> `housing_groups.id`.
    pub area_group: HashMap<u32, u32>,
    pub groups: HashMap<u32, Group>,
    pub land: Vec<Land>,
}

impl HousingData {
    /// The item enters the builder (a house design or a piece of furniture).
    pub fn placeable(&self, item_template: u32) -> bool {
        self.design_by_item.contains_key(&item_template) || self.decoration_by_item.contains_key(&item_template)
    }

    /// The model a house of `design` shows at build step `step` (`-1` or past the last step: finished).
    pub fn model_at_step(&self, design: u32, step: i32) -> Option<u32> {
        let d = self.designs.get(&design)?;
        Some(usize::try_from(step).ok().and_then(|s| d.steps.get(s)).map_or(d.main_model, |s| s.model))
    }

    /// The housing groups of the land at `p`.
    pub fn groups_at(&self, p: Vec2) -> impl Iterator<Item = (u32, &Group)> {
        self.land
            .iter()
            .filter(move |l| l.contains(p))
            .filter_map(|l| self.area_group.get(&l.area_id))
            .filter_map(|g| self.groups.get(g).map(|group| (*g, group)))
    }
}

fn int(r: &rusqlite::Row, i: usize) -> rusqlite::Result<i64> {
    use rusqlite::types::ValueRef;
    Ok(match r.get_ref(i)? {
        ValueRef::Integer(n) => n,
        ValueRef::Real(f) => f as i64,
        ValueRef::Text(t) => std::str::from_utf8(t).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0),
        _ => 0,
    })
}

fn flag(r: &rusqlite::Row, i: usize) -> rusqlite::Result<bool> {
    use rusqlite::types::ValueRef;
    Ok(match r.get_ref(i)? {
        ValueRef::Integer(n) => n != 0,
        ValueRef::Text(t) => matches!(t, b"t" | b"T" | b"1" | b"true"),
        _ => false,
    })
}

fn float(r: &rusqlite::Row, i: usize) -> rusqlite::Result<f32> {
    use rusqlite::types::ValueRef;
    Ok(match r.get_ref(i)? {
        ValueRef::Integer(n) => n as f32,
        ValueRef::Real(f) => f as f32,
        ValueRef::Text(t) => std::str::from_utf8(t).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0.0),
        _ => 0.0,
    })
}

impl HousingData {
    /// The database tables (everything but the land).
    pub fn load_tables(conn: &Connection) -> rusqlite::Result<HousingData> {
        let mut d = HousingData::default();
        let mut q = conn.prepare("SELECT id, category_id, garden_radius, main_model_id FROM housings")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            let id = int(r, 0)? as u32;
            d.designs.insert(id, Design { id, category_id: int(r, 1)? as u32, garden_radius: float(r, 2)?, main_model: int(r, 3)? as u32, steps: Vec::new() });
        }
        let mut q = conn.prepare(
            "SELECT b.housing_id, b.model_id, b.skill_id, b.num_actions, s.consume_lp,
                    (SELECT e.consume_item_id FROM skill_effects e WHERE e.skill_id = b.skill_id AND e.consume_item_id > 0 ORDER BY e.id LIMIT 1),
                    (SELECT e.consume_item_count FROM skill_effects e WHERE e.skill_id = b.skill_id AND e.consume_item_id > 0 ORDER BY e.id LIMIT 1)
             FROM housing_build_steps b LEFT JOIN skills s ON s.id = b.skill_id ORDER BY b.housing_id, b.step",
        )?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            if let Some(design) = d.designs.get_mut(&(int(r, 0)? as u32)) {
                design.steps.push(Step {
                    model: int(r, 1)? as u32,
                    skill: int(r, 2)? as u32,
                    actions: int(r, 3)? as i32,
                    labor: int(r, 4)? as i32,
                    item: int(r, 5)?.max(0) as u32,
                    count: int(r, 6)? as i32,
                });
            }
        }
        let mut q = conn.prepare("SELECT item_id, design_id FROM item_housings ORDER BY id")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            d.design_by_item.entry(int(r, 0)? as u32).or_insert(int(r, 1)? as u32);
        }
        let mut q = conn.prepare(
            "SELECT i.item_id, i.design_id, h.doodad_id FROM item_housing_decorations i JOIN housing_decorations h ON h.id = i.design_id ORDER BY i.id",
        )?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            d.decoration_by_item.entry(int(r, 0)? as u32).or_insert((int(r, 1)? as u32, int(r, 2)? as u32));
        }
        let mut q = conn.prepare(
            "SELECT m.id, e.file_path FROM models m JOIN prefab_elements e ON e.prefab_model_id = m.sub_id
             WHERE m.sub_type = 'PrefabModel' AND m.id IN (SELECT main_model_id FROM housings UNION SELECT model_id FROM housing_build_steps)
             ORDER BY e.state_id DESC",
        )?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            // ordered so that the lowest state (1, alive) is inserted last and wins
            if let Some(path) = r.get::<_, Option<String>>(1)? {
                d.prefabs.insert(int(r, 0)? as u32, path);
            }
        }
        let mut q = conn.prepare("SELECT id, housing_group_id FROM housing_areas")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            d.area_group.insert(int(r, 0)? as u32, int(r, 1)? as u32);
        }
        let mut q = conn.prepare("SELECT id, houseless, existing_category_id FROM housing_groups")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            d.groups.insert(int(r, 0)? as u32, Group { houseless: flag(r, 1)?, existing_category_id: int(r, 2)?.max(0) as u32, categories: Vec::new() });
        }
        let mut q = conn.prepare("SELECT housing_group_id, category_id FROM housing_group_categories")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            if let Some(g) = d.groups.get_mut(&(int(r, 0)? as u32)) {
                g.categories.push(int(r, 1)? as u32);
            }
        }
        Ok(d)
    }
}

/// `world.xml`: `<Zone name=".." id="KEY" originX="X" originY="Y">` -> zone key -> origin cell.
pub fn zone_origins(world_xml: &str) -> HashMap<u32, (i32, i32)> {
    let mut out = HashMap::new();
    let mut rest = world_xml;
    while let Some(at) = rest.find("<Zone ") {
        rest = &rest[at + 6..];
        let tag = &rest[..rest.find('>').unwrap_or(rest.len())];
        let (Some(id), Some(x), Some(y)) = (attr(tag, "id"), attr(tag, "originX"), attr(tag, "originY")) else { continue };
        if let (Ok(id), Ok(x), Ok(y)) = (id.parse(), x.parse(), y.parse()) {
            out.insert(id, (x, y));
        }
    }
    out
}

/// The value of attribute `name` in the text of one tag.
pub fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mut from = 0;
    while let Some(at) = tag[from..].find(name) {
        let start = from + at;
        let before = tag[..start].chars().next_back();
        let after = &tag[start + name.len()..];
        if before.is_none_or(char::is_whitespace) && after.starts_with("=\"") {
            let value = &after[2..];
            return Some(&value[..value.find('"')?]);
        }
        from = start + name.len();
    }
    None
}

fn vec3(text: &str) -> Option<[f32; 3]> {
    let mut it = text.split(',').map(|v| v.trim().parse::<f32>().ok());
    Some([it.next()??, it.next()??, it.next()??])
}

/// The land polygons of one `housing_area.xml` (zone origin cell `origin`), in game coordinates.
pub fn parse_land(xml: &str, origin: (i32, i32)) -> Vec<Land> {
    let mut out = Vec::new();
    let Ok(doc) = roxmltree::Document::parse(xml) else { return out };
    for entity in doc.root_element().children().filter(|n| n.has_tag_name("Entity")) {
        let start = entity.attribute("Pos").and_then(vec3).unwrap_or_default();
        let cell = |name| entity.attribute(name).and_then(|v| v.trim().parse::<i32>().ok()).unwrap_or(0);
        let base = Vec2::new(
            (origin.0 + cell("cellX")) as f32 * 1024.0 + start[0],
            (origin.1 + cell("cellY")) as f32 * 1024.0 + start[1],
        );
        for area in entity.children().filter(|n| n.has_tag_name("Area")) {
            let Some(id) = area.attribute("value1").or(area.attribute("Id")).and_then(|v| v.trim().parse().ok()) else { continue };
            let points: Vec<Vec2> = area
                .descendants()
                .filter(|n| n.has_tag_name("Point"))
                .filter_map(|p| p.attribute("Pos").and_then(vec3))
                .map(|p| base + Vec2::new(p[0], p[1]))
                .collect();
            if points.len() >= 3 {
                out.push(Land::new(id, points));
            }
        }
    }
    out
}

/// Every housing land polygon of world `world` from the pak.
fn load_land(world: &str) -> Vec<Land> {
    let pak = aa_pak::Pak::global();
    let Ok(xml) = pak.read_string(&format!("worlds/{world}/world.xml")) else {
        warn!("housing: no worlds/{world}/world.xml: no housing land");
        return Vec::new();
    };
    let origins = zone_origins(&xml);
    let prefix = format!("worlds/{world}/level_design/zone/");
    let files: Vec<String> = pak.list(&prefix).filter(|n| n.to_ascii_lowercase().ends_with("/client/housing_area.xml")).map(str::to_string).collect();
    let mut land = Vec::new();
    for file in files {
        let normal = aa_pak::normalize(&file);
        let key = normal.split("/zone/").nth(1).and_then(|r| r.split('/').next()).and_then(|k| k.parse::<u32>().ok());
        let (Some(key), Ok(text)) = (key, pak.read_string(&file)) else { continue };
        land.extend(parse_land(&text, origins.get(&key).copied().unwrap_or((0, 0))));
    }
    land
}

static DATA: OnceLock<HousingData> = OnceLock::new();

/// The housing data, once [`load_in_background`] is done.
pub fn get() -> Option<&'static HousingData> {
    DATA.get()
}

/// Loads the tables and the land of `world` on a thread of its own (once).
pub fn load_in_background(world: &'static str) {
    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.set(()).is_err() {
        return;
    }
    let spawned = std::thread::Builder::new().name("housing-data".into()).spawn(move || {
        let t = std::time::Instant::now();
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let tables = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
            .and_then(|conn| HousingData::load_tables(&conn));
        let mut data = match tables {
            Ok(d) => d,
            Err(e) => {
                warn!("housing: database {path}: {e}");
                HousingData::default()
            }
        };
        data.land = load_land(world);
        info!(
            "housing: {} designs, {} design items, {} furniture items, {} land polygons in {:.0} ms",
            data.designs.len(),
            data.design_by_item.len(),
            data.decoration_by_item.len(),
            data.land.len(),
            t.elapsed().as_secs_f32() * 1000.0
        );
        let _ = DATA.set(data);
    });
    if let Err(e) = spawned {
        warn!("housing: cannot start the loader: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn land_points_are_relative_to_the_entity_and_the_zone_cell() {
        let xml = r#"<Objects>
 <Entity Name="terrorhousing1" Pos="232.5,782.5,103.7" EntityClass="AreaShape" cellX="1" cellY="0">
  <Area Id="0" value1="212" Height="0"><Points>
    <Point Pos="0,0,0"/><Point Pos="10,0,0"/><Point Pos="10,10,0"/><Point Pos="0,10,0"/>
  </Points></Area>
 </Entity>
</Objects>"#;
        let land = parse_land(xml, (14, 9));
        assert_eq!(land.len(), 1);
        assert_eq!(land[0].area_id, 212);
        assert_eq!(land[0].points[0], Vec2::new(15.0 * 1024.0 + 232.5, 9.0 * 1024.0 + 782.5));
        assert!(land[0].contains(Vec2::new(15.0 * 1024.0 + 237.5, 9.0 * 1024.0 + 787.5)));
        assert!(!land[0].contains(Vec2::new(15.0 * 1024.0 + 243.5, 9.0 * 1024.0 + 787.5)));
    }

    #[test]
    fn zone_origins_come_from_world_xml() {
        let o = zone_origins(r#"<ZoneList><Zone name="w_gweonid_forest_1" id="129" originX="9" originY="14"><cellList/></Zone>
            <Zone name="x" id="133" originX="-2" originY="10"></ZoneList>"#);
        assert_eq!(o.get(&129), Some(&(9, 14)));
        assert_eq!(o.get(&133), Some(&(-2, 10)));
        assert_eq!(attr(r#"a Pos="1" xPos="2""#, "Pos"), Some("1"));
        assert_eq!(attr(r#"a xPos="2" Pos="1""#, "Pos"), Some("1"));
    }

    #[test]
    fn the_database_has_the_designs_their_steps_and_the_land_groups() {
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let Ok(conn) = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY) else { return };
        let d = HousingData::load_tables(&conn).unwrap();
        // the small Nuian house: three construction stages, then model 1086 (as the server's set_current_step)
        let design = &d.designs[&172];
        assert_eq!(design.category_id, 1);
        assert_eq!(design.steps.iter().map(|s| s.model).collect::<Vec<_>>(), vec![1083, 1084, 1085]);
        // stone, stone, lumber: one item, 25 labor and one action each
        assert_eq!((design.steps[0].skill, design.steps[0].item, design.steps[0].count, design.steps[0].labor), (14575, 17684, 1, 25));
        assert_eq!(design.steps[2].item, 17683);
        let farm = &d.designs[&171];
        assert_eq!(farm.steps.iter().map(|s| s.actions).collect::<Vec<_>>(), vec![10, 5, 5]);
        assert_eq!((farm.step_at(0), farm.step_at(9), farm.step_at(10), farm.step_at(19), farm.step_at(20)), (Some(0), Some(0), Some(1), Some(2), None));
        assert_eq!(d.model_at_step(172, 0), Some(1083));
        assert_eq!(d.model_at_step(172, -1), Some(design.main_model));
        assert_eq!(d.design_by_item.get(&21166), Some(&172));
        assert!(d.placeable(21166) && !d.placeable(1));
        assert!(d.prefabs[&50].starts_with("prefab://Prefabs/housing_house.xml/"));
        assert!(d.groups[&15].categories.contains(&18));
    }
}
