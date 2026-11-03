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
  `validation.rs:290`) when the water probe fails. The probe (`in_water` / `is_water`) was fixed in
  [AAEmu/AAEmu-1.2-Rust#43](https://github.com/AAEmu/AAEmu-1.2-Rust/pull/43). `target_fishing` is
  parsed (`data/templates.rs:150,311`) and **not enforced anywhere** (reported, server not changed).

## What the Bevy client does

`ingame/ground_target.rs`:

* `GroundSkills` reads `target_only_water` and `target_water` into `GroundSkill`; 18711/21571/22106
  classify as `Mode::Ground` from `(6, 4)`.
* `spot_under` / `pick_hit` intersect the ray with the water surface when either flag is set; other
  ground skills keep the lakebed (the planting water rule compares the spot with the surface).
* A `target_only_water` spot is refused with `INVALID_LOCATION` (0x23), without sending, when
  `water_at(x, y, z - 0.1)` is missing or above `z - 0.1`, or when the terrain floor is shallower
  than `level - 1.0`. This mirrors `FUN_390b43b0`. The reticle turns red for the same reason.
  **Approximation:** the native floor query (`I3DEngine` vtbl+0x108 with arg 1.5) is likely a
  bottom-level query that includes static geometry; Bevy uses `TerrainQuery` only, so a floating
  static floor over deep water is not seen here.
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
makes the trophy/furniture). Both are ordinary phase skills, so the dynamic action bar offers them;
`hud::doodad::tests::the_fish_stands_offer_their_sell_and_convert_skills` pins the real rows.

The 1.2 doodad-skill use path `FUN_393b3ec0` (770741) calls `FUN_393bc260(skill, funcId, doodad)`
(770820, 778111) before sending and switches on the func type (ids assigned by name in
`FUN_39673b90`, 1388594-1388674): `DoodadFuncBuyFish` (0x4f) -> `FUN_393b8870` (774963),
`DoodadFuncConvertFish` (0x50) -> `FUN_393b8a30` (775055). Both **return 1 = handled, do not send**
after the dialog opens (or after a check error).

* Buy checks `FUN_397cf5a0` (1727959): no item on the back -> 0x169 `STORE_BACKPACK_NOGOODS` (361);
  template not in `doodad_func_buy_fish_items` -> 0xe1 `STORE_NPC_NOT_HANDLE_THIS_ITEM` (225); item
  kind (`item_backpacks.backpack_type_id`) not 6, `items.price` 0, or no `fish_details` row ->
  0x58 `STORE_NOT_SELLABLE_ITEM` (88). The decompile checks the list **before** the kind.
* Convert `FUN_397cf660`: no item -> 0x169; kind not 6 -> 0x58 (checked **before** the list);
  template not in `doodad_func_convert_fish_items` -> 0xe1. No refund.
* On success the native enters interaction state 0xb/0xc (`FUN_393b7b10`), closes an open task-12/13
  dialog (`FUN_3902cef0`), and shows dialog type 0xC/0xD (`FUN_393b87b0` / `FUN_393b8970`,
  `FUN_3902d670` -> `FUN_3902d570` -> `FUN_3902d400`). The info table gets the base keys
  (`type`, `buttonType`, `isModal`) and `itemInfo` from `FUN_393b93e0` (775600) via
  `FUN_39421e20` (865686-865693). The fish detail tag 6 supplies `f32` weight and `f32` length
  (`itemInfo.weight` / `.length`), and the refund is
  `trunc(floor(items.refund * item_grades.refund_multiplier * 0.01 + 0.5) * weight /
  fish_details.max_weight)` (`FUN_397cd350` 1726122, `fish_details` loaded by `FUN_3979f2a0`);
  both tasks carry it. `dialog_handler.alb` F12 (task 12) / F21 (task 13) format it:
  `confirm_buy_fish_title/body` with the icon, name and refund, OK `sellAll`; or `ask_convert_fish`
  / `ask_convert_fish_description`, OK `make`. Both cancel on `INTERACTION_END`.
* The interaction ends (`FUN_393b7e40` 774405) when the doodad is gone or the player is out of the
  configured interaction distance (2.1 m, `FUN_39093800` with cfg +0x2a0); `FUN_393b7aa0` then fires
  `INTERACTION_END`.
* OK is `FUN_393b8800` (774939): it sends the ordinary doodad func skill (the same call as 770836),
  i.e. `UseDoodadSkill` 21904/22201. Cancel sends nothing.

Bevy implements this in `hud/doodad/fish.rs` + `hud/doodad/fish.lua`, hooked by a few lines in
`hud/doodad.rs`:

* `Tables` records `(func_group, skill)` -> `FishFunc { kind, actual_func_id }` and loads the two
  item lists (`doodad_func_buy_fish_items`, `doodad_func_convert_fish_items`).
* `execute` runs the existing labor/attachment checks first, then for a fish func runs `check`
  (the errors above through `ShowError`) and opens task 12/13 with
  `X2DialogManager:RequestTaskDialog`. `fish.lua` passes
  `{ type, buttonType = DBT_OK_CANCEL (0), isModal, itemInfo }` (no top-level `itemType`/
  `itemCount`, as native), `HUD.item_tip` supplies the icon and name, and `itemInfo` gets
  `weight`, `length` and `refund`.
* Nothing is sent before OK. `fish::tick` reads the state from the wrapped `OnOK`/`OnCancel`; OK
  sends the stored `UseDoodadSkill`, cancel drops it. The tick also drops it when the doodad is
  gone or the interaction gap reaches `INTERACTION_DISTANCE` (the coffer/bench distance check),
  firing `INTERACTION_END`.
* Without the UI runtime (or the `FishDialog` resource) the skill is sent unconfirmed.
* Tests drive the real `execute` and the original `dialog_handler.alb` from the pak:
  `hud::doodad::fish::tests` (three errors, price/fish_details, the weighted refund, weight/length,
  OK send, cancel, out-of-reach and despawn).

## Open (capture-pending)

* the native reticle material for a `target_only_water` skill on land;
* a real `NoFishing="1"` volume (none in the 1.2 pak) and its gameplay use;
* net fishing: how 1.2 picks the fish-school doodad as the skill target;
* a live fish-stand sale/convert and cast/loot on a lane server.

The fish-stand monitor checks reach and existence before consuming OK, including when a stand
disappears in the same frame as confirmation. Such a confirmation sends no skill packet.
