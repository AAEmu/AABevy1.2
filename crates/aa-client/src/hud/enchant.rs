//! Original enchant-window mode lifecycle. Item selection/planning is separate.
use aa_net::{Event, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, MultiValue, Table, Value};

use super::api::{HudBridge, Shared};
mod catalog;
mod controller;
mod records;
mod selection;
pub(super) use catalog::Tables;
pub(super) use controller::ResultKind;

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
    operation: Option<controller::Operation>,
}

pub(super) fn current(data: &super::api::HudData, id: u64) -> Option<&aa_net::model::Item> {
    data.enchant.selection.records.current(id)
}
pub(super) fn equipment(data: &super::api::HudData, slot: u8) -> Option<&aa_net::model::Item> {
    data.enchant.selection.equipment(slot)
}

pub(super) fn music_paper(data: &super::api::HudData, id: u64) -> Option<aa_net::item_effect::CapturedBagItem> {
    music_bag_item(data, id, 27902)
}
pub(super) fn music_sheet(data: &super::api::HudData, id: u64) -> Option<aa_net::item_effect::CapturedBagItem> {
    music_bag_item(data, id, 28051)
}
fn music_bag_item(data: &super::api::HudData, id: u64, template: u32) -> Option<aa_net::item_effect::CapturedBagItem> {
    let records = &data.enchant.selection.records;
    if data.character_id == 0 || data.disconnected || records.owner() != Some(data.character_id) { return None; }
    (0..130).find_map(|slot| {
        let item = records.item(aa_net::items::container::INVENTORY, slot)?;
        (item.id == id && item.template_id == template).then(|| aa_net::item_effect::CapturedBagItem { slot, item: item.clone() })
    })
}
pub(super) fn request_current(state: &Shared, character_id: u32, request: &aa_net::item_effect::ItemEffectRequest) -> bool {
    state
        .borrow()
        .enchant
        .operation
        .as_ref()
        .is_some_and(|operation| operation.matches(character_id, request))
}

fn complete(lua: Option<&Lua>, state: &Shared) {
    let presence = {
        let mut data = state.borrow_mut();
        //39046020 resets the captured operation before any selection setter or
        // synchronous UPDATE callback can reenter the controller.
        let Some(operation) = data.enchant.operation.take() else { return };
        let super::api::HudData {
            enchant, game, cooldowns, ..
        } = &mut *data;
        match game.get() {
            Some(game) => enchant
                .selection
                .restore(&game.enchant, cooldowns, enchant.mode, &operation.request, operation.result_seen()),
            None => {
                enchant.selection.clear();
                false
            }
        }
    };
    if let Some(lua) = lua {
        selection::update(lua, presence);
    }
}

