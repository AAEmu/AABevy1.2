//! Native SQL row order, descriptors and objective metadata, loaded without progress predictions.
use aa_net::AchievementRule;
use rusqlite::Connection;
use std::collections::HashMap;

#[derive(Default, Debug)]
pub(crate) struct Entry {
    pub id: u32,
    pub name: String,
    pub summary: String,
    pub desc: String,
    pub main: i32,
    pub sub: i32,
    pub parent: u32,
    pub active: bool,
    pub hidden: bool,
    pub priority: i32,
    pub rule: AchievementRule,
    pub reward: u32,
    pub icon_id: u32,
    pub objectives: Vec<i32>,
}

#[derive(Default)]
pub(crate) struct Tables {
    ///39752a01 preserves sqlite_step order. The separate lookup index never reorders these rows.
    pub rows: Vec<Entry>,
    index: HashMap<u32, usize>,
    pub icons: HashMap<u32, String>,
}

impl Tables {
    pub fn get(&self, id: u32) -> Option<&Entry> {
        self.index.get(&id).map(|&index| &self.rows[index])
    }

    pub fn rules(&self) -> HashMap<u32, AchievementRule> {
        self.rows.iter().map(|entry| (entry.id, entry.rule)).collect()
    }

    pub fn load(db: &Connection) -> rusqlite::Result<Self> {
        use super::super::data::{any_flag, any_int};
        let mut out = Self::default();
        let mut names = HashMap::new();
        let mut q = db.prepare("SELECT idx,en_us FROM localized_texts WHERE tbl_name='achievements' AND tbl_column_name='name' AND en_us IS NOT NULL AND en_us<>''")?;
        for row in q.query_map([], |row| Ok((any_int(row, 0)? as u32, row.get::<_, String>(1)?)))? {
            let (id, name) = row?;
            names.insert(id, name);
        }
        let mut q = db.prepare("SELECT id,filename FROM icons")?;
        for row in q.query_map([], |row| {
            Ok((any_int(row, 0)? as u32, row.get::<_, Option<String>>(1)?.unwrap_or_default()))
        })? {
            let (id, filename) = row?;
            out.icons.insert(id, filename);
        }
        let mut q = db.prepare("SELECT id,name,summary,description,category_id,sub_category_id,parent_achievement_id,is_active,is_hidden,priority,complete_num,item_id,icon_id FROM achievements")?;
        let mut rows = q.query([])?;
        while let Some(row) = rows.next()? {
            let id = any_int(row, 0)? as u32;
            let entry = Entry {
                id,
                name: names.remove(&id).unwrap_or(row.get::<_, Option<String>>(1)?.unwrap_or_default()),
                summary: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                desc: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                main: any_int(row, 4)? as i32,
                sub: any_int(row, 5)? as i32,
                parent: any_int(row, 6)? as u32,
                active: any_flag(row, 7)?,
                hidden: any_flag(row, 8)?,
                priority: any_int(row, 9)? as i32,
                rule: AchievementRule {
                    complete_num: any_int(row, 10)? as i32,
                    objective_count: 0,
                },
                reward: any_int(row, 11)? as u32,
                icon_id: any_int(row, 12)? as u32,
                objectives: Vec::new(),
            };
            out.index.insert(id, out.rows.len());
            out.rows.push(entry);
        }
        let mut records = HashMap::new();
        let mut q = db.prepare("SELECT id,kind_id,value1 FROM char_records")?;
        for row in q.query_map([], |row| Ok((any_int(row, 0)? as u32, any_int(row, 1)?, any_int(row, 2)? as i32)))? {
            let (id, kind, value) = row?;
            records.insert(id, (kind, value));
        }
        let mut q = db.prepare("SELECT achievement_id,record_id FROM achievement_objectives")?;
        let mut rows = q.query([])?;
        while let Some(row) = rows.next()? {
            let id = any_int(row, 0)? as u32;
            let Some(&index) = out.index.get(&id) else { continue };
            let entry = &mut out.rows[index];
            // Missing/non-kind9 records still contribute to the native completion threshold.
            entry.rule.objective_count += 1;
            if let Some(&(9, value)) = records.get(&(any_int(row, 1)? as u32)) {
                entry.objectives.push(value);
            }
        }
        Ok(out)
    }
}
