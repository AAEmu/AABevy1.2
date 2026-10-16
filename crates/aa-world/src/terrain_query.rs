//! Ground queries for gameplay code: the height, normal and slope of the terrain under a game position.
//!
//! [`TerrainQuery`] reads the same `heightmap.dat` files the renderer streams, but on its own: it needs no
//! window, no [`AaWorldPlugin`](crate::AaWorldPlugin) and no cell to be spawned, so a player controller can
//! stand on the ground before the first terrain mesh exists (or on a server-side tool without Bevy's
//! rendering at all). It is a cheap clonable handle (`Arc` inside, `&self` methods, `Send + Sync`) and a
//! Bevy [`Resource`]; `AaWorldPlugin` inserts one for its world.
//!
//! ```no_run
//! # use aa_world::TerrainQuery;
//! let terrain = TerrainQuery::new("main_world");
//! // game coordinates, as the server sends them: metres, x east, y north, z up
//! if let Some(z) = terrain.height_at(11_800.0, 11_500.0) {
//!     println!("ground at {z:.2} m, slope {:?} rad", terrain.slope_at(11_800.0, 11_500.0));
//! }
//! ```
//!
//! **The surface** is the triangle mesh the renderer draws at its finest level of detail: a 2 m grid whose
//! quads are split along the diagonal from the south west to the north east corner (`terrain.rs`,
//! `build_patch_mesh`), so a character standing at [`height_at`](TerrainQuery::height_at) stands on the
//! visible ground to the millimetre close by (coarser levels of detail of far patches differ, as they do on
//! screen). [`normal_at`](TerrainQuery::normal_at) is the face normal of that triangle, not the smoothed
//! vertex normal the shading uses.
//!
//! **Cells** are parsed on first use (about 1.3 MB each; a pak read and a 341 node quadtree) and kept in a
//! small least recently used cache ([`TerrainQuery::DEFAULT_CACHE_CELLS`] cells, so a few tens of MB), shared by
//! all clones and threads; concurrent first queries of one cell load it once. A cell that does not exist
//! (outside the map) answers `None`, and so does a position with a negative coordinate.
//! [`prefetch`](TerrainQuery::prefetch) loads the surroundings of a position on a background thread so the
//! first step into a new cell does not stall a frame.
//!
//! Not covered (see `docs/aa-world.md`, "Ground queries"): terrain holes (`hasHoles` nodes, none are known to be
//! used in the 1.2 data), water (use `World::ocean_level` and the water volumes of
//! `placement::parse_water`), and collision with brushes and other objects.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use bevy::prelude::*;

use crate::coords::CELL_SIZE;
use crate::heightmap::CellHeights;
use crate::stream::WorldConfig;
use crate::world::cell_dir;

/// The ground at a point: its height and the unit normal of the surface there (game space, up = +Z).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ground {
    pub height: f32,
    pub normal: Vec3,
}

impl Ground {
    /// Angle between the surface and the horizontal plane (radians): 0 flat, pi / 2 a wall.
    pub fn slope(&self) -> f32 {
        self.normal.z.clamp(-1.0, 1.0).acos()
    }
}

/// Height / normal / slope of the terrain of one world under game positions. See the module documentation.
#[derive(Resource, Clone)]
pub struct TerrainQuery {
    inner: Arc<Inner>,
}

struct Inner {
    world: String,
    capacity: usize,
    cache: Mutex<Cache>,
}

#[derive(Default)]
struct Cache {
    tick: u64,
    cells: HashMap<(i32, i32), Slot>,
}

/// One cell of the cache. The `OnceLock` is filled outside the cache lock, so the other cells stay
/// reachable while one loads, and two threads asking for the same cell load it once.
struct Slot {
    cell: Arc<OnceLock<Option<Arc<CellHeights>>>>,
    used: u64,
}

impl TerrainQuery {
    /// Cells kept parsed by [`TerrainQuery::new`] (1.3 MB each).
    pub const DEFAULT_CACHE_CELLS: usize = 48;

    /// A query of `world` (the name of a directory of `worlds/`, e.g. `"main_world"`).
    pub fn new(world: &str) -> TerrainQuery {
        TerrainQuery::with_capacity(world, Self::DEFAULT_CACHE_CELLS)
    }

