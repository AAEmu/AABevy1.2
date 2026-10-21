//! Static housing and public-farm tables from `compact.sqlite3`.
//!
//! Loaded once with the rest of [`super::GameData`]: `housings`, `item_housings`,
//! `housing_build_steps`, `housing_decorations`, `item_housing_decorations`,
//! `housing_areas`, `housing_groups`, `item_spawn_doodads`, `farm_groups`,
//! `farm_group_doodads` and `common_farms`. The loaders mirror the AAEmu server
//! (`systems/housing/data.rs`, `systems/npcs/public_farm.rs`): orphan build steps and
//! unknown groups are skipped, and `localized_texts` replaces the `housings.name` text.

use std::collections::HashMap;

use rusqlite::Connection;

use super::{any_flag, any_int};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ItemSpawnDoodad {
    pub id: u32,
    pub item_id: u32,
    pub doodad_id: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ItemHousing {
    pub id: u32,
    pub item_id: u32,
    pub design_id: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ItemHousingDecoration {
    pub id: u32,
    pub item_id: u32,
    pub design_id: u32,
    pub restore: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HousingBuildStep {
    pub id: u32,
    pub housing_id: u32,
    pub step: i16,
    pub model_id: u32,
    pub skill_id: u32,
    pub num_actions: i32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HousingTemplate {
    pub id: u32,
    pub name: String,
    pub category_id: u32,
    pub main_model_id: u32,
    pub door_model_id: u32,
    pub stair_model_id: u32,
    pub hp: i32,
    pub repair_cost: u32,
    pub garden_radius: f32,
    pub family: String,
    pub taxation_id: u32,
    /// `taxations.tax` of `taxation_id` (0 when the row is missing).
    pub tax: u32,
    pub deco_limit: u32,
    pub absolute_deco_limit: u32,
    pub housing_deco_limit_id: u32,
    pub is_sellable: bool,
    pub heavy_tax: bool,
    pub always_public: bool,
    pub build_steps: Vec<HousingBuildStep>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HousingDecoration {
    pub id: u32,
    pub name: String,
    pub allow_on_floor: bool,
    pub allow_on_wall: bool,
    pub allow_on_ceiling: bool,
    pub doodad_id: u32,
    pub allow_pivot_on_garden: bool,
    pub actability_group_id: u32,
    pub actability_up: u32,
    pub deco_actability_group_id: u32,
    pub allow_mesh_on_garden: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HousingArea {
    pub id: u32,
    pub housing_group_id: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HousingGroup {
    pub id: u32,
    pub houseless: bool,
    pub existing_category_id: u32,
    pub can_extend: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FarmGroup {
    pub id: u32,
    pub name: String,
    pub count: u32,
    pub description: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FarmGroupDoodad {
    pub id: u32,
    pub farm_group_id: u32,
    pub doodad_id: u32,
    pub item_id: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommonFarm {
    pub id: u32,
    pub name: String,
    pub guard_time: u32,
    pub farm_group_id: u32,
    pub comments: String,
}

/// The housing and farm tables the HUD resolves static data from.
#[derive(Clone, Debug, Default)]
pub struct Tables {
    pub item_spawn_doodads: Vec<ItemSpawnDoodad>,
    /// `item_spawn_doodads.item_id` -> `doodad_id` (one row per item in the 1.2 data).
    pub doodad_by_item: HashMap<u32, u32>,
    pub item_housings: Vec<ItemHousing>,
    /// `item_housings.item_id` -> design (`housings.id`).
    pub housing_design_by_item: HashMap<u32, u32>,
    pub housings: HashMap<u32, HousingTemplate>,
    pub decorations: HashMap<u32, HousingDecoration>,
    /// `housing_decorations.doodad_id` -> first decoration row, as the server's `FirstOrDefault`.
    pub decoration_by_doodad: HashMap<u32, u32>,
    pub item_housing_decorations: Vec<ItemHousingDecoration>,
    /// `item_housing_decorations.item_id` -> decoration design id.
    pub decoration_design_by_item: HashMap<u32, u32>,
    pub housing_areas: HashMap<u32, HousingArea>,
    pub housing_groups: HashMap<u32, HousingGroup>,
    pub farm_groups: HashMap<u32, FarmGroup>,
    pub farm_group_doodads: Vec<FarmGroupDoodad>,
    pub common_farms: Vec<CommonFarm>,
}

#[allow(dead_code)]
impl Tables {
    pub fn load(conn: &Connection) -> rusqlite::Result<Self> {
        let mut t = Tables::default();

        let mut taxes: HashMap<u32, u32> = HashMap::new();
        {
            let mut q = conn.prepare("SELECT id, tax FROM taxations")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                taxes.insert(r.get(0)?, any_int(r, 1)?.max(0) as u32);
            }
        }

        let mut names: HashMap<u32, String> = HashMap::new();
        {
            let mut q = conn.prepare(
                "SELECT idx, en_us FROM localized_texts \
                 WHERE tbl_name = 'housings' AND tbl_column_name = 'name' AND en_us IS NOT NULL AND en_us <> ''",
            )?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                names.insert(r.get(0)?, r.get(1)?);
            }
        }

        {
            let mut q = conn.prepare("SELECT * FROM housings")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let id = any_int(r, 0)?.max(0) as u32;
                let raw_name = r.get::<_, Option<String>>(1)?.unwrap_or_default();
                let taxation_id = any_int(r, 12)?.max(0) as u32;
                t.housings.insert(
                    id,
                    HousingTemplate {
                        id,
                        name: names.get(&id).cloned().unwrap_or(raw_name),
                        category_id: any_int(r, 2)?.max(0) as u32,
                        main_model_id: any_int(r, 3)?.max(0) as u32,
                        door_model_id: any_int(r, 4)?.max(0) as u32,
                        stair_model_id: any_int(r, 5)?.max(0) as u32,
                        hp: any_int(r, 8)? as i32,
                        repair_cost: any_int(r, 9)?.max(0) as u32,
                        garden_radius: r.get::<_, Option<f64>>(10)?.unwrap_or(0.0) as f32,
                        family: r.get::<_, Option<String>>(11)?.unwrap_or_default(),
                        taxation_id,
                        tax: taxes.get(&taxation_id).copied().unwrap_or(0),
                        deco_limit: any_int(r, 22)?.max(0) as u32,
                        absolute_deco_limit: any_int(r, 24)?.max(0) as u32,
                        housing_deco_limit_id: any_int(r, 25)?.max(0) as u32,
                        is_sellable: any_flag(r, 26)?,
                        heavy_tax: any_flag(r, 27)?,
                        always_public: any_flag(r, 28)?,
                        build_steps: Vec::new(),
                    },
                );
            }
        }

        {
            let mut q = conn.prepare("SELECT * FROM housing_build_steps")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let housing_id = any_int(r, 1)?.max(0) as u32;
                let Some(template) = t.housings.get_mut(&housing_id) else {
                    continue;
                };
                template.build_steps.push(HousingBuildStep {
                    id: any_int(r, 0)?.max(0) as u32,
                    housing_id,
                    step: any_int(r, 2)? as i16,
                    model_id: any_int(r, 3)?.max(0) as u32,
                    skill_id: any_int(r, 4)?.max(0) as u32,
                    num_actions: any_int(r, 5)? as i32,
                });
            }
        }

        {
            let mut q = conn.prepare("SELECT * FROM item_housings")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let row = ItemHousing {
                    id: any_int(r, 0)?.max(0) as u32,
                    item_id: any_int(r, 1)?.max(0) as u32,
                    design_id: any_int(r, 2)?.max(0) as u32,
                };
                t.housing_design_by_item.entry(row.item_id).or_insert(row.design_id);
                t.item_housings.push(row);
            }
        }

        {
            let mut q = conn.prepare("SELECT * FROM housing_decorations")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let row = HousingDecoration {
                    id: any_int(r, 0)?.max(0) as u32,
                    name: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    allow_on_floor: any_flag(r, 2)?,
                    allow_on_wall: any_flag(r, 3)?,
                    allow_on_ceiling: any_flag(r, 4)?,
                    doodad_id: any_int(r, 5)?.max(0) as u32,
                    allow_pivot_on_garden: any_flag(r, 6)?,
                    actability_group_id: any_int(r, 7)?.max(0) as u32,
                    actability_up: any_int(r, 8)?.max(0) as u32,
                    deco_actability_group_id: any_int(r, 9)?.max(0) as u32,
                    allow_mesh_on_garden: any_flag(r, 10)?,
                };
                t.decoration_by_doodad.entry(row.doodad_id).or_insert(row.id);
                t.decorations.insert(row.id, row);
            }
        }

        {
            let mut q = conn.prepare("SELECT * FROM item_housing_decorations")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let row = ItemHousingDecoration {
                    id: any_int(r, 0)?.max(0) as u32,
                    item_id: any_int(r, 1)?.max(0) as u32,
                    design_id: any_int(r, 2)?.max(0) as u32,
                    restore: any_flag(r, 3)?,
                };
                t.decoration_design_by_item.entry(row.item_id).or_insert(row.design_id);
                t.item_housing_decorations.push(row);
            }
        }

        {
            let mut q = conn.prepare("SELECT * FROM item_spawn_doodads")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let row = ItemSpawnDoodad {
                    id: any_int(r, 0)?.max(0) as u32,
                    item_id: any_int(r, 1)?.max(0) as u32,
                    doodad_id: any_int(r, 2)?.max(0) as u32,
                };
                t.doodad_by_item.entry(row.item_id).or_insert(row.doodad_id);
                t.item_spawn_doodads.push(row);
            }
        }

        {
            let mut q = conn.prepare("SELECT * FROM housing_areas")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let row = HousingArea {
                    id: any_int(r, 0)?.max(0) as u32,
                    housing_group_id: any_int(r, 2)?.max(0) as u32,
                };
                t.housing_areas.insert(row.id, row);
            }
        }

        {
            let mut q = conn.prepare("SELECT * FROM housing_groups")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let row = HousingGroup {
                    id: any_int(r, 0)?.max(0) as u32,
                    houseless: any_flag(r, 4)?,
                    existing_category_id: any_int(r, 5)?.max(0) as u32,
                    can_extend: any_flag(r, 7)?,
                };
                t.housing_groups.insert(row.id, row);
            }
        }

        {
            let mut q = conn.prepare("SELECT * FROM farm_groups")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let row = FarmGroup {
                    id: any_int(r, 0)?.max(0) as u32,
                    name: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    count: any_int(r, 2)?.max(0) as u32,
                    description: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                };
                t.farm_groups.insert(row.id, row);
            }
        }

        {
            let mut q = conn.prepare("SELECT * FROM farm_group_doodads")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                t.farm_group_doodads.push(FarmGroupDoodad {
                    id: any_int(r, 0)?.max(0) as u32,
                    farm_group_id: any_int(r, 1)?.max(0) as u32,
                    doodad_id: any_int(r, 2)?.max(0) as u32,
                    item_id: any_int(r, 3)?.max(0) as u32,
                });
            }
        }

        {
            let mut q = conn.prepare("SELECT * FROM common_farms")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                t.common_farms.push(CommonFarm {
                    id: any_int(r, 0)?.max(0) as u32,
                    name: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    guard_time: any_int(r, 2)?.max(0) as u32,
                    farm_group_id: any_int(r, 3)?.max(0) as u32,
                    comments: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                });
            }
        }

        Ok(t)
    }

    /// `farm_groups.count` of a farm kind (0 when unknown).
    pub fn farm_group_max_count(&self, kind: u32) -> u32 {
        self.farm_groups.get(&kind).map_or(0, |g| g.count)
    }

    /// The doodad templates allowed for a farm kind, in row order.
    pub fn allowed_farm_doodads(&self, kind: u32) -> impl Iterator<Item = u32> + '_ {
        self.farm_group_doodads
            .iter()
            .filter(move |d| d.farm_group_id == kind)
            .map(|d| d.doodad_id)
    }

    /// The guard time (ms) of the first farm of `kind` whose comment mentions
    /// `zone_key`, else the first farm of `kind`, else 0.
    pub fn farm_guard_time(&self, kind: u32, zone_key: u32) -> u32 {
        let key = zone_key.to_string();
        self.common_farms
            .iter()
            .find(|f| f.farm_group_id == kind && f.comments.contains(&key))
            .or_else(|| self.common_farms.iter().find(|f| f.farm_group_id == kind))
            .map_or(0, |f| f.guard_time)
    }

    /// The housing design of a `item_housings` item.
    pub fn housing_design_for_item(&self, item_id: u32) -> Option<u32> {
        self.housing_design_by_item.get(&item_id).copied()
    }

    /// The decoration design of an `item_housing_decorations` item.
    pub fn decoration_design_for_item(&self, item_id: u32) -> Option<u32> {
        self.decoration_design_by_item.get(&item_id).copied()
    }

    /// All construction actions of a housing template.
    pub fn all_actions(&self, housing_id: u32) -> i32 {
        self.housings
            .get(&housing_id)
            .map_or(0, |h| h.build_steps.iter().map(|s| s.num_actions).sum())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE taxations (id INT, tax INT, "desc" TEXT, SHOW NUM);
            CREATE TABLE localized_texts (tbl_name TEXT, idx INT, tbl_column_name TEXT, en_us TEXT);
            CREATE TABLE housings (id INT, name TEXT, category_id INT, main_model_id INT, door_model_id INT,
                stair_model_id INT, auto_z NUM, gate_exists NUM, hp INT, repair_cost INT, garden_radius REAL,
                family TEXT, taxation_id INT, guard_tower_setting_id INT, cinema_id INT, cinema_radius REAL,
                auto_z_offset_x REAL, auto_z_offset_y REAL, auto_z_offset_z REAL, alley REAL,
                extra_height_above REAL, extra_height_below REAL, deco_limit INT, comments TEXT,
                absolute_deco_limit INT, housing_deco_limit_id INT, is_sellable NUM, heavy_tax NUM,
                always_public NUM, demolish_refund_item_id INT);
            CREATE TABLE housing_build_steps (id INT, housing_id INT, step INT, model_id INT, skill_id INT, num_actions INT);
            CREATE TABLE item_housings (id INT, item_id INT, design_id INT);
            CREATE TABLE housing_decorations (id INT, name TEXT, allow_on_floor NUM, allow_on_wall NUM,
                allow_on_ceiling NUM, doodad_id INT, allow_pivot_on_garden NUM, actability_group_id INT,
                actability_up INT, deco_actability_group_id INT, allow_mesh_on_garden NUM);
            CREATE TABLE item_housing_decorations (id INT, item_id INT, design_id INT, RESTORE NUM);
            CREATE TABLE item_spawn_doodads (id INT, item_id INT, doodad_id INT);
            CREATE TABLE housing_areas (id INT, name TEXT, housing_group_id INT, comments TEXT);
            CREATE TABLE housing_groups (id INT, name TEXT, "desc" TEXT, doodad_id INT, houseless NUM,
                existing_category_id INT, allowed_tax_delay_week INT, can_extend NUM);
            CREATE TABLE farm_groups (id INT, name TEXT, count INT, description TEXT);
            CREATE TABLE farm_group_doodads (id INT, farm_group_id INT, doodad_id INT, item_id INT);
            CREATE TABLE common_farms (id INT, name TEXT, guard_time INT, farm_group_id INT, comments TEXT);
            INSERT INTO taxations VALUES (3, 150000, 'small', 't');
            INSERT INTO localized_texts VALUES ('housings', 100, 'name', 'Rose Villa');
            INSERT INTO housings VALUES (100, 'Korean', 1, 737, NULL, NULL, 't', 't', 1600, 10, 0.0, 'f',
                3, 0, 0, 0.0, 0.0, 0.0, 0.0, 0.0, 10.0, 40, 60, NULL, 61, 0, 'f', 't', 'f', 0);
            INSERT INTO housings VALUES (101, 'Second', 1, 921, 0, 0, 'f', 'f', 4000, 10, 0.0, 'f',
                0, 0, 0, 0.0, 0.0, 0.0, 0.0, 0.0, 10.0, 40, 60, NULL, 61, 0, 't', 'f', 't', 0);
            INSERT INTO housing_build_steps VALUES (1, 100, 0, 733, 11026, 1);
            INSERT INTO housing_build_steps VALUES (2, 100, 1, 734, 11026, 2);
            INSERT INTO housing_build_steps VALUES (3, 999, 0, 1, 2, 3);
            INSERT INTO item_housings VALUES (47, 14805, 100);
            INSERT INTO housing_decorations VALUES (10, 'Pot', '1', '0', '0', 5461, 't', 0, 0, 0, 'f');
            INSERT INTO item_housing_decorations VALUES (20, 107, 10, 't');
            INSERT INTO item_spawn_doodads VALUES (21, 4865, 413);
            INSERT INTO item_spawn_doodads VALUES (68, 13725, 380);
            INSERT INTO housing_areas VALUES (100, 'w_golden_plains_1', 5, 'shape');
            INSERT INTO housing_groups VALUES (5, 'group', 'desc', 0, 'f', 1, 1, 't');
            INSERT INTO farm_groups VALUES (1, 'Farm', 10, 'crops');
            INSERT INTO farm_groups VALUES (2, 'Nursery', 5, 'trees');
            INSERT INTO farm_group_doodads VALUES (13, 1, 380, 13725);
            INSERT INTO farm_group_doodads VALUES (14, 1, 381, 13726);
            INSERT INTO farm_group_doodads VALUES (15, 2, 413, 4865);
            INSERT INTO common_farms VALUES (10, 'Farm', 86400000, 1, 'zone (33)');
            INSERT INTO common_farms VALUES (11, 'Farm', 3600000, 1, 'other');
            INSERT INTO common_farms VALUES (12, 'Nursery', 0, 2, '');
            "#,
        )
        .unwrap();
        conn
    }

    #[test]
    fn tables_mirror_the_server_loaders_and_skip_orphans() {
        let conn = fixture();
        let t = Tables::load(&conn).unwrap();
        assert_eq!(t.housings.len(), 2);
        let house = &t.housings[&100];
        assert_eq!(house.name, "Rose Villa", "localized name wins");
        assert_eq!((house.main_model_id, house.hp, house.tax), (737, 1600, 150000));
        assert!(house.heavy_tax && !house.is_sellable && !house.always_public);
        assert_eq!(house.build_steps.len(), 2);
        assert_eq!(t.housings[&101].name, "Second");
        assert_eq!(t.all_actions(100), 3, "orphan build step 3 is not counted");
        assert_eq!(t.housing_design_for_item(14805), Some(100));
        assert_eq!(t.decorations[&10].doodad_id, 5461);
        assert_eq!(t.decoration_by_doodad[&5461], 10);
        assert_eq!(t.decoration_design_for_item(107), Some(10));
        assert!(t.item_housing_decorations[0].restore);
        assert_eq!(t.doodad_by_item[&4865], 413);
        assert_eq!(t.item_spawn_doodads.len(), 2);
        assert_eq!(t.housing_areas[&100].housing_group_id, 5);
        assert!(t.housing_groups[&5].can_extend && !t.housing_groups[&5].houseless);
        assert_eq!(t.farm_group_max_count(1), 10);
        assert_eq!(t.allowed_farm_doodads(1).collect::<Vec<_>>(), vec![380, 381]);
        assert_eq!(t.farm_guard_time(1, 33), 86400000);
        assert_eq!(t.farm_guard_time(1, 7), 86400000);
        assert_eq!(t.farm_guard_time(2, 7), 0);
        assert_eq!(t.common_farms.len(), 3);
    }

    #[test]
    fn real_compact_db_housing_and_farm_tables_match_row_counts() {
        let Ok(path) = std::env::var("AA_DB") else {
            return;
        };
        let conn = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .unwrap();
        let t = Tables::load(&conn).unwrap();
        let count = |table: &str| -> usize {
            conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get::<_, i64>(0))
                .unwrap() as usize
        };
        assert_eq!(t.housings.len(), count("housings"));
        assert_eq!(t.item_housings.len(), count("item_housings"));
        assert_eq!(t.item_housing_decorations.len(), count("item_housing_decorations"));
        assert_eq!(t.item_spawn_doodads.len(), count("item_spawn_doodads"));
        assert_eq!(t.housing_areas.len(), count("housing_areas"));
        assert_eq!(t.housing_groups.len(), count("housing_groups"));
        assert_eq!(t.farm_groups.len(), count("farm_groups"));
        assert_eq!(t.farm_group_doodads.len(), count("farm_group_doodads"));
        assert_eq!(t.common_farms.len(), count("common_farms"));
        let attached: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM housing_build_steps WHERE housing_id IN (SELECT id FROM housings)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let loaded: usize = t.housings.values().map(|h| h.build_steps.len()).sum();
        assert_eq!(loaded as i64, attached, "orphan build steps are skipped");
        assert!(t.housings.values().all(|h| h.build_steps.iter().all(|s| s.housing_id == h.id)));
        assert_eq!(t.decorations.len(), count("housing_decorations"));
    }
}
