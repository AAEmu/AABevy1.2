//! Actual controller admission and production player/cancellation/seat/solver systems.
//! Controller rows and active cast/loot state are fixtures; no socket or device.
use super::*;
use crate::hud::api::{HudBridge, HudData};
use crate::ingame::{anim::LocalJump, camera::OrbitCamera, dev::DevOptions, player, seating};
use aa_net::{DoodadBondEvent, NetEvent};
use std::{cell::RefCell, rc::Rc, time::Duration};

#[derive(Resource)]
struct Admit {
    kind: u32,
    caster: bool,
    target: bool,
}

fn admit(
    mut commands: Commands,
    roster: Roster,
    mut tracker: ResMut<Tracker>,
    book: Res<Book>,
    requested: Res<Admit>,
    time: Res<Time>,
    mut once: Local<bool>,
) {
    if *once {
        return;
    }
    *once = true;
    let mut c = Controller {
        kind: requested.kind,
        ..default()
    };
    c.v[0] = 1000;
    c.v[1] = 1000;
    c.v[2] = 1000;
    let mut start = Starter {
        book: &book,
        roster: &roster,
        commands: &mut commands,
        tracker: &mut tracker,
        now: time.elapsed_secs_f64(),
    };
    if c.kind == 1 {
        start.float(&c, 47, 7);
    } else {
        start.controller(
            &c,
            47,
            None,
            requested.target.then_some(Vec3::new(2.0, 2.0, 0.0)),
            requested.caster,
        );
    }
}

fn app() -> (App, Entity, Entity) {
    let (mut app, actor, owner) = seating::fixture_app();
    let terrain = TerrainQuery::new("main_world");
    app.insert_resource(WorldCollision::new("main_world", terrain.clone()))
        .insert_resource(aa_world::WaterQuery::with_terrain(
            "main_world",
            terrain.clone(),
        ))
        .insert_resource(terrain)
        .insert_resource(DevOptions::default())
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            Duration::from_secs_f32(1.0 / 60.0),
        ))
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<crate::settings::Actions>()
        .init_resource::<player::SendState>()
        .init_resource::<player::PlayerInput>()
        .init_resource::<LocalJump>()
        .init_resource::<ClipLibrary>()
        .init_resource::<CameraLag>()
        .init_resource::<Tracker>()
        .init_resource::<Inbox>()
        .insert_resource(Book::open().unwrap())
        .add_message::<NetCommand>();
    let shared = Rc::new(RefCell::new(HudData::default()));
    shared.borrow_mut().loot.last_owner = Some((1, 789));
    app.insert_non_send(HudBridge(shared));
    app.world_mut()
        .spawn((OrbitCamera::default(), GlobalTransform::IDENTITY));
    crate::ingame::combat::fixture_cancel_casts(&mut app, 47);
    app.add_systems(
        Update,
        (
            player::control_player.after(seating::SeatSet::Follow),
            seating::exit_on_input
                .after(crate::ingame::combat::InputCancellation)
                .in_set(seating::SeatSet::Exit),
            drive_local
                .after(seating::SeatSet::Exit)
                .before(crate::ingame::anim::drive_locomotion),
        ),
    );
    (app, actor, owner)
}

fn drain(app: &mut App) -> Vec<Command> {
    app.world_mut()
        .resource_mut::<Messages<NetCommand>>()
        .drain()
        .map(|m| m.0)
        .collect()
}

#[test]
fn admitted_controllers_cancel_then_close_loot_then_unbond_and_hold_until_completion() {
    for (kind, caster) in [(1, true), (2, true), (2, false), (4, true), (4, false)] {
        let (mut app, actor, _) = app();
        app.insert_resource(Admit {
            kind,
            caster,
            target: true,
        })
        .add_systems(
            Update,
            admit
                .after(seating::SeatSet::Follow)
                .before(player::control_player),
        );
        app.update();
        let held = app.world().get::<Mover>(actor).unwrap().pos;
        assert!(
            app.world()
                .get::<SkillMove>(actor)
                .unwrap()
                .controller_activity()
        );
        assert_eq!(app.world().get::<SkillMove>(actor).unwrap().t, 0.0);
        let sent = drain(&mut app);
        assert!(
            matches!(
                &sent[..],
                [
                    Command::StopCasting { tl: 11, plot_tl: 0 },
                    Command::StopCasting { tl: 0, plot_tl: 12 },
                    Command::LootClose {
                        owner_type: 1,
                        owner_obj_id: 789
                    },
                    Command::UnbondDoodad(_),
                ]
            ),
            "kind {kind} caster {caster}: {sent:?}"
        );
        let Command::UnbondDoodad(request) = sent[3] else {
            unreachable!()
        };
        app.update();
        assert!(
            drain(&mut app).is_empty(),
            "no duplicate packets while pending"
        );
        assert_eq!(app.world().get::<Mover>(actor).unwrap().pos, held);
        app.world_mut().write_message(NetEvent(Event::DoodadBond(
            DoodadBondEvent::UnbondRejected {
                request,
                message: "fixture refused write".into(),
            },
        )));
        app.update();
        assert_eq!(app.world().get::<Mover>(actor).unwrap().pos, held);
        assert_eq!(app.world().get::<SkillMove>(actor).unwrap().t, 0.0);
        assert!(
            drain(&mut app)
                .iter()
                .all(|c| matches!(c, Command::UnbondDoodad(_))),
            "retry still has no solver reports"
        );
        app.world_mut().write_message(NetEvent(Event::DoodadBond(
            DoodadBondEvent::UnbondRequested(request),
        )));
        app.update();
        assert!(app.world().get::<seating::Seated>(actor).is_none());
        assert_ne!(app.world().get::<Mover>(actor).unwrap().pos, held);
        assert!(
            drain(&mut app)
                .iter()
                .any(|c| matches!(c, Command::Move(_)))
        );
    }
}

