//! Libraries, textures and materials, loaded on background tasks.
//!
//! * [`FxLibraries`]: `libs/particles/<name>.xml` parsed on the compute task pool the first time an effect names the library.
//! * [`FxAssets`]: one [`FxMaterial`] per (texture or `.mtl`, blend mode, soft, draw order); the texture is decoded on a task
//!   (`aa_cry::texture::load_dds_from_pak`) and bound when it arrives. Emitters wait for their slot.
//! * Mesh particles: [`Geom`]s (a `.cgf` with its `.mtl`) are loaded on tasks too; once one is there the emitter gets a material
//!   slot made from what the `.mtl` says (blend mode, textures, opacity map).

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on, futures_lite::future};

use crate::geom::{Geom, GeomKey, LoadedGeom, Piece};
use crate::library::Library;
use crate::material::{FxMaterial, FxParams};
use crate::params::Blend;

enum LibEntry {
    Loading(Task<Result<Library, String>>),
    Ready(Arc<Library>),
    Failed,
}

/// State of a library request.
pub enum LibStatus {
    Loading,
    Ready(Arc<Library>),
    Failed,
}

/// The libraries by (lowercase) name.
#[derive(Resource, Default)]
pub struct FxLibraries {
    map: HashMap<String, LibEntry>,
}

impl FxLibraries {
    /// Starts loading a library unless it is known.
    pub fn request(&mut self, name: &str) {
        let key = name.to_ascii_lowercase();
        if self.map.contains_key(&key) {
            return;
        }
        let load = key.clone();
        let entry = match AsyncComputeTaskPool::try_get() {
            Some(pool) => LibEntry::Loading(pool.spawn(async move { Library::load(&load) })),
            None => match Library::load(&load) {
                Ok(l) => LibEntry::Ready(Arc::new(l)),
                Err(_) => LibEntry::Failed,
            },
        };
        self.map.insert(key, entry);
    }

    /// Puts a library in place without reading the pak (tests, tools).
    pub fn insert(&mut self, name: &str, library: Library) {
        self.map.insert(name.to_ascii_lowercase(), LibEntry::Ready(Arc::new(library)));
    }

    pub fn status(&self, name: &str) -> LibStatus {
        match self.map.get(&name.to_ascii_lowercase()) {
            Some(LibEntry::Ready(l)) => LibStatus::Ready(l.clone()),
            Some(LibEntry::Loading(_)) | None => LibStatus::Loading,
            Some(LibEntry::Failed) => LibStatus::Failed,
        }
    }

    /// Moves finished loads forward.
    pub fn poll(&mut self) {
        for (name, entry) in self.map.iter_mut() {
            if let LibEntry::Loading(task) = entry
                && let Some(done) = block_on(future::poll_once(task))
            {
                *entry = match done {
                    Ok(lib) => LibEntry::Ready(Arc::new(lib)),
                    Err(e) => {
                        warn!("aa-fx: particle library {name}: {e}");
                        LibEntry::Failed
                    }
                };
            }
        }
    }

    pub fn loading(&self) -> usize {
        self.map.values().filter(|e| matches!(e, LibEntry::Loading(_))).count()
    }
}

/// Where a material's texture comes from.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// A pak path of a texture.
    Texture(String),
    /// A pak path of a `.mtl`; its diffuse map is used.
    Material(String),
    /// A pak path of a texture that repeats (the texture of a mesh).
    Repeat(String),
    /// A pak path of an opacity map: repeats, and holds data (no sRGB decoding).
    Mask(String),
    /// A pak path of a normal map: repeats, linear.
    Normal(String),
    /// No texture (white).
    White,
}

/// A refraction material: the normal map that shifts the scene and how strongly.
#[derive(Clone, Debug, PartialEq)]
pub struct Refraction {
    /// Pak path of the normal map ("" = the material has none).
    pub normal: String,
    /// `RefrBumpScale`.
    pub bump: f32,
}

/// [`Refraction`] as a material key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RefrKey {
    pub normal: String,
    /// `RefrBumpScale` in 1/1000.
    pub bump: u16,
}

