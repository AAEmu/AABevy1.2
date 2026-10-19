# Equipment attachment: why items do not sit right, and the fix plan

Research only, no crate code changed. Scratch tools and outputs are in `data/extract/equip/` (git-ignored):
`chr.py` (CompiledBones reader straight from `game_pak` by the `pak12.tsv` offsets), `mesh.py` (Mesh 0x800 streams,
subset bone tables, bone mapping, the same decode as `aa_cry::mesh`), `sweep.py` -> `sweep.json` (every armour visual
of the 8 playable models against its base skeleton), `ropes.py`, `cross.py` (Cut hull containment), `farbones.py`,
`q.py` (read-only DB queries), and the IDA runs `ida/` and `ida2/` (x2game, headless) and `ida_anim/` (cryanimation.dll).
Inputs: `docs/research/equipment-data.md` (item -> model chain), the sweep agent's in-progress findings
`data/extract/aa-anim/equip_sweep/findings.tsv` (its doc `equipment-sweep.md` had not landed when this was written).

## 0. Short answer

Skinning itself is right. Over all 13,017 armour visuals of the 8 playable models (`item_assets` x `item_armor_assets`,
files in the pak), **no part has a bone the base skeleton lacks**, every part bone maps to the base by controller id, and the
bind-pose differences that exist (mainly `Bip01 Spine`, 8 cm lower in 7,316 parts, unused by their vertices) do not matter because
aa-cry skins every part with the **base** skeleton's inverse bind matrices, as CryEngine does. A part therefore sits exactly where
it was modelled in the rest pose.

What goes wrong is the assembly around the skin: which pieces are drawn together, what cuts what, where rigid items hang,
and the few columns that move the whole figure. Ranked by the number of items affected:

| # | Root cause | Items / assets affected | Where |
|---|---|---|---|
| 1 | Cut hulls only cut the nude body; the engine also cuts the other skin attachments (pants inside boots, sleeves inside gloves, waist inside the jacket); x2game sorts the parts by `detail` and each part is cut by the ones before it (1.3) | every multi-piece outfit: 8,976 chest / legs / hands / feet visuals (about 1,120 per model), 360 of 565 costume visuals | `aa-anim/src/character.rs` `load_part`, `costume_hulls` |
| 2 | A costume (`cosplay`, wire slot 27) from the server keeps the nude body and every armour piece under it (x2game clears 6 armour slots under a costume, 4.2) | 222 costume items (578 visuals over the 8 models); for 205 of them the body has nothing cutting it at all | `aa-client/.../appearance.rs` `worn_items`, `aa-anim/src/character.rs` `resolve` |
| 3 | Two-handed weapons hang on `item_back_c`, whose +Y axis points straight back: the weapon sticks 1.3 m out of the back, level | 704 two-handed weapon visuals (sword, axe, mace, spear, blade); 2,049 `floating` findings of the sweep | `aa-client/.../weapons.rs` `sheath_bone` |
| 4 | Rope (cloth physics) bones are not simulated; with no clip track they freeze in the bind pose, so skirts, tassets and cloaks stay behind / stretch | 179 of 361 chest visuals, all 40 cloaks, 50 of 73 costumes, 26 helmets (model 10 counts) | `aa-anim` animator (no rope solver) |
| 5 | `item_assets.heel_offset_height` ignored: heeled shoes / costumes are modelled with the sole 8 cm under the floor | 379 visuals (124 nu_f, 118 el_f, 125 ha_f, a few others) | `aa-anim/src/character.rs` (model root), `db.rs` |
| 6 | Weapon visuals that are `.cdf` (carrier `.chr` + `CA_BONE` blade + effect) are passed to `load_model`, which fails: the weapon is invisible | 29 weapon items (200 sweep `bad_model` rows over 8 models) | `weapons.rs` `attach_carried` |
| 7 | Weapon attachment rotation uses glam `EulerRot::XYZ` (Rx·Ry·Rz); CryEngine `Ang3` is Rz·Ry·Rx (to be confirmed in the x2game decompile, see 4.7) | 522 weapon visuals with two or more non-zero angles (1H axes 166/169, drums 30/30, lutes 111/129, 2H maces 95) | `weapons.rs` `carried` |
| 8 | Underwear visuals are `.dds` textures (body diffuse replacements, with `more_asset_id` = the top), loaded as meshes: warning, nothing drawn | 68 visuals (8-9 per model) | `appearance.rs` / `character.rs` `resolve_part` |
| 9 | Dangling paths (file not in the pak) | 1.5-3 % of armour rows (661 sweep rows) | data; the real client cannot draw them either |
| 10 | Slots not drawn: backpack / glider (26), tail, glasses, beard | 437 backpacks, few body parts | `appearance.rs` `WORN` |

