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
    let caret: Vec<_> = quads
        .iter()
        .filter(|q| q.color == [1.0, 0.0, 0.0, 1.0])
        .collect();
    assert_eq!(caret.len(), 1, "one caret quad");
    let p = caret[0].pos;
    assert!(
        p[0] > 100.0 && p[0] < 140.0 && p[1] >= 100.0 && p[3] <= 130.0,
        "{p:?}"
    );
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
    aa_ui::input::update(
        &ui.lua,
        &mut mouse,
        Some((x, 110.0)),
        true,
        false,
        false,
        0.0,
    );
    assert_eq!(eval::<f64>(&ui, "slider:GetValue()"), 75.0);
    assert_eq!(eval::<i64>(&ui, "changes"), 1);
    let tx = eval::<f64>(&ui, "local x = thumb:GetOffset() return x");
    assert_eq!(tx, 100.0 + 150.0);
    // Drag outside the widget (captured), past the end: clamped to the maximum.
    aa_ui::input::update(
        &ui.lua,
        &mut mouse,
        Some((900.0, 400.0)),
        false,
        false,
        false,
        0.0,
    );
    assert_eq!(eval::<f64>(&ui, "last"), 100.0);
    // Release, then moving does nothing.
    aa_ui::input::update(
        &ui.lua,
        &mut mouse,
        Some((900.0, 400.0)),
        false,
        true,
        false,
        0.0,
    );
    aa_ui::input::update(
        &ui.lua,
        &mut mouse,
        Some((110.0, 110.0)),
        false,
        false,
        false,
        0.0,
    );
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
    assert!(eval::<bool>(
        &ui,
        "win:GetChatEdit() ~= nil and win:GetAddButton() ~= nil"
    ));
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
        grid:SetRowHeight(35, 2)
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

#[test]
fn content_toggle_and_escape() {
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        UIC_TEST_BAG = 19
        bag = UIParent:CreateWidget("window", "testBag", "UIParent")
        bag:SetExtent(100, 100)
        bag:SetCloseOnEscape(true)
        bag:Show(false)
        ADDON:RegisterContentWidget(UIC_TEST_BAG, bag)
        hidden, menus = 0, 0
        bag:SetHandler("OnHide", function() hidden = hidden + 1 end)
        menu = UIParent:CreateWidget("emptywidget", "testMenu", "UIParent")
        menu:RegisterEvent("OPEN_CONFIG")
        menu:SetHandler("OnEvent", function() menus = menus + 1 end)
        "#,
    )
    .unwrap();
    let bag = ui.content_id("UIC_TEST_BAG").unwrap();
    assert!(!ui.content_visible(bag));
    ui.toggle_content(bag).unwrap();
    assert!(ui.content_visible(bag));
    assert!(matches!(
        ui.escape().unwrap(),
        aa_ui::content::Escaped::Closed(_)
    ));
    assert!(!ui.content_visible(bag));
    assert_eq!(eval::<i64>(&ui, "hidden"), 1);
    assert_eq!(ui.escape().unwrap(), aa_ui::content::Escaped::OpenedMenu);
    assert_eq!(eval::<i64>(&ui, "menus"), 1);
    assert!(ui.hotkey("open_config").unwrap());
    assert_eq!(eval::<i64>(&ui, "menus"), 2);
    assert!(!ui.hotkey("toggle_walk").unwrap());
}

#[test]
fn is_visible_is_the_own_flag() {
    // The system menu counts `buttons[i]:IsVisible()` to lay itself out before it shows.
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        frame = UIParent:CreateWidget("window", "testFrame", "UIParent")
        child = frame:CreateChildWidget("button", "child", 0, true)
        hiddenChild = frame:CreateChildWidget("button", "hiddenChild", 0, true)
        hiddenChild:Show(false)
        frame:Show(false)
        "#,
    )
    .unwrap();
    assert!(eval::<bool>(&ui, "child:IsVisible()"));
    assert!(!eval::<bool>(&ui, "hiddenChild:IsVisible()"));
    assert!(!eval::<bool>(&ui, "frame:IsVisible()"));
}

#[test]
fn scroll_thumb_follows_the_visible_part() {
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        vs = UIParent:CreateWidget("slider", "testScroll", "UIParent")
        vs:AddAnchor("TOPLEFT", "UIParent", 100, 100)
        vs:SetExtent(18, 412)
        thumb = vs:CreateChildWidget("button", "thumb", 0, true)
        thumb:SetExtent(17, 41)
        vs:SetThumbButtonWidget(thumb)
        vs:SetMinThumbLength(50)
        vs:Show(true)
        vs:SetMinMaxValues(0, 390)
        vs:SetPageStep(435)
        "#,
    )
    .unwrap();
    // reference/ingame/options_keybinds: 412 * 435 / (390 + 435) = 217.2.
    let h = eval::<f64>(&ui, "local _, h = thumb:GetEffectiveExtent() return h");
    assert!((h - 217.236).abs() < 0.01, "{h}");
    // Nothing to scroll: the whole track; a long list: the minimum.
    ui.exec("@fit", "vs:SetMinMaxValues(0, 0)").unwrap();
    assert_eq!(
        eval::<f64>(&ui, "local _, h = thumb:GetEffectiveExtent() return h"),
        412.0
    );
    ui.exec("@long", "vs:SetMinMaxValues(0, 100000)").unwrap();
    assert_eq!(
        eval::<f64>(&ui, "local _, h = thumb:GetEffectiveExtent() return h"),
        50.0
    );
    // A fixed thumb keeps its size.
    ui.exec("@fixed", "vs:SetFixedThumb(true)").unwrap();
    assert_eq!(
        eval::<f64>(&ui, "local _, h = thumb:GetEffectiveExtent() return h"),
        41.0
    );
}

