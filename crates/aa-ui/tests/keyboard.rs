//! Real key edges and original 1.2 rebinding handlers, without a window/server.

use aa_ui::{UiRuntime, lua};
use bevy::prelude::KeyCode;

fn eval<T: mlua::FromLua>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().expect(code)
}

fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920.0, 1080.0).unwrap();
    ui.exec(
        "@keyboard",
        r#"
        frame = UIParent:CreateWidget('emptywidget', 'keyCapture', 'UIParent')
        frame:SetExtent(100, 100); frame:Show(true)
        downs, ups, wheel = {}, {}, 0
        frame:SetHandler('OnKeyDown', function(self, key) table.insert(downs, key) end)
        frame:SetHandler('OnKeyUp', function(self, key) table.insert(ups, key) end)
        frame:SetHandler('OnWheelUp', function() wheel = wheel + 1 end)
        edit = frame:CreateChildWidget('editbox', 'edit', 0, true)
        edit:SetExtent(80, 20); edit:Show(true); edit:SetFocus()
    "#,
    )
    .unwrap();
    ui
}

/// The middle and the side mouse buttons are keys of the game (`mouse3`, `mouse4`, `mouse5`): the binding editor, a key capture,
/// takes them like any key; the left and right buttons are clicks, and nothing takes a button without a capture.
#[test]
fn capture_takes_the_middle_and_side_buttons_as_keys() {
    use bevy::input::mouse::MouseButton;
    let ui = setup();
    assert!(!ui.button_event("mouse4", true), "no capture, nobody takes it");
    ui.exec("@capture", "frame:EnableKeyboard(true)").unwrap();
    assert!(ui.button_event("mouse4", true));
    assert!(ui.button_event("mouse4", false));
    assert_eq!(eval::<String>(&ui, "table.concat(downs, ',')"), "mouse4");
    assert_eq!(eval::<String>(&ui, "table.concat(ups, ',')"), "mouse4");
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "", "not typed into the edit box either");
    assert!(ui.keyboard_captured());
    assert_eq!(lua::keyboard::button_name(MouseButton::Middle), Some("mouse3"));
    assert_eq!(lua::keyboard::button_name(MouseButton::Back), Some("mouse4"));
    assert_eq!(lua::keyboard::button_name(MouseButton::Forward), Some("mouse5"));
    assert_eq!(lua::keyboard::button_name(MouseButton::Left), None);
    assert_eq!(lua::keyboard::button_name(MouseButton::Right), None);
    ui.exec("@release", "frame:EnableKeyboard(false)").unwrap();
    assert!(!ui.button_event("mouse3", false), "released capture takes nothing");
}

#[test]
fn capture_receives_real_edges_and_wheel_instead_of_edit_text() {
    let ui = setup();
    ui.exec("@capture", "frame:EnableKeyboard(true)").unwrap();
    let frame = lua::value_id(&ui.lua, &eval(&ui, "frame")).unwrap();
    assert_eq!(ui.focus(), Some(frame));
    lua::ui(&ui.lua).tree.settle();
    let generation = lua::ui(&ui.lua).tree.generation;
    let content = lua::ui(&ui.lua).tree.content();
    assert!(ui.key_event(KeyCode::KeyA, true, Some("A")));
    assert_eq!(eval::<String>(&ui, "table.concat(downs, ',')"), "a");
    assert_eq!(eval::<usize>(&ui, "#ups"), 0);
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "");
    // Repeats are key-down events, never synthetic releases.
    assert!(ui.key_event(KeyCode::KeyA, true, Some("A")));
    assert!(ui.key_event(KeyCode::KeyA, false, None));
    assert_eq!(eval::<usize>(&ui, "#downs"), 2);
    assert_eq!(eval::<String>(&ui, "table.concat(ups, ',')"), "a");
    assert_eq!(lua::ui(&ui.lua).tree.generation, generation);
    assert_eq!(lua::ui(&ui.lua).tree.settle(), content);
    aa_ui::input::update(
        &ui.lua,
        &mut Default::default(),
        Some((300.0, 300.0)),
        false,
        false,
        false,
        1.0,
    );
    assert_eq!(eval::<usize>(&ui, "wheel"), 1);
    ui.exec("@release", "frame:EnableKeyboard(false)").unwrap();
    assert!(ui.key_event(KeyCode::KeyB, true, Some("B")));
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "B");
    assert!(ui.key_event(KeyCode::KeyB, false, None));
}

