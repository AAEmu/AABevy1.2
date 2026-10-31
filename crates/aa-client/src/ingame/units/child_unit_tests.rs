use super::*;
use aa_net::model::{UnitKind, UnitStateAttachments};
use bevy::mesh::skinning::SkinnedMeshInverseBindposes;

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

const SHIP_AT: Vec3 = Vec3::new(12000.0, 9000.0, 0.0);
const LOCAL: Vec3 = Vec3::new(2.0, 3.0, 1.0);
const SHIP_MODEL: u32 = 129;
const CANNON_MODEL: u32 = 117;

fn vehicle_unit(id: u32, pos: Vec3, yaw: f32, model_id: u32) -> Unit {
    let mut unit = slave(id, pos, yaw);
    unit.model_id = model_id;
    unit
}

#[derive(Resource)]
struct SeedChild(u32);

#[derive(Resource, Default)]
struct OrphanSeen(bool);

fn watch_orphans(
    views: Res<Views>,
    entities: Query<(), With<RemoteUnit>>,
    parents: Query<&ChildOf>,
    mut seen: ResMut<OrphanSeen>,
) {
    if let Some(entity) = views.unit_entity(7)
        && entities.get(entity).is_ok()
        && parents.get(entity).is_err()
    {
        seen.0 = true;
    }
}

/// A prepared child build that arrives before the parent root is drawn.
fn seed_child_build(mut views: ResMut<Views>, seed: Res<SeedChild>) {
    if seed.0 == 0 {
        return;
    }
    let generation = {
        let Some(view) = views.units.get_mut(&seed.0) else {
            return;
        };
        if view.entity.is_some() || view.preparing || view.pending.is_none() {
            return;
        }
        view.preparing = true;
        view.source_generation
    };
    views.ready.push(Built::Unit(
        seed.0,
        generation,
        UnitBuild::Effects(Vec::new()),
    ));
}

fn real_app(units: &[Unit], order: &[u32], seed_child: Option<u32>) -> App {
    let db = super::super::data::Db::open().expect("the test needs AA_DB");
    let mut app = App::new();
    app.add_plugins(bevy::transform::TransformPlugin)
        .add_message::<NetEvent>()
        .insert_resource(Time::<()>::default())
        .insert_resource(db)
        .insert_resource(Units::default())
        .insert_resource(Doodads::default())
        .insert_resource(Views::default())
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .init_resource::<Assets<Image>>()
        .init_resource::<Assets<SkinnedMeshInverseBindposes>>()
        .init_resource::<aa_cry::scene::CryCache>()
        .init_resource::<aa_anim::ClipLibrary>()
        .insert_resource(aa_world::TerrainQuery::new("__child_unit_test_world__"))
        .insert_resource(SeedChild(seed_child.unwrap_or(0)))
        .init_resource::<OrphanSeen>()
        .add_systems(
            Update,
            (
                receive_moves,
                seed_child_build,
                sync_views,
                move_units,
                watch_orphans,
            )
                .chain(),
        );
    app.world_mut().spawn((
        super::super::player::LocalPlayer { obj_id: 1 },
        super::super::player::Mover {
            pos: Vec3::new(12001.7, 8991.4, 0.0),
            facing: 0.0,
            vz: 0.0,
            grounded: true,
            velocity: Vec3::ZERO,
            swim: None,
            wading: 0.0,
        },
        Transform::default(),
    ));
    for unit in units {
        app.world_mut()
            .resource_mut::<Units>()
            .by_id
            .insert(unit.obj_id, unit.clone());
    }
    for id in order {
        let unit = units
            .iter()
            .find(|u| u.obj_id == *id)
            .expect("the announced unit");
        app.world_mut()
            .write_message(NetEvent(Event::UnitSpawned(Box::new(unit.clone()))));
    }
    app
}

