//! The ordinary slot's private completion child: native392c32f3/3928e6c0.
//! Its phase clock advances during admitted painting, including alpha-zero painting.

use mlua::{Lua, Table, Value};

use super::{Ui, ui};
use crate::render::{Quad, Tex};
use crate::tree::{Id, Widget};

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Completion {
    total_ms: f32,
    phase: u8,
    elapsed: f32,
    carry: f32,
    finished: bool,
    running: bool,
    scale: f32,
    alpha: f32,
}

impl Completion {
    fn new(total_ms: f32) -> Self {
        Self {
            total_ms,
            phase: 0,
            elapsed: 0.,
            carry: 0.,
            finished: false,
            running: true,
            scale: 0.3,
            alpha: 0.,
        }
    }

    fn advance(&mut self, dt_ms: f32) {
        if !self.running {
            return;
        }
        let dt = dt_ms / 1000.;
        if self.finished {
            if self.phase == 0 {
                self.phase = 1;
                self.elapsed = dt + self.carry;
                self.carry = 0.;
                self.finished = false;
                self.scale = 1.;
                self.alpha = 0.9;
            } else {
                self.running = false;
            }
            return;
        }
        self.elapsed += dt;
        let (initial, final_alpha, velocity_time, acceleration_time) = if self.phase == 0 {
            (0., 1., 0.5, 0.4)
        } else {
            (0.9, 0., 0.3, 0.2)
        };
        let delta = final_alpha - initial;
        let velocity = delta / velocity_time;
        let acceleration =
            2. * (delta - velocity * acceleration_time) / (acceleration_time * acceleration_time);
        // Native coefficients/elapsed are f32;3928a8d0 evaluates in x87 before its f32 spill.
        let t = f64::from(self.elapsed);
        let displacement =
            (f64::from(velocity) * t + (f64::from(acceleration) * 0.5) * (t * t)) as f32;
        let current = initial + displacement;
        if (delta > 0. && current >= final_alpha) || (delta < 0. && current <= final_alpha) {
            self.finished = true;
            self.carry = self.elapsed - acceleration_time;
            self.scale = 1.;
            self.alpha = final_alpha;
        } else {
            let fraction = ((current - initial) / delta).abs();
            self.alpha = initial + delta * fraction;
            self.scale = if self.phase == 0 {
                0.3 + (1. - 0.3) * fraction
            } else {
                1.
            };
        }
    }
}

const ACTIVE: &str = "aa_ui_slot_completion_children";
fn active(lua: &Lua) -> mlua::Result<Table> {
    if let Ok(t) = lua.named_registry_value(ACTIVE) {
        return Ok(t);
    }
    let t = lua.create_table()?;
    lua.set_named_registry_value(ACTIVE, &t)?;
    Ok(t)
}

pub(super) fn untrack(lua: &Lua, id: Id) {
    if let Ok(t) = active(lua) {
        let _ = t.raw_set(id, Value::Nil);
    }
}

pub(super) fn start(lua: &Lua, id: Id, total_ms: f32) {
    let mut u = ui(lua);
    if !u.tree.objects.get(id as usize).is_some_and(|o| o.alive) {
        return;
    }
    if let Some(s) = u.tree.widget_mut(id).and_then(|w| w.slot.as_mut()) {
        s.completion = Some(Completion::new(total_ms));
        if let Ok(t) = active(lua) {
            let _ = t.raw_set(id, true);
        }
    }
}

fn admitted(u: &Ui, w: &Widget, effect: &Completion) -> bool {
    let (total, remain) = w.cooldown.as_ref().map_or((effect.total_ms, 0.), |c| {
        (
            c.total_ms,
            c.sampled_ms.map(f64::from).unwrap_or(c.end_ms - u.time_ms) as f32,
        )
    });
    // Native parent draw gate, followed by392c3960's completion-child branch.
    !(total < super::LABEL_MIN_TOTAL_MS)
        && (w.slot.as_ref().is_some_and(|s| s.cooldown_mask.is_none()) || remain <= 0.)
}

