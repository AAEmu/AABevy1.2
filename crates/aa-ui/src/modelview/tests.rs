use super::*;
use crate::{UiFrame, UiModelViews, UiRuntime, UiStats, lua, render};
use bevy::prelude::*;

#[derive(Resource, Default)]
struct Host {
    passes: usize,
    token: Option<ModelViewId>,
    image: Handle<Image>,
    seen: Vec<ModelViewOp>,
}

fn host(ui: NonSend<UiRuntime>, mut host: ResMut<Host>) {
    let work = ui.take_modelview_work();
    host.seen.extend(work.commands.into_iter().map(|c| c.op));
    assert_eq!(work.surfaces.len(), 1);
    host.token = Some(work.surfaces[0].view);
    // Callback/reentry resolves Lua and tree state with no toolkit/app-data borrow held.
    ui.exec(
        "@host_reentry",
        "assert(updated);v:SetModelViewCoords(0,0,1,1);v:SetExtent(60,40)",
    )
    .unwrap();
    assert!(ui.set_modelview_image(
        work.surfaces[0].view,
        Some(ModelViewImage {
            image: host.image.clone(),
            extent_px: [1, 1],
        })
    ));
    host.passes += 1;
}

fn test_app() -> App {
    let mut app = App::new();
    app.init_resource::<Time>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<bevy::input::mouse::AccumulatedMouseScroll>()
        .init_resource::<crate::UiVirtualPointer>()
        .init_resource::<UiStats>()
        .init_resource::<render::UiRenderState>()
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<ColorMaterial>>()
        .init_resource::<Assets<render::MaskMaterial>>()
        .init_resource::<Assets<Image>>()
        .init_resource::<Host>();
    let image = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(Image::default());
    app.world_mut().resource_mut::<Host>().image = image;
    app.world_mut().spawn((
        Window {
            resolution: (1920, 1080).into(),
            ..default()
        },
        bevy::window::PrimaryWindow,
    ));
    crate::modelview_frame_sets(&mut app);
    app.add_systems(Update, crate::ui_callbacks.in_set(crate::UiCallbacks))
        .add_systems(Update, host.in_set(UiModelViews))
        .add_systems(Update, crate::ui_paint.in_set(crate::UiPaint));
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    ui.exec(
        "@schedule",
        r#"
        v=UIParent:CreateWidget('modelview','v','UIParent');v:SetExtent(50,50);v:Show(true)
        v:SetModelViewExtent(50,50)
        v:SetHandler('OnMouseUp',function() v:EquipItem(11) end)
        v:SetHandler('OnClick',function() v:ResetEquips() end)
        v:SetHandler('OnUpdate',function(self) updated=true;self:PlayAnimation('callback',true) end)
    "#,
    )
    .unwrap();
    app.world_mut().insert_non_send(ui);
    app
}

#[test]
fn host_seam_follows_full_pointer_release_and_lua_then_paints_this_frames_image() {
    let mut app = test_app();
    // External systems ordered before/after enclosing UiFrame still enclose the host.
    app.add_systems(
        Update,
        (|mut host: ResMut<Host>| {
            assert_eq!(host.passes, 0);
            host.passes = 10;
        })
        .before(UiFrame),
    );
    app.add_systems(
        Update,
        (|host: Res<Host>, stats: Res<UiStats>| {
            assert_eq!(host.passes, 11);
            assert_eq!(stats.quads, 1);
        })
        .after(UiFrame),
    );
    {
        let mut p = app.world_mut().resource_mut::<crate::UiVirtualPointer>();
        p.pos = Some(Vec2::new(10., 10.));
        p.press = true;
    }
    app.update();
    let h = app.world().resource::<Host>();
    assert!(h.seen.contains(&ModelViewOp::PlayAnimation {
        name: "callback".into(),
        looping: true
    }));
    // Use a fresh schedule for the release to avoid first-frame-only enclosing assertions.
    let mut app = test_app();
    {
        let mut p = app.world_mut().resource_mut::<crate::UiVirtualPointer>();
        p.pos = Some(Vec2::new(10., 10.));
        p.press = true;
    }
    app.update();
    app.world_mut().resource_mut::<Host>().seen.clear();
    app.world_mut()
        .resource_mut::<crate::UiVirtualPointer>()
        .release = true;
    app.update();
    assert_eq!(
        app.world().resource::<Host>().seen,
        [
            ModelViewOp::EquipItem(11),
            ModelViewOp::ResetEquips,
            ModelViewOp::PlayAnimation {
                name: "callback".into(),
                looping: true
            }
        ]
    );
    let state = app.world().resource::<render::UiRenderState>();
    assert_eq!(state.quads, 1);
    let ui = app.world().non_send::<UiRuntime>();
    let q = render::build_quads(&mut lua::ui(&ui.lua), None);
    assert_eq!(q[0].pos, [0., 0., 60., 40.]);
    assert_eq!(q[0].uv, [0., 0., 1., 1.]);
    // Same association and actual pixel changes must not rebuild or upload geometry.
    let before = app.world().resource::<UiStats>().rebuilds;
    let uploads = app.world().resource::<UiStats>().mesh_uploads;
    for _ in 0..10 {
        let image = app.world().resource::<Host>().image.clone();
        app.world_mut()
            .resource_mut::<Assets<Image>>()
            .get_mut(&image)
            .unwrap()
            .data
            .as_mut()
            .unwrap()[0] ^= 255;
        app.update();
    }
    assert_eq!(app.world().resource::<UiStats>().rebuilds, before);
    assert_eq!(app.world().resource::<UiStats>().mesh_uploads, uploads);
}
