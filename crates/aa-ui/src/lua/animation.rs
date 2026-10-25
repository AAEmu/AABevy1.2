//! Drawable timelines used by the original Lua UI. Generic Effects advance on painting;
//! atlas frames retain their own update clock. Samples do not invalidate layout.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use mlua::{Lua, Table};

use super::{Args, Ui, method, none, ret, ui};
use crate::tree::{Id, Point, Rect};

mod motion;
mod timing;
pub mod widgets;
use timing::{Clock, Timing};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub coords: Option<[f32; 4]>,
    pub color: Option<[f32; 4]>,
    pub alpha: f32,
    pub scale: [f32; 2],
    pub offset: [f32; 2],
    pub rotation: f32,
    pub pivot: Point,
    pub clip: bool,
}

impl Default for Sample {
    fn default() -> Self {
        Self {
            coords: None,
            color: None,
            alpha: 1.0,
            scale: [1.0; 2],
            offset: [0.0; 2],
            rotation: 0.0,
            pivot: Point::Center,
            clip: false,
        }
    }
}

impl Sample {
    pub fn rect(self, mut rect: Rect) -> Rect {
        let (w, h) = (rect.w * self.scale[0], rect.h * self.scale[1]);
        rect.x += self.offset[0] + (rect.w - w) * self.pivot.fx();
        rect.y += self.offset[1] + (rect.h - h) * self.pivot.fy();
        rect.w = w;
        rect.h = h;
        rect
    }
}

#[derive(Default)]
pub struct Animations {
    effects: HashMap<Id, Effect>,
    frames: HashMap<Id, Frames>,
    active: BTreeSet<Id>,
    samples: HashMap<Id, Sample>,
    widgets: widgets::Widgets,
}

impl Animations {
    pub fn with_ui_scale(ui_scale: f32) -> Self {
        let mut this = Self::default();
        this.widgets.set_ui_scale(ui_scale);
        this
    }

    pub fn set_ui_scale(&mut self, ui_scale: f32) {
        self.widgets.set_ui_scale(ui_scale);
    }

    pub fn sample(&self, id: Id) -> Sample {
        self.samples.get(&id).copied().unwrap_or_default()
    }

    pub fn remove(&mut self, id: Id) {
        self.effects.remove(&id);
        self.frames.remove(&id);
        self.active.remove(&id);
        self.samples.remove(&id);
        self.widgets.remove(id);
    }

    pub fn widget_sample(&self, id: Id) -> widgets::WidgetSample {
        self.widgets.sample(id)
    }

    pub fn widget_rect(&self, tree: &crate::tree::Tree, id: Id) -> Rect {
        widgets::transform_rect(&self.widgets, tree, id, tree.rect(id))
    }

    /// Native +68: own scale times the parent's inherited scale, in parent-first f32 order.
    pub(crate) fn widget_scale(&self, tree: &crate::tree::Tree, id: Id) -> f32 {
        self.widgets.effective_scale(tree, id)
    }

    pub fn widget_clip_rect(&self, tree: &crate::tree::Tree, id: Id) -> Option<Rect> {
        let mut out: Option<Rect> = None;
        let mut owner = tree.clip_start(id);
        while let Some(p) = owner {
            if tree.widget(p).is_some_and(|w| w.clip) {
                let rect = self.widget_rect(tree, p);
                out = Some(out.map_or(rect, |old| old.intersect(&rect)));
            }
            owner = tree.get(p).parent;
        }
        out
    }

    pub(crate) fn widget_clip_contains(&self, tree: &crate::tree::Tree, id: Id, x: f32, y: f32) -> bool {
        let mut owner = tree.clip_start(id);
        while let Some(p) = owner {
            if let Some(w) = tree.widget(p).filter(|w| w.clip) {
                let rect = self.widget_rect(tree, p);
                // Native3927c795..7bc admits all four ListCtrl clip edges.
                // The child's own hit rectangle and other scroll widgets keep
                // their existing half-open bounds.
                let inside = if w.ty == crate::tree::WidgetType::ListCtrl {
                    x >= rect.x && y >= rect.y && x <= rect.right() && y <= rect.bottom()
                } else {
                    rect.contains(x, y)
                };
                if !inside { return false; }
            }
            owner = tree.get(p).parent;
        }
        true
    }

    pub fn clear_widget_alpha(&mut self, id: Id) {
        self.widgets.clear_alpha(id);
    }

