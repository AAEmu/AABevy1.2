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
```

Add `--shot shots/hud/NAME.png --shot-after 25` for an unattended screenshot. `--no-hud` skips the
HUD (frame rate comparisons, bare world screenshots).

Scripted input for unattended runs (seconds counted from the moment the HUD is up):

| flag | does |
|---|---|
| `--hud-key NAME T` | a key: `enter`, `escape`, `backspace`, `delete`, `tab` (target next), `slotN` (action bar slot N, 1 based) |
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
| action bars: 2 x 12 slots, skill and item icons, item counts, grade frames, cooldown sweep | `aa_net::ActionBar`, `Inventory`, `Skills.cooldown_left`; the DB for names, icons, descriptions |
| using a slot: keys 1..9, 0, -, = (Shift for the second bar) | `Command::UseSkill` / `UseItem` |
| target frame, Tab targeting (nearest unit within 40 m, cycles), left click on a unit, Escape clears | `aa_net::Target`, `Command::Target`; the pick is a ray-to-unit-centre distance (1.1 m, 6 px slop) and ignores clicks over UI |
| chat: incoming (all channels, notices, system), sending with Enter, edit box focus | `aa_net::Chat`, `Command::Chat`; `HudKeyboardFocus` |
| roadmap: zone name, sub zone, faction, heading, clock | zone from the DB (`WorldInfo` zone group, then position), sub zones from the pak's `subzone_area.xml` polygons (the DB's rectangles are mostly 0), in-game time from `WorldInfo` |
| bag window | `aa_net::Inventory`: items, counts, icons, grade frames, tooltips, "n/50"; tabs other than All are empty |
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

## How it works

* `Screen::InGame` + the player spawned -> `build_hud` switches the shared `UiRuntime` to a fresh Lua
  state (`login::host::switch_stage`): `gameapi::install`, the fallback fakes, `hud/ingame.lua` (the real
  data layer), the world stage's scripts, then `ENTERED_WORLD`, `LEFT_LOADING`, `ENTERED_SUBZONE`, the
  joined chat channels and the tutorial (level 1). `OnExit(Screen::InGame)` forgets the build.
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

`aa-dev: fps=` over 12 two-second samples after the first, spawn scene, debug build, shared machine
(noisy, +-3 fps): 63 fps / 16.0 ms with the HUD, 70 fps / 15.0 ms without; second pair 62 / 16.4 against
67 / 15.7. The HUD costs about 0.7-1 ms per frame (Lua event dispatch, the UI draw and the plates). The
once-off cost is building the stage: about 1.5-2 s on entering (`login: UI stage ready in ...ms`).

## Not done

* NPC grade names (the DB's `npc_grades` is empty: the grade text is a guess); quest "!" markers over NPCs
  and roadmap markers are not fed (the capture shows one over "Maude").
* Bag tabs other than "All" show nothing real; the GCD is not drawn on the action bars.
* Mouse clicks on slots are not routed by aa-ui: using a slot goes through the number keys.
* `GetEntityByName` is a stub in aa-ui; `chatEdit` has no `/prefix` handling.
* Max HP / MP are the highest values aa-net has seen (the server sends no max).
* The 2 s HUD build on enter could be prebuilt during Loading.

## For the other crates

* ingame: skip key bindings while `HudKeyboardFocus.0` (the HUD also clears `ButtonInput` itself while an
  edit box has focus); skip camera orbit / click-to-move while `HudPointerOverUi.0`; `enter.rs` `drive` exits
  on `Disconnected` / `Kicked`, which hides the dialog in `--enter-world` mode; nothing despawns the 3D world
  on `OnExit(Screen::InGame)` (it stays behind the world select UI after a return); after a teleport a Move
  packet is needed for the server to stream the new area.
* aa-net: send max HP / MP; the NPC grade table.
* aa-ui: slot click routing, `GetEntityByName`, `/prefix` in `chatEdit`.
