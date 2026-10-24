use super::*;

include!("goldens.rs");
type ArithmeticGolden = (
    &'static str,
    f32,
    Option<i32>,
    f32,
    f32,
    f32,
    f32,
    i32,
    u64,
    u8,
);
include!("arithmetic_goldens.rs");

fn fixture() -> GameDb {
    let db = GameDb {
        conn: Connection::open_in_memory().unwrap(),
    };
    db.conn
        .execute_batch(include_str!("catalog_fixture.sql"))
        .unwrap();
    db
}

fn values(item: &ModelviewTrialItem) -> [i64; 9] {
    [
        item.kind.into(),
        item.gender.into(),
        item.bind_id.into(),
        item.slot_type.map_or(-1, i64::from),
        item.backpack_type.map_or(-1, i64::from),
        item.default_dyeing_item_id.into(),
        item.durability.into(),
        item.charge_count.into(),
        item.recharge_buff_id.into(),
    ]
}

#[test]
fn trial_metadata_matches_retained_native_catalog_and_constructor_goldens() {
    let db = fixture();
    for expected in GOLDENS {
        let item = db
            .modelview_trial_item(expected[0] as u32)
            .unwrap()
            .unwrap();
        assert_eq!(values(&item), expected[1..], "item {}", expected[0]);
    }
    for id in [0, 1607, 335, u32::MAX] {
        assert!(
            db.modelview_trial_item(id).unwrap().is_none(),
            "orphan subtype {id} is not an item descriptor"
        );
    }
}

#[test]
fn trial_metadata_real_catalog_matches_same_native_goldens() {
    if !GameDb::available() {
        return;
    }
    let db = GameDb::open().unwrap();
    for expected in GOLDENS {
        let item = db
            .modelview_trial_item(expected[0] as u32)
            .unwrap()
            .unwrap();
        assert_eq!(values(&item), expected[1..], "real item {}", expected[0]);
    }
}

#[test]
fn trial_metadata_matches_original_f32_x87_floor_store_and_byte_narrowing() {
    for &(name, ratio, coverage, holdable, wearable, grade, constant, multiplier, bits, byte) in
        ARITHMETIC
    {
        let base = durability_base(ratio, coverage, holdable, wearable, grade).unwrap();
        assert_eq!(base.to_bits(), bits, "original FSTP-double {name}");
        assert_eq!(
            durability_byte(base, constant, multiplier),
            byte,
            "original Item byte {name}"
        );
    }
}

#[test]
fn trial_metadata_preserves_descriptor_without_inventing_subtype_admission() {
    for (sql, item, kind) in [
        ("DELETE FROM item_weapons WHERE item_id=5569", 5569, 1),
        ("DELETE FROM holdables WHERE id=6", 5569, 1),
        ("DELETE FROM item_armors WHERE item_id=18180", 18180, 2),
        ("DELETE FROM item_backpacks WHERE item_id=23612", 23612, 22),
    ] {
        let db = fixture();
        db.conn.execute_batch(sql).unwrap();
        let got = db.modelview_trial_item(item).unwrap().unwrap();
        assert_eq!(got.kind, kind);
        assert_eq!(
            (got.slot_type, got.backpack_type, got.durability),
            (None, None, 0),
            "{sql}"
        );
    }
    let db = fixture();
    db.conn
        .execute_batch("UPDATE items SET impl_id=1 WHERE id=23612")
        .unwrap();
    let got = db.modelview_trial_item(23612).unwrap().unwrap();
    assert_eq!(
        (got.kind, got.slot_type, got.backpack_type),
        (1, None, None),
        "backpack subtype alone is not a weapon"
    );
    db.conn.execute_batch("UPDATE holdables SET slot_type_id=19 WHERE id=6; DELETE FROM item_configs; DELETE FROM item_grades").unwrap();
    let got = db.modelview_trial_item(5569).unwrap().unwrap();
    assert_eq!(
        (got.slot_type, got.durability),
        (Some(19), 0),
        "unmapped raw slot is retained, constructor not admitted"
    );
    let empty = GameDb {
        conn: Connection::open_in_memory().unwrap(),
    };
    assert!(
        empty.modelview_trial_item(0).unwrap().is_none(),
        "sentinel needs no catalog"
    );
}

