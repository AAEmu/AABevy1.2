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
//! (`Mesh3d` + `MeshMaterial3d`). Skinned models (`.chr`) instead get joint
//! entities (`CryJoint`, local bind pose) under the root and their meshes carry a
//! `SkinnedMesh`; the root gets a [`CrySkeleton`] listing the joint entities.
//!
//! The heavy work (parsing, texture decoding, mesh building) lives in [`crate::loader`] and can
//! run on worker threads: [`CryAssets::loader`] + [`CryLoader::prepare_model`] +
//! [`CryAssets::finish_model`]. [`CryAssets::load_model`] does both on the calling thread.
//!
//! **Orientation.** CryEngine space is Z up with characters facing **+Y** (their left is -X).
//! [`CRY_TO_BEVY`] maps `(x, y, z)` to `(x, z, -y)`, so after it a character faces Bevy **-Z**
//! (props authored facing -Y are the exception). To show a character to a camera on +Z, turn the
//! root by PI about Y.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::ecs::system::SystemParam;
use bevy::image::{CompressedImageFormatSupport, CompressedImageFormats};
use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use crate::loader::{CryLoader, Pending, Prep, PreparedModel, Shared, lock};
use crate::material::Material as CryMaterial;
use crate::mesh::{MeshData, Model, NodeKind, Subset};
use crate::skeleton::Skeleton;
use crate::{CRY_TO_BEVY, Result};

pub use crate::shade::{SKIN_ALBEDO, shininess_to_roughness, to_standard_material};

/// Registers the asset cache used by [`CryAssets`].
pub struct CryPlugin;

impl Plugin for CryPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CryCache>();
        // the hair material needs the renderer (headless tests add `CryPlugin` without it)
        if app.is_plugin_added::<bevy::pbr::PbrPlugin>() && !app.is_plugin_added::<crate::shade::hair::HairPlugin>() {
            app.add_plugins(crate::shade::hair::HairPlugin);
        }
        if app.is_plugin_added::<bevy::pbr::PbrPlugin>() && !app.is_plugin_added::<crate::shade::skin::SkinPlugin>() {
            app.add_plugins(crate::shade::skin::SkinPlugin);
        }
        if app.is_plugin_added::<bevy::pbr::PbrPlugin>() && !app.is_plugin_added::<crate::shade::illum::IllumPlugin>() {
            app.add_plugins(crate::shade::illum::IllumPlugin);
        }
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

/// Edits each sub-material's `StandardMaterial` right after the conversion from the `.mtl`
/// (given the CryEngine sub-material it came from), e.g. to tint skin or swap a texture.
/// Runs on the thread that prepares the model, so it must be `Send + Sync`.
#[derive(Clone)]
pub struct MaterialHook(pub Arc<dyn Fn(&CryMaterial, &mut StandardMaterial) + Send + Sync>);

impl MaterialHook {
    pub fn new(f: impl Fn(&CryMaterial, &mut StandardMaterial) + Send + Sync + 'static) -> Self {
        MaterialHook(Arc::new(f))
    }
}

impl std::fmt::Debug for MaterialHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MaterialHook")
    }
}

/// Edits the vertex data of every mesh of a model before the Bevy meshes are built (positions,
/// normals, UVs... in CryEngine model space), e.g. for morph targets.
#[derive(Clone)]
pub struct VertexHook(pub Arc<dyn Fn(&mut MeshData) + Send + Sync>);

impl VertexHook {
    pub fn new(f: impl Fn(&mut MeshData) + Send + Sync + 'static) -> Self {
        VertexHook(Arc::new(f))
    }
}

impl std::fmt::Debug for VertexHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VertexHook")
    }
}

/// How much work the material conversion may spend on one material.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Shading {
    /// StandardMaterial lighting with native Illum vertex RGB where enabled: world props.
    #[default]
    Basic,
    /// Native Illum, HumanSkin and Hair material composition with their maps; for characters
    /// and close-up objects. `AA_ILLUM_LEGACY=1` retains the old Illum metallic/roughness bake.
    Full,
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
    /// Restyle every sub-material after the conversion. With a hook the model is not shared:
    /// each load builds its own materials and meshes.
    pub material_hook: Option<MaterialHook>,
    /// Edit the vertices (morphs) before the meshes are built; same sharing rule as
    /// `material_hook`.
    pub vertex_hook: Option<VertexHook>,
    /// Material conversion quality, see [`Shading`].
    pub shading: Shading,
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

