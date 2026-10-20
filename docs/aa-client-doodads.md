# aa-client: using doodads (Memory Tome, navi doodads, any doodad func with a skill)

Research: `docs/research/portals.md` (1, 1.7, 5), `docs/research/doodad-use-server.md` (what the lane does with each packet).
Code:

| Where | What |
|---|---|
| `crates/aa-client/src/hud/doodad.rs` | the doodad in reach, the dynamic action bar, F / G / H / J, right click, hover name, the interaction list, the packets |
| `crates/aa-net/src/interaction.rs` | `Interact::NaviTeleport` / `NaviOpenPortal` (`CSNaviTeleport` 0x0e7, `CSNaviOpenPortal` 0x0e8, `bc doodadObjId`) |
| `crates/aa-net/src/session.rs` | `SCNaviTeleport` (0x15c, decoded in `portal.rs`) -> `CSTeleportEnded` + `Event::Teleported { yaw: 0, reason: 0 }` |
| `crates/aa-client/src/ingame/combat.rs` | a skill aimed at a doodad is aimed at the doodad's position (the player turns to it) |
| `crates/aa-client/src/hud/combat.rs` | a cast at a doodad does not start the fight ("Battle Start!" only for unit targets) |

## How a doodad is used

There is no "start interaction" packet for doodads (`CSStartInteraction` is NPCs only). The client reads from the game database
what the doodad's **current phase** (`func_group_id` of `SCDoodad(s)Created`, updated by `SCDoodadPhaseChanged`) can do:
`doodad_funcs` rows of that `doodad_func_group_id`.

* A func with a `func_skill_id` is used by casting that skill at the doodad: `CSStartSkill(skill, caster Unit(me), target
  Doodad(objId), flag 0)` = `Command::UseDoodadSkill` (it was already in aa-net). The server's `InteractionEffect` runs
  `Doodad.Use(caster, skill)` -> the current phase's func for that skill. Skills 0 and 10994 are left out (the original
  `dynamic_action_bar.lua` hides them).
* `DoodadFuncNaviTeleport` / `DoodadFuncNaviOpenPortal` without a skill: `CSNaviTeleport` / `CSNaviOpenPortal`. The answer to a
  teleport is `SCNaviTeleport` (a position); like the 1.2 client (`OnNaviTeleport` runs the `SCTeleportUnit` routine with reason 0,
  yaw 0) aa-net answers `CSTeleportEnded` and emits `Teleported`, so `player::follow_server` and the long-jump loading screen
  (`ingame::teleport`) handle it.

