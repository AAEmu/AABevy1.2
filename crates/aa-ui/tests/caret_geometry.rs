//! Original edit skins must extend the caret vertically, without moving its text boundary.
//! Native shared draw39286500 and the two signed-integer adapter pairs are the oracle.
#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::{UiKey, UiRuntime, lua, render};

const CURSOR: [f32; 4] = [0.137, 0.271, 0.419, 1.0];

fn run(ui: &UiRuntime, code: &str) {
    ui.exec("@caret_geometry", code).unwrap();
}

fn id(ui: &UiRuntime) -> u32 {
    lua::value_id(&ui.lua, &ui.lua.load("edit").eval().unwrap()).unwrap()
}

fn caret(ui: &UiRuntime) -> Option<[f32; 4]> {
    let mut u = lua::ui(&ui.lua);
    render::build_quads(&mut u, None)
        .into_iter()
        .find(|q| q.tex == render::Tex::Atlas && q.color == CURSOR)
        .map(|q| q.pos)
}

fn setup(multiline: bool, scale: f32) -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    run(
        &ui,
        &format!(
            "edit=UIParent:CreateWidget('{}','caretEdit','UIParent');edit:SetExtent(300,160);\
             edit:AddAnchor('TOPLEFT',UIParent,100,100);edit:Show(true);\
             edit:SetInset(20,20,20,20);edit:SetText('AB\\nCD');edit.style:SetSnap(false);\
             edit:SetCursorColor({},{},{},1)",
            if multiline {
                "editboxmultiline"
            } else {
                "editbox"
            },
            CURSOR[0],
            CURSOR[1],
            CURSOR[2]
        ),
    );
    run(&ui, &format!("UIParent:SetUIScale({scale})"));
    ui.set_focus(Some(id(&ui)));
    ui
}

fn near(a: f32, b: f32) {
    assert!((a - b).abs() < 0.001, "{a} != {b}");
}

#[test]
fn native_endpoint_parameters_are_vertical_independent_and_unscaled_pixels() {
    for multiline in [false, true] {
        for scale in [1., 0.85, 1.5] {
            let ui = setup(multiline, scale);
            let base = caret(&ui).unwrap();
            run(&ui, "edit:SetCursorOffset(-2)");
            let offset = caret(&ui).unwrap();
            near(offset[0], base[0]);
            near(offset[2], base[2]);
            near(offset[1], base[1] - 2.);
            near(offset[3], base[3]);
            run(&ui, "edit:SetCursorHeight(2)");
            let styled = caret(&ui).unwrap();
            near(styled[0], base[0]);
            near(styled[1], base[1] - 2.);
            near(styled[3], base[3] + 2.);
            run(&ui, "edit:SetCursorOffset(3);edit:SetCursorHeight(-1)");
            let inward = caret(&ui).unwrap();
            near(inward[1], base[1] + 3.);
            near(inward[3], base[3] - 1.);
        }
    }
}

#[test]
fn emitted_caret_uses_the_visible_text_baseline_native_three_pixels_and_snap_shift() {
    let ui = setup(false, 1.);
    run(
        &ui,
        "edit:SetText('O');edit:SetCursorOffset(-2);edit:SetCursorHeight(2)",
    );
    let mut u = lua::ui(&ui.lua);
    let text = u
        .tree
        .widget(id(&ui))
        .unwrap()
        .text
        .as_ref()
        .unwrap()
        .clone();
    let layout = u
        .fonts
        .layout_edit(&text.style.font, text.style.size, "O", None, 0.);
    let glyph = layout.glyphs.first().unwrap();
    let drawn = render::build_quads(&mut u, None);
    let glyph_quad = drawn.iter().find(|q| q.color != CURSOR).unwrap();
    let baseline = glyph_quad.pos[1] - glyph.glyph.top as f32;
    let pen = glyph_quad.pos[0] - glyph.glyph.left as f32;
    let expected = [
        (pen + layout.width + 0.5).trunc(),
        (baseline - layout.line_height + 3. - 2. + 0.5).trunc(),
        (pen + layout.width + 0.5).trunc() + 1.,
        (baseline + 3. + 2. + 0.5).trunc(),
    ];
    drop(u);
    assert_eq!(caret(&ui).unwrap(), expected);
    run(&ui, "edit.style:SetSnap(true)");
    let snapped = caret(&ui).unwrap();
    assert_eq!(snapped, expected.map(|v| v - 1.));
}

