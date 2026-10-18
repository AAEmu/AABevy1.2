//! Native chat edits display ordinary bracketed labels and retain the complete link separately.

use mlua::{Function, Lua, Table, Value};

use super::{edit_text, window_of};
use crate::{
    lua::{call_handler, ui},
    text::ChatLink,
    tree::Id,
};

#[derive(Clone, Debug, PartialEq)]
pub struct InputLink {
    pub label: String,
    pub raw: String,
}

pub(super) fn activate(lua: &Lua, target: Id) {
    let mut u = ui(lua);
    let mut at = Some(target);
    let mut found = None;
    while let Some(id) = at {
        if u.tree.widget(id).is_some_and(|w| w.chat.is_some()) {
            found = Some(id);
            break;
        }
        at = u.tree.get(id).parent;
    }
    let Some(found) = found else { return };
    for id in u.tree.of_class("chatwindow") {
        if let Some(c) = u.tree.widget_mut(id).and_then(|w| w.chat.as_mut()) {
            c.active = id == found;
        }
    }
}

pub(super) fn active(lua: &Lua) -> Option<(Id, Id)> {
    let u = ui(lua);
    u.tree.of_class("chatwindow").into_iter().find_map(|id| {
        let c = u.tree.widget(id)?.chat.as_ref()?;
        // Native395128e0 calls IsVisible(false) on the edit: its own flag, not focus.
        (c.active && u.tree.get(c.edit).alive && u.tree.get(c.edit).shown).then_some((id, c.edit))
    })
}

pub(super) fn install(lua: &Lua, chat: &Table) -> mlua::Result<()> {
    chat.raw_set(
        "IsActivatedChatInput",
        lua.create_function(|lua, _: Value| Ok(active(lua).is_some()))?,
    )?;
    for (name, kind) in [
        ("AddItemLinkToActiveChatInput", 'i'),
        ("AddQuestLinkToActiveChatInput", 'q'),
    ] {
        chat.raw_set(
            name,
            lua.create_function(move |lua, (_, raw): (Value, Option<String>)| {
                let Some((window, edit)) = active(lua) else {
                    return Ok(false);
                };
                if let Some(raw) = raw {
                    append(lua, window, edit, kind, &raw);
                }
                // Native3950884e/3950891e fail only without an active shown input.
                // Definition/length rejection inside the append method still returns true.
                Ok(true)
            })?,
        )?;
    }
    Ok(())
}

fn append(lua: &Lua, window: Id, edit: Id, kind: char, raw: &str) {
    let Some(body) = raw
        .strip_prefix(&format!("|{kind}"))
        .and_then(|s| s.strip_suffix(';'))
    else {
        return;
    };
    let Some(link) = crate::text::parse_chat_link(kind, body) else {
        return;
    };
    if raw.len() > 511 {
        return;
    }
    let (kind, id, grade) = match link {
        ChatLink::Item {
            item_type, grade, ..
        } => ("item", item_type, Some(grade)),
        ChatLink::Quest { quest_type } => ("quest", quest_type, None),
        _ => return,
    };
    let hook: Option<Function> = lua
        .globals()
        .raw_get::<Table>("AAUI")
        .ok()
        .and_then(|t| t.raw_get("ChatLink").ok());
    let Some(hook) = hook else { return };
    let label = match hook.call::<Option<Table>>((kind, id, grade)) {
        Ok(Some(t)) => t.raw_get::<Option<String>>("name").ok().flatten(),
        Ok(None) => None,
        Err(e) => {
            bevy::log::warn!("aa-ui: AAUI.ChatLink input: {e}");
            None
        }
    };
    let Some(name) = label.filter(|s| !s.is_empty() && !s.contains('\0')) else {
        return;
    };
    let label = format!("[{name}]");
    if label.len() > 255 || active(lua) != Some((window, edit)) {
        return;
    }
    let text = edit_text(lua, edit);
    let mut links = ui(lua)
        .tree
        .widget(window)
        .and_then(|w| w.chat.as_ref())
        .map(|c| c.links.clone())
        .unwrap_or_default();
    links.push(InputLink {
        label: label.clone(),
        raw: raw.to_string(),
    });
    // Native39511149/39511348 checks serialized byte length, including the newly added map.
    if serialize(&text, &links).len() + raw.len() >= 1024 {
        return;
    }
    let mut shown = text + &label; // Native appends at the end, independently of caret/selection.
    if shown.len() > 511 {
        let mut end = 511;
        while !shown.is_char_boundary(end) {
            end -= 1
        }
        shown.truncate(end);
    }
    if let Some(c) = ui(lua)
        .tree
        .widget_mut(window)
        .and_then(|w| w.chat.as_mut())
    {
        c.links = links;
    }
    if crate::lua::edit::set_text(lua, edit, shown) {
        super::text_changed(lua, edit);
        call_handler(lua, edit, "OnTextChanged", vec![]);
    }
    crate::lua::edit::set_focus(lua, Some(edit));
}

/// Native39510e80 substitutes the first remaining intact label for each retained entry.
/// User edits to a label are ordinary edits; a broken label stays literal when sent.
fn serialize(text: &str, links: &[InputLink]) -> String {
    let mut out = String::new();
    let mut rest = text;
    for link in links {
        if let Some(at) = rest.find(&link.label) {
            out.push_str(&rest[..at]);
            out.push_str(&link.raw);
            rest = &rest[at + link.label.len()..];
        }
    }
    out.push_str(rest);
    out
}

pub(super) fn send(lua: &Lua, edit: Id, text: &str) -> String {
    let Some(window) = window_of(lua, edit) else {
        return text.to_string();
    };
    let links = {
        let mut u = ui(lua);
        let Some(c) = u.tree.widget_mut(window).and_then(|w| w.chat.as_mut()) else {
            return text.to_string();
        };
        std::mem::take(&mut c.links)
    };
    serialize(text, &links)
}
