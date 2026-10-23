//! Native widget methods (`widget:AddAnchor(...)`, `button:SetNormalBackground(...)`, ...) and
//! text style methods (`widget.style:SetFont(...)`).

use mlua::{Lua, MultiValue, Table, Value};

use super::{Args, Fade, method, new_object_table, none, reg, ret, ui, value_id};
use crate::tree::{
    Align, Anchor, ButtonState, DrawKind, DrawLayer, Id, Kind, Layer, ListKey, Point, ROOT, TextBlock,
    WidgetType,
};

mod number;

/// Parse `AddAnchor(point, [relativeTo], [relativePoint], [x, y])` (after `self`).
pub(crate) fn parse_anchor(lua: &Lua, id: Id, a: &Args) -> Option<Anchor> {
    let point = Point::parse(&a.s(1)?)?;
    let owner = ui(lua).tree.get(id).parent.unwrap_or(ROOT);
    let mut i = 2;
    let mut target = owner;
    match a.v(i) {
        Value::Table(_) | Value::String(_) if !a.is_num(i) => {
            // A string may be a point name only if it is the relative point (no target given).
            let is_point = a.s(i).is_some_and(|s| Point::parse(&s).is_some())
                && matches!(a.v(i), Value::String(_));
            if !is_point {
                target = value_id(lua, &a.v(i))?;
                if !ui(lua).tree.get(target).alive { return None; }
                i += 1;
            }
        }
        Value::Nil if a.len() > i => i += 1,
        _ => {}
    }
    let mut rel = point;
    if let Some(s) = a.s(i).filter(|_| !a.is_num(i))
        && let Some(p) = Point::parse(&s)
    {
        rel = p;
        i += 1;
    }
    let (x, y) = if i + 1 < a.len() {
        if !a.is_num(i) || !a.is_num(i + 1) { return None; }
        (a.f(i), a.f(i + 1))
    } else {
        (0.0, 0.0)
    };
    Some(Anchor {
        point,
        target,
        rel,
        x,
        y,
    })
}

fn set_handler(lua: &Lua, id: Id, name: &str, f: Option<mlua::Function>) -> mlua::Result<()> {
    let r = reg(lua);
    let hs = match r.handlers.raw_get::<Option<Table>>(id)? {
        Some(t) => t,
        None => {
            let t = lua.create_table()?;
            r.handlers.raw_set(id, &t)?;
            t
        }
    };
    hs.raw_set(name, f.clone())?;
    let mut u = ui(lua);
    let list = u.handlers.entry(id).or_default();
    list.retain(|h| h != name);
    if f.is_some() {
        list.push(name.to_string());
    }
    let per_frame = match name {
        "OnUpdate" => Some("OnUpdate"),
        "OnEffect" => Some("OnEffect"),
        _ => None,
    };
    if let Some(h) = per_frame {
        if f.is_some() {
            u.frame_handlers.insert((id, h));
        } else {
            u.frame_handlers.remove(&(id, h));
        }
    }
    Ok(())
}

/// Show / hide with optional fade (ms), queueing `OnShow` / `OnHide`. A top-level window that comes up is put in
/// front of the others of its layer, as the client does (the bag opened over the quest tracker takes the mouse
/// before the tracker's invisible window).
pub(crate) fn show(lua: &Lua, id: Id, visible: bool, fade_ms: Option<f32>) {
    let mut g = ui(lua);
    let u = &mut *g;
    let was = u.tree.get(id).shown;
    // Repeating the visible flag never replaces/cancels a queued native manager record.
    if was == visible {
        return;
    }
    if visible {
        u.retiring.remove(&id);
    } else if was && u.tree.widget(id).is_some_and(|w| w.remove_on_hide) {
        u.retiring.insert(id);
    }
    if !visible && fade_ms.is_none_or(|ms| ms <= 0.0) {
        super::damage::hide(u, id);
    }
    if visible
        && !was
        && id != crate::tree::ROOT
        && u.tree.get(id).parent.is_none()
        && u.tree.get(id).is_widget()
    {
        u.tree.raise(id);
    }
    let clock = u.fade_clock;
    let order = if let Some(old) = u.fades.get(&id) {
        old.order
    } else {
        let order = u.next_fade_order;
        u.next_fade_order += 1;
        order
    };
    let from = if u.fades.contains_key(&id) {
        u.tree.widget(id).map_or(1.0, |w| w.fade_alpha)
    } else if visible { 0.0 } else { 1.0 };
    u.fades.insert(id, Fade {
        order,
        from,
        to: if visible { 1.0 } else { 0.0 },
        start: clock,
        end: clock + fade_ms.unwrap_or(0.0),
    });
    u.tree.get_mut(id).shown = visible;
    if let Some(w) = u.tree.widget_mut(id) {
        w.fading = true;
    }
    let mut sound = false;
    if was != visible && u.tree.get(id).is_widget() {
        let h = if visible { "OnShow" } else { "OnHide" };
        u.pending.push((id, h.to_string(), vec![]));
        // `OnVisibleChanged(self, visible)` (character create step 3 resets its name box there).
        u.pending
            .push((id, "OnVisibleChanged".into(), vec![Value::Boolean(visible)]));
        sound = u.widget_sounds.contains_key(&id);
    }
    drop(g);
    if sound {
        super::sound::widget_sound(lua, id, if visible { "show" } else { "hide" });
    }
}

/// Native hide completion (392ab33e) queues flagged widgets for removal after callbacks.
/// Keep them valid through OnHide and the current click; the next input frame has no stale
/// event listeners or descendants from the removed popup.
pub(super) fn remove_hidden(lua: &Lua) {
    let retiring = std::mem::take(&mut ui(lua).retiring);
    for root in retiring {
        let remove = {
            let u = ui(lua);
            u.tree.get(root).alive && u.tree.widget(root).is_some_and(|w| w.remove_on_hide)
        };
        if !remove {
            continue;
        }
        if ui(lua).tree.get(root).shown || ui(lua).tree.widget(root).is_some_and(|w| w.fading) {
            // A fade has not finished. Show(true) cancels retirement in show().
            ui(lua).retiring.insert(root);
            continue;
        }
        remove_subtree(lua, root);
    }
}

/// Retire a native composite, including its input/event registrations and children.
pub(super) fn remove_subtree(lua: &Lua, root: Id) {
    if !ui(lua).tree.get(root).alive {
        return;
    }
    let ids = {
        let u = ui(lua);
        let mut ids = std::collections::BTreeSet::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            if !ids.insert(id) {
                continue;
            }
            if let Some(w) = u.tree.widget(id) {
                stack.extend(w.children.iter().copied());
                stack.extend(w.drawables.iter().copied());
            }
        }
        ids
    };
    if ui(lua).focus.is_some_and(|id| ids.contains(&id)) {
        super::edit::set_focus(lua, None);
    }
    let mut g = ui(lua);
    let u = &mut *g;
    if let Some(parent) = u.tree.get(root).parent
        && let Some(w) = u.tree.widget_mut(parent)
    {
        w.children.retain(|id| !ids.contains(id));
    }
    let mut anchor_dependents = Vec::new();
    for &id in &ids {
        anchor_dependents.extend(u.tree.detach_anchor_target(id));
        u.tree.get_mut(id).alive = false;
        u.animations.remove(id);
        super::damage::hide(u, id);
        u.handlers.remove(&id);
        u.fades.remove(&id);
        u.widget_sounds.remove(&id);
        u.keyboard.removed(id);
        u.drag.registered.remove(&id);
        u.retiring.remove(&id);
        u.auto_clip_widgets.remove(&id);
    }
    u.drag.removed(&ids);
    for events in u.events.values_mut() {
        events.retain(|id| !ids.contains(id));
    }
    u.frame_handlers.retain(|(id, _)| !ids.contains(id));
    u.attachments
        .retain(|owner, target| !ids.contains(owner) && !ids.contains(target));
    u.pending.retain(|(id, _, _)| !ids.contains(id));
    u.entered.retain(|id| !ids.contains(id));
    if u.hover.is_some_and(|id| ids.contains(&id)) {
        u.hover = None;
    }
    drop(g);
    let r = reg(lua);
    for id in ids {
        let _ = r.handlers.raw_set(id, Value::Nil);
        let _ = r.objs.raw_set(id, Value::Nil);
    }
    for id in anchor_dependents {
        super::anchors::changed(lua, id);
    }
}

pub(crate) fn create_child(
    lua: &Lua,
    parent: Option<Id>,
    ty: &str,
    name: &str,
    index: i64,
    reflect: bool,
) -> mlua::Result<Table> {
    let pname = parent
        .map(|p| ui(lua).tree.get(p).name.clone())
        .unwrap_or_default();
    let full = name.replace("$parent", &pname);
    let id = ui(lua).tree.create_widget(ty, &full, parent);
    let t = new_object_table(lua, id)?;
    match ty.to_ascii_lowercase().as_str() {
        "slot" => super::slot::init(lua, id, &t)?,
        "cooldownconstantbutton" => {
            if let Some(button) = ui(lua).tree.widget_mut(id).and_then(|w| w.button.as_mut()) {
                // Native initializer 39284865..86: invalid skill ID 0, ready flag false.
                button.cooldown_skill = Some(super::slot::ConstantSkillCooldown {
                    skill_type: 0,
                    ready_notified: false,
                });
            }
        }
        "message" => {
            if let Some(w) = ui(lua).tree.widget_mut(id) {
                w.message = Some(Default::default());
            }
        }
        "chatwindow" => super::chat::init_window(lua, id, &t)?,
        _ => {}
    }
    if reflect
        && let Some(p) = parent
        && let Some(pt) = super::object_table(lua, p)?
    {
        if index > 0 {
            let sub = match pt.raw_get::<Value>(name)? {
                Value::Table(s) if reg(lua).ids.raw_get::<Option<Id>>(&s)?.is_none() => s,
                _ => {
                    let s = lua.create_table()?;
                    pt.raw_set(name, &s)?;
                    s
                }
            };
            sub.raw_set(index, &t)?;
        } else {
            pt.raw_set(name, &t)?;
        }
    }
    Ok(t)
}

pub(crate) fn create_widget_global(
    lua: &Lua,
    ty: &str,
    name: &str,
    parent: &Value,
) -> mlua::Result<Table> {
    let parent = value_id(lua, parent).filter(|&p| p != ROOT);
    create_child(lua, parent, ty, name, 0, false)
}

fn create_drawable(
    lua: &Lua,
    owner: Id,
    kind: DrawKind,
    layer: &str,
    texture: Option<String>,
) -> mlua::Result<Table> {
    let class = match kind {
        DrawKind::Image => "drawable:image",
        DrawKind::Icon => "drawable:iconimage",
        DrawKind::NinePart => "drawable:ninepart",
        DrawKind::ThreePart => "drawable:threepart",
        DrawKind::Color => "drawable:color",
        DrawKind::Text => "drawable:text",
    };
    create_drawable_class(lua, owner, kind, class, layer, texture)
}

fn create_drawable_class(
    lua: &Lua,
    owner: Id,
    kind: DrawKind,
    class: &'static str,
    layer: &str,
    texture: Option<String>,
) -> mlua::Result<Table> {
    let id = {
        let mut g = ui(lua);
        let u = &mut *g;
        let id = u
            .tree
            .create_drawable(owner, kind, class, DrawLayer::parse(layer));
        let d = u.tree.drawable_mut(id).unwrap();
        d.texture = texture;
        if class == "drawable:effect" {
            d.effect = Some(false);
        }
        id
    };
    new_object_table(lua, id)
}

