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
}

pub fn plugin(app: &mut App) {
    let args: Vec<String> = std::env::args().collect();
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).and_then(|v| v.parse::<f32>().ok());
    let Some(seconds) = value("--bench") else { return };
    aa_world::profile::uncapped(app);
    // the benchmark window may have the focus while someone else drives the machine: only the scripted keys count
    app.add_systems(PreUpdate, clear_input.after(bevy::input::InputSystems));
    app.insert_resource(Bench { seconds, warmup: value("--bench-after").unwrap_or(12.0), in_game: 0.0, frames: Vec::new() })
        .add_systems(Update, bench.run_if(in_state(crate::Screen::InGame)));
}

fn clear_input(mut keys: ResMut<ButtonInput<KeyCode>>, mut mouse: ResMut<ButtonInput<MouseButton>>) {
    keys.reset_all();
    mouse.reset_all();
}

fn bench(time: Res<Time<Real>>, mut state: ResMut<Bench>, views: Res<Views>, entities: &Entities, mut exit: MessageWriter<AppExit>) {
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
    info!("BENCH scene: entities {}, units drawn {}, doodads drawn {}", entities.count_spawned(), views.unit_entities(), views.doodad_entities());
    exit.write(AppExit::Success);
}
