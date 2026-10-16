//! Cell objects: brushes (`brush.dat`) and vegetation as meshes, spawned per 64 m sector near the camera.
//!
//! * placement files are parsed on background tasks and binned per sector;
//! * the main thread spawns sectors nearest first under a time budget per frame
//!   ([`WorldConfig::object_budget_ms`]); model loading (aa-cry) is the expensive part, so a sector
//!   fills in over a few frames instead of hitching;
//! * a sector is alive while the camera is within the largest view distance of its instances (capped by
//!   [`WorldConfig::brush_distance`] / [`WorldConfig::vegetation_distance`]); every part also carries a
//!   [`VisibilityRange`] for its own distance, so the renderer drops far instances without our help;
//! * level of detail: a sector is spawned with the `_lodN` / `$lodN` level for its distance band and respawned
//!   when the band changes;
//! * water volumes (rivers, lakes, ponds) of a cell are merged into one flat mesh per cell, built on the task;
//! * instancing: all parts of one model share mesh and material handles, which Bevy batches into instanced
//!   draws. Parts are flattened (the node transform is baked) so an instance costs one entity per drawn part.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use aa_cry::{CryAssets, ModelOptions};
use aa_cry::scene::LoadedModel;
use bevy::camera::visibility::VisibilityRange;
use bevy::light::NotShadowCaster;
use bevy::math::Affine3A;
use bevy::prelude::*;
use bevy::tasks::AsyncComputeTaskPool;

use crate::coords::{CRY_TO_BEVY, CELL_SIZE, SECTOR_SIZE, bevy_to_cry, cell_origin, cry_to_bevy};
use crate::placement::{CellObjects, load_cell_objects};
use crate::stream::{Streamer, WorldConfig, WorldFocus};
use crate::vegetation::VegGroups;
use crate::water::water_mesh;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Brush,
    Vegetation,
}

/// Placement data of one cell, binned by 64 m sector.
pub struct CellObjectData {
    pub objects: CellObjects,
    /// Instance indices per sector (`sx * 16 + sy`).
    pub brush_bins: Vec<Vec<u32>>,
    pub veg_bins: Vec<Vec<u32>>,
}

impl CellObjectData {
    pub fn new(objects: CellObjects) -> Self {
        let mut brush_bins = vec![Vec::new(); 256];
        let mut veg_bins = vec![Vec::new(); 256];
        let bin = |x: f32, y: f32| {
            let sx = ((x / SECTOR_SIZE) as usize).min(15);
            let sy = ((y / SECTOR_SIZE) as usize).min(15);
            sx * 16 + sy
        };
        for (i, b) in objects.brushes.iter().enumerate() {
            let t = b.translation();
            brush_bins[bin(t.x, t.y)].push(i as u32);
        }
        for (i, v) in objects.vegetation.iter().enumerate() {
            veg_bins[bin(v.pos.x, v.pos.y)].push(i as u32);
        }
        CellObjectData { objects, brush_bins, veg_bins }
    }
}

/// A sector's entity (children are the drawn parts).
#[derive(Component)]
pub struct ObjectSector {
    pub cell: (i32, i32),
    pub sector: u8,
    pub kind: Kind,
}

struct SectorState {
    entity: Entity,
    lod: u8,
    /// Next index into the sector's bin (spawning resumes here).
    next: usize,
    spawned: usize,
}

#[derive(Resource)]
pub struct ObjectState {
    cells: HashMap<(i32, i32), Arc<CellObjectData>>,
    loading: HashSet<(i32, i32)>,
    sectors: HashMap<(i32, i32, u8, Kind), SectorState>,
    failed_models: HashSet<String>,
    groups: Option<Arc<VegGroups>>,
    groups_rx: Mutex<Receiver<Option<VegGroups>>>,
    groups_requested: bool,
    tx: Sender<CellMessage>,
    rx: Mutex<Receiver<CellMessage>>,
    /// One water mesh entity per cell.
    water: HashMap<(i32, i32), Entity>,
    water_material: Option<Handle<StandardMaterial>>,
    groups_tx: Sender<Option<VegGroups>>,
    /// Counters for diagnostics.
    pub stats: ObjectStats,
}

/// What a placement task delivers: the binned data and the water mesh of the cell.
type CellMessage = ((i32, i32), CellObjectData, Option<Mesh>);

impl ObjectState {
    /// The material shared by all water volumes (tinted by the environment each frame).
    pub fn water_material(&self) -> Option<&Handle<StandardMaterial>> {
        self.water_material.as_ref()
    }
}

#[derive(Default, Clone, Debug)]
pub struct ObjectStats {
    pub sectors: usize,
    pub entities: usize,
    pub models_failed: usize,
    pub spawn_ms: f32,
    pub water_cells: usize,
}

