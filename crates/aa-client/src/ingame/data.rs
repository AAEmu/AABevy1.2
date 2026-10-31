//! Game database lookups the in-game world needs: which model an NPC / doodad template draws.
//!
//! The database is the server's plain SQLite copy (`compact.sqlite3`, read only). Results are
//! cached: a town has a few dozen distinct templates and hundreds of instances.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use aa_anim::GameDb;
use bevy::prelude::*;
use rusqlite::{Connection, OpenFlags, params};

pub(crate) use super::combat::skills::{Tool, tool_from_conn};

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
    /// `(template, func group)` -> the phase model.
    doodad_phase_models: HashMap<(u32, u32), Option<String>>,
    /// `(group id, sim radius in metres)` of a doodad template.
    doodad_shapes: HashMap<u32, (u32, f32)>,
    doodad_visuals: HashMap<(u32, u32), Option<String>>,
    doodad_growths: HashMap<u32, Option<DoodadGrowth>>,
    npc_scale: HashMap<u32, f32>,
    weapons: HashMap<u32, Option<Weapon>>,
    backpacks: HashMap<u32, Option<Weapon>>,
    anim_actions: HashMap<u32, Option<String>>,
    anim_action_tools: HashMap<u32, [Option<Tool>; 2]>,
    mount_poses: HashMap<u32, u32>,
    mount_speeds: HashMap<u32, MountSpeed>,
    camera_distances: HashMap<u32, Option<CameraDistance>>,
    attach_points: HashMap<u8, Option<AttachPointStrings>>,
    /// `(model, attach point)` -> the seat's `attach_anims.anim_action_id`.
    attach_anims: HashMap<(u32, u8), Option<u32>>,
    ships: HashMap<u32, bool>,
    ship_steer: HashMap<u32, f32>,
    ship_floors: HashMap<u32, Option<Arc<super::ship::floor::ShipFloor>>>,
    /// XY bounds of the cached hull floors, for the deck range gate.
    ship_bounds: HashMap<u32, Option<(Vec2, Vec2)>>,
}

/// Names of one `model_attach_point_strings` row: the actor bone creatures use and the prefab helper vehicle models use
/// (`$driver`, `$passenger0`, ...).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachPointStrings {
    pub actor: Option<String>,
    pub prefab: Option<String>,
}

/// The rowboat's `ship_models.steer_vel` (axis/s): the ramp rate of a ship model without a row.
pub const DEFAULT_SHIP_STEER_VEL: f32 = 6.0;

/// How fast a mount of a model goes with a rider (m/s): the run speed of its relaxed stance (`game_stances.ai_move_speed_run`,
/// stance 2: a Lilyut horse 9, a donkey 7.2) and the walking and backward factors of its actor model.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MountSpeed {
    pub run: f32,
    pub walk: f32,
    pub backward: f32,
}

impl Default for MountSpeed {
    /// A horse's.
    fn default() -> Self {
        MountSpeed { run: 9.0, walk: 0.2, backward: 0.2 }
    }
}

impl MountSpeed {
    /// The speed for a pressed direction (x right, y forward, unit length): backward and walking take their factors.
    pub fn of(&self, local_dir: Vec2, walking: bool) -> f32 {
        if local_dir == Vec2::ZERO {
            return 0.0;
        }
        let back = if local_dir.y < -0.1 { self.backward } else { 1.0 };
        let walk = if walking { self.walk } else { 1.0 };
        self.run * back * walk
    }
}

/// The third person camera distances of a vehicle or ship model: `models.camera_distance` and
/// `camera_distance_for_wide_angle`. 1.2's model loader FUN_396c7090 copies them into the descriptor
/// (desc+0x08 / +0x0c); FUN_3915c8a0 returns the first, or the second when option 65 (the camera mode)
/// is 1, for the driver of a `VehicleModel` / `ShipModel` only.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraDistance {
    pub normal: f32,
    pub wide: f32,
}

impl CameraDistance {
    /// The distance the client uses in camera mode `mode` (option 65): the wide-angle value in mode 1,
    /// the normal one otherwise; `None` when the chosen value is 0 (FUN_3915c8a0 then returns 0).
    pub fn for_mode(self, mode: i64) -> Option<f32> {
        let value = if mode == 1 { self.wide } else { self.normal };
        (value > 0.0).then_some(value)
    }
}

/// How a growing doodad phase scales (`doodad_func_growths`): from `start_scale` to `end_scale` over `delay_ms`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DoodadGrowth {
    pub delay_ms: u32,
    /// Per mille in the database, here as a factor (600 -> 0.6).
    pub start_scale: f32,
    pub end_scale: f32,
}

impl DoodadGrowth {
    /// The duration the scale is interpolated over: the `time_left` the server announced (it already applied the growth
    /// rate and climate), or the database `delay` when it announced none (0). The original `ClientDoodad::Update`
    /// (`0x393a4810`) reads exactly that duration and applies `start_scale` at once; a doodad first seen mid-phase
    /// therefore starts at `start_scale` for the remaining time. The linear ramp and reaching `end_scale` exactly at the
    /// phase change are **inferred** from `UpdateScaling`'s constant speed, not measured (F18 did not sample the
    /// original's scale: capture-pending). The phase-change re-init is inferred too: `0x251` gates it (the clear at
    /// `0x393a1c90` lets `Update` initialize again), also capture-pending (F18).
    pub fn total_ms(&self, time_left_ms: f32) -> f32 {
        if time_left_ms > 0.0 {
            time_left_ms
        } else {
            self.delay_ms as f32
        }
    }

