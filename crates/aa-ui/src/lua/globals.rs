//! Globals the toolkit provides: `UIParent` methods, UI constants, `Script.ReloadScript`, and
//! helpers to register game API objects (`X2World`, ...) that belong to the app.

use mlua::{Lua, MultiValue, Table, Value};

use super::{Args, method, none, reg, ret, ui};

/// The entity object of `GetEntityByName` (see there).
const ENTITY_LUA: &str = r#"
return function(name, info)
  local function vec(v)
    v = v or {}
    return { x = v.x or v[1] or 0, y = v.y or v[2] or 0, z = v.z or v[3] or 0 }
  end
  local e = {}
  e.GetName = function(self) return name end
  e.GetPos = function(self) return vec(info.pos) end
  e.GetWorldPos = e.GetPos
  e.GetAngles = function(self) return vec(info.angles) end
  e.GetWorldAngles = e.GetAngles
  e.IsValid = function(self) return true end
  return e
end
"#;

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

    def(
        "CreateWidget",
        method(lua, |lua, _, a| {
            let ty = a.s(1).unwrap_or_else(|| "emptywidget".into());
            let name = a.s(2).unwrap_or_default();
            ret(
                lua,
                super::widget::create_widget_global(lua, &ty, &name, &a.v(3))?,
            )
        })?,
    )?;
    def(
        "GetScreenWidth",
        method(lua, |lua, _, _| ret(lua, ui(lua).tree.screen_w))?,
    )?;
    def(
        "GetScreenHeight",
        method(lua, |lua, _, _| ret(lua, ui(lua).tree.screen_h))?,
    )?;
    def(
        "GetUIScale",
        method(lua, |lua, _, _| ret(lua, ui(lua).ui_scale))?,
    )?;
    def(
        "SetUIScale",
        method(lua, |lua, _, a| {
            let s = a.opt_f(1).unwrap_or(1.0).clamp(0.5, 2.0);
            let mut u = ui(lua);
            u.animations.set_ui_scale(s);
            if u.ui_scale != s {
                u.ui_scale = s;
                u.tree.repaint();
            }
            none()
        })?,
    )?;
    for (name, level) in [("Log", 0), ("LogAlways", 0), ("Warning", 1), ("Error", 2)] {
        def(
            name,
            method(lua, move |_, _, a| {
                let msg = a.s(1).unwrap_or_default();
                match level {
                    // `UIParent:Log` is the scripts' trace output (state names, event names): debug,
                    // under its own target (`RUST_LOG=aa_ui::lua=debug` brings it back).
                    0 => bevy::log::debug!(target: "aa_ui::lua", "lua: {msg}"),
                    1 => bevy::log::warn!("lua: {msg}"),
                    _ => bevy::log::error!("lua: {msg}"),
                }
                none()
            })?,
        )?;
    }
    // `UIParent:GetEntityByName(name)`: an entity of the level (the login stage reads the
    // `login_camera` entity's place for its camera). The app answers through
    // `AAUI.Entity(name) -> nil | { pos = {x, y, z}, angles = {x, y, z} }` (fields `x`/`y`/`z`
    // or 1..3); the result has `GetName`, `GetPos` / `GetWorldPos`, `GetAngles` /
    // `GetWorldAngles` (fresh `{x, y, z}` tables read from the hook's table at each call) and
    // `IsValid`. Without the hook, or for an unknown name, nil (the scripts then warn).
    def(
        "GetEntityByName",
        method(lua, |lua, _, a| {
            let name = a.s(1).unwrap_or_default();
            let Ok(aaui) = lua.globals().raw_get::<Table>("AAUI") else {
                return none();
            };
            let Ok(Some(f)) = aaui.raw_get::<Option<mlua::Function>>("Entity") else {
                return none();
            };
            let info = match f.call::<Value>(name.as_str()) {
                Ok(Value::Table(t)) => t,
                Ok(_) => return none(),
                Err(e) => {
                    bevy::log::warn!("aa-ui: AAUI.Entity({name}): {e}");
                    return none();
                }
            };
            let make: mlua::Function = lua.load(ENTITY_LUA).set_name("@aa-ui/entity").eval()?;
            ret(lua, make.call::<Table>((name, info))?)
        })?,
    )?;
    def(
        "GetFrameTime",
        method(lua, |lua, _, _| ret(lua, ui(lua).frame_ms / 1000.0))?,
    )?;
    def(
        "GetFrameRate",
        method(lua, |lua, _, _| {
            let ms = ui(lua).frame_ms.max(0.001);
            ret(lua, 1000.0 / ms)
        })?,
    )?;
    def(
        "GetCursorPosition",
        method(lua, |lua, _, _| {
            let (x, y) = ui(lua).cursor;
            ret(lua, (x, y))
        })?,
    )?;
    def(
        "GetTextureData",
        method(lua, |lua, _, a| {
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
        })?,
    )?;
    def(
        "SetEventHandler",
        method(lua, |lua, _, a| {
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
        })?,
    )?;
    def(
        "ReleaseEventHandler",
        method(lua, |lua, _, a| {
            let name = a.s(1).unwrap_or_default();
            if let Some(l) = ui(lua).events.get_mut(&name) {
                l.retain(|&x| x != 0);
            }
            none()
        })?,
    )?;
    def(
        "GetUIStamp",
        method(lua, |lua, _, a| {
            let key = a.s(1).unwrap_or_default();
            let stamps: Option<Table> = lua.named_registry_value("aa_ui_stamps").ok();
            match stamps {
                Some(t) => ret(lua, t.raw_get::<Value>(key)?),
                None => none(),
            }
        })?,
    )?;
    def(
        "SetUIStamp",
        method(lua, |lua, _, a| {
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
        })?,
    )?;
    def("SetUseInsertComma", method(lua, |_, _, _| none())?)?;
    def(
        "IsRenderThreadSupported",
        method(lua, |lua, _, _| ret(lua, true))?,
    )?;
    def("IsDX11Supported", method(lua, |lua, _, _| ret(lua, false))?)?;

    // CryEngine's Script table: ReloadScript("SCRIPTS/x2ui/...") loads scriptsbin/x2ui/...
    if g.raw_get::<Value>("Script")?.is_nil() {
        let script = lua.create_table()?;
        let load = lua.create_function(|lua, path: String| {
            let p = path.replace('\\', "/");
            let rel = match p.get(..8) {
                Some(pre) if pre.eq_ignore_ascii_case("scripts/") => {
                    format!("scriptsbin/{}", &p[8..])
                }
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
        script.raw_set(
            "UnloadScript",
            lua.create_function(|_, _: MultiValue| Ok(()))?,
        )?;
        g.raw_set("Script", script)?;
    }

    // The toolkit's part of the game API: `X2Locale:GetLocale()`, the `ADDON` content registry
    // and the `X2DialogManager` message boxes. `ADDON` and `X2` are one native table in the
    // client. The app adds the rest (`install_game_stub` returns these tables).
    let locale = ui(lua).textures.locale.clone();
    let x2locale = install_game_stub(lua, "X2Locale")?;
    x2locale.raw_set(
        "GetLocale",
        lua.create_function(move |_, _: MultiValue| Ok(locale.clone()))?,
    )?;
    let addon = install_game_stub(lua, "ADDON")?;
    if g.raw_get::<Value>("X2")?.is_nil() {
        g.raw_set("X2", &addon)?;
    }
    install_game_stub(lua, "X2DialogManager")?;
    // `X2Cursor` (what the pointer holds) and `RegisterForDrag`'s storage: see [`super::drag`].
    super::drag::install(lua)?;
    // `X2Chat:DispatchChatMessage(filter, text)` routes a line to the chat tabs (see
    // [`super::chat`]); the rest of `X2Chat` is the app's.
    let x2chat = install_game_stub(lua, "X2Chat")?;
    x2chat.raw_set(
        "DispatchChatMessage",
        lua.create_function(
            |lua, (_this, filter, text): (Value, Option<i64>, Option<String>)| {
                super::chat::dispatch(lua, filter.unwrap_or(0), &text.unwrap_or_default())
            },
        )?,
    )?;
    // `X2Input:GetMousePos()`: the cursor in screen pixels (the colour pad of character create
    // drags with it from `OnUpdate`).
    let x2input = install_game_stub(lua, "X2Input")?;
    x2input.raw_set(
        "GetMousePos",
        lua.create_function(|lua, _: MultiValue| {
            let u = ui(lua);
            Ok((u.cursor.0 * u.ui_scale, u.cursor.1 * u.ui_scale))
        })?,
    )?;
    super::keyboard::install_input(lua)?;
    // Hooks the app (or the example fakes) fills in: `AAUI.SlotContent`, `AAUI.ChatTabs`, ...
    if g.raw_get::<Value>("AAUI")?.is_nil() {
        g.raw_set("AAUI", lua.create_table()?)?;
    }
    // `X2Sound:PlayUISound` / `PlayMusic` ... call the `AAUI.PlaySound` / `PlayMusic` ... hooks (see [`super::sound`]).
    install_game_stub(lua, "X2Sound")?;
    lua.load(super::sound::X2SOUND_LUA).set_name("@aa-ui/x2sound").exec()?;
    lua.load(TOOLKIT_LUA).set_name("@aa-ui/toolkit").exec()?;
    Ok(())
}

/// * `ADDON` content registry (`UIC_*` categories): `RegisterContentWidget(cat, widget,
///   showFn)`, `GetContent(cat)` (nil when nothing registered), `RegisterContentTriggerFunc(cat,
///   fn)`, `ShowContent(cat, show)`, `ToggleContent(cat)`.
/// * `X2DialogManager`: `SetHandler(task, fn)` registers the decorator of a dialog kind
///   (`components/dialog_handler.lua` registers `DLG_TASK_DEFAULT`); `RequestDefaultDialog(fn,
///   ownerId)` / `RequestNoticeDialog(fn, ownerId)` create a window with `textbox`, `btnOk`,
///   `btnCancel`, run the decorator with `(wnd, info)` (`info.buttonType` `DBT_OK_CANCEL` /
///   `DBT_OK`, modal), then `fn(wnd, info)`, and show it; `RequestTaskDialog(task, info, ownerId)`
///   is the same with the handler registered for the task (`DLG_TASK_*`) as `fn`, `info` merged
///   into the dialog's info; `OnOK` / `OnCancel` / `OnCustom(id)`
///   close it, `DeleteByOwnerWindow(ownerId)` closes an owner's dialogs.
/// * `X2Locale:LocalizeUiText` returns the key until the app provides the UI text table.
const TOOLKIT_LUA: &str = r#"
local contents, shows, triggers = {}, {}, {}
function ADDON:RegisterContentWidget(cat, widget, show)
  contents[cat] = widget
  shows[cat] = show
end
function ADDON:GetContent(cat)
  return contents[cat]
end
function ADDON:RegisterContentTriggerFunc(cat, fn)
  triggers[cat] = fn
end
function ADDON:ShowContent(cat, show)
  if shows[cat] then
    shows[cat](show)
  elseif contents[cat] then
    contents[cat]:Show(show)
  elseif triggers[cat] and show then
    triggers[cat]()
  end
end
function ADDON:ToggleContent(cat)
  if triggers[cat] then
    triggers[cat]()
  elseif contents[cat] then
    ADDON:ShowContent(cat, not contents[cat]:IsVisible())
  end
end
function ADDON:ImportAPI() end
function ADDON:ImportObject() end
function X2Locale:LocalizeUiText(cat, key)
  return tostring(key)
end

local dialogHandlers, dialogs, dialogCount = {}, {}, 0
function X2DialogManager:SetHandler(task, fn)
  dialogHandlers[task] = fn
end
local function requestDialog(fn, owner, buttonType)
  dialogCount = dialogCount + 1
  local wnd = UIParent:CreateWidget("window", "x2Dialog" .. dialogCount, "UIParent")
  wnd:CreateChildWidget("textbox", "textbox", 0, true)
  wnd:CreateChildWidget("button", "btnOk", 0, true)
  wnd:CreateChildWidget("button", "btnCancel", 0, true)
  if buttonType ~= DBT_OK_CANCEL then
    wnd.btnCancel:Show(false)
  end
  local info = { buttonType = buttonType, isModal = true }
  local decorate = dialogHandlers[DLG_TASK_DEFAULT]
  if decorate then decorate(wnd, info) end
  if fn then fn(wnd, info) end
  dialogs[wnd:GetId()] = { wnd = wnd, owner = owner }
  wnd:Show(true)
  wnd:Raise()
  return wnd
end
function X2DialogManager:RequestDefaultDialog(fn, owner)
  return requestDialog(fn, owner, DBT_OK_CANCEL)
end
function X2DialogManager:RequestNoticeDialog(fn, owner)
  return requestDialog(fn, owner, DBT_OK)
end
function X2DialogManager:RequestTaskDialog(task, info, owner)
  return requestDialog(function(wnd, dialogInfo)
    for k, v in pairs(info or {}) do dialogInfo[k] = v end
    local handler = dialogHandlers[task]
    if handler then handler(wnd, dialogInfo) end
  end, owner, DBT_OK_CANCEL)
end
local function closeDialog(id)
  local d = dialogs[id]
  if d then
    dialogs[id] = nil
    d.wnd:Show(false)
  end
end
function X2DialogManager:OnOK(id) closeDialog(id) end
function X2DialogManager:OnCancel(id) closeDialog(id) end
function X2DialogManager:OnCustom(id) closeDialog(id) end
function X2DialogManager:DeleteByOwnerWindow(owner)
  for id, d in pairs(dialogs) do
    if d.owner == owner then closeDialog(id) end
  end
end
"#;

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
            let Value::String(k) = key else {
                return Ok(Value::Nil);
            };
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
