//! Native physical-face fitting and scale/geometry callback lifecycle.

#[path = "../examples/fake/auto_clip.rs"]
mod auto_clip;
#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::{UiRuntime, input, lua, render};

fn run(ui: &UiRuntime, code: &str) {
    ui.exec("@auto_clip_scale", code).unwrap();
}
fn id(ui: &UiRuntime, name: &str) -> u32 {
    lua::value_id(&ui.lua, &ui.lua.load(name).eval().unwrap()).unwrap()
}
fn count(ui: &UiRuntime) -> usize {
    lua::ui(&ui.lua)
        .tree
        .get(id(ui, "button"))
        .text()
        .unwrap()
        .auto_clip
        .count
}
fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    run(
        &ui,
        r#"
        owner=UIParent:CreateWidget('emptywidget','scaledClipOwner','UIParent')
        owner:SetExtent(200,80);owner:AddAnchor('TOPLEFT',UIParent,100,100);owner:Show(true)
        button=owner:CreateChildWidget('button','clip',0,true)
        button:SetExtent(90,30);button:AddAnchor('TOPLEFT',owner,0,0)
        button.style:SetFont('ui/font/yd_ygo540.ttf',13)
        button:SetAutoClipChar(true);button:SetText('WWWWWWWWWWWWWWWWWWWW')
    "#,
    );
    ui
}

// Use full-string measurement, independently of the cached fitting helper.
fn measured_count(ui: &UiRuntime, font: &str, physical_size: f32, available: f32) -> usize {
    let source = ui.lua.load("button:GetText()").eval::<String>().unwrap();
    let mut u = lua::ui(&ui.lua);
    let mut n = 0;
    for i in 1..=source.chars().count() {
        let prefix: String = source.chars().take(i).collect();
        if u.fonts.measure(font, physical_size, &prefix) > available {
            break;
        }
        n = i;
    }
    n
}

#[test]
fn ordinary_font_uses_native_rounding_effective_width_and_raw_fitting_insets() {
    let ui = setup();
    for (command, scale, size) in [
        ("UIParent:SetUIScale(0.85)", 0.85, 11.),
        ("UIParent:SetUIScale(0.93)", 0.93, 12.),
        ("owner:SetScale(1.5)", 1.5, 19.),
        ("button:SetScale(1.25)", 1.875, 24.),
    ] {
        run(&ui, "button:SetInset(13,0,7,0)");
        run(&ui, command);
        assert_eq!(
            count(&ui),
            measured_count(&ui, "ui/font/yd_ygo540.ttf", size, (90. * scale - 13.) - 7.),
            "{command}"
        );
    }
    run(&ui, "owner:ApplyUIScale(false);UIParent:SetUIScale(0.8)");
    assert_eq!(
        count(&ui),
        measured_count(&ui, "ui/font/yd_ygo540.ttf", 24., (90. * 1.875 - 13.) - 7.)
    );
}

#[test]
fn scaled_unicode_exact_fit_and_one_f32_ulp_exclusion() {
    let ui = setup();
    run(&ui, "owner:SetScale(2);button:SetInset(13,0,7,0)");
    let prefix = "Voé";
    let physical = lua::ui(&ui.lua)
        .fonts
        .measure("ui/font/yd_ygo540.ttf", 26., prefix);
    let local = (physical + 20.) / 2.;
    ui.lua.globals().set("exact", local).unwrap();
    ui.lua
        .globals()
        .set("below", f32::from_bits(local.to_bits() - 1))
        .unwrap();
    ui.lua.globals().set("unicode", "Voé友").unwrap();
    run(&ui, "button:SetWidth(exact);button:SetText(unicode)");
    assert_eq!(count(&ui), prefix.chars().count());
    run(&ui, "button:SetWidth(below)");
    assert_eq!(count(&ui), prefix.chars().count() - 1);
    assert_eq!(
        ui.lua.load("button:GetText()").eval::<String>().unwrap(),
        "Voé友"
    );
}

