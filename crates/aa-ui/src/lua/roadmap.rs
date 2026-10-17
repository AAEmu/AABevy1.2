//! `roadmap` widgets (the zone map overlay at the top right of the HUD) and a few small natives
//! of other classes (`listbox` tree images).
//!
//! The scripts make one hidden image drawable per zone map and size
//! (`map/road_map_list.lua`: `ui/map/road/<zone>_road_<50|70|100>.dds`, placed at its offset in
//! the roadmap) and register it with `roadmap:SaveMapData(level, "<zone>_<size>", drawable)`;
//! `SetMapSize(50|70|100)` picks the size and `SetPlayerDrawable(d)` the player cursor. The
//! native shows the current zone's drawable, raises `UPDATE_ROADMAP_ANCHOR(level, key)` when it
//! changes (the scripts re-anchor the window) and moves the cursor. The current zone and the
//! player's place come from the app, asked every frame:
//!
//! ```lua
//! AAUI.RoadMap() -> nil (no map) or {
//!   level = 1,                       -- 1 zone maps, 2 city maps (RMS_ZONE / RMS_CITY + 1)
//!   zone = 5,                        -- the map key's zone part ("5_100": Solzreed Peninsula)
//!   player = { u = 0.85, v = 0.27 }, -- optional: position on the map image, 0..1
//! }
//! ```

use mlua::{Function, Lua, Table, Value};

use super::{method, new_object_table, none, ret, ui, value_id};
use crate::tree::{Anchor, DrawKind, DrawLayer, Id, Point};

#[derive(Clone, Debug)]
pub struct RoadMapData {
    pub map_size: i64,
    /// (level, key, drawable) from `SaveMapData`.
    pub maps: Vec<(i64, String, Id)>,
    pub player: Option<Id>,
    /// The shown map (level, key).
    pub current: Option<(i64, String)>,
    /// Player position last applied (u, v).
    pub last_player: Option<(f32, f32)>,
}

impl Default for RoadMapData {
    fn default() -> Self {
        RoadMapData {
            map_size: 100,
            maps: Vec::new(),
            player: None,
            current: None,
            last_player: None,
        }
    }
}

fn hook(lua: &Lua) -> Option<Function> {
    let aaui: Table = lua.globals().raw_get("AAUI").ok()?;
    aaui.raw_get("RoadMap").ok()
}

fn with<R>(lua: &Lua, id: Id, f: impl FnOnce(&mut RoadMapData) -> R) -> Option<R> {
    let mut u = ui(lua);
    let w = u.tree.widget_mut(id)?;
    Some(f(w.roadmap.get_or_insert_with(Default::default)))
}

fn data(lua: &Lua, id: Id) -> RoadMapData {
    ui(lua)
        .tree
        .widget(id)
        .and_then(|w| w.roadmap.clone())
        .unwrap_or_default()
}

/// Ask the hook for every live roadmap: show the zone's map, move the player cursor.
pub(crate) fn update(lua: &Lua) {
    let Some(f) = hook(lua) else { return };
    let maps: Vec<Id> = {
        let u = ui(lua);
        u.tree
            .objects
            .iter()
            .enumerate()
            .filter(|(_, o)| o.alive && o.widget().is_some_and(|w| w.class == "roadmap"))
            .map(|(i, _)| i as Id)
            .collect()
    };
    if maps.is_empty() {
        return;
    }
    let r = match f.call::<Value>(()) {
        Ok(Value::Table(t)) => Some(t),
        Ok(_) => None,
        Err(e) => {
            bevy::log::warn!("aa-ui: AAUI.RoadMap: {e}");
            return;
        }
    };
    for id in maps {
        if let Err(e) = apply(lua, id, r.as_ref()) {
            bevy::log::warn!("aa-ui: AAUI.RoadMap result: {e}");
        }
    }
}