/// A material with an `AlphaTest` (the lace of `hit_pattern`): the sprite is cut where texture alpha times particle alpha is below
/// the cutoff, and a material with `PerturbationScale` deforms its texture by a screen-space noise map first.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LaceKey {
    /// `AlphaTest` in 1/1000.
    pub cutoff: u16,
    /// Pak path of the diffuse map (a deforming material repeats it, as the engine's default sampler does).
    pub diffuse: String,
    /// Pak path of the noise map (`Custom`, else `Normalmap`; "" = none).
    pub noise: String,
    /// `PerturbationScale`, `PerturbationStrenght`, `AnimSpeed` and `HorizonAnimSpeed` in 1/1000 (0 strength = no deformation).
    pub scale: i32,
    pub strength: i32,
    pub anim_v: i32,
    pub anim_h: i32,
}

/// What a mesh material adds to a sprite material.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MeshMat {
    /// Pak path of the opacity map ("" = none).
    pub mask: String,
    /// Weights of its channels, in 1/255.
    pub mask_chan: [u8; 4],
    /// Alpha test threshold in 1/255.
    pub cutoff: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MatKey {
    pub source: Source,
    pub blend: Blend,
    pub soft: bool,
    pub draw_last: i32,
    pub mesh: Option<MeshMat>,
    pub refract: Option<RefrKey>,
    /// The `AlphaTest` of a sprite's `.mtl`.
    pub lace: Option<LaceKey>,
    /// Sprites with an `AlphaTest` that changes per particle.
    pub atest: bool,
}

impl MatKey {
    /// The shader parameters of this material.
    fn params(&self, gain: f32, soft_distance: f32) -> FxParams {
        let mut p = FxParams::new(gain, if self.soft { soft_distance } else { 0.0 }, self.blend);
        if let Some(m) = &self.mesh {
            p.gain.w = m.cutoff as f32 / 255.0;
            if !m.mask.is_empty() {
                p.mask = Vec4::from_array(m.mask_chan.map(|c| c as f32 / 255.0));
            }
        }
        if self.atest {
            p.refr.z = 1.0;
        }
        if let Some(l) = &self.lace {
            p.gain.w = l.cutoff as f32 / 1000.0;
            if !l.noise.is_empty() {
                p.lace = Vec4::new(l.scale as f32, l.strength as f32, l.anim_v as f32, l.anim_h as f32) / 1000.0;
            }
        }
        if let Some(r) = &self.refract {
            // a unit normal moves the scene by `bump` times this fraction of the viewport height
            p.refr.x = r.bump as f32 / 1000.0 * REFRACTION_SCALE;
            p.refr.y = 1.0;
        }
        p
    }
}

/// Fraction of the viewport height that `RefrBumpScale = 1` shifts the scene by (a guess, tuned by eye).
const REFRACTION_SCALE: f32 = 0.2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotState {
    Loading,
    Ready,
    Failed,
}

pub struct MatSlot {
    pub handle: Handle<FxMaterial>,
    pub state: SlotState,
    /// Texture size in pixels once ready.
    pub size: UVec2,
    pub key: MatKey,
}

struct Loaded {
    image: Image,
}

enum GeomEntry {
    Loading(Task<Option<LoadedGeom>>),
    Ready(LoadedGeom),
    Failed,
}

/// State of a mesh request.
pub enum GeomState {
    Loading,
    Ready(Arc<Geom>),
    Failed,
}

/// The material table of the plugin.
#[derive(Resource, Default)]
pub struct FxAssets {
    keys: HashMap<MatKey, u32>,
    pub slots: Vec<MatSlot>,
    jobs: HashMap<Source, Task<Option<Loaded>>>,
    loaded: HashMap<Source, Option<(Handle<Image>, UVec2)>>,
    geoms: HashMap<GeomKey, GeomEntry>,
    refractions: HashMap<String, Option<Refraction>>,
    illums: HashMap<String, Option<Vec3>>,
    laces: HashMap<String, Option<LaceKey>>,
    /// A refraction material was asked for: the camera needs `ScreenSpaceTransmission`.
    pub refractive: bool,
}

fn load_image(path: &str, repeat: bool, srgb: bool) -> Option<Image> {
    let opts = aa_cry::texture::DdsOptions { srgb, bc_supported: true, repeat };
    let mut image = aa_cry::texture::load_dds_from_pak(path, opts).ok()?;
    let mode = if repeat { bevy::image::ImageAddressMode::Repeat } else { bevy::image::ImageAddressMode::ClampToEdge };
    image.sampler = bevy::image::ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
        address_mode_u: mode,
        address_mode_v: mode,
        mag_filter: bevy::image::ImageFilterMode::Linear,
        min_filter: bevy::image::ImageFilterMode::Linear,
        mipmap_filter: bevy::image::ImageFilterMode::Linear,
        ..default()
    });
    Some(image)
}

