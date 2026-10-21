//! Loading screen: the original loading UI (`scriptsbin/globalui/loading`) while the server
//! spawns the character and the world around the spawn streams in, then `Screen::InGame`.
//!
//! The UI is driven by the events the client raises: `ENTERED_LOADING(worldImagePath)`,
//! `STILL_LOADING(percent)` and `LEFT_LOADING`. The percentage is real:
//!
//! * 0..25: the server's enter-game sequence (select, state, spawn) until `PlayerSpawn`;
//! * 25..60: terrain cells within [`NEAR`] metres of the spawn built by aa-world's streamer (the
//!   world is pointed at the spawn through the [`WorldAnchor`]);
//! * 60..95: the rest of the world: the GPU pipelines compiled, the object sectors in draw distance complete, the units
//!   near the spawn built (each counts the share done of the most that was outstanding at once);
//! * 95..100: everything has been complete for [`SETTLE`] seconds: all terrain within the load radius, the placement
//!   files and every object, tree and grass sector within its draw distance (no model or instance waiting, no level of
//!   detail being rebuilt), the units, the pipelines, and no image added and no terrain patch rebuilt meanwhile.
//!
//! The world is not only streamed behind this screen but drawn: as soon as the spawn is known the player and the world
//! camera are built (`ingame::player::spawn_player`, `ingame::camera::spawn_camera`; the camera renders into an image),
//! so the first-use work of the renderer (pipeline compilation, model and texture uploads, the first skinned meshes)
//! happens here instead of in the first frames of the game. At 100 the screen is captured and stays on the window as the
//! [`curtain`](super::curtain) while `Screen::InGame` brings up the HUD, the name plates and the quest marks underneath;
//! it is lifted when those are complete too. `--hold-loading PCT` pins the gauge at PCT and stays (for screenshots against
//! `reference/loading`). `docs/aa-client-loading.md` has the measurements.

use aa_net::state::Player;
use aa_ui::{Stage, UiRuntime};
use aa_world::coords::cell_origin;
use aa_world::objects::ObjectState;
use aa_world::pipelines::PipelineStatus;
use aa_world::stream::{Streamer, TerrainPatch};
use aa_world::{CELL_SIZE, TerrainQuery, cry_to_bevy};
use bevy::prelude::*;

use super::curtain::{self, Curtain};
use super::{LoginArgs, LoginBridge, WorldAnchor, gameapi, host, ui_is};
use crate::Screen;

/// Terrain within this many metres of the spawn must be built before the game starts.
const NEAR: f32 = 500.0;
/// Seconds everything must have been complete, with nothing new (objects, units, pipelines, images), before the world counts
/// as settled.
const SETTLE: f32 = 0.4;
/// Seconds the gauge stays at 100 before the game starts.
const HOLD: f32 = 0.25;
/// Give up waiting for the world after this many seconds (the game starts anyway).
const TIMEOUT: f32 = 25.0;
/// Frames the world camera must have been up before the world counts as drawn (the first frames compile pipelines).
const WARM_FRAMES: u32 = 8;

pub fn plugin(app: &mut App) {
    app.init_resource::<Loading>()
        .add_systems(OnEnter(Screen::Loading), (reset, boost_streaming))
        .add_systems(OnExit(Screen::Loading), normal_streaming)
        .add_systems(Update, progress.run_if(ui_is(Screen::Loading)))
        .add_plugins(curtain::plugin);
}

/// Build the loading UI (a fresh Lua state, like the client's third state).
pub fn build(world: &mut World) -> bool {
    let shared = world.non_send::<LoginBridge>().0.clone();
    // a teleport to another world shows that world's loading art (all game/worlds folders have one)
    let world_name = world.resource::<aa_world::WorldConfig>().world.clone();
    let image = format!("worlds/{world_name}/loading.dds");
    let image = if aa_pak::Pak::global().exists(&image) {
        image
    } else {
        format!("worlds/{}/loading.dds", crate::ingame::WORLD)
    };
    host::switch_stage(world, |rt| {
        if let Err(e) = gameapi::install(rt, &shared) {
            error!("loading: game API: {e}");
        }
        rt.run_stage(Stage::Loading);
        rt.fire_event("ENTERED_LOADING", image.as_str());
        rt.fire_event("STILL_LOADING", 0.0);
    })
}

#[derive(Resource, Default)]
struct Loading {
    /// Seconds on this screen.
    t: f32,
    /// Percent shown so far (never goes back).
    shown: f32,
    /// Seconds since the spawn is known.
    since_spawn: Option<f32>,
    /// Object entity count, units announced and pipelines requested at the last look; `stable` is for how long none of them
    /// changed while nothing else was outstanding.
    entities: usize,
    announced: usize,
    pipelines: usize,
    /// Pipelines requested when the screen came up (what the character list's prewarm got done).
    pipelines_at_start: Option<usize>,
    stable: f32,
    /// The most pipelines waiting, object sectors incomplete and units outstanding seen at once (the 60..95 part of the gauge).
    peak: [usize; 3],
    /// Seconds into the loading screen when each part was last seen unfinished (for the log: what the loading screen waited
    /// for), see [`PARTS`].
    busy_until: [f32; 8],
    /// Seconds the gauge has been at 100.
    done: f32,
    /// Terrain cells loaded / needed around the spawn (for the log).
    cells: (usize, usize),
}