    pub(crate) fn set_widget_offset(&mut self, id: Id, offset: [f32; 2]) {
        self.widgets.set_offset(id, offset);
    }
}

pub(super) fn widget_methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    widgets::methods(lua, m)
}

struct Effect {
    phases: BTreeMap<i64, Phase>,
    repeat: u32,
    delay: f32,
    clock: Clock,
    current: Sample,
    draw_color: bool,
    draw_motion: bool,
    motion: motion::Motion,
}

impl Default for Effect {
    fn default() -> Self {
        Self {
            phases: Default::default(),
            repeat: 0,
            delay: 0.0,
            clock: Clock::default(),
            current: Sample::default(),
            draw_color: false,
            draw_motion: false,
            motion: motion::Motion::new(),
        }
    }
}

#[derive(Clone, Copy)]
struct Phase {
    timing: Timing,
    priority: usize,
    hold: f32,
    initial: [f32; 4],
    final_color: [f32; 4],
    scale: [f32; 4],
    rotation: [f32; 2],
    pivot: Point,
    clip: bool,
}

impl Default for Phase {
    fn default() -> Self {
        Self {
            timing: Default::default(),
            priority: 0,
            hold: 0.0,
            initial: [1.0; 4],
            final_color: [1.0; 4],
            scale: [1.0; 4],
            rotation: [0.0; 2],
            pivot: Point::Center,
            clip: false,
        }
    }
}

