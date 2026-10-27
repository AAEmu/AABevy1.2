//! Skeletons from `.chr` (and the few legacy `.cgf`/`.chr` with uncompiled bones).
//!
//! **CompiledBones** (`0xACDC0000`, v0x801, the only version in the 1.2 pak): 32 reserved bytes,
//! then one 324-byte record per bone, parents before children:
//!
//! ```text
//! +0   BonePhysics phys[2]       (alive, ragdoll; 104 bytes each, see BonePhysics)
//! +208 u32   controller_id       CRC-32 (IEEE) of the case-sensitive bone name = CAF controller id
//! +212 u32   limb_id
//! +216 char  name[48]
//! +264 i32   parent_offset       relative index of the parent (0 = root)
//! +268 u32   num_children
//! +272 i32   child_offset        relative index of the first child
//! +276 Matrix34 bone_to_model    row-major 3x4, metres, CryEngine model space (Z up)
//! ```
//!
//! **Legacy** (97 files): `BoneAnim` (0xCCCC0003 v0x290: `u32 count`, 152-byte
//! `{i32 id, i32 parent, i32 children, u32 controller_id, char props[32], BonePhysics}`),
//! `BoneNameList` (0xCCCC0005 v0x745: `u32 count`, NUL-terminated names) and `BoneInitialPos`
//! (0xCCCC0012 v1: `i32 mesh, u32 count, float[4][3]` per bone = axes + translation in cm).
//!
//! Skin attachments (`.chr` body parts) carry their own, smaller bone list; match bones to the
//! base skeleton by [`Bone::controller_id`] (or name). LOD files (`*_lodN.chr`) often have no
//! bones at all and use the bone list of their LOD0 file.

use crate::chunk::{ChunkFile, Reader, ty};
use crate::{Error, Result};
use glam::{Affine3A, Mat4, Quat, Vec3};

/// Physics parameters of a bone (`CryBonePhysics_Comp`, 104 bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BonePhysics {
    /// Physical geometry id, -1 = none.
    pub geometry: i32,
    pub flags: u32,
    pub min: Vec3,
    pub max: Vec3,
    pub spring_angle: Vec3,
    pub spring_tension: Vec3,
    pub damping: Vec3,
    /// Joint frame, row-major 3x3.
    pub frame: [f32; 9],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Bone {
    pub name: String,
    /// CRC-32 of the case-sensitive name; animation controllers reference bones by this id.
    pub controller_id: u32,
    pub parent: Option<usize>,
    pub limb_id: u32,
    /// Bind pose, bone space -> model space (CryEngine Z-up model space, metres).
    pub bind_model: Affine3A,
    /// Bind pose relative to the parent bone (equals `bind_model` for roots).
    pub bind_local: Affine3A,
    /// `[alive, ragdoll]`; zeroed for legacy skeletons.
    pub physics: [BonePhysics; 2],
}

impl Bone {
    /// Local bind rotation and translation (scale is 1 in every 1.2 skeleton).
    pub fn local_rotation_translation(&self) -> (Quat, Vec3) {
        let (_, r, t) = self.bind_local.to_scale_rotation_translation();
        (r, t)
    }

    /// Model-space bind rotation and translation.
    pub fn model_rotation_translation(&self) -> (Quat, Vec3) {
        let (_, r, t) = self.bind_model.to_scale_rotation_translation();
        (r, t)
    }

    /// Inverse bind matrix (model space -> bone space), as skinning wants it.
    pub fn inverse_bind(&self) -> Mat4 {
        Mat4::from(self.bind_model.inverse())
    }
}

/// A bone hierarchy in parent-before-child order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Skeleton {
    pub bones: Vec<Bone>,
}

impl Skeleton {
    /// Reads the skeleton of a chunk file: CompiledBones, else the legacy bone chunks.
    /// `Ok(None)` when the file has no bones (static meshes, most LOD `.chr`).
    pub fn from_chunks(file: &ChunkFile<'_>) -> Result<Option<Skeleton>> {
        if let Some(c) = file.first(ty::COMPILED_BONES) {
            if c.version != 0x801 {
                return Err(Error::Unsupported(format!(
                    "CompiledBones version {:#x}",
                    c.version
                )));
            }
            return compiled_bones_801(c.data).map(Some);
        }
        if file.first(ty::BONE_ANIM).is_some() {
            return legacy_bones(file).map(Some);
        }
        Ok(None)
    }

