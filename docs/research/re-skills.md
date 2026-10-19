# x2game RE: skill controllers, knockdown, effect lookup, beams, stealth

Source: Ghidra 12.1.4 on `D:\aa-re\client12\x2game_unpacked.dll` (image base `0x38FF0000`, full auto-analysis) and
`D:\AA1.2\bin32\cry3dengine.dll` (base `0x30000000`, read with capstone). Data: the client DB `D:\AA1.2\game\db\compact.sqlite3`
(same rows as the server copy for these tables). Addresses are VAs. Project and scripts: `data/extract/ghidra/` (git-ignored).

## 1. Skill controllers

### 1.1 How an id becomes a controller (no fallback)

* `skill_controllers` is loaded first (`3971E060`, called from the DB loader `3960B820` before effects, skills, buffs and
  plots). Each row is a 24-dword record in **SELECT order**: `[0] id, [1] active_weapon_id, [2] end_anim_id, [3]
  end_skill_id, [4] kind_id, [5] start_anim_id, [6] str_value1, [7] transition_anim_1_id, [8] transition_anim_2_id,
  [9] value1, [10] value10, [11] value11, [12] value12, [13..15] value13..15, [16] value2, [17] value3, [18] value4,
  [19] value5, [20] value6, [21] value7, [22] value8, [23] value9`. They go into a `std::map<id, desc*>` at DB `+0x127C`.
* The **skills** loader (`39731870`, column 0x5B) and the **buffs** loader (`39733A90`, column 0x8C) look the id up in that
  map **once, at load** (`396F7020` find, `cmp end`) and store the pointer, or **NULL when the row is missing**
  (skill desc `+0x13C`, `skill_controller_at_end` at `+0x140`; buff desc `+0x18C`). The id is never kept or re-resolved.
* Plot effects of `actual_type = 'SkillController'` (effects loader `39730620`, type index 0x17 -> `396FF9C0`) resolve the
  same way. In the 1.2 data all 348 such plot effects point at existing rows.
* There is **no hardcoded table and no default by kind**: a missing `skill_controller_id` simply means "no controller". The
  173 dangling skill ids (and 113 dangling buff ids) did nothing in the real client either.
* `SCSkillControllerState` (`OnSkillControllerState`, `391D4D60`) only carries rope state (`scType 0`: length / teared /
  cut); the server never sends a controller id to start.

### 1.2 Who starts a controller, and on which unit

`Actor::StartSkillController(desc, target, tag)` is vtable slot `+0x90` = `390CA700` (all six actor vtables); `+0x98` =
`390CA040` ends the current one (plays the desc's `end_anim_id` if set), `+0x94` = `390CA5F0` creates the network
(remote) variants.

* Skill fire (`3939B010`, "file skill: invalid skill type"): if `!skill_controller_at_end && controller && !end_skill_controller`
  and (unit `+0x3A72` is 0 **or** the controller is a Leap, kind 2) -> start it on the **caster** with the skill target.
  If `end_skill_controller` (desc `+0xC0`) is set it instead **ends** the caster's running controller (when the skill has no
  controller of its own or the running one has the same kind).
* Skill end (`39397FF0`, "end skill"): the same, for `skill_controller_at_end = true`.
* Buff start (`ClientBuffMan::ApplyAttrs`, `39389440`): buff desc `+0x18C` set and unit `+0x3A72 == 0` -> start on the **buff
  owner**, target "none" (type 0), tag = a per-unit counter stored in the buff; buff end (`39389B70`) ends it with that tag
  (owner type 1 only).
* Plot effect `SkillController` (`source_id`/`target_id` of the plot effect decide the unit: Lasso's 5515 is `src 4 -> tgt 3`,
  i.e. it runs **on the target** and moves it toward the caster).
* Inside `390CA700` kinds 1, 2, 3, 4, 6, 7 return immediately unless `actor->vfunc+4() == 1` (the locally controlled
  character class, vtable `399A4F68`): **the client only simulates controllers for its own player**. Remote units get
  `Net*` controllers (`390CA5F0`: net type 1 floating, 2 leap, 6 dash) that follow server movement. NPC movement comes from the
  server (AAEmu moves NPCs in its controller code). Kind 5 (rope) and 8 (flowgraph) are created for any actor.
* After creation: if `start_anim_id` is set the actor plays it (`vfunc+0x120`, flags 3); the controller keeps `desc` at `+8`.

### 1.3 Kinds and parameters (`390CA700`)

| kind | class (ctor) | parameters used (DB column names) |
|---|---|---|
| 1 Floating | `390992C0` | `value1 * 0.001`, `value2 * 0.001` |
| 2 Leap | `39095160` (0xE0) | `value1` deg -> rad (launch angle), `value2` float (min speed), `value3` int (duration ms), `value4 * 0.001` (end offset m), `value5 == 0`, `value6 == 0`, `value7` (direction), `value8 == 1`, `value9 == 1`, `value10 == 1`, `value11 == 1`, `value12 == 1`; target = unit (`390A1C20`) or position (`390A19C0`); fails (no controller) when the end point cannot be computed |
| 3 Wandering | `39094CB0` | `value1` |
| 4 Dash | `39094B50` (0x20) | `value1` = duration ms only |
| 5 Rope | `39096B90` / `39096E70` | `value1`, `value2 * 0.001`, `value3 * 0.01` |
| 6 Anchor | `390AD5F0` | three anchors (`value1..3`, `value4..6` + transition_anim_1, `value7..9` + transition_anim_2) |
| 7 Rotate | `390998B0` | `value1` deg -> rad, `value2`, `value3`, `value4 != 0` |
| 8 Flowgraph | `390959F0` | `str_value1` (graph name), `value1..9` |

