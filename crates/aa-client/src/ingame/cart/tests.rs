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

#[test]
fn flat_ground_gives_zero_tilt_and_the_rest_offset() {
    // b_forward/a_still (2026-10-29 s67): pitch 0, z - terrain 0.024 m.
    let mut controller = CartController::new(Pos::new(0.0, 0.0, 100.024), 0.0);
    for _ in 0..45 {
        controller.step_grounded(1.0 / 45.0, 0.0, 0.0, |_, _| Some(100.0));
    }
    assert!(controller.pitch.abs() < 1e-4, "{}", controller.pitch);
    assert!(controller.roll.abs() < 1e-4, "{}", controller.roll);
    assert!(
        (controller.pos.z - 100.024).abs() < 1e-3,
        "{}",
        controller.pos.z
    );
    assert!(
        controller.velocity[2].abs() < 1e-3,
        "{}",
        controller.velocity[2]
    );
}

#[test]
fn the_four_wheel_plane_sets_pitch_and_roll() {
    // h = 100 + 0.1 x + 0.05 y: rises to the right (+X) and forward (+Y), so
    // pitch = atan(0.05) nose up and roll = -atan(0.1) (right side high).
    let ground = |x: f32, y: f32| Some(100.0 + 0.1 * x + 0.05 * y);
    let mut controller = CartController::new(Pos::new(0.0, 0.0, 100.0), 0.0);
    for _ in 0..900 {
        controller.step_grounded(1.0 / 60.0, 0.0, 0.0, ground);
    }
    assert!(
        (controller.pitch - 0.05f32.atan()).abs() < 2e-3,
        "{}",
        controller.pitch
    );
    assert!(
        (controller.roll + 0.1f32.atan()).abs() < 2e-3,
        "{}",
        controller.roll
    );
    // the type-2 quaternion is the full Rz*Ry*Rx, not the yaw alone
    assert!(
        controller.rotation[0].abs() > 1e-3,
        "{:?}",
        controller.rotation
    );
    assert!(
        controller.rotation[1].abs() > 1e-3,
        "{:?}",
        controller.rotation
    );
}

#[test]
fn the_ground_height_is_the_plane_at_the_body_origin() {
    // h = 100 - 0.1 y: the four wheel heights average 0.467 m behind the
    // origin, so using their mean would put the body 0.047 m too high.
    let ground = |_: f32, y: f32| Some(100.0 - 0.1 * y);
    let mut controller = CartController::new(Pos::new(0.0, 0.0, 100.071), 0.0);
    for _ in 0..200 {
        controller.step_grounded(1.0 / 45.0, 0.0, 0.0, ground);
    }
    assert!(
        (controller.pos.z - 100.024).abs() < 2e-3,
        "the plane at the body origin is 100.024, got {}",
        controller.pos.z
    );
}

#[test]
fn height_and_tilt_are_first_order_lags() {
    // Height follows in ~0.15 s: one 22.5 FPS frame covers about a quarter of a
    // step, 0.15 s exactly 1-e^-1.
    let mut controller = CartController::new(Pos::new(0.0, 0.0, 100.024), 0.0);
    controller.step_grounded(1.0 / 22.5, 0.0, 0.0, |_, _| Some(101.0));
    assert!(
        controller.pos.z > 100.05 && controller.pos.z < 100.5,
        "one frame must not snap: {}",
        controller.pos.z
    );
    let mut controller = CartController::new(Pos::new(0.0, 0.0, 100.024), 0.0);
    for _ in 0..6 {
        controller.step_grounded(0.025, 0.0, 0.0, |_, _| Some(101.0));
    }
    assert!(
        (controller.pos.z - (100.024 + 1.0 - (-1.0f32).exp())).abs() < 0.02,
        "0.15 s covers 63 %: {}",
        controller.pos.z
    );

    // Tilt is the slower body settle: a 0.2 rad plane step moves the pitch by
    // <5 % in one frame and by 63 % after 2.5 s.
    let ground = |_: f32, y: f32| Some(100.0 + 0.2 * y);
    let mut controller = CartController::new(Pos::new(0.0, 0.0, 100.0), 0.0);
    controller.step_grounded(1.0 / 22.5, 0.0, 0.0, ground);
    let target = 0.2f32.atan();
    assert!(
        controller.pitch < target * 0.05,
        "one frame must not snap: {}",
        controller.pitch
    );
    let mut controller = CartController::new(Pos::new(0.0, 0.0, 100.0), 0.0);
    for _ in 0..100 {
        controller.step_grounded(0.025, 0.0, 0.0, ground);
    }
    assert!(
        (controller.pitch - target * (1.0 - (-1.0f32).exp())).abs() < 0.01,
        "2.5 s covers 63 %: {} vs {}",
        controller.pitch,
        target
    );
}