fn text_of(lua: &Lua, id: Id, f: impl FnOnce(&mut TextBlock)) {
    let mut u = ui(lua);
    if let Some(t) = u.tree.get_mut(id).text_mut() {
        f(t);
    }
}

/// Text height of a widget's text (all lines), using its current width for wrapping.
pub(crate) fn text_height(lua: &Lua, id: Id) -> f32 {
    let mut g = ui(lua);
    let u = &mut *g;
    let w = u.tree.rect(id).w;
    let Some(t) = u.tree.get(id).text().cloned() else {
        return 0.0;
    };
    let inset = t.inset[0] + t.inset[2];
    let maxw = (t.wrap && w > 0.0).then_some((w - inset).max(1.0));
    u.fonts
        .layout_size(&t.style.font, t.style.size, &t.text, maxw, t.line_space)
        .1
}

/// Line count of a widget's text (wrapped at its width) and its line spacing.
fn text_lines(lua: &Lua, id: Id) -> (usize, f32) {
    let mut g = ui(lua);
    let u = &mut *g;
    let w = u.tree.rect(id).w;
    let Some(t) = u.tree.get(id).text().cloned() else {
        return (0, 0.0);
    };
    let maxw = (t.wrap && w > 0.0).then_some((w - t.inset[0] - t.inset[2]).max(1.0));
    let lo = u.fonts.layout(
        &t.style.font,
        t.style.size,
        &t.text,
        maxw,
        t.line_space,
        false,
    );
    let n = if t.text.is_empty() {
        0
    } else {
        lo.line_widths.len()
    };
    (n, t.line_space)
}

/// `SetAutoResize(true)` widgets: labels and buttons take the text width (+ horizontal
/// insets); textboxes keep their width and take the height of the wrapped text (+ vertical
/// insets), as the step 3 name guide in `reference/character_create/step3_*` shows.
pub(crate) fn auto_size(lua: &Lua, id: Id) {
    let mut g = ui(lua);
    let u = &mut *g;
    let t = match u.tree.get(id).text() {
        Some(t) if t.auto_resize => t.clone(),
        _ => return,
    };
    if u.tree
        .widget(id)
        .is_some_and(|w| w.ty == WidgetType::TextBox)
    {
        drop(g);
        let h = text_height(lua, id) + t.inset[1] + t.inset[3];
        let mut u = ui(lua);
        if u.tree.get(id).height != h {
            u.tree.get_mut(id).height = h;
        }
        return;
    }
    let w = if u.tree.widget(id).is_some_and(|w| w.ty.is_button_like()) {
        (super::auto_clip::full_width(u, id, &t) + t.inset[0] as f64 + t.inset[2] as f64) as f32
    } else {
        u.fonts.layout_size(&t.style.font, t.style.size, &t.text, None, t.line_space).0
            + t.inset[0] + t.inset[2]
    };
    if u.tree.get(id).width != w {
        u.tree.get_mut(id).width = w;
        super::auto_clip::changed(u, id, false);
    }
}

fn longest_line(lua: &Lua, id: Id) -> f32 {
    let mut g = ui(lua);
    let u = &mut *g;
    let w = u.tree.rect(id).w;
    let Some(t) = u.tree.get(id).text().cloned() else {
        return 0.0;
    };
    let maxw = (t.wrap && w > 0.0).then_some(w.max(1.0));
    u.fonts
        .layout(
            &t.style.font,
            t.style.size,
            &t.text,
            maxw,
            t.line_space,
            false,
        )
        .width
}

