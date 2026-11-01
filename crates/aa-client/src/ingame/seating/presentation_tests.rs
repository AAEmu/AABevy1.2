use super::super::tests::{active, bond, completion, received, spawn};
use super::*;
use aa_net::model::Doodad;
use aa_net::{Event, NetCommand, NetEvent, Pos};

fn doodad() -> Doodad {
    Doodad {
        obj_id: 123,
        template_id: 1236,
        owner_obj_id: 0,
        parent_obj_id: 0,
        attach_point: 0,
        pos: Pos::default(),
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: 1.0,
        has_loot_item: false,
        func_group_id: 1951,
        owner_id: 0,
        ucc_id: 0,
        item_template_id: 0,
        type2: 0,
        time_left: 0,
        plant_time: 0,
        quest_glow: 0,
        family: 0,
        puzzle_group: 0,
        owner_type: 0,
        owner_db_id: 0,
        data: 0,
    }
}

pub(crate) fn app() -> (App, Entity, Entity) {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_message::<NetEvent>()
        .insert_resource(active(Some(bond())))
        .init_resource::<Views>()
        .init_resource::<Doodads>()
        .init_resource::<Units>()
        .add_systems(
            Update,
            (
                super::super::track,
                follow.in_set(super::super::SeatSet::Follow),
            )
                .chain(),
        );
    app.world_mut()
        .resource_mut::<Doodads>()
        .apply(&Event::DoodadsSpawned(vec![doodad()]));
    let actor = app
        .world_mut()
        .spawn((
            LocalPlayer { obj_id: 47 },
            Transform::from_xyz(8.0, 9.0, 10.0),
            Motion::default(),
            Mover {
                pos: Vec3::new(8.0, -10.0, 9.0),
                facing: 0.0,
                vz: 0.0,
                grounded: true,
                velocity: Vec3::ZERO,
                swim: None,
                wading: 0.0,
            },
        ))
        .id();
    let model = app.world_mut().spawn(Transform::IDENTITY).id();
    let owner = app
        .world_mut()
        .spawn((
            RemoteDoodad { obj_id: 123 },
            Transform::from_translation(Vec3::new(2.0, 0.0, 3.0)),
            DoodadVisual {
                path: "fixture".into(),
                model: Some(model),
                func_group_id: 1951,
                rendered_func_group_id: 1951,
                bond_helpers: [(
                    "bond0".into(),
                    Affine3A::from_translation(Vec3::new(0.0, -1.3, 0.0)),
                )]
                .into(),
                growth: None,
                remaining_ms: 0.0,
                total_ms: 0.0,
                base_scale: 1.0,
            },
        ))
        .id();
    app.world_mut()
        .resource_mut::<Views>()
        .fixture_doodad(123, owner);
    (app, actor, owner)
}

fn receipt(app: &mut App, event: Event) {
    app.world_mut().resource_mut::<Doodads>().apply(&event);
    app.world_mut().write_message(NetEvent(event));
    app.update();
}

#[test]
fn actual_follow_waits_for_current_phase_and_never_borrows_old_helpers() {
    let (mut app, actor, owner) = app();
    let original = *app.world().get::<Transform>(actor).unwrap();
    app.world_mut()
        .get_mut::<DoodadVisual>(owner)
        .unwrap()
        .rendered_func_group_id = 1;
    app.update();
    assert!(app.world().get::<Seated>(actor).is_none());
    assert_eq!(*app.world().get::<Transform>(actor).unwrap(), original);
    app.world_mut()
        .get_mut::<DoodadVisual>(owner)
        .unwrap()
        .rendered_func_group_id = 1951;
    app.update();
    let seated = *app.world().get::<Transform>(actor).unwrap();
    assert!(
        seated
            .translation
            .abs_diff_eq(Vec3::new(2.0, 0.0, 4.3), 1e-5)
    );
    assert_eq!(seated.scale, Vec3::ONE);
    assert!(app.world().get::<ChildOf>(actor).is_none());
    let relative = app.world().get::<Seated>(actor).unwrap().relative;
    app.world_mut()
        .get_mut::<Transform>(owner)
        .unwrap()
        .translation
        .x += 5.0;
    app.update();
    assert_eq!(app.world().get::<Seated>(actor).unwrap().relative, relative);
    assert!((app.world().get::<Transform>(actor).unwrap().translation.x - 7.0).abs() < 1e-5);

    let held = *app.world().get::<Transform>(actor).unwrap();
    app.world_mut()
        .resource_mut::<Doodads>()
        .by_id
        .get_mut(&123)
        .unwrap()
        .func_group_id = 22863;
    app.world_mut()
        .get_mut::<DoodadVisual>(owner)
        .unwrap()
        .func_group_id = 22863;
    app.update();
    assert!(app.world().get::<Seated>(actor).is_none());
    assert_eq!(*app.world().get::<Transform>(actor).unwrap(), held);
    assert!(
        app.world().resource::<Seats>().wanted(47).is_some(),
        "phase is not received unbond"
    );
    {
        let mut visual = app.world_mut().get_mut::<DoodadVisual>(owner).unwrap();
        visual.rendered_func_group_id = 22863;
        visual.bond_helpers.clear();
    }
    app.update();
    assert_eq!(
        app.world().get::<Transform>(actor).unwrap().translation,
        app.world().get::<Transform>(owner).unwrap().translation,
        "ready missing helper uses owner frame"
    );
}

