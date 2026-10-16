//! Streaming: loads and unloads cells around the focus (the camera) on background tasks.
//!
//! Loading a cell is split so no frame ever does heavy work:
//!
//! * a background task reads and decodes the files and builds the patch meshes (terrain), and sends the
//!   result over a channel;
//! * the main thread only moves finished assets into `Assets<_>` and spawns entities, at most
//!   [`WorldConfig::spawns_per_frame`] cells per frame; the GPU upload is done by Bevy's render
//!   asset preparation.
//!
//! Terrain patches pick their level of detail from the distance to the camera and rebuild meshes on
//! the task pool when the level changes.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};

use crate::coords::{CELL_SIZE, bevy_to_cry, cell_origin, cry_to_bevy};
use crate::cover::Cover;
use crate::heightmap::CellHeights;
use crate::terrain::{
    LOD_COUNT, PATCH_UNITS, PATCHES, build_patch_mesh, cover_image, lod_for_distance,
};
use crate::world::{World, cell_dir};

/// Plugin configuration (also a resource, adjustable at run time).
#[derive(Resource, Clone, Debug)]
pub struct WorldConfig {
    /// `main_world` or `login2`.
    pub world: String,
    /// Cells whose square is within this distance (metres) of the focus are loaded.
    pub load_radius: f32,
    /// Cells farther than this are unloaded (hysteresis against `load_radius`).
    pub unload_radius: f32,
    /// Cell loads running at once.
    pub max_loads: usize,
    /// Finished cells turned into entities per frame.
    pub spawns_per_frame: usize,
    /// Level of detail changes started per frame.
    pub lod_updates_per_frame: usize,
    /// Spawn brushes (static objects).
    pub objects: bool,
    /// Spawn vegetation.
    pub vegetation: bool,
    /// Brushes are drawn at most this far (m); each also has its own view distance.
    pub brush_distance: f32,
    /// Vegetation is drawn at most this far (m).
    pub vegetation_distance: f32,
    /// Water volumes (rivers, lakes, ponds) are loaded within this distance (m).
    pub water_distance: f32,
    /// Main thread time per frame for loading models and spawning object sectors (ms).
    pub object_budget_ms: f32,
    /// Drive sun, ambient light, fog, sky dome and ocean from the time of day data.
    pub environment: bool,
    /// Radius of the sky dome (m); the camera far plane must exceed it.
    pub sky_radius: f32,
    /// Sun shadows (cascaded).
    pub shadows: bool,
    /// Shadow distance (m).
    pub shadow_distance: f32,
    /// Lux per unit of `Sun color * multiplier` luminance (noon is about 6).
    pub sun_lux_scale: f32,
    /// Ambient brightness per unit of sky / ground colour luminance.
    pub ambient_scale: f32,
    /// Fog visibility distance (m) at the reference fog density.
    pub fog_visibility: f32,
    /// Linear colour factor on the terrain's cover texture. The engine multiplies the colour map with detail
    /// layers and a filmic curve we do not reproduce; this brings the result to the real client's brightness.
    pub terrain_tint: Vec3,
}

impl WorldConfig {
    pub fn new(world: &str) -> Self {
        WorldConfig {
            world: world.to_string(),
            load_radius: 3000.0,
            unload_radius: 3600.0,
            max_loads: 3,
            spawns_per_frame: 1,
            lod_updates_per_frame: 6,
            objects: true,
            vegetation: true,
            brush_distance: 1000.0,
            vegetation_distance: 220.0,
            water_distance: 1400.0,
            object_budget_ms: 3.0,
            environment: true,
            sky_radius: 9000.0,
            shadows: true,
            shadow_distance: 300.0,
            sun_lux_scale: 1700.0,
            ambient_scale: 1700.0,
            fog_visibility: 2500.0,
            terrain_tint: Vec3::new(0.3, 0.32, 0.45),
        }
    }
}

/// Marks the entity whose position drives streaming (default: the first `Camera3d`).
#[derive(Component)]
pub struct WorldFocus;

