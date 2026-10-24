use super::*;
use crate::{
    Event,
    game::{ClientMsg, ServerMsg},
    items::{InvenChunk, ItemTask},
    model::{EquipDetail, Item, ItemDetail},
    wire::{Reader, Writer},
};

pub(crate) const ACTOR: u32 = 0x010203;
pub(crate) const BASE_ID: u64 = 0x0102030405060708;
pub(crate) const IMAGE_ID: u64 = 0x1112131415161718;

pub(crate) fn captured(slot: u8, id: u64, template_id: u32, grade: u8) -> CapturedBagItem {
    CapturedBagItem {
        slot,
        item: Item {
            id,
            template_id,
            grade,
            count: 12,
            made_unit_id: 0x66778899,
            flags: 0x81,
            create_time: -7,
            unsecure_time: -9,
            unpack_time: 11,
            detail: ItemDetail::Equip(EquipDetail::default()),
            detail_payload: Some(vec![0; 55]),
            ..Default::default()
        },
    }
}

pub(crate) fn request(operation: ItemLookOperation) -> ItemLookRequest {
    let source = captured(
        3,
        BASE_ID,
        match operation {
            ItemLookOperation::Cleanup => 29704,
            ItemLookOperation::Skinize => 29880,
            ItemLookOperation::Convert => 0x11223344,
        },
        6,
    );
    let target = captured(4, IMAGE_ID, 0xaabbccdd, 7);
    match operation {
        ItemLookOperation::Convert => ItemLookRequest::convert(source, target),
        ItemLookOperation::Cleanup => ItemLookRequest::cleanup(23457, source, target),
        ItemLookOperation::Skinize => ItemLookRequest::skinize(23741, source, target),
    }
    .unwrap()
}

pub(crate) fn fixture(operation: ItemLookOperation) -> &'static [u8] {
    match operation {
        ItemLookOperation::Convert => {
            include_bytes!("../../tests/fixtures/look-conversion/convert.bin")
        }
        ItemLookOperation::Cleanup => {
            include_bytes!("../../tests/fixtures/look-conversion/cleanup.bin")
        }
        ItemLookOperation::Skinize => {
            include_bytes!("../../tests/fixtures/look-conversion/skinize.bin")
        }
    }
}

#[test]
fn executed_native_request_goldens_keep_field_widths_order_and_no_object() {
    for operation in [
        ItemLookOperation::Convert,
        ItemLookOperation::Cleanup,
        ItemLookOperation::Skinize,
    ] {
        let request = request(operation);
        let message = ClientMsg::ItemLook {
            self_obj_id: ACTOR,
            request: request.clone(),
        };
        let body = message.encode_body();
        assert_eq!(body, fixture(operation));
        assert_eq!(message.level(), 1);
        let opcode: u16 = if operation == ItemLookOperation::Convert {
            0x49
        } else {
            0x52
        };
        let mut expected = vec![0xdd, 1, 0, 0x7f];
        expected.extend_from_slice(&opcode.to_le_bytes());
        expected.extend_from_slice(fixture(operation));
        assert_eq!(message.encode(0x7f), expected);
        assert_eq!(
            message.name(),
            if opcode == 0x49 {
                "CSConvertItemLookPacket"
            } else {
                "CSStartSkillPacket"
            }
        );
        assert_eq!(request.operation(), operation);
        assert_eq!(request.captures().count(), 2);
        if operation == ItemLookOperation::Convert {
            assert_eq!(request.skill_id(), None);
            assert_eq!(
                (request.target().item.id, request.source().item.id),
                (BASE_ID, IMAGE_ID)
            );
        } else {
            // Independently decode the fields with the existing full native union codecs.
            let mut reader = Reader::new(&body);
            assert_eq!(Some(reader.u32().unwrap()), request.skill_id());
            let caster = crate::skill::SkillCasterData::read(&mut reader).unwrap();
            let target = crate::skill::SkillTargetData::read(&mut reader).unwrap();
            assert!(matches!(
                caster,
                crate::skill::SkillCasterData::Item {
                    obj_id: ACTOR,
                    item_id: BASE_ID,
                    type1: 6,
                    type2: 0x66778899,
                    ..
                }
            ));
            assert!(matches!(
                target,
                crate::skill::SkillTargetData::Item {
                    obj_id: ACTOR,
                    item_id: IMAGE_ID,
                    type1: 0xaabbccdd,
                    type2: 7
                }
            ));
            assert_eq!(reader.u8().unwrap(), 0);
            assert!(reader.is_empty());
            // No truncated prefix contains a complete CS052 union plus object flag.
            for end in 0..body.len() {
                let read = || -> crate::wire::Result<()> {
                    let mut r = Reader::new(&body[..end]);
                    r.u32()?;
                    crate::skill::SkillCasterData::read(&mut r)?;
                    crate::skill::SkillTargetData::read(&mut r)?;
                    r.u8()?;
                    Ok(())
                };
                assert!(read().is_err(), "{operation:?} prefix {end}");
            }
        }
    }
    let high = ItemLookRequest::convert(
        captured(0, u64::MAX, 1, 255),
        captured(1, 0x8877665544332211, 2, 0),
    )
    .unwrap();
    assert_eq!(
        ClientMsg::ItemLook {
            self_obj_id: ACTOR,
            request: high
        }
        .encode_body(),
        include_bytes!("../../tests/fixtures/look-conversion/convert_high.bin")
    );
    for body in [
        fixture(ItemLookOperation::Convert),
        include_bytes!("../../tests/fixtures/look-conversion/convert_high.bin"),
        include_bytes!("../../tests/fixtures/look-conversion/convert_zero.bin"),
    ] {
        for end in 0..body.len() {
            let mut r = Reader::new(&body[..end]);
            assert!(r.u64().and_then(|_| r.u64()).is_err());
        }
    }
}

