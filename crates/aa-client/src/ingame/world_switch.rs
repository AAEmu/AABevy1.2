//! The world the client has loaded, and the server's instance identity seen from the game screen.
//!
//! `SCLoadInstance`'s first field is a runtime `WorldInstance` id (0 for the character's main-world
//! position), not a directory; its `zone_id` is the world template's zone key. The server builds one
//! template per `game/worlds/<name>/world.xml`, so [`aa_world::world::world_for_zone`] names the
//! folder to load. [`aa_world::WorldSwitch`] installs it behind the loading screen and
//! [`CurrentWorld`] is the name the rest of the client sees.
//!
//! A switch drops the previous world's units and doodads: they belong to the server's old view and
//! are announced again for the new world (the player itself stays, moved by `player::follow_server`).

use aa_net::state::{Doodads, Units};
use aa_net::InstanceLoad;
use bevy::prelude::*;

use super::units::{RemoteDoodad, RemoteUnit, Views};
use super::WORLD;

/// The world folder (`game/worlds/<name>`) whose data is loaded.
#[derive(Resource, Clone, Debug)]
pub struct CurrentWorld(pub String);

impl Default for CurrentWorld {
    fn default() -> Self {
        Self(WORLD.to_string())
    }
}

pub fn plugin(app: &mut App) {
    app.init_resource::<CurrentWorld>()
        // on every screen: the switch back to main_world on leaving the game happens on character select
        .add_systems(Update, track_world.after(aa_world::WorldSwitchSet))
        .add_systems(Update, clear_old_units.after(track_world));
}

/// The world an `SCLoadInstance` asks for. Unknown zones fall back to `main_world` only when the
/// packet names instance 0 (the server's `DEFAULT_INSTANCE_ID`; `return_to_main_world` sends the
/// main-world template id there).
pub fn world_for_instance(load: &InstanceLoad) -> Option<String> {
    route_instance(load, aa_world::world::world_for_zone)
}

/// [`world_for_instance`] with the zone resolver passed in (tests, tools).
pub fn route_instance(load: &InstanceLoad, resolve: impl FnOnce(u32) -> Option<String>) -> Option<String> {
    if let Some(world) = resolve(load.zone_id) {
        return Some(world);
    }
    (load.raw_id == 0).then(|| WORLD.to_string())
}

/// The client's `CurrentWorld` follows the world the renderer's config points at.
fn track_world(cfg: Res<aa_world::WorldConfig>, mut current: ResMut<CurrentWorld>) {
    if current.0 != cfg.world {
        current.0 = cfg.world.clone();
    }
}

/// After a switch: the old world's units/doodads no longer exist (the server resends the new ones).
#[allow(clippy::type_complexity)]
fn clear_old_units(
    mut commands: Commands,
    current: Res<CurrentWorld>,
    mut last: Local<Option<String>>,
    mut units: ResMut<Units>,
    mut doodads: ResMut<Doodads>,
    mut views: ResMut<Views>,
    remote: Query<Entity, Or<(With<RemoteUnit>, With<RemoteDoodad>)>>,
) {
    if last.as_deref() == Some(current.0.as_str()) {
        return;
    }
    let changed = last.is_some();
    *last = Some(current.0.clone());
    if !changed {
        return;
    }
    for entity in &remote {
        commands.entity(entity).despawn();
    }
    units.by_id.clear();
    doodads.by_id.clear();
    *views = Views::default();
    info!("world switch: dropped the previous world's units, doodads and views (now {})", current.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use aa_net::Pos;

    fn load(raw_id: u32, zone_id: u32) -> InstanceLoad {
        InstanceLoad { raw_id, zone_id, pos: Pos::new(1.0, 2.0, 3.0), angles: [0.0; 3] }
    }

    #[test]
    fn the_zone_names_the_world_and_instance_zero_falls_back_to_main() {
        let camp = route_instance(&load(7, 198), |zone| (zone == 198).then(|| "instance_training_camp".to_string()));
        assert_eq!(camp.as_deref(), Some("instance_training_camp"));
        // A recall / return to the main world: instance 0 and a main-world zone.
        let main = route_instance(&load(0, 999), |_| None);
        assert_eq!(main.as_deref(), Some(WORLD));
        // An instance id whose zone no folder owns is not guessed.
        assert_eq!(route_instance(&load(7, 999), |_| None), None);
    }

    #[test]
    fn real_sc_load_instance_zones_map_to_world_folders() {
        if !aa_world::pak_available() {
            return;
        }
        assert_eq!(world_for_instance(&load(7, 198)).as_deref(), Some("instance_training_camp"));
        assert_eq!(world_for_instance(&load(7, 278)).as_deref(), Some("instance_nachashgar"));
        assert_eq!(world_for_instance(&load(0, 142)).as_deref(), Some("main_world"));
        assert_eq!(world_for_instance(&load(0, 0)).as_deref(), Some(WORLD));
    }

    #[test]
    fn a_minimal_app_switches_and_drops_the_old_world_entities() {
        let mut app = App::new();
        app.insert_resource(aa_world::WorldConfig::new("main_world"))
            .init_resource::<aa_world::WorldSwitch>()
            .insert_resource(aa_world::stream::WorldInfo(std::sync::Arc::new(aa_world::World::parse_g(
                "name main_world\nzones\n zone ( name main, id 142 )\n",
            ))));
        let old_cell = app.world_mut().spawn(aa_world::stream::CellRoot { cell: (0, 0) }).id();
        app.world_mut().resource_mut::<aa_world::WorldSwitch>().request("instance_next");
        assert_eq!(app.world().resource::<aa_world::WorldSwitch>().pending(), Some("instance_next"));
        assert!(aa_world::switch_world_data(
            app.world_mut(),
            "instance_next",
            aa_world::World::parse_g("name instance_next\nzones\n zone ( name instance_next, id 198 )\n"),
        ));
        assert!(app.world().get_entity(old_cell).is_err(), "the old world's cell root is gone");
        assert_eq!(app.world().resource::<aa_world::WorldConfig>().world, "instance_next");
        assert_eq!(app.world().resource::<aa_world::stream::WorldInfo>().0.zones[0].id, 198);
    }

    #[test]
    fn a_world_change_despawns_remote_units_and_clears_the_views() {
        let mut app = App::new();
        app.init_resource::<CurrentWorld>()
            .init_resource::<Units>()
            .init_resource::<Doodads>()
            .init_resource::<Views>()
            .add_systems(Update, clear_old_units);
        app.update();
        let remote = app.world_mut().spawn(RemoteDoodad { obj_id: 7 }).id();
        app.world_mut().resource_mut::<CurrentWorld>().0 = "instance_training_camp".into();
        app.update();
        assert!(app.world().get_entity(remote).is_err(), "the old world's doodad is gone");
    }
}
