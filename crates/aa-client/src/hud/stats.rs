//! Numbers the server does not send but the client works out from the game database: the maximum
//! HP and MP of the player and of NPCs.
//!
//! The server only reports current HP / MP (the unit state and `SCUnitPoints`). The maximum is
//! `unit_formulas` of kind `MaxHealth` / `MaxMana` over the unit's level and primary stats, then the
//! flat and percent bonuses of its buffs (`unit_modifiers` of owner type `Buff`), the way the
//! server's own `CharStats` and `NpcStatCalc` do:
//!
//! * a player: each primary stat (`Str` .. `Fai`) is its level formula + the worn gear's share of
//!   it, with the stat's buff bonuses; `max_hp = formula(level, str, dex, sta, int, spi, fai)` with
//!   the `MaxHealth` bonuses; the gear's stats come from the item's weight set, grade and level
//!   (`equip_item_attr_modifiers`, `item_grades.stat_multiplier`, `item_configs`);
//! * an NPC: the NPC-owned formula over level and the `npc_template` / `npc_kind` / `npc_grade`
//!   variables of `unit_formula_variables`, with the template's own bonuses (`unit_modifiers` of owner
//!   type `Npc`) and its buffs.

use std::collections::HashMap;

use rusqlite::Connection;

use super::formula::Formula;
mod mate;
pub(super) use mate::details::{Info as MateDetails, MODIFIERS as MATE_MODIFIERS, VALUE_KEYS as MATE_VALUE_KEYS};

/// `UnitFormulaOwnerType`.
const OWNER_CHARACTER: u8 = 0;
const OWNER_NPC: u8 = 1;
/// `UnitFormulaKind`.
const STR: u8 = 22;
const DEX: u8 = 23;
const STA: u8 = 24;
const INT: u8 = 25;
const SPI: u8 = 26;
const FAI: u8 = 27;
const MAX_HEALTH: u8 = 14;
const MAX_MANA: u8 = 15;
/// `UnitFormulaVariableType`.
const VAR_NPC_TEMPLATE: u8 = 2;
const VAR_NPC_KIND: u8 = 3;
const VAR_NPC_GRADE: u8 = 4;
/// `UnitAttribute`: the primaries are 0 to 5, then the maxima.
const ATTR_MAX_HEALTH: u8 = 6;
const ATTR_MAX_MANA: u8 = 7;
/// The server's `0.01f` as the C# compiler rounds it.
#[allow(clippy::excessive_precision)] // the server's literal, kept as is
const F_001: f32 = 0.009_999_999_8;

/// A bonus of a buff or an NPC template (`unit_modifiers`).
#[derive(Clone, Copy, Debug)]
struct Bonus {
    attr: u8,
    percent: bool,
    value: i32,
    /// Added per ability level / 100 (`unit_modifiers.linear_level_bonus`).
    per_level: i32,
}

/// What an item's stats are worked out from.
#[derive(Clone, Copy, Debug, Default)]
struct EquipStat {
    level: i32,
    mod_set: u32,
    /// The body share of an armor or accessory slot (`wearable_slots.coverage`), 0 for a weapon.
    coverage: i32,
    /// `holdables.stat_multiplier` of a weapon.
    holdable: Option<i32>,
}

#[derive(Clone, Copy, Debug, Default)]
struct StatConfig {
    item_stat: i32,
    holdable_stat: i32,
    wearable_stat: i32,
    stat_value: i32,
}

/// A bonus with its value worked out for one buff.
#[derive(Clone, Copy, Debug)]
struct Applied {
    attr: u8,
    percent: bool,
    value: i32,
}

/// The tables the maximum HP / MP are computed from.
#[derive(Default)]
pub struct StatTables {
    /// `(owner type, kind)` -> (`unit_formulas.id`, formula).
    formulas: HashMap<(u8, u8), (u32, Formula)>,
    /// `(unit formula id, variable kind, key)` -> value.
    variables: HashMap<(u32, u8, u32), f64>,
    buff_bonuses: HashMap<u32, Vec<Bonus>>,
    npc_bonuses: HashMap<u32, Vec<Bonus>>,
    items: HashMap<u32, EquipStat>,
    /// `equip_item_attr_modifiers.id` -> weights of str, dex, sta, int, spi.
    weights: HashMap<u32, [i32; 5]>,
    /// `item_grades.id` -> `stat_multiplier`.
    grade_stat: HashMap<i32, i32>,
    config: StatConfig,
    mate: mate::Tables,
}