fn wait_for_units(app: &mut App, count: usize) {
    for _ in 0..600 {
        app.update();
        if app.world().resource::<Views>().unit_entities() >= count {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("the real sync never drew {count} units");
}

#[test]
fn the_real_sync_draws_a_child_under_its_parent_in_both_arrival_orders() {
    if !aa_cry::pak_available() || !aa_anim::GameDb::available() {
        eprintln!("child units: skipped, no game pak or database");
        return;
    }
    for order in [[60u32, 7u32], [7u32, 60u32]] {
        let ship = vehicle_unit(60, SHIP_AT, 0.0, SHIP_MODEL);
        let child = {
            let mut child = attached(7, 60, LOCAL, 3);
            child.model_id = CANNON_MODEL;
            child
        };
        let mut app = real_app(&[ship.clone(), child.clone()], &order, None);
        wait_for_units(&mut app, 2);
        let ship_entity = app
            .world()
            .resource::<Views>()
            .unit_entity(60)
            .expect("the ship root");
        let child_entity = app
            .world()
            .resource::<Views>()
            .unit_entity(7)
            .expect("the child root");
        assert_eq!(
            app.world()
                .get::<ChildOf>(child_entity)
                .map(ChildOf::parent),
            Some(ship_entity),
            "order {order:?}: the child is not ChildOf the ship"
        );
        let expected = placement(SHIP_AT, 0.0, 1.0).transform_point(cry_to_bevy(LOCAL));
        let actual = app
            .world()
            .get::<GlobalTransform>(child_entity)
            .unwrap()
            .translation();
        assert!(
            actual.distance(expected) < 1e-3,
            "order {order:?}: child global {actual:?} != {expected:?}"
        );
        let units = app.world().resource::<Units>();
        assert_eq!(
            unit_world(&child, units).map(|p| bevy_to_cry(expected).distance(p) < 1e-3),
            Some(true),
            "order {order:?}: unit_world differs from the drawn root"
        );
    }
}

#[test]
fn a_prepared_child_waits_for_the_parent_root_before_it_is_spawned() {
    if !aa_cry::pak_available() || !aa_anim::GameDb::available() {
        eprintln!("child units: skipped, no game pak or database");
        return;
    }
    let ship = vehicle_unit(60, SHIP_AT, 0.0, SHIP_MODEL);
    let child = {
        let mut child = attached(7, 60, LOCAL, 3);
        child.model_id = CANNON_MODEL;
        child
    };
    let mut app = real_app(&[ship, child], &[60, 7], Some(7));
    wait_for_units(&mut app, 2);
    let ship_entity = app
        .world()
        .resource::<Views>()
        .unit_entity(60)
        .expect("the ship root");
    let child_entity = app
        .world()
        .resource::<Views>()
        .unit_entity(7)
        .expect("the child root");
    assert_eq!(
        app.world()
            .get::<ChildOf>(child_entity)
            .map(ChildOf::parent),
        Some(ship_entity),
        "the prepared child was spawned before its parent root"
    );
    assert!(
        !app.world().resource::<OrphanSeen>().0,
        "the child was drawn for a frame without its parent root"
    );
}

#[test]
fn an_attached_child_ignores_its_own_movement_reports_and_move_units() {
    if !aa_cry::pak_available() || !aa_anim::GameDb::available() {
        eprintln!("child units: skipped, no game pak or database");
        return;
    }
    let ship = vehicle_unit(60, SHIP_AT, 0.0, SHIP_MODEL);
    let child = {
        let mut child = attached(7, 60, LOCAL, 3);
        child.model_id = CANNON_MODEL;
        child
    };
    let mut app = real_app(&[ship, child], &[60, 7], None);
    wait_for_units(&mut app, 2);
    let child_entity = app
        .world()
        .resource::<Views>()
        .unit_entity(7)
        .expect("the child root");
    let far = Vec3::new(500.0, 500.0, 0.0);
    app.world_mut().write_message(NetEvent(Event::UnitMoved {
        obj_id: 7,
        movement: Movement::unit(aa_net::Pos::new(far.x, far.y, far.z), 0.0, 100),
    }));
    app.update();
    let unit = app.world().get::<RemoteUnit>(child_entity).unwrap();
    assert_eq!(
        unit.latest(),
        LOCAL,
        "receive_moves consumed the child's own report"
    );
    let transform = app.world().get::<Transform>(child_entity).unwrap();
    assert!(
        transform.translation.distance(cry_to_bevy(LOCAL)) < 1e-3,
        "the child root moved to {:?}",
        transform.translation
    );
    app.world_mut()
        .get_mut::<RemoteUnit>(child_entity)
        .unwrap()
        .track = Track::new(far, 0.0);
    app.update();
    let unit = app.world().get::<RemoteUnit>(child_entity).unwrap();
    assert_eq!(unit.pos, LOCAL, "move_units advanced an attached child");
    let transform = app.world().get::<Transform>(child_entity).unwrap();
    assert!(
        transform.translation.distance(cry_to_bevy(LOCAL)) < 1e-3,
        "move_units re-placed the attached child at {:?}",
        transform.translation
    );
}

#[test]
fn a_child_gives_up_when_its_parent_is_known_unbuildable() {
    if !aa_cry::pak_available() || !aa_anim::GameDb::available() {
        eprintln!("child units: skipped, no game pak or database");
        return;
    }
    // model 0 has no actor model: the parent is known unbuildable, so the child must not wait forever
    let ship = vehicle_unit(60, SHIP_AT, 0.0, 0);
    let child = {
        let mut child = attached(7, 60, LOCAL, 3);
        child.model_id = CANNON_MODEL;
        child
    };
    let mut app = real_app(&[ship, child], &[60, 7], None);
    for _ in 0..600 {
        app.update();
        if app
            .world()
            .resource::<Views>()
            .units
            .get(&7)
            .is_some_and(|v| v.no_model)
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let views = app.world().resource::<Views>();
    let parent = views.units.get(&60).expect("the parent view");
    let child = views.units.get(&7).expect("the child view");
    assert!(
        parent.no_model,
        "the model-less parent is not known unbuildable"
    );
    assert!(
        child.no_model,
        "the child still waits for the unbuildable parent"
    );
    assert!(!child.preparing, "the child is still marked preparing");
    assert!(
        child.why.contains("parent"),
        "the reason does not name the parent: {}",
        child.why
    );
}

#[derive(Component)]
struct ParentRoot;
#[derive(Component)]
struct ChildRoot;

#[derive(Resource, Default)]
struct PoseOut {
    accessor: Option<(Vec3, f32)>,
    global: Option<Vec3>,
}

#[derive(Resource, Default)]
struct MoveParent(bool);

fn move_parent(mut moved: ResMut<MoveParent>, mut roots: Query<&mut Transform, With<ParentRoot>>) {
    if !moved.0 {
        return;
    }
    if let Ok(mut transform) = roots.single_mut() {
        transform.translation += Vec3::new(10.0, 0.0, 0.0);
    }
}

fn read_attached_pose(
    views: Res<Views>,
    units: Res<Units>,
    transforms: Query<&Transform>,
    parents: Query<&ChildOf>,
    globals: Query<&GlobalTransform>,
    children: Query<Entity, With<ChildRoot>>,
    mut out: ResMut<PoseOut>,
) {
    out.accessor = units
        .get(7)
        .and_then(|u| unit_world_pose(u, &units, &views, &transforms, &parents));
    out.global = children
        .single()
        .ok()
        .and_then(|e| globals.get(e).ok())
        .map(|g| g.translation());
}

fn pose_app(parent: Transform, local: Transform, parent_id: u32, child_id: u32) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::transform::TransformPlugin))
        .init_resource::<Views>()
        .init_resource::<Units>()
        .init_resource::<PoseOut>()
        .init_resource::<MoveParent>()
        .add_systems(Update, (move_parent, read_attached_pose).chain());
    let parent_entity = app.world_mut().spawn((parent, ParentRoot)).id();
    let child_entity = app
        .world_mut()
        .spawn((local, ChildOf(parent_entity), ChildRoot))
        .id();
    let parent_unit = slave(parent_id, Vec3::new(100.0, 200.0, 5.0), 0.0);
    let child_unit = attached(child_id, parent_id, LOCAL, 3);
    {
        let mut views = app.world_mut().resource_mut::<Views>();
        views.track_unit_entity(parent_id, parent_entity);
        views.track_unit_entity(child_id, child_entity);
    }
    {
        let mut units = app.world_mut().resource_mut::<Units>();
        units.by_id.insert(parent_id, parent_unit);
        units.by_id.insert(child_id, child_unit);
    }
    app
}

