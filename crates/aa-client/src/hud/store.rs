//! NPC service context, local merchant catalogs and original store actions.
use std::collections::{HashMap, HashSet};

use aa_net::model::{Item, ItemDetail};
use aa_net::shop::{BuyEntry, SellEntry, Shop, ShopCommand, ShopEvent};
use aa_net::state::{Doodads, Inventory, Player, Units};
use aa_net::{Command, Event, NetCommand, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, Table};
use rusqlite::Connection;

use super::api::{HudBridge, Shared};
use super::data::{GameData, any_flag, any_int};
use super::tips::{self, Instance};
use crate::Screen;

pub mod services;

#[derive(Clone, Debug, PartialEq)]
pub struct Good {
    pub item: u32,
    /// Database grade ID, retained for the wire. Lua uses grade_order.
    pub grade: u8,
    pub currency: u8,
}

#[derive(Default)]
pub struct Tables {
    pub merchants: HashMap<u32, Vec<Good>>,
    /// `merchant_packs.id` -> its goods. A `DoodadFuncStoreUi` names the pack directly, not an NPC.
    pub packs: HashMap<u32, Vec<Good>>,
    pub bankers: HashSet<u32>,
    pub auctioneers: HashSet<u32>,
    refund_multipliers: HashMap<u8, i64>,
    /// The other services of NPCs: priest, repair, ability change (see [`services`]).
    pub services: services::Tables,
}

impl Tables {
    pub fn load(conn: &Connection) -> rusqlite::Result<Self> {
        let mut out = Self::default();
        let mut q = conn.prepare("SELECT m.npc_id,g.item_id,g.grade_id,p.kind_id FROM merchants m JOIN merchant_packs p ON p.id=m.merchant_pack_id JOIN merchant_goods g ON g.merchant_pack_id=m.merchant_pack_id ORDER BY m.id,g.id")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            let npc = any_int(r, 0)? as u32;
            let item = any_int(r, 1)? as u32;
            // Native GetStoreCurrency: siege stores share honor currency with honor stores.
            let currency = match any_int(r, 3)? {
                1 | 2 => 1,
                3 => 2,
                4 => 3,
                5 => 4,
                _ => 0,
            };
            let goods = out.merchants.entry(npc).or_default();
            if !goods.iter().any(|g| g.item == item) {
                goods.push(Good {
                    item,
                    grade: any_int(r, 2)?.clamp(0, 11) as u8,
                    currency,
                });
            }
        }
        // A doodad's merchant pack need not belong to any NPC (the bait barrel's pack 274 has no
        // `merchants` row), so the packs come from `merchant_goods` directly.
        let mut q = conn.prepare("SELECT g.merchant_pack_id,g.item_id,g.grade_id,p.kind_id FROM merchant_packs p JOIN merchant_goods g ON g.merchant_pack_id=p.id ORDER BY g.id")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            let pack = any_int(r, 0)? as u32;
            let item = any_int(r, 1)? as u32;
            let currency = match any_int(r, 3)? {
                1 | 2 => 1,
                3 => 2,
                4 => 3,
                5 => 4,
                _ => 0,
            };
            let good = Good {
                item,
                grade: any_int(r, 2)?.clamp(0, 11) as u8,
                currency,
            };
            let goods = out.packs.entry(pack).or_default();
            if !goods.iter().any(|g| g.item == item) {
                goods.push(good);
            }
        }
        let mut q = conn.prepare("SELECT id,banker,auctioneer FROM npcs")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            if any_flag(r, 1)? {
                out.bankers.insert(r.get(0)?);
            }
            if any_flag(r, 2)? {
                out.auctioneers.insert(r.get(0)?);
            }
        }
        let mut q = conn.prepare("SELECT id,refund_multiplier FROM item_grades")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            out.refund_multipliers.insert(r.get(0)?, any_int(r, 1)?);
        }
        out.services = services::Tables::load(conn)?;
        Ok(out)
    }

    /// The goods of an NPC's store or of a doodad's merchant pack.
    pub fn catalog(&self, npc: (u32, u32), doodad: bool) -> Option<&Vec<Good>> {
        if doodad {
            self.packs.get(&npc.1)
        } else {
            self.merchants.get(&npc.1)
        }
    }

    pub fn has_service(&self, npc: u32) -> bool {
        self.merchants.contains_key(&npc)
            || self.bankers.contains(&npc)
            || self.auctioneers.contains(&npc)
    }

    pub fn refund(&self, base: i64, raw_grade: u8) -> i64 {
        let multiplier = self
            .refund_multipliers
            .get(&raw_grade)
            .copied()
            .unwrap_or(100);
        // The lane and original use single precision and truncate each item's unit price.
        (base.saturating_mul(multiplier) as f32 / 100.0) as i32 as i64
    }

    pub(super) fn refund_multiplier(&self, raw_grade: u8) -> Option<i32> {
        self.refund_multipliers
            .get(&raw_grade)
            .and_then(|value| i32::try_from(*value).ok())
    }
}

