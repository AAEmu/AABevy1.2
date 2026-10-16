//! Geometry of `.cgf` / `.cga` / `.chr` files as plain data (no Bevy).
//!
//! A file is a node hierarchy ([`Node`], chunk `0xCCCC000B` v0x823) whose nodes reference a
//! compiled mesh (`0xCCCC0000` v0x800) or a helper (`0xCCCC0001`). A mesh lists up to 16
//! `DataStream` chunks (`0xCCCC0016`) by type and a `MeshSubsets` chunk (`0xCCCC0017`):
//!
//! ```text
//! Mesh 0x800:  i32 flags, flags2, nVerts, nIndices, nSubsets, subsetsChunk, vertAnimChunk,
//!              i32 streamChunk[16], i32 physicsChunk[4], Vec3 bboxMin, bboxMax, ...
//! DataStream:  i32 flags, type, count, elementSize, reserved[2], data
//!   0 positions f32x3   1 normals f32x3   2 uvs f32x2   3 colours u8x4 (RGBA)   5 indices u16/u32
//!   6 tangents i16x4 tangent + i16x4 binormal (/32767, w = sign)   9 bone mapping u8x4 ids + u8x4
//!   weights (ids index the subset's bone table)   15 P3S_C4B_T2S (half positions)
//! MeshSubsets: i32 flags, count, reserved[2], then count x {i32 firstIndex, numIndices,
//!              firstVert, numVerts, matId; f32 radius; Vec3 center}; with flags & 2 (bone
//!              indices) count x {u32 n; u16 boneIds[128]} follow.
//! Node 0x823:  char name[64]; i32 object, parent, nChildren, mat; u8 pad[4]; f32 tm[16]
//!              (row vectors, translation in centimetres); f32 pos[3], rot[4], scl[3];
//!              i32 ctrl[3]; i32 propLen; char props[propLen]
//! ```
//!
//! Conventions: everything stays in CryEngine model space (Z up, metres). Node transforms are
//! relative to the parent. For skinned meshes (`.chr`) the vertices are already in model space
//! and the node transform is ignored. Compiled CGFs often carry a `Merged` node holding the
//! merged geometry of all other nodes, whose own meshes are then empty (no indices).
//! `$lodN ...` / `$LODn_...` child nodes are lower LODs of their parent; other `$...` nodes and
//! `proxy` / collision nodes are not rendered.

use crate::chunk::{ChunkFile, cstr, ty};
use crate::skeleton::Skeleton;
use crate::{Error, Result};
use glam::{Affine3A, Vec2, Vec3, Vec4};

/// DataStream types (`streamChunk[i]` of a mesh is the stream of type `i`).
pub mod stream {
    pub const POSITIONS: usize = 0;
    pub const NORMALS: usize = 1;
    pub const TEXCOORDS: usize = 2;
    pub const COLORS: usize = 3;
    pub const COLORS2: usize = 4;
    pub const INDICES: usize = 5;
    pub const TANGENTS: usize = 6;
    pub const SH_COEFFS: usize = 7;
    pub const SHAPE_DEFORMATION: usize = 8;
    pub const BONE_MAPPING: usize = 9;
    pub const FACE_MAP: usize = 10;
    pub const VERT_MATS: usize = 11;
    pub const QTANGENTS: usize = 12;
    pub const SKIN_DATA: usize = 13;
    pub const P3S_C4B_T2S: usize = 15;
}

