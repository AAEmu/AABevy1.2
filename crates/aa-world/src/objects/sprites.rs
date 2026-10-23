//! Vegetation sprites: far trees, bushes and rocks drawn as camera-facing impostors instead of meshes.
//!
//! What the 1.2 engine does (Cry3DEngine 1.2, see `docs/aa-world.md`, "Vegetation sprites"):
//!
//! * `StatInstGroup::Update` (30109F70) gives every vegetation group with `bUseSprites` a switch distance
//!   `max(18 x fVegRadius x fSize x max(fSpriteDistRatio, e_vegetation_sprites_distance_custom_ratio_min)
//!   x e_vegetation_sprites_distance_ratio x max(width / 1024, 1), e_vegetation_sprites_min_distance)`;
//!   `fVegRadius = max(radiusHors, radiusVert)` of the LOD0 object (`CStatObj` 3001BA80). An instance switches at
//!   that distance times its scale (300F0C10). Client defaults (`option_view_dist_ratio_vegetation` group 4):
//!   ratio 0.8, custom minimum 0.01, minimum distance 8 m.
//! * `CVegetation::Render` (300F36E0) draws the sprite instead of the mesh beyond the switch distance, while the
//!   sprite is at most `e_vegetation_sprite_max_pixel` (1.5) times `r_VegetationSpritesTexRes` (128) on screen;
//!   the sprite (300F2C60) is a quad of half size `radiusHors x radiusVert`, picks one of 24 views around the up
//!   axis from the view angle plus the instance rotation, and is alpha tested. Up to the instance's view
//!   distance; past it the far-plane dissolve ([`super::dissolve`]) applies as for the mesh.
//!
//! Here each sprite-using model gets an atlas of [`ANGLES`] side views, rasterised on the CPU from its LOD0 mesh
//! (diffuse texture x diffuse colour x vertex-alpha AO, plus the surface normal for lighting) on the model
//! worker. All instances of a model share one quad mesh and material, drawn by [`SpriteMaterial`] which faces
//! every quad to the camera, picks the view, lights it like the meshes and crossfades with the mesh over the
//! dissolve band.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use aa_cry::CryLoader;
use aa_cry::material::{Material as CryMaterial, TextureMap};
use aa_cry::mesh::Model;
use bevy::asset::{RenderAssetUsages, embedded_asset};
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::RenderLayers;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
    TextureDimension, TextureFormat,
};
use bevy::shader::ShaderRef;

use crate::coords::CRY_TO_BEVY;

/// Views around the up axis (the native 24).
pub const ANGLES: usize = 24;
/// Texels of one view (`r_VegetationSpritesTexRes`, default group 4).
const RES: usize = 128;
/// Empty border of a view (texels), so filtering and the smaller mips never pull in the neighbouring view.
const PAD: usize = 4;
/// Supersampling of the rasteriser per axis (coverage becomes alpha).
const SUPER: usize = 2;
const COLS: usize = 6;
const ROWS: usize = 4;
/// Mip levels of the atlas (128 down to 8 texels per view).
const MIPS: usize = 5;
/// Alpha test of the sprite (coverage of the supersampled silhouette).
const ALPHA_REF: f32 = 0.4;

/// `e_vegetation_sprites_distance_ratio` (default group 4).
const DISTANCE_RATIO: f32 = 0.8;
/// `e_vegetation_sprites_distance_custom_ratio_min` (default group 4).
const CUSTOM_RATIO_MIN: f32 = 0.01;
/// `e_vegetation_sprites_min_distance`.
const MIN_DISTANCE: f32 = 8.0;

/// Only one normal perspective world camera drives the mesh/sprite handoff.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct View {
    width: f32,
    height: f32,
    fov: f32,
}

pub(super) fn select_view<'a>(
    cameras: impl Iterator<
        Item = (
            &'a Camera,
            &'a GlobalTransform,
            &'a Projection,
            Option<&'a RenderLayers>,
        ),
    >,
) -> Option<View> {
    let mut cameras = cameras.filter(|(c, _, _, layers)| {
        c.is_active && layers.is_none_or(|l| l.intersects(&RenderLayers::default()))
    });
    let (camera, _, projection, _) = cameras.next()?;
    if cameras.next().is_some() {
        return None;
    }
    let Projection::Perspective(projection) = projection else {
        return None;
    };
    let size = camera.physical_viewport_size()?;
    let fov = projection.fov;
    (size.x > 0 && size.y > 0 && fov.is_finite() && fov > 0.0 && fov < std::f32::consts::PI)
        .then_some(View {
            width: size.x as f32,
            height: size.y as f32,
            fov,
        })
}

