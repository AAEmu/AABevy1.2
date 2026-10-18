//! Unchanged original casting-bar view at three received progress values.

use aa_ui::UiRuntime;

pub fn install(ui: &mut UiRuntime) {
    super::fake::install(ui, "en_us", &super::fake::default_worlds());
    super::fake::install_ingame(ui);
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    ui.exec(
        "@native_castingbar_fixture",
        r#"
        nativeCastingBars={}
        for i,value in ipairs({25,50,75}) do
            local bar=SetViewOfCastingBar('nativeCasting'..i,UIParent)
            bar:AddAnchor('CENTER',UIParent,0,(i-2)*80)
            bar.statusBar:SetMinMaxValues(0,100)
            bar.statusBar:SetValue(value)
            bar.text:SetText(tostring(value)..'%')
            bar:StartAnmation(0.7)
            bar:Show(true)
            nativeCastingBars[i]=bar
        end
    "#,
    )
    .unwrap();
}
