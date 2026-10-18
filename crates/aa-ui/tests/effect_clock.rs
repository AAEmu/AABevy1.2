//! Executed3928e6c0/3928f460 phase goldens and unchanged original health consumers.
use aa_ui::{UiRuntime, lua, render::Quad};
#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;

fn eval<T: mlua::FromLuaMulti>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().expect(code)
}
fn id(ui: &UiRuntime, name: &str) -> u32 {
    lua::value_id(&ui.lua, &eval(ui, name)).unwrap()
}
fn sample(ui: &UiRuntime) -> lua::animation::Sample {
    lua::ui(&ui.lua).animations.sample(id(ui, "e"))
}
fn frame(ui: &UiRuntime, ms: f32) -> Vec<Quad> {
    lua::tick(&ui.lua, ms);
    aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None)
}
fn setup(extra: &str) -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    ui.exec(
        "@effect_clock",
        r#"
        host=UIParent:CreateWidget('window','effectClockHost','UIParent')
        host:SetExtent(200,100);host:Show(true)
        e=host:CreateEffectDrawable('ui/common/hud.dds','overlay')
        e:SetExtent(40,40);e:SetCoords(462,233,3,3);e:AddAnchor('CENTER',host,0,0)
        e:SetRepeatCount(1);e:SetEffectPriority(1,'alpha',0.5,0.4)
        e:SetEffectInitialColor(1,1,1,1,0);e:SetEffectFinalColor(1,1,1,1,1)
    "#,
    )
    .unwrap();
    ui.exec("@effect_configuration", extra).unwrap();
    lua::tick(&ui.lua, 0.);
    ui.exec("@start", "e:SetStartEffect(true)").unwrap();
    ui
}
fn bits(actual: f32, expected: f32) {
    assert_eq!(
        actual.to_bits(),
        expected.to_bits(),
        "{actual} != {expected}"
    );
}

#[test]
fn overshoot_stops_at_one_handoff_and_preserves_the_new_initial_sample() {
    let ui = setup(
        r#"
        e:SetEffectInterval(1,0.1)
        e:SetEffectPriority(2,'alpha',0.3,0.2)
        e:SetEffectInitialColor(2,1,1,1,0.9);e:SetEffectFinalColor(2,1,1,1,0)
    "#,
    );
    frame(&ui, 1000.);
    bits(sample(&ui).color.unwrap()[3], 1.);
    frame(&ui, 0.);
    bits(sample(&ui).color.unwrap()[3], 0.9);
    frame(&ui, 100.);
    bits(sample(&ui).color.unwrap()[3], 0.);
    // Empty motion clears its own flag; native IsRunning is an AND, not color OR motion.
    assert!(!eval::<bool>(&ui, "e:IsRunning()"));
    frame(&ui, 1000.);
    bits(sample(&ui).color.unwrap()[3], 0.);
}

#[test]
fn hidden_effects_and_unpainted_frames_pause_but_zero_alpha_and_clipping_do_not() {
    let ui = setup("");
    lua::tick(&ui.lua, 100.);
    bits(sample(&ui).color.unwrap()[3], 0.);
    aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None);
    bits(sample(&ui).color.unwrap()[3], 0.21250000596046448);
    ui.exec("@hide", "host:Show(false)").unwrap();
    assert!(frame(&ui, 500.).is_empty());
    bits(sample(&ui).color.unwrap()[3], 0.21250000596046448);
    ui.exec("@alpha0", "host:Show(true);host:SetAlpha(0)")
        .unwrap();
    assert!(frame(&ui, 100.).is_empty());
    bits(sample(&ui).color.unwrap()[3], 0.44999998807907104);
    ui.exec(
        "@clip",
        "host:SetAlpha(1);host:EnableScroll(true);host:SetExtent(0,0)",
    )
    .unwrap();
    frame(&ui, 100.);
    bits(sample(&ui).color.unwrap()[3], 0.7125000357627869);
    ui.exec("@offscreen", "host:AddAnchor('TOPLEFT',UIParent,5001,5001)")
        .unwrap();
    frame(&ui, 100.);
    bits(sample(&ui).color.unwrap()[3], 1.);
}

