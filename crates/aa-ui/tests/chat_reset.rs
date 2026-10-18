//! Immediate native reset, attached popup and retired input lifetime.
#[path = "../examples/native_chat_reset.rs"]
#[allow(dead_code)]
mod original;

use aa_ui::{UiRuntime, input, lua, tree::Id};
use lua::chat::{ChatTabInfo, ChatWindowInfo, hydrate_window, remove_tab, reset_windows};

fn run(ui: &UiRuntime, code: &str) {
    ui.exec("@chat_reset_test", code).unwrap();
}
fn id(ui: &UiRuntime, code: &str) -> Id {
    lua::value_id(&ui.lua, &ui.lua.load(code).eval().unwrap()).unwrap()
}
fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    run(
        &ui,
        r#"
        AAUI={ChatTabs=function() return {{id=0,name='All'}} end}
        window=UIParent:CreateWidget('chatwindow','resetChat','UIParent')
        window:SetExtent(400,180);window:AddAnchor('TOPLEFT',UIParent,100,100);window:Show(true)
        window:SetChatWindowId(0)
        other=UIParent:CreateWidget('window','survivor','UIParent')
        other:SetExtent(100,100);other:AddAnchor('TOPLEFT',UIParent,700,100);other:Show(true)
    "#,
    );
    ui
}
fn step(ui: &UiRuntime, mouse: &mut input::MouseState, p: (f32, f32), down: bool, up: bool) {
    input::update(&ui.lua, mouse, Some(p), down, up, false, 0.);
}

#[test]
fn reset_is_immediate_and_retires_focus_ime_handlers_and_descendants_only() {
    use bevy::{prelude::Entity, window::Ime};
    let ui = setup();
    run(
        &ui,
        r#"
        events,frames,lost,hides=0,0,0,0
        child=window:CreateChildWidget('editbox','resetEdit',0,true)
        child:SetExtent(100,30);child:SetFocus();child:SetText('base')
        child:RegisterEvent('RESET_PROBE')
        child:SetHandler('OnEvent',function() events=events+1 end)
        child:SetHandler('OnUpdate',function() frames=frames+1 end)
        child:SetHandler('OnReleaseFocus',function() lost=lost+1 end)
        window:SetHandler('OnHide',function() hides=hides+1 end)
        function OnResetAllChatWindow() error('application owns recreation') end
    "#,
    );
    assert!(lua::edit::ime::event(
        &ui.lua,
        &Ime::Enabled {
            window: Entity::PLACEHOLDER
        }
    ));
    assert!(lua::edit::ime::event(
        &ui.lua,
        &Ime::Preedit {
            window: Entity::PLACEHOLDER,
            value: "preview".into(),
            cursor: Some((7, 7)),
        }
    ));
    let window = id(&ui, "window");
    let old: Vec<_> = {
        let u = lua::ui(&ui.lua);
        (0..u.tree.objects.len() as Id)
            .filter(|&id| u.tree.is_descendant(id, window))
            .collect()
    };
    assert_eq!(reset_windows(&ui.lua), 1);
    assert_eq!(ui.focus(), None);
    for target in old {
        assert!(!lua::ui(&ui.lua).tree.get(target).alive);
    }
    assert!(lua::ui(&ui.lua).tree.get(id(&ui, "other")).alive);
    assert!(lua::ui(&ui.lua).tree.of_class("chatwindow").is_empty());
    assert!(!lua::edit::ime::event(
        &ui.lua,
        &Ime::Commit {
            window: Entity::PLACEHOLDER,
            value: "stale commit".into(),
        }
    ));
    lua::fire_ui_event(&ui.lua, "RESET_PROBE", vec![]);
    lua::tick(&ui.lua, 100.);
    run(
        &ui,
        "assert(events==0 and frames==0 and lost==1 and hides==0)",
    );
    assert_eq!(reset_windows(&ui.lua), 0);
}

#[test]
fn reset_does_not_invent_hide_for_a_separate_top_level_attachment() {
    let ui = setup();
    run(
        &ui,
        r#"
        popup=UIParent:CreateWidget('window','resetPopup','UIParent');popup:Show(true)
        child=window:CreateChildWidget('button','popupOwner',0,true)
        child:AttachWidget(popup);popupHides=0
        function popupHidden()
            assert(child:GetAttachedWidget()==popup)
            child:DetachWidget();popupHides=popupHides+1
        end
        popup:SetHandler('OnHide',popupHidden)
    "#,
    );
    let child = id(&ui, "child");
    reset_windows(&ui.lua);
    run(&ui, "assert(popupHides==0 and popup:IsVisible())");
    assert!(lua::ui(&ui.lua).tree.get(id(&ui, "popup")).alive);
    assert!(!lua::ui(&ui.lua).tree.get(child).alive);
    assert!(!lua::ui(&ui.lua).attachments.contains_key(&child));
}

