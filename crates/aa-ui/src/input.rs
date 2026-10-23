//! Mouse input: hit testing by layer and paint order, then `OnEnter` / `OnLeave` / `OnClick` /
//! `OnMouseDown` / `OnMouseUp` / `OnWheelUp` / `OnWheelDown` into Lua, plus the native
//! behaviour of buttons (state art), check buttons (toggle + `OnCheckChanged`), edit boxes
//! (keyboard focus, see [`crate::lua::edit`]), list controls (hover / selection) and sliders
//! (press on the track or thumb and drag: the value follows the cursor, `OnSliderChanged`). A press on a
//! widget registered with `RegisterForDrag` that moves on starts a drag ([`crate::lua::drag`]).
//! The wheel (`update`'s `wheel`, > 0 = up; `UiVirtualPointer::wheel` adds to the OS wheel) goes to
//! the `OnWheelUp` / `OnWheelDown` handler of the widget under the cursor or its nearest ancestor
//! with one; when no script takes it, the message frame under the cursor (the chat) scrolls
//! (`crate::lua::chat::wheel`).

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
    "OnDragReceive",
    "OnDragStop",
    "OnMouseMove",
];

/// Mouse state kept between frames.
#[derive(Default)]
pub struct MouseState {
    pub pressed: Option<Id>,
    last_click: Option<(Id, f64)>,
    /// The slider being dragged (captured until the button is released).
    slider: Option<Id>,
    last_cursor: Option<(f32, f32)>,
    /// The left button went down on a drag source and is still down.
    drag: Option<DragPress>,
    /// Native covered-border presses retain the child identity for release/click
    /// admission (392a71e0), independently of the owner's sizing capture.
    child_resize: bool,
}

/// Mouse button edges for this frame. Right clicks still activate on release.
#[derive(Clone, Copy, Debug, Default)]
pub struct MouseButtons {
    pub press: bool,
    pub release: bool,
    pub right_press: bool,
    pub right_release: bool,
}

