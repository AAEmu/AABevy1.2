//! Original fixed and auto chat tabs, including native clipping of long labels.

#[path = "fake/auto_clip.rs"]
mod auto_clip;
#[path = "fake/chat_tab_layout.rs"]
mod chat_tab_layout;
#[path = "fake/mod.rs"]
mod fake;

use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: native fixed and auto chat tabs"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            chat_tab_layout::install(ui);
            fake::report(ui, "native_chat_tab_layout");
        }))
        .add_systems(Startup, fake::physical_pixels)
        .add_systems(Update, fake::dump_tree)
        .add_systems(
            Update,
            hold_hover.after(aa_ui::UiInput).before(aa_ui::UiFrame),
        )
        .run();
}

fn hold_hover(ui: Option<NonSend<aa_ui::UiRuntime>>, mut pointer: ResMut<aa_ui::UiVirtualPointer>) {
    let Some(ui) = ui else { return };
    let u = aa_ui::lua::ui(&ui.lua);
    let Some(window) = u.tree.of_class("chatwindow").first().copied() else {
        return;
    };
    let r = u.tree.rect(window);
    pointer.pos = Some(Vec2::new(
        (r.x + 20.) * u.ui_scale,
        (r.y + 50.) * u.ui_scale,
    ));
}
