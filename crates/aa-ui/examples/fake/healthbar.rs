//! The registered original player frame; its unchanged SetHp method owns both gauges.

use aa_ui::UiRuntime;

pub fn install(ui: &mut UiRuntime) {
    super::fake::install(ui, "en_us", &super::fake::default_worlds());
    super::fake::install_ingame(ui);
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    ui.exec(
        "@native_healthbar_fixture",
        r#"
        nativeHealthFrame=ADDON:GetContent(UIC_PLAYER_UNITFRAME)
        assert(nativeHealthFrame and nativeHealthFrame.target=='player')
        FAKE.player.hp=100;FAKE.player.maxHp=100
        nativeHealthFrame.unableGageAni=true
        nativeHealthFrame:SetHp(100,100)
    "#,
    )
    .unwrap();
}

pub fn lose_health(ui: &UiRuntime) {
    ui.exec(
        "@native_healthbar_loss",
        "FAKE.player.hp=40;nativeHealthFrame:SetHp(40,100)",
    )
    .unwrap();
}
