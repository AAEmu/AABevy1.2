//! Mate owner5 formulas and native aggregate modifier arithmetic, distinct from lane formulas.
use super::{Bonus, StatTables};
use aa_net::{
    model::{Equipment, ItemDetail},
    state::BuffEntry,
};
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};
pub(super) mod details;

#[derive(Default)]
pub(super) struct Tables {
    kinds: HashMap<u32, u32>,
    item_levels: HashMap<u32, i8>,
    item_mods: HashMap<u32, Vec<Bonus>>,
    one_time: HashSet<u32>,
    conditional: HashSet<u32>,
    dynamic: HashSet<u32>,
    limits: HashMap<u8, (i32, i32)>,
    details: details::Tables,
}

impl Tables {
    pub(super) fn load(db: &Connection) -> rusqlite::Result<Self> {
        let mut out = Self::default();
        let mut q = db.prepare("SELECT id,mate_kind_id FROM npcs WHERE mate_kind_id IS NOT NULL AND mate_kind_id<>0")?;
        for row in q.query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, u32>(1)?)))? {
            let (id, kind) = row?;
            out.kinds.insert(id, kind);
        }
        let mut q = db.prepare("SELECT id,level FROM items")?;
        for row in q.query_map([], |r| Ok((super::int(r, 0)? as u32, super::int(r, 1)? as i8)))? {
            let (id, level) = row?;
            out.item_levels.insert(id, level);
        }
        let mut q=db.prepare("SELECT owner_id,unit_attribute_id,unit_modifier_type_id,value,linear_level_bonus FROM unit_modifiers WHERE owner_type='Item' AND unit_attribute_id BETWEEN 0 AND 7 AND unit_modifier_type_id IN(0,1)")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            out.item_mods.entry(super::int(r, 0)? as u32).or_default().push(Bonus {
                attr: super::int(r, 1)? as u8,
                percent: super::int(r, 2)? == 1,
                value: super::int(r, 3)? as i32,
                per_level: super::int(r, 4)? as i32,
            });
        }
        let mut q = db.prepare("SELECT id,one_time FROM buffs")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            if crate::hud::data::any_flag(r, 1)? {
                out.one_time.insert(super::int(r, 0)? as u32);
            }
        }
        let mut q=db.prepare("SELECT b.owner_id FROM buff_unit_modifiers b JOIN unit_modifiers m ON m.owner_type='BuffUnitModifier' AND m.owner_id=b.id WHERE b.owner_type='Buff' AND m.unit_attribute_id BETWEEN 0 AND 7")?;
        for row in q.query_map([], |r| r.get::<_, u32>(0))? {
            out.conditional.insert(row?);
        }
        let mut q = db.prepare("SELECT buff_id FROM dynamic_unit_modifiers WHERE unit_attribute_id BETWEEN 0 AND 7")?;
        for row in q.query_map([], |r| r.get::<_, u32>(0))? {
            out.dynamic.insert(row?);
        }
        let mut q = db.prepare("SELECT unit_attribute_id,minimum,maximum FROM unit_attribute_limits WHERE unit_attribute_id BETWEEN 0 AND 7")?;
        for row in q.query_map([], |r| Ok((r.get::<_, u8>(0)?, (super::int(r, 1)? as i32, super::int(r, 2)? as i32))))? {
            let (id, range) = row?;
            out.limits.insert(id, range);
        }
        out.details = details::Tables::load(db)?;
        Ok(out)
    }
}

fn amount(bonus: &Bonus, level: i32, multiplier: i32) -> i32 {
    let value = if bonus.per_level == 0 {
        bonus.value
    } else {
        let delta = (bonus.per_level.wrapping_mul(level) as f32 * 0.01f32 + 0.5f32).floor();
        (delta + bonus.value as f32) as i32
    };
    value.wrapping_mul(multiplier)
}

