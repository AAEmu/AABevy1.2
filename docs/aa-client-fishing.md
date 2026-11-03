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

All three rod skills and the big-fish skills set `target_only_water`, `target_water` and
`source_not_swim` (the rod skills also set `target_fishing` and `plot_only`); the net skills have
`target_water` but not `target_only_water`. The client's skills query (`x2game.dll.c` string
`39a06a90`, loader `FUN_39731870` and row copy `FUN_39719df0`) puts them at record offsets
`target_fishing` +0x194, `target_only_water` +0x1a0, `target_water` +0x1b8 and `source_not_swim`
+0x14f.

## Native targeting and validation

* The cast spot is the generic ground target of a `Pos`/`Location` skill (the reticle under the
  cursor, `target_correct` / `target_null`, `FUN_3942b060` line 871923 and `FUN_394322f0` line
  877419). The cursor ray pick is `FUN_394281b0`; both aim functions choose the **water surface**
  when `target_water` (+0x1b8) or `target_only_water` (+0x1a0) is set, not only for water-only
  skills.
* `FUN_390b5790` (149979) runs the water check (flags 0x442; `*param_10 >> 10 & 1`) only when
  `target_only_water` is set, and calls `FUN_390b43b0(target_fishing)` (148796):
  * the water level at `(x, y, z - 0.1)` must exist and `z - 0.1 <= level`;
  * the floor under the point (`I3DEngine` vtbl+0x108, arg 1.5) must be `<= level - 1.0` (at least
    1 m deep);
  * when `target_fishing` (+0x194) is set and the hit object is a type-8 water volume, its
    `vtbl+0x150` bit 0 must be clear. That is very likely the `NoFishing` prefab property read by
    `FUN_3910cd10` (209910, `WaterVolumePrefabObject::Load`, line 209997, `param_1 + 0x78`).
  * all failures return 0x23 `INVALID_LOCATION`; the click handlers `FUN_390f3b20` /
    `FUN_390f4c30` show the error and leave the aim up.
* `source_not_swim` (+0x14f) is **never read by the client**. A cast made while standing in water
  is sent; the server refuses it with 0x15 `CANNOT_CAST_IN_SWIMMING`.
* `NoFishing` data: the prefab XMLs under `game/prefabs/` carry it on 19 `Type="WaterVolume"`
  objects and every one is `NoFishing="0"`. `aa-world`'s gameplay `WaterQuery` loads only the
  `object.dat` type-11 water records (kinds 2/3 and the 0x200/0x400 river flags), which carry no
  `NoFishing`. There is therefore no shipped volume to refuse; loading the prefab water volumes is
  not needed for 1.2 data. If a future source sets the flag, it has to be added to `WaterQuery` and
  checked here.
* The refusal number is the client's own `SkillResult` name (`FUN_39897990`, 1929200):
  0x23 `INVALID_LOCATION`.
* Server (`aaemu-game`): `source_not_swim` refuses `skill_cannot_cast_in_swimming` (0x15,
  `validation.rs:265`) and `target_only_water` refuses `skill_invalid_location` (0x23,
  `validation.rs:290`) when `is_water(target + 0.1)` is false. `target_fishing` is parsed
  (`data/templates.rs:150,311`) and **not enforced anywhere** (reported, server not changed).

## What the Bevy client does

`ingame/ground_target.rs`:

* `GroundSkills` reads `target_only_water` and `target_water` into `GroundSkill`; 18711/21571/22106
  classify as `Mode::Ground` from `(6, 4)`.
* `spot_under` / `pick_hit` intersect the ray with the water surface when either flag is set; other
  ground skills keep the lakebed (the planting water rule compares the spot with the surface).
* A `target_only_water` spot is refused with `INVALID_LOCATION` (0x23), without sending, when
  `water_at(x, y, z - 0.1)` is missing or above `z - 0.1`, or when the terrain floor is shallower
  than `level - 1.0`. This mirrors `FUN_390b43b0`. The reticle turns red for the same reason.
* The picked surface point is sent **unchanged** (`FUN_393594c0` only clamps horizontally); there is
  no bobber sink.
* There is no client `source_not_swim` pre-check: the cast goes out and the server's 0x15 refusal
  is shown.
* Net fishing (11104/11105) is a `Doodad`-target skill; it is not a ground skill and is sent by the
  plain skill path. The fish school doodads (6447/6448) are the targets the server's
  `SpawnFishEffect`/`DoodadFuncFishSchool` flow names; selecting one as the client target is
  **capture-pending** (Bevy's click/Tab target picker is unit-only, and `Command::UseSkill` encodes
  a unit target).

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

## Fish stands and the dialog tasks

`doodad_funcs` gives the fish-scale phases (`tool.fish_scale_a`, groups 17262/19948/17729) skill
21904 (`DoodadFuncBuyFish`, sells the fish worn on the back) and 22201 (`DoodadFuncConvertFish`,
makes the trophy/furniture). Both are ordinary phase skills, so the dynamic action bar offers them
and the use sends `UseDoodadSkill` at the doodad (`hud/doodad.rs`, already generic);
`hud::doodad::tests::the_fish_stands_offer_their_sell_and_convert_skills` pins the real rows.

The client also exposes the dialog tasks `DLG_TASK_BUY_FISH` = 12 and `DLG_TASK_CONVERT_FISH` = 13
(`crates/aa-ui/src/lua/constants.rs:210,215`; native registration in `FUN_3905f3c0`, ~89430/89441).
`dialog_handler.alb` registers handlers F12 (line 254) and F21 (line 679). F12 builds a
`confirm_buy_fish_title` / `confirm_buy_fish_body` message box with the item icon, name, count and
the refund (`|m%s;`), OK text `sellAll`; F21 the same for `ask_convert_fish` / `ask_convert_fish_description`
with OK `make`. Both cancel on `INTERACTION_END`, and OK calls `X2DialogManager:OnOK`, i.e. the
native opener owns the action.

What opens them is not established: no 1.2 script opens either task (the only `DLG_TASK_*` use in
`game/scriptsbin/x2ui` is the registration in `dialog_handler.alb`), the client's native opener was
not located, and the server's `DoodadFuncBuyFish.Use` / `ConvertFish.Use` sell/convert immediately
when the skill arrives, with no dialog packet. The evidence-backed fish-stand path is therefore the
func skill; a confirmation dialog before the skill (if it exists natively) is **capture-pending**
and not implemented.

## Open (capture-pending)

* the native reticle material for a `target_only_water` skill on land;
* a real `NoFishing="1"` volume (none in the 1.2 pak) and its gameplay use;
* net fishing: how 1.2 picks the fish-school doodad as the skill target;
* a native opener for `DLG_TASK_BUY_FISH` / `DLG_TASK_CONVERT_FISH` (the dialog confirmation);
* a live cast/loot on a lane server.
