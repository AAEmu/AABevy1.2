//! Native numeric argument/field selection; the existing HUD countdown remains a compatibility clock.
use super::super::api::Shared;
use mlua::{Lua, MultiValue, Table, Value};

fn integer(value: Option<&Value>) -> Option<i32> {
    let number = match value? {
        Value::Integer(number) => *number as f32,
        Value::Number(number) => *number as f32,
        _ => return None,
    };
    Some(if number.is_finite() && (-2147483648.0..2147483648.0).contains(&number) {
        number as i32
    } else {
        i32::MIN
    })
}
pub(in crate::hud) fn install(lua: &Lua, hud: &Table, state: &Shared) -> mlua::Result<()> {
    hud.raw_set(
        "buff_fields",
        lua.create_function(|_, args: MultiValue| {
            let Some(index) = integer(args.front()) else {
                return Ok(MultiValue::new());
            };
            let index = index.wrapping_sub(1);
            if index < 0 {
                return Ok(MultiValue::new());
            }
            let flags = if args.len() == 1 { -1 } else { integer(args.get(1)).unwrap_or(0) };
            Ok(MultiValue::from_iter([
                crate::lua_int(i64::from(index) + 1),
                Value::Boolean(flags == -1),
                Value::Boolean(flags & 1 != 0),
                Value::Boolean(flags & 2 != 0),
                Value::Boolean(flags & 4 != 0),
                Value::Boolean(flags & 8 != 0),
                Value::Boolean(flags & 16 != 0),
            ]))
        })?,
    )?;
    let state = state.clone();
    hud.raw_set(
        "buff_mine",
        lua.create_function(move |_, caster: u32| {
            let id = state.borrow().character_id;
            Ok(id != 0 && id == caster)
        })?,
    )?;
    hud.raw_set("buff_float", lua.create_function(|_, value: f64| Ok(f64::from(value as f32)))?)
}

#[cfg(test)]
mod tests;
