# aa-client: the loading screen and the way into the game

Code: `crates/aa-client/src/login/loading.rs` (the gate and the gauge), `login/curtain.rs` (the hold over the game
screen), `ingame/warm.rs` (what the world behind the screen has done), `ingame/trace.rs` (the measurement below).

Start on the character list shows the original loading UI (`globalui/loading`). The server spawns the character, and the
world is streamed and drawn behind the screen: the player and the world camera are built as soon as the spawn is known, and
the camera renders into an image. The gauge reports real work:

| Gauge | What it waits for |
|---|---|
| 0..25 | the server's enter-game sequence up to `PlayerSpawn` |
| 25..60 | terrain cells within 500 m of the spawn |
| 60..95 | the share done of the rest: pipelines waiting, object sectors incomplete, units outstanding (each against the most seen at once) |
| 95..100 | everything complete for 0.4 s (`SETTLE`) |
| 100 | the screen is captured and held as the curtain, see below |

"Everything complete" means all of these at once, with nothing new meanwhile (object entities, units announced, pipelines
requested): the near terrain, **all** terrain within the load radius (`Streamer::is_loading`), the placement files of
the cells in reach and the vegetation groups (`ObjectStats::cells_pending`), every brush, entity, tree and grass sector
within its draw distance complete, with no model being prepared, no instance waiting for its model and no level of detail
being rebuilt (`sectors_incomplete`, `instances_waiting`, `models_pending`), the units and doodads near the spawn built
(`Warm::outstanding`), the camera drawn for 8 frames, no pipeline compiling (`PipelineStatus`), and no image added and no
terrain patch rebuilt in the last 0.4 s. `TIMEOUT` (25 s) is the safety net. The log line `loading: waited for (s): ...`
says when each part was last busy.

## The curtain

Part of the game screen only exists in game: the HUD stage (built ahead during loading, swapped in on `Screen::InGame` and
drawn for the first time there), the name plates and the quest marks over the NPCs with their light beams (HUD systems,
aa-fx). At 100 the window is captured (a screenshot) and drawn by the UI camera over everything; the screen switches to the
game underneath. The curtain is lifted when the HUD is up, the world camera is on the window, at least 40 frames and 0.4 s
have passed (the quest marks look every 30 frames) and for 0.3 s no pipeline was new or waiting, no image was added and no
unit was outstanding; at the latest after 6 s. A transparent sprite and a small HDR UI camera drawing into an image compile
the curtain's pipelines for both the plain (loading) and the HDR (game) UI camera while the loading screen is up. The
capture is pixel identical to the live loading screen (checked against `--hold-loading 100`).

## Measured (2026-10-19, dev build, lane server, account `bevyload1`)

`AA_ENTER_TRACE=1` (or `--enter-trace`) logs an `enter-trace:` line per second from the loading screen to 20 s into the
game: frames and worst frame, terrain cells and LOD swaps, object sectors and models pending, units, pipelines, images and
meshes added and on the GPU, UI rebuilds, and the meshes a camera sees for the first time by kind and nearest distance
(`LOAD`, `CURT` = the game screen under the curtain, `GAME` = after the lift). `--enter-shots DIR` saves the window at frames
1, 2, 5, 20, 50 under the curtain and 0, 0.25, 0.5, 1, 2, 4, 8 s after the lift.

```bash
# (a) an existing character, the list up for 8 s
aa-client --account <throwaway> --goto character-select --start-after 8 --enter-trace --enter-shots shots/x --exit-after 60
# (b) a character created and entered right after (the list is back 3 s)
aa-client --account <throwaway> --goto create --race elf --gender female --step 3 --clua "CC.create('Name', 1)" 8 \
    --start-after 3 --enter-trace --exit-after 75
# (c) the second character of the list, never entered, another zone, the list up 2 s
aa-client --account <throwaway> --goto character-select --lua "character-select:X2LoginCharacter:SelectCharacter(2)" --enter-trace
```

Before: the world itself was complete when the game screen came up in every case (no mesh became visible after the switch
except the ones below; the loading screen is held by the pipelines, 7 to 13 s, every time). What came up after the switch,
all of it from the HUD:

* the HUD missing on the first in-game frame, then 25 UI images uploaded and 9 to 11 pipelines compiled (the HDR variants
  of the UI's materials, the plates' text, the skinned quest mark models and their beam particles, tonemapping);
* the quest marks and their light beams 0.25 to 1 s after the switch, the "Welcome" chat line at 1 s;
* a 75 to 100 ms first frame and 5 to 7 frames over 33 ms in the first 5 s (none of these with `--no-hud`).

| Case | Loading before | After: loading + curtain | In game after the lift |
|---|---|---|---|
| (a) existing, list 8 s | 8.9 s | 9.6 + 1.1 s | nothing new visible, no pipeline compiled |
| (b) created, list 3 s | 13.0 s | 13.3 + 1.1 s | nothing new visible, no pipeline compiled |
| (c) other zone, list 2 s | 13.0 s | 15.6 + 1.1 s (machine busier: 45 to 55 fps) | nothing new visible |
| (d) other zone, list 24 s (prewarm settled) | 7.4 s | 7.4 + 0.9 s | nothing new visible |
| `--no-hud` | 14.8 s | 13.9 + 0.5 s | |

Frame rate in the spawn scenes: 70 to 80 fps on the loading screen once the world is in and in game (dev build).

After a create the list comes back with the new character selected (`Bridge::prefer`, the script's `GetLastPlayedCharacterIndex`;
the list is built from the refreshed one that has it), so Start enters it. Before, the server's list has no play time for a
new character, the script took slot 1 and Start entered the first character (seen with `Bevyloadc`: `Bevyloadb` entered).

## Open

* The pipelines are what the loading screen waits for (60 to 120 of them compile behind it at 5 to 10 per second). The
  character list's prewarm (`ingame::prewarm`) follows the selected character (`login::character_select::Selected`, set by
  `ShowSelectedCharacter`): a running one moves the world over when the selection goes more than 400 m away, a settled one
  runs again (about 2 s, the pipelines are compiled already), clicking through characters of one place builds nothing.
* For about 10 s after entering, the HUD changes every frame (aa-ui rebuilds its batches about 70 times a second, about 100
  new meshes a frame) and the game has a 45 to 60 ms frame every one to three seconds; the UI frame itself stays under 5 ms,
  so the spikes are likely the render side of the mesh churn (aa-ui). With `--no-hud` there are none.
