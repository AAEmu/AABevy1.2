use super::super::anim::{LocalJump, Motion};
use super::super::camera::OrbitCamera;
use super::super::combat::Dead;
use super::super::mount::{DismountMove, LocalMount, LocalSeat, MountReason, track_local_mount};
use super::super::player::{
    CarriedVelocity, Mover, PlayerInput, SendState, air_step, control_player, forward,
    ground_velocity,
};
use super::super::seating::Seated;
use super::super::skill_motion::SkillMove;
use super::super::sprint::Sprint;
use super::super::swim::ActorParams;
use super::super::units::{RemoteUnit, Views};
use super::super::glide::{Active, Flight, GlideParams, Gliding, Phase, Pose, control_glide};
use super::deck::{DeckContact, DeckRide, ship_to_world};
use super::floor::ShipFloor;
use super::*;
use aa_anim::ClipLibrary;
use bevy::input::mouse::AccumulatedMouseMotion;
use aa_net::Command;
use aa_net::NetEvent;
use aa_net::model::{Attachment, Movement, Unit, UnitKind};
use aa_net::PlayerSpawn;
use aa_net::state::{Player, PlayerStats};
use aa_world::TerrainQuery;
use aa_world::collide::Mask;
use aa_world::{WaterQuery, WorldCollision};
use std::sync::Arc;
use std::time::Duration;