#[test]
fn identical_extent_and_anchor_callbacks_refresh_but_inset_and_remove_anchors_do_not() {
    let ui = setup();
    for setter in [
        "button:SetWidth(90)",
        "button:SetHeight(30)",
        "button:SetExtent(90,30)",
        "owner:SetExtent(200,80)",
        "button:AddAnchor('TOPLEFT',owner,0,0)",
    ] {
        run(
            &ui,
            "button:SetInset(0,0,0,0);button:SetText(button:GetText())",
        );
        let old = count(&ui);
        run(&ui, "button:SetInset(35,0,35,0)");
        render::build_quads(&mut lua::ui(&ui.lua), None);
        assert_eq!(count(&ui), old);
        run(&ui, setter);
        assert!(count(&ui) < old, "{setter}");
    }
    run(
        &ui,
        "button:SetInset(0,0,0,0);button:SetText(button:GetText())",
    );
    let old = count(&ui);
    run(&ui, "button:SetInset(35,0,35,0);button:RemoveAllAnchors()");
    render::build_quads(&mut lua::ui(&ui.lua), None);
    assert_eq!(count(&ui), old);
    // A non-owner anchor target also propagates an identical extent callback.
    run(
        &ui,
        "reference=UIParent:CreateWidget('emptywidget','clipReference','UIParent');reference:SetExtent(200,80);reference:AddAnchor('TOPLEFT',UIParent,100,100);button:AddAnchor('TOPLEFT',reference,0,0);button:SetInset(0,0,0,0);button:SetText(button:GetText())",
    );
    let old = count(&ui);
    run(&ui, "button:SetInset(35,0,35,0);reference:SetWidth(200)");
    assert!(count(&ui) < old);
}

#[test]
fn hidden_scale_callbacks_same_scale_and_animation_ownership_preserve_native_lifecycle() {
    let ui = setup();
    run(
        &ui,
        "button:Show(false);button:SetInset(35,0,35,0);owner:SetScale(1)",
    );
    assert_eq!(
        count(&ui),
        measured_count(&ui, "ui/font/yd_ygo540.ttf", 13., 20.)
    );
    run(&ui, "button:SetInset(0,0,0,0);UIParent:SetUIScale(0.85)");
    // Explicit root scale applies global scale again when the global option changes.
    assert_eq!(
        count(&ui),
        measured_count(&ui, "ui/font/yd_ygo540.ttf", 11., 76.5)
    );
    run(
        &ui,
        "owner:SetScaleAnimation(1,1.5,0.2,0.2,'CENTER');owner:SetStartAnimation(false,true)",
    );
    let previous = count(&ui);
    run(&ui, "button:SetInset(35,0,35,0);owner:SetScale(2)");
    assert_eq!(count(&ui), previous); // SetScale is ignored while animation owns it.
    lua::tick(&ui.lua, 0.);
    assert!(count(&ui) < previous); // Even an unchanged active scale sample callbacks.
    lua::tick(&ui.lua, 250.);
    run(
        &ui,
        "owner:SetScale(1.5);button:SetText('');owner:SetScale(1)",
    );
    let retained = count(&ui);
    run(&ui, "button:SetText('\000rest');button:SetExtent(100,30)");
    assert_eq!(count(&ui), retained);
}

#[test]
fn drawing_uses_one_physical_face_in_both_flag_states_without_scaling_art_twice() {
    let ui = setup();
    run(
        &ui,
        "UIParent:SetUIScale(0.85);owner:SetScale(1.5);button:SetScale(1.25);art=button:CreateColorDrawable(1,0,0,1,'artwork');art:AddAnchor('TOPLEFT',button,0,0);art:AddAnchor('BOTTOMRIGHT',button,0,0)",
    );
    let clipped = render::build_quads(&mut lua::ui(&ui.lua), None);
    let full = ui.lua.load("button:GetText()").eval::<String>().unwrap();
    let prefix: String = full.chars().take(count(&ui)).collect();
    ui.lua.globals().set("prefix", prefix).unwrap();
    run(&ui, "button:SetAutoClipChar(false);button:SetText(prefix)");
    let plain = render::build_quads(&mut lua::ui(&ui.lua), None);
    assert_eq!(clipped.len(), plain.len());
    assert!(plain.len() > 1);
    for (a, b) in clipped.iter().zip(&plain) {
        assert_eq!((a.pos, a.uv, a.color), (b.pos, b.uv, b.color));
    }
    let art = &plain[0];
    // Artwork keeps the existing global-pixel snapping before widget transforms.
    let transform = 1.25 * (1.5 / 0.85);
    assert!((art.pos[2] - art.pos[0] - 77. * transform).abs() < 0.001);
    assert!((art.pos[3] - art.pos[1] - 26. * transform).abs() < 0.001);
    // An unscaled literal face at size24 has identical atlas glyphs, rather than
    // stretching a size11 glyph through the ancestor transforms.
    let glyph = plain[1].uv;
    run(
        &ui,
        "UIParent:SetUIScale(1);owner:SetScale(1);button:SetScale(1);button.style:SetFontSize(24)",
    );
    let literal = render::build_quads(&mut lua::ui(&ui.lua), None);
    assert_eq!(glyph, literal[1].uv);
}

