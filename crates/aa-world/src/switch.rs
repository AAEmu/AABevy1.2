//! Switching the loaded world at runtime.
//!
//! `SCLoadInstance` names a zone key, not a directory. [`world_for_zone`](crate::world::world_for_zone)
//! maps it to `game/worlds/<name>`; [`WorldSwitch::request`] queues that name and
//! [`apply_world_switch`] replaces the world's data before [`stream_cells`](crate::stream::stream_cells)
//! starts loading cells for the new one. The loading screen runs on the new world's
//! [`TerrainQuery`], [`Streamer`] and [`ObjectState`], so it waits for the destination exactly as it
//! does when entering the game.
//!
//! What is torn down: every terrain cell (`CellRoot`), object sector, placed water / road / decal /
//! particle / sound / light / fog volume / voxel of the previous world, the cell and placement caches,
//! the terrain/water/collision queries and the terrain layer library. Sky, sun and post effects are the
//! renderer's and stay; its time-of-day cache follows [`WorldConfig::world`] by itself.

use std::sync::Arc;

use bevy::prelude::*;

use crate::collide::WorldCollision;
use crate::objects::ObjectState;
use crate::stream::{CellRoot, Streamer, WorldConfig, WorldInfo};
use crate::terrain_layers::TerrainLayers;
use crate::terrain_query::TerrainQuery;
use crate::water_query::WaterQuery;
use crate::world::World as WorldData;

/// A pending runtime world change. [`WorldSwitch::request`] only queues it; the world's data is
/// replaced by the plugin's update system (or by [`switch_world`]) before the next cell streaming.
#[derive(Resource, Default, Debug, Clone)]
pub struct WorldSwitch {
    pending: Option<String>,
}

impl WorldSwitch {
    pub fn request(&mut self, world: impl Into<String>) {
        self.pending = Some(world.into());
    }

    pub fn pending(&self) -> Option<&str> {
        self.pending.as_deref()
    }

    pub fn take(&mut self) -> Option<String> {
        self.pending.take()
    }
}

/// The systems that install a requested world. They run before the cell streamer.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct WorldSwitchSet;

pub(crate) fn build(app: &mut App) {
    app.init_resource::<WorldSwitch>().add_systems(
        Update,
        apply_world_switch
            .in_set(WorldSwitchSet)
            .before(crate::stream::stream_cells),
    );
}

fn apply_world_switch(world: &mut World) {
    let Some(target) = world.resource_mut::<WorldSwitch>().take() else { return };
    if !switch_world(world, &target) {
        let current = world.resource::<WorldConfig>().world.clone();
        warn!("aa-world: cannot switch to world {target}; staying in {current}");
    }
}

/// Loads `target` from the pak and installs it. `false` (and nothing changed) when its `world.g`
/// cannot be read.
pub fn switch_world(world: &mut World, target: &str) -> bool {
    if world.resource::<WorldConfig>().world == target {
        return true;
    }
    let data = match WorldData::load(target) {
        Ok(data) => data,
        Err(e) => {
            warn!("aa-world: cannot read world {target}: {e}");
            return false;
        }
    };
    if !switch_world_data(world, target, data) {
        return false;
    }
    // the sector zones come from the 4.6 MB world.xml: parsed on a thread like at startup (zones fall back to world.g
    // cells until it is in)
    let info = world.resource::<WorldInfo>().0.clone();
    if let Some(mut cache) = world.get_resource_mut::<crate::sky::TodCache>() {
        crate::sky::start_sector_zones(&mut cache, &info);
    }
    true
}

