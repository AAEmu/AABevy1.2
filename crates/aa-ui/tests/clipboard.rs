//! Native clipboard key contracts. Every test supplies hooks; none touches the OS clipboard.

#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::{UiKey, UiRuntime, lua};
use bevy::prelude::KeyCode;

fn eval<T: mlua::FromLua>(ui: &UiRuntime, source: &str) -> T {
    ui.lua.load(source).eval().expect(source)
}

fn hooks(ui: &UiRuntime) {
    ui.exec(
        "@fixture_clipboard",
        r#"
        clipboard, reads, writes, changes = '', 0, 0, 0
        AAUI.GetClipboard = function() reads=reads+1; return clipboard end
        AAUI.SetClipboard = function(text) writes=writes+1; clipboard=text end
    "#,
    )
    .unwrap();
}

fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920.0, 1080.0).unwrap();
    hooks(&ui);
    ui.exec(
        "@clipboard_edit",
        r#"
        frame=UIParent:CreateWidget('emptywidget','clipboardFrame','UIParent')
        frame:SetExtent(500,300); frame:Show(true)
        edit=frame:CreateChildWidget('editbox','edit',0,true)
        edit:SetExtent(200,30); edit:Show(true); edit:SetFocus()
        edit:SetHandler('OnTextChanged',function() changes=changes+1 end)
        downs,ups={},{}
        edit:SetHandler('OnKeyDown',function(self,key) table.insert(downs,key) end)
        edit:SetHandler('OnKeyUp',function(self,key) table.insert(ups,key) end)
    "#,
    )
    .unwrap();
    ui
}

fn key(ui: &UiRuntime, key: KeyCode) {
    assert!(ui.key_event(key, true, None));
    assert!(ui.key_event(key, false, None));
}

fn chord(ui: &UiRuntime, modifier: KeyCode, code: KeyCode) {
    key_down(ui, modifier);
    key(ui, code);
    assert!(ui.key_event(modifier, false, None));
}

fn key_down(ui: &UiRuntime, key: KeyCode) {
    assert!(ui.key_event(key, true, None));
}

fn select_all(ui: &UiRuntime) {
    chord(ui, KeyCode::ControlLeft, KeyCode::KeyA);
}

#[test]
fn unicode_copy_all_selection_cut_and_both_insert_shortcuts_preserve_key_edges() {
    let ui = setup();
    ui.exec("@text", "edit:SetText('h\u{e9}友🙂!'); changes=0")
        .unwrap();
    lua::ui(&ui.lua).tree.settle();
    chord(&ui, KeyCode::ControlLeft, KeyCode::KeyC);
    assert_eq!(eval::<String>(&ui, "clipboard"), "hé友🙂!");
    assert!(!lua::ui(&ui.lua).tree.changed());
    chord(&ui, KeyCode::ControlLeft, KeyCode::KeyX); // No selection: no cut or write.
    assert_eq!(eval::<usize>(&ui, "writes"), 1);
    assert_eq!(eval::<usize>(&ui, "changes"), 0);
    key_down(&ui, KeyCode::ShiftLeft);
    key(&ui, KeyCode::ArrowLeft);
    key(&ui, KeyCode::ArrowLeft);
    ui.key_event(KeyCode::ShiftLeft, false, None);
    chord(&ui, KeyCode::ControlLeft, KeyCode::Insert);
    assert_eq!(eval::<String>(&ui, "clipboard"), "🙂!");
    chord(&ui, KeyCode::ControlLeft, KeyCode::KeyX);
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "hé友");
    assert_eq!(eval::<usize>(&ui, "changes"), 1);
    chord(&ui, KeyCode::ShiftLeft, KeyCode::Insert);
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "hé友🙂!");
    assert_eq!(eval::<usize>(&ui, "changes"), 2);
    assert_eq!(eval::<usize>(&ui, "reads"), 1);
    assert_eq!(eval::<String>(&ui, "downs[#downs]"), "insert");
    assert_eq!(eval::<String>(&ui, "ups[#ups-1]"), "insert");
    ui.exec("@combined", "clipboard='X'").unwrap();
    key_down(&ui, KeyCode::ControlLeft);
    key_down(&ui, KeyCode::ShiftLeft);
    key(&ui, KeyCode::Insert);
    ui.key_event(KeyCode::ShiftLeft, false, None);
    ui.key_event(KeyCode::ControlLeft, false, None);
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "hé友🙂!X");
    assert_eq!(eval::<String>(&ui, "clipboard"), "hé友🙂!X");
}