#[test]
fn fading_ancestor_admits_effect_clock_then_retirement_releases_its_repaint_state() {
    let ui = setup("");
    ui.exec("@hidden_fading", "host:Show(false,1000)").unwrap();
    assert!(!eval::<bool>(&ui, "host:IsVisible()"));
    assert!(!frame(&ui, 100.).is_empty());
    bits(sample(&ui).color.unwrap()[3], 0.21250000596046448);
    assert!(frame(&ui, 1000.).is_empty());
    bits(sample(&ui).color.unwrap()[3], 0.21250000596046448);
    ui.exec("@retire", "host:RemoveAllDrawables()").unwrap();
    assert_eq!(sample(&ui), Default::default());
    let content = lua::ui(&ui.lua).tree.settle();
    lua::tick(&ui.lua, 500.);
    assert_eq!(lua::ui(&ui.lua).tree.settle(), content);
    assert!(aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None).is_empty());
}

#[test]
fn manual_show_advances_stopped_flags_without_a_restart_or_reinitialization() {
    let ui = setup("");
    frame(&ui, 100.);
    ui.exec("@stop", "e:SetStartEffect(false)").unwrap();
    frame(&ui, 500.);
    bits(sample(&ui).color.unwrap()[3], 0.21250000596046448);
    ui.exec("@manual", "e:SetVisible(true);assert(not e:IsRunning())")
        .unwrap();
    let quads = frame(&ui, 100.);
    bits(sample(&ui).color.unwrap()[3], 0.44999998807907104);
    bits(quads[0].color[3], 0.44999998807907104);
    assert!(!eval::<bool>(&ui, "e:IsRunning()"));
}

#[test]
fn native_delay_rounding_and_zero_coefficients_do_not_jump_to_the_final_value() {
    let ui = setup("e:SetInterval(0.2)");
    assert!(frame(&ui, 100.).is_empty());
    assert!(frame(&ui, 100.).is_empty());
    frame(&ui, 100.);
    bits(sample(&ui).color.unwrap()[3], 0.21250002086162567);
    ui.exec(
        "@zero_coefficients",
        "e:SetInterval(0);e:SetEffectPriority(1,'alpha',0,0);e:SetStartEffect(true)",
    )
    .unwrap();
    frame(&ui, 100.);
    bits(sample(&ui).color.unwrap()[3], 0.);
    frame(&ui, 5000.);
    bits(sample(&ui).color.unwrap()[3], 0.);
}

#[test]
fn independent_channels_continue_after_is_running_becomes_false() {
    let ui = setup(
        r#"
        e:SetEffectPriority(1,'alpha',0.2,0)
        e:SetMoveRepeatCount(1);e:SetMoveInterval(0.1)
        e:SetMoveEffectType(1,'top',0.5,0.5,0.4,0);e:SetMoveEffectEdge(1,0,1)
    "#,
    );
    assert!(eval::<bool>(&ui, "e:IsRunning()"));
    frame(&ui, 100.);
    bits(sample(&ui).color.unwrap()[3], 0.5);
    frame(&ui, 100.);
    assert!(eval::<bool>(&ui, "e:IsRunning()"));
    frame(&ui, 100.);
    assert!(!eval::<bool>(&ui, "e:IsRunning()"));
    bits(sample(&ui).offset[0], 0.);
    frame(&ui, 100.);
    bits(sample(&ui).offset[0], 10.);
    bits(sample(&ui).color.unwrap()[3], 1.);
}

#[test]
fn live_phase_changes_retain_latched_coefficients_and_clear_retains_sample_storage() {
    let ui = setup(
        r#"
        e:SetMoveRepeatCount(1)
        e:SetMoveEffectType(1,'top',0.5,0.5,0.5,0.4);e:SetMoveEffectEdge(1,0,1)
    "#,
    );
    frame(&ui, 100.);
    let before = sample(&ui);
    ui.exec(
        "@reconfigure",
        r#"
        e:SetEffectPriority(1,'alpha',9,9);e:SetEffectFinalColor(1,1,1,1,2)
        e:SetMoveEffectEdge(1,0,2)
    "#,
    )
    .unwrap();
    frame(&ui, 100.);
    bits(sample(&ui).color.unwrap()[3], 0.44999998807907104);
    bits(sample(&ui).offset[0], -2.);
    let retained = sample(&ui);
    assert_ne!(before, retained);
    ui.exec(
        "@clear_move",
        "e:ClearMoveEffect();assert(not e:IsVisible() and not e:IsRunning())",
    )
    .unwrap();
    assert_eq!(sample(&ui), retained);
    ui.exec("@show_color", "e:SetVisible(true)").unwrap();
    frame(&ui, 100.);
    bits(sample(&ui).offset[0], 0.);
    assert!(sample(&ui).color.unwrap()[3] > retained.color.unwrap()[3]);
    let retained = sample(&ui);
    ui.exec("@clear_color", "e:ClearEffect()").unwrap();
    assert_eq!(sample(&ui), retained);
    ui.exec("@empty", "e:SetVisible(true)").unwrap();
    assert!(frame(&ui, 100.).is_empty());
}

