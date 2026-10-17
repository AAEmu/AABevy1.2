//! Separate motion channel of effect drawables (native 3928a950/3928e080/3928f460).

use mlua::{Lua, Table};

use super::{Effect, Sample, Timing, positive};
use crate::lua::{method, none, ui};
use crate::tree::{Id, Rect};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct Motion {
    phases: BTreeMap<i64, Phase>,
    repeat: u32,
    delay: f32,
    rotate: bool,
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

#[derive(Clone, Copy, Default)]
struct Phase {
    kind: Kind,
    radius: [f32; 2],
    circle: [f32; 2],
    edge: [f32; 2],
    timing: Timing,
    hold: f32,
}

impl Motion {
    pub fn new() -> Self {
        Self {
            rotate: true,
            ..Default::default()
        }
    }

    pub fn sample(&self, elapsed: f64, rect: Rect) -> (Sample, bool) {
        let mut elapsed = elapsed - self.delay as f64;
        if elapsed < 0.0 {
            return (Sample::default(), !self.phases.is_empty());
        }
        let duration: f64 = self
            .phases
            .values()
            .map(|p| p.duration() + p.hold as f64)
            .sum();
        let Some(last) = self.phases.last_key_value().map(|(_, p)| p) else {
            return (Sample::default(), false);
        };
        if duration <= 0.0 {
            return (last.sample(f64::MAX, rect, self.rotate), false);
        }
        if self.repeat > 0 && elapsed >= duration * self.repeat as f64 {
            return (last.sample(f64::MAX, rect, self.rotate), false);
        }
        elapsed %= duration;
        for phase in self.phases.values() {
            let len = phase.duration() + phase.hold as f64;
            if elapsed < len {
                return (phase.sample(elapsed, rect, self.rotate), true);
            }
            elapsed -= len;
        }
        (last.sample(f64::MAX, rect, self.rotate), true)
    }
}

impl Phase {
    fn duration(self) -> f64 {
        let bounds = if matches!(self.kind, Kind::Circle) {
            self.circle
        } else {
            self.edge
        };
        if matches!(self.kind, Kind::None) || bounds[0] == bounds[1] {
            0.0
        } else {
            self.timing.duration()
        }
    }

    fn sample(self, elapsed: f64, rect: Rect, rotate: bool) -> Sample {
        let bounds = if matches!(self.kind, Kind::Circle) {
            self.circle
        } else {
            self.edge
        };
        let t = if self.duration() == 0.0 {
            1.0
        } else {
            self.timing.fraction(elapsed)
        };
        let value = super::lerp(bounds[0], bounds[1], t);
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
            Kind::Top => ([rect.w * (value - 0.5), -rect.h * self.radius[1]], 0.0),
            Kind::Right => ([rect.w * self.radius[0], rect.h * (value - 0.5)], 90.0),
            Kind::Bottom => ([rect.w * (value - 0.5), rect.h * self.radius[1]], 180.0),
            Kind::Left => ([-rect.w * self.radius[0], rect.h * (value - 0.5)], 270.0),
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
                    velocity: positive(a.f(5)) * 1000.0,
                    acceleration: positive(a.f(6)) * 1000.0,
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
                p.hold = positive(a.f(2)) * 1000.0
            });
            none()
        })?,
    )?;
    m.set(
        "SetMoveRepeatCount",
        method(lua, |lua, id, a| {
            ui(lua)
                .animations
                .effects
                .entry(id)
                .or_default()
                .motion
                .repeat = positive(a.f(1)) as u32;
            none()
        })?,
    )?;
    m.set(
        "SetMoveInterval",
        method(lua, |lua, id, a| {
            ui(lua)
                .animations
                .effects
                .entry(id)
                .or_default()
                .motion
                .delay = positive(a.f(1)) * 1000.0;
            none()
        })?,
    )?;
    m.set(
        "SetMoveRotate",
        method(lua, |lua, id, a| {
            ui(lua)
                .animations
                .effects
                .entry(id)
                .or_default()
                .motion
                .rotate = a.b(1, true);
            none()
        })?,
    )?;
    m.set(
        "ClearMoveEffect",
        method(lua, |lua, id, _| {
            let mut u = ui(lua);
            if let Some(e) = u.animations.effects.get_mut(&id) {
                e.motion.phases.clear();
                e.running = false;
            }
            u.animations.active.remove(&id);
            u.animations.samples.remove(&id);
            if let Some(d) = u.tree.drawable_mut(id) {
                d.effect = Some(false);
            }
            u.tree.get_mut(id).shown = false;
            none()
        })?,
    )?;
    Ok(())
}
