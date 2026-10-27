//! Original speech-bubble sizing must use the same physical rows as its painter.
#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;
use aa_ui::{UiRuntime, lua, render};
fn eval<T: mlua::FromLua>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().unwrap()
}
fn setup(scale: f32, text: &str) -> UiRuntime {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    ui.load_ui_texts(std::path::Path::new(&fake::game_db()))
        .unwrap();
    fake::install(&mut ui, "en_us", &fake::default_worlds());
    ui.load_base_libs();
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    ui.exec(
        "@bubble_game",
        &format!(
            r#"
 UIParent:SetUIScale({scale});chatLocale={{lineSpace={{chatBubble=0}}}}
 X2Unit.GetUnitId=function() return 'player' end
 X2Unit.GetUnitInfoById=function() return {{faction='friendly',type='npc'}} end
 X2Unit.GetUnitScreenNameTagOffset=function() return 350,300,1 end
 "#
        ),
    )
    .unwrap();
    let errors = aa_lua::run_files(&ui.lua, &["scriptsbin/x2ui/chat/chatbubble.alb".into()]);
    assert!(errors.is_empty(), "{errors:?}");
    ui.lua.globals().set("bubbleText", text).unwrap();
    ui.exec("@original_bubble",r#"bubble=chatBubbleFrame:AddBubble('npc',true,CHAT_SAY,5000);bubble:SetChatMsg('Watcher',bubbleText);bubble:RemoveAllAnchors();bubble:AddAnchor('TOPLEFT',UIParent,100,100)"#).unwrap();
    ui
}
#[test]
fn original_bubble_height_matches_native_physical_font_rows() {
    let ui = setup(0.85, "Old forests conceal distant roads and hidden valleys");
    let height: f32 = eval(&ui, "bubble:GetHeight()");
    let rows: i64 = eval(&ui, "bubble:GetLineCount()");
    let width: f32 = eval(&ui, "bubble:GetLongestLineWidth()");
    eprintln!("BUBBLE height={height} rows={rows} longest={width}");
    assert!(
        (height - 32. / 0.85).abs() < 0.001,
        "original SetChatMsg/AdjustBubbleLayout used {height}, native 2x16/.85={}",
        32. / 0.85
    );
    let _ = render::build_quads(&mut lua::ui(&ui.lua), None);
}

#[test]
fn original_bubble_native_scale_controls() {
    for (scale, height, width) in [
        (1., 36., 178.),
        (0.85, 37.647057, 184. / 0.85),
        (0.7, 34.285713, 145. / 0.7),
        (1.25, 36.8, 272. / 1.25),
    ] {
        let ui = setup(
            scale,
            "Old forests conceal distant roads and hidden valleys",
        );
        let actual_h: f32 = eval(&ui, "bubble:GetHeight()");
        let actual_w: f32 = eval(&ui, "bubble:GetLongestLineWidth()");
        eprintln!("scale={scale} height={actual_h}/{height} width={actual_w}/{width}");
        assert!((actual_h - height).abs() < 0.001);
        assert!((actual_w - width).abs() < 0.001);
    }
}

#[test]
fn retained_native_rich_rows_with_real_font_match_textbox_queries() {
    let ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    ui.exec("@native_rows", "UIParent:SetUIScale(1); box=UIParent:CreateWidget('textbox','nativeRows','UIParent');box:SetAutoResize(false);box:Show(true)").unwrap();
    let mut failures = Vec::new();
    for line in include_str!("fixtures/textbox-native-rows.tsv")
        .lines()
        .filter(|line| !line.starts_with('#'))
    {
        let fields: Vec<_> = line.split('\t').collect();
        let size: f32 = fields[0].parse().unwrap();
        let width: f32 = fields[1].parse().unwrap();
        let text = fields[2].replace("\\r", "\r").replace("\\n", "\n");
        let widths: Vec<f32> = fields[3]
            .split(',')
            .filter(|v| !v.is_empty())
            .map(|v| v.parse().unwrap())
            .collect();
        ui.lua.globals().set("source", text.clone()).unwrap();
        ui.exec("@native_case", &format!("box:SetExtent({width},500);box.style:SetFont('ui/font/yd_ygo540.ttf',{size});box:SetText(source)")).unwrap();
        let rows: usize = eval(&ui, "box:GetLineCount()");
        let actual: f32 = eval(&ui, "box:GetLongestLineWidth()");
        if rows != widths.len() {
            failures.push(format!("{line:?}: rows {rows} != {}", widths.len()));
        }
        let expected = widths.iter().copied().fold(0., f32::max);
        if (actual - expected).abs() >= 0.001 {
            failures.push(format!("{line:?}: width {actual} != {expected}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} mismatches: {failures:#?}",
        failures.len()
    );
}

#[path = "../examples/native_textbox_layout.rs"]
#[allow(dead_code)]
mod original_bubbles;
#[test]
fn complete_original_bubbles_use_actual_locale_line_space_and_native_sizing() {
    for (scale, height, width) in [
        (1., 39., 178.),
        (0.85, 40.647057, 184. / 0.85),
        (0.7, 37.285713, 145. / 0.7),
        (1.25, 39.8, 272. / 1.25),
    ] {
        let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
        ui.load_ui_texts(std::path::Path::new(&fake::game_db()))
            .unwrap();
        original_bubbles::install_at_scale(&mut ui, scale);
        let actual_h: f32 = eval(&ui, "chatBubbleFrame.bubbles.first:GetHeight()");
        let actual_w: f32 = eval(&ui, "chatBubbleFrame.bubbles.first:GetLongestLineWidth()");
        assert!(
            (actual_h - height).abs() < 0.001,
            "scale{scale}: {actual_h}/{height}"
        );
        assert!(
            (actual_w - width).abs() < 0.001,
            "scale{scale}: {actual_w}/{width}"
        );
        aa_ui::lua::tick(&ui.lua, 0.);
        assert!(!render::build_quads(&mut lua::ui(&ui.lua), None).is_empty());
    }
}

fn painted_box(ui: &UiRuntime) -> Vec<aa_ui::render::Quad> {
    render::build_quads(&mut lua::ui(&ui.lua), None)
        .into_iter()
        .filter(|q| matches!(q.tex, render::Tex::Atlas) && q.color == [1., 0., 0., 1.])
        .collect()
}
fn paint_setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    ui.exec(
        "@physical_box",
        r#"
 UIParent:SetUIScale(.85)
 box=UIParent:CreateWidget('textbox','physicalRows','UIParent')
 box:SetAutoResize(false);box:SetExtent(41,300);box:AddAnchor('TOPLEFT',UIParent,100.3,80.25)
 box.style:SetFont('ui/font/yd_ygo540.ttf',15);box.style:SetAlign(ALIGN_TOP_LEFT)
 box.style:SetColor(1,0,0,1);box:SetLineSpace(3);box:SetText('OOOOOO');box:Show(true)
 box:SetHeight(box:GetTextHeight())
 "#,
    )
    .unwrap();
    ui
}
#[test]
fn actual_textbox_paint_uses_native_physical_font_and_per_row_snap() {
    let ui = paint_setup();
    let glyph = {
        let mut u = lua::ui(&ui.lua);
        u.fonts
            .layout(aa_ui::text::DEFAULT_FONT, 13., "O", None, 0., false)
            .glyphs[0]
            .glyph
    };
    for snap in [true, false] {
        ui.exec("@snap", &format!("box.style:SetSnap({snap})"))
            .unwrap();
        let paint = painted_box(&ui);
        assert_eq!(paint.len(), 6);
        let pixel = |v: f32| if snap { (v + 0.5).trunc() } else { v };
        for (i, q) in paint.iter().enumerate() {
            let x = pixel(100.3_f32 * 0.85 + glyph.left as f32 + (i % 3) as f32 * 11.);
            let y = pixel(
                80.25_f32 * 0.85 + 13. + glyph.top as f32 + (i / 3) as f32 * (16. + 3. * 0.85),
            );
            for (a, b) in q
                .pos
                .iter()
                .zip([x, y, x + glyph.w as f32, y + glyph.h as f32])
            {
                assert!((a - b).abs() < 0.001, "{snap}: {q:?} expected {x},{y}");
            }
            assert_eq!(
                q.uv,
                [
                    glyph.ax as f32 / 2048.,
                    glyph.ay as f32 / 2048.,
                    (glyph.ax + glyph.w) as f32 / 2048.,
                    (glyph.ay + glyph.h) as f32 / 2048.
                ]
            );
        }
    }
}
#[test]
fn frozen_textbox_retains_queries_and_physical_glyphs_across_global_scale() {
    let ui = paint_setup();
    ui.exec("@freeze", "box:ApplyUIScale(false)").unwrap();
    let before = painted_box(&ui);
    let height: f32 = eval(&ui, "box:GetTextHeight()");
    let width: f32 = eval(&ui, "box:GetLongestLineWidth()");
    for scale in [1., 1.25, 0.85] {
        ui.exec("@round_trip", &format!("UIParent:SetUIScale({scale})"))
            .unwrap();
        assert_eq!(eval::<f32>(&ui, "box:GetTextHeight()"), height);
        assert_eq!(eval::<f32>(&ui, "box:GetLongestLineWidth()"), width);
        assert_eq!(eval::<usize>(&ui, "box:GetLineCount()"), 2);
        let after = painted_box(&ui);
        assert_eq!(after.len(), before.len());
        for (a, b) in after.iter().zip(&before) {
            assert_eq!(a.uv, b.uv);
            for axis in 0..2 {
                assert!(
                    ((a.pos[axis + 2] - a.pos[axis]) - (b.pos[axis + 2] - b.pos[axis])).abs()
                        < 0.001
                );
                assert!(
                    ((a.pos[axis] - after[0].pos[axis]) - (b.pos[axis] - before[0].pos[axis]))
                        .abs()
                        < 0.001
                );
            }
        }
    }
}
#[test]
fn physical_wrap_allowance_respects_textbox_insets_and_auto_height() {
    let ui = paint_setup();
    for (width, rows) in [(53., 3), (55., 2)] {
        ui.exec("@inset_rows",&format!("box:SetInset(10,4,4,6);box:SetWidth({width});box:SetAutoResize(true);box:SetText('OOOOOO')")).unwrap();
        assert_eq!(eval::<usize>(&ui, "box:GetLineCount()"), rows);
        let expected = rows as f32 * 16. / 0.85 + (rows - 1) as f32 * 3.;
        assert!((eval::<f32>(&ui, "box:GetTextHeight()") - expected).abs() < 0.001);
        assert!((eval::<f32>(&ui, "box:GetHeight()") - expected - 10.).abs() < 0.001);
        assert_eq!(painted_box(&ui).len(), 6);
    }
}