pub fn ui_grade(raw: u8) -> u8 {
    match raw {
        0 => 1,
        1 => 0,
        g => g,
    }
}

#[derive(Debug)]
enum Req {
    Buy(Vec<(usize, i32, u8)>),
    Sell(Vec<(usize, u64)>),
    BuyBack(usize),
    ListSold,
    Deposit(i32),
    Withdraw(i32),
}

#[derive(Default, Debug)]
pub struct View {
    pub npc: Option<(u32, u32)>,
    bank: bool,
    /// The window belongs to a doodad: the wire names `doodad_obj_id`, and the pack catalog is
    /// `Tables::packs`.
    pub doodad: bool,
    scope: Option<super::dialog::NpcScope>,
    requests: Vec<Req>,
    sold: Vec<Item>,
    sold_seen: bool,
    refresh_pending: bool,
    /// The interaction list and the service windows; they outlive the store window.
    pub(super) services: services::View,
}

impl View {
    /// A store window or a bank for `npc` (the services state stays).
    fn open(&mut self, npc: (u32, u32), bank: bool) {
        let services = std::mem::take(&mut self.services);
        *self = View {
            npc: Some(npc),
            bank,
            services,
            ..default()
        };
    }

    fn close(&mut self) {
        let services = std::mem::take(&mut self.services);
        *self = View {
            services,
            ..default()
        };
    }
}

pub fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        update
            .after(super::sync_hud)
            .after(super::dialog::drive_npc)
            .before(aa_ui::UiFrame)
            .run_if(super::hud_up)
            .run_if(in_state(Screen::InGame)),
    )
    .add_systems(
        Update,
        services::update
            .after(update)
            .before(aa_ui::UiFrame)
            .run_if(super::hud_up)
            .run_if(in_state(Screen::InGame)),
    );
}

pub fn install(lua: &Lua, hud: &Table, bridge: &Shared) -> mlua::Result<()> {
    services::install(lua, hud, bridge)?;
    let b = bridge.clone();
    hud.set(
        "store_goods",
        lua.create_function(move |lua, ()| {
            let d = b.borrow();
            let out = lua.create_table()?;
            let Some(game) = d.game.get() else {
                return Ok(out);
            };
            let Some(npc) = d.store.npc else {
                return Ok(out);
            };
            for (index, good) in game
                .stores
                .catalog(npc, d.store.doodad)
                .into_iter()
                .flatten()
                .enumerate()
            {
                if let Some(t) = goods_info(lua, game, &d.tips, good, index)? {
                    out.raw_push(t)?;
                }
            }
            Ok(out)
        })?,
    )?;
    let b = bridge.clone();
    hud.set(
        "store_buy",
        lua.create_function(
            move |_, (indices, counts, currencies): (Table, Table, Table)| {
                let indices = indices
                    .sequence_values::<usize>()
                    .take(16)
                    .collect::<mlua::Result<Vec<_>>>()?;
                let counts = counts
                    .sequence_values::<i32>()
                    .take(16)
                    .collect::<mlua::Result<Vec<_>>>()?;
                let currencies = currencies
                    .sequence_values::<u8>()
                    .take(16)
                    .collect::<mlua::Result<Vec<_>>>()?;
                if !indices.is_empty()
                    && indices.len() == counts.len()
                    && counts.len() == currencies.len()
                {
                    b.borrow_mut().store.requests.push(Req::Buy(
                        indices
                            .into_iter()
                            .zip(counts)
                            .zip(currencies)
                            .map(|((i, n), c)| (i, n, c))
                            .collect(),
                    ));
                }
                Ok(())
            },
        )?,
    )?;
    let b = bridge.clone();
    hud.set(
        "store_sell",
        lua.create_function(move |_, (slots, ids): (Table, Table)| {
            let slots = slots
                .sequence_values::<usize>()
                .take(16)
                .collect::<mlua::Result<Vec<_>>>()?;
            let ids = ids
                .sequence_values::<String>()
                .take(16)
                .collect::<mlua::Result<Vec<_>>>()?;
            if !slots.is_empty() && slots.len() == ids.len() {
                let entries = slots
                    .into_iter()
                    .zip(ids)
                    .map(|(slot, id)| Some((slot, id.parse::<u64>().ok()?)))
                    .collect::<Option<Vec<_>>>();
                if let Some(entries) = entries {
                    b.borrow_mut().store.requests.push(Req::Sell(entries));
                }
            }
            Ok(())
        })?,
    )?;
    let b = bridge.clone();
    hud.set(
        "store_buyback",
        lua.create_function(move |_, index: usize| {
            b.borrow_mut().store.requests.push(Req::BuyBack(index));
            Ok(())
        })?,
    )?;
    let b = bridge.clone();
    hud.set(
        "store_list_sold",
        lua.create_function(move |_, ()| {
            b.borrow_mut().store.requests.push(Req::ListSold);
            Ok(())
        })?,
    )?;
    for (name, deposit) in [("bank_deposit", true), ("bank_withdraw", false)] {
        let b = bridge.clone();
        hud.set(
            name,
            lua.create_function(move |_, amount: String| {
                if let Ok(amount) = amount.parse::<i32>() {
                    if amount > 0 {
                        b.borrow_mut().store.requests.push(if deposit {
                            Req::Deposit(amount)
                        } else {
                            Req::Withdraw(amount)
                        });
                    }
                }
                Ok(())
            })?,
        )?;
    }
    Ok(())
}

