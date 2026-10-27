# Portals, recall, teleport: research for the client (remaining.md H5)

Read-only research, 2026-10-19. Sources: `compact.sqlite3` (server copy), the AAEmu Rust lane `<aaemu-src>`
(`rust/crates/aaemu-game/src/systems/npcs/portal/`, `systems/skills/effects/special/`, `systems/skills/resurrect.rs`,
`world/session.rs`), the decompiled UI (`data/extract/aa-lua/decompiled/x2ui/portal/`), `reference/ingame/combat_return/`,
`reference/api/*.json`, and strings of the unpacked `x2game.dll`. IDA was not reachable, so what the native `X2Warp` functions
send is **inferred** (marked "inferred") from the Lua, the DB and the server handlers.

Sections: 1 gameplay and data, 2 server, 3 client UI, 4 teleport and zone change in our client, 5 portal NPCs and doodads,
6 references, 7 gaps and implementation order.

## 1. Gameplay and data

There is no `portals` / `world_portals` table. The data is split three ways: DB tables, three JSON files that only the
server reads (`rust/crates/aaemu-game/app/Data/Portal/{recalls,respawns,worldgates}.json`), and the per-character book
(MySQL `portal_book_coords`, `portal_visited_district`).

### 1.1 The Teleport Book (the "portal book")

The window title is "Teleport Book" (`locale.portal.title`). Item **4045 "Teleport Book"** (category 18 Book, bound,
`use_skill_id` 11216). Two tabs (`locale.portal.menu`): **"Default Site"** (the visited districts, `GetReturnList`; the
current recall point has a marker icon) and **"Recorded Site"** (the player's own recorded spots, `GetPortalList`).
Columns: Name, Zone Name, Continent (`origin` = Auroria, `west` = Nuia, `east` = Haranya, `sea` = Sea, from
`zone_groups.target_id`: 3 west, 4 east, 6 sea, ...), and a "Map / View" button that opens the world map at the spot.
Buttons: spawn (open portal), delete, rename; the book has a "Record" entry (`ShowRegisterPotal`).
Item tooltip text (`itemDescUsed`): "Opens a worldgate with 1000 HP, 5 meters away, that takes 30 seconds to open (sic;
it lasts 30 s, 60 s with Occultism Lv20+)".

Entries come from three places:

| Kind | Where it comes from | Stored |
|---|---|---|
| Recorded spot (private, "Recorded Site") | skill **11215** "Entering location in Teleport Book..." (cast 5 s, cooldown 60 s, reagent **item 4053 "Memory Ink" x1**, effect: SpecialEffect type 30 `SavePortal`, id 0). Rename: skill **16841** (same special effect, id != 0). Skill 16387 is another `SavePortal` (target type 8). The position saved is the caster's position, zone and yaw. | `portal_book_coords` (id from `ids.private_book`) |
| District return point ("Default Site") | Automatic: entering a sub zone (`CSNotifySubZone`) that has a recall point adds it. Source: `recalls.json` (124 entries, `Id == ReturnPointId`, `SubZoneId`) joined with `district_return_points` (1730 rows: `district_id, faction_id, return_point_id`) and `return_points` (584 rows, names). Removable (not the bound one: `cantRemovePlace`). | `portal_visited_district` |
| Bound recall point | Interact with a **Memory Tome** doodad (`DoodadFuncBinding`, 136 funcs on 135 doodads, `doodad_func_bindings.district_id`; most use `func_skill_id` 14602 "Setting recall point...", cast 3 s). Sets `return_district_id`. The default is the race's start (`default_return_district_id` of the character template). | `characters.return_district` |

There is no "house" entry in the 1.2 data: no house recall skill or table was found (skill names with "Return to Room" /
"Return to Hal Hahpa" are quest/dungeon `Return` effects). Homes are not in the book.

### 1.2 Opening a portal (a worldgate others can use)

* Skill **11216** "Opening Worldgate..." (item 4045's `use_skill_id`; cast 10 s, cooldown 10 s, fx group 310, start anim 331,
  fire anim 39). One effect: `OpenPortalEffect` (`effects` id 15638 -> `open_portal_effects` id 1, `distance` 3.0).
* Cost (reagents, "first row the player can pay wins", priority order):
  `open_portal_inland_reagents` (same continent): 1x item 417 "Bound Hereafter Stone" or 1x item 4052 "Hereafter Stone";
  `open_portal_outland_reagents` (other continent): 3x 417 or 3x 4052. "Continent" = `zone_groups.target_id` of the owner's zone
  vs the target's zone. No labor.
* Result: **two NPC units** (not doodads): entrance `npcs` 3891 "Gate of Space (portal)" (model 308, `PrefabModel` 172) placed at
  the position the client sends (in front of the caster) and exit `npcs` 6949 "(portal-exit)" (model 667, prefab 271) at the
  recorded spot. Both are named after the book entry, owned by the caster, 30 s lifetime (server constant), linked.
  Not allowed in an instance (skill result `INVALID_LOCATION`).
* Everyone nearby sees both (normal `SCUnitState` of an NPC) and can use either end. Safe Portal Mode (option
  `OPTION_ITEM_USE_ONLY_MY_PORTAL`, `onlyMyPortal` flag in `CSUsePortal`) restricts use to the owner's portals.

### 1.3 Entering a portal

