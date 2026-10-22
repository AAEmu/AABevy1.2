//! Account-local UI settings shared by the Lua states of all screens.
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use mlua::{Lua, Table, Value};
use serde_json::{Map, Value as Json, json};

#[path = "settings/bindings.rs"]
mod bindings;
#[path = "settings/input.rs"]
mod input;
#[path = "settings/inventory_order.rs"]
mod inventory_order;
pub use input::Actions;
pub(crate) use input::update as update_input;

pub(crate) fn bindings_fixture(rt: aa_ui::UiRuntime) {
    bindings::fixture::run(rt);
}

pub(crate) fn isolated_bindings(rt: &aa_ui::UiRuntime) -> Result<PathBuf, Box<dyn std::error::Error>> {
    bindings::fixture::isolated(rt)
}

#[derive(Default)]
struct Store {
    path: PathBuf,
    data: Map<String, Json>,
    pending: Map<String, Json>,
    dirty: bool,
    bindings: Option<Arc<Mutex<bindings::Keymap>>>,
}

impl Store {
    fn open(path: PathBuf) -> Self {
        let data = match std::fs::metadata(&path) {
            Ok(meta) if meta.len() <= 2 * 1024 * 1024 => match std::fs::read(&path)
                .map_err(anyhow::Error::from)
                .and_then(|bytes| Ok(serde_json::from_slice::<Map<String, Json>>(&bytes)?))
            {
                Ok(data) => data,
                Err(e) => {
                    bevy::log::warn!("settings: {}: {e}", path.display());
                    Map::new()
                }
            },
            Ok(_) => {
                bevy::log::warn!("settings: {} is too large", path.display());
                Map::new()
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Map::new(),
            Err(e) => {
                bevy::log::warn!("settings: {}: {e}", path.display());
                Map::new()
            }
        };
        let mut data = data;
        let dirty = stamp_options(&mut data, true);
        Self {
            path,
            data,
            pending: Map::new(),
            dirty,
            bindings: None,
        }
    }

    fn save(&mut self) -> anyhow::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        stamp_options(&mut self.data, false);
        if let Some(parent) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let temp = self
            .path
            .with_extension(format!("json.{}.tmp", std::process::id()));
        std::fs::write(&temp, serde_json::to_vec_pretty(&self.data)?)?;
        std::fs::rename(&temp, &self.path)?;
        self.dirty = false;
        Ok(())
    }
}

/// The native options whose 1.2 default is not 0, by option id. The 1.2 scripts create no item for a native option
/// (`CreateOptionItemFloat` is for the named ones): its checkbox asks `GetOptionItemValue` and gets the default of the cvar the engine
/// registered it with, and a window with nothing to show leaves the box unchecked (a slider at its lowest step) and saves that over it.
/// `use_celerity_with_double_forward` (option 55, `x2game+0x107b40`) is registered with 1: Dash starts on a double tap of forward and
/// ends when the direction keys are let go until the player turns it off.
///
/// The graphics page's options are the cvar groups of `game/config64/cvargroups/option_*.cfg`, whose `[default]` is 4 (the highest
/// level) or 1 (shadows, clouds, water reflection, HDR and DoF on); gamma is 1, the anti-aliasing entry 3 (PostAA 2x, the SMAA the
/// world camera has always had) and the camera mode 2 (classic, `cl_fov` 42.75: what the running 1.2 the camera is fitted to showed).
pub(crate) fn native_default(id: u32) -> Option<f64> {
    match id {
        3 | 17 | 23 | 26 | 27 | 28 | 55 => Some(1.0),
        7..=11 | 13..=16 | 18..=22 | 24 | 25 | 33 => Some(4.0),
        29 => Some(3.0),
        65 => Some(2.0),
        _ => None,
    }
}

/// Marks an options map written for the defaults of [`native_default`]. `Defaults1` was the stamp before the graphics options had any.
const OPTIONS_STAMP: &str = "s:AoAOptionDefaults2";
const OLD_OPTIONS_STAMP: &str = "s:AoAOptionDefaults1";

