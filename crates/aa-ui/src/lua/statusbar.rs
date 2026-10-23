//! `statusbar` widgets (HP/MP bars, the exp and labor bars): a bar texture
//! (`SetBarTexture(path, layer)` + `SetBarTextureCoords(x, y, w, h)`, tinted by `SetBarColor`)
//! filled to the displayed value's fraction of the widget. Like the client the texture is cropped,
//! not squeezed: a half full bar shows the left half of its texture rectangle.

use mlua::{Lua, Table, Value};

use super::{Args, Ui, method, new_object_table, none, ret, ui, value_id};
use crate::tree::{Anchor, DrawKind, DrawLayer, Id, Point};

#[derive(Clone, Debug, PartialEq)]
pub struct StatusBarData {
    pub min: i32,
    pub max: i32,
    pub value: i32,
    /// Native displayed value is an offset from the range minimum, separate from GetValue.
    pub displayed: f32,
    pub speed: f32,
    pub animate: bool,
    pub running: bool,
    pub vertical: bool,
    /// The bar drawable (created by `SetBarTexture`).
    pub bar: Option<Id>,
    pub coords: Option<[f32; 4]>,
    pub color: [f32; 4],
}

impl Default for StatusBarData {
    fn default() -> Self {
        StatusBarData {
            min: 0,
            max: 0,
            value: 0,
            displayed: 0.0,
            speed: 0.0,
            animate: false,
            running: false,
            vertical: false,
            bar: None,
            coords: None,
            color: [1.0; 4],
        }
    }
}

impl StatusBarData {
    pub fn fraction(&self) -> f32 {
        if self.max != self.min {
            self.displayed / self.max.wrapping_sub(self.min) as f32
        } else {
            0.0
        }
    }

    pub fn bar_visible(&self) -> bool {
        self.max != self.min && self.value != self.min
    }

    fn set_value(&mut self, value: i32, animate: bool, seconds: f32) {
        let old = self.value;
        self.value = value.clamp(self.min, self.max);
        // Native same-target calls still update geometry, but do not reset the animation.
        if old == self.value {
            return;
        }
        self.animate = animate && seconds > 0.0;
        if !self.animate {
            self.running = false;
            self.speed = 1.0;
        } else {
            let previous_delta = self.displayed - old.wrapping_sub(self.min) as f32;
            let delta = self.displayed - self.value.wrapping_sub(self.min) as f32;
            self.speed = if self.running && previous_delta != 0.0 && self.speed != 0.0 {
                let remaining = (previous_delta / self.speed).abs();
                (delta / remaining).abs()
            } else {
                (delta / seconds).abs()
            };
        }
    }

    fn advance(&mut self, dt_ms: f32) {
        if self.bar.is_none() || !self.bar_visible() {
            return;
        }
        // 392c7fec compares the adjusted target with the absolute maximum.
        let target = self.value.wrapping_sub(self.min).min(self.max) as f32;
        if !self.animate || self.displayed == target {
            self.displayed = target;
        } else {
            let step = self.speed * dt_ms * 0.001;
            let delta = target - self.displayed;
            if step.abs() > delta.abs() {
                self.running = false;
                self.displayed = target;
            } else {
                self.displayed += (delta / delta.abs()) * step;
                self.running = true;
            }
        }
    }
}

fn with<R>(lua: &Lua, id: Id, f: impl FnOnce(&mut StatusBarData) -> R) -> Option<R> {
    let mut u = ui(lua);
    let w = u.tree.widget_mut(id)?;
    Some(f(w.status.get_or_insert_with(StatusBarData::default)))
}

/// Native zero/minimum hides the image without changing its last extent.
fn remember_extent(lua: &Lua, id: Id) {
    let mut u = ui(lua);
    let Some(s) = u.tree.widget(id).and_then(|w| w.status.as_ref()) else {
        return;
    };
    let Some(bar) = s.bar.filter(|_| s.bar_visible()) else {
        return;
    };
    let r = u.tree.rect(bar);
    let o = u.tree.get(bar);
    if (o.width, o.height) != (r.w, r.h) {
        let o = u.tree.get_mut(bar);
        o.width = r.w;
        o.height = r.h;
    }
}

/// Push the bar state into its drawable (fill fraction, coordinates, colour).
fn sync(lua: &Lua, id: Id) {
    sync_state(&mut ui(lua), id, 0.0);
}