fn positive(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn lerp(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
}

impl Phase {
    fn endpoint(self, final_point: bool) -> Sample {
        Sample {
            color: Some(if final_point { self.final_color } else { self.initial }),
            scale: if final_point { [self.scale[1], self.scale[3]] } else { [self.scale[0], self.scale[2]] },
            rotation: self.rotation[usize::from(final_point)].to_radians(),
            pivot: self.pivot, clip: self.clip, ..Default::default()
        }
    }

    fn bounds(self) -> [f32; 2] {
        match self.priority {
            0 => [self.scale[0], self.scale[1]],
            1 => [self.scale[2], self.scale[3]],
            2 => self.rotation,
            p => {
                let i = [3, 0, 1, 2][p.saturating_sub(3).min(3)];
                [self.initial[i], self.final_color[i]]
            }
        }
    }

    fn sample(self, t: f32) -> Sample {
        Sample {
            color: Some(std::array::from_fn(|i| {
                lerp(self.initial[i], self.final_color[i], t)
            })),
            scale: [
                lerp(self.scale[0], self.scale[1], t),
                lerp(self.scale[2], self.scale[3], t),
            ],
            rotation: lerp(self.rotation[0], self.rotation[1], t).to_radians(),
            pivot: self.pivot,
            clip: self.clip,
            ..Default::default()
        }
    }
}

impl Effect {
    fn initialize(&mut self, phase: usize) {
        if let Some(p) = self.phases.values().nth(phase).copied() {
            let [initial, final_value] = p.bounds();
            self.clock.initialize(phase, final_value - initial, p.timing);
            self.current = p.endpoint(false);
        }
    }

    fn sample(&self) -> Sample {
        let mut sample = self.current;
        let movement = if self.draw_motion { self.motion.sample() } else { Sample::default() };
        sample.offset = movement.offset;
        sample.rotation += movement.rotation;
        sample
    }

    fn advance_color(&mut self, dt: f32) -> bool {
        let Some(p) = self.phases.values().nth(self.clock.phase).copied() else {
            self.clock.running = false;
            return false;
        };
        let Some(dt) = self.clock.time(dt, self.delay) else { return false };
        if self.clock.finished {
            if let Some((phase, carry)) = self.clock.following(dt, p.hold, self.phases.len(), self.repeat) {
                self.initialize(phase);
                self.clock.elapsed += carry;
            }
        } else {
            let [initial, final_value] = p.bounds();
            //3928e7e0's scaley branch adds displacement to final Y, unlike the other six.
            let base = if p.priority == 1 { final_value } else { initial };
            let value = base + self.clock.displacement(dt);
            if timing::crossed(initial, final_value, value) {
                self.current = p.endpoint(true);
                self.clock.finish(p.timing);
            } else {
                self.current = p.sample(((value - initial) / (final_value - initial)).abs());
            }
        }
        true
    }

    fn advance(&mut self, dt: f32, rect: Rect) -> Sample {
        self.draw_color = self.advance_color(dt);
        self.draw_motion = self.motion.advance(dt, rect);
        if let Some(p) = self.phases.values().nth(self.clock.phase) {
            self.current.pivot = p.pivot;
            self.current.clip = p.clip;
        }
        self.sample()
    }

    fn pending(&self) -> bool {
        (!self.phases.is_empty() && !self.clock.terminal) || self.motion.pending()
    }
}

#[derive(Default)]
struct Frames {
    frames: Vec<Frame>,
    elapsed: f64,
    looping: bool,
    running: bool,
}

struct Frame {
    duration: f32,
    coords: Option<[f32; 4]>,
    // Animated movement is a delta for this phase; scale/alpha are target values.
    movement: [f32; 2],
    alpha: Option<f32>,
    scale: Option<f32>,
    linear_scale: bool,
}

impl Frames {
    fn sample(&self) -> (Sample, bool) {
        let duration: f64 = self.frames.iter().map(|f| f.duration as f64).sum();
        let mut t = self.elapsed;
        let running = duration > 0.0 && (self.looping || t < duration);
        if self.looping && duration > 0.0 {
            t %= duration;
        }
        let mut out = Sample::default();
        for frame in &self.frames {
            let k = if frame.duration > 0.0 {
                (t / frame.duration as f64).clamp(0.0, 1.0) as f32
            } else {
                1.0
            };
            if let Some(coords) = frame.coords {
                out.coords = Some(coords);
            }
            for i in 0..2 {
                out.offset[i] += frame.movement[i] * k;
            }
            if let Some(alpha) = frame.alpha {
                out.alpha = lerp(out.alpha, alpha, k);
            }
            if let Some(scale) = frame.scale {
                out.scale = [if frame.linear_scale {
                    lerp(out.scale[0], scale, k)
                } else {
                    scale
                }; 2];
            }
            if t < frame.duration as f64 {
                break;
            }
            t -= frame.duration as f64;
        }
        (out, running)
    }
}

fn phase(lua: &Lua, id: Id, a: &Args, change: impl FnOnce(&mut Phase)) {
    let mut u = ui(lua);
    let e = u.animations.effects.entry(id).or_default();
    change(e.phases.entry(a.f(1) as i64).or_default());
    e.clock.terminal = false;
    u.tree.repaint();
}

pub(super) fn effect_methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    motion::methods(lua, m)?;
    m.set(
        "SetEffectScaleAxis",
        method(lua, |lua, id, a| {
            phase(lua, id, a, |p| {
                p.pivot = a
                    .s(2)
                    .and_then(|s| Point::parse(&s))
                    .unwrap_or(Point::Center)
            });
            none()
        })?,
    )?;
    m.set(
        "SetEffectPriority",
        method(lua, |lua, id, a| {
            phase(lua, id, a, |p| {
                p.timing = Timing {
                    velocity: positive(a.f(3)),
                    acceleration: positive(a.f(4)),
                };
                p.priority = match a.s(2).unwrap_or_default().as_str() {
                    "scalex" => 0,
                    "scaley" => 1,
                    "rotate" => 2,
                    "colorr" => 4,
                    "colorg" => 5,
                    "colorb" => 6,
                    _ => 3,
                };
            });
            none()
        })?,
    )?;
    m.set(
        "SetEffectInitialColor",
        method(lua, |lua, id, a| {
            phase(lua, id, a, |p| p.initial = [a.f(2), a.f(3), a.f(4), a.f(5)]);
            none()
        })?,
    )?;
    m.set(
        "SetEffectFinalColor",
        method(lua, |lua, id, a| {
            phase(lua, id, a, |p| {
                p.final_color = [a.f(2), a.f(3), a.f(4), a.f(5)]
            });
            none()
        })?,
    )?;
    m.set(
        "SetEffectClip",
        method(lua, |lua, id, a| {
            phase(lua, id, a, |p| p.clip = a.b(2, false));
            none()
        })?,
    )?;
    m.set(
        "SetEffectScale",
        method(lua, |lua, id, a| {
            phase(lua, id, a, |p| p.scale = [a.f(2), a.f(3), a.f(4), a.f(5)]);
            none()
        })?,
    )?;
    m.set(
        "SetEffectRotate",
        method(lua, |lua, id, a| {
            phase(lua, id, a, |p| p.rotation = [a.f(2), a.f(3)]);
            none()
        })?,
    )?;
    m.set(
        "SetEffectInterval",
        method(lua, |lua, id, a| {
            phase(lua, id, a, |p| p.hold = positive(a.f(2)));
            none()
        })?,
    )?;
    m.set(
        "SetRepeatCount",
        method(lua, |lua, id, a| {
            let mut u = ui(lua);
            let e = u.animations.effects.entry(id).or_default();
            e.repeat = positive(a.f(1)) as u32;
            e.clock.terminal = false;
            u.tree.repaint();
            none()
        })?,
    )?;
    m.set(
        "SetInterval",
        method(lua, |lua, id, a| {
            let mut u = ui(lua);
            let e = u.animations.effects.entry(id).or_default();
            e.delay = positive(a.f(1));
            e.clock.terminal = false;
            u.tree.repaint();
            none()
        })?,
    )?;
    m.set(
        "SetStartEffect",
        method(lua, |lua, id, a| {
            //39236830/39235f40: required bool or numeric bool, without a default.
            let on = match a.v(1) {
                mlua::Value::Boolean(v) => v,
                mlua::Value::Integer(v) => v != 0,
                mlua::Value::Number(v) => v != 0.,
                _ => return none(),
            };
            let mut u = ui(lua);
            if !u.tree.get(id).drawable().is_some_and(|d| d.class == "drawable:effect") {
                return none();
            }
            let old_sample = u.animations.sample(id);
            let rect = u.tree.rect(id);
            if on {
                let e = u.animations.effects.entry(id).or_default();
                e.initialize(0);
                e.clock.start();
                e.motion.start(rect);
                e.draw_color = !e.phases.is_empty();
                e.draw_motion = e.motion.configured();
                let sample = e.sample();
                u.animations.samples.insert(id, sample);
            } else {
                if let Some(e) = u.animations.effects.get_mut(&id) {
                    e.clock.running = false;
                    e.motion.stop();
                }
            }
            //3928fb50 writes the drawable's own visibility for both branches.
            // Original low-health warnings use IsVisible() to admit the next restart.
            u.tree.get_mut(id).shown = on;
            if let Some(d) = u.tree.drawable_mut(id) {
                d.effect = Some(on);
            }
            if u.animations.sample(id) != old_sample && u.tree.visible(id) {
                u.tree.repaint();
            }
            none()
        })?,
    )?;
    for name in ["IsStartEffect", "IsRunning"] {
        m.set(
            name,
            method(lua, |lua, id, _| {
                if !ui(lua).tree.get(id).drawable().is_some_and(|d| d.class == "drawable:effect") {
                    return none();
                }
                ret(
                    lua,
                    ui(lua)
                        .animations
                        .effects
                        .get(&id)
                        .is_some_and(|e| e.clock.running && e.motion.running()),
                )
            })?,
        )?;
    }
    m.set(
        "ClearEffect",
        method(lua, |lua, id, _| {
            let mut u = ui(lua);
            if !u.tree.get(id).drawable().is_some_and(|d| d.class == "drawable:effect") {
                return none();
            }
            if let Some(e) = u.animations.effects.get_mut(&id) {
                e.phases.clear();
                e.clock.running = false;
                e.motion.stop();
            }
            if let Some(d) = u.tree.drawable_mut(id) {
                d.effect = Some(false);
            }
            u.tree.get_mut(id).shown = false;
            none()
        })?,
    )?;
    Ok(())
}

