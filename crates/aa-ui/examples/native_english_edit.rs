//! Original login controls exercise native English typed input and unrestricted buffers.
//! `cargo run -p aa-ui --example native_english_edit -- --shot shots/aa-ui/native-english-edit.png --shot-after 5`
//! Add `--paste` to show the separate native clipboard-buffer path; no OS clipboard is used.

#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::{UiKey, input, lua};
use bevy::{prelude::*, window::Ime};

fn main() {
    let paste = std::env::args().any(|a| a == "--paste");
    App::new()
        .add_plugins(aa_dev::default_plugins(
            "aa-ui: original login English input",
        ))
        .add_plugins(aa_dev::DevPlugin)
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(move |ui| {
            fake::install(ui, "en_us", &fake::default_worlds());
            // The original ready handler also constructs/hides the option window.
            fake::install_ingame(ui);
            ui.exec("@login_fixture_api", "Console.GetAttribute=function() return '' end; X2Debug.GetDevMode=function() return true end").unwrap();
            ui.run_stage(aa_ui::Stage::Login);
            assert!(ui.errors.is_empty(), "{:?}", ui.errors);
            ui.exec(
                "@login_ready",
                r#"
                login.stateHandler:Show(false)
                login.backMain.mainWindow:Show(true)
                SetReadyToLogin(true)
                AAUI.GetClipboard=function() return 'René Test' end
            "#,
            )
            .unwrap();
            for (name, text) in [
                ("login.backMain.mainWindow.nameEdit", "Account!"),
                ("login.backMain.mainWindow.passwdEdit", "Secret!"),
            ] {
                let value = ui.lua.load(name).eval().unwrap();
                let id = lua::value_id(&ui.lua, &value).unwrap();
                let r = lua::ui(&ui.lua).tree.rect(id);
                let p = (r.x + 12.0, r.y + r.h * 0.5);
                assert_eq!(input::pick(&ui.lua, p.0, p.1), Some(id));
                let mut mouse = Default::default();
                input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.0);
                input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.0);
                assert_eq!(ui.focus(), Some(id));
                for character in format!(" {text}é友").chars() {
                    ui.key(&UiKey::Text(character.to_string()));
                }
                assert_eq!(
                    ui.lua
                        .load(format!("{name}:GetText()"))
                        .eval::<String>()
                        .unwrap(),
                    text
                );
                assert!(!ui.ime_event(&Ime::Enabled {
                    window: Entity::PLACEHOLDER
                }));
                assert!(!ui.ime_event(&Ime::Commit {
                    window: Entity::PLACEHOLDER,
                    value: "ignored".into()
                }));
            }
            if paste {
                ui.exec("@paste_focus", "login.backMain.mainWindow.nameEdit:SetFocus()").unwrap();
                ui.key(&UiKey::SelectAll);
                ui.key(&UiKey::Paste);
                assert_eq!(
                    ui.lua.load("login.backMain.mainWindow.nameEdit:GetText()").eval::<String>().unwrap(),
                    "René Test"
                );
            }
            assert!(
                ui.lua
                    .load("login.backMain.mainWindow.loginButton:IsEnabled()")
                    .eval::<bool>()
                    .unwrap()
            );
            assert!(!ui.lua.load("login.backMain.mainWindow.characterPwdHelper:IsVisible()").eval::<bool>().unwrap());
            fake::report(ui, "native_english_edit");
        }))
        .add_systems(Startup, fake::physical_pixels)
        .add_systems(Update, fake::scripted_input)
        .run();
}
