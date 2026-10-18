//! Original achievement list constructor, data/layout callbacks and physical tracing controls.
#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::{UiRuntime, lua};
use bevy::prelude::*;

pub fn install(ui: &mut UiRuntime) {
    fake::install(ui, "en_us", &fake::default_worlds());
    fake::install_ingame(ui);
    ui.exec("@achievement_list_data", r#"
        local names={'An Adventurer Begins','Exploring Nuia','A Helping Hand','Crafting a Future','Across the Sea','A New Companion'}
        local summaries={'Complete your first adventure','Visit the regions of Nuia','Help your fellow adventurers','Craft useful equipment','Discover distant shores','Travel with a trusted companion'}
        nativeAchievementTracing={}
        X2Achievement.GetAchievementList=function() return {1,2,3,4,5,6} end
        X2Achievement.GetAchievementInfo=function(self,id)
            return {name=names[id],summary=summaries[id],iconPath='ui/icon/icon_skill_basic01.dds',
                completeNum=10,current=id,highRank=false,complete=false,reward=0}
        end
        X2Achievement.GetTracingAchievementCount=function() local n=0;for _ in pairs(nativeAchievementTracing) do n=n+1 end;return n,6 end
        X2Achievement.IsTracingAchievement=function(self,id) return nativeAchievementTracing[id]==true end
        X2Achievement.AddTracingAchievement=function(self,id) nativeAchievementTracing[id]=true;return true end
        X2Achievement.RemoveTracingAchievement=function(self,id) nativeAchievementTracing[id]=nil;return true end
    "#).unwrap();
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    assert!(aa_ui::content::hotkey(&ui.lua, "toggle_achievement").unwrap());
    let host = lua::ui(&ui.lua).tree.find("achievementWnd").unwrap();
    ui.lua
        .globals()
        .set(
            "nativeAchievementHost",
            lua::object_table(&ui.lua, host).unwrap().unwrap(),
        )
        .unwrap();
    ui.exec(
        "@original_achievement_list",
        r#"
        nativeAchievementHost:ShowAchivementListByCategory(1,1)
        nativeAchievementBody=nativeAchievementHost.contentWnd.listWnd.bodySectionWnd
        assert(nativeAchievementHost:IsVisible())
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 600.);
    lua::flush(&ui.lua);
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: original achievement list"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            install(ui);
            fake::report(ui, "native_achievement_list");
        }))
        .add_systems(Startup, fake::physical_pixels)
        .run();
}
