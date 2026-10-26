//! Original player-trade controls over acknowledged offers, locks and settlement.
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;
use web_time::Instant;

use aa_net::model::{Item, ItemDetail};
use aa_net::state::{Inventory, PlayerStats, Units};
use aa_net::trade::{Trade, TradeCommand, TradeEvent};
use aa_net::{Command, Event, NetCommand, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, ObjectLike, Table};

use super::api::{HudBridge, Shared};

pub(super) mod fixture;
#[cfg(test)]
mod tests;

struct Queued {
    command: TradeCommand,
    expected: Option<(u8, u64, i32)>,
}

#[derive(Default)]
struct View {
    initialized: bool,
    trade: Trade,
    inventory: Inventory,
    money: i64,
    commands: Vec<Queued>,
    pending: Vec<(TradeCommand, Instant)>,
    suppress_money: bool,
    notices: Vec<String>,
}

impl View {
    fn queue(&mut self, command: TradeCommand, expected: Option<(u8, u64, i32)>) -> bool {
        if command.validate().is_err() {
            return false;
        }
        if matches!(command, TradeCommand::OfferMoney { .. }) {
            // Retain the last edit in this frame. An equal amount sent earlier must not
            // suppress a new correction after an intervening amount already went out.
            self.commands.retain(|q| !matches!(q.command, TradeCommand::OfferMoney { .. }));
            self.commands.push(Queued { command, expected });
            return true;
        }
        if self.commands.iter().any(|q| q.command == command) || self.pending.iter().any(|(c, _)| *c == command) || self.trade.pending.contains(&command) {
            return false;
        }
        self.commands.push(Queued { command, expected });
        true
    }

    fn refuse(&mut self, message: &str) -> bool {
        self.notices.push(message.to_owned());
        false
    }
}

#[derive(Clone, Default)]
struct State(Rc<RefCell<View>>);

fn coin(text: &str) -> Option<i64> {
    if text.is_empty() { Some(0) } else { text.parse().ok().filter(|n| *n >= 0) }
}

fn money(gold: &str, silver: &str, copper: &str) -> Option<i32> {
    let g = coin(gold)?.checked_mul(10_000)?;
    let s = coin(silver).filter(|s| *s < 100)?.checked_mul(100)?;
    let c = coin(copper).filter(|c| *c < 100)?;
    i32::try_from(g.checked_add(s)?.checked_add(c)?).ok()
}

fn current_item(lua: &Lua, bridge: &Shared, item: &Item) -> mlua::Result<Option<Table>> {
    let d = bridge.borrow();
    let Some(game) = d.game.get() else { return Ok(None) };
    let instance = match &item.detail {
        ItemDetail::Equip(e) => super::tips::Instance {
            durability: Some(e.durability),
            temper: (e.temper_physical, e.temper_magical),
            flags: Some(item.flags),
        },
        _ => super::tips::Instance {
            flags: Some(item.flags),
            ..default()
        },
    };
    super::tips::item_tip(
        lua,
        game,
        &d.tips,
        item.template_id,
        Some(i64::from(super::store::ui_grade(item.grade))),
        Some(i64::from(item.count)),
        instance,
    )
}

fn offers_current(trade: &Trade, inventory: &Inventory) -> bool {
    trade.mine.iter().all(|o| {
        o.item
            .as_ref()
            .is_some_and(|item| inventory.item_at(o.container, usize::from(o.slot)) == Some(item))
    })
}

