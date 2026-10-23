//! Widget alpha/scale channels: native 392771c0/39277230, 3927db80 and 3927e017.
//! The two SetStartAnimation booleans enable alpha and scale independently.

use super::positive;
use crate::lua::{Ui, method, none, ret, ui};
use crate::tree::{Id, Point, Rect, Tree};
use mlua::{Lua, Table, Value};
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
    display_pivot: Point,
    scale_origin: [f32; 2],
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
            display_pivot: Point::Center,
            scale_origin: [0.0; 2],
            is_root: false,
            apply_ui_scale: true,
            explicit_scale: None,
        }
    }
}

#[derive(Clone)]
struct Channel {
    from: f32,
    to: f32,
    current: f32,
    velocity: f32,
    initial_velocity: f32,
    acceleration: f32,
    started: bool,
    running: bool,
}

impl Default for Channel {
    fn default() -> Self {
        Self {
            from: 1.0,
            to: 1.0,
            current: 1.0,
            velocity: 0.0,
            initial_velocity: 0.0,
            acceleration: 0.0,
            started: false,
            running: false,
        }
    }
}

impl Channel {
    fn configure(&mut self, from: f32, to: f32, velocity_time: f32, acceleration_time: f32) {
        self.from = from;
        self.to = to;
        let delta = to - from;
        self.velocity = delta / velocity_time;
        self.initial_velocity = self.velocity;
        // Native392771c0/39277230 retain the previous acceleration only for +/-0.
        // Reconfiguration writes velocity immediately without changing current/flags.
        if acceleration_time != 0.0 {
            self.acceleration = ((delta - self.velocity * acceleration_time) * 2.0)
                / (acceleration_time * acceleration_time);
        }
    }

    fn start(&mut self, on: bool) {
        if on {
            self.current = self.from;
            self.velocity = self.initial_velocity;
            self.started = true;
        } else if self.running {
            self.current = self.to;
        }
        self.running = on && self.started;
    }

    fn tick(&mut self, dt_ms: f32) -> bool {
        if !self.running {
            return false;
        }
        // The widget channel updates velocity first, then position; effects use the
        // analytic evaluator in timing.rs instead. This difference is in the native code.
        let dt = dt_ms * 0.001;
        self.velocity += self.acceleration * dt;
        self.current += self.velocity * dt;
        let ended = (self.to >= self.from && self.current >= self.to)
            || (self.from >= self.to && self.to >= self.current);
        if ended {
            self.current = self.to;
            self.running = false;
        }
        ended
    }
}

impl State {
    fn baseline(&self, ui_scale: f32) -> f32 {
        if self.is_root && self.apply_ui_scale {
            ui_scale
        } else {
            1.0
        }
    }

    fn raw_scale(&self, ui_scale: f32) -> f32 {
        self.explicit_scale.unwrap_or_else(|| {
            if self.scale.started {
                self.scale.current * self.baseline(ui_scale)
            } else {
                self.baseline(ui_scale)
            }
        })
    }

    fn sample(&self, ui_scale: f32) -> WidgetSample {
        WidgetSample {
            alpha: self
                .alpha
                .started
                .then_some(self.visible_alpha.unwrap_or(self.alpha.current)),
            scale: self.raw_scale(ui_scale) / if self.is_root { ui_scale } else { 1.0 },
            pivot: self.display_pivot,
            offset: self.scale_origin,
        }
    }
    fn running(&self) -> bool {
        self.alpha.running || self.scale.running
    }

    fn apply_scale(&mut self, rect: Rect, raw: f32, pivot: Point, ui_scale: f32) {
        let old = self.sample(ui_scale);
        // Native3927b2f0 retains the chosen point of the CURRENT rectangle. A new
        // configured pivot must not reinterpret geometry against the original anchors.
        self.scale_origin[0] += rect.w * (1.0 - old.scale) * (old.pivot.fx() - pivot.fx());
        self.scale_origin[1] += rect.h * (1.0 - old.scale) * (old.pivot.fy() - pivot.fy());
        self.display_pivot = pivot;
        self.explicit_scale = Some(raw);
    }
}

