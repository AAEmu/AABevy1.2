//! Collision geometry of static models (`.cgf`): the CryPhysics blobs of the `MeshPhysicsData` chunks as plain
//! triangles (no Bevy), for queries that need to know where a model is solid (camera and player collision).
//!
//! A compiled model keeps its collision as serialized CryPhysics geometry, not as raw triangles. A mesh chunk
//! (0xCCCC0000 v0x800) lists four physics chunk ids after its stream ids (`+92`): slot 0 is the geometry that
//! collides, slot 1 "no collide" (picking cylinders), slot 2 "obstruct" (a few models). The geometry hangs on
//! the node of a `proxy` (an empty mesh with only the physics, 2530 nodes) or on the render node itself
//! (`Merged`, the model's own name), and a model can have several (`daru_house`: 14). Per chunk (0xCCCC0018
//! v0x800, after the 16 byte header copy):
//!
//! ```text
//! u32 dataSize, u32 flags, u32 tetrLatticeSize, u32 tetrChunkId, u32[2] reserved      24 bytes
//! blob (dataSize bytes):
//!   u32 version (1), 64 B geometry header (centre of mass, inertia, ...), u32 type
//!   type 0 box:       f32 basis[9] (rows = the box axes in model space), u32 oriented, f32 centre[3], f32 half[3]
//!   type 1 trimesh:   u32 nVerts, u32 nTris, u32 valency, u32 flags;
//!                     u8 hasVtxMap [u16 vtxMap[nVerts]]; u8 hasForeignIdx;
//!                     foreign and not (flags & 0x400000): [u16 foreign[nTris]], the triangles are the render mesh's;
//!                     else [u16 foreign[nTris] if foreign] f32x3 verts[nVerts] (unless flags & 1: the render
//!                     mesh's), u16 idx[3 * nTris], u8 hasMats [u8 mat[nTris]]; then a bounding volume tree
//!   type 4 sphere:    f32 centre[3], f32 r
//!   type 5 cylinder:  f32 centre[3], f32 axis[3], f32 r, f32 halfHeight
//!   type 6 capsule:   like the cylinder; the half height is that of the straight part, the caps add `r`
//! ```
//!
//! The geometry is in the frame of the node that carries it ([`crate::mesh::Node`] transforms apply).
//! [`collision_mesh`] returns the default-slot geometry of every node as one triangle list in model space; the
//! primitives become small meshes (12 triangles for a box, a 12 sided prism for a cylinder).

use crate::chunk::{Chunk, ChunkFile, Reader, ty};
use crate::{Error, Result};
use glam::{Affine3A, Mat3, Vec3, Vec3A};

/// Where the triangles of a [`CollisionMesh`] come from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Source {
    /// The model's physics geometry (slot 0 of its physics chunks).
    #[default]
    Physics,
    /// The model has none: the coarsest visible level of detail (0 = the normal mesh).
    VisibleLod(u8),
}

/// Triangles in model space (metres, Z up).
#[derive(Clone, Debug, Default)]
pub struct CollisionMesh {
    pub positions: Vec<Vec3>,
    /// Three indices per triangle.
    pub indices: Vec<u32>,
    pub source: Source,
}

impl CollisionMesh {
    pub fn triangles(&self) -> impl Iterator<Item = [Vec3; 3]> + '_ {
        self.indices.as_chunks::<3>().0.iter().map(|t| [self.positions[t[0] as usize], self.positions[t[1] as usize], self.positions[t[2] as usize]])
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    fn push(&mut self, m: &Affine3A, positions: &[Vec3], indices: &[u32]) {
        let base = self.positions.len() as u32;
        self.positions.extend(positions.iter().map(|p| Vec3::from(m.transform_point3a(Vec3A::from(*p)))));
        self.indices.extend(indices.iter().map(|i| base + i));
    }
}

