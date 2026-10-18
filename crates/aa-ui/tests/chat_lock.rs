//! Actual native drag-start lock policy; simple selection and context stay available.
#[path = "../examples/native_chat_lock.rs"]
#[allow(dead_code)]
mod original;
use aa_ui::{
    UiRuntime,
    input::{self, MouseButtons, MouseState},
    lua,
    tree::{Id, Rect},
};

fn run(ui: &UiRuntime, code: &str) {
    ui.exec("@chat_lock", code).unwrap();
}
fn eval<T: mlua::FromLuaMulti>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().unwrap()
}
fn id(ui: &UiRuntime, code: &str) -> Id {
    lua::value_id(&ui.lua, &eval(ui, code)).unwrap()
}
fn setup() -> (UiRuntime, Id) {
    let ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    run(
        &ui,
        r#"
        log={};AAUI={ChatTabs=function() return {{id=0,name='All'},{id=3,name='Crew'}} end,
            ChatSelection=function() table.insert(log,'selection') end}
        function OnContextChatTab() table.insert(log,'context') end
        window=UIParent:CreateWidget('chatwindow','lockChat','UIParent')
        window:SetExtent(400,180);window:AddAnchor('TOPLEFT',UIParent,100,100);window:Show(true)
        window:SetChatWindowId(0);log={}
        window:SetMaxNotifyTime(3000);window:SetNotifyBlinkingFreq(400)
    "#,
    );
    let window = id(&ui, "window");
    (ui, window)
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
fn rect(ui: &UiRuntime, widget: Id) -> Rect {
    lua::ui(&ui.lua).tree.rect(widget)
}
fn point(ui: &UiRuntime, widget: Id) -> (f32, f32) {
    let r = rect(ui, widget);
    let p = (r.x + r.w / 2., r.y + r.h / 2.);
    assert_eq!(input::pick(&ui.lua, p.0, p.1), Some(widget));
    p
}
fn edge(ui: &UiRuntime, mouse: &mut MouseState, p: (f32, f32), buttons: MouseButtons) {
    input::update_buttons(&ui.lua, mouse, Some(p), buttons, 0.);
}
fn press(ui: &UiRuntime, mouse: &mut MouseState, p: (f32, f32)) {
    edge(
        ui,
        mouse,
        p,
        MouseButtons {
            press: true,
            ..Default::default()
        },
    );
}
fn motion(ui: &UiRuntime, mouse: &mut MouseState, p: (f32, f32)) {
    edge(ui, mouse, p, MouseButtons::default());
}
fn release(ui: &UiRuntime, mouse: &mut MouseState, p: (f32, f32)) {
    edge(
        ui,
        mouse,
        p,
        MouseButtons {
            release: true,
            ..Default::default()
        },
    );
}

#[test]
fn constructor_and_strict_setters_match_native_types_and_f32_edges() {
    let ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    run(
        &ui,
        "window=UIParent:CreateWidget('chatwindow','defaults','UIParent')",
    );
    let window = id(&ui, "window");
    let c = data(&ui, window);
    assert!(!c.allow_tab_switch);
    assert_eq!(
        (c.notify_blink_ms, c.notify_max_ms, c.notify_remaining_ms),
        (500., 2000., 0.)
    );
    run(
        &ui,
        "window:AllowTabSwitch(true);window:AllowTabSwitch();window:AllowTabSwitch(0);window:AllowTabSwitch('false');window:SetMaxNotifyTime('3000');window:SetNotifyBlinkingFreq(false)",
    );
    assert!(data(&ui, window).allow_tab_switch);
    assert_eq!(data(&ui, window).notify_max_ms, 2000.);
    assert_eq!(data(&ui, window).notify_blink_ms, 500.);
    run(
        &ui,
        "window:AllowTabSwitch(false);window:SetMaxNotifyTime(-9);window:SetNotifyBlinkingFreq(-0.0)",
    );
    let c = data(&ui, window);
    assert!(!c.allow_tab_switch);
    assert_eq!(c.notify_max_ms, 0.);
    assert!(c.notify_blink_ms.is_sign_negative());
    run(
        &ui,
        "window:SetMaxNotifyTime(0/0);window:SetNotifyBlinkingFreq(math.huge)",
    );
    assert!(data(&ui, window).notify_max_ms.is_nan());
    assert_eq!(data(&ui, window).notify_blink_ms, f32::INFINITY);
    assert!(!lua::chat::set_allow_tab_switch(&ui.lua, 0, true));
}

#[test]
fn locked_simple_click_and_context_do_not_arm_notification_or_block_selection() {
    let (ui, w) = setup();
    let button = data(&ui, w).tabs[1].button;
    let p = point(&ui, button);
    let mut mouse = MouseState::default();
    press(&ui, &mut mouse, p);
    motion(&ui, &mut mouse, (p.0 + 1., p.1));
    release(&ui, &mut mouse, (p.0 + 1., p.1));
    assert_eq!(data(&ui, w).selected, 1);
    assert_eq!(data(&ui, w).notify_remaining_ms, 0.);
    edge(
        &ui,
        &mut mouse,
        p,
        MouseButtons {
            right_press: true,
            ..Default::default()
        },
    );
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
        "selection,context"
    );
    assert_eq!(data(&ui, w).notify_remaining_ms, 0.);
}

