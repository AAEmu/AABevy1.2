use super::*;
use crate::hud::api::{HudData, Request};
use aa_ui::UiRuntime;
use mlua::FromLua;
use std::{cell::RefCell, rc::Rc};

pub(super) fn runtime() -> Result<(UiRuntime, Shared), Box<dyn std::error::Error>> {
    let rt = UiRuntime::new("en_us", 1280., 720.)?;
    let state = Rc::new(RefCell::new(HudData::default()));
    rt.lua
        .load(
            r#"
        X2Store={}; ERROR_MSG=1; LOCALIZED={}; NOTICES={}
        STUB_SELL=0
        X2Store.SellBackPackGoods=function() STUB_SELL=STUB_SELL+1 end
        X2Locale={LocalizeUiText=function(_,group,key)
          assert(group==ERROR_MSG)
          LOCALIZED[#LOCALIZED+1]={key,X2Store:GetSpecialtyRatio()}
          return key
        end}
        local w=UIParent:CreateWidget('emptywidget','specialtyObserver','UIParent')
        w:RegisterEvent('NPC_INTERACTION_START');w:RegisterEvent('SYSMSG')
        w:SetHandler('OnEvent',function(_,event,...)
          assert(select('#',...)==1)
          NOTICES[#NOTICES+1]={event,...,X2Store:GetSpecialtyRatio()}
        end)
    "#,
        )
        .exec()?;
    install(&rt.lua, &state)?;
    Ok((rt, state))
}

#[test]
fn getter_ignores_arguments_and_uses_signed_native_float_rounding() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = runtime()?;
    assert_eq!(rt.lua.load("X2Store:GetSpecialtyRatio()").eval::<f64>()?, 100.);
    for ratio in [1, -7, 101, 16_777_217, i32::MIN, i32::MAX] {
        receive(None, &state, &Event::SpecialtyRatio(ratio));
        let values: MultiValue = rt.lua.load("return X2Store.GetSpecialtyRatio(nil,{},false,'ignored',1)").eval()?;
        assert_eq!(values.len(), 1);
        assert_eq!(f64::from_lua(values.into_iter().next().unwrap(), &rt.lua)?, f64::from(ratio as f32));
    }
    rt.lua.load("assert(#NOTICES==0 and #LOCALIZED==0)").exec()?;
    Ok(())
}

#[test]
fn sell_backpack_goods_queues_the_native_sale_request() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = runtime()?;
    state.borrow_mut().dialog.npc = Some((0x684b, 4473));
    rt.lua.load("X2Store:SellBackPackGoods()").exec()?;
    assert_eq!(
        rt.lua.load("STUB_SELL").eval::<i64>()?,
        0,
        "X2Store:SellBackPackGoods is still the aa-ui stub, which sends nothing"
    );
    assert_eq!(
        state.borrow().requests,
        vec![Request::SellBackpack { obj_id: 0x684b }],
        "the sale call must queue the interaction NPC object id"
    );
    Ok(())
}

#[test]
fn sale_uses_the_interaction_npc_not_the_current_target() -> Result<(), Box<dyn std::error::Error>>
{
    let (rt, state) = runtime()?;
    {
        let mut data = state.borrow_mut();
        data.dialog.npc = Some((0x684b, 4473));
        data.specialty.target.obj_id = Some(0x1111);
    }
    rt.lua.load("X2Store:SellBackPackGoods()").exec()?;
    assert_eq!(
        state.borrow().requests,
        vec![Request::SellBackpack { obj_id: 0x684b }],
        "the sale must use the interaction manager's NPC, not the refund target"
    );
    Ok(())
}

#[test]
fn receipt_callbacks_read_native_order_and_zero_resets_only_after_error() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = runtime()?;
    for ratio in [145, -9, 0, 0, 211] {
        receive(Some(&rt.lua), &state, &Event::SpecialtyRatio(ratio));
    }
    rt.lua
        .load(
            r#"
        assert(#NOTICES==5 and #LOCALIZED==2)
        assert(NOTICES[1][1]=='NPC_INTERACTION_START' and NOTICES[1][2]=='specialty' and NOTICES[1][3]==145)
        assert(NOTICES[2][1]=='NPC_INTERACTION_START' and NOTICES[2][3]==-9)
        assert(LOCALIZED[1][1]=='STORE_NPC_NOT_HANDLE_THIS_ITEM' and LOCALIZED[1][2]==-9)
        assert(NOTICES[3][1]=='SYSMSG' and NOTICES[3][2]=='STORE_NPC_NOT_HANDLE_THIS_ITEM' and NOTICES[3][3]==-9)
        assert(LOCALIZED[2][2]==100 and NOTICES[4][3]==100)
        assert(NOTICES[5][1]=='NPC_INTERACTION_START' and NOTICES[5][3]==211)
        assert(X2Store:GetSpecialtyRatio()==211)
    "#,
        )
        .exec()?;
    Ok(())
}

