//! Opening and closing the in-game windows from the app's key bindings, the way the client's
//! natives do it: hotkey actions (`toggle_bag`, `toggle_character`, ...) toggle the `ADDON`
//! content (`UIC_*`) the scripts registered for it (`ADDON:RegisterContentWidget` /
//! `RegisterContentTriggerFunc`), Escape closes the topmost window that asked for it
//! (`SetCloseOnEscape(true)`) or else opens the system menu (`OPEN_CONFIG`).

use mlua::{Function, Lua, Table, Value};

use crate::lua::{self, object_table, ui};

/// Hotkey actions of the client's key bindings and the content they toggle, for the actions
/// the main menu table (`MAIN_MENU_INFO`) does not list.
const ACTIONS: &[(&str, &str)] = &[
    ("toggle_character", "UIC_CHARACTER_INFO"),
    ("toggle_bag", "UIC_BAG"),
    ("toggle_quest", "UIC_QUEST_LIST"),
    ("toggle_spellbook", "UIC_SKILL"),
    ("toggle_worldmap", "UIC_WORLDMAP"),
    ("toggle_achievement", "UIC_ACHIEVEMENT"),
    ("toggle_option", "UIC_OPTION_FRAME"),
    ("toggle_ranking", "UIC_RANK"),
    ("toggle_craft_book", "UIC_CRAFT_BOOK"),
    ("toggle_friend", "UIC_FRIEND"),
    ("toggle_expedition_management", "UIC_EXPEDITION"),
    ("toggle_raid_team_manager", "UIC_RAID_TEAM_MANAGER"),
    ("toggle_nation_management", "UIC_NATION"),
    ("toggle_common_farm_info", "UIC_MY_FARM_INFO"),
];

fn addon(lua: &Lua) -> mlua::Result<Table> {
    lua.globals().raw_get("ADDON")
}

/// `ADDON:ToggleContent(content)`.
pub fn toggle(lua: &Lua, content: i64) -> mlua::Result<()> {
    let a = addon(lua)?;
    let f: Function = a.get("ToggleContent")?;
    f.call::<()>((a, content))?;
    lua::flush(lua);
    Ok(())
}

/// `ADDON:ShowContent(content, show)`.
pub fn show(lua: &Lua, content: i64, on: bool) -> mlua::Result<()> {
    let a = addon(lua)?;
    let f: Function = a.get("ShowContent")?;
    f.call::<()>((a, content, on))?;
    lua::flush(lua);
    Ok(())
}

/// Whether the content's registered widget is shown (false when only a trigger function is
/// registered: those windows are not known to the registry).
pub fn visible(lua: &Lua, content: i64) -> bool {
    let w = addon(lua)
        .and_then(|a| {
            let f: Function = a.get("GetContent")?;
            f.call::<Value>((a, content))
        })
        .ok();
    match w.and_then(|v| lua::value_id(lua, &v)) {
        Some(id) => ui(lua).tree.visible(id),
        None => false,
    }
}

/// The value of a `UIC_*` constant.
pub fn content_id(lua: &Lua, name: &str) -> Option<i64> {
    match lua.globals().raw_get::<Value>(name).ok()? {
        Value::Integer(i) => Some(i),
        Value::Number(n) => Some(n as i64),
        _ => None,
    }
}

/// Run a hotkey action. Returns false for actions that open nothing here.
pub fn hotkey(lua: &Lua, action: &str) -> mlua::Result<bool> {
    if action == "open_config" {
        lua::fire_ui_event(lua, "OPEN_CONFIG", Vec::new());
        return Ok(true);
    }
    // The main menu's table: `{ hotkey = "toggle_bag", content = UIC_BAG, toggleFunc = ... }`.
    if let Ok(Some(menu)) = lua.globals().raw_get::<Option<Table>>("MAIN_MENU_INFO") {
        for entry in menu.pairs::<Value, Value>() {
            let Ok((_, Value::Table(e))) = entry else {
                continue;
            };
            if e.raw_get::<Option<String>>("hotkey")
                .ok()
                .flatten()
                .as_deref()
                != Some(action)
            {
                continue;
            }
            if let Some(c) = e.raw_get::<Option<i64>>("content")? {
                toggle(lua, c)?;
                return Ok(true);
            }
            if let Some(f) = e.raw_get::<Option<Function>>("toggleFunc")? {
                f.call::<()>(())?;
                lua::flush(lua);
                return Ok(true);
            }
        }
    }
    let Some(&(_, name)) = ACTIONS.iter().find(|(a, _)| *a == action) else {
        return Ok(false);
    };
    let Some(c) = content_id(lua, name) else {
        return Ok(false);
    };
    toggle(lua, c)?;
    Ok(true)
}

/// Escape outside an edit box: hide the topmost shown window that has `SetCloseOnEscape(true)`
/// (its `OnCloseByEsc` runs before hiding, followed by `OnHide`), else raise `OPEN_CONFIG`
/// (the scripts open the system menu, which itself closes on Escape). Returns what it did.
pub fn escape(lua: &Lua) -> mlua::Result<Escaped> {
    let top = {
        let u = ui(lua);
        let order = u.tree.paint_order();
        order.into_iter().rev().find(|&id| {
            u.tree.get(id).alive
                && u.tree.widget(id).is_some_and(|w| w.close_on_escape)
                && u.tree.visible(id)
        })
    };
    match top {
        Some(id) => {
            // Native 392d4500 dispatches handler index 0x29 (OnCloseByEsc) before hiding.
            // The original dialog handler uses it for CancelProc and manager cleanup.
            lua::call_handler(lua, id, "OnCloseByEsc", vec![]);
            if let Some(t) = object_table(lua, id)? {
                let f: Function = t.get("Show")?;
                f.call::<()>((t, false))?;
            } else {
                ui(lua).tree.get_mut(id).shown = false;
            }
            lua::flush(lua);
            Ok(Escaped::Closed(id))
        }
        None => {
            lua::fire_ui_event(lua, "OPEN_CONFIG", Vec::new());
            Ok(Escaped::OpenedMenu)
        }
    }
}

/// What [`escape`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Escaped {
    /// Hid this window.
    Closed(crate::tree::Id),
    /// Nothing to close: raised `OPEN_CONFIG`.
    OpenedMenu,
}
