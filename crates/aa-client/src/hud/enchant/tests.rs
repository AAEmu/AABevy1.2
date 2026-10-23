use super::*;
use std::{cell::RefCell, rc::Rc};

fn execute(rt: &UiRuntime, state: &Shared) -> aa_net::item_effect::ItemEffectRequest {
    rt.lua.load("X2ItemEnchant:Execute()").exec().unwrap();
    let requests = std::mem::take(&mut state.borrow_mut().requests);
    match requests.as_slice() {
        [super::super::api::Request::ItemEffect { character_id: 47, request }] => request.clone(),
        other => panic!("actual typed enchant request missing: {other:?}"),
    }
}

#[test]
fn original_execute_queues_all_four_typed_modes_without_optimistic_inventory_changes() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state, path) = fixture::runtime(1280., 720.)?;
    let mut app = App::new();
    app.insert_non_send(HudBridge(state.clone()))
        .init_resource::<NextState<crate::Screen>>()
        .init_resource::<aa_net::state::ActionBar>()
        .init_resource::<aa_net::state::Inventory>()
        .init_resource::<aa_net::state::Target>()
        .init_resource::<aa_net::state::Skills>()
        .init_resource::<aa_net::state::Buffs>()
        .init_resource::<super::super::usable::InFlight>()
        .add_message::<aa_net::NetCommand>()
        .add_message::<super::super::alerts::UseFailed>()
        .add_message::<crate::ingame::ground_target::SkillPressed>()
        .init_resource::<aa_net::state::Units>()
        .add_systems(Update, super::super::api::handle_requests);
    for (mode, skill) in [("grade", 22522), ("socket", 23728), ("unsocket", 23729), ("gem", 11285)] {
        fixture::select(&rt, mode)?;
        fixture::fill(&rt, &state, mode, mode == "grade")?;
        let before = state.borrow().enchant.selection.slots;
        let request = execute(&rt, &state);
        assert_eq!(request.skill_id(), skill);
        assert_eq!(request.target().item.id, 9_007_199_254_740_993);
        assert_eq!(request.target().slot, 0);
        assert_eq!(request.support().is_some(), mode == "grade");
        assert_eq!(state.borrow().enchant.selection.slots, before);
        assert_eq!(current(&state.borrow(), request.source().item.id).unwrap(), &request.source().item);
        assert!(request_current(&state, 47, &request));
        assert!(!request_current(&state, 48, &request));
        state.borrow_mut().requests.extend([
            super::super::api::Request::ItemEffect {
                character_id: 48,
                request: request.clone(),
            },
            super::super::api::Request::ItemEffect {
                character_id: 47,
                request: request.clone(),
            },
        ]);
        app.update();
        let commands: Vec<_> = app.world_mut().resource_mut::<Messages<aa_net::NetCommand>>().drain().collect();
        assert!(
            matches!(&commands[..], [aa_net::NetCommand(aa_net::Command::ItemEffect {character_id:47, request: sent})] if sent == &request)
        );
        // Even the source without reagent consumption waits for actual write.
        result(
            Some(&rt.lua),
            &state,
            if mode == "grade" {
                ResultKind::Grade
            } else if mode == "gem" {
                ResultKind::Magical
            } else {
                ResultKind::Socket
            },
        );
        assert!(state.borrow().enchant.operation.is_some());
        dispatch(
            Some(&rt),
            &state,
            &Event::ItemEffectRejected {
                character_id: 47,
                request: request.clone(),
                message: "fixture transport rejected".into(),
            },
            None,
        );
        assert!(state.borrow().enchant.operation.is_none());
        state.borrow_mut().requests.push(super::super::api::Request::ItemEffect {
            character_id: 47,
            request: request.clone(),
        });
        app.update();
        assert!(
            app.world_mut()
                .resource_mut::<Messages<aa_net::NetCommand>>()
                .drain()
                .next()
                .is_none(),
            "retired operation cannot drain an old queued request"
        );
    }
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
fn actual_result_callback_precedes_completion_and_each_row_completes_before_the_next() -> Result<(), Box<dyn std::error::Error>> {
    use aa_net::{
        item_effect::GradeEnchantResult,
        items::{ItemTask, ItemTasks},
    };
    let (rt, state, path) = fixture::runtime(1280., 720.)?;
    fixture::fill(&rt, &state, "grade", true)?;
    for result_first in [true, false] {
        let request = execute(&rt, &state);
        dispatch(
            Some(&rt),
            &state,
            &Event::ItemEffectRequested {
                character_id: 47,
                request: request.clone(),
            },
            None,
        );
        rt.lua
            .load(
                r#"
          ORDER={};COMPLETION_OBSERVER=COMPLETION_OBSERVER or UIParent:CreateWidget('emptywidget','completionOrder','UIParent')
          local w=COMPLETION_OBSERVER;w:RegisterEvent('GRADE_ENCHANT_RESULT');w:RegisterEvent('UPDATE_ENCHANT_ITEM_MODE')
          w:SetHandler('OnEvent',function(_,event,presence,working)
            if event=='GRADE_ENCHANT_RESULT' then ORDER[#ORDER+1]={'result'};return end
            if working then return end
            assert(working==false and presence==true)
            ORDER[#ORDER+1]={'complete',X2ItemEnchant:GetEnchantItemInfo().total}
            -- Synchronous reentry must see the reset operation/valid selection.
            X2ItemEnchant:SwitchItemEnchantGradeMode()
          end)
        "#,
            )
            .exec()?;
        let receipt = Event::GradeEnchantResult(GradeEnchantResult {
            raw_result: 2,
            item: Some(request.target().item.clone()),
            item_wire: vec![],
            before_grade: 6,
            after_grade: 6,
        });
        let changes = Event::ItemsChanged(ItemTasks {
            reason: 40,
            tasks: vec![
                ItemTask::Count {
                    container: 2,
                    slot: request.source().slot,
                    item_id: 999,
                    template_id: 999,
                    delta: -1,
                },
                ItemTask::Count {
                    container: 2,
                    slot: request.source().slot,
                    item_id: 999,
                    template_id: 999,
                    delta: -1,
                },
            ],
            ..Default::default()
        });
        if result_first {
            dispatch(Some(&rt), &state, &receipt, None);
            dispatch(Some(&rt), &state, &changes, None);
        } else {
            dispatch(Some(&rt), &state, &changes, None);
            dispatch(Some(&rt), &state, &receipt, None);
        }
        assert!(state.borrow().enchant.operation.is_none());
        // Initial total7; first pass restores at6 before the second decrement,
        // second pass starts at5 and restores after both rows at3.
        rt.lua
            .load(if result_first {
                "assert(#ORDER==2 and ORDER[1][1]=='result' and ORDER[2][1]=='complete' and ORDER[2][2]==6)"
            } else {
                "assert(#ORDER==2 and ORDER[1][1]=='result' and ORDER[2][1]=='complete' and ORDER[2][2]==3)"
            })
            .exec()?;
        // The original API can overwrite an active capture on another Execute;
        // no stop/skill-error ownership is inferred by this test.
    }
    rt.lua.load("COMPLETION_OBSERVER:SetHandler('OnEvent',function()end)").exec()?;
    fixture::fill(&rt, &state, "grade", true)?;
    let request = execute(&rt, &state);
    dispatch(
        Some(&rt),
        &state,
        &Event::ItemEffectRequested {
            character_id: 47,
            request: request.clone(),
        },
        None,
    );
    rt.lua
        .load(
            r#"
      local w=UIParent:CreateWidget('emptywidget','resultChangesMode','UIParent')
      w:RegisterEvent('GRADE_ENCHANT_RESULT')
      w:SetHandler('OnEvent',function()X2ItemEnchant:SwitchItemEnchantGemMode()end)
    "#,
        )
        .exec()?;
    dispatch(
        Some(&rt),
        &state,
        &Event::GradeEnchantResult(GradeEnchantResult {
            raw_result: 2,
            item: Some(request.target().item.clone()),
            item_wire: vec![],
            before_grade: 6,
            after_grade: 6,
        }),
        None,
    );
    assert_eq!(state.borrow().enchant.mode, Mode::Gem);
    assert!(
        !state.borrow().enchant.operation.as_ref().unwrap().result_seen(),
        "Lua switched current mode before the Grade receiver"
    );
    dispatch(
        Some(&rt),
        &state,
        &Event::ItemsChanged(ItemTasks {
            reason: 40,
            tasks: vec![ItemTask::Count {
                container: 2,
                slot: request.source().slot,
                item_id: 999,
                template_id: 999,
                delta: -1,
            }],
            ..Default::default()
        }),
        None,
    );
    assert!(state.borrow().enchant.operation.is_some());
    dispatch(
        Some(&rt),
        &state,
        &Event::LunastoneSocketingResult {
            raw_result: 0,
            item_id: request.target().item.id,
            template_id: 24632,
        },
        None,
    );
    assert!(state.borrow().enchant.operation.is_none());
    assert_eq!(
        state.borrow().enchant.selection.slots,
        [Some(0), None, None],
        "current Gem receipt restores only target, even for raw failure"
    );
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
fn native_mode_wrappers_preserve_picked_bag_ignore_arguments_and_emit_exact_synchronous_events() -> Result<(), Box<dyn std::error::Error>> {
    let rt = UiRuntime::new("en_us", 1280., 720.)?;
    rt.lua.load("X2Item={};X2ItemEnchant={}").exec()?;
    let state = Rc::new(RefCell::new(super::super::api::HudData::default()));
    install(&rt.lua, &state)?;
    rt.lua.load(r#"
      seen={};local w=UIParent:CreateWidget('emptywidget','enchantObserver','UIParent')
      for _,event in ipairs({'NPC_INTERACTION_END','ENTER_ENCHANT_ITEM_MODE','SWITCH_ENCHANT_ITEM_MODE','LEAVE_ENCHANT_ITEM_MODE','UPDATE_ENCHANT_ITEM_MODE'}) do w:RegisterEvent(event) end
      w:SetHandler('OnEvent',function(_,event,...)
        seen[#seen+1]={event=event,n=select('#',...),...}
        assert(X2Item:IsInEnchantMode()==(event~='LEAVE_ENCHANT_ITEM_MODE'))
        assert(X2Cursor:GetHeld().kind=='bag' and X2Cursor:GetHeld().slot==1)
        -- Actual window tab/submenu callbacks reenter these wrappers.
        if event=='ENTER_ENCHANT_ITEM_MODE' then X2ItemEnchant:SwitchItemEnchantGradeMode() end
      end)
      local function hold()X2Cursor:SetHeld({kind='bag',slot=1,count=1})end
      assert(select('#',X2Item:IsInEnchantMode(nil,'x'))==1 and X2Item:IsInEnchantMode()==false)
      hold();assert(select('#',X2ItemEnchant:LeaveItemEnchantMode(true,'x',{}))==0)
      assert(#seen==0 and X2Cursor:GetHeld().kind=='bag')
      hold();assert(select('#',X2ItemEnchant:EnterItemEnchantMode(nil,'socketing',false,{}))==0)
      assert(#seen==2 and seen[1].event=='NPC_INTERACTION_END' and seen[1].n==0)
      assert(seen[2].event=='ENTER_ENCHANT_ITEM_MODE' and seen[2].n==1 and seen[2][1]=='grade')
      hold();X2ItemEnchant:EnterItemEnchantMode();assert(#seen==2 and X2Cursor:GetHeld().kind=='bag')
      hold();X2ItemEnchant:SwitchItemEnchantSocketingMode('ignored')
      hold();X2ItemEnchant:SwitchItemEnchantUnsocketMode()
      hold();X2ItemEnchant:SwitchItemEnchantGemMode()
      hold();X2ItemEnchant:SwitchItemEnchantGradeMode()
      assert(#seen==6)
      for i,mode in ipairs({'socketing','unsocket','gem','grade'})do
        assert(seen[i+2].event=='SWITCH_ENCHANT_ITEM_MODE' and seen[i+2].n==1 and seen[i+2][1]==mode)
      end
      hold();assert(select('#',X2ItemEnchant:LeaveItemEnchantMode())==0)
      assert(#seen==7 and seen[7].event=='LEAVE_ENCHANT_ITEM_MODE' and seen[7].n==0)
      assert(X2Item:IsInEnchantMode()==false)
    "#).exec()?;
    assert_eq!(state.borrow().enchant.mode, Mode::Off);
    Ok(())
}

#[test]
fn typed_detail_receipt_updates_native_record_then_releases_only_its_proven_pending_key() -> Result<(), Box<dyn std::error::Error>> {
    use aa_net::{
        items::{InvenChunk, ItemDetailUpdate},
        model::{EquipDetail, Item, ItemDetail},
        state::Inventory,
    };
    let state = Rc::new(RefCell::new(super::super::api::HudData::default()));
    let spawn = super::super::expansion::fixture::spawn();
    let original = Item {
        id: 42,
        template_id: 31432,
        count: 1,
        ..Default::default()
    };
    let mut items = vec![None; 10];
    items[0] = Some(original.clone());
    for event in [
        Event::CharacterSelected(Box::new(spawn.state.clone())),
        Event::PlayerSpawn(Box::new(spawn)),
        Event::InventoryInit {
            bag_slots: 10,
            bank_slots: 10,
        },
        Event::InventoryChunk(InvenChunk {
            container: 2,
            chunks: 1,
            start_chunk: 0,
            items,
            truncated: false,
        }),
    ] {
        state.borrow_mut().enchant.selection.receive(&event);
    }
    let mut inventory = Inventory::default();
    inventory.bag_slots = 10;
    inventory.bag.resize(10, None);
    inventory.bag[0] = Some(original.clone());
    inventory.mark_item_slot_pending(2, 0);
    inventory.mark_item_slot_pending(2, 1);
    let rt = UiRuntime::new("en_us", 1280., 720.)?;
    rt.lua
        .load("X2Item={};X2ItemEnchant={};X2Bag={IsLocked=function(_,slot)return slot==40 end}")
        .exec()?;
    install(&rt.lua, &state)?;
    state
        .borrow()
        .game
        .0
        .set(super::super::data::GameData::load(aa_anim::db::DEFAULT_DB)?)
        .ok()
        .unwrap();
    rt.lua
        .load("X2ItemEnchant:EnterItemEnchantMode();assert(X2ItemEnchant:SetItemSlotFromBag(1) and X2Bag:IsLocked(1))")
        .exec()?;
    rt.lua
        .load(
            r#"
      BAG_LOCK_NOTICES={};local w=UIParent:CreateWidget('emptywidget','bagLockObserver','UIParent')
      w:RegisterEvent('BAG_LOCK_UPDATE');w:RegisterEvent('UPDATE_ENCHANT_ITEM_MODE')
      w:SetHandler('OnEvent',function(_,event,...)
        assert(event=='BAG_LOCK_UPDATE' and select('#',...)==1 and ...==1)
        assert(not X2Bag:IsLocked(1) and X2Bag:IsLocked(2) and X2Bag:IsLocked(40))
        BAG_LOCK_NOTICES[#BAG_LOCK_NOTICES+1]=...
      end)
    "#,
        )
        .exec()?;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_message::<NetEvent>()
        .insert_non_send(HudBridge(state.clone()))
        .insert_non_send(rt)
        .insert_resource(super::super::HudBuilt { up: true, fresh: false })
        .insert_resource(inventory);
    plugin(&mut app);
    let update = |item_id, container| {
        let mut raw_detail = vec![0; 56];
        raw_detail[0] = 1;
        raw_detail[5] = 77;
        ItemDetailUpdate {
            item_id,
            container,
            slot: 0,
            raw_detail,
        }
    };
    app.world_mut().write_message(NetEvent(Event::ItemDetailUpdated(update(43, 2))));
    app.update();
    assert!(
        app.world().resource::<Inventory>().item_slot_pending(2, 0),
        "wrong ID has no native mutation/unlock"
    );
    app.world_mut().write_message(NetEvent(Event::ItemDetailUpdated(update(42, 3))));
    app.update();
    assert!(
        app.world().resource::<Inventory>().item_slot_pending(2, 0),
        "bank detail receipt is a native no-op here"
    );
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("assert(#BAG_LOCK_NOTICES==0)")
        .exec()?;
    app.world_mut().write_message(NetEvent(Event::ItemDetailUpdated(update(42, 2))));
    app.update();
    assert!(!app.world().resource::<Inventory>().item_slot_pending(2, 0));
    assert!(
        app.world().resource::<Inventory>().item_slot_pending(2, 1),
        "unrelated pending key survives"
    );
    assert_eq!(
        app.world().resource::<Inventory>().bag[0].as_ref(),
        Some(&original),
        "legacy resource contents are not promoted to native records"
    );
    assert_eq!(
        state.borrow().enchant.selection.records.item(2, 0).unwrap().detail,
        ItemDetail::Equip(EquipDetail {
            durability: 77,
            ..Default::default()
        })
    );
    assert_eq!(
        state.borrow().enchant.selection.slots[0],
        Some(0),
        "a pending release does not clear the selected item pointer"
    );
    app.world_mut().write_message(NetEvent(Event::ItemDetailUpdated(update(42, 2))));
    app.update();
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("assert(#BAG_LOCK_NOTICES==2)")
        .exec()?;
    Ok(())
}

#[test]
fn item_operation_error_reports_before_each_ordered_lock_release_without_an_active_game() -> Result<(), Box<dyn std::error::Error>> {
    use aa_net::{items::ItemOperationError, state::Inventory};
    let state = Rc::new(RefCell::new(super::super::api::HudData::default()));
    let rt = UiRuntime::new("en_us", 800., 600.)?;
    rt.lua
        .load("X2Item={};X2ItemEnchant={};X2Bag={};X2Locale={LocalizeUiText=function(_,_,key)return key end};ERROR_MSG=1")
        .exec()?;
    install(&rt.lua, &state)?;
    rt.lua
        .load(
            r#"
      NOTICES={};local w=UIParent:CreateWidget('emptywidget','itemErrorObserver','UIParent')
      w:RegisterEvent('SYSMSG');w:RegisterEvent('BAG_LOCK_UPDATE');w:RegisterEvent('UPDATE_ENCHANT_ITEM_MODE')
      w:SetHandler('OnEvent',function(_,event,...)
        assert(select('#',...)==1 and event~='UPDATE_ENCHANT_ITEM_MODE')
        local value=...;NOTICES[#NOTICES+1]={event,value,X2Bag:IsLocked(1),X2Bag:IsLocked(2)}
      end)
    "#,
        )
        .exec()?;
    let mut inventory = Inventory::default();
    inventory.mark_item_slot_pending(2, 0);
    inventory.mark_item_slot_pending(2, 1);
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_message::<NetEvent>()
        .insert_non_send(HudBridge(state.clone()))
        .insert_non_send(rt)
        .insert_resource(super::super::HudBuilt { up: true, fresh: false })
        .insert_resource(inventory);
    plugin(&mut app);
    let receipt = ItemOperationError {
        container: 2,
        slot: 0,
        second_container: 2,
        second_slot: 1,
        error_code: 0,
    };
    // First-frame error Lua must see fresh shared locks before either release.
    app.world_mut()
        .write_message(NetEvent(Event::ItemOperationError(ItemOperationError {
            error_code: 0xb5,
            ..receipt
        })));
    app.update();
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load(
            r#"
      assert(#NOTICES==3 and NOTICES[1][1]=='SYSMSG' and NOTICES[1][2]=='BAG_INVALID_ITEM' and NOTICES[1][3] and NOTICES[1][4])
      assert(NOTICES[2][1]=='BAG_LOCK_UPDATE' and NOTICES[2][2]==1 and not NOTICES[2][3] and NOTICES[2][4])
      assert(NOTICES[3][1]=='BAG_LOCK_UPDATE' and NOTICES[3][2]==2 and not NOTICES[3][3] and not NOTICES[3][4])
    "#,
        )
        .exec()?;
    // Repeated absent keys still notify twice, in received order.
    app.world_mut()
        .write_message(NetEvent(Event::ItemOperationError(ItemOperationError {
            second_slot: 0,
            error_code: 0xb5,
            ..receipt
        })));
    app.update();
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("assert(#NOTICES==6 and NOTICES[5][2]==1 and NOTICES[6][2]==1)")
        .exec()?;
    app.world_mut().resource_mut::<Inventory>().mark_item_slot_pending(2, 0);
    app.world_mut().resource_mut::<Inventory>().mark_item_slot_pending(2, 1);
    app.world_mut().write_message(NetEvent(Event::ItemOperationError(receipt)));
    app.update();
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("assert(#NOTICES==6 and X2Bag:IsLocked(1) and X2Bag:IsLocked(2))")
        .exec()?;
    assert!(state.borrow().enchant.operation.is_none());
    Ok(())
}

#[test]
fn original_bag_enchant_button_tabs_submenus_and_close_follow_controller_mode() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state, path) = fixture::runtime(1920., 1080.)?;
    assert_eq!(state.borrow().enchant.mode, Mode::Grade);
    rt.lua
        .load("assert(GetEnchantWindow().tab:GetSelectedTab()==1 and not GetEnchantWindow().tab.window[1].leftButton:IsEnabled())")
        .exec()?;
    fixture::select(&rt, "socket")?;
    assert_eq!(state.borrow().enchant.mode, Mode::Socket);
    rt.lua
        .load("assert(GetEnchantWindow().tab:GetSelectedTab()==2 and GetEnchantWindow().tab.window[2].tabSubMenu.selIndex==1)")
        .exec()?;
    fixture::select(&rt, "unsocket")?;
    assert_eq!(state.borrow().enchant.mode, Mode::Unsocket);
    fixture::select(&rt, "gem")?;
    assert_eq!(state.borrow().enchant.mode, Mode::Gem);
    super::super::expansion::fixture::click(&rt, "GetEnchantWindow().titleBar.closeButton")?;
    rt.lua
        .load("assert(not GetEnchantWindow():IsVisible() and not X2Item:IsInEnchantMode())")
        .exec()?;
    assert_eq!(state.borrow().enchant.mode, Mode::Off);
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[test]
fn received_lifecycle_retires_the_mode_in_the_actual_schedule() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state, path) = fixture::runtime(1920., 1080.)?;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_message::<NetEvent>()
        .insert_non_send(HudBridge(state.clone()))
        .insert_non_send(rt)
        .insert_resource(super::super::HudBuilt { up: true, fresh: false });
    plugin(&mut app);
    app.world_mut()
        .write_message(NetEvent(Event::Disconnected { message: "fixture".into() }));
    app.update();
    assert_eq!(state.borrow().enchant.mode, Mode::Off);
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("assert(not GetEnchantWindow():IsVisible() and not X2Item:IsInEnchantMode())")
        .exec()?;
    app.world_mut().resource_mut::<super::super::HudBuilt>().up = false;
    state.borrow_mut().enchant.mode = Mode::Gem;
    app.world_mut().write_message(NetEvent(Event::LeftWorld { target: 0 }));
    app.update();
    assert_eq!(state.borrow().enchant.mode, Mode::Off);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[test]
fn original_filled_bag_right_clicks_admit_actual_items_in_all_four_enchant_modes() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state, path) = fixture::runtime(1920., 1080.)?;
    for mode in ["grade", "socket", "unsocket", "gem"] {
        fixture::select(&rt, mode)?;
        fixture::fill(&rt, &state, mode, mode == "grade")?;
        // Event dispatch may isolate a script exception. Invoke the original
        // full updater too, so a missing nested getter field cannot silently
        // leave the later source portrait/controls stale while these APIs pass.
        rt.lua.load("local p=GetEnchantWindow().tab.window[GetEnchantWindow().tab:GetSelectedTab()];p:SlotAllUpdate(true,false);assert(p.slotTargetItem.info.itemType==31432 and p.slotEnchantItem.info~=nil and p.leftButton:IsEnabled())").exec()?;
        rt.lua
            .load("assert(select('#',X2ItemEnchant:GetTargetItemInfo())==1);assert(select('#',X2ItemEnchant:GetEnchantItemInfo())==1)")
            .exec()?;
        if mode == "grade" {
            rt.lua.load("local t=X2ItemEnchant:GetTargetItemInfo();assert(t.grade_enchant_cost==506607 and t.grade_enchant_currency==0);local r=X2ItemEnchant:GetRatioInfos();assert(r.success_var_ratio==50)").exec()?;
        }
        if mode == "unsocket" {
            rt.lua.load("local s=X2ItemEnchant:GetTargetItemInfo().socketInfo;assert(s.maxSocket==4 and #s.socketItem==4 and s.socketItem[1]==31003 and s.socketItem[2]==0 and s.socketItem[4]==0);local m=X2Item:GetItemInfoByType(31003,0,1024).modifier;assert(#m==1 and m[1].name=='max_mana' and m[1].value==200)").exec()?;
        }
        if mode == "gem" {
            rt.lua.load("local m=X2ItemEnchant:GetEnchantItemInfo().modifier;assert(#m==2 and m[1].name=='dex' and m[1].value==2 and m[2].name=='sta' and m[2].value==3)").exec()?;
        }
    }
    rt.lua
        .load(
            r#"
      assert(X2Item:GetItemInfoByType(31003,0).modifier~=nil)
      for _,flags in ipairs({0,'1024',false,{},0/0,math.huge,2147483648})do assert(X2Item:GetItemInfoByType(31003,0,flags).modifier==nil)end
      assert(X2Item:GetItemInfoByType(31003,0,nil).modifier==nil)
      assert(X2Item:GetItemInfoByType(31003,0,1024.9).modifier~=nil)
      assert(X2Item:GetItemInfoByType(24632,0,0).modifier~=nil)
    "#,
        )
        .exec()?;
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}
