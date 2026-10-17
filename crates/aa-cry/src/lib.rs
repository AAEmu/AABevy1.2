//! CryEngine 3 asset formats of the ArcheAge 1.2 client, loaded into Bevy.
//!
//! Layers (see `docs/aa-cry.md`):
//!
//! * plain data, no Bevy: [`chunk`] (chunk file container), [`skeleton`] (bones / bind poses);
//! * Bevy: textures, materials, meshes and model spawning.
//!
//! All plain data is in CryEngine model space: right handed, **Z up**, metres, characters face
//! **+Y** (their left is -X). Bevy is Y up; [`CRY_TO_BEVY`] rotates one into the other, which
//! leaves characters facing Bevy **-Z** (turn them by PI about Y to face a camera on +Z).

pub mod chunk;
pub mod loader;
pub mod material;
pub mod mesh;
pub mod scene;
pub mod shade;
pub mod skeleton;
pub mod stage;
pub mod texture;
pub mod workers;

pub use chunk::ChunkFile;
pub use mesh::{MeshData, Model};
pub use loader::{CryLoader, PreparedModel};
pub use scene::{CryAssets, CryPlugin, MaterialHook, ModelOptions, Shading, VertexHook};
pub use skeleton::{Bone, Skeleton};

/// Rotation from CryEngine (Z up) to Bevy (Y up) space: `(x, y, z)` -> `(x, z, -y)`.
///
/// CryEngine characters face **+Y** (their left is -X), which this turns into Bevy **-Z**; props
/// authored facing -Y are the exception. To show a character to a camera on +Z, rotate its root
/// by PI about Y.
pub const CRY_TO_BEVY: glam::Quat = glam::Quat::from_xyzw(-std::f32::consts::FRAC_1_SQRT_2, 0.0, 0.0, std::f32::consts::FRAC_1_SQRT_2);

/// Errors of every loader in this crate.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not a CryTek chunk file")]
    NotChunkFile,
    #[error("unsupported chunk file version {0:#x}")]
    Version(u32),
    #[error("truncated: need {need} bytes at {at} of {len}")]
    Truncated { at: usize, need: usize, len: usize },
    #[error("invalid data: {0}")]
    Invalid(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl Error {
    pub(crate) fn invalid(msg: impl Into<String>) -> Self {
        Error::Invalid(msg.into())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Reads a file from the global `game_pak` (paths relative to `game/`, any case or slashes).
pub fn read(path: &str) -> Result<Vec<u8>> {
    Ok(aa_pak::Pak::global().read(path)?)
}

/// Whether the real client pak can be opened (tests that need it skip otherwise).
pub fn pak_available() -> bool {
    let path = std::env::var("AA_PAK").unwrap_or_else(|_| aa_pak::DEFAULT_PAK.to_string());
    std::path::Path::new(&path).exists()
}

/// `a/b_lod2.chr` -> `Some("a/b.chr")`; `None` when the name has no `_lodN` suffix.
pub fn lod0_path(path: &str) -> Option<String> {
    let (stem, ext) = path.rsplit_once('.')?;
    let lower = stem.to_ascii_lowercase();
    let i = lower.rfind("_lod")?;
    let digits = &stem[i + 4..];
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(format!("{}.{}", &stem[..i], ext))
}

/// `a/b.cgf`, 2 -> `a/b_lod2.cgf` (strips an existing `_lodN` first).
pub fn lod_path(path: &str, lod: u32) -> String {
    let base = lod0_path(path).unwrap_or_else(|| path.to_string());
    if lod == 0 {
        return base;
    }
    match base.rsplit_once('.') {
        Some((stem, ext)) => format!("{stem}_lod{lod}.{ext}"),
        None => format!("{base}_lod{lod}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lod_names() {
        assert_eq!(lod0_path("a/b_lod2.chr").as_deref(), Some("a/b.chr"));
        assert_eq!(lod0_path("a/b_LOD1.cgf").as_deref(), Some("a/b.cgf"));
        assert_eq!(lod0_path("a/b.chr"), None);
        assert_eq!(lod0_path("a/b_lodx.chr"), None);
        assert_eq!(lod_path("a/b.cgf", 1), "a/b_lod1.cgf");
        assert_eq!(lod_path("a/b_lod1.cgf", 0), "a/b.cgf");
    }

    #[test]
    fn cry_to_bevy_axes() {
        let up = CRY_TO_BEVY * glam::Vec3::Z;
        assert!((up - glam::Vec3::Y).length() < 1e-6);
        let fwd = CRY_TO_BEVY * glam::Vec3::Y;
        assert!((fwd - glam::Vec3::NEG_Z).length() < 1e-6);
    }
}
