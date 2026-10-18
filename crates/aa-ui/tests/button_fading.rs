//! Original creation controls retain their physical hit while their own fade suppresses clicks.
#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;
use aa_ui::{
    UiKey, UiRuntime, input, lua,
    tree::{ButtonState, Id},
};

fn eval<T: mlua::FromLuaMulti>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().expect(code)
}
fn id(ui: &UiRuntime, code: &str) -> Id {
    lua::value_id(&ui.lua, &eval(ui, code)).unwrap()
}
fn click(ui: &UiRuntime, target: Id) {
    let point = point(ui, target);
    assert_eq!(input::pick(&ui.lua, point.0, point.1), Some(target));
    let mut mouse = Default::default();
    input::update(&ui.lua, &mut mouse, Some(point), true, false, false, 0.);
    input::update(&ui.lua, &mut mouse, Some(point), false, true, false, 0.);
}
fn point(ui: &UiRuntime, target: Id) -> (f32, f32) {
    {
        let u = lua::ui(&ui.lua);
        let r = u.animations.widget_rect(&u.tree, target);
        (r.x + r.w * 0.5, r.y + r.h * 0.5)
    }
}
fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    ui.exec(
        "@button_fade_fixture",
        r#"
        under=UIParent:CreateWidget('button','under','UIParent')
        under:SetExtent(400,200);under:AddAnchor('TOPLEFT',UIParent,100,100)
        under:Show(true)
        parent=UIParent:CreateWidget('window','parent','UIParent')
        parent:SetExtent(400,200);parent:AddAnchor('TOPLEFT',UIParent,100,100)
        parent:Show(true)
        b=parent:CreateChildWidget('button','b',0,true)
        b:SetExtent(100,40);b:AddAnchor('TOPLEFT',parent,20,20)
        b:Show(true)
        counts={down=0,up=0,click=0,under=0,enter=0,wheel=0,key=0,focus=0,global=0}
        b:SetHandler('OnMouseDown',function() counts.down=counts.down+1 end)
        b:SetHandler('OnMouseUp',function() counts.up=counts.up+1 end)
        b:SetHandler('OnClick',function() counts.click=counts.click+1 end)
        b:SetHandler('OnEnter',function() counts.enter=counts.enter+1 end)
        b:SetHandler('OnWheelUp',function() counts.wheel=counts.wheel+1 end)
        b:SetHandler('OnKeyDown',function() counts.key=counts.key+1 end)
        b:SetHandler('OnAcceptFocus',function() counts.focus=counts.focus+1 end)
        under:SetHandler('OnClick',function() counts.under=counts.under+1 end)
        UIParent:SetEventHandler('MOUSE_DOWN',function(target)
            counts.globalTarget=target;counts.global=counts.global+1
        end)
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 0.);
    ui
}
fn own_fade(ui: &UiRuntime) {
    ui.exec("@hide", "b:Show(false)").unwrap();
    lua::tick(&ui.lua, 0.);
    ui.exec("@fade", "b:Show(true,1000)").unwrap();
}
fn state(ui: &UiRuntime) -> ButtonState {
    lua::ui(&ui.lua)
        .tree
        .widget(id(ui, "b"))
        .unwrap()
        .button
        .as_ref()
        .unwrap()
        .state
}

#[test]
fn strict_wrapper_numbers_defaults_and_failed_class_keep_native_zero_arity() {
    let ui = setup();
    assert!(
        !lua::ui(&ui.lua)
            .tree
            .widget(id(&ui, "b"))
            .unwrap()
            .button
            .as_ref()
            .unwrap()
            .ignore_input_on_fading
    );
    ui.exec(
        "@strict",
        r#"
        assert(select('#',b:IgnoreInputOnFading(true))==0)
        for _,f in ipairs({function() return b:IgnoreInputOnFading() end,
            function() return b:IgnoreInputOnFading(nil) end,
            function() return b:IgnoreInputOnFading('false') end,
            function() return b:IgnoreInputOnFading({}) end,
            function() return b.IgnoreInputOnFading(parent,true) end}) do
            assert(select('#',f())==0)
        end
    "#,
    )
    .unwrap();
    own_fade(&ui);
    click(&ui, id(&ui, "b"));
    assert_eq!(eval::<u32>(&ui, "counts.click"), 0);
    for (argument, on) in [
        ("false", false),
        ("0", false),
        ("-0", false),
        ("true", true),
        ("-2", true),
        ("1e-320", true),
        ("0/0", true),
        ("math.huge", true),
        ("false,true", false),
    ] {
        ui.exec(
            "@typed",
            &format!("assert(select('#',b:IgnoreInputOnFading({argument}))==0);counts.click=0"),
        )
        .unwrap();
        click(&ui, id(&ui, "b"));
        assert_eq!(
            eval::<u32>(&ui, "counts.click"),
            u32::from(!on),
            "{argument}"
        );
    }
}

