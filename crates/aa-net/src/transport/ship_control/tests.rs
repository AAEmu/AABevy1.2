use super::*;
use crate::game::{ClientMsg, LEVEL_GAME};
use crate::wire::Reader;

#[test]
fn ship_control_matches_retained_original_packet_and_movement_outputs() {
    let mut count = 0;
    for line in include_str!("../../../tests/fixtures/ship-control/native.tsv")
        .lines()
        .filter(|line| !line.starts_with('#'))
    {
        let columns: Vec<_> = line.split('\t').collect();
        assert_eq!(columns.len(), 10);
        let literal: Vec<_> = (0..columns[2].len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&columns[2][i..i + 2], 16).unwrap())
            .collect();
        let controller = match (columns[6], columns[7]) {
            ("-", "-") => None,
            (sc_type, phase) => Some((sc_type.parse().unwrap(), phase.parse().unwrap())),
        };
        let request = ShipControlRequest::new(
            columns[3].parse().unwrap(),
            columns[4].parse().unwrap(),
            columns[5].parse().unwrap(),
            controller,
            columns[8].parse().unwrap(),
            columns[9].parse().unwrap(),
        )
        .unwrap();
        let expected = match columns[1] {
            "packet" => literal,
            // Only the movement suffix is a native golden for these rows.
            "movement" => [&[0x33, 0x22, 0x11][..], &literal].concat(),
            other => panic!("unknown fixture scope {other}"),
        };
        assert_eq!(
            ClientMsg::ShipControl(request).encode_body(),
            expected,
            "{}",
            columns[0]
        );
        // Inspect exact boundaries without projecting the request to a transform.
        let mut r = Reader::new(&expected);
        assert_eq!(r.bc().unwrap(), request.slave_obj_id());
        assert_eq!(r.u8().unwrap(), 4);
        assert_eq!(r.u32().unwrap(), request.time());
        assert_eq!(r.u8().unwrap(), request.flags());
        if let Some((sc_type, phase)) = request.controller() {
            assert_eq!(r.u32().unwrap(), sc_type);
            assert_eq!(r.u8().unwrap(), phase);
        }
        assert_eq!(r.i8().unwrap(), request.throttle());
        assert_eq!(r.i8().unwrap(), request.steering());
        assert!(r.is_empty());
        count += 1;
    }
    assert_eq!(count, 18);
}

/// The eight bodies of the 2026-10-29 S9 capture (rowboat 8911): idle, both
/// throttle levels and the steering ramp/release samples.
#[test]
fn ship_control_reproduces_the_live_s910_type_4_bodies_byte_for_byte() {
    let mut count = 0;
    for line in include_str!("../../../tests/fixtures/ship-control/s910-native.tsv")
        .lines()
        .filter(|line| !line.starts_with('#'))
    {
        let columns: Vec<_> = line.split('\t').collect();
        assert_eq!(columns.len(), 7, "{}", columns[0]);
        let expected: Vec<u8> = columns[1]
            .split_whitespace()
            .map(|b| u8::from_str_radix(b, 16).unwrap())
            .collect();
        let request = ShipControlRequest::new(
            columns[2].parse().unwrap(),
            columns[3].parse().unwrap(),
            columns[4].parse().unwrap(),
            None,
            columns[5].parse().unwrap(),
            columns[6].parse().unwrap(),
        )
        .unwrap();
        assert_eq!(
            ClientMsg::ShipControl(request).encode_body(),
            expected,
            "{}",
            columns[0]
        );
        count += 1;
    }
    assert_eq!(count, 8);
}

