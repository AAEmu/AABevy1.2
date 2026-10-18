//! Original bag/bank expansion confirmation with fresh capacity and cost preflight.
use aa_net::bank::{BankCommand, BankEvent, ExpansionFailure, ExpansionQuote, ExpansionRule};
use aa_net::state::{Inventory, Player, PlayerStats};
use aa_net::{Command, Event, NetCommand, NetEvent};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Lua, Table};
use rusqlite::Connection;

use super::api::{HudBridge, HudData, Shared};

#[allow(dead_code)]
pub(super) mod fixture;

pub(super) fn extend_slots(rt: &UiRuntime, capacity: u32) {
    let result = rt.lua.load(include_str!("expansion_slots.lua")).set_name("@expansion_slots.lua")
        .eval::<mlua::Function>().and_then(|function| function.call::<()>(capacity));
    if let Err(error) = result {
        warn!("hud: inventory widget pool: {error}");
    }
}

pub(super) fn load(conn: &Connection) -> rusqlite::Result<Vec<ExpansionRule>> {
    let mut q = conn.prepare("SELECT is_bank,step,price,item_id,item_count,currency_id FROM bag_expands ORDER BY id")?;
    q.query_map([], |r| {
        Ok(ExpansionRule {
            is_bank: super::data::any_flag(r, 0)?,
            step: super::data::any_int(r, 1)? as i32,
            price: super::data::any_int(r, 2)? as i32,
            item_id: super::data::any_int(r, 3)?.max(0) as u32,
            item_count: super::data::any_int(r, 4)? as i32,
            currency_id: super::data::any_int(r, 5)? as i32,
        })
    })?
    .collect()
}

#[derive(Default)]
pub(super) struct View {
    owner: u32,
    received: Option<Inventory>,
    next: u32,
    quote: Option<(u32, ExpansionQuote)>,
    requests: Vec<(u32, ExpansionQuote, bool)>,
    auto_request: Option<(u32, bool)>,
    auto_waiting: Option<bool>,
    auto_pending: Option<bool>,
    auto_event: Option<u8>,
    events_active: bool,
}

impl View {
    pub(super) fn clear(&mut self) {
        let next = self.next;
        *self = Self { next, ..default() };
    }
}

pub(super) fn refresh(world: &World, data: &mut HudData) {
    if data.expansion.owner != data.character_id {
        data.expansion.clear();
        data.expansion.owner = data.character_id;
    }
    if let Some(inventory) = world.get_resource_ref::<Inventory>() {
        if data.expansion.received.is_none() || inventory.is_changed() {
            data.expansion.received = Some((*inventory).clone());
            if !data.expansion.events_active {
                data.expansion.auto_pending = inventory.pending_auto_use_aa_point;
            }
        }
    }
}

fn make_quote(data: &mut HudData, container: u8) -> Option<(u32, ExpansionQuote)> {
    if data.expansion.owner == 0 {
        return None;
    }
    let game = data.game.get()?;
    let inventory = data.expansion.received.as_ref()?;
    let quote = aa_net::bank::expansion_quote(inventory, container, data.snapshot.money, &game.expansions).ok()?;
    data.expansion.next = data.expansion.next.wrapping_add(1).max(1);
    let token = data.expansion.next;
    data.expansion.quote = Some((token, quote));
    Some((token, quote))
}

fn confirm(data: &mut HudData, token: u32, auto: bool) {
    if data.expansion.quote.is_some_and(|q| q.0 == token) {
        let (_, quote) = data.expansion.quote.take().unwrap();
        data.expansion.requests.push((data.expansion.owner, quote, auto));
    }
}

fn request_auto(data: &mut HudData, value: bool) {
    let view = &mut data.expansion;
    if view.owner != 0 && view.auto_request.is_none() && view.auto_waiting.is_none()
        && view.received.as_ref().is_some_and(|inventory| {
            let pending = if view.events_active { view.auto_pending } else { inventory.pending_auto_use_aa_point };
            pending.is_none() && view.auto_event.unwrap_or(inventory.auto_use_aa_point_raw) != u8::from(value)
        })
    {
        view.auto_request = Some((view.owner, value));
    }
}

