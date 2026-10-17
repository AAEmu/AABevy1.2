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
//!   them into [`LAYER_ARRAYS`] texture arrays, copying the DDS blocks: BC1 diffuse maps at 512 px (1024 px
//!   for the maps of that size, 2048 px for the few that big: a third array each), BC5 normal and detail
//!   normal maps at 512 px (the 256 px ones stored as 2 x 2 copies) and a second normal array for what does
//!   not fit the first; each with its mip chain down to 4 px and at most 255 layers (the limit of a wgpu
//!   texture array is 256). The BC1 (`NORMAL_DXT1`) normal maps are transcoded to BC5.
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
/// Texture arrays of a world, in [`Kind`] order: diffuse, normal, large diffuse, huge diffuse, more normal maps.
pub const LAYER_ARRAYS: usize = 5;
/// A texture is `slot + FLAG * flags` in the uniform table, where the flags say which array it is in and how it
/// is stored (see [`Kind::flag`]).
const FLAG: f32 = 1000.0;
/// Detail layers fade out like `1 - (d / DETAIL_DISTANCE)^4` (`e_detail_materials_view_dist_xy`, quality 4).
pub const DETAIL_DISTANCE: f32 = 1024.0;

/// The terrain material: a `StandardMaterial` (cover colour, lighting) extended with the layers.
pub type TerrainMaterial = ExtendedMaterial<StandardMaterial, TerrainExt>;

/// Per cell data of the terrain shader (uniform, binding 100).
#[derive(Clone, Copy, Debug, ShaderType, Reflect)]
pub struct TerrainParams {
    /// Linear colour factor (`WorldConfig::terrain_tint`), `w` = detail fade distance.
    pub tint: Vec4,
    /// Four vectors per `(surface type * 3 + projection)`: `(diffuse, normal, uv scale u, v)` (a texture is
    /// `slot + 1000 * flags`: diffuse flags 0 / 1 / 2 = 512 / 1024 / 2048 px array, normal flags bit 0 =
    /// 2 x 2 copies at half the uv, bit 1 = the second array),
    /// `(uv offset u, v, DetailTextureStrength, DetailBumpScale)`,
    /// `(detail normal, detail uv scale u, v, material colour r)` and
    /// `(detail uv offset u, v, material colour g, b)`;
    /// a slot `< 0` means none.
    pub layers: [Vec4; SURFACE_TYPES * PROJECTIONS * ENTRY],
}