fn sync_state(u: &mut Ui, id: Id, dt_ms: f32) {
    let Some(mut s) = u.tree.widget(id).and_then(|w| w.status.clone()) else {
        return;
    };
    let Some(bar) = s.bar else { return };
    s.advance(dt_ms);
    if u.tree.widget(id).and_then(|w| w.status.as_ref()) != Some(&s) {
        u.tree.widget_mut(id).unwrap().status = Some(s.clone());
    }
    let visible = s.bar_visible();
    if let Some(d) = u.tree.get(bar).drawable() {
        let fill = if visible || d.fill.is_none() {
            Some((s.fraction(), s.vertical))
        } else {
            d.fill
        };
        if (d.fill, d.coords, d.color) != (fill, s.coords, s.color) {
            let d = u.tree.drawable_mut(bar).unwrap();
            d.fill = fill;
            d.coords = s.coords;
            d.color = s.color;
        }
    }
    if visible {
        // Native stretches across the bar, leaving the filled axis at its explicit extent.
        for point in [
            Point::TopLeft,
            if s.vertical {
                Point::TopRight
            } else {
                Point::BottomLeft
            },
        ] {
            u.tree.add_anchor(
                bar,
                Anchor {
                    point,
                    target: id,
                    rel: point,
                    x: 0.0,
                    y: 0.0,
                },
            );
        }
    }
    if u.tree.get(bar).shown != visible {
        u.tree.get_mut(bar).shown = visible;
    }
}

pub(super) fn tick(lua: &Lua, dt_ms: f32) {
    let mut u = ui(lua);
    for id in u.tree.of_class("statusbar") {
        if u.tree.visible(id)
            && u.tree
                .widget(id)
                .and_then(|w| w.status.as_ref())
                .is_some_and(|s| s.animate && s.running)
        {
            sync_state(&mut u, id, dt_ms);
        }
    }
}

pub(crate) fn methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    let def = |name: &str, f: mlua::Function| m.raw_set(name, f);
    def(
        "AddAnchorChildToBar",
        method(lua, |lua, id, a| {
            let Some(bar) = ui(lua)
                .tree
                .widget(id)
                .and_then(|w| w.status.as_ref())
                .and_then(|s| s.bar)
            else {
                return none();
            };
            if let Value::Table(_) = a.v(1)
                && let Some(child) = value_id(lua, &a.v(1))
                && let Some(mut anchor) =
                    super::widget::parse_anchor(lua, child, &Args(a.0[1..].to_vec()))
            {
                anchor.target = bar;
                let accepted = ui(lua).tree.install_anchor(child, anchor);
                if accepted { super::anchors::changed(lua, child); }
            }
            none()
        })?,
    )?;
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
            let (lo, hi) = (a.f(1) as i32, a.f(2) as i32);
            if lo > hi {
                return none();
            }
            remember_extent(lua, id);
            with(lua, id, |s| {
                s.min = lo;
                s.max = hi;
            });
            sync(lua, id);
            none()
        })?,
    )?;
    def(
        "GetMinMaxValues",
        method(lua, |lua, id, _| {
            let (lo, hi) = with(lua, id, |s| (s.min as f32, s.max as f32)).unwrap_or((0.0, 0.0));
            ret(lua, (lo, hi))
        })?,
    )?;
    // (value, animate, seconds): GetValue changes immediately; the image eases separately.
    def(
        "SetValue",
        method(lua, |lua, id, a| {
            let v = a.f(1) as i32;
            remember_extent(lua, id);
            let animate = matches!(a.v(2), Value::Boolean(true));
            let seconds = match a.v(3) {
                Value::Number(n) => n as f32,
                Value::Integer(n) => n as f32,
                _ => 0.0,
            };
            with(lua, id, |s| s.set_value(v, animate, seconds));
            sync(lua, id);
            none()
        })?,
    )?;
    def(
        "GetValue",
        method(lua, |lua, id, _| {
            ret(lua, with(lua, id, |s| s.value as f32).unwrap_or(0.0))
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
        method(lua, |lua, id, _| match with(lua, id, |s| s.bar).flatten() {
            Some(b) => ret(lua, new_object_table(lua, b)?),
            None => none(),
        })?,
    )?;
    Ok(())
}
