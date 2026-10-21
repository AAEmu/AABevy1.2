//! Cell objects: brushes (`brush.dat`), static entity slots and vegetation as meshes, spawned per 64 m sector near the camera.
//!
//! * placement files are parsed on background tasks and binned per sector;
//! * models are prepared (parsed, textures decoded, materials and meshes built) on the worker threads of
//!   [`aa_cry::workers`], nearest first, and only inserted into the asset collections on the main thread
//!   ([`CryAssets::finish_model`]); an instance whose model is not ready yet waits in its sector and appears when the
//!   model arrives, so walking into new country never stalls a frame on a file;
//! * the main thread spawns sectors nearest first under a time budget per frame
//!   ([`WorldConfig::object_budget_ms`]);
//! * a sector is alive while the camera is within the largest view distance of its instances (the compiled
//!   `maxViewDist` times [`WorldConfig::view_dist_scale`], capped by [`WorldConfig::brush_distance`] /
//!   [`WorldConfig::vegetation_distance`] / [`WorldConfig::tree_distance`]); every part also carries a
//!   [`VisibilityRange`] for its own distance, so the renderer drops far instances without our help;
//! * distant objects: brushes that are seen beyond [`NEAR_BRUSH_DISTANCE`] (castles, walls, big rocks) are binned
//!   apart ([`Kind::FarBrush`]), so their sectors live out to `brush_distance` (the original `e_max_view_dst`)
//!   without spawning the small props next to them; far away they request the authored lowest `_lodN` meshes;
//! * level of detail: brushes retain their authored ratio and use the normal camera's native requested level;
//!   other objects use distance bands. A sector is respawned
//!   when the band changes (the new sector is built hidden next to the old one and swapped in when complete);
//! * water volumes (rivers, lakes, ponds) of a cell are merged into one flat mesh per water style (material and
//!   fog), built on the task, drawn with `water_material::WaterMaterial`;
//! * roads are terrain triangles clipped to the original spline boundaries on the placement task, with
//!   cached diffuse/normal/opacity materials and the same cell lifetime as the water;
//! * static decals are planar quads or project onto terrain/visible brush triangles, built once on the
//!   placement task and released with their cell;
//! * instancing: all parts of one model share mesh and material handles, which Bevy batches into instanced
//!   draws. Parts are flattened (the node transform is baked) so an instance costs one entity per drawn part.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use aa_cry::scene::LoadedModel;
use aa_cry::workers::Workers;
use aa_cry::{CryAssets, ModelOptions, PreparedModel};
use bevy::camera::visibility::{RenderLayers, VisibilityRange};
use bevy::light::NotShadowCaster;
use bevy::mesh::MeshTag;
use crate::vegetation_color::{self, VegetationMaterial, VegetationColorPart};
use bevy::math::{Affine3A, Vec3A};
use bevy::prelude::*;
use bevy::render::render_resource::TextureViewDimension;

use crate::coords::{CRY_TO_BEVY, CELL_SIZE, SECTOR_SIZE, bevy_to_cry, cell_origin, cry_to_bevy};
use crate::placement::{CellObjects, load_cell_objects};
use crate::roads::{RoadAssets, RoadMaterial, RoadMesh};
use crate::stream::{Streamer, WorldConfig, WorldFocus};
use crate::vegetation::VegGroups;
use crate::water::{WaterStyle, water_meshes};
use crate::water_material::{WaterMaterial, WaterStyles};

pub mod effects;
pub mod sounds;
pub mod lights;
pub mod rigid;
pub mod voxels;
pub mod lod;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Brush,
    /// Brushes whose own view distance exceeds [`NEAR_BRUSH_DISTANCE`], drawn up to [`WorldConfig::brush_distance`].
    FarBrush,
    /// Static `entities.xml` render slots; separate from brush/vegetation records.
    Entity,
    /// Playing rigid CGA render slots from `entities.xml`.
    Animated,
    /// Small vegetation (instances whose own view distance is at most [`WorldConfig::vegetation_distance`]).
    Vegetation,
    /// Large vegetation (trees): own view distance beyond `vegetation_distance`, drawn up to
    /// [`WorldConfig::tree_distance`].
    Trees,
}

/// Brushes seen farther than this go to [`Kind::FarBrush`] (when [`WorldConfig::brush_distance`] is larger).
pub const NEAR_BRUSH_DISTANCE: f32 = 1000.0;

/// How far instances are drawn (m), taken from [`WorldConfig`] when a cell's placement is binned.
#[derive(Clone, Copy, Debug)]
pub struct ViewLimits {
    /// [`WorldConfig::view_dist_scale`].
    pub scale: f32,
    /// Cap of [`Kind::Brush`], static entity slots and animated props.
    pub near: f32,
    /// [`WorldConfig::brush_distance`], the cap of [`Kind::FarBrush`].
    pub far: f32,
    pub vegetation: f32,
    pub trees: f32,
}

impl ViewLimits {
    pub fn new(cfg: &WorldConfig) -> Self {
        ViewLimits {
            scale: cfg.view_dist_scale,
            near: NEAR_BRUSH_DISTANCE.min(cfg.brush_distance),
            far: cfg.brush_distance,
            vegetation: cfg.vegetation_distance,
            trees: cfg.tree_distance,
        }
    }

    /// The engine's view distance of an instance: compiled `maxViewDist` x scale (records without one use `default`).
    fn own(&self, max_view_dist: f32, default: f32) -> f32 {
        if max_view_dist > 1.0 { max_view_dist * self.scale } else { default }
    }

    /// A brush's view distance and its kind.
    pub fn brush(&self, b: &crate::placement::Brush) -> (f32, Kind) {
        let view = self.own(b.max_view_dist, self.near);
        if view > self.near { (view.min(self.far), Kind::FarBrush) } else { (view, Kind::Brush) }
    }

    /// A vegetation instance's view distance and its kind. Small plants keep their compiled distance (the scale
    /// would add half again as many grass and bush entities near the camera, for no visible gain).
    pub fn vegetation(&self, v: &crate::placement::Vegetation) -> (f32, Kind) {
        let view = self.own(v.max_view_dist, self.vegetation);
        if view > self.vegetation {
            (view.min(self.trees), Kind::Trees)
        } else {
            let compiled = if v.max_view_dist > 1.0 { v.max_view_dist } else { self.vegetation };
            (compiled, Kind::Vegetation)
        }
    }
}

impl Default for ViewLimits {
    fn default() -> Self {
        Self::new(&WorldConfig::new("main_world"))
    }
}

/// Number of [`Kind`]s, for per-kind arrays.
const KINDS: usize = 6;

/// Placement data of one cell, binned by 64 m sector.
pub struct CellObjectData {
    pub objects: CellObjects,
    pub limits: ViewLimits,
    /// Largest view distance of the instances of a sector's bin, `[sector][kind as usize]`.
    pub views: Vec<[f32; KINDS]>,
    /// Instance indices per sector (`sx * 16 + sy`).
    pub brush_bins: Vec<Vec<u32>>,
    pub far_brush_bins: Vec<Vec<u32>>,
    pub entity_bins: Vec<Vec<u32>>,
    pub animated_bins: Vec<Vec<u32>>,
    pub veg_bins: Vec<Vec<u32>>,
    pub tree_bins: Vec<Vec<u32>>,
}

