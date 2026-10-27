//! Original default_binding.g records. Options and gameplay share these identities.
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use mlua::{Lua, Table, Value};
use serde_json::{Value as Json, json};

#[path = "bindings/fixture.rs"]
pub(super) mod fixture;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub manager: u8,
    pub mode: String,
    pub action: String,
    /// Original one-based builtin_multi index; ordinary actions use zero.
    pub index: u32,
    pub key: String,
    /// Native builtin0, registered button2, indexed builtin5.
    pub kind: u8,
}

/// Native393fc400 prints modifiers in this order. Key names themselves are lowercase.
pub fn chord(text: &str) -> Option<String> {
    let mut mods = BTreeSet::new();
    let mut key = text;
    while let Some((prefix, tail)) = key.split_once('-') {
        let prefix = prefix.to_ascii_uppercase();
        if matches!(prefix.as_str(), "CTRL" | "ALT" | "SHIFT") {
            mods.insert(prefix);
            key = tail;
        } else {
            break;
        }
    }
    if key.is_empty() || key == "-" || key.len() > 64 || !key.chars().all(|c| c.is_ascii_graphic()) {
        return None;
    }
    let mut result = String::new();
    for modifier in ["CTRL", "ALT", "SHIFT"] {
        if mods.contains(modifier) {
            result.push_str(modifier);
            result.push('-');
        }
    }
    result.push_str(&key.to_ascii_lowercase());
    Some(result)
}

pub fn parse_defaults(source: &str) -> Result<Vec<Binding>, String> {
    let mut result = Vec::new();
    let mut manager = 0;
    let mut mode = "";
    let mut multi = false;
    let mut action = "";
    for (line_number, line) in source.trim_start_matches('\u{feff}').lines().enumerate() {
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let fail = || format!("default_binding.g line {}: unsupported record", line_number + 1);
        if indent == 0 {
            manager = match text {
                "primary" => 1,
                "second" => 2,
                "removed" | "version 2" => 0,
                _ => return Err(fail()),
            };
            continue;
        }
        if manager == 0 {
            return Err(fail());
        }
        if indent == 4 {
            if !matches!(
                text,
                "normal_mode" | "mode_action_mode" | "builder_rotate_mode" | "screen_shot_mode" | "quest_directing_mode"
            ) {
                return Err(fail());
            }
            mode = text;
            continue;
        }
        if indent == 8 {
            multi = match text {
                "builtin" => false,
                "builtin_multi" => true,
                _ => return Err(fail()),
            };
            action = "";
            continue;
        }
        if mode.is_empty() {
            return Err(fail());
        }
        let mut add = |key: &str, action: &str, index: u32| -> Result<(), String> {
            if action.is_empty() || !action.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err(fail());
            }
            result.push(Binding {
                manager,
                mode: mode.to_owned(),
                action: action.to_owned(),
                index,
                key: chord(key).ok_or_else(fail)?,
                kind: if index > 0 { 5 } else { 0 },
            });
            Ok(())
        };
        if !multi && indent == 12 {
            let parts: Vec<_> = text.split_whitespace().collect();
            if parts.len() != 2 {
                return Err(fail());
            }
            add(parts[0], parts[1], 0)?;
        } else if multi && indent == 12 {
            if let Some((name, entries)) = text.split_once('(') {
                let entries = entries.strip_suffix(')').ok_or_else(fail)?;
                for entry in entries.split(',') {
                    let parts: Vec<_> = entry.split_whitespace().collect();
                    if parts.len() != 2 {
                        return Err(fail());
                    }
                    let index = parts[1].parse::<u32>().ok().filter(|i| *i > 0).ok_or_else(fail)?;
                    add(parts[0], name.trim(), index)?;
                }
                action = "";
            } else {
                action = text;
            }
        } else if multi && indent == 16 && !action.is_empty() {
            let parts: Vec<_> = text.split_whitespace().collect();
            if parts.len() != 2 {
                return Err(fail());
            }
            let index = parts[1].parse::<u32>().ok().filter(|i| *i > 0).ok_or_else(fail)?;
            add(parts[0], action, index)?;
        } else {
            return Err(fail());
        }
    }
    Ok(result)
}

pub fn load_defaults() -> anyhow::Result<Vec<Binding>> {
    let bytes = aa_pak::Pak::global().read("game/default_binding.g")?;
    parse_defaults(std::str::from_utf8(&bytes)?).map_err(anyhow::Error::msg)
}

