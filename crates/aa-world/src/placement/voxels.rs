//! Native type-6 compiled voxel data: first-LOD base geometry and serialized collision.

use std::sync::Arc;

use aa_cry::chunk::{ChunkFile, ty};
use aa_cry::physics::{CollisionMesh, PhysGeom};
use bevy::math::Mat4;

use crate::reader::{Reader, cstr, f32_at, u32_at};

/// Resource guards, not native authoring limits. Original surveyed cells have at most 21 slots.
const MAX_SLOTS_PER_FILE: usize = 128;
const MAX_RETAINED_BYTES_PER_FILE: usize = 16 * 1024 * 1024;
pub(crate) const MAX_DECODED_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct PlacedVoxel {
    /// Row-major cell-local Cry matrix, serialized at +198727. Apply cell origin once.
    pub matrix: [f32; 12],
    pub bbox: [f32; 6],
    /// SVoxelChunkVer5 flags (+67); unrelated to the common render-node flags.
    pub flags: u32,
    pub max_view_dist: f32,
    /// Local physics/material palette, resolved by name against cell leveldata.xml.
    pub surfaces: [String; 32],
    /// The first native compiled CGF memory block. Other LODs are consumed without copies.
    /// Empty when the source has no precompiled mesh; runtime volume meshing is unsupported.
    pub compiled: Arc<[u8]>,
}

impl PlacedVoxel {
    /// First-LOD native serialized triangle proxy, with local 0..31 palette IDs.
    /// Unsupported/malformed variants return None; this does not reconstruct voxel surfaces.
    pub fn compiled_collision(&self) -> Option<CollisionMesh> {
        collision(self)
    }
    /// Native Terrain base pass of a compiled static voxel. Colours are the original
    /// linear baked RGB reconstructed from COLORS.r/b and COLORS2.b. Projected terrain
    /// detail layers are not included. Colour alpha is the native per-vertex baked-colour flag;
    /// zero requires the cell's terrain-cover texture, and it interpolates across mixed triangles.
    pub fn compiled_base_mesh(&self) -> Option<aa_cry::mesh::MeshData> {
        base_mesh(self).map(|(mesh, _)| mesh)
    }
    pub(crate) fn compiled_surface_mesh(&self) -> Option<(aa_cry::mesh::MeshData, Vec<u8>)> { base_mesh(self) }
    pub fn mat4(&self) -> Mat4 {
        let m = self.matrix;
        Mat4::from_cols_array(&[
            m[0], m[4], m[8], 0.0, m[1], m[5], m[9], 0.0, m[2], m[6], m[10], 0.0, m[3], m[7],
            m[11], 1.0,
        ])
    }
}

pub(super) fn read_record(r: &mut Reader, out: &mut Vec<PlacedVoxel>) -> Option<()> {
    // Native 301F5E30, SVoxelObjectChunkVer5: 39-byte render-node prefix,
    // 198688-byte volume/palette/colour data, then a 48-byte object matrix.
    let header = r.bytes(198_775).ok()?;
    if u32_at(header, 39) != 5 || out.len() >= MAX_SLOTS_PER_FILE {
        return None;
    }
    let flags = u32_at(header, 67);
    let mut first = &[][..];
    if flags & 0x20 != 0 {
        let count = r.u32().ok()? as usize;
        if count > r.remaining() / 4 {
            return None;
        }
        for index in 0..count {
            let size = r.u32().ok()? as usize;
            let bytes = r.bytes(size).ok()?;
            if index == 0 {
                first = bytes;
            }
        }
    }
    let retained: usize = out.iter().map(|v| v.compiled.len()).sum();
    if first.len() > MAX_RETAINED_BYTES_PER_FILE.checked_sub(retained)? {
        return None;
    }
    out.push(PlacedVoxel {
        matrix: std::array::from_fn(|i| f32_at(header, 198727 + i * 4)),
        bbox: std::array::from_fn(|i| f32_at(header, i * 4)),
        flags,
        max_view_dist: f32_at(header, 28),
        surfaces: std::array::from_fn(|i| cstr(&header[65607 + i * 64..65607 + (i + 1) * 64])),
        compiled: Arc::from(first),
    });
    Some(())
}

pub(crate) fn decoded_size(bytes: &[u8]) -> Option<usize> {
    let size = if bytes.starts_with(b"CryTek") {
        bytes.len()
    } else {
        u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?) as usize
    };
    (size > 0 && size <= MAX_DECODED_BYTES).then_some(size)
}

