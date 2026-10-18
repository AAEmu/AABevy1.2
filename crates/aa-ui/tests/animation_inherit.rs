use aa_ui::{UiRuntime, lua};

#[path = "../examples/native_buff_inherit.rs"]
#[allow(dead_code)]
mod original;

fn eval<T: mlua::FromLuaMulti>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().expect(code)
}
fn id(ui: &UiRuntime, name: &str) -> u32 {
    lua::value_id(&ui.lua, &eval(ui, name)).unwrap()
}
fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 0.0001, "{a} != {b}");
}
fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    ui.exec(
        "@inherit_setup",
        r#"
        p=UIParent:CreateWidget('window','inheritParent','UIParent')
        p:SetExtent(0,0);p:Show(true)
        a=p:CreateChildWidget('button','source',0,true)
        b=p:CreateChildWidget('button','destination',0,true)
        a:SetExtent(100,40);b:SetExtent(100,40)
        b:AddAnchor('TOPLEFT',p,200,300)
        calls=0;b:SetHandler('OnScale',function() calls=calls+1 end)
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 0.);
    ui
}

#[test]
fn typed_source_and_widget_casts_return_zero_without_changing_channels() {
    let ui = setup();
    ui.exec(
        "@typed",
        r#"
        b:SetAlpha(.3);b:SetScale(.75);calls=0
        local d=b:CreateColorDrawable(1,1,1,1,'artwork')
        for _,v in ipairs({false,1,'source',{},d}) do
            assert(select('#',b:InheritAnimationData(v))==0)
        end
        assert(select('#',b:InheritAnimationData())==0)
        assert(select('#',b.InheritAnimationData(d,a))==0)
        assert(calls==0 and b:GetAlpha()>.299 and b:GetAlpha()<.301)
    "#,
    )
    .unwrap();
    near(
        lua::ui(&ui.lua)
            .animations
            .widget_sample(id(&ui, "b"))
            .scale,
        0.75,
    );
}

#[test]
fn active_alpha_copies_internal_current_instead_of_visible_override_and_continues() {
    let ui = setup();
    ui.exec(
        "@alpha",
        r#"
        a:SetAlphaAnimation(.5,1,.1,0);a:SetStartAnimation(true,false)
        b:SetAlphaAnimation(.8,0,.2,0);b:SetStartAnimation(true,false)
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 20.);
    ui.exec("@copy", "a:SetAlpha(.1);b:InheritAnimationData(a)")
        .unwrap();
    near(eval(&ui, "b:GetAlpha()"), 0.6);
    assert!(eval::<bool>(&ui, "b:IsNowAnimation()"));
    lua::tick(&ui.lua, 20.);
    near(eval(&ui, "b:GetAlpha()"), 0.7);
    near(eval(&ui, "a:GetAlpha()"), 0.7);
    assert_eq!(eval::<u32>(&ui, "calls"), 0);
}

#[test]
fn inactive_copy_preserves_visible_alpha_scale_and_geometry_but_copies_configuration() {
    let ui = setup();
    ui.exec(
        "@inactive",
        r#"
        a:SetAlphaAnimation(.2,.8,.1,0)
        a:SetScaleAnimation(1.5,1,.1,0,'BOTTOMRIGHT')
        b:SetScale(.75);b:SetAlpha(.3);calls=0
        b:InheritAnimationData(a)
        assert(not b:IsNowAnimation() and calls==0)
    "#,
    )
    .unwrap();
    near(eval(&ui, "b:GetAlpha()"), 0.3);
    near(
        lua::ui(&ui.lua)
            .animations
            .widget_sample(id(&ui, "b"))
            .scale,
        0.75,
    );
    let before = {
        let u = lua::ui(&ui.lua);
        u.animations.widget_rect(&u.tree, id(&ui, "b"))
    };
    ui.exec("@start_copied", "b:SetStartAnimation(true,true)")
        .unwrap();
    near(eval(&ui, "b:GetAlpha()"), 0.2);
    let after = {
        let u = lua::ui(&ui.lua);
        u.animations.widget_rect(&u.tree, id(&ui, "b"))
    };
    near(before.right(), after.right());
    near(before.bottom(), after.bottom());
    assert_eq!(eval::<u32>(&ui, "calls"), 1);
}

#[test]
fn self_copy_applies_internal_alpha_and_scale_event_even_at_identical_scale() {
    let ui = setup();
    ui.exec(
        "@self",
        r#"
        b:SetAlphaAnimation(.5,1,.1,0);b:SetScaleAnimation(1.5,1,.1,0,'CENTER')
        b:SetStartAnimation(true,true);calls=0
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 20.);
    let before = {
        let u = lua::ui(&ui.lua);
        u.animations.widget_rect(&u.tree, id(&ui, "b"))
    };
    ui.exec(
        "@self_copy",
        "b:SetAlpha(.1);calls=0;b:InheritAnimationData(b)",
    )
    .unwrap();
    near(eval(&ui, "b:GetAlpha()"), 0.6);
    let after = {
        let u = lua::ui(&ui.lua);
        u.animations.widget_rect(&u.tree, id(&ui, "b"))
    };
    assert_eq!(before, after);
    assert_eq!(eval::<u32>(&ui, "calls"), 1);
}

