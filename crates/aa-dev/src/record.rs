//! `--record <dir>`: frame capture for videos.
//!
//! Writes one PNG per frame (`f_000001.png`, ...) of the primary window at a fixed timestep: from the moment the recording starts
//! every frame advances the app clock by exactly `1 / --record-fps` (`TimeUpdateStrategy::ManualDuration`, so `Time<Real>`,
//! `Time<Virtual>` and the fixed clock all agree), however long rendering and PNG encoding take. The video is smooth even when
//! the app renders at 5 fps; only the wall clock is slower than the game. Encode with ffmpeg:
//!
//! ```text
//! ffmpeg -framerate 30 -i <dir>/f_%06d.png -c:v libx264 -crf 20 -pix_fmt yuv420p out.mp4
//! ```
//!
//! * `--record-fps N` frame rate of the capture (default 30).
//! * `--record-from S` game (`Time`) seconds to run normally first (assets stream in) before the first captured frame. Counted from the
//!   start of the app, or, with `--record-gate`, from the moment another plugin calls [`RecordGate::open`] (a scripted scene opens
//!   it when its own clock starts, so the recording origin does not depend on how long loading took).
//! * `--record-secs N` length of the recording (else it runs until `--exit-after`). The app exits once the last PNG is written.
//!
//! The log line `aa-dev: record start at gate+X.XXX s` says exactly which game second (counted from the start, or from the gate)
//! the first captured frame belongs to; a frame `k` (1-based) is at `X + k / fps`.
//!
//! PNGs are encoded on their own threads (fast compression, RGB); at most [`MAX_WRITING`] encoders run, the app waits for the
//! writers beyond that, and past [`MAX_IN_FLIGHT`] undelivered screenshots it renders frames with the clock frozen. The capture is the window: keep it 1280x720 for videos (`--size 1280x720`, about 1 MB per frame).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::time::TimeUpdateStrategy;

use super::DevArgs;

/// PNG encoders running at once (they are threads of their own, so waiting for them is safe).
const MAX_WRITING: usize = 10;
/// Screenshots requested but not handed back by the GPU yet. They only complete while the app keeps rendering frames (the render
/// thread polls the readback, a main-world system delivers it), so the app must never block on them: past this many it lets frames
/// pass with the clock frozen instead.
const MAX_IN_FLIGHT: usize = 4;

/// With `--record-gate` the recording clock (`--record-from`) starts when this opens.
#[derive(Resource, Default)]
pub struct RecordGate {
    opened_at: Option<f64>,
}

impl RecordGate {
    /// Starts the `--record-from` countdown (the first call counts, `now` is `Time::elapsed_secs_f64`).
    pub fn open(&mut self, now: f64) {
        self.opened_at.get_or_insert(now);
    }
}

#[derive(Resource, Default)]
pub struct Recorder {
    /// Frames requested so far (0 until the recording starts).
    frames: u32,
    started: bool,
    /// Screenshots requested and not delivered yet.
    in_flight: Arc<AtomicUsize>,
    /// PNG encoder threads running.
    writing: Arc<AtomicUsize>,
    origin: f64,
}

impl Recorder {
    /// PNGs are still being written.
    pub fn busy(&self) -> bool {
        self.in_flight.load(Ordering::Relaxed) + self.writing.load(Ordering::Relaxed) > 0
    }
}

pub fn plugin(app: &mut App) {
    app.init_resource::<RecordGate>().init_resource::<Recorder>();
    if app.world().resource::<DevArgs>().record.is_some() {
        app.add_systems(Last, capture);
    }
}

fn capture(
    mut commands: Commands,
    args: Res<DevArgs>,
    time: Res<Time>,
    gate: Res<RecordGate>,
    mut rec: ResMut<Recorder>,
    mut strategy: ResMut<TimeUpdateStrategy>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(dir) = args.record.as_deref() else { return };
    let fps = args.record_fps.max(1.0);
    let now = time.elapsed_secs_f64();
    if !rec.started {
        // a gated recording waits for its gate, then counts `--record-from` from there
        let base = if args.record_gate {
            match gate.opened_at {
                Some(at) => at,
                None => return,
            }
        } else {
            0.0
        };
        if now < base + args.record_from as f64 {
            return;
        }
        let _ = std::fs::create_dir_all(dir);
        rec.started = true;
        rec.origin = now - base;
        *strategy = TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / fps as f64));
        info!("aa-dev: record start at gate+{:.3} s ({} fps, {dir})", rec.origin, fps);
        return;
    }
    let total = args.record_secs.map(|s| (s * fps).round() as u32);
    if total.is_some_and(|n| rec.frames >= n) {
        if !rec.busy() {
            info!("aa-dev: recorded {} frames in {dir}", rec.frames);
            exit.write(AppExit::Success);
        }
        return;
    }
    // writers behind: wait for them (the clock only moves when frames do, so nothing is lost)
    while rec.writing.load(Ordering::Relaxed) >= MAX_WRITING {
        std::thread::sleep(Duration::from_millis(4));
    }
    // the GPU behind: let frames pass with the clock frozen until it has handed screenshots back (blocking here would stop the
    // very frames that deliver them: a deadlock)
    if rec.in_flight.load(Ordering::Relaxed) >= MAX_IN_FLIGHT {
        *strategy = TimeUpdateStrategy::ManualDuration(Duration::ZERO);
        return;
    }
    *strategy = TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / fps as f64));
    rec.frames += 1;
    let path = PathBuf::from(dir).join(format!("f_{:06}.png", rec.frames));
    let in_flight = rec.in_flight.clone();
    let writing = rec.writing.clone();
    in_flight.fetch_add(1, Ordering::Relaxed);
    commands.spawn(Screenshot::primary_window()).observe(move |captured: On<ScreenshotCaptured>| {
        let image = captured.image.clone();
        let path = path.clone();
        writing.fetch_add(1, Ordering::Relaxed);
        in_flight.fetch_sub(1, Ordering::Relaxed);
        let writing = writing.clone();
        std::thread::spawn(move || {
            if let Err(e) = write_png(image, &path) {
                error!("aa-dev: record {}: {e}", path.display());
            }
            writing.fetch_sub(1, Ordering::Relaxed);
        });
    });
}

fn write_png(image: Image, path: &std::path::Path) -> Result<(), String> {
    use image::ImageEncoder;
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    let rgb = image.try_into_dynamic().map_err(|e| e.to_string())?.into_rgb8();
    let file = std::io::BufWriter::new(std::fs::File::create(path).map_err(|e| e.to_string())?);
    PngEncoder::new_with_quality(file, CompressionType::Fast, FilterType::Sub)
        .write_image(rgb.as_raw(), rgb.width(), rgb.height(), image::ExtendedColorType::Rgb8)
        .map_err(|e| e.to_string())
}
