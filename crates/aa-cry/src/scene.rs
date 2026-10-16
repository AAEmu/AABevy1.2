//! Bevy side: textures, materials and meshes as Bevy assets (cached), and model spawning.
//!
//! ```no_run
//! use bevy::prelude::*;
//! use aa_cry::scene::{CryAssets, CryPlugin};
//!
//! fn setup(mut commands: Commands, mut cry: CryAssets) {
//!     let root = cry.spawn_model(&mut commands, "objects/env/01_nuia/001_ncom/ncom_city_signpost_c.cgf").unwrap();
//!     commands.entity(root).insert(Transform::from_xyz(0.0, 0.0, 0.0).with_rotation(aa_cry::CRY_TO_BEVY));
//! }
//! # fn main() { App::new().add_plugins((DefaultPlugins, CryPlugin)).add_systems(Startup, setup).run(); }
//! ```
//!
//! Spawned hierarchy: `root (CryModel, Transform = CRY_TO_BEVY unless cry_space)` ->
//! one child per file node (`CryNode`, node transform) -> one child per drawn subset
//! (`Mesh3d` + `MeshMaterial3d<StandardMaterial>`). Skinned models (`.chr`) instead get joint
//! entities (`CryJoint`, local bind pose) under the root and their meshes carry a
//! `SkinnedMesh`; the root gets a [`CrySkeleton`] listing the joint entities.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::ecs::system::SystemParam;
use bevy::image::{CompressedImageFormatSupport, CompressedImageFormats};
use bevy::math::Affine2;
use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::Face;

use crate::material::{Material as CryMaterial, TextureMap};
use crate::mesh::{MeshData, Model, NodeKind, Subset};
use crate::skeleton::Skeleton;
use crate::texture::DdsOptions;
use crate::{CRY_TO_BEVY, Result};

/// Registers the asset cache used by [`CryAssets`].
pub struct CryPlugin;

impl Plugin for CryPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CryCache>();
    }
}

/// Root of a spawned model.
#[derive(Component, Clone, Debug)]
pub struct CryModel {
    /// Pak path as given.
    pub path: String,
    pub model: Arc<Model>,
}

/// An entity standing for node `index` of the model file.
#[derive(Component, Clone, Copy, Debug)]
pub struct CryNode {
    pub index: usize,
}

/// A skeleton joint entity (bone `index` of the model's skeleton).
#[derive(Component, Clone, Copy, Debug)]
pub struct CryJoint {
    pub index: usize,
}

/// On the root of skinned models: the skeleton and its joint entities (same order as
/// `skeleton.bones`). Animate the joints' `Transform`s (local, CryEngine space).
#[derive(Component, Clone, Debug)]
pub struct CrySkeleton {
    pub skeleton: Arc<Skeleton>,
    pub joints: Vec<Entity>,
}

/// Drive a skin with joints that already exist (e.g. a character's base skeleton): bones of the
/// skin are matched to `skeleton` by controller id / name.
#[derive(Clone, Debug)]
pub struct SkeletonBinding {
    pub skeleton: Arc<Skeleton>,
    pub joints: Vec<Entity>,
}

#[derive(Clone, Debug, Default)]
pub struct ModelOptions {
    /// `.mtl` to use instead of the model's own (CDF `Material=`).
    pub material: Option<String>,
    /// Level of detail: 0 = full. For n > 0 a separate `*_lodN` file is used when it exists,
    /// else the embedded `$lodN` nodes, else the next lower level.
    pub lod: u32,
    /// Bind the skin to existing joints instead of spawning a skeleton.
    pub skeleton: Option<SkeletonBinding>,
    /// Leave the root transform at identity (content in CryEngine Z-up space) instead of
    /// [`CRY_TO_BEVY`]. Use when parenting under an already converted world root.
    pub cry_space: bool,
    /// Keep mesh data in the main world too (CPU access, e.g. for picking/raycasts).
    pub keep_cpu_mesh: bool,
}

