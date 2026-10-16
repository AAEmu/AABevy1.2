//! `cells/XXX_YYY/client/terrain/cover.ctc`: the baked terrain colour texture of a cell.
//!
//! CryEngine's compressed terrain cover: a quadtree of 128 x 128 px DXT tiles, one pre-filtered tile set
//! per level (the engine's mip chain).
//!
//! ```text
//! "CRY\0", u16 version (100), u16 8
//! u16 layerCount, u16 flags, f32 sunShadowIntensity
//! layerCount x { u16 sectorSizePixels (128), u16 reserved, u32 texFormat (22 = DXT1, 24 = DXT5),
//!                u32 sectorSizeBytes (8192 / 16384) }
//! u16 nodeCount, nodeCount x u16 tile index (0xFFFF = none)
//! tiles: tile[i] = the layers one after another, sectorSizeBytes each
//! ```
//!
//! The table is a depth first quadtree walk: a node with a tile (`!= 0xFFFF`) is followed by its four
//! children (x0y0, x1y0, x0y1, x1y1), a node without one has none, so `nodeCount = 1 + 4 * tiles`. Most
//! cells carry five levels (341 tiles, level 4 = 16 x 16 tiles of 64 m at 0.5 m/px); ocean cells
//! stop at level 1 or 2, coastal cells have fine tiles only on the land. Index = x + 2 * y of the
//! child.
//!
//! Level `n` is mip `maxLevel - n` of the cell atlas, so the file *is* a pre-built mip chain:
//! [`Cover::atlas`] rearranges the blocks into one BC texture and never decodes a pixel, except where
//! a fine tile is missing under a coarser one (coast): that tile is re-encoded from its parent.
//!
//! Orientation: the heightmap and every cell file are x major. The tile at quadtree position
//! `(x, y)` (x along world X) holds the picture with image *rows* along world X and *columns* along
//! world Y, so the cell atlas is sampled with `uv = (world_y / 1024, world_x / 1024)`.

use std::collections::HashMap;

use crate::reader::{ReadError, Reader};

/// Tile edge in pixels.
pub const TILE_PX: usize = 128;
const BLOCKS: usize = TILE_PX / 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoverFormat {
    /// DXT1 / BC1, 8 bytes per 4 x 4 block.
    Bc1,
    /// DXT5 / BC3, 16 bytes per 4 x 4 block.
    Bc3,
}

