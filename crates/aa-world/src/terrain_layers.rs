//! Terrain detail layers: the surface types of `leveldata.xml` blended over the baked cover colour.
//!
//! CryEngine draws the terrain twice over: a base pass with the baked `cover.ctc` colour, then one alpha
//! blended `Terrain.Layer` pass per surface type, whose weight per 2 m heightmap vertex is 1 for its own
//! type and 0 for the others (interpolated over the triangles), faded out towards
//! `e_detail_materials_view_dist_xy` (1024 m) and, for materials with three sub-materials (x, y, z), split
//! by the dominant axis of the surface normal (triplanar: the cliff faces use the x / y sub-material, the
//! ground the z one; a material without sub-materials only draws where the ground is flatter than about
//! 45 degrees). The layer colour is **added** to the cover in gamma space,
//! `srgb = saturate(cover + (detail - 0.5) * DetailTextureStrength)`, and lit with the layer's own
//! normal map plus a tiled `Detail` normal map (`terrain.cfx`, `TerrainLayerPS` / `frag_custom_begin`,
//! `fraglib.cfi`; the shader sources are in the pak).
//!
//! **The cover texels are not colours.** `tex2DTerrain` (`common.cfi`) decodes them after filtering: red and
//! green hold the chromaticity (`b = 1 - r - g`, with `r' = (r + 0.001012) * 31 / 30`) and blue the
//! brightness, `colour = saturate(chroma * 3 * blue)`, in gamma space (`pow(., 2.2)` for lighting). Read as
//! plain RGB (what this crate did first) the ground is a saturated yellow green and the rock is not grey.
//! The cover image is therefore a non-sRGB BC texture and the shader decodes it.
//!
//! Here the same blend runs in one pass of a [`TerrainMaterial`] (`StandardMaterial` extended by
//! `terrain_layers.wesl`):
//!
//! * [`LayerLibrary`] (built once per world on a task, 0.3 s): reads the 16 entry `SurfaceTypes` table of every
//!   cell's `leveldata.xml`, the `.mtl` of each `DetailMaterial` and every referenced texture and packs
//!   them into three texture arrays, copying the DDS blocks (BC1 diffuse and BC5 normal maps at 512 px,
//!   BC5 detail normal maps at 256 px, which also take the normal maps that are only 256 px), each with its
//!   mip chain down to 4 px and at most 255 layers (the limit of a wgpu texture array is 256). The BC1
//!   (`NORMAL_DXT1`) normal maps are transcoded to BC5.
//! * per cell: an `R8Uint` image of the 4 bit surface type of every 2 m sample (`heightmap.dat`) and a
//!   uniform table (`TerrainParams`) with, for each of the 16 surface types and 3 projections, the
//!   array slots, the uv scale (`DetailScaleX/Y` x the `.mtl` tiling), the uv offset that keeps tiles
//!   continuous across cell borders, the detail strength and the detail normal map's tiling.
//!
//! The shader takes the four surrounding samples (bilinear weights), the layers they select, the
//! projection weights from the normal and blends sequentially like the engine does.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use aa_cry::material::{Material as CryMaterial, TextureMap};
use aa_cry::texture::{Dds, DdsFormat};
use bevy::asset::{RenderAssetUsages, embedded_asset};
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, ShaderType, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
};
use bevy::shader::ShaderRef;
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};

use crate::coords::CELL_SIZE;
use crate::heightmap::GRID;
use crate::stream::WorldConfig;
use crate::world::cell_dir;

/// A wgpu texture array has at most 256 layers by default.
const MAX_LAYERS: usize = 255;
/// Surface types per cell (the heightmap stores 4 bits).
pub const SURFACE_TYPES: usize = 16;
/// Projections of a layer: x, y, z.
const PROJECTIONS: usize = 3;
/// Vectors of the uniform table per surface type and projection.
const ENTRY: usize = 4;
/// Added to the normal slot of a layer whose normal map lives in the 256 px (detail) array.
const SMALL_NORMAL: f32 = 1000.0;
/// Detail layers fade out like `1 - (d / DETAIL_DISTANCE)^4` (`e_detail_materials_view_dist_xy`, quality 4).
pub const DETAIL_DISTANCE: f32 = 1024.0;

/// The terrain material: a `StandardMaterial` (cover colour, lighting) extended with the layers.
pub type TerrainMaterial = ExtendedMaterial<StandardMaterial, TerrainExt>;

