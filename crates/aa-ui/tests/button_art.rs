//! Native state writes, strict bindings, reentrant pointer order and original consumers.
use aa_ui::{
    UiRuntime, input, lua, render,
    tree::{ButtonState, Id},
};
#[path = "../examples/native_button_art.rs"]
#[allow(dead_code)]
mod original;

fn eval<T: mlua::FromLuaMulti>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().expect(code)
}
fn id(ui: &UiRuntime, name: &str) -> Id {
    lua::value_id(&ui.lua, &eval(ui, name)).unwrap()
}
fn shown(ui: &UiRuntime, name: &str) -> bool {
    lua::ui(&ui.lua).tree.get(id(ui, name)).shown
}
fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    ui.exec(
        "@button_art",
        r#"
        button=UIParent:CreateWidget('button','nativeArt','UIParent')
        button:SetExtent(100,40);button:AddAnchor('TOPLEFT',UIParent,200,200);button:Show(true)
        local function art(r,g,b)
            local d=button:CreateColorDrawable(r,g,b,1,'artwork')
            d:AddAnchor('TOPLEFT',button,0,0);d:AddAnchor('BOTTOMRIGHT',button,0,0)
            return d
        end
        normal=art(0.1,0.2,0.3);highlight=art(0.2,0.3,0.4)
        pushed=art(0.3,0.4,0.5);disabled=art(0.4,0.5,0.6)
        button:SetNormalBackground(normal);button:SetHighlightBackground(highlight)
        button:SetPushedBackground(pushed);button:SetDisabledBackground(disabled)
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 0.);
    ui
}
fn point(ui: &UiRuntime, target: Id) -> (f32, f32) {
    let u = lua::ui(&ui.lua);
    let r = u.animations.widget_rect(&u.tree, target);
    (r.x + r.w * 0.5, r.y + r.h * 0.5)
}
fn color_count(ui: &UiRuntime, color: [f32; 4]) -> usize {
    render::build_quads(&mut lua::ui(&ui.lua), None)
        .iter()
        .filter(|q| q.color == color)
        .count()
}

