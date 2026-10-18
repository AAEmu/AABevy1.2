//! Native edit IME phases, original scripts and physical-key suppression, without an OS IME.

#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::{UiKey, UiRuntime, lua};
use bevy::{
    prelude::{Entity, KeyCode},
    window::Ime,
};

fn eval<T: mlua::FromLua>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().expect(code)
}

fn id(ui: &UiRuntime, name: &str) -> aa_ui::tree::Id {
    lua::value_id(&ui.lua, &eval(ui, name)).unwrap()
}

fn cursor(ui: &UiRuntime) -> usize {
    lua::ui(&ui.lua)
        .tree
        .widget(id(ui, "edit"))
        .unwrap()
        .edit
        .as_ref()
        .unwrap()
        .cursor
}

fn enabled(ui: &UiRuntime) -> bool {
    ui.ime_event(&Ime::Enabled {
        window: Entity::PLACEHOLDER,
    })
}

fn preedit(ui: &UiRuntime, value: &str, cursor: Option<usize>) -> bool {
    ui.ime_event(&Ime::Preedit {
        window: Entity::PLACEHOLDER,
        value: value.into(),
        cursor: cursor.map(|p| (p, p)),
    })
}

fn commit(ui: &UiRuntime, value: &str) -> bool {
    ui.ime_event(&Ime::Commit {
        window: Entity::PLACEHOLDER,
        value: value.into(),
    })
}

fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920.0, 1080.0).unwrap();
    ui.exec(
        "@ime_fixture",
        r#"
        frame=UIParent:CreateWidget('emptywidget','imeFrame','UIParent')
        frame:SetExtent(400,200); frame:Show(true)
        edit=frame:CreateChildWidget('editbox','edit',0,true)
        edit:SetExtent(200,30); edit:Show(true); edit:SetFocus()
        changes,composing,downs,ups={}, {}, {}, {}
        edit:SetHandler('OnTextChanged',function(self)
            table.insert(changes,self:GetText()); table.insert(composing,self:IsComposingText())
        end)
        edit:SetHandler('OnKeyDown',function(self,key) table.insert(downs,key) end)
        edit:SetHandler('OnKeyUp',function(self,key) table.insert(ups,key) end)
    "#,
    )
    .unwrap();
    ui
}

#[test]
fn visible_preedit_replaces_previous_preview_and_publishes_native_composing_state() {
    let ui = setup();
    ui.exec("@base", "edit:SetText('A!'); changes={}; composing={}")
        .unwrap();
    ui.key(&UiKey::Left);
    assert!(!commit(&ui, "late")); // No enabled OS context yet.
    assert!(enabled(&ui));
    assert!(preedit(&ui, "é友🙂", Some(5))); // é=2 bytes, 友=3 bytes.
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "Aé友🙂!");
    assert_eq!(cursor(&ui), 3);
    assert!(eval::<bool>(&ui, "edit:IsComposingText() and composing[1]"));
    assert!(preedit(&ui, "朋友", None));
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "A朋友!");
    assert!(commit(&ui, "友人"));
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "A友人!");
    assert_eq!(cursor(&ui), 3);
    assert_eq!(eval::<usize>(&ui, "#changes"), 3);
    assert!(eval::<bool>(
        &ui,
        "composing[1] and composing[2] and not composing[3] and not edit:IsComposingText()"
    ));
    assert_eq!(eval::<usize>(&ui, "#downs+#ups"), 2); // Only our explicit Left edge.
}

#[test]
fn starting_composition_deletes_selection_once_and_cancel_restores_committed_base() {
    let ui = setup();
    ui.exec("@selection", "edit:SetText('abcdef'); changes={}")
        .unwrap();
    ui.key(&UiKey::Left);
    ui.key_event(KeyCode::ShiftLeft, true, None);
    ui.key(&UiKey::Left);
    ui.key(&UiKey::Left);
    ui.key_event(KeyCode::ShiftLeft, false, None);
    assert!(enabled(&ui));
    assert!(preedit(&ui, "朋友", Some(6)));
    assert_eq!(
        eval::<String>(&ui, "table.concat(changes,',')"),
        "abcf,abc朋友f"
    );
    assert!(preedit(&ui, "", None));
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "abcf");
    assert_eq!(cursor(&ui), 3);
    assert!(!eval::<bool>(&ui, "edit:IsComposingText()"));
    assert!(preedit(&ui, "人", Some(3)));
    assert!(commit(&ui, "人"));
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "abc人f");
}

