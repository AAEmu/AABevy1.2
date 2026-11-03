//! The fish-stand confirmation dialogs (`DoodadFuncBuyFish` / `DoodadFuncConvertFish`).
//!
//! The 1.2 doodad-skill use path (`FUN_393b3ec0` 770741) asks `FUN_393bc260` before it sends
//! (770820, 778111). For `DoodadFuncBuyFish` (id 0x4f) it calls `FUN_393b8870` (774963) and for
//! `DoodadFuncConvertFish` (id 0x50) `FUN_393b8a30` (775055); both return 1 = handled, do not send,
//! after showing a confirmation dialog. This module runs the same local checks and opens task 12 /
//! 13 through the original `dialog_handler.alb`; only OK sends the doodad func skill, cancel sends
//! nothing.
//!
//! Checks (`FUN_397cf5a0` 1727959 buy, `FUN_397cf660` convert):
//! * no item worn on the back -> `STORE_BACKPACK_NOGOODS` (361, 0x169);
//! * buy: template not in `doodad_func_buy_fish_items` -> `STORE_NPC_NOT_HANDLE_THIS_ITEM` (225,
//!   0xe1); convert: not in `doodad_func_convert_fish_items` -> the same;
//! * buy: `item_backpacks.backpack_type_id` not 6 -> `STORE_NOT_SELLABLE_ITEM` (88, 0x58);
//!   convert checks the item kind 6 before the list.
//! On success the buy refund is the item's `refund` (`FUN_397cd350`), and the info table carries the
//! item info (icon, name, refund) to the script's `confirm_buy_fish_title` / `confirm_fish_body`
//! handler.

use std::collections::{HashMap, HashSet};

use aa_net::model::slot;
use aa_net::state::Inventory;
use aa_net::{Command, NetCommand};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, Table};
use rusqlite::Connection;

use crate::hud::data::GameData;

/// The fish-stand func (`doodad_funcs.actual_func_type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FishKind {
    Buy,
    Convert,
}

/// The phase func of a fish stand: its kind and `actual_func_id` (the item-list key).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FishFunc {
    pub kind: FishKind,
    pub func_id: u32,
}

/// The equipped fish and what the dialog shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Sale {
    pub template: u32,
    pub grade: u8,
    pub count: i32,
    /// The buy price; 0 for convert (native `FUN_393b8970` has no refund).
    pub refund: i64,
}

/// The fish-stand dialog that is waiting for OK/cancel.
#[derive(Resource, Default)]
pub(super) struct FishDialog {
    pending: Option<Pending>,
}

struct Pending {
    command: Command,
    obj_id: u32,
}

/// The 1.2 checks (`FUN_397cf5a0` / `FUN_397cf660`); `Err` is the `ErrorMessageType` to show.
pub(super) fn check(
    game: &GameData,
    items: Option<&HashSet<u32>>,
    inventory: &Inventory,
    kind: FishKind,
) -> Result<Sale, i16> {
    const BACKPACK_NOGOODS: i16 = 361;
    const NOT_HANDLED: i16 = 225;
    const NOT_SELLABLE: i16 = 88;
    let Some(item) = inventory.equipment.get(slot::BACKPACK) else {
        return Err(BACKPACK_NOGOODS);
    };
    let template = item.template_id;
    let kind_6 = game.item_backpack_types.get(&template) == Some(&6);
    match kind {
        FishKind::Buy => {
            if !items.is_some_and(|items| items.contains(&template)) {
                return Err(NOT_HANDLED);
            }
            if !kind_6 {
                return Err(NOT_SELLABLE);
            }
        }
        FishKind::Convert => {
            if !kind_6 {
                return Err(NOT_SELLABLE);
            }
            if !items.is_some_and(|items| items.contains(&template)) {
                return Err(NOT_HANDLED);
            }
        }
    }
    Ok(Sale {
        template,
        grade: item.grade,
        count: item.count.max(1),
        refund: if kind == FishKind::Buy {
            game.items.get(&template).map_or(0, |info| info.refund)
        } else {
            0
        },
    })
}