fn int(r: &rusqlite::Row, i: usize) -> rusqlite::Result<i64> {
    super::data::any_int(r, i)
}

impl StatTables {
    pub fn load(conn: &Connection) -> rusqlite::Result<StatTables> {
        let mut t = StatTables::default();
        {
            let mut q = conn.prepare("SELECT id, formula, kind_id, owner_type_id FROM unit_formulas")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let id = int(r, 0)? as u32;
                let text: String = r.get::<_, Option<String>>(1)?.unwrap_or_default();
                match Formula::parse(&text) {
                    Ok(f) => {
                        t.formulas.insert((int(r, 3)? as u8, int(r, 2)? as u8), (id, f));
                    }
                    Err(e) => bevy::log::debug!("hud: unit formula {id} {text:?}: {e}"),
                }
            }
        }
        {
            let mut q = conn.prepare("SELECT unit_formula_id, variable_kind_id, key, value FROM unit_formula_variables")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                t.variables.insert((int(r, 0)? as u32, int(r, 1)? as u8, int(r, 2)? as u32), r.get::<_, Option<f64>>(3)?.unwrap_or(0.0));
            }
        }
        {
            let mut q = conn.prepare(
                "SELECT owner_id, owner_type, unit_attribute_id, unit_modifier_type_id, value, linear_level_bonus FROM unit_modifiers \
                 WHERE owner_type IN ('Buff', 'Npc') AND unit_attribute_id BETWEEN 0 AND 7",
            )?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let owner = int(r, 0)? as u32;
                let bonus = Bonus { attr: int(r, 2)? as u8, percent: int(r, 3)? == 1, value: int(r, 4)? as i32, per_level: int(r, 5)? as i32 };
                match r.get::<_, Option<String>>(1)?.as_deref() {
                    Some("Buff") => t.buff_bonuses.entry(owner).or_default().push(bonus),
                    _ => t.npc_bonuses.entry(owner).or_default().push(bonus),
                }
            }
        }
        // gear
        {
            let mut q = conn.prepare("SELECT item_stat_const, holdable_stat_const, wearable_stat_const, stat_value_const FROM item_configs LIMIT 1")?;
            let mut rows = q.query([])?;
            if let Some(r) = rows.next()? {
                t.config = StatConfig { item_stat: int(r, 0)? as i32, holdable_stat: int(r, 1)? as i32, wearable_stat: int(r, 2)? as i32, stat_value: int(r, 3)? as i32 };
            }
        }
        {
            let mut q = conn.prepare("SELECT id, stat_multiplier FROM item_grades")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                t.grade_stat.insert(int(r, 0)? as i32, int(r, 1)? as i32);
            }
        }
        {
            let mut q = conn.prepare("SELECT id, str_weight, dex_weight, sta_weight, int_weight, spi_weight FROM equip_item_attr_modifiers")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                t.weights.insert(int(r, 0)? as u32, [int(r, 1)? as i32, int(r, 2)? as i32, int(r, 3)? as i32, int(r, 4)? as i32, int(r, 5)? as i32]);
            }
        }
        let mut level: HashMap<u32, i32> = HashMap::new();
        {
            let mut q = conn.prepare("SELECT id, level FROM items")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                level.insert(int(r, 0)? as u32, int(r, 1)? as i32);
            }
        }
        let mut coverage: HashMap<u32, i32> = HashMap::new();
        {
            let mut q = conn.prepare("SELECT slot_type_id, coverage FROM wearable_slots")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                coverage.insert(int(r, 0)? as u32, int(r, 1)? as i32);
            }
        }
        let mut holdable: HashMap<u32, i32> = HashMap::new();
        {
            let mut q = conn.prepare("SELECT id, stat_multiplier FROM holdables")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                holdable.insert(int(r, 0)? as u32, int(r, 1)? as i32);
            }
        }
        for sql in ["SELECT item_id, mod_set_id, slot_type_id FROM item_armors", "SELECT item_id, mod_set_id, slot_type_id FROM item_accessories"] {
            let mut q = conn.prepare(sql)?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let id = int(r, 0)? as u32;
                let slot = int(r, 2)? as u32;
                t.items.insert(id, EquipStat { level: level.get(&id).copied().unwrap_or(0), mod_set: int(r, 1)? as u32, coverage: coverage.get(&slot).copied().unwrap_or(0), holdable: None });
            }
        }
        {
            let mut q = conn.prepare("SELECT item_id, mod_set_id, holdable_id FROM item_weapons")?;
            let mut rows = q.query([])?;
            while let Some(r) = rows.next()? {
                let id = int(r, 0)? as u32;
                let h = holdable.get(&(int(r, 2)? as u32)).copied().unwrap_or(0);
                t.items.insert(id, EquipStat { level: level.get(&id).copied().unwrap_or(0), mod_set: int(r, 1)? as u32, coverage: 0, holdable: Some(h) });
            }
        }
        t.mate = mate::Tables::load(conn)?;
        Ok(t)
    }

    /// One of a worn item's primary stats (0 str, 1 dex, 2 sta, 3 int, 4 spi): the server's
    /// `Item.Str` .. `Item.Spi` for armor, accessories and weapons, in the same single precision.
    fn item_stat(&self, template: u32, grade: u8, which: usize) -> i32 {
        let Some(item) = self.items.get(&template) else { return 0 };
        if item.mod_set == 0 {
            return 0;
        }
        let (Some(w), Some(&multiplier)) = (self.weights.get(&item.mod_set), self.grade_stat.get(&i32::from(grade))) else { return 0 };
        let count = w.iter().filter(|v| **v > 0).count() as f32;
        let all: i32 = w.iter().sum();
        if all == 0 {
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
        let stat: f32 = match item.holdable {
            Some(holdable) => {
                let temp = c.item_stat as f32 * F_001 * item.level as f32 * m * c.holdable_stat as f32 * F_001;
                let temp2 = count * temp * weight * 1f32 / all as f32 * multiplier as f32 * F_001 + 0.5;
                ((temp2 as i32).wrapping_mul(holdable)) as f32 * F_001 + 0.5
            }
            None => {
                let temp = c.item_stat as f32 * F_001 * item.level as f32 * m * c.wearable_stat as f32 * F_001 * item.coverage as f32 * 0.01;
                count * temp * weight * 1f32 / all as f32 * multiplier as f32 * F_001 + 0.5
            }
        };
        stat as i32
    }

    /// The bonuses a unit's buffs (and an NPC template) give, with their values worked out.
    fn applied(&self, buffs: &[(u32, i16)], npc: Option<u32>) -> Vec<Applied> {
        let mut out = Vec::new();
        let mut add = |b: &Bonus, ability_level: i16| {
            let value = (f64::from(b.value) + f64::from(b.per_level) * f64::from(f32::from(ability_level) / 100.0)).round_ties_even() as i32;
            out.push(Applied { attr: b.attr, percent: b.percent, value });
        };
        if let Some(list) = npc.and_then(|n| self.npc_bonuses.get(&n)) {
            list.iter().for_each(|b| add(b, 0));
        }
        for (id, ability_level) in buffs {
            if let Some(list) = self.buff_bonuses.get(id) {
                list.iter().for_each(|b| add(b, *ability_level));
            }
        }
        out
    }

    fn formula(&self, owner: u8, kind: u8) -> Option<&(u32, Formula)> {
        self.formulas.get(&(owner, kind))
    }

    /// `Character.CalculateWithBonuses`: the flat bonuses, then each percent bonus on the running value.
    fn with_bonuses(value: f64, attr: u8, bonuses: &[Applied]) -> f64 {
        let mut v = value;
        for b in bonuses.iter().filter(|b| b.attr == attr && !b.percent) {
            v += f64::from(b.value);
        }
        for b in bonuses.iter().filter(|b| b.attr == attr && b.percent) {
            v += v * f64::from(b.value) / 100.0;
        }
        v
    }

    /// The player's `(max HP, max MP)` at `level` wearing `gear` (template id, grade) with `buffs`
    /// (buff id, ability level); `None` until the formulas are known.
    pub fn player_max(&self, level: u8, gear: &[(u32, u8)], buffs: &[(u32, i16)]) -> Option<(i32, i32)> {
        let bonuses = self.applied(buffs, None);
        let level_vars = HashMap::from([("level", f64::from(level))]);
        let mut primary = [0i32; 6];
        for (i, kind) in [STR, DEX, STA, INT, SPI, FAI].into_iter().enumerate() {
            let base = self.formula(OWNER_CHARACTER, kind)?.1.eval(&level_vars);
            let worn: i32 = if i < 5 { gear.iter().map(|(t, g)| self.item_stat(*t, *g, i)).sum() } else { 0 };
            primary[i] = Self::with_bonuses(base + f64::from(worn), i as u8, &bonuses) as i32;
        }
        let vars = HashMap::from([
            ("level", f64::from(level)),
            ("str", f64::from(primary[0])),
            ("dex", f64::from(primary[1])),
            ("sta", f64::from(primary[2])),
            ("int", f64::from(primary[3])),
            ("spi", f64::from(primary[4])),
            ("fai", f64::from(primary[5])),
        ]);
        let max_hp = Self::with_bonuses(self.formula(OWNER_CHARACTER, MAX_HEALTH)?.1.eval(&vars), ATTR_MAX_HEALTH, &bonuses) as i32;
        let max_mp = Self::with_bonuses(self.formula(OWNER_CHARACTER, MAX_MANA)?.1.eval(&vars), ATTR_MAX_MANA, &bonuses) as i32;
        Some((max_hp, max_mp))
    }

    /// An NPC's `(max HP, max MP)`: `npc` is its template (`npcs.id`) with the ids the formulas'
    /// variables are keyed by.
    pub fn npc_max(&self, npc: &NpcStatKey, level: u8, buffs: &[(u32, i16)]) -> Option<(i32, i32)> {
        let bonuses = self.applied(buffs, Some(npc.id));
        // the sequential int arithmetic of the server's NPC getters
        let apply = |attr: u8, mut v: i32| {
            for b in bonuses.iter().filter(|b| b.attr == attr) {
                v = if b.percent { v.wrapping_add((v.wrapping_mul(b.value) as f32 / 100.0) as i32) } else { v.wrapping_add(b.value) };
            }
            v
        };
        let eval = |kind: u8, stats: Option<[i32; 6]>| -> Option<f64> {
            let (id, f) = self.formula(OWNER_NPC, kind)?;
            let var = |k: u8, key: u32| self.variables.get(&(*id, k, key)).copied().unwrap_or(0.0);
            let mut vars = HashMap::from([
                ("level", f64::from(level)),
                ("npc_template", var(VAR_NPC_TEMPLATE, npc.template)),
                ("npc_kind", var(VAR_NPC_KIND, npc.kind)),
                ("npc_grade", var(VAR_NPC_GRADE, npc.grade)),
            ]);
            if let Some(s) = stats {
                for (name, v) in ["str", "dex", "sta", "int", "spi", "fai"].into_iter().zip(s) {
                    vars.insert(name, f64::from(v));
                }
            }
            Some(f.eval(&vars))
        };
        let mut stats = [0i32; 6];
        for (i, kind) in [STR, DEX, STA, INT, SPI, FAI].into_iter().enumerate() {
            stats[i] = apply(i as u8, eval(kind, None)? as i32);
        }
        let max_hp = apply(ATTR_MAX_HEALTH, eval(MAX_HEALTH, Some(stats))? as i32);
        let max_mp = apply(ATTR_MAX_MANA, eval(MAX_MANA, Some(stats))? as i32);
        Some((max_hp, max_mp))
    }
}

