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
cargo run  -p aa-anim --example compare -- --target step2_elf_female --shot shots/materials/step2_elf_female.png
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

Material / lighting comparison (`compare` example, `shots/materials/`): `after2_sheet.png` is the
contact sheet of the 8 `step1_<race>_<gender>` captures (real left, ours right; `b2_sheet.png` is
the same before the specular maps, per model skin calibration and ferre head fix, `before_sheet.png`
before the material work), `after2_step2_elf_female.png` the face close-up,
`after2_select_refone.png` the Nuian male Refone of the select screen. `compare` places the figure
by the real camera (`camera.json`: position, angles, roll, FOV 1920x1080) and measured feet / head
rows of each capture (figure height within 1 cm), lights it with `aa_cry::stage::StageLighting::login()`
and the login colour grading, and clears to the mean colour of the capture's background. Options:
`--target stepN_<race>_<gender>|select`, `--preset N`, `--costume none`, `--no-hair`, `--yaw deg`,
`--light K` (all lights), `--key/--fill/--rim/--env/--amb K`, `--sat K --contrast K`,
`--tonemap none|reinhard|aces|agx|blender|...`, `--bg r,g,b`. Side-by-side tools:
`python -I crates/aa-anim/tools/side_by_side.py <reference folder> <shot> <out> [x0 y0 x1 y1]` and
`sheet.py <shot prefix> <out> [scale]`.

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
3. **Materials**: aa-cry converts them from the shader parameters (`docs/aa-cry.md`); aa-anim loads
   every part with `Shading::Full` (per-pixel metal from the specular map) and tunes the converted
   sub-materials per character through aa-cry's `material_hook` (`restyle`):
   * HumanSkin: the chosen `skin_colors` row replaces `BSkinColor` (stored b, g, r) / `Diffuse`
     (`aa_cry::shade::skin_base_color`: the shader's `bright^1.5 * 1.5` lit colour, pulled towards
     pink and scaled; the calibration constants are in `shade.rs`), times a per model multiplier
     [`skin_tune`] (below); ferre fur has no skin sheen (`reflectance` 0.12, roughness >= 0.75).
     (The old `skin_albedo` scale is still computed and exposed but no longer used.)
   * face: texture composited on the CPU (`face.rs`): `face01/<pfx>_face01_df.dds` (the material's
     `face00_df copy.dds` does not exist in the pak) + decals + lip tint + iris tint, with a full mip
     chain; the face normal map of the preset replaces the material's.
   * lips: `face/<pfx>_face_mask_new.dds` (Nuian male only) or `<pfx>_face_mask.dds` (all others;
     before only the first was read, so only the Nuian male had tinted lips); the orange part of
     the mask (red and green set) is the lips. Skin-coloured lips are tinted 55% of the mask; the
     ferre lips are painted grey for the colour to replace and get up to 70%.
   * Eye: the same composited texture (the eye lives in the face atlas), iris recoloured from
     `left_pupil_color` (only its grey pixels: the pupil and sclera stay) inside the eyeball UV
     squares: the two square, equally sized pieces of the `Eye` geometry, either one shared rectangle
     (elf, nuian: both eyes use the same texels) or one each (ferre: two eyeballs side by side; the
     sub-material also draws teeth and tongue of the same atlas, whose union box covers the whole
     strip). `compose_face_eyes` takes the list, `compose_face` keeps its single `Option<EyeTint>`.
   * face mips are averaged as light (linear), not as sRGB codes, and decals are sampled with their
     colour weighted by alpha: sRGB averaging darkened every contrasty face texture (tattoos, scars)
     by up to a quarter in the mips a face uses at game distance, and the colour of a decal's
     transparent texels bled into its edges.
   * the composition (BC decode of the 512x512 texture, decals, lips, eyes, mips) costs 20-30 ms of
     CPU, so it is memoized and can run on a worker thread:
       - `face::FaceKey { base, layers, lips, eyes }` is everything the image depends on
         (`FaceKey::id()` is equal for equal keys, `FaceKey::compose()` composes without caching);
         `character::face_key(assets, mtl, model) -> Option<FaceKey>` builds it (`None`: the
         character uses the plain base texture).
       - `face::prewarm_face(&key) -> Result<bool>` composes into a global store (a no-op if it is
         there already; `true`: it composed) and `face::take_face(&key) -> Result<Image>` moves the
         prewarmed image out (else composes now). Both are CPU only and callable from any thread;
         `face::discard_face(&key)` drops an image nobody will take.
       - `character::prewarm_character_face(&CharacterAssets) -> Result<bool>` does it all for a
         character: loads the face part's model and material, builds the key, prewarms. Call it
         on `aa_cry::workers` before `spawn_character`.
       - `spawn_character` (`face_look`) takes the prewarmed image (or composes it) and only then
         calls `images.add` on the main thread; characters with equal keys share one texture
         (the second one costs nothing). The shared asset is looked up by id and not held by the
         cache, so the texture is freed with its last character.
   * Hair: aa-cry's conversion (`HairColor^1.5 * 2 * Diffuse`, alpha mask on the doubled alpha,
     isotropic highlight masked by the strand pattern; see `docs/aa-cry.md` for why not
     alpha-to-coverage / anisotropy); the colour `.mtl` of the chosen hair colour is the material.
   * Illum (cloth, armour): aa-cry's conversion; dyeable materials (`DyeingChannelMask`,
     `[1] Custom` mask) get a baked dyed diffuse (`dye.rs`) that replaces the base (and emissive) texture.
   Parts without a per-character change (most costumes) are the shared cached models; parts with
   one are built per character (hooked loads are not shared).