/// One deserialized physics geometry (the frame of its node).
#[derive(Clone, Debug)]
pub enum PhysGeom {
    /// `basis` rows are the box axes, `half` the half extents along them.
    Box { basis: Mat3, center: Vec3, half: Vec3 },
    /// `positions` / `indices` are `None` when the geometry uses the render mesh's.
    TriMesh { vertex_count: usize, triangle_count: usize, positions: Option<Vec<Vec3>>, indices: Option<Vec<u32>> },
    Sphere { center: Vec3, radius: f32 },
    Cylinder { center: Vec3, axis: Vec3, radius: f32, half_height: f32 },
    Capsule { center: Vec3, axis: Vec3, radius: f32, half_height: f32 },
}

/// Parses the blob of a `MeshPhysicsData` chunk (what follows its 24 byte header).
pub fn parse_blob(blob: &[u8]) -> Result<PhysGeom> {
    let mut r = Reader::new(blob);
    let version = r.u32()?;
    if version != 1 {
        return Err(Error::Unsupported(format!("physics geometry version {version}")));
    }
    r.skip(64)?;
    match r.u32()? {
        0 => {
            let mut m = [0f32; 9];
            for v in &mut m {
                *v = r.f32()?;
            }
            let oriented = r.u32()? != 0;
            let (center, half) = (r.vec3()?, r.vec3()?);
            // glam is column major: the rows of the file's matrix are the axes
            let basis = if oriented { Mat3::from_cols_array(&m).transpose() } else { Mat3::IDENTITY };
            Ok(PhysGeom::Box { basis, center, half })
        }
        1 => {
            let (nv, nt, _valency, flags) = (r.u32()? as usize, r.u32()? as usize, r.u32()?, r.u32()?);
            if nv > 1 << 20 || nt > 1 << 20 {
                return Err(Error::invalid(format!("trimesh of {nv} vertices / {nt} triangles")));
            }
            if r.u8()? != 0 {
                r.skip(2 * nv)?;
            }
            let foreign = r.u8()? != 0;
            if foreign {
                r.skip(2 * nt)?;
            }
            if foreign && flags & 0x40_0000 == 0 {
                return Ok(PhysGeom::TriMesh { vertex_count: nv, triangle_count: nt, positions: None, indices: None });
            }
            let positions = if flags & 1 == 0 {
                let mut p = Vec::with_capacity(nv);
                for _ in 0..nv {
                    p.push(r.vec3()?);
                }
                Some(p)
            } else {
                None
            };
            let mut indices = Vec::with_capacity(nt * 3);
            for _ in 0..nt * 3 {
                indices.push(r.u16()? as u32);
            }
            if let Some(bad) = indices.iter().find(|&&i| i as usize >= nv) {
                return Err(Error::invalid(format!("physics index {bad} >= {nv} vertices")));
            }
            Ok(PhysGeom::TriMesh { vertex_count: nv, triangle_count: nt, positions, indices: Some(indices) })
        }
        4 => Ok(PhysGeom::Sphere { center: r.vec3()?, radius: r.f32()? }),
        t @ (5 | 6) => {
            let (center, axis, radius, half_height) = (r.vec3()?, r.vec3()?, r.f32()?, r.f32()?);
            Ok(if t == 5 { PhysGeom::Cylinder { center, axis, radius, half_height } } else { PhysGeom::Capsule { center, axis, radius, half_height } })
        }
        t => Err(Error::Unsupported(format!("physics geometry type {t}"))),
    }
}

/// A node of the file as far as collision needs it.
struct NodeRec {
    name: String,
    chunk_id: i32,
    object: i32,
    parent: i32,
    local: Affine3A,
}

fn read_nodes(file: &ChunkFile<'_>) -> Result<Vec<NodeRec>> {
    let mut out = Vec::new();
    for c in file.of_type(ty::NODE) {
        if c.version != 0x823 {
            continue;
        }
        let mut r = c.reader();
        let name = r.fixed_str(64)?;
        let (object, parent, _children, _mat) = (r.i32()?, r.i32()?, r.i32()?, r.i32()?);
        r.skip(4)?;
        let mut tm = [0f32; 16];
        for v in &mut tm {
            *v = r.f32()?;
        }
        // rows of the 4x4, translation in centimetres (as `Model::from_chunks`)
        let local = Affine3A::from_cols(
            Vec3A::new(tm[0], tm[1], tm[2]),
            Vec3A::new(tm[4], tm[5], tm[6]),
            Vec3A::new(tm[8], tm[9], tm[10]),
            Vec3A::new(tm[12], tm[13], tm[14]) * 0.01,
        );
        out.push(NodeRec { name, chunk_id: c.id, object, parent, local });
    }
    Ok(out)
}

