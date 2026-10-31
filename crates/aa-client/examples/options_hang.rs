//! Regression: opening the original Options window must not stall the main thread.
//! `cargo run -p aa-client --example options_hang -- --headless --mute`
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
use bevy::prelude::*;
use lua_value::lua_int;
use std::time::{Duration, Instant};
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
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let rt = hud::options_hang_runtime(width as f32, height as f32).expect("real world-stage HUD");
    // The report's HUD path: open_config raises the system menu first.
    rt.hotkey("open_config").expect("open_config");
    aa_ui::lua::tick(&rt.lua, 16.0);
    let t = Instant::now();
    rt.lua
        .load("ToggleOptionFrame(true)")
        .exec()
        .expect("open Options");
    let open = t.elapsed();
    let t = Instant::now();
    aa_ui::lua::tick(&rt.lua, 16.0);
    let next_tick = t.elapsed();
    let total = open + next_tick;
    eprintln!(
        "options_hang: open={open:?} next_tick={next_tick:?} total={total:?} limit={:?}",
        Duration::from_secs(1)
    );
    if total > Duration::from_secs(1) {
        eprintln!("options_hang: FAIL, the Options open and next tick took {total:?}");
        std::process::exit(1);
    }
}
