//! Bounded image/command contract. No actor, camera, commerce or network fixture.
use aa_ui::{ModelViewImage, ModelViewOp as Op, UiRuntime, lua, render};
use bevy::prelude::*;

fn eval<T: mlua::FromLuaMulti>(ui: &UiRuntime, code: &str) -> T {
    ui.lua.load(code).eval().expect(code)
}
fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    ui.exec(
        "@view",
        r#"
        p=UIParent:CreateWidget('window','parent','UIParent');p:SetExtent(100,100)
        p:AddAnchor('TOPLEFT',UIParent,100,100);p:Show(true)
        v=p:CreateChildWidget('modelview','view',0,true);v:SetExtent(100,100)
        v:AddAnchor('TOPLEFT',p,0,0)
    "#,
    )
    .unwrap();
    lua::tick(&ui.lua, 0.);
    ui
}
fn image() -> ModelViewImage {
    ModelViewImage {
        image: Handle::default(),
        extent_px: [256, 512],
    }
}
fn quads(ui: &UiRuntime) -> Vec<render::Quad> {
    render::build_quads(&mut lua::ui(&ui.lua), None)
}
fn ops(ui: &UiRuntime) -> Vec<Op> {
    ui.take_modelview_work()
        .commands
        .into_iter()
        .map(|c| c.op)
        .collect()
}

#[test]
fn native_integer_geometry_crop_lifetime_and_actual_extent() {
    let ui = setup();
    let first = ui.take_modelview_work().surfaces.remove(0);
    assert_eq!(first.requested_texture_size, [128, 128]);
    assert_eq!(first.model_extent, [64, 64]);
    assert_eq!(first.source_xywh, [0, 0, 128, 128]);
    assert!(quads(&ui).is_empty());
    ui.exec(
        "@coords_before_extent",
        "v:SetModelViewCoords(9,8,7,6);v:SetTextureSize(0,512);v:SetTextureSize(512,-1)",
    )
    .unwrap();
    assert!(ops(&ui).is_empty());
    assert_eq!(ui.take_modelview_work().surfaces[0], first);
    ui.exec(
        "@integer_geometry",
        r#"
        assert(select('#',v:SetTextureSize(2048,1024))==0)
        v:SetModelViewExtent(270.9,450.1);v:SetModelViewCoords(121.9,31.9,270.9,450.9)
    "#,
    )
    .unwrap();
    let work = ui.take_modelview_work();
    assert_eq!(work.commands[0].op, Op::SetTextureSize([2048, 1024]));
    assert_eq!(work.surfaces[0].requested_texture_size, [2048, 1024]); // no invented caps
    assert_eq!(work.surfaces[0].model_extent, [270, 450]);
    assert_eq!(work.surfaces[0].source_xywh, [121, 31, 270, 450]);
    assert!(ui.set_modelview_image(first.view, Some(image())));
    let q = quads(&ui).remove(0);
    assert_eq!(q.tex, render::Tex::ModelView(first.view));
    assert_eq!(q.uv, [121. / 256., 31. / 512., 391. / 256., 481. / 512.]);
    // Native pairs0/3 anchor the displayed drawable to the widget corners.
    assert_eq!(q.pos, [100., 100., 200., 200.]);
    assert!(!ui.set_modelview_image(
        first.view,
        Some(ModelViewImage {
            extent_px: [0, 512],
            ..image()
        })
    ));
    ui.exec("@mirror", "v:SetModelViewCoords(-2,10,-20,-30)")
        .unwrap();
    assert_eq!(
        quads(&ui)[0].uv,
        [-2. / 256., 10. / 512., -22. / 256., -20. / 512.]
    );
}

