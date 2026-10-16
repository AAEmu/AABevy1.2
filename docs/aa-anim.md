# aa-anim: animations, characters, customisation

Crate `crates/aa-anim`. Decodes `.caf` animations, parses the text formats around them (`.cal`,
`.cdf`, `.lmg`, `*_targets.xml`), resolves a character from the game database, assembles it as an
animated, textured entity hierarchy and applies the character-create customisation. Depends on
`aa-cry` (chunk files, skeletons, meshes, materials, `CryAssets` / `spawn_loaded`) and `aa-pak`.

```text
cargo test -p aa-anim                                   # unit tests + real-data tests (skip without the pak)
cargo test -p aa-anim --lib decodes_the_whole_pak -- --ignored --nocapture   # all 25,202 .caf, ~15 s
cargo run  -p aa-anim --example lineup -- --shot shots/aa-anim/lineup.png     # 8 playable models, login costume, idle
cargo run  -p aa-anim --example faces  -- --race nuian --gender f --shot shots/aa-anim/faces_nuian_f.png
```

## Status

| | |
|---|---|
| `.caf` decode | all 25,202 clips decode (0 failures); rotation formats 1 / 5 / 8, position format 2 |
| `.cal` / `.cdf` / `.lmg` | parsed; idle (`fist_ac_stage_idle`) + random variant (`fist_ac_stage_rand_1`) play on every model; `.lmg` is parsed and `Lmg::blend` works but locomotion is not played |
| characters | nuian, elf, hariharan (harani), ferre x male, female: body, face, hair, eyes, login costume, animated |
| customisation | face sliders (morph targets), 9 create-screen presets, skin colour, hair style + colour, eyebrow / makeup / tattoo / deco / scar decals, eyebrow / deco / lip / iris colours, face normal map, costume dye |
| speed | `lineup` (8 animated characters, 3 shadowed lights, 2400x1350, RX 9070 XT, debug+opt profile): **about 150 fps**; one character close-up 150-200 fps. CPU-side per character: pose sampling of ~70 bones per frame; the face morph and the face / dye texture bakes run once at spawn, not per frame |

Screenshots (`shots/aa-anim/`): `lineup.png` (all 8 models side by side), `faces_<race>_<g>.png`
(five create-screen presets of one model), `faces_sliders_nuian_f.png` (same preset, different
sliders). The real client captures in `reference/character_create/` are the visual target; the
Nuian male / female (silver saint armour + cape), Harani (orange / red dyed jackets) and Ferre
(brown leather and feathers) costumes match them, faces are the same people (hair style, brows,
eyes, skin tone), the metal shine of the client (specular terms of the Illum shader) is
approximated with the game's own beach cubemap as environment light.

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
* quaternions are x,y,z,w. **CryEngine model space is Z up, metres, and characters face +Y.**
  `aa_cry::CRY_TO_BEVY` therefore leaves them facing Bevy **-Z**; the examples rotate each spawn by
  PI about Y to face a camera on +Z.
* `Pose::sample` with `lock_root_motion` holds the root position at the clip's first key (the
  stage idles have none, but walks / runs do).

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
targets with deltas on the render vertices (millimetres: the largest slider delta is 8 mm, so the
sliders change the face subtly; `hariharan_Morph` is 0.235 m, it reshapes the Nuian head for Harani).
`<race>/<gender>/face/*_targets.xml` maps slider `Idx` to targets; the 128-byte `modifier` of a
character holds one signed byte per `Idx` (`value/100 * Scale` weight; `minmax` sliders use
`MinMorph` for negative and `MaxMorph` for positive values). Race shapes `elf_Morph` and
`hariharan_Morph` are always on for that race. All races share the Nuian face mesh.

## Database tables used (`compact.sqlite3`, read-only) - `db.rs`

