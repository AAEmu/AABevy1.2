//! Original tooltip sections must reserve their icon columns in actual emitted paint.
use aa_ui::{UiRuntime, lua, render};

#[path = "../examples/native_tooltip_columns.rs"]
#[allow(dead_code)]
mod original_windows;

const FILE: &str = "scriptsbin/x2ui/baselib/tooltip/tooltip.alb";
const DESCRIPTION: &str = "Old forests conceal distant roads and hidden valleys. Follow the river toward the eastern watchtower and return before nightfall.";

fn run(ui: &UiRuntime, code: &str) {
    ui.exec("@tooltip_columns", code).unwrap();
}

fn eval<T: mlua::FromLua>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().unwrap()
}

// Execute unchanged original instruction streams with controlled enclosing upvalues.
fn original(ui: &UiRuntime, path: &[usize]) -> mlua::Function {
    use aa_lua::chunk::{self, Chunk, Format};
    let bytes = aa_pak::Pak::global().read(FILE).unwrap();
    let parsed = chunk::parse(&bytes).unwrap();
    let mut p = &parsed.main;
    for &i in path {
        p = &p.protos[i];
    }
    let bytes = chunk::write(
        &Chunk {
            format: Format::HOST,
            main: p.clone(),
        },
        Format::HOST,
    )
    .unwrap();
    ui.lua.load(bytes).into_function().unwrap()
}

fn setup(scale: f32) -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    run(
        &ui,
        r#"
        locale={tooltip={hour='hour',minute='minute',second='second'}}
        FONT_COLOR_HEX={SINERGY='|cFFFF0000'}
        TEXTURE_COLOR={SINERGY_BUFF={1,1,1},SINERGY_DEBUFF={1,1,1}}
        tip=UIParent:CreateWidget('gametooltip','columnTip','UIParent')
        tip:SetExtent(250,20);tip:SetInset(10,10,10,10);tip:SetAutoWordwrap(true)
        tip:AddAnchor('TOPLEFT',UIParent,100,100);tip:Show(true)
        section=UIParent:CreateWidget('emptywidget','synergySection','UIParent')
        section.index=1;section.anchorY=0
        section.conditionIcon=section:CreateIconImageDrawable('ui/hud.dds','overlay')
        section.resultIcon=section:CreateIconImageDrawable('ui/hud.dds','overlay')
        section.conditionIcon:SetExtent(24,24);section.resultIcon:SetExtent(24,24)
        for _,icon in ipairs({section.conditionIcon,section.resultIcon}) do
            icon.bg=section:CreateImageDrawable('ui/hud.dds','overlay')
            icon.deco=section:CreateImageDrawable('ui/hud.dds','overlay')
        end
        section.arrow=section:CreateImageDrawable('ui/hud.dds','overlay')
    "#,
    );
    for (name, path) in [
        ("FormatDurationTimeInDesc", vec![19]),
        ("originalShowSynergy", vec![64]),
        ("originalSettingSynergy", vec![63]),
        ("originalSynergy", vec![67, 0]),
    ] {
        ui.lua.globals().set(name, original(&ui, &path)).unwrap();
    }
    run(
        &ui,
        r#"
        debug.setupvalue(originalSynergy,1,8)
        debug.setupvalue(originalSynergy,2,5)
        debug.setupvalue(originalSynergy,3,section)
        debug.setupvalue(originalSynergy,4,originalShowSynergy)
        debug.setupvalue(originalSynergy,5,originalSettingSynergy)
        section.SetInfo=originalSynergy
    "#,
    );
    ui.lua.globals().set("description", DESCRIPTION).unwrap();
    run(&ui, &format!("UIParent:SetUIScale({scale})"));
    ui
}

fn synergy(ui: &UiRuntime, result_icon: bool) {
    run(
        ui,
        &format!(
            "tip:ClearLines();section:SetInfo(tip,{{synergyIconInfo={{{{desc=description,resulticon={}}}}}}})",
            if result_icon { "'ui/hud.dds'" } else { "nil" }
        ),
    );
}

fn painted_origin(ui: &UiRuntime, text: &str, color: [f32; 4], size: f32, ratio: f32) -> f32 {
    let mut u = lua::ui(&ui.lua);
    let scale = u.ui_scale;
    let physical_size = (size * scale).trunc();
    let layout = u.fonts.layout(
        aa_ui::text::DEFAULT_FONT,
        physical_size,
        text,
        Some((230. * ratio * scale - 1.).max(1.)),
        0.,
        false,
    );
    let g = layout.glyphs.first().unwrap();
    let quads = render::build_quads(&mut u, None);
    let q = quads
        .iter()
        .find(|q| q.tex == render::Tex::Atlas && q.color == color)
        .unwrap();
    q.pos[0] - g.x - g.glyph.left as f32
}

