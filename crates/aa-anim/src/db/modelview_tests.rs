use super::*;

fn fixture() -> GameDb {
    let db = GameDb { conn: Connection::open_in_memory().unwrap() };
    db.conn.execute_batch(
        "CREATE TABLE characters (
             id INT PRIMARY KEY, char_gender_id INT, char_race_id INT, creatable NUM,
             default_fx_voice_sound_pack_id INT, default_resurrection_district_id INT,
             default_return_district_id INT, default_system_voice_sound_pack_id INT,
             faction_id INT, model_id INT, preview_body_pack_id INT, preview_cloth_pack_id INT,
             starting_zone_id INT);
         CREATE TABLE equip_pack_cloths (id INT PRIMARY KEY, cosplay_id INT, shoes_id INT);
         CREATE TABLE item_armors (id INT PRIMARY KEY, item_id INT, asset_id INT);
         CREATE TABLE armor_assets (id INT PRIMARY KEY, default_asset_id INT);
         CREATE TABLE item_armor_assets (id INT PRIMARY KEY, armor_asset_id INT, asset_id INT);
         CREATE TABLE item_assets (id INT PRIMARY KEY, model_id INT, heel_offset_height REAL);
         INSERT INTO characters (id, char_gender_id, char_race_id, creatable, model_id, preview_cloth_pack_id)
             VALUES (90, 2, 1, 'f', 777, 500), (10, 2, 1, 't', 11, 183);
         INSERT INTO equip_pack_cloths VALUES (500, 100, 200), (183, 300, 0);
         INSERT INTO item_armors VALUES (1, 100, 1000), (2, 200, 2000), (3, 300, 3000);
         INSERT INTO armor_assets VALUES (1000, 0), (2000, 0), (3000, 0);
         INSERT INTO item_armor_assets VALUES (90, 1000, 900), (10, 1000, 100),
             (30, 2000, 200), (40, 3000, 300);
         INSERT INTO item_assets VALUES (900, 777, 0.0), (100, 777, 0.4),
             (200, 777, 0.09), (300, 11, 0.08);
         CREATE INDEX tempting_character_filter ON characters (char_race_id, char_gender_id, id);
         CREATE INDEX tempting_link_filter ON item_armor_assets (armor_asset_id, asset_id);",
    ).unwrap();
    db
}

fn heel(db: &GameDb) -> f32 {
    db.modelview_reference_heel(Race::Nuian, Gender::Female).unwrap()
}

#[test]
fn modelview_reference_first_original_row_including_noncreatable_and_zero_asset() {
    let db = fixture();
    // First SQL row is id90, not id10. Its source model777/pack500 must be used,
    // independent of the playable Nuian-female model11 and creatable flag.
    assert_eq!(heel(&db).to_bits(), 0);
    db.conn.execute("UPDATE item_assets SET heel_offset_height = 0.125 WHERE id = 900", []).unwrap();
    assert_eq!(heel(&db), 0.125);
    db.conn.execute_batch("PRAGMA reverse_unordered_selects=ON").unwrap();
    assert_eq!(heel(&db), 0.08_f32, "follow original SQL return order, not rowid or id sorting");
}

#[test]
fn modelview_reference_default_asset_precedes_model_links_and_missing_does_not_scan_links() {
    let db = fixture();
    db.conn.execute_batch(
        "INSERT INTO item_assets VALUES (999, 12345, 0.25);
         UPDATE armor_assets SET default_asset_id = 999 WHERE id = 1000;",
    ).unwrap();
    assert_eq!(heel(&db), 0.25, "default ignores the requested model");
    db.conn.execute("UPDATE item_assets SET heel_offset_height = 0 WHERE id = 999", []).unwrap();
    assert_eq!(heel(&db).to_bits(), 0, "present zero default wins over shoes");
    db.conn.execute("DELETE FROM item_assets WHERE id = 999", []).unwrap();
    assert_eq!(heel(&db), 0.09_f32, "missing nonzero default falls to feet, not model links");
    // A valid default must not depend on link-table availability.
    db.conn.execute_batch(
        "UPDATE armor_assets SET default_asset_id = 300 WHERE id = 1000;
         DROP TABLE item_armor_assets;",
    ).unwrap();
    assert_eq!(heel(&db), 0.08_f32);
}