#[test]
fn strict_state_and_background_wrappers_keep_zero_arity_and_native_names() {
    let ui = setup();
    ui.exec("@strict",r#"
        assert(button:GetButtonState()=='NORMAL' and select('#',button:GetButtonState())==1)
        for _,call in ipairs({
            function() return button:SetButtonState() end,
            function() return button:SetButtonState(nil) end,
            function() return button:SetButtonState(3) end,
            function() return button:SetNormalBackground(nil) end,
            function() return button:SetNormalBackground(normal:GetId()) end,
            function() return button:SetNormalBackground(button) end,
            function() return button.SetButtonState(normal,'PUSHED') end
        }) do assert(select('#',call())==0) end
        assert(normal:IsVisible() and button:GetButtonState()=='NORMAL')
        assert(select('#',button:SetButtonState('pUsHeD\0ignored','extra'))==0)
        assert(button:GetButtonState()=='PUSHED')
        button:SetButtonState('highlight');assert(button:GetButtonState()=='NORMAL')
        button:SetButtonState('highlighted');assert(button:GetButtonState()=='HIGHLIGHTED')
        button:Enable(false);button:SetButtonState('NORMAL');assert(button:GetButtonState()=='DISABLED')
        button:Enable(true);assert(button:GetButtonState()=='NORMAL')
    "#).unwrap();
    assert!(shown(&ui, "normal"));
    assert!(!shown(&ui, "highlight") && !shown(&ui, "pushed") && !shown(&ui, "disabled"));
}

#[test]
fn manual_visibility_lasts_until_state_write_and_rebinding_hides_old_art() {
    let ui = setup();
    ui.exec(
        "@manual",
        "normal:SetVisible(false);highlight:SetVisible(true)",
    )
    .unwrap();
    assert_eq!(color_count(&ui, [0.1, 0.2, 0.3, 1.]), 0);
    assert_eq!(color_count(&ui, [0.2, 0.3, 0.4, 1.]), 1);
    ui.exec("@same_state", "button:SetButtonState('NORMAL')")
        .unwrap();
    assert!(shown(&ui, "normal") && !shown(&ui, "highlight"));
    ui.exec(
        "@replacement",
        r#"
        late=button:CreateColorDrawable(0.8,0.7,0.6,1,'artwork')
        button:SetNormalBackground(late)
    "#,
    )
    .unwrap();
    assert!(!shown(&ui, "normal") && shown(&ui, "late"));
    ui.exec(
        "@old_manual",
        "normal:SetVisible(true);button:SetButtonState('PUSHED')",
    )
    .unwrap();
    assert!(shown(&ui, "normal") && shown(&ui, "pushed") && !shown(&ui, "late"));
    assert_eq!(color_count(&ui, [0.1, 0.2, 0.3, 1.]), 1);
    ui.exec(
        "@nil_is_not_unbind",
        "button:SetNormalBackground(nil);button:SetButtonState('NORMAL')",
    )
    .unwrap();
    assert!(shown(&ui, "late"));
}

#[test]
fn absent_state_has_no_fallback_and_extras_reapply_current_state_when_created() {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    ui.exec(
        "@extras",
        r#"
        button=UIParent:CreateWidget('button','extras','UIParent')
        normal=button:CreateColorDrawable(1,0,0,1,'artwork');button:SetNormalBackground(normal)
        button:SetButtonState('PUSHED');assert(not normal:IsVisible())
        pushed=button:CreatePushedDrawable('colorDrawable',1,1,1,1,nil,'artwork')
        highlight=button:CreateHighlightDrawable('colorDrawable',1,1,1,1,nil,'artwork')
        assert(pushed:IsVisible() and not highlight:IsVisible())
        pushed:SetVisible(false);normal:SetVisible(true)
        late=button:CreateHighlightDrawable('drawable',1,1,1,1,'ui/hud.dds','overlay')
        assert(pushed:IsVisible() and not normal:IsVisible() and not late:IsVisible())
        button:SetButtonState('HIGHLIGHTED')
        assert(late:IsVisible() and highlight:IsVisible() and not pushed:IsVisible())
        button:Enable(false);assert(not late:IsVisible() and not highlight:IsVisible())
    "#,
    )
    .unwrap();
}

#[test]
fn alias_state_writes_hide_all_then_show_selected_and_repeat_enable_is_inert() {
    let ui = setup();
    ui.exec(
        "@aliases",
        r#"
        button:SetHighlightBackground(normal);button:SetPushedBackground(normal)
        assert(normal:IsVisible() and not highlight:IsVisible() and not pushed:IsVisible())
        button:SetButtonState('PUSHED');assert(normal:IsVisible())
        normal:SetVisible(false);button:Enable(true);assert(not normal:IsVisible())
        button:SetButtonState('PUSHED');assert(normal:IsVisible())
        button:Enable(false);assert(not normal:IsVisible() and disabled:IsVisible())
        disabled:SetVisible(false);button:Enable(false);assert(not disabled:IsVisible())
    "#,
    )
    .unwrap();
}

#[test]
fn physical_pointer_state_precedes_handlers_and_release_rereads_callback_state() {
    let ui = setup();
    ui.exec("@handlers",r#"
        calls={};local function record(self,event) calls[#calls+1]=event..':'..self:GetButtonState() end
        button:SetHandler('OnEnter',function(self) record(self,'enter') end)
        button:SetHandler('OnLeave',function(self) record(self,'leave') end)
        button:SetHandler('OnMouseDown',function(self) record(self,'down') end)
        button:SetHandler('OnMouseUp',function(self) record(self,'up');self:SetButtonState('NORMAL') end)
        button:SetHandler('OnClick',function(self) record(self,'click') end)
    "#).unwrap();
    let target = id(&ui, "button");
    let p = point(&ui, target);
    let mut mouse = Default::default();
    assert_eq!(input::pick(&ui.lua, p.0, p.1), Some(target));
    input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.);
    assert!(shown(&ui, "pushed"));
    input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.);
    assert!(shown(&ui, "normal"));
    input::update(
        &ui.lua,
        &mut mouse,
        Some((10., 10.)),
        false,
        false,
        false,
        0.,
    );
    assert_eq!(
        eval::<String>(&ui, "table.concat(calls,',')"),
        "enter:HIGHLIGHTED,down:PUSHED,up:PUSHED,click:NORMAL,leave:NORMAL"
    );
}

#[test]
fn click_registration_is_case_sensitive_literal_bool_and_governs_pressed_art() {
    let ui = setup();
    let target = id(&ui, "button");
    let p = point(&ui, target);
    let mut mouse = Default::default();
    ui.exec(
        "@registration",
        r#"
        assert(select('#',button:RegisterForClicks('LeftButton',false))==0)
        button:RegisterForClicks('leftbutton',true)
        button:RegisterForClicks(1,true)
    "#,
    )
    .unwrap();
    input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.);
    assert_eq!(
        eval::<String>(&ui, "button:GetButtonState()"),
        "HIGHLIGHTED"
    );
    input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.);
    ui.exec(
        "@numeric_default",
        "button:RegisterForClicks('LeftButton',0)",
    )
    .unwrap();
    input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.);
    assert_eq!(eval::<String>(&ui, "button:GetButtonState()"), "PUSHED");
    input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.);
    assert_eq!(
        eval::<String>(&ui, "button:GetButtonState()"),
        "HIGHLIGHTED"
    );
    input::update_buttons(
        &ui.lua,
        &mut mouse,
        Some(p),
        input::MouseButtons {
            right_press: true,
            ..Default::default()
        },
        0.,
    );
    assert!(!shown(&ui, "pushed"));
    ui.exec(
        "@right_register",
        r#"button:RegisterForClicks('RightButton\0suffix')"#,
    )
    .unwrap();
    input::update_buttons(
        &ui.lua,
        &mut mouse,
        Some(p),
        input::MouseButtons {
            right_press: true,
            ..Default::default()
        },
        0.,
    );
    assert!(shown(&ui, "pushed"));
    input::update_buttons(
        &ui.lua,
        &mut mouse,
        Some(p),
        input::MouseButtons {
            right_release: true,
            ..Default::default()
        },
        0.,
    );
    assert!(shown(&ui, "highlight"));
}

