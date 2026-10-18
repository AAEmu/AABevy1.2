//! Read-only recipe/proficiency definitions. Raw IDs stay distinct from Lua display indices.
use super::super::data::{any_flag, any_int};
use rusqlite::Connection;
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Debug, Default)]
pub(super) struct Group {
    pub name: String,
    pub icon: String,
    pub visible: bool,
    pub skill_page_visible: bool,
    pub unit_attr: i32,
}

#[derive(Clone, Debug, Default)]
pub(super) struct Category {
    pub name: String,
    pub group: u32,
    pub visible: bool,
    pub order: i32,
}

#[derive(Clone, Debug)]
pub(super) struct Expert {
    pub name: String,
    pub up_limit: i32,
    pub expert_limit: i32,
    pub advantage: i32,
    pub cast_advantage: f32,
    pub color_argb: u32,
    pub gauge_color: u32,
    pub visible: bool,
    pub up_currency: i32,
    pub up_price: i32,
    pub down_currency: i32,
    pub down_price: i32,
}

#[derive(Clone, Debug)]
pub(super) struct Expansion {
    pub life_points: i32,
    pub item: u32,
    pub count: i32,
}

#[derive(Clone, Debug, Default)]
pub(super) struct Material {
    pub item: u32,
    pub amount: i32,
    pub main_grade: bool,
    pub require_grade: i32,
}

#[derive(Clone, Debug, Default)]
pub(super) struct Product {
    pub item: u32,
    pub amount: i32,
    pub rate: i32,
    pub show_lower: bool,
    pub use_grade: bool,
    pub grade: i32,
}

#[derive(Clone, Debug, Default)]
pub(super) struct Recipe {
    pub title: String,
    pub description: String,
    pub cast_delay: i32,
    pub tool: u32,
    pub skill: u32,
    pub wi: u32,
    pub required_doodad: u32,
    pub category: u32,
    pub actability_limit: i32,
    pub show_upper: bool,
    pub recommend_level: i32,
    pub order: i32,
    pub consume_lp: i32,
    pub actability_group: u32,
    pub materials: Vec<Material>,
    pub products: Vec<Product>,
}

#[derive(Default)]
pub(in crate::hud) struct Tables {
    pub(super) groups: BTreeMap<u32, Group>,
    pub(super) categories: BTreeMap<u32, Category>,
    /// Raw database IDs, independent of the native visible-grade index.
    pub(super) experts: BTreeMap<u32, Expert>,
    /// SHOW rows only, assigned consecutive grades in native up_limit order.
    pub(super) grades: Vec<u32>,
    /// Received expanded_expert is the current count, these keys are the next target count.
    pub(super) expansions: BTreeMap<u32, Expansion>,
    pub(super) recipes: BTreeMap<u32, Recipe>,
    pub(super) by_category: HashMap<u32, Vec<u32>>,
    pub(super) by_material: HashMap<u32, Vec<u32>>,
    pub(super) by_product: HashMap<u32, Vec<u32>>,
    /// Native global-table order, including repeated matching materials/products.
    pub(super) material_order: Vec<(u32, usize)>,
    pub(super) product_order: Vec<(u32, usize)>,
    pub(super) doodad_names: HashMap<u32, String>,
}

