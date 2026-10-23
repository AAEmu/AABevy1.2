use super::*;
pub(crate) fn spawn() -> crate::PlayerSpawn {
    crate::PlayerSpawn {
        obj_id: 100,
        state: crate::model::CharacterState {
            character: crate::Character {
                id: 47,
                level: 30,
                ..Default::default()
            },
            ..Default::default()
        },
        unit: crate::Unit {
            bounty_money: None,
            pvp_honor: 0,
            pvp_kills: 0,
            obj_id: 100,
            name: "Expansion fixture".into(),
            kind: crate::model::UnitKind::Character { char_id: 47 },
            owner_name: String::new(),
            pos: Default::default(),
            rotation: [0.0; 3],
            scale: 1.0,
            level: 30,
            model_id: 0,
            equipment: Default::default(),
            model_params: Default::default(),
            hp: 100,
            mp: 100,
            max_hp: 100,
            max_mp: 100,
            dead: false,
            race_gender: 1,
            movement: None,
            faction_id: 0,
            expedition_id: 0,
            appellation: 0,
            flags: 0,
            gm_modes: [0; 4],
            skills: vec![],
            passive_buffs: vec![],
            ability_exp: vec![],
            active_abilities: vec![],
            buffs: vec![],
            anim_action: 0,
            attached_to: None,
            doodad_bond: None,
            house_posture: None,
            state_attachments: None,
        },
    }
}

use crate::{items::InvenChunk, model::ItemDetail};

fn item(id: u64, template_id: u32, count: i32) -> Item {
    Item {
        id,
        template_id,
        count,
        world_id: 7,
        grade: 6,
        ..Default::default()
    }
}
fn seed() -> NativeItemRecords {
    let mut records = NativeItemRecords::default();
    let spawn = spawn();
    records.observe(&Event::CharacterSelected(Box::new(spawn.state.clone())));
    records.observe(&Event::PlayerSpawn(Box::new(spawn)));
    records.observe(&Event::InventoryInit {
        bag_slots: 20,
        bank_slots: 10,
    });
    chunk(&mut records, 2, 0, vec![None; 20]);
    chunk(&mut records, 3, 0, vec![None; 10]);
    records
}

#[test]
fn music_sheet_fresh_empty_snapshot_add_is_exactly_known_without_relogin() {
    let mut records = seed();
    assert!(records.complete_bag());
    assert_eq!(records.item(2, 3), Some(&Item::default()));
    let (_, _, add, expected) = crate::music_sheet_tests::goldens()
        .find(|(name, ..)| *name == "add")
        .unwrap();
    let received = add.clone();
    let output = records.apply_row(&add).unwrap();
    assert!(output.admitted);
    assert_eq!(output.item, Some(expected.clone()));
    assert_eq!(records.item(2, 3), Some(&expected));
    assert_eq!(records.current(expected.id), Some(&expected));
    assert_eq!(expected.template_id, 28051);
    assert_eq!(expected.detail, ItemDetail::Other(8));
    assert_eq!(expected.detail_payload, Some(7u64.to_le_bytes().to_vec()));
    assert_eq!(
        add, received,
        "original receipt retains all128 bytes including padding"
    );
    assert!(records.complete_bag());

    // The next physical row must observe the newly added item; no post-batch
    // Inventory relocation or relogin snapshot supplies this certainty.
    let count_output = records.apply_row(&count(3, -1)).unwrap();
    let mut current = expected;
    current.count = 0;
    assert_eq!(count_output.item, Some(current.clone()));
    assert_eq!(records.current(current.id), Some(&current));
    let (_, _, update, _) = crate::music_sheet_tests::goldens()
        .find(|(name, ..)| *name == "update_high")
        .unwrap();
    let ItemTask::Update {
        detail,
        detail_payload,
        ..
    } = &update
    else {
        panic!()
    };
    current.detail = detail.clone();
    current.detail_payload = detail_payload.clone();
    let output = records.apply_row(&update).unwrap();
    assert!(output.admitted);
    assert_eq!(output.item, Some(current.clone()));
    assert_eq!(records.item(2, 3), Some(&current));
    assert_eq!(records.current(current.id), Some(&current));
    records.finish(&ItemTasks::default());
    assert_eq!(records.current(current.id), Some(&current));
}