#[test]
fn enabled_restore_uses_current_hover_and_late_release_cannot_override_disabled_callback() {
    let ui = setup();
    let p = point(&ui, id(&ui, "button"));
    let mut mouse = Default::default();
    input::update(&ui.lua, &mut mouse, Some(p), false, false, false, 0.);
    ui.exec(
        "@enable_hover",
        "button:Enable(false);button:Enable(true);assert(button:GetButtonState()=='HIGHLIGHTED')",
    )
    .unwrap();
    ui.exec(
        "@disable_during_up",
        "button:SetHandler('OnMouseUp',function(self) self:Enable(false) end)",
    )
    .unwrap();
    input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.);
    input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.);
    assert!(shown(&ui, "disabled") && !shown(&ui, "highlight"));
}

#[test]
fn callback_rebinding_reapplies_live_state_and_pointer_leave_does_not_track_old_binding() {
    let ui = setup();
    ui.exec(
        "@reentrant_bind",
        r#"
        late=button:CreateColorDrawable(0.9,0.8,0.7,1,'artwork');late:SetVisible(false)
        button:SetHandler('OnMouseDown',function(self)
            assert(self:GetButtonState()=='PUSHED')
            self:SetPushedBackground(late)
            assert(late:IsVisible() and not pushed:IsVisible())
        end)
        button:SetHandler('OnMouseUp',function(self)
            assert(self:GetButtonState()=='PUSHED')
            pushed:SetVisible(true)
        end)
    "#,
    )
    .unwrap();
    let p = point(&ui, id(&ui, "button"));
    let mut mouse = Default::default();
    input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.);
    assert!(shown(&ui, "late") && !shown(&ui, "pushed"));
    input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.);
    assert!(!shown(&ui, "late") && shown(&ui, "highlight") && shown(&ui, "pushed"));
    input::update(
        &ui.lua,
        &mut mouse,
        Some((10., 10.)),
        false,
        false,
        false,
        0.,
    );
    assert!(shown(&ui, "normal") && shown(&ui, "pushed"));
}

#[test]
fn original_default_skin_aliases_and_cooldown_action_preserve_art_and_physical_callbacks() {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    original::install(&mut ui);
    ui.exec(
        "@skin_lifetime",
        r#"
        assert(not hiddenButton.bgs[BUTTON_STATE.NORMAL]:IsVisible())
        assert(aliasButton.bgs[BUTTON_STATE.PUSHED]:IsVisible())
        SetBGPushed(aliasButton,false)
        assert(aliasButton.bgs[BUTTON_STATE.NORMAL]:IsVisible())
        assert(not aliasButton.bgs[BUTTON_STATE.PUSHED]:IsVisible())
        SetBGHighlighted(aliasButton,true)
        assert(aliasButton.bgs[BUTTON_STATE.HIGHLIGHTED]:IsVisible())
        SetBGHighlighted(aliasButton,false)
        assert(not aliasButton.bgs[BUTTON_STATE.HIGHLIGHTED]:IsVisible())
    "#,
    )
    .unwrap();
    let mut mouse = Default::default();
    for name in ["artButtons[1]", "actionButton"] {
        let target = id(&ui, name);
        let p = point(&ui, target);
        assert_eq!(input::pick(&ui.lua, p.0, p.1), Some(target));
        input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.);
        assert_eq!(
            render::button_state(&lua::ui(&ui.lua), target, None),
            ButtonState::Pushed
        );
        input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.);
        assert_eq!(
            render::button_state(&lua::ui(&ui.lua), target, None),
            ButtonState::Highlight
        );
    }
    assert_eq!(eval::<u32>(&ui, "artClicks"), 2);
    assert_eq!(
        eval::<String>(&ui, "table.concat(artStates,',')"),
        "PUSHED,PUSHED"
    );
    assert!(render::build_quads(&mut lua::ui(&ui.lua), None).len() > 10);
}
