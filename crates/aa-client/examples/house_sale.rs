//! House sale, cancel, buy and demolish as two sessions (seller and buyer) over synthetic receipts, no socket.
//! The real `X2House` sale methods and `hud::housing::sync` run; every click must send one `CSSellHouse` /
//! `CSBuyHouse` / `CSSellHouseCancel` with its exact body, a refused gate none and its `ERROR_MSG` text, and the
//! receipts raise `HOUSE_SET_SELL_SUCCESS` -> `HOUSE_SALE_SUCCESS` / `HOUSE_BUY_SUCCESS` in the native order.
//!
//! `cargo run -p aa-client --example house_sale -- --mute` exits 0 when every check passes and non-zero when one
//! fails (a panic). Needs `AA_PAK` and `AA_DB`; without them it says so and exits 0.
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
    hud::house_sale_fixture();
}
