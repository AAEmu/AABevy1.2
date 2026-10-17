//! `damagedisplay` combat text: native 39288530 (frame parser), 39286710 (start),
//! 39287500 (render/timeline), 39286860 (shake), and 39286920 (afterimages).
//! Only active displays are sampled. Position/animation samples never rewrite anchors.

use std::collections::{BTreeSet, HashMap};

use mlua::{Function, Lua, Table, Value};

use super::{Ui, method, none, ui};
use crate::tree::{Align, Id, Rect, Widget};

#[derive(Default)]
pub struct Damage {
    states: HashMap<Id, State>,
    active: BTreeSet<Id>,
}

#[derive(Default)]
struct State {
    source: String,
    target: String,
    init: [f32; 2],
    frames: Vec<Frame>,
    index: usize,
    t0: f64,
    moved: [f32; 2],
    sample: Sample,
}

struct Frame {
    kind: u32,
    hold: f64,
    transition: f64,
    scale: f32,
    alpha: f32,
    movement: [f32; 2],
    after: Vec<Afterimage>,
    shake_time: f64,
    shake_count: f64,
    shake: [f32; 2],
    linear_time: f64,
}

#[derive(Clone, Debug, PartialEq)]
struct Afterimage {
    time: f64,
    offset: [f32; 2],
    alpha: f32,
    scale: f32,
}

#[derive(Clone, Debug, PartialEq)]
struct Sample {
    visible: bool,
    text: Option<String>,
    offset: [f32; 2],
    scale: f32,
    alpha: f32,
    after: Vec<Afterimage>,
}

impl Default for Sample {
    fn default() -> Self {
        Self {
            visible: true,
            text: None,
            offset: [0.0; 2],
            scale: 1.0,
            alpha: 1.0,
            after: Vec::new(),
        }
    }
}

fn number(table: &Table, key: &str, default: f64) -> mlua::Result<f64> {
    Ok(table
        .raw_get::<Option<f64>>(key)?
        .filter(|x| x.is_finite())
        .unwrap_or(default))
}

fn frames(table: Table) -> mlua::Result<Vec<Frame>> {
    let mut frames = Vec::new();
    for table in table.sequence_values::<Table>() {
        let table = table?;
        let kind = number(&table, "animType", 0.0)? as u32;
        let mut after = Vec::new();
        if kind & 4 != 0
            && let Some(list) = table.raw_get::<Option<Table>>("afterImage")?
        {
            for image in list.sequence_values::<Table>() {
                let image = image?;
                after.push(Afterimage {
                    time: number(&image, "time", 0.0)?,
                    offset: [
                        number(&image, "x", 0.0)? as f32,
                        number(&image, "y", 0.0)? as f32,
                    ],
                    alpha: number(&image, "alpha", 1.0)? as f32,
                    scale: number(&image, "scale", 1.0)? as f32,
                });
            }
        }
        frames.push(Frame {
            kind,
            hold: number(&table, "showTime", 0.0)?,
            transition: number(&table, "timeToNextFrame", 0.0)?,
            scale: (number(&table, "scale", 1.0)? as f32).max(0.1),
            alpha: number(&table, "alpha", 1.0)? as f32,
            movement: [
                number(&table, "moveX", 0.0)? as f32,
                number(&table, "moveY", 0.0)? as f32,
            ],
            after,
            shake_time: number(&table, "shakeTime", 0.0)?,
            shake_count: number(&table, "shakeCount", 1.0)?,
            shake: [
                number(&table, "shakeX", 0.0)? as f32,
                number(&table, "shakeY", 0.0)? as f32,
            ],
            linear_time: number(&table, "linearDisplayTime", 0.0)?,
        });
    }
    Ok(frames)
}

