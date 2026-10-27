# aa-client: portals, the Teleport Book, recall and teleports

Research: `docs/research/portals.md` (packets, server state machine, data). Code:

| Where | What |
|---|---|
| `crates/aa-net/src/portal.rs` | the packets (`PortalEvent`, `PortalCommand`), the book state `Portals` (in `net_state`) |
| `crates/aa-client/src/hud/portal.rs` | `X2Warp` for the original `x2ui/portal` window, its events and requests, `CSNotifySubZone`, using a gate |
| `crates/aa-client/src/ingame/teleport.rs` | the loading screen for long teleports, `CSInstanceLoaded` |
| `crates/aa-client/src/ingame/{data,prepare,units}.rs` | gate NPCs drawn as their prefab's particle effects |

## The book

aa-net folds `SCCharacterPortals` (full list at login, one entry per record), `SCCharacterReturnDistricts` (the Default Site
list and the bound return point) and `SCPortalInfoSaved` (rename) into `Portals`. `CSDeletePortal` removes the entry
locally at once (`Event::PortalDeleteSent`, so it also works against a server that never answers). The server's answer,
`SCPortalDeleted` (`u8` type, `i32` id), is idempotent: it removes the entry only if it is still in the book (and then
raises `DELETE_PORTAL` like any change); in the usual case, the local delete already done, it changes nothing and raises no
second refresh. Entry coordinates on the wire are relative to the
zone's origin cell (`world.xml` `Zone originX/originY` * 1024); the rows the scripts get are world coordinates.

`hud/portal.rs` installs on `X2Warp`: `GetReturnList` / `GetPortalList` (+ counts; called without `self`), `GetBoundPortalId`,
`SavePortal(name)` (skill 11215, Memory Ink), `RenamePortal(id, name)` (16841), `OpenPortal(type, id, name)` (11216, the book
item as caster, a `PortalInfo` skill object 2.5 m in front of the player; the server allows up to 3 m), `DeletePortal`,
`EndPortalInteraction`, `IsTeleporterNpc` (false), `UseTeleport` (no-op: NPC teleporters are a server stub). Events:
`SAVE_PORTAL`, `RENAME_PORTAL`, `DELETE_PORTAL` after the book changed, `TOGGLE_PORTAL_DIALOG(addPortal)` when the book is
used.

Assumption: using the Teleport Book (item 4045, its use skill is 11216) opens the window; with Shift held it opens with the
Record dialog on top (`addPortal = true`). How the native client decides `addPortal` is not known.

The Default Site tab fills as the player walks: every 0.5 s the sub zone chain at the player (smallest containing shape,
then its parents) is compared with the last one, and on change each id goes out as `CSNotifySubZone`, root first. The
server adds a visited district when `recalls.json` has that id (113 of its 124 ids are roots, 11 are children).

## Gates

The opened entrance (NPC 3891) and exit (6949) have models whose prefab (`fx.xml/portal_gate_dot`, `portal_gate_exit_dot`)
holds only particle effects; `Db::prefab_effects` reads them and `UnitBuild::Effects` spawns them as `aa_fx::Effect`s. F
(nearest NPC in talk range) or a right click on a gate sends `CSUsePortal` instead of a talk, with the interface option
"use only my portal". The server answers `SCTeleportUnit` (plus `SCLoadInstance` across instances).

### What a gate use looks like to the others: `SCUnitPortalUsed`

