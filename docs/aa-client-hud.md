# aa-client HUD: the original in-game UI on real data

`crates/aa-client/src/hud/` runs the client's own Lua HUD (`Stage::World`, about 900 scripts under
`scriptsbin/x2ui`) in `Screen::InGame`, on what `aa-net` reports, and draws the one thing the
toolkit does not: name plates over units. The scripts are unchanged. What they ask of the game
(`X2Unit`, `X2Player`, `X2Bag`, `X2Skill`, `X2World`, ...) is answered from the live data.

Status: after `--enter-world` or the full login flow the screen matches `reference/ingame/spawn/real.png`
(player frame, buffs, exp/labor bar, roadmap with the zone header and sub zone, clock, chat, both
action bars, bag/menu buttons, the tutorial window, name plates); 0 script errors.
`shots/hud/spawn.png` is the comparison shot.

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
| `--hud-key NAME T` | a key (lower case): `enter`, `escape`, `backspace`, `delete`, `tab` (target next), `slotN` (action bar slot N, 1 based) |
| `--hud-type TEXT T` | typed text (in Git Bash set `MSYS_NO_PATHCONV=1` before a text starting with `/`) |
| `--hud-open ACTION T` | a key binding action: `toggle_bag`, `toggle_character`, `toggle_spellbook`, `toggle_quest`, `toggle_worldmap` |
| `--hud-click X Y T` | a left click at physical pixel X, Y |
| `--hud-disconnect T` | the disconnect dialog, as if the server dropped the connection |

## What is wired

| feature | source |
|---|---|
| player frame: name, level, HP/MP bars and numbers | `aa_net::Player`, `PlayerStats` (`HP_CHANGED` / `MANA_CHANGED` / `PLAYER_LEVEL...` events on change) |
| exp bar, labor points, loyalty tokens | `PlayerStats`, `Player` |
| buffs of the player and of the target (icon, remaining time, tooltip text) | `aa_net::Buffs` + the DB's buff table; remaining time is pushed as an end time on the HUD clock, so the countdown costs nothing per frame |
| action bars: 2 x 12 slots, skill and item icons, item counts, grade frames, cooldown and global cooldown sweeps | `aa_net::ActionBar`, `Inventory`, `Skills.cooldown_left` / `gcd_left`; the DB for names, icons, descriptions, `custom_gcd` |
| using a slot: keys 1..9, 0, -, = (Shift for the second bar) | `Command::UseSkill` / `UseItem` |
| target frame, Tab targeting (nearest unit within 40 m, cycles), left click on a unit, Escape clears | `aa_net::Target`, `Command::Target`; the pick is a ray-to-unit-centre distance (1.1 m, 6 px slop) and ignores clicks over UI |
| chat: incoming (all channels, notices, system), sending with Enter, edit box focus | `aa_net::Chat`, `Command::Chat`; `HudKeyboardFocus` |
| roadmap: zone name, sub zone, faction, heading, clock | zone from the DB (`WorldInfo` zone group, then position), sub zones from the pak's `subzone_area.xml` polygons (the DB's rectangles are mostly 0), in-game time from `WorldInfo` |
| bag window | `aa_net::Inventory`: items, counts, icons, grade frames, tooltips, "n/50"; the tabs filter by item group (below) |
| max HP / MP of the player and of the target | the DB's unit formulas (below); the server sends none |
| quest marks over NPCs | `aa_net::Quests` + the DB's quest tables (below) |
| character info window | stats from `PlayerStats`, equipment slots from `Inventory` (see below) |
| name plates | `Units` + the DB; see below |
| leave the game, back to the character or server list, the leave countdown | `Command::LeaveWorld`, `Event::LeavePending` / `LeaveCancelled` / `LeftWorld` |
| disconnect dialog | `Event::Disconnected` / `Kicked` not asked for -> `DISCONNECTED_BY_WORLD`; OK logs in again and shows `Screen::WorldSelect` |
| keys I B C K L M, Escape | `UiRuntime::hotkey` / `escape()`; skipped while an edit box has the keyboard |

Anything the game side does not provide yet keeps the fake data of `aa-ui/examples/fake/fake_*.lua`
(logged as `FALLBACK`), so a window never comes up empty because of a missing hook.

### Character window and its model

