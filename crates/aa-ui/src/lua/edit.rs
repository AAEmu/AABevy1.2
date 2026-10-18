//! Edit boxes (`editbox`, `editboxmultiline`): keyboard focus, typing and the caret.
//!
//! One widget has the keyboard focus ([`Ui::focus`](super::Ui)): `SetFocus` or a click on an
//! edit box gives it, `ClearFocus`, Escape or a click elsewhere takes it. Keys go to the focused
//! widget: text is inserted at the caret (`SetMaxTextLength`, `SetDigit`, `SetDigitMax`,
//! `SetReadOnly` apply) and fires `OnTextChanged(self)`; Enter fires `OnEnterPressed(self)`
//! (multi-line boxes insert a line break), Escape `OnEscapePressed(self)`, Tab
//! `OnTabPressed(self)`. Physical keys fire `OnKeyDown(self, name)` on press and
//! `OnKeyUp(self, name)` on release; programmatic `UiRuntime::key` sends both.
//! Changed programmatic `SetText` dispatches `OnTextChanged` synchronously; same text is a no-op.

use mlua::{Lua, Table};

use super::{call_handler, method, none, ret, ui};
use crate::tree::{Id, WidgetType};

/// Caret blink half period.
const BLINK_MS: f64 = 530.0;

mod clipboard;
mod geometry;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EditData {
    /// Caret position in characters.
    pub cursor: usize,
    pub selection_anchor: Option<usize>,
    pub select_all_on_focus: bool,
    pub scroll: [f32; 2],
    pub preferred_x: Option<f32>,
    /// Affinity of a caret shared by the end/start of two soft-wrapped lines.
    pub visual_line: Option<usize>,
    /// Press coordinates, character boundary, and whether initial focus selected all.
    pub pointer: Option<((f32, f32), usize, bool)>,
    /// `SetMaxTextLength` in characters, 0 = unlimited.
    pub max_len: usize,
    pub password: bool,
    pub digit: bool,
    /// Native defaults to enabled; SetDigit(true) disables until UseClipboard(true).
    pub clipboard_disabled: bool,
    pub digit_max: Option<f64>,
    /// `SetInitVal`: empty, non-password digit edits materialize this value on GetText.
    pub init_val: i32,
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
    SelectAll,
    Copy,
    Cut,
    Paste,
    /// Native Ctrl+Shift+Insert runs both shortcut branches, paste then copy-all.
    PasteAndCopy,
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
            UiKey::SelectAll => "a".into(),
            UiKey::Copy => "c".into(),
            UiKey::Cut => "x".into(),
            UiKey::Paste => "v".into(),
            UiKey::PasteAndCopy => "insert".into(),
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
    let old = {
        let mut u = ui(lua);
        if u.focus == id {
            return;
        }
        let old = std::mem::replace(&mut u.focus, id);
        if let Some(old) = old
            && let Some(e) = u.tree.widget_mut(old).and_then(|w| w.edit.as_mut())
        {
            e.pointer = None;
        }
        u.caret_t0 = u.time_ms;
        u.caret_visible = id.is_some_and(|id| u.tree.widget(id).is_some_and(|w| is_edit(&w.ty)));
        // Caret at the end of the text.
        if let Some(i) = id {
            let len = u.tree.get(i).text().map_or(0, |t| t.text.chars().count());
            if let Some(w) = u.tree.widget_mut(i).filter(|w| is_edit(&w.ty)) {
                let edit = w.edit.get_or_insert_with(EditData::default);
                edit.cursor = len;
                edit.selection_anchor = edit.select_all_on_focus.then_some(0);
                edit.preferred_x = None;
                edit.visual_line = None;
            }
        }
        u.tree.repaint();
        if let Some(i) = id {
            geometry::ensure_caret(&mut u, i);
        }
        old
    };
    if let Some(old) = old {
        call_handler(lua, old, "OnReleaseFocus", vec![]);
    }
    if let Some(i) = id
        && ui(lua).focus == Some(i)
    {
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
            if let Some(w) = u.tree.widget(id)
                && (is_edit(&w.ty) || u.keyboard.takes_focus(id))
            {
                let takes = w.enabled && !w.edit.as_ref().is_some_and(|e| e.no_focus);
                found = takes.then_some(id);
                break;
            }
            t = u.tree.get(id).parent;
        }
        found
    };
    let old = ui(lua).focus;
    // Focusing moves the keyboard caret to the end. A pointer press must still pick from
    // the view the user clicked, before that keyboard-only scroll adjustment.
    let scroll = edit
        .and_then(|id| {
            ui(lua)
                .tree
                .widget(id)
                .and_then(|w| w.edit.as_ref())
                .map(|e| e.scroll)
        })
        .unwrap_or_default();
    set_focus(lua, edit);
    if let Some(id) = edit {
        let mut u = ui(lua);
        if u.focus != Some(id) || !u.tree.widget(id).is_some_and(|w| is_edit(&w.ty)) {
            return;
        }
        let selected_all = old != Some(id)
            && u.tree
                .widget(id)
                .and_then(|w| w.edit.as_ref())
                .is_some_and(|e| e.select_all_on_focus);
        if !selected_all && let Some(e) = u.tree.widget_mut(id).and_then(|w| w.edit.as_mut()) {
            e.scroll = scroll;
        }
        let cursor = u.cursor;
        let p = geometry::pointer(&u, id, cursor);
        let Some(g) = geometry::widget(&mut u, id) else {
            return;
        };
        let at = g.pick(p[0], p[1]);
        let line = g.picked_line(p[1]);
        let extend = u.keyboard.modifiers().shift;
        let e = u
            .tree
            .widget_mut(id)
            .unwrap()
            .edit
            .get_or_insert_with(Default::default);
        let drag_anchor = if extend {
            e.selection_anchor.unwrap_or(e.cursor)
        } else {
            at
        };
        e.pointer = Some((cursor, drag_anchor, selected_all));
        if !selected_all {
            if extend {
                e.selection_anchor.get_or_insert(e.cursor);
            } else {
                e.selection_anchor = None;
            }
            e.cursor = at;
            e.visual_line = Some(line);
            e.preferred_x = None;
            geometry::ensure_caret(&mut u, id);
        }
    }
}

