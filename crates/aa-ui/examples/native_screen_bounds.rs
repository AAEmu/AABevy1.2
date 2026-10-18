//! Original pet/tooltip overflow recovery, with aa-dev screenshot and automatic exit.

#[path = "fake/mod.rs"]
mod fake;
#[path = "fake/screen_bounds.rs"]
mod screen_bounds;

use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: original screen bounds recovery"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            screen_bounds::install(ui);
            fake::report(ui, "native_screen_bounds");
        }))
        .add_systems(Startup, fake::physical_pixels)
        .add_systems(Update, fake::dump_tree)
        .run();
}
