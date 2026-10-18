//! Native mask ownership/UV goldens and selected-art geometry.
use aa_ui::{
    UiRuntime, lua,
    render::{Quad, Tex},
    tree::Id,
};

const MASK: &str = "ui/icon/cooldown_mask.dds";

fn setup() -> (UiRuntime, Id) {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    ui.exec(
        "@mask",
        r#"
        button=UIParent:CreateWidget('cooldownbutton','maskButton','UIParent')
        button:SetExtent(42,42);button:AddAnchor('TOPLEFT',UIParent,100,100);button:Show(true)
        normal=button:CreateImageDrawable('ui/icon/icon_skill_basic01.dds','artwork')
        normal:AddAnchor('TOPLEFT',button,1,1);normal:AddAnchor('BOTTOMRIGHT',button,-1,-1)
        button:SetNormalBackground(normal);button:SetCoolDown(1000,1000)
    "#,
    )
    .unwrap();
    let id = lua::value_id(&ui.lua, &ui.lua.load("button").eval().unwrap()).unwrap();
    (ui, id)
}

fn mask(ui: &UiRuntime, id: Id) -> Option<lua::slot::CooldownMask> {
    lua::ui(&ui.lua)
        .tree
        .widget(id)
        .unwrap()
        .button
        .as_ref()
        .unwrap()
        .cooldown_mask
        .clone()
}
fn quads(ui: &UiRuntime) -> Vec<Quad> {
    aa_ui::render::build_quads(&mut lua::ui(&ui.lua), None)
        .into_iter()
        .filter(|q| q.corners.is_some() && matches!(q.tex, Tex::MaskedImage { .. }))
        .collect()
}

#[test]
fn absent_mask_load_failure_and_successful_reload_match_native_lifetime() {
    let (ui, id) = setup();
    assert!(mask(&ui, id).is_none());
    assert!(quads(&ui).is_empty());
    // Native would dereference a missing texture in this invalid ordering; refuse safely.
    ui.exec("@early_coords", "button:SetCoolDownMaskCoords(1,2,3,4)")
        .unwrap();
    assert!(mask(&ui, id).is_none());
    ui.exec(
        "@load",
        r#"assert(select('#',button:SetCoolDownMask('ui/icon/cooldown_mask.dds'))==0)"#,
    )
    .unwrap();
    assert_eq!(mask(&ui, id).unwrap().uv, [0., 0., 1., 1.]);
    assert_eq!(quads(&ui).len(), 8);
    ui.exec("@crop", "button:SetCoolDownMaskCoords(16,32,64,96)")
        .unwrap();
    let before = mask(&ui, id).unwrap();
    ui.exec(
        "@failure",
        "button:SetCoolDownMask('ui/no-such-native-mask.dds')",
    )
    .unwrap();
    assert_eq!(mask(&ui, id), Some(before));
    ui.exec(
        "@reload",
        "button:SetCoolDownMask('ui/icon/cooldown_mask.dds')",
    )
    .unwrap();
    assert_eq!(mask(&ui, id).unwrap().uv, [0., 0., 1., 1.]);
    ui.exec("@clear_timer", "button:SetCoolDown(0,0)").unwrap();
    assert!(quads(&ui).is_empty());
    assert!(mask(&ui, id).is_some());
}

#[test]
fn coordinates_keep_native_float_endpoints_and_zero_extent_rules() {
    let (ui, id) = setup();
    ui.exec(
        "@load",
        "button:SetCoolDownMask('ui/icon/cooldown_mask.dds')",
    )
    .unwrap();
    // Actual392809e0/3922b9d0 bit patterns from the executed native mask oracle.
    for (coords, expected) in [
        (
            [16., 32., 64., 96.],
            [0x3d800000, 0x3e000000, 0x3ea00000, 0x3f000000],
        ),
        (
            [16., 32., 0., 0.],
            [0x3d800000, 0x3e000000, 0x3f800000, 0x3f800000],
        ),
        (
            [16., 32., -8., -16.],
            [0x3d800000, 0x3e000000, 0x3d000000, 0x3d800000],
        ),
        (
            [8., 4., 0., 32.],
            [0x3d000000, 0x3c800000, 0x3f800000, 0x3e100000],
        ),
        (
            [8., 4., 32., 0.],
            [0x3d000000, 0x3c800000, 0x3e200000, 0x3f800000],
        ),
        (
            [0.3, 0.7, 100.1, 200.2],
            [0x3a99999a, 0x3b333333, 0x3ec8cccd, 0x3f48e666],
        ),
    ] {
        ui.lua
            .globals()
            .set("coords", ui.lua.create_sequence_from(coords).unwrap())
            .unwrap();
        ui.exec("@coords", "button:SetCoolDownMaskCoords(unpack(coords))")
            .unwrap();
        assert_eq!(mask(&ui, id).unwrap().uv.map(f32::to_bits), expected);
    }
}