/// Adds world streaming + terrain. Camera and window are the application's.
pub struct AaWorldPlugin {
    pub config: WorldConfig,
}

impl AaWorldPlugin {
    pub fn new(world: &str) -> Self {
        AaWorldPlugin { config: WorldConfig::new(world) }
    }
}

impl Plugin for AaWorldPlugin {
    fn build(&self, app: &mut App) {
        let (tx, rx) = channel();
        let world = World::load(&self.config.world).unwrap_or_else(|e| {
            warn!("aa-world: cannot read world {}: {e}", self.config.world);
            World { name: self.config.world.clone(), ..default() }
        });
        if !app.is_plugin_added::<aa_cry::CryPlugin>() {
            app.add_plugins(aa_cry::CryPlugin);
        }
        app.insert_resource(self.config.clone())
            .insert_resource(WorldInfo(Arc::new(world)))
            .init_resource::<crate::objects::ObjectState>()
            .insert_resource(Streamer { cells: HashMap::new(), loading: HashSet::new(), missing: HashSet::new(), tx, rx: Mutex::new(rx) })
            .add_systems(Update, (stream_cells, receive_cells, update_patch_lod, crate::objects::stream_objects).chain());
        crate::sky::build(app);
    }
}

/// The world description (`world.g`).
#[derive(Resource, Clone)]
pub struct WorldInfo(pub Arc<World>);

struct LoadedCell {
    root: Entity,
}

/// What a finished background load delivers.
struct CellLoad {
    heights: Arc<CellHeights>,
    cover: Option<crate::cover::CoverAtlas>,
    /// `(patch index, lod, mesh)` for the 16 patches.
    meshes: Vec<(usize, usize, Mesh)>,
}

#[derive(Resource)]
pub struct Streamer {
    cells: HashMap<(i32, i32), LoadedCell>,
    loading: HashSet<(i32, i32)>,
    /// Cells with no terrain data in the pak.
    missing: HashSet<(i32, i32)>,
    tx: Sender<((i32, i32), Option<CellLoad>)>,
    rx: Mutex<Receiver<((i32, i32), Option<CellLoad>)>>,
}

impl Streamer {
    pub fn loaded_cells(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        self.cells.keys().copied()
    }

    pub fn is_loading(&self) -> bool {
        !self.loading.is_empty()
    }

    pub fn loaded_count(&self) -> usize {
        self.cells.len()
    }
}

/// One terrain patch (256 m) of a loaded cell.
#[derive(Component)]
pub struct TerrainPatch {
    pub cell: (i32, i32),
    pub index: usize,
    pub lod: usize,
    heights: Arc<CellHeights>,
    /// Bevy-space bounds.
    min: Vec3,
    max: Vec3,
    build: Option<(usize, Task<Mesh>)>,
}

/// Root entity of a loaded cell; children are terrain patches and objects.
#[derive(Component)]
pub struct CellRoot {
    pub cell: (i32, i32),
}

fn focus_position(
    focus: &Query<&GlobalTransform, With<WorldFocus>>,
    cameras: &Query<&GlobalTransform, With<Camera3d>>,
) -> Option<Vec3> {
    focus.iter().next().or_else(|| cameras.iter().next()).map(|t| t.translation())
}

/// Distance from `p` to the rectangle of cell `(x, y)` (cry space, ignoring height).
fn cell_distance(p: Vec3, x: i32, y: i32) -> f32 {
    let o = cell_origin(x, y);
    let dx = (o.x - p.x).max(0.0).max(p.x - (o.x + CELL_SIZE));
    let dy = (o.y - p.y).max(0.0).max(p.y - (o.y + CELL_SIZE));
    (dx * dx + dy * dy).sqrt()
}

