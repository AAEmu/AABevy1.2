//! Drawable timelines exercised through the native Lua API and original .alb helpers.

use aa_ui::{UiRuntime, lua};

fn eval<T: mlua::FromLua>(ui: &UiRuntime, source: &str) -> T {
    ui.lua.load(source).eval().expect(source)
}

fn id(ui: &UiRuntime, source: &str) -> u32 {
    lua::value_id(&ui.lua, &eval::<mlua::Value>(ui, source)).unwrap()
}

fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920.0, 1080.0).unwrap();
    ui.exec(
        "@setup",
        r#"
        window = UIParent:CreateWidget("window", "animationTest", "UIParent")
        window:SetExtent(300, 100)
        window:Show(true)
        effect = window:CreateEffectDrawable("ui/common/hud.dds", "artwork")
        effect:SetCoords(462, 233, 3, 3)
        effect:SetExtent(42, 42)
        effect:AddAnchor("CENTER", window, 0, 0)
    "#,
    )
    .unwrap();
    // Settle the native zero-duration Show record before animation/cache measurements.
    lua::tick(&ui.lua, 0.0);
    ui
}

fn sample(ui: &UiRuntime, name: &str) -> lua::animation::Sample {
    let id = id(ui, name);
    lua::ui(&ui.lua).animations.sample(id)
}

fn frame(ui: &UiRuntime, ms: f32) -> Vec<aa_ui::render::Quad> {
    lua::tick(&ui.lua, ms);
    aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None)
}

fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 0.0001, "{a} != {b}");
}

#[test]
fn effects_interpolate_phases_repeat_and_stop_without_moving_layout() {
    let ui = setup();
    ui.exec(
        "@phases",
        r#"
        effect:SetRepeatCount(0)
        effect:SetEffectPriority(1, "alpha", 0.5, 0.5)
        effect:SetEffectInitialColor(1, 1, 1, 1, 0)
        effect:SetEffectFinalColor(1, 1, 1, 1, 0.6)
        effect:SetEffectScale(1, 0.3, 1, 0.3, 1)
        effect:SetEffectRotate(1, 0, 180)
        effect:SetEffectPriority(2, "alpha", 0.5, 0.5)
        effect:SetEffectInitialColor(2, 1, 1, 1, 0.6)
        effect:SetEffectFinalColor(2, 1, 1, 1, 0.2)
        effect:SetStartEffect(true)
    "#,
    )
    .unwrap();
    near(sample(&ui, "effect").color.unwrap()[3], 0.0);
    let d = id(&ui, "effect");
    let rect = lua::ui(&ui.lua).tree.rect(d);
    lua::ui(&ui.lua).tree.settle();
    let generation = lua::ui(&ui.lua).tree.generation;
    frame(&ui, 250.0);
    let s = sample(&ui, "effect");
    near(s.color.unwrap()[3], 0.3);
    near(s.scale[0], 0.65);
    near(s.rotation, std::f32::consts::FRAC_PI_2);
    assert_eq!(lua::ui(&ui.lua).tree.rect(d), rect);
    assert_eq!(lua::ui(&ui.lua).tree.generation, generation);
    frame(&ui, 500.0);
    near(sample(&ui, "effect").color.unwrap()[3], 0.6);
    frame(&ui, 2500.0);
    // Native keeps the next phase's initial sample on the handoff frame.
    near(sample(&ui, "effect").color.unwrap()[3], 0.6);
    ui.exec("@stop", "effect:SetStartEffect(false)").unwrap();
    let stopped = sample(&ui, "effect");
    lua::ui(&ui.lua).tree.settle();
    let content = lua::ui(&ui.lua).tree.content();
    frame(&ui, 400.0);
    assert_eq!(sample(&ui, "effect"), stopped);
    assert_eq!(lua::ui(&ui.lua).tree.settle(), content);
    assert!(!eval::<bool>(&ui, "effect:IsRunning()"));
    ui.exec("@already_stopped", "effect:SetStartEffect(false)")
        .unwrap();
    assert_eq!(lua::ui(&ui.lua).tree.settle(), content);
    ui.exec("@restart", "effect:SetStartEffect(true)").unwrap();
    near(sample(&ui, "effect").color.unwrap()[3], 0.0);
}

