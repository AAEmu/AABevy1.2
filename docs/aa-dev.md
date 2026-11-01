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
