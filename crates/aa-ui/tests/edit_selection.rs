//! Native edit selection and geometry, including the original friend/mail views.

#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::{UiKey, UiRuntime, lua};
use bevy::prelude::KeyCode;

fn eval<T: mlua::FromLua>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().expect(code)
}

fn id(ui: &UiRuntime, name: &str) -> aa_ui::tree::Id {
    lua::value_id(&ui.lua, &eval(ui, name)).unwrap()
}

fn state(ui: &UiRuntime) -> lua::edit::EditData {
    lua::ui(&ui.lua)
        .tree
        .widget(id(ui, "edit"))
        .unwrap()
        .edit
        .clone()
        .unwrap()
}

fn setup(multiline: bool) -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920.0, 1080.0).unwrap();
    ui.exec(
        "@edit",
        &format!(
            r#"
        frame = UIParent:CreateWidget('emptywidget', 'editFrame', 'UIParent')
        frame:AddAnchor('TOPLEFT', 'UIParent', 20, 30); frame:SetExtent(300, 200); frame:Show(true)
        edit = frame:CreateChildWidget('{}', 'edit', 0, true)
        edit:AddAnchor('TOPLEFT', frame, 0, 0); edit:SetExtent(100, 40)
        edit:SetInset(5, 4, 5, 4); edit.style:SetAlign(ALIGN_TOP_LEFT)
        edit.style:SetFontSize(16); edit:Show(true)
        changes, accepted, released = 0, 0, 0
        edit:SetHandler('OnTextChanged', function() changes = changes + 1 end)
        edit:SetHandler('OnAcceptFocus', function() accepted = accepted + 1 end)
        edit:SetHandler('OnReleaseFocus', function() released = released + 1 end)
    "#,
            if multiline {
                "editboxmultiline"
            } else {
                "editbox"
            }
        ),
    )
    .unwrap();
    ui
}

fn key(ui: &UiRuntime, code: KeyCode, text: Option<&str>) {
    ui.key_event(code, true, text);
    ui.key_event(code, false, None);
}

#[test]
fn focus_select_all_replaces_unicode_with_limits_and_releases_once() {
    let ui = setup(false);
    ui.exec("@focus", "edit:SetText('héllo'); changes=0; edit:SetMaxTextLength(5); edit:UseSelectAllWhenFocused(true); edit:SetFocus(); edit:SetFocus()").unwrap();
    assert_eq!(
        (state(&ui).cursor, state(&ui).selection_anchor),
        (5, Some(0))
    );
    ui.type_text("友人abcdef");
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "友人abc");
    assert_eq!(eval::<u32>(&ui, "changes"), 1);
    ui.exec(
        "@release",
        "edit:ClearFocus(); edit:ClearFocus(); edit:SetFocus(); frame:Show(false)",
    )
    .unwrap();
    lua::tick(&ui.lua, 16.0);
    lua::tick(&ui.lua, 16.0);
    assert_eq!(eval::<u32>(&ui, "accepted"), 2);
    assert_eq!(eval::<u32>(&ui, "released"), 2);
    assert_eq!(ui.focus(), None);
}

#[test]
fn shift_selection_ctrl_a_readonly_and_collapsing_edges() {
    let ui = setup(false);
    ui.exec(
        "@focus",
        "edit:SetText('abcde'); changes=0; edit:SetFocus()",
    )
    .unwrap();
    ui.key_event(KeyCode::ShiftLeft, true, None);
    key(&ui, KeyCode::ArrowLeft, None);
    key(&ui, KeyCode::ArrowLeft, None);
    ui.key_event(KeyCode::ShiftLeft, false, None);
    assert_eq!(
        (state(&ui).cursor, state(&ui).selection_anchor),
        (3, Some(5))
    );
    ui.key(&UiKey::Left);
    assert_eq!((state(&ui).cursor, state(&ui).selection_anchor), (3, None));
    ui.key_event(KeyCode::ControlLeft, true, None);
    key(&ui, KeyCode::KeyA, Some("a"));
    key(&ui, KeyCode::KeyC, Some("c")); // A control shortcut must not insert its letter.
    ui.key_event(KeyCode::ControlLeft, false, None);
    assert_eq!(
        (state(&ui).cursor, state(&ui).selection_anchor),
        (5, Some(0))
    );
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "abcde");
    ui.exec("@read_only", "edit:SetReadOnly(true)").unwrap();
    ui.key(&UiKey::Delete);
    ui.type_text("x");
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "abcde");
    ui.exec("@writable", "edit:SetReadOnly(false)").unwrap();
    ui.key(&UiKey::Backspace);
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "");
    assert_eq!(eval::<u32>(&ui, "changes"), 1);
}

