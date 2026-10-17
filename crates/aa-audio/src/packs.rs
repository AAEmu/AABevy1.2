//! Sound packs: the game's indirection from a name to an event path.
//!
//! The UI (`PlayUISound`, `PlayMusic`, `SetSounds`), animation `sound_pack` events and the FX voices
//! do not name events; they name an item of a pack (`global`: `login_stage_try_login`, `window`:
//! `bag_show`, `soundmood_w_gweonid_forest`: `music_01`, ...) and the database maps pack, item and
//! sound id to the event path (tables `sound_packs`, `sound_pack_items`, `sounds`).

use std::collections::HashMap;
use std::path::Path;

use rusqlite::{Connection, OpenFlags};

/// Pack name and item name to event path, both case-insensitive.
#[derive(Default, Debug, Clone)]
pub struct SoundPacks {
    items: HashMap<(String, String), String>,
    sounds: HashMap<i64, String>,
    packs: HashMap<String, usize>,
}

impl SoundPacks {
    /// Reads the game database (`compact.sqlite3`) read-only.
    pub fn open(db: impl AsRef<Path>) -> rusqlite::Result<SoundPacks> {
        let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
        SoundPacks::from_connection(&conn)
    }

    pub fn from_connection(conn: &Connection) -> rusqlite::Result<SoundPacks> {
        let mut packs = SoundPacks::default();
        let mut sounds = conn.prepare("SELECT id, path FROM sounds WHERE path IS NOT NULL AND path != ''")?;
        for row in sounds.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
            let (id, path) = row?;
            packs.sounds.insert(id, path);
        }
        let mut items = conn.prepare(
            "SELECT p.name, i.name, s.path FROM sound_pack_items i \
             JOIN sound_packs p ON p.id = i.sound_pack_id \
             JOIN sounds s ON s.id = i.sound_id \
             WHERE s.path IS NOT NULL AND s.path != '' ORDER BY i.id",
        )?;
        for row in items.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))? {
            let (pack, item, path) = row?;
            let pack = pack.to_ascii_lowercase();
            *packs.packs.entry(pack.clone()).or_default() += 1;
            packs.items.entry((pack, item.to_ascii_lowercase())).or_insert(path);
        }
        Ok(packs)
    }

    /// The event path of `item` in `pack`; `None` for unknown names and for items without a sound.
    pub fn path(&self, pack: &str, item: &str) -> Option<&str> {
        self.items.get(&(pack.to_ascii_lowercase(), item.to_ascii_lowercase())).map(String::as_str)
    }

    /// A UI sound or music name (`PlayUISound`, `PlayMusic`): an item of the `global` pack.
    pub fn ui(&self, name: &str) -> Option<&str> {
        self.path("global", name)
    }

    /// The event path of a sound id (`sounds.id`: items, doodads, quests, FX ...).
    pub fn sound(&self, id: i64) -> Option<&str> {
        self.sounds.get(&id).map(String::as_str)
    }

    /// Number of items with a sound in `pack`.
    pub fn len_of(&self, pack: &str) -> usize {
        self.packs.get(&pack.to_ascii_lowercase()).copied().unwrap_or(0)
    }

    /// Number of (pack, item) pairs with a sound.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_pack_items_to_paths() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE sounds (id INT, name TEXT, path TEXT);
             CREATE TABLE sound_packs (id INT, name TEXT);
             CREATE TABLE sound_pack_items (id INT, sound_pack_id INT, name TEXT, sound_id INT);
             INSERT INTO sounds VALUES (816, 'a', 'sounds/x2interface:login_stage:button_login'), (9, 'b', ''), (5, 'c', 'Sounds/x2music:screen:login_2013_07');
             INSERT INTO sound_packs VALUES (203, 'global'), (410, 'please_remove');
             INSERT INTO sound_pack_items VALUES
               (1, 203, 'login_stage_try_login', 816),
               (2, 203, 'login_stage_music_before_login', 5),
               (3, 203, 'empty_one', 9),
               (4, 203, 'no_sound', NULL),
               (5, 410, 'login_stage_try_login', 5);",
        )
        .unwrap();
        let p = SoundPacks::from_connection(&conn).unwrap();
        assert_eq!(p.ui("LOGIN_STAGE_TRY_LOGIN"), Some("sounds/x2interface:login_stage:button_login"));
        assert_eq!(p.path("please_remove", "login_stage_try_login"), Some("Sounds/x2music:screen:login_2013_07"));
        assert_eq!(p.ui("empty_one"), None);
        assert_eq!(p.ui("no_sound"), None);
        assert_eq!(p.ui("unknown"), None);
        assert_eq!(p.sound(5), Some("Sounds/x2music:screen:login_2013_07"));
        assert_eq!(p.len_of("global"), 2);
        assert_eq!(p.len(), 3);
    }
}