    /// The scale `remaining_ms` before the end of a phase of `total_ms`: `start_scale` with the full time left,
    /// `end_scale` at 0.
    pub fn scale_at(&self, remaining_ms: f32, total_ms: f32) -> f32 {
        if total_ms <= 0.0 {
            return self.end_scale;
        }
        let t = (1.0 - remaining_ms / total_ms).clamp(0.0, 1.0);
        self.start_scale + (self.end_scale - self.start_scale) * t
    }
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
    ("item_tools", "item_id"),
    ("holdables", "id"),
    ("anim_actions", "id"),
    ("attach_anims", "owner_id"),
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

impl Lookups {
    fn new(conn: Connection) -> Lookups {
        Lookups {
            conn,
            actor_files: HashMap::new(),
            doodad_models: HashMap::new(),
            doodad_phase_models: HashMap::new(),
            doodad_shapes: HashMap::new(),
            doodad_visuals: HashMap::new(),
            doodad_growths: HashMap::new(),
            npc_scale: HashMap::new(),
            weapons: HashMap::new(),
            backpacks: HashMap::new(),
            anim_actions: HashMap::new(),
            anim_action_tools: HashMap::new(),
            mount_poses: HashMap::new(),
            mount_speeds: HashMap::new(),
            camera_distances: HashMap::new(),
            attach_points: HashMap::new(),
            attach_anims: HashMap::new(),
            ships: HashMap::new(),
            ship_steer: HashMap::new(),
            ship_floors: HashMap::new(),
            ship_bounds: HashMap::new(),
        }
    }
}

impl Db {
    pub fn open() -> Option<Db> {
        let original = std::env::var("AA_DB").unwrap_or_else(|_| aa_anim::db::DEFAULT_DB.to_string());
        Db::open_path(&indexed_database(&original))
    }

    /// The database file at `path`, read only (tests open small files of their own rows).
    pub(crate) fn open_path(path: &str) -> Option<Db> {
        let game = match GameDb::open_path(path) {
            Ok(g) => g,
            Err(e) => {
                error!("game database: {e}");
                return None;
            }
        };
        let conn = match Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI) {
            Ok(c) => c,
            Err(e) => {
                error!("game database {path}: {e}");
                return None;
            }
        };
        Some(Db {
            game: Arc::new(Mutex::new(game)),
            lookups: Arc::new(Mutex::new(Lookups::new(conn))),
        })
    }

