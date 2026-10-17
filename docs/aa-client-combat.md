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
  37 / 38 (`Projectile` / `ProjectileAnim`: `value1` = `projectiles.id`). 3-hit slash 18132: steps 12130 / 12132 / 12133 play
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
* **Cost.** Skills are read from the database once and cached by id (`SkillBook`, plus the effect group, anim and plot step
  tables); a frame touches no database. Per frame: `follow_fx` / `fly_projectiles` iterate the live effect entities (a handful),
  `hold_weapons` the carried weapons (3 per character), `end_actions` the `Acting` units. Measured with `AA_FX_LOG=1` during a
  fight with arrows: aa-fx `sim_ms` about 0.03 ms, `upload_ms` 0.01, with the world's 7 ambient effects; the frame time with and
  without the combat code is the same within the noise (about 80 to 110 fps in the dev build on this box, 10 to 12 ms).

## What it was checked against

There are no real-client combat captures (`reference/` has none), so the check was the data itself and the server's behaviour on
lane `bevy` (account `bevytest`, the character with Shoot Arrow on slot 3, the 3-hit slash on 2, the melee attack on 4, Return on
slot 22 and Angry Rats / Solzreed Foxes around the spawn). Dev tools (all environment variables, off by default):

* `AA_COMBAT_LOG=1`: one line per combat event, clip, projectile and unit death.
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

## Not done

* **Upper body layering.** The data has it (`anims.move_ub`, the `_ub` / `_mub` clips: shoot while walking) but the animator has
  one clip at a time; a whole-body clip plays and locomotion resumes afterwards. Skills that need a standing caster (the server
  refuses casts while moving) look right; a skill used while running shows the full-body clip.
* Channeling skills (`channeling_anim_id`, `channeling_time`): the channel clip is not played.
* Two-handed / dual wield clip choice for plot steps is whatever the step names; the weapon in the hand is a single sword
  (`Grip::Blade` puts the main hand weapon in the right hand, an off-hand weapon in the left).
* Mounted, swimming and flying casters, doodad and ship cannon projectiles.
* Sounds and voices (`FxSound`, `FxVoice`), camera shakes and decals of the effect groups, `FxMaterial` flashes on the hit.
* Particle features aa-fx lacks (refraction materials, geometry particles, trails) show as their plain emitters: the
  `skill_refraction_*` distortions of the cast and hit effects are missing.
* A dead rat in a pond lies below the water surface (the corpse is not lifted).
* The player's own movement while a cast clip plays is not blocked: the server refuses casts while moving.

## Needed from other crates

* aa-anim: partial-body playback (play a clip on the bones above the pelvis only, layered over locomotion) would finish the
  upper-body case; `Animator::play_once_then` with a mask would be enough.
* aa-fx: `Geometry` emitters (`EffectDef::geometry` only finds the model, the application has to draw it); refraction materials
  (`skill_refraction_*`, `RefrBumpScale`) are on 385 effect emitters in `x2_effects.xml` alone.
* aa-cry: `Shading` has no additive "particle" mode, the arrow mesh is restyled through a `MaterialHook`.