#[test]
fn ordered_multiple_widgets_clear_rotation_and_void_returns() {
    let ui = setup();
    let token = ui.take_modelview_work().surfaces[0].view;
    ui.exec(
        "@ordered",
        r#"
        v:SetModelViewExtent(100,100)
        w=p:CreateChildWidget('modelview','second',0,true)
        assert(select('#',v:Init('player',true))==1);assert(v:Init('player',true)==false)
        v:SetIngameShopMode(true);w:EquipItem(7)
        v:AdjustCameraPos(0.1,0.2,0.3);v:AdjustCameraPos(-0.1,0.4,0.5)
        assert(select('#',v:SetRotation(90))==0);v:AddRotation(720)
        assert(select('#',v:GetRotation())==1)
        v:ClearModel();v:Init('player',true);v:ResetEquips()
        v:PlayAnimation('fist_ba_relaxed_rand_idle',true);v:StopAnimation()
        v:SetIngameShopMode(false)
    "#,
    )
    .unwrap();
    let work = ui.take_modelview_work();
    assert_eq!(work.commands.len(), 14);
    let commands: Vec<_> = work.commands.iter().map(|c| c.op.clone()).collect();
    assert_eq!(
        commands[..6],
        [
            Op::Init {
                unit: "player".into(),
                create_model: true
            },
            Op::Init {
                unit: "player".into(),
                create_model: true
            },
            Op::SetIngameShopMode(true),
            Op::EquipItem(7),
            Op::AdjustCameraPos([0.1, 0.2, 0.3]),
            Op::AdjustCameraPos([-0.1, 0.4, 0.5]),
        ]
    );
    assert_ne!(work.commands[3].view, token);
    assert!(
        work.commands
            .iter()
            .enumerate()
            .all(|(i, c)| i == 3 || c.view == token)
    );
    assert_eq!(commands[6], Op::SetRotation(std::f32::consts::FRAC_PI_2));
    let rotation = std::f32::consts::FRAC_PI_2 + 720f32.to_radians();
    assert_eq!(commands[7], Op::SetRotation(rotation));
    assert_eq!(commands[8], Op::ClearModel);
    assert!((eval::<f32>(&ui, "v:GetRotation()") - 810.).abs() < 0.001);
    assert_eq!(ui.take_modelview_work().commands.len(), 0);
    assert_eq!(ui.take_modelview_work().surfaces[0].view, token);
}

#[test]
fn image_withdraw_clear_retire_and_runtime_replacement_reject_stale_ids() {
    let ui = setup();
    ui.exec("@extent", "v:SetModelViewExtent(100,100)").unwrap();
    let token = ui.take_modelview_work().surfaces[0].view;
    assert!(ui.set_modelview_image(token, Some(image())));
    assert_eq!(quads(&ui).len(), 1);
    let content = lua::ui(&ui.lua).tree.settle();
    for _ in 0..100 {
        assert!(ui.set_modelview_image(token, Some(image())));
        assert_eq!(lua::ui(&ui.lua).tree.settle(), content);
    }
    assert!(ui.set_modelview_image(token, None));
    assert!(quads(&ui).is_empty());
    assert!(ui.set_modelview_image(token, Some(image())));
    ui.exec(
        "@clear",
        "v:SetRotation(90);v:ClearModel();v:SetModelViewCoords(8,7,6,5)",
    )
    .unwrap();
    assert!(quads(&ui).is_empty());
    let work = ui.take_modelview_work();
    assert_eq!(work.surfaces[0].view, token);
    assert_eq!(work.surfaces[0].source_xywh, [0, 0, 128, 128]);
    ui.exec("@reinit", "v:Init('player',true)").unwrap();
    assert!(quads(&ui).is_empty());
    assert!(ui.set_modelview_image(token, Some(image())));
    assert_eq!(quads(&ui).len(), 1);
    // Retire through the existing hide/frame lifecycle; stale Lua self is guarded.
    ui.exec("@retire", "p:EnableHidingIsRemove(true);p:Show(false)")
        .unwrap();
    lua::tick(&ui.lua, 0.);
    ui.exec("@stale", "v:EquipItem(99);v:Init('player',true)")
        .unwrap();
    let work = ui.take_modelview_work();
    assert!(work.surfaces.is_empty());
    assert_eq!(work.commands.last().unwrap().op, Op::Destroy);
    assert_eq!(
        work.commands.iter().filter(|c| c.op == Op::Destroy).count(),
        1
    );
    assert!(!ui.set_modelview_image(token, Some(image())));
    assert!(!ui.set_modelview_image(token, None));
    assert!(ui.take_modelview_work().commands.is_empty());
    let next = setup();
    let new_token = next.take_modelview_work().surfaces[0].view;
    assert_ne!(token, new_token);
    assert!(!next.set_modelview_image(token, Some(image())));
    ui.exec(
        "@recreate",
        "v=UIParent:CreateWidget('modelview','view','UIParent')",
    )
    .unwrap();
    assert_ne!(ui.take_modelview_work().surfaces[0].view, token);
}

