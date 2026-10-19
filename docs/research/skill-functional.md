# Skill functional test against the real server

Goal: every player skill must work against the lane server (`bevy`: login 27237, game 27239), not only look right with fake events.
This is the result of casting all 110 shown skills of the ten ability trees, the default skills, emotes and ground skills with a
real `aa-net` client and checking the packets that come back, plus a sample of the same skills in the real `aa-client`.

**Result: 109 of 110 tree skills pass end to end; 1 (Revive) cannot be tested by one player alone. All 14 default, emote and
ground casts pass. No packet was left unhandled and none failed to decode in the final runs.** Two packets that were unhandled in
the first runs (`SCICSCashPoint` 0x1d6 and `SCAggroTargetChanged` 0x1bd) are decoded now. One HUD bug (no cast bar for a channel without
a cast time) was found and fixed. No skill failed because of the server; the server notes below are about behaviour around the skills.

## How it was run

* Driver: `crates/aa-net/examples/skilltest.rs` (Rust, built on `aa_net::Client`). It logs in the GM character `Skilltestr` of account
  `bevyskilltest` (`access_level` 100 in `lane_bevy_game.accounts`), sets `/level 50` and `/ignoreskillcds true`, swaps the tree into
  the active abilities (`SwapAbility`, `ResetSkills` for the other two so the skill points last), learns the tree's skills (`LearnSkill`),
  puts a training doll or a real hostile next to the character and casts each skill in turn (selected target, self, or a ground spot
  about 7 m ahead for target type 6). For each skill it records the packets until the skill is over (cap 6 s, then `CSStopCasting`)
  and writes one JSON line: events by kind, damage, heals, buffs created and removed, refusals, unhandled and failed packets, cast state at the end.
* Names, target types, cast and channel times, cooldowns and expected effect kinds come from `compact.sqlite3` (`skills`, `skill_effects`,
  `effects`).
* Scripts in `tools/skilltest/`: `run_trees.sh` (one process per tree), `retry.sh` (login retry while the server holds the old session),
  `exp.sh` (the follow up experiments), `summary.py`, `show.py`, `analyze.py` (verdicts by rule), `report.py` (the tables below;
  manual verdicts are in `notes.json`), `client_sample.sh` (the real client).
* Passes: `d` (final: hostile target selected, dry ground, after the aa-net fixes), `a` (self or friendly target, in the water, for the
  friendly skills that refuse a hostile target), `c` (same as `d` before the fixes), `b_defaults`, `b_emotes`, `b_ground`, and
  `e1` to `e8` (experiments below). Results: `data/extract/skilltest/*.jsonl` and `.log` (git ignored).
* A skill passes when: the cast was accepted (or refused for the right reason), a `SkillStarted`, `SkillFired` or `PlotEvent` came back,
  it ended (`SkillEnded`, `PlotEnded`, `CastingStopped`), the effects the data lists show up (damage, heal, buff), nothing was unhandled
  and the cast state ends clean. Numbers like `Buff+ x200` are not all ours: the test spot is crowded (dolls, bats, foxes) and every
  nearby unit's buff packets count too.
* Run one tree: `MSYS_NO_PATHCONV=1 cargo run -p aa-net --example skilltest -- --user bevyskilltest --character Skilltestr --trees 6 --at 15530,15323,131.4`
  (add `RUST_LOG=aa_net=trace` for the wire trace).

## Results per tree

| tree | pass | skills |
|---|---|---|
| 1 Battlerage | 11 | 11 |
| 2 Witchcraft | 11 | 11 |
| 3 Defense | 11 | 11 |
| 4 Auramancy | 11 | 11 |
| 5 Occultism | 11 | 11 |
| 6 Archery | 11 | 11 |
| 7 Sorcery | 11 | 11 |
| 8 Shadowplay | 11 | 11 |
| 9 Songcraft | 11 | 11 |
| 10 Vitalism | 10 | 11 |
| total | 109 | 110 |

Notes on the verdict column: `PASS` means the rules above held. "channel" skills run until the driver stops them after 6 s: the stop
(`CSStopCasting`) comes back as `CastingStopped` or `PlotStopped` plus `SkillEnded` or `PlotEnded`, so cancelling a channel works. Friendly
skills (heals, buffs, resurrections) refuse a hostile selected target with "no target" (0x09), which is right; their row is from the
self or friendly pass.

## Follow up experiments

