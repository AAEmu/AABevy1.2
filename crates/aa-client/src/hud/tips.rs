//! The tooltip info tables the client's tooltip scripts read (`baselib/tooltip/tooltip.lua`): `HUD.item_tip`,
//! `HUD.skill_tip`, `HUD.buff_text`. The shapes are the captured ones of `aa-ui/examples/fake/fake_bag.lua` /
//! `fake_ingame.lua` (`reference/ingame/tooltip_{item,skill,buff}`), filled from the game database. They
//! are built when a slot asks for its content or a tooltip shows, not every frame.
//!
//! What the numbers are: [`tipcalc`](super::tipcalc) works out the placeholders of skill and buff descriptions
//! over the player's attack numbers ([`TipState::attrs`], kept current by [`TipState::refresh`]);
//! [`tipitem`](super::tipitem) the stats of weapons and armor (attack, defence, stat bonuses, durability).

use aa_net::model::Equipment;
use mlua::{Lua, Table};

use super::data::GameData;
use super::tipcalc::Attrs;
use super::tipitem::{slot_type_name, Class};

/// Native39570b90 ItemImplEnum names. Crafting compares these classes when inheriting grades.
pub(super) fn item_impl(id: i64) -> &'static str {
    const NAMES: [&str; 30] = [
        "misc",
        "weapon",
        "armor",
        "body",
        "bag",
        "housing",
        "housing_decoration",
        "tool",
        "summon_slave",
        "spawn_doodad",
        "accept_quest",
        "summon_mate",
        "recipe",
        "crafting",
        "portal",
        "enchanting_gem",
        "report_crime",
        "logic_doodad",
        "has_ucc",
        "open_emblem_ui",
        "shipyard",
        "socket",
        "backpack",
        "open_paper",
        "accessory",
        "treasure",
        "music_sheet",
        "dyeing",
        "slave_equipment",
        "grade_enchanting_support",
    ];
    usize::try_from(id).ok().and_then(|id| NAMES.get(id)).copied().unwrap_or("invalid item_impl")
}

/// X2Item's argument/returned itemGrade is a raw native grade ID. Internal bag snapshots
/// and item_tip use display order; keep that conversion at this public lookup boundary.
pub(super) fn install_native_lookup(lua: &Lua, bridge: &super::api::Shared) -> mlua::Result<()> {
    let item: Table = lua.globals().get("X2Item")?;
    let prior: mlua::Function = item.get("GetItemInfoByType")?;
    let state = bridge.clone();
    item.set(
        "GetItemInfoByType",
        lua.create_function(
            move |lua, (owner, id, grade, flags): (mlua::Value, u32, Option<u8>, mlua::Variadic<mlua::Value>)| {
                let data = state.borrow();
                if let (Some(game), Some(grade)) = (data.game.get(), grade) {
                    let result = item_tip(
                        lua,
                        game,
                        &data.tips,
                        id,
                        Some(i64::from(super::store::ui_grade(grade))),
                        None,
                        Instance::default(),
                    )?;
                    if let Some(info) = &result {
                        info.set("itemGrade", grade)?;
                        info.set("stack", mlua::Value::Nil)?;
                        //394224c6: impl15 always includes its modifier table;
                        // impl21 requires IIK_SOCKET_MODIFIER (default flags=-1).
                        //3947f318/39058d10: missing flags=-1; an explicitly supplied
                        // wrong type (including nil) leaves initialized output zero.
                        let flags = match flags.first() {
                            None => -1.,
                            Some(mlua::Value::Integer(n)) => *n as f32,
                            Some(mlua::Value::Number(n)) => *n as f32,
                            Some(_) => 0.,
                        };
                        let flags = if flags.is_finite() && (-2147483648.0..2147483648.0).contains(&flags) {
                            flags as i32
                        } else {
                            i32::MIN
                        };
                        if game.items.get(&id).is_some_and(|i| i.impl_id == 21) && flags & 1024 == 0 {
                            info.raw_set("modifier", mlua::Value::Nil)?;
                        }
                    }
                    Ok(result)
                } else {
                    drop(data);
                    prior.call::<Option<Table>>((owner, id, grade, flags))
                }
            },
        )?,
    )
}

/// `skills.ability_id` -> the key of the tooltip's ability colour and name (`locale.common.abilityCategory`).
const ABILITIES: [&str; 11] = ["general", "fight", "illusion", "adamant", "will", "death", "wild", "magic", "vocation", "romance", "love"];

/// What the tooltips need of the player: its attack numbers and what it wears.
#[derive(Default)]
pub struct TipState {
    /// The inputs `attrs` were worked out from.
    key: Option<(u8, Equipment, Vec<(u32, i16)>, [i32; 11])>,
    pub attrs: Attrs,
    /// The template ids of the worn items (the set bonuses count them).
    worn: Vec<u32>,
}