/// Per cell data of the terrain shader (uniform, binding 100).
#[derive(Clone, Copy, Debug, ShaderType, Reflect)]
pub struct TerrainParams {
    /// Linear colour factor (`WorldConfig::terrain_tint`), `w` = detail fade distance.
    pub tint: Vec4,
    /// Four vectors per `(surface type * 3 + projection)`: `(diffuse slot, normal slot (+ 1000: in the detail
    /// array), uv scale u, v)`,
    /// `(uv offset u, v, DetailTextureStrength, DetailBumpScale)`,
    /// `(detail normal slot, detail uv scale u, v, 0)` and `(detail uv offset u, v, 0, 0)`;
    /// a slot `< 0` means none.
    pub layers: [Vec4; SURFACE_TYPES * PROJECTIONS * ENTRY],
}

impl TerrainParams {
    /// A table without layers.
    pub fn new(tint: Vec3) -> Self {
        let mut layers = [Vec4::ZERO; SURFACE_TYPES * PROJECTIONS * ENTRY];
        for entry in layers.chunks_mut(ENTRY) {
            entry[0] = Vec4::new(-1.0, -1.0, 0.0, 0.0);
            entry[2] = Vec4::new(-1.0, 0.0, 0.0, 0.0);
        }
        TerrainParams { tint: tint.extend(DETAIL_DISTANCE), layers }
    }
}

impl Default for TerrainParams {
    fn default() -> Self {
        TerrainParams::new(Vec3::ONE)
    }
}

/// The material extension: layer arrays, the cell's surface map and the lookup table.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct TerrainExt {
    #[uniform(100)]
    pub params: TerrainParams,
    #[texture(101, dimension = "2d_array")]
    #[sampler(102)]
    pub diffuse: Handle<Image>,
    #[texture(103, dimension = "2d_array")]
    #[sampler(104)]
    pub normals: Handle<Image>,
    #[texture(105, dimension = "2d_array")]
    #[sampler(106)]
    pub details: Handle<Image>,
    #[texture(107, sample_type = "u_int")]
    pub surface: Handle<Image>,
}

impl MaterialExtension for TerrainExt {
    fn fragment_shader() -> ShaderRef {
        "embedded://aa_world/terrain_layers.wesl".into()
    }
}

/// Which array a texture goes to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// BC1, 512 px.
    Diffuse,
    /// BC5 (x, y), 512 px.
    Normal,
    /// BC5 detail normal map, 256 px.
    Detail,
}

impl Kind {
    const ALL: [Kind; 3] = [Kind::Diffuse, Kind::Normal, Kind::Detail];

    /// Edge of a layer (px). Larger sources use their lower mips.
    fn px(self) -> u32 {
        if self == Kind::Detail { 256 } else { 512 }
    }

    /// Mip levels of a layer: down to 4 px, where the DDS files of the pak stop.
    fn mips(self) -> u32 {
        self.px().trailing_zeros() - 1
    }

    fn format(self) -> TextureFormat {
        if self == Kind::Diffuse { TextureFormat::Bc1RgbaUnorm } else { TextureFormat::Bc5RgUnorm }
    }

    fn block_bytes(self) -> usize {
        if self == Kind::Diffuse { 8 } else { 16 }
    }

    /// Bytes of one layer with its whole mip chain.
    fn layer_bytes(self) -> usize {
        (0..self.mips()).map(|m| ((self.px() >> m).max(1) as usize).div_ceil(4).pow(2) * self.block_bytes()).sum()
    }
}

/// One projection of a surface material (x, y or z sub-material).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Projection {
    diffuse: Option<u16>,
    /// Slot of the normal map in the normal array, or in the 256 px detail array when `normal_small`.
    normal: Option<u16>,
    normal_small: bool,
    /// `TexMod` tiling of the diffuse map.
    tile: [f32; 2],
    /// `DetailTextureStrength`.
    strength: f32,
    /// The detail normal map (`Detail` texture, `DETAIL_BUMP_MAPPING`), its tiling relative to the layer
    /// uv (`DetailBumpTillingU / V`) and `DetailBumpScale`.
    detail: Option<u16>,
    detail_tile: [f32; 2],
    detail_scale: f32,
}

/// A `.mtl` reduced to what the terrain shader needs.
#[derive(Clone, Copy, Debug, Default)]
struct SurfaceMaterial {
    /// x, y, z.
    proj: [Option<Projection>; PROJECTIONS],
}

/// A `<SurfaceType>` of `leveldata.xml`.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceEntry {
    /// `DetailMaterial` as written.
    pub material: String,
    /// `DetailScaleX`, `DetailScaleY`.
    pub scale: [f32; 2],
}

/// The surface types of a `leveldata.xml` in file order (the index is what the heightmap stores).
pub fn parse_surface_types(xml: &str) -> Vec<SurfaceEntry> {
    let Ok(doc) = roxmltree::Document::parse(xml) else { return Vec::new() };
    doc.descendants()
        .filter(|n| n.has_tag_name("SurfaceType"))
        .map(|n| {
            let f = |name: &str| n.attribute(name).and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(1.0);
            SurfaceEntry {
                material: n.attribute("DetailMaterial").unwrap_or("").trim().to_string(),
                scale: [f("DetailScaleX"), f("DetailScaleY")],
            }
        })
        .collect()
}