fn apply(lua: &Lua, id: Id, r: Option<&Table>) -> mlua::Result<()> {
    let d = data(lua, id);
    let (want, player) = match r {
        Some(r) => {
            let level = r.raw_get::<Option<i64>>("level")?.unwrap_or(1);
            let zone = r.raw_get::<Option<i64>>("zone")?;
            let player = match r.raw_get::<Option<Table>>("player")? {
                Some(p) => Some((p.raw_get::<f32>("u")?, p.raw_get::<f32>("v")?)),
                None => None,
            };
            (zone.map(|z| (level, format!("{z}_{}", d.map_size))), player)
        }
        None => (None, None),
    };
    let shown_map = want
        .as_ref()
        .and_then(|(l, k)| d.maps.iter().find(|(ml, mk, _)| ml == l && mk == k))
        .map(|m| m.2);
    if want != d.current {
        {
            let mut u = ui(lua);
            for (_, _, m) in &d.maps {
                u.tree.get_mut(*m).shown = Some(*m) == shown_map;
            }
        }
        with(lua, id, |x| {
            x.current = want.clone();
            x.last_player = None;
        });
        if let Some((level, key)) = &want {
            let args = vec![Value::Integer(*level), Value::String(lua.create_string(key)?)];
            super::fire_ui_event(lua, "UPDATE_ROADMAP_ANCHOR", args);
        }
    }
    let Some(p) = d.player else { return Ok(()) };
    let place = match (shown_map, player) {
        (Some(m), Some(uv)) => Some((m, uv)),
        _ => None,
    };
    if place.map(|x| x.1) == data(lua, id).last_player && place.is_some() {
        return Ok(());
    }
    let mut u = ui(lua);
    match place {
        Some((m, (pu, pv))) => {
            let (mw, mh) = (u.tree.get(m).width, u.tree.get(m).height);
            let o = u.tree.get_mut(p);
            let (w, h) = (o.width, o.height);
            o.anchors = vec![Anchor {
                point: Point::TopLeft,
                target: m,
                rel: Point::TopLeft,
                x: (pu * mw - w / 2.0).round(),
                y: (pv * mh - h / 2.0).round(),
            }];
            o.shown = true;
        }
        None => u.tree.get_mut(p).shown = false,
    }
    drop(u);
    with(lua, id, |x| x.last_player = place.map(|x| x.1));
    Ok(())
}

pub(crate) fn methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    let def = |name: &str, f: mlua::Function| m.raw_set(name, f);
    def(
        "SetPlayerDrawable",
        method(lua, |lua, id, a| {
            let p = value_id(lua, &a.v(1));
            // Hidden until the app places it (`AAUI.RoadMap`).
            if let Some(p) = p {
                ui(lua).tree.get_mut(p).shown = false;
            }
            with(lua, id, |d| {
                d.player = p;
                d.last_player = None;
            });
            none()
        })?,
    )?;
    def(
        "SaveMapData",
        method(lua, |lua, id, a| {
            let level = a.f(1) as i64;
            let key = a.s(2).unwrap_or_default();
            if let Some(d) = value_id(lua, &a.v(3)) {
                with(lua, id, |x| {
                    x.maps.retain(|(l, k, _)| !(*l == level && *k == key));
                    x.maps.push((level, key, d));
                    x.current = None;
                });
            }
            none()
        })?,
    )?;
    def(
        "SetMapSize",
        method(lua, |lua, id, a| {
            let s = a.f(1).round() as i64;
            with(lua, id, |x| {
                x.map_size = s;
                x.current = None;
            });
            none()
        })?,
    )?;
    def(
        "GetTooltipController",
        method(lua, |lua, id, _| {
            let parts = super::chat::parts(lua, id)?;
            if let Some(t) = parts.raw_get::<Option<Table>>("tooltipController")? {
                return ret(lua, t);
            }
            let t = super::widget::create_child(
                lua,
                Some(id),
                "emptywidget",
                "tooltipController",
                0,
                false,
            )?;
            if let Some(c) = value_id(lua, &Value::Table(t.clone())) {
                ui(lua).tree.get_mut(c).shown = false;
            }
            parts.raw_set("tooltipController", &t)?;
            ret(lua, t)
        })?,
    )?;
    for name in ["UpdateAllDrawableAnchor", "SetTooltipColor", "ReloadAllInfo"] {
        def(name, method(lua, |_, _, _| none())?)?;
    }
    Ok(())
}

/// `listbox` tree images (`CreateOpenedImageDrawable(path)`, closed, separator): image drawables
/// the native list places on tree items; kept hidden (tree lists are not drawn yet).
pub(crate) fn listbox_methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    for name in [
        "CreateOpenedImageDrawable",
        "CreateClosedImageDrawable",
        "CreateSeparatorImageDrawable",
    ] {
        m.raw_set(
            name,
            method(lua, |lua, id, a| {
                let d = {
                    let mut u = ui(lua);
                    let d = u.tree.create_drawable(
                        id,
                        DrawKind::Image,
                        "drawable:image",
                        DrawLayer::parse(&a.s(2).unwrap_or_else(|| "artwork".into())),
                    );
                    if let Some(dr) = u.tree.drawable_mut(d) {
                        dr.texture = a.s(1);
                    }
                    u.tree.get_mut(d).shown = false;
                    d
                };
                ret(lua, new_object_table(lua, d)?)
            })?,
        )?;
    }
    Ok(())
}
