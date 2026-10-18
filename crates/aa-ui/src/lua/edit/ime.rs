//! Native IME visible buffer: model 39227da0 / result 392277b0 / start 392276f0.

use bevy::window::Ime;
use mlua::Lua;

use super::{byte_at, call_handler, geometry, is_edit, selection, store, ui};
use crate::tree::Id;

#[derive(Clone)]
struct Composition {
    owner: Id,
    base: String,
    anchor: usize,
    shown: String,
    length: usize,
    /// SetEnglish clears model buffers without clearing the native composing flag.
    cleared: bool,
}

#[derive(Default)]
struct State {
    enabled_owner: Option<Id>,
    window_owner: Option<Id>,
    rearm: bool,
    reset: bool,
    active: Option<Composition>,
    blocked: bool,
}

fn state(lua: &Lua) -> mlua::AppDataRefMut<'_, State> {
    if lua.app_data_ref::<State>().is_none() {
        lua.set_app_data(State::default());
    }
    lua.app_data_mut::<State>().unwrap()
}

fn eligible(lua: &Lua) -> Option<Id> {
    let u = ui(lua);
    let id = u.focus?;
    let w = u.tree.widget(id)?;
    (u.keyboard.owner(&u.tree, u.focus) == Some(id)
        && u.tree.visible(id)
        && w.enabled
        && is_edit(&w.ty)
        && !w
            .edit
            .as_ref()
            .is_some_and(|e| e.english || e.digit || e.read_only || e.password))
    .then_some(id)
}

fn text(lua: &Lua, id: Id) -> Option<String> {
    ui(lua)
        .tree
        .widget(id)?
        .text
        .as_ref()
        .map(|t| t.text.clone())
}

fn changed(lua: &Lua, id: Id) {
    super::super::chat_input::text_changed(lua, id);
    call_handler(lua, id, "OnTextChanged", vec![]);
}

pub(crate) fn begin_frame(lua: &Lua) {
    state(lua).blocked = false;
    let active = state(lua).active.clone();
    if let Some(c) = active {
        if text(lua, c.owner).as_deref() != Some(&c.shown) {
            state(lua).active = None;
        } else if eligible(lua) != Some(c.owner) {
            // Native focus release 39292290 completes before detaching the context.
            release(lua, c.owner);
        }
    }
}

pub(crate) fn composing(lua: &Lua, id: Id) -> bool {
    state(lua).active.as_ref().is_some_and(|c| c.owner == id)
}

pub(crate) fn blocks_key(lua: &Lua, id: Id) -> bool {
    let s = state(lua);
    s.blocked || s.active.as_ref().is_some_and(|c| c.owner == id)
}

pub(crate) fn forget(lua: &Lua, id: Id) {
    let mut s = state(lua);
    if s.active.as_ref().is_some_and(|c| c.owner == id) {
        s.active = None;
        s.enabled_owner = None;
        s.reset = true;
    }
}

///39228c70's model clear retains composing until the context or result completes.
pub(crate) fn clear_model(lua: &Lua, id: Id) {
    let mut s = state(lua);
    if let Some(c) = s.active.as_mut().filter(|c| c.owner == id) {
        c.base.clear();
        c.shown.clear();
        c.anchor = 0;
        c.length = 0;
        c.cleared = true;
    }
}

/// Detach the previous owner before a focus transfer or OS focus loss.
pub(crate) fn release(lua: &Lua, id: Id) {
    if state(lua).enabled_owner == Some(id) {
        state(lua).enabled_owner = None;
    }
    complete(lua, id);
}

/// Complete the visible composition on native focus/context release. Never overwrite a
/// newer application edit; late OS result events remain tied to the previous enabled owner.
pub(crate) fn complete(lua: &Lua, id: Id) {
    let c = {
        let mut s = state(lua);
        if !s.active.as_ref().is_some_and(|c| c.owner == id) {
            return;
        }
        s.active.take().unwrap()
    };
    if !c.cleared && text(lua, id).as_deref() == Some(&c.shown) {
        let shown_len = c.shown.chars().count();
        let caret = if c.base.chars().count() + c.length > shown_len {
            shown_len
        } else {
            c.anchor + c.length
        };
        store(lua, id, c.shown, caret);
        changed(lua, id);
    }
}

fn begin(lua: &Lua, id: Id) -> Option<Composition> {
    let active = state(lua).active.clone();
    if let Some(c) = active {
        if c.owner == id && text(lua, id).as_deref() == Some(&c.shown) {
            return Some(c);
        }
        state(lua).active = None;
    }
    let (mut base, edit) = {
        let u = ui(lua);
        let w = u.tree.widget(id)?;
        (
            w.text.as_ref()?.text.clone(),
            w.edit.clone().unwrap_or_default(),
        )
    };
    let range = selection(&edit, base.chars().count());
    if !range.is_empty() {
        base.replace_range(byte_at(&base, range.start)..byte_at(&base, range.end), "");
        store(lua, id, base.clone(), range.start);
        changed(lua, id);
        if eligible(lua) != Some(id) || text(lua, id).as_deref() != Some(&base) {
            return None;
        }
    }
    Some(Composition {
        owner: id,
        shown: base.clone(),
        base,
        anchor: range.start,
        length: 0,
        cleared: false,
    })
}

