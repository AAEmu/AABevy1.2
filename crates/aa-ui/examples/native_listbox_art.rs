//! Original Achievement separators/state art and original slider-list icon forwarding.
//! `cargo run -p aa-ui --example native_listbox_art -- --shot shots/aa-ui/native-listbox-art.png --shot-after 4.4`
#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;
use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins(aa_dev::default_plugins("aa-ui: native listbox artwork"))
        .add_plugins(aa_dev::DevPlugin)
        .add_plugins(aa_ui::UiPlugin { config: Some(fake::config()) })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            fake::install(ui, "en_us", &fake::default_worlds());
            fake::install_ingame(ui);
            ui.run_stage(aa_ui::Stage::World); fake::enter_world(ui);
            ui.exec("@listbox_art", include_str!("fixtures/listbox_art.lua")).unwrap();
            let mut mouse = Default::default();
            for (name,row) in [("tree",3),("icons.list",0)] {
                let p=ui.lua.load(format!("local x,y={name}:GetOffset();local l,t={name}:GetInset();return x+l+12,y+t+{name}:GetHeight()*({row}+.5)")).eval().unwrap();
                aa_ui::input::update(&ui.lua,&mut mouse,Some(p),true,false,false,0.);
                aa_ui::input::update(&ui.lua,&mut mouse,Some(p),false,true,false,0.);
            }
            ui.exec("@listbox_art_ready","assert(tree:IsItemOpened(3) and counts:ItemCount()==12 and icons.list:GetSelectedValue()==11)").unwrap();
            let p=ui.lua.load("local x,y=icons.list:GetOffset();local l,t=icons.list:GetInset();return x+l+12,y+t+icons.list:GetHeight()*1.5").eval().unwrap();
            aa_ui::input::update(&ui.lua,&mut mouse,Some(p),false,false,false,0.);
            fake::report(ui,"native_listbox_art");
        }))
        .add_systems(Startup,fake::physical_pixels)
        .add_systems(Update,fake::scripted_input)
        .run();
}