impl CellObjectData {
    pub fn new(objects: CellObjects, limits: &ViewLimits) -> Self {
        let limits = *limits;
        let mut views = vec![[0.0f32; KINDS]; 256];
        let mut brush_bins = vec![Vec::new(); 256];
        let mut far_brush_bins = vec![Vec::new(); 256];
        let mut entity_bins = vec![Vec::new(); 256];
        let mut animated_bins = vec![Vec::new(); 256];
        let mut veg_bins = vec![Vec::new(); 256];
        let mut tree_bins = vec![Vec::new(); 256];
        let bin = |x: f32, y: f32| {
            let sx = ((x / SECTOR_SIZE) as usize).min(15);
            let sy = ((y / SECTOR_SIZE) as usize).min(15);
            sx * 16 + sy
        };
        for (i, b) in objects.brushes.iter().enumerate() {
            let t = b.translation();
            let (view, kind) = limits.brush(b);
            let sector = bin(t.x, t.y);
            views[sector][kind as usize] = views[sector][kind as usize].max(view);
            (if kind == Kind::FarBrush { &mut far_brush_bins } else { &mut brush_bins })[sector].push(i as u32);
        }
        // Avoid drawing a slot twice if an export also includes its identical brush. Compare
        // once on the placement worker, not while deciding sector visibility every frame.
        let brush_paths: Vec<_> = objects.models.iter().map(|p| crate::entities::asset_path(p)).collect();
        for (i, prop) in objects.entity_props.iter().enumerate() {
            if prop.hidden { continue; }
            let duplicate = objects.brushes.iter().any(|brush| {
                let material = usize::try_from(brush.material).ok().and_then(|i| objects.materials.get(i))
                    .filter(|m| !m.is_empty()).map(|m| {
                        let path = crate::entities::asset_path(m);
                        if path.ends_with(".mtl") { path } else { format!("{path}.mtl") }
                    });
                brush_paths.get(brush.model as usize) == Some(&prop.model)
                    && brush.mat4().abs_diff_eq(prop.matrix, 1e-4)
                    && material == prop.material
            });
            if !duplicate {
                let t = prop.translation();
                let sector = bin(t.x, t.y);
                views[sector][Kind::Entity as usize] = limits.near;
                entity_bins[sector].push(i as u32);
            }
        }
        for (i, v) in objects.vegetation.iter().enumerate() {
            let (view, kind) = limits.vegetation(v);
            let sector = bin(v.pos.x, v.pos.y);
            views[sector][kind as usize] = views[sector][kind as usize].max(view);
            (if kind == Kind::Trees { &mut tree_bins } else { &mut veg_bins })[sector].push(i as u32);
        }
        for (i, prop) in objects.animated_props.iter().enumerate().filter(|(_, p)| !p.hidden) {
            let t = prop.translation();
            let sector = bin(t.x, t.y);
            views[sector][Kind::Animated as usize] = limits.near;
            animated_bins[sector].push(i as u32);
        }
        CellObjectData { objects, limits, views, brush_bins, far_brush_bins, entity_bins, animated_bins, veg_bins, tree_bins }
    }
}

/// A sector's entity (children are the drawn parts).
#[derive(Component)]
pub struct ObjectSector {
    pub cell: (i32, i32),
    pub sector: u8,
    pub kind: Kind,
}

/// Marks a drawn part of a static `entities.xml` slot (not an editor helper or actor).
#[derive(Component, Clone, Copy, Debug)]
pub struct StaticEntityPart {
    pub cell: (i32, i32),
    pub guid: u64,
}

struct SectorState {
    entity: Entity,
    /// Where the sector's root is (Bevy space).
    translation: Vec3,
    lod: u8,
    brush_lods: Option<lod::BrushLods>,
    /// Next index into the sector's bin (spawning resumes here).
    next: usize,
    spawned: usize,
    /// Instances (bin index, model) whose model is not ready yet.
    waiting: Vec<(u32, u32)>,
    /// The sector is built hidden (after a level of detail change) until it is complete.
    hidden: bool,
    /// Roots of the previous level of detail, dropped when this one is complete.
    retired: Vec<Entity>,
}

impl SectorState {
    fn complete(&self, bin_len: usize) -> bool {
        self.next >= bin_len && self.waiting.is_empty()
    }
}

/// What an instance needs from the model files: the model, its material file and the level of detail.
#[derive(Clone, PartialEq, Eq, Hash)]
struct ModelKey {
    path: String,
    material: Option<String>,
    lod: u8,
}

/// A model ready to be instanced: the loaded assets and the model space transform of each drawn node.
struct Instanced {
    lod: u8,
    loaded: Arc<LoadedModel>,
    nodes: Vec<Mat4>,
    rigid: Option<Arc<rigid::Clip>>,
    color_materials: vegetation_color::Materials,
    ao_meshes: vegetation_color::ao::Meshes,
}

enum ModelState {
    /// Not asked for yet (the workers are busy).
    Wanted,
    /// Being prepared on a worker.
    Loading,
    Ready(Arc<Instanced>),
    Failed,
}

/// Models by id; instances refer to them by id.
#[derive(Default)]
struct Models {
    keys: Vec<ModelKey>,
    states: Vec<ModelState>,
    by_key: HashMap<ModelKey, u32>,
    /// `(cell, kind, model/group/entity index, material, lod)` -> id: no per-frame strings.
    fast: HashMap<(i32, i32, Kind, u32, i32, u8), u32>,
}

impl Models {
    fn id(&mut self, key: ModelKey) -> u32 {
        if let Some(&id) = self.by_key.get(&key) {
            return id;
        }
        let id = self.keys.len() as u32;
        self.keys.push(key.clone());
        self.states.push(ModelState::Wanted);
        self.by_key.insert(key, id);
        id
    }
}

/// A model prepared by a worker.
type ModelResult = (u32, Result<(PreparedModel, Option<Arc<rigid::Clip>>, Vec<vegetation_color::ao::Prepared>), aa_cry::Error>);

/// Most model jobs queued on the workers at once; the rest wait their turn in `Wanted`, so the queue holds what is near.
const MAX_QUEUED_MODELS: usize = 48;

#[derive(Resource)]
pub struct ObjectState {
    cells: HashMap<(i32, i32), Arc<CellObjectData>>,
    loading: HashSet<(i32, i32)>,
    sectors: HashMap<(i32, i32, u8, Kind), SectorState>,
    models: Models,
    model_tx: Sender<ModelResult>,
    model_rx: Mutex<Receiver<ModelResult>>,
    /// Prepared models not inserted yet (the per frame budget).
    model_queue: VecDeque<ModelResult>,
    /// Loaded models (by `Arc` address) already checked for unusable textures.
    checked_models: HashSet<usize>,
    groups: Option<Arc<VegGroups>>,
    groups_rx: Mutex<Receiver<Option<VegGroups>>>,
    groups_requested: bool,
    tx: Sender<CellMessage>,
    rx: Mutex<Receiver<CellMessage>>,
    /// The water mesh entities of each cell (one per water style).
    water: HashMap<(i32, i32), Vec<Entity>>,
    /// Terrain road entities, released with their owning placement cell.
    roads: HashMap<(i32, i32), Vec<Entity>>,
    decals: HashMap<(i32, i32), Vec<Entity>>,
    voxels: HashMap<(i32, i32), voxels::CellVoxels>,
    road_assets: RoadAssets,
    color_mirrors: vegetation_color::Mirrors,
    ao_mirrors: vegetation_color::ao::Mirrors,
    /// The particle effects of each cell ([`effects`]).
    effects: HashMap<(i32, i32), Vec<Entity>>,
    /// The placed sounds of each cell ([`sounds`]).
    sounds: HashMap<(i32, i32), Vec<Entity>>,
    lights: HashMap<(i32, i32), Vec<Entity>>,
    fog: HashMap<(i32, i32), crate::fog::CellVolumes>,
    groups_tx: Sender<Option<VegGroups>>,
    /// Counter for [`ObjectStats::entities`] while models are still being prepared.
    busy_ticks: usize,
    /// Round-robin starting sector for bounded completed-brush LOD checks.
    lod_sector_cursor: usize,
    /// Counters for diagnostics.
    pub stats: ObjectStats,
}

