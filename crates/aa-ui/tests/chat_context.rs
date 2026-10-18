//! Actual native context dispatch order and the unchanged original chat popup.
#[path = "../examples/native_chat_alpha.rs"]
#[allow(dead_code)]
mod original;
use aa_ui::{
    UiRuntime,
    input::{self, MouseButtons, MouseState},
    lua,
    tree::Id,
};

fn run(ui: &UiRuntime, code: &str) {
    ui.exec("@chat_context", code).unwrap();
}
fn eval<T: mlua::FromLuaMulti>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().unwrap()
}
fn id(ui: &UiRuntime, code: &str) -> Id {
    lua::value_id(&ui.lua, &eval(ui, code)).unwrap()
}
fn tab(ui: &UiRuntime, window: Id, index: usize) -> (Id, Id) {
    let u = lua::ui(&ui.lua);
    let t = &u.tree.widget(window).unwrap().chat.as_ref().unwrap().tabs[index];
    (t.button, t.message)
}
fn point(ui: &UiRuntime, widget: Id) -> (f32, f32) {
    let u = lua::ui(&ui.lua);
    let r = u.animations.widget_rect(&u.tree, widget);
    let p = (r.x + r.w / 2., r.y + r.h / 2.);
    drop(u);
    assert_eq!(input::pick(&ui.lua, p.0, p.1), Some(widget));
    p
}
fn edge(ui: &UiRuntime, mouse: &mut MouseState, p: (f32, f32), buttons: MouseButtons) {
    input::update_buttons(&ui.lua, mouse, Some(p), buttons, 0.);
    lua::tick(&ui.lua, 16.);
}
fn right(ui: &UiRuntime, mouse: &mut MouseState, p: (f32, f32)) {
    edge(
        ui,
        mouse,
        p,
        MouseButtons {
            right_press: true,
            ..Default::default()
        },
    );
    edge(
        ui,
        mouse,
        p,
        MouseButtons {
            right_release: true,
            ..Default::default()
        },
    );
}
fn setup() -> (UiRuntime, Id) {
    let ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    run(
        &ui,
        r#"
        log={};AAUI={ChatTabs=function() return {{id=0,name='All'},{id=3,name='Crew'}} end,
            ChatSelection=function() table.insert(log,'selection') end}
        function OnCreateChatTabButton(button)
            button:SetHandler('OnClick',function(self,b) table.insert(log,'click:'..b) end)
        end
        function OnContextChatTab(message,button,rawId)
            assert(message==expectedMessage and button==expectedButton)
            table.insert(log,'context:'..rawId)
        end
        UIParent:SetEventHandler('SHOW_CHAT_TAB_CONTEXT',function(button,rawId)
            assert(button==expectedButton);table.insert(log,'event:'..rawId)
        end)
        window=UIParent:CreateWidget('chatwindow','contextChat','UIParent')
        window:SetExtent(400,180);window:AddAnchor('TOPLEFT',UIParent,100,100);window:Show(true)
        window:SetChatWindowId(0)
        log={}
    "#,
    );
    let window = id(&ui, "window");
    let (button, message) = tab(&ui, window, 1);
    ui.lua
        .globals()
        .set(
            "expectedButton",
            lua::object_table(&ui.lua, button).unwrap().unwrap(),
        )
        .unwrap();
    ui.lua
        .globals()
        .set(
            "expectedMessage",
            lua::object_table(&ui.lua, message).unwrap().unwrap(),
        )
        .unwrap();
    (ui, window)
}

#[test]
fn physical_context_on_unselected_tab_emits_event_then_callback_without_selection() {
    let (ui, window) = setup();
    let (button, _) = tab(&ui, window, 1);
    let p = point(&ui, button);
    let mut mouse = MouseState::default();
    edge(
        &ui,
        &mut mouse,
        p,
        MouseButtons {
            right_press: true,
            ..Default::default()
        },
    );
    assert_eq!(eval::<String>(&ui, "table.concat(log,',')"), "");
    edge(
        &ui,
        &mut mouse,
        p,
        MouseButtons {
            right_release: true,
            ..Default::default()
        },
    );
    assert_eq!(
        eval::<String>(&ui, "table.concat(log,',')"),
        "click:RightButton,event:3,context:3"
    );
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
    right(&ui, &mut mouse, p);
    assert_eq!(eval::<usize>(&ui, "#log"), 6);
    edge(
        &ui,
        &mut mouse,
        p,
        MouseButtons {
            press: true,
            ..Default::default()
        },
    );
    edge(
        &ui,
        &mut mouse,
        p,
        MouseButtons {
            release: true,
            ..Default::default()
        },
    );
    assert_eq!(eval::<String>(&ui, "log[#log]"), "selection");
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
}

