//! The character's saved UI data: `X2:GetCharacterUiData(key)`, `X2:SetCharacterUiData(key, value)` and
//! `X2:SaveCharacterUiData()`.
//!
//! The original scripts keep what the player arranged here: every window that calls
//! `AddUISaveHandlerByKey` (the chat windows `chatWindow[N]`, the character window, the bag ...) saves its
//! bound as `ui_bound_<key>` when it is moved or resized and applies it again when the interface is built,
//! `ui_management.lua` keeps its option flags as `uiOptionDatas`, the action bar its layout. The values are
//! Lua tables of numbers, strings and booleans.
//!
//! They are kept per world and character in the account settings file, next to the chat records
//! (`chat[<world>:<character>].ui`, the same key as [`super::config`]), and written a moment after the
//! last change (the scripts rarely call the save) and when the game ends. Without a genuine character
//! identity (offline fixtures) nothing is read or written; the value set only lives for the session, and
//! the reference character's quest notifier position of the fake stays as the fallback for that one key.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use bevy::prelude::*;
use mlua::{Function, Lua, Table, Value};
use serde_json::{Map, Value as Json, json};

use super::super::api::{HudBridge, Shared};

/// Marks an integer key of a table that is not a sequence: `"\u{1}i:7"`.
const INT_KEY: &str = "\u{1}i:";
/// Seconds after the last change before the file is written.
const WRITE_AFTER: f32 = 2.0;
/// Nesting beyond this is not a table of the scripts' (and a cycle never ends).
const DEPTH: usize = 12;

/// The values set or read this session, by key.
#[derive(Default)]
struct Store {
    values: BTreeMap<String, Json>,
    /// The character record was read (or there is nothing to read) for this key.
    loaded_for: Option<String>,
    /// Seconds since the last change; `None`: nothing to write.
    dirty: Option<f32>,
}

#[derive(Clone, Default)]
struct UiStore(Rc<RefCell<Store>>);

fn to_json(value: &Value, depth: usize) -> Option<Json> {
    Some(match value {
        Value::Boolean(b) => Json::Bool(*b),
        Value::Integer(n) => json!(*n),
        Value::Number(n) => json!(*n),
        Value::String(s) => Json::String(s.to_str().ok()?.to_string()),
        Value::Table(t) if depth < DEPTH => {
            let len = t.raw_len();
            let pairs: Vec<(Value, Value)> = t.pairs::<Value, Value>().flatten().collect();
            if len > 0 && pairs.len() == len {
                Json::Array(pairs_in_order(t, len, depth)?)
            } else {
                let mut map = Map::new();
                for (k, v) in pairs {
                    let key = match k {
                        Value::String(s) => s.to_str().ok()?.to_string(),
                        Value::Integer(n) => format!("{INT_KEY}{n}"),
                        Value::Number(n) if n.fract() == 0.0 => format!("{INT_KEY}{}", n as i64),
                        _ => continue,
                    };
                    if let Some(v) = to_json(&v, depth + 1) {
                        map.insert(key, v);
                    }
                }
                Json::Object(map)
            }
        }
        _ => return None,
    })
}

fn pairs_in_order(t: &Table, len: usize, depth: usize) -> Option<Vec<Json>> {
    let mut out = Vec::with_capacity(len);
    for i in 1..=len {
        out.push(to_json(&t.raw_get::<Value>(i).ok()?, depth + 1)?);
    }
    Some(out)
}

fn to_lua(lua: &Lua, value: &Json) -> mlua::Result<Value> {
    Ok(match value {
        Json::Null => Value::Nil,
        Json::Bool(b) => Value::Boolean(*b),
        Json::Number(n) => match n.as_i64() {
            Some(i) => Value::Integer(i),
            None => Value::Number(n.as_f64().unwrap_or(0.0)),
        },
        Json::String(s) => Value::String(lua.create_string(s)?),
        Json::Array(items) => {
            let t = lua.create_table()?;
            for (i, v) in items.iter().enumerate() {
                t.raw_set(i + 1, to_lua(lua, v)?)?;
            }
            Value::Table(t)
        }
        Json::Object(map) => {
            let t = lua.create_table()?;
            for (k, v) in map {
                match k.strip_prefix(INT_KEY).and_then(|n| n.parse::<i64>().ok()) {
                    Some(n) => t.raw_set(n, to_lua(lua, v)?)?,
                    None => t.raw_set(k.as_str(), to_lua(lua, v)?)?,
                }
            }
            Value::Table(t)
        }
    })
}