/// What a placement task delivers: binned objects, water meshes and prepared roads of the cell.
type CellMessage = ((i32, i32), CellObjectData, Vec<(WaterStyle, Mesh)>, Vec<RoadMesh>, Vec<RoadMesh>, voxels::PreparedCell);

#[derive(Default, Clone, Debug)]
pub struct ObjectStats {
    pub sectors: usize,
    /// Entities spawned for objects. While models are still being prepared a counter that changes every frame is added,
    /// so a loading screen that waits for this number to settle keeps waiting.
    pub entities: usize,
    /// Models being prepared or waiting for a worker.
    pub models_pending: usize,
    pub models_failed: usize,
    pub spawn_ms: f32,
    pub brush_lod_checks: usize,
    pub brush_lod_instances: usize,
    pub brush_lod_rebuilds: usize,
    pub models_cached: usize,
    pub water_cells: usize,
    pub roads: usize,
    /// Drawn decal meshes (a combined terrain/object decal can produce two).
    pub decals: usize,
    pub voxels: usize,
    pub voxel_triangles: usize,
    pub voxels_unsupported: usize,
    pub voxel_layers: usize,
    pub voxel_layers_unsupported: usize,
    /// Drawn parts of static `entities.xml` slots, also included in `entities`.
    pub entity_props: usize,
    /// Drawn parts of authored playing CGAs, also included in `entities`.
    pub animated_props: usize,
    pub fog_volumes: usize,
    /// Active visible slots declined by the static local-colour/HDR0 path.
    pub fog_unsupported: usize,
    pub fog_over_budget: usize,
    /// Terrain cells in reach whose placement files have not arrived yet (+1 while the vegetation groups load).
    pub cells_pending: usize,
    /// Sectors in range not completely drawn yet, and the instances waiting for their model.
    pub sectors_incomplete: usize,
    pub instances_waiting: usize,
}

impl ObjectState {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        let (groups_tx, groups_rx) = channel();
        let (model_tx, model_rx) = channel();
        ObjectState {
            cells: HashMap::new(),
            loading: HashSet::new(),
            sectors: HashMap::new(),
            models: Models::default(),
            model_tx,
            model_rx: Mutex::new(model_rx),
            model_queue: VecDeque::new(),
            checked_models: HashSet::new(),
            groups: None,
            groups_rx: Mutex::new(groups_rx),
            groups_requested: false,
            tx,
            rx: Mutex::new(rx),
            water: HashMap::new(),
            roads: HashMap::new(),
            decals: HashMap::new(),
            voxels: HashMap::new(),
            road_assets: RoadAssets::default(),
            color_mirrors: vegetation_color::Mirrors::default(),
            ao_mirrors: vegetation_color::ao::Mirrors::default(),
            effects: HashMap::new(),
            sounds: HashMap::new(),
            lights: HashMap::new(),
            fog: HashMap::new(),
            groups_tx,
            busy_ticks: 0,
            lod_sector_cursor: 0,
            stats: ObjectStats::default(),
        }
    }
}

impl Default for ObjectState {
    fn default() -> Self {
        Self::new()
    }
}

impl ObjectState {
    /// Despawns every cell, sector, placed object and light/sound/fog/voxel of the state and
    /// releases the cell assets it owns. Used by a runtime world switch; the state is left empty.
    pub fn unload_all(&mut self, world: &mut World) {
        fn despawn(world: &mut World, entity: Entity) {
            if world.get_entity(entity).is_ok() {
                world.despawn(entity);
            }
        }
        for (_, mut voxels) in self.voxels.drain() {
            voxels.release_world(world);
        }
        for (_, mut fog) in self.fog.drain() {
            fog.release_world(world);
        }
        for (_, entities) in self.water.drain() {
            for entity in entities {
                despawn(world, entity);
            }
        }
        for (_, entities) in self.roads.drain() {
            for entity in entities {
                despawn(world, entity);
            }
        }
        for (_, entities) in self.decals.drain() {
            for entity in entities {
                despawn(world, entity);
            }
        }
        for (_, entities) in self.effects.drain() {
            for entity in entities {
                despawn(world, entity);
            }
        }
        for (_, entities) in self.sounds.drain() {
            for entity in entities {
                despawn(world, entity);
            }
        }
        for (_, entities) in self.lights.drain() {
            for entity in entities {
                despawn(world, entity);
            }
        }
        for (_, sector) in self.sectors.drain() {
            despawn(world, sector.entity);
            for entity in sector.retired {
                despawn(world, entity);
            }
        }
        self.cells.clear();
        self.loading.clear();
        self.model_queue.clear();
        self.models = Models::default();
        self.checked_models.clear();
        self.groups = None;
        self.groups_requested = false;
        self.color_mirrors = vegetation_color::Mirrors::default();
        self.ao_mirrors = vegetation_color::ao::Mirrors::default();
        self.busy_ticks = 0;
        self.lod_sector_cursor = 0;
        self.stats = ObjectStats::default();
    }
}

/// Distance from the camera (cry space) to the rectangle of a sector.
fn sector_distance(p: Vec3, cell: (i32, i32), sector: usize) -> f32 {
    let o = cell_origin(cell.0, cell.1) + Vec3::new((sector / 16) as f32 * SECTOR_SIZE, (sector % 16) as f32 * SECTOR_SIZE, 0.0);
    let dx = (o.x - p.x).max(0.0).max(p.x - (o.x + SECTOR_SIZE));
    let dy = (o.y - p.y).max(0.0).max(p.y - (o.y + SECTOR_SIZE));
    (dx * dx + dy * dy).sqrt()
}

/// LOD for a distance band, with hysteresis around the current level.
fn object_lod(kind: Kind, d: f32, current: Option<u8>) -> u8 {
    if kind == Kind::Animated { return 0; }
    let bands: [f32; 2] = match kind {
        Kind::Brush | Kind::FarBrush | Kind::Entity => [140.0, 380.0],
        Kind::Animated => unreachable!(),
        Kind::Vegetation => [55.0, 130.0],
        Kind::Trees => [150.0, 350.0],
    };
    if let Some(c) = current {
        let lo = if c == 0 { 0.0 } else { bands[c as usize - 1] * 0.85 };
        let hi = if c >= 2 { f32::MAX } else { bands[c as usize] * 1.15 };
        if d >= lo && d < hi {
            return c;
        }
    }
    if d < bands[0] {
        0
    } else if d < bands[1] {
        1
    } else {
        2
    }
}

