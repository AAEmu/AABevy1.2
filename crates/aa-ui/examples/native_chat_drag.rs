//! Matched original-script drag-hover capture; no replacement chat UI.
#[path = "native_chat_split.rs"]
#[allow(dead_code)]
mod original;

use original::fake;

use aa_ui::{UiRuntime, input, lua};
use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: original chat drag preview"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            original::install(ui);
        }))
        .add_systems(Startup, fake::physical_pixels)
        .add_systems(Update, hover.after(aa_ui::UiInput).before(aa_ui::UiFrame))
        .run();
}

fn hover(
    ui: Option<NonSend<UiRuntime>>,
    time: Res<Time>,
    mut done: Local<bool>,
    mut pointer: ResMut<aa_ui::UiVirtualPointer>,
) {
    let Some(ui) = ui else { return };
    // Freeze both captures at the same physical pointer and original script geometry.
    pointer.pos = Some(Vec2::new(130., 235.));
    if *done || time.elapsed_secs() < 0.5 {
        return;
    }
    *done = true;
    let button = {
        let u = lua::ui(&ui.lua);
        let window = u.tree.of_class("chatwindow")[0];
        u.tree.widget(window).unwrap().chat.as_ref().unwrap().tabs[1].button
    };
    let r = lua::ui(&ui.lua).tree.rect(button);
    let p = (r.x + r.w / 2., r.y + r.h / 2.);
    let mut mouse = input::MouseState::default();
    input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.);
    input::update(
        &ui.lua,
        &mut mouse,
        Some((p.0 + 5., p.1)),
        false,
        false,
        false,
        0.,
    );
    input::update(
        &ui.lua,
        &mut mouse,
        Some((130., 235.)),
        false,
        false,
        false,
        0.,
    );
    ui.exec("@drag_tooltip", "HideTooltip()").unwrap();
    lua::tick(&ui.lua, 0.);
    let u = lua::ui(&ui.lua);
    println!(
        "native_chat_drag: windows={} retained_button={} parent={:?}",
        u.tree.of_class("chatwindow").len(),
        button,
        u.tree.get(button).parent
    );
}
