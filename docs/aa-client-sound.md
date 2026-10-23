# aa-client: sound

`crates/aa-client/src/sound/` wires `aa-audio` into the screens, the Lua UI and the game world. The audio engine is
`aa_audio::AaAudioPlugin` (added in `main.rs`), `sound::SoundPlugin` is the client's part. Research and data:
`docs/research/audio.md`; the placed sounds of the world are `aa-world`'s (`docs/aa-world.md`, "Placed sounds").

## Silence in unattended runs

`--shot`, `--exit-after`, `--mute` and `AA_MUTE=1` leave the engine muted (it opens no device and reads no sound); `--sound`
overrides that. In a muted run the client does not even read the sound database (the `sound-db` thread starts only when
something is audible or `AA_AUDIO_LOG` is set). Dev switches:

| | |
|---|---|
| `--sound` | play in an unattended run (`--exit-after N`) |
| `AA_VOLUME=0.05` | start the master slider at 5 % (quiet checks on a shared machine); the Options window moves it later |
| `AA_AUDIO_LOG=1` | one `audio:` log line per sound started (path and position), music changes, the zone pack, reverb, the placed sound counts and the cost of their evaluation; also reads the sound database in a muted run |
| `AA_NO_SOUND_ENTITIES=1` | no ambient / random / spot / reverb entities from the cells |

## What plays where

| Where | What | How |
|---|---|---|
| world select | `login_stage_music_before_login` = `x2music:screen:login_2013_07` | the screen's default (the script plays nothing of its own) |
| character list | `login_stage_music_character_select` = `login_cbt4_character` | the script's `PlayMusic` (and the same default) |
| character create | `login_stage_music_creator` = `login_creator_oblivion` | the screen's default (the original scripts call it only in the credits window) |
| loading screen | silence (the music fades out in 1 s) | the screen |
| in game | the zone's `soundmood_*` pack: `music_01..08` (the zone music), `combat`, `death`, `low_health`, `falling` ... (the mood sounds) | `music.rs`, `moods.rs`, see below |
| every screen | UI sounds: `X2Sound:PlayUISound(name)` (global pack), widget `SetSounds` sets (`window`, `button`, `global` packs) | `ui.rs` |
| in game | ambient volumes, random sounds, sound spots (reverb is read only) | `aa-world` `objects/sounds.rs`, 3D on the listener |

The login music crossfades (2 s in, 1.5 s out). `--enter-world` skips the login music (the app goes through the screens in a
second).

### The listener

`assign_listener` puts `SpatialListener` on the in-game orbit camera (`ingame::camera::OrbitCamera`), else on the login stage
camera (`login::stage3d::StageCamera`) and takes it off every other entity. World select has no 3D camera.

### UI sounds (`ui.rs`)

