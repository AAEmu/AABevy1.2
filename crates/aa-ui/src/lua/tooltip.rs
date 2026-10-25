//! `gametooltip` widgets: the client's tooltip text box. The scripts add lines one by one and lay
//! the tooltip out from their heights:
//!
//! * `AddLine(text, fontPath, fontSize, column, align, indent [, ratio])` adds a line and returns
//!   its 0-based index; `fontPath` `""` and `fontSize` 0 take the widget's text style; `indent`
//!   moves it right (45: the item and skill names next to their icon); `column` (`"left"` /
//!   `"right"`) selects the native column; `ratio` (default 1) sets its fraction of the inner width.
//! * `AddAnotherSideLine(index, text, fontPath, fontSize, align, indent [, ratio])` puts a second
//!   text on the rows of line `index` (the sell price's money, a skill's cooldown).
//! * `AttachUpperSpaceLine(index, px)` / `AttachLowerSpaceLine(index, px)` set the space above /
//!   below a line (the last call wins).
//! * `GetHeightToLastLine()` returns `(onlyLine, includeSpace)`: the height of all lines so far,
//!   without and with the last line's lower space (both include the top inset, not the bottom).
//!   The tooltip scripts size their
//!   sections from the differences (`reference/ingame/tooltip_*`: a 13 px row is 16 high, the
//!   rows of a wrapped line have no extra space, an empty line has no rows).
//! * `GetLineCount()` counts rows (wrapped lines), `GetLastLine()` the last line's index.
//!
//! The widget sizes itself to its lines: the height is the insets plus the lines; with
//! `SetAutoWordwrap(false)` (the simple text tooltips) the width follows the widest left plus right columns,
//! otherwise the lines wrap at the width minus the insets and the indent. The text is drawn in
//! the widget's style colour, with the lines' `|c` colours.

use mlua::{Lua, Table};

use super::{Ui, method, none, ret, ui};
use crate::render::Quad;
use crate::tree::{Align, Id, Rect, TextBlock};

/// One `AddLine` line.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TipLine {
    pub left: TipText,
    /// Native right column, populated by either AddLine or AddAnotherSideLine.
    pub right: Option<TipText>,
    pub upper: f32,
    pub lower: f32,
}

#[derive(Clone, Debug, PartialEq)]
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
    pub ratio: f32,
}

impl Default for TipText {
    fn default() -> Self {
        Self {
            text: String::new(),
            raw: String::new(),
            font: None,
            size: None,
            align: Align::TopLeft,
            indent: 0.0,
            ratio: 1.0,
        }
    }
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
        .map(|i| {
            t.raw_get::<Option<mlua::Value>>(i)
                .ok()
                .flatten()
                .and_then(|v| match v {
                    mlua::Value::String(s) => s.to_str().ok().map(|s| s.to_string()),
                    mlua::Value::Integer(n) => Some(n.to_string()),
                    mlua::Value::Number(n) => Some(format!("{}", n as i64)),
                    _ => None,
                })
        })
        .collect()
}