impl Widgets {
    pub(super) fn remove(&mut self, id: Id) {
        self.states.remove(&id);
        self.active.remove(&id);
        self.offsets.remove(&id);
    }

    pub(super) fn effective_scale(&self, tree: &Tree, id: Id) -> f32 {
        let own = self.states.get(&id).map(|s| s.raw_scale(self.ui_scale));
        match tree.get(id).parent {
            Some(parent) => own.unwrap_or(1.0) * self.effective_scale(tree, parent),
            None => own.unwrap_or(self.ui_scale),
        }
    }

    pub fn sample(&self, id: Id) -> WidgetSample {
        let mut sample = self
            .states
            .get(&id)
            .map_or_else(Default::default, |s| s.sample(self.ui_scale));
        if let Some(offset) = self.offsets.get(&id) {
            for i in 0..2 {
                sample.offset[i] += offset[i];
            }
        }
        sample
    }

    pub fn set_offset(&mut self, id: Id, offset: [f32; 2]) {
        if offset == [0.0; 2] {
            self.offsets.remove(&id);
        } else {
            self.offsets.insert(id, offset);
        }
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
        "InheritAnimationData",
        method(lua, |lua, id, a| {
            let Value::Table(source) = a.v(1) else {
                return none();
            };
            let Some(source) = super::super::value_id(lua, &Value::Table(source)) else {
                return none();
            };
            let mut u = ui(lua);
            if !u.tree.get(source).alive
                || u.tree.widget(source).is_none()
                || u.tree.widget(id).is_none()
            {
                return none();
            }
            let ui_scale = u.animations.widgets.ui_scale;
            let source_state = u.animations.widgets.states.get(&source);
            let alpha = source_state.map_or_else(Channel::default, |s| s.alpha.clone());
            let scale = source_state.map_or_else(Channel::default, |s| s.scale.clone());
            let pivot = source_state.and_then(|s| s.pivot);
            let raw = source_state.map_or_else(
                || {
                    if u.tree.get(source).parent.is_none() {
                        ui_scale
                    } else {
                        1.0
                    }
                },
                |s| s.raw_scale(ui_scale),
            );
            let old = u.animations.widget_sample(id);
            let visible = old.alpha.unwrap_or(u.tree.get(id).alpha);
            let rect = u.tree.rect(id);
            let state = state(&mut u, id);
            let old_raw = state.raw_scale(ui_scale);
            let active_scale = scale.running;
            // Copy the native channel fields/flags, keeping destination root/UI policy.
            state.alpha = alpha;
            state.visible_alpha = (!state.alpha.running).then_some(visible);
            state.explicit_scale = Some(old_raw);
            state.scale = scale;
            state.pivot = pivot;
            if active_scale {
                state.apply_scale(rect, raw, pivot.unwrap_or(Point::Center), ui_scale);
            } else {
                state.explicit_scale = Some(old_raw);
            }
            let new_visible = if state.alpha.running {
                state.alpha.current
            } else {
                visible
            };
            let running = state.running();
            u.tree.get_mut(id).alpha = new_visible;
            if running {
                u.animations.widgets.active.insert(id);
            } else {
                u.animations.widgets.active.remove(&id);
            }
            u.tree.repaint();
            if active_scale {
                super::super::auto_clip::changed(&mut u, id, true);
                drop(u);
                super::super::anchors::scaled(lua, id);
                super::super::call_handler(lua, id, "OnScale", vec![]);
            }
            none()
        })?,
    )?;
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
            let rect = u.tree.rect(id);
            let s = state(&mut u, id);
            s.scale.started = false;
            let scale = a.f(1);
            s.apply_scale(
                rect,
                if scale.is_finite() { scale } else { 0.0 },
                Point::Center,
                ui_scale,
            );
            if s.sample(ui_scale) != before && u.tree.visible(id) {
                u.tree.repaint();
            }
            // Native SetScale dispatches event 42 (OnScale), even for an identical value.
            super::super::auto_clip::changed(&mut u, id, true);
            drop(u);
            super::super::anchors::scaled(lua, id);
            super::super::call_handler(lua, id, "OnScale", vec![]);
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
            }
            s.explicit_scale = Some(raw);
            none()
        })?,
    )?;
    for (name, scale) in [("SetAlphaAnimation", false), ("SetScaleAnimation", true)] {
        m.set(
            name,
            method(lua, move |lua, id, a| {
                if !(1..=4).all(|i| a.is_num(i)) || (scale && !matches!(a.v(5), Value::String(_))) {
                    return none();
                }
                let mut u = ui(lua);
                if u.tree.widget(id).is_none() {
                    return none();
                }
                let state = state(&mut u, id);
                if scale {
                    state.scale.configure(a.f(1), a.f(2), a.f(3), a.f(4));
                    // XlCompareString is case-sensitive; native unknown enum9 has
                    // the same zero origin fractions as TOPLEFT in392772b0.
                    let name = a.s(5).unwrap();
                    let name = name.split('\0').next().unwrap();
                    state.pivot = Some(
                        Point::parse(name)
                            .filter(|p| p.name() == name)
                            .unwrap_or(Point::TopLeft),
                    );
                } else {
                    state.alpha.configure(a.f(1), a.f(2), a.f(3), a.f(4));
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
            let rect = u.tree.rect(id);
            let state = state(&mut u, id);
            let scale_callback = a.b(2, false);
            let before = state.sample(ui_scale);
            if a.b(1, false) || state.alpha.running {
                state.visible_alpha = None;
            }
            state.alpha.start(a.b(1, false));
            if scale_callback {
                let raw_before = state.raw_scale(ui_scale);
                state.scale.start(true);
                let raw = state.scale.current * state.baseline(ui_scale);
                state.explicit_scale = Some(raw_before);
                state.apply_scale(rect, raw, state.pivot.unwrap_or(Point::Center), ui_scale);
            } else {
                // Native3927dbce clears the scale flag before its stop branch: no snap,
                // geometry update, OnScale or completion callback when stopping scale.
                state.scale.running = false;
            }
            let changed = state.sample(ui_scale) != before;
            if state.running() {
                u.animations.widgets.active.insert(id);
            } else {
                u.animations.widgets.active.remove(&id);
            }
            if changed && u.tree.visible(id) {
                u.tree.repaint();
            }
            if scale_callback {
                super::super::auto_clip::changed(&mut u, id, true);
                drop(u);
                super::super::anchors::scaled(lua, id);
                super::super::call_handler(lua, id, "OnScale", vec![]);
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

pub(super) fn tick(lua: &Lua, id: Id, dt_ms: f32) {
    if !ui(lua).animations.widgets.states.contains_key(&id) {
        return;
    }
    let alpha_ended = {
        let mut u = ui(lua);
        if !u.tree.get(id).alive {
            u.animations.widgets.states.remove(&id);
            u.animations.widgets.active.remove(&id);
            return;
        }
        let ui_scale = u.animations.widgets.ui_scale;
        let state = u.animations.widgets.states.get_mut(&id).unwrap();
        let before = state.sample(ui_scale);
        if state.alpha.running {
            state.visible_alpha = None;
        }
        let ended = state.alpha.tick(positive(dt_ms));
        let changed = before != state.sample(ui_scale);
        if changed && u.tree.visible(id) {
            u.tree.repaint();
        }
        ended
    };
    if alpha_ended {
        super::super::call_handler(lua, id, "OnAlphaAnimeEnd", vec![]);
    }
    // Native checks the scale flag AFTER alpha's callback, so a newly enabled
    // scale gets this frame's step. Alpha itself is not stepped a second time.
    let scale_step = {
        let mut u = ui(lua);
        if !u.tree.get(id).alive {
            u.animations.widgets.states.remove(&id);
            u.animations.widgets.active.remove(&id);
            return;
        }
        let ui_scale = u.animations.widgets.ui_scale;
        let state = u.animations.widgets.states.get_mut(&id).unwrap();
        if state.scale.running {
            let raw = state.raw_scale(ui_scale);
            state.explicit_scale = Some(raw);
            state.scale.current = raw / state.baseline(ui_scale);
            let ended = state.scale.tick(positive(dt_ms));
            Some((state.scale.current, ended, state.is_root))
        } else {
            None
        }
    };
    if let Some((current, ended, was_root)) = scale_step {
        if ended {
            // Native3927e149 clears the flag but retains OLD raw geometry
            // through the callback. Its captured target survives reconfiguration.
            super::super::call_handler(lua, id, "OnScaleAnimeEnd", vec![]);
        }
        let applied = {
            let mut u = ui(lua);
            if u.tree.get(id).alive {
                let ui_scale = u.animations.widgets.ui_scale;
                let rect = u.tree.rect(id);
                let state = u.animations.widgets.states.get_mut(&id).unwrap();
                let before = state.sample(ui_scale);
                let baseline = if was_root && state.apply_ui_scale {
                    ui_scale
                } else {
                    1.0
                };
                state.apply_scale(
                    rect,
                    current * baseline,
                    state.pivot.unwrap_or(Point::Center),
                    ui_scale,
                );
                let changed = before != state.sample(ui_scale);
                if changed && u.tree.visible(id) {
                    u.tree.repaint();
                }
                super::super::auto_clip::changed(&mut u, id, true);
                true
            } else {
                false
            }
        };
        if applied {
            super::super::anchors::scaled(lua, id);
            super::super::call_handler(lua, id, "OnScale", vec![]);
        }
    }
    let mut u = ui(lua);
    if !u.tree.get(id).alive {
        u.animations.widgets.states.remove(&id);
        u.animations.widgets.active.remove(&id);
    } else if u
        .animations
        .widgets
        .states
        .get(&id)
        .is_some_and(State::running)
    {
        u.animations.widgets.active.insert(id);
    } else {
        u.animations.widgets.active.remove(&id);
    }
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

#[cfg(test)]
mod inherit_tests {
    use crate::{UiRuntime, lua};

    #[test]
    fn copied_raw_scale_keeps_native_bits_until_the_next_normalizing_step_and_offsets_survive() {
        let ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
        ui.exec(
            "@native_raw_bits",
            r#"
            p=UIParent:CreateWidget('window','rawParent','UIParent');p:Show(true)
            a=p:CreateChildWidget('button','rawSource',0,true)
            b=UIParent:CreateWidget('button','rawDestination','UIParent');b:Show(true)
            a:SetExtent(100,40);b:SetExtent(100,40)
            a:SetScaleAnimation(1.700007677078247,1,.1,0,'CENTER')
            a:SetStartAnimation(false,true)
        "#,
        )
        .unwrap();
        let b = lua::value_id(&ui.lua, &ui.lua.load("b").eval().unwrap()).unwrap();
        lua::ui(&ui.lua).animations.set_widget_offset(b, [7., -3.]);
        ui.exec("@raw_copy", "b:InheritAnimationData(a)").unwrap();
        {
            let u = lua::ui(&ui.lua);
            assert_eq!(u.animations.widget_scale(&u.tree, b).to_bits(), 0x3fd999da);
            assert_eq!(u.animations.widget_sample(b).offset, [7., -3.]);
        }
        lua::tick(&ui.lua, 0.);
        let u = lua::ui(&ui.lua);
        assert_eq!(u.animations.widget_scale(&u.tree, b).to_bits(), 0x3fd999db);
        assert_eq!(u.animations.widget_sample(b).offset, [7., -3.]);
    }
}
