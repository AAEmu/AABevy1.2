//! Original full row tooltip, from native listbox notification through original Lua.
//! `cargo run -p aa-ui --example native_listbox_tooltip -- --hover 430 356 1 --shot shots/aa-ui/native-listbox-tooltip.png --shot-after 4.4`
#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;
use bevy::prelude::*;
fn main() {
    App::new()
        .add_plugins(aa_dev::default_plugins("aa-ui: native listbox tooltip"))
        .add_plugins(aa_dev::DevPlugin)
        .add_plugins(aa_ui::UiPlugin{config:Some(fake::config())})
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            fake::install(ui,"en_us",&fake::default_worlds());fake::install_ingame(ui);
            ui.run_stage(aa_ui::Stage::World);fake::enter_world(ui);
            ui.exec("@native_tooltip_rows",include_str!("fixtures/listbox_tooltip.lua")).unwrap();
            fake::report(ui,"native_listbox_tooltip");
        }))
        .add_systems(Startup,fake::physical_pixels)
        .add_systems(Update,fake::scripted_input)
        .run();
}
