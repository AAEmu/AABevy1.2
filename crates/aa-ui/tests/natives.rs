//! Native widget behaviour driven from Lua, without a window: edit box focus and typing,
//! check button state and `OnCheckChanged`.

use aa_ui::{UiKey, UiRuntime};

fn runtime() -> UiRuntime {
    UiRuntime::new("en_us", 1920.0, 1080.0).expect("runtime")
}

fn eval<T: mlua::FromLua>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().expect(code)
}

#[test]
fn edit_box_typing() {
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        edit = UIParent:CreateWidget("editbox", "testEdit", "UIParent")
        edit:SetExtent(200, 24)
        edit:Show(true)
        changes, entered, escaped, focused = 0, 0, 0, 0
        edit:SetHandler("OnTextChanged", function(self) changes = changes + 1 end)
        edit:SetHandler("OnEnterPressed", function(self) entered = entered + 1 end)
        edit:SetHandler("OnEscapePressed", function(self) escaped = escaped + 1 end)
        edit:SetHandler("OnAcceptFocus", function(self) focused = focused + 1 end)
        "#,
    )
    .unwrap();
    // No focus: keys are not taken.
    assert!(!ui.type_text("x"));
    ui.exec("@focus", "edit:SetFocus()").unwrap();
    assert_eq!(eval::<i64>(&ui, "focused"), 1);
    assert!(ui.type_text("Ab"));
    assert!(ui.type_text("c"));
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "Abc");
    assert_eq!(eval::<i64>(&ui, "changes"), 2);
    ui.key(&UiKey::Left);
    ui.key(&UiKey::Backspace);
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "Ac");
    ui.key(&UiKey::Home);
    ui.type_text("Z");
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "ZAc");
    ui.key(&UiKey::End);
    ui.key(&UiKey::Delete);
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "ZAc");
    // SetText does not fire OnTextChanged.
    let before: i64 = eval(&ui, "changes");
    ui.exec("@set", "edit:SetText('hello')").unwrap();
    assert_eq!(eval::<i64>(&ui, "changes"), before);
    ui.key(&UiKey::Enter);
    assert_eq!(eval::<i64>(&ui, "entered"), 1);
    ui.key(&UiKey::Escape);
    assert_eq!(eval::<i64>(&ui, "escaped"), 1);
    assert_eq!(ui.focus(), None);
}

#[test]
fn edit_box_limits() {
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        edit = UIParent:CreateWidget("editbox", "testEdit", "UIParent")
        edit:Show(true)
        edit:SetMaxTextLength(4)
        edit:SetDigit(true)
        edit:SetFocus()
        "#,
    )
    .unwrap();
    ui.type_text("1a2b345");
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "1234");
    ui.exec(
        "@ro",
        "edit:SetText('') edit:SetDigit(false) edit:SetReadOnly(true)",
    )
    .unwrap();
    ui.type_text("x");
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "");
    ui.exec("@pw", "edit:SetPassword(true)").unwrap();
    assert!(eval::<bool>(&ui, "edit:IsPassword()"));
    // ClearFocus releases the keyboard.
    ui.exec("@clear", "edit:ClearFocus()").unwrap();
    assert_eq!(ui.focus(), None);
}

#[test]
fn check_button_state() {
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        chk = UIParent:CreateWidget("checkbutton", "testCheck", "UIParent")
        n, last = 0, nil
        chk:SetHandler("OnCheckChanged", function(self) n = n + 1 last = self:GetChecked() end)
        chk:SetChecked(true)
        chk:SetChecked(true)
        chk:SetChecked(false, false)
        chk:SetChecked(true)
        "#,
    )
    .unwrap();
    // Fires only on a change, and not when the second argument is false.
    assert_eq!(eval::<i64>(&ui, "n"), 2);
    assert!(eval::<bool>(&ui, "last"));
}

#[test]
fn addon_content_registry() {
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        wnd = UIParent:CreateWidget("window", "contentWnd", "UIParent")
        ADDON:RegisterContentWidget(7, wnd)
        ADDON:ShowContent(7, true)
        "#,
    )
    .unwrap();
    assert!(eval::<bool>(&ui, "ADDON:GetContent(7) == wnd"));
    assert!(eval::<bool>(&ui, "ADDON:GetContent(8) == nil"));
    assert!(eval::<bool>(&ui, "wnd:IsVisible()"));
    assert!(eval::<bool>(&ui, "X2 == ADDON"));
    assert_eq!(eval::<String>(&ui, "X2Locale:GetLocale()"), "en_us");
}

#[test]
fn caret_is_drawn() {
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        edit = UIParent:CreateWidget("editbox", "caretEdit", "UIParent")
        edit:SetExtent(200, 30)
        edit:AddAnchor("TOPLEFT", "UIParent", 100, 100)
        edit:SetCursorColor(1, 0, 0, 1)
        edit:Show(true)
        edit:SetFocus()
        "#,
    )
    .unwrap();
    ui.type_text("abc");
    let mut g = aa_ui::lua::ui(&ui.lua);
    let quads = aa_ui::render::build_quads(&mut g, None);
    let caret: Vec<_> = quads.iter().filter(|q| q.color == [1.0, 0.0, 0.0, 1.0]).collect();
    assert_eq!(caret.len(), 1, "one caret quad");
    let p = caret[0].pos;
    assert!(p[0] > 100.0 && p[0] < 140.0 && p[1] >= 100.0 && p[3] <= 130.0, "{p:?}");
}
