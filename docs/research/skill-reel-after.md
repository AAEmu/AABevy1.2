# Skill reel: after

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