#[test]
fn mask_samples_actual_texture_over_the_selected_background_rectangle() {
    let (ui, _) = setup();
    ui.exec(
        "@normal",
        r#"
        button:SetCoolDownMask('ui/icon/cooldown_mask.dds')
        button:SetCoolDownMaskCoords(16,32,64,96)
    "#,
    )
    .unwrap();
    let qs = quads(&ui);
    assert_eq!(qs.len(), 8);
    for q in qs {
        let Tex::MaskedImage {
            image,
            mask,
            image_uv,
            mask_uv,
        } = &q.tex
        else {
            panic!("dual-texture pass")
        };
        assert!(image.ends_with("icon_skill_basic01.dds"));
        assert_eq!(mask, MASK);
        assert_eq!(image_uv.map(f32::from_bits), [0., 0., 1., 1.]);
        assert_eq!(mask_uv.map(f32::from_bits), [0.0625, 0.125, 0.3125, 0.5]);
        assert_eq!(q.color, [1., 1., 1., 1.]);
        for (p, uv) in q.corners.unwrap().iter().zip(q.uvs.unwrap()) {
            assert!((101. - 0.001..=141. + 0.001).contains(&p[0]));
            assert!((101. - 0.001..=141. + 0.001).contains(&p[1]));
            assert!((0.0..=1.0).contains(&uv[0]));
            assert!((0.0..=1.0).contains(&uv[1]));
        }
    }
    ui.exec(
        "@highlight",
        r#"
        highlight=button:CreateImageDrawable('ui/icon/icon_skill_basic01.dds','artwork')
        highlight:SetExtent(20,18);highlight:AddAnchor('TOPLEFT',button,7,9)
        button:SetHighlightBackground(highlight)
    "#,
    )
    .unwrap();
    // This bare cooldownbutton has no script input handlers. Select its native
    // state explicitly; physical original-button transitions are covered in button_art.
    ui.exec("@selected_state", "button:SetButtonState('HIGHLIGHTED')").unwrap();
    assert_eq!(ui.lua.load("button:GetButtonState()").eval::<String>().unwrap(), "HIGHLIGHTED");
    let qs = quads(&ui);
    assert!(!qs.is_empty());
    for q in qs {
        for p in q.corners.unwrap() {
            assert!((107. - 0.001..=127. + 0.001).contains(&p[0]));
            assert!((109. - 0.001..=127. + 0.001).contains(&p[1]));
        }
    }
}

#[test]
fn mask_requires_an_image_or_icon_background() {
    let (ui, _) = setup();
    ui.exec(
        "@nine_part",
        r#"
        button:SetCoolDownMask('ui/icon/cooldown_mask.dds')
        local nine=button:CreateNinePartDrawable('ui/hud.dds','artwork')
        nine:AddAnchor('TOPLEFT',button,0,0);nine:AddAnchor('BOTTOMRIGHT',button,0,0)
        button:SetNormalBackground(nine)
    "#,
    )
    .unwrap();
    assert!(quads(&ui).is_empty());
}

