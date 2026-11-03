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
//! * buy: `item_backpacks.backpack_type_id` not 6, `items.price` 0, or no `fish_details` row ->
//!   `STORE_NOT_SELLABLE_ITEM` (88, 0x58); convert checks the item kind 6 before the list.
//! On success the info carries the fish's weight/length from the item's detail tag 6 (f32 weight,
//! f32 length) and the refund from `FUN_397cd350` (1726122):
//! `trunc(floor(items.refund * item_grades.refund_multiplier * 0.01 + 0.5) * weight /
//! fish_details.max_weight)`, for both tasks (`FUN_39421e20` category 0x16).
//!
//! The dialog closes natively (`FUN_393b7e40` 774405) when the doodad is gone or the player is out
//! of the configured interaction distance, then `FUN_393b7aa0` fires `INTERACTION_END`.

use std::collections::{HashMap, HashSet};

use aa_net::model::slot;
use aa_net::state::{Doodads, Inventory, Units};
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
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Sale {
    pub template: u32,
    pub grade: u8,
    pub count: i32,
    /// `Item` detail tag 6 (f32 weight, f32 length), when the item has it.
    pub weight: Option<f32>,
    pub length: Option<f32>,
    /// `FUN_397cd350`: the grade/weight-scaled refund, `None` without a fish detail or row.
    pub refund: Option<i64>,
}

/// The fish-stand dialog that is waiting for OK/cancel.
#[derive(Resource, Default)]
pub(crate) struct FishDialog {
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
            if !kind_6
                || game.items.get(&template).is_none_or(|info| info.price == 0)
                || game.doodad.fish_max_weight(template).is_none()
            {
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
    let (weight, length) = fish_measure(item);
    let refund = match (
        game.items.get(&template),
        weight,
        game.doodad.fish_max_weight(template),
    ) {
        (Some(info), Some(weight), Some(max_weight)) if max_weight > 0.0 => {
            let multiplier = game
                .refund_multipliers
                .get(&u32::from(item.grade))
                .copied()
                .unwrap_or(0.0);
            let base = (info.refund as f32 * multiplier * 0.01 + 0.5).floor();
            Some((base * weight / max_weight).trunc() as i64)
        }
        _ => None,
    };
    Ok(Sale {
        template,
        grade: item.grade,
        count: item.count.max(1),
        weight,
        length,
        refund,
    })
}

/// `Item` detail tag 6: `f32` weight, `f32` length (`FUN_39421e20` 865686-865693 reads them at
/// payload +0 and +4).
fn fish_measure(item: &aa_net::model::Item) -> (Option<f32>, Option<f32>) {
    if item.detail.kind() != 6 {
        return (None, None);
    }
    let Some(payload) = item
        .detail_payload
        .as_deref()
        .filter(|payload| payload.len() >= 8)
    else {
        return (None, None);
    };
    let f = |at: usize| {
        f32::from_le_bytes([
            payload[at],
            payload[at + 1],
            payload[at + 2],
            payload[at + 3],
        ])
    };
    let (weight, length) = (f(0), f(4));
    (
        weight.is_finite().then_some(weight),
        length.is_finite().then_some(length),
    )
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
        sale.weight,
        sale.length,
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
/// the interaction ends (the doodad is gone or the player is out of `INTERACTION_DISTANCE`, the
/// native `FUN_393b7e40`).
pub(super) fn tick(
    rt: Option<NonSend<UiRuntime>>,
    mut dialog: ResMut<FishDialog>,
    bridge: NonSend<crate::hud::api::HudBridge>,
    doodads: Res<Doodads>,
    units: Res<Units>,
    mut net: MessageWriter<NetCommand>,
) {
    let Some(obj_id) = dialog.pending.as_ref().map(|p| p.obj_id) else {
        return;
    };
    let Some(rt) = rt else {
        dialog.pending = None;
        return;
    };
    let in_reach = {
        let data = bridge.0.borrow();
        super::in_interaction_reach(&data, &doodads, &units, obj_id)
    };
    if !in_reach {
        info!("doodad: fish-stand interaction ended");
        rt.fire_event("INTERACTION_END", ());
        dialog.pending = None;
        clear(&rt);
        return;
    }
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
        _ => {}
    }
}

impl super::Tables {
    /// Records the fish-stand funcs (`DoodadFuncBuyFish` / `DoodadFuncConvertFish`) of a phase.
    pub(super) fn add_fish_func(&mut self, group: u32, skill: u32, kind: &str, func_id: u32) {
        if skill == 0 {
            return;
        }
        let kind = match kind {
            "DoodadFuncBuyFish" => FishKind::Buy,
            "DoodadFuncConvertFish" => FishKind::Convert,
            _ => return,
        };
        self.fish_funcs
            .insert((group, skill), FishFunc { kind, func_id });
    }

    /// The fish-stand func of a phase skill, if the bar's skill is one.
    pub(super) fn fish_func(&self, group: u32, skill: u32) -> Option<FishFunc> {
        self.fish_funcs.get(&(group, skill)).copied()
    }

    /// The item templates a fish-stand func accepts.
    pub(super) fn fish_items(&self, kind: FishKind, func_id: u32) -> Option<&HashSet<u32>> {
        match kind {
            FishKind::Buy => self.fish_buy_items.get(&func_id),
            FishKind::Convert => self.fish_convert_items.get(&func_id),
        }
    }

    /// `fish_details.max_weight` of an item template.
    pub(super) fn fish_max_weight(&self, template: u32) -> Option<f32> {
        self.fish_max_weights.get(&template).copied()
    }
}

/// The item templates each fish-stand func accepts (`doodad_func_buy_fish_items` /
/// `doodad_func_convert_fish_items`) and the `fish_details.item_id -> max_weight` refund denominator;
/// missing tables (fixtures) are empty.
pub(super) fn load_items(
    conn: &Connection,
) -> (
    HashMap<u32, HashSet<u32>>,
    HashMap<u32, HashSet<u32>>,
    HashMap<u32, f32>,
) {
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
    let mut max_weights = HashMap::new();
    if let Ok(mut q) = conn.prepare("SELECT item_id, max_weight FROM fish_details") {
        if let Ok(mut rows) = q.query([]) {
            while let Ok(Some(row)) = rows.next() {
                let Ok(item) = crate::hud::data::any_int(row, 0) else {
                    continue;
                };
                let Ok(max_weight) = crate::hud::data::any_int(row, 1) else {
                    continue;
                };
                max_weights.insert(item as u32, max_weight as f32);
            }
        }
    }
    (
        read("doodad_func_buy_fish_items", "doodad_func_buy_fish_id"),
        read(
            "doodad_func_convert_fish_items",
            "doodad_func_convert_fish_id",
        ),
        max_weights,
    )
}

pub(super) fn install(lua: &Lua) -> mlua::Result<()> {
    lua.load(include_str!("fish.lua"))
        .set_name("@fish.lua")
        .exec()
}

#[cfg(test)]
mod tests;
