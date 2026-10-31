use super::*;
use aa_net::model::{UnitKind, UnitStateAttachments};

fn slave(id: u32, pos: Vec3, yaw: f32) -> Unit {
    let mut unit = super::super::seating::fixture_unit(id, 42, None);
    unit.kind = UnitKind::Slave {
        id: 1,
        tl_id: 1,
        template_id: 1,
        summoner_id: 0,
    };
    unit.pos = aa_net::Pos::new(pos.x, pos.y, pos.z);
    unit.rotation = [0.0, 0.0, yaw];
    unit
}

fn attached(id: u32, parent: u32, local: Vec3, point: u8) -> Unit {
    let mut unit = slave(id, local, 0.0);
    unit.state_attachments = Some(UnitStateAttachments {
        first_point: point,
        first_obj_id: Some(parent),
        bonding_obj_id: None,
    });
    unit
}

#[test]
fn unit_world_composes_the_parent_placement_and_rotates_the_local_offset() {
    let local = Vec3::new(2.0, 3.0, 1.0);
    let mut units = Units::default();
    let mut first = slave(60, Vec3::new(12000.0, 9000.0, 0.0), 0.0);
    // the parent's scale scales the parent-local offset, as `ChildOf` does
    first.scale = 1.5;
    units.by_id.insert(60, first);
    units.by_id.insert(
        61,
        slave(
            61,
            Vec3::new(12000.0, 9000.0, 0.0),
            std::f32::consts::FRAC_PI_2,
        ),
    );
    let first_child = attached(7, 60, local, 3);
    let second_child = attached(8, 61, local, 3);

    let first_world = unit_world(&first_child, &units).expect("the parent is announced");
    let expected = bevy_to_cry(
        placement(Vec3::new(12000.0, 9000.0, 0.0), 0.0, 1.5).transform_point(cry_to_bevy(local)),
    );
    assert!(
        first_world.distance(expected) < 1e-4,
        "{first_world:?} != {expected:?}"
    );

    // a 90 degree yaw turns the local (2, 3) offset into (-3, 2)
    let second_world = unit_world(&second_child, &units).expect("the second parent is announced");
    let expected = bevy_to_cry(
        placement(
            Vec3::new(12000.0, 9000.0, 0.0),
            std::f32::consts::FRAC_PI_2,
            1.0,
        )
        .transform_point(cry_to_bevy(local)),
    );
    assert!(
        second_world.distance(expected) < 1e-4,
        "{second_world:?} != {expected:?}"
    );
    assert!((second_world.x - 11997.0).abs() < 1e-3 && (second_world.y - 9002.0).abs() < 1e-3);
}

#[test]
fn an_attached_unit_keeps_the_packet_rotation_in_the_parent_frame() {
    let mut child = attached(7, 60, Vec3::new(2.0, 3.0, 1.0), 3);
    child.rotation = [0.1, 0.2, 0.3];
    let transform = unit_local_transform(&child, 1.0);
    let expected = cry_rot_to_bevy(Quat::from_euler(EulerRot::ZYX, 0.3, 0.2, 0.1));
    assert!(
        transform.rotation.abs_diff_eq(expected, 1e-6),
        "{:?} != {expected:?}",
        transform.rotation
    );
}

#[test]
fn an_attached_unit_waits_for_its_parent_and_a_detached_state_is_world_again() {
    let mut units = Units::default();
    let mut child = attached(7, 60, Vec3::new(2.0, 3.0, 1.0), 3);
    assert_eq!(
        unit_world(&child, &units),
        None,
        "the parent state has not arrived"
    );
    units
        .by_id
        .insert(60, slave(60, Vec3::new(12000.0, 9000.0, 0.0), 0.0));
    assert!(unit_world(&child, &units).is_some());
    // a later state clears the first attachment: its packet position is a world position again
    child.state_attachments = Some(UnitStateAttachments {
        first_point: u8::MAX,
        first_obj_id: None,
        bonding_obj_id: None,
    });
    child.pos = aa_net::Pos::new(12002.0, 9003.0, 1.0);
    assert_eq!(
        unit_world(&child, &units),
        Some(Vec3::new(12002.0, 9003.0, 1.0))
    );
    // point 0 is no attachment in the 1.2 client (`Unit::AttachTo` ignores it)
    child.state_attachments = Some(UnitStateAttachments {
        first_point: 0,
        first_obj_id: Some(60),
        bonding_obj_id: None,
    });
    assert_eq!(
        unit_world(&child, &units),
        Some(Vec3::new(12002.0, 9003.0, 1.0))
    );
}

