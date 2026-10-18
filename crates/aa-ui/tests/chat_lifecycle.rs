//! Native configuration operations preserve chat rows, callbacks and history.
#[path = "../examples/native_chat_lifecycle.rs"]
#[allow(dead_code)]
mod original;

use aa_ui::{UiRuntime, input, lua, render};
use lua::chat::{ChatTabInfo, ChatWindowInfo, append_tab, hydrate_window, remove_tab, update_tab};

fn run(ui: &UiRuntime, code: &str) {
    ui.exec("@chat_lifecycle_test", code).unwrap();
}
fn id(ui: &UiRuntime, name: &str) -> u32 {
    lua::value_id(&ui.lua, &ui.lua.load(name).eval().unwrap()).unwrap()
}
fn data(ui: &UiRuntime) -> lua::chat::ChatData {
    lua::ui(&ui.lua)
        .tree
        .widget(id(ui, "window"))
        .unwrap()
        .chat
        .clone()
        .unwrap()
}
fn tab(raw: i64, name: &str) -> ChatTabInfo {
    ChatTabInfo {
        id: raw,
        name: name.into(),
        ..Default::default()
    }
}
fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    run(
        &ui,
        r#"
        log={}
        function OnCreateChatMessage(m) table.insert(log,'message');m:SetTimeVisible(120) end
        function OnCreateChatTabButton(b) table.insert(log,'button:'..b:GetText()) end
        function OnSetChatTabId(b,id) table.insert(log,'id:'..id);b.rawId=id end
        function OnSelectedChatTab(b,on) table.insert(log,'selected:'..b:GetText()..':'..tostring(on)) end
        window=UIParent:CreateWidget('chatwindow','lifecycle','UIParent')
        window:SetExtent(400,180);window:AddAnchor('TOPLEFT',UIParent,100,100);window:Show(true)
    "#,
    );
    ui
}
fn log(ui: &UiRuntime) -> String {
    ui.lua.load("table.concat(log,',')").eval().unwrap()
}

#[test]
fn hydrate_and_append_match_native_callback_order_and_same_id_is_noop() {
    let ui = setup();
    assert_eq!(data(&ui).window_id, -1);
    assert_eq!(data(&ui).selected, usize::MAX);
    let window = id(&ui, "window");
    let info = ChatWindowInfo {
        window_id: 0,
        tabs: vec![tab(0, "All"), tab(2, "Trade")],
        selected: 1,
        method: 4,
    };
    assert!(hydrate_window(&ui.lua, window, &info).unwrap());
    assert_eq!(
        log(&ui),
        "message,button:All,id:0,message,button:Trade,id:2,selected:All:false,selected:Trade:true"
    );
    assert_eq!(data(&ui).selected, 1);
    assert_eq!(data(&ui).method, 4);
    let before = data(&ui).tabs;
    run(&ui, "log={}");
    assert!(!hydrate_window(&ui.lua, window, &info).unwrap());
    assert_eq!(data(&ui).tabs, before);
    assert!(log(&ui).is_empty());
    assert!(append_tab(&ui.lua, window, &tab(5, "Friends")).unwrap());
    assert_eq!(
        log(&ui),
        "message,button:Friends,selected:All:false,selected:Trade:false,selected:Friends:true,id:5"
    );
    assert_eq!(&data(&ui).tabs[..2], &before);
    // A physical click on the already-selected tab does not repeat selection callbacks.
    run(&ui, "log={}");
    click(&ui, data(&ui).tabs[2].button);
    assert!(log(&ui).is_empty());
}

