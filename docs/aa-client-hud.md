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
| emotes (`/sad`, `/clap` ..., emote skills on the action bar) and speech bubbles (say, party, raid, guild chat; `SCChatBubble`) | `aa_net::EmoteEvent`, `Command::ExpressEmotion`; `speech.rs`, `ingame/emote.rs`; see "Emotes and speech bubbles" |
| roadmap: zone name, sub zone, faction, heading, clock | zone from the DB (`WorldInfo` zone group, then position), sub zones from the pak's `subzone_area.xml` polygons (the DB's rectangles are mostly 0), the clock from `aa_world::WorldClock` (the server's, or `--hour`) |
| sub zone header and the city map | the header is the top of the sub zone's `parent_sub_zone_id` chain (the Royal Palace is in Ezna: "Ezna"). Where that sub zone has a road map of its own (cities, caves: `HUD_ROAD_FRAMES[3]`) the roadmap is that map (level 3, id = sub zone), placed from the sub zone's `x, y, w, h` and the image's frame in the 926 x 556 roadmap window (`road_frames.lua`, generated from the client's `map/road_map_list.lua`); the zone maps are placed the same way (checked on `reference/ingame/spawn`: window pixel 703.2, 147.7; and `royal_palace`: Ezna 770, 363) |
| the big map (M) | `AAUI.WorldMap`: the player's place on the city or zone map (plain fraction of the rectangle) and on the continent and the world (`world_groups`) |
| tooltips: bag items, action bar and use-skill, buffs, equipped items | `tips.rs`: `HUD.item_tip` / `HUD.skill_tip` / `HUD.buff_text` fill the client's tooltip tables (`name`, `grade` / `gradeColor` from `item_grades`, `category`, `item_impl`, `soul_bound` / `soul_bind`, `skillType`, `description`, price, `tipType = "skill"` with ability, rank, mana, cast, range, radius, cooldown) from the DB; `slot.tooltip`, `X2Bag:GetBagItemInfo`, `X2Skill:GetSkillTooltip`, `X2Unit:UnitBuff` (`tipType = "buff"`) return them. The numbers in descriptions (`#{avg_damage}`, `#{buff_duration}`, `#{ua_*}` ...) are worked out in `tipcalc.rs`, the weapon and armor sections (attack, defence, speed, stat bonuses, durability, set bonuses) in `tipitem.rs` (see "Tooltip numbers" below) |
| unit frame tooltip (hover the player / target frame) | the original script formats the text with the `UFT_*` codes `|p1;` .. `|p11;` (name, hp, max hp, hp %, mp, max mp, mp %, labor, max labor, pvp honor, hostile faction kills); `SetTooltipData(unit)` binds the unit token to the tooltip widget and aa-ui substitutes the codes from `AAUI_TooltipUnitValues(unit)` (`ingame.lua`: `X2Unit` hp / mana, `floor(100 * cur / max)`, labor for the player, `Unit::pvp_honor` / `pvp_kills`), again every frame while the tooltip is shown. The pvp values come from the unit state and `SCUnitPvPPointsChanged` (0x204; kind 0 honor gained in combat, kind 1 hostile faction kills, per the server's C# port) |
| bag window | `aa_net::Inventory`: items, counts, icons, grade frames, tooltips, "n/50"; the tabs filter by item group (below) |
| max HP / MP of the player and of the target | the DB's unit formulas (below); the server sends none |
| quest marks over NPCs | `aa_net::Quests` + the DB's quest tables (below) |
| character info window | the numbers (`X2Unit:UnitInfo` / `UnitModifierInfo` / `UnitAttr` of the player) are worked out from the game database, not from `PlayerStats` (the server sends none): `tipcalc/charstats.rs` over level, worn items (`Inventory`) and the buffs (`Buffs`), see "Character stats" below; equipment slots from `Inventory` (see below) |
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

The exp bar and every level derived from experience (abilities, `levelPercent`, `prev` /
`nextLevelTotalExp`) use the server's `levels.total_exp`. `hud/data.rs` reads it once at startup
(`install_exp_table`, `AA_DB` or the server compact) and installs it in `aa_net::model::exp`; only
when the database is unavailable does the hardcoded 1.2 table remain in use. A server on the 3.0
retail curve for 51-55 (5,174,400 / 6,550,320 / 8,751,792 / 12,164,073 / 17,453,110) is followed
without a client change.

### Character stats

`UnitInfo("player")` and `UnitModifierInfo("player")` (the character window, `HUD.player_info()` / `HUD.player_modifiers()` in
`ingame.lua`) come from `TipTables::player_sheet` (`hud/tipcalc/charstats.rs`), recomputed with the attack numbers of the
tooltips (`TipState::refresh`: level, equipment, buffs). It follows the server's `CharStats` (`systems/skills/stats.rs`): each
primary stat is its level formula plus the worn items' stats with the `unit_modifiers` bonuses (attribute ids as in the server's
`UnitAttribute` and `tipcalc::attribute_name`: 0 Str, 1 Dex, 2 Sta, 3 Int, 4 Spi, 6/7 max health/mana, 8 armor, 64 magic
resistance ...), flat bonuses first, then each percent bonus; armor, magic resistance, DPS (`weapon + stat / 5`), rates over the
`Facet` formula, regeneration, critical and avoidance follow from those. The key set and units are the original client's, taken
from the server repo's `oracle stats` captures (`<shots>/oracle-stats-L*.json`): DPS in real DPS, `*_mul` percent
deltas, rates in percent. `UnitModifierInfo` is what the bonuses of the key's own attribute add (the value with them minus
without). When the numbers change the character window's own update handler runs again (`HUD.refresh_character_info`).
`cargo test -p aa-client charstats` checks the sheet against the oracle (levels 1 and 50, gear, shield buff, percent buffs). A level 10 capture
with empty hands is a second case (`unarmed_matches_the_retail_oracle`).

Attack speeds: with a weapon its `holdables.speed` (sword 1.6 s main hand, bow 1.0 s ranged; a shield in the off hand shows
0). Bare hands (retail capture): main hand 1.5, off hand 3.0 (only with both hands empty), ranged 1.5, and min DPS equals
max DPS (no spread). Unarmed ranged min/max is `floor(dps * 1.5)` (8 at 5.6 DPS); unarmed melee min/max is `floor(dps * 3.0)`
(16), which the capture cannot tell apart from `floor(dps * (1.5 + 1.5))`. Still unverified: the off-hand speed with a main-hand
weapon and an empty off hand (shown as 0), and unarmed values at other levels or with stat buffs.

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
button waits. Read-only `<client12-unpacked>/x2game_unpacked.dll`, callback `0x39495c00`, fixes the automatic deadline
at 300,000 ms; both times truncate to seconds. Local death starts that deadline and the packet's shorter wait;
`SCNotifyResurrection` permits in-place revival without restarting either and raises `PLAYER_RESURRECTION` (event 92)
with the caster's hexadecimal live-object id (`FUN_391d39f0`; see "Death window" for the captured run). The HUD tracks
this while loading too. An already-dead login has no fresh death timestamp and retains the native zero-deadline behaviour.

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
* `npcs` rows are read only for `UnitKind::Npc` and `UnitKind::Mate` (`plates::npc_of`). Mate templates are genuine
  NPC rows (the Brown Lilyut Horse is npc 5431, mate_kind 1); housing design ids and slave/transfer/shipyard
  templates share the numeric id space but are separate kinds: 231 of the 269 housing designs collide with an
  `npcs.id` (design 171 "Test Farmhouse" with NPC 171 "Plains Wolf Pup", faction 115 hostile). A house keeps the
  packet's faction and its own name, so design 171 is neutral yellow, as the native client shows, not hostile red.
  The target frame, combat names, speech bubbles and the target sound use the same guard. Evidence:
  `tools/evidence/house_plate/` (native capture, before/after crops, `bevy_report.txt`).
* The native `GetUnitInfoById(...).faction` is "friendly" for the house and for a same-faction NPC, while both
  nameplates are yellow (same_faction.txt). `relation_of` reports "neutral" for every non-character unit of the
  player's own faction, so this string differs from the native client for all such NPCs, not only houses. The
  plate colour is correct; the broader pre-existing string behaviour is deliberately unchanged.

The camera copies `Hdr` and `Msaa` from the world camera and is spawned only after the world camera has
`Hdr` (aa-world's post processing adds it a frame later); a frame where the two disagree is a wgpu
validation error ("attachments have differing sample counts"). `cameras.rs` keeps the UI camera in step.

### Emotes and speech bubbles

Code: `hud/speech.rs`, `ingame/emote.rs`, the `Event::Emote` arm of `ingame/combat.rs`, `hud/api.rs` (`emote_command`),
`ingame/ground_target.rs` (`route`); wire side in `docs/aa-net.md` ("Emotes and speech bubbles").

* **An emotion is an `express_texts` row** (109 rows): `anim_id` is the clip (`anims.name`, e.g. `fist_em_clap`), `me` / `me_target`
  / `other` / `other_target` / `other_me` the chat lines (English from `localized_texts`, with `@SOURCE_NAME(0)` /
  `@TARGET_NAME(0)`). The emote skills of the skill book's emote tab (`default_skills` category 5, 43 of them, English names
  `/sad`, `/clap`, `/flowers` ...) are the same emotes: a skill's clip (`fire_anim_id`, or the pose of its channeling buff for
  `/bow` and `/kneel`) is the `anim_id` of its row, which is how `Emotes` (`ingame/emote.rs`) maps skill and slash command to the
  emotion. `/sit`, `/beg` and `/pose` have no clip of their own in the database and stay ordinary skills (and are not slash
  commands here).
* **Sending.** Typing `/sad` (the toolkit leaves a command it does not know in the text) goes through `HUD.request("chat")`;
  `api::handle_requests` sees the slash command names an emote and sends `Command::ExpressEmotion { emotion_id, target_obj_id }`
  (the selected unit, else 0) instead of saying it. Using an emote skill from the action bar (`SkillPressed`) does the same in
  `ground_target::route`. Items whose skill is an emote stay item uses.
* **Receiving.** The server broadcasts `SCEmotionExpressed` to everything in range, the sender included. `combat::apply` plays the
  clip on the unit (`Stage::play`, upper body when the unit moves, the clip's `effect` anim events start the flowers and tears;
  a looping clip is held 8 s) and `speech::dispatch` raises `CHAT_EMOTION` with the line for the reader ("You clap.", "Emtwo weeps.",
  "Emtwo sprinkles flower petals."), which `chatbubble.lua` prints into the chat window as an emote line.
* **Bubbles are the original script's** (`x2ui/chat/chatbubble.lua`, loaded with the chat scripts). `chat.rs` now tells it the speaker
  is a unit the client has (`speakerInChatBound`, 6th argument of `CHAT_MESSAGE`, true for the player and any unit in `Units`;
  replayed lines get none), and the script draws the bubble of say, party, raid and guild lines (not whispers or shout) over the
  speaker for 5 s. It asks the game for the speaker's screen position with `X2Unit:GetUnitScreenNameTagOffset(id)` every frame and
  hides (and forgets) a bubble that gets no answer on its first frame, so `speech::project` projects the spot over the head
  of every unit the scene has every frame (plate heights, plate unit entities) and the answer is the last result;
  `X2Unit:GetUnitInfoById(id)` gives the speaker's type and relation (the colour of the name). The tail of the bubble sits 62
  px (a character) or 30 px (an NPC) over the plate's bar spot, i.e. over the name.
* **`SCChatBubble`** (the skill effect `BubbleEffect`; nothing else sends it): text (`how` 1) or the lines of a `bubbles` row
  (`bubble_chats` from the START row along `next`, English `speech`, 3.5 s apart, spoken by the NPC of `npc_id` when one is in view)
  are raised as `CHAT_MSG_QUEST(text, name, objId, kind, 5000, ...)`, the event the script uses for bubbles that are not chat. `kind`
  1 normal, 2 think, 3 and 4 system (blue text). `SCAreaChatBubble` is decoded but not drawn (the Rust server never sends it).
* **Limits.** No bubbles in `--headless` without `--offscreen-shot` (no camera, no plates, no position). Doodad bubbles
  (`GetDoodadScreenPosition`) are not drawn. The unit does not turn toward the emote's target. The clips of `/sit`, `/beg`, `/pose`
  and the seated ones are not played (no emotion id).
* **Not checked against the real 1.2 client.** The original binary is not available to read for this (the 1.2 `x2game.dll`
  is packed and the 10.x client names `X2Chat:ExpressEmotion(text)` / `GetEmotionCommand` which 1.2 does not have): whether the 1.2
  client sends `CSExpressEmotion` for emote skill use or only `CSStartSkill`, and which of the two it uses for slash commands, is
  unknown. The server announces both the same way to the player, and `CSExpressEmotion` is what the quest acts
  `ConAcceptNpcEmotion` / `ObjExpressFire` listen to.

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
(`icon_interaction21.dds`), opens the window; both act on the nearest corpse within 3 m. The retail ArcheAge 3.5 client
enables looting at 3.0 m using the distance its target frame shows (a frame reading 2.9 m has loot enabled); the 10.x
decompile has the same 3 m check (squared 9.0) against the target's body, and the AAEmu server's `GetDistanceTo` subtracts
model radii. This client uses that same body metric for both the target frame distance and the loot gate: 2D centre
distance minus both units' `ActorModel.Radius * Unit.scale` (0 without an actor model), clamped at 0, via `distance_to`
(`api.rs`). What is left comes as
`SCLootBagData` and the original's loot window (`x2ui/loot`) shows it: `LOOT_BAG_CHANGED(setTime)` (`setTime` for a "loot all":
the window lists what could not be taken for two seconds), `LOOT_BAG_CLOSE`; `X2Loot` answers from `HUD.loot_count` /
`HUD.loot_item` and takes an item with `HUD.loot_take` (`Command::LootItem`). The Angry Rat of the lane server drops nothing,
so this has only been seen with `--hud-fake-loot` (the icons match `reference/ingame/combat_death_2/real_08.png`; the
window is the script's). The wire formats are checked against the server's own byte tests.

**Tab** targets the nearest living enemy (the next one when pressed again), any unit if there is no enemy.

**Not done / assumptions.** The afterimage, shake and critical bursts; the plate shows no party, guild or PvP marks;
auto-away after 300 s is a guess; the capture's side-on camera is only approximated by
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
Presses during it are not sent (see "Dark slots" below).

A plot step can restart it: the `GlobalCooldown` special effect (type 41) sets `now + value1` on the server in the middle
of a plot, and the server sends nothing for it but the step's `SCPlotEvent`. Triple Slash's own `custom_gcd` is 500 ms, but
its finishing step 12148 sets 2 s, so the server refuses every press until about 2 s after that step. `GameData.plot_gcd`
(`plot_effects` joined with `special_effects`, 381 steps in the 1.2 database) goes to `Skills::set_plot_gcd_catalog` once the
data is loaded (`cooldown::bind_catalog`); an own `PlotEvent` of such a step stamps `Skills.plot_gcd`, and `cooldown::gcd_at`
takes the longer of the two. The caster's `GlobalCooldownMul` is not known to the client: 100 %. Before this, six presses of
Triple Slash 0.3 s apart sent four `CSStartSkill`, three of them refused with `COOLDOWN_TIME`; now they send one.

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
2 m beyond the reach is still sent (the server knows the size of the bodies), the own cooldown is refused with the alert ("Can't be used
right now."; any remaining time of it counts, also one as short as the global cooldown). The log has one line per change of the dark set:
`hud: dark slots [(2, "TOO_FAR_RANGE"), (4, "TOO_FAR_RANGE")]; target Some("Solzreed Fox") at Some(11.7) m`.

Holding a key or clicking fast must not flood the server, so a press is first held back without a word (`Env::held_back`, no
`UseFailed`, nothing sent), by any of:

* the skill is the cast or channel the player is in the middle of (the slot stays lit, but a second press would only be refused);
* the global cooldown of the last own cast has more than 100 ms left (`Env::gcd_left`, the estimate `cooldown::gcd_at` also draws
  on the slots; the last 100 ms are left to the server because the estimate starts when its answer arrives). Skills with
  `ignore_global_cooldown` are never held back;
* the previous press of a skill that waits for the global cooldown is still on its way (`InFlight`): the estimate above only starts with
  the server's `SCSkillStarted` / `SCSkillFired` / plot event, so the presses in between would all go out. `ground_target::route` (and
  the click that casts an aimed skill) marks a sent `SkillPressed { gcd: true }`; the mark ends when `Skills.last_cast` (a skill of the
  player started) or `last_failure` (the server refused one) is newer than it, or after 500 ms. Presses of ignore-gcd skills neither
  set it nor wait for it.

The server holds these back as well (`COOLDOWN_TIME` during the global cooldown); the client just does not ask.

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
`Command::Resurrect { in_place }`, `X2Unit:GetUnitNameById` answers for the player and the target.

The offer is its own event. The original's `OnNotifyResurrection` (`FUN_391d39f0`, `x2game.dll` line 363081) sets `[[g_connection+0x40]+0xafc]`
and raises `PLAYER_RESURRECTION` (event 92) with one argument, the caster's live-object id as a hexadecimal string. The death window
handles it with `ShowResurrectionMode(casterId)`: the title stays "Hereafter Threshold", the penalty line becomes
"[CgCombatb] is reviving you. You'll recover your XP when you revive. Accept revival?" and the buttons become "Accept" /
"Respawn: Temple". `X2Unit:GetUnitNameById` resolves that caster even when it is neither the target nor a party member
(`hud/unit_name`, backed by the received `Units` table); the offer does not restart either death timer. `PLAYER_RESURRECTED`
(event 93, no arguments) hides the window and clears the death timestamp and offer flag.

The fresh capture in `tools/evidence/resurrection/` used a real death on a live lane server and a second character casting
skill 10546 (Resurrection): `PLAYER_RESURRECTION` carried `3ee1` = 16097 = `CgCombatb`, and `GetUnitNameById` answered
`CgCombatb`. `GetResurrectionCountdown` read `(false, 300, 15)` at death, `(true, 290, 5)` when the offer arrived and
`(true, 285, 0)` when the Accept button enabled; the native fields were `+0xaf4=35830`, `+0xaf8=15000`, `+0xafc` 0 -> 1 -> 0.
The Bevy replay (`--headless --mute`) received the same event from the original caster (`64c0`/`CgCombata`), read
`(true, 289, 4)`, saw the Accept button, clicked it through the widget hit test and got `PLAYER_RESURRECTED` and HP 2434.

`--hud-sim dead` still covers the window without a server death. Item (caster kind 2) and mount (kind 3) offers and the
five-minute temple expiry were not reached; their branches are stated as unproven in the evidence report.

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
JSON after a successful write. The graphics page applies on save (see "Options graphics" below); key bindings are
editable (see "Original Options key bindings"). Raid role/move/convert,
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

## Station, gravesite and public farm icons

Source (`hud/map_services.rs`): `npc_station` = NPCs with `npcs.npc_nickname_id` 22 / 23 (Carriage Conductor) or 25
(Airship Conductor), found in `npc_map/<zone>.dat` like the bankers; `common_farm` = nickname 49 (Public Farm / Nursery /
Ranch Manager, NPCs 11545-11547); `npc_graveyard` = the server's `Data/Portal/respawns.json` (281 resurrection points
with x, y; the client DB only names them in `return_points`), copied to `hud/graveyards.tsv`. All three feed the roadmap
(current zone) and the world map atlas. World atlas counts: 97 stations, 727 graveyard entries (zone-group rectangles
overlap), 15 farm managers. Public farm doodads (`common_farms`, `doodad_func_open_farm_infos`) were not mapped.
Live check: carriage stand at Desireen / airship at a port; graveyard at Desireen (Solzreed) and Marianople.

