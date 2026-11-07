//! `--video <out.mp4>`: wall-clock video capture, frames taken from inside the app.
//!
//! Unlike `--record` (fixed timestep, PNGs, see [`record`](super::record)) the app clock stays the wall clock, so a run that
//! talks to a live server and passes `--require-wall-clock` can be filmed. The frames come from `Screenshot`s of the primary
//! window (a capture from outside the process is useless on a locked desktop) and go to an `ffmpeg` child process over a
//! pipe:
//!
//! ```text
//! ffmpeg -f rawvideo -pix_fmt rgba|bgra -s WxH -framerate FPS -i - -vf scale=trunc(iw/2)*2:trunc(ih/2)*2
//!        -c:v libx264 -preset ultrafast -crf 18 -pix_fmt yuv420p -y out.mp4
//! ```
//!
//! * `--video-fps N` frame rate of the file (default 30), `--ffmpeg <path>` (else the `FFMPEG` variable, else `ffmpeg` on PATH).
//! * `--video-gate` starts the capture when [`RecordGate::open`](super::RecordGate::open) or [`open_video_gate`](super::open_video_gate)
//!   is called (the script runner opens it at a `trace video-start` / `echo video-start` line, [`GATE_MARK`](super::GATE_MARK))
//!   instead of at the first frame.
//!
//! **Tick grid.** The capture has a fixed grid: tick `k` is at wall time `t0 + k / fps`, `t0` being `SystemTime::now()` (Unix
//! microseconds) at the first app frame of the capture. The grid itself runs on the monotonic clock from that moment. Every app
//! frame, each tick that is due gets the newest image that has been delivered (an image used by more than one tick is a
//! `repeated` one) and goes to the writer thread; a new `Screenshot` is requested at most once per tick, and not at all while
//! [`MAX_IN_FLIGHT`] are pending (`skipped`). Ticks due before the first image arrived get that first image. Frame `k` of the
//! file is tick `k`: when the writer's queue is full the ticks are not queued (`dropped`) and the writer repeats its last frame
//! for them, so the file keeps its constant frame rate and its clock.
//!
//! A screenshot is delivered a few app frames after it was requested, so a tick is not handed out at its time but a *hold* later
//! (about 1.5 times the measured delivery latency plus 3 ms, at most one tick period, see [`hold_us`]): by then the screenshot
//! requested at the tick's time is in, and the frame shows the world as it was at the tick. The writer only cares for the order, so
//! the hold costs nothing. The sidecar measures what is left: `image_age_ms_mean` / `image_age_ms_max` are the tick's time minus
//! the time the image on it was requested (about 0, a few ms either way, when the hold works; large when the app is slower than
//! the grid and at the start, before the first screenshot is back; `_mean`, `_p50`, `_p95`, `_max`), `latency_ms` the last smoothed
//! delivery latency.
//!
//! **Exit.** On `AppExit` (`--exit-after`, the end of a script, a closed window) the due ticks are flushed, ffmpeg's stdin is
//! closed and ffmpeg is waited for (at most [`FFMPEG_WAIT`], then it is killed); `<out.mp4>.json` and `video.json` next to it
//! are written before the process goes on to exit. Their fields: `t0_wall_us`, `fps`, `width`, `height` (of the file),
//! `frames`, `repeated`, `skipped`, `dropped`, `ffmpeg_exit`, `method` (`"aa-dev --video"`), and `format` (the window's texture
//! format), `latency_ms`, `image_age_ms_*`, `lost` (requests that never came back), `stopped` / `error` when set.
//!
//! **Limits.** A window resize while recording stops the capture cleanly (the file ends at the last frame of the first size,
//! `stopped` says why; ffmpeg's input size is fixed). `--video` refuses `--record` and `--headless` (no GPU); with
//! `--offscreen-shot` the offscreen image is what is filmed. Not in the browser.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bevy::app::OnAppExitSystems;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};

use super::{DevArgs, OffscreenTarget, RecordGate};

/// Screenshots requested and not delivered yet: past this many no new one is requested (counted as `skipped`).
pub const MAX_IN_FLIGHT: usize = 3;
/// Ticks (runs of ticks) the writer's queue holds: with the frame being written and the newest image that is at most
/// `QUEUE + 2` full-size frames in memory.
const QUEUE: usize = 8;
/// A screenshot request older than this is given up on (`lost`), so a window that cannot be captured (minimised) does not
/// block the requests for good.
const STALE: Duration = Duration::from_secs(5);
/// How long the exit waits for ffmpeg to finish the file before it kills it.
pub const FFMPEG_WAIT: Duration = Duration::from_secs(30);

// ---- the tick bookkeeping (pure) -------------------------------------------------------------------------------------------

/// Time of tick `k` in microseconds after the first tick.
fn tick_us(fps: u32, tick: u64) -> u64 {
    tick * 1_000_000 / u64::from(fps)
}

/// How many ticks (`0..n`) are due `elapsed_us` after the first one: those with `tick_us(k) <= elapsed_us` (tick 0 at once).
fn ticks_due(fps: u32, elapsed_us: u64) -> u64 {
    ((elapsed_us + 1) * u64::from(fps)).div_ceil(1_000_000)
}

