//! Character create screens (2D layer only) from the client's original Lua UI.
//!
//! ```text
//! cargo run -p aa-ui --example character_create -- --shot shots/aa-ui/character_create.png
//! cargo run -p aa-ui --example character_create -- --race elf --gender female --step 2 --shot shots/aa-ui/cc_step2.png
//! ```
//!
//! `--race nuian|elf|ferre|hariharan`, `--gender male|female`, `--step 1|2|3` reproduce
//! `reference/character_create/step<N>_<race>_<gender>` the way the capture did (button clicks
//! and `characterCreate:OnNextStep()` through Lua).

#[path = "fake/mod.rs"]
mod fake;

use bevy::prelude::*;

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn main() {
    let locale = "en_us";
    let race = arg("--race").unwrap_or_else(|| "nuian".into());
    let gender = arg("--gender").unwrap_or_else(|| "male".into());
    let step: u32 = arg("--step").and_then(|s| s.parse().ok()).unwrap_or(1);
    App::new()
        .add_plugins(aa_dev::default_plugins("aa-ui: character create"))
        .add_plugins(aa_dev::DevPlugin)
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(move |ui| {
            let worlds = fake::worlds_from_args();
            fake::install_with(ui, locale, &worlds, &[]);
            ui.run_stage(aa_ui::Stage::CharacterCreate);
            ui.fire_event("SHOW_CHARACTER_CREATE_WINDOW", true);
            // Race / gender buttons like the capture: raceBtn[1..4] = nuian, elf, ferre,
            // hariharan; genderBtn[1] male, [2] female.
            let ri = match race.as_str() {
                "elf" => 2,
                "ferre" => 3,
                "hariharan" => 4,
                _ => 1,
            };
            let gi = if gender == "female" { 2 } else { 1 };
            let code = format!(
                "step1.content.raceBtn[{ri}]:OnClick(\"LeftButton\")\n\
                 step1.content.rightPanel.genderBtn[{gi}]:OnClick(\"LeftButton\")"
            );
            if let Err(e) = ui.exec("@select race", &code) {
                warn!("character_create: {e}");
            }
            for _ in 1..step {
                if let Err(e) = ui.exec("@next step", "characterCreate:OnNextStep()") {
                    warn!("character_create: {e}");
                }
            }
            fake::report(ui, "character_create");
        }))
        .add_systems(Startup, fake::physical_pixels)
        .add_systems(Update, (fake::dump_tree, fake::scripted_input))
        .run();
}
