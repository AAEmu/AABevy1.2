//! Native owner overlay follows descendants, including original slot counters.
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

#[test]
fn original_slot_black_overlay_follows_its_child_stack_and_keeps_physical_clicks() {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    original::install(&mut ui);
    ui.exec(
        "@original_stack_overlay",
        r#"
        actionButton:ProcessEmptyStackColor(false)
        actionButton:SetStack(42)
        actionButton:SetOverlayColor('black')
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 0.);
    let target = id(&ui, "actionButton");
    assert_eq!(eval::<String>(&ui, "actionButton.stack:GetText()"), "42");
    let mut u = lua::ui(&ui.lua);
    let r = u.animations.widget_rect(&u.tree, target);
    let scale = u.ui_scale;
    let quads = render::build_quads(&mut u, None);
    let overlay = quads
        .iter()
        .position(|q| q.color == [0., 0., 0., 0.75])
        .unwrap();
    let glyph = quads
        .iter()
        .rposition(|q| {
            matches!(q.tex, render::Tex::Atlas)
                && q.color == [1.; 4]
                && q.pos[0] >= r.x * scale - 1.
                && q.pos[1] >= r.y * scale - 1.
                && q.pos[2] <= (r.x + r.w) * scale + 1.
                && q.pos[3] <= (r.y + r.h) * scale + 1.
        })
        .unwrap();
    assert!(
        glyph < overlay,
        "original child stack painted above owner black overlay: glyph{glyph}, overlay{overlay}"
    );
    let r = u.animations.widget_rect(&u.tree, target);
    let p = (r.x + r.w * 0.5, r.y + r.h * 0.5);
    drop(u);
    assert_eq!(input::pick(&ui.lua, p.0, p.1), Some(target));
    let mut mouse = Default::default();
    input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.);
    input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.);
    assert_eq!(eval::<u32>(&ui, "artClicks"), 1);
}

const ORDER: [&str; 16] = [
    "root.background",
    "root.artwork",
    "root.content",
    "a.background",
    "a.artwork",
    "a.content",
    "g.background",
    "g.artwork",
    "g.content",
    "g.overlay",
    "a.overlay",
    "b.background",
    "b.artwork",
    "b.content",
    "b.overlay",
    "root.overlay",
];
fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    ui.exec("@native_order_graph",r#"
        orderNodes={};orderNames={'root','a','b','g'}
        local function node(n,parent,size)
            local w=UIParent:CreateWidget('button',orderNames[n],parent or 'UIParent')
            w:SetExtent(size,size);w:AddAnchor('TOPLEFT',parent or UIParent,parent and 5 or 100,parent and 5 or 100)
            w:SetText('X');w:Show(true);orderNodes[n]=w
            local layers={'background','artwork','overlay'}
            for i,layer in ipairs(layers) do
                local index=(n-1)*4+(i==3 and 4 or i)
                local d=w:CreateColorDrawable(index/32,0.25,0.5,1,layer)
                d:AddAnchor('TOPLEFT',w,0,0);d:AddAnchor('BOTTOMRIGHT',w,0,0)
            end
            w.style:SetColor(((n-1)*4+3)/32,0.25,0.5,1)
            return w
        end
        root=node(1,nil,100);a=node(2,root,60);b=node(3,root,60);g=node(4,a,30)
    "#).unwrap();
    lua::tick(&ui.lua, 0.);
    ui
}
fn trace(ui: &UiRuntime) -> Vec<String> {
    render::build_quads(&mut lua::ui(&ui.lua), None)
        .iter()
        .filter_map(|q| {
            if q.color[1..] != [0.25, 0.5, 1.] {
                return None;
            }
            let n = (q.color[0] * 32.) as usize;
            if !(1..=16).contains(&n) {
                return None;
            }
            Some(format!(
                "{}.{}",
                ["root", "a", "b", "g"][(n - 1) / 4],
                ["background", "artwork", "content", "overlay"][(n - 1) % 4]
            ))
        })
        .collect()
}

#[test]
fn native_three_level_order_survives_clipping_and_disabled_owner_layers() {
    // Executed3927b010/3927afd0/3927af50 trace, restricted to supported layers0..2.
    for (configure, exclude) in [
        ("", None),
        ("root:EnableScroll(true)", None),
        ("a:EnableScroll(true)", None),
        ("root:EnableScroll(true);a:EnableScroll(true)", None),
        ("root:DisableDrawables('overlay')", Some("root.overlay")),
        ("a:DisableDrawables('overlay')", Some("a.overlay")),
    ] {
        let ui = setup();
        ui.exec("@admission", configure).unwrap();
        assert_eq!(
            trace(&ui),
            ORDER
                .iter()
                .filter(|&&s| Some(s) != exclude)
                .map(|s| s.to_string())
                .collect::<Vec<_>>(),
            "{configure}"
        );
    }
}