/// How long a tick waits before it is handed out, so that the screenshot requested at its time has been delivered by then: one and
/// a half times the recent delivery latency plus a margin, never more than a tick period (past that the next image would be
/// taken for this tick).
fn hold_us(fps: u32, latency_us: u64) -> u64 {
    let period = 1_000_000 / u64::from(fps);
    (latency_us * 3 / 2 + 3_000).min(period.saturating_sub(3_000))
}

/// Consecutive ticks that show the same image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Run {
    image: u64,
    first_tick: u64,
    count: u64,
}

/// Which image each tick gets: no clock, no Bevy, so the cadence logic is testable on its own.
#[derive(Debug)]
struct Ticks {
    fps: u32,
    /// Next tick to hand out (= ticks handed out).
    next: u64,
    /// The newest image delivered so far.
    latest: Option<u64>,
    /// The image the last tick got.
    last_used: Option<u64>,
    /// Tick period of the last screenshot request.
    requested: Option<u64>,
    /// Ticks that showed an image an earlier tick had shown.
    repeated: u64,
}

impl Ticks {
    fn new(fps: u32) -> Self {
        Ticks { fps, next: 0, latest: None, last_used: None, requested: None, repeated: 0 }
    }

    /// An image was delivered (ids grow with the requests; an older one arriving late does not replace a newer one).
    fn arrived(&mut self, image: u64) {
        self.latest = Some(self.latest.map_or(image, |l| l.max(image)));
    }

    /// The ticks due `elapsed_us` after the first one, all with the newest image (none until an image has arrived).
    fn due(&mut self, elapsed_us: u64) -> Option<Run> {
        let image = self.latest?;
        let upto = ticks_due(self.fps, elapsed_us);
        if upto <= self.next {
            return None;
        }
        let run = Run { image, first_tick: self.next, count: upto - self.next };
        self.next = upto;
        let fresh = self.last_used != Some(image);
        self.repeated += run.count - u64::from(fresh);
        self.last_used = Some(image);
        Some(run)
    }

    /// `true` once per tick period: time to request a new screenshot.
    fn request_due(&mut self, elapsed_us: u64) -> bool {
        let period = ticks_due(self.fps, elapsed_us) - 1;
        if self.requested.is_some_and(|r| r >= period) {
            return false;
        }
        self.requested = Some(period);
        true
    }
}

// ---- frames and the writer thread -----------------------------------------------------------------------------------------

/// One captured image as ffmpeg takes it: tightly packed 8-bit RGBA or BGRA.
struct Frame {
    width: u32,
    height: u32,
    bgra: bool,
    data: Vec<u8>,
}

/// The frame of a delivered image: the window's `Rgba8`/`Bgra8` bytes as they are (no copy), anything else converted.
fn frame_from(image: &mut Image) -> Result<Frame, String> {
    let (width, height) = (image.width(), image.height());
    let bgra = match image.texture_descriptor.format {
        TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb => Some(false),
        TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb => Some(true),
        _ => None,
    };
    let (bgra, data) = match bgra {
        Some(bgra) => (bgra, image.data.take().ok_or("the screenshot has no data")?),
        None => {
            let rgba = image.clone().try_into_dynamic().map_err(|e| format!("{:?}: {e}", image.texture_descriptor.format))?.into_rgba8();
            (false, rgba.into_raw())
        }
    };
    if data.len() != width as usize * height as usize * 4 {
        return Err(format!("{width}x{height} screenshot of {} bytes", data.len()));
    }
    Ok(Frame { width, height, bgra, data })
}

/// A run of ticks for the writer.
struct Msg {
    frame: Arc<Frame>,
    first_tick: u64,
    count: u64,
}

/// What the writer thread reports when it is done.
#[derive(Default)]
struct WriterEnd {
    exit: Option<i32>,
    error: Option<String>,
    tail: Vec<String>,
}

struct Writer {
    tx: Option<SyncSender<Msg>>,
    done: Mutex<Receiver<WriterEnd>>,
    thread: Option<JoinHandle<()>>,
    failed: Arc<AtomicBool>,
}

fn ffmpeg_args(fps: u32, width: u32, height: u32, bgra: bool, out: &Path) -> Vec<String> {
    let mut args: Vec<String> = ["-hide_banner", "-loglevel", "error", "-nostats", "-y", "-f", "rawvideo", "-pix_fmt"]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    args.push(if bgra { "bgra" } else { "rgba" }.into());
    args.extend(["-video_size".into(), format!("{width}x{height}"), "-framerate".into(), fps.to_string(), "-i".into(), "-".into()]);
    args.extend(["-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2", "-c:v", "libx264", "-preset", "ultrafast", "-crf", "18", "-pix_fmt", "yuv420p", "-r"].iter().map(|s| (*s).to_string()));
    args.push(fps.to_string());
    args.push(out.display().to_string());
    args
}

fn spawn_ffmpeg(ffmpeg: &str, args: &[String]) -> std::io::Result<Child> {
    let mut command = Command::new(ffmpeg);
    command.args(args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000); // CREATE_NO_WINDOW
    command.spawn()
}

impl Writer {
    /// Starts the writer thread; it spawns ffmpeg for frames of the size and pixel order of `first`.
    fn start(ffmpeg: String, out: PathBuf, fps: u32, first: &Frame) -> Writer {
        let (tx, rx) = sync_channel::<Msg>(QUEUE);
        let (done_tx, done) = std::sync::mpsc::channel();
        let failed = Arc::new(AtomicBool::new(false));
        let args = ffmpeg_args(fps, first.width, first.height, first.bgra, &out);
        let flag = failed.clone();
        let thread = std::thread::Builder::new()
            .name("aa-dev video writer".into())
            .spawn(move || {
                let _ = done_tx.send(run_writer(&rx, &ffmpeg, &args, &flag));
            })
            .ok();
        Writer { tx: Some(tx), done: Mutex::new(done), thread, failed }
    }