fn load_source(source: &Source) -> Option<Loaded> {
    let (path, repeat, srgb) = match source {
        Source::White => return None,
        Source::Texture(p) => (p.clone(), false, true),
        Source::Repeat(p) => (p.clone(), true, true),
        Source::Mask(p) => (p.clone(), true, false),
        Source::Normal(p) => (p.clone(), true, false),
        Source::Material(m) => {
            let mtl = aa_cry::material::Material::load(m).ok()?;
            let tex = std::iter::once(&mtl)
                .chain(mtl.sub_materials.iter())
                .find_map(|m| m.texture(&aa_cry::material::TextureMap::Diffuse))?;
            (tex.pak_path(), false, true)
        }
    };
    load_image(&path, repeat, srgb).map(|image| Loaded { image })
}

impl FxAssets {
    /// Starts loading a texture unless it is known.
    fn request_texture(&mut self, source: &Source) {
        if matches!(source, Source::White) || self.loaded.contains_key(source) || self.jobs.contains_key(source) {
            return;
        }
        if let Some(pool) = AsyncComputeTaskPool::try_get() {
            let src = source.clone();
            self.jobs.insert(source.clone(), pool.spawn(async move { load_source(&src) }));
        }
    }

    /// The slot of a material, created (and its texture requested) on first use.
    pub fn slot(&mut self, key: MatKey, materials: &mut Assets<FxMaterial>, gain: f32, soft_distance: f32) -> u32 {
        if let Some(&i) = self.keys.get(&key) {
            return i;
        }
        let params = key.params(gain, soft_distance);
        let refract = key.refract.is_some();
        self.refractive |= refract;
        let handle = materials.add(FxMaterial { params, texture: None, mask: None, normal: None, refract, blend: key.blend, depth_bias: key.draw_last as f32 * 0.02 });
        let index = self.slots.len() as u32;
        self.request_texture(&key.source.clone());
        if let Some(m) = key.mesh.clone()
            && !m.mask.is_empty()
        {
            self.request_texture(&Source::Mask(m.mask));
        }
        if let Some(r) = &key.refract {
            self.request_texture(&Source::Normal(r.normal.clone()));
        }
        if let Some(l) = &key.lace
            && !l.noise.is_empty()
        {
            self.request_texture(&Source::Normal(l.noise.clone()));
        }
        self.slots.push(MatSlot { handle, state: SlotState::Loading, size: UVec2::ONE, key: key.clone() });
        self.keys.insert(key, index);
        index
    }

    /// Whether a `.mtl` is a refraction material (`RefrBumpScale`), with its normal map; cached.
    pub fn refraction(&mut self, mtl: &str) -> Option<Refraction> {
        self.refractions
            .entry(mtl.to_string())
            .or_insert_with(|| {
                let m = aa_cry::material::Material::load(mtl).ok()?;
                let sub = std::iter::once(&m).chain(m.sub_materials.iter()).find(|m| m.public_param("RefrBumpScale").is_some())?;
                // `GenMask` bit 4 is refraction. A material that has the glow bit (0x10) as well, like the Return orb's, also draws its own
                // colour: the real client shows it bright, so it is drawn as a plain sprite here (no distortion)
                if sub.gen_mask & 0x10 != 0 {
                    return None;
                }
                let bump = sub.public_param("RefrBumpScale")?.trim().parse().unwrap_or(0.1);
                let normal = sub.texture(&aa_cry::material::TextureMap::Normal).map(|t| t.pak_path()).unwrap_or_default();
                Some(Refraction { normal, bump })
            })
            .clone()
    }

