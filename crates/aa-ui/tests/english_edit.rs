//! Original English edit flags: typed characters, unrestricted buffers and IME lifetime.

#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::{UiKey, UiRuntime, input, lua};
use bevy::{prelude::Entity, window::Ime};

fn eval<T: mlua::FromLua>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().expect(code)
}

fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1280.0, 720.0).unwrap();
    ui.exec(
        "@english_edit",
        r#"
        edit=UIParent:CreateWidget('editbox','englishEdit','UIParent')
        edit:SetExtent(220,30); edit:Show(true); edit:SetFocus()
        changes=0; edit:SetHandler('OnTextChanged',function() changes=changes+1 end)
        clipboard=''; AAUI.GetClipboard=function() return clipboard end
    "#,
    )
    .unwrap();
    ui
}

#[test]
fn typed_english_accepts_only_printable_ascii_without_space_and_rejected_keys_keep_selection() {
    let ui = setup();
    ui.exec("@english", "edit:SetEnglish(true)").unwrap();
    ui.key(&UiKey::Text(" !0AZaz~\u{7f}é友🙂".into()));
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "!0AZaz~");
    assert_eq!(eval::<usize>(&ui, "changes"), 1);
    ui.key(&UiKey::SelectAll);
    ui.key(&UiKey::Text(" é友".into()));
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "!0AZaz~");
    assert_eq!(eval::<usize>(&ui, "changes"), 1);
    ui.key(&UiKey::Text("Valid!".into()));
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "Valid!");
    ui.exec("@disable", "edit:SetEnglish(false)").unwrap();
    ui.key(&UiKey::Text("é 友".into()));
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "é 友");
}

#[test]
fn setter_always_clears_without_callback_but_missing_or_wrong_typed_arguments_do_nothing() {
    let ui = setup();
    for value in ["true", "true", "false", "false", "1", "0", "-1", "0/0"] {
        ui.exec("@seed", "edit:SetText('Retained'); changes=0")
            .unwrap();
        ui.key(&UiKey::SelectAll);
        ui.exec(
            "@set",
            &format!("assert(select('#',edit:SetEnglish({value}))==0)"),
        )
        .unwrap();
        assert_eq!(eval::<String>(&ui, "edit:GetText()"), "", "{value}");
        assert_eq!(eval::<usize>(&ui, "changes"), 0, "{value}");
        let id = lua::value_id(&ui.lua, &eval(&ui, "edit")).unwrap();
        let u = lua::ui(&ui.lua);
        let edit = u.tree.widget(id).unwrap().edit.as_ref().unwrap();
        assert_eq!(edit.cursor, 0);
        assert_eq!(edit.selection_anchor, None);
    }
    for value in ["", "nil", "'true'", "{}"] {
        ui.exec("@seed", "edit:SetText('Retained'); changes=0")
            .unwrap();
        ui.exec(
            "@invalid",
            &format!("assert(select('#',edit:SetEnglish({value}))==0)"),
        )
        .unwrap();
        assert_eq!(eval::<String>(&ui, "edit:GetText()"), "Retained", "{value}");
        assert_eq!(eval::<usize>(&ui, "changes"), 0, "{value}");
    }
}

#[test]
fn english_does_not_filter_settext_or_clipboard_and_preserves_original_maximum() {
    let ui = setup();
    ui.exec("@buffers", "edit:SetEnglish(true); edit:SetMaxTextLength(5); edit:SetText('é 友🙂'); clipboard=' é友🙂ABC'; changes=0").unwrap();
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "é 友🙂");
    ui.key(&UiKey::SelectAll);
    ui.key(&UiKey::Paste);
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), " é友🙂A");
    assert_eq!(eval::<usize>(&ui, "changes"), 1);
}

#[test]
fn bool_and_numeric_arguments_control_typing_while_missing_arguments_retain_the_flag() {
    for (argument, english) in [
        ("true", true),
        ("false", false),
        ("0", false),
        ("2", true),
        ("0/0", true),
    ] {
        let ui = setup();
        ui.exec(
            "@mode",
            &format!("edit:SetEnglish({argument}); edit:SetEnglish()"),
        )
        .unwrap();
        let ascii: String = (0..128).map(char::from).collect();
        ui.key(&UiKey::Text(ascii));
        let expected: String = if english {
            ('!'..='~').collect()
        } else {
            (0..128)
                .map(char::from)
                .filter(|c| !c.is_control() || *c == '\n')
                .collect()
        };
        assert_eq!(
            eval::<String>(&ui, "edit:GetText()"),
            expected,
            "{argument}"
        );
    }
}

