//! Original Options resolution dropdown, opened with the real pointer dispatch.
//! `cargo run -p aa-ui --example native_listbox -- --shot shots/aa-ui/native-listbox.png --shot-after 4.4`

#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins(aa_dev::default_plugins("aa-ui: original Options dropdown"))
        .add_plugins(aa_dev::DevPlugin)
        .add_plugins(aa_ui::UiPlugin { config: Some(fake::config()) })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            fake::install(ui, "en_us", &fake::default_worlds());
            fake::install_ingame(ui);
            ui.run_stage(aa_ui::Stage::World);
            fake::enter_world(ui);
            ui.exec("@original_options_dropdown", r#"
                options=CreateOptionFrame(); options:Show(true); options:Init(); options:SelectSubFrame(1)
                screen=options.contentWindow.pageWindow.subFrames[1]
                combo=screen.content.optionFrames[1].optionControl
            "#).unwrap();
            let value: mlua::Value = ui.lua.load("combo").eval().unwrap();
            let id = aa_ui::lua::value_id(&ui.lua, &value).unwrap();
            let r = aa_ui::lua::ui(&ui.lua).tree.rect(id);
            let p = (r.x + 20.0, r.y + r.h * 0.5);
            let mut mouse = Default::default();
            aa_ui::input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.0);
            aa_ui::input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.0);
            ui.exec("@opened_dropdown", "dropdown=combo:GetAttachedWidget(); assert(dropdown:IsVisible() and dropdown.list:ItemCount()==4)").unwrap();
            let p: (f32,f32) = ui.lua.load("local l=dropdown.list; local x,y=l:GetOffset(); local a,b=l:GetInset(); return x+a+14,y+b+l:GetHeight()*1.5").eval().unwrap();
            aa_ui::input::update(&ui.lua, &mut mouse, Some(p), false, false, false, 0.0);
            fake::report(ui,"native_listbox");
        }))
        .add_systems(Startup,fake::physical_pixels)
        .add_systems(Update,fake::scripted_input)
        .run();
}