fn goods_info(
    lua: &Lua,
    game: &GameData,
    state: &super::tips::TipState,
    good: &Good,
    index: usize,
) -> mlua::Result<Option<Table>> {
    let Some(info) = game.items.get(&good.item) else {
        return Ok(None);
    };
    let Some(t) = tips::item_tip(
        lua,
        game,
        state,
        good.item,
        Some(i64::from(ui_grade(good.grade))),
        Some(1),
        Instance::default(),
    )?
    else {
        return Ok(None);
    };
    t.set("goodIndex", index + 1)?;
    t.set("currency", good.currency)?;
    t.set("cost", info.price)?;
    t.set("money", info.price)?;
    t.set("honorPrice", info.honor_price)?;
    t.set("livingPointPrice", info.living_price)?;
    t.set("maxStack", info.max_stack.max(1))?;
    t.set("path", info.named.icon.as_str())?;
    Ok(Some(t))
}

fn sold_info(
    lua: &Lua,
    game: &GameData,
    state: &super::tips::TipState,
    sold: &[Item],
) -> mlua::Result<Table> {
    let out = lua.create_table()?;
    for item in sold {
        let instance = match &item.detail {
            ItemDetail::Equip(e) => Instance {
                durability: Some(e.durability),
                temper: (e.temper_physical, e.temper_magical),
                flags: Some(item.flags),
            },
            _ => Instance {
                flags: Some(item.flags),
                ..default()
            },
        };
        if let Some(t) = tips::item_tip(
            lua,
            game,
            state,
            item.template_id,
            Some(i64::from(ui_grade(item.grade))),
            Some(i64::from(item.count)),
            instance,
        )? {
            out.raw_push(t)?;
        }
    }
    Ok(out)
}

fn build_command(
    req: Req,
    npc: (u32, u32),
    doodad: bool,
    game: &GameData,
    inv: &Inventory,
    sold: &[Item],
) -> Option<ShopCommand> {
    // A doodad store names the doodad in `doodad_obj_id`; an NPC store names the NPC. Sell and the
    // sold list have only the NPC field on the wire, so both use the object id the server knows.
    let (npc_obj_id, doodad_obj_id) = if doodad { (0, npc.0) } else { (npc.0, 0) };
    Some(match req {
        Req::ListSold => ShopCommand::ListSold { npc_obj_id: npc.0 },
        Req::BuyBack(index) if index > 0 && index <= sold.len() => ShopCommand::Buy {
            npc_obj_id,
            doodad_obj_id,
            buy: vec![],
            buy_back: vec![(index - 1) as i32],
            use_aa_point: false,
        },
        Req::Buy(entries) if !entries.is_empty() => {
            let catalog = game.stores.catalog(npc, doodad)?;
            let mut buy = Vec::new();
            for (index, count, currency) in entries {
                let good = catalog.get(index.checked_sub(1)?)?;
                if count <= 0 || currency != good.currency || currency > 2 {
                    return None;
                }
                buy.push(BuyEntry {
                    item_id: good.item,
                    grade: good.grade,
                    count,
                    currency,
                });
            }
            ShopCommand::Buy {
                npc_obj_id,
                doodad_obj_id,
                buy,
                buy_back: vec![],
                use_aa_point: false,
            }
        }
        Req::Sell(slots) if !slots.is_empty() => {
            let mut items = Vec::new();
            for (slot, expected_id) in slots {
                let index = slot.checked_sub(1)?;
                let item = inv.bag.get(index)?.as_ref()?;
                if item.id != expected_id
                    || expected_id == 0
                    || !game.items.get(&item.template_id)?.sellable
                    || index > u8::MAX as usize
                {
                    return None;
                }
                items.push(SellEntry {
                    container: aa_net::items::container::INVENTORY,
                    slot: index as u8,
                    item_id: item.id,
                });
            }
            ShopCommand::Sell {
                npc_obj_id: npc.0,
                items,
            }
        }
        _ => return None,
    })
}