#[test]
fn native_eight_sector_goldens_preserve_linear_time_and_independent_uv_streams() {
    use aa_ui::lua::slot::mask_triangles;
    // Actual3928ace0 submission capture, native-cooldown-fan-oracle-20261018.json.
    for (progress, count, start) in [
        (0., 8, [0.5, 0.]),
        (0.0625, 8, [0.75, 0.]),
        (0.125, 7, [1., 0.]),
        (0.2, 7, [1., 0.3]),
        (0.25, 6, [1., 0.5]),
        (0.5, 4, [0.5, 1.]),
        (0.875, 1, [0., 0.]),
        (0.999, 1, [0.49600005, 0.]),
    ] {
        let fan = mask_triangles(progress);
        assert_eq!(fan.len(), count);
        for axis in 0..2 {
            assert!((fan[0][2][axis] - start[axis]).abs() < 1e-6);
        }
    }
    assert!(mask_triangles(1.).is_empty());
    let (ui, _) = setup();
    ui.exec(
        "@independent_uv",
        r#"
        normal:SetCoords(8,12,16,20);normal:SetColor(0.3,0.5,0.7,0.4)
        button:SetCoolDownMask('ui/icon/cooldown_mask.dds')
        button:SetCoolDownMaskCoords(16,32,-8,-16)
        button:SetCoolDown(800,1000)
    "#,
    )
    .unwrap();
    let qs = quads(&ui);
    assert_eq!(qs.len(), 7);
    for q in qs {
        let Tex::MaskedImage {
            image_uv, mask_uv, ..
        } = q.tex
        else {
            unreachable!()
        };
        assert_eq!(image_uv.map(f32::from_bits), [0.2, 0.3, 0.6, 0.8]);
        assert_eq!(
            mask_uv.map(f32::from_bits),
            [0.0625, 0.125, 0.03125, 0.0625]
        );
        assert_eq!(q.color, [76. / 255., 127. / 255., 178. / 255., 102. / 255.]);
    }
}

#[test]
fn icon_layers_share_native_uv_while_mask_uv_remains_independent() {
    let (ui, _) = setup();
    ui.exec(
        "@layers",
        r#"
        local icon=button:CreateIconImageDrawable('ui/icon/icon_skill_basic01.dds','artwork')
        icon:AddAnchor('TOPLEFT',button,1,1);icon:AddAnchor('BOTTOMRIGHT',button,-1,-1)
        icon:SetCoords(8,12,16,20);icon:AddTexture('ui/common/slot_over.dds')
        button:SetNormalBackground(icon);button:SetCoolDownMask('ui/icon/cooldown_mask.dds')
        button:SetCoolDownMaskCoords(16,32,0,0)
    "#,
    )
    .unwrap();
    let qs = quads(&ui);
    assert_eq!(qs.len(), 16);
    for q in qs {
        let Tex::MaskedImage {
            image_uv, mask_uv, ..
        } = q.tex
        else {
            unreachable!()
        };
        assert_eq!(image_uv.map(f32::from_bits), [0.2, 0.3, 0.6, 0.8]);
        assert_eq!(mask_uv.map(f32::from_bits), [0.0625, 0.125, 1., 1.]);
    }
}

#[test]
fn native_pixel_snap_keeps_fractional_uvs_before_rgba8_submission() {
    let (ui, _) = setup();
    ui.exec(
        "@raster",
        r#"
        button:SetCoolDownMask('ui/icon/cooldown_mask.dds')
        button:SetCoolDown(2000,3000)
    "#,
    )
    .unwrap();
    let q = quads(&ui).remove(0);
    assert_eq!(q.corners.unwrap()[2], [141., 134.]);
    assert!((q.uvs.unwrap()[2][1] - 0.8333334).abs() < 1e-6);
}

fn snapshot_bits(ui: &UiRuntime, id: Id) -> (u32, u32) {
    let u = lua::ui(&ui.lua);
    let cd = u.tree.widget(id).unwrap().cooldown.as_ref().unwrap();
    (cd.sampled_ms.unwrap().to_bits(), cd.total_ms.to_bits())
}

