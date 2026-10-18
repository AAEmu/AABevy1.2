//! Actual original chat input, native Trial restoration and strict pointer/edit probe.
#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;
#[path = "native_chat_method.rs"]
#[allow(dead_code)]
mod original;
use aa_ui::{UiRuntime, input, lua, tree::Id};
use bevy::prelude::*;

pub fn install(ui: &mut UiRuntime, immediate: bool) -> Id {
    let window = original::install(ui);
    ui.exec(
        "@trial_chat_draft",
        r#"
        methodEdit:SetText('This draft must clear before Trial is displayed.')
        X2Chat:DispatchChatMessage(2,'Trial input follows the native pending-method lifecycle.')
    "#,
    )
    .unwrap();
    if !immediate {
        ui.exec("@trial_close_input", "methodEdit:Show(false)")
            .unwrap();
    }
    lua::chat_input::set_forced_method(&ui.lua, 11);
    if !immediate {
        ui.exec(
            "@trial_deferred",
            "assert(methodChat:GetChatMethodSelector():GetText()==locale.chatChannel.party)",
        )
        .unwrap();
        lua::chat_input::prepare_input(&ui.lua, window);
        ui.exec("@trial_open", "methodEdit:Show(true);methodEdit:SetFocus()")
            .unwrap();
    }
    ui.exec(
        "@trial_restored",
        r#"
        assert(methodEdit:GetText()=='')
        assert(methodChat:GetChatMethodSelector():GetText()==locale.chatChannel.trial)
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 600.);
    let edit = lua::ui(&ui.lua)
        .tree
        .widget(window)
        .unwrap()
        .chat
        .as_ref()
        .unwrap()
        .edit;
    let r = lua::ui(&ui.lua).tree.rect(edit);
    let p = (r.x + r.w * 0.75, r.y + r.h / 2.);
    assert_eq!(input::pick(&ui.lua, p.0, p.1), Some(edit));
    let mut mouse = input::MouseState::default();
    input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.);
    input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.);
    assert_eq!(ui.focus(), Some(edit));
    assert!(ui.type_text("Ready to speak in Trial"));
    // Native clear only removes the pending request; it does not change the edit.
    lua::chat_input::set_forced_method(&ui.lua, -6);
    ui.exec(
        "@trial_clear_pending",
        "assert(methodEdit:GetText()=='Ready to speak in Trial')",
    )
    .unwrap();
    lua::tick(&ui.lua, 16.);
    window
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: native forced Trial chat"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            install(ui, std::env::args().any(|arg| arg == "--immediate"));
            fake::report(ui, "native_chat_forced_method");
        }))
        .add_systems(Startup, fake::physical_pixels)
        .run();
}