fn cancel(lua: &Lua) {
    let c = state(lua).active.take();
    if let Some(c) = c
        && text(lua, c.owner).as_deref() == Some(&c.shown)
    {
        store(lua, c.owner, c.base, c.anchor);
        changed(lua, c.owner);
    }
}

/// Forward a window IME event. Physical key names are separate and no synthetic key
/// events are invented for a composition/result string.
pub fn event(lua: &Lua, event: &Ime) -> bool {
    match event {
        Ime::Enabled { .. } => {
            let owner = eligible(lua);
            state(lua).enabled_owner = owner;
            return owner.is_some();
        }
        Ime::Disabled { .. } => {
            let owner = state(lua).enabled_owner.take();
            if let Some(owner) = owner {
                complete(lua, owner);
            }
            return owner.is_some();
        }
        _ => {}
    }
    let Some(id) = eligible(lua).filter(|id| state(lua).enabled_owner == Some(*id)) else {
        return false;
    };
    state(lua).blocked = true;
    ui(lua).keyboard.consume();
    let (value, cursor, commit) = match event {
        Ime::Preedit { value, cursor, .. } => (value, *cursor, false),
        Ime::Commit { value, .. } => (value, None, true),
        _ => unreachable!(),
    };
    if !commit && value.is_empty() {
        cancel(lua);
        return true;
    }
    if value.is_empty() {
        // Native result 392277bf clears composing even for an empty result. The
        // visible buffer becomes committed at 392278ff; no dirty flag/caret move.
        let c = state(lua).active.take();
        if let Some(c) = c
            && text(lua, id).as_deref() == Some(&c.shown)
        {
            let caret = ui(lua)
                .tree
                .widget(id)
                .and_then(|w| w.edit.as_ref())
                .map_or(0, |e| e.cursor);
            store(lua, id, c.shown, caret);
        }
        return true;
    }
    let Some(mut c) = begin(lua, id) else {
        return true;
    };
    // Native 39224c10 first reads at most 0x400 bytes (512 UTF-16 units),
    // then converts through UTF-8 to the 513-element Unicode32 destination.
    let mut units = 0;
    let value: String = value
        .chars()
        .take_while(|c| {
            units += c.len_utf16();
            *c != '\0' && units <= 512
        })
        .collect();
    let mut shown = c.base.clone();
    shown.insert_str(byte_at(&shown, c.anchor), &value);
    let limit = ui(lua)
        .tree
        .widget(id)
        .and_then(|w| w.edit.as_ref())
        .map_or(0, |e| e.max_len);
    let overflow = limit > 0 && shown.chars().count() > limit;
    if overflow {
        shown = shown.chars().take(limit).collect();
    }
    c.length = value
        .chars()
        .count()
        .min(shown.chars().count().saturating_sub(c.anchor));
    // Winit's cursor range uses UTF-8 byte offsets. Native cursor is scalar-based.
    let cursor = cursor.map(|(a, _)| {
        value
            .char_indices()
            .take_while(|(byte, _)| *byte < a.min(value.len()))
            .count()
            .min(c.length)
    });
    let caret = if commit && overflow {
        // Native result 392278ac..c7 places both caret and insertion anchor at the limit.
        limit
    } else {
        c.anchor + cursor.unwrap_or(c.length)
    };
    c.shown = shown.clone();
    c.cleared = false;
    state(lua).active = (!commit).then_some(c);
    store(lua, id, shown, caret);
    if !commit && let Some(e) = ui(lua).tree.widget_mut(id).and_then(|w| w.edit.as_mut()) {
        e.ime_cursor = Some(cursor.is_some());
    }
    changed(lua, id);
    true
}

