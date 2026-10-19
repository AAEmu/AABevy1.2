# Skill reel: after (final round, HEAD bab4c1a) and round 1 (c19c00f)

## Harness fixes after after3

* **Drop Back** (any reel): the `tp` to the open patch (15530, 15322) is now inside `skill_events`, so the Shadowplay tree reel casts it
  from the open spot like the motion reel (checked: it lands on open grass, no bushes).
* **The second doll** is not a summon and not Vicious Implosion's own: every `/spawn npc 13013` adds a runtime spawner that respawns
  its doll for good (`/despawn` does not clear them: the dolls come back), and every session spawns at the same spot (3 m ahead of a GM
  character parked there), so five or more dolls stand stacked on the stage. Vicious Implosion pulls units, which pulls one apart. The
  client now drops the other units of the target's model within 15 m when `AA_COMBAT_FAKE_ALONE=1` (set by `run_reel.py record`; a small
  hunk in `combat.rs` `fake_events`). The baseline exe has no such hook, so the baseline reels keep the stack (the doll is one; the pull is
  rare there). With the new exe the Auramancy test shows one doll after the pull.
* `compare` builds `all_skills_before_after.mp4` from trees 1 to 10 only (the motion reel is a separate file).

## Final round: `after3` at 421fc70 (harness v2)

All ten trees, the motion reel and the two ground reels recorded with `target/agents/reel/after3/aa-client.exe` (HEAD `421fc70`, deleted
afterwards) and harness v2 (buffs, landing times, 8 s channels). Videos (git-ignored): `shots/reel/after3/NN_<ability>.mp4` + `_sheet.png`,
`all_skills.mp4`, `11_motion.mp4`, `12_ground_missile_rain.mp4`, `13_ground_meteor_strike.mp4`; side by side with the baseline in
`shots/reel/compare3/` (trees and motion; `all_skills_before_after.mp4` is not built because the motion reel is in the list: use the
per-tree files); player stills (1600x900, sheathed and drawn weapon, full / mid / close, front / side / back) in
`shots/reel/after3/stills/` (`tools/reel/stills.py`). The after2 and compare2 videos were deleted. Capture: 30 fps fixed timestep,
17 to 36 rendered frames/s (the first reel 17 while shaders compiled), 40 to 76 s wall per reel plus about 15 s startup, about 14 minutes
for everything; `aa-dev: fps=30.0` in every log, no stall.

### What changed since after2 (from the sheets)

* Buffs show now (harness v2 and the client fixes): Earthen Grip's rubble at the doll, Bubble Trap's bubble around the doll, Insidious
  Whisper's fear smoke, Play Dead lies down, Snare's vines hold the doll, Freezing Arrow's ice on the doll, Stalker's Mark, Stealth (a
  pink outline instead of the model), Toxic Shot's cloud, Mocking Howl / Imprison / Redoubt / Toughen / Revitalizing Cheer auras.
* Beams are steady and run from the chest to the doll: Crippling Mire (orange), Absorb Lifeforce (green), Mana Force (purple, the
  caster is pushed back), Chain Lightning; Arc Lightning has its swirl at the doll; Summon Crows' crows fly at the doll.
* Songcraft: the lute is in the hand and the performances show: Quickstep's rings, Ode to Recovery's arcs, Bulwark Ballad, Startling
  Strain's pink burst on the doll. The staff goes onto the hip after a weapon skill; Bull Rush no longer dashes; Lasso pulls.
* The caster is clothed and shaded correctly (hair, skin, cloth); short-lived launch effects are drawn.

### Still looks wrong (after3)

* Oversized or odd geometry: Hell Spear (a huge red and white lattice over the stage), Ollo's Hammer (big blue burst, cyan blocks),
  Freezing Earth (wide water disc around the caster), Sunder Earth (dust and rock mass), Summon Wraith (a huge dark red cloud
  over the whole scene), Retribution (flame sheet around the caster), Whirlwind Slash (a long blade).