Not root causes (checked, see section 3): skeleton remap, extra bones, bind pose / inverse bind mismatch, race or gender
skeletons, the legacy 0x744 parts, helmet hair hiding (done in `hair.rs`), and most of the sweep's `pose_lag`, `stretched`,
`wrong_region` and `off_axis` findings (authored weights, Cut hull vertices, long robes).

The reel caster that rendered "pale and almost unclothed" is **not** this bug, see section 6.

## 1. How the real client attaches equipment

### 1.1 The character definition

Every player model is `objects/characters/<race>/<g>/nude/<pfx>_base.cdf`: `Model` = the `_base.chr` skeleton (162 bones for
nu_m), then an `AttachmentList` of named slots. `body`, `face`, `hair`, `head`, `chest`, `waist`, `legs`, `hands`, `feet`, `arms`,
`glasses`, `beard`, `cosplay` are `CA_SKIN`; `mainhand` (`item_hand_r`), `offhand` (`item_hand_l`), `ranged` (`item_hand_r`),
`back` and `backpack` (`item_back_c`) are `CA_BONE` with `AlignBoneAttachment="Bone"` (identity on the joint). Everything except
body / face / hair is `Flags="1"` (empty); x2game fills the slots from the equipment (`CharacterCustomizer::ChangeVisual`,
`ApplyChangeVisuals`). There is no slot for neck, ears, fingers, undershirt or underpants: accessories have no model, underwear is a
body texture.

### 1.2 Skin attachments

Item part `.cdf` files are only `Model` (a skinned `.chr` under `parts/<bp>/obj/`) + `Material`. The `.chr` carries a subset of
the base skeleton (helmet: `Bip01 .. Bip01 Head`, 8 bones; chest piece: 38 bones; cloak: `Bip01 Spine3`, `Neck`, the clavicles
and the `ropeCloak02/03_*` chains). Vertices are in model space; the bone mapping stream indexes the part's own bone list
(through the per-subset bone tables). CryEngine binds the part to the master skeleton by bone name / CRC and skins it with the
master's default pose. aa-cry does the same (`loader.rs` `prepare_model`: `own.map_to(&binding.skeleton)`, base
`inverse_bind()` for mapped bones).

Data check (`sweep.py`, all 8 models): 13,017 visuals in the pak, **0 with a bone missing from the base skeleton** (rope, item and
twist bones are all in the base skeletons: nu_m has 162 bones including `ropeRobe01..03`, `ropeSkirt`, `ropeCalf`, `ropeCloak02/03`,
`ropeHair*`, `ropeArm*`, `item_*`). Largest bind differences per part, after excluding `Bip01 Spine` and the `item_*` bones: under
2 cm for 12,572 of the 13,017 parts (96.6 %); the ferre `Bip01 R Finger12` (28 + 34 parts, ferre male / female) and `Bip01 Tail5` (7) are the only systematic ones, both on
bones that carry few or no vertices of those parts. With base inverse binds the rest pose is exact whatever the part's own binds.

### 1.3 Cut hulls

Many parts have a `Cut` sub-material (the `Cut.cfx` shader is `NoDraw`): a closed coarse hull. cryanimation.dll builds a cutter from
it when the model loads (`316095B0` / `316097E0`, "cutter mesh is not correctly triangulated") and
`ApplyCutterToCharacterInstance` (`3150ED30`) applies a cutter list to a character instance, LOD by LOD (up to 6) through the render
mesh. It is called for the master model (`31513740`, `this+0x2C`) **and for every skin attachment whose object is a character
instance** (`31514780`: attachment object type 2 -> its instance). So the engine cuts attachments, not only the nude body.

