//! Emotes: what the client knows about the `express_texts` rows (`CSExpressEmotion` / `SCEmotionExpressed`).
//!
//! An emotion id is an `express_texts` row. Its `anim_id` is the clip every unit in range plays when the
//! server announces the emote (`combat::apply`, `Event::Emote`), its `me` / `me_target` / `other` /
//! `other_target` / `other_me` are the chat lines (`hud::speech`). The emote skills of the skill book's
//! emote tab (`default_skills` category 5, named `/sad`, `/clap` ...) are the same emotes: a skill's
//! clip (its `fire_anim_id`, or the pose of its channeling buff: `/bow`, `/kneel`) is the `anim_id` of
//! an `express_texts` row, which is how a skill (or the slash command of its name) finds its emotion.
//!
//! The server's `CSExpressEmotion` handler only broadcasts: nothing is checked or cooled down.

use std::collections::HashMap;

use bevy::prelude::*;
use rusqlite::Connection;

/// The `express_texts` columns that hold chat lines, in the order of [`Emote::lines`].
const LINE_COLUMNS: [&str; 5] = ["me", "me_target", "other", "other_target", "other_me"];

/// One `express_texts` row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Emote {
    /// `anims.id` of the clip (0: none).
    pub anim_id: u32,
    /// English chat lines: `me`, `me_target`, `other`, `other_target`, `other_me`; empty when the row has none.
    pub lines: [String; 5],
}

/// The emotes of the game database.
#[derive(Resource, Clone, Debug, Default)]
pub struct Emotes {
    emotes: HashMap<u32, Emote>,
    /// Slash command (lower case, no slash: `sad`) -> emotion id.
    commands: HashMap<String, u32>,
    /// Emote skill id -> emotion id.
    skills: HashMap<u32, u32>,
}

impl Emotes {
    /// Reads the tables; a missing table leaves the part empty.
    pub fn load(conn: &Connection) -> rusqlite::Result<Self> {
        let mut out = Emotes::default();
        // English lines by (row id, column)
        let mut texts: HashMap<(u32, String), String> = HashMap::new();
        if let Ok(mut q) = conn.prepare("SELECT idx, tbl_column_name, en_us FROM localized_texts WHERE tbl_name = 'express_texts' AND en_us IS NOT NULL AND en_us <> ''") {
            for row in q.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?.flatten() {
                texts.insert((row.0 as u32, row.1), row.2);
            }
        }
        let mut q = conn.prepare("SELECT id, anim_id FROM express_texts ORDER BY id")?;
        for row in q.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?.unwrap_or(0))))?.flatten() {
            let id = row.0 as u32;
            // (the first row of an id wins, as the server's `ExpressTexts::load`)
            out.emotes.entry(id).or_insert_with(|| Emote {
                anim_id: row.1 as u32,
                lines: LINE_COLUMNS.map(|c| texts.get(&(id, c.to_string())).cloned().unwrap_or_default()),
            });
        }
        // the emote skills: the clip of the skill, else the pose of its channeling buff
        let Ok(mut q) = conn.prepare(
            "SELECT s.id, coalesce(nullif(s.fire_anim_id, 0), b.anim_start_id, 0), coalesce(l.en_us, '') \
             FROM default_skills d JOIN skills s ON s.id = d.skill_id \
             LEFT JOIN buffs b ON b.id = s.channeling_buff_id \
             LEFT JOIN localized_texts l ON l.tbl_name = 'skills' AND l.tbl_column_name = 'name' AND l.idx = s.id \
             WHERE d.skill_book_category_id = 5",
        ) else {
            return Ok(out);
        };
        let rows: Vec<(u32, u32, String)> =
            q.query_map([], |r| Ok((r.get::<_, i64>(0)? as u32, r.get::<_, Option<i64>>(1)?.unwrap_or(0) as u32, r.get::<_, String>(2)?)))?.flatten().collect();
        for (skill, anim, name) in rows {
            // the row with chat lines first (the coin exchanges share the clips of the plain emotes)
            let emotion = out.emotes.iter().filter(|(_, e)| anim != 0 && e.anim_id == anim).min_by_key(|(id, e)| (e.lines[0].is_empty(), **id)).map(|(id, _)| *id);
            let Some(emotion) = emotion else { continue };
            out.skills.insert(skill, emotion);
            if let Some(command) = name.strip_prefix('/').filter(|c| !c.is_empty()) {
                out.commands.entry(command.to_lowercase()).or_insert(emotion);
            }
        }
        Ok(out)
    }

    /// `AA_DB` (or the default database); empty when it cannot be read.
    pub fn from_env() -> Self {
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let conn = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI);
        match conn.map_err(|e| e.to_string()).and_then(|c| Self::load(&c).map_err(|e| e.to_string())) {
            Ok(e) => {
                info!("emotes: {} emotions, {} slash commands, {} emote skills", e.emotes.len(), e.commands.len(), e.skills.len());
                e
            }
            Err(e) => {
                warn!("emotes: {path}: {e}");
                Emotes::default()
            }
        }
    }

    /// The emotion of a slash command (`/Sad`, `sad`).
    pub fn command(&self, text: &str) -> Option<u32> {
        self.commands.get(text.trim().trim_start_matches('/').to_lowercase().as_str()).copied()
    }

    /// The emotion an emote skill expresses.
    pub fn skill(&self, skill_id: u32) -> Option<u32> {
        self.skills.get(&skill_id).copied()
    }

    /// `anims.id` of an emotion's clip.
    pub fn anim_id(&self, emotion: u32) -> Option<u32> {
        self.emotes.get(&emotion).map(|e| e.anim_id).filter(|a| *a != 0)
    }

    /// The chat line of an emotion for the one who reads it: `sender_is_me` / `target_is_me` say who is who, `has_target` whether a
    /// target was named. Empty when the row has no such line. `@SOURCE_NAME(0)` and `@TARGET_NAME(0)` are left in.
    pub fn line(&self, emotion: u32, sender_is_me: bool, target_is_me: bool, has_target: bool) -> &str {
        let Some(e) = self.emotes.get(&emotion) else { return "" };
        let index = if sender_is_me {
            usize::from(has_target)
        } else if target_is_me {
            4
        } else if has_target {
            3
        } else {
            2
        };
        &e.lines[index]
    }
}