#[test]
fn release_frame_and_held_keys_stay_consumed_after_capture_is_hidden_or_replaced() {
    let ui = setup();
    ui.exec(
        "@release",
        r#"
        edit:ClearFocus()
        frame:EnableKeyboard(true)
        frame:SetHandler('OnKeyUp', function(self, key)
            table.insert(ups, key); self:EnableKeyboard(false)
        end)
    "#,
    )
    .unwrap();
    lua::keyboard::begin_frame(&ui.lua);
    assert!(ui.key_event(KeyCode::F4, true, None));
    assert_eq!(eval::<usize>(&ui, "#ups"), 0);
    lua::keyboard::begin_frame(&ui.lua);
    assert!(ui.key_event(KeyCode::F4, false, None));
    assert!(ui.focus().is_none());
    assert!(ui.keyboard_captured()); // Lua has released capture on this very release.
    lua::keyboard::begin_frame(&ui.lua);
    assert!(!ui.keyboard_captured());
    assert!(!ui.key_event(KeyCode::F4, true, None));
    assert!(!ui.key_event(KeyCode::F4, false, None));
    ui.exec("@hide", "frame:EnableKeyboard(true)").unwrap();
    assert!(ui.key_event(KeyCode::KeyW, true, Some("w")));
    ui.exec("@hide", "frame:Show(false)").unwrap();
    lua::keyboard::begin_frame(&ui.lua);
    assert!(ui.focus().is_none());
    assert!(ui.keyboard_captured()); // A game movement system must not inherit this held W.
    assert!(ui.key_event(KeyCode::KeyW, false, None));
    assert_eq!(eval::<String>(&ui, "table.concat(ups, ',')"), "f4");
    lua::keyboard::begin_frame(&ui.lua);
    assert!(!ui.keyboard_captured());
    ui.exec(
        "@replacement",
        r#"
        frame:Show(true); frame:EnableKeyboard(true)
        other = UIParent:CreateWidget('emptywidget', 'otherCapture', 'UIParent')
        other:Show(true); other:EnableKeyboard(true)
        frame:EnableKeyboard(false)
    "#,
    )
    .unwrap();
    let other = lua::value_id(&ui.lua, &eval(&ui, "other")).unwrap();
    assert_eq!(ui.focus(), Some(other));
    ui.exec("@owned_press", "frame:EnableKeyboard(true)")
        .unwrap();
    ui.key_event(KeyCode::KeyW, true, Some("w"));
    ui.exec(
        "@transfer",
        r#"
        other:EnableKeyboard(true)
        otherUps = 0
        other:SetHandler('OnKeyUp', function() otherUps = otherUps + 1 end)
    "#,
    )
    .unwrap();
    assert!(ui.key_event(KeyCode::KeyW, false, None));
    assert_eq!(eval::<u32>(&ui, "otherUps"), 0);
    ui.key_event(KeyCode::AltLeft, true, None);
    assert!(ui.modifiers().alt);
    lua::keyboard::focus_lost(&ui.lua);
    assert!(!ui.modifiers().alt);
}

#[test]
fn native_names_and_modifier_queries_preserve_physical_keys() {
    let ui = setup();
    ui.exec("@clear", "edit:ClearFocus()").unwrap();
    assert!(!ui.key_event(KeyCode::ShiftLeft, true, None));
    assert!(!ui.key_event(KeyCode::ShiftRight, true, None));
    assert!(!ui.key_event(KeyCode::ControlLeft, true, None));
    assert!(!ui.key_event(KeyCode::AltRight, true, None));
    assert!(eval::<bool>(
        &ui,
        "X2Input:IsShiftKeyDown() and X2Input:IsControlKeyDown() and X2Input:IsAltKeyDown()"
    ));
    ui.key_event(KeyCode::ShiftLeft, false, None);
    assert!(ui.modifiers().shift); // The other shift is still held.
    ui.exec(
        "@fallback",
        "X2Input.IsShiftKeyDown = function() return false end",
    )
    .unwrap();
    lua::keyboard::install_input(&ui.lua).unwrap();
    assert!(eval::<bool>(&ui, "X2Input:IsShiftKeyDown()"));
    for (code, name) in [
        (KeyCode::Numpad3, "number3"),
        (KeyCode::NumpadAdd, "number+"),
        (KeyCode::Quote, "apostrophe"),
        (KeyCode::Backslash, "backslash"),
        (KeyCode::Digit1, "1"),
        (KeyCode::PageUp, "pageup"),
        (KeyCode::F15, "f15"),
    ] {
        assert_eq!(aa_ui::native_key_name(code), Some(name));
    }
    lua::keyboard::focus_lost(&ui.lua);
    assert_eq!(ui.modifiers(), Default::default());
}