#[test]
fn pointer_click_drag_reverse_and_inherited_scale() {
    let ui = setup(false);
    ui.exec("@text", "edit:SetText('abcdefgh'); frame:SetScale(1.5)")
        .unwrap();
    let edit = id(&ui, "edit");
    let (base, view, carets) = {
        let mut u = lua::ui(&ui.lua);
        let base = u.tree.rect(edit);
        let view = u.animations.widget_rect(&u.tree, edit);
        let lo = u.fonts.layout_edit("default", 16.0, "abcdefgh", None, 0.0);
        (base, view, lo.carets)
    };
    let point = |i: usize| {
        (
            view.x + (5.0 + carets[i].x) * view.w / base.w,
            view.y + 10.0 * view.h / base.h,
        )
    };
    let mut mouse = Default::default();
    let send = |mouse: &mut _, p, down, up| {
        aa_ui::input::update(&ui.lua, mouse, Some(p), down, up, false, 0.0)
    };
    send(&mut mouse, point(6), true, false);
    assert_eq!(state(&ui).cursor, 6);
    send(&mut mouse, point(2), false, false);
    send(&mut mouse, point(2), false, true);
    assert_eq!(
        (state(&ui).cursor, state(&ui).selection_anchor),
        (2, Some(6))
    );
    assert!(state(&ui).pointer.is_none());
    ui.type_text("X");
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "abXgh");
}

#[test]
fn first_click_preserves_focus_selection_and_second_click_places_caret() {
    let ui = setup(false);
    ui.exec(
        "@select_all",
        "edit:SetText('abcdef'); edit:UseSelectAllWhenFocused(true)",
    )
    .unwrap();
    let mut mouse = Default::default();
    for _ in 0..2 {
        aa_ui::input::update(
            &ui.lua,
            &mut mouse,
            Some((25.0, 40.0)),
            true,
            false,
            false,
            0.0,
        );
        aa_ui::input::update(
            &ui.lua,
            &mut mouse,
            Some((25.0, 40.0)),
            false,
            true,
            false,
            0.0,
        );
        if eval::<u32>(&ui, "accepted") == 1
            && state(&ui).selection_anchor == Some(0)
            && state(&ui).cursor == 6
        {
            continue;
        }
        assert_eq!(
            (state(&ui).cursor, state(&ui).selection_anchor),
            (0, Some(0))
        );
    }
    assert_eq!(state(&ui).cursor, 0);
}

#[test]
fn pointer_focus_picks_the_visible_start_of_long_text_before_caret_scroll() {
    let ui = setup(true);
    ui.exec(
        "@long",
        r"edit:SetText('first\nsecond\nthird\nfourth\nfifth\nsixth')",
    )
    .unwrap();
    let mut mouse = Default::default();
    aa_ui::input::update(
        &ui.lua,
        &mut mouse,
        Some((25.0, 35.0)),
        true,
        false,
        false,
        0.0,
    );
    aa_ui::input::update(
        &ui.lua,
        &mut mouse,
        Some((25.0, 35.0)),
        false,
        true,
        false,
        0.0,
    );
    assert_eq!(state(&ui).cursor, 0);
    assert_eq!(state(&ui).scroll, [0.0; 2]);
}

#[test]
fn literal_edit_layout_retains_markup_trailing_lines_and_wrapped_boundaries() {
    let mut fonts = aa_ui::text::FontLib::new("en_us");
    let raw = "|cffff0000Hi|r\n\n";
    let lo = fonts.layout_edit("default", 16.0, raw, None, 3.0);
    assert_eq!(lo.carets.len(), raw.chars().count() + 1);
    assert_eq!(lo.line_ranges.len(), 3);
    assert_eq!(lo.carets.last().unwrap().line, 2);
    assert!(lo.glyphs.iter().all(|g| g.color.is_none()));
    let wrapped = fonts.layout_edit("default", 16.0, "abcdefghijklmnop", Some(38.0), 3.0);
    assert!(wrapped.line_ranges.len() > 2);
    for (line, &(start, end)) in wrapped.line_ranges.iter().enumerate() {
        assert!(end > start);
        assert_eq!(wrapped.carets[start].line, line);
        assert!(wrapped.line_widths[line] <= 38.0);
    }
}

