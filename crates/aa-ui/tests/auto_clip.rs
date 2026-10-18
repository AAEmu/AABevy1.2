//! Native cached UTF-32 fitting, including the original chat-tab constructor.

#[path = "../examples/fake/auto_clip.rs"]
mod auto_clip;
#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::{UiRuntime, lua, render};

fn run(ui: &UiRuntime, code: &str) {
    ui.exec("@auto_clip_test", code).unwrap();
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
        button=UIParent:CreateWidget('button','clip','UIParent')
        button:SetExtent(50,30);button:AddAnchor('TOPLEFT',UIParent,100,100)
        button.style:SetFont('ui/font/yd_ygo540.ttf',13);button:Show(true)
    "#,
    );
    ui
}

#[test]
fn enabling_only_preserves_cache_and_full_text_getters() {
    let ui = setup();
    run(
        &ui,
        "button:SetText('Long channel name');button:SetAutoClipChar(true)",
    );
    assert_eq!(count(&ui), 0);
    assert!(render::build_quads(&mut lua::ui(&ui.lua), None).is_empty());
    run(
        &ui,
        r#"
        assert(button:GetText()=='Long channel name')
        assert(button.style:GetTextWidth()>button:GetWidth())
        button.style:GetTextWidth();button.style:GetColor();button.style:SetColor(1,0,0,1)
    "#,
    );
    assert_eq!(count(&ui), 0);
    run(&ui, "button:SetText(button:GetText())");
    assert!(count(&ui) > 0 && count(&ui) < 17);
    let cached = count(&ui);
    run(
        &ui,
        "button:SetAutoClipChar(false);button:SetWidth(200);button:SetAutoClipChar(true)",
    );
    render::build_quads(&mut lua::ui(&ui.lua), None);
    assert_eq!(count(&ui), cached);
    run(&ui, "button:SetAutoResize(false)");
    assert_eq!(count(&ui), "Long channel name".chars().count());
}

#[test]
fn exact_fit_uses_f32_boundary_and_unicode_characters_with_kerning() {
    let ui = setup();
    let text = "Voé友";
    let prefix = "Voé";
    let width = lua::ui(&ui.lua)
        .fonts
        .measure("ui/font/yd_ygo540.ttf", 13., prefix);
    assert!(width > 0.0);
    ui.lua.globals().set("text", text).unwrap();
    ui.lua.globals().set("exact", width).unwrap();
    ui.lua
        .globals()
        .set("below", f32::from_bits(width.to_bits() - 1))
        .unwrap();
    ui.lua
        .globals()
        .set(
            "roundsToExact",
            (width as f64 + f32::from_bits(width.to_bits() - 1) as f64) * 0.5,
        )
        .unwrap();
    run(
        &ui,
        "button:SetAutoClipChar(true);button:SetWidth(exact);button:SetText(text)",
    );
    assert_eq!(count(&ui), prefix.chars().count());
    run(&ui, "button:SetWidth(below)");
    assert_eq!(count(&ui), prefix.chars().count() - 1);
    run(&ui, "button:SetWidth(roundsToExact)");
    assert_eq!(count(&ui), prefix.chars().count());
    assert_eq!(
        ui.lua.load("button:GetText()").eval::<String>().unwrap(),
        text
    );
}

#[test]
fn inset_alone_retains_cache_then_font_extent_and_anchor_callbacks_refresh() {
    let ui = setup();
    run(
        &ui,
        "button:SetAutoClipChar(true);button:SetText('WWWWWWWW')",
    );
    let old = count(&ui);
    run(&ui, "button:SetInset(20,0,20,0)");
    render::build_quads(&mut lua::ui(&ui.lua), None);
    assert_eq!(count(&ui), old); // Native 39280730 just stores four inset floats.
    run(&ui, "button.style:SetFontSize(13)");
    assert!(count(&ui) < old);
    run(
        &ui,
        r#"
        owner=UIParent:CreateWidget('emptywidget','clipOwner','UIParent')
        owner:SetExtent(90,30);owner:AddAnchor('TOPLEFT',UIParent,100,100);owner:Show(true)
        button:RemoveAllAnchors();button:AddAnchor('TOPLEFT',owner,0,0)
        button:AddAnchor('TOPRIGHT',owner,0,0)
    "#,
    );
    render::build_quads(&mut lua::ui(&ui.lua), None);
    let wide = count(&ui);
    run(&ui, "owner:SetWidth(45)");
    render::build_quads(&mut lua::ui(&ui.lua), None);
    assert!(count(&ui) < wide);
    run(&ui, "owner:SetWidth(90)");
    render::build_quads(&mut lua::ui(&ui.lua), None);
    let previous = count(&ui);
    assert!(previous > 0);
    // Resolve a preceding anchor-size change before an inset-only write, just
    // as the native geometry callback runs before the later Lua setter.
    run(&ui, "owner:SetWidth(45);button:SetInset(0,0,0,0)");
    render::build_quads(&mut lua::ui(&ui.lua), None);
    assert_eq!(count(&ui), 0);
    run(&ui, "button.style:SetFontSize(13)");
    let narrow = count(&ui);
    run(
        &ui,
        "owner:SetWidth(90);button:SetText('');button:SetHeight(31)",
    );
    assert!(count(&ui) > narrow);
}

