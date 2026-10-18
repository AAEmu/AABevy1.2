//! Native flat and recursive listbox rows (3929fc40 / 392a10a0 / 392a22a0).
//! Source-backed row icons, branch images and authored state textures.

mod paint;
mod tree;

use super::{Ui, call_handler, method, none, ret, ui};
use crate::{
    render::{Quad, Tex},
    tree::{Id, Rect, TextBlock},
};
use mlua::{Lua, Table};

#[derive(Clone, Debug, PartialEq)]
struct Row {
    text: String,
    value: i32,
    color: [f32; 4],
    selected: bool,
    opened: bool,
    child: Vec<Row>,
    icon: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ListBoxData {
    rows: Vec<Row>,
    selected: i32,
    hover: Option<usize>,
    top: i32,
    padding: f32,
    text_limit: i32,
    consistent: bool,
    toggle: bool,
    text_colors: [[f32; 4]; 3],
    colors: [[f32; 4]; 3],
    tree_mode: bool,
    select_parent: bool,
    child_style: bool,
    indent: Option<i32>,
    opened_image: Option<Id>,
    closed_image: Option<Id>,
    epoch: u64,
    separator_image: Option<Id>,
    state_texture: Option<String>,
    state_coords: [[f32; 4]; 3],
    state_inset: [f32; 4],
    icons_seen: bool,
}

impl Default for ListBoxData {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            selected: -1,
            hover: None,
            top: 0,
            padding: 0.0,
            text_limit: 0,
            consistent: true,
            toggle: false,
            text_colors: [[1.0; 4]; 3],
            colors: [
                [0.4, 0.4, 0.4, 0.0],
                [0.4, 0.4, 0.4, 0.5],
                [0.4, 0.4, 0.4, 0.5],
            ],
            tree_mode: false,
            select_parent: true,
            child_style: false,
            indent: None,
            opened_image: None,
            closed_image: None,
            epoch: 0,
            separator_image: None,
            state_texture: None,
            state_coords: [[0., 0., 1., 1.]; 3],
            state_inset: [0.; 4],
            icons_seen: false,
        }
    }
}

fn data(u: &Ui, id: Id) -> Option<&ListBoxData> {
    u.tree.widget(id)?.listbox.as_ref()
}

fn change<R>(lua: &Lua, id: Id, f: impl FnOnce(&mut ListBoxData) -> (R, bool)) -> Option<R> {
    let mut u = ui(lua);
    let w = u.tree.widget_mut(id)?;
    if w.ty != crate::tree::WidgetType::ListBox {
        return None;
    }
    let d = w.listbox.get_or_insert_with(Default::default);
    let (result, changed) = f(d);
    if changed {
        u.tree.repaint();
    }
    Some(result)
}

fn metrics(u: &mut Ui, id: Id) -> Option<(Rect, f32, i32)> {
    let w = u.tree.widget(id)?;
    let d = w.listbox.as_ref()?;
    let text = w.text.clone().unwrap_or_default();
    let padding = d.padding;
    let scale = u.ui_scale;
    let font = u.fonts.layout(
        &text.style.font,
        text.style.size * scale,
        "",
        None,
        0.0,
        false,
    );
    let height = font.line_height / scale + padding;
    let r = u.tree.rect(id);
    let [left, top, right, bottom] = text.inset;
    let inner = Rect {
        x: r.x + left,
        y: r.y + top,
        w: (r.w - left - right).max(0.0),
        h: (r.h - top.max(0.0) - bottom.max(0.0)).max(0.0),
    };
    let count = if height > 0.0 {
        (inner.h / height + 0.5).floor() as i32
    } else {
        0
    };
    Some((inner, height, count))
}

fn max_top(u: &mut Ui, id: Id) -> i32 {
    let count = metrics(u, id).map_or(0, |m| m.2);
    (data(u, id).map_or(0, |d| d.opened_count() as i32) - count).max(0)
}