pub(super) fn methods(lua: &Lua, methods: &Table) -> mlua::Result<()> {
    methods.raw_set(
        "SetUnitId",
        method(lua, |lua, id, args| {
            let mut u = ui(lua);
            let state = u.damage.states.entry(id).or_default();
            state.source = args.s(1).unwrap_or_default();
            state.target = args.s(2).unwrap_or_default();
            none()
        })?,
    )?;
    methods.raw_set(
        "SetInitPos",
        method(lua, |lua, id, args| {
            // Native Lua binding truncates both coordinates to signed integers.
            ui(lua).damage.states.entry(id).or_default().init =
                [args.f(1) as i32 as f32, args.f(2) as i32 as f32];
            none()
        })?,
    )?;
    methods.raw_set(
        "SetAnimFrameInfo",
        method(lua, |lua, id, args| {
            let frames = match args.v(1) {
                Value::Table(t) => frames(t)?,
                _ => Vec::new(),
            };
            ui(lua).damage.states.entry(id).or_default().frames = frames;
            none()
        })?,
    )?;
    methods.raw_set(
        "Animation",
        method(lua, |lua, id, args| {
            let on = args.b(1, false);
            {
                let mut u = ui(lua);
                if on && u.damage.active.contains(&id) {
                    return none();
                }
                if on {
                    let now = u.time_ms;
                    let state = u.damage.states.entry(id).or_default();
                    state.t0 = now;
                    state.moved = [0.0; 2];
                    state.sample = Sample::default();
                    if let Some(first) = state.frames.get(state.index) {
                        state.sample.scale = first.scale;
                        state.sample.alpha = first.alpha;
                    }
                    u.damage.active.insert(id);
                    u.tree.repaint();
                }
            }
            super::widget::show(lua, id, on, None);
            // Position immediately, but phase advancement starts with the next render tick.
            if on {
                update_one(lua, id, false);
            }
            none()
        })?,
    )?;
    Ok(())
}

pub(super) fn hide(u: &mut Ui, id: Id) {
    if let Some(state) = u.damage.states.get_mut(&id) {
        state.index = 0;
        state.t0 = 0.0;
        state.source.clear();
        state.target.clear();
        state.sample = Sample::default();
    }
    u.damage.active.remove(&id);
    u.animations.set_widget_offset(id, [0.0; 2]);
}

pub(crate) fn visible(u: &Ui, id: Id) -> bool {
    u.damage.states.get(&id).is_none_or(|s| s.sample.visible)
}

pub(super) fn tick(lua: &Lua) {
    let active: Vec<Id> = ui(lua).damage.active.iter().copied().collect();
    for id in active {
        let (alive, shown, visible) = {
            let u = ui(lua);
            (
                u.tree.get(id).alive,
                u.tree.get(id).shown,
                u.tree.visible(id),
            )
        };
        if !alive || !shown {
            let mut u = ui(lua);
            hide(&mut u, id);
            if !alive {
                u.damage.states.remove(&id);
            }
        } else if visible {
            update_one(lua, id, true);
        }
    }
}