    /// Queues a run; `false` when the queue is full (or the writer is gone).
    fn offer(&self, msg: Msg) -> Result<(), TrySendError<Msg>> {
        match &self.tx {
            Some(tx) => tx.try_send(msg),
            None => Err(TrySendError::Disconnected(msg)),
        }
    }

    /// Closes the queue and waits for the writer (and ffmpeg) to finish, at most `wait`.
    fn finish(&mut self, wait: Duration) -> WriterEnd {
        self.tx = None;
        let end = match self.done.lock().unwrap().recv_timeout(wait) {
            Ok(end) => end,
            Err(_) => WriterEnd { error: Some(format!("ffmpeg did not finish within {} s", wait.as_secs())), ..WriterEnd::default() },
        };
        if let Some(thread) = self.thread.take()
            && thread.is_finished()
        {
            let _ = thread.join();
        }
        end
    }
}

/// The writer thread: ffmpeg is started on the first run; every run goes down the pipe, the ticks the queue could not take are
/// filled with the previous frame so that frame `k` of the file is tick `k`.
fn run_writer(rx: &Receiver<Msg>, ffmpeg: &str, args: &[String], failed: &AtomicBool) -> WriterEnd {
    let mut end = WriterEnd::default();
    let mut child = match spawn_ffmpeg(ffmpeg, args) {
        Ok(child) => child,
        Err(e) => {
            end.error = Some(format!("cannot start {ffmpeg}: {e}"));
            failed.store(true, Ordering::Relaxed);
            while rx.recv().is_ok() {}
            return end;
        }
    };
    let tail = Arc::new(Mutex::new(Vec::<String>::new()));
    let reader = child.stderr.take().map(|stderr| {
        let tail = tail.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let mut tail = tail.lock().unwrap();
                tail.push(line);
                if tail.len() > 12 {
                    tail.remove(0);
                }
            }
        })
    });
    let mut stdin = child.stdin.take();
    let mut next = 0u64;
    let mut last: Option<Arc<Frame>> = None;
    while let Ok(msg) = rx.recv() {
        let Some(pipe) = stdin.as_mut() else { continue }; // ffmpeg is gone: keep draining the queue
        let result = (|| {
            if msg.first_tick > next {
                let fill = last.as_ref().unwrap_or(&msg.frame);
                for _ in next..msg.first_tick {
                    pipe.write_all(&fill.data)?;
                }
            }
            for _ in 0..msg.count {
                pipe.write_all(&msg.frame.data)?;
            }
            std::io::Result::Ok(())
        })();
        match result {
            Ok(()) => {
                next = msg.first_tick + msg.count;
                last = Some(msg.frame);
            }
            Err(e) => {
                end.error = Some(format!("writing to ffmpeg: {e}"));
                failed.store(true, Ordering::Relaxed);
                stdin = None;
            }
        }
    }
    drop(stdin); // ffmpeg sees the end of the input and finishes the file
    let deadline = Instant::now() + FFMPEG_WAIT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                end.exit = status.code();
                break;
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                end.error.get_or_insert_with(|| format!("ffmpeg was killed after {} s", FFMPEG_WAIT.as_secs()));
                break;
            }
            Err(e) => {
                end.error.get_or_insert_with(|| format!("waiting for ffmpeg: {e}"));
                break;
            }
        }
    }
    if let Some(reader) = reader {
        let _ = reader.join();
    }
    end.tail = tail.lock().unwrap().clone();
    end
}

// ---- the capture ----------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    /// Waiting for the gate.
    Waiting,
    Running,
    /// The capture ended (resize, ffmpeg gone); the sidecar is written at exit.
    Stopped,
    Finished,
}

/// The newest delivered image.
struct Latest {
    frame: Arc<Frame>,
    /// When it was requested, in microseconds after the first tick.
    requested_us: i64,
}

#[derive(Default)]
struct Stats {
    skipped: u64,
    dropped: u64,
    lost: u64,
    /// Per tick: its time minus the request time of the image on it, in microseconds.
    ages_us: Vec<i32>,
}

/// The `p` (0..=100) percentile of the sorted values.
fn percentile(sorted: &[i32], p: usize) -> i32 {
    sorted[(sorted.len() * p / 100).min(sorted.len() - 1)]
}

#[derive(Resource)]
struct Video {
    out: PathBuf,
    fps: u32,
    ffmpeg: String,
    phase: Phase,
    /// The monotonic anchor of the grid and the wall clock at the same moment (Unix microseconds).
    start: Option<(Instant, u64)>,
    ticks: Ticks,
    next_id: u64,
    /// Requested screenshots by entity: (id, request time).
    pending: HashMap<Entity, (u64, Instant)>,
    latest: Option<Latest>,
    writer: Option<Writer>,
    /// Size and format of the first image.
    shape: Option<(u32, u32, bool, String)>,
    stats: Stats,
    /// Smoothed time from a screenshot request to its delivery, in microseconds (0 until one has come back).
    latency_us: u64,
    stopped: Option<String>,
    error: Option<String>,
}