pub(super) fn has(w: &Widget) -> bool {
    w.slot.as_ref().is_some_and(|s| s.completion.is_some())
}

/// Requests painting only. Visibility and layer membership are decided by the actual traversal.
pub(super) fn invalidate(lua: &Lua) {
    let Ok(t) = active(lua) else { return };
    let ids: Vec<Id> = t
        .pairs::<Id, bool>()
        .filter_map(|p| p.ok().map(|p| p.0))
        .collect();
    let mut u = ui(lua);
    let mut repaint = false;
    for id in ids {
        let Some(o) = u.tree.objects.get(id as usize).filter(|o| o.alive) else {
            untrack(lua, id);
            continue;
        };
        let Some((w, c)) = o
            .widget()
            .and_then(|w| w.slot.as_ref()?.completion.as_ref().map(|c| (w, c)))
        else {
            untrack(lua, id);
            continue;
        };
        if !c.running {
            untrack(lua, id);
            continue;
        }
        repaint |= u.tree.visible(id) && admitted(&u, w, c);
    }
    if repaint {
        u.tree.repaint();
    }
}

/// Called once per actual paint traversal, before its alpha skip and scissor handling.
pub(super) fn draw_frame(u: &mut Ui, id: Id) {
    let Some(w) = u.tree.widget(id) else { return };
    let Some(c) = w.slot.as_ref().and_then(|s| s.completion.as_ref()) else {
        return;
    };
    if !c.running || !admitted(u, w, c) {
        return;
    }
    let dt = u.frame_ms;
    if let Some(c) = u
        .tree
        .widget_mut(id)
        .and_then(|w| w.slot.as_mut())
        .and_then(|s| s.completion.as_mut())
    {
        c.advance(dt);
    }
}

pub(super) fn quads(u: &mut Ui, id: Id, w: &Widget, alpha: f32, scale: f32, out: &mut Vec<Quad>) {
    let Some(c) = w.slot.as_ref().and_then(|s| s.completion.as_ref()) else {
        return;
    };
    if c.alpha <= 0. || !admitted(u, w, c) {
        return;
    }
    const HUD: &str = "ui/common/hud.dds";
    let Some(path) = u.textures.resolve(HUD) else {
        return;
    };
    let Some((tw, th)) = u.textures.size(HUD) else {
        return;
    };
    let r = u.tree.rect(id);
    let rect = [
        (r.x + 1.) * scale,
        (r.y + 1.) * scale,
        (r.right() - 1.) * scale,
        (r.bottom() - 1.) * scale,
    ];
    let center = [(rect[0] + rect[2]) * 0.5, (rect[1] + rect[3]) * 0.5];
    let translation = [
        center[0] - center[0] * c.scale,
        center[1] - center[1] * c.scale,
    ];
    let xs = [rect[0], rect[0] + 6. * scale, rect[2] - 6. * scale, rect[2]];
    let ys = [rect[1], rect[1] + 7. * scale, rect[3] - 8. * scale, rect[3]];
    let us = [743., 749., 750., 756.].map(|v| v / tw as f32);
    let vs = [35., 42., 43., 51.].map(|v| v / th as f32);
    let color = [
        1.,
        1.,
        1.,
        ((f64::from((c.alpha * alpha).clamp(0., 1.)) * 255.).trunc() / 255.) as f32,
    ];
    for y in 0..3 {
        for x in 0..3 {
            let corners = [
                [xs[x], ys[y]],
                [xs[x + 1], ys[y]],
                [xs[x + 1], ys[y + 1]],
                [xs[x], ys[y + 1]],
            ]
            .map(|p| std::array::from_fn(|i| ((p[i] * c.scale + translation[i]) + 0.5).trunc()));
            out.push(Quad {
                tex: Tex::Image(path.clone()),
                pos: [corners[0][0], corners[0][1], corners[2][0], corners[2][1]],
                uv: [us[x], vs[y], us[x + 1], vs[y + 1]],
                color,
                corners: Some(corners),
                uvs: None,
                vcolors: None,
            });
        }
    }
}
