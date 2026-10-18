//! Executed native configure/start/update goldens; shared effect Timing is independent.
use aa_ui::{UiRuntime, input, lua, render};

#[path = "../examples/native_widget_channel.rs"]
#[allow(dead_code)]
mod original;

#[path = "../examples/native_widget_channel_callbacks.rs"]
#[allow(dead_code)]
mod original_callbacks;

fn eval<T: mlua::FromLuaMulti>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().expect(code)
}

fn id(ui: &UiRuntime) -> u32 {
    lua::value_id(&ui.lua, &eval(&ui, "w")).unwrap()
}

fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    ui.exec(
        "@channel_setup",
        r#"
        parent=UIParent:CreateWidget('window','channelParent','UIParent');parent:Show(true)
        w=parent:CreateChildWidget('button','channelButton',0,true)
        w:SetExtent(100,40);w:AddAnchor('TOPLEFT',parent,100,100)
        art=w:CreateColorDrawable(.2,.7,.9,1,'artwork')
        art:AddAnchor('TOPLEFT',w,0,0);art:AddAnchor('BOTTOMRIGHT',w,0,0)
        ends=0;scales=0
        w:SetHandler('OnAlphaAnimeEnd',function() ends=ends+1 end)
        w:SetHandler('OnScaleAnimeEnd',function() ends=ends+1 end)
        w:SetHandler('OnScale',function() scales=scales+1 end)
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 0.);
    ui
}

fn value(ui: &UiRuntime, scale: bool) -> f32 {
    if scale {
        lua::ui(&ui.lua).animations.widget_sample(id(ui)).scale
    } else {
        eval(ui, "w:GetAlpha()")
    }
}

fn observe_scale(ui: &UiRuntime) {
    ui.lua
        .globals()
        .set(
            "observedScale",
            ui.lua
                .create_function(|lua, widget: mlua::Value| {
                    let id = lua::value_id(lua, &widget).unwrap();
                    Ok(lua::ui(lua).animations.widget_sample(id).scale)
                })
                .unwrap(),
        )
        .unwrap();
}

#[test]
fn wrappers_require_all_typed_arguments_and_return_zero_results_without_partial_configuration() {
    let ui = setup();
    ui.lua
        .globals()
        .set(
            "observedScale",
            ui.lua
                .create_function(|lua, widget: mlua::Value| {
                    let id = lua::value_id(lua, &widget).unwrap();
                    Ok(lua::ui(lua).animations.widget_sample(id).scale)
                })
                .unwrap(),
        )
        .unwrap();
    ui.exec(
        "@typed_channels",
        r#"
        local badAlpha={
            function() return w:SetAlphaAnimation() end,
            function() return w:SetAlphaAnimation(.7,.8,.3) end,
            function() return w:SetAlphaAnimation('0.7',.8,.3,.4) end,
            function() return w:SetAlphaAnimation(.7,'0.8',.3,.4) end,
            function() return w:SetAlphaAnimation(.7,.8,'0.3',.4) end,
            function() return w:SetAlphaAnimation(.7,.8,.3,'0.4') end,
            function() return w:SetAlphaAnimation(.7,.8,true,.4) end,
            function() return w:SetAlphaAnimation(.7,nil,.3,.4) end,
            function() return w:SetAlphaAnimation({},.8,.3,.4) end,
            function() return w.SetAlphaAnimation(art,.7,.8,.3,.4) end
        }
        for _,call in ipairs(badAlpha) do
            w:SetAlphaAnimation(.2,.9,.3,.4)
            assert(select('#',call())==0)
            w:SetStartAnimation(true,false);assert(math.abs(w:GetAlpha()-.2)<.00001)
        end
        local badScale={
            function() return w:SetScaleAnimation() end,
            function() return w:SetScaleAnimation(.7,.8,.3,.4) end,
            function() return w:SetScaleAnimation(.7,.8,.3,.4,1) end,
            function() return w:SetScaleAnimation(.7,.8,.3,.4,true) end,
            function() return w:SetScaleAnimation(.7,.8,.3,.4,{}) end,
            function() return w:SetScaleAnimation(.7,.8,'0.3',.4,'CENTER') end,
            function() return w.SetScaleAnimation(art,.7,.8,.3,.4,'CENTER') end
        }
        for _,call in ipairs(badScale) do
            w:SetScaleAnimation(.2,.9,.3,.4,'CENTER')
            assert(select('#',call())==0)
            w:SetStartAnimation(false,true);assert(math.abs(observedScale(w)-.2)<.00001)
        end
        assert(select('#',w:SetAlphaAnimation(.2,.9,.3,.4,'ignored'))==0)
        assert(select('#',w:SetScaleAnimation(.2,.9,.3,.4,'CENTER',{}))==0)
    "#,
    )
    .unwrap();
}

