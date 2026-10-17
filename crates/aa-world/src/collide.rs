//! Static collision with the world's objects: buildings, rocks, bridges and walls next to the terrain.
//!
//! [`WorldCollision`] answers game-space (metres, x east, y north, z up, like [`TerrainQuery`]) questions
//! about the ground and the walls around a position, for the camera and the player controller of the
//! application:
//!
//! ```no_run
//! # use aa_world::{TerrainQuery, collide::WorldCollision};
//! # use bevy::math::Vec3;
//! let world = WorldCollision::new("main_world", TerrainQuery::new("main_world"));   // or Res<WorldCollision>
//! world.request_around(21_130.0, 10_594.0, 150.0);                                // build the cells near the player
//! // the camera: from the pivot above the player's head towards the camera position
//! let pivot = Vec3::new(21_130.0, 10_594.0, 20.0);
//! let wanted = Vec3::new(21_135.0, 10_599.0, 23.0);
//! let camera = world.clip_camera(pivot, wanted, 0.3);                             // Vec3, short of the wall if one is in the way
//! // the player: stand on bridges and stairs, slide along walls
//! let ground = world.ground_below(21_130.0, 10_594.0, 20.5, 0.5);                  // Option<Ground { height, normal }>
//! let moved = world.slide(pivot, bevy::math::Vec2::new(0.1, 0.0), 0.35, 1.8, 0.5);  // Pushed { pos, blocked, normal }
//! ```
//!
//! # What collides
//!
//! * The terrain, as the finest mesh the renderer draws ([`TerrainQuery`]).
//! * Every brush (`brush.dat`; 162k in the main world) whose model has physics geometry (`aa_cry::physics`: the
//!   CryPhysics proxies of the `.cgf`, 93 % of the placed brushes; the rest are decoration such as ivy, flags and
//!   laundry, which the real client walks through too). Vegetation (trees) is not included.
//!
//! Brushes are split by size: [`Mask::STRUCTURES`] (houses, walls, rocks, bridges, ...) and [`Mask::PROPS`]
//! (everything whose bounding box is below [`CollisionConfig::prop_volume`]: barrels, fences, lamp posts). The
//! camera should use [`Mask::CAMERA`] (terrain and structures) so that it does not jump at every barrel; the
//! player collides with everything ([`Mask::ALL`]).
//!
//! # Cells
//!
//! Brush collision is built per terrain cell (1024 m) on the `aa_cry::workers` threads: placement files read,
//! each distinct model's proxy triangles loaded once (shared by all cells), transformed into game space and indexed
//! by an 8 m grid. A cell that is not built yet only has the terrain: [`request_around`]
//! starts the build, [`ready_around`] says when it is done, and `AaWorldPlugin` follows the camera (or the
//! [`WorldFocus`](crate::WorldFocus) entity) with [`follow`](WorldCollision::follow): cells within
//! [`CollisionConfig::radius`] are built, cells beyond [`CollisionConfig::keep`] are dropped.

mod geom;
mod soup;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, RwLock};
use std::time::Instant;

use aa_cry::physics::CollisionMesh;
use bevy::prelude::*;

use crate::coords::CELL_SIZE;
use crate::heightmap::CellHeights;
use crate::placement::{parse_brush_dat, parse_object_dat, parse_statobjs};
use crate::stream::{WorldConfig, WorldFocus};
use crate::terrain_query::{Ground, TerrainQuery};
use crate::world::cell_dir;
use geom::{Tri, closest_point, cross_normal};
use soup::Soup;

/// Which surfaces a query looks at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mask(u8);

impl Mask {
    pub const TERRAIN: Mask = Mask(1);
    /// Big brushes: buildings, walls, rocks, bridges.
    pub const STRUCTURES: Mask = Mask(2);
    /// Small brushes: barrels, crates, fences, lamp posts.
    pub const PROPS: Mask = Mask(4);
    pub const ALL: Mask = Mask(7);
    /// What the camera stops at: the terrain and the structures.
    pub const CAMERA: Mask = Mask(3);

    pub const fn contains(self, other: Mask) -> bool {
        self.0 & other.0 == other.0
    }
}

impl std::ops::BitOr for Mask {
    type Output = Mask;
    fn bitor(self, rhs: Mask) -> Mask {
        Mask(self.0 | rhs.0)
    }
}

/// Settings of [`WorldCollision`].
#[derive(Clone, Debug)]
pub struct CollisionConfig {
    /// [`WorldCollision::follow`] builds the cells within this distance (m) of the focus; 0 or less: it builds
    /// nothing (the character select screens), and cells are only built by [`WorldCollision::request`].
    pub radius: f32,
    /// ... and drops the cells farther than this.
    pub keep: f32,
    /// Edge of the grid cells of a soup (m).
    pub grid: f32,
    /// Brushes whose bounding box is smaller than this (m^3) are props.
    pub prop_volume: f32,
    /// Steepest surface that counts as ground (radians from the horizontal).
    pub max_slope: f32,
    /// Radius of the feet for [`WorldCollision::ground_below`] (m).
    pub foot_radius: f32,
    /// Models without physics geometry collide as their coarsest visible mesh (the real client lets you walk
    /// through them: ivy, flags, laundry, straw).
    pub visible_fallback: bool,
}

impl Default for CollisionConfig {
    fn default() -> Self {
        CollisionConfig { radius: 150.0, keep: 400.0, grid: 8.0, prop_volume: 4.0, max_slope: 55f32.to_radians(), foot_radius: 0.4, visible_fallback: false }
    }
}

/// A ray or sweep hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// Distance along the (unit) direction: for a ray the surface point, for a sphere sweep its centre at the contact.
    pub t: f32,
    /// The surface point that was hit.
    pub point: Vec3,
    /// Unit surface normal, facing the origin of the ray.
    pub normal: Vec3,
    /// The terrain was hit (else a brush).
    pub terrain: bool,
}

/// Result of [`WorldCollision::push_out`] and [`WorldCollision::slide`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pushed {
    /// Feet position after the walls pushed the capsule back.
    pub pos: Vec3,
    /// A wall moved the capsule.
    pub blocked: bool,
    /// Horizontal unit direction of the last push (walls' normal), zero when not blocked.
    pub normal: Vec3,
}

/// What a cell's build produced.
#[derive(Clone, Copy, Debug, Default)]
pub struct CellStats {
    pub brushes: usize,
    /// Brushes whose model has collision geometry.
    pub colliding: usize,
    pub models: usize,
    pub structure_triangles: usize,
    pub prop_triangles: usize,
    pub bytes: usize,
    pub build_ms: f32,
}

/// The collision of one cell's brushes in game space.
pub struct CellCollision {
    pub cell: (i32, i32),
    structures: Soup,
    props: Soup,
    pub stats: CellStats,
}

impl CellCollision {
    pub fn is_empty(&self) -> bool {
        self.structures.is_empty() && self.props.is_empty()
    }

    fn soups(&self, mask: Mask) -> impl Iterator<Item = &Soup> {
        [(Mask::STRUCTURES, &self.structures), (Mask::PROPS, &self.props)].into_iter().filter(move |(m, _)| mask.contains(*m)).map(|(_, s)| s)
    }
}

/// A model's proxy geometry, filled once (`None`: it has no collision).
type ModelSlot = Arc<OnceLock<Option<Arc<CollisionMesh>>>>;

enum Entry {
    Loading,
    Ready(Arc<CellCollision>),
}

struct Inner {
    world: String,
    terrain: TerrainQuery,
    config: CollisionConfig,
    /// cos of the steepest ground slope.
    min_nz: f32,
    cells: RwLock<HashMap<(i32, i32), Entry>>,
    /// Proxy geometry per distinct model; filled outside the lock like `TerrainQuery`'s cells.
    models: Mutex<HashMap<String, ModelSlot>>,
    builds: AtomicU64,
    build_micros: AtomicU64,
}

/// Collision of one world's brushes and terrain. A cheap clonable handle (`Arc` inside, `&self` methods,
/// `Send + Sync`) and a [`Resource`]; `AaWorldPlugin` inserts one for its world. See the module documentation.
#[derive(Resource, Clone)]
pub struct WorldCollision {
    inner: Arc<Inner>,
}

/// Totals over the built cells, see [`WorldCollision::stats`].
#[derive(Clone, Copy, Debug, Default)]
pub struct CollisionStats {
    pub cells: usize,
    pub loading: usize,
    pub triangles: usize,
    pub bytes: usize,
    /// Distinct models loaded / of those without collision geometry.
    pub models: usize,
    pub models_without: usize,
    pub builds: u64,
    pub build_ms: f32,
}