impl View {
    pub(super) fn switch(self, radius: f32, size: f32, ratio: f32, scale: f32) -> f32 {
        // Native 300F3B.. also keeps the mesh while its angular diameter exceeds
        // e_vegetation_sprite_max_pixel (1.5) * r_VegetationSpritesTexRes (128).
        // Use the enclosing radius for a conservative screen-size guard.
        let horizontal_fov = 2.0 * ((self.fov * 0.5).tan() * self.width / self.height).atan();
        let pixel_angle = (self.fov / self.height).min(horizontal_fov / self.width);
        let pixel_distance = radius / (pixel_angle * (1.5 * RES as f32) * 0.5).min(1.5).tan();
        switch_distance(radius, size, ratio, self.width).max(pixel_distance) * scale
    }
}
/// The native sprite switch distance of a group (before the instance scale).
pub fn switch_distance(radius: f32, size: f32, sprite_ratio: f32, screen_width: f32) -> f32 {
    let d = 18.0
        * radius
        * size
        * sprite_ratio.max(CUSTOM_RATIO_MIN)
        * DISTANCE_RATIO
        * (screen_width / 1024.0).max(1.0);
    d.max(MIN_DISTANCE)
}

/// How far from a sector an instance keeps its mesh (hidden beyond `switch` by its visibility range): its switch
/// distance plus a sector, rounded up to whole sectors so a sector's instances share few thresholds and the
/// sector is rebuilt rarely when the camera moves.
pub fn mesh_reach(switch: f32) -> f32 {
    ((switch + 64.0) / 64.0).ceil() * 64.0
}

/// A baked atlas (CPU side), shared by every LOD of a model.
pub struct Baked {
    /// RGBA8 sRGB mips: albedo and coverage.
    albedo: Vec<Vec<u8>>,
    /// RG8 mips: view-space normal x / y.
    normal: Vec<Vec<u8>>,
    /// `fVegRadius`: max(horizontal, vertical radius).
    pub radius: f32,
    /// Half width (`radiusHors`, around the model origin) and half height (`radiusVert`) of the quad.
    pub half_width: f32,
    pub half_height: f32,
    /// Height of the quad's centre above the model origin.
    pub center_z: f32,
    /// Coverage-weighted roughness of the drawn materials.
    pub roughness: f32,
}

type BakeCell = Arc<OnceLock<Option<Arc<Baked>>>>;

/// Shared by a world's model jobs, released with ObjectState (including failed bakes).
#[derive(Default)]
pub(crate) struct Bakes(Mutex<HashMap<(String, Option<String>), BakeCell>>);

/// The atlas of a model (baked once from its LOD0, on the calling worker; later calls wait for or reuse it).
/// `lod0` is the already parsed LOD0 model when the caller has it.
pub(crate) fn baked(
    cache: &Bakes,
    loader: &CryLoader,
    path: &str,
    material: Option<&str>,
    lod0: Option<&Model>,
    mtl_path: Option<&str>,
) -> Option<Arc<Baked>> {
    let cell = cache
        .0
        .lock()
        .unwrap()
        .entry((path.to_string(), material.map(str::to_string)))
        .or_default()
        .clone();
    cell.get_or_init(|| {
        let owned;
        let model = match lod0 {
            Some(m) => m,
            None => {
                owned = Model::load(path).ok()?;
                &owned
            }
        };
        let mtl = loader.mtl(mtl_path?)?;
        let mut textures = HashMap::new();
        bake(model, Some(&mtl), &mut |file: &str| {
            textures
                .entry(file.to_string())
                .or_insert_with(|| load_texture(loader, file))
                .clone()
        })
        .map(Arc::new)
    })
    .clone()
}

/// RGBA8 of a texture at a mip of at most 512 texels (enough for a 256 texel view), shared by the triangles.
type Texture = Option<Arc<(usize, usize, Vec<u8>)>>;

fn load_texture(loader: &CryLoader, file: &str) -> Texture {
    let (mut w, mut h, mut px) = loader.decode_rgba8(file, 2)?;
    if w < 128 || h < 128 {
        (w, h, px) = loader.decode_rgba8(file, 0)?;
    }
    Some(Arc::new((w as usize, h as usize, px)))
}

/// One drawn triangle corner in model space.
#[derive(Clone, Copy)]
struct Corner {
    p: Vec3,
    n: Vec3,
    uv: Vec2,
    ao: f32,
}

/// What a subset draws with.
struct Surface {
    texture: Texture,
    /// Linear diffuse colour (x `DiffuseModifierInHDR`).
    color: Vec3,
    /// Alpha test of the texture's alpha, `None` for opaque materials.
    alpha_ref: Option<f32>,
    uv: (Vec2, Vec2, f32),
    roughness: f32,
}

fn srgb_to_linear(v: u8) -> f32 {
    let c = v as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0 + 0.5) as u8
}

