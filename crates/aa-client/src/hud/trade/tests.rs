use super::*;

fn item(id: u64) -> Item {
    Item {
        id,
        template_id: 5,
        count: 4,
        ..default()
    }
}

fn setup() -> (UiRuntime, Shared, State) {
    let mut rt = UiRuntime::new("en_us", 1280.0, 720.0).unwrap();
    let bridge = Rc::new(RefCell::new(super::super::api::HudData::default()));
    let mut game = super::super::data::GameData::default();
    game.items.insert(5, super::super::data::ItemInfo::default());
    assert!(bridge.borrow().game.0.set(game).is_ok());
    super::super::api::install(&mut rt, &bridge).unwrap();
    let state = rt.lua.app_data_ref::<State>().unwrap().clone();
    {
        let mut v = state.0.borrow_mut();
        v.trade.active = true;
        v.inventory.bag.resize(15, None);
        v.inventory.bag[4] = Some(item(9_007_199_254_740_993));
        v.money = 105;
    }
    (rt, bridge, state)
}

#[test]
fn checked_money_and_whole_physical_stack_do_not_mutate_offers() -> mlua::Result<()> {
    assert_eq!(money("", "1", "5"), Some(105));
    for (g, s, c) in [
        ("-1", "0", "0"),
        ("0", "100", "0"),
        ("0", "0", "100"),
        ("214749", "0", "0"),
        ("9223372036854775807", "0", "0"),
    ] {
        assert_eq!(money(g, s, c), None);
    }
    let (rt, _, state) = setup();
    rt.lua.load("X2Cursor:SetHeld({kind='bag',cursorInfo='Item',slot=5,uid='9007199254740993',count=4}); assert(X2Trade:PutupTradeItem(5,0)); assert(not X2Trade:PutupTradeItem(5,0)); assert(not X2Trade:PutupTradeItem(5,2)); assert(not X2Trade:PutupTradeMoneyByStr('','1','6')); assert(X2Trade:PutupTradeMoneyByStr('','1','5'))").exec()?;
    let v = state.0.borrow();
    assert_eq!(v.commands.len(), 2);
    assert_eq!(v.commands[0].command, TradeCommand::OfferItem { slot: 4 });
    assert_eq!(v.commands[0].expected, Some((4, 9_007_199_254_740_993, 4)));
    assert_eq!(v.commands[1].command, TradeCommand::OfferMoney { amount: 105 });
    assert!(v.trade.mine.is_empty());
    assert_eq!(v.inventory.bag[4].as_ref().unwrap().count, 4);
    assert!(!v.trade.my_locked);
    Ok(())
}

#[test]
fn target_request_uses_native_faction_categories_and_actual_character_identity() -> mlua::Result<()> {
    let (rt, bridge, state) = setup();
    {
        let mut d = bridge.borrow_mut();
        d.snapshot.unit_id = "100".into();
        d.panels.target = Some(super::super::panels::TargetSnap {
            obj_id: 200,
            unit_type: "character",
            relation: "neutral",
            ..default()
        });
    }
    state.0.borrow_mut().trade.active = false;
    rt.lua.load("assert(X2:GetTargetFactionRelationship()==2); HUD.target={relation='hostile'}; assert(X2:GetTargetFactionRelationship()==1); HUD.target={relation='friendly'}; assert(X2:GetTargetFactionRelationship()==3); HUD.target=nil; assert(X2:GetTargetFactionRelationship()==2); assert(X2Trade:RequestTrade())").exec()?;
    assert_eq!(state.0.borrow_mut().commands.pop().unwrap().command, TradeCommand::Invite { obj_id: 200 });
    bridge.borrow_mut().panels.target.as_mut().unwrap().relation = "hostile";
    rt.lua.load("assert(not X2Trade:RequestTrade())").exec()?;
    bridge.borrow_mut().panels.target.as_mut().unwrap().relation = "friendly";
    bridge.borrow_mut().panels.target.as_mut().unwrap().obj_id = 100;
    rt.lua.load("assert(not X2Trade:RequestTrade())").exec()?;
    assert!(state.0.borrow().commands.is_empty());
    Ok(())
}

