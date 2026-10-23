//! Thread-safe preparation of Bevy assets (`prepare` on any thread, `finish` on the main thread).
//!
//! Everything expensive happens in [`CryLoader::prepare_model`]: reading and parsing the model,
//! its `.mtl`, decoding / baking textures and building the meshes. The result, a
//! [`PreparedModel`], holds ready `Handle`s (reserved up front, see
//! `Assets::get_handle_provider`) and the asset values to move into the `Assets` collections;
//! [`crate::CryAssets::finish_model`] only inserts them. [`crate::CryAssets::load_model`] is
//! `prepare_model` + `finish_model` on the calling thread.
//!
//! ```no_run
//! # use bevy::prelude::*;
//! # use aa_cry::{CryAssets, ModelOptions};
//! fn start(mut cry: CryAssets) {
//!     // main thread, once: the loader is a cheap clone and `Send + Sync`
//!     let loader = cry.loader();
//!     // worker thread (any task pool or `std::thread`): parse, decode, build meshes
//!     let job = std::thread::spawn(move || loader.prepare_model("objects/env/x.cgf", &ModelOptions::default()));
//!     // ... later, in a system that holds `CryAssets`: insert the assets (moves values only)
//!     if let Ok(Ok(prepared)) = job.join() {
//!         let _loaded = cry.finish_model(prepared);
//!     }
//! }
//! ```
//!
//! Textures, materials and parsed `.mtl` files are shared between all threads (and with the
//! synchronous API). Two models prepared at the same time that need the same new texture do not
//! decode it twice: the first one creates the image and every model that references it carries
//! the same pending slot, so whichever is finished first inserts it (and a model that is never
//! finished does not strand the others).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use bevy::asset::{AssetHandleProvider, RenderAssetUsages};
use bevy::image::Image;
use bevy::mesh::skinning::SkinnedMeshInverseBindposes;
use bevy::prelude::*;

use crate::material::{Material as CryMaterial, texture_pak_path};
use crate::mesh::Model;
use crate::scene::{LoadedModel, MaterialHook, ModelOptions, Part, Shading, find_mtl, render_transform, subset_mesh_with};
use crate::skeleton::Skeleton;
use crate::texture::{Dds, DdsOptions, dds_to_image};
use crate::Result;

pub(crate) type Slot<T> = Arc<Mutex<Option<T>>>;

pub(crate) fn slot<T>(v: T) -> Slot<T> {
    Arc::new(Mutex::new(Some(v)))
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// An asset that several prepared models can reference: whoever is finished first inserts it.
#[derive(Clone)]
pub(crate) enum Shared {
    Image(Handle<Image>, Slot<Image>),
    Material(Handle<StandardMaterial>, Slot<StandardMaterial>),
}

/// An asset value waiting to be moved into its `Assets` collection by the main thread.
pub(crate) enum Pending {
    Shared(Shared),
    Mesh(Handle<Mesh>, Mesh),
    Bindposes(Handle<SkinnedMeshInverseBindposes>, SkinnedMeshInverseBindposes),
}

#[derive(Clone)]
struct ImageEntry {
    handle: Handle<Image>,
    slot: Slot<Image>,
}

#[derive(Clone)]
struct MaterialEntry {
    handle: Handle<StandardMaterial>,
    slot: Slot<StandardMaterial>,
    /// Textures the material references (so any model using it can deliver them).
    deps: Arc<Vec<Shared>>,
}

/// (path, material override, lod, keep CPU mesh)
pub(crate) type ModelKey = (String, Option<String>, u32, bool, Shading);

enum CacheKey {
    Plain(ModelKey),
    /// Bound to a base skeleton (keyed by the base skeleton's `Arc` address).
    Bound(ModelKey, usize),
}

/// A model that was prepared (possibly on another thread) and still has to be finished on the
/// main thread with [`crate::CryAssets::finish_model`].
pub struct PreparedModel {
    pub(crate) loaded: Arc<LoadedModel>,
    pub(crate) pending: Vec<Pending>,
    register: Vec<(CacheKey, Arc<LoadedModel>)>,
}

impl PreparedModel {
    /// The model's handles, valid immediately (the assets behind them arrive on `finish_model`).
    pub fn loaded(&self) -> &Arc<LoadedModel> {
        &self.loaded
    }
}

struct Inner {
    images: AssetHandleProvider,
    meshes: AssetHandleProvider,
    materials: AssetHandleProvider,
    bindposes: AssetHandleProvider,
    bc_supported: bool,
    mtls: Mutex<HashMap<String, Option<Arc<CryMaterial>>>>,
    textures: Mutex<HashMap<(String, bool), Option<ImageEntry>>>,
    opaque_alpha: Mutex<HashMap<String, Option<bool>>>,
    cubes: Mutex<HashMap<(String, bool), Option<ImageEntry>>>,
    baked: Mutex<HashMap<String, Option<ImageEntry>>>,
    materials_cache: Mutex<HashMap<(String, usize, Shading, bool), Option<MaterialEntry>>>,
    default_material: OnceLock<MaterialEntry>,
    raw_models: Mutex<HashMap<String, Arc<Model>>>,
    models: Mutex<HashMap<ModelKey, Arc<LoadedModel>>>,
    bound: Mutex<HashMap<(ModelKey, usize), Arc<LoadedModel>>>,
    by_basename: OnceLock<HashMap<String, String>>,
    missing: Mutex<Vec<String>>,
}

/// Prepares models, materials and textures as Bevy assets on any thread. Cheap to clone; get it
/// from [`crate::CryAssets::loader`].
#[derive(Clone)]
pub struct CryLoader {
    shared: Arc<Inner>,
}

impl std::fmt::Debug for CryLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CryLoader")
    }
}