fn surface(m: &CryMaterial, texture: &mut dyn FnMut(&str) -> Texture) -> Option<Surface> {
    let shader = m.shader.to_ascii_lowercase();
    if m.no_draw() || shader == "cut" {
        return None;
    }
    let modifier = m
        .public_param("DiffuseModifierInHDR")
        .and_then(|v| v.trim().parse::<f32>().ok())
        .unwrap_or(1.0);
    let diffuse = m.texture(&TextureMap::Diffuse);
    let alpha_ref = match shader.as_str() {
        "vegetation" | "vegetationleaves" | "grass" => Some(if m.alpha_test > 0.0 {
            m.alpha_test
        } else {
            0.5
        }),
        _ => (m.alpha_test > 0.0).then_some(m.alpha_test),
    };
    let uv = diffuse
        .and_then(|t| t.tex_mod.as_ref())
        .map_or((Vec2::ONE, Vec2::ZERO, 0.0), |t| {
            (
                Vec2::new(t.tile_u, t.tile_v),
                Vec2::new(t.offset_u, t.offset_v),
                t.rotate_w.to_radians(),
            )
        });
    Some(Surface {
        texture: diffuse.and_then(|t| texture(&t.file)),
        color: m.diffuse * modifier,
        alpha_ref,
        uv,
        // aa-cry's shininess_to_roughness, rounded: a leaf (shininess 10 .. 30) is a rough surface
        roughness: (1.0 - (m.shininess / 255.0).clamp(0.0, 1.0).sqrt()).clamp(0.3, 1.0),
    })
}

impl Surface {
    /// Linear colour and alpha at a texture coordinate (nearest texel, repeating).
    fn sample(&self, uv: Vec2) -> (Vec3, f32) {
        let Some(t) = &self.texture else {
            return (self.color, 1.0);
        };
        let (tile, offset, rot) = self.uv;
        let uv = bevy::math::Affine2::from_scale_angle_translation(tile, rot, offset)
            .transform_point2(uv);
        let (w, h, px) = (&t.0, &t.1, &t.2);
        let x = ((uv.x.rem_euclid(1.0) * *w as f32) as usize).min(w - 1);
        let y = ((uv.y.rem_euclid(1.0) * *h as f32) as usize).min(h - 1);
        let p = &px[(y * w + x) * 4..][..4];
        let rgb = Vec3::new(
            srgb_to_linear(p[0]),
            srgb_to_linear(p[1]),
            srgb_to_linear(p[2]),
        );
        (rgb * self.color, p[3] as f32 / 255.0)
    }
}

