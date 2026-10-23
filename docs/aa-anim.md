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
   Triangles whose three vertices lie inside a hull (generalised winding number) are collapsed
   (degenerate triangles) by a vertex hook. The parts cut each other, not only the body; see
   "Equipment assembly" below for who cuts whom.
4. **Morphs**: `FaceMaker::weights(modifier)` -> target weights -> vertex deltas applied to the face
   mesh on the CPU before the Bevy mesh is built (`MorphSet::apply`).
5. **Animation**: `Animator` samples the selected body clip, supports cross-fades and a masked upper-body
   layer, and applies the model's `.cal` position scale before transform propagation. Spawn starts the
   login idle (with its authored variant); `aa-client` selects locomotion and action clips in game.

Face texture details (`face.rs`): decals are small BC3 images whose alpha is the shape, drawn at
`(defaultX, defaultY)` (+ move offsets for the scar) on the 512x512 face texture. Eyebrow and deco
decals carry a constant 0.5 grey and modulate the skin, `skin * colour * 2 * grey` with
`eyebrow_color` / `deco_color` (`0xAABBGGRR`): the brows darken the strands painted into the face
texture, and the nuian male deco (`nu_m_decal_deco002`, the stubble on chin, jaw and upper lip) shows.
A blend towards the flat colour turned blond characters' brows ginger and washed the stubble out.
Other decals blend their own colours. Drawing order: tattoo, makeup, scar, eyebrow, deco. Lips:
green channel of `<pfx>_face_mask_new.dds` tinted by `lip_color`. Iris: `left_pupil_color`
multiplies the grey of the iris disc in linear light (`eye.cfx`: `diffuse *= lerp(1, IrisColor,
gloss.r)`; the female elf's `face01` atlas has a teal painted iris, taken as its grey).

## Equipment assembly (costumes, Cut order, underwear, heels)

`CharacterDesc::resolve` reads every costume piece with its equipment slot (x2game / protocol
numbering, `db::wire_slot` from `item_armors.slot_type_id`; login packs by column, `pack_slot`) and
its `item_assets` row (`ItemVisual`: `detail`, `heel`, `more_asset_id`). `PartAsset` carries
`equip_slot` and `detail`. Research: `docs/research/equipment-attach.md`, `re-equipment.md`.

* **Costume** (slot 27, i.e. `slot_type_id` 31; decided by the slot, not by the `"item"` tag or the
  `detail` type: the ferre login jacket has type 7 too): the nude body is dropped and the armour
  slots of x2game's table at `39AC7358` are not drawn: head, chest, legs, hands, feet, back
  (0, 2, 4, 5, 6, 8; read from a running client). Waist, arms and underwear stay. This works for
  server costumes (`Costume::Items`) as for the login one; `aa-client` needs no change
  (`appearance/tests.rs` checks the server path).
* **Cut order** (`cut_lists`, x2game `39320FC0` / `393211A0`): the equipment parts are sorted by
  `detail`, highest first (`cut_key`: gloves `|= chest & 0x1C7E`, arms `|= chest & 0x1FF0`, boots
  `|= legs & 0x1F8E`; no chest `0x4402`, no legs `0x4800`). Each part is cut by the hulls of the parts
  of its group sorted before it, then adds its own. Groups: head (helmet, costume) and body (the rest).
  Face, hair, glasses and beard (aa-client adds the last two as parts named by their slot) are cut
  by the head group, the nude body by the body group, the tail not at all. Starter
  set on nu_m: gloves (0x5580) > jacket (0x5500) > boots (0x5030) > trousers (0x5020); 378 of the
  trousers' 888 triangles go (the shin inside the boot shaft), the jacket's cuffs inside the gloves,
  70 triangles. Trousers with `0x1040` sort before the boots and cut them (trousers over boots).
  A hood's hull cuts the face skin under its mask. Cut-only parts (no material hook) are shared
  between characters with the same pieces (`CUT_MODELS`, by part + skeleton + cutter paths, at most
  256). `RUST_LOG=aa_anim=debug` logs `<part>: N of M triangles cut by K hulls`.
* **Underwear** (`detail` type 1, `.dds` visuals): not meshes. `CharacterAssets::underwear` lists
  the textures (underpants first, then the top of the pair from `more_asset_id`); the body's
  HumanSkin diffuse becomes the first one and its `_ddn` sibling the normal map. The pair's two
  textures have the same RGB (a full body texture with both pieces painted); only the alpha differs,
  a gloss mask of each piece that the opaque skin material does not use.
* **Heel** (`item_assets.heel_offset_height`, 379 visuals, 0.01-0.09 m, nearly all female models):
  heeled shoes and costumes are modelled with the sole that far under the bind floor (`nu_f_2bo202`,
  `el_f_1bo402`, the nu_f login costume `nu_f_sk_saint001_h`: z = -0.08 against -0.003 for the bare
  foot; same skeleton, every bone within 0.05 m). `CharacterAssets::heel` is the costume's heel if
  the costume has a visual for the model, else the shoes' (x2game `39311310(model, cosplay, shoes)`
  returns the first asset's `+0x28`, 0 without one), and `character::model_transform` lifts the
  model root by it, so the soles stand on the floor. The evidence that the model is lifted: x2game's
  character camera (`392BD640`) and portrait renderer (`39413F40`) raise their camera by
  `heel(own cosplay, shoes) - heel(cosplay, shoes of the camera preset's cloth pack)` (`397A7360` ->
  `equip_pack_cloths` record, `+0x1C` cosplay, `+0x4C` shoes): the camera follows a head that is
  `heel` higher. No x2game code reads the field otherwise (the lift is not found in the dumps either;
  it is not in the shoe models, whose skeleton equals the base one). The create screen's step 1
  fit already expected it: `poses.rs` measured the nuian female as 1.81 m from the soles to the head
  top, 1.73 m of body plus the 8 cm heel. Side view: `shots/aa-anim/ch/heel_side_after.png`
  (`outfit --outfits "-;18180;8000028" --models 11 --view feet --yaw 90`: bare feet, heeled boots
  and the login costume all stand on the floor).
* Check with `examples/outfit.rs` (several items per figure, floor, `--view feet|legs|arms|head`),
  `equip_sweep --analyze --slot costume` (no `no_cut_hull` any more: the costume takes the body off).

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

## Rope bones (`rope.rs`)

Robes, skirts, tassets, cloaks, sleeves, long hair and the like are skinned to `rope*` bone chains (in every base skeleton:
nu_m has 21 chains, `ropeRobe01..03` and `ropeSkirt01/02`, `ropeAccpelvis01/02` and `ropeCalf01/02` on the legs,
`ropeCloak02/03` on `Bip01 Spine3`, `ropeArm01/02` on the forearms, eight `ropeHair*`; nu_f also has `ropeBreast01/02`).
CryEngine hands each chain to the physics as a rope pulled towards the animated pose. Without that they hung in the bind pose
whenever the clip has no track for them. The run clips drive the robe, skirt, cloak and some hair chains (the trailing is baked
in), the idles do not. That was cause 4 of `docs/research/equipment-attach.md`.

* **Data.** A chain is a bone named `rope...` whose parent is not, followed down its single `rope` child (no chain branches in
  the 1.2 skeletons). Its first bone's `alive` `BonePhysics` holds the rope parameters (the others have `flags = -1`), written by
  the exporter as degrees into radian fields: `max.x/y/z` in degrees are `stiffnessAnim` / `stiffnessDecayAnim` / `dampingAnim`
  (robe 500 / 0.8 / 0.4, skirt 200 / 0.95 / 1.5, cloak 500 / 0.8 / 0.4, calf 1000 / 0.95 / 3, hair 100-1000). `damping.x` is the
  damping (0.1-1), `damping.y` probably the air resistance (0.6-1), `damping.z` the thickness (0.01-0.03). These three are
  inferred, as is `spring_tension.z` (1-3, possibly the mass, unused). The decay follows `CRopeEntity`'s target-pose spring
  (`sub_350E2450`, `D:\aa-re\cryphysics\notes\entities.md` 4.2): vertex `i` of `n` gets `k * (1 - i * decay / n)`. There are no
  rope joint limits in the data (`min`, `spring_angle` are 0), so each family gets a fixed cone per segment around its animated
  direction (robe / cloak 75°, skirt 60°, calf / sleeve 45°, hair 35°, breast 15°). No `.chrparams` exist in the 1.2 pak; the
  `.cdf` files have no physics block.
* **Solver.** It is a Verlet rope per chain, in world space, so the cloth keeps its inertia when the character runs, turns or
  stops. Each step applies, per vertex, the absolute damping and a spring towards "simulated parent vertex + animated segment"
  (stiffness from the data, clamped to stay stable at the step). A damper pulls the velocity towards the animated vertex's motion,
  at the rate `dampingAnim + 2 * 0.35 * sqrt(k)`. Then, root to tip: the cone, the body capsules, and the exact segment length
  (follow the leader, the root vertex pinned to its animated bone). The air drag is left out because the clip already trails the
  cloth, so drag would count it twice. The simulation adds only the lag on starts, stops and turns, the sway and the swing
  outward in a turn, and the body pushing the cloth.
* **Gravity.** The authored shapes already hang under gravity, so a chain gets only the gravity its parent bone's tilt since the
  bind pose adds: `g - tilt * g`, with `tilt` = parent model rotation x inverse bind rotation. An upright body keeps every rope
  exactly in its authored shape at rest; a bent one lets the ropes hang towards the ground. With full gravity the weak tips
  drooped: hair 4-10 cm, and the breast chains flipped (k at the tip of a one-segment chain is `1300 * 0.05`). In a linear
  spring the constant part of gravity only shifts the rest position and does not change the dynamics.
* **Collision.** Twelve capsules sit on the animated `Bip01` bones, sized for a Nuian male and scaled by the head height:
  * thigh -> calf (8.5 cm) and calf -> foot (6 cm), each side;
  * pelvis -> `Spine3` (13 cm) and `Spine3` -> neck (12 cm);
  * neck -> head (5.5 cm), and the skull 6-13 cm up the Head bone's X axis (9.5 cm);
  * clavicle -> upper arm (6 cm) and upper arm -> forearm (5 cm), each side.

  They come in groups per family: robes, skirts, calf flaps and sleeves get legs + torso; cloaks also get back and arms; hair
  gets back (upper back, shoulders), neck and skull; breasts get the torso. Each chain keeps only the capsules it can reach
  from its root. Segments are tested, not vertices (the skirt segments are 57 cm long). Each segment / capsule radius is
  clamped to that segment's bind-pose clearance (minus 1 cm), so the rest pose is never pushed out. A push moves the segment's
  end by at most `1 / 0.3` times the overlap: an uncapped lever flung the short chains.
* **Only skinned chains.** `mask_ropes` (in `AnimPlugin`) simulates a chain only when one of its bones is a joint of some
  `SkinnedMesh` under the animator's entity. A part's joints are the bones of its own `.chr`, which lists exactly the bones it
  is weighted to (checked: nude body none, robe chest `ropeRobe01..03`, cloak `ropeCloak02/03`, the dress `ropeRobe01/02` +
  `ropeBreast01/02`). It is recomputed for a character when one of its skins is added or removed and when its rope is attached,
  so equipment swaps and asynchronously loaded parts are picked up. In the example 34 of 66 chains run; a cloaked nu_m runs 11
  of 21, most of them hair. A character assembly that knows better calls `RopeSim::set_skinned(|bone| ...)` (then the automatic
  mask leaves it alone); `RopeSim::active_chains()` lists them. Turn it off with `RopeSettings::skinned_only`.
* **Write-back and order.** `attach_ropes`, `mask_ropes`, `track_ropes` and `animate` run chained in `PostUpdate`. The rope runs
  inside `animate`'s parallel loop after the sampling (clip, fade and layer) and before the bones are written. Each chain bone
  gets the smallest rotation from its animated to its simulated segment direction; the tip bone keeps its local rotation. The
  skeleton's world transform is last frame's `GlobalTransform` of the model root, so the inertia lags the body by one frame.
* **Time and LOD.** It uses fixed steps of 1/60 s up to `RopeSettings::near` (20 m from the active 3D camera) and 1/30 s up to
  `far` (60 m). Beyond that it does not simulate (the animated pose shows) and restarts at rest on the way back. At most 4
  steps run per update, interpolating the targets between updates, so the animator's own LOD (30 / 15 Hz) still works. A jump of
  a chain root over 1.5 m in one update (teleport) restarts it at rest.
* **Wiring.** `AnimPlugin` adds a `RopeSim` to every new `Animator` whose rig has chains (the `RopeRig` is shared per rig).
  Opt out per entity with `NoRope`, or globally with `RopeSettings { enabled: false, .. }`. Hair chains are on by default
  (`RopeSettings::hair`). `rope::take_stats()` returns updates and CPU nanoseconds; `take_mask_updates()` counts mask
  recomputes.
* **Check.** `cargo run -p aa-anim --example rope -- --only 0|1|2` shows a long robe (`nu_m_ar_cloth147`), a winged cloak over
  a chest (`nu_m_cp_cloth016` + `nu_m_ar_cloth101`) or a dress costume with a braid (`nu_f_sk_party001_v101`). Each runs 3 s on
  a 3.5 m circle (about 7 m/s² sideways) and stands 1.5 s. `--follow x,y,z` rides the camera along (hair close-ups),
  `--no-rope` compares, `--crowd N` measures the cost. Unit tests: every chain of nu_m, nu_f and el_f rests within 4 mm of the
  authored pose and stays finite and inside its cone running a 2 m circle with dead stops.
* **Looked at.** The robe tails, the dress train and the braid swing out in the turn and sway and settle after the stop; the
  short male hair stays put. The dress train with ropes trails no further than the run clip's own baked train (side views
  with / without, same frames). The first version's "flat at ankle height" train came from air drag, now removed. The wings
  of `cp_cloth016` sweep back in the run clip itself. In the 2 m circle (12.5 m/s²) a 60° hair cone let the braid curl over the
  head, hence 35° for hair. No real-client motion capture was made: `reference/hair/` has stills only, and the client on the
  box belonged to another session.
