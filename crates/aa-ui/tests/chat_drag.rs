//! Physical-input contracts recovered from native chat drag frame/release paths.
#[path = "../examples/native_chat_split.rs"]
#[allow(dead_code)]
mod original;

use aa_ui::{
    UiRuntime, input, lua,
    tree::{Id, Rect},
};

fn run(ui: &UiRuntime, code: &str) {
    ui.exec("@chat_drag_test", code).unwrap();
}
fn eval<T: mlua::FromLuaMulti>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().unwrap()
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
fn data(ui: &UiRuntime, id: Id) -> lua::chat::ChatData {
    lua::ui(&ui.lua)
        .tree
        .widget(id)
        .unwrap()
        .chat
        .clone()
        .unwrap()
}
fn rect(ui: &UiRuntime, id: Id) -> Rect {
    lua::ui(&ui.lua).tree.rect(id)
}
fn center(ui: &UiRuntime, id: Id) -> (f32, f32) {
    let r = rect(ui, id);
    (r.x + r.w / 2., r.y + r.h / 2.)
}
fn pointer(ui: &UiRuntime, mouse: &mut input::MouseState, p: Option<(f32, f32)>) {
    input::update(&ui.lua, mouse, p, false, false, false, 0.);
    lua::tick(&ui.lua, 0.);
}
fn begin(ui: &UiRuntime, mouse: &mut input::MouseState, button: Id) {
    let p = center(ui, button);
    input::update(&ui.lua, mouse, Some(p), true, false, false, 0.);
    input::update(
        &ui.lua,
        mouse,
        Some((p.0 + 5., p.1)),
        false,
        false,
        false,
        0.,
    );
    lua::tick(&ui.lua, 0.);
}
fn setup() -> (UiRuntime, Id) {
    let ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    run(
        &ui,
        r#"
        events={};function record(s) table.insert(events,s) end
        AAUI={ChatTabs=function() return {{id=7,name='Seven'},{id=8,name='Eight'},{id=9,name='Nine'}} end,
            ChatSelection=function(w,i) record('select:'..i) end,
            ChatTabAttached=function(w,t,i) record('attach:'..t..':'..i) end,
            ChatTabDetached=function(w,t) record('detach:'..t) end,
            ChatMethod=function(t,m) record('method:'..t..':'..m) end}
        window=UIParent:CreateWidget('chatwindow','dragTest','UIParent')
        window:SetExtent(400,180);window:AddAnchor('TOPLEFT',UIParent,100,100)
        window:SetChatWindowId(0);window:AllowTabSwitch(true)
        window:SetCaretOffset(0,5);window:GetCaretDrawable():SetExtent(19,14)
        window:Show(true);events={}
    "#,
    );
    let window = id(&ui, "window");
    lua::tick(&ui.lua, 0.);
    (ui, window)
}

#[test]
fn original_hover_caret_uses_authored_art_never_mutates_rows_and_clears_on_release() {
    let mut ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    let source = original::install(&mut ui);
    let row = data(&ui, source).tabs[1].clone();
    let history = lua::ui(&ui.lua)
        .tree
        .widget(row.message)
        .unwrap()
        .message
        .clone()
        .unwrap();
    let mut mouse = Default::default();
    begin(&ui, &mut mouse, row.button);
    let duplicate = lua::ui(&ui.lua).tree.get(row.button).parent.unwrap();
    let target = data(&ui, source).tabs[0].button;
    let r = rect(&ui, target);
    pointer(&ui, &mut mouse, Some((r.x + r.w * 0.75, r.y + r.h / 2.)));
    expose(&ui, "source", source);
    let caret = id(&ui, "source:GetCaretDrawable()");
    assert!(lua::ui(&ui.lua).tree.get(caret).shown);
    let cr = rect(&ui, caret);
    assert_eq!(
        (cr.x, cr.y, cr.w, cr.h),
        (r.right() - 9.5, r.y - 2., 19., 14.)
    );
    assert_eq!(data(&ui, source).tabs.len(), 1);
    assert_eq!(data(&ui, duplicate).tabs, vec![row.clone()]);
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .widget(row.message)
            .unwrap()
            .message
            .as_ref()
            .unwrap()
            .lines,
        history.lines
    );
    pointer(&ui, &mut mouse, Some((r.x + 1., r.y + r.h / 2.)));
    assert!(!lua::ui(&ui.lua).tree.get(caret).shown); // Before raw0 is protected.
    assert_eq!(rect(&ui, caret), cr); // Native hiding retains its authored anchor.
    pointer(&ui, &mut mouse, Some((r.x + r.w / 2., r.y + r.h / 2.)));
    assert!(lua::ui(&ui.lua).tree.get(caret).shown); // Exact center is RIGHT.
    input::update(&ui.lua, &mut mouse, None, false, true, false, 0.);
    assert!(!lua::ui(&ui.lua).tree.get(caret).shown);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
}