#[test]
fn original_synergy_reserves_its_icon_column_in_emitted_glyph_paint() {
    for scale in [1., 0.85, 1.5] {
        let ui = setup(scale);
        for (result_icon, ratio) in [(true, 0.68), (false, 0.88)] {
            synergy(&ui, result_icon);
            let id = lua::value_id(&ui.lua, &eval(&ui, "tip")).unwrap();
            let mut u = lua::ui(&ui.lua);
            let r = u.tree.rect(id);
            let text = u.tree.widget(id).unwrap().text.as_ref().unwrap().clone();
            let layout = u.fonts.layout(
                &text.style.font,
                (text.style.size * scale).trunc(),
                DESCRIPTION,
                None,
                0.,
                false,
            );
            let glyph = layout.glyphs.first().unwrap();
            let quads = render::build_quads(&mut u, None);
            let painted = quads
                .iter()
                .find(|q| q.tex == render::Tex::Atlas && q.color == [1., 0., 0., 1.])
                .unwrap();
            let origin = painted.pos[0] - glyph.glyph.left as f32 - glyph.x;
            // Native Draw392989d1..39298a5e: right - innerWidth*ratio.
            let expected = ((r.x + 10. + (r.w - 20.) * (1. - ratio)) * scale).round();
            assert_eq!(origin, expected, "scale={scale}, original ratio={ratio}");
        }
    }
}

#[test]
fn original_synergy_ratio_changes_wrapped_rows_section_height_and_clear_lifetime() {
    let ui = setup(1.);
    synergy(&ui, true);
    let narrow_rows = eval::<i64>(&ui, "tip:GetLineCount()");
    let narrow_section = eval::<f32>(&ui, "section:GetHeight()");
    synergy(&ui, false);
    let wide_rows = eval::<i64>(&ui, "tip:GetLineCount()");
    let wide_section = eval::<f32>(&ui, "section:GetHeight()");
    assert!(
        narrow_rows > wide_rows,
        "original .68/.88 ratios both emitted {narrow_rows}/{wide_rows} rows"
    );
    assert!(narrow_section > wide_section);
    run(
        &ui,
        "tip:ClearLines();assert(tip:GetLastLine()==-1 and tip:GetLineCount()==0)",
    );
    let mut u = lua::ui(&ui.lua);
    assert!(
        !render::build_quads(&mut u, None)
            .iter()
            .any(|q| q.color == [1., 0., 0., 1.])
    );
}

#[test]
fn original_sell_price_money_has_its_own_right_column_and_repair_left_alignment() {
    let ui = setup(1.);
    ui.lua
        .globals()
        .set("originalSell", original(&ui, &[75]))
        .unwrap();
    run(
        &ui,
        r#"
        locale.repair={repairCost='|cFF0000FFRepair Cost'}
        debug.setupvalue(originalSell,1,{sellPrice='|cFF00FF00Sell Price'})
        debug.setupvalue(originalSell,2,5)
        X2Item={IsInRepairMode=function() return true end}
        section.SetSellInfo=originalSell
        section:SetSellInfo(tip,{sellable=true,slotType='backpack',refund=2470005,isStackable=true,stack=2,repairable=true,repairCost=125})
        assert(tip:GetLastLine()==1)
    "#,
    );
    // Original main/75 supplies .35/.65 to sell label/money and .4/.65 to repair.
    // The repair money is ALIGN_LEFT in the RIGHT column, so the first digit
    // starts at 340-230*.65=190.5, rounded by the actual painter to191.
    let money = aa_ui::text::expand_codes("|m125;");
    let mut u = lua::ui(&ui.lua);
    let layout = u
        .fonts
        .layout(aa_ui::text::DEFAULT_FONT, 13., &money, None, 0., false);
    let g = layout.glyphs.first().unwrap();
    let quads = render::build_quads(&mut u, None);
    let white: Vec<_> = quads
        .iter()
        .filter(|q| q.tex == render::Tex::Atlas && q.color == [1., 1., 1., 1.])
        .collect();
    // Last line is repair money; locate its first glyph by its distinct row.
    let last_y = white
        .iter()
        .map(|q| q.pos[1])
        .fold(f32::NEG_INFINITY, f32::max);
    let first = white.iter().find(|q| q.pos[1] == last_y).unwrap();
    assert_eq!(first.pos[0] - g.x - g.glyph.left as f32, 191.);
    drop(u);
    assert_eq!(
        painted_origin(&ui, "Sell Price", [0., 1., 0., 1.], 13., 0.35),
        110.
    );
    run(
        &ui,
        "tip:ClearLines();section:SetSellInfo(tip,{sellable=true,slotType='backpack',refund=1});assert(tip:GetLastLine()==0)",
    );
}