#[test]
fn native_digit_clipboard_gate_password_and_readonly_are_independent() {
    let ui = setup();
    ui.exec(
        "@digit",
        "edit:SetDigit(true); edit:SetText('12'); clipboard='abc9'",
    )
    .unwrap();
    chord(&ui, KeyCode::ControlLeft, KeyCode::KeyV);
    chord(&ui, KeyCode::ControlLeft, KeyCode::KeyC);
    assert_eq!(eval::<usize>(&ui, "reads+writes"), 0);
    ui.exec("@digit_off", "edit:SetDigit(false)").unwrap();
    chord(&ui, KeyCode::ControlLeft, KeyCode::KeyV);
    assert_eq!(eval::<usize>(&ui, "reads"), 0); // Does not implicitly reenable clipboard.
    ui.exec(
        "@enable",
        "edit:UseClipboard(true); edit:SetDigit(true); edit:UseClipboard(true)",
    )
    .unwrap();
    chord(&ui, KeyCode::ControlLeft, KeyCode::KeyV);
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "12abc9"); // Native direct buffer paste.
    ui.exec("@password", "edit:SetPassword(true)").unwrap();
    chord(&ui, KeyCode::ControlLeft, KeyCode::KeyC);
    assert_eq!(eval::<String>(&ui, "clipboard"), "12abc9");
    ui.exec(
        "@readonly",
        "edit:SetReadOnly(true); changes=0; reads=0; writes=0",
    )
    .unwrap();
    select_all(&ui);
    for code in [KeyCode::KeyC, KeyCode::KeyX, KeyCode::KeyV] {
        chord(&ui, KeyCode::ControlLeft, code);
    }
    assert_eq!(eval::<usize>(&ui, "changes+reads+writes"), 0);
    ui.exec(
        "@disabled",
        "edit:SetReadOnly(false); edit:UseClipboard(false)",
    )
    .unwrap();
    chord(&ui, KeyCode::ShiftLeft, KeyCode::Insert);
    assert_eq!(eval::<usize>(&ui, "reads"), 0);
}

#[test]
fn paste_normalizes_crlf_replaces_selection_and_obeys_character_and_native_buffer_limits() {
    let ui = setup();
    ui.exec(
        "@limit",
        "edit:SetText('keep'); edit:SetMaxTextLength(6); clipboard='友🙂\\r\\nXYZ'; changes=0",
    )
    .unwrap();
    select_all(&ui);
    chord(&ui, KeyCode::ControlLeft, KeyCode::KeyV);
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "友🙂\nXYZ");
    assert_eq!(eval::<usize>(&ui, "changes"), 1);
    ui.exec("@empty", "clipboard=''").unwrap();
    select_all(&ui);
    chord(&ui, KeyCode::ControlLeft, KeyCode::KeyV);
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "");
    assert_eq!(eval::<usize>(&ui, "changes"), 2);
    ui.exec("@nil", "clipboard=nil; changes=0").unwrap();
    chord(&ui, KeyCode::ControlLeft, KeyCode::KeyV);
    assert_eq!(eval::<usize>(&ui, "changes"), 1); // Native empty transfer buffer still marks dirty.
    ui.exec("@cached", "edit:SetText('cached'); changes=0")
        .unwrap();
    chord(&ui, KeyCode::ControlLeft, KeyCode::KeyC);
    ui.exec("@unavailable", "clipboard=nil; edit:SetText(''); changes=0")
        .unwrap();
    chord(&ui, KeyCode::ControlLeft, KeyCode::KeyV);
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "cached");
    select_all(&ui);
    ui.exec(
        "@large",
        "clipboard=string.rep('🙂',1100); edit:SetMaxTextLength(0)",
    )
    .unwrap();
    chord(&ui, KeyCode::ControlLeft, KeyCode::KeyV);
    assert_eq!(eval::<String>(&ui, "edit:GetText()").chars().count(), 1024);
    chord(&ui, KeyCode::ControlLeft, KeyCode::KeyC);
    assert_eq!(eval::<String>(&ui, "clipboard").chars().count(), 512); // Wide1025 including NUL.
}