#[allow(clippy::too_many_arguments)]
fn stream_cells(
    mut commands: Commands,
    cfg: Res<WorldConfig>,
    info: Res<WorldInfo>,
    mut streamer: ResMut<Streamer>,
    focus: Query<&GlobalTransform, With<WorldFocus>>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
) {
    let Some(p_bevy) = focus_position(&focus, &cameras) else { return };
    let p = bevy_to_cry(p_bevy);

    // unload
    let far: Vec<(i32, i32)> = streamer
        .cells
        .keys()
        .copied()
        .filter(|&(x, y)| cell_distance(p, x, y) > cfg.unload_radius)
        .collect();
    for c in far {
        if let Some(cell) = streamer.cells.remove(&c) {
            commands.entity(cell.root).despawn();
        }
    }

    // load: the closest wanted cells first
    if streamer.loading.len() >= cfg.max_loads {
        return;
    }
    let r = (cfg.load_radius / CELL_SIZE).ceil() as i32 + 1;
    let (cx, cy) = ((p.x / CELL_SIZE).floor() as i32, (p.y / CELL_SIZE).floor() as i32);
    let mut wanted: Vec<(f32, (i32, i32))> = Vec::new();
    for y in cy - r..=cy + r {
        for x in cx - r..=cx + r {
            if x < 0 || y < 0 || (info.0.cells_x > 0 && (x >= info.0.cells_x || y >= info.0.cells_y)) {
                continue;
            }
            let c = (x, y);
            if streamer.cells.contains_key(&c) || streamer.loading.contains(&c) || streamer.missing.contains(&c) {
                continue;
            }
            let d = cell_distance(p, x, y);
            if d <= cfg.load_radius {
                wanted.push((d, c));
            }
        }
    }
    wanted.sort_by(|a, b| a.0.total_cmp(&b.0));
    let pool = AsyncComputeTaskPool::get();
    for (_, c) in wanted {
        if streamer.loading.len() >= cfg.max_loads {
            break;
        }
        let dir = cell_dir(&cfg.world, c.0, c.1);
        if !aa_pak::Pak::global().exists(&format!("{dir}/client/terrain/heightmap.dat")) {
            streamer.missing.insert(c);
            continue;
        }
        streamer.loading.insert(c);
        let tx = streamer.tx.clone();
        pool.spawn(async move {
            let _ = tx.send((c, load_cell(&dir, c, p_bevy)));
        })
        .detach();
    }
}

/// Background work: decode terrain files and build the patch meshes. `None` when the cell is unusable.
fn load_cell(dir: &str, cell: (i32, i32), focus: Vec3) -> Option<CellLoad> {
    let pak = aa_pak::Pak::global();
    let hm = pak.read(&format!("{dir}/client/terrain/heightmap.dat")).ok()?;
    let heights = match CellHeights::parse(&hm) {
        Ok(h) => Arc::new(h),
        Err(e) => {
            warn!("aa-world: {dir}: heightmap: {e}");
            return None;
        }
    };
    let cover = pak.read(&format!("{dir}/client/terrain/cover.ctc")).ok().and_then(|bytes| {
        match Cover::parse(&bytes) {
            Ok(c) => Some(c.atlas(0)),
            Err(e) => {
                warn!("aa-world: {dir}: cover: {e}");
                None
            }
        }
    });
    let mut meshes = Vec::with_capacity(PATCHES * PATCHES);
    for index in 0..PATCHES * PATCHES {
        let (min, max) = patch_bounds(cell, &heights, index);
        let lod = lod_for_distance(distance_to_box(focus, min, max), None);
        meshes.push((index, lod, build_patch_mesh(&heights, index % PATCHES, index / PATCHES, lod)));
    }
    Some(CellLoad { heights, cover, meshes })
}

