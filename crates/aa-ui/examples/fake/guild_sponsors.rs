//! Original guild creation/FillAllianceInfo; game APIs remain the existing example fixtures.
use aa_ui::UiRuntime;

pub fn install(ui: &mut UiRuntime, alliance: u32) {
    super::fake::install(ui, "en_us", &super::fake::default_worlds());
    super::fake::install_ingame(ui);
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    ui.lua.globals().set("sponsorAlliance", alliance).unwrap();
    ui.exec(
        "@original_guild_sponsors",
        r#"
        ShowCreateExpedition(true,sponsorAlliance)
        assert(expedition:IsVisible())
        sponsorTab=expedition.selectSponsor.tab
        assert(#sponsorTab.window==7)
        assert(sponsorTab:GetActivateTabCount()==(sponsorAlliance==148 and 6 or 5))
        assert(sponsorTab:GetSelectedTab()==1)
    "#,
    )
    .unwrap();
    aa_ui::lua::tick(&ui.lua, 0.0);
}