impl TipState {
    pub fn lung_capacity_ms(&self) -> u32 {
        self.key.as_ref().map_or(60_000, |_| self.attrs.lung_capacity_ms)
    }

    /// Bring the numbers up to date with the player's level, gear, buffs (id, ability level) and ability
    /// experience; nothing is worked out unless one of them changed.
    pub fn refresh(&mut self, game: &GameData, level: u8, equipment: &Equipment, buffs: Vec<(u32, i16)>, ability_exp: &[i32; 11]) {
        let abilities = ability_exp.map(|total| if total > 0 { i32::from(aa_net::model::exp::level_for_total(i64::from(total))) } else { 0 });
        if self.key.as_ref().is_some_and(|k| k.0 == level && k.1 == *equipment && k.2 == buffs && k.3 == abilities) {
            return;
        }
        let details = self.attrs.details;
        self.attrs = game.tips.attrs(level, equipment, &buffs, &abilities);
        self.attrs.details = details;
        self.worn = equipment.slots.iter().enumerate().filter(|(i, _)| !aa_net::model::slot::is_body_part(*i)).filter_map(|(_, it)| it.as_ref().map(|it| it.template_id)).collect();
        self.key = Some((level, equipment.clone(), buffs, abilities));
    }
}

/// What the tooltip knows of one item beyond its template: the slot table the scripts built the item from
/// (`HUD.bag[i]`): `dur` (durability), `tp` / `tm` (temper values), `fl` (item flags). Nothing for an item
/// that is not in the bag (a shop's, a loot window's).
#[derive(Clone, Copy, Debug, Default)]
pub struct Instance {
    pub durability: Option<u8>,
    pub temper: (u16, u16),
    pub flags: Option<u8>,
}

impl Instance {
    pub fn of(table: Option<&Table>) -> Instance {
        let Some(t) = table else { return Instance::default() };
        let get = |k: &str| t.raw_get::<Option<i64>>(k).ok().flatten();
        Instance {
            durability: get("dur").filter(|d| *d >= 0).map(|d| d.min(255) as u8),
            temper: (get("tp").unwrap_or(0).clamp(0, 65535) as u16, get("tm").unwrap_or(0).clamp(0, 65535) as u16),
            flags: get("fl").map(|f| f as u8),
        }
    }
}

/// `item_grades.id` of a grade the HUD numbers by `grade_order` (the order swaps Basic and Crude).
fn grade_id(order: i64) -> i32 {
    match order {
        1 => 0,
        0 => 1,
        g => g as i32,
    }
}

/// The info of an item as `X2Bag:GetBagItemInfo` returns it: `grade` is the number the server and the icons
/// use for display (0 crude, 1 basic, 2 grand ...); absent grade uses the item's fixed grade or basic.
pub fn item_tip(lua: &Lua, game: &GameData, state: &TipState, id: u32, grade: Option<i64>, count: Option<i64>, instance: Instance) -> mlua::Result<Option<Table>> {
    let Some(info) = game.items.get(&id) else { return Ok(None) };
    let grade = grade.unwrap_or_else(|| i64::from(super::store::ui_grade(info.fixed_grade.max(0) as u8)));
    let t = lua.create_table()?;
    t.raw_set("itemType", id)?;
    t.raw_set("isPetOnly", info.pet_only)?;
    t.raw_set("lookType", id)?;
    t.raw_set("itemGrade", grade)?;
    if let Some((name, color)) = game.grades.get(&grade) {
        t.raw_set("grade", name.as_str())?;
        t.raw_set("gradeColor", color.as_str())?;
    }
    t.raw_set("name", info.named.name.as_str())?;
    t.raw_set("category", info.category.as_str())?;
    t.raw_set("item_impl", item_impl(info.impl_id))?;
    match instance.flags {
        Some(flags) if flags & 1 != 0 => t.raw_set("soul_bound", 1)?,
        Some(_) => {}
        None => {
            if let Some(bind) = game.tips.items.soul_bind(id) {
                t.raw_set("soul_bind", bind)?;
            }
        }
    }
    if info.use_skill != 0 {
        t.raw_set("skillType", info.use_skill)?;
    }
    t.raw_set("description", info.desc.as_str())?;
    t.raw_set("stack", count.unwrap_or(1))?;
    t.raw_set("isStackable", info.max_stack > 1)?;
    t.raw_set("level_requirement", info.level)?;
    t.raw_set("level_limit", info.level_limit)?;
    t.raw_set("price", info.price)?;
    t.raw_set("refund", game.stores.refund(info.refund, grade_id(grade).clamp(0,255) as u8))?;
    t.raw_set("sellable", info.sellable)?;
    equipment_stats(lua, game, state, &t, id, grade_id(grade), instance)?;
    if matches!(info.impl_id, 15 | 21) {
        if let Some(modifiers) = game.enchant.modifier_table(lua, id)? {
            t.raw_set("modifier", modifiers)?;
        }
    }
    Ok(Some(t))
}

