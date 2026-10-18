//! Original Achievement image templates and native slider-list fallback state images.
#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;
use bevy::prelude::*;
fn main() {
    App::new()
        .add_plugins(aa_dev::default_plugins("aa-ui: native listbox templates"))
        .add_plugins(aa_dev::DevPlugin)
        .add_plugins(aa_ui::UiPlugin { config: Some(fake::config()) })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            fake::install(ui,"en_us",&fake::default_worlds());fake::install_ingame(ui);
            ui.run_stage(aa_ui::Stage::World);fake::enter_world(ui);
            ui.exec("@native_listbox_templates",include_str!("fixtures/listbox_templates.lua")).unwrap();
            let p=ui.lua.load("local x,y=tree:GetOffset();local l,t=tree:GetInset();return x+l+12,y+t+tree:GetHeight()*3.5").eval().unwrap();
            let mut mouse=Default::default();
            aa_ui::input::update(&ui.lua,&mut mouse,Some(p),true,false,false,0.);
            aa_ui::input::update(&ui.lua,&mut mouse,Some(p),false,true,false,0.);
            ui.exec("@native_templates_ready","assert(tree:IsItemOpened(3) and counts:ItemCount()==12)").unwrap();
            fake::report(ui,"native_listbox_templates");
        }))
        .add_systems(Startup,fake::physical_pixels)
        .add_systems(Update,fake::scripted_input)
        .run();
}
