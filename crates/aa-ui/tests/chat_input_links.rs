//! Native active chat insertion keeps visible labels and exact raw instances separately.

#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::{UiKey, UiRuntime, item_link, lua};
use bevy::prelude::KeyCode;

fn eval<T: mlua::FromLua>(ui: &UiRuntime, source: &str) -> T {
    ui.lua.load(source).eval().expect(source)
}

fn install(ui: &UiRuntime) {
    let record = item_link::ItemRecord {
        item_type: 24488,
        grade: 4,
        count: 9,
        id: 123,
        ..Default::default()
    };
    ui.lua
        .globals()
        .set("itemLink", item_link::format(&item_link::pack(&record)))
        .unwrap();
    ui.exec("@input_link_lookup",r#"
        sent={}
        AAUI.ChatSend=function(channel,text,target) table.insert(sent,{channel,text,target}) end
        AAUI.ChatLink=function(kind,id,grade)
            if kind=='item' and id==24488 and grade==4 then return {name='Crushing Grip',color={1,0,1,1}} end
            if kind=='quest' and id==2531 then return {name='The Prophet Terrien',color={0,1,1,1}} end
        end
    "#).unwrap();
}

fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920.0, 1080.0).unwrap();
    install(&ui);
    ui.exec(
        "@input",
        r#"
        win=UIParent:CreateWidget('chatwindow','linkInputWindow','UIParent')
        win:SetExtent(430,350); win:SetChatWindowId(0); win:Show(true)
        edit=win:GetChatEdit(); edit:Show(true); edit:SetFocus()
        changes=0; edit:SetHandler('OnTextChanged',function() changes=changes+1 end)
    "#,
    )
    .unwrap();
    ui
}

#[test]
fn appends_editable_labels_at_end_and_sends_exact_item_and_quest_markup() {
    let ui = setup();
    ui.type_text("Look: ");
    ui.key(&UiKey::Home);
    ui.exec(
        "@append",
        r#"
        assert(X2Chat:IsActivatedChatInput())
        assert(X2Chat:AddItemLinkToActiveChatInput(itemLink))
        assert(edit:GetText()=='Look: [Crushing Grip]')
        assert(X2Chat:AddQuestLinkToActiveChatInput('|q2531;'))
        assert(edit:GetText()=='Look: [Crushing Grip][The Prophet Terrien]')
    "#,
    )
    .unwrap();
    let edit = lua::value_id(&ui.lua, &eval(&ui, "edit")).unwrap();
    let text_len = eval::<String>(&ui, "edit:GetText()").chars().count();
    let u = lua::ui(&ui.lua);
    let e = u.tree.widget(edit).unwrap().edit.as_ref().unwrap();
    assert_eq!(e.cursor, text_len);
    assert!(e.selection_anchor.is_none());
    drop(u);
    ui.type_text("!");
    ui.key(&UiKey::Enter);
    assert_eq!(
        eval::<String>(&ui, "sent[1][2]"),
        format!("Look: {}|q2531;!", eval::<String>(&ui, "itemLink"))
    );
    assert!(!eval::<bool>(&ui, "X2Chat:IsActivatedChatInput()"));
    // Sending clears metadata. Manually typing the former label does not resurrect a link.
    ui.exec(
        "@again",
        "edit:Show(true); edit:SetFocus(); edit:SetText('[Crushing Grip]')",
    )
    .unwrap();
    ui.key(&UiKey::Enter);
    assert_eq!(eval::<String>(&ui, "sent[2][2]"), "[Crushing Grip]");
}