The real client shows the in-world character between the slot columns: there is no render to do.
`reference/ingame/character_info/camera.json` equals the spawn camera, so the default orbit camera
already frames the player between the columns (`shots/hud/charwin.png`). No camera override is needed.
The equipment slots take their items from `Inventory` (`ES_*` constant = `aa_net::model::slot` index + 1).

### Name plates

`plates.rs` draws them with a second 2D camera (`PlateCamera`, order 5, render layer 30, after the
world and before the UI camera at 10): `Text2d` in the client's English UI font (`flareserif_821_roman`)
plus two sprites for the 69 x 6 bar. Per frame, in `PostUpdate` before transform propagation, up to 40
plates for the nearest units (NPCs 45 m, characters 70 m, the player's own always):

* position: `Camera::world_to_viewport` of the unit's feet + the height of its model + 0.3 m. The height
  is measured from the AABBs of the model's meshes (they arrive part by part while the model loads, so
  the first 40 samples keep the maximum); a character's is capped at 1.85 m (the bounds take in the bow);
* colour by relation: hostile red, neutral yellow, friendly green (`system_faction_relations`; an NPC of the
  player's own faction is yellow, as in the capture; the player's own plate is green);
* NPCs with `show_name_tag = 0` get no plate; names are 11 px for NPCs and 26 px for characters.

The camera copies `Hdr` and `Msaa` from the world camera and is spawned only after the world camera has
`Hdr` (aa-world's post processing adds it a frame later); a frame where the two disagree is a wgpu
validation error ("attachments have differing sample counts"). `cameras.rs` keeps the UI camera in step.

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

### Bag tabs

The tabs are `default_inventory_tabs` / `default_inventory_tab_groups` (served by the fake bag data); an
item belongs to the group of its category (`items.category_id` -> `item_categories.item_group_id`, the
`group` field of a bag slot). The All tab shows the slots as they are, the others the items of their
groups one after the other (`SlotByIdx` in `ingame.lua`, cached per bag change). Tabs the player creates
live in the fake bag data, so they are lost with the stage; the original keeps them in its settings.

### The stage is prebuilt during Loading

Building the Lua stage takes 0.6-2 s. `prebuild_hud` does it a few frames into `Screen::Loading` with a
second `UiRuntime` (`PrebuiltStage`), while the loading screen's own runtime keeps running; `build_hud` then
swaps it in on entering the game (about 20-50 ms: the snapshot is refreshed, the world stage's events fire
again, the renderer is told to redraw everything). It is dropped again on the way back to the world, character
select or create screens. Without a prebuilt stage `build_hud` falls back to building it on entry.

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

## Cost

`aa-dev: fps=` in the spawn scene, debug build, shared machine (noisy, +-5 fps and worse while other agents
build): round 1 measured 63 fps / 16.0 ms with the HUD against 70 fps / 15.0 ms without, about 0.7-1 ms per
frame. Round 2 adds the quest marks (a handful of small models, refreshed every 30 frames) and the formulas
(evaluated when the gear or buffs change): full flow runs show 64-72 fps in game. Entering the game costs
about 20-50 ms instead of 1.5-2 s because the stage is built during Loading (`hud: UI stage ready in
580-1300 ms` while loading; `hud: swapped in the stage built during loading (19 ms)`).

## Not done

* Roadmap markers (quest and NPC icons on the minimap).
* The white beams of the quest effect over an NPC (a particle effect of the NPC, aa-fx).
* The cooldown sweep of aa-ui shows a time label also for the 0.5-1.5 s global cooldown; the original shows
  the dimming only (aa-ui).
* Bag tabs the player creates are not saved; clicking, dragging and using bag items other than through the
  action bar is not routed (aa-ui slot clicks).
* `GetEntityByName` is a stub in aa-ui; `chatEdit` has no `/prefix` handling.
* Quest marks ignore requirements the client cannot check (they hold), so an NPC may show a "!" for a quest
  the server then refuses.

## For the other crates

* ingame: skip key bindings while `HudKeyboardFocus.0` and camera orbit / click-to-move while
  `HudPointerOverUi.0` (done in round 2).
* aa-ui: slot click routing, `GetEntityByName`, `/prefix` in `chatEdit`, no time label on a global cooldown.