impl Video {
    fn new(out: PathBuf, fps: u32, ffmpeg: String) -> Self {
        Video {
            out,
            fps,
            ffmpeg,
            phase: Phase::Waiting,
            start: None,
            ticks: Ticks::new(fps),
            next_id: 0,
            pending: HashMap::new(),
            latest: None,
            writer: None,
            shape: None,
            stats: Stats::default(),
            latency_us: 0,
            stopped: None,
            error: None,
        }
    }

    fn elapsed_us(&self, now: Instant) -> u64 {
        self.start.map_or(0, |(start, _)| now.saturating_duration_since(start).as_micros() as u64)
    }

    /// Hands the ticks that are due to the writer: those whose time plus the hold ([`hold_us`]) has passed, or with `flush` (the
    /// exit) all of those whose time has.
    fn emit_due(&mut self, now: Instant, flush: bool) {
        let hold = if flush { 0 } else { hold_us(self.fps, self.latency_us) };
        let Some(elapsed) = self.elapsed_us(now).checked_sub(hold) else { return };
        let Some(run) = self.ticks.due(elapsed) else { return };
        let (Some(latest), Some(writer)) = (&self.latest, &self.writer) else { return };
        for k in run.first_tick..run.first_tick + run.count {
            let age = tick_us(self.fps, k) as i64 - latest.requested_us;
            self.stats.ages_us.push(age.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32);
        }
        let msg = Msg { frame: latest.frame.clone(), first_tick: run.first_tick, count: run.count };
        match writer.offer(msg) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => self.stats.dropped += run.count,
            Err(TrySendError::Disconnected(_)) => {
                self.stats.dropped += run.count;
                self.stop("the writer is gone");
            }
        }
    }

    /// A screenshot came back.
    fn deliver(&mut self, entity: Entity, image: &mut Image) {
        if self.phase != Phase::Running {
            return;
        }
        let Some((id, requested)) = self.pending.remove(&entity) else { return }; // given up on
        if self.ticks.latest.is_some_and(|newest| newest > id) {
            return; // overtaken by a newer one
        }
        let latency = Instant::now().saturating_duration_since(requested).as_micros() as u64;
        self.latency_us = if self.latency_us == 0 { latency } else { (self.latency_us * 7 + latency) / 8 };
        let frame = match frame_from(image) {
            Ok(frame) => frame,
            Err(e) => {
                self.error = Some(format!("unusable screenshot: {e}"));
                self.stop("unusable screenshot");
                return;
            }
        };
        let shape = (frame.width, frame.height, frame.bgra, format!("{:?}", image.texture_descriptor.format));
        match &self.shape {
            None => {
                info!("aa-dev: video {}x{} {} -> {}", shape.0, shape.1, shape.3, self.out.display());
                self.writer = Some(Writer::start(self.ffmpeg.clone(), self.out.clone(), self.fps, &frame));
                self.shape = Some(shape);
            }
            Some(first) if first.0 != shape.0 || first.1 != shape.1 || first.2 != shape.2 => {
                let why = format!("the window changed from {}x{} to {}x{}: capture stopped", first.0, first.1, shape.0, shape.1);
                warn!("aa-dev: video: {why}");
                self.stop(&why);
                return;
            }
            Some(_) => {}
        }
        let requested_us = self.start.map_or(0, |(start, _)| requested.saturating_duration_since(start).as_micros() as i64);
        self.latest = Some(Latest { frame: Arc::new(frame), requested_us });
        self.ticks.arrived(id);
    }

    /// Ends the capture: no more requests or ticks; the writer finishes the file in the background.
    fn stop(&mut self, why: &str) {
        if self.phase == Phase::Running {
            self.phase = Phase::Stopped;
            self.stopped = Some(why.to_string());
            if let Some(writer) = self.writer.as_mut() {
                writer.tx = None;
            }
        }
    }

    /// The exit: flush the ticks due so far, close the pipe, wait for ffmpeg and write the sidecars. Once.
    fn finish(&mut self) {
        if self.phase == Phase::Finished {
            return;
        }
        if self.phase == Phase::Running {
            self.emit_due(Instant::now(), true);
        }
        self.phase = Phase::Finished;
        let end = match self.writer.as_mut() {
            Some(writer) => {
                if writer.failed.load(Ordering::Relaxed) && self.error.is_none() {
                    self.error = Some("ffmpeg stopped taking frames".into());
                }
                writer.finish(FFMPEG_WAIT + Duration::from_secs(5))
            }
            None => WriterEnd::default(),
        };
        if let Some(e) = &end.error {
            self.error.get_or_insert_with(|| e.clone());
        }
        if self.writer.is_none() && self.error.is_none() {
            self.error = Some(if self.start.is_none() { "the capture never started (the gate stayed shut)".into() } else { "no screenshot was delivered".into() });
        }
        let json = self.sidecar(&end);
        for path in [PathBuf::from(format!("{}.json", self.out.display())), self.out.with_file_name("video.json")] {
            if let Err(e) = std::fs::write(&path, &json) {
                error!("aa-dev: video: cannot write {}: {e}", path.display());
            }
        }
        let frames = self.ticks.next;
        let level = if end.exit == Some(0) && self.error.is_none() { "ok" } else { "FAILED" };
        let msg = format!(
            "aa-dev: video {level}: {frames} frames ({} repeated, {} skipped, {} dropped, {} lost) ffmpeg exit {:?} -> {}{}",
            self.ticks.repeated,
            self.stats.skipped,
            self.stats.dropped,
            self.stats.lost,
            end.exit,
            self.out.display(),
            self.error.as_deref().map(|e| format!(" ({e})")).unwrap_or_default()
        );
        if level == "ok" {
            info!("{msg}");
        } else {
            error!("{msg}");
            for line in &end.tail {
                error!("aa-dev: ffmpeg: {line}");
            }
        }
    }

    fn sidecar(&self, end: &WriterEnd) -> String {
        let (width, height, format) = self.shape.as_ref().map_or((0, 0, ""), |s| (s.0 / 2 * 2, s.1 / 2 * 2, s.3.as_str()));
        let mut fields: Vec<(&str, String)> = vec![
            ("t0_wall_us", self.start.map_or("null".to_string(), |s| s.1.to_string())),
            ("fps", self.fps.to_string()),
            ("width", width.to_string()),
            ("height", height.to_string()),
            ("frames", self.ticks.next.to_string()),
            ("repeated", self.ticks.repeated.to_string()),
            ("skipped", self.stats.skipped.to_string()),
            ("dropped", self.stats.dropped.to_string()),
            ("ffmpeg_exit", end.exit.map_or("null".to_string(), |c| c.to_string())),
            ("method", json_str("aa-dev --video")),
            ("format", json_str(format)),
            ("lost", self.stats.lost.to_string()),
        ];
        fields.push(("latency_ms", format!("{:.1}", self.latency_us as f64 / 1000.0)));
        if !self.stats.ages_us.is_empty() {
            let mut sorted = self.stats.ages_us.clone();
            sorted.sort_unstable();
            let ms = |us: i32| format!("{:.1}", f64::from(us) / 1000.0);
            let mean = sorted.iter().map(|&a| f64::from(a)).sum::<f64>() / sorted.len() as f64 / 1000.0;
            fields.push(("image_age_ms_mean", format!("{mean:.1}")));
            fields.push(("image_age_ms_p50", ms(percentile(&sorted, 50))));
            fields.push(("image_age_ms_p95", ms(percentile(&sorted, 95))));
            fields.push(("image_age_ms_max", ms(sorted[sorted.len() - 1])));
        }
        if let Some(why) = &self.stopped {
            fields.push(("stopped", json_str(why)));
        }
        if let Some(e) = &self.error {
            fields.push(("error", json_str(e)));
        }
        if !end.tail.is_empty() {
            fields.push(("ffmpeg_tail", format!("[{}]", end.tail.iter().map(|l| json_str(l)).collect::<Vec<_>>().join(","))));
        }
        let body: Vec<String> = fields.into_iter().map(|(k, v)| format!("\"{k}\":{v}")).collect();
        format!("{{{}}}\n", body.join(","))
    }
}

