//! ArcheAge 1.2 client remake. Screen flow (mirrors the real client's login stages):
//! world select -> character select -> character create -> loading -> in game.
//!
//! Each screen is one module with its own plugin, active while `Screen` is in its state:
//! * [`login`]: world select, character select, character create and loading, on the original
//!   Lua UI
//! * [`ingame`]: world, player, units, camera, movement (`--enter-world` skips the login stage)
//! * [`hud`]: the in-game Lua UI on real data, name plates
//! * [`sound`]: `aa-audio` wired to the screens (music, UI sounds, zone music, volumes)
//! * [`priority`]: the process runs above normal priority (`AA_PRIORITY=normal` keeps the default)

mod hud;
mod ingame;
mod login;
mod priority;
mod sound;

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
        // a command on an entity that went in the same frame (screens close while models still come in) is
        // logged, not a crash
        .set_error_handler(bevy::ecs::error::warn)
        .add_plugins((aa_dev::default_plugins("ArcheAge"), aa_dev::DevPlugin))
        .add_plugins(aa_fx::FxPlugin)
        .add_plugins((aa_audio::AaAudioPlugin::default(), sound::SoundPlugin))
        .init_state::<Screen>()
        .add_plugins((login::LoginPlugin, ingame::InGamePlugin, hud::HudPlugin))
        // the game's threads before builds and other programs when the CPU is full (`priority`)
        .add_systems(Startup, priority::raise)
        .run();
}
