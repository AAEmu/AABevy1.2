//! The chat window's input (`chatwindow:GetChatEdit()`): slash commands pick the channel, Enter
//! sends.
//!
//! As in the client, the edit box handles the commands natively while typing: a command followed
//! by a space (`/s `, `/p `, `/1 `, `/w Name `) leaves the text and sets the input's channel; the
//! method selector (`GetChatMethodSelector()`) shows the channel's name and gets
//! `OnContentUpdated(self, "chat_method_changed")`, so the scripts resize it and move the edit's
//! text inset (`chat/chat_window.lua`); a whisper target is the edit's prefix text
//! (`SetPrefixText`, drawn in `prefixStyle`). Unknown commands stay in the text.
//!
//! Commands: the client's own from `locale.chatChannel.parse_chat_<kind>_<n>` (en_us: `/local`
//! `/s` say, `/shout` zone, `/party` `/p`, `/raid`, `/guild`, `/whisper` `/w`; its `/faction`
//! goes to the guild, as in the client's table), then `/say`, `/y` `/yell` (zone), `/trade`,
//! `/g` (guild), `/n` `/nation` (nation), `/a` `/ally` `/alliance` (alliance), `/t` `/tell`
//! (whisper) and the channel numbers of the "Entering Chat" lines (`/1` shout, `/2` trade,
//! `/3` party search, `/4` party, `/5` raid, `/6` nation, `/7` guild, `/11` trial,
//! `/14` alliance; `/0` say). The names come from `locale.chatChannel` (`say` = "General",
//! `chat_zone` = "Shout", ...).
//!
//! Enter (when no script set `OnEnterPressed` on the edit) sends the line to the app:
//!
//! ```lua
//! AAUI.ChatSend(channel, text, target)  -- channel: CHAT_SAY 0, CHAT_ZONE 1, ..., CHAT_WHISPER -3;
//!                                       -- target: the whisper name, else ""
//! ```
//!
//! then empties the edit, drops the focus and hides the edit (the client's input closes after a
//! line). A bare command before Enter (`/p`) only switches the channel. The channel stays for the
//! next line. Without `AAUI.ChatSend` the line is dropped.

use mlua::{Function, Lua, Table, Value};

use super::{call_handler, has_handler, ui};
use crate::tree::Id;

const CHAT_SAY: i64 = 0;
const CHAT_WHISPER: i64 = -3;

/// Channel -> (`locale.chatChannel` key of its name, English fallback).
const NAMES: &[(i64, &str, &str)] = &[
    (0, "say", "General"),
    (1, "chat_zone", "Shout"),
    (2, "chat_trade", "Trade"),
    (3, "chat_find_party", "Party Search"),
    (4, "party", "Party"),
    (5, "raid", "Raid"),
    (6, "faction", "Nation"),
    (7, "expedition", "Guild"),
    (11, "trial", "Trial"),
    (14, "race", "Alliance"),
    (CHAT_WHISPER, "whisper", "Whisper"),
];

/// `parse_chat_<kind>_<n>` kinds of `locale.chatChannel`.
const LOCALE_KINDS: &[(&str, i64)] = &[
    ("say", 0),
    ("yell", 1),
    ("party", 4),
    ("raid", 5),
    ("guild", 7),
    ("whisper", CHAT_WHISPER),
];

/// Commands that work whatever the locale says (after the locale's own).
const BUILTIN: &[(&str, i64)] = &[
    ("s", 0),
    ("say", 0),
    ("local", 0),
    ("shout", 1),
    ("y", 1),
    ("yell", 1),
    ("trade", 2),
    ("p", 4),
    ("party", 4),
    ("raid", 5),
    ("g", 7),
    ("guild", 7),
    ("n", 6),
    ("nation", 6),
    ("a", 14),
    ("ally", 14),
    ("alliance", 14),
    ("w", CHAT_WHISPER),
    ("whisper", CHAT_WHISPER),
    ("t", CHAT_WHISPER),
    ("tell", CHAT_WHISPER),
    ("0", 0),
    ("1", 1),
    ("2", 2),
    ("3", 3),
    ("4", 4),
    ("5", 5),
    ("6", 6),
    ("7", 7),
    ("11", 11),
    ("14", 14),
];

fn locale_channels(lua: &Lua) -> Option<Table> {
    let locale: Table = lua.globals().raw_get("locale").ok()?;
    locale.raw_get("chatChannel").ok()
}

/// The channel of a command (lower case, without the slash).
pub fn command_channel(lua: &Lua, cmd: &str) -> Option<i64> {
    if let Some(t) = locale_channels(lua) {
        for &(kind, ch) in LOCALE_KINDS {
            for n in 1..=8 {
                let key = format!("parse_chat_{kind}_{n}");
                let Ok(Some(c)) = t.raw_get::<Option<String>>(key.as_str()) else {
                    continue;
                };
                if c.trim_start_matches('/').eq_ignore_ascii_case(cmd) {
                    return Some(ch);
                }
            }
        }
    }
    BUILTIN.iter().find(|(c, _)| *c == cmd).map(|&(_, ch)| ch)
}

