# aa-client HUD: the original in-game UI on real data

`crates/aa-client/src/hud/` runs the client's own Lua HUD (`Stage::World`, about 900 scripts under
`scriptsbin/x2ui`) in `Screen::InGame`, on what `aa-net` reports, and draws the one thing the
toolkit does not: name plates over units. The scripts are unchanged. What they ask of the game
(`X2Unit`, `X2Player`, `X2Bag`, `X2Skill`, `X2World`, ...) is answered from the live data.

Status: after `--enter-world` or the full login flow the screen matches `reference/ingame/spawn/real.png`
(player frame, buffs, exp/labor bar, roadmap with the zone header and sub zone, clock, chat, both
action bars, bag/menu buttons, the tutorial window, name plates); 0 script errors.
`shots/hud/spawn.png` is the comparison shot. `cargo run -p aa-client --example client -- ...` runs the same app
through the `aa-dev` harness for unattended integration captures.

## Running

```
cargo run -p aa-client -- --enter-world --pos 15578.042,15382.122 --yaw 158.74
cargo run -p aa-client                      # the full login flow, then the same screen
cargo run -p aa-client -- --goto loading --account bevytest    # the flow without the login screens
```

Add `--shot shots/hud/NAME.png --shot-after 25` for an unattended screenshot. `--no-hud` skips the
HUD (frame rate comparisons, bare world screenshots).

Scripted input for unattended runs (seconds counted from the moment the HUD is up):