/// The graphics options (gamma, the quality sliders and check boxes, anti-aliasing): the window saved what it was shown, the lowest
/// step of every slider and an unchecked box, over values that were never set (nothing applied them then).
fn is_graphics_option(key: &str) -> bool {
    key == "s:MasterGrahicQuality" || key.strip_prefix("n:").and_then(|n| n.parse::<u32>().ok()).is_some_and(|id| id == 3 || (7..=33).contains(&id))
}

/// Stamps a file's options with [`OPTIONS_STAMP`]. A file written before the stamp may hold a 0 for an option whose default is 1 that
/// nobody chose (the Options window saved the unchecked box it was shown, see [`native_default`]): with `migrate` those are removed once,
/// so the default shows again, and so are the graphics options of a file older than the second stamp. True when the data changed.
fn stamp_options(data: &mut Map<String, Json>, migrate: bool) -> bool {
    let Some(options) = data.get_mut("options").and_then(Json::as_object_mut) else { return false };
    if options.contains_key(OPTIONS_STAMP) {
        return false;
    }
    if migrate {
        let id = |k: &str| k.strip_prefix("n:").and_then(|n| n.parse::<u32>().ok());
        let bad: Vec<String> = options
            .iter()
            .filter(|(k, v)| is_graphics_option(k) || (id(k).and_then(native_default).is_some() && v.as_f64() == Some(0.0)))
            .map(|(k, _)| k.clone())
            .collect();
        for key in bad {
            options.remove(&key);
        }
    }
    options.remove(OLD_OPTIONS_STAMP);
    options.insert(OPTIONS_STAMP.into(), json!(1));
    true
}

/// How many times the Options window saved (`X2Option:Save`) in this process: the systems that apply options compare it with the
/// count they last saw.
static SAVES: AtomicU32 = AtomicU32::new(0);

pub(crate) fn saves() -> u32 {
    SAVES.load(Ordering::Relaxed)
}

/// The screen resolutions the Options window offers, `(width, height)` ascending (the monitor's video modes, see `graphics`).
static RESOLUTIONS: Mutex<Vec<(u32, u32)>> = Mutex::new(Vec::new());

pub(crate) fn set_resolutions(list: Vec<(u32, u32)>) {
    *RESOLUTIONS.lock().unwrap_or_else(|e| e.into_inner()) = list;
}

fn path() -> PathBuf {
    if let Some(path) = std::env::var_os("AA_SETTINGS_FILE") {
        return path.into();
    }
    let args: Vec<_> = std::env::args().collect();
    let account = args
        .windows(2)
        .find(|a| matches!(a[0].as_str(), "--account" | "--user"))
        .map_or("bevytest", |a| a[1].as_str());
    // Hex encoding is unambiguous and cannot escape the settings directory.
    let account: String = account
        .to_lowercase()
        .bytes()
        .map(|b| format!("{b:02x}"))
        .collect();
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data/local"));
    base.join("AABevy1.2/settings")
        .join(format!("{account}.json"))
}

fn shared() -> Arc<Mutex<Store>> {
    static STORE: OnceLock<Arc<Mutex<Store>>> = OnceLock::new();
    STORE
        .get_or_init(|| Arc::new(Mutex::new(Store::open(path()))))
        .clone()
}

fn key(value: &Value) -> Option<String> {
    match value {
        Value::Integer(n) => Some(format!("n:{n}")),
        Value::Number(n) if n.is_finite() && n.fract() == 0.0 => Some(format!("n:{n:.0}")),
        Value::String(s) if s.as_bytes().len() <= 256 => Some(format!("s:{}", s.to_string_lossy())),
        _ => None,
    }
}

fn scalar(value: Value) -> Option<Json> {
    match value {
        Value::Boolean(b) => Some(b.into()),
        Value::Integer(n) => Some(n.into()),
        Value::Number(n) if n.is_finite() => Some(n.into()),
        Value::String(s) if s.as_bytes().len() <= 4096 => Some(s.to_string_lossy().into()),
        _ => None,
    }
}