fn angle_delta(to: f32, from: f32) -> f32 {
    (to - from + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
        - std::f32::consts::PI
}

fn slave() -> UnitKind {
    UnitKind::Slave { id: 1, tl_id: 2, template_id: 15, summoner_id: 3 }
}

fn level(controller: &ShipController) -> (i8, i8) {
    let request = controller.request(0x22CF, 0).unwrap();
    (request.throttle(), request.steering())
}

#[test]
fn the_captured_steering_samples_fit_the_ramp() {
    // c_right: 0 (t=0), 65 at the next send, 127 on the next frame.
    let mut right = ShipController::default();
    right.step(0.0853, 1.0, 1.0, STEER_RATE);
    assert_eq!(level(&right), (127, 65));
    right.step(0.128, 1.0, 1.0, STEER_RATE);
    assert_eq!(level(&right), (127, 127));

    // c_left: -98 at its first send, then full lock.
    let mut left = ShipController::default();
    left.step(0.1286, 1.0, -1.0, STEER_RATE);
    assert_eq!(level(&left), (127, -98));
    left.step(0.128, 1.0, -1.0, STEER_RATE);
    assert_eq!(level(&left), (127, -127));

    // c_left release: 127 -> 62 -> 0; c_right release: 127 -> 94 -> 0.
    left.step(0.0853, 1.0, 0.0, STEER_RATE);
    assert_eq!(level(&left), (127, -62));
    left.step(0.129, 1.0, 0.0, STEER_RATE);
    assert_eq!(level(&left), (127, 0));
    right.step(0.0433, 1.0, 0.0, STEER_RATE);
    assert_eq!(level(&right), (127, 94));
    right.step(0.129, 1.0, 0.0, STEER_RATE);
    assert_eq!(level(&right), (127, 0));
}

#[test]
fn each_ship_model_ramps_at_its_own_steer_vel() {
    // A 0.8 row (the server fixture value) turns slower than the rowboat's 6.0.
    let mut slow = ShipController::default();
    slow.step(0.1, 0.0, 1.0, 0.8);
    assert_eq!(level(&slow), (0, 10));
    slow.step(0.1, 0.0, 0.0, 0.8);
    assert_eq!(level(&slow), (0, 0), "release decays at the same rate");

    let mut rowboat = ShipController::default();
    rowboat.step(0.1, 0.0, 1.0, 6.0);
    assert_eq!(level(&rowboat), (0, 76));
    // No ship row: the lookup falls back to the rowboat's 6.0.
    let mut fallback = ShipController::default();
    fallback.step(0.1, 0.0, 1.0, STEER_RATE);
    assert_eq!(level(&fallback), (0, 76));
}

#[test]
fn throttle_is_the_native_level_without_a_ramp() {
    let mut controller = ShipController::default();
    controller.step(0.016, 1.0, 0.0, STEER_RATE);
    assert_eq!(level(&controller), (127, 0));
    controller.step(0.016, -1.0, 0.0, STEER_RATE);
    assert_eq!(level(&controller), (-127, 0));
    controller.step(0.016, 0.0, 0.0, STEER_RATE);
    assert_eq!(level(&controller), (0, 0));
    // A / D are opposite signs on the wire.
    assert_eq!(controls(false, false, true, false), (0.0, -1.0));
    assert_eq!(controls(false, false, false, true), (0.0, 1.0));
    assert_eq!(controls(true, false, false, false), (1.0, 0.0));
    assert_eq!(controls(false, true, false, false), (-1.0, 0.0));
}

#[test]
fn the_helm_resends_zero_on_the_100_ms_gate() {
    // The captured idle case sent 87 requests, all 0/0, median 127 ms apart.
    let mut pump = Pump::default();
    let mut controller = ShipController::default();
    let dt = 0.127;
    let mut t = 0.0;
    let mut sends = 0;
    while t < 1.0 {
        t += dt;
        controller.step(dt, 0.0, 0.0, STEER_RATE);
        if pump.tick(dt) {
            pump.sent();
            let request = controller.request(0x22CF, 0).unwrap();
            assert_eq!((request.throttle(), request.steering()), (0, 0));
            assert_eq!(request.flags(), 0);
            sends += 1;
        }
    }
    assert_eq!(sends, 8, "one 127 ms frame per send");
    // The 100 ms gate is the cart's: three 44.4 ms frames, 7 sends a second.
    let mut pump = Pump::default();
    let (mut sends, mut t) = (0, 0.0);
    while t < 1.0 {
        t += 1.0 / 22.5;
        if pump.tick(1.0 / 22.5) {
            pump.sent();
            sends += 1;
        }
    }
    assert_eq!(sends, 7);
}

#[test]
fn only_the_driver_seat_of_a_slave_steers() {
    let mate = UnitKind::Mate { tl_id: 2, template_id: 1, owner_id: 3 };
    assert!(driver_of(&slave(), 1));
    assert!(!driver_of(&slave(), 2), "a passenger does not steer");
    assert!(!driver_of(&mate, 1), "a mate is driven by mount::drive_mount");
}

#[test]
fn scripted_keys_steer_the_ship_like_the_bound_keys() {
    let actions = crate::settings::Actions::default();
    let mut dev = crate::ingame::DevOptions { script_keys: "WA".into(), ..Default::default() };
    assert_eq!(keyed_controls(&actions, &dev, 1.0, false), (1.0, -1.0));
    dev.script_keys = "SD".into();
    assert_eq!(keyed_controls(&actions, &dev, 1.0, false), (-1.0, 1.0));
    assert_eq!(keyed_controls(&actions, &dev, 1.0, true), (0.0, 0.0),
               "a HUD edit box keeps the keys from the helm");
    dev.script_keys.clear();
    assert_eq!(keyed_controls(&actions, &dev, 1.0, false), (0.0, 0.0));
}

#[test]
fn leaving_the_helm_ends_with_one_zero_request() {
    // The capture's last type-4 at the leave is 0/0, just before CSDiscardSlave.
    let mut drive = ShipDrive { obj_id: 0x22CF, rate: 0.8, ..Default::default() };
    drive.controller = ShipController { throttle: 1.0, steering: 0.75 };
    drive.pump.timer = 0.09;
    let request = drive.leave(0x22CF).expect("the final request");
    assert_eq!((request.slave_obj_id(), request.throttle(), request.steering()), (0x22CF, 0, 0));
    assert_eq!(request.flags(), 0);
    assert_eq!(drive.obj_id, 0);
    assert_eq!(drive.rate, 0.0);
    assert_eq!(drive.controller, ShipController::default(), "the ramp does not resume");
    assert_eq!(drive.pump, Pump::default());
    assert_eq!(drive.leaving, 0x22CF);
    assert!(drive.leave(0x22CF).is_none(), "the leave is already in flight");
}

#[test]
fn no_helm_frame_follows_the_leave_request() {
    let seat = LocalMount(Some(super::super::mount::LocalSeat {
        mount: 0x22CF,
        point: 1,
        drives: true,
        cart: false,
        ship: true,
        speed: Default::default(),
        lift: 0.0,
    }));
    let mut app = App::new();
    app.init_resource::<Time>()
        .insert_resource(crate::settings::Actions::default())
        .insert_resource(crate::ingame::DevOptions::default())
        .insert_resource(seat)
        .insert_resource(Units::default())
        .insert_resource(ShipDrive { leaving: 0x22CF, ..Default::default() })
        .add_message::<NetCommand>()
        .add_systems(Update, drive_ship);
    app.update();
    let sent: Vec<_> = app.world_mut().resource_mut::<Messages<NetCommand>>().drain().collect();
    assert!(sent.is_empty(), "nothing between the leave request and the detach");
    app.world_mut().resource_mut::<LocalMount>().0 = None;
    app.update();
    let sent: Vec<_> = app.world_mut().resource_mut::<Messages<NetCommand>>().drain().collect();
    assert!(sent.is_empty(), "nothing after the detach");
}

#[test]
fn deck_offsets_rotate_into_the_ship_frame_and_back() {
    use super::deck::{ship_to_world, world_to_ship};
    let root = Vec3::new(100.0, 200.0, 10.0);
    // game yaw 90 degrees: forward is west (-x), right is north (+y)
    let p = ship_to_world(root, Quat::from_rotation_z(std::f32::consts::FRAC_PI_2), Vec3::new(2.0, 3.0, 1.5));
    assert!((p.x - 97.0).abs() < 1e-4, "{p:?}");
    assert!((p.y - 202.0).abs() < 1e-4, "{p:?}");
    assert!((p.z - 11.5).abs() < 1e-4, "{p:?}");
    let back = world_to_ship(root, Quat::from_rotation_z(std::f32::consts::FRAC_PI_2), p);
    assert!(
        (back - Vec3::new(2.0, 3.0, 1.5)).length() < 1e-4,
        "{back:?}"
    );
}

#[test]
fn a_moving_hull_carries_the_same_deck_point() {
    use super::deck::{ship_to_world, world_to_ship};
    let local = Vec3::new(1.2, -2.5, 0.9);
    let root = Vec3::new(4306.78, 4128.0, 99.39);
    let yaw = std::f32::consts::FRAC_PI_2;
    let world = ship_to_world(root, Quat::from_rotation_z(yaw), local);
    // the hull moves 10 m and turns a quarter turn: the local point is unchanged
    let moved = root + Vec3::new(10.0, 4.0, -0.2);
    let turned = yaw + std::f32::consts::FRAC_PI_2;
    let carried = ship_to_world(moved, Quat::from_rotation_z(turned), local);
    assert!(
        (carried - world).length() > 5.0,
        "the point moved with the hull"
    );
    let back = world_to_ship(moved, Quat::from_rotation_z(turned), carried);
    assert!((back - local).length() < 1e-3, "{back:?}");
}

#[test]
fn the_deck_movement_carries_the_captured_support_block() {
    use super::deck::deck_move;
    use aa_net::model::{move_flags, radians_to_direction};
    use aa_net::{MoveState, Pos};
    let local = Vec3::new(0.0039, -1.7695, 0.9378);
    let world = Vec3::new(4308.541, 4128.0254, 100.5601);
    let relative = 0.5;
    let world_facing = std::f32::consts::FRAC_PI_2 + relative;
    let command = deck_move(
        8911,
        local,
        relative,
        world,
        world_facing,
        true,
        MoveState::Ground,
        [0, 127, 0],
    );
    let m = command.to_movement(0);
    assert_eq!(
        (m.flags, m.actor_flags),
        (move_flags::STANDING_ON_OBJECT, 0x24)
    );
    assert_eq!(m.pos, Pos::new(local.x, local.y, local.z));
    assert_eq!(m.direction, radians_to_direction(relative));
    let s = m.support.clone().unwrap();
    assert_eq!((s.kind, s.part, s.obj_id), (1, 1, 8911));
    let (aux, rot) = s.world.unwrap();
    assert!((aux.x - world.x).abs() < 1e-3 && (aux.z - world.z).abs() < 1e-3);
    assert_eq!(rot, [0, 0, radians_to_direction(world_facing)]);
    // a deck jump keeps the block: the captured header / actor bytes 0x44 / 0x34
    let jump = deck_move(
        8911,
        local,
        relative,
        world,
        world_facing,
        false,
        MoveState::Jump,
        [0; 3],
    )
    .to_movement(0);
    assert_eq!((jump.flags, jump.actor_flags), (0x44, 0x34));
    assert_eq!(jump.support.as_ref().unwrap().obj_id, 8911);
    // the capture's idle deck rows are 0x40 (no STOPPING); 0x44 is only the jump
    assert_eq!(
        deck_move(
            8911,
            local,
            relative,
            world,
            0.0,
            false,
            MoveState::Ground,
            [0; 3]
        )
        .to_movement(0)
        .flags,
        0x40
    );
}

#[test]
fn a_captured_deck_row_encodes_the_relative_and_world_facings() {
    use super::deck::deck_move;
    use aa_net::MoveState;
    // native.json d_leave at t=1793261507160: ship yaw 1.3977, player world
    // yaw -1.7344, primary rot (0, 2, -63), aux rot (0, 0, -35).
    let local = Vec3::new(-0.0215, -0.3125, 1.2469);
    let world = Vec3::new(4282.24, 4126.2, 100.73);
    let (ship_yaw, player_yaw) = (1.3977, -1.7344);
    let m = deck_move(
        8911,
        local,
        player_yaw - ship_yaw,
        world,
        player_yaw,
        true,
        MoveState::Ground,
        [0; 3],
    )
    .to_movement(0);
    assert_eq!(
        m.direction, -63,
        "the primary direction is the ship-relative facing"
    );
    let (_, rot) = m.support.unwrap().world.unwrap();
    assert_eq!(rot, [0, 0, -35], "aux z is the world facing");
    // a_idle at the same hull: player and ship both face pi/2, so primary 0
    // and aux 32.
    let m = deck_move(
        8911,
        Vec3::new(0.0039, -1.7695, 0.9378),
        0.0,
        Vec3::new(4308.541, 4128.0254, 100.5601),
        std::f32::consts::FRAC_PI_2,
        true,
        MoveState::Ground,
        [0; 3],
    )
    .to_movement(0);
    assert_eq!(m.direction, 0);
    let (_, rot) = m.support.unwrap().world.unwrap();
    assert_eq!(rot, [0, 0, 32]);
}

#[test]
fn the_deck_walks_where_the_camera_looks() {
    use super::super::anim::RUN_SPEED;
    use super::super::player::{FaceCamera, MoveKeys, face_camera, forward, resolve_keys};
    use super::deck::deck_step;
    let (fwd, strafe, turn) = resolve_keys(
        MoveKeys {
            forward: true,
            ..MoveKeys::default()
        },
        false,
        false,
        0,
    );
    assert_eq!((fwd, strafe, turn), (1.0, 0.0, 0.0));
    let (ship_yaw, camera_yaw) = (1.3977, -1.7344);
    // W on the first frame snaps the facing to the camera, as on foot
    assert_eq!(face_camera(false, false, true, false), FaceCamera::Snap);
    let facing = camera_yaw - ship_yaw;
    let velocity = Vec2::new(strafe, fwd).normalize_or_zero() * RUN_SPEED;
    let step = deck_step(facing, velocity, 1.0);
    let world = super::deck::ship_to_world(Vec3::ZERO, Quat::from_rotation_z(ship_yaw), Vec3::new(step.x, step.y, 0.0));
    let want = forward(camera_yaw) * RUN_SPEED;
    assert!((world.x - want.x).abs() < 1e-3, "{world:?} vs {want:?}");
    assert!((world.y - want.y).abs() < 1e-3, "{world:?} vs {want:?}");
    // with the camera beside the bow the step crosses the hull, not the bow
    assert!(world.x.abs() > 1.0 && world.y.abs() < world.x.abs());
}

#[test]
fn a_deck_jump_rises_and_lands_on_the_same_floor() {
    use super::deck::deck_air_step;
    // 1.2's jump: sqrt(2 * 25 * 1.3) = 8.06 m/s, peaks at 1.3 m.
    let (mut z, mut vz, ground) = (0.9, 8.06, 0.9);
    let mut peak = z;
    let mut landed = false;
    for _ in 0..120 {
        (z, vz, landed) = deck_air_step(z, vz, ground, 25.0, 1.0 / 60.0);
        peak = peak.max(z);
        if landed {
            break;
        }
    }
    assert!(landed, "the jump comes back down");
    assert_eq!(z, ground);
    assert!((peak - ground - 1.3).abs() < 0.02, "peak {peak}");
}

#[test]
fn scripted_keys_walk_turn_and_jump_the_deck() {
    let Some(mut f) = DeckApp::new(Vec2::ZERO, true, [0; 3], 1.3977, 1.3977) else {
        return;
    };
    let start = f.mover().pos;
    f.app
        .world_mut()
        .resource_mut::<crate::ingame::DevOptions>()
        .script_keys = "W".into();
    for _ in 0..2 {
        f.tick(0.1);
    }
    let walked = f.mover().pos.distance(start);
    assert!(walked > 0.3, "scripted W walked {walked} m");
    let moves = f.moves();
    assert!(
        moves.iter().any(|m| {
            let m = m.to_movement(0);
            m.flags == 0x40 && m.actor_flags == 0x24 && m.support.is_some()
        }),
        "no 0x40/0x24 support packet while scripted-W walking: {moves:?}"
    );
    f.app
        .world_mut()
        .resource_mut::<crate::ingame::DevOptions>()
        .script_keys = "A".into();
    let before = f.mover().facing;
    f.tick(0.2);
    assert!(
        (f.mover().facing - before).abs() > 0.05,
        "scripted A did not turn the deck"
    );
    f.app
        .world_mut()
        .resource_mut::<crate::ingame::DevOptions>()
        .script_keys = String::new();
    f.tick(0.1);
    f.app
        .world_mut()
        .resource_mut::<crate::ingame::DevOptions>()
        .script_keys = "J".into();
    f.tick(0.1);
    let moves = f.moves();
    assert!(
        moves.iter().any(|m| {
            let m = m.to_movement(0);
            m.flags == 0x44 && m.actor_flags == 0x34 && m.support.is_some()
        }),
        "no scripted deck jump 0x44/0x34: {moves:?}"
    );
    assert!(f.deck().is_some(), "a deck jump keeps the ride");
}

#[test]
fn dead_skill_moved_and_seated_players_do_not_ride_the_deck() {
    use super::deck::deck_allowed;
    assert!(deck_allowed(false, false, false));
    assert!(!deck_allowed(true, false, false));
    assert!(!deck_allowed(false, true, false));
    assert!(!deck_allowed(false, false, true));
}

#[test]
fn the_deck_range_gate_rejects_points_outside_the_hull() {
    use super::deck::in_hull_bounds;
    use super::floor::ShipFloor;
    let quad = |z: f32| {
        [
            [
                Vec3::new(-2.0, -5.0, z),
                Vec3::new(2.0, -5.0, z),
                Vec3::new(2.0, 5.0, z),
            ],
            [
                Vec3::new(-2.0, -5.0, z),
                Vec3::new(2.0, 5.0, z),
                Vec3::new(-2.0, 5.0, z),
            ],
        ]
    };
    let floor = ShipFloor::new(quad(1.0));
    let bounds = floor.bounds().expect("floor bounds");
    assert!(in_hull_bounds(bounds, Vec3::new(2.5, 5.5, 1.0), 2.0));
    assert!(!in_hull_bounds(bounds, Vec3::new(5.0, 5.5, 1.0), 2.0));
    assert!(!in_hull_bounds(bounds, Vec3::new(0.0, -8.0, 1.0), 2.0));
}

#[test]
fn a_teleport_clears_the_helm_ramp_and_pump() {
    let mut app = App::new();
    app.insert_resource(crate::ingame::teleport::Teleport::loading_fixture())
        .init_resource::<ShipDrive>()
        .add_systems(Update, reset_on_teleport);
    {
        let mut drive = app.world_mut().resource_mut::<ShipDrive>();
        drive.obj_id = 0x22CF;
        drive.rate = 6.0;
        drive.controller = ShipController { throttle: 1.0, steering: -0.5 };
        drive.pump.timer = 0.09;
    }
    app.update();
    let drive = app.world().resource::<ShipDrive>();
    assert_eq!(drive.obj_id, 0);
    assert_eq!(drive.rate, 0.0);
    assert_eq!(drive.controller, ShipController::default(), "the pre-teleport ramp does not resume");
    assert_eq!(drive.pump, Pump::default());
}

// ---- deck system tests (Bevy App with the real `board_deck` / `deck_control`) ----------------

const DECK_SHIP: u32 = 8911;
const DECK_MODEL: u32 = 129;
const ROWBOAT_PREFAB: &str = "prefab://prefabs/smallboat.xml/smallboat.smallboat_a";

fn deck_ship_unit(root: Vec3, yaw: f32, velocity: [i16; 3]) -> Unit {
    let mut movement = Movement::unit(aa_net::Pos::new(root.x, root.y, root.z), yaw, 0);
    movement.move_type = 3;
    movement.velocity = velocity;
    Unit {
        obj_id: DECK_SHIP,
        name: "Rowboat".into(),
        kind: UnitKind::Slave {
            id: 1,
            tl_id: 2,
            template_id: 15,
            summoner_id: 3,
        },
        owner_name: String::new(),
        pos: aa_net::Pos::new(root.x, root.y, root.z),
        rotation: [0.0, 0.0, yaw],
        scale: 1.0,
        level: 1,
        model_id: DECK_MODEL,
        equipment: Default::default(),
        model_params: Default::default(),
        hp: 100,
        mp: 0,
        max_hp: 100,
        max_mp: 0,
        dead: false,
        race_gender: 0,
        movement: Some(movement),
        faction_id: 0,
        expedition_id: 0,
        appellation: 0,
        flags: 0,
        gm_modes: [0; 4],
        bounty_money: None,
        pvp_honor: 0,
        pvp_kills: 0,
        skills: Vec::new(),
        passive_buffs: Vec::new(),
        ability_exp: Vec::new(),
        active_abilities: Vec::new(),
        buffs: Vec::new(),
        anim_action: 0,
        house_posture: None,
        state_attachments: None,
        attached_to: None,
        doodad_bond: None,
    }
}

struct DeckApp {
    app: App,
    player: Entity,
    camera: Entity,
    floor: Arc<ShipFloor>,
    one_shot: Vec<String>,
}

impl DeckApp {
    fn new(
        at: Vec2,
        ride: bool,
        ship_velocity: [i16; 3],
        ship_yaw: f32,
        camera_yaw: f32,
    ) -> Option<DeckApp> {
        if !aa_cry::pak_available() {
            return None;
        }
        let db = Db::ship_fixture(DECK_MODEL, ROWBOAT_PREFAB).expect("the temp ship catalog");
        let floor = db.ship_floor(DECK_MODEL).expect("the rowboat hull floor");
        DeckApp::with_floor(db, floor, at, ride, ship_velocity, ship_yaw, camera_yaw)
    }

    fn with_floor(
        db: Db,
        floor: Arc<ShipFloor>,
        at: Vec2,
        ride: bool,
        ship_velocity: [i16; 3],
        ship_yaw: f32,
        camera_yaw: f32,
    ) -> Option<DeckApp> {
        Self::with_floor_at(Vec3::new(100.0, 200.0, 10.0), db, floor, at, ride, ship_velocity, ship_yaw, camera_yaw)
    }

    #[allow(clippy::too_many_arguments)]
    fn with_floor_at(
        root: Vec3,
        db: Db,
        floor: Arc<ShipFloor>,
        at: Vec2,
        ride: bool,
        ship_velocity: [i16; 3],
        ship_yaw: f32,
        camera_yaw: f32,
    ) -> Option<DeckApp> {
        Self::with_floor_at_height(root, None, db, floor, at, ride, ship_velocity, ship_yaw, camera_yaw)
    }

    /// `start` is the ship-local floor height to start at; `None` picks the highest
    /// floor under `at` (the upper deck on a multi-level hull).
    #[allow(clippy::too_many_arguments)]
    fn with_floor_at_height(
        root: Vec3,
        start: Option<f32>,
        db: Db,
        floor: Arc<ShipFloor>,
        at: Vec2,
        ride: bool,
        ship_velocity: [i16; 3],
        ship_yaw: f32,
        camera_yaw: f32,
    ) -> Option<DeckApp> {
        let height = start
            .or_else(|| floor.height(at.x, at.y, 100.0))
            .expect("a floor point");
        let local = Vec3::new(at.x, at.y, height);
        let world = ship_to_world(root, Quat::from_rotation_z(ship_yaw), local);
        let facing = camera_yaw - ship_yaw;
        let mut units = Units::default();
        units
            .by_id
            .insert(DECK_SHIP, deck_ship_unit(root, ship_yaw, ship_velocity));
        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<crate::settings::Actions>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<AccumulatedMouseMotion>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<Player>()
            .init_resource::<PlayerInput>()
            .init_resource::<SendState>()
            .init_resource::<LocalMount>()
            .init_resource::<Sprint>()
            .init_resource::<crate::ingame::DevOptions>()
            .init_resource::<Views>()
            .init_resource::<LocalJump>()
            .init_resource::<Gliding>()
            .insert_resource(units)
            .insert_resource(db)
            .insert_resource(TerrainQuery::new("test"))
            .insert_resource(WaterQuery::new("test"))
            .insert_resource(WorldCollision::new("test", TerrainQuery::new("test")))
            .add_message::<NetCommand>()
            .add_systems(Update, (control_glide, board_deck, deck_control).chain());
        let camera = app
            .world_mut()
            .spawn(OrbitCamera {
                yaw: camera_yaw,
                ..default()
            })
            .id();
        let mut entity = app.world_mut().spawn((
            LocalPlayer { obj_id: 1 },
            Mover {
                pos: world,
                facing: camera_yaw,
                vz: 0.0,
                grounded: true,
                velocity: Vec3::ZERO,
                swim: None,
                wading: 0.0,
            },
            Transform::default(),
            Motion::default(),
            ActorParams::default(),
        ));
        if ride {
            entity.insert(DeckRide::fixture(
                DECK_SHIP, local, height, facing, false, 0.0, ship_yaw,
            ));
        }
        let player = entity.id();
        Some(DeckApp {
            app,
            player,
            camera,
            floor,
            one_shot: Vec::new(),
        })
    }

    fn tick(&mut self, dt: f32) {
        self.app
            .world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs_f32(dt));
        self.app.update();
        let mut actions = self
            .app
            .world_mut()
            .resource_mut::<crate::settings::Actions>();
        for action in self.one_shot.drain(..) {
            actions.release_for_test(&action);
        }
        actions.clear_taps_for_test();
    }

    fn tap(&mut self, action: &str) {
        self.app
            .world_mut()
            .resource_mut::<crate::settings::Actions>()
            .tap_for_test(action);
        self.one_shot.push(action.into());
    }

    fn hold(&mut self, action: &str) {
        self.app
            .world_mut()
            .resource_mut::<crate::settings::Actions>()
            .hold_for_test(action);
    }

    fn release(&mut self, action: &str) {
        self.app
            .world_mut()
            .resource_mut::<crate::settings::Actions>()
            .release_for_test(action);
    }

    fn add_on_foot(&mut self, terrain: TerrainQuery, collision: WorldCollision, water: WaterQuery) {
        self.app.insert_resource(terrain);
        self.app.insert_resource(collision);
        self.app.insert_resource(water);
        self.app
            .add_systems(Update, control_player.after(deck_control));
    }

    fn deck(&self) -> Option<&DeckRide> {
        self.app.world().get::<DeckRide>(self.player)
    }

    fn mover(&self) -> &Mover {
        self.app.world().get::<Mover>(self.player).unwrap()
    }

    fn orbit(&self) -> &OrbitCamera {
        self.app.world().get::<OrbitCamera>(self.camera).unwrap()
    }

    fn moves(&mut self) -> Vec<aa_net::MoveCommand> {
        self.app
            .world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .filter_map(|NetCommand(command)| match command {
                Command::Move(m) => Some(m),
                _ => None,
            })
            .collect()
    }
}

