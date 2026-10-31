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
//! once the server has spawned the character and built a few milliseconds per frame
//! ([`BuildingStage`]: the scripts one by one, until the frame's budget is spent), so the loading
//! bar keeps moving. When the game starts [`build_hud`] only swaps it in and raises the entering
//! events (a stage still unfinished is finished there in one go); what changed in the meantime
//! reaches the scripts as the usual snapshot differences.
//!
//! ## Keyboard
//!
//! While an edit box has the keyboard focus (chat input) the game must not treat keys as bindings:
//! [`HudKeyboardFocus`] mirrors `aa_ui::UiStats::keyboard_focus`. The in-game controls
//! (`ingame::player::control_player`, camera, hotkeys) should skip their key handling while
//! `HudKeyboardFocus.0` is true.

mod achievement;
mod alerts;
pub(crate) use alerts::{ShowError, UseFailed};
mod auction;
mod housing;
mod battlefield;
mod breath;
mod map_overlays;
mod buffs;
pub(crate) mod api;
pub(crate) mod cameras;
mod cast;
pub(crate) mod cinema;
mod cinema_flow;
mod chat;
mod combat;
mod cooldown;
mod crafting;
mod crime;
mod data;
#[allow(unused_imports)] // used by the app's `main` target, not by the standalone examples
pub(crate) use data::{install_exp_table, item_recharge_catalog, GameData, SharedGameData};
pub(crate) mod dialog;
mod drag;
mod duel;
mod speech;
mod unit_id;
mod appellation;
mod enchant;
mod expansion;
pub(crate) use expansion::fixture::spawn as fixture_spawn;
mod sorting;
mod specialty;
mod formula;
mod family;
mod expedition;
mod territory;
mod friends;
mod input;
mod itemlinks;
mod item_effects;
mod items;
mod loot;
mod loot_roll;
mod look_conversion;
pub(crate) use look_conversion::{ItemLookSource, source_use as use_look_source};
pub(crate) use look_conversion::{PointerFinished, blocks_look_pointer};
#[cfg(test)]
pub(crate) use look_conversion::test_runtime as look_test_runtime;
#[cfg(test)]
pub(crate) use targeting::escape_key as test_escape_key;
#[cfg(test)]
pub(crate) use chat::ScriptedKeys as TestScriptedKeys;
pub(crate) fn cancel_look_target(rt: Option<&aa_ui::UiRuntime>, bridge: Option<&api::HudBridge>) {
    if let (Some(rt),Some(bridge)) = (rt,bridge) { look_conversion::cancel_target(&rt.lua,&bridge.0); }
}

/// Complete received equipment for an existing actor's visual refresh. Unknown
/// native records never fall back to the tolerant inventory projection.
pub(crate) fn received_equipment(bridge: &api::HudBridge) -> Option<aa_net::model::Equipment> {
    let data = bridge.0.borrow();
    let records = enchant::records(&data);
    if data.disconnected || records.owner() != Some(data.character_id) { return None; }
    let mut equipment = aa_net::model::Equipment::default();
    for slot in 0..28 {
        let item = records.item(1, slot)?;
        equipment.slots[usize::from(slot)] = (item.template_id != 0).then(|| item.clone());
    }
    Some(equipment)
}
mod modelview;
pub(crate) use modelview::ModelViewCamera;
mod mail;
mod marks;
mod markers;
mod map_quests;
mod map_services;
mod map_slaves;
mod notices;
mod panels;
mod party;
mod period;
mod portal;
pub(crate) mod doodad;
mod dynamic_actions;
pub(crate) use portal::{OPEN_PORTAL_SKILL, PortalBookUsed};
mod mode_actions;
mod mounted;
#[cfg(test)]
mod mounted_tests;
mod pets;
mod overhead;
mod plates;
mod pvp;
mod radar;
mod ranking;
mod repair;
mod quests;
mod ring;
pub(crate) mod script_api;
mod salon;
mod session;
mod skillbook;
pub(crate) mod slave;
mod stats;
mod store;
mod stubs;
pub(crate) mod summon;
mod targeting;
mod tipcalc;
mod tipitem;
mod tips;
mod trade;
mod trial;
mod usable;
mod user_music;
pub(crate) use user_music::MusicItemUsed;

#[allow(dead_code)]
pub(crate) fn ranking_fixture() {
    ranking::fixture::run();
}