fn decode(bytes: &[u8]) -> Option<Vec<u8>> {
    let size = decoded_size(bytes)?;
    if bytes.starts_with(b"CryTek") {
        return Some(bytes.to_vec());
    }
    // Native 301A0D30 reads a leading decoded length before the compressed block.
    // Original surveyed compressed memory blocks use the zlib wrapper.
    let payload = bytes.get(4..)?;
    let mut raw = vec![0; size + 1];
    let mut decoder = flate2::Decompress::new(true);
    let status = decoder
        .decompress(payload, &mut raw, flate2::FlushDecompress::Finish)
        .ok()?;
    if status != flate2::Status::StreamEnd
        || decoder.total_out() != size as u64
        || decoder.total_in() != payload.len() as u64
    {
        return None;
    }
    raw.truncate(size);
    Some(raw)
}

/// Shared resource preflight for the retained first compiled LOD.
fn checked_file(raw: &[u8]) -> Option<ChunkFile<'_>> {
    let table = u32_at(raw, 16) as usize;
    if u32_at(raw, table) > 64 {
        return None;
    }
    let file = ChunkFile::parse(raw).ok()?;
    let mut meshes = file.of_type(ty::MESH);
    let mesh = meshes.next()?;
    if meshes.next().is_some()
        || mesh.version != 0x800
        || u32_at(mesh.data, 8) > 65536
        || u32_at(mesh.data, 12) > 1_000_000
        || u32_at(mesh.data, 16) > 32
    {
        return None;
    }
    if file.of_type(ty::DATA_STREAM).any(|c| {
        let limit = if u32_at(c.data, 4) == 5 {
            1_000_000
        } else {
            65536
        };
        u32_at(c.data, 8) > limit
    }) || file
        .of_type(ty::MESH_SUBSETS)
        .any(|c| u32_at(c.data, 4) > 32)
    {
        return None;
    }
    drop(meshes);
    Some(file)
}

fn static_model(file: &ChunkFile<'_>) -> Option<aa_cry::Model> {
    let model = aa_cry::Model::from_chunks(file).ok()?;
    if model.nodes.len() != 1
        || model.nodes[0].name != "Merged"
        || !Mat4::from(model.nodes[0].local).abs_diff_eq(Mat4::IDENTITY, 1e-6)
        || model.is_skinned()
    {
        return None;
    }
    Some(model)
}

// TerrainVS uses g >= 0.5, hence byte 128 is included. Native CRenderMesh's
// 382051A0 puts COLORS2 byte 2 into Normal.w; TerrainVS assembles (Color.r,b,Normal.w).
fn base_colour(colour: [u8; 4], extra: [u8; 4]) -> [u8; 4] {
    [colour[0], colour[2], extra[2], if colour[1] >= 128 { 255 } else { 0 }]
}

fn base_mesh(voxel: &PlacedVoxel) -> Option<(aa_cry::mesh::MeshData, Vec<u8>)> {
    let transform = voxel.mat4();
    if !transform.is_finite() || transform.determinant().abs() < 1e-9 {
        return None;
    }
    let raw = decode(&voxel.compiled)?;
    let file = checked_file(&raw)?;
    let model = static_model(&file)?;
    if model.meshes.len() != 1 || model.nodes[0].mesh != Some(0) {
        return None;
    }
    let header = file.of_type(ty::MESH).next()?;
    let extra = file.get_typed(u32_at(header.data, 28 + 4 * 4) as i32, ty::DATA_STREAM)?;
    let mut mesh = model.meshes.into_iter().next()?;
    let count = mesh.positions.len();
    if count == 0
        || mesh.indices.is_empty()
        || mesh.normals.len() != count
        || mesh.colors.len() != count
        || !mesh.uvs.is_empty()
        || u32_at(extra.data, 4) != 4
        || u32_at(extra.data, 8) as usize != count
        || u32_at(extra.data, 12) != 4
        || mesh.positions.iter().any(|p| !p.is_finite())
        || mesh
            .normals
            .iter()
            .any(|n| !n.is_finite() || n.length_squared() < 1e-10)
    {
        return None;
    }
    let colours = extra.data.get(24..24 + count * 4)?.as_chunks::<4>().0;
    let surfaces = mesh.colors.iter().map(|c| c[1] & 0x7f).collect();
    for (colour, &extra) in mesh.colors.iter_mut().zip(colours) {
        *colour = base_colour(*colour, extra);
    }
    Some((mesh, surfaces))
}