impl Drop for Video {
    /// The world goes away without an `AppExit` having reached [`capture_at_exit`]: still close the file.
    fn drop(&mut self) {
        if self.phase != Phase::Finished && self.writer.is_some() {
            self.finish();
        }
    }
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Why `--video` cannot run with these options, if it cannot.
pub fn conflict(args: &DevArgs) -> Option<&'static str> {
    args.video.as_ref()?;
    if args.record.is_some() {
        return Some("--video cannot be combined with --record: --record is a fixed-timestep PNG capture, --video films the wall clock");
    }
    if args.headless && args.offscreen_shot.is_none() {
        return Some("--video cannot be combined with --headless: there is no GPU to take the frames from (use a window, or --offscreen-shot)");
    }
    None
}

/// Refuses the options `--video` cannot work with (before any window is made) and checks that ffmpeg starts.
pub(crate) fn check(args: &DevArgs) {
    if let Some(why) = conflict(args) {
        eprintln!("aa-dev: {why}");
        std::process::exit(2);
    }
    if let Some(out) = &args.video {
        let ok = Command::new(&args.ffmpeg).arg("-version").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
        if !ok {
            eprintln!("aa-dev: --video {out}: cannot run ffmpeg ({}): pass --ffmpeg <path> or set FFMPEG", args.ffmpeg);
            std::process::exit(2);
        }
    }
}

pub fn plugin(app: &mut App) {
    let args = app.world().resource::<DevArgs>().clone();
    let Some(out) = args.video.clone() else { return };
    if let Some(dir) = Path::new(&out).parent().filter(|d| !d.as_os_str().is_empty()) {
        let _ = std::fs::create_dir_all(dir);
    }
    app.insert_resource(Video::new(PathBuf::from(out), args.video_fps, args.ffmpeg.clone()))
        .add_systems(Last, (capture, capture_at_exit.after(OnAppExitSystems)).chain());
}

