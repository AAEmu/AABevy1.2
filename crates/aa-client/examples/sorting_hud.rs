//! Original inventory sorting on explicit fixture inventory; no connection.
#![allow(dead_code)]
#[path = "../src/script/mod.rs"]
mod script;
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
#[path = "../src/task.rs"]
mod task;
#[path = "../src/lua_value.rs"]
mod lua_value;
use lua_value::lua_int;
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
    hud::sorting_fixture();
}
