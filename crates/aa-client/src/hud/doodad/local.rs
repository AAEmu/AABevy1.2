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
//! * `DoodadFuncAuctionUi` (0x55) -> `FUN_393b8b80` (775103) sets state 0xe and raises
//!   `NPC_INTERACTION_START("auction")`; `npc_interaction.alb` opens the original auction window.
//! * `DoodadFuncBubble` (0x3f) -> `FUN_393b8240` (774609): plays the `bubbles` / `bubble_chats`
//!   conversation `doodad_func_bubbles.bubble_id` through `FUN_393703b0` (the `bubbles` index,
//!   `FUN_39375140` from the `START` line along `next`) and, when `next_phase > 0`, sends
//!   `CSChangeDoodadPhasePacket` 0x0e9 through `FUN_397ac8d0(objId, skill, nextPhase, funcId)`.
//! * `DoodadFuncOpenPaper` (0x46) -> `FUN_393b8450` (774731) sets state 1, raises
//!   `OPEN_PAPER("page", book_page_id)` or `("book", book_id)` and sends the same phase packet
//!   when `next_phase > 0`; a func with neither id falls through to the skill (returns 0).
//! * `DoodadFuncPurchase` (0x40) -> `FUN_393b80d0` (774524) sets state 8 and opens the
//!   `DLG_TASK_PURCHASE` (10) / `DLG_TASK_PURCHASE_COIN` (11) dialog built from
//!   `doodad_func_purchases`. The confirmation sends `CSBuyItemsPacket` (0x0ae) or
//!   `CSBuyCoinItemPacket` (0x0af) (`FUN_393bc4e0` / `FUN_393bac20`).
//! * `DoodadFuncRenewItem` (0x47) -> `FUN_393b8610` (774823) sets state 2 and raises
//!   `RENEW_ITEM_WND_SHOW(doodad_func_renew_items.skill_id, objId)`.
//! * `DoodadFuncStampMaker` (0x3d) -> `FUN_393b82a0` (774630) sets state 7 and raises
//!   `OPEN_EMBLEM_UPLOAD_UI(objId)` with the object id as a string; `ucc.alb` opens.
//! * `DoodadFuncCofferPerm` (0x51) -> `FUN_393bc260` case 0x51: `FUN_3983b3b0` requires any
//!   `DoodadFuncCoffer` with `actual_func_id > 0` on the template; then state 0xd and
//!   `COFFER_INTERACTION_START` (579); otherwise returns 0 (the skill is sent).
//! * `DoodadFuncParentInfo` (0x43) -> `FUN_393b8400` (774707) opens the parent house's info
//!   through `FUN_393befd0` (`HOUSE_INTERACTION_START`, 109).
//! * `DoodadFuncEvidenceItemLoot` (0x34) -> `FUN_393bb090` (777166) builds the report text and
//!   raises `REPORT_CRIME` (514) with the doodad name and the text; returns 1.
//! * `DoodadFuncNaviOpenMailbox` (0x2c): `FUN_393bd290` (779073) lists it as skill 11400 through
//!   the func-type map of `FUN_397aa970` (1699182). 11400 is dispatched by the action-handler map
//!   before `FUN_393bc260` (`FUN_393ba9c0` -> handler vtable `0x399d3114` -> `FUN_393b7bc0` ->
//!   `FUN_393b1a90` -> table `0x399d2c58` -> `FUN_393b1040` 767979), so no `CSStartSkill` is sent.
//!   `FUN_393b1040` sets state 4, raises `TOGGLE_MAIL(doodadObjId, true)` and calls
//!   `FUN_393ccdd0`, which sends `CSListMailPacket` 0x09a only while the three mail lists are not
//!   all loaded. `mail::open_at`'s `MailCommand::List` stands in for it, queued whenever the
//!   window is hidden. It runs here with the other local windows.
//!
//! `DoodadFuncInsertCounter` (0x54) is the pre-check of its case: when both the bag count
//! (`FUN_397ccda0`) and the 28 equipment slots (`FUN_393b8f00`) hold fewer than
//! `doodad_func_insert_counters.item_count` of `item_id` it shows `NOT_ENOUGH_ITEM` (0x1b,
//! `FUN_39428370`) and does not send the skill; otherwise the skill is sent as usual. With no
//! local player it returns 1 (handled, no send).

use std::collections::{HashMap, HashSet};

