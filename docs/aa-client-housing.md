# aa-client: housing

Houses in the world, the builder that places a new one and the original housing windows (`x2ui/housing`: the build
window, the construction window, the house window and its sale form, unchanged Lua) over aa-net's housing protocol
(`crates/aa-net/src/housing.rs`).

Code:

| File | What |
|---|---|
| `crates/aa-client/src/ingame/housing.rs` | module root, plugin, `claims_item` (a design item enters the builder, it is not cast) |
| `ingame/housing/data.rs` | designs, construction steps, design items, prefabs, housing land polygons and groups (loaded once on a thread) |
| `ingame/housing/prefab.rs` | the parts (brushes, entities) of a house prefab |
| `ingame/housing/render.rs` | house units drawn at the model of their construction stage |
| `ingame/housing/placement.rs` | the builder: ghost, rotation, the placement checks, `CSConstructHouseTax` / `CSCreateHouse` |
| `crates/aa-client/src/hud/housing.rs` | `X2House`, the housing events, opening the windows at a house, own houses on the map |
| `crates/aa-client/src/hud/housing/fixture.rs` | the owner-menu self-check: original scripts over synthetic `SCMyHouse` / `SCHouseTaxInfo` |
| `crates/aa-client/examples/house_owner.rs` | runs the self-check (`--state paid|due|overdue`, headless with `--mute`) |
| `crates/aa-client/examples/house_sale.rs` | two-session sale/cancel/buy/demolish self-check over synthetic receipts |

Tests: `cargo test -p aa-client housing` (land polygons, zone origins, the design / step / prefab tables of the real
database, prefab parsing, the placement checks (the rotated model box and lowest terrain sample, an existing house as
0x70, unrestricted and zero-footprint designs, the player's own units and house-attached doodads), the locks with
their seconds, the `X2House` getters and setters, taxes and sale certificates, the build action of a stage, the map
icons, the interaction view type, the permission gates, the deco/actability counters, the raw tax weeks, the sale gates and receipt order, the sale form state, the fail events, and that no
housing system has conflicting parameters). The owner menu has a self-check
example: `cargo run -p aa-client --example house_owner -- --headless --mute --state paid --exit-after 2` (needs
`AA_PAK` and `AA_DB`; it asserts the `HOUSE_INTERACTION_START` / `HOUSE_TAX_INFO` tuples with a Lua spy, the one
`CSChangeHousePermission`, `GetHouseDecoCount`, and the paid/due/overdue `contentTaxInfo` texts).

## Data

* **Designs** (`housings`): category, `garden_radius`, main model; `housing_build_steps` per design (model, skill,
  `num_actions`), the step skill's `consume_lp` (labor) and its `skill_effects` consumed item (the stage material).
  `item_housings` maps a design item to its design, `item_housing_decorations` a furniture item to its decoration.
* **Models**: a house model is a prefab (`models.sub_type` PrefabModel): `prefab_elements` (state 1) gives
  `prefab://<file>/<name>`; the prefab's `<Objects>` hold `Brush` (`Prefab=` the geometry, `Material=` without
  `.mtl`, `Pos`, `Rotate` as `w,x,y,z`, `Scale`) and `Entity` / `GeomEntity` parts (`object_Model`, `Geometry=`).
* **Housing land**: the client's `worlds/main_world/level_design/zone/<key>/client/housing_area.xml`. A shape's points
  are relative to its entity `Pos`, plus `(zone originX/Y + cellX/Y) * 1024` from `world.xml`'s `<Zone>`; its `value1`
  is the `housing_areas.id` whose `housing_groups` row says which categories (`housing_group_categories`) may stand
  there, and the `houseless` / `existing_category_id` rules.

## Houses in the world

The server announces every house as a unit (`SCUnitState`, kind housing, any owner). `units.rs` leaves those alone;
`render::sync_houses` keeps one entity per house within 400 m: the parts of the model of its current stage
(`HouseSnapshot.model_id`, else the unit's model, else the design's step model for `current_actions`) are prepared on
the workers and spawned under a root at the unit's position and yaw. `SCHouseBuildProgress` carries the next stage's
model: the house is redrawn. A finished house sends 0 of 0 actions (`SCHouseState`).

