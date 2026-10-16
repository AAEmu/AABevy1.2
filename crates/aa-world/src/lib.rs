//! Worlds of the ArcheAge 1.2 client: terrain, cell objects, vegetation, water, sky / time of day, streaming.
//!
//! See `docs/aa-world.md` for the formats, coordinates, API and performance numbers.

pub mod coords;
pub mod cover;
pub mod heightmap;
pub mod placement;
pub mod reader;
pub mod stream;
pub mod terrain;
pub mod world;

pub use coords::{CELL_SIZE, bevy_to_cry, cry_to_bevy};
pub use stream::{AaWorldPlugin, WorldConfig, WorldFocus};
pub use world::World;

/// Whether the real client pak can be opened (tests that need it skip otherwise).
pub fn pak_available() -> bool {
    let path = std::env::var("AA_PAK").unwrap_or_else(|_| aa_pak::DEFAULT_PAK.to_string());
    std::path::Path::new(&path).exists()
}