/// Closes the store/bank window that is open, with its native end events: `FUN_393b7aa0` raises
/// `NPC_INTERACTION_END` (0x6c) for interaction states 0xe/0xf/0x11. `true` when one was open.
pub(super) fn end_interaction(rt: &UiRuntime, bridge: &Shared) -> bool {
    let mut data = bridge.borrow_mut();
    if data.store.npc.is_none() {
        return false;
    }
    let scope = data.store.scope.take();
    if let Some(scope) = scope {
        // `close_npc` owes the end through `flush_ends`; firing it here too would duplicate it.
        super::dialog::close_npc(&mut data, scope);
    }
    data.store.close();
    drop(data);
    if scope.is_none() {
        // A doodad store/bank has no NPC flow to flush its end.
        rt.fire_event("NPC_INTERACTION_END", ());
    }
    true
}

/// A `DoodadFuncStoreUi` / `DoodadFuncBankUi`: native `FUN_393b8cc0` / `FUN_393b8c20` open the
/// window locally with `NPC_INTERACTION_START("store" | "bank")` (one argument,
/// `FUN_39424240(0x6b, &one_arg)`) and send no packet. A store's catalog is the func's
/// `merchant_pack_id`, not an NPC's; a bank has no catalog.
///
/// Native `FUN_393b7b10` closes the active interaction (an open NPC store included) first.
pub(super) fn open_doodad(
    rt: &UiRuntime,
    bridge: &Shared,
    obj_id: u32,
    pack: u32,
    bank: bool,
) -> bool {
    if end_interaction(rt, bridge) {
        rt.fire_event("INTERACTION_END", ());
    }
    {
        let mut data = bridge.borrow_mut();
        data.store.open((obj_id, pack), bank);
        data.store.doodad = true;
    }
    rt.fire_event("NPC_INTERACTION_START", if bank { "bank" } else { "store" });
    true
}

/// Called only by the owner for the selected, received action.
pub(super) fn start_selected(
    rt: &UiRuntime,
    bridge: &Shared,
    selected: super::dialog::SelectedNpc,
) -> bool {
    let npc = selected.npc();
    let mut data = bridge.borrow_mut();
    if !super::dialog::scope_current(&data, selected.scope()) {
        return false;
    }
    data.store.open(npc, selected.skill() == 13238);
    data.store.scope = Some(selected.scope());
    drop(data);
    rt.fire_event(
        "NPC_INTERACTION_START",
        (
            if selected.skill() == 13238 {
                "bank"
            } else {
                "store"
            },
            "",
            npc.0.to_string(),
        ),
    );
    true
}