#[test]
fn active_window_and_own_visibility_are_independent_of_focus() {
    let ui = setup();
    ui.exec("@other",r#"
        edit:ClearFocus()
        assert(X2Chat:IsActivatedChatInput())
        other=UIParent:CreateWidget('editbox','ordinaryEdit','UIParent'); other:Show(true); other:SetFocus()
        assert(X2Chat:IsActivatedChatInput())
        assert(X2Chat:AddQuestLinkToActiveChatInput('|q2531;'))
        assert(other:GetText()=='')
        second=UIParent:CreateWidget('chatwindow','secondChat','UIParent')
        second:SetChatWindowId(1); second:Show(true)
        secondEdit=second:GetChatEdit()
        assert(not X2Chat:IsActivatedChatInput()) -- Newly created window owns the active flag.
        assert(not X2Chat:AddQuestLinkToActiveChatInput('|q2531;'))
        secondEdit:Show(true)
        assert(X2Chat:IsActivatedChatInput())
        X2Chat:AddQuestLinkToActiveChatInput('|q2531;')
        assert(secondEdit:GetText()=='[The Prophet Terrien]')
        edit:SetFocus()
        X2Chat:AddQuestLinkToActiveChatInput('|q2531;')
        assert(edit:GetText()=='[The Prophet Terrien][The Prophet Terrien]')
        edit:Show(false)
        assert(not X2Chat:IsActivatedChatInput())
    "#).unwrap();
}

#[test]
fn malformed_and_missing_definitions_do_not_dirty_and_serialized_byte_limit_rejects_fourth_item() {
    let ui = setup();
    lua::ui(&ui.lua).tree.settle();
    ui.exec(
        "@bad",
        r#"
        assert(X2Chat:AddItemLinkToActiveChatInput('not a link'))
        assert(X2Chat:AddItemLinkToActiveChatInput('|i999,4,opaque;'))
        assert(X2Chat:AddQuestLinkToActiveChatInput('|q999;'))
        assert(edit:GetText()=='' and changes==0)
    "#,
    )
    .unwrap();
    assert!(!lua::ui(&ui.lua).tree.changed());
    ui.exec(
        "@limit",
        r#"
        for n=1,4 do assert(X2Chat:AddItemLinkToActiveChatInput(itemLink)) end
        assert(edit:GetText()=='[Crushing Grip][Crushing Grip][Crushing Grip]')
    "#,
    )
    .unwrap();
    ui.key(&UiKey::Enter);
    assert_eq!(
        eval::<String>(&ui, "sent[1][2]"),
        eval::<String>(&ui, "itemLink").repeat(3)
    );
    // Broken labels remain ordinary visible text; the unmodified item still substitutes.
    ui.exec(
        "@broken",
        r#"
        edit:Show(true); edit:SetFocus()
        X2Chat:AddItemLinkToActiveChatInput(itemLink)
        X2Chat:AddQuestLinkToActiveChatInput('|q2531;')
    "#,
    )
    .unwrap();
    ui.key(&UiKey::Backspace);
    ui.key(&UiKey::Enter);
    assert_eq!(
        eval::<String>(&ui, "sent[2][2]"),
        format!("{}[The Prophet Terrien", eval::<String>(&ui, "itemLink"))
    );
}

#[test]
fn original_add_link_item_and_shift_quest_title_handler_use_native_input() {
    let mut ui = UiRuntime::new("en_us", 1920.0, 1080.0).unwrap();
    fake::install(&mut ui, "en_us", &fake::default_worlds());
    fake::install_ingame(&mut ui);
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    lua::keyboard::install_input(&ui.lua).unwrap();
    install(&ui);
    let win = lua::ui(&ui.lua).tree.of_class("chatwindow")[0];
    ui.lua
        .globals()
        .set("win", lua::object_table(&ui.lua, win).unwrap())
        .unwrap();
    ui.exec(
        "@original_input",
        r#"
        assert(win~=nil)
        edit=win:GetChatEdit(); edit:SetText(''); edit:Show(true); edit:SetFocus()
        assert(AddLinkItem(itemLink)); assert(not AddLinkItem(nil))
        assert(edit:GetText()=='[Crushing Grip]')
        owner=UIParent:CreateWidget('button','questLinkOwner','UIParent')
        title=owner:CreateChildWidget('label','title',0,true)
        SetTitleLabelEventHandler(title,2531)
        X2Quest.GetQuestLinkText=function(self,id) assert(id==2531); return '|q2531;' end
    "#,
    )
    .unwrap();
    ui.key_event(KeyCode::ShiftLeft, true, None);
    ui.exec("@original_shift_click", "title:OnClick('LeftButton')")
        .unwrap();
    ui.key_event(KeyCode::ShiftLeft, false, None);
    assert_eq!(
        eval::<String>(&ui, "edit:GetText()"),
        "[Crushing Grip][The Prophet Terrien]"
    );
    ui.key(&UiKey::Enter);
    assert_eq!(
        eval::<String>(&ui, "sent[1][2]"),
        format!("{}|q2531;", eval::<String>(&ui, "itemLink"))
    );
}