/// Rasterises the 24 side views of the model's LOD0 render nodes. `None` when nothing is drawn.
pub fn bake(
    model: &Model,
    mtl: Option<&CryMaterial>,
    texture: &mut dyn FnMut(&str) -> Texture,
) -> Option<Baked> {
    // triangles with their surface
    let mut surfaces: Vec<Surface> = Vec::new();
    let mut by_material: HashMap<u32, Option<usize>> = HashMap::new();
    let mut tris: Vec<([Corner; 3], usize)> = Vec::new();
    for ni in model.lod_nodes(0) {
        let Some(mi) = model.nodes[ni].mesh else {
            continue;
        };
        let mesh = &model.meshes[mi];
        if mesh.skin.is_some() {
            continue;
        }
        let xf = super::node_world(model, ni);
        for subset in &mesh.subsets {
            let surface = *by_material.entry(subset.material_id).or_insert_with(|| {
                let m = mtl.map(|m| m.sub(subset.material_id as usize).unwrap_or(m));
                let s = match m {
                    Some(m) => surface(m, texture)?,
                    None => Surface {
                        texture: None,
                        color: Vec3::splat(0.5),
                        alpha_ref: None,
                        uv: (Vec2::ONE, Vec2::ZERO, 0.0),
                        roughness: 0.8,
                    },
                };
                // vegetation.cfx: the normalised vertex alpha multiplies the lit colour (see vegetation_color/ao.rs)
                surfaces.push(s);
                Some(surfaces.len() - 1)
            });
            let Some(surface) = surface else { continue };
            let ao = mtl
                .map(|m| m.sub(subset.material_id as usize).unwrap_or(m))
                .is_some_and(|m| {
                    m.shader.eq_ignore_ascii_case("Vegetation") && m.gen_mask & 0x4000_0000 == 0
                })
                && mesh.colors.len() == mesh.positions.len();
            let end = (subset.first_index + subset.index_count) as usize;
            let Some(indices) = mesh.indices.get(subset.first_index as usize..end) else {
                continue;
            };
            for t in indices.chunks_exact(3) {
                let corner = |i: u32| {
                    let i = i as usize;
                    Corner {
                        p: xf.transform_point3(mesh.positions[i]),
                        n: xf
                            .transform_vector3(mesh.normals.get(i).copied().unwrap_or(Vec3::Z))
                            .normalize_or(Vec3::Z),
                        uv: mesh.uvs.get(i).copied().unwrap_or(Vec2::ZERO),
                        ao: if ao {
                            mesh.colors[i][3] as f32 / 255.0
                        } else {
                            1.0
                        },
                    }
                };
                tris.push(([corner(t[0]), corner(t[1]), corner(t[2])], surface));
            }
        }
    }
    if tris.is_empty() {
        return None;
    }
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for (c, _) in &tris {
        for c in c {
            lo = lo.min(c.p);
            hi = hi.max(c.p);
        }
    }
    // CStatObj radiuses (3001BA80): horizontal around the origin, vertical half the height
    let half_width = Vec2::new(lo.x.abs().max(hi.x.abs()), lo.y.abs().max(hi.y.abs())).length();
    let half_height = (hi.z - lo.z) * 0.5;
    if !(half_width > 0.01 && half_height > 0.01)
        || !half_width.is_finite()
        || !half_height.is_finite()
    {
        return None;
    }

    let w = RES * SUPER;
    let inner = (RES - 2 * PAD) * SUPER;
    let pad = PAD * SUPER;
    let (aw, ah) = (COLS * RES, ROWS * RES);
    let mut color = vec![Vec3::ZERO; aw * ah];
    let mut cover = vec![0.0f32; aw * ah];
    let mut normal = vec![Vec3::ZERO; aw * ah];
    let mut rough = (0.0f32, 0.0f32);
    let mut depth = vec![f32::MIN; w * w];
    let mut texel: Vec<Option<(Vec3, Vec3)>> = vec![None; w * w];
    for k in 0..ANGLES {
        let phi = std::f32::consts::TAU * k as f32 / ANGLES as f32;
        let toward = Vec3::new(phi.cos(), phi.sin(), 0.0);
        let right = (-toward).cross(Vec3::Z);
        depth.fill(f32::MIN);
        texel.fill(None);
        let project = |p: Vec3| {
            Vec3::new(
                pad as f32 + (p.dot(right) + half_width) / (2.0 * half_width) * inner as f32,
                pad as f32 + (hi.z - p.z) / (2.0 * half_height) * inner as f32,
                p.dot(toward),
            )
        };
        for (c, si) in &tris {
            let s = &surfaces[*si];
            let v = c.map(|c| project(c.p));
            let area =
                (v[1].x - v[0].x) * (v[2].y - v[0].y) - (v[2].x - v[0].x) * (v[1].y - v[0].y);
            if area.abs() < 1e-8 {
                continue;
            }
            let x0 = v
                .iter()
                .map(|v| v.x)
                .fold(f32::MAX, f32::min)
                .floor()
                .max(0.0) as usize;
            let x1 = (v.iter().map(|v| v.x).fold(f32::MIN, f32::max).ceil() as usize).min(w);
            let y0 = v
                .iter()
                .map(|v| v.y)
                .fold(f32::MAX, f32::min)
                .floor()
                .max(0.0) as usize;
            let y1 = (v.iter().map(|v| v.y).fold(f32::MIN, f32::max).ceil() as usize).min(w);
            for y in y0..y1 {
                for x in x0..x1 {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    let edge = |a: Vec3, b: Vec3| {
                        ((b.x - a.x) * (py - a.y) - (px - a.x) * (b.y - a.y)) / area
                    };
                    let (b0, b1, b2) = (edge(v[1], v[2]), edge(v[2], v[0]), edge(v[0], v[1]));
                    if b0 < 0.0 || b1 < 0.0 || b2 < 0.0 {
                        continue;
                    }
                    let z = b0 * v[0].z + b1 * v[1].z + b2 * v[2].z;
                    let i = y * w + x;
                    if z <= depth[i] {
                        continue;
                    }
                    let uv = c[0].uv * b0 + c[1].uv * b1 + c[2].uv * b2;
                    let (rgb, a) = s.sample(uv);
                    if s.alpha_ref.is_some_and(|r| a < r) {
                        continue;
                    }
                    let n = (c[0].n * b0 + c[1].n * b1 + c[2].n * b2).normalize_or(toward);
                    let mut n = Vec3::new(n.dot(right), n.z, n.dot(toward));
                    // two-sided leaves and the inside of a crown face the viewer
                    if n.z < 0.0 {
                        n = -n;
                    }
                    let ao = c[0].ao * b0 + c[1].ao * b1 + c[2].ao * b2;
                    depth[i] = z;
                    texel[i] = Some((rgb * ao, n));
                    rough.0 += s.roughness;
                    rough.1 += 1.0;
                }
            }
        }
        occlude(&mut texel, &depth, w, inner as f32, half_width);
        // supersampled view -> atlas cell: coverage is alpha, colour and normal the mean of the covered samples
        let (cx, cy) = ((k % COLS) * RES, (k / COLS) * RES);
        for y in 0..RES {
            for x in 0..RES {
                let (mut c, mut n, mut count) = (Vec3::ZERO, Vec3::ZERO, 0);
                for sy in 0..SUPER {
                    for sx in 0..SUPER {
                        if let Some((rgb, nn)) = texel[(y * SUPER + sy) * w + x * SUPER + sx] {
                            c += rgb;
                            n += nn;
                            count += 1;
                        }
                    }
                }
                if count > 0 {
                    let o = (cy + y) * aw + cx + x;
                    color[o] = c / count as f32;
                    normal[o] = n.normalize_or(Vec3::Z);
                    cover[o] = count as f32 / (SUPER * SUPER) as f32;
                }
            }
        }
    }
    if cover.iter().all(|&a| a == 0.0) {
        return None;
    }
    dilate(&mut color, &mut normal, &cover, aw, ah);
    let (albedo, normals) = mips(&color, &normal, &cover, aw, ah);
    Some(Baked {
        albedo,
        normal: normals,
        radius: half_width.max(half_height),
        half_width,
        half_height,
        center_z: (lo.z + hi.z) * 0.5,
        roughness: if rough.1 > 0.0 {
            rough.0 / rough.1
        } else {
            0.8
        },
    })
}

