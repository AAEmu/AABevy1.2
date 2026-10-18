//! Native row tooltip notifications (392a19c0 / 3929f730), without a tooltip timer.
use super::*;

pub(super) fn methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    m.raw_set(
        "ShowTooltip",
        method(lua, |lua, id, a| {
            let enabled = a.b(1, false);
            change(lua, id, |d| {
                d.tooltip = enabled;
                ((), false)
            });
            none()
        })?,
    )?;
    Ok(())
}

pub(super) fn update(lua: &Lua, id: Id) {
    let event = {
        let mut u = ui(lua);
        let Some(d) = data(&u, id) else { return };
        let Some(index) = d.hover else { return };
        if !d.tooltip || d.last_tooltip == Some(index) {
            return;
        }
        let Some((row, _)) = d.visible_rows().get(index).copied() else {
            return;
        };
        let (text, top, inset, epoch) = (row.text.clone(), d.top, d.state_inset, d.epoch);
        let Some((inner, height, _)) = metrics(&mut u, id) else {
            return;
        };
        let x = (inner.right() - 5.0 - inset[2]) * u.ui_scale;
        let y = (inner.y + (index as f32 - top as f32 + 1.0) * height - inset[3]) * u.ui_scale;
        (text, x, y, index, epoch)
    };
    let (text, x, y, index, epoch) = event;
    // XlWideToMulti copies to a 512-byte callback buffer and reserves the final NUL.
    // It may stop inside a UTF-8 sequence; Lua strings retain those native raw bytes.
    let bytes = text.as_bytes();
    let text = lua.create_string(&bytes[..bytes.len().min(511)]).expect("row text");
    call_handler(
        lua,
        id,
        "OnTooltip",
        vec![
            mlua::Value::String(text),
            mlua::Value::Number(x as f64),
            mlua::Value::Number(y as f64),
        ],
    );
    change(lua, id, |d| {
        // Native commits the current row after the callback; replacement storage must not
        // inherit the copied row's tooltip state.
        if d.epoch == epoch {
            d.last_tooltip = Some(index)
        }
        ((), false)
    });
}

pub(crate) fn leave(lua: &Lua, id: Id) {
    if data(&ui(lua), id).is_none() {
        return;
    }
    change(lua, id, |d| {
        let changed = d.hover.take().is_some();
        d.last_tooltip = None;
        ((), changed)
    });
    call_handler(lua, id, "OnTooltip", vec![]);
}