#[test]
fn modelview_reference_model_links_follow_native_loader_append_order() {
    let db = fixture();
    // Missing/wrong-model links are skipped; two matching assets with different
    // heels retain link-loader order, not either ID ordering.
    db.conn.execute_batch(
        "UPDATE item_armor_assets SET asset_id = 999 WHERE id = 90;
         UPDATE item_assets SET model_id = 11 WHERE id = 100;
         INSERT INTO item_armor_assets VALUES (80, 1000, 800), (20, 1000, 700);
         INSERT INTO item_assets VALUES (800, 777, 0.3), (700, 777, 0.6);",
    ).unwrap();
    assert_eq!(heel(&db), 0.3_f32);
    db.conn.execute_batch("PRAGMA reverse_unordered_selects=ON; DELETE FROM characters WHERE id=10").unwrap();
    assert_eq!(heel(&db), 0.6_f32, "follow changed original link-query return order");
}

#[test]
fn modelview_reference_missing_rows_packs_items_groups_links_assets_and_null_scalars() {
    for sql in [
        "DELETE FROM characters",
        "DELETE FROM equip_pack_cloths",
        "DELETE FROM equip_pack_cloths WHERE id=500",
        "DELETE FROM item_armors",
        "DELETE FROM armor_assets",
        "DELETE FROM item_armor_assets",
        "DELETE FROM item_assets",
        "UPDATE equip_pack_cloths SET cosplay_id=0, shoes_id=0",
        "UPDATE equip_pack_cloths SET cosplay_id=NULL, shoes_id=NULL",
    ] {
        let db = fixture();
        db.conn.execute_batch(sql).unwrap();
        assert_eq!(heel(&db).to_bits(), 0, "{sql}");
    }
    for sql in [
        "DELETE FROM item_armors WHERE item_id=100",
        "DELETE FROM armor_assets WHERE id=1000",
        "DELETE FROM item_armor_assets WHERE armor_asset_id=1000",
        "DELETE FROM item_assets WHERE id IN (900,100)",
        "UPDATE item_assets SET model_id=12345 WHERE id IN (900,100)",
    ] {
        let db = fixture();
        db.conn.execute_batch(sql).unwrap();
        assert_eq!(heel(&db), 0.09_f32, "{sql}");
    }
    let db = fixture();
    db.conn.execute("UPDATE item_assets SET heel_offset_height=NULL WHERE id=900", []).unwrap();
    assert_eq!(heel(&db).to_bits(), 0, "native SQLite null scalar converts to zero, still present");
    assert_eq!(db.modelview_reference_heel(Race::Elf, Gender::Male).unwrap().to_bits(), 0);
}

#[test]
fn modelview_reference_query_and_decode_failures_propagate_without_fallback() {
    for table in ["characters", "equip_pack_cloths", "item_armors", "armor_assets", "item_armor_assets", "item_assets"] {
        let db = fixture();
        db.conn.execute_batch(&format!("DROP TABLE {table}")).unwrap();
        assert!(matches!(db.modelview_reference_heel(Race::Nuian, Gender::Female), Err(Error::Db(_))), "{table}");
    }
    for sql in [
        "UPDATE characters SET char_race_id='bad' WHERE id=90",
        "UPDATE characters SET char_gender_id='bad' WHERE id=90",
        "UPDATE characters SET model_id='bad' WHERE id=90",
        "UPDATE characters SET preview_cloth_pack_id='bad' WHERE id=90",
        "UPDATE equip_pack_cloths SET cosplay_id='bad' WHERE id=500",
        "UPDATE equip_pack_cloths SET shoes_id='bad' WHERE id=500",
        "UPDATE item_armors SET asset_id='bad' WHERE item_id=100",
        "UPDATE armor_assets SET default_asset_id='bad' WHERE id=1000",
        "UPDATE item_armor_assets SET armor_asset_id='bad' WHERE id=90",
        "UPDATE item_armor_assets SET asset_id='bad' WHERE id=90",
        "UPDATE item_assets SET model_id='bad' WHERE id=900",
        "UPDATE item_assets SET heel_offset_height='bad' WHERE id=900",
        "UPDATE armor_assets SET default_asset_id=900 WHERE id=1000;
         UPDATE item_assets SET heel_offset_height='bad' WHERE id=900",
        "UPDATE characters SET model_id=-1 WHERE id=90",
    ] {
        let db = fixture();
        db.conn.execute_batch(sql).unwrap();
        assert!(matches!(db.modelview_reference_heel(Race::Nuian, Gender::Female), Err(Error::Db(_))), "{sql}");
    }
    // Preparing succeeds; stepping the original query raises an SQLite execution error.
    let db = fixture();
    db.conn.execute_batch(
        "ALTER TABLE characters RENAME TO characters_data;
         CREATE VIEW characters AS SELECT id, abs(-9223372036854775808) AS char_gender_id,
             char_race_id, creatable, default_fx_voice_sound_pack_id, default_resurrection_district_id,
             default_return_district_id, default_system_voice_sound_pack_id, faction_id, model_id,
             preview_body_pack_id, preview_cloth_pack_id, starting_zone_id FROM characters_data;",
    ).unwrap();
    assert!(matches!(db.modelview_reference_heel(Race::Nuian, Gender::Female), Err(Error::Db(_))));
}