#[test]
fn original_key_binding_bytecode_composes_modifiers_on_release_then_releases_capture() {
    let ui = setup();
    ui.exec(
        "@original_dependencies",
        r#"
        edit:ClearFocus()
        optionLocale = {uiCallActionNames = {}, useSlashOpenChat = false}
        FIXED_KEY_BINDING = function(key) return key end
        GetOptionListKeyButtonFontColor = function() return {1, 1, 1, 1} end
        SetBGHighlighted = function() end
        X2Hotkey = {
          EnableHotkey = function(self, enabled) hotkeys = enabled end,
          IsValidActionName = function() return true end,
          IsOverridableAction = function() return false end,
          GetOptionBinding = function() return '' end,
          SetOptionBindingWithIndex = function(self, action, key, manager, arg)
            binding = {action, key, manager, arg}
          end
        }
        frame.releaseHotkeyBtn = frame:CreateChildWidget('button', 'release', 0, true)
        primary = frame:CreateChildWidget('button', 'primary', 0, true)
        primary:SetExtent(150, 24); primary:AddAnchor('TOPLEFT', frame, 0, 30)
        secondary = frame:CreateChildWidget('button', 'secondary', 0, true)
        secondary:SetExtent(150, 24)
        OPTION_PARTITION_LINE = 'partition'
        frame.content = {optionFrames = {{controlType = 'keybindingcontrol', primaryKeyEdit = primary, secondaryKeyEdit = secondary}}}
    "#,
    )
    .unwrap();
    aa_lua::run_alb(&ui.lua, "scriptsbin/x2ui/option/key_binding_option.alb").unwrap();
    ui.exec(
        "@original_page",
        r#"
        local _, installHandlers = debug.getupvalue(CreatekeyBindgingFrame, 1)
        installHandlers(frame, {'moveforward'}, 'action')
        frame:Init()
    "#,
    )
    .unwrap();
    ui.key_event(KeyCode::ShiftLeft, true, None);
    ui.key_event(KeyCode::ControlRight, true, None);
    ui.key_event(KeyCode::AltLeft, true, None);
    assert_eq!(
        aa_ui::input::pick(&ui.lua, 10.0, 40.0),
        lua::value_id(&ui.lua, &eval(&ui, "primary"))
    );
    let mut mouse = Default::default();
    aa_ui::input::update(
        &ui.lua,
        &mut mouse,
        Some((10.0, 40.0)),
        true,
        false,
        false,
        0.0,
    );
    aa_ui::input::update(
        &ui.lua,
        &mut mouse,
        Some((10.0, 40.0)),
        false,
        true,
        false,
        0.0,
    );
    assert!(eval::<bool>(
        &ui,
        "hotkeys == false and frame.lastClickedWidget == primary"
    ));
    assert!(ui.key_event(KeyCode::F4, true, None));
    assert!(eval::<bool>(&ui, "binding == nil"));
    assert!(ui.key_event(KeyCode::F4, false, None));
    assert_eq!(eval::<String>(&ui, "binding[2]"), "SHIFT-CTRL-ALT-F4");
    assert_eq!(eval::<String>(&ui, "binding[1]"), "moveforward");
    assert_eq!(eval::<u32>(&ui, "binding[3]"), 1);
    assert!(eval::<bool>(&ui, "hotkeys"));
    assert!(ui.keyboard_captured());
    lua::keyboard::begin_frame(&ui.lua);
    // A highlighted binding button may retain navigation focus, but no longer captures game keys.
    assert!(!ui.keyboard_captured());
    ui.exec("@wheel_request", "secondary:OnClick('LeftButton')")
        .unwrap();
    aa_ui::input::update(
        &ui.lua,
        &mut Default::default(),
        Some((500.0, 500.0)),
        false,
        false,
        false,
        -1.0,
    );
    assert_eq!(
        eval::<String>(&ui, "binding[2]"),
        "SHIFT-CTRL-ALT-WHEELDOWN"
    );
    assert_eq!(eval::<u32>(&ui, "binding[3]"), 2);
    // a side button binds like a key, with the modifiers held
    ui.exec("@button_request", "primary:OnClick('LeftButton')")
        .unwrap();
    assert!(ui.button_event("mouse4", true));
    assert!(eval::<bool>(&ui, "hotkeys == false"));
    assert!(ui.button_event("mouse4", false));
    assert_eq!(eval::<String>(&ui, "binding[2]"), "SHIFT-CTRL-ALT-MOUSE4");
    assert_eq!(eval::<u32>(&ui, "binding[3]"), 1);
    ui.exec(
        "@hide_request",
        "primary:OnClick('LeftButton'); frame:Show(false)",
    )
    .unwrap();
    lua::flush(&ui.lua);
    assert!(eval::<bool>(&ui, "hotkeys"));
    assert!(ui.focus().is_none());
}
