//! Original chat tabs use cached fitting prefixes while their tooltip retains the full name.

#[path = "fake/auto_clip.rs"]
mod auto_clip;
#[path = "fake/mod.rs"]
mod fake;

use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: native chat tab clipping"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            auto_clip::install(ui);
            fake::report(ui, "native_auto_clip");
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
    let Some(tab) = u
        .tree
        .widget(window)
        .and_then(|w| w.chat.as_ref())
        .and_then(|c| c.tabs.first())
    else {
        return;
    };
    let r = u.tree.rect(tab.button);
    pointer.pos = Some(Vec2::new(
        (r.x + r.w * 0.5) * u.ui_scale,
        (r.y + r.h * 0.5) * u.ui_scale,
    ));
}