fn reset(mut loading: ResMut<Loading>) {
    *loading = Loading::default();
}

/// Main thread time per frame for the world's streaming on this screen and in game (ms): behind the loading screen nobody
/// minds frames of 15 ms, in game they would show.
const LOADING_BUDGET_MS: (f32, f32) = (10.0, 4.0);

fn boost_streaming(mut cfg: ResMut<aa_world::WorldConfig>) {
    (cfg.object_budget_ms, cfg.finish_budget_ms) = LOADING_BUDGET_MS;
}

fn normal_streaming(mut cfg: ResMut<aa_world::WorldConfig>) {
    let defaults = aa_world::WorldConfig::new(&cfg.world);
    (cfg.object_budget_ms, cfg.finish_budget_ms) = (defaults.object_budget_ms, defaults.finish_budget_ms);
}

/// Cells (cell coordinates) that exist within [`NEAR`] metres of a game position.
fn near_cells(terrain: &TerrainQuery, x: f32, y: f32) -> Vec<(i32, i32)> {
    let (c0x, c0y) = (((x - NEAR) / CELL_SIZE).floor() as i32, ((y - NEAR) / CELL_SIZE).floor() as i32);
    let (c1x, c1y) = (((x + NEAR) / CELL_SIZE).floor() as i32, ((y + NEAR) / CELL_SIZE).floor() as i32);
    let mut cells = Vec::new();
    for cy in c0y..=c1y {
        for cx in c0x..=c1x {
            let o = cell_origin(cx, cy);
            let dx = (o.x - x).max(0.0).max(x - (o.x + CELL_SIZE));
            let dy = (o.y - y).max(0.0).max(y - (o.y + CELL_SIZE));
            if (dx * dx + dy * dy).sqrt() <= NEAR && terrain.cell(cx, cy).is_some() {
                cells.push((cx, cy));
            }
        }
    }
    cells
}

