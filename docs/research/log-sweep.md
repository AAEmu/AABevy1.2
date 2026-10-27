# Log sweep of the full client flow

Date: 2026-10-17. Build: `target/debug/aa-client.exe` (built 13:23 local), run as the copy `target/agents/sweep/aa-client.exe`.
Account: `bevysweep`, character `Sweeprunner` (id 9, Nuian male, zone 179, Lane-bevy), created by the aa-net walkthrough.
Logs: `shots/sweep/*.log` (ANSI codes stripped when read). Screenshots: `shots/sweep/a_worldselect.png`, `b_charselect.png`, `c_create.png`.
All six runs exited on their own with code 0. None timed out or was killed. Free RAM before each run was 9.1 to 10.4 GB.

## Notes on this sweep

- **Run c used the forbidden account `bevytest` on its first launch.** The step-c command has no `--account`, so the client defaulted to `bevytest` and logged in once. That log is kept as `shots/sweep/c_create_bevytest_DO_NOT_USE.log` (and `.png`) and is excluded from every count below. Run c was rerun with `--account bevysweep`, and the results below come from that rerun. The step-c command in the sweep spec should get `--account bevysweep`.
- Lines containing `failed 0` match the spec's `failed` grep, but they are counters with zero failures (`built N, failed 0`). They are listed as benign and are not problems.
- `0 script errors` lines do not match the spec's grep (`ERROR` is case-sensitive). They report zero errors and are not problems.
- Run e (`--hud-open toggle_*`) prints no log line at all, so the sweep cannot confirm the toggles fired.
- Lua `print()` output is logged at INFO level and is noisy (see Top 15, item 12).

## Runs

| Run | Scenario | Log | Exit | WARN / ERROR / panic / FALLBACK |
|---|---|---|---|---|
| a | world select | `a_worldselect.log` | 0 | 0 / 0 / 0 / 0 |
| b | character select | `b_charselect.log` | 0 | 0 / 0 / 0 / 0 |
| c | create, step 3 (elf female) | `c_create.log` | 0 | 1 / 0 / 0 / 0 |
| d | in game, spawn | `d_ingame.log` | 0 | 0 / 0 / 0 / 0 |
| e | in game, HUD toggles | `e_hud.log` | 0 | 0 / 0 / 0 / 0 |
| f | leave to character list, re-enter (x2) | `f_leave_again.log` | 0 | 0 / 0 / 0 / 0 |

## Problems per run

Grep: `WARN`, `ERROR`, `panicked`, `FALLBACK`, `not found`, `missing`, `failed`, `unsupported`.

### a. World select

No matching lines.

### b. Character select

No matching lines.

### c. Create, step 3

| Count | Message | Crate |
|---|---|---|
| 1 | `WARN aa-fx: effect abillity_skill_table_m.login_stage_effect.elf_fog_01 not found in library abillity_skill_table_m` | `aa_fx` |

### d. In game, spawn

No WARN, ERROR, or other problem lines. Benign counters: 11 lines of `in game: player ... server units N (drawn N, built N, failed 0)` from `aa_client::ingame`.

### e. In game, HUD toggles

No WARN, ERROR, or other problem lines. Benign counters: 6 lines of the same form from `aa_client::ingame`.

### f. Leave and re-enter

No WARN, ERROR, or other problem lines. Benign counters: 13 lines of the same form from `aa_client::ingame`.

## FALLBACK lines

None in any run, including a case-insensitive check. The UI APIs are not reporting fake data through this log.

## Panics

None in any run.

## fps (`aa-dev: fps=` lines)

| Run | Samples | Min | Max | Last | Note |
|---|---|---|---|---|---|
| a world select | 5 | 288.4 | 335.7 | 322.1 | |
| b character select | 8 | 97.2 | 155.3 | 149.8 | |
| c create, step 3 | 8 | 34.2 | 138.1 | 74.4 | step 3 runs at 34 to 76 fps |
| d in game, spawn | 29 | 0.4 | 96.9 | 21.0 | min 0.4 is the frame-2 hitch (2847 ms); next lowest is 3.0 |
| e in game, HUD | 17 | 5.6 | 98.4 | 90.3 | |
| f leave and re-enter | 34 | 2.8 | 99.7 | 66.0 | in-game dips to single digits recur |