fn new_handle<A: Asset>(provider: &AssetHandleProvider) -> Handle<A> {
    provider.reserve_handle().typed::<A>()
}

impl CryLoader {
    pub(crate) fn new(
        images: AssetHandleProvider,
        meshes: AssetHandleProvider,
        materials: AssetHandleProvider,
        bindposes: AssetHandleProvider,
        bc_supported: bool,
    ) -> CryLoader {
        CryLoader {
            shared: Arc::new(Inner {
                images,
                meshes,
                materials,
                bindposes,
                bc_supported,
                mtls: Default::default(),
                textures: Default::default(),
                opaque_alpha: Default::default(),
                cubes: Default::default(),
                baked: Default::default(),
                materials_cache: Default::default(),
                default_material: OnceLock::new(),
                raw_models: Default::default(),
                models: Default::default(),
                bound: Default::default(),
                by_basename: OnceLock::new(),
                missing: Default::default(),
            }),
        }
    }

    /// Texture paths that could not be loaded since the last call (diagnostics).
    pub(crate) fn take_missing(&self) -> Vec<String> {
        std::mem::take(&mut *lock(&self.shared.missing))
    }

    /// A parsed `.mtl` (cached); `None` if missing or unreadable.
    pub fn mtl(&self, path: &str) -> Option<Arc<CryMaterial>> {
        let key = aa_pak::normalize(path);
        if let Some(m) = lock(&self.shared.mtls).get(&key) {
            return m.clone();
        }
        let m = if aa_pak::Pak::global().exists(&key) {
            match CryMaterial::load(&key) {
                Ok(m) => Some(Arc::new(m)),
                Err(e) => {
                    warn!("aa-cry: material {key}: {e}");
                    None
                }
            }
        } else {
            None
        };
        lock(&self.shared.mtls).entry(key).or_insert(m).clone()
    }

    fn raw_model(&self, path: &str) -> Result<Arc<Model>> {
        if let Some(m) = lock(&self.shared.raw_models).get(path) {
            return Ok(m.clone());
        }
        let _t = crate::prof::scope("cry.parse_model");
        let m = Arc::new(Model::load(path)?);
        Ok(lock(&self.shared.raw_models).entry(path.to_string()).or_insert(m).clone())
    }

    /// Picks the file for a LOD: `*_lodN` if it exists (falling back to lower N), else the file.
    fn load_lod(&self, path: &str, lod: u32) -> Result<(String, Arc<Model>)> {
        let pak = aa_pak::Pak::global();
        for l in (1..=lod).rev() {
            let p = crate::lod_path(path, l);
            if p != path && pak.exists(&p) {
                if let Ok(m) = self.raw_model(&p) {
                    return Ok((p, m));
                }
            }
        }
        Ok((path.to_string(), self.raw_model(path)?))
    }

    /// The pak file of a texture reference, resolving the ~1% of paths that moved (found by file
    /// name) and names with a stray ` copy`. `None` when nothing matches.
    pub fn resolve_texture(&self, path: &str) -> Option<String> {
        let pak = aa_pak::Pak::global();
        let file = texture_pak_path(path);
        if file.is_empty() {
            return None;
        }
        if pak.exists(&file) {
            return Some(file);
        }
        // `el_f_face00_df copy.dds` style names (the artists' file copies): the original
        let stripped = file.replace(" copy", "");
        if stripped != file && pak.exists(&stripped) {
            return Some(stripped);
        }
        let index = self.shared.by_basename.get_or_init(|| {
            let mut m = HashMap::new();
            for n in pak.names() {
                let n = aa_pak::normalize(n);
                if n.ends_with(".dds") {
                    let base = n.rsplit('/').next().unwrap_or(&n).to_string();
                    m.entry(base).or_insert(n);
                }
            }
            m
        });
        let base = stripped.rsplit('/').next().unwrap_or(&stripped);
        index.get(base).cloned()
    }

