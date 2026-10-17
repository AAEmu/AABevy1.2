//! Edit boxes (`editbox`, `editboxmultiline`): keyboard focus, typing and the caret.
//!
//! One widget has the keyboard focus ([`Ui::focus`](super::Ui)): `SetFocus` or a click on an
//! edit box gives it, `ClearFocus`, Escape or a click elsewhere takes it. Keys go to the focused
//! widget: text is inserted at the caret (`SetMaxTextLength`, `SetDigit`, `SetDigitMax`,
//! `SetReadOnly` apply) and fires `OnTextChanged(self)`; Enter fires `OnEnterPressed(self)`
//! (multi-line boxes insert a line break), Escape `OnEscapePressed(self)`, Tab
//! `OnTabPressed(self)`. Every key also fires `OnKeyDown(self, name)` / `OnKeyUp(self, name)`
//! with lower-case names (`"enter"`, `"a"`), as `auction/bid.lua` expects.
//! `SetText` does not fire `OnTextChanged` (the money edits call `SetText` from it).

use mlua::{Lua, Table, Value};

use super::{call_handler, method, none, ret, ui};
use crate::tree::{Id, WidgetType};

/// Caret blink half period.
const BLINK_MS: f64 = 530.0;

#[derive(Clone, Debug, Default)]
pub struct EditData {
    /// Caret position in characters.
    pub cursor: usize,
    /// `SetMaxTextLength` in characters, 0 = unlimited.
    pub max_len: usize,
    pub password: bool,
    pub digit: bool,
    pub digit_max: Option<f64>,
    pub read_only: bool,
    pub clear_on_enter: bool,
    /// `EnableFocus(false)`: clicks do not focus it.
    pub no_focus: bool,
    pub cursor_color: Option<[f32; 4]>,
    /// `SetCursorHeight(n)` / `SetCursorOffset(n)`: the baselib edits use (2, -2). Read as
    /// extra caret height over the line height and a horizontal offset (no capture shows a
    /// focused edit box, so this is a guess).
    pub cursor_height: Option<f32>,
    pub cursor_offset: f32,
    /// `SetPrefixText`: drawn in `prefixStyle` before the text (the chat input's whisper target),
    /// `SetPrefixInset(left, top, right, bottom)` around it; the text starts after it.
    pub prefix: String,
    pub prefix_inset: [f32; 4],
}

/// A key for the focused widget (see [`crate::UiRuntime::key`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UiKey {
    /// Typed text (one or more characters).
    Text(String),
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Enter,
    Escape,
    Tab,
}

impl UiKey {
    /// The key name handed to `OnKeyDown` / `OnKeyUp`.
    pub fn name(&self) -> String {
        match self {
            UiKey::Text(s) => s.to_lowercase(),
            UiKey::Backspace => "backspace".into(),
            UiKey::Delete => "delete".into(),
            UiKey::Left => "left".into(),
            UiKey::Right => "right".into(),
            UiKey::Up => "up".into(),
            UiKey::Down => "down".into(),
            UiKey::Home => "home".into(),
            UiKey::End => "end".into(),
            UiKey::Enter => "enter".into(),
            UiKey::Escape => "escape".into(),
            UiKey::Tab => "tab".into(),
        }
    }
}

pub fn is_edit(ty: &WidgetType) -> bool {
    matches!(ty, WidgetType::EditBox | WidgetType::EditBoxMultiline)
}

fn with<R>(lua: &Lua, id: Id, f: impl FnOnce(&mut EditData) -> R) -> Option<R> {
    let mut u = ui(lua);
    let w = u.tree.widget_mut(id)?;
    if !is_edit(&w.ty) {
        return None;
    }
    Some(f(w.edit.get_or_insert_with(EditData::default)))
}