#[test]
fn reconfiguring_running_channels_keeps_current_and_flags_but_replaces_velocity_immediately() {
    for scale in [false, true] {
        for (vt, at, after20, after30) in [
            (0.3, 0., 0x3eca34b4, 0x3ef4b3ae),
            (0.3, 0.4, 0x3ec91d16, 0x3eef837c),
            (-0.3, -0.4, 0x3e928012, 0x3e4df5e7),
        ] {
            let ui = setup();
            let configure = if scale {
                "SetScaleAnimation"
            } else {
                "SetAlphaAnimation"
            };
            let pivot = if scale { ",'CENTER'" } else { "" };
            let start = if scale { "false,true" } else { "true,false" };
            ui.exec(
                "@start_channels",
                &format!("w:{configure}(.2,1,.5,.4{pivot});w:SetStartAnimation({start})"),
            )
            .unwrap();
            lua::tick(&ui.lua, 80.);
            assert_eq!(value(&ui, scale).to_bits(), 0x3eae7d57);
            let before_callbacks = eval::<i64>(&ui, "scales");
            ui.exec(
                "@running_reconfigure",
                &format!("w:{configure}(.1,.9,{vt},{at}{pivot})"),
            )
            .unwrap();
            assert_eq!(value(&ui, scale).to_bits(), 0x3eae7d57);
            assert!(eval::<bool>(&ui, "w:IsNowAnimation()"));
            assert_eq!(eval::<i64>(&ui, "scales"), before_callbacks);
            lua::tick(&ui.lua, 0.);
            assert_eq!(value(&ui, scale).to_bits(), 0x3eae7d57);
            lua::tick(&ui.lua, 20.);
            assert_eq!(value(&ui, scale).to_bits(), after20);
            lua::tick(&ui.lua, 30.);
            assert_eq!(value(&ui, scale).to_bits(), after30);
            assert_eq!(eval::<i64>(&ui, "ends"), 0);
            ui.exec("@restart_channel", &format!("w:SetStartAnimation({start})"))
                .unwrap();
            assert_eq!(value(&ui, scale).to_bits(), 0x3dcccccd);
            lua::tick(&ui.lua, 20.);
            // Restart restores configured initial velocity, retaining configured acceleration.
            let expected = if at == 0. {
                0x3e1dd521
            } else if vt > 0. {
                0x3e1ba5e4
            } else {
                0x3d39af73
            };
            assert_eq!(value(&ui, scale).to_bits(), expected);
        }
    }
}

#[test]
fn scale_pivot_lookup_is_case_sensitive_and_unknown_strings_use_native_zero_origin() {
    for (pivot, expected) in [
        ("'CENTER'", [75., 90., 225., 150.]),
        ("'center'", [100., 100., 250., 160.]),
        ("'UNKNOWN'", [100., 100., 250., 160.]),
        ("''", [100., 100., 250., 160.]),
        (
            "'CENTER'..string.char(0)..'ignored'",
            [75., 90., 225., 150.],
        ),
    ] {
        let ui = setup();
        ui.exec(
            "@pivot",
            &format!("w:SetScaleAnimation(1.5,1,.3,0,{pivot});w:SetStartAnimation(false,true)"),
        )
        .unwrap();
        let q = &render::build_quads(&mut lua::ui(&ui.lua), None)[0];
        assert_eq!(q.pos, expected, "{pivot}");
        assert_eq!(
            input::pick(
                &ui.lua,
                (expected[0] + expected[2]) * 0.5,
                (expected[1] + expected[3]) * 0.5
            ),
            Some(id(&ui))
        );
    }
}

#[test]
fn raw_zero_and_nonfinite_times_do_not_gain_an_invented_duration_completion() {
    for scale in [false, true] {
        let ui = setup();
        let configure = if scale {
            "SetScaleAnimation"
        } else {
            "SetAlphaAnimation"
        };
        let pivot = if scale { ",'CENTER'" } else { "" };
        let start = if scale { "false,true" } else { "true,false" };
        ui.exec(
            "@raw_timing",
            &format!("w:{configure}(.2,.9,0,0{pivot});w:SetStartAnimation({start})"),
        )
        .unwrap();
        assert_eq!(value(&ui, scale).to_bits(), 0x3e4ccccd);
        lua::tick(&ui.lua, 0.);
        assert!(value(&ui, scale).is_nan());
        assert!(eval::<bool>(&ui, "w:IsNowAnimation()"));
        assert_eq!(eval::<i64>(&ui, "ends"), 0);
    }
    let ui = setup();
    ui.exec(
        "@nan_timing",
        "w:SetAlphaAnimation(.2,.9,.3,0/0);w:SetStartAnimation(true,false)",
    )
    .unwrap();
    lua::tick(&ui.lua, 20.);
    assert!(value(&ui, false).is_nan());
    assert_eq!(eval::<i64>(&ui, "ends"), 0);
    assert!(eval::<bool>(&ui, "w:IsNowAnimation()"));
}