#[test]
fn unicode_capacity_clamps_the_entire_visible_buffer_and_result_caret_matches_native() {
    let ui = setup();
    ui.exec(
        "@capacity",
        "edit:SetText('abcde'); edit:SetMaxTextLength(5)",
    )
    .unwrap();
    for _ in 0..3 {
        ui.key(&UiKey::Left);
    }
    assert!(enabled(&ui));
    assert!(preedit(&ui, "XYZ", Some(3)));
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "abXYZ");
    assert!(preedit(&ui, "", None));
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "abcde");
    assert!(commit(&ui, "朋友"));
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "ab朋友c");
    assert_eq!(cursor(&ui), 5); // Native overflow result moves to max length, including tail.
    ui.exec("@large", "edit:SetMaxTextLength(0); edit:SetText('')")
        .unwrap();
    assert!(enabled(&ui));
    assert!(commit(&ui, &"🙂".repeat(600)));
    assert_eq!(eval::<String>(&ui, "edit:GetText()").chars().count(), 256);
}

#[test]
fn composition_keys_do_not_edit_or_send_and_the_result_frame_keeps_enter_consumed() {
    let ui = setup();
    ui.exec(
        "@chat",
        r#"
        win=UIParent:CreateWidget('chatwindow','imeChat','UIParent')
        win:SetExtent(400,200); win:SetChatWindowId(0); win:Show(true)
        edit=win:GetChatEdit(); edit:Show(true); edit:SetFocus()
        sent={}; AAUI.ChatSend=function(channel,text) table.insert(sent,text) end
    "#,
    )
    .unwrap();
    assert!(enabled(&ui));
    assert!(preedit(&ui, "友人", Some(6)));
    for key in [
        UiKey::Enter,
        UiKey::Escape,
        UiKey::Left,
        UiKey::Backspace,
        UiKey::Text("duplicate".into()),
    ] {
        ui.key(&key);
    }
    ui.key_event(KeyCode::IntlYen, true, Some("duplicate")); // No native physical name.
    ui.key_event(KeyCode::IntlYen, false, None);
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "友人");
    assert_eq!(eval::<usize>(&ui, "#sent"), 0);
    assert_eq!(cursor(&ui), 2);
    assert!(commit(&ui, "友人"));
    ui.key(&UiKey::Enter);
    assert_eq!(eval::<usize>(&ui, "#sent"), 0);
    lua::keyboard::begin_frame(&ui.lua);
    ui.key(&UiKey::Enter);
    assert_eq!(eval::<String>(&ui, "sent[1]"), "友人");
}

#[test]
fn focus_transfer_and_programmatic_replacement_reject_stale_results() {
    let ui = setup();
    assert!(enabled(&ui));
    preedit(&ui, "preview", Some(7));
    ui.exec(
        "@transfer",
        r#"
        other=frame:CreateChildWidget('editbox','other',0,true)
        other:SetExtent(200,30); other:Show(true); other:SetFocus()
    "#,
    )
    .unwrap();
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "preview"); // Focus release completes.
    assert!(!commit(&ui, "late"));
    assert_eq!(eval::<String>(&ui, "other:GetText()"), "");
    ui.exec("@return", "edit:SetFocus()").unwrap();
    assert!(!commit(&ui, "late"));
    assert!(enabled(&ui));
    preedit(&ui, "next", Some(4));
    ui.exec("@same", "edit:SetText(edit:GetText())").unwrap();
    assert!(eval::<bool>(&ui, "edit:IsComposingText()"));
    ui.exec("@replacement", "edit:SetText('application')")
        .unwrap();
    assert!(!commit(&ui, "late"));
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "application");
    assert!(enabled(&ui));
    preedit(&ui, "done", Some(4));
    lua::keyboard::focus_lost(&ui.lua);
    assert!(!commit(&ui, "late"));
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "applicationdone");
}

#[test]
fn eligibility_excludes_native_digit_password_readonly_hidden_and_key_capture() {
    for script in [
        "edit:SetEnglish(true)",
        "edit:SetDigit(true)",
        "edit:SetPassword(true)",
        "edit:SetReadOnly(true)",
        "edit:Enable(false)",
        "frame:Show(false)",
        "frame:EnableKeyboard(true)",
    ] {
        let ui = setup();
        ui.exec("@ineligible", script).unwrap();
        assert!(!enabled(&ui), "{script}");
        assert!(!preedit(&ui, "ignored", Some(7)), "{script}");
        assert!(!commit(&ui, "ignored"), "{script}");
    }
    let ui = setup();
    enabled(&ui);
    preedit(&ui, "complete", Some(8));
    ui.exec("@hide", "frame:Show(false)").unwrap();
    lua::keyboard::begin_frame(&ui.lua);
    assert!(!eval::<bool>(&ui, "edit:IsComposingText()"));
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "complete");
    assert!(!commit(&ui, "late"));
}

