# Equipment data map: item -> model files on a character

Research only (no code changed). Sources: `<server>\Data\compact.sqlite3` (read-only),
`data/pak12.tsv` (pak index; paths there carry a `game/` prefix, `aa_cry`/`aa_anim` `norm()` strips it and
lower-cases), `tools/aa-pak.exe cat` for CDF text, the AAEmu server lane `<aaemu-src>\rust\crates\aaemu-game\src\data\items.rs`
for how the server reads the same tables. Queries were run from scripts in a temp folder, not in the repo.

## 1. The chain

```
items.id (template id, impl_id tells the kind: 2 armour, 1 weapon, 24 accessory, 22 backpack, 3 body part)
 armour : item_armors(item_id) -> slot_type_id (equip slot), type_id (wearable kind), asset_id (= armor_assets.id)
          -> item_armor_assets(armor_asset_id, asset_id) -> item_assets(id, model_id, path, detail)
          model_id = models.id of the race/gender (10 nu_m, 11 nu_f, 16 el_m, 17 el_f, 18 ha_m, 19 ha_f, 20 fe_m, 21 fe_f)
          path = part .cdf (Model + Material only) -> .chr body mesh (obj/ folder) + .mtl
 weapon : item_weapons(item_id) -> asset_id (item_assets.id, model_id 0 = any model), holdable_id -> holdables(code, slot_type_id, pose_id)
          path = .cgf; attachment_offset_pos/rot on item_assets; worn_scale / drawn_scale on item_weapons
 body   : item_body_parts(item_id, slot_type_id 23 face / 24 hair / 27 tail / 28 body / 29 beard, model_id, asset_id, asset_1..4)
          -> item_assets(path) ; hair hat variants asset_1..4; face base = *_face00.cdf
 costume: slot 31 rows in item_armors, same chain (model rows are .chr or _v100.cdf)
 login  : characters.preview_cloth_pack_id -> equip_pack_cloths.<slot>_id (underpants ... cosplay), same chain
 dye    : dyeable_items.item_id -> default_dyeing_item_id -> item_dyeings(item_id, dyeing_color_id) -> dyeing_colors.color_rgb (0xBBGGRR)
 look   : server sets EquipDetail.image_item_template_id (look-change); item_look_converts / *_holdables / *_wearables
          give the look-change slot rules (27 converts, keyed by holdable_id or wearables slot)
```

Model rows of a player-visible armour item: about 6,000 `item_armor_assets` links per playable model
(10, 11, 16, 17, 18, 19, 20, 21) and `model_id = 0` never occurs for armour. Weapons, props and
body-independent meshes use `model_id = 0`.

## 2. Tables and columns

