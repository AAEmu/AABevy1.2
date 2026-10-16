//! The login stage: the original Lua UI screens backed by the server and the 3D characters.
//!
//! * [`world_select`]: server list (`X2World` from aa-net), connect
//! * [`character_select`]: character list (`X2LoginCharacter`), 3D character, start / delete
//! * [`create`]: character creation (race, appearance with live preview, skillset and name)
//! * [`loading`]: the loading screen while the world streams in, then `Screen::InGame`

pub mod character_select;
pub mod create;
pub mod loading;
pub mod stage3d;
pub mod world_select;

use bevy::prelude::*;

pub struct LoginPlugin;

impl Plugin for LoginPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            world_select::plugin,
            character_select::plugin,
            create::plugin,
            loading::plugin,
            |app: &mut App| {
                app.add_plugins(stage3d::Stage3dPlugin);
            },
        ));
    }
}
