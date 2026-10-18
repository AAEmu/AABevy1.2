//! Native shown-write goldens, replacement lifetime and unchanged original controls.
use aa_ui::{UiRuntime, input, lua, render, tree::Id};
#[path = "../examples/native_checkbutton.rs"]
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
        "@check_setup",
        r#"
        check=UIParent:CreateWidget('checkbutton','nativeCheck','UIParent')
        check:SetExtent(30,30);check:AddAnchor('TOPLEFT',UIParent,200,200);check:Show(true)
        normal=check:CreateColorDrawable(0.2,0.8,0.4,0.6,'artwork')
        normal:SetExtent(16,16);normal:AddAnchor('CENTER',check,0,0)
        disabled=check:CreateColorDrawable(0.8,0.2,0.4,0.6,'artwork')
        disabled:SetExtent(16,16);disabled:AddAnchor('CENTER',check,0,0)
        check:SetCheckedBackground(normal);check:SetDisabledCheckedBackground(disabled)
        count=0;check:SetHandler('OnCheckChanged',function() count=count+1 end)
    "#,
    )
    .unwrap();
    ui
}
fn mark_quads(ui: &UiRuntime) -> usize {
    render::build_quads(&mut lua::ui(&ui.lua), None)
        .iter()
        .filter(|q| q.color == [0.2, 0.8, 0.4, 0.6] || q.color == [0.8, 0.2, 0.4, 0.6])
        .count()
}

#[test]
fn strict_checked_and_art_arguments_have_native_zero_arity_and_silent_gates() {
    let ui = setup();
    ui.exec(
        "@strict_checked",
        r#"
        local other=UIParent:CreateWidget('button','other','UIParent')
        for _,call in ipairs({
            function() return check:SetChecked() end,
            function() return check:SetChecked(nil) end,
            function() return check:SetChecked('true') end,
            function() return check:SetChecked({}) end,
            function() return check.SetChecked(other,true) end,
            function() return check:SetCheckedBackground(nil) end,
            function() return check:SetCheckedBackground('normal') end,
            function() return check:SetCheckedBackground(normal:GetId()) end,
            function() return check:SetCheckedBackground(other) end,
            function() return check.SetCheckedBackground(other,normal) end
        }) do assert(select('#',call())==0) end
        assert(not check:GetChecked() and count==0)
        assert(select('#',check:SetChecked(-2,false,'ignored'))==0)
        assert(check:GetChecked() and count==0)
        check:SetChecked(0);assert(not check:GetChecked() and count==1)
        check:SetChecked(0/0,'false');assert(check:GetChecked() and count==2)
    "#,
    )
    .unwrap();
    assert!(shown(&ui, "normal"));
    assert!(!shown(&ui, "disabled"));
}

#[test]
fn setters_write_mark_visibility_and_repeat_reasserts_without_another_event() {
    let ui = setup();
    assert!(!shown(&ui, "normal") && !shown(&ui, "disabled"));
    ui.exec("@checked", "check:SetChecked(true)").unwrap();
    assert!(shown(&ui, "normal") && !shown(&ui, "disabled"));
    assert_eq!(mark_quads(&ui), 1);
    ui.exec("@explicit_hide", "normal:SetVisible(false)")
        .unwrap();
    assert_eq!(mark_quads(&ui), 0);
    ui.exec("@same_enable", "check:Enable(true)").unwrap();
    assert!(!shown(&ui, "normal"));
    ui.exec("@same_checked", "check:SetChecked(true)").unwrap();
    assert!(shown(&ui, "normal"));
    assert_eq!(eval::<i32>(&ui, "count"), 1);
    ui.exec("@disable", "check:Enable(false)").unwrap();
    assert!(!shown(&ui, "normal") && shown(&ui, "disabled"));
    ui.exec("@uncheck", "check:SetChecked(false)").unwrap();
    assert!(!shown(&ui, "normal") && !shown(&ui, "disabled"));
    assert_eq!(mark_quads(&ui), 0);
}