#[test]
fn music_sheet_take_owner_replace_full_records_and_bad_rows_retire_only_the_declared_slot() {
    for (name, _, task, expected) in crate::music_sheet_tests::goldens()
        .filter(|(name, ..)| matches!(*name, "take" | "owner"))
    {
        let mut records = seed();
        records.bag[3] = Some(item(42, 22215, 2));
        let output = records.apply_row(&task).unwrap();
        assert!(output.admitted, "{name}");
        assert_eq!(output.item, Some(expected.clone()));
        assert_eq!(records.current(expected.id), Some(&expected));
        assert!(records.complete_bag());
    }
    for (_, _, task, _) in crate::music_sheet_tests::goldens()
        .filter(|(name, ..)| matches!(*name, "add" | "update"))
    {
        let mut records = seed();
        let (_, _, add, expected) = crate::music_sheet_tests::goldens()
            .find(|(name, ..)| *name == "add")
            .unwrap();
        records.apply_row(&add).unwrap();
        let mut bad = task;
        match &mut bad {
            ItemTask::Add { item, .. } => item.detail_payload.as_mut().unwrap()[0] ^= 1,
            ItemTask::Update {
                raw_detail_block, ..
            } => {
                raw_detail_block.as_mut().unwrap().pop();
            }
            _ => unreachable!(),
        }
        assert!(records.apply_row(&bad).is_none());
        assert!(records.item(2, 3).is_none());
        assert!(records.current(expected.id).is_none());
        assert!(!records.complete_bag());
        assert_eq!(records.item(2, 4), Some(&Item::default()));
        assert!(
            records.apply_row(&add).is_none(),
            "a later Add cannot invent unknown PRE-row certainty"
        );
        assert!(records.item(2, 3).is_none());
    }
}
fn chunk(
    records: &mut NativeItemRecords,
    container: u8,
    start_chunk: u8,
    items: Vec<Option<Item>>,
) {
    records.observe(&Event::InventoryChunk(InvenChunk {
        container,
        start_chunk,
        chunks: (items.len() / 10) as u8,
        items,
        truncated: false,
    }));
}
fn count(slot: u8, delta: i32) -> ItemTask {
    ItemTask::Count {
        container: 2,
        slot,
        item_id: 999,
        template_id: 999,
        delta,
    }
}

#[test]
fn current_result_lookup_keeps_first_equipment_hit_and_requires_known_earlier_bag_rows() {
    let mut records = seed();
    records.bag[8] = Some(item(42, 31432, 0));
    assert_eq!(records.current(42).unwrap().count, 0);
    records.equipment[0] = Some(item(42, 28299, -1));
    assert_eq!(records.current(42).unwrap().template_id, 28299);
    records.equipment[0] = Some(empty_equipment());
    records.bag[0] = None;
    assert!(
        records.current(42).is_none(),
        "unknown earlier row can contain the first duplicate ID"
    );
    assert_eq!(
        records.current(0).unwrap().template_id,
        0,
        "known equipment empty stops ID0 before unknown BAG"
    );
    records.bag[0] = Some(Item::default());
    records.bag_extent = Some(200);
    assert!(
        records.current(42).is_some(),
        "an actual early hit can precede an unsafe extent tail"
    );
    assert!(
        records.current(99).is_none(),
        "an overlong missing scan is not a known miss"
    );
}
fn remove(slot: u8, id: u64) -> ItemTask {
    ItemTask::Remove {
        container: 2,
        slot,
        item_id: id,
        template_id: 999,
    }
}

