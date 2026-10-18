//! Original crafting group/category handlers and authored tree images; no server traffic.
//! `cargo run -p aa-ui --example native_crafting_tree -- --shot shots/aa-ui/native-crafting-tree.png --shot-after 4.4`
#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;
use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins(aa_dev::default_plugins("aa-ui: original crafting tree"))
        .add_plugins(aa_dev::DevPlugin)
        .add_plugins(aa_ui::UiPlugin { config:Some(fake::config()) })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            fake::install(ui,"en_us",&fake::default_worlds());fake::install_ingame(ui);
            ui.exec("@craft_data",include_str!("fixtures/crafting_tree.lua")).unwrap();
            ui.exec("@craft_count","CRAFT_TREE_ROWS=4").unwrap();
            ui.run_stage(aa_ui::Stage::World);fake::enter_world(ui);
            ui.exec("@original_crafting_tree",r#"
                craft=CreateCraftingWindowRenewal('nativeCraftTree','UIParent')
                craft:AddAnchor('CENTER',UIParent,0,0);craft:FillCraft(false);craft:Show(true)
                group=craft.abilityGroupPage.buttons[1]
                list=craft.detailPage.categoryList.content
            "#).unwrap();
            let group:mlua::Value=ui.lua.load("group").eval().unwrap();
            let id=aa_ui::lua::value_id(&ui.lua,&group).unwrap();
            let r=aa_ui::lua::ui(&ui.lua).tree.rect(id);
            let mut mouse=Default::default();
            let mut click=|p| {
                aa_ui::input::update(&ui.lua,&mut mouse,Some(p),true,false,false,0.0);
                aa_ui::input::update(&ui.lua,&mut mouse,Some(p),false,true,false,0.0);
            };
            click((r.x+r.w*0.5,r.y+r.h*0.5));
            for row in [0,5,1] {
                let p:(f32,f32)=ui.lua.load(format!("local x,y=list:GetOffset();local l,t=list:GetInset();return x+l+40,y+t+list:GetHeight()*({row}+.5)")).eval().unwrap();
                click(p);
            }
            ui.exec("@craft_tree_ready","assert(list:GetOpenedItemCount()==11 and list:GetSelectedValue()==10101 and CRAFT_TREE_SELECTED[#CRAFT_TREE_SELECTED]==10101)").unwrap();
            fake::report(ui,"native_crafting_tree");
        }))
        .add_systems(Startup,fake::physical_pixels)
        .add_systems(Update,fake::scripted_input)
        .run();
}
