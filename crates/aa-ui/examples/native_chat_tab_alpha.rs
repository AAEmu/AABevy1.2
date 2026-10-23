//! Original chat art with the native stored tab-alpha pair and an idle pointer.
#[path = "native_chat_split.rs"]
#[allow(dead_code)]
mod original;
use original::fake;

use aa_ui::{UiRuntime, lua};
use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: native stored chat tab alpha"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            original::install(ui);
            ui.exec(
                "@tab_alpha_capture",
                "splitSource:SetTabButtonAlpha(.15,.85)",
            )
            .unwrap();
            fake::report(ui, "native_chat_tab_alpha");
        }))
        .add_systems(Startup, fake::physical_pixels)
        .add_systems(Update, point.after(aa_ui::UiInput).before(aa_ui::UiFrame))
        .run();
}

fn point(ui: Option<NonSend<UiRuntime>>, mut pointer: ResMut<aa_ui::UiVirtualPointer>) {
    let Some(ui) = ui else { return };
    let u = lua::ui(&ui.lua);
    let window = u.tree.of_class("chatwindow")[0];
    let r = u.tree.rect(window);
    pointer.pos = Some(if std::env::args().any(|a| a == "--hover") {
        Vec2::new((r.x + r.w / 2.) * u.ui_scale, (r.y + r.h / 2.) * u.ui_scale)
    } else {
        Vec2::new(1100., 40.)
    });
}