#[test]
fn preview_exit_reentry_locked_target_cancel_and_retirement_are_bounded() {
    let (ui, window) = setup();
    run(
        &ui,
        "other=UIParent:CreateWidget('chatwindow','other','UIParent');other:SetExtent(300,180);other:AddAnchor('TOPLEFT',UIParent,700,100);other:SetChatWindowId(1);other:AllowTabSwitch(true);other:Show(true)",
    );
    let other = id(&ui, "other");
    let mut mouse = Default::default();
    begin(&ui, &mut mouse, data(&ui, window).tabs[0].button);
    let p = center(&ui, data(&ui, other).tabs[1].button);
    let caret = id(&ui, "other:GetCaretDrawable()");
    pointer(&ui, &mut mouse, Some(p));
    assert!(lua::ui(&ui.lua).tree.get(caret).shown);
    pointer(&ui, &mut mouse, Some((600., 400.)));
    assert!(!lua::ui(&ui.lua).tree.get(caret).shown);
    pointer(&ui, &mut mouse, Some(p));
    assert!(lua::ui(&ui.lua).tree.get(caret).shown);
    run(&ui, "window:StopMovingOrSizing()");
    lua::tick(&ui.lua, 0.);
    assert!(!lua::ui(&ui.lua).tree.get(caret).shown);
    run(&ui, "other:AllowTabSwitch(false)");
    input::update(
        &ui.lua,
        &mut mouse,
        Some((600., 400.)),
        false,
        true,
        false,
        0.,
    );
    begin(&ui, &mut mouse, data(&ui, window).tabs[0].button);
    pointer(&ui, &mut mouse, Some(p));
    assert!(!lua::ui(&ui.lua).tree.get(caret).shown);
    run(&ui, "window:EnableHidingIsRemove(true);window:Show(false)");
    lua::tick(&ui.lua, 0.);
    input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.);
    assert!(!lua::ui(&ui.lua).tree.get(caret).shown);
}

#[test]
fn physical_receive_reorders_paired_rows_before_mouseup_without_reselection_or_detach() {
    let (ui, window) = setup();
    let c = data(&ui, window);
    expose(&ui, "first", c.tabs[0].button);
    expose(&ui, "last", c.tabs[2].button);
    expose(&ui, "message", c.tabs[0].message);
    run(
        &ui,
        r#"
        window:SelectTab(2);window:AllowTabSwitch(false);message:AddMessage('retained')
        first.marker={};last:SetHandler('OnDragReceive',function() record('receive');window:AllowTabSwitch(true) end)
        last:SetHandler('OnMouseUp',function() record('up') end)
        first:SetHandler('OnDragStop',function() record('stop') end);events={}
    "#,
    );
    let mut mouse = Default::default();
    begin(&ui, &mut mouse, c.tabs[0].button);
    let p = center(&ui, c.tabs[2].button);
    pointer(&ui, &mut mouse, Some(p));
    assert_eq!(data(&ui, window).tabs, c.tabs);
    input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.);
    assert_eq!(
        data(&ui, window).tabs,
        vec![c.tabs[1].clone(), c.tabs[2].clone(), c.tabs[0].clone()]
    );
    assert_eq!(data(&ui, window).selected, 1);
    assert_eq!(
        eval::<String>(&ui, "table.concat(events,',')"),
        "receive,attach:7:2,up,stop"
    );
    run(
        &ui,
        "events={};window:GetChatEdit():SetText('');window:GetChatEdit():Show(true);window:GetChatEdit():SetFocus()",
    );
    assert!(ui.type_text("/party hello"));
    // Reorder leaves the prior selected raw8 input binding, although row1 now displays raw9.
    let events = eval::<String>(&ui, "table.concat(events,',')");
    assert!(events.contains("method:8:4"), "{events}");
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .widget(c.tabs[0].message)
            .unwrap()
            .message
            .as_ref()
            .unwrap()
            .lines[0]
            .0,
        "retained"
    );
    assert!(eval::<bool>(&ui, "first.marker~=nil"));
}

#[test]
fn locked_receive_has_no_mutation_and_retiring_receiver_does_not_resurrect_widgets() {
    for retire in [false, true] {
        let (ui, window) = setup();
        let c = data(&ui, window);
        expose(&ui, "last", c.tabs[2].button);
        run(&ui, "window:AllowTabSwitch(false);events={}");
        if retire {
            run(
                &ui,
                "last:SetHandler('OnDragReceive',function() window:EnableHidingIsRemove(true);window:Show(false) end)",
            );
        }
        let mut mouse = Default::default();
        begin(&ui, &mut mouse, c.tabs[0].button);
        let p = center(&ui, c.tabs[2].button);
        pointer(&ui, &mut mouse, Some(p));
        input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.);
        if retire {
            lua::tick(&ui.lua, 0.);
            assert!(!lua::ui(&ui.lua).tree.get(window).alive);
        } else {
            assert_eq!(data(&ui, window).tabs, c.tabs);
        }
        assert!(!eval::<String>(&ui, "table.concat(events,',')").contains("attach:"));
    }
}