/// Model-space transform of a node as aa-cry's spawner builds it: parent chain, LOD nodes in their parent's frame.
fn node_world(model: &aa_cry::Model, node: usize) -> Affine3A {
    let n = &model.nodes[node];
    let local = if matches!(n.kind, aa_cry::mesh::NodeKind::Lod(_)) { Affine3A::IDENTITY } else { n.local };
    match n.parent {
        Some(p) if p != node => node_world(model, p) * local,
        _ => local,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn stream_objects(
    mut commands: Commands,
    cfg: Res<WorldConfig>,
    streamer: Res<Streamer>,
    mut state: ResMut<ObjectState>,
    mut cry: CryAssets,
    mut water_styles: ResMut<WaterStyles>,
    mut water_materials: ResMut<Assets<WaterMaterial>>,
    mut road_materials: ResMut<Assets<RoadMaterial>>,
    mut voxel_materials: ResMut<Assets<voxels::VoxelMaterial>>,
    mut vegetation_materials: ResMut<Assets<VegetationMaterial>>,
    mut fog_assets: crate::fog::FogAssets,
    terrain: Res<crate::TerrainQuery>,
    focus: Query<&GlobalTransform, With<WorldFocus>>,
    cameras: Query<(&Camera, &GlobalTransform, &Projection, Option<&RenderLayers>), With<Camera3d>>,
    timers: Option<Res<crate::profile::Timers>>,
    time: Res<Time>,
) {
    if !cfg.objects {
        return;
    }
    let Some(cam) = focus.iter().next().or_else(|| cameras.iter().next().map(|(_, t, _, _)| t)) else { return };
    let p = bevy_to_cry(cam.translation());
    // A portrait camera on another layer must not drive world LOD. Multiple world
    // views and orthographic projections retain the previous sector policy.
    let view = lod::select_view(cameras.iter());
    let state = &mut *state;
    let _t = timers.as_deref().map(|t| t.time("objects.stream"));
    // vegetation groups (world level)
    if !state.groups_requested {
        state.groups_requested = true;
        let world = cfg.world.clone();
        let tx = state.groups_tx.clone();
        // placement files, vegetation groups and terrain cells run on the workers, ahead of the models (priorities far below a
        // distance): Bevy's async compute pool is also where the pipeline compilation runs, and a first frame full of shaders
        // keeps it busy for seconds
        Workers::global().spawn(-60000.0, move || {
            let _ = tx.send(VegGroups::load(&world));
        });
    }
    if state.groups.is_none() {
        if let Ok(g) = state.groups_rx.lock().unwrap().try_recv() {
            state.groups = Some(Arc::new(g.unwrap_or_default()));
        }
    }

    // placement files of loaded terrain cells
    let limits = ViewLimits::new(&cfg);
    let reach = limits.far.max(cfg.vegetation_distance).max(cfg.tree_distance);
    let load_reach = reach.max(cfg.water_distance);
    let loaded: Vec<(i32, i32)> = streamer.loaded_cells().collect();
    state.cells.retain(|c, _| loaded.contains(c));
    let gone: Vec<_> = state.voxels.keys().filter(|c| !loaded.contains(c)).copied().collect();
    for c in gone {
        if let Some(voxels) = state.voxels.remove(&c) { voxels.release(&mut commands, &mut cry, &mut voxel_materials); }
    }
    let gone: Vec<_> = state.water.keys().filter(|c| !loaded.contains(c)).copied().collect();
    let fog_gone: Vec<_> = state.fog.keys().filter(|c| !loaded.contains(c)).copied().collect();
    for c in fog_gone {
        if let Some(fog) = state.fog.remove(&c) {
            fog.release(&mut commands, &mut cry.meshes, &mut fog_assets.materials);
        }
    }
    for c in gone {
        for e in state.water.remove(&c).unwrap_or_default() {
            commands.entity(e).despawn();
        }
    }
    let gone: Vec<_> = state.roads.keys().filter(|c| !loaded.contains(c)).copied().collect();
    for c in gone {
        for e in state.roads.remove(&c).unwrap_or_default() {
            commands.entity(e).despawn();
        }
    }
    let gone: Vec<_> = state.decals.keys().filter(|c| !loaded.contains(c)).copied().collect();
    for c in gone {
        for e in state.decals.remove(&c).unwrap_or_default() {
            commands.entity(e).despawn();
        }
    }
    let gone: Vec<_> = state.effects.keys().filter(|c| !loaded.contains(c)).copied().collect();
    for c in gone {
        for e in state.effects.remove(&c).unwrap_or_default() {
            commands.entity(e).despawn();
        }
    }
    let gone: Vec<_> = state.sounds.keys().filter(|c| !loaded.contains(c)).copied().collect();
    for c in gone {
        for e in state.sounds.remove(&c).unwrap_or_default() {
            commands.entity(e).despawn();
        }
    }
    let dead: Vec<_> = state.sectors.keys().filter(|k| !loaded.contains(&(k.0, k.1))).copied().collect();
    let gone: Vec<_> = state.lights.keys().filter(|c| !loaded.contains(c)).copied().collect();
    for c in gone {
        for e in state.lights.remove(&c).unwrap_or_default() { commands.entity(e).despawn(); }
    }
    for k in dead {
        if let Some(s) = state.sectors.remove(&k) {
            commands.entity(s.entity).despawn();
            for e in s.retired { commands.entity(e).despawn(); }
        }
    }
    for &c in &loaded {
        if state.cells.contains_key(&c) || state.loading.contains(&c) || state.loading.len() >= 2 {
            continue;
        }
        let o = cell_origin(c.0, c.1);
        let dx = (o.x - p.x).max(0.0).max(p.x - (o.x + CELL_SIZE));
        let dy = (o.y - p.y).max(0.0).max(p.y - (o.y + CELL_SIZE));
        if (dx * dx + dy * dy).sqrt() > load_reach + 200.0 {
            continue;
        }
        state.loading.insert(c);
        let tx = state.tx.clone();
        let world = cfg.world.clone();
        let distance = (dx * dx + dy * dy).sqrt();
        let terrain = terrain.clone();
        let loader = cry.loader();
        let road_assets = state.road_assets.clone();
        Workers::global().spawn(distance - 50000.0, move || {
            let objects = load_cell_objects(&world, c.0, c.1);
            let water = water_meshes(&objects.water, &objects.materials);
            let roads = crate::roads::prepare_cell(&objects, c, &terrain, &loader, &road_assets);
            let decals = crate::decals::prepare_cell(&world, &objects, c, &terrain, &loader, &road_assets);
            let voxels = voxels::prepare_cell(&world, c, &objects.voxels);
            let data = CellObjectData::new(objects, &limits);
            let _ = tx.send((c, data, water, roads, decals, voxels));
        });
    }
    let received: Vec<CellMessage> = {
        let rx = state.rx.lock().unwrap();
        std::iter::from_fn(|| rx.try_recv().ok()).collect()
    };
    for (c, data, water, roads, decals, voxels) in received {
        state.loading.remove(&c);
        if !loaded.contains(&c) {
            continue;
        }
        let mut entities = Vec::new();
        for (style, mesh) in water {
            let material = water_styles.material(&style, &mut water_materials, &mut cry.images);
            entities.push(
                commands
                    .spawn((
                        Mesh3d(cry.meshes.add(mesh)),
                        MeshMaterial3d(material),
                        crate::water::WaterSurface { cell: c },
                        Transform::from_translation(cry_to_bevy(cell_origin(c.0, c.1))),
                        NotShadowCaster,
                    ))
                    .id(),
            );
        }
        if !entities.is_empty() {
            state.water.insert(c, entities);
        }
        let mut entities = Vec::new();
        for road in roads {
            let early = road.early_surface();
            let Some(material) = road.material(&mut cry, &mut road_materials) else { continue };
            let end = road.distance.min(cfg.brush_distance).max(0.0) + road.radius;
            let mut entity = commands.spawn((
                Name::new("Terrain road"),
                Mesh3d(cry.meshes.add(road.mesh)),
                MeshMaterial3d(material),
                Transform::from_translation(cry_to_bevy(cell_origin(c.0, c.1))),
                VisibilityRange {
                    start_margin: 0.0..0.0,
                    end_margin: end..end + 20.0,
                    use_aabb: true,
                },
                NotShadowCaster,
            ));
            if early { entity.insert(crate::sky_material::composite::RoadSurfacePass); }
            entities.push(entity.id());
        }
        if !entities.is_empty() {
            state.roads.insert(c, entities);
        }
        let mut entities = Vec::new();
        for decal in decals {
            let deferred = decal.deferred.is_some();
            let forward = decal.early_forward_overlay();
            let Some(material) = decal.material(&mut cry, &mut road_materials) else { continue };
            let end = decal.distance.min(cfg.brush_distance).max(0.0) + decal.radius;
            let mut entity = commands.spawn((
                Name::new(if deferred { "Deferred terrain/object decal" } else { "Terrain/object decal" }),
                Mesh3d(cry.meshes.add(decal.mesh)),
                MeshMaterial3d(material),
                Transform::from_translation(cry_to_bevy(cell_origin(c.0, c.1))),
                VisibilityRange { start_margin: 0.0..0.0, end_margin: end..end + 20.0, use_aabb: true },
                NotShadowCaster,
            ));
            if deferred { entity.insert(crate::sky_material::composite::DeferredDecalPass); }
            else if forward { entity.insert(crate::sky_material::composite::ForwardDecalPass); }
            entities.push(entity.id());
        }
        if !entities.is_empty() { state.decals.insert(c, entities); }
        if !data.objects.voxels.is_empty() {
            state.voxels.insert(c, voxels::spawn_cell(&mut commands, &mut cry, &mut voxel_materials, c, voxels, cfg.brush_distance));
        }
        let placed = effects::spawn_cell(&mut commands, c, &data.objects.effects);
        if !placed.is_empty() {
            state.effects.insert(c, placed);
        }
        let placed = sounds::spawn_cell(&mut commands, c, &data.objects.sounds);
        if !placed.is_empty() {
            state.sounds.insert(c, placed);
        }
        let placed = lights::spawn_cell(&mut commands, c, &data.objects.lights);
        if !placed.is_empty() { state.lights.insert(c, placed); }
        if !data.objects.fog.is_empty() {
            let count = state.fog.values().map(|v| v.entities.len()).sum::<usize>();
            let fog = crate::fog::spawn_cell(&mut commands, &mut cry.meshes, &mut fog_assets.materials,
                c, &data.objects.fog, crate::fog::MAX_VOLUMES.saturating_sub(count), fog_assets.atmosphere.radiance);
            state.fog.insert(c, fog);
        }
        state.cells.insert(c, Arc::new(data));
    }
    state.stats.water_cells = state.water.len();
    state.stats.fog_volumes = state.fog.values().map(|v| v.entities.len()).sum();
    state.stats.fog_unsupported = state.fog.values().map(|v| v.unsupported).sum();
    state.stats.fog_over_budget = state.fog.values().map(|v| v.over_budget).sum();
    state.stats.roads = state.roads.values().map(Vec::len).sum();
    state.stats.decals = state.decals.values().map(Vec::len).sum();
    state.stats.voxels = state.voxels.values().map(|v| v.base_count).sum();
    state.stats.voxel_layers = state.voxels.values().map(|v| v.layer_count).sum();
    state.stats.voxel_layers_unsupported = state.voxels.values().map(|v| v.layers_unsupported).sum();
    state.stats.voxel_triangles = state.voxels.values().map(|v| v.triangles).sum();
    state.stats.voxels_unsupported = state.voxels.values().map(|v| v.unsupported).sum();
    // placement files still to come for the cells in reach (and the vegetation groups), for a loading screen
    state.stats.cells_pending = loaded
        .iter()
        .filter(|c| !state.cells.contains_key(c))
        .filter(|c| {
            let o = cell_origin(c.0, c.1);
            let dx = (o.x - p.x).max(0.0).max(p.x - (o.x + CELL_SIZE));
            let dy = (o.y - p.y).max(0.0).max(p.y - (o.y + CELL_SIZE));
            (dx * dx + dy * dy).sqrt() <= load_reach + 200.0
        })
        .count()
        + usize::from(state.groups.is_none());
    let Some(groups) = state.groups.clone() else { return };

    // models prepared by the workers: insert them (a few per frame)
    {
        let rx = state.model_rx.lock().unwrap();
        let arrived: Vec<ModelResult> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        drop(rx);
        state.model_queue.extend(arrived);
    }
    let finish_start = Instant::now();
    while let Some((id, result)) = state.model_queue.pop_front() {
        let key_path = state.models.keys[id as usize].path.clone();
        state.models.states[id as usize] = match result {
            Ok((prepared, rigid, ao)) => {
                let loaded = cry.finish_model(prepared);
                if state.checked_models.insert(Arc::as_ptr(&loaded) as usize) {
                    // the renderer prepares every material asset: a cube map in a 2D slot is a wgpu validation error
                    // that takes the whole app down, so such textures are dropped (aa-cry binds `Map="Environment"`
                    // cube maps as the base colour of some materials)
                    let stripped = strip_cube_textures(&mut cry, &loaded);
                    if stripped > 0 {
                        debug!("aa-world: {key_path}: dropped {stripped} cube map texture(s) bound to 2D slots");
                    }
                }
                let nodes = loaded.parts.iter().map(|(ni, _)| Mat4::from(node_world(&loaded.model, *ni))).collect();
                let ao_meshes = vegetation_color::ao::finish(ao, &mut cry.meshes, &mut state.ao_mirrors);
                // Once per prepared model, include only authored matching group variants.
                let key = &state.models.keys[id as usize];
                let styles = groups.styles(&key.path, key.material.as_deref());
                let epoch=vegetation_color::epoch(&time);
                // Native SetupBending uses the group's LOD0 stat object's height, even when rendering another LOD.
                // Separate LOD assets have no retained original root bounds here, so leave that path unsupported.
                let wind=(key.lod == 0).then_some(groups.wind);
                let color_materials = vegetation_color::prepare(&mut cry, &loaded, &ao_meshes, &styles, wind, epoch, &mut state.color_mirrors, &mut vegetation_materials);
                ModelState::Ready(Arc::new(Instanced { lod: key.lod, loaded, nodes, rigid, color_materials, ao_meshes }))
            }
            Err(e) => {
                debug!("aa-world: model {key_path}: {e}");
                ModelState::Failed
            }
        };
        if finish_start.elapsed().as_secs_f32() * 1000.0 > cfg.finish_budget_ms {
            break;
        }
    }
    let loader = cry.loader();

    // wanted sectors
    let mut wanted: Vec<(f32, (i32, i32), usize, Kind)> = Vec::new();
    let mut keep: HashSet<(i32, i32, u8, Kind)> = HashSet::new();
    for (&c, data) in &state.cells {
        let o = cell_origin(c.0, c.1);
        let dx = (o.x - p.x).max(0.0).max(p.x - (o.x + CELL_SIZE));
        let dy = (o.y - p.y).max(0.0).max(p.y - (o.y + CELL_SIZE));
        if (dx * dx + dy * dy).sqrt() > reach + 100.0 {
            continue;
        }
        for sector in 0..256usize {
            let d = sector_distance(p, c, sector);
            for kind in [Kind::Brush, Kind::FarBrush, Kind::Entity, Kind::Animated, Kind::Vegetation, Kind::Trees] {
                if bin_of(data, kind, sector).is_empty() || (matches!(kind, Kind::Vegetation | Kind::Trees) && !cfg.vegetation) {
                    continue;
                }
                // the farthest instance of the bin decides, so a far sector of small props is not spawned
                let cap = data.views[sector][kind as usize];
                let key = (c.0, c.1, sector as u8, kind);
                let limit = if state.sectors.contains_key(&key) { cap * 1.08 } else { cap };
                if d <= limit {
                    keep.insert(key);
                    wanted.push((d, c, sector, kind));
                }
            }
        }
    }
    // despawn what is out of range
    let gone: Vec<_> = state.sectors.keys().filter(|k| !keep.contains(k)).copied().collect();
    for k in gone {
        if let Some(s) = state.sectors.remove(&k) {
            commands.entity(s.entity).despawn();
            for e in s.retired {
                commands.entity(e).despawn();
            }
        }
    }
    wanted.sort_by(|a, b| a.0.total_cmp(&b.0));
    state.stats.brush_lod_checks = 0;
    // Check only completed sectors, round-robin, at most512 instances per frame.
    // Snapshot only at a scan boundary; never restart a hidden replacement while
    // its shared-worker model requests are still outstanding.
    let mut brush_relod = Vec::new();
    for _ in 0..wanted.len() {
        if state.stats.brush_lod_checks >= lod::MAX_CHECKS { break; }
        let i = state.lod_sector_cursor % wanted.len();
        state.lod_sector_cursor = (i + 1) % wanted.len();
        let (distance, cell, sector, kind) = wanted[i];
        if !matches!(kind, Kind::Brush | Kind::FarBrush) { continue; }
        let key = (cell.0, cell.1, sector as u8, kind);
        let Some(s) = state.sectors.get_mut(&key) else { continue };
        let data = &state.cells[&cell];
        let bin = bin_of(data, kind, sector);
        if s.hidden || !s.complete(bin.len()) { continue; }
        s.lod = object_lod(kind, distance, Some(s.lod));
        let levels = s.brush_lods.as_mut().unwrap();
        let (checked, changed) = levels.scan(lod::MAX_CHECKS - state.stats.brush_lod_checks, |i| {
            view.and_then(|v| v.requested(&data.objects.brushes[bin[i] as usize], cell)).unwrap_or(s.lod)
        });
        state.stats.brush_lod_checks += checked;
        if changed { levels.snapshot(); brush_relod.push((key, s.lod)); }
    }
    state.stats.brush_lod_rebuilds += brush_relod.len();
    // A new level of detail: build next to the old sector, swap when complete.
    let relod: Vec<_> = state
        .sectors
        .iter()
        .filter_map(|(k, s)| {
            if matches!(k.3, Kind::Brush | Kind::FarBrush) { return None; }
            let d = sector_distance(p, (k.0, k.1), k.2 as usize);
            let lod = object_lod(k.3, d, Some(s.lod));
            (lod != s.lod).then_some((*k, lod))
        })
        .collect();
    for (k, lod) in relod.into_iter().chain(brush_relod) {
        let Some(s) = state.sectors.get_mut(&k) else { continue };
        let root = commands
            .spawn((ObjectSector { cell: (k.0, k.1), sector: k.2, kind: k.3 }, Transform::from_translation(s.translation), Visibility::Hidden))
            .id();
        s.retired.push(std::mem::replace(&mut s.entity, root));
        s.hidden = true;
        s.lod = lod;
        s.next = 0;
        s.spawned = 0;
        s.waiting.clear();
    }

    // spawn, nearest first, within the time budget
    let start = Instant::now();
    let budget = cfg.object_budget_ms / 1000.0;
    let workers = Workers::global();
    let mut queued = workers.pending();
    for (d, c, sector, kind) in wanted {
        if start.elapsed().as_secs_f32() > budget {
            break;
        }
        let key = (c.0, c.1, sector as u8, kind);
        let data = state.cells[&c].clone();
        let bin = bin_of(&data, kind, sector);
        if state.sectors.get(&key).is_some_and(|s| s.complete(bin.len()) && !s.hidden) {
            continue;
        }
        let sector_origin = Vec3::new((sector / 16) as f32 * SECTOR_SIZE, (sector % 16) as f32 * SECTOR_SIZE, 0.0);
        let state = &mut *state;
        let entry = state.sectors.entry(key).or_insert_with(|| {
            let translation = cry_to_bevy(cell_origin(c.0, c.1) + sector_origin);
            let root = commands
                .spawn((ObjectSector { cell: c, sector: sector as u8, kind }, Transform::from_translation(translation), Visibility::default()))
                .id();
            SectorState { entity: root, translation, lod: object_lod(kind, d, None),
                brush_lods: matches!(kind, Kind::Brush | Kind::FarBrush).then(lod::BrushLods::default),
                next: 0, spawned: 0, waiting: Vec::new(), hidden: false, retired: Vec::new() }
        });
        let models = &mut state.models;
        let model_tx = &state.model_tx;

        // asks for a model if it is not on its way yet
        let mut request = |models: &mut Models, id: u32| {
            if matches!(models.states[id as usize], ModelState::Wanted) && queued < MAX_QUEUED_MODELS {
                models.states[id as usize] = ModelState::Loading;
                queued += 1;
                let tx = model_tx.clone();
                let loader = loader.clone();
                let key = models.keys[id as usize].clone();
                workers.spawn(d, move || {
                    let options = ModelOptions { lod: key.lod as u32, material: key.material.clone(), cry_space: true, ..default() };
                    // a panic must not leave the model "loading" for ever
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let prepared = loader.prepare_model(&key.path, &options)?;
                        let rigid = if key.path.to_ascii_lowercase().ends_with(".cga") && key.lod == 0 {
                            match rigid::Clip::load(&key.path, &prepared.loaded().model) {
                                Ok(clip) => Some(Arc::new(clip)),
                                Err(e) => { debug!("aa-world: {}: {e}", key.path); None }
                            }
                        } else { None };
                        let ao = vegetation_color::ao::prepare(&loader, prepared.loaded());
                        Ok((prepared, rigid, ao))
                    }))
                        .unwrap_or_else(|_| Err(aa_cry::Error::Invalid(format!("panic while preparing {}", key.path))));
                    let _ = tx.send((id, result));
                });
            }
        };

        // instances that waited for their model
        if !entry.waiting.is_empty() {
            let waiting = std::mem::take(&mut entry.waiting);
            let mut rest = waiting.into_iter();
            for (idx, id) in rest.by_ref() {
                match &models.states[id as usize] {
                    ModelState::Ready(inst) => {
                        let inst = inst.clone();
                        spawn_instance(&mut commands, entry, c, &data, kind, idx as usize, &inst, sector_origin, &groups);
                    }
                    ModelState::Failed => {}
                    _ => {
                        request(models, id);
                        entry.waiting.push((idx, id));
                    }
                }
                if start.elapsed().as_secs_f32() > budget {
                    break;
                }
            }
            entry.waiting.extend(rest);
        }
        while entry.next < bin.len() && start.elapsed().as_secs_f32() <= budget {
            let bin_index = entry.next;
            let idx = bin[entry.next];
            entry.next += 1;
            let lod = if let Some(levels) = &mut entry.brush_lods {
                let requested = view.and_then(|v| v.requested(&data.objects.brushes[idx as usize], c)).unwrap_or(entry.lod);
                levels.initial(bin_index, requested)
            } else { entry.lod };
            let Some((a, b)) = instance_model(&data, kind, idx as usize, &groups) else { continue };
            let fast = (c.0, c.1, kind, a, b, lod);
            let id = match models.fast.get(&fast) {
                Some(&id) => id,
                None => {
                    let Some((path, material)) = model_names(&data, kind, a, b, &groups) else { continue };
                    let id = models.id(ModelKey { path, material, lod });
                    models.fast.insert(fast, id);
                    id
                }
            };
            match &models.states[id as usize] {
                ModelState::Ready(inst) => {
                    let inst = inst.clone();
                    spawn_instance(&mut commands, entry, c, &data, kind, idx as usize, &inst, sector_origin, &groups);
                }
                ModelState::Failed => {}
                _ => {
                    request(models, id);
                    entry.waiting.push((idx, id));
                }
            }
        }
        if entry.hidden && entry.complete(bin.len()) {
            entry.hidden = false;
            commands.entity(entry.entity).insert(Visibility::Inherited);
            for e in entry.retired.drain(..) {
                commands.entity(e).despawn();
            }
        }
    }
    let waiting: usize = state.sectors.values().map(|s| s.waiting.len()).sum();
    // sectors in range that are not drawn completely yet (not started, spawning, waiting for models, a new LOD being built)
    state.stats.sectors_incomplete = keep
        .iter()
        .filter(|k| {
            let len = state.cells.get(&(k.0, k.1)).map_or(0, |data| bin_of(data, k.3, k.2 as usize).len());
            !state.sectors.get(*k).is_some_and(|s| s.complete(len) && !s.hidden)
        })
        .count();
    state.stats.instances_waiting = waiting;
    // models on their way; a `Wanted` one that no sector waits for any more (its sector went out of range) is not pending, and
    // the instances that do wait for one are `waiting`
    let pending = state.models.states.iter().filter(|m| matches!(m, ModelState::Loading)).count() + state.model_queue.len();
    state.stats.sectors = state.sectors.len();
    state.stats.models_pending = pending;
    state.stats.models_cached = state.models.keys.len();
    state.stats.brush_lod_instances = state.sectors.values().filter_map(|s| s.brush_lods.as_ref()).map(|s| s.current.len()).sum();
    state.stats.models_failed = state.models.states.iter().filter(|m| matches!(m, ModelState::Failed)).count();
    let spawned: usize = state.sectors.values().map(|s| s.spawned).sum();
    state.stats.entity_props = state.sectors.iter().filter(|(k, _)| k.3 == Kind::Entity).map(|(_, s)| s.spawned).sum();
    state.stats.animated_props = state.sectors.iter().filter(|(k, _)| k.3 == Kind::Animated).map(|(_, s)| s.spawned).sum();
    let busy = pending > 0 || waiting > 0;
    if busy {
        state.busy_ticks += 1;
    }
    state.stats.entities = spawned + if busy { state.busy_ticks } else { 0 };
    state.stats.spawn_ms = start.elapsed().as_secs_f32() * 1000.0;
}