#[test]
fn native_newline_count_quirk_and_nul_stop_do_not_mutate_the_source() {
    let ui = setup();
    let width = lua::ui(&ui.lua)
        .fonts
        .measure("ui/font/yd_ygo540.ttf", 13., "AB");
    ui.lua.globals().set("width", width).unwrap();
    run(
        &ui,
        r"button:SetAutoClipChar(true);button:SetWidth(width);button:SetText('A\nB')",
    );
    assert_eq!(count(&ui), 2); // Actual native oracle counts no newline and keeps its pen.
    assert_eq!(
        ui.lua.load("button:GetText()").eval::<String>().unwrap(),
        "A\nB"
    );
    run(&ui, r"button:SetText('A\000B')");
    assert_eq!(count(&ui), 1);
    assert_eq!(
        ui.lua.load("button:GetText()").eval::<String>().unwrap(),
        "A\0B"
    );
    run(&ui, r"button:SetText('\000B')");
    assert_eq!(count(&ui), 1); // Native empty check tests the first UTF-32 codepoint.
    assert!(render::build_quads(&mut lua::ui(&ui.lua), None).is_empty());
}

#[test]
fn renderer_matches_literal_prefix_and_leaves_copper_icon_and_other_text_unchanged() {
    let ui = setup();
    run(
        &ui,
        "button:SetAutoClipChar(true);button:SetText('Néighbours and friends')",
    );
    let n = count(&ui);
    let original = ui.lua.load("button:GetText()").eval::<String>().unwrap();
    let expected: String = original.chars().take(n).collect();
    let clipped = render::build_quads(&mut lua::ui(&ui.lua), None);
    ui.lua.globals().set("prefix", expected).unwrap();
    run(&ui, "button:SetAutoClipChar(false);button:SetText(prefix)");
    let plain = render::build_quads(&mut lua::ui(&ui.lua), None);
    assert_eq!(clipped.len(), plain.len());
    assert!(!plain.is_empty());
    for (a, b) in clipped.iter().zip(&plain) {
        assert_eq!((a.pos, a.uv, a.color), (b.pos, b.uv, b.color));
    }
    run(&ui, "button:SetText('|m3;')");
    let coins = render::build_quads(&mut lua::ui(&ui.lua), None);
    assert!(
        coins
            .iter()
            .any(|q| q.tex == render::Tex::Image(aa_ui::text::INLINE_ICON_TEXTURE.into()))
    );
    run(
        &ui,
        "button:SetAutoClipChar(true);button:SetAutoClipChar(false)",
    );
    let after = render::build_quads(&mut lua::ui(&ui.lua), None);
    assert_eq!(coins.len(), after.len());
    for (a, b) in coins.iter().zip(&after) {
        assert_eq!((a.pos, a.uv, a.color), (b.pos, b.uv, b.color));
    }
}

#[test]
fn original_chat_constructor_clips_long_names_and_tooltip_receives_full_name() {
    let mut ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    auto_clip::install(&mut ui);
    render::build_quads(&mut lua::ui(&ui.lua), None);
    for i in 1..=3 {
        let tab = id(&ui, &format!("nativeClipTabs[{i}]"));
        let u = lua::ui(&ui.lua);
        let t = u.tree.get(tab).text().unwrap();
        assert!(
            t.auto_clip.enabled
                && t.auto_clip.count > 0
                && t.auto_clip.count < t.text.chars().count()
        );
        assert!(!t.style.ellipsis);
    }
    run(
        &ui,
        r#"
        originalTooltip=SetTooltip
        SetTooltip=function(text,owner) tooltipText=text;return originalTooltip(text,owner) end
        nativeClipTabs[1]:OnEnter()
        assert(tooltipText==nativeClipNames[1])
    "#,
    );
    assert!(
        !lua::ui(&ui.lua)
            .stub_calls
            .contains_key("button:SetAutoClipChar")
    );
}