/// Terrain heights of a ray's neighbourhood with the current cell kept.
struct TerrainSampler<'a> {
    terrain: &'a TerrainQuery,
    key: (i32, i32),
    cell: Option<Arc<CellHeights>>,
}

impl<'a> TerrainSampler<'a> {
    fn new(terrain: &'a TerrainQuery) -> Self {
        TerrainSampler { terrain, key: (i32::MIN, i32::MIN), cell: None }
    }

    fn locate(&mut self, x: f32, y: f32) -> Option<(&CellHeights, f32, f32)> {
        if !(x.is_finite() && y.is_finite()) || x < 0.0 || y < 0.0 {
            return None;
        }
        let key = ((x / CELL_SIZE).floor() as i32, (y / CELL_SIZE).floor() as i32);
        if key != self.key {
            self.cell = self.terrain.cell(key.0, key.1);
            self.key = key;
        }
        let cell = self.cell.as_deref()?;
        Some((cell, x - key.0 as f32 * CELL_SIZE, y - key.1 as f32 * CELL_SIZE))
    }

    fn height(&mut self, x: f32, y: f32) -> Option<f32> {
        self.locate(x, y).map(|(c, lx, ly)| c.mesh_height(lx, ly))
    }

    fn normal(&mut self, x: f32, y: f32) -> Option<Vec3> {
        self.locate(x, y).map(|(c, lx, ly)| Vec3::from(c.mesh_normal(lx, ly)))
    }
}

impl WorldCollision {
    pub fn new(world: &str, terrain: TerrainQuery) -> WorldCollision {
        WorldCollision::with_config(world, terrain, CollisionConfig::default())
    }

    pub fn with_config(world: &str, terrain: TerrainQuery, config: CollisionConfig) -> WorldCollision {
        let min_nz = config.max_slope.cos();
        WorldCollision {
            inner: Arc::new(Inner {
                world: world.to_string(),
                terrain,
                config,
                min_nz,
                cells: RwLock::default(),
                models: Mutex::default(),
                builds: AtomicU64::new(0),
                build_micros: AtomicU64::new(0),
            }),
        }
    }

    pub fn world(&self) -> &str {
        &self.inner.world
    }

    pub fn config(&self) -> &CollisionConfig {
        &self.inner.config
    }

    /// The terrain queries this collision is built on (for the fallback when [`ground_below`](Self::ground_below)
    /// finds nothing).
    pub fn terrain(&self) -> &TerrainQuery {
        &self.inner.terrain
    }

    // ---------------------------------------------------------------- loading

    /// Starts building the brush collision of cell `(cx, cy)` on the worker threads unless it is built or
    /// building (just after the terrain cells in the queue of the workers; [`request_around`](Self::request_around) orders by distance).
    pub fn request(&self, cx: i32, cy: i32) {
        self.request_with_priority(cx, cy, -45_000.0);
    }

    fn request_with_priority(&self, cx: i32, cy: i32, priority: f32) {
        if cx < 0 || cy < 0 {
            return;
        }
        if self.inner.cells.read().unwrap_or_else(PoisonError::into_inner).contains_key(&(cx, cy)) {
            return;
        }
        {
            let mut cells = self.inner.cells.write().unwrap_or_else(PoisonError::into_inner);
            if cells.contains_key(&(cx, cy)) {
                return;
            }
            cells.insert((cx, cy), Entry::Loading);
        }
        let inner = self.inner.clone();
        aa_cry::workers::Workers::global().spawn(priority, move || inner.build_cell(cx, cy));
    }

    /// [`request`](Self::request) for every cell within `radius` metres of `(x, y)`, nearest first.
    pub fn request_around(&self, x: f32, y: f32, radius: f32) {
        for (d, (cx, cy)) in cells_within(x, y, radius) {
            self.request_with_priority(cx, cy, -45_000.0 + d);
        }
    }

    /// Whether every cell within `radius` metres of `(x, y)` is built (or has nothing to build).
    pub fn ready_around(&self, x: f32, y: f32, radius: f32) -> bool {
        let cells = self.inner.cells.read().unwrap_or_else(PoisonError::into_inner);
        cells_within(x, y, radius).into_iter().all(|(_, c)| matches!(cells.get(&c), Some(Entry::Ready(_))))
    }

    /// Drops the built cells farther than `keep` metres from `(x, y)`.
    pub fn release_outside(&self, x: f32, y: f32, keep: f32) {
        let near: Vec<(i32, i32)> = cells_within(x, y, keep).into_iter().map(|(_, c)| c).collect();
        let far: Vec<(i32, i32)> = {
            let cells = self.inner.cells.read().unwrap_or_else(PoisonError::into_inner);
            cells.iter().filter(|(k, e)| matches!(e, Entry::Ready(_)) && !near.contains(k)).map(|(k, _)| *k).collect()
        };
        if far.is_empty() {
            return;
        }
        let mut cells = self.inner.cells.write().unwrap_or_else(PoisonError::into_inner);
        for k in far {
            if matches!(cells.get(&k), Some(Entry::Ready(_))) {
                cells.remove(&k);
            }
        }
    }

    /// Builds the cells near `(x, y)` ([`CollisionConfig::radius`]) and drops the far ones
    /// ([`CollisionConfig::keep`]). `AaWorldPlugin` calls this for the camera a few times per second.
    pub fn follow(&self, x: f32, y: f32) {
        if self.inner.config.radius <= 0.0 {
            return;
        }
        self.request_around(x, y, self.inner.config.radius);
        self.release_outside(x, y, self.inner.config.keep);
    }

    /// Builds cell `(cx, cy)` on the calling thread (tools and tests); a cell that is built stays as it is.
    pub fn build_blocking(&self, cx: i32, cy: i32) -> Option<Arc<CellCollision>> {
        {
            let mut cells = self.inner.cells.write().unwrap_or_else(PoisonError::into_inner);
            match cells.get(&(cx, cy)) {
                Some(Entry::Ready(c)) => return Some(c.clone()),
                Some(Entry::Loading) => {}
                None => {
                    cells.insert((cx, cy), Entry::Loading);
                }
            }
        }
        self.inner.build_cell(cx, cy);
        self.cell(cx, cy)
    }

    /// The built collision of a cell.
    pub fn cell(&self, cx: i32, cy: i32) -> Option<Arc<CellCollision>> {
        match self.inner.cells.read().unwrap_or_else(PoisonError::into_inner).get(&(cx, cy)) {
            Some(Entry::Ready(c)) => Some(c.clone()),
            _ => None,
        }
    }

    /// The proxy geometry of a model (loaded once, shared by all cells); `None` when it has none.
    pub fn model(&self, path: &str) -> Option<Arc<CollisionMesh>> {
        self.inner.model(path)
    }

    pub fn stats(&self) -> CollisionStats {
        let mut s = CollisionStats { builds: self.inner.builds.load(Ordering::Relaxed), build_ms: self.inner.build_micros.load(Ordering::Relaxed) as f32 / 1000.0, ..default() };
        for e in self.inner.cells.read().unwrap_or_else(PoisonError::into_inner).values() {
            match e {
                Entry::Loading => s.loading += 1,
                Entry::Ready(c) => {
                    s.cells += 1;
                    s.triangles += c.stats.structure_triangles + c.stats.prop_triangles;
                    s.bytes += c.stats.bytes;
                }
            }
        }
        let models = self.inner.models.lock().unwrap_or_else(PoisonError::into_inner);
        s.models = models.len();
        s.models_without = models.values().filter(|m| m.get().is_some_and(|m| m.is_none())).count();
        s
    }

    /// The built cells whose grid reaches the XY box (queries look a few metres beyond their own cell
    /// because a model sticks out of the cell its brush sits in).
    fn cells_near(&self, lo: Vec2, hi: Vec2) -> Vec<Arc<CellCollision>> {
        const REACH: f32 = 48.0;
        let range = |v: f32| ((v / CELL_SIZE).floor() as i32).max(0);
        let (x0, y0, x1, y1) = (range(lo.x - REACH), range(lo.y - REACH), range(hi.x + REACH), range(hi.y + REACH));
        let cells = self.inner.cells.read().unwrap_or_else(PoisonError::into_inner);
        let mut out = Vec::new();
        for cy in y0..=y1 {
            for cx in x0..=x1 {
                if let Some(Entry::Ready(c)) = cells.get(&(cx, cy))
                    && !c.is_empty()
                {
                    out.push(c.clone());
                }
            }
        }
        out
    }

