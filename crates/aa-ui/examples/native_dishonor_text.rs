//! Original character panel's unmodified dishonor formatter and received-style refresh.
#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;
use aa_ui::{UiRuntime, lua, tree::Id};
use bevy::prelude::*;

pub fn install(ui: &mut UiRuntime) -> Id {
    fake::install(ui, "en_us", &fake::default_worlds());
    fake::install_ingame(ui);
    ui.exec(
        "@dishonor_receipt_fixture",
        r#"
        fixtureCrimeRecord=1234
        X2Player.GetCrimeInfo=function() return {crimePoint=80,crimeRecord=fixtureCrimeRecord} end
    "#,
    )
    .unwrap();
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    aa_ui::content::hotkey(&ui.lua, "toggle_character").unwrap();
    lua::tick(&ui.lua, 600.);
    let id = lua::ui(&ui.lua)
        .tree
        .find("character.window.DishonorPoint")
        .expect("original dishonor textbox");
    ui.lua
        .globals()
        .set(
            "originalDishonor",
            lua::object_table(&ui.lua, id).unwrap().unwrap(),
        )
        .unwrap();
    ui.exec(
        "@original_dishonor_verify",
        "assert(originalDishonor:GetText()=='|d1234')",
    )
    .unwrap();
    id
}
fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: original dishonor text"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            install(ui);
            fake::report(ui, "native_dishonor_text");
        }))
        .add_systems(Startup, fake::physical_pixels)
        .run();
}