#[allow(dead_code)]
pub(crate) fn buff_fixture() {
    buffs::cancel::fixture::run();
}

/// Run the isolated original-script combat example without a network connection.
#[allow(dead_code)]
pub(crate) fn combat_fixture() {
    combat::fixture::run();
}

#[allow(dead_code)]
pub(crate) fn mailbox_fixture() {
    mail::fixture::run();
}

#[allow(dead_code)]
pub(crate) fn loot_roll_fixture() {
    loot_roll::fixture::run();
}

#[allow(dead_code)]
pub(crate) fn trade_fixture() {
    trade::fixture::run();
}

pub(crate) fn keybindings_runtime(width: f32, height: f32) -> Result<UiRuntime, Box<dyn std::error::Error>> {
    loot_roll::fixture::runtime(width, height)
}

#[allow(dead_code)]
pub(crate) fn keybindings_fixture() {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    crate::settings::bindings_fixture(keybindings_runtime(width as f32, height as f32).unwrap());
}

#[allow(dead_code)]
pub(crate) fn pvp_fixture() {
    pvp::fixture::run();
}

#[allow(dead_code)]
pub(crate) fn breath_fixture() {
    breath::fixture::run();
}

#[allow(dead_code)]
pub(crate) fn expansion_fixture() {
    expansion::fixture::run();
}

#[allow(dead_code)]
pub(crate) fn sorting_fixture() {
    sorting::fixture::run();
}

#[allow(dead_code)]
pub(crate) fn repair_fixture() {
    repair::fixture::run();
}

#[allow(dead_code)]
pub(crate) fn item_effects_fixture() {
    item_effects::fixture::run();
}

#[allow(dead_code)]
pub(crate) fn enchant_fixture() {
    enchant::fixture::run();
}

#[allow(dead_code)]
pub(crate) fn look_conversion_fixture() {
    look_conversion::fixture();
}

#[allow(dead_code)]
pub(crate) fn modelview_fixture() { modelview::fixture(); }

#[allow(dead_code)]
pub(crate) fn specialty_fixture() {
    specialty::fixture::run();
}

#[allow(dead_code)]
pub(crate) fn crafting_fixture() {
    crafting::fixture::run();
}

#[allow(dead_code)]
pub(crate) fn user_music_fixture() { user_music::fixture::run(); }

#[allow(dead_code)]
pub(crate) fn stored_music_fixture() { user_music::cache::fixture::run();
}

#[allow(dead_code)]
pub(crate) fn mode_actions_fixture() {
    mode_actions::fixture::run(); }

#[allow(dead_code)]
pub(crate) fn stablehand_fixture() {
    pets::fixture::run();
}

#[allow(dead_code)]
pub(crate) fn owned_pet_fixture() { pets::active_fixture::run(); }

#[allow(dead_code)]
pub(crate) fn mount_walk_fixture() -> AppExit {
    pets::owned::mount_walk_fixture::run()
}

#[allow(dead_code)]
pub(crate) fn radar_fixture() { radar::fixture::run(); }

#[allow(dead_code)]
pub(crate) fn proficiency_fixture() { crafting::expert_fixture(); }

#[allow(dead_code)]
pub(crate) fn battlefield_fixture() { battlefield::fixture::run(); }

#[allow(dead_code)]
pub(crate) fn cooldown_fixture() { cooldown::fixture::run(); }

#[allow(dead_code)]
pub(crate) fn family_fixture() { family::fixture::run(); }
#[allow(dead_code)]
pub(crate) fn expedition_fixture() { expedition::fixture::run(); }

#[allow(dead_code)]
pub(crate) fn territory_fixture() { territory::fixture::run(); }

#[allow(dead_code)]
pub(crate) fn npc_fixture() { dialog::npc_fixture(); }

#[allow(dead_code)]
pub(crate) fn achievement_fixture() { achievement::fixture::run(); }

#[allow(dead_code)]
pub(crate) fn roadmap_fixture() { achievement::roadmap_fixture::run(); }

#[allow(dead_code)]
pub(crate) fn chat_config_fixture() { chat::config::fixture::run(); }

#[allow(dead_code)]
pub(crate) fn crime_fixture() { crime::fixture::run(); }

#[allow(dead_code)]
pub(crate) fn trial_fixture() { trial::fixture::run(); }

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

/// UI key capture and consumption are current before any gameplay input runs.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HudInputReady;

/// Both physical action keys and Lua requests have published their item uses.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct HudItemUse;