## Shipyards

A shipyard unit (`SCUnitState` kind shipyard) is drawn by `ingame/shipyard.rs` at the model of its construction
stage (`SCShipyardState` 0x1aa, `shipyard_steps` by position); `step == steps.len()` is the finished hull
(`shipyards.main_model_id`, the entry `FUN_394d8600` appends) and step 1000 the launch ceremony
(`shipyards.ceremony_model_id`). Like houses, shipyards are left out of the unit views (`units.rs`), so
`Views::unit_entity` answers `None` for one: the portal effects, speech bubbles, posture clips and mount/deck
lookups that use it never apply to a shipyard, and no pick box or selection ring is drawn for one. A right click
picks a shipyard by the drawn stage model's bounds
(the hull and its drydock parts), not by `build_radius`; the interaction ends at the same half-size plus the
10 m `LEAVE` margin. `X2House`'s `GetShipyard*` getters answer for the last shipyard interacted
with; the material window is not raised yet (`GetShipyardConstructionStepInfo` is still open).

## The builder (`x2game.dll` `HousingManager`, `3932A3B0` .. `39334460`)

Using a design item does not cast: `placement::start` reads the `SkillPressed` of an item in `design_by_item`
(`ground_target::route` skips it), and the builder runs:

1. **Position**: the ghost (the design's finished model) stands on the terrain under the cursor (a pick of up to
   200 m, `394298C0`). The wheel turns it (`builder_rotate_mode` of `default_binding.g`: the wheel
   `builder_rotate_left/right_normal` 5°, Shift `_large` 45°, Alt `_small` 1°; `settings/input.rs` resolves the mode
   while the builder runs). A left click fixes the spot.
2. **Rotate**: the house turns towards the cursor. A left click asks the server for the tax (`39325F60`:
   `CSConstructHouseTax`), unless the spot is refused: then the refusal is shown (`39428370`) and nothing is sent.
3. `SCConstructHouseTax` raises `HOUSE_BUILD_INFO(design, baseTax, totalTax, heavyCount, normalCount, heavy, deposit,
   item)`; the build window shows it. Start calls `X2House:NextStepFromBuildCheck` (`3932FE60`: `CSCreateHouse` with
   the quoted tax, at most once per 5 s; a second press shows `CANNOT_LOCATE_DOODAD_NOW` with the remaining seconds)
   and the builder ends; Cancel (`PrevStepFromBuildCheck`) returns to Position.

The end of a world load or instance load (`FUN_39426520`, including `SCLoadInstance`) arms the 30 s placement lock
(`FUN_393a6de0`, `_DAT_39ac7748` = 30000 ms; teleport.rs raises `WorldLoadFinished` then): using a design item during
it shows `CANNOT_LOCATE_DOODAD_NOW` with the seconds left (the recapture2 "22 second(s)") and does not start the
builder. The house builder reads the same `plant::PlantLock` as the seed builder, so the two can never disagree. It arms
only at the end of a world or instance load (`WorldLoadFinished` / `OnEnter(InGame)`), not on a short
`SCTeleportUnit` (`FUN_39426520` is the only caller of `FUN_393a6de0`); planting keeps its separate 2 s
post-send cooldown (`0x163`).

A right click goes back a step (Rotate -> Position, Position -> end), Escape ends it. `BUILDER_STEP` (`0x224`) is raised
on every step, `BUILDER_END` (`0x225`) when it ends (the build window closes with it).