#[test]
fn a_deck_jump_keeps_the_ride_and_lands_on_the_floor() {
    let Some(mut f) = DeckApp::new(Vec2::ZERO, true, [0; 3], 1.3977, 1.3977) else {
        return;
    };
    let start = f.mover().pos;
    f.tap("jump");
    f.tick(0.1);
    assert!(f.deck().is_some(), "the jump keeps DeckRide");
    let moves = f.moves();
    let jump = moves.last().expect("the jump packet").to_movement(0);
    assert_eq!((jump.flags, jump.actor_flags), (0x44, 0x34));
    assert_eq!(jump.support.as_ref().unwrap().obj_id, DECK_SHIP);
    let mut impacts = Vec::new();
    for _ in 0..20 {
        f.tick(0.1);
        impacts.extend(f.moves().iter().map(|move_command| move_command.fall_vel));
    }
    assert!(f.deck().is_some(), "landing keeps the ride");
    assert!(
        impacts.iter().any(|&impact| impact > 0),
        "the deck landing did not report its impact: {impacts:?}"
    );
    let mover = f.mover();
    assert!(mover.grounded);
    assert!(
        (mover.pos.z - start.z).abs() < 1e-3,
        "back on the hull floor"
    );
}

#[test]
fn a_deck_jump_past_the_rail_leaves_with_the_ships_velocity() {
    let Some(probe) = DeckApp::new(Vec2::ZERO, false, [0; 3], 1.3977, 1.3977) else {
        return;
    };
    let (_, max) = probe.floor.bounds().expect("rowboat bounds");
    let at = (0..50)
        .find_map(|i| {
            let x = max.x - i as f32 * 0.1;
            probe.floor.height(x, 0.0, 100.0).map(|_| Vec2::new(x, 0.0))
        })
        .expect("a floor point at the rail");
    let yaw = 1.3977;
    let camera_yaw = yaw - std::f32::consts::FRAC_PI_2;
    let ship_velocity = [3277, -1638, 0];
    let Some(mut f) = DeckApp::new(at, true, ship_velocity, yaw, camera_yaw) else {
        return;
    };
    f.hold("moveforward");
    f.tap("jump");
    f.tick(0.1);
    assert!(f.deck().is_none(), "the jump off the rail leaves the deck");
    assert!(
        f.app.world().get::<Motion>(f.player).unwrap().airborne,
        "the jump-off lost its air clip"
    );
    assert!(
        f.app.world().resource::<LocalJump>().rising,
        "the jump-off lost its take-off"
    );
    let mover = f.mover();
    assert!(!mover.grounded);
    let ship = Vec3::new(3277.0, -1638.0, 0.0) * (30.0 / 32767.0);
    let step = ship_to_world(Vec3::ZERO, Quat::from_rotation_z(yaw), Vec3::new(5.4, 0.0, 0.0));
    assert!(
        (mover.velocity.x - (ship.x + step.x)).abs() < 0.02,
        "{:?}",
        mover.velocity
    );
    assert!(
        (mover.velocity.y - (ship.y + step.y)).abs() < 0.02,
        "{:?}",
        mover.velocity
    );
    assert!(mover.vz > 0.0, "still rising when the feet leave the hull");
}

#[test]
fn the_deck_follows_the_drawn_hull_pose() {
    let Some(mut f) = DeckApp::new(Vec2::ZERO, false, [0; 3], 1.3977, 1.3977) else {
        return;
    };
    let height = f.floor.height(0.0, 0.0, 100.0).expect("a floor point");
    let wire = Vec3::new(100.0, 200.0, 10.0);
    let drawn = wire + Vec3::new(0.4, -0.3, 0.05);
    let drawn_yaw = 1.3977 + 0.04;
    let mut hull = RemoteUnit::fixture(DECK_SHIP, drawn);
    hull.facing = drawn_yaw;
    hull.game_rotation = Quat::from_rotation_z(drawn_yaw);
    hull.rotation = aa_world::coords::cry_rot_to_bevy(hull.game_rotation);
    let entity = f.app.world_mut().spawn(hull).id();
    let unit = deck_ship_unit(wire, 1.3977, [0; 3]);
    f.app
        .world_mut()
        .resource_mut::<Views>()
        .fixture_actor(&unit, Some(entity));
    let local = Vec3::new(0.0, 0.0, height);
    // stand at the drawn hull's deck centre so boarding must use the same pose
    let world = f.app.world_mut();
    let mut player = world.entity_mut(f.player);
    let mut mover = player.get_mut::<Mover>().unwrap();
    mover.pos = ship_to_world(drawn, Quat::from_rotation_z(drawn_yaw), local);
    mover.grounded = true;
    mover.vz = 0.0;
    f.tick(0.1);
    let want = ship_to_world(drawn, Quat::from_rotation_z(drawn_yaw), local);
    assert!(
        f.mover().pos.distance(want) < 1e-3,
        "placed at the wire hull: {:?} vs drawn {:?}",
        f.mover().pos,
        want
    );
    let moved = drawn + Vec3::new(0.7, 0.2, 0.0);
    {
        let world = f.app.world_mut();
        let mut drawn_entity = world.entity_mut(entity);
        let mut hull = drawn_entity.get_mut::<RemoteUnit>().unwrap();
        hull.pos = moved;
        hull.facing += 0.02;
        hull.game_rotation = Quat::from_rotation_z(drawn_yaw + 0.02);
        hull.rotation = aa_world::coords::cry_rot_to_bevy(hull.game_rotation);
    }
    f.tick(0.1);
    let want = ship_to_world(moved, Quat::from_rotation_z(drawn_yaw + 0.02), local);
    assert!(
        f.mover().pos.distance(want) < 1e-3,
        "the player kept the drawn hull's offset: {:?} vs {:?}",
        f.mover().pos,
        want
    );
}

