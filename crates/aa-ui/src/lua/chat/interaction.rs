//! Native drag frame preparation, insertion caret and overflow sliding.
use super::*;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Interaction {
    pub caret: Id,
    pub caret_offset: [f32; 2],
    pub preview: Option<(Id, bool)>,
    pub before: bool,
    pub slides: [Id; 2],
    pub use_slides: bool,
    pub slide_ms: f32,
    pub offset: f32,
    pub moving: bool,
}

pub(super) fn begin(lua: &Lua, button: Id, window: Id) {
    if !ui(lua).tree.get(window).alive {
        return;
    }
    if let Some(c) = ui(lua)
        .tree
        .widget_mut(window)
        .and_then(|w| w.chat.as_mut())
    {
        c.interaction.moving = true;
    }
    ui(lua).drag.chat = Some((button, window));
    //39502100 /395128e0: hidden chat inputs freeze three drawable layers at OVER.
    let windows = ui(lua).tree.of_class("chatwindow");
    for id in windows {
        let Some(c) = chat(lua, id) else { continue };
        if ui(lua).tree.get(c.edit).shown {
            continue;
        }
        alpha::configure(lua, id);
        for layer in 0..3 {
            alpha::enable(lua, id, layer, false);
            alpha::force(lua, id, layer, true);
        }
    }
}

pub(super) fn end(lua: &Lua) {
    end_on(lua, None);
}

pub(super) fn finish(lua: &Lua, button: Id) {
    let owner = {
        let u = ui(lua);
        tab_owner(&u, button).map(|p| p.0)
    };
    end_on(lua, owner);
}

fn end_on(lua: &Lua, owner: Option<Id>) {
    let Some((_, moving)) = ui(lua).drag.chat.take() else {
        return;
    };
    let moving = owner.unwrap_or(moving);
    let windows = ui(lua).tree.of_class("chatwindow");
    for id in windows {
        let Some(c) = chat(lua, id) else { continue };
        {
            let mut u = ui(lua);
            if let Some(c) = u.tree.widget_mut(id).and_then(|w| w.chat.as_mut()) {
                c.interaction.moving = false;
                c.interaction.preview = None;
                c.interaction.before = false;
            }
            u.tree.get_mut(c.interaction.caret).shown = false;
        }
        if ui(lua).tree.get(c.edit).shown {
            continue;
        }
        //395021b0: enable then force only the moved owner's layers to OVER.
        alpha::configure(lua, id);
        for layer in 0..3 {
            alpha::force(lua, id, layer, id == moving);
        }
    }
}

pub(super) fn strip(u: &super::super::Ui, id: Id, c: &ChatData, p: (f32, f32)) -> bool {
    let r = u.animations.widget_rect(&u.tree, id);
    let scale = u.animations.widget_scale(&u.tree, id);
    p.0 >= r.x
        && p.0 <= r.right()
        && p.1 >= r.y + c.tab_clip_vertical[0] * scale
        && p.1 <= r.y + c.tab_clip_vertical[1] * scale
}

///392ca6c0: nearest center, ties retain the first row; equality chooses RIGHT.
pub(super) fn nearest(u: &super::super::Ui, c: &ChatData, p: (f32, f32)) -> Option<(usize, bool)> {
    let mut nearest = None;
    let mut best = f32::MAX;
    for (i, tab) in c.tabs.iter().enumerate() {
        if !u.tree.get(tab.button).alive {
            continue;
        }
        let r = u.animations.widget_rect(&u.tree, tab.button);
        let center = (r.x + r.w * 0.5, r.y + r.h * 0.5);
        let dx = f64::from(center.0) - f64::from(p.0);
        let dy = f64::from(center.1) - f64::from(p.1);
        let distance = (dx * dx + dy * dy) as f32;
        if distance < best {
            best = distance;
            nearest = Some((i, center.0 > p.0));
        }
    }
    nearest
}

pub(super) fn slide(lua: &Lua, id: Id, right: bool) {
    let Some(c) = chat(lua, id) else { return };
    let (wr, first, last) = {
        let u = ui(lua);
        let (Some(first), Some(last)) = (c.tabs.first(), c.tabs.last()) else {
            return;
        };
        let last = if c.use_add_button {
            c.add_button
        } else {
            last.button
        };
        (
            u.tree.rect(id),
            u.tree.rect(first.button),
            u.tree.rect(last),
        )
    };
    let offset = if right {
        if last.right() <= wr.right() {
            return;
        }
        if last.right() - c.tab_width < wr.right() {
            wr.w - (last.right() - first.x + c.tab_offset)
        } else {
            c.interaction.offset - c.tab_width
        }
    } else {
        if first.x >= wr.x {
            return;
        }
        if first.x + c.tab_width > wr.x {
            0.
        } else {
            c.interaction.offset + c.tab_width
        }
    };
    if let Some(c) = ui(lua).tree.widget_mut(id).and_then(|w| w.chat.as_mut()) {
        c.interaction.offset = offset;
    }
    layout(lua, id);
}

pub(super) fn update(lua: &Lua, dt_ms: f32) {
    let session = ui(lua).drag.chat;
    let Some((button, moving)) = session else {
        return;
    };
    let valid = {
        let u = ui(lua);
        u.tree.get(button).alive && u.tree.visible(moving) && u.drag.moving_window() == Some(moving)
    };
    if !valid {
        end(lua);
        return;
    }
    let windows = ui(lua).tree.of_class("chatwindow");
    for id in windows {
        let Some(c) = chat(lua, id).filter(|c| c.allow_tab_switch) else {
            continue;
        };
        //392ca150 retains elapsed time across exits and even when not over an arrow.
        let repeat = {
            let mut u = ui(lua);
            let elapsed = u.drag.chat_elapsed_ms.entry(id).or_default();
            *elapsed += dt_ms;
            let repeat = *elapsed > c.interaction.slide_ms;
            if repeat {
                *elapsed = 0.;
            }
            repeat
        };
        if repeat {
            let hover = ui(lua).hover;
            let side = c.interaction.slides.iter().position(|&b| hover == Some(b));
            if let Some(side) = side {
                slide(lua, id, side == 1);
                continue;
            }
        }
        let preview = {
            let u = ui(lua);
            let p = u.cursor;
            if !c.interaction.moving && u.tree.visible(id) && strip(&u, id, &c, p) {
                nearest(&u, &c, p)
                    .filter(|&(i, left)| c.tabs[i].id != 0 || !left)
                    .map(|(i, left)| (c.tabs[i].button, left))
            } else {
                None
            }
        };
        let mut u = ui(lua);
        if c.interaction.preview != preview {
            if let Some((target, left)) = preview {
                u.tree.get_mut(c.interaction.caret).anchors = vec![Anchor {
                    point: Point::Center,
                    target,
                    rel: if left {
                        Point::TopLeft
                    } else {
                        Point::TopRight
                    },
                    x: c.interaction.caret_offset[0],
                    y: c.interaction.caret_offset[1],
                }];
            }
            if let Some(c) = u.tree.widget_mut(id).and_then(|w| w.chat.as_mut()) {
                c.interaction.preview = preview;
                if let Some((_, left)) = preview {
                    c.interaction.before = left;
                }
            }
        }
        let shown = preview.is_some();
        if u.tree.get(c.interaction.caret).shown != shown {
            u.tree.get_mut(c.interaction.caret).shown = shown;
        }
    }
}