/// `false` when the func is not a fish stand; otherwise the request is handled (error shown or the
/// dialog opened) and the caller must not send the skill.
#[allow(clippy::too_many_arguments)]
pub(super) fn request(
    rt: Option<&UiRuntime>,
    game: &GameData,
    items: Option<&HashSet<u32>>,
    inventory: &Inventory,
    func: FishFunc,
    skill_id: u32,
    doodad_obj_id: u32,
    dialog: &mut FishDialog,
    mut errors: Option<&mut MessageWriter<crate::hud::ShowError>>,
) -> bool {
    let sale = match check(game, items, inventory, func.kind) {
        Ok(sale) => sale,
        Err(code) => {
            if let Some(errors) = errors.as_mut() {
                errors.write(crate::hud::ShowError(code));
            }
            return true;
        }
    };
    if let Some(rt) = rt {
        if open(rt, func.kind, sale) {
            dialog.pending = Some(Pending {
                command: Command::UseDoodadSkill {
                    skill_id,
                    doodad_obj_id,
                },
                obj_id: doodad_obj_id,
            });
        } else {
            warn!("doodad: fish-stand dialog {func:?} did not open; not sending");
        }
    }
    true
}

fn open(rt: &UiRuntime, kind: FishKind, sale: Sale) -> bool {
    let Ok(hud) = rt.lua.globals().get::<Table>("HUD") else {
        return false;
    };
    let Ok(open) = hud.get::<mlua::Function>("fish_open") else {
        return false;
    };
    match open.call::<bool>((
        kind == FishKind::Buy,
        sale.template,
        sale.grade,
        sale.count,
        sale.refund,
    )) {
        Ok(opened) => opened,
        Err(error) => {
            warn!("doodad: fish dialog: {error}");
            false
        }
    }
}

fn state(rt: &UiRuntime) -> String {
    rt.lua
        .load("return HUD.fish_state and HUD.fish_state() or ''")
        .eval::<String>()
        .unwrap_or_default()
}

fn clear(rt: &UiRuntime) {
    let _ = rt
        .lua
        .load("if HUD.fish_clear then HUD.fish_clear() end")
        .exec();
}

/// Sends the stored doodad skill when the original dialog handler saw OK; drops it on cancel or when
/// the interaction ended (the handler cancels on `INTERACTION_END`).
pub(super) fn tick(
    rt: Option<NonSend<UiRuntime>>,
    mut dialog: ResMut<FishDialog>,
    bridge: NonSend<crate::hud::api::HudBridge>,
    mut net: MessageWriter<NetCommand>,
) {
    let Some(obj_id) = dialog.pending.as_ref().map(|p| p.obj_id) else {
        return;
    };
    let Some(rt) = rt else {
        dialog.pending = None;
        return;
    };
    match state(&rt).as_str() {
        "ok" => {
            if let Some(pending) = dialog.pending.take() {
                info!("doodad: fish-stand dialog confirmed: {:?}", pending.command);
                net.write(NetCommand(pending.command));
            }
            clear(&rt);
        }
        "cancel" | "" => {
            info!("doodad: fish-stand dialog cancelled");
            dialog.pending = None;
            clear(&rt);
        }
        _ => {
            let focus = bridge.0.borrow().doodad.focus.as_ref().map(|f| f.obj_id);
            if focus != Some(obj_id) {
                info!("doodad: fish-stand interaction ended");
                rt.fire_event("INTERACTION_END", ());
                dialog.pending = None;
                clear(&rt);
            }
        }
    }
}

/// The item templates each fish-stand func accepts (`doodad_func_buy_fish_items` /
/// `doodad_func_convert_fish_items`); missing tables (fixtures) are empty.
pub(super) fn load_items(
    conn: &Connection,
) -> (HashMap<u32, HashSet<u32>>, HashMap<u32, HashSet<u32>>) {
    let read = |table: &str, column: &str| -> HashMap<u32, HashSet<u32>> {
        let sql = format!("SELECT {column}, item_id FROM {table}");
        let Ok(mut q) = conn.prepare(&sql) else {
            return HashMap::new();
        };
        let Ok(rows) = q.query_map([], |r| {
            Ok((
                crate::hud::data::any_int(r, 0)? as u32,
                crate::hud::data::any_int(r, 1)? as u32,
            ))
        }) else {
            return HashMap::new();
        };
        let mut map: HashMap<u32, HashSet<u32>> = HashMap::new();
        for (func, item) in rows.flatten() {
            if item != 0 {
                map.entry(func).or_default().insert(item);
            }
        }
        map
    };
    (
        read("doodad_func_buy_fish_items", "doodad_func_buy_fish_id"),
        read(
            "doodad_func_convert_fish_items",
            "doodad_func_convert_fish_id",
        ),
    )
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    lua.load(include_str!("fish.lua"))
        .set_name("@fish.lua")
        .exec()
}

#[cfg(test)]
mod tests;
