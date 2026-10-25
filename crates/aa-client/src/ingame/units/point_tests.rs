//! Logical actors continue the same movement clock while models are absent.
use super::*;
use std::time::Duration;

fn app() -> App {
    let mut app = App::new();
    app.init_resource::<Time>()
        .init_resource::<Views>()
        .add_message::<NetEvent>()
        .insert_resource(TerrainQuery::new("__no_speech_test_world__"))
        .add_systems(Update, (receive_moves, move_units).chain());
    app
}

fn source(id: u32) -> Unit {
    super::super::seating::fixture_unit(id, 42, None)
}
fn send(app: &mut App, event: Event) {
    app.world_mut().write_message(NetEvent(event));
}
fn step(app: &mut App, dt: f32) {
    app.world_mut().resource_mut::<Time>().advance_by(Duration::from_secs_f32(dt));
    app.update();
}
fn moved(id: u32, x: f32, time: u32) -> Event {
    Event::UnitMoved {
        obj_id: id,
        movement: Movement::unit(aa_net::Pos::new(x, 0.0, 5.0), 0.5, time),
    }
}

#[test]
fn pending_and_rendered_playback_share_each_commit_and_capped_delta() {
    let mut app = app();
    let mut first = source(7);
    first.pos = aa_net::Pos::new(0.0, 0.0, 5.0);
    let mut second = first.clone();
    second.obj_id = 8;
    send(&mut app, Event::UnitSpawned(Box::new(first)));
    send(&mut app, Event::UnitSpawned(Box::new(second)));
    step(&mut app, 0.0);
    let actor = app
        .world_mut()
        .resource_mut::<Views>()
        .units
        .get_mut(&8)
        .unwrap()
        .pending
        .take()
        .unwrap();
    let entity = app.world_mut().spawn((placement(actor.pos, actor.facing, 1.0), actor)).id();
    app.world_mut().resource_mut::<Views>().units.get_mut(&8).unwrap().entity = Some(entity);
    for (i, x) in [0.0, 0.5, 1.0, 1.5, 1.5, 100.0, 100.5].into_iter().enumerate() {
        send(&mut app, moved(7, x, i as u32 * 100));
        send(&mut app, moved(8, x, i as u32 * 100));
        for dt in [0.0, 0.02, 0.04, 0.25] {
            step(&mut app, dt);
            let pending = app.world().resource::<Views>().units[&7].pending.as_ref().unwrap();
            let drawn = app.world().get::<RemoteUnit>(entity).unwrap();
            assert_eq!(
                (pending.pos, pending.facing, pending.combat, pending.track.clock()),
                (drawn.pos, drawn.facing, drawn.combat, drawn.track.clock())
            );
            assert_eq!(app.world().get::<Transform>(entity).unwrap().translation, cry_to_bevy(drawn.pos));
        }
    }
}

#[test]
fn source_refresh_preserves_motion_and_replacement_rejects_old_reports_in_the_same_batch() {
    let mut app = app();
    let mut unit = source(7);
    unit.pos = aa_net::Pos::new(3.0, 0.0, 5.0);
    send(&mut app, Event::UnitSpawned(Box::new(unit.clone())));
    send(&mut app, moved(7, 6.0, 100));
    step(&mut app, 0.1);
    let generation = app.world().resource::<Views>().units[&7].source_generation;
    let latest = app.world().resource::<Views>().units[&7].pending.as_ref().unwrap().latest();
    let mut refresh = unit.clone();
    refresh.pos.x = 900.0;
    send(&mut app, Event::UnitSpawned(Box::new(refresh)));
    step(&mut app, 0.0);
    let view = &app.world().resource::<Views>().units[&7];
    assert_eq!(
        (view.source_generation, view.pending.as_ref().unwrap().latest()),
        (generation, latest)
    );
    send(&mut app, moved(7, 1000.0, 200));
    send(&mut app, Event::UnitsRemoved(vec![7]));
    unit.pos.x = 30.0;
    send(&mut app, Event::UnitSpawned(Box::new(unit.clone())));
    step(&mut app, 0.0);
    let view = &app.world().resource::<Views>().units[&7];
    assert_ne!(view.source_generation, generation);
    assert_eq!(view.pending.as_ref().unwrap().latest().x, 30.0);
    send(&mut app, moved(99, 40.0, 300));
    step(&mut app, 0.1);
    assert!(!app.world().resource::<Views>().units.contains_key(&99));
}

