//! Original enchant-window mode lifecycle. Item selection/planning is separate.
use aa_net::{Event, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, MultiValue, Table, Value};

use super::api::{HudBridge, Shared};
mod catalog;
mod records;
mod selection;
pub(super) use catalog::Tables;

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
    selection: selection::State,
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
        data.enchant.selection.clear();
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
    //39046600 ->393be180(1): entering from Off retires the stored NPC
    // interaction and unconditionally emits END before ENTER. A switch between
    // active modes does not visit that helper. State/slots already reflect the
    // new mode when either original Lua callback runs.
    if previous == Mode::Off && mode != Mode::Off {
        aa_ui::lua::fire_ui_event(lua, "NPC_INTERACTION_END", Vec::new());
    }
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
    let active_state = state.clone();
    item.raw_set(
        "IsInEnchantMode",
        lua.create_function(move |_, _: MultiValue| Ok(active_state.borrow().enchant.mode != Mode::Off))?,
    )?;
    selection::install(lua, state)
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
fn track(
    mut events: MessageReader<NetEvent>,
    rt: Option<NonSend<UiRuntime>>,
    state: NonSend<HudBridge>,
    built: Res<super::HudBuilt>,
    inventory: Option<Res<aa_net::state::Inventory>>,
) {
    for NetEvent(event) in events.read() {
        if retires(event) {
            if let Some(rt) = rt.as_deref().filter(|_| built.up) {
                if let Err(error) = switch(&rt.lua, &state.0, Mode::Off) {
                    warn!("enchant: retiring mode: {error}");
                }
            } else {
                let mut data = state.0.borrow_mut();
                data.enchant.mode = Mode::Off;
                data.enchant.selection.clear();
            }
        }
        let cleared = {
            let mut data = state.0.borrow_mut();
            let before = data.enchant.selection.slots;
            data.enchant.selection.receive(event);
            before != data.enchant.selection.slots && data.enchant.mode != Mode::Off
        };
        if cleared {
            if let Some(rt) = rt.as_deref().filter(|_| built.up) {
                selection::update(&rt.lua, false);
            }
        }
    }
    if let Some(inventory) = inventory {
        state.0.borrow_mut().enchant.selection.sync_pending(&inventory);
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