#[test]
fn receipt_sources_distinguish_unknown_block_empty_ctor_empty_and_body_template_setter() {
    let mut records = NativeItemRecords::default();
    assert!(records.item(2, 0).is_none());
    let mut spawn = spawn();
    spawn.unit.equipment.slots[19] = Some(item(98, 31432, 75));
    records.observe(&Event::PlayerSpawn(Box::new(spawn)));
    assert_eq!(records.item(1, 0), Some(&empty_equipment()));
    assert_eq!(
        records.item(1, 19),
        Some(&Item {
            template_id: 31432,
            flags: 1,
            count: 1,
            ..empty_equipment()
        })
    );
    assert_eq!(
        records.item(1, 20),
        Some(&Item {
            flags: 1,
            count: 1,
            ..empty_equipment()
        })
    );
    records.observe(&Event::InventoryInit {
        bag_slots: 20,
        bank_slots: 10,
    });
    assert!(records.item(2, 0).is_none());
    chunk(&mut records, 2, 0, vec![None; 10]);
    assert_eq!(records.item(2, 0), Some(&Item::default()));
    assert!(records.item(2, 10).is_none());
    assert!(!records.complete_bag());
    chunk(&mut records, 2, 1, vec![None; 10]);
    assert!(records.complete_bag());
    records.observe(&Event::EquipmentChanged {
        obj_id: 100,
        items: vec![(19, Some(item(12, 55, 3))), (20, None)],
    });
    assert_eq!(
        records.item(1, 19),
        Some(&item(12, 55, 3)),
        "local SC094 is a full item, including body slots"
    );
    assert_eq!(records.item(1, 20), Some(&empty_equipment()));
}

#[test]
fn consecutive_rows_expose_zero_negative_wrap_and_grade_before_next_row() {
    let mut records = seed();
    records.bag[0] = Some(item(42, 28299, 1));
    let first = records.apply_row(&count(0, -1)).unwrap();
    assert!(first.admitted);
    assert_eq!(first.item.as_ref().unwrap().count, 0);
    assert_eq!(
        records.item(2, 0).unwrap().count,
        0,
        "native zero stack survives for the next row"
    );
    let second = records.apply_row(&count(0, -1)).unwrap();
    assert_eq!(second.item.unwrap().count, -1);
    let grade = records
        .apply_row(&ItemTask::Grade {
            container: 2,
            slot: 0,
            item_id: 999,
            grade: 9,
        })
        .unwrap();
    assert!(grade.admitted);
    assert_eq!(
        grade.item.unwrap().grade,
        6,
        "grade output precedes its mutation"
    );
    assert_eq!(records.item(2, 0).unwrap().grade, 9);
    records.apply_row(&count(0, i32::MIN)).unwrap();
    assert_eq!(records.item(2, 0).unwrap().count, i32::MAX);
    let output = records.apply_row(&remove(0, 42)).unwrap();
    assert!(output.admitted);
    assert_eq!(output.item.unwrap().grade, 9);
    assert_eq!(records.item(2, 0), Some(&empty_equipment()));
    assert!(records.complete_bag());
}

#[test]
fn remove_uses_id_order_and_mutates_before_declared_location_admission() {
    let mut records = seed();
    records.equipment[5] = Some(item(42, 0, 3));
    records.bag[0] = Some(item(42, 28299, 5));
    let output = records.apply_row(&remove(0, 42)).unwrap();
    assert!(!output.admitted);
    assert_eq!(output.item.as_ref().unwrap().template_id, 0);
    assert_eq!(records.item(1, 5), Some(&empty_equipment()));
    assert_eq!(records.item(2, 0).unwrap().count, 5);
    let output = records.apply_row(&remove(0, 42)).unwrap();
    assert!(output.admitted);
    assert_eq!(output.item.unwrap().count, 5);
    assert_eq!(records.item(2, 0), Some(&empty_equipment()));
    records.bag[5] = Some(item(77, 5, 2));
    records.equipment[0] = None;
    assert!(
        records.apply_row(&remove(5, 77)).is_none(),
        "unknown earlier ID candidate blocks the later match"
    );
    assert!(
        records.item(2, 5).is_none(),
        "possibly affected full record is retired, never inferred from the post-batch reducer"
    );
    assert!(records.item(2, 6).is_some());
}

#[test]
fn move_resolves_both_pre_rows_ignoring_packet_ids_and_never_consumes_an_item_output() {
    let mut records = seed();
    let a = item(1, 28299, 3);
    let b = item(2, 31432, 1);
    records.bag[0] = Some(a.clone());
    records.bank[1] = Some(b.clone());
    let task = ItemTask::Move {
        from_container: 2,
        from_slot: 0,
        from_item_id: 999,
        to_container: 3,
        to_slot: 1,
        to_item_id: 888,
    };
    let output = records.apply_row(&task).unwrap();
    assert!(output.admitted && output.item.is_none());
    assert_eq!(records.item(2, 0), Some(&b));
    assert_eq!(records.item(3, 1), Some(&a));
    records.bank[1] = None;
    assert!(records.apply_row(&task).is_none());
    assert!(records.item(2, 0).is_none());
    assert_eq!(records.item(2, 1), Some(&Item::default()));
}

