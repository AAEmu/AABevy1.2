//! Native persistent scale and per-phase effect clipping, including original tooltip Lua.

#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::{UiRuntime, lua};

fn eval<T: mlua::FromLua>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().expect(code)
}

fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 0.001, "{a} != {b}");
}

fn id(ui: &UiRuntime, name: &str) -> u32 {
    lua::value_id(&ui.lua, &eval(&ui, name)).unwrap()
}

fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920.0, 1080.0).unwrap();
    ui.exec(
        "@setup",
        r#"
        window = UIParent:CreateWidget('window', 'scaledWindow', 'UIParent')
        window:SetExtent(100, 100); window:AddAnchor('TOPLEFT', 'UIParent', 100, 100)
        window:Show(true)
        child = window:CreateChildWidget('button', 'child', 0, true)
        child:SetExtent(40, 20); child:AddAnchor('TOPLEFT', window, 0, 0)
        art = child:CreateColorDrawable(1, 1, 1, 1, 'artwork')
        art:AddAnchor('TOPLEFT', child, 0, 0); art:AddAnchor('BOTTOMRIGHT', child, 0, 0)
    "#,
    )
    .unwrap();
    // Complete the native immediate Show before measuring subsequent layout/cache changes.
    lua::tick(&ui.lua, 0.0);
    ui
}

#[test]
fn persistent_scale_inherits_centers_and_freezes_global_ui_scale_without_layout_churn() {
    let ui = setup();
    ui.exec("@scale", "UIParent:SetUIScale(0.8); window:SetScale(0.8)")
        .unwrap();
    near(eval(&ui, "window:GetEffectiveExtent()"), 80.0);
    near(eval(&ui, "child:GetEffectiveExtent()"), 32.0);
    // A top-level SetScale is absolute; children multiply their parent's native scale.
    ui.exec("@scale", "scales = 0; child:SetHandler('OnScale', function() scales = scales + 1 end); child:SetScale(0.5); window:ApplyUIScale(false)")
        .unwrap();
    near(eval(&ui, "child:GetEffectiveExtent()"), 16.0);
    lua::ui(&ui.lua).tree.settle();
    let generation = lua::ui(&ui.lua).tree.generation;
    let content = lua::ui(&ui.lua).tree.content();
    ui.exec("@same", "child:SetScale(0.5)").unwrap();
    assert_eq!(eval::<u32>(&ui, "scales"), 2);
    lua::tick(&ui.lua, 1000.0);
    assert_eq!(lua::ui(&ui.lua).tree.generation, generation);
    assert_eq!(lua::ui(&ui.lua).tree.settle(), content);
    ui.exec("@global", "UIParent:SetUIScale(1)").unwrap();
    near(eval(&ui, "window:GetEffectiveExtent()"), 80.0);
    near(eval(&ui, "child:GetEffectiveExtent()"), 16.0);
    let quads = aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None);
    assert_eq!(quads.len(), 1);
    near(quads[0].pos[2] - quads[0].pos[0], 16.0);
    let child_id = id(&ui, "child");
    let r = {
        let u = lua::ui(&ui.lua);
        u.animations.widget_rect(&u.tree, child_id)
    };
    assert_eq!(
        aa_ui::input::pick(&ui.lua, r.x + 1.0, r.y + 1.0),
        Some(child_id)
    );
    ui.exec(
        "@resume",
        "window:ApplyUIScale(true); UIParent:SetUIScale(0.75)",
    )
    .unwrap();
    near(eval(&ui, "window:GetEffectiveExtent()"), 75.0);
    // SetScale is ignored while native scale animation owns the channel, then takes over.
    ui.exec(
        "@animate",
        r#"
        window:SetScaleAnimation(0.5, 1, 0.2, 0.2, 'CENTER')
        window:SetStartAnimation(false, true); window:SetScale(2)
    "#,
    )
    .unwrap();
    near(eval(&ui, "window:GetEffectiveExtent()"), 37.5);
    lua::tick(&ui.lua, 200.0);
    ui.exec("@takeover", "window:SetScale(0.6)").unwrap();
    near(eval(&ui, "window:GetEffectiveExtent()"), 60.0);
}

