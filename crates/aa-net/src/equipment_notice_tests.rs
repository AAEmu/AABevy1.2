//! Native transport-equipment notices: bytes and non-settlement boundaries.
use crate::game::ServerMsg;
use crate::mate::MateEvent;
use crate::opcodes::sc;
use crate::session::Event;
use crate::slave::SlaveEvent;
use crate::wire::{Reader, Writer};

fn hex(raw: &str) -> Vec<u8> {
    raw.split_whitespace()
        .flat_map(|word| (0..word.len()).step_by(2).map(move |i| u8::from_str_radix(&word[i..i + 2], 16).unwrap()))
        .collect()
}

fn with_tail(opcode: u16, body: &[u8], expected: ServerMsg) {
    let mut plain = Writer::new();
    plain.raw(&[0xdd, 1, 0, 1]).u16(opcode).raw(body);
    assert_eq!(ServerMsg::decode(&plain.finish()).unwrap(), expected);
    let mut inner = Writer::new();
    inner.u16(0).u16(opcode).raw(body).u16(0).u16(sc::SCOnOffSnowPacket).bool(true);
    let inner = inner.finish();
    let len = inner.len() as u16;
    let mut compressed = Writer::new();
    compressed.raw(&[0xdd, 4]).u16(2).u8(1).u16(len).u16(!len).raw(&inner);
    assert_eq!(
        ServerMsg::decode(&compressed.finish()).unwrap(),
        ServerMsg::Batch(vec![expected, ServerMsg::Snow(true)])
    );
    for cut in 0..body.len() {
        let mut plain = Writer::new();
        plain.raw(&[0xdd, 1, 0, 1]).u16(opcode).raw(&body[..cut]);
        assert!(ServerMsg::decode(&plain.finish()).is_err(), "opcode {opcode:x}, cut {cut}");
    }
}

#[test]
fn slave_equipment_full_items_raw_locations_and_failure_flag_keep_packet_boundaries() {
    // Header matches lane item_effect_packets::SCSlaveEquipmentChangedPacket.Write;
    // full tag9 item preserves its four opaque detail bytes and timing/owner fields.
    let mut w = Writer::new();
    w.raw(&hex("06000000 0500 07000000 01 00000000"));
    w.u32(123)
        .u64(0x1122334455667788)
        .u8(2)
        .u8(3)
        .i32(4)
        .u8(9)
        .raw(&[0x81, 0x82, 0x83, 0x84])
        .i64(10)
        .i32(11)
        .u32(12)
        .u8(13)
        .i64(14)
        .i64(15);
    w.raw(&hex("00 00 fe ff 00"));
    let body = w.finish();
    let mut r = Reader::new(&body);
    let event = SlaveEvent::read(sc::SCSlaveEquipmentChangedPacket, &mut r).unwrap();
    assert!(r.is_empty());
    let SlaveEvent::EquipmentChanged(change) = &event else { panic!() };
    assert_eq!(
        (change.character_id, change.slave_tl_id, change.db_slave_id, change.bts, change.success),
        (6, 5, 7, true, false)
    );
    assert!(change.first_item.is_none());
    assert_eq!(
        (
            change.first_location.container,
            change.first_location.slot,
            change.second_location.container,
            change.second_location.slot
        ),
        (0, 0, 254, 255)
    );
    let item = change.second_item.as_ref().unwrap();
    assert_eq!((item.template_id, item.id, item.grade), (123, 0x1122334455667788, 2));
    assert_eq!(item.detail_payload.as_deref(), Some(&[0x81, 0x82, 0x83, 0x84][..]));
    with_tail(sc::SCSlaveEquipmentChangedPacket, &body, ServerMsg::Slave(event.clone()));
    let mut world = crate::state::WorldState::default();
    let before = world.clone();
    world.apply(&Event::Slave(event));
    assert_eq!(world.slaves, before.slaves, "raw transaction notice does not invent an owned vehicle");
    assert_eq!(
        world.units.by_id, before.units.by_id,
        "no equipment/units fabricated from an ambiguous transaction"
    );
}

#[test]
fn mate_expiry_is_raw_location_notice_without_deleting_or_replacing_equipment() {
    for (container, slot) in [(252, 3), (2, 255), (254, 255)] {
        let mut w = Writer::new();
        w.u16(0x102).u8(container).u8(slot);
        let raw = w.finish();
        let event = MateEvent::EquipmentExpired {
            mate_tl_id: 0x102,
            container,
            slot,
        };
        with_tail(sc::SCMateEquipmentExpiredPacket, &raw, ServerMsg::Mate(event.clone()));
        let mut world = crate::state::WorldState::default();
        let unit = crate::state::tests::unit(33);
        world.apply(&Event::UnitSpawned(Box::new(unit.clone())));
        let before = world.units.by_id.clone();
        assert!(!world.mates.apply(&Event::Mate(event.clone())));
        world.apply(&Event::Mate(event));
        assert_eq!(world.units.by_id, before, "EXPIRED_ITEM is presentation, not item removal");
    }
}
