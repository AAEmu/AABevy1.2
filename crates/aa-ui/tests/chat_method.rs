//! Native raw-tab ownership, save-before-display, and retained input selection identity.
#[path = "../examples/native_chat_method.rs"]
#[allow(dead_code)]
mod original;
use aa_ui::{
    UiRuntime, lua,
    lua::chat::{ChatTabInfo, ChatWindowInfo, hydrate_window, remove_tab},
};
fn run(ui: &UiRuntime, code: &str) {
    ui.exec("@chat_method", code).unwrap();
}
fn eval<T: mlua::FromLuaMulti>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().unwrap()
}
fn id(ui: &UiRuntime, code: &str) -> u32 {
    lua::value_id(&ui.lua, &eval(ui, code)).unwrap()
}
fn setup() -> (UiRuntime, u32) {
    let ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    run(
        &ui,
        r#"
        log={};saves=0;owners={[0]=0,[3]=0,[5]=0};extras={[0]={selected=9,method=0},[1]={selected=6,method=0}}
        AAUI={
            ChatSelection=function(window,index) extras[window].selected=index;table.insert(log,'selection') end,
            ChatMethod=function(raw,method)
                lastRaw=raw;lastMethod=method;oldDisplay=window:GetChatMethodSelector():GetText()
                table.insert(log,'method:'..raw..':'..method)
                local owner=owners[raw]
                if owner~=nil then extras[owner].method=method;saves=saves+1 end
            end
        }
        function OnSelectedChatTab(button,selected) table.insert(log,'selected:'..button:GetText()..':'..tostring(selected)) end
        function OnCreateChatWindow(window)
            window:GetChatMethodSelector():SetHandler('OnContentUpdated',function(self)
                table.insert(log,'display:'..self:GetText())
            end)
        end
        window=UIParent:CreateWidget('chatwindow','methodWindow','UIParent')
        window:SetExtent(400,180);window:AddAnchor('TOPLEFT',UIParent,100,100);window:Show(true)
        edit=window:GetChatEdit();log={}
    "#,
    );
    let window = id(&ui, "window");
    (ui, window)
}
fn info(window_id: i64, ids: &[i64], selected: i32, method: i64) -> ChatWindowInfo {
    ChatWindowInfo {
        window_id,
        tabs: ids
            .iter()
            .map(|&raw| ChatTabInfo {
                id: raw,
                name: raw.to_string(),
                ..Default::default()
            })
            .collect(),
        selected,
        method,
    }
}
fn type_method(ui: &UiRuntime, text: &str) {
    run(ui, "edit:Show(true);edit:SetFocus()");
    ui.type_text(text);
}

#[test]
fn hydration_and_repeat_save_after_selection_before_display_preserving_selected_index() {
    let (ui, window) = setup();
    hydrate_window(&ui.lua, window, &info(0, &[0, 3], 1, 4)).unwrap();
    assert_eq!(
        eval::<String>(&ui, "table.concat(log,',')"),
        "selection,selected:0:false,selected:3:true,method:3:4,display:Party"
    );
    assert_eq!(eval::<String>(&ui, "oldDisplay"), "General");
    assert_eq!(eval::<i64>(&ui, "extras[0].selected"), 1);
    run(&ui, "log={}");
    type_method(&ui, "/p ");
    assert_eq!(
        eval::<String>(&ui, "table.concat(log,',')"),
        "method:3:4,display:Party"
    );
    assert_eq!(eval::<i64>(&ui, "saves"), 2);
    assert_eq!(eval::<i64>(&ui, "extras[0].selected"), 1);
    assert!(!hydrate_window(&ui.lua, window, &info(0, &[0, 3], 1, 7)).unwrap());
    assert_eq!(eval::<i64>(&ui, "saves"), 2);
}

