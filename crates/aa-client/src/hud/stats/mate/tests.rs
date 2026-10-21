use super::*;
use crate::hud::data::GameData;
use aa_net::model::{EquipDetail, Item, UnitKind};
use bevy::prelude::default;

fn game() -> GameData {
    GameData::load(aa_anim::db::DEFAULT_DB).unwrap()
}
fn unit(template: u32) -> aa_net::Unit {
    let mut unit = crate::hud::expansion::fixture::spawn().unit;
    unit.kind = UnitKind::Mate {
        template_id: template,
        tl_id: 12,
        owner_id: 47,
    };
    unit.level = 5;
    unit.equipment = Equipment::default();
    unit
}

#[test]
fn primary_gear_stats_match_eighteen_executed_original_x86_oracles() {
    let g = game();
    for (template, grade, sta, int) in [
        (31432, 0, 29, 23),
        (31432, 1, 23, 19),
        (31432, 6, 41, 33),
        (32920, 0, 5, 4),
        (32920, 1, 4, 3),
        (32920, 6, 7, 6),
        (28905, 0, 32, 63),
        (28905, 1, 26, 50),
        (28905, 6, 44, 89),
    ] {
        assert_eq!(item_stat(&g.stats, template, grade, 2), sta, "STA {template} grade{grade}");
        assert_eq!(item_stat(&g.stats, template, grade, 3), int, "INT {template} grade{grade}");
    }
}

#[test]
fn pool_formula_uses_mate_owner_and_per_formula_kind_coefficients() {
    let g = game();
    assert_eq!(pools(&g, &unit(11554), Some(&[])), Some((1012, 335)));
    assert_eq!(
        pools(&g, &unit(10027), Some(&[])),
        Some((1113, 309)),
        "MP result rounds through f32 before integer truncation"
    );
    assert_eq!(pools(&g, &unit(11554), None), None, "unknown native buff history is not an empty known list");
    assert_eq!(pools(&g, &unit(504), Some(&[])), None, "an unresolved mate-kind is not an NPC-grade formula");
}

#[test]
fn native_modifier_rounding_aggregates_percentages_and_preserves_integer_fast_path() {
    for (linear, level, multiplier, expected) in [
        (1, 50, 1, 11),
        (1, 150, 1, 12),
        (-1, 50, 1, 10),
        (-1, 150, 1, 9),
        (3, 50, 1, 12),
        (1, 50, 3, 33),
        (1, 50, 0, 0),
    ] {
        let b = Bonus {
            attr: 6,
            percent: false,
            value: 10,
            per_level: linear,
        };
        assert_eq!(amount(&b, level, multiplier), expected);
    }
    let mut totals = Totals::default();
    let mut tables = Tables::default();
    totals.add(
        Some(&vec![
            Bonus {
                attr: 6,
                percent: false,
                value: 100,
                per_level: 0,
            },
            Bonus {
                attr: 6,
                percent: true,
                value: 20,
                per_level: 0,
            },
            Bonus {
                attr: 6,
                percent: true,
                value: 30,
                per_level: 0,
            },
        ]),
        0,
        1,
    );
    assert_eq!(
        totals.apply(6, 1000, &tables),
        1650,
        "percent contributions add before one final multiplication"
    );
    tables.limits.insert(6, (0, 1400));
    assert_eq!(totals.apply(6, 1000, &tables), 1400);
    assert_eq!(
        Totals::default().apply(7, 16_777_217, &tables),
        16_777_217,
        "percent100 keeps integer base+flat precision"
    );
}

#[test]
fn worn_item_modifiers_use_item_level_and_broken_or_wrong_slots_skip_secondary_items() {
    let mut g = game();
    let mut u = unit(11554);
    let slot = (g.tips.items.slot_type(31432) - 1) as usize;
    assert!(slot < 19);
    let mut detail = EquipDetail { durability: 100, ..default() };
    detail.rune_id = 900001;
    detail.gem_ids = [900002, 0, 900003, 0, 0, 0, 0];
    u.equipment.slots[slot] = Some(Item {
        template_id: 31432,
        id: 100,
        grade: 0,
        detail: ItemDetail::Equip(detail),
        ..default()
    });
    assert_eq!(pools(&g, &u, Some(&[])), Some((1360, 565)));
    g.stats.mate.item_mods.insert(
        31432,
        vec![Bonus {
            attr: 6,
            percent: false,
            value: 10,
            per_level: 1,
        }],
    );
    for (template, attr) in [(900001, 6), (900002, 7), (900003, 7)] {
        g.stats.mate.item_levels.insert(template, 5);
        g.stats.mate.item_mods.insert(
            template,
            vec![Bonus {
                attr,
                percent: false,
                value: 100,
                per_level: 0,
            }],
        );
    }
    assert_eq!(
        pools(&g, &u, Some(&[])),
        Some((1471, 665)),
        "direct mods use signed item level; gems stop at first sentinel"
    );
    let ItemDetail::Equip(detail) = &mut u.equipment.slots[slot].as_mut().unwrap().detail else {
        panic!()
    };
    detail.durability = 0;
    assert_eq!(
        pools(&g, &u, Some(&[])),
        Some((1012, 335)),
        "broken base skips generated stats, direct modifiers, rune and gems"
    );
    let item = u.equipment.slots[slot].take().unwrap();
    u.equipment.slots[19] = Some(item);
    assert_eq!(
        pools(&g, &u, Some(&[])),
        Some((1012, 335)),
        "body-part slots19..26 are not part of the native scan"
    );
    assert!(slot_matches(10, 9) && slot_matches(10, 10) && !slot_matches(10, 11));
    assert!(slot_matches(17, 15) && slot_matches(17, 16) && slot_matches(31, 27) && !slot_matches(19, 18));
}

#[test]
fn buff_stacks_use_actual_native_history_and_unsupported_dynamic_rows_remain_unknown() {
    let mut g = game();
    let u = unit(11554);
    let id = 900010;
    g.stats.buff_bonuses.insert(
        id,
        vec![Bonus {
            attr: 6,
            percent: false,
            value: 10,
            per_level: 1,
        }],
    );
    let mut buffs = vec![BuffEntry {
        buff: aa_net::buff::Buff {
            buff_id: id,
            ability_level: 50,
            stacks: 3,
            charged: 99,
            ..default()
        },
        received: web_time::Instant::now(),
    }];
    assert_eq!(pools(&g, &u, Some(&buffs)), Some((1045, 335)), "stack count is independent of charge");
    g.stats.mate.dynamic.insert(id);
    buffs[0].buff.duration_ms = 1000;
    assert_eq!(pools(&g, &u, Some(&buffs)), Some((1045, 335)), "zero tick skips duration modifier producer");
    buffs[0].buff.tick_ms = 100;
    assert_eq!(pools(&g, &u, Some(&buffs)), None);
    g.stats.mate.one_time.insert(id);
    assert_eq!(pools(&g, &u, Some(&buffs)), Some((1012, 335)), "one-time skips the whole modifier callback");
    g.stats.mate.one_time.remove(&id);
    g.stats.mate.dynamic.remove(&id);
    g.stats.mate.conditional.insert(id);
    assert_eq!(pools(&g, &u, Some(&buffs)), None);
}