impl CoverFormat {
    pub fn block_bytes(self) -> usize {
        match self {
            CoverFormat::Bc1 => 8,
            CoverFormat::Bc3 => 16,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CoverError {
    #[error("not a cover.ctc (bad signature)")]
    Signature,
    #[error("truncated cover.ctc")]
    Truncated,
    #[error("unsupported cover.ctc: {0}")]
    Unsupported(String),
}

impl From<ReadError> for CoverError {
    fn from(_: ReadError) -> Self {
        CoverError::Truncated
    }
}

#[derive(Clone, Debug)]
pub struct CoverLayer {
    pub format: CoverFormat,
    pub tile_bytes: usize,
}

/// A parsed `cover.ctc` (tile data borrowed from the file bytes).
pub struct Cover<'a> {
    pub layers: Vec<CoverLayer>,
    pub sun_shadow_intensity: f32,
    /// `(level, x, y)` -> tile index.
    tiles: HashMap<(u8, u8, u8), u16>,
    /// Deepest level that has a tile.
    pub max_level: usize,
    data: &'a [u8],
    tile_stride: usize,
}

/// One texture ready to upload: BC blocks with `mips` levels of a `size` x `size` image.
pub struct CoverAtlas {
    pub format: CoverFormat,
    pub size: u32,
    pub mips: u32,
    pub data: Vec<u8>,
}

impl<'a> Cover<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Cover<'a>, CoverError> {
        let mut r = Reader::new(bytes);
        if r.bytes(4)? != b"CRY\0" {
            return Err(CoverError::Signature);
        }
        let version = r.u16()?;
        let sub = r.u16()?;
        if version != 100 || sub != 8 {
            return Err(CoverError::Unsupported(format!("version {version}/{sub}")));
        }
        let layer_count = r.u16()? as usize;
        let _flags = r.u16()?;
        let sun_shadow_intensity = r.f32()?;
        if layer_count == 0 || layer_count > 4 {
            return Err(CoverError::Unsupported(format!("{layer_count} layers")));
        }
        let mut layers = Vec::new();
        for _ in 0..layer_count {
            let px = r.u16()?;
            let _reserved = r.u16()?;
            let format = r.u32()?;
            let bytes = r.u32()? as usize;
            if px as usize != TILE_PX {
                return Err(CoverError::Unsupported(format!("{px} px tiles")));
            }
            let format = match (format, bytes) {
                (22, 8192) => CoverFormat::Bc1,
                (24, 16384) => CoverFormat::Bc3,
                _ => return Err(CoverError::Unsupported(format!("format {format}, {bytes} bytes"))),
            };
            layers.push(CoverLayer { format, tile_bytes: bytes });
        }
        let node_count = r.u16()? as usize;
        let mut table = Vec::with_capacity(node_count);
        for _ in 0..node_count {
            table.push(r.u16()?);
        }
        let mut tiles = HashMap::new();
        let mut cursor = 0usize;
        walk(&table, &mut cursor, 0, 0, 0, &mut tiles)?;
        let tile_stride: usize = layers.iter().map(|l| l.tile_bytes).sum();
        let data = &bytes[r.pos()..];
        if let Some(max) = tiles.values().map(|&t| t as usize).max() {
            if (max + 1) * tile_stride > data.len() {
                return Err(CoverError::Truncated);
            }
        }
        let max_level = tiles.keys().map(|k| k.0 as usize).max().unwrap_or(0);
        Ok(Cover { layers, sun_shadow_intensity, tiles, max_level, data, tile_stride })
    }

    /// The raw blocks of tile `(level, x, y)` of `layer`, when the file has it.
    pub fn tile(&self, layer: usize, level: usize, x: usize, y: usize) -> Option<&'a [u8]> {
        let idx = *self.tiles.get(&(level as u8, x as u8, y as u8))?;
        let off: usize =
            idx as usize * self.tile_stride + self.layers[..layer].iter().map(|l| l.tile_bytes).sum::<usize>();
        self.data.get(off..off + self.layers[layer].tile_bytes)
    }

    /// Builds the BC atlas of `layer`: `128 << max_level` px square with `max_level + 1` mips.
    pub fn atlas(&self, layer: usize) -> CoverAtlas {
        let format = self.layers[layer].format;
        let bb = format.block_bytes();
        let top = self.max_level;
        let mut derived: HashMap<(usize, usize, usize), Vec<u8>> = HashMap::new();
        let mut data = Vec::new();
        for mip in 0..=top {
            let level = top - mip;
            let tiles_side = 1usize << level;
            let blocks_side = tiles_side * BLOCKS;
            let mut level_data = vec![0u8; blocks_side * blocks_side * bb];
            for x in 0..tiles_side {
                for y in 0..tiles_side {
                    let owned;
                    let tile: &[u8] = match self.tile(layer, level, x, y) {
                        Some(t) => t,
                        None => match self.derive(layer, level, x, y, &mut derived) {
                            Some(v) => {
                                owned = v;
                                &owned
                            }
                            None => continue,
                        },
                    };
                    // image rows along world X, columns along world Y
                    for r in 0..BLOCKS {
                        let src = &tile[r * BLOCKS * bb..(r + 1) * BLOCKS * bb];
                        let dst = ((x * BLOCKS + r) * blocks_side + y * BLOCKS) * bb;
                        level_data[dst..dst + src.len()].copy_from_slice(src);
                    }
                }
            }
            data.extend_from_slice(&level_data);
        }
        CoverAtlas { format, size: (TILE_PX << top) as u32, mips: top as u32 + 1, data }
    }

