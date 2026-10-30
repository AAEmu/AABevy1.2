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

## Doodads attached to a unit or a house (the farm wagon's storage crates)

A doodad with `parent_obj_id != 0` is drawn under its parent root (`units::doodad_root_transform`,
`units::parent_entity`) and follows it. The server writes the `SCDoodadCreated` position in the parent's frame when
`attach_point > 0 || parent_obj_id > 0` (AAEmu `models/doodad.rs` ~260); the client keeps it as the child's local
`Transform`, so the world transform is parent * local. A parented doodad waits until the parent root is drawn (a
unit's, or a house's through `DrawnHouses`) instead of drawing the local numbers as a world position, and it goes away
with the parent because Bevy despawns `ChildOf` descendants. The logout/world-switch and `units::sync_views` sweeps
therefore despawn doodads with `try_despawn`: the parent's recursive despawn may already have removed the child, and
commanding it again would warn about a missing entity. Ordinary world doodads keep world placements.

House decor and bridges parent to a house unit (`parent_obj_id` = the house). `housing::render` draws a house root at
its unit's position and yaw with **scale 1**, not the housing unit's `scale`; `units::parent_placement` uses that same
root transform, and `units::doodad_world_in` falls back to the drawn root in `DrawnHouses` while the housing unit is
no longer in `Units`. `units::doodad_world` is the no-houses variant the HUD and skill readers use.

Every reader that needs the doodad's world position (the HUD's reach/pick/tooltip/coffer, a skill aimed at a doodad,
planting room, the script runner, the mailbox reach and a cut-down tree's fall direction) uses `units::doodad_world`,
which applies the parent's placement, scale included (houses: 1), exactly as the `ChildOf` root does; a parented doodad
whose parent is not announced has no world position and is skipped. A seat bond on a parented doodad
(`seating::presentation::follow`) composes the parent root's `GlobalTransform` with the doodad's local `Transform`.

## The use-target highlight

`hud/doodad.rs` writes the doodad the use bar is about and the one under the pointer into `DoodadHighlights`, leaving
out templates whose `doodad_almighties.use_target_highlight` is off (`GameData::doodad.glows`). `doodad_visual::highlight`
gives each wanted doodad's meshes a private copy of their material with a warm emissive tint and restores the original
handles when the doodad leaves the list. The copy is complete (`StandardMaterial` or `IllumMaterial`, all maps kept).
The tint only touches the emissive term; when the material had no emissive map the glow is modulated by its diffuse
map, because a flat emissive over a dark prop (the cart's storage crate) washed the wood out to a pale panel. Commit
99d86241 removed the highlight on the assumption that the copy lost its diffuse: the copy itself was fine, the flat
tint was the problem, and the restore path also missed aa-cry's `StandardMaterial` -> `IllumMaterial` swap (the mesh
could keep a stale glowing material). Both are fixed; the warm tint is still an approximation of the native highlight,
and `use_target_decal` / silhouette are not drawn.

1.2 sources for the farm wagon (slave template 60, model 1008):

| Data | Value |
|---|---|
| `slave_doodad_bindings` 157..160 | owner 60, attach points 9..12, doodad 3446 ("backpack storage box") |
| `model_attach_point_strings` 9..12 | `$cannon0`..`$cannon3` |
| `prefab://prefabs/vehicle_tractor.xml/vehicle_tractor.tractor_body` | cannon0 (-0.6295166, -1.9423828, 1.1413727), cannon1 (-0.62768555, -3.1704102, 1.1413727), cannon2 (0.63293457, -3.1746826, 1.1413727), cannon3 (0.6338501, -1.9420166, 1.1414185) |

The server's `app/Data/slave_attach_points.json` "Farm Wagon" points 9..12 = (-0.55, -2.0, 1.15), (0.55, -2.0, 1.15),
(0.55, -3.15, 1.15), (-0.55, -3.15, 1.15); the ~0.08 m inward X and ~0.03 m Y differences from the prefab markers are
server data, not client conversion (reported, not corrected here).

## Cutting down trees

A cutdown is an ordinary doodad func use: the player casts a skill whose `InteractionEffect` is
`WorldInteractionType.Cutdown` (1) at the tree (`CSStartSkill`, target 4). Two 1.2 examples:

