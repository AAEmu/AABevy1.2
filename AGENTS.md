# AABevy1.2: notes for coding agents

A Bevy (0.20) client for ArcheAge 1.2 that looks and behaves like the original, built from the user's own copy of
the game files (including the original Lua UI) and played on an [AAEmu](https://github.com/AAEmu) server. Keep it
clean and simple: small crates, plain data types, no speculative abstractions.

Humans and agents follow the same rules: read [CONTRIBUTING.md](CONTRIBUTING.md) first. This file adds the
practical details an agent needs to work unattended.

## Layout

| Crate | Owns |
|---|---|
| `aa-pak` | `game_pak` reader (`aa_pak::Pak::global().read("ui/hud.dds")`) and the `aa-pak` CLI |
| `aa-dev` | Dev harness: standard window, fps log, `--shot <png>` screenshot-and-exit |
| `aa-lua` | Lua 5.1 runtime for the original `.alb` bytecode and `toc.g` script loading |
| `aa-ui` | The X2 UI widget toolkit (widgets, anchors, drawables, text, events), bound to Lua, drawn in Bevy |
| `aa-cry` | CryEngine formats: DDS, `.mtl`, `.cgf` / `.chr` / `.skin` meshes and skeletons, shaders |
| `aa-anim` | Animations (`.caf` / `.dba`), `.cdf` characters, character assembly and customization |
| `aa-fx` | The particle system for the game's effect libraries |
| `aa-world` | Terrain, objects, vegetation, water, sky and time of day, streaming, collision |
| `aa-audio`, `aa-midi` | FMOD banks (FSB5 / FEV) and reverb; the music sheets |
| `aa-net` | Login and game protocol client for AAEmu |
| `aa-client` | The app: screen flow, in-game systems, HUD; wires the other crates together |
| `aa-bake`, `aa-web` | The browser build: baked pak, and the server that hosts it |
| `aa-editor` | World editor (spawns, terrain, database rows) |

Each crate has its notes in `docs/<crate>.md`; research notes are in `docs/research/`. Keep a change inside the crate
that owns the behaviour, and update that crate's doc with it.

## Data (never in the repository)

Nothing from the game ships here. Point the tools at your own copies:

| Variable | What |
|---|---|
| `AA_PAK` | The 1.2 client's `game_pak` (read-only; read it through `aa-pak` only, never write to it) |
| `AA_DB` | The AAEmu server's game database `compact.sqlite3` (plain SQLite, read-only; the client's own copy inside the pak is encrypted) |
| `AA_LOGIN` | The login server, default `127.0.0.1:1237` |
| `AA_PASSWORD_FILE` | A file holding the test account's password (else the account name is used) |

* From the shell, `cargo run --release -p aa-pak -- ls ui/login_stage/` lists files, `cat <path>` dumps one,
  `x <prefix> <outdir>` extracts. Build it once and copy the exe to `tools/aa-pak.exe` (git-ignored; the docs
  use that path) so it never waits on the build lock.
* Extracted samples go to `data/extract/<crate>/` and local scratch to `data/local/` (both git-ignored). Keep them
  small and targeted.
* Real-client reference captures live in `reference/<screen>/`. The JSON/Lua widget dumps are tracked; the
  real-client images (`*.png`, `*.jpg`) are local only and git-ignored. Never commit them.
* Some research scripts (`tools/`, `crates/*/tools/`, `*_oracle.py`) read files outside the repository. Their
  paths are placeholders such as `<client>`, `<server>`, `<aaemu-src>` (an AAEmu server checkout) and
  `<re-notes>` / `<client12-unpacked>` (your own reverse-engineering material). Fill them in locally and don't
  commit the result.

## Building and checking

* Build and test only the crate you change: `cargo check -p aa-lua`, `cargo test -p aa-ui`,
  `cargo run -p aa-ui --example login`. Avoid `--workspace` runs: Bevy builds are large.
* Prefer `cargo check -p <crate>` while iterating and run `cargo clippy -p <crate>` once at the end.
* Keep every `Cargo.toml` parseable: an `[[example]]` whose file does not exist yet breaks cargo for the whole
  workspace. Create the file before declaring it.
* Tests that need the game files or the database must skip cleanly when `AA_PAK` / `AA_DB` are missing.
* Every visual change is checkable unattended: an example or test using `aa_dev::default_plugins` +
  `aa_dev::DevPlugin`, run with
  `cargo run -p <crate> --example <name> -- --shot shots/<crate>/<name>.png [--shot-after 8]`.
  Look at the PNG, and report the `aa-dev: fps=…` the console logs every 2 s.
* Compare against the original (the game's own data and scripts, the `reference/` captures) rather than guessing,
  and say what is still approximate.
* Close what you open: use `--shot` / `--exit-after` so Bevy windows don't pile up, and never stop processes you
  did not start.

## Git

Small commits with clear messages (`feat(aa-ui): ...`, `fix(aa-world): ...`), staging only the paths you changed.
No game files, no secrets, no machine-specific paths: use the variables above.