#[test]
fn finite_effects_hold_then_finish_and_hidden_loops_do_not_repaint() {
    let ui = setup();
    ui.exec(
        "@finite",
        r#"
        effect:SetRepeatCount(2)
        effect:SetInterval(0.1)
        effect:SetEffectPriority(1, "alpha", 0.2, 0.2)
        effect:SetEffectInitialColor(1, 1, 1, 1, 0)
        effect:SetEffectFinalColor(1, 1, 1, 1, 0.4)
        effect:SetEffectInterval(1, 0.1)
        effect:SetStartEffect(true)
    "#,
    )
    .unwrap();
    near(sample(&ui, "effect").color.unwrap()[3], 0.0);
    frame(&ui, 350.0);
    near(sample(&ui, "effect").color.unwrap()[3], 0.4);
    assert!(!eval::<bool>(&ui, "effect:IsRunning()")); // Empty native motion channel.
    frame(&ui, 1000.0);
    near(sample(&ui, "effect").color.unwrap()[3], 0.0);
    frame(&ui, 0.0);
    frame(&ui, 0.0);
    near(sample(&ui, "effect").color.unwrap()[3], 0.4);
    assert!(!eval::<bool>(&ui, "effect:IsRunning()"));
    assert!(lua::ui(&ui.lua).tree.get(id(&ui, "effect")).shown);
    ui.exec(
        "@hidden",
        "effect:SetRepeatCount(0); effect:SetStartEffect(true); window:Show(false)",
    )
    .unwrap();
    frame(&ui, 0.0);
    lua::ui(&ui.lua).tree.settle();
    let content = lua::ui(&ui.lua).tree.content();
    frame(&ui, 200.0);
    near(sample(&ui, "effect").color.unwrap()[3], 0.0);
    assert_eq!(lua::ui(&ui.lua).tree.settle(), content);
    let retained = sample(&ui, "effect");
    ui.exec("@clear", "effect:ClearEffect()").unwrap();
    assert_eq!(sample(&ui, "effect"), retained);
    assert!(!eval::<bool>(&ui, "effect:IsRunning()"));
}

#[test]
fn atlas_animation_second_argument_controls_repeat_and_step_frames_are_cached() {
    let ui = setup();
    ui.exec(
        "@atlas",
        r#"
        atlas = window:CreateImageDrawable("ui/hud.dds", "overlay")
        atlas:SetExtent(21, 30)
        atlas:SetAnimFrameInfo({
            {x = 118, y = 196, w = 21, h = 30, time = 150, scale = 1},
            {x = 139, y = 196, w = 21, h = 30, time = 150, scale = 1},
            {x = 160, y = 196, w = 21, h = 30, time = 150, scale = 1},
            {x = 139, y = 196, w = 21, h = 30, time = 150, scale = 1}
        })
        atlas:Animation(true, true)
    "#,
    )
    .unwrap();
    assert_eq!(sample(&ui, "atlas").coords.unwrap()[0], 118.0);
    lua::ui(&ui.lua).tree.settle();
    let content = lua::ui(&ui.lua).tree.content();
    lua::tick(&ui.lua, 100.0);
    assert_eq!(lua::ui(&ui.lua).tree.settle(), content);
    lua::tick(&ui.lua, 75.0);
    assert_eq!(sample(&ui, "atlas").coords.unwrap()[0], 139.0);
    lua::tick(&ui.lua, 600.0);
    assert_eq!(sample(&ui, "atlas").coords.unwrap()[0], 139.0);
    assert!(eval::<bool>(&ui, "atlas:IsVisible()"));
    ui.exec("@once", "atlas:Animation(true, false)").unwrap();
    assert!(eval::<bool>(&ui, "atlas:IsVisible()"));
    lua::tick(&ui.lua, 600.0);
    assert!(!eval::<bool>(&ui, "atlas:IsVisible()"));
    ui.exec("@stop", "atlas:Animation(false, false)").unwrap();
    lua::tick(&ui.lua, 600.0);
    assert!(!eval::<bool>(&ui, "atlas:IsVisible()"));
}

#[test]
fn original_arrow_helper_moves_back_and_forth_then_fades_out() {
    let ui = setup();
    ui.exec(
        "@textures",
        "TEXTURE_PATH = {HUD = 'ui/hud.dds', QUEST_LIST = 'ui/quest_list.dds'}",
    )
    .unwrap();
    aa_lua::run_alb(&ui.lua, "scriptsbin/x2ui/baselib/common_texture.alb").unwrap();
    ui.exec(
        "@arrow",
        "arrow = DrawArrowMoveAnim(window, 'v'); arrow:Animation(true, false)",
    )
    .unwrap();
    lua::tick(&ui.lua, 350.0);
    near(sample(&ui, "arrow").offset[1], 15.0);
    lua::tick(&ui.lua, 300.0);
    near(sample(&ui, "arrow").offset[1], 15.0);
    lua::tick(&ui.lua, 1100.0);
    near(sample(&ui, "arrow").alpha, 0.5);
    lua::tick(&ui.lua, 100.0);
    assert!(!eval::<bool>(&ui, "arrow:IsVisible()"));
    ui.exec("@remove", "window:RemoveAllDrawables()").unwrap();
    assert_eq!(sample(&ui, "arrow"), Default::default());
}