/// Darkens what lies recessed behind the surfaces around it (the inside of a crown seen through gaps, the trunk
/// under the leaves): the mesh gets that from its shadows and AO, a flat sprite has to carry it in the colour.
/// Front depths within a ring around each texel that are nearer to the viewer occlude it, up to
/// `OCCLUSION` darkening when they are a quarter of the model's radius nearer.
fn occlude(
    texel: &mut [Option<(Vec3, Vec3)>],
    depth: &[f32],
    w: usize,
    inner: f32,
    half_width: f32,
) {
    const OCCLUSION: f32 = 0.6;
    const RING: usize = 12;
    let falloff = 0.25 * half_width;
    let offsets: Vec<(i32, i32)> = (0..RING * 2)
        .map(|i| {
            let a = std::f32::consts::TAU * (i % RING) as f32 / RING as f32
                + if i >= RING { 0.26 } else { 0.0 };
            let r = inner * if i >= RING { 0.12 } else { 0.05 };
            ((a.cos() * r).round() as i32, (a.sin() * r).round() as i32)
        })
        .collect();
    let mut factor = vec![1.0f32; w * w];
    for y in 0..w {
        for x in 0..w {
            let i = y * w + x;
            if texel[i].is_none() {
                continue;
            }
            let mut occ = 0.0;
            for &(dx, dy) in &offsets {
                let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                if nx < 0 || ny < 0 || nx >= w as i32 || ny >= w as i32 {
                    continue;
                }
                let j = ny as usize * w + nx as usize;
                if texel[j].is_some() {
                    occ += ((depth[j] - depth[i]) / falloff).clamp(0.0, 1.0);
                }
            }
            factor[i] = 1.0 - OCCLUSION * occ / offsets.len() as f32;
        }
    }
    for (t, f) in texel.iter_mut().zip(factor) {
        if let Some((c, _)) = t {
            *c *= f;
        }
    }
}

/// Spreads the colour of covered texels into the empty ones next to them, so filtering at the silhouette does
/// not pull in black.
fn dilate(color: &mut [Vec3], normal: &mut [Vec3], cover: &[f32], w: usize, h: usize) {
    let mut filled: Vec<bool> = cover.iter().map(|&a| a > 0.0).collect();
    for _ in 0..PAD {
        let mut next = filled.clone();
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if filled[i] {
                    continue;
                }
                let (mut c, mut n, mut count) = (Vec3::ZERO, Vec3::ZERO, 0);
                for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        continue;
                    }
                    let j = ny as usize * w + nx as usize;
                    if filled[j] {
                        c += color[j];
                        n += normal[j];
                        count += 1;
                    }
                }
                if count > 0 {
                    color[i] = c / count as f32;
                    normal[i] = n.normalize_or(Vec3::Z);
                    next[i] = true;
                }
            }
        }
        filled = next;
    }
}

/// Coverage-weighted box mips: RGBA8 sRGB albedo + coverage and RG8 normals.
fn mips(
    color: &[Vec3],
    normal: &[Vec3],
    cover: &[f32],
    w: usize,
    h: usize,
) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
    let encode = |color: &[Vec3], normal: &[Vec3], cover: &[f32]| {
        let albedo = color
            .iter()
            .zip(cover)
            .flat_map(|(c, a)| {
                [
                    linear_to_srgb(c.x),
                    linear_to_srgb(c.y),
                    linear_to_srgb(c.z),
                    (a * 255.0 + 0.5) as u8,
                ]
            })
            .collect::<Vec<u8>>();
        let n = normal
            .iter()
            .flat_map(|n| {
                [
                    ((n.x * 0.5 + 0.5) * 255.0 + 0.5) as u8,
                    ((n.y * 0.5 + 0.5) * 255.0 + 0.5) as u8,
                ]
            })
            .collect();
        (albedo, n)
    };
    let (mut albedo, mut normals) = (Vec::new(), Vec::new());
    let (a, n) = encode(color, normal, cover);
    albedo.push(a);
    normals.push(n);
    let (mut c, mut nn, mut cv, mut lw, mut lh) =
        (color.to_vec(), normal.to_vec(), cover.to_vec(), w, h);
    for _ in 1..MIPS {
        let (nw, nh) = (lw / 2, lh / 2);
        let (mut c2, mut n2, mut a2) = (
            vec![Vec3::ZERO; nw * nh],
            vec![Vec3::ZERO; nw * nh],
            vec![0.0; nw * nh],
        );
        for y in 0..nh {
            for x in 0..nw {
                let (mut sum, mut ns, mut weight, mut plain) =
                    (Vec3::ZERO, Vec3::ZERO, 0.0, Vec3::ZERO);
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let j = (2 * y + dy) * lw + 2 * x + dx;
                    sum += c[j] * cv[j];
                    ns += nn[j] * cv[j].max(1e-3);
                    weight += cv[j];
                    plain += c[j];
                }
                let i = y * nw + x;
                c2[i] = if weight > 0.0 {
                    sum / weight
                } else {
                    plain / 4.0
                };
                n2[i] = ns.normalize_or(Vec3::Z);
                a2[i] = weight / 4.0;
            }
        }
        let (a, n) = encode(&c2, &n2, &a2);
        albedo.push(a);
        normals.push(n);
        (c, nn, cv, lw, lh) = (c2, n2, a2, nw, nh);
    }
    (albedo, normals)
}