3b. **Cut hulls** (`cut.rs`): costume models carry a closed coarse hull under a `Cut` sub-material.
   The nude body triangles whose three vertices lie inside a hull (generalised winding number)
   are collapsed (degenerate triangles) by a vertex hook on the body part, so the body no longer
   shows through armour (thighs through pants, chest through a cuirass). The costume's own `skin`
   sub-mesh covers the exposed skin. Callers need no depth bias any more.
4. **Morphs**: `FaceMaker::weights(modifier)` -> target weights -> vertex deltas applied to the face
   mesh on the CPU before the Bevy mesh is built (`MorphSet::apply`).
5. **Animation**: `Animator` samples `fist_ac_stage_idle` (with `fist_ac_stage_rand_1` every few
   loops) using the `.cal` position scale of the model, in `PostUpdate` before transform propagation.

Face texture details (`face.rs`): decals are small BC3 images whose alpha is the shape, drawn at
`(defaultX, defaultY)` (+ move offsets for the scar) on the 512x512 face texture. Eyebrow and deco
decals carry a constant grey; their colour is `eyebrow_color` / `deco_color` (`0xAABBGGRR`).
Drawing order: tattoo, makeup, scar, eyebrow, deco. Lips: green channel of `<pfx>_face_mask_new.dds`
tinted by `lip_color`.

## Performance notes

* Characters of one skeleton file share one `Rig` (`pose::shared_rig(path)`, any thread), so `ClipLibrary::bound` caches the clip
  binding per `(rig.id, clip)` and aa-cry's skeleton-bound model cache hits across characters. `Cal::shared(path)` parses a `.cal`
  once. A character that is spawned costs the main thread about 10 ms instead of 45: the composed face
  (`character::prewarm_character_face`), the part models (`CryLoader::prepare_model`) and the clip set are prepared on a worker first
  (see `crates/aa-client/src/ingame/prepare.rs`).
* `Animator::interval` (seconds between updates, 0 = every frame) lets the caller animate far characters at a lower rate; the clip
  time still advances in real time. The in-game client uses 30 Hz beyond 25 m and 15 Hz beyond 45 m. Sampling runs in parallel over
  the animators (`par_iter_mut`, batches of 8), the bone `Transform`s are written serially. A crowd of 90 characters in view costs about
  0.9 ms of `animate` and, mostly, the transform propagation of their 14,000 bones (2.1 ms before the rate limit, 0.9 ms after) and
  Bevy's per skinned mesh joint extraction.