/// One drawn piece of a node: a subset with its material.
#[derive(Clone, Debug)]
pub struct Part {
    pub mesh: Handle<Mesh>,
    pub material: Handle<StandardMaterial>,
    pub aabb: Aabb,
    pub skinned: bool,
}

/// A model converted to Bevy assets (shared between spawns of the same path/options).
#[derive(Clone, Debug)]
pub struct LoadedModel {
    pub model: Arc<Model>,
    /// Skeleton used by the skinned parts (the file's own or its LOD0's).
    pub skeleton: Option<Arc<Skeleton>>,
    pub inverse_bindposes: Option<Handle<SkinnedMeshInverseBindposes>>,
    /// `.mtl` actually used.
    pub material_path: Option<String>,
    /// (node index, parts), for the nodes drawn at the requested LOD.
    pub parts: Vec<(usize, Vec<Part>)>,
}

/// Caches of converted assets, keyed by normalised pak path.
#[derive(Resource, Default)]
pub struct CryCache {
    images: HashMap<(String, bool), Option<Handle<Image>>>,
    mtls: HashMap<String, Option<Arc<CryMaterial>>>,
    materials: HashMap<(String, usize), Option<Handle<StandardMaterial>>>,
    models: HashMap<(String, Option<String>, u32, bool), Arc<LoadedModel>>,
    default_material: Option<Handle<StandardMaterial>>,
    /// Texture paths that failed to load (for diagnostics).
    pub missing_textures: Vec<String>,
}

/// Everything needed to turn pak files into Bevy assets.
#[derive(SystemParam)]
pub struct CryAssets<'w> {
    pub meshes: ResMut<'w, Assets<Mesh>>,
    pub materials: ResMut<'w, Assets<StandardMaterial>>,
    pub images: ResMut<'w, Assets<Image>>,
    pub bindposes: ResMut<'w, Assets<SkinnedMeshInverseBindposes>>,
    pub cache: ResMut<'w, CryCache>,
    pub formats: Option<Res<'w, CompressedImageFormatSupport>>,
}

