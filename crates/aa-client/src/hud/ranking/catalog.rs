//! Original ranking descriptors; compact-copy link order is reconciled to captured native vectors.
use rusqlite::Connection;
use std::collections::HashMap;

#[derive(Debug)]
pub(super) struct Entry {
    pub id: u32,
    pub name: String,
    pub v1: String,
    pub v2: String,
    pub start: [i32; 5],
    pub end: [i32; 5],
    pub weekday: i32,
    pub start_hour: i32,
    pub end_hour: i32,
    pub reset_week: i32,
    pub kind: i32,
    pub start_alarm: String,
    pub end_alarm: String,
    pub scope_ids: Vec<u32>,
    pub reward_links: Vec<(u32, u32)>,
}

#[derive(Debug)]
pub(super) struct Scope {
    pub name: String,
    pub from: i32,
    pub to: i32,
}

#[derive(Debug)]
pub(super) struct Reward {
    pub name: String,
    pub appellation_id: u32,
    pub item_id: u32,
    pub item_grade: u8,
    pub item_count: i32,
}

#[derive(Debug)]
pub(super) struct Appellation {
    pub name: String,
    pub buff_id: u32,
}

#[derive(Default)]
pub(crate) struct Tables {
    pub(super) rows: Vec<Entry>,
    pub(super) scopes: HashMap<u32, Scope>,
    pub(super) rewards: HashMap<u32, Reward>,
    pub(super) appellations: HashMap<u32, Appellation>,
}

impl Tables {
    pub(super) fn get(&self, id: u32) -> Option<&Entry> {
        self.rows.iter().find(|entry| entry.id == id)
    }

    pub(crate) fn load(db: &Connection) -> rusqlite::Result<Self> {
        use super::super::data::any_int;
        let mut out = Self::default();
        let mut localized = HashMap::new();
        let mut query = db.prepare("SELECT tbl_name,idx,tbl_column_name,en_us FROM localized_texts WHERE tbl_name IN ('ranks','rank_scopes','rank_rewards','appellations') AND en_us IS NOT NULL AND en_us<>''")?;
        for row in query.query_map([], |row| {
            Ok((
                (row.get::<_, String>(0)?, any_int(row, 1)? as u32, row.get::<_, String>(2)?),
                row.get::<_, String>(3)?,
            ))
        })? {
            let (key, text) = row?;
            localized.insert(key, text);
        }
        let name = |table: &str, id: u32, column: &str, raw: Option<String>| {
            localized
                .get(&(table.to_owned(), id, column.to_owned()))
                .cloned()
                .or(raw)
                .unwrap_or_default()
        };
        //39836b90 has no ORDER BY. For this catalog its loaded native IDs are [1,2,3,4].
        let mut query = db.prepare("SELECT id,day_of_week_id,ed_day,ed_hour,ed_min,ed_month,ed_year,end_time,end_time_alarm,end_time_alarm_msg,name,rank_kind_id,reset_week,st_day,st_hour,st_min,st_month,st_year,start_time,start_time_alarm,start_time_alarm_msg,v1,v2,zone_group_id FROM ranks")?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            let id = any_int(row, 0)? as u32;
            out.rows.push(Entry {
                id,
                name: name("ranks", id, "name", row.get(10)?),
                v1: name("ranks", id, "v1", row.get(21)?),
                v2: name("ranks", id, "v2", row.get(22)?),
                start: [
                    any_int(row, 17)? as i32,
                    any_int(row, 16)? as i32,
                    any_int(row, 13)? as i32,
                    any_int(row, 14)? as i32,
                    any_int(row, 15)? as i32,
                ],
                end: [
                    any_int(row, 6)? as i32,
                    any_int(row, 5)? as i32,
                    any_int(row, 2)? as i32,
                    any_int(row, 3)? as i32,
                    any_int(row, 4)? as i32,
                ],
                weekday: any_int(row, 1)? as i32,
                start_hour: any_int(row, 18)? as i32,
                end_hour: any_int(row, 7)? as i32,
                reset_week: any_int(row, 12)? as i32,
                kind: any_int(row, 11)? as i32,
                start_alarm: name("ranks", id, "start_time_alarm_msg", row.get(20)?),
                end_alarm: name("ranks", id, "end_time_alarm_msg", row.get(9)?),
                scope_ids: Vec::new(),
                reward_links: Vec::new(),
            });
        }
        let mut query = db.prepare("SELECT id,name,scope_from,scope_to FROM rank_scopes")?;
        for row in query.query_map([], |row| {
            let id = any_int(row, 0)? as u32;
            Ok((
                id,
                Scope {
                    name: name("rank_scopes", id, "name", row.get(1)?),
                    from: any_int(row, 2)? as i32,
                    to: any_int(row, 3)? as i32,
                },
            ))
        })? {
            let (id, scope) = row?;
            out.scopes.insert(id, scope);
        }
        let mut query = db.prepare("SELECT id,appellation_id,item_count,item_grade_id,item_id,name,weeks FROM rank_rewards")?;
        for row in query.query_map([], |row| {
            let id = any_int(row, 0)? as u32;
            Ok((
                id,
                Reward {
                    name: name("rank_rewards", id, "name", row.get(5)?),
                    appellation_id: any_int(row, 1)? as u32,
                    item_id: any_int(row, 4)? as u32,
                    item_grade: any_int(row, 3)? as u8,
                    item_count: any_int(row, 2)? as i32,
                },
            ))
        })? {
            let (id, reward) = row?;
            out.rewards.insert(id, reward);
        }
        let mut query = db.prepare("SELECT id,name,buff_id FROM appellations")?;
        for row in query.query_map([], |row| {
            let id = any_int(row, 0)? as u32;
            Ok((
                id,
                Appellation {
                    name: name("appellations", id, "name", row.get(1)?),
                    buff_id: any_int(row, 2)? as u32,
                },
            ))
        })? {
            let (id, appellation) = row?;
            out.appellations.insert(id, appellation);
        }
        // Native association39836f40/fb0 appends in original loaded-link order.
        // The compact copy differs. ORDER BY id restores the captured original 1.2
        // parent vectors for this catalog; this is not attributed to native SQL.
        let mut query = db.prepare("SELECT id,rank_scope_id,rank_id FROM rank_scope_links ORDER BY id")?;
        for row in query.query_map([], |row| Ok((any_int(row, 1)? as u32, any_int(row, 2)? as u32)))? {
            let (scope, rank) = row?;
            if let Some(entry) = out.rows.iter_mut().find(|entry| entry.id == rank) {
                entry.scope_ids.push(scope);
            }
        }
        let mut query = db.prepare("SELECT id,rank_reward_id,rank_scope_id,rank_id FROM rank_reward_links ORDER BY id")?;
        for row in query.query_map([], |row| {
            Ok((any_int(row, 1)? as u32, any_int(row, 2)? as u32, any_int(row, 3)? as u32))
        })? {
            let (reward, scope, rank) = row?;
            if let Some(entry) = out.rows.iter_mut().find(|entry| entry.id == rank) {
                entry.reward_links.push((scope, reward));
            }
        }
        Ok(out)
    }
}