#[test]
fn native_default_repeat_is_unbounded_and_a_motion_only_effect_has_no_color_art() {
    let ui = setup(
        r#"
        e:SetStartEffect(false)
        e=host:CreateEffectDrawable('ui/common/hud.dds','overlay')
        e:SetExtent(40,40);e:SetCoords(462,233,3,3);e:AddAnchor('CENTER',host,0,0)
        e:SetEffectPriority(1,'alpha',0.5,0.4)
        e:SetEffectInitialColor(1,1,1,1,0);e:SetEffectFinalColor(1,1,1,1,1)
    "#,
    );
    frame(&ui, 400.);
    bits(sample(&ui).color.unwrap()[3], 1.);
    frame(&ui, 100.);
    bits(sample(&ui).color.unwrap()[3], 0.);
    frame(&ui, 100.);
    assert!(sample(&ui).color.unwrap()[3] > 0.);
    ui.exec(
        "@motion_only",
        r#"
        e:ClearEffect();e:SetMoveEffectType(1,'top',0.5,0.5,1,0)
        e:SetMoveEffectEdge(1,0,1);e:SetStartEffect(true)
    "#,
    )
    .unwrap();
    assert!(frame(&ui, 100.).is_empty());
    assert!(!eval::<bool>(&ui, "e:IsRunning()"));
}

#[test]
fn getter_and_clear_use_the_native_effect_cast_and_exact_return_arity() {
    let ui = setup("");
    ui.exec(
        "@class_gates",
        r#"
        image=host:CreateImageDrawable('ui/common/hud.dds','overlay');image:SetVisible(true)
        assert(select('#',e.IsRunning(image))==0)
        assert(select('#',e.ClearEffect(image))==0)
        assert(select('#',e.ClearMoveEffect(image))==0)
        assert(image:IsVisible())
        assert(select('#',e:IsRunning('ignored'))==1)
        assert(select('#',e:ClearMoveEffect('ignored'))==0)
        assert(not e:IsRunning())
    "#,
    )
    .unwrap();
}

#[test]
fn seven_native_priority_branches_keep_raw_f32_interpolation_and_scaley_quirk() {
    let expected = [
        (
            [
                0.27000001072883606,
                0.32750001549720764,
                0.3850000202655792,
                0.4425000250339508,
            ],
            [0.44875001907348633, 0.7125000357627869],
        ),
        (
            [
                0.8999999761581421,
                0.800000011920929,
                0.699999988079071,
                0.6000000238418579,
            ],
            [1., 1.5],
        ),
        (
            [
                0.27000001072883606,
                0.32750001549720764,
                0.3850000202655792,
                0.4424999952316284,
            ],
            [0.44875001907348633, 0.7124999761581421],
        ),
        (
            [
                0.2699999511241913,
                0.32749998569488525,
                0.38499999046325684,
                0.4424999952316284,
            ],
            [0.44874995946884155, 0.7124999165534973],
        ),
        (
            [
                0.26999998092651367,
                0.32749998569488525,
                0.38499999046325684,
                0.4424999952316284,
            ],
            [0.44875001907348633, 0.7124999761581421],
        ),
        (
            [
                0.27000001072883606,
                0.32750001549720764,
                0.3850000202655792,
                0.4424999952316284,
            ],
            [0.44875001907348633, 0.7124999761581421],
        ),
        (
            [
                0.2699999511241913,
                0.32749998569488525,
                0.38499999046325684,
                0.4424999952316284,
            ],
            [0.44874998927116394, 0.7124999761581421],
        ),
    ];
    for (i, priority) in [
        "scalex", "scaley", "rotate", "alpha", "colorr", "colorg", "colorb",
    ]
    .iter()
    .enumerate()
    {
        let ui = setup(&format!(
            r#"
            e:SetEffectPriority(1,'{priority}',0.5,0.4)
            e:SetEffectScale(1,0.3,1,0.5,1.5);e:SetEffectRotate(1,10,90)
            e:SetEffectInitialColor(1,0.1,0.2,0.3,0.4)
            e:SetEffectFinalColor(1,0.9,0.8,0.7,0.6)
        "#
        ));
        frame(&ui, 100.);
        let actual = sample(&ui);
        for (a, b) in actual.color.unwrap().into_iter().zip(expected[i].0) {
            bits(a, b);
        }
        for (a, b) in actual.scale.into_iter().zip(expected[i].1) {
            bits(a, b);
        }
    }
}