#[allow(clippy::too_many_arguments)]
pub(super) fn update(
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<HudBridge>,
    player: Res<Player>,
    units: Res<Units>,
    inventory: Res<Inventory>,
    shop: Res<Shop>,
    doodads: Option<Res<Doodads>>,
    mut events: MessageReader<NetEvent>,
    mut net: MessageWriter<NetCommand>,
) {
    let Some(rt) = rt else { return };
    for NetEvent(event) in events.read() {
        if let Event::Shop(ShopEvent::RequestRejected(message)) = event {
            warn!("hud: shop request: {message}");
            rt.fire_event("STORE_TRADE_FAILED", ());
        }
    }
    let (context, me, bank, doodad) = {
        let d = bridge.0.borrow();
        (d.store.npc, d.pos, d.store.bank, d.store.doodad)
    };
    let Some(npc) = context else { return };
    let visible = rt
        .lua
        .globals()
        .get::<mlua::Function>(if bank {
            "IsActivatedBank"
        } else {
            "IsActivatedStore"
        })
        .and_then(|f| f.call::<bool>(()))
        .unwrap_or(false);
    // A doodad store follows the doodad: native `FUN_393b7e40` ends the interaction at the same
    // measured 2.1 m surface gap as any doodad use. A fallback gap (no drawn bounds) cannot prove
    // the player left, so it does not close (the same rule `needs_walk` uses). An NPC store keeps
    // the talk target and its dialog.
    let close = if doodad {
        !visible
            || doodads.as_deref().is_none_or(|doodads| {
                super::doodad::left_interaction(&bridge.0.borrow(), doodads, &units, npc.0)
            })
    } else {
        !visible
            || !units
                .get(npc.0)
                .is_some_and(|u| me.distance(Vec3::new(u.pos.x, u.pos.y, u.pos.z)) <= 3.0)
            || player.spawn.is_none()
            || bridge.0.borrow().dialog.npc != Some(npc)
            || !bridge
                .0
                .borrow()
                .store
                .scope
                .is_some_and(|scope| super::dialog::scope_current(&bridge.0.borrow(), scope))
    };
    if close {
        let was_doodad = doodad;
        let mut data = bridge.0.borrow_mut();
        if let Some(scope) = data.store.scope {
            super::dialog::close_npc(&mut data, scope);
        }
        data.store.close();
        drop(data);
        // `FUN_393b7aa0`: the store/bank states raise NPC_INTERACTION_END before INTERACTION_END.
        if was_doodad {
            rt.fire_event("NPC_INTERACTION_END", ());
            rt.fire_event("INTERACTION_END", ());
        }
        return;
    }
    let requests = std::mem::take(&mut bridge.0.borrow_mut().store.requests);
    for request in requests {
        match request {
            Req::Deposit(amount) | Req::Withdraw(amount) if bank => {
                let command = if matches!(request, Req::Deposit(_)) {
                    aa_net::bank::BankCommand::DepositMoney {
                        amount,
                        aa_point: 0,
                    }
                } else {
                    aa_net::bank::BankCommand::WithdrawMoney {
                        amount,
                        aa_point: 0,
                    }
                };
                net.write(NetCommand(Command::Bank(command)));
                continue;
            }
            _ if bank => continue,
            _ => {}
        }
        let command = {
            let d = bridge.0.borrow();
            d.game
                .get()
                .and_then(|g| build_command(request, npc, doodad, g, &inventory, &shop.sold))
        };
        if let Some(command) = command {
            if matches!(command, ShopCommand::ListSold { .. }) {
                bridge.0.borrow_mut().store.refresh_pending = true;
            }
            info!("hud: shop request {command:?}");
            net.write(NetCommand(Command::Shop(command)));
        } else {
            rt.fire_event("STORE_TRADE_FAILED", ());
        }
    }
    if bank {
        return;
    }
    if shop.loaded {
        let push = {
            let mut d = bridge.0.borrow_mut();
            let changed = !d.store.sold_seen || d.store.sold != shop.sold;
            d.store.sold_seen = true;
            d.store.sold.clone_from(&shop.sold);
            if !shop.dirty {
                d.store.refresh_pending = false;
            }
            changed
        };
        if push {
            let list = {
                let d = bridge.0.borrow();
                d.game
                    .get()
                    .map(|g| sold_info(&rt.lua, g, &d.tips, &shop.sold))
            };
            match list {
                Some(Ok(list)) => rt.fire_event("STORE_SOLD_LIST", list),
                Some(Err(e)) => warn!("hud: sold list: {e}"),
                None => {}
            }
        }
    }
    if shop.dirty && !bridge.0.borrow().store.refresh_pending {
        bridge.0.borrow_mut().store.refresh_pending = true;
        net.write(NetCommand(Command::Shop(ShopCommand::ListSold {
            npc_obj_id: npc.0,
        })));
    }
}

pub fn load_script(lua: &Lua) -> mlua::Result<()> {
    lua.load(include_str!("store.lua"))
        .set_name("@store.lua")
        .exec()?;
    lua.load(include_str!("bank.lua"))
        .set_name("@bank.lua")
        .exec()
}

#[cfg(test)]
mod tests {
    use super::super::data::{ItemInfo, Named};
    use super::*;
    use aa_net::interaction::InteractEvent;