/// A synthetic deck quad at `z`, with no pak: x -4..4, y -6..6 (the tests below never read a
/// hull brush).
fn synthetic_deck(z: f32) -> Arc<ShipFloor> {
    let quad = |x0: f32, x1: f32, y0: f32, y1: f32| {
        [
            [
                Vec3::new(x0, y0, z),
                Vec3::new(x1, y0, z),
                Vec3::new(x1, y1, z),
            ],
            [
                Vec3::new(x0, y0, z),
                Vec3::new(x1, y1, z),
                Vec3::new(x0, y1, z),
            ],
        ]
    };
    Arc::new(ShipFloor::new(quad(-4.0, 4.0, -6.0, 6.0)))
}

fn synthetic_deck_app(
    at: Vec2,
    ride: bool,
    ship_yaw: f32,
    camera_yaw: f32,
) -> Option<(DeckApp, Arc<ShipFloor>, Db)> {
    let floor = synthetic_deck(0.0);
    let db = Db::ship_floor_fixture(DECK_MODEL, floor.clone())?;
    let app = DeckApp::with_floor(db.clone(), floor.clone(), at, ride, [0; 3], ship_yaw, camera_yaw)?;
    Some((app, floor, db))
}

#[test]
fn the_hull_yaw_comes_from_the_full_rotation() {
    let rotation =
        Quat::from_rotation_z(0.4) * Quat::from_rotation_x(0.07) * Quat::from_rotation_y(0.03);
    assert!(
        (super::deck::yaw_of(rotation) - 0.4).abs() < 5e-3,
        "yaw {}",
        super::deck::yaw_of(rotation)
    );
    assert!((super::deck::yaw_of(Quat::from_rotation_x(0.07))).abs() < 1e-3);
}

#[test]
fn a_pitched_deck_step_keeps_its_ship_local_length() {
    let rotation =
        Quat::from_rotation_z(0.7) * Quat::from_rotation_x(0.3) * Quat::from_rotation_y(0.2);
    let step = Vec2::new(0.12, 0.3);
    let flat = super::deck::deck_world_step(rotation, step);
    let local = rotation.inverse() * Vec3::new(flat.x, flat.y, 0.0);
    assert!(
        (local.truncate().length() - step.length()).abs() < 1e-5,
        "the deck step is {:.4} m instead of {:.4} m",
        local.truncate().length(),
        step.length()
    );
    // dropping the rotated z (the old slide input) is the cos² loss
    let naive = (rotation * Vec3::new(step.x, step.y, 0.0)).truncate();
    let naive_local = rotation.inverse() * Vec3::new(naive.x, naive.y, 0.0);
    assert!(
        naive_local.truncate().length() < step.length() * 0.99,
        "the old flat step was not shorter"
    );
}

#[test]
fn a_pitched_hull_keeps_a_stander_on_the_drawn_deck() {
    let yaw = 0.35;
    let pitch = 0.07f32; // about 4 degrees, the server's visual pitch range
    let rotation = Quat::from_rotation_z(yaw) * Quat::from_rotation_x(pitch);
    let local_y = 4.0;
    let Some((mut f, _, _)) = synthetic_deck_app(Vec2::new(0.0, local_y), false, yaw, yaw) else {
        return;
    };
    let root = Vec3::new(100.0, 200.0, 10.0);
    let height = f.floor.height(0.0, local_y, 100.0).expect("synthetic deck");
    let local = Vec3::new(0.0, local_y, height);
    let mut hull = RemoteUnit::fixture(DECK_SHIP, root);
    hull.facing = yaw;
    hull.game_rotation = rotation;
    hull.rotation = aa_world::coords::cry_rot_to_bevy(rotation);
    let hull_entity = f.app.world_mut().spawn(hull).id();
    let unit = deck_ship_unit(root, yaw, [0; 3]);
    f.app
        .world_mut()
        .resource_mut::<Views>()
        .fixture_actor(&unit, Some(hull_entity));
    // start on the pitch-composed deck point
    let start = ship_to_world(root, rotation, local);
    {
        let world = f.app.world_mut();
        let mut player = world.entity_mut(f.player);
        let mut mover = player.get_mut::<Mover>().unwrap();
        mover.pos = start;
        mover.grounded = true;
        mover.vz = 0.0;
    }
    f.tick(0.1);
    f.tick(0.1);
    assert!(f.deck().is_some(), "the player did not board the synthetic pitched hull");
    let want = ship_to_world(root, rotation, local);
    assert!(
        f.mover().pos.distance(want) < 1e-3,
        "the stander is not on the pitched deck: {:?} vs {want:?}",
        f.mover().pos
    );
    let yaw_only = ship_to_world(root, Quat::from_rotation_z(yaw), local);
    assert!(
        want.distance(yaw_only) > 0.05,
        "the pitched point is not distinguishable from yaw only ({:.3} m)",
        want.distance(yaw_only)
    );
    // the hull moves and pitches further: the stander keeps its deck point
    let moved = root + Vec3::new(0.6, -0.4, 0.0);
    let turned = rotation * Quat::from_rotation_x(0.02);
    {
        let world = f.app.world_mut();
        let mut entity = world.entity_mut(hull_entity);
        let mut hull = entity.get_mut::<RemoteUnit>().unwrap();
        hull.pos = moved;
        hull.game_rotation = turned;
        hull.rotation = aa_world::coords::cry_rot_to_bevy(turned);
    }
    f.tick(0.1);
    let want = ship_to_world(moved, turned, local);
    assert!(
        f.mover().pos.distance(want) < 1e-3,
        "the stander left the pitched deck: {:?} vs {want:?}",
        f.mover().pos
    );
}

#[test]
fn a_pitched_hull_landing_uses_the_drawn_deck() {
    use super::deck::deck_landing;
    let floor = synthetic_deck(0.0);
    let Some(db) = Db::ship_floor_fixture(DECK_MODEL, floor) else {
        return;
    };
    let root = Vec3::new(100.0, 200.0, 10.0);
    let yaw = 0.35;
    let rotation = Quat::from_rotation_z(yaw) * Quat::from_rotation_x(0.07);
    let mut units = Units::default();
    units
        .by_id
        .insert(DECK_SHIP, deck_ship_unit(root, yaw, [0; 3]));
    let local = Vec3::new(0.0, 4.0, 0.0);
    let at = ship_to_world(root, rotation, Vec3::new(0.0, 4.0, 1.2));
    let (_, landing) = deck_landing(
        &db,
        &units,
        DECK_SHIP,
        at,
        0.0,
        Some((root, rotation)),
    )
    .expect("a landing on the synthetic deck");
    let want = ship_to_world(root, rotation, local);
    assert!((landing - want).length() < 1e-4, "landing {landing:?} vs {want:?}");
    assert!(
        landing.distance(ship_to_world(root, Quat::from_rotation_z(yaw), local)) > 0.05,
        "the landing used the yaw-only point"
    );
}

#[test]
fn a_pitched_hull_walk_off_leaves_on_the_deck_plane() {
    let yaw = 0.35;
    let rotation = Quat::from_rotation_z(yaw) * Quat::from_rotation_x(0.07);
    let local = Vec3::new(0.0, 5.5, 0.0);
    let Some((mut f, _, _)) = synthetic_deck_app(Vec2::new(local.x, local.y), true, yaw, yaw) else {
        return;
    };
    let root = Vec3::new(100.0, 200.0, 10.0);
    let height = f.floor.height(local.x, local.y, 100.0).expect("synthetic deck");
    let start = ship_to_world(root, rotation, Vec3::new(local.x, local.y, height));
    {
        let world = f.app.world_mut();
        let mut player = world.entity_mut(f.player);
        let mut mover = player.get_mut::<Mover>().unwrap();
        mover.pos = start;
        mover.grounded = true;
        mover.vz = 0.0;
    }
    let mut hull = RemoteUnit::fixture(DECK_SHIP, root);
    hull.facing = yaw;
    hull.game_rotation = rotation;
    hull.rotation = aa_world::coords::cry_rot_to_bevy(rotation);
    let hull_entity = f.app.world_mut().spawn(hull).id();
    let unit = deck_ship_unit(root, yaw, [0; 3]);
    f.app
        .world_mut()
        .resource_mut::<Views>()
        .fixture_actor(&unit, Some(hull_entity));
    f.hold("moveforward");
    for _ in 0..12 {
        f.tick(0.1);
        if f.deck().is_none() {
            break;
        }
    }
    assert!(f.deck().is_none(), "the player did not walk off the pitched hull");
    let local = super::deck::world_to_ship(root, rotation, f.mover().pos);
    assert!(
        local.z.abs() < 0.06,
        "the walk-off left the deck plane (local z {:.3})",
        local.z
    );
    assert!(local.y > 4.5, "the walk-off did not move forward: {local:?}");
}

#[test]
fn a_face_target_turn_on_the_deck_survives_and_the_ship_turns_the_player() {
    let Some(mut f) = DeckApp::new(Vec2::ZERO, true, [0; 3], 1.3977, 1.3977) else {
        return;
    };
    f.tick(0.1);
    let yaw = 1.3977;
    {
        let world = f.app.world_mut();
        let mut player = world.entity_mut(f.player);
        player.get_mut::<Mover>().unwrap().facing = yaw + 0.5;
    }
    f.tick(0.1);
    assert!(
        (angle_delta(f.mover().facing, yaw + 0.5)).abs() < 1e-4,
        "the FaceTarget turn was rewritten to {}",
        f.mover().facing
    );
    // the hull turns: the player's world facing turns with it, relative facing stays
    {
        let mut units = f.app.world_mut().resource_mut::<Units>();
        units.by_id.get_mut(&DECK_SHIP).unwrap().rotation[2] += 0.2;
    }
    f.tick(0.1);
    assert!(
        (angle_delta(f.mover().facing, yaw + 0.7)).abs() < 0.02,
        "the player did not turn with the hull: {}",
        f.mover().facing
    );
}

#[test]
fn a_walk_off_starts_grounded() {
    let Some(probe) = DeckApp::new(Vec2::ZERO, false, [0; 3], 0.0, 0.0) else {
        return;
    };
    let (_, max) = probe.floor.bounds().expect("rowboat bounds");
    let at = (0..50)
        .find_map(|i| {
            let x = max.x - i as f32 * 0.1;
            probe.floor.height(x, 0.0, 100.0).map(|_| Vec2::new(x, 0.0))
        })
        .expect("a floor point at the rail");
    let db = Db::ship_fixture(DECK_MODEL, ROWBOAT_PREFAB).expect("the temp ship catalog");
    let floor = db.ship_floor(DECK_MODEL).expect("the rowboat hull floor");
    let Some(mut f) = DeckApp::with_floor_at(
        Vec3::new(100.0, 200.0, 10.0),
        db,
        floor,
        at,
        true,
        [0; 3],
        0.0,
        -std::f32::consts::FRAC_PI_2,
    ) else {
        return;
    };
    f.hold("moveforward");
    for _ in 0..10 {
        f.tick(0.1);
        if f.deck().is_none() {
            break;
        }
    }
    assert!(f.deck().is_none(), "the player did not walk off");
    assert!(f.mover().grounded, "a walk-off started as a fall");
    assert!(
        !f.app.world().get::<Motion>(f.player).unwrap().airborne,
        "a walk-off kept the jump/air clip"
    );
}