#[test]
fn hidden_child_subtree_is_excluded_while_native_pending_fade_keeps_its_position() {
    let ui = setup();
    ui.exec("@hidden_child", "a:Show(false)").unwrap();
    // The native manager settles the immediate record before the hidden draw.
    lua::tick(&ui.lua, 0.);
    assert_eq!(
        trace(&ui),
        ORDER
            .iter()
            .filter(|s| !s.starts_with("a.") && !s.starts_with("g."))
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
    );
    ui.exec("@show_child", "a:Show(true)").unwrap();
    lua::tick(&ui.lua, 0.);
    ui.exec("@pending_fade", "a:Show(false,1000)").unwrap();
    assert!(!eval::<bool>(&ui, "a:IsVisible()"));
    assert_eq!(trace(&ui), ORDER);
    lua::tick(&ui.lua, 1000.);
    assert_eq!(
        trace(&ui),
        ORDER
            .iter()
            .filter(|s| !s.starts_with("a.") && !s.starts_with("g."))
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn generic_effects_advance_once_in_each_actual_layer_pass_even_at_alpha_zero() {
    let ui = setup();
    ui.exec(
        "@ordered_effects",
        r#"
        local function effect(owner,layer)
            local e=owner:CreateEffectDrawable('ui/common/hud.dds',layer)
            e:SetExtent(20,20);e:SetCoords(462,233,3,3);e:AddAnchor('CENTER',owner,0,0)
            e:SetRepeatCount(1);e:SetEffectPriority(1,'alpha',0.5,0.4)
            e:SetEffectInitialColor(1,1,1,1,0);e:SetEffectFinalColor(1,1,1,1,1)
            e:SetStartEffect(true);return e
        end
        before=effect(root,'background');after=effect(root,'overlay');nested=effect(g,'overlay')
        root:SetAlpha(0)
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 100.);
    assert!(render::build_quads(&mut lua::ui(&ui.lua), None).is_empty());
    for name in ["before", "after", "nested"] {
        assert_eq!(
            lua::ui(&ui.lua)
                .animations
                .sample(id(&ui, name))
                .color
                .unwrap()[3]
                .to_bits(),
            0.21250000596046448_f32.to_bits(),
            "{name}"
        );
    }
    ui.exec(
        "@pause_overlay_only",
        "root:DisableDrawables('overlay');root:SetAlpha(1)",
    )
    .unwrap();
    lua::tick(&ui.lua, 100.);
    render::build_quads(&mut lua::ui(&ui.lua), None);
    for (name, expected) in [
        ("before", 0.44999998807907104_f32),
        ("after", 0.21250000596046448),
        ("nested", 0.44999998807907104),
    ] {
        assert_eq!(
            lua::ui(&ui.lua)
                .animations
                .sample(id(&ui, name))
                .color
                .unwrap()[3]
                .to_bits(),
            expected.to_bits(),
            "{name}"
        );
    }
}

#[test]
fn closing_overlay_keeps_its_owner_transform_and_existing_ancestor_clip() {
    let ui = setup();
    ui.exec(
        "@scaled_clip",
        r#"
        root:EnableScroll(true)
        a:SetExtent(80,80);a:RemoveAllAnchors();a:AddAnchor('TOPLEFT',root,50,50)
        root:SetScaleAnimation(1,2,1,1,0,0)
        root:SetStartAnimation(false,true)
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 500.);
    let mut u = lua::ui(&ui.lua);
    let q = render::build_quads(&mut u, None);
    let bg = q
        .iter()
        .find(|q| q.color == [5. / 32., 0.25, 0.5, 1.])
        .unwrap();
    let overlay = q
        .iter()
        .find(|q| q.color == [8. / 32., 0.25, 0.5, 1.])
        .unwrap();
    assert_eq!(bg.pos, overlay.pos);
    assert_eq!(bg.uv, overlay.uv);
    assert!(
        bg.pos[2] - bg.pos[0] < 80. * 1.5,
        "owner overlay escaped its existing clip"
    );
    assert!(
        q.iter().position(|q| q.color == overlay.color).unwrap()
            > q.iter()
                .position(|q| q.color == [16. / 32., 0.25, 0.5, 1.])
                .unwrap()
    );
}

#[test]
fn background_camera_split_contains_all_closing_passes_of_background_roots() {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    ui.exec(
        "@split_order",
        r#"
        back=UIParent:CreateWidget('window','backgroundRoot','UIParent');back:Show(true)
        back:SetExtent(100,100);back:SetUILayer('background')
        front=UIParent:CreateWidget('window','normalRoot','UIParent');front:Show(true)
        front:SetExtent(100,100)
        local function color(w,r,layer)
            local d=w:CreateColorDrawable(r,0,0,1,layer)
            d:AddAnchor('TOPLEFT',w,0,0);d:AddAnchor('BOTTOMRIGHT',w,0,0)
        end
        local child=back:CreateChildWidget('emptywidget','backChild',0,true);child:SetExtent(20,20)
        color(back,0.25,'background');color(back,0.5,'overlay');color(child,0.75,'background')
        color(front,1,'background')
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 0.);
    let (quads, n) = render::build_quads_split(&mut lua::ui(&ui.lua), None);
    assert_eq!(n, 3);
    assert_eq!(
        quads.iter().map(|q| q.color[0]).collect::<Vec<_>>(),
        [0.25, 0.75, 0.5, 1.]
    );
}