#[test]
fn own_fade_blocks_only_guarded_entries_keeps_hit_hover_focus_wheel_keys_and_global_mouse() {
    let ui = setup();
    ui.exec(
        "@ignore",
        "b:IgnoreInputOnFading(true);b:EnableFocus(true);b:EnableKeyboard(true)",
    )
    .unwrap();
    own_fade(&ui);
    let target = id(&ui, "b");
    let point = point(&ui, target);
    click(&ui, target);
    assert_eq!(
        eval::<[u32; 4]>(
            &ui,
            "return {counts.down,counts.up,counts.click,counts.under}"
        ),
        [0, 0, 0, 0]
    );
    assert_eq!(
        eval::<[u32; 3]>(&ui, "return {counts.enter,counts.focus,counts.global}"),
        [1, 1, 1]
    );
    assert_eq!(eval::<Id>(&ui, "counts.globalTarget"), target);
    assert_eq!(state(&ui), ButtonState::Highlight);
    input::update(
        &ui.lua,
        &mut Default::default(),
        Some(point),
        false,
        false,
        false,
        1.,
    );
    ui.key(&UiKey::Text("A".into()));
    assert_eq!(
        eval::<[u32; 2]>(&ui, "return {counts.wheel,counts.key}"),
        [1, 1]
    );
    lua::tick(&ui.lua, 1000.);
    click(&ui, target);
    assert_eq!(
        eval::<[u32; 3]>(&ui, "return {counts.down,counts.up,counts.click}"),
        [1, 1, 1]
    );
}

#[test]
fn parent_fade_alpha_zero_and_disabled_option_do_not_suppress_child_button() {
    let ui = setup();
    ui.exec("@ignore", "b:IgnoreInputOnFading(true);parent:Show(false)")
        .unwrap();
    lua::tick(&ui.lua, 0.);
    ui.exec("@parent_fade", "parent:Show(true,1000);b:SetAlpha(0)")
        .unwrap();
    assert!(
        lua::ui(&ui.lua)
            .tree
            .widget(id(&ui, "parent"))
            .unwrap()
            .fading
    );
    click(&ui, id(&ui, "b"));
    assert_eq!(eval::<u32>(&ui, "counts.click"), 1);
    own_fade(&ui);
    ui.exec("@allow", "b:IgnoreInputOnFading(false)").unwrap();
    click(&ui, id(&ui, "b"));
    assert_eq!(eval::<u32>(&ui, "counts.click"), 2);
}

#[test]
fn zero_duration_pending_record_and_late_option_changes_follow_actual_manager_completion() {
    let ui = setup();
    let target = id(&ui, "b");
    ui.exec(
        "@late_flag",
        "b:Show(false);b:Show(true);b:IgnoreInputOnFading(true)",
    )
    .unwrap();
    click(&ui, target);
    assert_eq!(eval::<u32>(&ui, "counts.click"), 0);
    lua::tick(&ui.lua, 0.);
    click(&ui, target);
    assert_eq!(eval::<u32>(&ui, "counts.click"), 1);
    own_fade(&ui);
    ui.exec("@release_flag", "b:IgnoreInputOnFading(false)")
        .unwrap();
    click(&ui, target);
    assert_eq!(eval::<u32>(&ui, "counts.click"), 2);
    // Setter does not cancel/restart the record or turn an alpha channel into a fade.
    assert!(lua::ui(&ui.lua).tree.widget(target).unwrap().fading);
    lua::tick(&ui.lua, 1000.);
    ui.exec(
        "@alpha_channel",
        "b:IgnoreInputOnFading(true);b:SetAlphaAnimation(1,0,1,1);b:SetStartAnimation(true,false)",
    )
    .unwrap();
    click(&ui, target);
    assert_eq!(eval::<u32>(&ui, "counts.click"), 3);
}

