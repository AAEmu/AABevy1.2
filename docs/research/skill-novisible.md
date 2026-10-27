# Skills with no visible effect or pose: data triage

Source list: the "No visible effect or pose" part of `docs/research/skill-reel-after.md` (after2 reel, sheets in
`shots/reel/after2/*_sheet.png`). Read-only research: DB `compact.sqlite3` (read-only), the pak through `tools/aa-pak.exe`,
the client code (`crates/aa-client/src/ingame/combat*`, `skill_motion*`), `tools/reel/*.py` and the existing audit notes.
No client was run and no cargo build was made.

## Summary

43 skills: the list of the reel doc (Battlerage 3, Witchcraft 7 including Focal Concussion, Defense 4, Auramancy 4, Occultism 4,
Archery 4, Sorcery 4, Shadowplay 3, Vitalism 3) and the seven songs it names. Verdicts:

| verdict | count | skills |
|---|---:|---|
| a: nothing visible by data, the reel is right | 2 | Protective Wings, Twilight |
| b: we fail to play something (cause given per row) | 27 | see table |
| c: unknown (data has a visible effect we do not see; cause not provable from data) | 14 | see table |

Top recurring causes, most to least common:

1. **The reel never sends buff events (harness).** `tools/reel/plan.py` and `run_reel.py` emit only start, fire, hit, end,
   plot and tp steps; no `T:buff` step. The buff path of the client (`ingame/combat/buffs.rs`) shows auras, poses, ragdolls
   and freezes only for buffs the server (or a `T:buff` step) creates. The audit sweep does inject them (`combat/sweep.rs`,
   `book.skill_buffs(id)`), the reel does not. About 24 of the 27 "b" rows have this as a cause. The real client plays these
   when the server sends `SCBuffCreated`, so this is a reel gap, not a client bug, but the reel cannot judge any buff-side visual.
2. **The reel's flights and channels are not the real ones (harness).** Projectile flight in the reel is the fake
   `effect_delay` (71 ms for Endless Arrows, 288 ms for Bubble Trap, 500 ms default when it is 0; the real arrow is
   600 to 720 ms at 8 to 17 m, per `docs/aa-client-combat.md`). Aa-fx starts a short arrow late (one pulse period plus model
   load), so the arrow is missing on the sheets: Endless Arrows, Freezing Arrow, Bubble Trap. Also `CHANNEL_CAP = 4.0`
   (`plan.py:55`) ends every channel after 4 s, so held effects of 9 to 60 s channels (Meditate, Invincibility, Telekinesis,
   Protective Wings, Twilight, Play Dead) are cut short.
3. **Missing skill controller rows (database).** 550 skills point at 173 `skill_controller_id` values that have no row in
   `skill_controllers` (926 rows). Bull Rush (4433), Precision Strike (703) and Mana Force / Lasso (732) are among them, so
   the client has no dash, lunge or pull to play. Plot steps reference controllers that all exist (348 of 348).
4. **FxChr items are never played (client).** `ingame/combat/skills.rs` counts `FxChr`, `FxRope`, `FxCamFov`, `FxMotionBlur`
   as "other" and plays none. Summon Crows' crows are `game/objects/effects/crow/crow.chr` (in the pak, 792 KB). The DB has 12
   FxChr items; the other ones belong to other skills (golem hand, fepee bird, pet net, glider).
5. **Aa-fx features the data needs and the coverage tool marks as lost (client, unverified).** Mesh beams with
   `ForceGeneration="Target"` (Telekinesis, Mana Force: `beam_test_water.cgf`), decal rings (Stillness, Magic Circle, Banshee
   Wail, Vicious Implosion), Boneyard and Flame Barrier. Round 10 of `skill-audit-fx.md` compared some of them with the real
   client at 8 m and found them close but not the same; the reel shows nothing, which is not explained by the data.
