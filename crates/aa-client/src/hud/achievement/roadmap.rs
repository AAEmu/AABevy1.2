//! The original roadmap option API over the existing shared type-5 tree.
use super::*;

pub(super) fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    let x2: Table = lua.globals().get("X2")?;
    let state = bridge.clone();
    x2.set(
        "GetRoadmapOptionUiData",
        lua.create_function(move |lua, _: MultiValue| {
            let options = state.borrow().achievement.persistence.roadmap();
            let Some(options) = options else {
                return Ok(None);
            };
            let out = lua.create_table()?;
            if let Some(size) = options.size {
                out.set("size", signed(size))?;
            }
            if let Some(visible) = options.visible {
                out.set("visible", visible)?;
            }
            if let Some(npc) = options.npc_show {
                out.set("npcShow", signed(npc))?;
            }
            Ok(Some(out))
        })?,
    )?;
    let state = bridge.clone();
    x2.set(
        "SetRoadmapOptionUiData",
        lua.create_function(move |lua, args: MultiValue| {
            let Some(Value::Table(options)) = args.get(1) else {
                return Ok(());
            };
            state
                .borrow_mut()
                .achievement
                .persistence
                .record_roadmap(None, None, None);
            //Read Lua before borrowing shared state; field providers can reenter.
            let size = integer(&options.get::<Value>("size")?);
            let boolean = |value| {
                if let Value::Boolean(v) = value {
                    Some(v)
                } else {
                    None
                }
            };
            let visible = boolean(options.get::<Value>("visible")?);
            let npc = boolean(options.get::<Value>("npcShow")?);
            {
                let mut data = state.borrow_mut();
                data.achievement
                    .persistence
                    .record_roadmap(size, visible, npc);
                //The original Init/click has already applied this local choice.
                data.achievement.roadmap_dirty = false;
            }
            if let Some(npc) = npc {
                let hud: Table = lua.globals().get("HUD")?;
                hud.call_function::<()>("map_dots", npc)?;
            }
            Ok(())
        })?,
    )
}

pub(super) fn flush(rt: Option<&UiRuntime>, bridge: &Shared) {
    if !bridge.borrow().achievement.roadmap_dirty {
        return;
    }
    let Some(rt) = rt else { return };
    let Ok(Some(button)) = rt.lua.globals().get::<Option<Table>>("roadmapSizeButton") else {
        return;
    };
    let Ok(Some(init)) = button.get::<Option<mlua::Function>>("Init") else {
        return;
    };
    bridge.borrow_mut().achievement.roadmap_dirty = false;
    //Use original sizing, checkbox handlers, NPC side effects and anchor updates.
    if let Err(error) = init.call::<()>(button) {
        warn!("roadmap: cannot apply received options: {error}");
    }
}