struct Totals {
    flat: [i32; 8],
    percent: [i32; 8],
}
impl Default for Totals {
    fn default() -> Self {
        Self {
            flat: [0; 8],
            percent: [100; 8],
        }
    }
}
impl Totals {
    fn add(&mut self, list: Option<&Vec<Bonus>>, level: i32, multiplier: i32) {
        for bonus in list.into_iter().flatten() {
            let attr = usize::from(bonus.attr);
            let value = amount(bonus, level, multiplier);
            let target = if bonus.percent { &mut self.percent[attr] } else { &mut self.flat[attr] };
            *target = target.wrapping_add(value);
        }
    }
    fn apply(&self, attr: usize, base: i32, tables: &Tables) -> i32 {
        let flat = base.wrapping_add(self.flat[attr]);
        let value = if self.percent[attr] == 100 {
            flat
        } else {
            (flat as f32 * self.percent[attr] as f32 * 0.01f32) as i32
        };
        tables.limits.get(&(attr as u8)).map_or(value, |(min, max)| value.clamp(*min, *max))
    }
}

fn slot_matches(kind: u32, slot: usize) -> bool {
    let (primary, secondary) = match kind {
        1..=9 => ((kind - 1) as usize, None),
        10 => (9, Some(10)),
        11 => (11, Some(12)),
        12..=15 => ((kind + 1) as usize, None),
        16 => (15, None),
        17 => (15, Some(16)),
        18 => (17, None),
        20 => (16, None),
        21 => (18, None),
        23..=31 => ((kind - 4) as usize, None),
        _ => return false,
    };
    slot == primary || secondary == Some(slot)
}

fn eligible(game: &crate::hud::data::GameData, template: u32, slot: usize) -> bool {
    // Backpacks select slotType30 (raw26), which is outside this actor's0..18+27 scan.
    game.items.get(&template).is_some_and(|i| matches!(i.impl_id, 1 | 2 | 24)) && slot_matches(game.tips.items.slot_type(template), slot)
}

/// Native3989fa80: x87/f64 products, narrowed power, and explicit f32 floor stages.
fn item_stat(t: &StatTables, template: u32, grade: u8, attr: usize) -> i32 {
    let Some(item) = t.items.get(&template) else { return 0 };
    let Some(weights) = t.weights.get(&item.mod_set) else { return 0 };
    let weight = f64::from(weights[attr] as u8);
    if weight == 0. {
        return 0;
    }
    let count = weights.iter().filter(|v| **v != 0).count() as u8;
    let sum = weights.iter().fold(0i32, |a, v| a.wrapping_add(*v)) as u8;
    if count == 0 || sum == 0 {
        return 0;
    }
    let Some(&grade) = t.grade_stat.get(&i32::from(grade)) else { return 0 };
    let factor = match count {
        1 => 3.0f64,
        2 => 1.5,
        _ => 1.0,
    };
    let exponent = 1.0 / (f64::from(t.config.stat_value as u32) * f64::from(0.01f32));
    let power = factor.powf(exponent) as f32;
    let mut base = f64::from(t.config.item_stat as u32) * 0.01 * f64::from(item.level as i8) * f64::from(power);
    if item.holdable.is_some() {
        base *= f64::from(t.config.holdable_stat as u32) * 0.01;
    } else {
        base *= f64::from(t.config.wearable_stat as u32) * 0.01;
        base *= f64::from(item.coverage) * 0.01;
    }
    let first = (f64::from(count) * base * weight / f64::from(sum) * f64::from(grade) * 0.01) as f32;
    let rounded = (first + 0.5f32).floor();
    item.holdable.map_or(rounded as i32, |multiplier| {
        ((f64::from(rounded) * f64::from(multiplier) * 0.01) as f32 + 0.5f32).floor() as i32
    })
}