impl CryAssets<'_> {
    fn bc_supported(&self) -> bool {
        self.formats.as_ref().is_none_or(|f| f.0.contains(CompressedImageFormats::BC))
    }

    /// A pak texture as an image handle (cached). `srgb` for colour maps.
    pub fn texture(&mut self, path: &str, srgb: bool) -> Option<Handle<Image>> {
        let key = (crate::material::texture_pak_path(path), srgb);
        if key.0.is_empty() {
            return None;
        }
        if let Some(h) = self.cache.images.get(&key) {
            return h.clone();
        }
        let options = DdsOptions { srgb, bc_supported: self.bc_supported(), repeat: true };
        let handle = match crate::texture::load_dds_from_pak(&key.0, options) {
            Ok(img) => Some(self.images.add(img)),
            Err(e) => {
                debug!("aa-cry: texture {}: {e}", key.0);
                self.cache.missing_textures.push(key.0.clone());
                None
            }
        };
        self.cache.images.insert(key, handle.clone());
        handle
    }

    /// A parsed `.mtl` (cached); `None` if missing or unreadable.
    pub fn mtl(&mut self, path: &str) -> Option<Arc<CryMaterial>> {
        let key = aa_pak::normalize(path);
        if let Some(m) = self.cache.mtls.get(&key) {
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
        self.cache.mtls.insert(key, m.clone());
        m
    }

    /// The Bevy material for sub-material `sub` of a `.mtl` (cached). `None` when the
    /// sub-material is not drawn (NoDraw, Cut, proxies).
    pub fn material(&mut self, mtl_path: Option<&str>, sub: usize) -> Option<Handle<StandardMaterial>> {
        let key = (mtl_path.map(aa_pak::normalize).unwrap_or_default(), sub);
        if let Some(h) = self.cache.materials.get(&key) {
            return h.clone();
        }
        let mtl = mtl_path.and_then(|p| self.mtl(p));
        let handle = match mtl.as_deref().and_then(|m| m.sub(sub).or(Some(m))) {
            Some(m) => {
                let m = m.clone();
                to_standard_material(&m, |path, srgb| self.texture(path, srgb)).map(|sm| self.materials.add(sm))
            }
            None => Some(self.default_material()),
        };
        self.cache.materials.insert(key, handle.clone());
        handle
    }

    fn default_material(&mut self) -> Handle<StandardMaterial> {
        if let Some(h) = &self.cache.default_material {
            return h.clone();
        }
        let h = self.materials.add(StandardMaterial {
            base_color: Color::srgb(0.6, 0.6, 0.6),
            perceptual_roughness: 0.8,
            ..default()
        });
        self.cache.default_material = Some(h.clone());
        h
    }

    /// Loads (or reuses) a model's Bevy assets without spawning anything.
    pub fn load_model(&mut self, path: &str, options: &ModelOptions) -> Result<Arc<LoadedModel>> {
        let key = (aa_pak::normalize(path), options.material.as_deref().map(aa_pak::normalize), options.lod, options.keep_cpu_mesh);
        if let Some(m) = self.cache.models.get(&key) {
            return Ok(m.clone());
        }
        let (file_path, model) = load_lod(&key.0, options.lod)?;
        let model = Arc::new(model);
        // embedded LOD level to draw: the requested one if this file has it (and we did not
        // switch to a separate _lodN file), else 0
        let embedded = if file_path == key.0 && options.lod > 0 {
            (options.lod.min(model.max_embedded_lod() as u32)) as u8
        } else {
            0
        };
        let skeleton = if model.is_skinned() {
            match &model.skeleton {
                Some(s) => Some(Arc::new(s.clone())),
                None => Skeleton::load(&file_path).ok().map(Arc::new),
            }
        } else {
            None
        };
        let material_path = options.material.clone().or_else(|| find_mtl(&file_path, &model));
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
            let skinned = mesh.skin.is_some() && skeleton.is_some();
            let mut node_parts = Vec::new();
            for subset in &mesh.subsets {
                let Some(material) = self.material(material_path.as_deref(), subset.material_id as usize) else {
                    continue;
                };
                let Some((bevy_mesh, aabb)) = subset_mesh(mesh, subset, skinned, usage) else { continue };
                node_parts.push(Part { mesh: self.meshes.add(bevy_mesh), material, aabb, skinned });
            }
            if !node_parts.is_empty() {
                parts.push((ni, node_parts));
            }
        }
        let inverse_bindposes = skeleton.as_ref().map(|s| {
            self.bindposes.add(SkinnedMeshInverseBindposes::from(
                s.bones.iter().map(|b| b.inverse_bind()).collect::<Vec<_>>(),
            ))
        });
        let loaded = Arc::new(LoadedModel { model, skeleton, inverse_bindposes, material_path, parts });
        self.cache.models.insert(key, loaded.clone());
        Ok(loaded)
    }

    /// Spawns a model with default options; returns the root entity.
    pub fn spawn_model(&mut self, commands: &mut Commands, path: &str) -> Result<Entity> {
        self.spawn_model_with(commands, path, &ModelOptions::default())
    }

    pub fn spawn_model_with(&mut self, commands: &mut Commands, path: &str, options: &ModelOptions) -> Result<Entity> {
        let loaded = self.load_model(path, options)?;
        Ok(spawn_loaded(commands, path, &loaded, options))
    }
}

