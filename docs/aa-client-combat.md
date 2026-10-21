# aa-client combat visuals

What a skill looks like on the units that use it and the ones it hits: cast and attack clips, particle effects at the right
bones, flying arrows, hit reactions, death. Code: `crates/aa-client/src/ingame/combat.rs` (events to clips and effects),
`combat/skills.rs` (the database side), `combat/clips.rs` (anim events of a clip), `weapons.rs` (weapon in the hand), `tools.rs`
(skill and pose tools: the uproot shovel), `sounds.rs` (skill sounds, voices, footsteps, blows).
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
skills.mainhand_tool_id ---+
skills.offhand_tool_id ----+--> item_tools.item_id -> item_assets   the tool model in that hand (uproot shovel, watering pot)
anim_actions.mainhand_tool_id / offhand_tool_id -> the same for an NPC pose (the florist's flowerpot)
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
* **Tools.** A skill that works with a tool names it in `mainhand_tool_id` / `offhand_tool_id` (uproot 13789:
  `offhand_tool_id` 2660; watering 13625: `mainhand_tool_id` 14511). The item goes through `item_tools.item_id` to its
  `item_assets` row: 2660 -> `item_tools` 9 -> asset 289 `objects/item/weapon/etc/shovel_general/shovel_general.cgf`, offset
  `(-0.01, 0.16, 0)` and rotation `(10, 88.88, 0)` degrees; 14511 -> tool 18 -> asset 1225
  `objects/env/01_nuia/001_ndeco/quest/ndeco_quest_pot_a.cgf`. The same columns on `anim_actions` give an NPC pose its tool
  (68 `fist_pos_stn_florist_idle`: `offhand_tool_id` 21283 -> asset 9700, the flowerpot). The client draws the tool as a child
  of `item_hand_r` for the main hand and `item_hand_l` for the off hand (the mapping `weapons.rs::carried` uses for
  blades), with the asset's offset and CryEngine rotation (`Rz * Ry * Rx`, the carried weapons' handling), full shading.
  A tool is in the hand while the skill's start or fire clip plays (`SkillTool`, inserted by `play_clip` for the local player
  and remote units) and goes with the action: the end of the clip, a stop step, `SkillEnded` / `CastingStopped` / `PlotStopped`
  or a cancel removes `Acting` and with it the tool (`tools.rs`). An NPC's pose tool (`PostureTool`) lives while the pose clip
  plays. **A drawn weapon is put away instantly**: the owner observed in the retail client (2026-10-21) that planting,
  uprooting, watering and gathering put the weapon away for every weapon type (swords included) with **no holster animation
  and no delay**; the tool animation and the tool in the hand start at once. The client only ends the stance and moves the
  weapon to its sheathed / carried position in the same frame (`Drawn` + `sheathe_for_tool`, `hold_weapons`); the skill's
  events are not held (the holster path `begin_sheathe` is only for a fight that ends or a manual sheathe). Remote units and
  pose tools follow the same rule. The weapon stays sheathed after the action. Whether the real client redraws it afterwards
  is not known: we assume it does not, and only the normal draw triggers (a weapon skill) bring it back. Skill events pick
  the local player's own entity first (`Stage::entity`): a stale remote copy of the same object id must never animate instead
  of the rig the player's camera follows.
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
| `UnitDeath` | after the last scheduled hit on the unit: `dead_01` once, then it stays on the last frame (`Dead`: locomotion never takes the unit back) until the server removes it; a unit other than the player is also thrown away from its last attacker (`Thrown`, see Death) |
| `Resurrected` | `Dead` / `Acting` removed, locomotion restarts |
| the player presses a skill that needs its weapon while it is sheathed (`request_skill`, `NetCommand::UseSkill`) | the weapon comes out and the shield onto the arm at once, the unit takes its combat stance (`<weapon>_ba_combat_idle`) and turns to the target, and the server's events of that skill wait until `DRAW_LEAD` (0.6 s) after the key (`Combat::held`): the first swing starts as late as in the real client (see Weapon draw). A refused skill (not `OUTOF_ANGLE`) puts the weapon back |
| the player presses another skill during a cast time (`preempt_cast`) | the cast is cancelled (`CSStopCasting`, clip and held effects end) and the skill is asked for again 0.25 s later (the server answers `ON_CASTING` to it while the cast runs); the same skill again waits for its cast |
| the local player enters the water (`Mover::swim` turns on, `cancel_in_water`) | every cast of the player is cancelled the same way |
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
  while it is not played), and goes back 10 s after the last action (4 s for an instrument, see Weapon draw).
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
  clips), and all go back to their sheath bones 10 s after the last action (`Drawn { out }`, holster clip first). The shield is on the arm only while
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
* **Weapon draw and combat stance** (`weapons.rs` `Drawn`, `combat.rs` `request_skill` / `sheathe`, `anim.rs` `Stance`). The real
  client (`reference/ingame/combat_melee`, `combat_triple_slash`): from the key the shield is on the arm and the sword in the hand
  within 0.12 s (frame 3), the body turns to the target and stands in the combat idle, the swing's clip starts at about 0.6 s (the
  first damage number is at 0.85 s; `onehand_co_attack_r_slash_2` meets the server's timeline 0.25 s into the clip, the first
  Triple Slash step 0.19 s), then the unit keeps the stance: in `combat_death_2` the shield is still up 7 s after the kill. The data
  has no draw clip (only `*_ac_holster_*`), so the draw is the weapon swap plus the stance idle:
  * `request_skill` reads the player's `UseSkill` (the key), finds the weapon the skill is played with (`skill_weapon`: the clips of
    its `SkillVisual`, the first plot step, the carried blade for the basic attack, which has no clip in the data) and, when it is not
    out in that stance already, inserts `Drawn` and starts a `Lead` of `DRAW_LEAD` = 0.6 s. The server answers after 0.12 s: its
    events of that skill (`SkillStarted`, `SkillFired`, `PlotEvent`, the `UnitDamaged` and end events of its timelines) wait in
    `Combat::held` until the lead is over (`hold_until`, `apply` runs them in order), so the clip, the effects and the damage all
    shift by the same time. A refused skill (`cancel_draw`) lets them go and the weapon goes back; events of other units go on.
    A swing while the weapon is out (the second melee swing, the next Triple Slash) is not delayed.
  * The stance (`Drawn::stance`: weapon out and not holstering) changes the locomotion clips (`drive_locomotion`): standing is
    `<prefix>_ba_combat_idle` (the one handed idle for the bow and the staff), running is `<prefix>_mo_combat_run_<dir>` (the
    model's `fist_mo_combat_run_*` where it has none: `music_`, `bow_`), walking stays the peaceful walk. The prefix is `onehand`
    (also dual wield), `twohand`, `music`, else `fist`. The clips are bound the first time a unit fights in the stance (`ClipSet::stances`,
    shared by the characters of a skeleton).
  * The weapon stays out 10 s after the last action with it (`Drawn::after_action`; 4 s for an instrument): a lower bound from
    `combat_death_2`, where the shield is up at the last frame 7 s after the kill; `CombatChanged` is not read, the server only
    reports "cleared" and the kill is also when the real client says "Battle End". Then `sheathe` plays the holster clip
    (`onehand_ac_holster_side_l`, `twohand_ac_holster_back_r`) on a standing unit, ends the stance and moves the weapon into its
    sheath at the clip's `aw_none` event (0.43 s into the one handed clip); a unit that is running has no holster clip (the real
    `_ub` variant is 1 s long) and puts the weapon away at once; a unit that starts to run during the clip leaves it; the bow and
    the instruments have none. An action in progress keeps the weapon out.