/// GPU atlas and one shared quad, automatically instanced across all sectors.
pub struct Atlas {
    pub material: Handle<SpriteMaterial>,
    pub mesh: Handle<Mesh>,
    pub aabb: Aabb,
    pub radius: f32,
    pub half_width: f32,
    pub half_height: f32,
    pub center_z: f32,
    brightness: HashMap<u32, Handle<SpriteMaterial>>,
}

impl Atlas {
    pub(crate) fn material(&self, brightness: f32) -> Handle<SpriteMaterial> {
        self.brightness
            .get(&brightness.to_bits())
            .unwrap_or(&self.material)
            .clone()
    }
}

impl Baked {
    /// Uploads an atlas and one quad shared by every instance of this model.
    pub fn upload(
        &self,
        images: &mut Assets<Image>,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<SpriteMaterial>,
    ) -> Atlas {
        let mut image = |mips: &[Vec<u8>], format: TextureFormat| {
            let size = Extent3d {
                width: (COLS * RES) as u32,
                height: (ROWS * RES) as u32,
                depth_or_array_layers: 1,
            };
            let mut image = Image::new_uninit(
                size,
                TextureDimension::D2,
                format,
                RenderAssetUsages::RENDER_WORLD,
            );
            image.texture_descriptor.mip_level_count = mips.len() as u32;
            image.data = Some(mips.concat());
            image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                address_mode_u: ImageAddressMode::ClampToEdge,
                address_mode_v: ImageAddressMode::ClampToEdge,
                mag_filter: ImageFilterMode::Linear,
                min_filter: ImageFilterMode::Linear,
                mipmap_filter: ImageFilterMode::Linear,
                ..default()
            });
            images.add(image)
        };
        let material = materials.add(SpriteMaterial {
            params: SpriteParams {
                grid: Vec4::new(COLS as f32, ROWS as f32, PAD as f32 / RES as f32, ALPHA_REF),
                // FarTreeSprites has diffuse + ambient lighting, no specular term.
                surface: Vec4::new(self.roughness, 0.0, 1.0, 0.0),
                shape: Vec4::new(self.half_width, self.half_height, self.center_z, 0.0),
            },
            albedo: image(&self.albedo, TextureFormat::Rgba8UnormSrgb),
            normal: image(&self.normal, TextureFormat::Rg8Unorm),
        });
        let mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0, 0.0, 0.0]; 4])
        // The custom prepass supplies its normal; presence also satisfies Bevy's prepass capability check.
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; 4])
        .with_inserted_attribute(
            Mesh::ATTRIBUTE_UV_0,
            vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
        )
        .with_inserted_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]));
        let extent = Vec3::new(self.half_width, self.half_height, self.half_width);
        let center = Vec3::Y * self.center_z;
        Atlas {
            material,
            mesh: meshes.add(mesh),
            aabb: Aabb::from_min_max(center - extent, center + extent),
            radius: self.radius,
            half_width: self.half_width,
            half_height: self.half_height,
            center_z: self.center_z,
            brightness: HashMap::new(),
        }
    }
}

/// Atlas cache by bake address. Model instances retain the actual handles.
#[derive(Default)]
pub(crate) struct Atlases {
    by_bake: HashMap<usize, std::sync::Weak<Atlas>>,
}

impl Atlases {
    pub(crate) fn atlas(
        &mut self,
        baked: &Arc<Baked>,
        styles: &[crate::vegetation::Style],
        images: &mut Assets<Image>,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<SpriteMaterial>,
    ) -> Arc<Atlas> {
        let key = Arc::as_ptr(baked) as usize;
        if let Some(a) = self.by_bake.get(&key).and_then(std::sync::Weak::upgrade) {
            return a;
        }
        let mut atlas = baked.upload(images, meshes, materials);
        for style in styles {
            let brightness = style.brightness;
            if brightness == 1.0 || atlas.brightness.contains_key(&brightness.to_bits()) {
                continue;
            }
            let mut material = materials.get(&atlas.material).unwrap().clone();
            material.params.surface.z = brightness;
            atlas
                .brightness
                .insert(brightness.to_bits(), materials.add(material));
        }
        let atlas = Arc::new(atlas);
        self.by_bake.retain(|_, w| w.strong_count() > 0);
        self.by_bake.insert(key, Arc::downgrade(&atlas));
        atlas
    }
    pub(crate) fn clear(&mut self) {
        self.by_bake.clear();
    }
}