/// The weapon / armor sections of an item's table: attack or defence, speed, stat bonuses, durability, set.
fn equipment_stats(lua: &Lua, game: &GameData, state: &TipState, t: &Table, id: u32, grade: i32, instance: Instance) -> mlua::Result<()> {
    let items = &game.tips.items;
    let Some(s) = items.stats(id, grade, instance.temper.0, instance.temper.1) else { return Ok(()) };
    let Some(class) = s.class else { return Ok(()) };
    t.raw_set("itemUsage", "equip")?;
    if let Some(name) = slot_type_name(s.slot_type) {
        t.raw_set("slotType", name)?;
    }
    t.raw_set("slotTypeNum", s.slot_type)?;
    if class == Class::Weapon {
        t.raw_set("DPS", s.dps)?;
        t.raw_set("minDamage", s.min_damage)?;
        t.raw_set("maxDamage", s.max_damage)?;
        if s.attack_delay > 0.0 {
            t.raw_set("attackDelay", s.attack_delay)?;
        }
        if s.magic_dps.round() != 0.0 {
            t.raw_set("magicDps", s.magic_dps.round())?;
        }
        if s.heal_dps.round() != 0.0 {
            t.raw_set("healDps", s.heal_dps.round())?;
        }
        // how the weapon hits: piercing, slashing, blunt
        if s.extra_damage.iter().any(|v| *v != 0) {
            for (i, v) in s.extra_damage.iter().enumerate() {
                t.raw_set(format!("extraDamage{}", i + 1), *v)?;
            }
        }
    }
    if s.armor != 0 {
        t.raw_set("armor", s.armor)?;
    }
    if s.magic_resistance != 0 {
        t.raw_set("magicResistance", s.magic_resistance)?;
    }
    if s.max_durability > 0 {
        let durability = i32::from(instance.durability.unwrap_or(u8::MAX));
        t.raw_set("durability", if instance.durability.is_some() { durability } else { s.max_durability })?;
        t.raw_set("maxDurability", s.max_durability)?;
        t.raw_set("repairable", s.repairable)?;
        if s.repairable && durability < s.max_durability && instance.durability.is_some() {
            if let Some(cost) = super::repair::item_cost(game, id, grade.clamp(0, 255) as u8, durability as u8, s.max_durability as u8) {
                t.raw_set("repairCost", cost)?;
            }
        }
    }
    if !s.use_as_stat {
        t.raw_set("useAsStat", false)?;
    }
    // the stat bonuses: {name, type 0 (a value), value}
    let modifiers = lua.create_table()?;
    for (name, value) in ["str", "dex", "sta", "int", "spi"].into_iter().zip(s.stats) {
        if value != 0 {
            let m = lua.create_table()?;
            m.raw_set("name", name)?;
            m.raw_set("type", 0)?;
            m.raw_set("value", value)?;
            modifiers.raw_push(m)?;
        }
    }
    if modifiers.raw_len() > 0 {
        t.raw_set("modifier", modifiers)?;
    }
    if let Some(set) = set_info(lua, game, state, s.set, game.items.get(&id).map_or(0, |i| i.level) as i32)? {
        t.raw_set("equipSetInfo", set)?;
    }
    Ok(())
}

/// `equipSetInfo`: the set's name, its pieces (worn ones lit) and bonuses (the ones the worn pieces earn lit).
fn set_info(lua: &Lua, game: &GameData, state: &TipState, set: u32, level: i32) -> mlua::Result<Option<Table>> {
    let Some(set) = (set != 0).then(|| game.tips.items.set(set)).flatten() else { return Ok(None) };
    let worn = set.items.iter().filter(|i| state.worn.contains(i)).count();
    let t = lua.create_table()?;
    t.raw_set("equipSetItemInfoDesc", set.name.as_str())?;
    t.raw_set("equippedSetItemCount", worn)?;
    t.raw_set("maxSetItemCount", set.items.len())?;
    let pieces = lua.create_table()?;
    for item in &set.items {
        let p = lua.create_table()?;
        p.raw_set("item_name", game.items.get(item).map_or("", |i| i.named.name.as_str()))?;
        p.raw_set("equipped", state.worn.contains(item))?;
        pieces.raw_push(p)?;
    }
    t.raw_set("setItems", pieces)?;
    let bonuses = lua.create_table()?;
    for (needed, buff) in &set.bonuses {
        let b = lua.create_table()?;
        b.raw_set("numPieces", *needed)?;
        b.raw_set("satisfied", worn as i32 >= *needed)?;
        let text = game.tips.buff_text(*buff, level, &state.attrs).or_else(|| game.buffs.get(buff).map(|b| b.desc.clone())).unwrap_or_default();
        b.raw_set("bufDesc", text)?;
        bonuses.raw_push(b)?;
    }
    t.raw_set("bonuses", bonuses)?;
    Ok(Some(t))
}