* Little or nothing visible: Bondbreaker, Frenzy, Precision Strike, Battle Focus (small glow), Shield Slam, Purge, Dahuta's Breath,
  Focal Concussion, Vicious Implosion, Teleportation (caster does not move in the tree reel), Meditate (no seated pose), Protective
  Wings (a hand orb, no wings), Leech, Telekinesis (a pink hand glow, no beam to the doll), Stillness (a thin ring), Endless Arrows
  (no arrows), Float (no levitation), Boneyard, Charged Bolt, Flamebolt (small blue flicker), Magic Circle, Insulating Lens,
  Infuse, Aranzeb's Boon, Twilight, Mend, Alarm Call, Hummingbird Ditty, Rhythmic Renewal, Bloody Chantey.
* aa-fx round 14 (real client on my account, `shots/aa-fx/r14/`, `docs/research/skill-audit-fx.md` round 14), per skill:
  * Ollo's Hammer: matches real, see `cmp_hellspear_ollo.png` (the real hit is a big white blue burst and a 5 m blue striped dome).
  * Freezing Earth: matches real, see `cmp_freezing_sunder_retribution.png` (real: turquoise ripple and a white mist 8 m wide; the 10 to 13 m decal is the data's).
  * Sunder Earth: matches real, see `cmp_freezing_sunder_retribution.png` (`ground_launch` alone is nearly invisible in the real client too; the dust and rocks
    come from `ground_launch_standalone*`, which the combat code spawns in the air).
  * Retribution: matches real, see `cmp_freezing_sunder_retribution.png` (a 2 m red orange glow with sparks).
  * Hell Spear: was ours (a white and red decal lattice hid the red glow): the data's decal is black (`decal_death_pattern_black`), fixed in round 14; real:
    a 5 m red glow sphere, `cmp_hellspear_ollo.png`, in game `hs_game.png`.
  * Summon Wraith: the size matches real (8 m x 2.5 m cloud); only the colour was too dark (see round 14); Boneyard's brown dust is the same lit smoke
    brightness, fixed. Precision Strike, Battle Focus, Shield Slam, Dahuta's Breath, Focal Concussion, Charged Bolt are that faint in the real client too;
    Bondbreaker, Purge, Frenzy, Vicious Implosion render like the real client but only for 0.2 to 1 s. Protective Wings: the aura and the ring are the
    channel doodad's clout (buff 258, projectile 51), which aa-client does not spawn.
* Wrong place: Searing Rain and Frigid Tracks at the caster (fake events: no ground position in the tree reels), Flame Barrier's flames
  on the caster, Thwart's light on the caster, Renewal's pink streak at the caster.
* Ground reels: the reticle and Meteor Strike on the doll look right; Missile Rain shows the ring and the pose but no arrow rain
  (`long_distance_shot_launch` missing in the library); in the motion reel (fake aim at 3 m) Missile Rain is still a very large red
  ribbon lattice around the caster.
* Reel artifacts: Drop Back in the Shadowplay tree reel ends in the bushes (cast from the stage spot; the motion reel casts it from an
  open patch); a second doll stands beside the first from Vicious Implosion on in the Auramancy reel (another session's doll or the pull);
  the doll plays no hit reaction.

## Reel harness v2 (after 4c8bb5c; used by the next re-record)

`tools/reel/plan.py` now sends what the server would, so auras, poses, freezes and stuns show (the triage in `skill-novisible.md` found
about 24 of 27 "we fail to show it" rows were the reel):

* **Buffs**: `T:buff[@self]:INDEX:BUFF:0` / `T:unbuff[@self]:INDEX` steps (the existing fake ops, same event as the audit sweep's
  `BuffCreated`), from the DB: the `skill_effects` BuffEffect rows at the lowest level that shows something (rows that need a buff tag on
  someone are left out), the skill's channeling / toggle buff, and for plot skills the BuffEffect rows of each plot step (plot target 1 =
  caster). A buff lands on the doll, on the caster for a self skill, or on the caster when its row is for friends only and the skill is
  hostile. Only buffs with an aura, start pose, framehold or ragdoll are sent. A buff stays on for its duration, at most 3.5 s (a
  channel buff up to `CHANNEL_CAP`), and the skill's slot lasts that long.
* **Flights**: `fire` delay = `effect_delay` + distance / `effect_speed` (the server's landing time; the client plays the projectile for
  `delay - effect_delay`), and the hit lands when the shot lands. The doll stands 4 m away, so a projectile skill is timed for 10 m
  (`SHOT_RANGE`, real shots fly 8 to 17 m); speedless projectiles take 500 ms. Plot steps' projectiles fly by their own data speed over the
  real 4 m (Endless Arrows: about 0.1 s) and cannot be lengthened from the reel.
* **Channels**: `CHANNEL_CAP` 4 s -> 8 s (Meditate 9, Invincibility 12, Telekinesis 12, Play Dead and Twilight 60 are cut at 8).
* A hostile skill whose target type is "any" (Snare) now aims at the doll, not the caster (`relation == 4` is not a self skill).
* Reel lengths grow by about 25 percent (35 to 66 s per tree); the ground reels have no buffs (real server flow).

Test captures at HEAD with the working tree of that moment: Play Dead lies down (a body at the caster's feet; the standing model stays, a
client matter), Bubble Trap flies about 1 s and the bubble and splash show at the doll, Meditate's pink aura shows over the 8 s channel.

## Final round: `after2` at bab4c1a

The same 110 skills, the motion reel and two new ground-target reels, recorded with `target/agents/reel/after2/aa-client.exe`
(HEAD `bab4c1a`: af75c03 staff stance and hang, effect areas on the ground, Meteor Strike at the target; bab4c1a aa-fx round 10,
beams stretch hand to target). The exe copy is deleted again after the recording.

* Videos (git-ignored): `shots/reel/after2/NN_<ability>.mp4` + `_sheet.png`, `shots/reel/after2/all_skills.mp4`,
  `shots/reel/after2/11_motion.mp4`; ground reels `shots/reel/after2/12_ground_missile_rain.mp4`, `13_ground_meteor_strike.mp4`;
  side by side with the baseline: `shots/reel/compare2/NN_<ability>_before_after.mp4`, `11_motion_before_after.mp4`,
  `all_skills_before_after.mp4` (the ground reels have no baseline: the baseline client has no ground targeting).
  The round 1 videos (`after/`, `compare/`) and the first-stage baseline were deleted to save disk.
* Capture: 30 fps fixed timestep, 23 to 37 rendered frames/s (14 to 20 for the ground reels, which render the world from above);
  25 to 45 s wall per reel plus about 15 s startup; all ten reels, the motion reel and the ground reels took about 17 minutes.
  `aa-dev: fps=30.0` in every log, no watchdog stall.
* `run_reel.py setup` ran before each batch (the training doll is killed by other sessions); the stage is the same as the baseline.
* Ground reels (`plan.py` ids 12 and 13): the real flow, no fake skill events: `--ground-press SKILL`, `--aim-world X Y`
  (the cursor held on a world point), `--click`, so the server answers and the world-space reticle is drawn. Missile Rain is aimed
  11.5 m ahead (its minimum is 10 m), Meteor Strike on the doll 4 m ahead. The camera is a fixed orbit (yaw 0, pitch -35, 8 m) from above
  and behind: a camera relative to the facing at 5 m has the ring behind it or inside the grass.

### What changed since round 1 (visible in the sheets)

* The ground reticle is drawn (green ring on the ground, follows the aimed point). Meteor Strike now lands on the aimed spot
  (crater, flames, ash), not on the caster.
* Staff casts: the arms no longer stretch out sideways with the staff across both hands; the staff hangs on the back like the bow
  (Witchcraft, Auramancy, Occultism, Sorcery, Vitalism casts look like casts).
* Beams run hand to target: Absorb Lifeforce (green beam to the doll), Crippling Mire (orange streak to the doll), Chain
  Lightning (white arcs to the doll), Mana Stars (orange glow at the doll). Summon Wraith's red smoke is a cloud around the
  caster, not 50 m away.
* Shadow Step and Teleportation blink in the tree reels too (the caster ends up elsewhere, the camera follows); Drop Back in the
  motion reel is cast from an open patch and no longer ends in the bushes.

### Still looks wrong (final round)

* No visible effect (pose or cast clip only): Battlerage Bondbreaker, Frenzy, Precision Strike; Witchcraft Bubble Trap, Insidious
  Whisper, Purge, Banshee Wail, Dahuta's Breath, Play Dead (no lie-down); Defense Shield Slam, Bull Rush (no dash), Redoubt,
  Invincibility; Auramancy Vicious Implosion, Meditate (no seated pose), Leech, Protective Wings (no wings, a glow at the hand);
  Occultism Mana Force, Telekinesis (no beam to the doll), Stillness, Summon Crows; Archery Endless Arrows (no arrows), Snare, Float,
  Boneyard; Sorcery Freezing Arrow (no projectile), Magic Circle, Flame Barrier, Insulating Lens (tiny); Shadowplay Stealth,
  Freerunner, Wallop; Songcraft most songs and performances (Quickstep, Ode to Recovery, Bulwark Ballad, Bloody Chantey, Alarm Call,
  Startling Strain and Rhythmic Renewal only a spark), the lute is still a paddle shape; Vitalism Infuse, Aranzeb's Boon, Twilight.
* Checked after bab4c1a: Arc Lightning is only a faint ring at the caster's feet (no arc to the doll); Lasso shows a green glow at the
  hand but no rope; Telekinesis and Mana Force show nothing; Crippling Mire still has the black ring around the caster besides the
  streak to the doll; Absorb Lifeforce and Chain Lightning are right.
* Oversized or odd geometry: Whirlwind Slash (very long thin blade streak), Sunder Earth (large rock and dust mass at the caster's feet),
  Ollo's Hammer (a big blue burst and cyan blocks over the doll), Hell Spear (a large red and white lattice covering the stage),
  Freezing Earth (a wide water-like disc around the caster), Skewer (a large white mist cloud), Retribution (flame sheet
  around the caster).
* Wrong place: Searing Rain, Frigid Tracks (orange flames, wrong colour for frost) and the fake Meteor Strike of the tree reel are
  drawn at the caster (the tree reels use fake events without a ground position; the ground reel shows the real thing); Mocking Howl and
  Imprison are drawn around the caster.
* Ground reels: Missile Rain shows the ring, the cast pose and a faint blue spark, but no arrow rain at the ring, and the log says
  `abillity_skill_table_m.wild.long_distance_shot_launch not found in library`; the effect is missing in the data library. In the
  motion reel (the old fake aim at 3 m) Missile Rain is still a very large red ribbon lattice near the caster.
* The caster's hit reaction cannot be judged (the doll plays no hit clip).

## Round 1: HEAD c19c00f (videos deleted)

The same reel as `skill-reel-baseline.md` (110 skills of the ten ability trees on a training doll, plus the extra motion reel),
recorded with the client built from HEAD `c19c00f` (aa-dev recorder deadlock fix) after the skill-fix rounds: 1a7b508, aa-fx rounds
6 to 8 (b28464b, 19eaa15, f58289f), ground targeting f7d00e1, skill movement 0ff8d1f, combat.rs repair 99c7dae.

* Videos: `shots/reel/after/NN_<ability>.mp4` (+ `_sheet.png` 4x4 contact sheets), `shots/reel/after/all_skills.mp4`,
  `shots/reel/after/11_motion.mp4`; side by side: `shots/reel/compare/NN_<ability>_before_after.mp4`,
  `shots/reel/compare/11_motion_before_after.mp4`, `shots/reel/compare/all_skills_before_after.mp4`. Git-ignored.
* Build: `target/agents/reel/after/aa-client.exe` (+ `commit.txt` = c19c00f, `status.txt`, `uncommitted.diff`: other sessions'
  uncommitted edits in aa-anim, aa-cry, aa-world, aa-audio were in the tree when it was built).
* Same stage, camera, weapons and event scripts as the baseline (`tools/reel/plan.py`, `tools/reel/run_reel.py`), with two
  additions: after every skill a `T:tp:X:Y:YAW` fake event puts the caster back on the stage spot (the moving skills Charge, Bull
  Rush, Drop Back, Overwhelm, Shadow Step carry the caster off, and Drop Back put the camera into the bushes for the rest of the
  Shadowplay reel); the baseline exe ignores `tp`, so its reels are unchanged. `tp` is in `combat.rs` (`fake_events`).
* Capture: 30 fps fixed timestep, 25 to 37 rendered frames/s (about 35 to 40 s wall per 30 to 50 s reel, plus about 15 s startup and
  a few seconds of ffmpeg; the 10 reels, the motion reel and the compare videos took about 20 minutes). No watchdog stall, no hang.
  All logs report `aa-dev: fps=30.0`.

## Harness notes

* The ground reticle is **not visible** in the motion reel. The scripted flow works (`--ground-press` aims, the cursor goes to the
  doll, the click casts, the server answers `SkillStarted` / `SkillFired` with the position target; Missile Rain is out of range at
  3 m, its minimum is 10 m, so it stays red and is dropped, as designed), but no decal is drawn in any frame, with or without HUD. The
  reticle is a 2D mesh on the plate camera's layer (`ground_target::reticle`); it hides itself when any ring point is behind the
  camera, and the ring is 10 m wide (Meteor Strike radius 5 m) with the reel's camera 5 m behind the caster. At a 9 to 14 m camera
  (no-clip) it was not visible either, so check the plate camera under `--no-hud` and the decal draw order; not investigated further.
* The real Meteor Strike cast of the motion reel hits the doll for real: the doll can die and the stage needs `run_reel.py setup`
  again before the next reel.
* The after `tp` first set the facing as 270 degrees; code that compares facings without wrapping then turned the caster the
  other way and the camera showed the stage from the opposite side. `tp` now wraps into (-pi, pi].
* The caster's weapon pose limits of the baseline (staff across both hands, fake hit timing, a doll that does not react) still apply.

## Fixed since the baseline (visible in the sheets)

* Moving skills move: Charge, Overwhelm, Shadow Step travel (the caster ends up next to the doll), Lasso lunges; Drop Back jumps back (and the camera is
  inside a bush until the caster is put back). Teleportation still shows no visible move.
* Ground skills (Meteor Strike, Missile Rain) now put a crater, flames and arrows on the ground with a fall of fire on the target spot.
* Gone: Pin Down's giant cyan beam, Antithesis' white rectangles (now a pink ring), Skewer's large white swirl, Earthen Grip's black
  blob (now a brown textured mound), Retribution's black slab (now a red flame lattice), Comet's Boon's horizontal beams (now white
  web lines on the ground), Stalker's Mark (target ring on the doll), Toxic Shot (green cloud at the doll), Mana Stars, Lassitude
  and Enervate (blue glow at the doll), Boastful Roar (green cloud at the doll).

## Still visibly wrong, per skill (from the after sheets; full size frames only for a few)

"None visible" = nothing readable at 1280x720 on the caster or the doll in the slot; some have particle fx in the data that may be
tiny or out of frame, so check the videos before filing a bug.

**Battlerage**: Whirlwind Slash still draws a very long thin blade streak past the doll (trail much too long); Sunder Earth is a huge
black rock and smoke mass at the caster's feet; Lasso shows no rope; Terrifying Roar only a faint orange ground glow; Bondbreaker,
Precision Strike, Frenzy, Battle Focus: none visible (a faint glow on the hands). Triple Slash, Charge, Tiger Strike look right.

**Witchcraft**: Play Dead has no lie-down pose; Bubble Trap, Insidious Whisper, Purge, Banshee Wail, Focal Concussion, Dahuta's
Breath: none visible; arms held out sideways with the staff across them for all casts (T-pose-like); Earthen Grip appears beside
the caster, not at the doll.

**Defense**: Shield Slam, Bull Rush (no dash, no impact), Redoubt, Invincibility (pose only): none visible; Ollo's Hammer is still
flat cyan blocks and a wide blue beam across the whole scene; Mocking Howl (red smoke) and Imprison (green aura) are drawn around
the caster, not the doll.

**Auramancy**: Teleportation (no movement), Vicious Implosion, Leech, Liberation: none visible; Meditate has no seated pose;
Protective Wings has no wings (a small glow at the hand); Conversion Shield only white lines at the doll's feet; Thwart (light at
the caster, not the target); arms out sideways with the staff.