#[test]
fn ship_control_literal_dd01_frames_have_only_the_fixed_body_for_every_counter() {
    // Frame payloads, excluding GameConn's separate u16 length prefix.
    let cases: [(ShipControlRequest, &[u8]); 2] = [
        (
            ShipControlRequest::new(0x112233, 0x89abcdef, 2, None, -128, 127).unwrap(),
            &[
                0xdd, 1, 0, 0, 0x89, 0, 0x33, 0x22, 0x11, 4, 0xef, 0xcd, 0xab, 0x89, 2, 0x80, 0x7f,
            ],
        ),
        (
            ShipControlRequest::new(0xffffff, 0x89abcdef, 0xff, Some((0xfedcba98, 255)), -1, 1)
                .unwrap(),
            &[
                0xdd, 1, 0, 0, 0x89, 0, 0xff, 0xff, 0xff, 4, 0xef, 0xcd, 0xab, 0x89, 0xff, 0x98,
                0xba, 0xdc, 0xfe, 0xff, 0xff, 1,
            ],
        ),
    ];
    for (request, literal) in cases {
        let message = ClientMsg::ShipControl(request);
        assert_eq!(message.level(), LEVEL_GAME);
        assert_eq!(message.opcode(), 0x089);
        assert_eq!(message.name(), "CSMoveUnitPacket");
        assert_eq!(
            message.encode_body().len(),
            if request.controller().is_some() {
                16
            } else {
                11
            }
        );
        for counter in 0..=255 {
            let mut expected = literal.to_vec();
            expected[3] = counter;
            assert_eq!(message.encode(counter), expected);
        }
    }
}

#[test]
fn ship_control_requires_nonzero_bc3_ids_without_truncation() {
    for bad in [0, 0x0100_0000, 0x0100_0001, 0xff11_2233, u32::MAX]
        .into_iter()
        .chain((24..32).map(|bit| 1 << bit))
    {
        assert!(ShipControlRequest::new(bad, 0, 0, None, 0, 0).is_err());
    }
    for id in [1, 255, 256, 65535, 65536, 0x800000, 0xffffff] {
        let request = ShipControlRequest::new(id, 0, 0, None, 0, 0).unwrap();
        assert_eq!(request.slave_obj_id(), id);
        assert_eq!(
            &ClientMsg::ShipControl(request).encode_body()[..3],
            &id.to_le_bytes()[..3]
        );
    }
}

#[test]
fn ship_control_all_flags_require_exact_controller_presence_without_normalization() {
    for flags in 0..=u8::MAX {
        let has_controller = flags & 0x10 != 0;
        for controller in [None, Some((0, 0)), Some((u32::MAX, u8::MAX))] {
            let result = ShipControlRequest::new(1, 0, flags, controller, -1, 1);
            assert_eq!(result.is_ok(), has_controller == controller.is_some());
            if let Ok(request) = result {
                assert_eq!(request.flags(), flags);
                assert_eq!(request.controller(), controller);
                let body = ClientMsg::ShipControl(request).encode_body();
                assert_eq!(body[8], flags);
                assert_eq!(body.len(), if has_controller { 16 } else { 11 });
                if let Some((sc_type, phase)) = controller {
                    assert_eq!(&body[9..13], sc_type.to_le_bytes());
                    assert_eq!(body[13], phase);
                }
                assert_eq!(&body[body.len() - 2..], &[255, 1]);
            }
        }
    }
}

#[test]
fn ship_control_preserves_every_signed_control_pair_in_wire_order() {
    for throttle in i8::MIN..=i8::MAX {
        for steering in i8::MIN..=i8::MAX {
            for (flags, controller) in [(0, None), (0x10, Some((0, 0)))] {
                let request =
                    ShipControlRequest::new(1, 0, flags, controller, throttle, steering).unwrap();
                assert_eq!(request.throttle(), throttle);
                assert_eq!(request.steering(), steering);
                let body = ClientMsg::ShipControl(request).encode_body();
                assert_eq!(&body[body.len() - 2..], &[throttle as u8, steering as u8]);
            }
        }
    }
}

#[test]
fn ship_control_time_controller_and_phase_preserve_all_bits_including_explicit_zeroes() {
    let words = [0, u32::MAX]
        .into_iter()
        .chain((0..32).flat_map(|bit| [1u32 << bit, !(1u32 << bit)]));
    for word in words {
        for phase in 0..=u8::MAX {
            let request =
                ShipControlRequest::new(1, word, 0x10, Some((!word, phase)), 0, 0).unwrap();
            assert_eq!(request.time(), word);
            assert_eq!(request.controller(), Some((!word, phase)));
            let body = ClientMsg::ShipControl(request).encode_body();
            assert_eq!(&body[4..8], word.to_le_bytes());
            assert_eq!(&body[9..13], (!word).to_le_bytes());
            assert_eq!(body[13], phase);
        }
    }
}