`sound::bind(lua)` installs the hooks on the `AAUI` table of every UI Lua state: the login stage (`login/gameapi.rs`
`install`, which the HUD's api install calls first) and the create screens (`login/create/base.rs`). It does nothing before the
plugin has set the process-wide `BINDING` (headless tests, examples).

* `AAUI.PlaySound(name, dup)`: the item `name` of the `global` pack (`SoundPacks::ui`), played 2D on the UI category through
  `Engine::play_with`; returns the voice id (0: the name has no sound); `AAUI.StopSound(id, mode)` stops it (a fade unless
  `mode` is given).
* `AAUI.PlayMusic(name)` / `AAUI.StopMusic()`: the request goes to the music director, which crossfades it. The request is
  dropped when a new screen comes up (its scripts run again).
* `AAUI.WidgetSound(set, event)`: `<set>_<event>` is looked up in the `window` (`bag_show`), `button` (`default_left_up`) and
  `global` packs in that order. The sounds are off while a screen's UI is being built (its scripts show and hide windows) and on
  the loading screen: the gate opens one second after a screen came up.
* `AAUI.OptionValue(id)` / `AAUI.OptionChanged(id, value)`: the sound sliders of the Options window (`OPTION_ITEM_*`:
  35 master, 36 BGM, 37 SFX, 38 use combat sound, 39 cinema). Master goes to `Volumes::master`, BGM to `Volumes::music`, SFX to
  effects, ambience, voice and UI; "use combat sound" switches the combat music; cinema is kept and not used yet. The HUD's
  Options window gets these through the toolkit fakes (`crates/aa-ui/examples/fake/fake_ingame.lua`,
  `fake_options.lua`, which the HUD includes), so `hud/` did not change. The values are not saved to disk.
* `AAUI.IsSoundPlaying(id)` (`X2Sound:IsPlaying`): `Engine::is_playing` on the voice id `PlaySound` returned.

### In game music (`music.rs`, `zone.rs`)

Reversed from x2game (`X2::GameClient::MusicManager`; `Update` is `FUN_39052ea0`, the track list `FUN_39052be0`):

* Which packs: `ZoneData::packs_at(x, y, zone_key, group)` lists the packs of the place under the listener, most specific first:
  the sub zone (the smallest `AreaShape` polygon of the pak's `worlds/main_world/level_design/zone/*/client/subzone_area.xml`
  around the player, 920 outlines, read once on a thread in about 0.5 s) going up its parents until one has
  `sub_zones.sound_pack_id`, then the pack of the zone group (`zones.zone_key` -> `zone_groups.sound_pack_id`).
  `ZonePacks` (resource, refreshed twice a second) holds the list for the music and the moods. At the Nuian spawn that is
  `soundmood_w_solzreed`.
* Which tracks: `SoundPacks::music_tracks(packs)` is `music_01`, `music_02` ... (up to the first number missing) of the first
  pack that has `music_01`; the original does the same (a sub zone pack without music leaves it to the zone group's).
* When: once a second (`POLL`). When the list differs from the one the playing track came from (a zone or sub zone border was
  crossed) a random track of the new list starts at once and `Engine::play_music_event` crossfades with the old one by the
  events' own fades (the zone tracks: 6 s in, 4 s out). A track that played out (`Stats::music_ended` went up) is followed by
  a random other one after 10 to 30 ms (the original's `10 + rand * 20`; the timer slot returns milliseconds, which an earlier
  version of this client took for seconds and paused 20 to 60 s), i.e. at the next look. A track that never sounded in 10 s
  (an event the pak lacks) is replaced. A place without tracks lets the music fade out.
* Combat, death and the other moods are no tracks: see the next section.

### Moods and their sounds (`moods.rs`)

`drive_moods` looks every 0.15 s at the state and hands the active mood names to `Engine::set_moods` (the highest priority is
the whole mix, `soundmoods.xml`):

| mood | trigger |
|---|---|
| `loading` | the loading screen |
| `death` | `PlayerStats.dead` |
| `combat` | `PlayerStats.in_combat` and 8 s after, with the "use combat sound" option |
| `underwater` | the listener is below the surface of a water body |
| `low_health` | alive with health strictly above zero and below a fifth; native `3907b221..3907b266`, f32 constant at `3999dd34` |
| `falling` | `LocalJump.freefall` |
| `driving` | `LocalMount::driving()` |
| `indoor` | the reverb preset under the listener is `x2_hall_1` / `x2_room_1` / `x2_cave_1` at weight 1 |
| `default` | always in game |

Not raised (no known trigger or no data): `gliding`, `flyng`, `moving`, `dancing`, `menu`, `quest_camera`, `siege_period`,
`machinima_default`.

A mood with an item in the zone's pack (`combat`, `death`, `low_health`, `falling`, `gliding`, `quest_camera`, `siege_period`;
the other moods have none in any of the 72 packs) has a sound. The original (`SoundMoodManager::AddSoundMood`,
`FUN_3907ac50`) plays it when the mood is added and fades it for 2 s when the mood is removed; `mood_sounds` does that with
`Engine::play_2d` / `stop(voice, true)`, taking the event from the most specific pack that has the item
(`SoundPacks::mood_event`). The zone music is not stopped for it: the combat mix has the `music` category at 0 and
`music_battle` (the category of `battle_normal`) at full, so the zone track is silent under the battle track and comes back
when the fight is over. The original has no ducking of its own (nothing in x2game touches an FMOD event queue or a category
volume at run time besides the mood table), the moods are the ducking.

Diving (`FUN_39132910`, effect kinds 10, 11, 12): going under the surface plays `sounds/x2physics:foley:dive_in` and starts the
loop `sounds/x2ambience:_zone_loop:underwater`; coming up stops the loop and plays `foley:dive_out`. The original keys it on
the character's swim state, here the camera below the surface is used. The kind 4 effect (`nuian_bodyfall_feedback`, the
landing after a fall) has no known trigger here and is not played.