pub(super) fn result(lua: Option<&Lua>, state: &Shared, kind: ResultKind) {
    let finished = {
        let mut data = state.borrow_mut();
        let mode = data.enchant.mode;
        data.enchant
            .operation
            .as_mut()
            .is_some_and(|operation| operation.result(mode, kind))
    };
    if finished {
        complete(lua, state);
    }
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
fn release_pending(rt: Option<&UiRuntime>, state: &Shared, inventory: Option<&mut aa_net::state::Inventory>, container: u8, slot: u8) {
    if let Some(inventory) = inventory {
        inventory.clear_item_slot_pending(container, slot);
        state.borrow_mut().enchant.selection.sync_pending(inventory);
    }
    state.borrow_mut().enchant.selection.release_pending(container, slot);
    if let Some(rt) = rt {
        let event = match container {
            2 => "BAG_LOCK_UPDATE",
            3 => "BANK_LOCK_UPDATE",
            _ => return,
        };
        // Installed C358=394e1370 ignores pending's bool; notification is
        // unconditional even for an absent key, with one numeric slot+1.
        aa_ui::lua::fire_ui_event(&rt.lua, event, vec![Value::Number(f64::from(u16::from(slot) + 1))]);
    }
}
pub(super) fn dispatch(rt: Option<&UiRuntime>, state: &Shared, event: &Event, mut inventory: Option<&mut aa_net::state::Inventory>) {
    let replaces_owner = matches!(event, Event::PlayerSpawn(spawn)
        if state.borrow().enchant.selection.records.owner() != Some(spawn.character().id));
    if retires(event) || replaces_owner {
        state.borrow_mut().enchant.operation = None;
        super::item_effects::clear(state);
        if let Some(rt) = rt {
            if let Err(error) = switch(&rt.lua, state, Mode::Off) {
                warn!("enchant: retiring mode: {error}");
            }
        } else {
            let mut data = state.borrow_mut();
            data.enchant.mode = Mode::Off;
            data.enchant.selection.clear();
        }
    }
    let (cleared, released_detail) = {
        let mut data = state.borrow_mut();
        let before = data.enchant.selection.slots;
        let released_detail = data.enchant.selection.observe(event);
        (
            before != data.enchant.selection.slots && data.enchant.mode != Mode::Off,
            released_detail,
        )
    };
    if released_detail {
        if let Event::ItemDetailUpdated(update) = event {
            //394e47b0 releases this key after the actual128-byte copy.
            // It does not dispatch an item-consumption notification.
            release_pending(rt, state, inventory.as_deref_mut(), 2, update.slot);
        }
    }
    if let Event::ItemOperationError(error) = event {
        if error.error_code != 0 {
            if let Some(rt) = rt {
                if let Err(error) = selection::error(&rt.lua, error.error_code) {
                    warn!("item operation error: {error}");
                }
            }
            //391d56c0 reports the error before either397d2200(false) call.
            // Preserve repeated keys; these are lock releases, not settlement.
            for (container, slot) in error.pending_releases() {
                release_pending(rt, state, inventory.as_deref_mut(), container, slot);
            }
        }
    }
    if cleared {
        if let Some(rt) = rt {
            selection::update(&rt.lua, false);
        }
    }
    if let Event::ItemsChanged(changes) = event {
        if !changes.truncated {
            for task in &changes.tasks {
                let finished = {
                    let mut data = state.borrow_mut();
                    let output = data.enchant.selection.apply_row(task);
                    let mode = data.enchant.mode;
                    output.as_ref().is_some_and(|output| {
                        data.enchant
                            .operation
                            .as_mut()
                            .is_some_and(|operation| operation.row(mode, changes.reason, output))
                    })
                };
                if finished {
                    complete(rt.map(|rt| &rt.lua), state);
                }
            }
            state.borrow_mut().enchant.selection.records.finish(changes);
        }
    }
    match event {
        Event::ItemEffectRequested { character_id, request } => {
            if let Some(operation) = state.borrow_mut().enchant.operation.as_mut() {
                operation.requested(*character_id, request);
            }
        }
        Event::ItemEffectRejected {
            character_id,
            request,
            message,
        } => {
            let matches = state
                .borrow()
                .enchant
                .operation
                .as_ref()
                .is_some_and(|operation| operation.matches(*character_id, request));
            if matches {
                state.borrow_mut().enchant.operation = None;
                if let Some(rt) = rt {
                    let presence = state.borrow().enchant.selection.presence();
                    selection::update(&rt.lua, presence);
                    rt.fire_event("SYSMSG", message.as_str());
                }
            }
        }
        _ => {}
    }
    // Actual result producer admission formats the current record here,
    // emits original Lua first, then advances the controller receiver.
    // Music's synchronous Started/Fired callbacks must see this row and no future row.
    super::user_music::receipt(rt, state, event);
    super::item_effects::receive(rt, state, event);
    // Specialty callbacks may query or change selected items synchronously.
    // They see all preceding native rows and no later receipt in this frame.
    super::specialty::receive(rt.map(|rt| &rt.lua), state, event);
}

pub(super) fn track(
    mut events: MessageReader<NetEvent>,
    rt: Option<NonSend<UiRuntime>>,
    state: NonSend<HudBridge>,
    built: Res<super::HudBuilt>,
    mut inventory: Option<ResMut<aa_net::state::Inventory>>,
) {
    // Flush real startup buff receipts even when this frame has no network events.
    super::user_music::refresh_catalog(&state.0);
    let stage_rt = rt.as_deref();
    let rt = stage_rt.filter(|_| built.up);
    super::mode_actions::ready(rt, &state.0);
    // Error callbacks can query locks before their first release in this same
    // frame; refresh the shared pending provider before any synchronous Lua.
    if let Some(inventory) = inventory.as_deref() {
        state.0.borrow_mut().enchant.selection.sync_pending(inventory);
    }
    if let Some(rt) = rt {
        super::item_effects::replay(rt, &state.0);
    }
    for NetEvent(event) in events.read() {
        // SC099 has no Game/HUD gate; retain the actual stage runtime while
        // other item callbacks keep their existing HUD admission policy.
        let event_rt = if matches!(event, Event::SpecialtyRatio(_)) { stage_rt } else { rt };
        dispatch(event_rt, &state.0, event, inventory.as_deref_mut());
    }
    if let Some(inventory) = inventory.as_deref() {
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
