//! Unit tests for the settled ladder rules; the wired controller is covered by `examples/ladder_climb`.
use super::*;

#[test]
fn the_climb_families_follow_the_native_kinds() {
    assert_eq!(Family::of(1), Family::Ladder);
    assert_eq!(Family::of(6), Family::Ladder);
    assert_eq!(Family::of(9), Family::Ladder);
    assert_eq!(Family::of(2), Family::Rope);
    assert_eq!(Family::of(5), Family::Rope);
    assert_eq!(Family::of(3), Family::Wood);
    assert_eq!(Family::of(4), Family::Net);
    assert_eq!(Family::of(7), Family::Mast);
    assert_eq!(Family::of(8), Family::Mast);
    assert_eq!(Family::of(10), Family::Mast);
}

#[test]
fn the_segment_projection_is_the_climb_distance() {
    let a = Vec3::ZERO;
    let b = Vec3::new(0.0, 0.0, 5.0);
    let (d, t, q) = segment_point(Vec3::new(1.0, 0.0, 2.5), a, b);
    assert!((d - 1.0).abs() < 1e-6);
    assert!((t - 0.5).abs() < 1e-6);
    assert!(q.distance(Vec3::new(0.0, 0.0, 2.5)) < 1e-6);
    // past an end it clamps to the end
    let (d, t, q) = segment_point(Vec3::new(0.0, 0.0, 9.0), a, b);
    assert!((d - 4.0).abs() < 1e-6);
    assert_eq!(t, 1.0);
    assert!(q.distance(b) < 1e-6);
}

#[test]
fn the_body_faces_the_ladder_from_the_outward_normal() {
    // ladder to the north (normal points south): facing north (yaw 0)
    assert!(facing_of_normal(Vec3::new(0.0, -1.0, 0.0)).abs() < 1e-6);
    // ladder to the east (normal points west): facing east (yaw -pi/2)
    assert!(
        (facing_of_normal(Vec3::new(-1.0, 0.0, 0.0)) + std::f32::consts::FRAC_PI_2).abs() < 1e-6
    );
    // no horizontal normal: the fallback heading
    assert_eq!(facing_of_normal(Vec3::Z), 0.0);
}

#[test]
fn only_the_relative_angle_kinds_use_the_helper_forward() {
    for kind in [1u8, 4, 5, 8, 9] {
        assert_eq!(relative_angle(kind, 1.234, Vec3::X), 0.0, "kind {kind}");
    }
    // a climber facing the helper's forward has no relative turn
    let facing = facing_of_normal(Vec3::new(0.0, -1.0, 0.0));
    assert!(relative_angle(2, facing, Vec3::Y).abs() < 1e-6);
    // facing the opposite way is half a turn
    let opposite = facing + std::f32::consts::PI;
    assert!((relative_angle(2, opposite, Vec3::Y).abs() - std::f32::consts::PI).abs() < 1e-5);
}

#[test]
fn the_body_sits_off_the_ladder_by_the_capsule_radius() {
    let p = body_point(Vec3::ZERO, Vec3::Z, Vec3::X, 10.0, 0.25);
    assert!(p.distance(Vec3::new(STANDOFF, 0.0, 2.5)) < 1e-6);
    // beyond the ends it clamps
    assert!(body_point(Vec3::ZERO, Vec3::Z, Vec3::X, 10.0, 2.0).z <= 10.0);
    assert!(body_point(Vec3::ZERO, Vec3::Z, Vec3::X, 10.0, -1.0).z >= 0.0);
}

/// The real database's climb rows (needs `AA_DB`): the ladder docs' example doodads.
#[test]
fn the_game_database_gives_the_climb_kinds() {
    let Some(db) = super::Db::open() else {
        eprintln!("no game database: skipped");
        return;
    };
    assert_eq!(db.ladder_kind(15013), Some(1), "doodad 5836's phase");
    assert_eq!(db.ladder_kind(7603), Some(4), "galleon rear ladder's phase");
    assert_eq!(db.ladder_kind(9478), Some(1), "rope ladder 10m's phase");
    assert_eq!(
        db.ladder_kind(1),
        None,
        "an ordinary phase is not climbable"
    );
}

use aa_net::state::{Doodads, Units};
use aa_net::{Command, NetCommand};
use bevy::time::TimeUpdateStrategy;