#[test]
fn model_admission_and_eviction_transfer_the_clock_and_committed_world_pose() {
    let mut app = app();
    let mut unit = source(7);
    unit.pos = aa_net::Pos::new(3.0, 4.0, 5.0);
    send(&mut app, Event::UnitSpawned(Box::new(unit)));
    send(&mut app, moved(7, 8.0, 100));
    step(&mut app, 0.1);
    let before = app.world().resource::<Views>().units[&7].pending.as_ref().unwrap().clone();
    let mut view = app.world_mut().resource_mut::<Views>().units.remove(&7).unwrap();
    let entity = app.world_mut().spawn_empty().id();
    view.admit(&mut app.world_mut().commands(), entity, &source(7));
    app.world_mut().flush();
    assert!(view.pending.is_none());
    assert_eq!(app.world().get::<RemoteUnit>(entity).unwrap().track.clock(), before.track.clock());
    let actor = app.world().get::<RemoteUnit>(entity).unwrap().clone();
    let root = placement(Vec3::new(20.0, 30.0, 5.0), 1.0, 1.0);
    view.evict(&mut app.world_mut().commands(), Some((&actor, &root)));
    app.world_mut().flush();
    assert!(view.entity.is_none() && app.world().get_entity(entity).is_err());
    let pending = view.pending.as_ref().unwrap();
    assert_eq!(pending.track.clock(), before.track.clock());
    assert_eq!(pending.pos, Vec3::new(20.0, 30.0, 5.0));
    assert!((pending.facing - 1.0).abs() < 1e-6);
    app.world_mut().resource_mut::<Views>().units.insert(7, view);
}

#[test]
fn failed_models_remain_logical_actors_but_relative_only_movement_never_becomes_world_position() {
    let mut app = app();
    let mut unit = source(7);
    unit.pos = aa_net::Pos::new(3.0, 4.0, 5.0);
    send(&mut app, Event::UnitSpawned(Box::new(unit)));
    step(&mut app, 0.0);
    app.world_mut().resource_mut::<Views>().units.get_mut(&7).unwrap().no_model = true;
    let mut movement = Movement::unit(aa_net::Pos::new(900.0, 0.0, 0.0), 0.0, 100);
    movement.support = Some(aa_net::model::MoveSupport { obj_id: 100, ..default() });
    send(
        &mut app,
        Event::UnitMoved {
            obj_id: 7,
            movement: movement.clone(),
        },
    );
    step(&mut app, 0.1);
    assert_eq!(
        app.world().resource::<Views>().units[&7].pending.as_ref().unwrap().latest(),
        Vec3::new(3.0, 4.0, 5.0)
    );
    movement.support.as_mut().unwrap().world = Some((aa_net::Pos::new(30.0, 0.0, 5.0), [0; 3]));
    send(&mut app, Event::UnitMoved { obj_id: 7, movement });
    step(&mut app, 0.1);
    let view = &app.world().resource::<Views>().units[&7];
    assert!(view.no_model);
    assert_eq!(view.pending.as_ref().unwrap().latest(), Vec3::new(30.0, 0.0, 5.0));
}

#[test]
fn resurrection_commits_identically_at_zero_delta_with_and_without_a_model() {
    let mut app = app();
    let first = source(7);
    let mut second = first.clone();
    second.obj_id = 8;
    send(&mut app, Event::UnitSpawned(Box::new(first)));
    send(&mut app, Event::UnitSpawned(Box::new(second)));
    step(&mut app, 0.0);
    let mut view = app.world_mut().resource_mut::<Views>().units.remove(&8).unwrap();
    let entity = app.world_mut().spawn_empty().id();
    view.admit(&mut app.world_mut().commands(), entity, &source(8));
    app.world_mut().flush();
    app.world_mut().resource_mut::<Views>().units.insert(8, view);
    for id in [7, 8] {
        send(
            &mut app,
            Event::Resurrected {
                obj_id: id,
                pos: aa_net::Pos::new(30.0, 40.0, 50.0),
                yaw: 1.0,
            },
        );
    }
    step(&mut app, 0.0);
    let pending = app.world().resource::<Views>().units[&7].pending.as_ref().unwrap();
    let root = app.world().get::<Transform>(entity).unwrap();
    assert_eq!(root.translation, cry_to_bevy(pending.pos));
    assert_eq!(pending.pos, Vec3::new(30.0, 40.0, 50.0));
}

#[test]
fn rebuilt_models_take_current_scale_and_reapply_their_shadow_policy() {
    let mut app = app();
    let unit = source(7);
    send(&mut app, Event::UnitSpawned(Box::new(unit.clone())));
    step(&mut app, 0.0);
    let mut view = app.world_mut().resource_mut::<Views>().units.remove(&7).unwrap();
    view.pending.as_mut().unwrap().casting = false;
    let mut refreshed = unit;
    refreshed.scale = 2.0;
    let entity = app
        .world_mut()
        .spawn((GlobalTransform::from_translation(Vec3::new(0.0, 0.0, -50.0)), Visibility::Inherited))
        .id();
    let mesh = app.world_mut().spawn((Mesh3d::default(), ChildOf(entity))).id();
    view.admit(&mut app.world_mut().commands(), entity, &refreshed);
    app.world_mut().flush();
    assert_eq!(app.world().get::<Transform>(entity).unwrap().scale, Vec3::splat(2.0));
    assert!(app.world().get::<RemoteUnit>(entity).unwrap().casting);
    app.world_mut().spawn((
        Camera3d::default(),
        GlobalTransform::IDENTITY,
        Projection::Perspective(PerspectiveProjection::default()),
    ));
    app.add_systems(Update, cull_units.after(move_units));
    app.update();
    assert!(
        app.world().get::<NotShadowCaster>(mesh).is_some(),
        "new meshes inherit the actual distant policy rather than a stale cache"
    );
}
