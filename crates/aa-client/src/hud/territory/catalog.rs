//! Catalog inputs only; ownership, active buffs and loaded building counts are received.
use super::super::data::any_int;
use rusqlite::Connection;

#[derive(Default)]
pub(in crate::hud) struct Tables {
    pub(super) taxes: Vec<Tax>,
    pub(super) steps: Vec<Step>,
}
pub(super) struct Tax {
    pub desc: String,
    pub amount: i32,
}
pub(super) struct Step {
    pub setting: u32,
    pub buff: u32,
    pub step: i8,
    pub gates: i8,
    pub walls: i8,
}
impl Tables {
    pub(in crate::hud) fn load(conn: &Connection) -> rusqlite::Result<Self> {
        let mut out = Self::default();
        // Native39676490 enumerates this exact SQL without sorting. The separate
        // ID index does not reorder the raw array consumed by GetTaxations.
        let mut names = std::collections::HashMap::new();
        let mut localized = conn.prepare("SELECT idx,en_us FROM localized_texts WHERE tbl_name='taxations' AND tbl_column_name='desc' AND en_us IS NOT NULL AND en_us<>''")?;
        for row in localized.query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, String>(1)?)))? {
            let (id, name) = row?;
            names.insert(id, name);
        }
        let mut query = conn.prepare("SELECT id, desc, show, tax FROM taxations")?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            if shown(conn, row)? {
                let id: u32 = row.get(0)?;
                let desc = names
                    .remove(&id)
                    .or(row.get::<_, Option<String>>(1)?)
                    .unwrap_or_default();
                out.taxes.push(Tax {
                    amount: any_int(row, 3)? as i32,
                    desc,
                });
            }
        }
        let mut query=conn.prepare("SELECT id, buff_id, guard_tower_setting_id, num_gates, num_walls, step FROM guard_tower_steps ORDER BY guard_tower_setting_id ASC, step ASC")?;
        let mut rows = query.query([])?;
        while let Some(row) = rows.next()? {
            out.steps.push(Step {
                buff: any_int(row, 1)? as u32,
                setting: any_int(row, 2)? as u32,
                gates: any_int(row, 3)? as i8,
                walls: any_int(row, 4)? as i8,
                step: any_int(row, 5)? as i8,
            });
        }
        Ok(out)
    }
}

/// Original loader tests the first byte returned by sqlite3_column_text, without
/// trimming. Keep SQLite's numeric spelling (including exponent notation).
fn shown(conn: &Connection, row: &rusqlite::Row) -> rusqlite::Result<bool> {
    use rusqlite::types::{Value, ValueRef};
    let number = match row.get_ref(2)? {
        ValueRef::Null => return Ok(false),
        ValueRef::Text(bytes) | ValueRef::Blob(bytes) => {
            return Ok(matches!(bytes.first(), Some(b'T' | b't' | b'1')));
        }
        ValueRef::Integer(value) => Value::Integer(value),
        ValueRef::Real(value) => Value::Real(value),
    };
    let first = conn.query_row("SELECT CAST(?1 AS TEXT)", [number], |row| {
        Ok(match row.get_ref(0)? {
            ValueRef::Text(bytes) => bytes.first().copied(),
            _ => None,
        })
    })?;
    Ok(matches!(first, Some(b'T' | b't' | b'1')))
}

/// The original GetTaxations expression has an f32 round after each operation.
pub(super) fn preview(base: i32, rate: i32) -> i32 {
    let total = base as f32 + (rate as f32 * base as f32) * 0.01_f32;
    if total.is_finite() && (-2147483648.0..2147483648.0).contains(&total) {
        total as i32
    } else {
        i32::MIN
    }
}