pub fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    let state = State::default();
    lua.set_app_data(state.clone());
    let trade: Table = lua.globals().get("X2Trade")?;
    let s = state.clone();
    let b = bridge.clone();
    trade.set(
        "RequestTrade",
        lua.create_function(move |_, _: Table| {
            let d = b.borrow();
            let Some(target) = d
                .panels
                .target
                .as_ref()
                .filter(|t| t.unit_type == "character" && t.relation != "hostile" && t.obj_id.to_string() != d.snapshot.unit_id)
            else {
                return Ok(false);
            };
            let mut v = s.0.borrow_mut();
            if v.trade.active {
                return Ok(false);
            }
            Ok(v.queue(TradeCommand::Invite { obj_id: target.obj_id }, None))
        })?,
    )?;
    for (name, accept) in [("StartTrade", true), ("CannotStartTrade", false)] {
        let s = state.clone();
        trade.set(
            name,
            lua.create_function(move |_, (_, id, reason): (Table, String, Option<i32>)| {
                let Ok(obj_id) = id.parse::<u32>() else { return Ok(false) };
                let mut v = s.0.borrow_mut();
                if v.trade.active || v.trade.incoming_invite != Some(obj_id) {
                    return Ok(false);
                }
                Ok(v.queue(
                    if accept {
                        TradeCommand::Accept { obj_id }
                    } else {
                        TradeCommand::Decline {
                            obj_id,
                            reason: reason.unwrap_or(0),
                        }
                    },
                    None,
                ))
            })?,
        )?;
    }
    let s = state.clone();
    trade.set(
        "CancelTrade",
        lua.create_function(move |_, (_, reason): (Table, Option<i32>)| {
            let mut v = s.0.borrow_mut();
            if !v.trade.active {
                return Ok(false);
            }
            Ok(v.queue(TradeCommand::Cancel { reason: reason.unwrap_or(0) }, None))
        })?,
    )?;
    for (name, other) in [("IsTradeLocked", false), ("IsOtherTradeLocked", true)] {
        let s = state.clone();
        trade.set(
            name,
            lua.create_function(move |_, _: Table| {
                let v = s.0.borrow();
                Ok(if other { v.trade.other_locked } else { v.trade.my_locked })
            })?,
        )?;
    }
    trade.set("GetCurrencyForUserTrade", lua.create_function(|_, _: Table| Ok(0))?)?;
    let s = state.clone();
    trade.set(
        "LockTrade",
        lua.create_function(move |_, _: Table| {
            let mut v = s.0.borrow_mut();
            if !v.trade.active {
                return Ok(false);
            }
            let locked = !v.trade.my_locked;
            Ok(v.queue(TradeCommand::Lock { locked }, None))
        })?,
    )?;
    let s = state.clone();
    let b = bridge.clone();
    trade.set(
        "OkTrade",
        lua.create_function(move |_, _: Table| {
            let mut v = s.0.borrow_mut();
            if !v.trade.can_confirm() {
                return Ok(false);
            }
            let d = b.borrow();
            if !offers_current(&v.trade, &v.inventory) || !d.game.get().is_some_and(|g| v.trade.other.iter().all(|i| g.items.contains_key(&i.template_id))) {
                return Ok(v.refuse("An offered item changed or is unavailable. Cancel and reopen this trade."));
            }
            Ok(v.queue(TradeCommand::Confirm, None))
        })?,
    )?;
    let s = state.clone();
    trade.set(
        "PutupTradeMoneyByStr",
        lua.create_function(move |_, (_, g, silver, c): (Table, String, String, String)| {
            let mut v = s.0.borrow_mut();
            if v.suppress_money || !v.trade.active || v.trade.my_locked || v.trade.my_ok || v.trade.other_ok {
                return Ok(false);
            }
            let Some(amount) = money(&g, &silver, &c) else {
                return Ok(v.refuse("Invalid trade money amount."));
            };
            if amount == v.trade.my_money
                && !v
                    .pending
                    .iter()
                    .map(|(c, _)| c)
                    .chain(v.trade.pending.iter())
                    .any(|c| matches!(c, TradeCommand::OfferMoney { amount: pending } if *pending != amount))
            {
                v.commands.retain(|q| !matches!(q.command, TradeCommand::OfferMoney { .. }));
                return Ok(true);
            }
            if i64::from(amount) > v.money {
                return Ok(v.refuse("Not enough money for this trade offer."));
            }
            Ok(v.queue(TradeCommand::OfferMoney { amount }, None))
        })?,
    )?;
    let s = state.clone();
    trade.set(
        "PutupTradeItem",
        lua.create_function(move |lua, (_, index, amount): (Table, Option<usize>, Option<i32>)| {
            let Some(index) = index else { return Ok(false) };
            let Some(slot) = index.checked_sub(1).and_then(|s| u8::try_from(s).ok()) else {
                return Ok(false);
            };
            let bag: Table = lua.globals().get("X2Bag")?;
            if bag.call_method::<bool>("IsLocked", index)? {
                return Ok(false);
            }
            let held = aa_ui::lua::ui(lua).drag.held.as_ref().map(|h| h.table.clone());
            let mut v = s.0.borrow_mut();
            let Some(item) = v.inventory.item_at(2, usize::from(slot)).cloned() else {
                return Ok(false);
            };
            if let Some(h) = held {
                let expected = h.get::<Option<String>>("uid")?.and_then(|s| s.parse::<u64>().ok());
                if h.get::<Option<String>>("kind")?.as_deref() != Some("bag") || expected != Some(item.id) || h.get::<Option<bool>>("partial")?.unwrap_or(false)
                {
                    return Ok(v.refuse("Attach the whole bag stack to trade. Split it first to trade fewer items."));
                }
            }
            let amount = amount.unwrap_or(0);
            if amount != 0 && amount != item.count {
                return Ok(v.refuse("Split the stack before trading fewer items."));
            }
            let command = match v.trade.plan_offer(&v.inventory, usize::from(slot)) {
                Ok(c) => c,
                Err(e) => return Ok(v.refuse(e)),
            };
            let offered: std::collections::BTreeSet<_> = v
                .trade
                .mine
                .iter()
                .filter(|o| o.container == 2)
                .map(|o| o.slot)
                .chain(
                    v.commands
                        .iter()
                        .map(|q| &q.command)
                        .chain(v.pending.iter().map(|(c, _)| c))
                        .chain(v.trade.pending.iter())
                        .filter_map(|c| if let TradeCommand::OfferItem { slot } = c { Some(*slot) } else { None }),
                )
                .collect();
            if offered.len() >= aa_net::trade::MAX_OFFERS {
                return Ok(v.refuse("Trade cart is full."));
            }
            Ok(v.queue(command, Some((slot, item.id, item.count))))
        })?,
    )?;
    trade.set(
        "TakeDownTradeItemByInventoryIdx",
        lua.create_function(move |_, (_, slot): (Table, u8)| {
            let mut v = state.0.borrow_mut();
            if !v.trade.active || v.trade.my_locked || v.trade.my_ok || v.trade.other_ok || !v.trade.mine.iter().any(|o| o.container == 2 && o.slot == slot) {
                return Ok(false);
            }
            Ok(v.queue(TradeCommand::RemoveItem { slot }, None))
        })?,
    )?;
    lua.load(include_str!("trade.lua")).set_name("@trade.lua").exec()
}