pub(super) fn frame_methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    m.set(
        "SetAnimFrameInfo",
        method(lua, |lua, id, a| {
            let mut frames = Vec::new();
            if let mlua::Value::Table(info) = a.v(1) {
                for value in info.sequence_values::<Table>() {
                    let f = value?;
                    let num = |key: &str| f.get::<Option<f32>>(key).ok().flatten();
                    let ty = num("animType").unwrap_or(0.0) as u32;
                    let coords = num("x")
                        .zip(num("y"))
                        .zip(num("w").zip(num("h")))
                        .map(|((x, y), (w, h))| [x, y, w, h]);
                    frames.push(Frame {
                        duration: positive(num("time").unwrap_or(0.0)),
                        coords,
                        movement: if ty & 1 != 0 {
                            [num("moveX").unwrap_or(0.0), num("moveY").unwrap_or(0.0)]
                        } else {
                            [0.0; 2]
                        },
                        alpha: if ty & 4 != 0 { num("alpha") } else { None },
                        scale: num("scale"),
                        linear_scale: ty & 2 != 0,
                    });
                }
            }
            let mut u = ui(lua);
            let had_sample = u.animations.sample(id) != Sample::default();
            u.animations.frames.insert(
                id,
                Frames {
                    frames,
                    ..Default::default()
                },
            );
            u.animations.active.remove(&id);
            u.animations.samples.remove(&id);
            if had_sample && u.tree.visible(id) {
                u.tree.repaint();
            }
            none()
        })?,
    )?;
    m.set(
        "Animation",
        method(lua, |lua, id, a| {
            let mut u = ui(lua);
            let on = a.b(1, true);
            let old_sample = u.animations.sample(id);
            if let Some(f) = u.animations.frames.get_mut(&id) {
                f.elapsed = 0.0;
                f.looping = a.b(2, false);
                f.running = on;
                let (sample, running) = f.sample();
                f.running &= running;
                u.animations.samples.insert(id, sample);
                if on && running {
                    u.animations.active.insert(id);
                } else {
                    u.animations.active.remove(&id);
                }
            }
            u.tree.get_mut(id).shown = on;
            if u.animations.sample(id) != old_sample && u.tree.visible(id) {
                u.tree.repaint();
            }
            none()
        })?,
    )?;
    Ok(())
}