/// The worn items of a character (slot index, item), without the body parts, as (template id, grade).
pub fn gear_of(equipment: &aa_net::model::Equipment) -> Vec<(u32, u8)> {
    equipment
        .slots
        .iter()
        .enumerate()
        .filter(|(i, _)| !aa_net::model::slot::is_body_part(*i))
        .filter_map(|(_, it)| it.as_ref().map(|it| (it.template_id, it.grade)))
        .collect()
}

/// A unit's buffs as (buff id, ability level), what the bonus arithmetic needs.
pub fn buff_list(buffs: &[aa_net::state::BuffEntry]) -> Vec<(u32, i16)> {
    buffs.iter().map(|b| (b.buff.buff_id, b.buff.ability_level)).collect()
}

/// The `(max HP, max MP)` of a unit in view: an NPC from its template, a character from its level, worn
/// items and buffs; `None` for anything else (or before the database is read).
pub fn unit_max(game: &super::data::GameData, unit: &aa_net::Unit, buffs: &[aa_net::state::BuffEntry]) -> Option<(i32, i32)> {
    let list = buff_list(buffs);
    let level = unit.level.max(1);
    match &unit.kind {
        aa_net::model::UnitKind::Npc { template_id, .. } => {
            let npc = game.npcs.get(template_id)?;
            let key = NpcStatKey { id: *template_id, template: npc.template as u32, kind: npc.kind as u32, grade: npc.grade as u32 };
            game.stats.npc_max(&key, level, &list)
        }
        aa_net::model::UnitKind::Character { .. } => game.stats.player_max(level, &gear_of(&unit.equipment), &list),
        _ => None,
    }
}