#[test]
fn rotated_effect_clips_to_original_bounds_with_affine_uvs_and_phase_changes() {
    let ui = setup();
    ui.exec(
        "@effect",
        r#"
        child:Show(false)
        effect = window:CreateEffectDrawable('ui/common/hud.dds', 'artwork')
        effect:SetCoords(462, 233, 3, 3); effect:SetExtent(42, 42)
        effect:AddAnchor('CENTER', window, 0, 0)
        effect:SetEffectPriority(1, 'alpha', 1, 1)
        effect:SetEffectInitialColor(1, 1, 1, 1, 0)
        effect:SetEffectFinalColor(1, 1, 1, 1, 1)
        effect:SetEffectScale(1, 2, 2, 2, 2)
        effect:SetEffectRotate(1, 45, 45); effect:SetEffectClip(1, true)
        effect:SetEffectPriority(2, 'alpha', 1, 1)
        effect:SetEffectInitialColor(2, 1, 1, 1, 1)
        effect:SetEffectFinalColor(2, 1, 1, 1, 0)
        effect:SetStartEffect(true)
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 500.0);
    let rect = lua::ui(&ui.lua).tree.rect(id(&ui, "effect"));
    let (tw, th) = lua::ui(&ui.lua).textures.size("ui/common/hud.dds").unwrap();
    let quads = aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None);
    assert!(quads.len() > 1);
    let mut area = 0.0;
    for q in quads {
        let points = q.corners.unwrap();
        let uv = q.uvs.unwrap();
        for i in 0..4 {
            let [x, y] = points[i];
            assert!(x >= rect.x - 0.001 && x <= rect.right() + 0.001);
            assert!(y >= rect.y - 0.001 && y <= rect.bottom() + 0.001);
            let dx = x - rect.x - rect.w / 2.0;
            let dy = y - rect.y - rect.h / 2.0;
            let c = std::f32::consts::FRAC_1_SQRT_2;
            near(
                (uv[i][0] * tw as f32 - 462.0) / 3.0,
                0.5 + c * (dx + dy) / 84.0,
            );
            near(
                (uv[i][1] * th as f32 - 233.0) / 3.0,
                0.5 + c * (dy - dx) / 84.0,
            );
        }
        area += ((points[1][0] - points[0][0]) * (points[2][1] - points[0][1])
            - (points[1][1] - points[0][1]) * (points[2][0] - points[0][0]))
            .abs()
            / 2.0;
    }
    near(area, 42.0 * 42.0);
    lua::tick(&ui.lua, 600.0);
    aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None);
    assert!(lua::ui(&ui.lua).animations.sample(id(&ui, "effect")).clip);
    // Native finishes one phase on this draw and initializes the next on a later draw.
    lua::tick(&ui.lua, 0.0);
    aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None);
    assert!(!lua::ui(&ui.lua).animations.sample(id(&ui, "effect")).clip);
}

#[test]
fn original_tooltip_scale_calls_preserve_pixels_across_global_scale_changes() {
    let mut ui = UiRuntime::new("en_us", 1920.0, 1080.0).unwrap();
    fake::install(&mut ui, "en_us", &fake::default_worlds());
    fake::install_ingame(&mut ui);
    ui.run_stage(aa_ui::Stage::World);
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    ui.exec(
        "@tooltip_scale",
        r#"
        UIParent:SetUIScale(0.8)
        testTip = hotKeyOverrideTooltip.window
        testTip:SetWidth(260)
        section = testTip:CreateChildWidget('emptywidget', 'scaleSection', 0, true)
        section:SetExtent(100, 20)
        section:AddAnchor('TOPLEFT', testTip, 0, 0)
        testTip.currentSections = {section}
        testTip.tooltipItemAndSkill = {ClearLines = function() end}
        testTip:SetInfo({})
        testTip:Show(true)
    "#,
    )
    .unwrap();
    near(eval(&ui, "testTip:GetEffectiveExtent()"), 208.0);
    ui.exec("@global", "UIParent:SetUIScale(1)").unwrap();
    near(eval(&ui, "testTip:GetEffectiveExtent()"), 208.0);
}

#[test]
fn scroll_clip_cuts_rotated_child_art_after_persistent_parent_scaling() {
    let ui = setup();
    ui.exec(
        "@scroll",
        r#"
        window:EnableScroll(true); window:SetScale(0.5)
        child:RemoveAllDrawables()
        effect = child:CreateImageDrawable('ui/common/hud.dds', 'artwork')
        effect:SetCoords(462, 233, 3, 3); effect:SetExtent(160, 160)
        effect:AddAnchor('CENTER', window, 0, 0)
        effect:SetRotation(45)
    "#,
    )
    .unwrap();
    let bounds = {
        let u = lua::ui(&ui.lua);
        u.animations.widget_rect(&u.tree, id(&ui, "window"))
    };
    let quads = aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None);
    assert!(!quads.is_empty());
    for q in quads {
        for [x, y] in q.corners.unwrap() {
            assert!(x >= bounds.x - 0.001 && x <= bounds.right() + 0.001);
            assert!(y >= bounds.y - 0.001 && y <= bounds.bottom() + 0.001);
        }
    }
}