#[test]
fn a_doodad_attached_to_a_unit_seats_in_the_unit_frame() {
    let (mut app, actor, owner) = app();
    // the bond doodad hangs off a vehicle root: its Transform is local to that root
    let vehicle = app
        .world_mut()
        .spawn(GlobalTransform::from(Transform::from_xyz(100.0, 0.0, 50.0)))
        .id();
    app.world_mut().entity_mut(owner).insert(ChildOf(vehicle));
    app.update();
    let seated = app.world().get::<Transform>(actor).unwrap().translation;
    assert!(seated.abs_diff_eq(Vec3::new(102.0, 0.0, 54.3), 1e-5), "seated at {seated:?}");
}

#[test]
fn actual_owner_destruction_preserves_actor_and_pose_without_wire_side_effects() {
    let (mut app, actor, owner) = app();
    app.update();
    let held = *app.world().get::<Transform>(actor).unwrap();
    app.world_mut().despawn(owner);
    receipt(&mut app, Event::DoodadsRemoved(vec![123]));
    assert!(app.world().get_entity(actor).is_ok());
    assert!(app.world().get::<Seated>(actor).is_none());
    assert_eq!(*app.world().get::<Transform>(actor).unwrap(), held);
    assert!(!app.world().resource::<Seats>().own_free());
    assert!(app.world().resource::<Seats>().pending.is_none());
}

#[test]
fn actual_follow_handles_local_completion_and_fresh_equal_receipt_in_order() {
    let (mut app, actor, _) = app();
    app.update();
    let relative = app.world().get::<Seated>(actor).unwrap().relative;
    let request = app
        .world_mut()
        .resource_mut::<Seats>()
        .capture_unbond()
        .unwrap();
    receipt(&mut app, completion(request));
    assert!(app.world().get::<Seated>(actor).is_none());
    for _ in 0..3 {
        app.update();
    }
    assert!(app.world().get::<Seated>(actor).is_none());
    receipt(&mut app, received(bond()));
    let attached = app.world().get::<Seated>(actor).unwrap();
    assert_eq!(attached.relative, relative);
    let current = attached.receipt;
    receipt(&mut app, received(bond()));
    assert_eq!(app.world().get::<Seated>(actor).unwrap().receipt, current);
}

#[test]
fn received_mount_is_a_separate_gate_even_before_local_mount_presentation() {
    let (mut app, actor, _) = app();
    let Event::PlayerSpawn(mut player) = spawn(Some(bond())) else {
        unreachable!()
    };
    player.unit.attached_to = Some(aa_net::model::Attachment {
        obj_id: 999,
        point: 1,
        reason: 0,
    });
    let mut received = aa_net::state::Player::default();
    received.apply(&Event::PlayerSpawn(player));
    app.insert_resource(received);
    app.update();
    assert!(app.world().get::<Seated>(actor).is_none());
    assert!(
        app.world().resource::<Seats>().wanted(47).is_some(),
        "mount never rewrites bond history"
    );
    app.world_mut()
        .resource_mut::<aa_net::state::Player>()
        .apply(&Event::UnitDetached {
            obj_id: 47,
            reason: 1,
        });
    app.update();
    assert!(app.world().get::<Seated>(actor).is_some());
}