    /// The colour factor of an `Illum` sprite material, `Diffuse + Emissive` (the shader draws `texture * (diffuse * lighting + emissive)`); `None`
    /// for any other shader or a plain white one. `decal_death_pattern_black` has a diffuse of 0.004, `decal_ground3` and `decal_pattern1` 0: the real
    /// client draws those decals dark, not as their white and red texture. Cached.
    pub fn illum_tint(&mut self, mtl: &str) -> Option<Vec3> {
        *self.illums.entry(mtl.to_string()).or_insert_with(|| {
            use aa_cry::material::TextureMap;
            let m = aa_cry::material::Material::load(mtl).ok()?;
            let sub = std::iter::once(&m).chain(m.sub_materials.iter()).find(|m| m.texture(&TextureMap::Diffuse).is_some())?;
            if !sub.shader.eq_ignore_ascii_case("Illum") {
                return None;
            }
            let tint = (sub.diffuse + sub.emissive).min(Vec3::ONE);
            (tint.distance(Vec3::ONE) > 0.02).then_some(tint)
        })
    }

    /// The `AlphaTest` of a `.mtl` (`None` = no alpha test), with the screen-space deformation of its shader; cached.
    pub fn lace(&mut self, mtl: &str) -> Option<LaceKey> {
        self.laces
            .entry(mtl.to_string())
            .or_insert_with(|| {
                use aa_cry::material::TextureMap;
                let m = aa_cry::material::Material::load(mtl).ok()?;
                // the sub-material whose diffuse map the sprite shows (`load_source`)
                let sub = std::iter::once(&m).chain(m.sub_materials.iter()).find(|m| m.texture(&TextureMap::Diffuse).is_some())?;
                if sub.alpha_test <= 0.0 {
                    return None;
                }
                let param = |name: &str, default: f32| sub.public_param(name).and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(default);
                let milli = |v: f32| (v * 1000.0).round().clamp(-1.0e6, 1.0e6) as i32;
                // `GenMask` bit 0x10 is the `SCREEN_SPACE_DEFORMATION` of the particle shader
                let deforms = sub.gen_mask & 0x10 != 0;
                let noise = if deforms {
                    sub.texture(&TextureMap::Custom).or_else(|| sub.texture(&TextureMap::Normal)).map(|t| t.pak_path()).unwrap_or_default()
                } else {
                    String::new()
                };
                Some(LaceKey {
                    cutoff: (sub.alpha_test * 1000.0).round().clamp(0.0, 1000.0) as u16,
                    diffuse: sub.texture(&TextureMap::Diffuse).map(|t| t.pak_path()).unwrap_or_default(),
                    scale: milli(param("PerturbationScale", 0.5)),
                    strength: milli(param("PerturbationStrenght", 0.01)),
                    anim_v: milli(param("AnimSpeed", 0.2)),
                    anim_h: milli(param("HorizonAnimSpeed", 0.0)),
                    noise,
                })
            })
            .clone()
    }

    /// The mesh a key names: requested on first use.
    pub fn geom(&mut self, key: &GeomKey) -> GeomState {
        match self.geoms.get(key) {
            Some(GeomEntry::Ready(g)) => return GeomState::Ready(g.geom.clone()),
            Some(GeomEntry::Failed) => return GeomState::Failed,
            Some(GeomEntry::Loading(_)) => return GeomState::Loading,
            None => {}
        }
        let load = key.clone();
        let entry = match AsyncComputeTaskPool::try_get() {
            Some(pool) => GeomEntry::Loading(pool.spawn(async move { Geom::load_with_pieces(&load) })),
            None => match Geom::load_with_pieces(key) {
                Some(g) => GeomEntry::Ready(g),
                None => GeomEntry::Failed,
            },
        };
        self.geoms.insert(key.clone(), entry);
        GeomState::Loading
    }

    pub(crate) fn pieces(&self, key: &GeomKey) -> Arc<[Piece]> {
        match self.geoms.get(key) {
            Some(GeomEntry::Ready(g)) => g.pieces.clone(),
            _ => Arc::from([]),
        }
    }