#[test]
fn the_census_measures_an_attached_unit_at_its_parent_composed_point() {
    let mut units = Units::default();
    units
        .by_id
        .insert(7, attached(7, 60, Vec3::new(2.0, 3.0, 1.0), 3));
    let views = Views::default();
    let me = Vec3::new(12001.7, 8991.4, 0.0);
    // the child state arrives before the parent: waiting, not far away
    assert_eq!(views.census(&units, me).0, 0);
    assert_eq!(views.census(&units, me).2, 1);
    units
        .by_id
        .insert(60, slave(60, Vec3::new(12000.0, 9000.0, 0.0), 0.0));
    let (far, failed, pending, _) = views.census(&units, me);
    assert_eq!(
        (far, failed),
        (0, 0),
        "the child is at the parent's world point, not at its local offset"
    );
    assert_eq!(pending, 2);
}

#[test]
fn a_parented_unit_root_goes_away_with_its_parent_and_leaves_no_orphan() {
    let mut app = App::new();
    let parent = app.world_mut().spawn_empty().id();
    let child = app
        .world_mut()
        .spawn((RemoteUnit::fixture(7, Vec3::ZERO), ChildOf(parent)))
        .id();
    app.update();
    app.world_mut().entity_mut(parent).despawn();
    app.update();
    assert!(
        app.world().get_entity(child).is_err(),
        "the child root goes with its parent"
    );
    let mut units = app.world_mut().query::<&RemoteUnit>();
    assert_eq!(
        units.iter(app.world()).count(),
        0,
        "no orphan unit root is left behind"
    );
}

#[test]
fn a_parented_unit_follows_the_parent_root_without_being_re_placed() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::transform::TransformPlugin));
    let parent = app
        .world_mut()
        .spawn(Transform::from_xyz(12000.0, 0.0, -9000.0))
        .id();
    let local = Transform::from_xyz(2.0, 1.0, -3.0);
    let child = app.world_mut().spawn((local, ChildOf(parent))).id();
    app.update();
    let before = app
        .world()
        .get::<GlobalTransform>(child)
        .unwrap()
        .translation();
    app.world_mut()
        .get_mut::<Transform>(parent)
        .unwrap()
        .translation += Vec3::new(10.0, 0.0, 0.0);
    app.update();
    let after = app
        .world()
        .get::<GlobalTransform>(child)
        .unwrap()
        .translation();
    assert!(
        (after - before).distance(Vec3::new(10.0, 0.0, 0.0)) < 1e-4,
        "the child did not follow: {before:?} -> {after:?}"
    );
    assert_eq!(
        app.world().get::<Transform>(child).unwrap().translation,
        local.translation,
        "the child is placed once; the hierarchy follows the parent"
    );
}

#[test]
fn a_later_state_that_clears_the_attachment_rebuilds_the_root_in_the_world() {
    let mut app = App::new();
    let mut views = Views::default();
    let child = attached(7, 60, Vec3::new(2.0, 3.0, 1.0), 3);
    views.announce_unit(&mut app.world_mut().commands(), &child);
    app.world_mut().flush();
    let entity = app.world_mut().spawn_empty().id();
    {
        let view = views.units.get_mut(&7).unwrap();
        view.admit(&mut app.world_mut().commands(), entity, &child);
    }
    app.world_mut().flush();
    assert_eq!(
        app.world().get::<RemoteUnit>(entity).unwrap().parent,
        Some(60)
    );

    let mut detached = child.clone();
    detached.state_attachments = Some(UnitStateAttachments {
        first_point: u8::MAX,
        first_obj_id: None,
        bonding_obj_id: None,
    });
    detached.pos = aa_net::Pos::new(12002.0, 9003.0, 1.0);
    {
        let view = views.units.get_mut(&7).unwrap();
        view.reparent(
            &mut app.world_mut().commands(),
            &detached,
            unit_parent(&detached),
        );
    }
    app.world_mut().flush();
    assert!(
        app.world().get_entity(entity).is_err(),
        "the parented root is replaced, not moved as a world root"
    );
    let view = &views.units[&7];
    assert_eq!(view.parent, None);
    let pending = view.pending.as_ref().unwrap();
    assert_eq!(pending.parent, None);
    assert_eq!(pending.pos, Vec3::new(12002.0, 9003.0, 1.0));
    assert_eq!(
        unit_world(&detached, &Units::default()),
        Some(Vec3::new(12002.0, 9003.0, 1.0))
    );
}