#[test]
fn physical_overflow_arrows_repeat_strictly_after_slide_time_and_scroll_by_max_width() {
    let (ui, window) = setup();
    run(
        &ui,
        "other=UIParent:CreateWidget('chatwindow','overflow','UIParent');other:SetExtent(100,180);other:AddAnchor('TOPLEFT',UIParent,700,100);other:SetTabWidth(80);other:UseSlidingButton(true);other:SetSlideTimeInDragging(150);other:SetChatWindowId(1);other:AllowTabSwitch(true);other:Show(true)",
    );
    let other = id(&ui, "other");
    let arrows = {
        let u = lua::ui(&ui.lua);
        let children = &u.tree.widget(other).unwrap().children;
        ["<-", "->"].map(|text| {
            *children
                .iter()
                .find(|&&i| u.tree.get(i).text().is_some_and(|t| t.text == text))
                .unwrap()
        })
    };
    assert_eq!(
        (rect(&ui, arrows[0]).x, rect(&ui, arrows[1]).x),
        (680., 800.)
    );
    let first = data(&ui, other).tabs[0].button;
    let mut mouse = Default::default();
    begin(&ui, &mut mouse, data(&ui, window).tabs[0].button);
    pointer(&ui, &mut mouse, Some(center(&ui, arrows[1])));
    assert_eq!(lua::ui(&ui.lua).hover, Some(arrows[1]));
    lua::tick(&ui.lua, 150.);
    assert_eq!(rect(&ui, first).x, 700.);
    lua::tick(&ui.lua, 1.);
    assert_eq!(rect(&ui, first).x, 620.);
    lua::tick(&ui.lua, 151.);
    assert_eq!(rect(&ui, first).x, 560.);
    lua::tick(&ui.lua, 151.);
    assert_eq!(rect(&ui, first).x, 560.); // Last 60px, not a full 80px step.
    pointer(&ui, &mut mouse, Some(center(&ui, arrows[0])));
    lua::tick(&ui.lua, 151.);
    assert_eq!(rect(&ui, first).x, 640.);
    run(&ui, "window:StopMovingOrSizing()");
    lua::tick(&ui.lua, 0.);
    assert!(
        data(&ui, other)
            .tabs
            .iter()
            .all(|t| lua::ui(&ui.lua).tree.get(t.button).alive)
    );
    run(&ui, "other:UseSlidingButton(false)");
    assert_eq!(rect(&ui, first).x, 700.);
    assert!(arrows.iter().all(|&b| !lua::ui(&ui.lua).tree.get(b).shown));
}

#[test]
fn physical_receive_crosses_owners_before_source_stop_and_preserves_original_row_objects() {
    let (ui, window) = setup();
    let row = data(&ui, window).tabs[0].clone();
    run(
        &ui,
        "AAUI.ChatTabs=function() return {{id=12,name='Twelve'},{id=13,name='Thirteen'}} end;other=UIParent:CreateWidget('chatwindow','target','UIParent');other:SetExtent(300,180);other:AddAnchor('TOPLEFT',UIParent,700,100);other:SetChatWindowId(1);other:AllowTabSwitch(true);other:Show(true);events={}",
    );
    let other = id(&ui, "other");
    let target = data(&ui, other).tabs[0].button;
    expose(&ui, "first", row.button);
    expose(&ui, "receiver", target);
    run(
        &ui,
        "first:SetHandler('OnDragStop',function() record('stop') end);receiver:SetHandler('OnDragReceive',function() record('receive') end)",
    );
    let mut mouse = Default::default();
    begin(&ui, &mut mouse, row.button);
    // Explicit capture stop leaves the physical drag source available to native receive.
    run(&ui, "window:StopMovingOrSizing()");
    lua::tick(&ui.lua, 0.);
    let p = center(&ui, target);
    pointer(&ui, &mut mouse, Some(p));
    input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.);
    assert_eq!(data(&ui, other).tabs[1], row);
    assert_eq!(
        data(&ui, window)
            .tabs
            .iter()
            .map(|t| t.id)
            .collect::<Vec<_>>(),
        vec![8, 9]
    );
    assert_eq!(lua::ui(&ui.lua).tree.get(row.message).parent, Some(other));
    assert_eq!(
        eval::<String>(&ui, "table.concat(events,',')"),
        "receive,select:0,detach:7,select:1,attach:7:1,stop"
    );
}

