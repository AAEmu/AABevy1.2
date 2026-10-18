//! Native owner-layer admission, independent of drawable visibility and button state.
use aa_ui::{UiRuntime, input, lua, render, tree::Id};
#[path = "../examples/native_button_art.rs"]
#[allow(dead_code)]
mod original;

fn eval<T: mlua::FromLuaMulti>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().expect(code)
}
fn id(ui: &UiRuntime, name: &str) -> Id {
    lua::value_id(&ui.lua, &eval(ui, name)).unwrap()
}
fn image_count(ui: &UiRuntime, owner: Id) -> usize {
    let mut u = lua::ui(&ui.lua);
    let r = u.animations.widget_rect(&u.tree, owner);
    let scale = u.ui_scale;
    render::build_quads(&mut u, None)
        .iter()
        .filter(|q| {
            !matches!(q.tex, render::Tex::Atlas)
                && q.pos[0] >= r.x * scale - 1.
                && q.pos[1] >= r.y * scale - 1.
                && q.pos[2] <= (r.x + r.w) * scale + 1.
                && q.pos[3] <= (r.y + r.h) * scale + 1.
        })
        .count()
}
fn icon_count(ui: &UiRuntime) -> usize {
    let normal = id(ui, "actionButton.bgs[BUTTON_STATE.NORMAL]");
    let textures = {
        let u = lua::ui(&ui.lua);
        let d = u.tree.get(normal).drawable().unwrap();
        d.texture
            .iter()
            .chain(&d.extra_textures)
            .cloned()
            .collect::<Vec<_>>()
    };
    render::build_quads(&mut lua::ui(&ui.lua), None)
        .iter()
        .filter(|q| match &q.tex {
            render::Tex::Image(path) | render::Tex::MaskedImage { image: path, .. } => {
                textures.contains(path)
            }
            _ => false,
        })
        .count()
}

fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    ui.exec(
        "@layers",
        r#"
        host=UIParent:CreateWidget('button','layerHost','UIParent')
        host:SetExtent(120,60);host:AddAnchor('TOPLEFT',UIParent,100,100);host:Show(true)
        local function art(layer,r,g,b)
            local d=host:CreateColorDrawable(r,g,b,1,layer)
            d:AddAnchor('TOPLEFT',host,0,0);d:AddAnchor('BOTTOMRIGHT',host,0,0)
            return d
        end
        bg=art('background',0.11,0.22,0.33)
        image=art('artwork',0.22,0.33,0.44)
        over=art('overlay',0.33,0.44,0.55)
        hidden=art('background',0.44,0.55,0.66);hidden:SetVisible(false)
        child=host:CreateChildWidget('emptywidget','layerChild',0,true)
        child:SetExtent(20,20);child:AddAnchor('TOPLEFT',host,5,5);child:Show(true)
        childArt=child:CreateColorDrawable(0.55,0.66,0.77,1,'background')
        childArt:AddAnchor('TOPLEFT',child,0,0);childArt:AddAnchor('BOTTOMRIGHT',child,0,0)
        host:SetText('Layer text')
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 0.);
    ui
}

fn flags(ui: &UiRuntime, name: &str) -> [bool; 4] {
    lua::ui(&ui.lua)
        .tree
        .widget(id(ui, name))
        .unwrap()
        .drawable_layers
}
fn color_count(ui: &UiRuntime, color: [f32; 4]) -> usize {
    render::build_quads(&mut lua::ui(&ui.lua), None)
        .iter()
        .filter(|q| q.color == color)
        .count()
}

#[test]
fn original_button_skin_gate_preserves_shown_and_input_then_button_init_restores_art() {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    original::install(&mut ui);
    let target = id(&ui, "artButtons[1]");
    let original_count = image_count(&ui, target);
    assert!(original_count > 0);
    ui.exec(
        "@disable_original_skin",
        r#"
        artButtons[1]:DisableDrawables('background')
        assert(artButtons[1].bgs[BUTTON_STATE.NORMAL]:IsVisible())
        assert(artButtons[1]:IsVisible() and artButtons[1]:IsEnabled())
    "#,
    )
    .unwrap();
    assert_eq!(
        image_count(&ui, target),
        0,
        "disabled owner background still painted its original skin"
    );
    let p = {
        let u = lua::ui(&ui.lua);
        let r = u.animations.widget_rect(&u.tree, target);
        (r.x + r.w * 0.5, r.y + r.h * 0.5)
    };
    assert_eq!(input::pick(&ui.lua, p.0, p.1), Some(target));
    let mut mouse = Default::default();
    input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.);
    input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.);
    assert_eq!(eval::<u32>(&ui, "artClicks"), 1);
    ui.exec(
        "@original_init_restores_layer",
        "ButtonInit(artButtons[1]);artButtons[1]:SetButtonState('NORMAL')",
    )
    .unwrap();
    assert_eq!(image_count(&ui, target), original_count);
}