    // ---------------------------------------------------------------- queries

    /// The highest walkable surface under `(x, y)` that is not above `z + step_up`: the terrain or a brush (a
    /// bridge, a floor, stairs) with a slope of at most [`CollisionConfig::max_slope`]. Pass the feet height for
    /// `z` and the tallest step the character climbs for `step_up` (0.5 m): a bridge deck 3 m over a river bed is
    /// ignored by someone standing on the bed and found by someone on the deck. `None` when everything under the
    /// point is above the limit (you are inside the ground; use [`terrain`](Self::terrain)) or outside the map.
    ///
    /// The feet are a disc of [`CollisionConfig::foot_radius`]: where no brush surface is under the centre, the
    /// highest one under the rim of the disc counts, so the seams between pieces of a bridge or between a quay and
    /// its stairs (the proxies leave gaps of up to 0.7 m) do not drop the character through.
    pub fn ground_below(&self, x: f32, y: f32, z: f32, step_up: f32) -> Option<Ground> {
        self.ground_below_radius(x, y, z, step_up, self.inner.config.foot_radius)
    }

    /// [`ground_below`](Self::ground_below) with the radius of the feet given (0: exactly under the point).
    pub fn ground_below_radius(&self, x: f32, y: f32, z: f32, step_up: f32, radius: f32) -> Option<Ground> {
        let limit = z + step_up;
        let mut best = self.inner.terrain.ground_at(x, y).filter(|g| g.height <= limit);
        let cells = self.cells_near(Vec2::new(x, y) - radius, Vec2::new(x, y) + radius);
        if cells.is_empty() {
            return best;
        }
        let probe = |dx: f32, dy: f32| -> Option<Ground> {
            let mut found: Option<Ground> = None;
            for cell in &cells {
                for soup in cell.soups(Mask::ALL) {
                    if let Some((height, normal)) = soup.highest_below(x + dx, y + dy, limit)
                        && found.is_none_or(|f| height > f.height)
                    {
                        found = Some(Ground { height, normal });
                    }
                }
            }
            found
        };
        // under the centre; only where nothing is, the rim of the feet looks for a surface across a seam
        let mut brush = probe(0.0, 0.0);
        if brush.is_none() && radius > 0.0 {
            const RIM: [(f32, f32); 8] = [(1.0, 0.0), (0.707, 0.707), (0.0, 1.0), (-0.707, 0.707), (-1.0, 0.0), (-0.707, -0.707), (0.0, -1.0), (0.707, -0.707)];
            for (dx, dy) in RIM {
                if let Some(g) = probe(dx * radius, dy * radius)
                    && brush.is_none_or(|b| g.height > b.height)
                {
                    brush = Some(g);
                }
            }
        }
        if let Some(g) = brush
            && best.is_none_or(|b| g.height > b.height)
        {
            best = Some(g);
        }
        best
    }

    /// First hit of the ray `origin + dir * t`, `t` in `0..=max`, with the terrain and the brushes of
    /// [`Mask::ALL`].
    pub fn raycast(&self, origin: Vec3, dir: Vec3, max: f32) -> Option<Hit> {
        self.raycast_mask(origin, dir, max, Mask::ALL)
    }

    /// [`raycast`](Self::raycast) against the surfaces of `mask`. `dir` need not be normalized; `t` is a distance
    /// in metres. A ray that starts inside the terrain hits it at `t = 0`.
    pub fn raycast_mask(&self, origin: Vec3, dir: Vec3, max: f32, mask: Mask) -> Option<Hit> {
        let dir = dir.try_normalize()?;
        if !(origin.is_finite() && max.is_finite()) || max < 0.0 {
            return None;
        }
        let mut best: Option<Hit> = None;
        if mask.contains(Mask::TERRAIN)
            && let Some((t, normal)) = self.terrain_ray(origin, dir, max, 0.0)
        {
            best = Some(Hit { t, point: origin + dir * t, normal, terrain: true });
        }
        let end = origin + dir * max;
        for cell in self.cells_near(origin.truncate().min(end.truncate()), origin.truncate().max(end.truncate())) {
            for soup in cell.soups(mask) {
                let reach = best.map_or(max, |b| b.t);
                if let Some((t, id)) = soup.ray(origin, dir, reach) {
                    let mut normal = cross_normal(soup.tri(id)).normalize_or(Vec3::Z);
                    if normal.dot(dir) > 0.0 {
                        normal = -normal;
                    }
                    best = Some(Hit { t, point: origin + dir * t, normal, terrain: false });
                }
            }
        }
        best
    }

    /// Where a third person camera may sit: the arm runs from `pivot` (above the player's head) towards `wanted`
    /// and stops where a sphere of `radius` (0.3 to 0.5 m, the near plane's reach) touches the terrain or a
    /// structure ([`Mask::CAMERA`]; barrels and fences do not stop it). Returns `wanted` when the arm is free.
    pub fn clip_camera(&self, pivot: Vec3, wanted: Vec3, radius: f32) -> Vec3 {
        let arm = wanted - pivot;
        let len = arm.length();
        if len < 1e-4 {
            return wanted;
        }
        match self.sweep_sphere(pivot, arm / len, len, radius, Mask::CAMERA) {
            Some(hit) => pivot + arm / len * hit.t,
            None => wanted,
        }
    }

    /// Moves a sphere of `radius` along `origin + dir * t` and returns where it first touches something: `t` is
    /// the distance its centre can travel (the camera sits at `origin + dir * t`), `point` and `normal` the
    /// contact. A sphere that already overlaps something hits at `t = 0`. The terrain is approximated as a
    /// plane under the sphere (exact on flat ground, a few centimetres off on steep slopes).
    pub fn sweep_sphere(&self, origin: Vec3, dir: Vec3, max: f32, radius: f32, mask: Mask) -> Option<Hit> {
        let dir = dir.try_normalize()?;
        if !(origin.is_finite() && max.is_finite()) || max < 0.0 {
            return None;
        }
        let mut best: Option<Hit> = None;
        if mask.contains(Mask::TERRAIN)
            && let Some((t, normal)) = self.terrain_ray(origin, dir, max, radius)
        {
            best = Some(Hit { t, point: origin + dir * t - normal * radius, normal, terrain: true });
        }
        let end = origin + dir * max;
        let reach_lo = origin.min(end) - radius - 0.1;
        let reach_hi = origin.max(end) + radius + 0.1;
        let mut tris: Vec<Tri> = Vec::new();
        let mut ids = Vec::new();
        for cell in self.cells_near(reach_lo.truncate(), reach_hi.truncate()) {
            for soup in cell.soups(mask) {
                ids.clear();
                soup.gather_along(origin, dir, max, radius + 0.1, &mut ids);
                tris.extend(ids.iter().map(|&id| *soup.tri(id)));
            }
        }
        if !tris.is_empty() {
            let limit = best.map_or(max, |b| b.t);
            if let Some(hit) = advance_sphere(&tris, origin, dir, limit, radius) {
                best = Some(hit);
            }
        }
        best
    }

    /// Hit of the terrain for the point (`lift` = 0) or the sphere of that radius travelling along the ray: `(t, normal)`.
    fn terrain_ray(&self, origin: Vec3, dir: Vec3, max: f32, lift: f32) -> Option<(f32, Vec3)> {
        const STEP: f32 = 1.0;
        let mut s = TerrainSampler::new(&self.inner.terrain);
        // height of the point over the surface it would rest on (the sphere centre is `lift` over the plane)
        let clearance = |s: &mut TerrainSampler, t: f32| -> Option<f32> {
            let p = origin + dir * t;
            let h = s.height(p.x, p.y)?;
            let lift = if lift > 0.0 { lift / s.normal(p.x, p.y)?.z.max(0.25) } else { 0.0 };
            Some(p.z - h - lift)
        };
        let normal_at = |s: &mut TerrainSampler, t: f32| {
            let p = origin + dir * t;
            let n = s.normal(p.x, p.y).unwrap_or(Vec3::Z);
            if n.dot(dir) > 0.0 { -n } else { n }
        };
        let mut before = clearance(&mut s, 0.0);
        if before.is_some_and(|c| c < 0.0) {
            return Some((0.0, normal_at(&mut s, 0.0)));
        }
        let mut t1 = 0.0f32;
        while t1 < max {
            let t0 = t1;
            t1 = (t1 + STEP).min(max);
            let now = clearance(&mut s, t1);
            if before.is_some_and(|b| b >= 0.0) && now.is_some_and(|n| n < 0.0) {
                let (mut lo, mut hi) = (t0, t1);
                for _ in 0..14 {
                    let mid = 0.5 * (lo + hi);
                    if clearance(&mut s, mid).is_some_and(|c| c < 0.0) {
                        hi = mid;
                    } else {
                        lo = mid;
                    }
                }
                return Some((lo, normal_at(&mut s, hi)));
            }
            before = now;
        }
        None
    }

