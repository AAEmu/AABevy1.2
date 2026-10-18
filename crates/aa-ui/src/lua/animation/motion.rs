//! Separate motion channel of effect drawables (native 3928a950/3928e080/3928f460).

use mlua::{Lua, Table};

use super::{Effect, Sample, Timing, positive};
use super::timing::{Clock, crossed};
use crate::lua::{method, none, ui};
use crate::tree::{Id, Rect};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct Motion {
    phases: BTreeMap<i64, Phase>,
    repeat: u32,
    delay: f32,
    rotate: bool,
    clock: Clock,
    current: Sample,
}

#[derive(Clone, Copy, Default)]
enum Kind {
    #[default]
    None,
    Circle,
    Top,
    Right,
    Bottom,
    Left,
}

#[derive(Clone, Copy)]
struct Phase {
    kind: Kind,
    radius: [f32; 2],
    circle: [f32; 2],
    edge: [f32; 2],
    timing: Timing,
    hold: f32,
}

impl Default for Phase {
    fn default() -> Self {
        Self { kind: Kind::None, radius: [0.5; 2], circle: [0.; 2], edge: [0., 1.],
            timing: Timing::default(), hold: 0. }
    }
}

impl Motion {
    pub fn new() -> Self {
        Self {
            rotate: true,
            ..Default::default()
        }
    }

    fn initialize(&mut self, phase: usize, rect: Rect) {
        if let Some(p) = self.phases.values().nth(phase).copied() {
            let [initial, final_value] = p.bounds();
            self.clock.initialize(phase, final_value - initial, p.timing);
            if !matches!(p.kind, Kind::None) {
                let mut current = p.sample(initial, rect, true);
                if !self.rotate && !matches!(p.kind, Kind::Circle) {
                    current.rotation = self.current.rotation;
                }
                self.current = current;
            }
        }
    }

    pub fn start(&mut self, rect: Rect) { self.initialize(0, rect); self.clock.start(); }
    pub fn stop(&mut self) { self.clock.running = false; }
    pub fn running(&self) -> bool { self.clock.running }
    pub fn configured(&self) -> bool { !self.phases.is_empty() }
    pub fn pending(&self) -> bool { self.configured() && !self.clock.terminal }
    pub fn sample(&self) -> Sample {
        let mut out = self.current;
        if !self.rotate { out.rotation = 0.; }
        out
    }

    pub fn advance(&mut self, dt: f32, rect: Rect) -> bool {
        let Some(p) = self.phases.values().nth(self.clock.phase).copied()
            .filter(|p| !matches!(p.kind, Kind::None)) else {
            self.clock.running = false;
            self.clock.terminal = true;
            return false;
        };
        let Some(dt) = self.clock.time(dt, self.delay) else { return false };
        if self.clock.finished {
            if let Some((phase, carry)) = self.clock.following(dt, p.hold, self.phases.len(), self.repeat) {
                self.initialize(phase, rect);
                self.clock.elapsed += carry;
            }
        } else {
            let [initial, final_value] = p.bounds();
            let mut value = initial + self.clock.displacement(dt);
            if crossed(initial, final_value, value) {
                value = final_value;
                self.clock.finish(p.timing);
            }
            self.current = p.sample(value, rect, true);
        }
        true
    }
}

impl Phase {
    fn bounds(self) -> [f32; 2] {
        if matches!(self.kind, Kind::Circle) {
            self.circle
        } else {
            self.edge
        }
    }