    fn read_dds(&self, path: &str) -> Option<(String, Vec<u8>)> {
        let file = self.resolve_texture(path)?;
        crate::read(&file).ok().map(|b| (file, b))
    }

    /// Decodes a flat (2D) texture; `None` for missing files and for cube / volume textures.
    fn load_flat(&self, key: &str, srgb: bool) -> Option<Image> {
        let _t = crate::prof::scope("cry.texture");
        let Some((file, bytes)) = self.read_dds(key) else {
            debug!("aa-cry: texture {key}: not found");
            lock(&self.shared.missing).push(key.to_string());
            return None;
        };
        let options = DdsOptions { srgb, bc_supported: self.shared.bc_supported, repeat: true };
        match Dds::parse(&bytes) {
            Ok(dds) if dds.faces > 1 || dds.depth > 1 => {
                // wgpu rejects a cube / volume view in a 2D slot (validation error, the app exits)
                debug!("aa-cry: texture {file} is a cube / volume map, not bound to a 2D slot");
                None
            }
            Ok(dds) => match dds_to_image(&dds, options) {
                Ok(img) => Some(img),
                Err(e) => {
                    debug!("aa-cry: texture {file}: {e}");
                    lock(&self.shared.missing).push(key.to_string());
                    None
                }
            },
            Err(e) => {
                debug!("aa-cry: texture {file}: {e}");
                lock(&self.shared.missing).push(key.to_string());
                None
            }
        }
    }

    /// Mean alpha (0..1) of a texture, from its smallest mip; `None` when it cannot be read.
    pub fn mean_alpha(&self, path: &str) -> Option<f32> {
        let (_, _, rgba) = self.decode_rgba8(path, u32::MAX)?;
        let n = rgba.len() / 4;
        (n > 0).then(|| rgba.chunks_exact(4).map(|p| p[3] as f32).sum::<f32>() / (255.0 * n as f32))
    }

    /// Cached proof of full opacity across every authored mip, independent of colour space.
    /// Missing/unreadable textures return `None`; incomplete/non-flat textures cannot prove it.
    pub fn alpha_is_opaque(&self, path: &str) -> Option<bool> {
        let key = crate::material::texture_pak_path(path);
        if let Some(opaque) = lock(&self.shared.opaque_alpha).get(&key) {
            return *opaque;
        }
        let opaque = self.read_dds(&key).and_then(|(_, bytes)| {
            Dds::parse(&bytes).ok().map(|dds| dds.alpha_is_opaque())
        });
        lock(&self.shared.opaque_alpha).insert(key, opaque);
        opaque
    }

    /// Every mip of a flat texture as RGBA8 (CPU decode), for baking: `(width, height, mips)`.
    pub fn decode_mips_rgba8(&self, path: &str) -> Option<(u32, u32, Vec<Vec<u8>>)> {
        let (_, bytes) = self.read_dds(path)?;
        let dds = Dds::parse(&bytes).ok()?;
        if dds.faces > 1 || dds.depth > 1 {
            return None;
        }
        Some((dds.width, dds.height, (0..dds.mips.max(1)).map(|m| dds.decode_rgba8(0, m)).collect()))
    }

    /// Raw RGBA8 pixels of mip `mip` of a texture (CPU decode), for baking; `(w, h, rgba)`.
    pub fn decode_rgba8(&self, path: &str, mip: u32) -> Option<(u32, u32, Vec<u8>)> {
        let (_, bytes) = self.read_dds(path)?;
        let dds = Dds::parse(&bytes).ok()?;
        if dds.faces > 1 || dds.depth > 1 {
            return None;
        }
        let mip = mip.min(dds.mips.saturating_sub(1));
        Some(((dds.width >> mip).max(1), (dds.height >> mip).max(1), dds.decode_rgba8(0, mip)))
    }
}

/// Collects the assets one preparation job creates or references.
pub struct Prep<'a> {
    pub(crate) loader: &'a CryLoader,
    pub(crate) pending: Vec<Pending>,
    pub(crate) shading: Shading,
}