#[test]
fn a_terrain_miss_holds_the_last_height() {
    let mut controller = CartController::new(Pos::new(0.0, 0.0, 100.024), 0.0);
    controller.step_grounded(0.1, 0.0, 0.0, |_, _| Some(100.0));
    let (z, pitch, roll) = (controller.pos.z, controller.pitch, controller.roll);
    for _ in 0..20 {
        controller.step_grounded(0.05, 0.0, 0.0, |_, _| None);
    }
    assert_eq!(controller.pos.z, z, "a miss keeps the last wheel heights");
    assert_eq!((controller.pitch, controller.roll), (pitch, roll));
    assert_eq!(
        controller.velocity[2], 0.0,
        "no falling when the terrain is missing"
    );
}

#[test]
fn a_synthetic_deck_over_the_terrain_keeps_the_wagon_on_it() {
    // The wheel probes see a deck at 105 over terrain at 100: the body must sit
    // on the deck (+0.024), not fall to the terrain below it.
    let deck = |_: f32, _: f32| Some(105.0);
    let mut controller = CartController::new(Pos::new(0.0, 0.0, 105.024), 0.0);
    for _ in 0..100 {
        controller.step_grounded(0.05, 0.0, 0.0, deck);
    }
    assert!(
        (controller.pos.z - 105.024).abs() < 1e-3,
        "{}",
        controller.pos.z
    );
}

#[test]
fn no_terrain_and_no_surface_hold_the_wagon_height() {
    // Off the map: no terrain cell and no collision surface. floor_at's on-foot
    // fallback would return the body height and the cart would climb by
    // REST_OFFSET each frame; the wheel query must return None instead.
    let terrain = TerrainQuery::new("__cart_no_terrain_test__");
    let collision = WorldCollision::new("__cart_no_terrain_test__", terrain.clone());
    let mut controller = CartController::new(Pos::new(-100.0, -100.0, 100.0), 0.0);
    for _ in 0..100 {
        let body_z = controller.pos.z;
        controller.step_grounded(0.05, 0.0, 0.0, |x, y| {
            wheel_ground(&terrain, Some(&collision), x, y, body_z)
        });
    }
    assert_eq!(controller.pos.z, 100.0, "no support must not climb");
    assert_eq!(controller.velocity[2], 0.0);
}

#[test]
fn a_wheel_probe_takes_a_bridge_deck_over_the_terrain() {
    if !aa_world::pak_available() {
        return;
    }
    let terrain = TerrainQuery::new("main_world");
    let collision = WorldCollision::new("main_world", terrain.clone());
    // Lutesong Harbor's arch bridge (aa-world's walk_over_the_bridge).
    let Some(_cell) = collision.build_blocking(20, 10) else {
        return;
    };
    let centre = Vec2::new(21_091.0, 10_551.2);
    let axis = Vec2::new(0.98, 0.19).normalize();
    let point = centre + axis * 9.0;
    let bed = terrain
        .ground_at(point.x, point.y)
        .expect("terrain under the bridge")
        .height;
    let deck = support_at(
        &terrain,
        &collision,
        Vec3::new(point.x, point.y, 125.0),
        STEP_UP,
    )
    .expect("the bridge deck");
    assert!(deck > bed + 1.0, "deck {deck}, bed {bed}");
    let wheel = wheel_ground(&terrain, Some(&collision), point.x, point.y, deck).unwrap();
    assert!(
        (wheel - deck).abs() < 0.05,
        "wheel probe {wheel}, deck {deck}, bed {bed}"
    );
}