#[test]
fn anchors_on_one_edge_go_by_point_order() {
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        a = UIParent:CreateWidget("emptywidget", "a", "UIParent")
        a:SetExtent(40, 30)
        a:AddAnchor("TOPLEFT", "UIParent", 1400, 385)
        b = UIParent:CreateWidget("emptywidget", "b", "UIParent")
        b:SetExtent(470, 265)
        b:AddAnchor("TOPLEFT", "UIParent", 1400, 415)
        line = UIParent:CreateWidget("emptywidget", "line", "UIParent")
        line:SetHeight(5)
        line:AddAnchor("TOPLEFT", a, "BOTTOMRIGHT", 0, -5)
        line:AddAnchor("TOPRIGHT", b, 11, 0)
        grid = UIParent:CreateWidget("emptywidget", "grid", "UIParent")
        grid:SetExtent(470, 600)
        grid:AddAnchor("LEFT", "UIParent", 0, 0)
        grid:AddAnchor("TOPLEFT", b, 0, 0)
        "#,
    )
    .unwrap();
    // reference/ingame/bag: the tab line sits at the TOPLEFT anchor's y (410, not 415).
    let r: Vec<f64> = eval(
        &ui,
        "local x, y = line:GetEffectiveOffset() return {x, y, (line:GetEffectiveExtent())}",
    );
    assert_eq!(r, [1440.0, 410.0, 441.0]);
    // The slot grid: TOPLEFT beats the LEFT it was created with.
    let r: Vec<f64> = eval(&ui, "return {grid:GetEffectiveOffset()}");
    assert_eq!(r, [1400.0, 415.0]);
}

#[test]
fn image_font_label() {
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        lbl = UIParent:CreateWidget("label", "level", "UIParent")
        lbl:SetAutoResize(true)
        lbl.style:SetFont("characterInfo", 37)
        lbl:SetText("1")
        lbl:Show(true)
        "#,
    )
    .unwrap();
    // ui/image_text.g: "1" is 29 - 14 = 15 wide (reference/ingame/character_info).
    assert_eq!(eval::<f64>(&ui, "return lbl:GetWidth()"), 15.0);
    assert_eq!(
        eval::<f64>(&ui, "return lbl.style:GetTextWidth('10')"),
        15.0 + 29.0
    );
}

#[test]
fn slot_click_uses_the_slot() {
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        used = {}
        AAUI.SlotContent = function(t, i) return { type = "skill", id = 100 + i, icon = "ui/icon/x.dds" } end
        AAUI.UseSlot = function(t, i, button) used[#used + 1] = t .. ":" .. i .. ":" .. button end
        s = UIParent:CreateWidget("slot", "testSlot", "UIParent")
        s:AddAnchor("TOPLEFT", "UIParent", 100, 100)
        s:SetExtent(42, 42)
        s:Show(true)
        s:EstablishSlot(ISLOT_ACTION, 3)
        clicks = 0
        s:SetHandler("OnClick", function() clicks = clicks + 1 end)
        "#,
    )
    .unwrap();
    let mut mouse = aa_ui::input::MouseState::default();
    aa_ui::input::update(
        &ui.lua,
        &mut mouse,
        Some((120.0, 120.0)),
        true,
        false,
        false,
        0.0,
    );
    aa_ui::input::update(
        &ui.lua,
        &mut mouse,
        Some((120.0, 120.0)),
        false,
        true,
        false,
        0.0,
    );
    assert_eq!(eval::<i64>(&ui, "clicks"), 1);
    let first: String = eval(&ui, "return used[1]");
    assert_eq!(
        first,
        format!("{}:3:LeftButton", eval::<i64>(&ui, "return ISLOT_ACTION"))
    );
}

#[test]
fn entity_by_name() {
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        AAUI.Entity = function(name)
          if name == "login_camera" then return { pos = { 1, 2, 3 }, angles = { x = 0, y = 0, z = 90 } } end
        end
        cam = UIParent:GetEntityByName("login_camera")
        none = UIParent:GetEntityByName("nothing")
        "#,
    )
    .unwrap();
    assert!(eval::<bool>(&ui, "return none == nil"));
    assert_eq!(eval::<f64>(&ui, "return cam:GetPos().z"), 3.0);
    assert_eq!(eval::<f64>(&ui, "return cam:GetAngles().z"), 90.0);
    assert_eq!(eval::<String>(&ui, "return cam:GetName()"), "login_camera");
}