use aa_net::Command;
use aa_net::shop::ShopCommand;
use aa_net::state::{Doodads, Inventory, Units};
use aa_ui::UiRuntime;
use mlua::ObjectLike;
use rusqlite::Connection;

use crate::hud::api::Shared;

/// The action id the native maps `DoodadFuncNaviOpenMailbox` to (`FUN_397aa970`). Its skill row is
/// 11400 "Open Mailbox"; the func itself has no `func_skill_id`.
pub(super) const MAILBOX_ACTION: u32 = 11400;

/// `DLG_TASK_PURCHASE` and `DLG_TASK_PURCHASE_COIN` (`dialoghandler/dialog_handler.alb`).
pub(super) const TASK_PURCHASE: u8 = 10;
pub(super) const TASK_PURCHASE_COIN: u8 = 11;

/// A `DoodadFuncPurchase` row (`doodad_func_purchases`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Purchase {
    pub item: u32,
    pub count: i32,
    pub coin_item: u32,
    pub coin_count: i32,
    pub currency: u8,
}

/// A `doodad_func_evidence_item_loots` row: the action id is its `skill_id` (`FUN_393b7eb0`
/// case 0x34 reads detail+8). `FUN_393bb090` builds the report text from the player's position
/// (sub-zone, zone entry, a localized text of category 0x5c), not from the crime kind or the
/// target.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Evidence {
    pub skill: u32,
}

/// The `doodad_func_open_papers` row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Paper {
    pub page: u32,
    pub book: u32,
}

impl Paper {
    fn open(self) -> Option<(&'static str, u32)> {
        if self.page != 0 {
            Some(("page", self.page))
        } else if self.book != 0 {
            Some(("book", self.book))
        } else {
            None
        }
    }
}

/// A func whose window the client opens itself, with its `doodad_funcs.perm_id`
/// (`FUN_393b3300`; the native checks it when listing and again before the switch).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::hud) struct LocalUse {
    pub action: LocalAction,
    pub perm: u8,
}

/// The window or check a local func runs.
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
    /// `NPC_INTERACTION_START("auction")` (`FUN_393b8b80`).
    Auction,
    /// The `bubbles` conversation `bubble_id` and the native phase change (`next_phase > 0`).
    Bubble {
        bubble: u32,
        next_phase: i32,
        func_id: u32,
    },
    /// `OPEN_PAPER(kind, id)` and the native phase change (`next_phase > 0`).
    OpenPaper {
        book: bool,
        id: u32,
        next_phase: i32,
        func_id: u32,
    },
    /// The purchase task dialog (`DLG_TASK_PURCHASE` / `_COIN`).
    Purchase { order: Purchase, func_id: u32 },
    /// `RENEW_ITEM_WND_SHOW(doodad_func_renew_items.skill_id, objId)`.
    RenewItem { skill: u32 },
    /// `OPEN_EMBLEM_UPLOAD_UI(objId as a string)` (`FUN_393b82a0`).
    StampMaker,
    /// `COFFER_INTERACTION_START` (`FUN_393bc260` case 0x51).
    CofferPerm,
    /// The parent house's info (`FUN_393b8400` -> `FUN_393befd0`).
    ParentInfo,
    /// `REPORT_CRIME`; the action id is `doodad_func_evidence_item_loots.skill_id`. The native
    /// location text is the player's sub-zone, zone entry and a localized text of category 0x5c;
    /// this port has the first two, see [`run`].
    Evidence { next_phase: i32, func_id: u32 },
}

/// Who says a line of a `bubbles` conversation (`FUN_39371fc0`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Speaker {
    /// A doodad in view: `CHAT_MSG_DOODAD` with its object id.
    Doodad(u32),
    /// An NPC in view or the player: `CHAT_MSG_QUEST` over the unit.
    Unit(u32),
}

/// A line of a doodad's bubble conversation, said at `at` (`Time::elapsed_secs`): `FUN_393745c0`
/// queues the whole chain at once, each line after the previous one's show and fade times.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct BubbleLine {
    pub at: f32,
    pub speaker: Speaker,
    pub author: String,
    pub text: String,
    pub kind: u8,
}

/// Raises a conversation line (`speech::say_bubble_line`).
pub(super) fn say_line(rt: &UiRuntime, line: &BubbleLine) {
    let (doodad, obj_id) = match line.speaker {
        Speaker::Doodad(obj_id) => (true, obj_id),
        Speaker::Unit(obj_id) => (false, obj_id),
    };
    super::super::speech::say_bubble_line(rt, doodad, obj_id, &line.author, &line.text, line.kind);
}

