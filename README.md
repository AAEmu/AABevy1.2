# AAEMU 1.2: an ArcheAge 1.2 client in Bevy

A from-scratch client for **ArcheAge 1.2**, written in Rust on [Bevy](https://bevyengine.org) 0.20. It reads the
original game files (models, animations, terrain, effects, sounds and the original Lua UI) and plays on an
[AAEmu](https://github.com/AAEmu/AAEmu) server. It covers the whole flow: login, world select, character select and
create, the loading screen, and in game. The aim is a client that looks and behaves like the real one and runs fast.

> Work in progress. This is a fan project, not affiliated with XL Games or Kakao Games. No game files are included: you
> need your own ArcheAge 1.2 client and an AAEmu server.

![In game](docs/screenshots/ingame_character.jpg)

## Screenshots

| | |
|---|---|
| ![Login](docs/screenshots/login.jpg) Login screen | ![World select](docs/screenshots/world_select.jpg) World select (original Lua UI) |
| ![Character create](docs/screenshots/create_appearance.jpg) Character create: appearance | ![Skillset](docs/screenshots/create_skillset.jpg) Character create: skillset |
| ![Loading](docs/screenshots/loading.jpg) Loading screen | ![HUD](docs/screenshots/ingame_hud.jpg) In game with the original HUD |
| ![Bag](docs/screenshots/ingame_bag.jpg) Bag and tooltips | ![World map](docs/screenshots/ingame_map.jpg) World map |
| ![Combat](docs/screenshots/combat_arrow.jpg) Combat | ![Skills](docs/screenshots/skills_sorcery.jpg) Sorcery skills, one frame each |
| ![Marianople](docs/screenshots/world_marianople.jpg) Marianople | ![Palace](docs/screenshots/world_palace.jpg) Ezna royal palace |
| ![Golden hour](docs/screenshots/world_golden_hour.jpg) Time of day: golden hour | ![Night](docs/screenshots/world_night.jpg) Night |
| ![Elf start](docs/screenshots/world_elf_start.jpg) Elf starting area | ![Costumes](docs/screenshots/costumes.jpg) Costumes and armour |
| ![Distant objects](docs/screenshots/world_distant.jpg) Two Crowns from 2 km away | ![Sun shafts](docs/screenshots/world_sun_shafts.jpg) Sun shafts through the canopy |
| ![Beach](docs/screenshots/world_beach.jpg) Sea foam and the beach gradient | ![Shallows](docs/screenshots/world_shallows.jpg) Clear shallows with caustics |

Material diagnostic scenes, before and after the native Illum recovery:

| Before | After |
|---|---|
| ![Palace before](docs/screenshots/illum_palace_before.png) Palace model | ![Palace after](docs/screenshots/illum_palace_after.png) Authored vertex lighting |
| ![Armour before](docs/screenshots/illum_armour_before.png) Armour model | ![Armour after](docs/screenshots/illum_armour_after.png) Native gloss and reflection composition |

These compare renderer implementations on the same models; original-client lighting parity remains open.

The original buff-driven mode bar, using the same received piano/score fixtures:

| Before control | After |
|---|---|
| ![Former zero-count getter](docs/screenshots/music_modes_before.png) Former zero-count getter | ![Original mode bar](docs/screenshots/music_modes_after.png) Eight piano notes and Play/Stop/Close |

This is an offline prior-contract control. It does not establish live seating or instrument animation parity.

![Live mode actions](docs/screenshots/music_modes_live.png)

The original Play/Stop/Close buttons in a windowed session on the isolated server, after receiving the score buff.
Placed-instrument seating and animation remain unfinished.

Ground-effect diagnostic, before and after adaptive terrain draping (synthetic rolling ground):

| Before | After |
|---|---|
| ![Ground decal before](docs/screenshots/fx_decal_before.png) Coarse-grid intersections | ![Ground decal after](docs/screenshots/fx_decal_after.png) Interior gaps removed |

Curved decals use more geometry; flat decals keep their previous budget. A small rim cutout and native receiver
projection remain open. Matched costs and limits are in [`docs/aa-fx.md`](docs/aa-fx.md#round-18-bounded-adaptive-ground-draping-2026-10-23).

Native sun-shaft controls and blend order, at the same camera and hour:

| Before | After |
|---|---|
| ![Sun shafts before](docs/screenshots/sunshaft_native_before.png) Broad glow around the sky gap | ![Sun shafts after](docs/screenshots/sunshaft_native_after.png) Recovered depth mask and post-film blend |

The original equations are checked with executable probes and GPU controls. A matched original-client forest
capture is still missing; camera-far conversion and shared-load performance limits are in [`docs/aa-world.md`](docs/aa-world.md).

Vegetation sprite correction at the same camera and hour:

| Before | After |
|---|---|
| ![Bush sprites before](docs/screenshots/vegetation_bush_before.png) Flat olive patches | ![Bush meshes after](docs/screenshots/vegetation_bush_after.png) Leaf detail restored |

Normal-mapped leaf-only vegetation keeps its meshes until the atlas can preserve the original leaf lighting.
Other supported models retain sprites. Observed FPS means were 94.6 before and 109.3 after on the shared machine;
this is a bounded correction, with native sprite-lighting parity still open. See [`docs/aa-world.md`](docs/aa-world.md).

Original composition controls, in the same isolated HUD fixture:

| Before | After |
|---|---|
| ![No composer before](docs/screenshots/music_editor_before.png) Item use did not open an editor | ![Original composer after](docs/screenshots/music_editor_after.png) Proficiency limit, preview and confirmed save cast |

The editor and local preview use the original Lua, MML and DLS assets. Saving uploads staged text only after
the matching received skill fires. Stored scores now persist by world and native note key, appear on rehover,
and feed received play/pause callbacks. Piano and drum overrides follow the received native buff lists;
live settlement and placed-instrument interaction remain open. See [`docs/aa-client-music.md`](docs/aa-client-music.md).
Blank-paper opening now follows the native musical-buff guard, and a complete own-unit snapshot can
restore buff certainty after decode loss without rebuilding the player or stopping playback. The original
buff mode-action bar and doodad seat presentation remain integration work.

Stored-score metadata through the original bag tooltip, before receipt and after rehover:

| Before | After |
|---|---|
| ![Score cache miss](docs/screenshots/stored_score_before.png) Title unavailable | ![Received score](docs/screenshots/stored_score_after.png) Harbor title and Artistry requirement |

The same original scripts run in an isolated received-data fixture. A stationary open item tooltip stays unchanged;
rehover queries the saved score. This is not a live-server settlement capture. See [`docs/aa-client-music.md`](docs/aa-client-music.md).

Secondary chat tabs retain their message frame and history when dragged into a separate window:

| Before | After |
|---|---|
| ![Chat tab before](docs/screenshots/chat_split_before.png) Harbor inside the original window | ![Chat tab after](docs/screenshots/chat_split_after.png) Harbor moved into its own window |

The client also persists tab ownership, filters, colours, input method and locks. The images use an original-script
toolkit fixture; client settings restoration has separate integration checks. See [`docs/aa-client-hud.md`](docs/aa-client-hud.md).

Authored waterfall texture motion, at the same two-second sample:

| Before | After |
|---|---|
| ![Static waterfall](docs/screenshots/waterfall_motion_before.png) Static UVs | ![Moving waterfall](docs/screenshots/waterfall_motion_after.png) Original type-2 motion with anchored opacity |

This original-mesh fixture checks the recovered texture animation. Dual passes, Custom perturbation and vertex
deformation remain open; this is not a matched native scene. See [`docs/aa-cry.md`](docs/aa-cry.md).

Authored Illum alpha and glow on the same original bow:

| Flags disabled control | Authored flags enabled |
|---|---|
| ![Bow control](docs/screenshots/illum_alpha_control.png) Private material data disabled | ![Bow native glow](docs/screenshots/illum_alpha_after.png) Original alpha and ambient glow composition |

Both images use the final renderer and the same camera. This isolates the recovered flags rather than
comparing different builds. The supported set includes 169 authored submaterials, 15 with alpha testing;
unsupported permutations are listed in [`docs/aa-cry.md`](docs/aa-cry.md). The client also checks the
production nested LOD material against the direct material in color, depth and normal prepasses.

The pond surface seen from below, at the same camera:

| Before | After |
|---|---|
| ![Pond before](docs/screenshots/pond_window_before.png) Surface window missing | ![Pond after](docs/screenshots/pond_window_after.png) Banks and sky visible through volume fog |

Authored WaterSurface underside/refraction and volume fog pass 136 pixel controls and five surface-crossing
states. Native near-clip exceptions and separate fog/surface ordering remain open. Shared-load FPS samples and exact
limits are in [`docs/aa-world.md`](docs/aa-world.md).

## What works

**Screens**
- A custom login screen (account, password, server address) with animated art. It remembers the account and server
  but never the password.
- World select, character select (3D stage) and character create (all four 1.2 races, both genders, all three steps,
  the server's name rules). These run in the **original Lua UI**, executed from the game's own bytecode.
- A loading screen with real progress. The world, the shaders and your character are built behind it.

**World**
- Streamed terrain with its detail layers, brushes and buildings, vegetation, roads and decals.
- Water, swimming and diving.
- Sky, sun, moons, stars, fog, and the time of day from the game clock.
- Distant buildings to 2 km and trees at the engine's own view distances (castles and bridges stand on the horizon).
- Vegetation wind: leaves flutter and branches bob, driven by the original materials and vertex colours.
- Post effects: sun shafts from the time of day, and eye adaptation (dark forests brighten, bright skies dim).
- The sea: foam along the shore, a sand-to-turquoise beach gradient, and caustics on the sea bed in the shallows.
- Collision with terrain and objects, and a camera that doesn't go through walls.
- Portals: record a location, open a gate with the portal book, walk through it with a loading screen, and Recall.
- Using doodads (F, the F/G/H/J bar, right click): set your recall point at a Memory Tome.

**Characters**
- All races and both genders, with faces, hair, the customization presets and dyes.
- Hair and skin shaders ported from the original CryEngine shaders.
- Equipment: armour pieces trim each other the way the client does, costumes replace the body, and weapons sit on the
  back and hip. Backpacks, tails and beards are drawn.
- Armour and weapon Illum materials use the original Phong highlights, gloss masks and environment reflections;
  authored vertex colours also shade palace and castle brushes.
- Cloth and hair physics: robes, skirts, cloaks and long hair follow the body.
- Animation blending, locomotion, mounts drawn as units, other players and NPCs.

**Combat and skills**
- All 231 player skills have their animations.
- Particle effects from the game's own effect libraries: projectiles, beams, buff auras and ground areas.
- Area targeting with a ground reticle (Meteor Strike, Missile Rain).
- Movement skills: charges, leaps, blinks, knockbacks, knockdowns, Float.
- Stealth, hit reactions, and floating damage numbers timed to the blow.
- The rules for the hard cases (bone table, beams, knockdown, missing controllers) come from reverse-engineering
  the 1.2 client.

**HUD (original windows on real data)**
- Action bars that grey out skills you can't use, cooldowns, buffs, unit frames and nameplates.
- Bag, bank, equipment and the character window.
- Merchants, repair, enchant, mail, trade, auction house.
- Party and raid, friends, guilds and families.
- Duels (challenge, countdown, result), titles over the name plate, emotes and speech bubbles.
- Quests, the world map and minimap, chat, skills, crafting, achievements, rankings, battlefields, pets.

The supported ocean underside also exposes the refracted surface window:

| Before | After |
|---|---|
| ![Sea underside before](docs/screenshots/ocean_window_before.png) Cone approximation | ![Sea underside after](docs/screenshots/ocean_window_after.png) Source-qualified refraction and signed Fresnel |

Native pixel controls pass; the inherited foam/reflection and colour remain visibly approximate. The source gate,
pass ordering and FFT limits are recorded in [`docs/aa-world.md`](docs/aa-world.md).

**Audio:** FMOD sound banks are decoded and played, including ambience (long beds streamed), zone and combat music with the original's mood mixes, skill sounds, authored 3D pan / pitch / low-pass curves, the five supported authored high-pass roots, doppler and reverb. The legacy CELT sword bank is supported by a pinned static decoder; the full bank survey decodes all 11,517 subsounds. Player-score preview and received performances follow their dedicated player-music volume and master controls independently of BGM/SFX; see the native gain limits in [`docs/aa-client-music.md`](docs/aa-client-music.md).

**Performance:** about 70–95 fps at 1920x1080 in the open world on an RX 9070 XT, and lower in big towns.

This list is what works, not a parity claim: see [`docs/aa-client-parity.md`](docs/aa-client-parity.md) for the unfinished systems and their evidence.

Recovery checkpoint, 2026-10-23: the integrated client suite passes **613 tests with no failures**;
25 focused music checks also pass with optional original-asset/device checks enabled. The nested Illum
caller passes 27 GPU captures and 18 exact image comparisons, including active mesh LOD dissolve.
Housing preview/quote/cancel has a fresh live check; salon physical payment and re-entry pass on an
isolated server with PR42, and appearance data roundtrips all eight race/gender models. Chat commands, coffers,
raid markers, item-instance tooltips, original music composition/preview, persistent chat-tab transfers and native
bound callbacks are integrated. Quietest audio policies and brush LOD pairing have separate native checks.
Server [PR44](https://github.com/NickMesser/AAEmu/pull/44) fixes uncached score requests. Construction settlement,
rollout to the existing bevy server and broader original-client parity remain open; mounts are deferred.
Exact evidence and limits are in the parity ledger.

The latest integration adds the original buff-driven mode action bar, including piano notes
and memorized-score Play/Stop/Close, with native chronological input-mode priority. The client
suite passes625 default checks; nine focused mode checks include original Lua slots, hotkeys,
cooldown/usability refresh, suspended-HUD recovery and reentrant callbacks. See the
[music integration notes](docs/aa-client-music.md) for exact scope and remaining seat presentation work.
Fresh-sheet composition, two plays, Stop and Close pass on an isolated server with the documented
original-script error allowance; [draft server PR45](https://github.com/NickMesser/AAEmu/pull/45)
contains the required callbacks. A fresh normal-entry capture shows the textured character and original HUD
after the loading curtain. Direct-entry scripts can capture incomplete pipelines; the loading notes record
the diagnostic evidence and its limits.

## What's missing

- **Transport:** ships, vehicles and gliders. Own mounts can be summoned, ridden and driven, but mount skills, the
  mounting animation, jumping and swimming on a mount are missing.
- **Housing:** placement previews and original management windows are connected; completed construction, decorating
  and farms still need end-to-end verification and remaining integration.
- **In-game shop, territory, siege and nation:** no client side yet.
- **Doodads:** gathering interactions, phase-model changes, planting previews and coffers are connected. Farming
  settlement and wider live lifecycle checks remain open.
- **Teleports:** the player-placed Navi signs are only unit-tested, and world gate 304 (the White Dress Portrait in
  Haranya) is missing from the server's data.
- **World rendering:**
  - Mesh LOD pairing and mesh/sprite dissolve are implemented. Native sprite lighting remains approximate;
    normal-mapped leaf-only vegetation retains meshes until its atlas preserves leaf lighting.
  - Vegetation wind has no wind areas (the data has four, in the world select scene).
  - Post: no glow buffer or screen-space bounce. Sun shafts use recovered strength, depth and blend controls. The time of day's levels, photo
    filter and colour matrix and the eye adaptation follow the engine.
  - Water: the sea has a sine swell instead of the engine's FFT waves. Waterfall type-2 texture motion is supported;
    dual passes, Custom perturbation and vertex deformation remain open.
- **Characters:** faces are flatter than the real client in character create. Heeled shoes and costumes now lift
  the model by their authored heel height; full character-lighting parity remains open.
- **Skills:** native particle receiver projection, wind/gravity force areas and indirect Target producers remain
  open. Authored compound geometry pieces and direct Target ancestors are supported. Missile Rain's arrow
  rain is missing from the game data itself.
- **Options:** the graphics page applies resolution, screen mode, vsync, gamma, object/vegetation distance, shadows, clouds, effect quality, HDR and anti-aliasing. Distance changes refresh already-loaded object ranges. Texture, terrain detail, character and water quality, depth of field, native global far clipping and key-binding sync remain open.
- **Performance:** big towns (Marianople, about 38 fps).
- **Portability:** it is developed and tested on Windows only.

The full list, with evidence and sizes, is in [`docs/research/remaining.md`](docs/research/remaining.md).

## Quick start

**You need:**
- Windows and a recent Rust toolchain (edition 2024; developed on Rust 1.99).
- An ArcheAge 1.2 client. The client reads its `game_pak` file.
- An AAEmu server for 1.2, and its game database `compact.sqlite3`. The client's own copy of that database is
  encrypted, so it reads the server's.

**Point the client at your files** (these are the defaults):
```
set AA_PAK=D:\AA1.2\game_pak
set AA_DB=D:\aaemu-live\rs-game\Data\compact.sqlite3
```
The native cursors are read from the original `archeage.exe`. Set `AA_EXE` to its path, or leave it unset and the
client uses `bin32\archeage.exe` next to the `game_pak` named by `AA_PAK` (the 1.2 layout). Without either,
cursors fall back to `D:/aa-re/client12/bin/archeage.exe`; a missing file leaves the OS cursor in place.

**Build and run**, from the repository root:
```
cargo build -p aa-client
target\debug\aa-client.exe
```
The dev profile is already optimized. The first build takes a while, because it compiles Bevy.

For an unattended render check with the client's dependency features and no server connection:

```text
cargo run --locked -p aa-client --example world_rendering -- --shot shots/resume/world-rendering.png --shot-after 30 --size 1280x720
```

The [inspected checkpoint](docs/screenshots/world_rendering_checkpoint.png) contains 4,894 object parts,
16 terrain patches and nine authored light ratios. Observed FPS was 87.5–117.9 under shared load;
this is a render/integration check, not a controlled performance comparison.

**Play:**
1. On the login screen, enter your account, your password and the server's login address, for example
   `127.0.0.1:1237` (AAEmu's default). Tab moves between the fields, Enter connects.
2. Pick a world, then a character, or create one.
3. Controls follow the 1.2 defaults:

| Key | Action |
|---|---|
| W / S | Forward / back |
| A / D | Turn |
| Q / E | Strafe |
| Space | Jump, or swim up |
| X | Swim down |
| Mouse | Left drag orbits the camera. Right drag steers. Both buttons run. Wheel zooms. |
| Tab | Next target |
| 1–0, Shift+1–0 | Action bar slots |
| NumLock | Auto-run |
| . | Toggle walking |
| I / C / M | Bag / character / map |

**Handy options:**

| Option | What it does |
|---|---|
| `AA_PRESENT=vsync` | Caps the frame rate to the monitor; it helps over remote desktop |
| `--size 1920x1080` | Window size |
| `--mute` | No sound |
| `--account NAME --enter-world` | Skips the menus: logs in and enters the world with the first character |
| `--shot out.png --shot-after 10` | Takes a screenshot after 10 s and exits |
| `--script run.txt [--headless]` | Drives the client from a line-based test script, optionally without a window ([`docs/aa-client-script.md`](docs/aa-client-script.md)) |

**Logs.** Every run also writes what the console shows (same `RUST_LOG` filter) to
`%LOCALAPPDATA%\AABevy1.2\logs\aa-client-YYYYMMDD-HHMMSS.log`, or to `logs\` next to the executable when
`LOCALAPPDATA` is not set. The path is logged at startup, panics are written there too, and the last 10 runs are
kept. `AA_LOG_FILE=D:\path\player.log` writes to that exact file instead; `AA_LOG_FILE=off` turns the file log off.

## Project layout

| Crate | What it does |
|---|---|
| `aa-pak` | Reads the client's `game_pak` |
| `aa-lua` | Lua 5.1 runtime for the original `.alb` bytecode |
| `aa-ui` | The X2 UI widget toolkit, bound to Lua and rendered in Bevy |
| `aa-cry` | CryEngine formats: DDS, `.mtl`, `.cgf` / `.chr` meshes, materials and shaders |
| `aa-anim` | Animations (`.caf`, `.dba`), character assembly, customization, cloth physics |
| `aa-world` | Terrain, objects, vegetation, water, sky and time of day, streaming, collision |
| `aa-fx` | The particle system for the game's effect libraries |
| `aa-audio` | FMOD (FSB5 / FEV) playback and reverb |
| `aa-net` | Login and game protocol client for AAEmu |
| `aa-dev` | Dev harness: window, fps log, screenshots, frame recording |
| `aa-client` | The application: screens, in-game systems, HUD |

Each crate has notes in `docs/<crate>.md`. Research and reverse-engineering notes are in `docs/research/`.

## License

The code is MIT or Apache-2.0, at your option. ArcheAge and its assets belong to their owners, and none are included
here.
