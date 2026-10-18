//! Widget alpha/scale channels: native 392771c0/39277230, 3927db80 and 3927e017.
//! The two SetStartAnimation booleans enable alpha and scale independently.

use super::{Timing, positive};
use crate::lua::{Ui, method, none, ret, ui};
use crate::tree::{Id, Point, Rect, Tree};
use mlua::{Lua, Table};
use std::collections::{BTreeSet, HashMap};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WidgetSample {
    pub alpha: Option<f32>,
    pub scale: f32,
    pub pivot: Point,
    pub offset: [f32; 2],
}

impl Default for WidgetSample {
    fn default() -> Self {
        Self {
            alpha: None,
            scale: 1.0,
            pivot: Point::Center,
            offset: [0.0; 2],
        }
    }
}

impl WidgetSample {
    pub fn point(self, rect: Rect, point: [f32; 2]) -> [f32; 2] {
        let pivot = [
            rect.x + rect.w * self.pivot.fx(),
            rect.y + rect.h * self.pivot.fy(),
        ];
        std::array::from_fn(|i| pivot[i] + (point[i] - pivot[i]) * self.scale + self.offset[i])
    }
}

pub(super) struct Widgets {
    states: HashMap<Id, State>,
    active: BTreeSet<Id>,
    ui_scale: f32,
    offsets: HashMap<Id, [f32; 2]>,
}

struct State {
    alpha: Channel,
    visible_alpha: Option<f32>,
    scale: Channel,
    pivot: Option<Point>,
    is_root: bool,
    apply_ui_scale: bool,
    explicit_scale: Option<f32>,
}

impl Default for Widgets {
    fn default() -> Self {
        Self {
            states: Default::default(),
            active: Default::default(),
            ui_scale: 1.0,
            offsets: Default::default(),
        }
    }
}

impl Default for State {
    fn default() -> Self {
        Self {
            alpha: Default::default(),
            visible_alpha: None,
            scale: Default::default(),
            pivot: None,
            is_root: false,
            apply_ui_scale: true,
            explicit_scale: None,
        }
    }
}

struct Channel {
    from: f32,
    to: f32,
    timing: Timing,
    current: f32,
    velocity: f64,
    acceleration: f64,
    started: bool,
    running: bool,
}

impl Default for Channel {
    fn default() -> Self {
        Self {
            from: 1.0,
            to: 1.0,
            timing: Timing::default(),
            current: 1.0,
            velocity: 0.0,
            acceleration: 0.0,
            started: false,
            running: false,
        }
    }
}

impl Channel {
    fn configure(&mut self, from: f32, to: f32, timing: Timing) {
        self.from = from;
        self.to = to;
        self.timing = timing;
    }

    fn start(&mut self, on: bool) {
        if on {
            let (v, a) = self.timing.coefficients();
            let delta = (self.to - self.from) as f64;
            self.current = self.from;
            self.velocity = delta * v;
            self.acceleration = delta * a;
            self.started = true;
        } else if self.running {
            self.current = self.to;
        }
        self.running = on && self.started;
    }

    fn tick(&mut self, dt: f64) -> bool {
        if !self.running {
            return false;
        }
        // The widget channel updates velocity first, then position; effects use the
        // analytic evaluator in timing.rs instead. This difference is in the native code.
        self.velocity += self.acceleration * dt;
        self.current += (self.velocity * dt) as f32;
        let ended = self.timing.duration() <= 0.0
            || if self.to >= self.from {
                self.current >= self.to
            } else {
                self.current <= self.to
            };
        if ended {
            self.current = self.to;
            self.running = false;
        }
        ended
    }
}

impl State {
    fn raw_scale(&self, ui_scale: f32) -> f32 {
        let baseline = if self.is_root && self.apply_ui_scale {
            ui_scale
        } else {
            1.0
        };
        if self.scale.started {
            self.scale.current * baseline
        } else {
            self.explicit_scale.unwrap_or(baseline)
        }
    }

