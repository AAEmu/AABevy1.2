use super::{super::api::Shared, controller as c};
use mlua::{Lua, MultiValue, ObjectLike, Table, Value};

fn number(value: Option<&Value>) -> Option<i32> {
    let value = match value? {
        Value::Integer(n) => *n as f32,
        Value::Number(n) => *n as f32,
        _ => return None,
    };
    Some(
        if value.is_finite() && (-2147483648.0..2147483648.0).contains(&value) {
            value as i32
        } else {
            i32::MIN
        },
    )
}
fn boolean(value: bool) -> MultiValue {
    MultiValue::from_iter([Value::Boolean(value)])
}
fn table(value: Option<Table>) -> MultiValue {
    value
        .map(|t| MultiValue::from_iter([Value::Table(t)]))
        .unwrap_or_default()
}
fn select(lua: &Lua, state: &Shared, index: usize, slot: u8) -> mlua::Result<bool> {
    let result = c::select(&mut state.borrow_mut(), index, slot);
    match result {
        Ok(()) => c::update(lua),
        Err(code) if code != 0 => super::super::enchant::selection_error(lua, code)?,
        _ => {}
    }
    Ok(result.is_ok())
}
pub(in crate::hud) fn install(lua: &Lua, state: &Shared) -> mlua::Result<()> {
    let api: Table = lua.globals().get("X2ItemLookConverter")?;
    for (name, active) in [
        ("EnterItemConvertMode", true),
        ("LeaveItemConvertMode", false),
    ] {
        let state = state.clone();
        api.raw_set(
            name,
            lua.create_function(move |lua, _: MultiValue| {
                c::switch(lua, &state, active);
                Ok(())
            })?,
        )?;
    }
    for (index, set_name, pick_name, get_name, clear_name) in [
        (
            0,
            "SetConvertBaseItemSlotFromBag",
            "SetConvertBaseItemSlotFromPick",
            "GetConvertBaseItemInfo",
            "ClearConvertBaseItemSlot",
        ),
        (
            1,
            "SetConvertLookItemSlotFromBag",
            "SetConvertLookItemSlotFromPick",
            "GetConvertLookItemInfo",
            "ClearConvertLookItemSlot",
        ),
    ] {
        let s = state.clone();
        api.raw_set(
            set_name,
            lua.create_function(move |lua, args: MultiValue| {
                let Some(slot) = number(args.get(1)).map(|n| n.wrapping_sub(1) as u8) else {
                    return Ok(MultiValue::new());
                };
                Ok(boolean(select(lua, &s, index, slot)?))
            })?,
        )?;
        let s = state.clone();
        api.raw_set(
            pick_name,
            lua.create_function(move |lua, _: MultiValue| {
                let cursor: Table = lua.globals().get("X2Cursor")?;
                let held: Option<Table> = cursor.call_method("GetHeld", ())?;
                let Some(held) = held else {
                    return Ok(MultiValue::new());
                };
                if held.raw_get::<Option<String>>("kind")?.as_deref() != Some("bag") {
                    return Ok(MultiValue::new());
                }
                let Some(slot) =
                    number(Some(&held.raw_get::<Value>("slot")?)).map(|n| n.wrapping_sub(1) as u8)
                else {
                    return Ok(MultiValue::new());
                };
                let count = number(Some(&held.raw_get::<Value>("count")?));
                let admitted = {
                    let data = s.borrow();
                    !super::super::enchant::bag_pending(&data, slot)
                        && c::item(&data, slot).is_some_and(|item| Some(item.count) == count)
                };
                if !admitted {
                    return Ok(MultiValue::new());
                }
                let selected = c::select(&mut s.borrow_mut(), index, slot);
                if let Err(code) = selected
                    && code != 0
                {
                    super::super::enchant::selection_error(lua, code)?;
                }
                cursor.call_method::<()>("ClearCursor", ())?;
                if selected.is_ok() {
                    c::update(lua);
                }
                Ok(boolean(selected.is_ok()))
            })?,
        )?;
        let s = state.clone();
        api.raw_set(
            get_name,
            lua.create_function(move |lua, _: MultiValue| Ok(table(c::info(lua, &s, index)?)))?,
        )?;
        let s = state.clone();
        api.raw_set(
            clear_name,
            lua.create_function(move |lua, _: MultiValue| {
                c::clear(lua, &s, index);
                Ok(())
            })?,
        )?;
    }
    let s = state.clone();
    api.raw_set(
        "GetConvertResultItemInfo",
        lua.create_function(move |lua, _: MultiValue| Ok(table(c::preview(lua, &s)?)))?,
    )?;
    let s = state.clone();
    api.raw_set(
        "GetItemLookConvertInfo",
        lua.create_function(move |lua, _: MultiValue| Ok(table(c::plan(lua, &s)?)))?,
    )?;
    let s = state.clone();
    api.raw_set(
        "ConvertItemLook",
        lua.create_function(move |lua, _: MultiValue| c::submit(lua, &s))?,
    )?;
    let item: Table = lua.globals().get("X2Item")?;
    let s = state.clone();
    item.raw_set(
        "IsInItemLookConvertMode",
        lua.create_function(move |_, _: MultiValue| Ok(s.borrow().look_conversion.active))?,
    )?;
    let s = state.clone();
    item.raw_set(
        "IsLookConvertableItem",
        lua.create_function(move |lua, args: MultiValue| {
            let Some(slot) = number(args.get(1)).map(|n| n.wrapping_sub(1)) else {
                return Ok(MultiValue::new());
            };
            if slot < 0 {
                aa_ui::lua::fire_ui_event(
                    lua,
                    "UI_ERROR_MESSAGE",
                    vec![Value::String(lua.create_string("invalid item slot index")?)],
                );
                return Ok(boolean(false));
            }
            Ok(boolean(c::eligible(&s.borrow(), slot as u8)))
        })?,
    )?;
    let s = state.clone();
    item.raw_set(
        "IsSkinizableItem",
        lua.create_function(move |lua, args: MultiValue| {
            let Some(slot) = number(args.get(1)).map(|n| n.wrapping_sub(1)) else {
                return Ok(MultiValue::new());
            };
            if slot < 0 {
                aa_ui::lua::fire_ui_event(
                    lua,
                    "UI_ERROR_MESSAGE",
                    vec![Value::String(lua.create_string("invalid item slot index")?)],
                );
                return Ok(boolean(false));
            }
            let data = s.borrow();
            Ok(boolean(
                data.game
                    .get()
                    .zip(c::item(&data, slot as u8))
                    .is_some_and(|(g, item)| g.look_conversion.skinizable(item)),
            ))
        })?,
    )?;
    super::target::install(lua, state)
}