#[test]
fn base_seat_sequence_preserves_performance_and_returns_to_loop_before_sit_up() {
    use aa_anim::{
        CharacterDesc,
        db::{GameDb, Gender, Race},
    };
    use std::sync::Arc;
    let db = GameDb::open().unwrap();
    let assets = CharacterDesc::from_preset(&db, Race::Nuian, Gender::Male, 0)
        .unwrap()
        .resolve(&db)
        .unwrap();
    let rig = aa_anim::pose::shared_rig(&assets.skeleton).unwrap();
    let (mut app, actor, _) = app();
    app.init_resource::<ClipLibrary>()
        .add_systems(Update, pose.after(follow));
    app.world_mut().entity_mut(actor).insert((
        Animator::new(rig, Vec::new()),
        Character {
            assets: Arc::new(assets),
            model_root: actor,
            parts: Vec::new(),
        },
    ));
    app.update();
    let animator = app.world().get::<Animator>(actor).unwrap();
    let start = animator.current.as_ref().unwrap().clip.clip.clone();
    assert!(!animator.current.as_ref().unwrap().looping);
    let cycle = animator.then.as_ref().unwrap().clip.clone();
    app.world_mut()
        .get_mut::<Animator>(actor)
        .unwrap()
        .update(2.0);
    assert!(
        app.world()
            .get::<Animator>(actor)
            .unwrap()
            .current
            .as_ref()
            .unwrap()
            .looping
    );
    app.world_mut().entity_mut(actor).insert(Acting {
        until: 1000.0,
        layered: false,
    });
    app.update();
    assert!(app.world().get::<Seated>(actor).is_some());
    assert!(app.world().get::<SeatPose>(actor).unwrap().playing == Pose::Interrupted);
    // An actual queued performance's ownership remains with the existing combat path.
    let before = app
        .world()
        .get::<Animator>(actor)
        .unwrap()
        .current
        .as_ref()
        .unwrap()
        .time;
    app.update();
    assert_eq!(
        app.world()
            .get::<Animator>(actor)
            .unwrap()
            .current
            .as_ref()
            .unwrap()
            .time,
        before
    );
    app.world_mut().entity_mut(actor).remove::<Acting>();
    app.update();
    let animator = app.world().get::<Animator>(actor).unwrap();
    assert!(Arc::ptr_eq(
        &animator.current.as_ref().unwrap().clip.clip,
        &cycle.clip
    ));
    assert!(!Arc::ptr_eq(
        &animator.current.as_ref().unwrap().clip.clip,
        &start
    ));
    let request = app
        .world_mut()
        .resource_mut::<Seats>()
        .capture_unbond()
        .unwrap();
    receipt(&mut app, completion(request));
    assert!(app.world().get::<SeatPose>(actor).unwrap().playing == Pose::End);
    assert!(
        !app.world()
            .get::<Animator>(actor)
            .unwrap()
            .current
            .as_ref()
            .unwrap()
            .looping
    );
    app.world_mut()
        .get_mut::<Animator>(actor)
        .unwrap()
        .update(2.0);
    app.update();
    assert!(app.world().get::<SeatPose>(actor).is_none());
}

#[test]
fn remote_reports_after_the_release_boundary_survive_the_world_pose_handoff() {
    use crate::ingame::units;
    for after in [false, true] {
        let (mut app, actor, _) = app();
        app.world_mut()
            .entity_mut(actor)
            .remove::<(LocalPlayer, Mover)>();
        app.world_mut()
            .entity_mut(actor)
            .insert(RemoteUnit::fixture(47, Vec3::ZERO));
        app.world_mut()
            .resource_mut::<Views>()
            .track_unit_entity(47, actor);
        app.add_systems(
            Update,
            units::receive_moves
                .after(super::super::track)
                .before(follow),
        );
        app.update();
        assert!(app.world().get::<Seated>(actor).is_some());
        let held = bevy_to_cry(app.world().get::<Transform>(actor).unwrap().translation);
        let moved = Vec3::new(20.0, 30.0, 5.0);
        let report = Event::UnitMoved {
            obj_id: 47,
            movement: aa_net::model::Movement::unit(Pos::new(moved.x, moved.y, moved.z), 0.7, 100),
        };
        let release = Event::DoodadBond(aa_net::DoodadBondEvent::Unbonded {
            character_obj_id: 47,
            character_id: 42,
            doodad_obj_id: 123,
        });
        let events = if after {
            [release, report]
        } else {
            [report, release]
        };
        for event in events {
            app.world_mut().write_message(NetEvent(event));
        }
        app.update();
        assert!(app.world().get::<Seated>(actor).is_none());
        assert!(
            app.world()
                .get::<RemoteUnit>(actor)
                .unwrap()
                .latest()
                .abs_diff_eq(if after { moved } else { held }, 1e-5)
        );
    }
}

