//! Native all-window retirement, four-key deletion, reload and original zero-argument callback.
use super::*;

pub(super) fn install(lua: &Lua, state: &Shared) -> mlua::Result<()> {
    let api: Table = lua.globals().get("X2Chat")?;
    let s = state.clone();
    api.raw_set(
        "InitChatWindow",
        lua.create_function(move |lua, _: MultiValue| {
            aa_ui::lua::chat::reset_windows(lua);
            let key = s.borrow().chat_config.key();
            if let Some(key) = key {
                // Native continues to reload and notify after a failed local-store deletion.
                crate::settings::chat_reset(lua, &key);
            }
            {
                let mut data = s.borrow_mut();
                let view = &mut data.chat_config;
                view.loaded = false;
                view.model = Model::default();
                view.transfers.clear();
            }
            aa_ui::lua::chat_input::set_forced_method(lua, -6);
            ensure(lua, &s)?;
            match lua.globals().get::<Value>("OnResetAllChatWindow")? {
                Value::Function(callback) => {
                    if let Err(error) = callback.call::<MultiValue>(()) {
                        warn!("chat: OnResetAllChatWindow: {error}");
                    }
                }
                Value::Nil => {}
                _ => warn!("chat: OnResetAllChatWindow is not a function"),
            }
            Ok(())
        })?,
    )?;
    Ok(())
}
