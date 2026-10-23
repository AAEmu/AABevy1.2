//! Local native chat records, original Lua dialogs, and character-scoped application settings.
use super::super::api::{HudBridge, Shared};
use aa_net::{Event, NetEvent};
use aa_ui::{
    UiRuntime,
    lua::{object_table, ui},
    tree::Id,
};
use bevy::prelude::*;
use mlua::{Lua, MultiValue, ObjectLike, Table, Value};
use serde_json::json;
#[allow(dead_code)]
pub(in crate::hud) mod fixture;
mod locks;
mod model;
mod reset;
mod table;
#[cfg(test)]
mod tests;
use model::Model;

pub(in crate::hud) struct View {
    world: Option<u8>,
    owner: u32,
    loaded: bool,
    font: i32,
    model: Model,
    options: [f32; 2],
    locks: [bool; 8],
}
impl Default for View {
    fn default() -> Self {
        Self {
            world: None,
            owner: 0,
            loaded: false,
            font: 15,
            model: Model::default(),
            options: [0.7, 0.5],
            locks: [false; 8],
        }
    }
}
impl View {
    /// `<world>:<character>` once the selected world and the character are known (the key of the stored records).
    pub(in crate::hud) fn character_key(&self) -> Option<String> {
        self.key()
    }
    fn key(&self) -> Option<String> {
        let world = self.world?;
        if self.owner == 0 {
            return None;
        }
        Some(format!("{world}:{}", self.owner as i32))
    }
    fn identity(&mut self, world: Option<u8>, owner: u32) {
        if (self.world, self.owner) != (world, owner) {
            self.world = world;
            self.owner = owner;
            self.loaded = false;
            self.model = Model::default();
        }
    }
}

fn numeric(value: &Value) -> Option<f32> {
    match value {
        Value::Integer(n) => Some(*n as f32),
        Value::Number(n) => Some(*n as f32),
        _ => None,
    }
}
fn integer(value: &Value) -> Option<i32> {
    numeric(value).map(|n| {
        if n.is_finite() && (-2147483648.0..2147483648.0).contains(&n) {
            n as i32
        } else {
            i32::MIN
        }
    })
}
fn argument(args: &MultiValue, n: usize) -> Option<i32> {
    integer(args.get(n)?)
}
fn boolean(value: bool) -> MultiValue {
    MultiValue::from_iter([Value::Boolean(value)])
}
fn text(args: &MultiValue, n: usize) -> Option<String> {
    let Value::String(value) = args.get(n)? else { return None };
    let bytes = value.as_bytes();
    std::str::from_utf8(bytes.split(|&b| b == 0).next()?).ok().map(str::to_string)
}

fn ensure(lua: &Lua, state: &Shared) -> mlua::Result<()> {
    if state.borrow().chat_config.loaded {
        return Ok(());
    }
    let locale: Table = lua.globals().get("X2Locale")?;
    let names = ["general_tab_name", "party_tab_name", "trade_tab_name"].map(|key| {
        locale
            .call_method::<String>("LocalizeUiText", (79, key))
            .unwrap_or_else(|_| key.into())
    });
    let mut data = state.borrow_mut();
    let view = &mut data.chat_config;
    let key = view.key();
    let loaded = key.as_deref().and_then(|key| {
        let tabs = crate::settings::chat_read(lua, key, "tabs")?;
        Model::load(&tabs, None)
    });
    view.model = loaded.unwrap_or_else(|| Model::defaults(names, view.font));
    let extra = key.as_deref().and_then(|key| crate::settings::chat_read(lua, key, "extra"));
    view.model.load_extra(extra.as_ref());
    view.options = key
        .as_deref()
        .and_then(|key| crate::settings::chat_read(lua, key, "options"))
        .and_then(|value| serde_json::from_value::<[u32; 2]>(value).ok())
        .map(|v| v.map(f32::from_bits))
        .unwrap_or([0.7, 0.5]);
    view.loaded = true;
    view.locks = key
        .as_deref()
        .and_then(|key| crate::settings::chat_read(lua, key, "locks"))
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or([false; 8]);
    let [over, leave] = view.options;
    drop(data);
    aa_ui::lua::chat::set_alpha_options(lua, over, leave);
    Ok(())
}
fn save(lua: &Lua, state: &Shared, suffix: &str) -> bool {
    let data = state.borrow();
    let view = &data.chat_config;
    let Some(key) = view.key() else { return false };
    let value = match suffix {
        "tabs" => view.model.tabs_json(),
        "extra" => view.model.extra_json(),
        "options" => json!(view.options.map(f32::to_bits)),
        "locks" => json!(view.locks),
        _ => return false,
    };
    crate::settings::chat_save(lua, &key, suffix, value)
}
fn live_window(lua: &Lua, window: i32) -> Option<Id> {
    let u = ui(lua);
    u.tree.of_class("chatwindow").into_iter().find(|&id| {
        u.tree
            .widget(id)
            .and_then(|w| w.chat.as_ref())
            .is_some_and(|c| c.window_id == i64::from(window))
    })
}