#[test]
fn original_action_slot_bytecode_pulses_autoattack_and_combo_borders() {
    let ui = setup();
    ui.exec("@slot_helpers", r#"
        ICON_SIZE = {DEFAULT = 42}
        TEXTURE_PATH = {HUD = "ui/hud.dds", SKILL = "ui/skill.dds"}
        FONT_PATH = {DEFAULT = "font_main", SNAIL = "font_main"}
        FONT_SIZE = {SMALL = 12, XLARGE = 18, XXLARGE = 20}
        FONT_COLOR = {SOFT_RED = {1, 0, 0, 1}, RED = {1, 0, 0, 1}, WHITE = {1, 1, 1, 1}}
        function ApplyTextColor(widget, color) widget.style:SetColor(unpack(color)) end
        function ConvertColor(value) return value / 255 end
        function HideTooltip() end
        function CreatePackIcon(slot)
          slot.packIcon = slot:CreateColorDrawable(1, 1, 1, 0, "artwork")
          slot.SetPackInfo = function() end
        end
        function CreateDyeingIcon(slot) slot.dyeingIcon = slot:CreateColorDrawable(1, 1, 1, 0, "artwork") end
        function CreateLookIcon(slot) slot.lookIcon = slot:CreateColorDrawable(1, 1, 1, 0, "artwork") end
        AAUI.SlotContent = function() return {type = "skill", id = 1, canUse = true} end
    "#).unwrap();
    aa_lua::run_alb(&ui.lua, "scriptsbin/x2ui/components/action_slot.alb").unwrap();
    ui.exec(
        "@slot",
        r#"
        NotifyEmptyActionBarSlotCount = function() end
        CancelAutoRegisteredEffect = function() end
        slot = CreateActionSlot(window, "animatedSlot", ISLOT_ACTION, 1)
        slot:Anim_auto_attack_effect(true)
        slot:Anim_combo_aciton_for_me_effect(true)
    "#,
    )
    .unwrap();
    near(
        sample(&ui, "slot.auto_attack_effect_bg").color.unwrap()[3],
        0.0,
    );
    near(
        sample(&ui, "slot.combo_action_for_me_effect")
            .color
            .unwrap()[3],
        0.4,
    );
    frame(&ui, 250.0);
    near(
        sample(&ui, "slot.auto_attack_effect_bg").color.unwrap()[3],
        0.3,
    );
    let combo = sample(&ui, "slot.combo_action_for_me_effect");
    near(combo.color.unwrap()[3], 0.746875);
    near(combo.scale[0], 0.7046875);
    let arrow = sample(&ui, "slot.auto_attack_effect_arrow");
    near(arrow.offset[0], 13.8564);
    near(arrow.offset[1], -8.0);
    near(arrow.rotation, std::f32::consts::FRAC_PI_3);
    frame(&ui, 350.0);
    near(
        sample(&ui, "slot.auto_attack_effect_bg").color.unwrap()[3],
        0.6,
    );
    near(
        sample(&ui, "slot.combo_action_for_me_effect")
            .color
            .unwrap()[3],
        1.0,
    );
    ui.exec("@reset", "slot:ResetAllAnimEffect()").unwrap();
    assert!(
        !lua::ui(&ui.lua)
            .tree
            .get(id(&ui, "slot.auto_attack_effect_bg"))
            .drawable()
            .unwrap()
            .effect
            .unwrap()
    );
}

#[test]
fn renderer_uses_animated_uv_color_scale_and_rotation() {
    let ui = setup();
    ui.exec(
        "@render",
        r#"
        effect:SetRepeatCount(1)
        effect:SetEffectPriority(1, "alpha", 1, 1)
        effect:SetEffectInitialColor(1, 1, 1, 1, 0)
        effect:SetEffectFinalColor(1, 1, 1, 1, 1)
        effect:SetEffectScale(1, 0, 1, 0, 1)
        effect:SetEffectRotate(1, 0, 180)
        effect:SetStartEffect(true)
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 500.0);
    let quads = aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None);
    assert_eq!(quads.len(), 1);
    near(quads[0].color[3], 0.5);
    near(quads[0].pos[2] - quads[0].pos[0], 21.0);
    assert!(quads[0].corners.is_some());
    ui.exec("@pivot", "effect:SetEffectScaleAxis(1, 'TOPLEFT'); effect:SetEffectRotate(1, 0, 0); effect:SetStartEffect(true)").unwrap();
    lua::tick(&ui.lua, 500.0);
    let quads = aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None);
    let rect = lua::ui(&ui.lua).tree.rect(id(&ui, "effect"));
    near(quads[0].pos[0], rect.x);
    near(quads[0].pos[1], rect.y);
    near(quads[0].pos[2], rect.x + 21.0);
    ui.exec(
        "@atlas_render",
        r#"
        effect:SetStartEffect(false)
        atlas = window:CreateImageDrawable("ui/common/hud.dds", "overlay")
        atlas:SetExtent(21, 30)
        atlas:SetAnimFrameInfo({
            {x = 118, y = 196, w = 21, h = 30, time = 150},
            {x = 139, y = 196, w = 21, h = 30, time = 150}
        })
        atlas:Animation(true, true)
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 175.0);
    let quads = aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None);
    assert_eq!(quads.len(), 1);
    let width = lua::ui(&ui.lua)
        .textures
        .size("ui/common/hud.dds")
        .unwrap()
        .0 as f32;
    near(quads[0].uv[0] * width, 139.0);
    near(quads[0].uv[2] * width, 160.0);
}