fn node_world(nodes: &[NodeRec], i: usize, lod_local_identity: bool) -> Affine3A {
    let mut m = Affine3A::IDENTITY;
    let mut at = Some(i);
    for _ in 0..nodes.len() + 1 {
        let Some(k) = at else { break };
        let n = &nodes[k];
        let local = if lod_local_identity && is_lod(&n.name) { Affine3A::IDENTITY } else { n.local };
        m = local * m;
        at = (n.parent > 0).then(|| nodes.iter().position(|p| p.chunk_id == n.parent)).flatten().filter(|&p| p != k);
    }
    m
}

fn is_lod(name: &str) -> bool {
    name.get(..4).is_some_and(|s| s.eq_ignore_ascii_case("$lod"))
}

/// Nodes that never carry collision: embedded levels of detail and helpers (`$picking`, `$occlusion`, ...).
/// `$collision` and `$physics_proxy` are proxies.
fn skipped(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with('$') && !lower.starts_with("$collision") && !lower.starts_with("$physics_proxy")
}

/// The four physics chunk ids of a mesh chunk (`0` / negative: none).
fn mesh_physics_ids(c: &Chunk<'_>) -> Option<[i32; 4]> {
    if c.version != 0x800 {
        return None;
    }
    let mut r = c.reader();
    r.skip(7 * 4 + 16 * 4).ok()?;
    Some([r.i32().ok()?, r.i32().ok()?, r.i32().ok()?, r.i32().ok()?])
}

/// The blob of a `MeshPhysicsData` chunk.
fn physics_blob<'a>(file: &ChunkFile<'a>, id: i32) -> Option<&'a [u8]> {
    let c = file.get_typed(id, ty::MESH_PHYSICS_DATA)?;
    let mut r = c.reader();
    let size = r.u32().ok()? as usize;
    r.skip(20).ok()?;
    r.bytes(size.min(r.remaining())).ok()
}

