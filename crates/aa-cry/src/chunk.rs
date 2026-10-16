//! CryEngine chunk files (`.cgf`, `.cga`, `.chr`, `.caf`, `.anm`): the container every compiled
//! CryEngine 3 asset of the 1.2 client uses.
//!
//! ```text
//! 0x00 char[8] "CryTek\0\0"
//! 0x08 u32     file type   0xFFFF0000 geometry (.cgf/.cga/.chr/.anm), 0xFFFF0001 animation (.caf)
//! 0x0C u32     version     0x744 or 0x745 (the only two in the 1.2 pak)
//! 0x10 u32     chunk table offset (usually 0x14; some 0x744 files keep the table at the end)
//! table: u32 count, then per chunk
//!   0x744: {u32 type, u32 version, u32 offset, u32 id}            sizes = distance to the next chunk
//!   0x745: {u32 type, u32 version, u32 offset, u32 id, u32 size}
//! ```
//!
//! Chunk data normally starts with a 16-byte copy of `{type, version, offset, id}`;
//! [`Chunk::data`] skips it when present (SourceInfo chunks often have none).
//!
//! ```no_run
//! let bytes = aa_cry::read("objects/characters/elf/female/nude/el_f_base.chr").unwrap();
//! let file = aa_cry::chunk::ChunkFile::parse(&bytes).unwrap();
//! for c in file.of_type(aa_cry::chunk::ty::NODE) {
//!     let mut r = c.reader();
//!     println!("node {} {}", c.id, r.fixed_str(64).unwrap());
//! }
//! ```

use crate::{Error, Result};
use glam::{Quat, Vec2, Vec3};

/// Chunk type ids (the `type` field of the chunk table).
pub mod ty {
    pub const MESH: u32 = 0xCCCC_0000;
    pub const HELPER: u32 = 0xCCCC_0001;
    pub const VERT_ANIM: u32 = 0xCCCC_0002;
    pub const BONE_ANIM: u32 = 0xCCCC_0003;
    pub const GEOM_NAME_LIST: u32 = 0xCCCC_0004;
    pub const BONE_NAME_LIST: u32 = 0xCCCC_0005;
    pub const MTL_LIST: u32 = 0xCCCC_0006;
    pub const MRM: u32 = 0xCCCC_0007;
    pub const SCENE_PROPS: u32 = 0xCCCC_0008;
    pub const LIGHT: u32 = 0xCCCC_0009;
    pub const PATCH_MESH: u32 = 0xCCCC_000A;
    pub const NODE: u32 = 0xCCCC_000B;
    pub const MTL: u32 = 0xCCCC_000C;
    pub const CONTROLLER: u32 = 0xCCCC_000D;
    pub const TIMING: u32 = 0xCCCC_000E;
    pub const BONE_MESH: u32 = 0xCCCC_000F;
    pub const BONE_LIGHT_BINDING: u32 = 0xCCCC_0010;
    pub const MESH_MORPH_TARGET: u32 = 0xCCCC_0011;
    pub const BONE_INITIAL_POS: u32 = 0xCCCC_0012;
    pub const SOURCE_INFO: u32 = 0xCCCC_0013;
    pub const MTL_NAME: u32 = 0xCCCC_0014;
    pub const EXPORT_FLAGS: u32 = 0xCCCC_0015;
    pub const DATA_STREAM: u32 = 0xCCCC_0016;
    pub const MESH_SUBSETS: u32 = 0xCCCC_0017;
    pub const MESH_PHYSICS_DATA: u32 = 0xCCCC_0018;

    pub const COMPILED_BONES: u32 = 0xACDC_0000;
    pub const COMPILED_PHYSICAL_BONES: u32 = 0xACDC_0001;
    pub const COMPILED_MORPH_TARGETS: u32 = 0xACDC_0002;
    pub const COMPILED_PHYSICAL_PROXIES: u32 = 0xACDC_0003;
    pub const COMPILED_INT_FACES: u32 = 0xACDC_0004;
    pub const COMPILED_INT_SKIN_VERTICES: u32 = 0xACDC_0005;
    pub const COMPILED_EXT_TO_INT_MAP: u32 = 0xACDC_0006;

    pub const BREAKABLE_PHYSICS: u32 = 0xAAFC_0000;
    pub const FACE_MAP: u32 = 0xAAFC_0001;
    /// `.caf`: motion parameters (v0x922).
    pub const MOTION_PARAMETERS: u32 = 0xAAFC_0002;
    /// `.caf`: foot plant info (v0x920).
    pub const FOOT_PLANT_INFO: u32 = 0xAAFC_0003;
    /// `.chr`: per-bone vertex boxes.
    pub const BONES_BOXES: u32 = 0xAAFC_0004;
    pub const FOLIAGE_INFO: u32 = 0xAAFC_0005;

