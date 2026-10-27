# aa-client: the login screen

`Screen::Login` is the first screen of a normal run: the player types an account, a password and a server
(`host[:port]`), presses Connect (or Enter), and after the login the original World Select comes up. It is plain
Bevy UI on its own camera and does not use the Lua UI. Code: `crates/aa-client/src/login/login_screen.rs`
(`login_screen/art.rs` the animated backdrop, `login_screen/config.rs` the saved settings and the address check).

## Flow

* Connect validates the form on the screen (empty account / password, bad address: the field is focused and the line
  under the fields says why), writes the values into `Credentials`, and sends `Command::Login`. The password goes
  through `aa_net::Secret`; it lives in the form and in `Credentials` only, is never logged and never saved.
* `LoginSucceeded` / `WorldList`: fade to black (0.5 s), then `Screen::WorldSelect` once the session is in
  `Phase::WorldSelect`. The login music (`login_stage_music_before_login`) is the same track as World Select's and
  goes on across the change.
* Errors stay on the screen and the form is editable again: `LoginFailed` / `ConnectFailed` / `Disconnected` are shown
  in plain words ("Login denied: wrong account name or password.", "Cannot reach the login server HOST:PORT ...",
  "... did not answer in time (connection timeout)"). A missing port means 1237. The screen's own timeout is 25 s
  (the session's request timeout is 20 s, but a connect to a black-holed address is not covered by it). Esc cancels a
  login in progress.
* Back to the screen (`login_screen::back_to_login`, the `Notice` resource carries the reason): World Select's
  Exit Game (`Request::Quit` on `Screen::WorldSelect`), a lost connection or kick or refused login on any login-stage
  screen (`login::route_events`), and the OK button of the HUD's disconnect dialog
  (`hud::session::return_to_world_select`). The password is cleared, the account and server stay. The screen has its
  own Exit button, which quits.
* Remembered: `%APPDATA%\AABevy\login.toml` (else `login.toml` next to the exe; `AA_LOGIN_CONFIG=PATH` overrides):
  `user` and `server`, saved after a successful login. Prefilled server: `$AA_LOGIN`, else `127.0.0.1:1237`.
  `--login-type` runs never write it.
* The loading screen after Start (what it waits for, the curtain that holds it over the game screen until the HUD,
  the name plates and the quest marks are in, and the measurements) is in [aa-client-loading.md](aa-client-loading.md).

## Character-list metadata

The original character-list API also preserves received lobby headers in `Bridge`:
extra slots, payment metadata and account premium points remain optional until
received on the current game connection. `GetExpandedCharacterCount` reads the
received extra slots; `GetCurrentTotalCharactersLimit` adds them to the login
server's base limit, widening before addition. Unknown extras retain the prior
getter fallback of zero, while `NET.account().expanded_characters` remains nil.
The base and expandable-maximum getters remain distinct. Native slot handler
391e1740 and getters 394cd0b0/394ccef0 establish this mapping. Slot receipt emits
no new UI event; the existing original `LOGIN_CHARACTER_UPDATED("refreshed")`
path updates the character list. Payment and premium stay typed metadata here,
without entitlement inference or replacing each character's received points.
See [network receipt contract](aa-net.md#lobby-header-receipts-and-payment-tail-alignment-2026-10-25).

## Input widget

Bevy 0.20 has `EditableText` + `ui_widgets::TextInput`, but no password masking, so the three fields are a small
widget of our own (`Field`): click or Tab / Shift+Tab / Up / Down to focus, typing (from `KeyboardInput::text`, so
layouts and key repeat work), Backspace, Delete, Left / Right, Home / End, Ctrl+V (`bevy::clipboard::Clipboard`),
a blinking caret, the password drawn as bullets. Plain and selected runs are split around the caret;
their shaped Bevy layouts also supply pointer hit positions. A field with more text than fits scrolls to keep
the caret in view. Fonts: the client's
own `ui/font/librebaskerville-bold.ttf` (title, labels, buttons) and `flareserif_821_roman.ttf` (typed text,
messages), read from the pak. The click sound is `button/default_left_up`, hover `button/default_over`, through
`Engine` / `Packs` (`sound/`).

Ctrl+A selects the focused field. Shift+Left/Right and Shift+Home/End extend or reverse the selection;
ordinary arrows collapse it. Typing, paste, Backspace and Delete replace/remove the selection on Unicode
character boundaries, including when the field was at its length limit. Selected runs have a teal highlight,
and scrolling measures both runs before the caret. Ctrl+C and Ctrl+X copy/cut selected account or server
text; Windows uses the same Unicode OS clipboard backend as the original-Lua edit controls, fixing the
previous process-local Bevy clipboard fallback. The masked password field keeps copy and cut inactive.
All password runs, including selected text,
remain bullets. Editing and selection shortcuts stay disabled during connection/fade-out.

The offline `login_selection` example renders this production form with synthetic text and no Session or
settings writes. `--password-selection` and `--reverse-selection` exercise the masked and left-caret controls.

## Art and animation

`crates/aa-client/assets/login/` (`README.md` there describes the manifest contract): `bg_far.png`, `bg_mid.png`,
`mist_a.png`, `mist_b.png`, `glow.png`, `panel.png`, `manifest.json`. `art.rs` lays out the 2560x1440 design canvas
scaled to cover the window and animates the manifest: mouse parallax (eased), drifting `tile_x` mist that wraps,
breathing scale / alpha, seeded motes (rise, sway, fade, prewarmed; drawn 3x the manifest's core `size_px` so the
glow reads), and the fixed panel. Fade in 0.9 s on enter, fade out on success. Layers are `ImageNode`s whose `Node` is
set every frame, so resizing needs no rebuild (only the form is rebuilt for the new scale).

The art is read through an extra asset source named `login` (`login_screen::register_assets`, called by `main`
before the asset plugin; bevy's `png` feature is on for aa-client). The folder is found next to the exe
(`assets/login`), else in the source tree, so the exe can run from anywhere (`AA_LOGIN_ASSETS=DIR` overrides). Without
the folder or without a file the screen is a dark teal gradient with a plain gold-edged panel.

## Flags and skipping

| Flag | Effect |
|---|---|
| (none) | login screen first |
| `--size`, `--mute`, `--sound` only | same: these do not skip it |
| any other `--flag` (`--account`, `--goto`, `--enter-world`, `--create-offline`, `--shot`, `--exit-after`, `--lua`, ...) | screen skipped: the client logs in at startup and starts on World Select exactly as before |
| `--login-screen` | show it anyway; `--account` / `--login` prefill the account and server |
| `--login-type USER PASS HOST T` | (implies the screen) type the values at T seconds and press Connect; once per run |
| `--login-no-connect` | with `--login-type`: type but do not press Connect |

`--enter-world` always wins (no login screen). `login_screen::enabled()` is the one switch; when it is off nothing of
the screen is built and Exit Game / disconnects behave as before.

## Checks

```text
cargo run -p aa-client -- --login-screen --shot shots/login/screen.png --shot-after 6
cargo run -p aa-client -- --login-screen --login-type bevylogin <throwaway> 127.0.0.1:1237 2 --login-no-connect --shot shots/login/filled.png --shot-after 5
cargo run -p aa-client -- --login-screen --login-type bevylogin <throwaway> 127.0.0.1:1 2 --shot shots/login/error.png --shot-after 6
cargo run -p aa-client -- --login-screen --login-type bevylogin <throwaway> 127.0.0.1:1237 2 --shot shots/login/world_select.png --shot-after 12
```

Use the test account `bevylogin` (the login server creates unknown accounts with the first password sent) and its own
throwaway password; do not put a real password on a command line. Results (debug build, 1920x1080, RX 9070 XT):
about 390 fps on the login screen (uncapped); real keyboard input (Tab, typing, Enter via `SendKeys`), wrong password,
refused connection, bad address, Exit Game back to the screen and the arrival at World Select all checked.
`cargo test -p aa-client --bin aa-client login_screen` runs the address / config unit tests.

### Keyboard selection checkpoint, 2026-10-25

Eight new checks cover multibyte replacement at capacity, selection reversal/deletion, all masked runs,
actual keyboard-to-highlight delivery, busy-state rejection and batched modifier releases. Keyboard input
reconstructs the batch's starting physical Ctrl/Shift state, then replays transitions in order; releasing
one side does not clear the other. An independent second review found no remaining actionable issue.

The final application suite passes **771 tests, zero failures, 91 optional checks ignored** (144.21 s).
The main executable and `login_selection` example build with the original toolkit double-click and
vegetation LOD wind commits. One scoped client Clippy run exits 0; its 183 warnings include existing
art/mouse warnings, with none in the new field/fixture/tests or changed selection logic.

All four final captures were inspected and self-closed with exit 0 and no runtime WARN/ERROR: account and
reverse selection at 1280x720, masked selection at 1920x1080, and normal idle startup at 1280x720.
The component runs report 340.6–422.2 FPS; normal startup reports 237.7–281.2 FPS on the shared machine.
These verify this custom form, not a retail login comparison or authenticated session. The Windows
clipboard calls use the existing toolkit backend; real clipboard success/failure was not exercised by
the headless checks. Logs, source/executable hashes and capture observations are retained in
`data/local/nonmount-20261025/checkpoint.json`; PNGs are under `shots/nonmount-20261025/`.

### Pointer selection checkpoint, 2026-10-25

Click places the caret using the actual shaped text runs; dragging retains the original field and Shift-click
extends its anchor. Password bullet positions map back to source character boundaries. Pending placement
survives mouse release while layout updates, and release applies its final endpoint. Losing primary-window
focus cancels capture and clears the held left button. Field revisions retire captured pointer work after
keyboard navigation, selection or replacement. Scrolling preserves a visible click position and clamps to
the complete row width when long text is replaced by a short value.

The final 35-phase production-form fixture passes at 1280x720, 1920x1080 and 1920x1080 with `UiScale(1.1)`.
It exercises Unicode click/drag, masked mapping, Shift-click, scroll/reveal, short replacement, pending
release, final release movement, focus loss, and Home/Ctrl+A arriving with mouse-up. No Connect/Exit action
is queued. All three captures and normal idle main startup were inspected, self-closed with exit 0 and
have no runtime WARN/ERROR. Fixture samples are 283.4–387.8 FPS; idle startup is 237.6–288.3 FPS under shared load.
Run `login_selection --pointer-check` for these assertions; `--ui-scale 1.1` selects the scale control.

The final application suite passes **773 tests, zero failures, 91 optional checks ignored** (142.05 s),
including all 12 login checks. The main executable builds; one scoped bin Clippy run exits 0 with 182
existing warnings and none in the new pointer/field/fixture logic. Three independent review rounds close
the discovered coordinate, release, focus, scroll-bound and keyboard-retirement defects. Logs, source and
artifact hashes are retained in `data/local/nonmount-20261025/pointer-checkpoint.json`; final PNGs use
`pointer-final-*.png` and `pointer-main.png` in `shots/nonmount-20261025/`.

## Not done

* No `field.png` / `button*.png` art: fields and buttons are drawn (dark wash, thin gold border).
* IME composition and word-level pointer selection remain open.
* The password bullets are small in the Libre Baskerville / Flare Serif glyph; a custom glyph would read better.
* The disconnect text of a kick on a screen other than World Select is generic.

## Art flicker

The art layers move and breathe by fractions of a pixel. Bevy UI rounds node layouts to whole pixels, so the
layers jumped a pixel at a time and every change of the rounded size resampled the whole 2560x1440 picture, which
is drawn smaller than that and had no mips: the line work flickered every second or two. The art node has
`LayoutConfig { use_rounding: false }`, and `art::mipmaps` gives every image of the `login` source a mip chain
(2x2 box filter in linear light, weighted by alpha) with a trilinear sampler when it has loaded.
`AA_PRESENT=vsync|novsync|fifo|mailbox|immediate` sets the window's present mode (the default stays uncapped).
