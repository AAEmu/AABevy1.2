//! Original pet bar and tooltip, deliberately positioned beyond the screen edges.

use aa_ui::{UiRuntime, lua};

pub fn install(ui: &mut UiRuntime) {
    super::fake::install(ui, "en_us", &super::fake::default_worlds());
    super::fake::install_ingame(ui);
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    ui.exec(
        "@screen_bounds_pet",
        r#"
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
    "#,
    )
    .unwrap();
    lua::fire_ui_event(&ui.lua, "SPAWN_PET", vec![]);
    ui.exec(
        "@screen_bounds_recovery",
        r#"
        assert(petBar and petFrame and petBar:IsVisible())
        local w,h=petBar:GetExtent();assert(w==250 and h==105)
        petBar:RemoveAllAnchors()
        petBar:AddAnchor('TOPLEFT',UIParent,1880,1040)
        assert(petBar:CheckOutOfScreen())
        -- Unchanged pet helper removes anchors before requesting correction.
        petBar:OnScale()
        assert(not petBar:CheckOutOfScreen())
        overflowTarget=UIParent:CreateWidget('emptywidget','overflowTarget','UIParent')
        overflowTarget:SetExtent(20,20)
        overflowTarget:AddAnchor('TOPLEFT',UIParent,2000,20)
        SetTooltip('Recovered original tooltip',overflowTarget)
        assert(not tooltip.window:CheckOutOfScreen())
    "#,
    )
    .unwrap();
}