/// Methods shared by every widget, plus type-specific ones for `ty`.
pub(crate) fn methods(lua: &Lua, ty: &str) -> mlua::Result<Table> {
    let m = lua.create_table()?;
    let def = |name: &str, f: mlua::Function| m.raw_set(name, f);

    // --- bounds -----------------------------------------------------------------------------
    def(
        "AddAnchor",
        method(lua, |lua, id, a| {
            let accepted = match parse_anchor(lua, id, a) {
                Some(an) => {
                    let mut u = ui(lua);
                    let accepted = u.tree.install_anchor(id, an);
                    super::auto_clip::changed(&mut u, id, false);
                    accepted
                }
                None => {
                    bevy::log::warn!(
                        "aa-ui: AddAnchor() failed on {}: bad arguments",
                        ui(lua).tree.get(id).name
                    );
                    false
                }
            };
            super::chat::geometry(lua, id);
            list_geometry(lua, id);
            if accepted { super::anchors::changed(lua, id); }
            none()
        })?,
    )?;
    def(
        "RemoveAllAnchors",
        method(lua, |lua, id, _| {
            ui(lua).tree.remove_anchors(id);
            super::chat::geometry(lua, id);
            list_geometry(lua, id);
            none()
        })?,
    )?;
    def(
        "SetExtent",
        method(lua, |lua, id, a| {
            if !a.is_num(1) || !a.is_num(2) { return none(); }
            let mut u = ui(lua);
            u.tree.prepare_anchor_change(id);
            let o = u.tree.get_mut(id);
            o.width = a.f(1);
            o.height = a.f(2);
            drop(u);
            super::auto_clip::changed(&mut ui(lua), id, false);
            super::chat::geometry(lua, id);
            list_geometry(lua, id);
            scale_event(lua, id);
            super::anchors::size_changed(lua, id, a);
            none()
        })?,
    )?;
    def(
        "SetWidth",
        method(lua, |lua, id, a| {
            if !a.is_num(1) { return none(); }
            ui(lua).tree.prepare_anchor_change(id);
            ui(lua).tree.get_mut(id).width = a.f(1);
            super::auto_clip::changed(&mut ui(lua), id, false);
            super::chat::geometry(lua, id);
            list_geometry(lua, id);
            super::anchors::size_changed(lua, id, a);
            none()
        })?,
    )?;
    def(
        "SetHeight",
        method(lua, |lua, id, a| {
            if !a.is_num(1) { return none(); }
            ui(lua).tree.prepare_anchor_change(id);
            ui(lua).tree.get_mut(id).height = a.f(1);
            super::auto_clip::changed(&mut ui(lua), id, false);
            super::chat::geometry(lua, id);
            list_geometry(lua, id);
            super::anchors::size_changed(lua, id, a);
            none()
        })?,
    )?;
    def(
        "GetWidth",
        method(lua, |lua, id, _| ret(lua, ui(lua).tree.size(id).0))?,
    )?;
    def(
        "GetHeight",
        method(lua, |lua, id, _| ret(lua, ui(lua).tree.size(id).1))?,
    )?;
    def(
        "GetExtent",
        method(lua, |lua, id, _| ret(lua, ui(lua).tree.size(id)))?,
    )?;
    def(
        "GetEffectiveExtent",
        method(lua, |lua, id, _| {
            let u = ui(lua);
            let r = u.animations.widget_rect(&u.tree, id);
            ret(lua, (r.w * u.ui_scale, r.h * u.ui_scale))
        })?,
    )?;
    def(
        "GetOffset",
        method(lua, |lua, id, _| {
            let r = ui(lua).tree.rect(id);
            ret(lua, (r.x, r.y))
        })?,
    )?;
    def(
        "GetEffectiveOffset",
        method(lua, |lua, id, _| {
            let u = ui(lua);
            let r = u.animations.widget_rect(&u.tree, id);
            ret(lua, (r.x * u.ui_scale, r.y * u.ui_scale))
        })?,
    )?;
    def(
        "GetContentExtent",
        method(lua, |lua, id, _| ret(lua, ui(lua).tree.size(id)))?,
    )?;
    def(
        "GetContentOffset",
        method(lua, |lua, id, _| {
            let r = ui(lua).tree.rect(id);
            ret(lua, (r.x, r.y))
        })?,
    )?;
    def(
        "CheckOutOfScreen",
        method(lua, |lua, id, _| {
            let u = ui(lua);
            let r = u.animations.widget_rect(&u.tree, id);
            // Native 39262170 uses effective pixel bounds; global scale cancels
            // when comparing those bounds with our logical screen dimensions.
            ret(
                lua,
                r.x < 0. || r.y < 0. || r.right() > u.tree.screen_w || r.bottom() > u.tree.screen_h,
            )
        })?,
    )?;
    def(
        "CorrectOffsetByScreen",
        method(lua, |lua, id, _| {
            let u = ui(lua);
            let r = u.animations.widget_rect(&u.tree, id);
            // Native 392622e0 prioritizes a negative near edge, then far overflow.
            // It returns physical coordinates without changing the anchors. An
            // oversized widget can therefore return a negative far-edge origin.
            let correct = |p: f32, size: f32, screen: f32| {
                if p < 0. {
                    0.
                } else if p + size > screen {
                    screen - size
                } else {
                    p
                }
            };
            ret(
                lua,
                (
                    correct(r.x, r.w, u.tree.screen_w) * u.ui_scale,
                    correct(r.y, r.h, u.tree.screen_h) * u.ui_scale,
                ),
            )
        })?,
    )?;

    // --- visibility / state ---------------------------------------------------------------
    def(
        "Show",
        method(lua, |lua, id, a| {
            if ui(lua).tree.widget(id).is_none() { return none(); }
            let visible = match a.v(1) {
                Value::Boolean(v) => v,
                Value::Integer(v) => v != 0,
                Value::Number(v) => v != 0.0,
                _ => return none(),
            };
            let duration = a.is_num(2).then(|| a.f(2));
            show(lua, id, visible, duration);
            none()
        })?,
    )?;
    def(
        "SetVisible",
        method(lua, |lua, id, a| {
            show(lua, id, a.b(1, true), None);
            none()
        })?,
    )?;
    // The object's own flag, not its ancestors': the system menu lays its buttons out
    // (`UpdateSystemConfigLayout` counts `buttons[i]:IsVisible()`) before showing the frame.
    def(
        "IsVisible",
        method(lua, |lua, id, _| ret(lua, ui(lua).tree.get(id).shown))?,
    )?;
    def(
        "Enable",
        method(lua, |lua, id, a| {
            let on = a.b(1, true);
            let children = a.b(2, false);
            let mut g = ui(lua);
            let u = &mut *g;
            let mut stack = vec![(id, false)];
            while let Some((w, visited)) = stack.pop() {
                if children && !visited {
                    stack.push((w, true));
                    if let Some(wd) = u.tree.widget(w) {
                        stack.extend(wd.children.iter().rev().map(|&c| (c, false)));
                    }
                    continue;
                }
                if u.tree.widget(w).is_some_and(|wd| wd.enabled != on) {
                    u.tree.widget_mut(w).unwrap().enabled = on;
                    //3927b290 dispatches the changed-enable continuation only after
                    // its children. Checkbutton392842a0 reasserts mark visibility.
                    checked_art(u, w);
                    let state = u.tree.widget(w).and_then(|w| w.button.as_ref()).map(|b| b.state);
                    if !on {
                        apply_button_state(u, w, ButtonState::Disabled);
                    } else if state == Some(ButtonState::Disabled) {
                        let over = u.hover.is_some_and(|h| u.tree.is_descendant(h, w));
                        apply_button_state(u, w, if over { ButtonState::Highlight } else { ButtonState::Normal });
                    }
                }
            }
            none()
        })?,
    )?;
    def(
        "IsEnabled",
        method(lua, |lua, id, _| {
            ret(lua, ui(lua).tree.widget(id).is_some_and(|w| w.enabled))
        })?,
    )?;
    def(
        "SetAlpha",
        method(lua, |lua, id, a| {
            if ui(lua).tree.widget(id).is_none() { return none(); }
            if !a.is_num(1) { return none(); }
            let mut u = ui(lua);
            let mut v = a.f(1);
            if v > 1.0 { v = 1.0; }
            let before = u.animations.widget_sample(id).alpha.unwrap_or(u.tree.get(id).alpha);
            super::animation::widgets::set_alpha(&mut u, id, v);
            if before != v && u.tree.visible(id) {
                u.tree.repaint();
            }
            none()
        })?,
    )?;
    def(
        "GetAlpha",
        method(lua, |lua, id, _| {
            let u = ui(lua);
            ret(
                lua,
                u.animations
                    .widget_sample(id)
                    .alpha
                    .unwrap_or(u.tree.get(id).alpha),
            )
        })?,
    )?;
    def(
        "SetUILayer",
        method(lua, |lua, id, a| {
            let layer = Layer::parse(&a.s(1).unwrap_or_default());
            let mut g = ui(lua);
            let u = &mut *g;
            let mut stack = vec![id];
            while let Some(w) = stack.pop() {
                if let Some(wd) = u.tree.widget_mut(w) {
                    wd.layer = layer;
                    stack.extend(wd.children.iter().copied());
                }
            }
            none()
        })?,
    )?;
    def(
        "Raise",
        method(lua, |lua, id, _| {
            ui(lua).tree.raise(id);
            none()
        })?,
    )?;
    def("Lower", method(lua, |_, _, _| none())?)?;
    def(
        "Clickable",
        method(lua, |lua, id, a| {
            if let Some(w) = ui(lua).tree.widget_mut(id) {
                w.clickable = a.b(1, true);
            }
            none()
        })?,
    )?;
    def(
        "EnablePick",
        method(lua, |lua, id, a| {
            if let Some(w) = ui(lua).tree.widget_mut(id) {
                w.clickable = a.b(1, true);
            }
            none()
        })?,
    )?;
    def(
        "SetWindowModal",
        method(lua, |lua, id, a| {
            if let Some(w) = ui(lua).tree.widget_mut(id) {
                w.modal = a.b(1, true);
            }
            none()
        })?,
    )?;
    def(
        "IsMouseOver",
        method(lua, |lua, id, _| {
            let u = ui(lua);
            let (x, y) = u.cursor;
            ret(lua, u.tree.visible(id) && u.tree.rect(id).contains(x, y))
        })?,
    )?;
    def("GetId", method(lua, |lua, id, _| ret(lua, id as i64))?)?;
    def(
        "GetName",
        method(lua, |lua, id, _| {
            ret(lua, ui(lua).tree.get(id).name.clone())
        })?,
    )?;
    def(
        "GetParent",
        method(lua, |lua, id, _| {
            let p = ui(lua).tree.get(id).parent;
            match p {
                Some(p) => ret(lua, new_object_table(lua, p)?),
                None => ret(lua, lua.globals().raw_get::<Value>("UIParent")?),
            }
        })?,
    )?;
    def(
        "IsDescendantWidget",
        method(lua, |lua, id, a| {
            let other = value_id(lua, &a.v(1));
            ret(
                lua,
                other.is_some_and(|o| ui(lua).tree.is_descendant(o, id)),
            )
        })?,
    )?;

    // --- handlers / events ------------------------------------------------------------------
    def(
        "SetHandler",
        method(lua, |lua, id, a| {
            let name = a.s(1).unwrap_or_default();
            let f = match a.v(2) {
                Value::Function(f) => Some(f),
                _ => None,
            };
            set_handler(lua, id, &name, f)?;
            none()
        })?,
    )?;
    def(
        "ReleaseHandler",
        method(lua, |lua, id, a| {
            set_handler(lua, id, &a.s(1).unwrap_or_default(), None)?;
            none()
        })?,
    )?;
    def(
        "HasHandler",
        method(lua, |lua, id, a| {
            ret(
                lua,
                super::has_handler(lua, id, &a.s(1).unwrap_or_default()),
            )
        })?,
    )?;
    def(
        "RegisterEvent",
        method(lua, |lua, id, a| {
            let name = a.s(1).unwrap_or_default();
            let mut u = ui(lua);
            let list = u.events.entry(name).or_default();
            if !list.contains(&id) {
                list.push(id);
            }
            none()
        })?,
    )?;
    def(
        "UnregisterEvent",
        method(lua, |lua, id, a| {
            let name = a.s(1).unwrap_or_default();
            if let Some(l) = ui(lua).events.get_mut(&name) {
                l.retain(|&x| x != id);
            }
            none()
        })?,
    )?;

    // --- children / drawables -------------------------------------------------------------
    def(
        "CreateChildWidget",
        method(lua, |lua, id, a| {
            let ty = a.s(1).unwrap_or_else(|| "emptywidget".into());
            let name = a.s(2).unwrap_or_default();
            let t = create_child(lua, Some(id), &ty, &name, a.f(3) as i64, a.b(4, false))?;
            ret(lua, t)
        })?,
    )?;
    def(
        "CreateChildWidgetByType",
        method(lua, |lua, id, a| {
            let ty = a.s(1).unwrap_or_else(|| "emptywidget".into());
            let name = a.s(2).unwrap_or_default();
            let t = create_child(lua, Some(id), &ty, &name, a.f(3) as i64, a.b(4, false))?;
            ret(lua, t)
        })?,
    )?;
    def(
        "CreateImageDrawable",
        method(lua, |lua, id, a| {
            ret(
                lua,
                create_drawable(
                    lua,
                    id,
                    DrawKind::Image,
                    &a.s(2).unwrap_or_default(),
                    a.s(1),
                )?,
            )
        })?,
    )?;
    def(
        "CreateIconImageDrawable",
        method(lua, |lua, id, a| {
            ret(
                lua,
                create_drawable_class(
                    lua,
                    id,
                    DrawKind::Image,
                    "drawable:iconimage",
                    &a.s(2).unwrap_or_default(),
                    a.s(1),
                )?,
            )
        })?,
    )?;
    def(
        "CreateEffectDrawable",
        method(lua, |lua, id, a| {
            ret(
                lua,
                create_drawable_class(
                    lua,
                    id,
                    DrawKind::Image,
                    "drawable:effect",
                    &a.s(2).unwrap_or_default(),
                    a.s(1),
                )?,
            )
        })?,
    )?;
    def(
        "CreateIconDrawable",
        method(lua, |lua, id, a| {
            ret(
                lua,
                create_drawable(lua, id, DrawKind::Icon, &a.s(1).unwrap_or_default(), None)?,
            )
        })?,
    )?;
    def(
        "CreateNinePartDrawable",
        method(lua, |lua, id, a| {
            ret(
                lua,
                create_drawable(
                    lua,
                    id,
                    DrawKind::NinePart,
                    &a.s(2).unwrap_or_default(),
                    a.s(1),
                )?,
            )
        })?,
    )?;
    def(
        "CreateThreePartDrawable",
        method(lua, |lua, id, a| {
            ret(
                lua,
                create_drawable(
                    lua,
                    id,
                    DrawKind::ThreePart,
                    &a.s(2).unwrap_or_default(),
                    a.s(1),
                )?,
            )
        })?,
    )?;
    def(
        "CreateColorDrawable",
        method(lua, |lua, id, a| {
            let t = create_drawable(lua, id, DrawKind::Color, &a.s(5).unwrap_or_default(), None)?;
            let did = value_id(lua, &Value::Table(t.clone())).unwrap();
            if let Some(d) = ui(lua).tree.drawable_mut(did) {
                d.color = [
                    a.opt_f(1).unwrap_or(1.0),
                    a.opt_f(2).unwrap_or(1.0),
                    a.opt_f(3).unwrap_or(1.0),
                    a.opt_f(4).unwrap_or(1.0),
                ];
            }
            ret(lua, t)
        })?,
    )?;
    def(
        "CreateTextDrawable",
        method(lua, |lua, id, a| {
            // (fontPath, fontSize, layer)
            let t = create_drawable(lua, id, DrawKind::Text, &a.s(3).unwrap_or_default(), None)?;
            let did = value_id(lua, &Value::Table(t.clone())).unwrap();
            let mut u = ui(lua);
            if let Some(tb) = u.tree.get_mut(did).text_mut() {
                if let Some(f) = a.s(1) {
                    tb.style.font = f;
                }
                if let Some(s) = a.opt_f(2) {
                    tb.style.size = s;
                }
            }
            ret(lua, t)
        })?,
    )?;
    def(
        "CreateDrawable",
        method(lua, |lua, id, a| {
            // (drawableTypeStr, path, nameLayer)
            let kind = match a.s(1).unwrap_or_default().to_ascii_lowercase().as_str() {
                "ninepart" | "ninepartdrawable" => DrawKind::NinePart,
                "threepart" | "threepartdrawable" => DrawKind::ThreePart,
                "color" | "colordrawable" => DrawKind::Color,
                _ => DrawKind::Image,
            };
            ret(
                lua,
                create_drawable(lua, id, kind, &a.s(3).unwrap_or_default(), a.s(2))?,
            )
        })?,
    )?;
    def(
        "RemoveAllDrawables",
        method(lua, |lua, id, _| {
            let mut g = ui(lua);
            let u = &mut *g;
            let ds = u
                .tree
                .widget(id)
                .map(|w| w.drawables.clone())
                .unwrap_or_default();
            for d in ds {
                u.animations.remove(d);
                u.tree.get_mut(d).alive = false;
            }
            if let Some(w) = u.tree.widget_mut(id) {
                w.drawables.clear();
            }
            none()
        })?,
    )?;
    def(
        "EnableDrawables",
        method(lua, |lua, id, a| {
            set_layer_enabled(lua, id, a.v(1), true);
            none()
        })?,
    )?;
    def(
        "DisableDrawables",
        method(lua, |lua, id, a| {
            set_layer_enabled(lua, id, a.v(1), false);
            none()
        })?,
    )?;

    // --- text -------------------------------------------------------------------------------
    def(
        "SetText",
        method(lua, |lua, id, a| {
            let label = ui(lua).tree.widget(id).is_some_and(|w| w.label.is_some());
            if label && !matches!(a.v(1), Value::String(_)) {
                return none();
            }
            super::auto_clip::geometry(&mut ui(lua), id);
            let s = if label {
                let Value::String(raw) = a.v(1) else { unreachable!() };
                let mut u = ui(lua);
                let w = u.tree.widget_mut(id).unwrap();
                number::set(
                    w.label.as_mut().unwrap(),
                    &w.text.as_ref().unwrap().text,
                    &raw.as_bytes(),
                )
            } else {
                Some(a.s(1).unwrap_or_default())
            };
            if let Some(s) = s {
                let mut u = ui(lua);
                // The same text again (the HUD's per-frame updates) changes nothing.
                let same = u
                    .tree
                    .widget(id)
                    .and_then(|w| w.text.as_ref().or(w.title.as_ref()))
                    .is_none_or(|t| t.text == s);
                if !same
                    && let Some(w) = u.tree.widget_mut(id)
                    && let Some(t) = w.text.as_mut().or(w.title.as_mut())
                {
                    t.text = s;
                    t.lines.clear();
                }
            }
            auto_size(lua, id);
            super::auto_clip::refresh(&mut ui(lua), id);
            none()
        })?,
    )?;
    def(
        "SetNumberOnly",
        method(lua, |lua, id, a| {
            let on = match a.v(1) {
                Value::Boolean(v) => v,
                Value::Integer(v) => v != 0,
                Value::Number(v) => v != 0.,
                _ => return none(),
            };
            if let Some(data) = ui(lua).tree.widget_mut(id).and_then(|w| w.label.as_mut()) {
                data.number_only = on;
            }
            none()
        })?,
    )?;
    def(
        "SetAutoResize",
        method(lua, |lua, id, a| {
            let on = a.b(1, true);
            text_of(lua, id, |t| t.auto_resize = on);
            auto_size(lua, id);
            super::auto_clip::refresh(&mut ui(lua), id);
            none()
        })?,
    )?;
    def(
        "SetAutoClipChar",
        method(lua, |lua, id, a| {
            super::auto_clip::set(lua, id, a.b(1, true));
            none()
        })?,
    )?;
    def(
        "GetText",
        method(lua, |lua, id, _| {
            if let Some(raw) = ui(lua).tree.widget(id).and_then(|w| w.label.as_ref())
                .and_then(|data| data.raw.as_ref()).cloned()
            {
                return ret(lua, lua.create_string(raw)?);
            }
            let s = ui(lua)
                .tree
                .get(id)
                .text()
                .map(|t| t.text.clone())
                .unwrap_or_default();
            ret(lua, s)
        })?,
    )?;
    def(
        "SetTitle",
        method(lua, |lua, id, a| {
            let s = a.s(1).unwrap_or_default();
            if let Some(w) = ui(lua).tree.widget_mut(id)
                && let Some(t) = w.title.as_mut()
            {
                t.text = s;
            }
            none()
        })?,
    )?;
    def(
        "SetInset",
        method(lua, |lua, id, a| {
            super::auto_clip::geometry(&mut ui(lua), id);
            let inset = [a.f(1), a.f(2), a.f(3), a.f(4)];
            text_of(lua, id, |t| t.inset = inset);
            none()
        })?,
    )?;
    def(
        "GetInset",
        method(lua, |lua, id, _| {
            let i = ui(lua)
                .tree
                .get(id)
                .text()
                .map(|t| t.inset)
                .unwrap_or_default();
            ret(lua, (i[0], i[1], i[2], i[3]))
        })?,
    )?;
    def(
        "SetLineSpace",
        method(lua, |lua, id, a| {
            let s = a.f(1);
            text_of(lua, id, |t| t.line_space = s);
            none()
        })?,
    )?;
    def(
        "SetAutoWordwrap",
        method(lua, |lua, id, a| {
            let on = a.b(1, true);
            text_of(lua, id, |t| t.wrap = on);
            none()
        })?,
    )?;
    def(
        "GetTextHeight",
        method(lua, |lua, id, _| ret(lua, text_height(lua, id)))?,
    )?;
    // Widgets without a value (scroll windows: `scrollWnd:GetValue("vert")`) report 0; sliders
    // and status bars override it.
    def("GetValue", method(lua, |lua, _, _| ret(lua, 0.0))?)?;
    def(
        "SetCloseOnEscape",
        method(lua, |lua, id, a| {
            let on = a.b(1, true);
            if let Some(w) = ui(lua).tree.widget_mut(id) {
                w.close_on_escape = on;
            }
            none()
        })?,
    )?;
    // Lines of the (wrapped) text; tooltips: their `AddLine` lines too.
    def(
        "GetLineCount",
        method(lua, |lua, id, _| {
            let extra = ui(lua).tree.get(id).text().map_or(0, |t| t.lines.len());
            ret(lua, text_lines(lua, id).0 + extra)
        })?,
    )?;
    def(
        "GetLongestLineWidth",
        method(lua, |lua, id, _| ret(lua, longest_line(lua, id)))?,
    )?;
    def(
        "GetTextWidth",
        method(lua, |lua, id, a| {
            let mut g = ui(lua);
            let u = &mut *g;
            let Some(t) = u.tree.get(id).text().cloned() else {
                return ret(lua, 0.0);
            };
            let s = a.s(1).unwrap_or(t.text);
            ret(lua, u.fonts.measure(&t.style.font, t.style.size, &s))
        })?,
    )?;
    def(
        "ClearLines",
        method(lua, |lua, id, _| {
            text_of(lua, id, |t| {
                t.lines.clear();
                t.text.clear();
            });
            none()
        })?,
    )?;
    def(
        "AddLine",
        method(lua, |lua, id, a| {
            let s = a.s(1).unwrap_or_default();
            let mut index = -1i64;
            text_of(lua, id, |t| {
                t.lines.push(s.clone());
                t.text = t.lines.join("\n");
                index = t.lines.len() as i64 - 1;
            });
            // The 0-based index of the new line (tooltip.lua keeps it: `lastLineIndex + 1`).
            ret(lua, index)
        })?,
    )?;
    def(
        "GetHeightToLastLine",
        // (lines only, including line spacing): tooltip.lua reads `_, includeSpace = ...`.
        method(lua, |lua, id, _| {
            let h = text_height(lua, id);
            let (n, ls) = text_lines(lua, id);
            ret(lua, (h - n.saturating_sub(1) as f32 * ls, h))
        })?,
    )?;

    // --- misc no-ops that scripts call during setup -------------------------------------
    def(
        "SetSounds",
        method(lua, |lua, id, a| {
            super::sound::set_sounds(lua, id, a.s(1).unwrap_or_default());
            none()
        })?,
    )?;
    def(
        "SetLimitWidth",
        method(lua, |lua, id, a| {
            // Native label setter and style:SetEllipsis both call 39274900 (style bit 0x10).
            if let Some(t) = ui(lua).tree.get_mut(id).text_mut() {
                t.style.ellipsis = a.b(1, true);
            }
            none()
        })?,
    )?;
    def(
        "EnableHidingIsRemove",
        method(lua, |lua, id, a| {
            if let Some(w) = ui(lua).tree.widget_mut(id) {
                // Native 392657e0 stores the flag at +0x2b5; hide completion checks it.
                w.remove_on_hide = a.b(1, true);
            }
            none()
        })?,
    )?;
    for name in [
        "RegisterForDrag",
        "StartMoving",
        "StopMovingOrSizing",
        "SetEscEvent",
        "SetSaveConfig",
        "UseDynamicContentState",
        "SetMsgToParent",
        "SetMaxTextLength",
        "UseConsistentTextColor",
    ] {
        def(name, method(lua, |_, _, _| none())?)?;
    }
    def("UseDynamicDrawableState", method(lua, |lua, id, a| {
        let (Some(layer), Value::Boolean(on)) = (a.s(1), a.v(2)) else { return none() };
        let layer = match layer.to_ascii_lowercase().as_str() {
            "background" => 0, "artwork" => 1, "overlay" => 2, "overoverlay" => 3,
            _ => return none(),
        };
        super::chat::alpha::enable(lua, id, layer, on);
        none()
    })?)?;
    def("IsNowAnimation", method(lua, |lua, _, _| ret(lua, false))?)?;
    // Scroll windows: `EnableScroll(true)` clips the children to the widget (and to mouse
    // hits); `ChangeChildAnchorByScrollValue("horz" | "vert", value)` sets how far the children
    // anchored to it are scrolled (absolute, in UI units; the scripts' sliders and `SetExpandMapAnchor`).
    def(
        "EnableScroll",
        method(lua, |lua, id, a| {
            let on = a.b(1, true);
            if ui(lua).tree.widget(id).is_some_and(|w| w.clip != on)
                && let Some(w) = ui(lua).tree.widget_mut(id)
            {
                w.clip = on;
            }
            none()
        })?,
    )?;
    def(
        "ChangeChildAnchorByScrollValue",
        method(lua, |lua, id, a| {
            let axis = usize::from(
                !a.s(1)
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .starts_with('h'),
            );
            let v = a.f(2);
            if ui(lua).tree.widget(id).is_some_and(|w| w.scroll[axis] != v)
                && let Some(w) = ui(lua).tree.widget_mut(id)
            {
                w.scroll[axis] = v;
            }
            none()
        })?,
    )?;
    super::edit::focus_methods(lua, &m)?;
    super::keyboard::methods(lua, &m)?;
    def(
        "SetTitleInset",
        method(lua, |lua, id, a| {
            let inset = [a.f(1), a.f(2), a.f(3), a.f(4)];
            if let Some(t) = ui(lua).tree.widget_mut(id).and_then(|w| w.title.as_mut()) {
                t.inset = inset;
            }
            none()
        })?,
    )?;
    // `titleBar:SetTitleText(text)` (baselib windows) is the title setter under another name.
    let set_title: mlua::Function = m.raw_get("SetTitle")?;
    def("SetTitleText", set_title)?;
    super::drag::methods(lua, &m)?;
    super::resize::methods(lua, &m)?;
    if matches!(ty, "editboxmultiline" | "textbox") {
        let class = ty.to_string();
        def(
            "GetTextLength",
            method(lua, move |lua, id, _| {
                let u = ui(lua);
                let Some(w) = u.tree.widget(id).filter(|w| w.class == class) else {
                    return none();
                };
                //3923b270 reads the multiline scalar count;3925c410 calls
                // XlStringLen on the textbox's native text buffer. Neither calls
                // a replaceable Lua GetText method or materializes digit defaults.
                ret(lua, w.text.as_ref().map_or(0., |t| {
                    t.text.split('\0').next().unwrap_or_default().chars().count() as f32
                }))
            })?,
        )?;
    }
    if matches!(ty, "editbox" | "editboxmultiline") {
        super::edit::methods(lua, &m)?;
    }
    if ty == "statusbar" {
        super::statusbar::methods(lua, &m)?;
    }
    if ty == "slot" {
        super::slot::methods(lua, &m)?;
    }
    if matches!(
        ty,
        "cooldownbutton"
            | "cooldownactionbutton"
            | "cooldownconstantbutton"
            | "cooldowninventorybutton"
    ) {
        super::slot::cooldown_button_methods(lua, &m)?;
    }
    if ty == "cooldownconstantbutton" {
        super::slot::constant_cooldown_methods(lua, &m)?;
    }
    if ty == "message" {
        super::chat::message_methods(lua, &m)?;
    }
    if ty == "chatwindow" {
        super::chat::window_methods(lua, &m)?;
    }
    if matches!(ty, "roadmap" | "worldmap") {
        super::roadmap::methods(lua, &m)?;
    }
    if ty == "worldmap" {
        super::worldmap::methods(lua, &m)?;
    }
    if matches!(ty, "roadmap" | "worldmap") {
        super::radar::methods(lua, &m)?;
    }
    if ty == "listbox" {
        super::listbox::methods(lua, &m)?;
    }
    if ty == "grid" {
        super::grid::methods(lua, &m)?;
    }
    if matches!(ty, "gametooltip" | "unitframetooltip") {
        super::tooltip::methods(lua, &m)?;
    }

    // --- buttons -----------------------------------------------------------------------------
    if is_button_class(ty) {
        // (drawableType, r, g, b, a, path, layer): a drawable shown only in one state.
        for (name, st) in [
            ("CreateNormalDrawable", ButtonState::Normal),
            ("CreateHighlightDrawable", ButtonState::Highlight),
            ("CreatePushedDrawable", ButtonState::Pushed),
            ("CreateDisabledDrawable", ButtonState::Disabled),
        ] {
            def(
                name,
                method(lua, move |lua, id, a| {
                    let ty = a.s(1).unwrap_or_default();
                    let color = [
                        a.opt_f(2).unwrap_or(1.0),
                        a.opt_f(3).unwrap_or(1.0),
                        a.opt_f(4).unwrap_or(1.0),
                        a.opt_f(5).unwrap_or(1.0),
                    ];
                    let (kind, class) = state_drawable_kind(&ty);
                    let layer = a.s(7).unwrap_or_else(|| "artwork".into());
                    let t = create_drawable_class(lua, id, kind, class, &layer, a.s(6))?;
                    if let Some(d) = value_id(lua, &Value::Table(t.clone()))
                        && let Some(dr) = ui(lua).tree.drawable_mut(d)
                    {
                        dr.state_only = Some(st);
                        dr.color = color;
                    }
                    let mut u = ui(lua);
                    reapply_button_state(&mut u, id);
                    ret(lua, t)
                })?,
            )?;
        }
        // (UI_BUTTON_*, UOT_* drawable type, path, layer).
        def(
            "CreateStateDrawable",
            method(lua, |lua, id, a| {
                let st = match a.opt_f(1).unwrap_or(1.0) as i64 {
                    0 => ButtonState::Disabled,
                    2 => ButtonState::Highlight,
                    3 => ButtonState::Pushed,
                    _ => ButtonState::Normal,
                };
                let (kind, class) = match a.opt_f(2).unwrap_or(10.0) as i64 {
                    8 => (DrawKind::NinePart, "drawable:ninepart"),
                    11 => (DrawKind::Image, "drawable:iconimage"),
                    _ => (DrawKind::Image, "drawable:image"),
                };
                let layer = a.s(4).unwrap_or_else(|| "artwork".into());
                let t = create_drawable_class(lua, id, kind, class, &layer, a.s(3))?;
                if let Some(d) = value_id(lua, &Value::Table(t.clone()))
                    && let Some(dr) = ui(lua).tree.drawable_mut(d)
                {
                    dr.state_only = Some(st);
                }
                let mut u = ui(lua);
                reapply_button_state(&mut u, id);
                ret(lua, t)
            })?,
        )?;
        for (name, state) in [
            ("SetNormalBackground", 0usize),
            ("SetHighlightBackground", 1),
            ("SetPushedBackground", 2),
            ("SetDisabledBackground", 3),
        ] {
            def(
                name,
                method(lua, move |lua, id, a| {
                    if !matches!(a.v(1), Value::Table(_)) { return none(); }
                    let Some(d) = value_id(lua, &a.v(1)) else { return none() };
                    let valid = { let u = ui(lua); u.tree.get(d).alive && u.tree.get(d).drawable().is_some() };
                    if valid { set_button_bg(lua, id, state, d); }
                    none()
                })?,
            )?;
        }
        def("IgnoreInputOnFading", method(lua, |lua, id, a| {
            //3922f600/3922f1b0 require a bool-convertible script bool/number.
            if !matches!(a.v(1), Value::Boolean(_) | Value::Integer(_) | Value::Number(_)) {
                return none();
            }
            if let Some(b) = ui(lua).tree.widget_mut(id).and_then(|w| w.button.as_mut()) {
                b.ignore_input_on_fading = a.b(1, false);
            }
            none()
        })?)?;
        def("SetButtonState", method(lua, |lua, id, a| {
            let Value::String(raw) = a.v(1) else { return none() };
            let bytes = raw.as_bytes();
            let name = bytes.split(|&b| b == 0).next().unwrap_or_default();
            let state = if name.eq_ignore_ascii_case(b"DISABLED") { ButtonState::Disabled }
                else if name.eq_ignore_ascii_case(b"HIGHLIGHTED") { ButtonState::Highlight }
                else if name.eq_ignore_ascii_case(b"PUSHED") { ButtonState::Pushed }
                else { ButtonState::Normal };
            apply_button_state(&mut ui(lua), id, state);
            none()
        })?)?;
        def("GetButtonState", method(lua, |lua, id, _| {
            let state = ui(lua).tree.widget(id).and_then(|w| w.button.as_ref()).map(|b| b.state);
            match state {
                Some(ButtonState::Disabled) => ret(lua, "DISABLED"),
                Some(ButtonState::Normal) => ret(lua, "NORMAL"),
                Some(ButtonState::Highlight) => ret(lua, "HIGHLIGHTED"),
                Some(ButtonState::Pushed) => ret(lua, "PUSHED"),
                None => none(),
            }
        })?)?;
        def("RegisterForClicks", method(lua, |lua, id, a| {
            let Value::String(raw) = a.v(1) else { return none() };
            let bytes = raw.as_bytes();
            let name = bytes.split(|&b| b == 0).next().unwrap_or_default();
            let index = match name { b"LeftButton" => 0, b"MiddleButton" => 1, b"RightButton" => 2, _ => return none() };
            //3922e490 parses the optional value only when its script type is bool.
            let enabled = if let Value::Boolean(v) = a.v(2) { v } else { true };
            if let Some(b) = ui(lua).tree.widget_mut(id).and_then(|w| w.button.as_mut()) {
                b.click_buttons[index] = enabled;
            }
            none()
        })?)?;
        for (name, index, state) in [
            ("SetNormalColor", 0usize, ButtonState::Normal),
            ("SetHighlightColor", 1, ButtonState::Highlight),
            ("SetPushedColor", 2, ButtonState::Pushed),
        ] {
            def(
                name,
                method(lua, move |lua, id, a| {
                    if !(1..=4).all(|i| a.is_num(i)) {
                        return none();
                    }
                    let mut u = ui(lua);
                    let Some(w) = u.tree.widget(id) else { return none() };
                    let Some(button) = w.button.as_ref() else { return none() };
                    // Native392806c0 first applies to the existing background, then
                    //39282f30 visits existing extras of this state, even while hidden.
                    // There is no stored palette for artwork created/bound later.
                    let targets: Vec<_> = button.bg[index].into_iter().chain(
                        w.drawables.iter().copied().filter(|&d| {
                            u.tree.get(d).drawable().is_some_and(|d| d.state_only == Some(state))
                        }),
                    ).collect();
                    let color = [a.f(1), a.f(2), a.f(3), a.f(4)];
                    for d in targets {
                        let object = u.tree.get(d);
                        if object.alive && object.drawable().is_some_and(|dr| dr.color != color) {
                            u.tree.drawable_mut(d).unwrap().color = color;
                        }
                    }
                    none()
                })?,
            )?;
        }
        for (name, state) in [
            ("SetTextColor", 0usize),
            ("SetHighlightTextColor", 1),
            ("SetPushedTextColor", 2),
            ("SetDisabledTextColor", 3),
        ] {
            def(
                name,
                method(lua, move |lua, id, a| {
                    if !(1..=4).all(|i| a.is_num(i)) { return none(); }
                    let c = [a.f(1), a.f(2), a.f(3), a.f(4)];
                    if let Some(w) = ui(lua).tree.widget_mut(id)
                        && let Some(b) = w.button.as_mut()
                    {
                        b.text_color[state] = c;
                        let current = match b.state { ButtonState::Normal => 0, ButtonState::Highlight => 1,
                            ButtonState::Pushed => 2, ButtonState::Disabled => 3 };
                        //39280680 updates the current font only for this selected entry.
                        if state == current && let Some(text) = w.text.as_mut() {
                            text.style.color = c;
                        }
                    }
                    none()
                })?,
            )?;
        }
        for (name, state) in [
            ("GetNormalBackground", 0usize),
            ("GetHighlightBackground", 1),
            ("GetPushedBackground", 2),
            ("GetDisabledBackground", 3),
        ] {
            def(
                name,
                method(lua, move |lua, id, _| {
                    let d = ui(lua)
                        .tree
                        .widget(id)
                        .and_then(|w| w.button.as_ref())
                        .and_then(|b| b.bg[state]);
                    match d {
                        Some(d) => ret(lua, new_object_table(lua, d)?),
                        None => none(),
                    }
                })?,
            )?;
        }
        def(
            "SetChecked",
            method(lua, |lua, id, a| {
                // (checked, fireEvent = true): OnCheckChanged when the state changes.
                if matches!(a.v(1), Value::Boolean(_) | Value::Integer(_) | Value::Number(_)) {
                    set_checked(lua, id, a.b(1, false), a.b(2, true));
                }
                none()
            })?,
        )?;
        def(
            "GetChecked",
            method(lua, |lua, id, _| {
                ret(
                    lua,
                    ui(lua)
                        .tree
                        .widget(id)
                        .and_then(|w| w.button.as_ref())
                        .is_some_and(|b| b.checked),
                )
            })?,
        )?;
        for (name, disabled) in [
            ("SetCheckedBackground", false),
            ("SetDisabledCheckedBackground", true),
        ] {
            def(name, method(lua, move |lua, id, a| {
                //39230b90/392307e0 require a drawable table; nil is not unbind.
                if !matches!(a.v(1), Value::Table(_)) { return none(); }
                let Some(d) = value_id(lua, &a.v(1)) else { return none() };
                let mut u = ui(lua);
                if !u.tree.get(d).alive || u.tree.get(d).drawable().is_none()
                    || !u.tree.widget(id).is_some_and(|w| w.ty == WidgetType::CheckButton)
                { return none(); }
                let b = u.tree.widget_mut(id).unwrap().button.as_mut().unwrap();
                if disabled { b.disabled_checked_bg = Some(d); } else { b.checked_bg = Some(d); }
                checked_art(&mut u, id);
                none()
            })?)?;
        }
        def(
            "Click",
            method(lua, |lua, id, _| {
                ui(lua).pending.push((
                    id,
                    "OnClick".into(),
                    vec![Value::String(lua.create_string("LeftButton")?)],
                ));
                none()
            })?,
        )?;
    }

    // --- list control --------------------------------------------------------------------------
    if ty == "listctrl" {
        crate::lua::widget::listctrl_methods(lua, &m)?;
    }
    if ty == "folder" {
        super::folder::methods(lua, &m)?;
    }
    if ty == "tab" {
        super::tab::methods(lua, &m)?;
    }
    if ty == "circlediagram" {
        super::diagram::methods(lua, &m)?;
    }
    if ty == "paintcolorpicker" {
        super::picker::methods(lua, &m)?;
    }

    // --- slider ------------------------------------------------------------------------------
    if ty == "slider" {
        def(
            "SetMinMaxValues",
            method(lua, |lua, id, a| {
                let (lo, hi) = if a.is_num(1) {
                    (a.f(1), a.f(2))
                } else {
                    (a.f(2), a.f(3))
                };
                if let Some(s) = ui(lua).tree.widget_mut(id).and_then(|w| w.slider.as_mut()) {
                    s.min = lo;
                    s.max = hi.max(lo);
                    s.value = s.value.clamp(s.min, s.max);
                }
                none()
            })?,
        )?;
        def(
            "GetMinMaxValues",
            method(lua, |lua, id, _| {
                let s = ui(lua)
                    .tree
                    .widget(id)
                    .and_then(|w| w.slider.clone())
                    .unwrap_or_default();
                ret(lua, (s.min, s.max))
            })?,
        )?;
        def(
            "SetValue",
            method(lua, |lua, id, a| {
                set_slider(lua, id, a.f(1), a.b(2, true));
                none()
            })?,
        )?;
        def(
            "GetValue",
            method(lua, |lua, id, _| {
                ret(
                    lua,
                    ui(lua)
                        .tree
                        .widget(id)
                        .and_then(|w| w.slider.as_ref())
                        .map_or(0.0, |s| s.value),
                )
            })?,
        )?;
        def(
            "SetValueStep",
            method(lua, |lua, id, a| {
                if let Some(s) = ui(lua).tree.widget_mut(id).and_then(|w| w.slider.as_mut()) {
                    s.step = a.f(1);
                }
                none()
            })?,
        )?;
        def(
            "GetValueStep",
            method(lua, |lua, id, _| {
                ret(
                    lua,
                    ui(lua)
                        .tree
                        .widget(id)
                        .and_then(|w| w.slider.as_ref())
                        .map_or(1.0, |s| s.step),
                )
            })?,
        )?;
        def(
            "SetPageStep",
            method(lua, |lua, id, a| {
                if let Some(s) = ui(lua).tree.widget_mut(id).and_then(|w| w.slider.as_mut()) {
                    s.page_step = a.f(1);
                }
                none()
            })?,
        )?;
        def(
            "SetOrientation",
            method(lua, |lua, id, a| {
                let v = a.v(1);
                let vertical = match v {
                    Value::Integer(n) => n != 1,
                    Value::Number(n) => n as i64 != 1,
                    Value::String(s) => !s.to_string_lossy().eq_ignore_ascii_case("HORIZONTAL"),
                    _ => true,
                };
                if let Some(s) = ui(lua).tree.widget_mut(id).and_then(|w| w.slider.as_mut()) {
                    s.vertical = vertical;
                }
                none()
            })?,
        )?;
        def(
            "SetThumbButtonWidget",
            method(lua, |lua, id, a| {
                let t = value_id(lua, &a.v(1));
                if let Some(s) = ui(lua).tree.widget_mut(id).and_then(|w| w.slider.as_mut()) {
                    s.thumb = t;
                }
                none()
            })?,
        )?;
        def(
            "GetThumbButtonWidget",
            method(lua, |lua, id, _| {
                let t = ui(lua)
                    .tree
                    .widget(id)
                    .and_then(|w| w.slider.as_ref())
                    .and_then(|s| s.thumb);
                match t {
                    Some(t) => ret(lua, new_object_table(lua, t)?),
                    None => none(),
                }
            })?,
        )?;
        def(
            "Up",
            method(lua, |lua, id, a| {
                let v = ui(lua)
                    .tree
                    .widget(id)
                    .and_then(|w| w.slider.as_ref())
                    .map_or(0.0, |s| s.value);
                set_slider(lua, id, v - a.opt_f(1).unwrap_or(1.0), true);
                none()
            })?,
        )?;
        def(
            "Down",
            method(lua, |lua, id, a| {
                let v = ui(lua)
                    .tree
                    .widget(id)
                    .and_then(|w| w.slider.as_ref())
                    .map_or(0.0, |s| s.value);
                set_slider(lua, id, v + a.opt_f(1).unwrap_or(1.0), true);
                none()
            })?,
        )?;
        def(
            "SetFixedThumb",
            method(lua, |lua, id, a| {
                if let Some(s) = ui(lua).tree.widget_mut(id).and_then(|w| w.slider.as_mut()) {
                    s.fixed_thumb = a.b(1, true);
                }
                none()
            })?,
        )?;
        def(
            "SetMinThumbLength",
            method(lua, |lua, id, a| {
                if let Some(s) = ui(lua).tree.widget_mut(id).and_then(|w| w.slider.as_mut()) {
                    s.min_thumb = a.f(1).max(0.0);
                }
                none()
            })?,
        )?;
    }
    super::animation::widget_methods(lua, &m)?;
    super::attachment::methods(lua, &m)?;
    if ty == "damagedisplay" {
        super::damage::methods(lua, &m)?;
    }
    super::stubs::finalize(lua, &m, ty)?;
    Ok(m)
}

