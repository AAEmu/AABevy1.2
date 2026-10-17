# aa-client combat visuals

What a skill looks like on the units that use it and the ones it hits: cast and attack clips, particle effects at the right
bones, flying arrows, hit reactions, death. Code: `crates/aa-client/src/ingame/combat.rs` (events to clips and effects),
`combat/skills.rs` (the database side), `combat/clips.rs` (anim events of a clip), `weapons.rs` (weapon in the hand).
The server only reports results (`aa_net::Event`, see `docs/aa-net.md`, "Combat events for visuals"); everything visible is derived
from the 1.2 game database and the model's own animation data.

## How 1.2 data links a skill to its visuals

```
skills.start_anim_id ------+                      cast clip, loops until the skill fires        (cast time > 0)
skills.fire_anim_id -------+--> anims.name  --->  clip of the caster's .cal (bow_co_sk_launch)
skills.twohand_fire_anim_id|                      (SCSkillFired.fire_anim_id is the same table: the server's choice wins)
skills.dual_wield_fire_anim_id
skills.fx_group_id --------> fx_group_fx_items --> fx_items --> fx_particles   particle effects of the skill
skills.projectile_id ------> projectiles ---------> fx_group_id (what flies), src_bone_id, dest_bone_id
skills.plot_id ------------> plots / plot_events -> plot_effects -> special_effects   one clip / effect group / projectile per step
skills.effect_delay (ms) -----------------------> the shot leaves this long after the fire (358 ms for Shoot Arrow)
```

* **Clips.** `anims.name` is a clip name of the caster's `.cal` (`animations/nuian/male/bow/bow_co_sk_launch.caf` through the
  `.cal`'s `AnimationList`). Players and monsters use the same names: the rat bites with `fist_co_attack_r` (`anims` id 1, skill 2
  "melee attack"), the player's Shoot Arrow (16064) casts `bow_co_sk_cast` (31) and fires `bow_co_sk_launch` (32). A cast skill
  (casting_time > 0, e.g. 10594 Return, 10 s) loops its `start_anim` (`all_co_sk_buff_cast_mental`) until `SCSkillFired`.