fn lua_scalar(lua: &Lua, value: Option<&Json>) -> mlua::Result<Value> {
    Ok(match value {
        Some(Json::Bool(v)) => Value::Boolean(*v),
        Some(Json::Number(v)) => Value::Number(v.as_f64().unwrap_or_default()),
        Some(Json::String(v)) => Value::String(lua.create_string(v)?),
        _ => Value::Nil,
    })
}

/// An option's value as the store holds it (an edit not yet saved first), by the store's key: `n:55` for the native option 55,
/// `s:Name` for a named one. `None` when it was never set (the option's default applies) or is not a number.
pub(crate) fn option_number(key: &str) -> Option<f64> {
    let store = shared();
    let s = store.lock().unwrap_or_else(|e| e.into_inner());
    let value = s.pending.get(key).or_else(|| s.data.get("options")?.get(key))?;
    value.as_f64().or_else(|| value.as_bool().map(|b| if b { 1.0 } else { 0.0 }))
}

pub fn install(lua: &Lua, net: &Table) -> mlua::Result<()> {
    install_store(lua, net, shared())
}

#[derive(Clone)]
struct LuaStore(Arc<Mutex<Store>>);

fn install_store(lua: &Lua, net: &Table, store: Arc<Mutex<Store>>) -> mlua::Result<()> {
    lua.set_app_data(LuaStore(store.clone()));
    let s = store.clone();
    net.raw_set(
        "option_get",
        lua.create_function(move |lua, id: Value| {
            let s = s.lock().unwrap_or_else(|e| e.into_inner());
            let id = key(&id);
            lua_scalar(
                lua,
                id.and_then(|id| {
                    s.pending
                        .get(&id)
                        .or_else(|| s.data.get("options")?.get(id))
                }),
            )
        })?,
    )?;
    // the native options with a default other than 0 (`native_default`), id -> value: the Options window shows them before anything is saved
    net.raw_set(
        "option_defaults",
        lua.create_function(|lua, ()| {
            let defaults = lua.create_table()?;
            for id in 0..256u32 {
                if let Some(value) = native_default(id) {
                    defaults.set(id, value)?;
                }
            }
            Ok(defaults)
        })?,
    )?;
    let s = store.clone();
    net.raw_set(
        "option_set",
        lua.create_function(move |_, (id, value): (Value, Value)| {
            if let (Some(id), Some(value)) = (key(&id), scalar(value)) {
                let mut s = s.lock().unwrap_or_else(|e| e.into_inner());
                s.pending.insert(id, value);
            }
            Ok(())
        })?,
    )?;
    let s = store.clone();
    net.raw_set(
        "save_settings",
        lua.create_function(move |_, ()| {
            let mut s = s.lock().unwrap_or_else(|e| e.into_inner());
            let pending = std::mem::take(&mut s.pending);
            for (id, value) in pending {
                let options = s.data.entry("options").or_insert_with(|| json!({}));
                if !options.is_object() {
                    *options = json!({});
                }
                if options.get(&id) != Some(&value) {
                    options[id] = value;
                    s.dirty = true;
                }
            }
            if let Err(e) = s.save() {
                bevy::log::warn!("settings: save: {e:#}");
            }
            SAVES.fetch_add(1, Ordering::Relaxed);
            Ok(())
        })?,
    )?;
    // the monitor's screen resolutions, `{ {width, height}, ... }` ascending
    net.raw_set(
        "resolutions",
        lua.create_function(|lua, ()| {
            let list = lua.create_table()?;
            for (w, h) in RESOLUTIONS.lock().unwrap_or_else(|e| e.into_inner()).iter() {
                list.raw_push(lua.create_sequence_from([*w, *h])?)?;
            }
            Ok(list)
        })?,
    )?;
    for (namespace, get_name, save_name) in [
        ("bag", "bag_tabs", "save_bag_tabs"),
        ("bank", "bank_tabs", "save_bank_tabs"),
    ] {
        let s = store.clone();
        net.raw_set(
            get_name,
            lua.create_function(move |lua, ()| {
                let s = s.lock().unwrap_or_else(|e| e.into_inner());
                let Some(bag) = s.data.get(namespace) else {
                    return Ok(None);
                };
                let out = lua.create_table()?;
                let tabs = lua.create_table()?;
                if let Some(items) = bag.get("tabs").and_then(Json::as_array) {
                    for tab in items.iter().take(32) {
                        let Some(name) = tab.get("name").and_then(Json::as_str) else {
                            continue;
                        };
                        let t = lua.create_table()?;
                        t.set("name", name)?;
                        t.set("icon", tab.get("icon").and_then(Json::as_i64).unwrap_or(1))?;
                        let groups = lua.create_table()?;
                        if let Some(items) = tab.get("groups").and_then(Json::as_array) {
                            for group in items.iter().take(256).filter_map(Json::as_i64) {
                                groups.raw_push(group)?;
                            }
                        }
                        t.set("groups", groups)?;
                        tabs.raw_push(t)?;
                    }
                }
                out.set("tabs", tabs)?;
                out.set(
                    "selected",
                    bag.get("selected").and_then(Json::as_i64).unwrap_or(1),
                )?;
                Ok(Some(out))
            })?,
        )?;
        let s = store.clone();
        net.raw_set(
            save_name,
            lua.create_function(move |_, (tabs, selected): (Table, u32)| {
                let mut items = Vec::new();
                for t in tabs.sequence_values::<Table>().take(32) {
                    let t = t?;
                    let name: String = t.get("name")?;
                    if name.len() > 256 {
                        continue;
                    }
                    let groups: Table = t.get("groups")?;
                    let groups = groups
                        .sequence_values::<i64>()
                        .take(256)
                        .collect::<mlua::Result<Vec<_>>>()?;
                    items.push(json!({"name":name,"icon":t.get::<i64>("icon")?,"groups":groups}));
                }
                let bag =
                    json!({"selected":selected.clamp(1, items.len() as u32 + 1),"tabs":items});
                let mut s = s.lock().unwrap_or_else(|e| e.into_inner());
                if s.data.get(namespace) != Some(&bag) {
                    s.data.insert(namespace.into(), bag);
                    s.dirty = true;
                }
                if let Err(e) = s.save() {
                    bevy::log::warn!("settings: inventory tab save: {e:#}");
                }
                Ok(())
            })?,
        )?;
    }
    inventory_order::install(lua, net, store)
}

