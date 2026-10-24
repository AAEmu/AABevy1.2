//! SC080 wire regressions under the accepted one-byte bool wire convention.
//! Native serializer traces use modeled archive leaves; these Rust checks do not
//! prove CryNetwork normalization, native truncation policy or factory behavior.

use super::InteractEvent;
use crate::game::ServerMsg;
use crate::opcodes::sc;
use crate::wire::Reader;

const ORDINARY: &[u8] = include_bytes!("../../tests/fixtures/npc-status/sc080.dd01.frame.bin");
const ADJACENT: &[u8] = include_bytes!(
    "../../tests/fixtures/npc-status/sc080-four-statuses-then-points.dd04.frame.bin"
);
const INNER: &[u8] =
    include_bytes!("../../tests/fixtures/npc-status/sc080-four-statuses-then-points.inner.bin");

#[test]
fn npc_status_preserves_all_wire_bytes_and_bc3_ids_with_exact_cursor() {
    assert_eq!(sc::SCNpcInteractionStatusChangedPacket, 0x080);
    assert_eq!(sc::name(0x080), "SCNpcInteractionStatusChangedPacket");
    assert!(InteractEvent::handles(0x080));
    for (id, bc) in [
        (0, [0, 0, 0]),
        (1, [1, 0, 0]),
        (0x112233, [0x33, 0x22, 0x11]),
        (0xffffff, [0xff, 0xff, 0xff]),
    ] {
        for raw in 0..=u8::MAX {
            let body = [bc[0], bc[1], bc[2], raw, 0xde, 0xad, 0xbe, 0xef];
            let mut reader = Reader::new(&body);
            let event = InteractEvent::read(0x080, &mut reader).unwrap();
            assert_eq!(
                event,
                InteractEvent::NpcStatusChanged {
                    npc_obj_id: id,
                    status_wire_byte: raw,
                }
            );
            assert_eq!(event.name(), "SCNpcInteractionStatusChanged");
            assert_eq!(reader.pos(), 4);
            assert_eq!(reader.rest(), [0xde, 0xad, 0xbe, 0xef]);
        }
    }
}

#[test]
fn npc_status_mandatory_prefixes_fail_without_publishing_a_partial_receipt() {
    for body in [
        [0x33, 0x22, 0x11, 0],
        [0x33, 0x22, 0x11, 1],
        [0x33, 0x22, 0x11, 2],
        [0x33, 0x22, 0x11, 255],
    ] {
        for cut in 0..body.len() {
            assert!(InteractEvent::read(0x080, &mut Reader::new(&body[..cut])).is_err());
            let mut frame = vec![0xdd, 1, 0, 0, 0x80, 0];
            frame.extend_from_slice(&body[..cut]);
            assert!(ServerMsg::decode(&frame).is_err(), "body cut {cut}");
        }
    }
}

#[test]
fn npc_status_ordinary_frame_decodes_after_the_transport_length_is_removed() {
    assert_eq!(
        ORDINARY,
        [0x0a, 0, 0xdd, 1, 0, 0, 0x80, 0, 0x33, 0x22, 0x11, 1]
    );
    let declared = usize::from(u16::from_le_bytes([ORDINARY[0], ORDINARY[1]]));
    assert_eq!(declared, ORDINARY.len() - 2);
    assert!(ServerMsg::decode(ORDINARY).is_err());
    assert_eq!(
        ServerMsg::decode(&ORDINARY[2..]).unwrap(),
        ServerMsg::Interaction(InteractEvent::NpcStatusChanged {
            npc_obj_id: 0x112233,
            status_wire_byte: 1,
        })
    );
}

#[test]
fn npc_status_adjacent_dd04_preserves_count_order_and_following_points_cursor() {
    assert_eq!(
        usize::from(u16::from_le_bytes([ADJACENT[0], ADJACENT[1]])),
        ADJACENT.len() - 2
    );
    assert_eq!(&ADJACENT[2..6], [0xdd, 4, 5, 0]);
    let inflated = crate::compression::inflate_batch(&ADJACENT[6..]).unwrap();
    assert_eq!(inflated, INNER);
    let mut reader = Reader::new(&inflated);
    let mut expected = Vec::new();
    for (index, raw) in [0, 1, 2, 255].into_iter().enumerate() {
        assert_eq!(reader.u16().unwrap(), 0);
        assert_eq!(reader.u16().unwrap(), 0x080);
        let event = InteractEvent::read(0x080, &mut reader).unwrap();
        assert_eq!(
            event,
            InteractEvent::NpcStatusChanged {
                npc_obj_id: 0x112233,
                status_wire_byte: raw
            }
        );
        assert_eq!(reader.pos(), (index + 1) * 8);
        expected.push(ServerMsg::Interaction(event));
    }
    assert_eq!(reader.u16().unwrap(), 0);
    assert_eq!(reader.u16().unwrap(), 0x0ba);
    assert_eq!(reader.bc().unwrap(), 0xaabbcc);
    assert_eq!(reader.i32().unwrap(), 12300);
    assert_eq!(reader.i32().unwrap(), -45600);
    assert_eq!(reader.pos(), 47);
    assert!(reader.is_empty());
    expected.push(ServerMsg::UnitPoints {
        obj_id: 0xaabbcc,
        hp: 123,
        mp: -456,
    });
    let decoded = ServerMsg::decode(&ADJACENT[2..]).unwrap();
    assert_eq!(decoded, ServerMsg::Batch(expected));
    let ServerMsg::Batch(messages) = decoded else {
        panic!("expected batch")
    };
    assert_eq!(messages.len(), 5);
}