* **15874** "Thinning": `DoodadFuncUse`-bound on the old trees (`doodad_funcs` groups 944, 888, 908, ...), 4 s cast,
  5 LP, no tool, `start_anim_id` 348. The dynamic action bar already shows it, so this is the path our client can
  trigger today.
* **16469** "cut the old tree of Memory Lake": 4 s cast, `start_anim_id` 51, `mainhand_tool_id` 2657 axe. It is not
  bound to a `doodad_funcs` row, so the player must cast it from the action bar at the tree; that generic doodad-target
  skill routing is the gathering slice's (all gathering funcs are skill-less in the same way), not part of this one.
  When routed, the existing cast plays the start animation and the equipped mainhand axe; nothing cutdown-specific is
  added for the cast.

When the cast fires, the server's `InteractionEffect` of `WorldInteractionType.Cutdown` (1) calls `doodad.Use` and
broadcasts `SCVegetationCutdowningPacket(unitObjId, doodadObjId)` (0x081, two `bc` values, 6 bytes). The server sends
the resulting `SCDoodadPhaseChanged` before the cutdown broadcast, inside the same interaction. `aa-net` exposes it as
`ServerMsg::VegetationCutdown` / `Event::VegetationCutdown`; the AAEmu encoder's own test pins unit 0x000100 and doodad
0x000200 to `00 01 00 00 02 00`.

The 2026-10-30 capture of the original client (world old birch template 407, phase 944, skill 15874) confirms that
shape: the dynamic action bar showed 13977/F, 15874/G and 11004/H; `SCSkillStarted` announced a 4000 ms cast; then, in
one log batch and in this order, came `SCDoodadPhaseChanged` to 6251 (`time_left` 180000), `SCErrorMsg` 181 and the
6-byte `SCVegetationCutdowning` for the same doodad; and `SCDoodadRemoved` arrived 2367 ms after the cut. The error
message sits between the phase change and the cut. 181 is `BAG_INVALID_ITEM` in the client's `ErrorMessageType` table
(`hud/error_names.txt`, the same code as the server's constant); its `ERROR_MSG` text is "Invalid item." (`ui_texts`
row 182: up to code 285 that group's rows are the codes plus one, so row 181 is `INTERACTION_NPC_DESCRIPTION_NOT_FOUND`).
The capture does not say what made the lane send it. It exposed no vegetation entity, so the fall transform, its model
and the swap could not be sampled.

The 1.2 client's `OnVegetationCutdowning` handler (decompile 368373-368397) resolves the unit and doodad and calls
`FUN_393b7800(unit, doodad, 100)`; `FUN_393b7140` stores that as a 0.1 s doodad-model effect (100 * 0.001,
773577-773584), and `FUN_393b6910` drives the vegetation model from the cutter's side of the tree and logs
`vegetation cut downed !!` (773342). The native model's fall direction blend, easing and duration were not recovered.

`ingame/cutdown.rs` implements the visible part:

* **Direction** is horizontal from the cutter to the tree (away from the cutter), using the remote unit's position or,
  for the local player (who is not in `Units`), the `Mover.pos` of the `LocalPlayer` entity. A tree with no known
  cutter falls towards `+Y` (our fallback). The capture had no transform samples, so the direction remains a guess.
* **Timing** uses the decompile's 0.1 s start delay followed by a 2.2 s quadratic tip. The fall duration and curve
  stay our guess inside the database's 2-3 s cutdown-to-final windows (e.g. phase groups 2178/3159). The capture's
  2367 ms removal leaves 67 ms after the 2.3 s fall, so the observed removal never shortens the fall; a test pins
  that window, not the native animation.
* **Phase after the fall**: the capture sends the phase change (then `SCErrorMsg` 181) before the cut in the same
  batch, and the plugin chain runs `cutdown::start` before `doodad_visual::phase_changes`, so the fall keeps the
  standing model and the phase's swap waits (a test runs the registered chain over a small database with the captured
  batch and fails when the order is reversed). While the tree falls, `phase_changes` keeps that model and the scale the
  tree stood with (`grow` leaves a falling or lying tree alone) and retries the swap next frame; the swap after the
  landing applies the new phase's growth scale together with its model, so a grown tree does not pop to the cut
  phase's scale mid-fall. A cut phase that keeps the model (e.g. group 1553, "cut") leaves the tree lying down until
  `SCDoodadRemoved`/respawn; a phase with a different explicit model (a stump) swaps upright when the fall ends. An
  empty phase model falls back to the doodad template (`Db::doodad_phase_visual`), which is usually the same tree, so
  the tree also stays lying: the database has phase 944 and template 407 on the same `white_birch_do_moss.cgf` and
  phase 6251 without a model, so the captured birch lies until its removal in this client. Whether the native client
  deliberately defers the swap and the scale like this is still unverified (the capture saw no model swap).