impl ObjectState {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        let (groups_tx, groups_rx) = channel();
        ObjectState {
            cells: HashMap::new(),
            loading: HashSet::new(),
            sectors: HashMap::new(),
            failed_models: HashSet::new(),
            groups: None,
            groups_rx: Mutex::new(groups_rx),
            groups_requested: false,
            tx,
            rx: Mutex::new(rx),
            water: HashMap::new(),
            water_material: None,
            groups_tx,
            stats: ObjectStats::default(),
        }
    }
}

impl Default for ObjectState {
    fn default() -> Self {
        Self::new()
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
    let bands: [f32; 2] = match kind {
        Kind::Brush => [140.0, 380.0],
        Kind::Vegetation => [55.0, 130.0],
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
    focus: Query<&GlobalTransform, With<WorldFocus>>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
) {
    if !cfg.objects {
        return;
    }
    let Some(cam) = focus.iter().next().or_else(|| cameras.iter().next()) else { return };
    let p = bevy_to_cry(cam.translation());
    let state = &mut *state;
    let pool = AsyncComputeTaskPool::get();

    // vegetation groups (world level)
    if !state.groups_requested {
        state.groups_requested = true;
        let world = cfg.world.clone();
        let tx = state.groups_tx.clone();
        pool.spawn(async move {
            let _ = tx.send(VegGroups::load(&world));
        })
        .detach();
    }
    if state.groups.is_none() {
        if let Ok(g) = state.groups_rx.lock().unwrap().try_recv() {
            state.groups = Some(Arc::new(g.unwrap_or_default()));
        }
    }

    // placement files of loaded terrain cells
    let reach = cfg.brush_distance.max(cfg.vegetation_distance);
    let load_reach = reach.max(cfg.water_distance);
    let loaded: Vec<(i32, i32)> = streamer.loaded_cells().collect();
    state.cells.retain(|c, _| loaded.contains(c));
    let gone: Vec<_> = state.water.keys().filter(|c| !loaded.contains(c)).copied().collect();
    for c in gone {
        if let Some(e) = state.water.remove(&c) {
            commands.entity(e).despawn();
        }
    }
    if state.water_material.is_none() {
        state.water_material = Some(cry.materials.add(StandardMaterial {
            base_color: Color::srgba(0.04, 0.17, 0.2, 0.8),
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 0.45,
            reflectance: 0.12,
            cull_mode: None,
            ..default()
        }));
    }
    let dead: Vec<_> = state.sectors.keys().filter(|k| !loaded.contains(&(k.0, k.1))).copied().collect();
    for k in dead {
        if let Some(s) = state.sectors.remove(&k) {
            commands.entity(s.entity).despawn();
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
        pool.spawn(async move {
            let objects = load_cell_objects(&world, c.0, c.1);
            let water = water_mesh(&objects.water);
            let data = CellObjectData::new(objects);
            let _ = tx.send((c, data, water));
        })
        .detach();
    }
    let received: Vec<CellMessage> = {
        let rx = state.rx.lock().unwrap();
        std::iter::from_fn(|| rx.try_recv().ok()).collect()
    };
    for (c, data, water) in received {
        state.loading.remove(&c);
        if !loaded.contains(&c) {
            continue;
        }
        if let (Some(mesh), Some(material)) = (water, state.water_material.clone()) {
            let entity = commands
                .spawn((
                    Mesh3d(cry.meshes.add(mesh)),
                    MeshMaterial3d(material),
                    Transform::from_translation(cry_to_bevy(cell_origin(c.0, c.1))),
                    NotShadowCaster,
                ))
                .id();
            state.water.insert(c, entity);
        }
        state.cells.insert(c, Arc::new(data));
    }
    state.stats.water_cells = state.water.len();
    let Some(groups) = state.groups.clone() else { return };

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
            for (kind, bin, cap) in [
                (Kind::Brush, &data.brush_bins[sector], cfg.brush_distance),
                (Kind::Vegetation, &data.veg_bins[sector], cfg.vegetation_distance),
            ] {
                if bin.is_empty() || (kind == Kind::Vegetation && !cfg.vegetation) {
                    continue;
                }
                let key = (c.0, c.1, sector as u8, kind);
                let limit = if state.sectors.contains_key(&key) { cap * 1.08 } else { cap };
                if d <= limit {
                    keep.insert(key);
                    wanted.push((d, c, sector, kind));
                }
            }
        }
    }
    // despawn what is out of range, or whose level of detail changed
    let stale: Vec<_> = state
        .sectors
        .iter()
        .filter(|(k, s)| {
            if !keep.contains(k) {
                return true;
            }
            let d = sector_distance(p, (k.0, k.1), k.2 as usize);
            object_lod(k.3, d, Some(s.lod)) != s.lod
        })
        .map(|(k, _)| *k)
        .collect();
    for k in stale {
        if let Some(s) = state.sectors.remove(&k) {
            commands.entity(s.entity).despawn();
        }
    }

    // spawn, nearest first, within the time budget
    wanted.sort_by(|a, b| a.0.total_cmp(&b.0));
    let start = Instant::now();
    let budget = cfg.object_budget_ms / 1000.0;
    for (d, c, sector, kind) in wanted {
        if start.elapsed().as_secs_f32() > budget {
            break;
        }
        let key = (c.0, c.1, sector as u8, kind);
        let data = state.cells[&c].clone();
        if state.sectors.get(&key).is_some_and(|s| s.next >= bin_of(&data, kind, sector).len()) {
            continue;
        }
        let lod = state.sectors.get(&key).map_or_else(|| object_lod(kind, d, None), |s| s.lod);
        let sector_origin = Vec3::new((sector / 16) as f32 * SECTOR_SIZE, (sector % 16) as f32 * SECTOR_SIZE, 0.0);
        let entry = state.sectors.entry(key).or_insert_with(|| {
            let root = commands
                .spawn((
                    ObjectSector { cell: c, sector: sector as u8, kind },
                    Transform::from_translation(cry_to_bevy(cell_origin(c.0, c.1) + sector_origin)),
                    Visibility::default(),
                ))
                .id();
            SectorState { entity: root, lod, next: 0, spawned: 0 }
        });
        let bin = bin_of(&data, kind, sector);
        while entry.next < bin.len() && start.elapsed().as_secs_f32() <= budget {
            let idx = bin[entry.next] as usize;
            entry.next += 1;
            let (path, material, matrix, view): (String, Option<String>, Mat4, f32) = match kind {
                Kind::Brush => {
                    let b = &data.objects.brushes[idx];
                    let Some(model) = data.objects.models.get(b.model as usize) else { continue };
                    let material = usize::try_from(b.material)
                        .ok()
                        .and_then(|i| data.objects.materials.get(i))
                        .filter(|m| !m.is_empty())
                        .map(|m| format!("{m}.mtl"));
                    let view = if b.max_view_dist > 1.0 { b.max_view_dist } else { cfg.brush_distance };
                    (model.clone(), material, b.mat4(), view.min(cfg.brush_distance))
                }
                Kind::Vegetation => {
                    let v = &data.objects.vegetation[idx];
                    let Some(g) = groups.groups.get(&v.group) else { continue };
                    let material = (!g.material.is_empty()).then(|| format!("{}.mtl", g.material));
                    let view = if v.max_view_dist > 1.0 { v.max_view_dist } else { cfg.vegetation_distance };
                    (g.model.clone(), material, v.mat4(), view.min(cfg.vegetation_distance))
                }
            };
            if state.failed_models.contains(&path) {
                continue;
            }
            let options = ModelOptions { lod: entry.lod as u32, material, cry_space: true, ..default() };
            let loaded: Arc<LoadedModel> = match cry.load_model(&path, &options) {
                Ok(m) => m,
                Err(e) => {
                    debug!("aa-world: model {path}: {e}");
                    state.failed_models.insert(path);
                    state.stats.models_failed = state.failed_models.len();
                    continue;
                }
            };
            let rel = Mat4::from_translation(-sector_origin) * matrix;
            for (ni, parts) in &loaded.parts {
                let full = Mat4::from_quat(CRY_TO_BEVY) * rel * Mat4::from(node_world(&loaded.model, *ni));
                if !full.is_finite() || full.determinant().abs() < 1e-9 {
                    continue;
                }
                let transform = Transform::from_matrix(full);
                for part in parts {
                    commands.spawn((
                        Mesh3d(part.mesh.clone()),
                        MeshMaterial3d(part.material.clone()),
                        transform,
                        part.aabb,
                        VisibilityRange::abrupt(0.0, view),
                        ChildOf(entry.entity),
                    ));
                    entry.spawned += 1;
                }
            }
        }
    }
    state.stats.sectors = state.sectors.len();
    state.stats.entities = state.sectors.values().map(|s| s.spawned).sum();
    state.stats.spawn_ms = start.elapsed().as_secs_f32() * 1000.0;
}

fn bin_of(data: &CellObjectData, kind: Kind, sector: usize) -> &[u32] {
    match kind {
        Kind::Brush => &data.brush_bins[sector],
        Kind::Vegetation => &data.veg_bins[sector],
    }
}
