//! Loading screen: the original loading UI (`scriptsbin/globalui/loading`) while the server
//! spawns the character and the world around the spawn streams in, then `Screen::InGame`.
//!
//! The UI is driven by the events the client raises: `ENTERED_LOADING(worldImagePath)`,
//! `STILL_LOADING(percent)` and `LEFT_LOADING`. The percentage is real:
//!
//! * 0..25: the server's enter-game sequence (select, state, spawn) until `PlayerSpawn`;
//! * 25..80: terrain cells within [`NEAR`] metres of the spawn built by aa-world's streamer (the
//!   world is pointed at the spawn through the [`WorldAnchor`]);
//! * 80..100: objects and vegetation around it have stopped appearing.
//!
//! Then the screen switches to `Screen::InGame`, whose `spawn_player` builds the character from the
//! spawn data exactly as `--enter-world` does. `--hold-loading PCT` pins the gauge at PCT and stays
//! (for screenshots against `reference/loading`).

use aa_net::state::Player;
use aa_ui::{Stage, UiRuntime};
use aa_world::coords::cell_origin;
use aa_world::objects::ObjectState;
use aa_world::stream::Streamer;
use aa_world::{CELL_SIZE, TerrainQuery, cry_to_bevy};
use bevy::prelude::*;

use super::{LoginArgs, LoginBridge, WorldAnchor, gameapi, host, ui_is};
use crate::Screen;

/// Terrain within this many metres of the spawn must be built before the game starts.
const NEAR: f32 = 500.0;
/// Seconds without new object entities after which the objects count as settled.
const SETTLE: f32 = 0.7;
/// Give up waiting for the world after this many seconds (the game starts anyway).
const TIMEOUT: f32 = 25.0;

pub fn plugin(app: &mut App) {
    app.init_resource::<Loading>()
        .add_systems(OnEnter(Screen::Loading), reset)
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
    /// Object entity count and since when it has not changed.
    entities: usize,
    stable: f32,
    /// Seconds the gauge has been at 100.
    done: f32,
}

fn reset(mut loading: ResMut<Loading>) {
    *loading = Loading::default();
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
            let terrain_done = needed.is_empty() || loaded == needed.len();
            let terrain_part = if needed.is_empty() { 1.0 } else { loaded as f32 / needed.len() as f32 };
            let entities = objects.as_ref().map_or(0, |o| o.stats.entities);
            if entities != loading.entities {
                loading.entities = entities;
                loading.stable = 0.0;
            } else if terrain_done {
                loading.stable += dt;
            }
            let timed_out = since > TIMEOUT;
            if timed_out {
                warn!("loading: the world around the spawn is not ready after {TIMEOUT} s ({loaded}/{} cells), entering anyway", needed.len());
            }
            if timed_out || (terrain_done && loading.stable >= SETTLE) {
                100.0
            } else {
                25.0 + 55.0 * terrain_part + if terrain_done { 20.0 * (loading.stable / SETTLE).min(1.0) * 0.95 } else { 0.0 }
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
        if loading.done > 0.4 {
            rt.fire_event("LEFT_LOADING", ());
            info!("loading: done after {:.1} s, entering the game", loading.t);
            next.set(Screen::InGame);
        }
    }
}
