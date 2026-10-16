//! Globals the toolkit provides: `UIParent` methods, UI constants, `Script.ReloadScript`, and
//! helpers to register game API objects (`X2World`, ...) that belong to the app.

use mlua::{Lua, MultiValue, Table, Value};

use super::{Args, method, none, reg, ret, ui};

/// Constants the reference dump does not list (scripts may still read them).
const EXTRA_CONSTANTS: &[(&str, i64)] = &[("VERTICAL", 0), ("HORIZONTAL", 1)];

pub(crate) fn install(lua: &Lua) -> mlua::Result<()> {
    let g = lua.globals();
    // Real values from the client (reference/api/x2_globals.json), e.g. ALIGN_*, UI_BUTTON_*.
    for (k, v) in super::constants::CONSTANTS {
        match v {
            super::constants::Const::Num(n) => g.raw_set(*k, *n)?,
            super::constants::Const::Str(s) => g.raw_set(*k, *s)?,
        }
    }
    for (k, v) in EXTRA_CONSTANTS {
        if g.raw_get::<Value>(*k)?.is_nil() {
            g.raw_set(*k, *v)?;
        }
    }
    let r = reg(lua);
    let root_methods: Table = r.mt["uiparent"].raw_get("__index")?;
    let def = |name: &str, f: mlua::Function| root_methods.raw_set(name, f);

    def("CreateWidget", method(lua, |lua, _, a| {
        let ty = a.s(1).unwrap_or_else(|| "emptywidget".into());
        let name = a.s(2).unwrap_or_default();
        ret(lua, super::widget::create_widget_global(lua, &ty, &name, &a.v(3))?)
    })?)?;
    def("GetScreenWidth", method(lua, |lua, _, _| ret(lua, ui(lua).tree.screen_w))?)?;
    def("GetScreenHeight", method(lua, |lua, _, _| ret(lua, ui(lua).tree.screen_h))?)?;
    def("GetUIScale", method(lua, |lua, _, _| ret(lua, ui(lua).ui_scale))?)?;
    def("SetUIScale", method(lua, |lua, _, a| {
        let s = a.opt_f(1).unwrap_or(1.0).clamp(0.5, 2.0);
        ui(lua).ui_scale = s;
        none()
    })?)?;
    for (name, level) in [("Log", 0), ("LogAlways", 0), ("Warning", 1), ("Error", 2)] {
        def(name, method(lua, move |_, _, a| {
            let msg = a.s(1).unwrap_or_default();
            match level {
                0 => bevy::log::info!("lua: {msg}"),
                1 => bevy::log::warn!("lua: {msg}"),
                _ => bevy::log::error!("lua: {msg}"),
            }
            none()
        })?)?;
    }
    def("GetFrameTime", method(lua, |lua, _, _| ret(lua, ui(lua).frame_ms / 1000.0))?)?;
    def("GetFrameRate", method(lua, |lua, _, _| {
        let ms = ui(lua).frame_ms.max(0.001);
        ret(lua, 1000.0 / ms)
    })?)?;
    def("GetCursorPosition", method(lua, |lua, _, _| {
        let (x, y) = ui(lua).cursor;
        ret(lua, (x, y))
    })?)?;
    def("GetTextureData", method(lua, |lua, _, a| {
        let path = a.s(1).unwrap_or_default();
        let key = a.s(2).unwrap_or_default();
        let info = ui(lua).textures.info(&path, &key);
        let t = lua.create_table()?;
        if let Some(info) = info {
            for c in &info.children {
                let nums = c.nums();
                if nums.is_empty() {
                    continue;
                }
                t.raw_set(c.name.as_str(), lua.create_sequence_from(nums)?)?;
            }
        }
        ret(lua, t)
    })?)?;
    def("SetEventHandler", method(lua, |lua, _, a| {
        // Global handler for an event: register UIParent itself.
        let name = a.s(1).unwrap_or_default();
        if let Value::Function(f) = a.v(2) {
            let r = reg(lua);
            let key = format!("__ev_{name}");
            let hs = match r.handlers.raw_get::<Option<Table>>(0)? {
                Some(t) => t,
                None => {
                    let t = lua.create_table()?;
                    r.handlers.raw_set(0, &t)?;
                    t
                }
            };
            hs.raw_set(key, f)?;
            ui(lua).events.entry(name).or_default().push(0);
        }
        none()
    })?)?;
    def("ReleaseEventHandler", method(lua, |lua, _, a| {
        let name = a.s(1).unwrap_or_default();
        if let Some(l) = ui(lua).events.get_mut(&name) {
            l.retain(|&x| x != 0);
        }
        none()
    })?)?;
    def("GetUIStamp", method(lua, |lua, _, a| {
        let key = a.s(1).unwrap_or_default();
        let stamps: Option<Table> = lua.named_registry_value("aa_ui_stamps").ok();
        match stamps {
            Some(t) => ret(lua, t.raw_get::<Value>(key)?),
            None => none(),
        }
    })?)?;
    def("SetUIStamp", method(lua, |lua, _, a| {
        let t = match lua.named_registry_value::<Option<Table>>("aa_ui_stamps")? {
            Some(t) => t,
            None => {
                let t = lua.create_table()?;
                lua.set_named_registry_value("aa_ui_stamps", &t)?;
                t
            }
        };
        t.raw_set(a.s(1).unwrap_or_default(), a.v(2))?;
        none()
    })?)?;
    def("SetUseInsertComma", method(lua, |_, _, _| none())?)?;
    def("IsRenderThreadSupported", method(lua, |lua, _, _| ret(lua, true))?)?;
    def("IsDX11Supported", method(lua, |lua, _, _| ret(lua, false))?)?;

    // CryEngine's Script table: ReloadScript("SCRIPTS/x2ui/...") loads scriptsbin/x2ui/...
    if g.raw_get::<Value>("Script")?.is_nil() {
        let script = lua.create_table()?;
        let load = lua.create_function(|lua, path: String| {
            let p = path.replace('\\', "/");
            let rel = match p.get(..8) {
                Some(pre) if pre.eq_ignore_ascii_case("scripts/") => format!("scriptsbin/{}", &p[8..]),
                _ => p.clone(),
            };
            match aa_lua::run_alb(lua, &rel) {
                Ok(()) => Ok(true),
                Err(e) => {
                    bevy::log::warn!("aa-ui: Script.ReloadScript({path}): {e:#}");
                    Ok(false)
                }
            }
        })?;
        script.raw_set("ReloadScript", load.clone())?;
        script.raw_set("LoadScript", load)?;
        script.raw_set("UnloadScript", lua.create_function(|_, _: MultiValue| Ok(()))?)?;
        g.raw_set("Script", script)?;
    }
    Ok(())
}

