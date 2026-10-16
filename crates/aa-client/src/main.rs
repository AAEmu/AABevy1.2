//! ArcheAge 1.2 client remake. Screen flow (mirrors the real client's login stages):
//! world select -> character select -> character create -> loading -> in game.
//!
//! Each screen is one module with its own plugin, active while `Screen` is in its state:
//! * `ingame`: world, player, units, camera, movement (entered via `--enter-world` until the
//!   login-stage screens are wired up)
//! * login stage screens (world select, character select/create) and loading: to come

mod ingame;

use bevy::prelude::*;

#[derive(States, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Screen {
    #[default]
    WorldSelect,
    CharacterSelect,
    CharacterCreate,
    Loading,
    InGame,
}

fn main() {
    App::new()
        .add_plugins((aa_dev::default_plugins("ArcheAge"), aa_dev::DevPlugin))
        .init_state::<Screen>()
        .add_plugins(ingame::InGamePlugin)
        .run();
}