    /// A missing tile re-encoded from its parent's quadrant (BC1 only).
    fn derive(
        &self,
        layer: usize,
        level: usize,
        x: usize,
        y: usize,
        cache: &mut HashMap<(usize, usize, usize), Vec<u8>>,
    ) -> Option<Vec<u8>> {
        if level == 0 || self.layers[layer].format != CoverFormat::Bc1 {
            return None;
        }
        if let Some(v) = cache.get(&(level, x, y)) {
            return Some(v.clone());
        }
        let (px, py) = (x / 2, y / 2);
        let parent = match self.tile(layer, level - 1, px, py) {
            Some(t) => t.to_vec(),
            None => self.derive(layer, level - 1, px, py, cache)?,
        };
        let rgb = bc1_decode(&parent);
        // the child covers one quadrant of the parent picture (rows = world X, columns = world Y)
        let (r0, c0) = ((x & 1) * TILE_PX / 2, (y & 1) * TILE_PX / 2);
        let mut up = vec![[0u8; 3]; TILE_PX * TILE_PX];
        for r in 0..TILE_PX {
            for c in 0..TILE_PX {
                let fr = r0 as f32 + (r as f32 + 0.5) / 2.0 - 0.5;
                let fc = c0 as f32 + (c as f32 + 0.5) / 2.0 - 0.5;
                up[r * TILE_PX + c] = bilinear(&rgb, fr, fc);
            }
        }
        let out = bc1_encode(&up);
        cache.insert((level, x, y), out.clone());
        Some(out)
    }
}

fn walk(
    table: &[u16],
    cursor: &mut usize,
    level: u8,
    x: u8,
    y: u8,
    out: &mut HashMap<(u8, u8, u8), u16>,
) -> Result<(), CoverError> {
    let t = *table.get(*cursor).ok_or(CoverError::Truncated)?;
    *cursor += 1;
    if t != 0xFFFF {
        if level >= 7 {
            return Err(CoverError::Unsupported("quadtree deeper than 7 levels".into()));
        }
        out.insert((level, x, y), t);
        for c in 0..4u8 {
            walk(table, cursor, level + 1, x * 2 + (c & 1), y * 2 + (c >> 1), out)?;
        }
    }
    Ok(())
}

fn c565(c: u16) -> [u8; 3] {
    let (r, g, b) = ((c >> 11) & 31, (c >> 5) & 63, c & 31);
    [((r << 3) | (r >> 2)) as u8, ((g << 2) | (g >> 4)) as u8, ((b << 3) | (b >> 2)) as u8]
}

/// Decodes one 128 x 128 BC1 tile to RGB (row major).
pub fn bc1_decode(data: &[u8]) -> Vec<[u8; 3]> {
    let mut out = vec![[0u8; 3]; TILE_PX * TILE_PX];
    for (bi, blk) in data.chunks_exact(8).enumerate() {
        let (c0, c1) = (u16::from_le_bytes([blk[0], blk[1]]), u16::from_le_bytes([blk[2], blk[3]]));
        let (a, b) = (c565(c0), c565(c1));
        let mix = |w0: u32, w1: u32, d: u32| -> [u8; 3] {
            [0, 1, 2].map(|i| ((a[i] as u32 * w0 + b[i] as u32 * w1) / d) as u8)
        };
        let pal = if c0 > c1 { [a, b, mix(2, 1, 3), mix(1, 2, 3)] } else { [a, b, mix(1, 1, 2), [0, 0, 0]] };
        let bits = u32::from_le_bytes([blk[4], blk[5], blk[6], blk[7]]);
        let (bx, by) = (bi % BLOCKS, bi / BLOCKS);
        for p in 0..16 {
            let (px, py) = (p % 4, p / 4);
            out[(by * 4 + py) * TILE_PX + bx * 4 + px] = pal[(bits >> (2 * p) & 3) as usize];
        }
    }
    out
}

fn bilinear(img: &[[u8; 3]], fr: f32, fc: f32) -> [u8; 3] {
    let max = (TILE_PX - 1) as f32;
    let (fr, fc) = (fr.clamp(0.0, max), fc.clamp(0.0, max));
    let (r, c) = (fr.floor() as usize, fc.floor() as usize);
    let (r1, c1) = ((r + 1).min(TILE_PX - 1), (c + 1).min(TILE_PX - 1));
    let (tr, tc) = (fr - r as f32, fc - c as f32);
    let at = |r: usize, c: usize| img[r * TILE_PX + c];
    [0, 1, 2].map(|i| {
        let top = at(r, c)[i] as f32 * (1.0 - tc) + at(r, c1)[i] as f32 * tc;
        let bot = at(r1, c)[i] as f32 * (1.0 - tc) + at(r1, c1)[i] as f32 * tc;
        (top * (1.0 - tr) + bot * tr + 0.5) as u8
    })
}

fn to565(c: [u8; 3]) -> u16 {
    (((c[0] as u16 * 31 + 127) / 255) << 11) | (((c[1] as u16 * 63 + 127) / 255) << 5) | ((c[2] as u16 * 31 + 127) / 255)
}

