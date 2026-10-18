use super::*;
use crate::{
    game::ClientMsg,
    mate::{MateCommand, MateEvent},
    mate_equipment::{MateEquipmentChange, MateItemLocation},
    session::Event,
};
fn item() -> (Item, Vec<u8>) {
    let mut payload = vec![0; 55];
    payload[4] = 25;
    payload[5] = 0xaa;
    payload[6] = 0xbb;
    let mut w = Writer::new();
    w.u32(77)
        .u64(0x0102030405060708)
        .u8(3)
        .u8(0x90)
        .i32(1)
        .u8(1)
        .raw(&payload)
        .i64(-2)
        .i32(19)
        .u32(42)
        .u8(7)
        .i64(101)
        .i64(202);
    let raw = w.finish();
    let i = Item::read(&mut Reader::new(&raw)).unwrap().unwrap();
    (i, raw)
}
fn entry(first: Option<Item>, second: Option<Item>) -> MateEquipmentEntry {
    MateEquipmentEntry {
        first: MateItemLocation {
            item: first,
            container: 2,
            slot: 6,
        },
        second: MateItemLocation {
            item: second,
            container: 0xfc,
            slot: 2,
        },
    }
}
#[test]
fn exact_one_two_entry_native_wire_preserves_full_item_reserved_bytes_and_locations() {
    let (i, raw) = item();
    for entries in [
        vec![entry(Some(i.clone()), None)],
        vec![entry(None, Some(i.clone()))],
        vec![entry(Some(i.clone()), Some(i.clone())), entry(None, Some(i.clone()))],
    ] {
        let request = MateEquipmentRequest::new(42, 0x102, 0xf1234567, true, entries.clone()).unwrap();
        let command = MateCommand::ChangeEquipment(Box::new(request));
        assert_eq!(command.opcode(), 0xa9);
        assert!(command.validate().is_ok());
        let mut expected = vec![42, 0, 0, 0, 2, 1, 0x67, 0x45, 0x23, 0xf1, 1, entries.len() as u8];
        for e in &entries {
            for x in [&e.first.item, &e.second.item] {
                if x.is_some() {
                    expected.extend(&raw);
                } else {
                    expected.extend([0; 4]);
                }
            }
            expected.extend([2, 6, 0xfc, 2]);
        }
        assert_eq!(ClientMsg::Mate(command).encode_body(), expected);
        // Same full-item body as SC126, with only its final success byte appended.
        let mut received = expected;
        received.push(1);
        let mut r = Reader::new(&received);
        let echo = MateEquipmentChange::read(&mut r).unwrap();
        assert!(r.is_empty());
        assert_eq!(echo.entries, entries);
    }
}
#[test]
fn invalid_echoes_containers_and_counts_are_refused_without_invented_padding() {
    let (i, _) = item();
    let e = entry(Some(i.clone()), None);
    for (owner, tl, entries) in [(0, 1, vec![e.clone()]), (1, 0, vec![e.clone()]), (1, 1, vec![]), (1, 1, vec![e.clone(); 3])] {
        assert!(MateEquipmentRequest::new(owner, tl, 0, false, entries).is_err());
    }
    for bad in [
        entry(None, None),
        {
            let mut e = e.clone();
            e.first.container = 3;
            e
        },
        {
            let mut e = e.clone();
            e.second.container = 0xfe;
            e
        },
        {
            let mut e = e.clone();
            e.second.slot = 28;
            e
        },
    ] {
        assert!(MateEquipmentRequest::new(1, 1, 0, false, vec![bad]).is_err());
    }
    for bad in [
        {
            let mut i = i.clone();
            i.detail_payload = None;
            i
        },
        {
            let mut i = i.clone();
            i.detail_payload.as_mut().unwrap().pop();
            i
        },
        {
            let mut i = i.clone();
            if let ItemDetail::Equip(d) = &mut i.detail {
                d.durability += 1;
            }
            i
        },
        {
            let mut i = i.clone();
            i.id = 0;
            i
        },
    ] {
        assert!(MateEquipmentRequest::new(1, 1, 0, false, vec![entry(Some(bad), None)]).is_err());
    }
}
#[test]
fn requests_rejections_and_raw_sc126_never_predict_item_swaps() {
    let (i, _) = item();
    let request = MateEquipmentRequest::new(42, 0x102, 0, false, vec![entry(Some(i), None)]).unwrap();
    let command = MateCommand::ChangeEquipment(Box::new(request));
    let mut state = crate::state::WorldState::default();
    for e in crate::equipment_notice_tests::flags_fixture() {
        state.apply(&e);
    }
    let before = state.units.by_id[&51].equipment.clone();
    let own = state.mates.own.clone();
    for e in [
        MateEvent::Requested(command.clone()),
        MateEvent::RequestRejected {
            command,
            message: "not sent".into(),
        },
    ] {
        state.apply(&Event::Mate(e));
        assert_eq!(state.units.by_id[&51].equipment, before);
        assert_eq!(state.mates.own, own);
    }
    state.apply(&Event::Mate(MateEvent::EquipmentChanged(MateEquipmentChange {
        character_id: 42,
        mate_tl_id: 0x102,
        passenger_id: 0,
        bts: false,
        entries: vec![entry(None, None)],
        success: true,
    })));
    assert_eq!(state.units.by_id[&51].equipment, before);
    state.apply(&Event::EquipmentChanged {
        obj_id: 51,
        items: vec![(1, None)],
    });
    assert!(state.units.by_id[&51].equipment.slots[1].is_none());
}
