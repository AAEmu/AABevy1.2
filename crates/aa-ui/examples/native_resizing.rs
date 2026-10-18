//! Original quest tracker resizing; aa-dev captures the guide and closes automatically.

#[path = "fake/mod.rs"]
mod fake;
#[path = "fake/resizing.rs"]
mod resizing;

use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: native quest resizing"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            resizing::install(ui);
            let pointer = resizing::drag_bottom(ui);
            aa_ui::lua::ui(&ui.lua).cursor = pointer;
            fake::report(ui, "native_resizing");
        }))
        .add_systems(Startup, fake::physical_pixels)
        .add_systems(Update, pin_pointer.before(aa_ui::UiFrame))
        .add_systems(Update, fake::dump_tree)
        .run();
}

fn pin_pointer(
    runtime: Option<NonSend<aa_ui::UiRuntime>>,
    mut pointer: ResMut<aa_ui::UiVirtualPointer>,
    mut done: Local<bool>,
) {
    if !*done && let Some(runtime) = runtime {
        let u = aa_ui::lua::ui(&runtime.lua);
        let id = u.tree.find("questContext.notifierBack").unwrap();
        let rect = u.tree.rect(id);
        pointer.pos = Some(Vec2::new(
            (rect.x + rect.w * 0.5) * u.ui_scale,
            (rect.bottom() - 5.0) * u.ui_scale,
        ));
        *done = true;
    }
}