pub(super) fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    let hud: Table = lua.globals().get("HUD")?;
    let state = bridge.clone();
    hud.set("auto_aa_point", lua.create_function(move |_, ()| {
        let data = state.borrow();
        Ok(data.expansion.auto_event.map_or_else(|| data.expansion.received.as_ref().is_some_and(Inventory::auto_use_aa_point), |raw| raw == 1))
    })?)?;
    let state = bridge.clone();
    hud.set("set_auto_aa_point", lua.create_function(move |_, value: bool| {
        request_auto(&mut state.borrow_mut(), value);
        Ok(())
    })?)?;
    let state = bridge.clone();
    hud.set(
        "expansion_max",
        lua.create_function(move |_, container: u8| {
            let data = state.borrow();
            let capacity = if container == 3 { data.panels.bank_slots } else { data.panels.bag_slots };
            Ok(data
                .game
                .get()
                .and_then(|g| {
                    g.expansions
                        .iter()
                        .filter(|r| r.is_bank == (container == 3) && r.step >= 0)
                        .map(|r| 50 + (r.step as u32 + 1) * 10)
                        .max()
                })
                .unwrap_or(capacity))
        })?,
    )?;
    let state = bridge.clone();
    hud.set(
        "expansion_quote",
        lua.create_function(move |lua, container: u8| {
            let Some((token, quote)) = make_quote(&mut state.borrow_mut(), container) else {
                return Ok(None);
            };
            let t = lua.create_table()?;
            t.set("token", token)?;
            t.set("kind", if container == 3 { "bank" } else { "bag" })?;
            t.set(
                "dialog",
                if quote.rule.item_id != 0 && quote.rule.item_count > 0 {
                    if quote.rule.price != 0 { "item_money" } else { "item" }
                } else {
                    "money"
                },
            )?;
            t.set("currency", quote.rule.currency_id)?;
            t.set("cost", quote.rule.price)?;
            t.set("itemType", quote.rule.item_id)?;
            t.set("itemCount", quote.rule.item_count)?;
            Ok(Some(t))
        })?,
    )?;
    let state = bridge.clone();
    hud.set(
        "expansion_confirm",
        lua.create_function(move |_, (token, auto): (u32, bool)| {
            confirm(&mut state.borrow_mut(), token, auto);
            Ok(())
        })?,
    )?;
    let state = bridge.clone();
    hud.set(
        "expansion_cancel",
        lua.create_function(move |_, token: u32| {
            let mut data = state.borrow_mut();
            if data.expansion.quote.is_some_and(|q| q.0 == token) {
                data.expansion.quote = None;
            }
            Ok(())
        })?,
    )?;
    lua.load(include_str!("expansion.lua")).set_name("@expansion.lua").exec()
}

fn command(owner: u32, active: u32, quote: ExpansionQuote, auto: bool, inventory: &Inventory, money: i64) -> Result<BankCommand, String> {
    if owner == 0 || owner != active {
        return Err("The character changed. Reopen inventory expansion.".into());
    }
    quote.recheck(inventory, money).map_err(|failure| {
        match failure {
            ExpansionFailure::NotEnoughMoney { .. } => "Not enough money to expand this inventory.",
            ExpansionFailure::NotEnoughCertificates { .. } => "The required expansion item is missing from your bag.",
            ExpansionFailure::CapacityChanged { .. } => "Inventory capacity changed. Reopen expansion to review the current cost.",
            _ => "This inventory cannot be expanded.",
        }
        .to_string()
    })?;
    // The native navigation-context producer is unresolved. Zero is the explicit unset tail,
    // not a guessed NPC/target ID. aa-net writes the complete native eight-byte request.
    Ok(BankCommand::Expand {
        container: quote.container,
        auto_use_aa_point: auto,
    })
}

