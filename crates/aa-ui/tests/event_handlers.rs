//! Native UIParent callback multiplicity, ordering and live release semantics.

use aa_ui::{UiRuntime, lua};

#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;

fn run(ui: &UiRuntime, code: &str) {
    ui.exec("@event_handlers", code).expect(code);
}

#[test]
fn root_callbacks_preserve_every_registration_after_widget_listeners() {
    let ui = UiRuntime::new("en_us", 800., 600.).unwrap();
    run(
        &ui,
        r#"
        calls={}
        function first(a,b) assert(a==7 and b=='payload');table.insert(calls,'first') end
        function second(a,b) assert(a==7 and b=='payload');table.insert(calls,'second') end
        UIParent:SetEventHandler('SPAWN_PET',first)
        listener=UIParent:CreateWidget('emptywidget','listener','UIParent')
        listener:RegisterEvent('SPAWN_PET')
        listener:SetHandler('OnEvent',function(self,event,a,b)
            assert(self==listener and event=='SPAWN_PET' and a==7 and b=='payload')
            table.insert(calls,'widget')
        end)
        UIParent:SetEventHandler('SPAWN_PET',second)
        UIParent:SetEventHandler('SPAWN_PET',first)
    "#,
    );
    lua::fire_ui_event(
        &ui.lua,
        "SPAWN_PET",
        vec![
            mlua::Value::Integer(7),
            mlua::Value::String(ui.lua.create_string("payload").unwrap()),
        ],
    );
    run(
        &ui,
        r#"
        assert(table.concat(calls,',')=='widget,first,second,first')
        calls={}
        UIParent:ReleaseEventHandler('SPAWN_PET',first)
        UIParent:ReleaseEventHandler('SPAWN_PET') -- callback is required, not release-all
        UIParent:ReleaseEventHandler('SPAWN_PET',function() end)
    "#,
    );
    lua::fire_ui_event(
        &ui.lua,
        "SPAWN_PET",
        vec![
            mlua::Value::Integer(7),
            mlua::Value::String(ui.lua.create_string("payload").unwrap()),
        ],
    );
    run(
        &ui,
        "assert(table.concat(calls,',')=='widget,second,first')",
    );
}

#[test]
fn root_dispatch_observes_additions_and_releases_during_the_same_event() {
    let ui = UiRuntime::new("en_us", 800., 600.).unwrap();
    run(
        &ui,
        r#"
        calls={}
        function last() table.insert(calls,'last') end
        function removed() error('released callback ran') end
        function first()
            table.insert(calls,'first')
            UIParent:ReleaseEventHandler('TEST',first)
            UIParent:ReleaseEventHandler('TEST',removed)
            UIParent:SetEventHandler('TEST',last)
        end
        UIParent:SetEventHandler('TEST',first)
        UIParent:SetEventHandler('TEST',removed)
        listener=UIParent:CreateWidget('emptywidget','listener','UIParent')
        listener:RegisterEvent('TEST')
        listener:SetHandler('OnEvent',function(self)
            table.insert(calls,'widget')
            UIParent:SetEventHandler('TEST',function() table.insert(calls,'fromWidget') end)
            self:ReleaseHandler('OnEvent')
        end)
    "#,
    );
    lua::fire_ui_event(&ui.lua, "TEST", vec![]);
    run(
        &ui,
        "assert(table.concat(calls,',')=='widget,first,fromWidget,last');calls={}",
    );
    lua::fire_ui_event(&ui.lua, "TEST", vec![]);
    run(
        &ui,
        "assert(table.concat(calls,',')=='fromWidget,last',table.concat(calls,','))",
    );
}

#[test]
fn callback_errors_do_not_skip_other_registrations() {
    let ui = UiRuntime::new("en_us", 800., 600.).unwrap();
    run(
        &ui,
        r#"
        calls=0
        UIParent:SetEventHandler('TEST',function() error('fixture failure') end)
        UIParent:SetEventHandler('TEST',function() calls=calls+1 end)
    "#,
    );
    lua::fire_ui_event(&ui.lua, "TEST", vec![]);
    run(&ui, "assert(calls==1)");
}

#[test]
fn slots_return_empty_hotkey_strings_when_unbound() {
    let ui = UiRuntime::new("en_us", 800., 600.).unwrap();
    run(
        &ui,
        r#"
        local slot=UIParent:CreateWidget('slot','unboundPetAction','UIParent')
        assert(slot:GetHotKey()=='')
        slot:EstablishSlot(ISLOT_PET_ACTION,1)
        assert(slot:GetHotKey()=='')
        AAUI={SlotHotKey=function() return nil end}
        assert(slot:GetHotKey()=='')
        AAUI.SlotHotKey=function(ty,idx)
            assert(ty==ISLOT_PET_ACTION and idx==1)
            return 'CTRL-F1'
        end
        assert(slot:GetHotKey()=='CTRL-F1')
    "#,
    );
}

#[test]
fn one_spawn_event_creates_both_unchanged_original_pet_frame_and_action_bar() {
    let mut ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    fake::install(&mut ui, "en_us", &fake::default_worlds());
    fake::install_ingame(&mut ui);
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    run(
        &ui,
        r#"
        assert(petFrame==nil and petBar==nil)
        -- Game responses only: both original SpawnPet closures and constructors stay intact.
        X2Mate.IsPlayerPetExists=function() return true end
        X2Mate.IsPlayerMounting=function() return false end
        X2Mate.PetState=function() return 'protective' end
        X2Mate.GetNumMountSkills=function() return 0 end
        X2Mate.IsAttackablePet=function() return true end
        X2Mate.IsMountablePet=function() return false end
        X2Mate.HavePassenger=function() return false end
        X2.GetPetExpToNextLevel=function() return 25,25,0,100 end
        local answers={UnitName='Stormmane',UnitLevel=30,UnitHealth=720,
            UnitMaxHealth=1000,UnitMana=260,UnitMaxMana=400,GetUnitId='201',
            GetUnitType='mate',GetUnitTypeString='mate',
            GetUnitInfoById={owner_name='Pet fixture'}}
        for name,answer in pairs(answers) do
            local original=X2Unit[name]
            X2Unit[name]=function(self,unit,...)
                if unit=='playerpet' or unit=='201' then return answer end
                return original(self,unit,...)
            end
        end
        assert(ADDON:GetContent(UIC_PLAYER_UNITFRAME))
    "#,
    );
    lua::fire_ui_event(&ui.lua, "SPAWN_PET", vec![]);
    run(
        &ui,
        r#"
        assert(petFrame and petFrame:IsVisible(),'original pet frame missing')
        assert(petBar and petBar:IsVisible(),'original pet bar missing')
        local width,height=petBar:GetExtent()
        assert(width==250 and height==105,'original pet bar constructor incomplete')
        assert(petFrame.target=='playerpet' and petFrame.name:GetText()=='Stormmane')
        assert(petBar.stateBar.selected==PET_STATE_PROTECTIVE)
        firstFrame,firstBar=petFrame,petBar
    "#,
    );
    lua::fire_ui_event(&ui.lua, "DISMISS_PET", vec![]);
    lua::tick(&ui.lua, 16.);
    run(&ui, "assert(petFrame==nil and petBar==nil)");
    lua::fire_ui_event(&ui.lua, "SPAWN_PET", vec![]);
    run(
        &ui,
        r#"
        assert(petFrame and petFrame:IsVisible() and petFrame~=firstFrame)
        assert(petBar and petBar:IsVisible() and petBar~=firstBar)
    "#,
    );
}
