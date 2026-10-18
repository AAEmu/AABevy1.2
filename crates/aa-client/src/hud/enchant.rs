//! Original enchant-window mode lifecycle. Item selection/planning is separate.
use aa_net::{Event, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, MultiValue, Table, Value};

use super::api::{HudBridge, Shared};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Off,
    Grade,
    Socket,
    Unsocket,
    Gem,
}
impl Mode {
    fn name(self) -> &'static str {
        match self {
            Self::Off => "",
            Self::Grade => "grade",
            Self::Socket => "socketing",
            Self::Unsocket => "unsocket",
            Self::Gem => "gem",
        }
    }
}
#[derive(Default)]
pub(super) struct View {
    mode: Mode,
}

fn switch(lua: &Lua, state: &Shared, mode: Mode) -> mlua::Result<()> {
    //39046740 updates the controller before synchronous callbacks. The original
    // window calls the same switch again while selecting its tabs/submenus.
    let previous = {
        let mut data = state.borrow_mut();
        let previous = data.enchant.mode;
        if mode == previous {
            return Ok(());
        }
        data.enchant.mode = mode;
        previous
    };
    let (event, args) = if mode == Mode::Off {
        ("LEAVE_ENCHANT_ITEM_MODE", Vec::new())
    } else {
        (
            if previous == Mode::Off {
                "ENTER_ENCHANT_ITEM_MODE"
            } else {
                "SWITCH_ENCHANT_ITEM_MODE"
            },
            vec![Value::String(lua.create_string(mode.name())?)],
        )
    };
    aa_ui::lua::fire_ui_event(lua, event, args);
    Ok(())
}

pub(super) fn install(lua: &Lua, state: &Shared) -> mlua::Result<()> {
    let api: Table = lua.globals().get("X2ItemEnchant")?;
    for (name, mode) in [
        ("EnterItemEnchantMode", Mode::Grade),
        ("SwitchItemEnchantGradeMode", Mode::Grade),
        ("SwitchItemEnchantSocketingMode", Mode::Socket),
        ("SwitchItemEnchantUnsocketMode", Mode::Unsocket),
        ("SwitchItemEnchantGemMode", Mode::Gem),
        ("LeaveItemEnchantMode", Mode::Off),
    ] {
        let state = state.clone();
        api.raw_set(
            name,
            lua.create_function(move |lua, _: MultiValue| {
                //39064b20..39064da0 ignore arguments and return zero results.
                // Their39426af0 helper cancels only native action-cursor kind4,
                // which is not yet represented here. A picked BAG item belongs
                // to a separate cursor and must survive these calls.
                switch(lua, &state, mode)
            })?,
        )?;
    }
    let item: Table = lua.globals().get("X2Item")?;
    let state = state.clone();
    item.raw_set(
        "IsInEnchantMode",
        lua.create_function(move |_, _: MultiValue| Ok(state.borrow().enchant.mode != Mode::Off))?,
    )
}

fn retires(event: &Event) -> bool {
    matches!(
        event,
        Event::CharacterSelected(_)
            | Event::LoginSucceeded(_)
            | Event::ReadyToConnect { .. }
            | Event::LeftWorld { .. }
            | Event::Disconnected { .. }
            | Event::Kicked { .. }
            | Event::PhaseChanged(aa_net::Phase::Authenticating | aa_net::Phase::Connecting)
    )
}
fn track(mut events: MessageReader<NetEvent>, rt: Option<NonSend<UiRuntime>>, state: NonSend<HudBridge>, built: Res<super::HudBuilt>) {
    for NetEvent(event) in events.read() {
        if !retires(event) {
            continue;
        }
        if let Some(rt) = rt.as_deref().filter(|_| built.up) {
            if let Err(error) = switch(&rt.lua, &state.0, Mode::Off) {
                warn!("enchant: retiring mode: {error}");
            }
        } else {
            state.0.borrow_mut().enchant = View::default();
        }
    }
}
pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        track
            .before(aa_ui::UiInput)
            .before(super::sync_hud)
            .before(super::prebuild_hud)
            .before(super::build_hud),
    );
}

#[allow(dead_code)]
pub(super) mod fixture;
#[cfg(test)]
mod tests;