#[test]
fn a_rising_jump_over_the_deck_is_not_snapped() {
    let Some(mut f) = DeckApp::new(Vec2::ZERO, false, [0; 3], 1.3977, 1.3977) else {
        return;
    };
    let height = f.floor.height(0.0, 0.0, 100.0).expect("a floor point");
    {
        let world = f.app.world_mut();
        let mut player = world.entity_mut(f.player);
        let mut mover = player.get_mut::<Mover>().unwrap();
        mover.pos.z = 10.0 + height + 0.3;
        mover.grounded = false;
        mover.vz = 3.0;
    }
    f.tick(0.1);
    assert!(f.deck().is_none(), "a rising jumper was snapped to the deck");
}

#[test]
fn a_descending_jump_lands_on_the_deck() {
    let Some(mut f) = DeckApp::new(Vec2::ZERO, false, [0; 3], 1.3977, 1.3977) else {
        return;
    };
    let height = f.floor.height(0.0, 0.0, 100.0).expect("a floor point");
    {
        let world = f.app.world_mut();
        let mut player = world.entity_mut(f.player);
        let mut mover = player.get_mut::<Mover>().unwrap();
        mover.pos.z = 10.0 + height + 0.3;
        mover.grounded = false;
        mover.vz = -3.0;
    }
    f.tick(0.1);
    assert!(f.deck().is_some(), "a descending jumper over the deck does not board");
    for _ in 0..10 {
        f.tick(0.1);
    }
    let mover = f.mover();
    assert!(f.deck().is_some(), "the descending jumper left the deck");
    assert!(mover.grounded, "the descending jumper did not land");
    assert!(
        (mover.pos.z - (10.0 + height)).abs() < 1e-3,
        "landed at {:?}",
        mover.pos.z
    );
}

#[test]
fn a_deck_leave_carries_only_the_ship_horizontal_speed() {
    if !aa_cry::pak_available() {
        return;
    }
    let leave_dx = |ship_velocity: [i16; 3]| -> Option<f32> {
        let probe = DeckApp::new(Vec2::ZERO, false, [0; 3], 0.0, 0.0)?;
        let (_, max) = probe.floor.bounds()?;
        let at = (0..50).find_map(|i| {
            let x = max.x - i as f32 * 0.1;
            probe.floor.height(x, 0.0, 100.0).map(|_| Vec2::new(x, 0.0))
        })?;
        let db = Db::ship_fixture(DECK_MODEL, ROWBOAT_PREFAB)?;
        let floor = db.ship_floor(DECK_MODEL)?;
        let terrain = TerrainQuery::new("main_world");
        let ground = terrain.height_at(15_575.0, 15_375.0)?;
        let mut f = DeckApp::with_floor_at(
            Vec3::new(15_575.0, 15_375.0, ground),
            db,
            floor,
            at,
            true,
            ship_velocity,
            0.0,
            -std::f32::consts::FRAC_PI_2,
        )?;
        f.add_on_foot(
            terrain.clone(),
            WorldCollision::new("main_world", terrain),
            WaterQuery::new("main_world"),
        );
        f.app
            .world_mut()
            .resource_mut::<SendState>()
            .settle_for_test();
        f.hold("moveforward");
        for _ in 0..10 {
            f.tick(0.1);
            if f.deck().is_none() {
                break;
            }
        }
        if f.deck().is_some() {
            return None;
        }
        // the exit frame is the deck's walking step; the on-foot controller's
        // first frame turns the step off the rail into the fall
        assert!(f.mover().grounded, "a walk-off started as a fall");
        f.tick(0.1);
        assert!(!f.mover().grounded, "the leave did not start an air fall");
        let left = f.mover().pos;
        f.tick(0.1);
        Some(f.mover().pos.x - left.x)
    };
    let still = leave_dx([0; 3]).expect("leaving a still hull");
    let moving = leave_dx([3277, 0, 0]).expect("leaving a moving hull");
    // the spawn point is shallow (wading factor ~0.75), so the keys alone move ~0.4 m;
    // the bug added that same walk speed to the carry and doubled it.
    assert!(
        (0.3..0.55).contains(&still),
        "a still hull's leave carried the walk speed twice: {:.2} m in 0.1 s",
        still
    );
    assert!(
        (moving - still - 0.3).abs() < 0.06,
        "a moving hull's leave did not add only its 3 m/s: {:.2} vs {:.2} m in 0.1 s",
        moving,
        still
    );
}

#[test]
fn a_step_off_a_moving_hull_carries_its_speed_until_landing() {
    let Some(probe) = DeckApp::new(Vec2::ZERO, false, [0; 3], 0.0, 0.0) else {
        return;
    };
    let (_, max) = probe.floor.bounds().expect("rowboat bounds");
    let at = (0..50)
        .find_map(|i| {
            let x = max.x - i as f32 * 0.1;
            probe.floor.height(x, 0.0, 100.0).map(|_| Vec2::new(x, 0.0))
        })
        .expect("a floor point at the rail");
    let terrain = TerrainQuery::new("main_world");
    let (x, y) = (15_575.0, 15_375.0);
    let Some(ground) = terrain.height_at(x, y) else {
        return;
    };
    let db = Db::ship_fixture(DECK_MODEL, ROWBOAT_PREFAB).expect("the temp ship catalog");
    let floor = db.ship_floor(DECK_MODEL).expect("the rowboat hull floor");
    let root = Vec3::new(x, y, ground);
    let Some(mut f) = DeckApp::with_floor_at(
        root,
        db,
        floor,
        at,
        true,
        [3277, 0, 0],
        0.0,
        -std::f32::consts::FRAC_PI_2,
    ) else {
        return;
    };
    f.add_on_foot(
        terrain.clone(),
        WorldCollision::new("main_world", terrain.clone()),
        WaterQuery::new("main_world"),
    );
    f.app
        .world_mut()
        .resource_mut::<SendState>()
        .settle_for_test();
    f.hold("moveforward");
    for _ in 0..10 {
        f.tick(0.1);
        if f.deck().is_none() {
            break;
        }
    }
    f.release("moveforward");
    assert!(f.deck().is_none(), "the player did not leave the rail");
    let left = f.mover().pos;
    let mut landed_at = None;
    for _ in 0..20 {
        f.tick(0.1);
        if f.mover().grounded {
            landed_at = Some(f.mover().pos);
            break;
        }
    }
    let landed = landed_at.expect("the carried fall did not land on the terrain");
    let carried = landed.truncate().distance(left.truncate());
    assert!(carried > 0.1, "the ship speed was lost at the leave: {carried} m");
    assert!(
        f.app
            .world()
            .get::<CarriedVelocity>(f.player)
            .is_none_or(|carried| carried.0.length() < 1e-3),
        "the carried speed survived the landing"
    );
    let after = f.mover().pos;
    f.tick(0.2);
    assert!(
        f.mover().pos.truncate().distance(after.truncate()) < 0.02,
        "the player kept moving after landing"
    );
}

/// A rider at the rowboat's rail on the main_world shore, with the on-foot
/// controller after the deck one, as the game orders them.
fn rail_app() -> Option<DeckApp> {
    let probe = DeckApp::new(Vec2::ZERO, false, [0; 3], 0.0, 0.0)?;
    let (_, max) = probe.floor.bounds().expect("rowboat bounds");
    let at = (0..50)
        .find_map(|i| {
            let x = max.x - i as f32 * 0.1;
            probe.floor.height(x, 0.0, 100.0).map(|_| Vec2::new(x, 0.0))
        })
        .expect("a floor point at the rail");
    let terrain = TerrainQuery::new("main_world");
    let (x, y) = (15_575.0, 15_375.0);
    let ground = terrain
        .height_at(x, y)
        .expect("the main_world shore terrain");
    let db = Db::ship_fixture(DECK_MODEL, ROWBOAT_PREFAB).expect("the temp ship catalog");
    let floor = db.ship_floor(DECK_MODEL).expect("the rowboat hull floor");
    let mut f = DeckApp::with_floor_at(
        Vec3::new(x, y, ground),
        db,
        floor,
        at,
        true,
        [0; 3],
        0.0,
        -std::f32::consts::FRAC_PI_2,
    )?;
    f.add_on_foot(
        terrain.clone(),
        WorldCollision::new("main_world", terrain),
        WaterQuery::new("main_world"),
    );
    f.app
        .world_mut()
        .resource_mut::<SendState>()
        .settle_for_test();
    Some(f)
}

#[test]
fn the_walk_off_frame_moves_the_player_once() {
    let Some(mut f) = rail_app() else {
        return;
    };
    let dt = 0.1;
    let step = ground_velocity(&ActorParams::default(), Vec2::Y, false, 1.0, 0.0).length() * dt;
    f.hold("moveforward");
    let mut before = f.mover().pos;
    for _ in 0..10 {
        before = f.mover().pos;
        f.moves();
        f.tick(dt);
        if f.deck().is_none() {
            break;
        }
    }
    assert!(f.deck().is_none(), "the player did not walk off");
    let moved = f.mover().pos - before;
    assert!(
        (moved.truncate().length() - step).abs() < 0.01,
        "the exit frame moved {:.3} m, one step is {:.3} m",
        moved.truncate().length(),
        step
    );
    assert!(
        moved.z.abs() < 1e-4,
        "the exit frame also fell {:.4} m",
        moved.z
    );
    assert!(
        f.moves().iter().all(|m| m.support.is_none()),
        "the exit frame reported the deck"
    );
    // the on-foot controller takes over on the next frame: it starts the fall
    // and reports the unparented movement at once
    f.tick(dt);
    assert!(!f.mover().grounded, "the step off the rail did not fall");
    let next = f.moves();
    assert!(
        !next.is_empty() && next.iter().all(|m| m.support.is_none()),
        "no on-foot movement after the leave: {next:?}"
    );
}

#[test]
fn the_jump_off_frame_applies_gravity_once() {
    let Some(mut f) = rail_app() else {
        return;
    };
    let dt = 0.1;
    let actor = ActorParams::default();
    let step = ground_velocity(&actor, Vec2::Y, false, 1.0, 0.0).length() * dt;
    let start = f.mover().pos;
    f.hold("moveforward");
    f.tap("jump");
    f.tick(dt);
    assert!(f.deck().is_none(), "the jump off the rail leaves the deck");
    let (dz, vz) = air_step(0.0, actor.jump_speed(), actor.gravity, dt, f32::INFINITY);
    let (pos, left_vz) = (f.mover().pos, f.mover().vz);
    assert!(
        (left_vz - vz).abs() < 1e-4,
        "vz {:.4} after the jump-off frame, one air step gives {:.4}",
        left_vz,
        vz
    );
    assert!(
        (pos.z - (start.z + dz)).abs() < 1e-4,
        "rose {:.4} m, one air step rises {:.4} m",
        pos.z - start.z,
        dz
    );
    assert!(
        ((pos - start).truncate().length() - step).abs() < 0.01,
        "the jump-off frame moved {:.3} m, one step is {:.3} m",
        (pos - start).truncate().length(),
        step
    );
    assert!(
        f.app.world().resource::<LocalJump>().rising,
        "the jump-off lost its take-off"
    );
    // the on-foot controller continues the same jump on the next frame
    f.tick(dt);
    assert!(!f.mover().grounded, "the jump-off landed at once");
    assert!(
        f.mover().vz < left_vz,
        "the next frame did not continue the fall"
    );
    assert!(
        f.app.world().resource::<LocalJump>().rising,
        "the on-foot controller dropped the take-off"
    );
}