x2game decides who cuts whom (`CharacterCustomizer`, decompiled without full analysis in `data/extract/equip/ida2/dress_quick.txt`
and `q2.txt`; the cryanimation side in `ida_anim/anim2.txt`):

* `39320FC0` (called from `DressUpUnit` `3931B700`) builds one `(slot, detail)` entry per equip slot (28 slots, wire numbering:
  0 head, 2 chest, 4 legs, 5 hands, 6 feet, 7 arms, 8 back, 13/14 underwear, 19 face, 20 hair, 21 glasses, 23 tail, 24 body,
  25 beard, 27 costume). A slot gets an entry when it has a drawn part; `detail` is the full `item_assets.detail` (type bits
  included), 0 for hair and beard, and underwear 13/14 count even without a mesh. Then it widens three masks:
  `hands |= chest & 0x1C7E`, `arms |= chest & 0x1FF0`, `feet |= legs & 0x1F8E` (no chest: `0x4402`, no legs: `0x4800`), and
  `qsort`s all entries by **detail, descending** (comparator `39320270`: `b.detail - a.detail`).
* `393211A0` creates two cutter lists on the character (`ICharacterInstance` vtable `+76`, `31519D10`: `head` and `body`) and walks
  the sorted entries. For each armour / costume slot (`3956FC40`: 0..=14 or 27) whose attachment is not hidden it calls
  `IAttachment +180` (`31514780`: **apply the list so far to this part**) and then `IAttachment +176` (`31507670`: **append this
  part's own cutter**). Slots 0 (head) and 27 (costume) use the `head` list, all others the `body` list.
* Then the body parts are cut (`3956FC20`: 19..=25): face, hair, glasses and beard by the `head` list, the nude body (24) by the
  `body` list; the tail (23) is not cut (`+192`). Finally both lists go to the master model (`+64`) and are freed (`+80`).

So each part is cut by every part of its group that sorts before it, and the nude body by all of them. For the starter set on
nu_m (chest `0x5500`, legs `0x5020`, feet `0x4030 | (0x5020 & 0x1F8E)` = `0x5030`) the order is jacket, boots, trousers: the
boots are cut by the jacket's hull, the trousers by the jacket's and the boots' (the 356 trouser triangles inside the boot shaft
go), the boots keep the 712 triangles that lie inside the trousers' hull. Ankle shoes (`0x4010` -> `0x5010`) sort after the
trousers and are cut by them; trousers with `0x1040` sort before boots and cut them (trousers over boots).

Counts (8 models): Cut hulls are on every chest (2,882) and legs (2,343) visual, 2,703 of 3,143 helmets, most gloves and shoes,
360 of 565 costume visuals with a material; no cloak (320) has one.

### 1.4 `item_assets.detail`

`(detail >> 13) & 7` is the visual type: 1 underwear (texture), 2 body armour, 3 headgear, 4 cloak, 7 costume (`cosplay`).
Type 3/7 carry the hair and beard selectors (done, `hair.rs`). For type 2 the low 13 bits are a body-region mask that matches the
mesh content (model 10, `mesh.py`):

| slot | detail & 0x1FFF | meshes |
|---|---|---|
| feet | 0x0010 (19) / 0x0030 (233) | shoes / boots that also cover the shin |
| hands | 0x0080 (7) / 0x0180 (215) | hand only / glove with cuff (forearm) |
| legs | 0x1020 (190) / 0x1040 (102) / 0x0800-0x0840 | trousers |
| chest | 0x1400 (80) / 0x1500 (121) / 0x1600 (116) / 0x1a00 (29) | jacket to the waist (min z 1.0 m), longer coats (0.6 m), robes to the floor (0.01 m) |

The masks overlap exactly where two pieces overlap on the body: boots 0x0020 with trousers 0x1020 (shin), gloves 0x0100 with long
sleeves 0x1500 (forearm), jackets 0x1000 with trousers 0x1000 (hips), robes 0x0800 with trousers. That is the shape of a
"which part cuts which" rule; section 1.3 has the rule the decompile shows (sort by detail, the earlier part cuts the later).

### 1.5 Rigid items