#[test]
fn locked_drag_arms_after_lua_and_blinks_with_native_expiry_and_zero_interval() {
    let (ui, w) = setup();
    let c = data(&ui, w);
    let button = c.tabs[0].button;
    let before = rect(&ui, w);
    ui.lua
        .globals()
        .set(
            "button",
            lua::object_table(&ui.lua, button).unwrap().unwrap(),
        )
        .unwrap();
    run(
        &ui,
        "button:SetHandler('OnDragStart',function() beforeArm=window:GetLockNotifyDrawable():IsVisible();table.insert(log,'drag') end)",
    );
    let p = point(&ui, button);
    let mut mouse = MouseState::default();
    press(&ui, &mut mouse, p);
    motion(&ui, &mut mouse, (p.0 + 5., p.1));
    assert_eq!(eval::<String>(&ui, "table.concat(log,',')"), "drag");
    assert!(!eval::<bool>(&ui, "beforeArm"));
    assert_eq!(data(&ui, w).notify_remaining_ms, 3000.);
    motion(&ui, &mut mouse, (p.0 + 90., p.1 + 40.));
    release(&ui, &mut mouse, (p.0 + 90., p.1 + 40.));
    assert_eq!(rect(&ui, w), before);
    lua::tick(&ui.lua, 400.);
    assert!(lua::ui(&ui.lua).tree.get(c.lock_notify).shown);
    assert_eq!(data(&ui, w).notify_remaining_ms, 2600.);
    lua::tick(&ui.lua, 400.);
    assert!(!lua::ui(&ui.lua).tree.get(c.lock_notify).shown);
    lua::tick(&ui.lua, 2201.);
    assert_eq!(data(&ui, w).notify_remaining_ms, -1.);
    assert!(!lua::ui(&ui.lua).tree.get(c.lock_notify).shown);
    run(&ui, "window:SetNotifyBlinkingFreq(0)");
    let p = point(&ui, button);
    press(&ui, &mut mouse, p);
    motion(&ui, &mut mouse, (p.0 + 5., p.1));
    release(&ui, &mut mouse, (p.0 + 5., p.1));
    lua::tick(&ui.lua, 400.);
    assert!(!lua::ui(&ui.lua).tree.get(c.lock_notify).shown);
}

