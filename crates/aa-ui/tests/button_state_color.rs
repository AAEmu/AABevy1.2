//! Native strict setter/target goldens and actual original empty-stack consumer.
use aa_ui::{UiRuntime, input, lua, render, tree::Id};

#[path = "../examples/native_button_colors.rs"]
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
        .get(id(ui, name))
        .drawable()
        .unwrap()
        .color
}
fn setup(code: &str) -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    ui.exec(
        "@state_colors",
        r#"
        button=UIParent:CreateWidget('button','colorButton','UIParent');button:Show(true)
        bg=button:CreateColorDrawable(1,1,1,1,'artwork');button:SetNormalBackground(bg)
        normal=button:CreateNormalDrawable('colorDrawable',1,1,1,1,nil,'artwork')
        highlight=button:CreateHighlightDrawable('colorDrawable',1,1,1,1,nil,'artwork')
        pushed=button:CreatePushedDrawable('colorDrawable',1,1,1,1,nil,'artwork')
        disabled=button:CreateDisabledDrawable('colorDrawable',1,1,1,1,nil,'artwork')
        plain=button:CreateColorDrawable(0.2,0.3,0.4,0.5,'overlay')
        child=button:CreateChildWidget('label','colorChild',0,true);child:SetText('unchanged')
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 0.);
    ui.exec("@control_colors", code).unwrap();
    ui
}

#[test]
fn four_real_numbers_are_required_and_zero_arity_raw_f32_payloads_are_preserved() {
    let ui = setup(
        r#"
        local bad={
            function() return button:SetNormalColor() end,
            function() return button:SetNormalColor(1,1,1) end,
            function() return button:SetNormalColor(1,1,1,'0.3') end,
            function() return button:SetNormalColor(1,1,1,true) end,
            function() return button:SetNormalColor(nil,1,1,1) end,
            function() return button:SetNormalColor({},1,1,1) end,
            function() return button.SetNormalColor(child,1,1,1,0.3) end,
            function() return button.SetNormalColor(bg,1,1,1,0.3) end
        }
        for _,call in ipairs(bad) do assert(select('#',call())==0) end
        assert(bg:GetColor().a==1 and normal:GetColor().a==1)
        assert(select('#',button:SetNormalColor(-2,3,16777217,-0.3,'ignored'))==0)
    "#,
    );
    assert_eq!(
        color(&ui, "bg").map(f32::to_bits),
        [-2., 3., 16777216., -0.3f32].map(f32::to_bits)
    );
    assert_eq!(color(&ui, "normal"), color(&ui, "bg"));
    ui.exec(
        "@nonfinite_colors",
        "button:SetPushedColor(0/0,math.huge,-math.huge,-1/math.huge)",
    )
    .unwrap();
    let c = color(&ui, "pushed");
    assert!(c[0].is_nan());
    assert_eq!(c[1], f32::INFINITY);
    assert_eq!(c[2], f32::NEG_INFINITY);
    assert_eq!(c[3].to_bits(), (-0.0f32).to_bits());
}

#[test]
fn existing_background_and_hidden_state_extras_change_without_other_art_or_text() {
    let ui = setup(
        r#"
        pushed:SetVisible(false)
        button:SetNormalColor(0.1,0.2,0.3,0.4)
        button:SetHighlightColor(0.2,0.3,0.4,0.5)
        button:SetPushedColor(0.3,0.4,0.5,0.6)
    "#,
    );
    assert_eq!(color(&ui, "bg"), [0.1, 0.2, 0.3, 0.4]);
    assert_eq!(color(&ui, "normal"), [0.1, 0.2, 0.3, 0.4]);
    assert_eq!(color(&ui, "highlight"), [0.2, 0.3, 0.4, 0.5]);
    assert_eq!(color(&ui, "pushed"), [0.3, 0.4, 0.5, 0.6]);
    assert!(!lua::ui(&ui.lua).tree.get(id(&ui, "pushed")).shown);
    assert_eq!(color(&ui, "disabled"), [1.; 4]);
    assert_eq!(color(&ui, "plain"), [0.2, 0.3, 0.4, 0.5]);
    assert_eq!(
        lua::ui(&ui.lua)
            .tree
            .get(id(&ui, "child"))
            .text()
            .unwrap()
            .style
            .color,
        [1.; 4]
    );
    assert!(eval::<bool>(&ui, "button:IsEnabled()"));
}