/// The collision triangles of a `.cgf` (bytes of the file), in model space.
///
/// Every node's default-slot physics geometry is transformed by its node chain and appended. A model without
/// any (`ndeco_common_straw01`, ivy, flags: 7 % of the placed brushes) has no collision in the real client;
/// with `visible_fallback` its coarsest visible level of detail is returned instead. `Ok(None)`: nothing to
/// collide with.
pub fn collision_mesh(bytes: &[u8], visible_fallback: bool) -> Result<Option<CollisionMesh>> {
    let file = ChunkFile::parse(bytes)?;
    if file.file_type != crate::chunk::FILE_GEOMETRY {
        return Ok(None);
    }
    let nodes = read_nodes(&file)?;
    let mut out = CollisionMesh::default();
    // the render mesh of nodes whose physics shares it, parsed on demand
    let mut model: Option<crate::mesh::Model> = None;
    for (i, node) in nodes.iter().enumerate() {
        if skipped(&node.name) {
            continue;
        }
        let Some(mesh_chunk) = file.get_typed(node.object, ty::MESH) else { continue };
        let Some(ids) = mesh_physics_ids(mesh_chunk) else { continue };
        if ids[0] <= 0 {
            continue;
        }
        let Some(blob) = physics_blob(&file, ids[0]) else { continue };
        let geom = match parse_blob(blob) {
            Ok(g) => g,
            Err(_) => continue,
        };
        let m = node_world(&nodes, i, false);
        match geom {
            PhysGeom::Box { basis, center, half } => {
                let (p, t) = box_mesh(basis, center, half);
                out.push(&m, &p, &t);
            }
            PhysGeom::Sphere { center, radius } => {
                let (p, t) = sphere_mesh(center, radius);
                out.push(&m, &p, &t);
            }
            PhysGeom::Cylinder { center, axis, radius, half_height } => {
                let (p, t) = cylinder_mesh(center, axis, radius, half_height);
                out.push(&m, &p, &t);
            }
            PhysGeom::Capsule { center, axis, radius, half_height } => {
                // a prism one cap longer on each side
                let (p, t) = cylinder_mesh(center, axis, radius, half_height + radius);
                out.push(&m, &p, &t);
            }
            PhysGeom::TriMesh { positions, indices, triangle_count, .. } => {
                let render = if positions.is_none() || indices.is_none() {
                    if model.is_none() {
                        model = crate::mesh::Model::from_chunks(&file).ok();
                    }
                    model.as_ref().and_then(|md| md.nodes.iter().find(|n| n.chunk_id == node.chunk_id)).and_then(|n| n.mesh).and_then(|mi| model.as_ref().map(|md| &md.meshes[mi]))
                } else {
                    None
                };
                let p = positions.as_deref().or_else(|| render.map(|r| r.positions.as_slice()));
                let t = indices.as_deref().or_else(|| render.map(|r| r.indices.as_slice()));
                if let (Some(p), Some(t)) = (p, t) {
                    let t = &t[..t.len().min(triangle_count * 3) / 3 * 3];
                    if t.iter().all(|&i| (i as usize) < p.len()) {
                        out.push(&m, p, t);
                    }
                }
            }
        }
    }
    if !out.is_empty() {
        return Ok(Some(out));
    }
    if !visible_fallback {
        return Ok(None);
    }
    let md = match model {
        Some(m) => m,
        None => crate::mesh::Model::from_chunks(&file)?,
    };
    if md.is_skinned() {
        return Ok(None);
    }
    let mut lod = md.max_embedded_lod();
    let mut ids = md.lod_nodes(lod);
    if ids.is_empty() {
        lod = 0;
        ids = md.lod_nodes(0);
    }
    for ni in ids {
        let Some(mi) = md.nodes[ni].mesh else { continue };
        let mesh = &md.meshes[mi];
        // the transform as the renderer builds it: LOD nodes sit in their parent's frame
        let m = nodes.iter().position(|n| n.chunk_id == md.nodes[ni].chunk_id).map_or_else(|| md.world_transform(ni), |k| node_world(&nodes, k, true));
        out.push(&m, &mesh.positions, &mesh.indices);
    }
    out.source = Source::VisibleLod(lod);
    Ok((!out.is_empty()).then_some(out))
}

/// Loads a model from the pak and returns its [`collision_mesh`].
pub fn load(path: &str, visible_fallback: bool) -> Result<Option<CollisionMesh>> {
    collision_mesh(&crate::read(path)?, visible_fallback)
}

/// Eight corners, twelve triangles, outward facing (counter clockwise seen from outside).
fn box_mesh(basis: Mat3, center: Vec3, half: Vec3) -> (Vec<Vec3>, Vec<u32>) {
    let axes = [basis.row(0) * half.x, basis.row(1) * half.y, basis.row(2) * half.z];
    // corner i has the sign of bit k on axis k
    let positions: Vec<Vec3> = (0..8)
        .map(|i| {
            let s = |k: usize| if i >> k & 1 == 1 { 1.0 } else { -1.0 };
            center + axes[0] * s(0) + axes[1] * s(1) + axes[2] * s(2)
        })
        .collect();
    // faces as quads (a, b, c, d) counter clockwise for a right handed basis
    const QUADS: [[u32; 4]; 6] = [[1, 3, 7, 5], [0, 4, 6, 2], [2, 6, 7, 3], [0, 1, 5, 4], [4, 5, 7, 6], [0, 2, 3, 1]];
    let flip = basis.determinant() < 0.0;
    let mut indices = Vec::with_capacity(36);
    for q in QUADS {
        if flip {
            indices.extend_from_slice(&[q[0], q[2], q[1], q[0], q[3], q[2]]);
        } else {
            indices.extend_from_slice(&[q[0], q[1], q[2], q[0], q[2], q[3]]);
        }
    }
    (positions, indices)
}

