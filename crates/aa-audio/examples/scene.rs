//! The Bevy plugin without a window: an emitter circles the listener while its event plays.
//!
//! ```text
//! cargo run -p aa-audio --release --example scene -- --sound [--event sounds/x2skills:wild:fire_arrow_form] [--seconds 6] [--volume 0.2]
//! ```
//!
//! Silent without `--sound`. The entity carries a [`Sound`]; the engine pans it from the listener's
//! transform and sets its loudness from the event's rolloff, and the sound ends with the entity.

use std::time::Duration;

use aa_audio::{AaAudioPlugin, Engine, Settings, Sound, SpatialListener, Volumes};
use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use bevy::transform::TransformPlugin;

#[derive(Resource)]
struct Run {
    seconds: f32,
    event: String,
}

#[derive(Component)]
struct Orbit;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    let run = Run { seconds: value("--seconds").and_then(|s| s.parse().ok()).unwrap_or(6.0), event: value("--event").unwrap_or_else(|| "sounds/x2skills:wild:fire_arrow_form".into()) };
    let volumes = Volumes { master: value("--volume").and_then(|s| s.parse().ok()).unwrap_or(0.2), ..Volumes::default() };
    let audible = args.iter().any(|a| a == "--sound");
    println!("{} for {} s, {}", run.event, run.seconds, if audible { "audible" } else { "muted (pass --sound)" });
    App::new()
        .add_plugins((MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_millis(16))), TransformPlugin, AaAudioPlugin::with(Settings { audible, ..Settings::default() })))
        .insert_resource(volumes)
        .insert_resource(run)
        .add_systems(Startup, setup)
        .add_systems(Update, (orbit, finish))
        .run();
}

fn setup(mut commands: Commands, run: Res<Run>) {
    commands.spawn((Transform::from_xyz(0.0, 0.0, 0.0), SpatialListener));
    commands.spawn((Transform::from_xyz(6.0, 0.0, 0.0), Sound::event(run.event.clone()), Orbit));
}

/// Once around the listener every four seconds, starting on the right.
fn orbit(time: Res<Time>, mut q: Query<&mut Transform, With<Orbit>>) {
    let a = time.elapsed_secs() * std::f32::consts::TAU / 4.0;
    for mut t in &mut q {
        t.translation = Vec3::new(6.0 * a.cos(), 0.0, -6.0 * a.sin());
    }
}

fn finish(time: Res<Time>, run: Res<Run>, engine: Res<Engine>, mut exit: MessageWriter<AppExit>) {
    if time.elapsed_secs() >= run.seconds {
        engine.stop_all();
        println!("{:?}", engine.stats());
        exit.write(AppExit::Success);
    }
}
