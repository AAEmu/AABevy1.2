//! Native Lua table conversion: only filters2..59 are exposed; hidden metadata stays in the copied record.
use super::{integer, model::Tab, numeric};
use mlua::{Lua, Table, Value};

pub(super) fn rgba(lua: &Lua, color: [f32; 4]) -> mlua::Result<Table> {
    let out = lua.create_table()?;
    for (key, value) in ["r", "g", "b", "a"].into_iter().zip(color) {
        out.raw_set(key, f64::from(value))?;
    }
    Ok(out)
}

pub(super) fn info(lua: &Lua, tab: &Tab, complete_palette: bool) -> mlua::Result<Table> {
    let out = lua.create_table()?;
    out.raw_set("id", f64::from(tab.id as f32))?;
    out.raw_set("name", tab.name.as_str())?;
    out.raw_set("fontSize", f64::from(tab.font as f32))?;
    out.raw_set("bgColor", rgba(lua, tab.background)?)?;
    let filters = lua.create_table()?;
    let colors = lua.create_table()?;
    let range = if complete_palette { 0..=199 } else { 2..=59 };
    for id in range {
        filters.raw_set(id, tab.filters[id as usize])?;
        colors.raw_set(id, rgba(lua, tab.colors[id as usize])?)?;
    }
    out.raw_set("filters", filters)?;
    out.raw_set("filterColors", colors)?;
    Ok(out)
}

///39458870 requires its six container/scalar fields, while retaining absent/wrong-kind optional components.
pub(super) fn update(table: Table, mut old: Tab) -> mlua::Result<Option<Tab>> {
    let Some(id) = integer(&table.raw_get::<Value>("id")?) else {
        return Ok(None);
    };
    let Value::String(name) = table.raw_get::<Value>("name")? else {
        return Ok(None);
    };
    let Some(font) = integer(&table.raw_get::<Value>("fontSize")?) else {
        return Ok(None);
    };
    let Value::Table(bg) = table.raw_get::<Value>("bgColor")? else {
        return Ok(None);
    };
    let Value::Table(filters) = table.raw_get::<Value>("filters")? else {
        return Ok(None);
    };
    let Value::Table(colors) = table.raw_get::<Value>("filterColors")? else {
        return Ok(None);
    };
    let bytes = name.as_bytes();
    let bytes = bytes.split(|&b| b == 0).next().unwrap_or_default();
    let Ok(name) = std::str::from_utf8(bytes) else { return Ok(None) };
    // Native destination is81bytes. Refuse a truncated UTF-8 sequence rather than constructing invalid Rust text.
    if bytes.len() > 80 {
        return Ok(None);
    }
    old.id = id;
    old.name = name.into();
    old.font = font;
    for (i, key) in ["r", "g", "b", "a"].into_iter().enumerate() {
        if let Some(value) = numeric(&bg.raw_get::<Value>(key)?) {
            old.background[i] = value;
        }
    }
    for id in 2..=59 {
        if let Value::Boolean(value) = filters.raw_get::<Value>(id)? {
            old.filters[id as usize] = value;
        }
        if let Value::Table(row) = colors.raw_get::<Value>(id)? {
            let mut color = [0.; 4];
            for (i, key) in ["r", "g", "b", "a"].into_iter().enumerate() {
                let Some(value) = numeric(&row.raw_get::<Value>(key)?) else {
                    return Ok(None);
                };
                color[i] = value;
            }
            old.colors[id as usize] = color;
        }
    }
    Ok(Some(old))
}