* Removal is the existing `SCDoodadRemoved` path: the root and its `Falling`/`Fallen` state go with the entity, and a
  swap still queued for it is dropped when it comes up (a test removes the tree at 2367 ms and one second into the
  fall). The cut phase's `DoodadFuncLootItem` rows are ordinary server loot (`SCLoot*`), not part of the fall. The
  captured removal at 2367 ms is after the fall lands.

**Seeing it** (`crates/aa-client/examples/doodad_cutdown.rs`, no server):

```text
cargo run -p aa-client --example doodad_cutdown -- --mute --offscreen-shot shots/aa-client/doodad_cutdown.png
```

It needs `AA_PAK` and `AA_DB` (it says so and exits 0 without them). It spawns the captured old birch (template 407, phase
944) with the cutter (the local player, unit 25178) 2.5 m west of it and feeds the captured batch (`SCDoodadPhaseChanged`
6251, `SCErrorMsg` 181, `SCVegetationCutdowning`) in that order, the way `NetPlugin::receive_events` hands every session
event to the ECS (`Doodads::apply`, then a `NetEvent` in `PreUpdate`), so `units::sync_views`, `doodad_visual` and `cutdown`
run unchanged. The tree is spawned the same way (`DoodadsSpawned`) and built from the database and the pak. It saves
`doodad_cutdown-0.0s.png` (standing, taken before the packets), `-1.2s.png` (falling, about 22 degrees over, still the
standing model) and `-2.4s.png` (lying, the cut phase drawn), then exits; it exits 1 when the tree does not end up lying
down. `--offscreen-shot` opens no window; `--shot` (the window) saves the same pictures, `--shot-after S` is the least time
before the scene is staged (default 5), and without either flag the cut plays once in the window. The pictures stay in
`shots/` (git-ignored). The offscreen loop is paced by `--headless-fps` (default 30), so its logged `aa-dev: fps=` reads about
that (about 116 with `--headless-fps 120`); the window runs uncapped (about 600 at 960x540). The server's `SCDoodadRemoved`
(2367 ms after the cut in the capture) is left out so the landed tree is still there for the last picture.

The old birch is about 40 m tall in the pictures (the bounds of its meshes at scale 1; the capture did not record the
doodad's scale), so the camera stands 66 m back and the cutter is a few pixels tall. The fall away from the cutter, its curve
and its duration are the estimates described above, and so is the lying tree touching the ground plane (no terrain here).

## Pending live checks

* Tree cutdown: the 2026-10-30 capture confirmed the cast (15874, 4000 ms), the batch order phase change, `SCErrorMsg`
  181, cut, the 6-byte cutdown body and the 2367 ms removal. Still not captured: the fall direction/start
  delay/duration/curve (no vegetation transform accessor), whether the phase swaps or hides the tree before the
  fall ends (stump vs whole tree lying), whether the tree keeps its scale while it falls, the beech 368 and oak 1704
  cut phases, the axe prop/cast animation, the loot window, whether the original client shows the 181 text ("Invalid
  item."; our HUD prints the `ERROR_MSG` text of every `SCErrorMsg`, `hud/alerts.rs`), and a cutdown that arrives a
  frame or more after its phase model swap (the standing model is not restored; only the same-batch order was
  observed).
* Crate placement against a 1.2 capture of the farm wagon (`shots/cart-12/` was empty): the server JSON points are
  used, not the prefab cannon markers, so the four boxes may sit a few centimetres inboard of the authored markers.
* The highlight against a 1.2 capture: the warm emissive colour and its strength are ours (the native
  `use_target_highlight` colour is not recovered); check that a highlighted storage crate keeps its wood texture.

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
* No silhouette or target decal on the doodad in reach (`doodad_almighties.use_target_decal`); the emissive
  `use_target_highlight` is restored, but its colour is our approximation. No walking to a doodad that is right clicked
  out of reach, no gathering / farming UI (labor, tools, the gauge of a harvest are whatever the server and the existing
  cast bar show).
* The bar stays up during the cast (not compared with the real client).