/// The model of an instance as two numbers (brush: model and material index; vegetation: group), for the fast lookup.
fn instance_model(data: &CellObjectData, kind: Kind, idx: usize, groups: &VegGroups) -> Option<(u32, i32)> {
    match kind {
        Kind::Brush | Kind::FarBrush => {
            let b = data.objects.brushes.get(idx)?;
            ((b.model as usize) < data.objects.models.len()).then_some((b.model, b.material))
        }
        Kind::Entity => data.objects.entity_props.get(idx).map(|_| (idx as u32, -1)),
        Kind::Animated => data.objects.animated_props.get(idx).map(|_| (idx as u32, -1)),
        Kind::Vegetation | Kind::Trees => {
            let v = data.objects.vegetation.get(idx)?;
            groups.groups.contains_key(&v.group).then_some((u32::from(v.group), -1))
        }
    }
}

/// File and material file for [`instance_model`]'s numbers.
fn model_names(data: &CellObjectData, kind: Kind, a: u32, b: i32, groups: &VegGroups) -> Option<(String, Option<String>)> {
    match kind {
        Kind::Brush | Kind::FarBrush => {
            let path = data.objects.models.get(a as usize)?.clone();
            let material = usize::try_from(b)
                .ok()
                .and_then(|i| data.objects.materials.get(i))
                .filter(|m| !m.is_empty())
                .map(|m| format!("{m}.mtl"));
            Some((path, material))
        }
        Kind::Entity => {
            let prop = data.objects.entity_props.get(a as usize)?;
            Some((prop.model.clone(), prop.material.clone()))
        }
        Kind::Animated => {
            let prop = data.objects.animated_props.get(a as usize)?;
            Some((prop.model.clone(), prop.material.clone()))
        }
        Kind::Vegetation | Kind::Trees => {
            let g = groups.groups.get(&(a as u16))?;
            Some((g.model.clone(), (!g.material.is_empty()).then(|| format!("{}.mtl", g.material))))
        }
    }
}