pub use input::HudPointerOverUi;
pub use usable::InFlight;

/// `--no-hud`: do not build the HUD (frame rate comparisons, screenshots of the bare world).
#[derive(Resource)]
pub(crate) struct HudOptions {
    pub(crate) enabled: bool,
}

/// Whether the HUD stage is up (the runtime currently is the in-game UI).
#[derive(Resource, Default)]
pub(crate) struct HudBuilt {
    pub(crate) up: bool,
    /// Built this frame: what aa-net reported until now has been replayed into the scripts.
    fresh: bool,
}

/// Mirrors [`SuspendedStage`] into a send resource. Run conditions may be evaluated on a worker
/// thread and must not touch the non-send Lua stage; see [`hud_wanted`].
#[derive(Resource, Default)]
pub(crate) struct HudSuspended(pub(crate) bool);

/// The world stage's UI state built during the loading screen, waiting for the game to start
/// (a non-send resource: it holds a Lua state).
struct PrebuiltStage(UiRuntime);

/// Progress of the prebuild: frames the loading screen had to itself, and whether it was started.
#[derive(Resource, Default)]
struct PrebuildState {
    waited: u32,
    tried: bool,
}

/// Frames of the loading screen drawn (with the spawn known) before the HUD build starts.
const PREBUILD_AFTER_FRAMES: u32 = 6;

/// The most the stage build takes of one loading frame (milliseconds); a single script that is longer
/// than what is left of it still runs whole.
const BUILD_BUDGET_MS: u64 = 8;

