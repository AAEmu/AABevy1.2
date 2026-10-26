//! Terrain sculpting on the streamed terrain.
//!
//! Edited cells are full copies of their [`CellHeights`]; the loaded [`TerrainPatch`]es of a cell get the copy
//! (`set_heights`) and the touched ones a rebuilt mesh, so the edit shows in the real terrain renderer with its
//! materials and LODs. Saving writes each edited cell's `heightmap.dat` into the pak overlay folder
//! ([`crate::heightmap_write`]); the editor and the client (`AA_PAK_OVERLAY`) then load it like the original.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use aa_world::coords::{CELL_SIZE, UNIT_SIZE};
use aa_world::heightmap::{CellHeights, GRID};
use aa_world::stream::TerrainPatch;
use aa_world::terrain::{PATCH_UNITS, PATCHES, build_patch_mesh};
use aa_world::{TerrainQuery, cry_to_bevy};
use bevy::prelude::*;

use crate::tools::Cursor;
use crate::{Editor, Mode, Request};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrushKind {
    Raise,
    Lower,
    Smooth,
    Flatten,
}

#[derive(Resource)]
pub struct Brush {
    pub kind: BrushKind,
    /// Metres.
    pub radius: f32,
    /// Metres per second at the centre (Smooth / Flatten: fraction per second).
    pub strength: f32,
    /// Flatten target height, taken when a stroke starts.
    flatten_to: Option<f32>,
}

/// The heights the editor works with: edited cells, else the pak's.
#[derive(Resource, Default)]
pub struct Heights {
    pub edited: HashMap<(i32, i32), Arc<CellHeights>>,
    /// Edited since the last save.
    pub unsaved: HashSet<(i32, i32)>,
    /// Cell copies from before each stroke.
    undo: Vec<HashMap<(i32, i32), Option<Arc<CellHeights>>>>,
    stroke: Option<HashMap<(i32, i32), Option<Arc<CellHeights>>>>,
}

impl Heights {
    pub fn cell(&self, query: &TerrainQuery, c: (i32, i32)) -> Option<Arc<CellHeights>> {
        self.edited.get(&c).cloned().or_else(|| query.cell(c.0, c.1))
    }

    /// Height at a cry-space point (bilinear, like the reader's `sample`).
    pub fn height_at(&self, query: &TerrainQuery, x: f32, y: f32) -> Option<f32> {
        if !(x.is_finite() && y.is_finite()) || x < 0.0 || y < 0.0 {
            return None;
        }
        let c = ((x / CELL_SIZE).floor() as i32, (y / CELL_SIZE).floor() as i32);
        let cell = self.cell(query, c)?;
        Some(cell.sample(x - c.0 as f32 * CELL_SIZE, y - c.1 as f32 * CELL_SIZE))
    }

