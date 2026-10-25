//! Actual plate/marker systems on received state, without a renderer or network.
use super::*;
use aa_net::party::PartyEvent;
use bevy::camera::{CameraProjection, ComputedCameraValues, RenderTargetInfo};

pub(super) fn fixture() -> (App, super::super::api::Shared) {
    let mut game = super::super::data::GameData::default();
    for (id, shown) in [(1, false), (2, true)] {
        game.npcs.insert(
            id,
            super::super::data::NpcInfo {
                name: format!("NPC {id}"),
                show_name_tag: shown,
                ..default()
            },
        );
    }
    let mut data = super::super::api::HudData::default();
    data.character_id = 47;
    assert!(data.game.0.set(game).is_ok());
    let bridge = std::rc::Rc::new(std::cell::RefCell::new(data));
    let spawn = crate::hud::expansion::fixture::spawn();
    let mut units = Units::default();
    for (obj_id, template_id) in [(601, 1), (602, 2)] {
        let mut unit = spawn.unit.clone();
        unit.obj_id = obj_id;
        unit.kind = UnitKind::Npc { template_id, owner_id: 0 };
        unit.name.clear();
        unit.pos = aa_net::wire::Pos {
            x: (obj_id - 601) as f32,
            y: 10.0,
            z: 0.0,
        };
        units.apply(&aa_net::Event::UnitSpawned(Box::new(unit)));
    }
    let mut app = App::new();
    app.init_resource::<PlateState>()
        .init_resource::<PlateFont>()
        .init_resource::<super::super::overhead::OverheadState>()
        .init_resource::<Assets<Image>>()
        .init_resource::<Assets<TextureAtlasLayout>>()
        .init_resource::<aa_net::state::Buffs>()
        .init_resource::<aa_net::Appellations>()
        .init_resource::<aa_net::state::Target>()
        .insert_resource(Player { spawn: Some(spawn) })
        .insert_resource(units)
        .insert_non_send(HudBridge(bridge.clone()))
        .add_systems(Update, (update_plates, super::super::overhead::update_overhead).chain());
    let projection = PerspectiveProjection {
        aspect_ratio: 1280.0 / 720.0,
        ..default()
    };
    let camera = Camera {
        computed: ComputedCameraValues {
            clip_from_view: projection.get_clip_from_view(),
            target_info: Some(RenderTargetInfo {
                physical_size: UVec2::new(1280, 720),
                scale_factor: 1.0,
            }),
            ..default()
        },
        ..default()
    };
    app.world_mut().spawn((
        Camera3d::default(),
        camera,
        Projection::Perspective(projection),
        Transform::from_xyz(0.0, 3.0, 0.0).looking_at(Vec3::new(0.0, 1.0, -10.0), Vec3::Y),
    ));
    (app, bridge)
}

fn receive(bridge: &super::super::api::Shared, index: i32, id: u32) {
    bridge.borrow_mut().party.apply(&aa_net::Event::Party(PartyEvent::Marker {
        team_id: 0,
        index,
        kind: 2,
        id,
    }));
}

#[test]
fn received_mark_overrides_hidden_npc_template_and_clear_restores_admission() {
    let (mut app, bridge) = fixture();
    app.update();
    assert!(app.world().resource::<PlateState>().parts(601).is_none());
    assert!(
        app.world().resource::<PlateState>().parts(602).is_some(),
        "ordinary template admission is unchanged"
    );
    receive(&bridge, 3, 601);
    app.update();
    let (root, text) = app
        .world()
        .resource::<PlateState>()
        .parts(601)
        .expect("received marker admits the hidden NPC");
    assert_eq!(app.world().get::<Text2d>(text).unwrap().0, "NPC 1");
    assert_eq!(*app.world().get::<Visibility>(root).unwrap(), Visibility::Inherited);
    let mut marks = app
        .world_mut()
        .query_filtered::<(&ChildOf, &Sprite), With<super::super::overhead::OverheadMark>>();
    let (parent, sprite) = marks.single(app.world()).unwrap();
    assert_eq!(parent.parent(), root);
    assert_eq!(sprite.texture_atlas.as_ref().unwrap().index, 3);
    receive(&bridge, 3, 0);
    app.update();
    assert_eq!(*app.world().get::<Visibility>(root).unwrap(), Visibility::Hidden);
    let mut marks = app
        .world_mut()
        .query_filtered::<Entity, With<super::super::overhead::OverheadMark>>();
    assert_eq!(marks.iter(app.world()).count(), 0);
    receive(&bridge, 3, 601);
    app.update();
    assert_eq!(app.world().resource::<PlateState>().parts(601).unwrap().0, root);
    assert_eq!(*app.world().get::<Visibility>(root).unwrap(), Visibility::Inherited);
}

#[test]
fn received_marker_keeps_the_existing_plate_distance_limit() {
    let (mut app, bridge) = fixture();
    receive(&bridge, 0, 601);
    bridge.borrow_mut().pos = Vec3::new(1000.0, 1000.0, 0.0);
    app.update();
    assert!(
        app.world().resource::<PlateState>().parts(601).is_none(),
        "a mark does not bypass range admission"
    );
    bridge.borrow_mut().pos = Vec3::ZERO;
    app.update();
    let root = app.world().resource::<PlateState>().parts(601).unwrap().0;
    assert_eq!(*app.world().get::<Visibility>(root).unwrap(), Visibility::Inherited);
    bridge.borrow_mut().pos = Vec3::new(1000.0, 1000.0, 0.0);
    app.update();
    assert_eq!(*app.world().get::<Visibility>(root).unwrap(), Visibility::Hidden);
}
