//! Game database lookups the in-game world needs: which model an NPC / doodad template draws.
//!
//! The database is the server's plain SQLite copy (`compact.sqlite3`, read only). Results are
//! cached: a town has a few dozen distinct templates and hundreds of instances.

use std::collections::HashMap;
use std::sync::Mutex;

use aa_anim::GameDb;
use bevy::prelude::*;
use rusqlite::{Connection, OpenFlags, params};

/// The database handles (a connection is `Send` but not `Sync`, hence the mutexes).
#[derive(Resource)]
pub struct Db {
    pub game: Mutex<GameDb>,
    lookups: Mutex<Lookups>,
}

struct Lookups {
    conn: Connection,
    actor_files: HashMap<u32, Option<String>>,
    doodad_models: HashMap<u32, Option<String>>,
    npc_scale: HashMap<u32, f32>,
}

impl Db {
    pub fn open() -> Option<Db> {
        let game = match GameDb::open() {
            Ok(g) => g,
            Err(e) => {
                error!("game database: {e}");
                return None;
            }
        };
        let path = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let conn = match Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI) {
            Ok(c) => c,
            Err(e) => {
                error!("game database {path}: {e}");
                return None;
            }
        };
        Some(Db {
            game: Mutex::new(game),
            lookups: Mutex::new(Lookups {
                conn,
                actor_files: HashMap::new(),
                doodad_models: HashMap::new(),
                npc_scale: HashMap::new(),
            }),
        })
    }

    /// The model file (`.cdf` / `.chr`) of an actor model (`models.id`), lower case, `/` separators.
    pub fn actor_model_file(&self, model_id: u32) -> Option<String> {
        let mut l = self.lookups.lock().unwrap();
        if let Some(f) = l.actor_files.get(&model_id) {
            return f.clone();
        }
        let file: Option<String> = l
            .conn
            .query_row(
                "SELECT a.model_file FROM models m JOIN actor_models a ON a.id = m.sub_id AND m.sub_type = 'ActorModel' WHERE m.id = ?1",
                params![model_id],
                |r| r.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten()
            .map(|f| norm(&f))
            .filter(|f| !f.is_empty());
        l.actor_files.insert(model_id, file.clone());
        file
    }

    /// `npcs.scale` of an NPC template (1 when unknown).
    pub fn npc_scale(&self, template_id: u32) -> f32 {
        let mut l = self.lookups.lock().unwrap();
        if let Some(s) = l.npc_scale.get(&template_id) {
            return *s;
        }
        let scale: f32 = l
            .conn
            .query_row("SELECT scale FROM npcs WHERE id = ?1", params![template_id], |r| r.get::<_, Option<f64>>(0))
            .ok()
            .flatten()
            .map(|s| s as f32)
            .filter(|s| *s > 0.0)
            .unwrap_or(1.0);
        l.npc_scale.insert(template_id, scale);
        scale
    }

    /// The `.cgf` a doodad template is drawn with (`doodad_almighties.model` is `cgf://game/<path>`).
    pub fn doodad_model(&self, template_id: u32) -> Option<String> {
        let mut l = self.lookups.lock().unwrap();
        if let Some(m) = l.doodad_models.get(&template_id) {
            return m.clone();
        }
        let model: Option<String> = l
            .conn
            .query_row("SELECT model FROM doodad_almighties WHERE id = ?1", params![template_id], |r| r.get::<_, Option<String>>(0))
            .ok()
            .flatten()
            .map(|m| {
                let m = norm(&m);
                let m = m.strip_prefix("cgf://").unwrap_or(&m);
                m.strip_prefix("game/").unwrap_or(m).to_string()
            })
            .filter(|m| !m.is_empty() && m.contains('.'));
        l.doodad_models.insert(template_id, model.clone());
        model
    }
}

/// Lower case with `/` separators, as the pak wants paths.
pub fn norm(p: &str) -> String {
    p.replace('\\', "/").to_ascii_lowercase()
}