/// Reads the saved record of the character once its identity is known.
fn load(lua: &Lua, bridge: &Shared, store: &mut Store) {
    load_for(lua, store, bridge.borrow().chat_config.character_key());
}

fn load_for(lua: &Lua, store: &mut Store, key: Option<String>) {
    if store.loaded_for == key {
        return;
    }
    if store.loaded_for.is_some() {
        // A queued save still belongs to the previous character, even after the bridge changed.
        if store.dirty.is_some() {
            write(lua, store);
        }
        store.values.clear();
        store.dirty = None;
    }
    let Some(key) = key else {
        store.loaded_for = None;
        return;
    };
    store.loaded_for = Some(key.clone());
    // (values set before the identity was known stay, the saved ones fill in the rest)
    if let Some(Json::Object(saved)) = crate::settings::chat_read(lua, &key, "ui") {
        for (k, v) in saved {
            store.values.entry(k).or_insert(v);
        }
    }
}

/// Writes the values of the character.
fn write(lua: &Lua, store: &mut Store) {
    let Some(key) = store.loaded_for.as_deref() else { return };
    let value = Json::Object(store.values.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
    if crate::settings::chat_save(lua, key, "ui", value) {
        store.dirty = None;
    }
}

pub(in crate::hud) fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    let shared = UiStore::default();
    lua.set_app_data(shared.clone());
    let x2: Table = lua.globals().get("X2")?;
    // (the fake file's answer for what the session has not set)
    let fallback: Option<Function> = x2.get("GetCharacterUiData").ok();
    let (s, b) = (shared.clone(), bridge.clone());
    x2.set(
        "GetCharacterUiData",
        lua.create_function(move |lua, (this, key): (Value, Value)| {
            let Value::String(name) = &key else { return Ok(Value::Nil) };
            let name = name.to_str()?.to_string();
            let mut store = s.0.borrow_mut();
            load(lua, &b, &mut store);
            if let Some(saved) = store.values.get(&name) {
                return to_lua(lua, saved);
            }
            drop(store);
            match &fallback {
                Some(f) => f.call((this, key)),
                None => Ok(Value::Nil),
            }
        })?,
    )?;
    let (s, b) = (shared.clone(), bridge.clone());
    x2.set(
        "SetCharacterUiData",
        lua.create_function(move |lua, (_, key, value): (Value, Value, Value)| {
            let Value::String(name) = &key else { return Ok(()) };
            let name = name.to_str()?.to_string();
            let mut store = s.0.borrow_mut();
            load(lua, &b, &mut store);
            match to_json(&value, 0) {
                Some(json) => {
                    if store.values.get(&name) != Some(&json) {
                        store.values.insert(name, json);
                        store.dirty = Some(0.0);
                    }
                }
                None if matches!(value, Value::Nil) => {
                    if store.values.remove(&name).is_some() {
                        store.dirty = Some(0.0);
                    }
                }
                None => {}
            }
            Ok(())
        })?,
    )?;
    let (s, b) = (shared, bridge.clone());
    x2.set(
        "SaveCharacterUiData",
        lua.create_function(move |lua, _: mlua::MultiValue| {
            let mut store = s.0.borrow_mut();
            load(lua, &b, &mut store);
            write(lua, &mut store);
            Ok(())
        })?,
    )
}