#[test]
fn native_lock_toggle_and_confirm_reject_changed_offers_and_cart_overflow() -> mlua::Result<()> {
    let (rt, _, state) = setup();
    rt.lua
        .load("assert(X2Trade:LockTrade()); assert(not X2Trade:IsTradeLocked()); assert(not X2Trade:OkTrade())")
        .exec()?;
    {
        let mut v = state.0.borrow_mut();
        assert_eq!(v.commands.pop().unwrap().command, TradeCommand::Lock { locked: true });
        v.trade.my_locked = true;
        v.trade.other_locked = true;
        v.trade.mine.push(aa_net::trade::TradeOffer {
            container: 2,
            slot: 4,
            amount: 0,
            item: Some(item(9_007_199_254_740_993)),
        });
    }
    rt.lua
        .load("assert(X2Trade:LockTrade()); assert(X2Trade:IsTradeLocked()); assert(X2Trade:OkTrade())")
        .exec()?;
    {
        let mut v = state.0.borrow_mut();
        assert_eq!(v.commands[0].command, TradeCommand::Lock { locked: false });
        assert_eq!(v.commands[1].command, TradeCommand::Confirm);
        v.commands.clear();
        v.inventory.bag[4].as_mut().unwrap().count -= 1;
    }
    rt.lua.load("assert(not X2Trade:OkTrade())").exec()?;
    {
        let mut v = state.0.borrow_mut();
        v.trade = Trade::default();
        v.trade.active = true;
        for slot in 0..11 {
            v.inventory.bag[slot] = Some(item(slot as u64 + 1));
        }
    }
    rt.lua
        .load("X2Cursor:ClearCursor(); for i=1,10 do assert(X2Trade:PutupTradeItem(i,0)) end; assert(not X2Trade:PutupTradeItem(11,0))")
        .exec()?;
    assert_eq!(state.0.borrow().commands.len(), 10);
    Ok(())
}

#[test]
fn money_edits_coalesce_to_the_final_value_including_zero() -> mlua::Result<()> {
    let (rt, _, state) = setup();
    rt.lua.load("assert(X2Trade:PutupTradeMoneyByStr('','1','0')); assert(X2Trade:PutupTradeMoneyByStr('','','')); assert(X2Trade:PutupTradeMoneyByStr('','','5')); assert(X2Trade:PutupTradeMoneyByStr('','','7'))").exec()?;
    {
        let mut v = state.0.borrow_mut();
        assert_eq!(v.commands.len(), 1);
        assert_eq!(v.commands.pop().unwrap().command, TradeCommand::OfferMoney { amount: 7 });
        v.pending.push((TradeCommand::OfferMoney { amount: 7 }, Instant::now()));
    }
    rt.lua.load("assert(X2Trade:PutupTradeMoneyByStr('','',''))").exec()?;
    assert_eq!(state.0.borrow().commands[0].command, TradeCommand::OfferMoney { amount: 0 });
    {
        let mut v = state.0.borrow_mut();
        v.commands.clear();
        v.pending.push((TradeCommand::OfferMoney { amount: 5 }, Instant::now()));
    }
    rt.lua.load("assert(X2Trade:PutupTradeMoneyByStr('','','7'))").exec()?;
    assert_eq!(state.0.borrow().commands[0].command, TradeCommand::OfferMoney { amount: 7 });
    Ok(())
}

fn controller() -> App {
    let (rt, bridge, state) = setup();
    state.0.borrow_mut().trade = Trade::default();
    rt.lua.load("seen={}; for _,event in ipairs({'TRADE_STARTED','TRADE_LOCKED','TRADE_OTHER_LOCKED','TRADE_UNLOCKED','TRADE_OK','TRADE_CANCELED','TRADE_MADE'}) do UIParent:SetEventHandler(event,function() seen[#seen+1]={event,X2Trade:IsTradeLocked(),X2Trade:IsOtherTradeLocked()} end) end").exec().unwrap();
    let inventory = state.0.borrow().inventory.clone();
    let mut app = App::new();
    app.insert_non_send(rt)
        .insert_non_send(HudBridge(bridge))
        .insert_resource(Trade::default())
        .insert_resource(inventory)
        .insert_resource(PlayerStats { money: 105, ..default() })
        .init_resource::<Units>()
        .add_message::<NetEvent>()
        .add_message::<NetCommand>()
        .add_systems(Update, sync);
    app.update();
    app
}

fn receive(app: &mut App, event: TradeEvent) {
    let event = Event::Trade(event);
    app.world_mut().resource_mut::<Trade>().apply(&event);
    app.world_mut().write_message(NetEvent(event));
}

