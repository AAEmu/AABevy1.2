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
list and the bound return point) and `SCPortalInfoSaved` (rename) into `Portals`. The lane never sends `SCPortalDeleted`:
`CSDeletePortal` removes the entry locally (`Event::PortalDeleteSent`). Entry coordinates on the wire are relative to the
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

* `SCPortalDeleted` and `SCUnitPortalUsed` are never sent (the client drops a deleted entry itself; bystanders see no use effect).
* `CSUseTeleport` (NPC teleporters), `CSNaviTeleport`, `CSNaviOpenPortal` only log.
* `OpenPortalEffect` checks the distance only as `x/y > pos + 3`; the gates live a fixed 30 s.
* No house entry exists in the 1.2 book data, so the house return is not there.