pub(super) fn tick(u: &mut Ui, dt_ms: f32) {
    let active = u.animations.active.iter().copied().collect::<Vec<_>>();
    let mut repaint = false;
    for id in active {
        if !u.tree.get(id).alive {
            u.animations.remove(id);
            continue;
        }
        let (sample, running, hide) = if let Some(f) = u.animations.frames.get_mut(&id) {
            f.elapsed += positive(dt_ms) as f64;
            let (sample, running) = f.sample();
            f.running = running;
            (sample, running, !running)
        } else {
            u.animations.active.remove(&id);
            continue;
        };
        let visible = u.tree.visible(id);
        if u.animations.samples.get(&id) != Some(&sample) {
            u.animations.samples.insert(id, sample);
            repaint |= visible;
        }
        if !running {
            u.animations.active.remove(&id);
        }
        if hide {
            u.tree.get_mut(id).shown = false;
        }
    }
    // Request painting, without advancing an Effect that native traversal would skip.
    for (&id, e) in &u.animations.effects {
        if !e.pending() { continue; }
        let o = u.tree.get(id);
        if let (Some(parent), Some(d)) = (o.parent, o.drawable())
            && u.tree.widget(parent).is_some_and(|w| !w.drawable_layers[d.layer as usize])
        { continue; }
        let mut owner = id;
        loop {
            let o = u.tree.get(owner);
            if !o.alive || !(o.shown || o.widget().is_some_and(|w| w.fading)) { break; }
            if let Some(parent) = o.parent { owner = parent; }
            else {
                repaint |= u.tree.top.contains(&owner);
                break;
            }
        }
        if repaint { break; }
    }
    if repaint {
        u.tree.repaint();
    }
}

/// One admitted owner paint pass, before alpha/scissor skips. Overlay follows
/// descendants; background/artwork precede content. No frame/slot clocks here.
pub(crate) fn prepare_effect_draw(u: &mut Ui, owner: Id, overlay: bool) {
    let Some(w) = u.tree.widget(owner) else { return };
    let ids: Vec<_> = w.drawables.iter().copied().filter(|&id| {
        let o = u.tree.get(id);
        o.alive && o.shown && o.drawable().is_some_and(|d| {
            d.class == "drawable:effect" && !d.row_template && w.drawable_layers[d.layer as usize]
                && (d.layer == crate::tree::DrawLayer::Overlay) == overlay
        })
    }).collect();
    for id in ids {
        let rect = u.tree.rect(id);
        if let Some(e) = u.animations.effects.get_mut(&id) {
            let sample = e.advance(u.frame_ms / 1000., rect);
            u.animations.samples.insert(id, sample);
        }
    }
}

pub(crate) fn effect_draws(u: &Ui, id: Id) -> bool {
    u.animations.effects.get(&id).is_some_and(|e| e.draw_color)
}

pub(super) fn tick_widget(lua: &Lua, id: Id, dt_ms: f32) {
    widgets::tick(lua, id, dt_ms);
}