#[test]
fn disabled_receiving_button_cannot_dispatch_lua_or_reorder() {
    let (ui, window) = setup();
    let c = data(&ui, window);
    expose(&ui, "last", c.tabs[2].button);
    run(
        &ui,
        "window:AllowTabSwitch(false);last:Enable(false);last:SetHandler('OnDragReceive',function() record('receive');window:AllowTabSwitch(true) end);events={}",
    );
    let mut mouse = Default::default();
    begin(&ui, &mut mouse, c.tabs[0].button);
    let p = center(&ui, c.tabs[2].button);
    pointer(&ui, &mut mouse, Some(p));
    input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.);
    assert_eq!(data(&ui, window).tabs, c.tabs);
    assert_eq!(eval::<String>(&ui, "table.concat(events,',')"), "");
}

#[test]
fn attached_callback_can_select_or_retire_without_a_stale_reorder_continuation() {
    for retire in [false, true] {
        let (ui, window) = setup();
        let c = data(&ui, window);
        expose(&ui, "last", c.tabs[2].button);
        run(
            &ui,
            "window:AllowTabSwitch(false);last:SetHandler('OnDragReceive',function() window:AllowTabSwitch(true) end)",
        );
        run(
            &ui,
            if retire {
                "AAUI.ChatTabAttached=function(w,raw,i) assert(raw==7 and i==2);window:EnableHidingIsRemove(true);window:Show(false) end"
            } else {
                "AAUI.ChatTabAttached=function(w,raw,i) assert(raw==7 and i==2);window:SelectTab(3) end"
            },
        );
        let mut mouse = Default::default();
        begin(&ui, &mut mouse, c.tabs[0].button);
        let p = center(&ui, c.tabs[2].button);
        pointer(&ui, &mut mouse, Some(p));
        input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.);
        lua::tick(&ui.lua, 0.);
        if retire {
            let u = lua::ui(&ui.lua);
            assert!(!u.tree.get(window).alive);
            assert!(
                c.tabs
                    .iter()
                    .all(|t| !u.tree.get(t.button).alive && !u.tree.get(t.message).alive)
            );
        } else {
            assert_eq!(data(&ui, window).selected, 2);
            assert_eq!(data(&ui, window).tabs[2], c.tabs[0]);
        }
    }
}

#[test]
fn drag_preparation_uses_own_input_visibility_and_restores_dynamic_layers_on_cancel() {
    let (ui, window) = setup();
    run(
        &ui,
        "other=UIParent:CreateWidget('chatwindow','shownInput','UIParent');other:SetExtent(300,180);other:AddAnchor('TOPLEFT',UIParent,700,100);other:SetChatWindowId(1);other:Show(true);other:GetChatEdit():Show(true)",
    );
    let other = id(&ui, "other");
    assert_eq!(lua::ui(&ui.lua).focus, None);
    let mut mouse = Default::default();
    begin(&ui, &mut mouse, data(&ui, window).tabs[0].button);
    {
        let u = lua::ui(&ui.lua);
        assert_eq!(
            u.tree.widget(window).unwrap().dynamic_drawable.enabled[..3],
            [false; 3]
        );
        assert_eq!(
            u.tree.widget(other).unwrap().dynamic_drawable.enabled[..3],
            [true; 3]
        );
        assert_eq!(
            u.tree.widget(window).unwrap().dynamic_drawable.current[..3],
            [0.7; 3]
        );
    }
    run(&ui, "window:StopMovingOrSizing()");
    lua::tick(&ui.lua, 0.);
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .widget(window)
            .unwrap()
            .dynamic_drawable
            .enabled[..3],
        [true; 3]
    );
}

#[test]
fn unchanged_drag_preview_and_repeat_clock_do_not_repaint_the_widget_tree() {
    let (ui, window) = setup();
    run(
        &ui,
        "other=UIParent:CreateWidget('chatwindow','stillPreview','UIParent');other:SetExtent(300,180);other:AddAnchor('TOPLEFT',UIParent,700,100);other:SetChatWindowId(1);other:AllowTabSwitch(true);other:SetSlideTimeInDragging(150);other:Show(true)",
    );
    let other = id(&ui, "other");
    let mut mouse = Default::default();
    begin(&ui, &mut mouse, data(&ui, window).tabs[0].button);
    pointer(
        &ui,
        &mut mouse,
        Some(center(&ui, data(&ui, other).tabs[1].button)),
    );
    let content = lua::ui(&ui.lua).tree.settle();
    for dt in [1., 149., 1., 100.] {
        lua::tick(&ui.lua, dt);
        assert_eq!(lua::ui(&ui.lua).tree.settle(), content);
    }
}
