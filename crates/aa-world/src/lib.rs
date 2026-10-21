//! Worlds of the ArcheAge 1.2 client: terrain, cell objects, vegetation, water, sky / time of day, streaming.
//!
//! See `docs/aa-world.md` for the formats, coordinates, API and performance numbers.

pub mod char_light;
pub mod area_tod;
pub mod cinema;
pub mod collide;
pub mod coords;
mod decals;
mod decal_projection;
pub mod cover;
pub mod env;
pub mod fog;
pub mod entities;
pub mod heightmap;
pub mod moons;
pub mod night_sky;
pub mod objects;
pub mod pipelines;
pub mod placement;
pub mod post;
mod post_adapt;
mod post_bloom;
mod post_shafts;
pub mod profile;
pub mod reader;
mod roads;
pub mod sky;
pub mod sky_clouds;
pub mod sky_material;
pub mod sky_transition;
pub mod stream;
pub mod subzone;
pub mod switch;
pub mod terrain;
pub mod terrain_layers;
pub mod terrain_query;
pub mod vegetation;
pub mod vegetation_color;
pub mod vegetation_wind;
pub mod water;
pub mod water_material;
pub mod water_query;
pub mod world;

pub use collide::WorldCollision;
pub use area_tod::{AreaToDState, AreaToDSubject};
pub use coords::{CELL_SIZE, bevy_to_cry, cry_to_bevy};
pub use sky::{AmbientCubes, WorldClock};
pub use stream::{AaWorldPlugin, WorldConfig, WorldFocus, WorldTimings};
pub use switch::{switch_world, switch_world_data, WorldSwitch, WorldSwitchSet};
pub use terrain_query::{Ground, TerrainQuery};
pub use water_query::{WaterBody, WaterKind, WaterQuery};
pub use world::World;

/// Whether the real client pak can be opened (tests that need it skip otherwise).
pub fn pak_available() -> bool {
    let path = std::env::var("AA_PAK").unwrap_or_else(|_| aa_pak::DEFAULT_PAK.to_string());
    std::path::Path::new(&path).exists()
}