#[test]
fn parent_clip_fade_scale_layers_and_ordinary_children_keep_draw_order() {
    let ui = setup();
    ui.exec(
        "@composition",
        r#"
        p:EnableScroll(true);p:SetAlpha(0.5)
        v:SetAlpha(0.5);v:RemoveAllAnchors();v:AddAnchor('TOPLEFT',p,50,25)
        before=v:CreateColorDrawable(0.1,0.2,0.3,1,'background');before:SetExtent(20,20)
        v:SetModelViewExtent(100,100);v:SetModelViewCoords(0,0,256,512)
        after=v:CreateColorDrawable(0.2,0.3,0.4,1,'background');after:SetExtent(20,20)
        child=v:CreateChildWidget('emptywidget','child',0,true);child:SetExtent(20,20)
        art=child:CreateColorDrawable(0.3,0.4,0.5,1,'artwork');art:SetExtent(20,20)
        overlay=v:CreateColorDrawable(0.4,0.5,0.6,1,'overlay');overlay:SetExtent(20,20)
    "#,
    )
    .unwrap();
    let token = ui.take_modelview_work().surfaces[0].view;
    ui.set_modelview_image(token, Some(image()));
    let q = quads(&ui);
    assert_eq!(q.len(), 5);
    assert_eq!(q[1].tex, render::Tex::ModelView(token));
    assert_eq!(q[0].color[..3], [0.1, 0.2, 0.3]);
    assert_eq!(q[2].color[..3], [0.2, 0.3, 0.4]);
    assert_eq!(q[3].color[..3], [0.3, 0.4, 0.5]);
    assert_eq!(q[4].color[..3], [0.4, 0.5, 0.6]);
    assert_eq!(q[1].pos, [150., 125., 200., 200.]);
    assert_eq!(q[1].uv, [0., 0., 0.5, 0.75]);
    assert_eq!(q[1].color[3], 0.25);
    ui.exec("@scale", "UIParent:SetUIScale(0.5);p:SetScale(0.5)")
        .unwrap();
    let q = quads(&ui);
    let model = q
        .iter()
        .find(|q| matches!(q.tex, render::Tex::ModelView(_)))
        .unwrap();
    // Compare against transformed inherited clip and verify crop fractions survive scaling.
    let id = lua::value_id(&ui.lua, &eval(&ui, "p")).unwrap();
    let r = {
        let u = lua::ui(&ui.lua);
        u.animations.widget_rect(&u.tree, id)
    };
    assert!(model.pos[2] <= r.right() * 0.5 + 0.001);
    assert!(model.pos[3] <= r.bottom() * 0.5 + 0.001);
    assert_eq!(model.uv, [0., 0., 0.5, 0.74]);
    ui.exec("@fade", "p:Show(false,100)").unwrap();
    lua::tick(&ui.lua, 50.);
    assert!(ui.take_modelview_work().surfaces[0].paint_admitted);
    lua::tick(&ui.lua, 51.);
    assert!(!ui.take_modelview_work().surfaces[0].paint_admitted);
    assert!(quads(&ui).is_empty());
    ui.exec("@zero", "p:Show(true);p:SetAlpha(0)").unwrap();
    lua::tick(&ui.lua, 0.);
    assert!(!ui.take_modelview_work().surfaces[0].paint_admitted);
}

// Load a specific unchanged original ALB prototype with controlled enclosing upvalues.
// These are original instruction streams, not source transcriptions or shop startup.
fn original(ui: &UiRuntime, file: &str, marker: &str) -> mlua::Function {
    use aa_lua::chunk::{self, Chunk, Constant, Format};
    let bytes = aa_pak::Pak::global().read(file).unwrap();
    let chunk = chunk::parse(&bytes).unwrap();
    let mut found = Vec::new();
    chunk.main.walk(&mut |p| {
        if marker.split('+').all(|m| {
            p.constants
                .iter()
                .any(|c| matches!(c,Constant::String(s) if s==m.as_bytes()))
        }) {
            found.push(p.clone());
        }
    });
    assert_eq!(found.len(), 1, "{marker}: {} matches", found.len());
    let bytes = chunk::write(
        &Chunk {
            format: Format::HOST,
            main: found.remove(0),
        },
        Format::HOST,
    )
    .unwrap();
    ui.lua.load(bytes).into_function().unwrap()
}