#[test]
fn context_event_precedes_fresh_raw_id_read_and_absent_global_does_not_suppress_event() {
    let (ui, window) = setup();
    ui.lua
        .globals()
        .set(
            "changeContextId",
            ui.lua
                .create_function(move |lua, (): ()| {
                    lua::ui(lua)
                        .tree
                        .widget_mut(window)
                        .unwrap()
                        .chat
                        .as_mut()
                        .unwrap()
                        .tabs[1]
                        .id = 7;
                    Ok(())
                })
                .unwrap(),
        )
        .unwrap();
    run(
        &ui,
        "UIParent:SetEventHandler('SHOW_CHAT_TAB_CONTEXT',changeContextId)",
    );
    let (button, _) = tab(&ui, window, 1);
    let mut mouse = MouseState::default();
    right(&ui, &mut mouse, point(&ui, button));
    assert_eq!(
        eval::<String>(&ui, "table.concat(log,',')"),
        "click:RightButton,event:3,context:7"
    );
    run(&ui, "OnContextChatTab=nil;log={}");
    right(&ui, &mut mouse, point(&ui, button));
    assert_eq!(
        eval::<String>(&ui, "table.concat(log,',')"),
        "click:RightButton,event:7"
    );
    run(&ui, "log={}");
    right(&ui, &mut mouse, (300., 220.));
    assert_eq!(eval::<usize>(&ui, "#log"), 0);
}

#[test]
fn original_context_popup_freezes_alpha_and_outside_click_retires_it_and_restores_state() {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    let window = original::install(&mut ui);
    let (button, message) = tab(&ui, window, 0);
    ui.lua
        .globals()
        .set(
            "contextButton",
            lua::object_table(&ui.lua, button).unwrap().unwrap(),
        )
        .unwrap();
    let mut mouse = MouseState::default();
    right(&ui, &mut mouse, point(&ui, button));
    assert!(eval::<bool>(
        &ui,
        "contextButton:GetAttachedWidget():IsVisible()"
    ));
    let popup = id(&ui, "contextButton:GetAttachedWidget()");
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .widget(window)
            .unwrap()
            .dynamic_drawable
            .enabled[..3],
        [false; 3]
    );
    input::update_buttons(&ui.lua, &mut mouse, Some((0., 0.)), Default::default(), 0.);
    lua::tick(&ui.lua, 16.);
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .widget(message)
            .unwrap()
            .dynamic_drawable
            .current[..3],
        [0.7; 3]
    );
    edge(
        &ui,
        &mut mouse,
        (0., 0.),
        MouseButtons {
            press: true,
            ..Default::default()
        },
    );
    assert!(!lua::ui(&ui.lua).tree.get(popup).alive);
    assert!(eval::<bool>(&ui, "contextButton:GetAttachedWidget()==nil"));
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .widget(window)
            .unwrap()
            .dynamic_drawable
            .enabled[..3],
        [true; 3]
    );
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .widget(message)
            .unwrap()
            .dynamic_drawable
            .current[..3],
        [0.5; 3]
    );
    right(&ui, &mut mouse, point(&ui, button));
    assert!(eval::<bool>(
        &ui,
        "contextButton:GetAttachedWidget():IsVisible()"
    ));
    run(
        &ui,
        "clearContextId=nil;X2Chat.ClearChatContentByUser=function(self,rawId) clearContextId=rawId end",
    );
    let clear = id(&ui, "contextButton:GetAttachedWidget().buttons[2]");
    let p = point(&ui, clear);
    edge(
        &ui,
        &mut mouse,
        p,
        MouseButtons {
            press: true,
            ..Default::default()
        },
    );
    edge(
        &ui,
        &mut mouse,
        p,
        MouseButtons {
            release: true,
            ..Default::default()
        },
    );
    assert_eq!(eval::<i64>(&ui, "clearContextId"), 0);
    assert!(eval::<bool>(&ui, "contextButton:GetAttachedWidget()==nil"));
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .widget(window)
            .unwrap()
            .dynamic_drawable
            .enabled[..3],
        [true; 3]
    );
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
}