/// Give the keyboard focus to `id` (`None` clears it): `OnAcceptFocus(self)` on the new owner.
pub fn set_focus(lua: &Lua, id: Option<Id>) {
    {
        let mut u = ui(lua);
        if u.focus == id {
            return;
        }
        u.focus = id;
        u.caret_t0 = u.time_ms;
        u.caret_visible = id.is_some();
        // Caret at the end of the text.
        if let Some(i) = id {
            let len = u.tree.get(i).text().map_or(0, |t| t.text.chars().count());
            if let Some(w) = u.tree.widget_mut(i).filter(|w| is_edit(&w.ty)) {
                w.edit.get_or_insert_with(EditData::default).cursor = len;
            }
        }
        u.tree.touch();
    }
    if let Some(i) = id {
        call_handler(lua, i, "OnAcceptFocus", vec![]);
    }
}

/// A click on `target`: focus the edit box it is in (if it takes focus), else drop the focus.
pub(crate) fn clicked(lua: &Lua, target: Id) {
    let edit = {
        let u = ui(lua);
        let mut t = Some(target);
        let mut found = None;
        while let Some(id) = t {
            if let Some(w) = u.tree.widget(id) {
                if is_edit(&w.ty) {
                    let takes = w.enabled && !w.edit.as_ref().is_some_and(|e| e.no_focus);
                    found = takes.then_some(id);
                    break;
                }
            }
            t = u.tree.get(id).parent;
        }
        found
    };
    set_focus(lua, edit);
}

/// Blink the caret of the focused edit box (called every frame).
pub(crate) fn tick(lua: &Lua) {
    let mut u = ui(lua);
    let Some(f) = u.focus else { return };
    if !u.tree.get(f).alive || !u.tree.visible(f) {
        u.focus = None;
        u.caret_visible = false;
        u.tree.touch();
        return;
    }
    let on = ((u.time_ms - u.caret_t0) / BLINK_MS) as i64 % 2 == 0;
    if on != u.caret_visible {
        u.caret_visible = on;
        u.tree.touch();
    }
}

/// The text an edit box shows (`*` per character for passwords).
pub fn display_text(text: &str, edit: Option<&EditData>) -> String {
    if edit.is_some_and(|e| e.password) {
        "*".repeat(text.chars().count())
    } else {
        text.to_string()
    }
}

fn byte_at(s: &str, chars: usize) -> usize {
    s.char_indices().nth(chars).map_or(s.len(), |(i, _)| i)
}

/// Deliver a key press to the focused widget. Returns whether a widget had the focus.
pub fn key(lua: &Lua, k: &UiKey) -> bool {
    let Some(id) = ui(lua).focus else { return false };
    let name = Value::String(lua.create_string(k.name()).unwrap());
    call_handler(lua, id, "OnKeyDown", vec![name.clone()]);
    let multiline = ui(lua)
        .tree
        .widget(id)
        .is_some_and(|w| w.ty == WidgetType::EditBoxMultiline);
    let changed = match k {
        UiKey::Text(s) => insert(lua, id, s),
        UiKey::Enter if multiline => insert(lua, id, "\n"),
        UiKey::Backspace => erase(lua, id, true),
        UiKey::Delete => erase(lua, id, false),
        UiKey::Left | UiKey::Right | UiKey::Home | UiKey::End => {
            move_caret(lua, id, k);
            false
        }
        _ => false,
    };
    if changed {
        // The chat input takes its slash commands (`/s `, `/w name `) out of the text.
        super::chat_input::text_changed(lua, id);
        call_handler(lua, id, "OnTextChanged", vec![]);
    }
    match k {
        UiKey::Enter if !multiline => {
            // The chat input sends natively unless a script took Enter.
            if !super::chat_input::enter(lua, id) {
                call_handler(lua, id, "OnEnterPressed", vec![]);
                let clear = with(lua, id, |e| e.clear_on_enter).unwrap_or(false);
                if clear && set_text(lua, id, String::new()) {
                    call_handler(lua, id, "OnTextChanged", vec![]);
                }
            }
        }
        UiKey::Escape => {
            call_handler(lua, id, "OnEscapePressed", vec![]);
            if ui(lua).focus == Some(id) {
                set_focus(lua, None);
            }
        }
        UiKey::Tab => call_handler(lua, id, "OnTabPressed", vec![]),
        _ => {}
    }
    call_handler(lua, id, "OnKeyUp", vec![name]);
    true
}

