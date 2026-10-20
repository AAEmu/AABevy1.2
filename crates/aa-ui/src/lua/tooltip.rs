//! `gametooltip` widgets: the client's tooltip text box. The scripts add lines one by one and lay
//! the tooltip out from their heights:
//!
//! * `AddLine(text, fontPath, fontSize, column, align, indent [, ratio])` adds a line and returns
//!   its 0-based index; `fontPath` `""` and `fontSize` 0 take the widget's text style; `indent`
//!   moves it right (45: the item and skill names next to their icon); `column` (`"left"` /
//!   `"right"`) and `ratio` are taken as the alignment.
//! * `AddAnotherSideLine(index, text, fontPath, fontSize, align, indent [, ratio])` puts a second
//!   text on the rows of line `index` (the sell price's money, a skill's cooldown).
//! * `AttachUpperSpaceLine(index, px)` / `AttachLowerSpaceLine(index, px)` set the space above /
//!   below a line (the last call wins).
//! * `GetHeightToLastLine()` returns `(onlyLine, includeSpace)`: the height of all lines so far,
//!   without and with the last line's lower space (no insets). The tooltip scripts size their
//!   sections from the differences (`reference/ingame/tooltip_*`: a 13 px row is 16 high, the
//!   rows of a wrapped line have no extra space, an empty line has no rows).
//! * `GetLineCount()` counts rows (wrapped lines), `GetLastLine()` the last line's index.
//!
//! The widget sizes itself to its lines: the height is the insets plus the lines; with
//! `SetAutoWordwrap(false)` (the simple text tooltips) the width follows the longest row too,
//! otherwise the lines wrap at the width minus the insets and the indent. The text is drawn in
//! the widget's style colour, with the lines' `|c` colours.

use mlua::{Lua, Table};

use super::{method, none, ret, ui, Ui};
use crate::render::Quad;
use crate::tree::{Align, Id, Rect, TextBlock};