/// The name the method selector shows for a channel.
pub fn channel_name(lua: &Lua, ch: i64) -> String {
    let Some(&(_, key, fallback)) = NAMES.iter().find(|n| n.0 == ch) else {
        return String::new();
    };
    locale_channels(lua)
        .and_then(|t| t.raw_get::<Option<String>>(key).ok().flatten())
        .unwrap_or_else(|| fallback.to_string())
}

/// The chat window whose input `edit` is.
fn window_of(lua: &Lua, edit: Id) -> Option<Id> {
    let u = ui(lua);
    let w = u.tree.get(edit).parent?;
    let c = u.tree.widget(w)?.chat.as_ref()?;
    (c.edit == edit).then_some(w)
}

fn edit_text(lua: &Lua, edit: Id) -> String {
    ui(lua)
        .tree
        .get(edit)
        .text()
        .map(|t| t.text.clone())
        .unwrap_or_default()
}

/// Show the input's channel: the selector's text, the edit's whisper prefix, and the selector's
/// `OnContentUpdated` for the scripts.
pub(crate) fn show_method(lua: &Lua, window: Id) {
    let Some(c) = ui(lua).tree.widget(window).and_then(|w| w.chat.clone()) else {
        return;
    };
    let name = channel_name(lua, c.method);
    let prefix = if c.method == CHAT_WHISPER {
        c.whisper.clone()
    } else {
        String::new()
    };
    {
        let mut u = ui(lua);
        if let Some(t) = u.tree.get_mut(c.selector).text_mut() {
            t.text = name;
        }
        if let Some(e) = u.tree.widget_mut(c.edit).and_then(|w| w.edit.as_mut()) {
            e.prefix = prefix;
        } else if let Some(w) = u.tree.widget_mut(c.edit) {
            w.edit = Some(super::edit::EditData {
                prefix,
                ..Default::default()
            });
        }
        u.tree.touch();
    }
    let kind = lua
        .create_string("chat_method_changed")
        .map(Value::String)
        .unwrap_or(Value::Nil);
    call_handler(lua, c.selector, "OnContentUpdated", vec![kind]);
}

fn set_method(lua: &Lua, window: Id, ch: i64, target: &str) {
    if let Some(c) = ui(lua)
        .tree
        .widget_mut(window)
        .and_then(|w| w.chat.as_mut())
    {
        c.method = ch;
        c.whisper = if ch == CHAT_WHISPER {
            target.to_string()
        } else {
            String::new()
        };
    }
    show_method(lua, window);
}

/// A leading command in `text`: `(channel, whisper target, rest of the text)`. The command (and a
/// whisper's name) must be followed by white space.
fn parse(lua: &Lua, text: &str) -> Option<(i64, String, String)> {
    let rest = text.strip_prefix('/')?;
    let sp = rest.find(char::is_whitespace)?;
    let cmd = rest[..sp].to_lowercase();
    let after = rest[sp..].trim_start_matches(' ');
    let ch = command_channel(lua, &cmd)?;
    if ch == CHAT_WHISPER {
        let sp2 = after.find(char::is_whitespace)?;
        let name = &after[..sp2];
        if name.is_empty() {
            return None;
        }
        let msg = after[sp2..].trim_start_matches(' ');
        return Some((ch, name.to_string(), msg.to_string()));
    }
    Some((ch, String::new(), after.to_string()))
}

/// The text of `edit` changed by typing: take a leading command out of it.
pub(crate) fn text_changed(lua: &Lua, edit: Id) {
    let Some(win) = window_of(lua, edit) else {
        return;
    };
    let text = edit_text(lua, edit);
    let Some((ch, target, rest)) = parse(lua, &text) else {
        return;
    };
    super::edit::set_text(lua, edit, rest);
    set_method(lua, win, ch, &target);
}

/// Enter in `edit`. True when it is a chat input without a script `OnEnterPressed`: the line
/// went to `AAUI.ChatSend` (see the module docs).
pub(crate) fn enter(lua: &Lua, edit: Id) -> bool {
    let Some(win) = window_of(lua, edit) else {
        return false;
    };
    if has_handler(lua, edit, "OnEnterPressed") {
        return false;
    }
    let mut text = edit_text(lua, edit);
    // A bare command (`/p`) switches the channel.
    if let Some((ch, target, rest)) = parse(lua, &format!("{text} ")) {
        set_method(lua, win, ch, &target);
        text = rest;
    }
    let text = text.trim().to_string();
    let (method, target) = ui(lua)
        .tree
        .widget(win)
        .and_then(|w| w.chat.as_ref())
        .map(|c| (c.method, c.whisper.clone()))
        .unwrap_or((CHAT_SAY, String::new()));
    if !text.is_empty() {
        let hook = lua
            .globals()
            .raw_get::<Option<Table>>("AAUI")
            .ok()
            .flatten()
            .and_then(|t| t.raw_get::<Option<Function>>("ChatSend").ok().flatten());
        if let Some(f) = hook
            && let Err(e) = f.call::<()>((method, text, target))
        {
            bevy::log::warn!("aa-ui: AAUI.ChatSend: {e}");
        }
    }
    if super::edit::set_text(lua, edit, String::new()) {
        call_handler(lua, edit, "OnTextChanged", vec![]);
    }
    super::edit::set_focus(lua, None);
    ui(lua).tree.get_mut(edit).shown = false;
    super::flush(lua);
    true
}