#[test]
fn raw_detail_admission_hydrates_real_add_update_and_retires_unqualified_rows_only() {
    let mut records = seed();
    let mut added = item(12, 28299, 3);
    added.detail_payload = Some(vec![]);
    let add = |slot, raw| ItemTask::Add {
        action: 5,
        container: 2,
        slot,
        item: added.clone(),
        raw_detail_block: raw,
    };
    let output = records.apply_row(&add(0, Some(vec![0; 128]))).unwrap();
    assert!(output.admitted);
    assert_eq!(output.item.as_ref(), Some(&added));
    assert_eq!(records.item(2, 0), Some(&added));
    assert!(records.apply_row(&add(1, None)).is_none());
    assert!(records.item(2, 1).is_none());
    assert_eq!(records.item(2, 2), Some(&Item::default()));
    let update = |raw| ItemTask::Update {
        container: 2,
        slot: 0,
        item_id: 999,
        detail: ItemDetail::None,
        detail_payload: Some(vec![]),
        raw_detail_block: raw,
    };
    let output = records.apply_row(&update(Some(vec![0; 128]))).unwrap();
    assert!(output.admitted);
    assert_eq!(output.item.as_ref(), records.item(2, 0));
    assert!(records.apply_row(&update(Some(vec![0; 127]))).is_none());
    assert!(records.item(2, 0).is_none());
    chunk(&mut records, 2, 0, vec![None; 10]);
    assert!(
        records.complete_bag(),
        "a later full contents block repairs specifically unknown records"
    );
}

#[test]
fn unsafe_receipts_and_actor_retirement_do_not_heal_with_same_owner_spawn() {
    let mut records = seed();
    records.observe(&Event::UnhandledGamePacket {
        opcode: 0x93,
        level: 3,
        len: 140,
    });
    assert!(
        !records.complete_bag(),
        "unhandled native detail mutation cannot preserve exact pre-row input"
    );
    let mut records = seed();
    records.observe(&Event::InventoryChunk(InvenChunk {
        container: 2,
        chunks: 1,
        start_chunk: 0,
        items: vec![None; 9],
        truncated: false,
    }));
    assert!(!records.complete_bag());
    chunk(&mut records, 2, 0, vec![None; 20]);
    records.observe(&Event::PlayerSpawn(Box::new(spawn())));
    assert!(!records.complete_bag());
    assert!(records.apply_row(&count(0, 1)).is_none());
    let mut records = seed();
    records.observe(&Event::UnitsRemoved(vec![100]));
    assert!(records.item(1, 0).is_none());
    records.observe(&Event::PlayerSpawn(Box::new(spawn())));
    assert!(records.item(1, 0).is_none());
    let mut spawn = spawn();
    spawn.state.character.id = 48;
    records.observe(&Event::CharacterSelected(Box::new(spawn.state.clone())));
    records.observe(&Event::PlayerSpawn(Box::new(spawn)));
    records.observe(&Event::InventoryInit {
        bag_slots: 20,
        bank_slots: 10,
    });
    chunk(&mut records, 2, 0, vec![None; 20]);
    assert!(records.complete_bag());
}

