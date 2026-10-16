# AABevy1.2: ArcheAge 1.2 client remake in Bevy

Goal: a Bevy (0.20) client that looks and behaves like the real ArcheAge 1.2 client, built from the
original game assets (including the Lua UI), from **world select → character select / create →
in game**, and runs fast. Keep it clean and simple: small crates, plain data types, no speculative
abstractions.

## Layout

| Crate | Owns |
|---|---|
| `crates/aa-pak` | `game_pak` reader (done). `aa_pak::Pak::global().read("ui/hud.dds")` |
| `crates/aa-dev` | Dev harness: standard window, FPS log, `--shot <png>` screenshot-and-exit (done) |
| `crates/aa-lua` | Lua 5.1 runtime for the original `.alb` bytecode + `toc.g` script loading |
| `crates/aa-ui` | X2 UI widget toolkit (widgets, anchors, drawables, text, events) rendered in Bevy, bound to Lua |
| `crates/aa-cry` | CryEngine asset formats: DDS, `.mtl`, `.cgf`/`.chr`/`.skin` meshes + skeletons -> Bevy |
| `crates/aa-anim` | Animations (`.caf`/`.dba`), `.cdf` characters, character assembly + customizing |
| `crates/aa-world` | Worlds: terrain, cell objects, vegetation, water, sky / time of day, streaming |
| `crates/aa-net` | Login + game protocol client against the AAEmu Rust server |
| `crates/aa-client` | The app: screen flow and integration (orchestrator owns it) |

Each agent owns **only its crate** (plus `docs/<crate>.md`). Never edit another agent's crate; if you
need something from it, say so in your final report. The orchestrator wires crates together in
`aa-client`. Add new dependencies to your own `Cargo.toml`; prefer the versions in the root
`[workspace.dependencies]`.

## Data

* Client (read-only, never write): `D:\AA1.2` (`game_pak` 24 GB, 218,066 files). Read through
  `aa-pak` only. From the shell use the prebuilt `tools/aa-pak.exe` (no cargo, so it never waits
  on the build lock): `tools/aa-pak.exe ls ui/login_stage/` lists, `cat <path>` dumps,
  `x <prefix> <outdir>` extracts.
* Pak index as text: `data/pak12.tsv` (`name \t offset \t size`) — grep it to find files.
* Extracted samples go to `data/extract/<your-crate>/` (git-ignored). The UI scripts are already
  extracted under `data/extract/game/scriptsbin/` (`.alb` = Lua 5.1 bytecode, `toc.g` = load order).
* Game database (plain SQLite, read-only): `D:\aaemu-live\rs-game\Data\compact.sqlite3` (679 tables:
  `models`, `item_body_parts`, `custom_face_presets`, `hair_colors`, … ). The client's own copy
  inside the pak is encrypted — use the server one.
* Reverse-engineering notes from the server project (read-only): `D:\aa-re\` — especially
  `D:\aa-re\zone\agent_geom\sub_cgf.md` and `tools\cgf.py` (CGF chunk format).
* Real-client reference captures (screenshots + UI widget dumps): `reference/<screen>/`.

## Building and checking

* One shared target dir (`D:/AABevy1.2/target`, see `.cargo/config.toml`). Cargo's build lock
  serialises parallel agents — that is intended (RAM is shared with other sessions). If you see
  "Blocking waiting for file lock", wait; never `cargo clean`, never point at another target dir.
* Build/test only your crate: `cargo test -p aa-lua`, `cargo run -p aa-ui --example login`. No
  `--workspace` runs.
* Your `Cargo.toml` must always parse: a broken manifest (e.g. an `[[example]]` whose file does
  not exist yet) stops cargo for **every** agent. Create the file before declaring it.
* Every visual deliverable is an example (or test) that uses `aa_dev::default_plugins` +
  `aa_dev::DevPlugin`, so it can be checked unattended:
  `cargo run -p <crate> --example <name> -- --shot shots/<crate>/<name>.png [--shot-after 8]`
  then look at the PNG. The console logs `aa-dev: fps=…` every 2 s — report it.
* Compare against the real client's screenshots in `reference/` and iterate until close.

## Machine rules (shared dev box)

* Other sessions run ArcheAge clients, AAEmu servers and builds on this machine. Never kill
  processes you did not start (`archeage.exe`, `aaemu-*.exe`, `cargo`, `clippy-driver`).
* RAM is tight (~32 GB shared). Close your Bevy windows when done (use `--shot`/`--exit-after`).
* `D:` has limited space: keep extracted data small and targeted.
* The AAEmu Rust server for this project runs in the lane `bevy`
  (`D:\aaemu-wt\bevy`, ports: login 27237, game 27239, stream 27250). Its docs:
  `D:\aaemu-wt\bevy\AGENT-QUICKSTART.md`. Never use the user's MySQL on 3306.
* Git: this repo (`main`). Commit only your own paths (`git add crates/<yours> docs/<yours>.md`),
  small commits with clear messages. If `index.lock` exists, wait and retry.
