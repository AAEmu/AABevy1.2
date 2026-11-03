# aa-client: fishing (1.2 has no minigame)

Research: `handoff/2026-11-03-farm-fishing-research.md` (Fishing). 1.2 has no `FISHING_*` events, no
fishing UI and no reel packets: fishing is three `skills` rows that cast at a water position, one
fish-school/loot flow on the server, and the generic doodad bar at the fish stands.

## The skills

`skills` rows (read-only `compact.sqlite3`):

| id | name | target | range | plot |
|---|---|---|---|---|
| 18711 | 낚시하기(테스트) (test, auto-learn) | `target_type_id` 6 `Pos`, `target_selection_id` 4 `Location` | 8..30 m | 659 |
| 21571 | 낚시하기 | 6 / 4 | 8..30 m | 809 |
| 22106 | 낚시하기 (Mirage Isle) | 6 / 4 | 8..30 m | 903 |
| 11104 | 낚시하기 (net) | `target_type_id` 8 `Doodad`, selection 2 `Target` | 0..50 m | none |
| 11105 | 낚시그물 치기 (net, 5 s cast) | 8 / 2 | 0..4 m | none |
| 21578, 21750, 22092, ... | 대어 낚시 (big fish) | 6 / 4 | 8..30 m | 821, 840, 897, ... |

All three rod skills have `target_only_water`, `target_water`, `source_not_swim`, `target_fishing`
and `plot_only` set; the net skills have `target_water` but not `target_only_water`/`target_fishing`.
The client's own skills query loads every one of these columns (`x2game.dll.c` string
`39a06a90`, `FUN_39731870`), so the native skill record knows them; the Bevy loader keeps the two
that decide the cast (`skills::ground_target`).

## Native targeting and validation

* The cast spot is the generic ground target of a `Pos`/`Location` skill (the reticle under the
  cursor, `target_correct` / `target_null`, `FUN_3942b060` line 871923 and `FUN_394322f0` line
  877419): a left click sends `CSStartSkill` with a position target (`FUN_39393920` line 744970).
  Nothing in those functions names water; the water rule is the server's.
* `NoFishing` in `x2game.dll.c:209997` is **not** an error code. `FUN_3910cd10` (209910) is
  `WaterVolumePrefabObject::Load`; it reads the prefab properties `VolumeDepth`, `StreamSpeed`,
  `UScale`/`VScale`, ... and `NoFishing` into the water volume object (`param_1 + 0x78`). Which
  data file carries the property and whether gameplay reads it are not established (the
  `object.dat` water records only carry the `kind` flags 2/3/0x203/0x403 in the sampled cells).
  **Capture-pending.**
* The refusal numbers are the client's own `SkillResult` names (`FUN_39897990` line 1929200):
  0x15 `CANNOT_CAST_IN_SWIMMING`, 0x23 `INVALID_LOCATION`.
* Server (`aaemu-game`): `source_not_swim` refuses `skill_cannot_cast_in_swimming` (0x15,
  `validation.rs:265`) and `target_only_water` refuses `skill_invalid_location` (0x23,
  `validation.rs:290`) when `is_water(target + 0.1)` is false. `target_fishing` is parsed
  (`data/templates.rs:150,311`) and **not enforced anywhere** (reported, server not changed).

## What the Bevy client does

`ingame/ground_target.rs`:

* `GroundSkills` reads `target_only_water` and `source_not_swim` into `GroundSkill`; 18711/21571/
  22106 classify as `Mode::Ground` from `(6, 4)`.
* `spot_under` also intersects the ray with a water surface (`WaterQuery`), so the reticle and the
  bobber sit on the sea/lake/river over the lakebed.
* A water-only spot that is dry turns the reticle red and a click raises `INVALID_LOCATION` (0x23)
  without sending. A water cast is sent 0.25 m under the surface (`WATER_CAST_DEPTH`) because the
  server asks `is_water(target.z + 0.1)`. The depth is **capture-pending** (the native hit point
  is not captured).
* `source_not_swim` refuses with `CANNOT_CAST_IN_SWIMMING` (0x15) when the caster's feet position
  `+ 0.1` is inside a water volume, the same test as the server.
* Net fishing (11104/11105) is a `Doodad`-target skill; it is not a ground skill and is sent by the
  plain skill path. The fish school doodads (6447/6448) are the targets the server's
  `SpawnFishEffect`/`DoodadFuncFishSchool` flow names; selecting one as the client target is
  **capture-pending** (Bevy's click/Tab target picker is unit-only).

## The cast and the catch

The plot runs through the existing skill visual system (`ingame/combat`: every `SCPlotEvent` step
plays its `Anim`/`FxGroup`/projectile effect):

| plot step | effect | what the client plays |
|---|---|---|
| 809/659/903 "시전" | special effect 34 `Anim` 472 | `fist_fishing_casting` |
| 809/659/903 the same step | 38 `ProjectileAnim` 359 | the fishing line `projectiles` row 359 (`황의 낚시 투사체`, fx group 1354) |
| "채널링 시작" | 34 `Anim` 468 (plot 659: 468) | `fist_fishing_idle` (looping) |
| "채널링 종료" | 34 `Anim` 48 | `all_ac_stop` (ends the loop) |
| "낚아올리기" | 79 `FishingLoot` | no client effect: the server gives the zone group's fishing loot pack |

The catch is given directly by the server (`FishingLoot` -> `give_loot_pack`, task type
`SKILL_EFFECT_GAIN_ITEM`); the client shows it through the ordinary item-task/`ADDED_ITEM` path.

**Big-fish fishing** (21578, 21750, 22092, ... `대어 낚시`, plots 821/840/897/696/...) is a separate
`Pos` skill family with the same water rules. Its "챔질" (hook) step carries a `SpawnFishEffect`
(`plot_effects.actual_type = 'SpawnFishEffect'`, 12 rows, e.g. plot 821 step 6, `spawn_fish_effects`
rows 1..12): the server spawns the fish NPC at the bobber through a temporary spawner, targets it and
sets its aggro (`systems/npcs/bridges.rs:722`). The client needs nothing extra: the NPC arrives as
an ordinary `SCNpc(s)Created` unit and is fought/looted like any other. The basic rod plots
(659/809/903) have no `SpawnFishEffect`.

## Fish stands

`doodad_funcs` gives the fish-scale phases (`tool.fish_scale_a`, groups 17262/19948/17729) skill
21904 (`DoodadFuncBuyFish`, sells the fish worn on the back) and 22201 (`DoodadFuncConvertFish`,
makes the trophy/furniture). The 1.2 client has no fish-stand window: both are ordinary phase
skills, so the dynamic action bar offers them and the use sends `UseDoodadSkill` at the doodad
(`hud/doodad.rs`, already generic). `hud::doodad::tests::the_fish_stands_offer_their_sell_and_convert_skills`
pins the real rows.

## Open (capture-pending)

* the bobber's exact target height (0.25 m is this client's choice, not a capture);
* the native reticle material for a `target_only_water` skill on land;
* `NoFishing` water volumes (which data carries the flag; whether the client refuses the cast);
* net fishing: how 1.2 picks the fish-school doodad as the skill target;
* a live cast/loot on a lane server.