    /// Parses a file's bytes and reads its skeleton.
    pub fn from_bytes(bytes: &[u8]) -> Result<Option<Skeleton>> {
        Self::from_chunks(&ChunkFile::parse(bytes)?)
    }

    /// Loads a skeleton from the pak. For `*_lodN.chr` without bones, falls back to the LOD0 file.
    pub fn load(path: &str) -> Result<Skeleton> {
        if let Some(s) = Self::from_bytes(&crate::read(path)?)? {
            return Ok(s);
        }
        if let Some(base) = crate::lod0_path(path) {
            if let Some(s) = Self::from_bytes(&crate::read(&base)?)? {
                return Ok(s);
            }
        }
        Err(Error::invalid(format!("{path}: no skeleton")))
    }

    pub fn len(&self) -> usize {
        self.bones.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bones.is_empty()
    }

    /// Index of a bone by exact name.
    pub fn find(&self, name: &str) -> Option<usize> {
        self.bones.iter().position(|b| b.name == name)
    }

    /// Index of a bone by name, ignoring ASCII case.
    pub fn find_ignore_case(&self, name: &str) -> Option<usize> {
        self.bones.iter().position(|b| b.name.eq_ignore_ascii_case(name))
    }

    /// Index of a bone by controller id (CAF controllers).
    pub fn find_controller(&self, controller_id: u32) -> Option<usize> {
        self.bones.iter().position(|b| b.controller_id == controller_id)
    }

    /// Children of a bone.
    pub fn children(&self, i: usize) -> impl Iterator<Item = usize> + '_ {
        self.bones.iter().enumerate().filter(move |(_, b)| b.parent == Some(i)).map(|(j, _)| j)
    }

    /// For each bone of `self`, the index of the bone with the same controller id (falling back
    /// to the same name, ignoring case) in `base`. Used to drive skin attachments with the
    /// base skeleton.
    pub fn map_to(&self, base: &Skeleton) -> Vec<Option<usize>> {
        self.bones
            .iter()
            .map(|b| {
                base.find_controller(b.controller_id).or_else(|| base.find_ignore_case(&b.name))
            })
            .collect()
    }
}

/// CRC-32 (IEEE, as zlib) of a bone name: the controller id CAF files use.
pub fn controller_id(name: &str) -> u32 {
    crc32(name.as_bytes())
}