#[test]
fn full_width_and_auto_resize_use_full_physical_text_in_local_units() {
    let ui = setup();
    run(&ui, "owner:SetScale(1.5);button:SetInset(3,0,7,0)");
    let full = ui.lua.load("button:GetText()").eval::<String>().unwrap();
    let physical = lua::ui(&ui.lua)
        .fonts
        .measure("ui/font/yd_ygo540.ttf", 19., &full);
    let width = ui
        .lua
        .load("button.style:GetTextWidth()")
        .eval::<f32>()
        .unwrap();
    assert_eq!(width, (physical as f64 / 1.5) as f32);
    assert!(count(&ui) < full.chars().count());
    run(&ui, "button:SetAutoResize(true)");
    let resized = ui.lua.load("button:GetWidth()").eval::<f32>().unwrap();
    assert_eq!(resized, (physical as f64 / 1.5 + 3. + 7.) as f32);
    assert_eq!(
        ui.lua.load("button:GetText()").eval::<String>().unwrap(),
        full
    );
}

#[test]
fn image_font_fitting_keeps_base_size_and_existing_bitmap_drawing_path() {
    let ui = setup();
    run(
        &ui,
        "button.style:SetFont('actionBar',13);button:SetWidth(40);button:SetText('12345678901234567890');owner:SetScale(1.5);button:SetInset(2,0,2,0);button:SetWidth(40)",
    );
    assert_eq!(count(&ui), measured_count(&ui, "actionBar", 13., 56.));
    let full = ui.lua.load("button:GetText()").eval::<String>().unwrap();
    let prefix: String = full.chars().take(count(&ui)).collect();
    let clipped = render::build_quads(&mut lua::ui(&ui.lua), None);
    assert!(!clipped.is_empty());
    ui.lua.globals().set("prefix", prefix).unwrap();
    run(&ui, "button:SetAutoClipChar(false);button:SetText(prefix)");
    let literal = render::build_quads(&mut lua::ui(&ui.lua), None);
    for (a, b) in clipped.iter().zip(&literal) {
        assert_eq!((a.pos, a.uv), (b.pos, b.uv));
    }
    assert_eq!(clipped.len(), literal.len());
}

#[test]
fn original_chat_default_and_option_scale_keep_full_hover_text_and_physical_clicks() {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    auto_clip::install(&mut ui);
    assert_eq!(lua::ui(&ui.lua).ui_scale, 0.85);
    run(
        &ui,
        "button=nativeClipTabs[1]; originalTooltip=SetTooltip;SetTooltip=function(text,owner) tooltipText=text;return originalTooltip(text,owner) end",
    );
    let mut mouse = Default::default();
    for (option, scale, size) in [(80, 0.85, 11.), (90, 0.93, 12.)] {
        run(
            &ui,
            &format!("UIParent:SetUIScale(GetUIScaleValueByOptionWindowValue({option}),true)"),
        );
        render::build_quads(&mut lua::ui(&ui.lua), None);
        assert_eq!(lua::ui(&ui.lua).ui_scale, scale);
        let (button, r, width, font, left, right) = {
            let u = lua::ui(&ui.lua);
            let b = id(&ui, "button");
            let t = u.tree.get(b).text().unwrap();
            (
                b,
                u.animations.widget_rect(&u.tree, b),
                u.tree.rect(b).w,
                t.style.font.clone(),
                t.inset[0],
                t.inset[2],
            )
        };
        assert_eq!(
            count(&ui),
            measured_count(&ui, &font, size, (width * scale - left) - right)
        );
        let p = (r.x + r.w * 0.5, r.y + r.h * 0.5);
        input::update(&ui.lua, &mut mouse, None, false, false, false, 0.);
        assert_eq!(input::pick(&ui.lua, p.0, p.1), Some(button));
        input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.);
        input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.);
        run(
            &ui,
            "assert(tooltipText==nativeClipNames[1]);assert(button:GetText()==nativeClipNames[1])",
        );
        assert!(
            !lua::ui(&ui.lua)
                .tree
                .get(button)
                .text()
                .unwrap()
                .style
                .ellipsis
        );
    }
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
}