#[test]
fn unchanged_original_shop_alb_init_equip_reset_hide_and_held_rotation() {
    const FILE: &str = "scriptsbin/x2ui/ingameshop/ingameshop.alb";
    let ui = setup();
    ui.exec("@original_endpoints",r#"
        frame=p;frame.modelView=v;frame.page={currentPage=3}
        v.rotateRight=v:CreateChildWidget('button','rotateRight',0,true)
        v.rotateLeft=v:CreateChildWidget('button','rotateLeft',0,true)
        v.beautyshop=v:CreateChildWidget('button','beautyshop',0,true)
        v.genderTransfer=v:CreateChildWidget('button','genderTransfer',0,true)
        X2Player={GetFeatureSet=function() return {beautyshopBypass=false} end}
        v.reset=v:CreateChildWidget('button','reset',0,true)
        RELAX_ANIMATION_NAME='fist_ba_relaxed_rand_idle';ONEHAND_WEAPON_ANIMATION_NAME='onehand_ba_combat_idle'
        X2Unit={UnitRace=function() return 'nuian' end,UnitGender=function() return 'male' end}
        X2Input={GetMousePos=function() return 40,0 end}
        X2InGameShop={SelectPage=function(_,page) assert(page==3);pageEndpoint=true end}
    "#).unwrap();
    let camera = original(&ui, FILE, "nuian");
    let init = original(&ui, FILE, "SetIngameShopMode");
    ui.lua.globals().set("originalInit", init).unwrap();
    ui.lua.globals().set("originalCamera", camera).unwrap();
    ui.exec(
        "@original_upvalue",
        "debug.setupvalue(originalInit,1,originalCamera);frame.InitModelView=originalInit",
    )
    .unwrap();
    // EquipItem's literal 'weapon' uniquely identifies its body.
    ui.lua
        .globals()
        .set("originalEquip", original(&ui, FILE, "weapon"))
        .unwrap();
    ui.lua
        .globals()
        .set("originalControls", original(&ui, FILE, "RegisterForDrag"))
        .unwrap();
    ui.lua
        .globals()
        .set(
            "originalReset",
            original(&ui, FILE, "ClearModel+SelectPage"),
        )
        .unwrap();
    ui.exec(
        "@original_call",
        r#"
        debug.setupvalue(originalReset,1,frame)
        originalControls(v);frame:InitModelView();originalEquip(frame,100,'weapon')
        originalEquip(frame,200,'armor');originalReset(frame)
    "#,
    )
    .unwrap();
    let calls = ops(&ui);
    assert_eq!(
        calls[..4],
        [
            Op::Init {
                unit: "player".into(),
                create_model: true
            },
            Op::SetIngameShopMode(true),
            Op::AdjustCameraPos([-0.2, 0.4, -0.07]),
            Op::PlayAnimation {
                name: "fist_ba_relaxed_rand_idle".into(),
                looping: true
            }
        ]
    );
    assert_eq!(
        calls[4..10],
        [
            Op::StopAnimation,
            Op::EquipItem(100),
            Op::PlayAnimation {
                name: "onehand_ba_combat_idle".into(),
                looping: true
            },
            Op::StopAnimation,
            Op::EquipItem(200),
            Op::PlayAnimation {
                name: "onehand_ba_combat_idle".into(),
                looping: true
            }
        ]
    );
    assert_eq!(calls[10], Op::ClearModel);
    assert_eq!(calls[11..], calls[..4]);
    let id = lua::value_id(&ui.lua, &eval(&ui, "v")).unwrap();
    ui.exec("@held_right", "v.right=true").unwrap();
    lua::tick(&ui.lua, 100.);
    assert_eq!(ops(&ui), [Op::SetRotation(17f32.to_radians())]);
    lua::call_handler(
        &ui.lua,
        id,
        "OnDragStart",
        vec![mlua::Value::String(
            ui.lua.create_string("RightButton").unwrap(),
        )],
    );
    assert!(!eval::<bool>(&ui, "v.drag or false"));
    ui.exec("@stop", "v.right=false").unwrap();
    let reset = lua::value_id(&ui.lua, &eval(&ui, "v.reset")).unwrap();
    lua::call_handler(&ui.lua, reset, "OnClick", vec![]);
    assert_eq!(
        ops(&ui),
        [
            Op::StopAnimation,
            Op::ResetEquips,
            Op::PlayAnimation {
                name: "fist_ba_relaxed_rand_idle".into(),
                looping: true
            }
        ]
    );
    ui.lua
        .globals()
        .set("originalHide", original(&ui, FILE, "ClearModel+cartFrame"))
        .unwrap();
    ui.exec("@original_hide", "originalHide(frame)").unwrap();
    assert_eq!(ops(&ui), [Op::StopAnimation, Op::ClearModel]);
    assert!(eval::<bool>(&ui, "pageEndpoint"));
    assert!(
        ui.stub_calls()
            .iter()
            .all(|(n, _)| !n.starts_with("modelview:"))
    );
}

#[test]
fn native_rotation_bits_and_class_scope_do_not_claim_customizer_readiness() {
    let ui = setup();
    for (degrees, radians) in [
        (0., 0.),
        (90., f32::from_bits(0x3fc90fdb)),
        (-170., f32::from_bits(0xc03de44e)),
        (720., f32::from_bits(0x41490fdb)),
    ] {
        ui.exec("@angle", &format!("v:SetRotation({degrees})"))
            .unwrap();
        assert_eq!(ops(&ui), [Op::SetRotation(radians)]);
    }
    ui.exec(
        "@scope",
        r#"
        local native=getmetatable(v).__index
        assert(select('#',native.Init(p,'player',true))==0)
        assert(select('#',native.GetRotation(p))==0)
        v:InitBeautyShop();v:SetFov(1);v:EquipCostume(1,2,3)
    "#,
    )
    .unwrap();
    assert!(ops(&ui).is_empty());
    for name in [
        "modelview:InitBeautyShop",
        "modelview:SetFov",
        "modelview:EquipCostume",
    ] {
        assert!(
            ui.stub_calls().iter().any(|(n, c)| n == name && *c == 1),
            "{name}"
        );
    }
}
