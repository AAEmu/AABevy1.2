# ArcheAge 1.2 audio: where and how the game triggers sounds

Research note (read-only). Sources:
- Pak: `tools/aa-pak.exe` over `<client>\game_pak` (index `data/pak12.tsv`). Sound banks: `game/sounds/` (8168 `.fsb`, 69 `.fev` entries; the voice projects repeat per language `en_us` / `fr` / `de`).
- Game DB, read only: `<server>\Data\compact.sqlite3`.
- UI scripts: the decompiled copy `data/extract/aa-lua/decompiled/` (another agent's extract, used for call sites), cross-checked against a Lua 5.1 bytecode scan of `data/extract/game/scriptsbin/**/*.alb` (own scanner, 961 files). Both agree on the call counts within a few percent.
- Cell `entities.xml` (1294 files, 1181 non-empty), `*.animevents` (771 files), `game/libs/particles`, `game/libs/materialeffects`, `game/libs/soundmoods`.

"Resolved" below means: the project FEV exists in the pak and the group and event names appear as null-terminated strings in it. This is a string check, not an FEV parse; the FEV decoder should confirm the numbers.

Scratch scripts were in `%TEMP%\aud` (not committed).

## Summary

| Source | Where the data is | Path / value format | Count | First-integration priority |
|---|---|---|---|---|
| 1. Sound table | DB `sounds` (+ `sound_packs`, `sound_pack_items`) | `sounds/<proj>:<group>:<event>` | 4934 rows, 4868 distinct paths; 476 packs, 13664 pack items (7211 with a sound) | P1 (every other source resolves to it) |
| 1b. Skill / buff / item / doodad / quest links | DB `fx_items`, `fx_voices`, `skills.fx_group_id`, `items.*_sound_id`, `doodad_funcs.sound_id`, `quest_components`, ... | sound id or pack item name | 422 FxSound, 143 FxVoice, 20k item sounds | P2 (items, doodads) / P3 (combat FX) |
| 2. Placed sounds | `game/worlds/*/cells/*/client/entities.xml` | `soundName="Sounds/<proj>:<group>:<event>"` | AmbientVolume 1362, RandomSoundVolume 1340, SoundEventSpot 486, ReverbVolume 587 | P2 (spawn area: 9 cells, 53 entities) |
| 3. Animation sounds | `game/animations/**/*.animevents` (771 files) | `sound` = event path, `sound_pack` = pack item name, `footstep` = fxlib name | sound 7444, sound_pack 12892, footstep 1458 | P1 for footsteps, P2 for the rest |
| 3b. Footstep surface map | `game/libs/materialeffects/materialeffects.xml`, `fxlibs/footstep*.xml` | `.mtl SurfaceType` x fxlib row -> Effect -> Sound | 38 surface columns, 37 fxlibs, 69 distinct sound names | P1 |
| 4. Particle sounds | `game/libs/particles/*.xml` (43 files), `x2_sounds.xml` | `Sound="Sounds/..."` + `SoundFXParam` curve | 1491 `Sound=` attributes, 632 distinct paths | P2 |
| 5. UI sounds | Lua in `x2ui/**` | `PlayUISound(pack item)`, `PlayMusic(pack item)`, `SetSounds(set name)` | 73 + 6 + 86 call sites | P1 |
| 6. Music | DB `soundmood_*` packs, `game/libs/soundmoods/soundmoods.xml`, login Lua | mood key -> `music_01..08`, `combat`, `death`, ... | 72 zone music packs x 17 mood keys | P1 (login music + zone music) |
| 7. Name resolution | all of the above | see section 7 | 4868 paths: 4764 resolved | - |

Also in the pak: `game/sounds/audiopreloads_32.xml` (275 banks: 13 global, 11 level sections, 116 cinematic sequences) and `audiopreloads_64.xml` (620 banks). These list the FSB banks to load per level and are the first thing a player needs (see section 7.6).

---

## 1. Database

### 1.1 Every table and column that matches sound|sfx|bgm|music|voice|ambien|audio

Non-zero means not NULL, not 0 and not empty. Columns with no value at all are listed as unused.

| table.column | rows | non-zero | meaning / sample |
|---|---|---|---|
| `sounds` (id, name, path, end_method_id, level_id, COMMENT, category_id) | 4934 | 4868 paths | the event table: `path` is the FMOD event path (section 7). `name` is a label, e.g. `x2skills/wild:fire_arrow_form` for path `sounds/x2skills:wild:fire_arrow_form`. 66 empty paths; 2 are Korean placeholders with no colon (`제작중입니다`, `제작예정`). |
| `sound_packs` (id, name, COMMENT, category_id) | 476 | - | named groups of pack items. COMMENT `BGM` on 76; 72 are `soundmood_*` zone packs (section 6). Others: `nuian_male` (41), `ferre_female` (48), `global` (203), `window` (204), `button` (205), `please_remove` (410, a copy of `global`). |
| `sound_pack_items` (id, sound_pack_id, name, sound_id, COMMENT) | 13664 | 7211 | name -> sound. Name classes: `Sounds/...` paths used as names 3295, `npc_chat_bubble_<n>` 2405, `sv_*` skill voices 1998, `mon_*` 965, `delete*` 950, other 4051. |
| `fx_items.fx_detail_type` | 2856 | FxParticle 2253, FxSound 422, FxVoice 143, others 38 | effect detail rows of skills, buffs, doodads. |
| `fx_items.asset_name` for FxSound | 422 | 418 are paths | the sound path itself, e.g. `sounds/x2eventsounds:interaction:unpack_form`. |
| `fx_sounds` (id, sound_id) | 422 | 2 | almost unused: the path lives in `fx_items.asset_name`. |
| `fx_voices` (id, sound_pack_item_name) | 143 | 143 | pack item names, e.g. `sv_fight_triple_slash_1_launch`, `sv_magic_meteor_form`. Each resolves in the eight race / gender voice packs (ids 41 to 48, 143 items each). `fx_items.fx_detail_id` -> `fx_voices.id`. |
| `fx_particles` (id, sound_id, sound_pack_id, in_water, scale) | 2253 | sound_id 888, sound_pack_id 35 | particle details with a sound, e.g. id 1002 -> 417 `sounds/x2skills:illusion:ground_restriction_hit`. |
| `combat_sounds` (id, source_sound_material_id, target_sound_material_id, sound_id, fx_group_id) | 392 | sound_id 392 | hit sounds by weapon (source 15 to 28) and surface (target 1 to 28), see 1.4. |
| `holdables.sound_material_id` / `wearable_kinds.sound_material_id` | 27 / 5 | all | weapon / armour material used as the combat source id. |
| `models.sound_material_id` | 1491 | 1477 | the model's hit material. Most common: 10 (606 models), 7 (415), 8 (301). |
| `models.sound_pack_id` | 1491 | 653 | the unit's sound pack, e.g. model 10 `nuian_male` -> pack 41. Animation `sound_pack` events resolve here (section 3.3). |
| `npcs.sound_pack_id` | 13382 | 2081 (85 packs) | NPC voice / interaction packs, e.g. pack 163 `창고 관리인_다루` (warehouse keeper). |
| `characters.default_system_voice_sound_pack_id` | 13 | 13 | system message voice packs 86 to 98 (`nuian_male_system_message` etc.). |
| `characters.default_fx_voice_sound_pack_id` | 13 | 0 | unused: the skill voice packs are reached through the model (above). |
| `items.pickup_sound_id` / `use_or_equipment_sound_id` | 21482 | 20322 / 20177 | id -> sound. E.g. item 10579 pickup `sounds/x2item:paper:pickup`, use `sounds/x2item:paper:use`. |
| `item_categories.pickup_sound_id` / `use_or_equipment_sound_id` | 136 | 136 / 136 | category defaults, e.g. category 100 pickup 201, use 328. The precedence item vs category is not checked. |
| `doodad_funcs.sound_id` | 13974 | 190 (38 distinct) | e.g. `DoodadFuncUse` 683 -> `sounds/x2eventsounds:doodad:flare`. |
| `doodad_func_groups.sound_id` / `sound_time` | 20077 | 485 / 47 | sound per doodad phase; `sound_time` (e.g. 6000) is the delay or length, unit not checked (probably ms). |
| `doodad_func_puzzle_outs.sound_id` | 120 | 40 | puzzle outcome sounds. |
| `quest_components.sound_id` / `summary_voice_id` | 17851 | 17 (11 distinct) / 203 | quest step sound; summary voice e.g. `sounds/x2voice_quest_summary:farmer_ma:4866_4867_ready`. |
| `quest_chat_bubbles.sound_id` | 13268 | 35 | e.g. 795 -> `sounds/x2monster_c:mob:cat_attack_quest`. |
| `bubble_chats.sound_id` / `voice_id` | 72 | 6 / 0 | e.g. sound 195 -> `sounds/x2eventsounds:quest:deep_breath_01`. |
| `sphere_chat_bubbles.sound_id` | 1022 | 2 | rare. |
| `sphere_sounds` (id, sound_id, broadcast) | 5 | 5 | `broadcast` is `f` / `t` (meaning not checked). |
| `npc_chat_bubbles` (id, bubble, ai_event_id) | 2475 | no sound column | the voice comes from pack 370 (section 1.5). |
| `vehicle_models.sound_id` | 63 | 5 | e.g. 748 -> `sounds/x2vehicles:tank:run`. |
| `zone_groups.sound_id` / `sound_pack_id` | 78 | 78 (both 1 or 228) / 72 | `sound_id` is a placeholder (`사용안함_선택하지 말것`, "not used, do not select") on every row. The zone music is `sound_pack_id`. |
| `sub_zones.sound_id` / `sound_pack_id` | 1097 | 0 / 235 (23 packs) | sub-zone music packs. `sound_id` never set. |
| `instrument_sounds` (id, item_id, midi) | 199 | 199 | instrument items; `midi` = GM program number (73 on 63 rows, probably flute; 29 = overdriven guitar; inferred). |
| `equip_pack_weapons.musical_id` | 533 | 45 | instrument ids; 21 rows join `instrument_sounds.item_id`. |
| `music_note_limits` (step, note_length) | 11 | - | step 0 to 10, note length 200 to 1000 (units not checked). |

Not sound: `ai_events`, `area_event_check_npcs`, and the `indun_event_*` tables match only the word "event".

### 1.2 Skills, buffs, projectiles: FX group chain

`skills.fx_group_id` (15126 skills) -> `fx_group_fx_items` (fx_group_id, fx_item_id) -> `fx_items` with `fx_detail_type` FxSound or FxVoice.

* 2866 skills reach an FxSound and 1625 reach an FxVoice.
* Example: skill 25164 `에아나드의 관악기 봉인 해제` -> group 1309 -> item 1786 FxSound `sounds/x2eventsounds:interaction:unpack_form` (moment 0).
* Example: skill 12104 (`위기속으로`) -> group 202 -> item 2448 FxVoice `fight_frenzy_launch` -> `fx_voices.sound_pack_item_name`.
* Buffs (`buffs.fx_group_id`, 9816 rows): buff 2409 `창고지기 쇼블의 석화` -> group 658 -> `sounds/x2skills:monster_skill:lithification_launch`.
* Projectiles (`projectiles.fx_group_id`): 73 of 495 projectiles reach an FxSound.
* The moment (`fx_event_start_id` / `fx_event_end_id`): 0 START, 1 FORM, 2 THROW, 3 LAUNCH, 4 HIT, 5 END, 6 PROJ_END (from `docs/aa-client-combat.md`). The combat code currently draws only FxParticle and leaves FxSound / FxVoice out (that doc, lines 33 to 36 and 153).

### 1.3 Combat hit sounds (combat_sounds)

* 392 rows, 14 source ids (15 to 28), 28 target ids (1 to 28), 16 fx groups (`skill_blood` 1010 with 39 rows, `skill_earth` 1141 with 24, `skill_fire` 1142, ...). 247 rows have fx_group_id 0 or NULL.
* Source = the holdable's material: 15 shield, 16 sword, 17 axe, 18 mace, 21 fist (knuckle), 22 spear, 23 staff, 24 bow, 25 dagger (names from `holdables`).
* Target = the struck material. Suffixes of the sound paths show the surface: 1 flesh, 5 water, 6 fire, 7 wood, 8 stone, 9 wind, 10 flesh (606 models use 10), 13 metal, 14 bone (inferred from the suffixes, not from an enum).
* Example: `(source 16, target 1) -> sounds/x2weapons_sword:sword:sword_flesh`; `(17, 1) -> sounds/x2weapons_axe:axe:axe_flesh`; `(15, 1) -> sounds/x2weapons:x2weapons:shield_flesh`.

### 1.4 Chat bubble and quest voices

* NPC chat bubbles (`npc_chat_bubbles`, 2475 rows) have no sound column. Their voices sit in pack 370 `voice` (`채팅 버블 음성 모음`, "chat bubble voice collection"): 2405 items named `npc_chat_bubble_<npc_chat_bubbles.id>`, e.g. `npc_chat_bubble_5051` -> `sounds/x2voice_ai:watcher_ma_aievent:005`. 2264 of the 2405 names match an existing bubble id.
* Quest summary voices: `sounds/x2voice_quest_summary:<npc>:<id>_<id>_ready` (238 paths, FEVs present).

---

## 2. World placement (cell entities)

Source: `game/worlds/main_world/cells/XXX_YYY/client/entities.xml` (1205 cells; 345 of them have any entity, the rest are empty `<Objects />`) and `game/worlds/login2/cells/*` (3 cells). Each entity has `EntityClass`, `Name`, `Pos`, `Layer` and `Properties` (XML attributes).

### 2.1 Sound-related entity classes (world-wide, all 1294 files)

| EntityClass | entities | cells | sound property | other properties (examples) |
|---|---|---|---|---|
| AmbientVolume | 1362 | 322 main-world cells | `soundName` (looping) | `OuterRadius` 3 to 50; `bEnabled`; `bIgnoreCulling`; `bIgnoreObstruction`; `bSensitiveToBattle`; `bLogBattleValue`; entity attr `OutdoorOnly` (855); `Rotate`, `Scale` (rare) |
| RandomSoundVolume | 1340 | 301 cells | `soundName` (one-shot, random) | `DiscRadius` 10 to 30; `MinWaitTime` 3 to 30 (seen: 3, 5, 10, 15, 20, 30); `MaxWaitTime` 20 to 60; `bRandomPosition` 1; `OutdoorOnly` (1032) |
| SoundEventSpot | 486 | 61 cells | `soundName` (`PlaySoundEvent`) | `bOnce` 0; `bPlay` 1; `Rotate`, `Scale` |
| ReverbVolume | 587 | 318 cells | `reverbpresetReverbPreset` | `Environment` 1; `bFullEffectWhenInside`; `OuterRadius` |
| MusicThemeSelector | 1 (cell 011_012) | - | `sTheme`, `sMood`, `sDefaultMood` (all empty there) | `bForceChange`, `bEndOnExit`, `bKeepMood` |
| AreaShape | 1100 | - | - | polygon `Points`, `Height` (50, 400 seen), `<Entities>` listing the sound entities it owns (3198 of 3725 unique sound GUIDs are referenced by an AreaShape), `<TimeOfDay>` (about 40 areas have a time range, the rest are empty) |

Not sound: `Birds` (161), `Fish` (421), `Chickens` (108) have no sound property; their sounds come from the animal models.

`soundName` values: 194 distinct, 3204 uses. Folders (`Sounds/<proj>:<group>`):
* AmbientVolume: `x2ambience:world2_day` 671, `world2_night` 468, `world2_object` 128, `login_stage` 32, `instance_nachashgar` 20.
* RandomSoundVolume: `x2ambience:world2_random` 851 (seagull, cricket), `world2_animal` 451 (sparrow01), `instance_cuttingwind_deadmine` 26.
* SoundEventSpot: `x2eventsounds:doodad3_instance:dungeon_steam_2`, `x2eventsounds:prefab:origin_tower_active`; `world2_object` 445.
* Resolved: 3195 of 3204 uses. Bad: 6 empty values, one doubled string `Sounds/x2ambience:world2_night:sea1_nightSounds/...`, and 2 uses of a project named `environment`.

Reverb presets used (8 distinct): `x2_outdoor_1` 492, `x2_cave_1` 66, `x2_hall_1` 24, `x2_outdoor_3_forest` 4, `x2_room_1` 1, empty 5. Two rows and one more hold a sound path in this field (data slip). Preset names exist in `game/libs/reverbpresets/reverbpresets.xml` (49 presets).

### 2.2 Around the Nuian start (cells 014_014 to 016_016)

Totals over the 9 cells: AmbientVolume 24, RandomSoundVolume 20, ReverbVolume 9, SoundEventSpot 0, Birds 24, Fish 22, Chickens 1, AnimObject 11, ParticleEffect 40.

| cell | AmbientVolume | RandomSoundVolume | ReverbVolume |
|---|---|---|---|
| 014_014 | 5 | 7 | 1 |
| 014_015 | 2 | 2 | 1 |
| 014_016 | 0 (empty file) | 0 | 0 |
| 015_014 | 3 | 3 | 1 |
| **015_015** (spawn, Nuian spawn pond) | 3 | 2 | 1 |
| 015_016 | 2 | 1 | 1 |
| 016_014 | 5 | 4 | 2 |
| 016_015 | 2 | 0 | 1 |
| 016_016 | 2 | 1 | 1 |

Spawn cell 015_015, layer `_sound_solzreed`, in full:
* AmbientVolume `solz_AmbientVolume_sea06` `Sounds/x2ambience:world2_day:beach_near`, OuterRadius 40, `OutdoorOnly`.
* AmbientVolume `solz_AmbientVolume_sea_night2` `Sounds/x2ambience:world2_night:beach_night2`.
* AmbientVolume `solz_AmbientVolume_pond_1` `Sounds/x2ambience:world2_object:pond_big`, OuterRadius 15.
* RandomSoundVolume `solz_RandomSoundVolume_seagull_random_1` and `_2` `Sounds/x2ambience:world2_random:seagull`, DiscRadius 30, MinWaitTime 10 / 15, MaxWaitTime 20 / 30, `bRandomPosition` 1.
* ReverbVolume `solz_ReverbVolume_24` `x2_outdoor_1`, OuterRadius 40.
* AreaShape `solz_sound_shape_pond_1` (Height 50, 13 points) and `solz_sound_shape_sea06` (Height 400), each listing the entities it owns by GUID.

The cell's zones are `w_solzreed_1` (142), `w_solzreed_3` (179), `s_silent_sea_7` (180) and `s_silent_sea_1` (210) (`world.g`).

### 2.3 Login world (`worlds/login2`, 2x2 cells)

* 000_000: no sound entities.
* 000_001: AmbientVolume 8, ReverbVolume 4, AreaShape 4.
* 001_000: AmbientVolume 8, ReverbVolume 4, AreaShape 5.
* Layers `_sound_login4`. Sounds: `Sounds/x2ambience:login_stage:wind_ghost_town`, `...:mar03`, `...:mountain_top1`, `...:wind_soft_03`. One ReverbVolume holds `Sounds/x2ambience:login_stage:wind_soft_03` in its preset field (data slip).
* Cell 001_001 is not in the pak.

The character create layers `create_nuian.lyr`, `create_elf.lyr`, `create_ferre.lyr`, `create_hariharan.lyr` contain no sound entities (only lights, cameras and one object). Login music is therefore Lua-only (section 6.1).

---

## 3. Animation sounds (`.animevents`)

### 3.1 Files and events

* 771 `.animevents` files under `game/animations/` and `game/objects/`; 359 contain sound events. Two flavours per rig: `event_database.animevents` (skill effects, combat sync) and `sound_event_database.animevents` (sounds only).
* Event names across all files: `sound` 7444, `sound_pack` 12892, `footstep` 1458 (other names are skill effects: `skill_effect` 2829, `!skill_effect` 5229, `combat_sync` 3938, ...).
* Form (one event per line):
  `<event name="sound" time="0.12" parameter="Sounds/x2eventsounds:interaction:ballista_fire" bone="" offset="0,0,0" dir="0,0,0"/>`
* `time` is seconds into the clip; `bone` is the bone the sound is attached to (`""` on 5083 events, `Bip01 R Hand` 1325, `Bip01 L Hand` 558, `Bip01 Pelvis` 254, `Bip01 L Foot` 126, `Bip01 R Foot` 92, `Bip01 L Heel` 14, `Bip01 R Heel` 14).

Nuian male (`game/animations/nuian/male/sound_event_database.animevents`, 253 KB): 1039 `sound`, 229 `sound_pack`, 206 `footstep`. Example:
`<animation name="animations/hariharan/male/fist/fist_ac_punishment_hit.caf"> <event name="sound_pack" time="0" parameter="Sounds/x2character:nuian_male:dmg" .../>`

### 3.2 `sound` events

* 7434 are `Sounds/<proj>:<group>:<event>` (same format as the DB). 10 more carry footstep names (`footstep_ferre_walk`, `footstep_warborn_walk`), which are bugs.
* 7230 of 7434 resolve to an FEV event (97 percent). 196 do not (e.g. `Sounds/x2monster_s:mob:snowlion_baby_idle`, `Sounds/x2character:temp_ani:sk_bow_launch`), and 8 use the project typo `x2phyhsics`.

### 3.3 `sound_pack` events: the unit's pack

`parameter` is a pack item name, not a path. It is looked up in the unit's sound pack (`models.sound_pack_id`, section 1.1). Verified with the player models:

| model (`models.id`) | sound pack | item `Sounds/x2character:nuian_male:dmg` -> |
|---|---|---|
| 10 `nuian_male` | 41 `nuian_male` | `Sounds/x2voice_combat:nuian_male:dmg` |
| 16 `elf_male` | 43 `elf_male` | `Sounds/x2voice_combat:elf_male:dmg` |
| 18 `hariharan_male` | 45 `hariharan_male` | `Sounds/x2voice_combat:hariharan_male:dmg` |
| 14 `dwarf_male` | 428 `dwarf_male` | `Sounds/x2voice_combat:dwarf_male:dmg` |

Pack 42 (`nuian_female`) has the female equivalents (`Sounds/x2character:nuian_female:crit_dmg`, `...:attack`), so the item names are per race and gender.

Packs 41 to 48 each hold 143 `sv_*` skill voices (e.g. `sv_fight_triple_slash_1_launch` -> `Sounds/x2voice_combat:nuian_male:skill` in pack 41).

Totals: 12892 `sound_pack` events; 12396 of them (1566 of 1638 distinct names) match a pack item name in the DB. Unmatched examples: `Sounds/x2monster_footstep:move:blackbird_runidle`, `Sounds/x2physics:foley:swim`, `Sounds/x2physics:horse_fs_1st:dirt`, empty strings, and a number (`0.84132`, a parse slip in the source).

Note: `Sounds/x2character:nuian_male:*` items are also in NPC and monster packs (e.g. pack 395 `anthalon_(nuian_male)`, pack 49 `highorc_(nuian_male)`), so a pack lookup is needed; the path alone is not enough.

### 3.4 `footstep` events and the surface map

Event: `<event name="footstep" time="0.61000001" parameter="footstep_nuian_walk" .../>`.
* Parameter = a row of the `materialeffects.xml` sheet, `footstep_nuian` (run) or `footstep_nuian_walk` (walk). Other rigs: `footstep_ferre_walk`, `footstep_warborn_walk`, `footstep_horse_1st/2nd/3rd`, `footstep_snowlion_walk`, `footstep_yata_walk`, `footstep_dwarf(_walk)`, `footstep_elf(_walk)`, `footstep_hariharan(_walk)`, `footstep_returned(_walk)`, `footstep_donkey_*`, `footstep_polarbear_*`, `footstep_tare(_walk)`.
* Counts: `footstep_nuian_walk` 268, `footstep_nuian` 151 (nuian male 206 events in total), `footstep_ferre_walk` 280, `footstep_warborn_walk` 112, `footstep_horse_2nd` 72. Folders: nuian_male 206, ferre_male 198, warborn_male 190, ferre_female 183, nuian_female 167, animals_pegasus 103, monster_lion 90, monster_yata 51.
* Typos (no matching row in `materialeffects.xml`): `footstep_nuain_walk` 4, `foostep_nuian_walk` 3, `nuian_footstep` 3, `foostep_ferre_walk` 2. They will not match if the lookup is exact.

Chain (verified in the data, the engine code is not in the pak):
1. The footstep event gives the row: `footstep_nuian_walk`.
2. The ground's surface type comes from the material. Terrain `.mtl` files carry it: `game/materials/aa_terrain/field/tr_field_dirt_001c.mtl` has `SurfaceType="mat_dirt"`.
3. `game/libs/materialeffects/materialeffects.xml` is an Excel XML sheet. Column header = surface (`mat_dirt`, `mat_grass`, `mat_sand_dry`, `mat_water`, `mat_rock`, `mat_snow`, `mat_metal`, `mat_wood_solid`, `mat_wood_hollow`, ...; 38 columns). Row = fxlib (`footstep_nuian`, `footstep_nuian_walk`, ...; 37 rows). Cell = `fxlib:effect`.
4. The fxlib file (`game/libs/materialeffects/fxlibs/footstep_nuian.xml`) holds the effect:

| surface | cell (`footstep_nuian`) | run sound | walk sound (`footstep_nuian_walk`) | particle / decal |
|---|---|---|---|---|
| mat_dirt | `footstep_nuian:dirt` | `sounds/x2physics:mid_fs:dirt_run` | `sounds/x2physics:mid_fs:dirt_walk` | `collisions.footsteps.dirt` |
| mat_grass | `:grass` | `...:mid_fs:grass_run` | `...:mid_fs:grass_walk` | `collisions.footsteps.grass` |
| mat_sand_dry | `:sand` | `...:mid_fs:sand_run` | `...:mid_fs:sand_walk` | `collisions_m.footsteps_nuian.sand`, decal `materials/decals/footstep_sand` |
| mat_water | `:water` | `...:mid_fs:water_run` | `...:mid_fs:water_walk` | `collisions.footsteps.water_shallow` |
| mat_rock | `:rock` | `...:mid_fs:rock_run` | `...:mid_fs:rock_walk` | - |
| mat_metal | `:metal_thick` | `...:mid_fs:metal_run` | `...:mid_fs:metal_walk` | - |
| mat_default | empty cell | - | - | - |

Effects in `footstep_nuian.xml`: default, board, ice, snow, sand, magma, stone, concrete, wood_hollow, grass, dirt, wood_solid, gravel, leaves, metal_thick, rubble_stone, rubble_wood, rubble_metal, rubble_mixed, vegetation, rock, water, water_fake, mud, gear, gear_search. The walk file has the same 26 names.

Resolution: the 69 distinct sound names in the footstep and foley libs: 65 resolve; 4 `sounds/x2physics:foley:bm_*_tall/short` do not. `sounds/x2physics:nuian_fs:gear` (the generic `footstep.xml` default) resolves on the event name only; the group `nuian_fs` was not found as a string.

Physics surface definitions: `game/libs/materialeffects/surfacetypes.xml` (Excel XML, `SurfaceType` entries with physics and breakage particles).

### 3.5 Other footstep-like libs

`game/libs/materialeffects/fxlibs/foley_*.xml` (`foley_nuian`, `foley_hariharan`, `foley_dwarf`, `foley_ferre`, ...) hold landing / clothing sounds (`sounds/x2physics:small2_landing:rock`, `mid_landing:grass`). `vegetation.xml` (377 effects) has tree and leaf sounds (`sounds/x2physics:tree:tree_falling`).

---

## 4. Particle effects with sounds

* `game/libs/particles/*.xml`: 43 files, 23.3 MB. 1491 `Sound=` attributes in 23 files, 632 distinct paths.
* Per file: `x2_effects.xml` 410, `abillity_skill_table_m.xml` 341, `x2_sounds.xml` 232, `explosions.xml` 157, `ability_skill_table_k.xml` 138, `vehicle_fx.xml` 44, `x2_effects_p.xml` 42, `x2_effect_o.xml` 29, `smoke_and_fire.xml` 17, `bullet.xml` 16, `machinima_05.xml` 16, `water.xml` 3, others 1 to 8.
* Form (`x2_sounds.xml`):
  `<Particles Name="particles.lute_playing_01"><Params Continuous="true" Speed="0" Sound="Sounds/x2eventsounds:doodad:guitar_loop_01" BindToEmitter="true"/></Particles>`
  `Continuous="true"` = the sound loops while the emitter lives (e.g. `particles.cat_crying_loop`, `Sounds/x2eventsounds:quest:cat_cry_loop`).
* `SoundFXParam` (232 across the libraries, none in `x2_sounds.xml`): a float or a curve (`1`, `1,,(,1;1)`, `1,,(,1;0.15625,0.5;0.6651786,0.5;1)`), probably a volume over the particle life (not confirmed).
* Resolution: `x2_sounds.xml` 232 refs, 227 resolve; the 5 misses are `Sounds/x2physics:particles:waterfall_*` and `Sounds/x2eventsounds:interaction:common_vender_launch`.
* Full particle name used by the DB: `<library>.<Name>`, e.g. `x2_sounds.particles.lute_playing_01`, `abillity_skill_table_m.magic.magic_reinforce_dot4` (the `Name` attribute is `magic.magic_reinforce_dot4`). 2253 `fx_items` of type FxParticle; 888 of their `fx_particles` rows carry a `sound_id`.

---

## 5. UI sounds (Lua)

### 5.1 Call sites

| call | sites (decompiled) | bytecode scan | where |
|---|---|---|---|
| `PlayUISound(name [, duplicable])` | 73 in 34 files | 72 | trade, mail, login, world select, character create, battlefield, racing, achievements, quests, tutorial |
| `SetSounds(set)` (window / button / editbox) | 86 in 69 files | 86 (54 distinct literal names) | `window_view.lua`, `dialog_handler.lua`, most views |
| `PlayMusic(name)` | 6 (incl. wrapper) | 5 | login screens (section 6.1) |
| `StopMusic()` | - | 7 | login screens |
| `StopSound(id [, force])` | - | 24 | tutorial, character create, clientdriven indun |
| `IsPlaying(id)`, `SetSoundVolume` | - | - | `docs/ui-api.md` lists `X2Sound` as PlayUISound, PlayMusic, StopMusic, StopSound, IsPlaying, SetSiegePeriod |

Examples:
* `world_select.lua`: `PlayUISound("login_stage_world_select")`, `PlayUISound("login_stage_ready_to_connect_world")`.
* `trade.lua`: `PlayUISound("event_trade_item_putup", true)`, `PlayUISound("event_trade_lock")`.
* `battlefield/scoreboard.lua`: `PlayUISound("battlefield_win", false)`, `"battlefield_draw"`, `"battlefield_defeat"`.
* `centermessage/kill_effect.lua`: `PlayUISound(soundTable[count], false)` with `battlefield_kill_first` ... `battlefield_kill_more_than_sixth`.
* `centermessage/racing_msg.lua`: `racing_checkpoint_pass`, `racing_end_fail_finish`.
* `expedition_management_siege_tab.lua`: `event_siege_warm_up`, `event_siege_victory`, `event_siege_defeat`.
* `clientdrivenindun/client_driven_indun.lua`: `PlayUISound(sceneInfo.soundKey, false)` (the key is in `clientdriven.g`, section 5.4).

### 5.2 How a UI name resolves

`PlayUISound(name)` takes a sound-pack item name. Pack `global` (id 203, "global (w, b가 아닌 sound)", 212 items) maps it to a sound:

| name | pack item sound_id | path |
|---|---|---|
| `login_stage_try_login` | 816 | `sounds/x2interface:login_stage:button_login` |
| `login_stage_world_select` | 817 | `sounds/x2interface:login_stage:button_server` |
| `login_stage_start_game` | 818 | `sounds/x2interface:login_stage:button_gamestart` |
| `login_stage_ready_to_connect_world` | 9 | `sounds/x2interface:login_stage:connect_world` |
| `event_trade_item_putup` | 2541 | `sounds/x2interface:trade:item_putup` |
| `event_mail_alarm` | 2327 | `sounds/x2interface:mail:alarm` |
| `event_siege_victory` | 1212 | `sounds/x2eventsounds:conditions:siege_victory` |
| `battlefield_kill_first` | 4323 | `sounds/x2voice_system:battlefield:1kill` |
| `racing_checkpoint_pass` | 4647 | `sounds/x2interface:racing:checkpoint_pass` |
| `low_rank_achievement` / `high_rank_achievement` | (in `global`) | achievement stingers |
| `tutorial_contents_2584_2_1` ... | (in `global`) | tutorial narration, one per tutorial step |

Pack `please_remove` (id 410, comment "made by mistake") is an old copy: 174 names, 170 shared with `global`, 168 with the same sound id. Use 203.

`global` names are grouped: `event_quest_started_normal|main|saga|tutorial|hidden|task|daily|livelihood|group|daily_hunt` and their `_completed_` / `_failed_` / `_dropped_` forms; `event_item_*_added` (rarity); `event_mail_*`; `event_siege_*`; `login_stage_*`; `edit_box_text_added|deleted`.

### 5.3 SetSounds sets

`window:SetSounds("ability_change")` (and the 53 other literal names) map to the `window` pack (id 204, 100 items, `<set>_show` / `<set>_hide`):
* `ability_change_show` -> `sounds/x2interface:ability_change:show`; `ability_change_hide` -> `...:hide`.
* `bag_show` -> `sounds/x2interface:bag:show`; `config_hide` -> `sounds/x2interface:interface:button_cancel_1`.
* 47 of the 54 SetSounds names have a `<name>_show` item. The show / hide pair is played by the window (the native side is not in the pak; inferred from the names).

The 4 names not in `window` (`default`, `auction_put_up`, `quest_directing_mode`, `store_drain`) are in the `button` pack (id 205, 12 items):
* `default_left_up` / `default` / `default_r_right_up` -> `sounds/x2interface:interface:button_default`.
* `default_over` -> `sounds/x2interface:interface:button_default_ro`.
* `action_bar_left_drag_start` -> `sounds/x2interface:skill_bar:select`; `action_bar_left_drag_receive` -> `...:skill_bar:slot_in`.
* `auction_put_up_left_up` -> `sounds/x2interface:auction:registration`; `store_drain_left_up` -> `sounds/x2interface:store:drain`.

So a button's sound key is `<set>_<mouse event>` (`left_up`, `over`, `right_up`, `left_drag_start`, ...).

### 5.4 Missing voices (flagged)

* Character create: `RACE_GENDER_VOICE_TABLE` (in `character_create_step1_view.lua`) calls `PlayUISound("login_stage_character_create_<race>_<male|female>_selected")` for each race and gender, and `narrationSoundName = "login_stage_character_create_narration"`, `abilityNarrationSoundName = "login_stage_character_create_ability_narration"`. All 10 of these names exist in `global` with **sound_id NULL**. The lines must come from somewhere else in the client.
* Client-driven indun (`clientdriven.g`, `soundKey` values like `cdi_scene_quest_accept_title`): 16 items in `global`, all -> `sounds/x2voice_prologue:system:61xx` (e.g. `cdi_scene_quest_accept_title` -> `sounds/x2voice_prologue:system:6170`). **`x2voice_prologue` has no FEV in the pak** (32 DB paths).

### 5.5 Entity scripts (in-world, not Lua UI)

The decompiled entity scripts show the engine calls the entity classes of section 2 use:
* `entities/sound/ambientvolume.lua`: `PlaySoundEventEx(soundName, sndFlags, 0, pos, 0, 0, SOUND_SEMANTIC_AMBIENCE)`; stop with `StopSound(self.soundid)`.
* `randomsoundvolume.lua`: `PlaySoundEventEx(soundName, sndFlags, fade, pos, 0, 0, SOUND_SEMANTIC_AMBIENCE_ONESHOT)`.
* `soundeventspot.lua`: `PlaySoundEvent(soundName, pos, dir, flags, SOUND_SEMANTIC_SOUNDSPOT)`.
* `missionhint.lua`, `dialog.lua`: `PlaySoundEvent(..., SOUND_SEMANTIC_DIALOG)`.
* `musicthemeselector.lua`: `SetMusicTheme(sTheme, bForceChange, bKeepMood, nDelay)`, `SetMusicMood(sMood, 1, bForceChange)`, `SetDefaultMusicMood(sDefaultMood)`, `EndMusicTheme(...)`.
* `switch.lua`, `destroyableobject.lua`, `interactiveentity.lua`, `elevator.lua`: `PlaySoundEvent(prop_value, ...)` with the sound paths from the entity's properties.

Those functions are native (not in the pak); the event path is the property value.

---

## 6. Music

### 6.1 Login screens

| screen | Lua call | pack item (`global`) | track (path) |
|---|---|---|---|
| login (before login) | `login.lua:318` `PlayMusic("login_stage_music_before_login")` | `login_stage_music_before_login` (4551) | `sounds/x2music:screen:login_2013_07` |
| world select | none found in the decompiled Lua | `login_stage_music_world_select` (815) | `sounds/x2music:screen:login_cbt4_server` |
| character select | `character_select.lua:1` and `:199` `PlayMusic("login_stage_music_character_select")` | (814) | `sounds/x2music:screen:login_cbt4_character` |
| ability select | none found | `login_stage_music_ability_select` (1370) | `sounds/x2music:screen:login_cbt4_ability` |
| character create | `common.lua:472` `PlayMusic("login_stage_music_creator")`; `StopMusic()` in `character_create_step3_view.lua:279` | (4589) | `sounds/x2music:screen:login_creator_oblivion` |

`x2music.fev` group `screen` (7 DB paths) holds these. The world-select and ability tracks have no Lua call in the scripts: either the world select keeps the previous track, or the call is native (inferred; check in game). The login 2D world ambience is section 2.3.

### 6.2 In game: zone music packs

* Zone packs: `zone_groups.sound_pack_id` (78 zones, 72 with a pack). The DB zone name matches the `world.g` zone name without its `_N` suffix (inferred from the names, e.g. `w_solzreed` covers zones 142 and 179). Sub-zones carry their own `sub_zones.sound_pack_id` (235 rows, 23 packs); which of the two wins at runtime is not confirmed.
* Example: `w_gweonid_forest` (zone group 1, `그위오니드 숲`) -> pack 101 `soundmood_w_gweonid_forest`.
* The spawn cell 015_015 is in `w_solzreed` (pack 99 `soundmood_w_solzreed`) and `s_silent_sea` (zone group 36 -> pack 128 `soundmood_s_all`). Which of zone and sub-zone wins is not confirmed.

Each of the 72 `soundmood_*` packs has the same 17 mood keys (each present in all 72):
`underwater`, `default`, `combat`, `menu`, `machinima_default`, `death`, `gliding`, `driving`, `flyng`, `moving`, `falling`, `dancing`, `low_health`, `quest_camera`, `loading`, `indoor`, `siege_period`.

These are the same names as the moods in `game/libs/soundmoods/soundmoods.xml` (section 6.3). Plus `music_01` .. `music_08` (in 60, 55, 37, 26, 17, 13, 9 and 2 packs).

Pack 101 `soundmood_w_gweonid_forest` (non-empty moods):

| item | sound_id | path |
|---|---|---|
| `music_01` | 2305 | `sounds/x2music:group_002:01` |
| `music_02` .. `music_05` | 2306 .. 2309 | `sounds/x2music:group_002:02` .. `:05` |
| `combat` | 1200 | `sounds/x2music:conditions_bgm:battle_normal` |
| `death` | 1202 | `sounds/x2music:conditions_bgm:death` |
| `gliding` | 2016 | `sounds/x2vehicles:glider:run_soundmood` |
| `falling` | 1207 | `sounds/x2eventsounds:conditions:falling` |
| `low_health` | 1208 | `sounds/x2eventsounds:conditions:lowhealth` |
| `quest_camera` | 1210 | `sounds/x2eventsounds:conditions:quest_camera` |

Empty in pack 101: `underwater`, `default`, `menu`, `machinima_default`, `driving`, `flyng`, `moving`, `dancing`, `indoor`, `siege_period`, `loading`. So a plain mood falls back to `music_NN`. Pack 128 (`soundmood_s_all`, sea): `music_01` -> `sounds/x2music:sea_001:01`, `combat` -> `sounds/x2music:conditions_bgm:battle_sea`. Library boss room (pack 462): `combat` -> `sounds/x2music:id_library:boss_battle`, `music_01` -> `sounds/x2music:id_library:boss`.

Totals: 659 pack items in soundmood packs have a sound.

### 6.3 The mood engine and mixer categories

`game/libs/soundmoods/soundmoods.xml`: 17 `<Mood>` entries with a `Priority` and per-category mix (volume, low-pass, EQ), e.g. `underwater` Priority 8, MusicVolume 0.8, low-pass 300 on ambience and footsteps. The 31 mixer categories (the FMOD categories, as used by the moods): `master`, `music`, `music_battle`, `music_death`, `music_loginstage`, `music_loading`, `music_siegeperiod`, `music_inside_off`, `ambience`, `ambience_inside`, `ambience_underwater`, `ambience_loginstage`, `condition`, `skill`, `weapon`, `footsteps`, `ui_button`, `npc_voice`, `monster_voice`, `player_voice`, `player_death`, `player_movement`, `system_voice`, `tutorial_voice`, `vehicle`, `physics`, `particle`, `items`, `cutscene`, `death_particle`, `underwater_only`. `master`, `music`, `skill`, `condition`, `death_particle` and `ui_button` also appear as strings in `x2skills.fev`.

`game/libs/musiclogic/musiclogic.xml` is the Crysis-era music logic (events `VehicleEnter`, `SniperModeEnter`, `EnemySpotted`, ...). It names no ArcheAge content and is probably unused.

### 6.4 Day and night

* No music pack has a day / night variant. The only day / night split is in ambience: `x2ambience:world2_day:*` (671 entity uses) vs `world2_night:*` (468).
* AreaShape `<TimeOfDay>` with a range is on about 40 of the 1100 areas; the rest are empty (`TimeStart` 0, `TimeEnd` 24).

### 6.5 Other music

* `x2music.fev` (139 DB paths): `subzone_bgm` 18, `conditions_bgm` 8, `group_001` .. `group_011` (4 to 8 each), `screen` 7, `jukebox` 7 (player music), `id_library` 7 (library boss rooms), `zone_bgm` 2 (not in the FEV: `zone_bgm:archemall` and `zone_bgm:o_group_01` do not resolve).
* Instruments: `instrument_sounds` (199 rows, GM program per item) and `music_co_sk_*` skill events (`combat_sync_event_list.g`).

---

## 7. Event path format

### 7.1 The DB form

`<folder>[/<subgroup>]:<group>:<event>`, with `folder` = `sounds` or `Sounds` (case differs in the data; the pak folders are lowercase).

Real examples:
* `sounds/x2skills:monster_skill:blood_spray_dot` (name `Monsterskill_blood_spray_dot`)
* `sounds/x2skills:wild:fire_arrow_form`
* `Sounds/x2monster_footstep:move:dirt_lots_mid`
* `sounds/x2item:paper:pickup`
* `sounds/x2eventsounds:doodad:piano_note_c`
* `Sounds/x2skills_npc:s15885:ground_hit` (group = NPC skill id)
* `sounds/x2weapons:x2weapons:shield_flesh` (group = project name; 72 such paths)
* `sounds/x2skills/monster_skill:pino_effect.mon.mon_heal_hit` (the only slash-group row; the event contains dots)

Note: the example `Sounds/x2skills:x2skills:fire_arrow` from the brief is not in the data. The real one is `sounds/x2skills:wild:fire_arrow_form`.

### 7.2 Mapping to the pak

| part | file / name |
|---|---|
| `folder` (`x2skills`) | `game/sounds/x2skills/x2skills.fev` (FMOD project); FSB banks `game/sounds/x2skills/x2skills_bank1100.fsb` ... |
| voices | `game/sounds/x2voice_combat/x2voice_combat.fev` and the language copies `game/sounds/x2voice_combat/{en_us,fr,de}/x2voice_combat.fev` (the same basename in each language folder); the audio banks are per language (`.../fr/*.fsb`, 66 each) |
| `group` (`monster_skill`) | an FMOD event group / category; a null-terminated name in the FEV (`\x00monster_skill\x00`) |
| `event` (`blood_spray_dot`) | the FMOD event; a null-terminated name inside that group (`\x00blood_spray_dot\x00`) |

The FEV is an FMOD Designer project (RIFF `FEV `, `LIST` / `PROJOBCT` chunks). Names are plain null-terminated ASCII. The wave source names inside are like `monster_skill/edit_22k/monsterskill_blood_spray_dot.wav`, which are only the originals.

### 7.3 Validation against the FEVs

| result | count of distinct DB paths (of 4868) |
|---|---|
| project FEV, group and event resolved | 4764 |
| event missing in the FEV (e.g. `sounds/x2skills:monster_skill:ground_launch_end`, `sounds/x2interface:interface:button_ok_2`, `sounds/x2music:zone_bgm:o_group_01`, `Sounds/x2skills:romance:song_00`) | 43 |
| project FEV not in the pak (`x2voice_prologue` 32, `x2voice_quest` 7, `x2voice_npc` 1, typo `x2minster_y` 1) | 41 |
| event found, group name not found (`test` 12, `fepee` 5) | 17 |
| no colon (2 placeholders), slash-group form | 3 |

The pak has 48 distinct FEV basenames; the DB and the animation / particle sounds use the same FEV set. Animation `sound` events resolve at 97 percent (section 3.2); the `Sound=` paths of `x2_sounds.xml` at 98 percent (227 of 232, section 4).

### 7.4 How each source names a sound

| source | field | example value |
|---|---|---|
| DB `sounds.path` | path | `sounds/x2skills:monster_skill:blood_spray_dot` |
| DB pack item | name -> `sound_id` -> path | `combat` -> 1200 -> `sounds/x2music:conditions_bgm:battle_normal` |
| DB `fx_items` FxSound | `asset_name` | `Sounds/x2skills_npc:s22127:mokujin_wind_hit` |
| DB `fx_items` FxVoice | `fx_detail_id` -> `fx_voices.sound_pack_item_name` | `sv_magic_meteor_form` (resolved in the caster's pack) |
| entities | `soundName` / `reverbpresetReverbPreset` | `Sounds/x2ambience:world2_day:arche_field_day` / `x2_outdoor_1` |
| animevents `sound` | `parameter` | `Sounds/x2eventsounds:interaction:ballista_fire` |
| animevents `sound_pack` | `parameter` = pack item name | `Sounds/x2character:nuian_male:dmg` (in pack 41) |
| animevents `footstep` | `parameter` = fxlib name | `footstep_nuian_walk` |
| particles | `Sound=` | `Sounds/x2eventsounds:doodad:guitar_loop_01` |
| Lua `PlayUISound` / `PlayMusic` | pack item name (global pack) | `login_stage_try_login` (-> `sounds/x2interface:login_stage:button_login`) |
| Lua `SetSounds` | set name -> `window` (`<set>_show`) or `button` (`<set>_left_up`) pack | `ability_change` -> `ability_change_show` |

The rule that matters for the player: an entry is either a full event path (DB paths, entity and particle `Sound`, animation `sound`) or a name that must first be looked up in a pack (`sound_pack`, `PlayUISound`, `SetSounds`, `fx_voices`).

Case: all lookups should be case-insensitive (`Sounds/` vs `sounds/`; pak folders lowercase).

### 7.5 Known bad data (skip or log)

* DB: 66 empty paths; 2 Korean placeholders; a doubled string `Sounds/x2ambience:world2_night:sea1_nightSounds/...` in an entity; 2 `environment` project paths in entities; 3 `ReverbVolume` rows with a sound path in the preset field.
* Animations: footstep names used as `sound` events (10); typos (section 3.4; `x2phyhsics` x 8).
* `x2voice_prologue`, `x2voice_quest`, `x2voice_npc` FEVs are not in this pak, so 41 DB paths and the `cdi_scene_*` voices cannot play from this client data.

### 7.6 Banks to preload

`game/sounds/audiopreloads_32.xml` (`<PreloadData>` > `<Platform PC="1">`): `<Global>` (13 files: `sounds/default.dls`, `x2eventsounds_bank214.fsb`, `x2interface/x2interface_global.fsb` (1220 KB UI), `x2item/x2item_bank110.fsb`, ... with Korean size comments), `<Level name="...">` (11 levels: `loginbg4` 3 files, `instance_prologue` 33, `instance_sal_temple` 1, `instance_library_boss_3` 2, `instance_training_camp` 2, ...), and `<anim_sequence_*>` (116 cinematic sequences, one bank each, e.g. `anim_sequence_zq_01_wing` -> `x2machinima_quest_bank09.fsb`). `audiopreloads_64.xml` has 620 files. `game/sounds/default.dls` (RIFF DLS, 3.4 MB) is a General MIDI instrument bank used for the instrument songs (inferred).

---

## Summary of first-integration order

1. **Login music and UI sounds** (P1): the `global` / `window` / `button` packs (212 + 100 + 12 items), 6 PlayMusic sites and 5 login tracks. Small, and they cover the three login screens the brief asks for. Needs the pack lookup, FEV event and FSB playback.
2. **Zone music** (P1): 72 `soundmood_*` packs x 17 moods, `zone_groups` + `sub_zones` for the zone, and the mood switch (`soundmoods.xml`). Music is the first thing a player hears in game.
3. **Footsteps** (P1): `footstep_*` rows x `mat_*` columns x `footstep_*.xml` (1458 events, 37 rows, 38 surfaces). Needs the surface from the terrain `.mtl`.
4. **Placed ambience** (P2): AmbientVolume, RandomSoundVolume, SoundEventSpot, ReverbVolume (3775 entities; 9 spawn cells = 53). Needs areas (AreaShape) for the on/off volumes.
5. **Animation sounds** (P2): `sound` 7444 (direct paths), `sound_pack` 12892 (per-unit pack; needs `models.sound_pack_id`).
6. **Items, doodads, quests, chat bubbles** (P2): direct `sound_id` lookups (`items`, `doodad_funcs`, `quest_components`, `quest_chat_bubbles`, `npc_chat_bubbles` via pack 370).
7. **Particles and skill FX sounds** (P2 / P3): `Sound=` in 43 particle libraries (1491); FxSound 422 and FxVoice 143 for the combat FX (currently not drawn by the combat code).
8. **Combat hit sounds** (P3): `combat_sounds` (392) with `holdables` / `models.sound_material_id`.
9. **Voice lines** (P3, blocked): the prologue and quest voice FEVs are missing from the pak; the character-create greetings have no sound ids.