/// A whole geometry file.
#[derive(Clone, Debug, Default)]
pub struct Model {
    /// In chunk order; parents may come after children (use [`Node::parent`]).
    pub nodes: Vec<Node>,
    pub meshes: Vec<MeshData>,
    /// Bones of `.chr` files (and legacy skinned `.cgf`); `None` for static geometry and for
    /// `_lodN.chr` files, which use their LOD0 skeleton (see [`Skeleton::load`]).
    pub skeleton: Option<Skeleton>,
    /// Material name stored in the file (MtlName chunk of the first rendered node), usually the
    /// `.mtl` file name without extension, relative to the model's folder.
    pub material_name: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    /// A renderable mesh (when [`Node::mesh`] is set) or an empty group.
    Mesh,
    /// Lower level of detail of the parent node (`$lod1 ...`, `$LOD2_...`).
    Lod(u8),
    /// Helper / dummy (`$...` names other than LODs, helper chunks): transform only.
    Helper,
    /// Physics proxy / collision geometry: never rendered.
    Proxy,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub name: String,
    pub chunk_id: i32,
    pub parent: Option<usize>,
    /// Relative to the parent (metres).
    pub local: Affine3A,
    pub kind: NodeKind,
    /// Index into [`Model::meshes`]; `None` when the node has no renderable geometry.
    pub mesh: Option<usize>,
    /// Material name of the node's MtlName chunk.
    pub material_name: Option<String>,
    /// 3ds Max user properties (`box`, `entity`, `mass=...`).
    pub properties: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Subset {
    pub first_index: u32,
    pub index_count: u32,
    pub first_vertex: u32,
    pub vertex_count: u32,
    /// Index into the material's sub-materials ([`crate::material::Material::sub`]).
    pub material_id: u32,
    pub radius: f32,
    pub center: Vec3,
}

/// Per-vertex skinning: up to 4 influences, indices into the model's [`Skeleton`].
#[derive(Clone, Debug, Default)]
pub struct Skin {
    pub joints: Vec<[u16; 4]>,
    /// Normalised to sum 1.
    pub weights: Vec<[f32; 4]>,
}

/// Vertex and index data of one mesh. All vertex arrays have `positions.len()` entries or are
/// empty.
#[derive(Clone, Debug, Default)]
pub struct MeshData {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    /// `xyz` = tangent, `w` = handedness such that `bitangent = cross(normal, tangent) * w`
    /// matches CryEngine's stored binormal (Bevy's convention).
    pub tangents: Vec<Vec4>,
    pub uvs: Vec<Vec2>,
    /// RGBA vertex colours (vegetation uses them for bending / AO, not as colour).
    pub colors: Vec<[u8; 4]>,
    pub indices: Vec<u32>,
    pub subsets: Vec<Subset>,
    pub skin: Option<Skin>,
    pub bbox_min: Vec3,
    pub bbox_max: Vec3,
}

impl Model {
    /// Parses `.cgf` / `.cga` / `.chr` bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Model> {
        Self::from_chunks(&ChunkFile::parse(bytes)?)
    }

    /// Loads a model from the pak.
    pub fn load(path: &str) -> Result<Model> {
        Self::from_bytes(&crate::read(path)?)
    }

    pub fn from_chunks(file: &ChunkFile<'_>) -> Result<Model> {
        if file.file_type != crate::chunk::FILE_GEOMETRY {
            return Err(Error::invalid(format!("not a geometry file (type {:#x})", file.file_type)));
        }
        let skeleton = Skeleton::from_chunks(file)?;
        let mut model = Model { skeleton, ..Default::default() };
        let mut mesh_of_chunk: Vec<(i32, Option<usize>)> = Vec::new();
        let node_chunks: Vec<_> = file.of_type(ty::NODE).collect();
        if node_chunks.is_empty() && file.of_type(ty::MESH).next().is_some() {
            // No hierarchy: one node per mesh.
            for c in file.of_type(ty::MESH) {
                let mesh = read_mesh(file, c, model.skeleton.is_some())?;
                let idx = mesh.map(|m| {
                    model.meshes.push(m);
                    model.meshes.len() - 1
                });
                model.nodes.push(Node {
                    name: format!("mesh{}", c.id),
                    chunk_id: c.id,
                    parent: None,
                    local: Affine3A::IDENTITY,
                    kind: NodeKind::Mesh,
                    mesh: idx,
                    material_name: None,
                    properties: String::new(),
                });
            }
        }
        let mut parents = Vec::with_capacity(node_chunks.len());
        for c in &node_chunks {
            if c.version != 0x823 {
                return Err(Error::Unsupported(format!("Node chunk version {:#x}", c.version)));
            }
            let mut r = c.reader();
            let name = r.fixed_str(64)?;
            let object = r.i32()?;
            let parent = r.i32()?;
            let _children = r.i32()?;
            let mat = r.i32()?;
            r.skip(4)?;
            let mut tm = [0f32; 16];
            for v in &mut tm {
                *v = r.f32()?;
            }
            r.skip(12 + 16 + 12 + 12)?; // pos, rot, scl, controllers
            let prop_len = r.i32()?.max(0) as usize;
            let properties = cstr(r.bytes(prop_len.min(r.remaining()))?);
            let local = Affine3A::from_cols(
                glam::Vec3A::new(tm[0], tm[1], tm[2]),
                glam::Vec3A::new(tm[4], tm[5], tm[6]),
                glam::Vec3A::new(tm[8], tm[9], tm[10]),
                glam::Vec3A::new(tm[12], tm[13], tm[14]) * 0.01,
            );
            let kind = node_kind(&name, file.get(object).map(|o| o.ty));
            let mesh = match file.get_typed(object, ty::MESH) {
                Some(m) if kind != NodeKind::Proxy && kind != NodeKind::Helper => {
                    if let Some(&(_, idx)) = mesh_of_chunk.iter().find(|e| e.0 == object) {
                        idx
                    } else {
                        let idx = read_mesh(file, m, model.skeleton.is_some())?.map(|m| {
                            model.meshes.push(m);
                            model.meshes.len() - 1
                        });
                        mesh_of_chunk.push((object, idx));
                        idx
                    }
                }
                _ => None,
            };
            let material_name = file.get_typed(mat, ty::MTL_NAME).and_then(|m| mtl_name(m.data));
            parents.push(parent);
            model.nodes.push(Node {
                name,
                chunk_id: c.id,
                parent: None,
                local,
                kind,
                mesh,
                material_name,
                properties,
            });
        }
        for (i, p) in parents.into_iter().enumerate() {
            if p > 0 {
                model.nodes[i].parent = model.nodes.iter().position(|n| n.chunk_id == p).filter(|&j| j != i);
            }
        }
        model.material_name = model
            .nodes
            .iter()
            .filter(|n| n.mesh.is_some())
            .chain(model.nodes.iter())
            .find_map(|n| n.material_name.clone())
            .or_else(|| file.of_type(ty::MTL_NAME).find_map(|m| mtl_name(m.data)));
        Ok(model)
    }