#[test]
fn original_window_onshow_reconfiguration_keeps_channels_and_physical_button_targets() {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    original::install(&mut ui);
    original::step(&ui, 0);
    lua::tick(&ui.lua, 80.);
    let before: f32 = eval(&ui, "windows[1]:GetAlpha()");
    original::step(&ui, 1);
    assert_eq!(eval::<f32>(&ui, "windows[1]:GetAlpha()"), before);
    assert!(eval::<bool>(&ui, "windows[1]:IsNowAnimation()"));
    lua::tick(&ui.lua, 100.);
    assert!((eval::<f32>(&ui, "windows[1]:GetAlpha()") - before - 0.1).abs() < 0.00001);
    let retained: f32 = eval(&ui, "windows[2]:GetAlpha()");
    let replaced: f32 = eval(&ui, "windows[3]:GetAlpha()");
    assert!(retained > replaced + 0.01);
    for i in 1..=3 {
        let value = eval(&ui, &format!("windows[{i}].inside"));
        let id = lua::value_id(&ui.lua, &value).unwrap();
        let r = {
            let u = lua::ui(&ui.lua);
            u.animations.widget_rect(&u.tree, id)
        };
        assert_eq!(
            input::pick(&ui.lua, r.x + r.w * 0.5, r.y + r.h * 0.5),
            Some(id)
        );
    }
    assert!(ui.errors.is_empty());
}