/// One `AddLine` line.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TipLine {
    pub left: TipText,
    /// `AddAnotherSideLine`: a second text on the same rows.
    pub right: Option<TipText>,
    pub upper: f32,
    pub lower: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TipText {
    /// The text drawn: `raw` with the bound unit's `|pN;` placeholders substituted.
    pub text: String,
    /// The text as the script gave it.
    pub raw: String,
    /// `None`: the widget's style font / size.
    pub font: Option<String>,
    pub size: Option<f32>,
    pub align: Align,
    pub indent: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TooltipData {
    pub lines: Vec<TipLine>,
    /// `SetTooltipData(unit)`: the unit token whose live values replace `|pN;` in the lines.
    pub unit: Option<String>,
}

/// Name of the Lua function the client provides: `(unit) -> { [1] = name, [2] = hp, ... }` with
/// the values of the `UFT_*` placeholders `|p1;` .. `|p11;` (nil / missing: leave the code).
const UNIT_VALUES_HOOK: &str = "AAUI_TooltipUnitValues";

fn unit_values(lua: &Lua, unit: &str) -> Vec<Option<String>> {
    let Ok(f) = lua.globals().get::<mlua::Function>(UNIT_VALUES_HOOK) else {
        return Vec::new();
    };
    let Ok(t) = f.call::<Option<Table>>(unit) else {
        return Vec::new();
    };
    let Some(t) = t else { return Vec::new() };
    (1..=11)
        .map(|i| t.raw_get::<Option<mlua::Value>>(i).ok().flatten().and_then(|v| match v {
            mlua::Value::String(s) => s.to_str().ok().map(|s| s.to_string()),
            mlua::Value::Integer(n) => Some(n.to_string()),
            mlua::Value::Number(n) => Some(format!("{}", n as i64)),
            _ => None,
        }))
        .collect()
}

/// Re-substitute the placeholders of a tooltip bound to a unit; refits when a text changed.
pub(crate) fn refresh_unit(lua: &Lua, id: Id) {
    let unit = {
        let u = ui(lua);
        match u.tree.widget(id).and_then(|w| w.tip.as_ref()).and_then(|t| t.unit.clone()) {
            Some(unit) => unit,
            None => return,
        }
    };
    let values = unit_values(lua, &unit);
    // compare first: `widget_mut` counts as a change of the tree, which must not happen every frame
    let changed: Vec<(usize, bool, String)> = {
        let u = ui(lua);
        let Some(tip) = u.tree.widget(id).and_then(|w| w.tip.as_ref()) else { return };
        let mut changed = Vec::new();
        for (i, l) in tip.lines.iter().enumerate() {
            for (right, t) in std::iter::once((false, &l.left)).chain(l.right.as_ref().map(|t| (true, t))) {
                let text = crate::text::substitute_unit_values(&t.raw, &values);
                if text != t.text {
                    changed.push((i, right, text));
                }
            }
        }
        changed
    };
    if changed.is_empty() {
        return;
    }
    {
        let mut u = ui(lua);
        let Some(tip) = u.tree.widget_mut(id).and_then(|w| w.tip.as_mut()) else { return };
        for (i, right, text) in changed {
            let Some(l) = tip.lines.get_mut(i) else { continue };
            if let Some(t) = if right { l.right.as_mut() } else { Some(&mut l.left) } {
                t.text = text;
            }
        }
    }
    fit(lua, id);
}

/// Per frame: the visible unit tooltips follow their unit's values.
pub(crate) fn update_units(lua: &Lua) {
    let ids: Vec<Id> = {
        let u = ui(lua);
        let mut all = u.tree.of_class("gametooltip");
        all.extend(u.tree.of_class("unitframetooltip"));
        all.into_iter()
            .filter(|&id| {
                u.tree.widget(id).and_then(|w| w.tip.as_ref()).is_some_and(|t| t.unit.is_some())
                    && u.tree.visible(id)
            })
            .collect()
    };
    for id in ids {
        refresh_unit(lua, id);
    }
}

/// Wrapped lines break 5 units before the right inset: in reference/ingame/tooltip_item a 204 wide
/// row stays on its line and a 206 wide one wraps, 210 inside the insets.
const WRAP_MARGIN: f32 = 5.0;

/// Where the lines go, in units from the widget's content top (below the top inset).
struct Placed {
    /// Per line: (text top, rows height, rows).
    lines: Vec<(f32, f32, usize)>,
    /// Per line: height without / with its lower space.
    heights: Vec<(f32, f32)>,
    total: f32,
    /// The widest row plus its indent.
    width: f32,
}

fn tip_text(a: &super::Args, text: usize, font: usize, size: usize, align: usize, indent: usize) -> TipText {
    let font = a.s(font).filter(|f| !f.is_empty());
    let size = a.opt_f(size).filter(|s| *s > 0.0);
    // `ALIGN_LEFT` / `ALIGN_CENTER` / `ALIGN_RIGHT` (or the top variants): the row's horizontal part.
    let align = match Align::from_i(a.opt_f(align).unwrap_or(3.0) as i64).fx() {
        0.0 => Align::TopLeft,
        1.0 => Align::TopRight,
        _ => Align::Top,
    };
    let raw = a.s(text).unwrap_or_default();
    TipText {
        text: raw.clone(),
        raw,
        font,
        size,
        align,
        indent: a.f(indent),
    }
}

/// The text block a line's text is drawn and measured with.
fn block(base: &TextBlock, t: &TipText) -> TextBlock {
    let mut b = base.clone();
    b.text = t.text.clone();
    if let Some(f) = &t.font {
        b.style.font = f.clone();
    }
    if let Some(s) = t.size {
        b.style.size = s;
    }
    b.style.align = t.align;
    b.inset = [0.0; 4];
    b.lines = Vec::new();
    b.auto_resize = false;
    b
}

/// (width, height, rows) of a line's text at `maxw`.
fn measure(u: &mut Ui, b: &TextBlock, maxw: Option<f32>) -> (f32, f32, usize) {
    if b.text.is_empty() {
        return (0.0, 0.0, 0);
    }
    let (w, h) = u
        .fonts
        .layout_size(&b.style.font, b.style.size, &b.text, maxw, b.line_space);
    let lh = u.fonts.metrics(&b.style.font, b.style.size).1;
    let rows = ((h + b.line_space) / (lh + b.line_space).max(1.0)).round().max(1.0) as usize;
    (w, h, rows)
}

fn place(u: &mut Ui, id: Id) -> Option<Placed> {
    let w = u.tree.widget(id)?;
    let lines = w.tip.as_ref()?.lines.clone();
    let base = w.text.clone().unwrap_or_default();
    let width = u.tree.rect(id).w;
    let inner = (width - base.inset[0] - base.inset[2]).max(0.0);
    let mut p = Placed {
        lines: Vec::with_capacity(lines.len()),
        heights: Vec::with_capacity(lines.len()),
        total: 0.0,
        width: 0.0,
    };
    let mut y = 0.0;
    for l in &lines {
        y += l.upper;
        let mut h: f32 = 0.0;
        let mut rows = 0;
        let mut row_w: f32 = 0.0;
        for t in std::iter::once(&l.left).chain(l.right.as_ref()) {
            let b = block(&base, t);
            let maxw = base.wrap.then_some((inner - t.indent - WRAP_MARGIN).max(1.0));
            let (tw, th, tr) = measure(u, &b, maxw);
            h = h.max(th);
            rows = rows.max(tr);
            row_w += t.indent + tw;
        }
        p.lines.push((y, h, rows));
        p.width = p.width.max(row_w);
        y += h;
        p.heights.push((y, y + l.lower));
        y += l.lower;
    }
    p.total = y;
    Some(p)
}

/// Size the widget to its lines (height; width too without word wrap).
fn fit(lua: &Lua, id: Id) {
    let mut g = ui(lua);
    let u = &mut *g;
    let Some(p) = place(u, id) else { return };
    let inset = u.tree.get(id).text().map_or([0.0; 4], |t| t.inset);
    let wrap = u.tree.get(id).text().is_some_and(|t| t.wrap);
    let h = inset[1] + p.total + inset[3];
    let w = inset[0] + p.width.ceil() + inset[2];
    let o = u.tree.get(id);
    if o.height == h && (wrap || o.width == w) {
        return;
    }
    let o = u.tree.get_mut(id);
    o.height = h;
    if !wrap {
        o.width = w;
    }
}

fn with_lines<R>(lua: &Lua, id: Id, f: impl FnOnce(&mut Vec<TipLine>) -> R) -> Option<R> {
    let mut u = ui(lua);
    let w = u.tree.widget_mut(id)?;
    Some(f(&mut w.tip.get_or_insert_with(TooltipData::default).lines))
}

/// The lines' text quads, drawn where the widget's own text would be.
pub(crate) fn quads(u: &mut Ui, id: Id, rect: Rect, alpha: f32, scale: f32, out: &mut Vec<Quad>) {
    let Some(p) = place(u, id) else { return };
    let Some(w) = u.tree.widget(id) else { return };
    let lines = w.tip.as_ref().map(|t| t.lines.clone()).unwrap_or_default();
    let base = w.text.clone().unwrap_or_default();
    let inner_w = (rect.w - base.inset[0] - base.inset[2]).max(0.0);
    for (l, &(y, h, _)) in lines.iter().zip(&p.lines) {
        for t in std::iter::once(&l.left).chain(l.right.as_ref()) {
            if t.text.is_empty() {
                continue;
            }
            let mut b = block(&base, t);
            b.wrap = base.wrap;
            if b.wrap {
                b.inset[2] = WRAP_MARGIN;
            }
            let r = Rect {
                x: rect.x + base.inset[0] + t.indent,
                y: rect.y + base.inset[1] + y,
                w: (inner_w - t.indent).max(0.0),
                h,
            };
            let c = b.style.color;
            crate::render::text_quads(u, &b, r, c, alpha, scale, out);
        }
    }
}

/// Whether a widget draws tooltip lines (instead of its own text).
pub(crate) fn has_lines(w: &crate::tree::Widget) -> bool {
    w.tip.as_ref().is_some_and(|t| !t.lines.is_empty())
}

pub(crate) fn methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    let def = |name: &str, f: mlua::Function| m.raw_set(name, f);
    def(
        "AddLine",
        method(lua, |lua, id, a| {
            let left = tip_text(a, 1, 2, 3, 5, 6);
            let index = with_lines(lua, id, |l| {
                l.push(TipLine {
                    left,
                    ..Default::default()
                });
                l.len() as i64 - 1
            })
            .unwrap_or(-1);
            refresh_unit(lua, id);
            fit(lua, id);
            ret(lua, index)
        })?,
    )?;
    def(
        "SetTooltipData",
        method(lua, |lua, id, a| {
            let unit = a.s(1).filter(|s| !s.is_empty());
            {
                let mut u = ui(lua);
                if let Some(w) = u.tree.widget_mut(id) {
                    w.tip.get_or_insert_with(TooltipData::default).unit = unit;
                }
            }
            refresh_unit(lua, id);
            none()
        })?,
    )?;
    def(
        "AddAnotherSideLine",
        method(lua, |lua, id, a| {
            let index = a.f(1) as i64;
            let right = tip_text(a, 2, 3, 4, 5, 6);
            with_lines(lua, id, |l| {
                if let Some(line) = usize::try_from(index).ok().and_then(|i| l.get_mut(i)) {
                    line.right = Some(right);
                }
            });
            refresh_unit(lua, id);
            fit(lua, id);
            none()
        })?,
    )?;
    for (name, lower) in [("AttachUpperSpaceLine", false), ("AttachLowerSpaceLine", true)] {
        def(
            name,
            method(lua, move |lua, id, a| {
                let index = a.f(1) as i64;
                let px = a.f(2);
                with_lines(lua, id, |l| {
                    if let Some(line) = usize::try_from(index).ok().and_then(|i| l.get_mut(i)) {
                        if lower {
                            line.lower = px;
                        } else {
                            line.upper = px;
                        }
                    }
                });
                fit(lua, id);
                none()
            })?,
        )?;
    }
    def(
        "ClearLines",
        method(lua, |lua, id, _| {
            with_lines(lua, id, |l| l.clear());
            fit(lua, id);
            none()
        })?,
    )?;
    def(
        "GetHeightToLastLine",
        method(lua, |lua, id, _| {
            let mut g = ui(lua);
            let (only, incl) = place(&mut g, id)
                .and_then(|p| p.heights.last().copied())
                .unwrap_or((0.0, 0.0));
            ret(lua, (only, incl))
        })?,
    )?;
    def(
        "GetLineCount",
        method(lua, |lua, id, _| {
            let mut g = ui(lua);
            let rows: usize = place(&mut g, id).map_or(0, |p| p.lines.iter().map(|l| l.2).sum());
            ret(lua, rows as i64)
        })?,
    )?;
    def(
        "GetLastLine",
        method(lua, |lua, id, _| {
            let n = with_lines(lua, id, |l| l.len() as i64).unwrap_or(0);
            ret(lua, n - 1)
        })?,
    )?;
    // The text settings change the layout: refit after the generic setter.
    for name in ["SetAutoWordwrap", "SetInset", "SetLineSpace", "SetWidth", "SetExtent"] {
        let generic: mlua::Function = m.raw_get(name)?;
        def(
            name,
            lua.create_function(move |lua, mv: mlua::MultiValue| {
                let this = mv.front().cloned();
                generic.call::<()>(mv)?;
                if let Some(id) = this.and_then(|v| super::value_id(lua, &v)) {
                    fit(lua, id);
                }
                Ok(())
            })?,
        )?;
    }
    Ok(())
}