/// Register a permissive stand-in for a game API object (`X2World`, `X2Unit`, ...): every method
/// exists and returns nothing (logged once). The app replaces methods with real ones by setting
/// fields: `X2World.GetWorldInfo = function(self) ... end`.
pub fn install_game_stub(lua: &Lua, name: &str) -> mlua::Result<Table> {
    let g = lua.globals();
    if let Some(t) = g.raw_get::<Option<Table>>(name)? {
        return Ok(t);
    }
    let t = lua.create_table()?;
    let mt = lua.create_table()?;
    let obj = name.to_string();
    mt.raw_set(
        "__index",
        lua.create_function(move |lua, (_t, key): (Table, Value)| {
            let Value::String(k) = key else { return Ok(Value::Nil) };
            let k = k.to_string_lossy();
            if !k.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                return Ok(Value::Nil);
            }
            let full = format!("{obj}:{k}");
            let f = lua.create_function(move |lua, _: MultiValue| {
                let mut u = ui(lua);
                let n = u.stub_calls.entry(full.clone()).or_insert(0);
                if *n == 0 {
                    bevy::log::debug!("aa-ui: game stub {full}");
                }
                *n += 1;
                Ok(())
            })?;
            Ok(Value::Function(f))
        })?,
    )?;
    t.set_metatable(Some(mt))?;
    g.raw_set(name, &t)?;
    Ok(t)
}

/// Set a global (convenience for apps: constants such as `RACE_NUIAN`).
pub fn set_global_value(lua: &Lua, name: &str, v: impl mlua::IntoLua) -> mlua::Result<()> {
    lua.globals().raw_set(name, v)
}

#[allow(dead_code)]
fn _args(_: &Args) {}