/// `leveldata.xml` table of a cell: material index into [`LayerLibrary::mats`] (`u32::MAX` = none) and scale.
type CellTable = [(u32, [f32; 2]); SURFACE_TYPES];

/// The detail layers of one world.
pub struct LayerLibrary {
    mats: Vec<SurfaceMaterial>,
    cells: HashMap<(i32, i32), CellTable>,
    /// Layers in the diffuse / normal / detail array.
    pub layers: [usize; 3],
}

/// What the loading task returns: the library and the three array images.
pub struct BuiltLayers {
    pub library: LayerLibrary,
    /// Diffuse, normal and detail normal arrays.
    pub images: [Image; 3],
}

impl LayerLibrary {
    /// Reads every cell's surface types and all layer textures. Takes a fraction of a second; run on a task.
    pub fn build(world: &str, cells: impl Iterator<Item = (i32, i32)>) -> Option<BuiltLayers> {
        let started = Instant::now();
        let pak = aa_pak::Pak::global();
        // 1. the surface type table of every cell; materials interned by normalised path
        let mut mat_index: HashMap<String, u32> = HashMap::new();
        let mut mat_paths: Vec<String> = Vec::new();
        let mut mat_cells: Vec<usize> = Vec::new();
        let mut tables: HashMap<(i32, i32), CellTable> = HashMap::new();
        for cell in cells {
            let path = format!("{}/client/leveldata.xml", cell_dir(world, cell.0, cell.1));
            let Ok(xml) = pak.read_string(&path) else { continue };
            let mut table: CellTable = [(u32::MAX, [1.0, 1.0]); SURFACE_TYPES];
            let mut seen = Vec::new();
            for (i, e) in parse_surface_types(&xml).into_iter().take(SURFACE_TYPES).enumerate() {
                if e.material.is_empty() {
                    continue;
                }
                let key = e.material.replace('\\', "/").to_ascii_lowercase();
                let id = *mat_index.entry(key.clone()).or_insert_with(|| {
                    mat_paths.push(key);
                    mat_cells.push(0);
                    mat_paths.len() as u32 - 1
                });
                if !seen.contains(&id) {
                    seen.push(id);
                    mat_cells[id as usize] += 1;
                }
                table[i] = (id, e.scale);
            }
            tables.insert(cell, table);
        }
        if mat_paths.is_empty() {
            return None;
        }

        // 2. the materials and the texture files they use (weighted by the cells that use the material)
        let loaded: Vec<Option<[Option<RawProjection>; PROJECTIONS]>> = parallel(&mat_paths, |p| load_material(p));
        let mut wants: [HashMap<String, usize>; 3] = Default::default();
        for (mat, raw) in loaded.iter().enumerate() {
            let Some(raw) = raw else { continue };
            for pr in raw.iter().flatten() {
                for (kind, path) in [(0, &pr.diffuse), (1, &pr.normal), (2, &pr.detail)] {
                    if let Some(path) = path {
                        let seen = wants[kind].entry(path.clone()).or_default();
                        *seen = (*seen).max(mat_cells[mat]);
                    }
                }
            }
        }

        // 3. read the textures (the most used first), keep what fits the arrays; normal maps that are not
        // 512 px go to the 256 px array
        let mut slots: [HashMap<String, u16>; 3] = Default::default();
        let mut data: [Vec<u8>; 3] = Default::default();
        for (k, kind) in Kind::ALL.into_iter().enumerate() {
            let mut paths: Vec<(&String, &usize)> = wants[k].iter().collect();
            paths.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            let paths: Vec<String> = paths.into_iter().map(|(p, _)| p.clone()).collect();
            let layers = parallel(&paths, |p| load_layer(p, kind));
            let mut dropped = 0;
            for (path, layer) in paths.into_iter().zip(layers) {
                match layer {
                    Some(bytes) if slots[k].len() < MAX_LAYERS => {
                        slots[k].insert(path, slots[k].len() as u16);
                        data[k].extend_from_slice(&bytes);
                    }
                    _ if kind == Kind::Normal => {
                        let uses = wants[1][&path];
                        let seen = wants[2].entry(path).or_default();
                        *seen = (*seen).max(uses);
                    }
                    _ => dropped += 1,
                }
            }
            if dropped > 0 {
                debug!("aa-world: terrain layers: {dropped} unusable {kind:?} textures");
            }
        }

        // 4. the library
        let mats: Vec<SurfaceMaterial> = loaded
            .iter()
            .map(|raw| {
                let mut m = SurfaceMaterial::default();
                if let Some(raw) = raw {
                    for (p, pr) in raw.iter().enumerate() {
                        let Some(pr) = pr else { continue };
                        let slot = |k: usize, path: &Option<String>| path.as_ref().and_then(|t| slots[k].get(t).copied());
                        let diffuse = slot(0, &pr.diffuse);
                        if diffuse.is_some() {
                            let small = pr.normal.as_ref().and_then(|t| slots[2].get(t).copied());
                            let big = slot(1, &pr.normal);
                            m.proj[p] = Some(Projection {
                                diffuse,
                                normal: big.or(small),
                                normal_small: big.is_none() && small.is_some(),
                                tile: pr.tile,
                                strength: pr.strength,
                                detail: slot(2, &pr.detail),
                                detail_tile: pr.detail_tile,
                                detail_scale: pr.detail_scale,
                            });
                        }
                    }
                }
                m
            })
            .collect();
        let layers = [slots[0].len(), slots[1].len(), slots[2].len()];
        info!(
            "aa-world: terrain layers of {world}: {} cells, {} materials, {} diffuse + {} normal + {} detail maps ({:.0} MB), {:.1} s",
            tables.len(),
            mats.len(),
            layers[0],
            layers[1],
            layers[2],
            data.iter().map(Vec::len).sum::<usize>() as f32 / 1e6,
            started.elapsed().as_secs_f32()
        );
        let [d0, d1, d2] = data;
        Some(BuiltLayers {
            library: LayerLibrary { mats, cells: tables, layers },
            images: [
                layer_array(d0, layers[0], Kind::Diffuse),
                layer_array(d1, layers[1], Kind::Normal),
                layer_array(d2, layers[2], Kind::Detail),
            ],
        })
    }