/// Every app frame: hand out the due ticks, request the next screenshot.
fn capture(mut commands: Commands, args: Res<DevArgs>, gate: Res<RecordGate>, offscreen: Option<Res<OffscreenTarget>>, mut video: ResMut<Video>) {
    let now = Instant::now();
    match video.phase {
        Phase::Stopped | Phase::Finished => return,
        Phase::Waiting => {
            if args.video_gate && !(gate.is_open()) {
                return;
            }
            video.phase = Phase::Running;
            video.start = Some((now, wall_us()));
            info!("aa-dev: video start: t0_wall_us={} fps={}", video.start.map_or(0, |s| s.1), video.fps);
        }
        Phase::Running => {}
    }
    if video.writer.as_ref().is_some_and(|w| w.failed.load(Ordering::Relaxed)) {
        video.error = Some("ffmpeg stopped taking frames".into());
        video.stop("ffmpeg failed");
        return;
    }
    video.emit_due(now, false);
    let elapsed = video.elapsed_us(now);
    if !video.ticks.request_due(elapsed) {
        return;
    }
    // give up on requests that never come back
    let before = video.pending.len();
    video.pending.retain(|_, (_, at)| now.saturating_duration_since(*at) < STALE);
    video.stats.lost += (before - video.pending.len()) as u64;
    if video.pending.len() >= MAX_IN_FLIGHT {
        video.stats.skipped += 1;
        return;
    }
    let id = video.next_id;
    video.next_id += 1;
    let target = match offscreen {
        Some(target) => Screenshot::image(target.0.clone()),
        None => Screenshot::primary_window(),
    };
    let entity = commands.spawn(target).observe(on_captured).id();
    video.pending.insert(entity, (id, now));
}

fn on_captured(mut captured: On<ScreenshotCaptured>, mut video: ResMut<Video>) {
    let entity = captured.entity;
    video.deliver(entity, &mut captured.image);
}

fn wall_us() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_micros() as u64)
}