#[test]
fn header_boundaries_force_remove_and_local_equipment_order_preserve_unrelated_certainty() {
    let mut records = seed();
    records.observe(&Event::InventoryExpanded {
        container: 2,
        slots: 29,
    });
    assert_eq!(records.bag_extent(), Some(20));
    assert!(records.complete_bag());
    records.observe(&Event::InventoryExpanded {
        container: 2,
        slots: 140,
    });
    assert!(!records.complete_bag());
    assert!(
        records.item(2, 19).is_some(),
        "bounded physical reads remain known under an unsafe whole-BAG header"
    );
    assert!(records.item(2, 130).is_none());
    records.observe(&Event::InventoryExpanded {
        container: 2,
        slots: 20,
    });
    records.bag[4] = Some(item(77, 28299, 2));
    records.finish(&ItemTasks {
        force_remove: vec![77],
        ..Default::default()
    });
    assert!(records.item(2, 4).is_none());
    assert!(records.item(2, 5).is_some());
    records.observe(&Event::EquipmentChanged {
        obj_id: 200,
        items: vec![(0, Some(item(7, 10, 2)))],
    });
    assert_eq!(records.item(1, 0), Some(&empty_equipment()));
    records.observe(&Event::EquipmentChanged {
        obj_id: 100,
        items: vec![(0, Some(item(7, 10, 2))), (0, Some(item(8, 11, 3)))],
    });
    assert_eq!(records.item(1, 0), Some(&item(8, 11, 3)));
}

pub(super) fn detail_update(item_id: u64, slot: u8) -> ItemDetailUpdate {
    let mut raw_detail = vec![0; 56];
    raw_detail[0] = 1;
    raw_detail[5] = 77;
    raw_detail[24..28].copy_from_slice(&31003u32.to_le_bytes());
    ItemDetailUpdate {
        item_id,
        container: 2,
        slot,
        raw_detail,
    }
}

#[test]
fn typed_detail_updates_require_known_matching_bag_record_and_preserve_other_metadata() {
    use crate::model::EquipDetail;
    let mut records = seed();
    let original = item(42, 31432, 3);
    records.bag[0] = Some(original.clone());
    let update = detail_update(42, 0);
    assert!(records.apply_detail(&update));
    let mut expected = original;
    expected.detail = ItemDetail::Equip(EquipDetail {
        durability: 77,
        gem_ids: [31003, 0, 0, 0, 0, 0, 0],
        ..Default::default()
    });
    expected.detail_payload = Some(update.raw_detail[1..].to_vec());
    assert_eq!(records.item(2, 0), Some(&expected));
    let mut wrong = update.clone();
    wrong.item_id = 43;
    assert!(!records.apply_detail(&wrong));
    assert_eq!(records.item(2, 0), Some(&expected));
    for container in [1, 3, 4, 0x64] {
        wrong = update.clone();
        wrong.container = container;
        assert!(!records.apply_detail(&wrong));
        assert_eq!(records.item(2, 0), Some(&expected));
    }
    assert!(
        !records.apply_detail(&detail_update(0, 1)),
        "known empty native record is not a valid template"
    );
    assert_eq!(records.item(2, 1), Some(&Item::default()));
    records.bag[2] = Some(item(0, 31432, 1));
    assert!(
        records.apply_detail(&detail_update(0, 2)),
        "ID0 is allowed when the physical valid-template record matches"
    );
    records.bag[3] = None;
    assert!(!records.apply_detail(&detail_update(42, 3)));
    assert!(records.item(2, 3).is_none());
    let mut malformed = update;
    malformed.raw_detail.pop();
    assert!(!records.apply_detail(&malformed));
    assert!(records.item(2, 0).is_none());
    assert!(
        records.item(2, 1).is_some(),
        "uncertain detail update retires only the affected physical row"
    );
    chunk(&mut records, 2, 0, vec![None; 10]);
    assert!(records.complete_bag());
}

#[test]
fn whole_event_fold_matches_explicit_staged_rows_without_callbacks() {
    let mut staged = seed();
    staged.bag[0] = Some(item(42, 28299, 1));
    let mut whole = staged.clone();
    let changes = ItemTasks {
        reason: 40,
        tasks: vec![
            count(0, -1),
            ItemTask::Grade {
                container: 2,
                slot: 0,
                item_id: 999,
                grade: 9,
            },
            remove(0, 42),
        ],
        force_remove: vec![],
        truncated: false,
    };
    staged.observe(&Event::ItemsChanged(changes.clone()));
    let outputs: Vec<_> = changes
        .tasks
        .iter()
        .map(|row| staged.apply_row(row).unwrap())
        .collect();
    staged.finish(&changes);
    assert_eq!(outputs[0].item.as_ref().unwrap().count, 0);
    assert_eq!(outputs[1].item.as_ref().unwrap().grade, 6);
    assert_eq!(outputs[2].item.as_ref().unwrap().grade, 9);
    whole.apply(&Event::ItemsChanged(changes));
    assert_eq!(whole, staged);
    assert!(whole.complete_bag());
    let mut raw = vec![0; 56];
    raw[0] = 1;
    let detail = ItemDetailUpdate {
        item_id: 0,
        container: 2,
        slot: 0,
        raw_detail: raw,
    };
    staged.apply_detail(&detail);
    whole.apply(&Event::ItemDetailUpdated(detail));
    assert_eq!(whole, staged);
}