`hud/doodad.rs` loads `doodad_funcs` once (`Tables`, in the HUD's `GameData`) as phase -> skills / navi uses, plus the English
doodad names (`localized_texts` of `doodad_almighties`).

## What the player sees (the original scripts)

* **Dynamic action bar** (`x2ui/dynamic_action_bar.lua`, already loaded with the world root scripts): while a doodad with a skill
  use is in reach, `DYNAMIC_ACTION_BAR_SHOW("doodad", objId)` shows its skills as 48 px buttons in the lower middle (2.75/4 of the
  screen height) with the key under each, F G H J (`do_interaction_1..4`); `DYNAMIC_ACTION_BAR_HIDE` when none is. The natives
  behind it: `X2:GetDynamicActionCount`, `GetDynamicActionSkill(i)`, `IsDynamicActionSkillToggled` (false),
  `ExecuteDynamicAction(i)` (a click), `X2Interaction:GetInteractionSkillIconPath(skill)` (the skill icon,
  `ui/icon/interaction/icon_interaction29.dds` for 14602; the script derives `_o` / `_c` / `_d`) and `IsValidIconPath` (pak lookup).
* **Which doodad**: the one last right clicked while it is in reach, else the nearest, a doodad behind the player counting up to 3 m
  further than one in front (an inn's chairs stand around the Crescent Throne tome). Reach: the skill's `max_range` (at least 2 m)
  + 1 m (the range is to the doodad's origin), 4 m without a skill.
* **Keys**: F (G, H, J for the 2nd to 4th button) uses the doodad when no loot is in reach (F loots first, `loot.rs`) and no NPC
  is nearer than the doodad (F talks, `dialog.rs` asks `View::claims_key`).
* **Right click** on a doodad (view ray against a sphere at the doodad, 1.3 m * scale, within 40 m; an NPC in front of it wins):
  one use at once; several (a Portable Sign: Teleport, Open Worldgate) open the original interaction list (`x2ui/interaction.lua`,
  `INTERACTION_START`; `X2Interaction:GetInteractionSkillCount/Id/Name`, `SelectInteraction(id)`, `CancelInteraction`, chained with
  the crafting module's). Out of reach: logged, nothing sent.
* **Hover**: the doodad under the pointer shows its name (`DRAW_DOODAD_TOOLTIP({name})`, hidden with `DRAW_DOODAD_SIGN_TAG()`).
* **The cast**: the server's `SCSkillStarted` drives the existing cast bar ("Setting recall point...", 3 s), the cast clip
  (`start_anim` 126) and the cast effect (fx group 310) like any skill; the player turns to the doodad.
* **The result** comes as server state: the Memory Tome's `SCCharacterReturnDistricts` updates `Portals` (aa-net) and the Teleport
  Book's Default Site moves its recall marker to the new point; `SCDoodadPhaseChanged` moves the tome to its timer phase (no skill
  use: the bar goes to the next doodad or hides) and back after a few seconds.

## Phase models and growth

A doodad is drawn with the model of its **current phase**, not just its template:

* `Db::doodad_phase_model(template, func_group)` (ingame/data.rs) reads `doodad_func_groups.model` and turns it into a
  pak path: `cgf://`, `vegetation://` and `cga://` lose their scheme and `game/`, `prefab://` resolves to the prefab's
  `object_Model` (like actor prefabs). An empty or unusable phase model falls back to `doodad_almighties.model`.
  Results are cached by `(template, func group)`.
* `Db::doodad_growth(func_group)` reads the phase's `DoodadFuncGrowth` (`doodad_phase_funcs` -> `doodad_func_growths`):
  `delay` ms, `start_scale` and `end_scale` (per mille, e.g. 600 -> 0.6).
* `prepare_doodad` (ingame/prepare.rs) prepares the phase model on the worker; a phase change while it runs makes the
  build stale (checked against the server's current `func_group_id`) and it is prepared again.
* `ingame/doodad_visual.rs` stores the drawn path, the phase, the growth, the remaining and the total milliseconds in a
  `DoodadVisual` component. The root scale is `doodad.scale * growth.scale_at(remaining, total)`; `grow` counts the
  announced `time_left` down locally (packets do not tick it). The server's `time_left` is the authority for the total
  duration: a phase change announces the whole remaining time of the new phase (after its growth rate and climate), so
  the growth reaches `end_scale` exactly when the server changes phase. A doodad first seen mid-phase uses
  `max(database delay, time_left)` so it keeps the phase's visible size; the database delay is only the fallback when no
  `time_left` is known (0). If the next phase has a different model with its own growth, the scale restarts at that
  phase's `start_scale`.
* `SCDoodadPhaseChanged` restarts the growth from the packet's `time_left` and, when the phase model differs, swaps the
  model under the doodad root in the same frame: the new model is spawned first, then the old one is despawned (no frame
  without a model). The same path (rice phases, timer phases falling back to the almighty) is not respawned. The root
  keeps its position and `doodad.scale`; only the scale/growth and the child model change.

Examples in the database: seedling 367 phase 2632 -> `sub_tree_bigfir01_grow02.cgf`, growth 0.6 -> 1.0 over 37,032 s;
rice 2246 phase 4403 -> `common_plants_rice01.cgf`, growth 0.75 -> 1.0 over 9,288 s; rice phase 10155 has an empty
model and the invisible phase groups (`ndeco_do_invisible.cgf`) draw nothing.

## Dev flags

* `--doodad-key N T`: press `do_interaction_N` T s after the HUD is up.
* `--doodad-rclick T`: right click the nearest usable doodad within 40 m.
* `--doodad-near T`: log the doodads within 10 m (name, object, template, phase, uses).
* With `--portal-lua T CODE` (portals): `error('bound '..tostring(X2Warp:GetBoundPortalId()))` logs the bound recall point,
  `FAKE.fire_now('TOGGLE_PORTAL_DIALOG', false)` opens the book, `tutorial.window:Show(false)` hides the tutorial box.

## Live on the lane (2026-10-19, account `bevydood1`, character Doodbevy, Nuian)

Memory Tomes near the Nuian start (`doodad_spawns.json` x `doodad_func_bindings`): **850 "Desireen Memory Tome"** at (15673.6,
15176.2, 121.0), 230 m from the spawn, district 1 -> return point 15; **851 "Crescent Throne Memory Tome"** at (15379.9, 13858.4,
159.6) in the inn, return point 17. Reached with `--pos` (the first movement packet moves the server side character too).

```
aa-client --enter-world --account bevydood1 --character Doodbevy --pos 15676.0,15173.6 --yaw 45 --cam-dist 6 \
  --portal-lua 2 "if tutorial.window then tutorial.window:Show(false) end" \
  --portal-lua 4 "error('bound before '..tostring(X2Warp:GetBoundPortalId()))" --doodad-key 1 8 \
  --portal-lua 14 "error('bound after '..tostring(X2Warp:GetBoundPortalId()))" \
  --portal-lua 15 "FAKE.fire_now('TOGGLE_PORTAL_DIALOG', false)" --shot ... --shot-after 27 --exit-after 30
```

```
doodad: 19999 in reach, uses [14602]
portal: --portal-lua: ... bound before 17
doodad: use Desireen Memory Tome (19999): UseDoodadSkill { skill_id: 14602, doodad_obj_id: 19999 }
doodad: cast 14602 at doodad 19999 (result 0, 3000 ms)
doodad: 19948 in reach, uses [15857]                  <- the tome is in its timer phase, a chair is next
doodad: recall point is 15 (3 districts in the book)  <- SCCharacterReturnDistricts
portal: --portal-lua: ... bound after 15
doodad: 19999 in reach, uses [14602]                  <- 4 s later the tome is back in its use phase
```

Server log: `StartSkill: Id 14602, flag 0, caster=21263, target=19999`. The run before bound Crescent Throne the same way
(bound 15 -> 17). fps 85-100 in game (dev build), 60-70 inside the Crescent Throne inn.

Screenshots (`shots/aa-client-doodads/`, git-ignored): `tome_bar.png` (at the Desireen tome, the green interaction icon with "F"
under the player), `tome_cast.png` (the 3 s cast: bar "Setting recall point...", the cast pose, the green cast effect, the open
book on its stand in front of the player), `book_default_site.png` (the Teleport Book after binding: the recall marker on
Desireen, Crescent Throne listed without it). There is no real-client capture of a Memory Tome in `reference/` (portals.md 6
lists `memory_tome` as wanted); the cast bar sits where `reference/ingame/combat_return` has the Recall bar (x 850-1070, y 910-930
here, about x 790-1130, y 920-940 there).

## Not done / known gaps

* **Navi doodads not tried live**: 1034 "Portable Sign" and 1219 "Guide Gate" exist only when a player places one (items 5054,
  8122); 3347 "White Dress Portrait" (skill 15193, a worldgate `Return`) is at (21966, 13075) in Haranya. The packets have unit
  tests only. The interaction list labels of the navi uses ("Teleport", "Open Worldgate") are ours, not the client's texts.
* No highlight / silhouette on the doodad in reach (`doodad_almighties.use_target_highlight`, `use_target_decal`), no walking to a
  doodad that is right clicked out of reach, no gathering / farming UI (labor, tools, the gauge of a harvest are whatever the
  server and the existing cast bar show).
* The bar stays up during the cast (not compared with the real client).
