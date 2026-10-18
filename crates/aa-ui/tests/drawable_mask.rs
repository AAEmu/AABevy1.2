//! Executed native image/icon mask contracts and original physical hover/cooldown consumers.
use aa_ui::{
    UiRuntime, input, lua,
    render::{self, Quad, Tex},
    tree::{DrawableMask, Id},
};
#[path = "../examples/native_drawable_mask.rs"]
#[allow(dead_code)]
mod original;
const MASK: &str = "ui/icon/cooldown_mask.dds";
fn eval<T: mlua::FromLuaMulti>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().expect(code)
}
fn id(ui: &UiRuntime, name: &str) -> Id {
    lua::value_id(&ui.lua, &eval(ui, name)).unwrap()
}
fn mask(ui: &UiRuntime, name: &str) -> Option<DrawableMask> {
    lua::ui(&ui.lua)
        .tree
        .get(id(ui, name))
        .drawable()
        .unwrap()
        .mask
        .clone()
}
fn visible(ui: &UiRuntime, name: &str) -> bool {
    lua::ui(&ui.lua)
        .tree
        .get(id(ui, name))
        .drawable()
        .unwrap()
        .mask_visible
}
fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    ui.exec(
        "@mask_art",
        r#"
        window=UIParent:CreateWidget('window','maskArt','UIParent')
        window:SetExtent(80,80);window:AddAnchor('TOPLEFT',UIParent,100,100);window:Show(true)
        image=window:CreateImageDrawable('ui/icon/icon_skill_basic01.dds','artwork')
        image:SetExtent(42,42);image:AddAnchor('TOPLEFT',window,0,0)
        icon=window:CreateIconImageDrawable('ui/icon/icon_skill_basic01.dds','artwork')
        icon:SetExtent(42,42);icon:AddAnchor('TOPLEFT',window,0,0);icon:SetVisible(false)
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 0.);
    ui
}
fn quads(ui: &UiRuntime) -> Vec<Quad> {
    render::build_quads(&mut lua::ui(&ui.lua), None)
}

#[test]
fn strict_wrappers_return_nothing_and_do_not_coerce_strings_or_wrong_classes() {
    let ui = setup();
    ui.exec(
        "@strict_masks",
        r#"
        local color=window:CreateColorDrawable(1,1,1,1,'overlay')
        for _,d in ipairs({image,icon}) do
            for _,call in ipairs({
                function()return d:SetMask()end,function()return d:SetMask(nil)end,
                function()return d:SetMask(1)end,function()return d:SetMask(true)end,
                function()return d:SetMaskCoords(1,2,3)end,
                function()return d:SetMaskCoords(1,2,3,'4')end,
                function()return d:SetMaskVisible()end,
                function()return d:SetMaskVisible('true')end,
                function()return d.SetMask(color,'ui/icon/cooldown_mask.dds')end,
                function()return d.SetMask(window,'ui/icon/cooldown_mask.dds')end
            })do assert(select('#',call())==0)end
        end
    "#,
    )
    .unwrap();
    assert!(mask(&ui, "image").is_none() && mask(&ui, "icon").is_none());
    assert!(!visible(&ui, "image") && !visible(&ui, "icon"));
    ui.exec(
        "@strict_flags",
        r#"
        assert(select('#',image:SetMaskVisible(-2))==0)
        icon:SetMaskVisible(0/0);image:SetMaskVisible('false')
    "#,
    )
    .unwrap();
    assert!(visible(&ui, "image") && visible(&ui, "icon"));
}

#[test]
fn load_failure_retains_texture_uv_and_flag_while_reload_resets_only_uv() {
    let ui = setup();
    ui.exec(
        "@load",
        r#"
        for _,d in ipairs({image,icon})do
            d:SetMaskVisible(true)
            assert(select('#',d:SetMask('ui/icon/cooldown_mask.dds\0ignored'))==0)
            d:SetMaskCoords(16,32,64,96)
        end
    "#,
    )
    .unwrap();
    for name in ["image", "icon"] {
        let old = mask(&ui, name).unwrap();
        assert_eq!(old.texture, MASK);
        assert_eq!(old.uv, [0.0625, 0.125, 0.3125, 0.5]);
        ui.exec(
            "@failed_load",
            &format!("{name}:SetMask('ui/no-such-original-mask.dds')"),
        )
        .unwrap();
        assert_eq!(mask(&ui, name), Some(old));
        assert!(visible(&ui, name));
        ui.exec("@reload", &format!("{name}:SetMask('{MASK}')"))
            .unwrap();
        assert_eq!(mask(&ui, name).unwrap().uv, [0., 0., 1., 1.]);
        assert!(visible(&ui, name));
    }
}