* `aa_anim::animator::take_stats()` returns the animators updated and the nanoseconds spent in `animate` since the last call.

## Partial-body layer (`Animator::play_layer`)

One clip can play over part of the body while the base clip (locomotion) keeps playing everywhere else: a cast or a shot on the
upper body while the legs run.

```rust
let mask = BoneMask::upper_body(&rig).unwrap();        // Bip01 Spine and everything below it in the tree
animator.play_layer(mask, clip, /* looping */ false, /* fade */ 0.1);
animator.stop_layer(0.12);                              // fades out (0 = at once); a one-shot ends by itself
```

* **Mask.** `BoneMask` is one weight per bone (shared, cheap to clone). `BoneMask::subtree(rig, "Bip01 Spine")` is 1 on that bone
  and its descendants (arms, hands, fingers, neck, head, the `item_*` and twist bones) and 0 on the pelvis, legs and root;
  `upper_body` is that, `with_weight` softens a bone (a join at 0.5). The 1.2 player skeletons (162 bones) all have `Bip01 Spine`;
  creature rigs without it get `None` (play the clip on the whole body).
* **Data.** `anims.move_ub` names the upper body variant of a clip. The `.cal` calls most of them `..._mub` ("move, upper body":
  `bow_co_sk_launch_ub` in the table is `bowow_co_sk_launch_mub.caf`) and they only carry tracks for `Bip01 Spine1` and up
  (51 to 60 tracks: no root, pelvis, spine or legs). A few are real `_ub` clips with the whole skeleton (`bow_co_sk_cast_ub`, 75
  tracks); the mask cuts those. 229 of the 329 `move_ub` names resolve in `global_player.cal` (199 via `_mub`, 30 as is); for the
  other 100 the client plays the clip itself through the mask.
* **Blend.** After the base pose (cross-fade included) is final, each bone the layer clip drives is lerped towards the clip by
  `mask weight * layer weight` (`Pose::sample_masked`: one lerp per driven bone, no second pose, no allocation). Bones the clip
  does not drive or the mask excludes keep the base pose.
* **Time.** The layer runs in real time: `Animator::speed` (which retimes the run cycle to the ground speed) does not apply to it,
  `paused` does. With the animation LOD the layer is advanced by the accumulated time like the base clip. The layer fades in over
  `fade`; a one-shot fades out over `fade` before its last frame and removes itself; a layer that replaces another continues
  from its weight (cast loop to shot without a pop).
* **Cost** (`cargo run -p aa-anim --example layerbench`, headless, 150 Nuian males with 162 bone entities each, the real `animate`
  system, per frame): no layer 0.50 to 0.54 ms (`animate`) and 2.1 to 2.2 ms with Bevy's transform propagation;
  a layer on every tenth unit the same within noise; a layer on every unit 0.63 ms (+0.8 us per layered unit; `animate`
  3.4 -> 4.2 us per update) and the same 2.1 to 2.3 ms in total. Units without a layer only test an `Option`. The LOD still
  scales it: at 30 Hz 0.21 ms (0.25 with layers on all 120), at 15 Hz 0.12 ms (0.15).

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
`Animator` (`play`, `play_once_then`, `play_layer`, `stop_layer`, `random_idle`, `speed`, `pose`) and a `Character`
(`assets`, `model_root`, `parts`). `Costume::{None, Login, Items(Vec<item id>), Meshes(Vec<path>)}`
picks the outfit; `GameDb::{playable, presets, skin_colors, hair_colors_for, body_parts, ...}`
list the choices. `Clip::load`, `Cal::load`, `Cdf::load`, `Lmg::load`, `MorphSet::load`,
`FaceMaker::load`, `Pose`, `Rig`, `ClipLibrary` are usable on their own.

`ModelOptions::shading` is set to `Shading::Full` for every character part.