fn row_at(u: &mut Ui, id: Id) -> Option<usize> {
    let (inner, height, _) = metrics(u, id)?;
    if height <= 0.0 {
        return None;
    }
    let base = u.tree.rect(id);
    let view = u.animations.widget_rect(&u.tree, id);
    let y = base.y + (u.cursor.1 - view.y) * base.h / view.h.max(0.001);
    // Native cvttss2si truncates toward zero, including the authored top inset.
    let index = ((y - inner.y) / height).trunc() as i32 + data(u, id)?.top;
    (index >= 0 && index < (data(u, id)?.opened_count() as i32)).then_some(index as usize)
}

fn select(lua: &Lua, id: Id, index: i32, notify_same: bool, clear: bool) {
    let event = change(lua, id, |d| {
        if clear && d.tree_mode {
            return (false, false);
        }
        let old = d.selected;
        let mut dirty = false;
        let valid = index < d.rows.len() as i32;
        if clear {
            for (i, row) in d.rows.iter_mut().enumerate() {
                let selected = valid && i as i32 == index;
                dirty |= row.selected != selected;
                row.selected = selected;
            }
        }
        if !valid {
            return (false, dirty);
        }
        d.selected = index;
        if let Some(row) = usize::try_from(index).ok().and_then(|i| d.rows.get_mut(i)) {
            dirty |= !row.selected;
            row.selected = true;
            if !clear && !row.child.is_empty() {
                row.opened = !row.opened;
                dirty = true;
            }
        }
        let event = notify_same || old != index;
        (event, dirty || old != index)
    })
    .unwrap_or(false);
    if event {
        call_handler(lua, id, "OnSelChanged", vec![]);
    }
}

pub(crate) fn clicked(lua: &Lua, id: Id) {
    let index = {
        let mut u = ui(lua);
        if !u
            .tree
            .widget(id)
            .is_some_and(|w| w.enabled && w.listbox.is_some())
        {
            return;
        }
        row_at(&mut u, id)
    };
    if let Some(index) = index {
        if data(&ui(lua), id).is_some_and(|d| d.tree_mode) {
            tree::clicked(lua, id, index);
            return;
        }
        change(lua, id, |d| {
            let selected = !(d.toggle && d.rows[index].selected);
            let mut dirty = d.selected != index as i32;
            for (i, row) in d.rows.iter_mut().enumerate() {
                let next = i == index && selected;
                dirty |= row.selected != next;
                row.selected = next;
            }
            d.selected = index as i32;
            ((), dirty)
        });
        call_handler(lua, id, "OnSelChanged", vec![]);
    }
}

pub(crate) fn hover(lua: &Lua, old: Option<Id>, new: Option<Id>) {
    if old != new
        && let Some(id) = old
        && data(&ui(lua), id).is_some_and(|d| d.hover.is_some())
    {
        change(lua, id, |d| {
            let changed = d.hover.take().is_some();
            ((), changed)
        });
    }
    let Some(id) = new else { return };
    let row = {
        let mut u = ui(lua);
        let Some(w) = u.tree.widget(id).filter(|w| w.listbox.is_some()) else {
            return;
        };
        if w.enabled { row_at(&mut u, id) } else { None }
    };
    if data(&ui(lua), id).is_some_and(|d| d.hover == row) {
        return;
    }
    change(lua, id, |d| {
        let changed = d.hover != row;
        d.hover = row;
        ((), changed)
    });
}

fn set_top(lua: &Lua, id: Id, requested: i32) {
    let limit = max_top(&mut ui(lua), id);
    let top = requested.clamp(0, limit);
    change(lua, id, |d| {
        let changed = d.top != top;
        d.top = top;
        ((), changed)
    });
    let h = ui(lua).hover;
    if h == Some(id) {
        hover(lua, h, h);
    }
}