#[test]
fn preedit_cursor_visibility_and_idle_geometry_do_not_repaint_the_tree() {
    let ui = setup();
    enabled(&ui);
    assert!(preedit(&ui, "visible", None));
    let hidden = aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None);
    assert!(preedit(&ui, "visible", Some(7)));
    let shown = aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None);
    assert_eq!(shown.len(), hidden.len() + 1); // Only the caret changes for an unchanged preview.
    lua::ui(&ui.lua).tree.settle();
    lua::keyboard::begin_frame(&ui.lua);
    assert!(!lua::ui(&ui.lua).tree.changed());
    assert!(ui.ime_event(&Ime::Disabled {
        window: Entity::PLACEHOLDER
    }));
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "visible");
    assert!(!eval::<bool>(&ui, "edit:IsComposingText()"));
}

#[test]
fn result_callback_focus_transfer_cannot_deliver_the_same_enter_to_the_next_edit() {
    let ui = setup();
    ui.exec(
        "@redirect",
        r#"
        other=frame:CreateChildWidget('editbox','redirect',0,true)
        other:SetExtent(200,30); other:Show(true)
        enters=0; other:SetHandler('OnEnterPressed',function() enters=enters+1 end)
        edit:SetHandler('OnTextChanged',function(self)
            if not self:IsComposingText() then other:SetFocus() end
        end)
    "#,
    )
    .unwrap();
    enabled(&ui);
    preedit(&ui, "result", Some(6));
    commit(&ui, "result");
    assert_eq!(ui.focus(), Some(id(&ui, "other")));
    ui.key(&UiKey::Enter);
    assert_eq!(eval::<usize>(&ui, "enters"), 0);
    lua::keyboard::begin_frame(&ui.lua);
    ui.key(&UiKey::Enter);
    assert_eq!(eval::<usize>(&ui, "enters"), 1);
}

#[test]
fn empty_result_commits_the_visible_buffer_without_a_dirty_event_or_caret_move() {
    let ui = setup();
    enabled(&ui);
    preedit(&ui, "preview", Some(2));
    assert_eq!(cursor(&ui), 2);
    assert_eq!(eval::<usize>(&ui, "#changes"), 1);
    assert!(commit(&ui, ""));
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "preview");
    assert_eq!(cursor(&ui), 2);
    assert_eq!(eval::<usize>(&ui, "#changes"), 1);
    assert!(!eval::<bool>(&ui, "edit:IsComposingText()"));
}

#[test]
fn original_friend_and_mail_views_accept_ime_and_preserve_real_script_handlers() {
    let mut ui = UiRuntime::new("en_us", 1920.0, 1080.0).unwrap();
    fake::install(&mut ui, "en_us", &fake::default_worlds());
    fake::install_ingame(&mut ui);
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    ui.exec(
        "@views",
        r#"
        validations={}
        local checkName=X2Util.CheckAvailableName
        X2Util.CheckAvailableName=function(self,text,composing)
            table.insert(validations,{text,composing}); return checkName(self,text,composing)
        end
        friends=SetViewOfRelationshipFrame('imeFriends',UIParent); friends:Show(true)
        edit=friends.tab.window[1].inputName; edit:SetText('Oldfriend'); edit:SetFocus()
        mail=SetViewOfWriteMailWindow('imeMail',UIParent); mail:Show(true)
    "#,
    )
    .unwrap();
    enabled(&ui);
    preedit(&ui, "New friend", Some(10));
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "New friend");
    commit(&ui, "New friend");
    assert!(eval::<bool>(
        &ui,
        "validations[#validations-2][2] and not validations[#validations-1][2] and not validations[#validations][2]"
    ));
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "Newfriend");
    ui.exec(
        "@mail",
        "edit=mail.content; edit:SetText('Dear friend,\\n')",
    )
    .unwrap();
    ui.set_focus(Some(id(&ui, "edit")));
    enabled(&ui);
    assert!(preedit(&ui, "Welcome to Solzreed.", Some(20)));
    // Native IsComposingText is single-line only; multiline still exposes the visible buffer.
    assert_eq!(
        eval::<String>(&ui, "edit:GetText()"),
        "Dear friend,\nWelcome to Solzreed."
    );
    commit(&ui, "Welcome to Solzreed.");
    assert_eq!(
        eval::<String>(&ui, "edit:GetText()"),
        "Dear friend,\nWelcome to Solzreed."
    );
    assert!(!aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None).is_empty());
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
}