/// Installs already parsed world data as `target` (tests, tools). Everything spawned from the old
/// world's data is despawned first; `data.name` is set to the folder name.
pub fn switch_world_data(world: &mut World, target: &str, mut data: WorldData) -> bool {
    if world.resource::<WorldConfig>().world == target {
        return true;
    }
    data.name = target.to_string();

    // Terrain and object roots. The object state below owns its sector roots too; despawning a
    // CellRoot takes the terrain patches with it.
    let roots: Vec<Entity> = {
        let mut query = world.query_filtered::<Entity, With<CellRoot>>();
        query.iter(world).collect()
    };
    for entity in roots {
        if world.get_entity(entity).is_ok() {
            world.despawn(entity);
        }
    }

    let mut objects = world.remove_resource::<ObjectState>().unwrap_or_default();
    objects.unload_all(world);
    world.insert_resource(ObjectState::new());

    if let Some(mut streamer) = world.get_resource_mut::<Streamer>() {
        streamer.reset();
    }
    let terrain = TerrainQuery::new(target);
    world.insert_resource(terrain.clone());
    world.insert_resource(WaterQuery::with_terrain(target, terrain.clone()));
    world.insert_resource(WorldCollision::new(target, terrain));
    world.insert_resource(WorldInfo(Arc::new(data)));
    if let Some(mut layers) = world.get_resource_mut::<TerrainLayers>() {
        layers.reset();
    }
    world.resource_mut::<WorldConfig>().world = target.to_string();
    info!("aa-world: switched to world {target}");
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World as WorldData;
    use bevy::ecs::world::World as BevyWorld;

    fn bevy_world() -> BevyWorld {
        let mut world = BevyWorld::new();
        world.insert_resource(WorldConfig::new("main_world"));
        world.insert_resource(WorldInfo(Arc::new(WorldData::load("main_world").unwrap_or_else(|_| {
            WorldData::parse_g("name main_world\ncellX 1\ncellY 1\nzones\n zone ( name old, id 1 )\n")
        }))));
        world.init_resource::<WorldSwitch>();
        world.init_resource::<ObjectState>();
        world.init_resource::<TerrainLayers>();
        world.insert_resource(TerrainQuery::new("main_world"));
        world.insert_resource(WaterQuery::new("main_world"));
        world.insert_resource(WorldCollision::new("main_world", TerrainQuery::new("main_world")));
        world
    }

    fn next_world() -> WorldData {
        WorldData::parse_g("name next_world\ncellX 1\ncellY 1\nzones\n zone ( name instance_next, id 198 )\n")
    }

    #[test]
    fn switch_despawns_the_old_cells_and_installs_the_new_world_data() {
        let mut world = bevy_world();
        let old_cell = world.spawn(CellRoot { cell: (0, 0) }).id();
        assert!(switch_world_data(&mut world, "next_world", next_world()));
        assert!(world.get_entity(old_cell).is_err(), "the old cell root is gone");
        assert_eq!(world.resource::<WorldConfig>().world, "next_world");
        assert_eq!(world.resource::<WorldInfo>().0.name, "next_world");
        assert_eq!(world.resource::<WorldInfo>().0.zones[0].id, 198);
        assert_eq!(world.resource::<TerrainQuery>().world(), "next_world");
        assert_eq!(world.resource::<WaterQuery>().world(), "next_world");
        assert_eq!(world.resource::<WorldCollision>().world(), "next_world");
    }

    #[test]
    fn a_requested_world_is_taken_by_the_plugin_system() {
        let mut world = bevy_world();
        world.resource_mut::<WorldSwitch>().request("next_world");
        assert_eq!(world.resource::<WorldSwitch>().pending(), Some("next_world"));
        assert_eq!(world.resource_mut::<WorldSwitch>().take().as_deref(), Some("next_world"));
        assert_eq!(world.resource::<WorldSwitch>().pending(), None);
    }

    #[test]
    fn switching_to_the_current_world_changes_nothing() {
        let mut world = bevy_world();
        let cell = world.spawn(CellRoot { cell: (0, 0) }).id();
        assert!(switch_world_data(&mut world, "main_world", next_world()));
        assert!(world.get_entity(cell).is_ok());
        assert_eq!(world.resource::<WorldInfo>().0.name, "main_world");
    }
}