/// Per-instance six-bit colour/disable flag (bits 0..18), rounded switch distance (bits 19..31).
/// The far distance uses VisibilityRange, sharing one GPU range slot per whole-metre view distance.
pub fn tag(appear: f32, variation: u32) -> u32 {
    ((appear.round().clamp(1.0, 8191.0) as u32) << 19) | (variation & ((1 << 19) - 1))
}

/// Instance transform in its sector's Bevy frame; the shared quad expands in the vertex shader.
pub fn transform(v: &crate::placement::Vegetation, sector_origin: Vec3) -> Transform {
    Transform {
        translation: CRY_TO_BEVY * (v.pos - sector_origin),
        rotation: Quat::from_rotation_y(v.yaw()),
        scale: Vec3::splat(v.scale.clamp(0.1, 5.0)),
    }
}

#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct SpriteParams {
    /// Columns, rows, border of a view (fraction), alpha test.
    pub grid: Vec4,
    /// Roughness, reflectance, group brightness.
    pub surface: Vec4,
    /// Half width, half height, centre height above the instance origin.
    pub shape: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct SpriteMaterial {
    #[uniform(0)]
    pub params: SpriteParams,
    #[texture(1)]
    #[sampler(2)]
    pub albedo: Handle<Image>,
    #[texture(3)]
    pub normal: Handle<Image>,
}

impl Material for SpriteMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://aa_world/objects/sprites.wesl".into()
    }
    fn fragment_shader() -> ShaderRef {
        "embedded://aa_world/objects/sprites.wesl".into()
    }
    fn prepass_vertex_shader() -> ShaderRef {
        "embedded://aa_world/objects/sprites.wesl".into()
    }
    fn prepass_fragment_shader() -> ShaderRef {
        "embedded://aa_world/objects/sprites.wesl".into()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Mask(ALPHA_REF)
    }
    fn enable_shadows() -> bool {
        // bSpriteCastShadow is 0 for almost every group, and the mesh stops casting where the sprite takes over
        false
    }
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.vertex.buffers = vec![layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(1),
        ])?];
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// Batched vegetation sprite rendering (installed by AaWorldPlugin).
pub struct SpritePlugin;

