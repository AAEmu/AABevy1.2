use super::*;
use aa_net::model::UnitKind;

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