#[test]
fn motion_runs_independently_rotates_art_and_clears_a_finished_channel() {
    let ui = setup();
    ui.exec(
        "@circle",
        r#"
        effect:SetEffectPriority(1, 'alpha', 0.2, 0.2)
        effect:SetEffectInitialColor(1, 1, 1, 1, 0)
        effect:SetEffectFinalColor(1, 1, 1, 1, 1)
        effect:SetMoveRepeatCount(1)
        effect:SetMoveEffectType(1, 'circle', 0.5, 0.25, 1, 0)
        effect:SetMoveEffectCircle(1, 0, 360)
        effect:SetStartEffect(true)
    "#,
    )
    .unwrap();
    near(sample(&ui, "effect").offset[1], -10.5);
    let quads = frame(&ui, 250.0);
    let s = sample(&ui, "effect");
    near(s.color.unwrap()[3], 1.0);
    near(s.offset[0], 21.0);
    near(s.offset[1], 0.0);
    near(s.rotation, std::f32::consts::FRAC_PI_2);
    assert!(eval::<bool>(&ui, "effect:IsRunning()"));
    assert_eq!(quads.len(), 1);
    near((quads[0].pos[0] + quads[0].pos[2]) * 0.5, 171.0);
    assert!(quads[0].corners.is_some());
    frame(&ui, 750.0);
    frame(&ui, 0.0); // Finishing the motion phase and dropping its flag are distinct draws.
    assert!(!eval::<bool>(&ui, "effect:IsRunning()"));
    near(sample(&ui, "effect").offset[1], -10.5);
    let retained = sample(&ui, "effect");
    ui.exec("@clear_motion", "effect:ClearMoveEffect()")
        .unwrap();
    assert_eq!(sample(&ui, "effect"), retained);
    assert!(!eval::<bool>(&ui, "effect:IsVisible()"));
    ui.exec("@replay_color", "effect:SetStartEffect(true)")
        .unwrap();
    frame(&ui, 100.0);
    near(sample(&ui, "effect").color.unwrap()[3], 0.5);
    near(sample(&ui, "effect").offset[0], 0.0);
}

#[test]
fn edge_motion_has_extent_relative_positions_orientation_delay_and_hold() {
    let ui = setup();
    ui.exec(
        "@edge",
        r#"
        effect:SetMoveRepeatCount(1)
        effect:SetMoveInterval(0.1)
        effect:SetMoveEffectType(1, 'top', 0, 0.25, 0.2, 0.2)
        effect:SetMoveEffectEdge(1, 0.5, 1.5)
        effect:SetMoveEffectInterval(1, 0.1)
        effect:SetMoveEffectType(2, 'right', 0.5, 0, 0.2, 0.2)
        effect:SetMoveEffectEdge(2, 0.5, 1.5)
        effect:SetStartEffect(true)
    "#,
    )
    .unwrap();
    near(sample(&ui, "effect").offset[0], 0.0);
    frame(&ui, 200.0);
    near(sample(&ui, "effect").offset[0], 21.0);
    near(sample(&ui, "effect").offset[1], -10.5);
    frame(&ui, 150.0);
    near(sample(&ui, "effect").offset[0], 42.0);
    frame(&ui, 150.0);
    near(sample(&ui, "effect").offset[0], 21.0);
    near(sample(&ui, "effect").offset[1], 0.0);
    frame(&ui, 0.0); // Native initializes the new phase before sampling its carried time.
    near(sample(&ui, "effect").offset[1], 21.0);
    near(sample(&ui, "effect").rotation, std::f32::consts::FRAC_PI_2);
    ui.exec("@no_rotate", "effect:SetMoveRotate(false)")
        .unwrap();
    frame(&ui, 10.0);
    near(sample(&ui, "effect").rotation, 0.0);
    frame(&ui, 200.0);
    assert!(!eval::<bool>(&ui, "effect:IsRunning()"));
}

