//! The owned mate's received items and native NPC slot pack. Cursor/transactions are separate.
use super::*;
use mlua::Function;

const SLOTS: [i32; 4] = [1, 3, 4, 7];

fn matches(data: &HudData, pet: &Pet, unit: &str) -> bool {
    unit == "playerpet"
        || unit == pet.object.to_string()
        || (unit == "target"
            && data
                .panels
                .target
                .as_ref()
                .is_some_and(|target| target.obj_id == pet.object))
}

fn equipable(tables: &Tables, pet: &Pet, slot: i32) -> bool {
    SLOTS
        .iter()
        .position(|candidate| *candidate == slot)
        .and_then(|index| tables.equipment.get(&pet.template).map(|pack| pack[index]))
        .unwrap_or(false)
}

pub(super) fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    let equipment: Table = lua.globals().get("X2Equipment")?;
    let state = bridge.clone();
    equipment.set(
        "IsMateEquipableSlot",
        lua.create_function(move |_, (_, unit, slot): (Value, String, i32)| {
            let data = state.borrow();
            Ok(data
                .pets
                .owned
                .current
                .as_ref()
                .filter(|pet| matches(&data, pet, &unit))
                .zip(data.game.get())
                .is_some_and(|(pet, game)| equipable(&game.pets.owned, pet, slot)))
        })?,
    )?;
    let previous: Function = equipment.get("GetEquippedItemTooltipText")?;
    let state = bridge.clone();
    equipment.set(
        "GetEquippedItemTooltipText",
        lua.create_function(move |lua, (this, unit, slot): (Value, String, i32)| {
            let data = state.borrow();
            if unit == "player"
                || (!data.snapshot.unit_id.is_empty() && unit == data.snapshot.unit_id)
            {
                // Release the shared borrow before invoking the existing player getter.
                drop(data);
                return previous.call::<MultiValue>((this, unit, slot));
            }
            let Some(pet) = data
                .pets
                .owned
                .current
                .as_ref()
                .filter(|pet| matches(&data, pet, &unit))
            else {
                return Ok(MultiValue::new());
            };
            let Some(index) = slot
                .checked_sub(1)
                .and_then(|slot| usize::try_from(slot).ok())
                .filter(|index| *index < aa_net::model::slot::COUNT)
            else {
                return Ok(MultiValue::new());
            };
            let Some(item) = &pet.equipment.slots[index] else {
                let out = lua.create_table()?;
                out.set("itemType", 0)?;
                out.set("lookType", 0)?;
                out.set("itemGrade", 0)?;
                return Ok(MultiValue::from_iter([Value::Table(out)]));
            };
            let detail = match &item.detail {
                ItemDetail::Equip(detail) => Some(detail),
                _ => None,
            };
            // Actual Pet Gear has no set rows in this catalog; this shares the existing static
            // item-tooltip calculations, not the player's equipped-slot or attribute fallback.
            let result = if let Some(game) = data.game.get() {
                super::super::super::tips::item_tip(
                    lua,
                    game,
                    &data.tips,
                    item.template_id,
                    Some(i64::from(super::super::super::store::ui_grade(item.grade))),
                    Some(i64::from(item.count)),
                    super::super::super::tips::Instance {
                        durability: detail.map(|detail| detail.durability),
                        temper: detail.map_or((0, 0), |detail| {
                            (detail.temper_physical, detail.temper_magical)
                        }),
                        flags: Some(item.flags),
                    },
                )?
            } else {
                None
            };
            let out = match result {
                Some(out) => out,
                None => {
                    let out = lua.create_table()?;
                    out.set("itemType", item.template_id)?;
                    out.set("lookType", item.template_id)?;
                    out.set(
                        "itemGrade",
                        super::super::super::store::ui_grade(item.grade),
                    )?;
                    out
                }
            };
            out.set("equip", true)?;
            Ok(MultiValue::from_iter([Value::Table(out)]))
        })?,
    )
}

pub(super) fn push(rt: &UiRuntime, old: Option<&Pet>, next: Option<&Pet>, identity_changed: bool) {
    if !identity_changed
        && old
            .zip(next)
            .is_none_or(|(old, next)| old.equipment == next.equipment)
    {
        return;
    }
    if next.is_none() {
        return;
    }
    // The original handler owns icon, lock, durability and hover-tooltip presentation.
    if let Err(error) = rt
        .lua
        .load("if petBar and petBar.equip then petBar.equip:OnEquipmentChanged() end")
        .set_name("@owned_pet_equipment_refresh")
        .exec()
    {
        warn!("hud: pet equipment refresh: {error}");
    }
}

pub(super) fn slots(data: &HudData, event: &Event) -> Vec<i64> {
    let Some(pet) = &data.pets.owned.current else {
        return Vec::new();
    };
    match event {
        Event::EquipmentChanged { obj_id, items } if *obj_id == pet.object => items
            .iter()
            .filter(|(slot, _)| usize::from(*slot) < aa_net::model::slot::COUNT)
            .map(|(slot, _)| i64::from(*slot) + 1)
            .collect(),
        Event::Mate(aa_net::mate::MateEvent::EquipmentFlagsChanged {
            mate_tl_id,
            container,
            slot,
            ..
        }) if *mate_tl_id == pet.spawn.tl_id && *container == 0xfc && *slot <= 27 => {
            vec![i64::from(*slot) + 1]
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests;
