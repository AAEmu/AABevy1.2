//! Water queries for gameplay code: is there water under a game position, at which height, and what kind.
//!
//! [`WaterQuery`] is the water counterpart of [`TerrainQuery`]: a cheap clonable handle (`Arc` inside, `&self` methods,
//! `Send + Sync`) and a Bevy [`Resource`] (`AaWorldPlugin` inserts one next to the terrain query), independent of the
//! renderer and of the cells being spawned, so footsteps, splashes and swimming can ask before the first water mesh exists.
//!
//! ```no_run
//! # use aa_world::{WaterKind, WaterQuery};
//! # use bevy::prelude::Vec3;
//! let water = WaterQuery::new("main_world");
//! // game coordinates: metres, x east, y north, z up (the Nuian spawn pond)
//! if let Some((surface, kind)) = water.water_at(Vec3::new(15_577.3, 15_379.9, 126.25)) {
//!     println!("{kind:?} surface at {surface} m");
//! }
//! ```
//!
//! **Bodies**: the water volumes of a cell (`object.dat`, type 11: ponds and lakes, rivers) are polygons at a surface height
//! with a depth; the sea is the plane at the world's `oceanLevel` (100 m in `main_world`, none in `login2`) wherever the
//! terrain lies below it (a cell without terrain inside the map is open sea). A volume applies to a position when its
//! outline contains `(x, y)` and the position is not below the volume's floor (`surface - depth`), so a cave under a lake is
//! dry. When volumes overlap the highest surface wins; a volume beats the sea. The position may be above the surface (a
//! bridge, a jump): the water below is still answered, so compare `pos.z` with the height to know whether the feet are wet.
//!
//! **Cells** are parsed on first use (the `object.dat` of the cell, a few ms) and kept in a small least recently used cache
//! shared by all clones and threads, like [`TerrainQuery`]. Positions outside the map answer `None`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use bevy::prelude::*;

use crate::coords::CELL_SIZE;
use crate::placement::{self, Water};
use crate::terrain_query::TerrainQuery;
use crate::world::{World, cell_dir};

/// The kind of a body of water.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WaterKind {
    /// The ocean plane.
    Sea,
    /// A pond or a lake (a volume of kind 2).
    Lake,
    /// A river (a volume of kind 3).
    River,
}

/// A body of water at a position: the height of its surface, its kind and (volumes only) the fog of the volume.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaterBody {
    pub height: f32,
    pub kind: WaterKind,
    /// The volume's own fog colour (linear) and density; zero for the sea (its colour is the zone's).
    pub fog_color: Vec3,
    pub fog_density: f32,
}

/// One volume of a cell: the outline in cell-local game coordinates and its heights.
struct Volume {
    surface: f32,
    floor: f32,
    kind: WaterKind,
    fog_color: Vec3,
    fog_density: f32,
    ring: Vec<Vec2>,
    min: Vec2,
    max: Vec2,
}

impl Volume {
    fn new(w: &Water) -> Option<Volume> {
        let ring = crate::water::ring(&w.points);
        let first = *ring.first()?;
        let (min, max) = ring.iter().fold((first, first), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
        let kind = if w.kind & 3 == 3 { WaterKind::River } else { WaterKind::Lake };
        (ring.len() >= 3).then_some(Volume {
            surface: w.surface,
            floor: w.surface - w.depth,
            kind,
            fog_color: Vec3::from(w.fog_color),
            fog_density: w.fog_density,
            ring,
            min,
            max,
        })
    }

    /// Even-odd test of a cell-local point against the outline.
    fn contains(&self, p: Vec2) -> bool {
        if p.x < self.min.x || p.x > self.max.x || p.y < self.min.y || p.y > self.max.y {
            return false;
        }
        let mut inside = false;
        let mut j = self.ring.len() - 1;
        for i in 0..self.ring.len() {
            let (a, b) = (self.ring[i], self.ring[j]);
            if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
                inside = !inside;
            }
            j = i;
        }
        inside
    }
}

/// Water surface height and kind under game positions. See the module documentation.
#[derive(Resource, Clone)]
pub struct WaterQuery {
    inner: Arc<Inner>,
}

struct Inner {
    world: String,
    /// `None` for a world without a sea.
    ocean_level: Option<f32>,
    cells_x: i32,
    cells_y: i32,
    terrain: TerrainQuery,
    capacity: usize,
    cache: Mutex<Cache>,
}

