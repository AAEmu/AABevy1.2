//! What the weapon and armour tooltips show: attack (DPS), defence, stat bonuses, durability, set bonuses.
//!
//! The tooltip scripts (`baselib/tooltip/tooltip.lua`) read these fields of the item's info table: `DPS`,
//! `minDamage`, `maxDamage`, `attackDelay`, `magicDps`, `healDps`, `armor`, `magicResistance`, `durability`,
//! `maxDurability`, `slotType`, `slotTypeNum`, `modifier` (`{name, type, value}`) and `equipSetInfo`. The client works them
//! out of the item's template, grade and detail block; the formulas are the game database's
//! (`holdables.formula_*`, `wearable_formulas`, `item_grades`, `item_configs`), the same the server
//! (`aaemu-game`, `models/item.rs`) evaluates for what a character wears, in the same single precision.

use std::collections::HashMap;

use rusqlite::Connection;

use super::data::{any_flag, any_int};
use super::formula::Formula;

/// The server's `0.01f` as the C# compiler rounds it.
#[allow(clippy::excessive_precision)] // the server's literal, kept as is
pub const F_001: f32 = 0.009_999_999_8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Weapon,
    Armor,
    Accessory,
}

/// A weapon, armour or accessory template (`item_weapons` / `item_armors` / `item_accessories`).
#[derive(Clone, Debug)]
pub struct EquipFx {
    pub class: Class,
    /// `items.level`: the item level the formulas take (not `level_requirement`).
    pub level: i32,
    /// `equip_item_attr_modifiers.id` of the stat weights, 0 for none.
    pub mod_set: u32,
    /// Weapons: `holdables.id`.
    pub holdable: u32,
    /// Armour and accessories: `slot_type_id`; weapons take it from the holdable.
    pub slot_type: u32,
    /// Armour and accessories: `type_id` (`wearable_kinds.armor_type_id`).
    pub armor_type: u32,
    pub durability_multiplier: i32,
    pub repairable: bool,
    pub use_as_stat: bool,
    /// `equip_item_sets.id`, 0 for none.
    pub set: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Holdable {
    /// Attack speed in ms.
    pub speed: i32,
    /// How much of the damage is piercing, slashing, blunt.
    pub extra: [i32; 3],
    pub damage_scale: i32,
    pub slot_type: u32,
    pub dps: Option<Formula>,
    pub mdps: Option<Formula>,
    pub hdps: Option<Formula>,
    pub armor: Option<Formula>,
    pub durability_ratio: f32,
    pub stat_multiplier: i32,
}

#[derive(Clone, Debug, Default)]
pub struct Kind {
    pub armor_ratio: i32,
    pub magic_resistance_ratio: i32,
    pub extra: [i32; 3],
    pub durability_ratio: f32,
}

/// One `item_grades` row.
#[derive(Clone, Debug, Default)]
pub struct Grade {
    pub dps: f32,
    pub armor: f32,
    pub magic_dps: f32,
    pub wearable_armor: f32,
    pub wearable_magic_resistance: f32,
    pub durability: f32,
    pub stat_multiplier: i32,
}

#[derive(Clone, Copy, Debug, Default)]
struct Config {
    durability_const: f32,
    holdable_durability_const: f32,
    item_stat: i32,
    holdable_stat: i32,
    wearable_stat: i32,
    stat_value: i32,
}

/// An equipment set: its name, the pieces that count and the bonuses.
#[derive(Clone, Debug, Default)]
pub struct EquipSet {
    pub name: String,
    pub items: Vec<u32>,
    /// `(pieces needed, buff id)`
    pub bonuses: Vec<(i32, u32)>,
}

/// An explicit bonus of an item (`unit_modifiers` of owner type `Item`).
#[derive(Clone, Copy, Debug)]
pub struct ItemMod {
    pub attr: u8,
    pub percent: bool,
    /// `unit_modifier_type_id` as stored (the tooltip's `type`).
    pub kind: i32,
    pub value: i32,
}

/// The stats of an item as the tooltip shows them.
#[derive(Clone, Debug, Default)]
pub struct ItemStats {
    pub class: Option<Class>,
    pub slot_type: u32,
    /// Attack in damage per second, with the damage range of one hit and the attack delay in seconds.
    pub dps: f64,
    pub min_damage: f64,
    pub max_damage: f64,
    pub attack_delay: f64,
    pub magic_dps: f64,
    pub heal_dps: f64,
    pub armor: i32,
    pub magic_resistance: i32,
    pub max_durability: i32,
    pub repairable: bool,
    pub use_as_stat: bool,
    /// Piercing, slashing, blunt (weapons: how it hits; armour: how it resists).
    pub extra_damage: [i32; 3],
    /// The primary stats the item gives (str, dex, sta, int, spi).
    pub stats: [i32; 5],
    /// The other bonuses (`unit_modifiers`).
    pub mods: Vec<ItemMod>,
    /// What the template's own armor and magic resistance modifiers (attributes 8 and 64) add on top of `armor` and
    /// `magic_resistance`: the tooltip shows the sum, the character sheet counts them as modifiers.
    pub armor_modifier: i32,
    pub magic_resistance_modifier: i32,
    pub set: u32,
}

/// The slot type's key in `locale.common.equipSlotType` (`EquipmentItemSlotType`).
pub fn slot_type_name(id: u32) -> Option<&'static str> {
    Some(match id {
        1 => "head",
        2 => "neck",
        3 => "chest",
        4 => "waist",
        5 => "legs",
        6 => "hands",
        7 => "feet",
        8 => "arms",
        9 => "back",
        10 => "ear",
        11 => "finger",
        12 => "underweartop",
        13 => "underpants",
        14 => "mainhand",
        16 => "2handed",
        17 => "1handed",
        18 => "ranged",
        20 => "shield",
        21 => "instrument",
        30 => "backpack",
        31 => "cosplay",
        _ => return None,
    })
}

