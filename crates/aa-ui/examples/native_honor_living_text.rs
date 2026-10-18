//! Unmodified original character points and terminator-less quest vocation reward.
#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;
use aa_ui::{UiRuntime, lua};
use bevy::prelude::*;

pub fn install(ui: &mut UiRuntime) {
    fake::install(ui, "en_us", &fake::default_worlds());
    fake::install_ingame(ui);
    ui.exec("@points_fixture",r#"
        fixtureHonor='001234';fixtureLiving='000567'
        local old=X2Player.GetGamePoints
        X2Player.GetGamePoints=function(self)
            local info=old(self);info.honorPointStr=fixtureHonor;info.livingPointStr=fixtureLiving;return info
        end
    "#).unwrap();
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    aa_ui::content::hotkey(&ui.lua, "toggle_character").unwrap();
    aa_ui::content::hotkey(&ui.lua, "toggle_quest").unwrap();
    lua::tick(&ui.lua, 600.);
    for (alias, name) in [
        ("originalHonor", "character.window.honorPoint"),
        ("originalLiving", "character.window.livingPoint"),
        ("honorReward", "rewardHonorPoint"),
        ("livingReward", "rewardLivingPoint"),
    ] {
        let id = lua::ui(&ui.lua)
            .tree
            .find(name)
            .expect("original point control");
        ui.lua
            .globals()
            .set(alias, lua::object_table(&ui.lua, id).unwrap().unwrap())
            .unwrap();
    }
    ui.exec("@original_quest_reward",r#"
        honorReward:UpdateValue(1234);livingReward:UpdateValue(567)
        assert(originalHonor:GetText()=='|h001234;' and originalLiving:GetText()=='|l000567;')
        assert(string.find(livingReward:GetText(),'|l567',1,true) and not string.find(livingReward:GetText(),';',1,true))
    "#).unwrap();
    aa_ui::content::hotkey(&ui.lua, "toggle_quest").unwrap();
    lua::tick(&ui.lua, 600.);
}
fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: native honor and vocation text"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            install(ui);
            fake::report(ui, "native_honor_living_text");
        }))
        .add_systems(Startup, fake::physical_pixels)
        .run();
}