* **Death** (`Thrown`, `throw_corpses`). In `combat_death_2` the rat is thrown up and back when the killing blow lands (frame at
  1.6 s: in the air about 0.9 m behind where it stood), lies on the water surface 0.3 s later, is gone from the surface
  0.3 s after that and is seen faintly on the pond floor for the rest of the capture. A unit other than the player that dies is
  moved on top of its synced transform (the sync rewrites it every frame, so the offset is added after `units::move_units`): away
  from its last attacker by `THROW_DISTANCE` 0.7 m in `THROW_TIME` 0.35 s on an arc of `THROW_HEIGHT` 0.35 m; where
  `WaterQuery::water_at` says the water is deeper than 0.1 m over the terrain it lands on the surface (`surface - 0.03`), floats
  `FLOAT_TIME` 0.2 s and sinks to the ground in `SINK_TIME` 0.45 s. Corpses that were there before the player looked are not moved.
* **Cancelling a cast** (`cancel_cast`): moving, jumping and turning (above), pressing another skill (`preempt_cast`: cancel,
  then ask for it again 0.25 s later, because the server refuses a skill while another casts, `skill_on_casting`, and a channel
  that is not `stop_channeling_on_start_skill` the same) and entering the water (`cancel_in_water`, the transition of `Mover::swim`).
  Only a cast with a cancel rule can be replaced (`Cast::cancel` with `on_move`: every cast time, the channels the data lets
  moving end); a channel without one is left to the server.
* **Sword trail**: the trail effects of the skills (`sword_trail_test1`, bone 9) are children of the weapon hand bone and
  `prime: false`, which is what aa-fx asks for (the ribbon follows the entity: nothing is copied per frame).
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
* `AA_COMBAT_FAKE` also takes `buff:INDEX:BUFF_ID:DURATION_MS` and `unbuff:INDEX` (a `BuffCreated` / `BuffRemoved` on the other unit, else the
  player). `AA_COMBAT_SWEEP=ID,ID,...` (skill ids, `b<buff id>` for a buff) walks the player through every skill / buff in turn on a
  running client (events from the data, a target within 20 m or a spot 6 m ahead, frames at the moments that matter, files in
  `AA_COMBAT_SWEEP_DIR`, then exits); `tools/skill_sheets.py DIR OUT` builds one contact sheet per skill from them. `AA_COMBAT_PRELOAD=ID,ID`
  adds skills to the preload (see below), `AA_COMBAT_NO_PRELOAD=1` turns it off, `AA_COMBAT_HITCH_MS=N` moves the log threshold of
  `hitch: a frame took N ms` (verbose log, default 40).
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

## Skill audit round (animations, buffs, projectiles; `docs/research/skill-audit-anim.md`)

What this round added, all in `ingame/combat.rs` and `ingame/combat/`:

