//! Game database lookups the in-game world needs: which model an NPC / doodad template draws.
//!
//! The database is the server's plain SQLite copy (`compact.sqlite3`, read only). Results are
//! cached: a town has a few dozen distinct templates and hundreds of instances.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use aa_anim::GameDb;
use bevy::prelude::*;
use rusqlite::{Connection, OpenFlags, params};

/// The database handles (a connection is `Send` but not `Sync`, hence the mutexes). Cloning shares them, so worker
/// threads can use the same lookups and caches.
#[derive(Resource, Clone)]
pub struct Db {
    pub game: Arc<Mutex<GameDb>>,
    lookups: Arc<Mutex<Lookups>>,
}

struct Lookups {
    conn: Connection,
    actor_files: HashMap<u32, Option<String>>,
    doodad_models: HashMap<u32, Option<String>>,
    npc_scale: HashMap<u32, f32>,
    weapons: HashMap<u32, Option<Weapon>>,
    backpacks: HashMap<u32, Option<Weapon>>,
    anim_actions: HashMap<u32, Option<String>>,
    mount_poses: HashMap<u32, u32>,
}

/// How a weapon item is drawn (`item_weapons` + its `item_assets` row).
#[derive(Clone, Debug)]
pub struct Weapon {
    pub path: String,
    /// `holdables.code`: `bow`, `shield`, `normal_piercing_sod`, ...
    pub code: String,
    /// Offset in the attachment bone's frame (m) and rotation (degrees about x, y, z).
    pub offset: [f32; 3],
    pub rotation: [f32; 3],
    /// Scale while it hangs on the body.
    pub worn_scale: f32,
    /// Scale while it is in the hand.
    pub drawn_scale: f32,
    /// Held with both hands (`holdables.slot_type_id` 16: the two handed swords, axes, hammers, spears and staves).
    pub two_handed: bool,
    /// A staff (`holdables.pose_id` 4): held two handed but fights and casts in the one handed stance (the `staff_` clips are
    /// the one handed ones).
    pub staff: bool,
    /// `item_assets.allow_mirror`: the off hand copy is mirrored (x2game flips `pos_x`, `rot_y` and `rot_z`).
    pub mirror: bool,
}

/// Columns the in-game lookups filter or join on. The server's database has no indexes beyond the
/// primary keys, so `item -> armour meshes` (three joined tables) takes about 30 ms a call; a
/// private indexed copy makes it a lookup.
const INDEXES: &[(&str, &str)] = &[
    ("item_armors", "item_id"),
    ("item_armors", "asset_id"),
    ("item_armor_assets", "armor_asset_id"),
    ("item_armor_assets", "asset_id"),
    ("item_assets", "id"),
    ("item_assets", "model_id"),
    ("item_body_parts", "item_id"),
    ("item_body_parts", "model_id, slot_type_id"),
    ("hair_colors", "id"),
    ("hair_colors", "model_id, asset_id"),
    ("skin_colors", "id"),
    ("skin_colors", "model_id"),
    ("dyeable_items", "item_id"),
    ("item_dyeings", "item_id"),
    ("dyeing_colors", "id"),
    ("total_character_customs", "id"),
    ("total_character_customs", "model_id, owner_type_id"),
    ("face_decal_assets", "id"),
    ("face_normal_maps", "id"),
    ("models", "id"),
    ("actor_models", "id"),
    ("npcs", "id"),
    ("doodad_almighties", "id"),
    ("characters", "model_id"),
    ("equip_pack_cloths", "id"),
    ("item_weapons", "item_id"),
    ("holdables", "id"),
];