/// Spawns the parts of one instance under the sector's root.
#[allow(clippy::too_many_arguments)]
fn spawn_instance(
    commands: &mut Commands,
    entry: &mut SectorState,
    cell: (i32, i32),
    data: &CellObjectData,
    kind: Kind,
    idx: usize,
    inst: &Instanced,
    sector_origin: Vec3,
    groups: &VegGroups,
) {
    let limits = &data.limits;
    let (matrix, view) = match kind {
        Kind::Brush | Kind::FarBrush => {
            let b = &data.objects.brushes[idx];
            (b.mat4(), limits.brush(b).0)
        }
        Kind::Entity => (data.objects.entity_props[idx].matrix, limits.near),
        Kind::Animated => (data.objects.animated_props[idx].matrix, limits.near),
        Kind::Vegetation | Kind::Trees => {
            let v = &data.objects.vegetation[idx];
            (v.mat4(), limits.vegetation(v).0)
        }
    };
    let pre = Mat4::from_quat(CRY_TO_BEVY) * Mat4::from_translation(-sector_origin) * matrix;
    if kind == Kind::Animated {
        if let Some(clip) = &inst.rigid {
            entry.spawned += rigid::spawn(commands, entry.entity, cell, &data.objects.animated_props[idx],
                &inst.loaded, clip.clone(), pre, view);
        }
        return;
    }
    for ((_, parts), node) in inst.loaded.parts.iter().zip(&inst.nodes) {
        let full = pre * *node;
        if !full.is_finite() || full.determinant().abs() < 1e-9 {
            continue;
        }
        let transform = Transform::from_matrix(full);
        for part in parts {
            let mut entity = commands.spawn((
                Mesh3d(part.mesh.clone()),
                transform,
                part.aabb,
                VisibilityRange::abrupt(0.0, view),
                ChildOf(entry.entity),
            ));
            // Native ordinary shadow collector301EE5E0 requires node flags&0x8.
            // Extra-range/terrain-only masks are distinct native shadow modes.
            let no_ordinary_shadow = match kind {
                Kind::Brush | Kind::FarBrush => data.objects.brushes[idx].flags & 0x8 == 0,
                Kind::Vegetation | Kind::Trees => data.objects.vegetation[idx].flags & 0x8 == 0,
                _ => false,
            };
            if no_ordinary_shadow { entity.insert(NotShadowCaster); }
            if matches!(kind, Kind::Brush | Kind::FarBrush) {
                entity.insert(lod::BrushLodPart { cell, index: idx as u32, requested: inst.lod });
            }
            if matches!(kind, Kind::Vegetation | Kind::Trees)
                && let Some(group) = groups.groups.get(&data.objects.vegetation[idx].group)
                && group.material_layers == 0
                && let Some(material) = inst.color_materials.get(&(part.material.id(), group.style().brightness.to_bits(), group.style().bending.to_bits()))
            {
                let bgr = data.objects.vegetation[idx].color_variation;
                let tag = if material.variation { vegetation_color::pack(bgr) } else { vegetation_color::NO_COLOR_VARIATION };
                entity.insert((MeshMaterial3d(material.material.clone()), MeshTag::with_type::<vegetation_color::VegetationColor>(tag)));
                if material.variation { entity.insert(VegetationColorPart { cell, bgr }); }
                if material.brightness != 1.0 {
                    entity.insert(vegetation_color::VegetationBrightnessPart { cell, group: group.id, brightness: material.brightness });
                }
                if material.wind.z > 0.0 {
                    entity.insert(crate::vegetation_wind::VegetationWindPart { cell, group: group.id,
                        bending: group.style().bending, half_height: material.wind.w });
                    // Native main bending preserves every local vertex's distance from the origin.
                    // Its enclosing sphere stays valid when a narrow static box would clip moving tips.
                    let radius=(part.aabb.center.abs()+part.aabb.half_extents).length();
                    entity.insert(bevy::camera::primitives::Aabb { center:Vec3A::ZERO, half_extents:Vec3A::splat(radius) });
                }
                if let Some(ao) = inst.ao_meshes.get(&part.mesh.id()) {
                    entity.insert((Mesh3d(ao.mesh.clone()), vegetation_color::VegetationAoPart { cell, min_alpha: ao.min_alpha }));
                }
            } else {
                entity.insert(MeshMaterial3d(part.material.clone()));
            }
            if kind == Kind::Entity {
                let prop = &data.objects.entity_props[idx];
                entity.insert((Name::new(prop.name.clone()), StaticEntityPart { cell, guid: prop.guid }));
                if !prop.cast_shadow { entity.insert(NotShadowCaster); }
            }
            entry.spawned += 1;
        }
    }
}

