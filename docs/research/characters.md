# ArcheAge 1.2 playable characters: default assets per race and gender

Research note (read-only). Sources:
- Pak index: `data\pak12.tsv` (all pak paths below carry the `game/` prefix in the index).
- Game DB: `<server>\Data\compact.sqlite3` (opened read-only).
- Extracted copies used for text reads: `data\extract\aa-anim\game\...` and `data\extract\aa-world\game\...`. Every path cited here was checked against `pak12.tsv`. The 40 costume-chain paths were checked programmatically; the rest were grepped.
- `cargo run -p aa-pak` was not used because the build was blocked by a shared cargo lock. Text was read from the existing extraction instead.

Confidence labels: **[verified]** = checked directly in DB or pak/extract text. **[inferred]** = follows from naming or structure but not tested in code. **[unresolved]** = no consistent join found.

---

## 1. Playable races and genders

Playable = `characters.creatable = 't'`. Exactly four races are flagged, and each has a `create_<race>.lyr` in the pak root. **[verified]**

| Internal race | `char_race_id` | Male `characters.id` / `char_gender_id=1` | Female `characters.id` / `char_gender_id=2` |
|---|---|---|---|
| nuian | 1 | 1 (model 10 `nuian_male`) | 2 (model 11 `nuian_female`) |
| elf | 4 | 5 (model 16 `elf_male`) | 6 (model 17 `elf_female`) |
| hariharan | 5 | 9 (model 18 `hariharan_male`) | 10 (model 19 `hariharan_female`) |
| ferre | 6 | 15 (model 20 `ferre_male`) | 16 (model 21 `ferre_female`) |

- Race and gender id meanings are **[inferred]** from the model names attached to each `characters` row. No lookup table for `char_race_id` or `char_gender_id` was found in the DB.
- Non-playable rows in `characters` (all `creatable='f'`): race 2 `fairy_female` (row 8), race 3 `dwarf_male`/`dwarf_female` (rows 3, 4), race 7 `returned_male` (row 11), race 8 `warborn_male` (row 13). Warborn and dwarf still appear in the login2 world (section 3).

---

## 2. Per race and gender: skeleton, cdf, meshes, login costume, default ids

### 2.1 Model and skeleton chain **[verified]**

Chain: `characters.model_id` → `models.id` (`sub_type='ActorModel'`) → `models.sub_id` = `actor_models.id` → `actor_models.model_file` (the base `.cdf`) → the `.cdf`'s `<Model File=...>` (the base `.chr`).

| Row | Race / gender | `models.id` | `actor_models.id` | `actor_models.model_file` (base cdf) | Base `.chr` (from cdf) | Actor height | Graph |
|---|---|---|---|---|---|---|---|
| 1 | nuian M | 10 | 1 | `objects/Characters/nuian/male/nude/nu_m.cdf` | `objects/characters/nuian/male/nude/nu_m_base.chr` | 1.9 | `player_full_body.xml` / `player_upper_body.xml` |
| 2 | nuian F | 11 | 2 | `objects/Characters/nuian/female/nude/nu_f.cdf` | `.../nuian/female/nude/nu_f_base.chr` | 1.72 | same |
| 5 | elf M | 16 | 7 | `objects/Characters/elf/male/nude/el_m.cdf` | `.../elf/male/nude/el_m_base.chr` | 1.9 | same |
| 6 | elf F | 17 | 8 | `objects/Characters/elf/female/nude/el_f.cdf` | `.../elf/female/nude/el_f_base.chr` | 1.78 | same |
| 9 | hariharan M | 18 | 9 | `objects/Characters/hariharan/male/nude/ha_m.cdf` | `.../hariharan/male/nude/ha_m_base.chr` | 1.67 | same |
| 10 | hariharan F | 19 | 10 | `objects/Characters/hariharan/female/nude/ha_f.cdf` | `.../hariharan/female/nude/ha_f_base.chr` | 1.55 | same |
| 15 | ferre M | 20 | 11 | `objects/Characters/ferre/male/nude/fe_m.cdf` | `.../ferre/male/nude/fe_m_base.chr` | 2.0 | same |
| 16 | ferre F | 21 | 12 | `objects/Characters/ferre/female/nude/fe_f.cdf` | `.../ferre/female/nude/fe_f_base.chr` | 1.7 | same |

