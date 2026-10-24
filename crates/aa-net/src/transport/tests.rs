//! Literal native field-order evidence uses explicit modeled archive leaves.
//! Rust truncation and raw-byte behavior do not establish native archive semantics.

use super::*;
use crate::game::ServerMsg;

pub(crate) const ADJACENT: &[u8] =
    include_bytes!("../../tests/fixtures/transport-fixed/adjacent.dd04.frame.bin");
const INNER: &[u8] = include_bytes!("../../tests/fixtures/transport-fixed/adjacent.inner.bin");

fn hex(raw: &str) -> Vec<u8> {
    (0..raw.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&raw[i..i + 2], 16).unwrap())
        .collect()
}

pub(crate) fn expected_adjacent() -> Vec<TransportReceipt> {
    use TransportReceipt::*;
    let rope = Some(ReceivedRopeState {
        length: -3.5,
        teared_wire_byte: 0,
        cutouted_wire_byte: 1,
    });
    let mut receipts = vec![
        ControllerState {
            obj_id: 0x112233,
            sc_type: 0,
            rope,
        },
        ControllerState {
            obj_id: 0x112233,
            sc_type: 1,
            rope: None,
        },
    ];
    receipts.extend([0, 1, 2, 255].map(|flying_wire_byte| FlyingState {
        obj_id: 0x112233,
        flying_wire_byte,
    }));
    receipts.extend([
        ControllerState {
            obj_id: 0xffffff,
            sc_type: 255,
            rope: None,
        },
        Hung {
            unit_obj_id: 0x112233,
            target_obj_id: 0xaabbcc,
        },
        Unhung {
            unit_obj_id: 0x112233,
            target_obj_id: 0xaabbcc,
            reason: -1,
        },
        ControllerState {
            obj_id: 0xffffff,
            sc_type: 0,
            rope,
        },
    ]);
    receipts
}

#[test]
fn transport_native_fixed_goldens_reject_every_prefix_and_preserve_sentinels() {
    let mut count = 0;
    for line in include_str!("../../tests/fixtures/transport-fixed/native.tsv")
        .lines()
        .filter(|line| !line.starts_with('#'))
    {
        let columns: Vec<_> = line.split('\t').collect();
        assert_eq!(columns.len(), 10);
        let op = u16::from_str_radix(columns[1], 16).unwrap();
        let body = hex(columns[2]);
        let values: Vec<i64> = columns[3..].iter().map(|v| v.parse().unwrap()).collect();
        let actor = values[0] as u32;
        let expected = match op {
            0x06d => TransportReceipt::ControllerState {
                obj_id: actor,
                sc_type: values[1] as u8,
                rope: (values[1] == 0).then_some(ReceivedRopeState {
                    length: f32::from_bits(values[2] as u32),
                    teared_wire_byte: values[3] as u8,
                    cutouted_wire_byte: values[4] as u8,
                }),
            },
            0x07a => TransportReceipt::FlyingState {
                obj_id: actor,
                flying_wire_byte: values[3] as u8,
            },
            0x137 => TransportReceipt::Hung {
                unit_obj_id: actor,
                target_obj_id: values[5] as u32,
            },
            0x138 => TransportReceipt::Unhung {
                unit_obj_id: actor,
                target_obj_id: values[5] as u32,
                reason: values[6] as i32,
            },
            _ => panic!("unexpected golden opcode"),
        };
        let mut supplied = body.clone();
        supplied.extend_from_slice(&[0xde, 0xad, 0xbe, 0xef]);
        let mut reader = Reader::new(&supplied);
        assert_eq!(TransportReceipt::read(op, &mut reader).unwrap(), expected);
        assert_eq!(reader.pos(), body.len());
        assert_eq!(reader.rest(), [0xde, 0xad, 0xbe, 0xef]);
        let mut frame = vec![0xdd, 1, 0, 0];
        frame.extend_from_slice(&op.to_le_bytes());
        frame.extend_from_slice(&body);
        let decoded = ServerMsg::decode(&frame).unwrap();
        assert_eq!(decoded.name(), expected.name());
        assert_eq!(decoded, ServerMsg::Transport(expected));
        for cut in 0..body.len() {
            assert!(
                TransportReceipt::read(op, &mut Reader::new(&body[..cut])).is_err(),
                "{} cut{cut}",
                columns[0]
            );
            assert!(ServerMsg::decode(&frame[..6 + cut]).is_err());
        }
        count += 1;
    }
    assert_eq!(count, 30);
}