pub(crate) fn pointer_move(lua: &Lua) {
    let mut u = ui(lua);
    let Some(id) = u.focus else { return };
    let e = u
        .tree
        .widget(id)
        .and_then(|w| w.edit.as_ref())
        .cloned()
        .unwrap_or_default();
    let Some((press, anchor, select_all)) = e.pointer else {
        return;
    };
    let cursor = u.cursor;
    if select_all && (cursor.0 - press.0).hypot(cursor.1 - press.1) < 3.0 {
        return;
    }
    let p = geometry::pointer(&u, id, cursor);
    let Some(g) = geometry::widget(&mut u, id) else {
        return;
    };
    let at = g.pick(p[0], p[1]);
    let line = g.picked_line(p[1]);
    if (e.cursor, e.selection_anchor, e.visual_line) != (at, Some(anchor), Some(line)) {
        let edit = u.tree.widget_mut(id).unwrap().edit.as_mut().unwrap();
        edit.cursor = at;
        edit.selection_anchor = Some(anchor);
        edit.preferred_x = None;
        edit.visual_line = Some(line);
        u.caret_t0 = u.time_ms;
        u.caret_visible = true;
        geometry::ensure_caret(&mut u, id);
    }
}

pub(crate) fn pointer_up(lua: &Lua) {
    let mut u = ui(lua);
    if let Some(id) = u.focus
        && u.tree
            .widget(id)
            .and_then(|w| w.edit.as_ref())
            .is_some_and(|e| e.pointer.is_some())
    {
        u.tree
            .widget_mut(id)
            .unwrap()
            .edit
            .as_mut()
            .unwrap()
            .pointer = None;
    }
}

pub(crate) fn wheel(lua: &Lua, mut id: Id, delta: f32) -> bool {
    let mut u = ui(lua);
    loop {
        if u.tree
            .widget(id)
            .is_some_and(|w| w.ty == WidgetType::EditBoxMultiline)
        {
            break;
        }
        let Some(parent) = u.tree.get(id).parent else {
            return false;
        };
        id = parent;
    }
    let Some(g) = geometry::widget(&mut u, id) else {
        return false;
    };
    let old = u
        .tree
        .widget(id)
        .and_then(|w| w.edit.as_ref())
        .map_or(0.0, |e| e.scroll[1]);
    let max = (g.layout.height / g.scale - g.inner.h).max(0.0);
    let next = (old - delta * g.step * 3.0).clamp(0.0, max);
    if old != next {
        u.tree
            .widget_mut(id)
            .unwrap()
            .edit
            .get_or_insert_with(Default::default)
            .scroll[1] = next;
    }
    true
}