Click the portal NPC (interaction); the client sends `CSUsePortal(objId, onlyMyPortal)`. Refused with `SCErrorMsg` for:
not your portal (609, safe mode), carrying a backpack (610, buff tag 831), in a trial (611). On success the server sets the
character's position to the *other* end (`teleport_position` of the NPC; same instance) and sends `SCTeleportUnit`.
Cross-instance teleport also sends `SCLoadInstance` first (not reachable with portals today, the target is always the
owner's instance).

### 1.4 Recall and return points

* **Recall** = skill **10594** (action bar slot 21, Shift-9; cast 10 s; cooldown 30 min; reagent none; fx group 288, start anim
  108): SpecialEffect type 25 `Return` with `value1 = 0` -> the bound district's recall point
  (`district_return_point(return_district, faction)` -> `recall_by_id`). Another effect (`BuffEffect` 10982) rides along.
  The real client shows the cast bar, a purple sphere around the left hand, then the **loading screen** and the arrival
  (`reference/ingame/combat_return/`).
* **Return** with `value1 > 0` = a **worldgate** id from `worldgates.json` (26 entries): used by 175 quest/dungeon skills
  (type 25), e.g. "Enter Continental Worldgate", "Leave Serpent's Pass". Item **3894 "Return Scroll"** has `use_skill_id` 0 (no
  skill in 1.2 data).
* **Escape** (type 74, skill 20297) and the death flow: see 1.5.
* The Return effect is not allowed while the target zone is closed (`NO_PERM`).

### 1.5 Death, inn and resurrection return point

* There is no separate "inn" bind in 1.2 data: the resurrection location is the **closest respawn point** of
  `respawns.json` (281 entries; only the coordinates matter) among those allowed for the character's faction
  (district-specific via `district_return_points`) in an open zone. War zones use the conflict's per-faction return point. In an
  instance it is the Temple Priestess NPC (template 502). `characters.resurrection_district` exists but the server does not use it
  for the choice (`default_resurrection_district_id` is only copied in).
* The client asks with `CSResurrectCharacter(inPlace)`; the server answers `SCCharacterResurrected(unitId, pos, zRot)`
  (**no zone/instance, position only**; the character is moved to a respawn point that can be kilometres away) and
  `SCUnitPoints`. Debuffs: Weakened Body 1128, Respawn Cooldown 2385 (5 min), War Zone Leech 4424.
* `SCNotifyResurrection(skillCaster)` is the "someone offers a resurrection" popup (different thing).

### 1.6 Teleport between zones and continents

The 1.2 `main_world` is one big world that holds both continents (Nuia around (15578, 15382), Haranya around (19304, 7646),
all in `recalls.json`). **A continent change inside `main_world` is just a position jump** (portal, recall, worldgate
`Return`, respawn). A different `world` is an **instance**: `instance_*` worlds (training camp, immortal isle, nachashgar,
howling abyss, burnt castle armory, ..., `arche_mall_world`, `login2`) exist in the pak (`game/worlds/<name>/`). Entering one
is `SCLoadInstance(instanceId, zoneId, pos, angles)` and leaving it is `Return`/`leave_instance`, both followed by the client
answering `CSInstanceLoaded`. Our client only has `main_world` and `login2` wired (`ingame::WORLD`).

Other teleports: `Blink` / `TeleportToUnit` skills (`SCBlinkUnit`), GM commands (`SCTeleportUnit` reason 14
`MoveToLocation`), and NPC teleporters (`npcs.teleporter`, `CSUseTeleport`): **no NPC in the data has `teleporter = 't'`**, so
that path is unused in 1.2 data.

### 1.7 Navi doodads (minor)

`DoodadFuncNaviOpenPortal` (ids 1, 3) and `DoodadFuncNaviTeleport` (ids 1, 2) belong to the "Portable Sign" (doodad 1034),
"Guide Gate" (1219) and "White Dress Portrait" (3347, skill 15193). They drive `CSNaviOpenPortal(objId)` /
`CSNaviTeleport(objId)` -> `SCNaviTeleport(x,y,z)`. All are stubs on the server (see 2.5). Also `WI_NAVI_DOODAD_REMOVE`
interaction icon. Skip until the rest works.

### 1.8 DB table index

| Table | Content |
|---|---|
| `open_portal_effects` (1 row), `open_portal_inland_reagents`, `open_portal_outland_reagents` | the cost of skill 11216 |
| `district_return_points` (1730), `return_points` (584), `districts`, `doodad_func_bindings` | district -> return point per faction; Memory Tome doodads |
| `skills` 11215 / 11216 / 16841 / 16387 / 10594 / 14602, `skill_reagents` (11215 -> item 4053), `special_effects` type 30 (SavePortal), 25 (Return), 74 (Escape) | the skills |
| `items` 4045 Teleport Book, 4053 Memory Ink, 417 / 4052 Hereafter Stone, 3894 Return Scroll | items |
| `npcs` 3891 / 6949, `models` 308 / 667, `prefab_models` 172 / 271, `prefab_elements` | the portal units |
| `doodad_func_navi_open_portals`, `doodad_func_navi_teleports` (both `id` only) | navi doodads |
| `zone_groups.target_id`, `zones`, `sub_zones` | continent / zone names |
| files `Data/Portal/recalls.json` (124), `respawns.json` (281), `worldgates.json` (26) | server only |

## 2. Server (AAEmu Rust lane `<aaemu-src>`)

A faithful port of AAEmu's C# `PortalManager`; almost everything is implemented. Files:
`rust/crates/aaemu-game/src/systems/npcs/portal/{mod,packets,data}.rs`, `packets/world.rs` (book packets),
`systems/skills/effects/special/{movement,misc}.rs` (Return, SavePortal, Escape), `systems/npcs/bridges.rs`
(OpenPortalEffect), `systems/skills/resurrect.rs`, `systems/npcs/subzone.rs`, `systems/npcs/doodad/funcs/world.rs` (Binding),
`world/session.rs` (book at enter-world), `systems/services/commands/scripts/add_portals.rs` (GM `/add_portal`).

### 2.1 Packets, client to server

All plain (`LEVEL_PLAIN`), opcodes from `opcodes.rs` (1.2). `bc` = 3-byte object id; coordinates marked "long" are
`(i64)(v*4096) << 32`; `position` = the 3x i32-ish `write_position` (as `SCTeleportUnit`).

| Opcode | Packet | Fields | Server |
|---|---|---|---|
| 0x0de | `CSUsePortal` | `bc objId`, `bool onlyMyPortal` | `use_portal_with_preference`. **Implemented** |
| 0x0df | `CSDeletePortal` | `u8 portalType` (1 = district, else private), `u32 portalId` | removes the entry; **sends nothing back** |
| 0x0b4 | `CSTeleportEnded` | `long x, long y, f32 z, 16 raw bytes (orientation)` | clears `DisabledSetPosition`, re-sends visible objects |
| 0x0e0 | `CSInstanceLoaded` | empty | sends `SCUnitState`, `SCDetailedTimeOfDay(12)`, clears `DisabledSetPosition` |
| 0x112 | `CSNotifySubZone` | `u32 subZoneId` (0 ignored) | stores `sub_zone_id`, adds the visited district, re-sends the book |
| 0x04e | `CSResurrectCharacter` | `bool inPlace` | respawn (1.5) |
| 0x0b3 | `CSUseTeleport` | `u32 returnPointId, i32 money, bc npcUnitId` | **stub: logs a warning** |
| 0x0e7 | `CSNaviTeleport` | `bc objId` | **stub: logs** |
| 0x0e8 | `CSNaviOpenPortal` | `bc objId` | **stub: logs** |
| CSStartSkill | with a skill object | see 2.3 | record / rename / open portal go through skills |

### 2.2 Packets, server to client

| Opcode | Packet | Fields | When |
|---|---|---|---|
| 0x05b | `SCCharacterPortals` | `i32 count`, then per portal `PortalWire` | at enter-world if the private list is non-empty; after `AddPrivatePortal` (one entry); an incremental add, the client appends |
| 0x05c | `SCCharacterReturnDistricts` | `i32 count`, `PortalWire[]`, `u32 returnDistrictId` (= the bound **return point id**) | at enter-world if the district list is non-empty; after a new district; after binding at a Memory Tome (full list again) |
| 0x05d | `SCPortalInfoSaved` | one `PortalWire` | after a rename (`ChangePrivatePortalName`) |
| 0x05e | `SCPortalDeleted` | `u8 portalType`, `i32 portalId` | answer to `CSDeletePortal` (the client already dropped the entry: idempotent, `aa-client-portals.md`) |
| 0x060 | `SCUnitPortalUsed` | `bc unitId` | sent around a gate when someone enters it (the gate NPC's id); the client plays fx item 57 there for 5 s (`aa-client-portals.md`) |
| 0x072 | `SCTeleportUnit` | `u8 reason, i16 errorMessage, position(x,y,z), f32 z2 (yaw radians)` | portal use (reason 0 `PORTAL`), `Return`, GM moves (reason 14) |
| 0x197 | `SCLoadInstance` | `u32 instanceId, u32 zoneId, f32 x,y,z, f32 angX, angY, angZ` (radians) | instance change; `Return` to the main world |
| 0x042 | `SCCharacterResurrected` | `bc unitId, position(x,y,z), f32 zRot` | respawn, Escape |
| 0x041 | `SCNotifyResurrection` | `SkillCaster` | resurrection offer |
| 0x15c | `SCNaviTeleport` | `long x, long y, f32 z` | defined, never sent |
| 0x1a2 | `SCAboxTeleport` | `i32 x, i32 y` | defined, never sent |
| SCUnitDeath | `SCUnitDeath(objId, killReason PortalTimeout=14, ...)` | broadcast when a portal NPC expires (30 s) | removes the portal |
| SCUnitState / SCUnitsRemoved | the portal NPCs | as any NPC | spawn with the cast |

`PortalWire` (`Portal.Write`), in this order:

```
u32 id            // private: portal_book_coords.id; district: the return point id
string name       // u16 length + UTF-8 (write_str)
u32 zoneId        // zone key (zones.zone_key)
f32 x             // metres RELATIVE to the zone's origin cell: world_x - origin_x * 1024
f32 y             // same for y
f32 z
f32 zRot          // degrees (the recorded yaw)
```

The zone-origin offsets come from the world's zone XML (`ZoneManager.GetZoneOriginCell`); the client must add
`origin * 1024` back to get world coordinates (the data is in `game/worlds/main_world/` zone files; our `aa-world` has the zone
rectangles). `SCCharacterReturnDistricts.returnDistrictId` is the bound return point id, compared with `portal_id` by the Lua
(`GetBoundPortalId`).

### 2.3 State machine (server)

```
ENTER WORLD   SCCharacterPortals (private), SCCharacterReturnDistricts (visited + bound)
SUBZONE       CSNotifySubZone(id) -> new district? -> push VisitedDistrict, rebuild list, resend SCCharacterReturnDistricts
RECORD        CSStartSkill(11215, skillObject{type 2: i32 id=0, string name}) -> cast 5 s, Memory Ink x1
                -> SavePortal: AddPrivatePortal(pos, zone, yaw, name) -> SCCharacterPortals([new])
RENAME        CSStartSkill(16841, skillObject{type 2: id, name}) -> ChangePrivatePortalName -> SCPortalInfoSaved
DELETE        CSDeletePortal(type, id) -> removed from the book; saved on the next save tick (no packet)
OPEN          CSStartSkill(11216, skillObject{type 1: u8 kind, i32 id, long x, long y, f32 z}) -> cast 10 s
                -> OpenPortalEffect: player within `distance` (3 m) of (x,y) [quirk: only checks x/y > pos+distance],
                   not in an instance, pays 1 (or 3 other-continent) Hereafter Stone, spawns entrance NPC 3891 at (x,y,z)
                   facing like the caster and exit NPC 6949 at the book entry; both die after 30 s (SCUnitDeath kill reason 14)
USE           CSUsePortal(objId, onlyMyPortal) -> checks -> Transform = other end
                -> SCTeleportUnit(reason 0, pos, yaw) (DisabledSetPosition = true until the client answers)
              client: CSTeleportEnded(pos) -> DisabledSetPosition = false, ResendVisibleObjects
RECALL        CSStartSkill(10594) -> Return(value1=0): SCLoadInstance(instance 0, zone, pos, yaw) then SCTeleportUnit(reason 14,
                pos, yaw), transform set, DisabledSetPosition = true -> the client answers CSTeleportEnded (aa-net does)
              (value1>0: worldgate id; with no gate found, the main-world position is restored with SCLoadInstance +
               SCTeleportUnit and the client has to answer CSInstanceLoaded as well)
BIND          Memory Tome doodad use -> return_district_id = district; SCCharacterReturnDistricts(full list, new bound id)
DEATH         CSResurrectCharacter(false) -> SCCharacterResurrected(respawn pos) + SCUnitPoints (+ debuffs)
```

`skill object` wire (`SkillObject`): `u8 type` then the body (types: 1 PortalInfo `u8 kind, i32 id, long x, long y, f32 z`;
2 SavePortalInfo `i32 id, string name`; 3 Text, 4 Position `long x, long y, f32 z`, ...). In `CSStartSkill` it follows the target
(`aa-net/src/skill.rs` already knows the layout but skips it). `PortalInfo.kind` mirrors the `portal_type` the UI passes to
`OpenPortal` (district vs private); `id` is the book/return-point id; (x,y,z) is where the gate appears.

### 2.4 What is implemented vs stub

Implemented: book load/save/send, record/rename/delete, visited districts, open portal (cost, NPC pair, lifetime, link), use
portal, Return (recall and worldgates), respawn, Memory Tome binding, `SCTeleportUnit`/`SCLoadInstance`, instance
return, `CSTeleportEnded`/`CSInstanceLoaded`, GM `/add_portal`.

Stubs and gaps on the server (all optional for a first client version):

* `SCPortalDeleted` / `SCUnitPortalUsed` were never sent by the first lane build; both are handled now (`docs/aa-client-portals.md`).
* `CSUseTeleport` / `CSNaviTeleport` / `CSNaviOpenPortal` only log; `SCNaviTeleport`, `SCAboxTeleport` never sent.
* `Return` sends `SCLoadInstance(instance 0, ...)` and then `SCTeleportUnit`; the client treats the position as the new
  position and (like the real client, see `reference/ingame/combat_return/`) should show the loading screen.
  `return_to_main_world` (leaving an instance) sends only `SCLoadInstance` and waits for `CSInstanceLoaded`.
* `SCCharacterResurrected` has no zone: respawn is main-world only (fine, `respawns.json` is main-world).
* `OpenPortalEffect` distance test only compares `x > pos.x + 3` and `y > pos.y + 3` (so any point behind or beside the
  player passes); the client should still send the point in front.
* The portal NPCs live 30 s fixed; the tooltip promises 60 s with Occultism Lv20+.
* The compact DB `open_portal_effects.distance` is 3.0 while the tooltip says "5 meters away".

## 3. Client UI (`x2ui/portal`, `.alb` decompiled in `data/extract/aa-lua/decompiled/x2ui/portal/`)

Files: `portal_view.lua` (layout), `portal.lua` (behaviour), `locale/{en_us,ko,ru}.lua` (`portalLocale`, Trion widths),
`components/edit_dialog_handler.lua` (`ShowRegisterPotal`, `ShowChangePortalName`), `baselib/locale_helper.lua`
(`locale.portal`, text group `PORTAL_TEXT`), `baselib/buttons/icon_style.lua` (`BUTTON_ICON.PORTAL_SPAWN/DELETE/RENAME`),
`baselib/tooltip/tooltip.lua` (`FillPortalInfo`, the item portal section), `map/world_map.lua`
(`worldmap:ToggleMapWithPortal`). Textures: `ui/portal.dds` (return marker, coords 0,38,21,17), `ui/button/icon_shape/portal.dds`.
The `portal` content is already in the `x2ui/*/toc.g` load set, so the scripts load today; only the native side is missing.
`X2Warp` is a bare stub in `crates/aa-client/src/login/gameapi.rs` (`GAME_OBJECTS`, a call returns nothing); the real client
has 12 native functions on it (`reference/api/x2_globals_ingame.json`).

### 3.1 Windows

* **Teleport Book** window (`portal.window`, 600 x 450, centred, Esc closes, title "Teleport Book"): a tab control with two
  `listctrl` tabs of 10 rows per page, columns Name (200, with a "recall point" icon `returnIcon` on the bound row), Zone Name
  (205), Continent (100), Map button (55), a page control per tab, bottom-left Delete and Rename icon buttons, bottom-right the
  "open portal" icon button. Selecting a row enables Open; Rename only on the Recorded tab; Delete is disabled for the bound
  return point. The window is created lazily on `TOGGLE_PORTAL_DIALOG` and destroyed when hidden (`EnableHidingIsRemove`).
* **Record dialog** (`ShowRegisterPotal` -> `EditTitleFunc`): title "Record", an edit box (name policy `VNT_PORTAL`, guide tip
  `name_guide_portal`), button "Record" -> `X2Warp:SavePortal(name)`.
* **Rename dialog** (`ShowChangePortalName`): the same dialog with the current name -> `X2Warp:RenamePortal(portalId, name)`.
* **Delete confirmation**: `X2DialogManager:RequestDefaultDialog` with `locale.portal.ask_delete(name)`; on OK
  `X2Warp:DeletePortal(portalType, portalId)`, then the Lua removes the row itself.
* **Confirm-to-enter dialog**: there is **none** in the scripts. Using a portal is a plain click on the portal NPC (no popup);
  the unit tooltip is the only portal UI (`FillPortalInfo`: `[owner]` and the name when `info.is_portal`).
* The Open button runs `X2Warp:OpenPortal(...)` and hides the window; the cast bar (skill 11216, 10 s) comes from the normal
  cast UI.

### 3.2 `X2Warp` functions the scripts call (signatures as used)

| Call | Args / returns (from the Lua) | What it must do (inferred for the native side) |
|---|---|---|
| `X2Warp.GetReturnList()` (called **without self**: `tab.getList = X2Warp.GetReturnList; tab.getList()`) | returns a 1-based array of row tables | the visited-district list ("Default Site"), from `SCCharacterReturnDistricts` |
| `X2Warp.GetReturnListCount()` | number | `#list` |
| `X2Warp.GetPortalList()` / `.GetPortalListCount()` | the same for the recorded list ("Recorded Site") | from `SCCharacterPortals` |
| `X2Warp:GetBoundPortalId()` | number: the return point id the Return rows' `portal_id` is compared with | `returnDistrictId` of `SCCharacterReturnDistricts` (0 / nil when none) |
| `X2Warp:OpenPortal(portalType, portalId, portalName)` | no return | start skill **11216** with a `SkillObject` `PortalInfo{kind = portalType, id = portalId, x, y, z}`; (x,y,z) = a spot in front of the player |
| `X2Warp:SavePortal(name)` | no return | start skill **11215** with `SavePortalInfo{id = 0, name}` (needs Memory Ink) |
| `X2Warp:RenamePortal(portalId, name)` | no return | start skill **16841** with `SavePortalInfo{id = portalId, name}` |
| `X2Warp:DeletePortal(portalType, portalId)` | no return | send `CSDeletePortal(u8 type, u32 id)`; remove the row locally, then fire `DELETE_PORTAL` (the server sends no `SCPortalDeleted`) |
| `X2Warp:EndPortalInteraction()` | no return | called from the window's `OnHide`; ends the "interaction" the window belongs to (safe as a no-op) |
| `X2Warp:IsTeleporterNpc()`, `X2Warp:UseTeleport(id, cost)` | not called by any script | `CSUseTeleport` for NPC teleporters; no NPC has `teleporter` set in the 1.2 data, skip |

Row fields the Lua reads (`GetPortalInfo`, `FillPortalList`): `portal_id` (number), `portal_type` (opaque, handed back to
`OpenPortal` / `DeletePortal`; the parameter is named `portalTypeStr` in the native string table, so a string is plausible, but a
number works for the Lua; the server wants 1 = district, anything else = private), `name`, `zone_name` (text, the zone's display
name), `world_name` (one of `origin`, `west`, `east`, `sea`; anything else shows "Sea"), `zone_id`, `x`, `y`, `z` (world
coordinates for `worldmap:ToggleMapWithPortal`). `portal_id` of a Return row is the **return point id**. A wire `PortalWire`
gives id, name, zone key and zone-relative x/y; the client derives `zone_name` from `zones` / `zone_groups.display_text` and
`world_name` from `zone_groups.target_id` (3 -> `west`, 4 -> `east`, 6 -> `sea`, 1/2/5 -> `origin` or unknown; verify against
the world map), and world `x,y` by adding the zone origin cell * 1024.

### 3.3 Events

Fired by the client into `UIParent` / the window (`RegisterEvent`):

| Event | Args | Lua reaction | Native source (`x2game.dll` handlers: `OnCharacterPortals`, `OnPortalSaved`, `OnPortalDeleted`, `OnUnitPortalUsed`) |
|---|---|---|---|
| `TOGGLE_PORTAL_DIALOG` | `(addPortal: bool, extra)` | creates and shows the book; with `addPortal` also opens the Record dialog | using the Teleport Book item (and probably the record flow); the exact trigger is not in the scripts |
| `SAVE_PORTAL` | none | selects the Recorded tab, refreshes, goes to the last page | after `SCCharacterPortals` adds an entry (inferred) |
| `RENAME_PORTAL` | none | refreshes the page | after `SCPortalInfoSaved` (inferred) |
| `DELETE_PORTAL` | none | refreshes the page | after `SCPortalDeleted` / the local delete |
| `INTERACTION_END` | none | hides the book | when the interaction ends (moving away, Esc); already fired by `hud/crafting/batch.rs`, `store.rs` |

Item tooltips: item impl `portal` shows the `tooltip.portalInfos` section (`portalZone`, `portalCoordinate`, `portalMemo` in
the tooltip info). In 1.2 the Teleport Book item carries no recorded spot (the server stores the book per character, not per
item), so only the "unused" text applies. Unit tooltip: `info.is_portal`, `info.portal_owner` (the owner's name = the unit's
`owner_name`).

Option: `OPTION_ITEM_USE_ONLY_MY_PORTAL` (interface option "use only my portal") becomes the `onlyMyPortal` flag of
`CSUsePortal`. The constant is already defined (`native_constants.txt`).

## 4. Teleport and zone change in our client (today)

Packets (`aa-net`): `SCTeleportUnit` (0x072), `SCLoadInstance` (0x197) and `SCCharacterResurrected` (0x042) are decoded;
`aa-net` answers `SCTeleportUnit` with `CSTeleportEnded` (`session.rs:1837`, `game.rs:369`) and emits `Event::Teleported { pos }`;
`Event::LoadInstance(InstanceLoad)` and `Event::Resurrected { obj_id, pos, yaw }` are emitted. Not decoded: the five portal
opcodes (0x05b, 0x05c, 0x05d, 0x05e, 0x060), 0x15c, 0x1a2. Not sent: `CSNotifySubZone` (0x112), `CSUsePortal` (0x0de),
`CSDeletePortal` (0x0df); `CSInstanceLoaded` (0x0e0) is sent by the teleport world switch. `CSStartSkill` is written **without a skill object**
(`skill.rs write_start_skill`, `w.u8(0)`; `skip_skill_object` already knows the layouts), so record, rename and open-portal cannot
be expressed yet. (An implementation was starting in `crates/aa-net/src/portal.rs` while this was written.)

What happens on a jump (runtime world switch, 2026-10-21):

* `ingame/player.rs::follow_server`: on `LoadInstance`, `Teleported` (own player) and `Resurrected` (own `obj_id`): `mover.pos = pos`
  (yaw for `Teleported` and `LoadInstance`; the `SCTeleportUnit` yaw `z2` is still **dropped by aa-net's decode**), vertical speed
  reset, `state.settled = false` so the player waits (up to `COLLISION_WAIT` 5 s) until the collision around the new spot is built,
  camera yaw set, and a standing `Move` packet sent so the server's idea of the position agrees. `LoadInstance` now moves the player
  too, so the streamer follows the instance destination while the loading screen is up.
* `ingame/teleport.rs`: `SCLoadInstance` is no longer only a curtain trigger. It maps the packet to a world folder
  (`ingame/world_switch.rs::world_for_instance`) and requests `aa_world::WorldSwitch`; the world is installed before the loading
  screen's gauge reads the destination, and `CSInstanceLoaded` is sent once the new world's terrain, objects, models and pipelines
  have settled (same progress test as the entry loading screen). A pure `SCLoadInstance` (dungeon entry, no following
  `SCTeleportUnit`) now works, and so does `return_to_main_world`.
* `ingame/world_switch.rs`: `CurrentWorld` is the loaded folder name. `track_world` copies `aa_world::WorldConfig::world`; on a change
  `clear_old_units` despawns every `RemoteUnit`/`RemoteDoodad`, clears aa-net's `Units`/`Doodads` and `Views` (the server resends the
  new world's view). The player entity stays and is moved by `follow_server`. After an instance leave the server currently does not
  resend nearby units until the player moves (a server-side gap being fixed separately); clearing the old world's units is still
  correct, the new view arrives with the next movement/visibility update.
* `aa_world`: `WorldSwitch::request("instance_training_camp")` queues a switch; `WorldSwitchSet` / `switch_world` load `world.g`,
  `world.xml` (sector zones, synchronously, so `zone_at`/environment work at once) and replace the previous world's data. The old
  cells (`CellRoot`), object sectors, placed water/roads/decals/effects/sounds/lights/fog volumes/voxels, `Streamer`, `ObjectState`,
  `TerrainQuery`, `WaterQuery`, `WorldCollision` and the terrain layer library are dropped. Sky, sun, post effects and the ocean plane
  are the renderer's and stay; `TodCache` follows `WorldConfig::world` by itself. The startup world is still `main_world`.
* `login/loading.rs::build` now shows `worlds/<current world>/loading.dds` (all instance worlds have one), so a switch uses the
  destination's loading art.
* Continent changes inside `main_world` are still just a position jump.

### 4.1 Instance identity -> `game/worlds/<name>`

`SCLoadInstancePacket` (`aaemu-game/src/systems/skills/packets_special.rs:153-178`) is
`instance_id, zone_id, x, y, z, ang_x, ang_y, ang_z`. The first field is the **runtime `WorldInstance` id**, not a folder: dungeon
entry sends `world.id` (`systems/instances/dungeon.rs:488`), a portal to another instance sends `teleport.world_id`
(`systems/npcs/portal/mod.rs:899`), and `return_to_main_world` sends `MainWorldPosition.WorldId`, which is
`DEFAULT_INSTANCE_ID` / the main-world template id 0 (`systems/instances/worlds.rs:333-365`). The second field is the template's
**zone key**. AAEmu builds one `WorldTemplate` per directory of `game/worlds/*/world.xml` (`data/world_templates.rs::WorldTemplates::load`),
so a zone key names exactly one folder (`get_world_id_by_zone_key`). The 1.2 `world.g` files agree: `instance_training_camp` owns zone
198, `instance_nachashgar` 278, `instance_prologue` 300, and `main_world` owns 129..307. `aa_world::world::world_for_zone` therefore
scans `worlds/*/world.g` for the zone and returns the **folder name**; `instance_id == 0` is the fallback for a main-world return
(packet zone 0). An unknown zone with a non-zero instance id is not guessed (the current world stays, with a warning).

Still missing for a good teleport:

1. The yaw from `SCTeleportUnit` (`z2`) and the `SCLoadInstance` angles are only partly used (the player takes `angles[2]`; aa-net
   still drops the `SCTeleportUnit` yaw).
2. Per-world state: target, cast bar (a Recall cast ends in the jump), mount state; units far from the new spot go by radius,
   but name plates and the minimap/radar centre must follow.
3. Dying: `SCCharacterResurrected` already moves the player; it should use the same curtain when the distance is large.
4. The position-derived HUD refresh (`ENTERED_SUBZONE`, zone text, music, time of day) should be checked after a jump
   (`hud/api.rs:978` compares the old and new `sub_zone`).

## 5. Portal NPCs (not doodads) and what is needed to draw and use them

The player-spawned portals are **NPC units** (`npcs` 3891 entrance, 6949 exit; `SCUnitState` with `owner_id` = the caster and
name = the book entry's name). They are not in `Doodads`. `UnitKind::Npc { template_id, owner_id }` and `Unit.name` /
`owner_name` already carry everything the tooltip needs. Doodad portals exist for the dungeon / navi cases
(`DoodadFuncNaviOpenPortal` / `Teleport`, `DoodadFuncEnterInstance`, the world gates) and are separate work.

Drawing today: `ingame/data.rs` resolves a `PrefabModel` NPC (`models.sub_type = 'PrefabModel'`) through `prefab_elements` to the
**first `object_Model`** of the prefab. The portal prefabs (`prefab://Prefabs/fx.xml/portal_gate_dot`, `.../portal_gate_end`,
`.../portal_gate_exit_dot`, `.../portal_gate_exit_end`; state 1 = alive, 5 = ending) have **no `object_Model`**: they contain an
invisible proxy brush (`objects/effects/portal_gate_proxy.cgf`, material `materials/effects/portal_gate_proxy`) and
`ParticleEffect` entities. So a portal NPC is reported as `unit ... is not drawn` (`units.rs:363`) and nothing appears.

Effects in the prefabs (all in `libs/particles/abillity_skill_table_m.xml`, played by `aa-fx`):

| Prefab | ParticleEffect | Offset z |
|---|---|---|
| `portal_gate_dot` (entrance, state 1) | `abillity_skill_table_m.system_skill.portal_gate_dot3_indivisual`; hit `...portal_gate_hit`; sound `x2_sounds.particles.portal` at z 2 | 0.037 |
| `portal_gate_end` (entrance, state 5, despawn) | `...portal_gate_dot3_indivisual_end` | 0.083 |
| `portal_gate_exit_dot` (exit, state 1) | `...portal_gate_exit_dot`; `_hit` = `...portal_gate_end`; sound `x2_sounds.particles.portal` | 0.75 |
| `portal_gate_exit_end` (exit, state 5) | `...portal_gate_exit_end` | 0.75 |

What to build:

1. **Model path**: in `prefab_model_file` / `prepare_unit`, when the prefab has no `object_Model`, parse its `ParticleEffect`
   entities (aa-world has `placement::parse_effect_entities`) and spawn `aa_fx::Effect::new(name)` entities (children of the
   unit entity, at the offset, unit scale, primed, looping). Treat such a unit as drawn. The pick sphere needs a height
   (`dialog.rs::right_click_pick` uses `z + 0.9 * scale`; a gate is about 3 m tall).
2. **Despawn**: `SCUnitDeath(killReason 14 PortalTimeout)` precedes `SCUnitsRemoved`; play the `_end` state (5) for about a second
   before removal (optional; the native npc table has `portal_despawn_fx_id`, `portal_spawn_fx_id`, `portal_scale`, `portal_time`).
3. **Use**: the existing right click on an NPC (`hud/dialog.rs` -> `Interact::Npc` talk) must branch first: a unit whose
   `template_id` is 3891 or 6949 sends `CSUsePortal { obj_id, only_my_portal }` instead of starting a talk (also by key F near it).
   Honor `OPTION_ITEM_USE_ONLY_MY_PORTAL`. The server does not check distance; about 3 m is natural. Refusals come as `SCErrorMsg`
   (609 / 610 / 611), which the HUD already turns into alerts.
4. **After the click**: `SCTeleportUnit` + `CSTeleportEnded` + `follow_server` already move the player. Add the curtain only for far
   destinations.
5. Tooltip: the unit tooltip info must carry `is_portal = true` and `portal_owner = owner_name` (find where the unit tooltip info
   table is built in `hud/`); the name is the book entry's name; colours from the faction.
6. The Safe Portal Mode interface option.

Generic doodad interaction (`SelectInteractionEx` / use-doodad) does not exist in the client yet (G3 in `remaining.md`; only
`Command::UseDoodadSkill` and the doodad drawing in `ingame/units.rs`, which uses `doodad_almighties.model` and ignores phases).
It is needed for Memory Tome binding (136 funcs, skill 14602 "Setting recall point...") and the navi doodads, not for the
portals above.

## 6. Reference captures

`reference/ingame/combat_return/` is a real-client capture of **Recall** (skill 10594, Shift-9): the cast bar "Recall", the sphere
effect at the left hand, the loading screen and the arrival (22 frames, `notes.md`, `frames.json`). It is the only
portal-related capture. There are none of the Teleport Book window, the Record dialog, the portal gates or a portal use.
Screenshots that would help (real client, `reference/ingame/<name>/` with `ui.json` widget dumps):

1. `portal_book_default` and `portal_book_recorded`: the open Teleport Book on each tab (recall marker icon, page controls, the
   tooltips of the three icon buttons); the widget tree of `portal.window` would let us check the layout pixel for pixel.
2. `portal_record_dialog`, `portal_rename_dialog`, the delete confirmation.
3. `portal_cast`: the 10 s cast of "Opening Worldgate" (cast bar, fx group 310).
4. `portal_gate`: entrance and exit gates (frames over 30 s, from several sides, with a character beside it), the despawn, the
   nameplate / tooltip ("[Owner] Name").
5. `portal_use`: entering a portal; is there a fade or loading screen for a short hop; the arrival; what a bystander sees (the
   `SCUnitPortalUsed` effect).
6. `memory_tome`: the Memory Tome doodad, the "Setting recall point..." cast and the result.
7. `death_respawn`: the death window and the arrival after `CSResurrectCharacter`.
8. How the book is opened (item right click vs. another way) and the Record flow (`TOGGLE_PORTAL_DIALOG` with `addPortal` true);
   an item tooltip of the Teleport Book.
9. A worldgate `Return` (a quest skill such as "Enter Continental Worldgate") for the loading behaviour of a skill-driven jump.

A run of `aaemu-clientdriver` (`rust/docs/client/LUA-DRIVER.md` in the server repo) could dump the `X2Warp` return values
(`GetPortalList()` rows with `portal_type`) from the real client; that would settle the only inferred part of the book.

## 7. Gaps and proposed implementation order

Gaps, by crate (this research changed no code):

* **aa-net**: decode `SCCharacterPortals` 0x05b, `SCCharacterReturnDistricts` 0x05c, `SCPortalInfoSaved` 0x05d, `SCPortalDeleted`
  0x05e, `SCUnitPortalUsed` 0x060 (0x15c / 0x1a2 later); a `Portals` state (private list, district list, bound return point);
  commands `UsePortal`, `DeletePortal`, `NotifySubZone`, `InstanceLoaded`; a `SkillObject` (PortalInfo / SavePortalInfo) in
  `ClientMsg::StartSkill` and its command; keep the `SCTeleportUnit` yaw; opcodes in `opcodes.rs` (cs 0x0de, 0x0df, 0x0e0, 0x112).
* **aa-client / HUD** (`hud/portal.rs` + `portal.lua`, in the style of `repair.rs` / `repair.lua`): install `X2Warp` over the stub
  (3.2), build the row tables from the `Portals` state (zone names, continent key, world coordinates), fire `SAVE_PORTAL` /
  `RENAME_PORTAL` / `DELETE_PORTAL` / `TOGGLE_PORTAL_DIALOG`, open the book on use of item 4045 (in `panels.rs::use_item`, a
  `use_skill == 11216` item fires `TOGGLE_PORTAL_DIALOG` instead of casting), send `CSNotifySubZone` when
  `HudData::sub_zone_root_at` changes, the `onlyMyPortal` option.
* **aa-client / ingame**: portal unit drawing (prefab particle entities), right click -> `CSUsePortal`, the yaw from
  `SCTeleportUnit`. The teleport curtain, `CSInstanceLoaded` and the `SCLoadInstance` world switch are done (section 4).
* **aa-ui**: `worldmap:ShowPortal(zone_id, x, y, z)` exists as a method name (`aa-ui/src/lua/worldmap.rs:1062`); check that it
  highlights the spot.
* **Server (not ours to change)**: `CSUseTeleport` / Navi are stubs; the
  `OpenPortalEffect` distance check only compares `x/y > pos + 3`; fixed 30 s lifetime.

Proposed order (each step is testable against the lane server; the GM command `/add_portal <name>` fills the Recorded tab without
Memory Ink):

1. **aa-net decode + state + commands** (`Portals`, `UsePortal`, `DeletePortal`, `NotifySubZone`, `InstanceLoaded`, the skill
   object on `StartSkill`) with unit tests of the byte layouts in 2.2 / 2.3.
2. **`X2Warp` and the book window**: row building, `GetBoundPortalId`, delete (local + packet), the events. Check the window with
   `--shot`; a dev flag to fire `TOGGLE_PORTAL_DIALOG`. Then record and rename via skill objects (Memory Ink needed).
3. **`CSNotifySubZone`** so the Return list fills as the player walks (verify the id: the root sub zone vs. the server's
   `recalls.json` `SubZoneId`; test by walking into a recall sub zone and watching `SCCharacterReturnDistricts`).
4. **Teleport polish**: done for `CSInstanceLoaded`, `LoadInstance` (including the runtime world switch) and the curtain for long
   jumps (Recall from the action bar is the test case; compare with `combat_return`). Remaining: the yaw from `SCTeleportUnit`,
   and using the same curtain for a long respawn.
5. **Portal NPC drawing and use**: prefab particle effects, right-click `CSUsePortal`, tooltip, safe-portal option, despawn
   effect. Needs Hereafter Stones for the real open flow; a GM-spawned pair is quicker.
6. **Open flow**: `OpenPortal` -> skill 11216 with `PortalInfo` (a spot in front of the player, `kind` / `id` from the row), the
   book item as caster, cast bar and FORM effect.
7. Later: Memory Tome binding (needs doodad use, G3), navi doodads, NPC teleporters, the `SCUnitPortalUsed` effect.
