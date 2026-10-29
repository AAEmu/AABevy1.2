// The captured f32 values keep their exact decimal expansion: the golden bodies
// are compared byte for byte.
#![allow(clippy::excessive_precision)]

use super::*;
use aa_net::game::ClientMsg;
use aa_net::model::Movement;
use aa_net::vehicle::VEL_SCALE_VEHICLE;
use aa_net::vehicle::codec::write_i16_vec3;
use aa_net::wire::Reader;

fn hex(raw: &str) -> Vec<u8> {
    raw.split_whitespace()
        .map(|b| u8::from_str_radix(b, 16).unwrap())
        .collect()
}

fn encoded(controller: &CartController, obj_id: u32, time: u32) -> Vec<u8> {
    let mut request = controller.request(obj_id).unwrap();
    request.set_time(time);
    ClientMsg::MoveVehicle(request).encode_body()
}

fn mps(raw: i16) -> f32 {
    f32::from(raw) * 30.0 / 32767.0
}

#[test]
fn a_captured_still_body_is_rebuilt_byte_for_byte() {
    // 2026-10-29 capture, case a_still, Farm Wagon 0x84C9 at client time
    // 1104185. The pose is seeded from the decoded body; the controller adds
    // no change while the keys are idle.
    let captured = hex("c9 84 00 02 39 d9 10 00 00 2f ad 51 86 d3 79 74
         06 05 00 00 00 00 00 00 e6 00 17 00 96 80 00 00
         00 00 00 00 00 00 00 00 00 00 00 00 00 00 00");
    let controller = CartController::from_pose(
        Pos::new(10454.591796875, 15593.76171875, 229.4652),
        [0.007019, 0.000702, -0.995453, 0.094995],
        [0.0; 3],
        0.0,
        0.0,
    );
    assert_eq!(encoded(&controller, 0x84C9, 1104185), captured);

    let body = encoded(&controller, 0x84C9, 1104185);
    let mut r = Reader::new(&body);
    assert_eq!(r.bc().unwrap(), 0x84C9);
    let movement = Movement::read(&mut r).unwrap();
    assert_eq!(
        (movement.move_type, movement.time, movement.flags),
        (2, 1104185, 0)
    );
    assert_eq!(movement.velocity, [0; 3]);
    let vehicle = movement.vehicle.as_ref().unwrap();
    assert_eq!(vehicle.angular_velocity, [0.0; 3]);
    assert_eq!(vehicle.steering, 0.0);
    assert!(vehicle.wheel_ang_vel.is_empty());
    assert!(r.is_empty());
}

#[test]
fn a_captured_turning_body_is_rebuilt_byte_for_byte() {
    // 2026-10-29 capture, case c_left, steering held +1: angVel 0.4 and the
    // 1/1092 m/s velocity [271, -1030, 1].
    let captured = hex("c9 84 00 02 9c 0d 11 00 00 db b4 51 c4 ab 79 74
         06 05 0f 01 fa fb 01 00 db 00 11 00 e9 80 00 00
         00 00 00 00 00 00 cd cc cc 3e 00 00 80 3f 00");
    let controller = CartController::from_pose(
        Pos::new(10458.427734375, 15573.8828125, 229.4652),
        [0.006684, 0.000519, -0.99292, 0.118598],
        [mps(271), mps(-1030), mps(1)],
        0.974,
        1.0,
    );
    let body = encoded(&controller, 0x84C9, 1117596);
    assert_eq!(body, captured);
    assert_eq!(controller.angular_velocity(), [0.0, 0.0, 0.4]);

    let mut r = Reader::new(&body);
    r.bc().unwrap();
    let movement = Movement::read(&mut r).unwrap();
    assert_eq!(movement.velocity, [271, -1030, 1], "1/1092 m/s wire scale");
    let vehicle = movement.vehicle.as_ref().unwrap();
    assert_eq!(vehicle.angular_velocity, [0.0, 0.0, 0.4]);
    assert_eq!(vehicle.steering, 1.0);
    assert!(
        vehicle.wheel_ang_vel.is_empty(),
        "the Farm Wagon sends no wheel block"
    );
}

#[test]
fn a_captured_right_turn_body_is_rebuilt_byte_for_byte() {
    // 2026-10-29 capture, case c_right: steering -1 at 0.96 m/s.
    let captured = hex("c9 84 00 02 f2 25 11 00 00 50 cb 51 ab 9e 79 71
         06 05 f6 03 06 ff 0c 00 c5 00 b2 ff 02 9c
         00 00 00 00 00 00 00 00 cd cc cc be 00 00 80 bf 00");
    let controller = CartController::from_pose(
        Pos::new(10469.65625, 15567.333984375, 229.4622),
        [0.006012, -0.00238, -0.781213, 0.624231],
        [mps(1014), mps(-250), mps(12)],
        0.96,
        -1.0,
    );
    assert_eq!(encoded(&controller, 0x84C9, 1123826), captured);
    assert_eq!(controller.angular_velocity(), [0.0, 0.0, -0.4]);
}

#[test]
fn a_captured_reverse_body_is_rebuilt_byte_for_byte() {
    // 2026-10-29 capture, case d_reverse: straight back at 4.00 m/s.
    let captured = hex("c9 84 00 02 1f 43 11 00 00 4d d7 51 69 8f 79 68
         04 05 9b fc 90 10 51 02 8c 09 b4 09 4f 81 00 00
         00 00 00 00 00 00 00 00 00 00 00 00 00 00 00");
    let controller = CartController::from_pose(
        Pos::new(10475.650390625, 15559.705078125, 228.941),
        [0.074587, 0.075808, -0.989807, 0.094723],
        [mps(-869), mps(4240), mps(593)],
        -4.0,
        0.0,
    );
    assert_eq!(encoded(&controller, 0x84C9, 1131295), captured);
    assert_eq!(controller.angular_velocity(), [0.0, 0.0, 0.0]);
}

#[test]
fn a_stepped_quaternion_matches_the_captured_right_turn() {
    // Start at case c_right's 1123826 sample (steering already at -1) and let step()
    // produce the next quaternion for the 143 ms up to 1123969; compare its six wire
    // bytes, not a quaternion copied from the expected row.
    let captured = hex("c9 84 00 02 81 26 11 00 00 9c cb 51 98 9e 79 72
         06 05 5a 05 5b fe 0f 00 c7 00 b8 ff c6 99
         00 00 00 00 00 00 00 00 cd cc cc be 00 00 80 bf 00");
    let mut controller = CartController::from_pose(
        Pos::new(10469.65625, 15567.333984375, 229.4622),
        [0.006012, -0.00238, -0.781213, 0.624231],
        [mps(1014), mps(-250), mps(12)],
        0.96,
        -1.0,
    );
    // the captured 1123969 sample is 0.05695 rad further round at -0.4 rad/s
    let dt = (controller.yaw - 2.0 * (-0.798_669f32).atan2(0.601_735)) / YAW_RATE_PER_STEER;
    controller.step(dt, 0.0, -1.0);
    let body = encoded(&controller, 0x84C9, 1123969);
    assert_eq!(
        &body[24..30],
        &captured[24..30],
        "quaternion bytes: {body:02x?} vs {captured:02x?}"
    );
}

#[test]
fn the_velocity_scale_is_thirty_over_32767() {
    assert_eq!(
        write_i16_vec3([1.0, 0.0, 0.0], VEL_SCALE_VEHICLE),
        [1092, 0, 0]
    );
    assert_eq!(
        write_i16_vec3([0.0, -4.0, 0.0], VEL_SCALE_VEHICLE),
        [0, -4369, 0]
    );
    assert_eq!(
        write_i16_vec3([0.0, -30.0, 0.0], VEL_SCALE_VEHICLE),
        [0, -32767, 0]
    );
}

#[test]
fn the_captured_steering_ramp_drives_the_yaw_rate() {
    // 2026-10-29 capture: the only non-zero steering values are send-gate
    // samples of one linear ramp: 0.484 two 22.5 FPS frames after the press,
    // -0.752 one frame after release and 0.269 three frames after release.
    let dt = 1.0 / 22.5;
    let mut controller = CartController::new(Pos::default(), 0.0);
    controller.step(dt, 0.0, 1.0);
    assert!(
        (controller.steering - 0.244).abs() < 0.02,
        "one frame in: {}",
        controller.steering
    );
    controller.step(dt, 0.0, 1.0);
    assert!(
        (controller.steering - 0.484).abs() < 0.02,
        "two frames in: {}",
        controller.steering
    );
    for _ in 0..3 {
        controller.step(dt, 0.0, 1.0);
    }
    assert_eq!(controller.steering, 1.0, "held A reaches +1 in ~4 frames");
    assert_eq!(controller.angular_velocity(), [0.0, 0.0, 0.4]);
    // the yaw rate of a moving cart (the captured turns were at 0.96 and 4.0 m/s)
    controller.speed = TOP_SPEED;
    let yaw = controller.yaw;
    controller.step(1.0, 1.0, 1.0);
    assert!((controller.yaw - yaw - 0.4).abs() < 1e-4);

    controller.step(dt, 0.0, 0.0);
    assert!(
        (controller.steering - 0.752).abs() < 0.02,
        "one frame after release: {}",
        controller.steering
    );
    controller.step(dt, 0.0, 0.0);
    controller.step(dt, 0.0, 0.0);
    assert!(
        (controller.steering - 0.269).abs() < 0.02,
        "three frames after release: {}",
        controller.steering
    );
    controller.step(dt, 0.0, 0.0);
    controller.step(dt, 0.0, 0.0);
    assert_eq!(controller.steering, 0.0);
}

#[test]
fn the_stand_in_speed_curve_matches_the_capture() {
    let dt = 1.0 / 22.5;
    let mut controller = CartController::new(Pos::default(), 0.0);
    let mut t = 0.0;
    while controller.speed < TOP_SPEED && t < 3.0 {
        controller.step(dt, 1.0, 0.0);
        t += dt;
    }
    assert_eq!(controller.speed, TOP_SPEED);
    assert!(
        (t - 1.6).abs() < 0.1,
        "0 to 4 m/s took {t} s (capture 1.73 s including the input delay)"
    );

    let mut t = 0.0;
    while controller.speed > 0.0 && t < 3.0 {
        controller.step(dt, 0.0, 0.0);
        t += dt;
    }
    assert_eq!(controller.speed, 0.0);
    assert!(
        (t - 1.6).abs() < 0.1,
        "4 m/s to rest took {t} s (capture 1.77 s)"
    );
}

#[test]
fn the_pump_resends_while_still_on_the_captured_cadence() {
    let dt = 1.0 / 22.5;
    let mut pump = Pump::default();
    let mut sends = 0;
    let mut t = 0.0;
    while t < 1.0 {
        t += dt;
        if pump.tick(dt) {
            pump.sent();
            sends += 1;
        }
    }
    assert_eq!(
        sends, 7,
        "100 ms gate behind 44.4 ms frames: every third frame, 143 ms"
    );
}

#[test]
fn a_still_cart_is_resent_unchanged() {
    // case a_still: 36 bodies in 5 s, every one the same pose and zero speed.
    // The pump has no change test, so the only body field that moves is the
    // session-filled time.
    let controller = CartController::new(
        Pos::new(10454.591796875, 15593.76171875, 229.4652),
        -2.95131,
    );
    let first = encoded(&controller, 0x84C9, 1104185);
    let second = encoded(&controller, 0x84C9, 1104328);
    assert_eq!(&first[..4], &second[..4], "object id and move type");
    assert_eq!(
        &first[8..],
        &second[8..],
        "every field after the time is resent"
    );
    let mut r = Reader::new(&second);
    r.bc().unwrap();
    let movement = Movement::read(&mut r).unwrap();
    assert_eq!(movement.velocity, [0; 3]);
    let vehicle = movement.vehicle.as_ref().unwrap();
    assert_eq!(
        (vehicle.steering, vehicle.angular_velocity),
        (0.0, [0.0; 3])
    );
    assert!(r.is_empty());
}

#[test]
fn only_the_driver_seat_of_a_slave_drives() {
    let slave = UnitKind::Slave {
        id: 1,
        tl_id: 2,
        template_id: 60,
        summoner_id: 3,
    };
    let mate = UnitKind::Mate {
        tl_id: 2,
        template_id: 1,
        owner_id: 3,
    };
    assert!(driver_of(&slave, 1));
    assert!(!driver_of(&slave, 2), "a passenger does not steer");
    assert!(
        !driver_of(&mate, 1),
        "a mate is driven by mount::drive_mount"
    );
    assert_eq!(controls(true, false, true, false), (1.0, 1.0));
    assert_eq!(controls(false, true, false, true), (-1.0, -1.0));
}

#[test]
fn scripted_keys_drive_the_cart_like_the_bound_keys() {
    let actions = crate::settings::Actions::default();
    let mut dev = crate::ingame::DevOptions::default();
    dev.script_keys = "WA".into();
    assert_eq!(keyed_controls(&actions, &dev, 1.0, false), (1.0, 1.0));
    dev.script_keys = "SD".into();
    assert_eq!(keyed_controls(&actions, &dev, 1.0, false), (-1.0, -1.0));
    assert_eq!(keyed_controls(&actions, &dev, 1.0, true), (0.0, 0.0),
               "a HUD edit box keeps the keys from the cart");
    dev.script_keys.clear();
    assert_eq!(keyed_controls(&actions, &dev, 1.0, false), (0.0, 0.0));
}

#[test]
fn a_dead_driver_stops_the_controller_and_the_pump() {
    use aa_net::state::PlayerStats;
    assert!(!driver_dead(false, None, 7));
    assert!(!driver_dead(
        false,
        Some(&PlayerStats {
            obj_id: 8,
            dead: true,
            ..default()
        }),
        7
    ));
    assert!(driver_dead(
        false,
        Some(&PlayerStats {
            obj_id: 7,
            dead: true,
            ..default()
        }),
        7
    ));
    assert!(driver_dead(true, None, 7));

    let mut drive = CartDrive {
        obj_id: 7,
        controller: Some(CartController::new(Pos::new(1.0, 2.0, 3.0), 0.4)),
        pump: Pump { timer: 0.5 },
        written: Some((Pos::new(1.0, 2.0, 3.0), 0.4)),
    };
    drive.stop();
    assert!(drive.controller.is_none());
    assert_eq!(drive.pump, Pump::default());
    assert_eq!(drive.obj_id, 0);
    assert!(drive.written.is_none());
}

#[test]
fn the_server_moving_the_cart_resyncs_the_controller_pose() {
    let mut controller = CartController::new(Pos::new(1.0, 2.0, 3.0), 0.4);
    controller.speed = 3.0;
    controller.steering = 1.0;
    let written = (controller.pos, controller.yaw);
    // the echo of our own packet (within a cart length) and a small yaw lag do not resync
    assert!(!server_moved(written, Pos::new(1.5, 2.0, 3.0), 0.5));
    // a teleport / server move does
    assert!(server_moved(written, Pos::new(10.0, 2.0, 3.0), 0.4));
    assert!(server_moved(written, Pos::new(1.0, 2.0, 3.0), 1.2));

    controller.resync(Pos::new(10.0, 20.0, 30.0), 1.0);
    assert_eq!(
        (
            controller.pos.x,
            controller.pos.y,
            controller.pos.z,
            controller.yaw
        ),
        (10.0, 20.0, 30.0, 1.0)
    );
    let (s, c) = (0.5f32).sin_cos();
    assert_eq!(controller.rotation, [0.0, 0.0, s, c]);
    // the driver's speed and steering survive the pose correction
    assert_eq!((controller.speed, controller.steering), (3.0, 1.0));
}

#[test]
fn the_stand_in_does_not_turn_on_the_spot_and_mirrors_the_turn_in_reverse() {
    let mut still = CartController::new(Pos::default(), 0.0);
    for _ in 0..20 {
        still.step(0.05, 0.0, 1.0);
    }
    assert_eq!(still.yaw, 0.0, "full lock at a standstill");
    assert_eq!(still.angular_velocity(), [0.0, 0.0, 0.4], "the wire keeps the captured angVel");

    let mut forward = CartController::new(Pos::default(), 0.0);
    let mut reverse = CartController::new(Pos::default(), 0.0);
    for _ in 0..40 {
        forward.step(0.05, 1.0, 1.0);
        reverse.step(0.05, -1.0, 1.0);
    }
    assert!(forward.yaw > 0.0 && reverse.yaw < 0.0, "{} {}", forward.yaw, reverse.yaw);
    assert!((forward.yaw + reverse.yaw).abs() < 1e-5);
}