    /// Collects finished texture and mesh loads and binds the textures to the waiting materials.
    pub fn poll(&mut self, images: &mut Assets<Image>, materials: &mut Assets<FxMaterial>) {
        let mut finished = Vec::new();
        for (source, task) in self.jobs.iter_mut() {
            if let Some(done) = block_on(future::poll_once(task)) {
                finished.push((source.clone(), done));
            }
        }
        for (source, done) in finished {
            self.jobs.remove(&source);
            let entry = done.map(|l| {
                let size = UVec2::new(l.image.width(), l.image.height());
                (images.add(l.image), size)
            });
            if entry.is_none() {
                warn!("aa-fx: texture {source:?} could not be loaded");
            }
            self.loaded.insert(source, entry);
        }
        for (key, entry) in self.geoms.iter_mut() {
            if let GeomEntry::Loading(task) = entry
                && let Some(done) = block_on(future::poll_once(task))
            {
                *entry = match done {
                    Some(g) => GeomEntry::Ready(g),
                    None => {
                        warn!("aa-fx: model {} could not be loaded", key.path);
                        GeomEntry::Failed
                    }
                };
            }
        }
        for slot in &mut self.slots {
            if slot.state != SlotState::Loading {
                continue;
            }
            let mask = match &slot.key.mesh {
                Some(m) if !m.mask.is_empty() => match self.loaded.get(&Source::Mask(m.mask.clone())) {
                    Some(Some((image, _))) => Some(Some(image.clone())),
                    Some(None) => Some(None),
                    None => continue,
                },
                _ => None,
            };
            let normal_path = match (&slot.key.refract, &slot.key.lace) {
                (Some(r), _) => Some(r.normal.clone()),
                (None, Some(l)) if !l.noise.is_empty() => Some(l.noise.clone()),
                _ => None,
            };
            let normal = match normal_path {
                Some(path) => match self.loaded.get(&Source::Normal(path)) {
                    Some(Some((image, _))) => Some(Some(image.clone())),
                    Some(None) => Some(None),
                    None => continue,
                },
                None => None,
            };
            let color = match &slot.key.source {
                Source::White => Some(None),
                source => match self.loaded.get(source) {
                    Some(Some((image, size))) => Some(Some((image.clone(), *size))),
                    Some(None) => None,
                    None => continue,
                },
            };
            let Some(color) = color else {
                slot.state = SlotState::Failed;
                continue;
            };
            if let Some(mut m) = materials.get_mut(&slot.handle) {
                m.texture = color.as_ref().map(|c| c.0.clone());
                match mask {
                    Some(Some(image)) => m.mask = Some(image),
                    Some(None) => m.params.mask = Vec4::ZERO,
                    None => {}
                }
                match normal {
                    Some(Some(image)) => m.normal = Some(image),
                    Some(None) if slot.key.refract.is_some() => m.params.refr = Vec4::ZERO,
                    Some(None) => m.params.lace = Vec4::ZERO,
                    None => {}
                }
            }
            if let Some((_, size)) = color {
                slot.size = size;
            }
            slot.state = SlotState::Ready;
        }
    }

    pub fn loading(&self) -> usize {
        self.slots.iter().filter(|s| s.state == SlotState::Loading).count()
    }

    /// Applies new colour gain / soft distance to every material.
    pub fn set_params(&self, materials: &mut Assets<FxMaterial>, gain: f32, soft_distance: f32) {
        for slot in &self.slots {
            let want = slot.key.params(gain, soft_distance).gain;
            if let Some(m) = materials.get(&slot.handle)
                && m.params.gain != want
                && let Some(mut m) = materials.get_mut(&slot.handle)
            {
                m.params.gain = want;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `hit_pattern` is the lace material of the white skills: an alpha test, a deforming shader and a noise map.
    #[test]
    fn reads_the_lace_of_hit_pattern() {
        let mut assets = FxAssets::default();
        let Some(lace) = assets.lace("Materials/effects/hit_pattern") else {
            eprintln!("hit_pattern.mtl is not readable here, skipped");
            return;
        };
        assert_eq!(lace.cutoff, 366);
        assert_eq!(lace.noise, "textures/p_effects/noise_ddn.dds");
        assert_eq!((lace.scale, lace.strength, lace.anim_v), (8800, 2195, 300));
        let params = MatKey {
            source: Source::White,
            blend: Blend::Additive,
            soft: false,
            draw_last: 0,
            mesh: None,
            refract: None,
            lace: Some(lace),
            atest: false,
        }
        .params(1.0, 0.0);
        assert!((params.gain.w - 0.366).abs() < 1e-6);
        assert!((params.lace.y - 2.195).abs() < 1e-6);
    }

    #[test]
    fn a_material_without_alpha_test_has_no_lace() {
        let mut assets = FxAssets::default();
        assert_eq!(assets.lace("Materials/effects/does_not_exist"), None);
    }
}