#[test]
fn transport_unknown_controller_types_never_consume_a_rope_suffix() {
    for sc_type in 1..=u8::MAX {
        let body = [
            0xff, 0xff, 0xff, sc_type, 0, 0, 0x80, 0, 0x33, 0x22, 0x11, 255,
        ];
        let mut reader = Reader::new(&body);
        assert_eq!(
            TransportReceipt::read(0x06d, &mut reader).unwrap(),
            TransportReceipt::ControllerState {
                obj_id: 0xffffff,
                sc_type,
                rope: None
            }
        );
        assert_eq!(reader.pos(), 4);
        assert_eq!(reader.rest(), &body[4..]);
    }
    for op in [0x06d, 0x07a, 0x137, 0x138] {
        assert!(TransportReceipt::handles(op));
    }
    for op in [0x08a, 0x08b, 0x139, 0x13a, 0x1aa] {
        assert!(!TransportReceipt::handles(op));
        assert!(TransportReceipt::read(op, &mut Reader::new(&[0; 32])).is_err());
    }
}

#[test]
fn transport_all_bool_wire_bytes_and_float_bits_remain_raw() {
    for raw in 0..=u8::MAX {
        let body = [0, 0, 0, raw];
        assert_eq!(
            TransportReceipt::read(0x07a, &mut Reader::new(&body)).unwrap(),
            TransportReceipt::FlyingState {
                obj_id: 0,
                flying_wire_byte: raw
            }
        );
        for bits in [
            0x80000000u32,
            0xc0600000,
            0x7f800000,
            0xff800000,
            0x7fc12345,
        ] {
            let mut body = vec![0x33, 0x22, 0x11, 0];
            body.extend_from_slice(&bits.to_le_bytes());
            body.extend_from_slice(&[raw, !raw]);
            let TransportReceipt::ControllerState {
                obj_id: 0x112233,
                sc_type: 0,
                rope: Some(rope),
            } = TransportReceipt::read(0x06d, &mut Reader::new(&body)).unwrap()
            else {
                panic!("missing rope")
            };
            assert_eq!(rope.length.to_bits(), bits);
            assert_eq!(rope.teared_wire_byte, raw);
            assert_eq!(rope.cutouted_wire_byte, !raw);
        }
    }
    for reason in [i32::MIN, -1, 0, i32::MAX] {
        let mut body = vec![0, 0, 0, 0xff, 0xff, 0xff];
        body.extend_from_slice(&reason.to_le_bytes());
        assert_eq!(
            TransportReceipt::read(0x138, &mut Reader::new(&body)).unwrap(),
            TransportReceipt::Unhung {
                unit_obj_id: 0,
                target_obj_id: 0xffffff,
                reason
            }
        );
    }
}

#[test]
fn transport_adjacent_dd04_preserves_exact_cursor_count_and_following_points() {
    assert_eq!(
        usize::from(u16::from_le_bytes([ADJACENT[0], ADJACENT[1]])),
        ADJACENT.len() - 2
    );
    assert_eq!(&ADJACENT[2..6], [0xdd, 4, 11, 0]);
    let raw = crate::compression::inflate_batch(&ADJACENT[6..]).unwrap();
    assert_eq!(raw, INNER);
    let mut r = Reader::new(&raw);
    let expected = expected_adjacent();
    let sizes = [10, 4, 4, 4, 4, 4, 4, 6, 10, 10];
    let mut cursor = 0;
    for (receipt, size) in expected.iter().zip(sizes) {
        assert_eq!(r.u16().unwrap(), 0);
        let op = r.u16().unwrap();
        assert_eq!(&TransportReceipt::read(op, &mut r).unwrap(), receipt);
        cursor += 4 + size;
        assert_eq!(r.pos(), cursor);
    }
    assert_eq!(r.u16().unwrap(), 0);
    assert_eq!(r.u16().unwrap(), 0x0ba);
    assert_eq!(r.bc().unwrap(), 0xaabbcc);
    assert_eq!(r.i32().unwrap(), 12300);
    assert_eq!(r.i32().unwrap(), -45600);
    assert!(r.is_empty());
    let mut expected: Vec<_> = expected.into_iter().map(ServerMsg::Transport).collect();
    expected.push(ServerMsg::UnitPoints {
        obj_id: 0xaabbcc,
        hp: 123,
        mp: -456,
    });
    assert_eq!(
        ServerMsg::decode(&ADJACENT[2..]).unwrap(),
        ServerMsg::Batch(expected)
    );
}