/// Chat's native local store uses independent tab/extra/option keys per world and character.
/// Resolve the Lua state's store on each call so isolated original-script fixtures stay isolated.
pub(crate) fn chat_read(lua: &Lua, owner: &str, suffix: &str) -> Option<Json> {
    let store = lua.app_data_ref::<LuaStore>().map(|s| s.0.clone()).unwrap_or_else(shared);
    let store = store.lock().unwrap_or_else(|e| e.into_inner());
    store.data.get("chat")?.get(owner)?.get(suffix).cloned()
}

pub(crate) fn chat_save(lua: &Lua, owner: &str, suffix: &str, value: Json) -> bool {
    let store = lua.app_data_ref::<LuaStore>().map(|s| s.0.clone()).unwrap_or_else(shared);
    let mut store = store.lock().unwrap_or_else(|e| e.into_inner());
    let chat = store.data.entry("chat").or_insert_with(|| json!({}));
    if !chat.is_object() { *chat = json!({}); }
    let record = chat.as_object_mut().unwrap().entry(owner).or_insert_with(|| json!({}));
    if !record.is_object() { *record = json!({}); }
    record[suffix] = value;
    store.dirty = true;
    match store.save() {
        Ok(()) => true,
        Err(e) => { bevy::log::warn!("settings: chat save: {e:#}"); false }
    }
}

/// Reset only the native four records; retain other characters, settings and pending edits.
pub(crate) fn chat_reset(lua: &Lua, owner: &str) -> bool {
    let store = lua.app_data_ref::<LuaStore>().map(|s| s.0.clone()).unwrap_or_else(shared);
    let mut store = store.lock().unwrap_or_else(|e| e.into_inner());
    let mut changed = false;
    if let Some(record) = store.data.get_mut("chat").and_then(|chat| chat.get_mut(owner)).and_then(Json::as_object_mut) {
        for key in ["tabs", "extra", "locks", "options"] {
            changed |= record.remove(key).is_some();
        }
    }
    store.dirty |= changed;
    match store.save() {
        Ok(()) => true,
        Err(e) => { bevy::log::warn!("settings: chat reset: {e:#}"); false }
    }
}