pub(in crate::hud) fn install(lua: &Lua, state: &Shared) -> mlua::Result<()> {
    locks::install(lua, state)?;
    reset::install(lua, state)?;
    let api: Table = lua.globals().get("X2Chat")?;
    let hooks: Table = lua.globals().get("AAUI")?;
    hooks.raw_set(
        "PrepareChatInput",
        lua.create_function(|lua, window: Value| {
            if let Some(window) = aa_ui::lua::value_id(lua, &window) {
                aa_ui::lua::chat_input::prepare_input(lua, window);
            }
            Ok(())
        })?,
    )?;
    let s = state.clone();
    hooks.raw_set(
        "ChatTabs",
        lua.create_function(move |lua, window: i32| {
            ensure(lua, &s)?;
            let data = s.borrow();
            let view = &data.chat_config;
            let out = lua.create_table()?;
            if let Some(ids) = view.model.windows.get(&window) {
                for id in ids {
                    out.raw_push(table::info(lua, &view.model.tabs[id], true)?)?;
                }
            }
            let extra = view.model.extra.get(&window).copied().unwrap_or_default();
            out.raw_set("selected", extra.selected)?;
            out.raw_set("method", extra.method)?;
            Ok(out)
        })?,
    )?;
    let s = state.clone();
    hooks.raw_set(
        "ChatSelection",
        lua.create_function(move |lua, (window, selected): (i32, i32)| {
            ensure(lua, &s)?;
            s.borrow_mut().chat_config.model.extra.entry(window).or_default().selected = selected;
            save(lua, &s, "extra");
            Ok(())
        })?,
    )?;
    let s = state.clone();
    hooks.raw_set(
        "ChatMethod",
        lua.create_function(move |lua, (raw_id, method): (i32, i32)| {
            ensure(lua, &s)?;
            let mut data = s.borrow_mut();
            let model = &mut data.chat_config.model;
            let Some(window) = model.tabs.get(&raw_id).map(|tab| tab.window) else {
                return Ok(());
            };
            model.extra.entry(window).or_default().method = method;
            drop(data);
            save(lua, &s, "extra");
            Ok(())
        })?,
    )?;
    let s = state.clone();
    api.raw_set(
        "AllChatWindowIds",
        lua.create_function(move |lua, _: MultiValue| {
            ensure(lua, &s)?;
            let out = lua.create_table()?;
            for &id in s.borrow().chat_config.model.windows.keys() {
                out.raw_push(f64::from(id as f32))?;
            }
            Ok(out)
        })?,
    )?;
    let s = state.clone();
    api.raw_set(
        "SetOptionDefaultFontSize",
        lua.create_function(move |_, args: MultiValue| {
            if let Some(font) = argument(&args, 1) {
                s.borrow_mut().chat_config.font = font;
            }
            Ok(())
        })?,
    )?;
    for (method, defaults) in [("GetChatTabInfoTable", false), ("GetDefaultChatTabInfoTable", true)] {
        let s = state.clone();
        api.raw_set(
            method,
            lua.create_function(move |lua, args: MultiValue| {
                let Some(id) = argument(&args, 1) else {
                    return Ok(MultiValue::new());
                };
                ensure(lua, &s)?;
                let data = s.borrow();
                let view = &data.chat_config;
                let Some(tab) = view.model.tabs.get(&id) else {
                    return Ok(MultiValue::new());
                };
                let reset;
                let tab = if defaults {
                    reset = tab.defaults(view.font);
                    &reset
                } else {
                    tab
                };
                Ok(MultiValue::from_iter([Value::Table(table::info(lua, tab, false)?)]))
            })?,
        )?;
    }
    let s = state.clone();
    api.raw_set(
        "AddNewChatTabByUser",
        lua.create_function(move |lua, args: MultiValue| {
            let (Some(window), Some(name)) = (argument(&args, 1), text(&args, 2)) else {
                return Ok(MultiValue::new());
            };
            if !crate::login::name_policy::valid_bytes(name.as_bytes(), 4) {
                return Ok(boolean(false));
            }
            ensure(lua, &s)?;
            let allocated = {
                let mut data = s.borrow_mut();
                let view = &mut data.chat_config;
                view.model.allocate(window, &name, view.font)
            };
            let Some(id) = allocated else { return Ok(boolean(false)) };
            let info = s.borrow().chat_config.model.tabs[&id].widget_info();
            let appended = if let Some(widget) = live_window(lua, window) {
                aa_ui::lua::chat::append_tab(lua, widget, &info)?
            } else {
                false
            };
            if !appended {
                let mut data = s.borrow_mut();
                let model = &mut data.chat_config.model;
                model.tabs.remove(&id);
                if let Some(ids) = model.windows.get_mut(&window) {
                    ids.retain(|&other| other != id);
                }
                return Ok(boolean(false));
            }
            save(lua, &s, "tabs");
            Ok(boolean(true))
        })?,
    )?;
    let s = state.clone();
    api.raw_set(
        "RenameChatTabByUser",
        lua.create_function(move |lua, args: MultiValue| {
            let (Some(id), Some(name)) = (argument(&args, 1), text(&args, 2)) else {
                return Ok(MultiValue::new());
            };
            ensure(lua, &s)?;
            if !s.borrow_mut().chat_config.model.rename(id, &name) {
                return Ok(boolean(false));
            }
            save(lua, &s, "tabs");
            let info = s.borrow().chat_config.model.tabs[&id].widget_info();
            aa_ui::lua::chat::update_tab(lua, i64::from(id), &info)?;
            Ok(boolean(true))
        })?,
    )?;
    let s = state.clone();
    api.raw_set(
        "UpdateChatTabInfo",
        lua.create_function(move |lua, args: MultiValue| {
            let (Some(id), Some(Value::Table(incoming))) = (argument(&args, 1), args.get(2)) else {
                return Ok(MultiValue::new());
            };
            ensure(lua, &s)?;
            let Some(old) = s.borrow().chat_config.model.tabs.get(&id).cloned() else {
                return Ok(boolean(false));
            };
            let Some(new) = table::update(incoming.clone(), old)? else {
                return Ok(boolean(false));
            };
            if new.id != id || !s.borrow_mut().chat_config.model.replace(new.clone()) {
                return Ok(boolean(false));
            }
            save(lua, &s, "tabs");
            aa_ui::lua::chat::update_tab(lua, i64::from(id), &new.widget_info())?;
            Ok(boolean(true))
        })?,
    )?;
    let s = state.clone();
    api.raw_set(
        "DeleteChatTabByUser",
        lua.create_function(move |lua, args: MultiValue| {
            let Some(id) = argument(&args, 1) else {
                return Ok(MultiValue::new());
            };
            ensure(lua, &s)?;
            aa_ui::lua::chat::remove_tab(lua, i64::from(id))?;
            let removed = s.borrow_mut().chat_config.model.remove(id);
            if removed {
                save(lua, &s, "tabs");
            }
            Ok(boolean(removed))
        })?,
    )?;
    let s = state.clone();
    api.raw_set(
        "ClearChatContentByUser",
        lua.create_function(move |lua, args: MultiValue| {
            let Some(id) = argument(&args, 1) else {
                return Ok(MultiValue::new());
            };
            ensure(lua, &s)?;
            let message = {
                let u = ui(lua);
                u.tree.of_class("chatwindow").into_iter().find_map(|widget| {
                    u.tree
                        .widget(widget)?
                        .chat
                        .as_ref()?
                        .tabs
                        .iter()
                        .find(|tab| tab.id == i64::from(id))
                        .map(|tab| tab.message)
                })
            };
            if let Some(message) = message.and_then(|id| object_table(lua, id).ok().flatten()) {
                message.call_method::<()>("Clear", ())?;
                return Ok(boolean(true));
            }
            Ok(boolean(false))
        })?,
    )?;
    let s = state.clone();
    api.raw_set(
        "GetChatOption",
        lua.create_function(move |lua, _: MultiValue| {
            ensure(lua, &s)?;
            let out = lua.create_table()?;
            let [over, leave] = s.borrow().chat_config.options;
            out.raw_set("overAlpha", f64::from(over))?;
            out.raw_set("leaveAlpha", f64::from(leave))?;
            Ok(out)
        })?,
    )?;
    let s = state.clone();
    api.raw_set(
        "UpdateChatOption",
        lua.create_function(move |lua, args: MultiValue| {
            let Some(Value::Table(input)) = args.get(1) else { return Ok(()) };
            ensure(lua, &s)?;
            let mut options = s.borrow().chat_config.options;
            for (i, key) in ["overAlpha", "leaveAlpha"].into_iter().enumerate() {
                if let Some(value) = numeric(&input.raw_get::<Value>(key)?) {
                    options[i] = value;
                }
            }
            s.borrow_mut().chat_config.options = options;
            save(lua, &s, "options");
            aa_ui::lua::chat::set_alpha_options(lua, options[0], options[1]);
            Ok(())
        })?,
    )?;
    Ok(())
}