/// Spawns an already loaded model (cheap; usable for many instances).
pub fn spawn_loaded(commands: &mut Commands, path: &str, loaded: &LoadedModel, options: &ModelOptions) -> Entity {
    let model = &loaded.model;
    let root_tf = if options.cry_space { Transform::IDENTITY } else { Transform::from_rotation(CRY_TO_BEVY) };
    let root = commands
        .spawn((
            Name::new(path.rsplit('/').next().unwrap_or(path).to_string()),
            CryModel { path: path.to_string(), model: model.clone() },
            root_tf,
            Visibility::default(),
        ))
        .id();

    // joints
    let mut joints: Vec<Entity> = Vec::new();
    if let Some(skel) = &loaded.skeleton {
        if let Some(binding) = &options.skeleton {
            let map = skel.map_to(&binding.skeleton);
            joints = map
                .iter()
                .map(|m| m.and_then(|i| binding.joints.get(i).copied()).unwrap_or(binding.joints.first().copied().unwrap_or(root)))
                .collect();
        } else {
            for (i, bone) in skel.bones.iter().enumerate() {
                let parent = bone.parent.map(|p| joints[p]).unwrap_or(root);
                let (s, r, t) = bone.bind_local.to_scale_rotation_translation();
                let e = commands
                    .spawn((
                        Name::new(bone.name.clone()),
                        CryJoint { index: i },
                        Transform { translation: t, rotation: r, scale: s },
                        Visibility::default(),
                        ChildOf(parent),
                    ))
                    .id();
                joints.push(e);
            }
            commands.entity(root).insert(CrySkeleton { skeleton: skel.clone(), joints: joints.clone() });
        }
    }

    // nodes (only the ones with parts, plus their ancestors for transforms)
    let mut node_entity: HashMap<usize, Entity> = HashMap::new();
    fn node_entity_of(
        commands: &mut Commands,
        model: &Model,
        root: Entity,
        map: &mut HashMap<usize, Entity>,
        i: usize,
        depth: usize,
    ) -> Entity {
        if let Some(&e) = map.get(&i) {
            return e;
        }
        let n = &model.nodes[i];
        let parent = match n.parent {
            Some(p) if depth < 64 => node_entity_of(commands, model, root, map, p, depth + 1),
            _ => root,
        };
        // LOD nodes replace their parent: drawn in the parent's frame.
        let local = if matches!(n.kind, NodeKind::Lod(_)) { glam::Affine3A::IDENTITY } else { n.local };
        let (s, r, t) = local.to_scale_rotation_translation();
        let e = commands
            .spawn((
                Name::new(n.name.clone()),
                CryNode { index: i },
                Transform { translation: t, rotation: r, scale: s },
                Visibility::default(),
                ChildOf(parent),
            ))
            .id();
        map.insert(i, e);
        e
    }
    for (ni, parts) in &loaded.parts {
        let skinned = parts.iter().any(|p| p.skinned);
        // skinned vertices are in model space: hang them directly under the root
        let parent = if skinned { root } else { node_entity_of(commands, model, root, &mut node_entity, *ni, 0) };
        for part in parts {
            let mut e = commands.spawn((
                Mesh3d(part.mesh.clone()),
                MeshMaterial3d(part.material.clone()),
                Transform::IDENTITY,
                Visibility::default(),
                ChildOf(parent),
            ));
            if part.skinned {
                if let Some(ibp) = &loaded.inverse_bindposes {
                    e.insert((SkinnedMesh { inverse_bindposes: ibp.clone(), joints: joints.clone() }, NoFrustumCulling));
                }
            } else {
                e.insert(part.aabb);
            }
        }
    }
    root
}

/// Picks the file for a LOD: `*_lodN` if it exists (falling back to lower N), else the file.
fn load_lod(path: &str, lod: u32) -> Result<(String, Model)> {
    let pak = aa_pak::Pak::global();
    for l in (1..=lod).rev() {
        let p = crate::lod_path(path, l);
        if p != path && pak.exists(&p) {
            if let Ok(m) = Model::load(&p) {
                return Ok((p, m));
            }
        }
    }
    Ok((path.to_string(), Model::load(path)?))
}