fn drain(app: &mut App) -> Vec<TradeCommand> {
    app.world_mut()
        .resource_mut::<Messages<NetCommand>>()
        .drain()
        .map(|NetCommand(c)| match c {
            Command::Trade(c) => c,
            other => panic!("unexpected {other:?}"),
        })
        .collect()
}

#[test]
fn coalesced_acks_expose_each_lock_transition_to_original_queries() -> mlua::Result<()> {
    let mut app = controller();
    receive(&mut app, TradeEvent::Started { obj_id: 200 });
    receive(&mut app, TradeEvent::Locked { mine: true, other: false });
    receive(&mut app, TradeEvent::Locked { mine: true, other: true });
    receive(&mut app, TradeEvent::Confirmed { mine: true, other: false });
    receive(&mut app, TradeEvent::Locked { mine: false, other: false });
    app.update();
    app.world().non_send::<UiRuntime>().lua.load("assert(#seen==5); assert(seen[1][1]=='TRADE_STARTED' and not seen[1][2]); assert(seen[2][1]=='TRADE_LOCKED' and seen[2][2] and not seen[2][3]); assert(seen[3][1]=='TRADE_OTHER_LOCKED' and seen[3][2] and seen[3][3]); assert(seen[4][1]=='TRADE_OK'); assert(seen[5][1]=='TRADE_UNLOCKED' and not seen[5][2] and not seen[5][3])").exec()?;
    assert!(drain(&mut app).is_empty());
    Ok(())
}

#[test]
fn queued_offer_rechecks_identity_and_count_before_network_dispatch() -> mlua::Result<()> {
    let mut app = controller();
    receive(&mut app, TradeEvent::Started { obj_id: 200 });
    app.update();
    app.world().non_send::<UiRuntime>().lua.load("assert(X2Trade:PutupTradeItem(5,0))").exec()?;
    app.world_mut().resource_mut::<Inventory>().bag[4].as_mut().unwrap().count = 3;
    app.update();
    assert!(drain(&mut app).is_empty());
    app.world().non_send::<UiRuntime>().lua.load("assert(X2Trade:PutupTradeItem(5,0))").exec()?;
    app.update();
    assert_eq!(drain(&mut app), [TradeCommand::OfferItem { slot: 4 }]);
    assert!(app.world().resource::<Trade>().mine.is_empty());
    assert_eq!(app.world().resource::<Inventory>().bag[4].as_ref().unwrap().count, 3);
    Ok(())
}