* **Cost** (release, 1280x720, 60 Hz step, hair on). A cloaked nu_m (11 active chains, 7 of them hair) costs about 8 µs CPU
  per character update. With 30 running characters that is 0.24 ms CPU spread over the parallel loop, and `animate` takes
  0.30-0.31 ms wall against 0.20-0.22 ms without ropes. The fps is the same within noise (177 vs 176 and 163 vs 173 in two
  pairs of runs). One character: 12 µs CPU. A dev build (opt-level 1) costs about 1.5x.

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

### Face normal strength

`face_normal_map_weight` now controls the selected normal map instead of always drawing it at
full strength. This affects normal presets as well as the create screen's wrinkle slider (Nuian
male presets include weights 0.33, 0.62 and 0.73). At strengths below 1, `face_normal` bakes linear
RGBA tangent-space normals blended towards `(0,0,1)`; ATI2's two channels are decoded with aa-cry's
YX correction and Z is reconstructed before blending. Every source mip is retained. At strength
1 the original compressed texture is used. Equal live path/strength combinations share an image;
the cache stores asset IDs, allowing discarded preview textures to be freed. No caller changes.

Validation: 41 aa-anim unit tests passed, 1 whole-pak decode test remains intentionally ignored;
the four new tests cover strength endpoints/interpolation, ATI2 Z reconstruction, invalid weights,
and the real DDS's linear format/mip sizes. The existing `faces` example accepts `;`-separated
normal choices, so the same face can be checked side by side:

```powershell
cargo run -p aa-anim --example faces -- --race nuian --gender m --presets 0,0,0 --normal '35:0;35:0.5;35:1' --fov 14 --shot shots/aa-anim/normal_strengths.png --shot-after 8
```

