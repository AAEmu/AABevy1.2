# aa-client combat visuals

What a skill looks like on the units that use it and the ones it hits: cast and attack clips, particle effects at the right
bones, flying arrows, hit reactions, death. Code: `crates/aa-client/src/ingame/combat.rs` (events to clips and effects),
`combat/skills.rs` (the database side), `combat/clips.rs` (anim events of a clip), `weapons.rs` (weapon in the hand), `sounds.rs`
(skill sounds, voices, footsteps, blows).
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
  `FxParticle` items are particle effects, `FxSound` and `FxVoice` items are played as sounds (see Sounds), `FxShakeCamera` items shake the
  player's camera; `FxMaterial` flashes are left out.

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
| `SkillStarted` with `cast_ms > 0` | the skill's `start_anim` (a song's: the clip of the instrument held) loops on the caster (`Acting`), the local player turns to the target, FORM effects play on the caster (held until the skill fires or stops) |
| `SkillStarted` refused with `OUTOF_ANGLE` (0x10) | the local player turns to the target, sends a standing `Move` with the new facing and asks again once (the real client turns on its own) |
| `SkillFired` | the fire clip: a song's for the instrument held, else the server's id (replaced by the skill's own `twohand_*` / `dual_wield_*` clip when the character holds such a weapon and the server sent the skill's one hand clip), else the skill's (a dash or a leap: its controller's clip); a clip with `anims.loop` loops until the skill ends; the player turns to the target; THROW effects at once, LAUNCH effects and the projectile at the clip's release moment (anim events, else `effect_delay`); an instant skill also plays the START / FORM effects that were skipped (effects that die at LAUNCH are not shown); camera shakes start with their effect moment |
| `PlotEvent` | the step's clip / effect group / projectile / camera shakes; a step that plays `all_co_sk_stop` ends the looping clip before it (alone it only ends it), a looping clip loops until the next step or the end |
| `UnitDamaged` (kind 0 / 1) | HIT effects (and HIT camera shakes: the player's own blows kick its camera) of the cast's group on the target at the moment the blow lands: `fired + delay_ms` for a flying shot, `fired + combat_sync` for a melee swing, at once for a plot step; then the target's hit reaction (0.5 s flinch, never interrupts an action or a run; critical hits use the critical clip); a miss only shows the clip |
| `UnitDeath` | after the last scheduled hit on the unit: `dead_01` once, then it stays on the last frame (`Dead`: locomotion never takes the unit back) until the server removes it |
| `Resurrected` | `Dead` / `Acting` removed, locomotion restarts |
| the local player moves or jumps during a cast time (or turns, for `stop_casting_by_turn`), or moves during a `channeling_cancelable` channel (`cancel_on_move`) | `CSStopCasting` goes out and the clip and held effects end at once; the server's `SkillEnded` / `CastingStopped` / `PlotStopped` that follows ends nothing more |
| `SkillEnded` / `CastingStopped` / `PlotEnded` / `PlotStopped` / `SkillStopped` | the effects held until END are released; the looping clip of that cast (cast pose, channel pose) ends and the unit returns to locomotion (an interrupted cast too); a one-shot clip runs its course |
| a unit that spawns dead (`Unit::dead`) | lies on its last death frame at once |

* **Locomotion** (`ingame/anim.rs`): `drive_locomotion` skips units that are `Acting` or `Dead` and restarts its clip when the
  action ends (`Locomotion::invalidate`). Rats and foxes use the same code: their `fist_co_attack_r` bite plays on `SkillFired`
  of skill 2 and the hit lands on the player at `combat_sync`.
* **Loops and ends** (`play_clip`, `end_loop`): a clip is looped when it is the cast pose or its `anims.loop` is `t` (`SkillBook::clip_loops`:
  every mage and healer cast / channel pose, 19 player skills). The clip belongs to the cast (timeline id) that started it
  (`Combat::owner`): the next clip of the cast replaces it, a stop step, `SkillEnded`, `PlotEnded`, `PlotStopped`, `CastingStopped` or
  `SkillStopped` of that cast ends it (`Acting` removed, `Locomotion::invalidate`), and a cast that is not the unit's current one
  leaves the clip of the newer cast alone. The safety hold, for an end that never comes, is `channeling_ms + 2 s` (20 s for a plot
  step). `channeling_anim_id` is a stop marker in every player skill: the channel pose is the fire clip or the plot step's clip.
* **Cancelling a cast** (`cancel_on_move`, `PlayerInput` of `player.rs`, `Command::StopCasting`): the server never cancels a cast
  because the player walks (it only keeps `skill_state.moving` for `source_cannot_use_while_walk`, a refusal at the start), so the
  client sends `CSStopCasting`. The rule is the live game's behaviour ("don't move while recalling"), **not a DB flag**:
  * moving or jumping during the cast time of any skill (`casting_time > 0`; for a plot, a step whose `PlotEvent.casting_ms` is
    set) cancels it: Return, Revive, the combat spells, placing a pot;
  * a channel is only cancelled by moving when the skill says so, `channeling_cancelable` (Invincibility, Twilight, Play Dead,
    Absorb Lifeforce, Crippling Mire, Telekinesis' channel, ...; their tooltips say "canceled if you move");
  * turning cancels only with `stop_casting_by_turn` (two quest skills), and only the cast time; turning in place with the mouse
    (camera) does not turn the character, Q / E and the right mouse button do. Whether the real client cancels on turning in
    place is not known: the rule is flag only.

  `casting_cancelable` is not read: it is set on 60 of the 6,877 skills with a cast time (placing a pot, the recall point, siege
  and quest casts; no tree skill) and most likely means the player may cancel the cast by hand. Moving is a held movement key
  that went somewhere, or the jump; turning is any change of the facing (Q / E, the right mouse button, the swing to the camera
  heading). Fields: a plain skill sends its `tl`, a plot its timeline as `plot_tl` (the real client sends 0 for the other one),
  once per cast. The local clip stops at once (`Acting` removed, the looping clip ends, the effects held until END are
  released); the server's `CastingStopped` / `SkillEnded` (32 ms later on the lane) ends nothing more. A cast the player starts
  while a movement key is held is cancelled in the next frame. `AA_COMBAT_NO_CANCEL=1` turns the cancel off (to look at casts
  while the player runs: the upper body clips of `Acting::layered`).

* **Weapon class** (`weapons.rs` `Wield`, inserted when the character's weapons are built): `two_handed` (`holdables.slot_type_id`
  16), `dual` (a weapon in the off hand that is not a shield), `instrument` (string 0 / percussion 1 / tube 2). The skill's fire clip
  is `SkillVisual::fire_clip(two_handed, dual)`; plot weapon variants are chosen by the server per event id (nothing to do). The
  weapon comes out for the clip's prefix (`weapon_for_clip`: `bow_` the bow, `onehand_` / `twohand_` the main hand weapon (a two
  handed one hangs on `item_back_c` and goes to the right hand), `twoswords_` both swords, `music_` the instrument, which is hidden
  while it is not played), and goes back 4 s after the last action.
