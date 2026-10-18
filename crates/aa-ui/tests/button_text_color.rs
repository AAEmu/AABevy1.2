//! Actual palette/current-style separation and unchanged original shop radio consumers.
use aa_ui::{
    UiRuntime, input, lua,
    render::{self, Tex},
    tree::{ButtonState, Id},
};
#[path = "../examples/native_button_text.rs"]
#[allow(dead_code)]
mod original;
fn eval<T: mlua::FromLuaMulti>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().expect(code)
}
fn id(ui: &UiRuntime, name: &str) -> Id {
    lua::value_id(&ui.lua, &eval(ui, name)).unwrap()
}
fn color(ui: &UiRuntime, name: &str) -> [f32; 4] {
    lua::ui(&ui.lua)
        .tree
        .widget(id(ui, name))
        .unwrap()
        .text
        .as_ref()
        .unwrap()
        .style
        .color
}
fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    ui.exec(
        "@text_button",
        r#"
        b=UIParent:CreateWidget('button','textColor','UIParent')
        b:SetExtent(240,36);b:AddAnchor('TOPLEFT',UIParent,200,200);b:Show(true)
        b:SetText('Palette lifetime');b.style:SetShadow(false)
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 0.);
    ui
}
fn glyph_colors(ui: &UiRuntime) -> Vec<[f32; 4]> {
    render::build_quads(&mut lua::ui(&ui.lua), None)
        .into_iter()
        .filter(|q| q.tex == Tex::Atlas)
        .map(|q| q.color)
        .collect()
}

#[test]
fn all_four_fresh_entries_are_white_and_missing_states_do_not_fall_back_to_normal() {
    let ui = setup();
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .widget(id(&ui, "b"))
            .unwrap()
            .button
            .as_ref()
            .unwrap()
            .text_color,
        [[1.; 4]; 4]
    );
    ui.exec(
        "@normal_only",
        "b:SetTextColor(0.2,0.3,0.4,0.5);b:SetButtonState('HIGHLIGHTED')",
    )
    .unwrap();
    assert_eq!(color(&ui, "b"), [1.; 4]);
    assert!(glyph_colors(&ui).iter().all(|c| *c == [1.; 4]));
}

#[test]
fn selected_setters_update_current_style_other_entries_wait_for_state_write() {
    let ui = setup();
    for (state, method, index) in [
        ("NORMAL", "SetTextColor", 0),
        ("HIGHLIGHTED", "SetHighlightTextColor", 1),
        ("PUSHED", "SetPushedTextColor", 2),
        ("DISABLED", "SetDisabledTextColor", 3),
    ] {
        ui.exec(
            "@selected",
            &format!("b:SetButtonState('{state}');b:{method}(0.2,0.4,0.6,0.8)"),
        )
        .unwrap();
        assert_eq!(color(&ui, "b"), [0.2, 0.4, 0.6, 0.8]);
        ui.exec("@direct", "b.style:SetColor(0.9,0.7,0.5,0.3)")
            .unwrap();
        for (other, which) in [
            ("SetTextColor", 0),
            ("SetHighlightTextColor", 1),
            ("SetPushedTextColor", 2),
            ("SetDisabledTextColor", 3),
        ] {
            if which == index {
                continue;
            }
            ui.exec("@other", &format!("b:{other}(0.1,0.3,0.5,0.7)"))
                .unwrap();
            assert_eq!(color(&ui, "b"), [0.9, 0.7, 0.5, 0.3]);
        }
        ui.exec("@repeat_state", &format!("b:SetButtonState('{state}')"))
            .unwrap();
        assert_eq!(color(&ui, "b"), [0.2, 0.4, 0.6, 0.8]);
    }
}