#[test]
fn multiline_visual_home_end_vertical_column_and_scroll_clipping() {
    let ui = setup(true);
    ui.exec(
        "@text",
        r"edit:SetLineSpace(3); edit:SetText('abcdefghijklmnopqr\nxy\nabcdefghijklmnopqr\n')",
    )
    .unwrap();
    ui.set_focus(Some(id(&ui, "edit")));
    assert!(state(&ui).scroll[1] > 0.0);
    ui.key(&UiKey::Up);
    ui.key(&UiKey::End);
    let before = state(&ui);
    ui.key(&UiKey::Up);
    ui.key(&UiKey::Down);
    assert_eq!(state(&ui).cursor, before.cursor);
    ui.key(&UiKey::Home);
    let home = state(&ui).cursor;
    ui.key(&UiKey::End);
    assert!(state(&ui).cursor > home);
    assert_eq!(state(&ui).visual_line, before.visual_line);
    ui.key_event(KeyCode::ControlLeft, true, None);
    key(&ui, KeyCode::Home, None);
    ui.key_event(KeyCode::ControlLeft, false, None);
    assert_eq!(state(&ui).cursor, 0);
    assert_eq!(state(&ui).scroll[1], 0.0);
    ui.key(&UiKey::SelectAll);
    let quads = aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None);
    assert!(!quads.is_empty());
    for q in quads
        .iter()
        .filter(|q| q.pos[2] > q.pos[0] && q.pos[3] > q.pos[1])
    {
        assert!(
            q.pos[0] >= 25.0 && q.pos[1] >= 34.0 && q.pos[2] <= 115.0 && q.pos[3] <= 66.0,
            "{:?}",
            q.pos
        );
    }
}

#[test]
fn label_limit_width_uses_ellipsis_and_narrow_labels_do_not_overflow() {
    let ui = setup(false);
    ui.exec("@label", "edit:Show(false); label=frame:CreateChildWidget('label','subject',0,true); label:SetExtent(1,30); label:SetText('A long mail subject'); label:SetLimitWidth(true); label:Show(true)").unwrap();
    let label = id(&ui, "label");
    assert!(
        lua::ui(&ui.lua)
            .tree
            .widget(label)
            .unwrap()
            .text
            .as_ref()
            .unwrap()
            .style
            .ellipsis
    );
    assert!(aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None).is_empty());
    ui.exec(
        "@alias",
        "label:SetLimitWidth(false); label.style:SetEllipsis(true)",
    )
    .unwrap();
    assert!(
        lua::ui(&ui.lua)
            .tree
            .widget(label)
            .unwrap()
            .text
            .as_ref()
            .unwrap()
            .style
            .ellipsis
    );
}

#[test]
fn original_friend_and_mail_edits_use_selection_limits_and_literal_multiline() {
    let mut ui = UiRuntime::new("en_us", 1920.0, 1080.0).unwrap();
    fake::install(&mut ui, "en_us", &fake::default_worlds());
    fake::install_ingame(&mut ui);
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    ui.exec(
        "@original_views",
        r#"
        friends = SetViewOfRelationshipFrame('selectionFriends', UIParent)
        friends:Show(true)
        edit = friends.tab.window[1].inputName
        edit:SetText('Oldfriend'); edit:SetFocus()
    "#,
    )
    .unwrap();
    assert!(state(&ui).select_all_on_focus);
    ui.type_text("Newfriend");
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "Newfriend");
    ui.exec(
        "@original_mail",
        r#"
        mail = SetViewOfWriteMailWindow('selectionMail', UIParent)
        mail:Show(true)
        edit = mail.content
        edit:SetText('Literal |cfffffffftext|r\nSecond line\n')
    "#,
    )
    .unwrap();
    // The native multiline class deliberately has no Lua SetFocus; it accepts pointer focus.
    ui.set_focus(Some(id(&ui, "edit")));
    assert!(!state(&ui).select_all_on_focus);
    assert!(state(&ui).max_len > 0);
    ui.key(&UiKey::Up);
    ui.key(&UiKey::Home);
    ui.type_text("Edited ");
    assert_eq!(
        eval::<String>(&ui, "edit:GetText()"),
        "Literal |cfffffffftext|r\nEdited Second line\n"
    );
    assert!(ui.stub_calls().iter().all(|(name, _)| !["UseSelectAllWhenFocused", "SetLimitWidth"].contains(&name.as_str())));
}