#[test]
fn removal_clamping_retains_input_raw_id_and_global_model_resolves_its_current_owner() {
    let (ui, window) = setup();
    hydrate_window(&ui.lua, window, &info(0, &[0, 3, 5], 2, 4)).unwrap();
    remove_tab(&ui.lua, 5).unwrap();
    run(&ui, "owners[5]=nil;log={};saves=0");
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .widget(window)
            .unwrap()
            .chat
            .as_ref()
            .unwrap()
            .selected,
        1
    );
    type_method(&ui, "/g ");
    assert_eq!(eval::<i64>(&ui, "lastRaw"), 5);
    assert_eq!(eval::<i64>(&ui, "saves"), 0);
    assert_eq!(
        eval::<String>(&ui, "edit:GetParent():GetChatMethodSelector():GetText()"),
        "Guild"
    );
    run(&ui, "owners[5]=1");
    type_method(&ui, "/p ");
    assert_eq!(eval::<i64>(&ui, "extras[1].method"), 4);
    assert_eq!(eval::<i64>(&ui, "extras[1].selected"), 6);
    assert_eq!(eval::<i64>(&ui, "extras[0].method"), 4);
    assert_eq!(eval::<i64>(&ui, "saves"), 1);
}

#[test]
fn invalid_rehydrate_retains_previous_raw_id_but_fresh_invalid_and_empty_windows_do_not_save() {
    let (ui, window) = setup();
    hydrate_window(&ui.lua, window, &info(0, &[0, 3], 1, 4)).unwrap();
    run(&ui, "log={};saves=0;owners[6]=1");
    hydrate_window(&ui.lua, window, &info(1, &[6], 99, 7)).unwrap();
    assert_eq!(
        eval::<String>(&ui, "table.concat(log,',')"),
        "method:3:7,display:Guild"
    );
    assert_eq!(eval::<i64>(&ui, "extras[0].method"), 7);
    assert_eq!(eval::<i64>(&ui, "extras[1].method"), 0);
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .widget(window)
            .unwrap()
            .chat
            .as_ref()
            .unwrap()
            .selected,
        usize::MAX
    );
    let (fresh, fresh_window) = setup();
    hydrate_window(&fresh.lua, fresh_window, &info(0, &[0], 99, 5)).unwrap();
    assert_eq!(eval::<i64>(&fresh, "lastRaw"), -1);
    assert_eq!(eval::<i64>(&fresh, "saves"), 0);
    assert_eq!(
        eval::<String>(&fresh, "window:GetChatMethodSelector():GetText()"),
        "Raid"
    );
    run(&fresh, "log={}");
    hydrate_window(&fresh.lua, fresh_window, &info(1, &[], 0, 7)).unwrap();
    assert_eq!(eval::<usize>(&fresh, "#log"), 0);
    assert_eq!(
        lua::ui(&fresh.lua)
            .tree
            .widget(fresh_window)
            .unwrap()
            .chat
            .as_ref()
            .unwrap()
            .method,
        5
    );
}

#[test]
fn unknown_constructor_input_and_failed_hook_still_update_display() {
    let (ui, window) = setup();
    type_method(&ui, "/p ");
    assert_eq!(eval::<i64>(&ui, "lastRaw"), -1);
    assert_eq!(eval::<i64>(&ui, "saves"), 0);
    assert_eq!(
        eval::<String>(&ui, "window:GetChatMethodSelector():GetText()"),
        "Party"
    );
    hydrate_window(&ui.lua, window, &info(0, &[0], 0, 4)).unwrap();
    run(
        &ui,
        "AAUI.ChatMethod=function() error('save rejected') end;log={}",
    );
    type_method(&ui, "/g ");
    assert_eq!(
        eval::<String>(&ui, "table.concat(log,',')"),
        "display:Guild"
    );
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .widget(window)
            .unwrap()
            .chat
            .as_ref()
            .unwrap()
            .method,
        7
    );
    assert_eq!(eval::<i64>(&ui, "extras[0].method"), 4);
}

#[test]
fn original_chat_restores_and_types_a_method_through_the_same_persistence_hook() {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    let window = original::install(&mut ui);
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .widget(window)
            .unwrap()
            .chat
            .as_ref()
            .unwrap()
            .method,
        4
    );
    assert_eq!(
        eval::<String>(&ui, "methodChat:GetChatMethodSelector():GetText()"),
        eval::<String>(&ui, "locale.chatChannel.party")
    );
    run(&ui, "methodEdit:SetText('');methodEdit:SetFocus()");
    ui.type_text("/g ");
    assert_eq!(eval::<i64>(&ui, "savedRaw"), 0);
    assert_eq!(eval::<i64>(&ui, "savedMethod"), 7);
    assert_eq!(
        eval::<String>(&ui, "methodChat:GetChatMethodSelector():GetText()"),
        eval::<String>(&ui, "locale.chatChannel.expedition")
    );
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
}
