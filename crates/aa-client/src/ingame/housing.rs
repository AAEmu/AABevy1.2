//! Housing in the world: the houses the server announces ([`render`]) and the builder that places a new one
//! ([`placement`]). The house windows (`X2House`) are the HUD's (`hud/housing`). See `docs/aa-client-housing.md`.
//!
//! * [`data`]: designs, construction stage models, design items and the housing land polygons, loaded once;
//! * [`prefab`]: the models of a house prefab;
//! * [`render`]: house units drawn at the model of their construction stage;
//! * [`placement`]: the builder (ghost, rotation, checks, `CSConstructHouseTax` / `CSCreateHouse`).

pub mod data;
pub mod placement;
pub mod prefab;
pub mod render;

use bevy::prelude::*;

use crate::Screen;

pub use placement::BuilderSignal;

/// Using this item enters the builder, it is not cast (`ground_target::route` skips it).
pub fn claims_item(item_template: u32) -> bool {
    data::get().is_some_and(|d| d.design_by_item.contains_key(&item_template))
}

pub fn plugin(app: &mut App) {
    data::load_in_background(super::WORLD);
    app.add_message::<BuilderSignal>()
        .init_resource::<placement::Builder>()
        .init_resource::<placement::GhostJobs>()
        .init_resource::<render::DrawnHouses>()
        .add_systems(PreUpdate, placement::input.in_set(crate::hud::cinema::CinemaInputRead).after(bevy::input::InputSystems).run_if(in_state(Screen::InGame)))
        .add_systems(Update, (placement::start, placement::follow, placement::ghost).chain().run_if(in_state(Screen::InGame)))
        .add_systems(Update, render::sync_houses.run_if(super::world_screens))
        .add_systems(OnExit(Screen::InGame), (render::clear, placement::clear));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bevy refuses a system whose parameters conflict (B0002) when it first runs: initialise every housing system.
    #[test]
    fn the_systems_have_no_conflicting_parameters() {
        fn init<M>(world: &mut World, system: impl IntoSystem<(), (), M>) {
            let mut system = IntoSystem::into_system(system);
            system.initialize(world);
        }
        let mut world = World::new();
        init(&mut world, placement::input);
        init(&mut world, placement::start);
        init(&mut world, placement::follow);
        init(&mut world, placement::ghost);
        init(&mut world, placement::clear);
        init(&mut world, render::sync_houses);
        init(&mut world, render::clear);
    }
}