#[test]
fn original_zero_health_re_shows_stopped_warning_and_recovery_hides_it() {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    fake::install(&mut ui, "en_us", &fake::default_worlds());
    fake::install_ingame(&mut ui);
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    let player = lua::ui(&ui.lua).tree.find("playerFrame").unwrap();
    ui.lua
        .globals()
        .raw_set(
            "effectPlayerFixture",
            lua::object_table(&ui.lua, player).unwrap().unwrap(),
        )
        .unwrap();
    ui.exec(
        "@original_low",
        r#"
        PlayerLifeAlertEffectVisibleHandler(true)
        FAKE.player.hp=FAKE.player.maxHp/5;effectPlayerFixture:UpdateAll()
    "#,
    )
    .unwrap();
    frame(&ui, 100.);
    let warning = id(&ui, "main_menu_bar.life_alert_effect");
    let before = lua::ui(&ui.lua).animations.sample(warning);
    ui.exec(
        "@original_zero",
        r#"
        FAKE.player.hp=FAKE.player.maxHp;effectPlayerFixture:UpdateAll()
        FAKE.player.hp=0;effectPlayerFixture:UpdateAll()
        assert(main_menu_bar.life_alert_effect:IsVisible())
        assert(not main_menu_bar.life_alert_effect:IsRunning())
    "#,
    )
    .unwrap();
    frame(&ui, 100.);
    let after = lua::ui(&ui.lua).animations.sample(warning);
    assert!(after.color.unwrap()[3] < before.color.unwrap()[3]);
    assert!(!eval::<bool>(
        &ui,
        "main_menu_bar.life_alert_effect:IsRunning()"
    ));
    ui.exec(
        "@recovered",
        "FAKE.player.hp=FAKE.player.maxHp;effectPlayerFixture:UpdateAll()",
    )
    .unwrap();
    assert!(!eval::<bool>(
        &ui,
        "main_menu_bar.life_alert_effect:IsVisible()"
    ));
}

#[test]
fn original_loading_texture_rotates_on_paint_and_pauses_with_its_owner() {
    let ui = setup("e:SetStartEffect(false);TEXTURE_PATH={DEFAULT='ui/common/default.dds'}");
    aa_lua::run_alb(&ui.lua, "scriptsbin/x2ui/baselib/common_texture.alb").unwrap();
    ui.exec(
        "@original_loading",
        r#"
        CreateLoadingTextureSet(host)
        loading=host.modalLoadingWindow;loading:SetExtent(100,100);loading:Show(true)
        loading.effect:SetStartEffect(true)
    "#,
    )
    .unwrap();
    let effect = id(&ui, "loading.effect");
    frame(&ui, 375.);
    bits(
        lua::ui(&ui.lua).animations.sample(effect).rotation,
        90_f32.to_radians(),
    );
    assert!(!eval::<bool>(&ui, "loading.effect:IsRunning()")); // Empty motion flag, rotating color channel.
    ui.exec("@pause_loading", "loading:Show(false)").unwrap();
    frame(&ui, 1000.);
    bits(
        lua::ui(&ui.lua).animations.sample(effect).rotation,
        90_f32.to_radians(),
    );
    ui.exec("@resume_loading", "loading:Show(true)").unwrap();
    frame(&ui, 375.);
    bits(
        lua::ui(&ui.lua).animations.sample(effect).rotation,
        180_f32.to_radians(),
    );
}