* **Buffs on units** (`combat/buffs.rs`). `aa_net::state::Buffs` says what a unit carries (the unit packet at first sight,
  `SCBuffCreated` / `SCBuffRemoved` later); `reconcile_buffs` shows it every frame, so a buff that was there when the unit came into
  view looks like one applied in front of the player. From `buffs`: the group's items that end at END live as long as the buff
  (`Fx` without a timer, released when it goes), the other ones play once, items that start at END play when it ends; `anim_start_id`
  is the pose (`anims.loop` clips loop for the buff's lifetime, others play once and hold their last frame; the `_start` clip falls
  into the loop: `all_re_combat_stun_start` -> `all_re_combat_stun`), a song buff plays the cast pose of the instrument held,
  `anim_end_id` plays when it goes (stop markers only release the unit), `framehold` freezes the animation (ice, stone) and `ragdoll`
  lies the unit in `dead_01` / `dead_02` (Play Dead). Skills carry their buff through `skill_effects` -> `BuffEffect`, and through
  `skills.channeling_buff_id` / `toggle_buff_id` (Invincibility's aura). 274 distinct buffs belong to player skills, 226 show
  something (220 an effect group, 53 a pose, 5 a freeze, 1 a fall); units beyond 60 m show none.
* **Anim-event effects**: the `effect` / `@effect` events of a clip start their effect on its bone at its time (`/flowers`: the
  flowers at `item_hand_l` 0.29 s into `fist_em_celebrate`; `/sad`: the tears). 114 such events exist in the nuian male database
  (dances, mining, rowing, swimming, the NPC archers).
* **Blade trails** end at the swing's throw moment and stay where they were (`Later::Detach`), they follow the bone before.
* **Stop markers**: a skill whose fire clip is `all_ac_stop` (Use Well) releases the unit instead of logging a missing clip.
* **Preload** (`combat/preload.rs`): the skills of the action bar (plus the basics: hit reactions, death, holster, stances) have their
  clips decoded and bound to the player's rig (2 ms per frame at most, none on a slow frame) and their effects started invisible one
  per frame, so that the first use finds everything loaded. On this machine no skill produced a frame over 35 ms on first use either
  way (7 spell skills, 4 runs); the effects' libraries load asynchronously, so what the preload buys is that the first effect is not
  late, not a measured hitch.
* **Projectiles** by `projectiles.proj_physic_id`: 0 / 1 fly (homing on the target's hit bone, as before; the hit and end items of the
  group now play where the projectile lands: `fireball_hit`), 2 is a **beam** (Arc Lightning, Absorb Lifeforce, Telekinesis, Lasso,
  Mana Force, Crippling Mire: the effect's mesh is one unit long and is stretched from the launch bone to the target's hit bone
  every frame; held until the channel ends), 3 is an **effect area**: all 257 are named "effect area" (Meteor Strike's stardust, Sunder Earth's rocks, the magic circles), the effect stands on the ground at the target and falls, spreads and fades by itself (`Projectile::is_zone`; round 1 dropped them from 30 m, wrongly). A plot step's
  projectile flies at the speed of its special effect (`value3`, m/s: Missile Rain 25) instead of a fixed 0.25 s.
* **Effects at the target's position**: an item for the target of a skill that was aimed at a point (`fx_location_id` 2, no unit)
  starts on the ground there, yaw only (the emitters rise along world up).

Counts of the 231 player skills before and after: clips missing 0 / 0 (the clip side was complete), stop markers 22 / 22 (they are
not clips), effects missing 2 / 2 (typos in the data), skills whose buff shows 0 / 110 of 128 that bring one, anim-event effects 0 / 114,
projectile physics handled 1 of 4 kinds / 4 of 4.

## Round 3 notes (`docs/research/skill-audit-anim.md`)

* The lute attaches to `item_hand_l` (the bone the music clip animates). The staff hangs on `item_side_l` after a weapon skill
  (`StaffSide`, set when it leaves the hand, cleared by the next `all_` clip), else on the back.
* A controller id with no row in `skill_controllers` is no controller (the 1.2 client has no default): Bull Rush (4433) is a shield shove, the
  target's knockdown pose throws it; Precision Strike (703) and Triple Slash (604) move nobody; Pin Down moves by its Charge special effect.
  Dash (kind 4) is forward at run speed for `value1` ms; buffs with a controller (Mana Force) start it on their owner. A `knock_down` buff plays
  `all_re_combat_knockback[_b]_start`, lies, and gets up with `standup_nw_back` / `_stomach` when it ends (`combat/buffs.rs`, round 5 of
  `docs/research/skill-audit-anim.md`).
* `ingame/stealth.rs`: a stealthed character the player sees (itself, friendly, detected) gets CryEngine's cloak layer (`stealth.wesl`: palette rim, scanlines, sparks);
  undetected ones and stealthed NPCs are hidden.

## Beams and character effects (round 6)

**Beams** (`proj_physic_id` 2: Arc Lightning 66, Lasso 86, Telekinesis 71, Mana Force 118, Crippling Mire 38, Absorb Lifeforce 81) were
checked one by one, with fake events (`AA_COMBAT_FAKE`) and with real server casts (see below). All of them spawn their anchor and stretch it
from the launch bone to the target (`projectile N links ...` in the log); the code is right. What the reel saw as "nothing" is the data:

| Beam | Effect | What it looks like |
|---|---|---|
| Arc Lightning | `lightning_magic_launch` | a bright white arc for 0.2 s of a 0.5 s link; a shot has to fall in that window |
| Lasso | `full_launch` (+ `full_form` on the hand) | a thin dark chain from hand to target, from the launch on (about 1 s after the key) |
| Telekinesis | `spell_move_launch` | black smoke streaks once (the 0.2 s effect), the link itself held for the 12 s channel |
| Mana Force | `vital_absorb_launch` | a few small pink streaks for 0.5 s: faint by data |
| Crippling Mire | `psychokinesis_launch` | a wide orange streak, held for the channel (right) |
| Absorb Lifeforce | `life_absorb_launch` | a green beam, held (right) |

The black ring of Crippling Mire is `psychokinesis_launch_form_end` (`AA_COMBAT_NOFX=launch_form_end` removes it, `launch_form2` does not). It is black by data (cracks and a ring mesh, `Facing = Velocity`, aa-fx round 12) and its emitters' axis is the effect's forward: hung on the hand with the caster's orientation the ring stood upright around the caster, looped for the whole channel. The real client draws it as a faint dark ellipse flat on the ground, once (aa-fx `form_end_real.png`). `FxItem::flat()` (THROW, axis 0, name ends `_form_end`: only this item of 2,253) turns the effect's forward to world up and plays it once (not held, not looped); the ring is now a flat dark ring at hand height for about 0.5 s (`AA_COMBAT_FAKE` + `BEAM_SELF`: plot events `@self`). The other held caster effects (1,243 START-to-END items, the FORM and THROW glows) keep the caster's orientation: none of them has an authored axis that shows; the exact rule of the real client for link and held effects is the open question of `docs/research/re-skills.md`. The other `_form_end` items start at LAUNCH or END, or have axis 1, and are not touched.

**The beam rules of the real client** (`docs/research/re-skills.md` 4, Ghidra): the beam sits on the caster's source bone (`projectiles.src_bone_id`; 0 is `bip01`: see the bone table below, a real capture put it at the chest), is re-aimed every frame at the target's `Bip01 Hit` with the effect's +Y (cry; Bevy -Z) along the link and stretched `(1, L, 1)` in cry axes, which is the anchor's `(1, 1, L)` here (the anchor faces the target with its -Z, aa-fx scales along it: the same axis end to end, nothing to change in aa-fx). It lives `special_effects.value2` ms of the step's `ProjectileAnim` (Lasso 700, Crippling Mire 4500, Absorb Lifeforce 6500; 0 for Arc Lightning and Mana Force: the effect plays out by itself, 0.2 to 0.5 s), or the channel of a `SkillFired` (Telekinesis 12 s), and ends with the skill (`cast.held`). The client never starts a finished effect again: an effect lasts only if its own emitters are continuous, so held effects and beams no longer replay (`looping: false`), the flat ring of Crippling Mire plays once and Telekinesis' smoke shows once. Since aa-fx round 13 the loop comes from the effect's own `Continuous` flag and the 50 closest looping fx-item effects are kept, so `Effect.looping` is no longer set anywhere; every fx item effect is spawned with `item: true` and `body: Some(caster)` (ground effects: `item` only), which gives Arc Lightning's `lightning_magic_throw` its blue bolts crawling over the caster's mesh (checked with `AA_COMBAT_FAKE`). Held effects now last only as long as their data loops: in the fake Absorb Lifeforce the green hand glow shows at 2 to 6 s and the beam flickers (it is not continuous by data, the real client showed it steadily: open, aa-fx).

**Bone table** (`combat/skills.rs` `bone_names` / `src_bone_names`, x2game `393E7C40` / `393E7D50`, `docs/research/re-skills.md` 4.4): fx item and dest bones 0 `bip01` (also every unknown id), 1 spine, 2 head, 3 neck, 4 / 5 L / R Hand, 6 / 7 L / R Foot, 8 / 9 `item_hand_l` / `_r`, 10 `effect_1`, 11 `bone_center`, 12 `bone_string`, 13 `bone_string_b8`, 14 to 18 spine, 19 `Bip01 Hit`, 20 / 21 `attach_form` / `attach_launch`, 22 to 24 spine1 to spine3; the projectile source adds 25 to 28 (`effect1` / `effect2` of the main / off hand weapon: the rigs have no such joint, the left hand stands for it, which is the bow's), 29 / 30 `item_side_l` / `_r`. Names match case-insensitively; a rig without the joint gets the unit's origin (id 0) or a point at chest height. The old table was wrong for 1, 3, 4, 5 and lacked 0, 10 to 13, 20 to 30. The override rule is in `launch()`: the items of a projectile group with bone 0, 8 or 9 (the launch flash, the beam) ride the anchor on the source bone; an item of another bone stays on its own bone of the caster.

**Where `bip01` is (real captures, bevycombat on the real client, `data/extract/aa-client-combat/real/`)**: x2game's id 0 is the joint `bip01`, which every clip puts on the ground, so the beams (23 of 30) and the flying shots (src 0: 143 homing and 65 ballistic classes) would leave the feet. The real client does not: Arc Lightning's launch disc hangs at the caster's waist to chest (about 1.2 m, ahead of the torso in the lunge), the Absorb Lifeforce beam runs at chest height from the hand side, the Flamebolt fireball leaves at shoulder height. Which engine detail makes the joint report there (an unextracted root offset or the bind pose, 1.04 m) is not known, so `src_bone_names` sends id 0, 14 to 18 and the unknown ids to `Bip01 Hit` (1.1 to 1.4 m above the unit, follows the pose), the rest of the table is x2game's. The fx items of bone 0 (1,352: auras, rings) stay on the origin: no capture shows one, the real client's launch disc is the only item known to sit on `bip01`. The real beams look different from ours in colour and shape (Arc Lightning is a blue swirl disc and thin strands, ours white strands only: aa-fx), the placement was compared by height only.

**Character effects** (`FxChr`, 12 items: Summon Crows' crows, the pet net, Golem's hand, the whip, the glider): `FxItem::chr`, spawned as
`aa_fx::ChrEffect` instead of `Effect` in `spawn_effect` / `ground_effect`, on the bone like a particle item, `life` = the held limit
(12 s) or 6 s (then the end clip plays). Summon Crows (13340): two crows circle in front of the caster with a black portal.

**Dev**: `AA_COMBAT_NOFX=a,b` mutes the effects whose name contains a part (to find which effect makes a look); the fake op
`T:use:SKILL[:TARGET]` asks the real server for a skill (target 0 or none: `AA_COMBAT_FAKE_TARGET`). A real server cast of the occultism,
sorcery and battlerage skills needs (character 73 `Bevycombat` of the lane DB, `mysql -P3317 -uroot lane_bevy_game`): `characters`
level / abilities set, `skills` rows (id, 1, 'Skill', owner) and `abilities.exp` (the plots check the ability level: Arc Lightning's
plot ends at once below level 15). The server also needs range (20 m), the facing within 90 degrees of the target (the client turns and asks again)
and a live target (a dead one answers code 4).

**Floating numbers** (`hud/combat.rs`): the HUD shows a damage number and writes the combat log line when the blow lands, as the real client does, not when the packet came. `ingame::combat` sends `DamageLanded { damage, received }` when `Later::Hit` is due (after the weapon draw: about 0.6 s after the key from a sheathed weapon; a swing at the clip's sync point; a flight after its time; a plot step as it comes). Buff ticks and units that are not drawn send it at once, and without the game database `pass_damage` forwards the packets. `route_combat` reads the other server events raw (the fight state, heals, experience) and `show_damage` the landed ones. A real melee cast (Triple Slash on a scarecrow, the key at 12.66 s): the packets are applied at 13.27 s and the numbers `COMBAT_TEXT` 15 ms later, the same frame; before they showed at the server's answer, about 0.1 s after the key, while the weapon was still coming out.

## After3 reel items (`docs/research/skill-reel-after.md`, after3)

* **Target hit reaction** (`Stage::hit_reaction`): the unit a blow lands on plays the hit clip of `Act::Hurt` picked from the data:
  `all_re_combat_hit` (`twohand_re_combat_hit` for a two handed target, plain `all_` as the fallback), `..._critical` for a crit, and for
  a normal blow the side variant `_l` / `_r` when the attacker stands more than ~40 degrees off the target's front (`_l` throws the head
  to the left, so it plays when the attacker is on the target's right; found by sampling the head bone of the clips,
  `cargo test -p aa-client hit_reaction_directions -- --ignored --nocapture`). Knockdown and knockback keep their buff poses. Why the doll
  never flinched: the training dummies 7511 / 7512 / 7513 are `models.sub_type = PrefabModel` (no `actor_model` row), so they were never
  drawn and `play` returned silently. `Db::actor_model_file` (`data.rs`, an edit outside the combat files, flagged in the commit) now
  reads the first `prefab_elements` row of a prefab model, opens `prefabs/<file>.xml` from the pak and takes the `object_Model` of the
  named `<Prefab>`. The log line is `combat: ... all_re_combat_hit on <unit> (Hurt, 0.33 s)`. A straw post has almost no bones that
  move with the clip, so the flinch is only visible on a humanoid target (not captured: no humanoid unit near the stage).
* **Leech and Telekinesis show no beam to the doll.** Leech (10104) has no projectile in the data at all (`projectile_id` NULL): its
  fx group 427 has `will_pillage_launch` / `masic_off_launch` at the caster and `masic_off_hit` at the target, so no beam is right. In the
  reel Telekinesis (11504) is the beam of projectile 71 (`proj_physic_id` 2, `spell_move_launch`); with the target 4.3 m away
  (`BEAM_POS=15539,15330`, target `model=1218`, the straw doll) it renders as a row of dark blobs between the caster and the doll, not
  a link: the effect is drawn but looks wrong (fx agent). Test target lesson: `model=1290` is the invisible prefab dummy 14 m away;
  the reel's straw doll is npc 13013, model 1218 (`run_reel.py` passes the doll's object id).
* **Endless Arrows** (14835, plot 1526, event 12104): the arrow is projectile 114 (group 670, `continue_shot_proj`, physic 0) at
  50 m/s (`special_effects` 38 value3): 0.28 s for 14 m, 0.08 s (the clamp) for the doll at 4 m. It renders (a thin streak, hit flash at
  the target) with the particle age fix; the real client's flight is the same data, no real capture was made.
* **Missile Rain's red lattice in the motion reel** is the ground reticle, not an effect of the skill: the key press starts the aim,
  the cursor on the doll is 3 m away, Missile Rain's range starts at 10 m, so the reticle is the red `target_null` ring of 7 m radius
  (`ground_target.rs`) draped around the caster. Seen with `--ground-press 13281 8 --aim 960 700 8.4`. The real client draws the red
  ring out of range too; the reel should aim at 10 m or more (`--aim-world`) or not hold the key. The skill's own events
  (`plot@ground` 209, 212, 231, 237) put `long_distance_shot_hit` on the ground at the target and nothing red at the caster;
  `long_distance_shot_launch` is still missing from the data library (aa-fx).
* **Searing Rain 11939, Frigid Tracks 11314, Flame Barrier 14774, Thwart 16486, Renewal 17412 appear at the caster: matches real.**
  By data: `target_type_id` 0 (self) for the first four (Searing Rain radius 10, Thwart radius 6, the other two no area), Renewal
  `target_type_id` 1 (a friendly target; a hostile or no target makes it the caster). Earlier rounds verified Searing Rain, Frigid
  Tracks and Flame Barrier in the real client as caster centred. Not captured again.
* **Doodad clouts** (Protective Wings 10714): `skills.channeling_doodad_id` 879 -> `doodad_func_groups` (first phase) ->
  `doodad_phase_funcs` `DoodadFuncClout` -> `doodad_func_clouts` 28: friendly relation, buff 258 (`spell_shield_dot`, the
  sparkle aura), projectile 51 (`spell_shield_proj`, `proj_physic_id` 3, the ring of light lines), area shape 14. The server's area
  trigger applies the buff (it arrives as a buff entry and `buffs.rs` draws the aura), the projectile is the client's:
  `SkillVisual::clout_projectiles` are launched at the caster's feet when the skill fires and held until the skill ends
  (`launch` accepts a zone without a bone point). Checked with `AA_COMBAT_FAKE="15:fire@self:100:10714:0:18000;15.05:buff@self:1:258:18000;23.1:end@self:100"`:
  a wide white ring pattern on the ground around the caster, a sparkle haze and the hand orb. Not compared with the real client
  (no capture of Protective Wings), and doodads that other players or npcs spawn (their `DoodadsSpawned` clouts) are not drawn:
  open.

## Not done

* Play Dead, `/bow`, `/kneel` have no clip in the skill data: their pose comes from their buff (`ragdoll`, `fist_ac_sitground`,
  `fist_ac_beggar_01`), which the server must send; Summon Wraith and Meditate have no pose or object in the
  data at all (the wraith would be an npc); Protective Wings' ring comes from its channel doodad's clout (above).
* Beams are stretched along the effect's forward axis, which is right for the meshes of the lightning and rope effects; an effect of
  the same group with sprite emitters stretches with it (sprites are not affected: aa-fx draws them camera facing).
* Races other than Nuian male were checked by data only (all 202 clips bind on all 8 models, 0 missing); no character of another
  race could be created in this session's walkthrough, so nothing was rendered for them.
* **Staves** (round 2; checked against the real 1.2 client with a Civilian Magic Staff, `shots/skill-audit/anim/real_fb_sheet.png`,
  `real_idle2_crop.png`, `real_st_sheet.png`): a staff (`holdables.pose_id` 4, code `2h_blunt_staff`; `Weapon::staff`) is held with two
  hands for the sheath but fights in the one handed stance: `Wield::staff` (not `two_handed`), `Stance::Staff` (`staff_` clips, which
  the `.cal` aliases to the `onehand_` ones), the holster `staff_ac_holster_staff_l`. The `all_` spell clips never draw it: the real staff
  stays on the back during a Flamebolt (the outstretched arms in the real capture are the clip's own). It hangs on `item_bow_l` like
  the bow (diagonal from the left shoulder, head above the shoulder), not on `item_back_c` (that was horizontal across the upper back), and
  the bow it shares the back with is hidden while the staff is stowed (`CarriedItem::hide_sheathed`: `sheathe_priority` 9 against 4).
  Not done: after a weapon skill in the real client the staff goes to the left hip (horizontal) and the bow shows on the back; ours
  returns it to the back.
* Cancelling a cast by jumping off a mount; moving, jumping, turning, another skill and the water cancel here. The rules come from
  the live game's behaviour and the skills' `channeling_cancelable` / `stop_casting_by_turn`, with no 1.2 client capture or
  disassembly to check them against.
* The two handed sword is tied to the right hand bone (the clip's pose holds it with both hands only because the grip is
  close); a dual wielder's second sword sits on `item_hand_l`. The instruments are placed by their `attachment_offset` of the
  right hand only (no frame of a real capture to compare).
* Camera shakes only reach the local player's camera (its own blows, or a shake that hits it) and at half strength; a shake that
  starts at HIT plays when the damage lands (a miss does not shake).
* Mounted, swimming and flying casters, doodad and ship cannon projectiles.
* Decals of the effect groups, `FxMaterial` flashes on the hit; the sounds of songs (bard instruments).
* The player's own movement while a cast clip plays is not blocked (the cast is cancelled instead); the upper body layer is for
  one-shot clips of the player while it runs (Shoot Arrow, slashes), for other units that move while they act, and for casts the
  cancel does not end (channels that are not `channeling_cancelable`).

## Needed from other crates

* aa-fx: beam effects (`proj_physic_id` 2) get the whole transform of their anchor: the combat code scales the anchor's local Z by the distance
  and expects mesh emitters (`beam_test.cgf`, `lightning.cgf`: one unit long along cry +Y = Bevy -Z, radius 0.85) to follow that
  non-uniform scale; sprite emitters should ignore it. Tails of projectile effects at flight speed are very long (Missile Rain's arrow:
  a streak across the screen at 24 m/s); `skill-audit-fx.md` has the details.
* aa-fx: the sprite sizes look half of what the real client draws (the Return orb: `Size 0.2` is 0.39 m across in the real capture,
  0.19 m in aa-fx; `AA_FX_SCALE=2` matches it), so check the `Size` convention against a second effect; a continuous emitter's first
  particle comes one pulse period after the start and after its assets loaded, so an arrow in a short flight is faint on its first
  frames (an option to start at once, or to emit at time 0, would do).
* Footsteps now read the supporting object's or terrain's physical material with `WorldCollision::surface_below`;
  `WaterQuery::water_at` supplies pond, lake and sea footsteps when the foot is submerged. The original terrain lookup
  remains the fallback while collision materials load.

## Complete received skill payloads (2026-10-18)

Synthetic combat/crafting/cooldown fixtures now supply the additive aa-net raw result,
caster and target data alongside their existing display projections. Actual incoming
Started/Fired packets retain every optional result field and identity component; a
nonzero result header no longer shifts Fired's animation/final-byte fields. The native
cooldown-suppression query tests packed bit1 only. Raw final byte1 does not suppress.
Legacy animation/crafting/timer behavior stays unchanged by this integration.

These fields are prerequisites for native cast-manager admission. Received Started/Fired
alone do not prove that a timeline key is absent: native local callers can create one,
and ending a retained node does not immediately erase its tree entry. Full native Fired
and attribute74/GCD producers remain open; no timer success is inferred from these
parser changes. aa-net.md records source/oracle coverage and retained-node limits.

## Ground targeting (`ingame/ground_target.rs`)

Skills aimed at a spot of the ground. What 1.2 says about them (all read from the game database and the client's own strings, the
exe could not be decompiled here):

| `skills.target_type_id` / `target_selection_id` | skills | client behaviour |
|---|---|---|
| 6 Pos, 11 BallisticPos, 15 ArtilleryPos, 13 RelativePos / selection 2 or 4 | player: Meteor Strike 10664 (r 5, range 0..25, cast 5 s), Missile Rain 13281 (r 7, range 10..50, cast 2 s), and the 3.0-era Fiend's Knell 23588, Gods' Whip 23593, Behind Enemy Lines 23587; items and gimmicks: Throw Firebomb, Dig, Plant Seed, Throw Hook ... (about 270 rows) | **ground reticle**, left click casts at the spot |
| 6 Pos / selection 1 (source) | detonations, Stomp, Nuclear Bomb ... (28 rows) | **self position**: the area is centred on the caster, cast at once with the caster's own spot, no reticle |
| 0 self (+ area radius, `target_angle`) | Freezing Earth 10151 (r 8) | not ground targeted in 1.2: plain self cast, the area is round the caster. The **front cone** is `skills.target_angle` (180 for Meteor Strike): the server fails the cast with `skill_outof_angle` unless the spot lies in front of the caster, so the caster is turned first |
| 4 hostile | Crippling Mire 10201, Earthen Grip 14376 | unit targeted in 1.2 (they only became ground skills later): plain `UseSkill` on the target |
| 12 SummonPos, 14 SourcePos, 7 Line | mount/pet summons, Throw Dagger, Spray | plain cast; the server's `initial_target` takes the caster for these |

Healing circles do not exist as ground skills in the 1.2 data (no player skill with a ground type heals).

* **Slot use**: `hud/panels.rs` `use_slot` / `use_item` / `use_bag_slot` send a `SkillPressed` message instead of `UseSkill` / `UseItem`;
  `ground_target::route` sends the plain command for everything but the types above (same frame, nothing changes for them).
* **Reticle** (`reticle`): the decal ring texture `textures/decal/target/target_correct_df.dds` (the texture of both
  `materials/decals/target/target_correct.mtl`, green (0.18, 1, 0.44), and `target_null.mtl`, red (0.74, 0.03, 0.01)), its outer ring
  as wide as `2 * target_area_radius` (`target_decal_radius` where set). Green while the spot is inside `min_range..=max_range`
  (3D distance from the feet less 0.5 m body, as the server's `distance_to(.., include_z)`), red outside. It is a **card of the 3D world**
  (`aa_fx::decal::drape_quad`, up to 12 x 12 cells), drawn by the world camera, depth tested, unlit and blended, lifted 0.25 m and pulled
  towards the camera by a depth bias; every vertex lies on `WorldCollision::ground_below` (brushes and terrain, so it climbs rocks and
  floors) and on the water surface (`WaterQuery`) where there is water, so it shows on a pond. It does not depend on the HUD or the
  plate camera (`--no-hud` works). The first version was a 2D grid on the plate camera: it vanished with `--no-hud` and whenever a
  ring point was behind the camera. The cursor spot is `WorldCollision::raycast` on the camera ray (objects included, so a bush between
  the camera and the ground takes the spot); `TerrainQuery` marching when no brush is loaded.
* **Confirm / cancel** (`aim_input`, `PreUpdate`): left click (not a drag, not on the HUD) casts; a click out of range (more than 1 m
  over the red edge) is not sent and the aim stays; right click and `Escape` cancel; pressing the same skill again casts at the cursor.
  The click and `Escape` are swallowed so they do not also select the unit under the cursor / drop the target.
* **Cast**: a standing `Move` packet with the new facing (the caster turns to the spot) and then `CSStartSkill` with the position
  target (`Command::UseSkillAt`, or `Command::UseItemAt` for an item's skill: kind 1, i64 x / i64 y in 1/4096 m << 32, f32 z, f32 rot 0,
  two object ids 0; the unit test `position_target_decodes_as_the_server_reads_it` reads the bytes the way the server's
  `SkillCastTarget::read` does).
* **Smart ground targeting**: the option `OPTION_ITEM_SMART_GROUND_TARGETING` ("Skill Deployment Location": activates skills at the
  location of the mouse cursor when their shortcut key is pressed; the Korean tooltip adds that a skill used with the mouse click on
  the slot still has to be placed). With it on, a hotkey (the cursor is not over the HUD) casts at the cursor's spot at once; with no
  ground under the cursor the current target's position is used. The slot click always aims.
* Dev flags (seconds from entering the game): `--ground-press SKILL T`, `--aim X Y T` (logical window pixels), `--click T`,
  `--ground-cancel T`, `--ground-smart 0|1`, `--bar-skill SLOT SKILL` (puts a skill on the action bar for `--hud-key slotN`).
* Not found in the client: any other option for casting at the current target's spot (that is only the smart-targeting fallback above),
  the `pointer_a` / `pointer_b` effect models (`objects/effects/pointer_*.cga`) the exe loads next to the decals are not drawn.

Checked against the lane server (account `bevyaoe2`, character Aoecast2 with Magic/Wild/Death abilities, Meteor Strike and Missile Rain learned; shots in `shots/aa-client/aoe/`, git-ignored):

* `reticle_far.png` (green, Meteor Strike, 19 m, range 25), `reticle_red.png` (red, Plant Seed 13139, 13.6 m, range 2).
* `meteor_cast.png`: `ground: cast skill 10664 at (15579.5, 15396.1, 126.0), 12.4 m away` -> the server answers `SkillStarted ... target kind 1 at (15579.541, 15396.062, 126.0335), cast 5000 ms`, then `SkillFired` at the same spot; the meteor effect falls there and the cast bar shows.
* `meteor_turned.png`: aimed to the left, the caster turned (facing 0.66 rad) before the cast and the server accepted it (`target_angle` 180).
* `missile_rain_cast.png`: through the real slot path (`--bar-skill 4 13281 --hud-key slot5`): `PlotEvent`s of the plot skill all carry the clicked spot as `target_pos`.
* Smart option (`--ground-smart 1`, `--hud-key slot5`): cast at the cursor at once; a click out of range logs `not cast, still aiming`; `--ground-cancel` cancels.
* Reticle checks (`shots/aa-client/aoe/decal_*.png`, `--no-hud`, camera 5 m and 12 m): `decal_dry_5` (terrain, climbs a rock),
  `decal_dry_12` (terrain), `decal_pond_5` (a ring on the water of the pond), `decal_pond_12`, `missile_aim` (Missile Rain, 12.1 m).
* For the reel: `--aim-world X Y T` holds the cursor on a ground point (game coordinates) through the camera. Missile Rain needs
  the spot at least 10 m from the caster: put the caster 11 to 12 m from the doll (west of the dirt-path doll at (15543, 15330),
  e.g. `--pos 15531,15330 --yaw -90` or a `tp` there) and use `--aim-world <doll x> <doll y> T`; Meteor Strike works from 3 m.
* Not run live: item skills (`UseItemAt`, no item with a ground skill in the test bag) and the self-position mode (no player skill); both are the same code path (`SkillTarget::position`, caster own spot).


## Skill movement (`ingame/skill_motion.rs`)

Skills that move units: Charge, Overwhelm and Drop Back (the caster), Lasso and Vicious Implosion (a pull), the knockbacks of Charged Bolt,
Whirlwind Slash, Focal Concussion, Meteor Strike, ... and the blinks of Teleportation and Shadow Step. Code: `skill_motion.rs` (systems and
components), `skill_motion/path.rs` (the paths as plain math, unit tests), `skill_motion/book.rs` (database lookups).
Videos (`tools/reel/run_motion.py`: fake server events or real casts on the lane plus `aa-dev --record`, account `bevymotion`, character
Motioncaster): `shots/skill-motion/*.mp4` (`charge`, `lasso`, `pull`, `knockback`, `overwhelm`, `dropback`, `shadowstep`, `teleport`; the
`*_live` ones are cast for real through the lane server).

### How 1.2 moves units (data and server)

* **Who moves.** The 1.2 server never moves a player character for a skill: its controllers (`systems/skills/controllers.rs` of the AAEmu
  server) run on non-character units only ("the 1.2 client moves itself"; `skill_controller` returns for player casters, `knock_back` for
  player targets, `Blink` / `TeleportToUnit` send the player a packet). The client runs the controller from the skill's data and reports
  where it went with its normal movement packets (`CSMoveUnit`; the zone's `ZWReportMoveHack` check is off: `movement_verify_enable` 0).
  For NPCs the server runs the controller and streams `SCOneUnitMovement` every 100 ms; a knockback moves an NPC with one stopping
  `SCOneUnitMovement` at the end point (no `SCBlinkUnit`).
* **Where the controller comes from.**

  | Data | Skills | What the server sends |
  |---|---|---|
  | `plot_effects` row `SkillController` (348 rows) of a plot step: `source_id` moves towards `target_id` | Charge ev 5100 (source 1 the caster, target 4 the step's target), Overwhelm ev 5107 / 7062, Tiger Strike ev 118 / 119, Lasso ev 2659 (source 4, the step's target, is pulled towards 3, the caster), Vicious Implosion ev 4628 (one step per victim) | `SCPlotEvent` for every step that has a special or controller effect, with the step's source and target |
  | `skills.skill_controller_id` of a skill **without** a plot | Drop Back (controller 8074: 15 degrees, 14 m/s, 700 ms, offset -10 m, target = itself) | `SCSkillFired` only |
  | `special_effects` type 11 `Blink` (`value1` metres forward) / 47 `TeleportToUnit` (mm and degrees round the target) | Teleportation 10152 (15 m), Shadow Step 12075 (1.3 m behind the target) | `SCBlinkUnit` (`distance`, `degree`, the end point the server worked out against the terrain) to the caster only |
  | `special_effects` type 13 `KnockBack` (`value1` mm, 0 = 3 m; `value2` degrees) of a plot step or a skill | Whirlwind Slash ev 2694 (0.45 m, 40), Focal Concussion (0.4 m, 0), Charged Bolt (1.2 m, 30), Meteor Strike (0.7 m, 70), Overwhelm ev 5116 (0.2 m) | `SCUnitDamaged` of the victims; an NPC victim is moved by the server (one report), a player by its own client |
  | `ImpulseEffect` / `PhysicalExplosionEffect` | vehicle and rider skills (`ability_id` 0), Vicious Implosion's explosion | `SCUnitImpulse` (0x79; aa-net does not decode it: no player skill of the ten trees uses it) |

* **The controller** (`skill_controllers`, `kind_id` 2 Leap, 4 Dash; `value1` ... `value13`) is the server's `LeapArc` (zone `0x39231780`):
  `value1` launch angle (degrees), `value2` minimum speed (m/s), `value3` duration (ms), `value4` offset (mm: the end point is that far
  past the reference unit on the line from the mover to it, negative stops short), `value7` direction (1 only towards the reference, 2 only away:
  else the mover stays), `value9` turn to the direction of travel, `value12` scale the duration by the attack speed. The mover goes in a
  straight line at `max(value2, distance / duration)` (at most 50 m/s) and, with an angle, along a parabola that lands on the end point; with
  itself as the reference it goes `offset` metres along its facing. Charge: 400 ms, 10 m/s, offset -0.8 m, forward only (a target 10 m
  away: 9.2 m in 0.4 s = 23 m/s); Lasso: 30 degrees, 40 m/s, offset -1 m (11 m in 0.28 s, an arc 1.6 m high); Overwhelm: 45 degrees, 7 m/s,
  600 ms (9 m, 2.3 m high); Drop Back: 10 m back in 0.7 s, 0.7 m high.
* **Server evidence** (`data/extract/motion/live.jsonl`: the lane server, `aa-net` example `skilltest` with the account `bevymotion`):
  Charge `SCPlotEvent` 5099 and 5100 at +155 ms, 5101 at +545 ms, the damage at +597 ms (the leap's 400 ms after the controller step);
  Lasso 2656 at +125 ms, 2657 +1126, 2659 +1173, 2658 +1847; Drop Back one `SCSkillFired` (target = the caster) at +109 ms and nothing else;
  Teleportation `SCBlinkUnit` {distance 15, end point} with the fire; Shadow Step `SCBlinkUnit` {distance 0, a point behind the target};
  Vicious Implosion eight `SCPlotEvent` 4628 at +143 ms (one per victim); Overwhelm 5107 at +171 ms.

### What the client does

* `combat::apply` hands every server event it applies (after the weapon draw delay, so in step with the clip) to `skill_motion::observe`;
  the `Inbox` it fills is turned into moves by `start_moves`. Per timeline it remembers the caster and the target (`SkillStarted`,
  `SkillFired`, the first `SCPlotEvent`): a plot step names only its own source and target.
* **The local player** gets a `SkillMove` component. While it is there `player::control_player` does nothing (input locked, auto-run off) and
  `drive_local` plays the path: constant horizontal speed, parabola in z, `WorldCollision::slide` for walls (a wall ends the move where the
  character is), never under `floor_at` (terrain or brush), landing on it. It sends a `CSMoveUnit` every 100 ms on the way (jump / fall
  state, velocity, forward delta) and a standing one at the end, so the server and the other players see the flight. `SCTeleportUnit` cancels
  a move and `follow_server` puts the player at the server's point. The clips are the combat module's (Charge `all_co_sk_dashattack`, Drop Back
  `all_co_sk_backstep`: the controller's `start_anim`).
* **Knockbacks** (a `KnockBack` skill hurts the player, a plot step's or a plain skill's, seen on `SCUnitDamaged`): a parabola away from the
  pusher, the distance from `value1`, the launch angle from `value2` (20 to 70 degrees, 0.3 to 1 s under gravity 25), the clip
  `all_re_combat_knockback_start`, then 0.45 s on the ground with `all_re_combat_crash_end` (the get-up) before the player can walk again.
  A made-up push for tests: `AA_MOTION_FAKE="T:push:DIST_MM:ANGLE"` (away from `AA_COMBAT_FAKE_TARGET`).
* **Blinks**: `SCBlinkUnit` for the local player puts it at the point at once (a `SkillMove` of length 0); the camera pivot trails and catches
  up. Another unit's blink resets its movement track, so it is not glided across the gap. `AA_MOTION_FAKE="T:blink:MM"` (forward) and
  `"T:behind:MM"` (behind `AA_COMBAT_FAKE_TARGET`) make the packet up.
* **Made-up events have no `SCBlinkUnit`** (the skill reel's `fire@self` of Teleportation, `fire` of Shadow Step): with `AA_COMBAT_FAKE` set,
  `SkillFired` of a skill with a `Blink` (type 11, `value1` metres forward) or `TeleportToUnit` (type 47, behind the target, 1.3 m) special
  effect makes the blink itself (`Starter::sim_blink`): a 50 ms dash that `WorldCollision::slide` and the floor stop as the server's terrain query
  would. With the real server nothing is made up. Teleportation in the reel therefore moves 15 m and the camera slides over.
* **Vegetation and Drop Back.** `WorldCollision` has the vegetation that is physical in the client (`vegetation.dat` instances whose model has a
  physics proxy: tree trunks, boulders, some plants; 85 % of the instances, `CVegetation::Physicalize`), and a back step stops at those like a
  walking player. Bushes without a proxy are walked through by the real client as well, so a Drop Back that ends in a thicket is correct and
  the camera may then show leaves; the skill reel casts it from an open patch instead (`tools/reel/plan.py` `DROP_BACK_SPOT`).
* **Other units**: players and NPCs that run a controller arrive as movement reports and are played by `remote::Track` (smooth already); the
  module simulates their controllers only when `AA_COMBAT_FAKE` or `AA_SKILL_MOTION_SIM=1` is set (nothing else moves them then). An NPC that is
  knocked back has one report at the end: the move waits for it (`RemoteUnit::latest`, at most 0.5 s, else the predicted end point), plays the
  parabola from where the unit is to where the server put it with `all_re_combat_knockback_start` / `crash_end`, and resets the track there. A
  pulled unit plays `all_re_combat_knockback_b_start`. A killed unit keeps `combat`'s `Thrown` flight.
* **Camera**: `CameraLag` (read by `camera::follow_player`) is how far the orbit pivot is behind the player: a move pushes its displacement
  in, it decays at 7 / s and is capped at 7 m. A charge is followed, a blink slides the view over.
* Hooks outside the module (all small): `combat.rs` `Stage.motion` + one call at the top of `apply`; `player.rs` `floor_at` is `pub(super)` and
  `control_player` returns early while a `SkillMove` runs; `camera.rs` `follow_player` adds the lag; `units.rs` `is_character` / `latest`;
  `remote.rs` `Track::latest`; `mod.rs` the plugin. `AA_SKILL_MOTION_LOG=1` logs every move.

### Not done

* `SCUnitImpulse` (0x79) is not decoded: only vehicle and rider skills use it (and aa-net's `game.rs` / `session.rs` were being edited by
  others).
* Not captured from the 1.2 client: the exact knockdown clips and durations, the look of a blink (fade, trail), whether a struck player is
  locked for the whole get-up. The numbers are the data's and the server reference's; the knockdown timing and the clips are chosen.
* Floating (kind 1), Wandering (3) and Rope (5) controllers (gliding, fear, harpoons) are not played for players; units under them follow
  the server's reports. A pull or knockback of another player's character is what their client reports (10 Hz, smoothed by `Track`).
* The recording hang (the client stops with no CPU use, mostly in the Shadow Step scenes, 3 of 3 runs) is **not** in the skill movement:
  `aa_dev::record::capture` (`crates/aa-dev/src/record.rs`) waits in `while pending >= MAX_PENDING { sleep(4 ms) }` inside a `Last` system
  while `pending` also counts screenshots the GPU has not handed back yet. Those are delivered by frames that the blocked system stops,
  so with 10 screenshots in flight (the scene renders at 100 fps on a quiet frame, the readback lags) it waits forever. Stack dump of
  the hung process (`tools/reel/threaddump.ps1`, written to `data/extract/motion/hang/`): the one running Bevy task sits in
  `std::thread::sleep` at `record.rs:115`, no PNG writer thread exists, every other thread is parked. The fix (aa-dev, not applied here):
  count `in_flight` (requested, not delivered) apart from `writing` (encoder threads), sleep only on `writing`, and when `in_flight`
  is at the cap freeze the clock (`TimeUpdateStrategy::ManualDuration(Duration::ZERO)`) and return, so frames keep passing. With that,
  4 of 4 Shadow Step recordings and the two re-recorded videos complete (180 frames each). `run_motion.py` dumps the stacks with
  `threaddump.ps1` and kills its own client when the log stops growing for 60 s.