| run | what | result |
|---|---|---|
| e1 hostile dummy | the skills that did not show damage, on the 7512 training dummy | Earthen Grip, Bubble Trap: still no damage (see below); Concussive Arrow, Antithesis: damage and heal as expected |
| e5, e6, e8 real hostile | `--npc 0`: the nearest living faction 115 mob, with a wire trace | Bubble Trap and Earthen Grip: the server sends no `SCUnitDamaged`, only the buffs; Enervate then Earthen Grip: no damage either (the debuff had run out by then) |
| e2 cooldowns | `--keep-cooldowns --repeat`: each skill cast twice at once, then after its cooldown | Charge (12 s) and Charged Bolt (9 s): the recast right away is refused with 8 (cooldown), accepted after the cooldown. The server does not send `SCCooldowns`; the client's estimate from `skills.cooldown_time` was right. Freezing Arrow (6 s) and Antithesis (5 s) had already run longer than that |
| e3 revive | `--corpse` | Revive on a living target: 0x06 target alive. A dead player is needed for the real path |
| e4 unlearned | `--no-learn`: Precision Strike and Charged Bolt without learning them | the server runs them. The 1.2 client refuses them first ("You haven't learned this skill yet", `URK_TRAINED_SKILL`), and `aa-client` does the same |

Earthen Grip (14376) and Bubble Trap (10154): the damage effects are conditional in the data. `skill_effects.target_buff_tag_id` of
Earthen Grip's damage effects is 27 (a rooted target) and 373 (the mind-destruction debuff of Enervate, buffs 101 and 2835); Bubble Trap's
damage needs tag 121 (lightning damage state) and its buff effects branch on tag 119 (fire damage state). On a plain target the server
applies only the buffs, which is the data. These are combos; they were not cast in the right order inside their buff windows.

## Defaults, emotes and ground casts

Ground casts send `CSStartSkill` with a position target (flag byte 1, two `<< 32` fixed point 1/4096 m longs, f32 height, f32 rotation,
two zero object ids); the server echoes the spot in `SCPlotEvent` rounded to 1/4096 m. Unit test
`skill::tests::ground_cast_bytes_follow_the_server_reader`. Dash and Recall are refused with 0x48 (`skill_urk_combat`: the unit
requirement "not in combat") while a mob is fighting the character, and the dances answer 0x15 (not while swimming) in the water: these
are the game's rules; only the names were missing in aa-net. The table is at the end of this document.

## Real client sample

`tools/skilltest/client_sample.sh` learns the tree with the driver, then runs a copy of `aa-client` (`target/agents/skilltest/`) with
`--enter-world --account bevyskilltest --character Skilltestr`, four skills on bar slots 1 to 4 (`--bar-skill`), `--hud-key tab` and
`--hud-key slotN`, `AA_COMBAT_LOG=1`, and a screenshot (`shots/skilltest/`, git ignored). Free RAM was 7 to 10 GB each time.

| tree | skills | what was checked | result |
|---|---|---|---|
| 6 Archery | Concussive Arrow, Charged Bolt, Freezing Arrow, Endless Arrows | cast bar of a 1.5 s cast, damage numbers, buff icons, cooldown dimming | the "Concussive Arrow" bar fills; yellow damage numbers over the target; buff icons appear (including Grit x3000, buff 864, `max_charge` 3000, from the hits the crowd lands); used slots dim |
| 3 Defense | Invincibility (12 s channel, no cast time) | channel bar | before the fix: **no bar**. After the fix an "Invincibility" bar 3 s in, emptying; the buff icon with its time appears |
| 1 Battlerage | Triple Slash, Charge | plot events | `PlotEvent`s with a clip per step, damage numbers, Charge asked and answered |
| 10 Vitalism | Antithesis | cast and heal | `SkillStarted`, `SkillFired` 1.9 s later, heal shown |
| 10 | Health Lift, Protective Wings (not learned in that run: other trees) | client refusal | HUD alert `URK_TRAINED_SKILL` "You haven't learned this skill yet", nothing sent |

No `Unhandled` and no decode failure in any client log. The warnings that did show (`aa-fx` textures, `tab_language.alb:178`) are not
skill related.

## Failures by class

(a) Client bug in aa-net (encoding, decoding). All fixed, with tests:

* `SCICSCashPoint` (0x1d6, an `i32`) was an unhandled packet 1.4 s after the login and showed in every run: now `Event::CashPoint { point }`
  (`game.rs`, `session.rs`, `opcodes.rs`; decode test in `game::tests` from the lane's bytes).
* `SCAggroTargetChanged` (0x1bd: two `bc` ids, the NPC and its new target, 0 = idle) arrived after Sunder Earth, Play Dead, Snare and
  Healing Hymn and was unhandled: now `Event::AggroTargetChanged { npc, target }`; test in `unit_msg.rs`.
* `skill::result`: names and constants for every code 0 to 130 (the server's `SkillResultHelper` table; the lane answered 0x48, 0x15
  and 0x06, which had no names). `result::name` falls back to the table. Tests `every_server_result_number_has_a_name` and
  `ground_cast_bytes_follow_the_server_reader`.
* No encode or decode bug in the cast, started, fired, plot, damage, heal, buff and cooldown paths: all 110 skills decoded cleanly.

(b) Client bug in aa-client.

For the HUD:

* **Fixed** (`crates/aa-client/src/hud/cast.rs`, `route_casts`): a channel without a cast time (Invincibility, Play Dead, Twilight,
  Telekinesis, Protective Wings) sends no `SkillStarted`, only `SkillFired` with `channeling_ms`, so the cast bar never started. A
  `SkillFired` with `channeling_ms > 0` and no cast of that `tl` now starts a channel bar (`SPELLCAST_START` with the channel length,
  emptying). Unit test in the same file; seen in the real client (`shots/skilltest/t3chan.png`).
* The buff "Grit" (864, `max_charge` 3000) shows as `x3000`. That follows the data (a charge buff), not a bug; compare with the real
  client's icon text if it looks wrong.

For the combat agent (combat.rs, combat/, anim.rs, weapons.rs; nothing blocked, ideas only):

* `Event::AggroTargetChanged` and `Event::AiAggro` are available: a mob that changes target could turn towards it, and the target frame
  could show who a mob fights.
* A friendly skill cast with a hostile selected (refused 0x09) shows nothing on the HUD; the real client prints an "invalid target" message.

(c) Server bug or missing feature (not edited; evidence for the server owner):

* The server runs skills the character did not learn, and from ability trees that are not active (e4, `--no-learn`).
  `systems/skills/handlers.rs` has no learned or active ability check. The client check hides it.
* No `SCCooldowns` after a cast, and a recast is refused with only `SkillStarted { result 8, tl 0 }`; the client has to estimate the
  cooldown from the database (it does, and it matched the server in e2).
* `/spawn npc 3492` (rat) and `/spawn npc 10006` (bat) answer "Spawn NPC ... facing you" but no unit appeared within 3 s. The log has
  `spawner - [Spawn] Can't spawn npc 13013 from spawnerId 0 - Template is null` (rs-game.log line 3119 and many more) also for dolls that did
  appear, so the message is noise at best and hides real failures.
* Character saves failed with `Out of range value for column 'level' at row 1` (rs-game.log 21:55:19 and 21:55:45, character 70
  `Aoecaster`, 32 lines in the log), after a `/level` command. While a save was pending the same account could not log in again
  ("Previous character save is pending"); it cleared by itself around 22:02. Not our character, but it stopped the driver for minutes.

aa-net state, left alone because `state/stats.rs` holds another agent's uncommitted edits: after `SCAbilitySwapped` into an empty slot (11)
the server copies the experience of ability 1 to the new ability; `PlayerStats::ability_exp` does not, so the title can read level 1
until the next `SCAbilityExpChanged`.

## Per skill tables

### 1. Battlerage (11 of 11 pass)

| id | skill | client sent | server | events received | unhandled / failed | verdict |
|---|---|---|---|---|---|---|
| 18132 | Triple Slash | unit | accepted | Fired x2, Plot x9, Ended x3, Dmg x7 (233), Buff+ x5, Buff- x3, CdReset, UnitsRemoved | none | PASS |
| 11918 | Charge | unit | accepted | Plot x7, Ended x1, Dmg x1 (140), Buff+ x1, Buff- x1, CdReset | none | PASS |
| 13282 | Whirlwind Slash | self | accepted | Plot x19, Ended x1, Dmg x4 (923), Buff+ x1, Buff- x1, CdReset | none | PASS |
| 10644 | Sunder Earth | self | accepted | Fired x2, Plot x36, Ended x3, Dmg x8 (1266), Buff+ x55, Buff- x20, CdReset, UnitDeath, UnitsRemoved | unhandled ['0x1bd'] | PASS (SCAggroTargetChanged 0x1bd was unhandled in this run, decoded since the fix) |
| 12039 | Lasso | unit | accepted | Fired x4, Plot x6, Ended x5, Dmg x4 (213), Buff+ x275, Buff- x5, CdReset | none | PASS |
| 18308 | Terrifying Roar | self | accepted | Fired x4, Ended x4, Dmg x3 (251), Buff+ x209, Buff- x3, CdReset, UnitsRemoved | none | PASS |
| 12034 | Bondbreaker | self | accepted | Fired x5, Ended x5, Dmg x4 (128), Buff+ x270, Buff- x12, CdReset | none | PASS |
| 12026 | Precision Strike | unit | accepted | Fired x4, Ended x4, Dmg x4 (380), Buff+ x208, Buff- x1, CdReset | none | PASS |
| 10455 | Frenzy | self | accepted | Fired x4, Ended x4, Dmg x3 (192), Buff+ x211, CdReset | none | PASS |
| 10377 | Battle Focus | self | accepted | Fired x6, Ended x6, Dmg x4 (211), Buff+ x283, CdReset | none | PASS |
| 13315 | Tiger Strike | unit | accepted | Fired x1, Plot x16, Ended x2, Dmg x6 (4596), Buff+ x5, Buff- x1, CdReset, UnitDeath | none | PASS |

### 2. Witchcraft (11 of 11 pass)

| id | skill | client sent | server | events received | unhandled / failed | verdict |
|---|---|---|---|---|---|---|
| 14376 | Earthen Grip | unit | accepted | Started, Fired x1, Ended x1, Buff+ x1, CdReset | none | PASS, combo untested (its damage effects need a rooted target, buff tag 27, or a mind-destruction debuff, tag 373: skill_effects.target_buff_tag_id, so no damage on a plain target is the data) |
| 10159 | Enervate | unit | accepted | Fired x1, Ended x1, Dmg x5 (408), Buff+ x1, Buff- x1, CdReset | none | PASS |
| 10154 | Bubble Trap | unit | accepted | Started, Fired x1, Ended x1, Buff+ x1, CdReset | none | PASS, combo untested (its damage effect needs a lightning-damage debuff, tag 121; the buff effects pick on tag 119) |
| 10409 | Insidious Whisper | unit | accepted | Started, Fired x1, Ended x1, Buff+ x1, Buff- x1, CdReset | none | PASS |
| 10712 | Purge | self | accepted | Fired x1, Ended x1, Buff+ x1, CdReset | none | PASS |
| 10488 | Play Dead | self | accepted | Fired x6, Ended x6, Stopped x1, Dmg x5 (210), Heal x20, Buff+ x253, Buff- x33, CdReset | unhandled ['0x1bd'] | PASS (channel: the driver stops it after 6 s, CastingStopped + SkillEnded came back; 0x1bd decoded since the fix) |
| 11424 | Courageous Action | self | accepted | Fired x4, Ended x4, Dmg x3 (194), Buff+ x214, CdReset | none | PASS |
| 10134 | Lassitude | unit | accepted | Fired x4, Ended x4, Dmg x3 (167), Buff+ x214, Buff- x2, CdReset | none | PASS |
| 12001 | Banshee Wail | self | accepted | Fired x1, Ended x1, Buff+ x6, Buff- x3, CdReset | none | PASS |
| 11353 | Focal Concussion | unit | accepted | Fired x4, Ended x4, Dmg x4 (574), Buff+ x215, Buff- x3, CdReset | none | PASS |
| 11443 | Dahuta's Breath | self | accepted | Fired x4, Ended x4, Dmg x5 (1876), Buff+ x215, Buff- x2, CdReset | none | PASS |

### 3. Defense (11 of 11 pass)

| id | skill | client sent | server | events received | unhandled / failed | verdict |
|---|---|---|---|---|---|---|
| 10399 | Shield Slam | unit | accepted | Fired x3, Plot x6, Ended x4, Dmg x1 (142), Buff+ x1, Buff- x2, CdReset | none | PASS |
| 10645 | Refreshment | self | accepted | Fired x4, Ended x4, Dmg x3 (268), Buff+ x201, Buff- x19, CdReset | none | PASS |
| 10501 | Bull Rush | unit | accepted | Fired x3, Plot x5, Ended x4, Dmg x4 (292), Buff+ x224, Buff- x3, CdReset | none | PASS |
| 12048 | Boastful Roar | unit | accepted | Fired x4, Ended x4, Dmg x4 (2819), Buff+ x221, Buff- x2, CdReset, UnitsRemoved | none | PASS |
| 11365 | Toughen | self | accepted | Fired x4, Ended x4, Dmg x3 (211), Buff+ x220, Buff- x1, CdReset | none | PASS |
| 12046 | Revitalizing Cheer | self | accepted | Fired x4, Ended x4, Dmg x3 (15), Heal x1, Buff+ x147, Buff- x1, CdReset | none | PASS |
| 10375 | Redoubt | self | accepted | Fired x4, Ended x4, Dmg x3 (87), Buff+ x147, CdReset | none | PASS |
| 18757 | Ollo's Hammer | unit | accepted | Fired x3, Plot x6, Ended x4, Dmg x7 (1930), Buff+ x147, Buff- x1, CdReset | none | PASS |
| 10436 | Mocking Howl | self | accepted | Fired x4, Ended x4, Dmg x3 (76), Buff+ x77, Buff- x7, CdReset | none | PASS |
| 14529 | Imprison | self | accepted | Fired x4, Ended x4, Dmg x3 (249), Buff+ x219, CdReset | none | PASS |
| 10372 | Invincibility | self | accepted | Fired x8, Ended x8, Stopped x1, Buff+ x1, Buff- x1, CdReset | none | PASS (channel: the driver stops it after 6 s, CastingStopped + SkillEnded came back) |

### 4. Auramancy (11 of 11 pass)

| id | skill | client sent | server | events received | unhandled / failed | verdict |
|---|---|---|---|---|---|---|
| 16486 | Thwart | self | accepted | Fired x4, Ended x4, Buff+ x5, Buff- x4, CdReset | none | PASS |
| 18222 | Comet's Boon | self | accepted | Fired x4, Plot x11, Ended x5, Dmg x9 (3725), Buff+ x121, Buff- x11, CdReset | none | PASS |
| 11869 | Conversion Shield | self | accepted | Fired x5, Ended x5, Dmg x4 (187), Buff+ x127, Buff- x3, CdReset | none | PASS |
| 10710 | Vicious Implosion | self | accepted | Fired x3, Plot x27, Ended x4, Dmg x13 (3428), Buff+ x221, Buff- x3, CdReset, UnitDeath | none | PASS |
| 10152 | Teleportation | self | accepted | Fired x4, Ended x4, Dmg x8 (323), Buff+ x339, Buff- x5, CdReset, BlinkUnit | none | PASS |
| 11991 | Health Lift | self | accepted | Started, Fired x5, Ended x5, Dmg x4 (280), Buff+ x173, Buff- x6, CdReset, UnitsRemoved | none | PASS |
| 11989 | Meditate | self | accepted | Fired x8, Ended x8, Dmg x13 (728), Heal x8, Buff+ x561, Buff- x1, CdReset | none | PASS |
| 11429 | Shrug It Off | self | accepted | Fired x4, Ended x4, Dmg x7 (283), Buff+ x303, Buff- x3, CdReset | none | PASS |
| 11380 | Liberation | self | accepted | Fired x4, Ended x4, Dmg x3 (361), Buff+ x130, CdReset, UnitsRemoved | none | PASS |
| 10104 | Leech | unit | accepted | Fired x4, Ended x5, Dmg x7 (310), Buff+ x301, Buff- x4, CdReset, UnitsRemoved | none | PASS |
| 10714 | Protective Wings | self | accepted | Fired x9, Ended x9, Stopped x1, Dmg x9 (508), Buff+ x347, Buff- x5, CdReset, UnitsRemoved | none | PASS (channel: the driver stops it after 6 s, CastingStopped + SkillEnded came back) |

### 5. Occultism (11 of 11 pass)

| id | skill | client sent | server | events received | unhandled / failed | verdict |
|---|---|---|---|---|---|---|
| 14810 | Mana Stars | unit | accepted | Fired x3, Plot x10, Ended x4, Dmg x7 (102), Buff+ x8, Buff- x8, CdReset | none | PASS |
| 10201 | Crippling Mire | unit | accepted | Fired x6, Plot x13, Ended x7, Dmg x14 (1848), Buff+ x245, Buff- x29, CdReset | none | PASS |
| 10135 | Hell Spear | self | accepted | Fired x3, Ended x3, Dmg x9 (3236), Buff+ x97, Buff- x8, CdReset | none | PASS |
| 11441 | Absorb Lifeforce | unit | accepted | Fired x7, Plot x28, Ended x8, Dmg x29 (2018), Buff+ x763, Buff- x5, CdReset | none | PASS |
| 11395 | Summon Crows | self | accepted | Fired x4, Ended x4, Dmg x31 (1846), Buff+ x149, CdReset, UnitsRemoved | none | PASS |
| 12759 | Mana Force | unit | accepted | Fired x4, Plot x4, Ended x5, Dmg x8 (551), Buff+ x136, Buff- x1, CdReset, UnitsRemoved | none | PASS |
| 11504 | Telekinesis | unit | accepted | Started, Fired x11, Ended x11, Stopped x1, Dmg x9 (642), Buff+ x363, Buff- x2, CdReset | none | PASS (channel: the driver stops it after 6 s, CastingStopped + SkillEnded came back) |
| 10655 | Retribution | self | accepted | Fired x4, Ended x4, Dmg x3 (202), Buff+ x136, CdReset | none | PASS |
| 10665 | Stillness | self | accepted | Fired x2, Ended x2, Dmg x1 (0), Buff+ x17, Buff- x18, CdReset, UnitsRemoved | none | PASS |
| 11442 | Urgency | self | accepted | Started, Fired x7, Ended x7, Dmg x6 (605), Buff+ x271, CdReset, UnitsRemoved | none | PASS |
| 10434 | Summon Wraith | self | accepted | Fired x1, Ended x1, Dmg x65 (17632), Buff+ x8, Buff- x8, CdReset, UnitDeath | none | PASS |

### 6. Archery (11 of 11 pass)

| id | skill | client sent | server | events received | unhandled / failed | verdict |
|---|---|---|---|---|---|---|
| 16210 | Charged Bolt | unit | accepted | Fired x1, Ended x1, Dmg x1 (456), Buff+ x1, Buff- x1, CdReset, UnitsRemoved | none | PASS |
| 13564 | Piercing Shot | unit | accepted | Fired x1, Ended x1, Dmg x5 (384), Buff+ x1, Buff- x1, CdReset | none | PASS |
| 14835 | Endless Arrows | unit | accepted | Fired x1, Plot x7, Ended x2, Dmg x10 (318), Buff+ x1, Buff- x1, CdReset | none | PASS |
| 11368 | Double Recurve | self | accepted | Fired x1, Ended x1, Dmg x1 (50), Buff+ x1, Buff- x1, CdReset | none | PASS |
| 15073 | Deadeye | self | accepted | Started, Fired x4, Ended x4, Dmg x3 (3), Buff+ x3, Buff- x2, CdReset, UnitsRemoved | none | PASS |
| 12133 | Snare | self | accepted | Fired x1, Ended x1, Buff+ x10, Buff- x2, CdReset | unhandled ['0x1bd'] | PASS (SCAggroTargetChanged 0x1bd was unhandled in this run, decoded since the fix) |
| 10694 | Float | self | accepted | Fired x4, Ended x4, Dmg x3 (143), Buff+ x75, Buff- x30, CdReset, UnitsRemoved | none | PASS |
| 14760 | Boneyard | unit | accepted | Fired x3, Plot x3, Ended x4, Dmg x15 (4360), Buff+ x313, Buff- x9, CdReset, UnitDeath | none | PASS |
| 11933 | Concussive Arrow | unit | accepted | Started, Fired x2, Ended x2, Dmg x16 (3631), Buff+ x381, Buff- x23, CdReset, UnitDeath, UnitsRemoved | none | PASS |
| 13281 | Missile Rain | ground (15540, 15329) | accepted | Plot x33, Ended x1, Buff+ x1, Buff- x1, CdReset, UnitsRemoved | none | PASS |
| 10708 | Intensity | self | accepted | Fired x1, Ended x1, Buff+ x2, Buff- x1, CdReset | none | PASS |

### 7. Sorcery (11 of 11 pass)

| id | skill | client sent | server | events received | unhandled / failed | verdict |
|---|---|---|---|---|---|---|
| 10752 | Flamebolt | unit | accepted | Plot x7, Ended x1, Dmg x1 (91), Buff+ x1, Buff- x1, CdReset | none | PASS |
| 10667 | Freezing Arrow | unit | accepted | Plot x6, Ended x1, Dmg x1 (754), Buff+ x2, Buff- x2, CdReset, UnitsRemoved | none | PASS |
| 10153 | Insulating Lens | self | accepted | Started, Fired x5, Ended x5, Dmg x4 (4), Buff+ x2, Buff- x1, CdReset, UnitsRemoved | none | PASS |
| 10670 | Arc Lightning | unit | accepted | Fired x4, Plot x6, Ended x5, Dmg x29 (1957), Buff+ x195, Buff- x4, CdReset | none | PASS |
| 10151 | Freezing Earth | self | accepted | Fired x2, Ended x2, Dmg x35 (4899), Buff+ x345, Buff- x9, CdReset, UnitDeath | none | PASS |
| 11939 | Searing Rain | self | accepted | Plot x45, Ended x1, Stopped x2, Dmg x28 (3859), Buff+ x257, Buff- x3, CdReset | none | PASS (channel plot: the driver stops it after 6 s, PlotStopped + PlotEnded came back) |
| 11314 | Frigid Tracks | self | accepted | Started, Fired x1, Ended x1, Buff+ x3, Buff- x9, CdReset, UnitsRemoved | none | PASS |
| 12796 | Magic Circle | self | accepted | Plot x5, Ended x1, Buff+ x2, Buff- x3, CdReset | none | PASS |
| 11967 | Chain Lightning | unit | accepted | Plot x16, Ended x1, Dmg x157 (6940), Buff+ x9, Buff- x1, CdReset, UnitsRemoved | none | PASS |
| 14774 | Flame Barrier | self | accepted | Plot x2, Ended x1, Dmg x119 (4513), Buff+ x8, Buff- x8, CdReset | none | PASS |
| 10664 | Meteor Strike | ground (15534, 15330) | accepted | Started, Fired x1, Ended x1, Dmg x9 (10222), Buff+ x16, Buff- x8, CdReset | none | PASS |

### 8. Shadowplay (11 of 11 pass)

| id | skill | client sent | server | events received | unhandled / failed | verdict |
|---|---|---|---|---|---|---|
| 18125 | Rapid Strike | unit | accepted | Fired x3, Plot x19, Ended x4, Dmg x3 (48), Buff+ x7, Buff- x4, CdReset | none | PASS |
| 10648 | Overwhelm | unit | accepted | Fired x4, Plot x9, Ended x5, Dmg x5 (415), Buff+ x131, Buff- x25, CdReset | none | PASS |
| 12049 | Drop Back | self | accepted | Fired x5, Ended x5, Dmg x4 (249), Buff+ x154, Buff- x1, CdReset | none | PASS |
| 12029 | Wallop | unit | accepted | Fired x7, Ended x7, Dmg x7 (628), Buff+ x155, Buff- x2, CdReset | none | PASS |
| 12139 | Stalker's Mark | unit | accepted | Fired x4, Ended x4, Dmg x4 (363), Buff+ x155, CdReset | none | PASS |
| 10082 | Stealth | self | accepted | Fired x4, Ended x4, Dmg x3 (172), Buff+ x153, Buff- x2, CdReset | none | PASS |
| 10481 | Toxic Shot | unit | accepted | Fired x4, Ended x4, Dmg x9 (704), Buff+ x52, Buff- x4, CdReset | none | PASS |
| 13344 | Pin Down | unit | accepted | Fired x5, Ended x5, Dmg x5 (407), Buff+ x206, Buff- x1, CdReset | none | PASS |
| 10496 | Shadowsmite | unit | accepted | Fired x3, Plot x4, Ended x4, Dmg x4 (129), Buff+ x154, Buff- x1, CdReset | none | PASS |
| 10189 | Freerunner | self | accepted | Fired x4, Ended x4, Dmg x3 (284), Buff+ x154, CdReset | none | PASS |
| 12075 | Shadow Step | self | accepted | Fired x5, Ended x5, Dmg x4 (237), Buff+ x204, Buff- x3, CdReset, BlinkUnit | none | PASS |

### 9. Songcraft (11 of 11 pass)

| id | skill | client sent | server | events received | unhandled / failed | verdict |
|---|---|---|---|---|---|---|
| 11973 | Critical Discord | unit | accepted | Started, Fired x5, Ended x5, Dmg x5 (767), Buff+ x1, Buff- x1, CdReset | none | PASS |
| 11934 | Startling Strain | unit | accepted | Fired x3, Plot x4, Ended x4, Dmg x3 (200), Buff+ x3, Buff- x1, CdReset | none | PASS |
| 10723 | [Perform] Quickstep | self | accepted | Fired x4, Ended x4, Dmg x3 (204), Buff+ x12, CdReset | none | PASS |
| 11943 | Dissonant | unit | accepted | Started, Fired x5, Ended x5, Dmg x4 (274), Buff+ x1, Buff- x4, CdReset | none | PASS |
| 17413 | Healing Hymn | self | accepted | Fired x4, Plot x18, Ended x5, Dmg x4 (184), Heal x15, Buff+ x134, Buff- x25, CdReset | unhandled ['0x1bd'] | PASS (SCAggroTargetChanged 0x1bd was unhandled in this run, decoded since the fix) |
| 11377 | Hummingbird Ditty | self | accepted | Started, Fired x5, Ended x5, Dmg x4 (297), Buff+ x158, Buff- x2, CdReset | none | PASS |
| 10724 | [Perform] Ode to Recovery | self | accepted | Fired x4, Ended x4, Dmg x3 (365), Heal x4, Buff+ x160, Buff- x2, CdReset, UnitsRemoved | none | PASS |
| 11948 | Rhythmic Renewal | self | accepted | Started, Fired x6, Ended x6, Dmg x5 (246), Heal x13, Buff+ x266, Buff- x6, CdReset, UnitsRemoved | none | PASS |
| 11396 | [Perform] Bulwark Ballad | self | accepted | Fired x4, Ended x4, Dmg x3 (96), Heal x5, Buff+ x162, CdReset | none | PASS |
| 10727 | [Perform] Bloody Chantey | self | accepted | Fired x4, Ended x4, Dmg x3 (115), Heal x5, Buff+ x162, Buff- x2, CdReset | none | PASS |
| 11961 | Alarm Call | self | accepted | Fired x5, Ended x5, Dmg x4 (164), Heal x10, Buff+ x160, Buff- x5, CdReset | none | PASS |

### 10. Vitalism (10 of 11 pass)

| id | skill | client sent | server | events received | unhandled / failed | verdict |
|---|---|---|---|---|---|---|
| 10534 | Antithesis | self | accepted | Started, Fired x5, Ended x5, Heal x2, Buff+ x1, Buff- x2, CdReset | none | PASS (cast on self: heal only; against a hostile mob it does damage and heal, run e1) |
| 11379 | Mirror Light | self | accepted | Fired x4, Ended x4, Dmg x3 (195), Heal x5, Buff+ x155, Buff- x11, CdReset | none | PASS |
| 10547 | Resurgence | self | accepted | Fired x4, Ended x4, Dmg x3 (264), Heal x9, Buff+ x165, Buff- x1, CdReset | none | PASS |
| 10546 | Revive | self | refused: 0x6 | Fired x8, Ended x8, Dmg x8 (618), Heal x3, Buff+ x441, Buff- x2, UnitsRemoved | none | UNTESTED (refused 0x06 target alive is the right answer; needs a dead player, which one session cannot make) |
| 13286 | Skewer | ground (15534, 15330) | accepted | Fired x4, Plot x9, Ended x5, Dmg x15 (8154), Buff+ x166, Buff- x5, CdReset | none | PASS |
| 10720 | Mend | self | accepted | Started, Fired x6, Ended x6, Dmg x5 (285), Heal x1, Buff+ x278, Buff- x2, CdReset, UnitsRemoved | none | PASS |
| 16783 | Infuse | self | accepted | Fired x4, Ended x4, Dmg x3 (221), Heal x1, Buff+ x166, Buff- x1, CdReset | none | PASS |
| 16004 | Aranzeb's Boon | self | accepted | Started, Fired x5, Ended x5, Dmg x4 (278), Buff+ x218, Buff- x4, CdReset, UnitsRemoved | none | PASS |
| 17412 | Renewal | self | accepted | Started, Fired x5, Ended x5, Dmg x4 (185), Heal x3, Buff+ x221, Buff- x2, CdReset | none | PASS |
| 14929 | Fervent Healing | self | accepted | Fired x4, Ended x4, Dmg x3 (311), Heal x6, Buff+ x167, Buff- x1, CdReset | none | PASS |
| 10721 | Twilight | self | accepted | Fired x1, Ended x1, Stopped x1, Buff+ x2, Buff- x1, CdReset | none | PASS (channel: the driver stops it after 6 s, CastingStopped + SkillEnded came back) |

### Default skills, emotes, ground casts

| id | skill | client sent | server | events received | unhandled / failed | verdict |
|---|---|---|---|---|---|---|
| 2 | Melee Attack | unit | accepted | Fired x1, Ended x1, Dmg x1 (23), CdReset | none | PASS (its buff is conditional) |
| 14495 | Kick | unit | accepted | Plot x3, Ended x1, Dmg x1 (38), Buff+ x1, Buff- x1, CdReset | none | PASS |
| 16064 | Shoot Arrow | unit | accepted | Fired x1, Ended x1, Dmg x1 (29), CdReset | none | PASS |
| 16287 | Dash | self | accepted | Fired x1, Plot x32, Ended x1, Buff+ x2, Buff- x1, CdReset | none | PASS |
| 14900 | Play Instrument | self | accepted | Fired x1, Ended x1, CdReset | none | PASS |
| 14901 | Use Glider | self | accepted | Fired x1, Ended x1, CdReset | none | PASS |
| 14902 | Drop Trade Pack | self | accepted | Fired x1, Ended x1, CdReset | none | PASS |
| 14152 | Dance: Gotta Get Down | self | accepted | Fired x1, Ended x1, Buff+ x1, CdReset | none | PASS |
| 15229 | /beg | self | accepted | Fired x1, Ended x1, Buff+ x1, Buff- x2, CdReset | none | PASS |
| 14451 | /waving | self | accepted | Fired x1, Ended x1, CdReset | none | PASS |
| 10594 | Recall | self | accepted | Started, Fired x1, Ended x1, Buff+ x2, Buff- x1, CdReset, Teleported | none | PASS |
| 13281 | Missile Rain | ground (15540, 15329) | accepted | Fired x5, Plot x33, Ended x6, Dmg x4 (183), Buff+ x92, Buff- x25, CdReset | none | PASS |
| 10664 | Meteor Strike | ground (15534, 15330) | accepted | Started, Fired x8, Ended x8, Dmg x22 (18525), Buff+ x428, Buff- x16, CdReset, UnitDeath, UnitsRemoved | none | PASS |
| 13286 | Skewer | ground (15534, 15330) | accepted | Fired x4, Plot x10, Ended x5, Dmg x19 (12193), Buff+ x232, Buff- x4, CdReset | none | PASS |

