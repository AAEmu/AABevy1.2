//! Native widget resize borders and captured, incremental edge motion.

use mlua::{Lua, Table};

use super::{Ui, method, none, ui};
use crate::tree::{Anchor, DrawKind, DrawLayer, Id, Point, ROOT, Rect};

/// Widget properties at native +244..264. Generic widgets start with zero limits/borders.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Data {
    pub enabled: bool,
    pub border: [f32; 4],
    pub min: [f32; 2],
    pub max: [f32; 2],
    guide: Option<Id>,
}

/// Native 39277580 / 399c533c: strict strips in physical pixels, not scaled widget units.
pub(crate) fn edge(u: &Ui, id: Id, pointer: (f32, f32)) -> Option<Point> {
    let w = u.tree.widget(id)?;
    if !w.enabled || !w.resizing.enabled {
        return None;
    }
    let r = u.animations.widget_rect(&u.tree, id);
    let [x, y] = [pointer.0, pointer.1];
    if !r.contains(x, y) {
        return None;
    }
    let b = w.resizing.border.map(|n| n / u.ui_scale);
    let mask = usize::from(x > r.x && x < r.x + b[0])
        | (usize::from(y > r.y && y < r.y + b[1]) << 1)
        | (usize::from(x < r.right() && x > r.right() - b[2]) << 2)
        | (usize::from(y < r.bottom() && y > r.bottom() - b[3]) << 3);
    match mask {
        1 => Some(Point::Left),
        2 => Some(Point::Top),
        3 => Some(Point::TopLeft),
        4 => Some(Point::Right),
        6 => Some(Point::TopRight),
        8 => Some(Point::Bottom),
        9 => Some(Point::BottomLeft),
        12 => Some(Point::BottomRight),
        _ => None,
    }
}

pub(crate) fn press(lua: &Lua, id: Id) -> bool {
    let point = {
        let u = ui(lua);
        edge(&u, id, u.cursor)
    };
    if let Some(point) = point {
        super::drag::start_sizing(lua, id, point);
        true
    } else {
        false
    }
}

/// Only switch the two affected guides; settled hover does no tree writes.
pub(crate) fn hover(lua: &Lua, hover: Option<Id>) {
    let mut u = ui(lua);
    let mut next = hover;
    while let Some(id) = next {
        if edge(&u, id, u.cursor).is_some() {
            break;
        }
        next = u.tree.get(id).parent;
    }
    let point = if let Some((id, point)) = super::drag::resize_capture(&u)
        && u.tree.visible(id)
        && u.tree.widget(id).is_some_and(|w| w.resizing.enabled)
    {
        next = Some(id);
        Some(point)
    } else {
        next.and_then(|id| edge(&u, id, u.cursor))
    };
    use crate::cursor::ResizeCursor as C;
    let cursor = point.and_then(|p| match p {
        Point::TopLeft | Point::BottomRight => Some(C::DiagonalDown),
        Point::TopRight | Point::BottomLeft => Some(C::DiagonalUp),
        Point::Left | Point::Right => Some(C::Horizontal),
        Point::Top | Point::Bottom => Some(C::Vertical),
        Point::Center => None,
    });
    crate::cursor::set_resize(lua, cursor);
    let old = u.drag.resize_hover;
    if old == next {
        return;
    }
    u.drag.resize_hover = next;
    for (id, shown) in [(old, false), (next, true)] {
        if let Some(guide) = id.and_then(|id| u.tree.widget(id)?.resizing.guide)
            && u.tree.get(guide).alive
            && u.tree.get(guide).shown != shown
        {
            u.tree.get_mut(guide).shown = shown;
        }
    }
}

fn limit(value: f32, min: f32, max: f32) -> f32 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

