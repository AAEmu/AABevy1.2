//! Executed native configure/start/update goldens; shared effect Timing is independent.
use aa_ui::{UiRuntime, input, lua, render};

#[path = "../examples/native_widget_channel.rs"]
#[allow(dead_code)]
mod original;

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
