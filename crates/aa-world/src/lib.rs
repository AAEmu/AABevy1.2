//! Worlds of the ArcheAge 1.2 client: terrain, cell objects, vegetation, water, sky / time of day, streaming.
//!
//! See `docs/aa-world.md` for the formats, coordinates, API and performance numbers.

pub mod cinema;
pub mod coords;
pub mod cover;
pub mod env;
pub mod heightmap;
pub mod objects;
pub mod pipelines;
pub mod placement;
pub mod post;
pub mod profile;
pub mod reader;
pub mod sky;
pub mod sky_clouds;
pub mod sky_material;
pub mod stream;
pub mod terrain;
pub mod terrain_layers;
pub mod terrain_query;
pub mod vegetation;
pub mod water;
pub mod water_material;
pub mod world;

pub use coords::{CELL_SIZE, bevy_to_cry, cry_to_bevy};
pub use sky::{AmbientCubes, WorldClock};
pub use stream::{AaWorldPlugin, WorldConfig, WorldFocus, WorldTimings};
pub use terrain_query::{Ground, TerrainQuery};
pub use world::World;

/// Whether the real client pak can be opened (tests that need it skip otherwise).
pub fn pak_available() -> bool {
    let path = std::env::var("AA_PAK").unwrap_or_else(|_| aa_pak::DEFAULT_PAK.to_string());
    std::path::Path::new(&path).exists()
}