#[test]
fn replacing_marks_preserves_old_visibility_and_no_disabled_mark_has_no_fallback() {
    let ui = setup();
    ui.exec(
        "@replace_hidden",
        r#"
        late=check:CreateColorDrawable(1,1,0,1,'artwork')
        check:SetCheckedBackground(late)
        check:SetChecked(true)
    "#,
    )
    .unwrap();
    assert!(!shown(&ui, "normal"));
    assert!(shown(&ui, "late"));
    ui.exec("@replace_shown", "check:SetCheckedBackground(normal)")
        .unwrap();
    assert!(shown(&ui, "normal") && shown(&ui, "late"));
    let missing = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    missing
        .exec(
            "@missing_mark",
            r#"
        check=UIParent:CreateWidget('checkbutton','missingCheck','UIParent')
        normal=check:CreateColorDrawable(1,1,1,1,'artwork')
        check:SetCheckedBackground(normal);check:Enable(false);check:SetChecked(true)
    "#,
        )
        .unwrap();
    assert!(!shown(&missing, "normal"));
    missing
        .exec("@manual_mark", "normal:SetVisible(true)")
        .unwrap();
    assert!(shown(&missing, "normal"));
    assert!(render::state_visible(
        &lua::ui(&missing.lua),
        id(&missing, "normal"),
        aa_ui::tree::ButtonState::Disabled
    ));
    missing.exec("@reassert", "check:SetChecked(true)").unwrap();
    assert!(!shown(&missing, "normal"));
}

#[test]
fn aliased_marks_and_recursive_enable_follow_native_write_order() {
    let ui = setup();
    ui.exec(
        "@aliased_mark",
        r#"
        check:SetDisabledCheckedBackground(normal);check:SetChecked(true)
    "#,
    )
    .unwrap();
    assert!(!shown(&ui, "normal"));
    ui.exec("@aliased_disabled", "check:Enable(false)").unwrap();
    assert!(shown(&ui, "normal"));
    let ui = setup();
    ui.exec(
        "@shared_children",
        r#"
        child=check:CreateChildWidget('checkbutton','child',0,true)
        child:SetCheckedBackground(normal);child:SetChecked(false)
        check:SetChecked(true);check:Enable(false,true);check:Enable(true,true)
    "#,
    )
    .unwrap();
    assert!(shown(&ui, "normal"), "children write before parent");
    assert!(eval::<bool>(&ui, "child:IsEnabled() and check:IsEnabled()"));
}

#[test]
fn callback_observes_new_state_and_art_before_reentrant_change() {
    let ui = setup();
    ui.exec(
        "@callback_order",
        r#"
        order={}
        check:SetHandler('OnCheckChanged',function(self)
            order[#order+1]=self:GetChecked()
            if self:GetChecked() then
                assert(normal:IsVisible() and not disabled:IsVisible())
                self:SetChecked(false)
            else assert(not normal:IsVisible() and not disabled:IsVisible()) end
        end)
        check:SetChecked(true)
        assert(#order==2 and order[1] and not order[2] and not check:GetChecked())
    "#,
    )
    .unwrap();
    assert!(!shown(&ui, "normal") && !shown(&ui, "disabled"));
}

fn click(ui: &UiRuntime, name: &str) {
    let target = id(ui, name);
    let (x, y) = {
        let u = lua::ui(&ui.lua);
        let r = u.animations.widget_rect(&u.tree, target);
        (r.x + r.w * 0.5, r.y + r.h * 0.5)
    };
    assert_eq!(input::pick(&ui.lua, x, y), Some(target), "{name}");
    let mut mouse = input::MouseState::default();
    input::update(&ui.lua, &mut mouse, Some((x, y)), true, false, false, 0.);
    input::update(&ui.lua, &mut mouse, Some((x, y)), false, true, false, 0.);
}

#[test]
fn original_check_and_text_clicks_keep_original_callback_and_enable_behavior() {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    original::install(&mut ui);
    click(&ui, "checks[1]");
    assert!(!eval::<bool>(&ui, "checks[1]:GetChecked()"));
    assert!(!shown(&ui, "checks[1].bgs[5]"));
    click(&ui, "checks[1].textButton");
    assert!(eval::<bool>(&ui, "checks[1]:GetChecked()"));
    assert!(shown(&ui, "checks[1].bgs[5]"));
    assert_eq!(eval::<i32>(&ui, "checkClicks"), 2);
    ui.exec("@original_disable", "checks[1]:SetEnableCheckButton(false)")
        .unwrap();
    assert!(!shown(&ui, "checks[1].bgs[5]"));
    assert!(shown(&ui, "checks[1].bgs[6]"));
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
}