    /// Replacing an NPC store with a local window must not add a second
    /// `NPC_INTERACTION_END`: `close_npc` already owes the talk's end through `flush_ends`.
    #[test]
    fn replacing_an_npc_store_fires_one_npc_end() -> Result<(), Box<dyn std::error::Error>> {
        if !aa_cry::pak_available() {
            return Ok(());
        }
        let (rt, shared, _path, inventory) =
            crate::hud::expansion::fixture::runtime(1280.0, 720.0)?;
        rt.lua
            .load(
                "NPC_ENDS=0; local w=UIParent:CreateWidget('emptywidget','npcStoreEndProbe','UIParent');                  w:RegisterEvent('NPC_INTERACTION_END');                  w:SetHandler('OnEvent',function() NPC_ENDS=NPC_ENDS+1 end)",
            )
            .exec()?;
        let spawn = crate::hud::fixture_spawn();
        let player = Player {
            spawn: Some(spawn.clone()),
        };
        let mut npc = spawn.unit.clone();
        npc.obj_id = 0x010203;
        npc.kind = aa_net::model::UnitKind::Npc {
            template_id: 1024,
            owner_id: 0,
        };
        npc.pos = aa_net::Pos::new(1.0, 0.0, 0.0);
        npc.dead = false;
        let mut units = Units::default();
        units.by_id.insert(npc.obj_id, npc.clone());
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_non_send(HudBridge(shared.clone()))
            .insert_non_send(rt)
            .insert_resource(player)
            .insert_resource(units)
            .insert_resource(inventory)
            .init_resource::<aa_net::state::PlayerStats>()
            .init_resource::<aa_net::state::Quests>()
            .init_resource::<aa_net::state::Skills>()
            .init_resource::<aa_net::state::Buffs>()
            .init_resource::<crate::hud::HudSuspended>()
            .insert_resource(crate::hud::HudBuilt {
                up: true,
                fresh: false,
            })
            .add_message::<NetEvent>()
            .add_message::<NetCommand>()
            .add_systems(
                Update,
                (crate::hud::dialog::track_npc, crate::hud::dialog::drive_npc).chain(),
            );
        for event in [
            Event::CharacterSelected(Box::new(spawn.state.clone())),
            Event::PlayerSpawn(Box::new(spawn.clone())),
            Event::EnteredWorld,
        ] {
            app.world_mut().write_message(NetEvent(event));
        }
        app.world_mut()
            .write_message(NetEvent(Event::UnitSpawned(Box::new(npc.clone()))));
        app.update();
        {
            let mut data = shared.borrow_mut();
            if let Ok(game) = crate::hud::data::GameData::load(aa_anim::db::DEFAULT_DB) {
                assert!(data.game.0.set(game).is_ok());
            }
            data.pos = Vec3::ZERO;
            data.panels.target = Some(crate::hud::panels::TargetSnap {
                obj_id: npc.obj_id,
                npc_type: Some(1024),
                detail: "npc",
                ..Default::default()
            });
            data.target_pos = Some(Vec3::X);
            crate::hud::dialog::query_npc(&mut data, (npc.obj_id, 1024), None);
        }
        app.update();
        app.update();
        app.world_mut()
            .write_message(NetEvent(Event::Interaction(InteractEvent::SkillList {
                npc_obj_id: npc.obj_id,
                obj_id: 0,
                extra_info: 0,
                pick_id: 0,
                mouse_button: 2,
                modifier_keys: 0,
                skills: vec![12_087],
                interactable: true,
            })));
        app.update();
        let lua = &app.world().non_send::<UiRuntime>().lua;
        assert!(lua.load("return IsActivatedStore()").eval::<bool>()?);
        assert!(shared.borrow().store.npc.is_some());
        // Count only the end the replacement causes; the talk's own target-change end already
        // fired when the store opened.
        lua.load("NPC_ENDS=0").exec()?;

        {
            let rt = app.world().non_send::<UiRuntime>();
            let bridge = app.world().non_send::<HudBridge>();
            assert!(end_interaction(rt, &bridge.0));
        }
        app.update();
        let lua = &app.world().non_send::<UiRuntime>().lua;
        assert_eq!(lua.load("return NPC_ENDS").eval::<i64>()?, 1);
        assert!(shared.borrow().store.npc.is_none());
        Ok(())
    }