/// Orthonormal frame with `axis` as its third vector.
fn frame(axis: Vec3) -> (Vec3, Vec3, Vec3) {
    let w = axis.try_normalize().unwrap_or(Vec3::Z);
    let u = w.any_orthonormal_vector();
    (u, w.cross(u), w)
}

/// A prism of `SEGMENTS` sides around `axis`, closed by two caps.
fn cylinder_mesh(center: Vec3, axis: Vec3, radius: f32, half_height: f32) -> (Vec<Vec3>, Vec<u32>) {
    const SEGMENTS: u32 = 12;
    let (u, v, w) = frame(axis);
    let mut positions = Vec::new();
    for ring in [-1.0f32, 1.0] {
        for k in 0..SEGMENTS {
            let a = k as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
            positions.push(center + w * (ring * half_height) + (u * a.cos() + v * a.sin()) * radius);
        }
    }
    let (bottom, top) = ((positions.len()) as u32, positions.len() as u32 + 1);
    positions.push(center - w * half_height);
    positions.push(center + w * half_height);
    let mut indices = Vec::new();
    for k in 0..SEGMENTS {
        let (a, b) = (k, (k + 1) % SEGMENTS);
        let (c, d) = (a + SEGMENTS, b + SEGMENTS);
        indices.extend_from_slice(&[a, b, d, a, d, c]); // side
        indices.extend_from_slice(&[bottom, b, a]); // bottom cap
        indices.extend_from_slice(&[top, c, d]); // top cap
    }
    (positions, indices)
}

