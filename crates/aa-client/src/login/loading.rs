//! Loading screen: the original loading UI (`scriptsbin/globalui/loading`) while the server
//! spawns the character and the world around the spawn streams in, then `Screen::InGame`.
//!
//! The UI is driven by the events the client raises: `ENTERED_LOADING(worldImagePath)`,
//! `STILL_LOADING(percent)` and `LEFT_LOADING`. The percentage is real:
//!
//! * 0..25: the server's enter-game sequence (select, state, spawn) until `PlayerSpawn`;
//! * 25..65: terrain cells within [`NEAR`] metres of the spawn built by aa-world's streamer (the
//!   world is pointed at the spawn through the [`WorldAnchor`]);
//! * 65..100: the world has settled: the objects and vegetation around it have stopped appearing, the player and the
//!   units near it are built, and the GPU pipelines are compiled.
//!
//! The world is not only streamed behind this screen but drawn: as soon as the spawn is known the player and the world
//! camera are built (`ingame::player::spawn_player`, `ingame::camera::spawn_camera`; the camera renders into an image),
//! so the first-use work of the renderer (pipeline compilation, model and texture uploads, the first skinned meshes)
//! happens here instead of in the first frames of the game. Then the screen switches to `Screen::InGame`, where the
//! camera is put on the window. `--hold-loading PCT` pins the gauge at PCT and stays (for screenshots against
//! `reference/loading`).

use aa_net::state::Player;
use aa_ui::{Stage, UiRuntime};
use aa_world::coords::cell_origin;
use aa_world::objects::ObjectState;
use aa_world::pipelines::PipelineStatus;
use aa_world::stream::Streamer;
use aa_world::{CELL_SIZE, TerrainQuery, cry_to_bevy};
use bevy::prelude::*;

use super::{LoginArgs, LoginBridge, WorldAnchor, gameapi, host, ui_is};
use crate::Screen;

/// Terrain within this many metres of the spawn must be built before the game starts.
const NEAR: f32 = 500.0;
/// Seconds without anything new (objects, units, pipelines) after which the world counts as settled.
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
        .add_systems(Update, progress.run_if(ui_is(Screen::Loading)));
}

/// Build the loading UI (a fresh Lua state, like the client's third state).
pub fn build(world: &mut World) -> bool {
    let shared = world.non_send::<LoginBridge>().0.clone();
    let image = format!("worlds/{}/loading.dds", crate::ingame::WORLD);
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
    stable: f32,
    /// Seconds into the loading screen when each part was last seen unfinished: terrain, camera frames, models, units,
    /// pipelines (for the log: what the loading screen waited for).
    busy_until: [f32; 5],
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

#[allow(clippy::too_many_arguments)]
fn progress(
    rt: NonSend<UiRuntime>,
    time: Res<Time>,
    args: Res<LoginArgs>,
    player: Res<Player>,
    terrain: Res<TerrainQuery>,
    streamer: Option<Res<Streamer>>,
    objects: Option<Res<ObjectState>>,
    warm: Res<crate::ingame::warm::Warm>,
    pipelines: Res<PipelineStatus>,
    mut anchor: Query<&mut Transform, With<WorldAnchor>>,
    mut loading: ResMut<Loading>,
    mut next: ResMut<NextState<Screen>>,
) {
    let dt = time.delta_secs();
    loading.t += dt;
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
            let p = spawn.position();
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
            let (entities, models) = objects.as_ref().map_or((0, 0), |o| (o.stats.entities, o.stats.models_pending));
            let (waiting, total) = (pipelines.waiting(), pipelines.total());
            let drawn = warm.player && warm.camera_frames >= WARM_FRAMES;
            let t = loading.t;
            if (t * 4.0).floor() != ((t - dt) * 4.0).floor() {
                debug!("loading: t {:.2} workers pending {} models {} units outstanding {} waiting pipelines {} entities {} cells {}/{}", t, aa_cry::workers::Workers::global().pending(), models, warm.outstanding, waiting, entities, loaded, needed.len());
            }
            for (i, busy) in [!terrain_done, !drawn, models > 0, warm.outstanding > 0, waiting > 0].into_iter().enumerate() {
                if busy {
                    loading.busy_until[i] = t;
                }
            }
            let changed = entities != loading.entities || warm.announced != loading.announced || total != loading.pipelines;
            (loading.entities, loading.announced, loading.pipelines) = (entities, warm.announced, total);
            if changed || !terrain_done || !drawn || models > 0 || warm.outstanding > 0 || waiting > 0 {
                loading.stable = 0.0;
            } else {
                loading.stable += dt;
            }
            let timed_out = since > TIMEOUT;
            if timed_out {
                warn!(
                    "loading: the world around the spawn is not ready after {TIMEOUT} s ({loaded}/{} cells, {models} models, {} units outstanding, {waiting} pipelines), entering anyway",
                    needed.len(),
                    warm.outstanding
                );
            }
            if timed_out || loading.stable >= SETTLE {
                100.0
            } else {
                25.0 + 40.0 * terrain_part + if terrain_done { 35.0 * (loading.stable / SETTLE).min(1.0) * 0.95 } else { 0.0 }
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
        if loading.done > HOLD {
            rt.fire_event("LEFT_LOADING", ());
            info!(
                "loading: done after {:.1} s ({}/{} terrain cells within {NEAR} m, {} objects, {} units, {} pipelines), entering the game",
                loading.t, loading.cells.0, loading.cells.1, loading.entities, loading.announced, loading.pipelines
            );
            let b = loading.busy_until;
            info!(
                "loading: waited for (s): terrain {:.1}, camera frames {:.1}, models {:.1}, units {:.1}, pipelines {:.1}; the objects and the rest settled after {:.1}",
                b[0], b[1], b[2], b[3], b[4], loading.t - HOLD - SETTLE
            );
            next.set(Screen::InGame);
        }
    }
}
