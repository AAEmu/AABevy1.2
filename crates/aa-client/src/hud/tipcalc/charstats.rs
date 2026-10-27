//! The player's Character Info numbers: `X2Unit:UnitInfo("player")`, `UnitModifierInfo("player")`.
//!
//! The 1.2 server sends no derived stats; the client works them out from the game database: the
//! character formulas (`unit_formulas` owner 0) over level and primary stats, the worn items
//! (stats, armor, magic resistance, weapon DPS) and the `unit_modifiers` of the worn items and the
//! buffs, applied the way the server's `CharStats` does (flat bonuses, then each percent bonus on
//! the running value). The key set and the units are those of the original client, recorded by the
//! server repo's `oracle stats` (`rust/docs/client/LUA-DRIVER.md`, JSON captures at
//! `<shots>/oracle-stats-L*.json`): DPS in real DPS, `*_mul` as percent deltas, rates
//! in percent.
//!
//! `UnitModifierInfo` is the part of a number the modifiers add: the value with only the key's own
//! attribute bonuses minus the value without bonuses (`armor` 200 from the shield buff, `max_health`
//! 1409.6 for `(10246 + 350) * 1.1 - 10246`).

use std::collections::HashMap;

use aa_net::model::{slot, EquipDetail, Equipment, ItemDetail};

use super::{with_bonuses, TipTables};

/// `(attribute id, percent, value)`.
type Mods = [(u8, bool, i32)];

