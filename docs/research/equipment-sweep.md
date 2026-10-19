# Equipment attachment sweep

Systematic check of how every equippable visual attaches to the eight playable bodies (4 races x 2
genders: model ids 10/11 nuian, 16/17 elf, 18/19 hariharan, 20/21 ferre). Produced by
`crates/aa-anim/examples/equip_sweep.rs`; contact sheets in `shots/aa-anim/equip/` (git-ignored,
about 260 MB), raw findings in `data/extract/aa-anim/equip_sweep/` (git-ignored: `findings.tsv`,
`totals.tsv`, `sample.tsv`, `report_tables.md`).

Companion docs: `docs/research/equipment-attach.md` (rules / fixes), `docs/research/equipment-data.md`
(data tables).

## Method

Two modes of the same example.

1. `--analyze` (no window). For **every distinct visual** of every slot (not a sample: 3,000+ visuals,
   about 6,000 items share them) on all 8 models it:
   * resolves the item the way `aa-anim` does (`CharacterDesc::resolve*`, `.cdf` / `.chr` / `.mtl`
     paths) and loads mesh, skeleton and materials through `aa_cry`;
   * emulates the renderer on the CPU: a part is skinned to the race's base skeleton by controller id,
     bones the base lacks get a joint under their parent (an orphan root bone stays at the model
     root), shared bones use the base bind poses;
   * skins the vertices in four poses (bind, idle `fist_ba_relaxed_idle`, sit
     `fist_ac_sit_down_loop` 60%, leap `all_co_sk_leapattack` 40%) and compares them with the nude
     body / face / hair of the same model (spatial hash of the body cloud);
   * runs `aa_anim::cut::cut_hulls` to see which body triangles the garment hides;
   * for weapons replicates the `aa-client` placement (`weapons.rs`: sheath bones `item_back_c`,
     `item_side_l/r`, `item_bow_l`, `item_arm_l`, `item_staff_l`; hand bones `item_hand_l/r`; the
     `item_assets` offsets/rotations and worn / drawn scales) and the `CA_BONE` object attachments of
     `.cdf` characters (`attachments.rs`).
   Each finding is one line per item x model with `severity` (fail / warn / info), category, slot,
   model, item id, name, level, an `npc` flag (npc / test / monster items, id range or name), the visual path
   and a one line detail.
2. `--sheets` (GPU window). One item per cell, 8 columns x 4 rows (32 items) per sheet, front and side
   (back view for the back / weapon slots), poses idle + action (+ sit for legs / feet / back / costume).
   Weapon slots show the weapon sheathed in `idle` and in the hand in `action`. Every cell is labelled
   with the item id and level; a cell marked `no_visual` has no model for that race / gender (the
   body stays naked, correct for items that are texture-only or have no variant).
   `tools/equip_sweep/run_sheets.py` drives it per model x slot (waits for 6 GB free RAM);
   `tools/equip_sweep/stitch.py` stitches the rows into a labelled JPEG (2400 px wide).

Item selection (`sample.tsv`): per slot at least 30 (40 for most slots) items spread evenly over
flavour / level / grade and set membership. The sheets show the first 32 of them (costume: all 78
costume visuals in three sheets `a`/`b`/`c`; percussion 12; waist none, see below). The static
analysis covers all visuals, not only the sampled ones.

### Data facts that shape the sweep

* Slot ids (`item_armors.slot_type_id`): head 1, chest 3, waist 4, legs 5, hands 6, feet 7, back 9
  (capes and wings), underwear 13, costume 31 (one-piece costume, "cosplay" body replacement).
* Waist (belts) has 11 visuals and **no player visual**: the 11 rows are saddles / mount gear and all
  8 models have no mesh for them. Arms / neck / ears / rings have no mesh at all. There is no shoulder
  slot in 1.2. Underwear is a texture swap only (`texture_only_visual`, 1 visual).
* Item grade is almost always -1 in this database, so "spread over grade" collapses into level and
  flavour (a flavour is a recolor / variant with the same mesh); set items are included by id.
* `no_visual_variant` (info): about 2,450 findings are items whose visual simply has no model for
  that race / gender. 58 of 401 items are player items; these are expected (race locked gear).

## Totals per slot and model

Each cell: distinct visuals that have a mesh for the model / how many of them have at least one failing
check (`fail`; warn / info not counted). `visuals` is the number of distinct visuals, `items` the number
of items sharing them.