/// Mate formulas require a known native stack view; a lane charge-derived list is not equivalent.
pub(super) fn mate_max(game: &super::data::GameData, unit: &aa_net::Unit, buffs: Option<&[aa_net::state::BuffEntry]>) -> Option<(i32, i32)> {
    mate::pools(game, unit, buffs)
}

pub(super) fn mate_details(game: &super::data::GameData, unit: &aa_net::Unit, buffs: Option<&[aa_net::state::BuffEntry]>) -> Option<MateDetails> {
    mate::details::get(game, unit, buffs)
}

/// The keys of an NPC's formula variables.
#[derive(Clone, Copy, Debug, Default)]
pub struct NpcStatKey {
    /// `npcs.id` (the template's own bonuses).
    pub id: u32,
    pub template: u32,
    pub kind: u32,
    pub grade: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tables() -> Option<StatTables> {
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
        StatTables::load(&conn).ok()
    }

    #[test]
    fn a_new_character_without_gear() {
        let Some(t) = tables() else { return };
        // level 1: every stat is 10; 150 + 10 * 12 + 100 HP, 100 + 10 * 12 + 100 MP
        assert_eq!(t.player_max(1, &[], &[]), Some((370, 320)));
        // level 50: 108 stamina and intelligence
        assert_eq!(t.player_max(50, &[], &[]), Some((50 * 150 + 108 * 12 + 100, 50 * 100 + 108 * 12 + 100)));
    }

    #[test]
    fn flat_then_percent_bonuses() {
        let b = |attr, percent, value| Applied { attr, percent, value };
        let bonuses = [b(6, true, 10), b(6, false, 100), b(7, false, 5)];
        assert!((StatTables::with_bonuses(1000.0, 6, &bonuses) - 1210.0).abs() < 1e-9);
        assert_eq!(StatTables::with_bonuses(1000.0, 2, &bonuses), 1000.0);
    }

    #[test]
    fn a_normal_npc_has_the_formula_hp() {
        let Some(t) = tables() else { return };
        // a level 1 monster of template 9, kind 1, grade 1 (a normal beast): positive and not absurd
        let key = NpcStatKey { id: 0, template: 9, kind: 1, grade: 1 };
        let (hp, _) = t.npc_max(&key, 1, &[]).unwrap();
        assert!((50..2000).contains(&hp), "{hp}");
        // grade 8 has a far larger multiplier than grade 1
        let boss = NpcStatKey { grade: 8, ..key };
        assert!(t.npc_max(&boss, 1, &[]).unwrap().0 > 50 * hp / 2);
    }
}