/// Every `CSStopCasting` the frames so far sent (draining all commands).
fn stop_castings(f: &mut DeckApp) -> usize {
    f.app
        .world_mut()
        .resource_mut::<Messages<NetCommand>>()
        .drain()
        .filter(|NetCommand(command)| matches!(command, Command::StopCasting { .. }))
        .count()
}

#[test]
fn a_cast_during_a_deck_jump_ends_when_the_keys_move_the_jumper() {
    // the casts' cancel rules come from the skill catalog
    if !aa_anim::db::GameDb::available() {
        return;
    }
    let Some(mut f) = DeckApp::new(Vec2::ZERO, true, [0; 3], 1.3977, 1.3977) else {
        return;
    };
    f.app
        .init_resource::<ClipLibrary>()
        .init_resource::<aa_net::state::Doodads>();
    let dt = 0.05;
    f.tap("jump");
    f.tick(dt);
    assert!(!f.mover().grounded, "the deck jump did not take off");
    // the cast starts in the air, after the jump press
    crate::ingame::combat::fixture_cancel_casts(&mut f.app, 1);
    // the game runs the cancellation after the movement controllers
    f.app.configure_sets(
        Update,
        crate::ingame::combat::InputCancellation.after(deck_control),
    );
    stop_castings(&mut f);
    f.tick(dt);
    assert!(f.deck().is_some() && !f.mover().grounded);
    assert!(
        !f.app.world().resource::<PlayerInput>().moving,
        "a jumper standing still in the air counts as moving"
    );
    assert_eq!(
        stop_castings(&mut f),
        0,
        "standing still in the air cancelled the cast"
    );
    f.hold("moveforward");
    let before = f.mover().pos;
    f.tick(dt);
    assert!(
        f.deck().is_some() && !f.mover().grounded,
        "the jumper landed or left"
    );
    assert!(
        f.mover().pos.truncate().distance(before.truncate()) > 0.1,
        "the keys did not move the jumper"
    );
    assert!(
        f.app.world().resource::<PlayerInput>().moving,
        "keys moving the jumper are no movement"
    );
    assert_eq!(
        stop_castings(&mut f),
        2,
        "moving in the air did not cancel the casts"
    );
}

#[test]
fn a_dead_stat_stops_boarding_and_the_deck_ride() {
    let dead_stats = |dead: bool| PlayerStats {
        obj_id: 1,
        dead,
        ..PlayerStats::default()
    };
    // the death event's stats, before combat adds `Dead`: no boarding
    let Some(mut f) = DeckApp::new(Vec2::ZERO, false, [0; 3], 1.3977, 1.3977) else {
        return;
    };
    f.app.insert_resource(dead_stats(true));
    f.tick(0.1);
    assert!(f.deck().is_none(), "a dead player boarded the deck");
    f.app.insert_resource(dead_stats(false));
    f.tick(0.1);
    assert!(f.deck().is_some(), "the living player did not board");
    // a rider whose stats die drops the ride without moving or reporting
    let Some(mut f) = DeckApp::new(Vec2::ZERO, true, [0; 3], 1.3977, 1.3977) else {
        return;
    };
    f.app.insert_resource(dead_stats(true));
    f.hold("moveforward");
    let before = f.mover().pos;
    f.moves();
    f.tick(0.1);
    assert!(f.deck().is_none(), "a dead player kept the deck ride");
    assert_eq!(f.mover().pos, before, "a dead player walked the deck");
    assert!(f.moves().is_empty(), "a dead player reported deck movement");
}

#[test]
fn a_fast_fall_is_swept_onto_the_deck() {
    let Some(mut f) = DeckApp::new(Vec2::ZERO, false, [0; 3], 1.3977, 1.3977) else {
        return;
    };
    let height = f.floor.height(0.0, 0.0, 100.0).expect("a floor point");
    {
        let world = f.app.world_mut();
        let mut player = world.entity_mut(f.player);
        let mut mover = player.get_mut::<Mover>().unwrap();
        // 2 m above the floor, falling 40 m/s: the next frame is 2 m below it
        mover.pos.z = 10.0 + height + 2.0;
        mover.grounded = false;
        mover.vz = -40.0;
    }
    f.tick(0.1);
    f.tick(0.1);
    assert!(f.deck().is_some(), "the swept fall did not board");
    let mover = f.mover();
    assert!(mover.grounded, "the swept fall did not land");
    assert!(
        (mover.pos.z - (10.0 + height)).abs() < 1e-3,
        "landed at {:?}",
        mover.pos.z
    );
}

#[test]
fn a_deck_jump_and_walk_update_local_jump() {
    let Some(mut f) = DeckApp::new(Vec2::ZERO, true, [0; 3], 1.3977, 1.3977) else {
        return;
    };
    f.hold("moveforward");
    f.tick(0.1);
    let walking = *f.app.world().resource::<LocalJump>();
    assert!(walking.dir.y > 0.9 && !walking.rising, "{walking:?}");
    f.tap("jump");
    f.tick(0.1);
    let jumping = *f.app.world().resource::<LocalJump>();
    assert!(jumping.rising, "a deck take-off is not rising: {jumping:?}");
    f.release("moveforward");
    for _ in 0..20 {
        f.tick(0.1);
    }
    let landed = *f.app.world().resource::<LocalJump>();
    assert!(!landed.rising, "landing keeps rising: {landed:?}");
    f.app
        .world_mut()
        .entity_mut(f.player)
        .insert(Dead { at: 0.0 });
    f.tick(0.1);
    assert_eq!(
        *f.app.world().resource::<LocalJump>(),
        LocalJump::default(),
        "dropping the ride clears LocalJump"
    );
}

#[test]
fn an_airborne_step_against_a_riser_keeps_the_ride() {
    let quad = |x0: f32, x1: f32, y0: f32, y1: f32, z: f32| {
        [
            [
                Vec3::new(x0, y0, z),
                Vec3::new(x1, y0, z),
                Vec3::new(x1, y1, z),
            ],
            [
                Vec3::new(x0, y0, z),
                Vec3::new(x1, y1, z),
                Vec3::new(x0, y1, z),
            ],
        ]
    };
    let floor = Arc::new(ShipFloor::new(
        quad(-2.0, 2.0, -2.0, 2.0, 0.0)
            .into_iter()
            .chain(quad(-2.0, 2.0, 0.5, 1.5, 0.8)),
    ));
    let db = Db::ship_floor_fixture(DECK_MODEL, floor.clone()).expect("the synthetic ship catalog");
    let Some(mut f) = DeckApp::with_floor(db, floor, Vec2::new(0.0, 0.2), true, [0; 3], 0.0, 0.0)
    else {
        return;
    };
    f.hold("moveforward");
    f.tap("jump");
    f.tick(0.1);
    assert!(f.deck().is_some(), "the riser ahead keeps the ride");
    assert!(
        (f.mover().pos.y - 200.2).abs() < 1e-3,
        "the step into the riser is blocked"
    );
    f.release("moveforward");
    for _ in 0..20 {
        f.tick(0.1);
    }
    assert!(f.deck().is_some(), "the jump lands back on the deck");
    assert!(f.mover().grounded);
}

#[test]
fn a_grounded_step_against_a_riser_keeps_the_ride() {
    let quad = |x0: f32, x1: f32, y0: f32, y1: f32, z: f32| {
        [
            [Vec3::new(x0, y0, z), Vec3::new(x1, y0, z), Vec3::new(x1, y1, z)],
            [Vec3::new(x0, y0, z), Vec3::new(x1, y1, z), Vec3::new(x0, y1, z)],
        ]
    };
    let floor = Arc::new(ShipFloor::new(
        quad(-2.0, 2.0, -2.0, 2.0, 0.0)
            .into_iter()
            .chain(quad(-2.0, 2.0, 0.5, 1.5, 0.8)),
    ));
    let db = Db::ship_floor_fixture(DECK_MODEL, floor.clone()).expect("the synthetic ship catalog");
    let Some(mut f) = DeckApp::with_floor(db, floor, Vec2::new(0.0, 0.2), true, [0; 3], 0.0, 0.0)
    else {
        return;
    };
    f.hold("moveforward");
    for _ in 0..5 {
        f.tick(0.1);
    }
    assert!(f.deck().is_some(), "the riser keeps the ride");
    assert!(
        (f.mover().pos.y - 200.2).abs() < 1e-3,
        "the grounded step into the riser is blocked: {:?}",
        f.mover().pos
    );
}

#[test]
fn a_grounded_step_walks_under_a_roof() {
    let quad = |x0: f32, x1: f32, y0: f32, y1: f32, z: f32| {
        [
            [Vec3::new(x0, y0, z), Vec3::new(x1, y0, z), Vec3::new(x1, y1, z)],
            [Vec3::new(x0, y0, z), Vec3::new(x1, y1, z), Vec3::new(x0, y1, z)],
        ]
    };
    // the deck at 0 and a roof well above head height (5 m) over it
    let floor = Arc::new(ShipFloor::new(
        quad(-2.0, 2.0, -2.0, 2.0, 0.0)
            .into_iter()
            .chain(quad(-2.0, 2.0, -2.0, 2.0, 5.0)),
    ));
    let db = Db::ship_floor_fixture(DECK_MODEL, floor.clone()).expect("the synthetic ship catalog");
    let Some(mut f) = DeckApp::with_floor_at_height(
        Vec3::new(100.0, 200.0, 10.0),
        Some(0.0),
        db,
        floor,
        Vec2::new(0.0, 0.2),
        true,
        [0; 3],
        0.0,
        0.0,
    ) else {
        return;
    };
    f.hold("moveforward");
    f.tick(0.1);
    assert!(f.deck().is_some(), "the roof lifted the ride");
    assert!(
        (f.mover().pos.z - 10.0).abs() < 0.01,
        "the walk started on the roof, not the lower deck: {:?}",
        f.mover().pos
    );
    assert!(
        (f.mover().pos.y - 200.74).abs() < 0.05,
        "the walk under the roof was blocked: {:?}",
        f.mover().pos
    );
}