**Dash (kind 4), exact 1.2 behaviour** (`39094B50` ctor, update `390A8EE0`, vtable `399A29E8`):

* ctor: `end = now + value1` ms; `actor->vfunc+0x24(0, "Dash Skill Controller")` increments the actor's input-lock counter 0
  (`390BF990`, released by `+0x28` in the dtor): **player input is locked** for the dash.
* every update until `now >= end`: build a `CMovementRequest` with `AddDeltaMovement((0, 1, 0))` (flag 0x20, local forward)
  and `PseudoSpeed = 1.0` (flag 0x800), send it to the actor's movement controller. So the dash is "hold forward at full
  run speed" in the facing direction for `value1` ms; there is no distance or speed parameter in 1.2. Collision is the
  normal living-entity movement (it stops at walls, follows the ground).
* finished flag `+0x1C` -> the actor ends it.
* Note: AAEmu's `DashSkillController` (value3 duration, value4 distance, value7 direction) is a later-version layout; the 1.2
  data only fills `value1` for kind 4 (39 plot effects, 46 skills, 20 buffs; values 0..10000 ms).

### 1.4 The six skills asked about

| skill | data | what moves, in the 1.2 client |
|---|---|---|
| Bull Rush 10501 (sc 4433, missing) | range 0..4 m, anim 28 `onehand_co_sk_shieldpush`, buff 505 (knock_down, stun, 2.3 s) | **No dash.** In 1.2 Bull Rush is "shoves an enemy away with your shield": the caster stays, the target gets the knock_down buff and is thrown by the knockdown pose (section 2). The id 4433 is dead data. |
| Mana Force 12759 (sc 732, missing) | anim 130, projectile anim 118, buffs 4813/4819/4814/4820 (1 s) | The skill controller is dead. The **buffs** carry controllers: 4813 -> 6772, 4819 -> 6773 (existing: kind 2 Leap, `0, 5, 400, -20000, 1, 1, 2, 1, 0, 1`), 4814 -> 6769 and 4820 -> 5729 (missing, so those variants do not move). So Mana Force **pushes**: the buff owner (the target) runs a 400 ms Leap with direction 2 (backward only), `value4 = -20000`; the exact end point of a buff-started Leap (its target is "none") is not decoded yet. |
| Lasso 12039 / 15027 (sc 732, missing) | channel 700 ms, anim 429 / 430 `all_co_sk_pull_launch`, plot event "당기기" | Plot effect `SkillController 5515` from target to caster: kind 2 Leap on the **target**, `value1 = 30` deg, `value2 = 40`, `value3 = 500` ms, `value4 = -1000` (1 m short of the caster), `value8..10 = 1`, start anim 49 `all_re_combat_struggle`. **Pull.** |
| Pin Down 13344 (sc 2338, missing) | anim 134 `onehand_co_sk_weapon_slash_2`, skill effects `Charge (870, 4, 6, 0)` and `(870, 8, 12, 0)`, buff 3842 | The controller is dead; any movement comes from the `Charge` special effect (type 1), not from a controller. |
| Precision Strike 12026 (sc 703, missing) | anim 132, no plot, no other movement data | **Nothing moves.** |
| Triple Slash 18132 (sc 604, missing) | plot 1528, anims 296..301 / 504..506 (`*_streakattack_1..3`), buff 141 knock_down on "1단 : 명중 연쇄", buff 3842 | **Nothing moves the caster**; hit 1 can knock the target down (buff 141, 2.5 s). |

Leap parameter meaning (from the ctor args; the update `39095160`'s vtable is not decoded yet, the AAEmu port in
`D:\aaemu-wt\bevy\rust\crates\aaemu-game\src\systems\skills\controllers.rs` `LeapArc` matches this argument layout):
`value1` launch angle, `value2` minimum horizontal speed (m/s), `value3` duration (ms), `value4` offset (mm) from the target
along the owner->target line (negative = short of the target), `value7` direction (0 both, 1 forward only, 2 backward only),
`value8` stop on overshoot, `value9` rotate to face, `value12` scale the duration by the attack-animation speed.
Open: the exact Leap update (horizontal speed cap 50, vertical arc) in the 1.2 binary.

## 2. Knockdown, knockback, getting up

### 2.1 Which buff field does what (`ClientBuffMan::ApplyAttrs` `39389440`, end `39389B70`)