#[derive(Default)]
pub struct ItemTables {
    equip: HashMap<u32, EquipFx>,
    holdables: HashMap<u32, Holdable>,
    kinds: HashMap<u32, Kind>,
    coverage: HashMap<u32, i32>,
    /// `wearable_formulas` kind 0: base armour, kind 1: base magic resistance.
    wearable: [Option<Formula>; 2],
    grades: HashMap<i32, Grade>,
    weights: HashMap<u32, [i32; 5]>,
    config: Config,
    sets: HashMap<u32, EquipSet>,
    mods: HashMap<u32, Vec<ItemMod>>,
    /// `items.bind_id`
    bind: HashMap<u32, i64>,
    /// Gem, dye, socket, proc and look tables of the item tooltips.
    pub extra: super::tips::extra::Extra,
}

fn formula(text: Option<String>) -> Option<Formula> {
    let text = text?;
    let text = text.trim();
    if text.is_empty() || text == "0" {
        return None;
    }
    Formula::parse(text).ok()
}

fn int(r: &rusqlite::Row, i: usize) -> rusqlite::Result<i64> {
    any_int(r, i)
}

impl ItemTables {
    pub fn load(conn: &Connection, set_names: &HashMap<u32, String>) -> rusqlite::Result<ItemTables> {
        let mut t = ItemTables::default();
        let mut level: HashMap<u32, i32> = HashMap::new();
        {
            let mut q = conn.prepare("SELECT id, level, bind_id FROM items")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let id = int(r, 0)? as u32;
                level.insert(id, int(r, 1)? as i32);
                t.bind.insert(id, int(r, 2)?);
            }
        }
        {
            let mut q = conn.prepare("SELECT item_stat_const, holdable_stat_const, wearable_stat_const, stat_value_const, durability_const, holdable_durability_const FROM item_configs LIMIT 1")?;
            let mut rows = q.query([])?;
            if let Some(r) = rows.next()? {
                t.config = Config {
                    item_stat: int(r, 0)? as i32,
                    holdable_stat: int(r, 1)? as i32,
                    wearable_stat: int(r, 2)? as i32,
                    stat_value: int(r, 3)? as i32,
                    durability_const: r.get::<_, Option<f64>>(4)?.unwrap_or(0.0) as f32,
                    holdable_durability_const: r.get::<_, Option<f64>>(5)?.unwrap_or(0.0) as f32,
                };
            }
        }
        {
            let mut q = conn.prepare(
                "SELECT id, var_holdable_dps, var_holdable_armor, var_holdable_magic_dps, var_wearable_armor, var_wearable_magic_resistance, durability_value, stat_multiplier FROM item_grades",
            )?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let f = |i: usize| -> rusqlite::Result<f32> { Ok(r.get::<_, Option<f64>>(i)?.unwrap_or(0.0) as f32) };
                t.grades.insert(
                    int(r, 0)? as i32,
                    Grade {
                        dps: f(1)?,
                        armor: f(2)?,
                        magic_dps: f(3)?,
                        wearable_armor: f(4)?,
                        wearable_magic_resistance: f(5)?,
                        durability: f(6)?,
                        stat_multiplier: int(r, 7)? as i32,
                    },
                );
            }
        }
        {
            let mut q = conn.prepare("SELECT id, str_weight, dex_weight, sta_weight, int_weight, spi_weight FROM equip_item_attr_modifiers")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                t.weights.insert(int(r, 0)? as u32, [int(r, 1)? as i32, int(r, 2)? as i32, int(r, 3)? as i32, int(r, 4)? as i32, int(r, 5)? as i32]);
            }
        }
        {
            let mut q = conn.prepare("SELECT slot_type_id, coverage FROM wearable_slots")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                t.coverage.insert(int(r, 0)? as u32, int(r, 1)? as i32);
            }
        }
        {
            let mut q = conn.prepare("SELECT kind_id, formula FROM wearable_formulas")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let kind = int(r, 0)? as usize;
                if kind < 2 {
                    t.wearable[kind] = formula(r.get::<_, Option<String>>(1)?);
                }
            }
        }
        {
            let mut q = conn.prepare("SELECT armor_type_id, armor_ratio, magic_resistance_ratio, extra_damage_pierce, extra_damage_slash, extra_damage_blunt, durability_ratio FROM wearable_kinds")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                t.kinds.insert(
                    int(r, 0)? as u32,
                    Kind {
                        armor_ratio: int(r, 1)? as i32,
                        magic_resistance_ratio: int(r, 2)? as i32,
                        extra: [int(r, 3)? as i32, int(r, 4)? as i32, int(r, 5)? as i32],
                        durability_ratio: r.get::<_, Option<f64>>(6)?.unwrap_or(1.0) as f32,
                    },
                );
            }
        }
        {
            let mut q = conn.prepare(
                "SELECT id, speed, extra_damage_pierce_factor, extra_damage_slash_factor, extra_damage_blunt_factor, damage_scale, slot_type_id, formula_dps, formula_mdps, formula_hdps, formula_armor, durability_ratio, stat_multiplier FROM holdables",
            )?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                t.holdables.insert(
                    int(r, 0)? as u32,
                    Holdable {
                        speed: int(r, 1)? as i32,
                        extra: [int(r, 2)? as i32, int(r, 3)? as i32, int(r, 4)? as i32],
                        damage_scale: int(r, 5)? as i32,
                        slot_type: int(r, 6)? as u32,
                        dps: formula(r.get(7)?),
                        mdps: formula(r.get(8)?),
                        hdps: formula(r.get(9)?),
                        armor: formula(r.get(10)?),
                        durability_ratio: r.get::<_, Option<f64>>(11)?.unwrap_or(1.0) as f32,
                        stat_multiplier: int(r, 12)? as i32,
                    },
                );
            }
        }
        {
            let mut q = conn.prepare("SELECT item_id, holdable_id, mod_set_id, eiset_id, durability_multiplier, repairable, useAsStat FROM item_weapons")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let id = int(r, 0)? as u32;
                t.equip.insert(
                    id,
                    EquipFx {
                        class: Class::Weapon,
                        level: level.get(&id).copied().unwrap_or(0),
                        mod_set: int(r, 2)?.max(0) as u32,
                        holdable: int(r, 1)?.max(0) as u32,
                        slot_type: 0,
                        armor_type: 0,
                        durability_multiplier: int(r, 4)? as i32,
                        repairable: any_flag(r, 5)?,
                        use_as_stat: any_flag(r, 6).unwrap_or(true),
                        set: int(r, 3)?.max(0) as u32,
                    },
                );
            }
        }
        for (sql, class) in [
            ("SELECT item_id, slot_type_id, type_id, mod_set_id, eiset_id, durability_multiplier, repairable, useAsStat FROM item_armors", Class::Armor),
            ("SELECT item_id, slot_type_id, type_id, mod_set_id, eiset_id, durability_multiplier, repairable, 1 FROM item_accessories", Class::Accessory),
        ] {
            let mut q = conn.prepare(sql)?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let id = int(r, 0)? as u32;
                t.equip.insert(
                    id,
                    EquipFx {
                        class,
                        level: level.get(&id).copied().unwrap_or(0),
                        mod_set: int(r, 3)?.max(0) as u32,
                        holdable: 0,
                        slot_type: int(r, 1)?.max(0) as u32,
                        armor_type: int(r, 2)?.max(0) as u32,
                        durability_multiplier: int(r, 5)? as i32,
                        repairable: any_flag(r, 6)?,
                        use_as_stat: any_flag(r, 7).unwrap_or(true),
                        set: int(r, 4)?.max(0) as u32,
                    },
                );
            }
        }
        {
            // sets: their pieces are the items that name them
            let mut q = conn.prepare("SELECT id, equip_item_set_id, num_pieces, buff_id FROM equip_item_set_bonuses ORDER BY num_pieces")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let set = int(r, 1)? as u32;
                t.sets.entry(set).or_default().bonuses.push((int(r, 2)? as i32, int(r, 3)?.max(0) as u32));
            }
            let mut ids: Vec<u32> = t.equip.keys().copied().collect();
            ids.sort_unstable();
            for id in ids {
                let set = t.equip[&id].set;
                if set != 0 {
                    t.sets.entry(set).or_default().items.push(id);
                }
            }
            for (id, set) in &mut t.sets {
                set.name = set_names.get(id).cloned().unwrap_or_default();
            }
        }
        {
            let mut q = conn.prepare("SELECT owner_id, unit_attribute_id, unit_modifier_type_id, value FROM unit_modifiers WHERE owner_type = 'Item'")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let id = int(r, 0)? as u32;
                if t.equip.contains_key(&id) {
                    t.mods.entry(id).or_default().push(ItemMod { attr: int(r, 1)? as u8, percent: int(r, 2)? == 1, kind: int(r, 2)? as i32, value: int(r, 3)? as i32 });
                }
            }
        }
        t.extra = match super::tips::extra::Extra::load(conn) {
            Ok(extra) => extra,
            Err(e) => {
                bevy::log::warn!("hud: item tooltip tables: {e}");
                Default::default()
            }
        };
        Ok(t)
    }

    /// The explicit bonuses of an equipment template (`unit_modifiers` owned by the item).
    pub fn mods(&self, template: u32) -> &[ItemMod] {
        self.mods.get(&template).map_or(&[], Vec::as_slice)
    }

    /// `items.level` of an equipment template: the item level the formulas and the socket rule take.
    pub fn item_level(&self, template: u32) -> Option<i32> {
        self.equip.get(&template).map(|e| e.level)
    }

    /// The tooltip's `soul_bind` of an item that is not bound yet: when it binds (`items.bind_id`).
    pub fn soul_bind(&self, template: u32) -> Option<&'static str> {
        match self.bind.get(&template)? {
            2 => Some("soulbound_pickup"),
            5 => Some("soulbound_pickup_pack"),
            3 => Some("soulbound_equip"),
            4 => Some("soulbound_unpack"),
            _ => None,
        }
    }

    /// `items.bind_id` of a template.
    pub fn bind_id(&self, template: u32) -> Option<i64> {
        self.bind.get(&template).copied()
    }

    pub fn set(&self, id: u32) -> Option<&EquipSet> {
        self.sets.get(&id)
    }

    /// The slot type an equipment template goes in (weapons: the holdable's).
    pub fn slot_type(&self, template: u32) -> u32 {
        match self.equip.get(&template) {
            Some(e) if e.class == Class::Weapon => self.holdables.get(&e.holdable).map_or(0, |h| h.slot_type),
            Some(e) => e.slot_type,
            None => 0,
        }
    }

    /// One primary stat of the item (0 str, 1 dex, 2 sta, 3 int, 4 spi): the server's `Item.Str` .. `Item.Spi`.
    fn stat(&self, e: &EquipFx, grade: &Grade, which: usize) -> i32 {
        if e.mod_set == 0 {
            return 0;
        }
        let Some(w) = self.weights.get(&e.mod_set) else { return 0 };
        let count = w.iter().filter(|v| **v > 0).count() as f32;
        let all: i32 = w.iter().sum();
        if all == 0 || self.config.stat_value == 0 {
            return 0;
        }
        let c = &self.config;
        let mut m = 1f32;
        if count == 1.0 {
            m = 3.0;
        }
        if count == 2.0 {
            m = 1.5;
        }
        let exponent = 1f32 / (c.stat_value as f32 * F_001);
        m = f64::from(m).powf(f64::from(exponent)) as f32;
        let weight = w[which] as f32;
        let multiplier = grade.stat_multiplier as f32;
        let stat = match e.class {
            Class::Weapon => {
                let Some(h) = self.holdables.get(&e.holdable) else { return 0 };
                let temp = c.item_stat as f32 * F_001 * e.level as f32 * m * c.holdable_stat as f32 * F_001;
                let temp2 = count * temp * weight * 1f32 / all as f32 * multiplier * F_001 + 0.5;
                ((temp2 as i32).wrapping_mul(h.stat_multiplier)) as f32 * F_001 + 0.5
            }
            _ => {
                let coverage = self.coverage.get(&e.slot_type).copied().unwrap_or(0);
                let temp = c.item_stat as f32 * F_001 * e.level as f32 * m * c.wearable_stat as f32 * F_001 * coverage as f32 * 0.01;
                count * temp * weight * 1f32 / all as f32 * multiplier * F_001 + 0.5
            }
        };
        stat as i32
    }

    fn weapon_formula(f: &Option<Formula>, level: i32, grade: f32, temper: u16) -> f64 {
        let Some(f) = f else { return 0.0 };
        let vars = HashMap::from([("item_level", f64::from(level)), ("item_grade", f64::from(grade))]);
        let mut res = f.eval(&vars);
        if temper > 100 {
            res *= f64::from(f32::from(temper) / 100.0f32);
        }
        res
    }

    fn base_armor(&self, e: &EquipFx, grade: &Grade, armor: bool, temper: u16) -> i32 {
        let Some(kind) = self.kinds.get(&e.armor_type) else { return 0 };
        let (formula, grade_value, ratio) = if armor {
            (&self.wearable[0], grade.wearable_armor, kind.armor_ratio)
        } else {
            (&self.wearable[1], grade.wearable_magic_resistance, kind.magic_resistance_ratio)
        };
        let Some(formula) = formula else { return 0 };
        let vars = HashMap::from([("item_level", f64::from(e.level)), ("item_grade", f64::from(grade_value))]);
        let mut res = formula.eval(&vars);
        res = res * f64::from(ratio) * f64::from(F_001);
        // accessories ignore the tempers
        if e.class == Class::Armor && armor && temper > 100 {
            res *= f64::from(f32::from(temper) / 100.0f32);
        }
        let coverage = self.coverage.get(&e.slot_type).copied().unwrap_or(0);
        (res * f64::from(coverage) * f64::from(F_001)) as i32
    }

    /// `EquipItem.MaxDurability` (0 for an item that does not wear out).
    pub fn max_durability(&self, e: &EquipFx, grade: &Grade) -> i32 {
        let c = &self.config;
        let (first, ratio) = match e.class {
            Class::Weapon => {
                let Some(h) = self.holdables.get(&e.holdable) else { return 0 };
                (((c.holdable_durability_const * 100.0 + 0.5f32) as i32), h.durability_ratio)
            }
            _ => {
                let Some(k) = self.kinds.get(&e.armor_type) else { return 0 };
                let coverage = self.coverage.get(&e.slot_type).copied().unwrap_or(0);
                ((((coverage * 100) as f32 + 0.5f32) as i32), k.durability_ratio)
            }
        };
        let ratio = (ratio * 1000.0 + 0.5f32) as i32;
        let inner = (first.wrapping_mul(ratio) as f32 * grade.durability * 1000.0 * 0.000_000_01f32) as i32;
        let durability = inner as f32 * c.durability_const;
        let v = f64::from(durability * e.durability_multiplier as f32 * F_001).round_ties_even() as f32;
        i32::from(v as u8)
    }

    /// Durability metadata without calculating damage, modifiers or allocating tooltip stats.
    pub fn durability(&self, template: u32, grade: u8) -> Option<(u8, bool)> {
        let e = self.equip.get(&template)?;
        let g = self.grades.get(&i32::from(grade)).or_else(|| self.grades.get(&0))?;
        let maximum = self.max_durability(e, g) as u8;
        (maximum != 0).then_some((maximum, e.repairable))
    }

    /// The stats of an item of `template` at `grade` (`item_grades.id`) with the temper values of its detail block.
    pub fn stats(&self, template: u32, grade: i32, temper_physical: u16, temper_magical: u16) -> Option<ItemStats> {
        let e = self.equip.get(&template)?;
        let g = self.grades.get(&grade).or_else(|| self.grades.get(&0))?;
        let mut s = ItemStats {
            class: Some(e.class),
            slot_type: self.slot_type(template),
            repairable: e.repairable,
            use_as_stat: e.use_as_stat,
            set: e.set,
            max_durability: self.max_durability(e, g),
            ..ItemStats::default()
        };
        for (i, v) in s.stats.iter_mut().enumerate() {
            *v = self.stat(e, g, i);
        }
        if let Some(m) = self.mods.get(&template) {
            s.mods = m.clone();
            let first = |attr: u8| m.iter().find(|m| m.attr == attr).map_or(0, |m| m.value);
            s.armor_modifier = first(8);
            s.magic_resistance_modifier = first(64);
        }
        match e.class {
            Class::Weapon => {
                let h = self.holdables.get(&e.holdable)?;
                s.dps = Self::weapon_formula(&h.dps, e.level, g.dps, temper_physical);
                s.magic_dps = Self::weapon_formula(&h.mdps, e.level, g.magic_dps, temper_magical);
                s.heal_dps = Self::weapon_formula(&h.hdps, e.level, g.magic_dps, temper_magical);
                s.armor = Self::weapon_formula(&h.armor, e.level, g.armor, temper_physical) as i32;
                s.extra_damage = h.extra;
                if h.speed > 0 {
                    s.attack_delay = f64::from(h.speed) / 1000.0;
                    // one hit: the damage over the attack delay, spread by the weapon's damage scale
                    let hit = s.dps * s.attack_delay;
                    let spread = f64::from(h.damage_scale) * 0.01;
                    s.min_damage = hit * (1.0 - spread);
                    s.max_damage = hit * (1.0 + spread);
                }
            }
            _ => {
                s.armor = self.base_armor(e, g, true, temper_physical);
                s.magic_resistance = self.base_armor(e, g, false, temper_magical);
                if let Some(k) = self.kinds.get(&e.armor_type) {
                    s.extra_damage = k.extra;
                }
            }
        }
        Some(s)
    }

    /// A weapon's attack delay in milliseconds (`holdables.speed`).
    pub fn weapon_speed(&self, template: u32) -> Option<i32> {
        let e = self.equip.get(&template).filter(|e| e.class == Class::Weapon)?;
        self.holdables.get(&e.holdable).map(|h| h.speed)
    }

    /// A weapon's DPS values for the player's attack: `(dps, magic dps, heal dps)` of the template at `grade`.
    pub fn weapon_dps(&self, template: u32, grade: i32, temper_physical: u16, temper_magical: u16) -> Option<(f64, f64, f64, i32)> {
        let e = self.equip.get(&template).filter(|e| e.class == Class::Weapon)?;
        let g = self.grades.get(&grade).or_else(|| self.grades.get(&0))?;
        let h = self.holdables.get(&e.holdable)?;
        Some((
            Self::weapon_formula(&h.dps, e.level, g.dps, temper_physical),
            Self::weapon_formula(&h.mdps, e.level, g.magic_dps, temper_magical),
            Self::weapon_formula(&h.hdps, e.level, g.magic_dps, temper_magical),
            h.damage_scale,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tables() -> Option<ItemTables> {
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
        ItemTables::load(&conn, &HashMap::new()).ok()
    }

    fn ids(sql: &str) -> Vec<u32> {
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let mut q = conn.prepare(sql).unwrap();
        q.query_map([], |r| r.get::<_, i64>(0)).unwrap().map(|r| r.unwrap() as u32).filter(|i| *i != 0).collect()
    }

    /// The gear a new character of each of the six starter abilities gets: every weapon attacks, every piece of
    /// clothing defends and wears out.
    #[test]
    fn starter_gear_has_stats() {
        let Some(t) = tables() else { return };
        let weapons = ids(
            "SELECT w.mainhand_id FROM character_equip_packs c JOIN equip_pack_weapons w ON w.id = c.newbie_weapon_pack_id WHERE c.ability_id IN (1, 5, 6, 7, 8, 10) \
             UNION SELECT w.ranged_id FROM character_equip_packs c JOIN equip_pack_weapons w ON w.id = c.newbie_weapon_pack_id WHERE c.ability_id IN (1, 5, 6, 7, 8, 10)",
        );
        assert!(weapons.len() >= 4, "{weapons:?}");
        for id in weapons {
            let s = t.stats(id, 0, 0, 0).unwrap_or_else(|| panic!("weapon {id} has no stats"));
            assert_eq!(s.class, Some(Class::Weapon));
            assert!(s.dps > 1.0 && s.min_damage > 0.0 && s.max_damage > s.min_damage && s.attack_delay >= 1.0, "{id}: {s:?}");
            assert!(slot_type_name(s.slot_type).is_some(), "{id}: slot type {}", s.slot_type);
            assert!(s.max_durability > 0, "{id}: {s:?}");
        }
        let cloth = ids(
            "SELECT headgear_id FROM equip_pack_cloths WHERE id = 11 UNION SELECT shirt_id FROM equip_pack_cloths WHERE id = 11 UNION SELECT pants_id FROM equip_pack_cloths WHERE id = 11 \
             UNION SELECT glove_id FROM equip_pack_cloths WHERE id = 11 UNION SELECT shoes_id FROM equip_pack_cloths WHERE id = 11",
        );
        assert!(!cloth.is_empty());
        for id in cloth {
            let s = t.stats(id, 0, 0, 0).unwrap_or_else(|| panic!("armor {id} has no stats"));
            assert!(s.armor > 0 && s.magic_resistance > 0, "{id}: {s:?}");
            assert!(s.max_durability > 0, "{id}: {s:?}");
            assert!(slot_type_name(s.slot_type).is_some(), "{id}: slot type {}", s.slot_type);
        }
    }

    /// A better grade attacks harder and wears out later; a temper adds to it.
    #[test]
    fn grades_and_tempers_raise_the_numbers() {
        let Some(t) = tables() else { return };
        let id = ids("SELECT mainhand_id FROM equip_pack_weapons WHERE id = 242")[0];
        let basic = t.stats(id, 0, 0, 0).unwrap();
        let rare = t.stats(id, 3, 0, 0).unwrap();
        assert!(rare.dps > basic.dps * 1.09 && rare.dps < basic.dps * 1.11, "{} {}", basic.dps, rare.dps);
        assert!(rare.max_durability > basic.max_durability);
        let tempered = t.stats(id, 0, 150, 0).unwrap();
        assert!((tempered.dps / basic.dps - 1.5).abs() < 1e-6);
    }
}
