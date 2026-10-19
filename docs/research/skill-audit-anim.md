# Skill audit: animations, timing, buffs and projectiles (aa-client combat)

Scope: every shown skill of the ten ability trees (110), the 15 basic skills, the 46 emotes and the buffs the player skills bring
(274 distinct). Owner: the combat agent (`crates/aa-client/src/ingame/combat*`, `anim.rs`, `weapons.rs`, `sounds.rs`, `aa-anim`). The effect
side (particles) is `docs/research/skill-audit-fx.md`; the design is `docs/aa-client-combat.md` ("Skill audit round").

## Method

* **Data**: `cargo run -p aa-client --example skill_coverage -- --out data/extract/aa-client-combat/skill_coverage.tsv` resolves every
  skill with the code the client runs: clips on all 8 race/gender models, effects, plots, buffs (new: `buffs of the player skills`).
  Result: **202 clips needed, 0 missing on any of the 8 models** (the elf, hariharan and ferre `.cal` take the player's `all_` clips from
  the Nuian folders), 2 stop markers (not clips), 2 effects missing (typos of the data: `illusion.tired_curse_form`,
  `wild.long_distance_shot_launch`), 226 of 274 buffs show something.
* **Frames**: the client with `AA_COMBAT_SWEEP=<ids>` (see `docs/aa-client-combat.md`, hooks) plays each skill or buff on the test
  character in the spawn pond (Nuian male, one handed sword and shield, hour 12, camera 4.5 m at -90 degrees, fov 60) and saves frames at
  the start, mid cast, fire, mid effect and end; `tools/skill_sheets.py` makes one contact sheet per skill: `shots/skill-audit/anim/sheets/<id>.png`
  (`b<id>.png` for buffs), 231 sheets in all (`shots/` is git-ignored). Real captures to compare with exist only for the melee attack,
  Triple Slash, Kick, Return and the two deaths (`reference/ingame/combat_*`, compared in earlier rounds); every other skill was checked
  against the data (clip, moment, bone) and for gross errors on the sheets.
* **Server**: the shared `bevy` lane wedged accounts during this session (`enter world denied (1)` after a walkthrough that waits for a
  spawn that never comes: the game server's `select_character` lock was held by a stuck connection and the server must be restarted by its
  owner). The audit ran on a private stack (login 28237, game 28239, account `bevyanimaudit`).