    fn sample(&self, ui_scale: f32) -> WidgetSample {
        WidgetSample {
            alpha: self.alpha.started.then_some(self.visible_alpha.unwrap_or(self.alpha.current)),
            scale: self.raw_scale(ui_scale) / if self.is_root { ui_scale } else { 1.0 },
            pivot: if self.scale.started {
                self.pivot.unwrap_or(Point::Center)
            } else {
                Point::Center
            },
            offset: [0.0; 2],
        }
    }
    fn running(&self) -> bool {
        self.alpha.running || self.scale.running
    }
}

impl Widgets {
    pub(super) fn effective_scale(&self, tree: &Tree, id: Id) -> f32 {
        let own = self.states.get(&id).map(|s| s.raw_scale(self.ui_scale));
        match tree.get(id).parent {
            Some(parent) => own.unwrap_or(1.0) * self.effective_scale(tree, parent),
            None => own.unwrap_or(self.ui_scale),
        }
    }

    pub fn sample(&self, id: Id) -> WidgetSample {
        let mut sample = self.states
            .get(&id)
            .map_or_else(Default::default, |s| s.sample(self.ui_scale));
        sample.offset = self.offsets.get(&id).copied().unwrap_or([0.0; 2]);
        sample
    }

    pub fn set_offset(&mut self, id: Id, offset: [f32; 2]) {
        if offset == [0.0; 2] { self.offsets.remove(&id); }
        else { self.offsets.insert(id, offset); }
    }

    pub fn set_ui_scale(&mut self, value: f32) {
        if self.ui_scale != value {
            self.ui_scale = value;
            for s in self
                .states
                .values_mut()
                .filter(|s| s.is_root && s.apply_ui_scale)
            {
                s.explicit_scale = None;
            }
        }
    }

    pub fn clear_alpha(&mut self, id: Id) {
        if let Some(s) = self.states.get_mut(&id) {
            s.alpha.started = false;
            s.alpha.running = false;
            if !s.running() {
                self.active.remove(&id);
            }
        }
    }
}

fn state(u: &mut Ui, id: Id) -> &mut State {
    let is_root = u.tree.get(id).parent.is_none();
    u.animations
        .widgets
        .states
        .entry(id)
        .or_insert_with(|| State {
            is_root,
            ..Default::default()
        })
}

/// Native SetAlpha writes visible +d8 while leaving the running +288 channel untouched.
pub(crate) fn set_alpha(u: &mut Ui, id: Id, value: f32) {
    if let Some(s) = u.animations.widgets.states.get_mut(&id) {
        s.visible_alpha = Some(value);
    }
    u.tree.get_mut(id).alpha = value;
}