#[test]
fn multiline_caret_endpoints_follow_painted_second_line_after_origin_rounding() {
    let ui = setup(true, 0.85);
    run(
        &ui,
        "edit:SetInset(20,16,20,20);edit:SetLineSpace(2);edit:SetText('O\\nO')",
    );
    let mut u = lua::ui(&ui.lua);
    let text = u
        .tree
        .widget(id(&ui))
        .unwrap()
        .text
        .as_ref()
        .unwrap()
        .clone();
    let layout = u.fonts.layout_edit(
        &text.style.font,
        text.style.size * 0.85,
        "O\nO",
        None,
        2. * 0.85,
    );
    let glyph = layout.glyphs.last().unwrap();
    let drawn = render::build_quads(&mut u, None);
    let painted = drawn.iter().filter(|q| q.color != CURSOR).last().unwrap();
    let baseline = painted.pos[1] - glyph.glyph.top as f32;
    let expected = [
        (baseline - layout.line_height + 3. + 0.5).trunc(),
        (baseline + 3. + 0.5).trunc(),
    ];
    drop(u);
    let caret = caret(&ui).unwrap();
    assert_eq!(
        [caret[1], caret[3]],
        expected,
        "caret must follow the actually painted second-line baseline"
    );
}

#[test]
fn native_signed_integer_narrowing_rejects_wrong_types_and_keeps_raw_endpoints() {
    for multiline in [false, true] {
        let ui = setup(multiline, 1.);
        run(&ui, "edit:SetCursorOffset(-2.9);edit:SetCursorHeight(2.9)");
        let value = || {
            let u = lua::ui(&ui.lua);
            let e = u.tree.widget(id(&ui)).unwrap().edit.as_ref().unwrap();
            (e.cursor_offset, e.cursor_height)
        };
        assert_eq!(value(), (-2., Some(2.)));
        run(
            &ui,
            r#"
            for _,v in ipairs({true,false,'4','bad',{}})do
              assert(select('#',edit:SetCursorOffset(v))==0)
              assert(select('#',edit:SetCursorHeight(v))==0)
            end
            assert(select('#',edit:SetCursorOffset())==0)
            assert(select('#',edit:SetCursorHeight(nil))==0)
        "#,
        );
        assert_eq!(value(), (-2., Some(2.)));
        run(
            &ui,
            &format!(
                r#"
            other=UIParent:CreateWidget('{}','otherCaret','UIParent')
            other:SetCursorOffset(5);other:SetCursorHeight(6)
            assert(select('#',edit.SetCursorOffset(other,9))==0)
            assert(select('#',edit.SetCursorHeight(other,9))==0)
        "#,
                if multiline {
                    "editbox"
                } else {
                    "editboxmultiline"
                }
            ),
        );
        let other = lua::value_id(&ui.lua, &ui.lua.load("other").eval().unwrap()).unwrap();
        let u = lua::ui(&ui.lua);
        let other = u.tree.widget(other).unwrap().edit.as_ref().unwrap();
        assert_eq!((other.cursor_offset, other.cursor_height), (5., Some(6.)));
        drop(u);
        run(
            &ui,
            "edit:SetCursorOffset(2147483647);edit:SetCursorHeight(0/0)",
        );
        assert_eq!(value(), (-2147483648., Some(-2147483648.)));
        run(
            &ui,
            "edit:SetCursorOffset(16777217);edit:SetCursorHeight(-1/0)",
        );
        assert_eq!(value(), (16777216., Some(-2147483648.)));
    }
}

#[test]
fn original_edit_skin_endpoints_reach_renderer_and_survive_edit_lifecycle() {
    let mut ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    fake::install(&mut ui, "en_us", &fake::default_worlds());
    ui.load_base_libs();
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    for (i, (class, scale)) in ["editbox", "editboxmultiline"]
        .into_iter()
        .flat_map(|class| [1., 0.85, 1.5].map(|scale| (class, scale)))
        .enumerate()
    {
        run(
            &ui,
            &format!(
                r#"
            UIParent:SetUIScale({scale})
            edit=UIParent:CreateWidget('{class}','originalCaret_{class}_{i}','UIParent')
            edit:SetExtent(300,160);edit:AddAnchor('TOPLEFT',UIParent,100,100);edit:Show(true)
            if '{class}'=='editbox' then InitEdit(edit) else InitMultiline(edit) end
            edit:SetText('AB\nCD');edit:SetCursorColor({},{},{},1)
        "#,
                CURSOR[0], CURSOR[1], CURSOR[2]
            ),
        );
        ui.set_focus(Some(id(&ui)));
        let original = caret(&ui).expect("original skin caret");
        run(&ui, "edit:SetCursorOffset(0);edit:SetCursorHeight(0)");
        let plain = caret(&ui).unwrap();
        near(original[0], plain[0]);
        near(original[1], plain[1] - 2.);
        near(original[3], plain[3] + 2.);
        run(
            &ui,
            "edit:SetCursorOffset(-2);edit:SetCursorHeight(2);edit:Show(false)",
        );
        // Show completion follows the established native widget frame lifecycle.
        lua::tick(&ui.lua, 0.);
        assert!(caret(&ui).is_none());
        run(&ui, "edit:Show(true)");
        lua::tick(&ui.lua, 0.);
        ui.set_focus(Some(id(&ui)));
        ui.key(&UiKey::Left);
        ui.type_text("Z");
        assert!(caret(&ui).is_some());
        run(&ui, "edit:Show(false)");
        lua::tick(&ui.lua, 0.);
        assert!(caret(&ui).is_none());
    }
}