#[test]
fn chat_input_slash_commands() {
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        sent, updates = {}, 0
        AAUI.ChatSend = function(ch, text, target) sent[#sent + 1] = { ch, text, target } end
        win = UIParent:CreateWidget("chatwindow", "chatWindow[1]", "UIParent")
        win:AddAnchor("BOTTOMLEFT", "UIParent", 0, -145)
        win:SetExtent(430, 350)
        win:SetChatWindowId(0)
        win:Show(true)
        edit = win:GetChatEdit()
        sel = win:GetChatMethodSelector()
        sel:SetHandler("OnContentUpdated", function() updates = updates + 1 end)
        edit:Show(true)
        edit:SetFocus()
        "#,
    )
    .unwrap();
    ui.type_text("/p");
    assert_eq!(eval::<String>(&ui, "return edit:GetText()"), "/p");
    ui.type_text(" ");
    assert_eq!(eval::<String>(&ui, "return edit:GetText()"), "");
    assert_eq!(eval::<String>(&ui, "return sel:GetText()"), "Party");
    assert!(eval::<i64>(&ui, "return updates") >= 1);
    ui.type_text("hello");
    ui.key(&UiKey::Enter);
    assert_eq!(eval::<String>(&ui, "return sent[1][2]"), "hello");
    assert_eq!(
        eval::<i64>(&ui, "return sent[1][1]"),
        eval::<i64>(&ui, "return CHAT_PARTY or 4")
    );
    assert!(!eval::<bool>(&ui, "return edit:IsVisible()"));
    // Whisper: the name is the prefix, the channel stays for the next line.
    ui.exec("@again", "edit:Show(true) edit:SetFocus()")
        .unwrap();
    ui.type_text("/w Maude hi");
    assert_eq!(eval::<String>(&ui, "return edit:GetText()"), "hi");
    ui.key(&UiKey::Enter);
    assert_eq!(eval::<String>(&ui, "return sent[2][3]"), "Maude");
    assert_eq!(eval::<i64>(&ui, "return sent[2][1]"), -3);
    // Unknown commands stay in the text.
    ui.exec("@again", "edit:Show(true) edit:SetFocus()")
        .unwrap();
    ui.type_text("/dance ");
    assert_eq!(eval::<String>(&ui, "return edit:GetText()"), "/dance ");
}

fn settle(ui: &UiRuntime) -> u64 {
    aa_ui::lua::ui(&ui.lua).tree.settle()
}

#[test]
fn rewriting_the_same_state_is_no_change() {
    // The HUD scripts re-anchor, re-text and re-show their widgets every frame; the meshes are
    // rebuilt only when something really changed.
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        w = UIParent:CreateWidget("label", "w", "UIParent")
        w:SetExtent(40, 20)
        w:AddAnchor("TOPLEFT", "UIParent", 5, 5)
        w:Show(true)
        w:SetText("a")
        d = w:CreateIconImageDrawable("ui/common/money_window.dds", "artwork")
        d:SetColor(1, 0, 0, 1)
        "#,
    )
    .unwrap();
    let c0 = settle(&ui);
    ui.exec(
        "@same",
        r#"
        w:RemoveAllAnchors() w:AddAnchor("TOPLEFT", "UIParent", 5, 5)
        w:SetText("a") w:Show(true) w:SetAlpha(1)
        d:SetColor(1, 0, 0, 1) d:ClearAllTextures() d:AddTexture("ui/common/money_window.dds")
        "#,
    )
    .unwrap();
    assert_eq!(settle(&ui), c0);
    ui.exec("@move", r#"w:AddAnchor("TOPLEFT", "UIParent", 6, 5)"#)
        .unwrap();
    let c1 = settle(&ui);
    assert!(c1 > c0);
    ui.exec("@text", r#"w:SetText("b")"#).unwrap();
    assert!(settle(&ui) > c1);
}

#[test]
fn timed_out_chat_lines_redraw() {
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        m = UIParent:CreateWidget("message", "m", "UIParent")
        m:SetExtent(300, 100)
        m:AddAnchor("TOPLEFT", "UIParent", 0, 0)
        m:Show(true)
        m:SetTimeVisible(1)
        m:AddMessage("hello")
        "#,
    )
    .unwrap();
    aa_ui::lua::tick(&ui.lua, 16.0);
    let c0 = settle(&ui);
    aa_ui::lua::tick(&ui.lua, 16.0);
    assert_eq!(settle(&ui), c0);
    // Past the visible time the line leaves the live view: a change to draw.
    aa_ui::lua::tick(&ui.lua, 1200.0);
    aa_ui::lua::tick(&ui.lua, 16.0);
    assert!(settle(&ui) > c0);
}