* **Limits**: the events are fake (no server result, no damage, target a unit within 20 m or a spot 6 m ahead), the melee attack (skill 2)
  has no clip of its own in the data (the server names the weapon's when it fires) so its sheet shows nothing: it was compared with the
  real capture in round 5. Frames are 960x540 and the character is 100 px high: gross errors only.

## Result

| | before | after |
|---|---:|---:|
| clips missing on any of the 8 models | 0 | 0 |
| stop markers played as a clip (`all_ac_stop`: Use Well) | logged as "no clip" | release the unit |
| buffs of player skills that show something (aura, pose, freeze, fall) | 0 of 226 | 226 (stun, struggle, levitation, fear, Play Dead, instrument and emote poses, ice and stone freeze, auras) |
| skills whose channel or toggle buff was not found (Invincibility, Play Dead) | all | found through `skills.channeling_buff_id` / `toggle_buff_id` |
| anim-event effects played (`effect` / `@effect`: `/flowers`, `/sad`, dances, mining) | 0 of 114 events | 114 |
| projectile kinds handled (`proj_physic_id` 0 and 1 fly, 2 beam, 3 falls) | 1 of 4 | 4 of 4 |
| projectile hit / end items (`fireball_hit` and the 120 others of the groups) | never played | at the landing point |
| plot projectile speed | fixed 0.25 s | `special_effects.value3` m/s |
| effects of a point-targeted skill | dropped (no unit) | on the ground at the position |
| blade trails | ran on | end at the swing's throw moment |
| first use of a skill | clips and effect libraries loaded on demand | preloaded for the action bar (no frame over 35 ms either way on this machine) |

## Worst remaining issues

1. **aa-fx**: Sunder Earth, Retribution, Antithesis, Ollo's Hammer, Whirlwind Slash, Pin Down, Skewer, Freezing Earth draw big flat or
   untextured geometry (`skill-audit-fx.md`); a projectile's tail at flight speed is a streak across the screen (Missile Rain, Pin Down).
2. **Poses without data**: Protective Wings (wings), Summon Wraith (the wraith), Meditate (seated pose): nothing in the skill or buff
   data draws them; Play Dead lies down through its buff's `ragdoll`, `/bow` and `/kneel` through the buff's `fist_ac_*` clips if the server sends the buffs.
3. **Movement of skills** (Charge, Overwhelm, Drop Back, Lasso pull, Shadow Step, Teleportation) is `ingame/skill_motion.rs`; this audit
   only checked their clips and effects.
4. **Races** other than Nuian male: data only, nothing rendered (no character of another race in the walkthrough).
5. **Staff casts** hold the staff across both hands (the `all_` cast clips); there is no 1.2 capture of a spell to compare with.

## Needed from others

* aa-fx: beams are stretched by scaling the anchor's Z by the length (`beam_test.cgf` is one unit long): mesh emitters should follow
  that scale and sprite emitters ignore it; long tails at flight speed; the geometry of the effects above.
* aa-net / the server: `SCBuffCreated` for channeling buffs and for the emote buffs (`fist_ac_sitground`), and the `SkillFired.delay_ms`
  of the projectile skills (the flight time) are what the client trusts.

## Statuses

`clips ok` = every clip of the skill binds on all 8 models; `fx plain / minor / degraded / missing / none` = the effect class of the
coverage example (aa-fx features ignored: minor = a light or a sound, degraded = something that changes the look, missing = not in the
libraries). A skill with no clip is a passive or a buff (its look is its buff). Notes name what this round changed.


## 1. Battlerage

| Skill | Id | Clips (roles) | Effects | Status | Notes / fix |
|---|---:|---|---|---|---|
| Battle Focus | 10377 | all_co_sk_shout[fire] | minor (3) | clips ok; fx minor |  |
| Frenzy | 10455 | all_co_sk_shout[fire] | minor (1) | clips ok; fx minor |  |
| Sunder Earth | 10644 | onehand_co_sk_weapon_blunt_3_cast[step];onehand_co_sk_weapon_blunt_3_launch[fire,step];twohand_co_sk_weapon_blunt_3_cast[start,step];twohand_co_sk_weapon_blunt_3_launch[step,twohand] | degraded (6) | clips ok; fx degraded (aa-fx) | plot 20 steps; cast 1s; giant dark mesh is aa-fx (geometry) |
| Charge | 11918 | all_co_sk_dashattack[controller] | minor (3) | clips ok; fx minor | plot 8 steps; controller dash: movement is skill_motion |
| Precision Strike | 12026 | onehand_co_sk_weapon_blunt_2[fire];twohand_co_sk_weapon_blunt_2[twohand] | minor (4) | clips ok; fx minor |  |
| Bondbreaker | 12034 | all_co_sk_shout[fire] | minor (1) | clips ok; fx minor |  |
| Lasso | 12039 | all_co_sk_pull_cast[start,step];all_co_sk_pull_launch[fire,step] | minor (2) | clips ok; fx minor | plot 7 steps; channel 0s; cast 1s; projectile 86 is a beam (the rope; held for the channel). The pull itself is skill_motion |
| Whirlwind Slash | 13282 | onehand_co_sk_weapon_slash_3[step];twohand_co_sk_weapon_slash_3[step];twoswords_co_sk_weapon_slash_3[step] | minor (7) | clips ok; fx minor | plot 22 steps |
| Tiger Strike | 13315 | (none: passive or buff) | minor (1) | clips ok; fx minor | plot 7 steps |
| Triple Slash | 18132 | onehand_co_sk_streakattack_1[fire,step];onehand_co_sk_streakattack_2[step];onehand_co_sk_streakattack_3[step];twohand_co_sk_streakattack_1[step,twohand];twohand_co_sk_streakattack_2[step];twohand_co_sk_streakattack_3[step];twoswords_co_sk_streakattack_1[dual,step];twoswords_co_sk_streakattack_2[step];twoswords_co_sk_streakattack_3[step] | minor (5) | clips ok; fx minor | plot 26 steps |
| Terrifying Roar | 18308 | all_co_sk_shout[fire] | minor (1) | clips ok; fx minor |  |

## 2. Witchcraft

| Skill | Id | Clips (roles) | Effects | Status | Notes / fix |
|---|---:|---|---|---|---|
| Lassitude | 10134 | all_co_sk_buff_launch_special[fire] | missing (3) | clips ok; fx missing (data) | `tired_curse_form` is missing in the effect library (a typo in the data: nothing to play) |
| Bubble Trap | 10154 | all_co_sk_spell_cast_d[start];all_co_sk_spell_launch_d[fire] | minor (5) | clips ok; fx minor | cast 1.5s |
| Enervate | 10159 | all_co_sk_buff_launch_defense[fire] | minor (5) | clips ok; fx minor |  |
| Insidious Whisper | 10409 | all_co_sk_buff_cast_mana[start];all_co_sk_buff_launch_special[fire] | minor (3) | clips ok; fx minor | cast 1.5s |
| Play Dead | 10488 | (none: passive or buff) | plain (1) | clips ok; fx plain | channel 60s; Play Dead: buff ragdoll pose (dead_01) held while the buff lasts |
| Purge | 10712 | all_co_sk_spell_launch_swich[fire] | minor (5) | clips ok; fx minor |  |
| Focal Concussion | 11353 | all_co_sk_buff_launch_mental[fire] | minor (3) | clips ok; fx minor |  |
| Courageous Action | 11424 | all_co_sk_shout[fire] | minor (1) | clips ok; fx minor |  |
| Dahuta's Breath | 11443 | all_co_sk_spell_launch_spread[fire] | minor (3) | clips ok; fx minor |  |
| Banshee Wail | 12001 | all_co_sk_buff_launch_special[fire] | minor (2) | clips ok; fx minor |  |
| Earthen Grip | 14376 | all_co_sk_spell_cast_site[start];all_co_sk_spell_launch_site[fire] | degraded (4) | clips ok; fx degraded (aa-fx) | cast 1s |

## 3. Defense

| Skill | Id | Clips (roles) | Effects | Status | Notes / fix |
|---|---:|---|---|---|---|
| Invincibility | 10372 | all_co_sk_buff_cast_defense[fire] | plain (1) | clips ok; fx plain | channel 12s; aura = the skill's channeling buff 131 (now shown); channel pose loops 12 s |
| Redoubt | 10375 | onehand_co_sk_shielddefense[fire] | minor (3) | clips ok; fx minor |  |
| Shield Slam | 10399 | onehand_co_sk_shieldwield[fire,step] | minor (6) | clips ok; fx minor | plot 12 steps |
| Mocking Howl | 10436 | all_co_sk_shout[fire] | minor (2) | clips ok; fx minor |  |
| Bull Rush | 10501 | onehand_co_sk_shieldpush[fire,step] | minor (2) | clips ok; fx minor | plot 15 steps |
| Refreshment | 10645 | all_co_sk_shout[fire] | minor (1) | clips ok; fx minor |  |
| Toughen | 11365 | all_co_sk_shout[fire] | minor (1) | clips ok; fx minor |  |
| Revitalizing Cheer | 12046 | all_co_sk_shout[fire] | minor (1) | clips ok; fx minor |  |
| Boastful Roar | 12048 | all_co_sk_spell_launch_spread[fire] | minor (5) | clips ok; fx minor |  |
| Imprison | 14529 | all_co_sk_spell_launch_spread[fire] | minor (2) | clips ok; fx minor |  |
| Ollo's Hammer | 18757 | all_co_sk_underthrow[step] | minor (3) | clips ok; fx minor | plot 7 steps |

## 4. Auramancy

| Skill | Id | Clips (roles) | Effects | Status | Notes / fix |
|---|---:|---|---|---|---|
| Leech | 10104 | all_co_sk_spell_launch_flash[fire] | minor (2) | clips ok; fx minor |  |
| Teleportation | 10152 | all_co_sk_buff_launch_teleport[fire] | degraded (1) | clips ok; fx degraded (aa-fx) |  |
| Vicious Implosion | 10710 | all_co_sk_shout[fire,step] | degraded (4) | clips ok; fx degraded (aa-fx) | plot 16 steps |
| Protective Wings | 10714 | all_co_sk_buff_cast_mental[fire] | minor (2) | clips ok; fx minor | channel 18s |
| Liberation | 11380 | all_co_sk_buff_launch_mana[fire] | minor (1) | clips ok; fx minor |  |
| Shrug It Off | 11429 | all_co_sk_buff_launch_mana[fire] | minor (1) | clips ok; fx minor |  |
| Conversion Shield | 11869 | all_co_sk_buff_launch_mana[fire] | minor (1) | clips ok; fx minor |  |
| Meditate | 11989 | all_co_sk_spell_channel_pray[fire] | minor (2) | clips ok; fx minor | channel 9s |
| Health Lift | 11991 | all_co_sk_spell_cast_vitality[start];all_co_sk_spell_launch_vitality[fire] | minor (4) | clips ok; fx minor | cast 1.5s |
| Thwart | 16486 | all_co_sk_shout[fire] | minor (1) | clips ok; fx minor |  |
| Comet's Boon | 18222 | all_co_sk_spell_launch_fireball2[step] | degraded (4) | clips ok; fx degraded (aa-fx) | plot 12 steps |

## 5. Occultism

| Skill | Id | Clips (roles) | Effects | Status | Notes / fix |
|---|---:|---|---|---|---|
| Hell Spear | 10135 | all_co_sk_spell_launch_field[fire] | minor (2) | clips ok; fx minor |  |
| Crippling Mire | 10201 | all_co_sk_buff_cast_defense[start];all_co_sk_spell_channel_sorb[fire,step] | minor (6) | clips ok; fx minor | plot 18 steps; channel 4s; projectile 38 is a beam (stretched, held for the channel) |
| Summon Wraith | 10434 | all_co_sk_spell_channel_cure[fire] | plain (1) | clips ok; fx plain | channel 9s |
| Retribution | 10655 | all_co_sk_shout[fire] | minor (1) | clips ok; fx minor |  |
| Stillness | 10665 | all_co_sk_buff_launch_mana[fire] | degraded (1) | clips ok; fx degraded (aa-fx) |  |
| Summon Crows | 11395 | all_co_sk_buff_launch_defense[fire] | plain (1) | clips ok; fx plain |  |
| Absorb Lifeforce | 11441 | all_co_sk_spell_channel_sorb[fire,step] | minor (3) | clips ok; fx minor | plot 10 steps; channel 6s; projectile 81 is a beam (stretched, held for the channel) |
| Urgency | 11442 | all_co_sk_spell_cast_dead[start];all_co_sk_spell_launch_dead[fire] | degraded (3) | clips ok; fx degraded (aa-fx) | cast 5s; plays `all_co_sk_spell_cast_dead` (a crouch by the data) |
| Telekinesis | 11504 | all_co_sk_spell_cast_tug[start];all_co_sk_spell_channel_tug[fire] | minor (3) | clips ok; fx minor | channel 12s; cast 2s; projectile 71 is a beam (stretched, held for the channel) |
| Mana Force | 12759 | all_co_sk_spell_launch_spread[fire,step] | minor (2) | clips ok; fx minor | plot 7 steps; projectile 118 is a beam (stretched, held for the channel) |
| Mana Stars | 14810 | all_co_sk_spell_launch_l[step];all_co_sk_spell_launch_r[fire,step] | minor (3) | clips ok; fx minor | plot 7 steps |

## 6. Archery

| Skill | Id | Clips (roles) | Effects | Status | Notes / fix |
|---|---:|---|---|---|---|
| Float | 10694 | all_co_sk_shout[fire] | minor (1) | clips ok; fx minor |  |
| Intensity | 10708 | all_co_sk_shout[fire] | minor (1) | clips ok; fx minor |  |
| Double Recurve | 11368 | all_co_sk_shout[fire] | minor (1) | clips ok; fx minor |  |
| Concussive Arrow | 11933 | bow_co_sk_cast_2[start];bow_co_sk_launch_2[fire] | minor (3) | clips ok; fx minor | cast 1.5s |
| Snare | 12133 | all_co_sk_buff_launch_force[fire] | degraded (1) | clips ok; fx degraded (aa-fx) |  |
| Missile Rain | 13281 | bow_co_sk_high_cast[step];bow_co_sk_high_launch[step] | missing (5) | clips ok; fx missing (data) | plot 12 steps; cast 2s; plot projectile speed from the data (value3 25 m/s); `long_distance_shot_launch` is a typo of the data (effect library has `test.` only) |
| Piercing Shot | 13564 | bow_co_sk_launch[fire] | minor (3) | clips ok; fx minor |  |
| Boneyard | 14760 | all_co_sk_spell_launch_spread[step] | plain (2) | clips ok; fx plain | plot 22 steps |
| Endless Arrows | 14835 | bow_co_sk_cast_2[start];bow_co_sk_launch_2[fire,step] | minor (2) | clips ok; fx minor | plot 7 steps |
| Deadeye | 15073 | all_co_sk_buff_cast_mana[start];all_co_sk_buff_launch_mana[fire] | minor (2) | clips ok; fx minor | cast 0.5s |
| Charged Bolt | 16210 | bow_co_sk_launch[fire] | minor (6) | clips ok; fx minor |  |

## 7. Sorcery

| Skill | Id | Clips (roles) | Effects | Status | Notes / fix |
|---|---:|---|---|---|---|
| Freezing Earth | 10151 | all_co_sk_spell_launch_ground[fire] | degraded (3) | clips ok; fx degraded (aa-fx) |  |
| Insulating Lens | 10153 | all_co_sk_spell_cast_shield[start];all_co_sk_spell_launch_shield[fire] | degraded (4) | clips ok; fx degraded (aa-fx) | cast 2s |
| Meteor Strike | 10664 | all_co_sk_spell_cast_meteor[start];all_co_sk_spell_launch_meteor[fire] | degraded (5) | clips ok; fx degraded (aa-fx) | cast 5s; projectile 8 is physics 3: falls from the sky onto the position (was: flew from the caster) |
| Freezing Arrow | 10667 | all_co_sk_spell_cast_fireball[start,step];all_co_sk_spell_launch_fireball[fire,step] | minor (9) | clips ok; fx minor | plot 16 steps; cast 2s |
| Arc Lightning | 10670 | all_co_sk_spell_cast_flash[start,step];all_co_sk_spell_launch_flash[fire,step] | minor (4) | clips ok; fx minor | plot 12 steps; cast 2s; projectile 66 is physics 2: a beam, now stretched hand to target (was: a tiny flash that flew) |
| Flamebolt | 10752 | all_co_sk_spell_cast_fireball[start,step];all_co_sk_spell_launch_fireball[fire,step] | minor (9) | clips ok; fx minor | plot 15 steps; cast 1s; projectile impact (`fireball_hit`) now plays where the bolt lands |
| Frigid Tracks | 11314 | all_co_sk_spell_cast_cold[start];all_co_sk_spell_launch_cold[fire] | degraded (3) | clips ok; fx degraded (aa-fx) | cast 2.5s |
| Searing Rain | 11939 | all_co_sk_spell_cast_cure[step];all_co_sk_spell_channel_cure[step] | degraded (6) | clips ok; fx degraded (aa-fx) | plot 11 steps; cast 2s |
| Chain Lightning | 11967 | all_co_sk_spell_launch_flash[step] | minor (3) | clips ok; fx minor | plot 13 steps |
| Magic Circle | 12796 | all_co_sk_spell_cast_shield[start];all_co_sk_spell_launch_shield[fire,step] | minor (5) | clips ok; fx minor | plot 7 steps |
| Flame Barrier | 14774 | all_co_sk_spell_launch_swich[step] | plain (1) | clips ok; fx plain | plot 9 steps |

## 8. Shadowplay

| Skill | Id | Clips (roles) | Effects | Status | Notes / fix |
|---|---:|---|---|---|---|
| Stealth | 10082 | (none: passive or buff) | plain (1) | clips ok; fx plain |  |
| Freerunner | 10189 | (none: passive or buff) | plain (1) | clips ok; fx plain |  |
| Toxic Shot | 10481 | bow_co_sk_launch[fire] | minor (2) | clips ok; fx minor |  |
| Shadowsmite | 10496 | onehand_co_sk_weapon_pierce[fire,step];twohand_co_sk_weapon_pierce[step,twohand];twoswords_co_sk_weapon_pierce[step] | minor (2) | clips ok; fx minor | plot 21 steps |
| Overwhelm | 10648 | all_co_sk_leapattack[controller] | minor (3) | clips ok; fx minor | plot 15 steps |
| Wallop | 12029 | onehand_co_sk_fastattack_1[fire];twohand_co_sk_fastattack_1[twohand];twoswords_co_sk_fastattack_1[dual] | minor (2) | clips ok; fx minor |  |
| Drop Back | 12049 | all_co_sk_backstep[controller] | plain (2) | clips ok; fx plain |  |
| Shadow Step | 12075 | all_co_sk_buff_launch_teleport[fire,twohand] | plain (2) | clips ok; fx plain |  |
| Stalker's Mark | 12139 | bow_co_sk_launch[fire] | minor (2) | clips ok; fx minor |  |
| Pin Down | 13344 | onehand_co_sk_weapon_slash_2[fire];twohand_co_sk_weapon_slash_2[twohand] | minor (2) | clips ok; fx minor |  |
| Rapid Strike | 18125 | onehand_co_sk_weapon_blunt[fire,step];onehand_co_sk_weapon_slash[step];twohand_co_sk_weapon_blunt[step,twohand];twohand_co_sk_weapon_slash[step];twoswords_co_sk_weapon_blunt[dual,step];twoswords_co_sk_weapon_slash[step] | minor (2) | clips ok; fx minor | plot 23 steps |

## 9. Songcraft

| Skill | Id | Clips (roles) | Effects | Status | Notes / fix |
|---|---:|---|---|---|---|
| [Perform] Quickstep | 10723 | (none: passive or buff) | degraded (2) | clips ok; fx degraded (aa-fx) |  |
| [Perform] Ode to Recovery | 10724 | (none: passive or buff) | degraded (2) | clips ok; fx degraded (aa-fx) |  |
| [Perform] Bloody Chantey | 10727 | (none: passive or buff) | minor (2) | clips ok; fx minor |  |
| Hummingbird Ditty | 11377 | music_co_sk_drum_cast[instrument];music_co_sk_drum_launch[instrument];music_co_sk_lute_cast[instrument];music_co_sk_lute_launch[instrument];music_co_sk_pipe_cast_3[instrument];music_co_sk_pipe_launch_2[instrument] | degraded (3) | clips ok; fx degraded (aa-fx) | cast 1.5s |
| [Perform] Bulwark Ballad | 11396 | (none: passive or buff) | degraded (3) | clips ok; fx degraded (aa-fx) |  |
| Startling Strain | 11934 | music_co_sk_drum_cast_2[instrument];music_co_sk_drum_launch_2[instrument];music_co_sk_lute_cast_2[instrument];music_co_sk_lute_launch[instrument,step];music_co_sk_pipe_cast_2[instrument];music_co_sk_pipe_launch_2[instrument,step] | minor (6) | clips ok; fx minor | plot 13 steps |
| Dissonant | 11943 | music_co_sk_drum_cast_2[instrument];music_co_sk_drum_launch_2[instrument];music_co_sk_lute_cast_3[instrument];music_co_sk_lute_launch_2[instrument];music_co_sk_pipe_cast_3[instrument];music_co_sk_pipe_launch_2[instrument] | minor (6) | clips ok; fx minor | cast 1.5s |
| Rhythmic Renewal | 11948 | music_co_sk_drum_cast[instrument];music_co_sk_drum_launch[instrument];music_co_sk_lute_cast[instrument];music_co_sk_lute_launch[instrument];music_co_sk_pipe_cast[instrument];music_co_sk_pipe_launch[instrument] | minor (2) | clips ok; fx minor | cast 2s |
| Alarm Call | 11961 | (none: passive or buff) | minor (1) | clips ok; fx minor |  |
| Critical Discord | 11973 | music_co_sk_drum_cast[instrument];music_co_sk_drum_launch_2[instrument];music_co_sk_lute_cast_3[instrument];music_co_sk_lute_launch_2[instrument];music_co_sk_pipe_cast_3[instrument];music_co_sk_pipe_launch_2[instrument] | minor (6) | clips ok; fx minor | cast 1s |
| Healing Hymn | 17413 | music_co_sk_drum_launch_2[step];music_co_sk_lute_launch_2[step];music_co_sk_pipe_launch_2[step] | minor (10) | clips ok; fx minor | plot 23 steps |

## 10. Vitalism

| Skill | Id | Clips (roles) | Effects | Status | Notes / fix |
|---|---:|---|---|---|---|
| Antithesis | 10534 | all_co_sk_spell_cast_swich[start];all_co_sk_spell_launch_swich[fire] | minor (4) | clips ok; fx minor | cast 2s |
| Revive | 10546 | all_co_sk_spell_cast_union[start];all_co_sk_spell_launch_union[fire] | degraded (5) | clips ok; fx degraded (aa-fx) | cast 7s |
| Resurgence | 10547 | all_co_sk_spell_launch_bless[fire] | minor (3) | clips ok; fx minor |  |
| Mend | 10720 | all_co_sk_spell_cast_love[start];all_co_sk_spell_launch_love[fire] | minor (6) | clips ok; fx minor | cast 3s |
| Twilight | 10721 | all_co_sk_buff_cast_mental[fire] | plain (2) | clips ok; fx plain | channel 60s |
| Mirror Light | 11379 | all_co_sk_spell_launch_missile[fire] | minor (2) | clips ok; fx minor |  |
| Skewer | 13286 | all_co_sk_spell_cast_spear[step];all_co_sk_spell_launch_spear[step] | degraded (5) | clips ok; fx degraded (aa-fx) | plot 13 steps; cast 2s |
| Fervent Healing | 14929 | all_co_sk_spell_launch_r[fire] | minor (3) | clips ok; fx minor |  |
| Aranzeb's Boon | 16004 | all_co_sk_buff_cast_mental[start];all_co_sk_buff_launch_mental[fire] | minor (2) | clips ok; fx minor | cast 2s |
| Infuse | 16783 | all_co_sk_spell_launch_swich[fire] | degraded (1) | clips ok; fx degraded (aa-fx) |  |
| Renewal | 17412 | all_co_sk_spell_cast_bless[start];all_co_sk_spell_launch_bless[fire] | minor (2) | clips ok; fx minor | cast 1s |

## Basic skills (15) and emotes (46)

Basic: Melee Attack (weapon clip chosen by the server, compared with the real capture), Kick (14495, plot, compared), Return (10594,
`all_co_sk_buff_cast_mental` loop 10 s), Rider's Escape (22143, `fist_ac_summon_cast` 3.5 s then `_launch`), Shoot Arrow (16064, compared
with the real capture), the three instruments' plays (`music_co_sk_*_cast` / `_launch` by the instrument held), glider and mount skills
(no clip, controllers). Emotes: every one of the 46 plays its clip (`fist_em_*`, `fist_ac_*`); `/flowers` (14492) starts the flowers at
`item_hand_l` 0.29 s into the clip, `/sad` (14453) the tears on `eye_right_bone`; the sitting and begging emotes hold their pose through their buff
(`fist_ac_sitground` 2004, `fist_ac_beggar_01` 2003). Sheets: `shots/skill-audit/anim/sheets/<id>.png`.

## Buffs

226 buffs of player skills show something: 220 with an effect group, 53 with a pose (stun `all_re_combat_stun` and its `_start`,
struggle, levitation `all_co_sk_hovering`, fear `fist_em_fear`, dance `onehand_re_dance`, sitting, begging), 5 freeze the animation
(`framehold`: ice and stone), 1 lies the unit down (Play Dead 649: `dead_01` for creatures, `dead_02` is the player's `.cal` entry; found
in the first buff sweep as "no clip", fixed). The sweep sheets are `b<id>.png`; most auras are subtle and only readable in the
full size frames. The 48 buffs of player skills that show nothing have neither group nor clips (passives and numeric buffs).

## Hidden and monster skills

The 60 hidden sub skills of the trees (call skills, weapon variants) use the clips of their parent and were checked by the data
(0 missing). The 370 monster skills are outside this audit (14 of them use a clip their model's `.cal` lacks: `docs/aa-client-combat.md`).
