//! The in-game HUD from the client's original Lua UI (`StartScreenWorld`: base libs,
//! `ui_management`, every content, the root scripts) with a fake game API.
//!
//! ```text
//! cargo run -p aa-ui --example ingame_hud -- --shot shots/aa-ui/ingame_hud.png
//! cargo run -p aa-ui --example ingame_hud -- --dump data/extract/aa-ui/check/ingame.json --dump-after 4 --exit-after 5
//! ```
//!
//! The scenario is `reference/ingame/spawn`: a level 1 Nuian (`Refone`) just spawned in
//! Solzreed Peninsula. `examples/fake/fake_ingame.lua` is the game API the HUD reads; the app
//! binds real data with the same names.

#[path = "fake/mod.rs"]
mod fake;

use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins(aa_dev::default_plugins("aa-ui: in-game HUD"))
        .add_plugins(aa_dev::DevPlugin)
        .add_plugins(aa_ui::UiPlugin { config: Some(fake::config()) })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            let worlds = fake::worlds_from_args();
            let chars = fake::characters_from_args();
            fake::install_with(ui, "en_us", &worlds, &chars);
            fake::install_ingame(ui);
            ui.run_stage(aa_ui::Stage::World);
            fake::enter_world(ui);
            fake::report(ui, "ingame_hud");
        }))
        .add_systems(Startup, fake::physical_pixels)
        .add_systems(Update, (fake::dump_tree, fake::scripted_input))
        .run();
}