pub(super) struct Keymap {
    defaults: Vec<Binding>,
    pub(super) live: Vec<Binding>,
    draft: Vec<Binding>,
    valid: BTreeSet<String>,
    pub(super) enabled: bool,
}

#[derive(Clone)]
pub(super) struct Context(pub Arc<Mutex<Keymap>>);

impl Keymap {
    pub(super) fn new(saved: Option<&Json>) -> anyhow::Result<Self> {
        let defaults = load_defaults()?;
        let catalog: Json = serde_json::from_str(include_str!("native-actions.json"))?;
        let valid = catalog["actions"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Json::as_str)
            .map(str::to_owned)
            .collect();
        let mut map = Self {
            live: defaults.clone(),
            draft: defaults.clone(),
            defaults,
            valid,
            enabled: true,
        };
        if let Some(records) = saved.filter(|s| s["version"] == 2).and_then(|s| s["records"].as_array()) {
            for value in records.iter().take(4096) {
                let Some(action) = value["action"].as_str() else { continue };
                let Some(manager) = value["manager"].as_u64().and_then(|n| u8::try_from(n).ok()).filter(|n| (1..=2).contains(n)) else {
                    continue;
                };
                let Some(index) = value["index"].as_u64().and_then(|n| u32::try_from(n).ok()).filter(|n| *n <= i32::MAX as u32) else {
                    continue;
                };
                let Some(kind) = value["kind"].as_u64().and_then(|n| u8::try_from(n).ok()).filter(|n| matches!(n, 0 | 2 | 5)) else {
                    continue;
                };
                let Some(key) = value["key"].as_str() else { continue };
                if !map.valid_identity(action, kind, index) {
                    continue;
                }
                let key = if key.is_empty() {
                    String::new()
                } else if let Some(key) = chord(key) {
                    key
                } else {
                    continue;
                };
                map.assign(action, manager, index, kind, key);
            }
            map.live = map.draft.clone();
        }
        Ok(map)
    }

    fn valid_identity(&self, action: &str, kind: u8, index: u32) -> bool {
        if kind == 2 {
            return index == 0 && !action.is_empty() && action.len() <= 256 && action.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.');
        }
        self.valid.contains(action) && ((kind == 0 && index == 0) || (kind == 5 && index > 0))
    }

    fn mode(&self, action: &str, kind: u8) -> String {
        if kind == 2 {
            return "normal_mode".into();
        }
        self.defaults
            .iter()
            .find(|b| b.action == action)
            .map_or("normal_mode", |b| b.mode.as_str())
            .to_owned()
    }

    fn lookup(&self, draft: bool, action: &str, manager: u8, index: u32, kind: u8) -> String {
        let records = if draft { &self.draft } else { &self.live };
        records
            .iter()
            .find(|b| b.action == action && b.manager == manager && b.index == index && b.kind == kind)
            .map(|b| b.key.clone())
            .unwrap_or_default()
    }

    fn assign(&mut self, action: &str, manager: u8, index: u32, kind: u8, key: String) -> Option<Option<String>> {
        if self.lookup(true, action, manager, index, kind) == key {
            return None;
        }
        let mode = self.mode(action, kind);
        let mut displaced = None;
        for record in &mut self.draft {
            if record.action == action && record.manager == manager && record.index == index && record.kind == kind {
                record.key.clear();
            } else if !key.is_empty() && record.mode == mode && record.key == key {
                displaced = Some(record.action.clone());
                record.key.clear();
            }
        }
        if let Some(record) = self
            .draft
            .iter_mut()
            .find(|b| b.action == action && b.manager == manager && b.index == index && b.kind == kind)
        {
            record.key = key;
        } else {
            self.draft.push(Binding {
                manager,
                mode,
                action: action.to_owned(),
                index,
                key,
                kind,
            });
        }
        Some(displaced)
    }

    fn save_value(&self) -> Json {
        json!({"version":2,"records":self.live.iter().map(|b| json!({"manager":b.manager,"action":b.action,"index":b.index,"kind":b.kind,"key":b.key})).collect::<Vec<_>>()})
    }
}

pub(super) fn map(store: &Arc<Mutex<super::Store>>) -> mlua::Result<Arc<Mutex<Keymap>>> {
    let mut s = store.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(map) = &s.bindings {
        return Ok(map.clone());
    }
    let map = Arc::new(Mutex::new(Keymap::new(s.data.get("bindings")).map_err(mlua::Error::external)?));
    s.bindings = Some(map.clone());
    Ok(map)
}

