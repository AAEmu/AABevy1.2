# Portals, recall, teleport: research for the client (remaining.md H5)

Read-only research, 2026-10-19. Sources: `compact.sqlite3` (server copy), the AAEmu Rust lane `D:\aaemu-wt\bevy`
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

## 2. Server (AAEmu Rust lane `D:\aaemu-wt\bevy`)

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
| 0x05e | `SCPortalDeleted` | `u8 portalType`, `i32 portalId` | **defined, never sent** (CSDeletePortal is silent) |
| 0x060 | `SCUnitPortalUsed` | `bc unitId` | **defined, never sent** (the "someone entered" effect) |
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

* `SCPortalDeleted` (0x05e) is never sent: after `CSDeletePortal` the client must remove the row itself.
* `SCUnitPortalUsed` (0x060) never sent (no use effect for bystanders).
* `CSUseTeleport` / `CSNaviTeleport` / `CSNaviOpenPortal` only log; `SCNaviTeleport`, `SCAboxTeleport` never sent.
* `Return` sends `SCLoadInstance(instance 0, ...)` and then `SCTeleportUnit`; the client treats the position as the new
  position and (like the real client, see `reference/ingame/combat_return/`) should show the loading screen.
  `return_to_main_world` (leaving an instance) sends only `SCLoadInstance` and waits for `CSInstanceLoaded`.
* `SCCharacterResurrected` has no zone: respawn is main-world only (fine, `respawns.json` is main-world).
* `OpenPortalEffect` distance test only compares `x > pos.x + 3` and `y > pos.y + 3` (so any point behind or beside the
  player passes); the client should still send the point in front.
* The portal NPCs live 30 s fixed; the tooltip promises 60 s with Occultism Lv20+.
* The compact DB `open_portal_effects.distance` is 3.0 while the tooltip says "5 meters away".
