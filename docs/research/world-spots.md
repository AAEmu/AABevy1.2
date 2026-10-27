# World spots: main_world spawns, cell mapping, login2 cameras

Date: 2026-10-16. Read-only research. Pak reads used `tools\aa-pak.exe cat` (the `cargo run` route was blocked by the shared build lock and an unfinished manifest edit in `crates/aa-net`). Sources: 1.2 pak `game_pak` (index `data\pak12.tsv`), server repo `<home>\Documents\GitHub\AAEmu-rust-1.2`, DB `<server>\Data\compact.sqlite3` (opened read-only). Prior RE notes in `<re-notes>\zone\agent_geom\` were used only as a cross-check: they come from ArcheAge 10.0.2.13, not 1.2.

## Summary

- Coordinates are CryEngine world space, Z-up, in metres. A cell is 1024 m and a sector is 64 m (16x16 sectors per cell).
- Cell folder = `game/worlds/main_world/cells/<cx:03>_<cy:03>/`, where `cx = floor(x/1024)` and `cy = floor(y/1024)`. X comes first. Verified against world.xml.
- New characters spawn at four fixed points (one per race, same point for both genders). The points fall in cells `015_015` (Nuian), `010_015` (Elf), `018_007` (Hariharan) and `023_010` (Ferre).
- login2 is a 2x2-cell scene (zones 177, 224, 255). Cameras are in cinema files `01`/`02` (world select), `03`/`04` (character select). The character-create cinemas (`05_*`) have no camera; the create camera is not in any text file I read.
- `login_stage_opt.cfg` (300 bytes) holds only render cvars. It names no world, cinema or time-of-day.

## 1. Coordinate system and units

| Item | Value | Source / status |
|---|---|---|
| Axes | Z up, X and Y horizontal, Z = height | CryEngine convention; the server treats Z as height (Nuian Z 126.484) |
| Handedness | Assumed right-handed | Not stated in data (uncertain) |
| Unit | metres | Server `CELL_SIZE = 1024` m, `REGION_SIZE = 64` m (`rust/crates/aaemu-game/src/data/world_templates.rs`) |
| Cell | 1024 m | world.xml `resolution="1024"`; server constant; prior RE "1 cell = 1024 m" |
| Sector | 64 m, 16x16 per cell | server `SECTORS_PER_CELL = 16`; world.xml sector indices run 0..15 |
| Heightmap unit | 2 m | world.xml `unitSize="2"`; heightmap.dat header word = 2 (interpretation: 2 m per sample) |
| Ocean level | 100 m | world.xml `oceanLevel="100"`; heightmap.dat header float 100.0 |
| Max terrain height | 4096 m | world.xml `maxTerrainHeight="4096"` |
| World extent | x in [0, 35840) m, y in [0, 38912) m | world.xml `cellXCount="35" cellYCount="38"` x 1024 |
| World origin | (0,0) = minimum corner of cell 000_000 | Inferred from the server's cell arithmetic (not stated in a file) |

Client caveat: `Docs/wiki/Client.md` says client entity coordinates are relative to the current seamless map origin, while GM commands use global coordinates. The server's `zone_of(x, y)` treats the spawn coordinates as global (see below), so this report uses global coordinates. Whether the engine needs an offset is unverified.

Yaw: `CharTemplates.json` gives `Yaw` in degrees; the server converts it with `MathF.PI / 180 * v`. The reference axis for yaw is not verified.

## 2. Cell size and naming

Rule (verified):

```
cx = floor(x / 1024),  cy = floor(y / 1024)
folder = game/worlds/main_world/cells/%03d_%03d/  (cx, cy)
local position in cell:  lx = x - 1024*cx,  ly = y - 1024*cy   (0..1024 m)
sector:  sx = floor(x / 64),  sy = floor(y / 64)
  cell  = (sx >> 4, sy >> 4)            (equals cx, cy)
  local sector = (sx & 15, sy & 15)
  global sector index used in world.xml = (cell.x*16 + sector.x, cell.y*16 + sector.y)