/// Bevy-space bounding box of patch `index` (`py * 4 + px` order: `index % 4` = patch x).
fn patch_bounds(cell: (i32, i32), h: &CellHeights, index: usize) -> (Vec3, Vec3) {
    let (px, py) = (index % PATCHES, index / PATCHES);
    let size = PATCH_UNITS as f32 * 2.0;
    let o = cell_origin(cell.0, cell.1) + Vec3::new(px as f32 * size, py as f32 * size, 0.0);
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for j in (0..=PATCH_UNITS).step_by(8) {
        for i in (0..=PATCH_UNITS).step_by(8) {
            let z = h.at(px * PATCH_UNITS + i, py * PATCH_UNITS + j);
            lo = lo.min(z);
            hi = hi.max(z);
        }
    }
    // cry box [o, o + size] x [lo, hi] -> bevy
    let a = cry_to_bevy(Vec3::new(o.x, o.y, lo));
    let b = cry_to_bevy(Vec3::new(o.x + size, o.y + size, hi));
    (a.min(b), a.max(b))
}

fn distance_to_box(p: Vec3, min: Vec3, max: Vec3) -> f32 {
    (min - p).max(Vec3::ZERO).max(p - max).length()
}

fn receive_cells(
    mut commands: Commands,
    cfg: Res<WorldConfig>,
    mut streamer: ResMut<Streamer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for _ in 0..cfg.spawns_per_frame {
        let msg = streamer.rx.lock().unwrap().try_recv();
        let Ok((cell, msg)) = msg else { break };
        streamer.loading.remove(&cell);
        let Some(load) = msg else {
            streamer.missing.insert(cell); // unreadable: do not retry
            continue;
        };
        if streamer.cells.contains_key(&cell) {
            continue;
        }
        let material = materials.add(StandardMaterial {
            base_color_texture: load.cover.map(|atlas| images.add(cover_image(atlas))),
            base_color: Color::linear_rgb(cfg.terrain_tint.x, cfg.terrain_tint.y, cfg.terrain_tint.z),
            perceptual_roughness: 1.0,
            reflectance: 0.05,
            ..default()
        });
        let origin = cry_to_bevy(cell_origin(cell.0, cell.1));
        let size = PATCH_UNITS as f32 * 2.0;
        let root = commands
            .spawn((CellRoot { cell }, Transform::from_translation(origin), Visibility::default()))
            .with_children(|parent| {
                for (index, lod, mesh) in load.meshes {
                    let (px, py) = (index % PATCHES, index / PATCHES);
                    let (min, max) = patch_bounds(cell, &load.heights, index);
                    parent.spawn((
                        Mesh3d(meshes.add(mesh)),
                        MeshMaterial3d(material.clone()),
                        Transform::from_translation(cry_to_bevy(Vec3::new(px as f32 * size, py as f32 * size, 0.0))),
                        TerrainPatch { cell, index, lod, heights: load.heights.clone(), min, max, build: None },
                    ));
                }
            })
            .id();
        streamer.cells.insert(cell, LoadedCell { root });
    }
}

fn update_patch_lod(
    cfg: Res<WorldConfig>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut patches: Query<(&mut TerrainPatch, &mut Mesh3d)>,
    focus: Query<&GlobalTransform, With<WorldFocus>>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
) {
    let Some(p) = focus_position(&focus, &cameras) else { return };
    let pool = AsyncComputeTaskPool::get();
    let mut started = 0;
    for (mut patch, mut mesh3d) in &mut patches {
        // finished rebuild?
        if let Some((lod, task)) = &mut patch.build {
            if let Some(mesh) = check_ready(task) {
                let lod = *lod;
                mesh3d.0 = meshes.add(mesh);
                patch.lod = lod;
                patch.build = None;
            }
            continue;
        }
        if started >= cfg.lod_updates_per_frame {
            continue;
        }
        let d = distance_to_box(p, patch.min, patch.max);
        let want = lod_for_distance(d, Some(patch.lod)).min(LOD_COUNT - 1);
        if want != patch.lod {
            let heights = patch.heights.clone();
            let index = patch.index;
            patch.build = Some((
                want,
                pool.spawn(async move { build_patch_mesh(&heights, index % PATCHES, index / PATCHES, want) }),
            ));
            started += 1;
        }
    }
}