| slot | visuals | items | of which npc/test | nu_m | nu_f | el_m | el_f | ha_m | ha_f | fe_m | fe_f |
|---|---|---|---|---|---|---|---|---|---|---|---|
| head | 472 | 888 | 204 | 420 / 13 | 428 / 19 | 415 / 15 | 412 / 10 | 417 / 10 | 420 / 10 | 413 / 12 | 417 / 7 |
| chest | 506 | 929 | 255 | 433 / 41 | 431 / 41 | 406 / 37 | 400 / 32 | 424 / 35 | 424 / 32 | 410 / 42 | 408 / 30 |
| waist | 11 | 23 | 3 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 |
| legs | 373 | 821 | 162 | 340 / 25 | 318 / 14 | 325 / 10 | 311 / 15 | 329 / 4 | 317 / 14 | 326 / 5 | 314 / 4 |
| hands | 268 | 625 | 92 | 251 / 21 | 242 / 13 | 236 / 13 | 234 / 11 | 236 / 11 | 232 / 12 | 234 / 12 | 232 / 11 |
| feet | 335 | 739 | 130 | 282 / 15 | 281 / 73 | 267 / 11 | 264 / 59 | 276 / 10 | 278 / 34 | 265 / 10 | 271 / 9 |
| back | 44 | 162 | 8 | 41 / 8 | 40 / 16 | 43 / 9 | 41 / 8 | 41 / 7 | 41 / 8 | 40 / 1 | 40 / 1 |
| costume | 78 | 222 | 0 | 73 / 43 | 74 / 43 | 70 / 46 | 74 / 42 | 71 / 39 | 74 / 43 | 70 / 45 | 73 / 39 |
| underwear | 1 | 1 | 0 | 1 / 0 | 1 / 0 | 1 / 0 | 1 / 0 | 1 / 0 | 1 / 0 | 1 / 0 | 1 / 0 |
| main1h | 272 | 445 | 46 | 272 / 17 | 272 / 19 | 272 / 15 | 272 / 19 | 272 / 18 | 272 / 18 | 272 / 19 | 272 / 19 |
| twohand | 526 | 934 | 78 | 526 / 419 | 526 / 419 | 526 / 419 | 526 / 419 | 526 / 419 | 526 / 419 | 526 / 420 | 526 / 418 |
| sword_main | 425 | 759 | 101 | 425 / 31 | 425 / 54 | 425 / 31 | 425 / 57 | 425 / 30 | 425 / 35 | 425 / 60 | 425 / 27 |
| sword_off | 425 | 759 | 101 | 425 / 29 | 425 / 31 | 425 / 29 | 425 / 31 | 425 / 30 | 425 / 33 | 425 / 58 | 425 / 24 |
| bow | 105 | 203 | 12 | 105 / 2 | 105 / 2 | 105 / 2 | 105 / 2 | 105 / 2 | 105 / 2 | 105 / 2 | 105 / 2 |
| shield | 99 | 216 | 15 | 99 / 6 | 99 / 6 | 99 / 5 | 99 / 6 | 99 / 5 | 99 / 6 | 99 / 6 | 99 / 6 |
| string | 50 | 129 | 4 | 50 / 2 | 50 / 2 | 50 / 2 | 50 / 2 | 50 / 2 | 50 / 2 | 50 / 2 | 50 / 2 |
| percussion | 12 | 30 | 0 | 12 / 0 | 12 / 0 | 12 / 0 | 12 / 0 | 12 / 0 | 12 / 0 | 12 / 0 | 12 / 0 |
| tube | 50 | 102 | 4 | 50 / 0 | 50 / 0 | 50 / 0 | 50 / 0 | 50 / 0 | 50 / 0 | 50 / 0 | 50 / 0 |

Slot names: `sword_main` / `sword_off` are the swords / axes of holdable class 17 worn in the main
hand and in the off hand; `main1h` maces / spears / wands (14); `twohand` two-handed (16); `string`,
`percussion`, `tube` the instruments (holdable 21 by `holdables.code`).

## Failure categories ranked by count

Counts are findings (item x model; one item on 8 models = 8). "Items" are distinct items and, in
brackets, the ones that are not npc / test / monster items.