    /// The uniform table of a cell (`None` when the cell has no `leveldata.xml`).
    pub fn params(&self, cell: (i32, i32), tint: Vec3) -> Option<TerrainParams> {
        let table = self.cells.get(&cell)?;
        let mut params = TerrainParams::new(tint);
        let origin = [cell.0 as f64 * CELL_SIZE as f64, cell.1 as f64 * CELL_SIZE as f64];
        // the layer uv is `world * scale`; keep it small and continuous across cells by shifting the
        // cell-local coordinate with the fractional part of the cell origin (the vertical one needs none)
        let offset = |p: usize, su: f32, sv: f32| -> [f32; 2] {
            let fract = |v: f64| v.rem_euclid(1.0) as f32;
            match p {
                0 => [fract(origin[1] * su as f64), 0.0],
                1 => [fract(origin[0] * su as f64), 0.0],
                _ => [fract(origin[0] * su as f64), fract(origin[1] * sv as f64)],
            }
        };
        for (s, &(mat, scale)) in table.iter().enumerate() {
            let Some(mat) = self.mats.get(mat as usize) else { continue };
            for (p, proj) in mat.proj.iter().enumerate() {
                let Some(proj) = proj else { continue };
                let (su, sv) = (scale[0] * proj.tile[0], scale[1] * proj.tile[1]);
                let off = offset(p, su, sv);
                let i = (s * PROJECTIONS + p) * ENTRY;
                params.layers[i] = Vec4::new(
                    proj.diffuse.map_or(-1.0, |v| v as f32),
                    proj.normal.map_or(-1.0, |v| v as f32 + if proj.normal_small { SMALL_NORMAL } else { 0.0 }),
                    su,
                    sv,
                );
                params.layers[i + 1] = Vec4::new(off[0], off[1], proj.strength, proj.detail_scale);
                if let Some(detail) = proj.detail {
                    // the detail map is tiled over the layer uv: `uv * tiling`
                    let (du, dv) = (su * proj.detail_tile[0], sv * proj.detail_tile[1]);
                    let doff = offset(p, du, dv);
                    params.layers[i + 2] = Vec4::new(detail as f32, du, dv, 0.0);
                    params.layers[i + 3] = Vec4::new(doff[0], doff[1], 0.0, 0.0);
                }
            }
        }
        Some(params)
    }
}

/// The surface type of every sample of a cell as an `R8Uint` image (`iy * 513 + ix`).
pub fn surface_image(surface: &[u8]) -> Image {
    let side = if surface.len() == GRID * GRID { GRID as u32 } else { 1 };
    let mut image = Image::default();
    image.texture_descriptor.size = Extent3d { width: side, height: side, depth_or_array_layers: 1 };
    image.texture_descriptor.dimension = TextureDimension::D2;
    image.texture_descriptor.format = TextureFormat::R8Uint;
    image.texture_descriptor.mip_level_count = 1;
    image.data = Some(surface.to_vec());
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    image
}

/// What a cell needs for the terrain material, built on the loading task.
pub struct CellSurfaces {
    pub surface: Image,
    pub params: TerrainParams,
}