The spot is first snapped to the 4 m grid (`39325e30`: the footprint's lower left corner to a whole cell). The
checks (`39333E20`, `39334060`, `39335860` natively) are the original client's, with the server's
`systems/housing/placement.rs` as the cross-check:

* a restricted category (1 and 7..=13, 15..=18) needs the centre's area group to allow the design's category; a
  centre outside every polygon is `HOUSE_CANNOT_LOACATE_INVALID_CATEGORY_AREA` ("Building doesn't fit in this
  area."). Unrestricted categories skip this check, as the client and the server do;
* every 4 m cell of the `n x n` footprint (`n = int((2r + 3) / 4)`, `r` the garden radius) must lie in one
  housing polygon and be free of an existing house, else `HOUSE_CANNOT_LOCATE_INVALID_AREA` ("You can't construct
  a building here."). An existing house on the spot answers 0x70, not the 0x71 house-overlap error (recapture2);
  the placement-mode cell query runs before that test. `n = 0` designs have no footprint and pass; `n >= 12` is
  refused as "house too big" (capture-pending);
* the footprint must not contain a unit or doodad (`HOUSE_CANNOT_LOCATE_OVERLAP_UNIT`; the exact collision shapes
  of mask `0x1f` are capture-pending). The player's own mount, mate and slave are skipped (the server's
  `player_owned`), and so is every doodad attached to a house;
* terrain over the ghost's rotated model box (its four corners and centre, `FUN_39326c10`) may vary by at most
  6 m (`HOUSE_CANNOT_LOCATE_TERRAIN_TOO_LOW`), and the spot's z is the lowest of those samples. `CSConstructHouseTax`
  and `CSCreateHouse` both carry that one z; the capture shows 173.05162 (`37 0d 2d 43`) in both.

The houseless / existing-category rules against the account's other houses are the server's. The refused ghost wears
the red of `materials/special/housing_build_invalid` (Diffuse 0.737, 0.013, 0.013, glow 2). Siege and castle land
rules are left to the server.

The 2026-10-31 original-client capture settles the refusal texts: `HOUSE_CANNOT_LOCATE_INVALID_AREA` is "You can't
construct a building here." and `HOUSE_CANNOT_LOACATE_INVALID_CATEGORY_AREA` is "Building doesn't fit in this area."
The spot Bevy used to accept, (15338.53, 14632.33), crosses area 338's south-east notch; the captured valid spot is
(15328.4, 14631.8), snapped to (15328, 14632).

## Sale, cancel, buy and demolish

The sell form's buttons (`sell_register_window.alb`) call `X2House:SetHouseForSale(gold, silver, copper, name)`
(price = `copper + (gold * 100 + silver) * 100`, `FUN_394799C0`), `CancelHouseForSale` (0x5f) and `BuyHouse`
(0x60). The client gates are `FUN_3932C640` (sell), `FUN_3932C800` (cancel) and `FUN_3932E020` (buy; the handoff
called it `FUN_3932C900`, which is a rendering function - the gate code is at `DEC:658897-659025`):

| Click | Gate, in the native order | `ERROR_MSG` |
|---|---|---|
| Sell | not owner | 0x282 `HOUSE_CANNOT_SELL_AS_NOT_OWNER` |
| | castle (`housings.category_id` 2..=5 or 14) | 0x283 `HOUSE_CANNOT_SELL_AS_CASTLE` |
| | under construction (demolished flag 0 and `current_actions < all_actions`) | 0x29c `HOUSE_CANNOT_SELL_AS_UNDER_CONSTRUCTION` |
| | already for sale (`sell_price != 0`) | 0x286 `HOUSE_CANNOT_SELL_AS_ALREADY_FOR_SALE` |
| | design missing from `housings` (`FUN_3963B390` 0): silent | |
| | `is_sellable` false | 0x284 `HOUSE_CANNOT_SELL_AS_TYPE` |
| | fewer appraisal certificates 28085 than `ceil(price / 100 gold)`, at least one | 0x287 `HOUSE_CANNOT_SELL_AS_NOT_ENOUGH_SEAL` |
| | sell-to name equals my own name | 0x2a9 `HOUSE_CANNOT_SELL_TO_ONESELF` |
| Cancel | not owner | 0x28b `HOUSE_CANNOT_CANCEL_SELL_AS_NOT_OWNER` |
| | nothing for sale (`sell_price == 0`) | 0x28c `HOUSE_CANNOT_CANCEL_SELL_AS_NOT_FOR_SALE` |
| Buy | I am the owner | 0x28f `HOUSE_CANNOT_BUY_AS_OWNER` |
| | nothing for sale | 0x28e `HOUSE_CANNOT_BUY_AS_NOT_FOR_SALE` |
| | designated buyer is someone else | 0x290 `HOUSE_CANNOT_BUY_AS_NOT_DESIGNATED_BUYER` |
| | money < price | 0x291 `HOUSE_CANNOT_BUY_AS_NOT_ENOUGH_MONEY` |

`GetHouseSaleInfo` (`FUN_3947A4D0`) answers `canSell` = `is_sellable` and owner and not a castle (it does not
look at construction or an existing sale: the click's gates do), `canBuy` = on sale and (no designated buyer or
me) - it does not look at ownership, the buy gate refuses the owner. `targetName` is absent without a designated
buyer. `GetHouseSaleItemInfo` (the form's certificate row) returns `itemType` 28085, `needCount` and `hasCount`
(bag).

The receipts (`FUN_3932B680`, `FUN_3932B790`, `FUN_3932B8A0`):
`SCHouseSetForSale` looks the house up by tl and raises `HOUSE_SET_SELL_SUCCESS(houseName)` for its owner even
when the window is closed, then `HOUSE_INFO_UPDATED` when the house is the open one; `SCHouseResetForSale` raises
`HOUSE_CANCEL_SELL_SUCCESS(houseName)` the same way, then `HOUSE_INFO_UPDATED`; `SCHouseSold` updates the owner,
clears the sale, re-requests the open house's tax and raises `HOUSE_INFO_UPDATED` first, then
`HOUSE_SALE_SUCCESS(houseName)` for the previous owner or `HOUSE_BUY_SUCCESS(houseName)` for the new one. The
event ids are 0x109..0x10c (`FUN_39214380`), and the scripts print their texts (`house_sale_success` ...).

The fail events `HOUSE_SET_SELL_FAIL` 0x106, `HOUSE_BUY_FAIL` 0x107 and `HOUSE_CANCEL_SELL_FAIL` 0x108 are
registered by `housing_manager.alb` and printed by `chat_msg_event.alb` (`house_set_sell_fail` ...). Their native
raiser is not visible: the error display at `FUN_39428370` is a packed thunk. Bevy raises the event of the
**pending operation** (`sell`, `cancel` or `buy`) for an `SCErrorMsg` in 0x266 (the buy's
`INVALID_HOUSE_INFO`), 0x282..=0x293 (including the 0x283 castle answer from `set_for_sale`/`buy_house` and the
0x28D cancel DB error), 0x29C or 0x2A9, and clears the pending request; a stale error still shows its own
`ERROR_MSG` text. This trigger is **inferred, capture-pending (C5)**.

`X2House:Demolish` (`FUN_39478740`) casts the skill at the house; the skill id in the data image (`DAT_39ac99e0`)
is **12945**, and `skills.casting_time` is 20000 ms (the 11404 copy has the same interaction 94 and cast time).
`SCHouseDemolished` (`FUN_391D7A40` -> `FUN_3932A300`) sets the house's demolished flag, clears its owner (the 600
sentinel) and owner name and calls `FUN_39326770` with `destroy_effect`, which walks the unit's effect objects and
starts the one of that name. It **keeps the model**: the server keeps the unit as an ownerless, attackable house
(`ops.rs:580-597`). None of the 30 housing prefab XMLs in `game_pak` contains a `destroy_effect` object; the other
sources of that list were not read, so whether 1.2 shows anything at runtime is not settled (capture-pending C5).
Bevy marks the `House` demolished and clears its owner/owner name in aa-net (the snapshot and model stay), the
open window sees no snapshot/owner, and `render::model_of` keeps drawing the demolished house; `drops` only
removes units that are gone or too far.

## Construction

A right click on a house within 6 m starts the interaction (`393BEFD0`): `CSRequestHouseTax`, then
`HOUSE_INTERACTION_START("housing", viewType, alwaysPublic)`; `viewType` comes from `housings.category_id` (7
`"seafarm"`, 8/16/17 `"farm"`, 9/10/11/12/15/18 `"house"`, else nil, `DEC:780640-780672`). `housing_manager.lua` opens
the construction window while the house is being built, the house window once it stands. Walking 10 m away ends it
(`HOUSE_INTERACTION_END`). On a house under construction the right click also casts the stage's build skill
(`housing_build_steps.skill_id`): the captured `CSStartSkill` is 13 bytes with a plain unit target (kind 0 and the
house object id; no BC-house target type), cast time 20.99 s, 25 labor and one stage material per action; the server
answers `SCHouseBuildProgress` -> `HOUSE_STEP_INFO_UPDATED`. The construction window's step list comes from
`GetHouseConstructionStepInfo` (`consumeItemStack`, `isLaborPower`, `step`, `needActions`).

The client's local material check looks only at the bag, as in the capture: without a bag stack it shows
`NOT_ENOUGH_REQUIRED_ITEM` ("Not enough <item>."), but the same click still casts when the material is equipped in
the backpack slot (the server pays it then). One click sends at most one `Command::UseSkill`; the cast bar and labor
bar follow the ordinary `SCSkillStarted` / `SCCharacterLaborPowerChanged` path. The capture covered action 1 only:
actions 2-20, the 10/5/5 boundaries, completion, the cast-bar text and the wrong-skill/out-of-range negatives remain
capture-pending (a second capture is running); the stage models in the example come from `housing_build_steps` and
the finished model from `housings.main_model_id`.

## `X2House`

The getters have no house argument: like the original (`HousingManager + 0x78`, the object id of the house being
interacted with) they answer for the current house.

| Group | Methods |
|---|---|
| House | `IsMyHouse`, `IsHouseUnderConstruction`, `GetHouseName`, `GetHouseOwnerName`, `GetHouseBaseProgress`, `GetHouseRemainProgress`, `GetHousePermission` (server enum + 1, always-public), `GetHouseAllowRecover`, `IsCastle`, `GetHouseDecoCount` (`placed - housing_binding_doodads`, `absolute_deco_limit - housing_binding_doodads`, `FUN_394796B0`) |
| Actability | `GetActabilityDecoCount`, `GetDecoActabilityPoint(i)` (`name`, `decoPoint`), `GetDecoActabilityByType(i)` (totals `type`/`totalDecoPoint`/`totalPoint`, item rows `name`/`decoPoint`), `GetDecoActabilityLimitGroup()` (`name`, `arrangeCount`, `maxCount` from `housing_deco_limit_elems`, or nil without a limit list, `FUN_3947A310`); the house entries group `housing_decorations.actability_group_id` and sum `actability_up`, but only for a design with a limit list and a decoration with a `deco_actability_group_id` (`FUN_3932FD00`); the limit rows count `deco_actability_group_id` |
| Owner | `SetHouseName`, `SetHousePermission` (owner only; one `CSChangeHousePermission` with the server enum, none for an always-public design 0x80, family needs a family 0x178, guild needs an expedition 0x137), `SetHouseAllowRecover` (owner; a toggle, only when it differs), `Demolish` (skill 12945 on the house) |
| Construction | `GetHousingModelName`, `GetHousingModelConstructionInfo`, `GetHouseConstructionStepInfo`, `NextStepFromBuildCheck`, `PrevStepFromBuildCheck` |
| Tax | `CountTaxItemForTax` (one certificate 31891 / 31892 per gold, rounded up; `featureSet.taxItem`), the `HOUSE_TAX_INFO` event |
| Sale | `GetHouseSaleInfo` (native `canSell`/`canBuy`/`targetName`), `GetHouseSaleItemInfo` (one appraisal certificate 28085 per 100 gold, at least one), `SetHouseForSale`, `CancelHouseForSale`, `BuyHouse`, with the gates and events of "Sale, cancel, buy and demolish" |

Events raised: `HOUSE_INTERACTION_START/END`, `HOUSE_TAX_INFO` (raw `weeksWithoutPay`, the native order),
`HOUSE_STEP_INFO_UPDATED`, `HOUSE_BUILD_INFO`, `BUILDER_STEP`, `BUILDER_END`, `UNIT_NAME_CHANGED`,
`HOUSE_PERMISSION_UPDATED`, `HOUSE_INFO_UPDATED`, `HOUSE_SET_SELL_SUCCESS`, `HOUSE_CANCEL_SELL_SUCCESS`,
`HOUSE_BUY_SUCCESS`, `HOUSE_SALE_SUCCESS`, `HOUSE_SET_SELL_FAIL`/`HOUSE_BUY_FAIL`/`HOUSE_CANCEL_SELL_FAIL`,
`HOUSE_DECO_UPDATED`, `CHANGE_ACTABILITY_DECO_NUM` (raised here when
the received actability points change; native `0x23c` is raised by `FUN_3907BA10`, which only resets the struct at
`DAT_39aff990`, so this trigger is capture-pending C7).

## Map

`hud/markers.rs` adds the player's own houses (from the `SCMyHouse` / `SCHouseState` list, wherever they are) with the
icons `markers.lua` already knows: `housing_fishfarm` (category 7), `housing_farm` (16 to 18: scarecrow gardens, the
thatched farm), `housing_house` otherwise. The tooltip is the house name.

## Dev hooks

`HUD.house_interact(obj_id)` opens the windows at a house, `HUD.house_builder(x, y, yaw)` pins the ghost to a game
position, `HUD.house_builder_click()` is a left click in the builder (for `--headless` scripts, where there is no
cursor ray).

## Not done

* Furniture placement (`CSDecorateHouse`): the decoration and actability tables are loaded and the owner menu's
  counters answer; the placement builder (ghost, surface/garden/overlap checks, `CSDecorateHouse`) is slice 7a and
  not implemented. The server attaches the doodad to the house; whether the position is house-local is not verified.
* The actability list order is the attached-doodad object-id order (the doodad store is a map); the native keeps
  placement order. `GetDecoActabilityByType`'s `totalPoint` is the received proficiency point (the native adds its
  equipment bonus too).
* Planting on farms is `ingame/ground_target/plant.rs` (`CSCreateDoodad`).
* Siege / castle land rules, the house's door and stair models, house HP and repair.

## Examples

* `house_build --shot` (needs `AA_PAK` and `AA_DB`): the real area-338 rules refuse (15338.53, 14632.33) and accept
  (15328.4, 14631.8), the Start button's `CSCreateHouse` is the captured 45 bytes, and the stage-0 captured
  `SCMyHouse`/`SCHouseState` body is drawn through `SCHouseBuildProgress` 1083 -> 1084 -> 1085 -> 1082 with the model
  ids asserted, no blank frame and no respawn for a same-model progress.
* `house_cast --mute` (needs `AA_DB` and the pak for the UI): one click sends one captured `CSStartSkill`; without
  the bag material it sends none and shows "Not enough Stone Pack.", and an equipped backpack still casts.
* `house_sale --mute` (needs `AA_PAK` and `AA_DB`): two sessions over synthetic receipts. One click sends one
  `07 00 10 27 00 00 00 00` (`CSSellHouse`, tl 7, 10000 copper, empty sell-to), `07 00` (`CSSellHouseCancel`) or
  `07 00 10 27 00 00` (`CSBuyHouse`); a refused gate sends nothing and shows its `ERROR_MSG` text; the receipts
  raise the events in the native order. The original `sell_register_window.alb` is created over the live getters:
  its register button is off for 0/0/0, on for 1 gold with the seller's two certificates, and off for 201 gold
  (three certificates needed).

## Recovery live check (2026-10-23, superseded)

On the bevy lane, Salonbevy uses Thatched Farmhouse Design 21165 (design 171), pins its ghost at
housing area 338 in zone 179, `(15338.53, 14632.33)`, and advances through position and rotation.
The original Start Construction window receives the server's tax quote: deposit 30, base tax 15,
total 45 certificates, plus the three material stages (10/5/5 actions). Cancel returns to placement,
Escape ends it, and the test character is restored to its starting position. That spot is the one the
2026-10-31 capture shows the original client refusing (the south-east notch of area 338); the builder
checks above now refuse it too. The check sent no create-house request; the 2026-10-31 capture and the
`house_build` example cover the create and stage progress.

The script passes all 20 commands with zero server errors, zero logged errors and the existing allowed
`tab_language` Lua error. Inspected captures are `shots/resume/housing/{before,ghost,quote}.png`;
the camera is inside the large ghost, so these do not establish whole-house model parity. The run logs
roughly 25–40 FPS after streaming. Reproduction: `data/local/resume/housing-preview.txt`, evidence:
`data/resume-housing.log`. The original-client comparison is still open.