| # | category | sev | findings | items (player) | where |
|---|---|---|---|---|---|
| 1 | `sheath_horizontal` | fail | 3557 | 497 (430) | twohand 3176, sword_main 194, sword_off 181, shield 6 |
| 2 | `missing_file` | fail | 661 | 132 (8) | chest 276, hands 100, feet 86, head 56, legs 47, sword_main 40 |
| 3 | `bad_size` | fail | 644 | 60 (46) | twohand 415, sword_main 64, sword_off 64, bow 32, string 32, main1h 24 |
| 4 | `floating` | fail | 561 | 111 (82) | twohand 271, main1h 135, sword_main 104, sword_off 43, back 7 |
| 5 | `no_cut_hull` (costume) | fail | 314 | 48 (48) | costume 314 (nude body shows under the costume) |
| 6 | `weapon_cdf` | fail | 200 | 25 (24) | twohand 64, sword_main 32, sword_off 32, shield 32, bow 16, string 16 |
| 7 | `weapon_off_body` | fail | 199 | 21 (12) | twohand 183, sword_main 8, sword_off 8 |
| 8 | `stretched` | fail | 198 | 103 (57) | feet 126, legs 44, costume 12, chest 10, hands 6 |
| 9 | `pose_lag` | fail | 105 | 29 (24) | back 45, head 43, feet 9, chest 4, costume 4 |
| 10 | `no_material` | fail | 29 | 5 (5) | shield 16, costume 13 |
| 11 | `subset_no_material` | fail | 10 | 10 (9) | head 9, costume 1 |
| 12 | `missing_texture` | fail | 7 | 1 (0) | back 7 |
| 13 | `off_axis` | fail | 4 | 4 (2) | head 4 (fe_m only) |
| 14 | `bad_vertices` | fail | 3 | 3 (2) | costume 2, chest 1 |
| 15 | `no_cut_hull` (other slots) | warn | 58 | 13 (5) | hands 42, feet 14, chest 2 |
| 16 | `extends_beyond_body` | warn | 17 | 11 (9) | feet 12, head 5 |
| 17 | `cut_hole` | info (experimental) | 4689 | 1191 (791) | chest 2015, legs 1115, feet 841, hands 709 |
| 18 | `no_visual_variant` | info | 2451 | 401 (58) | chest 712, feet 496, head 434, legs 404, hands 247 |
| 19 | `texture_only_visual` | info | 8 | 1 (1) | underwear 8 |

Checked and found **0 hits** (each probe is run for every visual on every model):

* item bones missing in the body skeleton / bones "remapped to root" in a harmful way
  (`bones_orphan`, `bones_ride`): none; armor parts bind cleanly to the base skeleton by controller id;
* bind-pose shift when the animated pose is evaluated (`bind_shift`): none;
* unskinned or rigid meshes in a skinned part (`unskinned`, `rigid_in_skin`): none;
* pieces at the model origin (`at_origin`) and armor bbox in the wrong region (`wrong_region`): none;
* `.cdf` object attachments (`attach_unbound`, `attach_model_missing`, `attach_off_body`): none;
* armor `pose_lag` (part stays in the bind pose while the body animates): only the cloak / headgear
  cases below, no whole garment.

So the data side of armor attachment is healthy. What fails is (a) weapon placement in the client
rules, (b) costumes not replacing the body, (c) `.cdf` weapon visuals, (d) a few missing files.

### 1. `sheath_horizontal`: two-handed weapons lie across the back, 1h axes stick out at the hip

3557 findings; 2736 of them are player two-handers. The sheathed weapon is placed on `item_back_c`
(two-handers) or `item_side_l/r` (swords, axes) so that its long axis is 80-90 degrees from vertical.
Visually confirmed:

* two-handers: maces, hammers, axes and greatswords are horizontal across the shoulder blades and poke
  out of the body on one side (the mesh long axis is the model's Y; the sheath rotation does not turn it
  upright); staffs and spears come out diagonal and look acceptable. Sheet:
  `shots/aa-anim/equip/twohand_nu_m_a_idle_back.jpg` (and `_side`, `_front`, all 8 models
  `twohand_<race>_<g>_a_idle_*.jpg`). Examples: 5936 "첫걸음 전투둔기"
  (`objects/item/mace_2h/mace_2h_16_2/mace_2h_16_2.cgf`, 88 degrees, 1.30 m long), 19357
  (`mace_2h_46_5.cgf`), 31130 "베로에 양손둔기" (`mace_2h_ori_1.cgf`), 6052, 21242, 20410, 31341.