pub(super) fn methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    m.set(
        "SetScale",
        method(lua, |lua, id, a| {
            let mut u = ui(lua);
            let ui_scale = u.animations.widgets.ui_scale;
            let s = state(&mut u, id);
            // Native 39265bd0 ignores SetScale while the scale animation is running.
            if s.scale.running {
                return none();
            }
            let before = s.sample(ui_scale);
            s.scale.started = false;
            let scale = a.f(1);
            s.explicit_scale = Some(if scale.is_finite() { scale } else { 0.0 });
            if s.sample(ui_scale) != before && u.tree.visible(id) {
                u.tree.repaint();
            }
            // Native SetScale dispatches event 42 (OnScale), even for an identical value.
            u.pending.push((id, "OnScale".into(), vec![]));
            super::super::auto_clip::changed(&mut u, id, true);
            none()
        })?,
    )?;
    m.set(
        "ApplyUIScale",
        method(lua, |lua, id, a| {
            let mut u = ui(lua);
            let ui_scale = u.animations.widgets.ui_scale;
            let s = state(&mut u, id);
            let raw = s.raw_scale(ui_scale);
            s.apply_ui_scale = a.b(1, true);
            // Freeze the current native scale when opting out of later global changes.
            if s.scale.started {
                let baseline = if s.is_root && s.apply_ui_scale {
                    ui_scale
                } else {
                    1.0
                };
                s.scale.current = raw / baseline;
            } else {
                s.explicit_scale = Some(raw);
            }
            none()
        })?,
    )?;
    for (name, scale) in [("SetAlphaAnimation", false), ("SetScaleAnimation", true)] {
        m.set(
            name,
            method(lua, move |lua, id, a| {
                let mut u = ui(lua);
                let state = state(&mut u, id);
                let timing = Timing {
                    velocity: positive(a.f(3)) * 1000.0,
                    acceleration: positive(a.f(4)) * 1000.0,
                };
                if scale {
                    state.scale.configure(a.f(1), a.f(2), timing);
                    state.pivot = a.s(5).and_then(|s| Point::parse(&s));
                } else {
                    state.alpha.configure(a.f(1), a.f(2), timing);
                }
                none()
            })?,
        )?;
    }
    m.set(
        "SetStartAnimation",
        method(lua, |lua, id, a| {
            let mut u = ui(lua);
            let ui_scale = u.animations.widgets.ui_scale;
            let state = state(&mut u, id);
            let scale_callback = a.b(2, false) || state.scale.running;
            let before = state.sample(ui_scale);
            if a.b(1, false) || state.alpha.running {
                state.visible_alpha = None;
            }
            state.alpha.start(a.b(1, false));
            state.scale.start(a.b(2, false));
            let scale_changed = state.sample(ui_scale).scale != before.scale;
            let changed = state.sample(ui_scale) != before;
            if state.running() {
                u.animations.widgets.active.insert(id);
            } else {
                u.animations.widgets.active.remove(&id);
            }
            if changed && u.tree.visible(id) {
                u.tree.repaint();
            }
            if scale_changed {
                u.pending.push((id, "OnScale".into(), vec![]));
            }
            if scale_callback {
                super::super::auto_clip::changed(&mut u, id, true);
            }
            none()
        })?,
    )?;
    m.set(
        "IsNowAnimation",
        method(lua, |lua, id, _| {
            ret(
                lua,
                ui(lua)
                    .animations
                    .widgets
                    .states
                    .get(&id)
                    .is_some_and(State::running),
            )
        })?,
    )?;
    Ok(())
}

pub(super) fn tick(u: &mut Ui, dt_ms: f32) -> Vec<(Id, &'static str)> {
    let ids = u
        .animations
        .widgets
        .active
        .iter()
        .copied()
        .collect::<Vec<_>>();
    let mut ended = Vec::new();
    let mut repaint = false;
    let mut scaled = Vec::new();
    for id in ids {
        if !u.tree.get(id).alive {
            u.animations.widgets.states.remove(&id);
            u.animations.widgets.active.remove(&id);
            continue;
        }
        let ui_scale = u.animations.widgets.ui_scale;
        let state = u.animations.widgets.states.get_mut(&id).unwrap();
        if state.scale.running {
            scaled.push(id);
        }
        let before = state.sample(ui_scale);
        if state.alpha.running {
            state.visible_alpha = None;
        }
        if state.alpha.tick(positive(dt_ms) as f64) {
            ended.push((id, "OnAlphaAnimeEnd"));
        }
        if state.scale.tick(positive(dt_ms) as f64) {
            ended.push((id, "OnScaleAnimeEnd"));
        }
        if before.scale != state.sample(ui_scale).scale {
            ended.push((id, "OnScale"));
        }
        if before != state.sample(ui_scale) {
            repaint |= u.tree.visible(id);
        }
        if !state.running() {
            u.animations.widgets.active.remove(&id);
        }
    }
    if repaint {
        u.tree.repaint();
    }
    for id in scaled {
        super::super::auto_clip::changed(u, id, true);
    }
    ended
}

/// Apply each widget's scale from the innermost widget to the outermost ancestor.
pub(super) fn transform_rect(widgets: &Widgets, tree: &Tree, mut owner: Id, rect: Rect) -> Rect {
    let mut lo = [rect.x, rect.y];
    let mut hi = [rect.right(), rect.bottom()];
    loop {
        let sample = widgets.sample(owner);
        if sample.scale != 1.0 || sample.offset != [0.0; 2] {
            let pivot_rect = tree.rect(owner);
            lo = sample.point(pivot_rect, lo);
            hi = sample.point(pivot_rect, hi);
        }
        match tree.get(owner).parent {
            Some(p) => owner = p,
            None => break,
        }
    }
    Rect {
        x: lo[0],
        y: lo[1],
        w: hi[0] - lo[0],
        h: hi[1] - lo[1],
    }
}