    /// Pushes a standing capsule out of the walls around it: `feet` is the position of its feet, `radius` its
    /// radius, `height` its total height and `step_up` the tallest obstacle it steps over (surfaces below
    /// `feet.z + step_up` never push; give the same value as to [`ground_below`](Self::ground_below)). Only the
    /// horizontal position changes. The terrain is not a wall here (steep terrain is the controller's slope rule).
    pub fn push_out(&self, feet: Vec3, radius: f32, height: f32, step_up: f32) -> Pushed {
        let mut p = feet;
        let mut out = Pushed { pos: feet, blocked: false, normal: Vec3::ZERO };
        let z_lo = feet.z + step_up + radius;
        let z_hi = (feet.z + height - radius).max(z_lo);
        let reach = Vec3::new(radius + 0.5, radius + 0.5, 0.0);
        let (lo, hi) = (Vec3::new(feet.x, feet.y, z_lo - radius) - reach, Vec3::new(feet.x, feet.y, z_hi + radius) + reach);
        let mut tris: Vec<Tri> = Vec::new();
        let mut ids = Vec::new();
        for cell in self.cells_near(lo.truncate(), hi.truncate()) {
            for soup in cell.soups(Mask::ALL) {
                ids.clear();
                soup.gather(lo, hi, &mut ids);
                tris.extend(ids.iter().map(|&id| *soup.tri(id)));
            }
        }
        if tris.is_empty() {
            return out;
        }
        // spheres along the axis, at most one radius apart
        let n = if z_hi - z_lo < 1e-3 { 1 } else { ((z_hi - z_lo) / radius).ceil() as usize + 1 };
        let max_move = 2.0 * radius + 0.5;
        for _ in 0..6 {
            let mut moved = false;
            for k in 0..n {
                let z = if n == 1 { z_lo } else { z_lo + (z_hi - z_lo) * k as f32 / (n - 1) as f32 };
                let c = Vec3::new(p.x, p.y, z);
                // all contacts of this sphere at once: the deepest push per direction wins
                let mut push = Vec3::ZERO;
                for t in &tris {
                    let q = closest_point(c, t);
                    let v = c - q;
                    let d2 = v.length_squared();
                    if d2 >= radius * radius {
                        continue;
                    }
                    let d = d2.sqrt();
                    let dir = if d > 1e-4 {
                        v / d
                    } else {
                        let n = cross_normal(t).normalize_or(Vec3::X);
                        if n.dot(c - t[0]) >= 0.0 { n } else { -n }
                    };
                    let horizontal = Vec2::new(dir.x, dir.y);
                    let hl = horizontal.length();
                    if hl < 0.35 {
                        continue; // resting on a floor or touching a ceiling, not a wall
                    }
                    let depth = (radius - d) / hl;
                    let h = horizontal / hl * depth;
                    push.x = if h.x.abs() > push.x.abs() { h.x } else { push.x };
                    push.y = if h.y.abs() > push.y.abs() { h.y } else { push.y };
                }
                if push.length_squared() > 1e-10 {
                    p += push;
                    moved = true;
                    out.normal = push.normalize();
                }
            }
            if !moved {
                break;
            }
        }
        let shift = Vec2::new(p.x - feet.x, p.y - feet.y);
        let shift = if shift.length() > max_move { shift.normalize() * max_move } else { shift };
        out.pos = Vec3::new(feet.x + shift.x, feet.y + shift.y, feet.z);
        out.blocked = shift.length() > 1e-4;
        if !out.blocked {
            out.normal = Vec3::ZERO;
        }
        out
    }

    /// Moves the capsule horizontally by `delta` and slides it along the walls in the way: `push_out` after
    /// every half radius of the move, so thin walls cannot be crossed. The result keeps `from.z`.
    pub fn slide(&self, from: Vec3, delta: Vec2, radius: f32, height: f32, step_up: f32) -> Pushed {
        let steps = ((delta.length() / (radius * 0.5)).ceil() as usize).clamp(1, 64);
        let mut out = Pushed { pos: from, blocked: false, normal: Vec3::ZERO };
        for _ in 0..steps {
            let next = out.pos + (delta / steps as f32).extend(0.0);
            let pushed = self.push_out(next, radius, height, step_up);
            out.blocked |= pushed.blocked;
            if pushed.blocked {
                out.normal = pushed.normal;
            }
            out.pos = pushed.pos;
        }
        out
    }

    /// The collision triangles (structures and props of `mask`) whose bounding box overlaps the cube of half size
    /// `radius` around `center`, in game space, for debug drawing.
    pub fn triangles_near(&self, center: Vec3, radius: f32, mask: Mask) -> Vec<[Vec3; 3]> {
        let (lo, hi) = (center - radius, center + radius);
        let mut out = Vec::new();
        let mut ids = Vec::new();
        for cell in self.cells_near(lo.truncate(), hi.truncate()) {
            for soup in cell.soups(mask) {
                ids.clear();
                soup.gather(lo, hi, &mut ids);
                out.extend(ids.iter().map(|&id| *soup.tri(id)));
            }
        }
        out
    }
}

/// Conservative advancement of a sphere along a ray against a few triangles: `Hit` at the first contact.
fn advance_sphere(tris: &[Tri], origin: Vec3, dir: Vec3, max: f32, radius: f32) -> Option<Hit> {
    let nearest = |p: Vec3| -> (f32, Vec3) {
        let mut best = (f32::INFINITY, p);
        for t in tris {
            let q = closest_point(p, t);
            let d2 = (p - q).length_squared();
            if d2 < best.0 {
                best = (d2, q);
            }
        }
        (best.0.sqrt(), best.1)
    };
    let contact = |t: f32| -> Hit {
        let c = origin + dir * t;
        let (_, q) = nearest(c);
        let normal = (c - q).try_normalize().unwrap_or(-dir);
        Hit { t, point: q, normal, terrain: false }
    };
    let eps = 1e-3;
    let min_step = radius * 0.25;
    let mut t = 0.0f32;
    let mut free = None; // last distance known to be clear
    for _ in 0..400 {
        let (d, _) = nearest(origin + dir * t);
        let gap = d - radius;
        if gap <= eps {
            return match free {
                // first contact is between `free` and `t`: bisect
                Some(mut lo) => {
                    let mut hi = t;
                    for _ in 0..10 {
                        let mid = 0.5 * (lo + hi);
                        if nearest(origin + dir * mid).0 - radius > eps {
                            lo = mid;
                        } else {
                            hi = mid;
                        }
                    }
                    Some(contact(lo))
                }
                None => Some(contact(0.0)),
            };
        }
        free = Some(t);
        if t >= max {
            return None;
        }
        t = (t + gap.max(min_step)).min(max);
    }
    // out of steps (creeping along a surface): stop where it was last clear
    free.map(contact)
}