/// The confirmation of a purchase dialog (`FUN_393bc4e0` / `FUN_393bac20`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PurchaseOrder {
    /// `CSBuyItemsPacket` (0x0ae): doodad, item and currency. The native's `unk` field is its
    /// func id (`FUN_397ac3f0` fills it); the server reads that field and ignores it, and this
    /// client sends the func id there too.
    Buy {
        obj_id: u32,
        func_id: u32,
        item: u32,
        count: i32,
        currency: u8,
    },
    /// `CSBuyCoinItemPacket` (0x0af): the doodad and the func id.
    Coin {
        obj_id: u32,
        func_id: u32,
        coin_item: u32,
        coin_count: i32,
    },
}

/// `DoodadFuncInsertCounter` requirement and its permission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::hud) struct CounterUse {
    pub item: u32,
    pub count: u32,
    pub perm: u8,
}

/// The detail rows the local funcs read.
#[derive(Default)]
pub(super) struct LocalFuncs {
    farms: HashMap<u32, u32>,
    stores: HashMap<u32, u32>,
    /// func id -> (item template, number the bag or equipment must hold).
    insert_counters: HashMap<u32, (u32, u32)>,
    /// func id -> `doodad_func_bubbles.bubble_id`.
    bubbles: HashMap<u32, u32>,
    /// func id -> `doodad_func_open_papers`.
    papers: HashMap<u32, Paper>,
    /// func id -> `doodad_func_purchases`.
    purchases: HashMap<u32, Purchase>,
    /// func id -> `doodad_func_renew_items.skill_id`.
    renews: HashMap<u32, u32>,
    /// `doodad_func_coffer_perms` func ids whose template has a positive `DoodadFuncCoffer`
    /// (`FUN_3983b3b0` checks the whole template, not the current phase).
    coffer_perms: HashSet<u32>,
    /// func id -> `doodad_func_evidence_item_loots` (the action id is its `skill_id`, `FUN_393b7eb0`
    /// case 0x34 reads detail+8; the report text uses localized text category 0x5c, not the
    /// crime kind at detail+0).
    evidence: HashMap<u32, Evidence>,
}

impl LocalFuncs {
    pub(super) fn load(conn: &Connection) -> Self {
        let mut out = Self::default();
        out.farms = read_map(conn, "SELECT id, farm_id FROM doodad_func_open_farm_infos");
        out.stores = read_map(
            conn,
            "SELECT id, merchant_pack_id FROM doodad_func_store_uis",
        );
        read_pairs(
            conn,
            "SELECT id, item_id, item_count FROM doodad_func_insert_counters",
            |id, a, b| {
                let (item, count) = (a as u32, b as u32);
                if item != 0 && count != 0 {
                    out.insert_counters.insert(id, (item, count));
                }
            },
        );
        read_pairs(
            conn,
            "SELECT id, bubble_id, 0 FROM doodad_func_bubbles",
            |id, a, _| {
                out.bubbles.insert(id, a as u32);
            },
        );
        read_pairs(
            conn,
            "SELECT id, book_page_id, book_id FROM doodad_func_open_papers",
            |id, a, b| {
                out.papers.insert(
                    id,
                    Paper {
                        page: a as u32,
                        book: b as u32,
                    },
                );
            },
        );
        read_quads(
            conn,
            "SELECT id, item_id, count, coin_item_id, coin_count, currency_id FROM doodad_func_purchases",
            |id, item, count, coin_item, coin_count, currency| {
                out.purchases.insert(
                    id,
                    Purchase {
                        item: item as u32,
                        count: count as i32,
                        coin_item: coin_item as u32,
                        coin_count: coin_count as i32,
                        currency: currency as u8,
                    },
                );
            },
        );
        read_pairs(
            conn,
            "SELECT id, skill_id, 0 FROM doodad_func_renew_items",
            |id, a, _| {
                out.renews.insert(id, a as u32);
            },
        );
        read_pairs(
            conn,
            "SELECT id, skill_id, 0 FROM doodad_func_evidence_item_loots",
            |id, a, _| {
                if a > 0 {
                    out.evidence.insert(id, Evidence { skill: a as u32 });
                }
            },
        );
        if let Ok(mut q) = conn.prepare(
            "SELECT f.id FROM doodad_funcs f JOIN doodad_func_groups g ON g.id = f.doodad_func_group_id
             WHERE f.actual_func_type = 'DoodadFuncCofferPerm' AND g.doodad_almighty_id IN (
               SELECT g2.doodad_almighty_id FROM doodad_funcs f2 JOIN doodad_func_groups g2 ON g2.id = f2.doodad_func_group_id
               WHERE f2.actual_func_type = 'DoodadFuncCoffer' AND f2.actual_func_id > 0)",
        ) {
            if let Ok(rows) = q.query_map([], |r| crate::hud::data::any_int(r, 0)) {
                for id in rows.flatten() {
                    out.coffer_perms.insert(id as u32);
                }
            }
        }
        out
    }