#[test]
fn the_wire_vertical_velocity_is_negative_downhill() {
    // h = 100 - 0.1 y: driving +Y (yaw 0) is downhill.
    let slope = |_: f32, y: f32| Some(100.0 - 0.1 * y);
    let mut controller = CartController::new(Pos::new(0.0, 0.0, 100.024), 0.0);
    for _ in 0..90 {
        controller.step_grounded(1.0 / 45.0, 1.0, 0.0, slope);
    }
    assert!(
        controller.velocity[2] < -0.2 && controller.velocity[2] > -0.6,
        "vel.z {} at {} m/s on a 10 % grade",
        controller.velocity[2],
        controller.speed
    );
    let request = controller.request(7).unwrap();
    assert_eq!(
        request.body().rotation(),
        controller.rotation,
        "the full tilt is sent"
    );
    assert!(controller.rotation[0].abs() > 1e-3);
}

#[derive(Clone, Copy)]
struct SlopeSample {
    ms: i64,
    x: f32,
    y: f32,
    z: f32,
    pitch: f32,
    roll: f32,
    yaw: f32,
    /// Terrain at the cart origin sampled by the capture (capture data; the test
    /// reads the four wheel heights from the user's pak at run time).
    terrain: f32,
}

/// e_slope from the 2026-10-29 s67 capture (58 samples, server coordinates).
#[rustfmt::skip]
const E_SLOPE: [SlopeSample; 58] = [
    SlopeSample { ms: 1793236953628, x: 10473.9673, y: 15568.3350, z: 229.4695, pitch: -0.06820, roll: 0.06930, yaw: -2.94850, terrain: 229.501 },
    SlopeSample { ms: 1793236953739, x: 10473.9685, y: 15568.3291, z: 229.4695, pitch: -0.06730, roll: 0.06840, yaw: -2.94840, terrain: 229.501 },
    SlopeSample { ms: 1793236953782, x: 10473.9709, y: 15568.3174, z: 229.4689, pitch: -0.06690, roll: 0.06800, yaw: -2.94840, terrain: 229.501 },
    SlopeSample { ms: 1793236953883, x: 10473.9792, y: 15568.2764, z: 229.4668, pitch: -0.06620, roll: 0.06720, yaw: -2.94830, terrain: 229.501 },
    SlopeSample { ms: 1793236953975, x: 10473.9919, y: 15568.2119, z: 229.4640, pitch: -0.06540, roll: 0.06640, yaw: -2.94820, terrain: 229.501 },
    SlopeSample { ms: 1793236954071, x: 10474.0095, y: 15568.1216, z: 229.4612, pitch: -0.06470, roll: 0.06560, yaw: -2.94820, terrain: 229.501 },
    SlopeSample { ms: 1793236954182, x: 10474.0320, y: 15568.0073, z: 229.4584, pitch: -0.06400, roll: 0.06490, yaw: -2.94810, terrain: 229.501 },
    SlopeSample { ms: 1793236954272, x: 10474.0593, y: 15567.8701, z: 229.4557, pitch: -0.06340, roll: 0.06410, yaw: -2.94800, terrain: 229.497 },
    SlopeSample { ms: 1793236954381, x: 10474.0911, y: 15567.7095, z: 229.4530, pitch: -0.06280, roll: 0.06340, yaw: -2.94800, terrain: 229.492 },
    SlopeSample { ms: 1793236954474, x: 10474.1277, y: 15567.5244, z: 229.4499, pitch: -0.06210, roll: 0.06260, yaw: -2.94790, terrain: 229.487 },
    SlopeSample { ms: 1793236954577, x: 10474.1687, y: 15567.3169, z: 229.4450, pitch: -0.06160, roll: 0.06200, yaw: -2.94780, terrain: 229.480 },
    SlopeSample { ms: 1793236954681, x: 10474.2146, y: 15567.0864, z: 229.4378, pitch: -0.06120, roll: 0.06140, yaw: -2.94780, terrain: 229.474 },
    SlopeSample { ms: 1793236954773, x: 10474.2654, y: 15566.8310, z: 229.4281, pitch: -0.06080, roll: 0.06090, yaw: -2.94780, terrain: 229.466 },
    SlopeSample { ms: 1793236954879, x: 10474.3210, y: 15566.5522, z: 229.4167, pitch: -0.06050, roll: 0.06050, yaw: -2.94770, terrain: 229.458 },
    SlopeSample { ms: 1793236954978, x: 10474.3814, y: 15566.2500, z: 229.4043, pitch: -0.06020, roll: 0.06010, yaw: -2.94770, terrain: 229.448 },
    SlopeSample { ms: 1793236955067, x: 10474.4465, y: 15565.9233, z: 229.3912, pitch: -0.06000, roll: 0.05980, yaw: -2.94770, terrain: 229.436 },
    SlopeSample { ms: 1793236955180, x: 10474.5161, y: 15565.5728, z: 229.3772, pitch: -0.05980, roll: 0.05950, yaw: -2.94760, terrain: 229.414 },
    SlopeSample { ms: 1793236955275, x: 10474.5911, y: 15565.1953, z: 229.3616, pitch: -0.05970, roll: 0.05930, yaw: -2.94760, terrain: 229.390 },
    SlopeSample { ms: 1793236955381, x: 10474.6685, y: 15564.8062, z: 229.3429, pitch: -0.05970, roll: 0.05920, yaw: -2.94760, terrain: 229.365 },
    SlopeSample { ms: 1793236955474, x: 10474.7466, y: 15564.4141, z: 229.3208, pitch: -0.05970, roll: 0.05910, yaw: -2.94760, terrain: 229.340 },
    SlopeSample { ms: 1793236955568, x: 10474.8252, y: 15564.0210, z: 229.2967, pitch: -0.05970, roll: 0.05900, yaw: -2.94760, terrain: 229.315 },
    SlopeSample { ms: 1793236955677, x: 10474.9031, y: 15563.6309, z: 229.2724, pitch: -0.05970, roll: 0.05890, yaw: -2.94760, terrain: 229.279 },
    SlopeSample { ms: 1793236955773, x: 10474.9810, y: 15563.2437, z: 229.2485, pitch: -0.05980, roll: 0.05890, yaw: -2.94760, terrain: 229.242 },
    SlopeSample { ms: 1793236955885, x: 10475.0591, y: 15562.8511, z: 229.2252, pitch: -0.05980, roll: 0.05880, yaw: -2.94760, terrain: 229.203 },
    SlopeSample { ms: 1793236955975, x: 10475.1375, y: 15562.4600, z: 229.2017, pitch: -0.05980, roll: 0.05880, yaw: -2.94760, terrain: 229.157 },
    SlopeSample { ms: 1793236956072, x: 10475.2146, y: 15562.0757, z: 229.1763, pitch: -0.05990, roll: 0.05880, yaw: -2.94760, terrain: 229.111 },
    SlopeSample { ms: 1793236956180, x: 10475.2910, y: 15561.6938, z: 229.1488, pitch: -0.05990, roll: 0.05880, yaw: -2.94760, terrain: 229.076 },
    SlopeSample { ms: 1793236956273, x: 10475.3679, y: 15561.3135, z: 229.1199, pitch: -0.06000, roll: 0.05880, yaw: -2.94760, terrain: 229.042 },
    SlopeSample { ms: 1793236956380, x: 10475.4822, y: 15560.7461, z: 229.0764, pitch: -0.06010, roll: 0.05900, yaw: -2.94760, terrain: 228.992 },
    SlopeSample { ms: 1793236956476, x: 10475.5200, y: 15560.5571, z: 229.0639, pitch: -0.06020, roll: 0.05920, yaw: -2.94760, terrain: 228.976 },
    SlopeSample { ms: 1793236956584, x: 10475.6340, y: 15559.9873, z: 229.0251, pitch: -0.06060, roll: 0.06040, yaw: -2.94770, terrain: 228.914 },
    SlopeSample { ms: 1793236956696, x: 10475.7112, y: 15559.6040, z: 228.9913, pitch: -0.06100, roll: 0.06190, yaw: -2.94770, terrain: 228.871 },
    SlopeSample { ms: 1793236956790, x: 10475.7854, y: 15559.2383, z: 228.9545, pitch: -0.06160, roll: 0.06420, yaw: -2.94790, terrain: 228.830 },
    SlopeSample { ms: 1793236956897, x: 10475.8547, y: 15558.8984, z: 228.9139, pitch: -0.06250, roll: 0.06720, yaw: -2.94800, terrain: 228.792 },
    SlopeSample { ms: 1793236956991, x: 10475.9207, y: 15558.5786, z: 228.8683, pitch: -0.06380, roll: 0.07040, yaw: -2.94810, terrain: 228.757 },
    SlopeSample { ms: 1793236957083, x: 10475.9822, y: 15558.2803, z: 228.8227, pitch: -0.06560, roll: 0.07400, yaw: -2.94830, terrain: 228.724 },
    SlopeSample { ms: 1793236957194, x: 10476.0386, y: 15558.0083, z: 228.7811, pitch: -0.06790, roll: 0.07790, yaw: -2.94840, terrain: 228.704 },
    SlopeSample { ms: 1793236957286, x: 10476.0896, y: 15557.7632, z: 228.7441, pitch: -0.07050, roll: 0.08210, yaw: -2.94850, terrain: 228.661 },
    SlopeSample { ms: 1793236957398, x: 10476.1357, y: 15557.5425, z: 228.7105, pitch: -0.07340, roll: 0.08660, yaw: -2.94870, terrain: 228.624 },
    SlopeSample { ms: 1793236957489, x: 10476.1768, y: 15557.3447, z: 228.6790, pitch: -0.07660, roll: 0.09140, yaw: -2.94890, terrain: 228.590 },
    SlopeSample { ms: 1793236957584, x: 10476.2136, y: 15557.1719, z: 228.6482, pitch: -0.08000, roll: 0.09660, yaw: -2.94910, terrain: 228.562 },
    SlopeSample { ms: 1793236957694, x: 10476.2463, y: 15557.0215, z: 228.6172, pitch: -0.08360, roll: 0.10160, yaw: -2.94930, terrain: 228.537 },
    SlopeSample { ms: 1793236957791, x: 10476.2747, y: 15556.8911, z: 228.5882, pitch: -0.08740, roll: 0.10660, yaw: -2.94950, terrain: 228.515 },
    SlopeSample { ms: 1793236957899, x: 10476.2981, y: 15556.7827, z: 228.5634, pitch: -0.09130, roll: 0.11150, yaw: -2.94970, terrain: 228.497 },
    SlopeSample { ms: 1793236957990, x: 10476.3167, y: 15556.6992, z: 228.5434, pitch: -0.09520, roll: 0.11630, yaw: -2.94990, terrain: 228.484 },
    SlopeSample { ms: 1793236958086, x: 10476.3296, y: 15556.6401, z: 228.5287, pitch: -0.09890, roll: 0.12060, yaw: -2.95010, terrain: 228.474 },
    SlopeSample { ms: 1793236958195, x: 10476.3398, y: 15556.5947, z: 228.5150, pitch: -0.10420, roll: 0.12660, yaw: -2.95050, terrain: 228.466 },
    SlopeSample { ms: 1793236958288, x: 10476.3408, y: 15556.5908, z: 228.5125, pitch: -0.10580, roll: 0.12840, yaw: -2.95060, terrain: 228.466 },
    SlopeSample { ms: 1793236958398, x: 10476.3408, y: 15556.5913, z: 228.5084, pitch: -0.11030, roll: 0.13340, yaw: -2.95090, terrain: 228.466 },
    SlopeSample { ms: 1793236958492, x: 10476.3408, y: 15556.5913, z: 228.5070, pitch: -0.11290, roll: 0.13640, yaw: -2.95110, terrain: 228.466 },
    SlopeSample { ms: 1793236958585, x: 10476.3411, y: 15556.5913, z: 228.5060, pitch: -0.11530, roll: 0.13910, yaw: -2.95130, terrain: 228.466 },
    SlopeSample { ms: 1793236958696, x: 10476.3413, y: 15556.5913, z: 228.5051, pitch: -0.11750, roll: 0.14150, yaw: -2.95150, terrain: 228.466 },
    SlopeSample { ms: 1793236958787, x: 10476.3413, y: 15556.5913, z: 228.5041, pitch: -0.11960, roll: 0.14380, yaw: -2.95170, terrain: 228.466 },
    SlopeSample { ms: 1793236958899, x: 10476.3416, y: 15556.5913, z: 228.5031, pitch: -0.12150, roll: 0.14590, yaw: -2.95190, terrain: 228.466 },
    SlopeSample { ms: 1793236958992, x: 10476.3416, y: 15556.5913, z: 228.5022, pitch: -0.12320, roll: 0.14780, yaw: -2.95210, terrain: 228.466 },
    SlopeSample { ms: 1793236959090, x: 10476.3418, y: 15556.5913, z: 228.5013, pitch: -0.12480, roll: 0.14960, yaw: -2.95220, terrain: 228.466 },
    SlopeSample { ms: 1793236959195, x: 10476.3420, y: 15556.5913, z: 228.5005, pitch: -0.12640, roll: 0.15130, yaw: -2.95240, terrain: 228.466 },
    SlopeSample { ms: 1793236959287, x: 10476.3420, y: 15556.5913, z: 228.4998, pitch: -0.12780, roll: 0.15290, yaw: -2.95250, terrain: 228.466 },
];