    /// First terrain hit of a cry-space ray (`dir` normalised), within `max` metres.
    pub fn raycast(&self, query: &TerrainQuery, origin: Vec3, dir: Vec3, max: f32) -> Option<Vec3> {
        let above = |p: Vec3| self.height_at(query, p.x, p.y).map(|h| p.z - h);
        let mut prev_t = 0.0;
        if above(origin)? < 0.0 {
            return None;
        }
        let mut t = 0.0;
        let mut step = 1.0f32;
        while t < max {
            t += step;
            step = (step * 1.02).min(8.0);
            let Some(d) = above(origin + dir * t) else { continue };
            if d < 0.0 {
                // bisect between prev_t and t
                let (mut a, mut b) = (prev_t, t);
                for _ in 0..16 {
                    let m = (a + b) * 0.5;
                    if above(origin + dir * m).is_some_and(|d| d < 0.0) { b = m } else { a = m }
                }
                return Some(origin + dir * b);
            }
            prev_t = t;
        }
        None
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
}

pub fn plugin(app: &mut App) {
    let num = |name: &str, default: f32| crate::arg(name).and_then(|v| v.parse().ok()).unwrap_or(default);
    app.insert_resource(Brush { kind: BrushKind::Raise, radius: num("--brush-radius", 12.0), strength: num("--brush-strength", 4.0), flatten_to: None })
        .init_resource::<Heights>()
        .add_systems(Update, (sculpt, refresh_new_patches, draw_brush, requests).chain().after(crate::tools::update_cursor));
}

fn falloff(d: f32, radius: f32) -> f32 {
    let t = (1.0 - d / radius).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[allow(clippy::too_many_arguments)]
fn sculpt(
    time: Res<Time>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut editor: ResMut<Editor>,
    cursor: Res<Cursor>,
    query: Res<TerrainQuery>,
    mut brush: ResMut<Brush>,
    mut heights: ResMut<Heights>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut patches: Query<(&mut TerrainPatch, &mut Mesh3d)>,
    mut scripted: Local<u32>,
) {
    if editor.mode != Mode::Terrain || editor.playing {
        return;
    }
    // `--sculpt x,y`: hold the brush there for two seconds' worth of frames (unattended checks)
    let script = crate::arg("--sculpt").and_then(|v| {
        let v: Vec<f32> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        (v.len() == 2 && *scripted < 120).then(|| Vec2::new(v[0], v[1]))
    });
    let script_hit = script.and_then(|p| heights.height_at(&query, p.x, p.y).map(|z| p.extend(z)));
    if script_hit.is_some() {
        *scripted += 1;
    } else if *scripted == 120 && crate::flag("--sculpt-save") {
        *scripted += 1;
        editor.requests.push(Request::SaveTerrain);
    }
    if !mouse.pressed(MouseButton::Left) && script_hit.is_none() {
        if let Some(stroke) = heights.stroke.take() {
            heights.undo.push(stroke);
        }
        brush.flatten_to = None;
        return;
    }
    let Some(hit) = script_hit.or(cursor.ground.filter(|_| !cursor.over_ui)) else { return };
    if heights.stroke.is_none() && !mouse.just_pressed(MouseButton::Left) && script_hit.is_none() {
        return; // a press that started on the panels
    }
    if heights.stroke.is_none() {
        heights.stroke = Some(HashMap::new());
        brush.flatten_to = Some(hit.z);
    }
    let dt = if script_hit.is_some() { 1.0 / 60.0 } else { time.delta_secs().min(0.1) };
    let r = brush.radius;
    let (x0, x1) = (((hit.x - r) / CELL_SIZE).floor() as i32, ((hit.x + r) / CELL_SIZE).floor() as i32);
    let (y0, y1) = (((hit.y - r) / CELL_SIZE).floor() as i32, ((hit.y + r) / CELL_SIZE).floor() as i32);
    let mut touched = Vec::new();
    for cy in y0..=y1 {
        for cx in x0..=x1 {
            let c = (cx, cy);
            let Some(source) = heights.cell(&query, c) else { continue };
            let before = heights.edited.get(&c).cloned();
            if let Some(stroke) = heights.stroke.as_mut() {
                stroke.entry(c).or_insert(before);
            }
            let mut cell = (*source).clone();
            let origin = Vec2::new(cx as f32 * CELL_SIZE, cy as f32 * CELL_SIZE);
            let local = Vec2::new(hit.x, hit.y) - origin;
            let lo = ((local - r) / UNIT_SIZE).floor().max(Vec2::ZERO);
            let hi = ((local + r) / UNIT_SIZE).ceil().min(Vec2::splat((GRID - 1) as f32));
            if lo.x > hi.x || lo.y > hi.y {
                continue;
            }
            let (ix0, iy0, ix1, iy1) = (lo.x as usize, lo.y as usize, hi.x as usize, hi.y as usize);
            let at = |c: &CellHeights, ix: usize, iy: usize| c.heights[iy * GRID + ix];
            let mut changed = false;
            for iy in iy0..=iy1 {
                for ix in ix0..=ix1 {
                    let p = Vec2::new(ix as f32, iy as f32) * UNIT_SIZE;
                    let w = falloff(p.distance(local), r);
                    if w <= 0.0 {
                        continue;
                    }
                    let h = at(&source, ix, iy);
                    let new = match brush.kind {
                        BrushKind::Raise => h + brush.strength * w * dt,
                        BrushKind::Lower => h - brush.strength * w * dt,
                        BrushKind::Smooth => {
                            let (a, b) = (ix.saturating_sub(1), (ix + 1).min(GRID - 1));
                            let (c0, d) = (iy.saturating_sub(1), (iy + 1).min(GRID - 1));
                            let avg = (at(&source, a, iy) + at(&source, b, iy) + at(&source, ix, c0) + at(&source, ix, d)) * 0.25;
                            h + (avg - h) * (w * brush.strength * 0.5 * dt).min(1.0)
                        }
                        BrushKind::Flatten => {
                            let target = brush.flatten_to.unwrap_or(hit.z);
                            h + (target - h) * (w * brush.strength * 0.5 * dt).min(1.0)
                        }
                    };
                    if new != h {
                        cell.heights[iy * GRID + ix] = new;
                        changed = true;
                    }
                }
            }
            if changed {
                let (mut lo, mut hi) = (f32::MAX, f32::MIN);
                for h in &cell.heights {
                    lo = lo.min(*h);
                    hi = hi.max(*h);
                }
                cell.min_z = lo;
                cell.max_z = hi;
                let cell = Arc::new(cell);
                heights.edited.insert(c, cell.clone());
                heights.unsaved.insert(c);
                touched.push((c, cell, (ix0, iy0, ix1, iy1)));
            }
        }
    }
    for (c, cell, rect) in touched {
        apply(c, &cell, Some(rect), &mut meshes, &mut patches);
    }
}

/// Gives the loaded patches of cell `c` the heights `cell` and rebuilds the meshes of those overlapping the sample
/// rectangle `rect` (all when `None`).
fn apply(
    c: (i32, i32),
    cell: &Arc<CellHeights>,
    rect: Option<(usize, usize, usize, usize)>,
    meshes: &mut Assets<Mesh>,
    patches: &mut Query<(&mut TerrainPatch, &mut Mesh3d)>,
) {
    for (mut patch, mut mesh) in patches.iter_mut().filter(|(p, _)| p.cell == c) {
        let (px, py) = (patch.index % PATCHES, patch.index / PATCHES);
        let overlaps = rect.is_none_or(|(x0, y0, x1, y1)| {
            let (a, b) = (px * PATCH_UNITS, (px + 1) * PATCH_UNITS);
            let (d, e) = (py * PATCH_UNITS, (py + 1) * PATCH_UNITS);
            x1 >= a && x0 <= b && y1 >= d && y0 <= e
        });
        patch.set_heights(cell.clone());
        if overlaps {
            mesh.0 = meshes.add(build_patch_mesh(cell, px, py, patch.lod));
        }
    }
}

/// Patches streamed in again after an unload come with the pak's heights: give them the edits.
fn refresh_new_patches(
    heights: Res<Heights>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut queries: ParamSet<(Query<&TerrainPatch, Added<TerrainPatch>>, Query<(&mut TerrainPatch, &mut Mesh3d)>)>,
) {
    let cells: HashSet<(i32, i32)> = queries
        .p0()
        .iter()
        .filter(|p| heights.edited.get(&p.cell).is_some_and(|h| !Arc::ptr_eq(h, p.heights())))
        .map(|p| p.cell)
        .collect();
    for c in cells {
        apply(c, &heights.edited[&c], None, &mut meshes, &mut queries.p1());
    }
}

fn draw_brush(editor: Res<Editor>, cursor: Res<Cursor>, brush: Res<Brush>, heights: Res<Heights>, query: Res<TerrainQuery>, mut gizmos: Gizmos) {
    if editor.mode != Mode::Terrain || editor.playing {
        return;
    }
    let Some(hit) = cursor.ground else { return };
    let color = match brush.kind {
        BrushKind::Raise => Color::srgb(0.3, 1.0, 0.4),
        BrushKind::Lower => Color::srgb(1.0, 0.4, 0.3),
        BrushKind::Smooth => Color::srgb(0.4, 0.7, 1.0),
        BrushKind::Flatten => Color::srgb(1.0, 0.9, 0.3),
    };
    for (radius, alpha) in [(brush.radius, 1.0), (brush.radius * 0.5, 0.45)] {
        let n = 64;
        let pts: Vec<Vec3> = (0..=n)
            .map(|i| {
                let a = i as f32 / n as f32 * std::f32::consts::TAU;
                let (x, y) = (hit.x + a.cos() * radius, hit.y + a.sin() * radius);
                let z = heights.height_at(&query, x, y).unwrap_or(hit.z) + 0.25;
                cry_to_bevy(Vec3::new(x, y, z))
            })
            .collect();
        gizmos.linestrip(pts, color.with_alpha(alpha));
    }
}

fn requests(
    mut editor: ResMut<Editor>,
    mut heights: ResMut<Heights>,
    mut meshes: ResMut<Assets<Mesh>>,
    query: Res<TerrainQuery>,
    mut patches: Query<(&mut TerrainPatch, &mut Mesh3d)>,
) {
    if let Some(i) = editor.requests.iter().position(|r| matches!(r, Request::UndoTerrain)) {
        editor.requests.remove(i);
        if let Some(stroke) = heights.undo.pop() {
            for (c, before) in stroke {
                let cell = match before {
                    Some(b) => {
                        heights.edited.insert(c, b.clone());
                        b
                    }
                    // back to the heights the editor started with (kept as an edit, so a save writes them)
                    None => match query.cell(c.0, c.1) {
                        Some(orig) => {
                            heights.edited.insert(c, orig.clone());
                            orig
                        }
                        None => continue,
                    },
                };
                heights.unsaved.insert(c);
                apply(c, &cell, None, &mut meshes, &mut patches);
            }
            editor.status("terrain: stroke undone");
        }
    }
    if let Some(i) = editor.requests.iter().position(|r| matches!(r, Request::SaveTerrain)) {
        editor.requests.remove(i);
        let result = save(&editor, &mut heights);
        match result {
            Ok(0) => editor.status("terrain: nothing to save"),
            Ok(n) => {
                let msg = format!("terrain: {n} cell(s) saved to {}", editor.paths.overlay.display());
                editor.status(msg)
            }
            Err(e) => editor.status(format!("terrain save failed: {e:#}")),
        }
    }
}

fn save(editor: &Editor, heights: &mut Heights) -> anyhow::Result<usize> {
    let mut saved = 0;
    let cells: Vec<(i32, i32)> = heights.unsaved.iter().copied().collect();
    for c in cells {
        let rel = format!("{}/client/terrain/heightmap.dat", aa_world::world::cell_dir(&editor.world, c.0, c.1));
        // the template: the overlay's earlier save or the original
        let original = aa_pak::Pak::global().read(&rel)?;
        let cell = match heights.edited.get(&c) {
            Some(cell) => cell.clone(),
            None => continue,
        };
        let bytes = crate::heightmap_write::write(&original, &cell)?;
        let path = editor.paths.overlay.join(&rel);
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(&path, bytes)?;
        heights.unsaved.remove(&c);
        saved += 1;
    }
    Ok(saved)
}
