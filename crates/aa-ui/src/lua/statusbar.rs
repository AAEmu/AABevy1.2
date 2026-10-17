//! `statusbar` widgets (HP/MP bars, the exp and labor bars): a bar texture
//! (`SetBarTexture(path, layer)` + `SetBarTextureCoords(x, y, w, h)`, tinted by `SetBarColor`)
//! filled to `(value - min) / (max - min)` of the widget. Like the client the texture is cropped,
//! not squeezed: a half full bar shows the left half of its texture rectangle.

use mlua::{Lua, Table, Value};

use super::{method, none, new_object_table, ret, ui};
use crate::tree::{Anchor, DrawKind, DrawLayer, Id, Point};

#[derive(Clone, Debug, PartialEq)]
pub struct StatusBarData {
    pub min: f32,
    pub max: f32,
    pub value: f32,
    pub vertical: bool,
    /// The bar drawable (created by `SetBarTexture`).
    pub bar: Option<Id>,
    pub coords: Option<[f32; 4]>,
    pub color: [f32; 4],
}

impl Default for StatusBarData {
    fn default() -> Self {
        StatusBarData {
            min: 0.0,
            max: 1.0,
            value: 0.0,
            vertical: false,
            bar: None,
            coords: None,
            color: [1.0; 4],
        }
    }
}

impl StatusBarData {
    pub fn fraction(&self) -> f32 {
        if self.max > self.min {
            ((self.value - self.min) / (self.max - self.min)).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

fn with<R>(lua: &Lua, id: Id, f: impl FnOnce(&mut StatusBarData) -> R) -> Option<R> {
    let mut u = ui(lua);
    let w = u.tree.widget_mut(id)?;
    Some(f(w.status.get_or_insert_with(StatusBarData::default)))
}

/// Push the bar state into its drawable (fill fraction, coordinates, colour).
fn sync(lua: &Lua, id: Id) {
    let mut g = ui(lua);
    let u = &mut *g;
    let Some(s) = u.tree.widget(id).and_then(|w| w.status.clone()) else {
        return;
    };
    let Some(bar) = s.bar else { return };
    if let Some(d) = u.tree.drawable_mut(bar) {
        d.fill = Some((s.fraction(), s.vertical));
        d.coords = s.coords;
        d.color = s.color;
    }
}

pub(crate) fn methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    let def = |name: &str, f: mlua::Function| m.raw_set(name, f);
    def(
        "SetBarTexture",
        method(lua, |lua, id, a| {
            let path = a.s(1);
            let layer = DrawLayer::parse(&a.s(2).unwrap_or_else(|| "artwork".into()));
            let existing = with(lua, id, |s| s.bar).flatten();
            let bar = match existing {
                Some(b) => b,
                None => {
                    let mut u = ui(lua);
                    let b = u
                        .tree
                        .create_drawable(id, DrawKind::Image, "drawable:image", layer);
                    for point in [Point::TopLeft, Point::BottomRight] {
                        u.tree.add_anchor(
                            b,
                            Anchor {
                                point,
                                target: id,
                                rel: point,
                                x: 0.0,
                                y: 0.0,
                            },
                        );
                    }
                    drop(u);
                    with(lua, id, |s| s.bar = Some(b));
                    b
                }
            };
            if let Some(d) = ui(lua).tree.drawable_mut(bar) {
                d.texture = path;
                d.layer = layer;
            }
            sync(lua, id);
            none()
        })?,
    )?;
    def(
        "SetBarTextureCoords",
        method(lua, |lua, id, a| {
            let c = [a.f(1), a.f(2), a.f(3), a.f(4)];
            with(lua, id, |s| s.coords = Some(c));
            sync(lua, id);
            none()
        })?,
    )?;
    def(
        "SetBarColor",
        method(lua, |lua, id, a| {
            let c = [a.f(1), a.f(2), a.f(3), a.opt_f(4).unwrap_or(1.0)];
            with(lua, id, |s| s.color = c);
            sync(lua, id);
            none()
        })?,
    )?;
    def(
        "SetMinMaxValues",
        method(lua, |lua, id, a| {
            let (lo, hi) = (a.f(1), a.f(2));
            with(lua, id, |s| {
                s.min = lo;
                s.max = hi;
                s.value = s.value.clamp(lo.min(hi), hi.max(lo));
            });
            sync(lua, id);
            none()
        })?,
    )?;
    def(
        "GetMinMaxValues",
        method(lua, |lua, id, _| {
            let (lo, hi) = with(lua, id, |s| (s.min, s.max)).unwrap_or((0.0, 1.0));
            ret(lua, (lo, hi))
        })?,
    )?;
    // (value, animate, ms): the bar jumps to the value (no fill animation).
    def(
        "SetValue",
        method(lua, |lua, id, a| {
            let v = a.f(1);
            with(lua, id, |s| s.value = v.clamp(s.min.min(s.max), s.max.max(s.min)));
            sync(lua, id);
            none()
        })?,
    )?;
    def(
        "GetValue",
        method(lua, |lua, id, _| {
            ret(lua, with(lua, id, |s| s.value).unwrap_or(0.0))
        })?,
    )?;
    // "HORIZONTAL" / "VERTICAL" (or 0 / 1).
    def(
        "SetOrientation",
        method(lua, |lua, id, a| {
            let vertical = match a.v(1) {
                Value::String(s) => s.to_string_lossy().eq_ignore_ascii_case("vertical"),
                _ => a.f(1) as i64 == 1,
            };
            with(lua, id, |s| s.vertical = vertical);
            sync(lua, id);
            none()
        })?,
    )?;
    def(
        "GetBarTexture",
        method(lua, |lua, id, _| {
            match with(lua, id, |s| s.bar).flatten() {
                Some(b) => ret(lua, new_object_table(lua, b)?),
                None => none(),
            }
        })?,
    )?;
    Ok(())
}