#[test]
fn actual_typed_setter_requires_two_numbers_and_preserves_rejected_samples() {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    for (i, class) in [
        "cooldownbutton",
        "cooldownactionbutton",
        "cooldowninventorybutton",
        "cooldownconstantbutton",
    ]
    .into_iter()
    .enumerate()
    {
        ui.exec("@class", &format!("button=UIParent:CreateWidget('{class}','rawCooldown{i}','UIParent');button:SetCoolDown(8000,12000)")).unwrap();
        let id = lua::value_id(&ui.lua, &ui.lua.load("button").eval().unwrap()).unwrap();
        for args in [
            "",
            "1000",
            "nil,1000",
            "1000,nil",
            "'1000',2000",
            "1000,'2000'",
            "true,2000",
            "{},2000",
        ] {
            ui.exec(
                "@strict_args",
                &format!("assert(select('#',button:SetCoolDown({args}))==0)"),
            )
            .unwrap();
            assert_eq!(
                snapshot_bits(&ui, id),
                (8000f32.to_bits(), 12000f32.to_bits()),
                "{class}: {args}"
            );
        }
        ui.exec(
            "@extra_arg",
            "assert(select('#',button:SetCoolDown(500,1000,99))==0)",
        )
        .unwrap();
        assert_eq!(
            snapshot_bits(&ui, id),
            (500f32.to_bits(), 1000f32.to_bits())
        );
    }
}

#[test]
fn raw_zero_negative_over_total_and_f32_values_survive_without_a_local_clock() {
    let (ui, id) = setup();
    for (args, expected) in [
        ("8000,4000", (0x45fa0000, 0x457a0000)),
        ("0,12000", (0, 0x463b8000)),
        ("-1,12000", (0xbf800000, 0x463b8000)),
        ("1000,0", (0x447a0000, 0)),
        ("1000,-1000", (0x447a0000, 0xc47a0000)),
        ("-0.0,-0.0", (0x80000000, 0x80000000)),
        ("16777217,16777219", (0x4b800000, 0x4b800002)),
        ("math.huge,1000", (0x7f800000, 0x447a0000)),
    ] {
        ui.exec("@raw_sample", &format!("button:SetCoolDown({args})"))
            .unwrap();
        assert_eq!(snapshot_bits(&ui, id), expected, "{args}");
        lua::tick(&ui.lua, 9000.);
        assert_eq!(snapshot_bits(&ui, id), expected, "after tick: {args}");
    }
    ui.lua.globals().set("rawNan", f64::NAN).unwrap();
    ui.exec("@nan", "button:SetCoolDown(rawNan,1000)").unwrap();
    assert!(f32::from_bits(snapshot_bits(&ui, id).0).is_nan());
}

#[test]
fn native_raw_draw_gates_and_carried_fan_endpoints_keep_nonfinite_inputs_finite() {
    use lua::slot::mask_triangles;
    // Actual3928ace0: first start stays at center for negative/unordered progress;
    // each later start is the previously delivered perimeter endpoint.
    for progress in [-1., f32::NEG_INFINITY, f32::NAN] {
        let fan = mask_triangles(progress);
        assert_eq!(fan.len(), 8);
        assert_eq!(fan[0], [[0.5, 0.5], [1., 0.], [0.5, 0.5]]);
        assert_eq!(fan[1], [[0.5, 0.5], [1., 0.5], [1., 0.]]);
        assert!(fan.iter().flatten().flatten().all(|v| v.is_finite()));
    }
    assert!(mask_triangles(f32::INFINITY).is_empty());
    let (ui, id) = setup();
    ui.lua.globals().set("rawNan", f64::NAN).unwrap();
    ui.exec(
        "@load",
        "button:SetCoolDownMask('ui/icon/cooldown_mask.dds')",
    )
    .unwrap();
    for (args, count) in [
        ("0,12000", 0),
        ("-1,12000", 0),
        ("8000,4000", 8),
        ("1000,0", 8),
        ("1000,-1000", 0),
        ("rawNan,1000", 8),
        ("math.huge,1000", 8),
        ("1000,math.huge", 8),
    ] {
        ui.exec("@raw_draw", &format!("button:SetCoolDown({args})"))
            .unwrap();
        let before = snapshot_bits(&ui, id);
        let qs = quads(&ui);
        assert_eq!(qs.len(), count, "{args}");
        assert!(
            qs.iter()
                .flat_map(|q| q.corners.unwrap())
                .flatten()
                .all(|v| v.is_finite())
        );
        assert_eq!(
            snapshot_bits(&ui, id),
            before,
            "draw preserves raw fields: {args}"
        );
    }
}