impl<'a> Prep<'a> {
    pub(crate) fn new(loader: &'a CryLoader) -> Prep<'a> {
        Prep { loader, pending: Vec::new(), shading: Shading::Basic }
    }

    pub(crate) fn into_pending(self) -> Vec<Pending> {
        self.pending
    }

    fn push(&mut self, s: Shared) {
        self.pending.push(Pending::Shared(s));
    }

    /// A pak texture as an image handle (cached). `srgb` for colour maps. `None` when the file
    /// is missing or a cube / volume map.
    pub fn texture(&mut self, path: &str, srgb: bool) -> Option<Handle<Image>> {
        let key = (texture_pak_path(path), srgb);
        if key.0.is_empty() {
            return None;
        }
        if let Some(entry) = lock(&self.loader.shared.textures).get(&key).cloned() {
            return entry.map(|e| {
                self.push(Shared::Image(e.handle.clone(), e.slot));
                e.handle
            });
        }
        let image = self.loader.load_flat(&key.0, srgb);
        let entry = image.map(|img| ImageEntry { handle: new_handle(&self.loader.shared.images), slot: slot(img) });
        let entry = lock(&self.loader.shared.textures).entry(key).or_insert(entry).clone();
        entry.map(|e| {
            self.push(Shared::Image(e.handle.clone(), e.slot));
            e.handle
        })
    }

    /// A cube map of the pak (reflection / environment maps), cached.
    pub fn cube_texture(&mut self, path: &str) -> Option<Handle<Image>> {
        self.cube_texture_with_encoding(path, true)
    }

    /// Native Illum's environment sampler does not use sRGBLookup; stage IBL does. Cache both views.
    pub(crate) fn cube_texture_with_encoding(&mut self, path: &str, srgb: bool) -> Option<Handle<Image>> {
        let key = (texture_pak_path(path), srgb);
        if key.0.is_empty() {
            return None;
        }
        if let Some(entry) = lock(&self.loader.shared.cubes).get(&key).cloned() {
            return entry.map(|e| {
                self.push(Shared::Image(e.handle.clone(), e.slot));
                e.handle
            });
        }
        let image = self.loader.read_dds(&key.0).and_then(|(_, bytes)| {
            let dds = Dds::parse(&bytes).ok()?;
            if !dds.is_cubemap() {
                return None;
            }
            dds_to_image(&dds, DdsOptions { srgb, bc_supported: self.loader.shared.bc_supported, repeat: false }).ok()
        });
        let entry = image.map(|img| ImageEntry { handle: new_handle(&self.loader.shared.images), slot: slot(img) });
        let entry = lock(&self.loader.shared.cubes).entry(key).or_insert(entry).clone();
        entry.map(|e| {
            self.push(Shared::Image(e.handle.clone(), e.slot));
            e.handle
        })
    }

    /// An image computed from other textures, cached under `key` (`make` runs once).
    pub fn baked_image(&mut self, key: &str, make: impl FnOnce(&CryLoader) -> Option<Image>) -> Option<Handle<Image>> {
        if let Some(entry) = lock(&self.loader.shared.baked).get(key).cloned() {
            return entry.map(|e| {
                self.push(Shared::Image(e.handle.clone(), e.slot));
                e.handle
            });
        }
        let entry = make(self.loader).map(|img| ImageEntry { handle: new_handle(&self.loader.shared.images), slot: slot(img) });
        let entry = lock(&self.loader.shared.baked).entry(key.to_string()).or_insert(entry).clone();
        entry.map(|e| {
            self.push(Shared::Image(e.handle.clone(), e.slot));
            e.handle
        })
    }

    /// An image that belongs to the caller alone (not cached).
    pub fn new_image(&mut self, image: Image) -> Handle<Image> {
        let handle = new_handle(&self.loader.shared.images);
        self.push(Shared::Image(handle.clone(), slot(image)));
        handle
    }

    /// Converts one sub-material for UVs in 0..1 (no cache): `None` for materials that are not
    /// drawn. Model preparation validates each referenced subset before enabling static UCC.
    pub fn convert(&mut self, m: &CryMaterial) -> Option<StandardMaterial> {
        crate::shade::convert_material(self, m, true)
    }

    fn default_material(&mut self) -> Handle<StandardMaterial> {
        let sh = &self.loader.shared;
        let e = sh.default_material.get_or_init(|| MaterialEntry {
            handle: new_handle(&sh.materials),
            slot: slot(StandardMaterial { base_color: Color::srgb(0.6, 0.6, 0.6), perceptual_roughness: 0.8, ..default() }),
            deps: Arc::new(Vec::new()),
        });
        let (handle, slot) = (e.handle.clone(), e.slot.clone());
        self.push(Shared::Material(handle.clone(), slot));
        handle
    }

    /// The Bevy material for sub-material `sub` of a `.mtl` (cached). `None` when the
    /// sub-material is not drawn (NoDraw, Cut, proxies). Direct callers provide UVs in 0..1;
    /// model preparation keeps the original material for unsupported UCC domains.
    pub fn material(
        &mut self,
        mtl_path: Option<&str>,
        sub: usize,
    ) -> Option<Handle<StandardMaterial>> {
        self.material_for_domain(mtl_path, sub, true)
    }

    fn material_for_domain(
        &mut self,
        mtl_path: Option<&str>,
        sub: usize,
        ucc_uv_square: bool,
    ) -> Option<Handle<StandardMaterial>> {
        let key = (
            mtl_path.map(aa_pak::normalize).unwrap_or_default(),
            sub,
            self.shading,
            ucc_uv_square,
        );
        if let Some(entry) = lock(&self.loader.shared.materials_cache).get(&key).cloned() {
            return entry.map(|e| self.use_material(&e));
        }
        let _t = crate::prof::scope("cry.material");
        let mtl = mtl_path.and_then(|p| self.loader.mtl(p));
        let entry = match mtl.as_deref().and_then(|m| m.sub(sub).or(Some(m))) {
            Some(m) => {
                let mark = self.pending.len();
                let converted = crate::shade::convert_material(self, m, ucc_uv_square);
                let deps: Vec<Shared> = self.pending[mark..]
                    .iter()
                    .filter_map(|p| {
                        if let Pending::Shared(s) = p {
                            Some(s.clone())
                        } else {
                            None
                        }
                    })
                    .collect();
                converted.map(|sm| MaterialEntry {
                    handle: new_handle(&self.loader.shared.materials),
                    slot: slot(sm),
                    deps: Arc::new(deps),
                })
            }
            None => return Some(self.default_material()),
        };
        let entry = lock(&self.loader.shared.materials_cache)
            .entry(key)
            .or_insert(entry)
            .clone();
        entry.map(|e| self.use_material(&e))
    }

    fn use_material(&mut self, e: &MaterialEntry) -> Handle<StandardMaterial> {
        for d in e.deps.iter() {
            self.push(d.clone());
        }
        self.push(Shared::Material(e.handle.clone(), e.slot.clone()));
        e.handle.clone()
    }

    /// A fresh (uncached) material for a sub-material with the caller's tweaks applied.
    pub fn material_with(
        &mut self,
        mtl_path: Option<&str>,
        sub: usize,
        hook: &MaterialHook,
    ) -> Option<Handle<StandardMaterial>> {
        self.material_with_domain(mtl_path, sub, hook, true)
    }

    fn material_with_domain(
        &mut self,
        mtl_path: Option<&str>,
        sub: usize,
        hook: &MaterialHook,
        ucc_uv_square: bool,
    ) -> Option<Handle<StandardMaterial>> {
        let _t = crate::prof::scope("cry.material_with");
        let mtl = mtl_path.and_then(|p| self.loader.mtl(p));
        let cry = mtl.as_deref().and_then(|m| m.sub(sub).or(Some(m))).cloned();
        let mut sm = match &cry {
            Some(m) => crate::shade::convert_material(self, m, ucc_uv_square)?,
            None => StandardMaterial {
                base_color: Color::srgb(0.6, 0.6, 0.6),
                perceptual_roughness: 0.8,
                ..default()
            },
        };
        apply_material_hook(&cry.unwrap_or_default(), &mut sm, hook);
        let handle = new_handle(&self.loader.shared.materials);
        self.push(Shared::Material(handle.clone(), slot(sm)));
        Some(handle)
    }

    fn mesh(&mut self, mesh: Mesh) -> Handle<Mesh> {
        let handle = new_handle(&self.loader.shared.meshes);
        self.pending.push(Pending::Mesh(handle.clone(), mesh));
        handle
    }
}

/// Eye emissive represents extra ambient multiplied by the same albedo as the base colour.
/// A custom face hook often replaces only the base texture. Follow that replacement when the
/// inherited emissive texture was linked to albedo and the hook did not explicitly change it.
fn apply_material_hook(m: &CryMaterial, sm: &mut StandardMaterial, hook: &MaterialHook) {
    let inherited = sm.base_color_texture.clone();
    let follow = m.shader.eq_ignore_ascii_case("eye")
        && inherited.is_some()
        && sm.emissive_texture == inherited;
    (hook.0)(m, sm);
    if follow && sm.base_color_texture != inherited && sm.emissive_texture == inherited {
        sm.emissive_texture = sm.base_color_texture.clone();
    }
}

impl CryLoader {
    /// Parses a model and builds every Bevy asset it needs; callable from any thread. Finish it
    /// with [`crate::CryAssets::finish_model`] on the main thread.
    ///
    /// Without hooks in `options` the result is cached and shared (a second call for the same
    /// path and options is free once the first one was finished); with `material_hook` /
    /// `vertex_hook` every call builds its own meshes and materials (the parsed file is still
    /// shared).
    pub fn prepare_model(&self, path: &str, options: &ModelOptions) -> Result<PreparedModel> {
        let _t = crate::prof::scope(if options.material_hook.is_some() || options.vertex_hook.is_some() { "cry.prepare_model(hooked)" } else { "cry.prepare_model" });
        let path = aa_pak::normalize(path);
        let hooked = options.material_hook.is_some() || options.vertex_hook.is_some();
        let key: ModelKey = (path.clone(), options.material.as_deref().map(aa_pak::normalize), options.lod, options.keep_cpu_mesh, options.shading);
        let mut prep = Prep::new(self);
        prep.shading = options.shading;
        let mut register = Vec::new();

        let cached = if hooked { None } else { lock(&self.shared.models).get(&key).cloned() };
        let loaded = match cached {
            Some(m) => m,
            None => {
                let m = Arc::new(self.build_model(&mut prep, &path, options)?);
                if !hooked {
                    register.push((CacheKey::Plain(key.clone()), m.clone()));
                }
                m
            }
        };
        let (Some(binding), Some(own)) = (&options.skeleton, &loaded.skeleton) else {
            return Ok(PreparedModel { loaded, pending: prep.pending, register });
        };
        let bound_key = (key, Arc::as_ptr(&binding.skeleton) as usize);
        if !hooked {
            if let Some(m) = lock(&self.shared.bound).get(&bound_key).cloned() {
                return Ok(PreparedModel { loaded: m, pending: prep.pending, register });
            }
        }
        // CryEngine skins attachments with the base skeleton's default pose: the part's own
        // bind poses (which differ, e.g. by 0.5 at a costume's neck) are only used for bones
        // the base does not have.
        let map = own.map_to(&binding.skeleton);
        let ibp: Vec<Mat4> = own
            .bones
            .iter()
            .zip(&map)
            .map(|(b, m)| m.map(|j| binding.skeleton.bones[j].inverse_bind()).unwrap_or_else(|| b.inverse_bind()))
            .collect();
        let handle = new_handle(&self.shared.bindposes);
        prep.pending.push(Pending::Bindposes(handle.clone(), SkinnedMeshInverseBindposes::from(ibp)));
        let bound = Arc::new(LoadedModel { inverse_bindposes: Some(handle), ..(*loaded).clone() });
        if !hooked {
            register.push((CacheKey::Bound(bound_key.0.clone(), bound_key.1), bound.clone()));
        }
        Ok(PreparedModel { loaded: bound, pending: prep.pending, register })
    }