    /// A catalog with just one ship model row (the tests use the real pak
    /// prefab named by `normal`). The temp file stays for the process; the
    /// connections keep it open on Windows.
    #[cfg(test)]
    pub(crate) fn ship_fixture(model_id: u32, normal: &str) -> Option<Db> {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "aa-client-ship-{}-{}.sqlite3",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);
        let conn = Connection::open(&path).ok()?;
        conn.execute_batch(
            "CREATE TABLE models(id INTEGER PRIMARY KEY, sub_type TEXT, sub_id INTEGER);
             CREATE TABLE ship_models(id INTEGER PRIMARY KEY, normal TEXT, steer_vel REAL);",
        )
        .ok()?;
        conn.execute(
            "INSERT INTO models VALUES(?1, 'ShipModel', 1)",
            params![model_id],
        )
        .ok()?;
        conn.execute(
            "INSERT INTO ship_models VALUES(1, ?1, 6.0)",
            params![normal],
        )
        .ok()?;
        drop(conn);
        let game = GameDb::open_path(&path).ok()?;
        let conn = Connection::open_with_flags(
            &path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
        )
        .ok()?;
        Some(Db {
            game: Arc::new(Mutex::new(game)),
            lookups: Arc::new(Mutex::new(Lookups::new(conn))),
        })
    }

    /// The ship catalog with a synthetic floor already cached (the tests that
    /// need a raised deck do not read a hull brush).
    #[cfg(test)]
    pub(crate) fn ship_floor_fixture(
        model_id: u32,
        floor: Arc<super::ship::floor::ShipFloor>,
    ) -> Option<Db> {
        let db = Db::ship_fixture(model_id, "prefab://test/none")?;
        {
            let mut l = db.lookups.lock().unwrap();
            l.ship_bounds.insert(model_id, floor.bounds());
            l.ship_floors.insert(model_id, Some(floor));
        }
        Some(db)
    }

    /// The model file (`.cdf` / `.chr`) of an actor model (`models.id`), lower case, `/` separators; for a prefab, vehicle
    /// or ship model the one model that stands for it.
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

    /// Whether a model is a ship (`models.sub_type` `ShipModel`): the server simulates its hull and the driver
    /// steers it with a CSMoveUnit type-4 request instead of the land-vehicle type 2.
    pub fn is_ship(&self, model_id: u32) -> bool {
        let mut l = self.lookups.lock().unwrap();
        if let Some(ship) = l.ships.get(&model_id) {
            return *ship;
        }
        let ship = l
            .conn
            .query_row(
                "SELECT 1 FROM models WHERE id = ?1 AND sub_type = 'ShipModel'",
                params![model_id],
                |_| Ok(()),
            )
            .is_ok();
        l.ships.insert(model_id, ship);
        ship
    }

    /// The steering ramp of a ship model (`ship_models.steer_vel`, axis/s) through its `models` row; the rowboat's
    /// [`DEFAULT_SHIP_STEER_VEL`] when the model has no ship row.
    pub fn ship_steer_vel(&self, model_id: u32) -> f32 {
        let mut l = self.lookups.lock().unwrap();
        if let Some(v) = l.ship_steer.get(&model_id) {
            return *v;
        }
        let vel = ship_steer_vel_from_conn(&l.conn, model_id);
        l.ship_steer.insert(model_id, vel);
        vel
    }

    /// The walkable hull floor of a ship model (the `body` brush of its `ship_models.normal` prefab), loaded once;
    /// `None` for a model that is not a ship or has no readable hull.
    pub fn ship_floor(&self, model_id: u32) -> Option<Arc<super::ship::floor::ShipFloor>> {
        // `ShipFloor::load` reads and parses the hull mesh: do it without the
        // `lookups` mutex, or every `is_ship` / `ship_steer_vel` caller waits
        // for the whole load. A racing loader may load twice; the first insert
        // wins.
        let normal: Option<String> = {
            let l = self.lookups.lock().unwrap();
            if let Some(floor) = l.ship_floors.get(&model_id) {
                return floor.clone();
            }
            l.conn
                .query_row(
                    "SELECT s.normal FROM models m JOIN ship_models s ON s.id = m.sub_id AND m.sub_type = 'ShipModel' WHERE m.id = ?1",
                    params![model_id],
                    |r| r.get::<_, Option<String>>(0),
                )
                .ok()
                .flatten()
        };
        let floor = normal
            .as_deref()
            .and_then(super::ship::floor::ShipFloor::load)
            .map(Arc::new);
        let mut l = self.lookups.lock().unwrap();
        let cached = l
            .ship_floors
            .entry(model_id)
            .or_insert_with(|| floor.clone())
            .clone();
        let bounds = cached.as_ref().and_then(|f| f.bounds());
        l.ship_bounds.entry(model_id).or_insert(bounds);
        cached
    }

    /// The XY bounds of a ship model's hull floor in the ship frame, cached by
    /// [`Db::ship_floor`] (unit preparation preloads it off the frame). Does not
    /// load anything; `None` until the floor has been prepared.
    pub fn ship_bounds(&self, model_id: u32) -> Option<(Vec2, Vec2)> {
        self.lookups
            .lock()
            .unwrap()
            .ship_bounds
            .get(&model_id)
            .copied()
            .flatten()
    }

    /// The look of a vehicle or ship model (`models.sub_type` `VehicleModel` / `ShipModel`: airships, carts, boats,
    /// siege weapons): its `normal` model reference and its two wheel models, if any.
    pub fn vehicle_look(&self, model_id: u32) -> Option<VehicleLook> {
        let l = self.lookups.lock().unwrap();
        let (normal, wheel, wheel2): (Option<String>, Option<String>, Option<String>) = l
            .conn
            .query_row(
                "SELECT COALESCE(v.normal, s.normal), v.wheel, v.wheel2 FROM models m
                 LEFT JOIN vehicle_models v ON v.id = m.sub_id AND m.sub_type = 'VehicleModel'
                 LEFT JOIN ship_models s ON s.id = m.sub_id AND m.sub_type = 'ShipModel'
                 WHERE m.id = ?1",
                params![model_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .ok()?;
        let normal = normal.map(|n| n.trim().to_string()).filter(|n| !n.is_empty())?;
        let file = |w: Option<String>| w.as_deref().and_then(model_file);
        Some(VehicleLook { normal, wheels: [file(wheel), file(wheel2)] })
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

    /// The tool models an NPC pose holds (`anim_actions.mainhand_tool_id` / `offhand_tool_id`): main hand first.
    pub fn anim_action_tools(&self, anim_action: u32) -> [Option<Tool>; 2] {
        let mut l = self.lookups.lock().unwrap();
        if let Some(t) = l.anim_action_tools.get(&anim_action) {
            return t.clone();
        }
        let ids: Option<(u32, u32)> = l
            .conn
            .query_row(
                "SELECT COALESCE(mainhand_tool_id, 0), COALESCE(offhand_tool_id, 0) FROM anim_actions WHERE id = ?1",
                params![anim_action],
                |r| Ok((r.get::<_, i64>(0)? as u32, r.get::<_, i64>(1)? as u32)),
            )
            .ok();
        let tools = match ids {
            Some((main, off)) => [tool_from_conn(&l.conn, main), tool_from_conn(&l.conn, off)],
            None => [None, None],
        };
        l.anim_action_tools.insert(anim_action, tools.clone());
        tools
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

    /// The `model_attach_point_strings` row of an attach point: the actor bone and the prefab helper name. The strings are
    /// the same for every model; callers pick actor for creatures and prefab for vehicle models.
    pub fn attach_point_strings(&self, point: u8) -> Option<AttachPointStrings> {
        let mut l = self.lookups.lock().unwrap();
        if let Some(s) = l.attach_points.get(&point) {
            return s.clone();
        }
        let strings = l
            .conn
            .query_row(
                "SELECT actor, prefab FROM model_attach_point_strings WHERE id = ?1",
                params![i64::from(point)],
                |r| {
                    let read = |i: usize| -> rusqlite::Result<Option<String>> {
                        Ok(r.get::<_, Option<String>>(i)?
                            .map(|s| s.trim().to_ascii_lowercase())
                            .filter(|s| !s.is_empty()))
                    };
                    Ok(AttachPointStrings {
                        actor: read(0)?,
                        prefab: read(1)?,
                    })
                },
            )
            .ok();
        l.attach_points.insert(point, strings.clone());
        strings
    }

    /// The animation action a rider at `point` of a vehicle model sits in (`attach_anims`: `owner_type` `VehicleModel`,
    /// `owner_id` the model's `sub_id`). 43 rows over the vehicle models: the Farm Wagon's driver (model 1008, `sub_id`
    /// 38) is action 12 `fist_ac_steer_sit_idle`; the siege weapons and cannon seats use `fist_ba_siegeweapon_idle` /
    /// `fist_ac_cannon_idle` / `fist_ac_ballista_idle`, the speedcars `speedcar` or `fist_ac_sit_down`, the drums
    /// `music_co_sk_drum_s`, the trailers `fist_ac_trailer_sit_idle`. Creatures have no row (their attach point is a
    /// bone, not a vehicle model).
    pub fn attach_anim(&self, model_id: u32, point: u8) -> Option<u32> {
        let mut l = self.lookups.lock().unwrap();
        if let Some(a) = l.attach_anims.get(&(model_id, point)) {
            return *a;
        }
        let action: Option<u32> = l
            .conn
            .query_row(
                "SELECT a.anim_action_id FROM models m
                 JOIN attach_anims a ON a.owner_id = m.sub_id AND a.owner_type = 'VehicleModel'
                 WHERE m.id = ?1 AND m.sub_type = 'VehicleModel' AND a.attach_point_id = ?2
                 ORDER BY a.id LIMIT 1",
                params![model_id, i64::from(point)],
                |r| r.get::<_, Option<u32>>(0),
            )
            .ok()
            .flatten();
        l.attach_anims.insert((model_id, point), action);
        action
    }

    /// The camera distances of a vehicle or ship model ([`CameraDistance`]); `None` for an actor, prefab or
    /// other model kind, and for a model whose normal distance is 0. The captured values: the Farm Wagon's
    /// model 1008 normal 20 m (wide 16) and the Eznan Cutter's model 393 (the row named `galleon`) normal 40 m
    /// (wide 25).
    pub fn camera_distance(&self, model_id: u32) -> Option<CameraDistance> {
        let mut l = self.lookups.lock().unwrap();
        if let Some(d) = l.camera_distances.get(&model_id) {
            return *d;
        }
        let row: Option<(Option<f64>, Option<f64>)> = l
            .conn
            .query_row(
                "SELECT camera_distance, camera_distance_for_wide_angle FROM models
                 WHERE id = ?1 AND sub_type IN ('VehicleModel', 'ShipModel')",
                params![model_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();
        let distance = row.and_then(|(normal, wide)| {
            let normal = normal? as f32;
            (normal > 0.0).then(|| CameraDistance {
                normal,
                wide: wide.map_or(normal, |w| w as f32),
            })
        });
        l.camera_distances.insert(model_id, distance);
        distance
    }

    /// How fast a mount of `model_id` goes ([`MountSpeed`]; a horse's when the database has no row).
    pub fn mount_speed(&self, model_id: u32) -> MountSpeed {
        let mut l = self.lookups.lock().unwrap();
        if let Some(s) = l.mount_speeds.get(&model_id) {
            return *s;
        }
        let d = MountSpeed::default();
        let speed = l
            .conn
            .query_row(
                "SELECT g.ai_move_speed_run, a.game_walk_multiplier, a.game_backward_multiplier FROM models m                  JOIN actor_models a ON a.id = m.sub_id AND m.sub_type = 'ActorModel'                  LEFT JOIN game_stances g ON g.actor_model_id = a.id AND g.stance_id = 2 WHERE m.id = ?1",
                params![model_id],
                |r| {
                    let f = |i: usize, default: f32| -> rusqlite::Result<f32> { Ok(r.get::<_, Option<f64>>(i)?.map_or(default, |v| v as f32)) };
                    Ok(MountSpeed { run: f(0, d.run)?, walk: f(1, d.walk)?, backward: f(2, d.backward)? })
                },
            )
            .unwrap_or(d);
        l.mount_speeds.insert(model_id, speed);
        speed
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

    /// The model a doodad template is drawn with (`doodad_almighties.model`: `cgf://`, `vegetation://`, `cga://` or
    /// `prefab://`).
    pub fn doodad_model(&self, template_id: u32) -> Option<String> {
        let mut l = self.lookups.lock().unwrap();
        if let Some(m) = l.doodad_models.get(&template_id) {
            return m.clone();
        }
        let model = almighty_model(&l.conn, template_id);
        l.doodad_models.insert(template_id, model.clone());
        model
    }

    /// The model of a doodad phase (`doodad_func_groups.model`) as a pak path; an empty (or unusable) phase model
    /// falls back to the template's [`Db::doodad_model`].
    pub fn doodad_phase_model(&self, template_id: u32, func_group_id: u32) -> Option<String> {
        let key = (template_id, func_group_id);
        let raw = {
            let l = self.lookups.lock().unwrap();
            if let Some(m) = l.doodad_phase_models.get(&key) {
                return m.clone();
            }
            l.conn
                .query_row(
                    "SELECT model FROM doodad_func_groups WHERE id = ?1 AND doodad_almighty_id = ?2",
                    params![func_group_id, template_id],
                    |r| r.get::<_, Option<String>>(0),
                )
                .ok()
                .flatten()
        };
        let model = raw.as_deref().and_then(model_file).or_else(|| self.doodad_model(template_id));
        self.lookups.lock().unwrap().doodad_phase_models.insert(key, model.clone());
        model
    }

    /// The complete phase geometry reference. Prefabs retain their identity and all parts.
    pub fn doodad_phase_visual(&self, template_id: u32, func_group_id: u32) -> Option<String> {
        let key = (template_id, func_group_id);
        let mut l = self.lookups.lock().unwrap();
        if let Some(reference) = l.doodad_visuals.get(&key) { return reference.clone(); }
        let phase = l.conn.query_row("SELECT model FROM doodad_func_groups WHERE id=?1 AND doodad_almighty_id=?2", params![func_group_id,template_id], |r| r.get::<_,Option<String>>(0)).ok().flatten();
        let base = l.conn.query_row("SELECT model FROM doodad_almighties WHERE id=?1",params![template_id],|r|r.get::<_,Option<String>>(0)).ok().flatten();
        let normalize = |raw: &str| {
            let raw = raw.trim().replace('\\',"/");
            if raw.to_ascii_lowercase().starts_with("prefab://") {
                let (file,name) = raw["prefab://".len()..].rsplit_once('/')?;
                let reference = format!("prefab://{}/{}",norm(file),name);
                (!super::housing::prefab::load(&reference).is_empty()).then_some(reference)
            } else { model_file(&raw) }
        };
        let reference = phase.as_deref().and_then(normalize).or_else(||base.as_deref().and_then(normalize));
        l.doodad_visuals.insert(key,reference.clone());
        reference
    }

    /// The doodad template a planting item places (`item_spawn_doodads`: seeds, saplings, livestock, ladders ...), `None` for
    /// every other item.
    pub fn item_doodad(&self, item_template: u32) -> Option<u32> {
        let l = self.lookups.lock().unwrap();
        l.conn
            .query_row("SELECT doodad_id FROM item_spawn_doodads WHERE item_id = ?1 ORDER BY id LIMIT 1", params![item_template], |r| r.get::<_, i64>(0))
            .ok()
            .map(|d| d as u32)
    }

    /// The model a new doodad of `template_id` has (the model of its start phase, group kind 1), its doodad group and
    /// the room it keeps around it (`sim_radius`, metres, at least 0.1).
    pub fn doodad_start(&self, template_id: u32) -> (Option<String>, u32, f32) {
        let (group, (group_id, radius)) = {
            let mut l = self.lookups.lock().unwrap();
            let group = l
                .conn
                .query_row("SELECT id FROM doodad_func_groups WHERE doodad_almighty_id = ?1 AND doodad_func_group_kind_id = 1 ORDER BY id LIMIT 1", params![template_id], |r| r.get::<_, i64>(0))
                .ok();
            let shape = if let Some(shape) = l.doodad_shapes.get(&template_id) {
                *shape
            } else {
                let shape = shape_of(&l.conn, template_id);
                l.doodad_shapes.insert(template_id, shape);
                shape
            };
            (group, shape)
        };
        let model = group.map_or_else(|| self.doodad_model(template_id), |g| self.doodad_phase_model(template_id, g as u32));
        (model, group_id, radius)
    }

    /// The doodad group and simulation radius (`max(sim_radius / 1000, 0.1)` m) of a template.
    pub fn doodad_shape(&self, template_id: u32) -> (u32, f32) {
        let mut l = self.lookups.lock().unwrap();
        if let Some(shape) = l.doodad_shapes.get(&template_id) {
            return *shape;
        }
        let shape = shape_of(&l.conn, template_id);
        l.doodad_shapes.insert(template_id, shape);
        shape
    }

    /// How a phase grows (`doodad_phase_funcs` -> `doodad_func_growths`), `None` when it does not.
    pub fn doodad_growth(&self, func_group_id: u32) -> Option<DoodadGrowth> {
        let mut l = self.lookups.lock().unwrap();
        if let Some(g) = l.doodad_growths.get(&func_group_id) {
            return *g;
        }
        let growth: Option<DoodadGrowth> = l
            .conn
            .query_row(
                "SELECT g.delay, g.start_scale, g.end_scale
                 FROM doodad_phase_funcs p JOIN doodad_func_growths g ON g.id = p.actual_func_id
                 WHERE p.doodad_func_group_id = ?1 AND p.actual_func_type = 'DoodadFuncGrowth'
                 ORDER BY p.id LIMIT 1",
                params![func_group_id],
                |r| {
                    Ok(DoodadGrowth {
                        delay_ms: r.get::<_, Option<i64>>(0)?.unwrap_or(0).max(0) as u32,
                        start_scale: r.get::<_, Option<i64>>(1)?.unwrap_or(1000) as f32 / 1000.0,
                        end_scale: r.get::<_, Option<i64>>(2)?.unwrap_or(1000) as f32 / 1000.0,
                    })
                },
            )
            .ok();
        l.doodad_growths.insert(func_group_id, growth);
        growth
    }
}

/// `doodad_almighties.group_id` and the simulation radius: `max(sim_radius * 0.001, 0.1)` m, as the 1.2 client's
/// `FUN_3983b280`.
fn shape_of(conn: &Connection, template_id: u32) -> (u32, f32) {
    let row = conn
        .query_row(
            "SELECT group_id, sim_radius FROM doodad_almighties WHERE id = ?1",
            params![template_id],
            |r| Ok((r.get::<_, Option<i64>>(0)?.unwrap_or(0), r.get::<_, Option<i64>>(1)?.unwrap_or(0))),
        )
        .ok()
        .unwrap_or((0, 0));
    (row.0.max(0) as u32, (row.1 as f32 * 0.001).max(0.1))
}

/// `doodad_almighties.model` as a pak path.
fn almighty_model(conn: &Connection, template_id: u32) -> Option<String> {
    conn.query_row("SELECT model FROM doodad_almighties WHERE id = ?1", params![template_id], |r| r.get::<_, Option<String>>(0))
        .ok()
        .flatten()
        .and_then(|m| model_file(&m))
}

/// A database model reference as a pak path: `cgf://Game/x.cgf`, `vegetation://x.cgf`, `cga://x.cga` lose their
/// scheme (and `game/`), a `prefab://` path becomes the `object_Model` of its prefab. Empty and unusable references are
/// `None`.
pub(crate) fn model_file(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if raw.to_ascii_lowercase().starts_with("prefab://") {
        return prefab_model_file(&format!("prefab://{}", &raw["prefab://".len()..]));
    }
    let mut path = norm(raw);
    for scheme in ["cgf://", "cga://", "vegetation://"] {
        if let Some(rest) = path.strip_prefix(scheme) {
            path = rest.to_string();
            break;
        }
    }
    let path = path.strip_prefix("game/").unwrap_or(&path).to_string();
    path.contains('.').then_some(path)
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

/// `ship_models.steer_vel` of a ship model through its `models` row: the axis/s the helm steering ramps at.
fn ship_steer_vel_from_conn(conn: &Connection, model_id: u32) -> f32 {
    conn.query_row(
        "SELECT s.steer_vel FROM models m JOIN ship_models s ON s.id = m.sub_id AND m.sub_type = 'ShipModel' WHERE m.id = ?1",
        params![model_id],
        |r| r.get::<_, Option<f64>>(0),
    )
    .ok()
    .flatten()
    .filter(|v| v.is_finite() && *v > 0.0)
    .map_or(DEFAULT_SHIP_STEER_VEL, |v| v as f32)
}

/// What a vehicle or ship is drawn from ([`Db::vehicle_look`]).
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleLook {
    /// `prefab://file/name` or a plain model reference (`cgf://...`).
    pub normal: String,
    /// The wheel models (pak paths): `wheel` for the `wheelN` mounts, `wheel2` for the `wheel_type2N` ones.
    pub wheels: [Option<String>; 2],
}

/// Lower case with `/` separators, as the pak wants paths.
pub fn norm(p: &str) -> String {
    p.replace('\\', "/").to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires original catalog and prefab assets"]
    fn placed_instrument_visuals_keep_the_complete_prefab() {
        let db = Db::open().expect("read-only game catalog");
        for (template,group,name) in [(1236,1951,"piano_a"),(6200,16414,"piano_a"),(8220,22863,"piano_b"),(8065,22477,"percussion_a"),(8255,23046,"percussion_a")] {
            let reference = db.doodad_phase_visual(template,group).unwrap();
            assert_eq!(reference,format!("prefab://prefabs/interaction_c.xml/furniture.{name}"));
            assert_eq!(super::super::housing::prefab::load(&reference).len(),2);
            assert_eq!(db.doodad_phase_model(template,group),None,"former first-object_Model lookup cannot draw this prefab");
            assert_eq!(db.doodad_phase_visual(template,group).as_deref(),Some(reference.as_str()),"cached reference");
        }
        // Ordinary plants retain the established phase/fallback model paths.
        assert_eq!(db.doodad_phase_visual(2246,4403),db.doodad_phase_model(2246,4403));
        assert_eq!(db.doodad_phase_visual(2246,10155),db.doodad_phase_model(2246,10155));
    }

    #[test]
    #[ignore = "requires original catalog assets"]
    fn vehicles_and_ships_have_a_look() {
        let db = Db::open().expect("read-only game catalog");
        let airship = db.vehicle_look(657).expect("airship");
        assert_eq!(airship.normal, "prefab://Prefabs/airship.xml/airship.normal");
        assert_eq!(airship.wheels, [None, None]);
        let tractor = db.vehicle_look(1008).expect("tractor");
        assert_eq!(tractor.wheels[0].as_deref(), Some("objects/env/06_unit/05_carriage/transfers_trailer_a_wheel01.cgf"));
        assert_eq!(tractor.wheels[1].as_deref(), Some("objects/env/06_unit/05_carriage/transfers_trailer_a_wheel02.cgf"));
        assert!(db.vehicle_look(393).is_some_and(|g| g.normal.ends_with("galleon.step_10galleon_normal")), "galleon");
        assert!(db.is_ship(129), "rowboat (smallboat) is a ShipModel");
        assert!(db.is_ship(393), "galleon is a ShipModel");
        assert!(!db.is_ship(1008), "tractor is a VehicleModel");
        assert!(db.actor_model_file(657).is_none());
    }

    /// Ship models 4 (the rowboat) and 19 (a 0.8 fixture value); model 5 has no ship row.
    #[test]
    fn ship_steer_vel_comes_from_the_ship_model() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE models(id INT PRIMARY KEY, sub_type TEXT, sub_id INT);
             CREATE TABLE ship_models(id INT PRIMARY KEY, steer_vel REAL);
             INSERT INTO ship_models VALUES (4, 6.0), (19, 0.8);
             INSERT INTO models VALUES (129, 'ShipModel', 4), (500, 'ShipModel', 19), (1008, 'VehicleModel', 7);",
        )
        .unwrap();
        assert_eq!(ship_steer_vel_from_conn(&conn, 129), 6.0, "rowboat");
        assert_eq!(ship_steer_vel_from_conn(&conn, 500), 0.8, "the 0.8 fixture value");
        assert_eq!(ship_steer_vel_from_conn(&conn, 1008), DEFAULT_SHIP_STEER_VEL, "not a ship");
        assert_eq!(ship_steer_vel_from_conn(&conn, 999), DEFAULT_SHIP_STEER_VEL, "no row");
    }

    /// The real catalog: the rowboat's smallboat model and the galleon.
    #[test]
    fn the_database_holds_the_ship_steer_rates() {
        let Some(db) = Db::open() else { return };
        assert_eq!(db.ship_steer_vel(129), 6.0, "rowboat");
        assert_eq!(db.ship_steer_vel(393), 1.0, "galleon");
        assert_eq!(db.ship_steer_vel(u32::MAX), DEFAULT_SHIP_STEER_VEL, "no ship row");
    }

    /// The load drops the `lookups` guard; the cache then returns the inserted
    /// floor (the rowboat) together with its bounds.
    #[test]
    fn the_ship_floor_cache_returns_the_inserted_floor_and_bounds() {
        let Some(db) = test_db() else { return };
        if !aa_cry::pak_available() {
            return;
        }
        let floor = db.ship_floor(129).expect("the rowboat hull");
        let bounds = db.ship_bounds(129).expect("bounds cached with the floor");
        assert_eq!(floor.bounds(), Some(bounds));
        let again = db.ship_floor(129).expect("the cached floor");
        assert!(
            Arc::ptr_eq(&floor, &again),
            "the cache keeps the loaded floor"
        );
    }

    #[test]
    fn the_farm_wagon_camera_distances_follow_the_models_row() {
        let Some(db) = Db::open() else {
            eprintln!("skip: no client database");
            return;
        };
        let wagon = db.camera_distance(1008).expect("the Farm Wagon model");
        assert_eq!((wagon.normal, wagon.wide), (20.0, 16.0));
        // option 65 == 1 (the capture's camera mode) uses the wide value; the others the normal one
        assert_eq!(wagon.for_mode(1), Some(16.0));
        assert_eq!(wagon.for_mode(2), Some(20.0));
        // an actor (the horse) and an unknown model have no driver camera distance; FUN_3915c8a0 only reads
        // VehicleModel / ShipModel descriptors
        assert_eq!(db.camera_distance(139), None);
        assert_eq!(db.camera_distance(0), None);
        // model 393 is the Eznan Cutter (its models row is named "galleon"): the doodads capture at its helm
        // (2026-10-30) read wanted = max 25.0 in camera mode 1, the row's normal 40 / wide 25
        let cutter = db
            .camera_distance(393)
            .expect("the Eznan Cutter's ShipModel row");
        assert_eq!((cutter.normal, cutter.wide), (40.0, 25.0));
        assert_eq!(
            cutter.for_mode(1),
            Some(25.0),
            "the Eznan Cutter helm's mode 1 camera distance"
        );
    }

    #[test]
    fn a_camera_distance_of_zero_takes_no_effect() {
        let none = CameraDistance {
            normal: 0.0,
            wide: 0.0,
        };
        assert_eq!(none.for_mode(1), None);
        assert_eq!(none.for_mode(2), None);
        assert_eq!(
            CameraDistance {
                normal: 12.0,
                wide: 0.0
            }
            .for_mode(1),
            None,
            "a missing wide value is 0 in the row"
        );
    }

    #[test]
    fn a_mount_runs_at_its_stance_speed_and_slower_backward_or_walking() {
        let horse = MountSpeed::default();
        assert_eq!(horse.of(Vec2::Y, false), 9.0);
        assert_eq!(horse.of(Vec2::X, false), 9.0);
        assert!((horse.of(-Vec2::Y, false) - 1.8).abs() < 1e-5);
        assert!((horse.of(Vec2::Y, true) - 1.8).abs() < 1e-5);
        assert_eq!(horse.of(Vec2::ZERO, false), 0.0);
    }

    /// The Lilyut horse (model 139) and the donkey (953) as the database has them.
    #[test]
    fn the_database_holds_the_mount_speeds() {
        let Some(db) = Db::open() else { return };
        assert_eq!(db.mount_speed(139), MountSpeed { run: 9.0, walk: 0.2, backward: 0.2 });
        assert!((db.mount_speed(953).run - 7.2).abs() < 1e-5);
    }

    /// The training dummies are prefab models: their model file is the `object_Model` of the prefab.
    #[test]
    fn a_portal_gate_prefab_is_its_particle_effects() {
        let e = prefab_effects_of("prefab://Prefabs/fx.xml/portal_gate_dot");
        let names: Vec<&str> = e.iter().map(|e| e.effect.as_str()).collect();
        assert_eq!(names, ["abillity_skill_table_m.system_skill.portal_gate_dot3_indivisual", "abillity_skill_table_m.system_skill.portal_gate_hit"]);
        assert!((e[0].pos.z - 0.036621094).abs() < 1e-6);
        assert!(!prefab_effects_of("prefab://Prefabs/fx.xml/portal_gate_exit_dot").is_empty());
    }

    /// `item_tools.item_id` -> `item_tools.asset_id` -> the `item_assets` row: the model path and the hand attachment offset.
    #[test]
    fn a_tool_item_resolves_through_item_tools_to_its_asset() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE item_tools(item_id INT, asset_id INT);
             CREATE TABLE item_assets(id INT PRIMARY KEY, path TEXT, attachment_offset_pos_x REAL, attachment_offset_pos_y REAL,
                 attachment_offset_pos_z REAL, attachment_offset_rot_x REAL, attachment_offset_rot_y REAL, attachment_offset_rot_z REAL);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO item_assets VALUES (289, 'Objects\\Item\\Weapon\\Etc\\Shovel_General\\Shovel_General.cgf', -0.01, 0.16, 0, 10, 88.88, 0)",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO item_tools VALUES (2660, 289)", []).unwrap();
        let tool = tool_from_conn(&conn, 2660).expect("the shovel");
        assert_eq!(tool.path, "objects/item/weapon/etc/shovel_general/shovel_general.cgf");
        assert_eq!(tool.offset, [-0.01, 0.16, 0.0]);
        assert_eq!(tool.rotation, [10.0, 88.88, 0.0]);
        assert_eq!(tool_from_conn(&conn, 0), None, "an empty tool id holds nothing");
        assert_eq!(tool_from_conn(&conn, 1), None, "an item without a tool row holds nothing");
    }

    /// The real row of the uproot shovel: item 2660 -> `item_tools` 9 -> `item_assets` 289.
    #[test]
    fn the_real_tools_come_from_item_tools() {
        let Some(db) = Db::open() else { return };
        let shovel = tool_from_conn(&db.lookups.lock().unwrap().conn, 2660).expect("the shovel");
        assert_eq!(shovel.path, "objects/item/weapon/etc/shovel_general/shovel_general.cgf");
        assert_eq!(shovel.offset, [-0.01, 0.16, 0.0]);
        assert_eq!(shovel.rotation, [10.0, 88.88, 0.0]);
        let pot = tool_from_conn(&db.lookups.lock().unwrap().conn, 14511).expect("the watering pot");
        assert!(pot.path.ends_with("ndeco_quest_pot_a.cgf"), "{}", pot.path);
        // anim_actions 68 `fist_pos_stn_florist_idle` holds the flowerpot in its off hand
        let pose = db.anim_action_tools(68);
        assert_eq!(pose[0], None);
        assert!(pose[1].as_ref().is_some_and(|t| t.path.ends_with("ndeco_housing_flowerpot01.cgf")));
    }

    #[test]
    fn a_prefab_model_has_the_file_of_its_animated_object() {
        if !aa_cry::pak_available() {
            return;
        }
        assert_eq!(prefab_model_file("prefab://prefabs/interaction_e.xml/scarecrow.learning_scarecrow_02").as_deref(), Some("objects/env/01_nuia/001_ndeco/old/learning_ob01_e.chr"));
    }

    #[test]
    fn phase_model_references_become_pak_paths() {
        assert_eq!(model_file("cgf://Game/Objects/Env/X.cgf").as_deref(), Some("objects/env/x.cgf"));
        assert_eq!(model_file("vegetation://objects/env/05_nature/rice01.cgf").as_deref(), Some("objects/env/05_nature/rice01.cgf"));
        assert_eq!(model_file("cga://objects/env/door.cga").as_deref(), Some("objects/env/door.cga"));
        assert_eq!(model_file(" \tvegetation://Objects/Env/rice01.cgf ").as_deref(), Some("objects/env/rice01.cgf"));
        assert_eq!(model_file(""), None);
        assert_eq!(model_file("a://invalid"), None);
    }

    #[test]
    fn prefab_phase_model_is_the_model_of_its_prefab() {
        if !aa_cry::pak_available() {
            return;
        }
        let path = model_file("prefab://prefabs/interaction_e.xml/tools.lever_left").unwrap();
        assert!(path.ends_with(".cgf") || path.ends_with(".chr") || path.ends_with(".cga"), "{path}");
    }

    #[test]
    fn a_phase_change_grows_over_the_announced_time_left() {
        let g = DoodadGrowth { delay_ms: 10_000, start_scale: 0.6, end_scale: 1.0 };
        // the server shortened its delay by the growth rate: the new phase lasts 4,000 ms, not the database's 10,000
        let total = g.total_ms(4_000.0);
        assert_eq!(total, 4_000.0);
        assert!((g.scale_at(total, total) - 0.6).abs() < 1e-6);
        assert!((g.scale_at(2_000.0, total) - 0.8).abs() < 1e-6);
        // exactly when the server changes phase
        assert!((g.scale_at(0.0, total) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_doodad_first_seen_mid_phase_starts_at_start_scale_for_the_announced_time() {
        let g = DoodadGrowth { delay_ms: 10_000, start_scale: 0.6, end_scale: 1.0 };
        // The original `ClientDoodad::Update` (0x393a4810) uses the packet's time_left as the whole duration and starts
        // at start_scale; a mid-phase remaining time is not stretched back to the database delay.
        let total = g.total_ms(4_000.0);
        assert_eq!(total, 4_000.0);
        assert!((g.scale_at(total, total) - 0.6).abs() < 1e-6);
        assert!((g.scale_at(2_000.0, total) - 0.8).abs() < 1e-6);
        // first seen with more time than the database delay (the server slowed the phase): that time is the duration
        assert_eq!(g.total_ms(12_000.0), 12_000.0);
        // no announced time_left: the database delay is the duration, again from start_scale (not the end)
        assert_eq!(g.total_ms(0.0), 10_000.0);
        assert!((g.scale_at(g.total_ms(0.0), g.total_ms(0.0)) - 0.6).abs() < 1e-6);
        // no delay to fall back on: it is over
        assert_eq!(DoodadGrowth { delay_ms: 0, start_scale: 0.5, end_scale: 0.9 }.scale_at(0.0, 0.0), 0.9);
    }

    fn test_db() -> Option<Db> {
        let path = std::env::var("AA_DB").ok()?;
        std::path::Path::new(&path).exists().then(Db::open).flatten()
    }

    #[test]
    fn phases_and_growths_come_from_the_database() {
        let Some(db) = test_db() else { return };
        let rice = "objects/env/05_nature/00_common/common_plants_rice01.cgf";
        assert_eq!(db.doodad_phase_model(2246, 4403).as_deref(), Some(rice));
        assert_eq!(db.doodad_phase_model(367, 2632).as_deref(), Some("objects/env/05_nature/04_subpol/sub_tree_bigfir01_grow02.cgf"));
        // an empty phase model falls back to the almighty one
        assert_eq!(db.doodad_phase_model(2246, 10155).as_deref(), Some(rice));
        let g = db.doodad_growth(4403).expect("rice phase 4403 grows");
        assert_eq!((g.delay_ms, g.start_scale, g.end_scale), (9_288_000, 0.75, 1.0));
        assert!(db.doodad_growth(10155).is_none());
    }
}