    #[test]
    fn catalog_preserves_native_currency_order_and_first_duplicate() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE merchants(id,npc_id,merchant_pack_id); CREATE TABLE merchant_packs(id,kind_id); CREATE TABLE merchant_goods(id,merchant_pack_id,item_id,grade_id); CREATE TABLE npcs(id,banker,auctioneer,priest,blacksmith,repairman,ability_changer,look_converter); CREATE TABLE item_grades(id,refund_multiplier); CREATE TABLE priest_buffs(id,cost); CREATE TABLE content_configs(id,value); CREATE TABLE formulas(id,formula);
            INSERT INTO merchants VALUES(2,10,2),(1,10,1),(3,11,3),(4,12,4);
            INSERT INTO merchant_packs VALUES(1,0),(2,2),(3,3),(4,1);
            INSERT INTO merchant_goods VALUES(2,1,100,0),(1,1,101,1),(3,2,100,5),(4,2,102,2),(5,3,103,0),(6,4,104,0);
            INSERT INTO npcs VALUES(1,'f','f','f','f','f','f','f'),(2,'t','f','f','f','f','f','f'); INSERT INTO item_grades VALUES(0,100),(1,75),(2,115);").unwrap();
        let tables = Tables::load(&db).unwrap();
        assert_eq!(
            tables.merchants[&10],
            vec![
                Good {
                    item: 101,
                    grade: 1,
                    currency: 0
                },
                Good {
                    item: 100,
                    grade: 0,
                    currency: 0
                },
                Good {
                    item: 102,
                    grade: 2,
                    currency: 1
                }
            ]
        );
        assert_eq!(tables.merchants[&11][0].currency, 2);
        assert_eq!(tables.merchants[&12][0].currency, 1);
        assert!(tables.has_service(2));
        assert!(!tables.has_service(1));
        assert_eq!(tables.refund(9, 2), 10);
        assert_eq!(tables.refund(0, 0), 0);
        assert_eq!((ui_grade(0), ui_grade(1)), (1, 0));
    }

    fn fixture() -> GameData {
        let mut game = GameData::default();
        game.items.insert(
            100,
            ItemInfo {
                named: Named {
                    name: "Supply".into(),
                    icon: "ui/icon/test.dds".into(),
                },
                price: 25,
                refund: 0,
                sellable: true,
                max_stack: 20,
                ..default()
            },
        );
        game.stores.merchants.insert(
            10,
            vec![Good {
                item: 100,
                grade: 1,
                currency: 0,
            }],
        );
        game
    }

    /// A doodad store names the doodad in `doodad_obj_id` and reads the func's merchant pack, not an
    /// NPC catalog.
    #[test]
    fn a_doodad_store_names_the_doodad_and_uses_its_pack() {
        let mut game = fixture();
        game.stores.packs.insert(
            274,
            vec![Good {
                item: 100,
                grade: 1,
                currency: 0,
            }],
        );
        let inv = Inventory::default();
        let Some(ShopCommand::Buy {
            npc_obj_id,
            doodad_obj_id,
            buy,
            ..
        }) = build_command(Req::Buy(vec![(1, 2, 0)]), (500, 274), true, &game, &inv, &[])
        else {
            panic!()
        };
        assert_eq!((npc_obj_id, doodad_obj_id), (0, 500));
        assert_eq!(buy.len(), 1);
        assert_eq!(buy[0].item_id, 100);
    }

    /// The bait barrel's pack 274 has no `merchants` row; its goods still load for the doodad store.
    #[test]
    fn a_doodad_pack_without_an_npc_loads() {
        let Some(path) = std::env::var("AA_DB")
            .ok()
            .filter(|path| std::path::Path::new(path).exists())
        else {
            return;
        };
        let conn = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let tables = Tables::load(&conn).unwrap();
        assert!(
            !tables.packs.get(&274).map_or(true, Vec::is_empty),
            "pack 274 (bait barrel) has no goods"
        );
    }

    #[test]
    fn shop_uses_catalog_grade_and_actual_inventory_identity() {
        let game = fixture();
        let mut inv = Inventory::default();
        inv.bag = vec![Some(Item {
            id: 999,
            template_id: 100,
            count: 3,
            ..default()
        })];
        let Some(ShopCommand::Buy { buy, .. }) =
            build_command(Req::Buy(vec![(1, 2, 0)]), (500, 10), false, &game, &inv, &[])
        else {
            panic!()
        };
        assert_eq!(
            buy,
            vec![BuyEntry {
                item_id: 100,
                grade: 1,
                count: 2,
                currency: 0
            }]
        );
        let Some(ShopCommand::Sell { items, .. }) =
            build_command(Req::Sell(vec![(1, 999)]), (500, 10), false, &game, &inv, &[])
        else {
            panic!()
        };
        assert_eq!(
            items,
            vec![SellEntry {
                container: 2,
                slot: 0,
                item_id: 999
            }]
        );
        for req in [
            Req::Buy(vec![(0, 1, 0)]),
            Req::Buy(vec![(2, 1, 0)]),
            Req::Buy(vec![(1, 0, 0)]),
            Req::Buy(vec![(1, 1, 1)]),
            Req::Sell(vec![(0, 999)]),
            Req::Sell(vec![(2, 999)]),
            Req::Sell(vec![(1, 888)]),
            Req::BuyBack(1),
        ] {
            assert!(build_command(req, (500, 10), false, &game, &inv, &[]).is_none());
        }
        let sold = vec![inv.bag[0].clone().unwrap()];
        let Some(ShopCommand::Buy { buy_back, .. }) =
            build_command(Req::BuyBack(1), (500, 10), false, &game, &inv, &sold)
        else {
            panic!()
        };
        assert_eq!(buy_back, vec![0]);
    }

    #[test]
    fn zero_grade_is_crude_and_zero_refund_stays_zero_in_real_tooltip() {
        let lua = Lua::new();
        let game = fixture();
        let tip = tips::item_tip(&lua, &game, &default(), 100, Some(0), Some(3), default())
            .unwrap()
            .unwrap();
        assert_eq!(tip.get::<i64>("itemGrade").unwrap(), 0);
        assert_eq!(tip.get::<i64>("refund").unwrap(), 0);
        let goods = goods_info(&lua, &game, &default(), &game.stores.merchants[&10][0], 0)
            .unwrap()
            .unwrap();
        assert_eq!(goods.get::<i64>("itemGrade").unwrap(), 0);
        assert_eq!(goods.get::<i64>("cost").unwrap(), 25);
        assert_eq!(goods.get::<i64>("goodIndex").unwrap(), 1);
    }

    #[test]
    fn bank_window_routes_filtered_and_partial_moves_without_mutating_slots() {
        let lua = Lua::new();
        lua.load(r#"
            X2Bank={}; X2Bag={}; X2Util={}; X2Item={}; X2Cursor={}; FAKE={}; CURRENCY_GOLD=0
            HUD={bank_slots=3,bag_slots=3,state={bank_money='12345'},bank={[1]={id=10,grade=1,count=8,stack=true,group=7}},bag={[1]={id=20,count=1}}}
            moves={}; currentHeld=nil; oldClicks=0
            function HUD.drag_move(...) moves[#moves+1]={...} end
            function HUD.shift_down() return false end
            function HUD.item_tip(id,grade,count) return {lookType=id,itemGrade=grade,stack=count} end
            function FAKE.fire_later(e) end
            function FAKE.fire_now(e,...) end
            function X2Item:GetItemIconSet() return {} end
            function X2Cursor:GetHeld() return currentHeld end
            function X2Cursor:SetHeld(h) currentHeld=h end
            function X2Cursor:ClearCursor() currentHeld=nil end
            function X2Bag:HandleClick() oldClicks=oldClicks+1 end
            function X2Bag:HandleUse() end
            function X2Bag:SlotByIdx(i) return HUD.bag[i] and i end
            function X2Bag:CurrentTabIdx() return 1 end
            function X2Bag:TabCount() return 1 end
            function X2Bag:IsLocked() return false end
            function IsActivatedBank() return true end
            function BagItemToEmptyBankSlot(slot) X2Bag:MoveToEmptyBankSlot(slot); return true end
        "#).exec().unwrap();
        lua.load(include_str!("bank.lua")).exec().unwrap();
        lua.load(r#"
            assert(X2Util:GetMyBankMoneyString()=='12345')
            assert(X2Bank:Capacity()==3 and X2Bank:CountItems()==1)
            assert(X2Bank:GetBagItemInfo(1).itemGrade==1)
            X2Bank:CreateTab(2,'Materials',{7})
            assert(X2Bank:SlotByIdx(1)==1 and X2Bank:SlotByIdx(2)==nil)
            X2Bank:PickupBankItemPartial(1,3)
            X2Bag:HandleClick(2)
            assert(#moves==1 and moves[1][1]==3 and moves[1][2]==1 and moves[1][3]==2 and moves[1][4]==2 and moves[1][5]==3)
            X2Bag:HandleUse(1)
            assert(#moves==2 and moves[2][1]==2 and moves[2][2]==1 and moves[2][3]==3 and moves[2][4]==2)
            X2Bank:HandleUse(1)
            assert(#moves==3 and moves[3][1]==3 and moves[3][3]==2)
            assert(HUD.bank[1].count==8 and HUD.bag[1].id==20 and HUD.bank[2]==nil)
            X2Bag:HandleClick(1); assert(oldClicks==1)
            X2Bank:RemoveTab(2); assert(X2Bank:CurrentTabIdx()==1 and X2Bank:TabCount()==1)
        "#).exec().unwrap();
    }

    #[test]
    fn sell_cart_keeps_locked_identity_when_a_server_update_replaces_the_slot() {
        let lua = Lua::new();
        let shared = std::rc::Rc::new(std::cell::RefCell::new(
            super::super::api::HudData::default(),
        ));
        let hud = lua.create_table().unwrap();
        install(&lua, &hud, &shared).unwrap();
        lua.globals().set("HUD", hud).unwrap();
        lua.load("X2Bag={}; X2Store={}; X2Interaction={}; HUD.bag={[1]={uid='999'}}")
            .exec()
            .unwrap();
        lua.load(include_str!("store.lua")).exec().unwrap();
        // The original submit callback queues the sale before unlocking its cart rows.
        lua.load("X2Bag:LockSlot(1); assert(X2Bag:IsLocked(1)); HUD.bag[1]={uid='888'}; X2Store:SellStoreItem({1},1,false); X2Bag:UnlockSlot(1); assert(not X2Bag:IsLocked(1))").exec().unwrap();
        let req = shared.borrow_mut().store.requests.pop().unwrap();
        let Req::Sell(entries) = &req else { panic!() };
        assert_eq!(entries, &vec![(1, 999)]);
        let mut inv = Inventory::default();
        inv.bag = vec![Some(Item {
            id: 888,
            template_id: 100,
            ..default()
        })];
        assert!(build_command(req, (500, 10), false, &fixture(), &inv, &[]).is_none());
    }
}