#[test]
fn actual_schedule_keeps_cache_without_hud_and_does_not_replay_dropped_callbacks() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = runtime()?;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_message::<NetEvent>()
        .insert_non_send(HudBridge(state.clone()))
        .insert_resource(crate::hud::HudBuilt { up: false, fresh: false });
    crate::hud::enchant::plugin(&mut app);
    app.world_mut().write_message(NetEvent(Event::SpecialtyRatio(141)));
    app.update();
    assert_eq!(state.borrow().specialty.ratio, 141);
    for event in [
        Event::Disconnected { message: String::new() },
        Event::LeftWorld { target: 0 },
        Event::PhaseChanged(aa_net::Phase::Authenticating),
    ] {
        app.world_mut().write_message(NetEvent(event));
    }
    app.update();
    assert_eq!(state.borrow().specialty.ratio, 141);
    app.insert_non_send(rt);
    app.update();
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("assert(#NOTICES==0 and X2Store:GetSpecialtyRatio()==141)")
        .exec()?;
    for ratio in [111, 0, 222] {
        app.world_mut().write_message(NetEvent(Event::SpecialtyRatio(ratio)));
    }
    app.update();
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load(
            r#"
      assert(#NOTICES==3 and NOTICES[1][3]==111 and NOTICES[2][1]=='SYSMSG' and NOTICES[2][3]==111)
      assert(NOTICES[3][3]==222 and X2Store:GetSpecialtyRatio()==222)
    "#,
        )
        .exec()?;
    Ok(())
}

#[test]
fn ratio_callback_observes_preceding_native_rows_and_not_later_rows_in_same_frame() -> Result<(), Box<dyn std::error::Error>> {
    use aa_net::{
        items::{InvenChunk, ItemTask, ItemTasks},
        model::Item,
    };
    let (rt, state) = runtime()?;
    let spawn = crate::hud::expansion::fixture::spawn();
    let mut items = vec![Some(Item {
        id: 123,
        template_id: 31432,
        count: 7,
        ..Default::default()
    })];
    items.resize(10, None);
    for event in [
        Event::CharacterSelected(Box::new(spawn.state.clone())),
        Event::PlayerSpawn(Box::new(spawn)),
        Event::InventoryInit {
            bag_slots: 10,
            bank_slots: 0,
        },
        Event::InventoryChunk(InvenChunk {
            container: 2,
            chunks: 1,
            start_chunk: 0,
            items,
            truncated: false,
        }),
    ] {
        crate::hud::enchant::dispatch(None, &state, &event, None);
    }
    let current = state.clone();
    rt.lua.globals().set(
        "native_count",
        rt.lua
            .create_function(move |_, ()| Ok(crate::hud::enchant::current(&current.borrow(), 123).map(|item| item.count)))?,
    )?;
    rt.lua
        .load(
            r#"
      assert(native_count()==7)
      NATIVE_ROW_NOTICES={}
      local w=UIParent:CreateWidget('emptywidget','specialtyNativeRowObserver','UIParent')
      w:RegisterEvent('NPC_INTERACTION_START')
      w:SetHandler('OnEvent',function(_,event,...)
        assert(event=='NPC_INTERACTION_START' and select('#',...)==1 and ...=='specialty')
        NATIVE_ROW_NOTICES[#NATIVE_ROW_NOTICES+1]=native_count()
      end)
    "#,
        )
        .exec()?;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_message::<NetEvent>()
        .insert_non_send(HudBridge(state.clone()))
        .insert_non_send(rt)
        .insert_resource(crate::hud::HudBuilt { up: false, fresh: false });
    crate::hud::enchant::plugin(&mut app);
    let count = |delta| {
        Event::ItemsChanged(ItemTasks {
            tasks: vec![ItemTask::Count {
                container: 2,
                slot: 0,
                item_id: 999,
                template_id: 999,
                delta,
            }],
            ..Default::default()
        })
    };
    for event in [count(-3), Event::SpecialtyRatio(127), count(5)] {
        app.world_mut().write_message(NetEvent(event));
    }
    app.update();
    app.world()
        .non_send::<UiRuntime>()
        .lua
        .load("assert(#NATIVE_ROW_NOTICES==1 and NATIVE_ROW_NOTICES[1]==4 and native_count()==9 and X2Store:GetSpecialtyRatio()==127)")
        .exec()?;
    Ok(())
}

#[test]
fn unchanged_original_interaction_dispatches_to_its_specialty_handle_after_cache_write() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state, path) = crate::hud::breath::fixture::runtime(1280., 720.)?;
    rt.lua
        .load(
            r#"
      -- A valid empty native equipment slot is a table with itemType0.
      assert(X2Equipment:GetEquippedItemTooltipText('player',ES_BACKPACK).itemType==0)
      SPECIALTY_OPENS=0;SPECIALTY_RATIO_AT_OPEN=nil
      -- Only the unprovided NPC/backpack price formatter endpoint is mocked.
      -- Original NPC event dispatch and ShowInteraction remain unchanged.
      GetSpecialtyStoreMessageBoxHandleWindow():SetHandler('OnShow',function(self)
        SPECIALTY_OPENS=SPECIALTY_OPENS+1
        SPECIALTY_RATIO_AT_OPEN=X2Store:GetSpecialtyRatio()
        self:Show(false)
      end)
    "#,
        )
        .exec()?;
    receive(Some(&rt.lua), &state, &Event::SpecialtyRatio(137));
    rt.lua
        .load("assert(X2Store:GetSpecialtyRatio()==137 and not GetSpecialtyStoreMessageBoxHandleWindow():IsVisible());assert(SPECIALTY_OPENS==1 and SPECIALTY_RATIO_AT_OPEN==137)")
        .exec()?;
    assert!(rt.errors.is_empty(), "original UI load errors: {:?}", rt.errors);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}
