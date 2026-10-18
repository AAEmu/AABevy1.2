//! Original enchant descriptors and quote arithmetic. All grade keys are raw IDs.
use std::collections::{HashMap, HashSet};

use aa_net::model::{Item, ItemDetail};
use rusqlite::{Connection, OptionalExtension};

use super::super::{
    data::{any_flag, any_int},
    formula::Formula,
};

#[derive(Default)]
pub(super) struct Descriptor {
    pub level: i8,
    pub requirement: i32,
    pub limit: i32,
    pub impl_id: i32,
    pub grade_enchantable: bool,
    pub skill: u32,
    pub reagent: bool,
    pub actability_group: u32,
    pub actability_requirement: i32,
}
#[derive(Default)]
pub(super) struct Grade {
    pub ratios: [i32; 3], // success, break, downgrade
    pub cost: f32,
    pub currency: i32,
}
#[derive(Default)]
pub(super) struct Support {
    pub minimum: i32,
    pub maximum: i32,
    pub flat: [i32; 3],
    pub mul: [i32; 3],
}
pub(super) struct Gem {
    pub level: i32,
    pub group: u32,
    pub grade: u8,
}
struct Effect {
    kind: i32,
    values: [i32; 3],
    tags: [u32; 4],
}
struct Modifier {
    attr: u8,
    kind: i32,
    value: i32,
}
#[derive(Default)]
pub(in crate::hud) struct Tables {
    pub(super) items: HashMap<u32, Descriptor>,
    pub(super) grades: HashMap<u8, Grade>,
    pub(super) supports: HashMap<u32, Support>,
    pub(super) gems: HashMap<u32, Gem>,
    pub(super) sockets: HashMap<u32, u32>,
    pub(super) socket_levels: HashMap<u32, i32>,
    pub(super) slot: HashMap<u32, u32>,
    pub(super) socket_count: HashMap<(u32, u8), u8>,
    groups: HashMap<u32, HashSet<u32>>,
    grade_order: Vec<u8>,
    slot_cost: HashMap<u32, f32>,
    cost_formula: Option<Formula>,
    pub(super) socket_minimum: Option<i32>,
    effects: HashMap<u32, Vec<Effect>>,
    zero_free_tags: bool,
    modifiers: HashMap<u32, Vec<Modifier>>,
}