fn auto_event(rt: &UiRuntime, bridge: &Shared, event: &Event) {
    match event {
        Event::AutoUseAaPointRequested(value) => {
            let mut data = bridge.borrow_mut();
            data.expansion.auto_pending = Some(*value);
            if data.expansion.auto_waiting == Some(*value) { data.expansion.auto_waiting = None; }
        }
        Event::AutoUseAaPointRequestRejected { value, message } => {
            {
                let mut data = bridge.borrow_mut();
                if data.expansion.auto_waiting == Some(*value) { data.expansion.auto_waiting = None; }
            }
            rt.fire_event("SYSMSG", message.as_str());
        }
        Event::CraftItemUnlocked(unlock) if unlock.unlocks_auto_use_aa_point() => {
            bridge.borrow_mut().expansion.auto_pending = None;
        }
        Event::ItemsChanged(tasks) => {
            let mut changed = false;
            for task in &tasks.tasks {
                if let aa_net::items::ItemTask::AutoUseAaPoint(raw) = task {
                    changed = true;
                    // Native394e758c queries the just-written byte and calls Lua inline, before
                    // the next action. Preserve that getter even though sync_hud saw the final
                    // resource snapshot. The pending key remains locked through all callbacks.
                    bridge.borrow_mut().expansion.auto_event = Some(*raw);
                    rt.fire_event("CHANGED_AUTO_USE_AAPOINT", *raw == 1);
                }
            }
            let mut data = bridge.borrow_mut();
            data.expansion.auto_event = None;
            if changed { data.expansion.auto_pending = None; }
        }
        _ => {}
    }
}

fn update(
    mut events: MessageReader<NetEvent>,
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<HudBridge>,
    inventory: Res<Inventory>,
    stats: Res<PlayerStats>,
    player: Res<Player>,
    mut net: MessageWriter<NetCommand>,
) {
    let Some(rt) = rt else {
        events.clear();
        return;
    };
    if !bridge.0.borrow().expansion.events_active {
        // Initial queries use the hydrated snapshot; loading history must not replay changes.
        bridge.0.borrow_mut().expansion.events_active = true;
        events.clear();
    }
    for NetEvent(event) in events.read() {
        auto_event(&rt, &bridge.0, event);
        if let Event::InventoryExpanded { container, .. } = event {
            match container {
                2 => rt.fire_event("BAG_EXPANDED", ()),
                3 => rt.fire_event("BANK_EXPANDED", ()),
                _ => {}
            }
        }
        if let Event::Bank(BankEvent::RequestRejected {
            command: BankCommand::Expand { .. } | BankCommand::ExpandWithContext { .. },
            message,
        }) = event
        {
            rt.fire_event("SYSMSG", message.as_str());
        }
    }
    let active = player.spawn.as_ref().map_or(0, |s| s.character().id);
    let auto_request = bridge.0.borrow_mut().expansion.auto_request.take();
    if let Some((owner, value)) = auto_request {
        if owner != 0 && owner == active && inventory.can_set_auto_use_aa_point(value) {
            bridge.0.borrow_mut().expansion.auto_waiting = Some(value);
            net.write(NetCommand(Command::SetAutoUseAaPoint(value)));
        }
    }
    let requests = std::mem::take(&mut bridge.0.borrow_mut().expansion.requests);
    for (owner, quote, auto) in requests {
        match command(owner, active, quote, auto, &inventory, stats.money) {
            Ok(command) => {
                net.write(NetCommand(Command::Bank(command)));
            }
            Err(message) => rt.fire_event("SYSMSG", message.as_str()),
        }
    }
}

