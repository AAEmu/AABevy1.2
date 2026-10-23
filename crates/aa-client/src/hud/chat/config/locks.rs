//! Eight native window locks, distinct from tab/selection storage.
use super::*;
use std::collections::BTreeMap;

#[derive(Default)]
struct Applied(BTreeMap<Id, bool>);

fn owner(state: &Shared, raw: i32) -> Option<usize> {
    usize::try_from(state.borrow().chat_config.model.current_window(raw)?)
        .ok()
        .filter(|&id| id < 8)
}
fn apply(lua: &Lua, widget: Id, locked: bool) -> mlua::Result<bool> {
    if !aa_ui::lua::chat::set_allow_tab_switch(lua, widget, !locked) {
        return Ok(false);
    }
    let Some(window) = object_table(lua, widget)? else {
        return Ok(false);
    };
    window.call_method::<()>("UseResizing", !locked)?;
    lua.app_data_mut::<Applied>().unwrap().0.insert(widget, locked);
    Ok(true)
}
pub(super) fn install(lua: &Lua, state: &Shared) -> mlua::Result<()> {
    lua.set_app_data(Applied::default());
    let api: Table = lua.globals().get("X2Chat")?;
    let s = state.clone();
    api.raw_set(
        "IsLockedChatWindowByChatTabId",
        lua.create_function(move |lua, args: MultiValue| {
            let Some(raw) = argument(&args, 1) else {
                return Ok(MultiValue::new());
            };
            ensure(lua, &s)?;
            let locked = owner(&s, raw).is_some_and(|id| s.borrow().chat_config.locks[id]);
            Ok(boolean(locked))
        })?,
    )?;
    let s = state.clone();
    api.raw_set(
        "LockChatWindowByTabId",
        lua.create_function(move |lua, args: MultiValue| {
            let Some(raw) = argument(&args, 1) else {
                return Ok(MultiValue::new());
            };
            let locked = match args.get(2) {
                Some(Value::Boolean(value)) => *value,
                Some(value) => match numeric(value) {
                    Some(value) => value != 0.,
                    None => return Ok(MultiValue::new()),
                },
                None => return Ok(MultiValue::new()),
            };
            ensure(lua, &s)?;
            let Some(id) = owner(&s, raw) else { return Ok(boolean(false)) };
            // Native writes the byte before checking the live window, but saves only on success.
            s.borrow_mut().chat_config.locks[id] = locked;
            let Some(widget) = live_window(lua, id as i32) else {
                return Ok(boolean(false));
            };
            if !apply(lua, widget, locked)? {
                return Ok(boolean(false));
            }
            Ok(boolean(save(lua, &s, "locks")))
        })?,
    )?;
    Ok(())
}

/// Apply restored state once per live widget/value. Explicit lock calls always reapply,
/// including an unchanged unlock that hides the toolkit's native warning.
pub(super) fn sync(state: NonSend<HudBridge>, rt: Option<NonSend<UiRuntime>>) {
    let Some(rt) = rt else { return };
    restore(&rt.lua, &state.0);
}
pub(super) fn restore(lua: &Lua, state: &Shared) {
    let locks = {
        let data = state.borrow();
        if !data.chat_config.loaded {
            return;
        }
        data.chat_config.locks
    };
    let windows = {
        let u = ui(lua);
        u.tree
            .of_class("chatwindow")
            .into_iter()
            .filter_map(|widget| {
                let id = usize::try_from(u.tree.widget(widget)?.chat.as_ref()?.window_id).ok()?;
                (id < 8).then_some((widget, locks.get(id).copied()?))
            })
            .collect::<Vec<_>>()
    };
    for &(widget, locked) in &windows {
        if lua.app_data_ref::<Applied>().and_then(|a| a.0.get(&widget).copied()) != Some(locked) {
            if let Err(error) = apply(lua, widget, locked) {
                warn!("chat: restore lock: {error}");
            }
        }
    }
    if let Some(mut applied) = lua.app_data_mut::<Applied>() {
        applied.0.retain(|id, _| windows.iter().any(|(widget, _)| widget == id));
    }
}