/// Removes cube map images from the 2D texture slots of the model's materials; returns how many.
fn strip_cube_textures(cry: &mut CryAssets, loaded: &LoadedModel) -> usize {
    let is_cube = |images: &Assets<Image>, h: &Option<Handle<Image>>| {
        h.as_ref()
            .and_then(|h| images.get(h))
            .and_then(|i| i.texture_view_descriptor.as_ref())
            .is_some_and(|d| d.dimension == Some(TextureViewDimension::Cube))
    };
    let mut bad = Vec::new();
    for part in loaded.parts.iter().flat_map(|(_, parts)| parts) {
        if let Some(m) = cry.materials.get(&part.material) {
            let slots = [&m.base_color_texture, &m.emissive_texture, &m.metallic_roughness_texture, &m.normal_map_texture, &m.occlusion_texture];
            if slots.into_iter().any(|h| is_cube(&cry.images, h)) {
                bad.push(part.material.clone());
            }
        }
    }
    let mut count = 0;
    for handle in bad {
        let Some(mut guard) = cry.materials.get_mut(&handle) else { continue };
        let m: &mut StandardMaterial = &mut guard;
        for slot in [&mut m.base_color_texture, &mut m.emissive_texture, &mut m.metallic_roughness_texture, &mut m.normal_map_texture, &mut m.occlusion_texture] {
            if is_cube(&cry.images, slot) {
                *slot = None;
                count += 1;
            }
        }
    }
    count
}