| table | use |
|---|---|
| `characters` | `creatable = 't'`: the 8 playable models (nuian 10/11, elf 16/17, hariharan 18/19, ferre 20/21); `preview_cloth_pack_id` = the login costume |
| `models`, `actor_models` | `models.sub_id -> actor_models.model_file` = base `.cdf` |
| `equip_pack_cloths` | the login-stage costume of a model: item per slot (`shirt_id`, `pants_id`, `glove_id`, `shoes_id`, `cosplay_id`, ...). Nuian: `cosplay` 8000028 (`sk_saint001_h`, silver armour + cape), Harani: `cosplay` 24583 / 14359 (`sk_hariharan001`), Elf and Ferre: 15548..15551 ("login stage" leather / fur outfit) |
| `item_armors`, `item_armor_assets`, `item_assets` | item -> per-model mesh `.chr` / `.cdf` |
| `dyeable_items`, `item_dyeings`, `dyeing_colors` | default dye of a costume item (`color_rgb` is `0x00BBGGRR`, linear intensities): Harani male orange, female red |
| `item_body_parts` + `item_assets` | face (slot 23) and hair (slot 24) meshes, `item_id -> asset_id -> path`; hair `asset_1..3` = hat variants (unused) |
| `hair_colors` | `id` -> `.mtl` colouring one hair style of one model (`asset_id` = the style's `item_assets.id`) |
| `skin_colors` | per-model skin rows: `diffuse_color` (dark vs light skin), `bright` / `middle`, `custom_postfix` (ferre `brop01`, `grap02`... select `<name>_<postfix>.mtl`) |
| `total_character_customs` | `owner_type_id = 1`: the 9 create-screen presets per model (hair, hair colour, skin, normal map, decals, colours, 128 morph bytes). Ordered by `name` they are the `<race>_face000N` of the screen |
| `face_normal_maps`, `face_decal_assets` | face normal map textures; decals (`category_id` 1 scar (movable), 2 tattoo, 3 makeup, 4 eyebrow, 5 deco) with default position |
| `equip_pack_body_parts` | login-stage face / hair ids per model, but its `hair_color_id` values belong to other models (the research note's join was wrong); the model-consistent `total_character_customs` presets are used instead |

## How a character is built

`CharacterDesc::resolve` (database + pak, no GPU) -> `CharacterAssets` (plain data) ->
`spawn_character` (needs `CryAssets`, `ClipLibrary`).

1. **Skeleton**: `<race>/<g>/nude/<pfx>_<g>_base.chr`; one entity per bone under a model root
   (rotated by `CRY_TO_BEVY`), animated by `Animator`.
2. **Parts**: body (`*_nude.chr`; dropped under a one-piece `cosplay` costume, which carries its own
   skin geometry), face (`*_face00.cdf` -> `nu_*_face01.chr`), hair (`.chr` + the `.mtl` of the
   chosen colour), costume pieces. Skinned parts are subsets of the base skeleton, matched by
   controller id. They use the **base skeleton's inverse bind matrices** (engine behaviour; needed for
   the Nuian head on Harani bodies).
3. **Materials** (aa-cry converts them generically; `restyle` copies and adjusts per shader):
   * HumanSkin: white base times `skin_albedo` (from `skin_colors.diffuse_color`, relative to the
     model's lightest skin; albedo 0.79 for Nuian male skin 1, 0.51 for the dark skin 21, 0.24 for skin 8000002).
     The face gets x1.2, the body x0.9 so the neck matches the face.
   * face: texture composited on the CPU (`face.rs`): `face01/<pfx>_face01_df.dds` (the material's
     `face00_df copy.dds` does not exist in the pak) + decals + lip tint + iris tint, with a full mip
     chain; the face normal map of the preset replaces the material's.
   * Eye: the same composited texture (the eye lives in the face atlas), iris recoloured from
     `left_pupil_color` inside the eye mesh's UV square.
   * Hair: `HairColor` of the colour `.mtl` times 0.55 (the shader's own diffuse is 0.07; the colour
     is mostly specular), no emissive.
   * Illum (cloth, armour): white base (the mtl `Diffuse` ~0.2 is a lighting scale), no emissive,
     reflectance 0.35; dyeable materials (`DyeingChannelMask`, `[1] Custom` mask) get a baked dyed
     diffuse (`dye.rs`).
4. **Morphs**: `FaceMaker::weights(modifier)` -> target weights -> vertex deltas applied to the face
   mesh on the CPU before the Bevy mesh is built (`MorphSet::apply`).
5. **Animation**: `Animator` samples `fist_ac_stage_idle` (with `fist_ac_stage_rand_1` every few
   loops) using the `.cal` position scale of the model, in `PostUpdate` before transform propagation.

Face texture details (`face.rs`): decals are small BC3 images whose alpha is the shape, drawn at
`(defaultX, defaultY)` (+ move offsets for the scar) on the 512x512 face texture. Eyebrow and deco
decals carry a constant grey; their colour is `eyebrow_color` / `deco_color` (`0xAABBGGRR`).
Drawing order: tattoo, makeup, scar, eyebrow, deco. Lips: green channel of `<pfx>_face_mask_new.dds`
tinted by `lip_color`.

## Customisation parameters (what the create screen sets)

`x2ui/customizing/*.lua` drives the native `X2Customizer` API; see the table in
`crates/aa-anim/src/custom.rs`. Short form:

| client | here |
|---|---|
| `SetCustomizingHair(i)`, `...HairColor(i)`, `...SkinColor(i)` | `CharacterDesc::{hair, hair_color, skin}` |
| `ModifyFaceParamValue(idx, value)` | `Customization::set_slider(idx, value)`; ranges and categories (`eyes`, `nose`, `mouth`, `shape`) from `FaceMaker::targets` (`TargetDef::{min, max, category}`), UI order in `modifier_list.lua` |
| `SetCustomizingTattoo / MakeUp(i, weight) / Eyebrow(i) / Deco(i, weight) / Scar(i, x, y, scale, rotate)` | `Customization::decals` (`Decal { asset_id, weight, scale, rotate, move_x, move_y }`) |
| `...EyebrowColor / DecoColor / PupilColor / LipColor (r, g, b)` | `Customization::{eyebrow_color, deco_color, left_pupil_color, right_pupil_color, lip_color}` (`0xAABBGGRR`) |
| `...FaceNormal(i, weight)`, `...FaceDiffuse(i)` | `normal_map`, `normal_map_weight`; `face_diffuse_maps` is empty in 1.2 |

## API

```rust
let db = GameDb::open()?;
let mut desc = CharacterDesc::from_preset(&db, Race::Elf, Gender::Female, 0)?;  // or default_for
desc.customization.set_slider(25, -40);            // FS_Width
desc.skin = 6;                                     // skin_colors.id
let assets = desc.resolve(&db)?;                   // plain data: files, colours, decals
// in a system, with `CryAssets` and `Res<ClipLibrary>`:
let root = spawn_character(&mut commands, &mut cry, &clips, assets, Transform::default())?;
```

Add `aa_cry::scene::CryPlugin` and `aa_anim::AnimPlugin` to the app. The root entity has an
`Animator` (`play`, `play_once_then`, `random_idle`, `speed`, `pose`) and a `Character`
(`assets`, `model_root`, `parts`). `Costume::{None, Login, Items(Vec<item id>), Meshes(Vec<path>)}`
picks the outfit; `GameDb::{playable, presets, skin_colors, hair_colors_for, body_parts, ...}`
list the choices. `Clip::load`, `Cal::load`, `Cdf::load`, `Lmg::load`, `MorphSet::load`,
`FaceMaker::load`, `Pose`, `Rig`, `ClipLibrary` are usable on their own.

The examples are the reference usage: `lineup` (`--preset N`, `--costume none`, `--no-hair 1`,
`--only N`, `--cam x,y,z --look x,y,z --fov deg`, `--light K`, `--env I`) and `faces` (`--race`,
`--gender`, `--presets 0,1,2`, `--morph "4=35,25=65;..."`, `--skin`, `--hair`, `--hair-color`,
`--fov`, `--full 1`). Lighting note: Bevy lights are in lux against its default exposure; the
showroom uses ~0.5 of "nominal" values, higher washes characters out.

## Gaps

* Skin shading: the engine's HumanSkin shader (SSS, rim, `bright` / `middle` colours) is replaced by
  a plain albedo scale; the `bright` / `middle` columns are not used (they do not explain the dark
  skins, `diffuse_color` does). Ferre fur is drawn flat (no fur shells), so ferre faces and bodies
  look paler than in the client.
* Hair: colour is an approximation (no Kajiya-Kay highlights, no alpha-blend sorting); hat variants
  (`asset_1..3`) and beards are not implemented; preset hair colours come from the colour `.mtl` of
  the chosen style (`hair_colors`).
* The face texture is a CPU bake per character at spawn; the right pupil colour is ignored (one eye
  texture); `face_normal_map_weight` below 1 is not applied (Bevy has no normal strength).
* Only the idle clips play. Locomotion blending (`Lmg`), facial animation (`.fsq`) and lip sync, eye
  blinks, dye choice for armour other than the default dye are not done. The `.dbh` track database
  is not decoded (not needed).
* Costume specular: the Illum look of the client (SpecularModifierInHDR, env map amounts) is only
  approximated by the game's beach cubemap; metal armour is a little duller than the capture.
* The order of the 9 presets matches the screen by name; the order of hair / hair colour grids of the
  screen (`character_customizing_hair_assets`, `hair_colors` by `comment`) is exposed but the exact
  screen order was not verified.

## Requests to aa-cry

1. `aa_cry::CRY_TO_BEVY` docs: say that characters face CryEngine **+Y**, i.e. Bevy -Z after the
   conversion (props authored facing -Y are the exception). Everything in aa-anim rotates by PI.
2. Material hook: `ModelOptions` could take an `Option<Arc<dyn Fn(&CryMaterial, &mut StandardMaterial)>>`
   (and a vertex hook `Fn(&mut MeshData)`) so callers can restyle materials and morph vertices inside
   `load_model` instead of re-running `subset_mesh` / `cry.material` themselves (aa-anim copies every
   part per character now).
3. Illum: `Diffuse` (~0.2) multiplied into the texture and `Specular = 1,1,1` with `SpecularLevel 2`
   (reflectance 1.0) make costumes dark and grey; `Emissive` 0.34 on metal armour washes it out. For
   textured Illum materials use a white base colour, cap reflectance (~0.35-0.5) and ignore emissive;
   that is what `restyle` does for characters. HumanSkin: `SKIN_ALBEDO` 0.22 is too dark for faces
   and body textures (aa-anim uses 0.5-1.0 depending on the skin colour).
4. `CryAssets::texture` could resolve the `... df copy.dds` style paths (names with spaces) by
   stripping ` copy`; the customisable faces reference such files.
5. Environment light helper: build a `GeneratedEnvironmentMapLight` from the material's
   `Environment` cubemap (`textures/cubemaps/beach_blur2.dds`) and expose it; characters look
   plasticky without it. aa-anim's examples do it by hand
   (`cry.texture("textures/cubemaps/beach_blur2.dds", true)` works as is).
6. Dyeing: `Illum` materials with `DyeingChannelMask` / `DyeingColor` and a `[1] Custom` mask texture
   (costume dye) need a per-instance baked texture or a shader; aa-anim has `dye::bake_dyed_diffuse`
   if aa-cry wants to own that.
