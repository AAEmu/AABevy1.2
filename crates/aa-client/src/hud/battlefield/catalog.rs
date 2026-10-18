//! Original recruiting-NPC lookup and descending level bands, separate from queue receipts.
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub(in crate::hud) struct Tables {
    pub(super) npcs: HashMap<u32, u32>,
    pub(super) fields: HashMap<u32, Field>,
    pub(super) blocked_buffs: HashSet<u32>,
    pub(super) killstreak_skills: HashSet<u32>,
}

pub(in crate::hud) struct Field {
    pub(super) id: u32,
    pub(super) name: String,
    pub(super) desc: String,
    rules: Vec<Rule>,
}

pub(super) struct Rule {
    id: u32,
    minimum: i32,
    pub(super) size: i32,
    pub(super) factions: [u32; 2],
    pub(super) score: i32,
    pub(super) kills: i32,
    pub(super) target: i32,
}

impl Tables {
    pub(in crate::hud) fn load(db: &Connection) -> rusqlite::Result<Self> {
        let mut out = Self::default();
        let mut q = db.prepare("SELECT id,recruiting_battle_field_id FROM npcs WHERE recruiting_battle_field_id<>0")?;
        for row in q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))? {
            let (id, field) = row?;
            out.npcs.insert(id, field);
        }
        let mut texts = HashMap::new();
        let mut q = db.prepare("SELECT idx,tbl_column_name,en_us FROM localized_texts WHERE tbl_name='battle_fields' AND en_us IS NOT NULL AND en_us<>''")?;
        for row in q.query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))? {
            let (id, column, text) = row?;
            texts.insert((id, column), text);
        }
        let mut q = db.prepare("SELECT id,name,desc FROM battle_fields")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            let id = r.get(0)?;
            let name = texts.remove(&(id, "name".into())).or(r.get::<_, Option<String>>(1)?).unwrap_or_default();
            let desc = texts.remove(&(id, "desc".into())).or(r.get::<_, Option<String>>(2)?).unwrap_or_default();
            out.fields.insert(
                id,
                Field {
                    id,
                    name,
                    desc,
                    rules: Vec::new(),
                },
            );
        }
        let mut q = db.prepare("SELECT battle_field_id,level_min,corps_size,corps1_id,corps2_id,victory_score,victory_kill_count,victory_kill_corps1_head,id FROM game_rule_sets ORDER BY battle_field_id,level_min DESC")?;
        let mut rows = q.query([])?;
        while let Some(r) = rows.next()? {
            let id = super::super::data::any_int(r, 0)? as u32;
            if let Some(field) = out.fields.get_mut(&id) {
                let value = |i| super::super::data::any_int(r, i);
                field.rules.push(Rule {
                    id: value(8)? as u32,
                    minimum: value(1)? as i32,
                    size: value(2)? as i32,
                    factions: [value(3)? as u32, value(4)? as u32],
                    score: value(5)? as i32,
                    kills: value(6)? as i32,
                    target: value(7)? as i32,
                });
            }
        }
        out.blocked_buffs.extend([631, 2028, 3619, 3623, 3621, 4868, 6295, 4876]);
        let mut q = db.prepare("SELECT buff_id FROM tagged_buffs WHERE tag_id=1253")?;
        for row in q.query_map([], |r| r.get(0))? {
            out.blocked_buffs.insert(row?);
        }
        let mut q = db.prepare("SELECT skill_id FROM tagged_skills WHERE tag_id=204")?;
        for row in q.query_map([], |r| r.get(0))? {
            out.killstreak_skills.insert(row?);
        }
        Ok(out)
    }

    pub(in crate::hud) fn field(&self, npc: u32) -> Option<&Field> {
        self.fields.get(self.npcs.get(&npc)?)
    }

    pub(super) fn rule(&self, id: u32) -> Option<(&Field, &Rule)> {
        self.fields
            .values()
            .find_map(|field| field.rules.iter().find(|rule| rule.id == id).map(|rule| (field, rule)))
    }
}

impl Field {
    pub(super) fn minimum(&self) -> i32 {
        self.rules.last().map_or(0, |rule| rule.minimum)
    }
    pub(super) fn rule(&self, level: i32) -> Option<&Rule> {
        self.rules.iter().find(|rule| rule.minimum <= level)
    }

    pub(super) fn band(&self, level: i32) -> (i32, i32) {
        let mut maximum = 55; // Original module-verified game level-cap byte.
        for rule in &self.rules {
            if rule.minimum <= level {
                return (rule.minimum, maximum);
            }
            maximum = rule.minimum.wrapping_sub(1);
        }
        (0, maximum)
    }
}