#[test]
fn a_drawn_attached_pose_composes_a_yawed_and_pitched_parent() {
    let yaw = std::f32::consts::FRAC_PI_2;
    let pitch = 0.4;
    let parent_rotation = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch);
    let parent = Transform::from_translation(cry_to_bevy(SHIP_AT)).with_rotation(parent_rotation);
    let mut app = pose_app(
        parent,
        Transform::from_translation(cry_to_bevy(LOCAL)),
        60,
        7,
    );
    app.update();
    let (pos, facing) = app
        .world()
        .resource::<PoseOut>()
        .accessor
        .expect("the drawn child pose");
    let expected = bevy_to_cry(parent.transform_point(cry_to_bevy(LOCAL)));
    assert!(pos.distance(expected) < 1e-3, "{pos:?} != {expected:?}");
    assert!(
        (facing - yaw).abs() < 1e-3,
        "the child's facing {facing} does not include the parent's yaw {yaw}"
    );
}

#[test]
fn an_attached_child_pose_includes_the_parent_and_local_yaw() {
    let mut units = Units::default();
    let mut parent = slave(60, SHIP_AT, std::f32::consts::FRAC_PI_2);
    parent.scale = 1.5;
    units.by_id.insert(60, parent);
    let mut child = attached(7, 60, LOCAL, 3);
    child.rotation = [0.0, 0.0, 0.25];
    let (pos, facing) = unit_world_pose_in(&child, &units).expect("the parent is announced");
    let expected = bevy_to_cry(
        placement(SHIP_AT, std::f32::consts::FRAC_PI_2, 1.5).transform_point(cry_to_bevy(LOCAL)),
    );
    assert!(pos.distance(expected) < 1e-3, "{pos:?} != {expected:?}");
    assert!(
        (facing - (std::f32::consts::FRAC_PI_2 + 0.25)).abs() < 1e-3,
        "facing {facing} does not compose the parent's yaw and the child's local yaw"
    );
}

