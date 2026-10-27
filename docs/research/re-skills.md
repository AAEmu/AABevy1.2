# x2game RE: skill controllers, knockdown, effect lookup, beams, stealth

Source: Ghidra 12.1.4 on `<client12-unpacked>\x2game_unpacked.dll` (image base `0x38FF0000`, full auto-analysis) and
`<client>\bin32\cry3dengine.dll` (base `0x30000000`, read with capstone). Data: the client DB `<client>\game\db\compact.sqlite3`
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
  and (the caster is not **rooted** **or** the controller is a Leap, kind 2) -> start it on the **caster** with the skill
  target. (Unit `+0x3A70..+0x3A7B` are 12 status flags rebuilt from the active buffs by the visitor `390B4670` /
  `390B4730`: `+0x3A70` stun, `+0x3A71` sleep, `+0x3A72` root, `+0x3A78` stealth; movement speed `39113800` is 0 while any
  of stun / sleep / root is set.)
  If `end_skill_controller` (desc `+0xC0`) is set it instead **ends** the caster's running controller (when the skill has no
  controller of its own or the running one has the same kind).
* Skill end (`39397FF0`, "end skill"): the same, for `skill_controller_at_end = true`.
* Buff start (`ClientBuffMan::ApplyAttrs`, `39389440`): buff desc `+0x18C` set and the owner not rooted -> start on the **buff
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

**Leap (kind 2)**: ctor `39095160` (vtable `399A2BF8`, update `390A2450`, end point `390A1C20` for a unit target /
`390A19C0` for a position, constraint check `39095500`). It locks the actor's input counters **0, 2, 3 and 4** (Dash locks
only 0). Parameters (the AAEmu port `<aaemu-src>\rust\crates\aaemu-game\src\systems\skills\controllers.rs` `LeapArc`
uses the same layout and agrees on every part checked here): `value1` launch angle (deg), `value2` minimum horizontal speed
(m/s), `value3` duration (ms), `value4` offset (mm) along the owner->target line (negative = short of the target), `value7`
direction (`39095500`: 1 = fail when the end point lies behind the owner, 2 = fail when it lies in front), `value8` stop on
overshoot, `value9` rotate, `value12` animation-speed scaling (`39095440`: `scale = max(attr 119, -1000) * 0.001 + 1`; min
speed `*= scale`, duration `/= scale` when `scale >= 0.01`). A self-targeted leap goes along the owner's forward; with a
negative offset the end point is clamped to the terrain under it. The per-frame arc (`390A2450`, drives the living entity
with a velocity request, flags `0x220000`) was not decoded in full; use the server's `LeapArc` for the curve.

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

### 4.1 Projectile classes (`ProjectileManager`, factory `393F1160`)

`projectiles` rows (loader `3979DC80`): `dest_bone_id`, `fx_group_id`, `ignore_z_rotation`, `is_permanent`,
`proj_physic_id`, `src_bone_id`. `proj_physic_id` picks the class:

| physic | vtable / update | behaviour |
|---|---|---|
| 0 | `399D51F4` / `393EF270` | homing: moves from the source toward the **current** target point, step `(dist / time_left) * dt` (capped at dist), oriented by the shortest arc from +Y to the direction; ends within 0.3 m or at its end time |
| 1 | `399D5218` / `393EFB50` | ballistic (gravity arc); `ignore_z_rotation` flattens its facing to XY |
| 2 | `399D5260` / `393F0410` | **beam / link** (30 rows: Arc Lightning 66, Telekinesis / Crippling Mire 38, Lasso 86, Mana Force 118, ...) |
| 3 | `399D523C` / `393F02E0` | static: placed once at the target point, identity rotation (`393F0120`) |

