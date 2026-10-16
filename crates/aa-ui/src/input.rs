//! Mouse input: hit testing by layer and paint order, then `OnEnter` / `OnLeave` / `OnClick` /
//! `OnMouseDown` / `OnMouseUp` / `OnWheelUp` / `OnWheelDown` into Lua, plus the native
//! behaviour of buttons (state art) and list controls (hover / selection).

use mlua::{Lua, Value};

use crate::lua::{self, call_handler, has_handler, ui};
use crate::tree::{Id, ROOT, WidgetType};

const MOUSE_HANDLERS: &[&str] = &[
    "OnClick",
    "OnEnter",
    "OnLeave",
    "OnMouseDown",
    "OnMouseUp",
    "OnWheelUp",
    "OnWheelDown",
    "OnDragStart",
    "OnDragStop",
    "OnMouseMove",
];

/// Mouse state kept between frames.
#[derive(Default)]
pub struct MouseState {
    pub pressed: Option<Id>,
    last_click: Option<(Id, f64)>,
}

/// Topmost widget under `(x, y)` (UI units) that takes the mouse.
pub fn pick(lua: &Lua, x: f32, y: f32) -> Option<Id> {
    let (order, modal) = {
        let u = ui(lua);
        let order = u.tree.paint_order();
        let modal = order.iter().rev().copied().find(|&id| {
            u.tree.widget(id).is_some_and(|w| w.modal) && u.tree.get(id).parent.is_none()
        });
        (order, modal)
    };
    for &id in order.iter().rev() {
        if id == ROOT {
            continue;
        }
        let (takes, inside) = {
            let u = ui(lua);
            let Some(w) = u.tree.widget(id) else { continue };
            if !w.clickable {
                continue;
            }
            if let Some(m) = modal {
                if !u.tree.is_descendant(id, m) {
                    continue;
                }
            }
            let r = u.tree.rect(id);
            if !r.contains(x, y) {
                continue;
            }
            let in_list = u
                .tree
                .get(id)
                .parent
                .and_then(|p| u.tree.widget(p))
                .is_some_and(|p| p.ty == WidgetType::ListCtrl);
            let native = w.ty.mouse_native() || in_list;
            let scripted = u
                .handlers
                .get(&id)
                .is_some_and(|hs| hs.iter().any(|h| MOUSE_HANDLERS.contains(&h.as_str())));
            (native || scripted, true)
        };
        if takes && inside {
            return Some(id);
        }
    }
    None
}

/// The list control row (list, row index) that contains `id`, if any.
fn list_row(lua: &Lua, mut id: Id) -> Option<(Id, usize)> {
    let u = ui(lua);
    loop {
        let parent = u.tree.get(id).parent?;
        if let Some(l) = u.tree.widget(parent).and_then(|w| w.list.as_ref()) {
            if let Some(i) = l.rows.iter().position(|&r| r == id) {
                return l
                    .has_data
                    .get(i)
                    .copied()
                    .unwrap_or(false)
                    .then_some((parent, i));
            }
        }
        id = parent;
    }
}

fn arg(lua: &Lua, s: &str) -> Value {
    Value::String(lua.create_string(s).unwrap())
}

/// Process one frame of mouse input. `cursor` is in UI units (`None` outside the window).
pub fn update(
    lua: &Lua,
    mouse: &mut MouseState,
    cursor: Option<(f32, f32)>,
    down: bool,
    up: bool,
    right_up: bool,
    wheel: f32,
) {
    let hover = cursor.and_then(|(x, y)| pick(lua, x, y));
    if let Some(c) = cursor {
        ui(lua).cursor = c;
    }
    let old = ui(lua).hover;
    if hover != old {
        {
            let mut u = ui(lua);
            u.hover = hover;
            u.tree.touch();
        }
        if let Some(o) = old {
            call_handler(lua, o, "OnLeave", vec![]);
        }
        if let Some(n) = hover {
            call_handler(lua, n, "OnEnter", vec![]);
        }
        update_list_hover(lua, old, hover);
    }
    if down {
        mouse.pressed = hover;
        ui(lua).tree.touch();
        if let Some(h) = hover {
            call_handler(lua, h, "OnMouseDown", vec![arg(lua, "LeftButton")]);
        }
    }
    if up {
        let pressed = mouse.pressed.take();
        ui(lua).tree.touch();
        if let Some(h) = hover {
            call_handler(lua, h, "OnMouseUp", vec![arg(lua, "LeftButton")]);
        }
        if let (Some(p), Some(h)) = (pressed, hover) {
            if ui(lua).tree.is_descendant(h, p) || h == p {
                click(lua, mouse, h, "LeftButton");
            }
        }
    }
    if right_up {
        if let Some(h) = hover {
            click(lua, mouse, h, "RightButton");
        }
    }
    if wheel != 0.0 {
        let name = if wheel > 0.0 {
            "OnWheelUp"
        } else {
            "OnWheelDown"
        };
        let mut t = hover;
        while let Some(id) = t {
            if has_handler(lua, id, name) {
                call_handler(lua, id, name, vec![]);
                break;
            }
            t = ui(lua).tree.get(id).parent;
        }
    }
}

fn click(lua: &Lua, mouse: &mut MouseState, target: Id, button: &str) {
    let now = ui(lua).time_ms;
    let double = mouse
        .last_click
        .is_some_and(|(id, t)| id == target && now - t < 350.0);
    mouse.last_click = Some((target, now));
    // Script OnClick on the target or the nearest ancestor that has one (enabled only).
    let mut t = Some(target);
    while let Some(id) = t {
        let enabled = ui(lua).tree.widget(id).is_some_and(|w| w.enabled);
        if has_handler(lua, id, "OnClick") {
            if enabled {
                call_handler(lua, id, "OnClick", vec![arg(lua, button)]);
            }
            break;
        }
        let is_button = ui(lua).tree.widget(id).is_some_and(|w| w.button.is_some());
        if is_button {
            break;
        }
        t = ui(lua).tree.get(id).parent;
    }
    // Native tab buttons.
    if button == "LeftButton" && lua::tab::button_clicked(lua, target) {
        lua::flush(lua);
        return;
    }
    // Native list selection.
    if button == "LeftButton" {
        if let Some((list, row)) = list_row(lua, target) {
            let enabled = ui(lua).tree.widget(list).is_some_and(|w| w.enabled);
            if enabled {
                lua::widget_set_selected(lua, list, row as i32, true, double);
                lua::flush(lua);
            }
        }
    }
}

fn update_list_hover(lua: &Lua, _old: Option<Id>, new: Option<Id>) {
    let lists: Vec<Id> = {
        let u = ui(lua);
        u.tree
            .objects
            .iter()
            .enumerate()
            .filter(|(_, o)| o.alive && o.widget().is_some_and(|w| w.list.is_some()))
            .map(|(i, _)| i as Id)
            .collect()
    };
    let hovered_row = new.and_then(|h| list_row(lua, h));
    for l in lists {
        let mut g = ui(lua);
        let u = &mut *g;
        let row = hovered_row
            .filter(|(list, _)| *list == l)
            .map(|(_, r)| r as i32)
            .unwrap_or(-1);
        let Some(ld) = u.tree.widget_mut(l).and_then(|w| w.list.as_mut()) else {
            continue;
        };
        if ld.overed == row {
            continue;
        }
        ld.overed = row;
        let img = ld.overed_image;
        let r = (row >= 0).then(|| ld.rows[row as usize]);
        if let Some(img) = img {
            lua::widget_place_on_row(u, img, r);
        }
    }
}
