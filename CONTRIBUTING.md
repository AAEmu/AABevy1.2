# Contributing

Thanks for helping. This project rebuilds the ArcheAge 1.2 client in Rust on Bevy, from the original game files and
the original Lua UI, against an [AAEmu](https://github.com/AAEmu) server. The goal is a client that looks and behaves
like the real one and runs fast.

## Ground rules

- **No game files, ever.** Do not commit anything from the ArcheAge client or its servers' private data: no
  `game_pak` contents (textures, models, sounds, `.alb` scripts, maps), no game database copies, no client
  executables, no extracted samples. Read them at run time through `aa-pak` from the user's own install.
  Real-client screenshots stay on your machine: `reference/**/*.png|jpg` is git-ignored, only the JSON/Lua
  captures beside them are tracked. Screenshots of *this* client go in the PR description, not the repository;
  `docs/screenshots/` only holds the few README images.
- **No secrets.** No passwords, tokens or account names of real servers in code, tests, logs or screenshots.
- **Clean-room behaviour.** Reverse-engineering notes describing formats and behaviour are welcome
  (`docs/research/`). Don't paste decompiled proprietary code.
- **Be kind.** Review the code, not the person.

## Getting set up

You need Windows, a recent Rust toolchain (edition 2024), your own ArcheAge 1.2 client and an AAEmu 1.2 server with
its `compact.sqlite3`. See [Quick start](README.md#quick-start) for the environment variables (`AA_PAK`, `AA_DB`).

```
cargo build -p aa-client
cargo test -p aa-client --bin aa-client <filter>
```

Build and test only the crate you change (`cargo check -p aa-world`, `cargo test -p aa-ui`). The first build compiles
Bevy and takes a while; the dev profile is already optimized.

## How the code is organized

Each crate owns one area (see [Project layout](README.md#project-layout)) and has its notes in `docs/<crate>.md`.
`aa-client` wires the crates together. Keep changes inside the crate that owns the behaviour, and keep crates small:
plain data types, no speculative abstractions.

## Making a change

1. **Open an issue first** for anything larger than a fix, so the approach can be agreed before you write it.
2. **Match the real client.** Compare against the original (the game's own data and scripts, `reference/` captures)
   rather than guessing. Say in the PR what you compared against and what is still approximate.
3. **Show it.** Every visual change should be checkable unattended: an example or test using
   `aa_dev::default_plugins` + `aa_dev::DevPlugin`, run with
   `cargo run -p <crate> --example <name> -- --shot shots/<crate>/<name>.png`. Attach the screenshot and the logged
   `fps=` to the PR. Gameplay changes can use the script runner ([docs](docs/aa-client-script.md)).
4. **Test it.** Add unit tests next to the code. Tests that need the game files or database must skip cleanly when
   they are absent, so CI and contributors without a client still pass.
5. **Document it.** Update the crate's `docs/<crate>.md` and, when you close a gap, `docs/research/remaining.md`.
6. **Keep commits small** with clear messages (`feat(aa-ui): ...`, `fix(aa-world): ...`), and run
   `cargo clippy -p <crate>` once before opening the PR.

## Style

Write code that reads like the code around it: same naming, comment density and idioms. Comments explain why, and
name the original source when behaviour mirrors the client (a script, a table, a file format).

## License

The code is licensed under the [GNU General Public License v3.0 or later](LICENSE). Any contribution you submit for
inclusion is licensed the same way. Code contributed before the switch to GPL is also available under its original
[MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) terms; keep those notices.

ArcheAge and its assets belong to their owners. This is a fan project, not affiliated with XL Games or Kakao Games.