#[test]
fn original_quest_title_wraps_in_its_reserved_extent_and_status_stays_at_right_edge() {
    let ui = setup(1.);
    ui.lua
        .globals()
        .set("originalQuest", original(&ui, &[50]))
        .unwrap();
    run(
        &ui,
        r#"
        FONT_COLOR_HEX.DEFAULT='|cFFFFFFFF';FONT_COLOR_HEX.SOFT_BROWN='|cFF00FF00';FONT_COLOR_HEX.SKYBLUE='|cFF0000FF'
        FONT_PATH={DEFAULT=''};FONT_SIZE={LARGE=18,MIDDLE=13}
        locale.questContext={doing='Active'}
        title='Old forests conceal distant roads and hidden valleys'
        X2Quest={GetQuestContextMainTitle=function() return title end,
            IsProgressQuestInJournal=function() return true end,IsReadyQuestInJournal=function() return false end,
            GetLinkedQuestSummary=function() end,IsSelectiveQuest=function() return false end,
            IsScoreQuest=function() return false end,IsGroupQuest=function() return false end,
            NowIsAggroComponent=function() return false end,GetLinkedQuestObjectives=function() end}
        debug.setupvalue(originalQuest,1,5);section.SetQuestInfo=originalQuest
        section:SetQuestInfo(tip,123)
    "#,
    );
    let title = eval::<String>(&ui, "title");
    let mut u = lua::ui(&ui.lua);
    let expected = u.fonts.layout(
        aa_ui::text::DEFAULT_FONT,
        18.,
        &title,
        Some(230. * 0.85 - 1.),
        0.,
        false,
    );
    let expected_rows = expected.line_widths.len();
    drop(u);
    assert_eq!(eval::<i64>(&ui, "tip:GetLineCount()"), expected_rows as i64);
    let actual = painted_origin(&ui, "Active", [0., 0., 1., 1.], 13., 1.);
    let width = lua::ui(&ui.lua)
        .fonts
        .measure(aa_ui::text::DEFAULT_FONT, 13., "Active");
    assert_eq!(actual, (340. - width).round());
    let (only, include): (f32, f32) = ui
        .lua
        .load("return tip:GetHeightToLastLine()")
        .eval()
        .unwrap();
    assert_eq!(include - only, 10.);
    assert_eq!(eval::<f32>(&ui, "section:GetHeight()"), include - 10. + 5.);
}

#[test]
fn native_side_replacement_width_refit_and_clear_do_not_reuse_old_ratio() {
    let ui = setup(1.);
    run(
        &ui,
        r#"
        a=tip:AddLine('|cFFFF0000O','','13','right',ALIGN_LEFT,0,.5)
        assert(a==nil and tip:GetLastLine()==-1)
        a=tip:AddLine('|cFFFF0000O','',13,'right',ALIGN_LEFT,3.9,.68)
        tip:AddAnotherSideLine(a,'|cFF00FF00O','',13,ALIGN_LEFT,4.9,.35)
    "#,
    );
    assert_eq!(painted_origin(&ui, "O", [1., 0., 0., 1.], 13., 0.68), 187.);
    assert_eq!(painted_origin(&ui, "O", [0., 1., 0., 1.], 13., 0.35), 114.);
    // Once left is occupied, the next additional side replaces the right side.
    run(
        &ui,
        "tip:AddAnotherSideLine(a,'|cFF0000FFO','',13,ALIGN_LEFT,0,.5)",
    );
    assert_eq!(painted_origin(&ui, "O", [0., 0., 1., 1.], 13., 0.5), 225.);
    run(&ui, "tip:SetWidth(350)");
    assert_eq!(painted_origin(&ui, "O", [0., 0., 1., 1.], 13., 0.5), 275.);
    run(
        &ui,
        "tip:ClearLines();tip:SetWidth(250);tip:AddLine('|cFFFF0000O','',13,'RIGHT',ALIGN_LEFT,0,.68)",
    );
    assert_eq!(painted_origin(&ui, "O", [1., 0., 0., 1.], 13., 0.68), 110.);
    run(
        &ui,
        "tip:ClearLines();tip:AddLine('|cFFFF0000O','',13,'right',ALIGN_LEFT,0)",
    );
    assert_eq!(painted_origin(&ui, "O", [1., 0., 0., 1.], 13., 1.), 110.);
}

#[test]
fn typed_native_arguments_preserve_raw_ratio_and_invalid_optional_defaults() {
    let ui = setup(1.);
    for (arg, ratio) in [
        ("nil", 1.),
        ("true", 1.),
        ("'0.35'", 1.),
        ("{}", 1.),
        ("0", 0.),
        ("-0.25", -0.25),
        ("1.5", 1.5),
    ] {
        run(
            &ui,
            &format!(
                "tip:ClearLines();tip:AddLine('|cFFFF0000O','',13.9,'right',ALIGN_LEFT,3.9,{arg})"
            ),
        );
        let x = painted_origin(&ui, "O", [1., 0., 0., 1.], 13., ratio);
        assert_eq!(x, (340. - 230. * ratio + 3.).round(), "{arg}");
    }
    run(
        &ui,
        r#"
        tip:ClearLines()
        for _,args in ipairs({{false,'',13,'left',3,0},{'O',true,13,'left',3,0},
            {'O','','13','left',3,0},{'O','',13,false,3,0},
            {'O','',13,'left','3',0},{'O','',13,'left',3,'0'}}) do
            assert(select('#',tip:AddLine(unpack(args)))==0 and tip:GetLastLine()==-1)
        end
        a=tip:AddLine('|cFFFF0000O','',13,'right',ALIGN_RIGHT,99,.68)
        tip:AddAnotherSideLine('0','bad','',13,ALIGN_LEFT,0)
        assert(tip:GetLineCount()==1)
    "#,
    );
    let width = lua::ui(&ui.lua)
        .fonts
        .measure(aa_ui::text::DEFAULT_FONT, 13., "O");
    assert_eq!(
        painted_origin(&ui, "O", [1., 0., 0., 1.], 13., 0.68),
        (340. - width).round()
    );
}

