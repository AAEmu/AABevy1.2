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

use crate::geom::{Geom, GeomKey};
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
        if let Some(r) = &self.refract {
            // a unit normal moves the scene by `bump` times this fraction of the viewport height
            p.refr = Vec4::new(r.bump as f32 / 1000.0 * REFRACTION_SCALE, 1.0, 0.0, 0.0);
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
    Loading(Task<Option<Geom>>),
    Ready(Arc<Geom>),
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
                let bump = sub.public_param("RefrBumpScale")?.trim().parse().unwrap_or(0.1);
                let normal = sub.texture(&aa_cry::material::TextureMap::Normal).map(|t| t.pak_path()).unwrap_or_default();
                Some(Refraction { normal, bump })
            })
            .clone()
    }

    /// The mesh a key names: requested on first use.
    pub fn geom(&mut self, key: &GeomKey) -> GeomState {
        match self.geoms.get(key) {
            Some(GeomEntry::Ready(g)) => return GeomState::Ready(g.clone()),
            Some(GeomEntry::Failed) => return GeomState::Failed,
            Some(GeomEntry::Loading(_)) => return GeomState::Loading,
            None => {}
        }
        let load = key.clone();
        let entry = match AsyncComputeTaskPool::try_get() {
            Some(pool) => GeomEntry::Loading(pool.spawn(async move { Geom::load(&load) })),
            None => match Geom::load(key) {
                Some(g) => GeomEntry::Ready(Arc::new(g)),
                None => GeomEntry::Failed,
            },
        };
        self.geoms.insert(key.clone(), entry);
        GeomState::Loading
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
                    Some(g) => GeomEntry::Ready(Arc::new(g)),
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
            let normal = match &slot.key.refract {
                Some(r) => match self.loaded.get(&Source::Normal(r.normal.clone())) {
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
                    Some(None) => m.params.refr = Vec4::ZERO,
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
