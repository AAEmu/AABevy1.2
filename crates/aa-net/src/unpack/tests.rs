use super::*;
use crate::{
    game::{ClientMsg, MAGIC, ServerMsg},
    items::{ItemTask, ItemTasks},
    model::Item,
    session::Event,
    wire::{Reader, Writer},
};
fn security(slot: u8, id: u64) -> ItemTask {
    ItemTask::Security {
        container: 2,
        slot,
        item_id: id,
        bits: 0x11,
        is_unsecure_excess: 2,
        is_unsecure_set: 255,
        is_unpack: 3,
        unsecure_time: -7,
        unpack_time: 1234567890123,
    }
}
fn receipt(task: ItemTask) -> Event {
    Event::ItemsChanged(ItemTasks {
        reason: 114,
        tasks: vec![task],
        ..Default::default()
    })
}
fn inventory() -> Inventory {
    let mut inv = Inventory::default();
    inv.bag_slots = 2;
    inv.bag = vec![
        Some(Item {
            id: 0x0102030405060708,
            template_id: 99,
            flags: 0x80,
            unsecure_time: 88,
            unpack_time: 99,
            ..Default::default()
        }),
        None,
    ];
    inv
}

fn security_body() -> Vec<u8> {
    let mut w = Writer::new();
    w.u8(114)
        .u8(1)
        .u8(11)
        .u8(2)
        .u8(0)
        .u64(0x0102030405060708)
        .u8(0x11)
        .u8(2)
        .u8(255)
        .u8(3)
        .i64(-7)
        .i64(1234567890123)
        .u8(0)
        .u32(0);
    w.finish()
}
#[test]
fn native_lane_unpack_request_is_ten_bytes_and_app_rechecks_exact_identity_location() {
    let request = UnpackItemRequest::new(2, 0, 0x0102030405060708).unwrap();
    assert_eq!(ClientMsg::UnpackItem(request).opcode(), 0x134);
    assert_eq!(ClientMsg::UnpackItem(request).encode_body(), vec![2, 0, 8, 7, 6, 5, 4, 3, 2, 1]);
    assert_eq!((request.container(), request.slot(), request.item_id()), (2, 0, 0x0102030405060708));
    assert!(UnpackItemRequest::new(2, 0, 0).is_err());
    let mut inv = inventory();
    assert!(request.recheck(&inv).is_ok());
    inv.bag.swap(0, 1);
    assert!(request.recheck(&inv).is_err());
    inv.bag[0] = Some(Item {
        id: 77,
        template_id: 99,
        ..Default::default()
    });
    assert!(request.recheck(&inv).is_err());
    assert!(UnpackItemRequest::new(252, 0, 77).unwrap().recheck(&inv).is_err());
    inv.bag_slots = 0;
    assert!(UnpackItemRequest::new(2, 0, 77).unwrap().recheck(&inv).is_err());
}
#[test]
fn action11_retains_raw_boolean_bytes_times_and_next_compressed_packet() {
    let body = security_body();
    let mut frame = Writer::new();
    frame.raw(&[MAGIC, 1, 0, 1]).u16(0x90).raw(&body);
    let expected = ItemTasks {
        reason: 114,
        tasks: vec![security(0, 0x0102030405060708)],
        ..Default::default()
    };
    assert_eq!(ServerMsg::decode(&frame.finish()).unwrap(), ServerMsg::ItemTasks(expected.clone()));
    for n in 0..body.len() {
        let mut frame = Writer::new();
        frame.raw(&[MAGIC, 1, 0, 1]).u16(0x90).raw(&body[..n]);
        assert!(ServerMsg::decode(&frame.finish()).is_err(), "cut{n}");
    }
    let mut raw = Writer::new();
    raw.u16(0).u16(0x90).raw(&body).u16(0).u16(crate::opcodes::sc::SCOnOffSnowPacket).bool(true);
    let raw = raw.finish();
    let n = raw.len() as u16;
    let mut frame = Writer::new();
    frame.raw(&[MAGIC, 4]).u16(2).u8(1).u16(n).u16(!n).raw(&raw);
    assert_eq!(
        ServerMsg::decode(&frame.finish()).unwrap(),
        ServerMsg::Batch(vec![ServerMsg::ItemTasks(expected), ServerMsg::Snow(true)])
    );
}
#[test]
fn set_bits_ors_but_security_replaces_only_existing_exact_item_and_updates_both_times() {
    let mut inv = inventory();
    let id = inv.bag[0].as_ref().unwrap().id;
    let mut raw = Writer::new();
    raw.u8(114).u8(1).u8(10).u8(2).u8(0).u64(id).u8(1).u64(0).u8(0);
    let bits = ItemTasks::read(&mut Reader::new(&raw.finish())).unwrap();
    assert!(inv.apply(&Event::ItemsChanged(bits)));
    assert_eq!(inv.bag[0].as_ref().unwrap().flags, 0x81);
    inv.mark_item_slot_pending(2, 0);
    assert!(inv.apply(&receipt(security(0, id))));
    let item = inv.bag[0].as_ref().unwrap();
    assert_eq!((item.flags, item.unsecure_time, item.unpack_time), (0x11, -7, 1234567890123));
    assert!(!inv.item_slot_pending(2, 0));
    let before = inv.clone();
    assert!(!inv.apply(&receipt(security(1, id))));
    assert_eq!((&inv.bag, &inv.bank, &inv.equipment), (&before.bag, &before.bank, &before.equipment));
    assert!(!inv.apply(&receipt(security(250, id))));
    assert_eq!((&inv.bag, &inv.bank, &inv.equipment), (&before.bag, &before.bank, &before.equipment));
    assert!(!inv.apply(&receipt(security(0, id + 1))));
    assert_eq!((&inv.bag, &inv.bank, &inv.equipment), (&before.bag, &before.bank, &before.equipment));
    let request = UnpackItemRequest::new(2, 0, id).unwrap();
    assert!(!inv.apply(&Event::UnpackItemRequested(request)));
    assert_eq!((&inv.bag, &inv.bank, &inv.equipment), (&before.bag, &before.bank, &before.equipment));
    assert!(!inv.apply(&Event::UnpackItemRequestRejected {
        request,
        message: String::new()
    }));
    assert_eq!((&inv.bag, &inv.bank, &inv.equipment), (&before.bag, &before.bank, &before.equipment));
    // Native assigns both times regardless of the three raw bool fields.
    let task = ItemTask::Security {
        container: 2,
        slot: 0,
        item_id: id,
        bits: 0,
        is_unsecure_excess: 0,
        is_unsecure_set: 0,
        is_unpack: 0,
        unsecure_time: 0,
        unpack_time: -1,
    };
    assert!(inv.apply(&receipt(task)));
    let item = inv.bag[0].as_ref().unwrap();
    assert_eq!((item.flags, item.unsecure_time, item.unpack_time), (0, 0, -1));
}
#[test]
fn world_state_security_update_is_authoritative_and_unknown_slots_do_not_grow() {
    let mut world = crate::state::WorldState::default();
    world.inventory = inventory();
    let id = world.inventory.bag[0].as_ref().unwrap().id;
    world.apply(&receipt(security(0, id)));
    assert_eq!(world.inventory.bag[0].as_ref().unwrap().unpack_time, 1234567890123);
    let before = world.inventory.clone();
    world.apply(&receipt(security(250, id)));
    assert_eq!(
        (&world.inventory.bag, &world.inventory.bank, &world.inventory.equipment),
        (&before.bag, &before.bank, &before.equipment)
    );
}