| table.column | meaning | value distribution (checked) | used by us |
|---|---|---|---|
| `items.impl_id` | item kind | 0 misc 6907, 1 weapon 2821, 2 armour 5245, 3 body part 443, 22 backpack 437, 24 accessory 456, 12 recipe 2561 | not read by aa-anim; hud drag reads the item_* tables directly |
| `items.char_gender_id` | gender lock | 0: 21358, 1: 50, 2: 74 (1 = male, 2 = female, inferred from coverage: gender-1 chest items have no nuian female row) | no |
| `item_armors.slot_type_id` | equip slot (`EquipmentItemSlotType`) | 1 head, 3 chest, 4 waist, 5 legs, 6 hands, 7 feet, 8 arms, 9 back, 12 undershirt, 13 underpants, 31 cosplay | yes (`hud/drag.rs` allowed slots; `db.rs` indirectly) |
| `item_armors.type_id` | wearable kind (`wearable_kinds.armor_type_id`) | 1, 2, 3 = cloth / leather / metal (labels inferred from asset paths and armor ratio 20 / 60 / 100); 4 = saddle (asset-proven: waist type 4); 5 = costume (asset-proven: `sk_*` items) | no |
| `item_armors.asset_id` | -> `armor_assets.id` (0 = no visual) | head 900 items with visual, chest 933, waist 23 (saddles only), legs 846, hands 670, feet 773, back 162, underpants 1, cosplay 222; arms 342 items and waist type 1-3 about 350 items all 0 | yes |
| `armor_assets.default_asset_id` | unused default | always 0 / null | no |
| `item_armor_assets` | armor_asset -> item_assets (many per row) | 18,003 rows, 2,466 armor assets | yes |
| `item_assets.model_id` | model the mesh is for (`models.id`) | armour: 10/11/16-21 about 5,900 each, 14 (dwarf) 101; weapons and props: 0 | yes (model filter) |
| `item_assets.path` | pak path (.cdf / .chr / .cgf / .dds) | armour .cdf 80 %, .chr 5 %, costume .chr/.cdf; weapons .cgf | yes |
| `item_assets.detail` | packed flags; head/cosplay: `(d>>13)&7` = 3 or 7 then hair selector `(d>>10)&7`, beard selector `(d>>7)&7` (0/6/7 default, 1 hidden, 2..5 alternate mesh); back 4; cosplay 7; weapons 40961..44039 | e.g. 459 head items hide hair (sel 1) | yes (`aa-anim/hair.rs`, headgear only) |
| `item_assets.attachment_offset_pos_*/rot_*` | attach offset of a rigid mesh | weapons: pos on 1,517 of 1,578 assets, rot (degrees) on 1,270; armour: pos non-zero on 231 assets, of which 188 carry a single `-1` on one axis (sentinel?), some garbage floats (1e27) | weapons only (`client weapons.rs`) |
| `item_assets.heel_offset_height` | heel lift (m) | 375 armour assets for the 8 models (0.05 to 0.09 m) | **no** |
| `item_assets.hinge_idx/limit/damping` | cloth hinge | armour: -1 / 0 / 0 on 14,703 of 14,703 (none) | no |
| `item_assets.more_asset_id` | second asset (underpants pair) | 28 armour assets, e.g. underpants 708 -> 707 (a `_df.dds` texture, not a mesh) | **no** |
| `item_assets.allow_mirror` | mirror for the off hand | weapons: 12 rows 't'; armour: none | **no** |
| `item_assets.default_anim`, `name_tag_offset_height` | - | unused in DB | no |
| `item_armors.skin_kind_id` | unknown (1, 2, 3 by slot) | not read by server either | no |
| `item_armors.equip_only_has_armor_visual`, `no_visual_error_message` | 'no visual' rules | 3-4 rows per slot (waist 4 type 4, feet 7 type 4) | no |
| `item_weapons` | weapon -> asset + holdable | 5,421 rows; 2,600 rows whose item_id is not in `items` (orphans); 15 rows with asset 0 (fist, ...) | yes (`client data.rs weapon()`) |
| `holdables` | weapon class | 27 rows; `code`, `slot_type_id` (14 mainhand, 16 two-handed, 17 one-handed, 18 ranged, 20 shield, 21 instrument), `pose_id` (4 = staff), `sheathe_priority` | yes except sheathe_priority |
| `item_accessories` | neck (slot 2, 170), ear (10, 156), finger (11, 165) | no asset column: never has a model | no (correctly) |
| `item_backpacks` | backpack (impl 22) | 437 rows, `asset_id` (model) | no: backpack slot 26 is not in `appearance.rs` WORN list |
| `item_body_parts` | face / hair / beard / tail / body | face 23: 142 items / 27 models; hair 24: 272 / 23; tail 27: 4; body 28: 26; beard 29: 3 | yes (`db.rs` body_part) |
| `equip_pack_cloths` | login costume per model | 1,712 packs; `*_id` per slot | yes (login costume) |
| `characters` | 13 rows, `creatable` = 't' for 8 | `preview_cloth_pack_id`, `preview_body_pack_id` | yes |
| `dyeable_items`, `item_dyeings`, `dyeing_colors` | default dye | 37 / 52 / 27 rows | yes (`default_dye`) |
| `item_look_converts` (+ `_required_items`, `_holdables`, `_wearables`) | look-change rules | 27 converts; holdable rules 22; wearable slot rules 8 (slots 1, 3, 5, 6, 7, 9, 31) | server only; client via detail |
| `wearables` / `wearable_slots` / `wearable_kinds` | armour class and coverage | wearables key `armor_type_id*128+slot_type_id` = identity on slot; `wearable_kinds.armor_ratio` 20/60/100/50/0 | no |
| `equip_slot_groups`, `equip_slot_group_maps`, `item_sockets`, `item_enchanting_gems` | gem / socket slots | 24 groups, 48 maps | enchant UI only |

