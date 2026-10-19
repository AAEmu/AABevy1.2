# Using a doodad: what the client sends, what the server does (AAEmu Rust lane `bevy`)

Server-side research for generic doodad use in the Bevy client, 2026-10-19. Source: the lane
`D:\aaemu-wt\bevy` (`rust/crates/aaemu-game/src/`), data: `compact.sqlite3` (server copy).
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

(Server behaviour for these: being implemented in this lane; this file is updated when done.)

## NPCs are different

NPC interaction (merchant, banker, ...) is `CSStartInteraction` (0x068: `bc npcObjId, bc objId, i32
extraInfo, i32 pickId, u8 mouseButton, i32 modifierKeys`) -> `SCNpcInteractionSkillList`; that path
does **not** handle doodads (it only answers for NPCs and mates).