    /// Like [`TerrainQuery::new`] with at most `cells` (at least 1) parsed cells in memory.
    pub fn with_capacity(world: &str, cells: usize) -> TerrainQuery {
        TerrainQuery {
            inner: Arc::new(Inner { world: world.to_string(), capacity: cells.max(1), cache: Mutex::default() }),
        }
    }

    pub fn world(&self) -> &str {
        &self.inner.world
    }

    /// The parsed heightmap of cell `(cx, cy)` (loaded on first use); `None` when the cell does not exist or
    /// cannot be read.
    pub fn cell(&self, cx: i32, cy: i32) -> Option<Arc<CellHeights>> {
        if cx < 0 || cy < 0 {
            return None;
        }
        let slot = {
            let mut cache = self.inner.cache.lock().unwrap_or_else(PoisonError::into_inner);
            cache.tick += 1;
            let tick = cache.tick;
            let slot = cache.cells.entry((cx, cy)).or_insert_with(|| Slot { cell: Arc::default(), used: tick });
            slot.used = tick;
            let slot = slot.cell.clone();
            if cache.cells.len() > self.inner.capacity {
                let oldest = cache.cells.iter().filter(|(k, _)| **k != (cx, cy)).min_by_key(|(_, s)| s.used).map(|(k, _)| *k);
                if let Some(oldest) = oldest {
                    cache.cells.remove(&oldest);
                }
            }
            slot
        };
        slot.get_or_init(|| load(&self.inner.world, cx, cy)).clone()
    }

    /// The cell and the cell-local position of a game position.
    fn locate(&self, x: f32, y: f32) -> Option<(Arc<CellHeights>, f32, f32)> {
        if !(x.is_finite() && y.is_finite()) || x < 0.0 || y < 0.0 {
            return None;
        }
        let (cx, cy) = ((x / CELL_SIZE).floor() as i32, (y / CELL_SIZE).floor() as i32);
        let cell = self.cell(cx, cy)?;
        Some((cell, x - cx as f32 * CELL_SIZE, y - cy as f32 * CELL_SIZE))
    }

    /// Terrain height (metres) under game position `(x, y)`; `None` outside the map.
    pub fn height_at(&self, x: f32, y: f32) -> Option<f32> {
        let (cell, lx, ly) = self.locate(x, y)?;
        Some(cell.mesh_height(lx, ly))
    }

    /// Unit normal of the terrain surface (game space, up = +Z) under `(x, y)`.
    pub fn normal_at(&self, x: f32, y: f32) -> Option<Vec3> {
        self.ground_at(x, y).map(|g| g.normal)
    }

    /// Slope of the terrain under `(x, y)` in radians (0 flat, pi / 2 vertical).
    pub fn slope_at(&self, x: f32, y: f32) -> Option<f32> {
        self.ground_at(x, y).map(|g| g.slope())
    }

    /// Height and normal under `(x, y)` with one cell lookup.
    pub fn ground_at(&self, x: f32, y: f32) -> Option<Ground> {
        let (cell, lx, ly) = self.locate(x, y)?;
        let (height, n) = (cell.mesh_height(lx, ly), cell.mesh_normal(lx, ly));
        Some(Ground { height, normal: Vec3::from(n) })
    }

    /// Starts loading the cells within `radius` metres of `(x, y)` (closest first) on a background thread and
    /// returns immediately. Call it when the player enters a new cell.
    pub fn prefetch(&self, x: f32, y: f32, radius: f32) {
        let (c0x, c0y) = (((x - radius) / CELL_SIZE).floor() as i32, ((y - radius) / CELL_SIZE).floor() as i32);
        let (c1x, c1y) = (((x + radius) / CELL_SIZE).floor() as i32, ((y + radius) / CELL_SIZE).floor() as i32);
        let mut wanted: Vec<(f32, (i32, i32))> = Vec::new();
        for cy in c0y.max(0)..=c1y {
            for cx in c0x.max(0)..=c1x {
                let (min_x, min_y) = (cx as f32 * CELL_SIZE, cy as f32 * CELL_SIZE);
                let d = Vec2::new(x - x.clamp(min_x, min_x + CELL_SIZE), y - y.clamp(min_y, min_y + CELL_SIZE)).length();
                if d <= radius {
                    wanted.push((d, (cx, cy)));
                }
            }
        }
        if wanted.is_empty() {
            return;
        }
        wanted.sort_by(|a, b| a.0.total_cmp(&b.0));
        let me = self.clone();
        let _ = std::thread::Builder::new().name("terrain-prefetch".into()).spawn(move || {
            for (_, (cx, cy)) in wanted {
                me.cell(cx, cy);
            }
        });
    }
}