fn track(mut events: MessageReader<NetEvent>, state: NonSend<HudBridge>, rt: Option<NonSend<UiRuntime>>) {
    for NetEvent(event) in events.read() {
        let mut data = state.0.borrow_mut();
        let view = &mut data.chat_config;
        match event {
            Event::ReadyToConnect { world_id, .. } => view.identity(Some(*world_id), 0),
            Event::CharacterSelected(selected) => view.identity(view.world, selected.character.id),
            Event::PlayerSpawn(spawn) => view.identity(view.world, spawn.character().id),
            Event::LoginSucceeded(_) | Event::Disconnected { .. } | Event::Kicked { .. } => view.identity(None, 0),
            Event::LeftWorld { .. } => view.identity(view.world, 0),
            _ => {}
        }
    }
    let cleaned = state.0.borrow_mut().chat_config.model.clean_empty();
    if cleaned && let Some(rt) = rt {
        save(&rt.lua, &state.0, "extra");
    }
}
pub(in crate::hud) fn plugin(app: &mut App) {
    super::ui_data::plugin(app);
    app.add_systems(
        Update,
        track
            .before(super::super::sync_hud)
            .before(super::super::prebuild_hud)
            .before(super::super::build_hud),
    )
    .add_systems(
        Update,
        locks::sync
            .after(super::super::sync_hud)
            .after(super::super::build_hud)
            .before(aa_ui::UiFrame),
    );
}