#[test]
fn scale_stop_preserves_own_geometry_without_scale_or_completion_callbacks() {
    let ui = setup();
    ui.exec(
        "@stop",
        r#"
        b:SetAlphaAnimation(.5,1,.1,0);b:SetScaleAnimation(1.5,1,.1,0,'CENTER')
        ended=0;b:SetHandler('OnScaleAnimeEnd',function() ended=ended+1 end)
        b:SetStartAnimation(true,true)
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 20.);
    let before = lua::ui(&ui.lua).animations.widget_sample(id(&ui, "b"));
    ui.exec("@stop_now", "calls=0;b:SetStartAnimation(false,false)")
        .unwrap();
    let after = lua::ui(&ui.lua).animations.widget_sample(id(&ui, "b"));
    near(after.scale, before.scale);
    near(eval(&ui, "b:GetAlpha()"), 1.);
    assert_eq!(eval::<u32>(&ui, "calls+ended"), 0);
    assert!(!eval::<bool>(&ui, "b:IsNowAnimation()"));
    lua::tick(&ui.lua, 1000.);
    near(
        lua::ui(&ui.lua)
            .animations
            .widget_sample(id(&ui, "b"))
            .scale,
        before.scale,
    );
}

#[test]
fn mixed_parent_raw_scale_and_changed_pivot_retain_current_native_rectangle_point() {
    let ui = setup();
    ui.exec(
        "@parents",
        r#"
        p:SetScale(1.5)
        a:SetScaleAnimation(1.25,1,.1,0,'CENTER');a:SetStartAnimation(false,true)
        b:SetScale(.75);calls=0
    "#,
    )
    .unwrap();
    let before = {
        let u = lua::ui(&ui.lua);
        u.animations.widget_rect(&u.tree, id(&ui, "b"))
    };
    ui.exec("@mixed_copy", "b:InheritAnimationData(a)").unwrap();
    let after = {
        let u = lua::ui(&ui.lua);
        u.animations.widget_rect(&u.tree, id(&ui, "b"))
    };
    near(after.w, 187.5);
    near(after.h, 75.);
    near(before.x + before.w / 2., after.x + after.w / 2.);
    near(before.y + before.h / 2., after.y + after.h / 2.);
    assert_eq!(eval::<u32>(&ui, "calls"), 1);
    // Reconfigure the source without stepping it: only its configured pivot changes.
    ui.exec("@new_pivot", "a:SetScaleAnimation(1.25,1,.1,0,'TOPLEFT')")
        .unwrap();
    let source_before = {
        let u = lua::ui(&ui.lua);
        u.animations.widget_rect(&u.tree, id(&ui, "a"))
    };
    ui.exec("@pivot_copy", "b:InheritAnimationData(a)").unwrap();
    let copied = {
        let u = lua::ui(&ui.lua);
        u.animations.widget_rect(&u.tree, id(&ui, "b"))
    };
    near(copied.x, after.x);
    near(copied.y, after.y);
    let source_after = {
        let u = lua::ui(&ui.lua);
        u.animations.widget_rect(&u.tree, id(&ui, "a"))
    };
    assert_eq!(source_before, source_after);
}

#[test]
fn original_staggered_buff_reorder_preserves_newer_icon_state_and_continues() {
    let mut ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    original::install(&mut ui);
    original::step(&ui, 0);
    lua::tick(&ui.lua, 40.);
    original::step(&ui, 1);
    lua::tick(&ui.lua, 20.);
    let old = lua::ui(&ui.lua)
        .animations
        .widget_sample(id(&ui, "buffs.button[2]"));
    let first = lua::ui(&ui.lua)
        .animations
        .widget_sample(id(&ui, "buffs.button[1]"));
    assert_ne!(old.alpha, first.alpha);
    assert_ne!(old.scale, first.scale);
    original::step(&ui, 2);
    let new = lua::ui(&ui.lua)
        .animations
        .widget_sample(id(&ui, "buffs.button[1]"));
    assert_eq!(new.alpha, old.alpha);
    assert_eq!(new.scale, old.scale);
    assert_eq!(eval::<u32>(&ui, "buffs.button[1].t.buff_id"), 22);
    assert!(eval::<bool>(&ui, "buffs.button[1]:IsNowAnimation()"));
    let button = id(&ui, "buffs.button[1]");
    let r = {
        let u = lua::ui(&ui.lua);
        u.animations.widget_rect(&u.tree, button)
    };
    assert_eq!(
        aa_ui::input::pick(&ui.lua, r.x + r.w / 2., r.y + r.h / 2.),
        Some(button)
    );
    lua::tick(&ui.lua, 20.);
    let after = lua::ui(&ui.lua)
        .animations
        .widget_sample(id(&ui, "buffs.button[1]"));
    assert!(after.alpha.unwrap() > new.alpha.unwrap());
    assert!(after.scale < new.scale);
}

#[test]
fn unconfigured_source_keeps_native_constructor_endpoints_instead_of_current_visible_values() {
    let ui = setup();
    ui.exec(
        "@constructor_config",
        r#"
        a:SetAlpha(.4);b:SetAlpha(.2);b:SetScale(.75)
        b:InheritAnimationData(a)
        assert(b:GetAlpha()>.199 and b:GetAlpha()<.201)
        b:SetStartAnimation(true,true)
        assert(b:GetAlpha()==1)
    "#,
    )
    .unwrap();
    near(
        lua::ui(&ui.lua)
            .animations
            .widget_sample(id(&ui, "b"))
            .scale,
        1.,
    );
}