/// Caches of converted assets, keyed by normalised pak path (the shared state lives in the
/// [`CryLoader`]).
#[derive(Resource, Default)]
pub struct CryCache {
    loader: Option<CryLoader>,
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

    /// The thread-safe loader (cheap to clone, `Send + Sync`): use it on worker threads to
    /// [`CryLoader::prepare_model`], then call [`CryAssets::finish_model`] here.
    pub fn loader(&mut self) -> CryLoader {
        if let Some(l) = &self.cache.loader {
            return l.clone();
        }
        let l = CryLoader::new(
            self.images.get_handle_provider(),
            self.meshes.get_handle_provider(),
            self.materials.get_handle_provider(),
            self.bindposes.get_handle_provider(),
            self.bc_supported(),
        );
        self.cache.loader = Some(l.clone());
        l
    }

    /// Moves prepared asset values into the `Assets` collections.
    fn insert_pending(&mut self, pending: Vec<Pending>) {
        for p in pending {
            match p {
                Pending::Shared(Shared::Image(h, s)) => {
                    if let Some(v) = lock(&s).take() {
                        let _ = self.images.insert(h.id(), v);
                    }
                }
                Pending::Shared(Shared::Material(h, s)) => {
                    if let Some(v) = lock(&s).take() {
                        let _ = self.materials.insert(h.id(), v);
                    }
                }
                Pending::Mesh(h, v) => {
                    let _ = self.meshes.insert(h.id(), v);
                }
                Pending::Bindposes(h, v) => {
                    let _ = self.bindposes.insert(h.id(), v);
                }
            }
        }
    }

    fn sync_missing(&mut self, loader: &CryLoader) {
        let missing = loader.take_missing();
        self.cache.missing_textures.extend(missing);
    }

