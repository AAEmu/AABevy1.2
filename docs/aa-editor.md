# aa-editor: world editor

`crates/aa-editor` is a world editor built on the client crates: the real streamed world of `aa-world` (terrain,
objects, vegetation, water, sky) with editing tools and egui panels on top. Branch `editor`.

```text
cargo run -p aa-editor                                   # main_world, Nuia start, the bevy lane's data
cargo run -p aa-editor -- --pos 15578,15382 --select 502 --shot shots/aa-editor/select.png --shot-after 20
cargo test -p aa-editor
```

Build it with its own target folder: `CARGO_TARGET_DIR=C:/Users/messe/aa-editor-target cargo run -p aa-editor`.
Worktrees of this repo share artifact hashes in the main `target/`, so building there can break other agents' builds.

## What it edits and where it saves

| What | Read from | Saved to |
|---|---|---|
| NPC / doodad spawns | every `npc_spawns*.json` / `doodad_spawns*.json` in `--spawn-dir` (default: the bevy lane's source `D:/aaemu-wt/bevy/rust/crates/aaemu-game/app/Data/Worlds/<world>`) | the same files. Moved and deleted spawns rewrite their file. Untouched entries keep their exact text and comments. New spawns go to `*_spawns_editor.json`, and the server loads that file as well. The first overwrite of a file keeps `<file>.json.editor-orig`. Saved files are also copied to each `--mirror`, by default the lane's runtime folder `D:/aaemu-live-bevy/rs-game/Data/Worlds/<world>`. |
| Game data: NPC, doodad, item and quest rows and their English names | `--db`, default the lane's copy `D:/aaemu-live-bevy/rs-game/Data/compact.sqlite3`. It falls back to the shared copy, read-only. | the same database: UPDATE, clone (`MAX(id)+1`, texts copied) and delete in one transaction each |
| Terrain heights | the pak (plus the overlay) | `heightmap.dat` per edited cell in the pak overlay `--overlay`, default `data/local/editor/overlay/worlds/<world>/cells/XXX_YYY/client/terrain/` |

The server loads spawns and game data once, at startup. Restart the lane's game server after saving.

**Pak overlay.** `aa-pak` now reads `AA_PAK_OVERLAY=<dir>`: a loose file at `<dir>/<pak path>` replaces or adds a pak
file for `read` and `exists`. The editor sets the overlay to its own folder, so saved terrain shows up when the
editor restarts. Run the remade client with the same variable to play on the edited terrain. The original
`game_pak` is never written. It has no writer yet (encrypted file table), and `D:\AA1.2` is read-only for this project.

## Tools

* **Select (1).** Click a marker or model. Drag the selection over the ground to move it (it snaps to the ground; the
  View panel can turn snapping off). `[` / `]` turn it 15°, Shift + wheel turns it 5°. Del deletes, Ctrl+D duplicates,
  F focuses. The inspector edits UnitId, position, yaw / pitch / roll and doodad scale.
* **Place (2).** Pick an NPC or doodad in the browser, then click the ground. The new spawn faces the camera.
  Shift-click keeps placing.
* **Terrain (3).** Raise, lower, smooth and flatten brushes with a radius and strength. Hold the left button.
  Ctrl+Z undoes a stroke. Edits go into the streamed terrain patches (`TerrainPatch::set_heights` plus a rebuilt
  mesh), so they draw with the real terrain materials and LODs.
* **Browser.** NPCs, doodads, items and quests with English names (`localized_texts`), filtered by search. ✎ or a click
  on an item or quest opens the **row editor**: every column, plus the English name and item description. Changed
  columns are highlighted. Save, revert, clone as new, delete (tick the box first). Quests list their
  `quest_components` steps, and "go to" flies to a spawn of the step's NPC.
* **Play (F5).** A default Nuian character walks the edited terrain: WASD, Shift to run, Space to jump, the right
  mouse button to turn the camera, the wheel to zoom. Esc or F5 returns to the editor camera.
* Undo / redo (Ctrl+Z / Ctrl+Y) covers spawn edits; in terrain mode Ctrl+Z undoes the last stroke. Ctrl+S saves everything.

## Previews

`preview.rs` is simplified from aa-client's `ingame::{creature, doodad_model, housing::prefab}`, which are binary-only
there:

* Doodads: the `.cgf` in `doodad_almighties.model`, or every mesh part of its `prefab://` element.
* NPCs: the actor model of `npcs.model_id` (`models` → `actor_models.model_file`, else a prefab element).
  * A humanoid body is dressed in the character-create default outfit. The NPC's own gear (`equip_*`) is **not**
    resolved yet.
  * A creature `.cdf` / `.chr` gets its skin attachments and an idle animation. Rigid bone attachments are not added.
* `npcs.scale` and the doodad's `Scale` apply.

Models load nearest first, within a 12 ms budget per frame. Spawns within the view radius (default 220 m) are shown;
the rest wait until the camera gets close.

## Coordinates

Spawn files use server / Cry space: metres, Z up, `Yaw` in degrees counter-clockwise from north (forward =
`(-sin, cos)`). This matches `aa-world`'s coordinates (`cry_to_bevy`), so positions need no conversion.

## Gaps

* Static world objects (brushes, vegetation, decals in the cell `.dat` files) cannot be selected, moved or removed yet.
  Only spawns can. Terrain texture painting and holes are missing too.
* The original `game_pak` cannot be written: there is no writer for the encrypted table, and the 24 GB archive is
  read-only here. Exports go to the overlay folder.
* The server keeps its own geodata and heights. Terrain edits change what the client draws and walks on. Server-side
  height checks may still use the old ground.
* Humanoid NPCs wear the default outfit, not their own gear.
* Play mode runs inside the editor, without the server: no NPC AI, no quests.