6. **Target hit effects of melee and shield skills are not seen** (Precision Strike, Wallop, Shield Slam). Data has the hit
   group on the target at the hit moment; the cause is not established (the reel's hit is non-critical; the doll shows no
   reaction).

Other notes: sound-only particles (`x2_sounds.particles.*`: Twilight, Startling Strain, Rhythmic Renewal) have nothing to draw
by design. Stealth's `buffs.stealth` flag is read nowhere in `aa-client` (grep), so that state has no look at all.

## Legend

* Effect names are `library.group.name` of `game/libs/particles/*.xml`; `m.` = `abillity_skill_table_m.`, `k.` =
  `ability_skill_table_k.`, `adament.` = `k.p_skill_adament.`, `will.` = `k.p_skill_will.`, `romance.` = `k.p_skill_romance.`.
  "caster" / "target" is the fx location; moments are the fx start-end ids (0 START, 1 FORM, 2 THROW, 3 LAUNCH, 4 HIT, 5 END,
  6 PROJ_END).
* Clips: `[L]` = `anims.loop` is set (loops until the skill ends). "start" is the cast clip (played only when `casting_time` > 0).
* "buff N" = a `BuffEffect` (or channel / toggle buff) of the skill. Buff visuals are only shown when the buff is created
  (server packet, or a `T:buff` step in the reel, which the reel never sends).

## Table