    fn build_model(&self, prep: &mut Prep<'_>, path: &str, options: &ModelOptions) -> Result<LoadedModel> {
        let (file_path, model) = self.load_lod(path, options.lod)?;
        // embedded LOD level to draw: the requested one if this file has it (and we did not
        // switch to a separate _lodN file), else 0
        let embedded = if file_path == path && options.lod > 0 {
            (options.lod.min(model.max_embedded_lod() as u32)) as u8
        } else {
            0
        };
        // The file's own bones (also for skeleton-only .chr); LOD files without bones use their
        // LOD0 skeleton.
        let skeleton = match &model.skeleton {
            Some(s) => Some(Arc::new(s.clone())),
            None if model.is_skinned() => Skeleton::load(&file_path).ok().map(Arc::new),
            None => None,
        };
        let material_path = options.material.clone().or_else(|| find_mtl(&file_path, &model));
        let material_data = material_path.as_deref().and_then(|p| self.mtl(p));
        let usage = if options.keep_cpu_mesh {
            RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD
        } else {
            RenderAssetUsages::RENDER_WORLD
        };
        let mut nodes = model.lod_nodes(embedded);
        if nodes.is_empty() && embedded > 0 {
            nodes = model.lod_nodes(0);
        }
        let mut parts = Vec::new();
        for ni in nodes {
            let Some(mi) = model.nodes[ni].mesh else { continue };
            let mesh = &model.meshes[mi];
            // Skeleton files (all 17 `*_base.chr`) carry a placeholder: one 10 cm triangle at
            // the origin without a material. Not drawn.
            if model.skeleton.is_some() && mesh.indices.len() <= 3 {
                continue;
            }
            let skinned = mesh.skin.is_some() && skeleton.is_some();
            // mirrored nodes (rare: ~1 in 3,000 files); meshes are not shared between nodes
            let flip = !skinned && render_transform(&model, ni).matrix3.determinant() < 0.0;
            let edited = options.vertex_hook.as_ref().map(|h| {
                let mut m = mesh.clone();
                (h.0)(&mut m);
                m
            });
            let data = edited.as_ref().unwrap_or(mesh);
            let mut node_parts = Vec::new();
            for subset in &mesh.subsets {
                let is_ucc = material_data
                    .as_deref()
                    .and_then(|m| m.sub(subset.material_id as usize).or(Some(m)))
                    .is_some_and(|m| {
                        m.shader.eq_ignore_ascii_case("cloth") && m.gen_mask & 0x10_0000 != 0
                    });
                let ucc_uv_square = !is_ucc || data.subset_uvs_in_unit_square(subset);
                let material = match &options.material_hook {
                    Some(hook) => prep.material_with_domain(material_path.as_deref(), subset.material_id as usize, hook, ucc_uv_square),
                    None => prep.material_for_domain(material_path.as_deref(), subset.material_id as usize, ucc_uv_square),
                };
                let Some(material) = material else { continue };
                let t_mesh = crate::prof::scope("cry.mesh");
                let built = subset_mesh_with(data, subset, skinned, usage, flip);
                drop(t_mesh);
                let Some((mut bevy_mesh, aabb)) = built else { continue };
                let vertex_color = material_data
                    .as_deref()
                    .and_then(|m| m.sub(subset.material_id as usize).or(Some(m)))
                    .is_some_and(crate::shade::illum::uses_vertex_color);
                if vertex_color {
                    crate::scene::insert_illum_vertex_colors(&mut bevy_mesh, data, subset, flip);
                }
                node_parts.push(Part { mesh: prep.mesh(bevy_mesh), material, aabb, skinned });
            }
            if !node_parts.is_empty() {
                parts.push((ni, node_parts));
            }
        }
        let inverse_bindposes = skeleton.as_ref().map(|s| {
            let handle = new_handle(&self.shared.bindposes);
            prep.pending.push(Pending::Bindposes(
                handle.clone(),
                SkinnedMeshInverseBindposes::from(s.bones.iter().map(|b| b.inverse_bind()).collect::<Vec<_>>()),
            ));
            handle
        });
        Ok(LoadedModel { model, skeleton, inverse_bindposes, material_path, parts })
    }