/// The parts [`Loading::busy_until`] follows.
const PARTS: [&str; 8] = ["near terrain", "far terrain", "camera frames", "placement", "objects", "units", "pipelines", "images / lod"];

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn progress(
    rt: NonSend<UiRuntime>,
    time: Res<Time>,
    args: Res<LoginArgs>,
    player: Res<Player>,
    dev: Res<crate::ingame::DevOptions>,
    terrain: Res<TerrainQuery>,
    streamer: Option<Res<Streamer>>,
    objects: Option<Res<ObjectState>>,
    warm: Res<crate::ingame::warm::Warm>,
    pipelines: Res<PipelineStatus>,
    mut anchor: Query<&mut Transform, With<WorldAnchor>>,
    mut loading: ResMut<Loading>,
    mut next: ResMut<NextState<Screen>>,
    (mut commands, mut curtain, lod_swaps, mut image_events): (
        Commands,
        ResMut<Curtain>,
        Query<(), (With<TerrainPatch>, Changed<Mesh3d>)>,
        MessageReader<AssetEvent<Image>>,
    ),
) {
    let dt = time.delta_secs();
    let images_added = image_events.read().any(|e| matches!(e, AssetEvent::Added { .. }));
    loading.t += dt;
    loading.pipelines_at_start.get_or_insert(pipelines.total());
    if let Some(hold) = args.hold_loading {
        rt.fire_event("STILL_LOADING", hold);
        return;
    }
    let target = match &player.spawn {
        None => 25.0 * (1.0 - (-loading.t / 1.5).exp()),
        Some(spawn) => {
            let first = loading.since_spawn.is_none();
            let since = loading.since_spawn.get_or_insert(0.0);
            *since += dt;
            let since = *since;
            let saved = spawn.position();
            let p = dev.pos.map_or(Vec3::new(saved.x, saved.y, saved.z), |p| {
                Vec3::new(p.x, p.y, if p.z.is_finite() { p.z } else { terrain.height_at(p.x, p.y).unwrap_or(saved.z) })
            });
            if first {
                info!("loading: spawned at ({:.1}, {:.1}, {:.1}); streaming the world around it", p.x, p.y, p.z);
                terrain.prefetch(p.x, p.y, NEAR + 100.0);
            }
            // the world follows the anchor: put it on the spawn
            for mut t in &mut anchor {
                t.translation = cry_to_bevy(Vec3::new(p.x, p.y, p.z));
            }
            let needed = near_cells(&terrain, p.x, p.y);
            let loaded = streamer.as_ref().map_or(0, |s| {
                let have: Vec<(i32, i32)> = s.loaded_cells().collect();
                needed.iter().filter(|c| have.contains(c)).count()
            });
            loading.cells = (loaded, needed.len());
            let terrain_done = needed.is_empty() || loaded == needed.len();
            let terrain_part = if needed.is_empty() { 1.0 } else { loaded as f32 / needed.len() as f32 };
            let stats = objects.as_ref().map(|o| o.stats.clone()).unwrap_or_default();
            let (entities, models) = (stats.entities, stats.models_pending);
            let (waiting, total) = (pipelines.waiting(), pipelines.total());
            let drawn = warm.player && warm.camera_frames >= WARM_FRAMES;
            let far_terrain = streamer.as_ref().is_some_and(|s| s.is_loading());
            let placement = objects.is_none() || stats.cells_pending > 0;
            let incomplete = stats.sectors_incomplete + stats.instances_waiting + models;
            let churn = images_added || !lod_swaps.is_empty();
            let t = loading.t;
            if (t * 4.0).floor() != ((t - dt) * 4.0).floor() {
                debug!(
                    "loading: t {:.2} workers pending {} models {} sectors incomplete {} instances waiting {} placement {} units outstanding {} waiting pipelines {} entities {} cells {}/{} far {}",
                    t,
                    aa_cry::workers::Workers::global().pending(),
                    models,
                    stats.sectors_incomplete,
                    stats.instances_waiting,
                    stats.cells_pending,
                    warm.outstanding,
                    waiting,
                    entities,
                    loaded,
                    needed.len(),
                    far_terrain
                );
            }
            let parts = [!terrain_done, far_terrain, !drawn, placement, incomplete > 0, warm.outstanding > 0, waiting > 0, churn];
            for (i, busy) in parts.into_iter().enumerate() {
                if busy {
                    loading.busy_until[i] = t;
                }
            }
            let changed = entities != loading.entities || warm.announced != loading.announced || total != loading.pipelines;
            (loading.entities, loading.announced, loading.pipelines) = (entities, warm.announced, total);
            let all_done = !parts.iter().any(|b| *b);
            if changed || !all_done {
                loading.stable = 0.0;
            } else {
                loading.stable += dt;
            }
            // the rest of the world: the share done of the most that was outstanding at once (no placement yet: none done)
            let now = [waiting, incomplete, warm.outstanding];
            let mut share = 0.0;
            for (i, n) in now.into_iter().enumerate() {
                loading.peak[i] = loading.peak[i].max(n);
                let done = if i == 1 && placement { 0.0 } else if loading.peak[i] == 0 { 1.0 } else { 1.0 - n as f32 / loading.peak[i] as f32 };
                share += done / 3.0;
            }
            let timed_out = since > TIMEOUT;
            if timed_out && loading.shown < 100.0 {
                warn!(
                    "loading: the world around the spawn is not ready after {TIMEOUT} s ({loaded}/{} cells, far terrain loading {far_terrain}, {} placement files, {} sectors incomplete, {models} models, {} units outstanding, {waiting} pipelines), entering anyway",
                    needed.len(),
                    stats.cells_pending,
                    stats.sectors_incomplete,
                    warm.outstanding
                );
            }
            if timed_out || loading.stable >= SETTLE {
                100.0
            } else if !terrain_done {
                25.0 + 35.0 * terrain_part
            } else if all_done {
                95.0 + 5.0 * (loading.stable / SETTLE).min(0.9)
            } else {
                60.0 + 35.0 * share.min(0.99)
            }
        }
    };
    let target = target.clamp(0.0, 100.0);
    if target >= loading.shown + 1.0 || (target >= 100.0 && loading.shown < 100.0) {
        loading.shown = target;
        rt.fire_event("STILL_LOADING", target as f64);
    }
    if loading.shown >= 100.0 {
        loading.done += dt;
        if loading.done <= HOLD {
            return;
        }
        // the loading screen at 100 stays on the window over the game screen until that is complete too
        if curtain.phase == curtain::Phase::Off {
            curtain::lower(&mut commands, &mut curtain);
            return;
        }
        curtain::tick(&mut curtain, dt);
        if !curtain.ready() {
            return;
        }
        rt.fire_event("LEFT_LOADING", ());
        info!(
            "loading: done after {:.1} s ({}/{} terrain cells within {NEAR} m, {} objects, {} units, {} pipelines of which {} compiled here), entering the game",
            loading.t,
            loading.cells.0,
            loading.cells.1,
            loading.entities,
            loading.announced,
            loading.pipelines,
            loading.pipelines.saturating_sub(loading.pipelines_at_start.unwrap_or(0))
        );
        let waited: Vec<String> = PARTS.iter().zip(loading.busy_until).map(|(name, t)| format!("{name} {t:.1}")).collect();
        info!("loading: waited for (s): {}", waited.join(", "));
        next.set(Screen::InGame);
    }
}