#[test]
fn trial_metadata_unsupported_kind_and_gender_remain_raw() {
    let db = GameDb {
        conn: Connection::open_in_memory().unwrap(),
    };
    db.conn
        .execute_batch(
            "CREATE TABLE items (id INT, impl_id INT, char_gender_id INT, bind_id INT);
         INSERT INTO items VALUES (1,99,2,5), (2,NULL,NULL,NULL);",
        )
        .unwrap();
    assert_eq!(
        values(&db.modelview_trial_item(1).unwrap().unwrap()),
        [99, 2, 5, -1, -1, 0, 0, 0, 0]
    );
    assert_eq!(
        values(&db.modelview_trial_item(2).unwrap().unwrap()),
        [0, 0, 0, -1, -1, 0, 0, 0, 0]
    );
    assert!(db.modelview_trial_item(3).unwrap().is_none());
    let db = fixture();
    // This API has no source-gender argument and does not filter either descriptor.
    assert_eq!(db.modelview_trial_item(19555).unwrap().unwrap().gender, 2);
    assert_eq!(db.modelview_trial_item(28756).unwrap().unwrap().gender, 1);
}

#[test]
fn trial_metadata_grade_zero_ignores_fixed_grade_and_icon_policy() {
    let db = fixture();
    db.conn
        .execute_batch(
            "UPDATE items SET fixed_grade=3,gradable='f';
         UPDATE item_grades SET durability_value=1000 WHERE id<>0;
         ALTER TABLE items DROP COLUMN fixed_grade;
         ALTER TABLE items DROP COLUMN gradable;",
        )
        .unwrap();
    for expected in GOLDENS {
        assert_eq!(
            values(
                &db.modelview_trial_item(expected[0] as u32)
                    .unwrap()
                    .unwrap()
            ),
            expected[1..]
        );
    }
    db.conn
        .execute(
            "UPDATE item_grades SET durability_value=NULL WHERE id=0",
            [],
        )
        .unwrap();
    assert_eq!(
        db.modelview_trial_item(5569).unwrap().unwrap().durability,
        0,
        "native SQLite null scalar converts to zero"
    );
}

#[test]
fn trial_metadata_dye_is_raw_kind_two_item_and_charge_is_raw_metadata() {
    let db = fixture();
    assert_eq!(
        db.modelview_trial_item(24583)
            .unwrap()
            .unwrap()
            .default_dyeing_item_id,
        29621
    );
    db.conn
        .execute(
            "UPDATE dyeable_items SET default_dyeing_item_id=4294967295 WHERE item_id=24583",
            [],
        )
        .unwrap();
    assert_eq!(
        db.modelview_trial_item(24583)
            .unwrap()
            .unwrap()
            .default_dyeing_item_id,
        u32::MAX,
        "no RGB lookup or referenced-dye substitution"
    );
    db.conn
        .execute("DELETE FROM dyeable_items WHERE item_id=24583", [])
        .unwrap();
    assert_eq!(
        db.modelview_trial_item(24583)
            .unwrap()
            .unwrap()
            .default_dyeing_item_id,
        0
    );
    db.conn
        .execute_batch("INSERT INTO dyeable_items VALUES (5569,12345), (23612,12345)")
        .unwrap();
    assert_eq!(
        db.modelview_trial_item(5569)
            .unwrap()
            .unwrap()
            .default_dyeing_item_id,
        0
    );
    assert_eq!(
        db.modelview_trial_item(23612)
            .unwrap()
            .unwrap()
            .default_dyeing_item_id,
        0
    );
    for (table, item) in [("item_weapons", 26390), ("item_armors", 29036)] {
        for (count, expected) in [(17, 17), (65535, 65535), (65536, 0), (-1, 65535)] {
            db.conn
                .execute(
                    &format!("UPDATE {table} SET charge_count=?1 WHERE item_id=?2"),
                    params![count, item],
                )
                .unwrap();
            let got = db.modelview_trial_item(item).unwrap().unwrap();
            assert_eq!(got.charge_count, expected, "native loader's u16 store");
            assert_ne!(got.recharge_buff_id, 0);
        }
        db.conn
            .execute(
                &format!(
                    "UPDATE {table} SET charge_count=17,recharge_buff_id=NULL WHERE item_id=?1"
                ),
                [item],
            )
            .unwrap();
        let got = db.modelview_trial_item(item).unwrap().unwrap();
        assert_eq!(
            (got.charge_count, got.recharge_buff_id),
            (17, 0),
            "caller only writes charge/time when recharge is nonzero"
        );
    }
}