* one-handed axes on the hip: the lower row of `shots/aa-anim/equip/sword_main_nu_m_a_idle_side.jpg`
  (31133, 22974, 23316, 21977, 20302, 20311, 34692) sticks forward horizontally out of the hip; swords
  in the same sheet hang correctly. Same for `sword_off_*`. All are `objects/item/axe_1h/axe_1h_*.cgf`
  (long axis 80-83 degrees from vertical on `item_side_l`): 22879 "부관 윌리엄의 한손도끼"
  (`axe_1h_26_1`), 22880 (`axe_1h_16_3`), 21964 (`axe_1h_21_7`), 22974 (`axe_1h_26_8`), 23189
  (`axe_1h_31_8`), 21965 (`axe_1h_21_6`), 20294 (`axe_1h_46_12`), 23253 (`axe_1h_31_9`).
* `floating` (561): spears sheathed on `item_side_l` hover 13-15 cm off the hip (main1h 5729
  "첫걸음 창" `spear_1h_16_2.cgf`, 21488, 21792, 22883, 20374, 20375); the same slot (sword, two-hand)
  accounts for most of the rest.

Where to fix: `aa-client/src/ingame/weapons.rs` (sheath bone, offsets and rotation per weapon type and
the DB `item_assets` offsets); the weapon model's own axis differs between types (mace / axe along Y,
sword along Z).

### 2. `missing_file`: visual path not in the pak (661 findings, 132 items, 8 are player items)

Mostly test / npc items; the player ones:

| slot | item | name | path | models |
|---|---|---|---|---|
| head | 15979 | 일리온 군의 머리띠 L6 | `objects/characters/nuian/male/cloth/cloth019/nu_m_hm_cloth019.chr` | 6 |
| chest | 16297 | test_chest_red001 | `objects/characters/nuian/male/metal/metal002_va001/nu_m_ar_metal002_va001.chr` | 2 |
| legs | 14846 | cloth본 테스트용 롱치마 | `.../nuian/male/cloth/cloth_test/nu_m_pt_cloth_test.chr` | 1 |
| hands | 14790 | cloth본 테스트용 망토 | `.../nuian/male/cloth/cloth_test/nu_m_gv_cloth_test.chr` | 1 |
| legs | 2689 | 해제 | `objects/characters/elf/male/light_armor/light_armor002_co01/el_m_pt_lightarmor002_co01.chr` | 4 |
| sword_main/off | 15276 | 컷신용 - 마력이 담긴 나뭇가지 | `objects/item/cutscene/gw_palnts_item_cutscene.cgf` | 8 |
| shield | 27793 | 단합의 방패 | `objects/item/shield/shield_ucc_1/shield_ucc_1.cgf` | 8 |

The item is shown naked / empty-handed. Nothing to fix in the client except choosing a fallback.

### 3. `bad_size`: absurd bounding boxes (644 findings, 60 items)

* Weapon `.cdf` visuals (see 6): the loadable bbox is 1-2 cm because only the empty `real_torch`
  placeholder mesh is found.
* Fishing rods (twohand 27308-27312, 27367-27370, 29702, 30584), `objects/env/01_nuia/001_ndeco/fish/
  ndeco_fishrod_*.chr`, 3.1-3.3 m long (they are props, drawn at prop scale), and two-hand spears
  `spear_2h_ean_1..3` (31383-31385, 2.94 m) on 5 models: much longer than the body. Fishing rods also
  trip `weapon_off_body` (the bbox centre is 1.3 m to the side of the hand): they probably need the
  rod / hand rotation of the real client.
* Pepe / Yata costumes 34308, 34800 (`nu_m_sk_fepee001`, 1.22x1.04x2.59 m), 30729, 30730
  (`el_m_sk_yata001`), 30731 (`fe_m_sk_yata002`): mascot costumes, large by design (visible in
  `costume_*_a_idle_front.jpg`, e.g. 30731 at the top right on the nu_f sheet). Not a bug.

### 4. Costumes do not replace the body (`no_cut_hull` fail, 314 findings, 48 costumes)