/// `BUILD_BUDGET_MS`, or `AA_HUD_BUDGET_MS` (a huge value builds the stage in one frame, to compare).
fn build_budget_ms() -> u64 {
    static MS: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *MS.get_or_init(|| std::env::var("AA_HUD_BUDGET_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(BUILD_BUDGET_MS))
}

/// What a [`BuildingStage`] does next.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BuildStep {
    /// The database's UI texts (cached by path, so cheap once the login screens loaded them).
    Texts,
    /// The game API and the data layer of `ingame.lua`.
    Api,
    /// The list of the stage's scripts, made on another thread (listing the pak for the stage's
    /// contents and reading the table of contents files take 20-90 ms and touch no Lua state).
    List,
    /// One script per step.
    Scripts,
    /// Flush what the scripts queued, then the stage is ready.
    Finish,
}

/// A world stage built a little at a time (a non-send resource: it holds a Lua state). This is
/// `UiRuntime::run_stage` taken apart, with the same scripts in the same order, so that the loading
/// screen is not held up for the 0.5-1.5 s the whole takes.
struct BuildingStage {
    rt: UiRuntime,
    step: BuildStep,
    texts_db: Option<std::path::PathBuf>,
    /// The scripts of the stage, as the other thread lists them.
    list: Option<std::sync::mpsc::Receiver<anyhow::Result<Vec<String>>>>,
    /// What is left to run.
    queue: std::collections::VecDeque<String>,
    started: web_time::Instant,
    frames: u32,
    /// Time spent in steps, the longest chunk of one frame and the longest single step (and what it was).
    spent: std::time::Duration,
    worst_frame: std::time::Duration,
    worst_step: (std::time::Duration, String),
}

impl BuildingStage {
    /// Runs steps until `budget` is used up (`None`: until done). True when the stage is ready.
    fn advance(&mut self, bridge: &api::Shared, budget: Option<std::time::Duration>) -> bool {
        use web_time::Instant;
        let frame = Instant::now();
        loop {
            let t = Instant::now();
            let what = match self.step {
                BuildStep::Texts => {
                    if let Some(db) = &self.texts_db
                        && let Err(e) = self.rt.load_ui_texts(db)
                    {
                        warn!("hud: UI texts from {}: {e:#}; LocalizeUiText returns keys", db.display());
                    }
                    self.step = BuildStep::Api;
                    "ui texts".to_string()
                }
                BuildStep::Api => {
                    if let Err(e) = api::install(&mut self.rt, bridge) {
                        error!("hud: game API: {e}");
                    }
                    self.step = BuildStep::List;
                    "game api".to_string()
                }
                BuildStep::List => {
                    let list = match (&self.list, budget) {
                        (Some(rx), Some(_)) => match rx.try_recv() {
                            Ok(l) => l,
                            // not ready: the next frame asks again
                            Err(std::sync::mpsc::TryRecvError::Empty) => return false,
                            Err(e) => Err(anyhow::anyhow!("{e}")),
                        },
                        (Some(rx), None) => rx.recv().unwrap_or_else(|e| Err(anyhow::anyhow!("{e}"))),
                        (None, _) => Err(anyhow::anyhow!("no script list")),
                    };
                    self.list = None;
                    self.queue.push_back(aa_lua::stage::INIT_SCRIPT.to_string());
                    match list {
                        Ok(files) => self.queue.extend(files),
                        Err(e) => {
                            warn!("hud: stage {}: {e:#}", aa_ui::Stage::World.name());
                            self.rt.errors.push((aa_ui::Stage::World.name().to_string(), format!("{e:#}")));
                        }
                    }
                    self.step = BuildStep::Scripts;
                    "script list".to_string()
                }
                BuildStep::Scripts => {
                    let path = self.queue.pop_front().unwrap_or_default();
                    if let Err(e) = aa_lua::run_alb(&self.rt.lua, &path) {
                        warn!("aa-ui: script {path}: {e:#}");
                        self.rt.errors.push((path.clone(), format!("{e:#}")));
                    }
                    if self.queue.is_empty() {
                        self.step = BuildStep::Finish;
                    }
                    path
                }
                BuildStep::Finish => {
                    aa_ui::lua::flush(&self.rt.lua);
                    self.step = BuildStep::Finish;
                    "flush".to_string()
                }
            };
            let took = t.elapsed();
            if took.as_millis() >= 5 {
                debug!("hud: stage step {what}: {:.1} ms", took.as_secs_f32() * 1000.0);
            }
            self.spent += took;
            if took > self.worst_step.0 {
                self.worst_step = (took, what.clone());
            }
            if what == "flush" {
                self.worst_frame = self.worst_frame.max(frame.elapsed());
                return true;
            }
            if budget.is_some_and(|b| frame.elapsed() >= b) {
                self.worst_frame = self.worst_frame.max(frame.elapsed());
                return false;
            }
        }
    }

    /// The finished runtime, with the numbers logged.
    fn finish(self) -> UiRuntime {
        info!(
            "hud: UI stage ready: {} frames over {:.0} ms, {:.0} ms of work, worst frame {:.1} ms, longest step {:.1} ms ({}), {} script errors",
            self.frames,
            self.started.elapsed().as_secs_f32() * 1000.0,
            self.spent.as_secs_f32() * 1000.0,
            self.worst_frame.as_secs_f32() * 1000.0,
            self.worst_step.0.as_secs_f32() * 1000.0,
            self.worst_step.1,
            self.rt.errors.len()
        );
        for (path, e) in self.rt.errors.iter().take(10) {
            warn!("hud: script {path}: {e}");
        }
        self.rt
    }
}

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        // `--enter-world` skips the login plugin, which is what normally adds the UI toolkit
        if !app.is_plugin_added::<aa_ui::UiPlugin>() {
            app.add_plugins(aa_ui::UiPlugin { config: Some(host::ui_config()) });
        }
        // the mouse paths read the script runner's pick even without `--script` (and without `ScriptPlugin`)
        app.init_resource::<crate::script::ScriptPick>();
        plates::plugin(app);
        combat::plugin(app);
        pvp::plugin(app);
        radar::plugin(app);
        battlefield::plugin(app);
        breath::plugin(app);
        expansion::plugin(app);
        repair::plugin(app);
        crafting::plugin(app);
        user_music::plugin(app);
        pets::plugin(app);
        loot::plugin(app);
        dialog::plugin(app);
        ring::plugin(app);
        markers::plugin(app);
        stubs::plugin(app);
        drag::plugin(app);
        party::plugin(app);
        friends::plugin(app);
        family::plugin(app);
        expedition::plugin(app);
        territory::plugin(app);
        achievement::plugin(app);
        chat::config::plugin(app);
        crime::plugin(app);
        duel::plugin(app);
        speech::plugin(app);
        appellation::plugin(app);
        trial::plugin(app);
        ranking::plugin(app);
        buffs::cancel::plugin(app);
        enchant::plugin(app);
        look_conversion::plugin(app);
        mail::plugin(app);
        auction::plugin(app);
        housing::plugin(app);
        loot_roll::plugin(app);
        trade::plugin(app);
        store::plugin(app);
        portal::plugin(app);
        salon::plugin(app);
        modelview::plugin(app);
        notices::plugin(app);
        cinema::plugin(app);
        doodad::plugin(app);
        app.add_systems(Update, alerts::track_death.before(sync_hud).run_if(crate::ingame::world_screens));
        app.add_systems(Update, skillbook::apply_resets.before(sync_hud).run_if(in_state(Screen::InGame)));
        app.add_systems(Update, skillbook::route_book.after(sync_hud).before(api::handle_requests).run_if(in_state(Screen::InGame)));
        app.insert_resource(HudOptions { enabled: !aa_dev::args().into_iter().any(|a| a == "--no-hud") })
            .init_resource::<HudKeyboardFocus>()
            .init_resource::<HudBuilt>()
            .init_resource::<HudSuspended>()
            .init_resource::<PrebuildState>()
            .init_resource::<HudPointerOverUi>()
            .init_resource::<InFlight>()
            .init_resource::<crate::settings::Actions>()
            .insert_resource(chat::ScriptedKeys::from_args())
            .insert_resource(alerts::ScriptedFails::from_args())
            .add_message::<alerts::UseFailed>()
            .add_message::<alerts::ShowError>()
            .add_message::<crate::ingame::combat::DamageLanded>()
            .insert_resource(session::ScriptedSession::from_args())
            .insert_resource(cameras::DevFov::from_args())
            .insert_non_send(api::HudBridge::default())
            .add_systems(First,(buffs::bind_catalog,cooldown::bind_catalog))
            .add_systems(Update,cooldown::advance_frame.before(aa_ui::UiInput).before(prebuild_hud).before(build_hud).before(sync_hud))
            .add_systems(Update,cooldown::sync_caption_option.after(aa_ui::UiInput).before(aa_ui::UiFrame))
            .add_systems(
                OnExit(Screen::InGame),
                |mut built: ResMut<HudBuilt>, mut suspended: ResMut<HudSuspended>| {
                    *built = HudBuilt::default();
                    suspended.0 = false;
                },
            )
            .add_systems(OnExit(Screen::InGame), |world: &mut World| drop(world.remove_non_send::<SuspendedStage>()))
            .add_systems(OnExit(Screen::InGame), |mut actions: ResMut<crate::settings::Actions>| *actions = default())
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
                    .in_set(cinema::CinemaInputRead)
                    .after(bevy::input::InputSystems)
                    .run_if(in_state(Screen::InGame)),
            )
            .add_systems(Update, (mirror_keyboard_focus, input::swallow_keys, crate::settings::update_input, input::hotkeys).chain().after(aa_ui::UiInput).in_set(HudInputReady).run_if(in_state(Screen::InGame)))
            .add_systems(Update, targeting::escape_key.after(HudInputReady).after(chat::chat_keys).before(api::handle_requests).run_if(in_state(Screen::InGame)))
            .add_systems(
                Update,
                (
                    build_hud.run_if(hud_wanted),
                    session::route_session,
                    session::scripted_session.run_if(hud_up),
                    (sync_hud, combat::route_combat, cast::route_casts, items::route_items, loot::fake_loot, loot::sync_loot, loot::loot_input, (dialog::sync_quests, dialog::handle_dialog, dialog::drive_npc.after(crate::ingame::teleport::detect).before(api::handle_requests).before(aa_ui::UiFrame), slave::track, dialog::talk_input).chain().after(sync_hud), chat::route_chat, chat::chat_keys, panels::action_keys.in_set(HudItemUse), targeting::tab_target, api::handle_requests.in_set(HudItemUse), alerts::show).run_if(hud_up),
                    cameras::match_world_camera,
                    cameras::dev_fov,
                )
                    .after(HudInputReady)
                    .chain()
                    .run_if(in_state(Screen::InGame)),
            );
        app.add_systems(Update, targeting::click_target.after(PointerFinished).run_if(hud_up).run_if(in_state(Screen::InGame)));
        app.add_systems(Update, slave::walk_off.after(sync_hud).before(api::handle_requests).run_if(hud_up).run_if(in_state(Screen::InGame)));
    }
}