/// Material lookup as the engine does it: `.chr` -> `<name>.mtl` first; `.cgf` -> the
/// material name stored in the file, relative to the file's folder, then `<name>.mtl`.
pub fn find_mtl(path: &str, model: &Model) -> Option<String> {
    let pak = aa_pak::Pak::global();
    let path = aa_pak::normalize(path);
    let base = crate::lod0_path(&path).unwrap_or(path.clone());
    let dir = base.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let same_name = format!("{}.mtl", base.rsplit_once('.').map(|(s, _)| s).unwrap_or(&base));
    let mut candidates = Vec::new();
    let is_chr = base.ends_with(".chr");
    if is_chr {
        candidates.push(same_name.clone());
    }
    if let Some(name) = &model.material_name {
        let name = name.replace('\\', "/");
        let name = name.trim_end_matches(".mtl");
        if name.contains('/') {
            candidates.push(format!("{}.mtl", aa_pak::normalize(name)));
        }
        let file = name.rsplit('/').next().unwrap_or(name);
        candidates.push(format!("{dir}/{}.mtl", file.to_ascii_lowercase()));
    }
    if !is_chr {
        candidates.push(same_name);
    }
    candidates.into_iter().find(|c| pak.exists(c))
}

/// One subset as a Bevy mesh with its own compacted vertices (+ local bounds).
pub fn subset_mesh(m: &MeshData, s: &Subset, skinned: bool, usage: RenderAssetUsages) -> Option<(Mesh, Aabb)> {
    let range = s.first_index as usize..(s.first_index + s.index_count) as usize;
    let idx = m.indices.get(range)?;
    if idx.is_empty() {
        return None;
    }
    let mut remap = vec![u32::MAX; m.positions.len()];
    let mut order: Vec<u32> = Vec::new();
    let indices: Vec<u32> = idx
        .iter()
        .map(|&i| {
            let r = &mut remap[i as usize];
            if *r == u32::MAX {
                *r = order.len() as u32;
                order.push(i);
            }
            *r
        })
        .collect();
    let pick3 = |v: &[Vec3]| order.iter().map(|&i| v[i as usize].to_array()).collect::<Vec<_>>();
    let positions = pick3(&m.positions);
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for p in &positions {
        lo = lo.min(Vec3::from(*p));
        hi = hi.max(Vec3::from(*p));
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, usage)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, pick3(&m.normals));
    if m.uvs.len() == m.positions.len() {
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, order.iter().map(|&i| m.uvs[i as usize].to_array()).collect::<Vec<_>>());
    } else {
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32, 0.0]; order.len()]);
    }
    if m.tangents.len() == m.positions.len() {
        mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, order.iter().map(|&i| m.tangents[i as usize].to_array()).collect::<Vec<_>>());
    }
    if skinned {
        if let Some(skin) = &m.skin {
            mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_INDEX, bevy::mesh::VertexAttributeValues::Uint16x4(
                order.iter().map(|&i| skin.joints[i as usize]).collect(),
            ));
            mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, order.iter().map(|&i| skin.weights[i as usize]).collect::<Vec<_>>());
        }
    }
    mesh.insert_indices(if order.len() <= u16::MAX as usize + 1 {
        Indices::U16(indices.iter().map(|&i| i as u16).collect())
    } else {
        Indices::U32(indices)
    });
    Some((mesh, Aabb::from_min_max(lo, hi)))
}