#[test]
fn constructors_require_captured_identity_and_explicit_skill_without_guessing_catalog() {
    let source = captured(0, 11, 101, 6);
    let target = captured(1, 22, 202, 7);
    assert!(ItemLookRequest::cleanup(0, source.clone(), target.clone()).is_err());
    assert!(ItemLookRequest::skinize(0, source.clone(), target.clone()).is_err());
    // Transport takes the caller's resolved skill. It does not replace its catalog/planner gates.
    assert_eq!(
        ItemLookRequest::cleanup(17, source.clone(), target.clone())
            .unwrap()
            .skill_id(),
        Some(17)
    );
    for member in 0..2 {
        for field in 0..2 {
            let mut pair = [source.clone(), target.clone()];
            if field == 0 {
                pair[member].item.id = 0;
            } else {
                pair[member].item.template_id = 0;
            }
            let [a, b] = pair;
            assert!(ItemLookRequest::convert(a.clone(), b.clone()).is_err());
            assert!(ItemLookRequest::cleanup(23457, a.clone(), b.clone()).is_err());
            assert!(ItemLookRequest::skinize(23741, a, b).is_err());
        }
    }
}

pub(crate) fn records(request: &ItemLookRequest) -> NativeItemRecords {
    let mut records = NativeItemRecords::default();
    let mut spawn = crate::native_item_records::tests::spawn();
    spawn.obj_id = ACTOR;
    spawn.unit.obj_id = ACTOR;
    records.observe(&Event::CharacterSelected(Box::new(spawn.state.clone())));
    records.observe(&Event::PlayerSpawn(Box::new(spawn)));
    records.observe(&Event::InventoryInit {
        bag_slots: 130,
        bank_slots: 0,
    });
    let mut items = vec![None; 130];
    for captured in request.captures() {
        items[usize::from(captured.slot)] = Some(captured.item.clone());
    }
    // Actual SC04c has at most five chunks; keep the fixture inside that shape.
    for (start, rows) in items.chunks(50).enumerate() {
        records.observe(&Event::InventoryChunk(InvenChunk {
            container: 2,
            chunks: (rows.len() / 10) as u8,
            start_chunk: (start * 5) as u8,
            items: rows.to_vec(),
            truncated: false,
        }));
    }
    records
}