/// Drawable for a script drawable type name (`ApplyButtonSkin`'s `drawableType`).
fn state_drawable_kind(ty: &str) -> (DrawKind, &'static str) {
    match ty {
        "ninePart" => (DrawKind::NinePart, "drawable:ninepart"),
        "threePart" => (DrawKind::ThreePart, "drawable:threepart"),
        "colorDrawable" => (DrawKind::Color, "drawable:color"),
        "icon" => (DrawKind::Image, "drawable:iconimage"),
        _ => (DrawKind::Image, "drawable:image"),
    }
}

fn is_button_class(class: &str) -> bool {
    matches!(
        class,
        "button"
            | "checkbutton"
            | "slot"
            | "cooldownbutton"
            | "cooldownactionbutton"
            | "cooldownconstantbutton"
            | "cooldowninventorybutton"
    )
}

/// Check / uncheck a button; `OnCheckChanged(self)` runs (after the native returns) when the
/// state changes and `fire` is set.
pub(crate) fn set_checked(lua: &Lua, id: Id, checked: bool, fire: bool) {
    let mut u = ui(lua);
    if !u.tree.widget(id).is_some_and(|w| w.ty == WidgetType::CheckButton) { return; }
    let b = u.tree.widget_mut(id).unwrap().button.as_mut().unwrap();
    let changed = b.checked != checked;
    b.checked = checked;
    checked_art(&mut u, id);
    if changed && fire {
        u.pending.push((id, "OnCheckChanged".into(), vec![]));
    }
}