Weapons, shields, instruments and backpacks are `CA_BONE` objects. Hand slots use `item_hand_r` / `item_hand_l` with identity
(`AlignBoneAttachment="Bone"`). x2game picks the sheath bone by a code in `39570E10` (decompiled, `data/extract/equip/ida/third.txt`):

| code | 3 | 4 | 5 | 6 | 7 | 100 | 101 | 102 | 103 |
|---|---|---|---|---|---|---|---|---|---|
| bone | `item_back_c` | `item_staff_l` | `item_bow_l` | `item_lute_r` | `item_book_c` | `item_side_l` | `item_side_r` | `item_back_l` | `item_back_r` |

Its other caller, `394E4DC0` / `394E4E80`, is the backpack / glider attachment: it sets `"item_glider"` or `"item_back_c"` on the
attachment (flag at `+0x30`). So `item_back_c` is the back-pack bone; the two-handed weapons use the left / right back bones
(codes 102 / 103). The animation data agrees:

* bind pose (`nu_m_base.chr`): `item_back_c` +Y = (0, -1, 0), straight back, level; `item_back_r` +Y runs from over the right
  shoulder down to the left hip; `item_back_l` mirrored. Weapon meshes lie along +Y (`sword_2h_01_1`: y -0.40 .. 1.33).
* `fist_ba_relaxed_idle` and `fist_mo_normal_run_f` key `item_back_r` / `item_back_l` (idle: local position (0.419, -0.173,
  -0.146) against the bind (0.532, -0.125, -0.191), i.e. pulled 12 cm closer to the back) but have no `item_back_c` track
  (`caf.py`, a port of `caf.rs`); the 2H holster clip is named `twohand_ac_holster_back_r`.

`item_assets.attachment_offset_pos/rot` (rot in degrees) are the per-visual offset on the sheath bone.

### 1.6 Heel height, more assets

`item_assets` is loaded in full by x2game (`SELECT id, allow_mirror, attachment_offset_pos_x .. rot_z, default_anim, detail,
heel_offset_height, hinge_damping, hinge_idx, hinge_limit, model_id, more_asset_id, name_tag_offset_height, path FROM item_assets`).
`heel_offset_height` is non-zero on 379 visuals; every one checked has its sole at z = -0.08 m (`nu_f_2bo202`, `nu_f_1bo203`,
`nu_f_1bf003_1bo801`, `el_f_1bo402` of item 20654, `ha_f_1bf004_1bo901` at -0.05 for 0.045), against -0.003 m for the bare foot and
0.000 for flat shoes. The client lifts the figure by that height.

## 2. Our pipeline (what it does with an item)

1. `aa-client/src/ingame/appearance.rs` `describe` -> `worn_items`: the 11 `WORN` slots plus `COSPLAY`, each turned into one
   template id (`selected_visuals`: look item first, all visual files must exist) -> `Costume::Items(ids)`.
2. `aa-anim/src/character.rs` `CharacterDesc::resolve`: `Costume::Items` -> `db.item_meshes(id, model)` for every id, each tagged
   with the slot name `"item"`; the `slot == "cosplay"` body removal only fires for `Costume::Login`. `resolve_part` unwraps a `.cdf`
   to `Model` + `Material` (+ its attachments).
3. `spawn_character_inner`: `costume_hulls` collects every Cut hull of every costume part; `load_part` applies them with a vertex
   hook **only when `part.kind == PartKind::Body`**. Each part is skinned to the base joints (`SkeletonBinding`).
4. `appearance::resolve` -> `resolve_with_headgear`: hair / beard variant from the cosplay or head visual's `detail` (correct).
5. Weapons: `weapons.rs` `carried` -> `CarriedItem { bone: sheath_bone(..), transform: offset + Quat::from_euler(XYZ, ..) }`,
   `attach_carried` loads `item.path` with `cry.load_model` and parents it to the bone.

## 3. Fifteen items traced

Model codes: nu_m 10, nu_f 11, el_m 16, el_f 17, ha_m 18, ha_f 19, fe_m 20, fe_f 21.

