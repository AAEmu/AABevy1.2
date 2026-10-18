use super::super::api::HudData;
use super::super::{store, tips};
use super::{Shared, Tables, labor, modify_point, proficiency};
use aa_net::model::Item;
use mlua::{Lua, Table, Value};

fn categories(tables: &Tables, group: Option<u32>) -> Vec<u32> {
    let mut ids = tables
        .categories
        .iter()
        .filter(|(_, c)| c.visible && group.is_none_or(|g| c.group == g))
        .map(|(&id, _)| id)
        .collect::<Vec<_>>();
    ids.sort_by_key(|id| (tables.categories[id].order, *id));
    ids
}

fn nonempty(lua: &Lua, ids: impl IntoIterator<Item = u32>) -> mlua::Result<Option<Table>> {
    let ids = ids.into_iter().collect::<Vec<_>>();
    if ids.is_empty() { Ok(None) } else { Ok(Some(lua.create_sequence_from(ids)?)) }
}

fn tooltip(lua: &Lua, data: &HudData, template: u32, grade: u8, item: Option<&Item>) -> mlua::Result<Option<Table>> {
    let Some(game) = data.game.get() else {
        return Ok(None);
    };
    let instance = item.map_or_else(tips::Instance::default, |item| tips::Instance {
        flags: Some(item.flags),
        durability: match &item.detail {
            aa_net::model::ItemDetail::Equip(d) => Some(d.durability),
            _ => None,
        },
        temper: match &item.detail {
            aa_net::model::ItemDetail::Equip(d) => (d.temper_physical, d.temper_magical),
            _ => (0, 0),
        },
    });
    let info = tips::item_tip(
        lua,
        game,
        &data.tips,
        template,
        Some(i64::from(store::ui_grade(grade))),
        item.map(|i| i64::from(i.count)),
        instance,
    )?;
    if let Some(info) = &info {
        info.set("itemGrade", grade)?;
        if item.is_none() {
            info.set("stack", Value::Nil)?;
        }
    }
    Ok(info)
}

// Native MaterialInfo and executable-count have different secure-item gates. When the
// respective policy is unknown, exclude flagged stacks conservatively; ordinary stacks
// remain exact. Do not infer playerflags2000 from the unrelated SC service flags.
fn materials(data: &HudData, template: u32, grade: i32, secure: Option<bool>) -> (i64, Option<&Item>) {
    let mut count = 0i64;
    let mut representative = None;
    if let Some(inventory) = &data.crafting.inventory {
        for item in inventory.bag.iter().take(inventory.bag_slots as usize).flatten() {
            if item.template_id != template || (grade != -1 && grade != i32::from(item.grade)) {
                continue;
            }
            if item.flags & 4 != 0 && secure != Some(false) {
                continue;
            }
            if count == 0 {
                representative = Some(item);
            }
            count = count.wrapping_add(i64::from(item.count));
        }
    }
    if count == 0 {
        representative = None;
    }
    (count, representative)
}

fn base_info(lua: &Lua, data: &HudData, craft: u32) -> mlua::Result<Option<Table>> {
    let Some(game) = data.game.get() else {
        return Ok(None);
    };
    let tables = &game.crafting;
    let Some(recipe) = tables.recipes.get(&craft) else {
        return Ok(None);
    };
    let t = lua.create_table()?;
    t.set("craft_type", craft)?;
    t.set("ac_type", recipe.category)?;
    t.set("title", recipe.title.as_str())?;
    t.set("desc", recipe.description.as_str())?;
    t.set("tool_type", recipe.tool)?;
    t.set("skill_type", recipe.skill)?;
    t.set("cast_delay", recipe.cast_delay)?;
    t.set("recommend_level", recipe.recommend_level)?;
    if let Some(name) = tables.doodad_names.get(&recipe.required_doodad) {
        t.set("doodad_name", name.as_str())?;
    }
    if game.skills.contains_key(&recipe.skill) {
        let known = proficiency(data, recipe.actability_group);
        let (points, grade) = known.unwrap_or((0, 0));
        let effective = points.wrapping_add(modify_point(data, recipe.actability_group));
        let needed = labor(tables, recipe.actability_group, recipe.consume_lp, effective, grade);
        t.set("consume_lp", recipe.consume_lp)?;
        t.set("needed_lp", needed)?;
        t.set("laborpower_satisfied", data.snapshot.labor >= needed)?;
        if let Some(group) = tables.groups.get(&recipe.actability_group) {
            t.set("required_actability_type", recipe.actability_group)?;
            t.set("required_actability_name", group.name.as_str())?;
            t.set("required_actability_point", recipe.actability_limit)?;
            t.set("actability_satisfied", known.is_some() && effective >= recipe.actability_limit)?;
        } else {
            t.set("actability_satisfied", true)?;
        }
    }
    Ok(Some(t))
}