#[test]
fn a_drawn_attached_pose_reads_a_parent_moved_in_the_same_update() {
    let parent =
        Transform::from_translation(cry_to_bevy(SHIP_AT)).with_rotation(Quat::from_rotation_y(0.3));
    let mut app = pose_app(
        parent,
        Transform::from_translation(cry_to_bevy(LOCAL)),
        60,
        7,
    );
    app.update();
    let before = app
        .world()
        .resource::<PoseOut>()
        .accessor
        .expect("the first pose");
    assert!(
        app.world().resource::<PoseOut>().global.is_some(),
        "the child has a GlobalTransform"
    );
    app.world_mut().resource_mut::<MoveParent>().0 = true;
    app.update();
    let out = app.world().resource::<PoseOut>();
    let (pos, facing) = out.accessor.expect("the new pose");
    let moved = Transform::from_translation(cry_to_bevy(SHIP_AT) + Vec3::new(10.0, 0.0, 0.0))
        .with_rotation(Quat::from_rotation_y(0.3));
    let expected = bevy_to_cry(moved.transform_point(cry_to_bevy(LOCAL)));
    assert!(
        pos.distance(expected) < 1e-3,
        "the accessor lags: {pos:?} != {expected:?}"
    );
    assert!((facing - 0.3).abs() < 1e-3);
    assert!(pos.distance(before.0) > 1.0, "the parent did not move");
    let global = out.global.expect("the child's global");
    assert!(
        global.distance(moved.transform_point(cry_to_bevy(LOCAL))) > 1.0,
        "GlobalTransform was already propagated inside Update; the staleness check cannot prove the lag"
    );
}

#[test]
fn reparent_clears_a_model_give_up() {
    let mut app = App::new();
    let mut views = Views::default();
    let mut unit = attached(7, 60, LOCAL, 3);
    views.announce_unit(&mut app.world_mut().commands(), &unit);
    app.world_mut().flush();
    {
        let view = views.units.get_mut(&7).unwrap();
        view.no_model = true;
        view.gave_up_on_parent = true;
        view.why = "parent 60 has no model".into();
    }
    {
        let view = views.units.get_mut(&7).unwrap();
        view.parent = Some(60);
        unit.state_attachments = Some(UnitStateAttachments {
            first_point: 3,
            first_obj_id: Some(61),
            bonding_obj_id: None,
        });
        view.reparent(&mut app.world_mut().commands(), &unit, Some(61));
        assert!(!view.no_model, "a reparent keeps the old model give-up");
        assert!(!view.gave_up_on_parent);
        assert!(view.why.is_empty());
    }
}

#[derive(Resource, Default)]
struct SeedBuilds(Vec<u32>);

fn seed_builds(mut views: ResMut<Views>, seed: Res<SeedBuilds>) {
    for id in &seed.0 {
        let Some(view) = views.units.get_mut(id) else {
            continue;
        };
        if view.entity.is_some() || view.preparing || view.pending.is_none() {
            continue;
        }
        view.preparing = true;
        let generation = view.source_generation;
        views
            .ready
            .push(Built::Unit(*id, generation, UnitBuild::Effects(Vec::new())));
    }
}