`clipinfo` prints the bones a clip drives, `layerbench` times the animation system with and without layers.
The examples are the reference usage: `compare` (the calibrated login stage, above), `lineup` (`--preset N`, `--costume none`, `--no-hair 1`,
`--only N`, `--cam x,y,z --look x,y,z --fov deg`, `--light K`, `--env I`, `--ssao 1`, `--msaa off`,
`--prepass 1`: the last three check the materials under SSAO / the normal prepass) and `faces` (`--race`,
`--gender`, `--presets 0,1,2`, `--morph "4=35,25=65;..."`, `--skin`, `--hair`, `--hair-color`,
`--decals "282:1:1,313:1"`, `--colors "lip=ffbeb1ff,deco=..."`, `--normal 29:0.62`, `--nude 1`,
`--fov`, `--full 1`; the create-screen looks without the server: the in-game character `Bevylook1`
is `--race nuian --gender m --skin 1 --decals "282:1:1,313:1,588:1,681:1" --normal 29:0.62`).
Lighting note: Bevy lights are in lux against its default exposure; the
showroom uses ~0.5 of "nominal" values, higher washes characters out.

### Skin calibration (`skin_tune`)

`aa_cry::shade::skin_base_color` has one pink pull for every skin. The models differ (the elf's
pale face texture needs it, the Nuian male's skin is already pink and turned red), so
`aa_anim::skin_tune(race, gender)` is a table of linear rgb multipliers on top of it, one row per
model, plus `body_skin_tune` (ferre male: his body fur texture is greyer than the face's while the
capture is dark brown all over; elf female: see "Body skin" below) and a sheen for the skin of the
body and the costumes' skin parts (`BODY_SKIN_ROUGHNESS` 0.3 / `BODY_SKIN_REFLECTANCE` 0.7; the
face keeps roughness 0.5 / reflectance 0.4).

The face rows are the output of `data/extract/aa-cry/tmp/skintune.py` (not committed: scratch):
render the 8 `step1_*` targets nude and bald (`compare --costume none --no-hair 1`), take the
mean linear colour of the eye-to-chin region in the render and in the real capture (same
pixels), multiply each row by `ratio^0.8` and repeat until the ratios are within 5% (three
rounds: Nuian male 1.0 / 1.01 / 1.0). The Nuian male row is `(0.93, 1.20, 0.79)` (less red); the
Hariharan rows are about 0.7 (their skin was too bright), the Nuian / elf female rows 1.5-1.8.
The ferre male row was set by hand from the `step2` close-up (`step1` shows him turned about 25
degrees, so the same pixels are not his face): `(0.30, 0.33, 0.30)` face, `(0.75, 0.52, 0.45)` more
on the body. The table is between `BEGIN SKIN TUNE` / `END SKIN TUNE` in `character.rs`.

Face against body. The body textures are brighter than the face textures of the same model (mean
linear luminance of `nude_df` / `face01_df`: Nuian male 1.66, female 1.70, elf male 1.24, female 1.65,
hariharan male 1.20, female 1.06) and both parts share the tune, so in ambient or back light the
neck is lighter than the face (in game, `shots/create/ingame2.png`: the sun is behind the
character). Front lit, as on the login stage, the face measures *brighter* than the neck (0.34 vs
0.17 linear luminance, `faces --nude 1`), and the captures fix the face tone, so the two are not
equalised. Face decals do not change the skin outside their shapes (compared with and without in
`faces`, same pixels).