#[test]
fn invalid_stored_indices_leave_no_selected_row_and_empty_hydration_has_no_fallback() {
    for selected in [-1, 3, i32::MAX] {
        let ui = setup();
        assert!(
            hydrate_window(
                &ui.lua,
                id(&ui, "window"),
                &ChatWindowInfo {
                    tabs: vec![tab(2, "Trade")],
                    selected,
                    ..Default::default()
                }
            )
            .unwrap()
        );
        assert_eq!(data(&ui).selected, usize::MAX);
        assert!(!log(&ui).contains("selected:"));
        let message = data(&ui).tabs[0].message;
        assert!(!lua::ui(&ui.lua).tree.get(message).shown);
        assert!(remove_tab(&ui.lua, 2).unwrap());
        assert_eq!(data(&ui).selected, usize::MAX);
    }
    let ui = setup();
    let window = id(&ui, "window");
    hydrate_window(&ui.lua, window, &ChatWindowInfo::default()).unwrap();
    assert!(data(&ui).tabs.is_empty());
    assert!(lua::ui(&ui.lua).tree.get(window).alive);
    lua::tick(&ui.lua, 0.);
    assert!(!lua::ui(&ui.lua).tree.get(window).alive);
}

#[test]
fn metadata_update_keeps_objects_handlers_history_scroll_and_recolors_old_links() {
    let ui = setup();
    let window = id(&ui, "window");
    let mut info = tab(0, "All");
    info.font_size = Some(17.);
    info.background = Some([0.2, 0.3, 0.4, 0.6]);
    info.filter_colors = vec![(2, [1., 0., 0., 1.])];
    hydrate_window(
        &ui.lua,
        window,
        &ChatWindowInfo {
            tabs: vec![info.clone()],
            ..Default::default()
        },
    )
    .unwrap();
    let row = data(&ui).tabs[0].clone();
    ui.lua
        .globals()
        .set(
            "message",
            lua::object_table(&ui.lua, row.message).unwrap().unwrap(),
        )
        .unwrap();
    ui.lua
        .globals()
        .set(
            "button",
            lua::object_table(&ui.lua, row.button).unwrap().unwrap(),
        )
        .unwrap();
    run(
        &ui,
        r#"
        message:SetHandler('OnContentUpdated',function() customHandler=true end)
        X2Chat:DispatchChatMessage(2,'old |kLiora,0; link |cFF0000FFblue|r')
        message:AddMessage('unfiltered text')
    "#,
    );
    let before = lua::ui(&ui.lua)
        .tree
        .widget(row.message)
        .unwrap()
        .message
        .clone()
        .unwrap();
    let red = render::build_quads(&mut lua::ui(&ui.lua), None);
    assert!(red.iter().any(|q| q.color == [1., 0., 0., 1.]));
    info.name = "Renamed".into();
    info.font_size = Some(20.);
    info.background = Some([0.3, 0.2, 0.1, 0.8]);
    info.filter_colors = vec![(2, [0., 1., 0., 1.])];
    run(&ui, "log={}");
    assert!(update_tab(&ui.lua, 0, &info).unwrap());
    assert!(log(&ui).is_empty());
    assert_eq!(data(&ui).tabs[0].button, row.button);
    assert_eq!(data(&ui).tabs[0].message, row.message);
    let after = lua::ui(&ui.lua)
        .tree
        .widget(row.message)
        .unwrap()
        .message
        .clone()
        .unwrap();
    assert_eq!(after.lines, before.lines);
    assert_eq!(after.scroll, before.scroll);
    let text = lua::ui(&ui.lua)
        .tree
        .get(row.message)
        .text()
        .unwrap()
        .clone();
    assert_eq!(text.style.size, 20.);
    let green = render::build_quads(&mut lua::ui(&ui.lua), None);
    assert!(green.iter().any(|q| q.color == [0., 1., 0., 1.]));
    assert!(green.iter().any(|q| q.color == [0., 0., 1., 1.]));
    assert!(!green.iter().any(|q| q.color == [1., 0., 0., 1.]));
    lua::call_handler(
        &ui.lua,
        row.message,
        "OnContentUpdated",
        vec![mlua::Value::String(
            ui.lua.create_string("read_chat_message").unwrap(),
        )],
    );
    run(&ui, "assert(customHandler)");
    let left_line = data(&ui).left_line;
    assert_eq!(
        lua::ui(&ui.lua).tree.get(row.button).text().unwrap().text,
        "Renamed"
    );
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .get(left_line)
            .drawable()
            .unwrap()
            .color,
        [1., 1., 1., 0.8]
    );
    assert!(!update_tab(&ui.lua, 1, &info).unwrap());
}