fn completed(command: &TradeCommand, event: &TradeEvent) -> bool {
    match (command, event) {
        (_, TradeEvent::Canceled { .. } | TradeEvent::Made(_)) => true,
        (TradeCommand::Invite { .. } | TradeCommand::Accept { .. }, TradeEvent::Started { .. }) => true,
        (TradeCommand::Invite { obj_id: a }, TradeEvent::Declined { obj_id: b, .. }) => a == b,
        (TradeCommand::OfferItem { slot: a }, TradeEvent::ItemOffered { container: 2, slot: b, .. }) => a == b,
        (TradeCommand::RemoveItem { slot: a }, TradeEvent::ItemRemoved { container: 2, slot: b }) => a == b,
        (TradeCommand::OfferMoney { amount: a }, TradeEvent::MoneyOffered(b)) => a == b,
        (TradeCommand::Lock { locked }, TradeEvent::Locked { mine, .. }) => locked == mine,
        (TradeCommand::Confirm, TradeEvent::Confirmed { mine: true, .. }) => true,
        (a, TradeEvent::RequestRejected { command: b, .. }) => a == b,
        _ => false,
    }
}

fn notice(rt: &UiRuntime, message: &str) {
    warn!("hud: trade: {message}");
    rt.fire_event("SYSMSG", message);
}

fn peer_name(trade: &Trade, units: &Units) -> String {
    units.get(trade.partner_obj_id).map(|u| u.name.clone()).unwrap_or_default()
}

fn other_offer(rt: &UiRuntime, bridge: &Shared, index: usize, item: &Item) {
    match current_item(&rt.lua, bridge, item) {
        Ok(Some(t)) => rt.fire_event("TRADE_OTHER_ITEM_PUTUP", (index, item.template_id, item.count, t)),
        Ok(None) => notice(rt, "An offered item's definition is unavailable. Cancel this trade."),
        Err(e) => warn!("hud: trade item tooltip: {e}"),
    }
}