#[test]
fn owner_lifecycle_and_headers_do_not_promote_unknown_rows_or_reset_loss_by_same_actor() {
    let initial = seed();
    assert_eq!(initial.owner(), Some(47));
    for event in [
        Event::LeftWorld { target: 0 },
        Event::Disconnected {
            message: String::new(),
        },
        Event::Kicked {
            reason: 0,
            message: String::new(),
        },
        Event::PhaseChanged(crate::Phase::Authenticating),
        Event::PhaseChanged(crate::Phase::Connecting),
    ] {
        let mut records = initial.clone();
        records.apply(&event);
        assert_eq!(records.owner(), None);
        assert!(records.item(2, 0).is_none());
        assert_eq!(records.bag_extent(), None);
    }
    let mut records = initial;
    records.observe(&Event::GameDecodeFailed {
        message: String::new(),
    });
    records.observe(&Event::InventoryInit {
        bag_slots: 20,
        bank_slots: 10,
    });
    chunk(&mut records, 2, 0, vec![None; 20]);
    assert!(
        !records.complete_bag(),
        "global lost history is not repaired by rows or init alone"
    );
    let spawn = spawn();
    records.observe(&Event::CharacterSelected(Box::new(spawn.state.clone())));
    records.observe(&Event::PlayerSpawn(Box::new(spawn)));
    records.observe(&Event::InventoryInit {
        bag_slots: 20,
        bank_slots: 10,
    });
    chunk(&mut records, 2, 0, vec![None; 20]);
    assert!(records.complete_bag());
}

#[test]
fn non_equipment_unpack_keeps_full_record_known_without_recharge_catalog() {
    let mut records = seed();
    records.bag[0] = Some(item(42, 28299, 2));
    let row = ItemTask::Security {
        container: 2,
        slot: 0,
        item_id: 999,
        bits: 4,
        is_unsecure_excess: 0,
        is_unsecure_set: 0,
        is_unpack: 255,
        unsecure_time: -7,
        unpack_time: -9,
    };
    let output = records.apply_row(&row).unwrap();
    assert!(output.admitted);
    let current = records.item(2, 0).unwrap();
    assert_eq!(
        (current.flags, current.unsecure_time, current.unpack_time),
        (4, -7, -9)
    );
    assert_eq!(output.item.as_ref(), Some(current));
    assert!(records.complete_bag());
}

#[test]
fn equipment_id_receipt_changes_only_known_ids_in_native_first_hit_order() {
    let mut records = seed();
    records.equipment[0] = Some(item(1, 0, 7));
    records.equipment[1] = Some(item(2, 31432, 3));
    records.equipment[2] = None;
    let before = records.equipment.clone();
    let mut ids = [0; 28];
    ids[0] = 42;
    ids[1] = 42;
    ids[2] = 99;
    records.observe(&Event::EquipmentIdsChanged { obj_id: 999, ids });
    assert_eq!(records.equipment, before);
    records.observe(&Event::EquipmentIdsChanged { obj_id: 100, ids });
    let mut expected = before[0].clone().unwrap();
    expected.id = 42;
    assert_eq!(records.item(1, 0), Some(&expected));
    assert_eq!(
        records.current(42),
        Some(&expected),
        "template0 equipment remains first ID match"
    );
    assert!(
        records.item(1, 2).is_none(),
        "ID receipt never creates unknown metadata"
    );
    ids[0] = 0;
    records.observe(&Event::EquipmentIdsChanged { obj_id: 100, ids });
    expected.id = 0;
    assert_eq!(records.current(0), Some(&expected));
    records.observe(&Event::GameDecodeFailed {
        message: String::new(),
    });
    records.observe(&Event::EquipmentIdsChanged { obj_id: 100, ids });
    assert!(records.item(1, 0).is_none());
    assert!(records.current(0).is_none());
}

