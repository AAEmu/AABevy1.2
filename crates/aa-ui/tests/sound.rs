//! Sounds: the `X2Sound` natives and `SetSounds` sets reach the app through the `AAUI` hooks
//! (`AAUI.PlaySound`, `PlayMusic`, `StopMusic`, `StopSound`, `WidgetSound`).

use aa_ui::input::{MouseState, update};
use aa_ui::{UiKey, UiRuntime};

fn runtime() -> UiRuntime {
    UiRuntime::new("en_us", 1920.0, 1080.0).expect("runtime")
}

fn eval<T: mlua::FromLua>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().expect(code)
}

#[test]
fn x2sound_calls_the_hooks() {
    let ui = runtime();
    // without hooks nothing happens (and nothing fails)
    ui.exec("@none", "assert(X2Sound:PlayUISound('a') == nil); X2Sound:PlayMusic('m'); X2Sound:StopMusic(); X2Sound:StopSound(3)").unwrap();
    assert!(!eval::<bool>(&ui, "X2Sound:IsPlaying(1)"));
    ui.exec(
        "@hooks",
        r#"
        calls = {}
        AAUI.PlaySound = function(name, dup) calls[#calls + 1] = "play:" .. name .. ":" .. tostring(dup); return 41 end
        AAUI.StopSound = function(id, mode) calls[#calls + 1] = "stop:" .. id .. ":" .. tostring(mode) end
        AAUI.IsSoundPlaying = function(id) return id == 41 end
        AAUI.PlayMusic = function(name) calls[#calls + 1] = "music:" .. name end
        AAUI.StopMusic = function() calls[#calls + 1] = "stopmusic" end
        id = X2Sound:PlayUISound("login_stage_try_login", true)
        X2Sound:StopSound(id, 1)
        X2Sound:PlayMusic("login_stage_music_creator")
        X2Sound:StopMusic()
        "#,
    )
    .unwrap();
    assert_eq!(eval::<i64>(&ui, "id"), 41);
    assert_eq!(
        eval::<String>(&ui, "table.concat(calls, ' ')"),
        "play:login_stage_try_login:true stop:41:1 music:login_stage_music_creator stopmusic"
    );
    assert!(eval::<bool>(&ui, "X2Sound:IsPlaying(41)"));
    assert!(!eval::<bool>(&ui, "X2Sound:IsPlaying(7)"));
}

#[test]
fn window_show_and_hide_sounds() {
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        events = {}
        AAUI.WidgetSound = function(set, event) events[#events + 1] = set .. ":" .. event end
        wnd = UIParent:CreateWidget("window", "soundWnd", "UIParent")
        plain = UIParent:CreateWidget("window", "plainWnd", "UIParent")
        wnd:SetSounds("bag")
        wnd:Show(true)
        wnd:Show(true)
        plain:Show(true)
        wnd:Show(false)
        wnd:SetSounds("")
        wnd:Show(true)
        "#,
    )
    .unwrap();
    assert_eq!(eval::<String>(&ui, "table.concat(events, ' ')"), "bag:show bag:hide");
}

#[test]
fn button_click_and_over_sounds() {
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        events = {}
        AAUI.WidgetSound = function(set, event) events[#events + 1] = set .. ":" .. event end
        ok = UIParent:CreateWidget("button", "okBtn", "UIParent")
        ok:AddAnchor("TOPLEFT", "UIParent", 100, 100)
        ok:SetExtent(80, 24)
        ok:Show(true)
        ok:SetSounds("default")
        mute = UIParent:CreateWidget("button", "muteBtn", "UIParent")
        mute:AddAnchor("TOPLEFT", "UIParent", 300, 100)
        mute:SetExtent(80, 24)
        mute:Show(true)
        "#,
    )
    .unwrap();
    let mut mouse = MouseState::default();
    let click = |mouse: &mut MouseState, x: f32, y: f32| {
        update(&ui.lua, mouse, Some((x, y)), true, false, false, 0.0);
        update(&ui.lua, mouse, Some((x, y)), false, true, false, 0.0);
    };
    click(&mut mouse, 120.0, 110.0);
    click(&mut mouse, 320.0, 110.0);
    // an enabled button plays over on entering and left_up on the click; a button without a set is silent
    assert_eq!(eval::<String>(&ui, "table.concat(events, ' ')"), "default:over default:left_up");
    ui.exec("@off", "events = {}; ok:Enable(false)").unwrap();
    click(&mut mouse, 120.0, 110.0);
    assert_eq!(eval::<String>(&ui, "table.concat(events, ' ')"), "", "a disabled button is silent");
}

#[test]
fn edit_box_typing_sounds() {
    let ui = runtime();
    ui.exec(
        "@setup",
        r#"
        events = {}
        AAUI.WidgetSound = function(set, event) events[#events + 1] = set .. ":" .. event end
        edit = UIParent:CreateWidget("editbox", "soundEdit", "UIParent")
        edit:SetExtent(200, 24)
        edit:Show(true)
        edit:SetSounds("edit_box")
        edit:SetFocus()
        "#,
    )
    .unwrap();
    ui.type_text("ab");
    ui.key(&UiKey::Backspace);
    ui.key(&UiKey::Home);
    ui.key(&UiKey::Backspace); // nothing to erase: no sound
    assert_eq!(eval::<String>(&ui, "table.concat(events, ' ')"), "edit_box:text_added edit_box:text_deleted");
}