#[test]
fn empty_digit_getter_materializes_init_value_without_a_text_change_event() {
    let ui = setup(false);
    ui.exec(
        "@native_digit_getter",
        r#"
        edit:SetText(''); assert(edit:GetText()=='')
        edit:SetDigit(true); edit:SetInitVal(17)
        edit:SetText(''); assert(edit:GetText()=='17'); assert(edit:GetText()=='17')
        edit:SetText('004'); assert(edit:GetText()=='004')
        edit:SetPassword(true); edit:SetText(''); assert(edit:GetText()=='')
        edit:SetPassword(false); edit:SetInitVal(-7); assert(edit:GetText()=='-7')
        edit:SetDigit(false); edit:SetText(''); assert(edit:GetText()=='')
        edit:SetDigit(true); edit:SetInitVal(0); edit:SetText('')
        changes=0
    "#,
    )
    .unwrap();
    let edit = id(&ui, "edit");
    lua::ui(&ui.lua).tree.settle();
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "0");
    let u = lua::ui(&ui.lua);
    assert_eq!(
        u.tree.widget(edit).unwrap().text.as_ref().unwrap().text,
        "0"
    );
    assert!(u.tree.changed());
    drop(u);
    assert_eq!(eval::<u32>(&ui, "changes"), 0);
}

#[test]
fn original_write_mail_empty_money_edits_do_not_add_a_phantom_attachment_fee() {
    let mut ui = UiRuntime::new("en_us", 1920.0, 1080.0).unwrap();
    ui.load_ui_texts(std::path::Path::new(&fake::game_db()))
        .unwrap();
    fake::install(&mut ui, "en_us", &fake::default_worlds());
    fake::install_ingame(&mut ui);
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    ui.exec("@original_mail_cost",r#"
        attachmentCount=1
        X2Mail.GetMailItemCount=function() return attachmentCount end
        mail=CreateWriteMailWindow('nativeMoneyMail',UIParent)
        mail:Clear()
        edits=mail.defaultFrame.moneyEditsWindow
        local normal=mail.checks[2].moneyWindow.Update
        local express=mail.checks[1].moneyWindow.Update
        mail.checks[2].moneyWindow.Update=function(self,value) normalFee=tonumber(value); return normal(self,value) end
        mail.checks[1].moneyWindow.Update=function(self,value) expressFee=tonumber(value); return express(self,value) end
        mail:OnUpdate()
        assert(edits.goldEdit:GetText()=='0' and edits.silverEdit:GetText()=='0' and edits.copperEdit:GetText()=='0')
        assert(normalFee==50 and expressFee==100)
        edits.goldEdit:SetText('1'); mail:OnUpdate()
        assert(normalFee==80 and expressFee==180)
        edits.goldEdit:SetText(''); attachmentCount=2; mail:OnUpdate()
        assert(normalFee==80 and expressFee==180)
    "#).unwrap();
}

#[test]
fn changed_programmatic_text_is_synchronous_reentrant_and_unchanged_text_stays_clean() {
    let ui = setup(false);
    ui.exec(
        "@text_events",
        r#"
        events={}
        edit:SetHandler('OnTextChanged',function(self)
            table.insert(events,self:GetText())
            if self:GetText()=='unclamped' then self:SetText('clamped') end
            self:SetText(self:GetText()) -- Native equality guard prevents recursion.
        end)
        edit:SetText('unclamped')
        assert(#events==2 and events[1]=='unclamped' and events[2]=='clamped')
        assert(edit:GetText()=='clamped')
    "#,
    )
    .unwrap();
    lua::ui(&ui.lua).tree.settle();
    ui.exec("@unchanged", "edit:SetText('clamped')").unwrap();
    assert!(!lua::ui(&ui.lua).tree.changed());
    assert_eq!(eval::<usize>(&ui, "#events"), 2);
}

#[test]
fn original_money_edit_clamps_changed_programmatic_text_without_event_recursion() {
    let mut ui = UiRuntime::new("en_us", 1920.0, 1080.0).unwrap();
    fake::install(&mut ui, "en_us", &fake::default_worlds());
    fake::install_ingame(&mut ui);
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    ui.exec(
        "@original_money_clamp",
        r#"
        edit=CreateMoneyEdit('nativeClampedMoney',UIParent)
        edit:SetDigit(true); edit:SetMinMaxValues(0,99)
        local original=edit.OnTextChanged
        calls=0
        edit:SetHandler('OnTextChanged',function(self) calls=calls+1; original(self) end)
        edit:SetText('999')
        assert(edit:GetText()=='99' and edit.lastMsg=='99' and calls==2)
        edit:SetText('99'); assert(calls==2)
        edit:SetText(''); assert(edit:GetText()=='0' and edit.lastMsg=='0' and calls==3)
    "#,
    )
    .unwrap();
}
