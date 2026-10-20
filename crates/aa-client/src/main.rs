//! ArcheAge 1.2 client remake. Screen flow (mirrors the real client's login stages):
//! world select -> character select -> character create -> loading -> in game.
//!
//! Each screen is one module with its own plugin, active while `Screen` is in its state:
//! * [`login`]: world select, character select, character create and loading, on the original
//!   Lua UI
//! * [`ingame`]: world, player, units, camera, movement (`--enter-world` skips the login stage)
//! * [`hud`]: the in-game Lua UI on real data, name plates
//! * [`sound`]: `aa-audio` wired to the screens (music, UI sounds, zone music, volumes)
//! * [`script`]: `--script file.txt`, a line-based script that drives the client (with `--headless`: no window)
//! * [`priority`]: the process runs above normal priority (`AA_PRIORITY=normal` keeps the default)

mod hud;
mod ingame;
mod login;
mod priority;
mod script;
mod settings;
mod sound;

use bevy::prelude::*;

#[derive(States, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Screen {
    /// The custom login screen (`login::login_screen`); the first screen unless a dev flag skips it.
    #[default]
    Login,
    WorldSelect,
    CharacterSelect,
    CharacterCreate,
    Loading,
    InGame,
}

fn main() -> AppExit {
    let mut app = App::new();
    // the login screen's art folder is an asset source: registered before the asset plugin
    login::login_screen::register_assets(&mut app);
    app
        // a command on an entity that went in the same frame (screens close while models still come in) is
        // logged, not a crash
        .set_error_handler(bevy::ecs::error::warn)
        .add_plugins((aa_dev::default_plugins("AAEMU 1.2"), aa_dev::DevPlugin))
        .add_plugins(aa_fx::FxPlugin)
        .add_plugins((aa_audio::AaAudioPlugin::default(), sound::SoundPlugin))
        .insert_state(login::login_screen::initial_screen())
        .insert_resource(aa_net::ClientOptions {
            item_recharge_catalog: hud::item_recharge_catalog(),
            ..Default::default()
        })
        .add_plugins((login::LoginPlugin, ingame::InGamePlugin, hud::HudPlugin, script::ScriptPlugin))
        // the game's threads before builds and other programs when the CPU is full (`priority`)
        .add_systems(Startup, (priority::raise, present_mode, hud::install_exp_table))
        .run()
}

/// `AA_PRESENT=vsync|novsync|fifo|mailbox|immediate`: the window's present mode.
fn present_mode(mut window: Query<&mut bevy::window::Window, With<bevy::window::PrimaryWindow>>) {
    use bevy::window::PresentMode;
    let Ok(v) = std::env::var("AA_PRESENT") else { return };
    let mode = match v.as_str() {
        "vsync" => PresentMode::AutoVsync,
        "novsync" => PresentMode::AutoNoVsync,
        "fifo" => PresentMode::Fifo,
        "mailbox" => PresentMode::Mailbox,
        "immediate" => PresentMode::Immediate,
        _ => return,
    };
    if let Ok(mut w) = window.single_mut() {
        w.present_mode = mode;
    }
}
