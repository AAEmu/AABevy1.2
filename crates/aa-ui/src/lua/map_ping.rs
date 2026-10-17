//! Shared native ping widget placement. The app owns an optional server echo; without it,
//! the world map keeps its local ping behavior.

use mlua::{Function, IntoLuaMulti, Lua, Table, Value};

use super::{object_table, queue_ui_event, ui};
use crate::tree::{Anchor, Id, Point};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ping {
    pub level: i64,
    pub id: i64,
    pub u: f32,
    pub v: f32,
    pub maps: [Option<(i64, i64, f32, f32)>; 4],
    /// Optional server sequence: a repeated ping at the same point can restart its pulse.
    pub version: Option<i64>,
}

impl Ping {
    pub(super) fn on_map(self, level: i64, id: i64) -> Option<(f32, f32)> {
        if (self.level, self.id) == (level, id) {
            Some((self.u, self.v))
        } else {
            self.maps
                .iter()
                .flatten()
                .find_map(|&(l, i, u, v)| ((l, i) == (level, id)).then_some((u, v)))
        }
    }
}

/// An omitted `ping` leaves local state alone; false clears it; a table is an authoritative
/// point. Invalid data leaves the last good echo in place.
pub(super) fn read(table: Option<&Table>) -> mlua::Result<Option<Option<Ping>>> {
    let Some(table) = table else { return Ok(None) };
    let ping = match table.raw_get::<Value>("ping")? {
        Value::Nil => return Ok(None),
        Value::Boolean(false) => return Ok(Some(None)),
        Value::Table(ping) => ping,
        _ => return Err(mlua::Error::runtime("ping must be false or a map point")),
    };
    let mut point = Ping {
        level: ping.raw_get("level")?,
        id: ping.raw_get("id")?,
        u: ping.raw_get("u")?,
        v: ping.raw_get("v")?,
        maps: [None; 4],
        version: ping.raw_get("version")?,
    };
    if !point.u.is_finite() || !point.v.is_finite() {
        return Err(mlua::Error::runtime("ping coordinates must be finite"));
    }
    if let Some(maps) = ping.raw_get::<Option<Table>>("maps")? {
        for (slot, map) in point.maps.iter_mut().zip(maps.sequence_values::<Table>()) {
            let map = map?;
            let (u, v): (f32, f32) = (map.raw_get("u")?, map.raw_get("v")?);
            if !u.is_finite() || !v.is_finite() {
                return Err(mlua::Error::runtime("ping map coordinates must be finite"));
            }
            *slot = Some((map.raw_get("level")?, map.raw_get("id")?, u, v));
        }
    }
    Ok(Some(Some(point)))
}

/// True means the app owns the request. Failure also consumes it so a network/API failure
/// cannot display an unconfirmed marker.
pub(super) fn request(lua: &Lua, name: &str, args: impl IntoLuaMulti) -> bool {
    let function = lua
        .globals()
        .raw_get::<Table>("AAUI")
        .ok()
        .and_then(|t| t.raw_get::<Option<Function>>(name).ok().flatten());
    let Some(function) = function else {
        return false;
    };
    match function.call::<Value>(args) {
        Ok(Value::Boolean(true)) => true,
        Ok(_) => false,
        Err(e) => {
            bevy::log::warn!("aa-ui: AAUI.{name}: {e}");
            true
        }
    }
}

/// Place the script's frame on the map image. Show/Hide and the pulse go through the original
/// handlers (`ping.lua` / `ping_view.lua`); unchanged echoes do not touch the tree.
pub(super) fn apply(lua: &Lua, frame: Id, map: Id, uv: Option<(f32, f32)>, restart: bool) {
    let (shown, changed) = {
        let mut u = ui(lua);
        let shown = u.tree.get(frame).shown;
        let mut changed = false;
        if let Some((pu, pv)) = uv {
            let (mw, mh) = u.tree.size(map);
            let o = u.tree.get(frame);
            let anchor = Anchor {
                point: Point::TopLeft,
                target: map,
                rel: Point::TopLeft,
                x: (pu * mw - o.width / 2.0).round(),
                y: (pv * mh - o.height / 2.0).round(),
            };
            if o.anchors.as_slice() != [anchor.clone()] {
                u.tree.get_mut(frame).anchors = vec![anchor];
                changed = true;
            }
        }
        (shown, changed)
    };
    let want = uv.is_some();
    let Ok(Some(table)) = object_table(lua, frame) else {
        return;
    };
    if shown != want
        && let Ok(show) = table.get::<Function>("Show")
    {
        let _ = show.call::<()>((table.clone(), want));
    }
    if shown != want || (want && (restart || changed)) {
        queue_ui_event(
            lua,
            "SET_EFFECT_ICON_VISIBLE",
            vec![Value::Boolean(want), Value::Table(table)],
        );
    }
}