impl CellSurfaces {
    /// With a library: the cell's surface map and layer table. Without (or for a cell without
    /// `leveldata.xml`): a one texel map and a table with no layers, which shades the cover colour only.
    pub fn new(library: Option<&LayerLibrary>, cell: (i32, i32), surface: &[u8], tint: Vec3) -> CellSurfaces {
        let params = library.and_then(|l| l.params(cell, tint));
        match params {
            Some(params) => CellSurfaces { surface: surface_image(surface), params },
            None => CellSurfaces { surface: surface_image(&[0]), params: TerrainParams::new(tint) },
        }
    }
}

/// A texture array image from layer after layer of `kind.mips()` mips.
fn layer_array(mut data: Vec<u8>, layers: usize, kind: Kind) -> Image {
    let mut image = Image::default();
    if layers == 0 {
        // a valid placeholder that is never indexed
        data = vec![0u8; kind.layer_bytes()];
    }
    image.texture_descriptor.size =
        Extent3d { width: kind.px(), height: kind.px(), depth_or_array_layers: layers.max(1) as u32 };
    image.texture_descriptor.dimension = TextureDimension::D2;
    image.texture_descriptor.format = kind.format();
    image.texture_descriptor.mip_level_count = kind.mips();
    image.texture_view_descriptor =
        Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::D2Array), ..default() });
    image.data = Some(data);
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        address_mode_w: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 8,
        ..default()
    });
    image
}

/// A one layer, 4 px array that is never indexed, for a material without layers.
fn dummy_array(kind: Kind) -> Image {
    let mut image = layer_array(vec![0u8; kind.block_bytes()], 1, kind);
    image.texture_descriptor.size = Extent3d { width: 4, height: 4, depth_or_array_layers: 1 };
    image.texture_descriptor.mip_level_count = 1;
    image
}

/// A projection with texture paths instead of slots.
#[derive(Clone, Debug)]
struct RawProjection {
    diffuse: Option<String>,
    normal: Option<String>,
    tile: [f32; 2],
    strength: f32,
    detail: Option<String>,
    detail_tile: [f32; 2],
    detail_scale: f32,
}

fn load_material(path: &str) -> Option<[Option<RawProjection>; PROJECTIONS]> {
    let mtl = CryMaterial::load(&format!("{path}.mtl")).ok()?;
    let pick = |m: &CryMaterial| -> Option<RawProjection> {
        let diffuse = m.texture(&TextureMap::Diffuse)?;
        let normal = m.texture(&TextureMap::Normal).or_else(|| m.texture(&TextureMap::Bump));
        let tile = diffuse.tex_mod.as_ref().map_or([1.0, 1.0], |t| [t.tile_u, t.tile_v]);
        let param = |name: &str, default: f32| m.public_param(name).and_then(|v| v.parse().ok()).unwrap_or(default);
        // %DETAIL_BUMP_MAPPING (0x8000 of the generation mask) enables the `Detail` normal map
        let detail = (m.gen_mask & 0x8000 != 0).then(|| m.texture(&TextureMap::Detail)).flatten();
        Some(RawProjection {
            diffuse: Some(diffuse.pak_path()),
            normal: normal.map(|t| t.pak_path()),
            tile,
            strength: param("DetailTextureStrength", 1.0),
            detail: detail.map(|t| t.pak_path()),
            detail_tile: [param("DetailBumpTillingU", 1.0), param("DetailBumpTillingV", 1.0)],
            detail_scale: param("DetailBumpScale", 1.0),
        })
    };
    Some(match mtl.sub_materials.as_slice() {
        [] => [None, None, pick(&mtl)],
        [x, y, z] => [pick(x), pick(y), pick(z)],
        [first, ..] => [None, None, pick(first)],
    })
}

/// One layer of an array from a DDS: `kind.px()` square, its mip chain, BC1 (diffuse) or BC5 with X in red
/// (normal maps); `None` for any other size or format. Bigger sources use their lower mips.
fn load_layer(path: &str, kind: Kind) -> Option<Vec<u8>> {
    let bytes = aa_cry::read(path).ok()?;
    let dds = Dds::parse(&bytes).ok()?;
    let px = kind.px();
    if dds.faces != 1 || dds.depth != 1 || dds.width != dds.height || !dds.width.is_power_of_two() || dds.width < px {
        return None;
    }
    let skip = (dds.width / px).trailing_zeros();
    if dds.mips < skip + kind.mips() {
        return None;
    }
    let mut out = Vec::with_capacity(kind.layer_bytes());
    for mip in skip..skip + kind.mips() {
        let src = &dds.data[dds.surface_range(0, mip)];
        match (kind, dds.format) {
            (Kind::Diffuse, DdsFormat::Bc1) => out.extend_from_slice(src),
            // the colour half of a BC3 block is a BC1 block
            (Kind::Diffuse, DdsFormat::Bc3) => src.as_chunks::<16>().0.iter().for_each(|b| out.extend_from_slice(&b[8..])),
            // x in red, y in green of a BC1 normal map
            (Kind::Normal | Kind::Detail, DdsFormat::Bc1) => src.as_chunks::<8>().0.iter().for_each(|b| {
                let texels = bc1_block(b);
                out.extend_from_slice(&bc4_block(&texels.map(|t| t[0])));
                out.extend_from_slice(&bc4_block(&texels.map(|t| t[1])));
            }),
            (Kind::Normal | Kind::Detail, DdsFormat::Bc5) => {
                if dds.swapped_rg {
                    src.as_chunks::<16>().0.iter().for_each(|b| {
                        out.extend_from_slice(&b[8..]);
                        out.extend_from_slice(&b[..8]);
                    });
                } else {
                    out.extend_from_slice(src);
                }
            }
            _ => return None,
        }
    }
    Some(out)
}

