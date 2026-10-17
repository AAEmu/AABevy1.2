//! Drawable timelines used by the original Lua UI. Only running timelines are ticked;
//! samples live outside the anchor tree, so animation never invalidates layout.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use mlua::{Lua, Table};

use super::{Args, Ui, method, none, ret, ui};
use crate::tree::{Id, Rect};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub coords: Option<[f32; 4]>,
    pub color: Option<[f32; 4]>,
    pub alpha: f32,
    pub scale: [f32; 2],
    pub offset: [f32; 2],
    pub rotation: f32,
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
        }
    }
}

impl Sample {
    pub fn rect(self, mut rect: Rect) -> Rect {
        let (w, h) = (rect.w * self.scale[0], rect.h * self.scale[1]);
        rect.x += self.offset[0] + (rect.w - w) * 0.5;
        rect.y += self.offset[1] + (rect.h - h) * 0.5;
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
}

impl Animations {
    pub fn sample(&self, id: Id) -> Sample {
        self.samples.get(&id).copied().unwrap_or_default()
    }

    pub fn remove(&mut self, id: Id) {
        self.effects.remove(&id);
        self.frames.remove(&id);
        self.active.remove(&id);
        self.samples.remove(&id);
    }
}

#[derive(Default)]
struct Effect {
    phases: BTreeMap<i64, Phase>,
    repeat: u32,
    delay: f32,
    elapsed: f64,
    running: bool,
}

#[derive(Clone, Copy)]
struct Phase {
    duration: f32,
    hold: f32,
    initial: [f32; 4],
    final_color: [f32; 4],
    scale: [f32; 4],
    rotation: [f32; 2],
}

impl Default for Phase {
    fn default() -> Self {
        Self {
            duration: 0.0,
            hold: 0.0,
            initial: [1.0; 4],
            final_color: [1.0; 4],
            scale: [1.0; 4],
            rotation: [0.0; 2],
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
    fn sample(self, elapsed: f64) -> Sample {
        let t = if self.duration > 0.0 {
            (elapsed / self.duration as f64).clamp(0.0, 1.0) as f32
        } else {
            1.0
        };
        Sample {
            color: Some(std::array::from_fn(|i| {
                lerp(self.initial[i], self.final_color[i], t)
            })),
            scale: [
                lerp(self.scale[0], self.scale[1], t),
                lerp(self.scale[2], self.scale[3], t),
            ],
            rotation: lerp(self.rotation[0], self.rotation[1], t).to_radians(),
            ..Default::default()
        }
    }
}

impl Effect {
    fn sample(&self) -> (Sample, bool) {
        let duration: f64 = self
            .phases
            .values()
            .map(|p| (p.duration + p.hold) as f64)
            .sum();
        let elapsed = self.elapsed - self.delay as f64;
        if elapsed < 0.0 {
            return (
                Sample {
                    alpha: 0.0,
                    ..Default::default()
                },
                true,
            );
        }
        let Some(last) = self.phases.last_key_value().map(|(_, p)| p) else {
            return (Sample::default(), false);
        };
        if duration <= 0.0 {
            return (last.sample(f64::MAX), false);
        }
        if self.repeat > 0 && elapsed >= duration * self.repeat as f64 {
            return (last.sample(f64::MAX), false);
        }
        let mut t = elapsed % duration;
        for p in self.phases.values() {
            let len = (p.duration + p.hold) as f64;
            if t < len {
                return (p.sample(t), true);
            }
            t -= len;
        }
        (last.sample(f64::MAX), true)
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
    let e = u.animations.effects.entry(id).or_insert_with(|| Effect {
        repeat: 1,
        ..Default::default()
    });
    change(e.phases.entry(a.f(1) as i64).or_default());
}

pub(super) fn effect_methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    m.set(
        "SetEffectPriority",
        method(lua, |lua, id, a| {
            phase(lua, id, a, |p| p.duration = positive(a.f(3)) * 1000.0);
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
            phase(lua, id, a, |p| p.hold = positive(a.f(2)) * 1000.0);
            none()
        })?,
    )?;
    m.set(
        "SetRepeatCount",
        method(lua, |lua, id, a| {
            ui(lua).animations.effects.entry(id).or_default().repeat = positive(a.f(1)) as u32;
            none()
        })?,
    )?;
    m.set(
        "SetInterval",
        method(lua, |lua, id, a| {
            ui(lua)
                .animations
                .effects
                .entry(id)
                .or_insert_with(|| Effect {
                    repeat: 1,
                    ..Default::default()
                })
                .delay = positive(a.f(1)) * 1000.0;
            none()
        })?,
    )?;
    m.set(
        "SetStartEffect",
        method(lua, |lua, id, a| {
            let on = a.b(1, true);
            let mut u = ui(lua);
            let old_sample = u.animations.sample(id);
            if on {
                let e = u.animations.effects.entry(id).or_insert_with(|| Effect {
                    repeat: 1,
                    ..Default::default()
                });
                e.elapsed = 0.0;
                let (sample, running) = e.sample();
                e.running = running;
                u.animations.samples.insert(id, sample);
                if running {
                    u.animations.active.insert(id);
                } else {
                    u.animations.active.remove(&id);
                }
                u.tree.get_mut(id).shown = true;
            } else {
                if let Some(e) = u.animations.effects.get_mut(&id) {
                    e.running = false;
                }
                u.animations.active.remove(&id);
            }
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
                ret(
                    lua,
                    ui(lua)
                        .animations
                        .effects
                        .get(&id)
                        .is_some_and(|e| e.running),
                )
            })?,
        )?;
    }
    m.set(
        "ClearEffect",
        method(lua, |lua, id, _| {
            let mut u = ui(lua);
            u.animations.remove(id);
            if let Some(d) = u.tree.drawable_mut(id) {
                d.effect = Some(false);
            }
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
        let (sample, running, hide) = if let Some(e) = u.animations.effects.get_mut(&id) {
            e.elapsed += positive(dt_ms) as f64;
            let (sample, running) = e.sample();
            e.running = running;
            (sample, running, false)
        } else if let Some(f) = u.animations.frames.get_mut(&id) {
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
    if repaint {
        u.tree.repaint();
    }
}