    /// Model-space transform of a node (product of the parent chain).
    pub fn world_transform(&self, node: usize) -> Affine3A {
        let mut m = self.nodes[node].local;
        let mut p = self.nodes[node].parent;
        let mut guard = 0;
        while let Some(i) = p {
            m = self.nodes[i].local * m;
            p = self.nodes[i].parent;
            guard += 1;
            if guard > self.nodes.len() {
                break;
            }
        }
        m
    }

    /// Whether a node is part of LOD 0 (neither it nor an ancestor is a LOD / helper / proxy).
    pub fn is_lod0_render_node(&self, node: usize) -> bool {
        let mut i = Some(node);
        let mut guard = 0;
        while let Some(n) = i {
            if self.nodes[n].kind != NodeKind::Mesh {
                return false;
            }
            i = self.nodes[n].parent;
            guard += 1;
            if guard > self.nodes.len() {
                break;
            }
        }
        true
    }

    /// Nodes holding the given embedded LOD level (`0` = the normal render nodes).
    pub fn lod_nodes(&self, lod: u8) -> Vec<usize> {
        (0..self.nodes.len())
            .filter(|&i| self.nodes[i].mesh.is_some())
            .filter(|&i| match lod {
                0 => self.is_lod0_render_node(i),
                n => self.nodes[i].kind == NodeKind::Lod(n),
            })
            .collect()
    }

    /// Highest embedded LOD level (`$lodN` nodes), 0 when there are none.
    pub fn max_embedded_lod(&self) -> u8 {
        self.nodes.iter().filter_map(|n| match n.kind {
            NodeKind::Lod(l) => Some(l),
            _ => None,
        }).max().unwrap_or(0)
    }

    pub fn is_skinned(&self) -> bool {
        self.meshes.iter().any(|m| m.skin.is_some())
    }

    /// Model-space bounds of the LOD 0 render nodes.
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        let mut out: Option<(Vec3, Vec3)> = None;
        for i in self.lod_nodes(0) {
            let m = &self.meshes[self.nodes[i].mesh.unwrap()];
            let tm = if m.skin.is_some() { Affine3A::IDENTITY } else { self.world_transform(i) };
            for p in &m.positions {
                let p = tm.transform_point3(*p);
                out = Some(match out {
                    None => (p, p),
                    Some((a, b)) => (a.min(p), b.max(p)),
                });
            }
        }
        out
    }
}

fn node_kind(name: &str, object_type: Option<u32>) -> NodeKind {
    let lower = name.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("$lod") {
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = digits.parse::<u8>() {
            return NodeKind::Lod(n.max(1));
        }
    }
    if lower.starts_with("proxy")
        || lower.starts_with("$collision")
        || lower.starts_with("$physics_proxy")
        || lower.starts_with("physicsproxy")
        || lower.starts_with("collision")
    {
        return NodeKind::Proxy;
    }
    if lower.starts_with('$') || object_type == Some(ty::HELPER) {
        return NodeKind::Helper;
    }
    NodeKind::Mesh
}