/// Re-substitute the placeholders of a tooltip bound to a unit; refits when a text changed.
pub(crate) fn refresh_unit(lua: &Lua, id: Id) {
    let unit = {
        let u = ui(lua);
        match u
            .tree
            .widget(id)
            .and_then(|w| w.tip.as_ref())
            .and_then(|t| t.unit.clone())
        {
            Some(unit) => unit,
            None => return,
        }
    };
    let values = unit_values(lua, &unit);
    // compare first: `widget_mut` counts as a change of the tree, which must not happen every frame
    let changed: Vec<(usize, bool, String)> = {
        let u = ui(lua);
        let Some(tip) = u.tree.widget(id).and_then(|w| w.tip.as_ref()) else {
            return;
        };
        let mut changed = Vec::new();
        for (i, l) in tip.lines.iter().enumerate() {
            for (right, t) in
                std::iter::once((false, &l.left)).chain(l.right.as_ref().map(|t| (true, t)))
            {
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
        let Some(tip) = u.tree.widget_mut(id).and_then(|w| w.tip.as_mut()) else {
            return;
        };
        for (i, right, text) in changed {
            let Some(l) = tip.lines.get_mut(i) else {
                continue;
            };
            if let Some(t) = if right {
                l.right.as_mut()
            } else {
                Some(&mut l.left)
            } {
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
                u.tree
                    .widget(id)
                    .and_then(|w| w.tip.as_ref())
                    .is_some_and(|t| t.unit.is_some())
                    && u.tree.visible(id)
            })
            .collect()
    };
    for id in ids {
        refresh_unit(lua, id);
    }
}

/// Where the lines go, in units from the widget's content top (below the top inset).
struct Placed {
    /// Per line: (text top, rows height, left/right rich-layout row counts).
    lines: Vec<(f32, f32, [usize; 2])>,
    total: f32,
    /// Sum of the widest left and right columns, including their indents.
    width: f32,
}

fn string(a: &super::Args, i: usize) -> Option<String> {
    let mlua::Value::String(s) = a.v(i) else {
        return None;
    };
    Some(
        s.to_string_lossy()
            .split('\0')
            .next()
            .unwrap_or_default()
            .to_string(),
    )
}

fn integer(a: &super::Args, i: usize) -> Option<i32> {
    if !a.is_num(i) {
        return None;
    }
    let v = a.f(i);
    // Original typed parser39058d10: f32 CVTTSS2SI, including integer-indefinite.
    Some(if (-2147483648.0..2147483648.0).contains(&v) {
        v as i32
    } else {
        i32::MIN
    })
}

fn tip_text(
    a: &super::Args,
    text: usize,
    font: usize,
    size: usize,
    align: usize,
    indent: usize,
) -> Option<TipText> {
    let font = string(a, font)?;
    let size = integer(a, size)?;
    let align = match integer(a, align)? {
        4 => Align::Top,
        5 => Align::TopRight,
        _ => Align::TopLeft,
    };
    let indent = integer(a, indent)?;
    let raw = string(a, text)?;
    Some(TipText {
        text: raw.clone(),
        raw,
        font: (!font.is_empty()).then_some(font),
        size: (size > 0).then_some(size as f32),
        align,
        indent: if align == Align::TopLeft {
            indent as f32
        } else {
            0.0
        },
        //391421c0 accepts only the numeric variant; failed/missing optional arg retains 1.
        ratio: if a.is_num(7) { a.f(7) } else { 1.0 },
    })
}

/// The text block a line's text is drawn and measured with.
fn block(base: &TextBlock, t: &TipText, scale: f32) -> TextBlock {
    let mut b = base.clone();
    b.text = t.text.clone();
    if let Some(f) = &t.font {
        b.style.font = f.clone();
    }
    if let Some(s) = t.size {
        b.style.size = s;
    }
    //39299ac8 /3929961b request the integer physical font size before laying out.
    b.style.size = (b.style.size * scale).trunc() / scale;
    b.style.align = t.align;
    b.inset = [0.0; 4];
    b.lines = Vec::new();
    b.auto_resize = false;
    b
}

/// (width, height, rows) of a line's text at `maxw`.
fn measure(u: &mut Ui, b: &TextBlock, maxw: Option<f32>, scale: f32) -> (f32, f32, usize) {
    if b.text.is_empty() {
        return (0.0, 0.0, 0);
    }
    let layout = u.fonts.layout(
        &b.style.font,
        b.style.size * scale,
        &b.text,
        maxw.map(|w| w * scale),
        b.line_space * scale,
        false,
    );
    // Native side admission tests the populated rich row vectors, not source
    // bytes. Use our painter's parsed glyph/image content: formatting alone
    // does not populate a row, even when its source string is nonempty.
    if layout.glyphs.is_empty() && layout.icons.is_empty() && layout.images.is_empty() {
        // Explicit hard breaks create empty native rows (39273890->39271570),
        // unlike colour formatting. CRLF is consumed as one break at392744bf.
        let expanded = crate::text::expand_codes(&b.text);
        let mut chars = expanded.chars().peekable();
        let mut rows = 0_usize;
        while let Some(ch) = chars.next() {
            if ch == '\r' {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                rows += 1;
            } else if ch == '\n' {
                rows += 1;
            }
        }
        let height =
            rows as f32 * layout.line_height + rows.saturating_sub(1) as f32 * b.line_space * scale;
        return (0.0, height / scale, rows);
    }
    (
        layout.width / scale,
        layout.height / scale,
        layout.line_widths.len(),
    )
}

/// Native draw392989d1..39298a5e aligns in the full ratio extent. The rich-text
/// wrapper39273b20 subtracts one physical pixel only from the wrapping limit.
fn column_rect(inner: Rect, t: &TipText, right: bool, wrap: bool, scale: f32) -> Rect {
    let extent = inner.w * t.ratio;
    let span = extent - t.indent;
    let width = if wrap {
        (span - 1.0 / scale).max(1.0 / scale)
    } else {
        span.max(0.0)
    };
    let x = match t.align {
        Align::TopRight => {
            (if right {
                inner.right()
            } else {
                inner.x + extent
            }) - width
        }
        Align::Top => {
            (if right {
                inner.right() - extent * 0.5
            } else {
                inner.x + extent * 0.5
            }) - width * 0.5
        }
        _ => inner.x + if right { inner.w - extent } else { 0.0 } + t.indent,
    };
    Rect {
        x,
        w: width,
        ..inner
    }
}

fn place(u: &mut Ui, id: Id) -> Option<Placed> {
    // Native +68 is local scale times the parent's retained scale. An
    // ApplyUIScale(false) root/ancestor need not follow the global UI scale.
    let scale = u.animations.widget_scale(&u.tree, id);
    let w = u.tree.widget(id)?;
    let lines = w.tip.as_ref()?.lines.clone();
    let base = w.text.clone().unwrap_or_default();
    let width = u.tree.rect(id).w;
    let inner = Rect {
        w: width - base.inset[0] - base.inset[2],
        ..Default::default()
    };
    let mut p = Placed {
        lines: Vec::with_capacity(lines.len()),
        total: 0.0,
        width: 0.0,
    };
    let mut y = 0.0;
    let mut column_widths = [0.0_f32; 2];
    for l in &lines {
        y += l.upper;
        let mut h: f32 = 0.0;
        let mut rows = [0; 2];
        for (side, t) in std::iter::once(&l.left).chain(l.right.as_ref()).enumerate() {
            let b = block(&base, t, scale);
            let rect = column_rect(inner, t, side == 1, base.wrap, scale);
            let (tw, th, tr) = measure(u, &b, base.wrap.then_some(rect.w), scale);
            h = h.max(th);
            rows[side] = tr;
            column_widths[side] = column_widths[side].max(t.indent + tw);
        }
        p.lines.push((y, h, rows));
        y += h;
        y += l.lower;
    }
    p.total = y;
    p.width = column_widths.into_iter().sum();
    Some(p)
}

/// Size the widget to its lines (height; width too without word wrap).
fn fit(lua: &Lua, id: Id) {
    let mut g = ui(lua);
    let u = &mut *g;
    let Some(p) = place(u, id) else {
        return;
    };
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

/// Snapshot native effective scales before a global scale mutation. Hidden
/// tooltips participate;39299f14 skips an empty line deque.
pub(super) fn scale_snapshot(u: &Ui) -> Vec<(Id, f32)> {
    let mut ids = u.tree.of_class("gametooltip");
    ids.extend(u.tree.of_class("unitframetooltip"));
    ids.into_iter()
        .filter(|&id| u.tree.widget(id).is_some_and(has_lines))
        .map(|id| (id, u.animations.widget_scale(&u.tree, id)))
        .collect()
}

/// Native +164->39299ee0 rebuilds populated lines at their new +68 scale.
/// Frozen roots/ancestors retain it, including any manual height override.
pub(super) fn refit_scale(lua: &Lua, before: &[(Id, f32)]) {
    for &(id, previous) in before {
        let changed = {
            let u = ui(lua);
            u.tree.get(id).alive
                && u.tree.widget(id).is_some_and(has_lines)
                && u.animations.widget_scale(&u.tree, id) != previous
        };
        if changed {
            fit(lua, id);
        }
    }
}

fn with_lines<R>(lua: &Lua, id: Id, f: impl FnOnce(&mut Vec<TipLine>) -> R) -> Option<R> {
    let mut u = ui(lua);
    let w = u.tree.widget_mut(id)?;
    Some(f(&mut w.tip.get_or_insert_with(TooltipData::default).lines))
}

/// The lines' text quads, drawn where the widget's own text would be.
pub(crate) fn quads(u: &mut Ui, id: Id, rect: Rect, alpha: f32, global_scale: f32, out: &mut Vec<Quad>) {
    let scale = u.animations.widget_scale(&u.tree, id);
    let Some(p) = place(u, id) else { return };
    let Some(w) = u.tree.widget(id) else { return };
    let lines = w.tip.as_ref().map(|t| t.lines.clone()).unwrap_or_default();
    let base = w.text.clone().unwrap_or_default();
    let inner_w = rect.w - base.inset[0] - base.inset[2];
    let start = out.len();
    for (l, &(y, h, _)) in lines.iter().zip(&p.lines) {
        for (side, t) in std::iter::once(&l.left).chain(l.right.as_ref()).enumerate() {
            if t.text.is_empty() {
                continue;
            }
            let mut b = block(&base, t, scale);
            b.wrap = base.wrap;
            let r = column_rect(
                Rect {
                    x: rect.x + base.inset[0],
                    y: rect.y + base.inset[1] + y,
                    w: inner_w,
                    h,
                },
                t,
                side == 1,
                base.wrap,
                scale,
            );
            let c = b.style.color;
            crate::render::text_quads(u, &b, r, c, alpha, scale, out);
        }
    }
    if scale != global_scale {
        // Rasterize/wrap at native +68, then return local geometry to the
        // painter's global coordinate space. Its existing ancestor transforms
        // apply the effective scale once, preserving physical glyph rasters.
        let to_global = global_scale / scale;
        for q in &mut out[start..] {
            q.pos = q.pos.map(|v| v * to_global);
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
            let Some(text) = tip_text(a, 1, 2, 3, 5, 6) else {
                return none();
            };
            let Some(column) = string(a, 4) else {
                return none();
            };
            let index = with_lines(lua, id, |l| {
                let mut line = TipLine::default();
                if column == "right" {
                    line.right = Some(text);
                } else {
                    line.left = text;
                }
                l.push(line);
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
            let Some(index) = integer(a, 1) else {
                return none();
            };
            let Some(text) = tip_text(a, 2, 3, 4, 5, 6) else {
                return none();
            };
            let Some(index) = usize::try_from(index).ok() else {
                return none();
            };
            let side = {
                let mut u = ui(lua);
                place(&mut u, id).and_then(|p| {
                    p.lines.get(index).and_then(|line| {
                        if line.2[0] > 0 {
                            Some(true)
                        } else if line.2[1] > 0 {
                            Some(false)
                        } else {
                            None
                        }
                    })
                })
            };
            let Some(right) = side else { return none() };
            with_lines(lua, id, |l| {
                if let Some(line) = l.get_mut(index) {
                    if right {
                        line.right = Some(text);
                    } else {
                        line.left = text;
                    }
                }
            });
            refresh_unit(lua, id);
            fit(lua, id);
            none()
        })?,
    )?;
    for (name, lower) in [
        ("AttachUpperSpaceLine", false),
        ("AttachLowerSpaceLine", true),
    ] {
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
            let g = ui(lua);
            let inset = g.tree.get(id).text().map_or([0.0; 4], |t| t.inset);
            let last = g
                .tree
                .widget(id)
                .and_then(|w| w.tip.as_ref())
                .and_then(|tip| tip.lines.last());
            //39296740 reads resolved height, not a fresh text layout. Both
            // outputs include the top inset, even when no lines exist.
            let (only, incl) = if let Some(last) = last {
                let incl = g.tree.rect(id).h - inset[3];
                (incl - last.lower, incl)
            } else {
                (inset[1], inset[1])
            };
            ret(lua, (only, incl))
        })?,
    )?;
    def(
        "GetLineCount",
        method(lua, |lua, id, _| {
            let mut g = ui(lua);
            let rows: usize = place(&mut g, id)
                .map_or(0, |p| p.lines.iter().map(|l| l.2[0].max(l.2[1])).sum());
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
    for name in [
        "SetAutoWordwrap",
        "SetInset",
        "SetLineSpace",
        "SetWidth",
        "SetExtent",
    ] {
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
