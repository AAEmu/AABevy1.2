//! Research S8: every loot item a plantable doodad can give exists in `items`.
//!
//! The audit walks the doodads in `item_spawn_doodads` (335 on the 1.2 data), their phase funcs and their
//! `doodad_funcs`, then `DoodadFuncLootPack` -> `doodad_func_loot_packs` -> `loots` -> `items` and
//! `DoodadFuncLootItem` -> `doodad_func_loot_items` -> `items`. It also follows the `next_phase` of the three funcs that
//! carry no item id of their own (`DoodadFuncFruitPick`, `DoodadFuncFakeUse`, `DoodadFuncCutdowning`) into that phase's
//! loot rows. It reports the holes and pins the set the 1.2 data actually has, so a new broken loot row fails a test
//! instead of only showing up as an empty loot roll.
//!
//! `cargo test -p aa-client --bin aa-client every_plantable -- --nocapture` prints the report. Without `AA_DB` the
//! test skips cleanly.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{Connection, OpenFlags};

/// `(doodad, func group, kind, func id, item id)`; item 0 means the func's own loot row is missing.
type Missing = (u32, u32, &'static str, u32, u32);

/// Collects one func group's loot rows and checks each item against `items`.
fn scan_group(
    conn: &Connection,
    doodad: u32,
    group: u32,
    packs: &mut BTreeSet<i64>,
    loot_rows: &mut usize,
    missing: &mut BTreeSet<Missing>,
) {
    let funcs: BTreeSet<(String, u32)> = conn
        .prepare(
            "SELECT actual_func_type, actual_func_id FROM doodad_funcs WHERE doodad_func_group_id = ?1
             UNION
             SELECT actual_func_type, actual_func_id FROM doodad_phase_funcs WHERE doodad_func_group_id = ?1",
        )
        .unwrap()
        .query_map([group], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u32)))
        .unwrap()
        .flatten()
        .collect();
    for (kind, func_id) in funcs {
        let (table, column, pack, kind_name) = match kind.as_str() {
            "DoodadFuncLootPack" => ("doodad_func_loot_packs", "loot_pack_id", true, "LootPack"),
            "DoodadFuncLootItem" => ("doodad_func_loot_items", "item_id", false, "LootItem"),
            _ => continue,
        };
        let value: Option<i64> = conn
            .query_row(
                &format!("SELECT {column} FROM {table} WHERE id = ?1"),
                [func_id],
                |r| r.get(0),
            )
            .ok();
        let Some(value) = value else {
            missing.insert((doodad, group, kind_name, func_id, 0));
            continue;
        };
        if pack {
            packs.insert(value);
            let items: Vec<u32> = conn
                .prepare("SELECT item_id FROM loots WHERE loot_pack_id = ?1")
                .unwrap()
                .query_map([value], |r| r.get::<_, i64>(0))
                .unwrap()
                .flatten()
                .map(|id| id as u32)
                .collect();
            for item in items {
                *loot_rows += 1;
                if conn
                    .query_row("SELECT 1 FROM items WHERE id = ?1", [item], |_| Ok(()))
                    .is_err()
                {
                    missing.insert((doodad, group, kind_name, func_id, item));
                }
            }
        } else {
            *loot_rows += 1;
            let item = value as u32;
            if conn
                .query_row("SELECT 1 FROM items WHERE id = ?1", [item], |_| Ok(()))
                .is_err()
            {
                missing.insert((doodad, group, kind_name, func_id, item));
            }
        }
    }
}

#[test]
fn every_plantable_doodad_loot_item_exists() {
    let Some(path) = std::env::var("AA_DB")
        .ok()
        .filter(|p| std::path::Path::new(p).exists())
    else {
        eprintln!("skip: no AA_DB database");
        return;
    };
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let doodads: Vec<u32> = conn
        .prepare("SELECT DISTINCT doodad_id FROM item_spawn_doodads WHERE doodad_id IS NOT NULL")
        .unwrap()
        .query_map([], |r| r.get::<_, i64>(0))
        .unwrap()
        .flatten()
        .map(|id| id as u32)
        .collect();
    let mut packs = BTreeSet::new();
    let mut loot_rows = 0;
    let mut missing = BTreeSet::new();
    for doodad in &doodads {
        let groups: Vec<u32> = conn
            .prepare("SELECT id FROM doodad_func_groups WHERE doodad_almighty_id = ?1")
            .unwrap()
            .query_map([doodad], |r| r.get::<_, i64>(0))
            .unwrap()
            .flatten()
            .map(|id| id as u32)
            .collect();
        for group in groups {
            scan_group(
                &conn,
                *doodad,
                group,
                &mut packs,
                &mut loot_rows,
                &mut missing,
            );
        }
    }
    // FruitPick / FakeUse / Cutdowning carry no item id: their loot is the `next_phase` group's. The main walk already
    // visits every group of the doodad; this pass makes that path explicit and reports what it covers.
    let mut seen = BTreeSet::new();
    let mut transition_groups = 0;
    let mut transition_loot = 0;
    let mut q = conn
        .prepare(
            "SELECT DISTINCT d.doodad_id, f.next_phase
             FROM doodad_funcs f
             JOIN doodad_func_groups g ON g.id = f.doodad_func_group_id
             JOIN item_spawn_doodads d ON d.doodad_id = g.doodad_almighty_id
             WHERE f.actual_func_type IN ('DoodadFuncFruitPick', 'DoodadFuncFakeUse', 'DoodadFuncCutdowning')
               AND f.next_phase > 0",
        )
        .unwrap();
    let transitions: Vec<(u32, u32)> = q
        .query_map([], |r| {
            Ok((r.get::<_, i64>(0)? as u32, r.get::<_, i64>(1)? as u32))
        })
        .unwrap()
        .flatten()
        .collect();
    for (doodad, group) in transitions {
        if !seen.insert((doodad, group)) {
            continue;
        }
        let before = transition_loot;
        scan_group(
            &conn,
            doodad,
            group,
            &mut packs,
            &mut transition_loot,
            &mut missing,
        );
        if transition_loot > before {
            transition_groups += 1;
        }
    }
    let mut by_item: BTreeMap<u32, Vec<(u32, u32, &str, u32)>> = BTreeMap::new();
    for &(doodad, group, kind, func, item) in &missing {
        by_item
            .entry(item)
            .or_default()
            .push((doodad, group, kind, func));
    }
    println!(
        "plantable loot audit: {} doodads, {} loot packs, {} loot rows, {} missing references",
        doodads.len(),
        packs.len(),
        loot_rows,
        missing.len()
    );
    println!(
        "  FruitPick/FakeUse/Cutdowning next_phase: {transition_groups} loot groups, {transition_loot} loot rows"
    );
    for (item, refs) in &by_item {
        println!("  item {item}: {refs:?}");
    }
    // The holes S8 was looking for: three LootItem rows name items the 1.2 database does not have.
    let missing_items: Vec<u32> = by_item.keys().copied().collect();
    assert_eq!(
        missing_items,
        vec![34156, 34157, 34158],
        "plantable doodad loot references items missing from `items` (see the printed report)"
    );
}
