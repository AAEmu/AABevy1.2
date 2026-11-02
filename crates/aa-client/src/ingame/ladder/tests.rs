//! Ladder tests: the settled rules and the wired controller (a minimal fixture with no game
//! database); `examples/ladder_climb` runs the drawn path.
use super::*;
use aa_world::coords::cry_rot_to_bevy;

#[test]
fn the_climb_families_follow_the_native_kinds() {
    // 1, 6, 9, 10 and the net (4) use the ladder clips (the net's states point at them)
    for kind in [1u8, 4, 6, 9, 10] {
        assert_eq!(Family::of(kind), Family::Ladder, "kind {kind}");
    }
    assert_eq!(Family::of(2), Family::Rope);
    assert_eq!(Family::of(3), Family::Wood);
    assert_eq!(Family::of(5), Family::Hrope);
    assert_eq!(Family::of(7), Family::Mast);
    assert_eq!(Family::of(8), Family::Guardrail);
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

#[test]
fn the_bottom_exit_stands_swims_or_falls() {
    let at = Vec3::new(0.0, 0.0, 0.3);
    let wet = |surface: f32| Wet {
        surface,
        kind: WaterKind::Sea,
        depth: 5.0,
    };
    assert_eq!(bottom_rest(at, Some(0.0), None, 1.3), Rest::Stand(0.0));
    // a point far above the ground falls instead of reporting grounded
    assert_eq!(
        bottom_rest(Vec3::new(0.0, 0.0, 5.0), Some(0.0), None, 1.3),
        Rest::Fall
    );
    // deep water at the feet starts swimming, even over a far ground
    assert_eq!(
        bottom_rest(at, Some(-5.0), Some(wet(0.4)), 1.3),
        Rest::Swim {
            surface: 0.4,
            kind: WaterKind::Sea
        }
    );
    // shallow water is not a swim: the ground stands
    let mut shallow = wet(0.4);
    shallow.depth = 0.2;
    assert_eq!(
        bottom_rest(at, Some(0.0), Some(shallow), 1.3),
        Rest::Stand(0.0)
    );
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

/// The player model's climb clips follow the graph (needs `AA_PAK`).
#[test]
fn the_player_climb_clips_follow_the_graph() {
    if !aa_cry::pak_available() {
        eprintln!("no pak: skipped");
        return;
    }
    let rig = aa_anim::pose::shared_rig("objects/characters/nuian/male/nude/nu_m_base.chr")
        .expect("the player skeleton");
    let cal = aa_anim::Cal::shared("objects/characters/nuian/male/nude/nu_m_base.cal")
        .expect("the player .cal");
    let clips = aa_anim::ClipLibrary::default();
    let set = LadderClips::load(&rig, &cal, &clips).expect("the climb clips");
    let name = |clip: Option<&aa_anim::pose::BoundClip>| {
        clip.and_then(|c| c.source.as_deref())
            .unwrap_or("")
            .rsplit('/')
            .next()
            .unwrap_or("")
            .to_string()
    };
    // kinds 1/4/6/9/10 use the ladder clips (the net's states play them)
    assert_eq!(name(set.up(4)), "fist_mo_ladder_up.caf");
    assert_eq!(name(set.idle(4)), "fist_mo_ladder_idle.caf");
    assert_eq!(name(set.down(4)), "fist_mo_ladder_down.caf");
    // the rope and wood states have one clip each
    assert_eq!(name(set.up(2)), "fist_mo_rope_up.caf");
    // `fist_mo_wood_up` is a cal alias of the ladder up clip
    assert_eq!(name(set.up(3)), "fist_mo_ladder_up.caf");
    // the guardrail is the climb family, not the mast
    assert_eq!(name(set.up(8)), "fist_mo_climb_up.caf");
    assert_eq!(name(set.idle(8)), "fist_mo_climb_idle.caf");
    assert_eq!(name(set.up(7)), "fist_mo_mast_up.caf");
    // exits: the net's two angle classes, the rope end, the guardrail's climb state
    assert_eq!(name(set.exit(4, 0.5)), "fist_mo_ladder_end.caf");
    assert_eq!(name(set.exit(4, 10.0)), "fist_mo_ladder_end_80.caf");
    assert_eq!(name(set.exit(4, 25.0)), "");
    assert_eq!(name(set.exit(2, 0.0)), "fist_mo_rope_end.caf");
    assert_eq!(name(set.exit(8, 0.0)), "fist_mo_climb_up.caf");
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

fn helper() -> LadderHelper {
    LadderHelper {
        bottom: Vec3::ZERO,
        top: Vec3::new(0.0, 5.0, 0.0),
        axis: Vec3::Y,
        forward: Vec3::NEG_Z,
    }
}

fn visual() -> DoodadVisual {
    DoodadVisual {
        path: String::new(),
        model: None,
        func_group_id: 15013,
        rendered_func_group_id: 15013,
        bond_helpers: Default::default(),
        ladder: Some(helper()),
        growth: None,
        remaining_ms: 0.0,
        total_ms: 0.0,
        base_scale: 1.0,
    }
}

/// A wired controller with a minimal climb fixture (no game database needed).
fn app_with(kind: u8, parent: u32) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(crate::settings::Actions::default())
        .insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(100),
        ))
        .insert_resource(Db::ladder_fixture(15013, kind).expect("the ladder fixture"))
        .insert_resource(Doodads::default())
        .insert_resource(Units::default())
        .insert_resource(Views::default())
        .insert_resource(LadderClaim::default())
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

fn app(parent: u32) -> App {
    app_with(1, parent)
}

fn player_entity(app: &mut App) -> Entity {
    let mut q = app
        .world_mut()
        .query_filtered::<Entity, With<LocalPlayer>>();
    q.single(app.world()).expect("the local player")
}

fn tap(app: &mut App, action: &str) {
    app.world_mut()
        .resource_mut::<crate::settings::Actions>()
        .script_tap(action);
}

fn clear_taps(app: &mut App) {
    app.world_mut()
        .resource_mut::<crate::settings::Actions>()
        .script_clear_taps();
}

fn hold(app: &mut App, action: &str) {
    app.world_mut()
        .resource_mut::<crate::settings::Actions>()
        .script_hold(action);
}

fn release(app: &mut App, action: &str) {
    app.world_mut()
        .resource_mut::<crate::settings::Actions>()
        .script_release(action);
}

fn sent(app: &mut App) -> Vec<Command> {
    app.world_mut()
        .resource_mut::<Messages<NetCommand>>()
        .drain()
        .map(|v| v.0)
        .collect()
}

fn moves(commands: &[Command]) -> Vec<&MoveCommand> {
    commands
        .iter()
        .filter_map(|c| {
            if let Command::Move(m) = c {
                Some(m)
            } else {
                None
            }
        })
        .collect()
}

fn ladder_of(app: &mut App) -> Option<Ladder> {
    let mut q = app.world_mut().query::<&Ladder>();
    q.iter(app.world()).next().cloned()
}

#[test]
fn f_grabs_the_climb_doodad_and_sends_the_native_hang_block() {
    let mut app = app(0);
    tap(&mut app, "do_interaction_1");
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
    let hang = moves(&commands)
        .into_iter()
        .next()
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
    assert_eq!(
        app.world().resource::<LadderClaim>().0,
        Some(77),
        "F is claimed"
    );
}

#[test]
fn climbing_moves_up_and_the_top_exit_waits_half_a_second() {
    let mut app = app(0);
    tap(&mut app, "do_interaction_1");
    app.update();
    let start = ladder_of(&mut app).expect("grabbed").fraction;
    clear_taps(&mut app);
    hold(&mut app, "moveforward");
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
fn stopping_sends_one_moving_false_packet_then_the_native_keep_alive() {
    let mut app = app(0);
    tap(&mut app, "do_interaction_1");
    app.update();
    clear_taps(&mut app);
    hold(&mut app, "moveforward");
    app.update();
    let climbing = sent(&mut app);
    let moving = moves(&climbing);
    assert!(
        moving.iter().any(|m| m.moving),
        "the climb movement is moving"
    );
    // let go: one stop packet, at once
    release(&mut app, "moveforward");
    app.update();
    let stop = sent(&mut app);
    let stop_moves = moves(&stop);
    assert_eq!(stop_moves.len(), 1, "one stop packet: {stop_moves:?}");
    assert!(!stop_moves[0].moving, "the stop packet says moving: false");
    assert_eq!(stop_moves[0].velocity[2], 0, "and carries no climb speed");
    // no second stop packet while the clock runs
    app.update();
    assert!(moves(&sent(&mut app)).is_empty(), "no duplicate stop");
    // 3 s later the native forced-max idle movement (DAT_3a59f850, 3000 ms)
    for _ in 0..31 {
        app.update();
    }
    let idle = sent(&mut app);
    let idle_moves = moves(&idle);
    assert_eq!(idle_moves.len(), 1, "one keep-alive: {idle_moves:?}");
    assert!(!idle_moves[0].moving, "the keep-alive is an idle movement");
}

#[test]
fn the_stop_packet_waits_for_the_native_minimum() {
    // `DAT_3a59f84c` (100 ms) is the least time between hanging packets: a climb that stops
    // right after a moving packet does not send the stop before it.
    let mut state = SendState::default();
    state.ladder_sent(true);
    state.ladder_clock(0.05);
    assert!(!state.ladder_due(false), "no stop inside 100 ms");
    state.ladder_clock(0.05);
    assert!(state.ladder_due(false), "the stop is due at 100 ms");
}

#[test]
fn a_moving_hull_ladder_keeps_the_primary_point_hull_local() {
    let mut app = app(0x1234);
    let mut hull = DeckHullPoses::default();
    hull.0
        .insert(0x1234, (Vec3::new(10.0, 0.0, 0.0), Quat::IDENTITY, 0.0));
    app.insert_resource(hull);
    // The hull is a parent entity and only its Transform moves; the GlobalTransforms stay stale
    // (MinimalPlugins does not propagate them), so a reader of GlobalTransform would place the
    // doodad at the origin and refuse the grab.
    let hull_entity = app
        .world_mut()
        .spawn((
            Transform::from_translation(aa_world::cry_to_bevy(Vec3::new(10.0, 0.0, 0.0))),
            GlobalTransform::default(),
        ))
        .id();
    {
        let doodad = app
            .world_mut()
            .query_filtered::<Entity, With<RemoteDoodad>>()
            .single(app.world())
            .unwrap();
        app.world_mut().entity_mut(doodad).insert((
            ChildOf(hull_entity),
            Transform::default(),
            GlobalTransform::default(),
        ));
    }
    {
        let mut q = app.world_mut().query::<&mut Mover>();
        q.single_mut(app.world_mut()).unwrap().pos = Vec3::new(10.5, 0.0, 0.0);
    }
    tap(&mut app, "do_interaction_1");
    app.update();
    let ladder = ladder_of(&mut app).expect("grabbed through this frame's ChildOf chain");
    assert_eq!(ladder.parent, 0x1234);
    let first_commands = sent(&mut app);
    let first = moves(&first_commands)
        .into_iter()
        .next()
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
    app.world_mut()
        .entity_mut(hull_entity)
        .insert(Transform::from_translation(aa_world::cry_to_bevy(
            Vec3::new(20.0, 0.0, 0.0),
        )));
    clear_taps(&mut app);
    hold(&mut app, "moveforward");
    app.update();
    let second_commands = sent(&mut app);
    let second = moves(&second_commands)
        .into_iter()
        .last()
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
fn a_rotated_hull_sends_the_hull_relative_facing() {
    let mut app = app(0x1234);
    let hull_yaw = std::f32::consts::FRAC_PI_2;
    let rotation = Quat::from_rotation_z(hull_yaw);
    let root = Vec3::new(10.0, 0.0, 0.0);
    let mut hull = DeckHullPoses::default();
    hull.0.insert(0x1234, (root, rotation, hull_yaw));
    app.insert_resource(hull);
    {
        let mut q = app
            .world_mut()
            .query_filtered::<(&mut Transform, &mut GlobalTransform), With<RemoteDoodad>>();
        let (mut t, mut g) = q.single_mut(app.world_mut()).unwrap();
        *t = Transform {
            translation: aa_world::cry_to_bevy(root),
            rotation: cry_rot_to_bevy(rotation),
            scale: Vec3::ONE,
        };
        *g = GlobalTransform::from(*t);
    }
    {
        let mut q = app.world_mut().query::<&mut Mover>();
        q.single_mut(app.world_mut()).unwrap().pos = Vec3::new(10.5, 0.0, 0.0);
    }
    tap(&mut app, "do_interaction_1");
    app.update();
    let ladder = ladder_of(&mut app).expect("grabbed");
    // the ladder is east of the hull's centre, the body faces east (pi/2 in the world)
    assert!((ladder.fraction - 0.01).abs() < 1e-3, "{ladder:?}");
    let first_commands = sent(&mut app);
    let first = moves(&first_commands)
        .into_iter()
        .next()
        .expect("hang move");
    let movement = first.to_movement(0);
    // the primary point is hull-local, so its direction byte is the facing in the hull frame
    assert!(
        movement.yaw().abs() < 0.02,
        "relative yaw {} (world facing would be {hull_yaw})",
        movement.yaw()
    );
    let support = movement.support.expect("both blocks");
    let (_, direction) = support.world.expect("world backup");
    assert_eq!(direction[2], radians_to_direction(hull_yaw), "world facing");
}

#[test]
fn a_mount_seat_glider_or_skill_refuses_the_grab() {
    // riding a mount
    let mut mounted = app(0);
    mounted.insert_resource(super::super::mount::LocalMount(Some(
        super::super::mount::LocalSeat {
            mount: 5,
            point: 1,
            drives: true,
            cart: false,
            ship: false,
            turret: false,
            speed: Default::default(),
            lift: 0.0,
        },
    )));
    tap(&mut mounted, "do_interaction_1");
    mounted.update();
    assert!(ladder_of(&mut mounted).is_none(), "mounted");
    assert!(
        !sent(&mut mounted)
            .iter()
            .any(|c| matches!(c, Command::Hang { .. })),
        "no CSHang while mounted"
    );
    assert_eq!(
        mounted.world().resource::<LadderClaim>().0,
        None,
        "a refused grab does not take F from talk or loot"
    );

    // seated
    let mut seated = app(0);
    let entity = player_entity(&mut seated);
    seated
        .world_mut()
        .entity_mut(entity)
        .insert(super::super::seating::Seated::fixture());
    tap(&mut seated, "do_interaction_1");
    seated.update();
    assert!(ladder_of(&mut seated).is_none(), "seated");
    assert_eq!(
        seated.world().resource::<LadderClaim>().0,
        None,
        "seated: no claim"
    );

    // a skill moves the body
    let mut skilled = app(0);
    let entity = player_entity(&mut skilled);
    skilled
        .world_mut()
        .entity_mut(entity)
        .insert(super::super::skill_motion::SkillMove::fixture());
    tap(&mut skilled, "do_interaction_1");
    skilled.update();
    assert!(ladder_of(&mut skilled).is_none(), "skill move");

    // gliding
    let mut gliding = app(0);
    let mut flight = super::super::glide::Gliding::default();
    flight.active = Some(super::super::glide::Active {
        flight: super::super::glide::Flight::start(super::super::glide::GlideParams::EXPERIMENTAL),
        buff_id: 1,
        buff_index: 1,
        pose: super::super::glide::Pose::Start,
        touched: false,
        touched_for: 0.0,
        missing: 0.0,
        water: false,
        deck: None,
    });
    gliding.insert_resource(flight);
    tap(&mut gliding, "do_interaction_1");
    gliding.update();
    assert!(ladder_of(&mut gliding).is_none(), "gliding");
}

#[test]
fn a_deck_walker_grabs_a_mast() {
    // A deck walker has no vehicle binding (`+0x3af0` is null: it sends its own deck moves), so
    // the native grab gate accepts the ladder and the body leaves the deck.
    let mut app = app_with(7, 0);
    let entity = player_entity(&mut app);
    app.world_mut()
        .entity_mut(entity)
        .insert(super::super::ship::DeckRide::fixture(
            9,
            Vec3::ZERO,
            0.0,
            0.0,
            false,
            0.0,
            0.0,
        ));
    tap(&mut app, "do_interaction_1");
    app.update();
    let ladder = ladder_of(&mut app).expect("a deck walker grabs the mast");
    assert_eq!(ladder.kind, 7);
    assert!(
        app.world()
            .get::<super::super::ship::DeckRide>(entity)
            .is_none(),
        "the grab takes the body off the deck"
    );
    assert_eq!(app.world().resource::<LadderClaim>().0, Some(77));
    assert!(
        sent(&mut app)
            .iter()
            .any(|c| matches!(c, Command::Hang { target_obj_id: 77 }))
    );
}

#[test]
fn swimming_allows_the_grab() {
    let mut app = app(0);
    {
        let mut q = app.world_mut().query::<&mut Mover>();
        q.single_mut(app.world_mut()).unwrap().swim = Some(Swim {
            surface: 0.2,
            kind: WaterKind::Sea,
            local_dir: Vec2::ZERO,
            speed: 0.0,
            vertical: 0.0,
            deep: false,
            hold: 0.0,
        });
    }
    tap(&mut app, "do_interaction_1");
    app.update();
    let ladder = ladder_of(&mut app).expect("a ship side ladder is climbed from the sea");
    assert_eq!(ladder.obj_id, 77);
    let mut q = app.world_mut().query::<&Mover>();
    assert!(
        q.single(app.world()).unwrap().swim.is_none(),
        "the grab leaves the water"
    );
}

#[test]
fn a_stance_timeout_or_a_mount_mid_climb_does_not_release() {
    // The only native 0xb site is death: a weapon stance that runs out, a mount starting or a
    // moving skill does not leave the ladder.
    let mut stanced = app(0);
    let entity = player_entity(&mut stanced);
    // The stance is active at the grab and times out during the climb.
    stanced
        .world_mut()
        .entity_mut(entity)
        .insert(super::super::weapons::Drawn {
            until: 0.3,
            out: super::super::weapons::Out::Blade { dual: false },
            sheathing: false,
        });
    tap(&mut stanced, "do_interaction_1");
    stanced.update();
    sent(&mut stanced);
    clear_taps(&mut stanced);
    for _ in 0..3 {
        stanced.update();
    }
    assert!(
        ladder_of(&mut stanced).is_some(),
        "a stance timeout keeps the hang"
    );
    assert!(
        !sent(&mut stanced)
            .iter()
            .any(|c| matches!(c, Command::Unhang { .. })),
        "no unhang on a stance timeout"
    );

    // A mount resource appearing while hanging (our client has no gate on that path) also keeps
    // the hang: the native has no release on mount.
    stanced.insert_resource(super::super::mount::LocalMount(Some(
        super::super::mount::LocalSeat {
            mount: 5,
            point: 1,
            drives: true,
            cart: false,
            ship: false,
            turret: false,
            speed: Default::default(),
            lift: 0.0,
        },
    )));
    stanced.update();
    assert!(
        ladder_of(&mut stanced).is_some(),
        "a mount starting while hanging does not release"
    );
    assert!(
        !sent(&mut stanced)
            .iter()
            .any(|c| matches!(c, Command::Unhang { .. })),
        "no unhang on a mount"
    );

    let mut skilled = app(0);
    tap(&mut skilled, "do_interaction_1");
    skilled.update();
    sent(&mut skilled);
    clear_taps(&mut skilled);
    let entity = player_entity(&mut skilled);
    skilled
        .world_mut()
        .entity_mut(entity)
        .insert(super::super::skill_motion::SkillMove::fixture());
    skilled.update();
    assert!(
        ladder_of(&mut skilled).is_some(),
        "a moving skill keeps the hang"
    );
    assert!(
        !sent(&mut skilled)
            .iter()
            .any(|c| matches!(c, Command::Unhang { .. })),
        "no unhang on a skill"
    );
}

#[test]
fn no_player_entity_clears_the_ladder_claim() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(crate::settings::Actions::default())
        .insert_resource(aa_anim::ClipLibrary::default())
        .insert_resource(Doodads::default())
        .insert_resource(Units::default())
        .insert_resource(LadderClaim(Some(77)))
        .insert_resource(SendState::default())
        .insert_resource(PlayerInput::default())
        .add_message::<NetCommand>()
        .add_systems(Update, control_ladder);
    app.update();
    assert_eq!(
        app.world().resource::<LadderClaim>().0,
        None,
        "without a local player nothing may keep F"
    );
}

#[test]
fn death_sends_unhang_0xb_and_no_release_movement() {
    let mut app = app(0);
    tap(&mut app, "do_interaction_1");
    app.update();
    assert!(ladder_of(&mut app).is_some());
    sent(&mut app);
    let entity = player_entity(&mut app);
    app.world_mut()
        .entity_mut(entity)
        .insert(super::super::combat::Dead { at: 0.0 });
    app.update();
    assert!(ladder_of(&mut app).is_none(), "death leaves the ladder");
    assert_eq!(app.world().resource::<LadderClaim>().0, None);
    let commands = sent(&mut app);
    assert!(
        commands
            .iter()
            .any(|c| matches!(c, Command::Unhang { reason: 0xb })),
        "CSUnhang 0xb: {commands:?}"
    );
    assert!(
        moves(&commands).is_empty(),
        "the native death callback sends only the unhang: {commands:?}"
    );
}

#[test]
fn the_bottom_falls_when_no_ground_is_known_instead_of_standing_in_the_air() {
    let mut app = app(0);
    tap(&mut app, "do_interaction_1");
    app.update();
    clear_taps(&mut app);
    hold(&mut app, "moveback");
    app.update();
    assert!(ladder_of(&mut app).is_none());
    assert!(
        sent(&mut app)
            .iter()
            .any(|c| matches!(c, Command::Unhang { reason: 1 }))
    );
    let mut q = app.world_mut().query::<&Mover>();
    assert!(
        !q.single(app.world()).unwrap().grounded,
        "no terrain fixture: the bottom is a fall, not a stand in the air"
    );
}

#[test]
fn kinds_without_a_top_exit_do_not_step_over() {
    fn exit_step(top: bool, kind: u8) -> Vec3 {
        let mut app = App::new();
        app.add_message::<NetCommand>();
        let entity = app
            .world_mut()
            .spawn((
                LocalPlayer { obj_id: 100 },
                Mover {
                    pos: Vec3::new(1.0, 2.0, 3.0),
                    facing: 0.0,
                    vz: 0.0,
                    grounded: false,
                    velocity: Vec3::ZERO,
                    swim: None,
                    wading: 0.0,
                },
                Motion::default(),
                Transform::default(),
            ))
            .id();
        app.insert_resource(SendState::default());
        app.add_systems(
            Update,
            move |mut commands: Commands,
                  mut q: Query<(&mut Mover, &mut Motion, &mut Transform)>,
                  mut state: ResMut<SendState>,
                  mut out: MessageWriter<NetCommand>| {
                let (mut mover, mut motion, mut transform) = q.single_mut().unwrap();
                finish_exit(
                    &mut commands,
                    entity,
                    &mut mover,
                    &mut motion,
                    &mut transform,
                    &mut state,
                    &mut out,
                    top,
                    kind,
                );
            },
        );
        app.update();
        let mut q = app.world_mut().query::<&Mover>();
        q.single(app.world()).unwrap().pos
    }
    let ladder_top = exit_step(true, 1);
    assert!(
        ladder_top.distance(Vec3::new(1.0, 2.6, 3.05)) < 1e-5,
        "a ladder steps over the top: {ladder_top:?}"
    );
    // the horizontal rope (5), wood wall (3), mast (7) and big tree (10) have no top-exit clip
    for kind in [3u8, 5, 7, 10] {
        let top = exit_step(true, kind);
        assert!(
            top.distance(Vec3::new(1.0, 2.0, 3.0)) < 1e-5,
            "kind {kind} ends at its point: {top:?}"
        );
    }
}

#[test]
fn scripted_actions_hold_and_release() {
    let mut actions = crate::settings::Actions::default();
    actions.script_hold("moveforward");
    assert!(actions.pressed("moveforward", 0));
    actions.script_release("moveforward");
    assert!(!actions.pressed("moveforward", 0));
}