#[derive(Default)]
struct Cache {
    tick: u64,
    cells: HashMap<(i32, i32), Slot>,
}

/// One cell of the cache; the `OnceLock` is filled outside the cache lock (see `TerrainQuery`).
struct Slot {
    cell: Arc<OnceLock<Arc<Vec<Volume>>>>,
    used: u64,
}

impl WaterQuery {
    /// Cells kept parsed by [`WaterQuery::new`].
    pub const DEFAULT_CACHE_CELLS: usize = 64;

    /// A query of `world` (a directory of `worlds/`, e.g. `"main_world"`) with its own terrain query for the sea.
    pub fn new(world: &str) -> WaterQuery {
        WaterQuery::with_terrain(world, TerrainQuery::new(world))
    }

    /// Like [`WaterQuery::new`], sharing the terrain cells of `terrain` (a clone of the app's [`TerrainQuery`]).
    pub fn with_terrain(world: &str, terrain: TerrainQuery) -> WaterQuery {
        let info = World::load(world).unwrap_or_default();
        let ocean_level = (info.ocean_level > 0.0).then_some(info.ocean_level);
        WaterQuery {
            inner: Arc::new(Inner {
                world: world.to_string(),
                ocean_level,
                cells_x: info.cells_x,
                cells_y: info.cells_y,
                terrain,
                capacity: Self::DEFAULT_CACHE_CELLS,
                cache: Mutex::default(),
            }),
        }
    }

    pub fn world(&self) -> &str {
        &self.inner.world
    }

    /// The water volumes of cell `(cx, cy)`, loaded on first use.
    fn cell(&self, cx: i32, cy: i32) -> Arc<Vec<Volume>> {
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
        slot.get_or_init(|| Arc::new(load(&self.inner.world, cx, cy))).clone()
    }

    /// The surface height (metres) and kind of the water under `pos` (game coordinates, z up), `None` on dry land or
    /// outside the map. See the module documentation for which volume wins and how the sea is decided.
    pub fn water_at(&self, pos: Vec3) -> Option<(f32, WaterKind)> {
        self.body_at(pos).map(|b| (b.height, b.kind))
    }

    /// Like [`WaterQuery::water_at`] with the volume's fog (what the renderer tints the view under water with).
    pub fn body_at(&self, pos: Vec3) -> Option<WaterBody> {
        if !pos.is_finite() || pos.x < 0.0 || pos.y < 0.0 {
            return None;
        }
        let (cx, cy) = ((pos.x / CELL_SIZE).floor() as i32, (pos.y / CELL_SIZE).floor() as i32);
        if cx >= self.inner.cells_x.max(1) || cy >= self.inner.cells_y.max(1) {
            return None;
        }
        let local = Vec2::new(pos.x - cx as f32 * CELL_SIZE, pos.y - cy as f32 * CELL_SIZE);
        let best = self
            .cell(cx, cy)
            .iter()
            .filter(|v| pos.z >= v.floor && v.contains(local))
            .max_by(|a, b| a.surface.total_cmp(&b.surface))
            .map(|v| WaterBody { height: v.surface, kind: v.kind, fog_color: v.fog_color, fog_density: v.fog_density });
        best.or_else(|| self.sea_at(pos.x, pos.y))
    }

    /// The sea plane where the terrain is below it (or missing: open sea).
    fn sea_at(&self, x: f32, y: f32) -> Option<WaterBody> {
        let level = self.inner.ocean_level?;
        match self.inner.terrain.height_at(x, y) {
            Some(ground) if ground >= level => None,
            _ => Some(WaterBody { height: level, kind: WaterKind::Sea, fog_color: Vec3::ZERO, fog_density: 0.0 }),
        }
    }
}

fn load(world: &str, cx: i32, cy: i32) -> Vec<Volume> {
    let path = format!("{}/client/object.dat", cell_dir(world, cx, cy));
    let Ok(bytes) = aa_pak::Pak::global().read(&path) else { return Vec::new() };
    match placement::parse_object_dat(&bytes) {
        Ok(objects) => objects.water.iter().filter_map(Volume::new).collect(),
        Err(e) => {
            warn!("aa-world: water query: {path}: {e}");
            Vec::new()
        }
    }
}