Body skin (elf female chest cutout). The starter costume `el_f_login001` has a heart-shaped cutout
on the chest. The body triangles under it are collapsed by the `Cut` hull, and the exposed skin is
the costume's own `skin` sub-mesh: a `HumanSkin` material (678 triangles, UVs on the chest of the
nude texture) like the body's. It rendered as a flat saturated salmon patch because the elf female
face tune (about 1.6 on every channel, fitted to the grey-pink face texture) was applied to the peach
nude texture: its red was pushed into the tone curve's shoulder (`(243, 161, 148)`, sd 1 in red,
against `(221, 165, 163)` with soft shading in `step2_elf_female`). Face lit by the same shader,
so the face looked right. The fix is a `body_skin_tune` row of the face / body texture ratio times
about 1.3, `(0.65, 0.78, 0.90)`, fitted to the mean of the two lobes, plus the sheen above (at the
face's roughness the chest is one flat colour, the capture has soft highlights). After: lobes
`(223, 167, 164)` / `(224, 170, 168)` against `(221, 165, 163)` / `(220, 161, 159)`, face, neck and the
other seven step 2 targets unchanged to the eye (the sheen is slightly glossier on the exposed
neck / chest skin of the others). The other races have no large area of body skin that disagrees
with its capture; if one shows up, its row goes in `body_skin_tune` the same way (face texture
mean / body texture mean per channel is the first guess, then iterate on the cutout pixels).

### Ferre heads (`FaceMaker::race_gender_weights`)

The ferre face files (`fe_{m,f}_face01.chr`) hold one neutral head for both genders and carry
`Male` (+0.14 m) and `Female` (-0.14 m) morph targets that are always on (like the elf's and
hariharan's `<race>_Morph`, which `race_weights` already returns). Without them the same head sat
at the wrong height on both bodies: sunk into the male's chest, floating above the female's
shoulders. Not a skeleton problem: `cargo run -p aa-anim --example bind_check -- --race ferre
--gender m [--all] [--pose] [--targets]` prints the bind displacement of every part against the
base skeleton (all 0.000 m), the face mesh bounds, the morph targets and the idle pose against the
bind pose, and the customisation colours of the preset. `face_dump` lists / sizes pak files
(`face_dump "game/objects/characters/ferre/female/face/*"`).

## Gaps

* Skin shading: the engine's HumanSkin shader (SSS map, rim, `bright` / `middle` colours per light
  angle) is replaced by one calibrated colour per model (`skin_tune`). `middle_skin_color` and the
  specular columns are not used. Ferre fur is drawn flat (no fur shells), so ferre skin is a
  brown tint of the grey fur texture; the ferre eyes are grey where the capture is blue (the
  eyeball texel region), the face is a shade lighter than the capture.
* Hair: the Kajiya-Kay streaks are a baked strand map, not a tangent highlight; no alpha-blend
  sorting (alpha mask); hat variants
  (`asset_1..3`) and beards are not implemented; preset hair colours come from the colour `.mtl` of
  the chosen style (`hair_colors`).
* The face texture is a CPU bake per character at spawn; the right pupil colour is ignored (one eye
  texture); `face_normal_map_weight` below 1 is not applied (Bevy has no normal strength).
* Only the idle clips play. Locomotion blending (`Lmg`), facial animation (`.fsq`) and lip sync, eye
  blinks, dye choice for armour other than the default dye are not done. The `.dbh` track database
  is not decoded (not needed).
* Costume specular: metal comes from the baked specular map; dielectric strength and (warm) tint
  from Bevy's `specular_texture` / `specular_tint_texture` (gold trims, leather streaks). Silver
  armour is still a bit duller than the captures.
* The `Cut` removal tests vertices against the hulls in the rest pose of the costume's own
  vertex positions; a body triangle that straddles a hull stays.
* The order of the 9 presets matches the screen by name; the order of hair / hair colour grids of the
  screen (`character_customizing_hair_assets`, `hair_colors` by `comment`) is exposed but the exact
  screen order was not verified.

## Requests to aa-cry (all done)

1. `CRY_TO_BEVY` docs say characters face CryEngine +Y (Bevy -Z): done.
2. Material and vertex hooks on `ModelOptions`: done, aa-anim uses them (no more per-character
   copies of shared parts).
3. Illum / HumanSkin conversion: done in `aa_cry::shade` (see `docs/aa-cry.md`).
4. ` copy` texture paths: done (`CryAssets::texture`, `CryLoader::resolve_texture`).
5. Environment light helper: `aa_cry::stage::environment_light` / `StageLighting`.
6. Dyeing stays in aa-anim (`dye::bake_dyed_diffuse`, applied by `restyle`); per-item dye colours
   for equipment attached outside `spawn_character` (weapons, bows) are not done.
