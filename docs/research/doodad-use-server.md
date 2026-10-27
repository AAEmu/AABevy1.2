# Using a doodad: what the client sends, what the server does (AAEmu Rust lane `bevy`)

Server-side research for generic doodad use in the Bevy client, 2026-10-19. Source: the lane
`<aaemu-src>` (`rust/crates/aaemu-game/src/`), data: `compact.sqlite3` (server copy).
File references are relative to `rust/crates/aaemu-game/src/`.

## TL;DR

There is **no separate "interaction start" packet for doodads**. A doodad is used by casting the
doodad func's skill **at the doodad**: one `CSStartSkill` (0x052) with the doodad as the cast target.

```
CSStartSkill 0x052 (plain, LEVEL_PLAIN)
  u32  skillId          // doodad_funcs.func_skill_id of the func in the doodad's CURRENT phase
  u8   casterType = 0   // SkillCasterType.Unit
  bc   casterObjId      // your own character's unit objId (3 bytes LE)
  u8   targetType = 4   // SkillCastTargetType.Doodad
  bc   doodadObjId      // the doodad's objId (from SCDoodadsCreated / SCDoodadCreated)
  u8   flag = 0         // low nibble = skill-object type; 0 = none, nothing follows
```

That is exactly `aa_net::skill::write_start_skill(w, skill_id, self_obj_id, StartCaster::Unit,
&target)` with a doodad target (`target_kind` 4 + bc). (`targetType = 0`, Unit, with the doodad's
objId also resolves on this server, because the lookup is by objId and skills of target type
`Doodad`/`AnyUnit` accept "unit or doodad" casts, see `systems/skills/skill.rs` `initial_target`;
send 4, it is what the type says.)

Example, Memory Tome (doodad 841 "Harpa Training Ground Memory Tome", phase 282, func Binding with
skill 14602), player objId 0x000123, doodad objId 0x004567:

```
52 00                      opcode 0x052 (in the frame header, not the body)
0a 39 00 00                skillId 14602
00 23 01 00                caster: Unit, objId 0x000123
04 67 45 00                target: Doodad, objId 0x004567
00                         flag: no skill object
```

## Which skill id

The client already knows the doodad's current phase: `func_group_id` in each `SCDoodadsCreated`
(0x112) / `SCDoodadCreated` (0x10c) entry and in `SCDoodadPhaseChanged` (0x10f: `bc objId, u32
funcGroupId, u32 timeLeft, i32 puzzleGroup, u32 itemTemplateId`). Then:

```sql
select id, actual_func_type, actual_func_id, func_skill_id, next_phase, perm_id
from doodad_funcs where doodad_func_group_id = <current func_group_id>;
```

* Use the row with a non-zero `func_skill_id` (if several: the one the interaction menu / cursor picks;
  most doodads have exactly one). The server matches it with `func_for_skill(current_phase, skillId)`
  (`systems/npcs/doodad/phase.rs` `use_doodad`): a skill that is not a func of the **current** phase
  does nothing (the cast still runs).
* `func_skill_id = 0` / NULL rows are not used by skill: they are either phase-entry automation or
  driven by their own packets (navi doodads: `CSNaviTeleport` 0x0e7 / `CSNaviOpenPortal` 0x0e8, `bc
  objId` each; see below).
* `doodad_phase_funcs` (timers, animations, require-quest, ...) run on the server when a phase is
  entered; the client never triggers them.
* Range: the skill's `max_range` (14602: 4 m). Stand within it (the server validates the skill like
  any other; refusals come back as `SCSkillStarted` with a non-zero result).

## What the server does and sends (14602 / Memory Tome)

1. `CSStartSkill` -> 100 ms delay -> `start_skill` (`systems/skills/handlers.rs`): 14602 is neither a
   learned, default nor item skill, so it takes the "doodad / quest-interaction skill" branch and runs
   `skill::use_skill` with the doodad target.
2. Cast: `SCSkillStarted` (0x0a1) broadcast (cast time 3000 ms for 14602), then after the cast
   `SCSkillFired` (0x0a2) (and `SCSkillEnded` / cooldowns as for every skill; 14602 cooldown 5 s).
   Moving cancels the cast like any cast (`CSStopCasting` 0x054 from the client to cancel).