    /// Registers a finished model in the cache (called by `finish_model`).
    pub(crate) fn register(&self, prepared: &PreparedModel) {
        for (key, model) in &prepared.register {
            match key {
                CacheKey::Plain(k) => {
                    lock(&self.shared.models).insert(k.clone(), model.clone());
                }
                CacheKey::Bound(k, ptr) => {
                    lock(&self.shared.bound).insert((k.clone(), *ptr), model.clone());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_cat_cutouts_survive_opaque_smallest_mip_and_cached_opacity_checks() {
        if !crate::pak_available() {
            return;
        }
        let images = Assets::<Image>::default();
        let meshes = Assets::<Mesh>::default();
        let materials = Assets::<StandardMaterial>::default();
        let binds = Assets::<SkinnedMeshInverseBindposes>::default();
        let loader = CryLoader::new(
            images.get_handle_provider(), meshes.get_handle_provider(),
            materials.get_handle_provider(), binds.get_handle_provider(), false,
        );
        let path = "objects/characters/animals/cat_baby_pet/cat_baby_black_pet_df.dds";
        assert_eq!(loader.mean_alpha(path), Some(1.0));
        assert_eq!(loader.alpha_is_opaque(path), Some(false));
        let alias = format!("GAME/{}", path.to_ascii_uppercase());
        assert_eq!(loader.alpha_is_opaque(&alias), Some(false));
        assert_eq!(lock(&loader.shared.opaque_alpha).len(), 1, "normalised cache ignores colour space");
        assert_eq!(loader.alpha_is_opaque("missing-alpha-proof.dds"), None);
        let mtl = loader.mtl("objects/characters/animals/cat_baby_pet/cat_baby_black_pet.mtl").unwrap();
        let m = mtl.sub(0).unwrap();
        for shading in [Shading::Basic, Shading::Full] {
            let mut prep = Prep::new(&loader);
            prep.shading = shading;
            assert_eq!(prep.convert(m).unwrap().alpha_mode, AlphaMode::Mask(0.5));
            let mut blend = m.clone();
            blend.opacity = 0.5;
            assert_eq!(prep.convert(&blend).unwrap().alpha_mode, AlphaMode::Blend);
            let mut hidden = m.clone();
            hidden.flags |= crate::material::flags::NO_DRAW;
            assert!(prep.convert(&hidden).is_none());
            // Genuine opaque BC1 still elides the redundant test. An out-of-range alpha ref
            // must remain a mask even when all source texels have alpha one.
            let mut opaque = m.clone();
            opaque.textures.iter_mut()
                .find(|t| t.map == crate::material::TextureMap::Diffuse).unwrap()
                .file = "objects/env/07_ucc/ucc_pattern04.dds".into();
            assert_eq!(prep.convert(&opaque).unwrap().alpha_mode, AlphaMode::Opaque);
            opaque.alpha_test = 1.1;
            assert_eq!(prep.convert(&opaque).unwrap().alpha_mode, AlphaMode::Mask(1.1));
        }
    }

    #[test]
    fn eye_hooks_follow_albedo_but_keep_explicit_emissive_overrides() {
        let mut images = Assets::<Image>::default();
        let old = images.add(Image::default());
        let custom = images.add(Image::default());
        let explicit = images.add(Image::default());
        let m = CryMaterial {
            shader: "Eye".into(),
            ..default()
        };
        let material = StandardMaterial {
            base_color_texture: Some(old.clone()),
            emissive_texture: Some(old.clone()),
            ..default()
        };
        let mut unchanged = material.clone();
        apply_material_hook(&m, &mut unchanged, &MaterialHook::new(|_, _| {}));
        assert_eq!(unchanged.emissive_texture, Some(old.clone()));
        let face = custom.clone();
        let mut changed = material.clone();
        apply_material_hook(&m, &mut changed, &MaterialHook::new(move |_, out| {
            out.base_color_texture = Some(face.clone());
        }));
        assert_eq!(changed.emissive_texture, Some(custom.clone()));
        let face = custom.clone();
        let emission = explicit.clone();
        let mut overridden = material.clone();
        apply_material_hook(&m, &mut overridden, &MaterialHook::new(move |_, out| {
            out.base_color_texture = Some(face.clone());
            out.emissive_texture = Some(emission.clone());
        }));
        assert_eq!(overridden.emissive_texture, Some(explicit));
        let other = CryMaterial {
            shader: "Illum".into(),
            ..default()
        };
        let mut independent = material;
        apply_material_hook(&other, &mut independent, &MaterialHook::new(move |_, out| {
            out.base_color_texture = Some(custom.clone());
        }));
        assert_eq!(independent.emissive_texture, Some(old));
    }

    #[test]
    fn loader_is_send_and_sync() {
        fn is<T: Send + Sync>() {}
        is::<CryLoader>();
        is::<PreparedModel>();
    }

    #[test]
    fn ucc_material_cache_and_model_preparation_separate_uv_domains() {
        if !crate::pak_available() {
            return;
        }
        for first_domain in [false, true] {
            let images = Assets::<Image>::default();
            let meshes = Assets::<Mesh>::default();
            let materials = Assets::<StandardMaterial>::default();
            let binds = Assets::<SkinnedMeshInverseBindposes>::default();
            let loader = CryLoader::new(
                images.get_handle_provider(),
                meshes.get_handle_provider(),
                materials.get_handle_provider(),
                binds.get_handle_provider(),
                false,
            );
            let mtl = "objects/env/06_unit/01_ship/galleon/galleon_mahadevi.mtl";
            let mut prep = Prep::new(&loader);
            let first = prep
                .material_for_domain(Some(mtl), 3, first_domain)
                .unwrap();
            let second = prep
                .material_for_domain(Some(mtl), 3, !first_domain)
                .unwrap();
            assert_ne!(
                first, second,
                "material cache includes bake eligibility in either load order"
            );
            let safe = if first_domain { &first } else { &second };
            let unsafe_domain = if first_domain { &second } else { &first };
            let path = "objects/env/06_unit/01_ship/galleon/galleon_deco_sail01.cgf";
            let options = ModelOptions {
                material: Some(mtl.into()),
                ..default()
            };
            let good = loader.prepare_model(path, &options).unwrap();
            assert!(
                good.loaded
                    .parts
                    .iter()
                    .flat_map(|(_, parts)| parts)
                    .any(|p| p.material == *safe)
            );
            let options = ModelOptions {
                vertex_hook: Some(crate::scene::VertexHook::new(|mesh| {
                    for uv in &mut mesh.uvs {
                        uv.x += 2.0;
                    }
                })),
                ..options
            };
            let bad = loader.prepare_model(path, &options).unwrap();
            assert!(
                bad.loaded
                    .parts
                    .iter()
                    .flat_map(|(_, parts)| parts)
                    .any(|p| p.material == *unsafe_domain)
            );
            assert!(
                !bad.loaded
                    .parts
                    .iter()
                    .flat_map(|(_, parts)| parts)
                    .any(|p| p.material == *safe)
            );
        }
    }
}