    /// The local window of `kind`: `None` when that kind has no client window here (or its detail
    /// row is missing).
    pub(super) fn local(
        &self,
        kind: &str,
        func_id: u32,
        row_id: u32,
        next_phase: i32,
    ) -> Option<LocalAction> {
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
            "DoodadFuncAuctionUi" => Some(LocalAction::Auction),
            "DoodadFuncBubble" => Some(LocalAction::Bubble {
                bubble: self.bubbles.get(&func_id).copied().unwrap_or(0),
                next_phase,
                func_id: row_id,
            }),
            "DoodadFuncOpenPaper" => self.papers.get(&func_id).and_then(|paper| {
                paper.open().map(|(kind, id)| LocalAction::OpenPaper {
                    book: kind == "book",
                    id,
                    next_phase,
                    func_id: row_id,
                })
            }),
            "DoodadFuncPurchase" => {
                self.purchases
                    .get(&func_id)
                    .map(|order| LocalAction::Purchase {
                        order: *order,
                        func_id: row_id,
                    })
            }
            "DoodadFuncRenewItem" => self
                .renews
                .get(&func_id)
                .map(|skill| LocalAction::RenewItem { skill: *skill }),
            "DoodadFuncStampMaker" => Some(LocalAction::StampMaker),
            "DoodadFuncCofferPerm" => self
                .coffer_perms
                .contains(&row_id)
                .then_some(LocalAction::CofferPerm),
            "DoodadFuncParentInfo" => Some(LocalAction::ParentInfo),
            "DoodadFuncEvidenceItemLoot" => {
                self.evidence
                    .get(&func_id)
                    .map(|_| LocalAction::Evidence {
                        next_phase,
                        func_id: row_id,
                    })
            }
            _ => None,
        }
    }

    /// The insert-counter requirement of a func: `(item template, count)`.
    pub(super) fn insert_counter(&self, func_id: u32) -> Option<(u32, u32)> {
        self.insert_counters.get(&func_id).copied()
    }

    /// The action id of `DoodadFuncEvidenceItemLoot`: `FUN_393b7eb0` case 0x34 reads detail+8,
    /// the row's `skill_id` (11672 on all shipped rows). `None` when it is 0 (no bar entry).
    pub(super) fn evidence_action(&self, func_id: u32) -> Option<u32> {
        self.evidence.get(&func_id).map(|detail| detail.skill)
    }
}

/// Reads a two-column `id -> value` detail table; a missing table (a fixture) is empty.
fn read_map(conn: &Connection, sql: &str) -> HashMap<u32, u32> {
    let Ok(mut q) = conn.prepare(sql) else {
        return HashMap::new();
    };
    let Ok(rows) = q.query_map([], |r| {
        Ok((
            crate::hud::data::any_int(r, 0)?,
            crate::hud::data::any_int(r, 1)?,
        ))
    }) else {
        return HashMap::new();
    };
    rows.flatten()
        .filter(|(_, value)| *value != 0)
        .map(|(id, value)| (id as u32, value as u32))
        .collect()
}

/// Reads three columns (`id`, two values) into `f`.
fn read_pairs(conn: &Connection, sql: &str, mut f: impl FnMut(u32, i64, i64)) {
    let Ok(mut q) = conn.prepare(sql) else { return };
    let Ok(rows) = q.query_map([], |r| {
        Ok((
            crate::hud::data::any_int(r, 0)?,
            crate::hud::data::any_int(r, 1)?,
            crate::hud::data::any_int(r, 2)?,
        ))
    }) else {
        return;
    };
    for (id, a, b) in rows.flatten() {
        f(id as u32, a, b);
    }
}