/// In game, spawned, and the HUD stage not built yet (nor put aside for a teleport's loading screen).
///
/// This runs as a run condition, which Bevy may evaluate on a worker thread: it reads only the send
/// [`HudSuspended`] mirror, never the non-send [`SuspendedStage`] itself.
pub(crate) fn hud_wanted(built: Res<HudBuilt>, player: Res<Player>, options: Res<HudOptions>, suspended: Res<HudSuspended>) -> bool {
    options.enabled && !built.up && player.spawn.is_some() && !suspended.0
}

/// The HUD stage put aside while a teleport's loading screen is drawn instead (`ingame::teleport`): the scripts keep
/// their state, the snapshot differences reach them when the stage is back.
struct SuspendedStage(UiRuntime);

/// Puts the HUD stage aside (the caller then switches to the loading stage). False when the HUD is not up.
pub(crate) fn suspend(world: &mut World) -> bool {
    if !world.resource::<HudBuilt>().up || world.get_non_send::<SuspendedStage>().is_some() {
        return false;
    }
    let Some(rt) = world.remove_non_send::<UiRuntime>() else { return false };
    world.insert_non_send(SuspendedStage(rt));
    world.resource_mut::<HudBuilt>().up = false;
    world.resource_mut::<HudSuspended>().0 = true;
    true
}