Bone ids (`393E7C40`, `393E7D50`): 0 `bip01`, 1 `bip01 spine`, 2 `bip01 head`, 3 `bip01 neck`, 4 `bip01 L Hand`, 5 `bip01 R
Hand`, 6 / 7 L / R Foot, 8 `item_hand_l`, 9 `item_hand_r`, 10 `effect_1`, 11 `bone_center`, 12 `bone_string`, 13
`bone_string_b8`, 19 `Bip01 Hit`, 20 `attach_form`, 21 `attach_launch`, 22..24 `bip01 spine1..3`, 25 / 26 the `mainhand`
attachment's `effect1` / `effect2`, 27 / 28 `offhand` `effect1` / `effect2`, 29 / 30 `item_side_l` / `_r`; anything else
`bip01` (full table and rules in 4.4). Of the 30 beams, 23 have `src_bone_id` 0, four 9, one each 1, 8 (Telekinesis) and
19; 25 have `dest_bone_id` 19 (`Bip01 Hit` of the target), three 1 and two 0.

### 4.2 The beam update (`393F0410`, every frame while alive)

```
src = world position of the caster's src bone   (393EDBE0; a weapon attachment's bone when the id names one)
dst = world position of the target's dest bone  (393EDC70; the point itself for position targets)
d = dst - src;  L = |d|;  dir = d / L
q = shortest-arc rotation from +Y (0,1,0) to dir   (180 deg flip when 1 + dot(+Y, dir) <= 0.0001)
every fx item: world TM = Matrix34(scale (1,1,1), q, src)   (393EE850 -> item vfunc +0x18)
every fx item: stretch  = Vec3(1, L, 1)                     (393EAEE0 -> item vfunc +0x1C)
```

So the link's **long axis is the effect's local +Y**, the effect sits on the **caster's** bone and is re-aimed every frame,
and the length is not a uniform scale: the particle item stores `(1, L, 1)` (`FxParticle::SetScale` `393E2D80`, item
`+0x450`) and hands it to the emitter in the last three floats of its `SpawnParams` (`GetSpawnParams +0x28`,
`SetSpawnParams +0x24`; an AA addition to the CryEngine struct, applied inside cry3dengine, not decoded there). The beam
lives until its end time = the `ProjectileAnim` special effect's `value2` ms (Lasso `(86, 700, ..)` = the 700 ms channel,
Crippling Mire `(38, 4500, ..)`), or forever with `is_permanent`; then `393EF0A0` sends fx event 5 (end) to its items. The
fx items of these beams use start event 0, end event 5.

### 4.3 Held / looping effects