/// Writes the changes a moment after the last one, and when the game ends.
fn flush(rt: Option<NonSend<aa_ui::UiRuntime>>, bridge: NonSend<HudBridge>, time: Res<Time>) {
    let Some(rt) = rt else { return };
    let Some(shared) = rt.lua.app_data_ref::<UiStore>().map(|s| s.clone()) else { return };
    let mut store = shared.0.borrow_mut();
    load(&rt.lua, &bridge.0, &mut store);
    let Some(waited) = store.dirty.as_mut() else { return };
    *waited += time.delta_secs();
    if *waited >= WRITE_AFTER {
        write(&rt.lua, &mut store);
    }
}

/// The game ends: whatever is waiting is written now.
fn flush_now(rt: Option<NonSend<aa_ui::UiRuntime>>) {
    let Some(rt) = rt else { return };
    let Some(shared) = rt.lua.app_data_ref::<UiStore>().map(|s| s.clone()) else { return };
    let mut store = shared.0.borrow_mut();
    if store.dirty.is_some() {
        write(&rt.lua, &mut store);
    }
}

pub(in crate::hud) fn plugin(app: &mut App) {
    app.add_systems(Update, flush.run_if(super::super::hud_up)).add_systems(bevy::app::Last, flush_now.run_if(on_message::<bevy::app::AppExit>));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_survive_the_trip_through_json() -> mlua::Result<()> {
        let lua = Lua::new();
        let value: Value = lua
            .load(r#"return { bound = { x = 12, y = 30.5, width = 430, height = 350 }, screenResolution = { x = 1920, y = 1080 }, list = { 3, 2, 1 }, name = "a", on = true, [7] = "seven" }"#)
            .eval()?;
        let json = to_json(&value, 0).unwrap();
        assert_eq!(json["bound"]["y"], json!(30.5));
        assert_eq!(json["list"], json!([3, 2, 1]));
        let back = to_lua(&lua, &json)?;
        lua.globals().set("back", back)?;
        lua.load(r#"assert(back.bound.x == 12 and back.bound.y == 30.5 and back.list[3] == 1 and #back.list == 3 and back.name == "a" and back.on == true and back[7] == "seven")"#).exec()?;
        Ok(())
    }

    #[test]
    fn functions_and_cycles_are_not_stored() -> mlua::Result<()> {
        let lua = Lua::new();
        let f: Value = lua.load("return function() end").eval()?;
        assert!(to_json(&f, 0).is_none());
        let cycle: Value = lua.load("local t = {}; t.self = t; return t").eval()?;
        // (the table is stored without the part that could not be: nothing loops)
        assert!(to_json(&cycle, 0).is_some());
        Ok(())
    }

    #[test]
    fn switching_characters_saves_the_old_owner_and_reloads_only_the_new_owner() -> Result<(), Box<dyn std::error::Error>> {
        let rt = aa_ui::UiRuntime::new("en_us", 1280.0, 720.0)?;
        rt.lua.globals().set("NET", rt.lua.create_table()?)?;
        let path = std::env::temp_dir().join(format!("aa-ui-data-owners-{}.json", std::process::id()));
        crate::settings::reopen_fixture(&rt, path.clone())?;
        let mut store = Store::default();
        load_for(&rt.lua, &mut store, Some("17:1".into()));
        store.values.insert("ui_bound_bag".into(), json!({"x":12}));
        store.dirty = Some(0.0);
        load_for(&rt.lua, &mut store, Some("17:2".into()));
        assert!(store.values.is_empty());
        assert_eq!(crate::settings::chat_read(&rt.lua, "17:1", "ui").unwrap()["ui_bound_bag"]["x"], json!(12));
        assert!(crate::settings::chat_read(&rt.lua, "17:2", "ui").is_none());
        store.values.insert("ui_bound_bag".into(), json!({"x":99}));
        store.dirty = Some(0.0);
        load_for(&rt.lua, &mut store, None);
        assert!(store.values.is_empty());
        load_for(&rt.lua, &mut store, Some("17:1".into()));
        assert_eq!(store.values["ui_bound_bag"]["x"], json!(12));
        assert_eq!(crate::settings::chat_read(&rt.lua, "17:2", "ui").unwrap()["ui_bound_bag"]["x"], json!(99));
        std::fs::remove_file(path)?;
        Ok(())
    }
}