///39281440/392814c0 write the current marks, including repeated calls. Old
/// bindings retain their last visibility; disabled marks have no normal fallback.
fn checked_art(u: &mut super::Ui, id: Id) {
    let Some(w) = u.tree.widget(id).filter(|w| w.ty == WidgetType::CheckButton) else { return };
    let b = w.button.as_ref().unwrap();
    let writes = [(b.checked_bg, b.checked && w.enabled),
                  (b.disabled_checked_bg, b.checked && !w.enabled)];
    // Keep the native normal-then-disabled order for aliased drawables.
    for (d, shown) in writes {
        if let Some(d) = d
            && u.tree.get(d).alive && u.tree.get(d).shown != shown
        {
            u.tree.get_mut(d).shown = shown;
        }
    }
}

pub(crate) fn set_slider(lua: &Lua, id: Id, v: f32, trigger: bool) {
    let mut g = ui(lua);
    let u = &mut *g;
    let Some(s) = u.tree.widget_mut(id).and_then(|w| w.slider.as_mut()) else {
        return;
    };
    let nv = v.clamp(s.min, s.max);
    let changed = nv != s.value;
    s.value = nv;
    if changed && trigger {
        u.pending
            .push((id, "OnSliderChanged".into(), vec![Value::Number(nv as f64)]));
    }
}

