//! Original equipped-item button and cooldown handler with explicit fixture timer samples.

use aa_ui::UiRuntime;

pub fn install(ui: &mut UiRuntime) {
    super::fake::install(ui, "en_us", &super::fake::default_worlds());
    super::fake::install_ingame(ui);
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    ui.exec(
        "@native_constant_cooldown",
        r#"
        nativeCooldownSamples={{8000,12000},{3000,12000},{0,12000}}
        AAUI.SkillCooldown=function(rawId)
            local sample=nativeCooldownSamples[rawId]
            if sample then return sample[1],sample[2] end
        end
        nativeCooldownButtons={}
        for i=1,3 do
            local button=CreateCoolDownActionButton('nativeCooldown'..i,UIParent,'constant')
            button:SetExtent(ICON_SIZE.DEFAULT,ICON_SIZE.DEFAULT)
            button:AddAnchor('CENTER',UIParent,(i-2)*72,0)
            button:SetItemIcon(4045,2)
            AddOnUpdateConstantCooldown(button,'action')
            button:SetSkill(i)
            button:Show(true)
            nativeCooldownButtons[i]=button
        end
    "#,
    )
    .unwrap();
}