## Checks (muted engine unless `--sound`)

```
cargo run -p aa-client -- --sound --exit-after 20                      # world select: login_2013_07
cargo run -p aa-client -- --sound --exit-after 25 --goto character-select --account bevycreate
cargo run -p aa-client -- --sound --exit-after 50 --enter-world --account bevycreate --hud-open toggle_bag 24
```
(prefix `AA_AUDIO_LOG=1 AA_VOLUME=0.05`; copy `target/debug/aa-client.exe` aside for repeated runs).

Log of world select, character list, create and the spawn, abridged:

```
audio: music sounds/x2music:screen:login_2013_07 (screen WorldSelect, fade in 2000 ms)
audio: ui sound login_stage_world_select -> sounds/x2interface:login_stage:button_server
audio: music sounds/x2music:screen:login_cbt4_character (screen CharacterSelect, fade in 2000 ms)
audio: music sounds/x2music:screen:login_creator_oblivion (screen CharacterCreate, fade in 2000 ms)
audio: zone sound pack None -> Some("soundmood_w_solzreed")
audio: music sounds/x2music:group_001:03 (screen InGame, fade in 3000 ms)
audio: ambient start Sounds/x2ambience:world2_object:pond_big at (15579.6, 15385.8, 128.0) volume=1.00 daylight=0.33
audio: widget sound window/bag_show -> sounds/x2interface:bag:show
audio: placed sound evaluation, 100 ticks of 0.1 s: mean 42 us, worst 121 us
```

At the exact spawn of `bevycreate` (15578.0, 15382.1, 126.2), once the cells around it are in: 50 ambient volumes loaded, **3
playing** (`pond_big`, `field_day2`, `forest_night2`; the two beach `sea06` / `sea_night2` volumes are 40 to 60 m outside their
area and 40 m radius, silent), 66 random volumes loaded, **7 inside** their area (the first voice of each comes `MinWaitTime`
10 to 30 s after entering: `sparrow01`, `cricket_random`, `bird_random`, `wind_field`, `bird01`), 10 sound spots loaded and
0 within 100 m, 19 reverb volumes loaded, the strongest `x2_outdoor_1` at weight 1.0 (read, not applied).

Frame cost: `update_sounds` runs every 0.1 s (mean 40 to 60 us, worst 120 us, one 1.6 ms tick while the cells were coming in), so
about 0.5 us per frame on the main thread; the music director and the option sync are a few atomic / mutex reads per frame. The
frame time of the in-game run is the same with and without (13 ms, the renderer).

## Not done

* The create screens' ambience (`worlds/login2` cells 000_001 and 001_000: 8 ambient volumes and 4 reverb volumes each, the
  `login_stage:*` winds): the create scenery (`login/create/scenery.rs`, not mine) loads `login2` itself; its owner calls
  `aa_world::objects::sounds::spawn_cell(&mut commands, cell, &objects.sounds)` (`objects` is the `CellObjects` it already has)
  and despawns the returned entities with the scenery. The character list's stage has no placed sounds.
* The moods listed above as not raised; the body fall effect; an ambient volume with no area is played as a point of its
  `OuterRadius`; the volume sliders are not persisted.
* Zone changes and the combat crossfade follow the data but were not walked through end to end (no scripted movement); the
  pack lookup, the music steps and the mood selection have unit tests (`sound::zone`, `sound::music`, `sound::moods`).

