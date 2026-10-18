//! Original loot dice scripts on isolated fixture state, without a server.
//! `cargo run -p aa-client --example loot_dice_hud -- --shot shots/hud/loot-dice.png --shot-after 5`

#![allow(dead_code)]

#[path = "../src/hud/mod.rs"]
mod hud;
#[path = "../src/ingame/mod.rs"]
mod ingame;
#[path = "../src/login/mod.rs"]
mod login;
#[path = "../src/settings.rs"]
mod settings;
#[path = "../src/sound/mod.rs"]
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
    hud::loot_roll_fixture();
}
