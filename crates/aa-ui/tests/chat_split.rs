//! Native transfer order plus physical input against unchanged original World/chat bytecode.
#[path = "../examples/native_chat_split.rs"]
#[allow(dead_code)]
mod original;

use aa_ui::{
    UiRuntime, input, lua,
    tree::{Id, Rect},
};

fn run(ui: &UiRuntime, code: &str) {
    ui.exec("@chat_split_test", code).unwrap();
}
fn eval<T: mlua::FromLuaMulti>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().unwrap()
}
fn data(ui: &UiRuntime, window: Id) -> lua::chat::ChatData {
    lua::ui(&ui.lua)
        .tree
        .widget(window)
        .unwrap()
        .chat
        .clone()
        .unwrap()
}
fn rect(ui: &UiRuntime, id: Id) -> Rect {
    lua::ui(&ui.lua).tree.rect(id)
}
fn windows(ui: &UiRuntime) -> Vec<Id> {
    lua::ui(&ui.lua).tree.of_class("chatwindow")
}
fn id(ui: &UiRuntime, name: &str) -> Id {
    lua::value_id(&ui.lua, &eval(ui, name)).unwrap()
}
fn expose(ui: &UiRuntime, name: &str, id: Id) {
    ui.lua
        .globals()
        .set(name, lua::object_table(&ui.lua, id).unwrap().unwrap())
        .unwrap();
}
fn setup() -> (UiRuntime, Id) {
    let ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    run(
        &ui,
        r#"
        events={}
        function record(s) table.insert(events,s) end
        AAUI={ChatTabs=function() return {{id=0,name='All'},{id=2,name='Harbor'},{id=5,name='Trade'}} end,
            ChatSelection=function(w,i) record('select:'..w..':'..i) end,
            ChatAllocateWindow=function() record('allocate');return 1 end,
            ChatReleaseWindow=function(w) record('release:'..w) end,
            ChatTabDetached=function(w,t) record('detach:'..w..':'..t) end,
            ChatTabAttached=function(w,t,i) record('attach:'..w..':'..t..':'..i) end}
        function OnCreateChatWindow(w) record('create') end
        function OnDuplicatedChatWindow(w,i) duplicate=w;record('duplicate:'..i) end
        function OnSelectedChatTab(b,on) record('skin:'..b:GetText()..':'..tostring(on)) end
        source=UIParent:CreateWidget('chatwindow','splitTest','UIParent')
        source:SetExtent(400,180);source:AddAnchor('TOPLEFT',UIParent,100,100);source:Show(true)
        source:SetChatWindowId(0);source:AllowTabSwitch(true);events={}
    "#,
    );
    let source = id(&ui, "source");
    lua::tick(&ui.lua, 0.);
    (ui, source)
}
fn drag(ui: &UiRuntime, button: Id, to: (f32, f32)) {
    original::drag(ui, &mut Default::default(), button, to);
}

#[test]
fn split_retains_objects_history_handlers_and_native_selection_model_order() {
    let (ui, source) = setup();
    let before = data(&ui, source);
    let row = before.tabs[1].clone();
    expose(&ui, "button", row.button);
    expose(&ui, "message", row.message);
    run(
        &ui,
        "message:AddMessage('retained');button.marker={};local marker=button.marker;button:SetHandler('OnDragStop',function(self) assert(self.marker==marker);record('stop') end)",
    );
    let source_rect = rect(&ui, source);
    drag(&ui, row.button, (800., 300.));
    let target = id(&ui, "duplicate");
    assert_eq!(windows(&ui).len(), 2);
    assert_eq!(rect(&ui, source), source_rect);
    assert_eq!(data(&ui, target).tabs, vec![row.clone()]);
    assert_eq!(data(&ui, target).window_id, 1);
    assert_eq!(
        data(&ui, source).tabs,
        vec![before.tabs[0].clone(), before.tabs[2].clone()]
    );
    assert_eq!(lua::ui(&ui.lua).tree.get(row.button).parent, Some(target));
    assert_eq!(lua::ui(&ui.lua).tree.get(row.message).parent, Some(target));
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .widget(row.message)
            .unwrap()
            .message
            .as_ref()
            .unwrap()
            .lines[0]
            .0,
        "retained"
    );
    assert_eq!(
        eval::<String>(&ui, "table.concat(events,',')"),
        "create,allocate,duplicate:1,select:0:0,skin:All:true,skin:Trade:false,detach:0:2,select:1:0,skin:Harbor:true,attach:1:2:0,stop"
    );
    assert_eq!((rect(&ui, target).w, rect(&ui, target).h), (400., 180.));
}