fn fixture_app(units: &[Unit], order: &[u32]) -> App {
    let db = super::super::data::Db::ship_fixture(SHIP_MODEL, "prefab://test/none")
        .expect("the sqlite fixture database");
    let mut app = App::new();
    app.add_plugins(bevy::transform::TransformPlugin)
        .add_message::<NetEvent>()
        .insert_resource(Time::<()>::default())
        .insert_resource(db)
        .insert_resource(Units::default())
        .insert_resource(Doodads::default())
        .insert_resource(Views::default())
        .insert_resource(SeedBuilds(units.iter().map(|u| u.obj_id).collect()))
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .init_resource::<Assets<Image>>()
        .init_resource::<Assets<SkinnedMeshInverseBindposes>>()
        .init_resource::<aa_cry::scene::CryCache>()
        .init_resource::<aa_anim::ClipLibrary>()
        .insert_resource(aa_world::TerrainQuery::new("__child_unit_fixture_world__"))
        .add_systems(
            Update,
            (receive_moves, seed_builds, sync_views, move_units).chain(),
        );
    app.world_mut().spawn((
        super::super::player::LocalPlayer { obj_id: 1 },
        super::super::player::Mover {
            pos: Vec3::new(12001.7, 8991.4, 0.0),
            facing: 0.0,
            vz: 0.0,
            grounded: true,
            velocity: Vec3::ZERO,
            swim: None,
            wading: 0.0,
        },
        Transform::default(),
    ));
    for unit in units {
        app.world_mut()
            .resource_mut::<Units>()
            .by_id
            .insert(unit.obj_id, unit.clone());
    }
    for id in order {
        let unit = units
            .iter()
            .find(|u| u.obj_id == *id)
            .expect("the announced unit");
        app.world_mut()
            .write_message(NetEvent(Event::UnitSpawned(Box::new(unit.clone()))));
    }
    app
}

#[test]
fn the_sync_draws_a_child_under_its_parent_without_the_game_pak() {
    let ship = vehicle_unit(60, SHIP_AT, 0.0, SHIP_MODEL);
    let child = {
        let mut child = attached(7, 60, LOCAL, 3);
        child.model_id = CANNON_MODEL;
        child
    };
    let mut app = fixture_app(&[ship, child], &[60, 7]);
    wait_for_units(&mut app, 2);
    let ship_entity = app
        .world()
        .resource::<Views>()
        .unit_entity(60)
        .expect("the ship root");
    let child_entity = app
        .world()
        .resource::<Views>()
        .unit_entity(7)
        .expect("the child root");
    assert_eq!(
        app.world()
            .get::<ChildOf>(child_entity)
            .map(ChildOf::parent),
        Some(ship_entity),
        "the fixture child is not ChildOf the ship"
    );
    let expected = placement(SHIP_AT, 0.0, 1.0).transform_point(cry_to_bevy(LOCAL));
    let actual = app
        .world()
        .get::<GlobalTransform>(child_entity)
        .unwrap()
        .translation();
    assert!(
        actual.distance(expected) < 1e-3,
        "child global {actual:?} != {expected:?}"
    );
}

#[test]
fn a_reannounced_parent_clears_the_childs_give_up() {
    let ship = vehicle_unit(60, SHIP_AT, 0.0, SHIP_MODEL);
    let child = attached(7, 60, LOCAL, 3);
    let mut app = fixture_app(&[ship, child], &[60, 7]);
    wait_for_units(&mut app, 2);
    {
        let mut views = app.world_mut().resource_mut::<Views>();
        let view = views.units.get_mut(&7).unwrap();
        view.no_model = true;
        view.gave_up_on_parent = true;
        view.why = "parent 60 has no model".into();
    }
    let mut parent = vehicle_unit(60, SHIP_AT, 0.0, SHIP_MODEL + 1);
    parent.name = "the re-announced ship".into();
    app.world_mut()
        .resource_mut::<Units>()
        .by_id
        .insert(60, parent.clone());
    app.world_mut()
        .write_message(NetEvent(Event::UnitSpawned(Box::new(parent))));
    app.update();
    let views = app.world().resource::<Views>();
    let view = views.units.get(&7).expect("the child view");
    assert!(
        !view.gave_up_on_parent,
        "the re-announced parent did not clear the give-up"
    );
    assert!(
        !view.no_model,
        "the child still refuses to build after the parent came back"
    );
}