#[test]
fn each_entry_rechecks_fade_but_admitted_release_continues_after_callback() {
    let ui = setup();
    let target = id(&ui, "b");
    let at = point(&ui, target);
    ui.exec(
        "@release_callback",
        r#"
        b:IgnoreInputOnFading(true)
        b:SetHandler('OnMouseUp',function()
            counts.up=counts.up+1;b:Show(false);b:Show(true,1000)
        end)
    "#,
    )
    .unwrap();
    let mut mouse = Default::default();
    input::update(&ui.lua, &mut mouse, Some(at), true, false, false, 0.);
    assert_eq!(state(&ui), ButtonState::Pushed);
    input::update(&ui.lua, &mut mouse, Some(at), false, true, false, 0.);
    assert_eq!(state(&ui), ButtonState::Highlight);
    assert_eq!(
        eval::<[u32; 3]>(&ui, "return {counts.down,counts.up,counts.click}"),
        [1, 1, 0]
    );
    lua::tick(&ui.lua, 1000.);
    ui.exec(
        "@press_callback",
        r#"
        b:SetHandler('OnMouseDown',function()
            counts.down=counts.down+1;b:Show(false);b:Show(true,1000)
        end)
    "#,
    )
    .unwrap();
    click(&ui, target);
    assert_eq!(state(&ui), ButtonState::Pushed);
    assert_eq!(
        eval::<[u32; 3]>(&ui, "return {counts.down,counts.up,counts.click}"),
        [2, 1, 0]
    );
}

#[test]
fn admitted_check_click_still_toggles_when_callback_starts_fade() {
    let ui = setup();
    ui.exec(
        "@check",
        r#"
        c=parent:CreateChildWidget('checkbutton','c',0,true)
        c:SetExtent(100,40);c:AddAnchor('TOPLEFT',parent,200,20)
        c:Show(true)
        c:IgnoreInputOnFading(true);c:SetChecked(false)
        c:SetHandler('OnClick',function() c:Show(false);c:Show(true,1000) end)
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 0.);
    click(&ui, id(&ui, "c"));
    assert!(eval::<bool>(&ui, "c:GetChecked()"));
    click(&ui, id(&ui, "c"));
    assert!(eval::<bool>(&ui, "c:GetChecked()"));
}

#[test]
fn drag_source_and_recipient_gates_are_independent_of_manager_cleanup() {
    let ui = setup();
    ui.exec("@drag",r#"
        b:IgnoreInputOnFading(true);counts.start=0;counts.stop=0;counts.receive=0
        b:SetHandler('OnDragStart',function() counts.start=counts.start+1 end)
        b:SetHandler('OnDragStop',function() counts.stop=counts.stop+1 end)
        b:SetHandler('OnDragReceive',function() counts.receive=counts.receive+1;X2Cursor:Clear() end)
    "#).unwrap();
    own_fade(&ui);
    let target = id(&ui, "b");
    ui.exec("@held", "X2Cursor:SetHeld({kind='thing'})")
        .unwrap();
    lua::drag::start(&ui.lua, target);
    lua::drag::finish(&ui.lua, target, Some(target));
    assert_eq!(
        eval::<[u32; 3]>(&ui, "return {counts.start,counts.stop,counts.receive}"),
        [0, 0, 0]
    );
    assert!(
        !ui.holding(),
        "unconsumed item still follows the existing manager cleanup path"
    );
    lua::tick(&ui.lua, 1000.);
    ui.exec("@held", "X2Cursor:SetHeld({kind='thing'})")
        .unwrap();
    lua::drag::start(&ui.lua, target);
    lua::drag::finish(&ui.lua, target, Some(target));
    assert_eq!(
        eval::<[u32; 3]>(&ui, "return {counts.start,counts.stop,counts.receive}"),
        [1, 1, 1]
    );
}

#[test]
fn original_character_creation_next_button_ignores_own_fade_then_advances() {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    fake::install(&mut ui, "en_us", &fake::default_worlds());
    fake::install_ingame(&mut ui);
    ui.run_stage(aa_ui::Stage::CharacterCreate);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    ui.fire_event("SHOW_CHARACTER_CREATE_WINDOW", true);
    lua::tick(&ui.lua, 2500.);
    assert_eq!(eval::<String>(&ui, "characterCreate.state"), "Step1");
    ui.exec(
        "@hide_original_button",
        "characterCreate.bgWindow.nextStepBtn:Show(false)",
    )
    .unwrap();
    lua::tick(&ui.lua, 0.);
    ui.exec(
        "@own_original_fade",
        "characterCreate.bgWindow.nextStepBtn:Show(true,1000)",
    )
    .unwrap();
    let target = id(&ui, "characterCreate.bgWindow.nextStepBtn");
    assert!(lua::ui(&ui.lua).tree.widget(target).unwrap().fading);
    click(&ui, target);
    assert_eq!(eval::<String>(&ui, "characterCreate.state"), "Step1");
    lua::tick(&ui.lua, 1000.);
    click(&ui, target);
    assert_eq!(eval::<String>(&ui, "characterCreate.state"), "Step2");
}