One-piece costume (slot 31) that has no Cut hull and does not replace the body: the nude body stays
under it. `aa-anim` `resolve` drops the Body only for the login-stage slot `cosplay`; the in-game
costume item (slot 31) keeps it. Visible on almost every costume sheet:
`shots/aa-anim/equip/costume_nu_f_a_idle_front.jpg` (maid dresses 29845-29847, sailor 28703 / 29003,
14359 "sk_hariharan001" shows dark rectangular patches on legs and torso where the body skin shows
through the stockings, 28902 / 28998 / 29950 show the chest through cut-outs). Close-up:
`shots/aa-anim/equip/closeup/costume_nu_f_dbg_idle_front.jpg`. Examples: 14359 (`nu_m_sk_hariharan001`),
16066 (`nu_m_sk_lucius001`), 18200 `sk_library001`, 25281 `sk_library003`. Likewise 42 hands items
(jelly wristbands 29137-29140, `nu_m_1gv501.chr`) and 14 feet items (28759 `nu_m_1bo105.chr`) have no
Cut hull (`warn`: the body is not hidden but the item is a small add-on).

Where to fix: `aa-anim` `character.rs` `resolve` (drop the Body for slot 31 as well, or add the Cut hull
of the costume).

### 5. `weapon_cdf`: weapons whose visual is a `.cdf` are not drawn (200 findings, 25 items)

A `.cdf` (character definition, e.g. a weapon with attached parts) is not loaded as a mesh by the
weapon code, so the weapon is missing; the code falls back to a 2 cm `real_torch` placeholder. 24
player items, all of the "찬란한 / 영원한" (radiant / eternal) L49 tier plus the monster set:

| slot | items |
|---|---|
| main1h | 24362 횃불 (`etc/torch/real_torch.cdf`) |
| twohand | 35523, 35524 (`mace_2h_36_7_v2/v3.cdf`), 27336, 27342 (`staff_2h_40_10/11.cdf`), 28905 (`staff_2h_dragon.cdf`), 27335, 27341 (`axe_2h_41_9/10.cdf`), 18663 (`blade_2h_36_1_e.cdf`) |
| sword_main / off | 18661, 18662, 27334, 27340 (`blade_1h_*_e.cdf`, `sword_1h_41_12/13.cdf`) |
| bow | 27337, 27343 (`bow_36_12/13.cdf`) |
| shield | 27338, 27344 (`shield_41_10/11.cdf`, no .mtl either), 30060 (`shield_gorgon_002.cdf`) |
| string | 27339, 27345 (`ins_s_36_6/7.cdf`) |

Visible as empty hands (e.g. 27339 has no instrument in `string_nu_m_a_action_front.jpg`). Where to
fix: `aa-client` `weapons.rs` (load the `.cdf` via `aa_anim::Cdf` / the attachment path as in
`attachments.rs`).

### 6. `stretched` / `pose_lag`

* `stretched` (198; 57 player items): edges more than 3x their bind length in a pose.
  `leap` on feet (`nu_f_1bo101.chr`, `nu_f_1bo501.chr`: items 6258, 19396, 23386, 23391; edges up to
  1.4 m) is probably an artifact of the leap clip driving bones the part has but the skeleton lacks;
  `sit` on legs (`nu_m_1pt101.chr`: 27998, 27804, 8 edges up to 0.20 m), hands 31958
  (`nu_m_2gv302.chr`, leap, 2.14 m), costume 29019 (`nu_m_sk_beggar001`, sit, 42 edges). Not verified
  on the sheets (the leap pose is analysis only).
* `pose_lag` (105; 24 player items): back (capes, `nu_m_1cp303.chr`: 34094, 28433; 34% of the vertices
  that touch the body lag by up to 20 cm in leap: that is expected for a cape on a physics-free rig) and
  head (`nu_m_4hm112.chr`: 27786 "영원한 세계", 18795 "글랑블루의 잠수모": the part lags up to 17 cm in sit).
  The back sheets (`back_*_a_sit_back.jpg`) show capes and wings sitting correctly on the shoulders.

### 7. Smaller categories

* `no_material` (29): costume 29901 (`nu_m_sk_enginer002.mtl` missing, 8 models), 34277
  (`nu_f_sk_whiteday002.mtl`, 4), 34276 (`ha_m_sk_whiteday001`, 1); shields 27338, 27344 (`.cdf`).
* `subset_no_material` (10): head `nu_f_hm_cloth173.chr` items 31530, 31537, 31544, 31551 (mtl of
  another cloth set lacks sub-material 1: that part of the headband is not drawn).