fn set_layer_enabled(lua: &Lua, id: Id, layer: Value, on: bool) {
    // Native392674c0 requires an actual string;3922c120 uses case-sensitive
    // strcmp, with every unknown name selecting all four owner layers.
    let Value::String(layer) = layer else { return };
    let bytes = layer.as_bytes();
    let name = bytes.split(|&b| b == 0).next().unwrap_or_default();
    let layer = match name {
        b"background" => Some(0),
        b"artwork" => Some(1),
        b"overlay" => Some(2),
        b"overoverlay" => Some(3),
        _ => None,
    };
    if let Some(w) = ui(lua).tree.widget_mut(id) {
        if let Some(layer) = layer {
            w.drawable_layers[layer] = on;
        } else {
            w.drawable_layers.fill(on);
        }
    }
}

fn set_button_bg(lua: &Lua, id: Id, state: usize, d: Id) {
    let mut g = ui(lua);
    let u = &mut *g;
    let Some(b) = u.tree.widget(id).and_then(|w| w.button.as_ref()) else { return };
    let old = b.bg[state];
    if let Some(old) = old && u.tree.get(old).alive { u.tree.get_mut(old).shown = false; }
    u.tree.widget_mut(id).unwrap().button.as_mut().unwrap().bg[state] = Some(d);
    let st = [
        ButtonState::Normal,
        ButtonState::Highlight,
        ButtonState::Pushed,
        ButtonState::Disabled,
    ][state];
    if let Some(dr) = u.tree.drawable_mut(d) {
        // Retain membership as metadata; state writes govern actual shown flags.
        dr.state_of = Some((id, st));
    }
    reapply_button_state(u, id);
}

fn reapply_button_state(u: &mut super::Ui, id: Id) {
    if let Some(state) = u.tree.widget(id).and_then(|w| w.button.as_ref()).map(|b| b.state) {
        apply_button_state(u, id, state);
    }
}

///39282f70 clears all current state art, then shows only the selected state.
/// Explicit drawable visibility survives until one of these native state writes.
pub(crate) fn apply_button_state(u: &mut super::Ui, id: Id, desired: ButtonState) {
    if !u.tree.get(id).alive { return; }
    let Some(w) = u.tree.widget(id) else { return };
    let Some(b) = w.button.as_ref() else { return };
    let state = if w.enabled { desired } else { ButtonState::Disabled };
    let old_highlight = (b.state == ButtonState::Highlight).then_some(b.bg[1]).flatten();
    let mask_targets: Vec<_> = b.bg[1].into_iter().chain(w.drawables.iter().copied().filter(|&d| {
        u.tree.get(d).drawable().is_some_and(|d| d.state_only == Some(ButtonState::Highlight))
    })).collect();
    let mut writes = Vec::new();
    for (st, i) in [(ButtonState::Disabled, 3), (ButtonState::Normal, 0),
                    (ButtonState::Highlight, 1), (ButtonState::Pushed, 2)] {
        if let Some(d) = b.bg[i] { writes.push((d, false)); }
        writes.extend(w.drawables.iter().copied().filter(|&d| {
            u.tree.get(d).drawable().is_some_and(|d| d.state_only == Some(st))
        }).map(|d| (d, false)));
    }
    let index = match state { ButtonState::Normal => 0, ButtonState::Highlight => 1,
                             ButtonState::Pushed => 2, ButtonState::Disabled => 3 };
    let text_color = b.text_color[index];
    if let Some(d) = b.bg[index] { writes.push((d, true)); }
    writes.extend(w.drawables.iter().copied().filter(|&d| {
        u.tree.get(d).drawable().is_some_and(|d| d.state_only == Some(state))
    }).map(|d| (d, true)));
    let changed = b.state != state;
    //39282f98 first disables the former highlight's mask, then the hide pass
    // enables masks of current highlight art even while that art remains hidden.
    if let Some(d) = old_highlight { highlight_mask(u, d, false); }
    if changed { u.tree.widget_mut(id).unwrap().button.as_mut().unwrap().state = state; }
    for (d, shown) in writes {
        if u.tree.get(d).alive && u.tree.get(d).shown != shown { u.tree.get_mut(d).shown = shown; }
    }
    for d in mask_targets { highlight_mask(u, d, true); }
    //39282f70 reasserts even the same state's palette after artwork writes.
    // A direct style:SetColor override survives drawing, but not this state write.
    if let Some(text) = u.tree.widget_mut(id).and_then(|w| w.text.as_mut()) {
        text.style.color = text_color;
    }
}