pub(crate) fn methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    tree::methods(lua, m)?;
    paint::methods(lua, m)?;
    let def = |name: &str, f: mlua::Function| m.raw_set(name, f);
    def(
        "AppendItem",
        method(lua, |lua, id, a| {
            // Native overloads: text/value[/icon] and text/value/RGBA[/icon].
            if !matches!(a.len(), 3 | 4 | 7 | 8) || !a.is_num(2) {
                return none();
            }
            let icon = match a.len() {
                4 | 8 => {
                    let mlua::Value::String(path) = a.v(a.len() - 1) else {
                        return none();
                    };
                    paint::icon(lua, Some(path.to_string_lossy()))
                }
                _ => None,
            };
            let mlua::Value::String(text) = a.v(1) else {
                return none();
            };
            let text = text.to_string_lossy();
            let value = match a.v(2) {
                mlua::Value::Integer(n) => n as i32,
                mlua::Value::Number(n) => n as i32,
                _ => unreachable!(), // Native numeric argument check above.
            };
            let mut color = ui(lua)
                .tree
                .widget(id)
                .and_then(|w| w.text.as_ref())
                .map_or([1.0; 4], |t| t.style.color);
            if a.len() >= 7 {
                if !(3..=6).all(|i| a.opt_f(i).is_some()) {
                    return none();
                }
                color = [a.f(3), a.f(4), a.f(5), a.f(6)];
            }
            change(lua, id, |d| {
                let mut text: String = text.chars().take_while(|c| *c != '\0').take(511).collect();
                if d.text_limit > 0 && text.chars().count() > d.text_limit as usize {
                    text = text
                        .chars()
                        .take(d.text_limit as usize - 1)
                        .chain("...".chars())
                        .take(511)
                        .collect();
                }
                d.rows.push(Row {
                    text,
                    value,
                    color,
                    selected: false,
                    opened: false,
                    child: Vec::new(),
                    icon,
                });
                d.epoch = d.epoch.wrapping_add(1);
                ((), true)
            });
            none()
        })?,
    )?;
    def(
        "ClearItem",
        method(lua, |lua, id, _| {
            change(lua, id, |d| {
                let dirty = !d.rows.is_empty() || d.selected != -1 || d.top != 0;
                d.rows.clear();
                d.selected = -1;
                d.top = 0;
                d.hover = None;
                d.tree_mode = false;
                d.epoch = d.epoch.wrapping_add(1);
                ((), dirty)
            });
            none()
        })?,
    )?;
    def(
        "ItemCount",
        method(lua, |lua, id, _| {
            ret(lua, data(&ui(lua), id).map_or(0, |d| d.rows.len()))
        })?,
    )?;
    def(
        "GetOpenedItemCount",
        method(lua, |lua, id, _| {
            ret(lua, data(&ui(lua), id).map_or(0, |d| d.opened_count()))
        })?,
    )?;
    def(
        "Select",
        method(lua, |lua, id, a| {
            select(lua, id, a.f(1) as i32, false, true);
            none()
        })?,
    )?;
    def(
        "InitializeSelect",
        method(lua, |lua, id, a| {
            let index = a.f(1) as i32;
            if index >= 0 {
                select(lua, id, index, true, false);
            }
            none()
        })?,
    )?;
    def(
        "ClearAllSelected",
        method(lua, |lua, id, _| {
            change(lua, id, |d| {
                let mut dirty = d.tree_mode && d.selected != -1;
                for row in &mut d.rows {
                    dirty |= row.clear(d.tree_mode);
                }
                if d.tree_mode {
                    d.selected = -1;
                }
                ((), dirty)
            });
            none()
        })?,
    )?;
    def(
        "GetSelectedIndex",
        method(lua, |lua, id, _| {
            ret(lua, data(&ui(lua), id).map_or(-1, |d| d.selected))
        })?,
    )?;
    def(
        "GetSelectedValue",
        method(lua, |lua, id, _| {
            let value = data(&ui(lua), id)
                .and_then(|d| tree::selected(&d.rows))
                .map(|r| r.value)
                .filter(|v| *v != -1);
            match value {
                Some(v) => ret(lua, v),
                None => none(),
            }
        })?,
    )?;
    def(
        "GetSelectedText",
        method(lua, |lua, id, _| {
            let text = data(&ui(lua), id)
                .and_then(|d| usize::try_from(d.selected).ok().and_then(|i| d.rows.get(i)))
                .map(|r| r.text.clone());
            match text {
                Some(t) => ret(lua, t),
                None => none(),
            }
        })?,
    )?;
    def(
        "SetHeight",
        method(lua, |lua, id, a| {
            let pad = a.f(1);
            if pad >= 0.0 {
                change(lua, id, |d| {
                    let dirty = d.padding != pad;
                    d.padding = pad;
                    ((), dirty)
                });
            }
            none()
        })?,
    )?;
    def(
        "GetHeight",
        method(lua, |lua, id, _| {
            ret(lua, metrics(&mut ui(lua), id).map_or(0.0, |m| m.1))
        })?,
    )?;
    def(
        "GetViewItemCount",
        method(lua, |lua, id, _| {
            ret(lua, metrics(&mut ui(lua), id).map_or(0, |m| m.2))
        })?,
    )?;
    def(
        "GetMaxTop",
        method(lua, |lua, id, _| ret(lua, max_top(&mut ui(lua), id)))?,
    )?;
    def(
        "GetTop",
        method(lua, |lua, id, _| {
            ret(lua, data(&ui(lua), id).map_or(0, |d| d.top))
        })?,
    )?;
    def(
        "SetTop",
        method(lua, |lua, id, a| {
            set_top(lua, id, a.f(1) as i32);
            none()
        })?,
    )?;
    for (name, delta) in [("ScrollUp", -1), ("ScrollDown", 1)] {
        def(
            name,
            method(lua, move |lua, id, _| {
                let top = data(&ui(lua), id).map_or(0, |d| d.top);
                set_top(lua, id, top.saturating_add(delta));
                none()
            })?,
        )?;
    }
    def(
        "SetTextLimit",
        method(lua, |lua, id, a| {
            let n = a.f(1) as i32;
            change(lua, id, |d| {
                d.text_limit = n;
                ((), false)
            });
            none()
        })?,
    )?;
    def(
        "UseConsistentTextColor",
        method(lua, |lua, id, a| {
            let on = a.b(1, true);
            change(lua, id, |d| {
                let dirty = d.consistent != on;
                d.consistent = on;
                ((), dirty)
            });
            none()
        })?,
    )?;
    def(
        "EnableSelectionToggle",
        method(lua, |lua, id, a| {
            let on = a.b(1, true);
            change(lua, id, |d| {
                d.toggle = on;
                ((), false)
            });
            none()
        })?,
    )?;
    for (prefix, index) in [("Default", 0), ("Overed", 1), ("Selected", 2)] {
        for text in [false, true] {
            let name = format!("Set{prefix}Item{}Color", if text { "Text" } else { "" });
            def(
                &name,
                method(lua, move |lua, id, a| {
                    let color = [a.f(1), a.f(2), a.f(3), a.opt_f(4).unwrap_or(1.0)];
                    change(lua, id, |d| {
                        let old = if text {
                            &mut d.text_colors[index]
                        } else {
                            &mut d.colors[index]
                        };
                        let dirty = *old != color;
                        *old = color;
                        ((), dirty)
                    });
                    none()
                })?,
            )?;
        }
    }
    Ok(())
}

