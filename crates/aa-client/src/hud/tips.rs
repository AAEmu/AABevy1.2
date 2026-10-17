//! The tooltip info tables the client's tooltip scripts read (`baselib/tooltip/tooltip.lua`): `HUD.item_tip`,
//! `HUD.skill_tip`. The shapes are the captured ones of `aa-ui/examples/fake/fake_bag.lua` /
//! `fake_ingame.lua` (`reference/ingame/tooltip_{item,skill,buff}`), filled from the game database. They
//! are built when a slot asks for its content or a tooltip shows, not every frame.

use mlua::{Lua, Table};

use super::data::GameData;

/// The tooltip's `item_impl` of an `items.impl_id` (`ItemImplEnum`). The equipment and portal ones have
/// sections of their own; everything else is "etc" (a plain item).
fn item_impl(id: i64) -> &'static str {
    match id {
        1 => "weapon",
        2 => "armor",
        14 => "portal",
        24 => "accessory",
        _ => "etc",
    }
}

/// `skills.ability_id` -> the key of the tooltip's ability colour and name (`locale.common.abilityCategory`).
const ABILITIES: [&str; 11] = ["general", "fight", "illusion", "adamant", "will", "death", "wild", "magic", "vocation", "romance", "love"];

/// The info of an item as `X2Bag:GetBagItemInfo` returns it: `grade` is the number the server and the icons
/// use (1 basic, 2 grand ...; 0 takes the item's fixed grade or basic), `count` the stack.
pub fn item_tip(lua: &Lua, game: &GameData, id: u32, grade: Option<i64>, count: Option<i64>) -> mlua::Result<Option<Table>> {
    let Some(info) = game.items.get(&id) else { return Ok(None) };
    let grade = match grade.unwrap_or(0) {
        0 if info.fixed_grade > 0 => info.fixed_grade,
        0 => 1,
        g => g,
    };
    let t = lua.create_table()?;
    t.raw_set("itemType", id)?;
    t.raw_set("lookType", id)?;
    t.raw_set("itemGrade", grade)?;
    if let Some((name, color)) = game.grades.get(&grade) {
        t.raw_set("grade", name.as_str())?;
        t.raw_set("gradeColor", color.as_str())?;
    }
    t.raw_set("name", info.named.name.as_str())?;
    t.raw_set("category", info.category.as_str())?;
    t.raw_set("item_impl", item_impl(info.impl_id))?;
    if info.soul_bound {
        t.raw_set("soul_bound", 1)?;
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
    t.raw_set("refund", if info.refund > 0 { info.refund } else { info.price })?;
    t.raw_set("sellable", info.sellable && info.price > 0)?;
    Ok(Some(t))
}

/// The info of a skill as `slot:GetTooltip` / `X2Skill:GetSkillTooltip` return it; `level` is the rank the
/// player has learned (1 when unknown).
pub fn skill_tip(lua: &Lua, game: &GameData, id: u32, level: u8) -> mlua::Result<Option<Table>> {
    let Some(info) = game.skills.get(&id) else { return Ok(None) };
    let t = lua.create_table()?;
    t.raw_set("tipType", "skill")?;
    t.raw_set("type", id)?;
    t.raw_set("name", info.named.name.as_str())?;
    t.raw_set("path", info.named.icon.as_str())?;
    t.raw_set("ability", ABILITIES[usize::from(info.ability).min(ABILITIES.len() - 1)])?;
    t.raw_set("skillLevel", level)?;
    t.raw_set("mana", info.mana)?;
    t.raw_set("casting", info.casting_ms)?;
    t.raw_set("channeling", info.channeling_ms)?;
    t.raw_set("minRange", info.min_range)?;
    t.raw_set("maxRange", info.max_range)?;
    t.raw_set("cooldown", info.cooldown_ms)?;
    t.raw_set("description", info.desc.as_str())?;
    Ok(Some(t))
}
