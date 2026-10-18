//! Original key-binding controls with isolated settings, without a network connection.
//! `cargo run -p aa-client --example keybindings_hud -- --shot shots/hud/keybindings.png --shot-after 5`
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
    Login,
    CharacterSelect,
    CharacterCreate,
    Loading,
    InGame,
}
fn main() {
    hud::keybindings_fixture();
}
