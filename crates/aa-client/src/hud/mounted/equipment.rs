//! Original customizing/slot queries and SC068's notification-only gear path.
use std::collections::{HashMap, HashSet};

use aa_net::{
    Event,
    model::{Item, ItemDetail},
    slave::SlaveEvent,
};
use aa_ui::UiRuntime;
use mlua::{Lua, MultiValue, Table, Value};
use rusqlite::Connection;

use super::super::{
    api::{HudData, Shared},
    tips,
};

#[derive(Default)]
pub(super) struct Tables {
    definitions: HashSet<u32>,
    names: HashMap<(u32, usize), String>,
    slots: HashSet<(u32, usize)>,
}

impl Tables {
    pub(super) fn load(db: &Connection) -> rusqlite::Result<Self> {
        let definitions = db
            .prepare("SELECT id FROM slave_customizings")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let mut value = Self {
            definitions,
            ..Default::default()
        };
        let mut query = db.prepare(
            "SELECT s.slave_customizing_id,s.equip_slot_id,coalesce(nullif(l.en_us,''),s.equip_slot_name,'') \
             FROM slave_customizing_equip_slots s LEFT JOIN localized_texts l \
             ON l.tbl_name='slave_customizing_equip_slots' AND l.tbl_column_name='equip_slot_name' AND l.idx=s.id"
        )?;
        for row in query.query_map([], |r| {
            Ok((
                r.get::<_, u32>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, String>(2)?,
            ))
        })? {
            let (kind, slot, name) = row?;
            // The native vector search uses its first matching definition.
            value.names.entry((kind, slot as usize)).or_insert(name);
        }
        let mut query = db.prepare("SELECT slave_id,equip_slot_id FROM slave_equip_slots")?;
        for row in query.query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, u32>(1)?)))? {
            let (template, slot) = row?;
            value.slots.insert((template, slot as usize));
        }
        Ok(value)
    }
}

fn customizing_type(data: &HudData) -> Option<u32> {
    // Native game+1A mask1 = received feature byte6 mask1. Missing stays unknown.
    if data
        .mounted
        .config
        .received
        .as_ref()?
        .feature_bytes
        .get(6)?
        & 1
        == 0
    {
        return None;
    }
    let (_, actor) = data.mounted.selected()?;
    data.game
        .get()?
        .mounted
        .rows
        .get(&actor.slave_template?)?
        .customizing
}

struct SlotInfo {
    name: String,
    has: bool,
    item: Option<Item>,
}

fn selected_slot(data: &HudData, index: usize) -> Option<SlotInfo> {
    if index >= 28 {
        return None;
    }
    let (_, actor) = data.mounted.selected()?;
    let template = actor.slave_template?;
    let tables = &data.game.get()?.mounted;
    let kind = tables.rows.get(&template)?.customizing?;
    if !tables.equipment.definitions.contains(&kind) {
        return None;
    }
    //394ABD90 has no feature-bit gate. Membership, name and current item are
    // independent: an unconfigured slot can still hold a received item.
    Some(SlotInfo {
        name: tables
            .equipment
            .names
            .get(&(kind, index))
            .cloned()
            .unwrap_or_default(),
        has: tables.equipment.slots.contains(&(template, index)),
        item: actor.equipment.slots[index]
            .clone()
            .filter(|item| item.template_id != 0),
    })
}

fn index(args: &MultiValue) -> Option<usize> {
    let number = match args.get(1)? {
        Value::Integer(value) => *value as f32,
        Value::Number(value) => *value as f32,
        _ => return None,
    };
    if !number.is_finite() || !(-2147483648.0..2147483648.0).contains(&number) {
        return None;
    }
    usize::try_from(number as i32)
        .ok()
        .filter(|slot| *slot < 28)
}

fn item_info(
    lua: &Lua,
    game: &super::super::data::GameData,
    state: &tips::TipState,
    item: &Item,
) -> mlua::Result<Option<Table>> {
    let instance = match &item.detail {
        ItemDetail::Equip(d) => tips::Instance {
            durability: Some(d.durability),
            temper: (d.temper_physical, d.temper_magical),
            flags: Some(item.flags),
        },
        _ => tips::Instance {
            flags: Some(item.flags),
            ..Default::default()
        },
    };
    let extra = tips::ItemExtra::of_item(item);
    let value = tips::item_tip_with(
        lua,
        game,
        state,
        item.template_id,
        Some(i64::from(super::super::store::ui_grade(item.grade))),
        Some(i64::from(item.count)),
        instance,
        &extra,
    )?;
    if let Some(value) = &value {
        let appearance = if extra.image == 0 {
            item.template_id
        } else {
            extra.image
        };
        value.raw_set("itemGrade", item.grade)?;
        value.raw_set("lookType", appearance)?;
        value.raw_set("lookChanged", appearance != item.template_id)?;
    }
    Ok(value)
}

pub(super) fn install(lua: &Lua, state: &Shared) -> mlua::Result<()> {
    let namespace: Table = lua.globals().get("X2SiegeWeapon")?;
    let shared = state.clone();
    namespace.set(
        "GetSlaveCustomizingType",
        lua.create_function(move |_, _: MultiValue| Ok(customizing_type(&shared.borrow())))?,
    )?;
    let shared = state.clone();
    namespace.set(
        "GetSlaveEquipSlotInfo",
        lua.create_function(move |lua, args: MultiValue| {
            let Some(index) = index(&args) else {
                return Ok(None);
            };
            // Clone receipt data and tooltip context before allocating Lua values;
            // callbacks/GC cannot observe a held mutable/immutable HudData borrow.
            let snapshot = {
                let data = shared.borrow();
                selected_slot(&data, index).map(|slot| (slot, data.game.clone(), data.tips.clone()))
            };
            let Some((slot, game, tips)) = snapshot else {
                return Ok(None);
            };
            let value = lua.create_table()?;
            value.raw_set("name", slot.name)?;
            value.raw_set("has", slot.has)?;
            if let Some(item) = slot.item {
                if let Some(game) = game.get() {
                    value.raw_set("itemInfo", item_info(lua, game, &tips, &item)?)?;
                }
            }
            Ok(Some(value))
        })?,
    )?;
    Ok(())
}

fn notice_slot(container: u8, slot: u8) -> f64 {
    if matches!(container, 1 | 0xfc | 0xf2) {
        f64::from(slot)
    } else {
        -1.0
    }
}

pub(super) fn notify(rt: Option<&UiRuntime>, event: &Event) {
    if let (Some(rt), Event::Slave(SlaveEvent::EquipmentChanged(change))) = (rt, event) {
        //391D4950->394E1DA0 always emits105, including failure and unrelated
        // headers. It does not copy either report item into slave equipment.
        // Native bag-copy/lock/cursor effects remain with the future action port.
        rt.fire_event(
            "SLAVE_UNIT_EQUIPMENT_CHANGED",
            (
                notice_slot(change.first_location.container, change.first_location.slot),
                notice_slot(
                    change.second_location.container,
                    change.second_location.slot,
                ),
            ),
        );
    }
}

#[cfg(test)]
mod tests;