| # | Item (model) | Visual | What goes wrong, and why (data) | Cause |
|---|---|---|---|---|
| 1 | 23388 첫걸음 하의 + 23390 첫걸음 신발 (nu_m, the starter set) | `pt_leather055` -> `nu_m_1pt201.chr`; `bo_leather029` -> `nu_m_1bf001_1bo201.chr` | 356 of the pants' 876 visible triangles (238 / 563 vertices) lie inside the boots' Cut hull and are drawn over / through the boot shaft; the pants' own hull holds 712 of the boots' 1,440 triangles, so the order matters | 1 |
| 2 | 23387 첫걸음 상의 + 23388 (nu_m) | `ar_leather028` -> `nu_m_1ar306_1arm001.chr` | 22 pants triangles inside the jacket hull (waistband through the jacket), 220 jacket triangles inside the pants hull | 1 |
| 3 | 28439 sk_maid_butler001 / 29844 단정한 마음 (cosplay, nu_m) | `nu_m_sk_butler001.chr` (Illum + HumanSkin, **no Cut**, its own neck skin, 86 vertices) | server path: body stays (no hull to cut it) and the starter jacket / pants / boots are drawn too: three layers in the same space | 2 |
| 4 | 28998 sk_lord006 (cosplay, nu_m) | `nu_m_sk_lord006.chr` (skin 1,322 vertices from 0.9 m up, no Cut) | same as 3; also its `ropeCloak` / `ropeRobe` vertices freeze in the leap / sit clips | 2, 4 |
| 5 | 5660 첫걸음 대검 (2H sword, all models) | `sword_2h_01_1.cgf` (blade along +Y, 1.33 m) | on `item_back_c` (+Y axis = (0, -1, 0) in model space) the tip ends at (0, -1.48, 1.47): level, straight out of the back. On `item_back_r` the grip is over the right shoulder (0.33, -0.11, 2.11) and the tip at the left hip (-0.27, -0.26, 0.50), the in-game look | 3 |
| 6 | 13883 미노타우르스 양손도끼, 26514, 18404, 34659 (2H axes) | `axe_2h_*` | sweep: "sheathed on item_back_c: 40 cm from the nearest body vertex" | 3 |
| 7 | 5230 (nu_m chest) | `nu_m_1ar602_1arm001_1bt001_3sh003.chr` | 7 % of the vertex weight on `ropeRobe01/02`; the leap clip has no rope tracks, so the tassets hold their bind pose while the thighs move: 15 % of 2,283 body-touching vertices 7-36 cm behind | 4 |
| 8 | 16249 빨간색 졸업생의 망토 (cloak, all models) | `cp_cloth001` -> `nu_m_1cp101.chr` (43 % of weight on `ropeCloak02/03`) | the cloak is rigid on the spine; idle / run clips animate `ropeCloak02_01/05`, `03_01/05` only (clipinfo), leap none | 4 |
| 9 | 20654 수습공의 바다 천 신발 (el_f feet) | `bo_cloth147` -> `el_f_1bo402.chr`, `heel_offset_height` 0.09 | sole at z = -0.084: the shoe is 8 cm in the ground | 5 |
| 10 | 18200 sk_library001 (nu_f cosplay) | `nu_f_sk_library001.chr`, heel 0.09 | costumes carry the heel too; same sink | 5, 2 |
| 11 | 18661 뜨거운 맹세 (1H blade), 18663 달콤한 슬픔 (2H blade) | `blade_1h_46_5_e.cdf`, `blade_2h_36_1_e.cdf` (Model `real_torch.chr`, `CA_BONE` "sword" on bone `hand` = the blade `.cgf` with `_enchant` material, + an effect) | `cry.load_model(".cdf")` fails, `attach_carried` `continue`s: no weapon at all | 6 |
| 12 | 30060 샤크라의 마지막 원한 (shield), 28905 광폭한 붉은 용의 울음 (2H staff), 35524 (2H mace) | `shield_gorgon_002.cdf`, `staff_2h_dragon.cdf`, `mace_2h_36_7_v3.cdf` | same as 11 | 6 |
| 13 | 5961 첫걸음 도끼 (1H axe) | `axe_1h_01_1.cgf`, rot (-157, 180, -1) | XYZ intrinsic gives Rx(-157)·Ry(180)·Rz(-1); Rz·Ry·Rx gives about Rx(+157)·Ry(180): the two differ by ~46° about X on the hip (pending 4.7) | 7 |
| 14 | 6227 첫걸음 타악기 (drum) | `ins_p_01_1.cgf`, rot (180, 150, -7) | as 13 (every drum visual has three non-zero angles) | 7 |
| 15 | 30721 전나무 아가씨 축제 기념 속옷 (underwear) | `.../nude/*_in_pants00N_df.dds` (+ `more_asset_id` the top) | `resolve_part` makes a `.chr` part of a `.dds`; `load_part` fails with a warning; the underwear never shows | 8 |