/// Reads six columns (`id` and five values) into `f`.
fn read_quads(conn: &Connection, sql: &str, mut f: impl FnMut(u32, i64, i64, i64, i64, i64)) {
    let Ok(mut q) = conn.prepare(sql) else { return };
    let Ok(rows) = q.query_map([], |r| {
        Ok((
            crate::hud::data::any_int(r, 0)?,
            crate::hud::data::any_int(r, 1)?,
            crate::hud::data::any_int(r, 2)?,
            crate::hud::data::any_int(r, 3)?,
            crate::hud::data::any_int(r, 4)?,
            crate::hud::data::any_int(r, 5)?,
        ))
    }) else {
        return;
    };
    for (id, a, b, c, d, e) in rows.flatten() {
        f(id as u32, a, b, c, d, e);
    }
}

/// Runs a local func. Native `FUN_393bc260` always handles these: the skill is never sent, even
/// when the window cannot open here (no UI runtime, no position, an empty pack), except for the
/// open-paper func with neither a page nor a book, which `local()` already filters out.
/// `FUN_393b7b10` first ends the interaction that is active (`FUN_393b7aa0`), which
/// `end_interaction` mirrors; the new window then becomes the active interaction
/// (`View::local_state`). The store and bank are tracked by the store view.
/// Returns the phase change the native sends after a bubble or an open paper.
#[allow(clippy::too_many_arguments)]
pub(super) fn run(
    rt: Option<&UiRuntime>,
    bridge: &Shared,
    doodads: &Doodads,
    units: &Units,
    inventory: &Inventory,
    houses: Option<&aa_net::Houses>,
    obj_id: u32,
    skill_id: u32,
    local: LocalAction,
    now: f32,
) -> Option<Command> {
    let Some(rt) = rt else {
        return None;
    };
    // `FUN_393b7b10` -> `FUN_393b7aa0` inside the funcs that switch interactions. The bubble,
    // parent-info and evidence funcs do not call it.
    match local {
        LocalAction::Bubble { .. } | LocalAction::ParentInfo | LocalAction::Evidence { .. } => {}
        _ => {
            super::end_interaction(Some(rt), bridge, inventory);
        }
    }
    // The states `FUN_393b7b10` remembers in `DAT_3a17e6a0`; the next local interaction ends the
    // window through `FUN_393b7aa0` (`end_interaction`).
    let state = match local {
        LocalAction::OpenPaper { .. } => 1,
        LocalAction::RenewItem { .. } => 2,
        // `FUN_393b1040` sets interaction state 4 before `TOGGLE_MAIL`.
        LocalAction::Mailbox => 4,
        LocalAction::StampMaker => 7,
        LocalAction::Purchase { .. } => 8,
        LocalAction::CofferPerm => 0xd,
        LocalAction::Auction => 0xe,
        _ => 0,
    };
    if state != 0 {
        bridge.borrow_mut().doodad.local_state = Some(super::LocalState { state, obj_id });
    }
    match local {
        LocalAction::Mailbox => super::super::mail::open_at(rt, obj_id),
        LocalAction::FarmInfo(farm) => {
            // Without a world position the native window still opens; this port cannot place it,
            // so it stays closed rather than sending the skill.
            if let Some(pos) = doodads
                .by_id
                .get(&obj_id)
                .and_then(|o| crate::ingame::units::doodad_world(o, units))
                && super::super::farm::show_doodad(rt, farm as i32, pos)
            {
                // `FUN_393b8700` sets interaction state 10.
                bridge.borrow_mut().doodad.local_state =
                    Some(super::LocalState { state: 10, obj_id });
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
        // `FUN_393b8b80`; the window's requests then carry the doodad as `obj_id2`.
        LocalAction::Auction => super::super::auction::start_doodad(rt, obj_id),
        LocalAction::Bubble {
            bubble,
            next_phase,
            func_id,
        } => {
            play_bubble(rt, bridge, doodads, units, obj_id, bubble, now);
            return phase_change(obj_id, skill_id, next_phase, func_id);
        }
        LocalAction::OpenPaper {
            book,
            id,
            next_phase,
            func_id,
        } => {
            rt.fire_event("OPEN_PAPER", (if book { "book" } else { "page" }, id));
            return phase_change(obj_id, skill_id, next_phase, func_id);
        }
        LocalAction::Purchase { order, func_id } => {
            open_purchase(rt, bridge, obj_id, func_id, order);
        }
        LocalAction::RenewItem { skill } => {
            rt.fire_event("RENEW_ITEM_WND_SHOW", (skill, obj_id));
        }
        LocalAction::StampMaker => {
            rt.fire_event("OPEN_EMBLEM_UPLOAD_UI", obj_id.to_string());
        }
        LocalAction::CofferPerm => rt.fire_event("COFFER_INTERACTION_START", ()),
        LocalAction::ParentInfo => {
            if let Some(house) = doodads
                .by_id
                .get(&obj_id)
                .and_then(|o| super::house_of(o, houses))
            {
                super::super::housing::open_interaction(rt, house.obj_id);
            }
        }
        LocalAction::Evidence {
            next_phase,
            func_id,
        } => {
            bridge.borrow_mut().doodad.crime_report = Some(super::CrimeReport {
                obj_id,
                skill_id,
                next_phase,
                func_id,
            });
            let (name, text) = {
                let d = bridge.borrow();
                let game = d.game.get();
                let name = doodads
                    .by_id
                    .get(&obj_id)
                    .and_then(|o| game.and_then(|g| g.doodad.name(o.template_id)))
                    .unwrap_or("?")
                    .to_owned();
                // FUN_393bb090 builds the location from the player's position, never the target:
                // the sub-zone name (`sub_zones`, `FUN_3976ade0`), `>>`, the name of the player's
                // zone entry (`FUN_395779f0`), `>>`, then a localized text of category 0x5c
                // reached through that entry (`FUN_397f2180` -> `FUN_397f23d0`, not identified
                // here, so left out). The HUD snapshot has the first two.
                let text = [d.snapshot.sub_zone.as_str(), d.snapshot.zone.as_str()]
                    .into_iter()
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join(">>");
                (name, text)
            };
            rt.fire_event("REPORT_CRIME", (name, text));
        }
    }
    None
}

/// `CSChangeDoodadPhasePacket` when the func names a following phase (`next_phase > 0`).
fn phase_change(obj_id: u32, skill_id: u32, next_phase: i32, func_id: u32) -> Option<Command> {
    (next_phase > 0).then_some(Command::ChangeDoodadPhase {
        obj_id,
        skill_id,
        next_phase: next_phase as u32,
        func_id,
    })
}

/// `FUN_393703b0(doodadObjId, bubbleId)`: the `bubbles` conversation (`bubble_chats`, not
/// `bubble_effects`): `FUN_39375140` plays it from its `START` line along `next`. The first line
/// is said now, the rest wait in `View::bubble_lines` for `execute`. A bubble without lines says
/// nothing. The doodad screen position is still missing (`speech.rs`), so a doodad's line is
/// raised but not drawn yet.
fn play_bubble(
    rt: &UiRuntime,
    bridge: &Shared,
    doodads: &Doodads,
    units: &Units,
    obj_id: u32,
    bubble: u32,
    now: f32,
) {
    if bubble == 0 {
        return;
    }
    let lines = {
        let d = bridge.borrow();
        let Some(game) = d.game.get() else { return };
        let player = d
            .snapshot
            .unit_id
            .parse::<u32>()
            .ok()
            .map(|id| (id, d.snapshot.name.clone()));
        let mut at = now;
        let mut lines = Vec::new();
        for chat in game.speech.bubble_chain(bubble) {
            if let Some((speaker, author)) =
                speaker_of(chat, game, doodads, units, obj_id, player.as_ref())
            {
                lines.push(BubbleLine {
                    at,
                    speaker,
                    author,
                    text: chat.text.clone(),
                    kind: chat.kind,
                });
            }
            at += super::super::speech::bubble_line_secs(&chat.text);
        }
        lines
    };
    let mut later = Vec::new();
    for line in lines {
        if line.at <= now {
            say_line(rt, &line);
        } else {
            later.push(line);
        }
    }
    bridge.borrow_mut().doodad.bubble_lines.extend(later);
}

/// `FUN_39371fc0`: the line's doodad (`doodad_id`: the used doodad when it is of that template,
/// else the nearest one in view); without one, the player when the line names no NPC, else the
/// nearest unit of the `npc_id` template. `None` when that NPC is not in view.
fn speaker_of(
    chat: &super::super::speech::BubbleChat,
    game: &crate::hud::data::GameData,
    doodads: &Doodads,
    units: &Units,
    obj_id: u32,
    player: Option<&(u32, String)>,
) -> Option<(Speaker, String)> {
    let used = doodads.by_id.get(&obj_id);
    let near = |x: f32, y: f32, z: f32| {
        used.map_or(0.0, |o| {
            let (dx, dy, dz) = (x - o.pos.x, y - o.pos.y, z - o.pos.z);
            dx * dx + dy * dy + dz * dz
        })
    };
    if chat.doodad != 0 {
        let doodad = used
            .filter(|o| o.template_id == chat.doodad)
            .or_else(|| {
                doodads
                    .by_id
                    .values()
                    .filter(|o| o.template_id == chat.doodad)
                    .min_by(|a, b| {
                        near(a.pos.x, a.pos.y, a.pos.z).total_cmp(&near(b.pos.x, b.pos.y, b.pos.z))
                    })
            });
        if let Some(o) = doodad {
            let name = game.doodad.name(o.template_id).unwrap_or_default().to_owned();
            return Some((Speaker::Doodad(o.obj_id), name));
        }
    }
    if chat.npc == 0 {
        return player.map(|(id, name)| (Speaker::Unit(*id), name.clone()));
    }
    units
        .by_id
        .values()
        .filter(|u| {
            matches!(u.kind, aa_net::model::UnitKind::Npc { template_id, .. } if template_id == chat.npc)
        })
        .min_by(|a, b| near(a.pos.x, a.pos.y, a.pos.z).total_cmp(&near(b.pos.x, b.pos.y, b.pos.z)))
        .map(|u| (Speaker::Unit(u.obj_id), u.name.clone()))
}

/// The conversation lines whose turn has come (`FUN_393745c0`'s queue).
pub(super) fn due_bubble_lines(bridge: &Shared, now: f32) -> Vec<BubbleLine> {
    let mut d = bridge.borrow_mut();
    let (due, later): (Vec<BubbleLine>, Vec<BubbleLine>) = std::mem::take(&mut d.doodad.bubble_lines)
        .into_iter()
        .partition(|line| line.at <= now);
    d.doodad.bubble_lines = later;
    due
}

/// The original purchase dialog (`FUN_393b80d0`): task 10 when the row has no coin item or no
/// coin count, else the coin task 11. The dialog's OK calls [`PurchaseOrder`]'s packet path.
fn open_purchase(rt: &UiRuntime, bridge: &Shared, obj_id: u32, func_id: u32, order: Purchase) {
    let task = if order.coin_item == 0 || order.coin_count < 1 {
        TASK_PURCHASE
    } else {
        TASK_PURCHASE_COIN
    };
    let lua = &rt.lua;
    let Some(manager) = lua
        .globals()
        .get::<Option<mlua::Table>>("X2DialogManager")
        .ok()
        .flatten()
    else {
        return;
    };
    let (item_name, price) = {
        let d = bridge.borrow();
        d.game
            .get()
            .and_then(|g| g.items.get(&order.item))
            .map_or((String::new(), 0), |i| (i.named.name.clone(), i.price))
    };
    let info = match lua.create_table() {
        Ok(t) => t,
        Err(_) => return,
    };
    let item = lua.create_table().ok();
    let Some(item) = item else { return };
    let _ = item.set("name", item_name);
    let _ = item.set("itemType", order.item);
    let _ = item.set("lookType", order.item);
    let _ = item.set("itemGrade", 0);
    let _ = item.set("gradeColor", "FFFFFFFF");
    let _ = info.set("itemInfo", item);
    let _ = info.set("itemCount", order.count);
    if task == TASK_PURCHASE {
        let _ = info.set("moneyString", (price * i64::from(order.count)).to_string());
        let _ = info.set("currency", order.currency);
    } else {
        let coin_name = {
            let d = bridge.borrow();
            d.game
                .get()
                .and_then(|g| g.items.get(&order.coin_item))
                .map(|i| i.named.name.clone())
                .unwrap_or_default()
        };
        let _ = info.set("coinName", coin_name);
        let _ = info.set("coinCount", order.coin_count);
    }
    // `FUN_3902db50(10/11)` clears the previous task dialogs before the new one is created.
    let _ = manager.call_method::<()>("Delete", (TASK_PURCHASE,));
    let _ = manager.call_method::<()>("Delete", (TASK_PURCHASE_COIN,));
    let wnd: Option<mlua::Table> = manager
        .call_method("RequestTaskDialog", (task, info, obj_id))
        .ok();
    let Some(wnd) = wnd else { return };
    let state = bridge.clone();
    let dialog = wnd.clone();
    let done = lua.create_function(move |_, _: mlua::MultiValue| {
        // A dialog cancelled by `INTERACTION_END` (or replaced by the next interaction) must not
        // send the old doodad's purchase.
        if dialog.raw_get::<bool>("__dialog_closed").unwrap_or(false) {
            return Ok(());
        }
        let order = if task == TASK_PURCHASE {
            PurchaseOrder::Buy {
                obj_id,
                func_id,
                item: order.item,
                count: order.count,
                currency: order.currency,
            }
        } else {
            PurchaseOrder::Coin {
                obj_id,
                func_id,
                coin_item: order.coin_item,
                coin_count: order.coin_count,
            }
        };
        state.borrow_mut().doodad.purchases.push(order);
        Ok(())
    });
    if let Ok(done) = done {
        let _ = wnd.set("OkProc", done);
    }
}

/// The `ErrorMessageType` of the refusal (`error_names.txt`: 27 `NOT_ENOUGH_ITEM`).
pub(super) const NOT_ENOUGH_ITEM: i16 = 27;
/// `ErrorMessageType` 26 `NOT_ENOUGH_MONEY` (`FUN_394f0eb0`'s money branch shows it and still
/// lets `FUN_393bc4e0` send).
pub(super) const NOT_ENOUGH_MONEY: i16 = 26;
/// `ErrorMessageType` 87 `STORE_INVALID_ITEM` (`FUN_393bc4e0` when the item template is missing).
pub(super) const STORE_INVALID_ITEM: i16 = 87;

/// `FUN_393bc260` shows `NO_PERM` (error 2) when `FUN_393b3300` refuses the func.
pub(super) const NO_PERM: i16 = 2;

/// Why a purchase confirmation does not send (`FUN_393bc4e0` / `FUN_393bac20`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PurchaseRefusal {
    /// The item template is missing (`FUN_393bc4e0`: error 0x57).
    InvalidItem,
    /// Too few coins and the coin template is missing (`FUN_393bac20`: error 0x1b).
    UnknownCoin,
    /// The bag and equipment together hold fewer coins (`FUN_397cce20` + message 0x1e3).
    NotEnoughCoin { coin_item: u32, count: i32 },
}

/// The packet a pending purchase confirmation sends (`FUN_393bc4e0` / `FUN_393bac20`), and whether
/// the money path is short of money. For the money currency (every shipped row) `FUN_394f0eb0`
/// shows `NOT_ENOUGH_MONEY` when the money is below price * count and returns 0 either way, so
/// `FUN_393bc4e0` still sends `CSBuyItems` and the server answers. The coin path counts the bag
/// plus the equipment (`FUN_397cce20`) first and needs the coin template only for the shortage
/// message, like the native.
pub(super) fn purchase_command(
    order: PurchaseOrder,
    inventory: &Inventory,
    game: Option<&crate::hud::data::GameData>,
    money: i64,
) -> Result<(Command, bool), PurchaseRefusal> {
    match order {
        PurchaseOrder::Buy {
            obj_id,
            func_id,
            item,
            count,
            currency,
        } => {
            let Some(info) = game.and_then(|g| g.items.get(&item)) else {
                return Err(PurchaseRefusal::InvalidItem);
            };
            let short_of_money = currency == aa_net::shop::currency::MONEY
                && money < info.price.saturating_mul(i64::from(count));
            let command = Command::Shop(ShopCommand::Buy {
                npc_obj_id: 0,
                doodad_obj_id: obj_id,
                func_id,
                buy: vec![aa_net::shop::BuyEntry {
                    item_id: item,
                    grade: 0,
                    count,
                    currency,
                }],
                buy_back: Vec::new(),
                use_aa_point: false,
            });
            Ok((command, short_of_money))
        }
        PurchaseOrder::Coin {
            obj_id,
            func_id,
            coin_item,
            coin_count,
        } => {
            let bag: i32 = inventory
                .bag
                .iter()
                .flatten()
                .filter(|i| i.template_id == coin_item)
                .map(|i| i.count.max(0))
                .sum();
            let equipped: i32 = inventory
                .equipment
                .slots
                .iter()
                .flatten()
                .filter(|i| i.template_id == coin_item)
                .map(|i| i.count.max(0))
                .sum();
            if bag + equipped >= coin_count {
                return Ok((Command::Shop(ShopCommand::BuyCoin { obj_id, func_id }), false));
            }
            // only the shortage message needs the coin template (`FUN_39695dc0`)
            if game.is_none_or(|g| !g.items.contains_key(&coin_item)) {
                return Err(PurchaseRefusal::UnknownCoin);
            }
            Err(PurchaseRefusal::NotEnoughCoin {
                coin_item,
                count: coin_count,
            })
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

#[cfg(test)]
mod tests;
