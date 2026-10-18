//! Original chat lock drawable after an actual pointer drag attempt.
#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;
#[path = "native_chat_alpha.rs"]
#[allow(dead_code)]
mod original;
use aa_ui::{
    UiRuntime,
    input::{self, MouseButtons, MouseState},
    lua,
    tree::Id,
};
use bevy::prelude::*;

pub fn install(ui: &mut UiRuntime) -> Id {
    let window = original::install(ui);
    lua::chat::set_allow_tab_switch(&ui.lua, window, false);
    ui.exec("@lock_example",r#"
        nativeAlphaChat:UseResizing(false)
        X2Chat:DispatchChatMessage(2,'The chat window is locked. Tab selection and its menu remain available.')
        X2Chat:DispatchChatMessage(2,'Dragging a tab displays the original lock notification.')
    "#).unwrap();
    window
}

pub fn drag_attempt(ui: &UiRuntime, window: Id) {
    let (button, r) = {
        let u = lua::ui(&ui.lua);
        let b = u.tree.widget(window).unwrap().chat.as_ref().unwrap().tabs[0].button;
        (b, u.tree.rect(b))
    };
    let p = (r.x + r.w / 2., r.y + r.h / 2.);
    assert_eq!(input::pick(&ui.lua, p.0, p.1), Some(button));
    let mut mouse = MouseState::default();
    for (p, buttons) in [
        (
            p,
            MouseButtons {
                press: true,
                ..Default::default()
            },
        ),
        ((p.0 + 5., p.1), MouseButtons::default()),
        ((p.0 + 30., p.1 + 15.), MouseButtons::default()),
        (
            (p.0 + 30., p.1 + 15.),
            MouseButtons {
                release: true,
                ..Default::default()
            },
        ),
    ] {
        input::update_buttons(&ui.lua, &mut mouse, Some(p), buttons, 0.);
    }
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: native chat locking"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            install(ui);
            fake::report(ui, "native_chat_lock");
        }))
        .add_systems(Startup, fake::physical_pixels)
        .add_systems(Update, attempt.after(aa_ui::UiInput).before(aa_ui::UiFrame))
        .run();
}

fn attempt(ui: Option<NonSend<UiRuntime>>, time: Res<Time>, mut last: Local<f32>) {
    let Some(ui) = ui else { return };
    if time.elapsed_secs() - *last < 2.0 {
        return;
    }
    *last = time.elapsed_secs();
    let window = lua::ui(&ui.lua).tree.of_class("chatwindow")[0];
    drag_attempt(&ui, window);
}