* **Controller and instrument clips**: a skill that has no clip of its own plays its `skill_controllers.start_anim_id` when its first
  step or fire comes (Charge, Overwhelm, Drop Back, Behind Enemy Lines, ...); a song plays `*_instrument_start_anim_id` (the cast loop) and
  `*_instrument_fire_anim_id` of the instrument held, else the skill's own clips.
* **Visual groups by level** (`skill_visual_groups`, 27 rows, 9 tree skills): `SkillVisual::look(level)` picks the last group at or
  below the caster's level (the player's from `aa_net::state::Skills`, others' from `Unit.skills`, else 1) for the hit effects, the shakes and
  the projectile; a skill without rows keeps its own group.
* **Camera shakes** (`ingame/camera.rs` `CameraShake`, 22 player skills, 46 `FxShakeCamera` items; `fx_shake_cameras`): an item
  starts at its effect moment on the caster (location 1) or the target (2); only the local player's camera shakes. The wave is the
  data's: `ang_*` degrees and `shift_*` metres per axis (cry axes), alternating every `frequency` s (at least 0.03) with
  `randomness`, fading out linearly over `duration`. Because this camera sits 4 m behind the character the amplitudes are halved
  (`SHAKE_SCALE`) and capped at 0.2 m / 2 degrees, at most 6 at once; it is added in `follow_player` after the orbit pose.