/// The 16 texels of a BC1 block (row major).
fn bc1_block(b: &[u8; 8]) -> [[u8; 3]; 16] {
    let c565 = |c: u16| {
        let (r, g, b) = ((c >> 11) & 31, (c >> 5) & 63, c & 31);
        [((r << 3) | (r >> 2)) as u8, ((g << 2) | (g >> 4)) as u8, ((b << 3) | (b >> 2)) as u8]
    };
    let (c0, c1) = (u16::from_le_bytes([b[0], b[1]]), u16::from_le_bytes([b[2], b[3]]));
    let (a, z) = (c565(c0), c565(c1));
    let mix = |w0: u32, w1: u32, d: u32| [0, 1, 2].map(|i| ((a[i] as u32 * w0 + z[i] as u32 * w1) / d) as u8);
    let palette = if c0 > c1 { [a, z, mix(2, 1, 3), mix(1, 2, 3)] } else { [a, z, mix(1, 1, 2), [0, 0, 0]] };
    let bits = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
    std::array::from_fn(|p| palette[((bits >> (2 * p)) & 3) as usize])
}

/// One BC4 block (8 byte: two endpoints, sixteen 3 bit indices) for 16 values.
fn bc4_block(v: &[u8; 16]) -> [u8; 8] {
    let (lo, hi) = (*v.iter().min().unwrap(), *v.iter().max().unwrap());
    let mut block = [0u8; 8];
    // endpoints in the 8 value mode (first > second); a flat block keeps its single value
    block[0] = hi;
    block[1] = lo;
    if hi == lo {
        return block;
    }
    let palette: [i32; 8] = std::array::from_fn(|k| match k {
        0 => hi as i32,
        1 => lo as i32,
        _ => ((8 - k as i32) * hi as i32 + (k as i32 - 1) * lo as i32) / 7,
    });
    let mut bits = 0u64;
    for (i, &x) in v.iter().enumerate() {
        let best = (0..8).min_by_key(|&k| (palette[k] - x as i32).abs()).unwrap() as u64;
        bits |= best << (3 * i);
    }
    block[2..].copy_from_slice(&bits.to_le_bytes()[..6]);
    block
}

/// `f` over `items` on a few scoped threads, results in order.
fn parallel<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(8);
    let chunk = items.len().div_ceil(threads).max(1);
    std::thread::scope(|s| {
        let f = &f;
        let handles: Vec<_> = items.chunks(chunk).map(|c| s.spawn(move || c.iter().map(f).collect::<Vec<R>>())).collect();
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
    })
}

/// Loading state of the layer library.
#[derive(Resource, Default)]
pub struct TerrainLayers {
    state: LayersState,
}

#[derive(Default)]
enum LayersState {
    /// Before the startup system ran.
    #[default]
    Pending,
    Loading(Task<Option<BuiltLayers>>),
    /// `library` is `None` when `WorldConfig::terrain_detail` is off or the library could not be built:
    /// the terrain then shows the decoded cover colour only.
    Ready {
        library: Option<Arc<LayerLibrary>>,
        /// Diffuse, normal and detail normal arrays.
        arrays: [Handle<Image>; 3],
    },
}

impl TerrainLayers {
    /// Whether the library is still being built (cells wait for it, their material needs the arrays).
    pub fn is_loading(&self) -> bool {
        !matches!(self.state, LayersState::Ready { .. })
    }

    /// The library once loaded.
    pub fn library(&self) -> Option<Arc<LayerLibrary>> {
        match &self.state {
            LayersState::Ready { library, .. } => library.clone(),
            _ => None,
        }
    }