pub(crate) fn quads(
    u: &mut Ui,
    id: Id,
    d: &ListBoxData,
    text: &TextBlock,
    alpha: f32,
    scale: f32,
    out: &mut Vec<Quad>,
) {
    let Some((inner, height, count)) = metrics(u, id) else {
        return;
    };
    if height <= 0.0 || inner.w <= 0.0 || inner.h <= 0.0 {
        return;
    }
    let first = out.len();
    let mut icons_seen = d.icons_seen;
    let child_style = if d.child_style {
        u.tree.widget(id).and_then(|w| w.aux_style.clone())
    } else {
        None
    };
    for (line, (row, depth)) in d
        .visible_rows()
        .into_iter()
        .skip(d.top.max(0) as usize)
        .take(count.max(0) as usize)
        .enumerate()
    {
        let y = inner.y + line as f32 * height;
        if y >= inner.y + inner.h {
            break;
        }
        let state = if row.selected && (d.select_parent || row.child.is_empty()) {
            2
        } else if d.hover == Some(d.top.max(0) as usize + line) {
            1
        } else {
            0
        };
        let depth_offset = 10.0 * depth as f32;
        paint::state(
            d,
            state,
            Rect {
                x: inner.x + 5.0 + depth_offset,
                y,
                w: (inner.w - 10.0 - depth_offset).max(0.0),
                h: height,
            },
            alpha,
            scale,
            out,
        );
        let child = (depth > 0).then_some(child_style.as_ref()).flatten();
        let mut t = child.unwrap_or(text).clone();
        t.text = row.text.clone();
        t.inset = [0.0; 4];
        let color = if d.consistent {
            if state == 0 {
                child.map_or(d.text_colors[state], |t| t.style.color)
            } else {
                d.text_colors[state]
            }
        } else {
            row.color
        };
        // Native caches tree icons but paints branch templates instead. Flat icon placement
        // uses TOPLEFT at the row midpoint (392a0ba7..0c0a), not the branch-image centering.
        if !d.tree_mode
            && let Some(path) = row.icon.as_ref()
        {
            paint::quad(
                Tex::Image(path.clone()),
                Rect {
                    x: inner.x + 10.0 + 15.0 * depth as f32,
                    y: y + height * 0.5,
                    w: height,
                    h: height,
                },
                [0., 0., 1., 1.],
                [1., 1., 1., alpha],
                scale,
                out,
            );
            icons_seen = true;
        }
        if d.tree_mode && !row.child.is_empty() {
            if let Some(image) = if row.opened {
                d.opened_image
            } else {
                d.closed_image
            } {
                tree::image(
                    u,
                    image,
                    (inner.x + 10.0 + 15.0 * depth as f32, y + height * 0.5),
                    alpha,
                    scale,
                    out,
                );
            }
        }
        let indent = if d.tree_mode || icons_seen {
            d.indent.map_or(height, |n| n as f32)
        } else {
            0.0
        };
        crate::render::text_quads(
            u,
            &t,
            Rect {
                x: inner.x + 5.0 + depth_offset + indent,
                y,
                w: (inner.w - 10.0 - depth_offset - 2.0 * indent).max(0.0),
                h: height,
            },
            color,
            alpha,
            scale,
            out,
        );
        if line > 0
            && depth == 0
            && let Some(image) = d.separator_image
        {
            let extent = u.tree.get(image);
            let center = (
                inner.x + 10.0 + extent.width * 0.5,
                y + 2.0 - extent.height * 0.5,
            );
            tree::image(u, image, center, alpha, scale, out);
        }
    }
    if icons_seen
        && !d.icons_seen
        && let Some(d) = u.tree.widget_mut(id).and_then(|w| w.listbox.as_mut())
    {
        // Native remembers that an icon was painted, including across ClearItem. Only the
        // first visible flat icon changes this; repeated paints do not write tree state.
        d.icons_seen = true;
    }
    let bounds = u.tree.rect(id);
    crate::render::clip::quads(
        out,
        first,
        [
            bounds.x * scale,
            bounds.y * scale,
            (bounds.x + bounds.w) * scale,
            (bounds.y + bounds.h) * scale,
        ],
    );
}
