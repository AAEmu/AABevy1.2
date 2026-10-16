//! Ground height under a game position, from the same `heightmap.dat` files aa-world renders.
//!
//! aa-world streams the terrain meshes but keeps the height samples private, so the client keeps
//! its own small cache of parsed cells (one 513 x 513 grid per 1024 m cell, loaded on first use).

use std::collections::HashMap;
use std::sync::Arc;

use aa_world::heightmap::CellHeights;
use aa_world::world::cell_dir;
use bevy::prelude::*;

pub const WORLD: &str = "main_world";

#[derive(Resource, Default)]
pub struct Terrain {
    cells: HashMap<(i32, i32), Option<Arc<CellHeights>>>,
}

impl Terrain {
    /// Terrain height at a game position (metres, x east, y north); `None` where the world has no
    /// terrain (outside the map) or the cell cannot be read.
    pub fn height(&mut self, x: f32, y: f32) -> Option<f32> {
        let (cx, cy) = ((x / aa_world::CELL_SIZE).floor() as i32, (y / aa_world::CELL_SIZE).floor() as i32);
        let cell = self.cells.entry((cx, cy)).or_insert_with(|| load_cell(cx, cy));
        let cell = cell.as_ref()?;
        Some(cell.sample(x - cx as f32 * aa_world::CELL_SIZE, y - cy as f32 * aa_world::CELL_SIZE))
    }

    /// Height, or `fallback` where there is no terrain.
    pub fn height_or(&mut self, x: f32, y: f32, fallback: f32) -> f32 {
        self.height(x, y).unwrap_or(fallback)
    }
}

fn load_cell(cx: i32, cy: i32) -> Option<Arc<CellHeights>> {
    if cx < 0 || cy < 0 {
        return None;
    }
    let path = format!("{}/client/terrain/heightmap.dat", cell_dir(WORLD, cx, cy));
    let bytes = aa_pak::Pak::global().read(&path).ok()?;
    match CellHeights::parse(&bytes) {
        Ok(h) => Some(Arc::new(h)),
        Err(e) => {
            warn!("terrain: {path}: {e}");
            None
        }
    }
}