Not present in this DB: a separate shoulder slot (shoulders are baked into chest meshes, see 4.) and a
per-model 'hidden' flag for body parts under armour (the cut hulls in `aa-anim/cut.rs` do that job at run time).

Orphans: 2,779 `item_armors` rows and 2,600 `item_weapons` rows reference item ids that are not in `items`.
The server never sees them; a join without `items` would show ghosts.

## 3. Per slot: sample items, model rows and files

Each row: item id, name, type, `asset_id`, then the nuian male (10) path where it exists. Elf female (17) and
ferre female (21) in brackets where the row exists. Paths are as in `item_assets.path`.

| slot | item (id, name, type) | nuian male (10) path | notes |
|---|---|---|---|
| head 1 | 6252 첫걸음 모자, t1 | `parts/hm/leather/hm_leather028/nu_m_hm_leather028.cdf` [el_f, fe_f same name] | detail 26624 = hair Type1 |
| head 1 | 19326 타락한 파괴신의 모자, t1 | `parts/hm/cloth/hm_cloth226/nu_m_hm_cloth226.cdf` | detail 25632 = hair Hidden |
| head 1 | 20744 대가의 태풍 가죽 모자, t2 | `parts/hm/leather/hm_leather158/nu_m_hm_leather158.cdf` | detail 25696 = hair Hidden; 8 models |
| chest 3 | 19327 타락한 파괴신의 상의, t1 | `parts/ar/cloth/ar_cloth226/nu_m_ar_cloth226.cdf` | detail 23040 |
| chest 3 | 20771 대가의 태풍 가죽 흉갑, t2 | `parts/ar/leather/ar_leather158/nu_m_ar_leather158.cdf` | 8 models |
| chest 3 | 20587 에페리움의 호수 천 상의, t1 | `parts/ar/cloth/ar_cloth159/nu_m_ar_cloth159.cdf` -> `parts/ar/obj/nu_m_1ar909_1arm008_1sh025.chr` | one mesh holds ar + arm + sh (shoulder) pieces |
| legs 5 | 19353 강인한 수문장의 경갑, t3 | `parts/pt/metal/pt_metal211/nu_m_pt_metal211.cdf` | 8 models |
| legs 5 | 20606 명인의 호수 천 하의, t1 | `parts/pt/cloth/pt_cloth156/nu_m_pt_cloth156.cdf` | pt_cloth156 is in pak; 8 models |
| legs 5 | 2689 해제, t3 | none (elf/ferre only) `light_armor/light_armor002_co01/el_f_pt_lightarmor002_co01.chr` | direct .chr, **not in pak** (dangling) |
| hands 6 | 6255 첫걸음 장갑, t1 | `parts/gv/leather/gv_leather027/nu_m_gv_leather027.cdf` | 8 models |
| hands 6 | 19351 강인한 수문장의 장갑, t3 | `parts/gv/metal/gv_metal201/nu_m_gv_metal201.cdf` | |
| hands 6 | 20822 명인의 번개 가죽 장갑, t2 | `parts/gv/leather/gv_leather156/nu_m_gv_leather156.cdf` | in pak |
| feet 7 | 6258 사랑의 초심자 신발, t1 | `parts/bo/leather/bo_leather025/nu_m_bo_leather025.cdf` | 8 models |
| feet 7 | 20852 대가의 태풍 가죽 장화, t2 | `parts/bo/leather/bo_leather158/nu_m_bo_leather158.cdf` | heel_offset_height on some feet assets |
| feet 7 | 19396 nact의 예식용 구두, t1 | `parts/bo/cloth/bo_cloth236/nu_m_bo_cloth236.cdf` | 6 models only: no elf / ferre female row |
| back 9 | 16249 빨간색 졸업생의 망토, t1 | `parts/cp/cloth/cp_cloth001/nu_m_cp_cloth001.cdf` | detail 32768 = back; 8 models |
| back 9 | 16416 파란색 졸업생의 망토, t1 | `parts/cp/cloth/cp_cloth002/...` | |
| back 9 | 17654 그림자 망토, t1 | 8 models | |
| waist 4 | 23091 허름한 소환수 안장, t4 (saddle) | mount models only (46 models, e.g. `horse/horse_armor_01/horse_saddle_01/horse_saddle_01.chr`) | no player row: a character never gets a waist mesh |
| waist 4 | belts type 1-3 (about 350 items) | none (asset 0) | no visual |
| arms 8 | 6737 첫걸음 가죽 토시, t2 / 7220 판금 완갑, t3 | none (asset 0) | 342 arm items, all without a visual; arm pieces are in chest meshes |
| underwear 13 | 30721 전나무 아가씨 축제 기념 속옷, t1 | one row with a visual | underpants 13 only 8 items; undershirt 12: 1 row, no visual |
| cosplay 31 | 8000028 sk_saint001_h (login, t5) | `costume/nu_m_sk_saint001/nu_m_sk_saint001_h.chr` + `na/` and `_lod1/_lod2` | Nuian login costume; one-piece skin |
| cosplay 31 | 34800 늦가을 페피 친구, t1 | `costume/nu_m_sk_fepee001/nu_m_sk_fepee001_v100.cdf` -> `nu_m_sk_fepee001.chr` | cdf wrapper; elf `el_f_sk_fepee001_v100.cdf` |
| cosplay 31 | 14359 sk_hariharan001 (Harani login) | Harani login costume | 222 cosplay items with visuals |