impl Tables {
    pub(in crate::hud) fn load(conn: &Connection) -> rusqlite::Result<Self> {
        let mut labels = HashMap::new();
        let mut query = conn.prepare("SELECT tbl_name,tbl_column_name,idx,en_us FROM localized_texts WHERE en_us IS NOT NULL AND en_us<>'' AND tbl_name IN ('crafts','actability_groups','actability_categories','expert_limits','doodad_almighties')")?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            let table: String = row.get(0)?;
            let column: String = row.get(1)?;
            let key = match (table.as_str(), column.as_str()) {
                ("crafts", "title") => "craft_title",
                ("crafts", "desc") => "craft_desc",
                ("actability_groups", "name") => "group",
                ("actability_categories", "name") => "category",
                ("expert_limits", "name") => "expert",
                ("doodad_almighties", "name") => "doodad",
                _ => continue,
            };
            labels.insert((key, any_int(row, 2)? as u32), row.get::<_, String>(3)?);
        }
        let name = |kind: &'static str, id, fallback: Option<String>| labels.get(&(kind, id)).cloned().or(fallback).unwrap_or_default();
        let mut out = Self::default();
        let mut query = conn.prepare("SELECT id,name,icon_path,visible,skill_page_visible,unit_attr_id FROM actability_groups ORDER BY id")?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            let id = any_int(row, 0)? as u32;
            out.groups.insert(
                id,
                Group {
                    name: name("group", id, row.get(1)?),
                    icon: row
                        .get::<_, Option<String>>(2)?
                        .unwrap_or_default()
                        .replace('\\', "/")
                        .trim_start_matches('/')
                        .into(),
                    visible: any_flag(row, 3)?,
                    skill_page_visible: any_flag(row, 4)?,
                    unit_attr: any_int(row, 5)? as i32,
                },
            );
        }
        let mut query = conn.prepare("SELECT id,name,group_id,visible_ui,visible_order FROM actability_categories ORDER BY id")?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            let id = any_int(row, 0)? as u32;
            out.categories.insert(
                id,
                Category {
                    name: name("category", id, row.get(1)?),
                    group: any_int(row, 2)? as u32,
                    visible: any_flag(row, 3)?,
                    order: any_int(row, 4)? as i32,
                },
            );
        }
        let mut query = conn.prepare("SELECT id,name,up_limit,expert_limit,advantage,cast_adv,color_argb,gauge_color,SHOW,up_currency_id,up_price,down_currency_id,down_price FROM expert_limits ORDER BY up_limit ASC")?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            let id = any_int(row, 0)? as u32;
            let visible = any_flag(row, 8)?;
            if visible {
                out.grades.push(id);
            }
            out.experts.insert(
                id,
                Expert {
                    name: name("expert", id, row.get(1)?),
                    up_limit: any_int(row, 2)? as i32,
                    expert_limit: any_int(row, 3)? as i32,
                    advantage: any_int(row, 4)? as i32,
                    cast_advantage: row.get::<_, f64>(5)? as f32,
                    color_argb: any_int(row, 6)? as u32,
                    gauge_color: any_int(row, 7)? as u32,
                    visible,
                    up_currency: any_int(row, 9)? as i32,
                    up_price: any_int(row, 10)? as i32,
                    down_currency: any_int(row, 11)? as i32,
                    down_price: any_int(row, 12)? as i32,
                },
            );
        }
        let mut query = conn.prepare("SELECT expand_count,life_point,item_id,item_count FROM expand_expert_limits ORDER BY expand_count,id")?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            out.expansions.insert(
                any_int(row, 0)? as u32,
                Expansion {
                    life_points: any_int(row, 1)? as i32,
                    item: any_int(row, 2)? as u32,
                    count: any_int(row, 3)? as i32,
                },
            );
        }
        let mut query = conn.prepare("SELECT c.id,c.title,c.desc,c.cast_delay,c.tool_id,c.skill_id,c.wi_id,c.req_doodad_id,c.ac_id,c.actability_limit,c.show_upper_crafts,c.recommend_level,c.visible_order,s.consume_lp,s.actability_group_id FROM crafts c LEFT JOIN skills s ON s.id=c.skill_id ORDER BY c.id")?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            let id = any_int(row, 0)? as u32;
            if id == 0 {
                continue;
            }
            out.recipes.insert(
                id,
                Recipe {
                    title: name("craft_title", id, row.get(1)?),
                    description: name("craft_desc", id, row.get(2)?),
                    cast_delay: any_int(row, 3)? as i32,
                    tool: any_int(row, 4)? as u32,
                    skill: any_int(row, 5)? as u32,
                    wi: any_int(row, 6)? as u32,
                    required_doodad: any_int(row, 7)? as u32,
                    category: any_int(row, 8)? as u32,
                    actability_limit: any_int(row, 9)? as i32,
                    show_upper: any_flag(row, 10)?,
                    recommend_level: any_int(row, 11)? as i32,
                    order: any_int(row, 12)? as i32,
                    consume_lp: any_int(row, 13)? as i32,
                    actability_group: any_int(row, 14)? as u32,
                    materials: Vec::new(),
                    products: Vec::new(),
                },
            );
        }
        let mut query = conn.prepare("SELECT craft_id,item_id,amount,main_grade,require_grade FROM craft_materials ORDER BY id")?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            let craft = any_int(row, 0)? as u32;
            let Some(recipe) = out.recipes.get_mut(&craft) else {
                continue;
            };
            out.material_order.push((craft, recipe.materials.len()));
            recipe.materials.push(Material {
                item: any_int(row, 1)? as u32,
                amount: any_int(row, 2)? as i32,
                main_grade: any_flag(row, 3)?,
                require_grade: any_int(row, 4)? as i32,
            });
        }
        let mut query = conn.prepare("SELECT craft_id,item_id,amount,rate,show_lower_crafts,use_grade,item_grade_id FROM craft_products ORDER BY id")?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            let craft = any_int(row, 0)? as u32;
            let Some(recipe) = out.recipes.get_mut(&craft) else {
                continue;
            };
            out.product_order.push((craft, recipe.products.len()));
            recipe.products.push(Product {
                item: any_int(row, 1)? as u32,
                amount: any_int(row, 2)? as i32,
                rate: any_int(row, 3)? as i32,
                show_lower: any_flag(row, 4)?,
                use_grade: any_flag(row, 5)?,
                grade: any_int(row, 6)? as i32,
            });
        }
        for (&id, recipe) in &out.recipes {
            out.by_category.entry(recipe.category).or_default().push(id);
            for material in &recipe.materials {
                out.by_material.entry(material.item).or_default().push(id);
            }
            for product in &recipe.products {
                out.by_product.entry(product.item).or_default().push(id);
            }
        }
        for ids in out.by_category.values_mut() {
            ids.sort_by_key(|id| (out.recipes[id].order, *id));
        }
        for ids in out.by_material.values_mut().chain(out.by_product.values_mut()) {
            ids.sort_unstable();
            ids.dedup();
        }
        let mut query = conn.prepare("SELECT id,name FROM doodad_almighties ORDER BY id")?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            let id = any_int(row, 0)? as u32;
            out.doodad_names.insert(id, name("doodad", id, row.get(1)?));
        }
        Ok(out)
    }
}