fn gear(game: &crate::hud::data::GameData, equipment: &Equipment, totals: &mut Totals) {
    let tables = &game.stats.mate;
    for slot in (0..=18).chain(std::iter::once(27)) {
        let Some(item) = equipment.slots.get(slot).and_then(Option::as_ref) else {
            continue;
        };
        if !eligible(game, item.template_id, slot) {
            continue;
        }
        let detail = match &item.detail {
            ItemDetail::Equip(d) => Some(d),
            _ => None,
        };
        if detail.is_none_or(|d| d.durability == 0) && game.tips.items.durability(item.template_id, item.grade).is_some() {
            continue;
        }
        for attr in 0..5 {
            totals.flat[attr] = totals.flat[attr].wrapping_add(item_stat(&game.stats, item.template_id, item.grade, attr));
        }
        let mut add = |template: u32| {
            let Some(level) = tables.item_levels.get(&template) else { return };
            totals.add(tables.item_mods.get(&template), i32::from(*level), 1);
        };
        add(item.template_id);
        if let Some(detail) = detail {
            if detail.rune_id != 0 {
                add(detail.rune_id);
            }
            for &gem in &detail.gem_ids {
                if gem == 0 {
                    break;
                }
                add(gem);
            }
        }
    }
}

fn context(game: &crate::hud::data::GameData, unit: &aa_net::Unit, buffs: Option<&[BuffEntry]>) -> Option<(u32, Totals)> {
    let aa_net::model::UnitKind::Mate { template_id, .. } = unit.kind else {
        return None;
    };
    let t = &game.stats;
    let tables = &t.mate;
    let kind = *tables.kinds.get(&template_id)?;
    let mut totals = Totals::default();
    totals.add(t.npc_bonuses.get(&template_id), i32::from(unit.level), 1);
    for entry in buffs? {
        let buff = &entry.buff;
        if tables.one_time.contains(&buff.buff_id) {
            continue;
        }
        if tables.conditional.contains(&buff.buff_id) || (tables.dynamic.contains(&buff.buff_id) && buff.tick_ms != 0 && buff.duration_ms != 0) {
            return None;
        }
        totals.add(t.buff_bonuses.get(&buff.buff_id), i32::from(buff.ability_level as u16), buff.stacks);
    }
    gear(game, &unit.equipment, &mut totals);
    Some((kind, totals))
}

fn evaluate(t: &StatTables, kind: u32, level: u8, formula_kind: u8, primary: [i32; 6]) -> Option<i32> {
    let (id, formula) = t.formulas.get(&(5, formula_kind))?;
    let variable = t.variables.get(&(*id, 5, kind)).copied().unwrap_or(0.) as f32;
    let mut vars = HashMap::from([("level", f64::from(level)), ("mate_kind", f64::from(variable))]);
    for (name, value) in ["str", "dex", "sta", "int", "spi", "fai"].into_iter().zip(primary) {
        vars.insert(name, f64::from(value as f32));
    }
    Some(details::trunc(formula.eval_native(&vars)? as f32))
}

fn primaries(t: &StatTables, kind: u32, level: u8, totals: &Totals) -> Option<[i32; 6]> {
    let mut primary = [0; 6];
    for (i, formula) in [22, 23, 24, 25, 26, 27].into_iter().enumerate() {
        primary[i] = totals.apply(i, evaluate(t, kind, level, formula, [0; 6])?, &t.mate);
    }
    Some(primary)
}

pub(super) fn pools(game: &crate::hud::data::GameData, unit: &aa_net::Unit, buffs: Option<&[BuffEntry]>) -> Option<(i32, i32)> {
    let (kind, totals) = context(game, unit, buffs)?;
    let t = &game.stats;
    let primary = primaries(t, kind, unit.level, &totals)?;
    Some((
        totals.apply(6, evaluate(t, kind, unit.level, 14, primary)?, &t.mate),
        totals.apply(7, evaluate(t, kind, unit.level, 15, primary)?, &t.mate),
    ))
}

#[cfg(test)]
mod tests;