fn bin_of(data: &CellObjectData, kind: Kind, sector: usize) -> &[u32] {
    match kind {
        Kind::Brush => &data.brush_bins[sector],
        Kind::FarBrush => &data.far_brush_bins[sector],
        Kind::Entity => &data.entity_bins[sector],
        Kind::Animated => &data.animated_bins[sector],
        Kind::Vegetation => &data.veg_bins[sector],
        Kind::Trees => &data.tree_bins[sector],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::placement::Brush;

    fn brush(max_view_dist: f32) -> Brush {
        Brush { matrix: [1.0, 0.0, 0.0, 10.0, 0.0, 1.0, 0.0, 10.0, 0.0, 0.0, 1.0, 0.0], model: 0, material: -1,
            flags: 0, max_view_dist, lod_ratio: 100, bbox: [0.0; 6] }
    }

    #[test]
    fn view_limits_scale_compiled_distances_and_split_far_brushes() {
        let limits = ViewLimits::default();
        assert_eq!(limits.brush(&brush(600.0)), (900.0, Kind::Brush));
        assert_eq!(limits.brush(&brush(800.0)), (1200.0, Kind::FarBrush));
        // e_max_view_dst caps the castles
        assert_eq!(limits.brush(&brush(9112.0)), (2000.0, Kind::FarBrush));
        assert_eq!(limits.brush(&brush(0.0)), (NEAR_BRUSH_DISTANCE, Kind::Brush));
        // a small configured distance keeps everything capped (the close-up examples)
        let mut cfg = WorldConfig::new("main_world");
        cfg.brush_distance = 150.0;
        let small = ViewLimits::new(&cfg);
        assert_eq!(small.brush(&brush(800.0)), (150.0, Kind::FarBrush));
        assert_eq!(small.brush(&brush(60.0)), (90.0, Kind::Brush));
    }

    #[test]
    fn palace_castle_brushes_are_far_and_sector_views_cover_their_instances() {
        if !crate::pak_available() { return; }
        let objects = crate::placement::load_cell_objects("main_world", 13, 10);
        let total = objects.brushes.len();
        let data = CellObjectData::new(objects, &ViewLimits::default());
        let far: usize = data.far_brush_bins.iter().map(Vec::len).sum();
        let near: usize = data.brush_bins.iter().map(Vec::len).sum();
        assert_eq!(far + near, total);
        assert!(far > 50 && far < total / 4, "far {far} of {total}");
        let castle = data.far_brush_bins.iter().flatten()
            .any(|&i| data.objects.models[data.objects.brushes[i as usize].model as usize].ends_with("castle_nuia_001.cgf"));
        assert!(castle);
        for (sector, bin) in data.far_brush_bins.iter().enumerate() {
            for &i in bin {
                let (view, kind) = data.limits.brush(&data.objects.brushes[i as usize]);
                assert_eq!(kind, Kind::FarBrush);
                assert!(view > NEAR_BRUSH_DISTANCE && view <= data.views[sector][Kind::FarBrush as usize]);
            }
        }
    }
}
