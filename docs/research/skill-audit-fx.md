# Skill audit: particle effects (aa-fx)

Scope: the effect groups of the 391 shown player skills of `data/extract/aa-client-combat/skill_coverage.tsv` (435 distinct
effects). Method: every effect rendered in isolation by the `effects` example (`--list`, contact sheets of 12, frames at 0.3 to 3 s),
and the ones with a visible problem or a large share of the data compared against the real 1.2 client (the account `bevyfxaudit`:
`Particle.SpawnEffect(name, pos, dir, scale)` through the client driver, a screenshot per 0.2 s, next to the Bevy render from the same
camera). Sheets: `shots/skill-audit/fx/raw` (before round 6, 33 sheets of the library), `shots/skill-audit/fx/after` (after), single
real vs Bevy comparisons in `shots/skill-audit/fx/real` (top row Bevy, bottom row the real client). Counts below are by effect.

## Result

| | effects |
|---|---|
| rendered and compared with the real client | 10 |
| rendered, behaviour changed by a round 6 fix (FocusAngle, Velocity facing, mesh lighting, stretch, frame blending, tile aspect) | 387 (the table lists which) |
| not in the libraries (sound only `x2_sounds.particles.*`, or a name the libraries do not have) | 9 |
| still wrong (known) | 1 group after round 8: `white0001_hit` ring orientation (see below) |

"Correct" means: rendered, looks like the data says and, where compared, like the real client. Only 10 effects were compared one by
one; the rest of the 435 were looked at on the sheets for gross errors only. The first number is therefore what is verified, the second
what changed and was spot checked.

## What was wrong and is fixed (all in `crates/aa-fx`)

Found by reading the engine (`cry3dengine.dll` decompiled with IDA, `particles.cfi`) and checked on the real client:

1. **FocusAngle** (4427 emitters in the libraries, 185 of the shown skills' effects). The engine tilts the emitter axis (cry +Y) about X
   toward +Z and spins the tilt by `FocusAzimuth` (default 0: fixed). aa-fx scattered it in a random azimuth, so `FocusAngle="90"` fire,
   smoke and sparks flew sideways and down. Now they rise. Real: `el_wind0004_hit` plume.
2. **Tile aspect** (`ResourceParticleParams::ComputeAspect` and the sprite writer `sub_3009CFD0`). The engine keeps the longer side of a
   texture tile at the authored `Size` and shrinks the other (width times `w/h` for a tall tile). aa-fx made tall tiles up to four times
   taller. Every streak, beam, `glow_big_velocity*` flare and sword line was too long (the "giant beams" and "oversized streaks" of the
   reel, for example `hitup`).
3. **Stretch and OrientToVelocity** on camera facing sprites. The engine builds a 2x2 matrix over the camera axes: roll, then the turn
   onto the screen velocity (plus the roll), then every row grows along the screen velocity by `velocity * stretch` (half length), and the
   centre moves on by `StretchOffsetRatio * stretch * velocity`. aa-fx aligned stretched sprites without `OrientToVelocity` and added
   half as much.
4. **Facing = Velocity** (287 emitters): a 3D card whose normal follows the direction of travel (rings of projectiles). Was a camera
   facing streak.
5. **Lit meshes** (`Illum` material, 126 effects with a mesh). The material colour (0.07 for the stones) is a reflectance, lit by the sun
   (about 7.6) and sky (about 1.8) of the engine; aa-fx multiplied the tiny colour by a dark factor: the stones of Sunder Earth and the
   golem hand of Earthen Grip were black. Now calibrated on the real rock of `ground_launch_standalone`.
6. **Material over Texture** (49 emitters have both): the material's diffuse texture is what the engine draws.
7. **AnimBlend** (`TextureTiling` field 7): two frames blended. **Colour gamma**: the engine raises the authored colour to the power
   2.2 (`particles.cfi`); `FxSettings::colour_gamma` (default 2.2, `AA_FX_GAMMA=1` for the old look).
8. **Endless tails**: a weapon trail with no `ParticleLifeTime` (49 tail emitters) asked for 100 or 200 s of tail spread over 64 samples
   (a beam across the map: the "giant diagonal beam" of Pin Down, the long streak of Whirlwind Slash): at most one second now.

## Reel items of the coordinator's list

| skill | effect | verdict |
|---|---|---|
| Retribution | `death.revenge_armor_launch` | matches the real client (the white dotted columns are in the real one too); the "slab" was the old scene lighting |
| Earthen Grip | `illusion.ground_restriction_form` | the black blob was the lit mesh bug; now the hand falls with the orange streaks like the real client. It is at the caster in the reel: spawn side |
| Sunder Earth | `fight.ground_launch_standalone` | the black rock was the lit mesh bug; now a lit grey rock like the real client (the real rock is a few jagged pieces, ours one big spike); in the reel it starts 6 m in the air because the projectiles are physic 3 (a ground zone) and the client drops them from 30 m (round 9, combat side) |
| Antithesis | `p_skill_love.hill_attack_*` | lotus petals of tall tiles (tile aspect); not compared |
| Ollo's Hammer | `p_skill_adament.hammer_*` | matches the real client: glowing cube outlines; the 14 m cyan band of the reel flight was `haje_hammer_proj5` stretched over the inherited velocity (round 9, fixed) |
| Whirlwind Slash | `fight.swirl_slash*` | the rings are in the real one too; the long thin streak of the reel frames is the equipped two-hander in its horizontal pose, not a particle (round 9) |
| Pin Down | `x2_effects.sword_trail_test1` | endless tail bug, fixed |
| Stillness | `death.wide_silence_launch` | big dark rings by data (13 m); the real one could not be seen (a persistent hammer effect of my own test hid it) |
| Skewer | `p_skill_love.judge_spear_hit` | matches the real client (tall violet vortex, white swirls, dark spear) |
| Freezing Earth | `magic.ice_piece_launch` | 10 m ice decal by data; the real one not seen (hidden by the same hammer) |

About 50 skills "without a visible effect": all but the ones below have an effect that draws (see the table, none is empty). They are
small by data (a few additive sprites of 0.1 to 0.5 m: `fear_form`, `buble_form2`, `tired_curse_throw`, `def_attack_hit_l`,
`hide_launch`, `life_absorb_*`, `spellblade_*`), or the spawn does not happen. Missing from the libraries: `illusion.tired_curse_form`
(Lassitude), `wild.long_distance_shot_launch` (Missile Rain), `p_skill_adament.p_skill_disarmament` (Strong Blow) and the sound only
`x2_sounds.particles.*` of the songs (Startling Strain, Rhythmic Renewal, Twilight, Healing Hymn, Play Lute): there is nothing to draw,
the songs' light comes from other effects or from the character.

## Still wrong

Round 7 (see `docs/aa-fx.md`, "Round 7") closed most of this list:

* `magic.firewall_launch` (Flame Barrier): fixed. The `ParentStart` children with an emitter life ride on the falling parent particle
  (a 3 m column of puffs, was one ball at the birth point); the column is now as tall as the real one.
* Lights (200 effects), decals and `EmissiveHDRDynamic`: implemented (pooled `PointLight`s, 8 by default, 0.01 ms; `Facing=Decal`
  cards drape on `FxSettings::ground`, `aa_fx::decal::drape_quad` for the ground target reticle; the emissive HDR power).
* Colour keys with two components had a blue of 1 (violet fire ends): now 0.
* Brightness after the gamma change: measured against the 15 real captures, no global change justified (see the doc).
* Still wrong: `white.white0001_hit` (the hit of many weapon skills): the real client draws its three `hit_exp_ssd` lace rings as thin 3D
  rings (one flat, two vertical), aa-fx as camera facing rings. The lace itself (the material's alpha test against a screen-space noise map)
  is drawn since round 8. The engine path says camera facing; I found no mechanism for the 3D. 3 particles per hit: visible, not dominant.
* Calibrations: the mesh lighting (two constants `LIT_SUN` / `LIT_SKY`) was fitted on one rock; `gl` (rock) and `rv` (armour) are about 2.5
  times brighter than the real ones in the capture comparison.

## Performance

The CPU cost of a busy skill is small: 12 skill effects at once (6000 particles, 7000 quads) cost 0.7 ms of simulation and 0.02 ms of
mesh upload per frame; one effect is about 0.1 ms. The hitch when a skill is used the first time is asset loading (library, textures,
meshes), which aa-client's `combat/preload.rs` warms (effects started invisible); aa-fx has no further cost on first use. The earlier perf
change (views that nothing sees skip their mesh rewrite) is committed.

## For the combat agent (spawn side)

* **Orientation.** `Particle.SpawnEffect(name, pos, dir, scale)` makes the emitter's cry +Y the direction `dir`. Every `FocusAngle="90"`
  emitter (fire, smoke, sparks: about a third of all emitters) rises along the emitter's +Z. Verified on the real client: spawn
  `el_wind0004_hit` with `dir = (0,0,1)` and the plume spreads horizontally, with `dir = (0,1,0)` it rises. So hit, buff and ground effects
  must be spawned with a horizontal, yaw only forward (up stays world up), never pointing at the target in 3D. Projectiles are different:
  they are oriented along their flight, as authored.
* **Endless effects.** Emitters without a lifetime loop until stopped (`hammer_proj1`, the projectiles' `*_proj`, auras): the effect has
  to be despawned when the projectile ends or the buff drops. A loop I spawned for a test is still burning in the real client; the
  Ollo's Hammer cubes are such a loop.
* **Position for ground and area effects.** Earthen Grip, Meteor Strike, Missile Rain, Crippling Mire, Arc Lightning, Thwart are drawn at
  the caster in the reel: their effect (`@ground` / location 2) needs the skill target's position. aa-fx draws an effect at its entity.
* **Trails.** Free ribbons (`swirl_slash*`, weapon trails) sample the entity's position every frame: they need the weapon bone's entity
  with its real motion; a trail effect spawned at the caster's root draws a streak of the root's movement.
* **Effects the libraries do not have:** `illusion.tired_curse_form`, `wild.long_distance_shot_launch`,
  `p_skill_adament.p_skill_disarmament`: skip them quietly.

## Table of every effect

Skills: the first ones that use it. "What it uses": features present in any of its emitters. Status as described above.

| effect | skills | what it uses | status |
|---|---|---|---|
| abillity_skill_table_m.fight.attack_speedup_arm | Battle Focus | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| abillity_skill_table_m.fight.attack_speedup_arm2 | Battle Focus | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| abillity_skill_table_m.fight.attack_speedup_cast | Battle Focus | AnimBlend, FocusAngle, OrientToVelocity, Stretch, decal, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.fight.reckless_cast | Frenzy; Berserk | AnimBlend, OrientToVelocity, Stretch | rendered; round 6 changes apply (Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.fight.ground_launch | Sunder Earth |  | rendered; not compared with the real client |
| abillity_skill_table_m.fight.ground_launch_standalone | Sunder Earth | FocusAngle, OrientToVelocity, Stretch, decal, material, mesh | compared with the real client (stone lit like the real rock) |
| abillity_skill_table_m.fight.ground_launch_standalone2 | Sunder Earth | FocusAngle, OrientToVelocity, Stretch, decal, material, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch, OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.fight.ground_launch_standalone3 | Sunder Earth | FocusAngle, OrientToVelocity, Stretch, decal, material, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch, OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.fight.ground_launch_standalone_end | Sunder Earth | OrientToVelocity, Stretch, mesh | rendered; round 6 changes apply (mesh, Stretch, OrientToVelocity); not compared with the real client |
| x2_effects.sword_trail_test1 | Sunder Earth; Triple Slash; Shadowsmite; Overwhelm; Pin Down; Uppercut; Flurry R | OrientToVelocity, material | rendered; round 6 changes apply (OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.fight.dash_hit | Charge; Uppercut; Graymist Warrior's Charge | AnimBlend, FocusAngle, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.fight.dash_launch | Charge | AnimBlend, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| x2_effects.p_effect.p0000 | Charge; Melee Attack; Uppercut; Graymist Warrior's Charge | Stretch, material | rendered; round 6 changes apply (Stretch); not compared with the real client |
| abillity_skill_table_m.fight.critical_damage_hit | Precision Strike; Strong Strike | AnimBlend, FocusAngle, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.fight.critical_damage_hit_level2 | Precision Strike | AnimBlend, FocusAngle, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.fight.critical_damage_hit_level3 | Precision Strike | AnimBlend, FocusAngle, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.fight.critical_damage_sword | Precision Strike; Strong Strike | OrientToVelocity, material | rendered; round 6 changes apply (OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.fight.restrict_clear_launch | Bondbreaker | FocusAngle, OrientToVelocity, Stretch, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch, OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.fight.full_form | Lasso | FocusAngle, OrientToVelocity, Stretch, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch, OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.fight.full_launch | Lasso | mesh | rendered; round 6 changes apply (mesh); not compared with the real client |
| abillity_skill_table_m.fight.swirl_slash | Whirlwind Slash | Facing=Velocity, FocusAngle, OrientToVelocity, Stretch, mesh | rendered; round 6 changes apply (FocusAngle, Facing=Velocity, mesh, Stretch, OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.fight.swirl_slash_hit | Whirlwind Slash | FocusAngle, OrientToVelocity, Stretch | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.fight.swirl_slash_level2 | Whirlwind Slash | Facing=Velocity, FocusAngle, OrientToVelocity, Stretch, mesh | rendered; round 6 changes apply (FocusAngle, Facing=Velocity, mesh, Stretch, OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.fight.swirl_slash_level3 | Whirlwind Slash | Facing=Velocity, FocusAngle, OrientToVelocity, Stretch, mesh | rendered; round 6 changes apply (FocusAngle, Facing=Velocity, mesh, Stretch, OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.fight.swirl_slash_swoosh_level1 | Whirlwind Slash | FocusAngle, OrientToVelocity, material | rendered; round 6 changes apply (FocusAngle, OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.fight.swirl_slash_swoosh_level2 | Whirlwind Slash | FocusAngle, OrientToVelocity, material | rendered; round 6 changes apply (FocusAngle, OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.fight.swirl_slash_swoosh_level3 | Whirlwind Slash | FocusAngle, OrientToVelocity, material | rendered; round 6 changes apply (FocusAngle, OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.fight.wind_slash_launch | Tiger Strike; Falcorth - Alpha Buffalo - Charge | AnimBlend, Facing=Velocity, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (Facing=Velocity, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.fight.3stage_slash_step3_launch | Triple Slash | FocusAngle, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.fight.3stage_slash_step3_trail | Triple Slash | OrientToVelocity, material | rendered; round 6 changes apply (OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.fight.hitup | Triple Slash; Uppercut; Graymist Warrior's Uppercut | FocusAngle, OrientToVelocity, Stretch | compared with the real client (flare, thin streak, sparks match) |
| abillity_skill_table_m.fight.hitup_level2 | Triple Slash | FocusAngle, OrientToVelocity, Stretch | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.fight.roaring_launch | Terrifying Roar | Facing=Velocity, FocusAngle, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (FocusAngle, Facing=Velocity, Stretch, OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.illusion.tired_curse_form | Lassitude |  | not found in the libraries (not rendered) |
| abillity_skill_table_m.illusion.tired_curse_throw | Lassitude | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| abillity_skill_table_m.magic.magic_reinforce_proj | Lassitude; Enervate | AnimBlend, Stretch | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| abillity_skill_table_m.illusion.buble_form | Bubble Trap; Dahuta's Breath | Stretch, material | rendered; round 6 changes apply (Stretch); not compared with the real client |
| abillity_skill_table_m.illusion.buble_form2 | Bubble Trap |  | rendered; not compared with the real client |
| abillity_skill_table_m.illusion.buble_hit | Bubble Trap | AnimBlend, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.illusion.buble_throw | Bubble Trap; Dahuta's Breath | Stretch, material | rendered; round 6 changes apply (Stretch); not compared with the real client |
| x2_effect_o.mon_effects.mon_water_bunch_proj | Bubble Trap | FocusAngle, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch); not compared with the real client |
| abillity_skill_table_m.illusion.mental_destroyed_hit | Enervate | FocusAngle, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.illusion.mental_destroyed_hit_level2 | Enervate | FocusAngle, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.illusion.mental_destroyed_hit_level3 | Enervate | AnimBlend, FocusAngle, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.illusion.mental_destroyed_throw | Enervate | FocusAngle, Stretch | rendered; round 6 changes apply (FocusAngle, Stretch); not compared with the real client |
| abillity_skill_table_m.illusion.fear_form | Insidious Whisper | AnimBlend | rendered; round 6 changes apply (AnimBlend); not compared with the real client |
| abillity_skill_table_m.illusion.fear_hit | Insidious Whisper | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| abillity_skill_table_m.illusion.fear_throw | Insidious Whisper | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| abillity_skill_table_m.death.death_trick_launch | Play Dead | Stretch, material | rendered; round 6 changes apply (Stretch); not compared with the real client |
| abillity_skill_table_m.illusion.spell_clear_form | Purge | AnimBlend, FocusAngle, OrientToVelocity, material | rendered; round 6 changes apply (FocusAngle, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.illusion.spell_clear_form_hand | Purge | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| abillity_skill_table_m.illusion.spell_clear_hit | Purge | FocusAngle, OrientToVelocity, Stretch, material, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch, OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.illusion.spell_clear_launch | Purge | FocusAngle, OrientToVelocity, Stretch, material, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch, OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.illusion.spell_clear_throw | Purge | OrientToVelocity, Stretch | rendered; round 6 changes apply (Stretch, OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.illusion.poison_exp_hit | Focal Concussion | FocusAngle, mesh | rendered; round 6 changes apply (FocusAngle, mesh); not compared with the real client |
| abillity_skill_table_m.illusion.poison_exp_launch | Focal Concussion | FocusAngle, mesh | rendered; round 6 changes apply (FocusAngle, mesh); not compared with the real client |
| abillity_skill_table_m.illusion.poison_exp_throw | Focal Concussion |  | rendered; not compared with the real client |
| abillity_skill_table_m.illusion.wakeup_launch | Courageous Action | FocusAngle, OrientToVelocity, Stretch, decal, material, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch, OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.illusion.water_wave_launch | Dahuta's Breath | AnimBlend, Facing=Velocity, FocusAngle, OrientToVelocity, Stretch, material, mesh | rendered; round 6 changes apply (FocusAngle, Facing=Velocity, mesh, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.illusion.shout_fear_launch | Banshee Wail | AnimBlend, FocusAngle, OrientToVelocity, Stretch, decal, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.illusion.shout_fear_throw | Banshee Wail | AnimBlend, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.illusion.ground_restriction_form | Earthen Grip | FocusAngle, OrientToVelocity, Stretch, decal, material, mesh | compared with the real client (hand and falling streaks match) |
| abillity_skill_table_m.illusion.ground_restriction_form_hand | Earthen Grip |  | rendered; not compared with the real client |
| abillity_skill_table_m.illusion.ground_restriction_hit | Earthen Grip; Wave of Earth | FocusAngle, OrientToVelocity, Stretch, decal, material, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch, OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.illusion.ground_restriction_proj | Earthen Grip; Wave of Earth | FocusAngle, mesh | rendered; round 6 changes apply (FocusAngle, mesh); not compared with the real client |
| ability_skill_table_k.p_skill_adament.unbeatable_form | Invincibility | AnimBlend, FocusAngle, Stretch | rendered; round 6 changes apply (FocusAngle, Stretch, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_adament.deffence_a_launch_level2 | Redoubt | AnimBlend, FocusAngle, OrientToVelocity, Stretch, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_adament.deffence_a_launch_level3 | Redoubt | AnimBlend, FocusAngle, OrientToVelocity, Stretch, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_adament.deffence_active_launch | Redoubt | AnimBlend, FocusAngle, OrientToVelocity, Stretch, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_adament.def_attack_hit_lv1 | Shield Slam; Bleeding | mesh | rendered; round 6 changes apply (mesh); not compared with the real client |
| ability_skill_table_k.p_skill_adament.def_attack_hit_lv2 | Shield Slam | mesh | rendered; round 6 changes apply (mesh); not compared with the real client |
| ability_skill_table_k.p_skill_adament.def_attack_hit_lv3 | Shield Slam | AnimBlend, OrientToVelocity, Stretch, mesh | rendered; round 6 changes apply (mesh, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_adament.def_attack_launch_lv1 | Shield Slam; Bleeding | mesh | rendered; round 6 changes apply (mesh); not compared with the real client |
| ability_skill_table_k.p_skill_adament.def_attack_launch_lv2 | Shield Slam | mesh | rendered; round 6 changes apply (mesh); not compared with the real client |
| ability_skill_table_k.p_skill_adament.def_attack_launch_lv3 | Shield Slam | mesh | rendered; round 6 changes apply (mesh); not compared with the real client |
| ability_skill_table_k.p_skill_adament.provoke_form | Mocking Howl | AnimBlend, Stretch, mesh | rendered; round 6 changes apply (mesh, Stretch, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_adament.provoke_launch | Mocking Howl | OrientToVelocity, material | rendered; round 6 changes apply (OrientToVelocity); not compared with the real client |
| ability_skill_table_k.p_skill_adament.deffence_hit | Bull Rush | AnimBlend | rendered; round 6 changes apply (AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_adament.deffence_launch | Bull Rush | mesh | rendered; round 6 changes apply (mesh); not compared with the real client |
| ability_skill_table_k.p_skill_adament.lifeup_launch | Refreshment | FocusAngle, OrientToVelocity, Stretch | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity); not compared with the real client |
| ability_skill_table_k.p_skill_adament.strong_resilient_launch | Toughen | AnimBlend, FocusAngle, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_adament.cure_shout_launch | Revitalizing Cheer | AnimBlend, FocusAngle, mesh | rendered; round 6 changes apply (FocusAngle, mesh, AnimBlend); not compared with the real client |
| ability_skill_table.blackmagic.black0009_form | Boastful Roar; Scratch; Treasure Guardian's Shout | AnimBlend, OrientToVelocity, Stretch | rendered; round 6 changes apply (Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| ability_skill_table.white.white0001_form | Boastful Roar; Treasure Guardian's Shout |  | rendered; not compared with the real client |
| ability_skill_table.white.white0001_hit | Boastful Roar; Treasure Guardian's Shout | Stretch, material | lace look done (round 8); the real rings are 3D oriented, camera facing here (orientation STILL WRONG) |
| ability_skill_table_k.p_skill_adament.winner_shout_hit | Boastful Roar; Imprison | AnimBlend, FocusAngle, Stretch | rendered; round 6 changes apply (FocusAngle, Stretch, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_adament.winner_shout_launch | Boastful Roar; Imprison | Stretch, decal, material | rendered; round 6 changes apply (Stretch); not compared with the real client |
| ability_skill_table_k.p_skill_adament.hammer_hit | Ollo's Hammer | AnimBlend, FocusAngle, Stretch, decal, material, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_adament.hammer_launch | Ollo's Hammer | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_adament.hammer_proj | Ollo's Hammer | AnimBlend, Stretch, material, mesh | compared with the real client (cube outlines match) |
| ability_skill_table_k.p_skill_will.masic_off_hit | Leech | FocusAngle, mesh | rendered; round 6 changes apply (FocusAngle, mesh); not compared with the real client |
| ability_skill_table_k.p_skill_will.masic_off_launch | Leech | Stretch, decal, material | rendered; round 6 changes apply (Stretch); not compared with the real client |
| ability_skill_table_k.p_skill_will.teleport_launch | Teleportation | AnimBlend, FocusAngle, OrientToVelocity, Stretch, decal, material, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_will.masic_ex_launch_level1 | Vicious Implosion | OrientToVelocity, Stretch, decal, material | rendered; round 6 changes apply (Stretch, OrientToVelocity); not compared with the real client |
| ability_skill_table_k.p_skill_will.masic_ex_launch_level2 | Vicious Implosion | AnimBlend, FocusAngle, OrientToVelocity, Stretch, decal, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_will.masic_ex_launch_level3 | Vicious Implosion | AnimBlend, FocusAngle, OrientToVelocity, Stretch, decal, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| x2_effects.mon_effects.gargoyle_leapattack_hit2 | Vicious Implosion | Stretch | rendered; round 6 changes apply (Stretch); not compared with the real client |
| ability_skill_table_k.p_skill_will.spell_shield_form | Protective Wings | AnimBlend, FocusAngle, Stretch | rendered; round 6 changes apply (FocusAngle, Stretch, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_will.spell_shield_form_end | Protective Wings | AnimBlend, Stretch | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_will.outofcontrolbattle_launch | Liberation | AnimBlend, Facing=Velocity, FocusAngle, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (FocusAngle, Facing=Velocity, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_will.strong_will_launch | Shrug It Off | FocusAngle, OrientToVelocity, Stretch, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch, OrientToVelocity); not compared with the real client |
| ability_skill_table_k.p_skill_will.vital_shield_launch | Conversion Shield | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_will.spirit_form | Meditate | material | rendered; not compared with the real client |
| ability_skill_table_k.p_skill_will.spirit_launch | Meditate | AnimBlend, FocusAngle, Stretch | rendered; round 6 changes apply (FocusAngle, Stretch, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_will.life_aura_form | Health Lift | FocusAngle, decal, material | rendered; round 6 changes apply (FocusAngle); not compared with the real client |
| ability_skill_table_k.p_skill_will.life_aura_form2 | Health Lift | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_will.life_aura_form2_end | Health Lift |  | rendered; not compared with the real client |
| ability_skill_table_k.p_skill_will.life_aura_throw | Health Lift |  | rendered; not compared with the real client |
| ability_skill_table_k.p_skill_will.sprit_charge_launch | Thwart | Facing=Velocity, FocusAngle, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (FocusAngle, Facing=Velocity, Stretch, OrientToVelocity); not compared with the real client |
| ability_skill_table_k.p_skill_will.haje_hammer_hit | Comet's Boon | AnimBlend, Stretch, decal, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_will.haje_hammer_launch | Comet's Boon | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_will.haje_hammer_proj | Comet's Boon | AnimBlend, Stretch | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_will.haje_hammer_proj_ground | Comet's Boon | AnimBlend, decal, material | rendered; round 6 changes apply (AnimBlend); not compared with the real client |
| abillity_skill_table_m.death.hell_spears_hit | Hell Spear |  | rendered; not compared with the real client |
| abillity_skill_table_m.death.hell_spears_range | Hell Spear | FocusAngle, OrientToVelocity, Stretch, decal, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.death.psychokinesis_form | Crippling Mire | FocusAngle, Stretch | rendered; round 6 changes apply (FocusAngle, Stretch); not compared with the real client |
| abillity_skill_table_m.death.psychokinesis_launch | Crippling Mire | mesh | rendered; round 6 changes apply (mesh); not compared with the real client |
| abillity_skill_table_m.death.psychokinesis_launch_form | Crippling Mire | Facing=Velocity, Stretch, material, mesh | rendered; round 6 changes apply (Facing=Velocity, mesh, Stretch); not compared with the real client |
| abillity_skill_table_m.death.psychokinesis_launch_form2 | Crippling Mire | FocusAngle, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch); not compared with the real client |
| abillity_skill_table_m.death.psychokinesis_launch_form_end | Crippling Mire | Facing=Velocity, material, mesh | rendered; round 6 changes apply (Facing=Velocity, mesh); not compared with the real client |
| abillity_skill_table_m.death.psychokinesis_launch_form_start | Crippling Mire | Facing=Velocity, material, mesh | rendered; round 6 changes apply (Facing=Velocity, mesh); not compared with the real client |
| abillity_skill_table_m.death.ghost_summon_launch | Summon Wraith; (Q803:눈물)_풀려난뱀_Reina | OrientToVelocity, Stretch, decal, material | round 10: the red smoke was on a 50 m circle (`RotateLength` without speed or axis): fixed; the real cloud is denser and brighter than ours (docs/aa-fx.md, round 10) |
| abillity_skill_table_m.death.revenge_armor_launch | Retribution | AnimBlend, FocusAngle, OrientToVelocity, Stretch | compared with the real client (matches; the white dotted columns are in the real one too) |
| abillity_skill_table_m.death.wide_silence_launch | Stillness; Area Silence | AnimBlend, Facing=Velocity, FocusAngle, Stretch, decal, material | rendered; round 6 changes apply (FocusAngle, Facing=Velocity, Stretch, AnimBlend); not compared with the real client |
| abillity_skill_table_m.death.crow_summon_throw | Summon Crows | Stretch, material | rendered; round 6 changes apply (Stretch); not compared with the real client |
| abillity_skill_table_m.death.life_absorb_form | Absorb Lifeforce | FocusAngle, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch); not compared with the real client |
| abillity_skill_table_m.death.life_absorb_launch | Absorb Lifeforce | mesh | rendered; round 6 changes apply (mesh); not compared with the real client |
| abillity_skill_table_m.death.life_absorb_launch_form | Absorb Lifeforce | FocusAngle, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch); not compared with the real client |
| abillity_skill_table_m.death.unfinished_spell_form | Urgency | AnimBlend, FocusAngle, OrientToVelocity, Stretch, decal, material, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.death.unfinished_spell_form2 | Urgency | AnimBlend, Stretch | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| abillity_skill_table_m.death.unfinished_spell_launch | Urgency | FocusAngle, OrientToVelocity, Stretch, decal, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch, OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.death.spell_move_form | Telekinesis; Doll's Curse | OrientToVelocity, Stretch | round 10: compared with the real client at 8 m: same flare, smaller halo (no bloom in the example) |
| abillity_skill_table_m.death.spell_move_launch | Telekinesis; Doll's Curse | mesh | round 10: stretches with the beam anchor (`--beam`) |
| abillity_skill_table_m.death.spell_move_launch_form | Telekinesis; Doll's Curse |  | round 10: compared with the real client: a pink flare, ours has the smaller halo |
| abillity_skill_table_m.magic.vital_absorb_form | Mana Force | FocusAngle, Stretch, material | round 10: compared with the real client: the violet star matches in shape, brighter and larger halo there |
| abillity_skill_table_m.magic.vital_absorb_launch | Mana Force | mesh | round 10: stretches with the beam anchor (`--beam`) |
| abillity_skill_table_m.death.spellblade_hit | Mana Stars | AnimBlend, Stretch | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| abillity_skill_table_m.death.spellblade_proj | Mana Stars | AnimBlend, Stretch, mesh | rendered; round 6 changes apply (mesh, Stretch, AnimBlend); not compared with the real client |
| abillity_skill_table_m.death.spellblade_throw | Mana Stars | mesh | rendered; round 6 changes apply (mesh); not compared with the real client |
| abillity_skill_table_m.wild.flowup_launch | Float | FocusAngle, Stretch | rendered; round 6 changes apply (FocusAngle, Stretch); not compared with the real client |
| abillity_skill_table_m.wild.pursuer_launch | Intensity | AnimBlend, FocusAngle, OrientToVelocity, Stretch, decal, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.wild.hawk_launch | Double Recurve | FocusAngle, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch); not compared with the real client |
| ability_skill_table_k.p_skill_vocation.bomb_shot_form | Concussive Arrow | OrientToVelocity, Stretch, mesh | rendered; round 6 changes apply (mesh, Stretch, OrientToVelocity); not compared with the real client |
| ability_skill_table_k.p_skill_vocation.bomb_shot_hit | Concussive Arrow | FocusAngle, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity); not compared with the real client |
| ability_skill_table_k.p_skill_vocation.bomb_shot_proj | Concussive Arrow | AnimBlend, Stretch, mesh | rendered; round 6 changes apply (mesh, Stretch, AnimBlend); not compared with the real client |
| abillity_skill_table_m.wild.hold_steps_throw | Snare | Stretch, decal | rendered; round 6 changes apply (Stretch); not compared with the real client |
| abillity_skill_table_m.wild.long_distance_shot_form | Missile Rain; Treasure Guardian's Buckshot Arrows | AnimBlend | rendered; round 6 changes apply (AnimBlend); not compared with the real client |
| abillity_skill_table_m.wild.long_distance_shot_form2 | Missile Rain; Treasure Guardian's Buckshot Arrows | AnimBlend, Stretch | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| abillity_skill_table_m.wild.long_distance_shot_hit | Missile Rain | AnimBlend, FocusAngle, Stretch, decal, material | rendered; round 6 changes apply (FocusAngle, Stretch, AnimBlend); not compared with the real client |
| abillity_skill_table_m.wild.long_distance_shot_launch | Missile Rain |  | not found in the libraries (not rendered) |
| abillity_skill_table_m.wild.long_distance_shot_proj | Missile Rain | AnimBlend, OrientToVelocity, Stretch, mesh | rendered; round 6 changes apply (mesh, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.wild.blood_shot_hit | Piercing Shot | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| abillity_skill_table_m.wild.blood_shot_proj | Piercing Shot | AnimBlend, Stretch, mesh | rendered; round 6 changes apply (mesh, Stretch, AnimBlend); not compared with the real client |
| abillity_skill_table_m.wild.blood_shot_throw | Piercing Shot | AnimBlend, OrientToVelocity | rendered; round 6 changes apply (OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.wild.bone_wall_a_throw | Boneyard | Stretch, mesh | rendered; round 6 changes apply (mesh, Stretch); not compared with the real client |
| abillity_skill_table_m.wild.bone_wall_proj_end | Boneyard |  | rendered; not compared with the real client |
| abillity_skill_table_m.wild.continue_shot_hit | Endless Arrows | material, mesh | rendered; round 6 changes apply (mesh); not compared with the real client |
| abillity_skill_table_m.wild.continue_shot_proj | Endless Arrows; Treasure Guardian's Arrow | Stretch, mesh | rendered; round 6 changes apply (mesh, Stretch); not compared with the real client |
| abillity_skill_table_m.wild.standhold_shot_form | Deadeye |  | rendered; not compared with the real client |
| abillity_skill_table_m.wild.standhold_shot_launch | Deadeye | AnimBlend, Stretch | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| abillity_skill_table_m.wild.shock_form2 | Charged Bolt; Freezing Arrow; Ice Blade; Pushing Arrow | mesh | rendered; round 6 changes apply (mesh); not compared with the real client |
| abillity_skill_table_m.wild.shock_hit | Charged Bolt; Pushing Arrow | AnimBlend, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.wild.shock_hit_level2 | Charged Bolt; Treasure Guardian's Arrow | AnimBlend, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.wild.shock_hit_level3 | Charged Bolt | AnimBlend, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.wild.shock_proj | Charged Bolt; Pushing Arrow; Treasure Guardian's Arrow | AnimBlend, Stretch, material, mesh | rendered; round 6 changes apply (mesh, Stretch, AnimBlend); not compared with the real client |
| abillity_skill_table_m.wild.shock_throw | Charged Bolt; Freezing Arrow; Ice Blade; Pushing Arrow | AnimBlend, OrientToVelocity | rendered; round 6 changes apply (OrientToVelocity, AnimBlend); not compared with the real client |
| ability_skill_table.mental.mt_spirit0002_form01 | Freezing Earth | OrientToVelocity, Stretch | rendered; round 6 changes apply (Stretch, OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.magic.ice_piece_hit | Freezing Earth | FocusAngle, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch); not compared with the real client |
| abillity_skill_table_m.magic.ice_piece_launch | Freezing Earth | Stretch, decal, material | rendered; round 6 changes apply (Stretch); not compared with the real client |
| abillity_skill_table_m.magic.mana_shield_form | Insulating Lens | material | rendered; not compared with the real client |
| abillity_skill_table_m.magic.mana_shield_form2 | Insulating Lens | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| abillity_skill_table_m.magic.mana_shield_launch | Insulating Lens | FocusAngle, Stretch, decal, material, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch); not compared with the real client |
| abillity_skill_table_m.magic.mana_shield_throw | Insulating Lens | OrientToVelocity, Stretch, material | rendered; round 6 changes apply (Stretch, OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.magic.meteo_form | Meteor Strike | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| abillity_skill_table_m.magic.meteo_form2 | Meteor Strike | AnimBlend, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.magic.meteo_form3 | Meteor Strike | AnimBlend, FocusAngle, Stretch, decal, material | rendered; round 6 changes apply (FocusAngle, Stretch, AnimBlend); not compared with the real client |
| abillity_skill_table_m.magic.meteo_hit | Meteor Strike | AnimBlend, Facing=Velocity, FocusAngle, OrientToVelocity, Stretch, decal, material, mesh | rendered; round 6 changes apply (FocusAngle, Facing=Velocity, mesh, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.magic.meteo_throw | Meteor Strike |  | rendered; not compared with the real client |
| abillity_skill_table_m.magic.icearrow_form | Freezing Arrow | Stretch, material, mesh | rendered; round 6 changes apply (mesh, Stretch); not compared with the real client |
| abillity_skill_table_m.magic.icearrow_form2 | Freezing Arrow | AnimBlend, FocusAngle, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, AnimBlend); not compared with the real client |
| abillity_skill_table_m.magic.icearrow_hit | Freezing Arrow | AnimBlend, OrientToVelocity, Stretch, material, mesh | rendered; round 6 changes apply (mesh, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.magic.icearrow_hit_level2 | Freezing Arrow | AnimBlend, FocusAngle, OrientToVelocity, Stretch, material, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.magic.icearrow_hit_level3 | Freezing Arrow | AnimBlend, FocusAngle, OrientToVelocity, Stretch, material, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.magic.icearrow_proj | Freezing Arrow; Ice Blade | Stretch, material, mesh | rendered; round 6 changes apply (mesh, Stretch); not compared with the real client |
| abillity_skill_table_m.magic.icearrow_proj_level2 | Freezing Arrow | Stretch, material, mesh | rendered; round 6 changes apply (mesh, Stretch); not compared with the real client |
| abillity_skill_table_m.magic.icearrow_proj_level3 | Freezing Arrow | Stretch, material, mesh | rendered; round 6 changes apply (mesh, Stretch); not compared with the real client |
| abillity_skill_table_m.magic.icearrow_throw | Freezing Arrow | Stretch | rendered; round 6 changes apply (Stretch); not compared with the real client |
| abillity_skill_table_m.magic.lightning_magic_form | Arc Lightning; Skrull's Chain Lightning | AnimBlend, Stretch | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| abillity_skill_table_m.magic.lightning_magic_hit | Arc Lightning; Skrull's Chain Lightning | AnimBlend, FocusAngle, Stretch, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch, AnimBlend); not compared with the real client |
| abillity_skill_table_m.magic.lightning_magic_launch | Arc Lightning | mesh | rendered; round 6 changes apply (mesh); not compared with the real client |
| abillity_skill_table_m.magic.lightning_magic_throw | Arc Lightning; Skrull's Chain Lightning | AnimBlend, FocusAngle, Stretch | rendered; round 6 changes apply (FocusAngle, Stretch, AnimBlend); not compared with the real client |
| abillity_skill_table_m.magic.fireball_form | Flamebolt; Lighting Fire; (Level Animation) Repeat Magic Casting (Flame Orb) | AnimBlend, FocusAngle, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.magic.fireball_form2 | Flamebolt; (Level Animation) Repeat Magic Casting (Flame Orb) |  | rendered; not compared with the real client |
| abillity_skill_table_m.magic.fireball_hit | Flamebolt; (Level Animation) Repeat Magic Casting (Flame Orb) | AnimBlend, FocusAngle, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.magic.fireball_hit_level2 | Flamebolt | AnimBlend, FocusAngle, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.magic.fireball_hit_level3 | Flamebolt | AnimBlend, FocusAngle, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.magic.fireball_proj | Flamebolt; (Level Animation) Repeat Magic Casting (Flame Orb) | AnimBlend, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.magic.fireball_proj_level2 | Flamebolt | AnimBlend, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.magic.fireball_proj_level3 | Flamebolt | AnimBlend, FocusAngle, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.magic.fireball_throw | Flamebolt; Lighting Fire; (Level Animation) Repeat Magic Casting (Flame Orb); Fi | AnimBlend, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.magic.ice_wall_form | Frigid Tracks | Stretch, material, mesh | rendered; round 6 changes apply (mesh, Stretch); not compared with the real client |
| abillity_skill_table_m.magic.ice_wall_launch | Frigid Tracks | Stretch, decal, material, mesh | rendered; round 6 changes apply (mesh, Stretch); not compared with the real client |
| abillity_skill_table_m.magic.ice_wall_throw | Frigid Tracks | mesh | rendered; round 6 changes apply (mesh); not compared with the real client |
| abillity_skill_table_m.magic.fire_rain_form | Searing Rain | AnimBlend, FocusAngle, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, AnimBlend); not compared with the real client |
| abillity_skill_table_m.magic.fire_rain_form2 | Searing Rain | AnimBlend, FocusAngle, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, AnimBlend); not compared with the real client |
| abillity_skill_table_m.magic.fire_rain_form3 | Searing Rain | decal, material | rendered; not compared with the real client |
| abillity_skill_table_m.magic.fire_rain_hit | Searing Rain | AnimBlend, FocusAngle, OrientToVelocity, Stretch, decal, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.magic.fire_rain_launch | Searing Rain | AnimBlend, FocusAngle, Stretch | rendered; round 6 changes apply (FocusAngle, Stretch, AnimBlend); not compared with the real client |
| abillity_skill_table_m.system_skill.decal_target_radius_5m | Searing Rain; Skewer | decal, material | rendered; not compared with the real client |
| abillity_skill_table_m.magic.lighting_serial_magic_hit | Chain Lightning; Skrull's Chain Lightning | AnimBlend, Stretch, mesh | rendered; round 6 changes apply (mesh, Stretch, AnimBlend); not compared with the real client |
| abillity_skill_table_m.magic.lighting_serial_magic_launch | Chain Lightning; Skrull's Chain Lightning | mesh | rendered; round 6 changes apply (mesh); not compared with the real client |
| abillity_skill_table_m.magic.lighting_serial_magic_throw | Chain Lightning; Father's Hammer | AnimBlend, Stretch | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| abillity_skill_table_m.magic.magic_reinforce_form | Magic Circle | material | rendered; not compared with the real client |
| abillity_skill_table_m.magic.magic_reinforce_form2 | Magic Circle | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| abillity_skill_table_m.magic.magic_reinforce_hit | Magic Circle | AnimBlend, FocusAngle, material | rendered; round 6 changes apply (FocusAngle, AnimBlend); not compared with the real client |
| abillity_skill_table_m.magic.magic_reinforce_launch | Magic Circle | FocusAngle, OrientToVelocity, Stretch, decal, material, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch, OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.magic.magic_reinforce_throw | Magic Circle | OrientToVelocity, material | rendered; round 6 changes apply (OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.magic.firewall_launch | Flame Barrier | AnimBlend, FocusAngle, Stretch | compared with the real client: the real column is about twice as tall (STILL WRONG, cause unknown) |
| ability_skill_table_k.p_skill_vocation.hide_launch | Stealth | Stretch | rendered; round 6 changes apply (Stretch); not compared with the real client |
| ability_skill_table_k.p_skill_vocation.p_skill_vocation_speed | Freerunner | OrientToVelocity | rendered; round 6 changes apply (OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.wild.poison_shot_hit | Toxic Shot | AnimBlend, OrientToVelocity, Stretch, material, mesh | rendered; round 6 changes apply (mesh, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.wild.poison_shot_proj | Toxic Shot; Stomach Acid Spit | AnimBlend, Stretch, material, mesh | rendered; round 6 changes apply (mesh, Stretch, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_vocation.p_skill_dark_attack_hit | Shadowsmite | AnimBlend, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_vocation.vaulting_jum_attack1_hit | Overwhelm | OrientToVelocity, Stretch, decal, material | rendered; round 6 changes apply (Stretch, OrientToVelocity); not compared with the real client |
| ability_skill_table_k.p_skill_vocation.vaulting_smoke_launch | Overwhelm |  | rendered; not compared with the real client |
| ability_skill_table_k.p_skill_vocation.nanta_hit | Wallop; Rapid Strike | FocusAngle, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity); not compared with the real client |
| ability_skill_table_k.p_skill_vocation.nanta_swoosh | Wallop; Rapid Strike | OrientToVelocity, material | rendered; round 6 changes apply (OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.wild.back_step_launch | Drop Back | Stretch | rendered; round 6 changes apply (Stretch); not compared with the real client |
| abillity_skill_table_m.wild.back_step_launch2 | Drop Back | Facing=Velocity, material | rendered; round 6 changes apply (Facing=Velocity); not compared with the real client |
| x2_effects.doodad_effect.qeust_teleport_launch | Shadow Step | AnimBlend, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| x2_effects.doodad_effect.qeust_teleport_launch2 | Shadow Step | OrientToVelocity, Stretch, material | rendered; round 6 changes apply (Stretch, OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.wild.pursuer_mark_hit | Stalker's Mark | AnimBlend, OrientToVelocity, Stretch, material, mesh | rendered; round 6 changes apply (mesh, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.wild.pursuer_mark_proj | Stalker's Mark | AnimBlend, Stretch, mesh | rendered; round 6 changes apply (mesh, Stretch, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_vocation.beatdown_hit | Pin Down | AnimBlend, FocusAngle, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_romance.song_of_speed_form | [Perform] Quickstep | FocusAngle, decal, material | rendered; round 6 changes apply (FocusAngle); not compared with the real client |
| ability_skill_table_k.p_skill_romance.song_of_speed_launch | [Perform] Quickstep | FocusAngle, material, mesh | rendered; round 6 changes apply (FocusAngle, mesh); not compared with the real client |
| ability_skill_table_k.p_skill_romance.song_of_res_form | [Perform] Ode to Recovery | FocusAngle, decal, material | rendered; round 6 changes apply (FocusAngle); not compared with the real client |
| ability_skill_table_k.p_skill_romance.song_of_res_launch | [Perform] Ode to Recovery | FocusAngle, OrientToVelocity, Stretch, decal, material, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch, OrientToVelocity); not compared with the real client |
| ability_skill_table_k.p_skill_romance.romance_form | [Perform] Bloody Chantey; Hummingbird Ditty; [Perform] Bulwark Ballad; Machine T | FocusAngle, material | rendered; round 6 changes apply (FocusAngle); not compared with the real client |
| ability_skill_table_k.p_skill_romance.song_of_battle_launch | [Perform] Bloody Chantey | AnimBlend, FocusAngle, OrientToVelocity, Stretch, decal, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_romance.romance_form_sound | Hummingbird Ditty; Machine Trap Skill |  | rendered; not compared with the real client |
| ability_skill_table_k.p_skill_romance.sage_of_poem_launch | Hummingbird Ditty | FocusAngle, OrientToVelocity, decal, material | rendered; round 6 changes apply (FocusAngle, OrientToVelocity); not compared with the real client |
| ability_skill_table_k.p_skill_romance.me_sg_form | [Perform] Bulwark Ballad | FocusAngle, decal, material | rendered; round 6 changes apply (FocusAngle); not compared with the real client |
| ability_skill_table_k.p_skill_romance.me_sg_launch | [Perform] Bulwark Ballad | FocusAngle, OrientToVelocity, Stretch | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity); not compared with the real client |
| ability_skill_table_k.p_skill_romance.song_of_charming_form | Startling Strain | FocusAngle, material | rendered; round 6 changes apply (FocusAngle); not compared with the real client |
| ability_skill_table_k.p_skill_romance.song_of_charming_hit | Startling Strain | FocusAngle, material | rendered; round 6 changes apply (FocusAngle); not compared with the real client |
| ability_skill_table_k.p_skill_romance.song_of_charming_proj | Startling Strain | OrientToVelocity | rendered; round 6 changes apply (OrientToVelocity); not compared with the real client |
| ability_skill_table_k.p_skill_romance.song_of_charming_throw | Startling Strain | FocusAngle, material | rendered; round 6 changes apply (FocusAngle); not compared with the real client |
| ability_skill_table_k.p_skill_romance.song_of_feathers_launch2 | Startling Strain | mesh | rendered; round 6 changes apply (mesh); not compared with the real client |
| x2_sounds.particles.empty_continuous | Startling Strain |  | not found in the libraries (not rendered) |
| ability_skill_table_k.p_skill_romance.dissonance_form | Dissonant | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_romance.dissonance_hit | Dissonant | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_romance.dissonance_hit_level2 | Dissonant | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_romance.dissonance_hit_level3 | Dissonant | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_romance.dissonance_launch | Dissonant | AnimBlend, FocusAngle, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_romance.dissonance_proj | Dissonant | AnimBlend, Stretch, material, mesh | rendered; round 6 changes apply (mesh, Stretch, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_love.cure_spring_form | Rhythmic Renewal | FocusAngle, OrientToVelocity, Stretch | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity); not compared with the real client |
| x2_sounds.particles.romance_008_launch | Rhythmic Renewal |  | not found in the libraries (not rendered) |
| ability_skill_table_k.p_skill_romance.morning_bugle_launch | Alarm Call | FocusAngle, OrientToVelocity, material | rendered; round 6 changes apply (FocusAngle, OrientToVelocity); not compared with the real client |
| ability_skill_table_k.p_skill_romance.blade_sound_form | Critical Discord | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_romance.blade_sound_hit | Critical Discord | AnimBlend, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_romance.blade_sound_hit_level2 | Critical Discord | AnimBlend, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_romance.blade_sound_hit_level3 | Critical Discord | AnimBlend, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_romance.blade_sound_proj | Critical Discord | AnimBlend, material, mesh | rendered; round 6 changes apply (mesh, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_romance.blade_sound_throw | Critical Discord | AnimBlend, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| ability_skill_table.element.el_fire0001_form | Healing Hymn | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_love.note_heal_02_hit | Healing Hymn | AnimBlend, FocusAngle, OrientToVelocity, material | rendered; round 6 changes apply (FocusAngle, OrientToVelocity, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_love.note_heal_02_proj | Healing Hymn | OrientToVelocity, material | rendered; round 6 changes apply (OrientToVelocity); not compared with the real client |
| ability_skill_table_k.p_skill_love.note_heal_04_hit | Healing Hymn | AnimBlend, FocusAngle, OrientToVelocity, material | rendered; round 6 changes apply (FocusAngle, OrientToVelocity, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_love.note_heal_04_proj | Healing Hymn | OrientToVelocity, material | rendered; round 6 changes apply (OrientToVelocity); not compared with the real client |
| ability_skill_table_k.p_skill_love.note_heal_08_hit | Healing Hymn | AnimBlend, FocusAngle, OrientToVelocity, material | rendered; round 6 changes apply (FocusAngle, OrientToVelocity, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_love.note_heal_08_proj | Healing Hymn | OrientToVelocity, material | rendered; round 6 changes apply (OrientToVelocity); not compared with the real client |
| ability_skill_table_k.p_skill_love.note_heal_16_hit | Healing Hymn | AnimBlend, FocusAngle, OrientToVelocity, material | rendered; round 6 changes apply (FocusAngle, OrientToVelocity, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_love.note_heal_16_proj | Healing Hymn | OrientToVelocity, material | rendered; round 6 changes apply (OrientToVelocity); not compared with the real client |
| x2_sounds.particles.empty_oneshot | Healing Hymn |  | not found in the libraries (not rendered) |
| ability_skill_table_k.p_skill_love.hill_attack_form | Antithesis | AnimBlend, FocusAngle, OrientToVelocity, Stretch | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_love.hill_attack_form_hand | Antithesis | Stretch | rendered; round 6 changes apply (Stretch); not compared with the real client |
| ability_skill_table_k.p_skill_love.hill_attack_hit | Antithesis | AnimBlend, FocusAngle, OrientToVelocity, Stretch, decal, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_love.hill_attack_launch | Antithesis | AnimBlend, FocusAngle, OrientToVelocity, Stretch, decal, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_love.reform_form | Revive | FocusAngle, OrientToVelocity, Stretch, material, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch, OrientToVelocity); not compared with the real client |
| ability_skill_table_k.p_skill_love.reform_form_r_hand | Revive | Stretch, material | rendered; round 6 changes apply (Stretch); not compared with the real client |
| ability_skill_table_k.p_skill_love.reform_throw | Revive | Stretch | rendered; round 6 changes apply (Stretch); not compared with the real client |
| ability_skill_table_k.p_skill_love.resurrection_form | Revive | AnimBlend, FocusAngle, Stretch | rendered; round 6 changes apply (FocusAngle, Stretch, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_love.resurrection_launch | Revive | FocusAngle, OrientToVelocity, Stretch, decal, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity); not compared with the real client |
| ability_skill_table_k.p_skill_love.overhill_form | Resurgence | FocusAngle, OrientToVelocity, Stretch, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch, OrientToVelocity); not compared with the real client |
| ability_skill_table_k.p_skill_love.overhill_hit | Resurgence | AnimBlend, FocusAngle, OrientToVelocity, Stretch, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_love.overhill_launch | Resurgence | FocusAngle, OrientToVelocity, Stretch, decal, material, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch, OrientToVelocity); not compared with the real client |
| ability_skill_table_k.p_skill_love.partyhill_form | Mend | AnimBlend, Stretch | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_love.partyhill_form_end | Mend | AnimBlend, Stretch | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_love.partyhill_form_sound | Mend |  | rendered; not compared with the real client |
| ability_skill_table_k.p_skill_love.partyhill_launch | Mend | Facing=Velocity, FocusAngle, material | rendered; round 6 changes apply (FocusAngle, Facing=Velocity); not compared with the real client |
| ability_skill_table_k.p_skill_love.partyhill_launch_level2 | Mend | Facing=Velocity, FocusAngle, material | rendered; round 6 changes apply (FocusAngle, Facing=Velocity); not compared with the real client |
| ability_skill_table_k.p_skill_love.partyhill_launch_level3 | Mend | Facing=Velocity, FocusAngle, material | rendered; round 6 changes apply (FocusAngle, Facing=Velocity); not compared with the real client |
| x2_sounds.particles.love_evening | Twilight |  | not found in the libraries (not rendered) |
| x2_sounds.particles.love_evening_end | Twilight |  | not found in the libraries (not rendered) |
| ability_skill_table_k.p_skill_love.blessandcurse_form | Mirror Light | AnimBlend, FocusAngle, Stretch | rendered; round 6 changes apply (FocusAngle, Stretch, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_love.blessandcurse_launch | Mirror Light | AnimBlend, Facing=Velocity, FocusAngle, OrientToVelocity, Stretch | rendered; round 6 changes apply (FocusAngle, Facing=Velocity, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_love.judge_spear_form | Skewer | AnimBlend, OrientToVelocity, Stretch, mesh | rendered; round 6 changes apply (mesh, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_love.judge_spear_form_end | Skewer | AnimBlend, Stretch, decal, material, mesh | rendered; round 6 changes apply (mesh, Stretch, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_love.judge_spear_hit | Skewer | AnimBlend, FocusAngle, Stretch, decal, material, mesh | compared with the real client (matches) |
| ability_skill_table_k.p_skill_love.judge_spear_proj | Skewer | Facing=Velocity, FocusAngle, decal, material | rendered; round 6 changes apply (FocusAngle, Facing=Velocity); not compared with the real client |
| ability_skill_table_k.p_skill_love.continue_recover_hit | Fervent Healing | AnimBlend, OrientToVelocity, mesh | rendered; round 6 changes apply (mesh, OrientToVelocity, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_love.continue_recover_proj | Fervent Healing | AnimBlend, OrientToVelocity, mesh | rendered; round 6 changes apply (mesh, OrientToVelocity, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_love.continue_recover_throw | Fervent Healing | AnimBlend, OrientToVelocity | rendered; round 6 changes apply (OrientToVelocity, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_love.god_bless_form | Aranzeb's Boon | AnimBlend, material | rendered; round 6 changes apply (AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_love.god_bless_launch | Aranzeb's Boon | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_love.vital_add_launch | Infuse | AnimBlend, FocusAngle, OrientToVelocity, Stretch, decal, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_love.recoverry_seed_launch | Renewal | FocusAngle, OrientToVelocity, Stretch, decal, material, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch, OrientToVelocity); not compared with the real client |
| ability_skill_table_k.p_skill_love.recovery_seed_form | Renewal | FocusAngle, Stretch, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch); not compared with the real client |
| x2_effects.sword_trail_test2 | Melee Attack | FocusAngle, material | rendered; round 6 changes apply (FocusAngle); not compared with the real client |
| abillity_skill_table_m.system_skill.return_skill_form | Recall | FocusAngle, OrientToVelocity, material | rendered; round 6 changes apply (FocusAngle, OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.system_skill.return_skill_launch | Recall | Facing=Velocity, FocusAngle, material | rendered; round 6 changes apply (FocusAngle, Facing=Velocity); not compared with the real client |
| x2_effects.e_effect.flower_throw | /flowers | FocusAngle | rendered; round 6 changes apply (FocusAngle); not compared with the real client |
| x2_effects.p_effect.st1000b | Shoot Arrow; Bow Attack | mesh | rendered; round 6 changes apply (mesh); not compared with the real client |
| x2_effects.p_effect.st1000c | Shoot Arrow; Bow Attack | OrientToVelocity, Stretch, material, mesh | rendered; round 6 changes apply (mesh, Stretch, OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.system_skill.reading_roll_form | Rider's Escape | AnimBlend, FocusAngle, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, AnimBlend); not compared with the real client |
| abillity_skill_table_m.system_skill.reading_roll_launch | Rider's Escape | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| pino_effect.mon.fireball_launch | Shoot Fireworks; Flamebolt | AnimBlend, OrientToVelocity, Stretch | rendered; round 6 changes apply (Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_fireball_form | Shoot Fireworks; Flamebolt | AnimBlend, FocusAngle | rendered; round 6 changes apply (FocusAngle, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_fireball_hit | Shoot Fireworks; Flamebolt | AnimBlend, FocusAngle, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_fireball_proj | Shoot Fireworks; Flamebolt | AnimBlend, FocusAngle, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| ability_skill_table.firing.firing0001_hit | Shoot Arrow | Stretch, mesh | rendered; round 6 changes apply (mesh, Stretch); not compared with the real client |
| ability_skill_table.firing.firing0001_launch | Shoot Arrow | mesh | rendered; round 6 changes apply (mesh); not compared with the real client |
| x2_effects.mon_effects.wood_elemental_hit | Throw Branch; Scatter Dirt | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| x2_effects.mon_effects.wood_elemental_proj | Throw Branch; Scatter Dirt | OrientToVelocity, Stretch, material, mesh | rendered; round 6 changes apply (mesh, Stretch, OrientToVelocity); not compared with the real client |
| x2_effect_o.mon_effects.mon_death01_launch | Crack Whip; Fatal Wound | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_melee01_hit | Crack Whip; Fatal Wound | AnimBlend, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_water_wave_s | Splash Shock; Water Spray | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_poison_attack_hit | Dirty Spit; Spit Poison | AnimBlend, OrientToVelocity, Stretch | rendered; round 6 changes apply (Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_throw_poison_proj | Dirty Spit; Spit Poison | AnimBlend, FocusAngle, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_range02_hit | Throw Dagger | AnimBlend, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_throw_dagger_proj | Throw Dagger | mesh | rendered; round 6 changes apply (mesh); not compared with the real client |
| ability_skill_table.firing.firing0003_hit | Stomach Acid Spit | AnimBlend, Facing=Velocity, Stretch, mesh | rendered; round 6 changes apply (Facing=Velocity, mesh, Stretch, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_spineshield_launch | Thorn Shield | AnimBlend, FocusAngle, material, mesh | rendered; round 6 changes apply (FocusAngle, mesh, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_infection_launch | Infection | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_wide_blast_form | Area Energy Blast | AnimBlend, OrientToVelocity, Stretch, material, mesh | rendered; round 6 changes apply (mesh, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_wide_blast_hit | Area Energy Blast | AnimBlend, FocusAngle, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_wide_blast_launch | Area Energy Blast | AnimBlend, FocusAngle, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_strong_light_form | Intense Light | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_strong_light_hit | Intense Light | Stretch | rendered; round 6 changes apply (Stretch); not compared with the real client |
| x2_effect_o.mon_effects.mon_strong_light_launch | Intense Light | Stretch | rendered; round 6 changes apply (Stretch); not compared with the real client |
| x2_effect_o.mon_effects.mon_throw_star_proj | Intense Light | AnimBlend, FocusAngle, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_range01_hit | Throw Axe; Throw Harpoon | AnimBlend, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_throw_axe_proj | Throw Axe | mesh | rendered; round 6 changes apply (mesh); not compared with the real client |
| x2_effect_o.mon_effects.mon_poison_bubble_launch | Toxic Foam | Stretch, material | rendered; round 6 changes apply (Stretch); not compared with the real client |
| x2_effect_o.mon_effects.mon_poison_gas_form | Poison Mist; Venom Attack; Poison Spray | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_heal_form | Heal | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_heal_launch | Heal | Stretch | rendered; round 6 changes apply (Stretch); not compared with the real client |
| x2_effect_o.com.dust_stike_01a | Hammer Smash | FocusAngle | rendered; round 6 changes apply (FocusAngle); not compared with the real client |
| x2_effect_o.mon_effects.mon_hammer_hit | Hammer Smash | FocusAngle, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity); not compared with the real client |
| x2_effect_o.mon_effects.mon_poison_attack_hit_c | Venom Attack; Poison Spray | AnimBlend, Stretch | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_savage_hit | Bestial Leap | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_water_gun_hit | Watergun | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_water_gun_proj | Watergun | FocusAngle, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch); not compared with the real client |
| x2_effects.mon_effects.nmon_water_gun_throw | Watergun | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_shield_attack_hit | Shield Smash | OrientToVelocity, Stretch, material | rendered; round 6 changes apply (Stretch, OrientToVelocity); not compared with the real client |
| x2_effects.mon_effects.nmon_elemental_attack_form | Element Attack | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| x2_effects.mon_effects.nmon_elemental_attack_form2 | Element Attack |  | rendered; not compared with the real client |
| x2_effects.mon_effects.nmon_elemental_attack_hit | Element Attack | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| x2_effects.mon_effects.nmon_elemental_attack_proj | Element Attack | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| x2_effects.mon_effects.nmon_elemental_attack_throw | Element Attack |  | rendered; not compared with the real client |
| x2_effect_o.mon_effects.mon_blood_splay_hit | Blood Spray | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_regenerate_launch | Regeneration | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_death_energy_form | Death Energy | AnimBlend, FocusAngle | rendered; round 6 changes apply (FocusAngle, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_death_energy_hit | Death Energy; 사슴 기절시키기_Reina; Groundling Doll's Curse | AnimBlend, Stretch | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_death_energy_proj | Death Energy; 사슴 기절시키기_Reina; Groundling Doll's Curse | AnimBlend, FocusAngle, Stretch | rendered; round 6 changes apply (FocusAngle, Stretch, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_dark_01a_throw | Throw Harpoon | AnimBlend | rendered; round 6 changes apply (AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_harpoon_proj | Throw Harpoon | AnimBlend, FocusAngle, OrientToVelocity, Stretch, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| ability_skill_table.blackmagic.black0009_launch | Scratch |  | rendered; not compared with the real client |
| x2_effect_o.mon_effects.mon_confusion_launch | Chaos; Groundling Doll's Curse | AnimBlend, FocusAngle, OrientToVelocity, material | rendered; round 6 changes apply (FocusAngle, OrientToVelocity, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_absorber_form | Absorb Life | AnimBlend | rendered; round 6 changes apply (AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_absorber_launch | Absorb Life | mesh | rendered; round 6 changes apply (mesh); not compared with the real client |
| x2_sounds.particles.lute_playing_01 | (Level Animation) Play Lute |  | not found in the libraries (not rendered) |
| x2_effects.doodad_effect.quest_woodbox_destroy | Destroy Scarecrow; Mounted Combat Practice Bat's Self Destruct | OrientToVelocity, Stretch | rendered; round 6 changes apply (Stretch, OrientToVelocity); not compared with the real client |
| x2_effect_o.mon_effects.mon_throw_thorn_hit | Shoot Thorns | AnimBlend, FocusAngle, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_throw_thorn_proj | Shoot Thorns | AnimBlend, FocusAngle, Stretch, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_strong_thrust_hit | Strong Lunge | FocusAngle, OrientToVelocity, Stretch | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity); not compared with the real client |
| x2_effect_o.mon_effects.mon_strong_thrust_launch | Strong Lunge | AnimBlend, FocusAngle, Stretch, material, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch, AnimBlend); not compared with the real client |
| x2_effects.doodad_effect.quest_spidernet_throw_hit | Spin Web | OrientToVelocity, Stretch, material | rendered; round 6 changes apply (Stretch, OrientToVelocity); not compared with the real client |
| x2_effects.doodad_effect.quest_spidernet_throw_launch | Spin Web | OrientToVelocity, mesh | rendered; round 6 changes apply (mesh, OrientToVelocity); not compared with the real client |
| x2_effects.doodad_effect.quest_spidernet_throw_proj | Spin Web | OrientToVelocity, Stretch, material, mesh | rendered; round 6 changes apply (mesh, Stretch, OrientToVelocity); not compared with the real client |
| x2_effect_o.mon_effects.mon_groundshake_launch | Tremor | AnimBlend, FocusAngle, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_poison_thorn_hit | Stinger Attack | AnimBlend, FocusAngle, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_poison_thorn_launch | Stinger Attack | AnimBlend, FocusAngle, Stretch, material, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_throw_stone_hit | Throw Stone | AnimBlend, FocusAngle, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_throw_stone_proj | Throw Stone | AnimBlend, FocusAngle, Stretch, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_wedge_arrow_hit | Wedge Arrow | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_wedge_arrow_proj | Wedge Arrow | FocusAngle, Stretch, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch); not compared with the real client |
| x2_effect_o.mon_effects.mon_wail_launch | Wailing; Piercing Howl; Demoralized Shout | AnimBlend, Stretch, material | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_digestive_fluid_hit | Spit Stomach Acid | AnimBlend, FocusAngle, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_digestive_fluid_proj | Spit Stomach Acid | AnimBlend, FocusAngle, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_wildshock_hit | Shockwave | AnimBlend, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_strong_attack_hit | Strong Strike; Ogre Warrior's Slam | AnimBlend, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_windgun_hit | Blowpipe | AnimBlend, FocusAngle, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_windgun_proj | Blowpipe | AnimBlend, FocusAngle, OrientToVelocity, Stretch, material, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| abillity_skill_table_m.fight.slash | Strong Strike | FocusAngle, OrientToVelocity, Stretch | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.fight.slash_trail_level1 | Strong Strike | OrientToVelocity, material | rendered; round 6 changes apply (OrientToVelocity); not compared with the real client |
| x2_effects.e_effect.wind_twist_doodad | Wind Messenger | FocusAngle, mesh | rendered; round 6 changes apply (FocusAngle, mesh); not compared with the real client |
| ability_skill_table_k.p_skill_will.spell_attack_hit | Acid Attack | FocusAngle, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity); not compared with the real client |
| ability_skill_table_k.p_skill_will.spell_attack_launch | Acid Attack | mesh | rendered; round 6 changes apply (mesh); not compared with the real client |
| ability_skill_table.weapon.weapon0006_form | Trip |  | rendered; not compared with the real client |
| ability_skill_table.weapon.weapon0006_hit | Trip |  | rendered; not compared with the real client |
| ability_skill_table_k.p_skill_love.holy_proclamation_form | Victory Shout | material | rendered; not compared with the real client |
| ability_skill_table_k.p_skill_love.holy_proclamation_launch | Victory Shout | material | rendered; not compared with the real client |
| ability_skill_table_k.p_skill_will.vital_missile_from | Mana Thorn | AnimBlend, Stretch | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_will.vital_missile_hit | Mana Thorn | AnimBlend, FocusAngle, Stretch, decal, material | rendered; round 6 changes apply (FocusAngle, Stretch, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_will.vital_missile_proj | Mana Thorn |  | rendered; not compared with the real client |
| ability_skill_table_k.p_skill_will.vital_missile_throw | Mana Thorn | AnimBlend, Stretch | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_lithification_hit | Wave of Earth | AnimBlend, FocusAngle, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, AnimBlend); not compared with the real client |
| x2_effects_p.doodad.quest_hydra_spawn_launch | Very Rigid; Call Waves | FocusAngle, OrientToVelocity, Stretch, decal, material, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch, OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.fight.3count_attack_1 | Flurry Rank 1 | FocusAngle, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity); not compared with the real client |
| abillity_skill_table_m.fight.3count_attack_form | Flurry Rank 1 | FocusAngle, decal, material, mesh | rendered; round 6 changes apply (FocusAngle, mesh); not compared with the real client |
| abillity_skill_table_m.illusion.summon_hit | Ghost Wolf Summon | AnimBlend, FocusAngle, Stretch, decal, material, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch, AnimBlend); not compared with the real client |
| abillity_skill_table_m.illusion.summon_launch | Ghost Wolf Summon | AnimBlend, FocusAngle, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, AnimBlend); not compared with the real client |
| abillity_skill_table_m.magic.lighting_serial_magic_form | Father's Hammer | Stretch, mesh | rendered; round 6 changes apply (mesh, Stretch); not compared with the real client |
| x2_effects.e_effect.magma_flow_up_water_loop_small | Wave Strike | AnimBlend, FocusAngle, Stretch, material | rendered; round 6 changes apply (FocusAngle, Stretch, AnimBlend); not compared with the real client |
| x2_effects.mon_effects.mon_bailisk_big_poisonpond_throw | Wave Push | AnimBlend, Stretch | rendered; round 6 changes apply (Stretch, AnimBlend); not compared with the real client |
| ability_skill_table.defense.defense0006_launch | Cracked Thorn Shield | material | rendered; not compared with the real client |
| x2_effect_o.mon_effects.mon_throw_ice_hit | Mistral Elemental Ice Arrow | AnimBlend, OrientToVelocity, Stretch, material | rendered; round 6 changes apply (Stretch, OrientToVelocity, AnimBlend); not compared with the real client |
| x2_effect_o.mon_effects.mon_throw_ice_proj | Mistral Elemental Ice Arrow | AnimBlend, FocusAngle, Stretch, mesh | rendered; round 6 changes apply (FocusAngle, mesh, Stretch, AnimBlend); not compared with the real client |
| ability_skill_table_k.p_skill_adament.p_skill_disarmament | Strong Blow |  | not found in the libraries (not rendered) |
| ability_skill_table_k.p_skill_vocation.p_skill_raid_surprise | Vital Strike | OrientToVelocity, Stretch, material | rendered; round 6 changes apply (Stretch, OrientToVelocity); not compared with the real client |
| x2_effects_p.doodad.quest_net_throw_hit | Throw Net | mesh | rendered; round 6 changes apply (mesh); not compared with the real client |
| x2_effects_p.doodad.quest_net_throw_proj | Throw Net | OrientToVelocity, Stretch, material, mesh | rendered; round 6 changes apply (mesh, Stretch, OrientToVelocity); not compared with the real client |
| x2_effects.doodad_effect.quest_whirlwind_launch | Wind Elemental Appears! | FocusAngle, mesh | rendered; round 6 changes apply (FocusAngle, mesh); not compared with the real client |
| x2_effects.e_effect.ship_canon_exp_miss | Water Elemental appears! | AnimBlend, FocusAngle, Stretch, decal, material | rendered; round 6 changes apply (FocusAngle, Stretch, AnimBlend); not compared with the real client |
| x2_effects.doodad_effect.quest_leaves_dust | Earth Guardian Appears! | AnimBlend, FocusAngle | rendered; round 6 changes apply (FocusAngle, AnimBlend); not compared with the real client |
| x2_effects_p.cutscene.kidela01 | Malcolm's Golden Mark |  | rendered; not compared with the real client |
| x2_effects_p.cutscene.kidela02 | Malcolm's Golden Mark | FocusAngle, decal, material, mesh | rendered; round 6 changes apply (FocusAngle, mesh); not compared with the real client |
| x2_effects.doodad_effect.quest_magic_rope_launch | Dahuta's Chains | mesh | rendered; round 6 changes apply (mesh); not compared with the real client |
| x2_effects_p.prefab.soulwood_c_wind | Rabbit Disappear | FocusAngle, mesh | rendered; round 6 changes apply (FocusAngle, mesh); not compared with the real client |
| x2_effects.mon_effects.skeleton_spawn_big | Summon Treasure Guardian | Stretch, decal, material | rendered; round 6 changes apply (Stretch); not compared with the real client |
| x2_effect_o.mon_effects.mon_multishot_launch_b | Treasure Guardian's Buckshot Arrows | FocusAngle, OrientToVelocity, Stretch | rendered; round 6 changes apply (FocusAngle, Stretch, OrientToVelocity); not compared with the real client |
| x2_effects.mon_effects.gargoyle_big_shout_launch | Treasure Guardian's Shout | Facing=Velocity, FocusAngle, Stretch, decal, material | rendered; round 6 changes apply (FocusAngle, Facing=Velocity, Stretch); not compared with the real client |
| x2_effects.mon_effects.skeleton_spawn | Treasure Guardian's Subordinate | Stretch, decal, material | rendered; round 6 changes apply (Stretch); not compared with the real client |
