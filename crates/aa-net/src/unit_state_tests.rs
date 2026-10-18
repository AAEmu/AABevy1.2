//! Independent lane SCUnitState fixtures for previously unconsumed object tails.

use crate::game::ServerMsg;
use crate::model::{Unit, UnitKind, UnitStateAttachments};
use crate::opcodes::sc;
use crate::session::Event;
use crate::state::WorldState;
use crate::wire::{Pos, Reader, Writer};

fn body(kind: u8, first: u8, first_id: Option<u32>, bond: Option<u32>) -> (Vec<u8>, usize) {
    // Source: world/unit_state.rs::write_net_unit and unit_state_tests.rs.
    let mut w = Writer::new();
    w.bc(0x123456).string("Object").u8(kind);
    match kind {
        2 => {
            w.u32(100).u16(7).u32(200).u32(300);
        }
        4 => {
            w.u16(7).u32(200);
        }
        6 => {
            w.u64(0x0102030405060708).u32(200);
        }
        _ => unreachable!(),
    }
    w.string("owner").position(Pos::new(10.0, 20.0, 30.0)).f32(1.5).u8(5).u32(77);
    if kind == 2 {
        w.raw(&[0; 112]);
    } else {
        for id in 1001..=1007 {
            w.u32(id);
        }
    }
    w.u8(0); // model parameters None, present for every kind.
    let tail = w.as_bytes().len();
    w.bc(0).i32(10000).i32(5000).u8(first);
    if let Some(id) = first_id {
        w.bc(id);
    }
    if let Some(id) = bond {
        w.bc(id);
    } else {
        w.u8(255);
    }
    if kind == 4 {
        w.u8(8).bool(false).f32(0.25).f32(-0.5);
    } else {
        w.u8(0).bool(false);
    }
    w.u8(0).u8(0).i32(0).i8(0).i8(0).i8(64).u8(0);
    w.pisc(&[0, 0, 0, 0]).pisc(&[13, 0, 0, 0]).pisc(&[0, 0]).u16(0x2000).raw(&[0, 0, 0]);
    (w.finish(), tail)
}

#[test]
fn slave_transfer_shipyard_tails_consume_exactly_and_preserve_raw_attachment_identity() {
    for (kind, point, first_id) in [
        (2, 255, None),
        (2, 128, None),
        (2, 0, Some(0x456789)),
        (2, 127, Some(0x456789)),
        (4, 255, None),
        (4, 0, Some(0x987654)),
        (4, 254, Some(0x987654)),
        (6, 255, None),
    ] {
        for bond in [None, Some(0x123478), Some(0x010100)] {
            let (raw, _) = body(kind, point, first_id, bond);
            let mut r = Reader::new(&raw);
            let unit = Unit::read(&mut r).unwrap();
            assert!(r.is_empty(), "kind {kind}, point {point}, bond {bond:?}");
            assert_eq!(
                unit.state_attachments,
                Some(UnitStateAttachments {
                    first_point: point,
                    first_obj_id: first_id,
                    bonding_obj_id: bond
                })
            );
            assert_eq!((unit.hp, unit.mp, unit.faction_id, unit.flags), (100, 50, 13, 0x2000));
            assert_eq!(unit.rotation, [0.0, 0.0, 64.0 * (std::f32::consts::TAU / 127.0)]);
            assert!(unit.attached_to.is_none(), "snapshot raw fields are not UnitAttached notifications");
            match kind {
                2 => assert_eq!(
                    unit.kind,
                    UnitKind::Slave {
                        id: 100,
                        tl_id: 7,
                        template_id: 200,
                        summoner_id: 300
                    }
                ),
                4 => assert_eq!(unit.kind, UnitKind::Transfer { tl_id: 7, template_id: 200 }),
                6 => assert_eq!(
                    unit.kind,
                    UnitKind::Shipyard {
                        id: 0x0102030405060708,
                        template_id: 200
                    }
                ),
                _ => unreachable!(),
            }
            if kind != 2 {
                for (index, id) in (19..=25).zip(1001..=1007) {
                    assert_eq!(unit.equipment.get(index).unwrap().template_id, id);
                }
                assert!(unit.equipment.get(0).is_none());
            }
            for cut in 0..raw.len() {
                assert!(Unit::read(&mut Reader::new(&raw[..cut])).is_err(), "kind {kind}, cut {cut}");
            }
        }
    }
}

#[test]
fn object_unit_packets_keep_the_next_compressed_entries_aligned() {
    let mut inner = Writer::new();
    let mut expected = Vec::new();
    for (kind, point, id) in [(2, 1, Some(0x456789)), (4, 1, Some(0x987654)), (6, 255, None)] {
        let (raw, _) = body(kind, point, id, None);
        inner.u16(0).u16(sc::SCUnitStatePacket).raw(&raw);
        expected.push(ServerMsg::UnitState(Box::new(Unit::read(&mut Reader::new(&raw)).unwrap())));
    }
    inner.u16(0).u16(sc::SCOnOffSnowPacket).bool(true);
    expected.push(ServerMsg::Snow(true));
    let inner = inner.finish();
    let len = inner.len() as u16;
    let mut frame = Writer::new();
    frame.raw(&[0xdd, 4]).u16(4).u8(1).u16(len).u16(!len).raw(&inner);
    let decoded = ServerMsg::decode(&frame.finish()).unwrap();
    assert_eq!(decoded, ServerMsg::Batch(expected));
    let ServerMsg::Batch(messages) = decoded else { panic!() };
    let mut world = WorldState::default();
    for message in messages {
        if let ServerMsg::UnitState(unit) = message {
            world.apply(&Event::UnitSpawned(unit));
        }
    }
    assert!(world.units.by_id[&0x123456].state_attachments.is_some());
}

#[test]
fn malformed_object_tails_fail_instead_of_exposing_partial_units() {
    for kind in [2, 4, 6] {
        let (raw, tail) = body(kind, 255, None, None);
        // After BC/HP/MP, first point, second unbound marker: model posture.
        let mut bad = raw.clone();
        bad[tail + 13] = 9;
        assert!(Unit::read(&mut Reader::new(&bad)).is_err());
        if kind == 6 {
            let mut bad = raw.clone();
            bad[tail + 11] = 0;
            assert!(Unit::read(&mut Reader::new(&bad)).is_err());
        }
        let posture_len = if kind == 4 { 10 } else { 2 };
        let mut bad = raw;
        bad[tail + 13 + posture_len + 1] = 1; // impossible lane learnedSkillCount
        assert!(Unit::read(&mut Reader::new(&bad)).is_err());
    }
}