#[test]
fn original_bytecode_split_saves_new_bounds_and_physical_drop_rejoins_original_window() {
    let mut ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    let source = original::install(&mut ui);
    let row = data(&ui, source).tabs[1].clone();
    let message = lua::ui(&ui.lua)
        .tree
        .widget(row.message)
        .unwrap()
        .message
        .clone()
        .unwrap();
    let button_rect = rect(&ui, row.button);
    drag(&ui, row.button, (870., 260.));
    let target = windows(&ui).into_iter().find(|&w| w != source).unwrap();
    assert_eq!(data(&ui, target).tabs, vec![row.clone()]);
    assert_eq!(data(&ui, target).selected, 0);
    assert_eq!(data(&ui, target).method, 0);
    let r = rect(&ui, target);
    assert_eq!(
        (r.x, r.y, r.w, r.h),
        (
            870. - button_rect.w / 2. - 5.,
            260. - button_rect.h / 2.,
            460.,
            220.
        )
    );
    assert_eq!(
        eval::<(f32, f32)>(
            &ui,
            "return splitBounds['ui_bound_chatWindow[1]'].bound.x,splitBounds['ui_bound_chatWindow[1]'].bound.y"
        ),
        (r.x, r.y)
    );
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .widget(row.message)
            .unwrap()
            .message
            .as_ref()
            .unwrap()
            .lines,
        message.lines
    );
    let r = rect(&ui, data(&ui, source).tabs[0].button);
    drag(&ui, row.button, (r.x + r.w * 0.75, r.y + r.h * 0.5));
    assert_eq!(data(&ui, source).tabs[1], row);
    assert_eq!(data(&ui, source).selected, 1);
    assert!(data(&ui, target).tabs.is_empty());
    assert!(lua::ui(&ui.lua).tree.get(target).alive);
    lua::tick(&ui.lua, 0.);
    assert_eq!(windows(&ui), vec![source]);
    assert_eq!(lua::ui(&ui.lua).tree.get(row.message).parent, Some(source));
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
}

#[test]
fn click_lock_and_allocation_failure_do_not_split_or_move_source() {
    for allocation in ["return nil", "error('allocator failure')"] {
        let (ui, source) = setup();
        let before = data(&ui, source);
        let bounds = rect(&ui, source);
        run(
            &ui,
            &format!("AAUI.ChatAllocateWindow=function() {allocation} end"),
        );
        drag(&ui, before.tabs[1].button, (800., 300.));
        assert_eq!(windows(&ui), vec![source]);
        assert_eq!(data(&ui, source).tabs, before.tabs);
        assert_eq!(rect(&ui, source), bounds);
        assert_eq!(eval::<String>(&ui, "table.concat(events,',')"), "create");
    }
    let (ui, source) = setup();
    let button = data(&ui, source).tabs[1].button;
    let r = rect(&ui, button);
    let p = (r.x + r.w / 2., r.y + r.h / 2.);
    let mut mouse = Default::default();
    input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.);
    input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.);
    assert_eq!(data(&ui, source).selected, 1);
    assert_eq!(windows(&ui), vec![source]);
    run(&ui, "source:AllowTabSwitch(false);events={}");
    drag(&ui, button, (800., 300.));
    assert_eq!(windows(&ui), vec![source]);
    assert_eq!(eval::<String>(&ui, "table.concat(events,',')"), "");
}

#[test]
fn original_drag_start_handler_precedes_clone_and_live_policy_check() {
    let (ui, source) = setup();
    expose(&ui, "button", data(&ui, source).tabs[1].button);
    run(
        &ui,
        "button:SetHandler('OnDragStart',function() source:AllowTabSwitch(false);record('lua_start') end)",
    );
    drag(&ui, data(&ui, source).tabs[1].button, (800., 300.));
    assert_eq!(windows(&ui), vec![source]);
    assert_eq!(eval::<String>(&ui, "table.concat(events,',')"), "lua_start");
}