**Occultism**: Mana Force, Telekinesis (no beam to the doll), Stillness, Summon Crows, Summon Wraith (no wraith) show nothing or
almost nothing; Crippling Mire is a black ring around the caster, not at the doll; Hell Spear is a very large red and white
circle with lines that covers the stage; Urgency has the caster crouched under orange beams; Absorb Lifeforce is a tiny green wisp.

**Archery**: Endless Arrows (no arrows), Snare, Float, Boneyard: none visible; Concussive Arrow and Missile Rain (fake events) draw the
bow but show no arrow or rain at the doll; Double Recurve only an orange glyph above the caster; Deadeye pose only; Charged Bolt's
arrow is small. Piercing Shot and Intensity look right.

**Sorcery**: Freezing Arrow has no projectile; Freezing Earth is still a wide water-like disc around the caster; Searing Rain,
Frigid Tracks (orange flames: wrong colour for frost) and the fake Meteor Strike are drawn at the caster, not at the target spot;
Arc Lightning, Magic Circle, Flame Barrier, Insulating Lens: none visible or tiny; Flamebolt is a small blue flicker at the hand
only. Chain Lightning looks right.

**Shadowplay**: Stealth, Freerunner, Shadow Step (in the ten tree reel the caster stands still: no teleport; it travels only in the
motion reel), Pin Down (no beam now, caster kneels), Wallop: none visible; Drop Back puts the camera in a bush while the caster is
mid air; Rapid Strike, Overwhelm, Shadowsmite look right.

**Songcraft**: Most songs and performances show nothing (no notes, no aura): Quickstep, Ode to Recovery, Bulwark Ballad, Bloody
Chantey, Alarm Call, Rhythmic Renewal (faint spark), Hummingbird Ditty (faint); the lute is still a flat paddle shape; Dissonant is
a thin line. Healing Hymn, Critical Discord and Startling Strain look right.

**Vitalism**: Infuse, Twilight, Aranzeb's Boon (faint orange glow), Mend and Renewal (faint pink): small; Revive is a green ring only;
arms out sideways for most casts. Resurgence, Fervent Healing, Mirror Light look right.

**Motion reel** (Meteor Strike, Missile Rain, Charge, Lasso, Overwhelm, Shadow Step, Teleportation, Drop Back; no HUD): the reticle
is not drawn (see the harness notes); Meteor Strike's crater and flames are centred about 3 m from the caster, so the caster
stands inside the burning ring and the camera is pulled in close; Missile Rain is out of range at the doll; Charge, Lasso, Overwhelm
and Shadow Step move the caster next to the doll and the doll burns afterwards (the hit effect); Teleportation shows no move;
Drop Back throws the caster backwards through the bushes (camera inside the leaves).
