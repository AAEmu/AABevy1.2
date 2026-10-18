//! `cells/XXX_YYY/client/terrain/heightmap.dat`: the cell's terrain heights.
//!
//! Layout (little endian), after the server project's `Hmap.cs` port plus our own checks:
//!
//! ```text
//! u8 version, u8 dummy, u8 flags, u8 flags2
//! i32 chunkSize, i32 heightmapSizeInUnits (4096), i32 unitSizeInMeters (2),
//! i32 sectorSizeInMeters (64), i32 sectorsTableSize (128), f32 zRatio, f32 oceanLevel
//! if version >= 24: u16[64]
//! nodes until the read position == chunkSize: a depth first quadtree (1+4+16+64+256 = 341 nodes);
//! node:
//!   u8 version, dummy, flags, flags2; f32 box[6] (min xyz, max xyz); u8 hasHoles;
//!   f32 offset, f32 range; i32 size; i32 unkCount; u16 data[size*size];
//!   i32, f32[4]; skip unkCount bytes; skip 36 bytes
//! ```
//!
//! Only the 256 leaf nodes (64 m sectors) have data, `size` = 33 (sometimes 17 / 5 for flat
//! sectors: resampled here). `data[x * size + y]` is x major; sector edges are shared with the
//! neighbours. Native node-v5 samples use five surface bits (31 means a hole), and eleven height
//! bits: `offset + (raw & 0xFFE0) * range` for
//! node versions < 7 (all of 1.2's data); newer nodes use 5 cm integer steps.
//!
//! The node boxes are in the coordinates of the engine's 8x8-cell window (a cell sits at some
//! multiple of 1024), not in cell-local metres: positions are taken relative to the root node's box
//! minimum.

use crate::coords::{CELL_UNITS, UNIT_SIZE};
use crate::reader::{Reader, ReadError};

/// Samples per cell side: 512 units plus the shared far edge.
pub const GRID: usize = CELL_UNITS + 1;

/// All heights of one cell.
#[derive(Clone, Debug)]
pub struct CellHeights {
    /// `heights[iy * GRID + ix]`, `ix` along cry X, `iy` along cry Y, 2 m apart, cell-local.
    pub heights: Vec<f32>,
    /// Native surface type (0..31; 31 is a hole) per sample, same indexing.
    pub surface: Vec<u8>,
    /// Native `hasHoles != 0` gate of each 64 m sector, indexed `sy * 16 + sx`.
    pub hole_sectors: [bool; 256],
    /// Ocean level stored in the file header (metres).
    pub ocean_level: f32,
    pub min_z: f32,
    pub max_z: f32,
    /// How many of the 256 sectors had data.
    pub sectors: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum HeightmapError {
    #[error("truncated heightmap")]
    Truncated,
    #[error("invalid heightmap: {0}")]
    Invalid(String),
}

impl From<ReadError> for HeightmapError {
    fn from(_: ReadError) -> Self {
        HeightmapError::Truncated
    }
}

struct Node {
    version: u8,
    min: [f32; 3],
    offset: f32,
    range: f32,
    size: usize,
    has_holes: bool,
    data: Vec<u16>,
}

impl CellHeights {
    pub fn parse(bytes: &[u8]) -> Result<CellHeights, HeightmapError> {
        let mut r = Reader::new(bytes);
        let version = r.u8()?;
        r.skip(3)?;
        let chunk_size = r.i32()?;
        let _units = r.i32()?;
        let unit_m = r.i32()?;
        let sector_m = r.i32()?;
        let _sectors_table = r.i32()?;
        let _z_ratio = r.f32()?;
        let ocean_level = r.f32()?;
        if unit_m != 2 || sector_m != 64 {
            return Err(HeightmapError::Invalid(format!("unit {unit_m} m, sector {sector_m} m")));
        }
        if version >= 24 {
            r.skip(128)?;
        }
        let mut nodes = Vec::with_capacity(341);
        while r.pos() != chunk_size as usize {
            if r.pos() > chunk_size as usize {
                return Err(HeightmapError::Invalid("node overruns chunk".into()));
            }
            nodes.push(read_node(&mut r)?);
        }
        let root = nodes.first().ok_or_else(|| HeightmapError::Invalid("no nodes".into()))?;
        let (ox, oy) = (root.min[0], root.min[1]);

        let mut heights = vec![0.0f32; GRID * GRID];
        let mut surface = vec![1u8; GRID * GRID];
        let mut hole_sectors = [false; 256];
        let mut sectors = 0;
        for n in nodes.iter().filter(|n| n.size > 0) {
            let sx = ((n.min[0] - ox) / UNIT_SIZE).round() as isize;
            let sy = ((n.min[1] - oy) / UNIT_SIZE).round() as isize;
            if sx < 0 || sy < 0 || sx as usize + 32 > CELL_UNITS || sy as usize + 32 > CELL_UNITS {
                continue;
            }
            sectors += 1;
            hole_sectors[sy as usize / 32 * 16 + sx as usize / 32] = n.has_holes;
            for y in 0..33usize {
                for x in 0..33usize {
                    let (z, raw) = sample_node(n, x, y);
                    let i = (sy as usize + y) * GRID + sx as usize + x;
                    heights[i] = z;
                    // A boundary belongs to the sector on its positive side. Shared far-edge
                    // samples may carry a different SID and must not overwrite that owner.
                    if (x < 32 || sx as usize + x == CELL_UNITS)
                        && (y < 32 || sy as usize + y == CELL_UNITS)
                    {
                        surface[i] = (raw & if n.version < 7 { 0x1F } else { 0xF }) as u8;
                    }
                }
            }
        }
        let (min_z, max_z) = heights.iter().fold((f32::MAX, f32::MIN), |(a, b), &h| (a.min(h), b.max(h)));
        Ok(CellHeights { heights, surface, hole_sectors, ocean_level, min_z, max_z, sectors })
    }