/// Native 301A0790 physicalizes the first compiled LOD through 3019BFA0. Supported
/// files contain one merged static mesh and its serialized triangle proxy/material IDs.
/// Runtime-generated geometry and foreign-index/no-material variants stay unsupported.
pub(crate) fn collision(voxel: &PlacedVoxel) -> Option<CollisionMesh> {
    let matrix = voxel.mat4();
    if !matrix.is_finite() || matrix.determinant().abs() < 1e-9 {
        return None;
    }
    let raw = decode(&voxel.compiled)?;
    let file = checked_file(&raw)?;
    let _model = static_model(&file)?;
    let mut chunks = file.of_type(ty::MESH_PHYSICS_DATA);
    let chunk = chunks.next()?;
    if chunks.next().is_some() {
        return None;
    }
    let mut r = chunk.reader();
    let size = r.u32().ok()? as usize;
    r.skip(20).ok()?;
    let blob = r.bytes(size).ok()?;
    if u32_at(blob, 68) != 1 || u32_at(blob, 72) > 65536 || u32_at(blob, 76) > 1_000_000 {
        return None;
    }
    let proxy = aa_cry::physics::parse_blob(blob).ok()?;
    let PhysGeom::TriMesh {
        materials: Some(materials),
        triangle_count,
        ..
    } = proxy
    else {
        return None;
    };
    if materials.len() != triangle_count || materials.iter().any(|&id| id >= 32) {
        return None;
    }
    let mesh = aa_cry::physics::collision_mesh(&raw, false).ok()??;
    if mesh.indices.len() / 3 != triangle_count
        || mesh.indices.len() % 3 != 0
        || mesh
            .indices
            .iter()
            .any(|&i| i as usize >= mesh.positions.len())
        || mesh.materials.len() != triangle_count
        || mesh
            .materials
            .iter()
            .zip(materials)
            .any(|(&a, b)| a != u16::from(b))
    {
        return None;
    }
    Some(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::math::Vec3;
    use std::io::Write;

    #[test]
    fn native_base_colour_flag_threshold_preserves_linear_packed_channels() {
        assert_eq!(base_colour([21, 127, 14, 255], [0, 0, 11, 0]), [21, 14, 11, 0]);
        for g in [128, 140, 255] {
            assert_eq!(
                base_colour([21, g, 14, 255], [0, 0, 11, 0]),
                [21, 14, 11, 255]
            );
        }
    }

    #[test]
    fn original_voxel_baked_base_pass_and_cover_path_are_distinct() {
        if !crate::pak_available() {
            return;
        }
        let objects = crate::placement::load_cell_objects("main_world", 20, 8);
        let voxel = &objects.voxels[0];
        let mesh = voxel.compiled_base_mesh().unwrap();
        assert_eq!(mesh.colors[0], [21, 14, 11, 255]);
        assert!(mesh.uvs.is_empty());
        assert!(
            mesh.colors
                .iter()
                .copied()
                .collect::<std::collections::HashSet<_>>()
                .len()
                > 100
        );
        let mut raw = decode(&voxel.compiled).unwrap();
        let file = checked_file(&raw).unwrap();
        let stream = file
            .of_type(ty::DATA_STREAM)
            .find(|c| u32_at(c.data, 4) == 3)
            .unwrap();
        let g = stream.offset as usize + 16 + 24 + 1;
        let mut control = voxel.clone();
        raw[g] = 127; // Native base pass would require the terrain cover projection.
        control.compiled = raw.into();
        assert_eq!(control.compiled_base_mesh().unwrap().colors[0], [21, 14, 11, 0]);
        // Two original exterior slots nearby mix baked-colour and cover-projection vertices.
        // Preserve those vertices' cover flag rather than painting them with the layer IDs.
        let exterior = crate::placement::load_cell_objects("main_world", 21, 8);
        assert_eq!(exterior.voxels.len(), 2);
        assert!(exterior.voxels.iter().all(|v| v.compiled_base_mesh().unwrap().colors.iter().any(|c| c[3] == 0)));
    }

    #[test]
    fn compressed_memory_blocks_have_bounded_exact_lengths_and_checked_checksums() {
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(b"native memory block fixture").unwrap();
        let body = encoder.finish().unwrap();
        let mut bytes = (27u32).to_le_bytes().to_vec();
        bytes.extend(body);
        assert_eq!(decode(&bytes).unwrap(), b"native memory block fixture");
        for size in [0, 26, 28, MAX_DECODED_BYTES as u32 + 1, u32::MAX] {
            let mut invalid = bytes.clone();
            invalid[..4].copy_from_slice(&size.to_le_bytes());
            assert!(decode(&invalid).is_none());
        }
        let mut invalid = bytes.clone();
        let end = invalid.len() - 1;
        invalid[end] ^= 0xff;
        assert!(decode(&invalid).is_none());
        for cut in [0, 3, 5, bytes.len() - 4] {
            assert!(decode(&bytes[..cut]).is_none());
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(decode(&trailing).is_none());
    }

    #[test]
    fn original_compiled_voxel_proxy_rejects_bad_cgf_counts_and_transforms() {
        if !crate::pak_available() {
            return;
        }
        let objects = crate::placement::load_cell_objects("main_world", 20, 8);
        let mut voxel = objects.voxels[0].clone();
        let raw = decode(&voxel.compiled).unwrap();
        let file = ChunkFile::parse(&raw).unwrap();
        let mesh = file.of_type(ty::MESH).next().unwrap();
        let stream = file.of_type(ty::DATA_STREAM).next().unwrap();
        let physics = file.of_type(ty::MESH_PHYSICS_DATA).next().unwrap();
        // These native chunks carry their 16-byte header copies.
        for offset in [
            u32_at(&raw, 16) as usize,
            mesh.offset as usize + 16 + 8,
            stream.offset as usize + 16 + 8,
            physics.offset as usize + 16 + 24 + 76,
        ] {
            let mut corrupt = raw.clone();
            corrupt[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
            voxel.compiled = corrupt.into();
            assert!(voxel.compiled_collision().is_none());
        }
        voxel.compiled = raw[..20].into();
        assert!(voxel.compiled_collision().is_none());
        voxel.compiled = raw.into();
        voxel.matrix[0] = f32::NAN;
        assert!(voxel.compiled_collision().is_none());
        voxel.matrix[0] = 0.0;
        assert!(voxel.compiled_collision().is_none());
    }

    #[test]
    fn original_voxel_first_lod_proxy_matches_native_material_majority_and_cell_units() {
        if !crate::pak_available() {
            return;
        }
        let bytes = aa_pak::Pak::global()
            .read("game/worlds/main_world/cells/020_008/client/visareas.dat")
            .unwrap();
        let objects = crate::placement::parse_visareas_dat(&bytes).unwrap();
        assert_eq!(objects.voxels.len(), 7);
        let voxel = &objects.voxels[0];
        assert_eq!(voxel.flags, 0xb4);
        assert_eq!(voxel.surfaces[0], "tr_canyon_rock3d_406b");
        assert_eq!(
            voxel.mat4().transform_point3(Vec3::ZERO),
            Vec3::new(832.0, 832.0, 434.29898)
        );
        let mesh = voxel.compiled_collision().unwrap();
        assert_eq!((mesh.positions.len(), mesh.indices.len() / 3), (2901, 5642));
        let counts = std::array::from_fn::<_, 3, _>(|id| {
            mesh.materials.iter().filter(|&&v| v == id as u16).count()
        });
        assert_eq!(counts, [5262, 362, 18]);
        let raw = decode(&voxel.compiled).unwrap();
        let file = ChunkFile::parse(&raw).unwrap();
        let model = aa_cry::Model::from_chunks(&file).unwrap();
        let render = &model.meshes[0];
        let vertex_mats = file
            .of_type(ty::DATA_STREAM)
            .find(|c| u32_at(c.data, 4) == 11)
            .unwrap();
        assert_eq!(
            (u32_at(vertex_mats.data, 8), u32_at(vertex_mats.data, 12)),
            (2901, 4)
        );
        let palette: Vec<_> = vertex_mats.data[24..]
            .chunks_exact(4)
            .map(|b| b[0])
            .collect();
        let mut oracle = std::collections::HashMap::new();
        for tri in render.indices.as_chunks::<3>().0 {
            let ids = tri.map(|i| palette[i as usize]);
            let id = if ids[1] == ids[2] && ids[1] != ids[0] {
                ids[1]
            } else {
                ids[0]
            };
            let mut key = *tri;
            key.sort_unstable();
            oracle.insert(key, u16::from(id));
        }
        assert_eq!(mesh.positions, render.positions);
        assert_ne!(
            mesh.indices, render.indices,
            "native serialized proxy reorders triangles"
        );
        for (tri, &id) in mesh.indices.as_chunks::<3>().0.iter().zip(&mesh.materials) {
            let mut key = *tri;
            key.sort_unstable();
            assert_eq!(oracle.get(&key), Some(&id));
        }
        let origin = crate::coords::cell_origin(20, 8);
        let p = origin + voxel.mat4().transform_point3(mesh.positions[0]);
        assert!(p.x >= 21312.0 && p.x < 21376.0 && p.y >= 9024.0 && p.y < 9088.0);
    }
}