    /// Runs a preparation step on this thread and inserts what it produced.
    fn with_prep<R>(&mut self, f: impl FnOnce(&mut Prep<'_>) -> R) -> R {
        let loader = self.loader();
        let mut prep = Prep::new(&loader);
        let r = f(&mut prep);
        let pending = prep.into_pending();
        self.insert_pending(pending);
        self.sync_missing(&loader);
        r
    }

    /// A pak texture as an image handle (cached). `srgb` for colour maps. Textures whose path
    /// moved are found by file name (about 1% of material references), ` copy` is stripped from
    /// names like `x_df copy.dds`. Cube / volume maps are not 2D textures: `None` (see
    /// [`CryAssets::cube_texture`]).
    pub fn texture(&mut self, path: &str, srgb: bool) -> Option<Handle<Image>> {
        self.with_prep(|p| p.texture(path, srgb))
    }

    /// A cube map of the pak (e.g. `textures/cubemaps/beach_blur2.dds`), for environment lighting.
    pub fn cube_texture(&mut self, path: &str) -> Option<Handle<Image>> {
        self.with_prep(|p| p.cube_texture(path))
    }

    /// A parsed `.mtl` (cached); `None` if missing or unreadable.
    pub fn mtl(&mut self, path: &str) -> Option<Arc<CryMaterial>> {
        self.loader().mtl(path)
    }

    /// The Bevy material for sub-material `sub` of a `.mtl` (cached). `None` when the
    /// sub-material is not drawn (NoDraw, Cut, proxies).
    /// Manually assigned UCC materials require mesh UVs in 0..1; model loading validates each
    /// subset and keeps the original material for unsupported domains.
    pub fn material(&mut self, mtl_path: Option<&str>, sub: usize) -> Option<Handle<StandardMaterial>> {
        self.with_prep(|p| p.material(mtl_path, sub))
    }

    /// Loads (or reuses) a model's Bevy assets without spawning anything. With
    /// `options.skeleton` the inverse bind poses are the base skeleton's (see
    /// [`SkeletonBinding`]). Equivalent to `loader().prepare_model(..)` + `finish_model(..)`.
    pub fn load_model(&mut self, path: &str, options: &ModelOptions) -> Result<Arc<LoadedModel>> {
        let prepared = self.loader().prepare_model(path, options)?;
        Ok(self.finish_model(prepared))
    }

    /// Inserts the assets of a model prepared with [`CryLoader::prepare_model`] (main thread,
    /// cheap: moves values) and returns the model. The model's handles were valid since the
    /// preparation; the assets behind them exist from now on.
    pub fn finish_model(&mut self, prepared: PreparedModel) -> Arc<LoadedModel> {
        let _t = crate::prof::scope("cry.finish_model");
        let loader = self.loader();
        loader.register(&prepared);
        let loaded = prepared.loaded.clone();
        self.insert_pending(prepared.pending);
        self.sync_missing(&loader);
        loaded
    }

    /// Spawns a model with default options; returns the root entity.
    pub fn spawn_model(&mut self, commands: &mut Commands, path: &str) -> Result<Entity> {
        self.spawn_model_with(commands, path, &ModelOptions::default())
    }

    pub fn spawn_model_with(&mut self, commands: &mut Commands, path: &str, options: &ModelOptions) -> Result<Entity> {
        Ok(self.spawn_model_full(commands, path, options)?.root)
    }

    /// Like [`CryAssets::spawn_model_with`], also returning the joint entities.
    pub fn spawn_model_full(&mut self, commands: &mut Commands, path: &str, options: &ModelOptions) -> Result<SpawnedModel> {
        let loaded = self.load_model(path, options)?;
        Ok(spawn_loaded_full(commands, path, &loaded, options))
    }
}

/// Entities of a spawned model.
#[derive(Clone, Debug)]
pub struct SpawnedModel {
    pub root: Entity,
    /// One per bone of [`LoadedModel::skeleton`] (bound joints when a [`SkeletonBinding`] was
    /// given); empty for static models.
    pub joints: Vec<Entity>,
}

/// Spawns an already loaded model (cheap; usable for many instances); returns the root.
pub fn spawn_loaded(commands: &mut Commands, path: &str, loaded: &LoadedModel, options: &ModelOptions) -> Entity {
    spawn_loaded_full(commands, path, loaded, options).root
}

/// [`spawn_loaded`], also returning the joint entities.
pub fn spawn_loaded_full(commands: &mut Commands, path: &str, loaded: &LoadedModel, options: &ModelOptions) -> SpawnedModel {
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
        // Bound skins reuse the base skeleton's joints; bones the base does not have (extra
        // hair / cloth bones) get their own joint under their parent.
        let map = options.skeleton.as_ref().map(|b| skel.map_to(&b.skeleton));
        for (i, bone) in skel.bones.iter().enumerate() {
            if let (Some(binding), Some(map)) = (&options.skeleton, &map) {
                if let Some(e) = map[i].and_then(|j| binding.joints.get(j).copied()) {
                    joints.push(e);
                    continue;
                }
            }
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
        if options.skeleton.is_none() {
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
    SpawnedModel { root, joints }
}

/// Model-space transform a static node's mesh is drawn with by [`spawn_loaded`]: the node chain,
/// where LOD nodes contribute identity (they are drawn in their parent's frame). Skinned
/// meshes ignore node transforms.
pub fn render_transform(model: &Model, node: usize) -> glam::Affine3A {
    let mut m = glam::Affine3A::IDENTITY;
    let mut i = Some(node);
    let mut guard = 0;
    while let Some(n) = i {
        let n = &model.nodes[n];
        if !matches!(n.kind, NodeKind::Lod(_)) {
            m = n.local * m;
        }
        i = n.parent;
        guard += 1;
        if guard > model.nodes.len() {
            break;
        }
    }
    m
}

/// Material lookup as the engine does it: `.chr` -> `<name>.mtl` first; `.cgf` -> the
/// material name stored in the file, relative to the file's folder, then `<name>.mtl`.
pub fn find_mtl(path: &str, model: &Model) -> Option<String> {
    let pak = aa_pak::Pak::global();
    crate::physics::material_paths(path, model.material_name.as_deref()).into_iter().find(|c| pak.exists(c))
}

/// Illum `%VERTCOLORS` (gen mask `0x400000`): the native shader multiplies the final lit RGB by the
/// interpolated vertex RGB squared (`fraglib.cfi` 766-768). Store the unsquared, normalized source bytes;
/// [`crate::shade::illum::IllumMaterial`] applies the square after lighting. Alpha stays 1: `%VERTCOLORS`
/// never changes opacity (`%VERTALPHA` is separate). Call only for an Illum subset using `%VERTCOLORS`:
/// other shaders assign different meanings to this stream (e.g. vegetation wind).
/// Native D3D10 uploads swap R/B (`CRenderMesh::SetMesh_IntImpl`, helpers 0x38206540/0x382065A0)
/// and `GetInputColor(Color.zyxw)` swaps back, so the effective colour is the original stream RGB.
pub fn insert_illum_vertex_colors(mesh: &mut Mesh, m: &MeshData, s: &Subset, flip_winding: bool) -> bool {
    let range = s.first_index as usize..(s.first_index + s.index_count) as usize;
    let Some(idx) = m.indices.get(range) else { return false };
    if m.colors.len() != m.positions.len() {
        return false;
    }
    // the compaction order of subset_mesh_with: first use in (possibly flipped) index order
    let mut seen = vec![false; m.positions.len()];
    let mut colors = Vec::new();
    let ordered: Vec<_> = if flip_winding {
        idx.chunks_exact(3).flat_map(|t| [t[0], t[2], t[1]]).collect()
    } else {
        idx.to_vec()
    };
    for i in ordered {
        let Some(used) = seen.get_mut(i as usize) else { return false };
        if !std::mem::replace(used, true) {
            let c = m.colors[i as usize];
            colors.push([c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0]);
        }
    }
    if colors.len() != mesh.count_vertices() {
        return false;
    }
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    true
}

/// One subset as a Bevy mesh with its own compacted vertices (+ local bounds).
pub fn subset_mesh(m: &MeshData, s: &Subset, skinned: bool, usage: RenderAssetUsages) -> Option<(Mesh, Aabb)> {
    subset_mesh_with(m, s, skinned, usage, false)
}

/// [`subset_mesh`]; `flip_winding` reverses every triangle. Bevy culls by winding only and does
/// not account for mirrored transforms, so meshes drawn under a transform with a negative
/// determinant (see [`render_transform`]) need it.
pub fn subset_mesh_with(m: &MeshData, s: &Subset, skinned: bool, usage: RenderAssetUsages, flip_winding: bool) -> Option<(Mesh, Aabb)> {
    let range = s.first_index as usize..(s.first_index + s.index_count) as usize;
    let idx = m.indices.get(range)?;
    if idx.is_empty() {
        return None;
    }
    let flipped: Vec<u32>;
    let idx = if flip_winding {
        flipped = idx.chunks_exact(3).flat_map(|t| [t[0], t[2], t[1]]).collect();
        &flipped[..]
    } else {
        idx
    };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn illum_vertex_colors_preserve_rgb_in_compacted_and_mirrored_order_without_affecting_alpha() {
        let m = MeshData {
            positions: vec![Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z],
            normals: vec![Vec3::Z; 4],
            colors: vec![[255, 0, 0, 7], [0, 128, 0, 7], [0, 0, 255, 7], [51, 51, 51, 7]],
            indices: vec![3, 1, 2],
            ..Default::default()
        };
        let s = Subset { first_index: 0, index_count: 3, first_vertex: 0, vertex_count: 4, material_id: 0, radius: 1.0, center: Vec3::ZERO };
        for flip in [false, true] {
            let (mut mesh, _) = subset_mesh_with(&m, &s, false, RenderAssetUsages::MAIN_WORLD, flip).unwrap();
            assert!(insert_illum_vertex_colors(&mut mesh, &m, &s, flip));
            let Some(bevy::mesh::VertexAttributeValues::Float32x4(c)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR) else { panic!() };
            let Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) else { panic!() };
            for (c, p) in c.iter().zip(p) {
                let source = m.positions.iter().position(|q| q.to_array() == *p).unwrap();
                let want = m.colors[source].map(|v| v as f32 / 255.0);
                assert_eq!(*c, [want[0], want[1], want[2], 1.0]);
            }
        }
        let mut plain = m.clone();
        plain.colors.clear();
        let (mut mesh, _) = subset_mesh_with(&plain, &s, false, RenderAssetUsages::MAIN_WORLD, false).unwrap();
        assert!(!insert_illum_vertex_colors(&mut mesh, &plain, &s, false));
    }
}