/// Native 39277650 and 3927a110. Pointer re-entry gates avoid a jump after a limit.
pub(crate) fn motion(u: &mut Ui, id: Id, point: Point, previous: (f32, f32), to: (f32, f32)) {
    if previous == to {
        return;
    }
    let Some(w) = u.tree.widget(id) else { return };
    let data = w.resizing.clone();
    let base = u.tree.rect(id);
    let r = u.animations.widget_rect(&u.tree, id);
    let own_scale = u.animations.widget_sample(id).scale;
    let mut scale = 1.0;
    let mut ancestor = Some(id);
    while let Some(a) = ancestor {
        scale *= u.animations.widget_sample(a).scale;
        ancestor = u.tree.get(a).parent;
    }
    if !scale.is_finite() || scale <= 0.0 || own_scale <= 0.0 {
        return;
    }
    let b = data.border.map(|n| n / u.ui_scale);
    let (dx, dy) = (to.0 - previous.0, to.1 - previous.1);
    let gates = [
        if dx > 0.0 {
            to.0 > r.x + b[0]
        } else {
            to.0 < r.x
        },
        if dy > 0.0 {
            to.1 > r.y + b[1]
        } else {
            to.1 < r.y
        },
        if dx > 0.0 {
            to.0 > r.right() - b[2]
        } else {
            to.0 < r.right()
        },
        if dy > 0.0 {
            to.1 > r.bottom() - b[3]
        } else {
            to.1 < r.bottom()
        },
    ];
    let min = data.min.map(|v| v * scale);
    let max = data.max.map(|v| v * scale);
    let mut out = r;
    if point == Point::Center {
        out.x += dx;
        out.y += dy;
    } else {
        if point.fx() == 0.0 && gates[0] {
            out.w = limit(r.w - dx, min[0], max[0]);
            out.x = r.right() - out.w;
        } else if point.fx() == 1.0 && gates[2] {
            out.w += dx;
        }
        if point.fy() == 0.0 && gates[1] {
            out.h = limit(r.h - dy, min[1], max[1]);
            out.y = r.bottom() - out.h;
        } else if point.fy() == 1.0 && gates[3] {
            out.h += dy;
        }
    }
    if data.enabled {
        out.w = limit(out.w, min[0], max[0]);
        out.h = limit(out.h, min[1], max[1]);
    }
    if out == r {
        return;
    }
    // Invert the current uniform widget/ancestor transform, including its resizing pivot.
    let parent_scale = scale / own_scale;
    let pivot = u.animations.widget_sample(id).pivot;
    let width = out.w / scale;
    let height = out.h / scale;
    let x =
        base.x + (out.x - r.x) / parent_scale + (own_scale - 1.0) * (width - base.w) * pivot.fx();
    let y =
        base.y + (out.y - r.y) / parent_scale + (own_scale - 1.0) * (height - base.h) * pivot.fy();
    set_rect(
        u,
        id,
        Rect {
            x,
            y,
            w: width,
            h: height,
        },
        false,
    );
}

pub(crate) fn place(u: &mut Ui, id: Id, rect: Rect) {
    set_rect(u, id, rect, true);
}

fn set_rect(u: &mut Ui, id: Id, rect: Rect, reset: bool) {
    if reset { u.tree.remove_anchors(id); }
    u.tree.add_anchor(
        id,
        Anchor {
            point: Point::TopLeft,
            target: ROOT,
            rel: Point::TopLeft,
            x: rect.x,
            y: rect.y,
        },
    );
    let o = u.tree.get_mut(id);
    o.width = rect.w;
    o.height = rect.h;
}

fn enable(u: &mut Ui, id: Id, enabled: bool) {
    let Some(w) = u.tree.widget(id) else { return };
    let old = w.resizing.guide;
    if w.resizing.enabled == enabled && old.is_some_and(|d| u.tree.get(d).alive) {
        return;
    }
    if !enabled {
        if !w.resizing.enabled && old.is_none() {
            return;
        }
        if let Some(guide) = old {
            u.tree.get_mut(guide).alive = false;
            u.tree
                .widget_mut(id)
                .unwrap()
                .drawables
                .retain(|d| *d != guide);
        }
        let data = &mut u.tree.widget_mut(id).unwrap().resizing;
        data.enabled = false;
        data.guide = None;
        if u.drag.resize_hover == Some(id) {
            u.drag.resize_hover = None;
        }
        return;
    }
    let guide = u.tree.create_drawable(
        id,
        DrawKind::NinePart,
        "drawable:ninepart",
        DrawLayer::Background,
    );
    let d = u.tree.drawable_mut(guide).unwrap();
    d.texture = Some("ui/hud/chat_guide.dds".into());
    d.coords = Some([0.0, 0.0, 16.0, 16.0]);
    d.inset = [7.0, 7.0, 8.0, 8.0];
    u.tree.get_mut(guide).shown = false;
    for (point, x, y) in [(Point::TopLeft, -3.0, -1.0), (Point::BottomRight, 2.0, 5.0)] {
        u.tree.add_anchor(
            guide,
            Anchor {
                point,
                target: id,
                rel: point,
                x,
                y,
            },
        );
    }
    let data = &mut u.tree.widget_mut(id).unwrap().resizing;
    data.enabled = true;
    data.guide = Some(guide);
}

pub(crate) fn methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    m.raw_set(
        "UseResizing",
        method(lua, |lua, id, a| {
            enable(&mut ui(lua), id, a.b(1, true));
            none()
        })?,
    )?;
    m.raw_set(
        "SetResizingBorderSize",
        method(lua, |lua, id, a| {
            let border = std::array::from_fn(|i| a.f(i + 1).max(0.0));
            if let Some(w) = ui(lua).tree.widget_mut(id) {
                w.resizing.border = border;
            }
            none()
        })?,
    )?;
    for (name, minimum) in [
        ("SetMinResizingExtent", true),
        ("SetMaxResizingExtent", false),
    ] {
        m.raw_set(
            name,
            method(lua, move |lua, id, a| {
                let extent = [a.f(1).max(0.0), a.f(2).max(0.0)];
                if let Some(w) = ui(lua).tree.widget_mut(id) {
                    if minimum {
                        w.resizing.min = extent;
                    } else {
                        w.resizing.max = extent;
                    }
                }
                none()
            })?,
        )?;
    }
    m.raw_set(
        "StartSizing",
        method(lua, |lua, id, a| {
            let point = a
                .s(1)
                .and_then(|s| Point::parse(&s))
                .unwrap_or(Point::Center);
            super::drag::start_sizing(lua, id, point);
            none()
        })?,
    )?;
    Ok(())
}