/// CRC-32 (IEEE 802.3 polynomial, reflected, as zlib's `crc32`).
pub fn crc32(bytes: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, e) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *e = c;
        }
        t
    });
    let mut c = !0u32;
    for &b in bytes {
        c = table[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    !c
}

fn bone_physics(r: &mut Reader<'_>) -> Result<BonePhysics> {
    let geometry = r.i32()?;
    let flags = r.u32()?;
    let min = r.vec3()?;
    let max = r.vec3()?;
    let spring_angle = r.vec3()?;
    let spring_tension = r.vec3()?;
    let damping = r.vec3()?;
    let mut frame = [0f32; 9];
    for v in &mut frame {
        *v = r.f32()?;
    }
    Ok(BonePhysics { geometry, flags, min, max, spring_angle, spring_tension, damping, frame })
}

const BONE_801_SIZE: usize = 324;

fn compiled_bones_801(data: &[u8]) -> Result<Skeleton> {
    if data.len() < 32 || (data.len() - 32) % BONE_801_SIZE != 0 {
        return Err(Error::invalid(format!("CompiledBones 0x801: {} bytes", data.len())));
    }
    let n = (data.len() - 32) / BONE_801_SIZE;
    let mut bones = Vec::with_capacity(n);
    let mut r = Reader::at(data, 32);
    for i in 0..n {
        let physics = [bone_physics(&mut r)?, bone_physics(&mut r)?];
        let controller_id = r.u32()?;
        let limb_id = r.u32()?;
        let name = r.fixed_str(48)?;
        let parent_offset = r.i32()?;
        let _num_children = r.u32()?;
        let _child_offset = r.i32()?;
        let bind_model = r.matrix34()?;
        let parent = if parent_offset == 0 {
            None
        } else {
            let p = i as i64 + parent_offset as i64;
            if p < 0 || p >= i as i64 {
                return Err(Error::invalid(format!("bone {i} ({name}): parent offset {parent_offset}")));
            }
            Some(p as usize)
        };
        bones.push(Bone {
            name,
            controller_id,
            parent,
            limb_id,
            bind_model,
            bind_local: bind_model,
            physics,
        });
    }
    Ok(finish(bones))
}

fn legacy_bones(file: &ChunkFile<'_>) -> Result<Skeleton> {
    let anim = file.first(ty::BONE_ANIM).unwrap();
    let mut r = anim.reader();
    let n = r.u32()? as usize;
    if n > 4096 {
        return Err(Error::invalid(format!("BoneAnim: {n} bones")));
    }
    // (id, parent id, controller id)
    let mut entities = Vec::with_capacity(n);
    for _ in 0..n {
        let id = r.i32()?;
        let parent = r.i32()?;
        let _children = r.i32()?;
        let controller_id = r.u32()?;
        r.skip(32)?; // props
        let _phys = bone_physics(&mut r)?;
        entities.push((id, parent, controller_id));
    }
    let mut names = Vec::with_capacity(n);
    if let Some(c) = file.first(ty::BONE_NAME_LIST) {
        let mut r = c.reader();
        let count = r.u32()? as usize;
        let rest = r.bytes(r.remaining())?;
        names.extend(rest.split(|&b| b == 0).take(count).map(crate::chunk::cstr));
    }
    let mut poses = vec![Affine3A::IDENTITY; n];
    if let Some(c) = file.first(ty::BONE_INITIAL_POS) {
        let mut r = c.reader();
        let _mesh = r.i32()?;
        let count = (r.u32()? as usize).min(n);
        for pose in poses.iter_mut().take(count) {
            let mut m = [0f32; 12];
            for v in &mut m {
                *v = r.f32()?;
            }
            // float[4][3]: rows are the images of X, Y, Z and the translation (cm).
            *pose = Affine3A::from_cols(
                glam::Vec3A::new(m[0], m[1], m[2]),
                glam::Vec3A::new(m[3], m[4], m[5]),
                glam::Vec3A::new(m[6], m[7], m[8]),
                glam::Vec3A::new(m[9], m[10], m[11]) * 0.01,
            );
        }
    }
    // Bone ids are indices; reorder to parent-before-child if needed.
    let index_of = |id: i32| entities.iter().position(|e| e.0 == id);
    let mut bones: Vec<Bone> = entities
        .iter()
        .enumerate()
        .map(|(i, &(_, parent, controller_id))| Bone {
            name: names.get(i).cloned().unwrap_or_else(|| format!("bone{i}")),
            controller_id,
            parent: if parent < 0 { None } else { index_of(parent) },
            limb_id: 0,
            bind_model: poses[i],
            bind_local: poses[i],
            physics: Default::default(),
        })
        .collect();
    if bones.iter().enumerate().any(|(i, b)| b.parent.is_some_and(|p| p >= i)) {
        bones = topo_sort(bones);
    }
    Ok(finish(bones))
}

/// Reorders bones so parents precede children (stable), remapping parent indices.
fn topo_sort(bones: Vec<Bone>) -> Vec<Bone> {
    let n = bones.len();
    let mut order = Vec::with_capacity(n);
    let mut placed = vec![false; n];
    while order.len() < n {
        let before = order.len();
        for i in 0..n {
            if !placed[i] && bones[i].parent.is_none_or(|p| placed[p]) {
                placed[i] = true;
                order.push(i);
            }
        }
        if order.len() == before {
            // cycle: detach the rest
            for (i, p) in placed.iter_mut().enumerate() {
                if !*p {
                    *p = true;
                    order.push(i);
                }
            }
        }
    }
    let mut new_index = vec![0; n];
    for (new, &old) in order.iter().enumerate() {
        new_index[old] = new;
    }
    order
        .iter()
        .map(|&old| {
            let mut b = bones[old].clone();
            b.parent = b.parent.map(|p| new_index[p]).filter(|&p| p < new_index[old]);
            b
        })
        .collect()
}

fn finish(mut bones: Vec<Bone>) -> Skeleton {
    for i in 0..bones.len() {
        if let Some(p) = bones[i].parent {
            bones[i].bind_local = bones[p].bind_model.inverse() * bones[i].bind_model;
        }
    }
    Skeleton { bones }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::tests::build_745;

    #[test]
    fn crc_matches_zlib() {
        assert_eq!(controller_id("Bip01"), 0x7856_1A24);
        assert_eq!(controller_id("Bip01 Pelvis"), 0xD355_3473);
        assert_eq!(controller_id("camera"), 0x3B1C_EE05);
    }

    fn bone_record(name: &str, parent_offset: i32, t: Vec3) -> Vec<u8> {
        let mut b = vec![0u8; BONE_801_SIZE];
        b[208..212].copy_from_slice(&controller_id(name).to_le_bytes());
        b[216..216 + name.len()].copy_from_slice(name.as_bytes());
        b[264..268].copy_from_slice(&parent_offset.to_le_bytes());
        let m = [1.0, 0.0, 0.0, t.x, 0.0, 1.0, 0.0, t.y, 0.0, 0.0, 1.0, t.z];
        for (k, v) in m.iter().enumerate() {
            b[276 + 4 * k..280 + 4 * k].copy_from_slice(&f32::to_le_bytes(*v));
        }
        b
    }

    #[test]
    fn compiled_bones() {
        let mut body = vec![0u8; 32];
        body.extend(bone_record("Bip01", 0, Vec3::new(0.0, 0.0, 1.0)));
        body.extend(bone_record("Bip01 Pelvis", -1, Vec3::new(0.0, 0.0, 1.1)));
        body.extend(bone_record("Bip01 Spine", -1, Vec3::new(0.0, 0.5, 1.1)));
        let f = build_745(&[(ty::COMPILED_BONES, 0x801, 5, body)]);
        let s = Skeleton::from_bytes(&f).unwrap().unwrap();
        assert_eq!(s.len(), 3);
        assert_eq!(s.bones[2].parent, Some(1));
        assert_eq!(s.find("Bip01 Spine"), Some(2));
        assert_eq!(s.find_controller(controller_id("Bip01 Pelvis")), Some(1));
        let (_, t) = s.bones[2].local_rotation_translation();
        assert!((t - Vec3::new(0.0, 0.5, 0.0)).length() < 1e-6);
        assert_eq!(s.children(1).collect::<Vec<_>>(), vec![2]);
    }

    /// Needs the real client (`AA_PAK` or `<client>/game_pak`); skipped when it is missing.
    #[test]
    fn real_elf_female_skeleton() {
        if !crate::pak_available() {
            return;
        }
        let s = Skeleton::load("objects/characters/elf/female/nude/el_f_base.chr").unwrap();
        assert_eq!(s.len(), 157);
        assert_eq!(s.bones[0].name, "Bip01");
        assert_eq!(s.bones[0].parent, None);
        let pelvis = s.find("Bip01 Pelvis").unwrap();
        let (_, t) = s.bones[pelvis].model_rotation_translation();
        assert!((t.z - 1.013).abs() < 0.01, "{t}");
        for (i, b) in s.bones.iter().enumerate() {
            assert_eq!(b.controller_id, controller_id(&b.name), "{i} {}", b.name);
        }
        // left side is -X
        let lt = s.find("Bip01 L Thigh").unwrap();
        assert!(s.bones[lt].model_rotation_translation().1.x < 0.0);
        // LOD file without bones falls back to LOD0
        let lod = Skeleton::load("objects/characters/elf/female/nude/el_f_base_lod1.chr").unwrap();
        assert!(!lod.is_empty());
        // legacy skeleton
        let buffalo = Skeleton::load("objects/characters/animals/buffalo/buffalo.cgf").unwrap();
        assert_eq!(buffalo.len(), 42);
        assert_eq!(buffalo.bones[0].name, "Bip01");
    }
}