/// CryEngine material -> `StandardMaterial`; `None` for materials that are not drawn.
/// `texture(path, srgb)` resolves texture files. The mapping is documented in `docs/aa-cry.md`.
pub fn to_standard_material(
    m: &CryMaterial,
    mut texture: impl FnMut(&str, bool) -> Option<Handle<Image>>,
) -> Option<StandardMaterial> {
    let shader = m.shader.to_ascii_lowercase();
    if m.no_draw() || shader == "cut" {
        return None;
    }
    let diffuse_tex = m.texture(&TextureMap::Diffuse).map(|t| (t.file.clone(), t.tex_mod.clone()));
    let normal_tex = m
        .texture(&TextureMap::Normal)
        .or_else(|| m.texture(&TextureMap::Bump).filter(|t| t.file.to_ascii_lowercase().contains("_ddn")))
        .map(|t| t.file.clone());
    let mut out = StandardMaterial {
        base_color: Color::linear_rgb(m.diffuse.x, m.diffuse.y, m.diffuse.z),
        base_color_texture: diffuse_tex.as_ref().and_then(|(f, _)| texture(f, true)),
        normal_map_texture: normal_tex.as_deref().and_then(|f| texture(f, false)),
        perceptual_roughness: shininess_to_roughness(m.shininess),
        metallic: 0.0,
        reflectance: ((m.specular.x * 0.2126 + m.specular.y * 0.7152 + m.specular.z * 0.0722) * m.specular_level * 0.5).clamp(0.0, 1.0),
        emissive: LinearRgba::rgb(m.emissive.x, m.emissive.y, m.emissive.z),
        double_sided: m.two_sided(),
        cull_mode: if m.two_sided() { None } else { Some(Face::Back) },
        ..default()
    };
    if let Some((_, Some(tm))) = &diffuse_tex {
        out.uv_transform = Affine2::from_scale_angle_translation(
            Vec2::new(tm.tile_u, tm.tile_v),
            tm.rotate_w.to_radians(),
            Vec2::new(tm.offset_u, tm.offset_v),
        );
    }
    let two_sided = |o: &mut StandardMaterial| {
        o.double_sided = true;
        o.cull_mode = None;
    };
    match shader.as_str() {
        "vegetation" | "vegetationleaves" | "grass" => {
            two_sided(&mut out);
            out.alpha_mode = AlphaMode::Mask(if m.alpha_test > 0.0 { m.alpha_test } else { 0.5 });
        }
        "hair" => {
            two_sided(&mut out);
            out.alpha_mode = AlphaMode::Mask(if m.alpha_test > 0.0 { m.alpha_test } else { 0.5 });
        }
        "glass" | "liquid" | "ice" => {
            out.alpha_mode = AlphaMode::Blend;
            out.base_color.set_alpha(m.opacity.min(0.6));
            out.perceptual_roughness = 0.1;
        }
        "metal" => {
            out.metallic = 0.6;
        }
        s if s.starts_with("water") || s.starts_with("ocean") => {
            out.alpha_mode = AlphaMode::Blend;
            out.base_color = Color::linear_rgba(0.05, 0.12, 0.15, 0.7);
            out.perceptual_roughness = 0.05;
        }
        s if s.contains("particle") || s.starts_with("fx_") || s.starts_with("light") || s.contains("beam")
            || s.starts_with("sky") || s.contains("cloud") || s == "specialeffects" || s == "hologram" =>
        {
            out.unlit = true;
            out.alpha_mode = AlphaMode::Add;
            two_sided(&mut out);
        }
        _ => {}
    }
    if matches!(out.alpha_mode, AlphaMode::Opaque) {
        if m.flags & crate::material::flags::ADDITIVE != 0 {
            out.alpha_mode = AlphaMode::Add;
        } else if m.opacity < 0.999 {
            out.alpha_mode = AlphaMode::Blend;
            out.base_color.set_alpha(m.opacity);
        } else if m.alpha_test > 0.0 {
            out.alpha_mode = AlphaMode::Mask(m.alpha_test);
        }
    }
    Some(out)
}

/// Blinn-Phong exponent -> perceptual roughness (`alpha = sqrt(2 / (n + 2))`, `r = sqrt(alpha)`).
pub fn shininess_to_roughness(n: f32) -> f32 {
    (2.0 / (n.max(0.0) + 2.0)).powf(0.25).clamp(0.05, 1.0)
}
