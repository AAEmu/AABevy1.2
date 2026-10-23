//! Chat: `message` widgets (scrolling message frames) and `chatwindow` widgets (tabbed chat).
//!
//! A message frame keeps lines (`AddMessage(text)`, colour codes `|cAARRGGBB` allowed) and shows
//! the newest ones bottom-aligned in its text style, wrapped at its width minus `SetInset`,
//! oldest lines dropping off the top; `SetTimeVisible(s)` hides lines older than `s` seconds.
//!
//! A chat window builds its tabs natively after `SetChatWindowId(id)` like the client, calling
//! the script's global callbacks: `OnCreateChatWindow(window)` when created,
//! `OnCreateChatMessage(message)`, `OnCreateChatTabButton(button)` and
//! `OnSetChatTabId(button, tabId)` per hydrated tab, then
//! `OnSelectedChatTab(button, selected)` on selection. A live append selects before ID binding.
//! The tabs come from the app: `AAUI.ChatTabs(windowId)` returns
//! `{ { id = n, name = "All", filters = { CMF_SAY, ... } }, ... }` (no `filters` = every
//! message; no hook = one "All" tab). `X2Chat:DispatchChatMessage(filter, text)` (toolkit) adds
//! a line to every tab that takes `filter`, coloured by `AAUI.ChatColor(filter)` (`{r, g, b}`)
//! or a default per filter. Configured per-tab colours override that fallback and recolour
//! retained history. The public Rust lifecycle helpers let the app update its local configuration
//! while preserving existing widget objects, handlers and scrollback.
//!
//! # Scrolling
//!
//! The chat scripts hook no wheel or scroll handler: the message frame scrolls natively. The
//! mouse wheel over a frame (`input::update` -> `wheel`, 3 messages per notch; scripts' own
//! `OnWheelUp` / `OnWheelDown` handlers on the widgets under the cursor win) scrolls back through
//! its messages (`MessageData::scroll` counts messages from the bottom, not wrapped lines, and is
//! clamped so the oldest message can reach the top of the frame); while scrolled back the time limit
//! (`SetTimeVisible`) does not hide old messages. A message arriving while scrolled back moves
//! `scroll` along, so the view does not jump (the down-to-bottom button's flicker, started by
//! `OnContentUpdated("unread_chat_message")`, tells there is more below; scrolling to the bottom
//! raises `"read_chat_message"`). While the cursor is over the selected tab's frame its native
//! controls show: the up / down buttons (one message), the slider (thumb at the top = the oldest
//! messages; dragging it scrolls) and, scrolled back, the down-to-bottom button.

use mlua::{Function, Lua, Table, Value};

use super::widget::create_child;
use super::{call_handler, method, new_object_table, none, object_table, ret, ui, value_id};
use crate::tree::{Align, Anchor, DrawKind, DrawLayer, Id, Point, Rect, TextBlock};

mod lifecycle;
mod transfer;
mod interaction;
pub(crate) mod alpha;
pub use alpha::set_alpha_options;
pub use lifecycle::{
    ChatTabInfo, ChatWindowInfo, append_tab, hydrate_window, remove_tab, reset_windows, update_tab,
};

#[derive(Clone, Debug, PartialEq)]
pub struct MessageData {
    /// (text, time added in UI ms).
    pub lines: Vec<(String, f64)>,
    /// Source filter and fallback colour for dispatched lines. Ordinary AddMessage has none.
    pub(crate) sources: Vec<Option<(i64, [f32; 4])>>,
    pub(crate) filter_colors: Vec<(i64, [f32; 4])>,
    pub(crate) background: Option<[f32; 4]>,
    pub max_lines: usize,
    /// Seconds a line stays visible (`SetTimeVisible`); `None` = forever.
    pub time_visible: Option<f32>,
    /// Lines scrolled up from the bottom.
    pub scroll: usize,
    /// Lines of the live view still within `time_visible` at the last frame: when one times
    /// out this changes, and the change redraws the frame.
    pub live: usize,
    /// Original script-message native controls: up, down, bottom and slider.
    /// Kept apart from script fields/getter replacement, like the native pointers.
    controls: Option<[Id; 4]>,
}

