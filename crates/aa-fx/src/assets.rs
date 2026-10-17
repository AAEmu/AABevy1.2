//! Libraries, textures and materials, loaded on background tasks.
//!
//! * [`FxLibraries`]: `libs/particles/<name>.xml` parsed on the compute task pool the first time an effect names the library.
//! * [`FxAssets`]: one [`FxMaterial`] per (texture or `.mtl`, blend mode, soft, draw order); the texture is decoded on a task
//!   (`aa_cry::texture::load_dds_from_pak`) and bound when it arrives. Emitters wait for their slot.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on, futures_lite::future};

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
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MatKey {
    pub source: Source,
    pub blend: Blend,
    pub soft: bool,
    pub draw_last: i32,
}

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

/// The material table of the plugin.
#[derive(Resource, Default)]
pub struct FxAssets {
    keys: HashMap<MatKey, u32>,
    pub slots: Vec<MatSlot>,
    jobs: HashMap<Source, Task<Option<Loaded>>>,
    loaded: HashMap<Source, Option<(Handle<Image>, UVec2)>>,
}

fn load_image(path: &str) -> Option<Image> {
    let opts = aa_cry::texture::DdsOptions { srgb: true, bc_supported: true, repeat: false };
    let mut image = aa_cry::texture::load_dds_from_pak(path, opts).ok()?;
    image.sampler = bevy::image::ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
        address_mode_u: bevy::image::ImageAddressMode::ClampToEdge,
        address_mode_v: bevy::image::ImageAddressMode::ClampToEdge,
        mag_filter: bevy::image::ImageFilterMode::Linear,
        min_filter: bevy::image::ImageFilterMode::Linear,
        mipmap_filter: bevy::image::ImageFilterMode::Linear,
        ..default()
    });
    Some(image)
}

fn load_source(source: &Source) -> Option<Loaded> {
    let path = match source {
        Source::Texture(p) => p.clone(),
        Source::Material(m) => {
            let mtl = aa_cry::material::Material::load(m).ok()?;
            let tex = std::iter::once(&mtl)
                .chain(mtl.sub_materials.iter())
                .find_map(|m| m.texture(&aa_cry::material::TextureMap::Diffuse))?;
            tex.pak_path()
        }
    };
    load_image(&path).map(|image| Loaded { image })
}

impl FxAssets {
    /// The slot of a material, created (and its texture requested) on first use.
    pub fn slot(&mut self, key: MatKey, materials: &mut Assets<FxMaterial>, gain: f32, soft_distance: f32) -> u32 {
        if let Some(&i) = self.keys.get(&key) {
            return i;
        }
        let params = FxParams::new(gain, if key.soft { soft_distance } else { 0.0 }, key.blend);
        let handle = materials.add(FxMaterial { params, texture: None, blend: key.blend, depth_bias: key.draw_last as f32 * 0.02 });
        let index = self.slots.len() as u32;
        if !self.loaded.contains_key(&key.source)
            && !self.jobs.contains_key(&key.source)
            && let Some(pool) = AsyncComputeTaskPool::try_get()
        {
            let source = key.source.clone();
            self.jobs.insert(key.source.clone(), pool.spawn(async move { load_source(&source) }));
        }
        self.slots.push(MatSlot { handle, state: SlotState::Loading, size: UVec2::ONE, key: key.clone() });
        self.keys.insert(key, index);
        index
    }

    /// Collects finished texture loads and binds them to the waiting materials.
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
        for slot in &mut self.slots {
            if slot.state != SlotState::Loading {
                continue;
            }
            match self.loaded.get(&slot.key.source) {
                Some(Some((image, size))) => {
                    if let Some(mut m) = materials.get_mut(&slot.handle) {
                        m.texture = Some(image.clone());
                    }
                    slot.size = *size;
                    slot.state = SlotState::Ready;
                }
                Some(None) => slot.state = SlotState::Failed,
                None => {}
            }
        }
    }

    pub fn loading(&self) -> usize {
        self.slots.iter().filter(|s| s.state == SlotState::Loading).count()
    }

    /// Applies new colour gain / soft distance to every material.
    pub fn set_params(&self, materials: &mut Assets<FxMaterial>, gain: f32, soft_distance: f32) {
        for slot in &self.slots {
            let want = FxParams::new(gain, if slot.key.soft { soft_distance } else { 0.0 }, slot.key.blend).gain;
            if let Some(m) = materials.get(&slot.handle)
                && m.params.gain != want
                && let Some(mut m) = materials.get_mut(&slot.handle)
            {
                m.params.gain = want;
            }
        }
    }
}
