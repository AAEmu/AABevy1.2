//! Native wearable basis-point and holdable formula paths, separate from lane tooltip arithmetic.
use super::*;
use crate::hud::{data::any_int as int, formula::Formula};

#[derive(Default)]
pub(super) struct Tables {
    items: HashMap<u32, ItemBase>,
    holdables: HashMap<u32, [Option<Formula>; 2]>,
    wearable: [Option<Formula>; 2],
    grades: HashMap<u8, [f32; 4]>,
}
struct ItemBase {
    level: i8,
    kind: Kind,
}
enum Kind {
    Wearable { armor: i32, resistance: i32 },
    Holdable(u32),
}

impl Tables {
    pub(super) fn load(db: &Connection) -> rusqlite::Result<Self> {
        let mut out = Self::default();
        let mut q=db.prepare("SELECT a.item_id,i.level,w.armor_bp,w.magic_resistance_bp FROM item_armors a JOIN items i ON i.id=a.item_id JOIN wearables w ON w.slot_type_id=a.slot_type_id AND w.armor_type_id=a.type_id UNION ALL SELECT a.item_id,i.level,w.armor_bp,w.magic_resistance_bp FROM item_accessories a JOIN items i ON i.id=a.item_id JOIN wearables w ON w.slot_type_id=a.slot_type_id AND w.armor_type_id=a.type_id")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            out.items.insert(
                int(r, 0)? as u32,
                ItemBase {
                    level: int(r, 1)? as i8,
                    kind: Kind::Wearable {
                        armor: int(r, 2)? as i32,
                        resistance: int(r, 3)? as i32,
                    },
                },
            );
        }
        let mut q = db.prepare("SELECT w.item_id,i.level,w.holdable_id FROM item_weapons w JOIN items i ON i.id=w.item_id")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            out.items.insert(
                int(r, 0)? as u32,
                ItemBase {
                    level: int(r, 1)? as i8,
                    kind: Kind::Holdable(int(r, 2)? as u32),
                },
            );
        }
        let mut q = db.prepare("SELECT id,formula_dps,formula_armor FROM holdables")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            out.holdables.insert(
                int(r, 0)? as u32,
                [
                    r.get::<_, Option<String>>(1)?.and_then(|text| Formula::parse(&text).ok()),
                    r.get::<_, Option<String>>(2)?.and_then(|text| Formula::parse(&text).ok()),
                ],
            );
        }
        let mut q = db.prepare("SELECT kind_id,formula FROM wearable_formulas WHERE kind_id IN(0,1)")?;
        for row in q.query_map([], |r| Ok((r.get::<_, u8>(0)?, r.get::<_, String>(1)?)))? {
            let (kind, text) = row?;
            out.wearable[usize::from(kind)] = Formula::parse(&text).ok();
        }
        let mut q = db.prepare("SELECT id,var_holdable_dps,var_holdable_armor,var_wearable_armor,var_wearable_magic_resistance FROM item_grades")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            out.grades.insert(
                r.get(0)?,
                [
                    r.get::<_, f64>(1)? as f32,
                    r.get::<_, f64>(2)? as f32,
                    r.get::<_, f64>(3)? as f32,
                    r.get::<_, f64>(4)? as f32,
                ],
            );
        }
        Ok(out)
    }

    fn wearable(&self, item: &ItemBase, grade: u8, resistance: bool, received_override: u16) -> Option<i32> {
        let Kind::Wearable { armor, resistance: resist } = item.kind else {
            return None;
        };
        let bp = if resistance { resist } else { armor };
        if bp == 0 {
            return Some(0);
        }
        let index = usize::from(resistance);
        let grade = self.grades.get(&grade)?[2 + index];
        let vars = HashMap::from([("item_level", f64::from(item.level)), ("item_grade", f64::from(grade))]);
        let mut value = self.wearable[index].as_ref()?.eval_native(&vars)? as f32;
        if received_override > 0 {
            value = value * f32::from(received_override) * 0.01f32;
        }
        Some(super::trunc(bp as f32 * value * 0.0001f32))
    }

    fn holdable(&self, item: &ItemBase, grade: u8, armor: bool, received_override: u16) -> Option<f64> {
        let Kind::Holdable(holdable) = item.kind else { return None };
        let index = usize::from(armor);
        let vars = HashMap::from([
            ("item_level", f64::from(item.level)),
            ("item_grade", f64::from(self.grades.get(&grade)?[index])),
        ]);
        let value = self.holdables.get(&holdable)?[index].as_ref()?.eval_native(&vars)?;
        // FST retains the evaluated x87 value when no positive override replaces it.
        Some(if received_override > 0 {
            f64::from(value as f32 * f32::from(received_override) * 0.01f32)
        } else if armor {
            f64::from(value as f32)
        } else {
            value
        })
    }

    pub(super) fn bases(&self, game: &crate::hud::data::GameData, equipment: &Equipment) -> Option<[i32; 3]> {
        let mut out = [0i32; 3]; // armor8, resistance64, mainhand-DPS96
        for slot in (0..=18).chain(std::iter::once(27)) {
            let Some(item) = equipment.slots[slot].as_ref() else { continue };
            if !super::super::eligible(game, item.template_id, slot) {
                continue;
            }
            let detail = match &item.detail {
                ItemDetail::Equip(detail) => Some(detail),
                _ => None,
            };
            if detail.is_none_or(|detail| detail.durability == 0) && game.tips.items.durability(item.template_id, item.grade).is_some() {
                continue;
            }
            let base = self.items.get(&item.template_id)?;
            let physical = detail.map_or(0, |detail| detail.temper_physical);
            let magical = detail.map_or(0, |detail| detail.temper_magical);
            match base.kind {
                Kind::Wearable { .. } => {
                    if slot < 15 {
                        out[0] = out[0].wrapping_add(self.wearable(base, item.grade, false, physical)?);
                    }
                    if slot < 15 || slot == 27 {
                        out[1] = out[1].wrapping_add(self.wearable(base, item.grade, true, magical)?);
                    }
                }
                Kind::Holdable(_) if (15..=18).contains(&slot) => {
                    out[0] = out[0].wrapping_add(trunc64(self.holdable(base, item.grade, true, physical)?));
                    if slot == 15 {
                        out[2] = trunc64(self.holdable(base, item.grade, false, physical)? * 1000.);
                    }
                }
                _ => {}
            }
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests;

fn trunc64(value: f64) -> i32 {
    if !(-2147483648.0..2147483648.0).contains(&value) {
        i32::MIN
    } else {
        value as i32
    }
}