#[test]
fn reset_includes_hidden_and_unconfigured_windows_and_preserves_unrelated_focus() {
    let ui = setup();
    run(
        &ui,
        r#"
        hidden=UIParent:CreateWidget('chatwindow','hiddenChat','UIParent');hidden:Show(false)
        unconfigured=UIParent:CreateWidget('chatwindow','unconfiguredChat','UIParent')
        survivorEdit=other:CreateChildWidget('editbox','survivorEdit',0,true)
        survivorEdit:SetExtent(80,20);survivorEdit:SetFocus()
    "#,
    );
    let focus = ui.focus();
    assert_eq!(reset_windows(&ui.lua), 3);
    assert_eq!(ui.focus(), focus);
    assert!(lua::ui(&ui.lua).tree.of_class("chatwindow").is_empty());
    assert!(ui.key(&aa_ui::lua::edit::UiKey::Text("still focused".into())));
    run(&ui, "assert(survivorEdit:GetText()=='still focused')");
}

#[test]
fn reset_clears_retained_history_even_without_live_windows_and_starts_fresh() {
    let ui = setup();
    run(&ui, "X2Chat:DispatchChatMessage(2,'retained before reset')");
    let raw = lua::ui(&ui.lua)
        .tree
        .widget(id(&ui, "window"))
        .unwrap()
        .chat
        .as_ref()
        .unwrap()
        .tabs[0]
        .id;
    remove_tab(&ui.lua, raw).unwrap();
    lua::tick(&ui.lua, 0.);
    assert_eq!(reset_windows(&ui.lua), 0);
    run(
        &ui,
        "fresh=UIParent:CreateWidget('chatwindow','freshChat','UIParent')",
    );
    hydrate_window(
        &ui.lua,
        id(&ui, "fresh"),
        &ChatWindowInfo {
            tabs: vec![ChatTabInfo {
                id: raw,
                name: "All".into(),
                ..Default::default()
            }],
            ..Default::default()
        },
    )
    .unwrap();
    let message = lua::ui(&ui.lua)
        .tree
        .widget(id(&ui, "fresh"))
        .unwrap()
        .chat
        .as_ref()
        .unwrap()
        .tabs[0]
        .message;
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

#[test]
fn stale_pressed_drag_does_not_stop_or_drop_unrelated_held_item_on_release() {
    let ui = setup();
    run(
        &ui,
        r#"
        source=window:CreateChildWidget('button','dragSource',0,true)
        source:SetExtent(100,40);source:AddAnchor('TOPLEFT',window,20,30);source:RegisterForDrag('LeftButton')
        starts,stops,drops=0,0,0
        source:SetHandler('OnDragStart',function() starts=starts+1 end)
        source:SetHandler('OnDragStop',function() stops=stops+1 end)
        AAUI.DropOnNothing=function() drops=drops+1 end
        X2Cursor:SetHeld({kind='bag',slot=4,count=2})
    "#,
    );
    let mut mouse = input::MouseState::default();
    assert_eq!(input::pick(&ui.lua, 150., 150.), Some(id(&ui, "source")));
    step(&ui, &mut mouse, (150., 150.), true, false);
    step(&ui, &mut mouse, (160., 150.), false, false);
    run(&ui, "assert(starts==1)");
    assert_eq!(reset_windows(&ui.lua), 1);
    step(&ui, &mut mouse, (800., 500.), false, true);
    assert_eq!(mouse.pressed, None);
    run(
        &ui,
        "assert(stops==0 and drops==0 and X2Cursor:GetHeld().slot==4)",
    );
}

#[test]
fn retirement_clears_owned_move_and_resize_but_preserves_other_capture() {
    let ui = setup();
    let mut mouse = input::MouseState::default();
    run(
        &ui,
        "window:StartMoving();X2Cursor:SetCursorImage('move',0,0)",
    );
    assert_eq!(reset_windows(&ui.lua), 1);
    assert_eq!(lua::drag::held_kind(&ui.lua), None);
    let dead = id(&ui, "window");
    let before = lua::ui(&ui.lua).tree.rect(dead);
    step(&ui, &mut mouse, (400., 300.), false, false);
    assert_eq!(lua::ui(&ui.lua).tree.rect(dead), before);
    run(
        &ui,
        "window2=UIParent:CreateWidget('chatwindow','second','UIParent');other:StartMoving();X2Cursor:SetCursorImage('move',0,0)",
    );
    let other = id(&ui, "other");
    let before = lua::ui(&ui.lua).tree.rect(other);
    assert_eq!(reset_windows(&ui.lua), 1);
    assert_eq!(lua::drag::held_kind(&ui.lua).as_deref(), Some("image"));
    step(&ui, &mut mouse, (450., 330.), false, false);
    assert_ne!(lua::ui(&ui.lua).tree.rect(other), before);
    run(&ui, "other:StopMovingOrSizing()");
    run(
        &ui,
        "window3=UIParent:CreateWidget('chatwindow','third','UIParent');window3:SetExtent(300,180);window3:StartSizing('RIGHT')",
    );
    let dead = id(&ui, "window3");
    let before = lua::ui(&ui.lua).tree.rect(dead);
    reset_windows(&ui.lua);
    step(&ui, &mut mouse, (500., 350.), false, false);
    assert_eq!(lua::ui(&ui.lua).tree.rect(dead), before);
}

#[test]
fn original_popup_reset_recreation_and_physical_input_remain_usable() {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    original::install(&mut ui);
    original::reset_and_recreate(&ui);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
}