Checked and fine: 6252 첫걸음 모자 / every helmet with `Bip01 Spine3`-weighted collars (`nu_m_1hm101`: 118 vertices from 1.60 to
1.69 m on `Spine3`, the coif; authored, so the sweep's helmet `pose_lag` is expected); el_f shoes `stretched` (the stretched edges
are the 24 Cut hull vertices, not drawn); `wrong_region` chest (robes to the floor, 1.77 m tall); `fe_m_4hm112` diving helmet
(`off_axis`: all 2,582 vertices on `Spine3`, authored).

## 4. Fix plan

Ordered by impact. File / function names are current `main` (another agent is editing `character.rs`, line numbers move).

### 4.1 Cut other skin parts, not only the body (cause 1)

`aa-anim/src/character.rs`:

* `costume_hulls` -> keep the hulls **per part** (`Vec<(slot, detail, Vec<Hull>)>`), not one flat list.
* Port the x2game rule of section 1.3 exactly (it is small):
  ```text
  key(slot) = item_assets.detail of the drawn visual (hair, beard: 0)
  key(hands) |= key(chest) & 0x1C7E   (no chest: 0x4402)
  key(arms)  |= key(chest) & 0x1FF0
  key(feet)  |= key(legs)  & 0x1F8E   (no legs:  0x4800)
  sort the armour / costume parts (slots 0..=14, 27) by key, descending
  head = [], body = []
  for part in sorted: list = if slot in {0, 27} { head } else { body }
                      cut(part, list); list += part's Cut hulls
  cut(face, hair, glasses, beard, head); cut(nude body, body)   // tail: never cut
  ```
* `load_part`: every part gets a `cut` vertex hook with its list (empty list: no hook), not only `PartKind::Body`.
* Data needed in `PartAsset`: the visual's `detail` and wire slot. `CharacterDesc::resolve` has them from `item_visuals`; carry them
  instead of the `"item"` tag. Face, hair and beard need the `head` list too (a hood's hull cuts the hair: today
  `hair.rs` only swaps hat variants).
* Performance: `cut_mesh` is per vertex against a few hulls; costume parts are ~1-4 k vertices, the body 2.6 k. Hooked loads are not
  shared (`MaterialHook`/`VertexHook` rule), so cache the cut result by `(part path, sorted cutter paths)`.

Check: `cross.py` pairs (starter set) should go from 356 drawn triangles inside the boots to 0; render `headgear`-style with the
starter set and compare the boot shaft.

### 4.2 Costumes from the server (cause 2)

* `appearance.rs` `worn_items`: when the cosplay slot has a drawable visual, drop the armour slots it covers. x2game does this in
  `39316BF0` (called from `39319B90` before every dress-up): when slot 27 is set it clears the **6 slots** listed at
  `39AC7358` and marks the body (24) for re-dressing; when the costume comes off it re-dresses the same 6 and the body. The table
  sits in `.data`, which is still encrypted in `x2game_unpacked.dll`, so its values are not readable statically; 6 matches
  chest, waist, legs, hands, feet, arms (2..=7) exactly, and the cut groups agree: head (0) is kept (the costume and the helmet
  share the `head` cutter list, the costume sorts first with type 7 and cuts the helmet), back (8, cloak) is kept. Read
  the table from a running client (6 dwords at `x2game+0xAD7358`) to confirm.
* Same function family: `39319B90` also clears slot 0 when the unit's "show helmet" flag (`+15286`) is off and slot 27 when
  "show costume" (`+15288`, the `ShowCosplay` option) is off; `39310A00` reads costume (else helmet) `detail` bits 3..6 into
  head-part flags.