/// A left button press on a widget registered with `RegisterForDrag` ([`lua::drag`]).
#[derive(Clone, Copy)]
struct DragPress {
    src: Id,
    at: (f32, f32),
    /// The pointer moved far enough: `OnDragStart` was sent.
    started: bool,
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
            if let Some(m) = modal
                && !u.tree.is_descendant(id, m)
            {
                continue;
            }
            let r = u.animations.widget_rect(&u.tree, id);
            if !r.contains(x, y) {
                continue;
            }
            // Scrolled out of its scroll window (`EnableScroll`): not there.
            if u.animations.widget_clip_rect(&u.tree, id).is_some_and(|c| !c.contains(x, y)) {
                continue;
            }
            if lua::chat::tab_clip(&u, id).is_some_and(|c| {
                let (x, y) = (x * u.ui_scale, y * u.ui_scale);
                // Native 3927c795..7bc rejects only strictly outside the area.
                x < c[0] || y < c[1] || x > c[2] || y > c[3]
            }) {
                continue;
            }
            let in_list = u
                .tree
                .get(id)
                .parent
                .and_then(|p| u.tree.widget(p))
                .is_some_and(|p| p.ty == WidgetType::ListCtrl);
            let native = w.ty.mouse_native()
                || w.slot.is_some()
                || u.drag.registered.contains(&id)
                || lua::resize::edge(&u, id, (x, y)).is_some()
                || in_list;
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

/// `id` and its ancestors whose rectangle holds the cursor, outermost first (not the root).
fn mouse_chain(lua: &Lua, id: Id, x: f32, y: f32) -> Vec<Id> {
    let u = ui(lua);
    let mut chain = vec![id];
    let mut t = u.tree.get(id).parent;
    while let Some(p) = t {
        if p == ROOT {
            break;
        }
        if u.tree.rect(p).contains(x, y) {
            chain.push(p);
        }
        t = u.tree.get(p).parent;
    }
    chain.reverse();
    chain
}

/// The list control row (list, row index) that contains `id`, if any.
fn list_row(lua: &Lua, mut id: Id) -> Option<(Id, usize)> {
    let u = ui(lua);
    loop {
        let parent = u.tree.get(id).parent?;
        if let Some(l) = u.tree.widget(parent).and_then(|w| w.list.as_ref())
            && let Some(i) = l.rows.iter().position(|&r| r == id)
        {
            return l
                .has_data
                .get(i)
                .copied()
                .unwrap_or(false)
                .then_some((parent, i));
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
    update_buttons(
        lua,
        mouse,
        cursor,
        MouseButtons {
            press: down,
            release: up,
            right_release: right_up,
            ..Default::default()
        },
        wheel,
    );
}

/// Process mouse input including both right-button edges. Native mouse handlers run before
/// `MOUSE_DOWN` / `MOUSE_UP`, whose argument is the target's `GetId()` identity.
pub fn update_buttons(
    lua: &Lua,
    mouse: &mut MouseState,
    cursor: Option<(f32, f32)>,
    buttons: MouseButtons,
    wheel: f32,
) {
    // Native destruction clears the GUI manager's captured widget references
    // immediately (392aa7e0). MouseState lives outside Lua: discard its retired
    // references before dispatching another frame, including a release edge.
    {
        let u = ui(lua);
        let alive = |id: Id| u.tree.objects.get(id as usize).is_some_and(|o| o.alive);
        mouse.pressed = mouse.pressed.filter(|&id| alive(id));
        mouse.last_click = mouse.last_click.filter(|&(id, _)| alive(id));
        mouse.slider = mouse.slider.filter(|&id| alive(id));
        mouse.drag = mouse.drag.filter(|d| alive(d.src));
    }
    let MouseButtons {
        press: down,
        release: up,
        right_press: right_down,
        right_release: right_up,
    } = buttons;
    if let Some(c) = cursor {
        ui(lua).cursor = c;
        lua::drag::move_to(lua, c);
    }
    let hover = cursor.and_then(|(x, y)| pick(lua, x, y));
    let old = ui(lua).hover;
    if hover != old {
        {
            let mut u = ui(lua);
            u.hover = hover;
            u.tree.touch();
        }
        // The client's mouse over is the hovered widget and its ancestors under the cursor
        // (reference/ingame/tooltip_*: the action bar window is "mouse over" with its slot, so
        // its OnEnter shows the lock and page buttons): OnLeave for those left (innermost
        // first), OnEnter for those entered (outermost first).
        let new_chain = match (hover, cursor) {
            (Some(h), Some((x, y))) => mouse_chain(lua, h, x, y),
            _ => Vec::new(),
        };
        let old_chain = std::mem::replace(&mut ui(lua).entered, new_chain.clone());
        for &o in old_chain.iter().rev() {
            if !new_chain.contains(&o) {
                lua::listbox::leave(lua, o);
                lua::widget::apply_button_state(&mut ui(lua), o, crate::tree::ButtonState::Normal);
                call_handler(lua, o, "OnLeave", vec![]);
                lua::chat::alpha::hover(lua, o, false);
            }
        }
        for &n in &new_chain {
            if !old_chain.contains(&n) {
                lua::widget::apply_button_state(&mut ui(lua), n, crate::tree::ButtonState::Highlight);
                call_handler(lua, n, "OnEnter", vec![]);
                lua::chat::alpha::hover(lua, n, true);
                lua::sound::over_sound(lua, n);
            }
        }
        update_list_hover(lua, old, hover);
    }
    lua::listbox::hover(lua, old, hover);
    lua::resize::hover(lua, hover);
    if down {
        if let Some(id) = hover {
            lua::chat_input::activate(lua, id);
        }
        mouse.pressed = hover;
        ui(lua).tree.touch();
        // A press on an edit box focuses it; anywhere else drops the keyboard focus.
        match hover {
            Some(h) => lua::edit::clicked(lua, h),
            None => lua::edit::set_focus(lua, None),
        }
        let mut resizing = hover.is_some_and(|h| lua::resize::press(lua, h));
        mouse.child_resize = false;
        if let Some(h) = hover.filter(|&h| !resizing && !lua::widget::button_ignores_fading_input(lua, h)) {
            lua::widget::button_pointer_down(lua, h, 0);
            let consumed = lua::call_handler_consumed(lua, h, "OnMouseDown", vec![arg(lua, "LeftButton")]);
            resizing = lua::resize::child_press(lua, h, consumed);
            mouse.child_resize = resizing;
            lua::worldmap::bubble(lua, h, "OnMouseDown", "LeftButton");
        }
        mouse_event(lua, cursor, hover, "MOUSE_DOWN");
        mouse.slider = hover.filter(|_| !resizing).and_then(|h| slider_of(lua, h));
        mouse.drag = hover
            .filter(|_| !resizing)
            .and_then(|h| lua::drag::source_of(lua, h))
            .zip(cursor)
            .map(|(src, at)| DragPress {
                src,
                at,
                started: false,
            });
        if let (Some(s), Some(c)) = (mouse.slider, cursor) {
            drag_slider(lua, s, c);
        }
    } else if let (Some(s), Some(c)) = (mouse.slider, cursor)
        && mouse.last_cursor != Some(c)
    {
        drag_slider(lua, s, c);
    }
    if right_down {
        if let Some(h) = hover.filter(|&h| !lua::widget::button_ignores_fading_input(lua, h)) {
            lua::widget::button_pointer_down(lua, h, 2);
            call_handler(lua, h, "OnMouseDown", vec![arg(lua, "RightButton")]);
            lua::worldmap::bubble(lua, h, "OnMouseDown", "RightButton");
        }
        mouse_event(lua, cursor, hover, "MOUSE_DOWN");
    }
    if cursor.is_some() {
        mouse.last_cursor = cursor;
    }
    if !down && mouse.pressed.is_some() && cursor.is_some() {
        lua::edit::pointer_move(lua);
    }
    // The pointer left the press by a few units: a drag begins (`OnDragStart`, or the slot picks its content up).
    if let (Some(p), Some(c)) = (mouse.drag.as_mut(), cursor)
        && !p.started
        && (c.0 - p.at.0).hypot(c.1 - p.at.1) >= lua::drag::DRAG_START_DISTANCE
    {
        p.started = true;
        let src = p.src;
        lua::drag::start(lua, src);
    }
    if up {
        if let (Some(p), Some(target)) = (mouse.drag.filter(|p| p.started), hover) {
            lua::chat::drag_receive(lua, p.src, target);
        }
        let child_resize = std::mem::take(&mut mouse.child_resize);
        let sizing = lua::drag::sizing(lua) && !child_resize;
        lua::edit::pointer_up(lua);
        mouse.slider = None;
        let dragged = mouse.drag.take().filter(|p| p.started);
        let pressed = mouse.pressed.take();
        ui(lua).tree.touch();
        if let Some(h) = hover.filter(|&h| !lua::widget::button_ignores_fading_input(lua, h)) {
            //39283c30 checks before OnMouseUp. A callback-created fade must not
            // cancel this admitted release's state continuation.
            call_handler(lua, h, "OnMouseUp", vec![arg(lua, "LeftButton")]);
            lua::widget::button_pointer_up(lua, h, 0);
            lua::worldmap::bubble(lua, h, "OnMouseUp", "LeftButton");
        }
        mouse_event(lua, cursor, hover, "MOUSE_UP");
        lua::drag::stop_moving(lua);
        if let Some(p) = dragged {
            // the release of a drag: the drop, not a click
            lua::drag::finish(lua, p.src, hover);
        } else if !sizing
            && let (Some(p), Some(h)) = (pressed, hover)
            && (h == p || !child_resize && ui(lua).tree.is_descendant(h, p))
        {
            click(lua, mouse, h, "LeftButton");
        }
    }
    if right_up {
        if let Some(h) = hover.filter(|&h| !lua::widget::button_ignores_fading_input(lua, h)) {
            call_handler(lua, h, "OnMouseUp", vec![arg(lua, "RightButton")]);
            lua::widget::button_pointer_up(lua, h, 2);
            lua::worldmap::bubble(lua, h, "OnMouseUp", "RightButton");
        }
        mouse_event(lua, cursor, hover, "MOUSE_UP");
    }
    if right_up && lua::drag::holding(lua) {
        // a right click puts what the pointer holds back
        lua::drag::clear(lua);
    } else if right_up && let Some(h) = hover {
        click(lua, mouse, h, "RightButton");
    }
    if wheel != 0.0 {
        let name = if wheel > 0.0 {
            "OnWheelUp"
        } else {
            "OnWheelDown"
        };
        let capture = {
            let u = ui(lua);
            u.keyboard.capture(&u.tree)
        };
        if capture.is_some() {
            ui(lua).keyboard.consume();
        }
        let mut t = capture.or(hover);
        let mut handled = false;
        while let Some(id) = t {
            if has_handler(lua, id, name) {
                call_handler(lua, id, name, vec![]);
                handled = true;
                break;
            }
            t = ui(lua).tree.get(id).parent;
        }
        // No script took the wheel: message frames (the chat) scroll natively.
        if let (false, Some(c)) = (handled, cursor) {
            if !hover.is_some_and(|id| lua::edit::wheel(lua, id, wheel)) {
                lua::chat::wheel(lua, c, hover, wheel);
            }
        }
    }
}

fn mouse_event(lua: &Lua, cursor: Option<(f32, f32)>, target: Option<Id>, name: &str) {
    if cursor.is_some() {
        // Native 39277da0 emits index 4/5 after the widget handler. Empty UI space belongs
        // to UIParent; a pointer outside the OS window does not produce a UI mouse event.
        lua::fire_ui_event(lua, name, vec![Value::Integer(target.unwrap_or(ROOT) as i64)]);
    }
}

/// The enabled slider a press on `id` drags: the slider itself or the slider of its thumb.
fn slider_of(lua: &Lua, id: Id) -> Option<Id> {
    let u = ui(lua);
    let enabled = |i: Id| u.tree.widget(i).is_some_and(|w| w.enabled);
    if u.tree.widget(id)?.slider.is_some() {
        return enabled(id).then_some(id);
    }
    let p = u.tree.get(id).parent?;
    let s = u.tree.widget(p)?.slider.as_ref()?;
    (s.thumb == Some(id) && enabled(p) && enabled(id)).then_some(p)
}

/// Set a dragged slider's value from the cursor: the thumb centre follows the cursor along the
/// track, snapped to the value step.
fn drag_slider(lua: &Lua, slider: Id, (cx, cy): (f32, f32)) {
    let v = {
        let u = ui(lua);
        let Some(s) = u.tree.widget(slider).and_then(|w| w.slider.clone()) else {
            return;
        };
        let r = u.tree.rect(slider);
        let thumb = s.thumb.map(|t| u.tree.rect(t));
        let (start, len, pos, tlen) = if s.vertical {
            (r.y, r.h, cy, thumb.map_or(0.0, |t| t.h))
        } else {
            (r.x, r.w, cx, thumb.map_or(0.0, |t| t.w))
        };
        let travel = (len - tlen).max(1.0);
        let f = ((pos - start - tlen / 2.0) / travel).clamp(0.0, 1.0);
        let mut v = s.min + f * (s.max - s.min);
        if s.step > 0.0 && s.step < s.max - s.min {
            v = s.min + ((v - s.min) / s.step).round() * s.step;
        }
        v.clamp(s.min, s.max)
    };
    lua::widget::set_slider(lua, slider, v, true);
    // A message frame's native slider scrolls the frame.
    lua::chat::slider_changed(lua, slider);
    lua::flush(lua);
}

fn click(lua: &Lua, mouse: &mut MouseState, target: Id, button: &str) {
    if lua::widget::button_ignores_fading_input(lua, target) { return; }
    let held_gen = ui(lua).drag.held_gen;
    lua::sound::click_sound(lua, target, button);
    let now = ui(lua).time_ms;
    let double = mouse
        .last_click
        .is_some_and(|(id, t)| id == target && now - t < 350.0);
    mouse.last_click = Some((target, now));
    // The check button this click lands on; it toggles after its OnClick ran (the scripts read the old
    // state there: the quest list's `show = not self:GetChecked()`, the radio buttons' OnClick).
    let check = if button == "LeftButton" {
        let u = ui(lua);
        let mut t = Some(target);
        let mut found = None;
        while let Some(id) = t {
            if let Some(w) = u.tree.widget(id).filter(|w| w.button.is_some()) {
                if w.ty == WidgetType::CheckButton && w.enabled {
                    found = Some(id);
                }
                break;
            }
            t = u.tree.get(id).parent;
        }
        found
    } else {
        None
    };
    // Script OnClick on the target or the nearest ancestor that has one (enabled only).
    let mut t = Some(target);
    while let Some(id) = t {
        // A nonbutton child can deliver its own entry before bubbling into a
        // button. Check the actual button owner at its entry, not its ancestors.
        if id != target && lua::widget::button_ignores_fading_input(lua, id) { return; }
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
    if let Some(id) = check {
        let checked = ui(lua)
            .tree
            .widget(id)
            .and_then(|w| w.button.as_ref())
            .is_some_and(|b| b.checked);
        lua::widget::set_checked(lua, id, !checked, true);
        lua::flush(lua);
    }
    // A slot uses what it holds (`AAUI.UseSlot` / `UseSkill` / `UseItem`).
    // A script may already have picked up or placed an item. Do not use the slot after
    // it cleared the hand, or immediately drop a thing the click just picked up.
    let hand_unchanged = ui(lua).drag.held_gen == held_gen;
    if hand_unchanged && lua::slot::clicked(lua, target, button) {
        lua::flush(lua);
    }
    // A row of a grid (quest list) that was clicked becomes its selected row.
    if button == "LeftButton" {
        lua::grid::clicked(lua, target);
        lua::flush(lua);
    }
    // Native tab buttons.
    if button == "LeftButton" && lua::tab::button_clicked(lua, target) {
        lua::flush(lua);
        return;
    }
    if button == "LeftButton" && lua::chat::button_clicked(lua, target) {
        lua::flush(lua);
        return;
    }
    if button == "RightButton" && lua::chat::context_clicked(lua, target) {
        lua::flush(lua);
        return;
    }
    // The world map: left shows the zone under the cursor, right the map above.
    if lua::worldmap::clicked(lua, target, button) {
        lua::flush(lua);
        return;
    }
    // Native list selection.
    if button == "LeftButton" {
        lua::listbox::clicked(lua, target);
        lua::flush(lua);
    }
    if button == "LeftButton"
        && let Some((list, row)) = list_row(lua, target)
    {
        let enabled = ui(lua).tree.widget(list).is_some_and(|w| w.enabled);
        if enabled {
            lua::widget_set_selected(lua, list, row as i32, true, double);
            lua::flush(lua);
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