/// Encodes a 128 x 128 RGB tile to BC1 (bounding box endpoints: plenty for smooth, blurred terrain).
pub fn bc1_encode(px: &[[u8; 3]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(BLOCKS * BLOCKS * 8);
    for by in 0..BLOCKS {
        for bx in 0..BLOCKS {
            let mut block = [[0u8; 3]; 16];
            for p in 0..16 {
                block[p] = px[(by * 4 + p / 4) * TILE_PX + bx * 4 + p % 4];
            }
            let mut lo = [255u8; 3];
            let mut hi = [0u8; 3];
            for c in &block {
                for i in 0..3 {
                    lo[i] = lo[i].min(c[i]);
                    hi[i] = hi[i].max(c[i]);
                }
            }
            let (mut e0, mut e1) = (to565(hi), to565(lo));
            if e0 < e1 {
                std::mem::swap(&mut e0, &mut e1);
            }
            let mut bits = 0u32;
            if e0 == e1 {
                // flat block: all index 0
            } else {
                let (a, b) = (c565(e0), c565(e1));
                let pal = [
                    a,
                    b,
                    [0, 1, 2].map(|i| ((a[i] as u32 * 2 + b[i] as u32) / 3) as u8),
                    [0, 1, 2].map(|i| ((a[i] as u32 + b[i] as u32 * 2) / 3) as u8),
                ];
                for (p, c) in block.iter().enumerate() {
                    let best = (0..4)
                        .min_by_key(|&k| (0..3).map(|i| (c[i] as i32 - pal[k][i] as i32).pow(2)).sum::<i32>())
                        .unwrap() as u32;
                    bits |= best << (2 * p);
                }
            }
            out.extend_from_slice(&e0.to_le_bytes());
            out.extend_from_slice(&e1.to_le_bytes());
            out.extend_from_slice(&bits.to_le_bytes());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_cover() {
        if !crate::pak_available() {
            return;
        }
        let bytes =
            aa_pak::Pak::global().read("worlds/main_world/cells/009_014/client/terrain/cover.ctc").unwrap();
        let c = Cover::parse(&bytes).unwrap();
        assert_eq!(c.layers.len(), 1);
        assert_eq!(c.max_level, 4);
        let a = c.atlas(0);
        assert_eq!(a.format, CoverFormat::Bc1);
        assert_eq!(a.size, 2048);
        let expect: usize = (0..5).map(|m| (128usize << (4 - m)).pow(2) / 2).sum();
        assert_eq!(a.data.len(), expect);
        assert!(c.tile(0, 4, 15, 15).is_some());
    }

    #[test]
    fn ocean_and_coast_cells() {
        if !crate::pak_available() {
            return;
        }
        let pak = aa_pak::Pak::global();
        // 21 nodes: two levels only
        let bytes = pak.read("worlds/main_world/cells/004_005/client/terrain/cover.ctc").unwrap();
        let c = Cover::parse(&bytes).unwrap();
        assert_eq!(c.max_level, 1);
        assert_eq!(c.atlas(0).size, 256);
        // every cell parses and builds an atlas of the right size (sample some irregular ones)
        for name in ["004_004", "006_012", "020_013"] {
            let bytes = pak.read(&format!("worlds/main_world/cells/{name}/client/terrain/cover.ctc")).unwrap();
            let c = Cover::parse(&bytes).unwrap();
            let a = c.atlas(0);
            let expect: usize = (0..=c.max_level).map(|m| (128usize << (c.max_level - m)).pow(2) / 2).sum();
            assert_eq!(a.data.len(), expect, "{name}");
        }
    }

    #[test]
    fn bc1_round_trip() {
        let mut px = vec![[0u8; 3]; TILE_PX * TILE_PX];
        for r in 0..TILE_PX {
            for c in 0..TILE_PX {
                px[r * TILE_PX + c] = [(r * 2) as u8, (c * 2) as u8, 120];
            }
        }
        let back = bc1_decode(&bc1_encode(&px));
        let err: i32 = px.iter().zip(&back).map(|(a, b)| (a[0] as i32 - b[0] as i32).abs()).sum::<i32>()
            / px.len() as i32;
        assert!(err < 6, "mean error {err}");
    }
}