/// MtlName chunk 0x800: `i32 flags, flags2; char name[128]; i32 physType, nSubMtls; ...`.
fn mtl_name(data: &[u8]) -> Option<String> {
    let name = cstr(data.get(8..136)?);
    (!name.is_empty()).then_some(name)
}

/// One DataStream: (element count, element size, bytes).
fn data_stream<'a>(file: &ChunkFile<'a>, id: i32) -> Result<Option<(usize, usize, &'a [u8])>> {
    let Some(c) = file.get_typed(id, ty::DATA_STREAM) else {
        return Ok(None);
    };
    let mut r = c.reader();
    let _flags = r.i32()?;
    let _ty = r.i32()?;
    let count = r.i32()?.max(0) as usize;
    let elem = r.i32()?.max(0) as usize;
    r.skip(8)?;
    let bytes = r.bytes(count.checked_mul(elem).ok_or_else(|| Error::invalid("stream size"))?)?;
    Ok(Some((count, elem, bytes)))
}

fn f32s(b: &[u8], i: usize) -> f32 {
    f32::from_le_bytes(b[i..i + 4].try_into().unwrap())
}

fn i16n(b: &[u8], i: usize) -> f32 {
    i16::from_le_bytes([b[i], b[i + 1]]) as f32 / 32767.0
}