* **Plots.** 1,328 of the 15,126 skills have a `plot_id`; every player weapon skill is one (the server's `skills.plot_id`). The
  server then sends `SCPlotEvent { event_id }` per step instead of one `SCSkillFired`. `plot_effects.actual_id` ->
  `special_effects` of type 34 (`Anim`: `value1` = `anims.id`), 35 / 36 (`FxGroup` / `FxGroupAnim`: `value1` = `fx_groups.id`),
  37 / 38 (`Projectile` / `ProjectileAnim`: `value1` = `projectiles.id`). 450 steps carry two anims: `all_co_sk_stop` (id 46; or
  `all_ac_stop`, 48), which only ends the clip before it and is in no `.cal`, then the clip: the step plays the last anim that is not a
  stop (`PlotVisual::anim`; before, the stop won and the clip was lost). 3-hit slash 18132: steps 12130 / 12132 / 12133 play
  `onehand_co_sk_streakattack_1 / _2 / _3` (the skill's own `fire_anim_id` 296 is stage 1; `twohand_*` 299 and
  `twoswords_*` 504 are the other weapon classes), the hit check steps (12144, 12148, ...) carry nothing.
* **Effect groups.** `fx_items`: `asset_name` is the aa-fx effect (`library.group.name` of `game/libs/particles/*.xml`),
  `fx_event_start_id` / `fx_event_end_id` the moment of the skill it starts and ends at, `fx_location_id` 1 caster / 2 target,
  `bone_id` where, `offset_*` (cry axes), `offset_axis_id` (4 = turn the effect towards the attacker), `fx_particles.scale`.
  Only `FxParticle` details are drawn here (sounds, voices, camera shakes, `FxMaterial` flashes are left out).

  | id | moment | seen on |
  |---|---|---|
  | 0 | START: the server accepted the skill | `_dot` buff visuals (ignored: they belong to the buff) |
  | 1 | FORM: the cast begins (charge-up around the hands) | `abillity_skill_table_m.system_skill.return_skill_form` |
  | 2 | THROW: the cast is complete (draw the arrow) | |
  | 3 | LAUNCH: it fires (the swing, the shot) | the weapon trails |
  | 4 | HIT: it lands on the target | `st1000c` arrow hit, `fight.hitup` |
  | 5 | END | everything that is not instant ends here |
  | 6 | PROJ_END: the projectile arrives | |

  Bones: 8 `item_hand_l`, 9 `item_hand_r` (the weapon bones; both read off the anim events of the clips), 6 / 7 the feet,
  19 `Bip01 Hit` (where a blow lands: monsters have it too), 1 to 5 body / head / pelvis, 0 the unit's origin. The mapping is
  `combat/skills.rs::bone_names`; rigs without a bone fall back to the unit's origin raised by its height.
* **Projectiles.** `projectiles.fx_group_id` is what flies: Shoot Arrow's arrow is group 115 = one effect `p_effect.st1000b` on
  bone 8 (the bow hand) starting at moment 0; `dest_bone_id` 19 is where it ends. Group 116 (the skill's own) has the hit effect
  `st1000c` on the target bone 19, axis 4. **The server sends no projectile packet**: the flight is derived from
  `SCSkillFired.delay_ms` (the time the effects take to land: 600 to 720 ms for the arrow at 8 to 17 m; 0 for melee) and the
  moment the arrow leaves, the clip's anim event.
* **Anim events.** Every character `.cal` names an `$AnimEventDatabase` (`animations/nuian/male/event_database.animevents`,
  XML). `<animation name="...caf"><event name="skill_effect" time="0.358" parameter="launch" bone="item_hand_l"/>` says when in
  the clip the arrow leaves (`launch`), the hand is drawn (`throw`), the charge starts (`form`); `combat_sync` marks where the
  server's skill timeline sits in the clip (the swing connects: 0.487 s for the rat's bite, 0.358 s for the bow). Monster
  databases name the folder of the base model (`animations/animals/rat/...` for `rat_black`): clips match by file name.
* **Reactions.** `all_re_combat_hit`, `all_re_combat_critical` (hit reactions, any model), `dead_01` (death). All come from the
  `.cal`, so every model that has them reacts the same way.

## What the client does (`ingame/combat.rs`)

| Server event | Visual |
|---|---|
| `SkillStarted` with `cast_ms > 0` | the skill's `start_anim` loops on the caster (`Acting`), the local player turns to the target, FORM effects play on the caster (held until the skill fires or stops) |
| `SkillStarted` refused with `OUTOF_ANGLE` (0x10) | the local player turns to the target, sends a standing `Move` with the new facing and asks again once (the real client turns on its own) |
| `SkillFired` | the one-shot fire clip (the id the server sends, else the skill's), the player turns to the target; THROW effects at once, LAUNCH effects and the projectile at the clip's release moment (anim events, else `effect_delay`); an instant skill also plays the START / FORM effects that were skipped (effects that die at LAUNCH are not shown) |
| `PlotEvent` | the step's clip / effect group / projectile (a step that changes the clip ends the cast loop) |
| `UnitDamaged` (kind 0 / 1) | HIT effects of the cast's group on the target at the moment the blow lands: `fired + delay_ms` for a flying shot, `fired + combat_sync` for a melee swing, at once for a plot step; then the target's hit reaction (0.5 s flinch, never interrupts an action or a run; critical hits use the critical clip); a miss only shows the clip |
| `UnitDeath` | after the last scheduled hit on the unit: `dead_01` once, then it stays on the last frame (`Dead`: locomotion never takes the unit back) until the server removes it |
| `Resurrected` | `Dead` / `Acting` removed, locomotion restarts |
| `SkillEnded` / `CastingStopped` / `Plot*` | the effects held until END are released; an interrupted cast returns to locomotion |
| a unit that spawns dead (`Unit::dead`) | lies on its last death frame at once |

* **Locomotion** (`ingame/anim.rs`): `drive_locomotion` skips units that are `Acting` or `Dead` and restarts its clip when the
  action ends (`Locomotion::invalidate`). Rats and foxes use the same code: their `fist_co_attack_r` bite plays on `SkillFired`
  of skill 2 and the hit lands on the player at `combat_sync`.
* **Effects** are aa-fx `Effect` entities (`prime: false`): held ones loop and follow their bone every frame (`follow_fx`, before
  transform propagation) until their END moment, short ones despawn themselves. A hit effect with axis 4 is turned towards the
  attacker and keeps that orientation. An effect that never finishes dies after 12 s.
* **Arrows**: the projectile is an anchor entity flying caster launch bone -> target `Bip01 Hit` over the flight time (homing: the
  end follows the bone), carrying the effect of group 115. aa-fx does not draw `Geometry` emitters (the arrow body of
  `st1000b` is a mesh emitter), so the client draws the `.cgf` the effect names (`EffectDef::geometry`, new in aa-fx) as an
  additive, unlit model, widened as the emitter's `SizeX` asks. When the arrow arrives, its glow fades out in 0.6 s.
* **Weapons** (`weapons.rs`): a character that fights takes the weapon out: the bow comes into the left hand (`item_hand_l`,
  turned 90 degrees about z so that the limbs stand upright) for the bow skills, a sword into the right hand for any other
  weapon clip, and both go back to their sheath bones 4 s after the last action (`Drawn`). The shield stays on the arm. The
  `drawn_scale` of `item_weapons` applies in the hand.
* **Moving casters** (`Acting::layered`): a unit whose speed is above 0.3 m/s plays a cast, fire or plot clip on its upper body
  (aa-anim's `Animator::play_layer` with `BoneMask::upper_body`, `docs/aa-anim.md`) while `drive_locomotion` keeps running the
  8-way run / walk clips underneath (the layer ignores the run clip's playback speed). The clip is the `move_ub` variant of the
  skill's `anims` row (`SkillBook::upper_clip`; `bow_co_sk_launch` -> `bow_co_sk_launch_ub` -> the model's `bow_co_sk_launch_mub`,
  else `_ub` as is), else the clip itself through the mask. A standing unit plays the whole-body clip as before; a unit that stops
  mid-action keeps its upper body clip over the idle, one that starts moving mid-action keeps its whole-body clip. A one-shot
  layer ends by itself, a cast loop is released (`release_layers`, 0.12 s fade) when `Acting` ends or the fire clip replaces it,
  and a layered action never invalidates the locomotion state (no restart of the run cycle). Hit reactions stay whole-body and
  never interrupt a run. `AA_COMBAT_FULLBODY=1` turns the layer off (to compare shots).
* **Cost.** Skills are read from the database once and cached by id (`SkillBook`, plus the effect group, anim and plot step
  tables); a frame touches no database. Per frame: `follow_fx` / `fly_projectiles` iterate the live effect entities (a handful),
  `hold_weapons` the carried weapons (3 per character), `end_actions` the `Acting` units. Measured with `AA_FX_LOG=1` during a
  fight with arrows: aa-fx `sim_ms` about 0.03 ms, `upload_ms` 0.01, with the world's 7 ambient effects; the frame time with and
  without the combat code is the same within the noise (about 80 to 110 fps in the dev build on this box, 10 to 12 ms).

Upper body layer frames (`shots/aa-anim/`, not committed; `--walk 9:12` runs the player from second 9 while slot 3 fires at 9.5,
`--cam-dist 5 --cam-pitch -12`, `--cam-yaw 120` for the arrow pair and 40 for the casts): `walk_arrow_before.png` (`AA_COMBAT_FULLBODY=1`: the legs stand planted in the
shooting pose while the player slides forward) and `walk_arrow_after.png` (the arms draw the bow, the legs keep their run stride),
`cast_standing.png` (Return from standing: whole-body cast, unchanged) and `cast_walking.png` (Return while running: the cast on
the upper body, the legs running).

## What it was checked against

There are no real-client combat captures (`reference/` has none), so the check was the data itself and the server's behaviour on
lane `bevy` (account `bevytest`, the character with Shoot Arrow on slot 3, the 3-hit slash on 2, the melee attack on 4, Return on
slot 22 and Angry Rats / Solzreed Foxes around the spawn). Dev tools (all environment variables, off by default):

* `AA_COMBAT_LOG=1`: one line per combat event, clip, projectile and unit death.
* `AA_COMBAT_FULLBODY=1`: moving units play action clips on the whole body (the behaviour before the upper body layer).
* `AA_COMBAT_SLOWMO=0.3`: the game clock slows to that speed when the local player first fires (catches a flight on screen).
* `AA_COMBAT_SHOT=<png>@<start|fire|fire2|plot|hit|death>+<s>`: screenshot that many game seconds after the first such event and
  exit; combine with `--hud-key tab T --hud-key slot3 T` (see `docs/aa-client-hud.md`).

```
AA_COMBAT_SHOT=shots/combat/arrow.png@fire+0.5 AA_COMBAT_SLOWMO=0.3 aa-client --enter-world --hour 12 --pos 15514,15330 \
  --yaw -45 --cam-yaw 40 --cam-dist 6 --cam-pitch -18 --hud-key tab 8 --hud-key slot3 9.5
```

Frames (`shots/combat/`, not committed):

| file | shows |
|---|---|
| `final_bow.png` | 0.3 s after Shoot Arrow fired: the bow in the left hand, arm out, limbs upright, the shield on the arm (`bow_co_sk_launch`) |
| `final_arrow.png` | 0.5 s after the shot: the arrow body (thin cream line, top left, over the target frame) flying from the bow hand towards the fox, the player in the release pose |
| `cast_a.png` | mid-cast of Return (10 s): `all_co_sk_buff_cast_mental` loops, the hands together at the chest |
| `swordr_0_0_0_0_0_0.png` | stage 1 of the 3-hit slash (`onehand_co_sk_streakattack_1`): the sword out of its sheath in the right hand, raised |
| `corpse_c.png` | a fox lying on its side after `UnitDeath` (a second fox still stands at the top) |
| `dead_a.png` | the Angry Rat of the pond died (`unit died` in the log); it lies under the water surface, so only its name plate shows |

Matched: the clip names and timing (launch 0.358 s into `bow_co_sk_launch`, bite sync 0.487 s), the effect bones (bow hand for the
arrow, `Bip01 Hit` for the hit spark), flight time and hit moment from `SCSkillFired.delay_ms` (the hit reaction starts when the arrow
arrives, 0.3 to 0.7 s after the shot), and the rat / fox attack clips on the player. Not matched against a real capture: the hit sparks (`st1000c`, `fight.hitup`) and the Return charge-up glow do spawn (aa-fx counts
the extra effects) but are short and small in the frames; effect
brightness, the arrow's glow (the mesh is a flat additive model; the particle trail of `st1000b` is aa-fx's), and the exact hand
pose of the bow.

## Coverage

How many skills would look right, measured from the data alone (no server, no window):

```
cargo run -p aa-client --example skill_coverage [-- --out data/extract/combat/skill_coverage.tsv]
```

The example (about 3 s once built) includes `combat/skills.rs` and `combat/clips.rs` as they are, so every skill is resolved by the
code the client runs (`SkillBook`, `upper_candidates`, `ClipEvents::of`; a few `pub(crate)` entry points were added for it:
`plot_steps`, `fx_group`, `fx_extras`, `projectile`). It writes one row per skill to `data/extract/combat/skill_coverage.tsv` (clips
with their roles and the models that lack them, `miss_<model>` counts, stop markers, unresolved `move_ub` variants, looping clips,
event gaps, effect class, missing / look-changing / invisible effects, features, hints, sound / voice / shake / material counts) and
prints the tables below.

**Skills.** Database `compact.sqlite3`, tables `skills`, `default_skills`, `localized_texts` (English names), `anims`, `plots`,
`plot_events`, `plot_effects`, `special_effects`, `fx_groups`, `fx_group_fx_items`, `fx_items`, `fx_particles`, `projectiles`,
`skill_visual_groups`, `skill_controllers`, `holdables` + `item_weapons` (the weapon attack clips of the auto attacks 2 / 3 / 4,
which the server picks per weapon), and for monsters `npcs`, `npc_spawner_npcs`, `np_skills`, `models`, `actor_models`.

* **Player skills, 231** = the ten ability trees (`skills.ability_id` 1 to 10: 170 skills, 110 of them `show`n in the skill window,
  60 hidden sub skills such as call skills and weapon variants) + the 61 `default_skills` (15 basic: melee attack, kick, Shoot
  Arrow, Return, instruments, glider, ...; 46 emotes). This DB has no racial skill table: nothing ties a skill to a race, so the
  race axis is the 8 models (nuian / elf / hariharan / ferre x male / female, the `.cal` of `<race>/<gender>/nude/<pfx>_base.cal`).
* **Monster skills, 370** (not already player skills): `base_skill_id` and `np_skills` of the NPCs that have a spawner and level 1
  to 12 (the starting areas are not in the DB; the level is the stand in). Their clips are checked against the model of each NPC
  that uses the skill.

**Clips** are `start_anim` (when the skill casts or is a plot), `fire_anim`, `twohand_*`, `dual_wield_*`, `channeling_anim`, every anim of every plot step, the weapon attack clips, and the
instrument and skill controller anims (the last two are not played by the client yet). "Present" means: a `.cal` entry, the `.caf`
in the pak, and it decodes and binds to the model's skeleton (`ClipLibrary::bound`). `all_co_sk_stop` and `all_ac_stop` are
markers ("end the clip before"), not clips: no `.cal` has them.

**Effects** are all `fx_items.asset_name` of the skill's group, its projectile's group, its plot steps and its per level groups
(`_dot` buff visuals left out, as the client does), looked up with aa-fx's own `Library::load` / `get`. What aa-fx cannot do is read
off the raw emitter XML of the effects that exist: an attribute `EmitterDef::from_attributes` never reads (found by feeding the
parser test values), a `Geometry` model, a `.mtl` with `RefrBumpScale`. aa-fx logs nothing for ignored attributes. The features
marked `*` change how an effect looks; the rest (a light, a sound, a rendering detail) leave its sprites as they are.

| | player (all) | tree, shown | basic + emote | monsters |
|---|---:|---:|---:|---:|
| skills | 231 | 110 | 61 | 370 |
| with clips (incl. instrument / controller anims) | 205 | 101 | 47 | 244 |
| **all clips present on all 8 models** (monsters: on their models) | 205 | 101 | 47 | 230 |
| clips missing on some model | 0 | 0 | 0 | 14 |
| clips missing on nu_m / nu_f / el_m / el_f / ha_m / ha_f / fe_m / fe_f | 0 each | 0 each | 0 each | n/a |
| uses a stop marker (no clip) | 22 | 17 | 0 | 30 |
| `move_ub` variant unresolved on the model (plays the whole clip through the mask) | 31 | 21 | 2 | 49 |
| with particle effects | 173 | 110 | 5 | 219 |
| **effects all found, nothing ignored by aa-fx** | 5 | 5 | 0 | 4 |
| effects all found, minor loss only (light / sound / detail) | 10 | 7 | 2 | 34 |
| effects all found, at least one looks different (`*` features) | 156 | 96 | 3 | 180 |
| **effects missing** (at least one) | 2 | 2 | 0 | 1 |
| at least one effect invisible (a mesh / trail / orbit and no sprite) | 10 | 7 | 0 | 16 |
| uses a sound (any kind) | 168 | 109 | 4 | 222 |
| ... `FxSound` item / sound bound to a particle item / emitter `Sound` | 26 / 151 / 12 | 7 / 109 / 2 | 0 / 1 / 3 | 18 / 207 / 20 |
| voice (`FxVoice`) | 137 | 97 | 2 | 54 |
| camera shake | 22 | 11 | 0 | 18 |
| material flash (`FxMaterial`) | 24 | 9 | 2 | 34 |
| **channeling** (channeling time or clip) | 12 | 10 | 2 | 10 |
| **plot skills** | 40 | 31 | 1 | 19 |

Distinct things of the player skills: **202 clips needed, none missing on any model** (the elf and hariharan `.cal` take the player clips of
`global_player.cal` from the Nuian folders, and those files exist); **356 effects, 354 found** (37 plain, 50 with a minor loss, 267 that look different, of which 9
draw nothing at all; 17 have nothing to draw by design: every emitter is switched off, or it only carries a sound or a light), 2
missing. Of the 2,364 switched on emitters of the 354 effects, 674 (28 %) carry a feature that changes the look; the other 72 % draw as
authored. Items that are not particles: 49 `FxSound`, 641 sounds bound to particles, 231 voices, 46 camera shakes, 55 material flashes.

**Top missing clips (player skills): none.** The only names that no `.cal` has are the two stop markers (`all_co_sk_stop`, 20 skills;
`all_ac_stop`, 4). Monsters (skills that use a clip their model's `.cal` lacks): `all_co_sk_spell_launch_d` 3, then 1 each:
`all_co_sk_pull`, `all_co_sk_spell_cast_d`, `all_co_sk_spell_cast_missile`, `all_co_sk_spell_channel_nd`,
`all_co_sk_spell_launch_missile`, `all_co_sk_spell_launch_nd`, `all_re_combat_stun`, `fist_ba_spawn`, `fist_co_sk_bite`,
`onehand_co_sk_shieldpush`, `onehand_co_sk_throw`, `onehand_co_sk_weapon_slash_3`, `onehand_co_sk_windslash`,
`twohand_co_sk_weapon_slash_3`, `twohand_co_sk_windslash`.

**Top missing effects (player skills): two**, one skill each, both typos of the data (the effect exists under another name):
`abillity_skill_table_m.illusion.tired_curse_form` (10134 Lassitude; the library has `_cast` and `_hit` only) and
`abillity_skill_table_m.wild.long_distance_shot_launch` (13281 Missile Rain; the library has `test.long_distance_shot_launch`). Monsters:
`ability_skill_table_k.p_skill_adament.p_skill_disarmament`.

**Unsupported features by player skills affected** (231 player skills; "tree" = the 110 shown tree skills; "share" = the part of the
enabled emitters of the effects that have the feature that carry it):

| skills | tree | effects | emitters | share | feature |
|---:|---:|---:|---:|---:|---|
| 157 | 96 | 242 | 312 | 15 % | light (`LightSource*`, `LightHDRDynamic`) |
| 98 | 57 | 120 | 150 | 13 % | **refraction** `*` (`.mtl` with `RefrBumpScale`; drawn as the plain diffuse map) |
| 78 | 49 | 100 | 167 | 20 % | **geometry** `*` (47 skills in projectile groups, 53 elsewhere; 9 effects are nothing but meshes) |
| 75 | 43 | 72 | 159 | 25 % | `InheritVelocity` |
| 75 | 49 | 78 | 140 | 19 % | `NoOffset` |
| 68 | 34 | 62 | 92 | 18 % | **tails** `*` (`TailLength`, `TailSteps`) |
| 62 | 34 | 62 | 135 | 19 % | **collisions** `*` (`PhysicsType`, `Collide*`, `Bounciness`, `ParentCollide`) |
| 61 | 35 | 58 | 113 | 21 % | `EmissiveHDRDynamic` |
| 58 | 26 | 48 | 81 | 29 % | `IndependentUVTimer` |
| 58 | 38 | 67 | 103 | 23 % | `SizeZ` |
| 57 | 37 | 65 | 116 | 18 % | **`Rotate*` orbiting** `*` |
| 37 | 23 | 47 | 72 | 21 % | **`AttachType` / `AttachForm`** `*` (emission shapes) |
| 31 | 19 | 33 | 50 | 11 % | **decals** `*` (`DecalMode`, ...) |
| 27 | 14 | 23 | 39 | 13 % | `AlphaTest` |
| 20 | 13 | 22 | 24 | 13 % | `ForceGeneration` |
| 12 | 2 | 10 | 14 | 8 % | emitter `Sound` |
| 10 | 1 | 5 | 12 | 20 % | `BindToOffset` |
| 8 | 4 | 15 | 38 | 16 % | `OceanParticle` |
| 5 / 4 / 4 / 3 / 2 | | | | | `SecondGenerationRemoveCamera`, `NoUseMaterialAlpha`, `Thickness`, `LocalRotate`, `SurfaceType` |

Ignored cost and visibility hints, not counted: `IgnoreAttractor` 39 skills, `VisibleUnderwater` 27, `NotAffectedByFog` 23,
`FillRateCost` 14, `DisableVisClipping` 3, `ViewDistanceAdjust` 2, `CastShadows` 1. By set, every ability tree has 11 to 22 skills that
look different and at most 2 plain ones.

**What the data says the client does not do yet** (counted over the player skills):

* **Stop markers.** 17 plot steps play two anims, `all_co_sk_stop` first and then the clip; 4 steps play a stop only (it means
  "end the clip that loops"). The two anims were a bug (see below); the stop-only steps still do nothing, so a cast loop that a
  plot ends with a stop keeps looping until its hold time runs out.
* **Looping clips are played once.** 16 clips of the fire / plot steps have `anims.loop` (every mage and healer cast and channel pose:
  `all_co_sk_spell_cast_fireball`, `_flash`, `_spear`, `_cure`, `all_co_sk_spell_channel_cure`, `_sorb`, `_tug`, `_pray`,
  `all_co_sk_pull_cast`, `bow_co_sk_high_cast`, `onehand_co_sk_weapon_blunt_3_cast`, ...) and the client plays plot clips once; 19
  player skills, 16 of the 110 shown tree skills. Channeling is part of it: every `channeling_anim_id` of the player skills is a stop
  marker, so the 12 channeling skills channel with their `fire_anim` (6: Invincibility, Protective Wings, Meditate, Summon Wraith,
  Telekinesis, Twilight), a plot step (3: Lasso, Crippling Mire, Absorb Lifeforce) or no clip (3: Play Dead, `/bow`, `/kneel`), and the
  pose plays once while the channel lasts 4 to 60 s.
* **Weapon class.** 25 player skills have a `twohand_*` clip and 10 a `dual_wield_*` clip (9 and 3 shown tree skills); the client plays the one hand
  clip. All of them exist on all models.
* **Controller and instrument anims** (5 and 5 skills: Charge, Overwhelm, Drop Back, the music skills) are not played; 9 shown tree
  skills have no clip anywhere in the data (passives and buffs: Stealth, Freerunner, Play Dead, ...).
* **Per level visuals** (`skill_visual_groups`: 27 rows for 9 tree skills) are not used.
* **Anim events.** 112 player skills time launch effects or a projectile by a clip; 7 of them (6 tree skills: Sunder Earth, Lasso,
  Freezing Arrow, Arc Lightning, Flamebolt, Skewer) use a looping cast clip that has no `launch` / `combat_sync` event, which is
  right: the effects start with the clip (release 0). Every other launch timed clip has a `launch` event on all 8 models.

**Build next**, in order of skills fixed per effort:

1. **Plot step clips in aa-client** (0.5 day): end the loop on a stop marker, loop `anims.loop` clips until the next step or
   `PlotEnded` / `PlotStopped`, pick the `twohand_` / `dual_` clip by the equipped weapon, play the controller and instrument
   anims. 16 shown tree skills' casting and channeling poses, 9 + 3 weapon variants.
2. **Geometry emitters in aa-fx** (2 to 3 days): mesh particles (size from `Size` / `SizeX` / `SizeY` / `SizeZ`, material from the
   `.mtl`, aa-cry's `MaterialHook` for the additive look; the arrow is done by hand in `combat.rs`). 78 skills, 49 tree skills, and the
   main shape of the slash, lightning and shock effects; 9 effects draw nothing without it.
3. **`Rotate*` orbiting in aa-fx** (0.5 to 1 day, simulation only): 57 skills, 37 tree skills.
4. **Refraction** (0.25 day to stop drawing them as plain glow sprites if the frames show blobs, then 1 to 2 days for a screen space
   distortion pass): 98 skills, 57 tree skills; the largest group after the light.
5. **Tails** (0.5 day as `Stretch` streaks, 1.5 days as ribbons): 68 skills, 34 tree skills.
6. **Lights** (1 day: a budgeted `PointLight` per effect): 157 skills, 96 tree skills, but only 15 % of the emitters.
7. **Camera shakes** (0.5 day, `fx_shake_cameras`: 22 skills, 46 items), **decals** on terrain (1 day, 31 skills: ground rings),
   **material flashes** (1 day, 24 skills), **per level visual groups** (0.5 day).
8. **Collisions** (1 day, terrain only: sparks that bounce, 62 skills) and **emission shapes** (1 to 2 days, 37 skills) last.
9. **Sounds and voices** (168 skills with a sound, 137 with a voice) need a sound system first (`Sounds/x2skills:...` names); a
   separate project.

## Not done

* Channeling skills (`channeling_anim_id`, `channeling_time`): the channel clip is not played. Looping clips of plot steps and fire
  clips (`anims.loop`) play once, stop markers do not end a cast loop, and the controller and instrument anims are unused; see
  "Coverage" for the counts.
* Two-handed / dual wield clip choice for plot steps is whatever the step names; the weapon in the hand is a single sword
  (`Grip::Blade` puts the main hand weapon in the right hand, an off-hand weapon in the left).
* Mounted, swimming and flying casters, doodad and ship cannon projectiles.
* Sounds and voices (`FxSound`, `FxVoice`), camera shakes and decals of the effect groups, `FxMaterial` flashes on the hit.
* Particle features aa-fx lacks (refraction materials, geometry particles, trails) show as their plain emitters: the
  `skill_refraction_*` distortions of the cast and hit effects are missing.
* A dead rat in a pond lies below the water surface (the corpse is not lifted).
* The player's own movement while a cast clip plays is not blocked: the server refuses casts while moving.

## Needed from other crates

* aa-fx: `Geometry` emitters (`EffectDef::geometry` only finds the model, the application has to draw it); refraction materials
  (`skill_refraction_*`, `RefrBumpScale`) are on 385 effect emitters in `x2_effects.xml` alone.
* aa-cry: `Shading` has no additive "particle" mode, the arrow mesh is restyled through a `MaterialHook`.