#[test]
fn native_one_physical_pixel_wrap_allowance_matches_measured_and_painted_rows() {
    for scale in [1., 0.85, 1.5] {
        let ui = setup(scale);
        let physical_size = (13_f32 * scale).trunc();
        let mut u = lua::ui(&ui.lua);
        let two = u
            .fonts
            .measure(aa_ui::text::DEFAULT_FONT, physical_size, "OO");
        let line_height = u.fonts.metrics(aa_ui::text::DEFAULT_FONT, physical_size).1;
        drop(u);
        // Native AddLine takes the ratio extent, then rich-text construction
        // subtracts ONE physical pixel. The old five-unit allowance fits only
        // one O; the native limit fits two, yielding two actual painted rows.
        let width = 20. + (two + 2.) / (0.68 * scale);
        run(
            &ui,
            &format!(
                "tip:SetWidth({width});tip:AddLine('|cFFFF0000OOOO','',13,'right',ALIGN_LEFT,0,.68)"
            ),
        );
        assert_eq!(eval::<i64>(&ui, "tip:GetLineCount()"), 2, "scale={scale}");
        let height = eval::<f32>(&ui, "local _,h=tip:GetHeightToLastLine();return h");
        assert!((height - (10. + 2. * line_height / scale)).abs() < 0.001);
        let mut u = lua::ui(&ui.lua);
        let mut rows: Vec<_> = render::build_quads(&mut u, None)
            .into_iter()
            .filter(|q| q.tex == render::Tex::Atlas && q.color == [1., 0., 0., 1.])
            .map(|q| q.pos[1])
            .collect();
        rows.sort_by(f32::total_cmp);
        rows.dedup();
        assert_eq!(rows.len(), 2, "scale={scale}");
        assert_eq!(rows[1] - rows[0], line_height);
    }
}

#[test]
fn formatting_only_opposite_side_keeps_right_text_and_replaces_the_empty_left_row() {
    let ui = setup(1.);
    run(
        &ui,
        r#"
        a=tip:AddLine('|cFFFF0000R','',13,'right',ALIGN_LEFT,0,.65)
        tip:AddAnotherSideLine(a,'|cFF00FF00','',13,ALIGN_LEFT,0,.35)
        tip:AddAnotherSideLine(a,'|cFF0000FFS','',13,ALIGN_LEFT,0,.35)
    "#,
    );
    let mut u = lua::ui(&ui.lua);
    let quads = render::build_quads(&mut u, None);
    assert!(
        quads
            .iter()
            .any(|q| q.tex == render::Tex::Atlas && q.color == [1., 0., 0., 1.]),
        "native right R was displaced by a formatting-only left side"
    );
    drop(u);
    assert_eq!(painted_origin(&ui, "S", [0., 0., 1., 1.], 13., 0.35), 110.);
    assert_eq!(painted_origin(&ui, "R", [1., 0., 0., 1.], 13., 0.65), 191.);
}

