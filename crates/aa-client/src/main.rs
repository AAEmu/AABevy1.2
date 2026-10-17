//! ArcheAge 1.2 client remake. Screen flow (mirrors the real client's login stages):
//! world select -> character select -> character create -> loading -> in game.
//!
//! Each screen is one module with its own plugin, active while `Screen` is in its state:
//! * [`login`]: world select, character select, character create and loading, on the original
//!   Lua UI
//! * [`ingame`]: world, player, units, camera, movement (`--enter-world` skips the login stage)
//! * [`hud`]: the in-game Lua UI on real data, name plates

mod hud;
mod ingame;
mod login;

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
        .add_plugins((login::LoginPlugin, ingame::InGamePlugin, hud::HudPlugin))
        .run();
}
