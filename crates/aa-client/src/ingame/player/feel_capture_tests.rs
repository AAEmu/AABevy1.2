//! The measured 1.2 movement of `handoff/2026-11-02-feel-capture-1.md`, pinned on the real controller.

use super::*;
use crate::ingame::data::MountSpeed;
use crate::ingame::mount::{LocalMount, LocalSeat, MountRear};
use crate::settings::Actions;
use std::time::Duration;

/// `control_player` over the real `main_world`, at a chosen point: the movement rules, the jump, the
/// packets and the slope probe are what matter here.
struct Feel {
    app: App,
    player: Entity,
    camera: Entity,
}

impl Feel {
    fn new() -> Feel {
        // a flat, dry spot of the captured area (the capture's start, 15512/15306, is on a slope)
        Feel::at(15480.0, 15282.0)
    }

    fn at(x: f32, y: f32) -> Feel {
        let terrain = TerrainQuery::new("main_world");
        let z = terrain.ground_at(x, y).map_or(134.86328, |g| g.height);
        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<Actions>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<PlayerInput>()
            .init_resource::<SendState>()
            .init_resource::<LocalJump>()
            .init_resource::<DevOptions>()
            .insert_resource(terrain.clone())
            .insert_resource(WaterQuery::new("main_world"))
            .insert_resource(WorldCollision::new("main_world", terrain.clone()))
            .add_message::<NetCommand>()
            .add_message::<MountRear>()
            .add_systems(Update, control_player);
        let camera = app.world_mut().spawn(OrbitCamera::default()).id();
        let player = app
            .world_mut()
            .spawn((
                LocalPlayer { obj_id: 1 },
                Mover {
                    pos: Vec3::new(x, y, z),
                    facing: 0.0,
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
        Feel {
            app,
            player,
            camera,
        }
    }

    fn hold(&mut self, action: &str) {
        self.app
            .world_mut()
            .resource_mut::<Actions>()
            .script_hold(action);
    }

    fn release(&mut self, action: &str) {
        self.app
            .world_mut()
            .resource_mut::<Actions>()
            .script_release(action);
    }

    fn tap(&mut self, action: &str) {
        self.app
            .world_mut()
            .resource_mut::<Actions>()
            .tap_for_test(action);
    }

    fn walk(&mut self, on: bool) {
        self.app
            .world_mut()
            .resource_mut::<DevOptions>()
            .script_keys = if on { "H".into() } else { String::new() };
    }

    fn mount(&mut self, speed: MountSpeed) {
        self.app.insert_resource(LocalMount(Some(LocalSeat {
            mount: 9,
            point: 1,
            drives: true,
            cart: false,
            ship: false,
            turret: false,
            speed,
            lift: 0.0,
        })));
    }

    fn tick(&mut self, dt: f32) -> Vec<MoveCommand> {
        self.tick_named(dt).into_iter().map(|(_, movement)| movement).collect()
    }

    /// One step's movement packets with the unit each names: `Some(mate)` for `CSMoveUnit`, `None` for the own `CSMove`.
    fn tick_named(&mut self, dt: f32) -> Vec<(Option<u32>, MoveCommand)> {
        self.step(dt);
        self.named_packets()
    }

    /// One step; its commands stay queued.
    fn step(&mut self, dt: f32) {
        self.app
            .world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs_f32(dt));
        self.app.update();
        self.app
            .world_mut()
            .resource_mut::<Actions>()
            .clear_taps_for_test();
    }

    fn packets(&mut self) -> Vec<MoveCommand> {
        self.named_packets().into_iter().map(|(_, movement)| movement).collect()
    }

    fn named_packets(&mut self) -> Vec<(Option<u32>, MoveCommand)> {
        self.app
            .world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .filter_map(|NetCommand(command)| match command {
                Command::Move(movement) => Some((None, movement)),
                Command::MoveUnit { obj_id, movement } => Some((Some(obj_id), movement)),
                _ => None,
            })
            .collect()
    }

    fn input(&self) -> PlayerInput {
        *self.app.world().resource::<PlayerInput>()
    }

    fn rears(&mut self) -> Vec<MountRear> {
        self.app.world_mut().resource_mut::<Messages<MountRear>>().drain().collect()
    }

    fn mover(&self) -> &Mover {
        self.app.world().get::<Mover>(self.player).unwrap()
    }

    fn orbit_yaw(&self) -> f32 {
        self.app
            .world()
            .get::<OrbitCamera>(self.camera)
            .unwrap()
            .yaw
    }

    fn since_spawn(&self) -> f32 {
        self.app.world().resource::<SendState>().since_spawn()
    }

    fn airborne(&mut self, vz: f32) {
        let mut mover = self.app.world_mut().get_mut::<Mover>(self.player).unwrap();
        mover.grounded = false;
        mover.vz = vz;
        // half a metre over the ground: one 0.1 s frame of the fall reaches it
        mover.pos.z += 0.5;
    }
}

fn speed(mover: &Mover) -> f32 {
    mover.velocity.truncate().length()
}

fn close(a: f32, b: f32, tolerance: f32) -> bool {
    (a - b).abs() <= tolerance
}

/// feel capture 1, on foot: run 5.40, walk 2.43, back 3.51 / 1.58, strafe 5.40, diagonal 5.36 (the
/// client normalises to 5.40; the capture's 5.36 is the same step within its 60 Hz sampling).
#[test]
fn on_foot_speeds_are_the_captured_ones_and_full_on_the_first_frame() {
    if !aa_cry::pak_available() {
        return;
    }
    let mut f = Feel::new();
    f.hold("moveforward");
    let packets = f.tick(0.1);
    assert!(
        close(speed(f.mover()), 5.40, 0.01),
        "run {}",
        speed(f.mover())
    );
    // the key-down packet has the pressed direction and the old (zero) velocity; the movement itself is
    // already at full speed on the same frame (no acceleration ramp)
    let first = packets.last().expect("the key-down packet").to_movement(0);
    assert_eq!(
        (first.flags, first.actor_flags),
        (move_flags::MOVING, actor_flags::STANDING_ON_SOLID)
    );
    assert_eq!(first.delta_movement, [0, 127, 0]);
    assert_eq!(
        first.velocity, [0; 3],
        "the key-down packet keeps the old velocity"
    );
    f.tick(0.1);
    assert!(close(speed(f.mover()), 5.40, 0.01));

    let mut walk = Feel::new();
    walk.walk(true);
    walk.hold("moveforward");
    walk.tick(0.1);
    assert!(
        close(speed(walk.mover()), 2.43, 0.01),
        "walk {}",
        speed(walk.mover())
    );

    let mut back = Feel::new();
    back.hold("moveback");
    back.tick(0.1);
    assert!(
        close(speed(back.mover()), 3.51, 0.01),
        "back {}",
        speed(back.mover())
    );

    let mut walk_back = Feel::new();
    walk_back.walk(true);
    walk_back.hold("moveback");
    walk_back.tick(0.1);
    assert!(
        close(speed(walk_back.mover()), 1.58, 0.01),
        "walk back {}",
        speed(walk_back.mover())
    );

    let mut strafe = Feel::new();
    strafe.hold("moveleft");
    strafe.tick(0.1);
    assert!(
        close(speed(strafe.mover()), 5.40, 0.01),
        "strafe {}",
        speed(strafe.mover())
    );

    let mut diagonal = Feel::new();
    diagonal.hold("moveforward");
    diagonal.hold("moveright");
    diagonal.tick(0.1);
    assert!(
        close(speed(diagonal.mover()), 5.40, 0.05),
        "diagonal {}",
        speed(diagonal.mover())
    );
}

/// No deceleration on foot: the release stops dead (the capture's slide is at most 0.13 m) and sends the
/// two stop packets 7-9 ms apart, the first with the old velocity.
#[test]
fn a_release_stops_dead_and_sends_the_two_captured_stop_packets() {
    if !aa_cry::pak_available() {
        return;
    }
    let mut f = Feel::new();
    f.hold("moveforward");
    f.tick(0.1);
    f.tick(0.1);
    let before = f.mover().pos;
    f.release("moveforward");
    let first = f.tick(0.016);
    assert_eq!(f.mover().pos, before, "the release must not slide");
    assert_eq!(speed(f.mover()), 0.0);
    let stop = first.last().expect("the first stop packet").to_movement(0);
    assert_eq!(stop.flags, move_flags::STOPPING);
    assert!(
        close(stop.velocity[1] as f32 / 1024.0, 5.40, 0.02),
        "the first stop packet keeps the old velocity: {:?}",
        stop.velocity
    );
    let second = f.tick(STOP_GAP);
    let zero = second
        .last()
        .expect("the second stop packet")
        .to_movement(0);
    assert_eq!(zero.flags, move_flags::STOPPING);
    assert_eq!(zero.velocity, [0; 3]);
}

/// Keyboard turning: a constant 90 deg/s in place, flags 0 on the turn packets, and the camera turns with
/// the character.
#[test]
fn turning_in_place_is_ninety_degrees_a_second_with_flags_zero() {
    if !aa_cry::pak_available() {
        return;
    }
    let mut f = Feel::new();
    let start = f.mover().facing;
    let camera = f.orbit_yaw();
    f.hold("turnleft");
    let mut turn_flags = Vec::new();
    for _ in 0..60 {
        turn_flags.extend(
            f.tick(1.0 / 60.0)
                .into_iter()
                .map(|m| m.to_movement(0).flags),
        );
    }
    let turned = f.mover().facing - start;
    assert!(
        close(turned.to_degrees(), 90.0, 0.01),
        "turned {} deg",
        turned.to_degrees()
    );
    assert!(
        close(f.orbit_yaw() - camera, turned, 1e-4),
        "the camera turns with the character"
    );
    assert!(!turn_flags.is_empty());
    assert!(
        turn_flags.iter().all(|&flags| flags == 0),
        "turn packets are flags 0: {turn_flags:?}"
    );
    f.release("turnleft");
    let stop = f
        .tick(0.016)
        .last()
        .expect("turn release stop")
        .to_movement(0);
    assert_eq!(stop.flags, move_flags::STOPPING);
}

/// While moving, the velocity direction is the facing with no lag (the capture's turn snaps; only the mount
/// curves).
#[test]
fn a_turn_while_moving_snaps_the_velocity_to_the_new_facing() {
    if !aa_cry::pak_available() {
        return;
    }
    let mut f = Feel::new();
    f.hold("moveforward");
    f.hold("turnleft");
    f.tick(0.016);
    f.tick(0.1);
    let heading = f.mover().velocity.truncate().normalize();
    let facing = forward(f.mover().facing);
    assert!(
        heading.dot(facing) > 0.9999,
        "velocity {heading:?} vs facing {facing:?}"
    );
}

/// feel capture 1: take-off flags 0x06 with actor 0x08 standing / 0x0c moving, launch vz 8.06 (the
/// standing packet samples 7.88 one frame later), gravity 25, peak 1.30-1.33 m, 0.60-0.68 s in the air,
/// the horizontal speed carried with no input and live input in the air, and a landing with actor 0x14
/// and no fall-velocity block.
#[test]
fn a_jump_takes_off_and_lands_like_the_capture() {
    if !aa_cry::pak_available() {
        return;
    }
    let actor = ActorParams::default();
    let launch = actor.jump_speed();
    assert!(close(launch, 8.062, 0.001));

    let mut f = Feel::new();
    f.hold("moveforward");
    f.tick(0.1);
    f.tick(0.1);
    f.tap("jump");
    let packets = f.tick(0.016);
    let takeoff = packets.last().expect("the take-off packet").to_movement(0);
    assert_eq!(
        (takeoff.flags, takeoff.actor_flags),
        (move_flags::JUMPING, 0x0c)
    );
    assert!(
        close(takeoff.velocity[2] as f32 / 1024.0, 8.062, 0.01),
        "launch vz"
    );
    assert!(
        close((takeoff.velocity[1] as f32 / 1024.0).abs(), 5.40, 0.05),
        "launch horizontal {:?} speed {:.3} pos {:?}",
        takeoff.velocity,
        speed(f.mover()),
        f.mover().pos
    );
    assert!(!f.mover().grounded);
    assert!(
        close(speed(f.mover()), 5.40, 0.05),
        "the horizontal speed is carried {:.3} at {:?}",
        speed(f.mover()),
        f.mover().pos
    );

    // letting go in the air keeps the carried speed (the capture's airborne rows have delta 0 with 5.4 m/s)
    f.release("moveforward");
    f.tick(0.1);
    assert!(
        close(speed(f.mover()), 5.40, 0.02),
        "air carry {}",
        speed(f.mover())
    );
    // and the input stays live
    f.hold("moveback");
    f.tick(0.1);
    assert!(
        close(speed(f.mover()), 3.51, 0.02),
        "air input {}",
        speed(f.mover())
    );

    // the standing take-off packet's actor byte is 0x08
    let mut standing = Feel::new();
    standing.tap("jump");
    let packets = standing.tick(0.016);
    let takeoff = packets
        .last()
        .expect("the standing take-off packet")
        .to_movement(0);
    assert_eq!(
        (takeoff.flags, takeoff.actor_flags),
        (move_flags::JUMPING, 0x08)
    );

    // the arc at 60 fps: peak 1.30 m, air time 0.645 s (the capture 1.33 m / 0.60-0.68 s)
    let dt = 1.0f32 / 60.0;
    let (mut z, mut vz, mut peak, mut air) = (0.0f32, launch, 0.0f32, 0.0f32);
    while !(z <= 0.0 && vz <= 0.0) {
        (z, vz) = air_step(z, vz, actor.gravity, dt, f32::INFINITY);
        peak = peak.max(z);
        air += dt;
    }
    assert!(close(peak, 1.30, 0.04), "peak {peak}");
    assert!((0.60..=0.68).contains(&air), "air {air}");

    // a normal landing: flags 0x06 moving / 0x04 standing, actor 0x14, no fall block
    let mut moving = Feel::new();
    moving.hold("moveforward");
    moving.airborne(-8.0);
    let landing = moving
        .tick(0.1)
        .last()
        .expect("the landing packet")
        .to_movement(0);
    assert_eq!(
        (landing.flags, landing.actor_flags),
        (move_flags::JUMPING, actor_flags::LANDING)
    );
    assert_eq!(landing.fall_vel, 0);
    assert!(close(landing.velocity[2] as f32 / 1024.0, 0.0, 0.01));
    // no landing slowdown: the next ground frame is at full run speed again
    moving.tick(0.02);
    assert!(close(speed(moving.mover()), 5.40, 0.05), "landing slowdown {}", speed(moving.mover()));

    let mut standing = Feel::new();
    standing.airborne(-8.0);
    let landing = standing
        .tick(0.1)
        .last()
        .expect("the standing landing packet")
        .to_movement(0);
    assert_eq!(
        (landing.flags, landing.actor_flags),
        (move_flags::STOPPING, actor_flags::LANDING)
    );

    // a fast fall carries the 0x80 fall block (the capture's 21.4 m/s drop)
    let mut fast = Feel::new();
    fast.airborne(-25.0);
    let landing = fast
        .tick(0.1)
        .last()
        .expect("the fast landing packet")
        .to_movement(0);
    assert_ne!(landing.actor_flags & actor_flags::FALL_VELOCITY, 0);
    assert!(landing.fall_vel > 0);
}

/// 10 Hz while moving, the key-down extra packet, and the 3 s idle heartbeat with flags 0.
#[test]
fn movement_packets_are_ten_hertz_and_idle_has_a_three_second_heartbeat() {
    if !aa_cry::pak_available() {
        return;
    }
    let mut f = Feel::new();
    f.hold("moveforward");
    f.tick(1.0 / 60.0);
    let mut times = Vec::new();
    for _ in 0..120 {
        for _ in f.tick(1.0 / 60.0) {
            times.push(f.since_spawn());
        }
    }
    assert!(times.len() >= 10, "{times:?}");
    let mut gaps: Vec<f32> = times.windows(2).map(|w| w[1] - w[0]).collect();
    gaps.sort_by(f32::total_cmp);
    let median = gaps[gaps.len() / 2];
    assert!(
        (0.09..=0.13).contains(&median),
        "median packet gap {median}"
    );

    f.release("moveforward");
    f.tick(0.016);
    f.tick(STOP_GAP);
    f.packets();
    let stopped_at = f.since_spawn();
    let mut heartbeat = None;
    for _ in 0..200 {
        for packet in f.tick(1.0 / 60.0) {
            let at = f.since_spawn();
            let movement = packet.to_movement(0);
            if at - stopped_at > 2.5 {
                heartbeat = Some((at, movement));
            }
        }
    }
    let (at, movement) = heartbeat.expect("the idle heartbeat");
    assert!(
        (3.0..=3.15).contains(&(at - stopped_at)),
        "heartbeat after {} s",
        at - stopped_at
    );
    assert_eq!(
        (movement.flags, movement.actor_flags),
        (0, actor_flags::STANDING_ON_SOLID)
    );
    assert_eq!(movement.velocity, [0; 3]);
}

/// The mount's inertia is CryPhysics' rate choice (`aaemu-physics entities/living/step.rs:171-179`):
/// `vel_requested` non-zero uses `inertia_accel` (11), no input uses `inertia` (4), and 0 is instant.
#[test]
fn mount_speed_uses_inertia_accel_for_a_request_and_inertia_without_one() {
    let mount_speed = MountSpeed {
        run: 9.0,
        walk: 0.2,
        backward: 0.2,
        inertia: 4.0,
        accel: 11.0,
        ..MountSpeed::default()
    };
    let target = Vec2::new(0.0, mount_speed.run);
    let dt = 0.1;
    let first = mount_velocity(Vec2::ZERO, target, mount_speed, dt);
    assert!(
        close(first.length(), 5.9, 0.15),
        "first packet {}",
        first.length()
    );
    let mut v = first;
    for _ in 0..4 {
        v = mount_velocity(v, target, mount_speed, dt);
    }
    assert!(close(v.length(), 8.85, 0.2), "0.5 s {}", v.length());

    let mut v = Vec2::new(0.0, 12.65);
    let mut distance = 0.0;
    let mut t = 0.0;
    for step in 0..15 {
        let next = mount_velocity(v, Vec2::ZERO, mount_speed, dt);
        distance += (v.length() + next.length()) * 0.5 * dt;
        v = next;
        t += dt;
        if step == 0 {
            assert!(close(v.length(), 8.3, 0.3), "0.1 s {}", v.length());
        }
        if step == 6 {
            assert!(close(v.length(), 0.7, 0.3), "0.7 s {}", v.length());
        }
    }
    assert_eq!(v, Vec2::ZERO, "the coast is over by 1.5 s");
    assert!((3.1..=3.4).contains(&distance), "slide {distance} m");
    assert!((0.9..=1.6).contains(&t));

    // a turn at speed is a move request: `inertia_accel` 11, not the old "target not faster" rule (4)
    let carried = Vec2::new(0.0, 9.0);
    let turn = Vec2::new(9.0, 0.0);
    let turned = mount_velocity(carried, turn, mount_speed, dt);
    let expected = carried + (turn - carried) * (1.0 - (-11.0f32 * dt).exp());
    assert!(close(turned.x, expected.x, 1e-5) && close(turned.y, expected.y, 1e-5), "{turned:?}");
    // the same speed with no input is slowing: `inertia` 4
    let coasted = mount_velocity(carried, Vec2::ZERO, mount_speed, dt);
    assert!(close(coasted.y, 9.0 * (-0.4f32).exp(), 1e-5), "{coasted:?}");
    // a 0 rate means no inertia: the new velocity is the target, not a freeze
    let instant = MountSpeed { run: 9.0, walk: 0.2, backward: 0.2, inertia: 0.0, accel: 0.0, ..MountSpeed::default() };
    assert_eq!(mount_velocity(Vec2::ZERO, Vec2::new(0.0, 9.0), instant, dt), Vec2::new(0.0, 9.0));
    assert_eq!(mount_velocity(Vec2::new(0.0, 9.0), Vec2::ZERO, instant, dt), Vec2::ZERO);
}

/// feel capture 1, mount: the first packet is 5.9 m/s and 8.85 by 0.5 s, and a release coasts
/// 12.65 -> 8.3 -> 0.7 -> 0 over 3.2-3.3 m; the released velocity keeps moving with flags 0 until the
/// explicit zero stop packet.
#[test]
fn a_mount_ramps_and_coasts_at_the_captured_rates() {
    if !aa_cry::pak_available() {
        return;
    }
    let mount_speed = MountSpeed {
        run: 9.0,
        walk: 0.2,
        backward: 0.2,
        inertia: 4.0,
        accel: 11.0,
        ..MountSpeed::default()
    };
    let mut f = Feel::new();
    f.mount(mount_speed);
    f.hold("moveforward");
    let packets = f.tick(0.1);
    // the capture's first packet: position delta 3.05 m/s, velocity field 5.95 m/s
    assert!(
        close(speed(f.mover()), 3.0, 0.2),
        "mount first frame position {}",
        speed(f.mover())
    );
    let moving = packets.last().expect("the mount packet").to_movement(0);
    assert_eq!((moving.flags, moving.move_type), (move_flags::MOVING, 1));
    assert!(
        close(moving.velocity[1] as f32 / 1024.0, 5.9, 0.15),
        "mount first packet velocity {}",
        moving.velocity[1] as f32 / 1024.0
    );
    for _ in 0..4 {
        f.tick(0.1);
    }
    assert!(
        close(speed(f.mover()), 8.9, 0.2),
        "mount 0.5 s {}",
        speed(f.mover())
    );
    f.release("moveforward");
    let start = f.mover().pos;
    let release = f.tick(0.1);
    let stop = release
        .last()
        .expect("the mount release packet")
        .to_movement(0);
    assert_eq!(
        stop.flags,
        move_flags::STOPPING,
        "the release packet keeps the old velocity"
    );
    let mut coast = 0;
    let mut coast_packets = Vec::new();
    for _ in 0..15 {
        let packets = f.tick(0.1);
        coast += packets.len();
        coast_packets.extend(packets);
    }
    let slid = f.mover().pos.distance(start);
    // from the base 9 m/s: v / inertia = 2.25 m; the capture's 3.2-3.3 m is its 12.65 m/s decel
    assert!((1.9..=2.5).contains(&slid), "mount slide {slid} m");
    assert!(
        f.tick(0.1)
            .iter()
            .all(|m| m.to_movement(0).flags != move_flags::MOVING)
    );
    assert!(coast > 0);
    assert_eq!(speed(f.mover()), 0.0);
    // the coast ends with an explicit v=0 stop packet (the capture's last mount row)
    let final_stop = coast_packets
        .last()
        .expect("the mount's final stop packet")
        .to_movement(0);
    assert_eq!((final_stop.flags, final_stop.velocity), (move_flags::STOPPING, [0; 3]));
}

/// Space on the driver's seat jumps the mate (`sub_39404D50` sets the request on the controlled unit) with the mount's
/// own row: the Lilyut horse's `game_jump_height` 1.3 and `physics_living_gravity` 30 launch at sqrt(2 g h) 8.83 m/s,
/// peak at 1.30 m and land 0.59 s later. The flags are the on-foot capture's (no mounted-jump capture exists): JUMPING
/// with actor 0x0c, then 0x14 on the landing, every packet a `CSMoveUnit` of the mate.
#[test]
fn a_driven_mount_jumps_with_its_own_height_and_gravity() {
    if !aa_cry::pak_available() {
        return;
    }
    let horse = MountSpeed::default();
    // `control_player` gives the rider's `ActorParams` the mount's row while it drives
    let launch = ActorParams { jump_height: horse.jump_height, gravity: horse.gravity, ..ActorParams::default() }.jump_speed();
    assert!(close(launch, 8.832, 0.001));
    let mut f = Feel::new();
    f.mount(horse);
    f.hold("moveforward");
    for _ in 0..5 {
        f.tick(0.1);
    }
    assert!(f.mover().grounded);
    let dt = 1.0 / 60.0;
    let start = f.mover().pos.z;
    f.tap("jump");
    let packets = f.tick_named(dt);
    assert!(f.input().jumped, "a mounted take-off is a jump for the cast rules");
    let (unit, takeoff) = packets.last().expect("the take-off packet");
    let takeoff = takeoff.to_movement(0);
    assert_eq!(*unit, Some(9), "the take-off names the mate");
    assert_eq!((takeoff.flags, takeoff.actor_flags), (move_flags::JUMPING, 0x0c));
    assert!(
        (i32::from(takeoff.velocity[2]) - 9044).abs() <= 2,
        "launch vz {} (8.83 m/s)",
        takeoff.velocity[2]
    );
    assert!(f.app.world().resource::<LocalJump>().rising);

    let mut air = dt;
    let mut peak = 0.0f32;
    let mut sent = packets;
    let mut landing = None;
    while air < 2.0 {
        let packets = f.tick_named(dt);
        air += dt;
        peak = peak.max(f.mover().pos.z - start);
        let grounded = f.mover().grounded;
        if grounded {
            landing = packets.last().map(|(_, m)| m.to_movement(0));
        }
        sent.extend(packets);
        if grounded {
            break;
        }
    }
    let landing = landing.expect("the landing packet");
    assert!(close(peak, 1.30, 0.05), "peak {peak}");
    assert!(close(air, 0.59, 0.05), "air time {air}");
    assert_eq!((landing.flags, landing.actor_flags), (move_flags::JUMPING, actor_flags::LANDING));
    assert_eq!(landing.fall_vel, 0);
    assert!(sent.iter().all(|(unit, _)| *unit == Some(9)), "every packet is the mate's CSMoveUnit");
}

/// A standing Space on the driver's seat does not lift the mount: with no move request `sub_3912B330` calls the unit's
/// +680 with 0 (`em_jump_in_place`) and leaves the jump vector at 0, so no vertical speed and no take-off packet, only
/// one [`MountRear`] of the mate. The rear is no jump for the cast rules.
#[test]
fn a_standing_mount_rears_instead_of_taking_off() {
    if !aa_cry::pak_available() {
        return;
    }
    let mut f = Feel::new();
    f.mount(MountSpeed::default());
    f.tick(0.1);
    f.tick(0.1);
    f.packets();
    f.tap("jump");
    let packets = f.tick(1.0 / 60.0);
    assert!(f.mover().grounded);
    assert_eq!(f.mover().vz, 0.0);
    assert!(!f.input().jumped);
    assert!(
        packets.iter().all(|m| m.to_movement(0).flags & move_flags::JUMPING != move_flags::JUMPING),
        "no take-off packet: {packets:?}"
    );
    assert_eq!(f.rears(), [MountRear { mate: 9 }]);
    f.tick(0.1);
    assert!(f.mover().grounded && f.mover().vz == 0.0);
    assert!(f.rears().is_empty(), "one rear per press");

    // a moving take-off is a jump, not a rear; on foot Space never rears
    f.hold("moveforward");
    f.tick(0.1);
    f.tap("jump");
    f.tick(1.0 / 60.0);
    assert!(!f.mover().grounded);
    assert!(f.rears().is_empty());
    let mut on_foot = Feel::new();
    on_foot.tap("jump");
    on_foot.tick(1.0 / 60.0);
    assert!(!on_foot.mover().grounded && on_foot.rears().is_empty());
}

/// A driven mount that leaves a ledge falls under its own gravity (30 for a horse), not the rider's 25.
#[test]
fn a_driven_mount_falls_with_its_own_gravity() {
    if !aa_cry::pak_available() {
        return;
    }
    let mut f = Feel::new();
    f.mount(MountSpeed::default());
    {
        let mut mover = f.app.world_mut().get_mut::<Mover>(f.player).unwrap();
        mover.grounded = false;
        mover.vz = 0.0;
        mover.pos.z += 3.0;
    }
    let before = f.mover().pos.z;
    f.tick(0.1);
    assert!(close(f.mover().vz, -3.0, 1e-4), "vz {}", f.mover().vz);
    assert!(close(f.mover().pos.z - before, -0.15, 1e-4), "dz {}", f.mover().pos.z - before);
}

/// A mounted take-off sets `PlayerInput::jumped`, so `combat::cancel_on_move` ends the driver's casts as it does on foot
/// (the fixture's skill and plot casts, one `StopCasting` each), once.
#[test]
fn a_mount_jump_cancels_the_drivers_cast() {
    if !aa_cry::pak_available() || !aa_anim::db::GameDb::available() {
        return;
    }
    let mut f = Feel::new();
    f.mount(MountSpeed::default());
    f.app
        .init_resource::<aa_anim::ClipLibrary>()
        .init_resource::<aa_net::state::Doodads>()
        .init_resource::<aa_net::state::Units>();
    crate::ingame::combat::fixture_cancel_casts(&mut f.app, 1);
    let stop_castings = |f: &mut Feel, dt: f32| {
        f.step(dt);
        f.app
            .world_mut()
            .resource_mut::<Messages<NetCommand>>()
            .drain()
            .filter(|NetCommand(command)| matches!(command, Command::StopCasting { .. }))
            .count()
    };
    assert_eq!(stop_castings(&mut f, 0.1), 0, "standing on the mount keeps the cast");
    f.hold("moveforward");
    f.tap("jump");
    assert_eq!(stop_castings(&mut f, 1.0 / 60.0), 2, "the take-off ends the skill and the plot cast");
    assert!(f.input().jumped && !f.mover().grounded);
    assert_eq!(stop_castings(&mut f, 0.1), 0, "a cancelled cast is not cancelled again");
}

/// feel capture 1, slopes: the 3D speed is 5.30 at 11 degrees, 5.06 at 21 and 4.1-4.2 at 38-41, with the
/// horizontal part the run speed times `cos^2`; uphill and downhill are symmetric.
#[test]
fn slopes_take_the_captured_share_of_the_run_speed() {
    let run = 5.4f32;
    for (deg, expect_3d) in [(11.0f32, 5.30), (21.0, 5.06), (40.0, 4.15)] {
        let angle = deg.to_radians();
        let factor = slope_factor(angle.tan() * SLOPE_PROBE, SLOPE_PROBE);
        let horizontal = run * factor;
        let speed_3d = horizontal / angle.cos();
        assert!(close(speed_3d, expect_3d, 0.08), "{deg} deg: 3D {speed_3d}");
        assert!(
            close(
                factor,
                slope_factor(-angle.tan() * SLOPE_PROBE, SLOPE_PROBE),
                1e-6
            ),
            "symmetric"
        );
    }
}

/// The slope probe behind the capture's `cos^2`: a real 10.8 degree terrain segment, a brush (the old
/// cave floor) and a ledge (a drop greater than `DROP`) all give the expected horizontal/vertical shares.
#[test]
fn the_slope_probe_reads_terrain_brushes_and_ledges() {
    if !aa_cry::pak_available() {
        return;
    }
    let terrain = TerrainQuery::new("main_world");
    let collision = WorldCollision::new("main_world", terrain.clone());
    // the capture's downhill segment (15512.2,15306.28 -> 15511.9,15303.0), probed uphill
    let pos = Vec3::new(15512.2, 15306.28, 131.17058);
    let here = terrain.ground_at(pos.x, pos.y).unwrap().height;
    let ahead = terrain.ground_at(pos.x, pos.y + SLOPE_PROBE).unwrap().height;
    let probe = step_slope(&terrain, &collision, pos, Vec2::Y, STEP_UP);
    assert!(close(probe.factor, slope_factor(ahead - here, SLOPE_PROBE), 1e-6));
    assert!(close(probe.tan, (ahead - here) / SLOPE_PROBE, 1e-6));
    assert!(probe.factor < 0.999 && probe.tan > 0.0, "{probe:?}");
    // downhill is the mirror and uses the same slope
    let down = step_slope(&terrain, &collision, pos, -Vec2::Y, STEP_UP);
    assert!(close(down.factor, slope_factor(here - terrain.ground_at(pos.x, pos.y - SLOPE_PROBE).unwrap().height, SLOPE_PROBE), 1e-6));

    // a brush (the known cave floor under a terrain hole) has its own footing: no slope share
    collision.build_blocking(20, 8);
    let cave = Vec3::new(21270.5, 8960.5, 474.8673 + 5.0);
    assert_eq!(step_slope(&terrain, &collision, cave, Vec2::Y, STEP_UP), StepSlope { factor: 1.0, tan: 0.0 });

    // a ledge is a fall: the character keeps its full speed
    let ledge = Vec3::new(21000.0, 10542.0, 119.993);
    assert_eq!(step_slope(&terrain, &collision, ledge, Vec2::Y, STEP_UP), StepSlope { factor: 1.0, tan: 0.0 });
}

/// A slow slide along a wall must not toggle the movement packets every frame: start at 0.05 m/s,
/// keep going down to 0.02 m/s.
#[test]
fn the_active_threshold_has_hysteresis() {
    assert!(!is_moving(0.04, false));
    assert!(is_moving(0.04, true));
    assert!(!is_moving(0.01, true));
    assert!(is_moving(0.06, false));
    assert!(is_moving(0.06, true));
}

/// Stale air carry is cleared or seeded at every owner hand-back: swim entry/exit, the glider, a ship
/// deck and a skill move. Otherwise a jump out of water, a folded glider or a stepped-off deck would
/// drift at an old velocity.
#[test]
fn stale_air_carry_is_cleared_or_seeded_at_the_hand_backs() {
    if !aa_cry::pak_available() {
        return;
    }
    // glider: the last glider frame's velocity is carried into the fall
    let mut glide = Feel::new();
    {
        let mut mover = glide.app.world_mut().get_mut::<Mover>(glide.player).unwrap();
        mover.grounded = false;
        mover.velocity = Vec3::new(3.0, 4.0, 0.0);
    }
    glide.app.world_mut().resource_mut::<SendState>().glide_owns = true;
    glide.tick(0.05);
    assert_eq!(glide.app.world().resource::<SendState>().air_velocity, Vec2::new(3.0, 4.0));
    glide.app.world_mut().resource_mut::<SendState>().glide_owns = false;
    glide.tick(0.05);
    assert!(close(glide.mover().velocity.x, 3.0, 0.1) && close(glide.mover().velocity.y, 4.0, 0.1), "{:?}", glide.mover().velocity);

    // deck hand-over: the hull speed is the CarriedVelocity, not an old own one
    let mut deck = Feel::new();
    {
        let mut state = deck.app.world_mut().resource_mut::<SendState>();
        state.air_velocity = Vec2::new(5.0, 0.0);
        state.hand_over_from_deck();
    }
    assert_eq!(deck.app.world().resource::<SendState>().air_velocity, Vec2::ZERO);

    // skill move: the skill's last velocity is carried after it returns
    let mut skill = Feel::new();
    {
        let mut mover = skill.app.world_mut().get_mut::<Mover>(skill.player).unwrap();
        mover.velocity = Vec3::new(3.0, 4.0, 0.0);
    }
    skill.app.world_mut().entity_mut(skill.player).insert(crate::ingame::skill_motion::SkillMove::fixture());
    skill.tick(0.01);
    assert_eq!(skill.app.world().resource::<SendState>().air_velocity, Vec2::new(3.0, 4.0));

    // swim entry clears the old land carry; leaving the water seeds the swimmer's own speed (zero here)
    let mut water = Feel::at(15600.0, 15458.0);
    water.app.world_mut().resource_mut::<SendState>().air_velocity = Vec2::new(5.4, 0.0);
    water.tick(0.05);
    assert!(water.mover().swim.is_some(), "the sea point starts swimming");
    assert_eq!(water.app.world().resource::<SendState>().air_velocity, Vec2::ZERO);
    {
        let mut mover = water.app.world_mut().get_mut::<Mover>(water.player).unwrap();
        mover.pos = Vec3::new(15480.0, 15282.0, 134.86328);
    }
    water.app.world_mut().resource_mut::<SendState>().air_velocity = Vec2::new(5.4, 0.0);
    water.tick(0.05);
    assert!(water.mover().swim.is_none(), "the dry point leaves the water");
    assert_eq!(water.app.world().resource::<SendState>().air_velocity, Vec2::ZERO);
}