/// One value per key, in the order of [`INFO_KEYS`] / [`MODIFIER_KEYS`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerSheet {
    pub info: Vec<(&'static str, f64)>,
    pub modifiers: Vec<(&'static str, f64)>,
}

/// The keys of `UnitInfo` (original client, `unit_info_keys` of the oracle capture).
pub const INFO_KEYS: &[&str] = &[
    "armor", "armor_percentage", "battle_resist", "block_rate", "bulls_eye", "casting_time", "casting_time_mul", "dex", "dodge_rate",
    "flexibility", "global_cooldown_mul", "heal_critical_bonus", "heal_critical_rate", "heal_dps", "health_regen",
    "incoming_melee_damage_mul", "incoming_melee_damage_val", "incoming_ranged_damage_mul", "incoming_ranged_damage_val",
    "incoming_siege_damage_mul", "incoming_siege_damage_val", "incoming_spell_damage_mul", "incoming_spell_damage_val", "int",
    "magic_effect_resist_percentage", "magic_resist", "magic_resist_percentage", "mainhand_melee_speed", "mana_regen",
    "melee_critical_bonus", "melee_critical_rate", "melee_damage_mul", "melee_dps", "melee_max_dps", "melee_min_dps", "melee_parry_rate",
    "melee_success_rate", "move_speed", "offhand_melee_speed", "persistent_health_regen", "persistent_mana_regen", "ranged_critical_bonus",
    "ranged_critical_rate", "ranged_damage_mul", "ranged_dps", "ranged_max_dps", "ranged_min_dps", "ranged_parry_rate", "ranged_speed",
    "ranged_success_rate", "spell_critical_bonus", "spell_critical_rate", "spell_damage_mul", "spell_dps", "spell_success_rate", "spi",
    "sta", "str",
];

/// The keys of `UnitModifierInfo` (original client).
pub const MODIFIER_KEYS: &[&str] = &[
    "armor", "battle_resist", "block_mul", "block_rate", "bulls_eye", "casting_time", "casting_time_mul", "dex", "dodge_mul", "dodge_rate",
    "flexibility", "global_cooldown_mul", "heal_critical_bonus", "heal_critical_mul", "heal_critical_rate", "heal_dps", "health_regen",
    "incoming_melee_damage_mul", "incoming_melee_damage_val", "incoming_ranged_damage_mul", "incoming_ranged_damage_val",
    "incoming_siege_damage_mul", "incoming_siege_damage_val", "incoming_spell_damage_mul", "incoming_spell_damage_val", "int",
    "magic_effect_resist_percentage", "magic_resist", "mainhand_melee_speed", "mana_regen", "max_health", "max_mana",
    "melee_attack_speed_mul", "melee_critical_bonus", "melee_critical_mul", "melee_critical_rate", "melee_damage_mul", "melee_dps",
    "melee_max_dps", "melee_min_dps", "melee_parry_mul", "melee_parry_rate", "melee_success_rate", "move_speed", "offhand_melee_speed",
    "persistent_health_regen", "persistent_mana_regen", "ranged_attack_speed_mul", "ranged_critical_bonus", "ranged_critical_mul",
    "ranged_critical_rate", "ranged_damage_mul", "ranged_dps", "ranged_max_dps", "ranged_min_dps", "ranged_parry_mul", "ranged_parry_rate",
    "ranged_speed", "ranged_success_rate", "spell_critical_bonus", "spell_critical_mul", "spell_critical_rate",
    "spell_damage_critical_bonus", "spell_damage_critical_mul", "spell_damage_mul", "spell_dps", "spell_success_rate", "spi", "sta", "str",
];

/// `UnitFormulaKind` of the character formulas.
mod kind {
    pub const MELEE_CRITICAL: u8 = 1;
    pub const MELEE_ANTI_MISS: u8 = 2;
    pub const MELEE_PARRY: u8 = 5;
    pub const RANGED_CRITICAL: u8 = 6;
    pub const RANGED_ANTI_MISS: u8 = 7;
    pub const SPELL_CRITICAL: u8 = 10;
    pub const SPELL_ANTI_MISS: u8 = 11;
    pub const MAX_HEALTH: u8 = 14;
    pub const MAX_MANA: u8 = 15;
    pub const HEALTH_REGEN: u8 = 16;
    pub const MANA_REGEN: u8 = 17;
    pub const ARMOR: u8 = 18;
    pub const MAGIC_RESIST: u8 = 19;
    pub const FACET: u8 = 20;
    pub const STR: u8 = 22;
    pub const PERSISTENT_HEALTH_REGEN: u8 = 31;
    pub const PERSISTENT_MANA_REGEN: u8 = 32;
    pub const RANGED_PARRY: u8 = 38;
    pub const INCOMING_MELEE_DAMAGE_ADD: u8 = 39;
    pub const INCOMING_RANGED_DAMAGE_ADD: u8 = 40;
    pub const INCOMING_SPELL_DAMAGE_ADD: u8 = 41;
    pub const HEAL_CRITICAL: u8 = 42;
    pub const BLOCK: u8 = 45;
    pub const DODGE: u8 = 46;
}

/// `UnitAttribute` ids (the server's `models/unit.rs`; `tipcalc::attribute_name` has the same table).
mod attr {
    pub const MOVE_SPEED_MUL: u8 = 10;
    pub const HEALTH_REGEN: u8 = 11;
    pub const MANA_REGEN: u8 = 12;
    pub const FACETS: u8 = 13;
    pub const MELEE_CRITICAL: u8 = 16;
    pub const MELEE_CRITICAL_BONUS: u8 = 17;
    pub const MELEE_ANTI_MISS: u8 = 18;
    pub const MELEE_PARRY: u8 = 22;
    pub const RANGED_ANTI_MISS: u8 = 23;
    pub const RANGED_CRITICAL: u8 = 25;
    pub const RANGED_CRITICAL_BONUS: u8 = 26;
    pub const SPELL_ANTI_MISS: u8 = 28;
    pub const SPELL_CRITICAL: u8 = 30;
    pub const SPELL_CRITICAL_BONUS: u8 = 31;
    pub const MELEE_DAMAGE_MUL: u8 = 51;
    pub const RANGED_DAMAGE_MUL: u8 = 52;
    pub const SPELL_DAMAGE_MUL: u8 = 53;
    pub const MELEE_SPEED_MUL: u8 = 54;
    pub const RANGED_SPEED_MUL: u8 = 55;
    pub const INCOMING_DAMAGE_MUL: u8 = 58;
    pub const ARMOR: u8 = 8;
    pub const MAGIC_RESIST: u8 = 64;
    pub const PERSISTENT_HEALTH_REGEN: u8 = 67;
    pub const PERSISTENT_MANA_REGEN: u8 = 68;
    pub const CASTING_TIME_MUL: u8 = 71;
    pub const GLOBAL_COOLDOWN_MUL: u8 = 74;
    pub const MELEE_CRITICAL_MUL: u8 = 77;
    pub const MELEE_ANTI_MISS_MUL: u8 = 78;
    pub const MELEE_PARRY_MUL: u8 = 81;
    pub const RANGED_CRITICAL_MUL: u8 = 82;
    pub const RANGED_ANTI_MISS_MUL: u8 = 83;
    pub const SPELL_CRITICAL_MUL: u8 = 86;
    pub const SPELL_DPS: u8 = 87;
    pub const SPELL_ANTI_MISS_MUL: u8 = 88;
    pub const MAINHAND_DPS: u8 = 96;
    pub const RANGED_DPS: u8 = 98;
    pub const HEAL_MUL: u8 = 120;
    pub const INCOMING_MELEE_DAMAGE_ADD: u8 = 142;
    pub const INCOMING_MELEE_DAMAGE_MUL: u8 = 143;
    pub const INCOMING_RANGED_DAMAGE_ADD: u8 = 144;
    pub const INCOMING_RANGED_DAMAGE_MUL: u8 = 145;
    pub const INCOMING_SPELL_DAMAGE_ADD: u8 = 146;
    pub const INCOMING_SPELL_DAMAGE_MUL: u8 = 147;
    pub const INCOMING_SIEGE_DAMAGE_ADD: u8 = 148;
    pub const INCOMING_SIEGE_DAMAGE_MUL: u8 = 149;
    pub const SPELL_DAMAGE_CRITICAL: u8 = 150;
    pub const SPELL_DAMAGE_CRITICAL_MUL: u8 = 151;
    pub const SPELL_DAMAGE_CRITICAL_BONUS: u8 = 152;
    pub const RANGED_PARRY: u8 = 153;
    pub const RANGED_PARRY_MUL: u8 = 154;
    pub const HEAL_DPS: u8 = 173;
    pub const HEAL_CRITICAL: u8 = 174;
    pub const HEAL_CRITICAL_BONUS: u8 = 176;
    pub const BLOCK: u8 = 177;
    pub const DODGE: u8 = 178;
    pub const BLOCK_MUL: u8 = 179;
    pub const DODGE_MUL: u8 = 180;
    pub const BULLS_EYE: u8 = 181;
    pub const BATTLE_RESIST: u8 = 182;
    pub const FLEXIBILITY: u8 = 183;
    pub const HEAL_CRITICAL_MUL: u8 = 185;
}

/// Armor / magic resistance to the percent the window shows: `x / (x + 5300)`, fitted to the oracle
/// (armor 269 -> 4.8303 %, magic resistance 377 / 475 / 525 -> 6.6408 / 8.2251 / 9.0129 %, levels 1 to 55).
const RESIST_CONSTANT: f64 = 5300.0;
/// Base movement speed in m/s (the oracle's `move_speed` with no modifier).
const MOVE_SPEED: f64 = 5.4;
/// `EquipmentItemSlotType.Shield`.
const SLOT_SHIELD: u32 = 20;

#[derive(Clone, Copy, Debug, Default)]
struct Weapon {
    dps: f64,
    mdps: f64,
    hdps: f64,
    scale: i32,
    /// Attack delay in seconds.
    speed: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Offhand {
    #[default]
    Empty,
    Shield,
    Other,
}

/// Attack delays (seconds) of the bare hands in the retail client: main hand, off hand (only with both hands
/// empty) and ranged.
const UNARMED_MAINHAND_SPEED: f64 = 1.5;
const UNARMED_OFFHAND_SPEED: f64 = 3.0;
const UNARMED_RANGED_SPEED: f64 = 1.5;

#[derive(Clone, Debug, Default)]
struct Gear {
    stats: [i32; 5],
    armor: i32,
    magic_resistance: i32,
    /// Main hand, off hand, ranged.
    weapons: [Option<Weapon>; 3],
    offhand: Offhand,
}

type Values = HashMap<&'static str, f64>;
/// Sheets computed with only some bonuses, by those bonuses.
type SheetsWith = Vec<(Vec<(u8, bool, i32)>, Values)>;

impl TipTables {
    /// The Character Info sheet of the player at `level` wearing `equipment` with `buffs` (buff id,
    /// ability level).
    pub fn player_sheet(&self, level: u8, equipment: &Equipment, buffs: &[(u32, i16)]) -> PlayerSheet {
        let level = i32::from(level.max(1));
        let (gear, mut mods) = self.worn(equipment);
        for (id, ab_level) in buffs {
            if let Some(list) = self.mods.get(id) {
                mods.extend(list.iter().map(|m| (m.attr, m.percent, m.at(i32::from(*ab_level)))));
            }
        }
        let all = self.values(level, &gear, &mods);
        let none = self.values(level, &gear, &[]);
        let info = INFO_KEYS.iter().map(|k| (*k, all[k])).collect();
        // keys of one attribute set (the melee DPS keys, ...) share the sheet with only their bonuses
        let mut with_only: SheetsWith = Vec::new();
        let mut modifiers = Vec::with_capacity(MODIFIER_KEYS.len());
        for key in MODIFIER_KEYS {
            let attrs = modifier_attrs(key);
            let only: Vec<(u8, bool, i32)> = mods.iter().filter(|m| attrs.contains(&m.0)).copied().collect();
            let with = if only.is_empty() {
                &none
            } else {
                let i = match with_only.iter().position(|(m, _)| *m == only) {
                    Some(i) => i,
                    None => {
                        let values = self.values(level, &gear, &only);
                        with_only.push((only, values));
                        with_only.len() - 1
                    }
                };
                &with_only[i].1
            };
            modifiers.push((*key, modifier_delta(key, with, &none)));
        }
        PlayerSheet { info, modifiers }
    }

    /// What is worn: stats, armor, weapons and the items' own `unit_modifiers`; broken items give nothing.
    fn worn(&self, equipment: &Equipment) -> (Gear, Vec<(u8, bool, i32)>) {
        let mut gear = Gear::default();
        let mut mods = Vec::new();
        for (i, item) in equipment.slots.iter().enumerate() {
            let Some(item) = item.as_ref().filter(|_| !slot::is_body_part(i)) else { continue };
            let (durability, tp, tm) = match &item.detail {
                ItemDetail::Equip(EquipDetail { durability, temper_physical, temper_magical, .. }) => (Some(*durability), *temper_physical, *temper_magical),
                _ => (None, 0, 0),
            };
            let grade = i32::from(item.grade);
            let Some(stats) = self.items.stats(item.template_id, grade, tp, tm) else { continue };
            if durability == Some(0) && stats.max_durability > 0 {
                continue;
            }
            for (total, v) in gear.stats.iter_mut().zip(stats.stats) {
                *total += v;
            }
            gear.armor += stats.armor;
            gear.magic_resistance += stats.magic_resistance;
            mods.extend(stats.mods.iter().map(|m| (m.attr, m.percent, m.value)));
            let weapon = self.items.weapon_dps(item.template_id, grade, tp, tm).map(|(dps, mdps, hdps, scale)| Weapon {
                dps,
                mdps,
                hdps,
                scale,
                speed: f64::from(self.items.weapon_speed(item.template_id).unwrap_or(0)) / 1000.0,
            });
            match i {
                slot::MAINHAND => gear.weapons[0] = weapon,
                slot::OFFHAND => {
                    gear.weapons[1] = weapon;
                    gear.offhand = if stats.slot_type == SLOT_SHIELD { Offhand::Shield } else { Offhand::Other };
                }
                slot::RANGED => gear.weapons[2] = weapon,
                _ => {}
            }
        }
        (gear, mods)
    }

    /// Every number the sheet needs, with `mods` applied (the server's `CharStats::get`).
    fn values(&self, level: i32, gear: &Gear, mods: &Mods) -> Values {
        let formula = |k: u8, vars: &HashMap<&str, f64>| self.formulas.get(&k).map_or(0.0, |f| f.eval(vars));
        let cwb = |v: f64, a: u8| with_bonuses(v, a, mods);
        let lv = HashMap::from([("level", f64::from(level))]);
        let mut p = [0i32; 6];
        for (i, v) in p.iter_mut().enumerate() {
            let worn = if i < 5 { f64::from(gear.stats[i]) } else { 0.0 };
            *v = cwb(formula(kind::STR + i as u8, &lv) + worn, i as u8) as i32;
        }
        let [str_, dex, sta, int, spi, fai] = p.map(f64::from);
        let all = HashMap::from([("level", f64::from(level)), ("str", str_), ("dex", dex), ("sta", sta), ("int", int), ("spi", spi), ("fai", fai)]);
        let facets = (cwb(formula(kind::FACET, &lv), attr::FACETS) as i32 as f32).max(1.0);
        let accuracy = |res: f64| -> f64 {
            let mut r = (1.0 - (f64::from(facets / 10.0) - res) * f64::from(1.0f32 / facets)) * 100.0;
            r = (r + 100.0 - (r - 100.0).abs()) / 2.0;
            r = (r.abs() + r) / 2.0;
            r as f32 as f64
        };
        let rate = |res: f64| res * f64::from(1.0f32 / facets) * 100.0;
        let mut v = Values::new();
        for (key, x) in [("str", str_), ("dex", dex), ("sta", sta), ("int", int), ("spi", spi)] {
            v.insert(key, x);
        }
        // pools and regeneration
        let max_health = cwb(formula(kind::MAX_HEALTH, &all), 6);
        let max_mana = cwb(formula(kind::MAX_MANA, &all), 7);
        v.insert("max_health", f64::from(max_health as i32));
        v.insert("max_mana", f64::from(max_mana as i32));
        v.insert("max_health_raw", max_health);
        v.insert("max_mana_raw", max_mana);
        let hp_base = cwb(formula(kind::HEALTH_REGEN, &all), attr::HEALTH_REGEN);
        let mp_base = cwb(formula(kind::MANA_REGEN, &all), attr::MANA_REGEN);
        let hp_persistent = cwb(formula(kind::PERSISTENT_HEALTH_REGEN, &all), attr::PERSISTENT_HEALTH_REGEN);
        let mp_persistent = cwb(formula(kind::PERSISTENT_MANA_REGEN, &all), attr::PERSISTENT_MANA_REGEN);
        v.insert("health_regen", f64::from(hp_base as i32) + hp_persistent / 5.0);
        v.insert("mana_regen", f64::from(mp_base as i32) + mp_persistent / 5.0);
        v.insert("health_regen_attr", hp_base);
        v.insert("mana_regen_attr", mp_base);
        v.insert("persistent_health_regen", hp_persistent / 5.0);
        v.insert("persistent_mana_regen", mp_persistent / 5.0);
        v.insert("persistent_health_regen_raw", hp_persistent);
        v.insert("persistent_mana_regen_raw", mp_persistent);
        // defence
        let armor = f64::from(cwb(f64::from(formula(kind::ARMOR, &all) as i32 + gear.armor), attr::ARMOR) as i32);
        let resist = f64::from(cwb(f64::from(formula(kind::MAGIC_RESIST, &all) as i32 + gear.magic_resistance), attr::MAGIC_RESIST) as i32);
        v.insert("armor", armor);
        v.insert("magic_resist", resist);
        let armor_pct = armor / (armor + RESIST_CONSTANT).max(1.0) * 100.0;
        let resist_pct = resist / (resist + RESIST_CONSTANT).max(1.0) * 100.0;
        v.insert("armor_percentage", armor_pct);
        v.insert("magic_resist_percentage", resist_pct);
        v.insert("magic_effect_resist_percentage", resist_pct / 4.0);
        let damage_vars = HashMap::from([("armor", armor), ("magic_resist", resist)]);
        let incoming = |k: u8, mul: u8, add: u8| -> (f64, f64) {
            let res = cwb(cwb(0.0, mul), attr::INCOMING_DAMAGE_MUL);
            let val = if k == 0 { cwb(0.0, add) } else { cwb(formula(k, &damage_vars), add) };
            (res / 10.0, val.trunc())
        };
        for (key_mul, key_val, k, mul, add) in [
            ("incoming_melee_damage_mul", "incoming_melee_damage_val", kind::INCOMING_MELEE_DAMAGE_ADD, attr::INCOMING_MELEE_DAMAGE_MUL, attr::INCOMING_MELEE_DAMAGE_ADD),
            ("incoming_ranged_damage_mul", "incoming_ranged_damage_val", kind::INCOMING_RANGED_DAMAGE_ADD, attr::INCOMING_RANGED_DAMAGE_MUL, attr::INCOMING_RANGED_DAMAGE_ADD),
            ("incoming_spell_damage_mul", "incoming_spell_damage_val", kind::INCOMING_SPELL_DAMAGE_ADD, attr::INCOMING_SPELL_DAMAGE_MUL, attr::INCOMING_SPELL_DAMAGE_ADD),
            ("incoming_siege_damage_mul", "incoming_siege_damage_val", 0, attr::INCOMING_SIEGE_DAMAGE_MUL, attr::INCOMING_SIEGE_DAMAGE_ADD),
        ] {
            let (m, a) = incoming(k, mul, add);
            v.insert(key_mul, m);
            v.insert(key_val, a);
        }
        v.insert("battle_resist", f64::from(cwb(0.0, attr::BATTLE_RESIST) as i32));
        v.insert("bulls_eye", f64::from(cwb(0.0, attr::BULLS_EYE) as i32));
        v.insert("flexibility", f64::from(cwb(0.0, attr::FLEXIBILITY) as i32));
        // avoidance
        let block = match gear.offhand {
            Offhand::Shield => rate(cwb(formula(kind::BLOCK, &all), attr::BLOCK)) + cwb(0.0, attr::BLOCK_MUL) / 10.0,
            _ => 0.0,
        };
        v.insert("block_rate", block as f32 as f64);
        v.insert("block_mul", cwb(0.0, attr::BLOCK_MUL) / 10.0);
        let dodge = rate(cwb(formula(kind::DODGE, &all), attr::DODGE)) + cwb(0.0, attr::DODGE_MUL) / 10.0;
        v.insert("dodge_rate", dodge as f32 as f64);
        v.insert("dodge_mul", cwb(0.0, attr::DODGE_MUL) / 10.0);
        let parry = rate(cwb(formula(kind::MELEE_PARRY, &all), attr::MELEE_PARRY)) + cwb(0.0, attr::MELEE_PARRY_MUL) / 10.0;
        v.insert("melee_parry_rate", parry as f32 as f64);
        v.insert("melee_parry_mul", cwb(0.0, attr::MELEE_PARRY_MUL) / 10.0);
        let _ = kind::RANGED_PARRY;
        let ranged_parry = rate(cwb(0.0, attr::RANGED_PARRY)) + cwb(0.0, attr::RANGED_PARRY_MUL) / 10.0;
        v.insert("ranged_parry_rate", ranged_parry as f32 as f64);
        v.insert("ranged_parry_mul", cwb(0.0, attr::RANGED_PARRY_MUL) / 10.0);
        // accuracy and criticals
        v.insert("melee_success_rate", accuracy(cwb(formula(kind::MELEE_ANTI_MISS, &all), attr::MELEE_ANTI_MISS)));
        v.insert("ranged_success_rate", accuracy(cwb(formula(kind::RANGED_ANTI_MISS, &all), attr::RANGED_ANTI_MISS)));
        v.insert("spell_success_rate", accuracy(cwb(formula(kind::SPELL_ANTI_MISS, &all), attr::SPELL_ANTI_MISS)));
        let melee_crit_mul = cwb(0.0, attr::MELEE_CRITICAL_MUL) as f32 as f64;
        let ranged_crit_mul = cwb(0.0, attr::RANGED_CRITICAL_MUL) as f32 as f64;
        let spell_crit_mul = cwb(cwb(0.0, attr::SPELL_CRITICAL_MUL), attr::SPELL_DAMAGE_CRITICAL_MUL) as f32 as f64;
        let heal_crit_mul = cwb(0.0, attr::HEAL_CRITICAL_MUL) as f32 as f64;
        v.insert("melee_critical_rate", (rate(cwb(formula(kind::MELEE_CRITICAL, &all), attr::MELEE_CRITICAL)) + melee_crit_mul / 10.0) as f32 as f64);
        v.insert("ranged_critical_rate", (rate(cwb(formula(kind::RANGED_CRITICAL, &all), attr::RANGED_CRITICAL)) + ranged_crit_mul / 10.0) as f32 as f64);
        let spell_crit = cwb(cwb(formula(kind::SPELL_CRITICAL, &all), attr::SPELL_CRITICAL), attr::SPELL_DAMAGE_CRITICAL) as f32 as f64;
        v.insert("spell_critical_rate", (rate(spell_crit) + spell_crit_mul / 10.0) as f32 as f64);
        v.insert("heal_critical_rate", (rate(cwb(formula(kind::HEAL_CRITICAL, &all), attr::HEAL_CRITICAL)) + heal_crit_mul / 10.0) as f32 as f64);
        v.insert("melee_critical_mul", melee_crit_mul / 10.0);
        v.insert("ranged_critical_mul", ranged_crit_mul / 10.0);
        v.insert("spell_critical_mul", spell_crit_mul / 10.0);
        v.insert("heal_critical_mul", heal_crit_mul / 10.0);
        let bonus = |base: f64, a: u8| f64::from(((cwb(base, a) as f32) - 1000.0) / 10.0);
        v.insert("melee_critical_bonus", bonus(1500.0, attr::MELEE_CRITICAL_BONUS));
        v.insert("ranged_critical_bonus", bonus(1500.0, attr::RANGED_CRITICAL_BONUS));
        let spell_bonus = cwb(cwb(1500.0, attr::SPELL_CRITICAL_BONUS) as f32 as f64, attr::SPELL_DAMAGE_CRITICAL_BONUS) as f32;
        v.insert("spell_critical_bonus", f64::from((spell_bonus - 1000.0) / 10.0));
        v.insert("spell_damage_critical_bonus", cwb(0.0, attr::SPELL_DAMAGE_CRITICAL_BONUS) / 10.0);
        v.insert("spell_damage_critical_mul", cwb(0.0, attr::SPELL_DAMAGE_CRITICAL_MUL) / 10.0);
        v.insert("heal_critical_bonus", bonus(1500.0, attr::HEAL_CRITICAL_BONUS));
        // damage multipliers as percent deltas
        let pct = |a: u8| ((cwb(0.0, a) + 1000.0) / 1000.0) as f32 as f64;
        v.insert("melee_damage_mul", (pct(attr::MELEE_DAMAGE_MUL) - 1.0) * 100.0);
        v.insert("ranged_damage_mul", (pct(attr::RANGED_DAMAGE_MUL) - 1.0) * 100.0);
        v.insert("spell_damage_mul", (pct(attr::SPELL_DAMAGE_MUL) - 1.0) * 100.0);
        let _ = attr::HEAL_MUL;
        // casting, cooldown, speed
        let cast = ((cwb(0.0, attr::CASTING_TIME_MUL) + 1000.0) / 1000.0).max(0.0) as f32 as f64;
        v.insert("casting_time", cast * 100.0);
        v.insert("casting_time_mul", (cast - 1.0) * 100.0);
        v.insert("global_cooldown_mul", f64::from((100_000.0f64 / (cwb(0.0, attr::GLOBAL_COOLDOWN_MUL) + 1000.0)) as i32));
        v.insert("move_speed", MOVE_SPEED * f64::from((cwb(1000.0, attr::MOVE_SPEED_MUL) as f32) / 1000.0));
        v.insert("melee_attack_speed_mul", cwb(0.0, attr::MELEE_SPEED_MUL) / 10.0);
        v.insert("ranged_attack_speed_mul", cwb(0.0, attr::RANGED_SPEED_MUL) / 10.0);
        // attack: the weapon's DPS plus a fifth of the stat, in thousandths, like the server
        let [main, off, ranged] = gear.weapons;
        let main_dps = main.map_or(0.0, |w| w.dps) as f32;
        let melee = f64::from(cwb(f64::from(main_dps * 1000.0 + str_ as f32 / 5.0 * 1000.0), attr::MAINHAND_DPS) as f32 as i32) / 1000.0;
        let ranged_dps = f64::from(cwb(f64::from(ranged.map_or(0.0, |w| w.dps) as f32 * 1000.0 + dex as f32 / 5.0 * 1000.0), attr::RANGED_DPS) as f32 as i32) / 1000.0;
        let spell = f64::from(cwb(f64::from(main.map_or(0.0, |w| w.mdps) as f32 * 1000.0 + int as f32 / 5.0 * 1000.0), attr::SPELL_DPS) as f32 as i32) / 1000.0;
        let heal = f64::from(cwb(f64::from(main.map_or(0.0, |w| w.hdps) as f32 * 1000.0) + f64::from(spi as f32 / 5.0 * 1000.0), attr::HEAL_DPS) as i32) / 1000.0;
        v.insert("melee_dps", melee);
        v.insert("ranged_dps", ranged_dps);
        v.insert("spell_dps", spell);
        v.insert("heal_dps", heal);
        // damage range of one hit: dps * attack delay, spread by the weapon's damage scale
        // unarmed (retail capture): no spread, and the melee hit is the dps over the off hand's 3.0 s delay
        let range = |dps: f64, w: Option<Weapon>, unarmed_delay: f64| -> (f64, f64) {
            let (speed, scale) = w.map_or((unarmed_delay, 0.0), |w| (w.speed, f64::from(w.scale) / 100.0));
            let hit = dps * speed;
            ((hit * (1.0 - scale)).floor(), (hit * (1.0 + scale)).floor())
        };
        let (lo, hi) = range(melee, main, UNARMED_OFFHAND_SPEED);
        v.insert("melee_min_dps", lo);
        v.insert("melee_max_dps", hi);
        let (lo, hi) = range(ranged_dps, ranged, UNARMED_RANGED_SPEED);
        v.insert("ranged_min_dps", lo);
        v.insert("ranged_max_dps", hi);
        v.insert("mainhand_melee_speed", f64::from(main.map_or(UNARMED_MAINHAND_SPEED, |w| w.speed) as f32));
        v.insert("offhand_melee_speed", f64::from(off.map_or(if main.is_none() && gear.offhand == Offhand::Empty { UNARMED_OFFHAND_SPEED } else { 0.0 }, |w| w.speed) as f32));
        v.insert("ranged_speed", f64::from(ranged.map_or(UNARMED_RANGED_SPEED, |w| w.speed) as f32));
        v
    }
}

/// The attributes whose bonuses make up the modifier of `key`.
fn modifier_attrs(key: &str) -> &'static [u8] {
    match key {
        "str" => &[0],
        "dex" => &[1],
        "sta" => &[2],
        "int" => &[3],
        "spi" => &[4],
        "max_health" => &[6],
        "max_mana" => &[7],
        "armor" => &[attr::ARMOR],
        "magic_resist" | "magic_effect_resist_percentage" => &[attr::MAGIC_RESIST],
        "health_regen" => &[attr::HEALTH_REGEN],
        "mana_regen" => &[attr::MANA_REGEN],
        "persistent_health_regen" => &[attr::PERSISTENT_HEALTH_REGEN],
        "persistent_mana_regen" => &[attr::PERSISTENT_MANA_REGEN],
        "move_speed" => &[attr::MOVE_SPEED_MUL],
        "casting_time" | "casting_time_mul" => &[attr::CASTING_TIME_MUL],
        "global_cooldown_mul" => &[attr::GLOBAL_COOLDOWN_MUL],
        "melee_critical_rate" => &[attr::MELEE_CRITICAL],
        "melee_critical_bonus" => &[attr::MELEE_CRITICAL_BONUS],
        "melee_critical_mul" => &[attr::MELEE_CRITICAL_MUL],
        "melee_success_rate" => &[attr::MELEE_ANTI_MISS, attr::MELEE_ANTI_MISS_MUL],
        "melee_parry_rate" => &[attr::MELEE_PARRY],
        "melee_parry_mul" => &[attr::MELEE_PARRY_MUL],
        "melee_damage_mul" => &[attr::MELEE_DAMAGE_MUL],
        "melee_attack_speed_mul" => &[attr::MELEE_SPEED_MUL],
        "melee_dps" | "melee_min_dps" | "melee_max_dps" => &[attr::MAINHAND_DPS],
        "ranged_critical_rate" => &[attr::RANGED_CRITICAL],
        "ranged_critical_bonus" => &[attr::RANGED_CRITICAL_BONUS],
        "ranged_critical_mul" => &[attr::RANGED_CRITICAL_MUL],
        "ranged_success_rate" => &[attr::RANGED_ANTI_MISS, attr::RANGED_ANTI_MISS_MUL],
        "ranged_parry_rate" => &[attr::RANGED_PARRY],
        "ranged_parry_mul" => &[attr::RANGED_PARRY_MUL],
        "ranged_damage_mul" => &[attr::RANGED_DAMAGE_MUL],
        "ranged_attack_speed_mul" => &[attr::RANGED_SPEED_MUL],
        "ranged_dps" | "ranged_min_dps" | "ranged_max_dps" => &[attr::RANGED_DPS],
        "spell_critical_rate" => &[attr::SPELL_CRITICAL, attr::SPELL_DAMAGE_CRITICAL],
        "spell_critical_bonus" => &[attr::SPELL_CRITICAL_BONUS],
        "spell_critical_mul" => &[attr::SPELL_CRITICAL_MUL],
        "spell_damage_critical_bonus" => &[attr::SPELL_DAMAGE_CRITICAL_BONUS],
        "spell_damage_critical_mul" => &[attr::SPELL_DAMAGE_CRITICAL_MUL],
        "spell_success_rate" => &[attr::SPELL_ANTI_MISS, attr::SPELL_ANTI_MISS_MUL],
        "spell_damage_mul" => &[attr::SPELL_DAMAGE_MUL],
        "spell_dps" => &[attr::SPELL_DPS],
        "heal_dps" => &[attr::HEAL_DPS],
        "heal_critical_rate" => &[attr::HEAL_CRITICAL],
        "heal_critical_bonus" => &[attr::HEAL_CRITICAL_BONUS],
        "heal_critical_mul" => &[attr::HEAL_CRITICAL_MUL],
        "block_rate" => &[attr::BLOCK],
        "block_mul" => &[attr::BLOCK_MUL],
        "dodge_rate" => &[attr::DODGE],
        "dodge_mul" => &[attr::DODGE_MUL],
        "battle_resist" => &[attr::BATTLE_RESIST],
        "bulls_eye" => &[attr::BULLS_EYE],
        "flexibility" => &[attr::FLEXIBILITY],
        "incoming_melee_damage_mul" => &[attr::INCOMING_MELEE_DAMAGE_MUL, attr::INCOMING_DAMAGE_MUL],
        "incoming_ranged_damage_mul" => &[attr::INCOMING_RANGED_DAMAGE_MUL, attr::INCOMING_DAMAGE_MUL],
        "incoming_spell_damage_mul" => &[attr::INCOMING_SPELL_DAMAGE_MUL, attr::INCOMING_DAMAGE_MUL],
        "incoming_siege_damage_mul" => &[attr::INCOMING_SIEGE_DAMAGE_MUL, attr::INCOMING_DAMAGE_MUL],
        "incoming_melee_damage_val" => &[attr::INCOMING_MELEE_DAMAGE_ADD],
        "incoming_ranged_damage_val" => &[attr::INCOMING_RANGED_DAMAGE_ADD],
        "incoming_spell_damage_val" => &[attr::INCOMING_SPELL_DAMAGE_ADD],
        "incoming_siege_damage_val" => &[attr::INCOMING_SIEGE_DAMAGE_ADD],
        _ => &[],
    }
}

/// The modifier value of `key` from the sheet with only its attributes' bonuses (`with`) and without any (`none`).
fn modifier_delta(key: &str, with: &Values, none: &Values) -> f64 {
    // regeneration: the attribute's own effect (the oracle's `health_regen` modifier stays 0 with persistent-regen buffs)
    let source = match key {
        "health_regen" => "health_regen_attr",
        "mana_regen" => "mana_regen_attr",
        "max_health" => "max_health_raw",
        "max_mana" => "max_mana_raw",
        "persistent_health_regen" => "persistent_health_regen_raw",
        "persistent_mana_regen" => "persistent_mana_regen_raw",
        "magic_effect_resist_percentage" => "magic_resist",
        other => other,
    };
    let delta = with.get(source).copied().unwrap_or(0.0) - none.get(source).copied().unwrap_or(0.0);
    // lower is better: the original shows these negated (`-0` in the oracle with no modifiers)
    if key.starts_with("incoming_") || key == "casting_time" || key == "casting_time_mul" {
        -delta
    } else {
        delta
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aa_net::model::Item;

    fn tables() -> Option<TipTables> {
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let conn = rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
        TipTables::load(&conn, &HashMap::new()).ok()
    }

    /// The oracle character's gear: Civilian sword, shield, bow, lute, shirt, pants, shoes (aa-net slot indices).
    fn oracle_gear() -> Equipment {
        let mut e = Equipment::default();
        for (slot, id) in [(15, 5569), (16, 6152), (17, 6127), (18, 6177), (2, 23387), (4, 23388), (6, 23390)] {
            e.slots[slot] = Some(Item { template_id: id, ..Item::default() });
        }
        e
    }

    fn get(v: &[(&'static str, f64)], key: &str) -> f64 {
        v.iter().find(|(k, _)| *k == key).unwrap_or_else(|| panic!("no key {key}")).1
    }

    fn close(a: f64, b: f64, what: &str) {
        assert!((a - b).abs() <= 0.01 + b.abs() * 1e-5, "{what}: {a} vs oracle {b}");
    }

    /// The retail client's `UnitInfo('player')` at level 1 and 50 (`oracle-stats-L1.json`, `-L50.json`: Civilian gear,
    /// buffs 8000012, 8000011, 8226 (shield: armor and magic resistance +200, health and mana +350), 6495).
    #[test]
    fn matches_the_retail_oracle() {
        let Some(t) = tables() else { return };
        let buffs = [(8_000_012, 0), (8_000_011, 0), (8226, 0), (6495, 0)];
        let l1 = t.player_sheet(1, &oracle_gear(), &buffs);
        for (key, want) in [
            ("str", 10.0), ("dex", 10.0), ("sta", 10.0), ("int", 10.0), ("spi", 10.0), ("armor", 269.0), ("magic_resist", 377.0),
            ("armor_percentage", 4.83031034), ("magic_resist_percentage", 6.64083147), ("magic_effect_resist_percentage", 1.66020787),
            ("block_rate", 2.85714269), ("dodge_rate", 3.80952358), ("melee_parry_rate", 2.14285707), ("melee_critical_rate", 4.76190472),
            ("heal_critical_rate", 4.76190472), ("melee_critical_bonus", 50.0), ("melee_success_rate", 100.0), ("melee_dps", 19.423),
            ("ranged_dps", 16.81), ("spell_dps", 2.0), ("heal_dps", 2.0), ("melee_min_dps", 27.0), ("melee_max_dps", 34.0), ("ranged_min_dps", 15.0),
            ("ranged_max_dps", 18.0), ("mainhand_melee_speed", 1.6), ("offhand_melee_speed", 0.0), ("ranged_speed", 1.0), ("incoming_melee_damage_val", -23.0),
            ("incoming_ranged_damage_val", -23.0), ("incoming_spell_damage_val", -31.0), ("casting_time", 100.0), ("global_cooldown_mul", 100.0),
            ("move_speed", 5.4), ("health_regen", 52.0), ("mana_regen", 17.0),
        ] {
            close(get(&l1.info, key), want, &format!("L1 {key}"));
        }
        // the level 1 oracle's modifiers: the shield buff's four
        for (key, want) in [("armor", 200.0), ("magic_resist", 200.0), ("max_health", 350.0), ("max_mana", 350.0), ("str", 0.0), ("melee_dps", 0.0)] {
            close(get(&l1.modifiers, key), want, &format!("L1 modifier {key}"));
        }
        let l50 = t.player_sheet(50, &oracle_gear(), &buffs);
        for (key, want) in [
            ("str", 108.0), ("armor", 269.0), ("magic_resist", 475.0), ("magic_resist_percentage", 8.22510815), ("block_rate", 1.97151232),
            ("dodge_rate", 2.62868309), ("melee_parry_rate", 1.47863424), ("melee_critical_rate", 3.28585386), ("melee_success_rate", 93.7787323),
            ("melee_dps", 39.023), ("ranged_dps", 36.41), ("spell_dps", 21.6), ("melee_min_dps", 56.0), ("melee_max_dps", 68.0),
            ("ranged_min_dps", 32.0), ("ranged_max_dps", 40.0), ("incoming_spell_damage_val", -37.0), ("mana_regen", 36.0),
        ] {
            close(get(&l50.info, key), want, &format!("L50 {key}"));
        }
    }

    /// The retail client at level 10 with nothing in the hands (Civilian Lute in the instrument slot, shirt, pants,
    /// shoes; buffs 8000012, 8000011, 5754): unarmed attack speeds 1.5 / 3.0 / 1.5 and no damage spread.
    #[test]
    fn unarmed_matches_the_retail_oracle() {
        let Some(t) = tables() else { return };
        let mut e = Equipment::default();
        for (slot, id) in [(18, 6177), (2, 23387), (4, 23388), (6, 23390)] {
            e.slots[slot] = Some(Item { template_id: id, ..Item::default() });
        }
        let s = t.player_sheet(10, &e, &[(8_000_012, 0), (8_000_011, 0), (5754, 0)]);
        for (key, want) in [
            ("melee_dps", 5.6), ("ranged_dps", 5.6), ("melee_min_dps", 16.0), ("melee_max_dps", 16.0), ("ranged_min_dps", 8.0),
            ("ranged_max_dps", 8.0), ("mainhand_melee_speed", 1.5), ("offhand_melee_speed", 3.0), ("ranged_speed", 1.5), ("move_speed", 5.4),
        ] {
            close(get(&s.info, key), want, &format!("unarmed L10 {key}"));
        }
    }

    /// Level 55 with Dragon's Energy 4698 (+10 % health and mana, persistent regeneration) and Warmth of the Campfire 5872.
    #[test]
    fn percent_bonuses_are_part_of_the_modifier() {
        let Some(t) = tables() else { return };
        let buffs = [(8_000_012, 0), (8_000_011, 0), (8226, 0), (4698, 0), (5872, 0)];
        let s = t.player_sheet(55, &oracle_gear(), &buffs);
        close(get(&s.modifiers, "max_health"), 1409.6, "max_health modifier");
        close(get(&s.modifiers, "max_mana"), 1134.6, "max_mana modifier");
        close(get(&s.modifiers, "persistent_health_regen"), 65.0, "persistent_health_regen modifier (the oracle's 70 has a hidden +5)");
        close(get(&s.info, "persistent_mana_regen"), 9.4, "persistent_mana_regen");
    }

    /// Buff 5754 (Scent of Saffron): intelligence +3.
    #[test]
    fn a_flat_stat_buff_raises_the_stat_and_its_modifier() {
        let Some(t) = tables() else { return };
        let plain = t.player_sheet(1, &Equipment::default(), &[]);
        let buffed = t.player_sheet(1, &Equipment::default(), &[(5754, 0)]);
        assert_eq!(get(&buffed.info, "int") - get(&plain.info, "int"), 3.0);
        assert_eq!(get(&buffed.modifiers, "int"), 3.0);
        assert_eq!(get(&plain.modifiers, "int"), 0.0);
        assert_eq!(get(&buffed.info, "str"), get(&plain.info, "str"));
    }

    #[test]
    fn every_key_of_the_original_is_present() {
        let Some(t) = tables() else { return };
        let s = t.player_sheet(1, &Equipment::default(), &[]);
        assert_eq!((s.info.len(), s.modifiers.len()), (58, 70));
    }
}