#[test]
fn full_original_constructor_showtooltip_extent_and_current_font_paint_bounds_at_both_scales() {
    for scale in [1., 0.85] {
        let mut ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
        ui.load_ui_texts(std::path::Path::new(&std::env::var("AA_DB").unwrap_or_else(|_| "compact.sqlite3".into())))
        .unwrap();
        original_windows::install_at_scale(&mut ui, scale);
        run(
            &ui,
            "assert(FONT_SIZE.LARGE==15 and FONT_SIZE.MIDDLE==13);assert(locale.questContext.doing=='In-Progress')",
        );
        let mut u = lua::ui(&ui.lua);
        let quads = render::build_quads(&mut u, None);
        for index in [5, 6, 1, 8] {
            let value = eval(&ui, &format!("tooltip.tooltipWindows[{index}]"));
            let id = lua::value_id(&ui.lua, &value).unwrap();
            let frame = u.animations.widget_rect(&u.tree, id);
            let left = frame.x * scale;
            let right = frame.right() * scale;
            let top = frame.y * scale;
            let bottom = frame.bottom() * scale;
            // Native frame sizing is asserted exactly below. Current FontLib
            // baseline/cap-height parity is separately held: check ink bounds
            // against that painter's measured descent, without adding frame
            // padding or changing the original ShowTooltip/paint origin.
            let inherited = frame.w / u.tree.rect(id).w;
            let mut descent = 0_f32;
            for size in [13_f32, 15.] {
                let (ascent, height) = u
                    .fonts
                    .metrics(aa_ui::text::DEFAULT_FONT, (size * scale).trunc());
                descent = descent.max((height - ascent) * inherited);
            }
            let text: Vec<_> = quads
                .iter()
                .filter(|q| {
                    q.tex == render::Tex::Atlas
                        && q.pos[0] >= left
                        && q.pos[0] < right
                        && q.pos[1] >= top
                })
                .collect();
            assert!(
                !text.is_empty(),
                "original window{index}, scale{scale} has no text paint"
            );
            let painted_bottom = text
                .iter()
                .map(|q| q.pos[3])
                .fold(f32::NEG_INFINITY, f32::max);
            assert!(
                painted_bottom <= bottom + descent,
                "original window{index}, scale{scale}: text bottom{painted_bottom} exceeds frame{bottom} + measured descent{descent}"
            );
        }
        drop(u);
        run(
            &ui,
            r#"
            for _,i in ipairs({5,6,1,8}) do
                local w=tooltip.tooltipWindows[i]
                assert(w.needShow==true)
                local sections=w.currentSections
                local _,base=w:GetOffset()
                local _,first=sections[1]:GetOffset()
                local _,last=sections[#sections]:GetOffset()
                local _,height=sections[#sections]:GetExtent()
                assert(math.abs(w:GetHeight()-((last+height-base)+(first-base)))<.001)
            end
        "#,
        );
    }
}

#[test]
fn native_height_getter_keeps_top_inset_empty_populated_spaced_and_cleared() {
    for scale in [1., 0.85, 1.5] {
        let ui = setup(scale);
        run(&ui, "tip:SetInset(7,9,13,11)");
        let heights = || {
            ui.lua
                .load("return tip:GetHeightToLastLine()")
                .eval::<(f32, f32)>()
                .unwrap()
        };
        assert_eq!(heights(), (9., 9.));
        run(
            &ui,
            "a=tip:AddLine('O','',13,'left',ALIGN_LEFT,0);tip:AttachUpperSpaceLine(a,3);tip:AttachLowerSpaceLine(a,5)",
        );
        let physical = eval::<f32>(&ui, "tip:GetHeight()") * scale;
        let (only, include) = heights();
        // Original39296740: height/nativeScale-bottomInset, then last lower-space.
        assert!((include - (physical / scale - 11.)).abs() < 0.001);
        assert_eq!(include - only, 5.);
        // The native getter reads the widget's current height; it does not
        // recompute the text layout as an incidental side effect.
        run(&ui, "tip:SetHeight(tip:GetHeight()+17)");
        let (only, include) = heights();
        let resolved = eval::<f32>(&ui, "tip:GetHeight()");
        assert_eq!(include, resolved - 11.);
        assert_eq!(only, include - 5.);
        run(
            &ui,
            "tip:ClearLines();assert(tip:GetLineCount()==0 and tip:GetLastLine()==-1)",
        );
        assert_eq!(heights(), (9., 9.));
    }
}

#[test]
fn explicit_blank_rows_and_transparent_glyphs_remain_populated_native_sides() {
    for content in ["'\\n'", "'\\r\\n'", "'\\n\\n'", "'|c00000000.'"] {
        let ui = setup(1.);
        run(
            &ui,
            &format!(
                r#"
            a=tip:AddLine('|cFFFF0000R','',13,'right',ALIGN_LEFT,0,.65)
            tip:AddAnotherSideLine(a,{content},'',13,ALIGN_LEFT,0,.35)
            tip:AddAnotherSideLine(a,'|cFF0000FFS','',13,ALIGN_LEFT,0,.5)
        "#
            ),
        );
        assert_eq!(painted_origin(&ui, "S", [0., 0., 1., 1.], 13., 0.5), 225.);
        let mut u = lua::ui(&ui.lua);
        assert!(
            !render::build_quads(&mut u, None)
                .iter()
                .any(|q| q.color == [1., 0., 0., 1.]),
            "{content}"
        );
    }
}

#[test]
fn current_inline_icon_and_image_font_rows_remain_populated_without_outline_glyphs() {
    // Admission must follow the current rich-layout containers, including the
    // existing inline-icon and image-font paths. This is renderer-model coverage,
    // not a native font-manager or empty-value-markup equivalence claim.
    for (text, font, size) in [("|b;", "", 13), ("1", "characterInfo", 37)] {
        let ui = setup(1.);
        ui.lua.globals().set("sideText", text).unwrap();
        ui.lua.globals().set("sideFont", font).unwrap();
        ui.lua.globals().set("sideSize", size).unwrap();
        run(
            &ui,
            r#"
            a=tip:AddLine('|cFFFF0000R','',13,'right',ALIGN_LEFT,0,.65)
            tip:AddAnotherSideLine(a,sideText,sideFont,sideSize,ALIGN_LEFT,0,.35)
            tip:AddAnotherSideLine(a,'|cFF0000FFS','',13,ALIGN_LEFT,0,.5)
        "#,
        );
        assert_eq!(painted_origin(&ui, "S", [0., 0., 1., 1.], 13., 0.5), 225.);
        let mut u = lua::ui(&ui.lua);
        let paint = render::build_quads(&mut u, None);
        let texture = if font.is_empty() {
            aa_ui::text::INLINE_ICON_TEXTURE.to_string()
        } else {
            u.fonts
                .layout(font, size as f32, text, None, 0., false)
                .image_texture
                .unwrap()
        };
        let resolved = u.textures.resolve(&texture).unwrap();
        assert!(!paint.iter().any(|q| q.color == [1., 0., 0., 1.]));
        assert!(
            paint
                .iter()
                .any(|q| matches!(&q.tex,render::Tex::Image(path) if path==&resolved)),
            "{font}:{text}"
        );
    }
}

#[derive(Debug, PartialEq)]
struct ScalePaint {
    height: f32,
    to_last: (f32, f32),
    rows: i64,
    /// Actual top coordinates of equal O glyphs, not layout parameters.
    painted_rows: Vec<f32>,
}

fn scale_paint(ui: &UiRuntime) -> ScalePaint {
    let height = eval(ui, "tip:GetHeight()");
    let to_last = ui
        .lua
        .load("return tip:GetHeightToLastLine()")
        .eval()
        .unwrap();
    let rows = eval(ui, "tip:GetLineCount()");
    let mut u = lua::ui(&ui.lua);
    let mut painted_rows: Vec<_> = render::build_quads(&mut u, None)
        .into_iter()
        .filter(|q| q.tex == render::Tex::Atlas && q.color == [1., 0., 0., 1.])
        .map(|q| q.pos[1])
        .collect();
    painted_rows.sort_by(f32::total_cmp);
    painted_rows.dedup();
    ScalePaint {
        height,
        to_last,
        rows,
        painted_rows,
    }
}

fn populated_scale_transition(from: f32, to: f32) {
    for class in ["gametooltip", "unitframetooltip"] {
        let ui = setup(from);
        ui.lua.globals().set("scaleClass", class).unwrap();
        run(
            &ui,
            r#"
            tip:Show(false)
            tip=UIParent:CreateWidget(scaleClass,'liveScaleTip','UIParent')
            tip:SetWidth(250);tip:SetInset(10,9,10,11);tip:SetAutoWordwrap(true)
            tip:AddAnchor('TOPLEFT',UIParent,100,100);tip:Show(true)
            a=tip:AddLine('|cFFFF0000'..string.rep('O',100),'',15,'right',ALIGN_LEFT,0,.68)
            tip:AttachUpperSpaceLine(a,3);tip:AttachLowerSpaceLine(a,5)
        "#,
        );
        let initial = scale_paint(&ui);
        run(&ui, &format!("UIParent:SetUIScale({to})"));
        let changed = scale_paint(&ui);
        // SAME width is a diagnostic control: scale propagation must have
        // already done the native reflow, without requiring a script resize.
        run(&ui, "tip:SetWidth(tip:GetWidth())");
        let same_width = scale_paint(&ui);
        eprintln!(
            "{class} {from}->{to}: initial={initial:?}, changed={changed:?}, sameWidth={same_width:?}"
        );
        assert_ne!(
            initial.rows, changed.rows,
            "fixture must cross a wrap boundary"
        );
        assert_eq!(changed.rows as usize, changed.painted_rows.len());
        assert_eq!(changed.to_last.1, changed.height - 11.);
        assert_eq!(changed.to_last.0, changed.to_last.1 - 5.);
        assert_eq!(
            changed, same_width,
            "{class} {from}->{to} needed an unchanged SetWidth to refit"
        );
    }
}

#[test]
fn populated_15px_wrapped_tooltips_refit_on_live_scale_85_to_1() {
    populated_scale_transition(0.85, 1.);
}

#[test]
fn populated_15px_wrapped_tooltips_refit_on_live_scale_1_to_85() {
    populated_scale_transition(1., 0.85);
}

#[test]
fn original_quest_default_15px_title_remains_coherent_through_live_scale_round_trip() {
    let ui = setup(0.85);
    ui.lua
        .globals()
        .set("originalQuest", original(&ui, &[50]))
        .unwrap();
    run(
        &ui,
        r#"
        FONT_COLOR_HEX.DEFAULT='|cFFFF0000';FONT_COLOR_HEX.SOFT_BROWN='|cFFFF0000';FONT_COLOR_HEX.SKYBLUE='|cFF0000FF'
        FONT_PATH={DEFAULT=''};FONT_SIZE={LARGE=15,MIDDLE=13}
        locale.questContext={doing='In-Progress'}
        title=string.rep('O',100)
        tip.style:SetColor(1,0,0,1)
        X2Quest={GetQuestContextMainTitle=function() return title end,
            IsProgressQuestInJournal=function() return true end,IsReadyQuestInJournal=function() return false end,
            GetLinkedQuestSummary=function() end,IsSelectiveQuest=function() return false end,
            IsScoreQuest=function() return false end,IsGroupQuest=function() return false end,
            NowIsAggroComponent=function() return false end,GetLinkedQuestObjectives=function() end}
        debug.setupvalue(originalQuest,1,5);section.SetQuestInfo=originalQuest
        section:SetQuestInfo(tip,123)
    "#,
    );
    let first = scale_paint(&ui);
    let mut previous_rows = first.rows;
    for scale in [1., 0.85] {
        run(&ui, &format!("UIParent:SetUIScale({scale})"));
        let changed = scale_paint(&ui);
        assert_ne!(changed.rows, previous_rows);
        assert_eq!(changed.rows as usize, changed.painted_rows.len());
        run(&ui, "tip:SetWidth(tip:GetWidth())");
        assert_eq!(
            scale_paint(&ui),
            changed,
            "original main/50 at scale{scale}"
        );
        previous_rows = changed.rows;
    }
    assert_eq!(scale_paint(&ui), first);
}

#[test]
fn scale_refit_includes_hidden_live_tooltips_and_preserves_empty_dead_and_same_scale_sizing() {
    for class in ["gametooltip", "unitframetooltip"] {
        let ui = setup(0.85);
        ui.lua.globals().set("scaleClass", class).unwrap();
        run(
            &ui,
            r#"
            tip:Show(false)
            tip=UIParent:CreateWidget(scaleClass,'hiddenScaleTip','UIParent')
            tip:SetWidth(250);tip:SetInset(10,9,10,11);tip:SetAutoWordwrap(true)
            a=tip:AddLine(string.rep('O',100),'',15,'right',ALIGN_LEFT,0,.68)
            tip:AttachLowerSpaceLine(a,5)
            AAUI_TooltipUnitValues=function() valueCalls=valueCalls+1;return {} end
            valueCalls=0;tip:SetTooltipData('player');valueCalls=0
            tip:SetHeight(777)
            local function empty(name,clear)
                local w=UIParent:CreateWidget(scaleClass,name,'UIParent')
                if clear then w:AddLine('O','',15,'left',ALIGN_LEFT,0);w:ClearLines() end
                w:SetHeight(321);return w
            end
            emptyTip=empty('neverPopulated',false);clearedTip=empty('cleared',true)
            deadTip=UIParent:CreateWidget(scaleClass,'deadScaleTip','UIParent')
            deadTip:AddLine('O','',15,'left',ALIGN_LEFT,0);deadTip:SetHeight(456)
            unrelated=UIParent:CreateWidget('label','unrelatedScaleLabel','UIParent');unrelated:SetHeight(654)
            UIParent:SetUIScale(.85)
            assert(tip:GetHeight()==777)
            local only,include=tip:GetHeightToLastLine()
            assert(only==777-11-5 and include==777-11)
        "#,
        );
        let dead = lua::value_id(&ui.lua, &eval(&ui, "deadTip")).unwrap();
        lua::ui(&ui.lua).tree.get_mut(dead).alive = false;
        // A Lua event handler can synchronously enter SetUIScale, read the
        // refitted height, and re-enter the idempotent scale setter.
        run(
            &ui,
            r#"
            trigger=UIParent:CreateWidget('emptywidget','scaleTrigger','UIParent')
            scaleHandled=false
            trigger:SetHandler('OnShow',function()
                UIParent:SetUIScale(1)
                assert(tip:GetHeight()~=777 and not tip:IsVisible())
                local height=tip:GetHeight()
                UIParent:SetUIScale(1)
                assert(tip:GetHeight()==height)
                scaleHandled=true
            end)
            trigger:Show(true)
            assert(scaleHandled)
            assert(emptyTip:GetHeight()==321 and clearedTip:GetHeight()==321)
            assert(unrelated:GetHeight()==654 and valueCalls==0)
            local height=tip:GetHeight()
            tip:SetWidth(tip:GetWidth());assert(tip:GetHeight()==height)
            tip:Show(true);tip:SetHeight(888);UIParent:SetUIScale(1)
            local only,include=tip:GetHeightToLastLine()
            assert(tip:GetHeight()==888 and only==888-11-5 and include==888-11)
        "#,
        );
        assert_eq!(lua::ui(&ui.lua).tree.get(dead).height, 456.);
    }
}

#[derive(Debug)]
struct RetainedPaint {
    height: f32,
    to_last: (f32, f32),
    rows: i64,
    effective_scale: f32,
    /// Native font raster identity and physical dimensions, independent of
    /// separately qualified absolute origin rounding.
    glyphs: Vec<([f32; 4], [f32; 2], [f32; 2])>,
}

fn retained_paint(ui: &UiRuntime) -> RetainedPaint {
    let height = eval(ui, "tip:GetHeight()");
    let to_last = ui
        .lua
        .load("return tip:GetHeightToLastLine()")
        .eval()
        .unwrap();
    let rows = eval(ui, "tip:GetLineCount()");
    let id = lua::value_id(&ui.lua, &eval(ui, "tip")).unwrap();
    let mut u = lua::ui(&ui.lua);
    let view = u.animations.widget_rect(&u.tree, id);
    let effective_scale = view.w / u.tree.rect(id).w * u.ui_scale;
    let (left, right, top) = (
        view.x * u.ui_scale,
        view.right() * u.ui_scale,
        view.y * u.ui_scale,
    );
    let glyphs: Vec<_> = render::build_quads(&mut u, None)
        .into_iter()
        .filter(|q| {
            q.tex == render::Tex::Atlas && q.pos[0] >= left && q.pos[0] < right && q.pos[1] >= top
        })
        .collect();
    let first = [
        glyphs
            .iter()
            .map(|q| q.pos[0])
            .fold(f32::INFINITY, f32::min),
        glyphs
            .iter()
            .map(|q| q.pos[1])
            .fold(f32::INFINITY, f32::min),
    ];
    let glyphs = glyphs
        .into_iter()
        .map(|q| {
            (
                q.uv,
                [q.pos[2] - q.pos[0], q.pos[3] - q.pos[1]],
                [q.pos[0] - first[0], q.pos[1] - first[1]],
            )
        })
        .collect();
    RetainedPaint {
        height,
        to_last,
        rows,
        effective_scale,
        glyphs,
    }
}

fn assert_retained_paint(actual: &RetainedPaint, expected: &RetainedPaint) {
    assert!((actual.effective_scale - expected.effective_scale).abs() < 0.00001);
    assert_eq!(actual.height, expected.height);
    assert_eq!(actual.to_last, expected.to_last);
    assert_eq!(actual.rows, expected.rows);
    assert!(!actual.glyphs.is_empty());
    assert_eq!(actual.glyphs.len(), expected.glyphs.len());
    for (a, b) in actual.glyphs.iter().zip(&expected.glyphs) {
        assert_eq!(
            a.0, b.0,
            "retained native font selected a different atlas raster"
        );
        for axis in 0..2 {
            assert!(
                (a.1[axis] - b.1[axis]).abs() < 0.001,
                "glyph physical extent changed: {a:?} vs {b:?}"
            );
            assert!(
                (a.2[axis] - b.2[axis]).abs() < 0.001,
                "relative column/row paint changed: {a:?} vs {b:?}"
            );
        }
    }
}

#[test]
fn frozen_root_and_ancestor_tooltips_keep_effective_font_wrap_and_height_through_global_scale() {
    for class in ["gametooltip", "unitframetooltip"] {
        for ancestor in [false, true] {
            let ui = setup(0.85);
            ui.lua.globals().set("scaleClass", class).unwrap();
            ui.lua.globals().set("freezeAncestor", ancestor).unwrap();
            run(
                &ui,
                r#"
                tip:Show(false)
                local parent=UIParent
                if freezeAncestor then
                    parent=UIParent:CreateWidget('emptywidget','frozenAncestor','UIParent')
                    parent:SetExtent(500,500);parent:Show(true);parent:ApplyUIScale(false)
                end
                tip=UIParent:CreateWidget(scaleClass,'frozenScaleTip',parent)
                tip:SetWidth(250);tip:SetInset(10,9,10,11);tip:SetAutoWordwrap(true)
                tip:AddAnchor('TOPLEFT',UIParent,100,100);tip:Show(true)
                a=tip:AddLine('|cFFFF0000'..string.rep('O',100),'',15,'right',ALIGN_LEFT,0,.68)
                tip:AttachUpperSpaceLine(a,3);tip:AttachLowerSpaceLine(a,5)
                if not freezeAncestor then tip:ApplyUIScale(false) end
            "#,
            );
            let initial = retained_paint(&ui);
            assert!((initial.effective_scale - 0.85).abs() < 0.00001);
            assert_eq!(initial.rows, 8);
            for scale in [1., 0.85] {
                run(&ui, &format!("UIParent:SetUIScale({scale})"));
                let changed = retained_paint(&ui);
                eprintln!(
                    "frozen {class} ancestor={ancestor} global={scale}: scale={}, height={}, rows={}, glyphs={}",
                    changed.effective_scale,
                    changed.height,
                    changed.rows,
                    changed.glyphs.len()
                );
                assert_retained_paint(&changed, &initial);
                // Setter/content/query paths must also use the retained scale.
                run(&ui, "tip:SetWidth(tip:GetWidth())");
                assert_retained_paint(&retained_paint(&ui), &initial);
                run(
                    &ui,
                    r#"
                    tip:ClearLines()
                    a=tip:AddLine('|cFFFF0000'..string.rep('O',100),'',15,'right',ALIGN_LEFT,0,.68)
                    tip:AttachUpperSpaceLine(a,3);tip:AttachLowerSpaceLine(a,5)
                "#,
                );
                assert_retained_paint(&retained_paint(&ui), &initial);
            }
            run(&ui, "tip:SetHeight(777);UIParent:SetUIScale(1)");
            assert_eq!(eval::<f32>(&ui, "tip:GetHeight()"), 777.);
        }
    }
}

#[test]
fn complete_original_tooltip_windows_populated_once_retain_their_frozen_scale_on_global_round_trip()
{
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    original_windows::install_at_scale(&mut ui, 0.85);
    let mut before = Vec::new();
    for index in [5, 6, 1, 8] {
        run(
            &ui,
            &format!("tip=tooltip.tooltipWindows[{index}].tooltipItemAndSkill"),
        );
        // Full original SetInfo (main/1/5) applies the current scale, populates
        // its native tooltip child, then ApplyUIScale(false) freezes its root.
        let snapshot = retained_paint(&ui);
        assert!((snapshot.effective_scale - 0.85).abs() < 0.00001);
        before.push((
            index,
            snapshot,
            eval::<f32>(&ui, &format!("tooltip.tooltipWindows[{index}]:GetHeight()")),
        ));
    }
    for scale in [1., 0.85] {
        run(&ui, &format!("UIParent:SetUIScale({scale})"));
        for (index, snapshot, frame_height) in &before {
            run(
                &ui,
                &format!("tip=tooltip.tooltipWindows[{index}].tooltipItemAndSkill"),
            );
            assert_retained_paint(&retained_paint(&ui), snapshot);
            assert_eq!(
                eval::<f32>(&ui, &format!("tooltip.tooltipWindows[{index}]:GetHeight()")),
                *frame_height
            );
            run(&ui, "tip:SetWidth(tip:GetWidth())");
            assert_retained_paint(&retained_paint(&ui), snapshot);
        }
    }
}