#[test]
fn a_world_wall_blocks_a_deck_step() {
    if !aa_cry::pak_available() {
        return;
    }
    // a real harbour wall from the world collision (Lutesong's cell 20,10)
    let terrain = TerrainQuery::new("main_world");
    let collision = WorldCollision::new("main_world", terrain.clone());
    if collision.build_blocking(20, 10).is_none() {
        return;
    }
    let (x, y) = (21_131.0, 10_594.0);
    let z = terrain.height_at(x, y).unwrap_or(0.0);
    let mut found = None;
    for triangle in collision.triangles_near(Vec3::new(x, y, z + 3.0), 40.0, Mask::STRUCTURES) {
        let normal = (triangle[1] - triangle[0]).cross(triangle[2] - triangle[0]);
        if normal.length() < 1e-4 {
            continue;
        }
        let normal = normal.normalize();
        let (lo, hi) = triangle
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), point| {
                (lo.min(point.z), hi.max(point.z))
            });
        if normal.z.abs() > 0.3 || hi - lo < 2.5 || Vec2::new(normal.x, normal.y).length() < 0.1 {
            continue;
        }
        found = Some((triangle, normal, lo, hi));
        break;
    }
    let Some((triangle, normal, lo, hi)) = found else {
        return;
    };
    let db = Db::ship_fixture(DECK_MODEL, ROWBOAT_PREFAB).expect("the temp ship catalog");
    let floor = db.ship_floor(DECK_MODEL).expect("the rowboat hull floor");
    let height = floor.height(0.0, 0.0, 100.0).expect("a floor point");
    let plane = triangle.iter().sum::<Vec3>() / 3.0;
    let mut root = plane + normal * 1.0;
    root.z = (lo + hi) / 2.0 - height;
    let into = -Vec2::new(normal.x, normal.y).normalize();
    // the hull faces `into` (game yaw: forward = (-sin, cos))
    let ship_yaw = (-into.x).atan2(into.y);
    let Some(mut f) = DeckApp::with_floor_at(
        root,
        db,
        floor,
        Vec2::ZERO,
        true,
        [0; 3],
        ship_yaw,
        ship_yaw,
    ) else {
        return;
    };
    *f.app.world_mut().resource_mut::<WorldCollision>() = collision.clone();
    f.hold("moveforward");
    for _ in 0..6 {
        f.tick(0.1);
    }
    let feet = f.mover().pos;
    let normal2 = Vec2::new(normal.x, normal.y);
    let distance = (feet.truncate() - plane.truncate()).dot(normal2);
    assert!(
        (0.25..1.0).contains(&distance),
        "the deck walk did not stop at the world wall: {feet:?}, plane distance {distance}"
    );
    assert!(f.deck().is_some(), "the wall took the deck ride away");
}

#[test]
fn a_small_deck_step_down_stays_grounded_and_a_big_one_falls() {
    let quad = |x0: f32, x1: f32, y0: f32, y1: f32, z: f32| {
        [
            [Vec3::new(x0, y0, z), Vec3::new(x1, y0, z), Vec3::new(x1, y1, z)],
            [Vec3::new(x0, y0, z), Vec3::new(x1, y1, z), Vec3::new(x0, y1, z)],
        ]
    };
    let deck = |upper: f32| {
        Arc::new(ShipFloor::new(
            quad(-2.0, 2.0, 0.0, 4.0, 0.0)
                .into_iter()
                .chain(quad(-2.0, 2.0, -4.0, 0.0, upper)),
        ))
    };
    // a 0.3 m step down is walked, like the on-foot DROP
    let floor = deck(0.3);
    let db = Db::ship_floor_fixture(DECK_MODEL, floor.clone()).expect("the synthetic ship catalog");
    let Some(mut f) = DeckApp::with_floor(db, floor, Vec2::new(0.0, -0.5), true, [0; 3], 0.0, 0.0)
    else {
        return;
    };
    f.hold("moveforward");
    f.tick(0.1);
    f.release("moveforward");
    assert!(f.mover().grounded, "a 0.3 m step down left the ground");
    assert!(
        (f.mover().pos.z - 10.0).abs() < 0.01,
        "the 0.3 m step snapped to {:?}",
        f.mover().pos.z
    );
    // a 3 m drop starts a deck fall and lands on the lower floor
    let floor = deck(3.0);
    let db = Db::ship_floor_fixture(DECK_MODEL, floor.clone()).expect("the synthetic ship catalog");
    let Some(mut f) = DeckApp::with_floor(db, floor, Vec2::new(0.0, -0.5), true, [0; 3], 0.0, 0.0)
    else {
        return;
    };
    f.hold("moveforward");
    f.tick(0.1);
    f.release("moveforward");
    assert!(!f.mover().grounded, "a 3 m drop did not fall");
    let mut impacts = Vec::new();
    for _ in 0..12 {
        f.tick(0.1);
        impacts.extend(f.moves().iter().map(|movement| movement.fall_vel));
    }
    assert!(f.deck().is_some(), "the 3 m drop left the hull");
    assert!(f.mover().grounded, "the 3 m drop did not land");
    assert!(
        (f.mover().pos.z - 10.0).abs() < 0.01,
        "the 3 m drop landed at {:?}",
        f.mover().pos.z
    );
    assert!(
        impacts.iter().any(|&impact| impact > 0),
        "the 3 m drop did not report its landing: {impacts:?}"
    );
}

#[test]
fn turning_the_deck_turns_the_camera_and_the_facing_together() {
    let Some(mut f) = DeckApp::new(Vec2::ZERO, true, [0; 3], 1.3977, 1.3977) else {
        return;
    };
    let before_facing = f.mover().facing;
    let before_camera = f.orbit().yaw;
    f.hold("turnleft");
    f.tick(0.1);
    let (facing, camera) = (f.mover().facing, f.orbit().yaw);
    assert!(facing - before_facing > 0.04, "A turns left");
    assert!((facing - before_facing - (camera - before_camera)).abs() < 1e-5);
    assert!(
        (facing - camera).abs() < 1e-5,
        "the character stays under the camera"
    );
}

#[test]
fn dead_seated_and_skill_moved_players_drop_the_deck_ride() {
    for state in 0..3 {
        let Some(mut f) = DeckApp::new(Vec2::ZERO, true, [0; 3], 1.3977, 1.3977) else {
            return;
        };
        match state {
            0 => {
                f.app
                    .world_mut()
                    .entity_mut(f.player)
                    .insert(Dead { at: 0.0 });
            }
            1 => {
                f.app
                    .world_mut()
                    .entity_mut(f.player)
                    .insert(Seated::fixture());
            }
            _ => {
                f.app
                    .world_mut()
                    .entity_mut(f.player)
                    .insert(SkillMove::fixture());
            }
        }
        f.tick(0.02);
        assert!(f.deck().is_none(), "state {state} drops the ride");
    }
}

#[test]
fn the_deck_walk_toggle_and_autorun_change_the_speed() {
    let Some(mut f) = DeckApp::new(Vec2::ZERO, true, [0; 3], 1.3977, 1.3977) else {
        return;
    };
    let actor = ActorParams::default();
    f.tap("toggle_walk");
    f.tick(0.1);
    f.hold("moveforward");
    let before = f.mover().pos;
    f.tick(0.1);
    let walk = (f.mover().pos - before).truncate().length();
    assert!(
        (walk - 5.4 * actor.walk * 0.1).abs() < 1e-2,
        "walking step {walk}"
    );
    f.release("moveforward");
    f.tap("toggle_walk");
    f.tick(0.1);
    f.hold("moveforward");
    let before = f.mover().pos;
    f.tick(0.1);
    let run = (f.mover().pos - before).truncate().length();
    assert!((run - 5.4 * 0.1).abs() < 1e-2, "running step {run}");
    assert!(walk < run * 0.6, "the walk toggle slows the deck walk");
    f.release("moveforward");
    f.app
        .world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::ShiftLeft);
    f.hold("moveforward");
    let before = f.mover().pos;
    f.tick(0.1);
    let shift = (f.mover().pos - before).truncate().length();
    assert!(
        (shift - walk).abs() < 1e-2,
        "Shift walks like the toggle ({shift})"
    );
    f.app
        .world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::ShiftLeft);
    f.release("moveforward");
    f.app.world_mut().resource_mut::<Sprint>().speed_mul = 1.5;
    f.hold("moveforward");
    let before = f.mover().pos;
    f.tick(0.1);
    let dash = (f.mover().pos - before).truncate().length();
    assert!(
        (dash - run * 1.5).abs() < 1e-2,
        "the buff multiplier scales the deck walk ({dash})"
    );

    let Some(mut f) = DeckApp::new(Vec2::ZERO, true, [0; 3], 1.3977, 1.3977) else {
        return;
    };
    f.tap("autorun");
    f.tick(0.1);
    let before = f.mover().pos;
    f.tick(0.1);
    let autorun = (f.mover().pos - before).truncate().length();
    assert!(
        autorun > 5.4 * 0.1 * 0.9,
        "autorun moves without keys ({autorun})"
    );
}

#[test]
fn a_mounted_player_does_not_board_or_keeps_the_ride() {
    for (drives, ship) in [(false, false), (true, false), (false, true), (true, true)] {
        let seat = LocalMount(Some(LocalSeat {
            mount: DECK_SHIP,
            point: if drives { 1 } else { 2 },
            drives,
            cart: false,
            ship,
            speed: Default::default(),
            lift: 0.0,
        }));
        let Some(mut f) = DeckApp::new(Vec2::ZERO, false, [0; 3], 1.3977, 1.3977) else {
            return;
        };
        f.app.insert_resource(seat);
        f.tick(0.1);
        assert!(
            f.deck().is_none(),
            "a seat (drives {drives}, ship {ship}) does not board"
        );
        let Some(mut f) = DeckApp::new(Vec2::ZERO, true, [0; 3], 1.3977, 1.3977) else {
            return;
        };
        f.app.insert_resource(seat);
        f.tick(0.1);
        assert!(
            f.deck().is_none(),
            "mounting drops the ride (drives {drives}, ship {ship})"
        );
    }
}

#[test]
fn a_camera_walk_crossing_pi_moves_toward_the_camera() {
    let (ship_yaw, camera_yaw) = (3.1, -3.1);
    let Some(mut f) = DeckApp::new(Vec2::ZERO, true, [0; 3], ship_yaw, camera_yaw) else {
        return;
    };
    f.hold("moveforward");
    let before = f.mover().pos;
    f.tick(0.1);
    let step = f.mover().pos - before;
    let want = forward(camera_yaw) * 5.4 * 0.1;
    assert!(
        (step.x - want.x).abs() < 1e-2 && (step.y - want.y).abs() < 1e-2,
        "{step:?} vs {want:?}"
    );
    assert!((forward(f.mover().facing) - forward(camera_yaw)).length() < 1e-4);
}

fn app_tick(app: &mut App, dt: f32) {
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(Duration::from_secs_f32(dt));
    app.update();
}

/// The deck systems with the real `control_player` and `track_local_mount`: the
/// player starts in a ship-seat get-off placement over the rowboat.
fn dismount_app() -> Option<(App, Entity)> {
    if !aa_cry::pak_available() {
        return None;
    }
    let db = Db::ship_fixture(DECK_MODEL, ROWBOAT_PREFAB).expect("the temp ship catalog");
    let floor = db.ship_floor(DECK_MODEL).expect("the rowboat hull floor");
    let root = Vec3::new(100.0, 200.0, 10.0);
    let yaw = 1.3977;
    let height = floor.height(0.0, 0.0, 100.0).expect("a floor point");
    let local = Vec3::new(0.0, 0.0, height);
    let world = ship_to_world(root, Quat::from_rotation_z(yaw), local);
    let mut units = Units::default();
    units
        .by_id
        .insert(DECK_SHIP, deck_ship_unit(root, yaw, [0; 3]));
    let terrain = TerrainQuery::new("test");
    let collision = WorldCollision::new("test", terrain.clone());
    let water = WaterQuery::new("test");
    let mut app = App::new();
    app.init_resource::<Time>()
        .init_resource::<crate::settings::Actions>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<AccumulatedMouseMotion>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<Player>()
        .init_resource::<PlayerInput>()
        .init_resource::<SendState>()
        .init_resource::<LocalMount>()
        .init_resource::<Sprint>()
        .init_resource::<LocalJump>()
        .init_resource::<crate::ingame::DevOptions>()
        .init_resource::<Views>()
        .init_resource::<ClipLibrary>()
        .init_resource::<MountReason>()
        .insert_resource(units)
        .insert_resource(db)
        .insert_resource(terrain)
        .insert_resource(collision)
        .insert_resource(water)
        .add_message::<NetEvent>()
        .add_message::<NetCommand>()
        .add_systems(
            Update,
            (track_local_mount, board_deck, deck_control, control_player).chain(),
        );
    app.world_mut().spawn(OrbitCamera { yaw, ..default() });
    let player = app
        .world_mut()
        .spawn((
            LocalPlayer { obj_id: 1 },
            Mover {
                pos: world,
                facing: yaw,
                vz: 0.0,
                grounded: true,
                velocity: Vec3::ZERO,
                swim: None,
                wading: 0.0,
            },
            Transform::default(),
            Motion::default(),
            ActorParams::default(),
            DismountMove::fixture(world),
        ))
        .id();
    app.world_mut()
        .resource_mut::<SendState>()
        .settle_for_test();
    Some((app, player))
}

