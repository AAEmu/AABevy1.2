//! Static native buff rules shared by the HUD and the standalone skill book.
use aa_net::{buff::BuffKind, state::NativeBuffRule};
use rusqlite::{Connection, Row, types::Value};
use std::collections::HashMap;

// Preserve the existing catalog's mixed SQLite text/integer/null coercions.
fn int(row: &Row, i: usize) -> rusqlite::Result<i64> {
    Ok(match row.get::<_, Value>(i)? {
        Value::Integer(n) => n,
        Value::Real(f) => f as i64,
        Value::Text(t) => t.trim().parse().unwrap_or(0),
        _ => 0,
    })
}

fn flag(row: &Row, i: usize) -> rusqlite::Result<bool> {
    Ok(match row.get::<_, Value>(i)? {
        Value::Integer(n) => n != 0,
        Value::Text(t) => matches!(t.trim(), "t" | "T" | "1" | "true"),
        _ => false,
    })
}

pub(crate) fn load(db: &Connection) -> rusqlite::Result<HashMap<u32, NativeBuffRule>> {
    let mut out = HashMap::new();
    let mut query = db.prepare(
        "SELECT id,kind_id,per_unit_creation,stack_rule_id,max_stack,max_charge FROM buffs",
    )?;
    let mut rows = query.query([])?;
    while let Some(row) = rows.next()? {
        let id = int(row, 0)? as u32;
        let kind = match int(row, 1)? {
            1 => BuffKind::Good,
            2 => BuffKind::Bad,
            3 => BuffKind::Hidden,
            _ => BuffKind::Unknown,
        };
        out.insert(
            id,
            NativeBuffRule {
                kind,
                per_unit_creation: flag(row, 2)?,
                stack_rule_id: int(row, 3)? as u32,
                max_stack: int(row, 4)? as i32,
                max_charge: int(row, 5)? as i32,
            },
        );
    }
    Ok(out)
}