fn locks(rt: &UiRuntime, old: &Trade, new: &Trade) {
    if (!new.my_locked && old.my_locked) || (!new.other_locked && old.other_locked) {
        rt.fire_event("TRADE_UNLOCKED", ());
    }
    if new.my_locked && !old.my_locked {
        rt.fire_event("TRADE_LOCKED", ());
    }
    if new.other_locked && !old.other_locked {
        rt.fire_event("TRADE_OTHER_LOCKED", ());
    }
    if new.my_ok && !old.my_ok {
        rt.fire_event("TRADE_OK", ());
    }
    if new.other_ok && !old.other_ok {
        rt.fire_event("TRADE_OTHER_OK", ());
    }
}

fn restore_locks(rt: &UiRuntime, trade: &Trade) {
    // The original button paints its pressed state immediately. A local send failure
    // must restore the actual server flags rather than leave that presentation frozen.
    rt.fire_event("TRADE_UNLOCKED", ());
    locks(rt, &Trade::default(), trade);
}

fn bootstrap(rt: &UiRuntime, bridge: &Shared, state: &State, trade: &Trade, units: &Units) {
    if trade.active {
        state.0.borrow_mut().suppress_money = true;
        rt.fire_event("TRADE_STARTED", peer_name(trade, units));
        for offer in &trade.mine {
            if offer.container == 2 {
                rt.fire_event("TRADE_ITEM_PUTUP", (usize::from(offer.slot) + 1, offer.amount));
            }
        }
        for (index, item) in trade.other.iter().enumerate() {
            other_offer(rt, bridge, index, item);
        }
        rt.fire_event("TRADE_OTHER_MONEY_PUTUP", trade.other_money.to_string());
        let result = (|| -> mlua::Result<()> {
            let table: Table = rt.lua.load("return trade.window.myWindow.moneyEditsWindow").eval()?;
            table.call_method::<()>("SetAmountStr", trade.my_money.to_string())
        })();
        if let Err(e) = result {
            warn!("hud: restore trade money: {e}");
        }
        state.0.borrow_mut().suppress_money = false;
        locks(rt, &Trade::default(), trade);
    } else if let Some(id) = trade.incoming_invite {
        rt.fire_event("TRADE_CAN_START", id.to_string());
    }
}

fn valid(command: &TradeCommand, trade: &Trade, inventory: &Inventory, balance: i64) -> bool {
    match command {
        TradeCommand::Invite { .. } => !trade.active,
        TradeCommand::Accept { obj_id } | TradeCommand::Decline { obj_id, .. } => !trade.active && trade.incoming_invite == Some(*obj_id),
        TradeCommand::Cancel { .. } => trade.active,
        TradeCommand::OfferItem { slot } => trade.plan_offer(inventory, usize::from(*slot)).is_ok(),
        TradeCommand::RemoveItem { slot } => {
            trade.active && !trade.my_locked && !trade.my_ok && !trade.other_ok && trade.mine.iter().any(|o| o.container == 2 && o.slot == *slot)
        }
        TradeCommand::OfferMoney { amount } => {
            trade.active && !trade.my_locked && !trade.my_ok && !trade.other_ok && *amount >= 0 && i64::from(*amount) <= balance
        }
        TradeCommand::Lock { .. } => trade.active,
        TradeCommand::Confirm => trade.can_confirm() && offers_current(trade, inventory) && i64::from(trade.my_money) <= balance,
    }
}

