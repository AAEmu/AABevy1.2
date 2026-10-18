//! Small immutable lookup needed before the asynchronous HUD database finishes.
use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

use rusqlite::{Connection, OpenFlags};

/// A successful complete catalog is sparse: an absent item means known no buff.
/// A failed read stays unavailable, so both consumers preserve unknown records.
pub(crate) fn item_recharge_catalog() -> Option<Arc<HashMap<u32, u32>>> {
    static CATALOG: OnceLock<Option<Arc<HashMap<u32, u32>>>> = OnceLock::new();
    CATALOG
        .get_or_init(|| {
            let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_owned());
            let result = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
                .and_then(|conn| load(&conn));
            match result {
                Ok(catalog) => Some(Arc::new(catalog)),
                Err(error) => {
                    bevy::log::warn!("aa-client: item recharge catalog unavailable: {error}");
                    None
                }
            }
        })
        .clone()
}

fn load(conn: &Connection) -> rusqlite::Result<HashMap<u32, u32>> {
    // ItemDesc chooses its class using items.impl_id. Child-only/orphan rows are
    // not descriptors; all three class tables must be readable even if empty.
    let mut query = conn.prepare(
        "SELECT i.id, COALESCE(CASE i.impl_id \
         WHEN 1 THEN w.recharge_buff_id WHEN 2 THEN a.recharge_buff_id \
         WHEN 24 THEN x.recharge_buff_id END, 0) AS buff \
         FROM items i \
         LEFT JOIN item_weapons w ON i.impl_id=1 AND w.item_id=i.id \
         LEFT JOIN item_armors a ON i.impl_id=2 AND a.item_id=i.id \
         LEFT JOIN item_accessories x ON i.impl_id=24 AND x.item_id=i.id \
         WHERE buff <> 0",
    )?;
    let mut catalog = HashMap::new();
    let mut rows = query.query([])?;
    while let Some(row) = rows.next()? {
        catalog.insert(row.get(0)?, row.get(1)?);
    }
    Ok(catalog)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE items(id INTEGER PRIMARY KEY, impl_id INTEGER); \
             CREATE TABLE item_weapons(item_id INTEGER PRIMARY KEY, recharge_buff_id INTEGER); \
             CREATE TABLE item_armors(item_id INTEGER PRIMARY KEY, recharge_buff_id INTEGER); \
             CREATE TABLE item_accessories(item_id INTEGER PRIMARY KEY, recharge_buff_id INTEGER);",
        )
        .unwrap();
        conn
    }

    #[test]
    fn complete_catalog_uses_matching_item_class_and_excludes_orphans() {
        let conn = database();
        conn.execute_batch(
            "INSERT INTO items VALUES (1,1),(2,2),(3,24),(4,15),(5,1),(6,1),(7,1),(8,2); \
             INSERT INTO item_weapons VALUES (1,101),(2,999),(4,888),(5,NULL),(6,0),(99,777); \
             INSERT INTO item_armors VALUES (1,999),(2,202),(7,999),(8,NULL); \
             INSERT INTO item_accessories VALUES (3,303);",
        )
        .unwrap();
        assert_eq!(load(&conn).unwrap(), HashMap::from([(1, 101), (2, 202), (3, 303)]));
    }

    #[test]
    fn an_empty_complete_catalog_differs_from_a_missing_table() {
        let conn = database();
        assert!(load(&conn).unwrap().is_empty());
        conn.execute_batch("DROP TABLE item_accessories;").unwrap();
        assert!(load(&conn).is_err());
    }

    #[test]
    fn malformed_nonzero_values_refuse_the_entire_catalog() {
        let conn = database();
        conn.execute_batch(
            "INSERT INTO items VALUES (1,1),(2,2); \
             INSERT INTO item_weapons VALUES (1,101); \
             INSERT INTO item_armors VALUES (2,-1);",
        )
        .unwrap();
        assert!(load(&conn).is_err());
    }
}
