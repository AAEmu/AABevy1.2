//! `--bench S`: frame time statistics of the in-game screen, then exit.
//!
//! * `--bench S` collects the frame times of `S` seconds, starting `--bench-after A` seconds (default 12) after the
//!   player appeared (the town needs that long to fill up), prints one `BENCH` line (frames, average fps, 1 % low, worst
//!   frame, frames over 20 ms) and the scene size, then exits. Combine with `--walk` for the running case.
//! * With `AA_PROFILE=1` the `aa-profile:` blocks of [`aa_world::profile`] are logged as well.

use bevy::ecs::entity::Entities;
use bevy::prelude::*;

use super::units::Views;

#[derive(Resource)]
struct Bench {
    seconds: f32,
    warmup: f32,
    in_game: f32,
    frames: Vec<f32>,
    /// Asset changes (added or modified) seen while measuring: every one is extracted and prepared again by the render world.
    churn: Churn,
}

/// Changed assets while measuring: events and distinct ids per kind, and the vertices of the changed meshes.
#[derive(Default)]
struct Churn {
    meshes: (u64, std::collections::HashSet<AssetId<Mesh>>),
    mesh_vertices: u64,
    images: (u64, std::collections::HashSet<AssetId<Image>>),
    standard: (u64, std::collections::HashSet<AssetId<StandardMaterial>>),
    color: (u64, std::collections::HashSet<AssetId<ColorMaterial>>),
}

pub fn plugin(app: &mut App) {
    let args: Vec<String> = std::env::args().collect();
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).and_then(|v| v.parse::<f32>().ok());
    let Some(seconds) = value("--bench") else { return };
    aa_world::profile::uncapped(app);
    // the benchmark window may have the focus while someone else drives the machine: only the scripted keys count
    app.add_systems(PreUpdate, clear_input.after(bevy::input::InputSystems));
    app.insert_resource(Bench { seconds, warmup: value("--bench-after").unwrap_or(12.0), in_game: 0.0, frames: Vec::new(), churn: Churn::default() })
        .add_systems(Update, bench.run_if(in_state(crate::Screen::InGame)))
        .add_systems(Last, count_churn.run_if(in_state(crate::Screen::InGame)));
}

fn clear_input(mut keys: ResMut<ButtonInput<KeyCode>>, mut mouse: ResMut<ButtonInput<MouseButton>>) {
    keys.reset_all();
    mouse.reset_all();
}

/// Counts the asset changes of the measured frames (`Last`: after everything that writes assets this frame).
#[allow(clippy::too_many_arguments)]
fn count_churn(
    mut state: ResMut<Bench>,
    meshes: Res<Assets<Mesh>>,
    mut mesh_events: MessageReader<AssetEvent<Mesh>>,
    mut image_events: MessageReader<AssetEvent<Image>>,
    mut standard_events: MessageReader<AssetEvent<StandardMaterial>>,
    mut color_events: MessageReader<AssetEvent<ColorMaterial>>,
) {
    let measuring = state.in_game >= state.warmup;
    let churn = &mut state.churn;
    for e in mesh_events.read() {
        if let (true, AssetEvent::Added { id } | AssetEvent::Modified { id }) = (measuring, e) {
            churn.meshes.0 += 1;
            churn.meshes.1.insert(*id);
            churn.mesh_vertices += meshes.get(*id).map_or(0, |m| m.count_vertices() as u64);
        }
    }
    fn count<A: Asset>(measuring: bool, events: &mut MessageReader<AssetEvent<A>>, into: &mut (u64, std::collections::HashSet<AssetId<A>>)) {
        for e in events.read() {
            if let (true, AssetEvent::Added { id } | AssetEvent::Modified { id }) = (measuring, e) {
                into.0 += 1;
                into.1.insert(*id);
            }
        }
    }
    count(measuring, &mut image_events, &mut churn.images);
    count(measuring, &mut standard_events, &mut churn.standard);
    count(measuring, &mut color_events, &mut churn.color);
}

fn bench(
    time: Res<Time<Real>>,
    mut state: ResMut<Bench>,
    views: Res<Views>,
    entities: &Entities,
    fx: Option<Res<aa_fx::FxStats>>,
    mut exit: MessageWriter<AppExit>,
) {
    let dt = time.delta_secs();
    state.in_game += dt;
    if state.in_game < state.warmup {
        return;
    }
    state.frames.push(dt);
    if state.in_game < state.warmup + state.seconds {
        return;
    }
    let mut f = state.frames.clone();
    f.sort_by(|a, b| b.total_cmp(a));
    let n = f.len().max(1);
    let avg = f.iter().sum::<f32>() / n as f32;
    let k = (n / 100).max(1);
    let low1 = f.iter().take(k).sum::<f32>() / k as f32;
    let over = f.iter().filter(|d| **d > 0.02).count();
    info!(
        "BENCH frames={} avg_fps={:.0} 1%_low_fps={:.0} worst_ms={:.1} over_20ms={} median_ms={:.2}",
        n,
        1.0 / avg,
        1.0 / low1,
        f.first().copied().unwrap_or(0.0) * 1000.0,
        over,
        f.get(n / 2).copied().unwrap_or(0.0) * 1000.0
    );
    info!(
        "BENCH scene: entities {}, units drawn {}, doodads drawn {}, still preparing {}",
        entities.count_spawned(),
        views.unit_entities(),
        views.doodad_entities(),
        views.pending()
    );
    let c = &state.churn;
    let per = |v: u64| v as f32 / n as f32;
    info!(
        "BENCH asset changes per frame (distinct): meshes {:.1} ({}, {:.0} vertices per frame), images {:.2} ({}), standard materials {:.2} ({}), color materials {:.2} ({})",
        per(c.meshes.0),
        c.meshes.1.len(),
        per(c.mesh_vertices),
        per(c.images.0),
        c.images.1.len(),
        per(c.standard.0),
        c.standard.1.len(),
        per(c.color.0),
        c.color.1.len()
    );
    if let Some(fx) = fx {
        info!(
            "BENCH fx: effects {} (active {}), emitters {}, particles {}, quads {}, main thread ms: sim {:.2} upload {:.2} manage {:.2}",
            fx.effects, fx.active, fx.emitters, fx.particles, fx.quads, fx.sim_ms, fx.upload_ms, fx.manage_ms
        );
    }
    exit.write(AppExit::Success);
}