#[test]
fn retired_owner_removes_masked_art_and_stale_lua_methods_cannot_mutate_it() {
    let ui = setup();
    ui.exec(
        "@retired_mask",
        r#"
        image:SetMask('ui/icon/cooldown_mask.dds');image:SetMaskVisible(true)
        window:EnableHidingIsRemove(true);window:Show(false)
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 0.);
    let target = id(&ui, "image");
    assert!(!lua::ui(&ui.lua).tree.get(target).alive);
    let old = mask(&ui, "image");
    ui.exec(
        "@stale_mask",
        r#"
        assert(select('#',image:SetMask('ui/common/slot_over.dds'))==0)
        image:SetMaskCoords(1,2,3,4);image:SetMaskVisible(false)
    "#,
    )
    .unwrap();
    assert_eq!(mask(&ui, "image"), old);
    assert!(visible(&ui, "image"));
    assert!(quads(&ui).is_empty());
}

#[test]
fn native_uv_endpoints_and_raw_nonfinite_values_remain_independent_of_visibility() {
    let ui = setup();
    for name in ["image", "icon"] {
        // Native image is a no-op without a texture; icon's invalid dereference is safely refused.
        ui.exec(
            "@unloaded",
            &format!("{name}:SetMaskCoords(1,2,3,4);{name}:SetMask('{MASK}')"),
        )
        .unwrap();
        for (values, bits) in [
            (
                [16., 32., 0., 0.],
                [0x3d800000, 0x3e000000, 0x3f800000, 0x3f800000],
            ),
            (
                [16., 32., -8., -16.],
                [0x3d800000, 0x3e000000, 0x3d000000, 0x3d800000],
            ),
            (
                [0.3, 0.7, 100.1, 200.2],
                [0x3a99999a, 0x3b333333, 0x3ec8cccd, 0x3f48e666],
            ),
        ] {
            ui.lua
                .globals()
                .set("coords", ui.lua.create_sequence_from(values).unwrap())
                .unwrap();
            ui.exec("@coords", &format!("{name}:SetMaskCoords(unpack(coords))"))
                .unwrap();
            assert_eq!(mask(&ui, name).unwrap().uv.map(f32::to_bits), bits);
        }
        ui.exec("@nan_coords", &format!("{name}:SetMaskCoords(0/0,0,1,1)"))
            .unwrap();
        assert!(mask(&ui, name).unwrap().uv[0].is_nan());
        assert!(!visible(&ui, name));
    }
}

#[test]
fn real_mask_pass_preserves_two_uv_streams_native_color_bytes_and_manual_visibility() {
    let ui = setup();
    ui.exec(
        "@draw",
        r#"
        image:SetMask('ui/icon/cooldown_mask.dds');image:SetMaskCoords(16,32,64,96)
        image:SetCoords(8,16,32,64);image:SetColor(.3,.5,.7,.4)
        image:SetMaskVisible(true)
    "#,
    )
    .unwrap();
    let qs = quads(&ui);
    assert_eq!(qs.len(), 1);
    let Tex::MaskedImage {
        image,
        mask,
        image_uv,
        mask_uv,
    } = &qs[0].tex
    else {
        panic!("masked native image")
    };
    assert!(image.ends_with("icon_skill_basic01.dds"));
    assert_eq!(mask, MASK);
    let (w, h) = lua::ui(&ui.lua).textures.size(image).unwrap();
    assert_eq!(
        image_uv.map(f32::from_bits),
        [
            8. / w as f32,
            16. / h as f32,
            40. / w as f32,
            80. / h as f32
        ]
    );
    assert_eq!(mask_uv.map(f32::from_bits), [0.0625, 0.125, 0.3125, 0.5]);
    assert_eq!(qs[0].uv, [0., 0., 1., 1.]);
    assert_eq!(
        qs[0].color,
        [76. / 255., 127. / 255., 178. / 255., 102. / 255.]
    );
    ui.exec("@hide_mask", "image:SetMaskVisible(false)")
        .unwrap();
    assert!(matches!(quads(&ui)[0].tex, Tex::Image(_)));
    assert_eq!(quads(&ui)[0].color, [0.3, 0.5, 0.7, 0.4]);
    ui.exec(
        "@hide_image",
        "image:SetMaskVisible(true);image:SetVisible(false)",
    )
    .unwrap();
    assert!(quads(&ui).is_empty());
}

#[test]
fn icon_layers_share_primary_uv_and_mask_but_keep_their_texture_identity() {
    let ui = setup();
    ui.exec(
        "@icon_layers",
        r#"
        image:SetVisible(false);icon:SetVisible(true)
        icon:AddTexture('ui/common/hud.dds');icon:SetCoords(8,16,32,64)
        icon:SetMask('ui/icon/cooldown_mask.dds');icon:SetMaskVisible(true)
    "#,
    )
    .unwrap();
    let qs = quads(&ui);
    assert_eq!(qs.len(), 2);
    let Tex::MaskedImage {
        image: a,
        image_uv: a_uv,
        mask_uv: a_mask,
        ..
    } = &qs[0].tex
    else {
        panic!("base")
    };
    let Tex::MaskedImage {
        image: b,
        image_uv: b_uv,
        mask_uv: b_mask,
        ..
    } = &qs[1].tex
    else {
        panic!("overlay")
    };
    assert_ne!(a, b);
    assert_eq!(a_uv, b_uv);
    assert_eq!(a_mask, b_mask);
    ui.exec("@plain_layers", "icon:SetMaskVisible(false)")
        .unwrap();
    let plain = quads(&ui);
    assert_eq!(plain[0].uv, plain[1].uv);
    assert_eq!(plain[0].uv, a_uv.map(f32::from_bits));
}