Weapons (item id, name, holdable code; offset in m, rot in degrees; path):

| class | item | holdable (slot, pose, rows) | path |
|---|---|---|---|
| 1H sword | 5643 로그인_의지_한손검 | normal_slashing_sod (17, 2, 224) | `objects/item/sword_1h/sword_1h_46_8/sword_1h_46_8.cgf`, off (0,-0.23,0.08), rot (-15,0,0) |
| 2H sword | 5660 첫걸음 대검 | 2h_slashing_sod (16, 3, 300) | `objects/item/sword_2h/sword_2h_01_1/sword_2h_01_1.cgf`, off (-0.03,0,0), rot (0,0,-2) |
| staff (2H) | 6102 첫걸음 마법지팡이 | 2h_blunt_staff (16, pose 4, 383) | `objects/item/staff_2h/staff_2h_26_1/staff_2h_26_1.cgf`, off (0.05,-0.07,0), rot (-2.54,180,0) |
| bow | 6127 첫걸음 활 | bow (18, 2, 352) | `objects/item/bow/bow_01_1/bow_01_1.cgf`, off (-0.05,0.03,0.03) |
| shield | 6152 첫걸음 방패 | shield (20, 2, 360) | `objects/item/shield/shield_01_1/shield_01_1.cgf`, off (0,0.01,-0.02), rot (-14,0,0) |
| lute (string) | 6177 첫걸음 현악기 | string_instrument (21, 5, 219) | `objects/item/ins_s/ins_s_01_1/ins_s_01_1.cgf` |
| drum | 6227 첫걸음 타악기 | percussion_instrument (21, 5, 107) | `objects/item/ins_p/ins_p_01_1/ins_p_01_1.cgf` |
| pipe | 6202 첫걸음 관악기 | tube_instrument (21, 5, 184) | `objects/item/ins_w/ins_w_01_1/ins_w_01_1.cgf` |
| dagger (off hand allowed) | 5318 첫걸음 단검 | fast_piercing_sod (17, 2, 318) | `objects/item/dagger/dagger_01_1/dagger_01_1.cgf` |
| 1H mace (main only) | 5845 첫걸음 둔기 | nor_blunt_mace (14, 2, 372) | `objects/item/mace_1h/mace_1h_16_3/mace_1h_16_3.cgf` |

