//! The in-game HUD: the client's original Lua UI (`Stage::World`: about 900 scripts) bound to the
//! game data aa-net reports, plus the 3D side the UI toolkit does not draw (name plates).
//!
//! * [`api`]: the game API the scripts call (`X2Unit`, `X2Player`, `X2World`, `X2Bag`, ...) over a
//!   snapshot of the data ([`data::Snapshot`]), the requests the scripts make and the Lua glue
//!   (`ingame.lua`). Whatever aa-net does not provide yet falls back to the fake in-game data of
//!   `aa-ui/examples/fake/fake_ingame.lua`, marked `FALLBACK` in the log.
//! * [`data`]: the snapshot (player, zone, clock, chat, bars) and the game database lookups
//!   (names and icons of skills, items, buffs, zones, NPC templates).
//!
//! ## Flow
//!
//! Entering `Screen::InGame` builds a fresh UI stage ([`build_hud`], like the real client which
//! rebuilds its UI state on every stage change): the game API, the world stage's scripts, then the
//! events the client raises on entering the world (`ENTERED_WORLD`, `LEFT_LOADING`, the chat
//! channels joined ...). Every frame [`sync_hud`] folds aa-net's resources into the snapshot and,
//! when something changed, tells the scripts (`UNIT` queries read the snapshot; UI events such as
//! `PLAYER_HEALTH_CHANGED` or `CHAT_MESSAGE` raise the widgets that listen).
//!
//! ## Keyboard
//!
//! While an edit box has the keyboard focus (chat input) the game must not treat keys as bindings:
//! [`HudKeyboardFocus`] mirrors `aa_ui::UiStats::keyboard_focus`. The in-game controls
//! (`ingame::player::control_player`, camera, hotkeys) should skip their key handling while
//! `HudKeyboardFocus.0` is true.

mod api;
mod cameras;
mod chat;
mod data;
mod input;

use aa_net::state::Player;
use aa_ui::UiRuntime;
use bevy::prelude::*;

use crate::Screen;
use crate::login::host;

/// True while a UI edit box (the chat input) has the keyboard: the game's key bindings (movement,
/// action bar hotkeys, ...) must not fire. Updated every frame after the UI processed its input.
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct HudKeyboardFocus(pub bool);

pub use input::HudPointerOverUi;

/// `--no-hud`: do not build the HUD (frame rate comparisons, screenshots of the bare world).
#[derive(Resource)]
struct HudOptions {
    enabled: bool,
}

/// Whether the HUD stage is up (the runtime currently is the in-game UI).
#[derive(Resource, Default)]
struct HudBuilt {
    up: bool,
    /// Built this frame: what aa-net reported until now has been replayed into the scripts.
    fresh: bool,
}

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        // `--enter-world` skips the login plugin, which is what normally adds the UI toolkit
        if !app.is_plugin_added::<aa_ui::UiPlugin>() {
            app.add_plugins(aa_ui::UiPlugin { config: Some(host::ui_config()) });
        }
        app.insert_resource(HudOptions { enabled: !std::env::args().any(|a| a == "--no-hud") })
            .init_resource::<HudKeyboardFocus>()
            .init_resource::<HudBuilt>()
            .init_resource::<HudPointerOverUi>()
            .insert_resource(chat::ScriptedKeys::from_args())
            .insert_non_send(api::HudBridge::default())
            .add_systems(OnExit(Screen::InGame), |mut built: ResMut<HudBuilt>| *built = HudBuilt::default())
            // after the OS events became `ButtonInput`, before the game's systems read it
            .add_systems(
                PreUpdate,
                (input::swallow_keys, input::pointer_over_ui)
                    .chain()
                    .after(bevy::input::InputSystems)
                    .run_if(in_state(Screen::InGame)),
            )
            .add_systems(
                Update,
                (
                    build_hud.run_if(hud_wanted),
                    (sync_hud, chat::route_chat, chat::chat_keys, input::hotkeys, api::handle_requests).run_if(hud_up),
                    mirror_keyboard_focus,
                    cameras::match_world_camera,
                )
                    .chain()
                    .run_if(in_state(Screen::InGame)),
            );
    }
}

/// In game, spawned, and the HUD stage not built yet.
fn hud_wanted(built: Res<HudBuilt>, player: Res<Player>, options: Res<HudOptions>) -> bool {
    options.enabled && !built.up && player.spawn.is_some()
}

fn hud_up(built: Res<HudBuilt>) -> bool {
    built.up
}

/// Builds the HUD stage: a fresh Lua state with the game API and the world stage's scripts, then the
/// events of entering the world.
fn build_hud(world: &mut World) {
    let bridge = world.non_send::<api::HudBridge>().0.clone();
    let chat_state = world.resource::<aa_net::state::Chat>().clone();
    // the scripts read the data while they load: bring the snapshot up to date first
    api::refresh_snapshot(world, &bridge);
    let ok = host::switch_stage(world, |rt| {
        if let Err(e) = api::install(rt, &bridge) {
            error!("hud: game API: {e}");
        }
        rt.run_stage(aa_ui::Stage::World);
        api::enter_world(rt, &bridge, &chat_state);
    });
    if ok {
        *world.resource_mut::<HudBuilt>() = HudBuilt { up: true, fresh: true };
        // the stage was just built from this snapshot
        api::mark_pushed(&bridge);
    }
}

/// Folds aa-net's state into the snapshot and tells the scripts what changed.
fn sync_hud(world: &mut World) {
    let bridge = world.non_send::<api::HudBridge>().0.clone();
    api::refresh_snapshot(world, &bridge);
    if let Some(rt) = world.get_non_send::<UiRuntime>() {
        api::push_changes(rt, &bridge);
    }
}

fn mirror_keyboard_focus(stats: Res<aa_ui::UiStats>, mut focus: ResMut<HudKeyboardFocus>) {
    if focus.0 != stats.keyboard_focus {
        focus.0 = stats.keyboard_focus;
    }
}
