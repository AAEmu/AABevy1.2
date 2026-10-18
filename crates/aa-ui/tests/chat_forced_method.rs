//! Native pending Trial method and restoration before the original input opens.
#[path = "../examples/native_chat_forced_method.rs"]
#[allow(dead_code)]
mod original;

use aa_ui::{UiRuntime, lua, tree::Id};
use lua::chat::{ChatTabInfo, ChatWindowInfo, hydrate_window};
use lua::chat_input::{prepare_input, set_forced_method};

fn run(ui: &UiRuntime, code: &str) {
    ui.exec("@forced_method_test", code).unwrap();
}
fn eval<T: mlua::FromLuaMulti>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().unwrap()
}
fn id(ui: &UiRuntime, code: &str) -> Id {
    lua::value_id(&ui.lua, &eval(ui, code)).unwrap()
}
fn method(ui: &UiRuntime, window: Id) -> i64 {
    lua::ui(&ui.lua)
        .tree
        .widget(window)
        .unwrap()
        .chat
        .as_ref()
        .unwrap()
        .method
}
fn type_command(ui: &UiRuntime, command: &str) {
    run(ui, "edit:SetText('');edit:Show(true);edit:SetFocus()");
    assert!(ui.type_text(command));
    run(ui, "log={}");
}
fn setup() -> (UiRuntime, Id) {
    let ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    ui.lua
        .globals()
        .set(
            "testPrefix",
            ui.lua
                .create_function(|lua, ()| {
                    let edit = lua::value_id(lua, &lua.globals().get("edit")?).unwrap();
                    Ok(lua::ui(lua)
                        .tree
                        .widget(edit)
                        .unwrap()
                        .edit
                        .as_ref()
                        .unwrap()
                        .prefix
                        .clone())
                })
                .unwrap(),
        )
        .unwrap();
    run(
        &ui,
        r#"
        AAUI={ChatTabs=function() return {{id=2,name='All'},method=4} end}
        window=UIParent:CreateWidget('chatwindow','forcedChat','UIParent')
        window:SetExtent(430,220);window:AddAnchor('TOPLEFT',UIParent,100,100)
        window:SetChatWindowId(0);window:Show(true)
        edit=window:GetChatEdit();selector=window:GetChatMethodSelector()
        edit:Show(true);edit:SetFocus();edit:SetText('draft');log={}
        edit:SetHandler('OnTextChanged',function(self)
            table.insert(log,'text:'..self:GetText()..':'..testPrefix()..':'..selector:GetText())
        end)
        AAUI.ChatMethod=function(raw,method)
            table.insert(log,'save:'..raw..':'..method..':'..selector:GetText())
        end
        selector:SetHandler('OnContentUpdated',function(self)
            table.insert(log,'display:'..self:GetText())
        end)
    "#,
    );
    let window = id(&ui, "window");
    (ui, window)
}

#[test]
fn immediate_restoration_clears_prefix_and_draft_before_save_display_and_repeats() {
    let (ui, window) = setup();
    run(&ui, "edit:SetPrefixText('Old prefix');log={}");
    set_forced_method(&ui.lua, 11);
    assert_eq!(method(&ui, window), 11);
    assert_eq!(
        eval::<String>(&ui, "table.concat(log,',')"),
        "text:::Party,save:2:11:Party,display:Trial"
    );
    assert_eq!(ui.focus(), Some(id(&ui, "edit")));
    run(&ui, "log={}");
    set_forced_method(&ui.lua, 11);
    assert_eq!(
        eval::<String>(&ui, "table.concat(log,',')"),
        "save:2:11:Trial,display:Trial"
    );
    run(&ui, "edit:SetText('another draft');log={}");
    set_forced_method(&ui.lua, 11);
    assert_eq!(
        eval::<String>(&ui, "table.concat(log,',')"),
        "text:::Trial,save:2:11:Trial,display:Trial"
    );
}

#[test]
fn pending_trial_is_consumed_only_by_explicit_open_with_a_different_current_method() {
    let (ui, window) = setup();
    run(&ui, "edit:Show(false);log={}");
    set_forced_method(&ui.lua, 11);
    assert_eq!(method(&ui, window), 4);
    run(&ui, "edit:Show(true);edit:SetFocus()");
    assert_eq!(
        method(&ui, window),
        4,
        "generic visibility/focus must not consume"
    );
    prepare_input(&ui.lua, window);
    assert_eq!(method(&ui, window), 11);
    type_command(&ui, "/p ");
    assert_eq!(method(&ui, window), 4);
    prepare_input(&ui.lua, window);
    assert_eq!(method(&ui, window), 4, "the deferred request was consumed");
    run(&ui, "edit:Show(false)");
    set_forced_method(&ui.lua, 11);
    type_command(&ui, "/11 ");
    prepare_input(&ui.lua, window);
    assert_eq!(
        eval::<usize>(&ui, "#log"),
        0,
        "already Trial does not restore"
    );
    type_command(&ui, "/p ");
    prepare_input(&ui.lua, window);
    assert_eq!(
        method(&ui, window),
        11,
        "already Trial had left the request pending"
    );
}

