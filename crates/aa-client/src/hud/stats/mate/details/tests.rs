use super::*;
use crate::hud::data::GameData;
use aa_net::model::{EquipDetail, Item, UnitKind};
use bevy::prelude::default;

fn unit() -> aa_net::Unit {
    let mut unit = crate::hud::expansion::fixture::spawn().unit;
    unit.kind = UnitKind::Mate {
        template_id: 11554,
        tl_id: 12,
        owner_id: 47,
    };
    unit.level = 5;
    unit.equipment = Equipment::default();
    unit
}
fn game() -> GameData {
    GameData::load(aa_anim::db::DEFAULT_DB).unwrap()
}
fn buff(id: u32) -> BuffEntry {
    BuffEntry {
        buff: aa_net::buff::Buff {
            buff_id: id,
            ability_level: 1,
            stacks: 1,
            ..default()
        },
        received: web_time::Instant::now(),
    }
}
fn bonus(attr: u8, value: i32) -> Bonus {
    Bonus {
        attr,
        percent: false,
        value,
        per_level: 0,
    }
}

#[test]
fn bare_pet_fields_use_own_owner5_attributes_native_units_and_fixed_5300_denominator() {
    let game = game();
    let unit = unit();
    let info = get(&game, &unit, Some(&[])).unwrap();
    assert_eq!(
        info.values.map(f32::to_bits),
        [
            0x41e9999a, 0x40cf76f9, 0x40a5f8c7, 0, 0, 0x42500000, 0x3f78babd, 0x3f800000, 0x41200000, 0x3f800000, 0x41100000,
        ]
    );
    assert_eq!(info.modifiers, [0.; 17]);
    assert!(get(&game, &unit, None).is_none());
    let mut unresolved = unit;
    unresolved.kind = UnitKind::Mate {
        template_id: 504,
        tl_id: 12,
        owner_id: 47,
    };
    assert!(get(&game, &unresolved, Some(&[])).is_none());
}

#[test]
fn rating_multiplier_uses_negative_thousandth_and_modifier_fields_keep_raw_units() {
    let mut game = game();
    let id = 900101;
    game.stats
        .mate
        .details
        .buffs
        .insert(id, vec![bonus(77, 100), bonus(180, -10), bonus(33, 1000), bonus(67, 10)]);
    let info = get(&game, &unit(), Some(&[buff(id)])).unwrap();
    let inv = 1.0f32 / 401032.0f32;
    assert_eq!(info.values[1], 66103.0f32 * inv * 100.0f32);
    assert_eq!(info.values[2], 16790.0f32 * inv * 100.0f32);
    assert_eq!(info.values[0], 30.2f32);
    assert_eq!(info.modifiers[0], 999.9993286132812f32);
    assert_eq!(info.modifiers[4], -10.);
    assert_eq!(info.modifiers[8], 10.);
    assert_eq!(info.values[7], 3.);
    assert_eq!(info.values[8], 12.);
}

#[test]
fn modifier_is_independent_of_final_attribute_clamp_and_static_percentage_baseline() {
    let mut game = game();
    let id = 900102;
    game.stats.mate.details.buffs.insert(id, vec![bonus(8, 100)]);
    game.stats.mate.details.limits.insert(8, (0, 0));
    let info = get(&game, &unit(), Some(&[buff(id)])).unwrap();
    assert_eq!(info.values[3], 0.);
    assert_eq!(info.modifiers[5], 100.);
    assert_eq!(
        Pair::default().modifier(16777217, Pair { flat: 33, percent: 117 }),
        0.,
        "native exact neutral shortcut"
    );
    assert_eq!(Pair { flat: 100, percent: 150 }.final_value(1000, Some(&(0, 1400))), 1400);
}

#[test]
fn relevant_conditional_and_active_duration_modifiers_remain_unknown_and_one_time_skips_them() {
    let mut game = game();
    let id = 900103;
    let unit = unit();
    game.stats.mate.details.dynamic.insert(id);
    game.stats.mate.details.buffs.insert(id, vec![bonus(33, 1000)]);
    let mut entry = buff(id);
    entry.buff.duration_ms = 1000;
    assert!(get(&game, &unit, Some(&[entry.clone()])).is_some());
    entry.buff.tick_ms = 100;
    assert!(get(&game, &unit, Some(&[entry.clone()])).is_none());
    game.stats.mate.one_time.insert(id);
    assert_eq!(
        get(&game, &unit, Some(&[entry.clone()])).unwrap().values,
        get(&game, &unit, Some(&[])).unwrap().values
    );
    game.stats.mate.one_time.remove(&id);
    entry.buff.tick_ms = 0;
    game.stats.mate.details.conditional.insert(id);
    assert!(get(&game, &unit, Some(&[entry])).is_none());
}

#[test]
fn modifier_skip_main_gear_still_includes_runes_and_stops_gems_at_zero() {
    let mut game = game();
    let mut unit = unit();
    let slot = (game.tips.items.slot_type(31432) - 1) as usize;
    unit.equipment.slots[slot] = Some(Item {
        template_id: 31432,
        id: 100,
        grade: 0,
        detail: ItemDetail::Equip(EquipDetail { durability: 100, ..default() }),
        ..default()
    });
    assert_eq!(
        get(&game, &unit, Some(&[])).unwrap().modifiers[11],
        0.,
        "main generated DEX is omitted by source modifier getter"
    );
    for (id, value) in [(900104, 100), (900105, 20), (900106, 999)] {
        game.stats.mate.item_levels.insert(id, 5);
        game.stats.mate.item_mods.insert(id, vec![bonus(1, value)]);
        game.stats.mate.details.items.insert(id, vec![bonus(1, value)]);
    }
    let ItemDetail::Equip(detail) = &mut unit.equipment.slots[slot].as_mut().unwrap().detail else {
        panic!()
    };
    detail.rune_id = 900104;
    detail.gem_ids = [900105, 0, 900106, 0, 0, 0, 0];
    assert_eq!(get(&game, &unit, Some(&[])).unwrap().modifiers[11], 120.);
}

#[test]
fn modifier_values_match_ten_executed_original_baseline_and_x87_delta_goldens() {
    // Original394bb740; B/S/T lookups mocked, original arithmetic and neutral shortcut executed.
    for (base, static_flat, static_percent, flat, percent, expected) in [
        (29200, 0, 100, 0, 100, 0.0f32),
        (29200, 3, 101, 0, 100, 0.0f32),
        (29200, 3, 101, 1, 100, 0.9993407130241394f32),
        (29200, 3, 101, 0, 101, 294.9496154785156f32),
        (401032, 11, 127, -37, 88, -61151.51953125f32),
        (16777217, 1, 100, 1, 103, 503317.125f32),
        (-25, 1, 99, 0, 100, 0.0f32),
        (29200, 0, 100, 1000, 100, 999.9993286132812f32),
        (0, 0, 100, -10, 100, -10.0f32),
        (5, 0, 100, 10, 100, 10.0f32),
    ] {
        assert_eq!(
            Pair { flat, percent }.modifier(
                base,
                Pair {
                    flat: static_flat,
                    percent: static_percent
                }
            ),
            expected
        );
    }
}