impl TerrainParams {
    /// A table without layers.
    pub fn new(tint: Vec3) -> Self {
        let mut layers = [Vec4::ZERO; SURFACE_TYPES * PROJECTIONS * ENTRY];
        for entry in layers.chunks_mut(ENTRY) {
            entry[0] = Vec4::new(-1.0, -1.0, 0.0, 0.0);
            entry[2] = Vec4::new(-1.0, 0.0, 0.0, 1.0);
            entry[3] = Vec4::new(0.0, 0.0, 1.0, 1.0);
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
    /// Diffuse maps, BC1 512 px.
    #[texture(101, dimension = "2d_array")]
    #[sampler(102)]
    pub diffuse: Handle<Image>,
    /// Normal and detail normal maps, BC5 512 px.
    #[texture(103, dimension = "2d_array")]
    #[sampler(104)]
    pub normals: Handle<Image>,
    /// The diffuse maps of 1024 px, BC1.
    #[texture(105, dimension = "2d_array")]
    #[sampler(106)]
    pub large: Handle<Image>,
    #[texture(107, sample_type = "u_int")]
    pub surface: Handle<Image>,
    /// The diffuse maps of 2048 px, BC1 (sampled with the diffuse sampler, the arrays all filter alike).
    #[texture(108, dimension = "2d_array")]
    pub huge: Handle<Image>,
    /// The normal maps that did not fit `normals`, BC5 512 px (sampled with the normal sampler).
    #[texture(109, dimension = "2d_array")]
    pub more_normals: Handle<Image>,
}

impl TerrainExt {
    /// The extension from the cell's table, the layer arrays (in [`Kind`] order, see [`BuiltLayers::images`]) and
    /// its surface map.
    pub fn new(params: TerrainParams, arrays: [Handle<Image>; LAYER_ARRAYS], surface: Handle<Image>) -> Self {
        let [diffuse, normals, large, huge, more_normals] = arrays;
        TerrainExt { params, diffuse, normals, large, huge, more_normals, surface }
    }
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
    /// BC5 (x, y), 512 px: normal maps and detail normal maps.
    Normal,
    /// BC1, 1024 px: the diffuse maps of 1024 px.
    Large,
    /// BC1, 2048 px: the diffuse maps of 2048 px or more.
    Huge,
    /// BC5, 512 px: the normal maps that did not fit the first normal array.
    NormalMore,
}

impl Kind {
    const ALL: [Kind; LAYER_ARRAYS] = [Kind::Diffuse, Kind::Normal, Kind::Large, Kind::Huge, Kind::NormalMore];

    /// Edge of a layer (px). Larger sources use their lower mips.
    fn px(self) -> u32 {
        match self {
            Kind::Large => 1024,
            Kind::Huge => 2048,
            _ => 512,
        }
    }

    /// The flags of a texture in this array in the uniform table: diffuse 0 / 1 / 2 for the 512 / 1024 / 2048 px
    /// array; normal bit 1 for the second array (bit 0 is for 2 x 2 copies).
    fn flag(self) -> u8 {
        match self {
            Kind::Large => 1,
            Kind::Huge | Kind::NormalMore => 2,
            _ => 0,
        }
    }

    /// Mip levels of a layer: down to 4 px, where the DDS files of the pak stop.
    fn mips(self) -> u32 {
        self.px().trailing_zeros() - 1
    }

    fn is_normal(self) -> bool {
        matches!(self, Kind::Normal | Kind::NormalMore)
    }

    fn format(self) -> TextureFormat {
        if self.is_normal() { TextureFormat::Bc5RgUnorm } else { TextureFormat::Bc1RgbaUnorm }
    }

    fn block_bytes(self) -> usize {
        if self.is_normal() { 16 } else { 8 }
    }

    /// Bytes of one layer with its whole mip chain.
    fn layer_bytes(self) -> usize {
        (0..self.mips()).map(|m| ((self.px() >> m).max(1) as usize).div_ceil(4).pow(2) * self.block_bytes()).sum()
    }
}

/// One projection of a surface material (x, y or z sub-material).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Projection {
    diffuse: Option<Tex>,
    normal: Option<Tex>,
    /// `TexMod` tiling of the diffuse map.
    tile: [f32; 2],
    /// `DetailTextureStrength`.
    strength: f32,
    /// The material's `Diffuse` colour (`MatDifColor`, linear), a factor of the lit layer.
    color: [f32; 3],
    /// The detail normal map (`Detail` texture, `DETAIL_BUMP_MAPPING`), its tiling relative to the layer
    /// uv (`DetailBumpTillingU / V`) and `DetailBumpScale`.
    detail: Option<Tex>,
    detail_tile: [f32; 2],
    detail_scale: f32,
}

/// A layer of an array: the slot and the flags that find the array (see [`FLAG`]).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Tex {
    slot: u16,
    flags: u8,
}

impl Tex {
    /// The value in the uniform table.
    fn code(self) -> f32 {
        self.slot as f32 + FLAG * self.flags as f32
    }
}

/// The array and slot of the next layer of `kind` (normal maps overflow into the second normal array); `None`
/// when the array is full.
fn place(counts: &mut [usize; LAYER_ARRAYS], kind: Kind, copies: bool) -> Option<(Kind, Tex)> {
    let kind = if kind == Kind::Normal && counts[kind as usize] >= MAX_LAYERS { Kind::NormalMore } else { kind };
    let count = &mut counts[kind as usize];
    if *count >= MAX_LAYERS {
        return None;
    }
    let tex = Tex { slot: *count as u16, flags: kind.flag() + copies as u8 };
    *count += 1;
    Some((kind, tex))
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
    /// Layers in each array (diffuse, normal, large, huge, more normal maps).
    pub layers: [usize; LAYER_ARRAYS],
}

/// What the loading task returns: the library and the three array images.
pub struct BuiltLayers {
    pub library: LayerLibrary,
    /// The arrays: diffuse (512 px), normal and detail normal (512 px), large diffuse (1024 px), huge diffuse
    /// (2048 px) and more normal maps (512 px).
    pub images: [Image; LAYER_ARRAYS],
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
        // diffuse maps, and normal maps with detail normal maps (they share an array)
        let mut wants: [HashMap<String, usize>; 2] = Default::default();
        for (mat, raw) in loaded.iter().enumerate() {
            let Some(raw) = raw else { continue };
            for pr in raw.iter().flatten() {
                for (set, path) in [(0, &pr.diffuse), (1, &pr.normal), (1, &pr.detail)] {
                    if let Some(path) = path {
                        let seen = wants[set].entry(path.clone()).or_default();
                        *seen = (*seen).max(mat_cells[mat]);
                    }
                }
            }
        }