#[test]
#[ignore = "loads the original world-stage assets; run in an isolated fixture process"]
fn original_trade_drag_money_lock_confirm_and_cancel() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, bridge, inventory, units) = fixture::runtime(1920.0, 1080.0)?;
    let mut app = App::new();
    app.insert_non_send(rt)
        .insert_non_send(HudBridge(bridge))
        .insert_resource(inventory)
        .insert_resource(units)
        .init_resource::<Trade>()
        .insert_resource(PlayerStats { money: 20_000, ..default() })
        .add_message::<NetEvent>()
        .add_message::<NetCommand>()
        .add_systems(Update, sync);
    app.update();
    app.world().non_send::<UiRuntime>().lua.load("local old=CreateEmptyWindow; CreateEmptyWindow=function(id,parent,...) local w=old(id,parent,...); if id=='popupMenu' then TRADE_POPUP=w end; return w end; ActivatePopupMenu(targetFrame,'target'); CreateEmptyWindow=old; assert(TRADE_POPUP); local clicked=false; for _,button in ipairs(TRADE_POPUP.buttons) do if button:GetText()==locale.targetPopup.requestTrade then button:OnClick(); clicked=true; break end end; assert(clicked,'native target menu must offer trade')").exec()?;
    app.update();
    assert_eq!(drain(&mut app), [TradeCommand::Invite { obj_id: 200 }]);
    receive(&mut app, TradeEvent::Started { obj_id: 200 });
    app.update();
    app.world().non_send::<UiRuntime>().lua.load("assert(IsVisibleTradeWindow()); X2Bag:HandleClick(5); assert(X2Cursor:GetCursorInfo()=='Item'); trade.window.myWindow.slotWindow:OnDragReceive('LeftButton'); assert(X2Cursor:GetHeld()==nil); assert(trade.window.myWindow.slotWindow.item[1].inventoryIdx==nil); local w=trade.window.myWindow.moneyEditsWindow; w.silverEdit:SetText('1'); w.copperEdit:SetText('5')").exec()?;
    app.update();
    assert_eq!(drain(&mut app), [TradeCommand::OfferItem { slot: 4 }, TradeCommand::OfferMoney { amount: 105 }]);
    receive(
        &mut app,
        TradeEvent::ItemOffered {
            container: 2,
            slot: 4,
            amount: 0,
        },
    );
    let own = app.world().resource::<Inventory>().bag[4].clone();
    app.world_mut().resource_mut::<Trade>().mine[0].item = own;
    receive(&mut app, TradeEvent::MoneyOffered(105));
    receive(&mut app, TradeEvent::OtherItemOffered(fixture::other_item()));
    receive(&mut app, TradeEvent::OtherMoneyOffered(25));
    app.update();
    app.world().non_send::<UiRuntime>().lua.load("X2Bag:HandleClick(7); assert(X2Cursor:GetCursorInfo()=='Item_Id'); trade.window.myWindow.slotWindow.item[2]:OnClickProc('LeftButton'); assert(X2Cursor:GetHeld()==nil)").exec()?;
    app.update();
    assert_eq!(drain(&mut app), [TradeCommand::OfferItem { slot: 6 }]);
    receive(
        &mut app,
        TradeEvent::ItemOffered {
            container: 2,
            slot: 6,
            amount: 0,
        },
    );
    let own = app.world().resource::<Inventory>().bag[6].clone();
    app.world_mut().resource_mut::<Trade>().mine[1].item = own;
    app.update();
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("trade.window.myWindow.slotWindow.item[2]:OnClickProc('RightButton')")
        .exec()?;
    app.update();
    assert_eq!(drain(&mut app), [TradeCommand::RemoveItem { slot: 6 }]);
    receive(&mut app, TradeEvent::ItemRemoved { container: 2, slot: 6 });
    let mut second = fixture::other_item();
    second.id += 1;
    receive(&mut app, TradeEvent::OtherItemOffered(second));
    receive(&mut app, TradeEvent::OtherItemRemoved(fixture::other_item()));
    app.update();
    app.world().non_send::<UiRuntime>().lua.load("assert(not X2Bag:IsLocked(7) and trade.window.myWindow.slotWindow.item[2].inventoryIdx==nil); assert(trade.window.targetWindow.slotWindow.item[1].otherIdx==nil and trade.window.targetWindow.slotWindow.item[2].otherIdx==0)").exec()?;
    app.world().non_send::<UiRuntime>().lua.load("assert(trade.window.myWindow.slotWindow.item[1].inventoryIdx==5 and trade.window.myWindow.slotWindow.item[1].tradeStack==4 and X2Bag:IsLocked(5)); trade.window.buttonLock:OnClick('LeftButton'); assert(not X2Trade:IsTradeLocked())").exec()?;
    app.update();
    assert_eq!(drain(&mut app), [TradeCommand::Lock { locked: true }]);
    receive(&mut app, TradeEvent::Locked { mine: true, other: false });
    receive(&mut app, TradeEvent::Locked { mine: true, other: true });
    app.update();
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("assert(trade.window.buttonOk:IsEnabled()); trade.window.buttonOk:OnClick('LeftButton')")
        .exec()?;
    app.update();
    assert_eq!(drain(&mut app), [TradeCommand::Confirm]);
    receive(&mut app, TradeEvent::Made(aa_net::items::ItemTasks::default()));
    app.update();
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("assert(not IsVisibleTradeWindow() and not X2Bag:IsLocked(5))")
        .exec()?;
    assert!(drain(&mut app).is_empty(), "original OnHide after settlement must not send another cancel");
    assert_eq!(
        app.world().resource::<Inventory>().bag[4].as_ref().unwrap().count,
        4,
        "controller must not apply settlement inventory tasks itself"
    );
    receive(&mut app, TradeEvent::Started { obj_id: 200 });
    app.update();
    app.world().non_send::<UiRuntime>().lua.load("trade.window:Show(false)").exec()?;
    app.update();
    assert!(matches!(drain(&mut app).as_slice(), [TradeCommand::Cancel { .. }]));
    receive(&mut app, TradeEvent::Canceled { reason: 0, caused_by_me: true });
    app.update();
    assert!(drain(&mut app).is_empty());
    assert!(app.world().non_send::<UiRuntime>().errors.is_empty());
    Ok(())
}