#[test]
fn the_captured_slope_drive_matches_pitch_roll_and_z() {
    // Replays the captured e_slope poses through the controller, starting from
    // the captured tilt. The rows are capture data; the four wheel heights are
    // read at run time from the user's pak (skipped without AA_PAK). The capture's
    // own `terrain` sample at the cart origin is kept as a check of that read.
    // Measured after the body-origin height fix: mean |error| 0.007 / 0.010 rad
    // and 0.029 m; max 0.014 / 0.022 rad and 0.043 m. Tolerance: mean <= 0.02 rad
    // (pitch/roll) and 0.035 m (z); max <= 0.03 rad and 0.05 m. The capture's tilt
    // lags the local plane (the body still relaxes from the previous slope), so a
    // per-sample exact match is not expected.
    if !aa_world::pak_available() {
        return;
    }
    let terrain = TerrainQuery::new("main_world");
    let first = &E_SLOPE[0];
    let mut controller = CartController::new(Pos::new(first.x, first.y, first.z), first.yaw);
    controller.pitch = first.pitch;
    controller.roll = first.roll;
    let mut prev = first.ms;
    let mut errors = Vec::new();
    for (i, row) in E_SLOPE.iter().enumerate() {
        let origin = terrain
            .ground_at(row.x, row.y)
            .expect("the captured route lies in the pak's terrain");
        assert!(
            (origin.height - row.terrain).abs() < 0.05,
            "row {i}: pak terrain {:.3}, capture terrain {:.3}",
            origin.height,
            row.terrain
        );
        if i > 0 {
            let dt = ((row.ms - prev) as f32 / 1000.0).clamp(1e-4, 0.2);
            controller.pos.x = row.x;
            controller.pos.y = row.y;
            controller.yaw = row.yaw;
            controller.follow_ground(dt, &|x, y| terrain.ground_at(x, y).map(|g| g.height));
        }
        prev = row.ms;
        if i >= 3 {
            errors.push((
                (controller.pitch - row.pitch).abs(),
                (controller.roll - row.roll).abs(),
                (controller.pos.z - row.z).abs(),
            ));
        }
    }
    let n = errors.len() as f32;
    let mean = |j: usize| errors.iter().map(|e| [e.0, e.1, e.2][j]).sum::<f32>() / n;
    let max = |j: usize| {
        errors
            .iter()
            .map(|e| [e.0, e.1, e.2][j])
            .fold(0.0f32, f32::max)
    };
    assert!(mean(0) <= 0.02, "mean pitch error {}", mean(0));
    assert!(mean(1) <= 0.02, "mean roll error {}", mean(1));
    assert!(mean(2) <= 0.035, "mean z error {}", mean(2));
    assert!(max(0) <= 0.03, "max pitch error {}", max(0));
    assert!(max(1) <= 0.03, "max roll error {}", max(1));
    assert!(max(2) <= 0.05, "max z error {}", max(2));
    let last = E_SLOPE.last().unwrap();
    assert!(
        controller.pitch < -0.08,
        "downhill nose-down pitch {}",
        controller.pitch
    );
    assert!(controller.roll > 0.08, "downhill roll {}", controller.roll);
    assert!(
        (controller.pitch - last.pitch).abs() <= 0.03,
        "the replay ends on the captured tilt: {} vs {}",
        controller.pitch,
        last.pitch
    );
}
