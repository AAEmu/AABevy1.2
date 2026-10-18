//! Original equipped-item cooldown buttons; unattended aa-dev capture.

#[path = "fake/constant_cooldown.rs"]
mod constant_cooldown;
#[path = "fake/mod.rs"]
mod fake;

use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: native equipment cooldown"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            constant_cooldown::install(ui);
            fake::report(ui, "native_constant_cooldown");
        }))
        .add_systems(Startup, fake::physical_pixels)
        .add_systems(Update, fake::dump_tree)
        .run();
}