/// Replace the text, caret at the end. Returns whether it changed.
pub(crate) fn set_text(lua: &Lua, id: Id, s: String) -> bool {
    let mut u = ui(lua);
    let n = s.chars().count();
    let Some(w) = u.tree.widget_mut(id) else { return false };
    let Some(t) = w.text.as_mut() else { return false };
    let changed = t.text != s;
    t.text = s;
    if let Some(e) = w.edit.as_mut() {
        e.cursor = n;
    }
    u.caret_t0 = u.time_ms;
    u.tree.touch();
    changed
}

fn insert(lua: &Lua, id: Id, s: &str) -> bool {
    let (text, e) = {
        let u = ui(lua);
        let Some(w) = u.tree.widget(id) else { return false };
        let text = w.text.as_ref().map(|t| t.text.clone()).unwrap_or_default();
        (text, w.edit.clone().unwrap_or_default())
    };
    if e.read_only || !ui(lua).tree.widget(id).is_some_and(|w| w.enabled) {
        return false;
    }
    let mut add: String = s
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .filter(|c| !e.digit || c.is_ascii_digit())
        .collect();
    if e.max_len > 0 {
        let room = e.max_len.saturating_sub(text.chars().count());
        add = add.chars().take(room).collect();
    }
    if add.is_empty() {
        return false;
    }
    let cursor = e.cursor.min(text.chars().count());
    let mut new = text.clone();
    new.insert_str(byte_at(&text, cursor), &add);
    let mut caret = cursor + add.chars().count();
    if let (true, Some(max)) = (e.digit, e.digit_max) {
        if new.parse::<f64>().is_ok_and(|v| v > max) {
            new = format!("{}", max as i64);
            caret = new.chars().count();
        }
    }
    store(lua, id, new, caret);
    true
}

fn erase(lua: &Lua, id: Id, before: bool) -> bool {
    let (text, e) = {
        let u = ui(lua);
        let Some(w) = u.tree.widget(id) else { return false };
        let text = w.text.as_ref().map(|t| t.text.clone()).unwrap_or_default();
        (text, w.edit.clone().unwrap_or_default())
    };
    if e.read_only {
        return false;
    }
    let n = text.chars().count();
    let cursor = e.cursor.min(n);
    let at = if before {
        if cursor == 0 {
            return false;
        }
        cursor - 1
    } else {
        if cursor >= n {
            return false;
        }
        cursor
    };
    let mut new = text.clone();
    new.replace_range(byte_at(&text, at)..byte_at(&text, at + 1), "");
    store(lua, id, new, at);
    true
}

fn store(lua: &Lua, id: Id, text: String, caret: usize) {
    let mut u = ui(lua);
    if let Some(w) = u.tree.widget_mut(id) {
        if let Some(t) = w.text.as_mut() {
            t.text = text;
        }
        w.edit.get_or_insert_with(EditData::default).cursor = caret;
    }
    u.caret_t0 = u.time_ms;
    u.caret_visible = true;
    u.tree.touch();
}

fn move_caret(lua: &Lua, id: Id, k: &UiKey) {
    let mut u = ui(lua);
    let n = u.tree.get(id).text().map_or(0, |t| t.text.chars().count());
    let Some(w) = u.tree.widget_mut(id) else { return };
    let e = w.edit.get_or_insert_with(EditData::default);
    let c = e.cursor.min(n);
    e.cursor = match k {
        UiKey::Left => c.saturating_sub(1),
        UiKey::Right => (c + 1).min(n),
        UiKey::Home => 0,
        _ => n,
    };
    u.caret_t0 = u.time_ms;
    u.caret_visible = true;
    u.tree.touch();
}

