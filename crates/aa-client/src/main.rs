//! ArcheAge 1.2 client remake. Screen flow (mirrors the real client's login stages):
//! world select -> character select -> character create -> loading -> in game.
//!
//! Each screen is one module with its own plugin, active while `Screen` is in its state:
//! * [`login`]: world select, character select, character create and loading, on the original
//!   Lua UI
//! * [`ingame`]: world, player, units, camera, movement (`--enter-world` skips the login stage)
//! * [`hud`]: the in-game Lua UI on real data, name plates
//! * [`sound`]: `aa-audio` wired to the screens (music, UI sounds, zone music, volumes)
//! * [`graphics`]: the Options window's screen and quality pages applied to the window and the renderer
//! * [`script`]: `--script file.txt`, a line-based script that drives the client (with `--headless`: no window)
//! * [`priority`]: the process runs above normal priority (`AA_PRIORITY=normal` keeps the default)

mod graphics;
mod hud;
mod ingame;
mod logging;
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
    // open the log file and hook panics before the log plugin is built (`AA_LOG_FILE=off` turns it off)
    logging::init();
    let mut app = App::new();
    // the login screen's art folder is an asset source: registered before the asset plugin
    login::login_screen::register_assets(&mut app);
    app
        // a command on an entity that went in the same frame (screens close while models still come in) is
        // logged, not a crash
        .set_error_handler(bevy::ecs::error::warn)
        .add_plugins((aa_dev::default_plugins_with_log("AAEMU 1.2", logging::layer), aa_dev::DevPlugin))
        .add_plugins(aa_fx::FxPlugin)
        // Audited constant local Wind bursts affect other FX roots; vegetation has
        // a separate native cache boundary and is not connected here.
        .insert_resource(aa_fx::wind::FxWindSettings { enabled: true, max_fields: 64 })
        .add_plugins((aa_audio::AaAudioPlugin::default(), sound::SoundPlugin))
        .insert_state(login::login_screen::initial_screen())
        .insert_resource(aa_net::ClientOptions {
            item_recharge_catalog: hud::item_recharge_catalog(),
            ..Default::default()
        })
        .add_plugins((login::LoginPlugin, ingame::InGamePlugin, hud::HudPlugin, script::ScriptPlugin, graphics::GraphicsPlugin))
        // the game's threads before builds and other programs when the CPU is full (`priority`)
        .add_systems(Startup, (priority::raise, present_mode, hud::install_exp_table));
    logging::startup_line();
    app.run()
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
