# aa-dev: the dev harness

`aa_dev::default_plugins(title)` (the standard window) plus `aa_dev::DevPlugin` (FPS log, screenshots, recording,
headless support). Every example and `aa-client` use them. `default_plugins_with_log(title, layer)` replaces the
`LogPlugin` custom layer; the client passes one that composes this capture layer with its log file
(`crates/aa-client/src/logging.rs`). The flags are parsed from `std::env::args` (`DevArgs`).

| Flag | Meaning |
|---|---|
| `--size WxH` | window size (default 1920x1080) |
| `--shot FILE.png`, `--shot-after S` | screenshot of the primary window after S seconds (default 5), then exit |
| `--exit-after S` | exit after S seconds |
| `--record DIR`, `--record-fps`, `--record-from`, `--record-secs`, `--record-gate` | one PNG per frame at a fixed time step (`record.rs`) |
| `--video OUT.mp4`, `--video-fps N`, `--ffmpeg PATH`, `--video-gate` | wall-clock video of the window through ffmpeg, frames taken from inside the app (`video.rs`, see Video) |
| `--headless`, `--headless-fps N` | no window and no GPU, a timed schedule loop (default 30 Hz) |
| `--offscreen-shot FILE.png` | headless with the GPU: the window cameras draw into an image that is saved |
| `--allow-lua-error TEXT` | do not count Lua errors that contain TEXT (see `capture.rs`) |
| `--require-wall-clock` | every frame after 1 s of wall time, exit non-zero when the app clock is not the wall clock (see Clock); also keeps a windowed run continuous |

`aa-dev: fps=… frame_ms=… clock=wall|manual game_per_wall=… real_per_wall=…` is logged every 2 s.

## Clock

Bevy's `Time` (the app clock, `Time<Virtual>`) normally follows `Time<Real>` (the wall clock): `game_per_wall` and
`real_per_wall` are 1, so a duration measured on `Time` is a real duration. `--headless` only caps the loop rate
(`--headless-fps`, which defaults to 30); it does not change the clock. The script runner's `wait`, its timeouts and
combat's visual schedule read the same `Time`, so a comparison or replay script must not run on a manual step.

`--record` and deterministic replay examples (`crates/aa-client/examples/remote_vehicle_motion.rs`) set
`TimeUpdateStrategy::ManualDuration`: every frame advances the app clock by exactly the step, however fast the loop
runs. Their timings are game time. The 2 s log line and the final `recorded … frames` line report the measured
`clock`, `game/wall` (Virtual) and `real/wall` (Real) ratios, so a tool that prefers the fixed step can divide its
timings by it.

A run that claims wall time passes `--require-wall-clock` (`clock.rs`):

* After a 1 s wall warm-up, every frame fails at once when `TimeUpdateStrategy` is not `Automatic`,
  `Time<Virtual>::relative_speed` is not 1, or `Time<Virtual>` is paused. It does **not** latch: a manual clock
  installed after login, `--record --record-from`, or `AA_COMBAT_SLOWMO` (which only appears on the first skill the
  player fires) is caught on the frame it appears.
* `Time<Real>` is compared with the wall clock directly (a manual strategy drives Real too), and game time over a
  rolling 3 s window of per-frame deltas. A frame whose `Time<Virtual>` delta hit `max_delta` (250 ms) was clamped by
  a hitch and is left out with its wall interval, so a slow machine or an unfocused window is not a failure; frame
  offsets at 15-30 fps cannot distort the ratio either. A frame whose `Time<Real>` delta differs from the wall delta
  by more than 50 ms is the render thread's lag (or catch-up) after a stall and is left out of both windows. `Time<Real>`
  fails only when it runs **faster** than wall; a lag is not the bug. Game time **faster** than wall time beyond 5 %
  fails (the direction that inflates recorded timings); slower is logged as a warning.
* The final ratio is judged once more at `AppExit`, so a drift after the last update is not lost. The run also
  becomes continuous when windowed, so an unfocused window is not throttled.

The regression tests drive the systems with a hand-stepped wall clock and render-time channel (no sleeps): a fixed
step or `set_relative_speed` installed after the warm-up, the original `ManualDuration(1/60)` at a 120 fps loop, a
400 ms hitch that clamps game time, a delayed render instant (stall and catch-up), a slower Real or game clock
(warning), and a manual clock judged only at exit.

## Video