#[test]
fn keyed_native_filters_legacy_filters_and_native_filter56_gate_coexist() {
    let ui = setup();
    run(
        &ui,
        r#"
        AAUI={ChatTabs=function() return {
            {id=0,name='Native',filters={[2]=true,[5]=true,[56]=false},fontSize=19,
             bgColor={r=.1,g=.2,b=.3,a=.4},filterColors={[56]={r=0,g=1,b=0,a=1}}},
            {id=1,name='Legacy',filters={3}},
            {id=2,name='Empty',filters={}}
        } end}
        window:SetChatWindowId(0)
        X2Chat:DispatchChatMessage(2,'say');X2Chat:DispatchChatMessage(3,'whisper')
        X2Chat:DispatchChatMessage(56,'raid-system')
    "#,
    );
    let c = data(&ui);
    let counts: Vec<_> = c
        .tabs
        .iter()
        .map(|t| {
            lua::ui(&ui.lua)
                .tree
                .widget(t.message)
                .unwrap()
                .message
                .as_ref()
                .unwrap()
                .lines
                .len()
        })
        .collect();
    assert_eq!(counts, [2, 1, 0]);
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .get(c.tabs[0].message)
            .text()
            .unwrap()
            .style
            .size,
        19.
    );
    let q = render::build_quads(&mut lua::ui(&ui.lua), None);
    assert!(q.iter().any(|q| q.color == [0., 1., 0., 1.]));
    run(
        &ui,
        "log={};AAUI.ChatTabs=function() error('same-ID must not call hook') end;window:SetChatWindowId(0)",
    );
    assert!(log(&ui).is_empty());
}

#[test]
fn remove_retains_numeric_selection_without_callbacks_and_retires_all_descendants() {
    let ui = setup();
    let window = id(&ui, "window");
    hydrate_window(
        &ui.lua,
        window,
        &ChatWindowInfo {
            tabs: vec![tab(0, "All"), tab(2, "Trade"), tab(5, "Other")],
            selected: 1,
            ..Default::default()
        },
    )
    .unwrap();
    let before = data(&ui);
    let row = before.tabs[0].clone();
    ui.lua
        .globals()
        .set(
            "message",
            lua::object_table(&ui.lua, row.message).unwrap().unwrap(),
        )
        .unwrap();
    ui.lua
        .globals()
        .set(
            "button",
            lua::object_table(&ui.lua, row.button).unwrap().unwrap(),
        )
        .unwrap();
    run(
        &ui,
        r#"
        staleEvents,staleFrames,staleLost=0,0,0
        child=button:CreateChildWidget('editbox','focusedChild',0,true)
        child:SetExtent(100,30);child:Show(true);child:SetFocus()
        child:RegisterEvent('CHAT_LIFETIME_PROBE')
        child:SetHandler('OnEvent',function() staleEvents=staleEvents+1 end)
        child:SetHandler('OnUpdate',function() staleFrames=staleFrames+1 end)
        child:SetHandler('OnReleaseFocus',function() staleLost=staleLost+1 end)
        log={}
    "#,
    );
    let child = id(&ui, "child");
    assert_eq!(ui.focus(), Some(child));
    assert!(remove_tab(&ui.lua, 0).unwrap());
    assert!(log(&ui).is_empty());
    assert_eq!(data(&ui).selected, 1); // numeric1 now selects raw5; do not retain raw2.
    assert_eq!(data(&ui).tabs[1].id, 5);
    assert_eq!(ui.focus(), None);
    for target in [child, row.message, row.button] {
        assert!(!lua::ui(&ui.lua).tree.get(target).alive);
    }
    lua::fire_ui_event(&ui.lua, "CHAT_LIFETIME_PROBE", vec![]);
    lua::tick(&ui.lua, 0.);
    run(
        &ui,
        "assert(staleEvents==0 and staleFrames==0 and staleLost==1)",
    );
    assert!(!remove_tab(&ui.lua, 99).unwrap());
    assert!(remove_tab(&ui.lua, 5).unwrap());
    assert_eq!(data(&ui).selected, 0);
    assert!(remove_tab(&ui.lua, 2).unwrap());
    assert_eq!(data(&ui).selected, usize::MAX);
    assert!(lua::ui(&ui.lua).tree.get(window).alive);
    lua::tick(&ui.lua, 0.);
    assert!(!lua::ui(&ui.lua).tree.get(window).alive);
}