#[test]
fn shared_background_aliases_are_mutated_and_late_art_keeps_authored_colors() {
    let ui = setup(
        r#"
        button:SetHighlightBackground(bg);button:SetPushedBackground(bg)
        button:SetPushedColor(0.4,0.5,0.6,0.7)
        local bare=UIParent:CreateWidget('button','lateColorButton','UIParent')
        bare:SetNormalColor(0.1,0.1,0.1,0.1)
        late=bare:CreateNormalDrawable('colorDrawable',0.6,0.7,0.8,0.9,nil,'artwork')
        lateBg=bare:CreateColorDrawable(0.3,0.4,0.5,0.6,'artwork');bare:SetNormalBackground(lateBg)
    "#,
    );
    assert_eq!(color(&ui, "bg"), [0.4, 0.5, 0.6, 0.7]);
    assert_eq!(color(&ui, "late"), [0.6, 0.7, 0.8, 0.9]);
    assert_eq!(color(&ui, "lateBg"), [0.3, 0.4, 0.5, 0.6]);
    assert!(eval::<bool>(
        &ui,
        r#"
        local slot=UIParent:CreateWidget('slot','surfaceSlot','UIParent')
        return slot.SetNormalColor==nil and child.SetNormalColor==nil and bg.SetNormalColor==nil
    "#
    ));
}

fn original_ui() -> UiRuntime {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    original::install(&mut ui);
    ui
}

fn icon_alphas(ui: &UiRuntime, button: Id, pressed: Option<Id>) -> Vec<f32> {
    let (r, path, scale) = {
        let mut u = lua::ui(&ui.lua);
        let background = u.tree.widget(button).unwrap().button.as_ref().unwrap().bg[0].unwrap();
        let path = u
            .tree
            .get(background)
            .drawable()
            .unwrap()
            .texture
            .clone()
            .unwrap();
        (
            u.animations.widget_rect(&u.tree, button),
            u.textures.resolve(&path).unwrap(),
            u.ui_scale,
        )
    };
    render::build_quads(&mut lua::ui(&ui.lua), pressed)
        .into_iter()
        .filter(|q| {
            matches!(&q.tex,render::Tex::Image(p)|render::Tex::MaskedImage{image:p,..}if p==&path)
                && q.pos[0] >= r.x * scale - 1.
                && q.pos[2] <= r.right() * scale + 1.
                && q.pos[1] >= r.y * scale - 1.
                && q.pos[3] <= r.bottom() * scale + 1.
        })
        .map(|q| q.color[3])
        .collect()
}

#[test]
fn original_empty_stack_dim_restore_changes_rendered_icons_and_preserves_disabled_art_and_backdrop() {
    let ui = original_ui();
    for (i, alpha) in [(1, 1.), (2, 0.3), (3, 1.)] {
        for state in 1..=3 {
            assert_eq!(
                color(&ui, &format!("colorButtons[{i}].bgs[{state}]"))[3],
                alpha
            );
        }
        assert_eq!(color(&ui, &format!("colorButtons[{i}].bgs[4]"))[3], 0.5);
        assert_eq!(color(&ui, &format!("colorButtons[{i}].back"))[3], 1.);
        assert_eq!(color(&ui, &format!("colorButtons[{i}].overlay"))[3], 0.);
        let actual = icon_alphas(&ui, id(&ui, &format!("colorButtons[{i}]")), None);
        assert!(!actual.is_empty());
        assert!(actual.iter().all(|&a| a == alpha), "{actual:?}");
    }
}

#[test]
fn original_dim_button_hover_press_release_remains_clickable_and_uses_matching_state_art() {
    let ui = original_ui();
    let button = id(&ui, "colorButtons[2]");
    let point = {
        let u = lua::ui(&ui.lua);
        let r = u.animations.widget_rect(&u.tree, button);
        (r.x + r.w * 0.5, r.y + r.h * 0.5)
    };
    assert_eq!(input::pick(&ui.lua, point.0, point.1), Some(button));
    let mut mouse = input::MouseState::default();
    input::update(&ui.lua, &mut mouse, Some(point), false, false, false, 0.);
    let hover = icon_alphas(&ui, button, None);
    assert!(!hover.is_empty());
    // Native image-plus-mask submission truncates the authored alpha to RGBA8.
    assert!(hover.iter().all(|&a| a == 76. / 255.));
    input::update(&ui.lua, &mut mouse, Some(point), true, false, false, 0.);
    assert_eq!(mouse.pressed, Some(button));
    let pushed = icon_alphas(&ui, button, mouse.pressed);
    assert!(!pushed.is_empty());
    assert!(pushed.iter().all(|&a| a == 0.3));
    input::update(&ui.lua, &mut mouse, Some(point), false, true, false, 0.);
    assert_eq!(eval::<i32>(&ui, "colorClicks"), 1);
    ui.exec("@disabled_state", "colorButtons[2]:Enable(false)")
        .unwrap();
    let disabled = icon_alphas(&ui, button, None);
    assert!(!disabled.is_empty());
    assert!(disabled.iter().all(|&a| a == 0.5));
    ui.exec(
        "@restore_after_click",
        "colorButtons[2]:Enable(true);colorButtons[2]:ProcessEmptyStackColor(false)",
    )
    .unwrap();
    assert!(icon_alphas(&ui, button, None).iter().all(|&a| a == 1.));
}
