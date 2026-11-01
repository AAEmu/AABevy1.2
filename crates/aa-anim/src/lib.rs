//! Animations, CDF characters, character assembly and customizing.
//!
//! * [`caf`]: `.caf` clips (per-bone rotation / position tracks) decoded to plain data.
//! * [`cal`], [`cdf`], [`lmg`]: the text formats around them (animation name tables, character
//!   definitions, locomotion groups).
//!
//! See `docs/aa-anim.md`.

pub mod animator;
pub mod caf;
pub mod cal;
pub mod cdf;
pub mod character;
pub mod custom;
pub mod cut;
pub mod db;
pub mod dye;
pub mod face;
pub mod facial;
pub mod hair;
mod face_normal;
pub mod lmg;
pub mod morph;
pub mod pose;
pub mod rope;

pub use animator::{AnimPlugin, AnimSet, Animator, ClipLibrary, Playing, RandomIdle, spawn_bones};
pub use caf::{Clip, Track};
pub use cal::Cal;
pub use cdf::Cdf;
pub use character::{Character, CharacterAssets, CharacterDesc, Costume, PartAsset, PartKind, spawn_character, spawn_character_with_facial};
pub use custom::Customization;
pub use db::{GameDb, Gender, Race};
pub use lmg::Lmg;
pub use morph::{FaceMaker, MorphSet};
pub use pose::{BoneMask, BoundClip, ClipBinding, Pose, Rig, SampleOptions};
pub use rope::{NoRope, RopeSettings, RopeSim};

/// Errors of every loader in this crate.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Cry(#[from] aa_cry::Error),
    #[error("bad data: {0}")]
    Format(String),
    #[error("xml: {0}")]
    Xml(String),
    #[error("database: {0}")]
    Db(String),
    #[error("not found: {0}")]
    Missing(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