#[test]
fn full_native_capture_recheck_refuses_changed_metadata_unknown_and_out_of_extent() {
    for operation in [
        ItemLookOperation::Convert,
        ItemLookOperation::Cleanup,
        ItemLookOperation::Skinize,
    ] {
        let request = request(operation);
        let known = records(&request);
        assert!(request.recheck_native(&known).is_ok());
        assert!(
            request
                .recheck_native(&NativeItemRecords::default())
                .is_err()
        );
        for selected in 0..2 {
            for field in 0..13 {
                let mut source = request.source().clone();
                let mut target = request.target().clone();
                let capture = if selected == 0 {
                    &mut source
                } else {
                    &mut target
                };
                match field {
                    0 => capture.item.id ^= 1 << 60,
                    1 => capture.item.template_id ^= 1,
                    2 => capture.item.grade ^= 1,
                    3 => capture.item.flags ^= 8,
                    4 => capture.item.count -= 1,
                    5 => capture.item.made_unit_id ^= 1,
                    6 => capture.item.create_time += 1,
                    7 => capture.item.lifespan_mins += 1,
                    8 => capture.item.world_id ^= 1,
                    9 => capture.item.unsecure_time += 1,
                    10 => capture.item.unpack_time += 1,
                    11 => capture.item.detail_payload.as_mut().unwrap()[5] ^= 1,
                    _ => capture.slot = 130,
                }
                let changed =
                    ItemLookRequest::new(operation, request.skill_id(), source, target).unwrap();
                assert!(
                    changed.recheck_native(&known).is_err(),
                    "{operation:?}, capture{selected}, field{field}"
                );
            }
        }
    }
}

pub(crate) fn dd01(opcode: u16, body: &[u8]) -> Vec<u8> {
    let mut w = Writer::new();
    w.raw(&[0xdd, 1, 0, 0]).u16(opcode).raw(body);
    w.finish()
}

/// Lane-shaped received update with a nonzero raw reserved tail. This is not a
/// newly claimed native server golden; native request evidence is separate.
pub(crate) fn detail_receipt(slot: u8, packet_id: u64, image: u32) -> Vec<u8> {
    let mut raw = vec![0; 128];
    raw[0] = 1;
    raw[1..5].copy_from_slice(&image.to_le_bytes());
    raw[127] = 0xa5;
    let mut w = Writer::new();
    w.u8(111)
        .u8(1)
        .u8(9)
        .u8(2)
        .u8(slot)
        .u64(packet_id)
        .i16(128)
        .raw(&raw)
        .u8(0)
        .u32(0);
    dd01(0x90, &w.finish())
}

pub(crate) fn skin_receipt(slot: u8, packet_id: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u8(113)
        .u8(1)
        .u8(10)
        .u8(2)
        .u8(slot)
        .u64(packet_id)
        .u8(8)
        .u64(0)
        .u8(0)
        .u32(0);
    dd01(0x90, &w.finish())
}

#[test]
fn received_conversion_and_skinize_reuse_existing_projection_and_truncation_guards() {
    for operation in [
        ItemLookOperation::Convert,
        ItemLookOperation::Cleanup,
        ItemLookOperation::Skinize,
    ] {
        let request = request(operation);
        let mut records = records(&request);
        let target = request.target();
        let frame = if operation == ItemLookOperation::Skinize {
            skin_receipt(target.slot, 999)
        } else {
            detail_receipt(target.slot, 999, 8002)
        };
        for end in 0..frame.len() {
            assert!(
                ServerMsg::decode(&frame[..end]).is_err(),
                "{operation:?} truncated receipt{end}"
            );
        }
        let ServerMsg::ItemTasks(changes) = ServerMsg::decode(&frame).unwrap() else {
            panic!()
        };
        assert_eq!(
            changes.reason,
            if operation == ItemLookOperation::Skinize {
                113
            } else {
                111
            }
        );
        let row = changes.tasks.first().unwrap();
        let output = records.apply_row(row).unwrap();
        assert!(output.admitted);
        let mut expected = target.item.clone();
        if operation == ItemLookOperation::Skinize {
            expected.flags |= 8;
        } else {
            let ItemTask::Update {
                detail,
                detail_payload,
                raw_detail_block,
                ..
            } = row
            else {
                panic!()
            };
            assert_eq!(raw_detail_block.as_ref().unwrap()[127], 0xa5);
            expected.detail = detail.clone();
            expected.detail_payload = detail_payload.clone();
        }
        assert_eq!(output.item, Some(expected.clone()));
        assert_eq!(records.item(2, target.slot), Some(&expected));
        assert!(request.recheck_native(&records).is_err());
        assert_eq!(
            records.item(2, request.source().slot),
            Some(&request.source().item)
        );
        // Metadata/actual physical ID survive the deliberately wrong packet ID.
        assert_eq!(expected.id, target.item.id);
        let received = changes.clone();
        records.finish(&changes);
        assert_eq!(changes, received);
    }
}