fn highlight_mask(u: &mut super::Ui, id: Id, visible: bool) {
    if u.tree.get(id).alive && u.tree.get(id).drawable().is_some_and(|d| {
        matches!(d.class, "drawable:image" | "drawable:iconimage") && d.mask.is_some() && d.mask_visible != visible
    }) {
        u.tree.drawable_mut(id).unwrap().mask_visible = visible;
    }
}

/// The native ButtonWidget input entry gate, not a hit/focus/hover admission rule.
/// Read once per entry: callbacks may start a fade without cancelling that entry's continuation.
pub(crate) fn button_ignores_fading_input(lua: &Lua, id: Id) -> bool {
    ui(lua).tree.widget(id).is_some_and(|w| {
        w.fading && w.button.as_ref().is_some_and(|b| b.ignore_input_on_fading)
    })
}

pub(crate) fn button_pointer_down(lua: &Lua, id: Id, button: usize) {
    let mut u = ui(lua);
    let admitted = u.tree.widget(id).and_then(|w| w.button.as_ref()).is_some_and(|b| {
        b.state != ButtonState::Disabled && b.click_buttons[button]
    });
    if admitted { apply_button_state(&mut u, id, ButtonState::Pushed); }
}

pub(crate) fn button_pointer_up(lua: &Lua, id: Id, button: usize) {
    let mut u = ui(lua);
    let admitted = u.tree.widget(id).and_then(|w| w.button.as_ref()).is_some_and(|b| {
        b.state == ButtonState::Pushed && b.click_buttons[button]
    });
    if admitted { apply_button_state(&mut u, id, ButtonState::Highlight); }
}

/// `OnScale` after a size change of the root-anchored windows is what the client sends on
/// resolution / UI scale changes; we only send it from `set_screen`. Kept as a hook.
fn scale_event(_lua: &Lua, _id: Id) {}

/// Text style methods (`widget.style`, `window.titleStyle`, text drawables).
pub(crate) fn style_methods(lua: &Lua) -> mlua::Result<Table> {
    let m = lua.create_table()?;
    fn with_style(
        lua: &Lua,
        this: &Value,
        refresh_clip: bool,
        f: impl FnOnce(&mut TextBlock),
    ) -> mlua::Result<()> {
        let Value::Table(t) = this else { return Ok(()) };
        let Some(code) = reg(lua).styles.raw_get::<Option<i64>>(t)? else {
            return Ok(());
        };
        let (owner, which) = ((code / 4) as Id, code % 4);
        let mut u = ui(lua);
        let o = u.tree.get_mut(owner);
        let tb = match (&mut o.kind, which) {
            (Kind::Widget(w), 1) => w.title.as_mut(),
            (Kind::Widget(w), 2) => w.aux_style.as_mut(),
            (Kind::Widget(w), _) => w.text.as_mut(),
            (Kind::Drawable(d), _) => d.text.as_mut(),
        };
        if let Some(tb) = tb {
            f(tb);
        }
        drop(u);
        if which == 0 {
            auto_size(lua, owner);
            if refresh_clip {
                super::auto_clip::refresh(&mut ui(lua), owner);
            }
        }
        Ok(())
    }
    let sm = |name: &str, f: fn(&mut TextBlock, &Args)| -> mlua::Result<()> {
        let refresh_clip = matches!(name, "SetFont" | "SetFontSize");
        m.raw_set(
            name,
            lua.create_function(move |lua, mv: MultiValue| {
                let a = Args::new(mv);
                with_style(lua, &a.v(0), refresh_clip, |tb| f(tb, &a))?;
                Ok(())
            })?,
        )
    };
    sm("SetFont", |t, a| {
        if let Some(f) = a.s(1) {
            t.style.font = f;
        }
        if let Some(s) = a.opt_f(2) {
            t.style.size = s;
        }
    })?;
    sm("SetFontSize", |t, a| t.style.size = a.f(1))?;
    sm("SetAlign", |t, a| {
        t.style.align = Align::from_i(a.f(1) as i64)
    })?;
    sm("SetShadow", |t, a| t.style.shadow = a.b(1, true))?;
    sm("SetOutline", |t, a| t.style.outline = a.b(1, true))?;
    sm("SetSnap", |t, a| t.style.snap = a.b(1, true))?;
    sm("SetEllipsis", |t, a| t.style.ellipsis = a.b(1, true))?;
    sm("SetColor", |t, a| {
        if (1..=4).all(|i| a.is_num(i)) {
            t.style.color = [a.f(1), a.f(2), a.f(3), a.f(4)];
        }
    })?;
    sm("SetLineSpace", |t, a| t.line_space = a.f(1))?;
    for name in [
        "SetItemLinkColor",
        "SetMiddleLineColor",
        "SetMiddleLineHeight",
    ] {
        sm(name, |_, _| {})?;
    }
    m.raw_set(
        "GetTextWidth",
        lua.create_function(|lua, mv: MultiValue| {
            let a = Args::new(mv);
            let mut style = None;
            with_style(lua, &a.v(0), false, |tb| {
                style = Some((tb.style.clone(), tb.text.clone()))
            })?;
            let Some((st, text)) = style else {
                return Ok(0.0);
            };
            let s = a.s(1).unwrap_or(text);
            let owner = if let Value::Table(t) = a.v(0) {
                reg(lua).styles.raw_get::<Option<i64>>(t)?.filter(|c| c % 4 == 0).map(|c| (c / 4) as Id)
            } else { None };
            let mut u = ui(lua);
            if let Some(owner) = owner.filter(|&id| u.tree.widget(id).is_some_and(|w| w.ty.is_button_like())) {
                let full = TextBlock { text: s, style: st, ..Default::default() };
                Ok(super::auto_clip::full_width(&mut u, owner, &full) as f32)
            } else {
                Ok(u.fonts.measure(&st.font, st.size, &s))
            }
        })?,
    )?;
    m.raw_set(
        "GetLineHeight",
        lua.create_function(|lua, mv: MultiValue| {
            let a = Args::new(mv);
            let mut style = None;
            with_style(lua, &a.v(0), false, |tb| style = Some(tb.style.clone()))?;
            let Some(st) = style else { return Ok(0.0) };
            Ok(ui(lua).fonts.metrics(&st.font, st.size).1)
        })?,
    )?;
    m.raw_set(
        "GetColor",
        lua.create_function(|lua, mv: MultiValue| {
            let a = Args::new(mv);
            let mut c = [1.0f32; 4];
            with_style(lua, &a.v(0), false, |tb| c = tb.style.color)?;
            Ok((c[0], c[1], c[2], c[3]))
        })?,
    )?;
    Ok(m)
}

/// Native list control (`listctrl`): columns are buttons in a row, rows are empty widgets with
/// one sub-item widget per column. Scripts reach them as `listCtrl.column[i]`,
/// `listCtrl.items[i]` and `listCtrl.items[i].subItems[j]`.
pub(crate) fn listctrl_methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    m.raw_set(
        "InsertColumn",
        method(lua, |lua, id, a| {
            // (width, itemType)
            let width = a.f(1);
            let col = create_child(lua, Some(id), "button", "column", 0, false)?;
            let cid = value_id(lua, &Value::Table(col.clone())).unwrap();
            {
                let mut g = ui(lua);
                let u = &mut *g;
                let l = u.tree.widget_mut(id).unwrap().list.as_mut().unwrap();
                l.columns.push(cid);
                l.column_widths.push(width);
                l.column_types.push(a.opt_f(2).map_or(2, |t| t as i64));
                let ch = l.column_height;
                let n = l.columns.len();
                let prev = if n >= 2 { Some(l.columns[n - 2]) } else { None };
                let o = u.tree.get_mut(cid);
                o.width = width;
                o.height = ch;
                o.anchors.clear();
                o.anchors.push(match prev {
                    Some(p) => Anchor {
                        point: Point::TopLeft,
                        target: p,
                        rel: Point::TopRight,
                        x: 0.0,
                        y: 0.0,
                    },
                    None => Anchor {
                        point: Point::TopLeft,
                        target: id,
                        rel: Point::TopLeft,
                        x: 0.0,
                        y: 0.0,
                    },
                });
            }
            let this = super::object_table(lua, id)?.unwrap();
            let cols: Table = this.raw_get("column")?;
            cols.raw_set(cols.raw_len() + 1, col)?;
            relayout_rows(lua, id);
            none()
        })?,
    )?;
    m.raw_set(
        "SetColumnHeight",
        method(lua, |lua, id, a| {
            let h = a.f(1);
            let mut g = ui(lua);
            let u = &mut *g;
            let cols = {
                let l = u.tree.widget_mut(id).unwrap().list.as_mut().unwrap();
                l.column_height = h;
                l.columns.clone()
            };
            for c in cols {
                u.tree.get_mut(c).height = h;
            }
            drop(g);
            relayout_rows(lua, id);
            none()
        })?,
    )?;
    m.raw_set(
        "SetItemLineGap",
        method(lua, |lua, id, a| {
            if let Some(l) = ui(lua).tree.widget_mut(id).and_then(|w| w.list.as_mut()) {
                l.item_line_gap = a.f(1);
            }
            relayout_rows(lua, id);
            none()
        })?,
    )?;
    m.raw_set(
        "SetItemHeight",
        method(lua, |lua, id, a| {
            if let Some(l) = ui(lua).tree.widget_mut(id).and_then(|w| w.list.as_mut()) {
                l.explicit_row_height = Some(a.f(1));
            }
            relayout_rows(lua, id);
            none()
        })?,
    )?;
    m.raw_set(
        "InsertRows",
        method(lua, |lua, id, a| {
            let count = a.f(1).max(0.0) as usize;
            let this = super::object_table(lua, id)?.unwrap();
            let items: Table = this.raw_get("items")?;
            for _ in 0..count {
                let row = create_child(lua, Some(id), "window", "item", 0, false)?;
                let rid = value_id(lua, &Value::Table(row.clone())).unwrap();
                let types = ui(lua)
                    .tree
                    .widget(id)
                    .unwrap()
                    .list
                    .as_ref()
                    .unwrap()
                    .column_types
                    .clone();
                let ncols = types.len();
                let subs = lua.create_table()?;
                let mut sub_ids = Vec::new();
                for j in 0..ncols {
                    // The sub item class follows the column's `LCCIT_*` type: a text item
                    // (`SetInset`, `style`), a button, or a window (the reference's world list).
                    let class = match types.get(j).copied().unwrap_or(2) {
                        0 => "listctrlitem",
                        1 => "button",
                        _ => "window",
                    };
                    let sub = create_child(lua, Some(rid), class, "subItem", 0, false)?;
                    let sid = value_id(lua, &Value::Table(sub.clone())).unwrap();
                    // Sub-items show once their row gets data.
                    ui(lua).tree.get_mut(sid).shown = false;
                    sub_ids.push(sid);
                    subs.raw_set(j + 1, sub)?;
                }
                row.raw_set("subItems", subs)?;
                {
                    let mut u = ui(lua);
                    let l = u.tree.widget_mut(id).unwrap().list.as_mut().unwrap();
                    l.rows.push(rid);
                    l.sub_items.push(sub_ids);
                    l.has_data.push(false);
                    l.keys.push(None);
                }
                items.raw_set(items.raw_len() + 1, row)?;
            }
            relayout_rows(lua, id);
            none()
        })?,
    )?;
    // The client's name is `DeleteRows`; `DeleteAllRows` stays for the toolkit's own callers.
    for name in ["DeleteAllRows", "DeleteRows"] {
        m.raw_set(
        name,
        method(lua, |lua, id, _| {
            let mut g = ui(lua);
            let u = &mut *g;
            let rows = u
                .tree
                .widget(id)
                .unwrap()
                .list
                .as_ref()
                .unwrap()
                .rows
                .clone();
            for r in rows {
                u.tree.get_mut(r).alive = false;
            }
            let l = u.tree.widget_mut(id).unwrap().list.as_mut().unwrap();
            l.rows.clear();
            l.sub_items.clear();
            l.has_data.clear();
            l.keys.clear();
            l.selected = -1;
            l.layout_extent = None;
            let img = l.selected_image;
            if let Some(img) = img {
                place_on_row(u, img, None);
            }
            drop(g);
            let this = super::object_table(lua, id)?.unwrap();
            this.raw_set("items", lua.create_table()?)?;
            none()
        })?,
    )?;
    }
    m.raw_set(
        "InsertData",
        method(lua, |lua, id, a| {
            // (key, col, text), col 1-based. The key is an id the script chose (a portal id, a mail
            // id), not a row number: a new key takes the lowest free row, a known key updates its
            // row. The row now holds data (its sub-items show) and the text goes into the sub-item
            // (LCCIT_STRING columns). Scripts that pass 1..n in order land on row n as before.
            let Some(key) = list_key(&a.v(1)) else {
                return none();
            };
            let col = a.f(2) as usize;
            let text = a.s(3).unwrap_or_default();
            let Some(row) = key_row(lua, id, &key, true) else {
                return none();
            };
            set_row_data(lua, id, row, true);
            let mut u = ui(lua);
            let sub = u
                .tree
                .widget(id)
                .and_then(|w| w.list.as_ref())
                .and_then(|l| l.sub_items.get(row)?.get(col.wrapping_sub(1)).copied());
            if let Some(s) = sub
                && let Some(t) = u.tree.get_mut(s).text_mut()
            {
                t.text = text;
            }
            none()
        })?,
    )?;
    m.raw_set(
        "DeleteData",
        method(lua, |lua, id, a| {
            // By key; the row becomes free again.
            if let Some(key) = list_key(&a.v(1))
                && let Some(row) = key_row(lua, id, &key, false)
            {
                set_row_data(lua, id, row, false);
            }
            none()
        })?,
    )?;
    m.raw_set(
        "DeleteDataByIndex",
        method(lua, |lua, id, a| {
            set_row_data(lua, id, (a.f(1) as usize).wrapping_sub(1), false);
            none()
        })?,
    )?;
    m.raw_set(
        "DeleteAllDatas",
        method(lua, |lua, id, _| {
            let n = ui(lua)
                .tree
                .widget(id)
                .and_then(|w| w.list.as_ref())
                .map(|l| l.rows.len())
                .unwrap_or(0);
            for r in 0..n {
                set_row_data(lua, id, r, false);
            }
            none()
        })?,
    )?;
    m.raw_set("SetUseDoubleClick", method(lua, |_, _, _| none())?)?;
    m.raw_set(
        "Select",
        method(lua, |lua, id, a| {
            // 0-based index, -1 clears.
            let idx = a.f(1) as i32;
            set_selected(lua, id, idx, a.b(2, true), false);
            none()
        })?,
    )?;
    m.raw_set(
        "ClearSelection",
        method(lua, |lua, id, _| {
            // Drops the selection; the scripts' handlers start with `if selIdx == 0 then return`,
            // so the client reports the change as `OnSelChanged(0)`.
            let had = ui(lua)
                .tree
                .widget(id)
                .and_then(|w| w.list.as_ref())
                .is_some_and(|l| l.selected >= 0);
            if had {
                set_selected(lua, id, -1, true, false);
            }
            none()
        })?,
    )?;
    m.raw_set(
        "GetSelectedIdx",
        method(lua, |lua, id, _| {
            let s = ui(lua)
                .tree
                .widget(id)
                .and_then(|w| w.list.as_ref())
                .map_or(-1, |l| l.selected);
            ret(lua, if s < 0 { 0 } else { s + 1 })
        })?,
    )?;
    m.raw_set(
        "CreateOveredImage",
        method(lua, |lua, id, a| {
            let t = list_image(lua, id, a, false)?;
            ret(lua, t)
        })?,
    )?;
    m.raw_set(
        "CreateSelectedImage",
        method(lua, |lua, id, a| {
            let t = list_image(lua, id, a, true)?;
            ret(lua, t)
        })?,
    )?;
    m.raw_set("EnableScroll", method(lua, |_, _, _| none())?)?;
    Ok(())
}