/// `AaWorldPlugin` inserts the world's [`WaterQuery`] (unless the app has one already), sharing the terrain query.
pub(crate) fn build(app: &mut App, world: &str) {
    if !app.world().contains_resource::<WaterQuery>() {
        let terrain = app.world().get_resource::<TerrainQuery>().cloned().unwrap_or_else(|| TerrainQuery::new(world));
        app.insert_resource(WaterQuery::with_terrain(world, terrain));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outline_contains() {
        let w = Water {
            bbox: [0.0; 6],
            surface: 10.0,
            depth: 2.0,
            uv_scale: [0.2, 0.2],
            river_u: [0.0,1.0],
            points: vec![Vec3::new(0.0, 0.0, 10.0), Vec3::new(10.0, 0.0, 10.0), Vec3::new(10.0, 10.0, 10.0), Vec3::new(5.0, 4.0, 10.0), Vec3::new(0.0, 10.0, 10.0)],
            kind: 3,
            material: 0,
            fog_density: 0.1,
            fog_color: [0.0; 3],
        };
        let v = Volume::new(&w).unwrap();
        assert_eq!((v.kind, v.surface, v.floor), (WaterKind::River, 10.0, 8.0));
        assert!(v.contains(Vec2::new(2.0, 2.0)) && v.contains(Vec2::new(8.0, 2.0)) && v.contains(Vec2::new(0.5, 9.0)));
        // the notch of the outline and the outside
        assert!(!v.contains(Vec2::new(5.0, 8.0)) && !v.contains(Vec2::new(8.0, 8.0)) && !v.contains(Vec2::new(-1.0, 2.0)) && !v.contains(Vec2::new(11.0, 5.0)));
    }

    #[test]
    fn ponds_rivers_and_the_sea() {
        if !crate::pak_available() {
            return;
        }
        let q = WaterQuery::new("main_world");
        // the Nuian spawn pond: a lake volume at 127 m over a flat bed at 126.25 m
        assert_eq!(q.water_at(Vec3::new(15_577.3, 15_379.9, 126.25)), Some((127.0, WaterKind::Lake)));
        // above the surface (a jump) still answers, below the floor of the volume (a cave) does not
        assert_eq!(q.water_at(Vec3::new(15_577.3, 15_379.9, 130.0)), Some((127.0, WaterKind::Lake)));
        assert_eq!(q.water_at(Vec3::new(15_577.3, 15_379.9, 100.0)), None);
        // the volume's fog (the renderer's under water tint): lake_tank, density 0.6
        let pond = q.body_at(Vec3::new(15_577.3, 15_379.9, 126.25)).unwrap();
        assert!((pond.fog_density - 0.6).abs() < 1e-5 && pond.fog_color.y > pond.fog_color.x, "{pond:?}");
        // the meadow next to it is dry
        assert_eq!(q.water_at(Vec3::new(15_540.0, 15_330.0, 130.5)), None);
        // a river in cell (9, 12) and a mountain lake in cell (27, 6)
        assert_eq!(q.water_at(Vec3::new(9_566.0, 12_588.0, 163.6)).map(|w| w.1), Some(WaterKind::River));
        assert_eq!(q.water_at(Vec3::new(28_498.0, 6_844.0, 554.0)).map(|w| w.1), Some(WaterKind::Lake));
        // the open sea at the corner of the map (no terrain cell), and nothing outside the map
        assert_eq!(q.water_at(Vec3::new(100.0, 100.0, 90.0)), Some((100.0, WaterKind::Sea)));
        assert_eq!(q.water_at(Vec3::new(-5.0, 10.0, 100.0)), None);
        assert_eq!(q.water_at(Vec3::new(900_000.0, 10.0, 100.0)), None);
        assert_eq!(q.water_at(Vec3::new(f32::NAN, 10.0, 100.0)), None);
        // a world without a sea
        assert_eq!(WaterQuery::new("login2").water_at(Vec3::new(100.0, 100.0, 0.0)), None);
    }

    #[test]
    fn shared_between_threads() {
        if !crate::pak_available() {
            return;
        }
        let q = WaterQuery::new("main_world");
        let answers: Vec<Option<(f32, WaterKind)>> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..6).map(|_| s.spawn(|| q.water_at(Vec3::new(15_577.3, 15_379.9, 126.25)))).collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert!(answers.iter().all(|a| *a == Some((127.0, WaterKind::Lake))));
    }
}
