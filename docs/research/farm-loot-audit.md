# Plantable doodad loot audit (research S8)

Test: `crates/aa-client/src/ingame/farm_audit.rs`

```
cargo test -p aa-client --bin aa-client every_plantable -- --nocapture
```

It needs `AA_DB`; without it the test skips. It walks every doodad in `item_spawn_doodads` (335 on the 1.2 data), its
func groups, both `doodad_funcs` and `doodad_phase_funcs`, and resolves each loot func:

* `DoodadFuncLootPack` -> `doodad_func_loot_packs` -> `loots` -> `items`;
* `DoodadFuncLootItem` -> `doodad_func_loot_items` -> `items`.

`DoodadFuncFruitPick`, `DoodadFuncFakeUse` and `DoodadFuncCutdowning` carry no item id; their loot is in the
`next_phase` group. The main walk already scans every group of each plantable, and a separate pass follows those three
transitions explicitly: 9 loot groups and 9 loot rows on the 1.2 data (checked with the same item lookup).

Result on the 1.2 data (2026-10-31): 335 doodads, 352 loot packs, 1,563 loot rows, three missing item references.

| Doodad | Func group | Func | Func id | Missing item |
|---|---|---|---|---|
| 8071 | 22507 | LootItem | 3104 | 34156 |
| 8072 | 22527 | LootItem | 3106 | 34157 |
| 8073 | 22547 | LootItem | 3108 | 34158 |

The test pins that set, so a new broken loot row fails it. The same data has 85 `item_spawn_doodads` rows whose source
item and 32 whose doodad template are absent; those are not loot rows and are not pinned by this audit.