#[cfg(test)]
impl Emotes {
    /// Tests that read the real database skip when it is not there.
    pub(crate) fn is_empty_for_tests(&self) -> bool {
        self.emotes.is_empty()
    }
}

/// The chat line with the names filled in.
pub fn fill(line: &str, source: &str, target: &str) -> String {
    line.replace("@SOURCE_NAME(0)", source).replace("@TARGET_NAME(0)", target)
}

pub fn plugin(app: &mut App) {
    app.insert_resource(Emotes::from_env());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE express_texts (id INT, anim_id INT, name TEXT);
             CREATE TABLE localized_texts (idx INT, tbl_name TEXT, tbl_column_name TEXT, en_us TEXT);
             CREATE TABLE default_skills (skill_id INT, skill_book_category_id INT);
             CREATE TABLE skills (id INT, fire_anim_id INT, channeling_buff_id INT);
             CREATE TABLE buffs (id INT, anim_start_id INT);
             INSERT INTO express_texts VALUES (3, 66, ''), (20, 124, ''), (37, 66, ''), (3, 999, 'dup');
             INSERT INTO localized_texts VALUES (3, 'express_texts', 'me', 'You are sad.'), (3, 'express_texts', 'other', '@SOURCE_NAME(0) is sad.'),
               (3, 'express_texts', 'other_me', '@SOURCE_NAME(0) is sad at you.'), (3, 'express_texts', 'me_target', 'You are sad at @TARGET_NAME(0).'),
               (14453, 'skills', 'name', '/Sad'), (14471, 'skills', 'name', '/bow');
             INSERT INTO default_skills VALUES (14453, 5), (14471, 5), (1, 2), (2, 5);
             INSERT INTO skills VALUES (14453, 66, NULL), (14471, NULL, 4636), (1, 66, NULL), (2, NULL, NULL);
             INSERT INTO buffs VALUES (4636, 124);",
        )
        .unwrap();
        c
    }

    #[test]
    fn skills_and_commands_find_their_emotion() {
        let e = Emotes::load(&db()).unwrap();
        // the first row of an id wins; the row with a chat line is preferred over the other row of the same clip
        assert_eq!(e.anim_id(3), Some(66));
        assert_eq!(e.skill(14453), Some(3));
        // a pose of the channeling buff is the clip of /bow
        assert_eq!(e.skill(14471), Some(20));
        // not an emote skill (category 2), and an emote skill without a clip
        assert_eq!(e.skill(1), None);
        assert_eq!(e.skill(2), None);
        assert_eq!(e.command("/Sad"), Some(3));
        assert_eq!(e.command("bow"), Some(20));
        assert_eq!(e.command("/nothing"), None);
        assert_eq!(e.anim_id(99), None);
    }

    #[test]
    fn the_line_depends_on_who_reads_it() {
        let e = Emotes::load(&db()).unwrap();
        assert_eq!(e.line(3, true, false, false), "You are sad.");
        assert_eq!(e.line(3, true, false, true), "You are sad at @TARGET_NAME(0).");
        assert_eq!(e.line(3, false, false, false), "@SOURCE_NAME(0) is sad.");
        assert_eq!(e.line(3, false, true, true), "@SOURCE_NAME(0) is sad at you.");
        assert_eq!(e.line(3, false, false, true), "");
        assert_eq!(fill(e.line(3, false, false, false), "Ann", ""), "Ann is sad.");
    }
}
