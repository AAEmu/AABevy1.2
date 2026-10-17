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
//! The scripts take about 1.5 s to load, so with the loading screen in front ([`Screen::Loading`])
//! the stage is built ahead ([`prebuild_hud`]): a second Lua state next to the loading UI's, started
//! once the server has spawned the character. When the game starts [`build_hud`] only swaps it in
//! and raises the entering events; what changed in the meantime reaches the scripts as the usual
//! snapshot differences.
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
mod formula;
mod input;
mod marks;
mod panels;
mod plates;
mod quests;
mod session;
mod stats;
mod targeting;

use aa_net::state::Player;
use aa_ui::render::UiRenderState;
use aa_ui::{UiConfig, UiRuntime};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

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

/// The world stage's UI state built during the loading screen, waiting for the game to start
/// (a non-send resource: it holds a Lua state).
struct PrebuiltStage(UiRuntime);

/// Progress of the prebuild: frames the loading screen had to itself, and whether it was made.
#[derive(Resource, Default)]
struct PrebuildState {
    waited: u32,
    tried: bool,
}

/// Frames of the loading screen drawn (with the spawn known) before the HUD build holds the app up.
const PREBUILD_AFTER_FRAMES: u32 = 6;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        // `--enter-world` skips the login plugin, which is what normally adds the UI toolkit
        if !app.is_plugin_added::<aa_ui::UiPlugin>() {
            app.add_plugins(aa_ui::UiPlugin { config: Some(host::ui_config()) });
        }
        plates::plugin(app);
        app.insert_resource(HudOptions { enabled: !std::env::args().any(|a| a == "--no-hud") })
            .init_resource::<HudKeyboardFocus>()
            .init_resource::<HudBuilt>()
            .init_resource::<PrebuildState>()
            .init_resource::<HudPointerOverUi>()
            .insert_resource(chat::ScriptedKeys::from_args())
            .insert_resource(session::ScriptedSession::from_args())
            .insert_non_send(api::HudBridge::default())
            .add_systems(OnExit(Screen::InGame), |mut built: ResMut<HudBuilt>| *built = HudBuilt::default())
            .add_systems(OnEnter(Screen::Loading), |mut state: ResMut<PrebuildState>| *state = PrebuildState::default())
            .add_systems(Update, prebuild_hud.run_if(in_state(Screen::Loading)))
            // a stage built ahead for a game that did not start (the connection dropped while loading)
            .add_systems(OnEnter(Screen::WorldSelect), drop_prebuilt)
            .add_systems(OnEnter(Screen::CharacterSelect), drop_prebuilt)
            .add_systems(OnEnter(Screen::CharacterCreate), drop_prebuilt)
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
                    session::route_session,
                    session::scripted_session.run_if(hud_up),
                    (sync_hud, chat::route_chat, chat::chat_keys, input::hotkeys, panels::action_keys, targeting::tab_target, targeting::click_target, targeting::escape_key, api::handle_requests).run_if(hud_up),
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

/// A fresh UI runtime with the game API and the world stage's scripts loaded (not yet the runtime
/// the renderer draws), built from the current snapshot. `None` while the window has no size yet.
fn make_stage(world: &mut World, bridge: &api::Shared) -> Option<UiRuntime> {
    let (w, h) = {
        let mut q = world.query_filtered::<&Window, With<PrimaryWindow>>();
        let window = q.single(world).ok()?;
        (window.resolution.physical_width() as f32, window.resolution.physical_height() as f32)
    };
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let cfg = world.resource::<UiConfig>().clone();
    let t0 = std::time::Instant::now();
    let mut rt = match UiRuntime::new(&cfg.locale, w, h) {
        Ok(rt) => rt,
        Err(e) => {
            error!("hud: cannot create the UI runtime: {e:#}");
            return None;
        }
    };
    if let Some(db) = &cfg.ui_texts_db
        && let Err(e) = rt.load_ui_texts(db)
    {
        warn!("hud: UI texts from {}: {e:#}; LocalizeUiText returns keys", db.display());
    }
    if let Err(e) = api::install(&mut rt, bridge) {
        error!("hud: game API: {e}");
    }
    rt.run_stage(aa_ui::Stage::World);
    info!("hud: UI stage ready in {:.0} ms, {} script errors", t0.elapsed().as_secs_f32() * 1000.0, rt.errors.len());
    for (path, e) in rt.errors.iter().take(10) {
        warn!("hud: script {path}: {e}");
    }
    Some(rt)
}

/// Builds the HUD stage ahead while the loading screen is up, once the server has spawned the player.
fn prebuild_hud(world: &mut World) {
    if !world.resource::<HudOptions>().enabled || world.resource::<HudBuilt>().up || world.get_non_send::<PrebuiltStage>().is_some() {
        return;
    }
    if world.resource::<Player>().spawn.is_none() {
        return;
    }
    {
        let mut state = world.resource_mut::<PrebuildState>();
        if state.tried {
            return;
        }
        state.waited += 1;
        if state.waited < PREBUILD_AFTER_FRAMES {
            return;
        }
    }
    let bridge = world.non_send::<api::HudBridge>().0.clone();
    // the scripts read the data while they load: bring the snapshot up to date first
    api::refresh_snapshot(world, &bridge);
    match make_stage(world, &bridge) {
        Some(rt) => {
            world.resource_mut::<PrebuildState>().tried = true;
            // the stage was just built from this snapshot
            api::mark_pushed(&bridge);
            world.insert_non_send(PrebuiltStage(rt));
        }
        // no window size yet (or the runtime failed): try again next frame
        None => world.resource_mut::<PrebuildState>().waited = PREBUILD_AFTER_FRAMES - 1,
    }
}

fn drop_prebuilt(world: &mut World) {
    if world.remove_non_send::<PrebuiltStage>().is_some() {
        debug!("hud: dropped the stage built ahead");
    }
}

/// Starts the HUD: swaps in the stage built during loading, or builds one now (`--enter-world` has no
/// loading screen), then raises the events of entering the world.
fn build_hud(world: &mut World) {
    let bridge = world.non_send::<api::HudBridge>().0.clone();
    {
        let mut d = bridge.borrow_mut();
        d.leaving = None;
        d.disconnected = false;
    }
    let chat_state = world.resource::<aa_net::state::Chat>().clone();
    let t0 = std::time::Instant::now();
    if let Some(PrebuiltStage(rt)) = world.remove_non_send::<PrebuiltStage>() {
        // what changed since it was built (position, zone, bars ...) reaches the scripts through `sync_hud`
        api::refresh_snapshot(world, &bridge);
        api::enter_world(&rt, &bridge, &chat_state);
        world.insert_non_send(rt);
        // the new tree starts counting generations again: force a full redraw
        world.resource_mut::<UiRenderState>().generation = u64::MAX;
        *world.resource_mut::<HudBuilt>() = HudBuilt { up: true, fresh: true };
        info!("hud: swapped in the stage built during loading ({:.0} ms)", t0.elapsed().as_secs_f32() * 1000.0);
        return;
    }
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