/// `AppExit` is out (after the systems that raise it): finish the file before the process goes on.
fn capture_at_exit(mut exits: MessageReader<AppExit>, mut video: ResMut<Video>) {
    if exits.read().next().is_some() {
        video.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plays a timeline of app frames and image arrivals (microseconds after the first tick, sorted) and returns the image of
    /// every tick handed out, plus the repeated count.
    fn play(fps: u32, frames: &[u64], arrivals: &[(u64, u64)]) -> (Vec<u64>, u64) {
        let mut ticks = Ticks::new(fps);
        let mut shown = Vec::new();
        let mut next_arrival = 0;
        for &t in frames {
            while next_arrival < arrivals.len() && arrivals[next_arrival].0 <= t {
                ticks.arrived(arrivals[next_arrival].1);
                next_arrival += 1;
            }
            if let Some(run) = ticks.due(t) {
                assert_eq!(run.first_tick as usize, shown.len(), "runs are consecutive");
                shown.extend((0..run.count).map(|_| run.image));
            }
        }
        (shown, ticks.repeated)
    }

    #[test]
    fn tick_grid() {
        assert_eq!(tick_us(30, 0), 0);
        assert_eq!(tick_us(30, 3), 100_000);
        assert_eq!(tick_us(25, 1), 40_000);
        assert_eq!(ticks_due(30, 0), 1, "tick 0 is due at once");
        assert_eq!(ticks_due(30, 33_332), 1);
        assert_eq!(ticks_due(30, 33_333), 2, "tick 1 is at 33333 us");
        assert_eq!(ticks_due(10, 1_000_000), 11);
    }

    #[test]
    fn every_tick_gets_the_newest_image() {
        // 10 fps: ticks at 0, 100, 200, ... ms; app frames every 20 ms; a new image every 100 ms, 10 ms after each tick
        let frames: Vec<u64> = (0..=50).map(|i| i * 20_000).collect();
        let arrivals: Vec<(u64, u64)> = (0..5).map(|i| (i * 100_000 + 10_000, i)).collect();
        let (shown, repeated) = play(10, &frames, &arrivals);
        // tick 0 waits for image 0 (at 10 ms), ticks 1..: image k-1 arrived at 100(k-1)+10, image k at 100k+10 > the tick
        assert_eq!(&shown[..6], &[0, 0, 1, 2, 3, 4]);
        assert_eq!(shown.len(), 11);
        assert_eq!(&shown[5..], &[4; 6], "no new image: the last one repeats");
        assert_eq!(repeated, 6);
    }

    #[test]
    fn a_slow_frame_repeats_the_same_image() {
        // 10 fps, 5 frames of 20 ms then a 500 ms hitch, one image per frame
        let frames = [0, 20_000, 40_000, 60_000, 80_000, 580_000, 600_000];
        let arrivals: Vec<(u64, u64)> = frames.iter().enumerate().map(|(i, &t)| (t, i as u64)).collect();
        let (shown, repeated) = play(10, &frames, &arrivals);
        // tick 0 at 0 (image 0), ticks 1..=5 are due together at 580 ms with image 5, tick 6 at 600 ms gets image 6
        assert_eq!(shown, [0, 5, 5, 5, 5, 5, 6]);
        assert_eq!(repeated, 4);
    }

    #[test]
    fn ticks_before_the_first_image_wait_for_it() {
        // 10 fps: the first image comes at 250 ms, ticks 0..=2 are due by then
        let (shown, repeated) = play(10, &[0, 100_000, 250_000, 300_000], &[(250_000, 0)]);
        assert_eq!(shown, [0, 0, 0, 0], "the first image covers the ticks that were waiting; one tick is the image itself");
        assert_eq!(repeated, 3);
        let (none, repeated) = play(10, &[0, 100_000, 200_000], &[]);
        assert!(none.is_empty());
        assert_eq!(repeated, 0);
    }

    #[test]
    fn an_older_image_arriving_late_does_not_replace_a_newer_one() {
        let mut ticks = Ticks::new(10);
        ticks.arrived(4);
        ticks.arrived(3);
        assert_eq!(ticks.due(0).map(|r| r.image), Some(4));
    }

    #[test]
    fn a_tick_waits_for_the_screenshot_requested_at_its_time() {
        assert_eq!(hold_us(30, 0), 3_000, "no latency known yet: a margin only");
        assert_eq!(hold_us(30, 10_000), 18_000);
        assert_eq!(hold_us(30, 500_000), 33_333 - 3_000, "never more than a tick period");
        assert_eq!(hold_us(240, 100_000), 4_166 - 3_000, "a short period leaves a short hold");
    }

    #[test]
    fn percentiles() {
        let v: Vec<i32> = (1..=100).collect();
        assert_eq!((percentile(&v, 50), percentile(&v, 95), percentile(&v, 100)), (51, 96, 100));
        assert_eq!(percentile(&[7], 95), 7);
    }

    #[test]
    fn one_request_per_tick_period() {
        let mut ticks = Ticks::new(10);
        assert!(ticks.request_due(0));
        assert!(!ticks.request_due(50_000));
        assert!(!ticks.request_due(99_999));
        assert!(ticks.request_due(100_000));
        assert!(!ticks.request_due(150_000));
        assert!(ticks.request_due(1_000_000), "a long hitch asks once, not once per missed tick");
        assert!(!ticks.request_due(1_050_000));
    }

    #[test]
    fn frames_come_out_as_ffmpeg_takes_them() {
        use bevy::asset::RenderAssetUsages;
        use bevy::render::render_resource::{Extent3d, TextureDimension};
        let size = Extent3d { width: 2, height: 1, depth_or_array_layers: 1 };
        let mut bgra = Image::new(size, TextureDimension::D2, vec![1, 2, 3, 255, 4, 5, 6, 255], TextureFormat::Bgra8UnormSrgb, RenderAssetUsages::MAIN_WORLD);
        let frame = frame_from(&mut bgra).unwrap();
        assert!(frame.bgra);
        assert_eq!((frame.width, frame.height), (2, 1));
        assert_eq!(frame.data, [1, 2, 3, 255, 4, 5, 6, 255], "the bytes are not touched");
        let mut rgba = Image::new(size, TextureDimension::D2, vec![1, 2, 3, 255, 4, 5, 6, 255], TextureFormat::Rgba8Unorm, RenderAssetUsages::MAIN_WORLD);
        assert!(!frame_from(&mut rgba).unwrap().bgra);
        let mut short = Image::new(size, TextureDimension::D2, vec![0; 8], TextureFormat::Rgba8Unorm, RenderAssetUsages::MAIN_WORLD);
        short.data = Some(vec![0; 7]);
        assert!(frame_from(&mut short).is_err());
    }

    #[test]
    fn ffmpeg_command() {
        let args = ffmpeg_args(30, 1920, 1080, true, Path::new("out.mp4")).join(" ");
        assert!(args.contains("-f rawvideo -pix_fmt bgra -video_size 1920x1080 -framerate 30 -i -"), "{args}");
        assert!(args.contains("-vf scale=trunc(iw/2)*2:trunc(ih/2)*2 -c:v libx264 -preset ultrafast -crf 18 -pix_fmt yuv420p -r 30 out.mp4"), "{args}");
        assert!(ffmpeg_args(30, 8, 8, false, Path::new("o.mp4")).join(" ").contains("-pix_fmt rgba"));
    }

    /// A fresh directory for the files a test's `Video::finish` writes (`out.mp4.json` and `video.json`).
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aa-dev-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A delivered screenshot of the given size, as the window gives it.
    fn shot(width: u32, height: u32) -> Image {
        use bevy::asset::RenderAssetUsages;
        use bevy::render::render_resource::{Extent3d, TextureDimension};
        let size = Extent3d { width, height, depth_or_array_layers: 1 };
        Image::new(size, TextureDimension::D2, vec![9; (width * height * 4) as usize], TextureFormat::Bgra8UnormSrgb, RenderAssetUsages::MAIN_WORLD)
    }

    #[test]
    fn a_resize_stops_the_capture_cleanly_and_the_sidecar_says_so() {
        let mut world = World::new();
        let (a, b, c) = (world.spawn_empty().id(), world.spawn_empty().id(), world.spawn_empty().id());
        // an ffmpeg that does not exist: the writer thread fails by itself, the bookkeeping is what is under test
        let dir = scratch("resize");
        let mut video = Video::new(dir.join("r.mp4"), 30, "no-such-ffmpeg-for-the-test".into());
        let now = Instant::now();
        video.phase = Phase::Running;
        video.start = Some((now, 1_700_000_000_000_000));
        video.pending.extend([(a, (0, now)), (b, (1, now)), (c, (2, now))]);
        video.deliver(a, &mut shot(4, 2));
        assert_eq!(video.shape.as_ref().map(|s| (s.0, s.1)), Some((4, 2)));
        assert_eq!(video.phase, Phase::Running);
        video.deliver(b, &mut shot(6, 4));
        assert_eq!(video.phase, Phase::Stopped);
        assert!(video.stopped.as_deref().unwrap().contains("changed from 4x2 to 6x4"), "{:?}", video.stopped);
        video.deliver(c, &mut shot(4, 2)); // anything after the stop is ignored
        assert_eq!(video.ticks.latest, Some(0));
        video.finish();
        assert_eq!(video.phase, Phase::Finished);
        let json = video.sidecar(&WriterEnd::default());
        assert!(json.contains("\"stopped\":\"the window changed from 4x2 to 6x4"), "{json}");
        assert!(json.contains("\"width\":4,\"height\":2"), "{json}");
        assert_eq!(std::fs::read_to_string(dir.join("r.mp4.json")).unwrap(), json);
        assert_eq!(std::fs::read_to_string(dir.join("video.json")).unwrap(), json, "video.json next to it");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_image_overtaken_by_a_newer_one_is_ignored() {
        let mut world = World::new();
        let (a, b) = (world.spawn_empty().id(), world.spawn_empty().id());
        let dir = scratch("order");
        let mut video = Video::new(dir.join("o.mp4"), 30, "no-such-ffmpeg-for-the-test".into());
        let now = Instant::now();
        video.phase = Phase::Running;
        video.start = Some((now, 0));
        video.pending.extend([(a, (4, now)), (b, (3, now))]);
        video.deliver(a, &mut shot(2, 2));
        video.deliver(b, &mut shot(2, 2));
        assert_eq!(video.ticks.latest, Some(4));
        assert!(video.pending.is_empty());
        video.stop("test");
        video.finish();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn json_strings_are_escaped() {
        assert_eq!(json_str("a\"b\\c\nd"), "\"a\\\"b\\\\c\\nd\"");
    }

    #[test]
    fn refuses_what_cannot_work() {
        let base = DevArgs::from_env();
        let video = DevArgs { video: Some("v.mp4".into()), record: None, headless: false, offscreen_shot: None, ..base.clone() };
        assert!(conflict(&video).is_none());
        assert!(conflict(&DevArgs { record: Some("d".into()), ..video.clone() }).unwrap().contains("--record"));
        assert!(conflict(&DevArgs { headless: true, ..video.clone() }).unwrap().contains("--headless"));
        assert!(conflict(&DevArgs { headless: true, offscreen_shot: Some("s.png".into()), ..video.clone() }).is_none());
        assert!(conflict(&DevArgs { video: None, record: Some("d".into()), ..video }).is_none(), "nothing to refuse without --video");
    }

    /// The whole pipe with a real ffmpeg (`FFMPEG` or PATH; skipped without one): a dropped tick is filled, the file has
    /// exactly the ticks that were handed out.
    #[test]
    fn writer_makes_a_constant_frame_rate_file() {
        let ffmpeg = std::env::var("FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
        if !Command::new(&ffmpeg).arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success()) {
            eprintln!("skipped: no ffmpeg");
            return;
        }
        let dir = std::env::temp_dir().join(format!("aa-dev-video-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("t.mp4");
        let frame = |shade: u8| Arc::new(Frame { width: 64, height: 48, bgra: false, data: vec![shade; 64 * 48 * 4] });
        let mut writer = Writer::start(ffmpeg.clone(), out.clone(), 10, &frame(0));
        let send = |writer: &Writer, f: &Arc<Frame>, first_tick, count| writer.offer(Msg { frame: f.clone(), first_tick, count }).ok().unwrap();
        let (a, b) = (frame(40), frame(200));
        send(&writer, &a, 0, 3);
        send(&writer, &b, 5, 2); // ticks 3 and 4 never reached the writer
        let end = writer.finish(Duration::from_secs(60));
        assert_eq!(end.exit, Some(0), "{:?} {:?}", end.error, end.tail);
        let probe = Command::new(&ffmpeg).args(["-hide_banner", "-i"]).arg(&out).args(["-map", "0:v:0", "-f", "null", "-"]).output().unwrap();
        let log = String::from_utf8_lossy(&probe.stderr).into_owned();
        let frames = log.rsplit("frame=").next().unwrap().split_whitespace().next().unwrap().parse::<u32>().unwrap();
        assert_eq!(frames, 7, "{log}");
        assert!(log.contains("64x48"), "{log}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_sidecar_has_the_documented_fields() {
        let mut video = Video::new(PathBuf::from("o.mp4"), 30, "ffmpeg".into());
        video.start = Some((Instant::now(), 1_700_000_000_000_000));
        video.shape = Some((1921, 1081, true, "Bgra8UnormSrgb".into()));
        video.ticks.next = 240;
        video.ticks.repeated = 12;
        video.stats.skipped = 1;
        video.stats.ages_us = vec![-2_000, 1_000, 3_000, 40_000];
        let json = video.sidecar(&WriterEnd { exit: Some(0), ..WriterEnd::default() });
        for want in [
            "\"t0_wall_us\":1700000000000000", "\"fps\":30", "\"width\":1920", "\"height\":1080", "\"frames\":240", "\"repeated\":12",
            "\"skipped\":1", "\"dropped\":0", "\"ffmpeg_exit\":0", "\"method\":\"aa-dev --video\"",
        ] {
            assert!(json.contains(want), "{want} in {json}");
        }
        for want in ["\"image_age_ms_mean\":10.5", "\"image_age_ms_p50\":3.0", "\"image_age_ms_p95\":40.0", "\"image_age_ms_max\":40.0"] {
            assert!(json.contains(want), "{want} in {json}");
        }
        assert!(!json.contains("\"error\""));
    }
}