#[test]
fn seat_retirement_precedes_authoritative_teleport_placement() {
    use crate::ingame::player::{SendState, follow_server};
    let (mut app, actor, _) = app();
    app.init_resource::<SendState>()
        .add_message::<NetCommand>()
        .add_systems(Update, follow_server.after(follow));
    app.update();
    let destination = Pos::new(100.0, 200.0, 30.0);
    for event in [
        Event::DoodadBond(aa_net::DoodadBondEvent::Unbonded {
            character_obj_id: 47,
            character_id: 42,
            doodad_obj_id: 123,
        }),
        Event::Teleported {
            pos: destination,
            yaw: 0.5,
            reason: 0,
        },
    ] {
        app.world_mut().write_message(NetEvent(event));
    }
    app.update();
    assert!(app.world().get::<Seated>(actor).is_none());
    assert_eq!(
        app.world().get::<Transform>(actor).unwrap().translation,
        aa_world::cry_to_bevy(Vec3::new(destination.x, destination.y, destination.z))
    );
    assert_eq!(
        app.world().get::<Mover>(actor).unwrap().pos,
        Vec3::new(100.0, 200.0, 30.0)
    );
}

#[test]
fn stale_loading_input_cannot_send_and_live_exit_closes_loot_before_unbond() {
    use crate::hud::api::{HudBridge, HudData};
    use crate::ingame::{player::PlayerInput, teleport};
    use std::{cell::RefCell, rc::Rc};
    let (mut app, actor, _) = app();
    let shared = Rc::new(RefCell::new(HudData::default()));
    shared.borrow_mut().loot.last_owner = Some((1, 789));
    app.add_plugins(bevy::state::app::StatesPlugin)
        .insert_state(crate::Screen::Loading)
        .init_resource::<teleport::Teleport>()
        .insert_resource(PlayerInput {
            moving: true,
            jumped: false,
            turned: false,
        })
        .insert_non_send(HudBridge(shared))
        .add_message::<NetCommand>()
        .add_systems(
            Update,
            super::super::exit_on_input
                .run_if(in_state(crate::Screen::InGame))
                .run_if(teleport::idle)
                .after(follow),
        );
    app.update();
    app.update();
    assert!(app.world().get::<Seated>(actor).is_some());
    assert!(
        app.world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .next()
            .is_none()
    );
    app.insert_state(crate::Screen::InGame);
    app.update();
    let sent: Vec<_> = app
        .world_mut()
        .resource_mut::<Messages<NetCommand>>()
        .drain()
        .map(|m| m.0)
        .collect();
    assert!(matches!(
        &sent[..],
        [
            aa_net::Command::LootClose {
                owner_type: 1,
                owner_obj_id: 789
            },
            aa_net::Command::UnbondDoodad(_)
        ]
    ));
    assert!(
        app.world().get::<Seated>(actor).is_some(),
        "enqueue cannot detach before completion"
    );
    app.update();
    assert!(
        app.world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .next()
            .is_none(),
        "one outstanding capture"
    );
}