impl Plugin for SpritePlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "sprites.wesl");
        app.add_plugins(MaterialPlugin::<SpriteMaterial>::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_switch_distance() {
        // 18 x 5 m x 0.8 x 1920 / 1024
        assert!((switch_distance(5.0, 1.0, 1.0, 1920.0) - 135.0).abs() < 1e-3);
        assert!((switch_distance(5.0, 1.2, 2.0, 1920.0) - 324.0).abs() < 1e-3);
        // fSpriteDistRatio below the custom minimum is clamped up to it, the distance to 8 m
        assert_eq!(switch_distance(5.0, 1.0, 0.0, 1920.0), 8.0);
        assert_eq!(switch_distance(0.2, 1.0, 1.0, 1920.0), 8.0);
        assert_eq!(switch_distance(5.0, 1.0, 1.0, 1024.0), 72.0);
        assert_eq!(switch_distance(5.0, 1.0, 1.0, 800.0), 72.0);
        assert_eq!(mesh_reach(135.0), 256.0);
        assert_eq!(mesh_reach(8.0), 128.0);
    }

    #[test]
    fn viewport_guard_keeps_large_and_zoomed_plants_as_meshes() {
        let view = View {
            width: 1920.0,
            height: 1080.0,
            fov: 42.75f32.to_radians(),
        };
        let native = switch_distance(5.0, 1.0, 1.0, view.width);
        assert_eq!(view.switch(5.0, 1.0, 1.0, 1.0), native);
        let at = view.switch(5.0, 1.0, 0.01, 1.0);
        assert!(at > 8.0);
        assert!(2.0 * (5.0 / at).atan() / view.fov * view.height <= 192.001);
        assert_eq!(view.switch(5.0, 1.0, 0.01, 2.0), at * 2.0);
        assert!(
            View {
                fov: view.fov * 0.5,
                ..view
            }
            .switch(5.0, 1.0, 0.01, 1.0)
                > at
        );
    }

    #[test]
    fn bakes_a_quad_into_every_view() {
        // a 2 m square facing +x and -x (two-sided), standing on the origin
        let mesh = aa_cry::MeshData {
            positions: vec![
                Vec3::new(0.0, -1.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
                Vec3::new(0.0, 1.0, 2.0),
                Vec3::new(0.0, -1.0, 2.0),
            ],
            normals: vec![Vec3::X; 4],
            uvs: vec![Vec2::ZERO; 4],
            indices: vec![0, 1, 2, 0, 2, 3],
            subsets: vec![aa_cry::mesh::Subset {
                first_index: 0,
                index_count: 6,
                first_vertex: 0,
                vertex_count: 4,
                material_id: 0,
                radius: 1.5,
                center: Vec3::Z,
            }],
            ..Default::default()
        };
        let model = Model {
            nodes: vec![aa_cry::mesh::Node {
                name: "a".into(),
                chunk_id: 0,
                parent: None,
                local: bevy::math::Affine3A::IDENTITY,
                kind: aa_cry::mesh::NodeKind::Mesh,
                mesh: Some(0),
                material_name: None,
                properties: String::new(),
            }],
            meshes: vec![mesh],
            ..Default::default()
        };
        let b = bake(&model, None, &mut |_| None).unwrap();
        assert!(
            (b.half_width - 1.0).abs() < 1e-5
                && (b.half_height - 1.0).abs() < 1e-5
                && (b.center_z - 1.0).abs() < 1e-5
        );
        assert_eq!(b.albedo.len(), MIPS);
        assert_eq!(b.albedo[0].len(), COLS * RES * ROWS * RES * 4);
        assert_eq!(
            b.normal[MIPS - 1].len(),
            (COLS * RES >> (MIPS - 1)) * (ROWS * RES >> (MIPS - 1)) * 2
        );
        let coverage = |k: usize| {
            let (cx, cy) = ((k % COLS) * RES, (k / COLS) * RES);
            (0..RES)
                .flat_map(|y| (0..RES).map(move |x| (x, y)))
                .filter(|(x, y)| b.albedo[0][((cy + y) * COLS * RES + cx + x) * 4 + 3] > 0)
                .count()
        };
        // seen face on from +x (slot 0) and -x (slot 12): the full inner square; edge on (slot 6): nothing
        let inner = (RES - 2 * PAD) * (RES - 2 * PAD);
        assert!(coverage(0) > inner * 9 / 10, "{}", coverage(0));
        assert!(coverage(12) > inner * 9 / 10);
        assert!(coverage(6) < RES * 2, "{}", coverage(6));
        // the view border stays empty
        assert_eq!(b.albedo[0][3], 0);
    }

    /// Bakes real bushes and trees; `AA_SPRITE_DUMP=<dir>` writes each atlas (over grey) and its normals as PPM.
    #[test]
    fn bakes_real_vegetation() {
        if !crate::pak_available() {
            return;
        }
        for path in [
            "objects/env/05_nature/00_common/common_bush_ec03.cgf",
            "objects/env/05_nature/08_cry_source/trees/river_tree/river_tree_a.cgf",
            "objects/env/05_nature/08_cry_source/bushes/cliffbush/cliff_bush_green_mini.cgf",
        ] {
            let model = Model::load(path).unwrap();
            let mtl =
                aa_cry::scene::find_mtl(path, &model).and_then(|p| CryMaterial::load(&p).ok());
            assert!(mtl.is_some(), "{path}");
            let b = bake(&model, mtl.as_ref(), &mut |file: &str| {
                let bytes = aa_pak::Pak::global()
                    .read(&aa_cry::material::texture_pak_path(file))
                    .ok()?;
                let dds = aa_cry::texture::Dds::parse(&bytes).ok()?;
                let mip = 2.min(dds.mips.saturating_sub(1));
                Some(Arc::new((
                    (dds.width >> mip).max(1) as usize,
                    (dds.height >> mip).max(1) as usize,
                    dds.decode_rgba8(0, mip),
                )))
            })
            .unwrap();
            let covered = b.albedo[0]
                .chunks_exact(4)
                .filter(|p| p[3] as f32 >= ALPHA_REF * 255.0)
                .count();
            let total = COLS * RES * ROWS * RES;
            assert!(
                covered > total / 20 && covered < total * 9 / 10,
                "{path}: {covered} of {total}"
            );
            assert!(b.radius > 0.5 && b.half_height > 0.2, "{path}");
            if let Ok(dir) = std::env::var("AA_SPRITE_DUMP") {
                let name = path.rsplit('/').next().unwrap().trim_end_matches(".cgf");
                let (w, h) = (COLS * RES, ROWS * RES);
                let mut rgb = format!(
                    "P6 {w} {h} 255
"
                )
                .into_bytes();
                for p in b.albedo[0].chunks_exact(4) {
                    let a = if p[3] as f32 >= ALPHA_REF * 255.0 {
                        1.0
                    } else {
                        0.0
                    };
                    rgb.extend(
                        p[..3]
                            .iter()
                            .map(|&c| (c as f32 * a + 128.0 * (1.0 - a)) as u8),
                    );
                }
                std::fs::write(format!("{dir}/{name}.ppm"), rgb).unwrap();
                let mut n = format!(
                    "P6 {w} {h} 255
"
                )
                .into_bytes();
                for p in b.normal[0].chunks_exact(2) {
                    n.extend([p[0], p[1], 255]);
                }
                std::fs::write(format!("{dir}/{name}_n.ppm"), n).unwrap();
            }
        }
    }

    #[test]
    fn instance_tag_keeps_colour_and_switch_separate() {
        let color = crate::vegetation_color::pack([17, 83, 195]);
        assert_eq!(tag(135.3, color) >> 19, 135);
        assert_eq!(tag(135.3, color) & ((1 << 19) - 1), color);
        assert_eq!(
            tag(0.0, crate::vegetation_color::NO_COLOR_VARIATION),
            (1 << 19) | (1 << 18)
        );
        assert_eq!(tag(9000.0, color) >> 19, 8191);
    }
}