#[test]
fn trial_metadata_required_dependency_absence_is_an_error_even_for_zero_costume() {
    for (sql, item) in [
        ("DELETE FROM item_configs", 5569),
        ("DELETE FROM item_grades WHERE id=0", 5569),
        ("DELETE FROM wearable_kinds WHERE armor_type_id=2", 18180),
        ("DELETE FROM wearable_slots WHERE slot_type_id=7", 18180),
        ("DELETE FROM wearable_slots WHERE slot_type_id=31", 8000028),
        ("DELETE FROM item_grades WHERE id=0", 8000028),
    ] {
        let db = fixture();
        db.conn.execute_batch(sql).unwrap();
        assert!(
            matches!(db.modelview_trial_item(item), Err(Error::Db(_))),
            "{sql}"
        );
    }
    let db = fixture();
    db.conn
        .execute_batch("DELETE FROM item_configs; DELETE FROM item_grades")
        .unwrap();
    let got = db.modelview_trial_item(23612).unwrap().unwrap();
    assert_eq!(
        (got.slot_type, got.backpack_type, got.durability),
        (Some(30), Some(2), 0),
        "native no-durability-subtype fallback does not dereference grade/config"
    );
}

#[test]
fn trial_metadata_durability_keeps_original_subtype_fallback_precedence() {
    // Original constructor overlay controls retained in native-fallback-controls.json:
    // weapon5569 + actual armor18180 fields ->40; actual accessory18814 fields ->15.
    let db = fixture();
    db.conn.execute_batch(
        "INSERT INTO item_armors SELECT 5569,slot_type_id,type_id,durability_multiplier,charge_count,recharge_buff_id
             FROM item_armors WHERE item_id=18180;",
    ).unwrap();
    let got = db.modelview_trial_item(5569).unwrap().unwrap();
    assert_eq!((got.kind, got.slot_type, got.durability), (1, Some(17), 40));
    db.conn
        .execute_batch(
            "DELETE FROM item_armors WHERE item_id=5569;
         INSERT INTO item_accessories VALUES (5569,10,5,100);",
        )
        .unwrap();
    let got = db.modelview_trial_item(5569).unwrap().unwrap();
    assert_eq!((got.kind, got.slot_type, got.durability), (1, Some(17), 15));
    db.conn
        .execute("DELETE FROM wearable_kinds WHERE armor_type_id=5", [])
        .unwrap();
    assert!(
        matches!(db.modelview_trial_item(5569), Err(Error::Db(_))),
        "unresolved accessory dependency must not fall through to weapon"
    );
}

