//! Terrain: heightmap -> LOD patch meshes, `cover.ctc` -> one BC texture per cell.
//!
//! A cell (1024 m) is 4 x 4 patches of 256 m. A patch is a regular grid over the 2 m height samples at
//! one of five levels of detail (step 1, 2, 4, 8, 16 samples = 2 m .. 32 m), each with a skirt around the
//! border so neighbouring patches of different detail never show cracks. Normals always come from the
//! full resolution heights (taken over the LOD step), so lighting does not pop when a patch changes
//! level. All vertices are patch-local, Bevy space (see [`crate::coords`]); the entity transform
//! places the patch. UVs map the cell's cover atlas: `uv = (cell_y / 1024, cell_x / 1024)`.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, Mesh, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::cover::{CoverAtlas, CoverFormat};
use crate::heightmap::CellHeights;

/// Height samples along one patch side (256 m).
pub const PATCH_UNITS: usize = 128;
/// Patches along one cell side.
pub const PATCHES: usize = 4;
/// Sample step of each level of detail.
pub const LOD_STEPS: [usize; 5] = [1, 2, 4, 8, 16];
/// Number of levels of detail.
pub const LOD_COUNT: usize = LOD_STEPS.len();

/// Distance (metres, to the patch centre) up to which each LOD is used: LOD `i` for
/// `d < LOD_DISTANCES[i]`, the last LOD beyond.
pub const LOD_DISTANCES: [f32; LOD_COUNT - 1] = [350.0, 750.0, 1500.0, 2600.0];

/// Level of detail for a patch at distance `d` with hysteresis around the current level.
pub fn lod_for_distance(d: f32, current: Option<usize>) -> usize {
    let at = |i: usize, grow: f32| LOD_DISTANCES[i] * grow;
    if let Some(c) = current {
        // stay while within 12 % of the thresholds around the current level
        let lower = if c == 0 { 0.0 } else { at(c - 1, 0.88) };
        let upper = if c >= LOD_COUNT - 1 { f32::MAX } else { at(c, 1.12) };
        if d >= lower && d < upper {
            return c;
        }
    }
    (0..LOD_COUNT - 1).find(|&i| d < LOD_DISTANCES[i]).unwrap_or(LOD_COUNT - 1)
}

/// The mesh of patch `(px, py)` of a cell at `lod` (patch-local Bevy-space vertices).
pub fn build_patch_mesh(h: &CellHeights, px: usize, py: usize, lod: usize) -> Mesh {
    let step = LOD_STEPS[lod];
    let n = PATCH_UNITS / step;
    let side = n + 1;
    let (ox, oy) = (px * PATCH_UNITS, py * PATCH_UNITS);
    let skirt = 3.0 * step as f32;

    let mut pos: Vec<[f32; 3]> = Vec::with_capacity(side * side + 4 * side);
    let mut nrm: Vec<[f32; 3]> = Vec::with_capacity(pos.capacity());
    let mut uv: Vec<[f32; 2]> = Vec::with_capacity(pos.capacity());
    for j in 0..=n {
        for i in 0..=n {
            let (ix, iy) = (ox + i * step, oy + j * step);
            let z = h.at(ix, iy);
            let (lx, ly) = ((i * step) as f32 * 2.0, (j * step) as f32 * 2.0);
            pos.push([lx, z, -ly]);
            let c = h.normal_step(ix, iy, step);
            nrm.push([c[0], c[2], -c[1]]);
            uv.push([(iy as f32 * 2.0) / 1024.0, (ix as f32 * 2.0) / 1024.0]);
        }
    }

    let mut idx: Vec<u32> = Vec::with_capacity(n * n * 6 + 4 * n * 6);
    for j in 0..n {
        for i in 0..n {
            if h.quad_is_hole(ox + i * step, oy + j * step, step) {
                continue;
            }
            let a = (j * side + i) as u32;
            let b = a + 1;
            let d = a + side as u32;
            let c = d + 1;
            // counter clockwise seen from above (+Y)
            idx.extend_from_slice(&[a, b, d, b, c, d]);
        }
    }

    // skirts: walk the border counter clockwise seen from above (outside on the right)
    let mut border: Vec<u32> = Vec::with_capacity(4 * n);
    border.extend((0..n).map(|i| i as u32)); // bottom edge, going east
    border.extend((0..n).map(|j| (j * side + n) as u32)); // right edge, going north
    border.extend((0..n).map(|i| (n * side + n - i) as u32)); // top edge, going west
    border.extend((0..n).map(|j| ((n - j) * side) as u32)); // left edge, going south
    let mut skirt_idx = std::collections::HashMap::new();
    for &v in &border {
        skirt_idx.entry(v).or_insert_with(|| {
            let k = pos.len() as u32;
            let mut p = pos[v as usize];
            p[1] -= skirt;
            pos.push(p);
            nrm.push(nrm[v as usize]);
            uv.push(uv[v as usize]);
            k
        });
    }
    for (p, &a) in border.iter().enumerate() {
        // Do not seal an opening on a patch border with its artificial crack skirt.
        let (i, j) = match p / n {
            0 => (p % n, 0),
            1 => (n - 1, p % n),
            2 => (n - 1 - p % n, n - 1),
            _ => (0, n - 1 - p % n),
        };
        if h.quad_is_hole(ox + i * step, oy + j * step, step) {
            continue;
        }
        let b = border[(p + 1) % border.len()];
        let (a2, b2) = (skirt_idx[&a], skirt_idx[&b]);
        idx.extend_from_slice(&[a, b2, b, a, a2, b2]);
    }

    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    mesh.insert_indices(Indices::U32(idx));
    mesh
}