impl Default for MessageData {
    fn default() -> Self {
        MessageData {
            lines: Vec::new(),
            sources: Vec::new(),
            filter_colors: Vec::new(),
            background: None,
            max_lines: 100,
            time_visible: None,
            scroll: 0,
            live: 0,
            controls: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChatTab {
    pub id: i64,
    pub button: Id,
    pub message: Id,
    /// `None` = every filter.
    pub filters: Option<Vec<i64>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChatData {
    pub(crate) interaction: interaction::Interaction,
    /// Native +358/+35c: stored/copied only; inherited draw preparation does not read it.
    tab_button_alpha: [f32; 2],
    /// Native active-window flag, independent of edit keyboard focus.
    pub(crate) active: bool,
    pub(crate) links: Vec<super::chat_input::InputLink>,
    pub window_id: i64,
    pub tabs: Vec<ChatTab>,
    /// Zero-based numeric selection; usize::MAX is native -1 (no selected row).
    pub selected: usize,
    pub tab_width: f32,
    pub min_tab_width: f32,
    pub auto_resize_tabs: bool,
    pub tab_offset: f32,
    pub tab_gap: f32,
    pub tab_area_height: f32,
    /// `SetTabAreaInset(left, top, right, bottom)`.
    pub tab_inset: [f32; 4],
    /// Signed top-left/bottom-right offsets for message frames, independent of tabs.
    pub content_offset: [f32; 4],
    pub add_button: Id,
    pub use_add_button: bool,
    pub edit: Id,
    /// The chat method selector (`GetChatMethodSelector`), a child of the edit.
    pub selector: Id,
    /// The input's channel (`CHAT_*`) and whisper target ([`super::chat_input`]).
    pub method: i64,
    pub whisper: String,
    /// Native input +e70: last selected raw ID, independent of row-index clamping.
    pub(crate) input_tab_id: i64,
    pub left_line: Id,
    pub right_line: Id,
    pub lock_notify: Id,
    /// Native drag policy; ordinary tab selection and context remain available.
    pub allow_tab_switch: bool,
    pub notify_remaining_ms: f32,
    pub notify_blink_ms: f32,
    pub notify_max_ms: f32,
    pub line_offset: [f32; 2],
    /// Local top/bottom anchor offsets of the native internal clipping area.
    tab_clip_vertical: [f32; 2],
    /// Cursor over the window at the last update.
    pub hovered: Option<bool>,
    /// Last resolved local extent passed through the native geometry callback.
    layout_extent: [f32; 2],
    /// Only configured windows participate in native empty-window retirement.
    hydrated: bool,
}

fn with_msg<R>(lua: &Lua, id: Id, f: impl FnOnce(&mut MessageData) -> R) -> Option<R> {
    let mut u = ui(lua);
    let w = u.tree.widget_mut(id)?;
    Some(f(w.message.get_or_insert_with(MessageData::default)))
}

fn chat(lua: &Lua, id: Id) -> Option<ChatData> {
    ui(lua).tree.widget(id)?.chat.clone()
}

/// Apply native AllowTabSwitch. Enabling also cancels/hides the lock notification.
/// This does not change resizing, selection, or an already captured move.
pub fn set_allow_tab_switch(lua: &Lua, id: Id, allowed: bool) -> bool {
    let mut u = ui(lua);
    let Some(c) = u.tree.widget_mut(id).and_then(|w| w.chat.as_mut()) else {
        return false;
    };
    c.allow_tab_switch = allowed;
    let notify = c.lock_notify;
    if allowed {
        c.notify_remaining_ms = 0.0;
        u.tree.get_mut(notify).shown = false;
    }
    true
}

/// Native tab construction registers LeftButton dragging independently of Lua.
pub(crate) fn tab_drag_source(u: &super::Ui, id: Id) -> bool {
    tab_owner(u, id).is_some()
}

fn tab_owner(u: &super::Ui, id: Id) -> Option<(Id, usize)> {
    let window = u.tree.get(id).parent?;
    let c = u.tree.widget(window)?.chat.as_ref()?;
    let index = c.tabs.iter().position(|tab| tab.button == id)?;
    Some((window, index))
}

/// ButtonWidget::OnDragStart delivers Lua first, then internal event 4.
pub(crate) fn drag_started(lua: &Lua, button: Id) {
    let window = {
        let u = ui(lua);
        tab_owner(&u, button)
    };
    let Some((window, index)) = window else { return };
    let allowed = {
        let mut u = ui(lua);
        let c = u.tree.widget_mut(window).unwrap().chat.as_mut().unwrap();
        if !c.allow_tab_switch {
            c.notify_remaining_ms = c.notify_max_ms;
        }
        c.allow_tab_switch
    };
    if allowed && index == 0 {
        // Native starts the entire window before checking raw-tab-zero split protection.
        super::drag::start_moving(lua, window);
        interaction::begin(lua, button, window);
    } else if allowed {
        transfer::split(lua, window, button);
    }
}

/// Native button internal event 8 follows the source's Lua OnDragStop.
pub(crate) fn drag_stopped(lua: &Lua, button: Id) {
    interaction::finish(lua, button);
    transfer::drop_window(lua, button);
}

/// Native receiving button dispatches Lua event22, then chat internal event16,
/// before the GUI's mouse-up and source drag-stop continuations.
pub(crate) fn drag_receive(lua: &Lua, source: Id, target: Id) {
    let eligible = {
        let u = ui(lua);
        tab_owner(&u, source).is_some() && tab_owner(&u, target).is_some()
            && u.tree.widget(target).is_some_and(|w| w.enabled)
    };
    if !eligible || super::widget::button_ignores_fading_input(lua, target) { return; }
    call_handler(lua, target, "OnDragReceive", vec![Value::String(lua.create_string("LeftButton").unwrap())]);
    transfer::receive(lua, source, target);
}

fn notify_tick(u: &mut super::Ui, id: Id, dt_ms: f32) {
    let Some(c) = u.tree.widget_mut(id).and_then(|w| w.chat.as_mut()) else { return };
    if c.notify_remaining_ms > 0.0 {
        c.notify_remaining_ms -= dt_ms;
        let elapsed = c.notify_max_ms - c.notify_remaining_ms;
        let phase = elapsed / c.notify_blink_ms;
        // CVTTSS2SI returns integer-indefinite for NaN/overflow; its signed %2 is zero.
        let phase = if phase >= -2147483648.0 && phase < 2147483648.0 {
            phase as i32
        } else { i32::MIN };
        let shown = phase % 2 == 1 && c.notify_remaining_ms > 0.0;
        let notify = c.lock_notify;
        if u.tree.get(notify).shown != shown {
            u.tree.get_mut(notify).shown = shown;
        }
    }
}

fn global_fn(lua: &Lua, name: &str) -> Option<Function> {
    lua.globals()
        .raw_get::<Option<Function>>(name)
        .ok()
        .flatten()
}

fn call_global(lua: &Lua, name: &str, args: impl mlua::IntoLuaMulti) {
    if let Some(f) = global_fn(lua, name) {
        if let Err(e) = f.call::<()>(args) {
            bevy::log::warn!("aa-ui: {name}: {e}");
        }
        super::flush(lua);
    }
}

/// Encode chat text after input-link substitution and before sending it on the wire.
/// Native 395115b8 preserves item/quest markup while escaping other bars and literal @;
/// network send 393c5be2 then normalizes CRLF and remaining CR/LF to spaces.
/// `AAUI.ChatSend` receives raw text; its application adapter calls this exactly once.
pub fn encode_for_transport(text: &str) -> String {
    text.replace('|', "||")
        .replace("||i", "|i")
        .replace("||q", "|q")
        .replace('@', "@@")
        .replace("\r\n", " ")
        .replace(['\r', '\n'], " ")
}

/// Add a line to a message frame.
pub(crate) fn add_line(lua: &Lua, id: Id, text: &str) {
    // Native AddMessage 392ba8fc calls formatter 39435050, whose escaped-@ branch
    // 39435156..39435181 decodes once before storing the line. Bars remain escaped
    // through link expansion and are consumed by glyph layout instead.
    let decoded;
    let text = if text.contains("@@") {
        decoded = text.replace("@@", "@");
        &decoded
    } else {
        text
    };
    resolve_links(lua, text);
    let mut u = ui(lua);
    let now = u.time_ms;
    if let Some(w) = u.tree.widget_mut(id) {
        let m = w.message.get_or_insert_with(MessageData::default);
        m.lines.push((text.to_string(), now));
        m.sources.resize(m.lines.len() - 1, None);
        m.sources.push(None);
        // Scrolled back: the view stays on the same messages.
        if m.scroll > 0 {
            m.scroll += 1;
        }
        let max = m.max_lines.max(1);
        if m.lines.len() > max {
            let n = m.lines.len() - max;
            m.lines.drain(..n);
            m.sources.drain(..n.min(m.sources.len()));
            m.scroll = m.scroll.min(m.lines.len().saturating_sub(1));
        }
    }
    u.tree.touch();
}

/// Native link names come from the game's item/quest databases. Resolve once when messages
/// arrive, outside the Ui borrow; font layout/rendering only reads the resulting plain data.
fn resolve_links(lua: &Lua, text: &str) {
    use crate::text::{ChatLink, LinkLabel};
    let Some(hook) = lua
        .globals()
        .raw_get::<Table>("AAUI")
        .ok()
        .and_then(|a| a.raw_get::<Option<Function>>("ChatLink").ok().flatten())
    else {
        return;
    };
    let mut seen = std::collections::HashSet::new();
    for link in crate::text::chat_link_references(text) {
        let Some(key) = link.key().filter(|key| seen.insert(*key)) else {
            continue;
        };
        let (kind, grade) = match &link {
            ChatLink::Item { grade, .. } => ("item", Some(*grade)),
            ChatLink::Quest { .. } => ("quest", None),
            _ => continue,
        };
        let label = match hook.call::<Option<Table>>((kind, key.1, grade)) {
            Ok(Some(t)) => (|| -> mlua::Result<Option<LinkLabel>> {
                let name = t.raw_get::<Option<String>>("name")?.unwrap_or_default();
                if name.is_empty() || name.len() > 4096 {
                    return Ok(None);
                }
                let mut color = [1.0; 4];
                if let Some(c) = t.raw_get::<Option<Table>>("color")? {
                    for (i, v) in color.iter_mut().enumerate() {
                        *v = c.raw_get::<Option<f32>>(i + 1)?.unwrap_or(1.0);
                    }
                }
                Ok(color
                    .iter()
                    .all(|v| v.is_finite())
                    .then_some(LinkLabel { name, color }))
            })()
            .ok()
            .flatten(),
            Ok(None) => None,
            Err(e) => {
                bevy::log::warn!("aa-ui: AAUI.ChatLink: {e}");
                None
            }
        };
        ui(lua).fonts.set_link_label(&link, label);
    }
}

/// The text block a message frame shows now: its newest lines that fit, bottom-aligned.
pub(crate) fn compose(u: &mut super::Ui, id: Id, rect: Rect) -> Option<TextBlock> {
    let w = u.tree.widget(id)?;
    let m = w.message.as_ref()?;
    let base = w.text.clone().unwrap_or_default();
    let now = u.time_ms;
    let scale = u.ui_scale;
    let inner_w = (rect.w - base.inset[0] - base.inset[2]).max(1.0);
    let inner_h = (rect.h - base.inset[1] - base.inset[3]).max(0.0);
    let live: Vec<String> = m
        .lines
        .iter()
        .enumerate()
        // Scrolled back, the whole history is there (the time limit only fades the live view).
        .filter(|(_, (_, t))| m.scroll > 0 || m.time_visible.is_none_or(|s| now - t < s as f64 * 1000.0))
        .map(|(i, (text, _))| {
            let Some((filter, fallback)) = m.sources.get(i).copied().flatten() else {
                return text.clone();
            };
            let color = m.filter_colors.iter().find(|(key, _)| *key == filter)
                .map_or(fallback, |(_, color)| *color);
            // Dispatch always contributes exactly one base colour. The original inline
            // markup remains after it and can override/reset that colour as before.
            colored(text.get(10..).unwrap_or(text), color)
        })
        .collect();
    let end = live.len().saturating_sub(m.scroll).max(live.len().min(1));
    let mut shown: Vec<&str> = Vec::new();
    let size = base.style.size * scale;
    for line in live[..end].iter().rev() {
        let mut test = vec![line.as_str()];
        test.extend(shown.iter().copied());
        let text = test.join("\n");
        let lo = u.fonts.layout(
            &base.style.font,
            size,
            &text,
            Some(inner_w * scale),
            base.line_space * scale,
            false,
        );
        if lo.height > inner_h * scale + 0.5 && !shown.is_empty() {
            break;
        }
        shown.insert(0, line);
    }
    let mut t = base;
    t.text = shown.join("\n");
    t.wrap = true;
    t.style.align = match t.style.align.fx() {
        0.0 => Align::BottomLeft,
        1.0 => Align::BottomRight,
        _ => Align::Bottom,
    };
    Some(t)
}

/// Messages one wheel notch scrolls.
const WHEEL_LINES: i64 = 3;

/// How many of a message frame's messages, from the oldest, fit its height: the oldest one is in
/// view once the frame is scrolled back to `lines - fit`.
fn fit_top(u: &mut super::Ui, id: Id) -> usize {
    let rect = u.tree.rect(id);
    let Some(w) = u.tree.widget(id) else { return 0 };
    let Some(m) = w.message.as_ref() else {
        return 0;
    };
    let base = w.text.clone().unwrap_or_default();
    let scale = u.ui_scale;
    let inner_w = (rect.w - base.inset[0] - base.inset[2]).max(1.0);
    let inner_h = (rect.h - base.inset[1] - base.inset[3]).max(0.0);
    let size = base.style.size * scale;
    let (mut n, mut text) = (0usize, String::new());
    for (line, _) in &m.lines {
        let test = if n == 0 {
            line.clone()
        } else {
            format!("{text}\n{line}")
        };
        let lo = u.fonts.layout(
            &base.style.font,
            size,
            &test,
            Some(inner_w * scale),
            base.line_space * scale,
            false,
        );
        if lo.height > inner_h * scale + 0.5 && n > 0 {
            break;
        }
        text = test;
        n += 1;
    }
    n
}

/// The most a message frame can be scrolled back.
fn max_scroll(u: &mut super::Ui, id: Id) -> usize {
    let fit = fit_top(u, id);
    let len = u
        .tree
        .widget(id)
        .and_then(|w| w.message.as_ref())
        .map_or(0, |m| m.lines.len());
    len.saturating_sub(fit)
}

/// The native slider of a message frame (`GetSlider()`), if it has one.
fn frame_slider(lua: &Lua, msg: Id) -> Option<Id> {
    let t = parts(lua, msg)
        .ok()?
        .raw_get::<Option<Table>>("slider")
        .ok()??;
    value_id(lua, &Value::Table(t))
}

/// Move the slider to where the scroll is: the thumb at the top is the oldest messages.
fn sync_slider(lua: &Lua, msg: Id) {
    let Some(slider) = frame_slider(lua, msg) else {
        return;
    };
    let mut g = ui(lua);
    let u = &mut *g;
    let max = max_scroll(u, msg);
    let scroll = u
        .tree
        .widget(msg)
        .and_then(|w| w.message.as_ref())
        .map_or(0, |m| m.scroll.min(max));
    let want = (0.0, max as f32, (max - scroll) as f32);
    let cur = u
        .tree
        .widget(slider)
        .and_then(|w| w.slider.as_ref())
        .map(|s| (s.min, s.max, s.value));
    if cur != Some(want)
        && let Some(s) = u.tree.widget_mut(slider).and_then(|w| w.slider.as_mut())
    {
        (s.min, s.max, s.value) = want;
        s.step = 1.0;
    }
}

/// Scroll a message frame back to `to` messages from the bottom (clamped): the slider follows,
/// and reaching the bottom marks the frame read (`OnContentUpdated(self, "read_chat_message")`).
pub(crate) fn set_scroll(lua: &Lua, id: Id, to: i64) {
    let (changed, now) = {
        let mut g = ui(lua);
        let u = &mut *g;
        let max = max_scroll(u, id) as i64;
        let to = to.clamp(0, max) as usize;
        let cur = u
            .tree
            .widget(id)
            .and_then(|w| w.message.as_ref())
            .map_or(0, |m| m.scroll);
        if to != cur
            && let Some(m) = u.tree.widget_mut(id).and_then(|w| w.message.as_mut())
        {
            m.scroll = to;
        }
        (to != cur, to)
    };
    if !changed {
        return;
    }
    sync_slider(lua, id);
    if now == 0
        && let Ok(s) = lua.create_string("read_chat_message")
    {
        call_handler(lua, id, "OnContentUpdated", vec![Value::String(s)]);
    }
}

/// Scroll a message frame back (`delta` > 0) or forward by `delta` messages.
pub(crate) fn scroll_by(lua: &Lua, id: Id, delta: i64) {
    let cur = with_msg(lua, id, |m| m.scroll).unwrap_or(0) as i64;
    set_scroll(lua, id, cur + delta);
}

/// The mouse wheel at `cursor` (UI units), no script handler took it: scroll the topmost message
/// frame under the cursor, unless another window is in the way. `wheel` > 0 scrolls back.
pub(crate) fn wheel(lua: &Lua, (x, y): (f32, f32), hover: Option<Id>, wheel: f32) -> bool {
    let target = {
        let u = ui(lua);
        let hover_top = hover.map(|h| u.tree.top_of(h));
        u.tree.paint_order().into_iter().rev().find(|&id| {
            u.tree.widget(id).is_some_and(|w| w.message.is_some())
                && u.tree.rect(id).contains(x, y)
                && hover_top.is_none_or(|t| t == u.tree.top_of(id))
        })
    };
    let Some(msg) = target else { return false };
    let notches = wheel.abs().round().clamp(1.0, 5.0) as i64;
    let sign = if wheel > 0.0 { 1 } else { -1 };
    scroll_by(lua, msg, sign * notches * WHEEL_LINES);
    true
}

/// A drag on a slider moved it: when it is a message frame's native slider, scroll to it.
pub(crate) fn slider_changed(lua: &Lua, slider: Id) {
    let msg = {
        let u = ui(lua);
        match u.tree.get(slider).parent {
            Some(p) if u.tree.widget(p).is_some_and(|w| w.message.is_some()) => p,
            _ => return,
        }
    };
    if frame_slider(lua, msg) != Some(slider) {
        return;
    }
    let Some((max, value)) = ui(lua)
        .tree
        .widget(slider)
        .and_then(|w| w.slider.as_ref())
        .map(|s| (s.max, s.value))
    else {
        return;
    };
    set_scroll(lua, msg, (max - value).round() as i64);
}

/// A click on one of a message frame's native scroll buttons. Returns whether `target` was one.
fn scroll_button_clicked(lua: &Lua, target: Id) -> bool {
    let msg = {
        let u = ui(lua);
        match u.tree.get(target).parent {
            Some(p) if u.tree.widget(p).is_some_and(|w| w.message.is_some()) => p,
            _ => return false,
        }
    };
    let Ok(p) = parts(lua, msg) else { return false };
    let is = |field: &str| {
        p.raw_get::<Option<Table>>(field)
            .ok()
            .flatten()
            .and_then(|t| value_id(lua, &Value::Table(t)))
            == Some(target)
    };
    if is("upButton") {
        scroll_by(lua, msg, 1);
    } else if is("downButton") {
        scroll_by(lua, msg, -1);
    } else if is("downToBottomButton") {
        set_scroll(lua, msg, 0);
    } else {
        return false;
    }
    true
}

/// Per frame: the selected tab's native scroll controls show while the cursor is over its frame
/// (and there is something to scroll), and the slider keeps up with the scroll range.
fn scroll_controls(lua: &Lua, wins: &[Id]) {
    for &win in wins {
        let Some(c) = chat(lua, win) else { continue };
        for (i, tab) in c.tabs.iter().enumerate() {
            let msg = tab.message;
            let Ok(p) = parts(lua, msg) else { continue };
            let id_of = |field: &str| {
                p.raw_get::<Option<Table>>(field)
                    .ok()
                    .flatten()
                    .and_then(|t| value_id(lua, &Value::Table(t)))
            };
            let controls = [
                id_of("upButton"),
                id_of("downButton"),
                id_of("slider"),
                id_of("downToBottomButton"),
            ];
            let mut g = ui(lua);
            let u = &mut *g;
            let (cx, cy) = u.cursor;
            let inside = i == c.selected
                && u.tree.visible(msg)
                && u.tree.rect(msg).contains(cx, cy)
                && u.hover
                    .is_none_or(|h| u.tree.top_of(h) == u.tree.top_of(msg));
            let (max, scroll) = if inside {
                (
                    max_scroll(u, msg),
                    u.tree
                        .widget(msg)
                        .and_then(|w| w.message.as_ref())
                        .map_or(0, |m| m.scroll),
                )
            } else {
                (0, 0)
            };
            let want = [max > 0, max > 0, max > 0, scroll > 0];
            for (ctl, show) in controls.into_iter().zip(want) {
                if let Some(ctl) = ctl
                    && u.tree.get(ctl).shown != show
                {
                    u.tree.get_mut(ctl).shown = show;
                }
            }
            drop(g);
            if inside {
                // A message dropped off the top while scrolled back, or the frame was resized.
                if scroll > max {
                    set_scroll(lua, msg, max as i64);
                } else {
                    sync_slider(lua, msg);
                }
            }
        }
    }
}

/// Parts a native composite made (field -> object table), by owner id. Kept outside the object
/// tables so `pairs(widget)` still sees only script fields.
pub(crate) fn parts(lua: &Lua, owner: Id) -> mlua::Result<Table> {
    const KEY: &str = "aa_ui_native_parts";
    let all = match lua.named_registry_value::<Option<Table>>(KEY)? {
        Some(t) => t,
        None => {
            let t = lua.create_table()?;
            lua.set_named_registry_value(KEY, &t)?;
            t
        }
    };
    match all.raw_get::<Option<Table>>(owner)? {
        Some(t) => Ok(t),
        None => {
            let t = lua.create_table()?;
            all.raw_set(owner, &t)?;
            Ok(t)
        }
    }
}

/// A child widget made by the native (chat edit, scroll buttons ...), made once so `Get...()`
/// returns the same object.
fn native_child(
    lua: &Lua,
    owner: Id,
    field: &str,
    class: &str,
    name: &str,
    shown: bool,
) -> mlua::Result<(Id, Table)> {
    let t = parts(lua, owner)?;
    if let Some(c) = t.raw_get::<Option<Table>>(field)? {
        let cid = value_id(lua, &Value::Table(c.clone())).unwrap();
        return Ok((cid, c));
    }
    let c = create_child(lua, Some(owner), class, name, 0, false)?;
    let cid = value_id(lua, &Value::Table(c.clone())).unwrap();
    ui(lua).tree.get_mut(cid).shown = shown;
    t.raw_set(field, &c)?;
    Ok((cid, c))
}

fn native_drawable(lua: &Lua, owner: Id, field: &str) -> mlua::Result<(Id, Table)> {
    let t = parts(lua, owner)?;
    if let Some(c) = t.raw_get::<Option<Table>>(field)? {
        let cid = value_id(lua, &Value::Table(c.clone())).unwrap();
        return Ok((cid, c));
    }
    let d =
        ui(lua)
            .tree
            .create_drawable(owner, DrawKind::Image, "drawable:image", DrawLayer::Overlay);
    let c = new_object_table(lua, d)?;
    t.raw_set(field, &c)?;
    Ok((d, c))
}

/// Instance getters a native composite gives its parts (`chatMessage:GetUpButton()`): the
/// client's chat message class has methods the reference capture could not list, so they are
/// set on the object itself, which scripts cannot tell apart.
fn set_getter(lua: &Lua, t: &Table, name: &str, child: Table) -> mlua::Result<()> {
    let f = lua.create_function(move |_, _: mlua::MultiValue| Ok(child.clone()))?;
    t.raw_set(name, f)
}

/// The chat tab message frame: a `message` with scroll buttons and a slider.
fn create_tab_message(lua: &Lua, window: Id, i: usize) -> mlua::Result<(Id, Table)> {
    let t = create_child(
        lua,
        Some(window),
        "message",
        &format!("chatMessage{i}"),
        0,
        false,
    )?;
    let id = value_id(lua, &Value::Table(t.clone())).unwrap();
    let mut controls = [crate::tree::ROOT; 4];
    for (index, (getter, class, name)) in [
        ("GetUpButton", "button", "upButton"),
        ("GetDownButton", "button", "downButton"),
        ("GetDownToBottomButton", "button", "downToBottomButton"),
        ("GetSlider", "slider", "slider"),
    ].into_iter().enumerate() {
        // Scroll controls show on mouse over in the client; hidden here.
        let (cid, c) = native_child(lua, id, name, class, name, false)?;
        controls[index] = cid;
        if class == "slider" {
            // A native slider comes with its thumb (`GetThumbButtonWidget`).
            let (thumb, _) = native_child(lua, cid, "thumb", "button", "thumb", true)?;
            if let Some(s) = ui(lua).tree.widget_mut(cid).and_then(|w| w.slider.as_mut()) {
                s.thumb = Some(thumb);
            }
        }
        set_getter(lua, &t, getter, c)?;
    }
    with_msg(lua, id, |m| {
        m.max_lines = 300;
        m.controls = Some(controls);
    });
    alpha::scroll_pairs(lua, id);
    Ok((id, t))
}

/// Native virtual +118: 392ca3c0 (chat) / 39511d00 (script message).
/// Tabs/scroll buttons use mouse-over; the slider tests its rectangle even hidden.
pub(crate) fn resize_allowed(u: &super::Ui, id: Id, pointer: (f32, f32)) -> bool {
    let Some(w) = u.tree.widget(id) else { return false };
    if let Some(c) = &w.chat {
        if c.tabs.iter().any(|tab| u.entered.contains(&tab.button)) {
            return false;
        }
        return c.tabs.get(c.selected).is_none_or(|tab| {
            tab.message == crate::tree::ROOT || resize_allowed(u, tab.message, pointer)
        });
    }
    let Some([up, down, bottom, slider]) = w.message.as_ref().and_then(|m| m.controls) else {
        return true;
    };
    ![up, down, bottom].iter().any(|id| u.entered.contains(id))
        && (!u.tree.get(slider).alive
            || !u.animations.widget_rect(&u.tree, slider).contains(pointer.0, pointer.1))
}

/// Called when a chat window is created: its edit box, buttons and line drawables.
pub(crate) fn init_window(lua: &Lua, id: Id, t: &Table) -> mlua::Result<()> {
    init_window_from(lua, id, t, None)
}

fn init_window_from(lua: &Lua, id: Id, t: &Table, source: Option<Id>) -> mlua::Result<()> {
    let (edit, _) = native_child(lua, id, "chatEdit", "editbox", "chatEdit", false)?;
    let (add, _) = native_child(lua, id, "addButton", "button", "addButton", false)?;
    {
        let mut u = ui(lua);
        let o = u.tree.get_mut(add);
        o.width = 20.0;
        o.height = 20.0;
    }
    native_child(lua, id, "imeToggle", "button", "imeToggleButton", false)?;
    let (selector_id, selector) = native_child(
        lua,
        edit,
        "chatMethodSelector",
        "button",
        "chatMethodSelector",
        true,
    )?;
    let (_, list) = native_child(
        lua,
        edit,
        "chatMethodList",
        "listbox",
        "chatMethodList",
        false,
    )?;
    set_getter(lua, &selector, "GetList", list)?;
    let (left_line, _) = native_drawable(lua, id, "leftLine")?;
    let (right_line, _) = native_drawable(lua, id, "rightLine")?;
    // Native constructor 392cbb07/392cbb48: one-pixel dividers, initially hidden.
    for line in [left_line, right_line] {
        let mut u = ui(lua);
        let o = u.tree.get_mut(line);
        o.height = 1.0;
        o.shown = false;
    }
    let (lock_notify, _) = native_drawable(lua, id, "lockNotify")?;
    let (caret, _) = native_drawable(lua, id, "caret")?;
    let (slide_left, _) = native_child(lua, id, "slideLeft", "button", "tab", false)?;
    let (slide_right, _) = native_child(lua, id, "slideRight", "button", "tab", false)?;
    for (slide, text, point, rel) in [
        (slide_left, "<-", Point::Right, Point::Left),
        (slide_right, "->", Point::Left, Point::Right),
    ] {
        let mut u = ui(lua);
        let o = u.tree.get_mut(slide);
        o.width = 20.; o.height = 20.;
        o.anchors = vec![Anchor { point, target: id, rel, x: 0., y: 0. }];
        if let Some(t) = o.text_mut() { t.text = text.into(); }
    }
    ui(lua).tree.get_mut(lock_notify).shown = false;
    ui(lua).tree.get_mut(lock_notify).anchors = vec![Anchor {
        point: Point::BottomLeft, target: id, rel: Point::TopLeft, x: 0.0, y: 0.0,
    }];
    ui(lua).tree.get_mut(caret).shown = false;
    if let Some(w) = ui(lua).tree.widget_mut(id) {
        w.chat = Some(ChatData {
            interaction: interaction::Interaction { caret, slides: [slide_left, slide_right], ..Default::default() },
            tab_button_alpha: [1.0, 0.3],
            active: false,
            links: Vec::new(),
            window_id: -1,
            tabs: Vec::new(),
            selected: usize::MAX,
            tab_width: 50.0,
            min_tab_width: 0.0,
            auto_resize_tabs: false,
            tab_offset: 0.0,
            tab_gap: 0.0,
            tab_area_height: 20.0,
            tab_inset: [0.0; 4],
            content_offset: [0.0; 4],
            add_button: add,
            use_add_button: false,
            edit,
            selector: selector_id,
            method: 0,
            whisper: String::new(),
            input_tab_id: -1,
            left_line,
            right_line,
            lock_notify,
            allow_tab_switch: false,
            notify_remaining_ms: 0.0,
            notify_blink_ms: 500.0,
            notify_max_ms: 2000.0,
            line_offset: [0.0; 2],
            tab_clip_vertical: [3.0, 20.0],
            hovered: None,
            layout_extent: [0.0; 2],
            hydrated: false,
        });
    }
    if let Some(source) = source {
        transfer::copy_configuration(lua, source, id);
    }
    super::chat_input::activate(lua, id);
    alpha::configure(lua, id);
    call_global(lua, "OnCreateChatWindow", t.clone());
    super::chat_input::show_method(lua, id);
    Ok(())
}

fn nonnegative(value: f32) -> f32 {
    if value < 0.0 { 0.0 } else { value }
}

/// Tab buttons left to right in the tab area, the add button after them;
/// only the selected message frame is shown. Content anchors have their own callback.
fn layout(lua: &Lua, id: Id) {
    let Some(c) = chat(lua, id) else { return };
    let mut g = ui(lua);
    let u = &mut *g;
    let wr = u.tree.rect(id);
    if let Some(c) = u.tree.widget_mut(id).and_then(|w| w.chat.as_mut()) {
        c.layout_extent = [wr.w, wr.h];
    }
    // 392c97e0 returns before inspecting or changing child controls when empty.
    if c.tabs.is_empty() {
        return;
    }
    if let Some(c) = u.tree.widget_mut(id).and_then(|w| w.chat.as_mut()) {
        c.tab_clip_vertical = [0.0, c.tab_area_height];
    }
    let n = c.tabs.len() as f32;
    let add_w = if c.use_add_button {
        u.tree.get(c.add_button).width
    } else {
        0.0
    };
    let tw = if c.auto_resize_tabs {
        // Keep the SSE f32 addition order and minimum-first native clamp. In
        // particular min > max is legal; Rust's clamp would panic here.
        let mut used = c.tab_offset + add_w;
        used += (n - 1.0) * c.tab_gap;
        used += c.tab_inset[0];
        used += c.tab_inset[2];
        let proposed = ((wr.w - used) / n).floor();
        let proposed = if proposed == 0.0 { 0.0 } else { proposed };
        if c.min_tab_width > proposed {
            c.min_tab_width
        } else if proposed > c.tab_width {
            c.tab_width
        } else {
            proposed
        }
    } else {
        c.tab_width
    };
    let th = nonnegative(c.tab_area_height - c.tab_inset[1] - c.tab_inset[3]);
    let anchor = |point, x, y| Anchor {
        point,
        target: id,
        rel: point,
        x,
        y,
    };
    for (i, tab) in c.tabs.iter().enumerate() {
        let o = u.tree.get_mut(tab.button);
        o.anchors = vec![if i == 0 {
            anchor(Point::TopLeft, c.tab_offset + c.interaction.offset + c.tab_inset[0], c.tab_inset[1])
        } else {
            Anchor {
                point: Point::TopLeft,
                target: c.tabs[i - 1].button,
                rel: Point::TopRight,
                x: c.tab_gap,
                y: 0.0,
            }
        }];
        o.width = tw;
        o.height = th;
        o.shown = true;
        if let Some(b) = u
            .tree
            .widget_mut(tab.button)
            .and_then(|w| w.button.as_mut())
        {
            b.checked = i == c.selected;
        }
    }
    u.tree.remove_anchors(c.add_button);
    let o = u.tree.get_mut(c.add_button);
    if c.use_add_button {
        o.anchors = vec![Anchor {
            point: Point::BottomLeft,
            target: c.tabs.last().unwrap().button,
            rel: Point::BottomRight,
            x: c.tab_gap,
            y: 0.0,
        }];
    }
    o.shown = c.use_add_button;
    let first = u.tree.rect(c.tabs[0].button);
    let last = u.tree.rect(if c.use_add_button { c.add_button } else { c.tabs.last().unwrap().button });
    let overflow = last.right() - first.x + c.tab_offset > wr.w;
    let sliding = c.interaction.use_slides && overflow;
    for (i, slide) in c.interaction.slides.into_iter().enumerate() {
        let o = u.tree.get_mut(slide);
        o.shown = sliding;
        o.anchors = vec![Anchor {
            point: if i == 0 { Point::Right } else { Point::Left }, target: id,
            rel: if i == 0 { Point::TopLeft } else { Point::TopRight },
            x: 0., y: c.tab_area_height * 0.5,
        }];
    }
    if !sliding && c.interaction.offset != 0. {
        if let Some(c) = u.tree.widget_mut(id).and_then(|w| w.chat.as_mut()) { c.interaction.offset = 0.; }
        drop(g);
        layout(lua, id);
        return;
    }
    layout_lines(u, id, &c);
    u.tree.touch();
}

/// 392ca420 anchors each divider to the tab area and selected tab. The
/// drawables retain their own extents; both bottom anchors do not set height.
fn layout_lines(u: &mut super::Ui, id: Id, c: &ChatData) {
    u.tree.get_mut(c.left_line).shown = false;
    u.tree.get_mut(c.right_line).shown = false;
    for tab in &c.tabs {
        if tab.message != crate::tree::ROOT && u.tree.get(tab.message).alive {
            u.tree.get_mut(tab.message).shown = false;
        }
    }
    let Some(tab) = c.tabs.get(c.selected) else { return };
    let h = c.tab_area_height;
    u.tree.get_mut(c.left_line).anchors = vec![
        Anchor { point: Point::BottomLeft, target: id, rel: Point::TopLeft, x: c.line_offset[0], y: h },
        Anchor { point: Point::BottomRight, target: tab.button, rel: Point::BottomLeft, x: 0.0, y: 1.0 },
    ];
    u.tree.get_mut(c.right_line).anchors = vec![
        Anchor { point: Point::BottomRight, target: id, rel: Point::TopRight, x: c.line_offset[1], y: h + 1.0 },
        Anchor { point: Point::BottomLeft, target: tab.button, rel: Point::BottomRight, x: 0.0, y: 0.0 },
    ];
    u.tree.get_mut(c.left_line).shown = true;
    u.tree.get_mut(c.right_line).shown = true;
    if tab.message != crate::tree::ROOT && u.tree.get(tab.message).alive {
        u.tree.get_mut(tab.message).shown = true;
    }
}

/// Actual painter push 3921f480: no clamping of disjoint rectangles; compare
/// order also preserves the prior edge on equal values (including signed zero).
pub(crate) fn intersect_clip(next: [f32; 4], previous: [f32; 4]) -> [f32; 4] {
    std::array::from_fn(|i| if i < 2 {
        if next[i] > previous[i] { next[i] } else { previous[i] }
    } else if previous[i] > next[i] { next[i] } else { previous[i] })
}

/// Physical f32 clipping rectangle for native tab-area members and descendants.
/// Messages, edit, caret and lock notifier remain outside this internal area.
/// Keep public parent identity intact; this models the native render membership.
pub(crate) fn tab_clip(u: &super::Ui, mut id: Id) -> Option<[f32; 4]> {
    let mut result = None;
    while let Some(parent) = u.tree.get(id).parent {
        if let Some(c) = u.tree.widget(parent).and_then(|w| w.chat.as_ref())
            && (id == c.add_button || id == c.left_line || id == c.right_line
                || c.tabs.iter().any(|tab| tab.button == id))
        {
            let view = u.animations.widget_rect(&u.tree, parent);
            let scale = u.animations.widget_scale(&u.tree, parent);
            let x = view.x * u.ui_scale;
            let y = view.y * u.ui_scale;
            let bounds = [x, y + c.tab_clip_vertical[0] * scale,
                x + u.tree.rect(parent).w * scale, y + c.tab_clip_vertical[1] * scale];
            result = Some(result.map_or(bounds, |old| intersect_clip(old, bounds)));
        }
        id = parent;
    }
    result
}

/// Native 392ca610 runs on SetContentOffset and tab insertion, separately from
/// tab layout. In particular, later tab-area-height changes retain these anchors.
fn layout_content(lua: &Lua, id: Id) {
    let Some(c) = chat(lua, id) else { return };
    let [left, top, right, bottom] = c.content_offset;
    let top = c.tab_area_height + top;
    let mut u = ui(lua);
    for tab in c.tabs {
        if tab.message == crate::tree::ROOT || !u.tree.get(tab.message).alive { continue; }
        u.tree.get_mut(tab.message).anchors = vec![
            Anchor { point: Point::TopLeft, target: id, rel: Point::TopLeft, x: left, y: top },
            Anchor { point: Point::BottomRight, target: id, rel: Point::BottomRight, x: right, y: bottom },
        ];
    }
}

/// Native extent/anchor callbacks 392ca020/30/40 run tab layout before the
/// ordinary widget geometry callback. Anchor-derived sizes are settled per frame.
pub(crate) fn geometry(lua: &Lua, id: Id) {
    let changed = {
        let u = ui(lua);
        u.tree.widget(id).and_then(|w| w.chat.as_ref()).is_some_and(|c| {
            let r = u.tree.rect(id);
            c.layout_extent != [r.w, r.h]
        })
    };
    if changed {
        layout(lua, id);
    }
}

fn select(lua: &Lua, id: Id, idx: usize) {
    select_inner(lua, id, idx, false);
}

fn select_inner(lua: &Lua, id: Id, idx: usize, force: bool) {
    let Some(c) = chat(lua, id) else { return };
    if idx >= c.tabs.len() || (!force && c.selected == idx) {
        return;
    }
    if let Some(w) = ui(lua).tree.widget_mut(id)
        && let Some(ch) = w.chat.as_mut()
    {
        ch.selected = idx;
        ch.hovered = None;
    }
    layout(lua, id);
    lifecycle::tint_dividers(lua, id, c.tabs[idx].message);
    // Native3951394b..67 binds the input raw ID before selected-index persistence.
    if let Some(ch) = ui(lua).tree.widget_mut(id).and_then(|w| w.chat.as_mut()) {
        ch.input_tab_id = c.tabs[idx].id;
    }
    // Native395139b4..bb stores/saves the numeric index before OnSelectedChatTab.
    // This app-only hook preserves the application's other window-extra fields.
    let hook = lua.globals().raw_get::<Option<Table>>("AAUI").ok().flatten()
        .and_then(|t| t.raw_get::<Option<Function>>("ChatSelection").ok().flatten());
    if let Some(hook) = hook {
        if let Err(error) = hook.call::<()>((c.window_id, idx as i32)) {
            bevy::log::warn!("aa-ui: AAUI.ChatSelection: {error}");
        }
        super::flush(lua);
    }
    for (i, tab) in c.tabs.iter().enumerate() {
        if let Ok(Some(b)) = object_table(lua, tab.button) {
            call_global(lua, "OnSelectedChatTab", (b, i == idx));
        }
    }
}

/// A click on a chat tab button selects its tab. Returns whether `target` was one.
pub(crate) fn button_clicked(lua: &Lua, target: Id) -> bool {
    if scroll_button_clicked(lua, target) {
        return true;
    }
    let Some(parent) = ui(lua).tree.get(target).parent else {
        return false;
    };
    let Some(c) = chat(lua, parent) else {
        return false;
    };
    if let Some(side) = c.interaction.slides.iter().position(|&b| b == target) {
        interaction::slide(lua, parent, side == 1);
        return true;
    }
    match c.tabs.iter().position(|t| t.button == target) {
        Some(i) => {
            select(lua, parent, i);
            true
        }
        None => false,
    }
}

/// Native392caf30 -> 39513750: context on an exact tab button never selects it.
pub(crate) fn context_clicked(lua: &Lua, target: Id) -> bool {
    let Some(parent) = ui(lua).tree.get(target).parent else { return false };
    let Some(c) = chat(lua, parent) else { return false };
    let Some(tab) = c.tabs.iter().find(|tab| tab.button == target) else { return false };
    let (Ok(Some(message)), Ok(Some(button))) =
        (object_table(lua, tab.message), object_table(lua, tab.button)) else { return true };
    if ui(lua).tree.widget(tab.message).is_none_or(|w| w.message.is_none()) {
        return true;
    }
    let raw_id = |id: i64| (id as i32 as f32) as f64;
    super::fire_ui_event(lua, "SHOW_CHAT_TAB_CONTEXT", vec![
        Value::Table(button.clone()), Value::Number(raw_id(tab.id)),
    ]);
    // Native reads the message's ID again after event listeners have run.
    let current_id = chat(lua, parent).and_then(|c| c.tabs.into_iter()
        .find(|t| t.message == tab.message).map(|t| t.id)).unwrap_or(tab.id);
    call_global(lua, "OnContextChatTab", (message, button, raw_id(current_id)));
    true
}

fn set_window_id(lua: &Lua, id: Id, window_id: i64) -> mlua::Result<()> {
    if chat(lua, id).is_some_and(|c| c.window_id == window_id) {
        return Ok(());
    }
    let info = lifecycle::info_from_hook(lua, window_id)?;
    hydrate_window(lua, id, &info)?;
    Ok(())
}

/// Native base/new-tab metadata: font15 and opaque black; global dynamic alpha is separate.
fn apply_chat_options(lua: &Lua, msg: Id) {
    let mut g = ui(lua);
    let u = &mut *g;
    if let Some(t) = u.tree.get_mut(msg).text_mut() {
        t.style.size = 15.0;
    }
    let ds = u
        .tree
        .widget(msg)
        .map(|w| w.drawables.clone())
        .unwrap_or_default();
    for d in ds {
        if let Some(dr) = u.tree.drawable_mut(d)
            && dr.layer == DrawLayer::Background
        {
            dr.color = [0.0, 0.0, 0.0, 1.0];
        }
    }
}

/// Native chat frame state, timed messages, lock notification and drag preview.
pub(crate) fn update(lua: &Lua, dt_ms: f32) {
    let windows = ui(lua).tree.of_class("chatwindow");
    for id in windows {
        if chat(lua, id).is_some_and(|c| c.hydrated && c.tabs.is_empty()) {
            super::widget::remove_subtree(lua, id);
            continue;
        }
        geometry(lua, id);
    }
    interaction::update(lua, dt_ms);
    let mut g = ui(lua);
    let u = &mut *g;
    // Lines that time out (`SetTimeVisible`) leave the live view without a call.
    let now = u.time_ms;
    for id in u.tree.of_class("message") {
        let Some(m) = u.tree.widget(id).and_then(|w| w.message.as_ref()) else {
            continue;
        };
        let Some(s) = m.time_visible.filter(|_| m.scroll == 0) else {
            continue;
        };
        let live = m
            .lines
            .iter()
            .filter(|(_, t)| now - t < s as f64 * 1000.0)
            .count();
        if live != m.live
            && let Some(m) = u.tree.widget_mut(id).and_then(|w| w.message.as_mut())
        {
            m.live = live;
        }
    }
    let (cx, cy) = u.cursor;
    let wins: Vec<Id> = u
        .tree
        .of_class("chatwindow")
        .into_iter()
        .filter(|&i| u.tree.widget(i).is_some_and(|w| w.chat.is_some()))
        .collect();
    for &id in &wins {
        notify_tick(u, id, dt_ms);
        let inside = u.tree.visible(id) && u.tree.rect(id).contains(cx, cy);
        // Native395130e0 applies dynamic state every frame, including newly attached children.
        alpha::state(u, id, inside);
        if let Some(ch) = u.tree.widget_mut(id).and_then(|w| w.chat.as_mut()) {
            ch.hovered = Some(inside);
        }
    }
    drop(g);
    scroll_controls(lua, &wins);
}

/// Default line colours by chat filter (`CMF_*`), when the app gives none.
fn default_color(filter: i64) -> [f32; 3] {
    match filter {
        2 => [1.0, 1.0, 1.0],   // CMF_SAY
        3 => [1.0, 0.55, 0.85], // CMF_WHISPER
        4 => [0.45, 0.78, 1.0], // CMF_PARTY
        5 => [1.0, 0.55, 0.2],  // CMF_RAID
        6 => [0.55, 0.9, 0.45], // CMF_EXPEDITION
        _ => [1.0, 246.0 / 255.0, 94.0 / 255.0],
    }
}

/// `X2Chat:DispatchChatMessage(filter, text)`: a line on every chat tab that takes `filter`.
pub(crate) fn dispatch(lua: &Lua, filter: i64, text: &str) -> mlua::Result<()> {
    let hook = lua
        .globals()
        .raw_get::<Option<Table>>("AAUI")?
        .and_then(|t| t.raw_get::<Option<Function>>("ChatColor").ok().flatten());
    let rgb = match hook
        .map(|f| f.call::<Option<Table>>(filter))
        .transpose()?
        .flatten()
    {
        Some(t) => [
            t.raw_get::<f32>(1).unwrap_or(1.0),
            t.raw_get::<f32>(2).unwrap_or(1.0),
            t.raw_get::<f32>(3).unwrap_or(1.0),
        ],
        None => default_color(filter),
    };
    let color = [rgb[0], rgb[1], rgb[2], 1.0];
    let line = colored(text, color);
    let frames: Vec<Id> = {
        let u = ui(lua);
        u.tree
            .objects
            .iter()
            .filter(|o| o.alive)
            .filter_map(|o| o.widget()?.chat.as_ref())
            .flat_map(|c| c.tabs.iter())
            // Native 39511860 uses RAID's enable bit for filter56, but colour56.
            .filter(|t| t.filters.as_ref().is_none_or(|f| f.contains(&if filter == 56 { 5 } else { filter })))
            .map(|t| t.message)
            .collect()
    };
    for m in frames {
        add_line(lua, m, &line);
        with_msg(lua, m, |data| {
            if let Some(source) = data.sources.last_mut() {
                *source = Some((filter, color));
            }
        });
        // Scrolled back, the new line is below the view: the down-to-bottom button flickers.
        if with_msg(lua, m, |x| x.scroll).unwrap_or(0) > 0 {
            call_handler(
                lua,
                m,
                "OnContentUpdated",
                vec![Value::String(lua.create_string("unread_chat_message")?)],
            );
        }
    }
    Ok(())
}

fn colored(text: &str, color: [f32; 4]) -> String {
    let to = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("|c{:02X}{:02X}{:02X}{:02X}{text}", to(color[3]), to(color[0]), to(color[1]), to(color[2]))
}

pub(crate) fn message_methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    let def = |name: &str, f: mlua::Function| m.raw_set(name, f);
    // (text[, r, g, b, a]).
    let add = method(lua, |lua, id, a| {
        let text = a.s(1).unwrap_or_default();
        let text = if a.is_num(2) {
            let to = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            format!(
                "|c{:02X}{:02X}{:02X}{:02X}{text}",
                to(a.opt_f(5).unwrap_or(1.0)),
                to(a.f(2)),
                to(a.f(3)),
                to(a.f(4))
            )
        } else {
            text
        };
        add_line(lua, id, &text);
        none()
    })?;
    def("AddMessage", add.clone())?;
    def("AddMessageEx", add.clone())?;
    let refresh = method(lua, |lua, id, a| {
        with_msg(lua, id, |m| { m.lines.clear(); m.sources.clear(); });
        add_line(lua, id, &a.s(1).unwrap_or_default());
        none()
    })?;
    def("AddMessageRefresh", refresh.clone())?;
    def("AddMessageExRefresh", refresh)?;
    def(
        "Clear",
        method(lua, |lua, id, _| {
            with_msg(lua, id, |m| {
                m.lines.clear();
                m.sources.clear();
                m.scroll = 0;
            });
            none()
        })?,
    )?;
    def(
        "SetMaxLines",
        method(lua, |lua, id, a| {
            let n = a.f(1).max(1.0) as usize;
            with_msg(lua, id, |m| m.max_lines = n);
            none()
        })?,
    )?;
    def(
        "SetTimeVisible",
        method(lua, |lua, id, a| {
            let s = a.f(1);
            with_msg(lua, id, |m| m.time_visible = (s > 0.0).then_some(s));
            none()
        })?,
    )?;
    def(
        "ResetVisibleTime",
        method(lua, |lua, id, _| {
            let now = ui(lua).time_ms;
            with_msg(lua, id, |m| m.lines.iter_mut().for_each(|l| l.1 = now));
            none()
        })?,
    )?;
    def(
        "GetMessageLines",
        method(lua, |lua, id, _| {
            ret(lua, with_msg(lua, id, |m| m.lines.len()).unwrap_or(0))
        })?,
    )?;
    def(
        "GetCurrentScroll",
        method(lua, |lua, id, _| {
            ret(lua, with_msg(lua, id, |m| m.scroll).unwrap_or(0))
        })?,
    )?;
    for (name, delta) in [
        ("ScrollUp", 1i64),
        ("ScrollDown", -1),
        ("PageUp", 5),
        ("PageDown", -5),
    ] {
        def(
            name,
            method(lua, move |lua, id, _| {
                scroll_by(lua, id, delta);
                none()
            })?,
        )?;
    }
    def(
        "ScrollToBottom",
        method(lua, |lua, id, _| {
            set_scroll(lua, id, 0);
            none()
        })?,
    )?;
    def(
        "ScrollToTop",
        method(lua, |lua, id, _| {
            set_scroll(lua, id, i64::MAX);
            none()
        })?,
    )?;
    def(
        "GetLinkInfoOnCursor",
        method(lua, |lua, id, _| {
            let t = lua.create_table()?;
            t.raw_set("linkType", "none")?;
            if let Some(link) = link_on_cursor(&mut ui(lua), id) {
                // Native 3924b360 returns the fields for item 1, character 2 and quest 3.
                match link {
                    crate::text::ChatLink::Character { name, is_hostile } => {
                        t.raw_set("linkType", "character")?;
                        t.raw_set("characterName", name)?;
                        t.raw_set("isHostile", is_hostile)?;
                    }
                    crate::text::ChatLink::Item {
                        item_type,
                        grade,
                        encoded,
                    } => {
                        t.raw_set("linkType", "item")?;
                        t.raw_set("itemType", item_type)?;
                        t.raw_set("itemGrade", grade)?;
                        t.raw_set("itemLinkText", encoded)?;
                    }
                    crate::text::ChatLink::Quest { quest_type } => {
                        t.raw_set("linkType", "quest")?;
                        t.raw_set("questType", quest_type)?;
                    }
                }
            }
            ret(lua, t)
        })?,
    )?;
    for name in [
        "SetFadeDuration",
        "EnableItemLink",
        "ChangeDefaultStyle",
        "ChangeTextStyle",
    ] {
        def(name, method(lua, |_, _, _| none())?)?;
    }
    Ok(())
}

/// Match the cached visible message layout used by rendering, including wrapping, timeout,
/// scroll, inset, per-line alignment, UI scale and inherited widget transforms.
fn link_on_cursor(u: &mut super::Ui, id: Id) -> Option<crate::text::ChatLink> {
    if !u.tree.visible(id) {
        return None;
    }
    let rect = u.tree.rect(id);
    let view = u.animations.widget_rect(&u.tree, id);
    let point = [
        rect.x + (u.cursor.0 - view.x) * rect.w / view.w.max(0.001),
        rect.y + (u.cursor.1 - view.y) * rect.h / view.h.max(0.001),
    ];
    let text = compose(u, id, rect)?;
    let scale = u.ui_scale;
    let inner = Rect {
        x: rect.x + text.inset[0],
        y: rect.y + text.inset[1],
        w: (rect.w - text.inset[0] - text.inset[2]).max(0.0),
        h: (rect.h - text.inset[1] - text.inset[3]).max(0.0),
    };
    if !inner.contains(point[0], point[1]) {
        return None;
    }
    let layout = u.fonts.layout(
        &text.style.font,
        text.style.size * scale,
        &text.text,
        (inner.w > 0.0).then_some(inner.w * scale),
        text.line_space * scale,
        false,
    );
    let fx = text.style.align.fx();
    let ox = (inner.x * scale + (inner.w * scale - layout.width) * fx).round();
    let oy = (inner.y * scale + (inner.h * scale - layout.height) * text.style.align.fy()).round();
    for link in layout.links {
        let dx = ((layout.width - layout.line_widths[link.line]) * fx).round();
        let [x0, y0, x1, y1] = link.rect;
        let x = point[0] * scale - ox - dx;
        let y = point[1] * scale - oy;
        if x >= x0 && x < x1 && y >= y0 && y < y1 {
            return Some(link.payload);
        }
    }
    None
}

pub(crate) fn window_methods(lua: &Lua, m: &Table) -> mlua::Result<()> {
    let def = |name: &str, f: mlua::Function| m.raw_set(name, f);
    def(
        "SetChatWindowId",
        method(lua, |lua, id, a| {
            set_window_id(lua, id, a.f(1) as i64)?;
            none()
        })?,
    )?;
    fn set(lua: &Lua, id: Id, f: impl FnOnce(&mut ChatData)) {
        if let Some(c) = ui(lua).tree.widget_mut(id).and_then(|w| w.chat.as_mut()) {
            f(c);
        }
        layout(lua, id);
    }
    def(
        "SetTabWidth",
        method(lua, |lua, id, a| {
            let v = nonnegative(a.f(1));
            set(lua, id, |c| c.tab_width = v);
            none()
        })?,
    )?;
    def(
        "SetMinTabWidth",
        method(lua, |lua, id, a| {
            let v = nonnegative(a.f(1));
            set(lua, id, |c| c.min_tab_width = v);
            none()
        })?,
    )?;
    def(
        "SetTabAreaHeight",
        method(lua, |lua, id, a| {
            let v = nonnegative(a.f(1));
            set(lua, id, |c| c.tab_area_height = v);
            none()
        })?,
    )?;
    def(
        "UseAutoResizingTabButtonMode",
        method(lua, |lua, id, a| {
            let on = a.b(1, true);
            set(lua, id, |c| c.auto_resize_tabs = on);
            none()
        })?,
    )?;
    def(
        "SetOffset",
        method(lua, |lua, id, a| {
            let v = a.f(1);
            set(lua, id, |c| c.tab_offset = v);
            none()
        })?,
    )?;
    def(
        "SetGap",
        method(lua, |lua, id, a| {
            let v = a.f(1);
            set(lua, id, |c| c.tab_gap = v);
            none()
        })?,
    )?;
    def(
        "SetTabAreaInset",
        method(lua, |lua, id, a| {
            let v = [a.f(1), a.f(2), a.f(3), a.f(4)];
            set(lua, id, |c| c.tab_inset = v);
            none()
        })?,
    )?;
    def(
        "UseAddTabButton",
        method(lua, |lua, id, a| {
            let on = a.b(1, true);
            set(lua, id, |c| c.use_add_button = on);
            none()
        })?,
    )?;
    def(
        "SelectTab",
        method(lua, |lua, id, a| {
            select(lua, id, (a.f(1) as usize).saturating_sub(1));
            none()
        })?,
    )?;
    def(
        "GetSelectedTab",
        method(lua, |lua, id, _| {
            ret(lua, chat(lua, id).map_or(0, |c| c.selected + 1))
        })?,
    )?;
    for (getter, field) in [
        ("GetChatEdit", "chatEdit"),
        ("GetAddButton", "addButton"),
        ("GetImeToggleButton", "imeToggle"),
    ] {
        def(
            getter,
            method(lua, move |lua, id, _| {
                ret(lua, parts(lua, id)?.raw_get::<Value>(field)?)
            })?,
        )?;
    }
    def(
        "GetChatMethodSelector",
        method(lua, |lua, id, _| {
            let Some(c) = chat(lua, id) else {
                return none();
            };
            ret(
                lua,
                parts(lua, c.edit)?.raw_get::<Value>("chatMethodSelector")?,
            )
        })?,
    )?;
    for (getter, field) in [
        ("GetLockNotifyDrawable", "lockNotify"),
        ("GetCaretDrawable", "caret"),
        ("GetLeftLineDrawable", "leftLine"),
        ("GetRightLineDrawable", "rightLine"),
    ] {
        def(
            getter,
            method(lua, move |lua, id, _| {
                ret(lua, parts(lua, id)?.raw_get::<Value>(field)?)
            })?,
        )?;
    }
    def(
        "SetContentOffset",
        method(lua, |lua, id, a| {
            if let Some(c) = ui(lua).tree.widget_mut(id).and_then(|w| w.chat.as_mut()) {
                c.content_offset = [a.f(1), a.f(2), a.f(3), a.f(4)];
            }
            layout_content(lua, id);
            none()
        })?,
    )?;
    for (name, index) in [("SetLeftLineOffset", 0), ("SetRightLineOffset", 1)] {
        def(name, method(lua, move |lua, id, a| {
            if let Some(c) = ui(lua).tree.widget_mut(id).and_then(|w| w.chat.as_mut()) {
                c.line_offset[index] = a.f(1);
            }
            if let Some(c) = chat(lua, id) { layout_lines(&mut ui(lua), id, &c); }
            none()
        })?)?;
    }
    def("AllowTabSwitch", method(lua, |lua, id, a| {
        let allowed = match a.v(1) {
            Value::Boolean(value) => value,
            Value::Integer(value) => value != 0,
            Value::Number(value) => value != 0.0,
            _ => return none(),
        };
        set_allow_tab_switch(lua, id, allowed);
        none()
    })?)?;
    for (name, blink) in [("SetMaxNotifyTime", false), ("SetNotifyBlinkingFreq", true)] {
        def(name, method(lua, move |lua, id, a| {
            let value = match a.v(1) {
                Value::Number(value) => value as f32,
                Value::Integer(value) => value as f32,
                _ => return none(),
            };
            if let Some(c) = ui(lua).tree.widget_mut(id).and_then(|w| w.chat.as_mut()) {
                if blink { c.notify_blink_ms = nonnegative(value); }
                else { c.notify_max_ms = nonnegative(value); }
            }
            none()
        })?)?;
    }
    def("SetCaretOffset", method(lua, |lua, id, a| {
        if let Some(c) = ui(lua).tree.widget_mut(id).and_then(|w| w.chat.as_mut()) {
            c.interaction.caret_offset = [a.f(1), a.f(2)];
            c.interaction.preview = None;
        }
        none()
    })?)?;
    def("SetSlideTimeInDragging", method(lua, |lua, id, a| {
        if let Some(c) = ui(lua).tree.widget_mut(id).and_then(|w| w.chat.as_mut()) { c.interaction.slide_ms = a.f(1); }
        none()
    })?)?;
    def("UseSlidingButton", method(lua, |lua, id, a| {
        set(lua, id, |c| c.interaction.use_slides = a.b(1, true));
        none()
    })?)?;
    def("SetTabButtonAlpha", method(lua, |lua, id, a| {
        //39259fa0/3922f380: both parameters must be numbers. No clamp,
        // layout, callback, dynamic-layer update or fade is performed.
        if a.is_num(1) && a.is_num(2)
            && let Some(c) = ui(lua).tree.widget_mut(id).and_then(|w| w.chat.as_mut())
        {
            c.tab_button_alpha = [a.f(1), a.f(2)];
        }
        none()
    })?)?;
    Ok(())
}