Other weapon classes in the DB: 1H staffs (14, pose 2) 258 rows, 2H spears 218, 2H axe 263, 2H blunt mace 245, 2H blade 269, throw 10.

Body parts: face base 19838 `nu_m_face00` -> `objects/characters/nuian/male/face/nu_m_face00.cdf`; face 419
`nu_m_face01` -> `face/face01/nu_m_face01.chr`; hair 4084 `nu_m_hair02` (asset 676, hat variants 681) ->
`hair/nu_m_hair02/nu_m_hair02.chr`. The hair `.chr` of 4084 is **not in the pak index** (dangling; `hair020` exists).

## 4. Files and naming

| item | pattern | observed |
|---|---|---|
| base body | `objects/characters/<race>/<g>/nude/<pfx>_<g>_base.chr` (+ `nu_m.cdf`, `nu_m_base.cdf` = AttachmentList) | `nu_m_base.cdf` has `customize="6"` and all slot names (body, face, hair, head, chest, waist, legs, hands, feet, arms, glasses, beard CA_SKIN; mainhand, offhand, ranged, musical, craft_*, back, backpack, cosplay CA_BONE with `item_hand_r`, `item_hand_l`, `item_back_c`). Every entry has `Flags="1"` (empty/hidden) |
| armour part | `parts/<bp>/<material>/<bp>_<material><id>/<pfx>_<g>_<bp><material><id>.cdf` | cdf = Model + Material only, **no AttachmentList** (41 sampled item cdfs: 0 with AttachmentList). bp codes: hm head, ar chest, pt legs, gv hands, bo feet, cp back |
| armour mesh | `parts/<bp>/obj/<pfx>_<g>_<n><bp><id>[_<n><bp><id>...].chr` | one mesh can hold several pieces: `1ar909_1arm008_1sh025` (chest + arms + shoulders), `1bt001` (belt) appear in the same name; `_lod1/_lod2` variants |
| light armour | `<race>/<g>/light_armor/<name>/<pfx>_<g>_pt_<name>.chr` | direct skin .chr, no cdf; elf / ferre only |
| costume | `costume/<name>/<pfx>_<g>_sk_<name>.chr` (or `_v100.cdf` -> `.chr`), `na/` sub-folder, `_lod1/_lod2` | cosplay skins; one-piece |
| weapon / shield / instrument | `objects/item/<class>/<class>_<nn>_<n>/<class>_<nn>_<n>.cgf` | static mesh on a holdable bone; `_posture.cgf` in model 0 props |
| body parts | `<race>/<g>/face/faceNN/<pfx>_<g>_faceNN.chr`, `<race>/<g>/hair/<pfx>_<g>_hairNN/<pfx>_<g>_hairNN.chr` | hair uses `hair_base` dds on some races |
| skinning | no `.skin` files in the 1.2 pak; all skinned meshes are `.chr`; `.cgf` for rigid | `_lod` names: 12,670 `.chr`, 1,627 `.cgf` in the pak index; `item_assets` itself has one `_lod` path |

