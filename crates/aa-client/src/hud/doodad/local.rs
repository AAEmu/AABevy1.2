//! The doodad funcs 1.2 runs on the client instead of sending their skill.
//!
//! `FUN_393bc260` (778111) switches on the current func's `actual_func_type` before
//! `FUN_393b3ec0` (770741) sends anything. The cases handled here return 1 (handled) and open
//! the original window locally; the server never sees the func's skill:
//!
//! * `DoodadFuncOpenFarmInfo` (0x48) -> `FUN_393b8700` (774877) sets interaction state 10 and
//!   raises `OPEN_COMMON_FARM_INFO(actual_func_id)`, the farm type of
//!   `doodad_func_open_farm_infos`.
//! * `DoodadFuncStoreUi` (0x57) -> `FUN_393b8cc0` (775175) sets state 0x11 and raises
//!   `NPC_INTERACTION_START("store")`; `store.alb` opens the store window, whose catalog is the
//!   func's `doodad_func_store_uis.merchant_pack_id` (`merchant_goods`), not an NPC's.
//! * `DoodadFuncBankUi` (0x56) -> `FUN_393b8c20` (775139) sets state 0xf and raises
//!   `NPC_INTERACTION_START("bank")`.
//! * `DoodadFuncNaviOpenMailbox` (0x2c): `FUN_393bd290` (779073) lists it as skill 11400 through
//!   the func-type map of `FUN_397aa970` (1699182), and using it reaches `FUN_393b1040` (767979),
//!   which sets state 4 and raises `TOGGLE_MAIL(doodadObjId, true)`. The handler is reached through
//!   the pointer table `0x399d2c58` (direct caller not recovered), calls `FUN_393ccdd0` (not ported)
//!   and mailbox group 107 has `next_phase` 10279, so whether it sends a packet is unverified.
//!
//! `DoodadFuncInsertCounter` (0x54) is the pre-check of its case: when both the bag count
//! (`FUN_397ccda0`) and the 28 equipment slots (`FUN_393b8f00`) hold fewer than
//! `doodad_func_insert_counters.item_count` of `item_id` it shows `NOT_ENOUGH_ITEM` (0x1b,
//! `FUN_39428370`) and does not send the skill; otherwise the skill is sent as usual. With no
//! local player it returns 1 (handled, no send).
//!
//! The other client-side cases of `FUN_393bc260` stay in the main module's use path or need a
//! dependency that is not here yet (coffer permission protocol, the book/emblem windows); the
//! table in `docs/aa-client-doodads.md` says what each does.

use std::collections::HashMap;

use aa_net::state::{Doodads, Inventory, Units};
use aa_ui::UiRuntime;
use rusqlite::Connection;

use crate::hud::api::Shared;

/// The action id the native maps `DoodadFuncNaviOpenMailbox` to (`FUN_397aa970`). Its skill row is
/// 11400 "Open Mailbox"; the func itself has no `func_skill_id`.
pub(super) const MAILBOX_ACTION: u32 = 11400;

/// A func whose window the client opens itself, with its `doodad_funcs.perm_id`
/// (`FUN_393b3300`; the native checks it when listing and again before the switch).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::hud) struct LocalUse {
    pub action: LocalAction,
    pub perm: u8,
}

/// The window a local func opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::hud) enum LocalAction {
    /// `TOGGLE_MAIL(doodadObjId, true)` (`FUN_393b1040`).
    Mailbox,
    /// `OPEN_COMMON_FARM_INFO(farm_id)` (`FUN_393b8700`, `doodad_func_open_farm_infos.farm_id`).
    FarmInfo(u32),
    /// `NPC_INTERACTION_START("store")`; the catalog is this `merchant_pack_id`.
    Store(u32),
    /// `NPC_INTERACTION_START("bank")` (`FUN_393b8c20`).
    Bank,
}

/// `DoodadFuncInsertCounter` requirement and its permission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::hud) struct CounterUse {
    pub item: u32,
    pub count: u32,
    pub perm: u8,
}

/// The detail rows the local funcs read: `doodad_func_open_farm_infos.farm_id`,
/// `doodad_func_store_uis.merchant_pack_id` and the insert-counter requirement.
#[derive(Default)]
pub(super) struct LocalFuncs {
    farms: HashMap<u32, u32>,
    stores: HashMap<u32, u32>,
    /// func id -> (item template, number the bag or equipment must hold).
    insert_counters: HashMap<u32, (u32, u32)>,
}