/// A BC texture (with its mip chain) from a cover atlas. The texels are **not** colours (see
/// [`crate::terrain_layers`]): the format is plain `Unorm`, the terrain shader decodes them after filtering.
pub fn cover_image(atlas: CoverAtlas) -> Image {
    let mut image = Image::default();
    image.texture_descriptor.size = Extent3d { width: atlas.size, height: atlas.size, depth_or_array_layers: 1 };
    image.texture_descriptor.dimension = TextureDimension::D2;
    image.texture_descriptor.mip_level_count = atlas.mips;
    image.texture_descriptor.format = match atlas.format {
        CoverFormat::Bc1 => TextureFormat::Bc1RgbaUnorm,
        CoverFormat::Bc3 => TextureFormat::Bc3RgbaUnorm,
    };
    image.data = Some(atlas.data);
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 8,
        ..default()
    });
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lod_selection() {
        assert_eq!(lod_for_distance(10.0, None), 0);
        assert_eq!(lod_for_distance(400.0, None), 1);
        assert_eq!(lod_for_distance(9000.0, None), LOD_COUNT - 1);
        // hysteresis: just past a threshold keeps the current level
        assert_eq!(lod_for_distance(360.0, Some(0)), 0);
        assert_eq!(lod_for_distance(500.0, Some(0)), 1);
        assert_eq!(lod_for_distance(340.0, Some(1)), 1);
        assert_eq!(lod_for_distance(300.0, Some(1)), 0);
    }

    #[test]
    fn patch_mesh_counts() {
        let h = CellHeights {
            heights: (0..crate::heightmap::GRID * crate::heightmap::GRID).map(|i| (i % 7) as f32).collect(),
            surface: vec![1; crate::heightmap::GRID * crate::heightmap::GRID],
            hole_sectors: [false; 256],
            ocean_level: 100.0,
            min_z: 0.0,
            max_z: 6.0,
            sectors: 256,
        };
        for lod in 0..LOD_COUNT {
            let m = build_patch_mesh(&h, 1, 2, lod);
            let n = PATCH_UNITS / LOD_STEPS[lod];
            assert_eq!(m.count_vertices(), (n + 1) * (n + 1) + 4 * n);
            assert_eq!(m.indices().unwrap().len(), n * n * 6 + 4 * n * 6);
        }
    }

    #[test]
    fn original_hole_leaf_uses_native_lod_centre_and_triangle_order() {
        if !crate::pak_available() { return; }
        let bytes = aa_pak::Pak::global().read("worlds/main_world/cells/020_008/client/terrain/heightmap.dat").unwrap();
        let h = CellHeights::parse(&bytes).unwrap();
        // Full original 300E0A10 branch outputs, both interior and border loops.
        for (lod, omitted) in [22, 6, 3, 0, 0].into_iter().enumerate() {
            let step = LOD_STEPS[lod];
            let native_holes = (352..384).step_by(step).flat_map(|x|
                (416..448).step_by(step).map(move |y| (x,y)))
                .filter(|&(x,y)| h.quad_is_hole(x,y,step)).count();
            assert_eq!(native_holes, omitted);
            let mesh = build_patch_mesh(&h, 2, 3, lod);
            let side = PATCH_UNITS / step + 1;
            let tris: Vec<_> = mesh.indices().unwrap().iter().collect();
            for tri in tris.chunks_exact(3).filter(|t| t.iter().all(|&v| v < side * side)) {
                let coords: Vec<_> = tri.iter().map(|&v| (256 + v % side * step, 384 + v / side * step)).collect();
                let x = coords.iter().map(|c| c.0).min().unwrap();
                let y = coords.iter().map(|c| c.1).min().unwrap();
                assert!(!h.quad_is_hole(x,y,step));
                assert!(!coords.contains(&(x,y)) || !coords.contains(&(x+step,y+step)), "native diagonal");
            }
        }
    }
}