/// The database path to use: a copy of the server's database with the [`INDEXES`] added, kept under
/// `data/extract/aa-client/` and rebuilt when the original changes. Falls back to the original.
fn indexed_database(original: &str) -> String {
    // no file system in the browser: the database is used as it is
    if cfg!(target_arch = "wasm32") {
        return original.to_string();
    }
    let build = || -> Option<String> {
        let meta = std::fs::metadata(original).ok()?;
        let stamp = format!("{} {:?}", meta.len(), meta.modified().ok()?);
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/extract/aa-client");
        std::fs::create_dir_all(&dir).ok()?;
        let target = dir.join("compact-indexed.sqlite3");
        let marker = dir.join("compact-indexed.stamp");
        if target.exists() && std::fs::read_to_string(&marker).ok().as_deref() == Some(stamp.as_str()) {
            return Some(target.to_string_lossy().into_owned());
        }
        info!("building the indexed game database copy {} (once)", target.display());
        let tmp = dir.join(format!("compact-indexed.{}.tmp", std::process::id()));
        std::fs::copy(original, &tmp).ok()?;
        {
            let conn = Connection::open(&tmp).ok()?;
            for (i, (table, columns)) in INDEXES.iter().enumerate() {
                // tables or columns the database lacks are skipped
                let _ = conn.execute_batch(&format!("CREATE INDEX IF NOT EXISTS aa_client_idx_{i} ON {table}({columns})"));
            }
            let _ = conn.execute_batch("ANALYZE");
        }
        std::fs::rename(&tmp, &target).ok()?;
        std::fs::write(&marker, stamp).ok()?;
        Some(target.to_string_lossy().into_owned())
    };
    build().unwrap_or_else(|| original.to_string())
}

