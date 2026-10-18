//! Native widget methods (`widget:AddAnchor(...)`, `button:SetNormalBackground(...)`, ...) and
//! text style methods (`widget.style:SetFont(...)`).

use mlua::{Lua, MultiValue, Table, Value};

use super::{Args, Fade, method, new_object_table, none, reg, ret, ui, value_id};
use crate::tree::{
    Align, Anchor, ButtonState, DrawKind, DrawLayer, Id, Kind, Layer, Point, ROOT, TextBlock,
    WidgetType,
};

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
                target = value_id(lua, &a.v(i)).unwrap_or(owner);
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
    let x = a.f(i);
    let y = a.f(i + 1);
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
    if visible {
        u.retiring.remove(&id);
    } else if was && u.tree.widget(id).is_some_and(|w| w.remove_on_hide) {
        u.retiring.insert(id);
    }
    if !visible && fade_ms.is_none_or(|ms| ms <= 0.0) {
        super::damage::hide(u, id);
    }
    if (was != visible || fade_ms.is_some_and(|ms| ms > 0.0))
        && let Some(alpha) = u.animations.widget_sample(id).alpha
    {
        u.tree.get_mut(id).alpha = alpha;
        u.animations.clear_widget_alpha(id);
    }
    if visible && !was && id != crate::tree::ROOT && u.tree.get(id).parent.is_none() && u.tree.get(id).is_widget() {
        u.tree.raise(id);
    }
    match fade_ms.filter(|&ms| ms > 0.0) {
        Some(ms) if visible => {
            let o = u.tree.get_mut(id);
            o.shown = true;
            let from = if was { o.alpha } else { 0.0 };
            o.alpha = from;
            u.fades.insert(
                id,
                Fade {
                    from,
                    to: 1.0,
                    t: 0.0,
                    duration: ms,
                },
            );
        }
        Some(ms) if was => {
            let from = u.tree.get(id).alpha;
            u.fades.insert(
                id,
                Fade {
                    from,
                    to: 0.0,
                    t: 0.0,
                    duration: ms,
                },
            );
        }
        _ => {
            u.fades.remove(&id);
            let o = u.tree.get(id);
            if (o.shown, o.alpha) != (visible, 1.0) {
                let o = u.tree.get_mut(id);
                o.shown = visible;
                o.alpha = 1.0;
            }
        }
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
        if ui(lua).tree.get(root).shown {
            // A fade has not finished. Show(true) cancels retirement in show().
            ui(lua).retiring.insert(root);
            continue;
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
        for &id in &ids {
            u.tree.get_mut(id).alive = false;
            u.animations.remove(id);
            super::damage::hide(u, id);
            u.handlers.remove(&id);
            u.fades.remove(&id);
            u.widget_sounds.remove(&id);
            u.keyboard.removed(id);
            u.drag.registered.remove(&id);
        }
        for events in u.events.values_mut() {
            events.retain(|id| !ids.contains(id));
        }
        u.frame_handlers.retain(|(id, _)| !ids.contains(id));
        u.attachments.retain(|owner, target| !ids.contains(owner) && !ids.contains(target));
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
    let w = u
        .fonts
        .layout_size(&t.style.font, t.style.size, &t.text, None, t.line_space)
        .0
        + t.inset[0]
        + t.inset[2];
    if u.tree.get(id).width != w {
        u.tree.get_mut(id).width = w;
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
            match parse_anchor(lua, id, a) {
                Some(an) => ui(lua).tree.add_anchor(id, an),
                None => bevy::log::warn!(
                    "aa-ui: AddAnchor() failed on {}: bad arguments",
                    ui(lua).tree.get(id).name
                ),
            }
            none()
        })?,
    )?;
    def(
        "RemoveAllAnchors",
        method(lua, |lua, id, _| {
            let mut u = ui(lua);
            if !u.tree.get(id).anchors.is_empty() {
                u.tree.get_mut(id).anchors.clear();
            }
            none()
        })?,
    )?;
    def(
        "SetExtent",
        method(lua, |lua, id, a| {
            let mut u = ui(lua);
            let o = u.tree.get_mut(id);
            o.width = a.f(1);
            o.height = a.f(2);
            drop(u);
            scale_event(lua, id);
            none()
        })?,
    )?;
    def(
        "SetWidth",
        method(lua, |lua, id, a| {
            ui(lua).tree.get_mut(id).width = a.f(1);
            none()
        })?,
    )?;
    def(
        "SetHeight",
        method(lua, |lua, id, a| {
            ui(lua).tree.get_mut(id).height = a.f(1);
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

    // --- visibility / state ---------------------------------------------------------------
    def(
        "Show",
        method(lua, |lua, id, a| {
            show(lua, id, a.b(1, true), a.opt_f(2));
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
            let mut stack = vec![id];
            while let Some(w) = stack.pop() {
                if let Some(wd) = u.tree.widget_mut(w) {
                    wd.enabled = on;
                    if children {
                        stack.extend(wd.children.iter().copied());
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
            let (mut u, v) = (ui(lua), a.f(1));
            let animated = u.animations.widget_sample(id).alpha;
            u.animations.clear_widget_alpha(id);
            if u.tree.get(id).alpha != v {
                u.tree.get_mut(id).alpha = v;
            }
            if animated.is_some_and(|old| old != v) && u.tree.visible(id) {
                u.tree.repaint();
            }
            none()
        })?,
    )?;
    def(
        "GetAlpha",
        method(lua, |lua, id, _| {
            let u = ui(lua);
            ret(lua, u.animations.widget_sample(id).alpha.unwrap_or(u.tree.get(id).alpha))
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
            set_layer_visible(lua, id, a.s(1).as_deref(), true);
            none()
        })?,
    )?;
    def(
        "DisableDrawables",
        method(lua, |lua, id, a| {
            set_layer_visible(lua, id, a.s(1).as_deref(), false);
            none()
        })?,
    )?;

    // --- text -------------------------------------------------------------------------------
    def(
        "SetText",
        method(lua, |lua, id, a| {
            let s = a.s(1).unwrap_or_default();
            {
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
            none()
        })?,
    )?;
    def(
        "SetAutoResize",
        method(lua, |lua, id, a| {
            let on = a.b(1, true);
            text_of(lua, id, |t| t.auto_resize = on);
            auto_size(lua, id);
            none()
        })?,
    )?;
    def(
        "GetText",
        method(lua, |lua, id, _| {
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
        "RegisterForClicks",
        "RegisterForDrag",
        "StartMoving",
        "StopMovingOrSizing",
        "SetEscEvent",
        "SetSaveConfig",
        "UseDynamicDrawableState",
        "UseDynamicContentState",
        "SetMsgToParent",
        "SetMaxTextLength",
        "SetNumberOnly",
        "UseConsistentTextColor",
        "InheritAnimationData",
        "CheckOutOfScreen",
        "CorrectOffsetByScreen",
    ] {
        def(name, method(lua, |_, _, _| none())?)?;
    }
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
                    let d = value_id(lua, &a.v(1));
                    set_button_bg(lua, id, state, d);
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
                    let c = [a.f(1), a.f(2), a.f(3), a.opt_f(4).unwrap_or(1.0)];
                    if let Some(b) = ui(lua).tree.widget_mut(id).and_then(|w| w.button.as_mut()) {
                        b.text_color[state] = Some(c);
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
                set_checked(lua, id, a.b(1, true), a.b(2, true));
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
        def(
            "SetCheckedBackground",
            method(lua, |lua, id, a| {
                let d = value_id(lua, &a.v(1));
                if let Some(b) = ui(lua).tree.widget_mut(id).and_then(|w| w.button.as_mut()) {
                    b.checked_bg = d;
                }
                none()
            })?,
        )?;
        def(
            "SetDisabledCheckedBackground",
            method(lua, |lua, id, a| {
                let d = value_id(lua, &a.v(1));
                if let Some(b) = ui(lua).tree.widget_mut(id).and_then(|w| w.button.as_mut()) {
                    b.disabled_checked_bg = d;
                }
                ui(lua).tree.touch();
                none()
            })?,
        )?;
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
    if ty == "damagedisplay" { super::damage::methods(lua, &m)?; }
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
    let Some(b) = u.tree.widget_mut(id).and_then(|w| w.button.as_mut()) else {
        return;
    };
    let changed = b.checked != checked;
    b.checked = checked;
    if changed {
        u.tree.touch();
        if fire {
            u.pending.push((id, "OnCheckChanged".into(), vec![]));
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

fn set_layer_visible(lua: &Lua, id: Id, layer: Option<&str>, on: bool) {
    let mut g = ui(lua);
    let u = &mut *g;
    let layer = layer.map(DrawLayer::parse);
    let ds = u
        .tree
        .widget(id)
        .map(|w| w.drawables.clone())
        .unwrap_or_default();
    for d in ds {
        let matches = u
            .tree
            .get(d)
            .drawable()
            .is_some_and(|dr| layer.is_none_or(|l| dr.layer == l) && dr.state_of.is_none());
        if matches {
            u.tree.get_mut(d).shown = on;
        }
    }
}

fn set_button_bg(lua: &Lua, id: Id, state: usize, d: Option<Id>) {
    let mut g = ui(lua);
    let u = &mut *g;
    let Some(b) = u.tree.widget_mut(id).and_then(|w| w.button.as_mut()) else {
        return;
    };
    b.bg[state] = d;
    let st = [
        ButtonState::Normal,
        ButtonState::Highlight,
        ButtonState::Pushed,
        ButtonState::Disabled,
    ][state];
    if let Some(d) = d {
        if let Some(dr) = u.tree.drawable_mut(d) {
            // A drawable can serve several states (SetBGPushed uses one for two).
            dr.state_of = Some((id, st));
        }
        // Becoming a state background shows the drawable again: `SetBGPushed(btn, false)`
        // restores `bgs[NORMAL]` after `SetBGPushed(btn, true)` hid it with `SetVisible(false)`
        // (race / gender buttons in reference/character_create/step1_*).
        u.tree.get_mut(d).shown = true;
    }
}

/// `OnScale` after a size change of the root-anchored windows is what the client sends on
/// resolution / UI scale changes; we only send it from `set_screen`. Kept as a hook.
fn scale_event(_lua: &Lua, _id: Id) {}

/// Text style methods (`widget.style`, `window.titleStyle`, text drawables).
pub(crate) fn style_methods(lua: &Lua) -> mlua::Result<Table> {
    let m = lua.create_table()?;
    fn with_style(lua: &Lua, this: &Value, f: impl FnOnce(&mut TextBlock)) -> mlua::Result<()> {
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
        }
        Ok(())
    }
    let sm = |name: &str, f: fn(&mut TextBlock, &Args)| -> mlua::Result<()> {
        m.raw_set(
            name,
            lua.create_function(move |lua, mv: MultiValue| {
                let a = Args::new(mv);
                with_style(lua, &a.v(0), |tb| f(tb, &a))?;
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
        t.style.color = [a.f(1), a.f(2), a.f(3), a.opt_f(4).unwrap_or(1.0)]
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
            with_style(lua, &a.v(0), |tb| {
                style = Some((tb.style.clone(), tb.text.clone()))
            })?;
            let Some((st, text)) = style else {
                return Ok(0.0);
            };
            let s = a.s(1).unwrap_or(text);
            Ok(ui(lua).fonts.measure(&st.font, st.size, &s))
        })?,
    )?;
    m.raw_set(
        "GetLineHeight",
        lua.create_function(|lua, mv: MultiValue| {
            let a = Args::new(mv);
            let mut style = None;
            with_style(lua, &a.v(0), |tb| style = Some(tb.style.clone()))?;
            let Some(st) = style else { return Ok(0.0) };
            Ok(ui(lua).fonts.metrics(&st.font, st.size).1)
        })?,
    )?;
    m.raw_set(
        "GetColor",
        lua.create_function(|lua, mv: MultiValue| {
            let a = Args::new(mv);
            let mut c = [1.0f32; 4];
            with_style(lua, &a.v(0), |tb| c = tb.style.color)?;
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
        "SetItemHeight",
        method(lua, |lua, id, a| {
            if let Some(l) = ui(lua).tree.widget_mut(id).and_then(|w| w.list.as_mut()) {
                l.row_height = a.f(1);
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
                }
                items.raw_set(items.raw_len() + 1, row)?;
            }
            relayout_rows(lua, id);
            none()
        })?,
    )?;
    m.raw_set(
        "DeleteAllRows",
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
            drop(g);
            let this = super::object_table(lua, id)?.unwrap();
            this.raw_set("items", lua.create_table()?)?;
            none()
        })?,
    )?;
    m.raw_set(
        "InsertData",
        method(lua, |lua, id, a| {
            // (row, col, text), 1-based: the row now holds data (its sub-items show); the text goes
            // into the sub-item (LCCIT_STRING columns).
            let (row, col) = (a.f(1) as usize, a.f(2) as usize);
            let text = a.s(3).unwrap_or_default();
            set_row_data(lua, id, row.wrapping_sub(1), true);
            let mut u = ui(lua);
            let sub = u
                .tree
                .widget(id)
                .and_then(|w| w.list.as_ref())
                .and_then(|l| {
                    l.sub_items
                        .get(row.wrapping_sub(1))?
                        .get(col.wrapping_sub(1))
                        .copied()
                });
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

/// Position rows below the column header and sub-items under their columns.
pub(crate) fn relayout_rows(lua: &Lua, id: Id) {
    let mut g = ui(lua);
    let u = &mut *g;
    let Some(l) = u.tree.widget(id).and_then(|w| w.list.clone()) else {
        return;
    };
    for (i, &r) in l.rows.iter().enumerate() {
        let o = u.tree.get_mut(r);
        o.height = l.row_height;
        o.anchors = vec![
            Anchor {
                point: Point::TopLeft,
                target: id,
                rel: Point::TopLeft,
                x: 0.0,
                y: l.column_height + i as f32 * l.row_height,
            },
            Anchor {
                point: Point::TopRight,
                target: id,
                rel: Point::TopRight,
                x: 0.0,
                y: l.column_height + i as f32 * l.row_height,
            },
        ];
        for (j, &s) in l
            .sub_items
            .get(i)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
            .iter()
            .enumerate()
        {
            let Some(&col) = l.columns.get(j) else {
                continue;
            };
            let o = u.tree.get_mut(s);
            o.anchors = vec![
                Anchor {
                    point: Point::TopLeft,
                    target: col,
                    rel: Point::BottomLeft,
                    x: 0.0,
                    y: i as f32 * l.row_height,
                },
                Anchor {
                    point: Point::BottomRight,
                    target: col,
                    rel: Point::BottomRight,
                    x: 0.0,
                    y: (i + 1) as f32 * l.row_height,
                },
            ];
        }
    }
}