fn doodad_source(obj_id: u32, parent: u32, attach_point: u8) -> aa_net::model::Doodad {
    aa_net::model::Doodad {
        obj_id,
        template_id: 303,
        owner_obj_id: 0,
        parent_obj_id: parent,
        attach_point,
        pos: Pos::new(0.0, 0.0, 0.0),
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: 1.0,
        has_loot_item: false,
        func_group_id: 15013,
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

fn visual() -> DoodadVisual {
    DoodadVisual {
        path: String::new(),
        model: None,
        func_group_id: 15013,
        rendered_func_group_id: 15013,
        bond_helpers: Default::default(),
        ladder: Some(LadderHelper {
            bottom: Vec3::ZERO,
            top: Vec3::new(0.0, 5.0, 0.0),
            axis: Vec3::Y,
            forward: Vec3::NEG_Z,
        }),
        growth: None,
        remaining_ms: 0.0,
        total_ms: 0.0,
        base_scale: 1.0,
    }
}

fn app(parent: u32) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(crate::settings::Actions::default())
        .insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(100),
        ))
        .insert_resource(Db::open().expect("the game database"))
        .insert_resource(Doodads::default())
        .insert_resource(Units::default())
        .insert_resource(Views::default())
        .insert_resource(SendState::default())
        .insert_resource(PlayerInput::default())
        .insert_resource(aa_anim::ClipLibrary::default())
        .add_message::<NetCommand>()
        .add_systems(Update, control_ladder);
    app.world_mut()
        .resource_mut::<Doodads>()
        .by_id
        .insert(77, doodad_source(77, parent, 24));
    app.world_mut().spawn((
        RemoteDoodad { obj_id: 77 },
        visual(),
        Transform::from_xyz(0.0, 0.0, 0.0),
        GlobalTransform::default(),
    ));
    app.world_mut().spawn((
        LocalPlayer { obj_id: 100 },
        Mover {
            pos: Vec3::new(0.5, 0.0, 0.0),
            facing: 0.0,
            vz: 0.0,
            grounded: true,
            velocity: Vec3::ZERO,
            swim: None,
            wading: 0.0,
        },
        Transform::default(),
        GlobalTransform::default(),
        Motion::default(),
    ));
    app
}

fn sent(app: &mut App) -> Vec<Command> {
    app.world_mut()
        .resource_mut::<Messages<NetCommand>>()
        .drain()
        .map(|v| v.0)
        .collect()
}

fn ladder_of(app: &mut App) -> Option<Ladder> {
    let mut q = app.world_mut().query::<&Ladder>();
    q.iter(app.world()).next().cloned()
}

#[test]
fn f_grabs_the_climb_doodad_and_sends_the_native_hang_block() {
    let Some(_) = Db::open() else {
        eprintln!("no game database: skipped");
        return;
    };
    let mut app = app(0);
    app.world_mut()
        .resource_mut::<crate::settings::Actions>()
        .script_tap("do_interaction_1");
    app.update();
    let ladder = ladder_of(&mut app).expect("the ladder is grabbed");
    assert_eq!((ladder.obj_id, ladder.kind, ladder.parent), (77, 1, 0));
    assert_eq!(ladder.climb_data, aa_net::climb::pack_ladder(0.05, 0.0));
    let commands = sent(&mut app);
    assert!(
        commands
            .iter()
            .any(|c| matches!(c, Command::Hang { target_obj_id: 77 }))
    );
    let hang = commands
        .iter()
        .find_map(|c| {
            if let Command::Move(m) = c {
                Some(m)
            } else {
                None
            }
        })
        .expect("the first hang movement");
    let movement = hang.to_movement(0);
    assert_eq!(
        movement.actor_flags,
        aa_net::model::actor_flags::HANGING_FROM_OBJECT
    );
    assert_eq!(movement.stance, aa_net::model::stance::CROUCH);
    let support = movement.support.expect("the hanging block");
    assert_eq!(
        (support.obj_id, support.climb),
        (1, Some(ladder.climb_data))
    );
}

#[test]
fn climbing_moves_up_and_the_top_exit_waits_half_a_second() {
    let Some(_) = Db::open() else {
        eprintln!("no game database: skipped");
        return;
    };
    let mut app = app(0);
    app.world_mut()
        .resource_mut::<crate::settings::Actions>()
        .script_tap("do_interaction_1");
    app.update();
    let start = ladder_of(&mut app).expect("grabbed").fraction;
    app.world_mut()
        .resource_mut::<crate::settings::Actions>()
        .script_clear_taps();
    app.world_mut()
        .resource_mut::<crate::settings::Actions>()
        .script_hold("moveforward");
    app.update();
    let climbed = ladder_of(&mut app).expect("still hanging").fraction;
    assert!(climbed > start, "{start} -> {climbed}");
    // keep holding until the top: the exit starts and the body stays on the ladder for 0.5 s
    for _ in 0..80 {
        app.update();
        if ladder_of(&mut app).and_then(|l| l.exiting).is_some() {
            break;
        }
    }
    let exiting = ladder_of(&mut app)
        .expect("at the top")
        .exiting
        .expect("the top exit");
    assert!(exiting.top);
    assert!(exiting.left > 0.0);
    // fewer than 0.5 s: still on the ladder
    for _ in 0..4 {
        app.update();
    }
    assert!(ladder_of(&mut app).is_some());
    for _ in 0..3 {
        app.update();
    }
    assert!(ladder_of(&mut app).is_none(), "the exit finished");
    let commands = sent(&mut app);
    assert!(
        commands
            .iter()
            .any(|c| matches!(c, Command::Unhang { reason: 1 }))
    );
    let mut q = app.world_mut().query::<&Mover>();
    let mover = q.single(app.world()).unwrap();
    assert!(mover.pos.z > 5.0, "stepped over the top: {:?}", mover.pos);
}