/// Reads a compiled mesh; `None` when it has nothing to render (no indices or positions).
fn read_mesh(file: &ChunkFile<'_>, c: &crate::chunk::Chunk<'_>, skinned: bool) -> Result<Option<MeshData>> {
    if c.version != 0x800 {
        return Err(Error::Unsupported(format!(
            "Mesh chunk version {:#x} (uncompiled legacy mesh)",
            c.version
        )));
    }
    let mut r = c.reader();
    let _flags = r.i32()?;
    let _flags2 = r.i32()?;
    let _n_verts = r.i32()?;
    let _n_indices = r.i32()?;
    let _n_subsets = r.i32()?;
    let subsets_id = r.i32()?;
    let _vert_anim = r.i32()?;
    let mut streams = [0i32; 16];
    for s in &mut streams {
        *s = r.i32()?;
    }
    r.skip(16)?; // physics chunk ids
    let bbox_min = r.vec3()?;
    let bbox_max = r.vec3()?;

    let mut m = MeshData { bbox_min, bbox_max, ..Default::default() };
    // positions
    if let Some((n, e, b)) = data_stream(file, streams[stream::POSITIONS])? {
        if e < 12 {
            return Err(Error::invalid(format!("position stream element size {e}")));
        }
        m.positions = (0..n).map(|i| Vec3::new(f32s(b, i * e), f32s(b, i * e + 4), f32s(b, i * e + 8))).collect();
    } else if let Some((n, e, b)) = data_stream(file, streams[stream::P3S_C4B_T2S])? {
        if e < 16 {
            return Err(Error::invalid(format!("P3S stream element size {e}")));
        }
        let h = |o: usize| crate::texture::half_to_f32(u16::from_le_bytes([b[o], b[o + 1]]));
        for i in 0..n {
            let o = i * e;
            m.positions.push(Vec3::new(h(o), h(o + 2), h(o + 4)));
            m.colors.push([b[o + 8], b[o + 9], b[o + 10], b[o + 11]]);
            m.uvs.push(Vec2::new(h(o + 12), h(o + 14)));
        }
    }
    let nv = m.positions.len();
    // indices
    if let Some((n, e, b)) = data_stream(file, streams[stream::INDICES])? {
        m.indices = match e {
            2 => (0..n).map(|i| u16::from_le_bytes([b[2 * i], b[2 * i + 1]]) as u32).collect(),
            4 => (0..n).map(|i| u32::from_le_bytes(b[4 * i..4 * i + 4].try_into().unwrap())).collect(),
            _ => return Err(Error::invalid(format!("index element size {e}"))),
        };
    }
    if nv == 0 || m.indices.is_empty() {
        return Ok(None);
    }
    if let Some(&bad) = m.indices.iter().find(|&&i| i as usize >= nv) {
        return Err(Error::invalid(format!("index {bad} >= {nv} vertices")));
    }
    m.indices.truncate(m.indices.len() / 3 * 3);
    // normals
    if let Some((n, e, b)) = data_stream(file, streams[stream::NORMALS])? {
        if n == nv && e >= 12 {
            m.normals = (0..n).map(|i| Vec3::new(f32s(b, i * e), f32s(b, i * e + 4), f32s(b, i * e + 8))).collect();
        }
    }
    // uvs
    if let Some((n, e, b)) = data_stream(file, streams[stream::TEXCOORDS])? {
        if n == nv && e >= 8 {
            m.uvs = (0..n).map(|i| Vec2::new(f32s(b, i * e), f32s(b, i * e + 4))).collect();
        }
    }
    // colours
    if let Some((n, e, b)) = data_stream(file, streams[stream::COLORS])? {
        if n == nv && e >= 4 {
            m.colors = (0..n).map(|i| [b[i * e], b[i * e + 1], b[i * e + 2], b[i * e + 3]]).collect();
        }
    }
    // tangents: i16x4 tangent + i16x4 binormal
    let mut binormals: Vec<Vec3> = Vec::new();
    if let Some((n, e, b)) = data_stream(file, streams[stream::TANGENTS])? {
        if n == nv && e >= 16 {
            for i in 0..n {
                let o = i * e;
                let t = Vec3::new(i16n(b, o), i16n(b, o + 2), i16n(b, o + 4));
                let bn = Vec3::new(i16n(b, o + 8), i16n(b, o + 10), i16n(b, o + 12));
                m.tangents.push(t.extend(i16n(b, o + 6).signum()));
                binormals.push(bn);
            }
        }
    }
    if m.normals.len() != nv {
        m.normals = if m.tangents.len() == nv {
            // CryEngine: normal = cross(tangent, binormal) * sign
            m.tangents.iter().zip(&binormals).map(|(t, b)| (t.truncate().cross(*b) * t.w).normalize_or_zero()).collect()
        } else {
            flat_normals(&m.positions, &m.indices)
        };
    }
    if m.tangents.len() == nv {
        for ((t, b), n) in m.tangents.iter_mut().zip(&binormals).zip(&m.normals) {
            let w = if n.cross(t.truncate()).dot(*b) < 0.0 { -1.0 } else { 1.0 };
            *t = t.truncate().normalize_or(Vec3::X).extend(w);
        }
    }

    // subsets (+ per-subset bone tables)
    let mut bone_tables: Vec<Vec<u16>> = Vec::new();
    if let Some(sc) = file.get_typed(subsets_id, ty::MESH_SUBSETS) {
        let mut r = sc.reader();
        let flags = r.i32()?;
        let count = r.i32()?.max(0) as usize;
        r.skip(8)?;
        for _ in 0..count {
            m.subsets.push(Subset {
                first_index: r.i32()?.max(0) as u32,
                index_count: r.i32()?.max(0) as u32,
                first_vertex: r.i32()?.max(0) as u32,
                vertex_count: r.i32()?.max(0) as u32,
                material_id: r.i32()?.max(0) as u32,
                radius: r.f32()?,
                center: r.vec3()?,
            });
        }
        if flags & 2 != 0 {
            for _ in 0..count {
                let n = (r.u32()? as usize).min(128);
                let ids = r.bytes(256)?;
                bone_tables.push((0..n).map(|k| u16::from_le_bytes([ids[2 * k], ids[2 * k + 1]])).collect());
            }
        }
    }
    let ni = m.indices.len() as u32;
    m.subsets.retain(|s| s.index_count > 0 && s.first_index < ni);
    for s in &mut m.subsets {
        s.index_count = s.index_count.min(ni - s.first_index) / 3 * 3;
    }
    if m.subsets.is_empty() {
        m.subsets.push(Subset {
            first_index: 0,
            index_count: ni,
            first_vertex: 0,
            vertex_count: nv as u32,
            material_id: 0,
            radius: 0.0,
            center: Vec3::ZERO,
        });
    }

    // skinning
    if let Some((n, e, b)) = data_stream(file, streams[stream::BONE_MAPPING])? {
        if n == nv && (e == 8 || e == 12) {
            let mut skin = Skin { joints: vec![[0; 4]; nv], weights: vec![[0.0; 4]; nv] };
            let (id_size, w_off) = if e == 8 { (1, 4) } else { (2, 8) };
            let local: Vec<[u16; 4]> = (0..nv)
                .map(|i| {
                    let o = i * e;
                    std::array::from_fn(|k| {
                        if id_size == 1 { b[o + k] as u16 } else { u16::from_le_bytes([b[o + 2 * k], b[o + 2 * k + 1]]) }
                    })
                })
                .collect();
            for i in 0..nv {
                let o = i * e + w_off;
                let w = [b[o], b[o + 1], b[o + 2], b[o + 3]].map(|v| v as f32);
                let sum: f32 = w.iter().sum();
                skin.weights[i] = if sum > 0.0 { w.map(|v| v / sum) } else { [1.0, 0.0, 0.0, 0.0] };
                skin.joints[i] = local[i];
            }
            if !bone_tables.is_empty() {
                // Remap subset-local ids through the subset's bone table, per referenced vertex.
                let mut done = vec![false; nv];
                for (si, s) in m.subsets.iter().enumerate() {
                    let Some(table) = bone_tables.get(si) else { continue };
                    let range = s.first_index as usize..(s.first_index + s.index_count) as usize;
                    for &vi in &m.indices[range] {
                        let vi = vi as usize;
                        if done[vi] {
                            continue;
                        }
                        done[vi] = true;
                        skin.joints[vi] = local[vi].map(|l| table.get(l as usize).copied().unwrap_or(0));
                    }
                }
            }
            // zero-weight influences point at joint 0
            for (j, w) in skin.joints.iter_mut().zip(&skin.weights) {
                for k in 0..4 {
                    if w[k] == 0.0 {
                        j[k] = 0;
                    }
                }
            }
            m.skin = Some(skin);
        }
    }
    let _ = skinned;
    Ok(Some(m))
}