#[test]
fn direct_style_survives_repeated_paints_tick_and_unrelated_changes() {
    let ui = setup();
    ui.exec(
        "@palette_direct",
        r#"
        b:SetTextColor(0.2,0.3,0.4,1);b.style:SetColor(0.8,0.6,0.4,0.5)
        child=b:CreateChildWidget('label','child',0,true);child:SetText('')
        b:SetText('Direct style');b:SetWidth(260)
    "#,
    )
    .unwrap();
    for _ in 0..3 {
        let colors = glyph_colors(&ui);
        assert!(!colors.is_empty());
        assert!(
            colors.iter().all(|c| *c == [0.8, 0.6, 0.4, 0.5]),
            "{colors:?}"
        );
        lua::tick(&ui.lua, 16.);
    }
    ui.exec("@same_setter", "b:SetTextColor(0.2,0.3,0.4,1)")
        .unwrap();
    assert_eq!(color(&ui, "b"), [0.2, 0.3, 0.4, 1.]);
}

#[test]
fn strict_four_number_wrappers_have_zero_arity_and_retain_raw_f32_values() {
    let ui = setup();
    ui.exec("@strict_colors",r#"
        for _,method in ipairs({'SetTextColor','SetHighlightTextColor','SetPushedTextColor','SetDisabledTextColor'})do
            for _,args in ipairs({{}, {1,2,3}, {1,2,3,'4'}, {1,2,3,true}, {false,2,3,4}})do
                assert(select('#',b[method](b,unpack(args)))==0)
            end
        end
        for _,args in ipairs({{}, {1,2,3}, {1,2,3,'4'}, {1,2,3,true}})do
            assert(select('#',b.style:SetColor(unpack(args)))==0)
        end
    "#).unwrap();
    assert_eq!(color(&ui, "b"), [1.; 4]);
    ui.exec("@raw", "b:SetTextColor(-0,16777217,0/0,1/0,'ignored')")
        .unwrap();
    let c = color(&ui, "b");
    assert_eq!(c[0].to_bits(), (-0.0f32).to_bits());
    assert_eq!(c[1], 16777216.);
    assert!(c[2].is_nan() && c[3].is_infinite());
    ui.exec("@raw_direct", "b.style:SetColor(-2,3,-1/0,0/0)")
        .unwrap();
    let c = color(&ui, "b");
    assert_eq!(c[..2], [-2., 3.]);
    assert!(c[2].is_infinite() && c[3].is_nan());
}

#[test]
fn original_shop_confirmation_direct_radio_color_then_physical_states_and_choice() {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    original::install(&mut ui);
    let target = id(&ui, "textDetails[2].textButton");
    let expected: Vec<f32> = eval(&ui, "return FONT_COLOR.DEFAULT");
    let initial = color(&ui, "textDetails[2].textButton");
    assert_eq!(initial, expected.as_slice());
    let palette = lua::ui(&ui.lua)
        .tree
        .widget(target)
        .unwrap()
        .button
        .as_ref()
        .unwrap()
        .text_color;
    assert_ne!(
        initial, palette[0],
        "the original direct color must exercise a distinct current style"
    );
    let (x, y) = {
        let u = lua::ui(&ui.lua);
        let r = u.animations.widget_rect(&u.tree, target);
        (r.x + r.w * 0.5, r.y + r.h * 0.5)
    };
    assert_eq!(input::pick(&ui.lua, x, y), Some(target));
    let mut mouse = Default::default();
    input::update(&ui.lua, &mut mouse, Some((x, y)), false, false, false, 0.);
    assert_eq!(color(&ui, "textDetails[2].textButton"), palette[1]);
    input::update(&ui.lua, &mut mouse, Some((x, y)), true, false, false, 0.);
    assert_eq!(color(&ui, "textDetails[2].textButton"), palette[2]);
    assert_eq!(eval::<String>(&ui, "return textEvents[1]"), "PUSHED");
    input::update(&ui.lua, &mut mouse, Some((x, y)), false, true, false, 0.);
    assert_eq!(eval::<i32>(&ui, "return textDetails:GetChecked()"), 2);
    input::update(&ui.lua, &mut mouse, Some((0., 0.)), false, false, false, 0.);
    assert_eq!(color(&ui, "textDetails[2].textButton"), palette[0]);
    assert_eq!(color(&ui, "textDetails[3].textButton"), initial);
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .widget(target)
            .unwrap()
            .button
            .as_ref()
            .unwrap()
            .state,
        ButtonState::Normal
    );
    assert!(ui.errors.is_empty());
}
