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

#[test]
fn slider_drag() {
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        slider = UIParent:CreateWidget("slider", "testSlider", "UIParent")
        slider:AddAnchor("TOPLEFT", "UIParent", 100, 100)
        slider:SetExtent(220, 26)
        slider:SetOrientation(1)
        slider:SetMinMaxValues(0, 100)
        thumb = slider:CreateChildWidget("button", "thumb", 0, true)
        thumb:SetExtent(20, 26)
        slider:SetThumbButtonWidget(thumb)
        slider:Show(true)
        changes, last = 0, nil
        slider:SetHandler("OnSliderChanged", function(self, v) changes = changes + 1 last = v end)
        "#,
    )
    .unwrap();
    let mut mouse = aa_ui::input::MouseState::default();
    // Press on the track at 3/4 of the travel: the thumb centre jumps there.
    let x = 100.0 + 10.0 + 0.75 * 200.0;
    aa_ui::input::update(&ui.lua, &mut mouse, Some((x, 110.0)), true, false, false, 0.0);
    assert_eq!(eval::<f64>(&ui, "slider:GetValue()"), 75.0);
    assert_eq!(eval::<i64>(&ui, "changes"), 1);
    let tx = eval::<f64>(&ui, "local x = thumb:GetOffset() return x");
    assert_eq!(tx, 100.0 + 150.0);
    // Drag outside the widget (captured), past the end: clamped to the maximum.
    aa_ui::input::update(&ui.lua, &mut mouse, Some((900.0, 400.0)), false, false, false, 0.0);
    assert_eq!(eval::<f64>(&ui, "last"), 100.0);
    // Release, then moving does nothing.
    aa_ui::input::update(&ui.lua, &mut mouse, Some((900.0, 400.0)), false, true, false, 0.0);
    aa_ui::input::update(&ui.lua, &mut mouse, Some((110.0, 110.0)), false, false, false, 0.0);
    assert_eq!(eval::<f64>(&ui, "slider:GetValue()"), 100.0);
    assert_eq!(eval::<i64>(&ui, "changes"), 2);
}

#[test]
fn chat_window_tabs() {
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        created, buttons, msgs, selected = 0, 0, {}, {}
        function OnCreateChatWindow(w) created = created + 1 end
        function OnCreateChatTabButton(b) buttons = buttons + 1 end
        function OnCreateChatMessage(m)
          msgs[#msgs + 1] = m
          up = m:GetUpButton()
          slider = m:GetSlider()
        end
        function OnSelectedChatTab(b, on) selected[#selected + 1] = on end
        AAUI.ChatTabs = function(id)
          return { { id = 0, name = "All" }, { id = 1, name = "Party", filters = { CMF_PARTY } } }
        end
        win = UIParent:CreateWidget("chatwindow", "chatWindow[1]", "UIParent")
        win:AddAnchor("BOTTOMLEFT", "UIParent", 0, -145)
        win:SetExtent(430, 350)
        win:SetTabAreaHeight(30)
        win:SetChatWindowId(0)
        win:Show(true)
        X2Chat:DispatchChatMessage(CMF_SYSTEM, "Welcome")
        X2Chat:DispatchChatMessage(CMF_PARTY, "hi party")
        "#,
    )
    .unwrap();
    assert_eq!(eval::<i64>(&ui, "created"), 1);
    assert_eq!(eval::<i64>(&ui, "buttons"), 2);
    assert!(eval::<bool>(&ui, "up ~= nil and slider ~= nil"));
    assert!(eval::<bool>(&ui, "win:GetChatEdit() ~= nil and win:GetAddButton() ~= nil"));
    // "All" takes both lines, "Party" only its own.
    assert_eq!(eval::<i64>(&ui, "msgs[1]:GetMessageLines()"), 2);
    assert_eq!(eval::<i64>(&ui, "msgs[2]:GetMessageLines()"), 1);
}

#[test]
fn textbox_line_count_and_grid_rows() {
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        box = UIParent:CreateWidget("textbox", "testBox", "UIParent")
        box:SetExtent(60, 13)
        box:SetText("one")
        short = box:GetLineCount()
        box:SetText("a much longer line of text that has to wrap in sixty pixels")
        long = box:GetLineCount()
        grid = UIParent:CreateWidget("grid", "testGrid", "UIParent")
        grid:SetRowCount(3)
        grid:SetDefaultRowHeight(20)
        grid:SetRowHeight(2, 35)
        "#,
    )
    .unwrap();
    assert_eq!(eval::<i64>(&ui, "short"), 1);
    assert!(eval::<i64>(&ui, "long") > 1);
    assert_eq!(eval::<f64>(&ui, "grid:GetAdjustRowHeight()"), 75.0);
    assert_eq!(eval::<i64>(&ui, "grid:GetRowCount()"), 3);
}

#[test]
fn tab_select_out_of_range_keeps_selection() {
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        tab = UIParent:CreateWidget("tab", "testTab", "UIParent")
        tab:SetExtent(200, 100)
        tab:AddSimpleTab("a")
        tab:AddSimpleTab("b")
        changed = {}
        tab:SetHandler("OnTabChanged", function(self, i) changed[#changed + 1] = i end)
        tab:SelectTab(2)
        tab:SelectTab(0)
        tab:SelectTab(5)
        "#,
    )
    .unwrap();
    assert_eq!(eval::<i64>(&ui, "#changed"), 1);
    assert_eq!(eval::<i64>(&ui, "changed[1]"), 2);
    assert_eq!(eval::<i64>(&ui, "tab:GetSelectedTab()"), 2);
}
