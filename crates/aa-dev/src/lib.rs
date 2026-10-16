//! Shared dev harness for every example and the client binary.
//!
//! Command line flags (all optional), parsed from `std::env::args`:
//!
//! * `--shot <file.png>`  save a screenshot of the primary window, then exit
//! * `--shot-after <s>`   seconds to wait before the screenshot (default 5; covers asset loading)
//! * `--exit-after <s>`   exit after this many seconds (no screenshot)
//! * `--size <WxH>`       window size (default 1920x1080, the real client's resolution)
//!
//! FPS and frame time are logged every 2 s (`aa-dev: fps=… frame_ms=…`) so performance can be
//! compared from the console. Typical check:
//!
//! ```text
//! cargo run -p aa-ui --example login -- --shot shots/aa-ui/login.png
//! ```

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::{PresentMode, WindowResolution};

/// Parsed command line options.
#[derive(Resource, Clone, Debug)]
pub struct DevArgs {
    pub shot: Option<String>,
    pub shot_after: f32,
    pub exit_after: Option<f32>,
    pub size: (u32, u32),
}

impl DevArgs {
    pub fn from_env() -> Self {
        let args: Vec<String> = std::env::args().collect();
        let value = |flag: &str| {
            args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
        };
        let size = value("--size")
            .and_then(|s| {
                let (w, h) = s.split_once('x')?;
                Some((w.parse().ok()?, h.parse().ok()?))
            })
            .unwrap_or((1920, 1080));
        DevArgs {
            shot: value("--shot"),
            shot_after: value("--shot-after").and_then(|s| s.parse().ok()).unwrap_or(5.0),
            exit_after: value("--exit-after").and_then(|s| s.parse().ok()),
            size,
        }
    }
}

/// `DefaultPlugins` with the standard window and the pak-friendly asset settings.
pub fn default_plugins(title: &str) -> impl PluginGroup {
    let args = DevArgs::from_env();
    DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: title.to_string(),
            resolution: WindowResolution::new(args.size.0, args.size.1),
            present_mode: PresentMode::AutoNoVsync,
            ..default()
        }),
        ..default()
    })
}

/// FPS logging plus `--shot` / `--exit-after` handling.
pub struct DevPlugin;

impl Plugin for DevPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<FrameTimeDiagnosticsPlugin>() {
            app.add_plugins(FrameTimeDiagnosticsPlugin::default());
        }
        app.insert_resource(DevArgs::from_env())
            .init_resource::<DevState>()
            .add_systems(Update, (log_fps, shot_and_exit));
    }
}

#[derive(Resource, Default)]
struct DevState {
    since_log: f32,
    shot_requested: bool,
    shot_wait: f32,
}

fn log_fps(time: Res<Time>, diagnostics: Res<DiagnosticsStore>, mut state: ResMut<DevState>) {
    state.since_log += time.delta_secs();
    if state.since_log < 2.0 {
        return;
    }
    state.since_log = 0.0;
    let fps = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0);
    let ms = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FRAME_TIME)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0);
    info!("aa-dev: fps={fps:.1} frame_ms={ms:.2}");
}

fn shot_and_exit(
    mut commands: Commands,
    time: Res<Time>,
    args: Res<DevArgs>,
    mut state: ResMut<DevState>,
    mut exit: MessageWriter<AppExit>,
) {
    let t = time.elapsed_secs();
    if let Some(path) = &args.shot {
        if !state.shot_requested && t >= args.shot_after {
            if let Some(dir) = std::path::Path::new(path).parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path.clone()));
            state.shot_requested = true;
            info!("aa-dev: screenshot -> {path}");
        } else if state.shot_requested {
            // Give the capture a moment to be read back and written before exiting.
            state.shot_wait += time.delta_secs();
            if state.shot_wait > 1.5 && std::path::Path::new(path).exists() {
                exit.write(AppExit::Success);
            }
        }
    }
    if args.exit_after.is_some_and(|s| t >= s) {
        exit.write(AppExit::Success);
    }
}