The client never re-triggers a finished particle. `FxParticle::Start` (`393E7340`) looks the effect up
(`FindEffect(name, "", "", true)`, section 3) and takes "loops" from the **effect itself** (its params byte `+1`, or the
effect's `vfunc+0x2C | vfunc+0x30` when a global option is 0). Looping effects join a global list that
`FxParticle::UpdateLoopParticles` (`393E78E0`, called from `FxGroupManager::Update` with a budget of **50**, or a cvar / 2)
sorts by distance to the camera: only the closest N keep an emitter (items with `+0x204 == 1` always keep theirs), the rest
are killed and respawned when they come back into the budget. Non-looping effects spawn once. So a `*_launch` /
`*_form_end` effect repeats during a channel **only if its library entry is continuous**; the beam merely keeps moving and
stretching whatever emitter is alive until the end event.

### 4.4 Effect bone ids

There are two copies of one id -> name switch:

* `393E7C40` (returns a bone name). Used for `fx_items.bone_id` (`393E8270`), the projectile `dest_bone_id` (`393F1160`)
  and the sound position (`393F1F50`, `SoundManager::GetUnitSoundPos`).
* `393E7D50` (outputs a bone name and an attachment name). Used for the projectile `src_bone_id` (`393F1160`) and
  `3938DFE0`. It is the only one that knows ids 25..30.

Names are looked up through the skeleton's joint-by-name call, case-insensitively, because the table writes
`bip01 spine` and the rigs have `Bip01 Spine`.

| id | `393E7C40` (fx items, dest bones) | `393E7D50` (projectile source): attachment / bone |
|---|---|---|
| 0 | `bip01` (the default) | - / `bip01` |
| 1 | `bip01 spine` | `bip01 spine` |
| 2 | `bip01 head` | `bip01 head` |
| 3 | `bip01 neck` | `bip01 neck` |
| 4 | `bip01 L Hand` | `bip01 L Hand` |
| 5 | `bip01 R Hand` | `bip01 R Hand` |
| 6 | `bip01 L Foot` | `bip01 L Foot` |
| 7 | `bip01 R Foot` | `bip01 R Foot` |
| 8 | `item_hand_l` | `item_hand_l` |
| 9 | `item_hand_r` | `item_hand_r` |
| 10 | `effect_1` | `effect_1` |
| 11 | `bone_center` | `bone_center` |
| 12 | `bone_string` | `bone_string` |
| 13 | `bone_string_b8` | `bone_string_b8` |
| 14..18 | `""`, an empty name | `bip01`, the default |
| 19 | `Bip01 Hit` | `Bip01 Hit` |
| 20 | `attach_form` | `attach_form` |
| 21 | `attach_launch` | `attach_launch` |
| 22 | `bip01 spine1` | `bip01 spine1` |
| 23 | `bip01 spine2` | `bip01 spine2` |
| 24 | `bip01 spine3` | `bip01 spine3` |
| 25 | `bip01` (default) | `mainhand` / `effect1` |
| 26 | `bip01` | `mainhand` / `effect2` |
| 27 | `bip01` | `offhand` / `effect1` |
| 28 | `bip01` | `offhand` / `effect2` |
| 29 | `bip01` | `item_side_l` |
| 30 | `bip01` | `item_side_r` |
| other | `bip01` | `bip01` |

Rules around the table:

* **Id 0 is the joint `bip01`, nothing else.** It is not a weapon hand, not an attach point by kind and not the
  entity origin. Only ids 25..28 involve a weapon. With them the position is the bone `effect1` / `effect2` inside the
  object in the unit's `mainhand` / `offhand` attachment (`390B8210`, attachment branch).
* **Fx item override** (`FxGroupManager` `393E8270`): the creator receives an optional bone name and attachment name
  from the event. When that name is non-empty **and the item's `bone_id` is 0, 8 or 9**, the item goes on the passed
  name instead of its own id. The beam and projectile start event passes the projectile's **source bone** (`393EFA28`:
  event 0 with the `+0x64` source bone and the `+0x24` attachment).
  * So the launch particle of a beam, which is stored as bone 9 `item_hand_r` in all beams checked (Arc Lightning 818,
    Lasso 802, Absorb Lifeforce 733, Telekinesis 105), is moved onto the projectile's source bone. For `src_bone_id` 0
    that is `bip01`.
  * Ids 20 / 21 (`attach_form` / `attach_launch`): if the unit's character has an **attachment** of that name, the item
    is bound to the attachment (flag `+0x194 = 1`). Otherwise the name is used as a bone.
* **Bone position** (`390B8210(src_unit, unit, bone, attachment, out)`, used by the beam source `393EDBE0`, with
  `src_unit` 0, and by the beam target `393EDC70`):
  * attachment given: that attachment's object joint, through the attachment TM. If the attachment is missing, the
    entity position.
  * empty bone name: `bip01 spine`. An empty name is what ids 14..18 give.
  * otherwise: entity world TM x the joint's absolute pose.
    * If the joint is one special joint (skeleton `vfunc+0x1C(0x2F)`) and a source unit is given, the result is the
      joint, from a per-skeleton list (`vfunc+0x114`), that is **closest to the source unit**. This is most likely the
      `Bip01 Hit` hit-point set, for target bones.
    * Joint not found: a point from the source unit toward the entity, or the entity position when there is no source
      unit.
* **Fx item position** (`393E35E0`): world TM x the joint's absolute pose, then the item offset. If the joint is not
  found (index -1), the item sits on the **entity origin** plus its offset.