pub(super) fn cvtt(value: f64) -> i32 {
    if value.is_finite() && (-2147483648.0..2147483648.0).contains(&value) {
        value as i32
    } else {
        i32::MIN
    }
}
fn variation(base: i32, flat: i32, mul: i32) -> i32 {
    //39044120: narrow after each SSE operation, CVTT before signed clamp.
    let factor = 1.0f32 + mul as f32 * 0.01f32;
    let adjusted = cvtt(f64::from(factor * base.wrapping_add(flat) as f32)).clamp(0, 10000);
    if base == 0 {
        cvtt(f64::from(adjusted as f32 * 0.01f32))
    } else {
        cvtt(f64::from(adjusted.wrapping_sub(base)) / f64::from(base) * 100.0)
    }
}
impl Tables {
    pub(in crate::hud) fn load(conn: &Connection) -> rusqlite::Result<Self> {
        let mut out = Self::default();
        let mut q = conn.prepare("SELECT id,level,level_requirement,level_limit,impl_id,grade_enchantable,use_skill_id,use_skill_as_reagent,actability_group_id,actability_requirement FROM items")?;
        for row in q.query_map([], |r| {
            Ok((
                any_int(r, 0)? as u32,
                Descriptor {
                    level: any_int(r, 1)? as i8,
                    requirement: any_int(r, 2)? as i32,
                    limit: any_int(r, 3)? as i32,
                    impl_id: any_int(r, 4)? as i32,
                    grade_enchantable: any_flag(r, 5)?,
                    skill: any_int(r, 6)? as u32,
                    reagent: any_flag(r, 7)?,
                    actability_group: any_int(r, 8)? as u32,
                    actability_requirement: any_int(r, 9)? as i32,
                },
            ))
        })? {
            let (id, value) = row?;
            out.items.insert(id, value);
        }
        let mut q = conn.prepare("SELECT id,grade_enchant_success_ratio,grade_enchant_break_ratio,grade_enchant_downgrade_ratio,grade_enchant_cost,currency_id FROM item_grades")?;
        for row in q.query_map([], |r| {
            Ok((
                any_int(r, 0)? as u8,
                Grade {
                    ratios: [any_int(r, 1)? as i32, any_int(r, 2)? as i32, any_int(r, 3)? as i32],
                    cost: any_int(r, 4)? as f32,
                    currency: any_int(r, 5)? as i32,
                },
            ))
        })? {
            let (id, value) = row?;
            out.grades.insert(id, value);
        }
        let mut q = conn.prepare("SELECT id FROM item_grades ORDER BY grade_order ASC")?;
        out.grade_order = q.query_map([], |r| Ok(any_int(r, 0)? as u8))?.collect::<rusqlite::Result<_>>()?;
        let mut q = conn.prepare("SELECT item_id,require_grade_min,require_grade_max,add_success_ratio,add_break_ratio,add_downgrade_ratio,add_success_mul,add_break_mul,add_downgrade_mul FROM item_grade_enchanting_supports")?;
        for row in q.query_map([], |r| {
            Ok((
                any_int(r, 0)? as u32,
                Support {
                    minimum: any_int(r, 1)? as i32,
                    maximum: any_int(r, 2)? as i32,
                    flat: [any_int(r, 3)? as i32, any_int(r, 4)? as i32, any_int(r, 5)? as i32],
                    mul: [any_int(r, 6)? as i32, any_int(r, 7)? as i32, any_int(r, 8)? as i32],
                },
            ))
        })? {
            let (id, value) = row?;
            out.supports.insert(id, value);
        }
        let mut q = conn.prepare("SELECT item_id,equip_level,equip_slot_group_id,item_grade_id FROM item_enchanting_gems")?;
        for row in q.query_map([], |r| {
            Ok((
                any_int(r, 0)? as u32,
                Gem {
                    level: any_int(r, 1)? as i32,
                    group: any_int(r, 2)? as u32,
                    grade: any_int(r, 3)? as u8,
                },
            ))
        })? {
            let (id, value) = row?;
            out.gems.insert(id, value);
        }
        for (sql, sockets) in [
            ("SELECT item_id,equip_slot_group_id FROM item_sockets", true),
            (
                "SELECT w.item_id,h.slot_type_id FROM item_weapons w JOIN holdables h ON h.id=w.holdable_id JOIN items i ON i.id=w.item_id WHERE i.impl_id=1",
                false,
            ),
            (
                "SELECT a.item_id,a.slot_type_id FROM item_armors a JOIN items i ON i.id=a.item_id WHERE i.impl_id=2",
                false,
            ),
            (
                "SELECT a.item_id,a.slot_type_id FROM item_accessories a JOIN items i ON i.id=a.item_id WHERE i.impl_id=24",
                false,
            ),
        ] {
            let destination = if sockets { &mut out.sockets } else { &mut out.slot };
            let mut q = conn.prepare(sql)?;
            for row in q.query_map([], |r| Ok((any_int(r, 0)? as u32, any_int(r, 1)? as u32)))? {
                let (id, value) = row?;
                destination.insert(id, value);
            }
        }
        let mut q = conn.prepare("SELECT item_id,level FROM item_socket_level_limits")?;
        for row in q.query_map([], |r| Ok((any_int(r, 0)? as u32, any_int(r, 1)? as i32)))? {
            let (id, value) = row?;
            out.socket_levels.insert(id, value);
        }
        let mut q = conn.prepare("SELECT slot_id,grade_id,num_socket FROM item_socket_num_limits")?;
        for row in q.query_map([], |r| Ok(((any_int(r, 0)? as u32, any_int(r, 1)? as u8), any_int(r, 2)? as u8)))? {
            let (id, value) = row?;
            out.socket_count.insert(id, value);
        }
        let mut q = conn.prepare("SELECT id FROM equip_slot_groups")?;
        for row in q.query_map([], |r| Ok(any_int(r, 0)? as u32))? {
            out.groups.insert(row?, HashSet::new());
        }
        let mut q = conn.prepare("SELECT equip_slot_group_id,equip_slot_type_id FROM equip_slot_group_maps")?;
        for row in q.query_map([], |r| Ok((any_int(r, 0)? as u32, any_int(r, 1)? as u32)))? {
            let (id, value) = row?;
            if let Some(group) = out.groups.get_mut(&id) {
                group.insert(value);
            }
        }
        let mut q = conn.prepare("SELECT slot_type_id,cost FROM equip_slot_enchanting_costs")?;
        for row in q.query_map([], |r| Ok((any_int(r, 0)? as u32, any_int(r, 1)? as f32)))? {
            let (id, value) = row?;
            out.slot_cost.insert(id, value);
        }
        let source = conn
            .query_row("SELECT formula FROM formulas WHERE id=22", [], |r| r.get::<_, String>(0))
            .optional()?;
        out.cost_formula = source.and_then(|s| Formula::parse(&s).ok());
        out.socket_minimum = conn
            .query_row("SELECT value FROM content_configs WHERE id=62", [], |r| Ok(any_int(r, 0)? as i32))
            .optional()?;
        //3989fa80 returns generated primary0 when all three equipment lookups
        // miss. All actual impl15/21 sources satisfy this, independently checked
        // against the catalog and1785 executed original helper cases.
        let mut equipment = HashSet::new();
        for sql in [
            "SELECT item_id FROM item_weapons",
            "SELECT item_id FROM item_armors",
            "SELECT item_id FROM item_accessories",
        ] {
            let mut q = conn.prepare(sql)?;
            for id in q.query_map([], |r| Ok(any_int(r, 0)? as u32))? {
                equipment.insert(id?);
            }
        }
        for (&id, desc) in &out.items {
            if matches!(desc.impl_id, 15 | 21) && !equipment.contains(&id) {
                out.modifiers.insert(id, Vec::new());
            }
        }
        let mut q = conn.prepare("SELECT owner_type,owner_id,unit_attribute_id,unit_modifier_type_id,value FROM unit_modifiers")?;
        for row in q.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                any_int(r, 1)? as u32,
                any_int(r, 2)?,
                any_int(r, 3)? as i32,
                any_int(r, 4)? as i32,
            ))
        })? {
            let (owner, id, attr, kind, value) = row?;
            if owner != "Item" {
                continue;
            }
            if let Some(rows) = out.modifiers.get_mut(&id) {
                if let Ok(attr) = u8::try_from(attr) {
                    rows.push(Modifier { attr, kind, value });
                } else {
                    out.modifiers.remove(&id);
                }
            }
        }
        // A replacement catalog with unprovided attribute names cannot become a
        // misleading partially empty effect quote.
        out.modifiers
            .retain(|_, rows| rows.iter().all(|m| super::super::tipcalc::attribute_name(m.attr).is_some()));
        out.zero_free_tags = conn.query_row(
            "SELECT NOT EXISTS(SELECT 1 FROM tagged_buffs WHERE COALESCE(tag_id,0)=0)",
            [],
            |r| r.get(0),
        )?;
        let mut q=conn.prepare("SELECT se.skill_id,s.special_effect_type_id,s.value1,s.value2,s.value3,se.source_buff_tag_id,se.source_nobuff_tag_id,se.target_buff_tag_id,se.target_nobuff_tag_id FROM skill_effects se JOIN effects e ON e.id=se.effect_id AND e.actual_type='SpecialEffect' JOIN special_effects s ON s.id=e.actual_id WHERE s.special_effect_type_id IN (92,106)")?;
        for row in q.query_map([], |r| {
            Ok((
                any_int(r, 0)? as u32,
                Effect {
                    kind: any_int(r, 1)? as i32,
                    values: [any_int(r, 2)? as i32, any_int(r, 3)? as i32, any_int(r, 4)? as i32],
                    tags: [
                        any_int(r, 5)? as u32,
                        any_int(r, 6)? as u32,
                        any_int(r, 7)? as u32,
                        any_int(r, 8)? as u32,
                    ],
                },
            ))
        })? {
            let (id, value) = row?;
            out.effects.entry(id).or_default().push(value);
        }
        Ok(out)
    }
    pub fn cost(&self, item: &Item) -> Option<i32> {
        let Some(desc) = self.items.get(&item.template_id) else {
            return Some(0);
        };
        let Some(cost) = self.slot.get(&item.template_id).and_then(|slot| self.slot_cost.get(slot)) else {
            return Some(0);
        };
        self.formula_cost(desc.level, item.grade, *cost)
    }
    fn formula_cost(&self, level: i8, grade: u8, cost: f32) -> Option<i32> {
        let vars = HashMap::from([
            ("item_level", f64::from(level)),
            ("item_grade", f64::from(self.grades.get(&grade)?.cost)),
            ("equip_slot_enchant_cost", f64::from(cost)),
        ]);
        Some(cvtt(f64::from(self.cost_formula.as_ref()?.eval_native(&vars)? as f32)))
    }
    pub fn ratios(&self, grade: u8, support: Option<u32>) -> Option<([bool; 3], Option<[i32; 3]>)> {
        let base = self.grades.get(&grade)?.ratios;
        let variation = match support {
            None => None,
            Some(id) => {
                let support = self.supports.get(&id)?;
                Some(std::array::from_fn(|i| variation(base[i], support.flat[i], support.mul[i])))
            }
        };
        Some((base.map(|value| value != 0), variation))
    }
    fn group_allows(&self, group: u32, slot: u32) -> bool {
        group == 0
            || self
                .groups
                .get(&group)
                .is_some_and(|members| members.is_empty() || members.contains(&slot))
    }
    pub fn gem_allows(&self, target: &Item, source: &Item) -> bool {
        let (Some(gem), Some(desc), Some(&slot)) = (
            self.gems.get(&source.template_id),
            self.items.get(&target.template_id),
            self.slot.get(&target.template_id),
        ) else {
            return false;
        };
        self.items.contains_key(&source.template_id)
            && slot != 0
            && target.grade >= gem.grade
            && desc.requirement >= gem.level
            && self.group_allows(gem.group, slot)
    }
    pub fn support_allows(&self, target: &Item, source: &Item, support: &Item) -> bool {
        if target.template_id == 30803 || source.template_id == 30804 {
            return false;
        }
        let Some(s) = self.supports.get(&support.template_id) else {
            return false;
        };
        if s.minimum > s.maximum {
            return false;
        }
        let index = |grade| self.grade_order.iter().position(|&g| g == grade).unwrap_or(self.grade_order.len());
        (s.minimum == -1 || index(target.grade) >= index(s.minimum as u8))
            && (s.maximum == -1 || index(target.grade) <= index(s.maximum as u8))
    }
    pub fn socket_allows(&self, target: &Item, source: &Item, install: bool) -> bool {
        let Some(desc) = self.items.get(&target.template_id).filter(|_| target.template_id != 0) else {
            return false;
        };
        if self.socket_minimum.is_none_or(|minimum| i32::from(desc.level) < minimum) {
            return false;
        }
        let count = gem_count(target);
        if !install {
            return count > 0;
        }
        let (Some(&group), Some(&slot)) = (self.sockets.get(&source.template_id), self.slot.get(&target.template_id)) else {
            return false;
        };
        slot != 0
            && self.items.contains_key(&source.template_id)
            && self.group_allows(group, slot)
            && self.socket_levels.get(&source.template_id).copied().unwrap_or(i32::MAX) <= desc.requirement
            && count < self.socket_count.get(&(slot, target.grade)).copied().unwrap_or(0)
    }
    pub(super) fn pair_allows(&self, mode: super::Mode, target: &Item, source: &Item) -> bool {
        if mode == super::Mode::Gem {
            return self.gem_allows(target, source);
        }
        let Some(desc) = self.items.get(&source.template_id) else {
            return false;
        };
        let Some(effects) = self.effects.get(&desc.skill) else {
            return false;
        };
        let eligible = |effect: &&Effect| self.zero_free_tags && effect.tags == [0; 4];
        if mode == super::Mode::Grade {
            //39045400 stops after the FIRST condition-admitted SpecialEffect92,
            // even when its local preview fails. Unknown nonzero tag providers refuse.
            let Some(effect) = effects.iter().find(|e| e.kind == 92) else {
                return false;
            };
            if !eligible(&effect) {
                return false;
            }
            let Some(target_desc) = self.items.get(&target.template_id) else {
                return false;
            };
            target_desc.grade_enchantable
                && target_desc.requirement >= effect.values[1]
                && (effect.values[2] == -1 || effect.values[2] == target_desc.impl_id)
                && target.grade != 11
                && ((source.template_id == 30804) == (target.template_id == 30803))
        } else if matches!(mode, super::Mode::Socket | super::Mode::Unsocket) {
            //39045550 instead continues after a failed socket preview.
            let install = mode == super::Mode::Socket;
            effects
                .iter()
                .filter(|e| e.kind == 106 && (e.values[0] != 0) == install)
                .filter(eligible)
                .any(|_| self.socket_allows(target, source, install))
        } else {
            false
        }
    }
    pub(super) fn modifier_known(&self, id: u32) -> bool {
        self.modifiers.contains_key(&id)
    }
    pub(in crate::hud) fn modifier_table(&self, lua: &mlua::Lua, id: u32) -> mlua::Result<Option<mlua::Table>> {
        let Some(rows) = self.modifiers.get(&id) else { return Ok(None) };
        let out = lua.create_table()?;
        let add = |attr: u8, value: i32, kind: i32| -> mlua::Result<()> {
            let row = lua.create_table()?;
            row.raw_set("name", super::super::tipcalc::attribute_name(attr))?;
            row.raw_set("value", f64::from(value as f32))?;
            row.raw_set("type", f64::from(kind as f32))?;
            out.raw_push(row)
        };
        //3941dc60 emits generated+FIRST matching static primary in attr0..4
        // order, omitting zero. These qualified source classes generate zero.
        for attr in 0..5 {
            if let Some(m) = rows.iter().find(|m| m.attr == attr).filter(|m| m.value != 0) {
                add(attr, m.value, 0)?;
            }
        }
        let mut shared = false;
        for m in rows.iter().filter(|m| m.attr > 4) {
            if matches!(m.attr, 54 | 55 | 74 | 119) {
                if shared {
                    continue;
                }
                shared = true;
            }
            add(m.attr, m.value, m.kind)?;
        }
        Ok(Some(out))
    }
}
pub(super) fn gem_count(item: &Item) -> u8 {
    match &item.detail {
        ItemDetail::Equip(e) => e.gem_ids.iter().take_while(|&&id| id != 0).count() as u8,
        _ => 0,
    }
}

#[cfg(test)]
mod tests;