impl LocalFuncs {
    pub(super) fn load(conn: &Connection) -> Self {
        let mut out = Self::default();
        out.farms = read_map(conn, "SELECT id, farm_id FROM doodad_func_open_farm_infos");
        out.stores = read_map(
            conn,
            "SELECT id, merchant_pack_id FROM doodad_func_store_uis",
        );
        if let Ok(mut q) =
            conn.prepare("SELECT id, item_id, item_count FROM doodad_func_insert_counters")
        {
            if let Ok(rows) = q.query_map([], |r| {
                Ok((
                    crate::hud::data::any_int(r, 0)? as u32,
                    crate::hud::data::any_int(r, 1)? as u32,
                    crate::hud::data::any_int(r, 2)? as u32,
                ))
            }) {
                for (func, item, count) in rows.flatten() {
                    if item != 0 && count != 0 {
                        out.insert_counters.insert(func, (item, count));
                    }
                }
            }
        }
        out
    }

    /// The local window of `kind`: `None` when that kind has no client window here (or its detail
    /// row is missing).
    pub(super) fn local(&self, kind: &str, func_id: u32) -> Option<LocalAction> {
        match kind {
            "DoodadFuncNaviOpenMailbox" => Some(LocalAction::Mailbox),
            "DoodadFuncOpenFarmInfo" => self
                .farms
                .get(&func_id)
                .map(|farm| LocalAction::FarmInfo(*farm)),
            "DoodadFuncStoreUi" => self
                .stores
                .get(&func_id)
                .map(|pack| LocalAction::Store(*pack)),
            "DoodadFuncBankUi" => Some(LocalAction::Bank),
            _ => None,
        }
    }

    /// The insert-counter requirement of a func: `(item template, count)`.
    pub(super) fn insert_counter(&self, func_id: u32) -> Option<(u32, u32)> {
        self.insert_counters.get(&func_id).copied()
    }
}

/// Reads a two-column `id -> value` detail table; a missing table (a fixture) is empty.
fn read_map(conn: &Connection, sql: &str) -> HashMap<u32, u32> {
    let Ok(mut q) = conn.prepare(sql) else {
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
    rows.flatten().filter(|(_, value)| *value != 0).collect()
}

/// Runs a local func. Native `FUN_393bc260` always handles these: the skill is never sent, even
/// when the window cannot open here (no UI runtime, no position, an empty pack). `FUN_393b7b10`
/// first ends the interaction that is active (`FUN_393b7aa0`), which `end_interaction` mirrors.
#[allow(clippy::too_many_arguments)]
pub(super) fn run(
    rt: Option<&UiRuntime>,
    bridge: &Shared,
    doodads: &Doodads,
    units: &Units,
    inventory: &Inventory,
    obj_id: u32,
    local: LocalAction,
) {
    let Some(rt) = rt else {
        return;
    };
    super::end_interaction(Some(rt), bridge, inventory);
    match local {
        LocalAction::Mailbox => super::super::mail::open_at(rt, obj_id),
        LocalAction::FarmInfo(farm) => {
            // Without a world position the native window still opens; this port cannot place it,
            // so it stays closed rather than sending the skill.
            if let Some(pos) = doodads
                .by_id
                .get(&obj_id)
                .and_then(|o| crate::ingame::units::doodad_world(o, units))
            {
                super::super::farm::show_doodad(rt, farm as i32, pos);
            }
        }
        // An empty or unknown merchant pack still opens the native store window (it has no
        // goods); the server owns the purchase check.
        LocalAction::Store(pack) => {
            super::super::store::open_doodad(rt, bridge, obj_id, pack, false);
        }
        LocalAction::Bank => {
            super::super::store::open_doodad(rt, bridge, obj_id, 0, true);
        }
    }
}

/// `DoodadFuncInsertCounter` (`FUN_393bc260` case 0x54): the bag count (`FUN_397ccda0`) and the
/// 28 equipment slots (`FUN_393b8f00`) are compared separately; only when both are below
/// `item_count` does the native refuse. Equipment items count through their template id.
pub(super) fn missing_counter_item(inventory: &Inventory, item: u32, count: u32) -> bool {
    let bag: u32 = inventory
        .bag
        .iter()
        .flatten()
        .filter(|i| i.template_id == item)
        .map(|i| i.count.max(0) as u32)
        .sum();
    let equipped: u32 = inventory
        .equipment
        .slots
        .iter()
        .flatten()
        .filter(|i| i.template_id == item)
        .map(|i| i.count.max(0) as u32)
        .sum();
    bag < count && equipped < count
}

/// The `ErrorMessageType` of the refusal (`error_names.txt`: 27 `NOT_ENOUGH_ITEM`).
pub(super) const NOT_ENOUGH_ITEM: i16 = 27;

/// `FUN_393bc260` shows `NO_PERM` (error 2) when `FUN_393b3300` refuses the func.
pub(super) const NO_PERM: i16 = 2;

#[cfg(test)]
mod tests;
