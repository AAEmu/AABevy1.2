//! Original casting bars; aa-dev captures the filled image and anchored flare, then exits.

#[path = "fake/castingbar.rs"]
mod castingbar;
#[path = "fake/mod.rs"]
mod fake;

use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: original casting-bar anchors"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            castingbar::install(ui);
            fake::report(ui, "native_castingbar");
        }))
        .add_systems(Startup, fake::physical_pixels)
        .add_systems(Update, fake::dump_tree)
        .run();
}