Race / gender suffixes: `nu_m`, `nu_f`, `el_m`, `el_f`, `ha_m`, `ha_f`, `fe_m`, `fe_f` (prefix `nu`, `el`, `ha`, `fe`). Folder: `nuian/male`, `elf/female` ... (hariharan = `hariharan`, ferre = `ferre`).

Missing from the pak (dangling DB paths, player models only, from `item_armors` x `item_assets`):

| slot | rows (8 playable models) | rows whose path is not in pak index | example |
|---|---|---|---|
| head 1 | 6,569 | 62 | `elf/male/costume/intro001/el_m_hm_intro001.cdf` |
| chest 3 | 6,616 | 276 | `elf/female/parts/ar/metal/ar_metal029/el_f_ar_metal029.cdf` |
| legs 5 | 6,160 | 47 | `elf/male/parts/pt/metal/pt_metal027/el_m_pt_metal027.cdf` |
| hands 6 | 4,753 | 100 | `nuian/male/parts/gv/leather/gv_leather005/nu_m_gv_leather005.cdf` |
| feet 7 | 5,308 | 86 | `elf/male/costume/intro001/el_m_bo_intro001.cdf` |
| back 9, cosplay 31, underpants 13 | 1,271 / 1,709 / 8 | 0 | |
| weapons (impl 1, 2,818 rows) | 2,818 | 8 | `objects/machinima/common_flower_lily_cutscene.cgf` |