fn list_image(lua: &Lua, id: Id, a: &Args, selected: bool) -> mlua::Result<Table> {
    // A nine-part drawable the scripts texture (`SetTexture`, `SetCoords`, `SetInset`, `SetColor`).
    let layer = a.s(1).unwrap_or_else(|| "artwork".into());
    let t = create_drawable(lua, id, DrawKind::NinePart, &layer, None)?;
    let did = value_id(lua, &Value::Table(t.clone())).unwrap();
    let mut g = ui(lua);
    let u = &mut *g;
    u.tree.get_mut(did).shown = false;
    if let Some(l) = u.tree.widget_mut(id).and_then(|w| w.list.as_mut()) {
        if selected {
            l.selected_image = Some(did);
        } else {
            l.overed_image = Some(did);
        }
    }
    Ok(t)
}

/// A script value as a row key (numbers and strings; anything else is not a key).
fn list_key(v: &Value) -> Option<ListKey> {
    match v {
        Value::Integer(n) => Some(ListKey::Num(*n as f64)),
        Value::Number(n) => Some(ListKey::Num(*n)),
        Value::String(s) => Some(ListKey::Str(s.to_string_lossy())),
        _ => None,
    }
}

/// The row (0-based) that owns `key`. With `claim`, an unknown key takes the lowest free row
/// (`None` when every row is taken).
fn key_row(lua: &Lua, id: Id, key: &ListKey, claim: bool) -> Option<usize> {
    let mut g = ui(lua);
    let l = g.tree.widget_mut(id)?.list.as_mut()?;
    if let Some(i) = l.keys.iter().position(|k| k.as_ref() == Some(key)) {
        return Some(i);
    }
    if !claim {
        return None;
    }
    let i = l.keys.iter().position(Option::is_none)?;
    l.keys[i] = Some(key.clone());
    Some(i)
}

/// Mark row `row` (0-based) as holding data or not; shows / hides its sub-items.
fn set_row_data(lua: &Lua, id: Id, row: usize, on: bool) {
    let mut g = ui(lua);
    let u = &mut *g;
    let Some(l) = u.tree.widget_mut(id).and_then(|w| w.list.as_mut()) else {
        return;
    };
    let Some(flag) = l.has_data.get_mut(row) else {
        return;
    };
    *flag = on;
    if !on && let Some(k) = l.keys.get_mut(row) {
        *k = None;
    }
    let subs = l.sub_items[row].clone();
    for s in subs {
        u.tree.get_mut(s).shown = on;
    }
}

/// Select row `idx` (0-based, -1 none); `fire` queues `OnSelChanged(self, index, doubleClick)`.
pub(crate) fn set_selected(lua: &Lua, id: Id, idx: i32, fire: bool, double: bool) {
    let mut g = ui(lua);
    let u = &mut *g;
    let Some(l) = u.tree.widget_mut(id).and_then(|w| w.list.as_mut()) else {
        return;
    };
    l.selected = if idx >= 0 && (idx as usize) < l.rows.len() {
        idx
    } else {
        -1
    };
    let sel = l.selected;
    let rows = l.rows.clone();
    let img = l.selected_image;
    if let Some(img) = img {
        place_on_row(
            u,
            img,
            rows.get(sel.max(0) as usize).copied().filter(|_| sel >= 0),
        );
    }
    if fire {
        u.pending.push((
            id,
            "OnSelChanged".into(),
            vec![Value::Integer((sel + 1) as i64), Value::Boolean(double)],
        ));
    }
}

/// Anchor a list highlight drawable over a row (hidden when `row` is `None`).
pub(crate) fn place_on_row(u: &mut super::Ui, img: Id, row: Option<Id>) {
    let o = u.tree.get_mut(img);
    match row {
        Some(r) => {
            o.shown = true;
            o.anchors = vec![
                Anchor {
                    point: Point::TopLeft,
                    target: r,
                    rel: Point::TopLeft,
                    x: 0.0,
                    y: 0.0,
                },
                Anchor {
                    point: Point::BottomRight,
                    target: r,
                    rel: Point::BottomRight,
                    x: 0.0,
                    y: 0.0,
                },
            ];
        }
        None => o.shown = false,
    }
}

/// Settle anchored/parent-resized controls without rebuilding unchanged row geometry.
pub(crate) fn update_lists(lua: &Lua) {
    let ids = ui(lua).tree.of_class("listctrl");
    for id in ids {
        list_geometry(lua, id);
    }
}

fn list_geometry(lua: &Lua, id: Id) {
    let changed = {
        let u = ui(lua);
        let Some(l) = u.tree.widget(id).and_then(|w| w.list.as_ref()) else { return };
        let (w, h) = u.tree.size(id);
        l.layout_extent != Some([w.to_bits(), h.to_bits()])
    };
    if changed {
        relayout_rows(lua, id);
    }
}

/// Native392a4410: subtract header and intervening gaps as f32, CVTTSS2SI,
/// signed integer division, then assign that one height to every row/sub-item.
pub(crate) fn relayout_rows(lua: &Lua, id: Id) {
    let mut g = ui(lua);
    let u = &mut *g;
    let Some(l) = u.tree.widget(id).and_then(|w| w.list.clone()) else {
        return;
    };
    let (width, height) = u.tree.size(id);
    u.tree.widget_mut(id).unwrap().list.as_mut().unwrap().layout_extent =
        Some([width.to_bits(), height.to_bits()]);
    if l.columns.is_empty() || l.rows.is_empty() {
        return;
    }
    let occupied = (l.rows.len() - 1) as f32 * l.item_line_gap + l.column_height;
    let available = height - occupied;
    let integral = if available.is_finite() && (-2147483648.0..2147483648.0).contains(&available) {
        available as i32
    } else {
        i32::MIN
    };
    let row_height = l.explicit_row_height.unwrap_or_else(|| (integral / l.rows.len() as i32) as f32);
    u.tree.widget_mut(id).unwrap().list.as_mut().unwrap().row_height = row_height;
    for (i, &r) in l.rows.iter().enumerate() {
        let o = u.tree.get_mut(r);
        o.width = width;
        o.height = row_height;
        o.anchors = vec![Anchor {
            point: Point::TopLeft,
            target: if i == 0 { l.columns[0] } else { l.rows[i - 1] },
            rel: Point::BottomLeft,
            x: 0.0,
            y: if i == 0 { 0.0 } else { l.item_line_gap },
        }];
        for (j, &s) in l
            .sub_items
            .get(i)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
            .iter()
            .enumerate()
        {
            let Some(&column_width) = l.column_widths.get(j) else {
                continue;
            };
            let o = u.tree.get_mut(s);
            o.width = column_width;
            o.height = row_height;
            o.anchors = vec![Anchor {
                point: Point::TopLeft,
                target: if j == 0 { r } else { l.sub_items[i][j - 1] },
                rel: if j == 0 { Point::TopLeft } else { Point::TopRight },
                x: 0.0,
                y: 0.0,
            }];
        }
    }
}