    /// The material of a cell (`None` while the library is loading).
    pub fn material(&self, cover: Option<Handle<Image>>, surfaces: CellSurfaces, images: &mut Assets<Image>) -> Option<TerrainMaterial> {
        let LayersState::Ready { arrays, .. } = &self.state else { return None };
        // a cell without cover texture shows mid grey (a white texel would decode to yellow)
        let cover = cover.unwrap_or_else(|| {
            let mut grey = Image::new(
                Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                TextureDimension::D2,
                vec![82, 85, 128, 255],
                TextureFormat::Rgba8Unorm,
                RenderAssetUsages::RENDER_WORLD,
            );
            grey.sampler = ImageSampler::linear();
            images.add(grey)
        });
        Some(ExtendedMaterial {
            base: StandardMaterial {
                base_color: Color::WHITE,
                base_color_texture: Some(cover),
                perceptual_roughness: 1.0,
                reflectance: 0.05,
                ..default()
            },
            extension: TerrainExt {
                params: surfaces.params,
                diffuse: arrays[0].clone(),
                normals: arrays[1].clone(),
                details: arrays[2].clone(),
                surface: images.add(surfaces.surface),
            },
        })
    }
}

/// Registers the terrain material and starts building the library of the world.
pub fn build(app: &mut App, cfg: &WorldConfig) {
    embedded_asset!(app, "terrain_layers.wesl");
    app.add_plugins(MaterialPlugin::<TerrainMaterial>::default());
    let enabled = cfg.terrain_detail;
    app.init_resource::<TerrainLayers>().add_systems(
        Startup,
        move |mut layers: ResMut<TerrainLayers>, info: Res<crate::stream::WorldInfo>, mut images: ResMut<Assets<Image>>| {
            if enabled {
                let world = info.0.clone();
                layers.state = LayersState::Loading(AsyncComputeTaskPool::get().spawn(async move {
                    let built = LayerLibrary::build(&world.name, world.cells.keys().copied());
                    if built.is_none() {
                        warn!("aa-world: no terrain layers for {}", world.name);
                    }
                    built
                }));
            } else {
                layers.state = LayersState::Ready { library: None, arrays: Kind::ALL.map(|k| images.add(dummy_array(k))) };
            }
        },
    );
    app.add_systems(Update, poll_layers.before(crate::stream::stream_cells));
}