/// Cells within `radius` metres of `(x, y)` with their distance, nearest first.
fn cells_within(x: f32, y: f32, radius: f32) -> Vec<(f32, (i32, i32))> {
    let (c0x, c0y) = (((x - radius) / CELL_SIZE).floor() as i32, ((y - radius) / CELL_SIZE).floor() as i32);
    let (c1x, c1y) = (((x + radius) / CELL_SIZE).floor() as i32, ((y + radius) / CELL_SIZE).floor() as i32);
    let mut out = Vec::new();
    for cy in c0y.max(0)..=c1y {
        for cx in c0x.max(0)..=c1x {
            let (min_x, min_y) = (cx as f32 * CELL_SIZE, cy as f32 * CELL_SIZE);
            let d = Vec2::new(x - x.clamp(min_x, min_x + CELL_SIZE), y - y.clamp(min_y, min_y + CELL_SIZE)).length();
            if d <= radius {
                out.push((d, (cx, cy)));
            }
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out
}

impl Inner {
    fn model(&self, path: &str) -> Option<Arc<CollisionMesh>> {
        let key = aa_pak::normalize(path.trim());
        let slot = self.models.lock().unwrap_or_else(PoisonError::into_inner).entry(key.clone()).or_default().clone();
        slot.get_or_init(|| match aa_cry::physics::load(&key, self.config.visible_fallback) {
            Ok(mesh) => mesh.map(Arc::new),
            Err(e) => {
                debug!("aa-world: collision: {key}: {e}");
                None
            }
        })
        .clone()
    }

    /// Builds a cell and stores it (always: a cell without objects is stored empty so it is not built again).
    fn build_cell(&self, cx: i32, cy: i32) {
        let start = Instant::now();
        // a panic (a malformed file) must not leave the cell loading forever
        let mut cell = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.build_cell_data(cx, cy))).unwrap_or_else(|_| {
            warn!("aa-world: collision of cell {cx},{cy} failed to build");
            CellCollision { cell: (cx, cy), structures: Soup::default(), props: Soup::default(), stats: CellStats::default() }
        });
        cell.stats.build_ms = start.elapsed().as_secs_f32() * 1000.0;
        self.builds.fetch_add(1, Ordering::Relaxed);
        self.build_micros.fetch_add(start.elapsed().as_micros() as u64, Ordering::Relaxed);
        self.cells.write().unwrap_or_else(PoisonError::into_inner).insert((cx, cy), Entry::Ready(Arc::new(cell)));
    }

    fn build_cell_data(&self, cx: i32, cy: i32) -> CellCollision {
        let dir = cell_dir(&self.world, cx, cy);
        let pak = aa_pak::Pak::global();
        let read = |f: &str| pak.read(&format!("{dir}/client/{f}")).ok();
        // brushes as the renderer reads them: brush.dat with statobjs.dat, else the object.dat octree
        let (brushes, models) = match (read("brush.dat").and_then(|d| parse_brush_dat(&d).ok()), read("statobjs.dat")) {
            (Some(brushes), Some(statobjs)) => (brushes, parse_statobjs(&statobjs)),
            _ => read("object.dat").and_then(|d| parse_object_dat(&d).ok()).map(|o| (o.brushes, o.models)).unwrap_or_default(),
        };
        let origin = Vec3::new(cx as f32 * CELL_SIZE, cy as f32 * CELL_SIZE, 0.0);
        let mut stats = CellStats { brushes: brushes.len(), ..default() };
        let meshes: Vec<OnceLock<Option<Arc<CollisionMesh>>>> = models.iter().map(|_| OnceLock::new()).collect();
        let (mut structures, mut props): (Vec<Tri>, Vec<Tri>) = (Vec::new(), Vec::new());
        for b in &brushes {
            let Some(path) = models.get(b.model as usize) else { continue };
            let Some(mesh) = meshes[b.model as usize].get_or_init(|| self.model(path)) else { continue };
            let m = b.mat4();
            if !m.is_finite() || m.determinant().abs() < 1e-9 {
                continue;
            }
            // game space: cell-local to world
            let m = Mat4::from_translation(origin) * m;
            let mirrored = m.determinant() < 0.0;
            let world: Vec<Vec3> = mesh.positions.iter().map(|p| m.transform_point3(*p)).collect();
            let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            for p in &world {
                lo = lo.min(*p);
                hi = hi.max(*p);
            }
            let size = hi - lo;
            let target = if size.x * size.y * size.z < self.config.prop_volume { &mut props } else { &mut structures };
            target.extend(mesh.indices.as_chunks::<3>().0.iter().map(|t| {
                let (a, b, c) = (world[t[0] as usize], world[t[1] as usize], world[t[2] as usize]);
                if mirrored { [a, c, b] } else { [a, b, c] }
            }));
            stats.colliding += 1;
        }
        stats.models = meshes.iter().filter(|m| m.get().is_some_and(|m| m.is_some())).count();
        stats.structure_triangles = structures.len();
        stats.prop_triangles = props.len();
        let structures = Soup::build(structures, self.config.grid, self.min_nz);
        let props = Soup::build(props, self.config.grid, self.min_nz);
        stats.bytes = structures.bytes() + props.bytes();
        CellCollision { cell: (cx, cy), structures, props, stats }
    }
}

/// `AaWorldPlugin`: inserts the world's [`WorldCollision`] (unless the app has one) and builds the cells around
/// the camera.
pub(crate) fn build(app: &mut App, cfg: &WorldConfig) {
    if !app.world().contains_resource::<WorldCollision>() {
        let terrain = app.world().get_resource::<TerrainQuery>().cloned().unwrap_or_else(|| TerrainQuery::new(&cfg.world));
        app.insert_resource(WorldCollision::new(&cfg.world, terrain));
    }
    // `AA_NO_COLLISION=1`: the cells are not built by the plugin (for A/B measurements; queries still work after `request`)
    if std::env::var_os("AA_NO_COLLISION").is_none() {
        app.add_systems(Update, follow_focus);
    }
}