#[test]
fn generic_capture_and_reentrant_focus_release_do_not_paste_into_an_old_owner() {
    let ui = setup();
    ui.exec("@capture", "clipboard='wrong'; frame:EnableKeyboard(true)")
        .unwrap();
    chord(&ui, KeyCode::ControlLeft, KeyCode::KeyV);
    assert_eq!(eval::<usize>(&ui, "reads"), 0);
    ui.exec(
        "@release",
        r#"
        frame:EnableKeyboard(false)
        AAUI.GetClipboard=function() reads=reads+1; edit:ClearFocus(); return 'wrong' end
    "#,
    )
    .unwrap();
    chord(&ui, KeyCode::ControlLeft, KeyCode::KeyV);
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "");
    assert_eq!(eval::<usize>(&ui, "reads"), 1);
    ui.exec(
        "@callback",
        r#"
        edit:SetFocus(); edit:SetText('before')
        AAUI.SetClipboard=function(text)
            assert(edit:GetText()==''); clipboard=text; edit:SetText('after')
        end
    "#,
    )
    .unwrap();
    select_all(&ui);
    chord(&ui, KeyCode::ControlLeft, KeyCode::KeyX);
    assert_eq!(eval::<String>(&ui, "clipboard"), "before");
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "after");
}

#[test]
fn original_friend_recipient_and_mail_multiline_accept_paste_with_authored_limits() {
    let mut ui = UiRuntime::new("en_us", 1920.0, 1080.0).unwrap();
    fake::install(&mut ui, "en_us", &fake::default_worlds());
    fake::install_ingame(&mut ui);
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    hooks(&ui);
    ui.exec(
        "@original",
        r#"
        friends=SetViewOfRelationshipFrame('clipboardFriends',UIParent)
        friends:Show(true)
        edit=friends.tab.window[1].inputName
        edit:SetText('Oldfriend'); edit:SetFocus()
        clipboard='Newfriend'
    "#,
    )
    .unwrap();
    chord(&ui, KeyCode::ControlLeft, KeyCode::KeyV);
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "Newfriend");
    chord(&ui, KeyCode::ControlLeft, KeyCode::KeyC);
    ui.exec(
        "@mail",
        r#"
        mail=SetViewOfWriteMailWindow('clipboardMail',UIParent); mail:Show(true)
        edit=mail.roleEdit; edit:SetFocus()
    "#,
    )
    .unwrap();
    chord(&ui, KeyCode::ControlLeft, KeyCode::KeyV);
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "Newfriend");
    ui.exec(
        "@body",
        "edit=mail.content; clipboard='Dear friend,\\r\\n\\r\\nA pasted letter.\\r\\n'",
    )
    .unwrap();
    let edit = lua::value_id(&ui.lua, &eval(&ui, "edit")).unwrap();
    ui.set_focus(Some(edit)); // Original multiline class takes pointer/native focus.
    ui.key(&UiKey::Paste);
    assert_eq!(
        eval::<String>(&ui, "edit:GetText()"),
        "Dear friend,\n\nA pasted letter.\n"
    );
    let limit = lua::ui(&ui.lua)
        .tree
        .widget(edit)
        .unwrap()
        .edit
        .as_ref()
        .unwrap()
        .max_len;
    ui.exec("@long", "clipboard=string.rep('a',1100)").unwrap();
    select_all(&ui);
    chord(&ui, KeyCode::ShiftLeft, KeyCode::Insert);
    assert_eq!(eval::<String>(&ui, "edit:GetText()").len(), limit.min(1024));
    assert!(
        ui.stub_calls()
            .iter()
            .all(|(name, _)| name != "UseClipboard")
    );
}
