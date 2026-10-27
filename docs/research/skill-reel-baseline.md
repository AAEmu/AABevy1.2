# Skill reel: baseline

Every shown skill of the ten ability trees (110 skills), cast on a training doll, recorded as it looks before the skill-fix
work. Videos: `shots/reel/baseline/NN_<ability>.mp4` (1280x720, 30 fps, H.264 crf 20, captioned `<tree>: <skill>`),
`NN_<ability>_sheet.png` (4x4 contact sheet: one frame 0.8 s into each skill, then a second frame of the longest ones) and
`all_skills.mp4` (all ten in a row). Both `shots/` and `data/extract/reel/` are git-ignored.

## How it was made

* Build: `target/agents/reel/baseline/aa-client.exe`, git `77d3f01` plus the uncommitted tree of that moment (other sessions'
  edits to aa-anim, aa-cry, aa-world and aa-net, kept in `uncommitted.diff` and `status.txt` beside it), plus the reel hooks
  (aa-dev `--record`, `AA_COMBAT_FAKE_TARGET`, `@self` / `@ground`). Built from a snapshot checkout with hard-linked
  deps (`tools/reel/linkfarm.py`), so the shared tree's later skill-fix edits are not in it.
* Scene: Nuian male `Reelcasterx` (account `bevyreel9`, access level 100 for the GM `/spawn`), the open dirt patch at
  (15530, 15325) near the spawn pond, hour 12, no HUD, camera 5 m at -90 degrees from the facing direction, fov 60. Target: npc
  13013 (straw training doll, model 1218) 4 m ahead. The "training dummy" 7512 has a prefab model that the client does not draw,
  so it cannot be used. The sky is the zone's overcast weather at hour 12.
* Events: `tools/reel/plan.py` writes `AA_COMBAT_FAKE` scripts from the database (cast, fire, channel, plot steps walked through
  `plot_next_events` with the wielded weapon class; no real server cast). Weapon per tree: two handed sword (Battlerage), staff
  6102 (Witchcraft, Auramancy, Occultism, Sorcery, Vitalism), sword and shield (Defense), bow (Archery), dual swords
  (Shadowplay), lute (Songcraft). About 2.5 s per skill, longer for cast + channel + flight (channels cut at 4 s).
* Fixed timestep: `aa-dev --record` advances the clock exactly 1/30 s per captured frame. Capture ran at 24 to 38 rendered
  frames/s (30 to 55 s wall for 30 to 50 s of video, plus about 15 s startup and a few seconds of ffmpeg).
* Reproduce: `python tools/reel/run_reel.py setup` (spawns the doll after a server restart), then
  `data/extract/reel/venv/Scripts/python.exe tools/reel/run_reel.py baseline|after [ability numbers]` and
  `... compare` for before | after.

## Harness limits (not skill bugs)

* Fake events carry no server-side result: no damage numbers, no real hit timing (a `hit` is sent right after the fire / plot step).
* Plot variants (weapon class, combo stage) are chosen by a small simulation of the plot conditions, not by the server.
* The doll does not react (it has no hit clip), so "hit reaction" cannot be judged here.
* The mage staff is carried in both hands horizontally in the cast poses; whether the real client draws it like that is unchecked.

## Findings (first pass from the contact sheets and a few full size frames)

"No visible effect" means nothing readable at 1280x720 on the caster or the doll during the skill; some of those have particle
effects in the data (`skill_coverage.tsv`) that are small or hidden, so the audit should check them in the full video.

**Broken-looking geometry or scale**

* Retribution (Occultism): a tall black and orange slab of flat quads with white tile artifacts instead of a flame pillar.
* Earthen Grip (Witchcraft): a black untextured blob next to the caster (should be at the target).
* Sunder Earth (Battlerage): a huge black rock mass with smoke in front of the caster.
* Antithesis (Vitalism): large semi-transparent white rectangles with hard edges.
* Ollo's Hammer (Defense): flat cyan rectangles at the doll.
* Whirlwind Slash (Battlerage): a long thin orange streak running past the doll (trail much too long); Pin Down (Shadowplay): a giant
  diagonal cyan beam; Stillness (Occultism): a very large ring / dome arc at the left edge; Skewer (Vitalism): a very large white
  swirl at the doll; Freezing Earth (Sorcery): a wide water-like disc around the caster.

**Effect at the wrong place**

* Ground and area skills put their effect on the caster instead of the target spot: Meteor Strike (the fake event does carry the
  target position; the rubble still lands at the caster's feet), Missile Rain, Crippling Mire (black ring around the caster), Arc
  Lightning (cracks at the caster), Thwart (light pillars far left and right of the caster). Check whether the client uses
  `SkillTarget.pos` for fx location 2.
* Comet's Boon (Auramancy): horizontal white beams through the caster.

**Pose problems**

* Arms held straight out sideways, T-pose-like, with the staff across the hands, for most staff casts: Teleportation, Conversion
  Shield, Vicious Implosion, Meditate, Protective Wings (Auramancy), Purge, Insidious Whisper, Bubble Trap (Witchcraft), Mana Force,
  Summon Crows, Telekinesis, Absorb Lifeforce, Summon Wraith (Occultism), Insulating Lens, Magic Circle (Sorcery) and most Vitalism
  casts. No lie-down for Play Dead (dark smoke only), no wings for Protective Wings, no wraith for Summon Wraith, no seated pose for Meditate.
* Urgency (Occultism): the caster crouches / falls into a heap under orange beams.
* Absorb Lifeforce (Occultism): the bow comes into the hand during a staff cast.

**Skills with no visible effect (clip plays, nothing else)**

* Battlerage: Terrifying Roar, Bondbreaker, Frenzy, Battle Focus (only a faint glow on the hands), Precision Strike.
* Witchcraft: Enervate and Lassitude (tiny), Bubble Trap, Insidious Whisper, Purge, Focal Concussion, Dahuta's Breath.
* Defense: Shield Slam, Bull Rush (no dash, no impact), Invincibility (pose only), Redoubt, Toughen, Revitalizing Cheer (faint).
* Auramancy: Teleportation (no movement, no effect), Conversion Shield, Vicious Implosion, Meditate, Protective Wings, Leech.
* Occultism: Mana Stars, Mana Force, Summon Crows, Absorb Lifeforce, Telekinesis (no beam to the doll), Summon Wraith.
* Archery: Endless Arrows (no arrows), Charged Bolt (arrow not seen in flight), Snare, Float, Boneyard, Missile Rain.
* Sorcery: Freezing Arrow (no projectile), Magic Circle, Flame Barrier. Flamebolt flies correctly.
* Shadowplay: Stalker's Mark, Stealth, Freerunner, Shadow Step (no teleport), Toxic Shot (hit glow only).
* Songcraft: most songs and performances (a paddle-shaped lute in hand, no notes or auras): Startling Strain, Quickstep, Ode to Recovery,
  Bulwark Ballad, Bloody Chantey, Alarm Call, Rhythmic Renewal.
* Vitalism: Mirror Light, Revive (ring only), Infuse, Aranzeb's Boon.

**Looks right, for comparison**: Triple Slash, Charge (clip and beam), Refreshment, Courageous Action, Hell Spear, Intensity,
Resurgence, Flamebolt, Piercing Shot (blood spray), Overwhelm, Drop Back, Shadowsmite, Critical Discord (sound rings).

## Not covered

Default (basic) skills and emotes, the hidden sub skills, the races other than Nuian male, mounted or swimming casters.