About 1.5 to 3 % of the armour rows of a slot point at files that are not in this pak. The client's
`appearance.rs` filters those out with `pak.exists`, so such an item draws nothing (or the image item's look)
and no error is shown.

## 5. Our code: what consumes this and what it assumes

| file | consumes | assumes / notes |
|---|---|---|
| `crates/aa-anim/src/db.rs` | `item_armors`, `item_armor_assets`, `item_assets` (`item_visuals`, `item_meshes`), `item_body_parts` + `item_assets` (`body_part`), `equip_pack_cloths` (`login_costume`), `dyeable_items`/`item_dyeings`/`dyeing_colors` (`default_dye`), hair/skin/presets | armour filter by `ia.model_id = ?`; does not join `items` (orphans harmless); `slot` names from `CLOTH_PACK_SLOTS`; `models` 8 playable only |
| `crates/aa-anim/src/character.rs` | `Costume::{None, Login, Items, Meshes}` -> `costume_paths`; `resolve_part` (cdf unwrap); `cut` hulls | `Items` pushes slot name `"item"`; the one-piece `cosplay` check (`slot == "cosplay"`, line ~269) only sees the login costume (see gap 1) |
| `crates/aa-anim/src/cdf.rs` | CDF `Model`, `Material`, `AttachmentList` | part cdfs have none; base `nu_m_base.cdf` attachments are `Flags="1"` |
| `crates/aa-anim/src/custom.rs` | preset / hair / skin ids | no equipment |
| `crates/aa-anim/src/hair.rs` | `item_assets.detail` bits 13-15, 7-12 (headgear) | head / cosplay only; beards only decoded, not attached |
| `crates/aa-anim/docs` `aa-anim.md` | documents the tables above | no mention of heel, look-convert, dangling paths |
| `crates/aa-client/src/ingame/appearance.rs` | server `Equipment`: WORN = underpants, undershirt, legs, chest, waist, hands, feet, arms, neck, head, back + cosplay; `selected_visuals` (look item then own item, `pak.exists`); `headgear_detail` | backpack (26) and beard / tail / glasses body slots not drawn; waist/neck/arms have no model anyway |
| `crates/aa-client/src/ingame/attachments.rs` | CDF `Attachment`s of a part (object / effect) | parts have none; only the horse-type creatures use it in practice |
| `crates/aa-client/src/ingame/weapons.rs` | `data.rs weapon()`: `item_weapons` + `item_assets` + `holdables` (`code`, `slot_type_id == 16`, `pose_id == 4`) | attach bones hard-coded by holdable code (`item_bow_l`, `item_arm_l`, `item_side_l/r`, `item_back_c`, `item_staff_l`), not from the DB; `sheathe_priority` ignored; `allow_mirror` ignored |
| `crates/aa-client/src/hud/drag.rs` | `item_armors.slot_type_id` directly, `item_weapons` -> `holdables.slot_type_id`, `item_backpacks` = 30 | consistent with the server's wearables identity mapping |

## 6. Gaps we suspect (ranked)

1. **Cosplay does not remove the nude body when it comes from the server.** `worn_items` produces
   `Costume::Items`, and `character.rs` gives those pieces the slot name `"item"`, so the
   `slot == "cosplay"` check that drops the nude `body` never fires. A one-piece costume then sits
   over the body mesh (skin pokes through). Fix candidate: pass the slot name through (or check the
   cosplay item id). Not verified in the running client.
2. **Missing meshes read as "not attached".** About 2 % of armour rows point at files not in the pak
   (chest 276 rows, feet 86, hands 100, head 62, legs 47). `selected_visuals` drops them silently, so the
   item shows nothing or the look item only. Likely one class of "doesn't attach" reports. Check
   `pak.exists` coverage per slot in the visual sweep.
3. **Heel offset ignored.** 375 armour assets (shoes / boots, 0.05 to 0.09 m) carry `heel_offset_height`.
   Without it the foot mesh sits on the same plane as bare feet, so the sole sinks into the ground or
   the leg pieces clip. Plausible for "feet look wrong" reports.
4. **`allow_mirror` ignored for weapons.** 12 weapon assets say the mesh may be mirrored for the off hand.
   `weapons.rs` offhand copies use the same transform; check left-hand dual wield sweeps.
5. **`more_asset_id` ignored.** Underpants pairs reference a second asset (a `_df.dds` texture, not a mesh).
   Only 28 armour assets; probably harmless, but confirm underwear colour.
6. **Armour `attachment_offset_*` ignored.** 188 armour assets have a single `-1` on one axis (sentinel
   or flags, meaning unknown), and some have garbage floats. Our code never reads them for armour. The
   pattern clusters on the `metal002_va002` / `cloth122` families: worth checking whether those pieces
   sit off by a few cm.
7. **Shoulders and arms are baked into chest meshes, but arm items have no visual.** 342 arm items have
   no visual. If the real client shows arm guards, they come from the chest `obj/` names (`1arm`,
   `1sh`), which our code draws as part of the chest. Visual check needed.
8. **Waist slot (4) is saddle-only.** Belts type 1-3 have no visual, so a character never shows a
   waist mesh. Fine for the DB; check that `Costume::Items` never pulls a saddle onto a character.
9. **Backpack (slot 26) and tail/glasses are not drawn** by `appearance.rs`. Backpacks may need the base
   `backpack` bone (`item_back_c`) as the real client does.
10. **Staff detection and sheath bones are hard-coded.** `pose_id == 4` picks the only 2H staff, and
    bones are chosen by holdable code, not from the DB. Fine for the 27 holdables, but any new holdable
    needs code changes.
11. **Dangling hair path.** `nu_m_hair02` (item 4084) points at a `.chr` that is not in the pak
    (verified case-insensitively; only `hair020` exists). Check how the hair part handles a missing mesh
    (not traced in code).
12. **Orphan rows.** 2,779 armour and 2,600 weapon rows point at missing items. Harmless to us, but any
    tool that scans `item_armors` without joining `items` will report phantom equipment.

## 7. Open questions for the sweep

- What `skin_kind_id` (1, 2, 3) and `equip_only_has_armor_visual` mean in the real client (not read by the server).
- Whether the real client uses `heel_offset_height` (it is per asset, so probably yes).
- Meaning of the single `-1` in armour `attachment_offset_pos` (sentinel vs. flags).
- Whether `more_asset_id` underpants textures are the second mesh's diffuse or a separate layer.