    #[inline]
    pub fn at(&self, ix: usize, iy: usize) -> f32 {
        self.heights[iy.min(GRID - 1) * GRID + ix.min(GRID - 1)]
    }

    /// Native point hole query. A sample owns the 2 m square on its positive X/Y side.
    /// Height queries remain defined over holes, as in the original engine.
    pub fn is_hole(&self, x: f32, y: f32) -> bool {
        let ix = ((x / UNIT_SIZE).floor() as usize).min(CELL_UNITS - 1);
        let iy = ((y / UNIT_SIZE).floor() as usize).min(CELL_UNITS - 1);
        self.surface[iy * GRID + ix] == 31
    }

    /// Native index-generation gate: test the LOD quad centre if its sector has holes.
    pub fn quad_is_hole(&self, ix: usize, iy: usize, step: usize) -> bool {
        let x = (ix + step / 2).min(CELL_UNITS - 1);
        let y = (iy + step / 2).min(CELL_UNITS - 1);
        self.hole_sectors[y / 32 * 16 + x / 32] && self.surface[y * GRID + x] == 31
    }

    /// Bilinear terrain height at a cell-local position in metres (clamped to the cell).
    pub fn sample(&self, x: f32, y: f32) -> f32 {
        let fx = (x / UNIT_SIZE).clamp(0.0, CELL_UNITS as f32);
        let fy = (y / UNIT_SIZE).clamp(0.0, CELL_UNITS as f32);
        let (ix, iy) = (fx.floor() as usize, fy.floor() as usize);
        let (tx, ty) = (fx - ix as f32, fy - iy as f32);
        let a = self.at(ix, iy);
        let b = self.at(ix + 1, iy);
        let c = self.at(ix, iy + 1);
        let d = self.at(ix + 1, iy + 1);
        (a + (b - a) * tx) * (1.0 - ty) + (c + (d - c) * tx) * ty
    }