    fn sample(self, value: f32, rect: Rect, rotate: bool) -> Sample {
        let (offset, angle) = match self.kind {
            Kind::None => ([0.0; 2], 0.0),
            Kind::Circle => {
                let (sin, cos) = value.to_radians().sin_cos();
                let (rx, ry) = (rect.w * self.radius[0], rect.h * self.radius[1]);
                // The native projects a rotated ray onto the ellipse; it does not use
                // independently scaled sine/cosine coordinates for unequal radii.
                let denominator = ((ry * sin).powi(2) + (rx * cos).powi(2)).sqrt();
                let radius = if denominator > 0.0 {
                    rx * ry / denominator
                } else {
                    0.0
                };
                ([sin * radius, -cos * radius], value)
            }
            //3928a950 multiplies before subtracting the half extent (separate f32 ops).
            Kind::Top => ([rect.w * value - rect.w * 0.5, -rect.h * self.radius[1]], 0.0),
            Kind::Right => ([rect.w * self.radius[0], rect.h * value - rect.h * 0.5], 90.0),
            Kind::Bottom => ([rect.w * value - rect.w * 0.5, rect.h * self.radius[1]], 180.0),
            Kind::Left => ([-rect.w * self.radius[0], rect.h * value - rect.h * 0.5], 270.0),
        };
        Sample {
            offset,
            rotation: if rotate { angle.to_radians() } else { 0.0 },
            ..Default::default()
        }
    }
}

fn change(lua: &Lua, id: Id, index: i64, f: impl FnOnce(&mut Phase)) {
    let mut u = ui(lua);
    let e = u
        .animations
        .effects
        .entry(id)
        .or_insert_with(Effect::default);
    f(e.motion.phases.entry(index).or_default());
    e.motion.clock.terminal = false;
    u.tree.repaint();
}

pub(super) fn methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    m.set(
        "SetMoveEffectType",
        method(lua, |lua, id, a| {
            let kind = match a.s(2).unwrap_or_default().as_str() {
                "circle" => Kind::Circle,
                "top" => Kind::Top,
                "right" => Kind::Right,
                "bottom" => Kind::Bottom,
                "left" => Kind::Left,
                _ => Kind::None,
            };
            change(lua, id, a.f(1) as i64, |p| {
                p.kind = kind;
                p.radius = [a.f(3), a.f(4)];
                p.timing = Timing {
                    velocity: positive(a.f(5)),
                    acceleration: positive(a.f(6)),
                };
            });
            none()
        })?,
    )?;
    for (name, circle) in [("SetMoveEffectCircle", true), ("SetMoveEffectEdge", false)] {
        m.set(
            name,
            method(lua, move |lua, id, a| {
                change(lua, id, a.f(1) as i64, |p| {
                    if circle {
                        p.circle = [a.f(2), a.f(3)];
                    } else {
                        p.edge = [a.f(2), a.f(3)];
                    }
                });
                none()
            })?,
        )?;
    }
    m.set(
        "SetMoveEffectInterval",
        method(lua, |lua, id, a| {
            change(lua, id, a.f(1) as i64, |p| {
                p.hold = positive(a.f(2))
            });
            none()
        })?,
    )?;
    m.set(
        "SetMoveRepeatCount",
        method(lua, |lua, id, a| {
            let mut u = ui(lua);
            let e = u.animations.effects.entry(id).or_default();
            e.motion.repeat = positive(a.f(1)) as u32;
            e.motion.clock.terminal = false;
            u.tree.repaint();
            none()
        })?,
    )?;
    m.set(
        "SetMoveInterval",
        method(lua, |lua, id, a| {
            let mut u = ui(lua);
            let e = u.animations.effects.entry(id).or_default();
            e.motion.delay = positive(a.f(1));
            e.motion.clock.terminal = false;
            u.tree.repaint();
            none()
        })?,
    )?;
    m.set(
        "SetMoveRotate",
        method(lua, |lua, id, a| {
            let mut u = ui(lua);
            u.animations.effects.entry(id).or_default().motion.rotate = a.b(1, true);
            u.tree.repaint();
            none()
        })?,
    )?;
    m.set(
        "ClearMoveEffect",
        method(lua, |lua, id, _| {
            let mut u = ui(lua);
            if !u.tree.get(id).drawable().is_some_and(|d| d.class == "drawable:effect") {
                return none();
            }
            if let Some(e) = u.animations.effects.get_mut(&id) {
                e.motion.phases.clear();
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
