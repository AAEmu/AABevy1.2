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

`aa-dev: fps=… frame_ms=…` is logged every 2 s.

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