pub(super) fn plugin(app: &mut App) {
    app.add_systems(Update, update.after(super::sync_hud).before(aa_ui::UiFrame).run_if(super::hud_up));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_aa_getter_hydrates_and_replays_each_received_action_before_batch_unlock() -> Result<(), Box<dyn std::error::Error>> {
        let mut rt = UiRuntime::new("en_us", 1280.0, 720.0)?;
        let bridge = std::rc::Rc::new(std::cell::RefCell::new(HudData::default()));
        super::super::api::install(&mut rt, &bridge)?;
        let mut world = World::new();
        let mut inventory = Inventory::default();
        inventory.auto_use_aa_point_raw = 1;
        world.insert_resource(inventory);
        bridge.borrow_mut().character_id = 47;
        refresh(&world, &mut bridge.borrow_mut());
        rt.lua.load("assert(X2Bag:GetAutoUseAAPoint()); seen={}; UIParent:SetEventHandler('CHANGED_AUTO_USE_AAPOINT',function(on) seen[#seen+1]={on,X2Bag:GetAutoUseAAPoint()}; X2Bag:SetAutoUseAAPoint(not on) end)").exec()?;
        bridge.borrow_mut().expansion.events_active = true;
        bridge.borrow_mut().expansion.auto_waiting = Some(false);
        auto_event(&rt, &bridge, &Event::AutoUseAaPointRequested(false));
        assert!(bridge.borrow().expansion.auto_waiting.is_none());
        assert_eq!(bridge.borrow().expansion.auto_pending, Some(false));
        // sync_hud has already received the final byte, but each callback reads its own action.
        world.resource_mut::<Inventory>().auto_use_aa_point_raw = 2;
        refresh(&world, &mut bridge.borrow_mut());
        auto_event(&rt, &bridge, &Event::ItemsChanged(aa_net::items::ItemTasks {
            tasks: [0, 1, 1, 2].into_iter().map(aa_net::items::ItemTask::AutoUseAaPoint).collect(),
            ..default()
        }));
        rt.lua.load("assert(#seen==4 and seen[1][1]==false and seen[2][1]==true and seen[3][1]==true and seen[4][1]==false); for _,v in ipairs(seen) do assert(v[1]==v[2]) end; assert(not X2Bag:GetAutoUseAAPoint())").exec()?;
        assert!(bridge.borrow().expansion.auto_request.is_none(), "all callbacks run before the native pending key is released");
        assert!(bridge.borrow().expansion.auto_pending.is_none());
        // Raw2 reads false, but native compares the raw byte against requested0 rather than
        // comparing two Lua booleans. It therefore permits this explicit false request.
        rt.lua.load("X2Bag:SetAutoUseAAPoint(false)").exec()?;
        assert_eq!(bridge.borrow_mut().expansion.auto_request.take(), Some((47, false)));
        auto_event(&rt, &bridge, &Event::AutoUseAaPointRequested(false));
        auto_event(&rt, &bridge, &Event::CraftItemUnlocked(aa_net::craft::CraftItemUnlock { kind: 0xf0, slot: 1 }));
        assert_eq!(bridge.borrow().expansion.auto_pending, Some(false));
        auto_event(&rt, &bridge, &Event::CraftItemUnlocked(aa_net::craft::CraftItemUnlock { kind: 0xf0, slot: 0 }));
        assert!(bridge.borrow().expansion.auto_pending.is_none());
        bridge.borrow_mut().expansion.auto_waiting = Some(true);
        auto_event(&rt, &bridge, &Event::AutoUseAaPointRequestRejected { value: true, message: "No connection".into() });
        assert!(bridge.borrow().expansion.auto_waiting.is_none());
        rt.lua.load("assert(#seen==4 and not X2Bag:GetAutoUseAAPoint())").exec()?;
        assert!(rt.errors.is_empty(), "{:?}", rt.errors);
        Ok(())
    }

    #[test]
    fn auto_aa_requests_preserve_received_value_and_deduplicate_queued_and_pending_sends() {
        let mut data = HudData::default();
        data.expansion.owner = 47;
        data.expansion.received = Some(Inventory::default());
        request_auto(&mut data, false);
        assert!(data.expansion.auto_request.is_none());
        request_auto(&mut data, true);
        request_auto(&mut data, true);
        assert_eq!(data.expansion.auto_request, Some((47, true)));
        assert!(!data.expansion.received.as_ref().unwrap().auto_use_aa_point());
        data.expansion.auto_request = None;
        data.expansion.auto_waiting = Some(true);
        request_auto(&mut data, true);
        assert!(data.expansion.auto_request.is_none());
        data.expansion.clear();
        assert!(data.expansion.auto_waiting.is_none() && data.expansion.auto_request.is_none());
        request_auto(&mut data, true);
        assert!(data.expansion.auto_request.is_none(), "no requests without the active character's inventory");
    }

    #[test]
    fn expansion_confirmation_rechecks_owner_capacity_money_and_bag_certificates() {
        let mut inventory = Inventory::default();
        inventory.bag_slots = 50;
        inventory.bank_slots = 50;
        inventory.bag = vec![Some(aa_net::model::Item {
            id: 9,
            template_id: 700,
            count: 2,
            ..default()
        })];
        let rules = [ExpansionRule {
            is_bank: false,
            step: 0,
            price: 25,
            item_id: 700,
            item_count: 1,
            currency_id: 0,
        }];
        let quote = aa_net::bank::expansion_quote(&inventory, 2, 100, &rules).unwrap();
        assert_eq!(
            command(47, 47, quote, true, &inventory, 100).unwrap(),
            BankCommand::Expand {
                container: 2,
                auto_use_aa_point: true
            }
        );
        assert!(command(47, 48, quote, true, &inventory, 100).is_err());
        assert!(command(47, 47, quote, true, &inventory, 24).is_err());
        inventory.bag.clear();
        inventory.bank = vec![Some(aa_net::model::Item {
            template_id: 700,
            count: 10,
            ..default()
        })];
        assert!(
            command(47, 47, quote, true, &inventory, 100).is_err(),
            "bank certificates do not pay for expansion"
        );
        inventory.bag_slots = 60;
        assert!(command(47, 47, quote, true, &inventory, 100).unwrap_err().contains("capacity changed"));
        assert_eq!(quote.current_slots, 50, "confirmation never changes received capacity or costs");
    }

    #[test]
    fn quotes_are_bound_to_the_latest_dialog_and_confirm_only_once() {
        let mut data = HudData::default();
        data.expansion.owner = 47;
        let mut game = super::super::data::GameData::default();
        game.expansions.push(ExpansionRule {
            is_bank: false,
            step: 0,
            price: 0,
            item_id: 0,
            item_count: 0,
            currency_id: 0,
        });
        assert!(data.game.0.set(game).is_ok());
        let mut inventory = Inventory::default();
        inventory.bag_slots = 50;
        data.expansion.received = Some(inventory);
        let (old, _) = make_quote(&mut data, 2).unwrap();
        let (current, _) = make_quote(&mut data, 2).unwrap();
        confirm(&mut data, old, false);
        assert!(data.expansion.requests.is_empty());
        confirm(&mut data, current, true);
        confirm(&mut data, current, true);
        assert_eq!(data.expansion.requests.len(), 1);
        assert!(data.expansion.requests[0].2);
        assert_eq!(data.expansion.received.as_ref().unwrap().bag_slots, 50);
        data.expansion.requests.clear();
        data.expansion.clear();
        data.expansion.owner = 48;
        let mut inventory = Inventory::default();
        inventory.bag_slots = 50;
        data.expansion.received = Some(inventory);
        let (next, _) = make_quote(&mut data, 2).unwrap();
        assert_ne!(next, current, "a closed previous character's callback cannot confirm the new dialog");
        confirm(&mut data, current, true);
        assert!(data.expansion.requests.is_empty());
    }

    #[test]
    #[ignore = "loads the original world-stage assets; run in an isolated fixture process"]
    fn original_expansion_buttons_confirm_cancel_and_wait_for_received_capacity() -> Result<(), Box<dyn std::error::Error>> {
        let (rt, bridge, path, inventory) = fixture::runtime(1920.0, 1080.0)?;
        fixture::click(&rt, "newbag.expandBtn")?;
        rt.lua
            .load("assert(EXPANSION_DIALOG:IsVisible() and EXPANSION_INFO.kind=='bag' and EXPANSION_INFO.itemType==8000025)")
            .exec()?;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_non_send(rt)
            .insert_non_send(HudBridge(bridge.clone()))
            .insert_resource(inventory)
            .insert_resource(PlayerStats {
                obj_id: 100,
                character_id: 47,
                money: 1000,
                ..default()
            })
            .insert_resource(Player { spawn: Some(fixture::spawn()) })
            .add_message::<NetEvent>()
            .add_message::<NetCommand>()
            .add_systems(Update, update);
        let drain = |app: &mut App| app.world_mut().resource_mut::<Messages<NetCommand>>().drain().map(|v| v.0).collect::<Vec<_>>();
        fixture::click(app.world().non_send::<UiRuntime>(), "EXPANSION_DIALOG.btnCancel")?;
        app.update();
        assert!(drain(&mut app).is_empty());
        app.world().non_send::<UiRuntime>().lua.load("X2Bag:SetAutoUseAAPoint(true); X2Bag:SetAutoUseAAPoint(true); assert(not X2Bag:GetAutoUseAAPoint())").exec()?;
        app.update();
        assert!(matches!(drain(&mut app).as_slice(), [Command::SetAutoUseAaPoint(true)]));
        app.world_mut().resource_mut::<Inventory>().apply(&Event::AutoUseAaPointRequested(true));
        app.world_mut().write_message(NetEvent(Event::AutoUseAaPointRequested(true)));
        let ack = Event::ItemsChanged(aa_net::items::ItemTasks {
            tasks: vec![aa_net::items::ItemTask::AutoUseAaPoint(1)],
            ..default()
        });
        app.world_mut().resource_mut::<Inventory>().apply(&ack);
        bridge.borrow_mut().expansion.received = Some(app.world().resource::<Inventory>().clone());
        app.world_mut().write_message(NetEvent(ack));
        app.update();
        assert!(drain(&mut app).is_empty());
        app.world().non_send::<UiRuntime>().lua.load("assert(X2Bag:GetAutoUseAAPoint())").exec()?;
        fixture::click(app.world().non_send::<UiRuntime>(), "newbag.expandBtn")?;
        fixture::click(app.world().non_send::<UiRuntime>(), "EXPANSION_DIALOG.btnOk")?;
        app.update();
        assert!(matches!(
            drain(&mut app).as_slice(),
            [Command::Bank(BankCommand::Expand {
                container: 2,
                auto_use_aa_point: true
            })]
        ));
        assert_eq!(app.world().resource::<Inventory>().bag_slots, 50);
        assert_eq!(app.world().resource::<Inventory>().count_of(8000025), 2);
        assert_eq!(app.world().resource::<PlayerStats>().money, 1000);
        app.world().non_send::<UiRuntime>().lua.load("assert(X2Bag:Capacity()==50)").exec()?;
        fixture::click(app.world().non_send::<UiRuntime>(), "newbag.expandBtn")?;
        let ack = Event::InventoryExpanded { container: 2, slots: 60 };
        app.world_mut().resource_mut::<Inventory>().apply(&ack);
        app.world_mut().write_message(NetEvent(ack));
        // This isolated controller probe supplies the same snapshot refresh sync_hud performs.
        {
            let inventory = app.world().resource::<Inventory>();
            let mut data = bridge.borrow_mut();
            data.expansion.received = Some(inventory.clone());
            data.panels.bag_slots = inventory.bag_slots;
        }
        super::super::api::push_changes(app.world().non_send::<UiRuntime>(), &bridge);
        fixture::click(app.world().non_send::<UiRuntime>(), "EXPANSION_DIALOG.btnOk")?;
        app.update();
        assert!(drain(&mut app).is_empty(), "a stale confirmation must not buy a later tier");
        app.world()
            .non_send::<UiRuntime>()
            .lua
            .load("assert(X2Bag:Capacity()==60); X2Bank:Expand(); assert(EXPANSION_INFO.kind=='bank')")
            .exec()?;
        fixture::click(app.world().non_send::<UiRuntime>(), "EXPANSION_DIALOG.btnOk")?;
        app.update();
        assert!(matches!(
            drain(&mut app).as_slice(),
            [Command::Bank(BankCommand::Expand {
                container: 3,
                auto_use_aa_point: true
            })]
        ));
        assert_eq!(app.world().resource::<Inventory>().bank_slots, 50);
        app.world().non_send::<UiRuntime>().lua.load("X2Bank:Expand(); assert(EXPANSION_INFO.kind=='bank'); X2DialogManager:Delete(DLG_TASK_EXPAND_INVENTORY); assert(not EXPANSION_DIALOG:IsVisible())").exec()?;
        app.update();
        assert!(drain(&mut app).is_empty());
        // The database's upper bag tiers exceed the shipped Lua's fixed130-cell allocation.
        let ack = Event::InventoryExpanded { container: 2, slots: 150 };
        app.world_mut().resource_mut::<Inventory>().apply(&ack);
        app.world_mut().write_message(NetEvent(ack));
        bridge.borrow_mut().panels.bag_slots = 150;
        super::super::api::push_changes(app.world().non_send::<UiRuntime>(), &bridge);
        app.update();
        app.world().non_send::<UiRuntime>().lua.load("assert(X2Bag:Capacity()==150 and #newbag.slots.btns==150); newbag:Update(); assert(newbag.slots.btns[150]:IsVisible())").exec()?;
        assert!(app.world().non_send::<UiRuntime>().errors.is_empty());
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
}
