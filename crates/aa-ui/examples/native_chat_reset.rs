//! Original chat context popup followed by native all-window reset and recreation.
#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;
#[path = "native_chat_alpha.rs"]
#[allow(dead_code)]
mod original;
use aa_ui::{UiRuntime, input, lua, tree::Id};
use bevy::prelude::*;

pub fn install(ui: &mut UiRuntime) -> Id {
    let window = original::install(ui);
    ui.exec(
        "@reset_history",
        "X2Chat:DispatchChatMessage(2,'This old message must disappear after reset.')",
    )
    .unwrap();
    window
}

pub fn reset_and_recreate(ui: &UiRuntime) {
    let (windows, button) = {
        let u = lua::ui(&ui.lua);
        let windows = u.tree.of_class("chatwindow");
        let button = u
            .tree
            .widget(windows[0])
            .unwrap()
            .chat
            .as_ref()
            .unwrap()
            .tabs[0]
            .button;
        (windows, button)
    };
    let r = lua::ui(&ui.lua).tree.rect(button);
    let p = (r.x + r.w / 2., r.y + r.h / 2.);
    assert_eq!(input::pick(&ui.lua, p.0, p.1), Some(button));
    let mut mouse = input::MouseState::default();
    for release in [false, true] {
        input::update_buttons(
            &ui.lua,
            &mut mouse,
            Some(p),
            input::MouseButtons {
                right_press: !release,
                right_release: release,
                ..Default::default()
            },
            0.,
        );
        lua::tick(&ui.lua, 16.);
    }
    let popup = lua::ui(&ui.lua)
        .attachments
        .get(&button)
        .copied()
        .expect("original context popup");
    assert!(lua::ui(&ui.lua).tree.get(popup).shown);
    // The original popup handles the outside press that opens another control,
    // including the reset dialog. Destruction itself does not invent OnHide.
    input::update(&ui.lua, &mut mouse, Some((0., 0.)), true, false, false, 0.);
    input::update(&ui.lua, &mut mouse, Some((0., 0.)), false, true, false, 0.);
    lua::tick(&ui.lua, 16.);
    assert!(!lua::ui(&ui.lua).tree.get(popup).alive);
    assert_eq!(lua::chat::reset_windows(&ui.lua), windows.len());
    assert!(!lua::ui(&ui.lua).tree.get(popup).shown);
    for old in windows {
        assert!(!lua::ui(&ui.lua).tree.get(old).alive);
    }
    assert_eq!(ui.focus(), None);
    // The application has reset configuration here. The toolkit does not call this callback.
    ui.exec("@reset_original_callback", "assert(OnResetAllChatWindow())")
        .unwrap();
    lua::tick(&ui.lua, 600.);
    let window = lua::ui(&ui.lua).tree.of_class("chatwindow")[0];
    let (button, message) = {
        let u = lua::ui(&ui.lua);
        let row = &u.tree.widget(window).unwrap().chat.as_ref().unwrap().tabs[0];
        (row.button, row.message)
    };
    assert!(
        lua::ui(&ui.lua)
            .tree
            .widget(message)
            .unwrap()
            .message
            .as_ref()
            .unwrap()
            .lines
            .is_empty()
    );
    let r = lua::ui(&ui.lua).tree.rect(button);
    let p = (r.x + r.w / 2., r.y + r.h / 2.);
    assert_eq!(input::pick(&ui.lua, p.0, p.1), Some(button));
    input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.);
    input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.);
    ui.exec("@reset_new_history",r#"
        HideTooltip()
        X2Chat:DispatchChatMessage(2,'Chat reset completed. Original windows and controls were recreated.')
        X2Chat:DispatchChatMessage(2,'Old history and the attached popup have been retired.')
        X2Chat:DispatchChatMessage(2,'|kLiora,0;: New messages and links remain usable.')
    "#).unwrap();
    lua::tick(&ui.lua, 16.);
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: original chat reset"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            install(ui);
            reset_and_recreate(ui);
            fake::report(ui, "native_chat_reset");
        }))
        .add_systems(Startup, fake::physical_pixels)
        .run();
}
