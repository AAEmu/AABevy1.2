# aa-client: public farms

The original public-farm windows (`x2ui/commonfarm`: `info.alb`, `my_farm_info.alb` and their views,
unchanged Lua) over aa-net's farm protocol and the `farm_groups` / `farm_group_doodads` /
`common_farms` tables. Code: `crates/aa-client/src/hud/farm.rs`; the tables load with the rest of
`hud/data/housing.rs`.

## Native mechanism (x2game.dll.c)

The farm natives are registered next to the other `X2` methods (`FUN_39214380`; the getters are
`FUN_394445B0` … `FUN_394447C0`):

| `X2` method | Native | Answers |
|---|---|---|
| `GetFarmGroups()` | `FUN_394447C0` | array of `{ type = farm_groups.id, name = name }` in the loaded row order |
| `GetFarmGorupInfo(kind)` | `FUN_394445F0` → `FUN_39441450` | `(name, count, description)` from the `farm_groups` row |
| `GetFarmGroupDoodadList(kind)` | `FUN_39445BA0` | the names of the `farm_group_doodads` doodads of `kind`, in row order |
| `GetCommonFarmInfo(id)` | `FUN_394445B0` → `FUN_39441370` | `(name, guard_time, farm_group_id)` from the `common_farms` row |
| `GetRuntimeCommonFarmDoodadsInfo(kind)` | `FUN_39444630` → `FUN_39035620` | one table per planted crop of `kind` (nil when there is none) |
| `GetRuntimeCommonFarmDoodadCount(kind)` | `FUN_39444730` | the crop count of `kind` (0 allowed) |
| `RequestRuntimeCommonFarmDoodadInfo()` | `FUN_39440B20` → `FUN_39035560` | `CSRequestCommonFarmList` (0x10b) |

The client's typo is the native spelling (`GetFarmGorupInfo`); the scripts call the methods with
`:` (`SELF` in `commonfarm/info.alb` / `my_farm_info.alb`), so the closures take `(self, arg)`.

`FUN_39035560` refuses the request in an instant zone: it clears the list, raises the error
`cannot_request_commonfarm_info_in_instant_zone` (0x242) and `COMMON_FARM_UPDATED` (0x101). It also
turns a request within 2 s (`_DAT_3999d848` = 2.0) into `COMMON_FARM_UPDATED` without sending. A
sent request clears the list before `CSRequestCommonFarmList`. The Bevy side uses the character
state's instance id for the gate and a 2 s receipt clock.

Runtime entries (`FUN_390324C0` / `FUN_39035620`) carry `name`, `currentPhaseName`, `doodadName`,
`itemType`, `leftGrowthTime`, `leftProtectTime`, `location`, `vPosX`, `vPosY`, `farmType` and
`growUp`.

* The packet's first field is a **`common_farms.id`**; `FUN_397f2750` gives the row and its
  `farm_group_id` keys the runtime entry (`GetRuntimeCommonFarmDoodadsInfo` filters by that group).
  `farmType` is the packet's common farm id. Entries whose common farm, farm group, doodad or func
  group is unknown are skipped.
* `leftGrowthTime` and `leftProtectTime` are **seconds**. Growth is `time_left` plus the rounded
  delays of the following `DoodadFuncGrowth` phases (`FUN_3983c1b0` / `FUN_3983c0e0`), scaled by
  the zone-climate flag (`FUN_39032470`): 1.0 normally, 0.7 when the doodad's
  `doodad_almighties.climate_id` is 1 (`DAT_39ac318c`, "all") or is in the zone's climate list.
  Protect is
  `common_farms.guard_time * 0.001` minus the seconds since `plant_time`, floored at 0
  (`FUN_3902fb80`). The getter subtracts whole seconds since the list arrived.
* `growUp` means **fully grown**: `FUN_3983c170(func_group)` has no further growth phase
  (`next_phase == 0`, or the phase has no `DoodadFuncGrowth`). `my_farm_info` shows the time only
  while `leftGrowthTime > 0` and not `growUp`, else the grown label.
* `location` is the main-world zone under the crop's packet position (`FUN_3902fb00`), `"unknown"`
  when the world/zone is not loaded.
* `currentPhaseName` is the `doodad_func_groups.name` at descriptor +0x14 (`FUN_390324C0`),
  localized like the phase message; `doodadName` is the doodad template name and `name` the planted
  item's name (else the doodad name).

## Windows and events

* `SCShowCommonFarm` (0x1b0, sent by the `DoodadFuncOpenFarmInfo` doodad func) raises
  `OPEN_COMMON_FARM_INFO(farmType)`; `info.alb` opens `farm_info` and fills it from the getters
  above. The packet's `farmType` is the doodad func's `farm_id`.
* `INTERACTION_END` closes the info window. This module mirrors the interaction by ending it when
  the player walks more than 10 m from the shown farm doodad (the native interaction reach is not
  recovered: **capture-pending**).
* `SCResponseCommonFarmList` (0x1b1) is folded into the runtime list and raises
  `COMMON_FARM_UPDATED` (0x101); `my_farm_info.alb` refreshes its tabs from it and asks again with
  `RequestRuntimeCommonFarmDoodadInfo`.
* `SCIsUnitInFarm` (0x1c9) is stored as the unit farm type (`FUN_391E0C80`, `DAT_39adb270`). The
  native has no Lua getter for it and 1.2's farm windows never read it.
* `my_farm_info.alb` registers `ToggleCommonFarm` as `ADDON:RegisterContentTriggerFunc(
  UIC_MY_FARM_INFO, ...)`; the `toggle_common_farm_info` hotkey and the main menu's farm entry
  (`MAIN_MENU_INFO`) go through `ADDON:ToggleContent` / `ToggleContent` (see
  `aa-ui/src/content.rs`).

## Open

* The farm-area drawable of `worldmap.ShowCommonFarm` and `SetCommonFarmDrawable`
  (`FUN_3926DE70`, `FUN_392D48C0` / `FUN_392D4F20`) is not drawn: aa-ui still treats
  `ShowCommonFarm` as a no-op (`crates/aa-ui/src/lua/worldmap.rs`) and only stores the drawable.
* The runtime list is rebuilt from the last `SCResponseCommonFarmList`. The getters count the
  seconds down on every call, and `my_farm_info.alb`'s `OnUpdate` refreshes the visible tab each
  frame, so the row text follows without a new packet until the list is cleared or refreshed.

## Tests

`cargo test -p aa-client hud::farm` (with `AA_PAK` and `AA_DB` for the real-script tests): the getter
shapes, the runtime entry (seconds, following growth phases and climate scale, `growUp`, common
farm id, phase name, crop zone), a list event raising `COMMON_FARM_UPDATED`, `Show` raising
`OPEN_COMMON_FARM_INFO` and walking away raising `INTERACTION_END`, the request gate (send once,
sent-request clearing, 2 s debounce, instant-zone refusal clearing and raising the update), and the
original `commonfarm` scripts opening `farm_info`, toggling `myFarminfo` through the main menu
action and rendering a growing crop's time next to a grown crop's label.

The leave-distance check keeps both the received stand position and `HudData::pos` in game
coordinates. The farm receipt and refresh clocks use `web_time::Instant` for the browser build.