/// Keeps the collision cells around the focus built (the [`WorldFocus`] entity, else the first camera), a few
/// times per second.
fn follow_focus(
    time: Res<Time>,
    collision: Res<WorldCollision>,
    focus: Query<&GlobalTransform, With<WorldFocus>>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    mut since: Local<f32>,
) {
    *since += time.delta_secs();
    if *since < 0.25 {
        return;
    }
    let Some(t) = focus.iter().next().or_else(|| cameras.iter().next()) else { return };
    *since = 0.0;
    let p = crate::coords::bevy_to_cry(t.translation());
    collision.follow(p.x, p.y);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Marianople's market place and Lutesong Harbor's centre (the middle of its `sub_zones` box).
    const LUTESONG: (f32, f32) = (21_131.0, 10_594.0);

    fn world() -> Option<WorldCollision> {
        crate::pak_available().then(|| WorldCollision::new("main_world", TerrainQuery::new("main_world")))
    }

    #[test]
    fn sphere_advances_to_contact() {
        // a wall at x = 10 (two triangles) and a sphere of 0.5 coming from the west
        let wall = [[Vec3::new(10.0, -5.0, -5.0), Vec3::new(10.0, 5.0, -5.0), Vec3::new(10.0, 5.0, 5.0)], [Vec3::new(10.0, -5.0, -5.0), Vec3::new(10.0, 5.0, 5.0), Vec3::new(10.0, -5.0, 5.0)]];
        let hit = advance_sphere(&wall, Vec3::ZERO, Vec3::X, 30.0, 0.5).unwrap();
        assert!((hit.t - 9.5).abs() < 0.01, "{}", hit.t);
        assert!((hit.normal - Vec3::NEG_X).length() < 1e-3 && (hit.point.x - 10.0).abs() < 1e-3);
        // too short, and passing beside it
        assert!(advance_sphere(&wall, Vec3::ZERO, Vec3::X, 8.0, 0.5).is_none());
        assert!(advance_sphere(&wall, Vec3::new(0.0, 8.0, 0.0), Vec3::X, 30.0, 0.5).is_none());
        // grazing along a wall does not take forever or hit
        assert!(advance_sphere(&wall, Vec3::new(0.0, 0.0, 6.0), Vec3::X, 30.0, 0.5).is_none());
        // already inside
        assert_eq!(advance_sphere(&wall, Vec3::new(9.8, 0.0, 0.0), Vec3::X, 5.0, 0.5).unwrap().t, 0.0);
    }

    #[test]
    fn terrain_rays() {
        let Some(w) = world() else { return };
        // the Nuian spawn meadow
        let (x, y) = (15_575.0, 15_375.0);
        let h = w.terrain().height_at(x, y).unwrap();
        let hit = w.raycast_mask(Vec3::new(x, y, h + 10.0), Vec3::NEG_Z, 50.0, Mask::TERRAIN).unwrap();
        assert!((hit.t - 10.0).abs() < 0.01 && hit.terrain && hit.normal.z > 0.5, "{hit:?}");
        // a short ray, and one from under the ground
        assert!(w.raycast_mask(Vec3::new(x, y, h + 10.0), Vec3::NEG_Z, 5.0, Mask::TERRAIN).is_none());
        assert_eq!(w.raycast_mask(Vec3::new(x, y, h - 1.0), Vec3::X, 5.0, Mask::TERRAIN).unwrap().t, 0.0);
        // a slanted ray lands on the same surface it would be measured against
        let dir = Vec3::new(1.0, 0.0, -0.5).normalize();
        let hit = w.raycast_mask(Vec3::new(x, y, h + 6.0), dir, 100.0, Mask::TERRAIN).unwrap();
        let ground = w.terrain().height_at(hit.point.x, hit.point.y).unwrap();
        assert!((hit.point.z - ground).abs() < 0.01, "{} {ground}", hit.point.z);
        // a sphere rests one radius above flat ground
        let sweep = w.sweep_sphere(Vec3::new(x, y, h + 3.0), Vec3::NEG_Z, 10.0, 0.5, Mask::TERRAIN).unwrap();
        assert!((sweep.t - (3.0 - 0.5 / w.terrain().normal_at(x, y).unwrap().z)).abs() < 0.05, "{}", sweep.t);
    }

    /// Which models make a cell's collision big (`--ignored --nocapture`).
    #[test]
    #[ignore]
    fn survey_cell() {
        let Some(w) = world() else { return };
        for (cx, cy) in [(20, 10), (10, 11), (14, 11)] {
            let dir = cell_dir("main_world", cx, cy);
            let pak = aa_pak::Pak::global();
            let brushes = parse_brush_dat(&pak.read(&format!("{dir}/client/brush.dat")).unwrap()).unwrap();
            let models = parse_statobjs(&pak.read(&format!("{dir}/client/statobjs.dat")).unwrap());
            let mut per: HashMap<usize, (usize, usize)> = HashMap::new();
            let mut without = 0;
            for b in &brushes {
                match w.model(&models[b.model as usize]) {
                    Some(m) => {
                        let e = per.entry(b.model as usize).or_default();
                        e.0 += 1;
                        e.1 = m.indices.len() / 3;
                    }
                    None => without += 1,
                }
            }
            let mut rows: Vec<_> = per.iter().map(|(m, (n, t))| (n * t, *n, *t, models[*m].clone())).collect();
            rows.sort_by(|a, b| b.cmp(a));
            println!("cell {cx},{cy}: {} brushes, {without} without collision, {} models", brushes.len(), per.len());
            for (total, n, t, name) in rows.iter().take(14) {
                println!("  {total:7} tris = {n:4} x {t:5}  {name}");
            }
        }
    }

    /// Query costs on the dense cells of Lutesong Harbor and Marianople (`--ignored --nocapture`).
    #[test]
    #[ignore]
    fn query_costs() {
        let Some(w) = world() else { return };
        for (name, (px, py), (cx, cy)) in [("lutesong", LUTESONG, (20, 10)), ("marianople", (11_186.0, 12_002.0), (10, 11))] {
            let t = Instant::now();
            let cell = w.build_blocking(cx, cy).unwrap();
            println!("{name}: build (cold models) {:.0} ms, {:?}", t.elapsed().as_secs_f32() * 1000.0, cell.stats);
            // warm: again without the model cache
            let t = Instant::now();
            let again = WorldCollision::new("main_world", w.terrain().clone());
            again.build_blocking(cx, cy);
            println!("{name}: build (cold pak, cold models) {:.0} ms", t.elapsed().as_secs_f32() * 1000.0);
            let rebuilt = w.inner.build_cell_data(cx, cy);
            println!("{name}: rebuild with the model cache {:.0} ms", {
                let t = Instant::now();
                let _ = w.inner.build_cell_data(cx, cy);
                t.elapsed().as_secs_f32() * 1000.0
            });
            drop(rebuilt);
            let mut seed = 99u32;
            let mut rnd = || {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (seed >> 8) as f32 / 16777216.0
            };
            let points: Vec<Vec3> = (0..2000)
                .map(|_| {
                    let (x, y) = (px - 80.0 + rnd() * 160.0, py - 80.0 + rnd() * 160.0);
                    Vec3::new(x, y, w.terrain().height_at(x, y).unwrap() + 1.7)
                })
                .collect();
            let time = |label: &str, f: &dyn Fn(Vec3, Vec3) -> usize| {
                let t = Instant::now();
                let mut hits = 0;
                for (i, p) in points.iter().enumerate() {
                    let a = i as f32 * 2.399;
                    let dir = Vec3::new(a.cos(), a.sin(), 0.35).normalize();
                    hits += f(*p + Vec3::Z * 0.6, dir);
                }
                println!("  {label}: {:.1} us per call, {hits} of {} hit", t.elapsed().as_secs_f32() * 1e6 / points.len() as f32, points.len());
            };
            time("raycast 10 m (terrain + brushes)", &|o, d| w.raycast(o, d, 10.0).is_some() as usize);
            time("raycast 10 m (camera mask)", &|o, d| w.raycast_mask(o, d, 10.0, Mask::CAMERA).is_some() as usize);
            time("sweep_sphere 10 m r 0.3 (camera mask)", &|o, d| w.sweep_sphere(o, d, 10.0, 0.3, Mask::CAMERA).is_some() as usize);
            time("ground_below", &|o, _| w.ground_below(o.x, o.y, o.z - 1.7, 0.5).is_some() as usize);
            time("push_out r 0.35 h 1.8", &|o, _| w.push_out(o - Vec3::Z * 1.7, 0.35, 1.8, 0.5).blocked as usize);
            time("slide 0.2 m", &|o, d| w.slide(o - Vec3::Z * 1.7, Vec2::new(d.x, d.y) * 0.2, 0.35, 1.8, 0.5).blocked as usize);
        }
    }

    /// Prints standing spots on bridges and stairs and camera spots at walls (`--ignored --nocapture`).
    #[test]
    #[ignore]
    fn find_spots() {
        let Some(w) = world() else { return };
        for (name, (cx, cy)) in [("lutesong", (20, 10)), ("marianople", (10, 11))] {
            w.build_blocking(cx, cy).unwrap();
            let o = crate::placement::load_cell_objects("main_world", cx, cy);
            let origin = Vec3::new(cx as f32 * CELL_SIZE, cy as f32 * CELL_SIZE, 0.0);
            println!("== {name}");
            let mut shown: HashMap<String, usize> = HashMap::new();
            for b in &o.brushes {
                let model = &o.models[b.model as usize];
                let short = model.rsplit('/').next().unwrap_or(model).to_string();
                if !(short.contains("bridge") || short.contains("stair")) || short.contains("railing") || short.contains("leg") {
                    continue;
                }
                let n = shown.entry(short.clone()).or_default();
                if *n >= 1 {
                    continue;
                }
                let p = origin + b.translation();
                // highest walkable surface over the brush's centre
                let ground = w.ground_below(p.x, p.y, p.z + 12.0, 0.0);
                let terrain = w.terrain().height_at(p.x, p.y).unwrap();
                println!("  {short:36} at {:.1},{:.1}  brush z {:.2}  terrain {:.2}  ground_below {:?}", p.x, p.y, p.z, terrain, ground.map(|g| (g.height, g.normal.z)));
                *n += 1;
            }
        }
    }

    /// Camera spots where the arm would end inside a structure (`--ignored --nocapture`).
    #[test]
    #[ignore]
    fn find_camera_spots() {
        let Some(w) = world() else { return };
        w.build_blocking(20, 10).unwrap();
        let mut found = 0;
        for i in 0..4000 {
            let a = i as f32 * 2.399;
            let r = 5.0 + (i % 97) as f32;
            let (x, y) = (LUTESONG.0 + a.cos() * r * 2.0, LUTESONG.1 + a.sin() * r);
            let Some(g) = w.ground_below(x, y, 1000.0, 0.0) else { continue };
            let pivot = Vec3::new(x, y, g.height + 1.9);
            // the arm points away from the player at yaw a
            let back = Vec3::new(a.cos(), a.sin(), 0.25).normalize();
            let wanted = pivot + back * 7.0;
            let clipped = w.sweep_sphere(pivot, back, 7.0, 0.3, Mask::CAMERA);
            let Some(h) = clipped else { continue };
            if h.terrain || h.t > 3.0 || h.t < 1.0 {
                continue;
            }
            // is the wanted camera position inside something: a ray up from it hits a structure from below?
            let up = w.raycast_mask(wanted, Vec3::Z, 20.0, Mask::STRUCTURES);
            let down = w.raycast_mask(wanted, Vec3::NEG_Z, 3.0, Mask::STRUCTURES);
            if up.is_some() && down.is_some() && found < 6 {
                found += 1;
                println!("player {x:.1},{y:.1} feet {:.2}  yaw(cam behind) {:.0} deg  arm hits at {:.2} m  wanted cam {:.1},{:.1},{:.1} is inside", g.height, a.to_degrees().rem_euclid(360.0), h.t, wanted.x, wanted.y, wanted.z);
            }
        }
    }

    /// Walks a player over the arch bridge of Lutesong Harbor (20 m over the water, 10 m rise) with the queries a
    /// controller uses, in both directions.
    #[test]
    fn walk_over_the_bridge() {
        let Some(w) = world() else { return };
        w.build_blocking(20, 10).unwrap();
        // si_bridge_b01 at (21091.0, 10551.2), its long axis along (0.98, 0.19) in game space
        let (centre, axis) = (Vec2::new(21_091.0, 10_551.2), Vec2::new(0.98, 0.19).normalize());
        println!("profile along the bridge (z limit 125, no step):");
        for i in -12..=12 {
            let p = centre + axis * (i as f32 * 3.0);
            let g = w.ground_below(p.x, p.y, 125.0, 0.0);
            println!("  {:6.1} m  {:.1},{:.1}  terrain {:.2}  ground {:?}", i as f32 * 3.0, p.x, p.y, w.terrain().height_at(p.x, p.y).unwrap(), g.map(|g| (g.height, g.normal.z)));
        }
        // the controller: 6 m/s at 60 fps, capsule 0.35 x 1.8, step 0.5
        for (name, from, to) in [("west to east", -40.0f32, 40.0f32), ("east to west", 33.0, -40.0)] {
            let start = centre + axis * from;
            let terrain = w.terrain().height_at(start.x, start.y).unwrap();
            let mut pos = Vec3::new(start.x, start.y, w.ground_below(start.x, start.y, terrain, 0.5).unwrap().height);
            let step = axis * (to - from).signum() * 0.1;
            let (mut max_jump, mut top, mut blocked) = (0.0f32, f32::MIN, 0);
            for _ in 0..((to - from).abs() / 0.1) as usize {
                let moved = w.slide(pos, step, 0.35, 1.8, 0.5);
                blocked += moved.blocked as usize;
                let ground = w.ground_below(moved.pos.x, moved.pos.y, pos.z, 0.5);
                let z = ground.map_or(pos.z, |g| g.height);
                max_jump = max_jump.max((z - pos.z).abs());
                pos = Vec3::new(moved.pos.x, moved.pos.y, z);
                top = top.max(z);
            }
            println!("{name}: ended at {:.1},{:.1},{:.2} (terrain {:.2}), highest {top:.2}, biggest height change per 0.1 m step {max_jump:.2} m, blocked {blocked} steps", pos.x, pos.y, pos.z, w.terrain().height_at(pos.x, pos.y).unwrap());
            // over the crest at 113.8 (the water is at 90) and out on the other side
            assert!(top > 113.0, "{name}: only up to {top}");
            assert!((Vec2::new(pos.x, pos.y) - (centre + axis * to)).length() < 12.0, "{name}: stopped at {:.1},{:.1}", pos.x, pos.y);
            assert!(blocked < 100, "{name}: blocked {blocked} steps");
        }
    }

    /// A walking player (the controller's loop: slide, then ground) never has its centre cross a wall, and
    /// ends every step free of penetration.
    #[test]
    fn walking_never_crosses_a_wall() {
        let Some(w) = world() else { return };
        for ((cx, cy), (px, py)) in [((20, 10), LUTESONG), ((10, 11), (11_186.0, 12_002.0))] {
            w.build_blocking(cx, cy).unwrap();
            let mut seed = 7u32;
            let mut rnd = || {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (seed >> 8) as f32 / 16777216.0
            };
            let (mut steps, mut crossed, mut blocked, mut stuck) = (0, 0, 0, 0);
            for _ in 0..200 {
                let (x, y) = (px - 90.0 + rnd() * 180.0, py - 60.0 + rnd() * 120.0);
                let Some(g) = w.ground_below(x, y, 1000.0, 0.0) else { continue };
                let mut pos = Vec3::new(x, y, g.height);
                let a = rnd() * std::f32::consts::TAU;
                let step = Vec2::new(a.cos(), a.sin()) * 0.1;
                for _ in 0..300 {
                    let moved = w.slide(pos, step, 0.35, 1.8, 0.5);
                    blocked += moved.blocked as usize;
                    // the centre's path at chest height must not cross a structure or a prop
                    let (from, to) = (pos + Vec3::Z, moved.pos + Vec3::Z);
                    let len = (to - from).length();
                    // (a start inside a model is pushed out by up to 1.2 m in one step: not a crossing)
                    if (1e-4..0.5).contains(&len) && w.raycast_mask(from, to - from, len, Mask::STRUCTURES | Mask::PROPS).is_some_and(|h| h.normal.z.abs() < 0.5) {
                        crossed += 1;
                    }
                    let z = w.ground_below(moved.pos.x, moved.pos.y, pos.z, 0.5).map_or(pos.z, |g| g.height);
                    pos = Vec3::new(moved.pos.x, moved.pos.y, z);
                    // nothing pushes the capsule any more
                    stuck += w.push_out(pos, 0.35, 1.8, 0.5).blocked as usize;
                    steps += 1;
                }
            }
            println!("cell {cx},{cy}: {steps} steps, {blocked} blocked, {crossed} crossed a wall, {stuck} still penetrating");
            assert!(blocked > 100, "the walkers never met a wall");
            assert!(crossed * 1000 < steps, "{crossed} of {steps} steps crossed a wall");
        }
    }

    /// Walks up every staircase model of four towns from its low end to its high end (`--ignored --nocapture`).
    #[test]
    #[ignore]
    fn climb_the_stairs() {
        let Some(w) = world() else { return };
        let (mut tried, mut climbed) = (0, 0);
        for (cx, cy) in [(20, 10), (10, 11), (10, 12), (11, 11)] {
            w.build_blocking(cx, cy).unwrap();
            let o = crate::placement::load_cell_objects("main_world", cx, cy);
            let origin = Vec3::new(cx as f32 * CELL_SIZE, cy as f32 * CELL_SIZE, 0.0);
            let mut seen: HashMap<String, usize> = HashMap::new();
            for b in &o.brushes {
                let model = &o.models[b.model as usize];
                let short = model.rsplit('/').next().unwrap_or(model).to_string();
                if !(short.contains("stair") || short.contains("step")) || short.contains("rail") || short.contains("add") {
                    continue;
                }
                let n = seen.entry(short.clone()).or_default();
                if *n >= 2 {
                    continue;
                }
                *n += 1;
                let Some(mesh) = w.model(model) else { continue };
                let m = Mat4::from_translation(origin) * b.mat4();
                let pts: Vec<Vec3> = mesh.positions.iter().map(|p| m.transform_point3(*p)).collect();
                let (zmin, zmax) = pts.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.z), b.max(p.z)));
                if zmax - zmin < 1.0 {
                    continue;
                }
                let centroid = |f: &dyn Fn(f32) -> bool| {
                    let sel: Vec<&Vec3> = pts.iter().filter(|p| f(p.z)).collect();
                    sel.iter().fold(Vec3::ZERO, |a, p| a + **p) / sel.len().max(1) as f32
                };
                let low = centroid(&|z| z < zmin + 0.2 * (zmax - zmin));
                let high = centroid(&|z| z > zmax - 0.2 * (zmax - zmin));
                let run = Vec2::new(high.x - low.x, high.y - low.y);
                if run.length() < 1.5 {
                    continue;
                }
                let dir = run.normalize();
                let start = Vec2::new(low.x, low.y) - dir * 1.5;
                let Some(g0) = w.ground_below(start.x, start.y, low.z + 1.0, 0.5) else { continue };
                tried += 1;
                let mut pos = Vec3::new(start.x, start.y, g0.height);
                let steps = ((run.length() + 3.0) / 0.1) as usize;
                for _ in 0..steps {
                    let moved = w.slide(pos, dir * 0.1, 0.35, 1.8, 0.5);
                    let z = w.ground_below(moved.pos.x, moved.pos.y, pos.z, 0.5).map_or(pos.z, |g| g.height);
                    pos = Vec3::new(moved.pos.x, moved.pos.y, z);
                }
                let top_ok = pos.z > high.z - 0.8;
                climbed += top_ok as usize;
                println!("  {short:34} rise {:.1} m over {:.1} m (slope {:.0} deg): start z {:.2}, end z {:.2} of {:.2}  {}", high.z - low.z, run.length(), ((high.z - low.z) / run.length()).atan().to_degrees(), g0.height, pos.z, high.z, if top_ok { "up" } else { "STUCK" });
            }
        }
        println!("{climbed} of {tried} staircases climbed");
    }

    /// Every model placed in the main world: how many have collision geometry, how many fail to parse
    /// (`--ignored --nocapture`).
    #[test]
    #[ignore]
    fn survey_world() {
        if !crate::pak_available() {
            return;
        }
        let pak = aa_pak::Pak::global();
        let mut instances: HashMap<String, usize> = HashMap::new();
        let t = Instant::now();
        for cy in 0..40 {
            for cx in 0..40 {
                let dir = cell_dir("main_world", cx, cy);
                let (Ok(b), Ok(s)) = (pak.read(&format!("{dir}/client/brush.dat")), pak.read(&format!("{dir}/client/statobjs.dat"))) else { continue };
                let models = parse_statobjs(&s);
                for br in parse_brush_dat(&b).unwrap_or_default() {
                    if let Some(m) = models.get(br.model as usize) {
                        *instances.entry(aa_pak::normalize(m.trim())).or_default() += 1;
                    }
                }
            }
        }
        println!("placements read in {:.1} s: {} distinct models, {} brushes", t.elapsed().as_secs_f32(), instances.len(), instances.values().sum::<usize>());
        let t = Instant::now();
        let (mut physics, mut visible, mut none, mut missing, mut errors) = ((0, 0), (0, 0), (0, 0), (0, 0), Vec::new());
        let (mut tris_total, mut max_tris) = (0usize, (0usize, String::new()));
        let mut without: Vec<(usize, String, Vec3)> = Vec::new();
        for (path, n) in &instances {
            match pak.read(path) {
                Err(_) => {
                    missing.0 += 1;
                    missing.1 += n;
                }
                Ok(bytes) => match aa_cry::physics::collision_mesh(&bytes, true) {
                    Ok(Some(m)) if m.source == aa_cry::physics::Source::Physics => {
                        physics.0 += 1;
                        physics.1 += n;
                        tris_total += m.indices.len() / 3;
                        if m.indices.len() / 3 > max_tris.0 {
                            max_tris = (m.indices.len() / 3, path.clone());
                        }
                    }
                    Ok(Some(m)) => {
                        visible.0 += 1;
                        visible.1 += n;
                        let (lo, hi) = m.positions.iter().fold((Vec3::MAX, Vec3::MIN), |(a, b), p| (a.min(*p), b.max(*p)));
                        without.push((*n, path.clone(), hi - lo));
                    }
                    Ok(None) => {
                        none.0 += 1;
                        none.1 += n;
                    }
                    Err(e) => errors.push((path.clone(), e.to_string(), *n)),
                },
            }
        }
        println!("{:.1} s: with physics {physics:?} (models, brushes), no physics but a visible mesh {visible:?}, nothing {none:?}, file missing {missing:?}, {} parse errors", t.elapsed().as_secs_f32(), errors.len());
        println!("physics triangles of all models: {tris_total}, biggest {max_tris:?}");
        without.sort_by_key(|w| std::cmp::Reverse(w.0));
        for (n, p, size) in without.iter().take(30) {
            println!("  no physics: {n:5} x {p}  ({:.1} x {:.1} x {:.1} m)", size.x, size.y, size.z);
        }
        for (p, e, n) in errors.iter().take(15) {
            println!("  error {n:5} x {p}: {e}");
        }
        assert!(errors.is_empty());
    }

    /// Build time and memory of the cells of the middle of the world (`--ignored --nocapture`).
    #[test]
    #[ignore]
    fn survey_cells() {
        let Some(w) = world() else { return };
        let mut rows = Vec::new();
        let t = Instant::now();
        for cy in 6..18 {
            for cx in 6..22 {
                if let Some(c) = w.build_blocking(cx, cy)
                    && c.stats.brushes > 0
                {
                    rows.push((cx, cy, c.stats));
                }
            }
        }
        println!("{} cells with brushes built in {:.1} s on one thread (models cached across cells)", rows.len(), t.elapsed().as_secs_f32());
        let mut mb: Vec<f32> = rows.iter().map(|r| r.2.bytes as f32 / 1e6).collect();
        let mut ms: Vec<f32> = rows.iter().map(|r| r.2.build_ms).collect();
        let mut tris: Vec<usize> = rows.iter().map(|r| r.2.structure_triangles + r.2.prop_triangles).collect();
        mb.sort_by(f32::total_cmp);
        ms.sort_by(f32::total_cmp);
        tris.sort();
        let q = |v: &[f32], p: f32| v[((v.len() - 1) as f32 * p) as usize];
        println!("memory per cell MB: median {:.1}, 90 % {:.1}, max {:.1}", q(&mb, 0.5), q(&mb, 0.9), mb.last().unwrap());
        println!("build per cell ms: median {:.0}, 90 % {:.0}, max {:.0}", q(&ms, 0.5), q(&ms, 0.9), ms.last().unwrap());
        println!("triangles per cell: median {}, max {}", tris[tris.len() / 2], tris.last().unwrap());
        rows.sort_by_key(|r| std::cmp::Reverse(r.2.bytes));
        for (cx, cy, st) in rows.iter().take(4) {
            println!("  biggest: cell {cx},{cy}: {:.0} MB, {} brushes, {} + {} triangles, {:.0} ms", st.bytes as f32 / 1e6, st.brushes, st.structure_triangles, st.prop_triangles, st.build_ms);
        }
        let st = w.stats();
        println!("models cached {} ({} without collision), total {:.0} MB in {} cells", st.models, st.models_without, st.bytes as f32 / 1e6, st.cells);
    }

    /// The third person camera at Lutesong Harbor: 7 m behind a player who stands at a house, the arm ends in the
    /// wall; the clipped camera is in front of it (`shots/aa-world/collide_a_clip.png`).
    #[test]
    fn camera_stops_at_the_wall() {
        let Some(w) = world() else { return };
        w.build_blocking(20, 10).unwrap();
        let (x, y) = (21_140.0f32, 10_596.2f32);
        let feet = w.ground_below(x, y, 1000.0, 0.0).unwrap().height;
        let pivot = Vec3::new(x, y, feet + 1.9);
        let wanted = pivot + Vec3::new(26f32.to_radians().cos(), 26f32.to_radians().sin(), 0.25).normalize() * 7.0;
        let camera = w.clip_camera(pivot, wanted, 0.3);
        let arm = (camera - pivot).length();
        assert!(arm > 0.8 && arm < 2.0, "arm {arm}");
        // the wanted camera is inside a structure (something solid above and below it), the clipped one is not
        let inside = |p: Vec3| w.raycast_mask(p, Vec3::Z, 30.0, Mask::STRUCTURES).is_some() && w.raycast_mask(p, Vec3::NEG_Z, 30.0, Mask::STRUCTURES).is_some();
        assert!(inside(wanted), "{wanted}");
        assert!(!inside(camera), "{camera}");
        // free air: the camera stays where it wants to be; the terrain stops a camera that would go under it
        let up = pivot + Vec3::Z * 5.0;
        assert_eq!(w.clip_camera(pivot, up, 0.3), up);
        let down = w.clip_camera(pivot, pivot - Vec3::Z * 30.0, 0.3);
        assert!(down.z > feet - 1.0 && down.z < feet + 1.0, "{down}");
    }

    /// Needs the real client: builds the cell of Lutesong Harbor and checks the queries against its models.
    #[test]
    fn lutesong_harbor_walls_and_bridges() {
        let Some(w) = world() else { return };
        let (cx, cy) = ((LUTESONG.0 / CELL_SIZE) as i32, (LUTESONG.1 / CELL_SIZE) as i32);
        let cell = w.build_blocking(cx, cy).unwrap();
        println!("lutesong cell: {:?}", cell.stats);
        assert!(cell.stats.colliding > 1000 && cell.stats.structure_triangles > 10_000, "{:?}", cell.stats);
        assert!(w.stats().cells == 1);
        // every ray from far above hits something, brush or terrain, and never below the terrain
        let mut brush_hits = 0;
        for i in 0..200 {
            let (x, y) = (LUTESONG.0 - 200.0 + i as f32 * 2.0, LUTESONG.1 - 100.0 + (i * 7 % 200) as f32);
            let hit = w.raycast_mask(Vec3::new(x, y, 400.0), Vec3::NEG_Z, 500.0, Mask::ALL).unwrap();
            let ground = w.terrain().height_at(x, y).unwrap();
            assert!(hit.point.z >= ground - 0.01, "{x},{y}: hit {} under the terrain {ground}", hit.point.z);
            brush_hits += !hit.terrain as usize;
            // standing there: the highest ground not above the head is what the ray found (when walkable)
            if hit.normal.z > w.config().max_slope.cos() + 0.01 {
                let g = w.ground_below(x, y, hit.point.z, 0.2).unwrap();
                assert!((g.height - hit.point.z).abs() < 0.05, "ground {} under the hit {}", g.height, hit.point.z);
            }
        }
        println!("ray hits on brushes: {brush_hits} of 200");
        assert!(brush_hits > 20, "{brush_hits}");
    }
}