#[test]
fn native_typed_name_goldens_require_string_use_case_sensitive_nul_prefix_and_unknown_all() {
    // Native39267c10 ->392674c0 ->3922c120 ->39277bb0:32 executed groups.
    let cases = [
        ("", None),
        ("nil", None),
        ("true", None),
        ("0", None),
        ("{}", None),
        ("'background'", Some(0)),
        ("'artwork'", Some(1)),
        ("'overlay'", Some(2)),
        ("'overoverlay'", Some(3)),
        ("'BACKGROUND'", Some(4)),
        ("'all'", Some(4)),
        ("''", Some(4)),
        ("'highlight'", Some(4)),
        ("'background\\0overlay'", Some(0)),
        ("'artwork',false", Some(1)),
    ];
    let ui = setup();
    for on in [false, true] {
        let (method, baseline) = if on {
            ("EnableDrawables", "DisableDrawables")
        } else {
            ("DisableDrawables", "EnableDrawables")
        };
        for (args, layer) in cases {
            ui.exec("@baseline", &format!("host:{baseline}('all')"))
                .unwrap();
            let mut expected = [!on; 4];
            if let Some(layer) = layer {
                if layer == 4 {
                    expected.fill(on)
                } else {
                    expected[layer] = on
                }
            }
            assert_eq!(
                eval::<usize>(&ui, &format!("select('#',host:{method}({args}))")),
                0
            );
            assert_eq!(flags(&ui, "host"), expected, "{method}({args})");
            assert_eq!(flags(&ui, "child"), [true; 4]);
            assert!(eval::<bool>(
                &ui,
                "bg:IsVisible() and image:IsVisible() and over:IsVisible() and not hidden:IsVisible()"
            ));
        }
        ui.exec("@baseline", &format!("host:{baseline}('all')"))
            .unwrap();
        assert_eq!(
            eval::<usize>(&ui, &format!("select('#',host.{method}(bg,'background'))")),
            0
        );
        assert_eq!(flags(&ui, "host"), [!on; 4]);
    }
}

#[test]
fn layers_gate_existing_and_late_art_without_changing_children_text_or_dynamic_factors() {
    let ui = setup();
    assert_eq!(flags(&ui, "host"), [true; 4]);
    let host = id(&ui, "host");
    {
        let mut u = lua::ui(&ui.lua);
        let factors = &mut u.tree.widget_mut(host).unwrap().dynamic_drawable;
        factors.current = [0.3, 0.4, 0.75, 0.9];
        factors.enabled = [true, false, true, false];
        factors.pairs[0] = [0.13, 0.91];
    }
    let factors = lua::ui(&ui.lua)
        .tree
        .widget(host)
        .unwrap()
        .dynamic_drawable
        .clone();
    ui.exec(
        "@all_off",
        r#"
        host:DisableDrawables('unknown')
        late=host:CreateColorDrawable(0.66,0.77,0.88,1,'overlay')
        late:SetExtent(10,10);late:AddAnchor('CENTER',host,0,0)
        assert(late:IsVisible() and not hidden:IsVisible())
    "#,
    )
    .unwrap();
    assert_eq!(color_count(&ui, [0.11, 0.22, 0.33, 1.]), 0);
    assert_eq!(color_count(&ui, [0.66, 0.77, 0.88, 1.]), 0);
    assert_eq!(color_count(&ui, [0.55, 0.66, 0.77, 1.]), 1);
    assert!(
        render::build_quads(&mut lua::ui(&ui.lua), None)
            .iter()
            .any(|q| matches!(q.tex, render::Tex::Atlas) && q.color == [1.; 4])
    );
    assert_eq!(
        lua::ui(&ui.lua).tree.widget(host).unwrap().dynamic_drawable,
        factors
    );
    ui.exec("@late_visible", "host:EnableDrawables('overlay')")
        .unwrap();
    assert_eq!(color_count(&ui, [0.66, 0.77, 0.88, 0.75]), 1);
    ui.exec(
        "@remove_art",
        "host:RemoveAllDrawables();host:EnableDrawables('all')",
    )
    .unwrap();
    assert!(
        lua::ui(&ui.lua)
            .tree
            .widget(host)
            .unwrap()
            .drawables
            .is_empty()
    );
    assert_eq!(flags(&ui, "host"), [true; 4]);
    assert_eq!(flags(&ui, "child"), [true; 4]);
}