#[test]
fn hydration_can_restore_global_history_but_new_id_allocation_starts_empty() {
    let ui = setup();
    let window = id(&ui, "window");
    let all = tab(0, "All");
    hydrate_window(
        &ui.lua,
        window,
        &ChatWindowInfo {
            tabs: vec![all.clone()],
            ..Default::default()
        },
    )
    .unwrap();
    run(&ui, "X2Chat:DispatchChatMessage(2,'retained')");
    hydrate_window(
        &ui.lua,
        window,
        &ChatWindowInfo {
            window_id: 1,
            tabs: vec![tab(2, "Trade")],
            ..Default::default()
        },
    )
    .unwrap();
    hydrate_window(
        &ui.lua,
        window,
        &ChatWindowInfo {
            tabs: vec![all],
            ..Default::default()
        },
    )
    .unwrap();
    let message = data(&ui).tabs[0].message;
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .widget(message)
            .unwrap()
            .message
            .as_ref()
            .unwrap()
            .lines
            .len(),
        1
    );
    assert!(remove_tab(&ui.lua, 0).unwrap());
    assert!(append_tab(&ui.lua, window, &tab(0, "New")).unwrap());
    let message = data(&ui).tabs[0].message;
    assert!(
        lua::ui(&ui.lua)
            .tree
            .widget(message)
            .unwrap()
            .message
            .as_ref()
            .unwrap()
            .lines
            .is_empty()
    );
}

fn click(ui: &UiRuntime, button: u32) {
    let r = lua::ui(&ui.lua).tree.rect(button);
    let p = (r.x + r.w * 0.5, r.y + r.h * 0.5);
    assert_eq!(input::pick(&ui.lua, p.0, p.1), Some(button));
    let mut mouse = Default::default();
    input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.);
    input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.);
}

#[test]
fn original_chat_add_update_remove_keeps_script_handlers_tooltip_and_physical_selection() {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    let window = original::install(&mut ui);
    let c = lua::ui(&ui.lua)
        .tree
        .widget(window)
        .unwrap()
        .chat
        .clone()
        .unwrap();
    assert_eq!(c.tabs.len(), 2);
    assert_eq!(c.tabs[1].id, 5);
    assert_eq!(c.selected, 1);
    run(&ui,"oldTooltip=SetTooltip;SetTooltip=function(text,owner) capturedTip=text;return oldTooltip(text,owner) end");
    click(&ui, c.tabs[0].button);
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .widget(window)
            .unwrap()
            .chat
            .as_ref()
            .unwrap()
            .selected,
        0
    );
    click(&ui, c.tabs[1].button);
    run(&ui,"assert(capturedTip=='Harbor');HideTooltip();nativeChatNames[5]='Neighbors'");
    let updated=ChatTabInfo {id:5,name:"Neighbors".into(),filters:Some(vec![2,4]),..Default::default()};
    assert!(update_tab(&ui.lua,5,&updated).unwrap());
    render::build_quads(&mut lua::ui(&ui.lua), None);
    let text=lua::ui(&ui.lua).tree.get(c.tabs[1].button).text().unwrap().clone();
    assert_eq!(text.text,"Neighbors");
    assert_eq!(text.auto_clip.count,9);
    click(&ui,c.tabs[0].button);click(&ui,c.tabs[1].button);
    run(&ui,"assert(capturedTip=='Neighbors')");
    run(&ui, "HideTooltip()");
    render::build_quads(&mut lua::ui(&ui.lua), None);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
}