| id | name | class | expected anim | expected fx paths | verdict | note |
|---|---|---|---|---|---|---|
| 12034 | Bondbreaker | Battlerage | fire `all_co_sk_shout[L]` | caster `m.fight.restrict_clear_launch` (0-5); voice; dispel 57 (caster's root/stun), immunity buff 810 (no visuals) | c | The only own visual is the caster particle (FocusAngle, Stretch, mesh), rendered in aa-fx but not seen. The removal of root/stun needs a buff the reel never creates. |
| 10455 | Frenzy | Battlerage | fire `all_co_sk_shout[L]` | caster `m.fight.reckless_cast` (2-5); buff 182 (35 s, self): `m.fight.reckless_dot` on both hands, `reckless_end` | b | Aura is buff 182 only: the reel sends no buff (harness). |
| 12026 | Precision Strike | Battlerage | fire `onehand_co_sk_weapon_blunt_2[L]` (2H `twohand_..._2`) | caster `m.fight.critical_damage_sword` (1-2, hand); target `m.fight.critical_damage_hit` (4-6, L2); camera_recoil; FxMaterial flash (skipped by design) | c | Swing shows, the target hit does not. Controller 703 has no row (cause 3). Reel hit is non-critical; not provable. |
| 10154 | Bubble Trap | Witchcraft | cast `all_co_sk_spell_cast_d[L]` 1.5 s; fire `all_co_sk_spell_launch_d[L]` | caster `m.illusion.buble_form`, `buble_form2`, `buble_throw`; projectile 137 (group 728 `x2_effect_o.mon_effects.mon_water_bunch_proj`, delay 288 ms); target `m.illusion.buble_hit`; buffs 96 / 2286 on target: pose `all_re_combat_struggle[L]`, `buble_dot`, `buble_dot2` | b | Target bubble and struggle pose are buff-only (harness). Short flight (cause 2) and the monster projectile's Stretch/FocusAngle also unseen. |
| 10409 | Insidious Whisper | Witchcraft | cast `all_co_sk_buff_cast_mana[L]` 1.5 s; fire `all_co_sk_buff_launch_special[L]` | caster `m.illusion.fear_form`, `fear_throw`; target `m.illusion.fear_hit`; voice; buffs 156 / 2277 on target: pose `fist_em_fear[L]` (end `all_ac_stop`), `m.illusion.fear_dot` | b | Target fear pose is buff-only (harness). |
| 10712 | Purge | Witchcraft | fire `all_co_sk_spell_launch_swich[L]` | caster `m.illusion.spell_clear_form`, `spell_clear_form_hand`, `spell_clear_throw`, `spell_clear_launch`; target `m.illusion.spell_clear_hit` (mesh, Stretch); dispel 5; buffs 266 / 11 / 3190 | c | Caster launch and target hit are rendered in aa-fx but not seen. Target is an area (type 5); the reel's hit may be placed off the doll. Not provable from data. |
| 12001 | Banshee Wail | Witchcraft | fire `all_co_sk_buff_launch_special[L]`, effect delay 130 | caster `m.illusion.shout_fear_throw`, `shout_fear_launch` (decal, Stretch: degraded in coverage); area 5 m; buffs 975 / 2278 fear on targets: `shout_fear_hit`, `shout_fear_dot` | b | Fear on the targets is buff-only (harness). The launch ring is a decal (unverified). |
| 11443 | Dahuta's Breath | Witchcraft | fire `all_co_sk_spell_launch_spread[L]`, effect delay 200 (start clip 171 unused: no cast time) | caster `m.illusion.water_wave_launch` (AnimBlend, Facing=Velocity, mesh), `buble_form`, `buble_throw`; voice; area damage 865 on the doll, buff 1366 (no visuals) | c | The caster wave is the only data visual (mesh, Facing=Velocity: degraded in coverage). Not seen; the target has no hit group in data. |
| 10488 | Play Dead | Witchcraft | no clip in the skill; channel 60 s (stop) | caster `m.death.death_trick_launch` (0-5); channel buff 649 (60 s, ragdoll = yes): the client plays `dead_01` and holds | b | The lie-down is the channel buff's ragdoll (buffs.ragdoll = t). Reel sends no buff 649 and caps the channel at 4 s. |
| 11353 | Focal Concussion | Witchcraft | fire `all_co_sk_buff_launch_mental[L]` | caster `m.illusion.poison_exp_throw`; target `m.illusion.poison_exp_launch`, `poison_exp_hit` (L2); voice; buff 449 (1.5 s, target): pose `all_re_combat_struggle[L]`, `poison_exp_dot_end`, sound | b | Target pose is buff-only (harness). |
| 10399 | Shield Slam | Defense | fire `onehand_co_sk_shieldwield[L]` (plot 266, 12 steps) | caster `adament.def_attack_launch_lv1..3`; target `adament.def_attack_hit_lv1..3` (mesh, Stretch); camera_recoil; plot stun step (buff) | b | Target stun pose is buff 501 / plot buff (harness). Target hit group is mesh and not seen (see cause 6). |
| 10501 | Bull Rush | Defense | fire `onehand_co_sk_shieldpush[L]` (plot 267, 15 steps) | caster `adament.deffence_launch`; target `adament.deffence_hit`; camera_recoil; target buffs 257 (silence 3 s), 505 (knockdown, end `all_re_stop`) | b | Dash controller 4433 has no row in `skill_controllers` (cause 3): no movement at all. Target buffs: harness. |
| 10375 | Redoubt | Defense | fire `onehand_co_sk_shielddefense[L]` | caster `adament.deffence_active_launch` (0-5); voice; buffs 328-330 (`adament.deffence_active_dot`) and 2206-2208 (`magic.magic_reinforce_dot_hand_l/r`), 20 s on the caster | b | The aura is buff-only (harness). |
| 10372 | Invincibility | Defense | fire `all_co_sk_buff_cast_defense[L]`; channel 12 s (stop `all_ac_stop`) | caster `adament.unbeatable_form` (0-5), `unbeatable_dot`, `unbeatable_dot_end`; voice; channel buff 131 (12 s): `defense.defense0008_form` (1-2, hand), `defense0008_launch` | b | Channel buff 131 carries the visible shield (harness). Reel caps the channel at 4 s. |
| 10710 | Vicious Implosion | Auramancy | fire `all_co_sk_shout[L]` (plot 580, 16 steps) | caster `will.masic_ex_launch_level1..3` (decal, OrientToVelocity, Stretch); pull step with skill controller 6702 on the targets (kind 2, value4 -500); buff 3200 "vacuum pulled" on targets (`m.death.psychokinesis_dot`) | c | The pull is a controller on the targets: the doll does not move in the reel. Caster ring is a decal (degraded). Lead: check how kind 2 controllers are applied to a target (`skill_motion/path.rs` reads kind 2 as a leap). |
| 11989 | Meditate | Auramancy | fire `all_co_sk_spell_channel_pray[L]`; channel 9 s (stop `all_co_sk_stop`) | caster `will.spirit_form` (1-3, hand), `will.spirit_launch` (3-5); voice; channel buff 455 (9 s): `will.spirit_dot01`, `spirit_dot_end` | b | No seated clip in skill or buff data: the standing pray pose on the sheet is what the data gives. The visible aura is buff 455 (harness); the channel is cut at 4 s. |
| 10104 | Leech | Auramancy | fire `all_co_sk_spell_launch_flash[L]` | caster `will.masic_off_launch` (decal, Stretch); target `will.masic_off_hit` (FocusAngle, mesh); buff 84 (5 s, target): pose `all_re_combat_stun[L]`, `m.illusion.sleeping_dot` | b | Target stun pose is buff-only (harness). The target hit mesh is unverified. |
| 10714 | Protective Wings | Auramancy | fire `all_co_sk_buff_cast_mental[L]`; channel 18 s (stop `all_co_sk_stop`) | caster `will.spell_shield_form` (1-5, hand), `spell_shield_form_end`; voice. No skill effects, no buffs, no channel buff | a | The only visuals in the data are the hand orb and the cast pose; the sheet matches. The wings are in no skill, buff or effect row. |
| 12759 | Mana Force | Occultism | fire `all_co_sk_spell_launch_spread[L]`; channel (tick 250, stop `all_ac_stop`) | caster `m.magic.vital_absorb_form` (3-5, star); beam projectile 118 (physic 2, group 681 `m.magic.vital_absorb_launch`: mesh `beam_test_water.cgf`, `ForceGeneration="Target"`); target buffs 1226 / 3743 (1 s, `m.death.psychokinesis_dot`) | c | Caster star and mesh beam are not seen, though round 10 matched the star in the example. Force generation is not modelled (cause 5). Target drain buffs are harness. |
| 11504 | Telekinesis | Occultism | cast `all_co_sk_spell_cast_tug[L]` 2 s; fire `all_co_sk_spell_channel_tug[L]`; channel 12 s (stop `all_co_sk_stop`); no controller | caster `m.death.spell_move_form` (1-2), `spell_move_launch_form` (3-5), `spell_move_dot_end`; beam projectile 71 (physic 2, group 493 `m.death.spell_move_launch`: mesh + ForceGeneration), bone 9 to 19 | c | Beam on the doll not seen. Absorb Lifeforce's channel beam renders in the same reel, so the difference is the mesh / ForceGeneration beam (cause 5). Channel cut at 4 s. |
| 10665 | Stillness | Occultism | fire `all_co_sk_buff_launch_mana[L]`, effect delay 200 | caster `m.death.wide_silence_launch` (decal ring 13 m, AnimBlend, Facing=Velocity, Stretch; degraded); voice; target buffs 245 (silence 3.5 s, `m.death.wide_silence_dot`), 425 (cast block) | c | Ring is a decal and not seen. Silence on the target is buff-only (harness). |
| 11395 | Summon Crows | Occultism | fire `all_co_sk_buff_launch_defense[L]`, effect delay 200 | caster `m.death.crow_summon_throw` (3-5); voice; target buff 470 (5 s): `m.death.crow_summon_launch`, **FxChr `game/objects/effects/crow/crow.chr`** (not played), FxSound; buff 196 (poison, dot `m.wild.poison_shot_dot`) | b | The crows are FxChr items and the client never plays FxChr (cause 4). The crow buff is also buff-only (harness). |
| 14835 | Endless Arrows | Archery | plot 1526: launch anim `bow_co_sk_launch_2[L]`, cast `bow_co_sk_cast_2[L]` (no cast time); effect delay 71 ms | launch step: `m.wild.continue_shot_hit` (target 4-5, L2), projectile 114 (group 670 `m.wild.continue_shot_proj`: Geometry `objects/effects/fx_arrowhead_tehgin.cgf`, Material `fx_arrow_a1`) | b | Arrow is an aa-fx Geometry on a short flight: missing, as for all arrows in this reel (cause 2). Piercing Shot shows no arrow either. Real flight 600 to 720 ms. |
| 12133 | Snare | Archery | fire `all_co_sk_buff_launch_force[L]`, effect delay 200 | caster `m.wild.hold_steps_throw` (fume, decal, Stretch, bone 9, 3-5); voice; target buff 883 (4.6 s, root): `m.wild.hold_steps_dot`, `hold_steps_dot_start`, `hold_steps_dot_end` | b | Root and its fume on the target are buff-only (harness). Caster throw is a decal (degraded). |
| 10694 | Float | Archery | fire `all_co_sk_shout[L]`; toggle buff 181 (6 s) | caster `m.wild.flowup_launch` (0-5, FocusAngle, Stretch); voice; toggle buff 181: pose `all_co_sk_hovering[L]` (levitation), `m.wild.flowup_dot`, end `all_re_stop` | b | The levitation is the toggle buff's start pose (harness): the sheet shows the caster on the ground. |
| 14760 | Boneyard | Archery | plot 118 (22 steps): anim `all_co_sk_spell_launch_spread[L]` | caster `m.wild.bone_wall_a_throw` (launch, 3-5, 8 smoke puffs, Stretch, mesh); projectile 126 `m.wild.bone_wall_proj_end` (end step, plain); the four bone-shot steps carry no effect rows | c | By data only the smoke burst at the caster's hand and the cast pose. The bone prison has no effect row at its steps. Needs a real 1.2 capture before calling it a bug. |
| 10667 | Freezing Arrow | Sorcery | cast `all_co_sk_spell_cast_fireball[L]` 2 s; fire `all_co_sk_spell_launch_fireball[L]` (plot 278, 16 steps) | caster `m.magic.icearrow_form`, `icearrow_form2`, `icearrow_throw` (cast steps); projectile 3 (group 222 `m.magic.icearrow_proj`: mesh, Stretch; effect delay 0, 500 ms in the reel); target `m.magic.icearrow_hit` (4-5); buffs 247 (freeze 4 s, `icearrow_dot2`), 2279 (strong freeze 30 s, `ice_wall_dot3`, framehold) | b | Arrow flight too short for aa-fx (cause 2). Ice on the target is buff-only (harness); the freeze also freezes the unit's animation. |
| 12796 | Magic Circle | Sorcery | cast `all_co_sk_spell_cast_shield[L]`; fire `all_co_sk_spell_launch_shield[L]` (plot 1267, 7 steps) | caster `m.magic.magic_reinforce_form`, `form2` (0-2), `launch` (3-5, decal), `throw`; target `magic_reinforce_hit` (4-5); summon steps 2196 / 2197 / 8063 (SpecialEffect 15: a server NPC spawn) | c | Ring at the caster not seen. The summoned unit is spawned by the server and cannot appear in the reel. |
| 14774 | Flame Barrier | Sorcery | plot 120 (9 steps): anim `all_co_sk_spell_launch_swich[L]` (33 s) | launch step `m.magic.firewall_launch` (columns, ParentStart children; round 7 fixed), voice; repeated move steps 1/2 and 2/2; buff 1987 (5 s, target): `m.magic.fireball_dot` | c | The columns are drawn by steps with no own effect rows; none seen at the caster. Round 7 notes the real column is still about twice as tall. The target dot is buff-only. |
| 10153 | Insulating Lens | Sorcery | cast `all_co_sk_spell_cast_shield[L]` 2 s; fire `all_co_sk_spell_launch_shield[L]` | caster `m.magic.mana_shield_form` (0-2), `form2` (1-2), `mana_shield_throw` (2-5), `mana_shield_launch` (3-5, decal, mesh); voice; caster buffs 95 / 426-429 (shield steps, `m.magic.mana_shield_dot`, `dot2`, FxSound) | b | The shield aura is buff-only (harness). The cast particles are small (unverified). |
| 10082 | Stealth | Shadowplay | no clip | caster `k.p_skill_vocation.hide_launch` (0-5); buffs 599 (45 s, `stealth` = yes; FxSound `hide_end` only) and the other stealth steps | b | `buffs.stealth` is read by nothing in `aa-client` (grep): the stealthed state has no look at all. The launch particle is not seen. Expected look unknown (needs a capture). |
| 10189 | Freerunner | Shadowplay | no clip | caster `k.p_skill_vocation.p_skill_vocation_speed` (0-5, OrientToVelocity); buff 340 (30 s, caster): `p_skill_vo_speed_dot` | b | The speed aura is buff-only (harness). |
| 12029 | Wallop | Shadowplay | fire `onehand_co_sk_fastattack_1[L]` (2H `twohand_..._1`, dual `twoswords_..._1`) | caster `k.p_skill_vocation.nanta_swoosh` (1-2); target `nanta_hit` (4-5, L2); voice; damage 591, SpecialEffects 850 / 1095 / 2171 | c | The swing shows; swoosh and target hit do not (cause 6). Needs a full-size frame. |
| 10723 | [Perform] Quickstep | Songcraft | no clip (no start, fire or instrument anim ids) | caster `k.p_skill_romance.song_of_speed_launch` (0-5, FocusAngle, mesh), `song_of_speed_form` (decal); buffs 656-660 and 2183-2187 (15 s, kind 3): `song_of_speed_dot2` on the hands | b | No performance pose in the data (a): the hand aura is the song buff's (harness). The lute is the game's own model (`objects/item/ins_s/ins_s_01_1.cgf`, item 6177). |
| 10724 | [Perform] Ode to Recovery | Songcraft | no clip | caster `romance` `song_of_res_form` (decal), `song_of_res_launch` (FocusAngle, OrientToVelocity, Stretch, decal, mesh); buffs 661-664 and 2189-2193 (15 s): `song_of_res_dot2` | b | As Quickstep: buff-only hand aura (harness). Caster decals unverified. |
| 11396 | [Perform] Bulwark Ballad | Songcraft | no clip | caster `romance.me_sg_form` (decal), `me_sg_launch` (FocusAngle, OrientToVelocity, Stretch), `romance_form`; buffs 778, 4386, 2199, 4387 (15 s): `me_sg_dot2` | b | Buff-only hand aura (harness). |
| 10727 | [Perform] Bloody Chantey | Songcraft | no clip | caster `romance.song_of_battle_launch` (AnimBlend, decal, Stretch), `romance_form`; buffs 665-667, 7662, 7664 (15 s): `song_of_battle_dot2` | b | Buff-only hand aura (harness). |
| 11961 | Alarm Call | Songcraft | no clip; channel tick 1000 | caster `romance.morning_bugle_launch` (0-6); buff 2362 (15 s, end `all_ac_stop`): `morning_bugle_launch_dot` (bone 1), `romance_form` | b | Buff-only aura (harness); the launch particle (FocusAngle, OrientToVelocity) is unseen. |
| 11934 | Startling Strain | Songcraft | instrument fire, by the held instrument: lute `music_co_sk_lute_launch[L]` (string 36), drum 402, pipe 404; projectile 82 (beam) to the target; plot 376 (13 steps) | caster `romance.song_of_charming_form`, `song_of_charming_throw`; sound-only `x2_sounds.particles.empty_continuous` (nothing by design); target buff 770 (2 s, stun): pose `onehand_re_dance[L]`, `song_of_charming_numb_dot` | b | The dance pose on the target is buff-only (harness). The caster performance shows only a spark on the sheet (c). |
| 11948 | Rhythmic Renewal | Songcraft | cast 2 s, instrument loop (lute `music_co_sk_lute_cast[L]` (33), drum (35), pipe (34)); fire of the held instrument, lute `music_co_sk_lute_launch[L]` (36) | caster `k.p_skill_love.cure_spring_form` (0-3, FocusAngle, OrientToVelocity, Stretch); sound-only `romance_008_launch`; interaction 221 (server heal) | c | Instrument poses and the spring particle: only a spark is seen. Needs a full-size frame. Paddle-shaped lute is the item model (see Quickstep). |
| 16783 | Infuse | Vitalism | cast `all_co_sk_spell_cast_swich[L]` (no cast time: unused); fire `all_co_sk_spell_launch_swich[L]` | caster `k.p_skill_love.vital_add_launch` (3-5, decal, Stretch; degraded); voice; restore mana effects; buffs 2952-2954 (1 s): `vital_add_hit` (target) | c | The caster launch is the only own visual; rendered but not seen. The 1 s target hit is buff-only (harness). |
| 16004 | Aranzeb's Boon | Vitalism | cast `all_co_sk_buff_cast_mental[L]` 2 s; fire `all_co_sk_buff_launch_mental[L]` | caster `k.p_skill_love.god_bless_form` (1-3), `god_bless_launch` (3-5, sparks); buffs 2955 / 2956 / 7661 (30 min, kind 1): aura `god_bless_dot` (bone 1) | b | The 30-minute aura is buff-only (harness). The orange spark on the sheet is the launch. |
| 10721 | Twilight | Vitalism | start `all_co_sk_buff_cast_mana[L]` (no cast time: unused); fire `all_co_sk_buff_cast_mental[L]`; channel 60 s (stop `all_co_sk_stop`) | `x2_sounds.particles.love_evening` and `love_evening_end` only: sound-only, nothing to draw by design | a | The data's visuals are the cast pose and a sound. The sheet shows the pose. |

## Notes and limits

* Verdicts are from data and code reading only. A "b" row marked "harness" would show in the real client when the server
  sends the buff; the skill data names that buff, but whether the server sends it was not checked. A "c" row needs a
  full-size frame or a real capture before it becomes a client bug.
* The 4-second channel cap and the reel's fake flights are harness settings (`tools/reel/plan.py`), not client behaviour.
* `data/extract/aa-client-combat/coverage_after.txt` lists "fire clips that loop in the data but are played once"; the code
  (`combat.rs` `play_clip`: `looping = kind == Act::Cast || clip_loops(name)`) loops them. The list is stale and was not used.
* The 43 rows cover the named skills. Other songs on `shots/reel/after2/09_songcraft_sheet.png` (Critical Discord, Dissonant,
  Hummingbird Ditty, Healing Hymn) were not classified.

## Recommended next steps (not done here)

1. Reel: inject the skill's buffs at cast time (the way `combat/sweep.rs` does with `book.skill_buffs(id)`, as `T:buff` steps)
   and rerun the 27 "b" skills. This decides most of them in one run.
2. Reel: use the real flight times (600 to 720 ms for arrows at 8 to 17 m, `docs/aa-client-combat.md`) and hold channel
   effects for the channel time (`CHANNEL_CAP` only for the sheets).
3. Database: 173 controller ids are missing from `compact.sqlite3`. Check whether the client's own data has them
   (Bull Rush 4433, Precision Strike 703, Mana Force 732) before the combat code changes anything.
4. aa-fx / aa-client: FxChr items (crow.chr first) are not played; either spawn the `.chr` or file it as a gap for aa-fx.
5. Look at full-size frames for the "c" rows (Precision Strike, Wallop, Boneyard, Flame Barrier, Telekinesis, Mana Force).

Output file: `docs\research\skill-novisible.md`.