fn load(world: &str, cx: i32, cy: i32) -> Option<Arc<CellHeights>> {
    let path = format!("{}/client/terrain/heightmap.dat", cell_dir(world, cx, cy));
    let bytes = aa_pak::Pak::global().read(&path).ok()?;
    match CellHeights::parse(&bytes) {
        Ok(h) => Some(Arc::new(h)),
        Err(e) => {
            warn!("aa-world: terrain query: {path}: {e}");
            None
        }
    }
}

/// `AaWorldPlugin` inserts the world's [`TerrainQuery`] (unless the app has one already).
pub(crate) fn build(app: &mut App, cfg: &WorldConfig) {
    if !app.world().contains_resource::<TerrainQuery>() {
        app.insert_resource(TerrainQuery::new(&cfg.world));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coords::UNIT_SIZE;

    /// The Nuian spawn, a meadow at the pond in cell `(15, 15)`.
    const SPAWN: (f32, f32) = (15575.0, 15375.0);

    #[test]
    fn heights_match_the_grid() {
        if !crate::pak_available() {
            return;
        }
        let q = TerrainQuery::new("main_world");
        let cell = q.cell(15, 15).unwrap();
        // on a grid point the query is the sample itself, in the middle of a quad it stays between its corners
        let (ix, iy) = (260usize, 150usize);
        let (x, y) = (15.0 * CELL_SIZE + ix as f32 * UNIT_SIZE, 15.0 * CELL_SIZE + iy as f32 * UNIT_SIZE);
        assert!((q.height_at(x, y).unwrap() - cell.at(ix, iy)).abs() < 0.01);
        let corners = [cell.at(ix, iy), cell.at(ix + 1, iy), cell.at(ix, iy + 1), cell.at(ix + 1, iy + 1)];
        let mid = q.height_at(x + 1.0, y + 0.5).unwrap();
        let (lo, hi) = corners.iter().fold((f32::MAX, f32::MIN), |(a, b), &c| (a.min(c), b.max(c)));
        assert!(mid >= lo - 1e-3 && mid <= hi + 1e-3, "{mid} not in {lo}..{hi}");
        // the spawn area is dry land
        let spawn = q.height_at(SPAWN.0, SPAWN.1).unwrap();
        assert!(spawn > 100.0 && spawn < 400.0, "{spawn}");
    }

    #[test]
    fn follows_the_rendered_diagonal() {
        if !crate::pak_available() {
            return;
        }
        let q = TerrainQuery::new("main_world");
        let cell = q.cell(15, 14).unwrap();
        // the triangle (a, b, c) below the diagonal and (a, c, d) above it share the diagonal's heights
        let (ix, iy) = (100usize, 200usize);
        let (a, b, c, d) = (cell.at(ix, iy), cell.at(ix + 1, iy), cell.at(ix + 1, iy + 1), cell.at(ix, iy + 1));
        let origin = (15.0 * CELL_SIZE + ix as f32 * UNIT_SIZE, 14.0 * CELL_SIZE + iy as f32 * UNIT_SIZE);
        let on_diagonal = q.height_at(origin.0 + 1.0, origin.1 + 1.0).unwrap();
        assert!((on_diagonal - (a + c) / 2.0).abs() < 0.05);
        let below = q.height_at(origin.0 + 1.5, origin.1 + 0.5).unwrap();
        assert!((below - (a + 0.75 * (b - a) + 0.25 * (c - b))).abs() < 0.05);
        let above = q.height_at(origin.0 + 0.5, origin.1 + 1.5).unwrap();
        assert!((above - (a + 0.25 * (c - d) + 0.75 * (d - a))).abs() < 0.05);
    }

    #[test]
    fn continuous_across_cells() {
        if !crate::pak_available() {
            return;
        }
        let q = TerrainQuery::new("main_world");
        for y in [100.5f32, 400.0, 900.25] {
            let (west, east) = (q.height_at(15.0 * CELL_SIZE - 0.001, 15.0 * CELL_SIZE + y).unwrap(), q.height_at(15.0 * CELL_SIZE, 15.0 * CELL_SIZE + y).unwrap());
            assert!((west - east).abs() < 1.0, "x border at y {y}: {west} / {east}");
        }
        for x in [100.5f32, 400.0, 900.25] {
            let (south, north) = (q.height_at(15.0 * CELL_SIZE + x, 15.0 * CELL_SIZE - 0.001).unwrap(), q.height_at(15.0 * CELL_SIZE + x, 15.0 * CELL_SIZE).unwrap());
            assert!((south - north).abs() < 1.0, "y border at x {x}: {south} / {north}");
        }
    }

    #[test]
    fn normals_and_slopes() {
        if !crate::pak_available() {
            return;
        }
        let q = TerrainQuery::new("main_world");
        let g = q.ground_at(SPAWN.0, SPAWN.1).unwrap();
        assert!((g.normal.length() - 1.0).abs() < 1e-4 && g.normal.z > 0.0);
        assert!((g.slope() - q.slope_at(SPAWN.0, SPAWN.1).unwrap()).abs() < 1e-6);
        // the rock cliffs of cell (15, 14) have steep faces
        let steepest = (0..64)
            .flat_map(|i| (0..64).map(move |j| (15.0 * CELL_SIZE + i as f32 * 16.0, 14.0 * CELL_SIZE + j as f32 * 16.0)))
            .filter_map(|(x, y)| q.slope_at(x, y))
            .fold(0.0f32, f32::max);
        assert!(steepest.to_degrees() > 40.0, "{}", steepest.to_degrees());
        // a flat quad has the normal (0, 0, 1) and the slope 0
        let flat = Ground { height: 0.0, normal: Vec3::Z };
        assert_eq!(flat.slope(), 0.0);
    }

    #[test]
    fn outside_the_map_is_none() {
        if !crate::pak_available() {
            return;
        }
        let q = TerrainQuery::new("main_world");
        assert_eq!(q.height_at(-5.0, 100.0), None);
        assert_eq!(q.height_at(100.0, -5.0), None);
        assert_eq!(q.height_at(f32::NAN, 0.0), None);
        assert_eq!(q.height_at(900_000.0, 900_000.0), None);
        assert_eq!(TerrainQuery::new("no_such_world").height_at(SPAWN.0, SPAWN.1), None);
    }

    #[test]
    fn shared_between_threads_and_bounded() {
        if !crate::pak_available() {
            return;
        }
        // a cache of 2 cells: three cells still answer, the oldest is dropped and loads again
        let q = TerrainQuery::with_capacity("main_world", 2);
        let heights: Vec<f32> = (13..16).map(|c| q.height_at(c as f32 * CELL_SIZE + 500.0, 15.0 * CELL_SIZE + 500.0).unwrap()).collect();
        assert!(q.inner.cache.lock().unwrap().cells.len() <= 2);
        assert_eq!(q.height_at(13.0 * CELL_SIZE + 500.0, 15.0 * CELL_SIZE + 500.0), Some(heights[0]));
        // many threads, one answer, one parse of the cell
        let q = TerrainQuery::new("main_world");
        let answers: Vec<Option<f32>> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..6).map(|_| s.spawn(|| q.height_at(SPAWN.0, SPAWN.1))).collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert!(answers.iter().all(|a| a.is_some() && *a == answers[0]));
        assert_eq!(q.inner.cache.lock().unwrap().cells.len(), 1);
    }

    #[test]
    fn prefetch_loads_in_the_background() {
        if !crate::pak_available() {
            return;
        }
        let q = TerrainQuery::new("main_world");
        q.prefetch(SPAWN.0, SPAWN.1, 600.0);
        let loaded = |q: &TerrainQuery| q.inner.cache.lock().unwrap().cells.values().filter(|s| s.cell.get().is_some()).count();
        for _ in 0..200 {
            if loaded(&q) >= 4 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        assert!(loaded(&q) >= 4, "{}", loaded(&q));
    }
}