#[test]
fn modelview_reference_io_failure_is_not_missing_reference() {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("aa-anim-modelview-{}-{stamp}.sqlite3", std::process::id()));
    assert!(matches!(GameDb::open_path(&path), Err(Error::Db(_))));
    std::fs::write(&path, b"deliberately invalid SQLite input").unwrap();
    let result = GameDb::open_path(&path).and_then(|db| db.modelview_reference_heel(Race::Nuian, Gender::Female));
    std::fs::remove_file(path).unwrap();
    assert!(matches!(result, Err(Error::Db(_))));
}

#[test]
fn modelview_reference_real_eight_catalog_contexts() {
    if !GameDb::available() { return; }
    let db = GameDb::open().unwrap();
    for race in Race::ALL {
        for gender in Gender::ALL {
            let expected = if (race, gender) == (Race::Nuian, Gender::Female) { 0.08_f32 } else { 0.0 };
            assert_eq!(db.modelview_reference_heel(race, gender).unwrap().to_bits(), expected.to_bits(), "{race:?} {gender:?}");
        }
    }
}

#[test]
fn modelview_reference_real_model11_item_controls_match_native_proof() {
    if !GameDb::available() { return; }
    let source = GameDb::open().unwrap();
    let db = fixture();
    db.conn.execute_batch(
        "DELETE FROM characters WHERE id=10; UPDATE characters SET model_id=11 WHERE id=90;
         DELETE FROM item_armors; DELETE FROM armor_assets; DELETE FROM item_armor_assets; DELETE FROM item_assets;",
    ).unwrap();
    for item in [18180, 8000028, 28439] {
        let group: u32 = source.conn.query_row("SELECT asset_id FROM item_armors WHERE item_id=?1", [item], |r| r.get(0)).unwrap();
        let default: u32 = source.conn.query_row("SELECT default_asset_id FROM armor_assets WHERE id=?1", [group], |r| r.get(0)).unwrap();
        db.conn.execute("INSERT INTO item_armors (item_id,asset_id) VALUES (?1,?2)", params![item, group]).unwrap();
        db.conn.execute("INSERT INTO armor_assets VALUES (?1,?2)", params![group, default]).unwrap();
        let mut links = source.conn.prepare("SELECT id,armor_asset_id,asset_id FROM item_armor_assets").unwrap();
        for row in links.query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, u32>(1)?, r.get::<_, u32>(2)?))).unwrap() {
            let (id, link_group, asset) = row.unwrap();
            if link_group != group { continue; }
            let (model, height): (u32, f64) = source.conn.query_row(
                "SELECT model_id,heel_offset_height FROM item_assets WHERE id=?1", [asset], |r| Ok((r.get(0)?, r.get(1)?)),
            ).unwrap();
            db.conn.execute("INSERT INTO item_armor_assets VALUES (?1,?2,?3)", params![id, group, asset]).unwrap();
            db.conn.execute("INSERT INTO item_assets VALUES (?1,?2,?3)", params![asset, model, height]).unwrap();
        }
    }
    for (costume, feet, expected) in [(0, 0, 0.0_f32), (0, 18180, 0.09), (8000028, 18180, 0.08), (28439, 18180, 0.0)] {
        db.conn.execute("UPDATE equip_pack_cloths SET cosplay_id=?1,shoes_id=?2 WHERE id=500", params![costume, feet]).unwrap();
        assert_eq!(heel(&db).to_bits(), expected.to_bits(), "costume={costume}, feet={feet}");
    }
    db.conn.execute("UPDATE characters SET model_id=10 WHERE id=90", []).unwrap();
    db.conn.execute("UPDATE equip_pack_cloths SET cosplay_id=8000028 WHERE id=500", []).unwrap();
    assert_eq!(heel(&db).to_bits(), 0, "male reference selects its own asset");
}
