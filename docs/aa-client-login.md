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
  "... did not answer in time (connection timeout)"). A missing port means 27237. The screen's own timeout is 25 s
  (the session's request timeout is 20 s, but a connect to a black-holed address is not covered by it). Esc cancels a
  login in progress.
* Back to the screen (`login_screen::back_to_login`, the `Notice` resource carries the reason): World Select's
  Exit Game (`Request::Quit` on `Screen::WorldSelect`), a lost connection or kick or refused login on any login-stage
  screen (`login::route_events`), and the OK button of the HUD's disconnect dialog
  (`hud::session::return_to_world_select`). The password is cleared, the account and server stay. The screen has its
  own Exit button, which quits.
* Remembered: `%APPDATA%\AABevy\login.toml` (else `login.toml` next to the exe; `AA_LOGIN_CONFIG=PATH` overrides):
  `user` and `server`, saved after a successful login. Prefilled server: `127.0.0.1:27237`.
  `--login-type` runs never write it.

## Input widget

Bevy 0.20 has `EditableText` + `ui_widgets::TextInput`, but no password masking, so the three fields are a small
widget of our own (`Field`): click or Tab / Shift+Tab / Up / Down to focus, typing (from `KeyboardInput::text`, so
layouts and key repeat work), Backspace, Delete, Left / Right, Home / End, Ctrl+V (`bevy::clipboard::Clipboard`),
a blinking caret, the password drawn as bullets. Text is drawn as `[before caret][caret][after caret]`, so the caret
needs no glyph measuring; a field with more text than fits scrolls to keep the caret in view. Fonts: the client's
own `ui/font/librebaskerville-bold.ttf` (title, labels, buttons) and `flareserif_821_roman.ttf` (typed text,
messages), read from the pak. The click sound is `button/default_left_up`, hover `button/default_over`, through
`Engine` / `Packs` (`sound/`).

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
cargo run -p aa-client -- --login-screen --login-type bevylogin <throwaway> 127.0.0.1:27237 2 --login-no-connect --shot shots/login/filled.png --shot-after 5
cargo run -p aa-client -- --login-screen --login-type bevylogin <throwaway> 127.0.0.1:1 2 --shot shots/login/error.png --shot-after 6
cargo run -p aa-client -- --login-screen --login-type bevylogin <throwaway> 127.0.0.1:27237 2 --shot shots/login/world_select.png --shot-after 12
```

Use the test account `bevylogin` (the login server creates unknown accounts with the first password sent) and its own
throwaway password; do not put a real password on a command line. Results (debug build, 1920x1080, RX 9070 XT):
about 390 fps on the login screen (uncapped); real keyboard input (Tab, typing, Enter via `SendKeys`), wrong password,
refused connection, bad address, Exit Game back to the screen and the arrival at World Select all checked.
`cargo test -p aa-client --bin aa-client login_screen` runs the address / config unit tests.

## Not done

* No `field.png` / `button*.png` art: fields and buttons are drawn (dark wash, thin gold border).
* No text selection (Ctrl+A, shift-arrows, mouse drag) and no IME composition in the fields; no Ctrl+C.
* The password bullets are small in the Libre Baskerville / Flare Serif glyph; a custom glyph would read better.
* The disconnect text of a kick on a screen other than World Select is generic.
