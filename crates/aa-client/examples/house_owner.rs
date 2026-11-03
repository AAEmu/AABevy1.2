//! The original housing owner menu (`housing_manager.alb` + `maintain_window.alb`) over synthetic
//! `SCMyHouse` / `SCHouseTaxInfo` events: no socket. Asserts the interaction/tax argument tuples, the permission
//! packet, `GetHouseDecoCount` and the paid/due/overdue texts.
//!
//! `cargo run -p aa-client --example house_owner -- --state paid --shot shots/housing-owner/paid.png`
//! (headless check: `--headless --mute --state paid --exit-after 2`).
#![allow(dead_code)]

#[path = "../src/hud/mod.rs"]
mod hud;
#[path = "../src/ingame/mod.rs"]
mod ingame;
#[path = "../src/login/mod.rs"]
mod login;
#[path = "../src/lua_value.rs"]
mod lua_value;
#[path = "../src/script/mod.rs"]
mod script;
#[path = "../src/skilltrace/mod.rs"]
mod skilltrace;
#[path = "../src/settings.rs"]
mod settings;
#[path = "../src/sound/mod.rs"]
mod sound;
#[path = "../src/task.rs"]
mod task;
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
    hud::house_owner_fixture();
}