#[test]
fn real_player_input_reports_exit_intent_before_the_seated_motion_lock() {
    use crate::ingame::{anim::LocalJump, camera::OrbitCamera, dev::DevOptions, player};
    for (keys, mouse, moving, turned) in [
        ("W", None, true, true),
        ("A", None, false, true),
        ("J", None, false, false),
        ("", Some(MouseButton::Right), false, true),
        ("", Some(MouseButton::Left), false, false),
    ] {
        let (mut app, actor, _) = app();
        let terrain = aa_world::TerrainQuery::new("main_world");
        app.insert_resource(aa_world::WorldCollision::new("main_world", terrain.clone()))
            .insert_resource(aa_world::WaterQuery::with_terrain(
                "main_world",
                terrain.clone(),
            ))
            .insert_resource(terrain)
            .insert_resource(DevOptions {
                script_keys: keys.into(),
                ..default()
            })
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<crate::settings::Actions>()
            .init_resource::<player::SendState>()
            .init_resource::<player::PlayerInput>()
            .init_resource::<LocalJump>()
            .add_message::<NetCommand>()
            .add_systems(Update, player::control_player.after(follow));
        app.world_mut().spawn(OrbitCamera {
            yaw: std::f32::consts::PI,
            ..default()
        });
        if let Some(mouse) = mouse {
            app.world_mut()
                .resource_mut::<ButtonInput<MouseButton>>()
                .press(mouse);
        }
        app.update();
        let input = app.world().resource::<player::PlayerInput>();
        assert_eq!(
            (input.moving, input.turned, input.jumped),
            (moving, turned, false),
            "{keys} {mouse:?}"
        );
        let transform = app.world().get::<Transform>(actor).unwrap();
        assert!(
            transform
                .translation
                .abs_diff_eq(Vec3::new(2.0, 0.0, 4.3), 1e-5)
        );
        assert_eq!(
            app.world().get::<Mover>(actor).unwrap().velocity,
            Vec3::ZERO
        );
        assert!(
            app.world_mut()
                .resource_mut::<Messages<NetCommand>>()
                .drain()
                .next()
                .is_none(),
            "no walking packets while held"
        );
    }
}


#[test]
fn old_release_does_not_reset_a_same_id_replacement_to_its_first_report() {
    use crate::ingame::units;
    use std::time::Duration;
    let (mut app, actor, _) = app();
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(Duration::from_millis(20)))
        .insert_resource(aa_world::TerrainQuery::new("__no_seating_test_world__"));
    app.world_mut().entity_mut(actor).remove::<(LocalPlayer, Mover)>().insert(RemoteUnit::fixture(47, Vec3::ZERO));
    app.world_mut().resource_mut::<Views>().track_unit_entity(47, actor);
    app.add_systems(Update, (
        units::receive_moves.after(super::super::track).before(units::move_units),
        units::move_units.before(follow),
    ));
    app.update();
    assert!(app.world().get::<Seated>(actor).is_some());
    let mut replacement = crate::ingame::seating::fixture_unit(47,42,None);
    replacement.pos = Pos::new(29.0,0.0,5.0);
    for event in [
        Event::DoodadBond(aa_net::DoodadBondEvent::Unbonded { character_obj_id:47,character_id:42,doodad_obj_id:123 }),
        Event::UnitsRemoved(vec![47]),
        Event::UnitSpawned(Box::new(replacement.clone())),
        Event::UnitMoved { obj_id:47,movement:aa_net::model::Movement::unit(Pos::new(31.0,0.0,5.0),0.0,100) },
    ] { app.world_mut().write_message(NetEvent(event)); }
    app.update();
    let point = app.world().resource::<Views>().unit_point(&replacement).unwrap();
    assert!(point.x < 31.0,"new incarnation must follow normal playback, not consume the old accepted release: {point:?}");
}


#[test]
fn replacement_snapshot_release_is_not_the_new_incarnations_movement_boundary() {
    use crate::ingame::units;
    use std::time::Duration;
    let (mut app, actor, _) = app();
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(Duration::from_millis(20)))
        .insert_resource(aa_world::TerrainQuery::new("__no_seating_test_world__"));
    app.world_mut().entity_mut(actor).remove::<(LocalPlayer, Mover)>().insert(RemoteUnit::fixture(47, Vec3::ZERO));
    app.world_mut().resource_mut::<Views>().track_unit_entity(47, actor);
    app.add_systems(Update, (
        units::receive_moves.after(super::super::track).before(units::move_units),
        units::move_units.before(follow),
    ));
    app.update();
    assert!(app.world().get::<Seated>(actor).is_some());
    let mut replacement = crate::ingame::seating::fixture_unit(47,42,None);
    replacement.model_id = 999;
    replacement.pos = Pos::new(29.0,0.0,5.0);
    app.world_mut().write_message(NetEvent(Event::UnitSpawned(Box::new(replacement.clone()))));
    app.world_mut().write_message(NetEvent(Event::UnitMoved { obj_id:47,
        movement:aa_net::model::Movement::unit(Pos::new(31.0,0.0,5.0),0.0,100) }));
    app.update();
    let point = app.world().resource::<Views>().unit_point(&replacement).unwrap();
    assert!(point.x < 31.0,"release and birth at the same snapshot belong to different incarnations: {point:?}");
}
