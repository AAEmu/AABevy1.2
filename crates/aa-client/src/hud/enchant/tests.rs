use super::*;
use std::{cell::RefCell, rc::Rc};

#[test]
fn native_mode_wrappers_preserve_picked_bag_ignore_arguments_and_emit_exact_synchronous_events() -> Result<(), Box<dyn std::error::Error>> {
    let rt = UiRuntime::new("en_us", 1280., 720.)?;
    rt.lua.load("X2Item={};X2ItemEnchant={}").exec()?;
    let state = Rc::new(RefCell::new(super::super::api::HudData::default()));
    install(&rt.lua, &state)?;
    rt.lua.load(r#"
      seen={};local w=UIParent:CreateWidget('emptywidget','enchantObserver','UIParent')
      for _,event in ipairs({'ENTER_ENCHANT_ITEM_MODE','SWITCH_ENCHANT_ITEM_MODE','LEAVE_ENCHANT_ITEM_MODE','UPDATE_ENCHANT_ITEM_MODE'}) do w:RegisterEvent(event) end
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
      assert(#seen==1 and seen[1].event=='ENTER_ENCHANT_ITEM_MODE' and seen[1].n==1 and seen[1][1]=='grade')
      hold();X2ItemEnchant:EnterItemEnchantMode();assert(#seen==1 and X2Cursor:GetHeld().kind=='bag')
      hold();X2ItemEnchant:SwitchItemEnchantSocketingMode('ignored')
      hold();X2ItemEnchant:SwitchItemEnchantUnsocketMode()
      hold();X2ItemEnchant:SwitchItemEnchantGemMode()
      hold();X2ItemEnchant:SwitchItemEnchantGradeMode()
      assert(#seen==5)
      for i,mode in ipairs({'socketing','unsocket','gem','grade'})do
        assert(seen[i+1].event=='SWITCH_ENCHANT_ITEM_MODE' and seen[i+1].n==1 and seen[i+1][1]==mode)
      end
      hold();assert(select('#',X2ItemEnchant:LeaveItemEnchantMode())==0)
      assert(#seen==6 and seen[6].event=='LEAVE_ENCHANT_ITEM_MODE' and seen[6].n==0)
      assert(X2Item:IsInEnchantMode()==false)
    "#).exec()?;
    assert_eq!(state.borrow().enchant.mode, Mode::Off);
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