## Not done

* The target frame's job icon for another character reads `Unit::active_abilities` (`GetTargetAbilityTemplates("target")`): the data path
  is unit tested in aa-net, but no other character was in view on the lane server to see it.
* Dark slots: buff restrictions, equipment and `or_unit_reqs` rules are covered by unit tests (`usable.rs`), the lane run only
  met range, mana, casting, target and combat. The unit requirement kinds that need quests, doodads, zones or factions count as met.
* Map icons: live housing icons (no data source), the carried-backpack slave markers
  (`UPDATE_CARRYING_BACKPACK_SLAVE_INFO`: no packet tells the client where other players' trade packs are).
* The white beams of the quest effect over an NPC (a particle effect of the NPC, aa-fx).
* Bag/equipment/action-bar drag requests are now routed through `drag.lua` / `drag.rs`;
  their live visual coverage is still being completed.
* Tooltips: what is still open is listed under "Tooltip numbers" (the damage breakdown option, `ea_*` / `return_point`,
  the buff caster's own stats for debuffs on others, gems / tempers / enchants, repair cost).
* Map conflict zones: the time left is the server's end time minus the wall clock (the epoch of the native clock is unknown); a zone
  the server has not reported yet shows no state (`-1`).
* Quest marks ignore requirements the client cannot check (they hold), so an NPC may show a "!" for a quest
  the server then refuses.

## Map icon pass, live check (2026-10-27)

On the `bevy` server (account `<account>`, Desireen): Respawn Base (`npc_graveyard`) icons appear once checked in "Show Icons"
(off by default, as the original filter has it); "Temperate" comes from the DB; the western continent shows the server's
conflict states (four tension zones and Halcyona at war). The own-ship marker (`map_slaves.rs`) is unit-tested only: no ship
was summoned live.

## Map climate and conflict overlays

Source (`hud/map_overlays.rs`, installed after the fake `fake_worldmap.lua`, which stays for aa-ui's examples; the "Temperate" on
Solzreed was the fake's hard-coded `{ 2 }`, now it is data). `AAUI.MapClimate(zoneGroupId)`: `zones.group_id` ->
`zone_climate_id` -> `zone_climate_elems.climate_id` (sorted union over the group's zones; nil when none).
`AAUI.ConflictZones(worldId)`: open `conflict_zones` rows whose zone group name starts with the continent's letter (`w_` west 3,
`e_` east 4, `o_` origin 5; the map rectangles overlap so they cannot decide); west = 19, 20, 22, 26, 27. `AAUI.ConflictState(zone)`:
the latest `SCConflictZoneState` (`aa_net::state::ConflictZones`, copied in `refresh_snapshot`): state byte = `HPWS_*`
(0..4 trouble, 5 battle, 6 war, 7 peace), time left = end time (unix s) minus now, in ms. Tests: `cargo test -p aa-client --bin aa-client map_overlays`.
Live check: open the map (M) in Solzreed: zone map label "Temperate"; click the western continent: Karkasse, Cinderstone,
Hellswamp, Sanddeep tinted by the state the server last sent (the lane server's `test_zone_state` script can change one), Halcyona peace.

## Quest areas and notifier numbers on the maps

`hud/map_quests.rs` + `map_quests.lua`. The client's own `quest_sign_sphere.g` files (`worlds/main_world/level_design/zone/<key>/client/`,
69 zones, the server reads the same) give `qtype` (quest), `ctype` (step = `quest_components.id`), a place relative to the zone's
origin cell and a radius. `HUD.quest_targets(qtype)` returns the areas of the step the server says the active quest is on
(`Quest::component_id`). The original `quest_context_notifier` does the rest: `X2Decal:CanMakeGuidedDecal` is true when the step
has an area, so the quest gets a decal index (0 main quest, 1-9 others) and calls `X2Map:AddNotifyQuestInfo / UpdateNotifyQuestInfo /
RemoveNotifyQuestInfo`; the Lua keeps that state and adds, to both maps' markers, a circle of the radius tinted with the client's
`notifyAreaColors` (`ui/map/territory_circle.dds`, alpha 0.5) under everything and the index's number cell of `npc_icon.dds`
(`notifyAreaCoords`; the main quest's green diamond) over it, with the quest title as tooltip. The world map shows them on every zone's
map, the roadmap on the player's zone. aa-ui's `Marker` got `radius` (fraction of the map image width) and `color`.
Tests: `cargo test -p aa-client --bin aa-client map_quests` (the real files all lie on their zone's map).

Limits: a step without an area (talk to / kill an NPC, report) gets no decal and no icon (the real client likely points at the
NPC; its source is not known); the ground path of the guided decal (`SetQuestGuidDecalByIndex`) is not drawn; no city (level 3)
maps; the circle's exact look (the native draws it from a texture of its own) and the numbers' size are by eye; the notifier
list is rebuilt per session, so the numbers may differ from the real client's; the sheet cell (480,72) for index 0 is read from
`notifyAreaCoords[1]`. The `bevy` server reports a quest's start component while it is in progress (quest 251: `component_id`
383, its area belongs to step 633), so when the reported step has no area and the quest is not ready/completed, the quest's own
areas stand in. Checked live 2026-10-27: quest 251 "Boars in the Canyon" accepted from Mayor Gott (template 3512, Desireen) shows
the index-1 circle and number west of Desireen on the world map and the roadmap. A GM-added quest (`/quest add`) arrives with
status 0 and the original notifier does not list it, so it gets no area. The script's own `[WARN] Failed : update quest sign`
for quests without a decal (e.g. 330) is the original notifier's diagnostic, not an error here.

## Other zones on the world map (2026-10-27)

The world map turned to another zone than the player's shows that zone's service icons (banks, auction houses,
stables, mailboxes, crafting stations, portals, housing province signs), and so do the city maps inside it.
`hud/markers.rs` builds them once, after the map tables are read, from every zone group's
`worlds/main_world/map_data/{npc_map,doodad_map}/<key>.dat`: 3,332 icons in about 20 ms. They are pushed once per
Lua state as `HUD.map_atlas` (`group`, `u`, `v`, `kind`, `tip`, plus `city`, `cu`, `cv`), already placed as
fractions of the zone and city map rectangles, and `markers.lua` appends them to `AAUI.WorldMap().markers` for
every group but the player's (that one stays in `HUD.map_icons`, with its quest marks overriding service icons).
The "Show Icons" filters apply to them. The roadmap still shows the player's zone only. Units, quests, party and
the gravesite exist only around the player, so other zones show none. Checked live on the `bevy` server:
Two Crowns with the character in Solzreed ([screenshot](screenshots/readme_map.jpg));
`cargo test -p aa-client --bin aa-client markers::` passes (`atlas_covers_other_zones` uses the real data).

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

## Native item links and group loot (2026-10-17)

`itemlinks.rs` maps actual lane items to the native 192-byte record and formatter recovered in
aa-ui. Retained detail payloads preserve opaque fixed tags and equipment reserved bytes. Typed
synthetic fixtures support None/Equip/Mate; absent unknown bytes are never invented. The lane's
33-byte slave detail cannot fit native tag2's 29-byte wire payload and remains unsupported rather
than truncated. The native formatter normalizes only instance ID to0 and count to1; the real
corpse key and offered count remain separately in `LootRolls`.

AAUI.ChatLink resolves real item/quest names and colors outside the toolkit's UI borrow.
Item colors use the real grade tables; quest colors follow native39271e10's task/detail6 and
level-difference bands. `X2Quest:GetQuestLinkText` now returns native `|qID;` markup.
`X2Item:InfoFromLink` accepts canonical full item markup and the bare encoded payload used by
original chat clicks. Payload type/grade are authoritative, as in native394dfae0. Malformed and
obsolete 176-byte debug links fail. The original tooltip uses existing real template/instance
tables; retained rune/dye/gem fields do not imply every tooltip section is complete. Label lookup
is cached when a message arrives; old quest-label colors are not refreshed on a level change yet.
Bag add/use/remove/store chat events now use native clickable links when actual details survive;
missing instances retain plain colored labels.

`loot_roll.rs` binds X2Loot.DoDiceAction, RequestNextDiceItem and the exact
SetMaxConcurrentNotifyDiceItem setter. Original `loot/loot_dice.lua` owns the three window
positions, Roll/Pass/close handlers, fade, 60-second pass deadline and 500ms next-item delay.
Opaque local u32 UI keys resolve to exact packed corpse keys without a Lua64-bit conversion.
Duplicate queued choices are suppressed. Packet-send notifications and server responses remain
distinct; no request grants an item, invents a winner or changes balances. Signed notifications
raise original LOOT_DICE chat events. Summary packets do not become a synthetic winner event.
An unsupported detail reports its limit instead of building a false item record.

The shared game API now supplies X2Time.GetUiMsec from the toolkit's monotonic frame clock.
The original native394b37f0 reads UI timer selector 1, returning milliseconds. Our stage-local
epoch is not claimed to match the original epoch or f32 quantization. Prompt receipt timestamps
are translated to that epoch while preserving age, so original delays and deadlines agree.

Validation: 117 client tests pass, 16 probes are ignored. Binary adapter tests preserve real
fields, opaque payloads, reserved bytes and native normalization; controller tests check exact
key/choice requests, duplicate refusal, concurrency and nonoptimistic state. The isolated original
stage exercises real item/quest lookups, native Roll/Pass/deadline, next queued item and title-bar
close. Mail read/send/recovery also passes with these bindings. `loot_dice_hud` is an aa-dev
example with explicit synthetic instances of original debug templates, not live loot settlement.
Its inspected `shots/hud/sol61-loot/native.png` shows all three original windows and measured
185–213FPS at1920×1080, with zero Lua errors. The main client world-select smoke has zero script
errors and231–277FPS; its inspected capture is in the same folder. All root QA windows exited.
Natural paired live loot settlement remains pending while the lane game connection is stalled.

The root playable copy is refreshed at `target/debug/aa-client-play.exe` (SHA256
`887F09EBADA1D9848DA7B4ED262C93730988B09FD445C005B02C184350B4D8D8`).

## Original player trade and chat transport (2026-10-17)

`trade.rs` binds the original target-menu request, incoming invitation response, ten-slot carts,
money editors, lock toggle, confirmation and cancellation to `aa_net::Trade`. Own cart indices
are one-based physical bag slots; the native removal API takes zero-based physical indices.
Remote item indices are zero-based and the original Lua compacts them after removal. Real
item definitions and retained instance fields supply the original tooltips. An unavailable
definition refuses confirmation instead of displaying a fabricated offer.

Original `popup_menu_proc.alb` calls RequestTrade with no arguments; it resolves the actual
target. Incoming TRADE_CAN_START runs the original exclusive-window gate and accept/decline
handlers. X2.GetTargetFactionRelationship now follows native39442960/394bb977: hostile1,
neutral2, friendly3, unknown0 and missing target2. The original target menu excludes hostile1;
the request adapter likewise permits neutral characters and rejects self/hostile/noncharacters.
Native LockTrade registration394b4cd5 reaches394fd5b0, which sends the inverse of
the acknowledged byte at3a5ba538 without changing it locally. The binding consequently toggles
the acknowledged flag. It permits the native unlock operation after a confirmation, while
offer edits retain the lane's explicit refusal once either participant has confirmed.

Bag right-click prioritizes a visible original trade cart. Real drag payloads now carry the
native cursor category separately from their internal kind: stackable definitions use Item,
unique definitions use Item_Id, and skills use Spell. The criterion is maximum stack size,
including a stackable item whose current count is1. The original trade background accepts
Item only; its empty item-slot click accepts both Item and Item_Id. No original handler is
replaced to bypass those rules. Exact item IDs remain decimal strings above2^53.

Whole-stack preflight checks physical inventory, bound flags, cart capacity including outstanding
offers, held identity and count. Dispatch rechecks identity/count against current inventory.
Confirmation requires both acknowledged locks and unchanged complete own-item snapshots.
Coin parsing checks ranges, overflow and the current PlayerStats balance; edits coalesce to the
last amount in the frame, including returning to zero while a previous amount is outstanding.
Queued sends do not create cart entries, lock resources, grant settlement items or change money.
SCTradeMade's item tasks are applied by aa-net once, before the UI event. Closing after Made or
Canceled does not send a redundant cancel from the original OnHide handler.

Coalesced network events replay their individual snapshots before firing Lua events, so a lock
query sees the corresponding transition rather than the frame's final flags. Local rejection
and an explicit60-second app timeout report failure and restore the original button's
presentation from acknowledged flags. A timeout does not retry or clear authoritative state.
The original controls and bag locks follow actual offer/remove/cancel/settlement notifications.

Known limits: native RequestTrade also checks configured player/target level thresholds,
runtime distance and several action/status flags. Their exact values/meaning remain unresolved;
they are not replaced with guessed constants. The lane does not implement all those eligibility
checks. The lane supports whole stacks, and its explicit unlock retains confirmation flags;
changing a confirmed offer requires cancellation. Settlement rollback, partial stacks and
generic failures retain the network/server limitations recorded in aa-net docs. Original UI
transactions still need paired live validation after the lane game connection recovers.

The chat request adapter now calls `aa_ui::lua::chat::encode_for_transport` exactly once after
native input-link substitution. Native395115b8 escapes bars except item/quest prefixes, doubles
literal @, and393c5be2 collapses CRLF before replacing remaining CR/LF with spaces. Incoming
CHAT_MESSAGE text is forwarded unchanged; the toolkit's native message insertion handles the
single escaped-@ decode and glyph parsing consumes escaped bars. Runtime character limits and
the native1024-byte conversion overflow policy remain unresolved, rather than guessed.

Validation:124 client tests passed,17 probes ignored. Six trade regressions cover whole stacks,
64-bit identities, ten outstanding offers, changed counts, native lock toggling, current balance,
neutral/self/hostile target gates and per-event coalesced lock queries. The root chat adapter
test verifies native escaping/newline conversion occurs once on the actual Request-to-Command
path. The isolated original-stage trade test passed in4.79s: original target popup/button,
stack background drag, unique-item empty-slot click, right-click removal, remote index compaction,
money change events, ACK-driven locking/confirmation, settlement close and cancellation.
It also verifies no optimistic cart entry, inventory mutation or duplicate close-time cancel.

`trade_hud` is an aa-dev fixture using actual item definitions and explicit synthetic instances.
The inspected `shots/hud/sol61-trade/native.png` measured289-327FPS at1920x1080, with zero Lua
errors. It preserves the original authored anchor, which places30px of the430px outer window
left of the screen; native window-bound clamping/reference calibration remains an open layout
check. Its original cart controls and the bag are visible. The inspected world-select capture
in the same folder measured271-307FPS with zero script errors. These checks are not paired
live trade settlement; the game lane timeout still prevents that follow-up. All root QA
processes exited. The playable copy at `target/debug/aa-client-play.exe` now has SHA256
`37992B80A74EE8FD3956D40B8857F333135416825FF9B2459C01548FF9825676`.

## Original Options key bindings and gameplay input (2026-10-17)

`settings/bindings.rs` reads `game/default_binding.g` through aa-pak instead of maintaining a
second key table. It retains both managers, all five mode groups and original one-based indexed
actions. The native104-action catalogue comes from393f8d20/39ac8e68 and remains case-sensitive.
Original defaults include Q/E strafe, A/D turn, arrow-key secondary movement, Mouse4 secondary
autorun, Ctrl-E mail, Shift-V friends, I inventory and both action-bar rows. The former B alias
is removed. The native18-entry overridable-action table is runtime-encoded; the implementation
uses the non-normal groups in the authored defaults as its documented inference.

Live and draft keymaps are separate. Original X2Hotkey getters, BindingToOption, InitOptionHotKey,
indexed/button setters, removal, SaveHotKey and EnableHotkey now operate on them. A changed
assignment displaces the same physical chord across managers within its mode, then raises native
UPDATE_OPTION_BINDINGS(overridden, displacedBaseAction); unchanged assignments raise no event.
Save commits the draft and raises UPDATE_BINDINGS, refreshing the original action-bar labels.
Original Apply, Cancel and Default Keys handlers are used without replacement. Explicit empty
records preserve removed defaults across restart. Settings use the existing bounded, atomic
account-local JSON store; pending general-option edits remain separate from a hotkey save.

Native393fc270 tries exact modifiers and then removes Alt only within each active mode before
normal mode; Ctrl/Shift do not fall back. Native393ff880/393ff9a0 latch the action on physical
press and release it by physical key, even if modifiers change. `settings/input.rs` follows that
contract and tracks ordered key events. Captured/disabled input clears active actions, held-key
repeats cannot reactivate them, focus loss clears state, and entering a world does not synthesize
presses from keys already held. Native key names come from aa-ui's recovered keyboard mapping.
Mouse3/4/5 and wheel pulses use the same bindings. Wheel pixel scaling remains the existing app
camera calibration, not a recovered native constant.

Movement, jump/dive, walk/autorun, camera zoom, action-bar slots1-58, chat opening, interaction
keys, quest-directing choices, target cycling, mail and registered window/button actions consume
the same live map. Original visible enabled button globals can receive a bound left-click.
Action-bar requests retain their real usability/range/mana gates. Target cycling retains the
app's existing range/order approximation. Literal Escape still closes windows/cursors/targets;
the configured open_config action opens the original system menu. Existing Shift-to-walk and
two-mouse-button movement conveniences remain. Dev scripted controls remain available.

Known limits: normal and quest-directing modes are active in gameplay; builder, screenshot and
context-sensitive mode activation still need their owning systems. Direct native spell/macro/item
binding kinds, less common action consumers, camera preset actions and native key-name rejection
need further audit. This local JSON is not the native hotkey.g/character UIData blob. aa-net now
preserves UIData type2, but native xlcommon::Data grammar/version and lifecycle synchronization
are unresolved; no guessed JSON is uploaded as native binding data.

Validation:129 client tests pass,18 probes ignored. Six settings/input tests pass, including physical-release behavior, mode
priority, primary/secondary keys, indexed/button assignments, conflict events, explicit removal,
Reset/Cancel and restart. The isolated original world-stage fixture passed in5.45s: the actual
Character Controls editor captures P, original Apply saves it, Cancel rejects O, Default Keys
resets only the draft, and injected physical P subsequently activates moveforward while W does
not. The fixture uses an isolated temporary store and leaves the user's account settings intact.
`keybindings_hud` is an aa-dev example showing the original Game Keybinds page for comparison
with `reference/ingame/options_keybinds/real.png`.

The inspected `shots/hud/sol61-bindings/native.png` preserves the reference Game Keybinds
window/art/rows and gold/blue key fields, with443-445FPS before capture at1920x1080 and zero
script errors. Generic body/menu text still uses a visibly different serif face from the
reference; shared font alias/fallback calibration remains open. The inspected world-select
smoke in the same folder shows the live Lane-bevy list, zero script errors and292-405FPS.
Both root QA windows exited. The playable copy `target/debug/aa-client-play.exe` is refreshed,
SHA256 `A9523DFD34F89BC4F772104D8D78AE2295555A620F6D9A5EAA1352CC26B123C4`.

## Options graphics and mouse-button bindings (2026-10-22)

`graphics.rs` applies the original Options window's screen and quality pages (`x2ui/option/screen_option` and
`screen_advanced_option`, unchanged) to the window and the renderer. The page's Apply is the apply: `X2Option:Save` counts a save
(`settings::saves`), and the systems read the saved options again when the count moves and once at startup. Nothing is applied
while a slider is dragged, and an unsaved edit never counts (`settings::option_number` reads pending edits first, `graphics` asks
for saved ones through the same store after the save). The mapping uses the engine's own cvar groups
(`game/config64/cvargroups/option_*.cfg`, levels 1 to 4, `[default]` 4):

| Option (id) | Applied as |
|---|---|
| screen mode (0), resolution (1, 2) | primary window: exclusive fullscreen on the monitor's video mode of the chosen size (fastest refresh), borderless when it has none; windowed at the chosen size (a `WindowResized` message is sent with it, without it the passes of that frame disagree about the target size and wgpu rejects the frame). The page's list is the monitor's video modes from 1024x720 up plus the window's size (`NET.resolutions`) |
| vertical sync (4) | present mode `AutoVsync` / `AutoNoVsync` |
| gamma (3) | exposure compensation of the world camera's post pass, +-1 stop over the slider (the engine's gamma curve is not in the post shader) |
| draw distance (10) | `WorldConfig::brush_distance`: `e_max_view_dst` 600 / 1000 / 1500 / 2000 |
| terrain detail distance group (11) | `WorldConfig::terrain_detail_distance`: the group's XY values 256 / 512 / 1024 / 1024 m drive the existing shared-axis detail fade; default 1024. Native Z projection distances remain unsupported. |
| object distance (14), grass distance (15) | `view_dist_scale` (`e_view_dist_ratio` 30 / 30 / 40 / 60 over the 40 of the placement files), `vegetation_distance` / `tree_distance` (the default caps scaled by `e_view_dist_ratio_vegetation` 30 / 40 / 50 / 60) |
| shadows (17), shadow distance (18) | `WorldConfig::shadows` / `shadow_distance` (100 / 150 / 200 / 300 m) and the sun's shadow switch and cascades (`apply_sun`) |
| clouds (23) | `CloudSettings::enabled` |
| effect quality (24) | `FxSettings::quality`, one step below the level (Very High is the High the effects were fitted at) |
| HDR (27) | the post pass's bloom |
| anti-aliasing (29) | the world camera's SMAA: not used / PostAA 1x (Medium) / PostAA 2x (High). `EnumAAFormats` returns no MSAA or TXAA entries |
| camera mode (65, in game) | `cl_fov` of the world camera: 1 action 60 degrees, 2 classic 42.75 (the default, what the camera was fitted to) |

Saved and shown but without a renderer knob: texture quality (8, 9), terrain level of detail (13),
character and animation level of detail (16, 33), shadow level of detail (19, 20), shader quality (21), volumetric effects (22),
water quality and reflection (25, 26), depth of field (28), render thread (5), DirectX (6) and the master quality (7).
`WorldConfig`'s distances are baked per cell when it streams in (`CellObjectData::new`), so a changed draw distance reaches the
cells loaded afterwards (and everything after a restart or leaving and entering the world); shadows, clouds, effects, gamma,
bloom, anti-aliasing, terrain detail fade and the window apply at once. Terrain detail changes preserve existing patch/material handles and also initialize late worker admissions with the current distance. The native option 11 applies the whole `option_terrain_detail` group despite its `*_Z` Lua name; the separate Z values 64 / 128 / 128 / 256 m remain open. The prewarm at the character list holds its own streaming settings: an
apply waits until it is done.

Nothing saved leaves the window as it started (`--size`, `AA_PRESENT`, `--headless` are never overridden at startup) and the
defaults of `settings::native_default` equal what the world was calibrated at, so a fresh account changes nothing (a test asserts
the default options leave `WorldConfig` and `FxSettings` as built). The page showed the lowest step of every slider and unchecked
boxes for options with no default, and saved them: the options stamp is now `AoAOptionDefaults2` and drops the graphics options
(gamma, ids 7 to 33, `MasterGrahicQuality`) of an older file once. The master quality slider's default is step 5 (Very High), the
step the original script computes for system spec level 4.

Key bindings: the binding editor (`Character Controls`, `Game Keybinds`, ...) already captured keys, modifiers and the wheel; the
middle and the two side mouse buttons now bind too (`aa_ui::lua::keyboard::button_event`: `mouse3`, `mouse4`, `mouse5` as
`OnKeyDown` / `OnKeyUp` of the capture owner; the left and right clicks stay the page's cancel). Gameplay already resolved those
names (`settings/input.rs`).

Checks (debug build, 1920x1080, offscreen unless noted, about 26 to 35 FPS in the log, Lane-bevy server):

* `shots/opt/q_default.png` the quality page with the defaults (Very High); `q_applied.png` after clicking the master slider's Low
  and Apply (view distance 1, shadows off, HDR off, anti-aliasing off saved and applied, the scene loses its bloom)
  `before.png` / `after.png` gamma 0.7, no bloom, no shadows; `reset.png` after Reset (`shots/` is git-ignored).
* A real window: vsync and 1280x720 applied while playing (`win_after.png` is 1280x720, the HUD re-laid out), the same size after a
  restart from the saved file.
* Bindings with real window messages: `toggle_bag` rebound to P: I no longer opens the bag, P does, also after a restart;
  `Game Keybinds` page: F row = K, G row = K displaces F (its row empties), SHIFT-M on the fourth row, Apply
  (`kb_shift.png`); middle button = `mouse3`, Back = `mouse4`, Ctrl + Forward = `CTRL-mouse5` (`kb_mouse.png`).
* Tests: `cargo test -p aa-client --bin aa-client -- graphics:: settings::` (17 passed, 1 ignored fixture) and `cargo test -p aa-ui
  --test keyboard` (5 passed).

## Native force-attack state and environmental combat reports (2026-10-17)

`pvp.rs` replaces the fallback UnitIsForceAttack query and consumes the configured
toggle_force_attack action. Native394bd710 returns boolean false for missing/unresolved units
and non-characters; characters read the received force byte. Initial state comes from native
SCUnitState bit3 (397c992b ->398a222a ->unit+3bbc), with later SC043 acknowledgments replacing it.
The lane currently writes zero for that character spawn bit even if its runtime mode was on;
this server snapshot limitation is not hidden by persisting a local sticky flag.

During loading the view hydrates from actual units and CombatStatus before original
ENTERED_WORLD/UpdatePVPIcon handlers run. Once active it replays known-object announcements,
removals and acknowledgments in order. Native391d3c60 emits FORCE_ATTACK_CHANGED(stringId,on),
including duplicate explicit reports; each Lua handler can query the corresponding state even
when opposite changes arrived in one frame. Original player/target/party handlers own the
icons, and the original center-message handler owns the Bloodlust on/off message. Announcements
and local send notifications do not synthesize that event. Removal, identity reuse, logout and
character changes cannot retain a stale flag.

The toggle follows native39390ec0: invert the currently acknowledged own-character byte and
send Command::SetForceAttack. A request does not change the icon, buff, combat relationship or
HP. Repeated presses before an acknowledgment consequently send the same inverse value.
Local transport/phase rejection raises a system notice. Full protected-zone/team/retribution
eligibility and force-mode skill/target preflight remain a separate source audit; the existing
friendly-target skill gate has not been replaced with an unconditional friendly-fire bypass.
Invisibility and breath reports also do not become guessed visibility or breathing models.

`combat.rs` now translates SCEnvDamage through the exact native39391720/3940a3e0 path.
Only nonzero sources0/2 produce COMBAT_MSG with12 arguments: target ID, ENVIRONMENTAL_DAMAGE,
target name twice, FALLING/DROWNING, amount, HEALTH, HIT,0,idle,0,false. Missing names use literal
unknown. Native3940a9dd interprets the raw dword as signed i32 and converts it to f32; presentation
preserves that conversion/rounding, while aa-net keeps the original u32 report. The original
combat-text script creates its local damage text from this message. No extra COMBAT_TEXT,
health subtraction, combat engagement or death is invented. Gimmick/ship/unknown sources retain
raw metadata without an unsupported presentation event.

The current full client suite passes131 tests with19 intentional probes ignored. Focused
tests cover missing/non-character getter behavior, per-event acknowledgment queries, duplicate
reports, removal/reuse and exact environmental arguments, filtering and numeric conversion.
The original-stage icon/environment/physical-toggle fixture passed in2.01s: initial own state,
acknowledged player/target icons, original falling/drowning text, and repeated physical Ctrl-F
sends before/after an acknowledgment. Original Options, mail and trade compatibility probes
also passed after the toolkit listbox/layer changes (3.97s,3.35s,3.34s). These fixtures do not
replace natural live combat checks while the lane game connection is stalled.

Native394c0d80 also distinguishes unresolved combat relationship (numeric0) from missing unit
(nil), while known base relations return hostile/neutral/friendly. The real-data Lua getter
now preserves those shapes. The relation string is not an attack-permission bit: native39838880
tracks permission separately and checks ownership, team and protected low-level zones before
the force override. Those complete permission rules remain an explicit preflight gap.

The inspected `shots/hud/sol61-pvp/native.png` shows both original parent frames and their red
Bloodlust icons, using explicit fixture reports. It measured452-461FPS at1920x1080 with zero
script errors. The fixture's character target uses the original broad TARGET_CHANGED category
npc, as target.lua requires; it asserts both parent frames are shown. The inspected live
world-select smoke in the same folder has zero script errors and300-370FPS. All root QA
windows exited. The playable copy `target/debug/aa-client-play.exe` is refreshed, SHA256
`42E330CBCA7C38C059D5B011A055AD394864945BB5B07CD90D41609536F8076C`.

## Received cooldowns and the original breath bar (2026-10-17)

Action-bar snapshots now preserve the winning `(remaining,total)` from aa-net's received
skill/tag timers instead of replacing a received duration with the database's duration. An
explicit zero total remains zero. The longer existing cast/DB estimate or global cooldown
still wins where appropriate; the existing quarter-second end-time quantization remains.
Active received own timers also refuse a use request when the database duration is zero or
no longer than the GCD. The original cooldown sweep conveys this state; target, mana,
casting and other restriction gates still supply their separate dark-slot reasons.
The lane currently has only an empty SCCooldowns writer and no runtime emitter, so this
does not establish live cooldown recovery on re-entry.

`breath.rs` binds `X2Player:GetBreathTime()` to a local controller and processes received
events even during Loading. Every SCUnderWater report emits the original no-argument
DIVE_START/DIVE_END while the world HUD is active, including repeats. Start/end mutate
the timer only when the local actor exists. SCSetBreath independently rebases the timer
without emitting DIVE or inferring underwater status. Original player_breath_bar.lua
handles visibility, LEFT_LOADING restoration, fill and its authored 200ms hide fade.
No damage, HP change, movement packet or client-side drowning is invented.

Native proofs are recorded in docs/aa-net.md: handler391dde80, controller3930b600,
correction3930b050 and getter3930b660; Lua registration39499367/wrapper39495d10 returns
TOTAL then LEFT. Character attribute91 resolves to the source-backed60000ms base
(398b4e80/398a0d60/table398a11a4/398a10e5). The existing cached gear/buff modifier
calculation now includes attribute91, so capacity is not fixed at that base. Inactive or
missing-actor queries return0,0 before touching cached capacity. Capacity changes preserve
the elapsed fraction; correction and remaining calculations retain native u32 wrapping.
Native3905aca0 converts each unsigned result to f32 before Lua's number representation.

Explicit limits: the original clock's epoch is unresolved, so the app uses a local
millisecond origin with the same modular differences. Native fraction rescaling uses
x87 registers with unrecovered ambient precision; the app uses f64 for that finite
calculation. An old capacity of zero explicitly restarts elapsed time rather than
claiming native floating-point exception behavior. Existing modifier math and Rust's
bounded u32 capacity conversion do not reproduce the unrecovered final native attribute
limit helper. Natural live underwater/drowning validation remains pending while the lane
game connection is stalled. Raw network counters and reports remain unchanged.

Validation: full client suite135 passed/20 intentional probes ignored in0.98s. Focused
checks cover received duration/zero-total/reset and click refusal, native breath correction
order, repeated starts/end, capacity fractions, missing actors, wraparound and attribute91
bonuses/removal. The isolated original-stage breath test passed in2.54s, including real
bar range/fill, zero-argument event handlers, repeated reports, loading restoration and
hide fade. `shots/hud/sol61-breath/native.png` was inspected at1920x1080: original Breath
Remaining bar visibly counts down from an explicit fixture correction;223–238FPS, zero
Lua errors. This fixture uses no connection or3D world. Both client and breath examples
built successfully. The refreshed playable copy SHA256 is
`D1FD79D8523DFBD9CD8CB83CB180E7C4D7BC0904C0970145EFBDE77E824E615E`.
The inspected world-select smoke in the same folder shows the live Lane-bevy list,
307–409FPS and zero script errors. Both root QA windows exited normally.

Follow-up source review found and corrected a same-frame capacity ordering bug: net resources
already contain the final batch, while the old breath tracker read the previous frame's cached
attributes. The tracker now replays the existing Inventory/Buffs/PlayerStats reducers in event
order and refreshes attribute inputs before each relevant breath report. Thus a buff applied
before DIVE_START supplies the correct initial capacity, and a removal later in that batch
does not retroactively change an earlier correction. During initial DB loading, breath notices
retain their receipt clocks, actor identity and exact gear/buff/ability inputs for replay when
the data becomes available (an explicit128-report application budget). Normal movement events
do not copy equipment or recompute attributes. The real-DB regression passed in2.04s for both
ready-data and delayed-data paths, with a real attribute91 buff, half-capacity correction and
same-batch buff removal. This fixes source review findings; the earlier playable hash above
predates this follow-up and will be refreshed with the next verified build.

## Original bag and bank expansion (2026-10-17)

The original bag expansion button and bank API now open the authored inventory task dialog,
including the real Expansion Scroll icon/tooltip and cost layout. Quotes come from the read-only
`bag_expands` rows: bag maximum150, bank maximum130 in this database. Confirmation is bound to
the current character and latest dialog; the sender rechecks received capacity, money and bag
certificates before queuing aa-net's eight-byte expansion request. Cancel/close sends nothing.
Repeated or stale callbacks cannot consume a new quote, including after character changes.
No item, capacity or balance changes locally: received InventoryExpanded updates capacity and
raises original BAG_EXPANDED/BANK_EXPANDED; separate item/money reports retain their own reducers.

The auto-AA checkbox now uses the received setting, as described in the later AA milestone. Premium purchase and
persistence are not implemented. The navigation-context producer remains unresolved, so the
explicit unset tail is zero; it is not a guessed NPC ID. The lane currently ignores that tail,
AA and currency ID, and does not send MoneyChange for positive-price expansion. All18 current
database tiers cost one scroll and zero money. Natural paid/live expansion remains unverified
while game connections are stalled; fixtures do not grant items to a live character.

Validation: full client suite138 passed/21 intentional probes ignored. The isolated original
stage probe passed in2.95s using actual mouse hit testing on the bag and dialog buttons. It checks
cancel, one bag request, received50-to60 capacity, refusal of the old quote, a bank request and
bank-dialog close, while certificates/balance remain received-only. Original Options/mail/trade
compatibility passed in2.43/1.72/1.72s after the latest toolkit geometry changes. The inspected
`shots/hud/sol61-expansion/native-complete.png` uses explicit synthetic inventory and no connection;
its original count label is refreshed through the same BAG_UPDATE as the production drag system.
It measured430-483FPS at1920x1080 with zero script errors and exited normally.
The earlier `native-verified.png` exposed that missing fixture event and is superseded.

## Native inventory view sorting and higher-capacity cells (2026-10-17)

Bag/bank ManualSort now changes a local virtual-to-physical permutation. Its default-tab keys
are category_order ascending, grade_order descending, localized name by unsigned bytes to NUL,
then physical slot ascending; empty slots follow items. Category order is loaded from the real
item_categories rows. Native391f2d50 default Sort is a literal return: ordinary Sort only emits
the tab event and does not reset or comparator-sort the default view. Received inventory changes
sort only the physical numbers in positions currently empty, retaining occupied positions.
This follows391f4480, not an invented automatic-compaction flag. Filter tabs rebuild their
item-ID list in unsigned-u64 order (39202350), and their ManualSort is a no-op (391f2a30).
IDs stay decimal strings across Lua, including values above2^53. Native390105e0 confirms byte
comparison rather than case folding or locale collation; the bridge compares bytes explicitly.

Dragging, use, selling, links and tooltips still read received physical slots. An empty All-tab
cell now resolves through the permutation too, including bag-to-bank and bank-to-bag drops.
No sort packet, item movement, stack merge, money change or repair is fabricated. The separate
bag/bank tab event carries its current one-based index. Filtered rows use the same physical-slot
contract and choose a free physical slot only for an empty filtered destination.

Orders save independently by account-file/character/container without applying pending options.
On a new manager, every currently occupied physical slot/UID must match its saved pair before
the saved permutation is accepted; otherwise the initial identity view is used. Ordinary item
updates never run that reload validation. Native394dd090 uses fixed0xad8 LevelDB records and
391f3a90's130-slot record; this app uses its existing account-local JSON, with an explicit256-slot
input budget and extension for received capacity above130. It does not claim binary compatibility
with the original LevelDB keys or character synchronization of that file.

The database permits150 bag slots, while shipped SetViewOfItemSlots creates13x10 buttons and
its Update/UpdateAt iterate received capacity. The bridge now extends the existing bag pool
before publishing higher capacity. Actual original-stage probes proved the private factory is
the named CreateInvenCoolDownActionButton upvalue of SetViewOfItemSlots, and the existing first
button's Update carries its adapter/injector upvalues. Extra cells reuse that original factory,
skin, click/drag/cooldown handlers and row spacing. Allocation occurs at HUD entry or capacity
change, not every frame. The original expansion probe passed with a received150-slot report,
all150 buttons present and the final cell visible; no live character was expanded by the fixture.

Validation so far: full client140 passed/22 intentional probes ignored. The isolated original
mouse test passed in2.62s: sorting moves displayed bag slot5 and bank slot8 to each first cell,
then real clicks queue the correct physical SwapItems for bag/bank internal and both cross-
container empty destinations. Received UIDs/items remain unchanged. The expansion150-slot
probe passed in2.89s. Unit checks cover exact keys, default-vs-filter sort semantics, empty-only
updates, >2^53 identities, stale/matching reloads, separate owners/containers and pending options.

Final compatibility caught and fixed an ownerless-HUD fallback: the sorting destination hook
must defer to the existing physical/filtered bag behavior until character identity is known.
The original trade fixture covers that path; its first offer failed before the guard was added.
After the fix the full suite passed140/22 in1.28s, and isolated sorting/expansion/Options/mail/
trade/breath tests passed in3.35/2.10/2.45/2.03/1.87/1.82s. Each exact-name run asserted that
one test actually ran. The inspected sorted bag+bank capture `shots/hud/sol61-sorting/native.png`
measured324-423FPS. `native-150.png` shows2/150, the disabled maximum-expansion button and a
real item at physical147 in the added final row;265-301FPS. Both use explicit synthetic inventory,
have zero script errors, and exited normally. The client and sorting examples built successfully.

Playable refresh after c73d453: `target/debug/aa-client-play.exe` SHA256
`1F9789094736416D34A5D866BB7F6132A871E4931FC755EABE084FA4EF5F4AD0`.
The inspected `world-select-final.png` in this folder shows the live Lane-bevy list with zero
script errors and305-390FPS. The same copy's offline Ferre-female original creation screen
(`create-ferre-final.png`, step2) has zero script errors and149-185FPS before capture/140FPS
during capture. No character creation request was submitted. The rebuilt client-helper facial
fixture (`client-paths-final.png`) again passed body-clock/FSQ/GPU-blink assertions;201-315FPS
before capture/101FPS in its capture interval. These shared-machine captures exited normally;
none establishes natural game-session repair/expansion while the existing lane stall persists.

## Received auto-use AA Points (2026-10-17)

`X2Bag:GetAutoUseAAPoint` now reads the character's received value. The setter queues aa-net's
native CS0133 one-byte request only when the value differs and no send is queued or pending;
it never changes the getter or emits CHANGED_AUTO_USE_AAPOINT on a send. Failed/inactive
transport releases the local queue latch through the setting-specific rejection event.
Expansion confirmation forwards this received value instead of an invented session preference.

Native394e758c/394e75af emits each action18 change after writing its raw byte; raw1 alone means
true, including when the same value repeats. The inline3920fcf0 listener path means a coalesced
true/false pair must expose each action's new getter during its own callback. A temporary getter
overlay provides that behavior while the resource contains the final batch snapshot. Native
pending key0xf000 remains locked through the callbacks and is released after the batch
(394f021b/397d22b0), or by the separate received SC00f5 unlock with kindf0/slot0. Unrelated
unlocks, requests and rejection notifications emit no changed-setting event. Initial HUD
hydration uses the received snapshot and does not replay loading history.

The full client suite passed142 tests with22 ignored. Six exact-name original-script probes
(expansion, sorting, Options, mail, trade and breath) each ran and passed in an isolated process;
logs are in `shots/hud/sol61-auto-aa/`. The expansion probe verifies duplicate setters send once,
the getter stays unchanged until a received item task, and both bag and bank confirmations
forward the confirmed true value. The focused callback test covers repeated/coalesced values,
raw2, pending-before-callback release, unrelated/matching unlocks and rejection cleanup.
The current lane does not register the setting packet, so live persistence remains unverified;
these tests supply explicit received fixtures and do not fabricate a server acknowledgement.

## Original equipment repair controls (2026-10-17)

The authored blacksmith window now uses received bag/equipment details. X2Item repair-mode
entry/leave/getter, RepairAll/RepairAllCost, both physical ItemRepair APIs, ItemDurability and
the original DameagedItems spelling are bound. Enter/leave is idempotent. Escape cancels
repair mode; NPC cancellation schedules mode cleanup after the event listeners. Merely hiding
the window preserves mode, as the original Lua does. The mode setter uses aa-ui's separate
native repair cursor hook and never changes the held item.

Native391ff2f0 maps a bag cell to its physical slot before391f2a70 consumes a repair-mode click,
including rejected/empty/healthy targets. The bridge follows that boundary after original
Lua PreClickSlot. Bag HandleUse has a separate native path and stays unchanged; the original
equipped-item Lua itself intercepts either mouse button. Regular repair slots are one-based
Lua to zero-based wire; the native equipment-slot0 last-location sentinel remains unsupported.

RepairAll quotes exactly28 equipped slots, excludes bag items and sends nothing for an empty
candidate set. Confirmation captures character, NPC context, UIDs, physical locations, grade,
wear and costs, then compares the complete target set against fresh Inventory before sending.
Gold insufficiency rejects locally. No durability, balance, inventory or successful-repair event
is invented. The current bridge supports the authored gold window; native premium currency
selection and charge/use-AA task dialogs remain separate work.

ItemDurability returns nil for absent/unsupported/zero-max items, otherwise {current,max},
with actual unsigned-byte wear. Its optional repairability filter defaults true; false includes
unrepairable durable definitions. DameagedItems scans equipment, includes ratio<=0.25f32,
stable-sorts by ratio, then truncates to the required signed numeric count. It returns parallel
template/raw-grade/physical-slot arrays; missing numeric count returns nil. Healthy or
unrepairable item tooltips omit repairCost. Metadata queries avoid allocating complete item stats.

Displayed costs reproduce native397cea70's separately rounded f32 multiplications, verified
against ten original-x86 oracle cases. Eight intentionally differ by one copper from the lane's
existing operation order; aa-net's lane helper is unchanged. Actual charged money is always
received. The numeric RepairAllCost follows the original integer-to-f32-to-Lua conversion.

UPDATE_DURABILITY_STATUS replays the original UID tracker, rather than comparing final frames.
Initialization scans equipment and bag without an event. Only action5/6 insert,7 remove and9
detail update affect it; a subsequent bank3 create/take can also enter the map. A detail update
uses the stored maximum. Duplicate low insertion reports added again, missing UIDs do nothing,
and high/low crossings accumulate both flags through the batch. Grade/move/owner/crafting/
seize tasks and the trailing force-remove list do not synthesize edges. Emission follows the
per-action auto-AA callbacks. Native audit addresses and cursor-resource contracts are in aa-ui.md.

Validation: client146 passed/23 ignored in5.96s. The exact isolated original repair probe passed
in2.54s: real Single Repair/Repair All buttons, real bag and equipment cells, physical request
slots, no optimistic wear/money, mode/escape/hide/cancel behavior, tooltip-cost removal after
received repair, and both threshold edges in one received batch. Six exact original compatibility
probes also passed: expansion2.73s, sorting2.45s, Options2.86s, mail1.85s, trade1.77s, breath1.84s.
Logs are in `shots/hud/sol61-repair/`. These are explicit offline fixtures. Natural repair NPC
selection is still unresolved: lane skill12086 belongs to repairman data while repair execution
requires blacksmith data, and the original repairman UI branch references an undefined constant.
No unproved service routing or server restart was added to conceal that gap.

The inspected `shots/hud/sol61-repair/native-controls.png` shows the original repair window,
equipped low-durability icon/notifier, bag cells, numeric repair cost and correct remaining-gold
display. The authored bag art/layout was compared with `reference/ingame/bag/real.png`.
The explicit fixture has no world scene or character-model preview; it is not a live game
capture. It measured294-352FPS, reported no script errors and exited normally.

The refreshed `target/debug/aa-client-play.exe` has SHA256
`2FF8F4EE669002BD5FDB380ED1C6F5A1ACC69027616A5915B9ED2059F7A15580`.
Its inspected `world-select.png` in this folder reaches the live Lane-bevy world list with
zero script errors;394-439FPS before capture,314FPS during capture. The process exited normally.

## Original recipe-book browsing (2026-10-18)

The original Folio uses read-only database definitions:7,010 recipes,34 groups,273 categories,
ordered material/product rows and localized English labels, loaded with the existing background
HUD catalog. Native visible expert grades are the seven SHOW rows sorted by up_limit, not all31
raw expert rows. Real recipes have at most eight materials; the getter has no invented12-row
cap. Empty visible categories remain part of the catalog. Categories and ordinary recipe lists
use native visible-order/ID sorting; search uses case-insensitive substrings without trimming,
with the verified1..30 UTF-8-sequence length limit (Unicode scalar count for valid UTF-8).
Material-name search and higher-tier links retain repeated matching material rows. The latter
uses the source recipe's last product and each consuming recipe's first product, as native.

Material quantities use received physical bag slots, raw grade requirements and the authored
secure-item gate. Row.count is the total; item_info.stack is the first positive eligible
representative's own stack. A synthesized descriptor has no owned stack and uses required grade,
or native raw0 for grade-1, independently of item.fixed_grade. A missing material/product item
definition invalidates that entire getter. Product success_rate remains the raw f32 rate, with
no percentage normalization. The executable-count getter follows the source's unusual rule:
labor limits only recipes with no materials; material recipes take the minimum material quotient.

Received proficiency points/grades back GetMyActabilityInfo; unknown totals (including a
downgrade ACK without points) remain unknown. Unit-attribute item/buff modifiers are retained
for the displayed modifier and labor calculation. Native SSE f32 labor rounding/previous-grade
threshold selection matches independent original-byte oracle cases. This does not yet fold
the separate enclosing-owned-house decoration bonus or emit the complete proficiency event set.

The native X2Item numeric-grade lookup now converts raw grades0/1 exactly once into the existing
internal display-order tooltip path, returning the original raw itemGrade. Internal bag lookups
retain their display-order convention. The complete native30-value item_impl names replace the
coarse `etc` class: the original crafting Lua compares classes when inheriting product grades,
so unrelated classes must remain distinct. Omitted-grade X2Item calls retain the existing path.

Validation:150 client tests passed /24 ignored in1.46s. Exact isolated original-stage probes
passed for recipe book, repair, expansion, sorting, mail and trade. The book probe physically
clicks the original group button, category header and recipe row and confirms the detail item
and hidden execution-button frame. Focused tests cover raw0/1 grade colors, representative
stacks, missing descriptors, secure policies, raw rates, duplicate links and Unicode length.
Logs live under `shots/hud/sol61-crafting/`. The inspected `recipe-details-final.png` shows the
original Folio and real Wrapped Potion Rack recipe/workbench/material requirements;391–446 FPS,
no script/GPU errors, normal screenshot exit. No same-screen original Folio reference exists
in the current `reference/` tree; absolute font/layout calibration remains open.

This milestone is recipe browsing. Natural workbench selection/current recipe maps, batch
requests and observed cast presentation, proficiency tier/expansion controls, housing modifiers,
special-currency material IDs and the separate playerflags2000 executable-count secure policy
remain active work. Unknown secure policy conservatively excludes flagged stacks. Protected
catalog exclusion/all-group sentinel IDs remain source boundaries. No bench context, craft
completion, product, labor expenditure or successful request is fabricated by opening the book.

The refreshed playable copy `target/debug/aa-client-play.exe` has SHA256
`B85B7F63D8731CEFFBC82C7D3E4E4C39E8668FCBCE603D3F118BCCDC19314447`.
Its inspected `world-select-final.png` in this folder reaches the live Lane-bevy world list,
with no reported script errors and357–404 FPS. It exited after the screenshot. This verifies
login/world listing; it does not resolve the existing lane game-connection stall.

## Workbench recipe maps and observed batch controls (2026-10-18)

Workbench maps now union unique, resolved recipes from every static template function group,
including CraftStart lists and CraftPack lists. They do not filter solely by the received current
phase or crafts.req_doodad_id. Alchemy Table568 contributes71 recipes and Regal Alchemy Table2241
contributes153. CURRENT group/category/recipe getters use these maps and authored visible-order/ID
sorting. Closing clears the active object/template identity while retaining the maps until the
next open. Current product-name search uses retained map membership; current material-name search
instead requires the active bench template to match recipe.req_doodad_id and preserves duplicates.

The original Craft/Max Qty/Cancel buttons now queue actual ExecuteCraft and StopCasting commands.
A request rechecks character ownership, active bench identity, received object/template, recipe
membership and CraftEffect skill classification. A complete requested count is sent ONCE. The
server owns subsequent casts; Ended never sends another ExecuteCraft. Plain and plot cancel slots
are tracked separately from received local casts. Sending Cancel does not zero the batch or alter
inventory/labor. Native first-Started, successful-Fired, Ended, failed-Started and Stopped latches
control the presentation count/events. An unfired Ended or Stopped emits no-argument CRAFT_ENDED
and resets the count at the next manager update; failed Started resets immediately. Wire Ended
contains only a timeline, so classification uses the observed current local CraftEffect cast.
This is not a product-settlement acknowledgment or a guarantee that the lane cancels queued tasks.
Mounted-caster cancellation remains open because the current StopCasting command addresses self.

Received craft-failure item notices now produce native item-link STRING arguments for CRAFT_FAILED,
with duplicate resolved entries retained and unresolved item definitions skipped. The original
chat/alert scripts consume these strings. They do not decrement/reset the queue or imply another
Ended. Synthetic link defaults use constructor-zero grade/maker, flags1 and world_idff; the two
native grade/maker globals are zero-filled in the image, with indirect initialization still an
explicit source boundary.

Validation:153 client tests passed /25 ignored. Exact original-stage workbench probe passed in1.80s
and Folio regression in1.90s, each with exactly one test executed. The workbench probe physically
clicks Max Qty, Craft and Cancel, observes one full count3 request, feeds received Started/Fired/
Ended/Stopped packets, verifies no resend, preserves inventory and confirms close-map lifetime.
Focused checks cover stale character/object identity, both cancel slots, native latch transitions,
failure-link fields/duplicates and distinct current search sources. Logs are under
shots/hud/sol61-workbench/. The initial inspected alchemy-native.png shows original Craft art,
selected Potion Rack, real material counts and enabled controls;305-367 FPS, normal screenshot exit.
The inspected alchemy-received-final.png also shows the same received three material stacks in the
bag and Labor1,000, with matching per-material totals and3/50 occupied slots;314-361 FPS. Both
captures reported no script/GPU errors and exited normally. No same-screen reference exists in
the current reference tree, so absolute font/layout calibration remains open.

This milestone uses an EXPLICIT received-state fixture to open a resolved bench, not a live
natural-interaction claim. Initial/queued native range is a geometry-shape offset predicate with
max_interaction_doodad_distance default2.0999999m, not a center-distance2.1m test. The protected
craft callback action, player-state/permission gates and full natural input routing remain open.
SCWorldInteractionSkillList/Cancel now decode in aa-net, but the lane has no emitter and null/zero
CraftStart/Pack func_skill_id cannot supply a guessed action. No server ACK or successful craft
is manufactured to hide these gaps. Broader native execution validation, special-currency and
secure-playerflag policies, full proficiency controls/housing modifiers and pack flows remain open.

## Stablehand revival service (2026-10-18)

A received NPC interaction list containing HealPetSWounds12195 now opens the original stablehand
window for the actual current stablehand NPC. The static npcs.stabler flag keeps normal talk/input
routing alive for these services. Confirmation rechecks character identity, NPC context, actual
received NPC kind/template and static service flag before issuing existing Mate::ReviveItems. Cancel
sends nothing. The native quote itself has no stablehand/affordability requirement.

X2:GetTotalRepairsForPetItems scans received physical Bag2 and Bank3 slots, excluding pending keys,
live items, missing mappings and non-summon implementations. The mapping is item_summon_mates,
not npcs.pet_item_id. A nonzero dead detail byte charges once per item regardless of stack count;
level is the received byte, with no invented cap. Costs follow original double exponent
2.299999952316284 followed by individually rounded f32 power/additions/floor, signed sum and f32
Lua return. This reproduces the documented formula and one-copper differences from the lane's
entirely-double exponent2.3 at levels144/164/219/226/244; it is not a native MSVCR pow oracle.

Validation:154 client tests pass /26 ignored. The exact isolated original stablehand test passes
in1.91s: actual received NPC skill list, physical Cancel/Confirm, stale-interaction rejection,
unchanged dead details on send, authoritative reason58 detail updates, then the original zero-quote
notice. Numeric/eligibility tests include Bag+Bank, pending/unmapped/live/out-of-capacity cases,
nonzero dead bytes and stack independence. The inspected native-confirmation.png in
shots/hud/sol61-stablehand/ shows the original9g46s78c quote for explicit level30 bag and level144
bank fixtures. The inventory has10silver: native quote/confirmation does not promise affordability
or a successful transaction.291-343FPS, no reported script/GPU errors, screenshot exit completed.
No pet was killed/revived or granted against the stalled lane. Full owned pet/slave UI, skills,
equipment, mounting and natural live service verification remain active work.

The refreshed playable copy target/debug/aa-client-play.exe has SHA256
366EC42C2E53D7F6F30FCA20C551516AF34DD83BA3A113C1B4C6DAD08BEF41D7.
Its inspected world-select.png in the same folder reaches the live Lane-bevy list at268-317FPS
while the stablehand fixture was also rendering. Both processes exited normally. This checks
login/world listing and does not establish that the lane game-connection stall is resolved.

## Current owned-pet frame and controls (2026-10-18)

The original single-pet frame/bar now hydrate from Mates::own only after a received Unit matches
its runtime object, nonzero timeline and local stored owner ID. No roster or unit is fabricated
from inventory summon items. playerpet and its object-ID string resolve through the existing
unit getters; GetUnitType returns the specific mate/slave/housing detail while GetUnitTypeString
retains its native broad-kind/detail pair. The pet owner tooltip reads received owner_name.

Capabilities use actual model point1/2 bindings and the received ten mount-skill IDs. Displayed
slots preserve received order, require real mount/combat records, and consult only each mount
record's first attached row in native query cursor order. Passenger-only skills are excluded;
driver context is1 only on this owned pet's point1. IsPlayerMounting separately recognizes any
mate attachment, including another owner's/passenger seat. PetState is a string, suppressed for
any seat on the same owned pet. GetMountSkill returns BOOL learned/needdiver and NUMBER skillUsable;
required level is source-proven skills.ability_level. Only the target_type_id6 display-usability
bypass is represented as ready; the general protected validator remains unimplemented/dark.

Rename uses the original dialog and sends the current mate timeline, with character/object/TL/
stored-mate identity checked again at dispatch. Receipt remains name authority. Passive requests
always invoke as native; Stand/Protective/Aggressive retain attackable and duplicate-state guards.
Their active commanded raw mount-skill field is separate local client state, initially0 while
the native manual action/AI planner remains unwired. These calls never mutate received Mates.

GetPetExpToNextLevel returns four f32 Lua numbers from received spawn XP and levels.total_mate_exp,
deriving its own level with equality at thresholds, bounded100 and the real level101 next delta.
It reproduces the arithmetic SSE/x87 clamp rather than substituting a host clamp. With no pet
it returns zero values. Configured-cap XP delta folding is still open; the native getter itself
does not consult that cap. Current HP/MP/level/name are received; maximum pools still use the
existing observed lower-bound fallback until native Mate5 formula/item/buff integration lands.
Full pet-stat tooltips currently retain player-attribute limitations and are not a fidelity claim.

Validation:160 client tests /27 ignored (5.31s), plus the exact original pet probe passes in1.89s.
It requires both original widgets and completed250x105 bar sizing, exercises physical rename
Cancel/OK and passive mode, checks native getter types/XP thresholds, and rejects a queued rename
after runtime-ID reuse. Twelve existing original HUD probes passed under the additive global-event
fix. The inspected native-frame-and-bar-final.png in shots/hud/sol61-owned-pet/ shows the frame
and bar at304-399 FPS; the process exited normally. Its log exposes one newly reachable original
battlefield ENTERED_WORLD counter error (nil maxCount); no zero-error claim is made.

The source fix initializes X2BattleField:GetJoinedInfos to the native manager constructor's
three values(0,0,-1), with IsInInstantGame=false for initial state0. This is the local unqueued
default, not a received queue report. The original-stage fixture now requires no entrance alarm.
Actual battlefield queue requests/receipts and state changes are being implemented separately.

The follow-up native-frame-bar-no-queue.png was inspected at454-487 FPS, with no warnings,
Lua/runtime errors or leftover window. A fresh exact original probe also hovers an unlearned
level20 skill at pet level15 through SetPetBarSkillTooltip: required learnLevel is numeric,
learned is false and the original red requirement text builds successfully (2.99s). The twelve
other original HUD probes were rerun individually and all passed. Their final-* logs and the
new no-queue.log/.err are in the same capture directory.

This integration exposed and depends on aa-ui's additive global event registration: the original
frame and bar both register SPAWN_PET. The previous implementation replaced the first callback and
repeated the second. Native listener order/release/mutation and empty unbound GetHotKey are now
covered by original pet spawn/dismiss/respawn regressions. Full pet gear/stat controls, actual skill
planning/auto-use, mount/dismiss eligibility and live world validation remain active work.

### Received telescope maps (2026-10-18)

`hud/radar.rs` hydrates ship, public-transport and fish radar from aa-net's received
resources, then replays subsequent events in packet order. Incomplete snapshots never
reach the maps. Publication precedes original AREA/REMOVE/UPDATE/CLEAR callbacks; toggles
emit AREA before the observer, including duplicate reports. Removed object IDs are Lua
numbers. Each channel clears independently. Loading does not replay historical packets.

The aa-ui radar bridge consumes the same world/continent/zone/city rectangles used by
the client's maps. Ship raw kind2 selects ship, every other value shipyard. Transfer
templates resolve through models.name (airship/carriage_main/other valid model), and
unknown templates are skipped. Transfer tooltip names are received strings; fish names
are localized DoodadAlmighty names. Ship kinds use native slave_kind keys1..9, buff tests
raw_tail==3554, and initial tooltip HP is min(current,max). The numeric health getter
instead returns raw received HP/max through f32, or zero Lua values for a missing ID.
Expedition names are omitted until received. Enemy classification currently uses the
app's static faction relation; native expedition/custom relation inputs remain open.
Range circles, fog/instance masks and actual live lane radar remain unverified.

Three bridge tests and the final full163-test client suite passed (28 ignored,1.91s).
The additional exact ignored original-stage probe passed1 test in1.82s: both maps show
eight symbols and retain all four transport/fish symbols when ships clear. Example
`received_radar` uses aa-dev and explicit synthetic receipts on the actual catalog;
`shots/hud/sol61-radar/received-original-maps.png` was inspected, with193 startup and
265-330 steady FPS, zero runtime errors and normal screenshot exit. Network fixtures
are not evidence of a live server transaction.

### Original proficiency controls (2026-10-18)

`crafting/expert.rs` replaces the original skill book's proficiency fallbacks with
SHOW-filtered native grade indices, current/previous limits, colors, price strings,
language IDs21..30, skill-page lists, exact-grade counts and separate >=grade upgrade
capacity. Capacity excludes0/19/21..30 and hidden skill-page groups; base limit0 is
unlimited. Craft-group enumeration separately excludes20 (33 actual catalog groups).
Before the initial snapshot the native getters display constructor zeros; actions and
recipe eligibility still require actual known data. Missing/unknown downgrade points
are not presented as a received total.

Original UpgradeExpert/DowngradeExpert/ExpandExpert confirmations queue typed commands,
bound to the current character and revalidated before dispatch. Current catalog regrade
prices are all raw currency0/price0; priced variants remain gated on the unresolved
native playerflags1000 payment configuration. The auto-AA bit comes from actual inventory.
Expansion checks actual vocation and physical certificates, excluding pending bag slots.
The original bag-count getter includes pending stacks and has no secure-item flag check;
neither path counts bank or slots outside the received bag capacity. No tier, currency,
item consumption or expansion count is invented from a send.

Received events update state before original callbacks, including four-argument point
and grade notifications. Complete lane snapshots refresh the original page; the native
SC136 handler itself ignores last and refreshes per packet (all current34 lane groups
fit one packet). The explicit lane downgrade helper uses ALL raw rows, distinct from
the original SHOW grade map. Housing proficiency modifiers and live transactions remain
unverified; unsupported premium-cost paths are not claimed complete.

The full client suite passes168 tests (29 ignored,2.04s), including five focused tests.
The exact ignored original-script pointer probe passes1 test
in1.91s: upgrade Cancel/Confirm, delayed upgrade/downgrade replies, expansion Cancel/
Confirm, unchanged certificate until its own receipt, and stale-character rejection.
Example `proficiency -- --confirm` and `--expand` uses aa-dev. Inspected captures under
`shots/hud/sol61-proficiency/` show the original upgrade dialog at302-339 FPS and the
certificate1/1 capacity dialog at355-439 FPS, zero runtime errors and normal exit.

## Native mate health and mana (2026-10-18)

The owned-pet frame now calculates maxima with the original owner-5 UnitFormula and
per-formula mate-kind coefficients, using actual received pet level, equipment and
the native buff-history sidecar. Static buff rules bind in First when the background
catalog is ready. Unknown history, unresolved mate kind, and unsupported conditional
or active dynamic primary/pool modifiers retain received maxima instead of asserting
that the missing inputs are zero. The legacy lane buff view remains separate.

NPC, buff and eligible item/rune/gem modifiers aggregate into one flat/percentage pair.
Native linear-level rounding, signed item levels, integer fast path, limits, broken
gear, slot eligibility and first-zero gem termination are retained. Generated item
primary stats have their own f64/f32 rounding stages; 18 results match executed
original x86 arithmetic (static lookups mocked with actual DB rows; external pow
uses Python libm), recorded under `data/extract/aa-net/mate-stats-native/`.
UnitFormula results narrow to f32 before integer truncation, including the kind-3
level-5 mana result309. Full pet UnitInfo, modifier colors, local model actions and
equipment cursor planning are still open.

Six focused checks pass. Full client suite:174 passed/29 ignored in2.24s.
`owned_pet_hud -- --native-pools` asserts calculated1012/335 versus received1000/400
and renders the original frame/bar. Inspected capture
`shots/hud/sol61-native-pools/original-pet.png`:306-524 FPS after startup,
zero runtime errors, screenshot-and-exit completed. This is an isolated received-state
fixture; it does not claim a live lane pet transaction.


## Original battlefield flow (2026-10-18)

`hud/battlefield` connects the original recruiter, queue alarm/cancel, three-choice
invitation, waiting/countdown, match bars, result roster and Leave button to separately
received `aa-net::Battlefield` state. Recruiter routing requires actual received
service12660 on a raw NPC, the authored recruiting field, native service minimum
level and original near-target/stored-NPC selection contract. It does not fabricate a
service from static NPC data. Entrance getters retain descending level bands, the
native55 cap, corps/ruleset lookup, exact victory fields and zero-Lua-result sentinels.
Native request preflight uses known native buff history and the eight explicit blockers
plus tag1253; unknown history does not assert an empty buff list.

Local request/cancel state is separate from queue ACKs and admission. Cancel resets
the local getter/UI and fires `INSTANT_GAME_JOIN_CANCEL` synchronously before its
command; received counts and invitation/Joined state change only on their receipts.
An invitation retains its entire qualifier as u64 in Rust, including values above
2^53. Accept/Next/Decline revalidate owner, token, current invitation and blockers;
ESC only closes the task, matching native vtable+18. The native custom button type
and nonmodal setting are supplied before decoration, avoiding the default Cancel
button behind the authored three-button layout. The actual HUD entered-world
continuation sends e4 only for the retained accepted offer; Joined receipt alone
never invents world readiness.

Native progress getters expose states5..8, including results in `IsInInstantGame`.
The skill getter always returns24 indexed rows while the authored bar creates10
buttons. Time uses backend-captured Pong/TimeKeeper mapping, including native
wrapping arithmetic, f32 conversion and Countdown's+6000 offset. Joined deliberately
omits start/end keys; SC191's `now` is unused. No clock-dependent table is invented
before an actual clock sample. The HUD manager survives Lua world transfers; an
actual LoadInstance resets an admitted match and emits the native leave callback.
The clock and raw received protocol remain owned by aa-net.

Received Point/Kill/SkillEnable callbacks use native admission and source-before-event
order. A local killstreak notification precedes phase/instance checks and global
kill-count updates; a victim callback is0 while the stored getter still receives the
killer's signed streak. Duplicate accepted Point receipts still notify; unchanged
skill enable bytes do not. Wait sees state6 before an immediate Start sees state7.
Frame-time promotion emits Start once, actual End emits the result callback, and
SC196 emits the unsigned half-up rounded notice without inventing a countdown.

The result getter preserves50 members per team, two team arrays, native12-key rows,
low-five bonus flags, zero-based own index and integer-truncated positive scheduled
ratio before f32 score multiplication. Original-source ScheduledEventStarted folds
its full received record and values[10] ratio; the lane defines that opcode but has
no sender. Global cumulative point/kill fields stay separate from result-team totals.
AskLeave only queues the source state8 e5 command, leaving the received match intact
until its real world-load receipt.

Slot244 resolves actual received IDs, tag204 metadata and ordinary skill icons/tooltips.
Its extra native gate scans the first matching ID among24 rows, so duplicate displayed
IDs share that first enable byte. Existing ordinary HUD resource/target/death/cooldown
checks also apply; changing eligibility refreshes the original slot's `can_use` event
and dark overlay. Protected native ordinary eligibility and actor-planner click/forced-
target behavior remain open; no guessed cast request or successful cast is claimed.
The no-active-cutscene Movie count getter returns0 so the original level-up effect
can finish; a cinematic player remains separate open work.

Twelve focused normal tests pass. Full client suite:186 passed/32 ignored in9.94s.
Three exact original-script probes pass individually: recruiter2.43s, invitation2.44s,
and full match/results/physical Leave with received-load reset2.76s. The latter
advances the original alarm animation through its ten-second automatic result reveal.
Inspected aa-dev captures under `shots/hud/sol61-battlefield/` show the clean original
invitation at190-238 FPS, received score/results at209-250 FPS, and enabled/dark
killstreak slots during play at295-314 FPS, with no runtime WARN/ERROR and all QA
windows self-closed. The real playable binary was rebuilt/copied to
`target/debug/aa-client-play.exe`; its live lane world list capture shows Lane-bevy Up,
zero script errors and369-417 FPS. No live game-session/match claim follows from
that successful login/world-list check.
All battlefield probes are isolated received-state fixtures with an explicit controlled
shared clock domain. The active lane currently gives Pong and battlefield start two
separate timer origins, so these are not claims of natural live matchmaking or timer
correctness against that lane defect. Native source contracts and executed timer
oracles are linked in `docs/aa-ui.md` and `docs/aa-net.md`.


## Original owned-pet information panel (2026-10-18)

The original equipment/info toggle now opens the original eleven-label pet information
panel over the correlated owned Mate actor. `UnitInfo('playerpet')` and the actual
object-ID alias return eleven native presentation values; `UnitModifierInfo` supplies
all seventeen raw modifier keys used by the original label-color loop. They never
reuse the player's attribute snapshot. Known-stat changes refresh the visible original
panel. Unknown native buff history, unresolved mate kind, relevant conditional/dynamic
modifiers or unsupported formula operators return zero Lua values; the application
disables the info toggle and closes an existing panel in that case. This is an explicit
unknown-data boundary, separate from native equip-command eligibility.

The owner5 attribute adapter uses the pet's final primaries, actual gear and native
buff stacks, independent static/temporary/main/rune/gem modifier buckets, single
aggregate percentages and native limits. Armor/resistance use original wearable
basis points and signed item levels; held armor narrows to f32 before integer
conversion. Mainhand DPS retains the no-override expression result before its
thousandth-unit conversion; a positive received override uses the separate f32 path.
Broken or ineligible gear contributes neither generated stats nor secondary modifiers;
gems terminate at the first zero. Empty mainhand's verified default holdable0 has
zero DPS in this catalog.

The original compiled expression operators narrow each child result to f32, including
literal/variable leaves. The new separate native evaluator implements source-closed
add/subtract/multiply/divide/power/negate/floor; the lane-oriented evaluator remains
unchanged. Division by either zero returns0 through the native error branch; NaN
continues through arithmetic. Unsupported native operators stay unknown. The existing
native Mate pool adapter now shares this evaluator and keeps its prior pool goldens.

Presentation retains separate weapon/additional DPS scaled terms, fixed5300 armor/
resistance denominators, rating multipliers using negative0.001, persistent regeneration
scaled by0.2 and independent raw modifier deltas. Modifier colors use the original
Lua helper, including its asymmetric near-zero interval. Their static baseline is
separately rounded, then the transient delta uses the source x87 operation order
modeled in f64 before the Lua f32 result; it is not a difference between clamped
attributes. No-override gear expression precision and positive overrides are tested
independently.

Executed original x86 goldens are recorded under
`data/extract/aa-net/mate-stats-native/`:72 gear wrappers/recursive AST cases,
two owner5 expression cases and ten modifier cases. Static DB/AST lookups are
reconstructed; external CRT pow/floor use explicitly mocked libm/integral-double
results. These goldens are embedded in client tests, without depending on ignored
artifact files. All72 gear values, ten returned modifier values and two expression
stages match. Native engine-component `GetSpeedInfo` remains open: the original Lua
nil-to0 speed presentation is retained, with no invented maximum-speed formula.
Pet equipment item display/cursor/confirmation/transactions remain separate work.

Full client suite:195 passed/33 ignored in14.01s. The exact original panel probe
passes1 test in2.54s, including unknown-data gating, actual info toggle, eleven labels,
object alias, all modifier keys, received armor/regeneration changes and green label
colors. The original battlefield match/results/Leave regression also passes1 test in3s.
`owned_pet_hud -- --info` and `--modified` are aa-dev capture variants. The inspected
base capture `shots/hud/sol61-pet-info/original-info-final.png` shows health720/1012,
mana260/335, attack29.20, critical6.48%, dodge5.19%, magic defense52(0.97%) and
regeneration10/9 at260-441 FPS, with no runtime errors and a self-closed QA window.
This is isolated received-state validation, not a live lane pet transaction.

The inspected modified capture
`shots/hud/sol61-pet-info/original-modified-final.png` shows received defense122
(2.25%) and health regeneration18 in the original green modifier colors, alongside
the two actual buff icons. It runs at204-306 FPS without runtime errors and its
QA window closes after capture.

### Received equipment cooldowns and complete skill tags (2026-10-18)

The original `cooldownconstantbutton:SetSkill` now reads the app's
`AAUI.SkillCooldown(rawSkillID)` hook. A missing descriptor returns no Lua values;
a known descriptor without a local actor returns0/0. Learned ownership is not a
descriptor lookup condition. The toolkit retains its native ready-once/active-per-tick
callback behavior and f32 callback conversion; rebinding only changes the cooldown
identity, leaving the item icon and physical click handler intact.

Skill descriptors now retain every `SELECT skill_id, tag_id FROM tagged_skills` row,
in SQL order including duplicates, matching395d15a0. Native3989a290 selection starts
with global timing unless `ignore_global_cooldown`, then scans all descriptor tags
and raw skill-ID rows. A candidate replaces the result only when its unsigned
remaining milliseconds are strictly greater. The winner retains its own total,
including zero; resets/expired entries do not become DB-duration receipts. Action
slots and received preflight checks now use all these tags, including shared tags
that differ from `skills.cooldown_tag_id`.

Global timing remains the existing successful-cast/DB observation. Ordinary action
slots separately retain the older own-cast estimate; constant equipment buttons do
not turn that estimate into a received timer. The new source audit confirms more
work is required: native start GCD depends on timing_id1 and attribute74, ordinary
own cooldown inserts on Fired with toggle/timing/packed-bit1 gates and skill-specific
modifiers, and stopped casts can insert supplied received durations. These are open
producer gaps, not completed native cast-manager parity. The lane's received-list
emitter remains absent; no live cooldown receipt is claimed.

Validation: full client199 passed/34 ignored in47.46s; the subsequent HUD-only
regression after isolating the legacy estimate passes133/21 ignored in42.29s.
Four focused timer/hook tests pass, and the actual original equipped-item screen
probe passes1 test in2.38s. It verifies real items15875/26743, their use skills
13419/20676, tag655/1004 sweeps, independent reset and unchanged icons/accessors.
Inspected `equipment_cooldown` aa-dev captures in
`shots/hud/sol61-equipment-cooldown/` show the two original equipment icons with
active native-handler countdowns at188-268 FPS and unchanged ready icons at405-417
FPS. Both QA processes self-close and contain no runtime errors. Shared build load
differs between captures; these are behavior checks, not a performance comparison.

### Owned-pet equipment presentation (2026-10-18)

The original pet information window now reads its own received physical equipment
and the NPC's `mate_equip_slot_pack_id`. Pack1 exposes head/saddle/feet, pack2
head/chest/feet and pack3 none. The native one-based slot IDs are1/3/4/7; missing
NPC/pack data enables no slots. `IsMateEquipableSlot` uses the existing uniquely
correlated owned-Mate identity, including owner and timeline checks. It does not
turn an unowned NPC template into an equipable pet.

`GetEquippedItemTooltipText` retains the existing player getter, with its bridge
borrow released before the callback. The current pet's token, object-ID alias
and a target alias that actually denotes that pet read the pet's own28-slot array.
An empty physical item returns a table with itemType/lookType0. A known unit with
an unknown static item retains the received template/look identity. Unknown units
return zero Lua values instead of falling back to the player's worn items.
The original native read getter accepts other resolved actor units without an
ownership guard; broader actor inspection remains unwired in this bounded adapter.
Its slot bounds check is an app guard; the native wrapper lacks that check.

The original `OnEquipmentChanged` helper owns item/grade icons, locks, durability
presentation and hover tooltips. Received full equipment rows and current-mate
FC flag rows raise `UNIT_NPC_EQUIPMENT_CHANGED` with exactly one numeric argument,
the one-based physical slot, retaining duplicate rows and order. Native event
sources3936b850/394e111d do not use item equality as an event gate. Snapshot-only
refreshes invoke the original presentation helper without inventing received
notifications. Raw mate equipment transaction receipts are still not settled into
bag/pet state: their nominal success flag is not a reliable settlement signal.

The item formatter shares the existing static item tooltip calculations and reads
the pet item's received grade, flags, durability and temper. All63 actual Pet Gear
catalog rows have no equipment-set row, so their display does not borrow the
player's set counts. Native pet actor attribute calculations remain separate from
the existing general tooltip arithmetic and its open corner cases. Cursor pickup,
drag destination mapping, bind/unpack confirmations and transaction settlement
remain separate work; this milestone does not claim those controls are complete.

Native `GetFeatureSet` reads initial-config byte1 as numeric `levelLimit` and
byte8 as numeric `mateLevelLimit` (39497025/39497071). These now come from received
GameConfig even before player spawn; a received zero remains zero. Missing bytes
use an explicit app zero sentinel, never an inferred XP-table maximum. Other
feature fields still retain their prior providers. The original tooltip therefore
uses the pet's own level/cap. Native item formatter3942281a calls397cf020 with tag29
to produce the boolean `isPetOnly`; that field now comes from actual tagged_items.
All63 current Pet Gear templates have tag29. Source artifacts are under
`data/extract/aa-ui/native-feature-level-caps-20261018.txt` and
`data/extract/aa-net/pet-only-{source,runtime,items}-20261018.*`.

Validation: full client203 passed/35 ignored in53.62s; four focused equipment
tests pass, including the original physical panel toggle and real hover tooltip.
The original pet-statistics and equipped-item-cooldown probes also pass separately
(3.25s/3.09s). Duplicate slot events, removal of an actual icon, replacement pet
identity, absent/unknown items and the two NPC pack layouts are covered. Numeric
feature cap tests include absent, zero and255 bytes without positive defaults.
Inspected aa-dev captures in `shots/hud/sol61-pet-equipment/` show the Snowlion's
head/saddle/feet at194.6-196.8 FPS and Wolfhound's head/chest/feet at189.2-222.3 FPS.
Both capture processes self-close with no runtime errors; the user's existing
world-selection client remains open. These captures use explicit received fixtures
and do not claim a live pet-equipment transaction.

### Original family controls and received roster callbacks (2026-10-18)

The original Social/Family tab now uses real family getters and requests. Its UI
cache retains the native first8 rows, name128/title104 byte buffers and first-NUL
string behavior, separately from aa-net's complete lane roster. The actor's family
ID starts from actual PlayerSpawn and changes only through native per-member
callbacks; another family's roster does not establish local membership. Missing
cache does not erase a nonzero actor family ID. GetMembers returns dense
name/title/raw-role/online rows in received order before the original UI sorts
them, and GetMemberCount returns online,total. Offline filtering and case-sensitive
IsMyFamily use those rows; IsOwner tests the first local-ID row's nonzero role.
GetMembers' original checkbox-boolean path is verified; nonboolean binding
coercion remains unclaimed (the app refuses it with zero Lua values).

Every received change mutates state before its original FAMILY_* callback. Equal
online notices still fire, removal affects the first matching row, and dropping
below2 members dissolves the cache without inventing an extra FAMILY_REMOVED.
Explicit removal emits FAMILY_REMOVED even without a cache entry. Created uses
rows0/1, later additions/title changes use the last nonzero-role owner name (or
native "X"). Owner changes clear all other roles and set every matching-ID row;
the last matching name supplies the event. Described has no immediate UI event.
Invalid incoming addition indices use an explicit app bounds guard. Overhead
FamilyTitle unit presentation is separate and remains open.

Invite, Leave, Kick and ChangeTitle require received feature byte0 mask2. The
ChangeOwner wrapper and captured Task4 reply do not have this feature gate.
Native ownership/membership/name checks use the UI cache; failures use original
localized FAMILY_* errors. All six typed requests retain the selected character
ID and are rechecked against the actual player before sending. Roster/membership
updates remain received-only, including after a requested ownership transfer.
The additional absent-actor and stale-character checks are app guards.

Invitations use the original type4 nonmodal OK/Cancel task, invitorName/familyTitle
fields and numeric account-backed option74 (native g_ignore_family_invite).
Duplicate queued invitations and ignore=1 automatically queue a decline of that
captured inviter/title. Join, Decline, close and task cancellation answer once;
tokens survive resets monotonically so an old callback cannot answer a later
invitation. There is no invented existing-family refusal. Without a player the
invitation is discarded. During loading the app retains the task until the world
HUD exists and applies option74 at presentation; native non-HUD task timing is
not claimed. Scene/character reset closes old task windows and clears pending
actions while preserving the received connection feature bit appropriately.

Sources: native wrappers394747e0/39474840..394748d0, family cache398a8730,
getters392ffeb0/39300440, callbacks392ff2e0..39300340 and Task4
391d29e0/392ff820/392ff590. Detailed source artifacts are under
`data/extract/aa-ui/native-family-*` and `data/extract/aa-client/family-native/`.

Validation: full client207 passed/36 ignored in28.41s. All five focused family
checks pass, including the original Invite/Join/Decline buttons, member popup
Kick/ChangeOwner/ChangeTitle, its original title edit dialog, received-owner
transition and Leave, plus original task cancellation. Unit probes cover native
duplicate/name/buffer/cache rules, mutation-before-callback, feature gates,
captured replies/ignore behavior and stale selected characters. Inspected aa-dev
`family` captures under `shots/hud/sol61-family/` show real fixture rows with the
original offline color and3/4 count at337.5-378.5 FPS, and the original invitation
at306.6-371.2 FPS. Both processes self-close without runtime errors. These are
controlled original-script checks; paired live family settlement is unverified.

### Received stop-duration cooldowns in the original HUD (2026-10-18)

Action sweeps, the original equipped-item constant buttons and click preflight now
read aa-net's mutable native cooldown rows. These include original SCCooldowns
snapshots and admitted CastingStopped/PlotStopped durations; the separate raw
snapshot history remains unchanged. Selection retains every actual descriptor tag
in SQL order, then the raw skill ID, strict unsigned greater-than and each winner's
own total (including zero). Global timing still uses the prior observed-cast/DB
estimate; this does not claim native default/custom GCD or Fired producers.

The actual skill ID to primary cooldown tag catalog is bound once in First, before
network PreUpdate, from GameData's existing skills query. A known zero routes to
the direct skill timer; missing catalog rows remain unknown. aa-net shares this
static map through Arc across clones and lifecycle resets, so the HUD's per-frame
Skills copy does not clone the whole descriptor table.

An admitted stop requires the uniquely tracked local actor timeline, a known
descriptor routing row, a nonzero received duration and the actual received local
GM mode1 byte equal to zero. It inserts before removal of the existing cast-bar
record. The first matching timer extends only when the new duration is greater
than its current integer-millisecond remainder; expired compaction preserves
survivor order and the native150-row capacity. Such a stop also invalidates only
that skill's older app DB estimate, even if an existing longer native timer wins.
Consequently the old estimate cannot mask the received duration or reappear after
expiry. A later actual Started can establish the existing fallback again. Unknown,
ambiguous, nonlocal, zero-duration and GM-blocked stops do not apply this policy.

Native ordinary TL records actually outlive stop, with reference/frame cleanup;
plot active-caster lifetime differs. The current actor-keyed cast model cannot
represent all duplicate/repeated timeline cases, so it deliberately refuses that
ambiguity. Full native tracking, Fired duration modifiers and attribute74 GCD
remain open. Source contracts and actual-code oracles are recorded in aa-net/UI
docs and `data/extract/aa-net/native-cooldown-timeline-design-20261018.md`.

Validation: six focused client checks pass, including real catalog installation
through Bevy First, actual received stop folding, fallback expiry/new-start
behavior and the original item button's7000ms sweep (2.37s). Full client208 passed/
36 ignored in33.21s. Separate network checks cover unchanged longer timer rows,
unrelated estimates, admission failures and static Arc sharing through resets.

The inspected `equipment_cooldown --stopped` aa-dev capture in
`shots/hud/sol61-native-stop/original-stopped.png` uses a received12000ms stop after
the actual reset/start sequence, with the original item icons/countdowns intact;
245.1-277.2 FPS, zero runtime errors and self-closed. The refreshed main build's
world-selection capture in the same folder reaches the live Lane-bevy list with
zero script errors at387.8-463.0 FPS. It was compared with
`reference/world_select/real.png`; UI/art placement remains aligned. This login
check used a separate QA account and does not claim a live in-game stop receipt.
The tested executable `target/debug/aa-client-20261018-hud.exe` has SHA256
`5EC6517593D6FBFAEBAF9ACEA8D8842C313920899E4A895561E4E9F104B55003`.

### Original guild roster, permissions and management (2026-10-18)

The original expedition window now reads aa-net's admitted native faction, policy
and member caches. It uses the actual selected/spawned character's guild and role;
receiving a faction or an incomplete roster does not invent local membership.
The native loaded-ID comparison includes reset0==0, while the original Lua also
checks for a nonzero own guild. Names retain byte buffers until Lua's first-NUL
conversion. Policies use the native ten-column order, excluding the wire dismiss
byte; the original Lua adds its own derived raid/memo/policy columns.

LocalRequestExpeditionMembers performs the native local query, without a packet:
stable level-descending order over character-ID order, nonzero online filtering,
50 rows per page and the original exact-multiple empty-last-page behavior. The
original screen displays ten rows from each callback. Native f32/signed conversion
and wrapping page decrement are preserved, including invalid float conversion;
non-boolean/non-numeric inputs are refused at the app adapter boundary. Online
locations use id3 subzone, zone-key display text and localized world-group name.
Offline rows use actual Unix seconds,360-day years and30-day months, with all six
duration fields and the native unsigned future/negative-time guard. This is not
Windows FILETIME despite the native helper's name. Missing location data remains
absent rather than using the player's location.

The shared period formatter follows original394245f0: unsigned64 division,
signed low-DWORD components converted through native f32, and subtraction of the
full quotient only when its signed component is positive. A365-day duration is
one year and five days. Calendar date formatting remains separate. Eighteen
executed original x86 cases in hud/period_cases.json cover normal boundaries,
negative raw bit patterns, signed/unsigned quotient overflow and f32 rounding;
the expedition caller retains its unsigned future-time clamp. Oracle instructions
and captures are in data/extract/aa-client/period/. Focused verification is
recorded with the later reset milestone after compilation.

Received changes update the cache before their ordered original EXPEDITION_*
events. MemberStatus emits exactly one notice: an existing name-flag row emits
NAME_CHANGED, another existing row STATUS_CHANGED, and a missing row UPDATED.
Initial member batches do not invent an event. Policy changes require their
native existing-key/success conditions; owner settlement updates faction metadata
before its policy-dependent role changes. aa-net also updates only admitted,
matching existing actors for membership/dismissal receipts. No request changes
membership, rank, ownership or permission values optimistically.

Invite, Kick, rank, Change Title/Apply and Delegate use source-backed native
permission, target-name and rank checks, including equal-rank kick admission and
the singleton-leader Leave-to-Disband route. Names use first-NUL ASCII case folding;
locale-dependent high-byte CRT folding remains unclaimed. Policy names follow the
actual en_us/type2 path, including its byte-indexed trailing-space quirk, verified
against23 executed native cases. Malformed UTF-8 and unsupported argument types
are explicit app refusals. The native policy decoder leaves its dismiss byte
uninitialized; this app deliberately preserves the known received value and
normalizes it for the writer instead of claiming deterministic native behavior.

Delegate checks aa-net's received Dominion state: only siege period4 and a matching
defender/offense guild block it. The native constructor's empty map is known empty;
an unhandled relevant packet, abandoned compressed tail or decode failure makes
nonzero-guild eligibility unknown. A later individual row or final-data flag does
not restore completeness. Unknown eligibility refuses the action without inventing
the native active-siege error. Full territory/siege UI remains separate work.

Invitations use original nonmodal Task3 Join/Decline with captured inviter/guild IDs,
cached faction-name lookup and account-backed option73. Duplicate tasks and ignore=1
automatically answer decline; the native existing-guild callback error also applies
to decline. Leave and Disband use original modal Tasks26/27 and send only on a
positive result. Each task answers once with captured actual character/object IDs;
tokens remain monotonic across resets. Character/guild context changes clear queued
actions and close old task windows. Pre-HUD invitation presentation and stale-actor
refusal are explicit app lifecycle boundaries. Local write notices are not server
acknowledgments.

The aa-dev `expedition` example supports default permissions, `--members`,
`--invitation`, `--leave` and `--dismiss`. Inspected captures under
`shots/hud/sol61-expedition/` show original permissions at286.8-313.2 FPS,
member/location/offline rows and3/4 count at286.8-424.2 FPS, invitation at426.5-471.0,
Leave at294.4-324.2 and Disband at388.8-426.6 FPS. All five processes self-close
without runtime errors. These are controlled original-script fixtures; there is
no reference guild screenshot in this checkout and no paired live guild claim.
Creation/rename, web content, siege management and overhead guild titles remain
open. Native source/packet evidence is in docs/aa-net.md and targeted artifacts
under `data/extract/aa-net/native-expedition-*`,
`data/extract/aa-net/native-dominion-received-20261018.txt` and
`data/extract/aa-client/expedition/`.

Validation: all11 focused guild checks pass in4.54s. They include physical original
Apply/Change Title/Invite, Delegate/OK, Leave/Disband Confirm/Cancel and Task3
Join/Decline controls; captured typed kick/rank requests; native cache/getter,
callback, paging, name and stale-identity boundaries; received siege eligibility
and actual owner settlement. Full client219 passed/36 ignored in14.37s against
the completed native chat clipping and received Dominion/decode-gap changes.

The refreshed main `target/debug/aa-client-20261018-guild.exe` reaches the live
Lane-bevy world list with zero script errors at260.3-361.5 FPS; its inspected
`world-select-final.png` is under the same capture directory. SHA256:
`B9D37122CC7910E138F342B735A8B32DD85AC1B97D8AE85709677AF9766B62A2`.
A fresh `original-members-final.png` from the integrated example confirms the
same original rows at316.6-450.0 FPS with no runtime errors. Both self-close.
This verifies login/world selection; the existing lane game-connection timeout
still prevents natural in-game settlement QA.

### Original guild creation and received rename prompt (2026-10-18)

Original creation now sends the captured Create command after name validation and
the native own-guild/current-faction cache checks, preserving error order0x22 then
0x21. Its sponsor argument follows the actual numeric-only f32/CVTTSS2SI adapter,
including negative two's-complement and invalid-conversion0x80000000. The current
faction starts from Character.faction_id and follows guarded received SC01b changes.
Current faction IDs>=1000 replace the supplied sponsor:1000 is verified in the
original1.2 runtime and original Lua; the protected native global's initializer
remains unresolved. It is an explicit build compatibility value, not a claimed
received GameConfig field.

GetMyTopLevelFaction follows cached mother links and returns the last reached ID
when a link is absent or mother0 ends the chain. It has no static Nuia fallback.
Malformed cycles are an app refusal instead of native nontermination. Original
Nuia/Harihara sponsor allocation stays seven tabs, with six/five active buttons;
outlaw and user-alliance layouts use native SelectTab(0) to hide every pane.

The lane's received guild-service skill13736 opens the original NPC preconfirmation
and sponsor flow only for the actual local actor, matching pending NPC, known
expedition template flag, existing live NPC and six-metre talk range. NPC removal,
range or context change cancels the service. It sends no creation command until
the original Create button is clicked. This follows the lane's service mapping;
multi-service selection and quest/service precedence remain the app's existing
bounded policy rather than a claim of full native NPC planner parity.

VNT_FACTION2 now exposes the native en_us name-policy record: local2..32,
English3..32, limit_space_cnt0, space/mixed-case allowed and mixed-language/special
letter flags false. Its original IsValidName uses the already executed type2
validator, including the native trailing-byte quirk. The later shared-name milestone
below replaces the other policy adapters too. Rename's original edit handler has
autoTextChange=false, so it intentionally does not call CheckAvailableName.

Actual empty SC00f opens original nonmodal Task35 only for a cached own faction
with political_system0 and matching actual owner; failures and duplicate tasks
are silent. Captured expeditionName and original edit/OK/Cancel controls own the
presentation. Its native task result callback does nothing; original Lua OkProc
calls RenameExpedition. That direct method sends the current guild ID/name and
returns boolean true without name/owner validation; true is invocation, not
success. Character/object/task tokens suppress stale OkProc closures. Received
Renamed/Created and actor-membership receipts alone change state. The lane's
Rename handler currently logs the request without implementing success; no
server-side completion is fabricated.

The lane constructs new guild factions with political_system1. Native Created
membership/menu effects require incoming political_system0, so those effects
correctly skip that SC013; the later real SC01c(0->newGuild) establishes local
membership. This is a lane/native mismatch, not a reason to infer creation success
from its request. The same political-system mismatch prevents native Task35 for
such lane-created guilds, independently of the lane's log-only Rename handler.

The `expedition` aa-dev example additionally supports `--create --alliance
148|149|114|1005` and `--rename`. Source evidence is in
`data/extract/aa-net/native-expedition-create-rename-20261018.txt` and
`data/extract/aa-ui/native-name-policy-table-20261018.txt`. Full received/raw-row,
actor guard and transport contracts remain in docs/aa-net.md; native active-tab
setter/selection lifetime and independent original sponsor captures are in
docs/aa-ui.md.

Validation: all16 focused guild checks pass in7.62s, including physical original
Nuia/Harihara/outlaw/user-alliance Create, active sponsor picking, numeric adapter
edge cases, original NPC preconfirmation, stale service/actor refusal, Task35
Confirm/Cancel/duplicate handling, received rename settlement and stale OkProc
suppression. The native direct Rename empty-name/nonowner call is checked
separately from its guarded task. Full client224 passed/36 ignored in14.87s
against the corrected SC01b decoder, native active-tab changes and authored fog
implementation. These controlled original-script tests do not claim live guild
creation/rename settlement.

Inspected fresh aa-dev captures in `shots/hud/sol61-expedition-create/` show Nuia's
six sponsors at287.8-395.2 FPS, Harihara's five at283.2-378.3, outlaw at292.5-381.0,
user alliance at374.9-390.2 and original Rename at320.3-398.4. The native3-32 guide
and four source layouts are visible; every process self-closes with zero runtime
warnings/errors. A refreshed main world-selection capture reaches live Lane-bevy
at266.3-346.9 FPS with zero script errors. These are inspected original-script
fixtures, without a native guild reference screenshot comparison.

Tested main: `target/debug/aa-client-20261018-guild-create.exe`, SHA256
`A59873D362C91FAE163AB7B9084871AF4220562BD470B78A0179C0EEFA8C41D6`.
The playable alias `target/debug/aa-client-play.exe` is refreshed from that copy
after confirming no user process is using it. This build includes32fd907 active
tabs, c39411a network creation/rename/faction handling and906413a authored fog.

### Shared original English name rules (2026-10-18)

`login/name_policy.rs` now installs the actual en_us policy metadata, validator and
formatter for all login stages, character creation and HUD name controls. The two
Lua fallbacks and guild-only override are removed. Policy0/1/6 (character/summons)
uses2-26 with no internal ASCII whitespace or uppercase after the first character;
guild policy2 has local2-32/English3-32, family-title3 has2-26, chat-tab4 has2-10,
and portal5 has3-32. Guild/chat/portal allow internal ASCII whitespace and mixed
case; family title allows mixed case. The actual nine metadata fields drive the
original scripts' maximum lengths and localized guides.

These are the original rules, including accepted Latin punctuation/digits, DEL,
the first-byte English/local length choice and the trailing check that indexes
original UTF-8 bytes with the decoded scalar count. Raw length includes a leading
BOM, while validation/format decoding strips exactly one BOM. Embedded NUL ends
the input before decoding. Formatting preserves punctuation/spaces, limits to32
decoded scalars, and, when its second argument is false, uppercases an alphabetic
first scalar and lowercases the rest. A leading ASCII space then yields empty;
a nonalphabetic first scalar leaves case unchanged. Exact original Xl/English CRT
maps replace Rust Unicode folding; codepoints above0x7ff remain unchanged. This
preserves cases such as `a_1B -> A_1b`, `éCOLE -> École` and `ſAbC -> ſabc`.

The typed wrappers preserve native numeric-f32/CVTTSS2SI kinds, numeric/boolean
formatter arguments, strict string input and zero-result parse failures. An
accepted numeric but invalid validator kind returns one false. Invalid UTF-8 in
the C-string prefix is an explicit safe app refusal: validator false, formatter
zero results. The native unchecked malformed decoder is not reproduced.

Original scripts still decide whether to format: pet/character name fields do,
chat/portal/guild rename fields intentionally do not. Local acceptance never
fabricates server availability or create/rename success. The independent native
captures are retained in the tracked `login/name_policy/native_cases.json` and
`native_helpers.bin` fixtures:448 validator cases,58 formatting cases,31 typed
wrapper cases and the helper output domain0..0x7ff. Source/oracle provenance and
malformed-input limits are also recorded in docs/aa-ui.md.

Validation: all230 normal client tests pass/36 existing probes ignored in17.12s;
the separate original pet frame/rename/physical Cancel/Confirm/stale-identity probe
passes in5.39s. The actual original character flow advances Step1→Step2→Step3,
then checks typed26-character capping, unrestricted SetText, real OnTextChanged
formatting, pending-create disable and IME preview/commit (`éCOLE -> École`). Chat
and portal controls test distinct typed caps and intentional unformatted case.

Fresh inspected aa-dev captures under `shots/hud/sol61-names/` show pet rename
`A_1b` with2-26 guide (250.3-307.7 FPS), chat `Raid Room!` with2-10 guide
(225.3-293.8), and portal `Western Coast` with3-32 guide (319.4-437.9). All three
close themselves with zero runtime warnings/errors. Fresh main world selection
shows live Lane-bevy at196.2-422.0 FPS and zero script errors/warnings. Original
Elf-female Step3 uses the actual Battlerage button and name-field click, types
`a_1B`, verifies `A_1b`/enabled Create, and closes without submitting anything;
80.3-164.0 FPS on shared load. It has zero script errors and the existing one
missing `login_stage_effect.elf_fog_01` asset warning. The source/reference Step3
comparison preserves panel/name placement; existing lighting/material/field
differences remain documented in aa-client-create.md.

Tested main: `target/debug/aa-client-20261018-names.exe`, SHA256
`A57C89D80CA5B3A5BBFC56B67541CF8550A813FBC07AAB47F3A60F87E3730657`.
This includes24694fd client skill-payload integration, ce96961/f05af16 complete
received skill result/identity, f45717b native English edit behavior and the
333b414 global FogVolume implementation. The playable `aa-client-play.exe` alias
is refreshed to the same hash after confirming it has no active user process.
GUI QA processes are all closed.

## Duels (2026-10-20)

`hud/duel.rs` + `duel.lua` over `aa_net::Duels`. The original has no script for the challenge dialog:
native raises `DLG_TASK_CHALLENGE_DUEL` (9) and the `duel_result` message box with the texts of
`locale_helper` (`locale.duel.challenge`, `ask_duel_msg`, `locale.messageBox.duel_result`). So:

* `SCDuelChallenged` opens the default OK/Cancel dialog (`locale.duel.ask_duel_msg(challenger)`); OK
  sends `CSStartDuel` 0, Cancel `CSStartDuel` 507. The name is the visible unit with that character id,
  else the streamed character name, else the id.
* `SCDuelStarted` fires `STARTED_DUEL` (the target frame refreshes its faction) and prints the
  `DUEL_TEXT` "start" text; `SCDuelEnded` fires `ENDED_DUEL` and shows the result notice
  (`result_winner` / `loser` / `draw` / `runaway`).
* `X2Unit:CanChallengeTargetToDuel()` / `ChallengeTargetToDuel()` (the target popup's "Challenge to
  Duel", `popup_menu_proc`): a living player other than oneself, no duel running, no open challenge.
* Name plates and the target frame follow the temporary faction through `relation(own, faction)`; nothing
  duel specific.

A challenge that arrives before the HUD is built is prompted once it is. Live (our server lane, headless bot as
opponent): the bot's challenge opens "Request Duel", Confirm accepts, the opponent's plate and target frame turn
hostile for the duel and green again after, the loss shows "Duel lost."; the target popup's "Request Duel"
challenges the bot and the duel starts.

Second pass (2026-10-20), all live against our server lane with the headless bot as opponent (screenshots in
`<aaemu-src>/duels2/`):

* **Results** (`duel_result` box, texts from `locale.duel`): win ("Duel won!", the Bevy player won with `fight 2` after
  `face name:...`), loss ("Duel lost."), draw (both at full health, the 5 min server timer, "Duel ended in a draw."),
  run-away (walking 75 m from the flag: "Surrendered."), the opponent leaving the world mid-duel ("Opponent
  surrendered.", `result_remained`: the original keeps a separate text for the side that stayed, and the server's
  `det` 2 with the local character as first id is that side), and the refusal ("Opponent declined duel.",
  server error 507, no dialog). Name plates and target frame are hostile (red) for the duel and green again after every
  outcome (`SCUnitFactionChanged` temp flag).
* **Ignore duel invitations** (`OPTION_ITEM_IGNORE_DUEL_INVITATION`, numeric option 78, the Options window checkbox
  of `interface_option`; the 1.2 client also has the console variable `g_ignore_duel_invite`): the option item is
  created with its default 0 at install; a challenge that arrives while it is 1 is refused at once with
  `CSStartDuel` 507 and no prompt (the same answer as the Cancel button), like the family/expedition/jury options.
* **Countdown**: the 1.2 text table has `DUEL_TEXT` "countdown" ("Duel will begin in %d sec."); the UI scripts have no
  countdown widget (only `STARTED_DUEL` / `ENDED_DUEL`), so the native handler of `SCDuelStartCountdown` displays it.
  The client runs a fixed 3 s countdown (the 10.x server's `SCDuelStartCountdownPacket` notes the same), announced as
  system messages "... 3 sec.", "... 2 sec.", "... 1 sec." and cut short by the start or the end. A countdown that
  arrives after `SCDuelStarted` is not run. Our server sends both packets together after its 3 s wait, so the
  countdown never shows live (server suspect, see the report); it is covered by unit tests.
* **Duel-area warnings**: none. The server only ends the duel silently at 75 m from the flag (`det` 2); the
  `duel_distance_invalid*` / `duel_bad_location` strings exist only in the 10.x client, the 1.2 x2game has no
  such warning.
* **Server duel errors** (fixed server: countdown at acceptance, validation, expiry) arrive as `SCErrorMsg` and show through
  the generic alert path with the 1.2 texts, live: 513 "Unable to duel right now." (self), 504 "Unable to request a duel
  with that player.", 505 "Already dueling.", 506 "That person is already dueling.", and 507 "Opponent declined duel."
  when an unanswered challenge expires after 30 s. The countdown shows live ("Duel will begin in 3/2/1 sec." before
  "The duel with <name> has begun!"). `Command::ChallengeDuel` no longer refuses a self challenge itself (the server
  answers 513); `HUD.duel_challenge(id)` sends a challenge without the popup's checks for scripts.
* Script runner: `face SPEC`, conditions `duel`, `duel_challenge`, `duel_result KEY`; `fight` ends when a duel ends
  (see `aa-client-script.md`).

## Titles

`hud/appellation.rs` over `aa_net::Appellations`, behind the original title window of the character window (no
script is replaced). Wire side in `docs/aa-net.md` ("Titles").

* `X2Player:GetAppellationCount()`, `GetAppellations(page)` (one page of the owned list; each row carries `id`, `name`
  and, when the title gives a buff, `buff_id`, `buff_name`, `path`, `description`, `buff_kind` from the DB and
  `tips::buff_text`), `GetShowingAppellation()` and `ChangeAppellation(type)` (`Command::ChangeAppellation`; 0 takes the
  title off).
* Events: `APPELLATION_CHANGED` when the own list or shown title changes, `APPELLATION_GAINED(id)` for
  `SCAppellationGained`. A list that arrives before the HUD is built is raised once it is.
* English names come from `localized_texts` (`'appellations'` in the `hud/data.rs` name tables), the buff from `appellations.buff_id`.
* Name plates (`hud/plates.rs`) draw the title as a prefix on the name's own line, "<title> <name>": one space, no
  brackets, same font, size, colour and shadow as the name, the whole line centred like a name-only plate (retail 1.2
  capture; no title draws the name alone). The own title comes from `Appellations.active`, other units' from
  `Unit.appellation` (spawn and `SCAppellationChanged`).

Tests: `cargo test -p aa-client appellation` (5). Live (AAEmu Rust server, headless script and windowed): owned titles
list in the window, changing and removing the title updates the own plate and the server's answer.

## Beauty salon

`hud/salon.rs` + `hud/salon.lua`: the original beautyshop window over the character-create state, its stage, the paid change
and the rebuilt player model. See `docs/aa-client-salon.md`.

## Chat tab transfer (2026-10-23)

The client wires the toolkit's native split/reparent notifications through
`AAUI.ChatAllocateWindow`, `ChatTabDetached`, `ChatTabAttached` and `ChatReleaseWindow`.
Allocation uses the lowest free ID 0–7 with selection 0, input method 0 and an unlocked window.
It saves the tab/extra/lock records in one settings-file replacement. If saving fails,
both the application model and settings snapshot roll back before the allocator returns nil.
The toolkit then refuses the split safely; the original allocator's void-return/data-loss
bug is not reproduced.

Detach removes the raw ID from its current window order, retaining its complete tab record
and pending recovery information. Empty source window/extra/lock records are removed.
Attach updates the existing raw record's owner and insertion order; it never creates another
tab or message frame. Input method and lock lookups use current membership, so a pending
tab cannot update its former window. Viable rollback restores the source's method and lock.
Reset and character/world identity changes clear pending operations. Release is idempotent
and never deletes a populated window or any tab.

Pending records retain their former window IDs and extras in saved settings for later
reload recovery. Layout equivalence after both owners retire or IDs are reused is not
claimed. The native toolkit retains its separate live message history. History stays with
the same message frame during normal split/drop; this change does not add disk history.

Five client checks pass, including physical dragging through the original World/chat
bytecode, unchanged message/button identities and history, all 200 filters/colours,
current-owner method changes, a fresh UI-runtime reload, batch ordering, allocation limits,
rollback and actual disk-write failure. Together with the five music checks, the final
focused selection passes 10/10. The broader client suite is checked separately.

The original-script toolkit before/after images are
`shots/aa-ui/native-chat-split-{before,after}-20261023.png` (1920×1080), inspected by root.
Observed shared-load FPS ranges were 263.1–389.9 before and 196.8–244.5 after; no controlled
performance comparison is inferred. Native selection/clone/drop ordering and 32 executed
oracle groups are recorded in `docs/aa-ui.md`. The subsequent toolkit change `9a618ee` adds
slide/caret previews and release-time reordering, with 271 toolkit checks and 32 native groups.
Same-owner reordering uses `ChatTabAttached` alone; the client adapter already preserves its
numeric selection and input method. Broader geometry/class/hover/link parity remains open.

## Received overhead affiliations and original option save path (2026-10-25)

The original interface option's affiliation radio now reaches
`X2NameTag:SetNameTagFactionSelection`, backed by the same pending numeric option52
as `X2Option`. Its unchanged `interface_option.alb` callbacks initialize from
`option52+1` and save `checked-1`. The existing plate reads that getter, so guild / family
changes update its text without replacing the plate. Cancel restores the opening value;
Save and a fresh runtime use the existing per-account settings file. This does not add a
second affiliation preference or rewrite the original Lua consumers.

`SCFamilyTitle` is an object-owned received value, not a family-roster fallback. The
plate observer now replays object creation, replacement, removal and world retirement
in message order, including the own object, before accepting a title. It observes those
lifetimes before screen entry too, so a PlayerSpawn cannot expire during world loading.
An unmatched receipt is ignored rather than queued for a future object with the same ID.
Empty owner bytes up to the first NUL explicitly remove the previous line. Own unit
snapshots preserve a live title; removal/replacement and world exits retire it.

Formatting uses the current UI locale's category92 `community_desc`, `owner_title` and
`family_leader` entries. Role0 never substitutes a leader title; every nonzero role does
so only when its received title is empty. Guild names still require the matching received
faction record with nonzero mother ID, and a known eligible empty name preserves the
native `<>` line instead of becoming unresolved metadata. Ordinary plate range, geometry,
title/name order, marker sprites and unrelated name-tag options are unchanged.

Native contracts are retained at `393006a0` (family formatter), `391d2e60` / `39360020`
(receipt / loaded-object lookup), `39366430` (copy and rebuild), `3935dde0` (guild row)
and `39491740` (affiliation setter). Existing bounded native controls and original ALB
pins are preserved in `data/extract/aa-ui/overhead-affiliation-20261025/`; new checks and
preservation hashes are under `data/extract/aa-client/affiliation-20261025/`.

The original callback/store regression fails before the bridge (option remains0), and
three actual-system regressions fail before the family correction (unknown-unit caching,
stale reused-object title and hardcoded English). Nine plate checks, the unchanged ALB
option callback/persistence check and nine graphics checks pass. The numeric adapter has
21 new unchanged-x86 controls with modeled ScriptAny and method-call endpoints; they prove
parameter admission/conversion, not arbitrary CVar callback policy. An existing sound-only
fixture exposed absent `X2NameTag` during the first full run (787 passed / one failed).
Registration now checks that the optional table and account bridge exist, without changing
that fixture. Fresh R2 review is clear, and the final corrected full client run passes
**788 tests, zero failures, 91 existing optional ignored** (128.47 s). All 23 prior research
files remain byte-identical. The all-targets client check passes. The single final scoped
Clippy exits0 with 182 existing client warnings and no primary diagnostic on a changed line;
all nine frozen production/test files still match their v3 hashes. The current lane has no
runtime SCFamilyTitle sender, so this is
received-path coverage, not a live family-overhead claim. Nation presentation, mode53,
the complete native name-tag option admission policy and billboard geometry remain open.


## Received skill speech and native live-object strings (2026-10-25)

`SCChatBubble` (0x154) now resolves an ID in localized `bubble_effects.speech`.
The former `bubbles`/`bubble_chats` lookup and invented 3.5-second continuation
queue were a different feature and have been removed from this route. Missing
effects remain silent. Empty/leading-NUL localization falls back to authored
speech; C-string prefixes are preserved without trimming whitespace. Ordinary
inline/effect speech sends exactly six `CHAT_MSG_QUEST` arguments. Raw kind4
sends only `CHAT_MSG_ALARM(text)`; other raw kinds are preserved. The initialized
native speed wrapper produces89 ms/codepoint, a500 ms show minimum and the
separate f32 SSE fade arithmetic (46 codepoints:4094 ms and317.9921875 ms).
Original Lua still owns display, tails, filters, chat logging and projection.

The public live-object domain is lowercase, unpadded hexadecimal. Object100 is
`"64"`, object256 is `"100"`; numeric Lua ById inputs are rejected before lookup.
`GetUnitId` accepts implemented unit tokens, rather than treating a raw object
string as a token. Private HUD tables keep decimal strings through explicit
used adapters. Name/grade/target getters, chat/combat/party/marker/pet events and
projection consumers cross the public boundary consistently. Character, party
membership, item and other persistent IDs retain their separate domains. The
legacy owned-pet info provider keeps its owner field and gains missing shared
name-colour fields only when a received unit snapshot supplies them.

The scheduled 0x154 consumer uses a matching live LocalPlayer root and current
remote root before any packet/HUD cached position. Logical units without a
render tree retain the existing movement Track in Views; render admission and
eviction transfer that clock and committed pose instead of resetting it from a
snapshot. Source lifetime and movement are folded together in receipt order,
so an old movement report cannot enter a replacement with the same ID. Failed
models, preparation and camera culling do not by themselves unload an actor.
The same movement sampling/ground snap is used with and without a model. A
relative attachment report without a supplied world point is not interpreted
as world coordinates. Ordinary emote text is independent of this range gate.
Range is3D and includes exactly30 m. Hard world/session retirement remains
fenced until a new own spawn; temporary HUD unavailability does not retire the
actor lifetime. Rendering availability still determines whether the current
Lua receipt can be delivered; no delayed speech queue is introduced.

Source and bounded execution evidence are in ignored
`data/extract/aa-net/chat-bubble-native-20261025/`, its `unit-id-followup/`,
`data/extract/aa-client/chat-bubble-native-20261025/` and
`data/extract/aa-client/chat-actor-points-20261025/`. Original archive primitive
normalization remains protected/unproved, so aa-net's wire variant codec is
unchanged. The native ID hook is an enabled shared voice slot (pack370), not a
text macro. The provided voice cache has no sound for effects1/3; four Anthalon
IDs do resolve. Exact sound-stop/fade and live client-cache authority remain
open, and this change does not implement that voice continuation.

This is bounded dispatch/data/ordinary actor-position support. Full receipt-local
speech ordering inside a batch already reduced by aa-net, unavailable attachment
world poses, pending scripted skill flight/blink, native physical actor-center
suppliers and housing replacement races remain open. It does not implement
0x153 sphere dialogue: that route additionally needs physical lifecycle/current
bounds authority; a walk capsule, root animation or fabricated zero correction
cannot replace it. Native font, plate geometry/admission and existing screenshot
limits remain as previously documented. Final validation follows.


Final frozen-v4 client validation: `cargo test --locked -p aa-client --bin aa-client
--no-fail-fast` passes **808 tests, zero failures, 91 existing optional ignored**
(116.03 s). This includes the exact release-at-birth replacement regression,
zero-delta resurrection commits, current-scale/shadow re-admission, current-root
range tests and both directions of pure-digit ID collisions. The preceding
7 focused movement and229 in-game tests passed; their earlier source/logs remain
interim evidence rather than substitutes for this full final-source run. Shared
ID boundaries have fresh R4 qualified clearance; actor integration has fresh R3
qualified clearance. All26 final code/test size/SHA256 pins remain unchanged.

The one final all-targets Clippy invocation exits0. Its retained JSON identifies
no primary diagnostic on any changed source/test line; existing warnings and
the old duplicate main/example manifest target warning remain. The final main
build exits0. A copied final executable ran the idle custom login screen at
1280x720 without authenticating: clean self-exit, zero renderer WARN/ERROR or
script errors, FPS236.2/241.4/254.7/246.1 under shared load. Its PNG was inspected.
The capture log check was corrected to exclude the informational zero-errors
line; no application change or capture rerun was required. This is an integrated
startup/render smoke, not live in-world speech or isolated performance evidence.

Executable `target/debug/aa-client-20261025-native-speech-v4.exe` and refreshed
`aa-client-play.exe` share SHA256
`e8b4add28b67e430433945ab98ea75f779458291b28991aed5643d2fbf7392b5`.
The frozen source pins, exact command logs, Clippy summary, main-smoke metadata
and shot hashes are retained in the ignored `chat-bubble-native-20261025` root
evidence directory. Screenshot: `shots/hud/native-speech-20261025/main-v4.png`.
No server, live login, mount feature or dependency/manifest/lock change was made.

A separate read-only follow-up, `data/extract/aa-audio/chat-voice-stop-20261025/`,
passes17 bounded controls: raw stop flag1 reaches FMOD `stop(false)` by default,
with the native immediate override selecting `true`; cancellation does not
Release the sound. This narrows the earlier stop-policy uncertainty, but exact
fade timing, callback registration, deferred-stop and live voice integration
remain open. All150 accepted input/proof files were preserved by that research.


## Family-title refresh lifetime correction (2026-10-25)

Repeated remote `SCUnitState`/`UnitSpawned` snapshots preserve the separately
received per-actor family line. Previously every snapshot erased it. The new
actual-system regression fails before the fix (missing Ann's Son), and now
checks the received line and actual Text2d/plate-root retention after changed
health, name and appellation. Existing removal/recreation, unloaded receipt,
empty owner, locale and world/session reset behavior is retained. The earlier
test that treated an identical clone as replacement now asserts preservation;
its explicit removal/recreation and world-exit checks remain unchanged.

The existing loaded-family observer now retains the same `(UnitKind, model_id)`
source identity used by `Views`, through one shared used helper. It adds no
family-only generation counter or new lifetime service. Actual host source
replacement clears the old line; ordinary refresh does not. Own appearance/
appellation snapshots retain the current player actor and its title. Tests run
the actual family and plate systems, and compare host identity replacement
against the actual scene receipt fold. All eight focused affiliation checks
pass. No live SCFamilyTitle sender or GPU/visual parity is claimed.

Native source and eight retained byte controls are described in
`data/extract/aa-client/nametag-admission-20261025-r1/REFRESH-FAMILY-ADDENDUM.txt`:
existing-object guard391b90a9 exits before InitFromNetUnit; the per-actor family
buffer is separate from numeric membership and appellation snapshot fields.
The native existing-ID guard also exits for changed payload identity: that
control does not prove a universal object-ID-only host generation rule. The
established host source replacement policy is preserved rather than inferring
new native replacement semantics. New RED/GREEN logs and source pins are in
`data/extract/aa-client/family-refresh-20261025/`. Broader name-tag admission,
physical/native actor lifetimes and the server sender remain open.


## Received bubble-effect voice continuation (2026-10-25)

The received `0x154` ID route now invokes its used voice hook after the existing
QUEST/ALARM Lua delivery. It rechecks the loaded actor, then resolves
`voice/bubble_effect_<id>` through the existing `SoundPacks` cache. Original
items 1 and 3 are absent and preserve the previous voice; 969, 925, 923 and 2939
resolve to retained Anthalon events. Inline text has no effect key. An unready
pack cache does not invent a cue or queue a late retry.

The private slot holds one exact `aa_audio::Voice` request. An admitted cache
hit cancels the prior request with `Engine::stop(old, true)` before querying the
speaker's fresh current point. A missing point after admission leaves the slot
empty. Playback uses that point as a snapshot, without following the entity or
using bubble text lifetime as a sound timeout. The slot polls only its current
handle with `Engine::is_playing`; it does not consume the audio plugin's shared
completion list. A completed older handle therefore cannot retire its successor.
Temporary HUD suspension and actor disappearance are not invented sound-stop
signals; unrelated global SoundManager clear/block/cinema routes remain held.

The new native continuation evidence in
`data/extract/aa-audio/chat-voice-runtime-20261025/REPORT.md` closes registration,
matching-handle callbacks and deferred-stop retry. The game stores GetID before
Play and ignores Play's result. Our handle is still an engine request identity,
not proof of native synchronous factory success, audible output, exact FMOD
fade timing or mixer equivalence. Root's scheduled test deliberately uses the
muted engine, while aa-audio's queued-command test checks exact-handle liveness
before and after completion drains without opening a device or processing Play.
This closes the bounded bubble hook, not every user of the native shared voice
manager. Sphere `0x153`, native physical speech-center authority and protected
wire-variant normalization remain open. Final whole-client validation follows
once this wave's source freezes and reviews settle.


### Received lifetime checkpoint validation (2026-10-25)

The family refresh and effect-voice integrations retain their independent
qualified clearances. Family's eight focused checks preserve actual plate/Text2d
identity across same-source refresh and clear on genuine host-source retirement.
Speech's18 checks include the scheduled original Lua consumer and muted-engine
request semantics; audio's exact-handle observer test does not process queued
Play commands or establish native backend/factory success. Reviewed code and
tests remained byte-identical through the final checkpoint.

Final client tests:846 passed,0 failed,91 existing ignored. The one all-target
Clippy invocation and final single-job main build both exited0. Clippy retained
560 unique warnings/0 errors, without a baseline-clean claim. The newly built
muted login-only capture was inspected and self-closed OS0 with engine
WARN0/ERROR0/script0 at236.0-275.5FPS. Screenshot:
`shots/hud/framehold-20261025/main-final-v7.png`. Commands, exact hashes and
limits are in `data/extract/aa-client/framehold-admission-20261025/` alongside
`validation-summary-final-root-v7.json`. Only validation documentation was
appended after review; this does not close native factory/fade/clock parity,
physical sphere geometry, protected wire normalization or the held0x153 route.


## Mounted slave information: source selection refined (2026-10-26)

Original `SHOW_SLAVE_INFO` still has no complete source-backed host adapter.
Static same-client source work in `ship-control-selection-20261026-r1/` closes
several supplier edges without executing the original target or an emulator.
The feature remains held for implementation; no new Runtime RED or UI success
is claimed.

The selector `39345F10` first resolves the current player's `+0x3AF0` key and
requires the resolved object's literal subtype 2. Its fallback resolves the
separate manager `+0x18` key under the same subtype gate. R6 joins the published
player base to the factory return, and corrects the earlier snapshot-copy
shortcut: the `398A1F90` destination is player+8, so its matching displacements
write eight bytes past those selector fields. The separate active writer
`390B3F10` does write the selector's two DWORDs directly.

R7 verifies the received attachment's child receiver and loaded-parent gate.
A missing parent goes to a deferred queue instead of directly replacing those
fields; its callback re-resolves the child. The queue's lifetime/cancellation
contract remains unread. The detach chain locally resets the primary fields
before further calls. Separately, the admitted `SCSlaveBound` path compares its
master with the current character, resolves the bound object and copies its
object key into manager+0x18. Creation ownership, snapshot words, accepted
attachment and that bound fallback remain distinct.

The getters also differ. Metadata and customizing use the shared selector;
speed, turn speed and vehicle info resolve only the primary attachment. Engine
method gates and returned vectors/state supply movement and vehicle fields.
Customizing additionally tests an unnamed global byte mask 0x01 and descriptor
sentinel. The unitId wrapper supplies u32/destination/17/radix16 to the original
formatter import; its external formatting policy remains held. Source-verified
event-name slots are 96 SHOW_SLAVE_INFO, 97 SIEGEWEAPON_BOUND and 98
SIEGEWEAPON_UNBOUND, but the action producing 96 is not yet joined.

The remaining inputs are the deferred/reset/replacement lifetime policy,
accepted snapshot admission, descriptor/database join, actual engine getter
identities and formatter/action contracts. Production getters remain unchanged.
R6/R7 reports, exact static bytes and artifact hashes are under
`data/extract/aa-client/ship-control-selection-20261026-r1/`.

## Mounted-information header receipt adapter (2026-10-26)

`hud/mounted.rs` now supplies the original
`X2SiegeWeapon:GetMountedSiegeWeaponInfo()` header query. It returns the
selected received object's name, English-localized `slaves.name`, mapped
`slave_kind_id`, and lowercase hexadecimal object key without a prefix or
padding. The descriptor and formatter contracts come from the retained R8
source evidence. GameData loads the descriptor rows from the existing read-only
database; object names and keys come from received units.

The receipt-local view is updated before Lua queries in `enchant::dispatch`.
A loaded slave reached through the player's known primary attachment wins;
otherwise the admitted, loaded `SlaveEvent::Bound` object is the fallback.
Binding requires the current character's master ID. A `SlaveCreated` ownership
announcement does not establish that fallback. Missing parents defer attachment
without replacing the previous link; duplicate child/parent requests keep the
first point, and explicit detach cancels that child's queued requests. Removal
retires lookup results, while connection/character retirement clears the view.
Renames use their explicit receipt rather than a duplicate spawn snapshot.

Absent initial player attachment state remains unknown, so the getter returns
nil until explicit state permits selection. Non-character archive fields are
not promoted to player attachment identity. Matching driver detach invalidates
the cached fallback conservatively: the original continuation's additional
manager-validity member is not mapped, so this is not full native lifetime or
object-generation parity. Positive initial archive mapping remains open.

The pre-edit host test compiled and failed on the original hidden window's
bound-object identity assertion. Its first fixture used a controlled
non-character archive ff value on the player; this admission was corrected to
an explicit `UnitDetached` receipt for the final case. The exact original Lua
assertion chunk was preserved. The focused command
`cargo test --locked -p aa-client --jobs 1 --bin aa-client hud::mounted -- --include-ignored --nocapture --test-threads=1`
then passed six cases: five receipt/getter controls and the unchanged original
hidden-header assertions for name, Catapult class, siege-weapon kind and
`9abc` identity. No original getter or consumer was replaced. The original-asset
case is opt-in because it requires the game pak and read-only game database.

The complete mounted window remains unfinished. Normal mode-action generation,
its SHOW event, engine-derived speed/turn speed, vehicle controls, customizing
equipment and initial snapshot/lifetime joins are separate ports. No zero-speed,
empty-equipment or successful-action substitutes were added. This work has no
renderer, screenshot, audio, live-server or original-native execution result.
The playable alias still contains the earlier startup-tested build, not this
metadata change. Frozen sources, pre-edit failure and passing focused evidence
are in `ship-control-selection-20261026-r1/mounted-header-r9/` and
`mounted-header-r10/` under the ignored aa-client extraction directory.

The separate wasm32 browser compile check exited 0 in 23.016s and retained
25 aa-client warnings. One final scoped Clippy (`--bin aa-client --tests`)
exited 0 in 92.047s: 597 warning emissions, 418 unique diagnostic-site records,
zero errors. Eighteen primary emissions are in touched source paths; none
overlap added lines or either new module. This is a location check, not a
warning-baseline comparison or clean-lint claim. Raw JSON, stderr, every site
and source postchecks are retained. No second Clippy or warning cleanup ran.

## Mounted-equipment read-side stopping checkpoint (2026-10-26)

The original GetSlaveCustomizingType and GetSlaveEquipSlotInfo queries now use
the same receipt-local mounted selection as the header. Customizing visibility
requires received feature byte 6, mask 1. Connection configuration survives
character/world changes and retires with its connection. Slot names and allowed
membership come from the read-only slave customization tables; membership and
the actual received item remain independent. The slot query has no feature gate.
Tooltip construction reuses the existing item-detail path after releasing the
HUD borrow, retaining the received grade and appearance metadata.

Initial typed unit equipment and matching EquipmentChanged receipts update all
28 physical slots. SC068 transaction reports do not settle slave equipment.
Their original gear-related notification fires even for failed reports, with
container 1/fc/f2 mapped to the raw slot and other containers mapped to -1.
Native bag-copy, item-lock and cursor effects remain outside this read-side port.

The initial host check failed with two E0277 errors because rusqlite does not
implement FromSql for usize. Reading u32 and then converting the slot index
corrected those errors; the original failure and source freeze are preserved.
The corrected aa-client bin/tests check exited 0 in 43.015s. The already-running
focused mounted test command then exited 0 in 418.140s: 11 passed, zero failed
or ignored. This includes both unchanged original hidden-window consumers and
four new receipt/configuration controls. The equipment consumer used real
database rows, retained item identities above 2^53, and refreshed original rows
after failed and repeated notifications without using their payloads as gear
settlement. The run retained 11 compiler warnings and the duplicate bin/example
manifest warning; it is not a clean-warning claim. All 1,391 frozen code pins
matched after validation.

Work stops here at the user's request. No additional browser check, Clippy,
main executable build, visible-window capture, live-network/server test or
original-native execution was run for this equipment slice. The playable alias
continues to contain the earlier startup-tested revision. Motion retention and
outgoing equipment requests remain unapplied aa-net proposals. A separate
source-derived kind-2 full-item decoder correction for slots 19 through 25 is
also unapplied and untested; typed receipt tests do not establish that wire path.
Normal show/motion/action integration and remaining identity/lifetime joins
are unfinished, so this checkpoint does not establish complete ship control.

Sources, initial compile failure, corrected validation and unapplied proposals
are preserved under the ignored ship-control-selection-20261026-r1 directory,
especially mounted-read-side-r12/validation-r2 and its stopping checkpoint.

## Mount approach reason (2026-10-27)

`X2Mate:MountMate()` always sent reason 1. The original client computes the
reason byte (1..4) in `x2game.dll` `FUN_3909ae90` (decompile line 134731) from
the mate's entity quaternion and the vector from the player to the mate, and
`FUN_3909b070` (line 134812) sends it with the mate's timeline id and seat 1.
For the mate unit it reads position `[unit+0x3be8]->vtable+0x228` and the
quaternion at the same entity's offset 0; for the player it uses
`FUN_390b3850`. The arithmetic is `gx = (w*w + y*y)*2 - 1`,
`gy = (x*y - z*w)*2`, `gz = (w*x + z*y)*2` (the up component), then the fast
reciprocal square root and one Newton step, `dot = dy*gx + dx*gy + dz*gz` and
`lateral = dy*gy - dx*gx` with `d = mate - player`. `dot >= 0.96` is reason 3,
`dot < -0.96` is reason 4, and the band between returns reason 2 when
`lateral >= 0`, otherwise reason 1; that middle return reads no gate. The
model-list lookup `FUN_390c9f40(model, 1|2)` is the circular list at
`[model+0x120]` (node key at +0xc, value at +8); the decompile does not name
that list. Entry 2 forces reason 1 only in the `dot >= 0.96` branch, entry 1
forces 1 only in the `dot < -0.96` branch.

`tools/mount_reason_probe.py` drives the hidden 1.2 client through
AAEmu's client driver (`aaemu-clientdriver`), reads the entity positions and the mate quaternion with
`ReadProcessMemory`, and walks the character to the front, back, left, right
and diagonal sides before each mount. The server's `CSMountMatePacket fields`
trace supplies the reason. It captured 24 cases (3 per requested side) with
reasons 1:10, 2:9, 3:3, 4:2, and the decompiled arithmetic predicts all 24.
The eight constants were read from the running `x2game.dll` and match the
decompile: 2.0, 1.0, 1.17549435e-38, 1.5, 0.5, +0.96, -0.96 and 0.0. The
smallest captured distance to the +/-0.96 thresholds is 0.002124, so the
Python report's `1/sqrt` (instead of `rsqrtss`) cannot change a reason. The
committed run is `tools/evidence/mount_reason/` (native.csv, native.json,
report.txt); the ignored raw output is under
`data/extract/mount-reason-20261027-r1/`.

`hud/pets/mount_reason.rs` is the plain-Rust arithmetic, including the x86
`rsqrtss` instruction on x86/x86_64 and the original's multiplication order;
the CSV test is the captured run. `Pet` now keeps the received roll/pitch/yaw,
and `mount` builds the yaw quaternion and sends the computed reason with the
same timeline id and seat. The received yaw comes from the movement
packet's direction byte (2*pi/128 steps), while the original reads the exact
entity quaternion; half a step (0.0245 rad) moves the dot by about 0.007 near
the +/-0.96 thresholds, enough to flip 1 of the 24 captured rows (dot -0.9621). The Bevy replay (`aa-client --headless --mute`,
a second account) sends 3 from behind, 2 from the right, 1 from the left and 4 from a
separate front run, matching the native values for those actual sides; the
replay scripts and compare are in `tools/evidence/mount_reason/`.

Validation: `cargo test -p aa-client --bin aa-client` passed 859 tests, zero
failed, 93 ignored. `cargo clippy -p aa-client --bin aa-client` exited 0 with
194 generated warnings and zero errors; this is not a clean-warning claim. The
model-list gates are not ported and were not measured: the run never read
the list, but reasons 3 and 4 both occurred for this horse (item 8159), so
neither gate was set for it; other mate models are untested; the two-argument `mount_reason` passes them as false, while
`mount_reason_gated` covers both forced branches with chosen inputs. The two
chosen-input tests for the up term (`w*x + z*y`) and the ungated middle band
join the 24-case CSV test: 4 passed, 0 failed, 948 filtered. Unmount reasons
9/10, the camera, the mount animation and every server file are unchanged.

## Own ship / vehicle on the maps

The original scripts react to `UPDATE_MY_SLAVE_POS_INFO` / `CLEAR_MY_SLAVE_POS_INFO` by calling `X2Map:UpdateMySlaveInfo`,
which draws the `my_slave` icon (`M_MY_SLAVE`, the roadmap sprite cell at 48,216). Here `hud/map_slaves.rs` returns that icon
from `aa_net::Slaves` (`own` timeline + the latest `SCMySlave` report, which the server sends on summon and every 5 s
whatever the distance, so a ship far out at sea keeps its marker) and `markers.rs` appends it with the party and house
icons; the tooltip is the slave's name. It disappears with `SCSlaveRemoved` for the owned timeline.
Live check: log in with the test character, `/slave spawn <slaves.id of a boat>` (or summon a ship from its item),
open the roadmap (top right) and the world map (M): a `my_slave` icon at the ship, updating every 5 s while it sails.

## Action bar pages (2026-10-27)

The original client keeps four action bar pages. Page 1 shows hotbar slots 1..12; pages 2..4 show 49..60,
61..72 and 73..84. Those numbers are the `CSUpdateActionSlot` slot bytes, one based (slot 0 is unused). The
two 6-slot sub bars stay at 13..24. `X2Action:GetActionBarPage` / `SetActionBarPage` count pages from 1;
`SetActionBarPage` returns its argument even when the page is outside 1..4 (the page does not change), and
`ACTION_BAR_PAGE_CHANGED` carries the 1-based page; a repeated call on the current page fires it too (the
probe's fresh widget already held the loop's last event, `before` [4], and the repeated call added one more).
The original default bindings are Shift+Up
(`action_bar_page_prev`) and Shift+Down (`action_bar_page_next`); they wrap 4 -> 1 and 1 -> 4. Shift+wheel
does not page (the wheel is the camera zoom).

Native capture (original r208022, hidden, combat lane): the Lua API returned 1 initially, accepted 1..4,
ignored 0 / -1 / 5 / 99 while still returning the argument, and fired `ACTION_BAR_PAGE_CHANGED` with 1..4.
A second `SetActionBarPage(4)` while page 4 was current added one event: the fresh widget already held
[4] before the call and [4, 4] after it. Shift+Down from page 4 reached 1, Shift+Up from page 1 reached 4,
and Shift+wheel left the page unchanged.
A real drag of the learned skill 18132 from the first to the second cell of each page sent
`CSUpdateActionSlotPacket` bodies with slots 1, 49, 61 and 73 (kind 2, spell 18132); the last cell gave 12,
60, 72 and 84. Pages 3 and 4 were captured, not extrapolated. The clean run's destination column is 8/8;
the page-1 first-cell row also shows the source clear (1:0:0), while pages 2..4 show the source slot set
again in the captured window, so the mapping proof is the destination slot, not a source clear on every
page. Evidence:
`tools/evidence/action_bar_pages/` (`native.csv`, `native.json`, `report.txt`); the probe is
`tools/action_bar_page_probe.py`.

In the Bevy client `aa_net::state::ActionBar` holds `page` (0 based) next to the 85 slots indexed by the
packet slot (slot 0 unused). `ActionBar::display_slot` maps a 0-based display cell to the global slot: the
first 12 follow the page, the sub bars are `display + 1`. `HUD.actions` is keyed by the global slot and
`HUD.action_slot(display)` gives the global slot of a visible cell. `HudData.action_page` carries the page for
`X2Action:GetActionBarPage` / `SetActionBarPage`. Drags, action bar clicks, hotkeys (including a scripted
`slotN`) and `HUD.request("use_action")` all go through the same mapping; the page keys set the page and fire
`ACTION_BAR_PAGE_CHANGED`, and a new world starts on page 1 (both the resource and the bridge page reset).

Replay (Bevy `--headless --mute`, second account): each page was selected through the Lua API and the first
cell filled through `HUD.drag_action_set`; the server field trace showed `CSUpdateActionSlot` slots 1, 49, 61
and 73 with skill 18132. The same replay also asserts the captured page API edges live: a repeated call on
the current page adds one `ACTION_BAR_PAGE_CHANGED` and page 5 is ignored (client exit 0). Evidence:
`tools/evidence/action_bar_pages/replay.txt` and `replay-report.txt`; the runner is
`tools/action_bar_page_replay.py`.

The native probe's clean run is 8/8 drag cases (first and last slots of all four pages, slots 1/2, 11/12,
49/50, 59/60, 61/62, 71/72, 73/74 and 83/84) and 4/4 key cases; the Bevy replay is 4/4 with slots
1/49/61/73 and exit 0. Validation: the scripted-slot mapping test failed with the old extra decrement
(0 passed, 1 failed, 974 filtered; `scripted-slot-prefix-failure.txt`) and the ignored sorting test that
exercises `apply_requests` passes explicitly after it inserts the HudBridge (1 passed, 0 failed); the full
client test binary is 880 passed, 0 failed, 95 ignored, and the aa-net library is 776 passed, 0 failed.
Scoped Clippy exited 0 with 196 generated warnings for the `aa-client` bin and 41 for the aa-net library,
zero errors; this is not a warning-clean claim. Deliberately not changed: the unbound
indexed `action_bar_page` action, the visual page control animation, and the server's slot storage (the
server already stores slots by the 1-based number).