/// Restart the OS context between edit owners, mirroring native detach/reattach. The
/// caller supplies physical screen scale and converts the returned UI point to OS units.
pub(crate) fn window(lua: &Lua, focused: bool) -> (bool, Option<[f32; 2]>) {
    if !focused {
        let owner = state(lua).active.as_ref().map(|c| c.owner);
        if let Some(owner) = owner {
            release(lua, owner);
        }
    }
    let desired = focused.then(|| eligible(lua)).flatten();
    let enabled = {
        let mut s = state(lua);
        if s.reset {
            s.reset = false;
            s.window_owner = desired;
            s.enabled_owner = None;
            s.rearm = desired.is_some();
            false
        } else if s.window_owner != desired {
            let had_owner = s.window_owner.is_some();
            s.window_owner = desired;
            s.enabled_owner = None;
            s.rearm = had_owner && desired.is_some();
            !s.rearm && desired.is_some()
        } else if s.rearm {
            s.rearm = false;
            desired.is_some()
        } else {
            desired.is_some()
        }
    };
    let point = desired.and_then(|id| {
        let mut u = ui(lua);
        let g = geometry::widget(&mut u, id)?;
        let edit = u.tree.widget(id)?.edit.clone().unwrap_or_default();
        let p = g.point_on(edit.cursor, edit.visual_line);
        let base = u.tree.rect(id);
        let view = u.animations.widget_rect(&u.tree, id);
        Some([
            view.x + (p[0] - base.x) * view.w / base.w.max(0.001),
            view.y + (p[1] + g.line_height - base.y) * view.h / base.h.max(0.001),
        ])
    });
    (enabled, point)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{UiRuntime, UiStats};
    use bevy::{
        input::{
            ButtonState, InputPlugin,
            keyboard::{Key, KeyboardInput},
        },
        prelude::*,
        window::{PrimaryWindow, WindowResolution},
    };

    #[test]
    fn bevy_primary_window_orders_result_before_enter_and_restarts_transferred_context() {
        let rt = UiRuntime::new("en_us", 1920.0, 1080.0).unwrap();
        rt.exec(
            "@ime_bevy",
            r#"
            edit=UIParent:CreateWidget('editbox','imeBevyEdit','UIParent')
            edit:SetExtent(200,30); edit:AddAnchor('TOPLEFT',UIParent,100,100)
            edit:Show(true); edit:SetFocus()
            changes={}; enters=0
            edit:SetHandler('OnTextChanged',function(self) table.insert(changes,self:GetText()) end)
            edit:SetHandler('OnEnterPressed',function() enters=enters+1 end)
        "#,
        )
        .unwrap();
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, InputPlugin))
            .add_message::<Ime>()
            .init_resource::<UiStats>()
            .insert_non_send(rt)
            .add_systems(Update, (crate::ui_keyboard, crate::ui_ime_window).chain());
        let primary = app
            .world_mut()
            .spawn((
                Window {
                    resolution: WindowResolution::new(960, 540).with_scale_factor_override(2.0),
                    ..default()
                },
                PrimaryWindow,
            ))
            .id();
        let other = app.world_mut().spawn_empty().id();
        app.update();
        assert!(app.world().get::<Window>(primary).unwrap().ime_enabled);
        let initial = app.world().get::<Window>(primary).unwrap().ime_position;
        assert!(
            initial.x >= 50.0 && initial.x <= 150.0 && initial.y >= 50.0 && initial.y <= 65.0,
            "{initial:?}"
        );
        app.world_mut()
            .write_message(Ime::Enabled { window: primary });
        app.world_mut().write_message(Ime::Preedit {
            window: other,
            value: "wrong window".into(),
            cursor: None,
        });
        app.world_mut().write_message(Ime::Preedit {
            window: primary,
            value: "preview".into(),
            cursor: Some((7, 7)),
        });
        app.update();
        let rt = app.world().non_send::<UiRuntime>();
        assert_eq!(
            rt.lua.load("edit:GetText()").eval::<String>().unwrap(),
            "preview"
        );
        app.world_mut().write_message(Ime::Preedit {
            window: primary,
            value: String::new(),
            cursor: None,
        });
        app.world_mut().write_message(Ime::Commit {
            window: primary,
            value: "result".into(),
        });
        let enter = KeyboardInput {
            key_code: KeyCode::Enter,
            logical_key: Key::Enter,
            state: ButtonState::Pressed,
            text: None,
            repeat: false,
            window: primary,
        };
        app.world_mut().write_message(enter.clone());
        app.update();
        let rt = app.world().non_send::<UiRuntime>();
        assert_eq!(
            rt.lua
                .load("table.concat(changes,',')")
                .eval::<String>()
                .unwrap(),
            "preview,result"
        );
        assert_eq!(rt.lua.load("enters").eval::<u32>().unwrap(), 0);
        app.world_mut().write_message(enter);
        app.update();
        let rt = app.world().non_send::<UiRuntime>();
        assert_eq!(rt.lua.load("enters").eval::<u32>().unwrap(), 1);
        rt.exec(
            "@transfer",
            r#"
            other=UIParent:CreateWidget('editbox','imeBevyOther','UIParent')
            other:SetExtent(200,30); other:Show(true); other:SetFocus()
        "#,
        )
        .unwrap();
        app.update();
        assert!(!app.world().get::<Window>(primary).unwrap().ime_enabled);
        app.world_mut().write_message(Ime::Commit {
            window: primary,
            value: "stale".into(),
        });
        app.update();
        assert!(app.world().get::<Window>(primary).unwrap().ime_enabled);
        let rt = app.world().non_send::<UiRuntime>();
        assert_eq!(rt.lua.load("other:GetText()").eval::<String>().unwrap(), "");
        app.world_mut().get_mut::<Window>(primary).unwrap().focused = false;
        app.update();
        assert!(!app.world().get::<Window>(primary).unwrap().ime_enabled);
    }
}