pub fn install_hud(lua: &Lua) -> mlua::Result<()> {
    lua.load(include_str!("settings.lua"))
        .set_name("@settings.lua")
        .exec()?;
    let store = lua.app_data_ref::<LuaStore>().map(|s| s.0.clone()).unwrap_or_else(shared);
    bindings::install(lua, store)
}

pub fn install_options(lua: &Lua) -> mlua::Result<()> {
    lua.load(include_str!("settings_options.lua"))
        .set_name("@settings_options.lua")
        .exec()
}

#[cfg(test)]
pub(crate) fn reopen_fixture(rt: &aa_ui::UiRuntime, path: PathBuf) -> mlua::Result<()> {
    let net: Table = rt.lua.globals().get("NET")?;
    install_store(&rt.lua, &net, Arc::new(Mutex::new(Store::open(path))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_survive_new_lua_state_and_file_replacement()
    -> Result<(), Box<dyn std::error::Error>> {
        let path =
            std::env::temp_dir().join(format!("aa-client-settings-{}.json", std::process::id()));
        let store = Arc::new(Mutex::new(Store::open(path.clone())));
        let bind = |store| -> mlua::Result<Lua> {
            let lua = Lua::new();
            let net = lua.create_table()?;
            install_store(&lua, &net, store)?;
            lua.globals().set("NET", net)?;
            Ok(lua)
        };
        let lua = bind(store.clone())?;
        lua.load("NET.option_set(35, 0.25); NET.option_set('35', 'named'); NET.save_settings()")
            .exec()?;
        lua.load("NET.option_set(35, 0.5); NET.save_settings(); NET.option_set(35, 0.9); NET.save_bag_tabs({{name='Supplies',icon=3,groups={1,7}}}, 2)").exec()?;
        lua.load("NET.save_bank_tabs({{name='Stored',icon=4,groups={8}}},2)")
            .exec()?;
        drop(lua);
        let lua = bind(Arc::new(Mutex::new(Store::open(path.clone()))))?;
        lua.load("assert(NET.option_get(35) == 0.5); assert(NET.option_get('35') == 'named'); local b=NET.bag_tabs(); assert(b.selected==2 and b.tabs[1].name=='Supplies' and b.tabs[1].groups[2]==7)").exec()?;
        lua.load("local b=NET.bank_tabs(); assert(b.selected==2 and b.tabs[1].name=='Stored'); assert(NET.bag_tabs().tabs[1].name=='Supplies')").exec()?;
        // Use the actual bag/filter wrappers in a fresh HUD, as a stage switch/restart does.
        let mut rt = aa_ui::UiRuntime::new("en_us", 1280.0, 720.0)?;
        let bridge = std::rc::Rc::new(std::cell::RefCell::new(crate::hud::api::HudData::default()));
        crate::hud::api::install(&mut rt, &bridge)?;
        let net: Table = rt.lua.globals().get("NET")?;
        install_store(
            &rt.lua,
            &net,
            Arc::new(Mutex::new(Store::open(path.clone()))),
        )?;
        install_options(&rt.lua)?;
        install_hud(&rt.lua)?;
        rt.lua.load("assert(X2Bag:TabCount()==2 and X2Bag:CurrentTabIdx()==2); local n,i,g=X2Bag:TabInfo(2); assert(n=='Supplies' and i==3 and g[2]==7); X2Option:CreateOptionItemFloat(35,1); assert(X2Option:GetOptionItemValue(35)==0.5); X2Option:Reset(); assert(X2Option:GetOptionItemValue(35)==1); X2Bag:RemoveTab(2)").exec()?;
        let disk = Store::open(path.clone());
        assert_eq!(
            disk.data["options"]["n:35"],
            json!(0.5),
            "bag saves must not commit an unapplied option edit"
        );
        assert_eq!(
            disk.data["bag"]["tabs"],
            json!([]),
            "deleting every custom tab persists an empty list"
        );
        // a native option the scripts create no item for shows 1.2's default before anything is saved (the Options window
        // saved the unchecked box it was shown as 0, which switched Dash's double tap off), and Reset brings it back
        rt.lua.load("assert(OPTION_ITEM_USE_CELERITY_WITH_DOUBLE_FORWARD==55); assert(X2Option:GetOptionItemValue(OPTION_ITEM_USE_CELERITY_WITH_DOUBLE_FORWARD)==1); X2Option:SetItemFloatValue(55,0); assert(X2Option:GetOptionItemValue(55)==0); X2Option:Reset(); assert(X2Option:GetOptionItemValue(55)==1); assert(X2Option:GetOptionItemValue(OPTION_ITEM_CLICK_TO_MOVE)==0)").exec()?;
        // the graphics page shows the 1.2 defaults (every level 4, shadows and HDR on, gamma 1) before anything is saved, and Reset restores them
        rt.lua.load("assert(X2Option:GetOptionItemValue(OPTION_ITEM_VIEW_DST)==4 and X2Option:GetOptionItemValue(OPTION_ITEM_ENABLE_SHADOW)==1 and X2Option:GetOptionItemValue(OPTION_ITEM_GAMMA)==1 and X2Option:GetOptionItemValue(OPTION_ITEM_ANTI_ALIASING)==3 and X2Option:GetOptionItemValue(OPTION_ITEM_VSYNC)==0); X2Option:SetItemFloatValue(OPTION_ITEM_VIEW_DST,2); X2Option:SetItemFloatValue(OPTION_ITEM_HDR,0); assert(X2Option:GetOptionItemValue(OPTION_ITEM_VIEW_DST)==2); X2Option:Reset(); assert(X2Option:GetOptionItemValue(OPTION_ITEM_VIEW_DST)==4 and X2Option:GetOptionItemValue(OPTION_ITEM_HDR)==1)").exec()?;
        // the resolution list is the monitor's (`graphics` fills it); the window's own size without one
        rt.lua.load("assert(X2Option:GetResolutionCount()==1); assert(#X2Option:EnumAAFormats()==0)").exec()?;
        set_resolutions(vec![(1280, 720), (1920, 1080)]);
        rt.lua.load("assert(X2Option:GetResolutionCount()==2); local w,h,bpp=X2Option:GetResolution(2); assert(w==1920 and h==1080 and bpp==32); assert(X2Option:GetResolution(3)==nil)").exec()?;
        set_resolutions(Vec::new());
        // a save counts for the systems that apply the options
        let before = saves();
        rt.lua.load("X2Option:Save()").exec()?;
        assert_eq!(saves(), before + 1);
        // Exercise the original lazy Options frame: Reset re-Init changes control.originalValue,
        // but the frame's Cancel must restore the values from when it opened.
        rt.load_base_libs();
        rt.lua.load("X2Option:SetItemFloatValueByName('ShowBuffDuration',0); ToggleOptionFrame(true); assert(GetOptionItemValue('ShowBuffDuration')==0); X2Option:Reset(); assert(GetOptionItemValue('ShowBuffDuration')==1); for _,f in ipairs(optionFrame.contentWindow.pageWindow.subFrames) do f:Init() end; optionFrame:Cancel(); assert(GetOptionItemValue('ShowBuffDuration')==0); ToggleOptionFrame(false); X2Option:Save()").exec()?;
        assert_eq!(
            Store::open(path.clone()).data["options"]["s:ShowBuffDuration"].as_f64(),
            Some(0.0)
        );
        std::fs::write(&path, b"invalid json")?;
        let mut broken = Store::open(path.clone());
        assert!(broken.data.is_empty());
        broken.data.insert("options".into(), json!({"n:35":0.75}));
        broken.dirty = true;
        broken.save()?;
        assert_eq!(
            Store::open(path.clone()).data["options"]["n:35"],
            json!(0.75)
        );
        std::fs::remove_file(path)?;
        Ok(())
    }

    /// A file an older build wrote holds 0 for option 55 (Dash on a double tap) that nobody chose: it is dropped once, so the default
    /// of 1 shows; a 0 saved after the stamp is the player's.
    #[test]
    fn an_unchosen_zero_for_a_native_default_is_dropped_once() {
        assert_eq!(native_default(55), Some(1.0));
        let mut data = json!({"options": {"n:55": 0, "n:54": 0, "n:35": 0.5}}).as_object().cloned().unwrap();
        assert!(stamp_options(&mut data, true));
        let options = data["options"].as_object().unwrap();
        assert!(!options.contains_key("n:55"), "the unchosen 0 of an option whose default is 1 is gone");
        assert_eq!(options["n:54"], json!(0), "an option whose default is 0 stays");
        assert_eq!(options["n:35"], json!(0.5));
        assert!(options.contains_key(OPTIONS_STAMP));
        // stamped: a 0 the player chose since stays, and nothing changes again
        data["options"]["n:55"] = json!(0);
        assert!(!stamp_options(&mut data, true));
        assert_eq!(data["options"]["n:55"], json!(0));
        // a file with no options is left alone (the first save stamps it, without dropping anything)
        let mut fresh = Map::new();
        assert!(!stamp_options(&mut fresh, true) && fresh.is_empty());
        let mut saved = json!({"options": {"n:55": 0}}).as_object().cloned().unwrap();
        assert!(stamp_options(&mut saved, false));
        assert_eq!(saved["options"]["n:55"], json!(0), "a save does not drop a value");
        // a file on disk: opened, migrated, written by the next save
        let path = std::env::temp_dir().join(format!("aa-client-settings-stamp-{}.json", std::process::id()));
        std::fs::write(&path, br#"{"options":{"n:55":0,"n:4":1}}"#).unwrap();
        let mut store = Store::open(path.clone());
        assert!(store.dirty && store.data["options"].get("n:55").is_none());
        store.save().unwrap();
        let reopened = Store::open(path.clone());
        assert!(!reopened.dirty && reopened.data["options"].get("n:55").is_none() && reopened.data["options"]["n:4"] == json!(1));
        std::fs::remove_file(path).unwrap();
    }

    /// A file of the first stamp holds what the window showed for the graphics options (nothing applied them then): the lowest step of
    /// every slider, unchecked boxes. They are dropped once so the defaults show; the other options and later saves stay.
    #[test]
    fn graphics_options_of_the_first_stamp_are_dropped_once() {
        let mut data = json!({"options": {
            OLD_OPTIONS_STAMP: 1, "n:3": 0.5, "n:7": 1, "n:10": 1, "n:17": 0, "n:29": 1, "n:33": 1, "s:MasterGrahicQuality": 4,
            "n:4": 1, "n:0": 1, "n:1": 1920, "n:35": 0.5, "n:65": 1, "s:ShowBuffDuration": 0
        }})
        .as_object()
        .cloned()
        .unwrap();
        assert!(stamp_options(&mut data, true));
        let options = data["options"].as_object().unwrap();
        for gone in ["n:3", "n:7", "n:10", "n:17", "n:29", "n:33", "s:MasterGrahicQuality", OLD_OPTIONS_STAMP] {
            assert!(!options.contains_key(gone), "{gone} is dropped");
        }
        // vsync, screen mode and size are the player's (the window never showed anything else), as are the other pages' options
        for kept in ["n:4", "n:0", "n:1", "n:35", "n:65", "s:ShowBuffDuration", OPTIONS_STAMP] {
            assert!(options.contains_key(kept), "{kept} stays");
        }
        // the options saved since are kept
        data["options"]["n:10"] = json!(2);
        assert!(!stamp_options(&mut data, true));
        assert_eq!(data["options"]["n:10"], json!(2));
        assert!(is_graphics_option("n:18") && !is_graphics_option("n:34") && !is_graphics_option("n:2"));
    }
}