3. Effect `InteractionEffect` (effect 10871 -> `interaction_effects` 1266, `wi_id` 19 = `Use`)
   -> `execute_interaction` (`systems/npcs/doodad/interactions.rs`) -> `doodad.Use(caster, 14602)`
   -> `DoodadFuncBinding.Use` (`systems/npcs/doodad/funcs/world.rs` `binding`):
   * `return_point = district_return_point(doodad_func_bindings.district_id, faction)` (841: district 15),
   * sets `characters.return_district = district_id` (saved with the character),
   * sends **`SCCharacterReturnDistricts` (0x05c)**: `i32 count, PortalWire[count], u32 returnDistrictId`
     (the full visited list again; `returnDistrictId` = the bound **return point id**, compared with
     the book entries' `id` by the Lua `GetBoundPortalId`),
   * moves the doodad to `next_phase` (841: 6999, a `DoodadFuncTimer` phase) ->
     **`SCDoodadPhaseChanged` (0x10f)** to everyone around; the timer later returns it to phase 282.
4. Quest hooks: `on_doodad_interaction` (quest objectives "interact with doodad X") fire here too.

Recall (skill 10594) afterwards goes to the new bound point.

**Verified live** on the lane (2026-10-19, headless `aaemu-testclient --scenario portal`, account `bevysrv1`):
`CSStartSkill(14602, Unit(me), Doodad(9253), 0)` 2.5 m from tome 841 -> `SCSkillStarted` (cast 300 = 3000 ms, target type 4
objId 9253) -> 3 s -> `SCSkillFired` -> `SCCharacterReturnDistricts` body `00000000 01000000` (0 districts, bound return point
1) -> `SCDoodadPhaseChanged(9253, 6999, timeLeft 4000)` -> 4 s later `SCDoodadPhaseChanged(9253, 282)`.

## Other doodads (same packet)

Every doodad func with a `func_skill_id` works the same way: `CSStartSkill(func_skill_id, Unit(me),
Doodad(objId), 0)`. The skill's effects decide what happens; usually an `InteractionEffect` (`wi_id`:
0 Looting, 1 Cutdown, 4 Harvest, 19 Use, 28 Binding, 73 OreMine, ...) that calls `doodad.Use` and
the current phase's func for that skill. Results the client sees: `SCDoodadPhaseChanged`,
`SCDoodadRemoved` (0x10d), loot/item packets, teleports (`SCTeleportUnit` 0x072 / `SCLoadInstance`
0x197), etc.

Some doodad skills carry their own effect besides the interaction, e.g. 3347 "White Dress Portrait"
(skill 15193, cast 2 s): effect 12504 `SpecialEffect` type 25 `Return` value1 304 (a worldgate) +
12505 `InteractionEffect` wi 19 (`Use` -> `DoodadFuncNaviTeleport` -> phase 8135 timer).

## Navi doodads (no skill)

`DoodadFuncNaviTeleport` / `DoodadFuncNaviOpenPortal` funcs with `func_skill_id = 0` (doodad 1034
"Portable Sign", 1219 "Guide Gate") are used with their own packets, not `CSStartSkill`:

```
CSNaviTeleport   0x0e7  bc doodadObjId
CSNaviOpenPortal 0x0e8  bc doodadObjId
```

Server (lane `bevy`, `systems/npcs/portal/mod.rs` `navi_teleport` / `navi_open_portal`):

* Both need the doodad's **current** phase to hold a func of that type with `func_skill_id = 0`;
  otherwise `SCErrorMsg` 3 (`NoInteractionAvailable`).
* `CSNaviTeleport`: the data carries no destination (`doodad_func_navi_teleports` is `id` only,
  `DoodadFuncNaviMarkPosToMap` of 1034 is (0, 0, 0) = "here"), so the destination is the navi
  doodad itself (a sign you marked on the map). Answer: **`SCNaviTeleport` (0x15c): `i64 x, i64 y,
  f32 z`** (x/y as `(i64)(v * 4096) << 32`, like `CSTeleportEnded`). The 1.2 client's
  `OnNaviTeleport` (x2game `0x391dc8c0`) runs the same local teleport routine (`0x3930c9e0`) as
  `SCTeleportUnit` with reason 0 and yaw 0; answer it with `CSTeleportEnded` (0x0b4) as after any
  teleport (the server holds movement until then). Same world only, no range limit.
* `CSNaviOpenPortal`: the character must be within 5 m of the doodad (else `SCErrorMsg` 154
  `TooFarAway`); the func then leads the doodad to its next phase (1219 "Guide Gate": phase 2046 =
  timer + opening animation) -> `SCDoodadPhaseChanged` to everyone around. No teleport is attached
  to it in the 1.2 data.
* In practice: 1034 is only spawned by item 5054 (not in the 1.2 item table) and 1219 by item 8122
  "Guide Gate", so these doodads only exist when a player places one.

## NPCs are different

NPC interaction (merchant, banker, ...) is `CSStartInteraction` (0x068: `bc npcObjId, bc objId, i32
extraInfo, i32 pickId, u8 mouseButton, i32 modifierKeys`) -> `SCNpcInteractionSkillList`; that path
does **not** handle doodads (it only answers for NPCs and mates).

## Related portal packets the lane now sends (2026-10-19)

| Request | Answer |
|---|---|
| `CSDeletePortal` 0x0df (`u8 type, u32 id`) | `SCPortalDeleted` 0x05e (`u8 type, i32 id`) to the owner, one per removed entry (deleting a district removes its visited sub zone, so every recall point of that sub zone is reported); the bound recall point is refused silently |
| `CSUsePortal` 0x0de (`bc objId, bool onlyMyPortal`) | first `SCUnitPortalUsed` 0x060 (`bc unitId` = the **gate NPC's** objId) to everyone around the gate (the traveller included), then `SCTeleportUnit` (+ `SCLoadInstance` across instances) |
| `CSUseTeleport` 0x0b3 (`u32 returnPointId, i32 money, bc npcObjId`) | NPC with `teleporter` within 10 m, `returnPointId` a recall point, the fare `money` paid -> `SCTeleportUnit` reason 14; no 1.2 NPC is a teleporter |
| `OpenPortalEffect` (skill 11216) | the gate point must be within 3 m + 2 m (3D) of the caster |