* `character.rs` `resolve`: tag `Costume::Items` pieces with their slot (`"cosplay"` for slot 31 visuals: `item_armors.slot_type_id`
  31 or `(detail >> 13) & 7 == 7`), so the existing `slot == "cosplay"` rule drops the nude body for server costumes as it does for
  the login one. Every costume visual carries its own skin where skin shows (butler 86 neck vertices ... swimsuit 2,466), so the
  body is never needed under a costume.
* The option strings `ShowCosplay`, `ShowBackPackWithCosplay` exist in x2game: costume display can be turned off by the player and
  backpacks hide under a costume by default.

### 4.3 Two-handed weapons on `item_back_r` (cause 3)

`weapons.rs` `sheath_bone`: `_ if two_handed => "item_back_r"` (x2game sheath code 103; the bone the relaxed / run clips key; the
holster clip is `twohand_ac_holster_back_r`). Keep `item_staff_l` for staves (pose 4), `item_bow_l`, `item_arm_l`,
`item_side_l/r`. `item_back_c` is the glider / backpack bone (`394E4E80`), so the fallback arm `_ => "item_back_c"` should become
`item_back_r` too. Check with the sweep's `weapon_off_body` / `floating` (2,049 rows should drop to ~0) and against a real-client view
of a 2H sword at peace (grip over the right shoulder).

### 4.4 Rope bones (cause 4)

`aa-anim` animator: bones whose name starts with `rope` and that the current clip does not drive get a simple chain solver:
each joint a damped spring towards its animated parent's bind-relative pose plus gravity, with a capsule test against the thighs /
spine (`BonePhysics` of the base skeleton already has the limits and damping of the ragdoll setup: `aa_cry::skeleton::BonePhysics`).
Cheaper first step: when a clip has no track for a rope chain, keep the chain's world direction from the previous pose blended
toward straight down, instead of the bind-relative one. Affects robes, tassets, cloaks, long hair.

### 4.5 Heel height (cause 5)

* `aa-anim/src/db.rs`: add `heel_offset_height` to `ItemVisual` (the query already joins `item_assets`).
* `character.rs`: `CharacterAssets::heel` = the largest heel of the drawn visuals (feet, or the costume when it has one);
  `spawn_character_inner` adds `Vec3::Z * heel` (Cry space) to the `model` entity's transform, or to the root `Bip01` after
  sampling, so the hit box / ground placement stays. Check: el_f with item 20654 stands on the ground, no 8 cm sink.

### 4.6 `.cdf` weapons (cause 6)

`weapons.rs` `attach_carried`: when `item.path` ends in `.cdf`, load the CDF; spawn its `Model` (a small carrier `.chr` with a `hand`
/ `fire` bone, usually `real_torch.chr` with `torch_nodraw.mtl`) on the sheath / hand bone, then its `CA_BONE` attachments through
the existing `attachments::spawn` (object: the blade `.cgf` with the `_enchant` material; effect: `aa_fx`). `attachments.rs` already
handles the bone / align rules (`Attachment::bone_binding`).

### 4.7 Weapon rotation order (cause 7) and the x2game decompile

Euler angles in CryEngine (`Ang3`, `Quat::CreateRotationXYZ`) mean Rz·Ry·Rx (X applied first); `docs/aa-anim.md` records the same
order for the facial offsets read by `31591680`. glam's `EulerRot::XYZ` is intrinsic Rx·Ry·Rz. Proposed:
`Quat::from_euler(EulerRot::ZYX, rz, ry, rx)` in `weapons.rs` `carried`. Only visuals with two or more non-zero angles change
(522). Not confirmed in x2game yet: the function that turns `attachment_offset_rot_*` into a rotation was not found in the
decompiled dress-up path (`3935E610` / `39360430` decompile only partly without full analysis, and the `item_assets` loader
`396C89D0` did not decompile at all). The full x2game analysis (`data/extract/equip/ida/full.py`, saves `x2game.dll.i64`) was still
running when this was committed. Every CryEngine path that builds a rotation from degrees (`Ang3` -> `Quat::CreateRotationXYZ`)
is X first, so ZYX is the safe default; verify it on one item with two large angles (1H axe `axe_1h_01_1`, (-157, 180, -1)) in
the real client and compare where the axe head points on the hip (the two orders differ by about 46 degrees about X).