#[test]
fn the_bottom_stands_again_with_unhang_reason_one() {
    let Some(_) = Db::open() else {
        eprintln!("no game database: skipped");
        return;
    };
    let mut app = app(0);
    app.world_mut()
        .resource_mut::<crate::settings::Actions>()
        .script_tap("do_interaction_1");
    app.update();
    app.world_mut()
        .resource_mut::<crate::settings::Actions>()
        .script_clear_taps();
    app.world_mut()
        .resource_mut::<crate::settings::Actions>()
        .script_hold("moveback");
    app.update();
    assert!(ladder_of(&mut app).is_none());
    assert!(
        sent(&mut app)
            .iter()
            .any(|c| matches!(c, Command::Unhang { reason: 1 }))
    );
    let mut q = app.world_mut().query::<&Mover>();
    assert!(q.single(app.world()).unwrap().grounded);
}

#[test]
fn a_moving_hull_ladder_keeps_the_primary_point_hull_local() {
    let Some(_) = Db::open() else {
        eprintln!("no game database: skipped");
        return;
    };
    let mut app = app(0x1234);
    let mut hull = DeckHullPoses::default();
    hull.0
        .insert(0x1234, (Vec3::new(10.0, 0.0, 0.0), Quat::IDENTITY, 0.0));
    app.insert_resource(hull);
    {
        let mut q = app
            .world_mut()
            .query_filtered::<(&mut Transform, &mut GlobalTransform), With<RemoteDoodad>>();
        let (mut t, mut g) = q.single_mut(app.world_mut()).unwrap();
        *t = Transform::from_translation(aa_world::cry_to_bevy(Vec3::new(10.0, 0.0, 0.0)));
        *g = GlobalTransform::from(*t);
    }
    {
        let mut q = app.world_mut().query::<&mut Mover>();
        q.single_mut(app.world_mut()).unwrap().pos = Vec3::new(10.5, 0.0, 0.0);
    }
    app.world_mut()
        .resource_mut::<crate::settings::Actions>()
        .script_tap("do_interaction_1");
    app.update();
    let ladder = ladder_of(&mut app).expect("grabbed");
    assert_eq!(ladder.parent, 0x1234);
    let first = sent(&mut app)
        .into_iter()
        .find_map(|c| {
            if let Command::Move(m) = c {
                Some(m)
            } else {
                None
            }
        })
        .expect("hang move");
    let first_m = first.to_movement(0);
    let support = first_m.support.expect("both blocks");
    assert_eq!(
        (support.obj_id, support.kind, support.part),
        (0x1234, 1, 24)
    );
    assert_eq!(support.climb, Some(ladder.climb_data));
    assert!(support.world.is_some());
    // Move the hull 10 m north: the primary (hull-local) point stays, the world point moves.
    app.world_mut()
        .resource_mut::<DeckHullPoses>()
        .0
        .insert(0x1234, (Vec3::new(20.0, 0.0, 0.0), Quat::IDENTITY, 0.0));
    {
        let mut q = app
            .world_mut()
            .query_filtered::<(&mut Transform, &mut GlobalTransform), With<RemoteDoodad>>();
        let (mut t, mut g) = q.single_mut(app.world_mut()).unwrap();
        *t = Transform::from_translation(aa_world::cry_to_bevy(Vec3::new(20.0, 0.0, 0.0)));
        *g = GlobalTransform::from(*t);
    }
    app.world_mut()
        .resource_mut::<crate::settings::Actions>()
        .script_clear_taps();
    app.world_mut()
        .resource_mut::<crate::settings::Actions>()
        .script_hold("moveforward");
    app.update();
    let second = sent(&mut app)
        .into_iter()
        .rev()
        .find_map(|c| {
            if let Command::Move(m) = c {
                Some(m)
            } else {
                None
            }
        })
        .expect("climb move");
    let second_m = second.to_movement(0);
    let second_support = second_m.support.expect("both blocks");
    assert!(
        (second_m.pos.x - first_m.pos.x).abs() < 0.02,
        "hull-local x changed: {} -> {}",
        first_m.pos.x,
        second_m.pos.x
    );
    let (world, _) = second_support.world.expect("world backup");
    assert!(
        (world.x - (20.0 + STANDOFF)).abs() < 0.05,
        "world x {}",
        world.x
    );
}

#[test]
fn scripted_actions_hold_and_release() {
    let mut actions = crate::settings::Actions::default();
    actions.script_hold("moveforward");
    assert!(actions.pressed("moveforward", 0));
    actions.script_release("moveforward");
    assert!(!actions.pressed("moveforward", 0));
}
