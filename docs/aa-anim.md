# aa-anim: animations, characters, customisation

Crate `crates/aa-anim`. Decodes `.caf` animations, parses the text formats around them (`.cal`,
`.cdf`, `.lmg`, `*_targets.xml`), resolves a character from the game database, assembles it as an
animated entity hierarchy, and applies the character-create customisation. Depends on `aa-cry`
(chunk files, skeletons, meshes, materials, `CryAssets` / `spawn_loaded`) and `aa-pak`.

```text
cargo test -p aa-anim                                   # unit tests + real-data tests (skip without the pak)
cargo test -p aa-anim --lib decodes_the_whole_pak -- --ignored --nocapture   # all 25,202 .caf, ~15 s
cargo run  -p aa-anim --example lineup -- --shot shots/aa-anim/lineup.png
```

## Formats

### `.caf` (animation clip) - `caf.rs`

CryEngine chunk file, file type `0xFFFF0001`, container 0x744 or 0x745. 25,202 files, no `.dba`
in the 1.2 pak (the `.dbh` next to each race's clips is a 7-9 MB track database that the client
uses for fast loading; the loose `.caf` carry the same data and are all that is needed).

| chunk | content |
|---|---|
| Timing `0xCCCC000E` v0x918 | `f32 secs_per_tick (1/4800), i32 ticks_per_frame (160), char name[32], i32 start, i32 end`: 30 fps, `end - start` frames |
| Controller `0xCCCC000D` v0x829 | one per animated bone (25,045 files); older v0x827 / v0x828 exist in 399 env / monster clips and are skipped |
| `0xAAFC0002` / `0xAAFC0003` | motion parameters / foot plants (ignored) |

Controller v0x829 (after the 16-byte chunk header copy):

```text
+0  u32 controller_id        CRC-32 (zlib) of the case-sensitive bone name
+4  u16 rot_keys  +6 u16 pos_keys
+8  u8 rot_format  +9 u8 rot_time_format  +10 u8 pos_format  +11 u8 pos_keys_info (1 = own times)
+12 u8 pos_time_format  +13 u8 tracks_aligned     (16-byte descriptor)
+16 rotation keys, rotation times, position keys, [position times]
```

* rotation formats: `1` = f32 x,y,z,w; `5` = 48-bit small-tree quaternion (3x15 bits, index at
  bits 46-47); `8` = 64-bit extended small-tree (21+21+20 bits, index at bits 62-63). The three
  smallest components are quantised to ±1/sqrt2, lowest bits first in x,y,z,w order skipping the
  largest, which is rebuilt from unit length. Verified: only the 21,21,20 order keeps |q| <= 1.
* position format `2` = f32 x,y,z in metres; time formats `0` f32, `1` u16, `2` u8, in **frames**.
* values are the bone's **absolute local transform** (relative to its parent), replacing the
  bind pose. Idle clips set the root `Bip01` to identity at the origin and give `Bip01 Pelvis` its
  height (about 1.03 m) as a position track. Bones without a track keep their bind transform.
  Checked against the skeleton: the idle rotations match the bind rotations (dot > 0.97).
* quaternions are x,y,z,w; CryEngine model space is Z up, +Y forward... characters face -Y.

### `.cal` - `cal.rs`

Text name table per character: `name = fist\fist_ac_stage_idle.caf`, `#filepath`, `#BaseModelHeight`,
`#AnimModelHeight`, `$TracksDBH`, `$Include`. Elves and Hariharan first declare Nuian clips with
`#AnimModelHeight` of the Nuian model, then their own. Position tracks are scaled by
`BaseModelHeight / AnimModelHeight` of the entry. `*.fsq` lines are facial sequences, `*.lmg`
locomotion groups.

### `.cdf` - `cdf.rs`

XML `CharacterDefinition`: `<Model File Material>` plus an `AttachmentList` (`body`, `face`,
`hair`, ... `CA_SKIN`; `mainhand`, `back` ... `CA_BONE`). `*_base.cdf` binds the nude body;
part `.cdf` (costume pieces, `*_face00.cdf`) are only `Model` + `Material`. Rotation attributes are
`w,x,y,z`.

### `.lmg` - `lmg.rs`

Locomotion group XML: `BLENDTYPE`, `CAPS`, `Example Position="x, y, z" AName=...`, `TimeList`.
`Lmg::blend(dir)` returns the nearest two clips with inverse distance weights.

### Face morph targets - `morph.rs`

`nuian/*/face/nu_*_face01.chr` holds a `CompiledMorphTargets` chunk (`0xACDC0002` v0x801): 91 named
targets with deltas on the render vertices. `<race>/<gender>/face/*_targets.xml` maps slider
`Idx` to targets; the 128-byte `modifier` of a character holds one signed byte per `Idx`.

## Database tables used (`compact.sqlite3`) - `db.rs`

| table | use |
|---|---|
| `characters` | `creatable = 't'`: the 8 playable models (nuian 10/11, elf 16/17, hariharan 18/19, ferre 20/21) |
| `models`, `actor_models` | `models.sub_id -> actor_models.model_file` = base `.cdf` |
| `item_body_parts` + `item_assets` | face (slot 23) and hair (slot 24) meshes, `item_id -> asset_id -> path`; hair `asset_1..3` = hat variants, `hair_base` |
| `hair_colors` | `id` (unique) -> `.mtl` colouring one hair style of one model |
| `skin_colors` | bright / middle colour (`BSkinColor` / `MSkinColor`), diffuse / specular tint, ferre `custom_postfix` |
| `armor_assets` 1111..1114, `item_armor_assets`, `item_assets` | login costume: chest, legs, hands, feet per model |
| `total_character_customs` | `owner_type_id = 1`: the 9 character-create faces per model (hair, hair colour, skin, normal map, colours, 128 morph bytes) |
| `face_normal_maps`, `face_decal_assets`, `equip_pack_*` | face texture sets, decals |

`equip_pack_body_parts` (login-stage packs) references faces and hair by `item_body_parts.item_id`
(the research note misjoined them against `item_assets`), but its `hair_color_id` values belong to
other models; the model-consistent `total_character_customs` presets are used instead.

## API

```rust
let db = GameDb::open()?;
let desc = CharacterDesc::from_preset(&db, Race::Elf, Gender::Female, 0)?;   // or default_for
let assets = desc.resolve(&db)?;                                             // plain data: files
// in a system, with `CryAssets` and `Res<ClipLibrary>`:
let entity = spawn_character(&mut commands, &mut cry, &clips, assets, Transform::default())?;
```

Add `aa_cry::scene::CryPlugin` and `aa_anim::AnimPlugin` to the app. The `Animator` component
(on the character root) exposes `play`, `play_once_then`, `random_idle`, `speed`, `pose`.

## Requests to aa-cry

(see bottom of this file; kept current)