In game (d, e, f) the fps repeatedly drops to 3 to 10 for a second or two and then recovers. Frame times of 90 to 340 ms recur every few seconds. The logs do not attribute these frames to a cause.

## Top 15 to fix

Ranked by how visible or frequent each problem is. Only item 4 is a logged WARN. The rest come from the flow and timing lines (`ingame::flow`, `hud`, `prewarm`) or from the Lua and UI logging.

1. **In-game frame stalls.** fps drops to 3 to 10 repeatedly in every in-game run, with 90 to 340 ms frames. Crate `aa_client` (`ingame::flow`). Very visible.
2. **First in-game frame hitch on every entry.** Frame 2 takes 768 to 2847 ms (d 2847, e 1068, f 1161, 796, 768). Prewarm runs only from character select (run b, settled after 5.7 s), not on the enter-world path. Crate `aa_client` (`ingame::flow`, `ingame::prewarm`).
3. **Create step 3 is slow.** fps falls to 34 to 76 on the create screen, against 288 to 336 at world select. Crate `aa_client` (`login::create::scenery`). The cost is not attributed to a specific crate in the log.
4. **Missing elf fog effect on create step 3.** `WARN aa_fx: ... elf_fog_01 not found in library abillity_skill_table_m`. The only WARN in the sweep. Crate `aa_fx`. Visual impact not checked.
5. **HUD toggles leave no trace.** The `--hud-open toggle_bag / character / worldmap / spellbook / quest` run (e) logs nothing, so the toggles cannot be verified. Crate `aa_client` (`hud`).
6. **Units in view are not all drawn.** `server units 18 (drawn 13, built 13)` in run d; `17 (drawn 12)` and `16 (drawn 11)` in run f. Five units are never drawn and nothing is logged to say why. The cause may be dead NPCs or distance culling; this is unverified. Crate `aa_client` (`ingame`).
7. **Ability list mismatch.** The walkthrough reports abilities `[1, 11, 11]` for this character, but the client HUD logs `hud: abilities [1]` on every entry. Crate `aa_client` (`hud::api`). Needs a check against the server data.
8. **Spawn facing.** The walkthrough's first entry reported `yaw 2.79`, but the client spawns with `facing 0.00` in runs d, e, f. The walkthrough moved the character before logging out, so the stored yaw may have changed. Verify. Crate `aa_client` (`ingame::enter`, `ingame::player`).
9. **Enter-world data load.** `hud: game data in 615 to 1077 ms` (15126 skills, 21482 items, 9816 buffs, 13382 npcs) on every entry. Crate `aa_client` (`hud::data`).
10. **In-game UI stage ready.** `login: UI stage ready` takes 621 to 1615 ms on every in-game entry (d 1615, e 695, f 656 / 631 / 621). Crate `aa_client` (`login::host`).
11. **Terrain layer load.** `terrain layers of main_world` takes 0.3 to 0.9 s on every entry. Crate `aa_world` (`terrain_layers`).
12. **Lua output floods the log.** INFO lines such as `CurrentState : None`, `SetState : Step1`, `*_selected`, `ENTERED_WORLD_SELECT`, and `SHOW_SERVER_SELECT_WINDOW` (logged twice in world select) hide real warnings. Crate `aa_ui` (`lua::globals`).
13. **Sky box dump at INFO on every entry.** A multi-line debug struct of the sky box handles is logged each time. Log noise. Crate `aa_world` (`sky`).
14. **UI texts read from a dev-box absolute path.** `aa-ui: 6325 UI texts from <server>/Data/compact.sqlite3`. Works here but breaks on any other machine. Crate `aa_ui`.
15. **Create-offline still goes through the login server.** Run c logs in, enters world select, and then opens the create screen. This may be intended. Crate `aa_client` (`login`).