#[test]
fn a_ship_dismount_finishes_before_the_deck_boards() {
    let Some((mut app, player)) = dismount_app() else {
        return;
    };
    let mut boarded = false;
    for _ in 0..30 {
        app_tick(&mut app, 0.1);
        let dismounting = app.world().get::<DismountMove>(player).is_some();
        let deck = app.world().get::<DeckRide>(player).is_some();
        if dismounting {
            assert!(!deck, "boarded while the get-off placement runs");
        }
        boarded |= deck;
    }
    assert!(
        app.world().get::<DismountMove>(player).is_none(),
        "the finished placement is removed"
    );
    assert!(boarded, "boarding resumes after the placement");
}

/// A real `track_local_mount` detach from the rowboat's helm seat (reason 5):
/// the seat state and the player's attachment are already set, then the server
/// detach event arrives. Returns the app, the player and the hull floor height.
fn ship_get_off_app() -> Option<(App, Entity, f32, Vec3)> {
    if !aa_cry::pak_available() {
        return None;
    }
    let db = Db::ship_fixture(DECK_MODEL, ROWBOAT_PREFAB).expect("the temp ship catalog");
    let floor = db.ship_floor(DECK_MODEL).expect("the rowboat hull floor");
    let root = Vec3::new(100.0, 200.0, 10.0);
    let yaw = 1.3977;
    let drawn = root + Vec3::new(0.4, -0.3, 0.05);
    let drawn_yaw = yaw + 0.04;
    let height = floor.height(0.0, 0.0, 100.0).expect("a floor point");
    let mut units = Units::default();
    units
        .by_id
        .insert(DECK_SHIP, deck_ship_unit(root, yaw, [0; 3]));
    let mut player_unit = deck_ship_unit(root, yaw, [0; 3]);
    player_unit.obj_id = 1;
    player_unit.kind = UnitKind::Character { char_id: 1 };
    player_unit.name = "Capturer".into();
    player_unit.attached_to = Some(Attachment {
        obj_id: DECK_SHIP,
        point: 1,
        reason: 1,
    });
    let player = Player {
        spawn: Some(PlayerSpawn {
            obj_id: 1,
            state: Default::default(),
            unit: player_unit,
        }),
    };
    let terrain = TerrainQuery::new("test");
    let collision = WorldCollision::new("test", terrain.clone());
    let water = WaterQuery::new("test");
    let mut app = App::new();
    app.init_resource::<Time>()
        .init_resource::<crate::settings::Actions>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<AccumulatedMouseMotion>()
        .init_resource::<ButtonInput<KeyCode>>()
        .insert_resource(player)
        .init_resource::<PlayerInput>()
        .init_resource::<SendState>()
        .insert_resource(LocalMount(Some(LocalSeat {
            mount: DECK_SHIP,
            point: 1,
            drives: true,
            cart: false,
            ship: true,
            speed: Default::default(),
            lift: 0.0,
        })))
        .init_resource::<Sprint>()
        .init_resource::<LocalJump>()
        .init_resource::<crate::ingame::DevOptions>()
        .init_resource::<Views>()
        .init_resource::<ClipLibrary>()
        .init_resource::<MountReason>()
        .insert_resource(units)
        .insert_resource(db)
        .insert_resource(terrain)
        .insert_resource(collision)
        .insert_resource(water)
        .add_message::<NetEvent>()
        .add_message::<NetCommand>()
        .add_systems(
            Update,
            (track_local_mount, board_deck, deck_control).chain(),
        );
    app.world_mut().spawn(OrbitCamera { yaw, ..default() });
    let mut hull = RemoteUnit::fixture(DECK_SHIP, drawn);
    hull.facing = drawn_yaw;
    hull.game_rotation = Quat::from_rotation_z(drawn_yaw);
    hull.rotation = aa_world::coords::cry_rot_to_bevy(hull.game_rotation);
    let hull_entity = app.world_mut().spawn(hull).id();
    let ship_unit = deck_ship_unit(root, yaw, [0; 3]);
    app.world_mut()
        .resource_mut::<Views>()
        .fixture_actor(&ship_unit, Some(hull_entity));
    let player = app
        .world_mut()
        .spawn((
            LocalPlayer { obj_id: 1 },
            Mover {
                pos: drawn,
                facing: drawn_yaw,
                vz: 0.0,
                grounded: true,
                velocity: Vec3::ZERO,
                swim: None,
                wading: 0.0,
            },
            Transform::default(),
            Motion::default(),
            ActorParams::default(),
        ))
        .id();
    app.world_mut()
        .resource_mut::<SendState>()
        .settle_for_test();
    app.world_mut()
        .resource_mut::<Player>()
        .spawn
        .as_mut()
        .unwrap()
        .unit
        .attached_to = None;
    app.world_mut()
        .write_message(NetEvent(aa_net::Event::UnitDetached {
            obj_id: 1,
            reason: 5,
        }));
    app_tick(&mut app, 0.1);
    Some((app, player, height, drawn))
}

#[test]
fn the_scripted_clock_advances_while_on_the_deck() {
    let Some((mut app, player)) = dismount_app() else {
        return;
    };
    app.world_mut().entity_mut(player).remove::<DismountMove>();
    app.world_mut().entity_mut(player).insert(DeckRide::fixture(
        DECK_SHIP,
        Vec3::new(0.0, 0.0, 0.9),
        0.9,
        0.0,
        false,
        0.0,
        1.3977,
    ));
    let before = app.world().resource::<SendState>().since_spawn();
    for _ in 0..3 {
        app_tick(&mut app, 0.1);
    }
    let after = app.world().resource::<SendState>().since_spawn();
    assert!(
        (after - before - 0.3).abs() < 0.05,
        "scripted clock {before} -> {after} over 0.3 s on the deck"
    );
}

#[test]
fn a_ship_get_off_lands_on_the_deck_without_a_placement() {
    let Some((mut app, player, height, drawn)) = ship_get_off_app() else {
        return;
    };
    assert!(
        app.world().get::<DeckRide>(player).is_some(),
        "the get-off over the hull starts the deck ride"
    );
    assert!(
        app.world().get::<DismountMove>(player).is_none(),
        "no get-off placement may remain once the ride starts"
    );
    let landed = app.world().get::<Mover>(player).unwrap().pos;
    assert!(
        (landed.z - (drawn.z + height)).abs() < 0.05,
        "landed at {landed:?}, drawn floor {}",
        drawn.z + height
    );
    assert!(
        (landed.x - drawn.x).abs() < 0.05 && (landed.y - drawn.y).abs() < 0.05,
        "the get-off used the wire hull, not the drawn seat: {landed:?} vs drawn {drawn:?}"
    );
    let mut support = false;
    for _ in 0..6 {
        app_tick(&mut app, 0.1);
        assert!(
            app.world().get::<DismountMove>(player).is_none(),
            "a placement came back while the ride runs"
        );
        let moves: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .filter_map(|NetCommand(command)| match command {
                Command::Move(m) => Some(m),
                _ => None,
            })
            .collect();
        support |= moves.iter().any(|m| {
            let m = m.to_movement(0);
            m.flags == 0x40 && m.actor_flags == 0x24 && m.support.is_some()
        });
    }
    assert!(support, "the landed deck does not send the 0x40/0x24 block");
    assert!(
        app.world().get::<DeckRide>(player).is_some(),
        "the ride is still there after the get-off"
    );
    assert!(
        app.world().get::<Mover>(player).unwrap().swim.is_none(),
        "the get-off fell into the water instead of the deck"
    );
}

#[test]
fn a_glider_slide_off_a_raised_deck_falls_and_lands() {
    let quad = |x0: f32, x1: f32, y0: f32, y1: f32, z: f32| {
        [
            [Vec3::new(x0, y0, z), Vec3::new(x1, y0, z), Vec3::new(x1, y1, z)],
            [Vec3::new(x0, y0, z), Vec3::new(x1, y1, z), Vec3::new(x0, y1, z)],
        ]
    };
    // upper deck y -3..0 at z 3, lower deck y 0..6 at z 0: a 3 m drop
    let floor = Arc::new(ShipFloor::new(
        quad(-2.0, 2.0, -3.0, 0.0, 3.0)
            .into_iter()
            .chain(quad(-2.0, 2.0, 0.0, 6.0, 0.0)),
    ));
    let Some(db) = Db::ship_floor_fixture(DECK_MODEL, floor.clone()) else {
        return;
    };
    let root = Vec3::new(100.0, 200.0, 10.0);
    let Some(mut f) = DeckApp::with_floor_at_height(
        root,
        Some(3.0),
        db,
        floor,
        Vec2::new(0.0, -1.0),
        true,
        [0; 3],
        0.0,
        0.0,
    ) else {
        return;
    };
    let contact = DeckContact {
        ship: DECK_SHIP,
        local: Vec3::new(0.0, -1.0, 3.0),
        root,
        rotation: Quat::IDENTITY,
        facing: 0.0,
    };
    let mut flight = Flight::start(GlideParams::EXPERIMENTAL);
    flight.elapsed = 10.0;
    flight.smoothed_v = -1.5;
    flight.smoothed_dv = 0.0;
    flight.vz = -1.5;
    assert_eq!(flight.stop(-1.0, Some(0.0)), Phase::Sliding);
    f.app.world_mut().resource_mut::<Gliding>().active = Some(Active {
        flight,
        buff_id: 1029,
        buff_index: 1,
        pose: Pose::Slide,
        touched: false,
        touched_for: 0.0,
        missing: 0.0,
        water: false,
        deck: Some(contact),
    });
    f.app
        .world_mut()
        .entity_mut(f.player)
        .insert(DeckRide::new(DECK_SHIP, contact.local, contact.facing, contact.yaw()));
    let mut airborne = false;
    let mut ground_while_airborne = false;
    let mut impact = None;
    for _ in 0..180 {
        f.tick(1.0 / 60.0);
        let grounded = f.mover().grounded;
        if !grounded {
            airborne = true;
        }
        for movement in f.moves() {
            if !grounded && movement.state == aa_net::MoveState::Ground {
                ground_while_airborne = true;
            }
            if movement.fall_vel > 0 {
                impact = Some(movement.fall_vel);
            }
        }
        if impact.is_some() && f.mover().grounded {
            break;
        }
    }
    assert!(airborne, "the slide over the 3 m drop never left the upper deck");
    assert!(
        !ground_while_airborne,
        "a ground packet was sent while the glider was falling"
    );
    assert!(impact.is_some(), "the deck landing did not report an impact");
    assert!(f.mover().grounded, "the deck fall never landed");
    assert!(f.deck().is_some(), "the landed glider left the deck");
}