#[test]
fn trial_metadata_query_and_row_decode_failures_propagate() {
    for (table, item) in [
        ("items", 5569),
        ("item_weapons", 5569),
        ("holdables", 5569),
        ("item_armors", 18180),
        ("item_accessories", 5569),
        ("item_backpacks", 23612),
        ("dyeable_items", 24583),
        ("item_configs", 5569),
        ("item_grades", 5569),
        ("wearable_kinds", 18180),
        ("wearable_slots", 18180),
    ] {
        let db = fixture();
        db.conn
            .execute_batch(&format!("DROP TABLE {table}"))
            .unwrap();
        assert!(
            matches!(db.modelview_trial_item(item), Err(Error::Db(_))),
            "{table}"
        );
    }
    for (sql, item) in [
        ("UPDATE items SET impl_id='bad' WHERE id=5569", 5569),
        ("UPDATE items SET char_gender_id='bad' WHERE id=5569", 5569),
        ("UPDATE items SET bind_id=-1 WHERE id=5569", 5569),
        (
            "UPDATE item_weapons SET holdable_id='bad' WHERE item_id=5569",
            5569,
        ),
        (
            "UPDATE item_weapons SET charge_count='bad' WHERE item_id=5569",
            5569,
        ),
        (
            "UPDATE item_weapons SET recharge_buff_id='bad' WHERE item_id=5569",
            5569,
        ),
        (
            "UPDATE item_weapons SET durability_multiplier='bad' WHERE item_id=5569",
            5569,
        ),
        ("UPDATE holdables SET slot_type_id='bad' WHERE id=6", 5569),
        (
            "UPDATE holdables SET durability_ratio='bad' WHERE id=6",
            5569,
        ),
        (
            "UPDATE item_armors SET slot_type_id='bad' WHERE item_id=18180",
            18180,
        ),
        (
            "UPDATE item_armors SET type_id='bad' WHERE item_id=18180",
            18180,
        ),
        (
            "UPDATE item_armors SET durability_multiplier='bad' WHERE item_id=18180",
            18180,
        ),
        (
            "UPDATE item_armors SET charge_count='bad' WHERE item_id=18180",
            18180,
        ),
        (
            "UPDATE item_armors SET recharge_buff_id='bad' WHERE item_id=18180",
            18180,
        ),
        (
            "UPDATE wearable_kinds SET durability_ratio='bad' WHERE armor_type_id=2",
            18180,
        ),
        (
            "UPDATE wearable_slots SET coverage='bad' WHERE slot_type_id=7",
            18180,
        ),
        (
            "UPDATE dyeable_items SET default_dyeing_item_id='bad' WHERE item_id=24583",
            24583,
        ),
        (
            "UPDATE item_backpacks SET backpack_type_id='bad' WHERE item_id=23612",
            23612,
        ),
        (
            "UPDATE item_grades SET durability_value='bad' WHERE id=0",
            5569,
        ),
        ("UPDATE item_configs SET durability_const='bad'", 5569),
        (
            "UPDATE item_configs SET holdable_durability_const='bad'",
            5569,
        ),
        (
            "UPDATE item_configs SET wearable_durability_const='bad'",
            5569,
        ),
        ("UPDATE item_configs SET durability_const=1e999", 5569),
    ] {
        let db = fixture();
        db.conn.execute_batch(sql).unwrap();
        assert!(
            matches!(db.modelview_trial_item(item), Err(Error::Db(_))),
            "{sql}"
        );
    }
    let db = fixture();
    db.conn.execute_batch(
        "ALTER TABLE items RENAME TO item_data;
         CREATE VIEW items AS SELECT id,abs(-9223372036854775808) AS impl_id,char_gender_id,bind_id FROM item_data;",
    ).unwrap();
    assert!(
        matches!(db.modelview_trial_item(5569), Err(Error::Db(_))),
        "SQLite stepping failure is not a missing descriptor"
    );
}

#[test]
fn trial_metadata_file_errors_do_not_become_missing_items() {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "aa-anim-trial-{}-{stamp}.sqlite3",
        std::process::id()
    ));
    assert!(matches!(GameDb::open_path(&path), Err(Error::Db(_))));
    std::fs::write(&path, b"invalid SQLite input").unwrap();
    let result = GameDb::open_path(&path).and_then(|db| db.modelview_trial_item(5569));
    std::fs::remove_file(path).unwrap();
    assert!(matches!(result, Err(Error::Db(_))));
}

#[test]
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn trial_metadata_restores_callers_x87_control_word() {
    let mut original = 0_u16;
    // SAFETY: control-word pointers are live locals, and the original is restored before assertions.
    unsafe {
        std::arch::asm!("fnstcw word ptr [{p}]",p=in(reg)&mut original,options(nostack,preserves_flags));
    }
    let altered = (original & !0x0f00) | 0x0a00; // PC53 / round toward +infinity
    unsafe {
        std::arch::asm!("fldcw word ptr [{p}]",p=in(reg)&altered,options(nostack,preserves_flags));
    }
    let value = durability_base(1.0, None, 26.0, 100.0, 1.0);
    let mut after = 0_u16;
    unsafe {
        std::arch::asm!("fnstcw word ptr [{after}]","fldcw word ptr [{original}]",
        after=in(reg)&mut after,original=in(reg)&original,options(nostack,preserves_flags));
    }
    assert_eq!(after, altered);
    assert_eq!(value.unwrap().to_bits(), 26.0_f64.to_bits());
}