#[test]
fn whole_window_drop_detaches_all_rows_before_insertion_and_protects_raw_zero() {
    let (ui, target) = setup();
    run(
        &ui,
        "other=UIParent:CreateWidget('chatwindow','other','UIParent');other:SetExtent(400,180);other:AddAnchor('TOPLEFT',UIParent,700,300);other:Show(true);other:AllowTabSwitch(true)",
    );
    let source = id(&ui, "other");
    lua::chat::hydrate_window(
        &ui.lua,
        source,
        &lua::chat::ChatWindowInfo {
            window_id: 2,
            tabs: vec![
                lua::chat::ChatTabInfo {
                    id: 7,
                    name: "Seven".into(),
                    ..Default::default()
                },
                lua::chat::ChatTabInfo {
                    id: 8,
                    name: "Eight".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        },
    )
    .unwrap();
    run(&ui, "events={}");
    let row = data(&ui, source).tabs[0].clone();
    // Before protected raw0: no detach. Equality at raw0 center inserts after it.
    drag(&ui, row.button, (110., 110.));
    assert_eq!(data(&ui, source).tabs.len(), 2);
    assert!(!eval::<String>(&ui, "table.concat(events,',')").contains("detach"));
    let first = rect(&ui, data(&ui, target).tabs[0].button);
    run(&ui, "events={}");
    drag(
        &ui,
        row.button,
        (first.x + first.w / 2., first.y + first.h / 2.),
    );
    assert_eq!(
        data(&ui, target)
            .tabs
            .iter()
            .map(|t| t.id)
            .collect::<Vec<_>>(),
        vec![0, 7, 8, 2, 5]
    );
    assert!(data(&ui, source).tabs.is_empty());
    let events = eval::<String>(&ui, "table.concat(events,',')");
    assert!(
        events.starts_with("detach:2:7,detach:2:8,select:0:1,"),
        "{events}"
    );
    assert!(events.find("attach:0:7:1").unwrap() < events.find("select:0:2").unwrap());
    assert!(events.ends_with("attach:0:8:2"));
    // Whole-window movement of protected default window must never extract raw0.
    drag(&ui, data(&ui, target).tabs[0].button, (1000., 500.));
    assert_eq!(data(&ui, target).tabs.len(), 5);
}

#[test]
fn callback_failure_continues_without_recreating_tab_objects() {
    let (ui, source) = setup();
    let row = data(&ui, source).tabs[1].clone();
    run(
        &ui,
        "OnDuplicatedChatWindow=function(w,i) duplicate=w;error('callback failure probe') end",
    );
    drag(&ui, row.button, (800., 300.));
    assert_eq!(data(&ui, id(&ui, "duplicate")).tabs, vec![row]);
}

#[test]
fn duplicate_copies_configuration_before_original_creation_callback_and_keeps_input_fresh() {
    let (ui, source) = setup();
    run(
        &ui,
        r#"
        source:SetTabWidth(83);source:SetTabAreaHeight(31);source:SetGap(4)
        source:SetContentOffset(1,2,3,4);source:UseAddTabButton(true)
        OnCreateChatWindow=function(w)
            assert(w:GetWidth()==400 and w:GetHeight()==180)
            record('create');w:SetTabWidth(79)
        end
    "#,
    );
    let row = data(&ui, source).tabs[1].clone();
    drag(&ui, row.button, (800., 300.));
    let c = data(&ui, id(&ui, "duplicate"));
    assert_eq!((c.tab_width, c.tab_area_height, c.tab_gap), (79., 31., 4.));
    assert_eq!(c.content_offset, [1., 2., 3., 4.]);
    assert!(c.use_add_button);
    assert_eq!(c.method, 0);
    assert_eq!(c.whisper, "");
}

#[test]
fn no_hook_allocator_refuses_when_all_eight_window_ids_are_occupied() {
    let (ui, source) = setup();
    run(&ui, "AAUI.ChatAllocateWindow=nil");
    for n in 1..8 {
        run(
            &ui,
            &format!("spare=UIParent:CreateWidget('chatwindow','spare{n}','UIParent')"),
        );
        lua::chat::hydrate_window(
            &ui.lua,
            id(&ui, "spare"),
            &lua::chat::ChatWindowInfo {
                window_id: n,
                tabs: vec![lua::chat::ChatTabInfo {
                    id: 100 + n,
                    name: "Spare".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        )
        .unwrap();
    }
    let rows = data(&ui, source).tabs;
    run(&ui, "events={}");
    drag(&ui, rows[1].button, (800., 300.));
    assert_eq!(windows(&ui).len(), 8);
    assert_eq!(data(&ui, source).tabs, rows);
    assert_eq!(eval::<String>(&ui, "table.concat(events,',')"), "create");
}

#[test]
fn release_lua_callback_can_lock_current_owner_before_internal_drop() {
    let (ui, source) = setup();
    let row = data(&ui, source).tabs[1].clone();
    drag(&ui, row.button, (800., 300.));
    let duplicate = id(&ui, "duplicate");
    expose(&ui, "button", row.button);
    run(
        &ui,
        "button:SetHandler('OnDragStop',function(self) self:GetParent():AllowTabSwitch(false) end);events={}",
    );
    let first = rect(&ui, data(&ui, source).tabs[0].button);
    drag(
        &ui,
        row.button,
        (first.x + first.w * 0.75, first.y + first.h * 0.5),
    );
    assert_eq!(data(&ui, duplicate).tabs, vec![row]);
    assert_eq!(data(&ui, source).tabs.len(), 2);
    assert_eq!(eval::<String>(&ui, "table.concat(events,',')"), "");
}

#[test]
fn retirement_after_allocation_releases_empty_duplicate_without_detaching_source() {
    let (ui, source) = setup();
    let rows = data(&ui, source).tabs;
    ui.lua
        .globals()
        .set(
            "retireDuplicate",
            ui.lua
                .create_function(|lua, t: mlua::Table| {
                    // Use normal hide retirement, including its native manager update.
                    use mlua::ObjectLike;
                    t.call_method::<()>("Show", true)?;
                    t.call_method::<()>("EnableHidingIsRemove", true)?;
                    t.call_method::<()>("Show", false)?;
                    lua::tick(lua, 0.);
                    Ok(())
                })
                .unwrap(),
        )
        .unwrap();
    run(
        &ui,
        "OnDuplicatedChatWindow=function(w,i) retireDuplicate(w) end",
    );
    drag(&ui, rows[1].button, (800., 300.));
    assert_eq!(windows(&ui), vec![source]);
    assert_eq!(data(&ui, source).tabs, rows);
    assert_eq!(
        eval::<String>(&ui, "table.concat(events,',')"),
        "create,allocate,release:1"
    );
}

#[test]
fn target_retirement_after_detach_restores_original_row_and_attached_hook() {
    let (ui, source) = setup();
    let rows = data(&ui, source).tabs;
    ui.lua
        .globals()
        .set(
            "retireDuplicate",
            ui.lua
                .create_function(|lua, (): ()| {
                    use mlua::ObjectLike;
                    let t: mlua::Table = lua.globals().get("duplicate")?;
                    t.call_method::<()>("Show", true)?;
                    t.call_method::<()>("EnableHidingIsRemove", true)?;
                    t.call_method::<()>("Show", false)?;
                    lua::tick(lua, 0.);
                    Ok(())
                })
                .unwrap(),
        )
        .unwrap();
    run(
        &ui,
        "AAUI.ChatTabDetached=function(w,t) record('detach:'..w..':'..t);retireDuplicate() end",
    );
    drag(&ui, rows[1].button, (800., 300.));
    assert_eq!(windows(&ui), vec![source]);
    assert_eq!(data(&ui, source).tabs, rows);
    let events = eval::<String>(&ui, "table.concat(events,',')");
    assert!(events.ends_with("attach:0:2:1,release:1"), "{events}");
    assert_eq!(
        lua::ui(&ui.lua).tree.get(rows[1].message).parent,
        Some(source)
    );
}

#[test]
fn retirement_of_both_owners_keeps_detached_history_for_later_model_rehydration() {
    let (ui, source) = setup();
    let row = data(&ui, source).tabs[1].clone();
    expose(&ui, "message", row.message);
    run(&ui, "message:AddMessage('survives retirement')");
    ui.lua
        .globals()
        .set(
            "retireBoth",
            ui.lua
                .create_function(|lua, (): ()| {
                    lua::chat::reset_windows(lua);
                    Ok(())
                })
                .unwrap(),
        )
        .unwrap();
    run(
        &ui,
        "AAUI.ChatTabDetached=function(w,t) record('detach:'..w..':'..t);retireBoth() end",
    );
    drag(&ui, row.button, (800., 300.));
    assert!(windows(&ui).is_empty());
    assert!(!lua::ui(&ui.lua).tree.get(row.button).alive);
    assert!(!lua::ui(&ui.lua).tree.get(row.message).alive);
    assert!(eval::<String>(&ui, "table.concat(events,',')").ends_with("detach:0:2,release:1"));
    run(
        &ui,
        "restored=UIParent:CreateWidget('chatwindow','restorePending','UIParent')",
    );
    let restored = id(&ui, "restored");
    lua::chat::hydrate_window(
        &ui.lua,
        restored,
        &lua::chat::ChatWindowInfo {
            tabs: vec![lua::chat::ChatTabInfo {
                id: 2,
                name: "Harbor".into(),
                ..Default::default()
            }],
            ..Default::default()
        },
    )
    .unwrap();
    let message = data(&ui, restored).tabs[0].message;
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .widget(message)
            .unwrap()
            .message
            .as_ref()
            .unwrap()
            .lines[0]
            .0,
        "survives retirement"
    );
}