pub(super) fn install(lua: &Lua, store: Arc<Mutex<super::Store>>) -> mlua::Result<()> {
    let map = map(&store)?;
    lua.set_app_data(Context(map.clone()));
    map.lock().unwrap_or_else(|e| e.into_inner()).enabled = true;
    let table: Table = lua.globals().get("X2Hotkey")?;
    for (name, draft, button) in [
        ("GetBinding", false, false),
        ("GetBindingButton", false, true),
        ("GetOptionBindingButton", true, true),
    ] {
        let map = map.clone();
        table.set(
            name,
            lua.create_function(move |_, (_, action, manager): (Table, String, i64)| {
                let Ok(manager) = u8::try_from(manager) else { return Ok(String::new()) };
                Ok(map
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .lookup(draft, &action, manager, 0, if button { 2 } else { 0 }))
            })?,
        )?;
    }
    let m = map.clone();
    table.set(
        "GetOptionBinding",
        lua.create_function(move |_, (_, action, manager, draft, index): (Table, String, i64, Option<bool>, Option<i64>)| {
            let Ok(manager) = u8::try_from(manager) else { return Ok(String::new()) };
            let index = index.unwrap_or(0);
            let index = if index > 0 { u32::try_from(index).ok() } else { Some(0) };
            Ok(index
                .map(|index| {
                    m.lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .lookup(draft.unwrap_or(false), &action, manager, index, if index > 0 { 5 } else { 0 })
                })
                .unwrap_or_default())
        })?,
    )?;
    let m = map.clone();
    table.set(
        "IsValidActionName",
        lua.create_function(move |_, (_, action): (Table, String)| Ok(m.lock().unwrap_or_else(|e| e.into_inner()).valid.contains(&action)))?,
    )?;
    let m = map.clone();
    table.set(
        "IsOverridableAction",
        lua.create_function(move |_, (_, action): (Table, String)| {
            // Authored non-normal groups are the available mode evidence. Exact membership
            // of native's runtime-encoded override table remains an explicit audit gap.
            let m = m.lock().unwrap_or_else(|e| e.into_inner());
            Ok(m.valid.contains(&action) && m.mode(&action, 0) != "normal_mode")
        })?,
    )?;
    for (name, button) in [("SetOptionBindingWithIndex", false), ("SetOptionBindingButtonWithIndex", true)] {
        let m = map.clone();
        table.set(
            name,
            lua.create_function(move |lua, (_, action, key, manager, index): (Table, String, String, i64, Option<i64>)| {
                let Some(manager) = u8::try_from(manager).ok().filter(|n| (1..=2).contains(n)) else {
                    return Ok(());
                };
                let index = if button { 0 } else { index.unwrap_or(0).max(0) };
                let Some(index) = u32::try_from(index).ok().filter(|i| *i <= i32::MAX as u32) else {
                    return Ok(());
                };
                let kind = if button {
                    2
                } else if index > 0 {
                    5
                } else {
                    0
                };
                let Some(key) = chord(&key) else { return Ok(()) };
                let changed = {
                    let mut m = m.lock().unwrap_or_else(|e| e.into_inner());
                    if !m.valid_identity(&action, kind, index) {
                        return Ok(());
                    }
                    m.assign(&action, manager, index, kind, key)
                };
                if let Some(displaced) = changed {
                    let old = match displaced {
                        Some(ref name) => Value::String(lua.create_string(name)?),
                        None => Value::Nil,
                    };
                    aa_ui::lua::fire_ui_event(lua, "UPDATE_OPTION_BINDINGS", vec![Value::Boolean(displaced.is_some()), old]);
                }
                Ok(())
            })?,
        )?;
    }
    let m = map.clone();
    table.set(
        "RemoveOptionBinding",
        lua.create_function(move |lua, (_, action, manager, index): (Table, String, i64, Option<i64>)| {
            let Some(manager) = u8::try_from(manager).ok().filter(|n| (1..=2).contains(n)) else {
                return Ok(());
            };
            let index = index.unwrap_or(0).max(0);
            let Some(index) = u32::try_from(index).ok() else { return Ok(()) };
            {
                let mut m = m.lock().unwrap_or_else(|e| e.into_inner());
                for binding in &mut m.draft {
                    if binding.action == action && binding.manager == manager && binding.index == index {
                        binding.key.clear();
                    }
                }
            }
            aa_ui::lua::fire_ui_event(lua, "UPDATE_OPTION_BINDINGS", vec![]);
            Ok(())
        })?,
    )?;
    for (name, reset) in [("BindingToOption", false), ("InitOptionHotKey", true)] {
        let m = map.clone();
        table.set(
            name,
            lua.create_function(move |lua, _: Table| {
                {
                    let mut m = m.lock().unwrap_or_else(|e| e.into_inner());
                    m.draft = if reset { m.defaults.clone() } else { m.live.clone() };
                }
                aa_ui::lua::fire_ui_event(lua, "UPDATE_OPTION_BINDINGS", vec![]);
                Ok(())
            })?,
        )?;
    }
    let m = map.clone();
    table.set(
        "EnableHotkey",
        lua.create_function(move |_, (_, on): (Table, bool)| {
            m.lock().unwrap_or_else(|e| e.into_inner()).enabled = on;
            Ok(())
        })?,
    )?;
    table.set(
        "SaveHotKey",
        lua.create_function(move |lua, _: Table| {
            let value = {
                let mut m = map.lock().unwrap_or_else(|e| e.into_inner());
                m.live = m.draft.clone();
                m.save_value()
            };
            aa_ui::lua::fire_ui_event(lua, "UPDATE_BINDINGS", vec![]);
            let mut s = store.lock().unwrap_or_else(|e| e.into_inner());
            if s.data.get("bindings") != Some(&value) {
                s.data.insert("bindings".into(), value);
                s.dirty = true;
            }
            if let Err(e) = s.save() {
                bevy::log::warn!("settings: hotkey save: {e:#}");
            }
            Ok(())
        })?,
    )?;
    lua.load(
        r#"
        local previous=AAUI.SlotHotKey
        AAUI.SlotHotKey=function(kind,index)
          if kind==ISLOT_ACTION or kind==ISLOT_MODE_ACTION then
            local action=kind==ISLOT_ACTION and 'action_bar_button' or 'mode_action_bar_button'
            local primary=X2Hotkey:GetOptionBinding(action,1,false,index)
            if primary=='' then primary=X2Hotkey:GetOptionBinding(action,2,false,index) end
            return string.upper(primary)
          end
          if previous then return previous(kind,index) end
        end
    "#,
    )
    .set_name("@native_action_bindings")
    .exec()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authored_defaults_keep_modes_managers_and_native_indexing() -> anyhow::Result<()> {
        let records = load_defaults()?;
        let get = |manager, mode: &str, action: &str, index| {
            records
                .iter()
                .find(|b| b.manager == manager && b.mode == mode && b.action == action && b.index == index)
                .map(|b| b.key.as_str())
        };
        assert_eq!(get(1, "normal_mode", "moveleft", 0), Some("q"));
        assert_eq!(get(1, "normal_mode", "turnleft", 0), Some("a"));
        assert_eq!(get(2, "normal_mode", "moveforward", 0), Some("up"));
        assert_eq!(get(1, "normal_mode", "toggle_post", 0), Some("CTRL-e"));
        assert_eq!(get(1, "normal_mode", "action_bar_button", 13), Some("SHIFT-1"));
        assert_eq!(get(1, "normal_mode", "action_bar_page_prev", 0), Some("SHIFT-up"));
        assert_eq!(get(1, "normal_mode", "action_bar_page_next", 0), Some("SHIFT-down"));
        assert_eq!(get(1, "mode_action_mode", "mode_action_bar_button", 1), Some("r"));
        assert_eq!(get(1, "builder_rotate_mode", "builder_rotate_left_large", 0), Some("SHIFT-wheelup"));
        assert_eq!(get(1, "quest_directing_mode", "quest_directing_interaction", 3), Some("g"));
        assert_eq!(chord("SHIFT-CTRL-ALT-TAB"), Some("CTRL-ALT-SHIFT-tab".into()));
        assert_eq!(chord("CTRL--"), None);
        assert_eq!(chord("SHIFT-number-"), Some("SHIFT-number-".into()));
        assert_eq!(chord("SHIFT"), Some("shift".into()));
        Ok(())
    }

    #[test]
    fn native_draft_conflicts_indexed_keys_and_removed_defaults_survive_restart() -> Result<(), Box<dyn std::error::Error>> {
        let path = std::env::temp_dir().join(format!("aa-client-binding-state-{}.json", std::process::id()));
        let store = Arc::new(Mutex::new(super::super::Store::open(path.clone())));
        // Use a fresh file even after a prior interrupted probe.
        store.lock().unwrap().data.remove("bindings");
        let rt = aa_ui::UiRuntime::new("en_us", 1280.0, 720.0)?;
        rt.lua.globals().set("X2Hotkey", rt.lua.create_table()?)?;
        install(&rt.lua, store.clone())?;
        rt.lua
            .load(
                r#"
            events={}
            UIParent:SetEventHandler('UPDATE_OPTION_BINDINGS',function(...) events[#events+1]={n=select('#',...),...} end)
            assert(X2Hotkey:GetBinding('moveforward',1)=='w' and X2Hotkey:GetBinding('moveforward',2)=='up')
            assert(X2Hotkey:GetBinding('never_an_action',1)=='')
            assert(X2Hotkey:IsValidActionName('moveforward') and not X2Hotkey:IsValidActionName('MOVEFORWARD'))
            assert(not X2Hotkey:IsValidActionName('PetCommandButton1'))
            X2Hotkey:BindingToOption()
            assert(events[#events].n==0)
            X2Hotkey:SetOptionBindingWithIndex('moveforward','SHIFT-CTRL-ALT-TAB',1,0)
            assert(X2Hotkey:GetBinding('moveforward',1)=='w')
            assert(X2Hotkey:GetOptionBinding('moveforward',1,true,0)=='CTRL-ALT-SHIFT-tab')
            assert(X2Hotkey:GetOptionBinding('moveforward',1,false,0)=='w')
            X2Hotkey:SetOptionBindingWithIndex('moveforward','s',1,0)
            assert(events[#events].n==2 and events[#events][1]==true and events[#events][2]=='moveback')
            assert(X2Hotkey:GetOptionBinding('moveback',1,true,0)=='')
            local count=#events
            X2Hotkey:SetOptionBindingWithIndex('moveforward','s',1,0)
            assert(#events==count)
            X2Hotkey:SetOptionBindingWithIndex('moveright','s',2,0)
            assert(events[#events][2]=='moveforward' and X2Hotkey:GetOptionBinding('moveforward',1,true,0)=='')
            X2Hotkey:SetOptionBindingWithIndex('action_bar_button','j',1,13)
            assert(X2Hotkey:GetOptionBinding('action_bar_button',1,true,13)=='j')
            assert(events[#events][2]=='do_interaction_4')
            X2Hotkey:RemoveOptionBinding('action_bar_button',1,13)
            X2Hotkey:SetOptionBindingButtonWithIndex('PetCommandButton1','CTRL-p',2)
            assert(X2Hotkey:GetOptionBindingButton('PetCommandButton1',2)=='CTRL-p')
            assert(X2Hotkey:GetBindingButton('PetCommandButton1',2)=='')
            X2Hotkey:EnableHotkey(false)
            X2Hotkey:SaveHotKey()
            assert(X2Hotkey:GetBinding('moveforward',1)=='')
            assert(X2Hotkey:GetBinding('moveright',2)=='s')
            assert(X2Hotkey:GetBindingButton('PetCommandButton1',2)=='CTRL-p')
        "#,
            )
            .exec()?;
        let state = map(&store)?;
        assert!(!state.lock().unwrap().enabled);
        let restored = Arc::new(Mutex::new(super::super::Store::open(path.clone())));
        let next = aa_ui::UiRuntime::new("en_us", 1280.0, 720.0)?;
        next.lua.globals().set("X2Hotkey", next.lua.create_table()?)?;
        install(&next.lua, restored)?;
        next.lua
            .load(
                r#"
            assert(X2Hotkey:GetBinding('moveforward',1)=='')
            assert(X2Hotkey:GetBinding('moveright',2)=='s')
            assert(X2Hotkey:GetOptionBinding('action_bar_button',1,false,13)=='')
            assert(X2Hotkey:GetOptionBinding('action_bar_button',1,false,1)=='1')
            assert(X2Hotkey:GetBindingButton('PetCommandButton1',2)=='CTRL-p')
            X2Hotkey:InitOptionHotKey()
            assert(X2Hotkey:GetOptionBinding('moveforward',1,true,0)=='w')
            assert(X2Hotkey:GetBinding('moveforward',1)=='')
            X2Hotkey:BindingToOption()
            assert(X2Hotkey:GetOptionBinding('moveforward',1,true,0)=='')
        "#,
            )
            .exec()?;
        std::fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn defaults_in_other_modes_are_not_displaced_by_normal_mode_changes() -> anyhow::Result<()> {
        let mut state = Keymap::new(None)?;
        assert_eq!(state.assign("moveforward", 1, 0, 0, "r".into()), Some(None));
        assert_eq!(state.lookup(true, "mode_action_bar_button", 1, 1, 5), "r");
        assert_eq!(state.assign("moveright", 2, 0, 0, "r".into()), Some(Some("moveforward".into())));
        assert_eq!(state.lookup(true, "mode_action_bar_button", 1, 1, 5), "r");
        Ok(())
    }

    #[test]
    #[ignore = "loads the original world-stage assets; run in an isolated fixture process"]
    fn original_options_rebind_save_cancel_reset_and_action_dispatch() -> Result<(), Box<dyn std::error::Error>> {
        use bevy::input::keyboard::{Key, KeyboardInput};
        use bevy::input::{ButtonState, InputPlugin};
        use bevy::prelude::*;

        let rt = crate::hud::keybindings_runtime(1920.0, 1080.0)?;
        let path = fixture::prepare(&rt)?;
        rt.lua
            .load("assert(BINDING_PRIMARY:GetText()=='W'); BINDING_PRIMARY:OnClick('LeftButton')")
            .exec()?;
        assert!(rt.keyboard_captured());
        rt.key_event(KeyCode::KeyP, true, None);
        rt.key_event(KeyCode::KeyP, false, None);
        rt.lua.load("assert(BINDING_PRIMARY:GetText()=='P'); assert(X2Hotkey:GetBinding('moveforward',1)=='w'); optionFrame:Save(); assert(X2Hotkey:GetBinding('moveforward',1)=='p')").exec()?;
        fixture::open(&rt)?;
        rt.lua
            .load("assert(BINDING_PRIMARY:GetText()=='P'); BINDING_PRIMARY:OnClick('LeftButton')")
            .exec()?;
        rt.key_event(KeyCode::KeyO, true, None);
        rt.key_event(KeyCode::KeyO, false, None);
        rt.lua
            .load("assert(BINDING_PRIMARY:GetText()=='O'); optionFrame:Cancel(); ToggleOptionFrame(false); assert(X2Hotkey:GetBinding('moveforward',1)=='p')")
            .exec()?;
        fixture::open(&rt)?;
        rt.lua.load("assert(BINDING_PRIMARY:GetText()=='P'); optionFrame.contentWindow.resetBtn:OnClick('LeftButton'); assert(BINDING_PRIMARY:GetText()=='W'); assert(X2Hotkey:GetBinding('moveforward',1)=='p'); optionFrame:Cancel(); ToggleOptionFrame(false)").exec()?;
        aa_ui::lua::keyboard::begin_frame(&rt.lua);
        assert!(!rt.keyboard_captured());
        let saved = super::super::Store::open(path.clone());
        let restored = Keymap::new(saved.data.get("bindings"))?;
        assert_eq!(restored.lookup(false, "moveforward", 1, 0, 0), "p");
        let bridge = std::rc::Rc::new(std::cell::RefCell::new(crate::hud::api::HudData::default()));
        let mut app = App::new();
        app.add_plugins(InputPlugin)
            .insert_non_send(rt)
            .insert_non_send(crate::hud::api::HudBridge(bridge))
            .init_resource::<crate::settings::Actions>()
            .init_resource::<crate::hud::HudPointerOverUi>()
            .add_systems(Update, crate::settings::update_input);
        let send = |app: &mut App, code: KeyCode, down| {
            app.world_mut().write_message(KeyboardInput {
                key_code: code,
                logical_key: Key::Character("x".into()),
                state: if down { ButtonState::Pressed } else { ButtonState::Released },
                text: None,
                repeat: false,
                window: Entity::PLACEHOLDER,
            });
            app.update();
        };
        send(&mut app, KeyCode::KeyP, true);
        assert!(app.world().resource::<crate::settings::Actions>().pressed("moveforward", 0));
        send(&mut app, KeyCode::KeyP, false);
        send(&mut app, KeyCode::KeyW, true);
        assert!(!app.world().resource::<crate::settings::Actions>().pressed("moveforward", 0));
        assert!(app.world().non_send::<aa_ui::UiRuntime>().errors.is_empty());
        std::fs::remove_file(path)?;
        Ok(())
    }
}