`shots/aa-anim/normal_strengths.png` was inspected: left to right, the smooth face develops the
selected map's forehead and cheek creases progressively. Settled FPS was 190.4-192.3 with three
characters at 1920x1080, 168.4 during the capture. The reference
`reference/character_create/step2_nuian_male/real.png` was also inspected; the showroom check uses
different lighting and verifies the strength behavior, without changing existing skin calibration.

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
about 1.3 (`(0.65, 0.78, 0.90)`; `(0.43, 0.54, 0.69)` since the stage-frame fix, which made the stage's front light stronger), fitted to the mean of the two lobes, plus the sheen above (at the
face's roughness the chest is one flat colour, the capture has soft highlights). After: lobes
`(223, 167, 164)` / `(224, 170, 168)` against `(221, 165, 163)` / `(220, 161, 159)`, face, neck and the
other seven step 2 targets unchanged to the eye (the sheen is slightly glossier on the exposed
neck / chest skin of the others). The other races have no large area of body skin that disagrees
with its capture; if one shows up, its row goes in `body_skin_tune` the same way (face texture
mean / body texture mean per channel is the first guess, then iterate on the cutout pixels).

Refit after the stage-frame fix: `aa_cry::stage` lit the figure from behind (see `docs/aa-cry.md`), the key
is now in front, so every face was about 20% too bright; the `skin_tune` rows were refitted with the
procedure above (four rounds, ratios 0.95 - 0.99; ferre male kept by hand, scaled by the same 0.74).

Refit after the stage rig refit (key moved to the viewer's right, 52 degrees azimuth / 38 up, see `docs/aa-cry.md`):
two `skintune.py apply` rounds, then ratios 0.98 - 1.03 on the seven fitted models (nuian female 0.95 in blue, the
mask of its eye-to-chin region moves a little with the render); the rows moved by at most 8% (nuian female
+10-15% on green / blue, elf male -4%, hariharan male +7% on red). Ferre male kept by hand. The elf female
`body_skin_tune` is scaled by 1.06 (the chest cutout lobes read 196 / 143 / 152 in the render and 217 / 167 / 167 in the capture,
same mask on both).

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

## Authored facial sequences and eye blinks

`facial::{FacialSequence, ExpressionLibrary, Curve}` reads the original XML `.fsq`/`.fxl` assets.
The 1.2 pak has **894 FSQs, 9 FXLs, no FXXs**; a regression test parses every FSQ and FXL.
Sequence channels sum their Hermite splines, keep signed values and tangent flags, and ignore editor
folder/selection state. FXL controllers recurse into the original morphs through Catmull-Rom curves
or a constant multiplier; a spline controller does not multiply its output by `Weight`.
The engine's [facial animation description](https://www.cryengine.com/docs/static/engines/cryengine-5/categories/23756813/pages/23306439)
and Crytek-authored [channel implementation](https://github.com/ValtoGameEngines/CryEngine/blob/release/Code/CryEngine/CryAnimation/FacialAnimation/FaceAnimSequence.cpp)
and [effector implementation](https://github.com/ValtoGameEngines/CryEngine/blob/release/Code/CryEngine/CryAnimation/FacialAnimation/FaceEffector.cpp)
were checked for these contracts; the regression samples come directly from the 1.2 pak.

```rust
app.add_plugins(aa_anim::facial::FacialPlugin); // alongside CryPlugin and AnimPlugin
let root = aa_anim::spawn_character_with_facial(&mut commands, &mut cry, &clips, assets, transform)?;
// The root has FacialAnimator and Bevy MorphWeights. Existing spawn_character stays unchanged.
```

Human face01 meshes carry their static sliders, while their expression companions carry 28
original morphs; Ferre companions carry 33, including tongue movement. These include each eyelid,
brows and `m_a` / `m_b` / `m_e` / `m_f` / `m_i` / `m_o` / `m_u`. All **eight playable race/gender
heads** now support the overlay without replacing customisation, materials or skinning.

`facial/remap.rs` proves original vertex identity before copying any delta:

* Position and UV must coincide (position tolerance 0.00001 m covers original export float noise).
  Every target and source vertex must have a counterpart. Nuian female exports reorder vertices,
  split seams differently and combine source materials: vertex counts/indices alone are not IDs.
* Ferre M/F have the same 3,522 positions and UVs (maximum residual below 0.00000063 m), but change
  68 of 5,512 triangles. The 5,444 common triangles match exactly. Each of the five remaining
  connected patches (8, 26, 26, 2 and 6 triangles) must have the same vertex set, oriented boundary
  and triangle count. Only diagonals/fans change; reversed, closed, missing or non-manifold patches
  are rejected. No surface fitting or interpolation is performed.
* Coincident vertices must have identical morph deltas, unless an unchanged oriented incident ring
  uniquely establishes their surface identity. This distinguishes Ferre vertices 3397 and 3417:
  identical neutral position/UV, different incident rings and native internal IDs (2649 / 2745),
  with zero versus nonzero `m_f` deltas. Rings at a changed patch never resolve an ambiguity.
* Before using a source, its v0x800 `CompiledExt2IntMap` (u16 IDs) and v0x801 internal skin-vertex
  count are checked. Every external morph delta must exactly equal its internal morph delta through
  that map. Native internal IDs are local to an export, so they are not joined across files blindly.

Hariharan female has the original Nuian female template's `hariharan_Morph=1` baked into its neutral
positions. The overlay first proves the ordinary Nuian expression-to-template map, then applies only
that original race target to a temporary template and proves its map to Hariharan face01. Every one
of 3,024 target vertices matches position and UV; maximum residual measured 0.000000358 m. The
expression deltas themselves are unchanged, and the displayed Hariharan head is not transformed.
This is the same authored race baseline that `FaceMaker::race_gender_weights` uses where it remains
an unbaked target. Ferre's existing `Male`/`Female` baseline stays in the static customisation.

The verified remaps are cached per head. Failed vertex, patch, native-map or delta checks retain the
existing face and report the reason. Playable heads use `animations/pc_expressions.fxl`; older
race-specific FXLs contain prefixed names that do not match these heads/current FSQs.

`FacialAnimator` follows the current `Animator` CAF's matching `.cal` facial entry, using the same
clock (including body pause, speed, random idle variants and one-shot return). Missing facial entries
and a stopped body blend to neutral. `BoundClip::source` retains the normalized CAF path for this;
raw in-memory bound clips have no source. Sequence changes blend for `fade_seconds` (default 0.08).
For an independent authored expression/speech sequence:

```rust
face.play(FacialSequence::load("animations/nuian/male/facial/default.fsq")?, true);
face.time = 3.5;       // seek, in original sequence seconds
face.paused = true;   // hold a reference frame
face.stop();          // blend to neutral
face.follow_body();   // resume matching the body clip
```

The original stage idle includes the actual eye-close/brow curves, around 1.0-1.2 seconds in the
Nuian male's three-second loop. No blink interval or closure geometry is invented. Morphs run on
the GPU before skinning; frames update only the 28 or 33 GPU weights. Bone channels write joint
transforms before Bevy transform propagation. On this Bevy 0.20 GPU path, initial instance metadata can precede the morph descriptor. The
plugin refreshes each new facial mesh's instance metadata for three frames, then removes its marker.
No vertex uploads or ongoing instance refreshes occur. A uniquely named eight-second capture verified
that the authored blink/speech persist after those initial frames; the no-refresh comparison stays neutral. Morph attributes cost roughly 4-6 MB per enabled head on the GPU;
use the opt-in spawn for the player/nearby faces instead of enabling an entire distant crowd.

The original **1.2 expression stream is XYZ-only**. Its compiled internal/external morph records
contain vertex IDs and three position floats. The original `bin32/cryanimation.dll` function
`CSkinInstance::AddMorphTargetsToVertexBuffer_Verified` at `31502200` accumulates position deltas and
uploads exactly 12 bytes per vertex to the GPU morph stream (`MorphCopyToGPU`). The original normal
and tangent basis continues through skinning; no authored expression normal/tangent deltas are
present in these records. Later CryEngine's masked, eight-weight software tangent-recalculation
path is a separate contract and is not used to invent a shading correction for these heads.

Bone effectors now follow the original client contract in `facial/bone.rs`:

* `31591680` reads radian XYZ offsets (`Rz * Ry * Rx`). Attachment names bind only to actual
  skeleton joints, case-insensitively; missing joints are skipped.
* Channel/controller contributions sum at each effector's face-state index first. Bone effectors
  compose in their original `AllEffectors` order. `31599C60` linearly weights the quaternion toward
  identity, conjugates for negative weights, and extrapolates beyond unit weights. The individual
  weighted quaternions remain unnormalised until `3159AA40` has composed all of them.
* `3159A3C0` rotates the resulting model-space translation by the inverse default-parent rotation.
  `SkeletonPostProcess` (`3154B4B0`) calls `31599DC0` to reset active joints to the master default
  local pose, then `31599E80` right-multiplies that pose by the displacement. Thus active facial
  joints use the default pose; inactive joints retain the current body pose.
* Each evaluation restores the previous facial joints from the stored `Animator` pose before
  applying new offsets. Body pause, reduced update intervals, no current body clip, stop/fades and
  library changes cannot accumulate offsets on last frame's transforms. Morph/bone weights share
  the same sequence clock and interruption fade.

These numeric regressions execute the original DLL bytes in an isolated x86 Unicorn process,
including its `3166F5F0` SSE-mask CRT initializer. A fresh one-file `aa-pak` extraction matches
SHA256 `c630e58a62e36b33fd4dedad8917aad000474bf35d7054d9a2bbe00fb057ca6f`.
The bounded decompilations, `oracle.py` and `oracle.json` remain under
`data/extract/aa-anim/facial/native/`; golden tests cover signed/extrapolated weights, noncommuting
ordered composition, parent-space translation and default-pose multiplication.

Playable face CALs specify `animations/pc_expressions.fxl` and retain that selection. An explicit
library assignment (for example `face.library = ExpressionLibrary::load(path)?`) also rebinds bones.
The older Ferre FXL's `fist_em_cry` eye curves are real, but both root face00 and current face01
assign **zero vertices** to its `eye_left_bone` / `eye_right_bone` joints. Their bone transforms
evaluate accurately without visible gaze movement; the original skin weights stay unchanged.

Limits: skeleton-animation names are retained but are not started by FSQs.
Procedural/balance channels, sound scheduling, phoneme extraction/blending and
camera paths are not played. Existing FSQ speech/viseme curves can play; this is not automatic
voice-driven lip sync.

### Authored sound metadata audit

`FacialSequence::sounds` now preserves each original `SoundEntry` path, start offset in
seconds, optional sentence text, word intervals and the raw serialized phoneme records.
`sound_entries` remains available and counts modern plus legacy root-level entries. The
native loader appends a legacy root `Sound`/`Sentence` after modern entries, at time zero;
this ordering is retained. This follows the original 1.2 `CryAnimation.dll` sequence XML
loader at `315827C0` and sentence serializer at `3159DBC0`, inspected directly in IDA.
Native phonemes use comma-separated `start_ms:end_ms:phoneme` records; they are retained,
not interpreted or synthesized. The bounded native decompilation and asset inventory are
in `data/extract/aa-anim/voice/native.txt` and `audit.json`.

The complete shipped catalog has **894 FSQs and 9 FXLs**. Only **7 FSQs** contain audio
entries, all under `animations/nuian/male/machinima_facial/`, with start time zero. All seven
have empty sentence text, empty words and no phoneme records. Their six distinct referenced
`Game/Animations/el_02_*.wav` files are absent from the pak (one is referenced twice).
The regular playable-character facial sequences have no authored audio linkage. Thus these
assets do not provide automatic voice/phoneme lip sync. Existing explicit playback can
already follow an external audio clock with `follow_body = false`, `paused = true` and
`time = sound.start + audio_elapsed_seconds`; no redundant clock wrapper was added.
The normal FSQ sampling still clamps spline endpoints and never invents mouth motion.

Validation: `cargo test -p aa-anim --lib`: **59 passed, 1 ignored**. Regression tests check
native modern/legacy ordering, negative offsets, sentence/word/phoneme preservation,
non-finite offset rejection and the complete original catalog's seven empty sentences and
absent WAV references. No rendering behavior changed in this metadata milestone.

### Native NPC heads and head CAL library selection

`Cal::face_library` retains `$facelib` as a pak-relative path, including assignments from
included CALs. Facial preparation now reads the rendered head's sibling `.cal` instead of
unconditionally selecting the PC library; absent declarations retain the previous PC
fallback. All eight playable heads still select the same cached `pc_expressions.fxl`.

NPC chef `objects/characters/people/npc_chef01/face/npc_chef01_face01.chr` has its own **23**
authored speech/lid targets and no `face00` companion. Facial preparation now accepts such
native heads directly when both `m_a` and `eye_close_l` exist, after the same original
internal/external vertex-record validation used for overlays. Its target names and deltas
remain exactly equal to its original `MorphSet`; PC slider-only heads keep the proven
companion mapping. This supports native NPC expression data through the existing
`spawn_character_with_facial` API; it does not wire the client's separate NPC spawn path.

`facial --chef` uses the original chef `fist_ac_talk_11.fsq` at 0 / 1.7218 / 3 seconds.
Inspected `shots/aa-anim/facial_npc_chef_native_20261018.png` shows neutral, authored A and
return poses at **272.0–295.0 FPS**. This isolated-head diagnostic omits the CDF's separate
eye/hair/clothing attachments and uses the existing showroom shading; it is a morph check,
not a complete NPC appearance comparison. The chef has no stage-idle CAF, so the diagnostic
logs that missing idle and holds its bind pose. The inspected PC regression capture
`shots/aa-anim/facial_pc_headcal_regression_20261018.png` retains open/blink/speech at
**228.4–252.7 FPS**. Both screenshots exit after eight seconds.

Current validation: **61 unit tests passed, 1 ignored**, including original NPC target
identity, nonzero original speech evaluation, all eight PC library selections and CAL
include/override semantics. Native helmet/hair selection is described below.

The public loaded-head integration path is now:

```rust,ignore
let plain = cry.load_model(head_path, &options)?;
let (loaded, face, weights) = aa_anim::facial::prepare_loaded_face(
    cry, head_path, body_cal_path, &plain,
)?;
let head = aa_cry::scene::spawn_loaded(commands, head_path, &loaded, &options);
commands.entity(head).insert(ChildOf(model_root));
commands.entity(body_root).insert((face, weights)); // same entity as body Animator
```

Call the factory once per independently animated instance, before spawning its head. The
returned `LoadedModel` has private cloned subset mesh handles with the actual GPU morph
descriptors/names installed; the cached input's meshes, materials, skeleton, bind poses and
model remain unchanged. Insert its `FacialAnimator` and `MorphWeights` on the body root;
`FacialPlugin` binds the new head mesh entities and performs the initial descriptor upload
refresh. Existing `ModelOptions`/body joint binding and the Cry-to-Bevy model-root transform
remain the caller's responsibility, as in the ordinary `spawn_loaded` CDF skin path. A
matching body CAF automatically selects its CAL's same-name FSQ and samples the body's
clock, including pauses. Unsupported heads return an error and can use the untouched plain
model. The client's separate NPC path is still awaiting this opt-in integration.

`facial --chef` now uses this public factory directly, without a synthetic CharacterAssets
wrapper or PC material customization. Inspected factory capture
`shots/aa-anim/facial_npc_loaded_factory_20261018.png` shows three independently animated
heads at **256.0–302.1 FPS**. The uncalibrated native skin material is brighter than the
earlier PC-tinted diagnostic; this does not change target geometry. The PC regression
`shots/aa-anim/facial_pc_factory_regression_20261018.png` retains open/blink/speech at
**228.7–278.2 FPS** before capture. Latest tests: **62 passed, 1 ignored**. The factory
regression verifies descriptor creation, unchanged cached input, distinct output mesh
handles across two preparations, 23 initial zero weights and original chef CAF/FSQ clock
following at 1.7218 and 3 seconds. The authored return has small signed values, retained
without forcing neutral.

Factory failure is allocation-free for Bevy assets: expression eligibility, head library,
body CAL, subset/vertex mapping and every morph descriptor are validated before cloned
meshes are inserted. Unsupported CDF body parts, a missing body CAL and incomplete subset
data all leave mesh/image/material/bind-pose counts unchanged (regression tested). This
allows the caller to attempt native head preparation with safe fallback to the plain model.
The successful result still retains exactly its private prepared mesh handles. Validation
after the guard refactor: **62 passed, 1 ignored**.

### Native helmet hair/beard selectors

`hair::HeadgearVariants::from_asset_detail` decodes the original packed appearance flags
of the **effective visible cosplay/head asset**.
The original `x2game.dll` `CharacterCustomizer::AdjustHairAsset` at `39315820` first uses
the cosplay descriptor (wire slot 27), then the head descriptor (wire slot 0).
`ChangeVisual` stores descriptors at `model + 0x1c + 4 * slot`; the preferred pointer
at `model + 0x88` is therefore cosplay, not an image-item override.
`HeadgearVariants::effective_detail(head, cosplay)` preserves that exact priority, including
a present cosplay descriptor whose detail selects ordinary hair. Only descriptor detail types
3 or 7 (`(detail >> 13) & 7`) select variants; other types/default absence keep normal hair
and beard. Hair uses `(detail >> 10) & 7`, beard uses `(detail >> 7) & 7`:

| Selector | Native result |
|---|---|
| 0 | normal `asset_id` |
| 1 | no attachment |
| 2–5 | `asset_1_id` through `asset_4_id` |
| 6–7 | normal `asset_id` |

A zero alternate ID falls back to the normal asset, not hidden hair. These rules are from
`39315710` and `394E0D10`; `39571050` confirms the names default/none/type1/type2/type3/type4.
The item-assets loader `396C89D0` copies SQL column 9 (`detail`) to descriptor offset `0x24`.
The bounded assembler listings and original-byte Unicorn oracle are under
`data/extract/aa-anim/hair-native/`. All **512** combinations of type/hair/beard fields,
cosplay precedence, absence defaults, normal/missing alternate lookups and the native zero
sentinel were executed against the original unpacked 1.2 binary from read-only RE notes.

`GameDb::item_visuals(item_id, model_id)` exposes asset ID, path and detail in the same order
as `item_meshes`. The latter retains its existing path-only API. For the selected helmet
visual, decode its detail; then `BodyPart::variant_asset_id` / `variant_path` selects the
original hair or beard body-part row. `Hidden` returns `None`; otherwise callers replace
the mesh path while retaining their style/colour setup. The character assembly path is
unchanged: selecting the actual visible cosplay/head descriptor and applying the result remains an
integration task. Beard selector decoding is supported, but beard attachment assembly is
not added. No names, material classes or guessed helmet hiding rules are used.

`hair_variants` renders female hairstyle item 24024, normal asset 10175 and all four distinct
original alternates 10292–10295, plus explicit none. Helmet item 22874/model 11 provides
asset 10663/detail 28744 and selects Type3 hair/default beard. Tests verify those database
rows, original pak paths, zero-ID fallback and native selector-table goldens. Latest
`cargo test -p aa-anim --lib`: **64 passed, 1 ignored**. This diagnostic isolates the authored
meshes without claiming a complete helmet-equipped appearance match.
Inspected final capture `shots/aa-anim/hair_native_variants_final_20261018.png` shows all
six choices with aligned labels at **321.2–405.4 FPS**, after the shared build lock cleared.

### Rigid CDF attachments (NPC eyes)

`cdf::Attachment::bone_binding(&Skeleton)` returns the named joint index and a Cry-space
joint-local `Affine3A` for a `CA_BONE` binding. `BoneAlignment` preserves the original
`AlignBoneAttachment` mode. The original loader `3150D870`, assembly `3150BA00`, constructor
`31513900` and attachment methods `315199E0` / `3150C530` establish these rules:

* `Bone`: the loader ignores serialized `Rotation`/`Position`; the created attachment is
  aligned to its joint, with only the declared uniform scale applied locally.
* Absent/other (`Model`): the serialized quaternion/position is an absolute character-space
  default, converted to local space by `joint.bind_model.inverse() * authored_transform`.
  The native loader only applies CDF scale in Bone mode.
* `Skin`: native surface projection is required and the helper returns an explicit error.

The helper rejects missing joints/non-bone attachments and leaves visibility, empty-slot
filtering, binding asset/material loading and animation to the caller. Spawn the rigid
binding with `cry_space = true`, then insert `Transform::from_matrix(Mat4::from(local))`
and `ChildOf(body_bones[joint])` on its loaded-model root. This reuses the existing animated
joint hierarchy and performs no second Cry-to-Bevy conversion. The local default remains
correct as body/facial animation moves the joint. Bounded original decompilations are in
`data/extract/aa-anim/attachments-native/`.

CDF pak-path normalization now collapses repeated slashes. The original chef left-eye
binding contains `face//npc_chef01_eye_l.cgf`; it now resolves to the real pak file. The
public `facial --chef` example loads both original rigid CGFs from its base CDF and uses
`bone_binding` directly. Inspected `shots/aa-anim/facial_npc_bone_eyes_20261018.png` shows
both eyeballs correctly seated across neutral/A/return heads at **260.2–294.1 FPS**. This
remains an isolated-head diagnostic with uncalibrated native materials; NPC clothing/hair
appearance is not compared here. Latest own tests: **65 passed, 1 ignored**, including
original eye paths/joints, ignored Bone-mode offsets, uniform scale, absolute-to-local
composition, and unsupported Skin/missing-joint errors. Client CDF attachment assembly
still needs this opt-in helper; existing character assembly/calibration was untouched.

### Packet body items and blank NPC actor CDFs

An NPC actor CDF can intentionally leave its named body/face/hair attachments blank while
binding its rigid eyes. For example, model 631 uses
`objects/characters/people/npc_chef01/nude/npc_chef01.cdf`; its populated `_base.cdf` is a
different authored definition and is not an inferred replacement. The live server's
`npc/data.rs::apply_item_body_parts` produces model body items for the packet. The client's
`Unit.equipment` already retains the face/hair/body/beard item IDs; the separate non-human
creature producer previously consumed only the actor CDF.

`BodyPart::attachment_slot()` exposes the original native slot-name mapping. The original
`x2game.dll` routines `3956F160` and `3956F460` were executed in Unicorn to verify all 31
database-slot names and 28 wire-slot names: database 23–29 and wire 19–25 map to
face/hair/glasses/reserved/tail/body/beard. Artifacts are under
`data/extract/aa-anim/body-parts-native/`. Native `ChangeVisual` (`39315250`) looks up an
existing attachment slot and skips a missing slot rather than creating one.

`Cdf::bind_body_part(&BodyPart) -> bool` binds an explicitly supplied body-part item's raw
asset to its existing `CA_SKIN` slot. It replaces the binding/material override, makes the
explicit binding visible, and preserves other flags and unrelated slots. Missing/non-skin
or unknown slots return false without modifying the definition. The caller selects
compatible model rows and supplies actual packet item IDs. The helper does not choose
default equipment/body packs, create slots, replace CDF files, generate NPC appearances or
apply custom skin/face/hair texture colours.

Chef's explicit item IDs are face 14478, hair 14475 and body 14479 (all model 631). The
`facial --chef-items` example fills those three originally blank actor-CDF slots and renders
their original meshes with the public facial factory and rigid-eye helper. This is explicit
diagnostic input, not a claim that every chef packet supplies all three items. The actor
CDF names the left-eye CGF for both eyes; that authored binding is preserved rather than
silently substituted. Latest own tests: **66 passed, 1 ignored**, including original blank
slots, real item paths, native slot names, visibility of assigned slots, unchanged eyes and
equipment slots, and no mutations for unsupported/missing slots.
Inspected final `shots/aa-anim/facial_npc_packet_body_slots_20261018.png` shows the
original bound face, hair and clothing plus rigid eyes and authored neutral/A/return
speech poses at **246.0–309.3 FPS**. It holds the body bind pose for the facial diagnostic.

Validation: **57 unit tests passed, 1 whole-pak CAF test intentionally ignored**. The new tests cover
curve endpoints/duplicates, signed values, tangent modes, groups/multiple splines, recursive FXL
controllers, cycle limits, native blink/viseme keys, interrupted fades, body pause/unmatched/stop/
explicit-sequence transitions, native bone arithmetic and lifecycle, all 903 facial XML assets, and exact transfer for all eight
playable race/gender heads.

```powershell
cargo run -p aa-anim --example facial -- --shot shots/aa-anim/facial_nuian_male.png --shot-after 8
# Repeat the copied executable with --race nuian --gender f; --live follows the body idle.
```

The example selects original keys for open stage idle, the authored blink peak and an A speech
peak. Nuian female's default FSQ is neutral; its own `fist_ac_talk_11` supplies the speech frame. `reference/character_create/step2_nuian_male/real.png` contains a real
closed-eye frame and was inspected as a geometry reference. The showroom uses different lighting
and deliberately removes hair to make the lids visible; it does not recalibrate skin.
Inspected final captures are `shots/aa-anim/facial_startup_refresh_verify.png` (male) and
`shots/aa-anim/facial_startup_refresh_female.png`. At 1920x1080 with three frozen reference heads,
male FPS was 259.1-260.4 after the first 311.0 sample (212.3 during capture); female was
193.1-249.9 (240.9 during capture), with other builds sharing the machine. The copied executable is
`data/extract/aa-anim/facial-qa.exe`. Use unique screenshot names when iterating: repeated reads of
an overwritten path returned stale imagery during QA.

The three newly supported heads were also inspected against their matching original
`reference/character_create/step2_*` captures, using unique eight-second screenshots:
`shots/aa-anim/facial_hariharan_female_exact.png`, `facial_ferre_male_exact.png` and
`facial_ferre_female_exact.png`. Each shows open, blink and authored A speech frames. Hariharan
female settled at 245.4-261.0 FPS (184.9 during capture), Ferre male at 198.8-269.6 FPS
(243.5 during capture), and Ferre female at 209.2-243.6 FPS (196.5 during capture).
Ferre female's authored blink leaves a narrow eye sliver; the original deltas are retained.

Bone QA uses the same `facial` example: `--head` previews the original PC library's `head_x`
effector at -1, 0 and +1 (authored -50/0/+50 degree offsets). The diagnostic's held scalar keys
are explicit reference inputs, not an original FSQ head-motion claim. Inspected eight-second
capture `shots/aa-anim/facial_bone_head_reference.png` shows the skinned head turning correctly,
at 244.4-254.7 FPS (237.9 during capture). `--bones` instead plays the original Ferre male cry
FSQ with its older FXL at 0 / 0.239096 / 1.67368 seconds; the visually neutral eyes in
`facial_bone_ferre_cry.png` reflect the zero eye-joint skin weights described above (225.3-268.4 FPS,
194.2 during capture). Default-library regression captures `facial_bones_default_nuian_verified.png`
and `facial_bones_default_ferre_verified.png` still show the original open/blink/speech poses after
eight seconds: Nuian 238.6-250.1 FPS after the first 311.2 sample (244.3 during capture), Ferre
female 193.8-250.9 FPS (235.5 during capture). The three-frame startup refresh remains unchanged.

### Opt-in helmet application for resolved characters

`CharacterDesc::resolve_with_headgear(db, Option<detail>)` wraps the existing resolver in
`hair.rs` without changing character calibration. `CharacterAssets::apply_headgear(db,
detail)` applies the same native selection to already resolved assets and returns both
hair/beard selectors. They alter only the hair mesh path or explicit hidden state, retain
the selected style/colour and other parts, and restore the original hair after a hidden
helmet by applying detail zero. A bald description stays bald. Normal `resolve` behavior
is unchanged. The caller supplies the actual visible helmet's detail after equipment
visibility and image-item overrides; these methods do not hide helmet meshes themselves
or add beard geometry.

`hair_variants --helmets` exercises this resolver using original Nuian female hairstyle
24024, metal helmet item 22874 (Type3) and leather helmet item 18230 (None). Each visible
helmet's detail comes from the same `item_visuals` row used for the actual costume model.
The ordinary six-choice mode also uses the public resolver wrapper. Latest own tests:
**67 passed, 1 ignored**. Regression checks original selected mesh/material, other-part
preservation, actual hidden selection, restoration, no-helmet equivalence and a bald
description.
Inspected `shots/aa-anim/headgear_resolve_native_final_20261018.png` shows normal hair,
the original metal helmet with Type3 hair and the original leather hood with hidden hair
at **320.2–414.3 FPS**. This is an isolated mesh-selection diagnostic.

## Gaps

* Skin shading: the engine's HumanSkin shader (SSS map, rim, `bright` / `middle` colours per light
  angle) is replaced by one calibrated colour per model (`skin_tune`). `middle_skin_color` and the
  specular columns are not used. Ferre fur is drawn flat (no fur shells), so ferre skin is a
  brown tint of the grey fur texture; the ferre eyes are grey where the capture is blue (the
  eyeball texel region), the face is a shade lighter than the capture.
* Hair: the Kajiya-Kay streaks are a baked strand map, not a tangent highlight; no alpha-blend
  sorting (alpha mask). Native hat variants (`asset_1..4`) are available through the opt-in
  resolver above; full beard assembly remains a gap. Preset hair colours come from the
  colour `.mtl` of the chosen style (`hair_colors`).
* The face texture is a CPU bake per character at spawn; the right pupil colour is ignored (one eye
  texture).
* `aa-client` selects locomotion and action clips; `Lmg` parsing and direction sampling are available.
  Facial playback is opt-in and supports all eight heads and authored bone controllers (above).
  Automatic voice/phoneme lip sync and non-default
  armour dye choice remain gaps. The `.dbh` track database is not decoded (not needed).
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

### Visible head/cosplay priority and original visibility options

All **7204** playable-model head item/model groups in the original database have exactly
one visual row. Across all 10668 head groups the only duplicate is item 27519/model 19,
with two identical asset 16609/detail 25600 rows. This establishes unambiguous original
head descriptors for playable models; it is not permission to choose an arbitrary visual
when a future query returns distinct rows. The bounded audit is recorded in
`data/extract/aa-anim/hair-native/helmet-cardinality.json`.

The original `interface_option.alb` defines `ShowPlayerHelmet`, default 1, character-level
storage, calling `X2Unit:ShowHelmet`. Its native wrapper `394BB640` calls `391E8420`,
which updates the local player's helmet flag and refreshes head slot 0. Appearance assembly
`39319B90` clears that effective slot when the flag at unit+0x3bb6 is false, retaining the
equipped item. A separate `ShowMyCosplay` option calls `X2Player:ShowCosplay`; assembly
clears slot 27 separately using unit+0x3bb8. Thus hiding a helmet alone does **not** force
normal hair if visible cosplay supplies a descriptor. Callers pass visibility-filtered
slot details to `effective_detail` before `resolve_with_headgear`/`apply_headgear`.
Image-item substitution is an earlier descriptor-production concern; the two pointers
in `AdjustHairAsset` do not establish its policy. No automatic image substitution rule
is inferred here.

### Image-item substitution before descriptor selection

Native `394E58A0` reads the base item template from instance+8 and looks up its
template record through `39695DC0`. For native template-class values 1 or 2, or
class 22 with instance detail type 9, a nonzero image template at instance+0x19
replaces the base ID. It then calls `394E5630` exactly once with that chosen ID.
A missing substituted visual returns null; it does **not** retry the base item's
visual. Other classes/detail types keep the base ID. Class names are not inferred
from these numeric checks. The original-byte Unicorn regression
`hair-native/image-override-oracle.py` executes all 30 combinations of classes
0/1/2/3/22, detail types 0/1/9 and absent/present image IDs, verifying the exact
lookup arguments and null-return behavior. This is independent of the later
cosplay-before-head descriptor priority.

After the priority correction, the complete own-crate library check passes
**68 tests / 1 ignored** (`cargo test -p aa-anim --lib`).

### Explicit beard inventory and original NPC facial heads

The existing `Cdf::bind_body_part` handles explicitly received wire beard slot 25
(database body-part slot 29) through an already declared `CA_SKIN beard` attachment.
No additional playable-character beard API or inferred beard default is needed.
Original dwarf male CDF binds the real item 392/model 14/asset 180
(`dw_m_beard01.chr`). Its Type3 alternate is asset 181 (`dw_m_beard01_hm.chr`),
which is absent from this pak; the other zero alternates fall back to asset 180,
and selector 1 remains explicitly hidden. Ferre item 564/model 20/asset 229 and
orc item 584/model 40/asset 246 exist in the DB but their meshes are absent from
the original 1.2 pak. Database presence does not establish renderable geometry.
The real-asset regression verifies dwarf explicit binding, untouched other slots,
selector outcomes and all three missing paths. No cross-model substitute is used.

The complete set of NPC head CALs under `objects/characters/people/*/face/` is
chef face01, blacksmith face01/face02 and girl01 face01. The chef and both
blacksmith heads already pass the public loaded-head factory with their exact
23 native targets and `pc_expressions.fxl`. Girl01 exposed a concrete eligibility
omission: its head CAL selects `animations/people/girl01/facial/girl01.fxl`, and
its rendered head has 24 native `npc_*` targets, including `npc_a` and
`npc_eye_close_l`. The factory previously recognized only the `m_a`/`eye_close_l`
family and wrongly tried a nonexistent expression companion. It now recognizes
the authored girl family too, retaining the same native internal/external vertex
and delta validation. It uses this head's own targets with no renaming or transfer.

The public-path regression prepares all four original heads, verifies original
target order, subset morph descriptors and selected FXL, and samples their actual
CAL-linked FSQs. Girl talk_01/talk_02 FSQs are available for explicit playback;
they are not inferred aliases for the different body talk_11 etc. names. Normal
body following still uses only same-name authored CAL entries.
`facial --girl` uses the original girl's head, body skeleton and CA_BONE eyeballs
through the same public preparation API, showing original talk_02 samples.

Verified capture: `shots/aa-anim/facial_girl_native_final_20261018.png`, inspected at
0, 3.46667 and 4.16667 seconds of the original talk_02 FSQ. The head opens its
mouth at the authored A key and returns at the authored end; both original
eyeballs remain correctly attached. Shared-GPU FPS: **257.0?282.6**.
`cargo test -p aa-anim --lib`: **70 passed / 1 ignored**.

### Facial crowd allocation and lifetime audit

`facial_crowd` creates 32 identical original heads through `prepare_loaded_face`,
with separate joints, animators and root weights. Each starts the same authored
FSQ at a different phase. Default is Nuian male; `--girl` uses the girl's own
head/library plus native CA_BONE eyes. A live assertion checks that mesh weight
references address 32 independent roots and yield different current poses.
The PC diagnostic uses the head's plain authored material, without the full
character customization/material pass; this is an animation and allocation
fixture, not a complete character appearance comparison.

The targeted `repeated_heads_allocate_independent_meshes_and_sample_independent_poses`
regression measures 16 repeated preparations after source/topology/CAL warmup.
It verifies unique subset handles, no image/material/bindpose growth, independent
poses and stopping one instance without changing another. Morph payloads are
actual `MorphAttributes` counts times their native Rust size, including padding:

| Original head | Targets/subsets | Morph bytes per instance | Added meshes for 16 | Added images/materials/bindposes |
|---|---:|---:|---:|---:|
| Nuian male face01 | 28 / 3 | 4,049,472 | 48 | 0 / 0 / 0 |
| Chef face01 | 23 / 1 | 1,923,168 | 16 | 0 / 0 / 0 |
| Girl face01 | 24 / 1 | 1,097,856 | 16 | 0 / 0 / 0 |

Bevy 0.20 does not expose an independent asset handle for these immutable targets:
`bevy_mesh/src/mesh.rs` owns `Vec<MorphAttributes>` inside each `Mesh`, and
`try_set_morph_targets` takes an owned vector. Its `bevy_render/src/mesh/morph.rs`
allocator creates images per `AssetId<Mesh>` on the uniform fallback and uses
mesh-allocator storage on this Vulkan device. A new aa-anim descriptor cache would
retain another copy while every independently handled mesh still needs its own
owned target vector and GPU allocation. No such redundant cache was introduced,
and no sharing by path alone bypasses custom geometry/subset identity validation.
Sharing GPU target storage across distinct mesh IDs would require a renderer
change. Sharing the actual mesh asset instead would require changing the current
mesh-ID-to-owner binding contract and respecting customization identity.

The `prepared_meshes_are_released_when_instance_handles_drop` regression uses a
real minimal Bevy `AssetPlugin`: after 16 prepared model/animator values drop, one
`app.update()` removes all their mesh IDs. No prepared handles are retained in a
static cache; the ordinary plain-model cache remains separate. Unsupported-input
fail-fast allocation regressions continue to pass.

At 100 Nuian heads the morph payload alone is about **386 MiB**, before other
mesh/render allocations. Sampling cost is modest; unrestricted remote facial
opt-in remains a memory/visibility policy decision for the client. This audit
does not enable it automatically.

Representative optimized-dev measurements on the shared Ryzen 5600X machine:
16 warm preparations took **30.80 ms** (Nuian), **14.27 ms** (chef) and
**8.48 ms** (girl). 9600 active face updates took **23.38 / 26.61 / 28.10 ms**
respectively, about **2.44 / 2.77 / 2.93 microseconds per face**. These are
diagnostic timings, not asserted performance thresholds. The microbenchmark
measures expression evaluation/update only, with explicit FSQ playback; it excludes
ECS scheduling, body animation, facial bone restore/apply, culling and rendering.
The live captures include the actual facial plugin and bone hierarchy.

Inspected captures: `shots/aa-anim/facial_crowd_pc_20261018.png`
(**349.6?413.1 FPS**) and `shots/aa-anim/facial_crowd_girl_20261018.png`
(**290.5?386.6 FPS**), both on the shared RX 9070 XT. The girl run additionally
reports 32 subset references, 32 independent roots and 32 distinct current poses.
Its extra two mesh assets and one image are the shared original eyeballs,
separate from the measured zero-image-growth facial factory.

Full own-crate verification after the audit: **72 passed / 1 ignored**.

### In-place facial opt-in for an existing Character

`facial::enable_character_face(world, root) -> Result<bool>` enables only an
already spawned ordinary `Character` with an awake `Animator`. Repeated enable
returns false without allocating. Existing spawn-time facial playback (including
the local player) is rejected rather than replaced. `disable_character_face(world,
root) -> bool` undoes only this helper's attachment; it leaves spawn-time facial
characters alone. Both are exclusive world operations suitable for a deferred
`commands.queue` on an eligibility transition, not per-frame mesh rebuilding.

The helper finds the face through `Character.assets.parts` and the corresponding
`Character.parts` entity, validates its `CryModel` path, one rendered LOD0 node
and direct skinned subset order from aa-cry's `spawn_loaded`. All checks and
morph descriptors validate before adding assets. It clones those **actual rendered
mesh handles** while their CPU data remains available. After GPU extraction,
aa-cry intentionally releases that data; the helper rebuilds only the selected
subsets through the same `aa_cry::scene::subset_mesh` converter and the existing
`character::face_morph` recipe (now crate-visible), applying the same static
race/gender and modifier weights. The regression compares every rebuilt vertex
attribute and index against the original styled Character mesh after simulating
GPU extraction. The contract is the Character's authored customization recipe;
untracked external mesh edits cannot be reconstructed after their CPU data is gone.
Materials, skin bindings, hierarchy, Animator and other parts stay untouched.
It never reloads a plain head over customization. Only the face subset `Mesh3d` handles change.
Each gets its explicit root weight reference and three initial upload refreshes,
since changing existing Mesh3d is not an Added<Mesh3d> event.

One private attachment component retains the original subset handles while enabled.
Disable restores them, removes only the helper's facial weights/references and lets
Bevy release the private clones. Facial bone channels remember the underlying
body pose only for joints they actually modify; detach restores it even if the
client has already parked Animator in `Asleep`. This bounded vector is reused,
with no per-frame mesh or target clones. Root despawn drops all attachment handles.

A concrete client policy can retain the existing local-player face and opt in at
most **one remote Character**: the server-confirmed `aa_net::state::Target.obj_id`,
matched to `RemoteUnit.obj_id`, with existing `cull_units` visibility and awake
Animator eligibility. That culler already applies the camera cone, draw distance
and 60-metre animation range; no additional arbitrary facial radius is needed.
Disable the previous eligible entity before enabling a different one, and disable
on hidden/asleep/removed target. The helper does not own this target/camera policy
and adds no aa-client/aa-net dependency. A missing/unsupported candidate stays
plain; retry only on eligibility/entity/appearance changes, not each frame.

The original-asset toggle regression modifies a rendered face vertex before enable,
checks it survives, preserves material IDs and unrelated characters, exercises
repeated/idempotent toggles and fail-fast invalid part indexing, and verifies
private mesh release after disable. The existing bone regression now also removes
Animator before detach and checks exact underlying body-pose restoration.
`facial_attach` compares plain, enabled authored A, and disabled/restored characters;
it waits three seconds before attachment so the plain meshes have already rendered.

Final capture: `shots/aa-anim/facial_attach_toggle_final_20261018.png`, inspected.
At 3 seconds, after plain GPU upload, the middle and right heads enable in place;
at 5 seconds the right head disables after its facial meshes have also rendered.
The final image shows middle authored A and the restored right head matching the
plain left head. Shared-GPU FPS **251.6-339.1**. Full library verification:
**73 passed / 1 ignored**. The only character.rs change is face_morph visibility;
existing material calibration remains outside this milestone.

### Complete original CDF attachment-kind and payload inventory

`cargo run -p aa-anim --example cdf_inventory` is a manual original-catalogue
regression. It reads only the 13,701 CDF files (3,489,412 bytes) through aa-pak,
asserts the totals below and writes an inspectable inventory to stdout. It is not
a default whole-pak geometry test. Sixteen malformed doll-pet armour part CDFs
are rejected by strict XML: they omit whitespace between File and Material
attributes. All sixteen contain no Attachment/AttachmentList tag, so they cannot
hide uncounted attachment kinds. Some also reference legacy singular `/animal/`
paths absent from this pak; no path alias or permissive XML rule is invented.

| Attachment kind/alignment | Total | Bound | Bound and visible |
|---|---:|---:|---:|
| CA_BONE / absent (Model) | 348 | 203 | 203 |
| CA_BONE / Bone | 1118 | 119 | 117 |
| CA_SKIN / absent | 939 | 153 | 148 |

There are **zero** Skin-aligned bones, CA_FACE, CA_CAMERA, CA_PROX or other
attachment kinds in these original CDFs. No surface-projection implementation or
transform fallback is justified by this inventory; BoneAlignment::Skin continues
to return an explicit unsupported error.

A separate native field is required for real assets: `AttachmentType`. Original
loader `3150D870` defaults descriptor+0x44 to **1 (object)**; case-insensitive
`effect` selects **5**, `entity` selects **3**. Unknown/absent values retain 1.
Assembler `3150BA00` case 5 creates a particle attachment from the Binding name,
rather than loading it as a model. Inventory: 2191 absent/default payloads,
**211 effect payloads** and **3 entity payloads**; all effect/entity entries are
bound and visible. A dotted effect identifier is not a missing pak file.

`cdf::BindingKind` preserves these numeric semantics and is exposed as
`Attachment.binding_kind`. Object paths retain existing normalization; effect
and entity identifiers retain their authored spelling/case. Explicit body-part
binding resets the payload to Object. `AttachmentKind` and `bone_binding` stay
unchanged: a CA_BONE effect uses the same native bone/default-model alignment.
No aa-fx dependency, emitter lifetime rule, forced looping/prime override or
entity-spawn behavior is introduced by this parser milestone. The client owns
resource creation and should dispatch the explicit payload kind before prewarm.

Original examples for client integration:

- `animals/horse/horse_lord.cdf`: effect_foot_lf/lr/rf/rr bind
  `abillity_skill_table_m.system_skill.lord_horse_foot_LF/LR/RF/RR`; default Model
  alignment against their authored finger/toe bones. The numeric regression
  proves joint-bind * returned-local equals the authored absolute transform.
- `elf/female/costume/el_f_sk_whiteday001/el_f_sk_whiteday001.cdf`: headring
  binds `X2_EFFECTS.doodad_effect.whiteday_custume_head_elf` at Bip01 Head.
  Its authored spelling and the same absolute/local transform identity are tested.
  This effect belongs to the costume-part CDF, so resolving only its Model/Material
  loses the attachment; client assembly must inspect the retained part definition.

Full paths above are relative to `objects/characters/`. Reproducible inventory:
`data/extract/aa-anim/cdf-attachment-inventory-verified.tsv` (ignored artifact).

Native particle-attachment lifecycle evidence (bounded, no guessed flag names):
`CEffectAttachment` constructor 31515900 stores the assembler's zero offset,
Cry +Y direction and unit scale, resolves the Binding effect and retains its
definition. Lazy creation 31517A40 builds its local matrix and calls adapter
315170B0 with `(effect, 0, matrix, 0x20000)`; the adapter converts Matrix34 to
QuatTS and invokes effect virtual+0x0c with that transform and flags 0x20000.
No explicit prime call or looping override appears in this constructor/spawn/update
path. The high flag's game-specific semantics remain unresolved; it is not evidence
for enabling aa-fx priming. The later official
[IParticles compatibility API](https://raw.githubusercontent.com/aws/lumberyard/master/dev/Code/CryEngine/CryCommon/IParticles.h)
identifies the compatibility boolean as independence, not priming; that later
header is corroboration only, not authority for the original custom high flag.

Update 31517CC0 releases/clears its emitter when the attachment's hidden predicate
is true; otherwise it lazily creates only for a null pointer and updates the
emitter world matrix. It does not restart a non-null finished emitter in this
path. Destructor 31515A90 calls emitter virtual+0x10 with false, then releases
the definition and emitter. This supports attachment-owned cleanup and no forced
replay loop; the client still owns its ECS parent/despawn implementation. Exact
source listings are `attachments-native/particle-{decomp,methods,spawn,spawn-adapter}.txt`.

Parser and original numeric checks: **75 passed / 1 ignored**. The manual complete
CDF inventory assertions also pass. Visual particle assembly remains a client/aa-fx
integration deliverable, rather than a claimed parser screenshot.

### Preserve attachments while unwrapping a character part

`PartAsset.attachments: Vec<cdf::Attachment>` retains the exact attachment
definitions when `CharacterDesc::resolve` unwraps a part CDF into its Model/Material.
A directly supplied CHR and the ordinary body constructor have an empty vector;
no similarly named CDF is inferred. Hair variant CDF resolution preserves the
same data and replaces the old variant's attachments when its mesh changes.
The selected path, material and dye semantics are unchanged. There is no aa-fx
dependency or automatic emitter creation in aa-anim.

The public character-resolution regression supplies the original elf female
white-day001 costume CDF, verifies the unwrapped model/material, retains its
headring Effect identifier and placement verbatim, and proves the returned local
transform against the shared base-skeleton Bip01 Head. Supplying the underlying
raw CHR explicitly has no inferred headring. Client assembly can now iterate
`Character.assets.parts[*].attachments` after Character is added, for in-game
characters and login previews alike. Payload dispatch and native hidden/emitter
lifetime handling remain client responsibilities.