The server sends `SCUnitPortalUsed` (`bc unitId`) to the players around a gate when someone goes through it (the lane
sends the **gate NPC's** id, as the later servers do). aa-net decodes it as `PortalEvent::UnitPortalUsed`; `unit_used` in
`hud/portal.rs` plays the client's effect on that unit: at the unit's position (its entity, else the player's, else the
server's position), for 5 s (`UseEffect` timer), as an `aa_fx::Effect`.

What the real client does (x2game.dll, Ghidra 12.1.4 on `<client12-unpacked>\x2game_unpacked.dll`, the setup of
`docs/research/re-skills.md`; helper scripts in `data/extract/ghidra/scripts`):

* The handler is `FUN_391d3970` (registered as `"OnUnitPortalUsed"`, 2 arguments). It looks the unit up in the client's unit
  table (`FUN_39360020(packet + 0xc)`; an unknown unit: nothing happens) and calls `FUN_393558e0` on it.
* `FUN_393558e0` spawns one **fx item** (`FUN_393f1160`: the `fx_items` row by id, its `asset_name`, `bone_id`, detail type)
  in "world position" mode at the unit's position (`unit->vtable[0x228]()+0x10`), sets its end time to **now + 5.0 s**
  (`FUN_393eee80`) and starts it (`FUN_393eefa0`). No sound, animation, camera effect or UI is involved in the handler.
* The fx item id is a constant in the client's `.data` (`0x39AC7460`, `x2game+0xAD7460`). The unpacked file has `.data`
  encrypted, so it was read from a running 1.2 client (`ReadProcessMemory`, read-only, as for the costume table of
  `re-equipment.md`; the client was started by me from the test client copy, never logged in, and closed again): the
  value is **57**. It sits in a block of hardcoded ids next to the portal NPC templates (3891, 5476, 6629 are the gate NPCs
  "Door of Space (portal / guide / portal exit)"), and the block at `x2game+0xAD7660` holds the portal skills 11215,
  11216, 16841 and the fx items 386 / 387 (`portal_gate_dot` / `portal_gate_form`), so the server database's numbering is the
  client's.
* In the 1.2 data `fx_items` 57 is "Curse of Doll_hit": `ability_skill_table.blackmagic.black0011_hit`, a particle effect with
  **no emitters** (`<Params />`, no children), and `fx_particles` 57 has no sound. So with this data **the real client's
  bystander effect is invisible**, and that is what we play: the effect is looked up from the database by the constant
  `USE_FX_ITEM`, nothing is special-cased. Confidence: high that 57 is the fx item played (live memory read plus the
  disassembled handler); that it shows nothing follows from the data and was not compared with a real capture (there is
  none of a portal use).
* Unrelated flashes that do show: the gate prefabs hold `_hit` entities (`portal_gate_hit` at the entrance,
  `portal_gate_end` at the exit) that play when the gate appears.

Dev: `AA_PORTAL_USE_FX=388` plays the casting effect `portal_caster_target` instead (a green column of light, sound
`portal_exit_start`; `shots/aa-client-portals/p2_use_388.png`); `--portal-used T` fakes the packet for the player's own
unit T s after the world is entered (`p2_use_57.png`: the real constant, nothing to see).

## Teleports

`SCTeleportUnit` carries the yaw at the destination (radians), `player::follow_server` uses it. A jump of more than 300 m
(`FAR`) or any `SCLoadInstance` (recall, world gates) shows the loading screen: the HUD stage is put aside (`hud::suspend`),
the original loading UI is built (`login::loading::build`), streaming is boosted, input is off, the gauge follows the
destination (terrain within 400 m, then objects, models, units), then the curtain (`login::curtain`) holds the last frame
while the HUD comes back (`hud::resume`). After an `SCLoadInstance`, `CSInstanceLoaded` is sent once the destination is
loaded (the server re-enables movement then). With `--enter-world` (no login plugin, no curtain) the loading UI goes away
without the curtain. A short hop is a plain move.

## Dev flags

* `--portal-lua T CODE` (repeatable): run CODE in the HUD Lua state T s after the HUD is up, e.g.
  `X2Warp:SavePortal('Lake')`, `X2Warp:OpenPortal(2, 4096, 'TestSpot')`, `FAKE.fire_now('TOGGLE_PORTAL_DIALOG', true)`.
* `--portal-used T` (repeatable): fake `SCUnitPortalUsed` for the player's unit after T s; `AA_PORTAL_USE_FX=<fx_items id>`
  swaps the effect it plays (see above).
* GM (account access level 100): `/add_portal <name>`, `/teleport <place>`, `/item add self 4052 10` (Hereafter Stone),
  `/item add self 4053 10` (Memory Ink).

## Live on the lane (2026-10-19, account `bevyportal`, character Portalbevy)

| Step | Result |
|---|---|
| Book load | `SCCharacterPortals` / `SCCharacterReturnDistricts` decoded; `X2Warp` counts right (1 district, bound 243, the recorded entries) |
| Return list | `CSNotifySubZone 324` at the Nuian start: the server records the district ("Enter RegionId: 324"), the list comes back |
| Record | `X2Warp:SavePortal('Lakeside')` / `('Solzreed')`: skill 11215 cast, Memory Ink used, entries 4097 / 4098 in the book |
| Open a portal | `X2Warp:OpenPortal(2, 4096, 'TestSpot')` in Solzreed: 10 s cast, Hereafter Stone used, the entrance gate drawn 2.5 m ahead |
| Enter it | F: `CSUsePortal` -> `SCTeleportUnit`, a 1534 m jump, loading screen, arrival at the exit gate (drawn too) |
| Recall | action bar slot 22 (skill 10594): `SCLoadInstance` + `SCTeleportUnit`, loading screen, `CSInstanceLoaded`, back at the return point |
| Login path | `--goto character-select`: the teleport shows the original loading UI to 100 %, the curtain holds it, the HUD comes back (4.2 s for 1530 m) |

fps 70-100 in game (dev build), 50-65 for the first seconds after a jump while the destination streams in.

### Second pass with the new server code (2026-10-19, accounts `<account>` / `<account>`, GM level 100 set on `lane_bevy_game.accounts`)

Characters Livebevyone / Livebevytwo (Nuian male, created with `--goto create ... --clua "CC.create(..)"`). GM chat commands are
sent from `--portal-lua` with `AAUI.ChatSend(0, '/item add self 4053 10')` / `'/move X Y Z'` (the HUD's own chat request). The
client now logs `portal: SCPortalDeleted type T id N (book: ...)` when the server's answer arrives (`log_deleted`). Logs and shots
are `shots/aa-client-portals/live2_*`.

| Step | Result |
|---|---|
| 1 Record + delete | `SavePortal('Lake')`, `('Second')` recorded (4104, 4105), `DeletePortal(2, 4104)`: `CSDeletePortal` -> `SCPortalDeleted type 2 id 4104` 14 ms later (trace shows the `SCPortal` packet right after the request), the book already without it (`after-delete 1 2:4105=Second`). Idempotent: one `DELETE_PORTAL` event per delete (the local one; the answer raises none); a second `DeletePortal` of the same id leaves the book alone (the lane sends no answer for an unknown id). `live2_s1_book_recorded.png`: Recorded Site lists only "Second" (opened with `FAKE.fire_now('TOGGLE_PORTAL_DIALOG', false)`, tab with `portal.listWnd.tab:SelectTab(2)`), `live2_s1_book.png`: Default Site |
| 2 Gate + `SCUnitPortalUsed` | `/move` to the Crescent Throne inn, record "Inn" (4106), `/move` back to the start, `OpenPortal(2, 4106)` (10 s cast), gate NPC 34757 up 2.5 m ahead, F: `CSUsePortal` -> `game -> SCPortal` (`SCUnitPortalUsed`) 25 us **before** `game -> SCTeleportUnit` (aa-net trace), `portal: unit 34757 used a portal: ... black0011_hit at ...` (the gate's id, the unit is known, no "not known" debug line, no warnings besides the known `portal_gate.cgf`), then `teleport: a long jump of 1537 m`, `loading screen up`, `arrived after 5.7 s`. `live2_s2_loading.png`: at the exit gate. fps 85-95 before, 40-55 during the first seconds after the jump |
| 2b Second client | `<account>` (second client copy) stood at the start next to the gate: its log has `portal: unit 34757 used a portal: ... at 15578.0 126.2 -15384.5`, the same gate id, once (the gate then expires: units 18 -> 17). So the lane sends it to the bystanders |
| 3 Memory Tome | `--pos 15676.0,15173.6 --doodad-key 1 8`: `use Desireen Memory Tome (19999): UseDoodadSkill 14602`, cast result 0 / 3000 ms, `recall point is 15`, `bound before 243` -> `bound after 15`, the tome back in its use phase 4 s later. fps 77-95 |

Notes from this pass: the exit gate (NPC 6949) is also "gone through" by F / right click (`CSUsePortal`); the lane answers with
`SCUnitPortalUsed` + a `SCTeleportUnit` to the exit's own position (a no-op teleport). Whether the real client lets the exit be used is
not known. `--hud-quest-key f T` counts from the first in-game frame, so a key that falls into a teleport's loading screen is lost.

Screenshots (`shots/aa-client-portals/`): `r3__11.png` (the gate opened in Solzreed), `r3__14.png` (arrival at the exit
gate), `r4__18.png` (the teleport loading screen at 100 %), `r4__33.png` (arrived in Solzreed, the Teleport Book open over the
exit gate), `r5__01.png` (`--enter-world`: arrived, the HUD back).

## Known problems

* **aa-ui, the book's rows stay empty**: `x2ui/portal/portal.lua` calls `listCtrl:InsertData(portal_id, 3, world_name)`. The
  first argument is a key (243, 4096 ...), not a row: the native list control gives a new key the next row. aa-ui's `InsertData`
  (`crates/aa-ui/src/lua/widget.rs`) uses it as a 1-based row, so the row is never marked as holding data and stays hidden.
  Other scripts pass keys too (achievement ids, member names, `itemInfo.idx`).
* **aa-fx, the gates' centre is an opaque white shape**: one of the prefab's particles (`portal_gate_dot3_indivisual` and
  the exit's) draws as an opaque white shape (probably a translucent or distortion particle without its blending; not compared with the real client yet); `objects/effects/portal_gate.cgf` (a model one of the
  effects references) does not load.

## Server (lane) gaps, not worked around

* `CSUseTeleport`, `CSNaviTeleport`, `CSNaviOpenPortal` are handled now (not tried live, see `docs/aa-client-doodads.md`).
* `OpenPortalEffect` checks the distance only as `x/y > pos + 3`; the gates live a fixed 30 s.
* No house entry exists in the 1.2 book data, so the house return is not there.