/// Moves the finished library into the asset storage.
fn poll_layers(mut layers: ResMut<TerrainLayers>, mut images: ResMut<Assets<Image>>) {
    let LayersState::Loading(task) = &mut layers.state else { return };
    let Some(result) = check_ready(task) else { return };
    layers.state = match result {
        Some(built) => LayersState::Ready {
            library: Some(Arc::new(built.library)),
            arrays: built.images.map(|image| images.add(image)),
        },
        None => LayersState::Ready { library: None, arrays: Kind::ALL.map(|k| images.add(dummy_array(k))) },
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_types_of_a_cell() {
        let xml = r#"<LevelData><SurfaceTypes>
            <SurfaceType Name="a" DetailScaleX="0.3" DetailScaleY="0.4" DetailMaterial="materials/x/a" Id="0"/>
            <SurfaceType Name="b" DetailScaleX="2" DetailScaleY="2" DetailMaterial="Materials/x/b" Id="7"><VegetationGroup Id="3"/></SurfaceType>
        </SurfaceTypes></LevelData>"#;
        let t = parse_surface_types(xml);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0], SurfaceEntry { material: "materials/x/a".into(), scale: [0.3, 0.4] });
        assert_eq!(t[1].scale, [2.0, 2.0]);
    }

    #[test]
    fn default_params_have_no_layers() {
        let p = TerrainParams::default();
        assert!(p.layers.chunks(ENTRY).all(|c| c[0].x < 0.0 && c[0].y < 0.0 && c[2].x < 0.0));
        // 512 px BC1 down to 4 px: 128 x 128 blocks of 8 bytes and the chain below
        assert_eq!(Kind::Diffuse.layer_bytes(), 174_760);
        assert_eq!(Kind::Normal.layer_bytes(), 349_520);
        assert_eq!(Kind::Detail.layer_bytes(), 87_376);
    }

    #[test]
    fn bc4_and_bc1_blocks() {
        // BC4: endpoints and indices reproduce a ramp within one step
        let ramp: [u8; 16] = std::array::from_fn(|i| (i * 16) as u8);
        let b = bc4_block(&ramp);
        let (e0, e1) = (b[0] as i32, b[1] as i32);
        assert!(e0 > e1);
        let bits = u64::from_le_bytes([b[2], b[3], b[4], b[5], b[6], b[7], 0, 0]);
        for (i, &x) in ramp.iter().enumerate() {
            let k = ((bits >> (3 * i)) & 7) as i32;
            let p = match k {
                0 => e0,
                1 => e1,
                _ => ((8 - k) * e0 + (k - 1) * e1) / 7,
            };
            assert!((p - x as i32).abs() <= 20, "texel {i}: {p} vs {x}");
        }
        assert_eq!(bc4_block(&[77; 16])[..2], [77, 77]);
        // BC1: c0 = pure red, c1 = pure blue, all indices 0 / 1
        let red = 0xF800u16.to_le_bytes();
        let blue = 0x001Fu16.to_le_bytes();
        let t = bc1_block(&[red[0], red[1], blue[0], blue[1], 0b0100_0100, 0, 0, 0]);
        assert_eq!(t[0], [255, 0, 0]);
        assert_eq!(t[1], [0, 0, 255]);
    }

    #[test]
    fn small_and_bc1_normal_maps() {
        if !crate::pak_available() {
            return;
        }
        // a DXT1 normal map is transcoded to BC5, a 1024 px one is read from its 512 px mip
        for path in ["wasteland/w_forest_white_sand_a_ddn", "field/tr_field_dirt_241a_ddn"] {
            let n = load_layer(&format!("game/textures/aa_terrain/{path}.dds"), Kind::Normal).unwrap();
            assert_eq!(n.len(), Kind::Normal.layer_bytes(), "{path}");
        }
        // a 256 px normal map only fits the detail array
        let small = "game/textures/aa_terrain/canyon/cliff_rock_b_ddn.dds";
        assert!(load_layer(small, Kind::Normal).is_none());
        assert_eq!(load_layer(small, Kind::Detail).unwrap().len(), Kind::Detail.layer_bytes());
    }

    #[test]
    fn spawn_layers() {
        if !crate::pak_available() {
            return;
        }
        // the Nuian spawn cell and its southern neighbour (rock cliff + grass)
        let built = LayerLibrary::build("main_world", [(15, 15), (15, 14)].into_iter()).unwrap();
        let lib = &built.library;
        assert!(lib.layers[0] > 5 && lib.layers[1] > 5 && lib.layers[2] > 0, "{:?}", lib.layers);
        let params = lib.params((15, 14), Vec3::ONE).unwrap();
        let entry = |s: usize, p: usize| &params.layers[(s * PROJECTIONS + p) * ENTRY..(s * PROJECTIONS + p + 1) * ENTRY];
        // nibble 3 of cell 015_014 is a rock with three projections
        assert!((0..3).all(|p| entry(3, p)[0].x >= 0.0), "{:?}", entry(3, 0));
        // nibble 1 (grass 006a) only has the ground projection, with a detail normal map at 0.75 x the layer uv
        assert!(entry(1, 0)[0].x < 0.0 && entry(1, 1)[0].x < 0.0 && entry(1, 2)[0].x >= 0.0);
        assert!(entry(1, 2)[2].x >= 0.0 && (entry(1, 2)[2].y - 0.2 * 0.75).abs() < 1e-5, "{:?}", entry(1, 2));
        for (k, kind) in Kind::ALL.into_iter().enumerate() {
            let image = &built.images[k];
            assert_eq!(image.texture_descriptor.size.depth_or_array_layers as usize, lib.layers[k]);
            assert_eq!(image.data.as_ref().unwrap().len(), lib.layers[k] * kind.layer_bytes());
        }
    }

    /// The layer uv of a position on the shared edge of two cells agrees modulo the texture period.
    #[test]
    fn layer_uv_is_continuous_across_cells() {
        if !crate::pak_available() {
            return;
        }
        let built = LayerLibrary::build("main_world", [(15, 15), (15, 14), (14, 15)].into_iter()).unwrap();
        let lib = &built.library;
        let (south, north, west) =
            (lib.params((15, 14), Vec3::ONE).unwrap(), lib.params((15, 15), Vec3::ONE).unwrap(), lib.params((14, 15), Vec3::ONE).unwrap());
        let whole = |v: f32| (v - v.round()).abs() < 2e-3;
        let mut compared = 0;
        for s in 0..SURFACE_TYPES {
            for t in 0..SURFACE_TYPES {
                // ground projection of the same layer in both cells
                let (a, b) = ((s * PROJECTIONS + 2) * ENTRY, (t * PROJECTIONS + 2) * ENTRY);
                for (lo, hi, along_x) in [(&south, &north, false), (&west, &north, true)] {
                    let (la, lb) = (lo.layers[a], hi.layers[b]);
                    if la.x < 0.0 || la.x != lb.x || la.z != lb.z || la.w != lb.w {
                        continue;
                    }
                    // `lo` is the cell before `hi`: its far edge (1024 m) meets the near edge (0 m) of `hi`
                    let (scale, off_lo, off_hi) = if along_x {
                        (la.z, lo.layers[a + 1].x, hi.layers[b + 1].x)
                    } else {
                        (la.w, lo.layers[a + 1].y, hi.layers[b + 1].y)
                    };
                    assert!(whole(1024.0 * scale + off_lo - off_hi), "{s} {t} {along_x}: {scale} {off_lo} {off_hi}");
                    compared += 1;
                }
            }
        }
        assert!(compared > 0);
    }
}
