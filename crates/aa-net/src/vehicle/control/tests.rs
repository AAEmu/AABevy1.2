use super::*;
use crate::game::ClientMsg;
use crate::vehicle::move_unit::MOVE_UNIT_SIZE;
use crate::wire::Reader;
use serde_json::Value;

fn hex(raw: &str) -> Vec<u8> {
    (0..raw.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&raw[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn vehicle_move_body_is_the_1_2_serializer_output() {
    // `move.json` case 5013 is a recorded type-2 `FUN_397c0470` output. The
    // request must add only the BC3 object id in front of it.
    let doc: Value = serde_json::from_str(include_str!("../goldens/move.json")).unwrap();
    let case = doc
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["type"].as_u64() == Some(2))
        .expect("type-2 golden");
    let mut body = MoveUnit::default();
    body.bytes
        .copy_from_slice(&hex(case["input"].as_str().unwrap()));
    let expected: Vec<u8> = case["calls"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|call| hex(call["value"].as_str().unwrap()))
        .collect();
    assert_eq!(expected.len(), 60); // header 6 + position 9 + velocity/rotation 12 + vehicle 33

    let request = VehicleMoveRequest::new(0x445566, body).unwrap();
    let encoded = ClientMsg::MoveVehicle(request.clone()).encode_body();
    assert_eq!(&encoded[..3], &[0x66, 0x55, 0x44]);
    assert_eq!(&encoded[3..], expected);
    assert_eq!(ClientMsg::MoveVehicle(request).opcode(), 0x089);
}

#[test]
fn vehicle_move_from_parts_matches_the_1_2_layout() {
    // feat/vehicle-drive d34a6a0d's case, kept where the 1.2 layout agrees:
    // position (10, 20, 3), yaw 0.7, angular velocity (0.1, 0.2, 0.3),
    // steering -0.4, wheels 1.5 / 2.5. Its velocity [1024, -2048, 0] was in
    // 1/1024 m/s (1.0 and -2.0 m/s), so it is given here in m/s; the 1.2 wire
    // scale (30/32767, i.e. m/s * 1092) is the codec's, not 1024 or 2048.
    let half = 0.7f32 * 0.5;
    let request = VehicleMoveRequest::from_parts(
        0x445566,
        42,
        Pos::new(10.0, 20.0, 3.0),
        [0.0, 0.0, half.sin(), half.cos()],
        [1.0, -2.0, 0.0],
        [0.1, 0.2, 0.3],
        -0.4,
        &[1.5, 2.5],
    )
    .unwrap();
    let body = ClientMsg::MoveVehicle(request).encode_body();
    assert_eq!(
        body,
        [
            0x66, 0x55, 0x44, // BC3 vehicle object
            0x02, // move type 2 (land vehicle)
            0x2A, 0x00, 0x00, 0x00, // time
            0x00, // flags
            0x00, 0x14, 0x00, 0x00, 0x28, 0x00, 0x2E, 0x92, 0x01, // position
            0x44, 0x04, 0x78, 0xF7, 0x00, 0x00, // velocity, 1/1092 m/s
            0x00, 0x00, 0x00, 0x00, 0xE4, 0x2B, // rotation x, y, z (i16)
            0xCD, 0xCC, 0xCC, 0x3D, 0xCD, 0xCC, 0x4C, 0x3E, 0x9A, 0x99, 0x99, 0x3E, // angVel
            0xCD, 0xCC, 0xCC, 0xBE, // steering
            0x02, // wheel count
            0x00, 0x00, 0xC0, 0x3F, 0x00, 0x00, 0x20, 0x40, // wheel angVel
        ]
    );

    let mut r = Reader::new(&body);
    assert_eq!(r.bc().unwrap(), 0x445566);
    let decoded = crate::model::Movement::read(&mut r).unwrap();
    assert_eq!((decoded.move_type, decoded.time, decoded.flags), (2, 42, 0));
    assert!((decoded.pos.x - 10.0).abs() < 1e-3);
    assert!((decoded.pos.y - 20.0).abs() < 1e-3);
    assert!((decoded.pos.z - 3.0).abs() < 1e-3);
    assert_eq!(decoded.velocity, [1092, -2184, 0]);
    let vehicle = decoded.vehicle.as_ref().expect("type-2 body");
    assert_eq!(vehicle.rotation, [0, 0, 0x2BE4]);
    assert_eq!(vehicle.angular_velocity, [0.1, 0.2, 0.3]);
    assert_eq!(vehicle.steering, -0.4);
    assert_eq!(vehicle.wheel_ang_vel, vec![1.5, 2.5]);
    assert!(r.is_empty());
}

#[test]
fn vehicle_move_rejects_bad_ids_bodies_and_wheel_counts() {
    for id in [0, 0x0100_0000, u32::MAX] {
        let mut body = MoveUnit::default();
        body.set_move_type(2);
        assert!(VehicleMoveRequest::new(id, body).is_err());
    }
    for move_type in [0, 1, 3, 4, 5] {
        let mut body = MoveUnit::default();
        body.set_move_type(move_type);
        assert!(VehicleMoveRequest::new(1, body).is_err());
    }
    let mut body = MoveUnit::default();
    body.set_move_type(2);
    body.set_wheel_count(19);
    assert!(VehicleMoveRequest::new(1, body).is_err());
    let mut body = MoveUnit::default();
    body.set_move_type(2);
    body.set_vehicle_steering(f32::NAN);
    assert!(VehicleMoveRequest::new(1, body).is_err());
    assert_eq!(MOVE_UNIT_SIZE, 0x130);
}
