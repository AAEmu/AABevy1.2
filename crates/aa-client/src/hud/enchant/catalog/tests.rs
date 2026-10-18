use super::*;

fn original() -> Tables {
    let conn = Connection::open_with_flags(aa_anim::db::DEFAULT_DB, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    Tables::load(&conn).unwrap()
}
fn item(template_id: u32, grade: u8) -> Item {
    Item {
        template_id,
        grade,
        ..Item::default()
    }
}

#[test]
fn quotes_match_33_executed_native_formula_and_slot_controls() {
    let mut data = original();
    // Actual397cc850 routing +395684c0 AST/float/CVTT oracle, readonly catalog.
    for (template, expected) in [
        (31432, [163517, 157537, 506607]),
        (19326, [140464, 135017, 320177]),
        (28905, [425146, 409598, 1317179]),
        (18420, [235366, 225013, 428586]),
    ] {
        for (grade, value) in [0, 1, 6].into_iter().zip(expected) {
            assert_eq!(data.cost(&item(template, grade)), Some(value), "{template}/{grade}");
            let level = data.items[&template].level;
            let cost = data.slot_cost[&data.slot[&template]];
            assert_eq!(data.formula_cost(level, grade, cost), Some(value));
        }
        let slot = data.slot[&template];
        let cost = data.slot_cost.remove(&slot).unwrap();
        assert_eq!(data.cost(&item(template, 6)), Some(0));
        data.slot_cost.insert(slot, cost);
    }
    for (level, grade, cost, expected) in [
        (127, 6, 13., 29235252),
        (-128, 6, 13., -30112808),
        (50, 11, 13., 1522598),
        (50, 6, -1., -55576),
        (0, 0, 0., 0),
    ] {
        assert_eq!(data.formula_cost(level, grade, cost), Some(expected));
    }
    assert_eq!(data.cost(&item(u32::MAX, 6)), Some(0));
    assert_eq!(data.cost(&item(31432, 255)), None);
    assert_eq!(cvtt(f64::NAN), i32::MIN);
    assert_eq!(cvtt(2147483648.), i32::MIN);
}

#[test]
fn all_39_original_ratio_getter_goldens_preserve_bool_availability_and_relative_changes() {
    let data = original();
    let ids = [31974, 31975, 31976, 31977, 31978, 31979, 31980, 31981, 31982, 31983, 34323, 34563];
    let values = [
        [
            [50, 0, 0],
            [50, 0, 0],
            [100, 0, 0],
            [100, 0, 0],
            [0, 0, 0],
            [0, 0, 0],
            [0, 0, 0],
            [0, 0, 0],
            [0, 0, 100],
            [100, 0, 0],
            [75, 0, 0],
            [75, 0, 0],
        ],
        [
            [0, 0, 0],
            [0, 0, 0],
            [0, 0, 0],
            [0, 0, 0],
            [0, 0, 0],
            [0, 0, 0],
            [0, 0, 0],
            [0, 0, 0],
            [0, 0, 100],
            [0, 0, 0],
            [0, 0, 0],
            [0, 0, 0],
        ],
        [
            [50, 0, 0],
            [50, 0, 0],
            [100, 0, 0],
            [100, 0, 0],
            [0, 0, 0],
            [0, 0, 0],
            [0, 0, -50],
            [0, 0, -100],
            [0, 0, 17],
            [566, 0, -100],
            [75, 0, 0],
            [75, 0, 0],
        ],
    ];
    for (i, grade) in [0, 1, 6].into_iter().enumerate() {
        let base = [true, false, grade == 6];
        assert_eq!(data.ratios(grade, None), Some((base, None)));
        for (id, want) in ids.into_iter().zip(values[i]) {
            assert_eq!(data.ratios(grade, Some(id)), Some((base, Some(want))), "grade{grade},support{id}");
        }
    }
    assert!(data.ratios(255, None).is_none());
    assert!(data.ratios(0, Some(0)).is_none());
}

#[test]
fn source_descriptors_grade_order_gem_groups_and_prefix_gems_remain_distinct() {
    let mut data = original();
    assert_eq!(&data.grade_order[..3], &[1, 0, 2]);
    assert_eq!(data.socket_minimum, Some(20));
    assert_eq!(data.items[&327].skill, 23729);
    assert!(!data.items[&327].reagent);
    for desc in data
        .items
        .values()
        .filter(|d| matches!(d.skill, 22520 | 22522 | 22523 | 22524 | 24998 | 24999 | 25503 | 23728 | 11285))
    {
        assert!(desc.reagent);
        assert!(desc.actability_group == 0 || desc.actability_requirement == 0);
    }
    let mut target = item(31432, 0);
    target.detail = ItemDetail::Equip(aa_net::model::EquipDetail {
        gem_ids: [10, 0, 20, 30, 0, 0, 0],
        ..Default::default()
    });
    assert_eq!(gem_count(&target), 1);
    assert!(data.group_allows(0, 123));
    assert!(!data.group_allows(u32::MAX, 123));
    data.groups.insert(u32::MAX, HashSet::new());
    assert!(data.group_allows(u32::MAX, 123));
    data.groups.get_mut(&u32::MAX).unwrap().insert(5);
    assert!(!data.group_allows(u32::MAX, 123));
    assert!(data.group_allows(u32::MAX, 5));
    let source = item(0, 0);
    let support = item(31974, 0);
    assert!(!data.support_allows(&item(30803, 0), &source, &support));
    assert!(!data.support_allows(&target, &item(30804, 0), &support));
}

#[test]
fn original_source_modifiers_keep_native_primary_order_and_empty_tables() -> mlua::Result<()> {
    let data = original();
    let lua = mlua::Lua::new();
    // All357 original impl15/21 sources have source-proven zero generated primary
    // bonuses. These static vectors are native owner="Item" attachments.
    assert_eq!(data.modifiers.len(), 357);
    let socket = data.modifier_table(&lua, 31003)?.unwrap();
    assert_eq!(socket.raw_len(), 1);
    let row: mlua::Table = socket.raw_get(1)?;
    assert_eq!(row.get::<String>("name")?, "max_mana");
    assert_eq!(row.get::<f64>("value")?, 200.);
    assert_eq!(row.get::<f64>("type")?, 0.);
    // DB descriptor order is sta then dex;3941dc60 instead emits primary attr
    // IDs0..4 in ascending order, with the native literal names and values.
    let gem = data.modifier_table(&lua, 24632)?.unwrap();
    assert_eq!(gem.raw_len(), 2);
    for (i, name, value) in [(1, "dex", 2.), (2, "sta", 3.)] {
        let row: mlua::Table = gem.raw_get(i)?;
        assert_eq!(row.get::<String>("name")?, name);
        assert_eq!(row.get::<f64>("value")?, value);
        assert_eq!(row.get::<f64>("type")?, 0.);
    }
    assert_eq!(data.modifier_table(&lua, 24758)?.unwrap().raw_len(), 0);
    assert!(data.modifier_table(&lua, 31432)?.is_none());
    Ok(())
}
