//! `circlediagram` widgets: a filled polygon (the skillset diagram of character create step 3).
//! `AddPoint(x, y)` adds a vertex at its full value (widget-local position), `SetMaxValue(max)`
//! sets the full value and `SetPointValue(i, v)` moves vertex `i` to `v / max` of the way from
//! the widget centre; the polygon is filled with `SetDiagramColor(r, g, b, a)`.

use mlua::{Lua, Table};

use super::{method, none, ret, ui};
use crate::tree::Id;

#[derive(Clone, Debug, Default)]
pub struct DiagramData {
    pub color: [f32; 4],
    pub max: f32,
    /// Vertices at the full value, widget-local.
    pub points: Vec<(f32, f32)>,
    pub values: Vec<f32>,
}

impl DiagramData {
    /// Current vertex `i`, widget-local, for a widget of `w` x `h`.
    pub fn vertex(&self, i: usize, w: f32, h: f32) -> (f32, f32) {
        let (cx, cy) = (w / 2.0, h / 2.0);
        let (px, py) = self.points[i];
        let k = if self.max > 0.0 {
            (self.values.get(i).copied().unwrap_or(0.0) / self.max).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (cx + (px - cx) * k, cy + (py - cy) * k)
    }
}

fn with<R>(lua: &Lua, id: Id, f: impl FnOnce(&mut DiagramData) -> R) -> Option<R> {
    let mut u = ui(lua);
    let w = u.tree.widget_mut(id)?;
    Some(f(w.diagram.get_or_insert_with(|| DiagramData {
        color: [1.0; 4],
        max: 1.0,
        ..Default::default()
    })))
}

pub(crate) fn methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    let def = |name: &str, f: mlua::Function| m.raw_set(name, f);
    def(
        "SetDiagramColor",
        method(lua, |lua, id, a| {
            let c = [a.f(1), a.f(2), a.f(3), a.opt_f(4).unwrap_or(1.0)];
            with(lua, id, |d| d.color = c);
            none()
        })?,
    )?;
    def(
        "SetMaxValue",
        method(lua, |lua, id, a| {
            let v = a.f(1);
            with(lua, id, |d| d.max = v);
            none()
        })?,
    )?;
    def(
        "AddPoint",
        method(lua, |lua, id, a| {
            let p = (a.f(1), a.f(2));
            with(lua, id, |d| {
                d.points.push(p);
                d.values.push(0.0);
            });
            none()
        })?,
    )?;
    def(
        "ClearPoints",
        method(lua, |lua, id, _| {
            with(lua, id, |d| {
                d.points.clear();
                d.values.clear();
            });
            none()
        })?,
    )?;
    def(
        "SetPointValue",
        method(lua, |lua, id, a| {
            let i = a.f(1) as usize;
            let v = a.f(2);
            with(lua, id, |d| {
                if (1..=d.values.len()).contains(&i) {
                    d.values[i - 1] = v;
                }
            });
            none()
        })?,
    )?;
    def(
        "GetCenterOffset",
        method(lua, |lua, id, _| {
            let r = ui(lua).tree.rect(id);
            ret(lua, (r.w / 2.0, r.h / 2.0))
        })?,
    )?;
    def(
        "GetPointOffset",
        method(lua, |lua, id, a| {
            let i = a.f(1) as usize;
            let r = ui(lua).tree.rect(id);
            let v = with(lua, id, |d| {
                (1..=d.points.len())
                    .contains(&i)
                    .then(|| d.vertex(i - 1, r.w, r.h))
            })
            .flatten();
            match v {
                Some((x, y)) => ret(lua, (x, y)),
                None => none(),
            }
        })?,
    )?;
    def("SetLineThickness", method(lua, |_, _, _| none())?)?;
    Ok(())
}