fn material_info(lua: &Lua, data: &HudData, craft: u32) -> mlua::Result<Option<Table>> {
    let Some(game) = data.game.get() else {
        return Ok(None);
    };
    let Some(recipe) = game.crafting.recipes.get(&craft) else {
        return Ok(None);
    };
    let result = lua.create_table()?;
    for material in &recipe.materials {
        if !game.items.contains_key(&material.item) {
            return Ok(None);
        }
        let (count, representative) = materials(data, material.item, material.require_grade, data.crafting.item_secure);
        let grade = representative.map_or(if material.require_grade == -1 { 0 } else { material.require_grade as u8 }, |i| i.grade);
        let Some(item) = tooltip(lua, data, material.item, grade, representative)? else {
            return Ok(None);
        };
        item.set("mainGrade", material.main_grade)?;
        let row = lua.create_table()?;
        row.set("amount", material.amount)?;
        row.set("count", count)?;
        row.set("mainGrade", material.main_grade)?;
        row.set("item_info", item)?;
        result.raw_push(row)?;
    }
    Ok(Some(result))
}

fn products(lua: &Lua, data: &HudData, craft: u32) -> mlua::Result<Option<Table>> {
    let Some(game) = data.game.get() else {
        return Ok(None);
    };
    let Some(recipe) = game.crafting.recipes.get(&craft) else {
        return Ok(None);
    };
    let result = lua.create_table()?;
    for product in &recipe.products {
        let Some(item) = game.items.get(&product.item) else {
            return Ok(None);
        };
        let row = lua.create_table()?;
        row.set("itemType", product.item)?;
        row.set("item_name", item.named.name.as_str())?;
        row.set("amount", product.amount)?;
        row.set("success_rate", product.rate as f32)?;
        row.set("useGrade", product.use_grade)?;
        row.set("productGrade", product.grade)?;
        result.raw_push(row)?;
    }
    Ok(Some(result))
}

fn executable(data: &HudData, craft: u32) -> Option<i32> {
    let game = data.game.get()?;
    let recipe = game.crafting.recipes.get(&craft)?;
    let mut count = i32::MAX;
    if recipe.materials.is_empty() && game.skills.contains_key(&recipe.skill) && recipe.consume_lp > 0 {
        let (points, grade) = proficiency(data, recipe.actability_group).unwrap_or((0, 0));
        let needed = labor(
            &game.crafting,
            recipe.actability_group,
            recipe.consume_lp,
            points.wrapping_add(modify_point(data, recipe.actability_group)),
            grade,
        );
        if needed > 0 {
            count = data.snapshot.labor / needed;
        }
    }
    for material in &recipe.materials {
        if material.amount == 0 {
            return None;
        }
        // Special currency-material ID and playerflags2000 producer remain source boundaries.
        let available = if game.items.contains_key(&material.item) {
            materials(data, material.item, material.require_grade, None).0
        } else {
            0
        };
        count = count.min((available / i64::from(material.amount)) as i32);
    }
    Some(count)
}