`--video OUT.mp4` films the primary window while the app clock stays the wall clock, so it works with `--require-wall-clock`
and a live server (`--record` is a fixed time step and PNGs; the two cannot be combined, nor can `--video` and `--headless`:
the run is refused with a message). The frames come from `Screenshot`s taken inside the app (a capture of the desktop from
outside is useless when it is locked) and go over a bounded queue to a writer thread, which feeds raw RGBA/BGRA frames (the
window's actual `TextureFormat`, no conversion for the 8-bit ones) to `ffmpeg -f rawvideo ... -c:v libx264 -preset ultrafast
-crf 18 -pix_fmt yuv420p` (even output size). `--video-fps` defaults to 30, `--ffmpeg` to the `FFMPEG` variable, then
`ffmpeg` on PATH; a missing ffmpeg is refused at start. Unset, nothing is added to the app.

* **Tick grid.** Tick `k` is at wall time `t0 + k / fps`; `t0` is `SystemTime::now()` (Unix microseconds) at the first app
  frame of the capture, the grid itself runs on the monotonic clock from there. Each app frame, every tick that is due gets
  the newest delivered image (a tick that reuses an image is `repeated`); a new screenshot is requested at most once per
  tick, and not while 3 are pending (`skipped`). A screenshot is delivered a few app frames after it was requested, so a
  tick is handed out a *hold* after its time (1.5 times the measured delivery latency plus 3 ms, at most one tick period): the
  frame then shows the world as it was at the tick instead of one tick earlier (the writer only needs the order, so this costs
  nothing). Ticks due before the first image arrived get that first image. Frame `k` of
  the file is tick `k`: ticks the writer queue could not take (`dropped`) are filled with the previous frame, so the file
  keeps its constant frame rate and its clock. The tick bookkeeping is a pure type (`Ticks`) with unit tests.
* **Start.** The first app frame, or with `--video-gate` the moment `RecordGate::open` (as for `--record-gate`) or
  `aa_dev::open_video_gate()` is called. The script runner calls the latter for a `trace video-start` (or `echo video-start`)
  line, so a script chooses where the video begins and a login is not filmed.
* **Exit.** On `AppExit` (`--exit-after`, the end of a script, a closed window) the due ticks are flushed, ffmpeg's stdin is
  closed and ffmpeg is waited for (30 s, then killed) before the process goes on, inside the exit itself (unlike `--record`,
  whose `--exit-after` polls `Recorder::busy`, nothing has to wait for it). `OUT.mp4.json` and `video.json` next to it hold `t0_wall_us`, `fps`, `width`, `height`
  (of the file), `frames`, `repeated`, `skipped`, `dropped`, `ffmpeg_exit`, `method` (`"aa-dev --video"`), plus `format`
  (the texture format), `lost` (requests that never came back and were given up on after 5 s), `latency_ms` (smoothed
  request-to-delivery time), `image_age_ms_mean` / `_p50` / `_p95` / `_max` (the tick's time minus the time the image on it was
  requested: what the frame shows is that much older than its tick; about 0 when the hold works, large while the app runs
  slower than the grid or at the start, before the first screenshot is back), and `stopped` / `error` when something ended it.
* **Limits.** A window resize (or a change of pixel format) while recording stops the capture cleanly: the file ends at the
  last frame of the first size and `stopped` says why (ffmpeg's input size is fixed). With `--offscreen-shot` the offscreen
  image is filmed (the loop then runs at `--headless-fps`). Native only. ffmpeg runs next to the game and takes CPU; the
  3 pending screenshots and the queue hold at most about a dozen full-size frames.

## Headless

`headless.rs`, `lib.rs::default_plugins`: with `--headless` the plugin group drops `WinitPlugin`, keeps the primary
`Window` as a component (`WindowPlugin { exit_condition: DontExit }`; the Lua UI reads its size), adds
`ScheduleRunnerPlugin::run_loop` and, unless `--offscreen-shot` needs the GPU, sets `RenderPlugin` to
`WgpuSettings { backends: None }` (no renderer at all; the crates already tolerate a missing `RenderApp`).
`--offscreen-shot` retargets every camera whose `RenderTarget` is the primary window to an `Image`
(`Rgba8UnormSrgb`, window size, `COPY_SRC`) and saves it with a `Screenshot::image` request. A script runner sets
`aa_dev::SCRIPTED` so the process stays alive after the first shot.

## Log capture

`capture.rs` is the `LogPlugin` custom layer. It watches WARN and ERROR records for Lua errors of the UI scripts
(`aa_dev::capture::lua_errors()`) and counts them (`counts()`), so an unattended run can fail on them. The script
runner (`docs/aa-client-script.md`) is the user.