#[test]
fn parented_blink_and_declined_impulse_flight_create_no_controller_exit_or_move_report() {
    for kind in [Kind::Blink, Kind::Knock] {
        let (mut app, actor, _) = app();
        app.update();
        let held = app.world().get::<Mover>(actor).unwrap().pos;
        app.world_mut().entity_mut(actor).insert(Acting {
            until: 100.0,
            layered: false,
        });
        let flight = Flight {
            from: held,
            to: held + Vec3::Y * 8.0,
            dir: Vec2::Y,
            dist: 8.0,
            hs: 8.0,
            vs: 0.0,
            az: 0.0,
            eta: 0.0,
        };
        let movement = SkillMove::new(flight, kind, false);
        assert!(
            !movement.controller_activity(),
            "an enum is not controller admission"
        );
        app.world_mut().entity_mut(actor).insert(movement);
        app.update();
        assert!(app.world().get::<SkillMove>(actor).is_none());
        assert_eq!(app.world().get::<Mover>(actor).unwrap().pos, held);
        assert!(app.world().get::<seating::Seated>(actor).is_some());
        assert!(
            app.world().get::<Acting>(actor).is_some(),
            "real combat pose is preserved"
        );
        assert!(drain(&mut app).is_empty());
    }
}

#[test]
fn logical_bond_without_a_physical_parent_does_not_trigger_blinks_parent_guard() {
    let (mut app, actor, owner) = app();
    app.world_mut()
        .get_mut::<crate::ingame::doodad_visual::DoodadVisual>(owner)
        .unwrap()
        .rendered_func_group_id = 0;
    let from = app.world().get::<Mover>(actor).unwrap().pos;
    let target = from + Vec3::Y * 5.0;
    app.world_mut().entity_mut(actor).insert(SkillMove::new(
        Flight {
            from,
            to: target,
            dir: Vec2::ZERO,
            dist: 0.0,
            hs: 0.0,
            vs: 0.0,
            az: 0.0,
            eta: 0.0,
        },
        Kind::Blink,
        false,
    ));
    app.update();
    assert_eq!(app.world().get::<Mover>(actor).unwrap().pos, target);
    assert!(app.world().get::<seating::Seated>(actor).is_none());
    let sent = drain(&mut app);
    assert!(sent.iter().any(|c| matches!(c, Command::Move(_))));
    assert!(!sent.iter().any(|c| matches!(c, Command::UnbondDoodad(_))));
}

#[test]
fn failed_controller_admission_does_not_invent_activity() {
    for kind in [99, 2] {
        let (mut app, actor, _) = app();
        app.insert_resource(Admit {
            kind,
            caster: true,
            target: false,
        })
        .add_systems(
            Update,
            admit
                .after(seating::SeatSet::Follow)
                .before(player::control_player),
        );
        app.update();
        assert!(app.world().get::<SkillMove>(actor).is_none());
        assert!(drain(&mut app).is_empty());
    }
}

#[test]
fn matched_float_receipt_ends_controller_activity_while_the_free_falling_phase_remains() {
    let (mut app, actor, owner) = app();
    // Unknown terrain deliberately treats the current height as support. Use a
    // real loaded terrain sample so the falling tail has somewhere below to go.
    let (x, y) = (15578.2, 15368.1);
    let floor = app
        .world()
        .resource::<TerrainQuery>()
        .height_at(x, y)
        .unwrap();
    app.world_mut()
        .get_mut::<Transform>(owner)
        .unwrap()
        .translation = Vec3::new(x, floor + 10.0, -y);
    app.insert_resource(Admit {
        kind: 1,
        caster: true,
        target: true,
    })
    .add_systems(
        Update,
        (
            admit
                .after(seating::SeatSet::Follow)
                .before(player::control_player),
            start_moves
                .after(seating::SeatSet::Follow)
                .before(player::control_player),
        ),
    );
    app.update();
    let sent = drain(&mut app);
    let request = sent
        .iter()
        .find_map(|c| match c {
            Command::UnbondDoodad(r) => Some(*r),
            _ => None,
        })
        .unwrap();
    app.world_mut().write_message(NetEvent(Event::DoodadBond(
        DoodadBondEvent::UnbondRequested(request),
    )));
    app.update();
    drain(&mut app);
    observe(
        &mut app.world_mut().resource_mut::<Inbox>(),
        &Event::BuffRemoved {
            obj_id: 47,
            index: 8,
        },
        0.0,
    );
    app.update();
    assert!(
        app.world()
            .get::<SkillMove>(actor)
            .unwrap()
            .controller_activity(),
        "wrong tag cannot end current controller"
    );
    drain(&mut app);
    observe(
        &mut app.world_mut().resource_mut::<Inbox>(),
        &Event::BuffRemoved {
            obj_id: 47,
            index: 7,
        },
        0.0,
    );
    app.update();
    let movement = app.world().get::<SkillMove>(actor).unwrap();
    assert!(movement.hover.unwrap().released);
    assert!(!movement.controller_activity());
    assert!(
        !app.world().resource::<player::PlayerInput>().moving,
        "falling is not native controller B8"
    );
    assert!(
        !drain(&mut app)
            .iter()
            .any(|c| matches!(c, Command::UnbondDoodad(_)))
    );
}
