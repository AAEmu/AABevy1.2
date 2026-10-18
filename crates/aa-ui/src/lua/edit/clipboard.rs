//! Unicode clipboard access, requested only by an explicit edit shortcut.
//! Optional AAUI hooks let an embedded/headless runtime supply its own clipboard.

use mlua::{Function, Lua, Table};

fn hook(lua: &Lua, name: &str) -> Option<Function> {
    let aaui: Table = lua.globals().raw_get("AAUI").ok()?;
    aaui.raw_get(name).ok()
}

pub(super) fn get(lua: &Lua) -> Option<String> {
    if let Some(f) = hook(lua, "GetClipboard") {
        return match f.call::<Option<String>>(()) {
            Ok(text) => text,
            Err(e) => {
                bevy::log::warn!("aa-ui: AAUI.GetClipboard: {e}");
                None
            }
        };
    }
    #[cfg(windows)]
    return clipboard_win::get_clipboard_string().ok();
    #[cfg(not(windows))]
    None
}

pub(super) fn set(lua: &Lua, text: &str) {
    // Native39227b60/39227c30 retain the complete Unicode range before OS conversion.
    super::super::ui(lua).clipboard = text.to_string();
    // Native39224a60 converts into a 0x401-wide-character buffer including the NUL.
    let mut units = 0;
    let text: String = text
        .chars()
        .take_while(|c| {
            units += c.len_utf16();
            *c != '\0' && units <= 1024
        })
        .collect();
    if let Some(f) = hook(lua, "SetClipboard") {
        if let Err(e) = f.call::<()>(text) {
            bevy::log::warn!("aa-ui: AAUI.SetClipboard: {e}");
        }
        return;
    }
    #[cfg(windows)]
    let _ = clipboard_win::set_clipboard_string(&text);
}

pub(super) fn paste(lua: &Lua) -> String {
    if let Some(text) = get(lua) {
        super::super::ui(lua).clipboard = paste_text(&text);
    }
    // Native392282c1 skips loading unsupported clipboard formats, then3922836f uses
    // the existing global transfer buffer at39ac6a04 (including an initially empty one).
    super::super::ui(lua).clipboard.clone()
}

pub(super) fn paste_text(text: &str) -> String {
    // Native39228270: UTF-8 buffer4097, CRLF -> LF, then UTF-32 buffer1025.
    let mut bytes = 0;
    let text: String = text
        .chars()
        .take_while(|c| {
            bytes += c.len_utf8();
            *c != '\0' && bytes <= 4096
        })
        .collect();
    text.replace("\r\n", "\n").chars().take(1024).collect()
}