Buff desc offsets: `+0x0C` anim_action_id, `+0x10` anim_end_id, `+0x14` anim_start_id, `+0x7C` (flag -> `vfunc+0x2B0`),
`+0xF8` knock_down, `+0x13C` ragdoll, `+0x18C` controller.

* `ragdoll` (69 buffs): `actor->vfunc+0x174(duration)` (`390C0B20`: character instance `+0x124(0,0,1,duration)`, ragdollize).
  `ClientBuffMan::Update` (`393888B0`) calls `vfunc+0x17C` (`390BB780`, stand up from ragdoll) **early**, as soon as the
  buff's remaining time is no longer than the stand-up clip, so the get-up ends with the buff.
* else `knock_down` (163 buffs, no clip in the data): `dir = owner.pos - source.pos` (`39386EF0`; zero when the buff is
  self-cast) and `actor->vfunc+0x180(duration_s, dir, flags)`; duration = the buff's remaining time
  (`1 200 000` s when permanent).
* else: play `anim_start_id` (`+0x14`) chosen by instrument type (`39005B20`) via `39388210`.
* `anim_action_id` (`+0x0C`) is set as an animation-graph action (`vfunc+0x130(id, 1, 0)`) and cleared at the end.
* Buff end: play `anim_end_id` (`+0x10`) — for every knock_down buff that is 47 `all_re_stop` (or 48 `all_ac_stop`).

### 2.2 The knockdown pose (`390C0B80`, player class vtable `399A4F68 + 0x180`)

```
anim = 269 all_re_combat_knockback               (falls on the back)
if |dir| > 0 and dot(dir, model forward Y axis) > 0:  anim = 276 all_re_combat_knockback_b   (attacker behind: falls forward)
stop the previous knockdown anim (+0x124), play anim (+0x120, flags) and remember it at +0x254
```

The same front/back rule is in the generic anim player `39388210` whenever anim 269 is requested (source = the effect's
caster, `dot(target forward, target - caster) > 0` -> 276). NPC classes (`390E4E70`) only store `end = now + duration`, `dir`
and flags at `+0x1F8..+0x208` and run their own timer.

### 2.3 Timing comes from the animation graph (`animations/graphs/player_full_body.xml`)

Action `all_re_combat_knockback` -> `Reaction Start` -> `all_re_combat_knockback_start` (template `action_start_no_ik`:
blend 0.2 s, play once, wait for the clip) -> `all_re_combat_knockback` (`action_match_no_ik`: blend 0.1 s, not looping,
**RepeatLastKey = hold the last frame**, lying on the ground). When the Action changes (buff end plays `all_re_stop`) the
graph leaves through `standup_nw_back` (`action_end_no_ik`: blend 0.2 s, stays until 98 % of the clip) -> `Reaction End` ->
`Main_Hub`. The `_b` chain is the same with `all_re_combat_knockback_b_start` -> `all_re_combat_knockback_b` ->
`standup_nw_stomach`. Clips: `animations/<race>/<gender>/all/all_re_combat_knockback{,_start,_b,_b_start}.caf`,
`animations/<race>/<gender>/special/standup_nw_{back,stomach}.caf`.

All these states use `MovementControlMethod horizontal="DecoupledCatchUp"`: the entity follows the clip's root motion, so the
**throw distance of a knockdown is the root motion of the `_start` clip** (no impulse in the data for Bull Rush). Rules:

1. on a knock_down buff: pick back/stomach by the dot rule, play `*_start` once (with root motion), then hold the last frame
   of the loop clip;
2. at buff end: play `standup_nw_back` / `standup_nw_stomach` (to 98 %), then idle. The stand-up starts **after** the buff
   ends (it is not part of the buff duration).
3. ragdoll buffs instead: ragdoll for the buff, start the stand-up early so it finishes with the buff.

Stun (`stun` without knock_down) uses the buff's own `anim_start_id` (e.g. 16 `all_re_combat_stun`, `_start` -> loop) and
its `anim_end_id`.

## 3. Particle effect name lookup: exact, case-insensitive, no fallback

`CParticleManager::FindEffect(name, source, source2, bLoad)` = `cry3dengine.dll 30092A00`:

1. look the **full name** up in the loaded-effects map (`300956B0`, key compare `_stricmp`: case-insensitive);
2. not found and `bLoad`: library = the text before the **first** `.` (`strchr(name, '.')`), `LoadLibrary(lib)`
   (`30091E20`, `Libs/Particles/<lib>.xml`), then look the full name up again;
3. still not found: insert a **NULL entry** for the name (so it is never searched or loaded again) and log
   `Particle effect not found: '%s', from %s %s`; return NULL.
4. found: if the effect is not loaded yet and `bLoad`, load its resources (`vfunc+0x54`); return it.

No fallback by library, by last name part or by any other rule. `wild.long_distance_shot_launch` (Missile Rain) is simply
missing in the real client. The name passed is the full `library.group.effect` path (e.g.
`abillity_skill_table_m.wild.long_distance_shot_launch`), library `abillity_skill_table_m`.

## 4. Beams / link effects

Not done yet.

## 5. Stealth look

Not done yet (`X2::GameClient::ClientUnit::ApplyStealth` = `3935C270` is the entry point).