## What `aa-audio` gave for this

* `Engine::music_playing()` and `Stats::music_ended` (the music director follows the engine instead of keeping a clock),
  `Engine::play_music_event` (the events' own fades), `Engine::is_playing(voice)` (for `X2Sound:IsPlaying`).
* Event lookups parse the FEV project on first use (a few ms).

Area reverb is applied through `sound/reverb.rs` and `Engine::set_reverb`; `sound/moods.rs` applies the mixes and category
low-pass settings of the moods (above). Open: the volume-slider persistence and the moods without a trigger.

## Player-score route recovery (2026-10-23)

Composition preview now uses `Category::UserMusic`, with the original independent
option 40 MIDI slider and option 41 other-player mute preference. Saved options 40/41
are applied when each UI stage starts, before the Options page opens; Reset restores
1/0. Own preview is exempt from the other-player preference. Received performance
consumers remain pending.

The route bypasses BGM/SFX and broad category mood LPF, while retaining the existing
engine's once-master policy. Native pre-device gain is recovered; downstream FMOD
master/parent gain, nonunity source/global scales and exact synthesis remain outside
this claim. See `aa-client-music.md` and `aa-audio.md` for source evidence and checks.


## Native weapon-impact parameters (2026-10-23)

Received `UnitDamage` now retains its exact holdable byte and packed hit word. The app carries
that complete receipt through its existing impact scheduler. Attack material comes from the
received holdable, while the current mainhand at impact independently controls `twohand`
through `holdables.slot_type_id == 16` (including staves). Blocks use target offhand material;
parries use mainhand material. Ordinary targets retain the model-material path.

The selector admits only native hit types 1/3/4/6/7/9/10/11/17/20. It writes `pierce`, `miss`,
`critical` and `twohand` before submitting the Sound entity. Critical means types 3/11; miss
means 4/10/14 (the selector still rejects spell 14). Pierce is packed high nibble 1 -> 1.0,
2 -> float32 0.3, otherwise zero. A miss, zero damage or projectile delay does not suppress
weapon audio through the unrelated hurt-FX decision. A missing cast does suppress this path.
Existing event gains, unit caps, reach, bone placement and audio APIs are preserved.

The original selector/producer oracle reexecutes 336 parameter/material, 20 packed-field and
nine timing cases from the pinned x2game DLL. The app matches all 336 selector/parameter
fixtures through the real read-only catalog. A scheduler regression retains two different
contexts through a delayed impact and rejects the unknown-cast control; ECS submission
observes all four initial parameters on the Sound entity. The actual sword fixture
(item 5660 / holdable 3 / target model 1035 / hit 3) selects sound 137, its authored critical
CELT layer and bank055. A CPU-only root test decodes all 116224 frames at 44100 Hz mono;
normal, miss and one-handed controls do not produce the same audible critical layer.

This does not establish live critical-hit delivery, native device output, native per-impact
gain or exact animation timing. Scheduling still uses the existing fired + delay/sync bridge;
the native radius/actor callback and plot/projectile queues remain unfinished. Rendered chest
armor material selection also remains open. The current server emits a zero pierce selector.
No new audio backend, embedded bank or event-ID special case was introduced.

Reproducer: `python tools/combat_sound_producer_oracle.py` (existing original DLL read in place).
Retained corpus/provenance: `tools/evidence/combat_sound/`. Focused app commands:
`cargo test -p aa-client --bin aa-client weapon_sound` and
`cargo test -p aa-client --bin aa-client ingame::sounds:: -- --include-ignored`.


Final consistent-source verification: 631 default client checks pass (61 optional ignores),
all 13 selected sound checks pass with original assets enabled, and the scheduled-impact
regression passes. The single scoped Clippy run completes with existing warnings and none
in the new weapon module; the main executable rebuild passes. Independent read-only review
finds no P1/P2 issue in this bounded diff. `tools/evidence/combat_sound/validation.json` pins
source/binary hashes and exact commands/logs. No new FPS or live combat-audio claim applies.