#[test]
fn clear_and_other_pending_methods_do_not_change_closed_input_on_open() {
    let (ui, window) = setup();
    run(&ui, "edit:Show(false);log={}");
    set_forced_method(&ui.lua, 7);
    prepare_input(&ui.lua, window);
    assert_eq!(method(&ui, window), 4);
    set_forced_method(&ui.lua, 11);
    set_forced_method(&ui.lua, -6);
    prepare_input(&ui.lua, window);
    assert_eq!(method(&ui, window), 4);
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "draft");
    run(&ui, "edit:Show(true);log={}");
    set_forced_method(&ui.lua, -6);
    assert_eq!(eval::<usize>(&ui, "#log"), 0);
    set_forced_method(&ui.lua, -9);
    assert_eq!(method(&ui, window), -9);
    assert_eq!(eval::<String>(&ui, "selector:GetText()"), "");
}

#[test]
fn active_window_and_own_edit_visibility_gate_immediate_restoration_without_focus_requirement() {
    let (ui, window) = setup();
    run(
        &ui,
        r#"
        other=UIParent:CreateWidget('chatwindow','otherChat','UIParent')
        other:SetExtent(430,220);other:SetChatWindowId(1)
        other:GetChatEdit():Show(false)
        outside=UIParent:CreateWidget('editbox','outsideEdit','UIParent')
        outside:SetExtent(100,30);outside:Show(true);outside:SetFocus();log={}
    "#,
    );
    set_forced_method(&ui.lua, 11);
    assert_eq!(
        method(&ui, window),
        4,
        "the other window is active but closed"
    );
    run(
        &ui,
        "edit:SetFocus();outside:SetFocus();window:Show(false);log={}",
    );
    set_forced_method(&ui.lua, 11);
    assert_eq!(
        method(&ui, window),
        11,
        "own shown input works through hidden parent"
    );
    assert_eq!(ui.focus(), Some(id(&ui, "outside")));
}

#[test]
fn restore_preserves_cached_whisper_and_hydration_uses_the_same_clear_order() {
    let (ui, window) = setup();
    type_command(&ui, "/w Liora draft");
    assert_eq!(method(&ui, window), -3);
    set_forced_method(&ui.lua, 11);
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .widget(window)
            .unwrap()
            .chat
            .as_ref()
            .unwrap()
            .whisper,
        "Liora"
    );
    set_forced_method(&ui.lua, -3);
    assert_eq!(eval::<String>(&ui, "testPrefix()"), "Liora");
    run(&ui, "edit:SetText('rehydrate draft');log={}");
    let info = ChatWindowInfo {
        window_id: 7,
        tabs: vec![ChatTabInfo {
            id: 3,
            name: "New".into(),
            ..Default::default()
        }],
        selected: 0,
        method: 7,
    };
    hydrate_window(&ui.lua, window, &info).unwrap();
    assert_eq!(
        eval::<String>(&ui, "table.concat(log,',')"),
        "text:::Whisper,save:3:7:Whisper,display:Guild"
    );
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "");
}

#[test]
fn restoration_cancels_composition_and_retired_or_unknown_windows_are_safe() {
    use bevy::{prelude::Entity, window::Ime};
    let (ui, window) = setup();
    ui.ime_event(&Ime::Enabled {
        window: Entity::PLACEHOLDER,
    });
    ui.ime_event(&Ime::Preedit {
        window: Entity::PLACEHOLDER,
        value: "preview".into(),
        cursor: Some((7, 7)),
    });
    set_forced_method(&ui.lua, 11);
    assert!(!ui.ime_event(&Ime::Commit {
        window: Entity::PLACEHOLDER,
        value: "stale".into()
    }));
    assert_eq!(eval::<String>(&ui, "edit:GetText()"), "");
    lua::chat::reset_windows(&ui.lua);
    set_forced_method(&ui.lua, -6);
    prepare_input(&ui.lua, window);
    prepare_input(&ui.lua, Id::MAX);
    assert!(lua::ui(&ui.lua).tree.of_class("chatwindow").is_empty());
}

#[test]
fn unchanged_empty_text_preserves_edit_state_like_native_set_text_early_return() {
    let (ui, _) = setup();
    run(&ui, "edit:SetText('');log={}");
    let edit = id(&ui, "edit");
    {
        let mut u = lua::ui(&ui.lua);
        let state = u.tree.widget_mut(edit).unwrap().edit.as_mut().unwrap();
        state.selection_anchor = Some(0);
        state.scroll = [3., 4.];
    }
    set_forced_method(&ui.lua, 11);
    assert_eq!(
        eval::<String>(&ui, "table.concat(log,',')"),
        "save:2:11:Party,display:Trial"
    );
    let u = lua::ui(&ui.lua);
    let state = u.tree.widget(edit).unwrap().edit.as_ref().unwrap();
    assert_eq!(state.selection_anchor, Some(0));
    assert_eq!(state.scroll, [3., 4.]);
}

#[test]
fn text_callback_prefix_mutation_survives_nonwhisper_restoration() {
    let (ui, window) = setup();
    run(
        &ui,
        "edit:SetHandler('OnTextChanged',function(self) self:SetPrefixText('Callback prefix') end)",
    );
    set_forced_method(&ui.lua, 11);
    assert_eq!(method(&ui, window), 11);
    assert_eq!(eval::<String>(&ui, "testPrefix()"), "Callback prefix");
}

#[test]
fn original_control_deferred_and_immediate_paths_remain_physically_editable() {
    for immediate in [false, true] {
        let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
        let window = original::install(&mut ui, immediate);
        assert_eq!(method(&ui, window), 11);
        run(&ui, "assert(savedRaw==0 and savedMethod==11)");
        assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    }
}
