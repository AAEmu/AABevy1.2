//! Self-checking walk to the mount through the real HUD binding and game systems, with no socket.
//!
//! A mate 10 m away is mounted with `X2Mate:MountMate()`: the press starts `hud::pets::walk_to_mount`, the real
//! `ingame::player::control_player` moves the character on the scripted forward key, the distance shrinks, the walk
//! stops at the 1.2 controller's 3 m (`0x39AC3878 = 9.0`, squared) and exactly one `CSMountMate` is queued. A mate
//! 2.5 m away mounts at once: no walk, no forward key, one request. The run exits non-zero when a case is wrong.
//!
//! `cargo run -p aa-client --example mount_walk -- --headless --headless-fps 240` needs `AA_PAK` and `AA_DB`; without
//! them it says so and exits 0. Add `--mute` in a shared session (the audio device is not part of the check).
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
    hud::mount_walk_fixture();
}