| flag | does |
|---|---|
| `--hud-key NAME T` | a key (lower case): `enter`, `escape`, `backspace`, `delete`, `tab` (target the next enemy), `slotN` (action bar slot N, 1 based), `f` / `g` (loot all / open the loot) |
| `--hud-type TEXT T` | typed text (in Git Bash set `MSYS_NO_PATHCONV=1` before a text starting with `/`) |
| `--hud-open ACTION T` | a key binding action: `toggle_bag`, `toggle_character`, `toggle_spellbook`, `toggle_quest`, `toggle_worldmap` |
| `--hud-click X Y T` | a left click at physical pixel X, Y |
| `--hud-rclick X Y T` | a right click at X, Y (a bag item: use it) |
| `--hud-hover X Y T` | the pointer moves to X, Y and stays (tooltips; move it somewhere else first when a window was just opened, then onto the slot) |
| `--hud-disconnect T` | the disconnect dialog, as if the server dropped the connection |
| `--hud-burst PREFIX T DT N` | N screenshots `PREFIX_01.png` ... from T seconds on, DT seconds apart (frame by frame comparisons; the game keeps running) |
| `--hud-fov DEG`, `--hud-side-cam` | the world camera's vertical field of view (the combat captures use 71.25), and the capture's side-on camera (yaw = the player's yaw - 90 degrees; add `--cam-dist 4 --cam-pitch 0`) |
| `--hud-afk` | the "-Away-" state at once (else after 300 s without input) |
| `--hud-fake-loot T` | the lane server's Angry Rat never drops anything: T seconds after the HUD is up the nearest unit is a lootable corpse (the F / G icons) and two seconds later the loot window opens on three made up items |
| `--hud-fail NAME T` | the refusal `NAME` (`LACK_MANA`, `TOO_FAR_RANGE`, `NO_TARGET`, `URK_COMBAT` ... the skill result names of `usable.rs`) at T seconds: the red text in the middle of the screen and the voice, as for a real refusal |
| `--hud-sim mp=N`, `--hud-sim dead` | the HUD pretends the player has at most N mana / is dead (screenshots of the dark bars and the death window without a fight; only the HUD's view, the server knows nothing of it) |
| `--hud-ability-swap NEW OLD T` | raises `ABILITY_CHANGED` for the archetype ids NEW and OLD (1..10) at T seconds |

## What is wired

| feature | source |
|---|---|
| player frame: name, level, HP/MP bars and numbers | `aa_net::Player`, `PlayerStats` (`HP_CHANGED` / `MANA_CHANGED` / `PLAYER_LEVEL...` events on change) |
| exp bar, labor points, loyalty tokens | `PlayerStats`, `Player` |
| buffs of the player and of the target (icon, remaining time, tooltip text) | `aa_net::Buffs` + the DB's buff table; remaining time is pushed as an end time on the HUD clock, so the countdown costs nothing per frame |
| action bars: 2 x 12 slots, skill and item icons, item counts, grade frames, cooldown and global cooldown sweeps | `aa_net::ActionBar`, `Inventory`, `Skills.cooldown_left` / `gcd_left`; the DB for names, icons, descriptions, `custom_gcd` |
| using a slot: keys 1..9, 0, -, = (Shift for the second bar) | `Command::UseSkill` / `UseItem` |
| dark action bar slots: no target, out of range, no mana, wrong target, casting, dead, stunned, missing weapon, empty item stack | `usable.rs` over the DB and aa-net's state; see "Dark slots" |
| refused uses: "Select a target.", "Target is too far.", "Insufficient mana to use this." ... and the character's voice line | `alerts.rs`: the client's own check before sending, the server's `SCSkillStarted` refusals and `SCErrorMsg`; see "Refusals" |
| death window ("Hereafter Threshold": Revive Here / Respawn: Temple), `ABILITY_CHANGED` after an ability swap, the target frame's job icon for other characters | `alerts.rs` (`DeathDetails`, `Resurrected`, `AbilitySwapped`), `Command::Resurrect`; `Unit::active_abilities` |
| target frame, Tab targeting (nearest unit within 40 m, cycles), left click on a unit, Escape clears | `aa_net::Target`, `Command::Target`; the pick is a ray-to-unit-centre distance (1.1 m, 6 px slop) and ignores clicks over UI |
| chat: incoming (all channels, notices, system), sending with Enter, edit box focus | `aa_net::Chat`, `Command::Chat`; `HudKeyboardFocus`. A zone channel join is "1.Shout", "2.Trade", "3.Party Search" (aa-net raises all three, as the client does); labor changes print "Gained 10 Labor Point(s). (Total LP: 105)" (`PlayerStats::labor_log`, also those that arrived while loading) |
| roadmap: zone name, sub zone, faction, heading, clock | zone from the DB (`WorldInfo` zone group, then position), sub zones from the pak's `subzone_area.xml` polygons (the DB's rectangles are mostly 0), the clock from `aa_world::WorldClock` (the server's, or `--hour`) |
| sub zone header and the city map | the header is the top of the sub zone's `parent_sub_zone_id` chain (the Royal Palace is in Ezna: "Ezna"). Where that sub zone has a road map of its own (cities, caves: `HUD_ROAD_FRAMES[3]`) the roadmap is that map (level 3, id = sub zone), placed from the sub zone's `x, y, w, h` and the image's frame in the 926 x 556 roadmap window (`road_frames.lua`, generated from the client's `map/road_map_list.lua`); the zone maps are placed the same way (checked on `reference/ingame/spawn`: window pixel 703.2, 147.7; and `royal_palace`: Ezna 770, 363) |
| the big map (M) | `AAUI.WorldMap`: the player's place on the city or zone map (plain fraction of the rectangle) and on the continent and the world (`world_groups`) |
| tooltips: bag items, action bar and use-skill, buffs, equipped items | `tips.rs`: `HUD.item_tip` / `HUD.skill_tip` / `HUD.buff_text` fill the client's tooltip tables (`name`, `grade` / `gradeColor` from `item_grades`, `category`, `item_impl`, `soul_bound` / `soul_bind`, `skillType`, `description`, price, `tipType = "skill"` with ability, rank, mana, cast, range, radius, cooldown) from the DB; `slot.tooltip`, `X2Bag:GetBagItemInfo`, `X2Skill:GetSkillTooltip`, `X2Unit:UnitBuff` (`tipType = "buff"`) return them. The numbers in descriptions (`#{avg_damage}`, `#{buff_duration}`, `#{ua_*}` ...) are worked out in `tipcalc.rs`, the weapon and armor sections (attack, defence, speed, stat bonuses, durability, set bonuses) in `tipitem.rs` (see "Tooltip numbers" below) |
| bag window | `aa_net::Inventory`: items, counts, icons, grade frames, tooltips, "n/50"; the tabs filter by item group (below) |
| max HP / MP of the player and of the target | the DB's unit formulas (below); the server sends none |
| quest marks over NPCs | `aa_net::Quests` + the DB's quest tables (below) |
| character info window | stats from `PlayerStats`, equipment slots from `Inventory` (see below) |
| name plates | `Units` + the DB; see below |
| combat: floating damage / heal / miss texts, "Battle Start!" / "Battle End!", "XP +14", the red numbers over the player | `UnitDamaged`, `UnitHealed`, `ExpChanged`, `CombatChanged` as the scripts' `COMBAT_TEXT` / `COMBAT_MSG` / `UNIT_COMBAT_STATE_CHANGED` / `EXP_CHANGED` events; see "Combat" |
| target ring under the selected unit | `ring.rs`, see "Combat" |
| cast bar ("Recall") | `SkillStarted` / `SkillFired` / `CastingStopped` / plot events, see "Combat" |
| chat lines "Received:" / "Used:", "Gained 14 XP." | `ItemsChanged` (`items.rs`), `ExpChanged` |
| loot: the F / G icons, the loot window | `Loot` (`SCLootableState`, `SCLootBagData` ...), `loot.rs`, see "Combat" |
| leave the game, back to the character or server list, the leave countdown | `Command::LeaveWorld`, `Event::LeavePending` / `LeaveCancelled` / `LeftWorld` |
| disconnect dialog | `Event::Disconnected` / `Kicked` not asked for -> `DISCONNECTED_BY_WORLD`; OK logs in again and shows `Screen::WorldSelect` |
| keys I B C K L M, Escape | `UiRuntime::hotkey` / `escape()`; skipped while an edit box has the keyboard |

Anything the game side does not provide yet keeps the fake data of `aa-ui/examples/fake/fake_*.lua`
(logged as `FALLBACK`), so a window never comes up empty because of a missing hook.

### Character window and its model

The real client shows the in-world character between the slot columns: there is no render to do. The back view of
the player with the slots floating over the live world is what the capture itself shows
(`reference/ingame/character_info/real.png`), and the capture's own note says so (`player_screen.json`: "The 3D
model is the in-world player itself; the equipment slot columns are widgets around it"). The widget dump
(`ui.json`, 761 widgets) has no model-view or render-target widget either: `equippedItem` is a plain window at
(960, 290) with `leftSlots` (760, 290) and `rightSlots` (1118, 290), 42 x 485 each, next to the 430 px wide
`characterInfo.mainWindow` at (28, 60). `camera.json` equals the spawn camera, so the default orbit camera
already frames the player between the columns (`shots/hud/charwin.png`). No camera override is needed.
The equipment slots take their items from `Inventory` (`ES_*` constant = `aa_net::model::slot` index + 1).

The title under the level ("Archery Novice") is the script's `GetCombinedAbilityName` over the names
`X2Unit:GetTargetAbilityTemplates("player")` answers; with fewer than three abilities it is the "newbie job" of the first
one (`locale.newbieJobName`: fight Battlerage, wild Archery, magic Sorcery, love Vitalism, death Occultism, vocation
Shadowplay). The answers come from the character's own set (`PlayerStats::active_abilities` and `ability_exp`, the ids of
the character list's `abilities`, kept current by `SCAbilitySwapped`): `Snapshot.abilities` / `ability_exp` ->
`HUD.state.abilities` (the rows of `X2Ability:GetActiveAbility`: `type`, `level`, `levelPercent` 0..100, `exp`,
`prevLevelTotalExp`, `nextLevelTotalExp`) and `ability_levels`, which `ingame.lua` serves through
`GetTargetAbilityTemplates`, `GetActiveAbility`, `NumActiveAbility`, `IndexFromAbility`, `GetAbilityLevel` and
`GetAllAbility` (the player frame's ability icon and the skill window read the same). The log has one
`hud: abilities [6] (exp [...])` line per change. `ABILITY_CHANGED` refreshes the title and the open skill window
after a swap; character targets supply their active abilities to the same job-name lookup.

### Skill book and learning

`skillbook.rs` loads all ten ability trees, passive buffs, skill-point budgets and basic/emote lists from the game
database. `skillbook.lua` answers the original skill window's `X2Ability` queries and `ISLOT_ABILITY_VIEW` slots.
Learned ranks, free points, ability levels and points spent in a tree determine which skills can be learned.
The original action-slot script receives `learned`, `canLearn`, `reqPoints` and `skillLevel` for its red/teal tints
and passive requirement badges. Action-bar shortcuts to unlearned tree skills use the learning tint and refuse
locally; ordinary target/range/mana/state restrictions still darken usable skills through `usable.rs`.

Clicking a learnable skill or passive opens the original confirmation dialog. OK sends `LearnSkill` / `LearnBuff`;
resetting a tree and choosing an ability use `ResetSkills` / `SwapAbility`. Server replies refresh the book and
points through the existing UI events. Reset membership is applied before the HUD snapshot, with subsequent learns
replayed in packet order so a reset/relearn in one frame retains the new skill. Ability experience refreshes the
open book without inventing a gained-experience chat message from a total.

`cargo test -p aa-client hud::skillbook` checks the real database's ten trees and skill-point budgets, learning
requirements, passive spending and same-frame reset/relearn ordering.

Live validation (`shots/hud/sol61/learn_02.png`, `learned-tooltip.png`): reset Battlerage, recover one point,
show Triple Slash as learnable, confirm learning, receive the server acknowledgement, restore the learned icon
and show Charge's level/point requirements. Zero script errors; 82-99 FPS in these shared-machine debug runs.
The final complete login flow (`verified-flow-final.png` / `.log`) enters after 7.5 s of loading, with zero script
errors or runtime warnings and 92-102 FPS once settled. The crate's normal test run passes 94 tests; 13 data surveys
and timing probes remain intentionally ignored. Clippy completes with existing style/dead-code warnings.

### Death and resurrection

The native `GetResurrectionCountdown` returns `(inPlace, autoTempleSeconds, buttonWaitSeconds)`, rather than two
button waits. Read-only `D:/aa-re/client12/x2game_unpacked.dll`, callback `0x39495c00`, fixes the automatic deadline
at 300,000 ms; both times truncate to seconds. Local death starts that deadline and the packet's shorter wait;
`SCNotifyResurrection` permits in-place revival without restarting either. The HUD tracks this while loading too.
An already-dead login has no fresh death timestamp and retains the native zero-deadline behaviour.

`--hud-key escape` now follows the physical Escape path: cancel a held icon, close the top window, clear a target,
then open the system menu. The original edit-box Escape handling still runs while typing.

### Name plates

`plates.rs` draws them with a second 2D camera (`PlateCamera`, order 5, render layer 30, after the
world and before the UI camera at 10): `Text2d` in the client's English UI font (`flareserif_821_roman`)
plus two sprites for the 69 x 6 bar. Per frame, in `PostUpdate` before transform propagation, up to 40
plates for the nearest units (NPCs 45 m, characters 70 m, the player's own always):

* position: `Camera::world_to_viewport` of the unit's feet + the height of its model + a gap over the head (0.3 m for
  a person, `0.3 * sqrt(height / 1.8)` for smaller units: the capture's rat bar is 0.24 m over its feet). The
  height (`plates::unit_height`, also used by the quest marks and the floating texts) comes from the bounds of the
  model's static meshes and, since skinned meshes carry none (aa-cry draws them without frustum culling), from the
  highest joint of its skeleton (hair and cloth bones, `rope*`, left out) + 4 %. The first result is kept (the unit at rest;
  a rat that rears up in a fight is not tall) with a few more tries for late parts; a character's is capped at 1.85 m
  (the bounds take in the bow). The Angry Rat (scale 1.5) comes out at 0.16 m, Maude at 1.7 m;
* colour by relation: hostile orange-red, neutral yellow, friendly green. The rule is the server's: both factions go
  to their root faction (`system_factions.mother_id`: 1 friendly, 2 neutral, 3 hostile), the same group is friendly, else
  the stored relation (`system_faction_relations`, state 1 hostile, 2 neutral, 3 friendly) of the mother faction or the
  faction; the Angry Rat (faction 115) is hostile to a Nuian (101). An NPC of the player's own faction is yellow, as in
  the capture; the player's own plate is green;
* the target has a bigger name (26 px) over a framed yellow bar (81 x 14); a corpse has a grey name and an empty
  bar; the player's own plate shows "-Away-" under the name after 300 s without input (the capture, assumed time);
* NPCs with `show_name_tag = 0` get no plate; names are 11 px for NPCs and 26 px for characters.

The camera copies `Hdr` and `Msaa` from the world camera and is spawned only after the world camera has
`Hdr` (aa-world's post processing adds it a frame later); a frame where the two disagree is a wgpu
validation error ("attachments have differing sample counts"). `cameras.rs` keeps the UI camera in step.

### Combat

All of it is fed by `route_combat` / `route_casts` / `route_items` / `sync_loot` (systems after `sync_hud`) from
aa-net's events; what the scripts get is what the client's C++ raises. Tested on the lane server at the Nuian spawn
pond (account `bevytest`, the Angry Rats) with
`--enter-world --hour 12 --pos 15578.042,15382.122 --yaw 158.74 --cam-yaw -90 --cam-dist 4 --cam-pitch 0 --hud-fov 71.25 --hud-side-cam --hud-key tab 12 --hud-key slot4 14 --hud-burst shots/hud/b/f 14 0.3 16`
(Tab, then the melee attack in slot 4; compare with `reference/ingame/combat_*`).

**Floating text** (`combat.rs`, `combat/positions.rs`). Original scripts own the 30
`damagedisplay` widgets, colors, fonts, frame tables and critical atlas animation. The former
`combat.lua` method overrides and pooled Bevy `Text2d` renderer have been removed. Native
toolkit timelines now provide holds/transitions, movement, counting/reveal, shake and afterimages.
The server event bridge retains negative damage / positive healing and the original event order.

`AAUI.CombatTextPosition(source,target)` reads a separate projection cache, avoiding a borrowed
game-state callback during Lua events. It selects the target's identity, with empty/player targets
resolving to the player; unknown IDs never fall back to a different unit. Only active requested
IDs plus the player are projected. The system follows current camera/player/remote movement,
before `aa_ui::UiFrame`, so text follows moving units instead of a frozen first-hit world point.
Logical viewport coordinates become physical pixels before the hook. Unknown/offscreen positions
return nil: toolkit drawing is suppressed while the authored timeline still ends and releases its
slot. Measured plate height plus the existing scaled0.18m gap remains an app calibration, not a
recovered native height rule. Startup's immediate animation sample can be unprojected until that
frame's cache update.

The server-free `combat_hud` example loads actual game tables and the whole original world stage,
then moves fixture units and camera through this same app projection system. Its assertions check
that no method overrides remain, unknown IDs return nil, target identity stays separate and screen
coordinates actually move. `shots/hud/sol61-combat-native/native-final.png` was inspected: basic22,
critical128 with the original red burst and Battle End. Zero Lua errors,268-276FPS at1920x1080 on
the shared machine. The original melee reference was inspected for typography/behavior context;
this diagnostic scene is not a pixel-matched world capture. Live combat follow-up remains blocked
by the lane connection timeout. The toolkit's original30-slot nil/reuse regression also passes.

**Fight state.** The server only sends `SCCombatCleared` (when the player's fight ends, the kill); it never sends
`SCCombatEngaged` in these fights. The HUD enters combat itself when the player attacks or is attacked
(`HudData::local_combat`, in the snapshot's `in_combat`, so the diff raises `UNIT_COMBAT_STATE_CHANGED(true)`: the red
"Battle Start!") and leaves it on `CombatChanged` for the player, death or resurrection ("Battle End!", with the pink
"XP +14" from `EXP_CHANGED`). The packet of the killing blow follows the one that ends the fight, so damage within a
second of the end does not start a new one.

**Target ring** (`ring.rs`). The decal `textures/decal/target/target_enemy04.dds` under the selected unit, drawn over
the water like the real one: a grid of 12 x 12 points on the terrain around the unit is projected every frame and drawn
as a textured 2D mesh on the plate camera ("up" of the texture points away from the camera). Red (250, 80, 68) hostile,
green friendly player, yellow-gold friendly NPC, yellow neutral; the diameter is 1.2 m at least (the rat's 1.28 m).

**Cast bar** (`cast.rs`). The original's `player_casting_bar.lua` (and the target frame's) listens for
`SPELLCAST_START(name, ms, caster)` / `SPELLCAST_STOP` / `SPELLCAST_SUCCEEDED` (`caster` "player" or "target") and asks
`X2Unit:UnitCastingInfo(unit)` (`HUD.casting_info`: name, total ms, ms so far) each frame. `SkillStarted` with a cast time
starts a cast (the skill's name from the DB), `SkillFired` ends it (a channel keeps it and counts down), `CastingStopped` /
`SkillStopped` / death interrupt, plot steps with a cast time restart it. Recall (10 s) gives the capture's blue bar at
x 848-1070, y 910-928.

**Chat lines** (`items.rs`). `ItemsChanged` in the bag becomes `ADDED_ITEM` / `REMOVED_ITEM` ("Received: [item]",
"Used: [Heart's Beat]" when a skill consumed it, the reasons 13, 40, 41, 45; swaps and splits are silent); the link is the
item's name in brackets in its grade's colour (`item_grades.color_argb`; the capture's tan is the basic grade). "Gained 14
XP." is `EXP_CHANGED` with the gain (`X2Util:MakeMarkedCiphersStr` puts the thousands commas in).

**Loot** (`loot.rs`, aa-net's `Loot`). The server tells the client a corpse has loot for it (`SCLootableState`): two icons
appear beside the player (the capture: 25 px to each side, 0.52 m over the feet): F, three bags
(`ui/icon/interaction/icon_interaction20.dds`), loots everything (`Command::LootOpen` with `loot_all`), G, one bag
(`icon_interaction21.dds`), opens the window; both act on the nearest corpse within 8 m (assumed range). What is left comes as
`SCLootBagData` and the original's loot window (`x2ui/loot`) shows it: `LOOT_BAG_CHANGED(setTime)` (`setTime` for a "loot all":
the window lists what could not be taken for two seconds), `LOOT_BAG_CLOSE`; `X2Loot` answers from `HUD.loot_count` /
`HUD.loot_item` and takes an item with `HUD.loot_take` (`Command::LootItem`). The Angry Rat of the lane server drops nothing,
so this has only been seen with `--hud-fake-loot` (the icons match `reference/ingame/combat_death_2/real_08.png`; the
window is the script's). The wire formats are checked against the server's own byte tests.

**Tab** targets the nearest living enemy (the next one when pressed again), any unit if there is no enemy.

**Not done / assumptions.** The afterimage, shake and critical bursts; the plate shows no party, guild or PvP marks;
auto-away after 300 s and the loot range of 8 m are guesses; the capture's side-on camera is only approximated by
`--hud-side-cam` (the character is not exactly side-on); a unit's height has no source for models without a skeleton
and bounds (1.8 m times its scale).

### Max HP / MP

The server sends only the current points (`SCUnitPoints`); the client works the maxima out from the
database, as the server does (`aaemu-game`, `Unit::max_health` / `calculate_with_bonuses`). `formula.rs`
parses and evaluates the `unit_formulas` expressions; `stats.rs` feeds them:

* a character: `unit_formulas` kinds MaxHealth (14) / MaxMana (15), owner type 0, over `level` and the six
  primaries (kinds 22..27). A primary is the level formula plus the gear's stat (a port of the server's
  `Item::stat`: weights of `equip_item_attr_modifiers`, grade multiplier, `item_configs`, slot coverage) plus
  buff bonuses (`unit_modifiers` of owner type `Buff`, scaled by the buff level). Then the flat bonuses and the
  percent bonuses, in that order. A new level 1 character with no gear gets 370 / 320; the `bevytest` character
  with its starter gear gets 720 / 670, equal to what the server sends at spawn;
* an NPC: owner type 1, with `npc_template` / `npc_kind` / `npc_grade` variables (`unit_formula_variables`)
  and the NPC's own `unit_modifiers`; used for the target frame;
* `PlayerStats::set_max` hands the result to everything else that reads the maxima; the highest value seen
  is only the fallback while the data is not loaded.

### NPC grades

`npc_grades` is empty and the client has no table of its own: the grade is `npcs.npc_grade_id`, and the
scripts show one of eight strings (weak, normal, strong, elite, boss_a, boss_b, boss_c, boss_s). The mapping
of id to string comes from the grade multipliers of the unit formulas (6 weak, 1 normal, 7 strong, 2 elite,
5 boss_c, 4 boss_b, 3 boss_a, 8 boss_s), see `GameData::grade_name`.

### Global cooldown

Casting a skill starts the global cooldown of the skill that was cast: its `custom_gcd`, or one second for
`default_gcd` skills, unless a skill has `ignore_global_cooldown`. `aa_net::Skills` records the last own cast
(`last_cast`, `last_cast_skill`, set when the server's `SCSkillStarted` is not a failure); every slot of
a skill that does not ignore it shows the later of its own cooldown and the global one. A cast the server
refuses (a neutral target, for instance) starts nothing.

### Dark slots: what can be used now

The original decides in native code (`ActionAccessor::CanUseSkill`) and the scripts only hear the answer,
`OnContentUpdated(slot, "can_use", bool)` in `x2ui/components/action_slot.alb`: `false` lays a black layer of 75 % over the icon
and leaves the hotkey letter red. There is no separate "out of range" or "no mana" look, so every reason darkens the slot the same
way (`SlotSnap.blocked` carries the reason, `ingame.lua`'s `slotContent` turns it into `canUse`). The captures show it: the whole
bar is dark without a target or when the target dies, Recall and Dash are dark in a fight and lit again after "Battle
End!" (`combat_death`), everything is dark while Recall is cast and only the teleport book being used stays lit (`combat_return`).

`usable.rs` mirrors the server's `Skill.Use` checks over the 1.2 database (`skills`, `unit_reqs`, `buffs`, `item_weapons` with
`holdables`), in the server's order, so the text of a refusal is the one the server would send:

| # | rule | data |
|---|---|---|
| 1 | the caster is dead (`source_alive`), or alive for a skill meant for the dead (`source_dead`) | `PlayerStats` |
| 2 | the skill's unit requirements (`unit_reqs`, `or_unit_reqs`): level, ability level, race, gender, a worn item in a slot (a sword in the main hand) or by template, an owned item, a learned skill, "not in combat" (Recall, Dash, the dances), a buff on me / on the target, a shield, a bow or an instrument. Quests, doodads, zones and factions count as met: the server decides | `Snapshot`, `Inventory`, `Skills`, `Buffs` |
| 3 | buffs on the player: stun, sleep, knock down (every skill but those with `source_stun`), blank minded, silence (magic skills), crippled (melee and ranged) | `buffs` flags |
| 4 | the player is casting (every other slot is dark, the skill being cast stays lit) or channeling; a cast whose end never came stops counting 2 s after it should have ended | `Skills.casting` |
| 5 | the target: hostile, "others" and friendly-others skills need one (`NO_TARGET`), of the right relation (`INVALID_TARGET`: a rat for a heal, a friend for Triple Slash), alive or dead as the skill wants (`TARGET_DIED`) | `TargetSnap` |
| 6 | range: the skill's `min_range` / `max_range`, or the reach of the weapon in the equipment slot `weapon_slot_for_range_id` names (fists 3 m, a sword 4); edge to edge, so 1 m is taken off the centre distance | both positions |
| 7 | mana `((ability level - 1) * 1.6 + 8) * 3 / 3.65 * mana_level_md + mana_cost` (the skill's own ability level), labor points | `Snapshot.mp`, the ability exp |

An item slot is dark when its stack is empty or its use skill is (a potion goes dark while Recall is cast). The own cooldown and the
global cooldown are not reasons: their sweeps show them.

Before it sends `UseSkill` / `UseItem`, `use_slot` / `use_bag_slot` ask the same rules (`Env::precheck`): a use that cannot work is
not sent and says why. The client refuses less than the slot shows, so it never blocks what the server would allow: a target less than
2 m beyond the reach is still sent (the server knows the size of the bodies), the global cooldown is not refused (the server holds it
back quietly), the own cooldown is ("Can't be used right now."). The log has one line per change of the dark set:
`hud: dark slots [(2, "TOO_FAR_RANGE"), (4, "TOO_FAR_RANGE")]; target Some("Solzreed Fox") at Some(11.7) m`.

Checked on the lane server with account `bevyhud` (`shots/hud/r5/`): a fox 10.7 m away darkens Triple Slash, Melee Attack and Kick but not
Shoot Arrow (range 20, bow worn) (`far.png`); the angry rat walking up lights them one by one (log); `--hud-sim mp=10` darkens only
Triple Slash (30 mana) with the rat at 3 m and Kick (2 m) lit (`lowmana_03.png`); casting Recall darkens all but the Recall slot
(`cast_02.png`); `--hud-sim dead` darkens everything (`dead2_02.png`). Assumed, not seen in a capture: range and mana darken the same
way as the other reasons (no capture has a skill out of range or without mana).

### Refusals: the red text and the voice

Three things refuse an action and all of them end in the same alert (`alerts.rs`, `UseFailed`):

* the client's own check before it asks (above);
* the server's `SCSkillStarted` with a result and no timeline (`SkillStarted::failed`), for what the client cannot know;
* the server's `SCErrorMsg` (`Event::ServerError`): `SYSMSG(text)` with the text of the error's name in the `ERROR_MSG` group
  (`error_names.txt` is the server's `ErrorMessageType`: `BAG_FULL`, `NOT_ENOUGH_MONEY` ...). A code without a text keeps the red chat
  line "Server error N".

The scripts do the showing, as in the original (`center_message_manager.alb`): `SKILL_MSG(name, arg1, "")` with the skill result's name
prints the text `skill_<name>` of the system texts through `formatSkillMessage`, red `XLARGE` text 180 px under the top edge
(`systemAlertWindow:AddMessageRefresh`). `arg1` names the item or buff a requirement wants. Texts seen: `NO_TARGET` "Select a target.",
`TOO_FAR_RANGE` "Target is too far.", `TOO_CLOSE_RANGE` "Target is too close.", `LACK_MANA` "Insufficient mana to use this.",
`INVALID_TARGET` "Invalid target.", `SOURCE_DIED` "Can't be used while dead.", `SOURCE_ALIVE` "Can only be used while dead.",
`URK_COMBAT` "Can't be used in combat.", `ON_CASTING` "Already performing an action.", `COOLDOWN_TIME` "Can't be used right now.".

A refusal also speaks: the character's `<race>_<gender>_system_message` sound pack (86 to 98, `sound_packs`) has one voice line per
result (`FAILURE`, `NO_TARGET`, `LACK_HEALTH`, `LACK_MANA`, `OBSTACLE`, `OUTOF_HEIGHT`, `TOO_CLOSE_RANGE`, `OUTOF_ANGLE`; all the
packs hold the same eight). `TOO_FAR_RANGE` has none: the database leaves its sound out (sound 42, "do not use"), and so are all the
results without an item of that name silent (an assumption: the native code is not read). The same line is not repeated within 1.2 s.
The log says what was shown and played: `hud: client refused "LACK_MANA" (0) for skill 18132 "Triple Slash": Insufficient mana to use
this.; voice sounds/x2voice_system:message:lack_mana` (needs `AA_AUDIO_LOG=1` for the voice path, else the sound packs are not read in a
muted run).

`AbilitySwapped` for the player raises `ABILITY_CHANGED(new, old)` with the archetype keys (`wild`, `fight`: the scripts' `abilityCategory`
is keyed by name): the chat prints "The Battlerage skillset is now Archery." and the unit frames refresh their ability icon.

### Death window

When the server says the player died (`DeathDetails`) the original window comes up (`death_and_resurrection_window.alb`: "Hereafter
Threshold", "Your last breath has escaped you.", the countdown, "Revive Here" / "Respawn: Temple"); `Resurrected` closes it.
`X2Player:GetResurrectionCountdown` counts the server's wait down on the HUD clock, `X2Player:Resurrect(inPlace)` sends
`Command::Resurrect { in_place }`, `X2Unit:GetUnitNameById` answers for the player and the target. Seen with `--hud-sim dead` only
(`dead2_02.png`: all slots dark, "Revive Here" greyed by the script's own zone rule); a real death on the lane server and the
server's answer to the buttons are not checked.

### Quest marks

`quests.rs` decides who carries a mark, `marks.rs` draws it. Without anything from the server about who
starts a quest, the client works it out from its tables, as the original does:

* an NPC offers a quest when a Start component (`quest_components.component_kind_id` 2) has a
  `QuestActConAcceptNpc` act for it, the player has neither taken nor (for non repeatable quests) completed
  it, and the component's `unit_reqs` hold (level, race, quests done, items ... the server's `req_kind`
  numbers; a kind the client cannot check holds): a yellow "!";
* the NPC where a quest is handed in (`QuestActConReportNpc` on the Ready component) shows a yellow "?" when
  the quest is ready and a grey "?" while it is in progress.

The player's quests are `aa_net::Quests` (`SCQuests`, `SCCompletedQuests`, started / updated / completed /
reset packets). A mark is the original's skinned model (`objects/ui/quest_mark/exclamation_mark_red.chr`
and `question_mark_red.chr`, the yellow / grey materials of the `*_mul.mtl` files, unlit) loaded through
`aa-cry`: twice the model's size, 0.72 m above the plate's anchor, turning at 1.2 rad/s and bobbing 5 cm.
The original's `Illum` material (diffuse 0.2, emissive 0.1, a strong orange specular and glow) comes out as a
flat bright gold, (217, 214, 70) at the top and (211, 164, 54) below; the world's post pass (fog, grade, film
curve) washes a plain unlit texture out to (238, 232, 145), so the yellow marks draw a small float gradient
texture (`RAMP_TOP` / `RAMP_BOTTOM`, scene-linear, blue slightly negative) calibrated to come out of the post pass as the
capture's colours within 3 %. The look is the capture's (about 40 px tall, 4-5 px above the name). Marks are refreshed every 30 frames for
NPCs within 45 m. The white beams over Maude in the capture are a particle effect of the NPC, not drawn.
Quests can be tried with the GM chat commands `/quest add 330` / `/quest remove 330` (`--hud-type`), which
also exercises the live updates.

### Map icons (roadmap and big map)

`hud/markers.rs` + `hud/markers.lua`. The original's scripts only prod natives (`X2Map:SetNpcFilter`,
`SetNpcShowFilter` / `SetDoodadShowFilter` / `SetHouseShowFilter`, `roadmap:SetRoadMapNpc`, the roadmap option
data); the icons are drawn natively from `ui/map/icon/npc_icon.dds` (24 px cells, the symbol names of
`map/common.lua`) and the player is `player_cursor.dds`, turned by the heading.

* **Where things are.** The client's own `worlds/main_world/map_data/npc_map/<zone>.dat` and `doodad_map/<zone>.dat`
  (16 byte records: template id, x, y, z, game metres) list every NPC and map-worthy doodad of a zone group. The
  database says what they are: NPC service flags -> bank, auction house, stabler, expedition, trade, ability
  changer; the doodad's `group_id` -> mailbox, workbenches, portals, memory tomes (the blue book, `doodad_inn`,
  mostly hidden behind the bank chest), housing province signs (`doodad_icon`). Weapon / armor / general
  merchants and priests get no icon: `reference/ingame/worldmap_zone` shows none (only the bank, mailbox,
  memory tome and stables of Desireen, the farm sign west of it).
* **Quests** come from the units the client has, like the 3D marks (`game.quests.mark`: "!" yellow, "?" for ready,
  grey "?" in progress, the main / livelihood / repeat variants of the sheet). The capture's big map shows no "!" at
  Desireen, 170 m from where it stands, and its roadmap the two givers within 120 m (Maude and an NPC south west).
* **Gravesite** (`corpse_pos`) while the exp lost can still be won back; **dots** (`hostile` / `friendly`) of the
  units in view within 250 m when the roadmap's "NPC" check is on.
* **Filters.** The "Show Icons" window of the big map works: `markers.lua` keeps each check (`SetNpcFilter`) and the
  three folders (Merchants, Infrastructure, Housing Province) and leaves the filtered icons out of both maps; the
  roadmap's NPC check hides every npc-like icon until it is on, as the toolkit's `roadmap` widget does.
* **Placement.** Rust hands `HUD.map_icons` (game x, y, symbol, name, city) to Lua; `markers.lua` turns them into
  the fraction of the zone rectangle (the big map's zone level, the whole image) or of the road map image (the
  roadmap, through `HUD_ROAD_FRAMES` / the form frames) and of the city's rectangle (level 3) and hands them over as
  `AAUI.RoadMap().markers` / `AAUI.WorldMap().markers` with a `markers_version` (an unchanged version costs no
  re-read). aa-ui draws them (`Marker` in `roadmap.rs`, `place_markers` in `worldmap.rs`) and shows the tooltip of
  the icon under the mouse through `SHOW_WORLDMAP_TOOLTIP`.
* The list is rebuilt every 30 frames and only pushed to Lua when it changed; the database tables are read once on a
  background thread (own connection).

### Bag tabs

The tabs are `default_inventory_tabs` / `default_inventory_tab_groups` (served by the fake bag data); an
item belongs to the group of its category (`items.category_id` -> `item_categories.item_group_id`, the
`group` field of a bag slot). The All tab shows the slots as they are, the others the items of their
groups one after the other (`SlotByIdx` in `ingame.lua`, cached per bag change). Tabs the player creates
live in the fake bag data, so they are lost with the stage; the original keeps them in its settings.

### The stage is prebuilt during Loading

Building the Lua stage takes 0.35-1.5 s. `prebuild_hud` does it during `Screen::Loading`, once the player has
spawned, with a second `UiRuntime` next to the loading screen's, and a little per frame: `BuildingStage` is
`UiRuntime::run_stage` taken apart (the init script, then the stage's scripts in the same order, one
script per step) and runs steps for 8 ms of each frame (`BUILD_BUDGET_MS`, or `AA_HUD_BUDGET_MS`; a huge value
builds in one frame to compare). Listing the stage's scripts (the pak listing for the content directories and
reading the table of contents files, 20-90 ms) is done on another thread. What is left to the main thread is the
Lua itself: the longest single script is `skill/main_window.alb` (30-40 ms), so no loading frame is held up much
longer than that. Release, loading screen worst frame (`flow: loading took ... (worst frame)`): 322-326 ms with
the whole build in one frame, 51-53 ms chunked; the stage is ready after 40 frames, about 0.6 s. If the game
starts before the stage is done, `build_hud` finishes it in one go. `build_hud` then swaps the stage in (about
20-50 ms: the snapshot is refreshed, the world stage's events fire again, the renderer is told to redraw
everything). It is dropped again on the way back to the world, character select or create screens. Without a
prebuilt stage (`--enter-world`) `build_hud` builds it on entry.

### Tooltip numbers

The client fills the `#{key}` placeholders of a skill, item or buff description itself (C++), the Lua only prints the
result. Here `tipcalc.rs` does it (`TipTables::skill_text` / `buff_text`), `tipitem.rs` computes the weapon and armor
stats, `tips.rs` puts both into the tooltip tables the original `baselib/tooltip/tooltip.lua` reads. Both are
ports of the server (`combat.rs`, `stats.rs`, `models/item.rs`), so the tooltip shows what the server will do.

**The player's numbers** (`TipState`, `Attrs`): the six primaries (character formula + gear stats + buff
modifiers, flat before percent), the weapon dps of each hand (weapon dps x 1000 + stat / 5 x 1000: strength for the
main hand, dexterity for the bow, intelligence for spells, spirit for heals), `dps_inc` (formula kinds 21 / 28 / 29 /
43), the damage multipliers `(cwb(0, attr) + 1000) / 1000`, the level dps (formula kind 12 with `ab_level` = level)
and the ability levels. It is recomputed only when the level, the equipment, the buffs or the ability points
change (`TipState::refresh` in the snapshot pass of `api.rs`), never per frame. Checked against
`reference/ingame/character_info/real.png` (level 1 Battlerage in the starter gear): 19.42 melee, 16.81 ranged,
2.00 magic, 2.00 healing, 269 physical and 377 magic defense, all reproduced (test
`a_new_characters_numbers_match_the_capture`; the 200 over the cloth is the shield's gear buff "Equip Shield").

**Skills.** Which effect a key reads: `tooltip_skill_effects` first, else the first effect of that kind that applies at
the character's level, then the plot's effects, then the effects of a buff's `triggers` (what runs when a buff
ends). Damage is `level_range(lvl_md, rank, start, end)` + `dps_inc` + the weapon's dps scaled by the cast time
(`max(1000, casting ms)`), spread by the holdable's `damage_scale`, times the damage-type multiplier; heal, mana
burn and mana restore are the same with their dps. Ranges print `min-max` (`avg_*` the middle), `buff_total_*` is
the per-tick value times `duration / tick`. Durations print as the client's `FormatDurationTimeInDesc` does
("0.7sec", "30m", "1h", no space, mixed ones joined by a space); the number format is `|cFFFF9C27n|r` (`ui_texts`
4012). `buff_duration` / `duration2` / `duration3` are the first, second and third buff with a duration (a channeled
skill falls back to its channeling time). A buff's level is `max(ability_level, player_points / level_step *
level_step)` (without that ability: `ability_level + (rank - 1) * level_step`); `duration + level_duration * level`,
and `ua_*` is `value + linear_level_bonus * level / 100`, converted per attribute by the Lua
`GetCalculatedAddModifierValue` rules (percent, per mille, tenths of a second ...). Mana is
`((ab_level - 1) * 1.6 + 8) * 3 / 3.65 * mana_level_md + mana_cost`.

**Items.** `ItemTables::stats(template, grade, temper_p, temper_m)`: the stat bonuses (`mod_set` weights,
grade `stat_multiplier`, `item_configs`), the weapon dps / magic dps / heal dps / armor from the holdable formulas with
the item level and grade (the temper factor over 100 on top), one hit's min / max damage (`dps` x attack delay,
spread by `damage_scale`), armor and magic resistance of clothes (wearable formula x kind ratio x coverage), the
maximum durability, repairable, the set (`equip_item_set_bonuses`, bonus text = the bonus buff's description filled
with the item's level), the slot type name. The tooltip table gets: `slotType`, `DPS`, `minDamage`, `maxDamage`,
`attackDelay`, `magicDps`, `healDps`, `extraDamage1..3`, `armor`, `magicResistance`, `durability` /
`maxDurability` / `repairable` (the real wear when the slot carries it: `dur`, `tp`, `tm`, `fl` are extra fields of
the slot table, from `Inventory`), `modifier` (the stat bonuses), `equipSetInfo` (`equipSetItemInfoDesc`,
`equippedSetItemCount`, `maxSetItemCount`, `setItems`, `bonuses` with `numPieces` / `satisfied` / `bufDesc`),
`itemUsage`, and `soul_bound` (the instance flag) or `soul_bind` (`soulbound_pickup` / `_pickup_pack` / `_equip` /
`_unpack`, from `items.bind_id`, for an item not yet owned). Compare tooltips: the original Lua asks
`X2Equipment:GetEquippedItemTooltipInfo` for the worn item(s) of the slot and shows them beside the hovered one; that call
returns the same kind of table (`equippedInfo` in `ingame.lua`, with the wear of the worn item).

**Tests** (`cargo test -p aa-client hud::tip`, they read the real DB and skip when it is missing): every visible
skill of the ability trees and of `default_skills` has no placeholder left (`the_starter_skills_have_every_number`); the new character's attack and defence numbers match the capture; Flamebolt prints a damage range and its
burning time; the starter gear has the stats the DB gives and a better grade or temper raises them.
`cargo test -p aa-client tips_coverage -- --ignored --nocapture` lists what is left over all 840 skills and 3461 buffs
with placeholders.

**Assumptions and what is missing.**

* The real client shows the damage breakdown (`$1 + $2% Melee Attack`) only with the option "Show Damage Breakdowns
  in Skill Tooltips" (default off); it is not offered here, so the `details` stay empty as in the default client.
* A skill's effect level filter uses the character level; a debuff's numbers on another unit use the player's
  attributes, not the caster's.
* `*_critical_bonus` uses the 0.2 scaling; `ea_*` are the tooltip-listed effects. `return_point` prints empty.
* Not filled: placeholders of mob / vehicle / event skills (the ability-0 skills of the DB that no player has; about 40
  show `?`), the doodad-driven `ea_*` of Missile Rain / Rain of Fire, buffs without a duration (Stun's `#{duration}`),
  empty-data buffs' `ua_*` (`10363+`), the `din_damage` typo of the DB.
* Weapons: gems, tempers and enchants add `extraDPS` / `extraArmor` in the original; only the temper factor is applied.
  No repair cost, `dyeingType`, `gender`, `scalable` / `convertible` lines.

## How it works

* `Screen::InGame` + the player spawned -> `build_hud` puts the stage built during Loading in place of the
  shared `UiRuntime` (or, without one, switches it to a fresh Lua state with `login::host::switch_stage`):
  `gameapi::install`, the fallback fakes, `hud/ingame.lua` (the real data layer), the world stage's scripts,
  then `ENTERED_WORLD`, `LEFT_LOADING`, `ENTERED_SUBZONE`, the joined chat channels and the tutorial
  (level 1). `OnExit(Screen::InGame)` forgets the build.
* `api.rs`: `HudData` (in the non-send `HudBridge`) holds a `Snapshot` of the aa-net resources, the game
  database (`data.rs`: skills, items, buffs, NPC templates, zones, factions, sub zone polygons) and the
  requests the scripts made. `sync_hud` refreshes the snapshot every frame and `push_changes` diffs it
  against what the scripts last saw, firing only the events that changed.
* `panels.rs`: action bars, bag, equipment, buffs and target as plain `SlotSnap` / `BuffSnap` lists, pushed
  per group when they differ (`refresh_slots`).
* Lua to Rust goes through the `HUD.*` functions (`player_pos`, `faction_name`, `request`, `now_ms`,
  `target_distance`, `skill_info`, `item_info`, `chat_window`); requests (chat, leave, cancel leave) are
  taken by `handle_requests` and become `NetCommand`s.
* Countdowns are absolute end times on `HUD.now_ms()`, never per-frame pushes.

## Parties and saved settings (2026-10-17)

`party.rs` / `party.lua` bind the original party and raid frames to `aa_net::Party`, including sparse
one-based `team1`..`team50` aliases, HP/MP, names, levels, abilities, offline/death state, authority,
member targeting, and local frame visibility preferences. Original popup actions send invite,
leave, kick, dismiss and transfer-leader requests. Invitations use the original localized
Join/Decline dialog, with replies tied to the current invitation sender. Membership changes wait
for the server; native UI event argument shapes are preserved. The roadmap and world map show
party/raid/leader/offline icons in the current zone and instance.

`settings.rs`, `settings_options.lua` and `settings.lua` share option values between all screen
Lua states and persist applied edits per account under `%LOCALAPPDATA%/AABevy1.2/settings/`.
`AA_SETTINGS_FILE` selects an isolated file for development. Account filenames use lowercase
UTF-8 hex encoding. Options save when the original UI calls `X2Option:Save`; sound changes also
reach the audio mixer immediately. Cancel, Escape and close restore the window's opening values,
including after Reset reinitializes controls. Bag tab names, icons, groups and selection save on
edit/switch; bag saves do not commit unapplied option edits. A temporary file replaces the previous
JSON after a successful write. Graphics option values are retained; renderer quality/resolution
controls and editable key bindings still need their runtime integrations. Raid role/move/convert,
looting-rule editing, unit-frame marks and map pings are now integrated below. Party buff/combat
indicators and 3D overhead marks remain separate work.

Validation: `cargo test -p aa-client --bin aa-client` passes 97 tests, with 13 intentional probes
ignored. This includes real Options Lua Reset/re-Init/Cancel, bag restoration, file replacement,
corrupt-file recovery, sparse raid aliases, command character/object IDs, and batched leader-transfer/disband
notifications. The live two-client
party protocol test passed twice in aa-net. The integrated full-flow client joined a fresh headless
peer, showed its original party frame/leader mark and localized join message without Lua errors:
`shots/hud/sol61-wave2/party_08.png`, about 85–94 FPS at 1920x1080 after joining (shared debug build).
`--lua ingame:CODE` now supports unattended checks after the full login flow.
The restart run verified persisted master volume 0.4, ShowBuffDuration=0, custom Supplies tab
(icon 3, groups 1/7), its selected index, Reset/re-Init/Cancel, and party-frame targeting with Lua
assertions. `shots/hud/sol61-wave2/restart-bag.png` shows the restored bag. The peer observed the
real offline/rejoin snapshots, then left/logout cleanly. Both client runs had zero script errors
and no warning/error log entries; test processes exited automatically.

## Cost

`aa-dev: fps=` in the spawn scene, debug build, shared machine (noisy, +-5 fps and worse while other agents
build): round 1 measured 63 fps / 16.0 ms with the HUD against 70 fps / 15.0 ms without, about 0.7-1 ms per
frame. Round 2 adds the quest marks (a handful of small models, refreshed every 30 frames) and the formulas
(evaluated when the gear or buffs change): full flow runs show 64-72 fps in game. Entering the game costs
about 20-50 ms instead of 1.5-2 s because the stage is built during Loading (`hud: UI stage ready in
580-1300 ms` while loading; `hud: swapped in the stage built during loading (19 ms)`).

## Not done

* The target frame's job icon for another character reads `Unit::active_abilities` (`GetTargetAbilityTemplates("target")`): the data path
  is unit tested in aa-net, but no other character was in view on the lane server to see it.
* Dark slots: buff restrictions, equipment and `or_unit_reqs` rules are covered by unit tests (`usable.rs`), the lane run only
  met range, mana, casting, target and combat. The unit requirement kinds that need quests, doodads, zones or factions count as met.
* Map icons: quest objective areas and the numbered notifier icons
  (`X2Map:AddNotifyQuestInfo`), icons for other zones than the one the player is in, the real client's `npc_station`,
  `npc_graveyard`, public farm and live housing icons (no data source), ship markers.
* The white beams of the quest effect over an NPC (a particle effect of the NPC, aa-fx).
* Bag/equipment/action-bar drag requests are now routed through `drag.lua` / `drag.rs`;
  their live visual coverage is still being completed.
* Tooltips: what is still open is listed under "Tooltip numbers" (the damage breakdown option, `ea_*` / `return_point`,
  the buff caster's own stats for debuffs on others, gems / tempers / enchants, repair cost).
* `AAUI.MapClimate`, `ConflictZones` and `ConflictState`.
* Quest marks ignore requirements the client cannot check (they hold), so an NPC may show a "!" for a quest
  the server then refuses.

## For the other crates

* ingame: skip key bindings while `HudKeyboardFocus.0` and camera orbit / click-to-move while
  `HudPointerOverUi.0` (done in round 2).
* aa-ui: the roadmap markers in `AAUI.RoadMap` still take the image
  fraction (`markers.lua` knows the image frames); a window fraction would save that.


## Commerce and keyboard integration (2026-10-17)

`hud/store.rs` loads original merchant packs/goods from the read-only database, keeping the native
first-item-wins catalog order and stock grade IDs. Native currency mapping treats siege packs as
honor stores. The original purchase/cart/sell/buyback windows call real aa-net shop commands;
inventory and money follow server item tasks. The buyback list is refreshed after STORE_SELL=53
and STORE_BUY=54. Local and server state are not optimistically changed.

Grade IDs on the wire are database IDs (0 Basic, 1 Crude). Lua/icon/tooltips use grade_order
(0 Crude, 1 Basic); bag, equipment, action items, loot, commerce and item chat now convert at that
boundary. Explicit display grade zero stays Crude. Refund uses the actual grade multiplier and
per-unit truncation; zero refund is retained even for a priced item.

`hud/bank.lua` feeds the original lazy bank window with server slots/items, balances and money
requests. Bag/bank clicks, right-click transfer and partial-stack dragging share inventory move
planning and real item identity. Bank filter tabs are independent of bag tabs and are saved in the
same account-local settings file. Bank capacity/items are pushed separately; original BANK_UPDATE
and PLAYER_BANK_MONEY events refresh their widgets. NPC context ends on target, range, logout or
window close. Remaining: sorting, expansion dialog/certificate flow, full lock/new-item behavior,
merchant service choice for NPCs that also offer quests, and unsupported AA-cash services.

Original PLAYER_MONEY receives `(numeric_delta, decimal_delta_string)`; the balance is read via
GetMyMoneyString. A real sell/buyback exposed the former balance-string argument error; the new
regression checks both money events and current state. Native keyboard/modifier hooks are restored
after fallback scripts. `HudInputReady` follows aa-ui's input set and consumes UI-owned keys before
camera, movement and HUD bindings, including the release frame and keys held after capture ends.
Actual account-backed key rebinding and gameplay action maps remain a following milestone.

Validation: client 103 passed, 13 intentional data/timing probes ignored; live commerce
through original submit/rebuy handlers confirmed 33-copper sale, same-item buyback and a 25-copper
purchase. `merchant-final_09.png` and `bank-final2_04.png` were inspected at 1920?1080. The original money
edits/deposit/withdraw handlers confirmed 8?3?8 bag copper and 0?5?0 bank copper. The paid seed
moved bag slot4?bank slot0?bag slot4 with UID16777730 throughout. Bank tab Stored was saved
separately from the bag. Logs have zero Lua handler errors; representative in-game FPS was
70?87 on this shared machine. The catalog/rebuy flow remains the original Lua with live data.

The original bank injector calls native `X2Bank:MoveToEmptyBagSlot`, now implemented explicitly;
replacing a similarly named Lua helper before stage load is insufficient because the stage defines
its own helper. Sale cart locks retain the exact instance ID as a decimal string. The queued sale
refuses an item that replaced the marked slot; clearing UI locks after Submit cannot erase the
queued identity. Captured UID is never passed through a floating-point Lua number.


A second real session checked cart lock/unlock and same-UID buyback with two sold entries, selecting
the matching original rebuy row rather than assuming list chronology. Closing the window ended
its NPC context. A later `--enter-world` cold run opened the bank with the saved sixth Stored tab,
zero script/handler errors and 79?84 FPS (`cold-bank.png`). Full-flow and direct world entry both
work with actual spawned data. The earlier empty-data `run_stage(World)` unit probe hang was not
reproduced by this actual cold path; it is not counted as a client-flow failure.

## Raid controls and authoritative map pings (2026-10-17)

The original raid manager's conversion, role buttons and drag handlers now send `PartyCommand`
requests with character IDs and physical member slots. Empty-group drops choose the first free
slot. Authority, slot bounds and role values are checked locally; membership stays unchanged
until the server acknowledges it. Native moved events receive the two global one-based slots,
not their group numbers. Loot helpers read and edit current method/master/display grade/BoP
state; the changed-field notices carry the original typed arguments. Raw item-grade IDs are
converted at the Lua boundary.

Overhead-marker commands resolve character aliases to character IDs and NPCs to object IDs.
Server-confirmed marks resolve back to visible object IDs for the original unit-frame markers,
including when a marked character becomes visible later. Clear/toggle/reset and raid-owner
permissions are enforced. The separate 3D overhead glyph rendering is still pending.

`party/ping.rs` and `party_ping.lua` bind the toolkit's optional map-ping hooks to real commands.
A click never places an optimistic local marker. The server echo supplies named projections
for every overlapping world/continent/zone/city rectangle (bounded to 256); the roadmap applies
its authored image crop. Repeating an identical echoed point restarts the pulse, a clear removes
both markers, and a ping for another nonzero instance is hidden. Solo, leader and character-marked
members may send pings. The lane's initial true ping at (0,0,0) is retained as received.

Native world-map XY is rectangle origin plus `(u,1-v)` times its size. The recovered native Z
rule is the maximum of two surface queries plus 2 metres; current terrain and authored-water
queries match ground/lake/sea placement. Brush roofs and instance-specific collision surfaces
are not yet queried. There is no fallback to the player's height. Native instance selection is
the current instance only for a matching current map context, otherwise zero. Removal sends
false, zero position and zero instance.

Validation: six focused party/ping tests pass, including sparse drag/role/loot identity, native
event shapes, nine overlapping city projections, lake surface+2, repeat pulses, no optimistic
display and instance filtering. Fresh root character Solraidtest (47) and headless peer (48)
formed team9. The original conversion/role/drag/popup helpers confirmed owner tank role, peer
healer role, peer move to group2 slot1, loot-master/grade/BoP edits, marker identity, both peer and
root ping echoes, original `RemovePing`, reset and original raid dismissal. No area invite was
broadcast. `shots/hud/sol61-raid/raid_04.png`, `_05.png` and `_06.png` were inspected: original map
ping pulse and original raid roster/role colors. There were zero Lua errors. Representative
1920x1080 FPS was 68-95 on the shared machine. Geometry/icon positions are not claimed as an
exact reference match without a corresponding real raid capture.

## Friends and blocked users (2026-10-17)

`friends.rs` binds the original relationship tabs and character/chat popup predicates to
`aa_net::Friends`. Add/remove/block/unblock queue real commands; list membership changes only
on acknowledged state. Self/duplicate/missing-entry checks are local preflight notices. Silent
lane refusals time out after20seconds with a localized failure notice, without inventing a
server reply or changing membership. Online status updates refresh rows and online counts;
returning friends use the original localized online-acquaintance notice.

Native list rows retain columns1name/2level/3abilities/4location/5race/6online/7in-party.
Deletion sends the name string instead of a row. Pagination filters offline entries before
stable level-descending sorting (native merge39302f90); `RequestFriendList` is a local cache
query. The app uses ten rows per page. Native's runtime page-size global is still unresolved,
so ten is an explicit app choice. Unused English column4 shows localized Offline; the native
offline elapsed-duration formatter remains unrecovered. Block-list rows retain their one-name
shape. `GetFriendList`, `IsMyFriend` and `IsBlockedUser` support the original popup/gift callers.

Live/replayed displayed chat filters blocked character names case-insensitively; raw network
events/logs remain intact, and system lines are retained. Login list snapshots survive stage
construction and feed the first UI queries. Both HUD entry paths wait for background game
tables before loading native scripts; an empty-table original-stage fixture stalled, while
the same fixture with actual game data loaded completed its stage in about2seconds.

Validation: 110 default client tests pass, with 14 intentional probes ignored. The separate
original-stage fixture passes with actual game tables: the original blocked-user popup queues
Unblock, a simulated acknowledgement removes its row, the original offline checkbox refreshes
the friend row and count, and its original delete popup queues Remove. Focused tests cover native rows/name deletion, commands without
optimistic mutation, filtering/sorting/pagination and blocked live/replayed chat with the raw
log unchanged. A fresh headless peer55 whispered only our client47 every5seconds. The original
friend Add button confirmed its online row; the original Block button confirmed membership,
and12seconds of incoming whispers were absent from displayed chat. The inspected original
block-tab capture is `shots/hud/sol61-social/social_02.png`, about78-88FPS at1920x1080.
The first popup fixture mistakenly read a global where original Lua has a local upvalue;
the corrected fixture retains the original constructor result. Subsequent live follow-up is
blocked by the lane-server connection timeout after the separate auction/refund test. Native
unblock/offline/delete script checks pass in the isolated original-stage fixture. Full
live unblock/offline/restart coverage is explicitly pending. The headless peer logged out;
all root test windows exited.


## Original mailbox integration (2026-10-17)

`mail.rs` binds the original received/sent lists, unread count, detail tables, date tables,
individual money/item claims, sequential claims, deletion, outgoing normal/express mail and
housing-tax request to `aa_net::Mailbox`. Mail IDs and header `extra` remain decimal strings;
`chargeMoney` is the native numeric second money field. Item claims clone the cached original
Item.Write attachment, including detail bytes. No request changes inventory or balances locally.
Body/header queries and success events use acknowledged state. Batched prepare-body/claim packets
retain physical attachment indices, and a completed sequential claim advances its original Lua
queue once. The first synchronization seeds a new stage even if the resource did not change.
The attached-count query reads the authoritative header byte (native3948d550/393cce70);
the item-count query counts only nonzero instances with valid item definitions
(native3948d010), independently of all three money fields.

Ctrl-E opens the original mailbox locally. It chooses a currently visible NaviOpenMailbox
function group within the lane's five-metre 2D send radius, when available; inbox reading also
works without a mailbox. Native393b1040 emits numeric doodad ID plus boolean true; there is no
invented interaction acknowledgement. The broader native interaction chooser and exact automatic
end-distance behavior are still pending. Sending revalidates the selected mailbox against all
currently nearby mailboxes, rather than requiring it to remain the nearest. Hotkey consumption
now completes before gameplay input, so Ctrl-E does not also move the character right.

Outgoing drafts use ten local attachment slots and real physical bag addresses. Whole-stack
pickup records exact unique IDs as strings, rejects partial drags and bound items, locks attached
slots, and checks identity again at dispatch after the original button clears its cart. Money
parsing checks arithmetic and coin ranges. Only one send is outstanding through this UI; the
write window is disabled while awaiting its response. Failure, local rejection or the explicit
20-second app timeout restores the cleared text/coins and still-valid attachments without a fake
ACK. Timeouts do not change network state or automatically retry. Drafts are not persisted over
process exit. The native digit-edit getter returns its initial value (normally zero) for an
empty numeric field and writes that text back without a change event. The toolkit implements
that behavior from native392930c0; the original mail fee calculation consequently shows100c
express/50c normal with one item and no money, without an app-side edit override.

The English gold-only client returns gold currency and zero from GetCurrentMailAAPointStr.
Native3948f4b0 likewise returns literal zero with feature0x1000 disabled; when enabled its value
is the raw third money field, not billing_amount. That raw amount remains retained/countable, but
AA-point claims/currency mode, commercial/MIA classification, complete system-mail variants and
quest-mail sender resolution remain pending. CanSendMail is the account/trial gate; the lane
exposes no trial entitlement, so it is permissive and proximity is validated separately. Return
has no usable1.2 lane opcode and reports unavailable; sent-mail deletion and generic COD retain
their documented lane limitations. General failure notices currently retain raw result codes.

Validation: 111 client tests pass, with15 probes ignored. The isolated original-stage mail test
passes with real tables: native list double-click queues Read, its acknowledged body opens the
original detail view, the native money button queues the correct claim, the original Write/Send
buttons queue the expected recipient/text/mailbox, and failed-send recovery restores the form.
Focused tests preserve IDs above2^53, exact attachment bytes, nonoptimistic claims, physical bag
slots, whole-stack identity, locks and checked copper. `mailbox_hud` is a server-free aa-dev
example with read and --compose views. Inspected `read-native.png` and `compose-native.png`
under `shots/hud/sol61-mail/` show the original controls with a real Heart's Beat template,
4-item stack,5c incoming and50c/100c outgoing fees. Both have zero Lua errors; read measured
303?465FPS and compose404?455FPS at1920?1080. Both root-owned windows exited. Source fixtures do not substitute for live transaction verification: the lane game
connection timeout still blocks that follow-up.