        // 3. read the textures (the most used first) into the arrays, as many as fit
        let mut slots: [HashMap<String, Tex>; 2] = Default::default();
        let mut data: [Vec<u8>; LAYER_ARRAYS] = Default::default();
        let mut layers = [0usize; LAYER_ARRAYS];
        for (set, normal) in [(0, false), (1, true)] {
            let mut paths: Vec<(&String, &usize)> = wants[set].iter().collect();
            paths.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            let paths: Vec<String> = paths.into_iter().map(|(p, _)| p.clone()).collect();
            let loaded = parallel(&paths, |p| load_layer(p, normal));
            let mut dropped = 0;
            for (path, layer) in paths.into_iter().zip(loaded) {
                let Some(layer) = layer else {
                    dropped += 1;
                    continue;
                };
                let Some((kind, tex)) = place(&mut layers, layer.kind, layer.copies) else {
                    dropped += 1;
                    continue;
                };
                slots[set].insert(path, tex);
                data[kind as usize].extend_from_slice(&layer.data);
            }
            if dropped > 0 {
                debug!("aa-world: terrain layers: {dropped} unusable {} textures", ["diffuse", "normal"][set]);
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
                        let slot = |set: usize, path: &Option<String>| path.as_ref().and_then(|t| slots[set].get(t).copied());
                        let diffuse = slot(0, &pr.diffuse);
                        if diffuse.is_some() {
                            m.proj[p] = Some(Projection {
                                diffuse,
                                normal: slot(1, &pr.normal),
                                tile: pr.tile,
                                strength: pr.strength,
                                color: pr.color,
                                detail: slot(1, &pr.detail),
                                detail_tile: pr.detail_tile,
                                detail_scale: pr.detail_scale,
                            });
                        }
                    }
                }
                m
            })
            .collect();
        info!(
            "aa-world: terrain layers of {world}: {} cells, {} materials, {} + {} + {} diffuse (512 / 1024 / 2048 px) + {} + {} normal maps ({:.0} MB), {:.1} s",
            tables.len(),
            mats.len(),
            layers[Kind::Diffuse as usize],
            layers[Kind::Large as usize],
            layers[Kind::Huge as usize],
            layers[Kind::Normal as usize],
            layers[Kind::NormalMore as usize],
            data.iter().map(Vec::len).sum::<usize>() as f32 / 1e6,
            started.elapsed().as_secs_f32()
        );
        let mut data = data.into_iter();
        let images = Kind::ALL.map(|kind| layer_array(data.next().unwrap(), layers[kind as usize], kind));
        Some(BuiltLayers { library: LayerLibrary { mats, cells: tables, layers }, images })
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
                params.layers[i] = Vec4::new(proj.diffuse.map_or(-1.0, Tex::code), proj.normal.map_or(-1.0, Tex::code), su, sv);
                params.layers[i + 1] = Vec4::new(off[0], off[1], proj.strength, proj.detail_scale);
                if let Some(detail) = proj.detail {
                    // the detail map is tiled over the layer uv: `uv * tiling`
                    let (du, dv) = (su * proj.detail_tile[0], sv * proj.detail_tile[1]);
                    let doff = offset(p, du, dv);
                    params.layers[i + 2] = Vec4::new(detail.code(), du, dv, 0.0);
                    params.layers[i + 3] = Vec4::new(doff[0], doff[1], 0.0, 0.0);
                }
                params.layers[i + 2].w = proj.color[0];
                params.layers[i + 3].z = proj.color[1];
                params.layers[i + 3].w = proj.color[2];
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
fn layer_array(data: Vec<u8>, layers: usize, kind: Kind) -> Image {
    if layers == 0 {
        // a small placeholder that is never indexed
        return dummy_array(kind);
    }
    let mut image = Image::default();
    image.texture_descriptor.size =
        Extent3d { width: kind.px(), height: kind.px(), depth_or_array_layers: layers as u32 };
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
    color: [f32; 3],
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
            color: m.diffuse.to_array(),
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

/// A texture ready for an array.
struct Layer {
    kind: Kind,
    /// A normal map stored as 2 x 2 copies of a half size source (the shader samples it at half the uv).
    copies: bool,
    /// `kind.layer_bytes()` of blocks.
    data: Vec<u8>,
}

/// One layer of an array from a DDS, BC1 (diffuse) or BC5 with X in red (normal maps), with its mip chain.
/// A diffuse map goes to the array of its size (512, 1024 or 2048 px); a normal map to the 512 px array, where a
/// 256 px one is stored as 2 x 2 copies (the shader samples those at half the uv, which gives the original back).
/// `None` for any other size or format. Bigger sources use their lower mips.
fn load_layer(path: &str, normal: bool) -> Option<Layer> {
    let bytes = aa_cry::read(path).ok()?;
    let dds = Dds::parse(&bytes).ok()?;
    if dds.faces != 1 || dds.depth != 1 || dds.width != dds.height || !dds.width.is_power_of_two() {
        return None;
    }
    let kind = match (normal, dds.width) {
        (true, _) => Kind::Normal,
        (false, w) if w >= Kind::Huge.px() => Kind::Huge,
        (false, w) if w >= Kind::Large.px() => Kind::Large,
        (false, _) => Kind::Diffuse,
    };
    let copies = normal && dds.width * 2 == kind.px();
    if dds.width < kind.px() && !copies {
        return None;
    }
    let skip = if copies { 0 } else { (dds.width / kind.px()).trailing_zeros() };
    // copies have one mip less to read: their last one (4 px) repeats the 4 px level of the source
    let mips = kind.mips() - copies as u32;
    if dds.mips < skip + mips {
        return None;
    }
    let mut data = Vec::with_capacity(kind.layer_bytes());
    let mut level = Vec::new();
    for mip in skip..skip + mips {
        level = convert(&dds.data[dds.surface_range(0, mip)], &dds, kind)?;
        if copies {
            let blocks = ((dds.width >> mip).max(4) / 4) as usize;
            let row = blocks * kind.block_bytes();
            for _ in 0..2 {
                for r in level.chunks(row) {
                    data.extend_from_slice(r);
                    data.extend_from_slice(r);
                }
            }
        } else {
            data.extend_from_slice(&level);
        }
    }
    if copies {
        data.extend_from_slice(&level);
    }
    debug_assert_eq!(data.len(), kind.layer_bytes());
    Some(Layer { kind, copies, data })
}

/// The blocks of one mip level as the array of `kind` wants them: BC1, or BC5 with x in red.
fn convert(src: &[u8], dds: &Dds, kind: Kind) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(src.len());
    match (kind.is_normal(), dds.format) {
        (false, DdsFormat::Bc1) => out.extend_from_slice(src),
        // the colour half of a BC3 block is a BC1 block
        (false, DdsFormat::Bc3) => src.as_chunks::<16>().0.iter().for_each(|b| out.extend_from_slice(&b[8..])),
        // x in red, y in green of a BC1 normal map
        (true, DdsFormat::Bc1) => src.as_chunks::<8>().0.iter().for_each(|b| {
            let texels = bc1_block(b);
            out.extend_from_slice(&bc4_block(&texels.map(|t| t[0])));
            out.extend_from_slice(&bc4_block(&texels.map(|t| t[1])));
        }),
        (true, DdsFormat::Bc5) => {
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
        /// The arrays, see [`BuiltLayers::images`].
        arrays: [Handle<Image>; LAYER_ARRAYS],
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
            extension: TerrainExt::new(surfaces.params, arrays.clone(), images.add(surfaces.surface)),
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
        assert_eq!(Kind::Large.layer_bytes(), 699_048);
        assert_eq!(Kind::Huge.layer_bytes(), 2_796_200);
        assert_eq!(Kind::NormalMore.layer_bytes(), Kind::Normal.layer_bytes());
    }

    /// The table code of a texture and the overflow of a full array.
    #[test]
    fn slots_and_flags() {
        let mut counts = [0usize; LAYER_ARRAYS];
        let (kind, tex) = place(&mut counts, Kind::Huge, false).unwrap();
        assert_eq!((kind, tex.code()), (Kind::Huge, 2000.0));
        assert_eq!(place(&mut counts, Kind::Large, false).unwrap().1.code(), 1000.0);
        assert_eq!(place(&mut counts, Kind::Large, false).unwrap().1.code(), 1001.0);
        assert_eq!(place(&mut counts, Kind::Normal, true).unwrap().1.code(), 1000.0);
        // a full first normal array continues in the second, a full other array drops
        counts[Kind::Normal as usize] = MAX_LAYERS;
        assert_eq!(place(&mut counts, Kind::Normal, false).map(|(k, t)| (k, t.code())), Some((Kind::NormalMore, 2000.0)));
        assert_eq!(place(&mut counts, Kind::Normal, true).map(|(k, t)| (k, t.code())), Some((Kind::NormalMore, 3001.0)));
        counts[Kind::NormalMore as usize] = MAX_LAYERS;
        assert!(place(&mut counts, Kind::Normal, false).is_none());
        counts[Kind::Diffuse as usize] = MAX_LAYERS;
        assert!(place(&mut counts, Kind::Diffuse, false).is_none());
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
            let n = load_layer(&format!("game/textures/aa_terrain/{path}.dds"), true).unwrap();
            assert_eq!((n.kind, n.copies, n.data.len()), (Kind::Normal, false, Kind::Normal.layer_bytes()), "{path}");
        }
        // a 256 px normal map is stored as 2 x 2 copies: the first row of blocks is the source's row twice
        let small = "game/textures/aa_terrain/canyon/cliff_rock_b_ddn.dds";
        let n = load_layer(small, true).unwrap();
        assert_eq!((n.kind, n.copies, n.data.len()), (Kind::Normal, true, Kind::Normal.layer_bytes()));
        let row = 64 * Kind::Normal.block_bytes();
        assert_eq!(n.data[..row], n.data[row..2 * row]);
        // a 256 px map is no diffuse map
        assert!(load_layer(small, false).is_none());
    }

    /// Diffuse maps go to the array of their size: the 2048 px ground of Austera to the huge one, 1024 px maps to
    /// the large one, 512 px maps to the normal one.
    #[test]
    fn large_diffuse_maps() {
        if !crate::pak_available() {
            return;
        }
        for (path, kind) in [
            ("canyon/cliff_grey_with_dry_bushes", Kind::Huge),
            ("canyon/cliff_is7", Kind::Large),
            ("canyon/w_cliff_rock_02", Kind::Diffuse),
        ] {
            let d = load_layer(&format!("game/textures/aa_terrain/{path}.dds"), false).unwrap();
            assert_eq!((d.kind, d.copies, d.data.len()), (kind, false, kind.layer_bytes()), "{path}");
        }
    }

    /// The material's `Diffuse` colour reaches the table (the Royal Palace court yard is `tr_af_pavingstone_001b`,
    /// 0.82 / 0.82 / 0.78), and a layer without one keeps a white factor.
    #[test]
    fn material_colour_in_the_table() {
        if !crate::pak_available() {
            return;
        }
        let built = LayerLibrary::build("main_world", [(13, 10)].into_iter()).unwrap();
        let params = built.library.params((13, 10), Vec3::ONE).unwrap();
        let entry = (13 * PROJECTIONS + 2) * ENTRY;
        let colour = Vec3::new(params.layers[entry + 2].w, params.layers[entry + 3].z, params.layers[entry + 3].w);
        assert!((colour - Vec3::new(0.819_963_8, 0.819_963_8, 0.781_750_8)).abs().max_element() < 1e-4, "{colour}");
        // a projection the material does not have stays white
        let unused = (13 * PROJECTIONS) * ENTRY;
        assert_eq!((params.layers[unused + 2].w, params.layers[unused + 3].zw()), (1.0, Vec2::ONE));
    }

    #[test]
    fn spawn_layers() {
        if !crate::pak_available() {
            return;
        }
        // the Nuian spawn cell and its southern neighbour (rock cliff + grass)
        let built = LayerLibrary::build("main_world", [(15, 15), (15, 14)].into_iter()).unwrap();
        let lib = &built.library;
        assert!(lib.layers[0] > 5 && lib.layers[1] > 5, "{:?}", lib.layers);
        let params = lib.params((15, 14), Vec3::ONE).unwrap();
        let entry = |s: usize, p: usize| &params.layers[(s * PROJECTIONS + p) * ENTRY..(s * PROJECTIONS + p + 1) * ENTRY];
        // nibble 3 of cell 015_014 is a rock with three projections
        assert!((0..3).all(|p| entry(3, p)[0].x >= 0.0), "{:?}", entry(3, 0));
        // nibble 1 (grass 006a) only has the ground projection, with a detail normal map at 0.75 x the layer uv
        assert!(entry(1, 0)[0].x < 0.0 && entry(1, 1)[0].x < 0.0 && entry(1, 2)[0].x >= 0.0);
        assert!(entry(1, 2)[2].x >= 0.0 && (entry(1, 2)[2].y - 0.2 * 0.75).abs() < 1e-5, "{:?}", entry(1, 2));
        for (k, kind) in Kind::ALL.into_iter().enumerate() {
            let image = &built.images[k];
            if lib.layers[k] == 0 {
                // an array without layers is a 4 px placeholder
                assert_eq!(image.texture_descriptor.size.width, 4);
                continue;
            }
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