#[test]
fn clearing_an_active_composition_keeps_flag_until_completion_without_synthetic_change() {
    for english in [false, true] {
        let ui = setup();
        let window = Entity::PLACEHOLDER;
        assert!(ui.ime_event(&Ime::Enabled { window }));
        assert!(ui.ime_event(&Ime::Preedit {
            window,
            value: "友".into(),
            cursor: Some((3, 3))
        }));
        ui.exec("@mode", &format!("changes=0; edit:SetEnglish({english})"))
            .unwrap();
        assert_eq!(eval::<String>(&ui, "edit:GetText()"), "");
        assert!(eval::<bool>(&ui, "edit:IsComposingText()"));
        assert_eq!(eval::<usize>(&ui, "changes"), 0);
        if english {
            lua::keyboard::begin_frame(&ui.lua);
            assert!(!ui.ime_event(&Ime::Commit {
                window,
                value: "late".into()
            }));
        } else {
            assert!(ui.ime_event(&Ime::Commit {
                window,
                value: String::new()
            }));
        }
        assert!(!eval::<bool>(&ui, "edit:IsComposingText()"));
        assert_eq!(eval::<String>(&ui, "edit:GetText()"), "");
        assert_eq!(eval::<usize>(&ui, "changes"), 0);
    }
}

#[test]
fn disabling_english_during_composition_clears_its_base_before_the_next_result() {
    let ui = setup();
    let window = Entity::PLACEHOLDER;
    ui.exec("@base", "edit:SetText('Old')").unwrap();
    assert!(ui.ime_event(&Ime::Enabled { window }));
    assert!(ui.ime_event(&Ime::Preedit {
        window,
        value: "preview".into(),
        cursor: Some((7, 7))
    }));
    ui.exec("@clear", "edit:SetEnglish(false); changes=0")
        .unwrap();
    assert!(ui.ime_event(&Ime::Commit {
        window,
        value: "友".into()
    }));
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "友");
    assert_eq!(eval::<usize>(&ui, "changes"), 1);
    assert!(!eval::<bool>(&ui, "edit:IsComposingText()"));
}

#[test]
fn original_login_username_and_password_keep_handlers_and_block_ime() {
    let mut ui = UiRuntime::new("en_us", 1280.0, 720.0).unwrap();
    fake::install(&mut ui, "en_us", &fake::default_worlds());
    // SetReadyToLogin also constructs/hides the original option window.
    fake::install_ingame(&mut ui);
    ui.exec("@login_fixture_api", "Console.GetAttribute=function() return '' end; X2Debug.GetDevMode=function() return true end").unwrap();
    ui.run_stage(aa_ui::Stage::Login);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    ui.exec("@ready", "login.stateHandler:Show(false); login.backMain.mainWindow:Show(true); SetReadyToLogin(true)").unwrap();
    for (name, text) in [
        ("login.backMain.mainWindow.nameEdit", "Account!"),
        ("login.backMain.mainWindow.passwdEdit", "Secret!"),
    ] {
        let id = lua::value_id(&ui.lua, &eval(&ui, name)).unwrap();
        let rect = lua::ui(&ui.lua).tree.rect(id);
        let point = (rect.x + 10.0, rect.y + rect.h * 0.5);
        assert_eq!(input::pick(&ui.lua, point.0, point.1), Some(id));
        let mut mouse = Default::default();
        input::update(&ui.lua, &mut mouse, Some(point), true, false, false, 0.0);
        input::update(&ui.lua, &mut mouse, Some(point), false, true, false, 0.0);
        assert_eq!(ui.focus(), Some(id));
        // Match physical character messages so original helper-label callbacks run
        // at the intermediate username/password lengths too.
        for character in format!(" {text}é友").chars() {
            ui.key(&UiKey::Text(character.to_string()));
        }
        assert_eq!(eval::<String>(&ui, &format!("{name}:GetText()")), text);
        assert!(!ui.ime_event(&Ime::Enabled {
            window: Entity::PLACEHOLDER
        }));
        assert!(!ui.ime_event(&Ime::Preedit {
            window: Entity::PLACEHOLDER,
            value: "ignored".into(),
            cursor: Some((7, 7))
        }));
    }
    assert!(eval::<bool>(
        &ui,
        "login.backMain.mainWindow.loginButton:IsEnabled()"
    ));
    assert!(!eval::<bool>(
        &ui,
        "login.backMain.mainWindow.characterPwdHelper:IsVisible()"
    ));
}
