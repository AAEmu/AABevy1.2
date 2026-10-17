//! Account-local UI settings shared by the Lua states of all screens.
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use mlua::{Lua, Table, Value};
use serde_json::{Map, Value as Json, json};

#[derive(Default)]
struct Store {
    path: PathBuf,
    data: Map<String, Json>,
    pending: Map<String, Json>,
    dirty: bool,
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
        Self {
            path,
            data,
            pending: Map::new(),
            dirty: false,
        }
    }

    fn save(&mut self) -> anyhow::Result<()> {
        if !self.dirty {
            return Ok(());
        }
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

pub fn install(lua: &Lua, net: &Table) -> mlua::Result<()> {
    install_store(lua, net, shared())
}

fn install_store(lua: &Lua, net: &Table, store: Arc<Mutex<Store>>) -> mlua::Result<()> {
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
            Ok(())
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
    Ok(())
}

pub fn install_hud(lua: &Lua) -> mlua::Result<()> {
    lua.load(include_str!("settings.lua"))
        .set_name("@settings.lua")
        .exec()
}

pub fn install_options(lua: &Lua) -> mlua::Result<()> {
    lua.load(include_str!("settings_options.lua"))
        .set_name("@settings_options.lua")
        .exec()
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
}