The heel height (4.5) is also not placed in the decompile yet; the measured soles (1.6) fix the sign and size.

### 4.8 Underwear textures (cause 8)

`character.rs` `resolve_part`: a `.dds` visual is not a mesh. Make it a `PartKind::Body` texture override: the body's HumanSkin
diffuse is the underwear texture (`*_underpants01_df.dds`, `*_in_pants00N_df.dds`); `more_asset_id` names the matching top
(`*_undershirt01_df.dds`). The textures are full body textures (the `nude/custom/` copies are listed in `body_diffuse_maps`).

### 4.9 Missing slots and files (causes 9, 10)

* `appearance.rs` `WORN`: add backpack (wire 26, `item_backpacks.asset_id`, a rigid `.cgf` / `.chr` / `.cdf` on `item_back_c`,
  hidden under a costume unless `ShowBackPackWithCosplay`). Tail / glasses / beard: bind through `Cdf::bind_body_part`.
* Dangling visuals: keep the silent skip, but log once per item at debug level so the sweep can tell "missing file" from "not
  attached".

## 5. Sweep findings: which are real

Of the sweep's categories (`findings.tsv`, 06:07 run): `floating` / `weapon_off_body` on two-handed weapons = cause 3 (real);
`bad_model` = cause 6 (real); `missing_file` = cause 9; `pose_lag` mostly cause 4 (rope parts: cloaks, robes, tassets) plus
authored weights (helmet collars on `Spine3`), not a skinning error; `stretched` = Cut hull vertices of shoes (not drawn) or rope
chains; `wrong_region` = robes; `bad_size` = NPC tool props (fishing rods, censers) used as weapons.

## 6. The reel caster ("pale and almost unclothed")

Not the same root cause, and not the Cut hulls:

* `Cut` hulls can only remove triangles of the nude body: `load_part` hooks them only when `part.kind == PartKind::Body`. A costume
  piece cannot be dropped by them.
* The caster (`Reelcasterx`, lane DB character 68, nu_m) wears 23387 / 23388 / 23390 (starter jacket, pants, boots), face 19838, hair
  34113 and weapons. All three armour visuals resolve (`ar_leather028`, `pt_leather055`, `bo_leather029`), the `.chr` and `.mtl` exist,
  all 17 textures of the three materials exist, and the skeletons map to `nu_m_base` (bind differences < 1 mm except `Bip01 Spine`).
* Combat captures of the same caster from the same morning (`shots/combat/cmp/beamb_10201.png`, 06:01; `shots/combat/beam/*`)
  show it dressed: jacket, pants, boots, bow on the back.
* Most likely: the new stealth look (`ingame/stealth.rs`, commits 93e393b / 53c0b4e) swaps **every** mesh of the local player to
  one shared translucent pink material while a `buffs.stealth` buff (71 rows) is on it; the clothes then read as pale skin
  (`shots/skill-audit/anim/bv_st_02.png` shows that look). None of the 13 reel plans fakes a stealth buff (`buff` steps checked
  against `buffs.stealth`), so it would come from a server cast (Shadowplay's Stealth) or from a ghost material that was not
  swapped back. Check the capture's skill and the unit's buff list at that time; if no stealth buff was on, grep the run log for
  `aa-anim: <path>:` warnings (a part that failed to load: `spawn_character_inner` warns and skips it).

## 7. Real-client check

Not done: it needs an own account and a character with GM-given 2H weapons, heeled shoes and a costume on the lane server, and the
RAM was at 5.7-6.7 GB free with two IDA runs going. The checks to do with one client (at most a few PNGs under
`reference/equipment/`): a two-handed sword at peace (expected over the right shoulder), item 20654 on an elf female (feet on the
ground), the starter set (boot shafts clean), a costume over armour (armour hidden; and which 6 slots, from the table at `x2game+0xAD7358` in the running
client), the 1H axe `axe_1h_01_1` on the hip (rotation order, 4.7), a lute and a drum on the back.