fn search(data: &HudData, group: u32, category: u32, name: &str, material: bool) -> Vec<u32> {
    if !(1..=30).contains(&name.chars().count()) {
        return Vec::new();
    }
    let Some(game) = data.game.get() else {
        return Vec::new();
    };
    let tables = &game.crafting;
    let name = name.to_lowercase();
    let matches = |id: u32| {
        tables
            .recipes
            .get(&id)
            .is_some_and(|r| tables.categories.get(&r.category).is_some_and(|c| c.group == group) && r.category == category)
    };
    if material {
        tables
            .material_order
            .iter()
            .filter_map(|&(id, i)| {
                let m = &tables.recipes[&id].materials[i];
                (matches(id) && game.items.get(&m.item).is_some_and(|item| item.named.name.to_lowercase().contains(&name))).then_some(id)
            })
            .collect()
    } else {
        tables
            .recipes
            .iter()
            .filter_map(|(&id, r)| (matches(id) && r.title.to_lowercase().contains(&name)).then_some(id))
            .collect()
    }
}

fn upper(lua: &Lua, data: &HudData, craft: u32, category: i32) -> mlua::Result<Option<Table>> {
    let Some(game) = data.game.get() else {
        return Ok(None);
    };
    let tables = &game.crafting;
    let Some(source) = tables.recipes.get(&craft) else {
        return Ok(None);
    };
    let result = lua.create_table()?;
    let Some(source) = source.products.last() else {
        return Ok(Some(result));
    };
    for &(craft, i) in &tables.material_order {
        let recipe = &tables.recipes[&craft];
        let material = &recipe.materials[i];
        if material.item != source.item || !recipe.show_upper || (category > 0 && category as u32 != recipe.category) {
            continue;
        }
        let Some(product) = recipe.products.first() else {
            continue;
        };
        let row = lua.create_table()?;
        row.set("craft_Type", craft)?;
        row.set("ac_type", recipe.category)?;
        row.set("itemType", product.item)?;
        row.set("useGrade", product.use_grade)?;
        row.set("itemGrade", product.grade)?;
        row.set("mainGrade", material.main_grade)?;
        if let Some(item) = game.items.get(&product.item) {
            row.set("item_impl", tips::item_impl(item.impl_id))?;
        }
        result.raw_push(row)?;
    }
    Ok(Some(result))
}