#[test]
fn widget_animation_scales_descendants_about_the_anchor_and_finishes_once() {
    let ui = setup();
    ui.exec(
        "@widget",
        r#"
        child = window:CreateChildWidget('button', 'scaleChild', 0, true)
        child:SetExtent(40, 20)
        child:AddAnchor('TOPLEFT', window, 20, 20)
        local fill = child:CreateColorDrawable(1, 1, 1, 1, 'artwork')
        fill:AddAnchor('TOPLEFT', child, 0, 0)
        fill:AddAnchor('BOTTOMRIGHT', child, 0, 0)
        alphaEnded, scaleEnded = 0, 0
        window:SetHandler('OnAlphaAnimeEnd', function() alphaEnded = alphaEnded + 1 end)
        window:SetHandler('OnScaleAnimeEnd', function() scaleEnded = scaleEnded + 1 end)
        window:SetAlphaAnimation(0, 1, 0.2, 0.2)
        window:SetScaleAnimation(0.5, 1, 0.2, 0.2, 'TOPLEFT')
        window:SetStartAnimation(true, true)
    "#,
    )
    .unwrap();
    let window = id(&ui, "window");
    let child = id(&ui, "child");
    let rect = lua::ui(&ui.lua).tree.rect(child);
    lua::ui(&ui.lua).tree.settle();
    let generation = lua::ui(&ui.lua).tree.generation;
    lua::tick(&ui.lua, 100.0);
    let s = lua::ui(&ui.lua).animations.widget_sample(window);
    near(s.alpha.unwrap(), 0.5);
    near(s.scale, 0.75);
    assert_eq!(lua::ui(&ui.lua).tree.rect(child), rect);
    assert_eq!(lua::ui(&ui.lua).tree.generation, generation);
    let quads = aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None);
    assert_eq!(quads.len(), 1);
    assert_eq!(quads[0].pos, [15.0, 15.0, 45.0, 30.0]);
    near(quads[0].color[3], 0.5);
    assert_eq!(aa_ui::input::pick(&ui.lua, 16.0, 16.0), Some(child));
    assert_ne!(aa_ui::input::pick(&ui.lua, 59.0, 39.0), Some(child));
    lua::tick(&ui.lua, 150.0);
    assert_eq!(eval::<i64>(&ui, "alphaEnded + scaleEnded"), 2);
    assert!(!eval::<bool>(&ui, "window:IsNowAnimation()"));
    lua::ui(&ui.lua).tree.settle();
    let content = lua::ui(&ui.lua).tree.content();
    lua::tick(&ui.lua, 1000.0);
    assert_eq!(eval::<i64>(&ui, "alphaEnded + scaleEnded"), 2);
    assert_eq!(lua::ui(&ui.lua).tree.settle(), content);
}

#[test]
fn original_window_animation_helper_fades_and_scales_together() {
    let ui = setup();
    aa_lua::run_alb(&ui.lua, "scriptsbin/x2ui/baselib/window_view.alb").unwrap();
    ui.exec(
        "@window_animation",
        "SetttingUIAnimation(window); window:SetStartAnimation(true,true)",
    )
    .unwrap();
    let window = id(&ui, "window");
    let s = lua::ui(&ui.lua).animations.widget_sample(window);
    near(s.alpha.unwrap(), 0.0);
    near(s.scale, 0.95);
    lua::tick(&ui.lua, 50.0);
    let s = lua::ui(&ui.lua).animations.widget_sample(window);
    near(s.alpha.unwrap(), 0.5);
    near(s.scale, 0.975);
    ui.exec("@stop_alpha", "window:SetStartAnimation(false,true)")
        .unwrap();
    near(eval::<f32>(&ui, "window:GetAlpha()"), 1.0);
    ui.exec("@explicit_alpha", "window:SetAlpha(0.4)").unwrap();
    near(eval::<f32>(&ui, "window:GetAlpha()"), 0.4);
}