/// The info of a skill as `slot:GetTooltip` / `X2Skill:GetSkillTooltip` return it; `level` is the rank the
/// player has learned (1 when unknown).
pub fn skill_tip(lua: &Lua, game: &GameData, state: &TipState, id: u32, level: u8) -> mlua::Result<Option<Table>> {
    let Some(info) = game.skills.get(&id) else { return Ok(None) };
    let t = lua.create_table()?;
    t.raw_set("tipType", "skill")?;
    t.raw_set("type", id)?;
    t.raw_set("name", info.named.name.as_str())?;
    t.raw_set("path", info.named.icon.as_str())?;
    t.raw_set("ability", ABILITIES[usize::from(info.ability).min(ABILITIES.len() - 1)])?;
    t.raw_set("skillLevel", level)?;
    t.raw_set("mana", game.tips.mana(id, &state.attrs).unwrap_or(info.mana))?;
    t.raw_set("casting", info.casting_ms)?;
    t.raw_set("channeling", info.channeling_ms)?;
    // a skill without a range is used on oneself (the script prints "Self" when there is no `minRange`)
    if info.max_range > 0 {
        t.raw_set("minRange", info.min_range)?;
        t.raw_set("maxRange", info.max_range)?;
    }
    t.raw_set("cooldown", info.cooldown_ms)?;
    let radius = game.tips.skill_radius(id);
    if radius > 0 {
        t.raw_set("targetAreaRadius", radius)?;
    }
    let desc = game.tips.skill_text(id, level, &state.attrs);
    t.raw_set("description", desc.as_deref().unwrap_or(info.desc.as_str()))?;
    Ok(Some(t))
}

/// A buff's description with its numbers filled in (the buff tooltips); `None` when it has no placeholders.
pub fn buff_text(game: &GameData, state: &TipState, id: u32, ab_level: i32) -> Option<String> {
    game.tips.buff_text(id, ab_level, &state.attrs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};

    #[test]
    fn native_item_lookup_keeps_raw_grade_and_distinct_crafting_classes() -> Result<(), Box<dyn std::error::Error>> {
        let mut game = GameData::default();
        game.items.insert(
            42,
            super::super::data::ItemInfo {
                impl_id: 11,
                ..Default::default()
            },
        );
        game.grades.insert(0, ("Crude".into(), "FF888888".into()));
        game.grades.insert(1, ("Basic".into(), "FFFFFFFF".into()));
        let data = super::super::api::HudData::default();
        assert!(data.game.0.set(game).is_ok());
        let bridge = Rc::new(RefCell::new(data));
        let lua = Lua::new();
        let api = lua.create_table()?;
        api.set("GetItemInfoByType", lua.create_function(|_, _: mlua::MultiValue| Ok(None::<Table>))?)?;
        lua.globals().set("X2Item", api)?;
        install_native_lookup(&lua, &bridge)?;
        lua.load(
            r#"
          local basic=X2Item:GetItemInfoByType(42,0)
          local crude=X2Item:GetItemInfoByType(42,1)
          assert(basic.itemGrade==0 and basic.grade=='Basic' and basic.gradeColor=='FFFFFFFF')
          assert(crude.itemGrade==1 and crude.grade=='Crude' and crude.gradeColor=='FF888888')
          assert(basic.item_impl=='summon_mate' and basic.stack==nil)
          assert(X2Item:GetItemInfoByType(43,0)==nil)
        "#,
        )
        .exec()?;
        let data = bridge.borrow();
        let game = data.game.get().unwrap();
        let bag = item_tip(&lua, game, &data.tips, 42, Some(1), Some(7), Instance::default())?.unwrap();
        assert_eq!(bag.get::<String>("grade")?, "Basic");
        assert_eq!(bag.get::<u8>("itemGrade")?, 1);
        assert_eq!(bag.get::<i32>("stack")?, 7);
        assert_ne!(item_impl(3), item_impl(5));
        assert_ne!(item_impl(11), item_impl(22));
        assert_eq!(item_impl(21), "socket");
        assert_eq!(item_impl(30), "invalid item_impl");
        Ok(())
    }
}