impl Db {
    pub fn open() -> Option<Db> {
        let original = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        let path = indexed_database(&original);
        let game = match GameDb::open_path(&path) {
            Ok(g) => g,
            Err(e) => {
                error!("game database: {e}");
                return None;
            }
        };
        let conn = match Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI) {
            Ok(c) => c,
            Err(e) => {
                error!("game database {path}: {e}");
                return None;
            }
        };
        Some(Db {
            game: Arc::new(Mutex::new(game)),
            lookups: Arc::new(Mutex::new(Lookups {
                conn,
                actor_files: HashMap::new(),
                doodad_models: HashMap::new(),
                npc_scale: HashMap::new(),
                weapons: HashMap::new(),
                backpacks: HashMap::new(),
                anim_actions: HashMap::new(),
                mount_poses: HashMap::new(),
            })),
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
            .filter(|f| !f.is_empty())
            .or_else(|| {
                // a prefab model (the training dummies, 46 NPCs): the `object_Model` of the prefab's animated object
                let prefab: Option<String> = l
                    .conn
                    .query_row(
                        "SELECT e.file_path FROM models m JOIN prefab_elements e ON e.prefab_model_id = m.sub_id AND m.sub_type = 'PrefabModel' WHERE m.id = ?1 ORDER BY e.state_id LIMIT 1",
                        params![model_id],
                        |r| r.get::<_, Option<String>>(0),
                    )
                    .ok()
                    .flatten();
                prefab.and_then(|p| prefab_model_file(&p))
            });
        l.actor_files.insert(model_id, file.clone());
        file
    }

    /// The clip (`.cal` entry name, lower case) of an animation action (`anim_actions.anim_name`): the idle pose of an NPC.
    pub fn anim_action_clip(&self, id: u32) -> Option<String> {
        let mut l = self.lookups.lock().unwrap();
        if let Some(c) = l.anim_actions.get(&id) {
            return c.clone();
        }
        let clip: Option<String> = l
            .conn
            .query_row("SELECT anim_name FROM anim_actions WHERE id = ?1", params![id], |r| r.get::<_, Option<String>>(0))
            .ok()
            .flatten()
            .map(|n| n.to_ascii_lowercase())
            .filter(|n| !n.is_empty() && n != "none");
        l.anim_actions.insert(id, clip.clone());
        clip
    }

    /// What a model is ridden like (`models.mount_pose_id`: 1 horse, 2 lion, 4 wildboar, 10 pangolin; 0 for none).
    pub fn mount_pose(&self, model_id: u32) -> u32 {
        let mut l = self.lookups.lock().unwrap();
        if let Some(p) = l.mount_poses.get(&model_id) {
            return *p;
        }
        let pose: u32 = l
            .conn
            .query_row("SELECT mount_pose_id FROM models WHERE id = ?1", params![model_id], |r| r.get::<_, Option<u32>>(0))
            .ok()
            .flatten()
            .unwrap_or(0);
        l.mount_poses.insert(model_id, pose);
        pose
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

    /// The model and attachment data of a weapon / shield item template.
    pub fn weapon(&self, item_id: u32) -> Option<Weapon> {
        let mut l = self.lookups.lock().unwrap();
        if let Some(w) = l.weapons.get(&item_id) {
            return w.clone();
        }
        let weapon = l
            .conn
            .query_row(
                "SELECT ia.path, h.code, ia.attachment_offset_pos_x, ia.attachment_offset_pos_y, ia.attachment_offset_pos_z,                  ia.attachment_offset_rot_x, ia.attachment_offset_rot_y, ia.attachment_offset_rot_z, w.worn_scale, w.drawn_scale, h.slot_type_id, h.pose_id, ia.allow_mirror                  FROM item_weapons w JOIN item_assets ia ON ia.id = w.asset_id JOIN holdables h ON h.id = w.holdable_id WHERE w.item_id = ?1",
                params![item_id],
                |r| {
                    let f = |i: usize| -> rusqlite::Result<f32> { Ok(r.get::<_, Option<f64>>(i)?.unwrap_or(0.0) as f32) };
                    Ok(Weapon {
                        path: norm(&r.get::<_, String>(0)?),
                        code: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                        offset: [f(2)?, f(3)?, f(4)?],
                        rotation: [f(5)?, f(6)?, f(7)?],
                        worn_scale: f(8)?,
                        drawn_scale: f(9)?,
                        two_handed: r.get::<_, Option<i64>>(10)?.unwrap_or(0) == 16,
                        staff: r.get::<_, Option<i64>>(11)?.unwrap_or(0) == 4,
                        mirror: matches!(r.get::<_, Option<String>>(12)?.as_deref(), Some("t" | "T" | "1")),
                    })
                },
            )
            .ok();
        l.weapons.insert(item_id, weapon.clone());
        weapon
    }

    /// The model a backpack / glider item template wears on the back (`item_backpacks`): its stowed look when it has one
    /// (`asset2_id`: a glider's folded `_back.cgf`), else `asset_id`. [`Weapon::code`] is `backpack`. A glider without a stowed look
    /// (`glider_bx.chr`, 5 items) is only its flying model, a 6 m canopy: nothing is drawn for it.
    pub fn backpack(&self, item_id: u32) -> Option<Weapon> {
        let mut l = self.lookups.lock().unwrap();
        if let Some(w) = l.backpacks.get(&item_id) {
            return w.clone();
        }
        let backpack = l
            .conn
            .query_row(
                "SELECT ia.path, ia.attachment_offset_pos_x, ia.attachment_offset_pos_y, ia.attachment_offset_pos_z, ia.attachment_offset_rot_x, ia.attachment_offset_rot_y, ia.attachment_offset_rot_z
                 FROM item_backpacks b JOIN item_assets ia ON ia.id = CASE WHEN b.asset2_id > 0 THEN b.asset2_id ELSE b.asset_id END WHERE b.item_id = ?1 AND NOT (b.backpack_type_id = 2 AND b.asset2_id = 0)",
                params![item_id],
                |r| {
                    let f = |i: usize| -> rusqlite::Result<f32> { Ok(r.get::<_, Option<f64>>(i)?.unwrap_or(0.0) as f32) };
                    Ok(Weapon {
                        path: norm(&r.get::<_, String>(0)?),
                        code: "backpack".into(),
                        offset: [f(1)?, f(2)?, f(3)?],
                        rotation: [f(4)?, f(5)?, f(6)?],
                        worn_scale: 1.0,
                        drawn_scale: 1.0,
                        two_handed: false,
                        staff: false,
                        mirror: false,
                    })
                },
            )
            .ok();
        l.backpacks.insert(item_id, backpack.clone());
        backpack
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

impl Db {
    /// The particle effects of a prefab model without a model of its own (the summoned portal gates, `models` 308 / 667:
    /// an invisible proxy brush and `ParticleEffect` entities): its first state's (`prefab_elements.state_id` 1, alive)
    /// effects, positions relative to the unit. Sounds (`x2_sounds.*`) are left out.
    pub fn prefab_effects(&self, model_id: u32) -> Vec<aa_world::placement::PlacedEffect> {
        let l = self.lookups.lock().unwrap();
        let path: Option<String> = l
            .conn
            .query_row(
                "SELECT e.file_path FROM models m JOIN prefab_elements e ON e.prefab_model_id = m.sub_id AND m.sub_type = 'PrefabModel' WHERE m.id = ?1 ORDER BY e.state_id LIMIT 1",
                params![model_id],
                |r| r.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten();
        drop(l);
        path.map(|p| prefab_effects_of(&p)).unwrap_or_default()
    }
}

/// The `ParticleEffect` entities (the `<export>` list) of a prefab element path.
fn prefab_effects_of(path: &str) -> Vec<aa_world::placement::PlacedEffect> {
    let Some((file, name)) = path.strip_prefix("prefab://").and_then(|r| r.rsplit_once('/')) else { return Vec::new() };
    let Ok(xml) = aa_pak::Pak::global().read_string(&norm(file)) else { return Vec::new() };
    let Some(from) = xml.find(&format!("<Prefab Name=\"{name}\"")) else { return Vec::new() };
    let tail = &xml[from..];
    let prefab = &tail[..tail.find("</Prefab>").unwrap_or(tail.len())];
    let Some(start) = prefab.find("<export>") else { return Vec::new() };
    let export = &prefab[start..prefab[start..].find("</export>").map_or(prefab.len(), |e| start + e + "</export>".len())];
    aa_world::placement::parse_effect_entities(export).into_iter().filter(|e| !e.effect.to_ascii_lowercase().starts_with("x2_sounds.")).collect()
}

/// The model file of a prefab element path (`prefab://prefabs/interaction_e.xml/scarecrow.learning_scarecrow_02`): the first `object_Model`
/// of that prefab in its xml file.
fn prefab_model_file(path: &str) -> Option<String> {
    let rest = path.strip_prefix("prefab://")?;
    let (file, name) = rest.rsplit_once('/')?;
    let xml = aa_pak::Pak::global().read_string(&norm(file)).ok()?;
    let from = xml.find(&format!("<Prefab Name=\"{name}\""))?;
    let tail = &xml[from..];
    let tail = &tail[..tail.find("</Prefab>").unwrap_or(tail.len())];
    let start = tail.find("object_Model=\"")? + "object_Model=\"".len();
    let model = &tail[start..start + tail[start..].find('"')?];
    Some(norm(model)).filter(|m| !m.is_empty())
}

/// Lower case with `/` separators, as the pak wants paths.
pub fn norm(p: &str) -> String {
    p.replace('\\', "/").to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The training dummies are prefab models: their model file is the `object_Model` of the prefab.
    #[test]
    fn a_portal_gate_prefab_is_its_particle_effects() {
        let e = prefab_effects_of("prefab://Prefabs/fx.xml/portal_gate_dot");
        let names: Vec<&str> = e.iter().map(|e| e.effect.as_str()).collect();
        assert_eq!(names, ["abillity_skill_table_m.system_skill.portal_gate_dot3_indivisual", "abillity_skill_table_m.system_skill.portal_gate_hit"]);
        assert!((e[0].pos.z - 0.036621094).abs() < 1e-6);
        assert!(!prefab_effects_of("prefab://Prefabs/fx.xml/portal_gate_exit_dot").is_empty());
    }

    #[test]
    fn a_prefab_model_has_the_file_of_its_animated_object() {
        if !aa_cry::pak_available() {
            return;
        }
        assert_eq!(prefab_model_file("prefab://prefabs/interaction_e.xml/scarecrow.learning_scarecrow_02").as_deref(), Some("objects/env/01_nuia/001_ndeco/old/learning_ob01_e.chr"));
    }
}