zone = world.xml <Zone id> that owns the sector
```

Evidence for the X-then-Y order: all 626 cells listed in world.xml `cellList` have a folder under `(x, y)`. The discriminating cells are (8,3) through (11,3): folders `008_003` to `011_003` exist and `003_008` to `003_011` do not.

Folder inventory:
- `game/worlds/main_world/cells/` has 1205 folders (x in 002..034, y in 000..037).
- world.xml lists 626 cells. All 626 have folders. 400 of them appear in more than one zone's `cellList`, because ownership is per sector.
- 579 folders are not in any zone's `cellList` (sectors there map to zone 0 in the server's `zone_id_at`).
- world.g has 1330 cell entries (35x38), 626 with zone ids.
- world.xml and world.g both list 122 zones.

Worked example: Nuian spawn (15578.042, 15382.122).
- cx = floor(15578.042/1024) = 15, cy = floor(15382.122/1024) = 15, so folder `015_015`.
- Local position: (218.042, 22.122) m.
- Sector: sx = 243, sy = 240, so local sector (3, 0) with offset (26.042, 22.122) m.
- Zone 179 (`w_solzreed_3`).

## 3. New-character spawn points

Source: server file `rust/crates/aaemu-game/app/Data/CharTemplates.json`. It is byte-identical to `<server>\Data\CharTemplates.json` (checked with `cmp`).

The loader (`rust/crates/aaemu-game/src/data/character_templates.rs`, `CharacterData::load`) stores one spawn per race (`Id`), applied to both genders. Template key = `16*gender + race`. The loader ignores the JSON `Name` field and recomputes `zone_id` with `zone_of(x, y)` (the main_world `zone_id_at`). Creation (`rust/crates/aaemu-game/src/managers/character.rs`, around line 306) sets `world_id = default_world_template_id`, `zone_id = sp.zone_id`, and x/y/z/yaw from the template.

| Race (JSON name) | Race id | Template keys (M / F) | DB char ids (M / F) | X | Y | Z | Yaw (deg) | Cell folder | Cell-local (x, y) | Sector (global) -> local | Zone key (name) |
|---|---|---|---|---|---|---|---|---|---|---|---|
| Nuian | 1 | 17 / 33 | 1 / 2 | 15578.042 | 15382.122 | 126.484 | 160 | `015_015` | (218.042, 22.122) | (243,240) -> (3,0) | 179 `w_solzreed_3` |
| Elf | 4 | 20 / 36 | 5 / 6 | 10388.576 | 15982.466 | 367.606 | 45 | `010_015` | (148.576, 622.466) | (162,249) -> (2,9) | 129 `w_gweonid_forest_1` |
| Hariharan | 5 | 21 / 37 | 9 / 10 | 19304.874 | 7646.06 | 212.704 | 210 | `018_007` | (872.874, 478.060) | (301,119) -> (13,7) | 187 `e_rainbow_field_2` |
| Ferre | 6 | 22 / 38 | 15 / 16 | 23881.218 | 10495.782 | 589.7838 | 200 | `023_010` | (329.218, 255.782) | (373,163) -> (5,3) | 184 `e_falcony_plateau_2` |

Roll and pitch are 0 for all four. The four cells are all in the world.xml cell list and have folders.

Race-to-name mapping: the race ids 1, 4, 5 and 6 match the four creatable races in the DB `characters` table (`creatable = 't'`). The names Elf/Hariharan/Ferre for ids 4/5/6 come only from the JSON `Name` field, which the loader ignores. Not verified against the client. Gender 1 = male is assumed.

DB discrepancy (not used at spawn): `compact.sqlite3` table `characters.starting_zone_id` gives Nuian 179, Elf 129, Hariharan 187, Ferre 184. These are `zones.id` values, not zone keys. For example `zones.id = 179` is `e_tiger_spine_mountains_2` (zone_key 258), and `zones.id = 184` is `instance_cuttingwind_deadmine`. The loader stores this as `CharacterTemplate.zone_id`, but I found no reader on the creation path (creation uses `sp.zone_id` from the spawn position).

Zone key mapping check: `zones.zone_key` in the DB matches the world.xml zone `id`: 179 -> `w_solzreed_3` (DB id 125), 129 -> `w_gweonid_forest_1` (DB id 1), 187 -> `e_rainbow_field_2` (DB id 133), 184 -> `e_falcony_plateau_2` (DB id 130).

Not verified: the Z values against the terrain height (heightmap.dat not decoded).

## 4. login2 world (world select / character select / create scene)

From `game/worlds/login2/world.xml` and `world.g`: `name="login2" resolution="1024" unitSize="2" cellXCount="2" cellYCount="2" isInstance="1" oceanLevel="0" maxTerrainHeight="4096"`.

| Zone (world.xml id) | Name | Cell (world.g) | Folder | Time of day (see section 6) |
|---|---|---|---|---|
| 177 | loginbg2 | (0,0), origin (0,0) | `000_000` | zone `177` |
| 224 | loginbg3 | (1,0), origin (1,0) | `001_000` | zone `224` |
| 255 | loginbg4 | (0,1) | `000_001` | zone `255` |
| none | | (1,1) | (no folder) | |

Contents of the login2 folder: terrain, object, vegetation, brush, statobjs, materials, visareas and entities per cell (13 to 16 files each). Also `surface.xml`, `vegetation.xml`, `clouds.xml`, `env.xml`, `lighting.xml`, `time_of_day.xml`, and `cinemas/`. Not analysed beyond the names: vegetation, surface, clouds.

Cinema-only entities (`cinemas/*_cinema_only_objs.xml`):
- `cam_server` (CameraSource, EntityId 137 in file 01 and 138 in file 02), `cam_select` (139, file 03), `cam_04` (140, file 04). All share EntityGuid `4faced2dbab3ddde`.
- `donotreset_fade black_login avi` (AreaTrigger, EntityId 131135, file 00), with an X2UI ScreenFader for 1.5 s fade in and out.
- Node `cloud` (EntityGuid `4105ab9565531d44`) and `server_effectlight01/02`, disabled in the cinemas.
- Videos: `Game/objects/machinima/login/loginbg.avi` (file 00, 0 to 186.7 s), `game/objects/machinima/avi/intro.avi` (file 08, 0 to 85 s).
- Sounds: `wrold_select_whoosh` and `wrold_select_whoosh2`, `character_select_fadein` and `character_select_fadeout`.

## 5. Cameras and cinemas

ParamId meanings (CryEngine Trackview convention, inferred): 0 = FOV (degrees), 1 = position (world metres), 2 = rotation quaternion, 4 = event (Enable/Disable), 6 = camera switch (node name), 7 = sound, 110 = video.

Quaternions are stored as `w,x,y,z` (inferred from `RecordOriginQuat="1,0,0,0"`). Camera forward is assumed to be local +Y. Both are uncertain; see section 7.

| Cinema (file) | Duration | Camera node | Position keys (t: x,y,z) | Rotation keys (t: w,x,y,z) | FOV keys (t: deg) |
|---|---|---|---|---|---|
| `01_world_select_e1.xml` (world select, 1) | 5 s | `cam_server` | t0: 1652.551, 465.72119, 164.7596 (ease in) ; t4: 1655.411, 461.6911, 156.34689 (ease out) | t0: 0.68578714, 0.22380592, -0.10642799, 0.6843099 ; t4: 0.66555727, 0.22211917, 0.42034426, 0.57533234 | t0: 80 ; t3.5: 30 |
| `02_world_select_e2.xml` (world select, 2) | 3 s | `cam_server` | t0: 1652.551, 465.72119, 164.7596 ; t3: 1657.651, 464.98001, 159.0405 | t0: 0.6857872, 0.22380589, -0.106428, 0.68430984 ; t3: 0.68309253, 0.18271467, 0.18271467, 0.68309247 | t0: 80 ; t2.5: 30 |
| `03_character_select_s.xml` (character select, start) | 0.1 s | `cam_select` (static) | 1655.4113, 461.69107, 156.34694 | 0.6655575, 0.22211914, 0.420344, 0.57533222 | 30 (constant) |
| `04_character_select_e.xml` (character select, end) | 2.5 s | `cam_04` | t0: 1655.411, 461.6911, 156.34689 ; t2.5: 1657.651, 464.98001, 159.0405 | t0: 0.66555727, 0.22211917, 0.42034426, 0.57533234 ; t2.5: 0.68309253, 0.18271466, 0.18271466, 0.68309247 | 30 (constant) |
| `08_intro.xml` (intro) | 85 s | `08_intro` (static) | 1655.4113, 461.80765, 155.81134 | 0.6655575, 0.22211914, 0.420344, 0.57533222 | node FOV attribute 60, no keys |

Character create and customize:
- `05_character_create_s_{nuian,elf,hariharan,ferre}_{male,female}.xml` (8 files, 0.1 s): one Scene track with an empty camera switch. No camera, no objects (`05_character_create_s_elf_male_cinema_only_objs.xml` has `NumObjects="0"`). The create camera is therefore not set by any cinema file. It is probably set in code or the Lua UI, but this is unverified (the `.alb` bytecode was out of scope).
- `06_character_customize_s.xml` and `07_character_customize_e.xml` (1 s each): sound only (fadein and fadeout), no camera.

Login and intro:
- `00_login_s.xml` (186.7 s): video `loginbg.avi`, fade entity. The camera switch key names node `01_server_nuian`, which is not defined in the file (dangling reference).

Other cinemas present but not analysed: `09_ci.xml`, `10_opening.xml`, `en_us/`, `fr/`, `de/` variants, and the job/hariharan/elf/nuian/ferre cinemas.

Object placement vs track keys: the cinema-only entity Pos values equal the track keys minus exactly (1024, 0, 0) for `00` (AreaTrigger 629.30078 vs track 1653.3008), `01` and `04` (cam Pos 631.41101 vs 1655.411), and `03` (631.41125 vs 1655.4113). For `02` the offset is approximate: Pos 632.74304, 465.11194, 160.05862 vs track end 1657.651 minus 1024 = 633.651, 464.98001, 159.0405. The track keys are presumably the world-space camera path during playback (inferred).

Zone of the camera: track x = 1652 to 1657 lies in login2 cell (1,0), which is zone 224 (`loginbg3`). Entity Pos values (x about 631) lie in cell (0,0), zone 177 (`loginbg2`).

## 6. Time-of-day zones and env

No cinema sets `TimeOfDayName` (all empty), so the cinemas do not select a time of day.

| File | Zone | TimeOfDay element(s) | Notes |
|---|---|---|---|
| `game/worlds/login2/time_of_day.xml` | (world default) | `Time="6.8343458" TimeStart="0" TimeEnd="24" TimeAnimSpeed="0.001"` | 171 splines |
| `game/worlds/login2/zone/177/time_of_day.xml` | 177 loginbg2 | `Time="12" TimeAnimSpeed="0"` | static noon, 171 splines |
| `game/worlds/login2/zone/224/time_of_day.xml` | 224 loginbg3 | relic_layer 0: `Time="4.9882698"`, speed 0.001 ; relic_layer 1: `Time="23.29554"`, speed 0.001 | weather_layer 0 |
| `game/worlds/login2/zone/255/time_of_day.xml` | 255 loginbg4 | relic_layer 0: `Time="12"`, speed 0.001, sRGB 1 ; relic_layer 1: `Time="12.460519"`, speed 0.001, sRGB 1 | |

Time units are assumed to be hours (`TimeStart=0`, `TimeEnd=24`); not stated in the files.

Env and lighting:
- `zone/177/env.xml`: no skybox; ocean fog 32,112,208, density 0.07.
- `zone/224/env.xml`: skybox `Materials/sky/sky_ancientforest_cbt3`; same fog.
- `zone/255/env.xml`: no skybox; same fog.
- `login2/env.xml`: `Zone_Dominion_Normal` with four `Moon` entries and an `EnvState`.
- `login2/lighting.xml`: SunRotation 260, SunHeight 80, Latitude 40, Algorithm 3, Shadows 1, SunVector (-0.59088475, -0.1041889, -0.79999995).

Which zone's time of day the scene uses is not determined by the cinema files (the camera is in zone 224 and the entities are in zone 177). Uncertain.

## 7. login_stage_opt.cfg

Index: `game/login_stage_opt.cfg`, offset 1181696, size 300. Full content (11 cvar lines, render options only):

```
r_HDRRendering = 3
r_DepthOfField = 1
q_ShaderGeneral = 2
q_ShaderShadow = 3
option_anti_aliasing = 2
e_lod_skin_ratio = 1
e_shadows_max_texture_size=512
r_ShadowJittering=.25
e_custom_build_normalmap = 1
e_custom_build_body_glossmap = 1
e_custom_build_extramaps_fromshaderquality = 1
```

It contains no world, zone, cinema or time-of-day names.

Login UI script load lists (`game/scriptsbin/x2ui/loginstage/*/toc.g`, text) name only UI scripts (`world_select_view.lua`, `character_select_view.lua`, `character_create_view.lua`, `character_create_step1/2/3_view.lua`, and so on). They do not name cinema files. The Lua sources are `.alb` bytecode and were not read, so the stage-to-cinema wiring is inferred from file names only.

## 8. Uncertain or unverified

1. Quaternion order and camera axis: `wxyz` with +Y forward gives plausible views (for example `02` end: forward (-0.866, 0.000, 0.499), pitch +30.0 deg). If the camera looks along -Y, every pitch sign flips. The `xyzw` reading gives a near-vertical view (90 deg) for the same key, which is implausible, so `wxyz` is preferred.
2. Cinema track keys vs entity Pos: a 1024 m X offset (exact for 00, 01, 03, 04; about 1 m for 02). Which one the engine uses at runtime is inferred, not confirmed.
3. Race names for ids 4, 5 and 6 come from the JSON `Name` field only.
4. The DB `starting_zone_id` values do not match the spawn zones (see section 3). The spawn zone comes from `zone_of(x, y)`.
5. Spawn Z vs terrain height is not checked (heightmap.dat not decoded).
6. The heightmap header (`0x18`, `0x90d0d`, `0x1000`, `2`, `0x40`, `0x80`, float 0.0625, float 100.0, ...) contains unitSize 2, max height 4096 and ocean level 100, consistent with world.xml. The grid size and the meaning of the other words are not decoded.
7. The create camera and the stage-to-cinema wiring are not found in the text sources (Lua is bytecode, not read).
8. The prior RE notes are from 10.0.2.13, not 1.2. Used only to cross-check the 1024 m cell and the cell-local coordinates.
9. Yaw reference axis and handedness are not verified.