    /// Short readable name for logs.
    pub fn name(t: u32) -> &'static str {
        match t {
            MESH => "Mesh",
            HELPER => "Helper",
            VERT_ANIM => "VertAnim",
            BONE_ANIM => "BoneAnim",
            GEOM_NAME_LIST => "GeomNameList",
            BONE_NAME_LIST => "BoneNameList",
            MTL_LIST => "MtlList",
            MRM => "MRM",
            SCENE_PROPS => "SceneProps",
            LIGHT => "Light",
            PATCH_MESH => "PatchMesh",
            NODE => "Node",
            MTL => "Mtl",
            CONTROLLER => "Controller",
            TIMING => "Timing",
            BONE_MESH => "BoneMesh",
            BONE_LIGHT_BINDING => "BoneLightBinding",
            MESH_MORPH_TARGET => "MeshMorphTarget",
            BONE_INITIAL_POS => "BoneInitialPos",
            SOURCE_INFO => "SourceInfo",
            MTL_NAME => "MtlName",
            EXPORT_FLAGS => "ExportFlags",
            DATA_STREAM => "DataStream",
            MESH_SUBSETS => "MeshSubsets",
            MESH_PHYSICS_DATA => "MeshPhysicsData",
            COMPILED_BONES => "CompiledBones",
            COMPILED_PHYSICAL_BONES => "CompiledPhysicalBones",
            COMPILED_MORPH_TARGETS => "CompiledMorphTargets",
            COMPILED_PHYSICAL_PROXIES => "CompiledPhysicalProxies",
            COMPILED_INT_FACES => "CompiledIntFaces",
            COMPILED_INT_SKIN_VERTICES => "CompiledIntSkinVertices",
            COMPILED_EXT_TO_INT_MAP => "CompiledExt2IntMap",
            BREAKABLE_PHYSICS => "BreakablePhysics",
            FACE_MAP => "FaceMap",
            MOTION_PARAMETERS => "MotionParameters",
            FOOT_PLANT_INFO => "FootPlantInfo",
            BONES_BOXES => "BonesBoxes",
            FOLIAGE_INFO => "FoliageInfo",
            _ => "?",
        }
    }
}

/// `file_type` of geometry files (`.cgf`, `.cga`, `.chr`, `.anm`).
pub const FILE_GEOMETRY: u32 = 0xFFFF_0000;
/// `file_type` of animation files (`.caf`).
pub const FILE_ANIMATION: u32 = 0xFFFF_0001;

/// One chunk: table entry plus its data.
#[derive(Clone, Copy, Debug)]
pub struct Chunk<'a> {
    pub ty: u32,
    pub version: u32,
    pub id: i32,
    /// File offset of the chunk (including the header copy, if any).
    pub offset: u32,
    /// Chunk payload, after the 16-byte header copy when the chunk has one.
    pub data: &'a [u8],
}

impl<'a> Chunk<'a> {
    /// A little-endian reader over [`Chunk::data`].
    pub fn reader(&self) -> Reader<'a> {
        Reader::new(self.data)
    }
}

/// A parsed chunk file borrowing the file bytes (no copies).
#[derive(Clone, Debug)]
pub struct ChunkFile<'a> {
    pub data: &'a [u8],
    pub file_type: u32,
    /// 0x744 or 0x745.
    pub version: u32,
    /// In chunk-table order.
    pub chunks: Vec<Chunk<'a>>,
    /// (id, index into `chunks`), sorted by id.
    by_id: Vec<(i32, u32)>,
}