/// Brings the HUD stage back (the loading stage drops). False when none was put aside.
pub(crate) fn resume(world: &mut World) -> bool {
    let Some(SuspendedStage(rt)) = world.remove_non_send::<SuspendedStage>() else { return false };
    // The loading runtime uploaded its own glyphs into the shared GPU atlas.
    // Restoring the old tree also has to restore its atlas, even with no new glyphs.
    aa_ui::lua::ui(&rt.lua).fonts.atlas_dirty = true;
    world.insert_non_send(rt);
    world.resource_mut::<UiRenderState>().generation = u64::MAX;
    *world.resource_mut::<HudBuilt>() = HudBuilt { up: true, fresh: false };
    world.resource_mut::<HudSuspended>().0 = false;
    true
}

fn hud_up(built: Res<HudBuilt>) -> bool {
    built.up
}

/// A fresh UI runtime (not yet the one the renderer draws), ready to be built a step at a time from
/// the current snapshot. `None` while the window has no size yet.
fn start_stage(world: &mut World) -> Option<BuildingStage> {
    let (w, h) = {
        let mut q = world.query_filtered::<&Window, With<PrimaryWindow>>();
        let window = q.single(world).ok()?;
        (window.resolution.physical_width() as f32, window.resolution.physical_height() as f32)
    };
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let cfg = world.resource::<UiConfig>().clone();
    let started = web_time::Instant::now();
    let rt = match UiRuntime::new(&cfg.locale, w, h) {
        Ok(rt) => rt,
        Err(e) => {
            error!("hud: cannot create the UI runtime: {e:#}");
            return None;
        }
    };
    let spent = started.elapsed();
    debug!("hud: stage step new runtime: {:.1} ms", spent.as_secs_f32() * 1000.0);
    let (tx, rx) = std::sync::mpsc::channel();
    crate::task::spawn("hud-stage-scripts", move || {
        let _ = tx.send(aa_ui::Stage::World.scripts());
    });
    Some(BuildingStage {
        rt,
        list: Some(rx),
        step: BuildStep::Texts,
        texts_db: cfg.ui_texts_db.clone(),
        queue: Default::default(),
        started,
        frames: 1,
        spent,
        worst_frame: spent,
        worst_step: (spent, "new runtime".to_string()),
    })
}

