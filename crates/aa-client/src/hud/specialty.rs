//! Original SC099 ratio receipt/getter. Request planning and price context are separate.
use aa_net::{Event, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, MultiValue, ObjectLike, Table, Value};

use super::api::{HudBridge, Shared};

#[allow(dead_code)]
pub(super) mod fixture;

pub(super) struct View {
    ratio: i32,
}

impl Default for View {
    fn default() -> Self {
        //39991f50 loads global39ac9b74; both globals were read as100 in the
        // original StandMode client. The protected global initializer is not modeled.
        Self { ratio: 100 }
    }
}

pub(super) fn install(lua: &Lua, state: &Shared) -> mlua::Result<()> {
    let store: Table = lua.globals().get("X2Store")?;
    let state = state.clone();
    store.set(
        "GetSpecialtyRatio",
        lua.create_function(move |_, _: MultiValue| {
            //394aeb80: no argument/context gate; signed CVTSI2SS before Lua number.
            Ok(f64::from(state.borrow().specialty.ratio as f32))
        })?,
    )
}

pub(super) fn receive(lua: Option<&Lua>, state: &Shared, event: &Event) {
    let Event::SpecialtyRatio(ratio) = event else { return };
    if *ratio == 0 {
        // Original error225 is synchronous and precedes the reset to100.
        // Never hold the shared state borrow across either Lua call.
        if let Some(lua) = lua {
            let report = || -> mlua::Result<()> {
                if let Some(key) = super::alerts::error_name(225) {
                    let locale: Table = lua.globals().get("X2Locale")?;
                    let group: Value = lua.globals().get("ERROR_MSG")?;
                    let text: Value = locale.call_method("LocalizeUiText", (group, key))?;
                    aa_ui::lua::fire_ui_event(lua, "SYSMSG", vec![text]);
                }
                Ok(())
            };
            if let Err(error) = report() {
                warn!("hud: specialty error225: {error}");
            }
        }
        state.borrow_mut().specialty.ratio = 100;
    } else {
        state.borrow_mut().specialty.ratio = *ratio;
        if let Some(lua) = lua {
            aa_ui::lua::fire_ui_event(
                lua,
                "NPC_INTERACTION_START",
                vec![Value::String(lua.create_string("specialty").expect("Lua string"))],
            );
        }
    }
}

#[cfg(test)]
mod tests;