/// Blink the caret of the focused edit box (called every frame).
pub(crate) fn tick(lua: &Lua) {
    let hidden = {
        let u = ui(lua);
        u.focus
            .is_some_and(|f| !u.tree.get(f).alive || !u.tree.visible(f))
    };
    if hidden {
        set_focus(lua, None);
        return;
    }
    let mut u = ui(lua);
    let Some(f) = u.focus else { return };
    if !u.tree.widget(f).is_some_and(|w| is_edit(&w.ty)) {
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
    super::keyboard::key(lua, k)
}

pub(crate) fn press(lua: &Lua, id: Id, k: &UiKey) {
    let multiline = ui(lua)
        .tree
        .widget(id)
        .is_some_and(|w| w.ty == WidgetType::EditBoxMultiline);
    let changed = match k {
        UiKey::Text(s) => insert(lua, id, s),
        UiKey::Enter if multiline => insert(lua, id, "\n"),
        UiKey::Backspace => erase(lua, id, true),
        UiKey::Delete => erase(lua, id, false),
        UiKey::Copy | UiKey::Cut | UiKey::Paste => clipboard_key(lua, id, k),
        UiKey::PasteAndCopy => {
            let changed = clipboard_key(lua, id, &UiKey::Paste);
            let owner = {
                let u = ui(lua);
                u.keyboard.owner(&u.tree, u.focus)
            };
            if owner == Some(id) {
                clipboard_key(lua, id, &UiKey::Copy);
            }
            changed
        }
        UiKey::Left
        | UiKey::Right
        | UiKey::Up
        | UiKey::Down
        | UiKey::Home
        | UiKey::End
        | UiKey::SelectAll => {
            move_caret(lua, id, k);
            false
        }
        _ => false,
    };
    if changed {
        // `SetSounds("edit_box")`: a key click per typed or erased character
        super::sound::widget_sound(
            lua,
            id,
            if matches!(k, UiKey::Backspace | UiKey::Delete | UiKey::Cut) {
                "text_deleted"
            } else {
                "text_added"
            },
        );
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
}

/// Replace the text, caret at the end. Returns whether it changed.
pub(crate) fn set_text(lua: &Lua, id: Id, s: String) -> bool {
    let mut u = ui(lua);
    let n = s.chars().count();
    let Some(w) = u.tree.widget_mut(id) else {
        return false;
    };
    let Some(t) = w.text.as_mut() else {
        return false;
    };
    let changed = t.text != s;
    t.text = s;
    if let Some(e) = w.edit.as_mut() {
        e.cursor = n;
        e.selection_anchor = None;
        e.preferred_x = None;
        e.visual_line = None;
        e.scroll = [0.0; 2];
    }
    u.caret_t0 = u.time_ms;
    u.tree.touch();
    if u.focus == Some(id) {
        geometry::ensure_caret(&mut u, id);
    }
    changed
}

fn insert(lua: &Lua, id: Id, s: &str) -> bool {
    let (text, e) = {
        let u = ui(lua);
        let Some(w) = u.tree.widget(id) else {
            return false;
        };
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
        let selected = selection(&e, text.chars().count());
        let room = e
            .max_len
            .saturating_sub(text.chars().count() - (selected.end - selected.start));
        add = add.chars().take(room).collect();
    }
    if add.is_empty() {
        return false;
    }
    let selected = selection(&e, text.chars().count());
    let cursor = selected.start;
    let mut new = text.clone();
    new.replace_range(
        byte_at(&text, selected.start)..byte_at(&text, selected.end),
        &add,
    );
    let mut caret = cursor + add.chars().count();
    if let (true, Some(max)) = (e.digit, e.digit_max)
        && new.parse::<f64>().is_ok_and(|v| v > max)
    {
        new = format!("{}", max as i64);
        caret = new.chars().count();
    }
    store(lua, id, new, caret);
    true
}

fn erase(lua: &Lua, id: Id, before: bool) -> bool {
    let (text, e) = {
        let u = ui(lua);
        let Some(w) = u.tree.widget(id) else {
            return false;
        };
        let text = w.text.as_ref().map(|t| t.text.clone()).unwrap_or_default();
        (text, w.edit.clone().unwrap_or_default())
    };
    if e.read_only || !ui(lua).tree.widget(id).is_some_and(|w| w.enabled) {
        return false;
    }
    let n = text.chars().count();
    let cursor = e.cursor.min(n);
    let selected = selection(&e, n);
    if !selected.is_empty() {
        let mut new = text.clone();
        new.replace_range(
            byte_at(&text, selected.start)..byte_at(&text, selected.end),
            "",
        );
        store(lua, id, new, selected.start);
        return true;
    }
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
        let edit = w.edit.get_or_insert_with(EditData::default);
        edit.cursor = caret;
        edit.selection_anchor = None;
        edit.preferred_x = None;
        edit.visual_line = None;
    }
    u.caret_t0 = u.time_ms;
    u.caret_visible = true;
    u.tree.touch();
    geometry::ensure_caret(&mut u, id);
}

fn clipboard_key(lua: &Lua, id: Id, key: &UiKey) -> bool {
    let (text, e) = {
        let u = ui(lua);
        let Some(w) = u.tree.widget(id).filter(|w| w.enabled) else {
            return false;
        };
        let e = w.edit.clone().unwrap_or_default();
        // Native39292840 sets model+9b;3922a0d1 rejects input before clipboard handling.
        if e.clipboard_disabled || e.read_only {
            return false;
        }
        (
            w.text.as_ref().map(|t| t.text.clone()).unwrap_or_default(),
            e,
        )
    };
    let selected = selection(&e, text.chars().count());
    match key {
        UiKey::Copy => {
            let range = if selected.is_empty() {
                0..text.chars().count()
            } else {
                selected
            };
            clipboard::set(
                lua,
                &text[byte_at(&text, range.start)..byte_at(&text, range.end)],
            );
            false
        }
        UiKey::Cut if !selected.is_empty() => {
            let copied =
                text[byte_at(&text, selected.start)..byte_at(&text, selected.end)].to_string();
            let mut new = text.clone();
            new.replace_range(
                byte_at(&text, selected.start)..byte_at(&text, selected.end),
                "",
            );
            // Native cut erases before trying the OS clipboard, including a failed write.
            store(lua, id, new, selected.start);
            clipboard::set(lua, &copied);
            true
        }
        UiKey::Paste => {
            let pasted = clipboard::paste(lua);
            // Hooks run outside all Ui borrows and may move focus or disable editing.
            let u = ui(lua);
            if u.keyboard.owner(&u.tree, u.focus) != Some(id)
                || !u.tree.widget(id).is_some_and(|w| {
                    w.enabled
                        && w.text.as_ref().is_some_and(|t| t.text == text)
                        && w.edit.clone().unwrap_or_default() == e
                })
            {
                return false;
            }
            drop(u);
            let mut add = pasted;
            if e.max_len > 0 {
                let room = e
                    .max_len
                    .saturating_sub(text.chars().count() - selected.len());
                add = add.chars().take(room).collect();
            }
            let mut new = text.clone();
            new.replace_range(
                byte_at(&text, selected.start)..byte_at(&text, selected.end),
                &add,
            );
            // Paste is a direct buffer operation in native39228270: digit filtering is bypassed.
            store(lua, id, new, selected.start + add.chars().count());
            true
        }
        _ => false,
    }
}

pub(crate) fn selection(edit: &EditData, len: usize) -> std::ops::Range<usize> {
    let caret = edit.cursor.min(len);
    let anchor = edit.selection_anchor.unwrap_or(caret).min(len);
    anchor.min(caret)..anchor.max(caret)
}

pub(crate) fn render(
    u: &mut super::Ui,
    id: Id,
    text: &crate::tree::TextBlock,
    edit: Option<&EditData>,
    rect: crate::tree::Rect,
    alpha: f32,
    scale: f32,
    caret: bool,
    out: &mut Vec<crate::render::Quad>,
) {
    use crate::{
        render::{Quad, Tex},
        tree::{Align, Rect},
    };
    let fallback = EditData::default();
    let edit = edit.unwrap_or(&fallback);
    let multiline = u
        .tree
        .widget(id)
        .is_some_and(|w| w.ty == WidgetType::EditBoxMultiline);
    let g = geometry::Geometry::new(u, text, edit, rect, multiline);
    let start = out.len();
    let range = selection(edit, text.text.chars().count());
    if !range.is_empty() {
        let from = g.caret(range.start);
        let to = g.caret(range.end);
        // Native 39220fd0 uses the global Textures/Defaults/grey.dds, loaded at3921ddba.
        let texture = u.textures.resolve("textures/defaults/grey.dds");
        for line in from.line..=to.line {
            let lo = if line == from.line { from.x } else { 0.0 };
            let hi = if line == to.line {
                to.x
            } else {
                g.layout.line_widths.get(line).copied().unwrap_or(0.0)
            };
            if hi <= lo {
                continue;
            }
            let x = g.origin[0] * scale + lo;
            let y = (g.origin[1] + line as f32 * g.step) * scale;
            let color = if texture.is_some() {
                [1.0, 1.0, 1.0, alpha]
            } else {
                [0.5, 0.5, 0.5, alpha]
            };
            out.push(Quad {
                tex: texture.clone().map_or(Tex::Atlas, Tex::Image),
                pos: [x, y, g.origin[0] * scale + hi, y + g.line_height * scale],
                uv: if texture.is_some() {
                    [0.0, 0.0, 1.0, 1.0]
                } else {
                    crate::render::white_uv()
                },
                color,
                corners: None,
                uvs: None,
                vcolors: None,
            });
        }
    }
    let mut shown = text.clone();
    shown.inset = [0.0; 4];
    shown.style.align = Align::TopLeft;
    shown.wrap = multiline;
    let r = Rect {
        x: g.origin[0],
        y: g.origin[1],
        w: g.inner.w,
        h: g.inner.h,
    };
    crate::render::edit_text_quads(u, &shown, r, shown.style.color, alpha, scale, out);
    if caret {
        let p = g.point_on(edit.cursor, edit.visual_line);
        let h = g.line_height + edit.cursor_height.unwrap_or(0.0);
        let x = ((p[0] + edit.cursor_offset) * scale).round();
        let y = ((p[1] + (g.line_height - h) * 0.5) * scale).round();
        let c = edit.cursor_color.unwrap_or(text.style.color);
        out.push(Quad {
            tex: Tex::Atlas,
            pos: [x, y, x + scale.max(1.0), y + h * scale],
            uv: crate::render::white_uv(),
            color: [c[0], c[1], c[2], c[3] * alpha],
            corners: None,
            uvs: None,
            vcolors: None,
        });
    }
    crate::render::clip::quads(
        out,
        start,
        [
            g.inner.x * scale,
            g.inner.y * scale,
            g.inner.right() * scale,
            g.inner.bottom() * scale,
        ],
    );
}

fn move_caret(lua: &Lua, id: Id, k: &UiKey) {
    let mut u = ui(lua);
    let n = u.tree.get(id).text().map_or(0, |t| t.text.chars().count());
    let modifiers = u.keyboard.modifiers();
    let multiline = u
        .tree
        .widget(id)
        .is_some_and(|w| w.ty == WidgetType::EditBoxMultiline);
    let Some(g) = geometry::widget(&mut u, id) else {
        return;
    };
    let Some(w) = u.tree.widget_mut(id) else {
        return;
    };
    let e = w.edit.get_or_insert_with(EditData::default);
    let c = e.cursor.min(n);
    let range = selection(e, n);
    let old_x = e.preferred_x;
    let current_line = g.caret_on(c, e.visual_line).line;
    let mut visual_line = None;
    e.cursor = match k {
        UiKey::SelectAll => n,
        UiKey::Left if !modifiers.shift && !range.is_empty() => range.start,
        UiKey::Right if !modifiers.shift && !range.is_empty() => range.end,
        UiKey::Left => c.saturating_sub(1),
        UiKey::Right => (c + 1).min(n),
        UiKey::Home if multiline && !modifiers.control => {
            visual_line = Some(current_line);
            g.line_end(c, false, e.visual_line)
        }
        UiKey::End if multiline && !modifiers.control => {
            visual_line = Some(current_line);
            g.line_end(c, true, e.visual_line)
        }
        UiKey::Home => 0,
        UiKey::End => n,
        UiKey::Up | UiKey::Down if multiline => {
            let p = g.caret_on(c, e.visual_line);
            let x = old_x.unwrap_or(p.x);
            let last = g.layout.carets.last().map_or(0, |p| p.line);
            let line = if matches!(k, UiKey::Up) {
                p.line.saturating_sub(1)
            } else {
                (p.line + 1).min(last)
            };
            e.preferred_x = Some(x);
            visual_line = Some(line);
            g.on_line(line, x)
        }
        _ => c,
    };
    e.visual_line = visual_line;
    if matches!(k, UiKey::SelectAll) {
        e.selection_anchor = Some(0);
    } else if modifiers.shift {
        e.selection_anchor.get_or_insert(c);
    } else {
        e.selection_anchor = None;
    }
    if !matches!(k, UiKey::Up | UiKey::Down) {
        e.preferred_x = None;
    }
    u.caret_t0 = u.time_ms;
    u.caret_visible = true;
    u.tree.touch();
    geometry::ensure_caret(&mut u, id);
}

pub(crate) fn methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    let def = |name: &str, f: mlua::Function| m.raw_set(name, f);
    def(
        "SetText",
        method(lua, |lua, id, a| {
            let text = a.s(1).unwrap_or_default();
            let same = ui(lua).tree.get(id).text().is_some_and(|t| t.text == text);
            // Native EditBox39292f70 compares before mutation and dispatches event6 afterward.
            if !same && set_text(lua, id, text) {
                call_handler(lua, id, "OnTextChanged", vec![]);
            }
            none()
        })?,
    )?;
    def(
        "GetText",
        method(lua, |lua, id, _| {
            let (text, init_val) = {
                let u = ui(lua);
                let w = u.tree.widget(id);
                let text = w
                    .and_then(|w| w.text.as_ref())
                    .map(|t| t.text.clone())
                    .unwrap_or_default();
                let init_val = w
                    .and_then(|w| w.edit.as_ref())
                    .filter(|e| e.digit && !e.password && text.is_empty())
                    .map(|e| e.init_val);
                (text, init_val)
            };
            // Native 392930c0: numeric fallback is a getter side effect, not a SetText rule.
            // It writes the default back into the buffer without dispatching OnTextChanged.
            let text = if let Some(value) = init_val {
                let text = value.to_string();
                set_text(lua, id, text.clone());
                text
            } else {
                text
            };
            ret(lua, text)
        })?,
    )?;
    def(
        "SetInitVal",
        method(lua, |lua, id, a| {
            let value = a.s(1).and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0) as i32;
            with(lua, id, |e| e.init_val = value);
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
        method(lua, |lua, id, _| {
            ret(lua, with(lua, id, |e| e.max_len as i64).unwrap_or(0))
        })?,
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
        method(lua, |lua, id, _| {
            ret(lua, with(lua, id, |e| e.password).unwrap_or(false))
        })?,
    )?;
    def(
        "SetDigit",
        method(lua, |lua, id, a| {
            let on = a.b(1, true);
            with(lua, id, |e| {
                e.digit = on;
                if on {
                    e.clipboard_disabled = true;
                }
            });
            none()
        })?,
    )?;
    def(
        "IsDigit",
        method(lua, |lua, id, _| {
            ret(lua, with(lua, id, |e| e.digit).unwrap_or(false))
        })?,
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
    def(
        "UseSelectAllWhenFocused",
        method(lua, |lua, id, a| {
            with(lua, id, |e| e.select_all_on_focus = a.b(1, true));
            none()
        })?,
    )?;
    def(
        "UseClipboard",
        method(lua, |lua, id, a| {
            with(lua, id, |e| e.clipboard_disabled = !a.b(1, true));
            none()
        })?,
    )?;
    def("SetEnglish", method(lua, |_, _, _| none())?)?;
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