fn update_one(lua: &Lua, id: Id, advance: bool) {
    let (source, target) = {
        let u = ui(lua);
        let Some(state) = u.damage.states.get(&id) else {
            return;
        };
        (state.source.clone(), state.target.clone())
    };
    let hook = lua.globals().raw_get::<Table>("AAUI").ok().and_then(|t| {
        t.raw_get::<Option<Function>>("CombatTextPosition")
            .ok()
            .flatten()
    });
    let projected = match hook {
        Some(hook) => match hook.call::<Value>((source, target)) {
            Ok(Value::Table(t)) => {
                let point = || -> mlua::Result<([f32; 2], f32)> {
                    Ok((
                        [t.raw_get("x")?, t.raw_get("y")?],
                        t.raw_get::<Option<f32>>("alpha")?.unwrap_or(1.0),
                    ))
                };
                point()
                    .ok()
                    .filter(|(p, a)| p.iter().all(|v| v.is_finite()) && a.is_finite())
            }
            Ok(_) => None,
            Err(e) => {
                bevy::log::warn!("aa-ui: AAUI.CombatTextPosition: {e}");
                None
            }
        },
        None => {
            let u = ui(lua);
            let r = u.tree.rect(crate::tree::ROOT);
            Some((
                [
                    (r.w * u.ui_scale * 0.5).trunc(),
                    (r.h * u.ui_scale * 0.5).trunc(),
                ],
                1.0,
            ))
        }
    };
    let ended = {
        let mut u = ui(lua);
        let before = u.damage.states[&id].sample.clone();
        let offset_before = u.animations.widget_sample(id).offset;
        let now = u.time_ms;
        let text = u
            .tree
            .widget(id)
            .and_then(|w| w.text.as_ref())
            .map(|t| t.text.clone())
            .unwrap_or_default();
        let rect = u.tree.rect(id);
        let ui_scale = u.ui_scale;
        let state = u.damage.states.get_mut(&id).unwrap();
        let mut offset = offset_before;
        let ended = if let Some((point, alpha)) = projected {
            offset = [
                point[0] / ui_scale + state.init[0] / ui_scale - rect.x - rect.w * 0.5,
                point[1] / ui_scale + state.init[1] / ui_scale - rect.y - rect.h * 0.5,
            ];
            if text.is_empty() {
                false
            } else {
                state.update(now, &text, alpha, advance)
            }
        } else {
            // No projected anchor suppresses drawing, but must not strand a finite pool
            // slot. Native 39287500 proceeds from projection into the authored timeline;
            // only explicit distance culling hides the widget before phase sampling.
            let ended = !text.is_empty() && state.update(now, &text, 1.0, advance);
            state.sample.visible = false;
            ended
        };
        let changed = state.sample != before || offset != offset_before;
        u.animations.set_widget_offset(id, offset);
        if changed {
            u.tree.repaint();
        }
        ended
    };
    if ended {
        super::widget::show(lua, id, false, None);
    }
}

