//! ArcheAge 1.2 sound.

pub mod decode;
pub mod fsb5;
pub mod mpeg;
pub mod vorbis;

pub use decode::{Pcm, decode};
pub use fsb5::Fsb5;