#[test]
fn scale_end_observes_old_geometry_and_captured_target_survives_a_nested_restart() {
    let ui = setup();
    observe_scale(&ui);
    ui.exec("@scale_end_restart",r#"
        log={};ended=0
        w:SetHandler('OnScale',function(self,...) assert(select('#',...)==0);table.insert(log,observedScale(self)) end)
        w:SetHandler('OnScaleAnimeEnd',function(self,...)
            assert(select('#',...)==0 and not self:IsNowAnimation())
            ended=ended+1;table.insert(log,observedScale(self))
            self:SetScaleAnimation(.5,1.25,.3,0,'CENTER');self:SetStartAnimation(false,true)
            assert(#log==2 and log[2]==.5)
            table.insert(log,observedScale(self))
        end)
        w:SetScaleAnimation(1.01,1,.01,0,'CENTER');w:SetStartAnimation(false,true)
        log={}
    "#).unwrap();
    lua::tick(&ui.lua, 20.);
    let log: Vec<f32> = eval(&ui, "log");
    assert_eq!(
        log.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
        [
            1.01f32.to_bits(),
            0.5f32.to_bits(),
            0.5f32.to_bits(),
            1f32.to_bits()
        ]
    );
    assert_eq!(eval::<i64>(&ui, "ended"), 1);
    assert!(eval::<bool>(&ui, "w:IsNowAnimation()"));
    lua::tick(&ui.lua, 20.);
    assert_eq!(value(&ui, true).to_bits(), 1.05f32.to_bits());
    assert_eq!(eval::<i64>(&ui, "ended"), 1);
}

#[test]
fn alpha_end_can_enable_scale_for_this_frame_without_restepping_new_alpha() {
    for scale in [false, true] {
        let ui = setup();
        observe_scale(&ui);
        let action = if scale {
            "self:SetScaleAnimation(1.5,1,.1,0,'CENTER');self:SetStartAnimation(false,true)"
        } else {
            "self:SetAlphaAnimation(.1,.7,.2,0);self:SetStartAnimation(true,false)"
        };
        ui.exec(
            "@alpha_end_action",
            &format!(
                r#"
            log={{}};ended=0
            w:SetHandler('OnScale',function(self) table.insert(log,observedScale(self)) end)
            w:SetHandler('OnAlphaAnimeEnd',function(self,...)
                assert(select('#',...)==0 and not self:IsNowAnimation())
                ended=ended+1;assert(math.abs(self:GetAlpha()-.9)<.00001)
                {action}
            end)
            w:SetAlphaAnimation(.85,.9,.01,0);w:SetStartAnimation(true,false)
        "#
            ),
        )
        .unwrap();
        lua::tick(&ui.lua, 20.);
        assert_eq!(eval::<i64>(&ui, "ended"), 1);
        if scale {
            let log: Vec<f32> = eval(&ui, "log");
            assert_eq!(log, [1.5, 1.4]);
            assert_eq!(value(&ui, true).to_bits(), 1.4f32.to_bits());
        } else {
            assert_eq!(value(&ui, false).to_bits(), 0.1f32.to_bits());
        }
        assert!(eval::<bool>(&ui, "w:IsNowAnimation()"));
    }
}

#[test]
fn scale_end_can_enable_alpha_and_hide_without_skipping_the_captured_scale_continuation() {
    let ui = setup();
    observe_scale(&ui);
    ui.exec("@scale_end_alpha",r#"
        log={}
        w:SetHandler('OnScale',function(self) table.insert(log,{observedScale(self),self:IsVisible()}) end)
        w:SetHandler('OnScaleAnimeEnd',function(self)
            self:SetAlphaAnimation(.1,.7,.2,0);self:SetStartAnimation(true,false)
            self:Show(false)
        end)
        w:SetScaleAnimation(1.01,1,.01,0,'CENTER');w:SetStartAnimation(false,true)
        log={}
    "#).unwrap();
    lua::tick(&ui.lua, 20.);
    assert_eq!(value(&ui, false).to_bits(), 0.1f32.to_bits());
    assert_eq!(value(&ui, true).to_bits(), 1f32.to_bits());
    assert!(!eval::<bool>(&ui, "w:IsVisible()"));
    assert!(eval::<bool>(
        &ui,
        "#log==1 and log[1][1]==1 and log[1][2]==false"
    ));
    assert!(eval::<bool>(&ui, "w:IsNowAnimation()"));
}

#[test]
fn owned_scale_methods_flush_before_return_and_release_in_end_suppresses_the_following_handler() {
    let ui = setup();
    observe_scale(&ui);
    ui.exec(
        "@synchronous_scale",
        r#"
        n=0;w:SetHandler('OnScale',function(self,...) assert(select('#',...)==0);n=n+1 end)
        w:SetScale(.7);assert(n==1)
        w:SetScaleAnimation(1.01,1,.01,0,'CENTER');w:SetStartAnimation(false,true);assert(n==2)
        w:InheritAnimationData(w);assert(n==3)
        w:SetHandler('OnScaleAnimeEnd',function(self) self:ReleaseHandler('OnScale') end)
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 20.);
    assert_eq!(eval::<i64>(&ui, "n"), 3);
    assert_eq!(value(&ui, true).to_bits(), 1f32.to_bits());
    assert!(!eval::<bool>(&ui, "w:IsNowAnimation()"));
}

#[test]
fn original_achievement_installs_its_scale_handler_between_old_and_final_geometry() {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    original_callbacks::install(&mut ui);
    original_callbacks::demonstrate(&ui);
    let close = lua::value_id(&ui.lua, &eval(&ui, "achievement.titleBar.closeButton")).unwrap();
    let r = {
        let u = lua::ui(&ui.lua);
        u.animations.widget_rect(&u.tree, close)
    };
    let point = (r.x + r.w * 0.5, r.y + r.h * 0.5);
    assert_eq!(input::pick(&ui.lua, point.0, point.1), Some(close));
    let mut mouse = Default::default();
    input::update(&ui.lua, &mut mouse, Some(point), true, false, false, 0.);
    input::update(&ui.lua, &mut mouse, Some(point), false, true, false, 0.);
    assert!(!eval::<bool>(&ui, "achievement:IsVisible()"));
    assert!(ui.errors.is_empty());
}

#[test]
fn hidden_end_callback_still_gets_final_scale_before_deferred_retirement() {
    let ui = setup();
    observe_scale(&ui);
    let w = id(&ui);
    ui.exec(
        "@retire_after_end",
        r#"
        w:EnableHidingIsRemove(true)
        lastScale=nil
        w:SetHandler('OnScale',function(self) lastScale=observedScale(self) end)
        w:SetHandler('OnScaleAnimeEnd',function(self) self:Show(false) end)
        w:SetScaleAnimation(1.01,1,.01,0,'CENTER');w:SetStartAnimation(false,true)
        lastScale=nil
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 20.);
    assert_eq!(eval::<f32>(&ui, "lastScale"), 1.);
    assert!(!eval::<bool>(&ui, "w:IsVisible()"));
    assert!(lua::ui(&ui.lua).tree.get(w).alive);
    lua::tick(&ui.lua, 0.);
    assert!(!lua::ui(&ui.lua).tree.get(w).alive);
}
