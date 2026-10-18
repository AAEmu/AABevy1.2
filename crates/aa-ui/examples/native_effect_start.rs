//! Original low-health warning recovery and action-slot effect controls.
#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;
use aa_ui::{UiRuntime, lua};
use bevy::prelude::*;

pub fn install(ui: &mut UiRuntime) {
    fake::install(ui, "en_us", &fake::default_worlds());
    fake::install_ingame(ui);
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    lua::tick(&ui.lua, 600.);
    let player = lua::ui(&ui.lua).tree.find("playerFrame").unwrap();
    ui.lua
        .globals()
        .raw_set(
            "effectPlayerFixture",
            lua::object_table(&ui.lua, player).unwrap().unwrap(),
        )
        .unwrap();
    ui.exec(
        "@native_effect_restart",
        r#"
        PlayerLifeAlertEffectVisibleHandler(true)
        FAKE.player.hp=FAKE.player.maxHp;effectPlayerFixture:UpdateAll()
        FAKE.player.hp=FAKE.player.maxHp/5;effectPlayerFixture:UpdateAll()
        FAKE.player.hp=FAKE.player.maxHp;effectPlayerFixture:UpdateAll()
        FAKE.player.hp=FAKE.player.maxHp/5;effectPlayerFixture:UpdateAll()
        effectStartPanel=CreateWindow('nativeEffectStartPanel',UIParent)
        effectStartPanel:SetExtent(520,180);effectStartPanel:AddAnchor('CENTER',UIParent,0,0)
        effectStartPanel:SetTitle('Original HUD effect lifecycle');effectStartPanel:Show(true)
        local labels={'Auto attack active','Combo stopped','Toggle restarted'}
        effectStartSlots={}
        for i=1,3 do
            local slot=CreateActionSlot(effectStartPanel,'effectStartSlot'..i,ISLOT_ACTION,i)
            slot:AddAnchor('TOPLEFT',effectStartPanel,65+(i-1)*170,65);slot:Show(true)
            effectStartSlots[i]=slot
            local label=effectStartPanel:CreateChildWidget('label','slotCaption',i,true)
            label:SetExtent(165,24);label:AddAnchor('TOP',slot,'BOTTOM',0,8)
            label:SetText(labels[i]);label:Show(true);ApplyTextColor(label,FONT_COLOR.DEFAULT)
        end
        effectStartSlots[1]:Anim_auto_attack_effect(true)
        effectStartSlots[2]:Anim_combo_aciton_for_me_effect(true)
        effectStartSlots[2]:Anim_combo_aciton_for_me_effect(false)
        effectStartSlots[3]:AnimToggleSkill(true);effectStartSlots[3]:AnimToggleSkill(false)
        effectStartSlots[3]:AnimToggleSkill(true)
    "#,
    )
    .unwrap();
    if std::env::args().any(|a| a == "--healthy") {
        ui.exec(
            "@healthy_effect_control",
            r#"
            FAKE.player.hp=FAKE.player.maxHp;effectPlayerFixture:UpdateAll()
            effectStartPanel:SetTitle('Recovered: health warning stopped')
        "#,
        )
        .unwrap();
    }
    lua::tick(&ui.lua, 0.);
    fake::report(ui, "native_effect_start");
}
fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: native effect restart"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(install))
        .add_systems(Startup, fake::physical_pixels)
        .run();
}