All `.chr` paths above are in the pak (checked). Each `.cdf` is a `CharacterDefinition` with the `<Model File>` above, a `customize="6"` or `"7"` attachment list, and these slots: `body`, `face`, `hair`, `head`, `chest`, `waist`, `legs`, `hands`, `feet`, `arms`, `glasses`, `beard`, `cosplay`, plus weapon bones (`mainhand`, `offhand`, `ranged`, `musical`, `craft_*`, `back`, `backpack`). Ferre has an extra `tail` slot. **[verified]**

Each base cdf sits beside a `*_base.cal` (animation alias list) and `*_base.chr`. The pak also contains variants such as `nu_m_base_lod1.chr` and `nu_m_nude.chr` that the base cdf does not reference. **[verified in pak]**

The `.cdf` used by the create screen (`*_base.cdf`, female variant for all four races) is in section 3.

### 2.2 Body, face, hair and skin ids from `equip_pack_body_parts` (login-stage packs)

`characters.preview_body_pack_id` → `equip_pack_body_parts.id`. The `model_id` on that pack equals `characters.model_id` for all 8 rows. **[verified]**

| Row | Pack id / name | `skin_color_id` (`skin_colors.model_id` also matches) | Raw `face_id` | Raw `hair_id` | Raw `hair_color_id` | `beard_id` | `body_diffuse_map_id` |
|---|---|---|---|---|---|---|---|
| 1 nuian M | 3 `로그인 스테이지 - 누이안 남` | 1 | 419 | 4085 | 308 | 0 | 0 |
| 2 nuian F | 4 `로그인 스테이지 - 누이안 여` | 4 | 4081 | 19025 | 367 | 0 | 0 |
| 5 elf M | 6 `로그인 스테이지 - 엘프 남` | 5 | 4087 | 8475 | 238 | 0 | 0 |
| 6 elf F | 7 `로그인 스테이지 - 엘프 여` | 7 | 1130 | 19360 | 418 | 0 | 0 |
| 15 ferre M | 31 `로그인 스테이지 - 페레 남` | 9 | 563 | 15821 | 79 | 0 | 0 |
| 16 ferre F | 32 `로그인 스테이지 - 페레 여` | 11 | 408 | 16245 | 303 | 0 | 0 |
| 9 hariharan M | 70 `로그인 스테이지 - 하리하란 남` | 13 | 551 | 552 | 264 | 0 | 0 |
| 10 hariharan F | 71 `로그인 스테이지 - 하리하란 여` | 15 | 410 | 411 | 183 | 0 | 0 |