**Where `Bip01` is.** The bind pose in the `.chr` (nuian male `nu_m_base.chr`, z up) puts `Bip01` at `(0, -0.006, 1.037)`.
`Bip01 Pelvis` is also at 1.037, `Bip01 Hit` at 1.366, `item_hand_r` at 1.024 and the head at 1.733. **But every
clip moves the root to the ground.**

| clip | `Bip01` position track | `Bip01 Pelvis` position, first key |
|---|---|---|
| `all_co_sk_swim_spell_cast_site` | (0, 0, 0) | (0, 0, 1.075) |
| `fist_pos_sit_crouch_idle` | (0, 0, 0) | 0.298 |
| `fist_mo_normal_run_fl` | (0, 0, 0) -> (-2.55, 2.55, 0), root motion | 0.899 |

The height lives in the pelvis track. While any animation plays, the CryEngine 2 runtime pose has joint 0 `Bip01` on
the ground under the character, apart from not-yet-extracted root motion. So x2game's id 0 resolves to the **feet**,
not the pelvis, in any animated pose. Only an unanimated bind pose has it at 1.04 m.

Consequence: a beam with `src_bone_id` 0 (23 of 30, including `분노의벼락_빔` 66 and `연쇄번개_beam` 87, Lasso `당기기_beam` 86,
Absorb Lifeforce `생명력흡수_beam` 81 and Mana Force `활력추출_beam` 118) starts at the joint `bip01`, and its
launch particle is moved there too. The particles add no offset: `magic.lighting_serial_magic_launch` has none,
and `death.life_absorb_launch` / `fight.full_launch` have `PositionOffset` `(0,-1,0)`, which is along the beam axis.
This is what x2game computes. It has **not** been checked against a real-client capture of a beam; no
`reference/` capture shows one. If the real client draws these beams from the hands or chest, the reason is in
cry3dengine or CryAnimation (how the root joint's absolute pose is reported), not in x2game's table.

The port's tables, in this worktree:

* `crates/aa-client/src/ingame/combat/skills.rs` `bone_names` (around line 823).
* `crates/aa-client/src/ingame/combat.rs` `BEAM_ROOT` / `bone_world` (around line 1909, id 0 -> `Bip01` for beam
  sources).

Differences from x2game:

| id | port | x2game |
|---|---|---|
| 1 | `Bip01 Spine1`, then `Bip01 Spine` | `bip01 spine` |
| 2 | (same) | `bip01 head` |
| 3 | `Bip01 Spine2` | `bip01 neck` |
| 4 | `Bip01 Pelvis` | `bip01 L Hand` |
| 5 | `item_hand_r` | `bip01 R Hand` |
| 0, 10..13, 20..30 | missing | as in the table above |
| 0 on fx items and dest bones | the model origin | joint `bip01` (on the ground when animated, so the same in practice) |

The port also lacks the 0 / 8 / 9 fx item override by the projectile's source bone.

## 5. Stealth look

`ClientUnit::ApplyStealth` (`3935C270`, on every update):

* `stealthed` = unit `+0x3A78` (any active buff with `stealth`).
* `hidden` = (`+0x3C59`, the fade timers `+0x3C5C` / `+0x3C60`, `+0x3C5A` or `+0x3BBB`) or (`stealthed` and the local
  player does **not** see the unit). `3935A2B0(local, unit)` sees it when it is the local player itself or its own pet
  (`+0x3AF0`), when it is friendly (`39308CA0`, unless `39358AB0`), or when it is within the detection range: the front or
  back range (`detect_stealth_range_front` / `_back`, chosen by whether the unit is in front of the local player's facing),
  scaled by `(attribute 0x5E + 100) %`.
* `actor->vfunc+0xF0(hidden)` hides / shows the model (skipped for the local player while a camera condition holds), then
  `actor->vfunc+0xE8(stealthed)` sets the look.

The look (`390C0240`, character class only; NPC classes have an empty `+0xE8`, so a stealthed NPC is only hidden or
shown): `IEntityRenderProxy::SetMaterialLayersMask(stealthed ? 4 : 0)` (render proxy = `entity->GetProxy(0)`,
`vfunc+0x54`); 4 is CryEngine's **`MTL_LAYER_CLOAK`**. It also toggles an entity flag (`vfunc+0x1C(2)` when leaving
stealth, `vfunc+0x20` when entering; probably the cast-shadow flag, not confirmed). **The same look is used for the local
player, allies and detected enemies**; x2game has no per-relation colour.

The cloak layer is the Crysis shader `game/shaders/hwscripts/cryfx/cloaklayer.cfx` (copy in `data/extract/ghidra/shaders/`),
two passes:

1. `CloakRefrationPS`: alpha blend (`SRC_ALPHA, ONE_MINUS_SRC_ALPHA`), z-test, no z-write, back-face cull.
   * Screen-space refraction of the scene: offset `normal.xy * 0.025 * cloak_amount * interlation.x` (or, at higher
     quality, a 3D refracted ray `pos + refract * 0.05 * cloak_amount * interlation.x`); red is sampled with a `1.01`
     scale around the centre (slight colour split).
   * `interlation` = `textures/defaults/palette/cloak_interlation.dds` at `s = (screen_y + random) * screen_height * 0.25`
     (scanlines; G channel 0.80..0.95).
   * colour `= refr + (1 + TimeOfDay.x) * palette(interlation.y * (1 - N.E)) * light_amount * HDR_MAX * (1 - N.E)` with
     `palette` = `textures/defaults/palette/cloak_palette.dds` (256 x 1 RGB): black at 0, dark red `(58,0,0)` at 80,
     magenta / purple `(59,8,32)`..`(50,18,74)` at 128..160, violet `(37,30,122)` at 192, blue `(30,76,246)` at 208, light
     blue to white `(78,127,255)`..`(166,192,255)` at 224..240. Faces turned to the camera are pure refraction
     (see-through); the **rim** runs dark red -> magenta -> blue -> white with the grazing angle (the red-pink look).
   * alpha `= cloak_blend_amount (CloakParams.w) * diffuse.a`.
2. `CloakSparksPS`: additive (`ONE, ONE`), no z-test, vertices pushed along the normal by `0.0025 + 0.01 * sin(4 t + 20
   |pos|)`; sparks from three `Noise3D.dds` lookups through `cloak_sparks.dds`, `* SparksColor (0.3, 0.3, 1.0)` squared,
   `* interlation.w * max(speed_scale, transition^4)` (`cloak_transition.dds`), soft depth test, `* light_amount`: the
   **blue crackle**.

`CloakParams` (`PI_CloakParams`): x speed scale (sparks), z light amount, w cloak blend amount; the renderer fills them from
the per-instance material-layer blend (x2game does not set them). Port rule: draw any stealthed character the local player
can see with this two-pass material (blend about 1), self, allies and detected enemies alike; hide undetected ones.

## 6. Open points

* Leap per-frame arc (`390A2450`) and the end point of a buff-started Leap (Mana Force's 6772 / 6773, target "none").
* How cry3dengine applies the AA `SpawnParams` stretch vector `(1, L, 1)` to the emitter (positions, sizes or both).
* Clip lengths, read from the `.caf` files (nuian male): `all_re_combat_knockback_start` 1.17 s, `_knockback` loop
  0.33 s, `_knockback_b_start` 1.17 s, `_knockback_b` 0.33 s, `standup_nw_back` / `_stomach` 0.50 s each.
* Whether CryAnimation reports joint 0 `Bip01` at the ground while animated (section 4.4). The clips put it there, but
  no real-client beam capture confirms it.
* The special joint `vfunc+0x1C(0x2F)` and its closest-joint list in `390B8210` (probably the `Bip01 Hit` set).
* What the entity flag toggled by the stealth look is (`vfunc+0x1C(2)` / `+0x20`).