* `missing_texture` (7): back 18716 "npc cp_cloth010" (`nu_m_1cp101_c_v01100_ddn.dds`, npc item).
  Also a white (untextured) bow 27062 on the bow sheet.
* `off_axis` (4): head `fe_m_4hm112.chr` (27786, 18795, 18806): the bbox centre is 0.73 m beside the
  body; the ferre male variant of the diving helm is offset (the other races are fine).
* `bad_vertices` (3): `el_m_sk_uniform001.chr` (27561, 27562, 2 vertices NaN) and
  `nu_m_ar_npc002.chr` (npc item 25042).
* `extends_beyond_body` (17, warn): feet 12, head 5.
* `cut_hull` (info, 4689): the experimental check "Cut hull hides body triangles that have no garment
  surface within 4 cm" (the body would show a hole). It could not be validated against the real client
  and is therefore demoted to `info`; treat the count as a hint, not a failure.

## Sheets

Naming: `shots/aa-anim/equip/<slot>_<race>_<g>_<a|b|c>_<pose>_<view>.jpg`, race `nu` / `el` / `ha` /
`fe`, gender `m` / `f`. Poses `idle`, `action` (armor: `onehand_ba_combat_idle`; weapon slots: combat
idle with the weapon in the hand), `sit`. Views `front`, `side`, `back` (back only for the back /
weapon slots). `<slot>_<race>_<g>_<tag>.tsv` lists the cells (row, col, item, level, status, name).
816 sheets (all 17 non-empty slots x 8 models). Useful starting points:

* `costume_nu_f_a_idle_front.jpg`, `costume_*_{a,b,c}_*`: costumes with the body showing.
* `twohand_nu_m_a_idle_back.jpg`, `twohand_*_idle_side.jpg`: horizontal weapons.
* `sword_main_nu_m_a_idle_side.jpg`, `main1h_*_a_idle_side.jpg`: hip axes / spears.
* `back_nu_f_a_sit_back.jpg`: capes and wings (clean).
* `head_*`, `chest_*`, `legs_*`, `hands_*`, `feet_*`: armor (clean apart from the lists above).
* `string_*`, `percussion_*`, `tube_*`, `bow_*`, `shield_*`: instruments in the hand, bows and
  shields on the back / arm.

Visual pass: armor (head / chest / legs / hands / feet) fits the body in the idle, action and sit
poses on all 8 models; no floating, stretched or origin pieces were visible apart from the
categories above. Instruments are held in the left hand correctly. Sheet limitation: the last column
of the `sit` side sheets (the seated body is offset from the cell centre) is partly cut off at the
sheet edge (e.g. `legs_ha_m_a_sit_side.jpg`), and the `action` / `idle` front sheets for chest crop
the legs by design (chest framing).

## Not covered / limitations

* Waist / belt, arms, neck, ears, ring slots: no player visuals exist in 1.2 data.
* The leap pose is analysis only (no sheet); sit is shown for legs / feet / back / costume.
* Hair / face parts and headgear interplay (hair hidden under helmets) are not in this sweep.
* Mounts, gliders and vehicles are not equipment slots here.
* The analysis emulates the renderer's skinning on the CPU; GPU-side differences (alpha, shaders) are
  only visible on the sheets.
* `cut_hole` is experimental, see above.

## Reproduce

```text
# static analysis (about 4 minutes, release build is faster)
cargo run -p aa-anim --example equip_sweep -- --analyze [--slot costume] [--model 10]
python -I data/extract/aa-anim/agg2.py            # tables like the ones above (scratch script, not committed)

# contact sheets: copy the built exe aside so cargo stays free for others
cp target/debug/examples/equip_sweep.exe data/extract/aa-anim/equip_sweep_sheets.exe
python -I tools/equip_sweep/run_sheets.py data/extract/aa-anim/equip_sweep_sheets.exe [--slots feet] [--models 10,11]
# a single slot
cargo run -p aa-anim --example equip_sweep -- --sheets --slot twohand --model 10 --poses idle,action --out shots/aa-anim/equip/_raw --tag a
python -I tools/equip_sweep/stitch.py shots/aa-anim/equip/_raw shots/aa-anim/equip
```

`--frame centre_y,height,cell_width` overrides the per-slot camera framing, `--items id,id` renders
specific items.