pub(crate) fn methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    let def = |name: &str, f: mlua::Function| m.raw_set(name, f);
    def(
        "SetText",
        method(lua, |lua, id, a| {
            set_text(lua, id, a.s(1).unwrap_or_default());
            none()
        })?,
    )?;
    def(
        "SetMaxTextLength",
        method(lua, |lua, id, a| {
            let n = a.f(1).max(0.0) as usize;
            with(lua, id, |e| e.max_len = n);
            none()
        })?,
    )?;
    def(
        "MaxTextLength",
        method(lua, |lua, id, _| ret(lua, with(lua, id, |e| e.max_len as i64).unwrap_or(0)))?,
    )?;
    def(
        "SetPassword",
        method(lua, |lua, id, a| {
            let on = a.b(1, true);
            with(lua, id, |e| e.password = on);
            ui(lua).tree.touch();
            none()
        })?,
    )?;
    def(
        "IsPassword",
        method(lua, |lua, id, _| ret(lua, with(lua, id, |e| e.password).unwrap_or(false)))?,
    )?;
    def(
        "SetDigit",
        method(lua, |lua, id, a| {
            let on = a.b(1, true);
            with(lua, id, |e| e.digit = on);
            none()
        })?,
    )?;
    def(
        "IsDigit",
        method(lua, |lua, id, _| ret(lua, with(lua, id, |e| e.digit).unwrap_or(false)))?,
    )?;
    def(
        "SetDigitMax",
        method(lua, |lua, id, a| {
            let v = a.opt_f(1).map(f64::from);
            with(lua, id, |e| e.digit_max = v);
            none()
        })?,
    )?;
    def(
        "SetReadOnly",
        method(lua, |lua, id, a| {
            let on = a.b(1, true);
            with(lua, id, |e| e.read_only = on);
            none()
        })?,
    )?;
    def(
        "ClearTextOnEnter",
        method(lua, |lua, id, a| {
            let on = a.b(1, true);
            with(lua, id, |e| e.clear_on_enter = on);
            none()
        })?,
    )?;
    def(
        "EnableFocus",
        method(lua, |lua, id, a| {
            let on = a.b(1, true);
            with(lua, id, |e| e.no_focus = !on);
            none()
        })?,
    )?;
    def(
        "SetCursorColor",
        method(lua, |lua, id, a| {
            let c = [a.f(1), a.f(2), a.f(3), a.opt_f(4).unwrap_or(1.0)];
            with(lua, id, |e| e.cursor_color = Some(c));
            none()
        })?,
    )?;
    def(
        "SetCursorHeight",
        method(lua, |lua, id, a| {
            let h = a.f(1);
            with(lua, id, |e| e.cursor_height = Some(h));
            none()
        })?,
    )?;
    def("IsComposingText", method(lua, |lua, _, _| ret(lua, false))?)?;
    def(
        "SetCursorOffset",
        method(lua, |lua, id, a| {
            let o = a.f(1);
            with(lua, id, |e| e.cursor_offset = o);
            none()
        })?,
    )?;
    def(
        "SetPrefixText",
        method(lua, |lua, id, a| {
            let t = a.s(1).unwrap_or_default();
            with(lua, id, |e| e.prefix = t);
            ui(lua).tree.touch();
            none()
        })?,
    )?;
    def(
        "SetPrefixInset",
        method(lua, |lua, id, a| {
            let i = [a.f(1), a.f(2), a.f(3), a.f(4)];
            with(lua, id, |e| e.prefix_inset = i);
            ui(lua).tree.touch();
            none()
        })?,
    )?;
    for name in ["SetEnglish", "UseSelectAllWhenFocused", "UseClipboard"] {
        def(name, method(lua, |_, _, _| none())?)?;
    }
    Ok(())
}

/// `SetFocus` / `ClearFocus` (every class that lists them).
pub(crate) fn focus_methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    m.raw_set(
        "SetFocus",
        method(lua, |lua, id, _| {
            set_focus(lua, Some(id));
            none()
        })?,
    )?;
    m.raw_set(
        "ClearFocus",
        method(lua, |lua, id, _| {
            if ui(lua).focus == Some(id) {
                set_focus(lua, None);
            }
            none()
        })?,
    )?;
    Ok(())
}