/// Builds the HUD stage ahead while the loading screen is up, once the server has spawned the
/// player: a few milliseconds of it per frame.
fn prebuild_hud(world: &mut World) {
    if !world.resource::<HudOptions>().enabled
        || world.resource::<HudBuilt>().up
        || world.resource::<HudSuspended>().0
        || world.get_non_send::<PrebuiltStage>().is_some()
    {
        return;
    }
    if world.resource::<Player>().spawn.is_none() {
        return;
    }
    let bridge = world.non_send::<api::HudBridge>().0.clone();
    if bridge.borrow().game.get().is_none() { return; }
    if let Some(mut building) = world.remove_non_send::<BuildingStage>() {
        building.frames += 1;
        if building.advance(&bridge, Some(std::time::Duration::from_millis(build_budget_ms()))) {
            // the stage was built from the snapshot of its first frame
            api::mark_pushed(&bridge);
            world.insert_non_send(PrebuiltStage(building.finish()));
        } else {
            world.insert_non_send(building);
        }
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
    // the scripts read the data while they load: bring the snapshot up to date first (it is not
    // touched again until the stage is done, so every script sees the same one)
    api::refresh_snapshot(world, &bridge);
    match start_stage(world) {
        Some(building) => {
            world.resource_mut::<PrebuildState>().tried = true;
            world.insert_non_send(building);
        }
        // no window size yet (or the runtime failed): try again next frame
        None => world.resource_mut::<PrebuildState>().waited = PREBUILD_AFTER_FRAMES - 1,
    }
}

fn drop_prebuilt(world: &mut World) {
    let built = world.remove_non_send::<PrebuiltStage>().is_some();
    let building = world.remove_non_send::<BuildingStage>().is_some();
    if built || building {
        debug!("hud: dropped the stage built ahead");
    }
}

/// Starts the HUD: swaps in the stage built during loading, or builds one now (`--enter-world` has no
/// loading screen), then raises the events of entering the world.
fn build_hud(world: &mut World) {
    let bridge = world.non_send::<api::HudBridge>().0.clone();
    // Native widgets query skill/item tables while loading; wait for the background DB read.
    if bridge.borrow().game.get().is_none() { return; }
    {
        let mut d = bridge.borrow_mut();
        d.leaving = None;
        d.disconnected = false;
        // A new world starts on page 1, like the rebuilt original UI (`ActionBar` resets too).
        d.action_page = 0;
    }
    let chat_state = world.resource::<aa_net::state::Chat>().clone();
    let t0 = web_time::Instant::now();
    // the loading screen ended before the stage was done: do the rest now
    if let Some(mut building) = world.remove_non_send::<BuildingStage>() {
        building.advance(&bridge, None);
        api::mark_pushed(&bridge);
        world.insert_non_send(PrebuiltStage(building.finish()));
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modelview_hud_resume_reuploads_the_retained_runtime_glyph_atlas() {
        let mut world = World::new();
        world.init_resource::<HudBuilt>();
        world.resource_mut::<HudBuilt>().up = true;
        world.init_resource::<HudSuspended>();
        let original = UiRuntime::new("en_us", 800., 600.).unwrap();
        original.exec("@restored_glyphs", "local label=UIParent:CreateWidget('label','retainedText','UIParent');label:SetExtent(200,40);label:SetText('Restored HUD text');label:Show(true)").unwrap();
        let mut state = UiRenderState::default();
        let mut images = Assets::<Image>::default();
        {
            let mut ui = aa_ui::lua::ui(&original.lua);
            assert!(!aa_ui::render::build_quads(&mut ui, None).is_empty());
            aa_ui::render::sync_atlas(&mut ui, &mut state, &mut images);
            assert!(!ui.fonts.atlas_dirty);
        }
        let atlas = state.atlas.clone().unwrap();
        let expected = images.get(&atlas).unwrap().data.clone();
        world.insert_non_send(original);
        world.insert_resource(state);
        assert!(suspend(&mut world));
        let loading = UiRuntime::new("en_us", 800., 600.).unwrap();
        let mut state = world.remove_resource::<UiRenderState>().unwrap();
        aa_ui::render::sync_atlas(&mut aa_ui::lua::ui(&loading.lua), &mut state, &mut images);
        assert_ne!(images.get(&atlas).unwrap().data, expected, "loading replaced the shared atlas");
        world.insert_resource(state);
        world.insert_non_send(loading);
        assert!(resume(&mut world));
        let mut state = world.remove_resource::<UiRenderState>().unwrap();
        let rt = world.get_non_send::<UiRuntime>().unwrap();
        let mut ui = aa_ui::lua::ui(&rt.lua);
        assert!(ui.fonts.atlas_dirty);
        aa_ui::render::sync_atlas(&mut ui, &mut state, &mut images);
        assert_eq!(images.get(&atlas).unwrap().data, expected);
        assert!(!ui.fonts.atlas_dirty);
        assert_eq!(state.generation, u64::MAX);
    }

    /// The suspended check is a run condition: it must only read send resources (Bevy evaluates
    /// conditions on the multi-threaded executor; a `NonSend` there panics when it lands on a worker).
    #[test]
    fn suspended_hud_condition_accesses_only_send_state() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(HudOptions { enabled: true })
            .insert_resource(HudBuilt::default())
            .insert_resource(HudSuspended(true))
            .insert_resource(Player { spawn: None })
            .add_systems(Update, (|| {}).run_if(hud_wanted));
        for _ in 0..3 {
            app.update();
        }
        app.world_mut().resource_mut::<HudSuspended>().0 = false;
        for _ in 0..3 {
            app.update();
        }
    }
}