impl<'a> ChunkFile<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        if data.len() < 20 || &data[..6] != b"CryTek" {
            return Err(Error::NotChunkFile);
        }
        let mut r = Reader::at(data, 8);
        let file_type = r.u32()?;
        let version = r.u32()?;
        let table = r.u32()? as usize;
        let entry_size = match version {
            0x744 => 16,
            0x745 => 20,
            v => return Err(Error::Version(v)),
        };
        let mut t = Reader::at(data, table);
        let count = t.u32()? as usize;
        if count > (data.len().saturating_sub(table + 4)) / entry_size {
            return Err(Error::invalid(format!("chunk table: {count} entries at {table:#x}")));
        }
        let mut raw = Vec::with_capacity(count);
        for _ in 0..count {
            let ty = t.u32()?;
            let ver = t.u32()?;
            let offset = t.u32()?;
            let id = t.i32()?;
            let size = if entry_size == 20 { t.u32()? } else { 0 };
            raw.push((ty, ver, offset, id, size));
        }
        if version == 0x744 {
            // Sizes run to the next chunk, the table, or the end of the file.
            let mut ends: Vec<u32> = raw.iter().map(|c| c.2).collect();
            ends.push(table as u32);
            ends.push(data.len() as u32);
            ends.sort_unstable();
            ends.dedup();
            for c in &mut raw {
                let i = ends.partition_point(|&e| e <= c.2);
                c.4 = ends.get(i).copied().unwrap_or(data.len() as u32) - c.2;
            }
        }
        let mut chunks = Vec::with_capacity(count);
        for (ty, ver, offset, id, size) in raw {
            let start = offset as usize;
            if start > data.len() {
                return Err(Error::invalid(format!("chunk {id} offset {start:#x} past end")));
            }
            let end = start.saturating_add(size as usize).min(data.len());
            let mut body = &data[start..end];
            if body.len() >= 16 {
                let w = |i: usize| u32::from_le_bytes(body[i..i + 4].try_into().unwrap());
                if w(0) == ty && w(8) == offset && w(12) as i32 == id {
                    body = &body[16..];
                }
            }
            chunks.push(Chunk { ty, version: ver, id, offset, data: body });
        }
        let mut by_id: Vec<(i32, u32)> =
            chunks.iter().enumerate().map(|(i, c)| (c.id, i as u32)).collect();
        by_id.sort_by_key(|e| e.0);
        Ok(ChunkFile { data, file_type, version, chunks, by_id })
    }

    /// The chunk with this id. References between chunks use ids <= 0 for "none"; prefer
    /// [`ChunkFile::get_typed`] for them (id 0 is usually the SourceInfo chunk).
    pub fn get(&self, id: i32) -> Option<&Chunk<'a>> {
        let i = self.by_id.binary_search_by_key(&id, |e| e.0).ok()?;
        Some(&self.chunks[self.by_id[i].1 as usize])
    }

    /// The chunk with this id if it has the expected type.
    pub fn get_typed(&self, id: i32, ty: u32) -> Option<&Chunk<'a>> {
        self.get(id).filter(|c| c.ty == ty)
    }

    /// All chunks of a type, in table order.
    pub fn of_type(&self, ty: u32) -> impl Iterator<Item = &Chunk<'a>> + '_ {
        self.chunks.iter().filter(move |c| c.ty == ty)
    }

    /// The first chunk of a type.
    pub fn first(&self, ty: u32) -> Option<&Chunk<'a>> {
        self.chunks.iter().find(|c| c.ty == ty)
    }
}

/// Little-endian cursor with bounds-checked reads (errors instead of panics).
#[derive(Clone, Debug)]
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Reader { data, pos: 0 }
    }

    pub fn at(data: &'a [u8], pos: usize) -> Self {
        Reader { data, pos }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    pub fn seek(&mut self, pos: usize) {
        self.pos = pos;
    }

    pub fn skip(&mut self, n: usize) -> Result<()> {
        self.bytes(n).map(|_| ())
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.data.len()).ok_or(
            Error::Truncated { at: self.pos, need: n, len: self.data.len() },
        )?;
        let b = &self.data[self.pos..end];
        self.pos = end;
        Ok(b)
    }

    fn arr<const N: usize>(&mut self) -> Result<[u8; N]> {
        Ok(self.bytes(N)?.try_into().unwrap())
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.arr::<1>()?[0])
    }
    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.arr()?))
    }
    pub fn i16(&mut self) -> Result<i16> {
        Ok(i16::from_le_bytes(self.arr()?))
    }
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.arr()?))
    }
    pub fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.arr()?))
    }
    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.arr()?))
    }
    pub fn vec2(&mut self) -> Result<Vec2> {
        Ok(Vec2::new(self.f32()?, self.f32()?))
    }
    pub fn vec3(&mut self) -> Result<Vec3> {
        Ok(Vec3::new(self.f32()?, self.f32()?, self.f32()?))
    }
    /// Quaternion stored as `x, y, z, w` (CryEngine `Quat` is `{Vec3 v; float w}`).
    pub fn quat_xyzw(&mut self) -> Result<Quat> {
        Ok(Quat::from_xyzw(self.f32()?, self.f32()?, self.f32()?, self.f32()?))
    }
    /// Row-major 3x4 `Matrix34` (rotation/scale | translation) as a glam affine transform.
    pub fn matrix34(&mut self) -> Result<glam::Affine3A> {
        let mut m = [0f32; 12];
        for v in &mut m {
            *v = self.f32()?;
        }
        Ok(matrix34_to_affine(&m))
    }
    /// Fixed-size, NUL-padded string (bytes after the first NUL are ignored; latin-1).
    pub fn fixed_str(&mut self, n: usize) -> Result<String> {
        Ok(cstr(self.bytes(n)?))
    }
}