fn flat_normals(positions: &[Vec3], indices: &[u32]) -> Vec<Vec3> {
    let mut n = vec![Vec3::ZERO; positions.len()];
    for t in indices.chunks_exact(3) {
        let (a, b, c) = (positions[t[0] as usize], positions[t[1] as usize], positions[t[2] as usize]);
        let f = (b - a).cross(c - a);
        for &i in t {
            n[i as usize] += f;
        }
    }
    n.iter().map(|v| v.normalize_or(Vec3::Z)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_kinds() {
        assert_eq!(node_kind("$lod1 ncom_city_signpost_c01", None), NodeKind::Lod(1));
        assert_eq!(node_kind("$LOD2_tree_large_b_low", None), NodeKind::Lod(2));
        assert_eq!(node_kind("proxy", None), NodeKind::Proxy);
        assert_eq!(node_kind("$picking01", None), NodeKind::Helper);
        assert_eq!(node_kind("branch1_1", Some(ty::HELPER)), NodeKind::Helper);
        assert_eq!(node_kind("Merged", Some(ty::MESH)), NodeKind::Mesh);
    }

    /// Needs the real client; skipped when it is missing.
    #[test]
    fn real_models() {
        if !crate::pak_available() {
            return;
        }
        // static CGF with a Merged node, empty source node and embedded LODs
        let m = Model::load("objects/env/01_nuia/001_ncom/ncom_city_signpost_c.cgf").unwrap();
        assert_eq!(m.material_name.as_deref(), Some("ncom_signpost"));
        let lod0 = m.lod_nodes(0);
        assert_eq!(lod0.len(), 1);
        assert_eq!(m.nodes[lod0[0]].name, "Merged");
        assert_eq!(m.max_embedded_lod(), 3);
        let mesh = &m.meshes[m.nodes[lod0[0]].mesh.unwrap()];
        assert_eq!(mesh.positions.len(), 981);
        assert_eq!(mesh.indices.len(), 2076);
        assert_eq!(mesh.tangents.len(), 981);
        // skinned character part
        let c = Model::load("objects/characters/elf/female/costume/el_f_login001/el_f_ar_cloth001.chr").unwrap();
        let s = c.skeleton.as_ref().unwrap();
        assert_eq!(s.len(), 63);
        let mesh = &c.meshes[0];
        let skin = mesh.skin.as_ref().unwrap();
        // vertex 0 is weighted to Spine1 / Spine2 / Pelvis (global ids via the subset table)
        let names: Vec<&str> = skin.joints[0][..3].iter().map(|&j| s.bones[j as usize].name.as_str()).collect();
        assert_eq!(names, ["Bip01 Spine1", "Bip01 Spine2", "Bip01 Pelvis"]);
        assert!(skin.joints.iter().flatten().all(|&j| (j as usize) < s.len()));
    }
}