/// A coarse sphere: 8 meridians, 3 rings and the poles.
fn sphere_mesh(center: Vec3, radius: f32) -> (Vec<Vec3>, Vec<u32>) {
    const MERIDIANS: u32 = 8;
    const RINGS: u32 = 3;
    let mut positions = vec![center + Vec3::Z * radius, center - Vec3::Z * radius];
    for ring in 1..=RINGS {
        let phi = ring as f32 / (RINGS + 1) as f32 * std::f32::consts::PI;
        for k in 0..MERIDIANS {
            let a = k as f32 / MERIDIANS as f32 * std::f32::consts::TAU;
            positions.push(center + Vec3::new(phi.sin() * a.cos(), phi.sin() * a.sin(), phi.cos()) * radius);
        }
    }
    let at = |ring: u32, k: u32| 2 + (ring - 1) * MERIDIANS + k % MERIDIANS;
    let mut indices = Vec::new();
    for k in 0..MERIDIANS {
        indices.extend_from_slice(&[0, at(1, k), at(1, k + 1)]);
        indices.extend_from_slice(&[1, at(RINGS, k + 1), at(RINGS, k)]);
        for ring in 1..RINGS {
            let (a, b, c, d) = (at(ring, k), at(ring, k + 1), at(ring + 1, k + 1), at(ring + 1, k));
            indices.extend_from_slice(&[a, d, c, a, c, b]);
        }
    }
    (positions, indices)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signed_volume(p: &[Vec3], t: &[u32]) -> f32 {
        t.as_chunks::<3>().0.iter().map(|t| p[t[0] as usize].dot(p[t[1] as usize].cross(p[t[2] as usize])) / 6.0).sum()
    }

    #[test]
    fn primitives_are_closed_and_outward() {
        let (p, t) = box_mesh(Mat3::IDENTITY, Vec3::new(1.0, 2.0, 3.0), Vec3::new(1.0, 2.0, 0.5));
        assert_eq!(t.len(), 36);
        let centered: Vec<Vec3> = p.iter().map(|v| *v - Vec3::new(1.0, 2.0, 3.0)).collect();
        assert!((signed_volume(&centered, &t) - 8.0).abs() < 1e-4, "{}", signed_volume(&centered, &t));
        // a mirrored basis keeps the faces outward
        let mirrored = Mat3::from_diagonal(Vec3::new(-1.0, 1.0, 1.0));
        let (p, t) = box_mesh(mirrored, Vec3::ZERO, Vec3::ONE);
        assert!((signed_volume(&p, &t) - 8.0).abs() < 1e-4);
        // a turned basis: rows are the axes
        let turned = Mat3::from_cols(Vec3::Y, -Vec3::X, Vec3::Z).transpose();
        let (p, _) = box_mesh(turned, Vec3::ZERO, Vec3::new(3.0, 1.0, 1.0));
        let (lo, hi) = p.iter().fold((Vec3::MAX, Vec3::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
        assert!((hi - lo - Vec3::new(2.0, 6.0, 2.0)).abs().max_element() < 1e-4 || (hi - lo - Vec3::new(6.0, 2.0, 2.0)).abs().max_element() < 1e-4);
        let (p, t) = cylinder_mesh(Vec3::new(0.0, 0.0, 1.0), -Vec3::Z, 0.5, 1.0);
        let centered: Vec<Vec3> = p.iter().map(|v| *v - Vec3::new(0.0, 0.0, 1.0)).collect();
        assert!(signed_volume(&centered, &t) > 1.4, "{}", signed_volume(&centered, &t));
        let (p, t) = sphere_mesh(Vec3::ZERO, 1.0);
        assert!(signed_volume(&p, &t) > 1.0, "{}", signed_volume(&p, &t));
    }

    /// A box blob: version, 64 B header, type 0, basis, oriented, centre, half.
    #[test]
    fn box_blob() {
        let mut blob = Vec::new();
        blob.extend_from_slice(&1u32.to_le_bytes());
        blob.extend_from_slice(&[0u8; 64]);
        blob.extend_from_slice(&0u32.to_le_bytes());
        for v in [0.0f32, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
            blob.extend_from_slice(&v.to_le_bytes());
        }
        blob.extend_from_slice(&1u32.to_le_bytes());
        for v in [0.0f32, 0.0, 3.89, 4.93, 4.18, 2.6] {
            blob.extend_from_slice(&v.to_le_bytes());
        }
        let PhysGeom::Box { basis, center, half } = parse_blob(&blob).unwrap() else { panic!() };
        assert_eq!(center, Vec3::new(0.0, 0.0, 3.89));
        // axis 0 (4.93 long) is model Z, axis 1 (4.18) model X, axis 2 (2.6) model Y
        let (p, _) = box_mesh(basis, center, half);
        let (lo, hi) = p.iter().fold((Vec3::MAX, Vec3::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
        assert!((hi - lo - Vec3::new(2.0 * 4.18, 2.0 * 2.6, 2.0 * 4.93)).abs().max_element() < 1e-4, "{lo} {hi}");
        assert!(parse_blob(&blob[..40]).is_err());
    }

    /// Needs the real client; skipped when it is missing.
    #[test]
    fn real_models() {
        if !crate::pak_available() {
            return;
        }
        let rel = |m: &str| format!("objects/env/02_harihara/05_si/{m}.cgf");
        // proxy box: a wooden bridge deck, 8.8 x 4.1 x 0.64
        let bridge = load(&rel("si_woodbridge_01"), false).unwrap().unwrap();
        assert_eq!(bridge.source, Source::Physics);
        assert_eq!(bridge.indices.len(), 36);
        let (lo, hi) = bridge.positions.iter().fold((Vec3::MAX, Vec3::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
        assert!((hi.x - lo.x - 8.82).abs() < 0.05 && (hi.y - lo.y - 4.05).abs() < 0.05, "{lo} {hi}");
        // trimesh on the render node: a shop
        let shop = load(&rel("si_shop_g"), false).unwrap().unwrap();
        assert!(shop.indices.len() / 3 == 206, "{}", shop.indices.len() / 3);
        // a trimesh proxy: stairs
        let stairs = load(&rel("si_woodbridge_stairs01"), false).unwrap().unwrap();
        assert_eq!(stairs.indices.len() / 3, 66);
        // no physics: nothing, or the visible mesh with the fallback
        let straw = "objects/env/01_nuia/001_ndeco/ndeco_common_straw01.cgf";
        assert!(load(straw, false).unwrap().is_none());
        let visible = load(straw, true).unwrap().unwrap();
        assert!(matches!(visible.source, Source::VisibleLod(_)) && !visible.is_empty());
    }
}