#[allow(clippy::too_many_arguments)]
fn sync(
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<HudBridge>,
    trade: Res<Trade>,
    inventory: Res<Inventory>,
    stats: Res<PlayerStats>,
    units: Res<Units>,
    mut events: MessageReader<NetEvent>,
    mut net: MessageWriter<NetCommand>,
) {
    let Some(rt) = rt else {
        events.clear();
        return;
    };
    let Some(state) = rt.lua.app_data_ref::<State>().map(|s| s.clone()) else {
        events.clear();
        return;
    };
    let signals: Vec<_> = events
        .read()
        .filter_map(|e| if let Event::Trade(t) = &e.0 { Some(t.clone()) } else { None })
        .collect();
    let (fresh, mut before) = {
        let mut v = state.0.borrow_mut();
        let fresh = !v.initialized;
        v.initialized = true;
        v.inventory = inventory.clone();
        v.money = stats.money;
        if fresh {
            v.trade = trade.clone();
        }
        (fresh, v.trade.clone())
    };
    if fresh && (trade.active || signals.is_empty()) {
        bootstrap(&rt, &bridge.0, &state, &trade, &units);
    }
    for event in signals {
        state.0.borrow_mut().pending.retain(|(c, _)| !completed(c, &event));
        if fresh && trade.active {
            if let TradeEvent::RequestRejected { message, .. } = &event {
                notice(&rt, message);
            }
            continue;
        }
        let previous = before.clone();
        let removed = if let TradeEvent::OtherItemRemoved(item) = &event {
            before.other.iter().position(|i| i.id == item.id)
        } else {
            None
        };
        before.apply(&Event::Trade(event.clone()));
        if let TradeEvent::ItemOffered { container, slot, .. } = &event {
            if let Some(o) = before.mine.iter_mut().rev().find(|o| o.container == *container && o.slot == *slot) {
                o.item = trade
                    .mine
                    .iter()
                    .find(|o| o.container == *container && o.slot == *slot)
                    .and_then(|o| o.item.clone())
                    .or_else(|| inventory.item_at(*container, usize::from(*slot)).cloned());
            }
        }
        state.0.borrow_mut().trade = before.clone();
        match event {
            TradeEvent::Invited { obj_id } => rt.fire_event("TRADE_CAN_START", obj_id.to_string()),
            TradeEvent::Started { .. } => rt.fire_event("TRADE_STARTED", peer_name(&before, &units)),
            TradeEvent::Canceled { .. } => rt.fire_event("TRADE_CANCELED", ()),
            TradeEvent::Made(_) => rt.fire_event("TRADE_MADE", ()),
            TradeEvent::ItemOffered { container: 2, slot, amount } => rt.fire_event("TRADE_ITEM_PUTUP", (usize::from(slot) + 1, amount)),
            TradeEvent::ItemRemoved { container: 2, slot } => rt.fire_event("TRADE_ITEM_TOOKDOWN", usize::from(slot) + 1),
            TradeEvent::OtherItemOffered(item) => other_offer(&rt, &bridge.0, previous.other.len(), &item),
            TradeEvent::OtherItemRemoved(_) => {
                if let Some(index) = removed {
                    rt.fire_event("TRADE_OTHER_ITEM_TOOKDOWN", index);
                }
            }
            TradeEvent::MoneyOffered(amount) => rt.fire_event("TRADE_MONEY_PUTUP", amount.to_string()),
            TradeEvent::OtherMoneyOffered(amount) => rt.fire_event("TRADE_OTHER_MONEY_PUTUP", amount.to_string()),
            TradeEvent::Locked { .. } | TradeEvent::Confirmed { .. } => locks(&rt, &previous, &before),
            TradeEvent::Declined { reason, .. } => notice(&rt, &format!("Trade invitation declined (code {reason}).")),
            TradeEvent::RequestRejected { command, message } => {
                if matches!(command, TradeCommand::Lock { .. }) && before.active {
                    restore_locks(&rt, &before);
                }
                notice(&rt, &message);
            }
            _ => {}
        }
    }
    let (commands, notices, timed_out) = {
        let mut v = state.0.borrow_mut();
        v.trade = trade.clone();
        let mut timed_out = false;
        v.pending.retain(|(_, at)| {
            let keep = at.elapsed() < Duration::from_secs(60);
            timed_out |= !keep;
            keep
        });
        (std::mem::take(&mut v.commands), std::mem::take(&mut v.notices), timed_out)
    };
    for text in notices {
        notice(&rt, &text);
    }
    if timed_out {
        if trade.active {
            restore_locks(&rt, &trade);
        }
        notice(&rt, "Trade request timed out. Cancel and reopen the trade to retry.");
    }
    for queued in commands {
        if queued
            .expected
            .is_some_and(|(slot, id, count)| inventory.item_at(2, usize::from(slot)).is_none_or(|item| item.id != id || item.count != count))
        {
            notice(&rt, "The offered bag item changed. Offer it again.");
            continue;
        }
        if !valid(&queued.command, &trade, &inventory, stats.money) {
            continue;
        }
        if !matches!(queued.command, TradeCommand::Decline { .. }) {
            state.0.borrow_mut().pending.push((queued.command.clone(), Instant::now()));
        }
        net.write(NetCommand(Command::Trade(queued.command)));
    }
}

pub fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        sync.after(super::sync_hud)
            .before(aa_ui::UiFrame)
            .run_if(in_state(crate::Screen::InGame))
            .run_if(super::hud_up),
    );
}
