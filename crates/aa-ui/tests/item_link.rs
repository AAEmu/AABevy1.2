//! Byte comparisons against actual original x86 constructor/reader/encoder outputs.
//! See fixtures/item_record_native.py for the reproducible offline fixture generator.

use aa_ui::item_link::{self, EquipDetail, ItemDetail, ItemRecord};

fn common(detail: ItemDetail) -> ItemRecord {
    ItemRecord {
        id: 0x8877665544332211,
        item_type: 24488,
        grade: 4,
        flags: 0x18,
        count: 17,
        made_unit_id: 12345,
        detail,
        create_time: 1710000000,
        lifespan_mins: 360,
        world_id: 2,
        unsecure_time: 1710000300,
        unpack_time: 1710000600,
    }
}

fn equip() -> ItemDetail {
    ItemDetail::Equip(EquipDetail {
        image_item_template_id: 14698,
        durability: 83,
        reserved: [0; 2],
        rune_id: 567,
        charge_time: 1709999999,
        dye_item_id: 678,
        gem_ids: [101, 202, 303, 404, 505, 606, 707],
        temper_physical: 1234,
        temper_magical: 4321,
    })
}

#[test]
fn all_supported_fields_match_actual_native_reader_and_encoder() {
    let fixtures: [(ItemDetail, &[u8; 192], &str); 3] = [
        (
            ItemDetail::None,
            include_bytes!("fixtures/item-record-none.bin"),
            include_str!("fixtures/item-record-none.txt"),
        ),
        (
            equip(),
            include_bytes!("fixtures/item-record-equip.bin"),
            include_str!("fixtures/item-record-equip.txt"),
        ),
        (
            ItemDetail::Mate {
                exp: 54321,
                dead: 1,
                level: 42,
            },
            include_bytes!("fixtures/item-record-mate.bin"),
            include_str!("fixtures/item-record-mate.txt"),
        ),
    ];
    for (detail, native, encoded) in fixtures {
        let expected = common(detail);
        assert_eq!(&item_link::pack(&expected), native);
        assert_eq!(item_link::unpack(native), expected);
        assert_eq!(item_link::encode_payload(native), encoded);
        assert_eq!(item_link::decode_payload(encoded), Some(*native));
        let link = item_link::format(native);
        let transported = item_link::decode_payload(
            link.strip_prefix("|i24488,4,")
                .unwrap()
                .strip_suffix(';')
                .unwrap(),
        )
        .unwrap();
        let mut normalized = expected;
        normalized.id = 0;
        normalized.count = 1;
        assert_eq!(item_link::unpack(&transported), normalized);
    }
}

#[test]
fn equipment_reserved_bytes_survive_and_padding_is_only_canonicalized_by_field_packing() {
    let mut native = *include_bytes!("fixtures/item-record-equip.bin");
    native[30..32].copy_from_slice(&[0x43, 0x21]);
    native[14..16].fill(0xcd);
    native[169..176].fill(0xcd);
    assert_eq!(
        item_link::decode_payload(&item_link::encode_payload(&native)),
        Some(native)
    );
    let parsed = item_link::unpack(&native);
    let ItemDetail::Equip(detail) = &parsed.detail else {
        panic!()
    };
    assert_eq!(detail.reserved, [0x43, 0x21]);
    let canonical = item_link::pack(&parsed);
    native[14..16].fill(0);
    native[169..176].fill(0);
    assert_eq!(canonical, native);
}

#[test]
fn unsupported_native_detail_keeps_its_complete_union_without_inventing_fields() {
    let payload = std::array::from_fn(|i| i as u8 + 1);
    let original = common(ItemDetail::Opaque { kind: 7, payload });
    let bytes = item_link::pack(&original);
    assert_eq!(bytes[24], 7);
    assert_eq!(&bytes[25..152], &payload);
    assert_eq!(item_link::unpack(&bytes), original);
}

#[test]
fn wire_details_match_native_fixed_sizes_and_preserve_uninterpreted_bytes() {
    let native = include_bytes!("fixtures/item-record-opaque.bin");
    let detail = ItemDetail::from_wire(4, &[1, 2, 3, 4, 5, 6, 7, 8, 9]).unwrap();
    assert_eq!(&item_link::pack(&common(detail.clone())), native);
    assert_eq!(item_link::unpack(native).detail, detail);
    assert_eq!(
        item_link::encode_payload(native),
        include_str!("fixtures/item-record-opaque.txt")
    );
    for (kind, size) in [
        (0, 0),
        (1, 55),
        (2, 29),
        (3, 6),
        (4, 9),
        (5, 24),
        (6, 16),
        (7, 16),
        (8, 8),
        (9, 4),
    ] {
        assert!(ItemDetail::from_wire(kind, &vec![0; size]).is_some());
        assert!(ItemDetail::from_wire(kind, &vec![0; size + 1]).is_none());
        if size > 0 {
            assert!(ItemDetail::from_wire(kind, &vec![0; size - 1]).is_none());
        }
    }
    assert!(ItemDetail::from_wire(10, &[]).is_none());
    // A variable-length 33-byte slave detail cannot masquerade as native29.
    assert!(ItemDetail::from_wire(2, &[0; 33]).is_none());
}