#[test]
fn original_icon_refresh_enables_only_background_without_reviving_hidden_owned_art() {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    original::install(&mut ui);
    ui.exec(
        "@refresh_icon",
        r#"
        iconMark=actionButton:CreateColorDrawable(0.15,0.85,0.25,1,'background')
        iconMark:SetExtent(20,20);iconMark:AddAnchor('CENTER',actionButton,0,0)
        iconMark:SetVisible(false)
        actionButton:DisableDrawables('background');actionButton:DisableDrawables('artwork')
        actionButton:SetItemIcon(4045,2)
        assert(not iconMark:IsVisible())
        assert(actionButton.bgs[BUTTON_STATE.NORMAL]:IsVisible())
    "#,
    )
    .unwrap();
    assert_eq!(flags(&ui, "actionButton"), [true, false, true, true]);
    assert_eq!(icon_count(&ui), 0);
    ui.exec("@restore_icon", "actionButton:EnableDrawables('artwork')")
        .unwrap();
    assert!(icon_count(&ui) > 0);
    assert!(!eval::<bool>(&ui, "iconMark:IsVisible()"));
}

#[test]
fn disabled_owner_layer_pauses_effect_clock_without_suppressing_child_owner_clock() {
    let ui = setup();
    ui.exec(
        "@layer_effects",
        r#"
        local function make_effect(owner)
            local e=owner:CreateEffectDrawable('ui/common/hud.dds','overlay')
            e:SetExtent(20,20);e:SetCoords(462,233,3,3);e:AddAnchor('CENTER',owner,0,0)
            e:SetRepeatCount(1);e:SetEffectPriority(1,'alpha',0.5,0.4)
            e:SetEffectInitialColor(1,1,1,1,0);e:SetEffectFinalColor(1,1,1,1,1)
            e:SetStartEffect(true);return e
        end
        effect=make_effect(host);childEffect=make_effect(child)
    "#,
    )
    .unwrap();
    let effect = id(&ui, "effect");
    let child = id(&ui, "childEffect");
    lua::tick(&ui.lua, 100.);
    render::build_quads(&mut lua::ui(&ui.lua), None);
    let first = lua::ui(&ui.lua).animations.sample(effect);
    assert_eq!(
        first.color.unwrap()[3].to_bits(),
        0.21250000596046448_f32.to_bits()
    );
    ui.exec("@pause_owner_overlay", "host:DisableDrawables('overlay')")
        .unwrap();
    lua::tick(&ui.lua, 100.);
    render::build_quads(&mut lua::ui(&ui.lua), None);
    assert_eq!(lua::ui(&ui.lua).animations.sample(effect), first);
    assert_eq!(
        lua::ui(&ui.lua).animations.sample(child).color.unwrap()[3].to_bits(),
        0.44999998807907104_f32.to_bits()
    );
    ui.exec("@pause_child_overlay", "child:DisableDrawables('overlay')")
        .unwrap();
    lua::ui(&ui.lua).tree.settle();
    let settled = lua::ui(&ui.lua).tree.settle();
    lua::tick(&ui.lua, 500.);
    assert_eq!(
        lua::ui(&ui.lua).tree.settle(),
        settled,
        "disabled layers kept requesting repaint"
    );
    render::build_quads(&mut lua::ui(&ui.lua), None);
    assert_eq!(lua::ui(&ui.lua).animations.sample(effect), first);
    ui.exec("@resume_owner", "host:EnableDrawables('overlay')")
        .unwrap();
    lua::tick(&ui.lua, 100.);
    render::build_quads(&mut lua::ui(&ui.lua), None);
    assert_eq!(
        lua::ui(&ui.lua).animations.sample(effect).color.unwrap()[3].to_bits(),
        0.44999998807907104_f32.to_bits()
    );
    assert!(eval::<bool>(
        &ui,
        "effect:IsVisible() and childEffect:IsVisible()"
    ));
}
