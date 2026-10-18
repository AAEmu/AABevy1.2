use super::*;
use aa_net::{items::InvenChunk, model::ItemDetail};

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
fn seed() -> Records {
    let mut records = Records::default();
    let spawn = crate::hud::expansion::fixture::spawn();
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
fn chunk(records: &mut Records, container: u8, start_chunk: u8, items: Vec<Option<Item>>) {
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
    let mut records = Records::default();
    assert!(records.item(2, 0).is_none());
    let mut spawn = crate::hud::expansion::fixture::spawn();
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
    assert_eq!(records.item(2, 0).unwrap().count, 0, "native zero stack survives for the next row");
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
    assert_eq!(grade.item.unwrap().grade, 6, "grade output precedes its mutation");
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
    records.observe(&Event::InventoryChunk(InvenChunk {
        container: 2,
        chunks: 1,
        start_chunk: 0,
        items: vec![None; 9],
        truncated: false,
    }));
    assert!(!records.complete_bag());
    chunk(&mut records, 2, 0, vec![None; 20]);
    records.observe(&Event::PlayerSpawn(Box::new(crate::hud::expansion::fixture::spawn())));
    assert!(!records.complete_bag());
    assert!(records.apply_row(&count(0, 1)).is_none());
    let mut records = seed();
    records.observe(&Event::UnitsRemoved(vec![100]));
    assert!(records.item(1, 0).is_none());
    records.observe(&Event::PlayerSpawn(Box::new(crate::hud::expansion::fixture::spawn())));
    assert!(records.item(1, 0).is_none());
    let mut spawn = crate::hud::expansion::fixture::spawn();
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
    records.observe(&Event::InventoryExpanded { container: 2, slots: 29 });
    assert_eq!(records.bag_extent(), Some(20));
    assert!(records.complete_bag());
    records.observe(&Event::InventoryExpanded { container: 2, slots: 140 });
    assert!(!records.complete_bag());
    assert!(
        records.item(2, 19).is_some(),
        "bounded physical reads remain known under an unsafe whole-BAG header"
    );
    assert!(records.item(2, 130).is_none());
    records.observe(&Event::InventoryExpanded { container: 2, slots: 20 });
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