impl State {
    fn update(&mut self, now: f64, text: &str, distance_alpha: f32, advance: bool) -> bool {
        let Some(frame) = self.frames.get(self.index) else {
            self.sample.visible = false;
            return false; // Native stays shown with an empty frame table.
        };
        let age = now - self.t0;
        let fraction = if frame.transition > 0.0 {
            ((age - frame.hold) / frame.transition) as f32
        } else {
            1.0
        };
        let mut display = None;
        if self.index == 0 {
            if frame.kind & 2 != 0 {
                let trimmed = text.trim_start();
                let n: String = trimmed
                    .chars()
                    .enumerate()
                    .take_while(|&(i, c)| c.is_ascii_digit() || (i == 0 && (c == '-' || c == '+')))
                    .map(|(_, c)| c)
                    .collect();
                if let Ok(n) = n.parse::<i32>()
                    && n != 0
                {
                    display = Some(((n as f32 * fraction) as i32).to_string());
                }
            }
            if frame.kind & 16 != 0 && frame.linear_time >= 1.0 {
                let chars: Vec<char> = text.chars().collect();
                let count = (age.max(0.0) as usize / frame.linear_time as usize).saturating_add(1);
                let spaces = chars.iter().take(count).filter(|&&c| c == ' ').count();
                display = Some(
                    chars
                        .into_iter()
                        .take(count.saturating_add(spaces))
                        .collect(),
                );
            }
        }
        let mut scale = frame.scale;
        let mut alpha = frame.alpha;
        let mut movement = [0.0; 2];
        let mut shake = [0.0; 2];
        if frame.kind & 8 != 0 && age <= frame.shake_time && frame.shake_count > 0.0 {
            let period = frame.shake_time / frame.shake_count;
            let divisor = period as i64;
            if divisor > 0 {
                let remainder = (frame.shake_time - age) as i64 % divisor;
                let sign = if (remainder as f64) < period * 0.5 {
                    1.0
                } else {
                    -1.0
                };
                shake = frame.shake.map(|v| v * sign);
            }
        }
        if age >= frame.hold && age < frame.hold + frame.transition {
            if let Some(next) = self.frames.get(self.index + 1) {
                scale += (next.scale - scale) * fraction;
                alpha += (next.alpha - alpha) * fraction;
            }
            if frame.kind & 1 != 0 {
                movement = frame.movement.map(|v| v * fraction);
            }
        } else if advance && age >= frame.hold + frame.transition {
            // The native advances exactly one phase per render and discards overshoot.
            if frame.kind & 1 != 0 {
                for (sum, delta) in self.moved.iter_mut().zip(frame.movement) {
                    *sum += delta;
                }
            }
            self.index += 1;
            let Some(next) = self.frames.get(self.index) else {
                return true;
            };
            self.t0 = now;
            scale = next.scale;
            alpha = next.alpha;
        }
        let frame = &self.frames[self.index];
        let after = if frame.kind & 4 != 0 {
            frame
                .after
                .iter()
                .filter(|a| now - self.t0 <= a.time)
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        self.sample = Sample {
            visible: true,
            text: display,
            scale,
            alpha: alpha * distance_alpha,
            offset: std::array::from_fn(|i| self.moved[i] + movement[i] + shake[i]),
            after,
        };
        false
    }
}

pub(crate) fn quads(
    u: &mut Ui,
    id: Id,
    widget: &Widget,
    alpha: f32,
    ui_scale: f32,
    out: &mut Vec<crate::render::Quad>,
) {
    let Some(text) = widget.text.as_ref() else {
        return;
    };
    let sample = u
        .damage
        .states
        .get(&id)
        .map(|s| s.sample.clone())
        .unwrap_or_default();
    if !sample.visible {
        return;
    }
    let mut text = text.clone();
    if let Some(display) = sample.text {
        text.text = display;
    }
    if text.text.is_empty() {
        return;
    }
    if !u.fonts.supports(&text.style.font, &text.text)
        && let Some(extra) = widget.aux_style.as_ref()
    {
        text.style = extra.style.clone();
    }
    let r = u.tree.rect(id);
    let x = r.x + (r.w + text.inset[0] - text.inset[2]) * 0.5 + sample.offset[0] / ui_scale;
    let y = r.y + (r.h + text.inset[1] - text.inset[3]) * 0.5 + sample.offset[1] / ui_scale;
    text.inset = [0.0; 4];
    text.wrap = false;
    text.style.align = Align::Top;
    // Afterimage alpha is the authored alpha, independent of the current frame's alpha.
    for image in sample.after {
        let rect = Rect {
            x: x + image.offset[0] / ui_scale,
            y: y + image.offset[1] / ui_scale,
            w: 0.0,
            h: 0.0,
        };
        let mut color = text.style.color;
        color[3] = image.alpha;
        scaled_text(
            u,
            &text,
            rect,
            color,
            alpha,
            ui_scale,
            sample.scale * image.scale,
            out,
        );
    }
    let mut color = text.style.color;
    color[3] = sample.alpha;
    scaled_text(
        u,
        &text,
        Rect {
            x,
            y,
            w: 0.0,
            h: 0.0,
        },
        color,
        alpha,
        ui_scale,
        sample.scale,
        out,
    );
}

fn scaled_text(
    u: &mut Ui,
    text: &crate::tree::TextBlock,
    rect: Rect,
    color: [f32; 4],
    alpha: f32,
    ui_scale: f32,
    scale: f32,
    out: &mut Vec<crate::render::Quad>,
) {
    if scale <= 0.0 {
        return;
    }
    let start = out.len();
    crate::render::text_quads(u, text, rect, color, alpha, ui_scale, out);
    // Native text draw scales existing glyph art; it does not rasterize a new font size
    // for every animation sample. This also bounds atlas/cache growth during combat.
    let pivot = [rect.x * ui_scale, rect.y * ui_scale];
    for q in &mut out[start..] {
        for i in 0..4 {
            q.pos[i] = pivot[i % 2] + (q.pos[i] - pivot[i % 2]) * scale;
        }
    }
}