    /// Native finest-grid triangle (south east to north west diagonal), with height and slopes.
    fn facet(&self, x: f32, y: f32) -> (f32, f32, f32) {
        let fx = (x / UNIT_SIZE).clamp(0.0, CELL_UNITS as f32);
        let fy = (y / UNIT_SIZE).clamp(0.0, CELL_UNITS as f32);
        let (ix, iy) = ((fx.floor() as usize).min(CELL_UNITS - 1), (fy.floor() as usize).min(CELL_UNITS - 1));
        let (tx, ty) = (fx - ix as f32, fy - iy as f32);
        let (a, b, c, d) = (self.at(ix, iy), self.at(ix + 1, iy), self.at(ix + 1, iy + 1), self.at(ix, iy + 1));
        if tx + ty <= 1.0 {
            (a + (b - a) * tx + (d - a) * ty, (b - a) / UNIT_SIZE, (d - a) / UNIT_SIZE)
        } else {
            (c + (d - c) * (1.0 - tx) + (b - c) * (1.0 - ty), (c - d) / UNIT_SIZE, (c - b) / UNIT_SIZE)
        }
    }

    /// Height of the rendered terrain surface (the finest mesh, linear over its triangles) at a cell-local
    /// position in metres (clamped to the cell). Within a few centimetres of [`CellHeights::sample`]'s bilinear
    /// value; this one is exactly the surface that is drawn.
    pub fn mesh_height(&self, x: f32, y: f32) -> f32 {
        self.facet(x, y).0
    }

    /// Unit face normal (cry space, up = +Z) of the finest mesh's triangle at a cell-local position.
    pub fn mesh_normal(&self, x: f32, y: f32) -> [f32; 3] {
        let (_, dx, dy) = self.facet(x, y);
        let len = (dx * dx + dy * dy + 1.0).sqrt();
        [-dx / len, -dy / len, 1.0 / len]
    }

    /// Surface normal (cry space, up = +Z) at sample `(ix, iy)`.
    pub fn normal(&self, ix: usize, iy: usize) -> [f32; 3] {
        self.normal_step(ix, iy, 1)
    }

