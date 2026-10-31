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

Tests: `cargo test -p aa-client housing` (land polygons, zone origins, the design / step / prefab tables of the real
database, prefab parsing, the placement checks, the `X2House` getters and setters, taxes and sale certificates, the
build action of a stage, the map icons, and that no housing system has conflicting parameters).

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
   the quoted tax, at most once per 5 s, else `CANNOT_LOCATE_DOODAD_NOW`) and the builder ends; Cancel
   (`PrevStepFromBuildCheck`) returns to Position.

A right click goes back a step (Rotate -> Position, Position -> end), Escape ends it. `BUILDER_STEP` (`0x224`) is raised
on every step, `BUILDER_END` (`0x225`) when it ends (the build window closes with it).

The spot is first snapped to the 4 m grid (`39325e30`: the footprint's lower left corner to a whole cell). The
checks (`39333E20`, `39334060`, `39335860` natively) are the original client's, with the server's
`systems/housing/placement.rs` as the cross-check:

* a restricted category (1 and 7..=13, 15..=18) needs the centre's area group to allow the design's category; a
  centre outside every polygon is `HOUSE_CANNOT_LOACATE_INVALID_CATEGORY_AREA` ("Building doesn't fit in this
  area.");
* every 4 m cell of the `n x n` footprint (`n = int((2r + 3) / 4)`, `r` the garden radius) must lie in one
  housing polygon, else `HOUSE_CANNOT_LOCATE_INVALID_AREA` ("You can't construct a building here.");
* the footprint must not overlap another house's snapped axis-aligned footprint (`HOUSE_CANNOT_LOCATE_OVERLAP_HOUSE`)
  or a unit/doodad standing in it (`HOUSE_CANNOT_LOCATE_OVERLAP_UNIT`; the exact collision shapes of mask `0x1f`
  are capture-pending);
* terrain under the footprint's corners may vary by at most 6 m (`HOUSE_CANNOT_LOCATE_TERRAIN_TOO_LOW`).

The houseless / existing-category rules against the account's other houses are the server's. The refused ghost wears
the red of `materials/special/housing_build_invalid` (Diffuse 0.737, 0.013, 0.013, glow 2). Siege and castle land
rules are left to the server.

The 2026-10-31 original-client capture settles the refusal texts: `HOUSE_CANNOT_LOCATE_INVALID_AREA` is "You can't
construct a building here." and `HOUSE_CANNOT_LOACATE_INVALID_CATEGORY_AREA` is "Building doesn't fit in this area."
The spot Bevy used to accept, (15338.53, 14632.33), crosses area 338's south-east notch; the captured valid spot is
(15328.4, 14631.8), snapped to (15328, 14632).

## Construction

A right click on a house within 6 m starts the interaction (`393BEFD0`): `CSRequestHouseTax`, then
`HOUSE_INTERACTION_START("housing", ..)`; `housing_manager.lua` opens the construction window while the house is being
built, the house window once it stands. Walking 10 m away ends it (`HOUSE_INTERACTION_END`). On a house under
construction the right click also casts the stage's build skill (`housing_build_steps.skill_id`): the captured
`CSStartSkill` is 13 bytes with a plain unit target (kind 0 and the house object id; no BC-house target type), cast
time 20.99 s, 25 labor and one stage material per action; the server answers `SCHouseBuildProgress` ->
`HOUSE_STEP_INFO_UPDATED`. The construction window's step list comes from `GetHouseConstructionStepInfo`
(`consumeItemStack`, `isLaborPower`, `step`, `needActions`).

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
| House | `IsMyHouse`, `IsHouseUnderConstruction`, `GetHouseName`, `GetHouseOwnerName`, `GetHouseBaseProgress`, `GetHouseRemainProgress`, `GetHousePermission` (server enum + 1, always-public), `GetHouseAllowRecover`, `IsCastle`, `GetHouseDecoCount` |
| Owner | `SetHouseName`, `SetHousePermission` (sent only when it changed), `SetHouseAllowRecover` (a toggle, only when it differs), `Demolish` (skill 11404 on the house) |
| Construction | `GetHousingModelName`, `GetHousingModelConstructionInfo`, `GetHouseConstructionStepInfo`, `NextStepFromBuildCheck`, `PrevStepFromBuildCheck` |
| Tax | `CountTaxItemForTax` (one certificate 31891 / 31892 per gold, rounded up; `featureSet.taxItem`), the `HOUSE_TAX_INFO` event |
| Sale | `GetHouseSaleInfo`, `GetHouseSaleItemInfo` (one appraisal certificate 28085 per 100 gold, at least one), `SetHouseForSale`, `CancelHouseForSale`, `BuyHouse` (refused with `HOUSE_CANNOT_BUY_AS_NOT_ENOUGH_MONEY` locally) |

Events raised: `HOUSE_INTERACTION_START/END`, `HOUSE_TAX_INFO`, `HOUSE_STEP_INFO_UPDATED`, `HOUSE_BUILD_INFO`,
`BUILDER_STEP`, `BUILDER_END`, `UNIT_NAME_CHANGED`, `HOUSE_PERMISSION_UPDATED`, `HOUSE_INFO_UPDATED`,
`HOUSE_SET_SELL_SUCCESS`, `HOUSE_CANCEL_SELL_SUCCESS`, `HOUSE_BUY_SUCCESS`, `HOUSE_SALE_SUCCESS`,
`HOUSE_DECO_UPDATED`.

## Map

`hud/markers.rs` adds the player's own houses (from the `SCMyHouse` / `SCHouseState` list, wherever they are) with the
icons `markers.lua` already knows: `housing_fishfarm` (category 7), `housing_farm` (16 to 18: scarecrow gardens, the
thatched farm), `housing_house` otherwise. The tooltip is the house name.

## Dev hooks

`HUD.house_interact(obj_id)` opens the windows at a house, `HUD.house_builder(x, y, yaw)` pins the ghost to a game
position, `HUD.house_builder_click()` is a left click in the builder (for `--headless` scripts, where there is no
cursor ray).

## Not done

* Furniture (`CSDecorateHouse`): the decoration tables are loaded, the window's actability calls answer empty. The
  server attaches the doodad to the house; whether the position is house-local is not verified.
* Planting on farms is `ingame/ground_target/plant.rs` (`CSCreateDoodad`).
* Siege / castle land rules, the house's door and stair models, house HP and repair.

## Examples

* `house_build --shot` (needs `AA_PAK` and `AA_DB`): the real area-338 rules refuse (15338.53, 14632.33) and accept
  (15328.4, 14631.8), the Start button's `CSCreateHouse` is the captured 45 bytes, and the stage-0 captured
  `SCMyHouse`/`SCHouseState` body is drawn through `SCHouseBuildProgress` 1083 -> 1084 -> 1085 -> 1082 with the model
  ids asserted, no blank frame and no respawn for a same-model progress.
* `house_cast --mute` (needs `AA_DB` and the pak for the UI): one click sends one captured `CSStartSkill`; without
  the bag material it sends none and shows "Not enough Stone Pack.", and an equipped backpack still casts.

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