* **Effects** are aa-fx `Effect` entities (`prime: false`). An effect on a bone is a child of that bone (of the unit for the origin),
  its local pose giving it the world pose it would have had (the unit's orientation and scale), so aa-fx sees the motion of the bone
  (tails, `InheritVelocity`) and nothing is copied per frame; held ones loop until their END moment, short ones despawn themselves.
  The one exception is a hit effect with axis 4: it is turned towards the attacker and keeps that orientation, so it stays a root
  entity and only follows the bone's position (`follow_fx`). An effect that never finishes dies after 12 s. `feed_fx` hands aa-fx the
  sun (the direction of aa-world's `SunLight`) and the ground (`TerrainQuery::height_at`, so sparks bounce on the real terrain) every
  frame (`FxSettings.sun` / `ground`; the coverage example counts collisions as supported since `GROUND_WIRED`).
  The world camera and the prewarm camera carry `ScreenSpaceTransmission` from the start (`camera::transmission()`): it is what aa-fx
  adds to a camera when the first refraction effect appears, and it is part of the view layout every pipeline is keyed on, so a camera
  that got it in the middle of the game would compile every material again. `AA_PIPELINE_DUMP` of the prewarm against a normal run
  shows no difference: the loading screen still compiles 21 to 22 pipelines in both (the build before this change: 22).
* **Arrows**: the projectile is an anchor entity flying caster launch bone -> target `Bip01 Hit` over the flight time (homing: the
  end follows the bone). The effect group of the projectile (115 for Shoot Arrow) is an `aa_fx::Effect` child of the anchor
  (`prime: false`, forward = the flight direction = Bevy -Z): aa-fx draws the arrow body from the effect's own `Geometry` emitter, the
  client has no special case for it any more (the `.cgf` hand drawing and `EffectDef::geometry` are gone). When the arrow arrives the
  emitters stop (`looping = false`) and the effect finishes by itself. aa-fx emits the first particle of a continuous emitter one
  pulse period after the start and waits for its textures and models: in a short flight (8 m, 0.6 s) the arrow is therefore faint or
  missing on the first frames. `AA_COMBAT_CHASE=1` puts the camera behind the arrow to look at it.
* **Weapons** (`weapons.rs`): a character that fights takes the weapon out: the bow comes into the left hand (`item_hand_l`,
  turned 90 degrees about z so that the limbs stand upright) for the bow skills, a sword into the right hand for any other
  weapon clip (the two handed sword from the back, both swords for the dual wield clips, the lute / drum / pipe for the music
  clips), and all go back to their sheath bones 4 s after the last action (`Drawn { out }`). The shield is on the arm only while
  the sword is out (the real client shows none on a character at peace: it comes onto the arm with the sword, `Grip::Shield` in
  `weapons.rs`; with the bow or an instrument out it is away too). The `drawn_scale` of `item_weapons` applies in the hand.
* **Moving casters** (`Acting::layered`): a unit whose speed is above 0.3 m/s plays a cast, fire or plot clip on its upper body
  (aa-anim's `Animator::play_layer` with `BoneMask::upper_body`, `docs/aa-anim.md`) while `drive_locomotion` keeps running the
  8-way run / walk clips underneath (the layer ignores the run clip's playback speed). The clip is the `move_ub` variant of the
  skill's `anims` row (`SkillBook::upper_clip`; `bow_co_sk_launch` -> `bow_co_sk_launch_ub` -> the model's `bow_co_sk_launch_mub`,
  else `_ub` as is), else the clip itself through the mask. A standing unit plays the whole-body clip as before; a unit that stops
  mid-action keeps its upper body clip over the idle, one that starts moving mid-action keeps its whole-body clip. A one-shot
  layer ends by itself, a cast loop is released (`release_layers`, 0.12 s fade) when `Acting` ends or the fire clip replaces it,
  and a layered action never invalidates the locomotion state (no restart of the run cycle); an action that had the whole body
  restarts locomotion when `Acting` is removed by an end event (`release_layers`). Hit reactions stay whole-body and
  never interrupt a run. `AA_COMBAT_FULLBODY=1` turns the layer off (to compare shots).
* **Root motion** (`anim::bind_action_clip`): an action clip moves the unit's body (the root bone) the way the clip says, as the real
  client does: the kick steps about a metre towards its target and comes back while the unit's own position, the camera and the
  server's idea of the position stay where they are (`reference/ingame/combat_kick`). A clip whose root travels more than 2.5 m
  from where it starts (a dash, a charge: the server moves the unit then) stays locked, and so do the locomotion clips.
* **Cost.** Skills are read from the database once and cached by id (`SkillBook`, plus the effect group, anim and plot step
  tables); a frame touches no database. Per frame: `follow_fx` / `fly_projectiles` iterate the live effect entities (a handful),
  `hold_weapons` the carried weapons (3 per character), `end_actions` the `Acting` units. Measured with `AA_FX_LOG=1` during a
  fight with arrows: aa-fx `sim_ms` about 0.03 ms, `upload_ms` 0.01, with the world's 7 ambient effects; the frame time with and
  without the combat code is the same within the noise (about 80 to 110 fps in the dev build on this box, 10 to 12 ms). With the
  round 2 additions (clip loop lookups, `Wield`, shakes) a fight with arrows, three slashes and melee on the lane showed
  `frame_ms` 11.3 to 14.8: a loop lookup is one cached map hit per clip, a shake a few multiplications per frame while it runs.
  Round 3 (release build, a fight at the pond with an arrow, the 3-hit slash and seven melee swings, same script for both binaries):
  `frame_ms` 10.4 to 12.0 (83 to 97 fps) during the fight and 9.4 to 10.6 before it; the build from before round 3 measured 10.7 to
  11.0 in the fight. aa-fx `sim_ms` 0.04 to 0.08 with 200 particles (single spikes of 0.3 to 0.9 ms), `upload_ms` about 0.01. The
  difference is inside the noise of the shared box (a second client was running); the sounds cost nothing when muted.

Upper body layer frames (`shots/aa-anim/`, not committed; `--walk 9:12` runs the player from second 9 while slot 3 fires at 9.5,
`--cam-dist 5 --cam-pitch -12`, `--cam-yaw 120` for the arrow pair and 40 for the casts): `walk_arrow_before.png` (`AA_COMBAT_FULLBODY=1`: the legs stand planted in the
shooting pose while the player slides forward) and `walk_arrow_after.png` (the arms draw the bow, the legs keep their run stride),
`cast_standing.png` (Return from standing: whole-body cast, unchanged) and `cast_walking.png` (Return while running: the cast on
the upper body, the legs running).

## Sounds (`ingame/sounds.rs`)

`aa-audio` plays event paths (`sounds/x2skills:wild:fire_arrow_form`); `sounds.rs` decides which, when and where. Every sound is an
`aa_audio::Sound` entity under the bone it comes from (3D at the bone, follows the unit, ends with it). Nothing is read or decoded
in a muted run (`--shot`, `--exit-after`, `--mute`, `AA_MUTE=1`); `AA_AUDIO_LOG=1` makes the databases load anyway and logs every
sound (`sound: voice Sounds/x2voice_combat:nuian_male:attack on 563v0`) and, every 2 s, the counts.

| what | data | when and where |
|---|---|---|
| skill sounds | `fx_items` of type `FxSound` (`asset_name` = the event path), `fx_particles.sound_id` of a particle item, and the `Sound=""` attribute of the emitters in `libs/particles/<lib>.xml` (aa-fx ignores it: `SoundBook::particle_sounds` scans the library text) | at the item's moment (START, FORM, THROW, LAUNCH, HIT, END) on the item's bone of the caster or the target; a sound of a held effect hangs under the effect and stops with it; the projectile's sounds fly with the arrow |
| voices | `FxVoice` items (`fx_voices.sound_pack_item_name`, moment id 7 = `voice_launch`) -> `sound_pack_items` of the caster's pack (`models.sound_pack_id`: `nuian_male`, `elf_female`, ...) -> `Sounds/x2voice_combat:nuian_male:attack` | at the clip's `voice_launch` event (else the release moment) at the caster's head; the pack comes from the model, so race and gender follow |
| blows | `combat_sounds` (weapon material x target material), `holdables.sound_material_id` of the main hand weapon (fists without), `models.sound_material_id` of the target | when a melee hit lands (`UnitDamaged` that did not fly), at the target's `Bip01 Hit`: sword on flesh, knuckles on a player |
| animation sounds | the `$SoundEventDatabase` of the model's `.cal`: `sound` (event path), `sound_pack` (an item of the unit's pack) and `footstep` events of every clip | by the clip's playhead (`anim_sounds`), a clip on the upper body of a moving unit by the clock (`LayerSounds`); at the event's bone, else the unit |
| footsteps | `footstep` event -> row of `libs/materialeffects/materialeffects.xml` (`footstep_nuian_walk`) -> column by the ground surface (`mat_dirt`, `mat_water` below the sea level, `mat_default`) -> `lib:effect` -> `fxlibs/<lib>.xml` -> `sounds/x2physics:mid_fs:grass_run` | the run and walk clips have no events of their own: the steps are phases of the cycle (0.25 / 0.75) in the `.lmg` of their group (`fist_mo_normal_run.lmg`) |

* **The ground surface** is read client side: aa-world's `CellHeights::surface` holds one of 16 surface type indices per terrain
  sample, `leveldata.xml` of the world says which layer material each index is and its `.mtl` has `SurfaceType="mat_dirt"`
  (`SoundBook::surface`, read on a thread the first time a cell is stepped on). Not covered: the object material under the foot
  (a floor, a bridge) and a water body above the terrain (a pond is `mat_dirt` / `mat_grass` of its bed; only the sea is `mat_water`).
* **Cap and culling**: at most 4 sounds per unit at a time (`UNIT_CAP`; a new one replaces the oldest of the same or a lower class,
  classes are Foley < Voice < Skill, else it is dropped), a sound is not started when its source is further from the camera than
  32 m (foley), 55 m (voices) or 75 m (skills), and units further than 45 m from the camera do not play animation sounds at all.
* **Not done**: the sounds of songs (`fx_particles.sound_pack_id`, `$holdable_N`: the bard's instruments), the object material under
  the foot, the hit sounds of skills that hit through a projectile (the arrow's own hit sound is the item of the group).

Runs on lane `bevy` (muted, `AA_AUDIO_LOG=1`): one pond fight (Shoot Arrow, the 3-hit slash, seven melee swings, the rat dies) started
**21 foley, 6 voice and 12 skill sounds, none too far, 3 foley left out for the cap**: the swing sounds of the clips
(`swing:pc_med`, `swing:large_1`), the nuian_male voices (`attack`, `skill`), `bow:bow_proj`, `slash3_launch`, `fight:hitup`, the blows
(`sword:sword_flesh` on the rat, `knuckle_flesh` on the player) and the rats' own (`rat_attack`, `rat_damage`, `rat_relax_idle`); a
4 s run on grass, dirt (the pond edge) and the beach played 21 footsteps (`mid_fs:grass_run`, `dirt_run`, `water_run`). One quiet
`--sound` run (`AA_VOLUME=0.05`) played them through the device without errors. In the data: 168 player skills have a sound (27 skills
with `FxSound` items, 151 with a sound bound to a particle, 12 with an emitter `Sound`) and 138 have a voice.

## What it was checked against

The first two rounds had no real-client combat capture (`reference/ingame/combat_*` came later, see Real captures below), so the check
was the data itself and the server's behaviour on lane `bevy` (account `bevytest`, the character with Shoot Arrow on slot 3, the 3-hit slash on 2, the melee attack on 4, Return on
slot 22 and Angry Rats / Solzreed Foxes around the spawn). Dev tools (all environment variables, off by default):

* `AA_COMBAT_LOG=1`: one line per combat event, clip, projectile and unit death.
* `AA_COMBAT_FULLBODY=1`: moving units play action clips on the whole body (the behaviour before the upper body layer).
* `AA_COMBAT_SLOWMO=0.3`: the game clock slows to that speed when the local player first fires (catches a flight on screen).
* `AA_COMBAT_SHOT=<png>@<start|fire|fire2|plot|hit|death>+<s>`: screenshot that many game seconds after the first such event and
  exit; combine with `--hud-key tab T --hud-key slot3 T` (see `docs/aa-client-hud.md`).
* `AA_DEV_MAINHAND=<item>` / `AA_DEV_OFFHAND=<item>` / `AA_DEV_MUSICAL=<item>` (0 for none): the item the character holds in that
  slot instead of its own, to see the two handed (5660 is the first two handed sword), dual wield (5569 in both) and instrument (6177
  the first lute) clips with the one handed test character.
* `AA_COMBAT_SHOT=<png>@<event>+<s>[,<s>...]`: with several times a series `<png>_01.png`, `_02.png`, ... (game seconds after the
  event; `fire2` is the second time the player fires, `plot` a step of its plot skill, `death` a unit's death).
* `AA_COMBAT_CHASE=1`: the camera follows the projectile in flight. `AA_COMBAT_CAM_REL=-90`: the camera stays at that heading from
  the way the player faces (the real client's `right_camera` is -90 until the character turns to its target). `--cam-fov DEG`: the
  lens (the game's is 42.75; the real captures used `cl_fov 75` = 71.25). `AA_FX_SCALE=2`: scales every skill effect.
* `AA_AUDIO_LOG=1`: logs every sound the combat code starts and the per class counts (see Sounds).
* `AA_COMBAT_NO_CANCEL=1`: moving, jumping and turning cancel no cast of the player (no `CSStopCasting`), to look at casts while it runs.
* `AA_COMBAT_FAKE="T:op:args;..."`: server events for the local player, `T` seconds after the first frame (the test character has
  no channeled or two handed skills, and the lane does not interrupt a cast when the player walks): `start:TL:SKILL:CAST_MS`,
  `fire:TL:SKILL:DELAY_MS:CHANNEL_MS[:FIRE_ANIM]`, `plot:TL:EVENT:SKILL[:CASTING_MS]`, `hit:TL:SKILL:DAMAGE`, `end:TL`, `stop:TL`,
  `plotend:TL`, `plotstop:TL`. Start it at 14 s or later, before that the world is still streaming in.

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
| `r2_invincibility.png` | 7 s into a 12 s channel of Invincibility (`AA_COMBAT_FAKE="3:fire:102:10372:0:12000;15:end:102"`): the 1.33 s clip `all_co_sk_buff_cast_defense` still loops, the charge-up glow on the hands |
| `r2_return_stopped.png` | Return cast of 10 s, `CastingStopped` after 3 s (`start:130:10594:10000;stop:130`), shot 1 s later: idle again, bow and sword back on the body |
| `r2_stop_before.png` / `r2_stop_after.png` | Absorb Lifeforce (plot, `plot:200:2499:11441;plot:200:2501:11441` 9 s apart): 8.5 s into step 2499 the 7 s clip `all_co_sk_spell_channel_sorb` has looped and the pose still holds (`_before`); step 2501 only plays `all_co_sk_stop` and ends it (`_after` is an earlier run, the stop 6 s in, 2 s later: idle) |
| `r2_wallop_one.png` / `_two.png` / `_dual.png` | Wallop (12029) 0.3 s into the swing with a sword and shield (`onehand_co_sk_fastattack_1`), with the two handed sword (`AA_DEV_MAINHAND=5660`: `twohand_co_sk_fastattack_1`, the greatsword in the hands) and with a sword in each hand (`twoswords_co_sk_fastattack_1`, both out) |
| `r2_song.png` | a song (11377) cast with the lute (`AA_DEV_MUSICAL=6177`): `music_co_sk_lute_cast`, the lute in the hands |

Matched in rounds 1 and 2: the clip names and timing (launch 0.358 s into `bow_co_sk_launch`, bite sync 0.487 s), the effect bones (bow hand for the
arrow, `Bip01 Hit` for the hit spark), flight time and hit moment from `SCSkillFired.delay_ms` (the hit reaction starts when the arrow
arrives, 0.3 to 0.7 s after the shot), and the rat / fox attack clips on the player. Not matched against a real capture: the hit sparks (`st1000c`, `fight.hitup`) and the Return charge-up glow do spawn (aa-fx counts
the extra effects) but are short and small in the frames; effect
brightness, the arrow's glow (the mesh is a flat additive model; the particle trail of `st1000b` is aa-fx's), and the exact hand
pose of the bow.

## Real captures (`reference/ingame/combat_*`)

Real 1.2 client, Refone (Nuian male, sword and shield, bow on the back), the spawn pond, camera 4 m side-on with `cl_fov 75`. Mine:
bevytest at the same spot, `--cam-fov 71.25 --cam-yaw -90 --cam-dist 4 --cam-pitch 0`, `AA_COMBAT_CAM_REL`, and
`AA_COMBAT_SHOT=...@fire2+<t_req + 0.05 of each real frame>`; the rats here are not at the same place, so the viewpoints differ by up
to a quarter turn. Frames and sheets are in `shots/combat/cmp/` (not committed).

| capture | matches | does not match |
|---|---|---|
| melee (skill 2, `combat_melee`) | the clips (`onehand_co_attack_r_slash_2`, then `_slash`), about 1.5 s per swing as in the real frames, the sword in the right hand, the cut ending with the blade extended and the low guard between swings, the shield forward on the arm | the real first swing starts about 0.5 s after the key (the shield comes up and the stance changes first, the damage number shows at 0.8 to 0.9 s): mine starts the clip when the server's `SkillFired` arrives; the rat is not visible in the pond (the water of aa-world is too opaque for a rat on the bed) |
| Triple Slash (18132) | the three clips of the plot (`streakattack_1` / `_2` / `_3`) from the server's steps, the body pose of the third slam (sword down to the right, body bent) at the same time as the real frame, the jump of stage 3, a red streak on the hit | the jump of stage 3 comes about 0.4 s before the real one; the blue-white weapon trail is not visible in my frames (nor clearly in the real ones at the 0.3 s spacing) |
| Kick (14495) | the clip `all_co_sk_lowkick` of plot step 2517 (a standing target; `all_co_sk_trample` is the fallen target's), the body now steps towards the target and back (root motion, about 0.7 m against about 1 m) | the real character already has the sword out and the shield on the arm from an earlier fight (mine is at peace: an `all_` clip draws nothing); the kick is not on bevytest's bar, the frames are from `AA_COMBAT_FAKE="16:plot:300:2517:14495"` |
| Return (10594) | the cast pose (hands out at chest height, palms together), the 10 s cast, the effect group (`return_skill_form` on the right hand: 110 particles alive); with the shield no longer shown at peace the pose is the real one | the glowing sphere: at the effect's own scale 1 mine is a faint haze hidden by the hands, the real one is a bright spiky white-purple orb about 0.39 m across; with `AA_FX_SCALE=2` mine has the same size, colour, halo and place (but less bright). The emitter says `Size 0.2`: aa-fx draws its sprites at half the size the real client does here (not changed: aa-fx). In the real the arms hang for the first 4 s and the pose comes in slowly; mine plays the pose at once |
| death, death_2 | the death clip once, then the last frame; the name plate | the real rat is thrown back, lies on its side on the surface and sinks over seconds; mine is invisible (below the water, the corpse is not lifted) and is not thrown back |

Changes this made: the shield is shown only while the sword is out, action clips move the body (root motion), the hit effects and
sounds of plot steps were checked against the real frames.

## Coverage

How many skills would look right, measured from the data alone (no server, no window):

```
cargo run -p aa-client --example skill_coverage [-- --out data/extract/aa-client-combat/skill_coverage.tsv]
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
instrument and skill controller anims. "Present" means: a `.cal` entry, the `.caf`
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
| **effects all found, nothing ignored by aa-fx** | 11 | 11 | 0 | 29 |
| effects all found, minor loss only (light / sound / detail) | 128 | 77 | 5 | 170 |
| effects all found, at least one looks different (`*` features) | 32 | 20 | 0 | 19 |
| **effects missing** (at least one) | 2 | 2 | 0 | 1 |
| at least one effect invisible (a mesh / trail / orbit and no sprite) | 0 | 0 | 0 | 0 |
| at least one effect with nothing to draw by design (off / sound / light only) | 24 | 14 | 0 | 22 |
| uses a sound (any kind, all played by `sounds.rs`) | 168 | 109 | 4 | 222 |
| ... `FxSound` item / sound bound to a particle item / emitter `Sound` | 27 / 151 / 12 | 8 / 109 / 2 | 0 / 1 / 3 | 18 / 207 / 20 |
| voice (`FxVoice`) | 138 | 98 | 2 | 54 |
| camera shake | 22 | 11 | 0 | 18 |
| material flash (`FxMaterial`, not played) | 24 | 9 | 2 | 34 |
| **channeling** (channeling time or clip) | 12 | 10 | 2 | 10 |
| **plot skills** | 40 | 31 | 1 | 19 |

The effects rows changed with aa-fx: the counts of round 1 (5 plain, 10 minor, 156 different, 10 invisible of 173 skills) were made when
aa-fx had no geometry, tails, refraction, orbiting or collisions; they are now (aa-fx with those, `GROUND_WIRED = true`, the
projectile group wired as an effect) 11 plain, 128 minor, 32 different, 0 invisible. The sound rows count what `sounds.rs` plays
(the example needs no change: it counts the items of the groups).

Distinct things of the player skills: **202 clips needed, none missing on any model** (the elf and hariharan `.cal` take the player clips of
`global_player.cal` from the Nuian folders, and those files exist); **356 effects, 354 found** (88 plain, 232 with a minor loss, 34 that
look different, none that draw nothing at all; 17 have nothing to draw by design: every emitter is switched off, or it only carries a
sound or a light), 2 missing. Of the 2,364 switched on emitters of the 354 effects, 55 (2 %) carry a feature that changes the look;
the other 98 % draw as authored. Items that are not particles: 50 `FxSound`, 641 sounds bound to particles, 232 voices, 46 camera
shakes, 55 material flashes.

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
| 61 | 35 | 58 | 113 | 21 % | `EmissiveHDRDynamic` |
| 31 | 19 | 33 | 50 | 11 % | **decals** `*` (`DecalMode`, ...) |
| 20 | 13 | 22 | 24 | 13 % | `ForceGeneration` |
| 12 | 2 | 10 | 14 | 8 % | emitter `Sound` (played by `sounds.rs`) |
| 10 | 1 | 5 | 12 | 20 % | `BindToOffset` |
| 8 | 4 | 15 | 38 | 16 % | `OceanParticle` |
| 5 / 4 / 3 / 2 / 2 | | | | | `SecondGenerationRemoveCamera`, `Thickness`, `LocalRotate`, **geometry** `*` (`GeometryInPieces`: aa-fx draws it whole; 1 skill in a projectile group, 1 elsewhere), `SurfaceType` |

Ignored cost and visibility hints, not counted: `IgnoreAttractor` 39 skills, `VisibleUnderwater` 27, `NotAffectedByFog` 23,
`FillRateCost` 14, `CollideStaticObjects` 5, `DisableVisClipping` 3, `CollideDynamicObjects` 2, `MaxCollisionEvents` 2,
`ViewDistanceAdjust` 2, `CastShadows` 1. By set, the 32 skills that look different are spread over the trees (tree 7: 11, tree 9: 5, tree 5: 4, tree 10: 3, the others 1 to 2);
the 11 plain ones are in trees 2, 3, 5, 6, 7, 8 (4) and 10.

**What the client does with them** (counted over the player skills; the `client handling` block of the example is computed by the code
the client runs; "shown" = the shown tree skills):

| item | skills | shown | what the client does |
|---|---:|---:|---|
| a plot step that only plays `all_co_sk_stop` (4 steps: Lasso, Crippling Mire, Absorb Lifeforce, Searing Rain) | 4 | 4 | ends the cast loop of that timeline instead of letting it run to the hold time |
| 17 steps (not skills) play a stop and then the clip | - | - | the clip (and the stop ends the loop before it) |
| a fire / weapon variant / plot step clip with `anims.loop` (`all_co_sk_spell_cast_fireball`, `_flash`, `_spear`, `_cure`, `all_co_sk_spell_channel_cure`, `_sorb`, `_tug`, `_pray`, `all_co_sk_buff_cast_defense`, `_mental`, `bow_co_sk_high_cast`, ...) | 19 | 16 | loops until the next step or the end event |
| ... of them the channeling skills with a pose (9 of the 12: Invincibility, Protective Wings, Meditate, Summon Wraith, Telekinesis, Twilight, Lasso, Crippling Mire, Absorb Lifeforce; the other 3, Play Dead, `/bow` and `/kneel`, have no clip in the skill data) | 9 | 8 | the pose for the channeling time |
| `twohand_*` (23) / `dual_wield_*` (10) fire clips | 23 | 8 | by the weapon held (and the weapon model in the hand); the plot skills' variants are the server's choice per event |
| controller clip only (Charge, Overwhelm, Drop Back, Behind Enemy Lines, ...) | 5 | 3 | the controller's `start_anim` |
| instrument clips (the music skills) | 5 | 5 | the start and fire clip of the instrument held |
| per level visual groups (`skill_visual_groups`: 27 rows) | 9 | 8 | the group of the caster's level |
| camera shake (46 items; 3 skills have it in the projectile's group) | 22 | 11 | the player's camera, see above |

9 shown tree skills have no clip anywhere in the data (passives and buffs: Stealth, Freerunner, Play Dead, ...).

* **Anim events.** 112 player skills time launch effects or a projectile by a clip; 7 of them (6 tree skills: Sunder Earth, Lasso,
  Freezing Arrow, Arc Lightning, Flamebolt, Skewer) use a looping cast clip that has no `launch` / `combat_sync` event, which is
  right: the effects start with the clip (release 0). Every other launch timed clip has a `launch` event on all 8 models.

**What is left**: what changes the look is decals (31 skills: ground rings) and `GeometryInPieces` (2), the rest is lights (a budget
`PointLight` per effect), `EmissiveHDRDynamic` and `ForceGeneration`. `FxMaterial` flashes (24 skills) are not played.

## Not done

* Channel poses of the skills that have none in the skill data (Play Dead, `/bow`, `/kneel`: 3 of the 12 channeling skills).
* Cancelling by other player actions: pressing another skill, jumping off a mount, entering the water; only moving, jumping and
  turning cancel a cast here. The rule comes from the live game's behaviour and the skills' `channeling_cancelable` /
  `stop_casting_by_turn`, with no 1.2 client capture or disassembly to check it against.
* The two handed sword is tied to the right hand bone (the clip's pose holds it with both hands only because the grip is
  close); a dual wielder's second sword sits on `item_hand_l`. The instruments are placed by their `attachment_offset` of the
  right hand only (no frame of a real capture to compare).
* Camera shakes only reach the local player's camera (its own blows, or a shake that hits it) and at half strength; a shake that
  starts at HIT plays when the damage lands (a miss does not shake).
* Mounted, swimming and flying casters, doodad and ship cannon projectiles.
* Decals of the effect groups, `FxMaterial` flashes on the hit; the sounds of songs (bard instruments) and the object material under the
  foot for footsteps (see Sounds).
* The weapon draw and the combat stance of the real client: the first swing from peace starts about 0.5 s after the key there, with
  the shield coming up and a stance change first, and between swings the real unit idles in `<weapon>_ba_combat_idle` (and runs
  `<weapon>_mo_combat_run_*` while it fights); mine plays the swing at once and idles in the relaxed pose.
* The real rat is thrown back when it dies and lies on the surface before it sinks; mine is submerged and not thrown back.
* A dead rat in a pond lies below the water surface (the corpse is not lifted).
* The player's own movement while a cast clip plays is not blocked (the cast is cancelled instead); the upper body layer is for
  one-shot clips of the player while it runs (Shoot Arrow, slashes), for other units that move while they act, and for casts the
  cancel does not end (channels that are not `channeling_cancelable`).

## Needed from other crates

* aa-fx: the sprite sizes look half of what the real client draws (the Return orb: `Size 0.2` is 0.39 m across in the real capture,
  0.19 m in aa-fx; `AA_FX_SCALE=2` matches it), so check the `Size` convention against a second effect; a continuous emitter's first
  particle comes one pulse period after the start and after its assets loaded, so an arrow in a short flight is faint on its first
  frames (an option to start at once, or to emit at time 0, would do).
* aa-world: the water is too opaque for things standing in a pond (a rat on the bed is invisible); a surface query under the foot
  (object materials: floors, bridges) and a water body height / type query for footsteps in ponds.
* hud: `hud::combat::tests::moves_add_up` fails (not part of this work).