    /// Normal from central differences `step` samples apart (smoother for coarse levels of detail).
    pub fn normal_step(&self, ix: usize, iy: usize, step: usize) -> [f32; 3] {
        let (x0, x1) = (ix.saturating_sub(step), (ix + step).min(GRID - 1));
        let (y0, y1) = (iy.saturating_sub(step), (iy + step).min(GRID - 1));
        let dx = self.at(x1, iy) - self.at(x0, iy);
        let dy = self.at(ix, y1) - self.at(ix, y0);
        let nx = -dx / ((x1 - x0) as f32 * UNIT_SIZE);
        let ny = -dy / ((y1 - y0) as f32 * UNIT_SIZE);
        let len = (nx * nx + ny * ny + 1.0).sqrt();
        [nx / len, ny / len, 1.0 / len]
    }
}

fn read_node(r: &mut Reader) -> Result<Node, HeightmapError> {
    let version = r.u8()?;
    r.skip(3)?;
    let min = [r.f32()?, r.f32()?, r.f32()?];
    r.skip(12)?; // box max
    let has_holes = r.u8()? != 0;
    let offset = r.f32()?;
    let range = r.f32()?;
    let size = r.i32()?;
    let unk = r.i32()?;
    if !(0..=1024).contains(&size) || size == 1 {
        return Err(HeightmapError::Invalid(format!("node size {size}")));
    }
    let size = size as usize;
    let mut data = Vec::with_capacity(size * size);
    for _ in 0..size * size {
        data.push(r.u16()?);
    }
    r.skip(4 + 16)?;
    if unk >= 1 {
        r.skip_clamped(unk as usize);
    }
    r.skip_clamped(36);
    Ok(Node { version, min, offset, range, size, has_holes, data })
}

/// Height and raw sample for sector-local point `(x, y)` (0..=32) of a node. Nodes smaller than 33 x 33
/// (flat or smooth sectors; sample spacing `64 / (size - 1)` m) are interpolated bilinearly.
fn sample_node(n: &Node, x: usize, y: usize) -> (f32, u16) {
    if n.size == 33 {
        let raw = n.data[x * 33 + y];
        return (node_height(n, raw), raw);
    }
    let s = (n.size - 1) as f32 / 32.0;
    let (fx, fy) = (x as f32 * s, y as f32 * s);
    let (ix, iy) = ((fx.floor() as usize).min(n.size - 2), (fy.floor() as usize).min(n.size - 2));
    let (tx, ty) = (fx - ix as f32, fy - iy as f32);
    let h = |a: usize, b: usize| node_height(n, n.data[a * n.size + b]);
    let top = h(ix, iy) * (1.0 - tx) + h(ix + 1, iy) * tx;
    let bottom = h(ix, iy + 1) * (1.0 - tx) + h(ix + 1, iy + 1) * tx;
    // Native surface lookup shifts by the sample spacing; it does not round to nearest.
    let near = n.data[(fx.floor() as usize).min(n.size - 1) * n.size + (fy.floor() as usize).min(n.size - 1)];
    (top * (1.0 - ty) + bottom * ty, near)
}

fn node_height(n: &Node, raw: u16) -> f32 {
    if n.version < 7 {
        // 300C8BA1..300C8BAC keeps the product and sum in x87 precision before
        // storing the primary sample. Preserve that single final f32 rounding.
        (n.offset as f64 + (raw & 0xFFE0) as f64 * n.range as f64) as f32
    } else {
        let f_max = n.offset + 0xFFF0 as f32 * n.range;
        let i_offset = (n.offset * 20.0) as i32;
        let i_range = ((f_max - n.offset) * 20.0) as i32;
        let step = if i_range > 0 { (i_range as i64 + 4095 - 1) / 4095 } else { 1 };
        0.05 * i_offset as f32 + ((raw >> 4) as i64 * step) as f32 * 0.05
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_v5_loader_height_surface_and_hole_oracle() {
        if !crate::pak_available() { return; }
        let bytes = aa_pak::Pak::global().read("worlds/main_world/cells/020_008/client/terrain/heightmap.dat").unwrap();
        let h = CellHeights::parse(&bytes).unwrap();
        // Executed original 300DB920 loader, 300C8B60 heights and full 300CAB30 hole query.
        // Leaf file offset 536543, native box minimum (4800,5952), cell-local (704,832).
        let mut signature = 0xcbf29ce484222325_u64;
        let mut holes = 0;
        for x in 0..32 {
            for y in 0..32 {
                let (ix, iy) = (352 + x, 416 + y);
                let sid = h.surface[iy * GRID + ix];
                signature = (signature ^ (h.at(ix, iy).to_bits() as u64 | ((sid as u64) << 32)))
                    .wrapping_mul(0x100000001b3);
                holes += usize::from(h.is_hole(ix as f32 * 2.0 + 0.5, iy as f32 * 2.0 + 0.5));
            }
        }
        assert_eq!(signature, 0x20d861c1961fc442);
        assert_eq!(holes, 22);
        // This authored hole lies immediately above a sector boundary. The positive-side
        // sector owns the SID; the previous sector's shared far-edge sample must not replace it.
        assert!(h.is_hole(731.0, 833.0));
        assert!(!h.is_hole(731.0, 831.99));
    }

    #[test]
    fn native_coarse_surface_uses_shift_not_nearest_sample() {
        let n = Node {
            version: 5, min: [0.0; 3], offset: 10.0, range: 0.25,
            size: 17, has_holes: true,
            data: (0..17 * 17).map(|i| 0x4000 | (i % 31) as u16).collect(),
        };
        assert_eq!(sample_node(&n, 1, 1).1 & 31, 0);
        assert_eq!(sample_node(&n, 2, 2).1 & 31, 18);
        assert_eq!(sample_node(&n, 3, 3).1 & 31, 18);
        for sid in 0..32 { assert_eq!(node_height(&n, 0x4000 | sid), 4106.0); }
    }

    #[test]
    fn real_cell() {
        if !crate::pak_available() {
            return;
        }
        let bytes = aa_pak::Pak::global()
            .read("worlds/main_world/cells/009_014/client/terrain/heightmap.dat")
            .unwrap();
        let h = CellHeights::parse(&bytes).unwrap();
        assert_eq!(h.sectors, 256);
        assert!(h.min_z > 100.0 && h.max_z < 700.0, "{} {}", h.min_z, h.max_z);
        // neighbouring sectors share their edge
        let a = h.at(32, 40);
        let b = h.at(33, 40);
        assert!((a - b).abs() < 5.0);
        let n = h.normal(100, 100);
        assert!(n[2] > 0.0);
    }
}