#[test]
fn highlight_state_reapply_enables_loaded_masks_including_late_extras() {
    let ui = setup();
    ui.exec("@highlight_lifetime",r#"
        button=UIParent:CreateWidget('button','maskedButton','UIParent')
        button:SetHighlightBackground(image)
        image:SetMask('ui/icon/cooldown_mask.dds');image:SetMaskVisible(false)
        button:SetButtonState('NORMAL')
        assert(not image:IsVisible())
        late=button:CreateHighlightDrawable('drawable',1,1,1,1,'ui/icon/icon_skill_basic01.dds','artwork')
        late:SetMask('ui/icon/cooldown_mask.dds');late:SetMaskVisible(false)
        button:SetButtonState('NORMAL')
    "#).unwrap();
    assert!(visible(&ui, "image") && visible(&ui, "late"));
    ui.exec("@manual_mask_off", "image:SetMaskVisible(false)")
        .unwrap();
    assert!(!visible(&ui, "image"));
    ui.exec("@repeat_state", "button:SetButtonState('HIGHLIGHTED')")
        .unwrap();
    assert!(visible(&ui, "image"));
}

#[test]
fn clipped_rotated_scaled_images_keep_unit_uvs_attached_to_both_source_rectangles() {
    let ui = setup();
    ui.exec("@clip_mask",r#"
        image:SetVisible(false);window:SetExtent(30,30);window:EnableScroll(true);window:SetScale(.5)
        child=window:CreateChildWidget('emptywidget','maskChild',0,true)
        child:SetExtent(42,42);child:AddAnchor('TOPLEFT',window,0,0)
        art=child:CreateImageDrawable('ui/icon/icon_skill_basic01.dds','artwork')
        art:SetExtent(42,42);art:AddAnchor('TOPLEFT',child,0,0);art:SetRotation(.4)
        art:SetMask('ui/icon/cooldown_mask.dds');art:SetMaskCoords(16,32,64,96);art:SetMaskVisible(true)
    "#).unwrap();
    lua::tick(&ui.lua, 0.);
    let bounds = {
        let u = lua::ui(&ui.lua);
        let r = u.animations.widget_rect(&u.tree, id(&ui, "window"));
        // Ancestor scissor bounds are physical pixel edges, after UI scaling/snapping.
        [r.x, r.y, r.right(), r.bottom()].map(|v| (v * u.ui_scale).round())
    };
    let qs = quads(&ui);
    assert!(!qs.is_empty());
    for q in qs {
        let Tex::MaskedImage { mask_uv, .. } = q.tex else {
            panic!("clipped mask")
        };
        assert_eq!(mask_uv.map(f32::from_bits), [0.0625, 0.125, 0.3125, 0.5]);
        for p in q.corners.unwrap() {
            assert!(
                p[0] >= bounds[0] - 0.001
                    && p[0] <= bounds[2] + 0.001
                    && p[1] >= bounds[1] - 0.001
                    && p[1] <= bounds[3] + 0.001,
                "point={p:?} clip={bounds:?}"
            )
        }
        assert!(
            q.uvs
                .unwrap()
                .iter()
                .flatten()
                .all(|v| (-0.001..=1.001).contains(v))
        );
    }
}

#[test]
fn original_physical_hover_keeps_cooldown_fan_and_click_while_mask_roles_stay_independent() {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    original::install(&mut ui);
    let target = id(&ui, "colorButtons[2]");
    let p = {
        let u = lua::ui(&ui.lua);
        let r = u.animations.widget_rect(&u.tree, target);
        (r.x + r.w * 0.5, r.y + r.h * 0.5)
    };
    let count = |suffix: &str| {
        quads(&ui)
            .iter()
            .filter(|q| matches!(&q.tex,Tex::MaskedImage{mask,..}if mask.ends_with(suffix)))
            .count()
    };
    let fans = count("cooldown_mask.dds");
    assert!(fans > 0);
    let hover_before = count("slot_over.dds");
    assert_eq!(input::pick(&ui.lua, p.0, p.1), Some(target));
    let mut mouse = Default::default();
    input::update(&ui.lua, &mut mouse, Some(p), false, false, false, 0.);
    assert!(count("slot_over.dds") > hover_before);
    assert_eq!(count("cooldown_mask.dds"), fans);
    input::update(&ui.lua, &mut mouse, Some(p), true, false, false, 0.);
    assert_eq!(count("cooldown_mask.dds"), fans);
    input::update(&ui.lua, &mut mouse, Some(p), false, true, false, 0.);
    assert_eq!(eval::<u32>(&ui, "colorClicks"), 1);
    assert_eq!(count("cooldown_mask.dds"), fans);
    input::update(
        &ui.lua,
        &mut mouse,
        Some((10., 10.)),
        false,
        false,
        false,
        0.,
    );
    assert_eq!(count("cooldown_mask.dds"), fans);
}