/// Row-major `Matrix34` (`m[row*4 + col]`, column 3 = translation) to a glam affine.
pub fn matrix34_to_affine(m: &[f32; 12]) -> glam::Affine3A {
    glam::Affine3A::from_cols(
        glam::Vec3A::new(m[0], m[4], m[8]),
        glam::Vec3A::new(m[1], m[5], m[9]),
        glam::Vec3A::new(m[2], m[6], m[10]),
        glam::Vec3A::new(m[3], m[7], m[11]),
    )
}

/// Bytes up to the first NUL as a string (latin-1, so every byte maps to a char).
pub fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    b[..end].iter().map(|&c| c as char).collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Builds a 0x745 chunk file: `(type, version, id, payload)`; payloads get the header copy.
    pub(crate) fn build_745(chunks: &[(u32, u32, i32, Vec<u8>)]) -> Vec<u8> {
        let table = 20usize;
        let mut out = Vec::new();
        out.extend_from_slice(b"CryTek\0\0");
        out.extend_from_slice(&FILE_GEOMETRY.to_le_bytes());
        out.extend_from_slice(&0x745u32.to_le_bytes());
        out.extend_from_slice(&(table as u32).to_le_bytes());
        out.extend_from_slice(&(chunks.len() as u32).to_le_bytes());
        let mut off = table + 4 + chunks.len() * 20;
        let mut offsets = Vec::new();
        for (t, v, id, body) in chunks {
            let size = 16 + body.len();
            for w in [*t, *v, off as u32, *id as u32, size as u32] {
                out.extend_from_slice(&w.to_le_bytes());
            }
            offsets.push(off);
            off += size;
        }
        for ((t, v, id, body), o) in chunks.iter().zip(offsets) {
            for w in [*t, *v, o as u32, *id as u32] {
                out.extend_from_slice(&w.to_le_bytes());
            }
            out.extend_from_slice(body);
        }
        out
    }

    #[test]
    fn parses_745() {
        let f = build_745(&[
            (ty::TIMING, 0x918, 2, vec![1, 2, 3, 4]),
            (ty::NODE, 0x823, 7, vec![9; 10]),
        ]);
        let cf = ChunkFile::parse(&f).unwrap();
        assert_eq!(cf.version, 0x745);
        assert_eq!(cf.chunks.len(), 2);
        assert_eq!(cf.get(2).unwrap().data, &[1, 2, 3, 4]);
        assert_eq!(cf.get(7).unwrap().data.len(), 10);
        assert!(cf.get(3).is_none());
        assert_eq!(cf.of_type(ty::NODE).count(), 1);
    }

    #[test]
    fn parses_744_with_table_at_end() {
        // header, two chunks with header copies, table at the end
        let mut f = Vec::new();
        f.extend_from_slice(b"CryTek\0\0");
        f.extend_from_slice(&FILE_GEOMETRY.to_le_bytes());
        f.extend_from_slice(&0x744u32.to_le_bytes());
        f.extend_from_slice(&0u32.to_le_bytes()); // patched below
        let c1 = f.len() as u32;
        for w in [ty::HELPER, 0x744, c1, 1] {
            f.extend_from_slice(&w.to_le_bytes());
        }
        f.extend_from_slice(&[5; 8]);
        let c2 = f.len() as u32;
        for w in [ty::NODE, 0x823, c2, 2] {
            f.extend_from_slice(&w.to_le_bytes());
        }
        f.extend_from_slice(&[6; 3]);
        let table = f.len() as u32;
        f[16..20].copy_from_slice(&table.to_le_bytes());
        f.extend_from_slice(&2u32.to_le_bytes());
        for w in [ty::HELPER, 0x744, c1, 1, ty::NODE, 0x823, c2, 2] {
            f.extend_from_slice(&w.to_le_bytes());
        }
        let cf = ChunkFile::parse(&f).unwrap();
        assert_eq!(cf.get(1).unwrap().data, &[5; 8]);
        assert_eq!(cf.get(2).unwrap().data, &[6; 3]);
    }

    #[test]
    fn rejects_garbage() {
        assert!(matches!(ChunkFile::parse(b"hello world, not cry"), Err(Error::NotChunkFile)));
        let mut f = build_745(&[]);
        f[16..20].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        assert!(ChunkFile::parse(&f).is_err());
    }

    #[test]
    fn reader_bounds() {
        let mut r = Reader::new(&[1, 0, 0, 0, 2]);
        assert_eq!(r.u32().unwrap(), 1);
        assert!(r.u32().is_err());
        assert_eq!(r.u8().unwrap(), 2);
    }
}