fn charged_item(template: u32) -> Item {
    let mut item = item(42, template, 1);
    item.detail = ItemDetail::Equip(crate::model::EquipDetail {
        charge_time: 102,
        ..Default::default()
    });
    let mut payload = vec![0; 55];
    payload[5] = 0xa5;
    payload[11..19].copy_from_slice(&102i64.to_le_bytes());
    item.detail_payload = Some(payload);
    item
}
fn unpack() -> ItemTask {
    ItemTask::Security {
        container: 2,
        slot: 0,
        item_id: 999,
        bits: 4,
        is_unsecure_excess: 0,
        is_unsecure_set: 0,
        is_unpack: 255,
        unsecure_time: -7,
        unpack_time: -9,
    }
}
#[test]
fn complete_recharge_catalog_distinguishes_unknown_zero_and_actual_static_sources() {
    use std::{collections::HashMap, sync::Arc};
    // Readonly compactDB items join matching impl1/2 class rows, not orphan child descriptors.
    let actual = [
        (26390, 4874),
        (26587, 4926),
        (29036, 6539),
        (30006, 5797),
        (30007, 5797),
        (30008, 5797),
        (30009, 5797),
        (31888, 7511),
        (31889, 7511),
        (33049, 7724),
        (34308, 8262),
        (34800, 11011),
    ];
    let catalog = Arc::new(HashMap::from(actual));
    for (template, _) in actual {
        let mut records = seed();
        records.bag[0] = Some(charged_item(template));
        records.set_recharge_catalog(catalog.clone());
        let output = records.apply_row(&unpack()).unwrap();
        assert!(output.admitted);
        let current = records.item(2, 0).unwrap();
        assert!(matches!(&current.detail,ItemDetail::Equip(e) if e.charge_time == -9));
        assert_eq!(
            &current.detail_payload.as_ref().unwrap()[11..19],
            &(-9i64).to_le_bytes()
        );
        assert_eq!(current.detail_payload.as_ref().unwrap()[5], 0xa5);
    }
    let mut records = seed();
    records.bag[0] = Some(charged_item(31432));
    records.set_recharge_catalog(catalog.clone());
    assert!(records.apply_row(&unpack()).unwrap().admitted);
    assert!(
        matches!(&records.item(2,0).unwrap().detail,ItemDetail::Equip(e) if e.charge_time == 102)
    );
    let mut unknown = seed();
    unknown.bag[0] = Some(charged_item(26390));
    assert!(unknown.apply_row(&unpack()).is_none());
    assert!(unknown.item(2, 0).is_none());
    unknown.set_recharge_catalog(catalog);
    assert!(
        unknown.item(2, 0).is_none(),
        "catalog arrival never repairs unknown full metadata"
    );
}
#[test]
fn recharge_catalog_arc_is_shared_and_survives_every_runtime_reset() {
    use std::{collections::HashMap, sync::Arc};
    let catalog = Arc::new(HashMap::from([(26390, 4874)]));
    let mut records = seed();
    records.set_recharge_catalog(catalog.clone());
    assert!(Arc::ptr_eq(
        records.clone().recharge_catalog.as_ref().unwrap(),
        &catalog
    ));
    let spawn = spawn();
    for event in [
        Event::LeftWorld { target: 0 },
        Event::Disconnected {
            message: String::new(),
        },
        Event::Kicked {
            reason: 0,
            message: String::new(),
        },
        Event::PhaseChanged(crate::Phase::Authenticating),
        Event::CharacterSelected(Box::new(spawn.state.clone())),
    ] {
        records.observe(&event);
        assert!(Arc::ptr_eq(
            records.recharge_catalog.as_ref().unwrap(),
            &catalog
        ));
    }
    records.observe(&Event::PlayerSpawn(Box::new(spawn)));
    records.observe(&Event::InventoryInit {
        bag_slots: 10,
        bank_slots: 0,
    });
    let mut items = vec![Some(charged_item(26390))];
    items.resize(10, None);
    chunk(&mut records, 2, 0, items);
    assert!(records.apply_row(&unpack()).unwrap().admitted);
}