pub(super) fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    let craft: Table = lua.globals().get("X2Craft")?;
    let state = bridge.clone();
    craft.set(
        "EnumActabilityGroupTypes",
        lua.create_function(move |lua, _: Value| {
            nonempty(
                lua,
                state
                    .borrow()
                    .game
                    .get()
                    .map(|g| g.crafting.groups.keys().copied().collect::<Vec<_>>())
                    .unwrap_or_default(),
            )
        })?,
    )?;
    let state = bridge.clone();
    craft.set(
        "GetActabilityGroupInfoByGroupType",
        lua.create_function(move |lua, (_self, id): (Value, u32)| {
            let data = state.borrow();
            let Some(g) = data.game.get().and_then(|g| g.crafting.groups.get(&id)) else {
                return Ok(None);
            };
            let t = lua.create_table()?;
            t.set("type", id)?;
            t.set("name", g.name.as_str())?;
            t.set("icon_path", g.icon.as_str())?;
            t.set("visible", g.visible)?;
            t.set("skill_page_visible", g.skill_page_visible)?;
            Ok(Some(t))
        })?,
    )?;
    let state = bridge.clone();
    craft.set(
        "GetActabilityGroupNameByGroupType",
        lua.create_function(move |_, (_self, id): (Value, u32)| {
            Ok(state.borrow().game.get().and_then(|g| g.crafting.groups.get(&id)).map(|g| g.name.clone()))
        })?,
    )?;
    let state = bridge.clone();
    craft.set(
        "GetCategoryType",
        lua.create_function(move |lua, _: Value| {
            lua.create_sequence_from(state.borrow().game.get().map(|g| categories(&g.crafting, None)).unwrap_or_default())
        })?,
    )?;
    let state = bridge.clone();
    craft.set(
        "GetCategoryName",
        lua.create_function(move |_, (_self, id): (Value, u32)| {
            Ok(state.borrow().game.get().and_then(|g| g.crafting.categories.get(&id)).map(|c| c.name.clone()))
        })?,
    )?;
    let state = bridge.clone();
    craft.set(
        "GetActabilityCategoryTypesGroupType",
        lua.create_function(move |lua, (_self, group): (Value, u32)| {
            nonempty(lua, state.borrow().game.get().map(|g| categories(&g.crafting, Some(group))).unwrap_or_default())
        })?,
    )?;
    let state = bridge.clone();
    craft.set(
        "GetActabilityCategoryInfoByTypes",
        lua.create_function(move |lua, (_self, id): (Value, u32)| {
            let data = state.borrow();
            let Some(c) = data.game.get().and_then(|g| g.crafting.categories.get(&id)) else {
                return Ok(None);
            };
            let t = lua.create_table()?;
            t.set("type", id)?;
            t.set("name", c.name.as_str())?;
            Ok(Some(t))
        })?,
    )?;
    let state = bridge.clone();
    craft.set(
        "GetActabilityCategoryCraftTypesByTypes",
        lua.create_function(move |lua, (_self, group, category): (Value, u32, u32)| {
            let data = state.borrow();
            let ids = data
                .game
                .get()
                .filter(|g| g.crafting.categories.get(&category).is_some_and(|c| c.group == group))
                .and_then(|g| g.crafting.by_category.get(&category))
                .cloned()
                .unwrap_or_default();
            nonempty(lua, ids)
        })?,
    )?;
    let state = bridge.clone();
    craft.set(
        "GetCraftName",
        lua.create_function(move |_, (_self, id): (Value, u32)| {
            Ok(state.borrow().game.get().and_then(|g| g.crafting.recipes.get(&id)).map(|r| r.title.clone()))
        })?,
    )?;
    let state = bridge.clone();
    craft.set(
        "GetCraftBaseInfo",
        lua.create_function(move |lua, (_self, id): (Value, u32)| base_info(lua, &state.borrow(), id))?,
    )?;
    let state = bridge.clone();
    craft.set(
        "GetCraftMaterialInfo",
        lua.create_function(move |lua, (_self, id): (Value, u32)| material_info(lua, &state.borrow(), id))?,
    )?;
    let state = bridge.clone();
    craft.set(
        "GetCraftProductInfo",
        lua.create_function(move |lua, (_self, id): (Value, u32)| products(lua, &state.borrow(), id))?,
    )?;
    let state = bridge.clone();
    craft.set(
        "GetExecutableCraftCount",
        lua.create_function(move |_, (_self, id): (Value, u32)| Ok(executable(&state.borrow(), id)))?,
    )?;
    let state = bridge.clone();
    craft.set(
        "GetUpperCraftsInfo",
        lua.create_function(move |lua, (_self, id, category): (Value, u32, i32)| upper(lua, &state.borrow(), id, category))?,
    )?;
    let state = bridge.clone();
    craft.set(
        "GetCraftTypeByItemType",
        lua.create_function(move |_, (_self, item): (Value, u32)| {
            let data = state.borrow();
            Ok(data.game.get().and_then(|g| {
                g.crafting.product_order.iter().find_map(|&(id, i)| {
                    let p = &g.crafting.recipes[&id].products[i];
                    (p.item == item && p.show_lower).then_some(id)
                })
            }))
        })?,
    )?;
    for (method, material) in [("GetAllCraftTypesInGroupByName", false), ("GetCraftTypesByMaterialName", true)] {
        let state = bridge.clone();
        craft.set(
            method,
            lua.create_function(move |lua, (_self, group, category, name): (Value, u32, u32, String)| {
                nonempty(lua, search(&state.borrow(), group, category, &name, material))
            })?,
        )?;
    }
    // Book mode has no selected workbench. Do not manufacture an executable context.
    for method in [
        "EnumActabilityGroupTypesFromCurrentDoodad",
        "GetActabilityCategoryTypesFromCurrentDoodad",
        "GetCraftTypesFromCurrentDoodad",
        "GetCurrentDoodadCraftTypesInGroupByName",
        "GetCurrentDoodadCraftTypesByMaterialName",
    ] {
        craft.set(method, lua.create_function(|lua, _: mlua::MultiValue| lua.create_table())?)?;
    }
    craft.set("IsWorkingCraft", lua.create_function(|_, _: Value| Ok(false))?)?;
    craft.set("GetLeftBatchCount", lua.create_function(|_, _: Value| Ok(0))?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::super::data::{GameData, ItemInfo, Named};
    use super::super::data::{Category, Group, Material, Product, Recipe};
    use super::*;
    use aa_net::state::Inventory;
    use std::{cell::RefCell, rc::Rc};

    #[test]
    fn raw_material_grades_secure_policies_products_links_and_unicode_search() -> Result<(), Box<dyn std::error::Error>> {
        let mut game = GameData::default();
        game.items.insert(
            402,
            ItemInfo {
                named: Named {
                    name: "Copper Dust".into(),
                    ..Default::default()
                },
                fixed_grade: 6,
                ..Default::default()
            },
        );
        game.crafting.groups.insert(
            1,
            Group {
                visible: true,
                ..Default::default()
            },
        );
        game.crafting.categories.insert(
            5,
            Category {
                group: 1,
                visible: true,
                ..Default::default()
            },
        );
        game.crafting.recipes.insert(
            1,
            Recipe {
                title: "가".repeat(30),
                category: 5,
                consume_lp: 100,
                actability_group: 1,
                show_upper: true,
                materials: vec![
                    Material {
                        item: 402,
                        amount: 2,
                        require_grade: 2,
                        main_grade: true,
                    },
                    Material {
                        item: 402,
                        amount: 2,
                        require_grade: 2,
                        main_grade: false,
                    },
                ],
                products: vec![Product {
                    item: 402,
                    amount: 3,
                    rate: 175,
                    use_grade: true,
                    grade: 2,
                    ..Default::default()
                }],
                ..Default::default()
            },
        );
        game.crafting.recipes.insert(
            2,
            Recipe {
                materials: vec![
                    Material {
                        item: 402,
                        amount: 1,
                        require_grade: -1,
                        main_grade: false,
                    },
                    Material {
                        item: 403,
                        amount: 1,
                        require_grade: -1,
                        main_grade: false,
                    },
                ],
                ..Default::default()
            },
        );
        game.crafting.recipes.insert(
            3,
            Recipe {
                materials: vec![Material {
                    item: 402,
                    amount: 0,
                    require_grade: -1,
                    main_grade: false,
                }],
                ..Default::default()
            },
        );
        game.crafting.recipes.insert(
            10,
            Recipe {
                products: vec![Product {
                    item: 402,
                    ..Default::default()
                }],
                ..Default::default()
            },
        );
        game.crafting.material_order = vec![(1, 0), (1, 1), (2, 0), (2, 1), (3, 0)];
        let mut inventory = Inventory::default();
        inventory.bag_slots = 5;
        inventory.bag = vec![
            Some(Item {
                id: 1,
                template_id: 402,
                grade: 0,
                count: 7,
                ..Default::default()
            }),
            Some(Item {
                id: 2,
                template_id: 402,
                grade: 2,
                count: 3,
                flags: 4,
                ..Default::default()
            }),
            Some(Item {
                id: 3,
                template_id: 402,
                grade: 2,
                count: 5,
                ..Default::default()
            }),
            None,
            None,
            Some(Item {
                id: 4,
                template_id: 402,
                grade: 2,
                count: 99,
                ..Default::default()
            }),
        ];
        let mut data = HudData::default();
        data.crafting.inventory = Some(inventory);
        data.crafting.item_secure = Some(false);
        assert!(data.game.0.set(game).is_ok());
        assert_eq!(materials(&data, 402, -1, Some(false)).0, 15);
        assert_eq!(materials(&data, 402, 2, Some(false)).0, 8);
        assert_eq!(materials(&data, 402, 2, Some(true)).0, 5);
        assert_eq!(materials(&data, 402, 2, None).1.unwrap().id, 3);
        assert_eq!(
            executable(&data, 1),
            Some(2),
            "material recipes do not apply labor or proficiency gates in this getter"
        );
        assert_eq!(executable(&data, 3), None);
        let lua = Lua::new();
        let rows = material_info(&lua, &data, 1)?.unwrap();
        let row: Table = rows.get(1)?;
        assert_eq!(row.get::<i32>("count")?, 8);
        assert!(row.get::<bool>("mainGrade")?);
        assert!(row.get::<Table>("item_info")?.get::<bool>("mainGrade")?);
        assert_eq!(row.get::<Table>("item_info")?.get::<u8>("itemGrade")?, 2);
        assert_eq!(
            row.get::<Table>("item_info")?.get::<i32>("stack")?,
            3,
            "descriptor stack belongs to the representative, row.count is aggregate"
        );
        assert!(
            material_info(&lua, &data, 2)?.is_none(),
            "one unresolved material descriptor invalidates the whole getter"
        );
        let product: Table = products(&lua, &data, 1)?.unwrap().get(1)?;
        assert_eq!(product.get::<f32>("success_rate")?, 175.0);
        assert!(product.get::<Option<Table>>("item_info")?.is_none());
        assert_eq!(product.get::<u8>("productGrade")?, 2);
        assert_eq!(upper(&lua, &data, 10, 5)?.unwrap().raw_len(), 2, "matching materials are not deduplicated");
        assert_eq!(search(&data, 1, 5, "cOpPeR", true), vec![1, 1]);
        assert_eq!(search(&data, 1, 5, &"가".repeat(30), false), vec![1]);
        assert!(search(&data, 1, 5, &"가".repeat(31), false).is_empty());
        assert!(
            search(&data, 1, 5, " Copper ", true).is_empty(),
            "native substring search does not trim the query"
        );
        data.crafting.inventory.as_mut().unwrap().bag.fill(None);
        let missing = material_info(&lua, &data, 1)?.unwrap().get::<Table>(1)?.get::<Table>("item_info")?;
        assert_eq!(missing.get::<u8>("itemGrade")?, 2, "required grade is retained without an owned stack");
        data.game = super::super::super::data::SharedGameData::default();
        let mut game = GameData::default();
        game.items.insert(
            402,
            ItemInfo {
                fixed_grade: 6,
                ..Default::default()
            },
        );
        game.crafting.recipes.insert(
            1,
            Recipe {
                materials: vec![Material {
                    item: 402,
                    amount: 1,
                    require_grade: -1,
                    main_grade: false,
                }],
                ..Default::default()
            },
        );
        assert!(data.game.0.set(game).is_ok());
        let missing = material_info(&lua, &data, 1)?.unwrap().get::<Table>(1)?.get::<Table>("item_info")?;
        assert_eq!(missing.get::<u8>("itemGrade")?, 0, "native static default is0, independent of item.fixed_grade");
        assert!(
            missing.get::<Option<i32>>("stack")?.is_none(),
            "a synthesized descriptor is not an owned zero-count stack"
        );
        let bridge = Rc::new(RefCell::new(data));
        lua.globals().set("X2Craft", lua.create_table()?)?;
        install(&lua, &bridge)?;
        lua.load("assert(X2Craft:GetCraftBaseInfo(0)==nil and X2Craft:GetCraftProductInfo(0)==nil)")
            .exec()?;
        Ok(())
    }
}