#[test]
fn callback_unlock_moves_first_tab_and_locking_does_not_cancel_existing_capture() {
    let (ui, w) = setup();
    let button = data(&ui, w).tabs[0].button;
    ui.lua
        .globals()
        .set(
            "button",
            lua::object_table(&ui.lua, button).unwrap().unwrap(),
        )
        .unwrap();
    run(
        &ui,
        "button:SetHandler('OnDragStart',function() window:AllowTabSwitch(true) end)",
    );
    let p = point(&ui, button);
    let r = rect(&ui, w);
    let mut mouse = MouseState::default();
    press(&ui, &mut mouse, p);
    motion(&ui, &mut mouse, (p.0 + 5., p.1));
    run(&ui, "window:AllowTabSwitch(false)");
    motion(&ui, &mut mouse, (p.0 + 45., p.1 + 20.));
    assert_eq!(rect(&ui, w).x, r.x + 40.);
    assert_eq!(rect(&ui, w).y, r.y + 20.);
    release(&ui, &mut mouse, (p.0 + 45., p.1 + 20.));
    let r = rect(&ui, w);
    motion(&ui, &mut mouse, (p.0 + 100., p.1 + 80.));
    assert_eq!(rect(&ui, w), r);
    assert_eq!(data(&ui, w).notify_remaining_ms, 0.);
    assert_eq!(eval::<String>(&ui, "table.concat(log,',')"), "");
}

#[test]
fn secondary_split_is_not_substituted_with_whole_window_moving_and_removed_source_is_safe() {
    let (ui, w) = setup();
    lua::chat::set_allow_tab_switch(&ui.lua, w, true);
    let button = data(&ui, w).tabs[1].button;
    let r = rect(&ui, w);
    let p = point(&ui, button);
    let mut mouse = MouseState::default();
    press(&ui, &mut mouse, p);
    motion(&ui, &mut mouse, (p.0 + 5., p.1));
    motion(&ui, &mut mouse, (p.0 + 80., p.1 + 40.));
    release(&ui, &mut mouse, (p.0 + 80., p.1 + 40.));
    assert_eq!(rect(&ui, w), r);
    ui.lua
        .globals()
        .set(
            "button",
            lua::object_table(&ui.lua, button).unwrap().unwrap(),
        )
        .unwrap();
    ui.lua
        .globals()
        .set(
            "removeSource",
            ui.lua
                .create_function(|lua, (): ()| {
                    lua::chat::remove_tab(lua, 3)?;
                    Ok(())
                })
                .unwrap(),
        )
        .unwrap();
    run(&ui, "button:SetHandler('OnDragStart',removeSource)");
    let p = point(&ui, button);
    press(&ui, &mut mouse, p);
    motion(&ui, &mut mouse, (p.0 + 5., p.1));
    release(&ui, &mut mouse, (p.0 + 5., p.1));
    assert!(!lua::ui(&ui.lua).tree.get(button).alive);
}

#[test]
fn enabling_clears_notification_but_disabling_and_timer_setters_retain_live_state() {
    let (ui, w) = setup();
    let c = data(&ui, w);
    let p = point(&ui, c.tabs[0].button);
    let mut mouse = MouseState::default();
    press(&ui, &mut mouse, p);
    motion(&ui, &mut mouse, (p.0 + 5., p.1));
    release(&ui, &mut mouse, (p.0 + 5., p.1));
    lua::tick(&ui.lua, 400.);
    run(
        &ui,
        "window:AllowTabSwitch(false);window:SetMaxNotifyTime(10);window:SetNotifyBlinkingFreq(200)",
    );
    assert_eq!(data(&ui, w).notify_remaining_ms, 2600.);
    assert!(lua::ui(&ui.lua).tree.get(c.lock_notify).shown);
    assert!(lua::chat::set_allow_tab_switch(&ui.lua, w, true));
    assert_eq!(data(&ui, w).notify_remaining_ms, 0.);
    assert!(!lua::ui(&ui.lua).tree.get(c.lock_notify).shown);
}

#[test]
fn original_lock_art_and_physical_drag_use_source_geometry_and_script_timers() {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    let w = original::install(&mut ui);
    let c = data(&ui, w);
    assert_eq!((c.notify_max_ms, c.notify_blink_ms), (3000., 400.));
    assert!(!c.allow_tab_switch);
    original::drag_attempt(&ui, w);
    lua::tick(&ui.lua, 450.);
    let r = rect(&ui, w);
    let n = rect(&ui, c.lock_notify);
    assert!(lua::ui(&ui.lua).tree.get(c.lock_notify).shown);
    assert_eq!(n.x, r.x);
    assert_eq!(n.y + n.h, r.y);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
}