**Skin colour rows** (`skin_colors.id`, all with `model_id` equal to the character's model): nuian M → 1 (`custom01`), nuian F → 4, elf M → 5, elf F → 7, ferre M → 9 (`custom_postfix='brop01'`), ferre F → 11 (`brop01`), hariharan M → 13, hariharan F → 15. **[verified]**

**Face, hair and hair colour: [unresolved].** These three columns do not resolve consistently:
- `face_id`: the only model-consistent hits are pack 3 (nuian M, face 419) → `total_character_customs.id = 419` (model 10, `nu_m_lucius`), and pack 71 (hariharan F, face 410) → `face_decal_assets.id = 410` (model 19, a makeup decal rather than a face mesh). The other six packs hit a row with a different model_id or no row at all.
- `hair_id`: values such as 4085, 19025, 8475 point to `item_assets` rows whose paths are other races' leather or plate armour (e.g. `item_assets.id=4085` is `objects/characters/hariharan/female/leather/leather013/ha_f_ar_leather013.chr`). `character_customizing_hair_assets` has only two rows, both with `model_id=0`.
- `hair_color_id`: `hair_colors.id` matches a row with the same `model_id` only for packs 32 (303, model 21) and 70 (264, model 18). The other six point to another race's hair colour.

Treat the face, hair and hair colour defaults as unknown for this DB. The raw values above are the only ids the DB gives.

### 2.3 Login-stage costume (default cloth pack → DB-resolved meshes) **[verified]**

`characters.preview_cloth_pack_id` → `equip_pack_cloths.id`. Each non-zero slot id is an `items.id`, resolved through the chain in section 4. Every resolved path below is in the pak. Slot-to-cdf-attachment mapping (shirt→chest, pants→legs, glove→hands, shoes→feet) is **[inferred]**.

| Row | Cloth pack (`equip_pack_cloths.id`, name) | Slots set | Resolved mesh paths (`game/` prefix omitted, all under `objects/characters/`) |
|---|---|---|---|
| 1 nuian M | 183 `로그인 스테이지 - 누이안` | cosplay 8000028 (`sk_saint001_h`) | `nuian/male/costume/nu_m_sk_saint001/nu_m_sk_saint001_h.chr` (item_assets 8000005) |
| 2 nuian F | 183 | cosplay 8000028 | `nuian/female/costume/nu_f_sk_saint001/nu_f_sk_saint001_h.chr` (item_assets 8000006) |
| 5 elf M | 184 `로그인 스테이지  - 엘프` (two spaces in name) | shirt 15548, glove 15549, pants 15550, shoes 15551 | `elf/male/parts/ar/leather/ar_leather218/el_m_ar_leather218.cdf` (9671); `elf/male/parts/gv/leather/gv_leather224/el_m_gv_leather224.cdf` (9673); `elf/male/parts/pt/leather/pt_leather221/el_m_pt_leather221.cdf` (9672); `elf/male/parts/bo/leather/bo_leather225/el_m_bo_leather225.cdf` (9674) |
| 6 elf F | 184 | same slots | `elf/female/parts/ar/leather/ar_leather218/el_f_ar_leather218.cdf` (9676); `.../gv/leather/gv_leather224/el_f_gv_leather224.cdf` (9678); `.../pt/leather/pt_leather221/el_f_pt_leather221.cdf` (9677); `.../bo/leather/bo_leather225/el_f_bo_leather225.cdf` (9679) |
| 15 ferre M | 185 `로그인 스테이지 - 페레` | shirt 15548, glove 15549, pants 15550, shoes 15551 | `ferre/male/costume/fe_m_login001/fe_m_ar_login001.chr` (6210), `fe_m_gv_login001.chr` (6211), `fe_m_pt_login001.chr` (6212), `fe_m_bo_login001.chr` (6213) |
| 16 ferre F | 185 | same | `ferre/female/costume/fe_f_login001/fe_f_ar_login001.chr` (6478), `fe_f_gv_login001.chr` (6479), `fe_f_pt_login001.chr` (6480), `fe_f_bo_login001.chr` (6481) |
| 9 hariharan M | 497 `로그인 스테이지 - 하리하란 남` | cosplay 24583 (`sk_hariharan001`) | `hariharan/male/costume/ha_m_sk_hariharan001/ha_m_sk_hariharan001.chr` (21273) |
| 10 hariharan F | 193 `로그인 스테이지 - 하리하란 여` | cosplay 14359 (`sk_hariharan001`) | `hariharan/female/costume/ha_f_sk_hariharan001/ha_f_sk_hariharan001.chr` (21366) |

Notes:
- Ferre's login outfit is a four-piece `*_login001.chr` set (these are the "login001" costume files in the pak, `game/objects/characters/ferre/{male,female}/costume/fe_{m,f}_login001/`).
- Elf's default uses leather armour meshes from the normal `parts/` tree. The pak does contain `elf/{male,female}/costume/login001/` and `el_{m,f}_ar_cloth500`/`cloth001` files, but **no DB default pack references them**. **[unresolved: whether the client uses them]**
- Hariharan's default is one full-body skin cosplay per gender. The pak also has `hariharan/{male,female}/costume/ha_{m,f}_login001/` (`ha_*_ar_login001`, `ha_*_pt_login001`, `ha_*_bo_login001`). The DB links those only through shirt/pants/glove/shoes items 15548–15551, which the hariharan default packs do not use. **[unresolved]**
- Nuian's default is also a single cosplay skin. `nuian/female/costume/login001/` (`nu_f_*_login001.chr`) exists in the pak. The DB links those files only through items 15548–15551, which the nuian default pack does not use. **[unresolved]**
- All cloth packs in this table have `cosplay_id` (or `shirt/pants/glove/shoes`) only; `*_grade_id` columns are not used for the default look.

### 2.4 Mesh folder counts (pak) **[verified]**

Mesh and texture source trees per race and gender (`game/objects/characters/<race>/<gender>/`). Counts are file lines in `pak12.tsv`.

| Race / gender | All files under `characters/<race>/<gender>/` | Face sub-folders | Hair sub-folders |
|---|---|---|---|
| nuian M | 7627 | 30 | 30 |
| nuian F | 8325 | 22 | 33 |
| elf M | 6695 | 15 | 24 |
| elf F | 7538 | 13 | 31 |
| hariharan M | 7149 | 14 | 27 |
| hariharan F | 7773 | 16 | 31 |
| ferre M | 6578 | 51 | 18 |
| ferre F | 7166 | 53 | 22 |

Face and hair content, if you need the actual default face or hair mesh, has to come from the client's runtime logic rather than the DB (see 2.2).

---

## 3. Create and login stage: cdf and cinema references

### 3.1 Create-scene layers **[verified]**

`game/create_<race>.lyr` (extracted to `data/extract/aa-anim/game/`). Each has one `AnimObject` named `create_character_<race>`, and its `object_Model` is the **female** base cdf for that race. There is no male cdf in these layers. `Animation="Default"` on every one.

| Layer | AnimObject | `object_Model` | Animation |
|---|---|---|---|
| `create_nuian.lyr` | `create_character_nuian` | `game/objects/characters/nuian/female/nude/nu_f_base.cdf` | Default |
| `create_elf.lyr` | `create_character_elf` | `game/objects/characters/elf/female/nude/el_f_base.cdf` | Default |
| `create_ferre.lyr` | `create_character_ferre` | `game/objects/characters/ferre/female/nude/fe_f_base.cdf` | Default |
| `create_hariharan.lyr` | `create_character_hariharan` | `game/objects/characters/hariharan/female/nude/ha_f_base.cdf` | Default |

`create_nuian.lyr` also contains camera objects `04_customize_nuian_female_1..4` and `04_customize_nuian_male_1..4`. The male view appears to be done by code or by swapping models at runtime (not verified).

### 3.2 Login2 world entities **[verified]**

`game/worlds/login2/cells/000_001/client/entities.xml` and `.../001_000/client/entities.xml` (extracted to `data/extract/aa-world/game/worlds/login2/`):

| Cell | Entity name | Class | `object_Model` (base cdf) | Animation |
|---|---|---|---|---|
| 000_001 | `select_character` | AnimObject | `nuian/female/nude/nu_f_base.cdf` | Default |
| 000_001 | `create_character_nuian` | AnimObject | `nuian/female/nude/nu_f_base.cdf` | Default |
| 000_001 | `create_character_ferre` | AnimObject | `ferre/female/nude/fe_f_base.cdf` | Default |
| 000_001 | `create_character_elf` | AnimObject | `elf/female/nude/el_f_base.cdf` | Default |
| 000_001 | `create_character_hariharan` | AnimObject | `hariharan/female/nude/ha_f_base.cdf` | Default |
| 000_001 | `create_character_dwarf` | AnimObject | `dwarf/male/nude/dw_m_base.cdf` | Default |
| 000_001 | `create_character_warborn` | AnimObject | `warborn/male/nude/wa_m_base.cdf` | Default |
| 000_001 | `create_character` | AnimObject | `ferre/female/nude/fe_f_base.cdf` | Default |
| 001_000 | `select_character` | AnimObject | `hariharan/female/nude/ha_f_base.cdf` | Default |
| 001_000 | `create_character` | AnimObject | `ferre/female/nude/fe_f_base.cdf` | Default |

- `select_character` in cell 000_001 is at `727.16266,563.70398,296.66144`; in cell 001_000 it is at `627.83185,461.45251,159.82037`. Which character's model is shown on the select screen at runtime is **[unresolved]**. Presumably the client swaps the account's real characters onto these slots, but no code confirms it.
- Every entity that represents a character uses `Animation="Default"`. No alias named `Default` exists in any base `.cal` or in `global_player.cal` **[verified by grep]**. The meaning of `Default` is therefore **[unresolved]**.

### 3.3 Login-stage cinema sequences **[verified names]**

`game/worlds/login2/cinemas/` (28 files, extracted at `data/extract/aa-world/game/worlds/login2/cinemas/`):
- `01_world_select_e1.xml`, `02_world_select_e2.xml`
- `03_character_select_s.xml` (camera `cam_select`), `04_character_select_e.xml`
- `05_character_create_s_<race>_<gender>.xml` for nuian/elf/ferre/hariharan × male/female (8 files)
- `06_character_customize_s.xml`, `07_character_customize_e.xml`
- Each has a matching `*_cinema_only_objs.xml`. The nuian-male one is 27 bytes.

Cinema XML only contains tracks and entity GUIDs, not model paths. The model-to-entity links are in the entities.xml above.

---

## 4. Animation sources per race and gender

### 4.1 Animation list (`.cal`) per race and gender **[verified]**

Each base `.cal` is the animation alias list for that race and gender. It `$Include`s `global_player.cal` and one `global_player_machinima_*.cal`.

| Race / gender | Base cal | Lines | `$Include` (machinima) | `$TracksDBH` |
|---|---|---|---|---|
| nuian M | `game/objects/characters/nuian/male/nude/nu_m_base.cal` | 253 | `global_player_machinima_m1.cal` | `animations\nuian\male\nu_m.dbh` |
| nuian F | `.../nuian/female/nude/nu_f_base.cal` | 228 | `global_player_machinima_f1.cal` | `animations\nuian\female\nu_f.dbh` |
| elf M | `.../elf/male/nude/el_m_base.cal` | 254 | `global_player_machinima_m1.cal` | `animations\elf\male\el_m.dbh` |
| elf F | `.../elf/female/nude/el_f_base.cal` | 226 | `global_player_machinima_f1.cal` | `animations\elf\female\el_f.dbh` |
| hariharan M | `.../hariharan/male/nude/ha_m_base.cal` | 264 | `global_player_machinima_m1.cal` | `animations\hariharan\male\ha_m.dbh` |
| hariharan F | `.../hariharan/female/nude/ha_f_base.cal` | 223 | `global_player_machinima_f1.cal` | `animations\hariharan\female\ha_f.dbh` |
| ferre M | `.../ferre/male/nude/fe_m_base.cal` | 239 | `global_player_machinima_m2.cal` | `animations\ferre\male\fe_m.dbh` |
| ferre F | `.../ferre/female/nude/fe_f_base.cal` | 202 | `global_player_machinima_f2.cal` | `animations\ferre\female\fe_f.dbh` |

Shared: `game/objects/characters/global_player.cal` (4354 lines, about 552 lines with "idle"). `#filepath` and `$AnimEventDatabase` in the elf and hariharan base cals still say `nuian` (copy-paste in the source). **[verified]**

### 4.2 Animation files per race and gender (pak) **[verified]**

Counts are for `game/animations/<race>/<gender>/` only (gender-less files directly under `game/animations/<race>/` are not counted).

| Race / gender | Total | `.caf` | `.lmg` | `.fsq` (facial) | `.fxl` |
|---|---|---|---|---|---|
| nuian M | 2231 | 2025 | 34 | 168 | 1 |
| nuian F | 1891 | 1693 | 33 | 161 | 1 |
| elf M | 76 | 51 | 0 | 23 | 1 |
| elf F | 101 | 69 | 0 | 30 | 1 |
| hariharan M | 186 | 157 | 0 | 28 | 0 |
| hariharan F | 158 | 129 | 0 | 28 | 0 |
| ferre M | 1928 | 1733 | 33 | 158 | 1 |
| ferre F | 1826 | 1642 | 33 | 147 | 1 |

`.lmg` files are in `lmg_files/` sub-folders. `.cal`/`.chrparams`/`.dba` are not used under `game/animations/`, so the anim-list step is the base `.cal` above.

### 4.3 Login-stage idle **[inferred, with strong evidence]**

Every base cal defines these two aliases (identical names for all 8 races):
- `fist_ac_stage_idle` → `fist/fist_ac_stage_idle.caf` (plus facial `facial/fist_ac_stage_idle.fsq`)
- `fist_ac_stage_rand_1` → `fist/fist_ac_stage_rand_1.caf` (plus facial `facial/fist_ac_stage_rand_1.fsq`)

The base `.cal` comment next to `fist_ac_stage_idle` decodes (cp949) to `로그인스테이지용`, meaning "for login stage". All of the following exist in the pak for all 8 race/gender pairs: `game/animations/<race>/<gender>/fist/fist_ac_stage_idle.caf`, `.../fist/fist_ac_stage_rand_1.caf`, `.../facial/fist_ac_stage_idle.fsq`, `.../facial/fist_ac_stage_rand_1.fsq`. **[verified exist]**

- Nuian base cals also define `fist_ac_stage_closeup_idle` → `facial/fist_ac_stage_idle.fsq` (nuian only). **[verified]**
- The likely login idle is `fist_ac_stage_idle`, with `fist_ac_stage_rand_1` as the random variant. **[inferred]** The `AnimObject` `Animation="Default"` is **[unresolved]** (section 3.2).
- Animation graph used by actors: `player_full_body.xml` / `player_upper_body.xml` in `game/animations/graphs/` (from `actor_models.animation_graph`/`upperbody_graph`). **[verified in DB and pak]**

---

## 5. How the DB tables link (keys and joins) **[verified]**

```
characters (id, char_race_id, char_gender_id, model_id, preview_body_pack_id, preview_cloth_pack_id, creatable)
  ├─ model_id ─────────────→ models (id, sub_type='ActorModel', sub_id, name)
  │                            └─ sub_id ─────→ actor_models (id, model_file = base .cdf, animation_graph, upperbody_graph)
  │                                              └─ base .cdf (game pak) → <Model File> base .chr + attachment slots
  ├─ preview_body_pack_id ─→ equip_pack_body_parts (id, model_id, skin_color_id, face_id*, hair_id*, hair_color_id*, beard_id, body_diffuse_map_id)
  │                            └─ skin_color_id ──→ skin_colors (id, model_id)   [model_id must match]
  │                            (* face_id, hair_id, hair_color_id: unresolved, see 2.2)
  └─ preview_cloth_pack_id → equip_pack_cloths (id, shirt_id, pants_id, glove_id, shoes_id, cosplay_id, ...)
                               └─ <slot>_id ──→ items (id)  ─→ item_armors (item_id, asset_id)
                                                  └─ asset_id ─→ armor_assets (id, default_asset_id)
                                                  └─ item_armors.asset_id ─→ item_armor_assets (armor_asset_id, asset_id)
                                                       └─ asset_id ─→ item_assets (id, model_id, path)   [model_id must match characters.model_id]
```

Other relevant tables (not on the default path): `custom_face_presets` (per-model eye/nose/mouth/face-shape morph options, not defaults), `total_character_customs` (per-model full face presets, owner type), `face_diffuse_maps`, `face_normal_maps`, `face_eyelash_maps`, `body_diffuse_maps`, `hair_colors` (`model_id`, `hair_material` mtl path), `character_customizing_hair_assets` (only 2 rows), `character_equip_packs` (`ability_id`, `newbie_cloth_pack_id`, `newbie_weapon_pack_id`; no FK from `characters` found, so not used here).

Checks done: `characters.model_id` = `equip_pack_body_parts.model_id` for 8/8; `skin_colors.model_id` matches for 8/8; `item_assets.model_id` filtered by `characters.model_id` gives exactly one row per non-zero slot (20 rows for the 8 characters). **[verified]**

### 5.1 Worked SQL example (verified, returns 20 rows for characters 1,2,5,6,9,10,15,16)

```sql
WITH slots AS (
  SELECT c.id AS char_id, c.model_id, 'shirt'   AS slot, cl.shirt_id   AS item_id FROM characters c JOIN equip_pack_cloths cl ON cl.id = c.preview_cloth_pack_id
  UNION ALL SELECT c.id, c.model_id, 'pants',  cl.pants_id  FROM characters c JOIN equip_pack_cloths cl ON cl.id = c.preview_cloth_pack_id
  UNION ALL SELECT c.id, c.model_id, 'glove',  cl.glove_id  FROM characters c JOIN equip_pack_cloths cl ON cl.id = c.preview_cloth_pack_id
  UNION ALL SELECT c.id, c.model_id, 'shoes',  cl.shoes_id  FROM characters c JOIN equip_pack_cloths cl ON cl.id = c.preview_cloth_pack_id
  UNION ALL SELECT c.id, c.model_id, 'cosplay',cl.cosplay_id FROM characters c JOIN equip_pack_cloths cl ON cl.id = c.preview_cloth_pack_id
)
SELECT s.char_id, s.slot, s.item_id, ia.id AS asset_id, ia.model_id, ia.path
FROM slots s
JOIN item_armors arm        ON arm.item_id = s.item_id
JOIN item_armor_assets iaa  ON iaa.armor_asset_id = arm.asset_id
JOIN item_assets ia         ON ia.id = iaa.asset_id AND ia.model_id = s.model_id
WHERE s.char_id IN (1,2,5,6,9,10,15,16) AND s.item_id > 0
ORDER BY s.char_id, s.slot;
```

Sample output (verified): `(1,'cosplay',8000028,8000005,10,'objects/characters/nuian/male/costume/nu_m_sk_saint001/nu_m_sk_saint001_h.chr')`, `(15,'shirt',15548,6210,20,'objects/characters/ferre/male/costume/fe_m_login001/fe_m_ar_login001.chr')`.

Pack-to-model check (verified, the body-pack join for character 1):
```sql
SELECT c.id, m.name, bp.id, bp.name, sc.id, sc.COMMENT
FROM characters c
JOIN models m            ON m.id = c.model_id
JOIN equip_pack_body_parts bp ON bp.id = c.preview_body_pack_id AND bp.model_id = c.model_id
JOIN skin_colors sc      ON sc.id = bp.skin_color_id AND sc.model_id = c.model_id
WHERE c.id = 1;
-- returns (1, 'nuian_male', 3, '로그인 스테이지 - 누이안 남', 1, 'custom01')
```

---

## 6. Unsure or not verified

1. **Race and gender id meanings** (`char_race_id`, `char_gender_id`) are inferred from model names. No enum or lookup table was found in the DB.
2. **Playable set** is the `creatable='t'` set (nuian, elf, hariharan, ferre). Warborn, dwarf and returned exist in the login2 cinematic entities but are not creatable.
3. **Face, hair and hair colour defaults** are unresolved (section 2.2). The DB values do not join consistently to face or hair assets. `total_character_customs.id=419` matches nuian M's `face_id` by model, and nothing else matched cleanly.
4. **Elf and hariharan login folders in the pak** (`costume/login001/`, `ha_*_login001`, nuian `nu_f_*_login001`) are not referenced by any default cloth-pack chain. The client may still use them through paths not in this DB.
5. **Login idle**: `fist_ac_stage_idle` is the likely login idle (section 4.3). The AnimObject `Animation="Default"` mapping is not resolved.
6. **Create scene male**: `create_<race>.lyr` only references the female base cdf. The male model is presumably swapped at runtime.
7. **No code that reads these tables** was found in `crates` or the `.cs` tree under `<server>`. All join semantics here come from data shape, not from code.
8. **Mesh contents of `*_base.chr`** (which body, face and hair meshes are inside each base `.chr`) were not parsed. The `.cdf` attachment slots are the only evidence.
