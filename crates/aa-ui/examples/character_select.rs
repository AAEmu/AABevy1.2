//! Character select screen (2D layer only) from the client's original Lua UI with fake data.
//!
//! ```text
//! cargo run -p aa-ui --example character_select -- --shot shots/aa-ui/character_select.png
//! cargo run -p aa-ui --example character_select -- --characters 0 --dump data/extract/aa-ui/cs_tree.json --exit-after 4
//! ```
//!
//! `--characters 0|1` matches `reference/character_select/0_characters` / `1_character`.

#[path = "fake/mod.rs"]
mod fake;

use bevy::prelude::*;

fn main() {
    let locale = "en_us";
    App::new()
        .add_plugins(aa_dev::default_plugins("aa-ui: character select"))
        .add_plugins(aa_dev::DevPlugin)
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(move |ui| {
            let worlds = fake::worlds_from_args();
            let chars = fake::characters_from_args();
            fake::install_with(ui, locale, &worlds, &chars);
            ui.run_stage(aa_ui::Stage::CharacterSelect);
            ui.fire_event("SHOW_CHARACTER_SELECT_WINDOW", true);
            fake::report(ui, "character_select");
        }))
        .add_systems(Startup, fake::physical_pixels)
        .add_systems(Update, (fake::dump_tree, fake::scripted_input))
        .run();
}
