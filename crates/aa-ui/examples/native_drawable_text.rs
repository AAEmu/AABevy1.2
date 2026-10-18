//! Original action-slot drawable alignment/color/decoration; unattended aa-dev capture.

#[path = "fake/drawable_text.rs"]
mod drawable_text;
#[path = "fake/mod.rs"]
mod fake;

use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: native drawable text"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            drawable_text::install(ui);
            fake::report(ui, "native_drawable_text");
        }))
        .add_systems(Startup, fake::physical_pixels)
        .add_systems(Update, fake::dump_tree)
        .run();
}
