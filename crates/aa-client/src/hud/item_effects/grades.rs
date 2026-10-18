//! Native39480f20/39480fc0: strict numeric argument, low-byte raw grade lookup.
use super::Shared;
use mlua::{Lua, MultiValue, Table, Value};

fn grade(value: Option<&Value>) -> Option<u8> {
    let number = match value? {
        Value::Integer(value) => *value as f32,
        Value::Number(value) => *value as f32,
        _ => return None,
    };
    let integer = if number.is_finite() && (-2147483648.0..2147483648.0).contains(&number) {
        number as i32
    } else {
        i32::MIN
    };
    Some(integer as u8)
}
pub(in crate::hud) fn install(lua: &Lua, state: &Shared) -> mlua::Result<()> {
    let item: Table = lua.globals().get("X2Item")?;
    for (name, color) in [("GradeName", false), ("GradeColor", true)] {
        let state = state.clone();
        item.raw_set(
            name,
            lua.create_function(move |lua, args: MultiValue| {
                let Some(raw) = grade(args.get(1)) else {
                    return Ok(MultiValue::new());
                };
                let data = state.borrow();
                let value = data
                    .game
                    .get()
                    .and_then(|game| game.grades.get(&i64::from(super::super::store::ui_grade(raw))));
                let Some((name, argb)) = value else { return Ok(MultiValue::new()) };
                Ok(MultiValue::from_iter([Value::String(lua.create_string(if color {
                    argb
                } else {
                    name
                })?)]))
            })?,
        )?;
    }
    Ok(())
}
