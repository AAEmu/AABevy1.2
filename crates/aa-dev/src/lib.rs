//! Shared dev harness for every example and the client binary.
//!
//! Command line flags (all optional), parsed from `std::env::args`:
//!
//! * `--shot <file.png>`  save a screenshot of the primary window, then exit
//! * `--shot-after <s>`   seconds to wait before the screenshot (default 5; covers asset loading)
//! * `--exit-after <s>`   exit after this many seconds (no screenshot)
//! * `--size <WxH>`       window size (default 1920x1080, the real client's resolution)
//! * `--record <dir>`     write one PNG per frame (`f_000001.png`, ...) at a fixed timestep, then exit: see [`record`]
//!   (`--record-fps 30`, `--record-from S` game seconds before the first frame, `--record-secs N` length,
//!   `--record-gate` count `--record-from` from [`RecordGate::open`] instead of from the start)
//! * `--headless`         no OS window and no GPU: the primary `Window` is only a component (the UI needs its size),
//!   the schedule loop runs at `--headless-fps N` (default 30) and the game logic, `aa-net`, the Lua UI and the HUD run
//!   as usual. `--shot` is not possible then; see `--offscreen-shot`
//! * `--offscreen-shot <file.png>`  headless with a GPU: every camera that would draw into the window draws into an
//!   image instead, and the image is saved `--shot-after` seconds (default 5) after start (the process then exits
//!   unless a script is running, see [`capture`])
//! * `--allow-lua-error <text>`  (repeatable) a Lua error whose message contains the text is not counted, see [`capture`]
//!
//! FPS and frame time are logged every 2 s (`aa-dev: fps=… frame_ms=…`) so performance can be
//! compared from the console. With `--shot` or `--exit-after` the app also runs uncapped when the
//! window is unfocused. Typical check:
//!
//! ```text
//! cargo run -p aa-ui --example login -- --shot shots/aa-ui/login.png
//! ```

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::{PresentMode, WindowResolution};
use bevy::winit::WinitSettings;

pub mod capture;
mod headless;
mod record;
pub use headless::{OffscreenTarget, SCRIPTED, take_offscreen_shot};
pub use record::RecordGate;

/// Parsed command line options.
#[derive(Resource, Clone, Debug)]
pub struct DevArgs {
    pub shot: Option<String>,
    pub shot_after: f32,
    pub exit_after: Option<f32>,
    pub size: (u32, u32),
    /// `--record <dir>`: frame capture, see [`record`].
    pub record: Option<String>,
    pub record_fps: f32,
    pub record_from: f32,
    pub record_secs: Option<f32>,
    pub record_gate: bool,
    /// `--headless` (also implied by `--offscreen-shot`): no OS window.
    pub headless: bool,
    /// `--headless-fps N`: the headless loop's frame rate.
    pub headless_fps: f32,
    /// `--offscreen-shot <png>`: render into an image, save it, see [`headless`].
    pub offscreen_shot: Option<String>,
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
            record: value("--record"),
            record_fps: value("--record-fps").and_then(|s| s.parse().ok()).unwrap_or(30.0),
            record_from: value("--record-from").and_then(|s| s.parse().ok()).unwrap_or(0.0),
            record_secs: value("--record-secs").and_then(|s| s.parse().ok()),
            record_gate: args.iter().any(|a| a == "--record-gate"),
            headless: args.iter().any(|a| a == "--headless") || value("--offscreen-shot").is_some(),
            headless_fps: value("--headless-fps").and_then(|s| s.parse().ok()).unwrap_or(30.0),
            offscreen_shot: value("--offscreen-shot"),
        }
    }
}

/// `DefaultPlugins` with the standard window and the pak-friendly asset settings. With `--headless` there is no
/// OS window (no winit), the loop is a plain timed schedule runner and, unless `--offscreen-shot` needs the GPU, the
/// renderer is off.
pub fn default_plugins(title: &str) -> impl PluginGroup {
    default_plugins_with_log(title, capture::layer)
}

/// [`default_plugins`] with a different `LogPlugin` custom layer, e.g. the client's log file on top of the
/// Lua-error capture.
pub fn default_plugins_with_log(
    title: &str,
    custom_layer: fn(&mut App) -> Option<bevy::log::BoxedLayer>,
) -> impl PluginGroup {
    let args = DevArgs::from_env();
    let window = Window {
        title: title.to_string(),
        // One window pixel per image pixel whatever the desktop scaling, so screenshots are
        // the real client's 1920x1080 and compare pixel for pixel.
        resolution: WindowResolution::new(args.size.0, args.size.1).with_scale_factor_override(1.0),
        present_mode: PresentMode::AutoNoVsync,
        ..default()
    };
    let log = bevy::log::LogPlugin { custom_layer, ..default() };
    if !args.headless {
        return DefaultPlugins
            .set(WindowPlugin { primary_window: Some(window), ..default() })
            .set(log)
            .build();
    }
    let mut group = DefaultPlugins
        .set(WindowPlugin {
            primary_window: Some(window),
            exit_condition: bevy::window::ExitCondition::DontExit,
            close_when_requested: false,
            ..default()
        })
        .set(log)
        .disable::<bevy::winit::WinitPlugin>()
        .add(bevy::app::ScheduleRunnerPlugin::run_loop(std::time::Duration::from_secs_f64(1.0 / f64::from(args.headless_fps.max(1.0)))));
    if args.offscreen_shot.is_none() {
        group = group.set(bevy::render::RenderPlugin {
            render_creation: bevy::render::settings::WgpuSettings { backends: None, ..default() }.into(),
            ..default()
        });
    }
    group
}

/// FPS logging plus `--shot` / `--exit-after` handling.
pub struct DevPlugin;

impl Plugin for DevPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<FrameTimeDiagnosticsPlugin>() {
            app.add_plugins(FrameTimeDiagnosticsPlugin::default());
        }
        let args = DevArgs::from_env();
        if !args.headless && (args.shot.is_some() || args.exit_after.is_some() || args.record.is_some()) {
            // Unattended runs usually have no focus; don't let Bevy cap them at 60 Hz, or the
            // logged FPS is meaningless.
            app.insert_resource(WinitSettings::continuous());
        }
        app.insert_resource(args)
            .init_resource::<DevState>()
            .add_systems(Update, (log_fps, shot_and_exit));
        record::plugin(app);
        headless::plugin(app);
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
    recording: Res<record::Recorder>,
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
    if args.exit_after.is_some_and(|s| t >= s) && !recording.busy() {
        exit.write(AppExit::Success);
    }
}
