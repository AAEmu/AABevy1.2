//! Native Window::OnHide deletes dialogs owned by that exact widget before Lua.
//! Keep this private bridge limited to the five proven NPC messagebox owners.
use aa_ui::UiRuntime;
use mlua::{Function, MultiValue, ObjectLike, Table, Value};

pub(super) fn install(rt: &UiRuntime) -> mlua::Result<()> {
    for getter in [
        "GetDivineBlessingMessageBoxHandleWindow",
        "GetRecoverExpMessageBoxHandleWindow",
        "GetStablerMessageBoxHandleWindow",
        "GetCreateExpeditionMessageBoxHandleWindow",
        "GetSpecialtyStoreMessageBoxHandleWindow",
    ] {
        let Some(get) = rt.lua.globals().raw_get::<Option<Function>>(getter)? else {
            continue;
        };
        let Some(window) = get.call::<Option<Table>>(())? else {
            continue;
        };
        if window.raw_get::<Option<bool>>("__aa_npc_owner_cleanup")? == Some(true) {
            continue;
        }
        let Some(id) = aa_ui::lua::value_id(&rt.lua, &Value::Table(window.clone())) else {
            continue;
        };
        let owner: Value = window.call_method("GetId", ())?;
        let handlers = aa_ui::lua::reg(&rt.lua)
            .handlers
            .raw_get::<Option<Table>>(id)?;
        let prior = handlers.and_then(|handlers| {
            handlers
                .raw_get::<Option<Function>>("OnHide")
                .ok()
                .flatten()
        });
        let hide = rt.lua.create_function(move |lua, args: MultiValue| {
            let manager: Table = lua.globals().get("X2DialogManager")?;
            manager.call_method::<()>("DeleteByOwnerWindow", owner.clone())?;
            if let Some(prior) = &prior {
                prior.call::<()>(args)?;
            }
            Ok(())
        })?;
        window.call_method::<()>("SetHandler", ("OnHide", hide))?;
        window.raw_set("__aa_npc_owner_cleanup", true)?;
    }
    Ok(())
}
