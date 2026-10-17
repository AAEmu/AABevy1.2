# aa-cry: CryEngine 3 assets of the 1.2 client in Bevy

`crates/aa-cry` reads ArcheAge 1.2's CryEngine assets from `game_pak`: DDS textures, `.mtl`
materials, `.cgf`/`.cga` static meshes and `.chr` skinned meshes with skeletons, and turns them
into Bevy assets and entity hierarchies.

Layers:

| module | Bevy? | what |
|---|---|---|
| `chunk` | no | chunk file container (`ChunkFile`, `Chunk`, `Reader`, chunk type ids `ty::*`) |
| `skeleton` | no | `Skeleton` / `Bone` (bind poses, controller ids), `controller_id(name)` |
| `mesh` | no | `Model` (nodes, meshes, subsets, skin weights, LODs) |
| `material` | no | `Material` (`.mtl` tree, textures, flags, public params) |
| `texture` | Image | `Dds` parser, `load_dds(bytes, DdsOptions) -> Image`, CPU BC decoder |
| `scene` | yes | `CryPlugin`, `CryAssets` (cached conversion), `spawn_model*`, `ModelOptions`, hooks |
| `loader` | yes | `CryLoader` (Send + Sync): the whole conversion on any thread, `prepare_model` -> `PreparedModel` |
| `shade` | yes | CryEngine shader parameters -> `StandardMaterial` (`to_standard_material`, skin colour helpers) |
| `stage` | yes | lighting for showing characters: `StageLighting::login()`, environment maps, colour grading |

All plain data uses glam types (the same types as `bevy::math`) and stays in **CryEngine model
space: right handed, Z up, metres; characters face +Y, their left is -X**. `aa_cry::CRY_TO_BEVY`
(`(x, y, z) -> (x, z, -y)`) rotates into Bevy's Y-up space; `spawn_model` puts it on the root.

## API

```rust
// plain data
let bytes = aa_cry::read("objects/characters/elf/female/nude/el_f_base.chr")?;   // from the pak
let file = aa_cry::ChunkFile::parse(&bytes)?;           // chunks: file.get(id), file.of_type(ty), c.reader()
let skel = aa_cry::Skeleton::load(path)?;               // LOD files without bones fall back to LOD0
let model = aa_cry::Model::load("objects/.../x.cgf")?;  // nodes, meshes, skeleton, material_name
let mtl = aa_cry::material::Material::load("objects/.../x.mtl")?;
let img = aa_cry::texture::load_dds(&dds_bytes, DdsOptions { srgb: true, ..default() })?;

// Bevy
app.add_plugins(aa_cry::CryPlugin);
fn setup(mut commands: Commands, mut cry: aa_cry::CryAssets) {
    let root = cry.spawn_model(&mut commands, "objects/env/.../house.cgf").unwrap();
    // or: cry.load_model(path, &opts) -> Arc<LoadedModel> once, then spawn_loaded(...) per instance
}
```

* `ModelOptions { material, lod, skeleton: Option<SkeletonBinding>, cry_space, keep_cpu_mesh,
  material_hook, vertex_hook, shading }`.
  `skeleton` binds a skin (`.chr` body part) to existing joint entities of a base skeleton
  (bones matched by controller id, then name); `cry_space` keeps the root at identity (for
  parents that are already in CryEngine space); `lod` picks `*_lodN` files / `$lodN` nodes.
* Spawned hierarchy: root (`CryModel`, `Transform = CRY_TO_BEVY`) -> per file node (`CryNode`,
  node transform) -> per drawn subset (`Mesh3d`, `MeshMaterial3d<StandardMaterial>`, `Aabb`).
  Skinned models: joint entities (`CryJoint`, local bind pose) under the root, meshes under
  the root with `SkinnedMesh`; the root has `CrySkeleton { skeleton, joints }`.
  `spawn_model_full` / `spawn_loaded_full` also return the joint entities.
* `CryAssets::texture(path, srgb)`, `::mtl(path)`, `::material(mtl_path, sub)` give cached
  handles. Textures whose path moved are found by file name (about 1% of material references),
  and ` copy` is stripped from names like `el_f_face01_df copy.dds` (which exist only without it).
  **Cube and volume maps are never bound to 2D slots** (that is a wgpu validation error): a
  material whose base colour names a cube map (e.g. `hdeco_table_h.cgf`) gets no texture there;
  `CryAssets::texture` returns `None` for them, `cube_texture(path)` returns the cube map.
* **Threads.** `CryAssets::loader()` returns the `CryLoader` (cheap clone, `Send + Sync`, shared
  caches). `loader.prepare_model(path, &options) -> PreparedModel` parses, builds meshes and
  decodes DDS on any thread (`PreparedModel::loaded()` already has final handles);
  `CryAssets::finish_model(prepared) -> Arc<LoadedModel>` inserts the assets on the main thread.
  `load_model` is `prepare` + `finish`, so both share one cache.
* **Hooks.** `ModelOptions::material_hook` (`MaterialHook::new(|cry_sub_material, &mut
  StandardMaterial| ..)`) restyles every converted sub-material, `vertex_hook`
  (`VertexHook::new(|&mut MeshData| ..)`, CryEngine space) edits vertices (morphs) or collapses
  triangles (index ranges of the subsets stay valid: set a triangle's three indices equal).
  A hooked load builds its own meshes and materials (the parsed file is still shared); both must
  be `Send + Sync`.
* **Shading level.** `ModelOptions::shading`: `Shading::Basic` (default) converts with uniform
  factors; `Shading::Full` additionally bakes the specular (gloss) map of `Illum` materials into
  a metallic / roughness texture (one cached DDS decode per material): for characters and
  close-up objects. Costumes and weapons look wrong without it (see below).
* **Stage lighting** (`aa_cry::stage`): `StageLighting::login().spawn(&mut commands, &mut cry,
  camera, frame)` spawns the three directional lights (key with shadows, fill, rim), the
  environment map (`GeneratedEnvironmentMapLight` from the `beach_blur2.dds` cube the costumes
  name) and the ambient light on the camera; `aa_cry::stage::login_color_grading()` is the
  matching `ColorGrading` component. Lights are in engine light units (`LIGHT_UNIT_LUX`,
  `LOGIN_EXPOSURE`); `environment_light(cry, cubemap, intensity)` and
  `material_environment(&Material)` are the generic helpers. Directions are in the stage frame
  (+Z towards the camera, +Y up, +X to the viewer's right).
* **Facing.** CryEngine characters face **+Y** (their left is -X); `CRY_TO_BEVY` leaves them
  facing Bevy **-Z**, so rotate the root by PI about Y to face a camera on +Z.
* `LoadedModel { model, skeleton, inverse_bindposes, material_path, parts: Vec<(node, Vec<Part>)> }`
  with `Part { mesh, material, aabb, skinned }`: shared handles for instancing; static parts are
  in the node's frame (`scene::render_transform(&model, node)`), skinned parts in model space.
  With a `SkeletonBinding`, `inverse_bindposes` are the base skeleton's (as CryEngine does it).
* Lower-level helpers: `scene::find_mtl`, `scene::subset_mesh[_with]`, `scene::to_standard_material`,
  `mesh::compute_tangents`, `chunk::Reader` (bounds-checked; also for CAF / other chunk formats).

## Formats

### Chunk files (`.cgf .cga .chr .anm .caf`)

```text
0x00 char[8] "CryTek\0\0"   0x08 u32 file type (0xFFFF0000 geometry, 0xFFFF0001 animation)
0x0C u32 version (0x744 or 0x745: the only two in the pak)   0x10 u32 chunk table offset
table: u32 count; 0x744 entries {type, version, offset, id} (size = up to the next chunk, the
table or EOF; the table may be at the end), 0x745 entries {type, version, offset, id, size}
```

Chunk data normally starts with a 16-byte copy of `{type, version, offset, id}`; `Chunk::data`
skips it when present (SourceInfo, BoneNameList, BoneInitialPos often have none).

Pak survey (versions): geometry 0x744 4,849 cgf / 111 cga / 75 chr, 0x745 6,846 cgf / 147 cga /
19,609 chr; caf 9,551 x 0x744, 15,651 x 0x745. Chunk versions: Mesh 0x800 (1 cgf + 75 chr use
the legacy uncompiled 0x744 mesh, see below), Node 0x823, DataStream / MeshSubsets / MtlName /
MeshPhysicsData 0x800, Helper 0x744, CompiledBones 0x801, CAF controllers 0x827/0x828/0x829.

### Node (0xCCCC000B v0x823)

`char name[64]; i32 object, parent, nChildren, mtl; u8 pad[4]; f32 tm[16]` (row vectors: rows
0-2 are the axes incl. scale, row 3 the translation **in centimetres**, x0.01); `f32 pos[3],
rot[4], scl[3]; i32 ctrl[3]; i32 propLen; char props[]`. Transforms are parent-relative and
apply to static geometry; skinned (`.chr`) vertices are already in model space and the node
transform is ignored (e.g. a 1.05 scale on costume nodes). Kinds: `$lodN ...`/`$LODn_...` = LOD
of the parent (drawn in the parent's frame); other `$...` (`$picking`, `$occlusion`, ...) and
helper chunks = helpers; `proxy*`, `$collision`, `PhysicsProxy` = physics, never drawn.
Compiled CGFs often have a `Merged` node with the merged geometry; the source nodes keep
hierarchy and helpers but their meshes have no indices.

### Mesh (0xCCCC0000 v0x800) and streams

`i32 flags, flags2, nVerts, nIndices, nSubsets, subsetsChunk, vertAnimChunk; i32 stream[16];
i32 physics[4]; Vec3 bboxMin, bboxMax; ...` DataStream: `i32 flags, type, count, elemSize,
reserved[2]`, then data. Used in the pak (all files): positions f32x3, normals f32x3, uvs f32x2,
colours u8x4 (vegetation: bending/AO data, not colour), indices u16, tangents 16 bytes
(`i16x4` tangent + `i16x4` binormal, /32767, w = ±1), bone mapping 8 bytes (`u8x4` ids +
`u8x4` weights /255). No QTangents, no P3S, no u32 indices occur (both are handled anyway).
Tangents are converted to Bevy's `(xyz, w)` with `w` chosen so `cross(N, T) * w` equals the
stored binormal, which makes Bevy's normal mapping match CryEngine's `float3x3(T, B, N)`.

MeshSubsets: `i32 flags, count, reserved[2]`, then `count x {firstIndex, numIndices,
firstVert, numVerts, matId, f32 radius, Vec3 center}`; with `flags & 2` (all `.chr`) a
`{u32 n; u16 boneIds[128]}` table per subset follows. **Bone mapping ids are subset-local**:
the global bone is `subset.boneIds[id]` (checked against CompiledIntSkinVertices).

Legacy uncompiled Mesh (0xCCCC0000 v0x744, 76 files, mostly old `.chr` parts such as
`objects/characters/ferre/male/parts/hm/obj/fe_m_1hm217.chr`): `u8 hasBones, hasColors, pad[2];
i32 nVerts, nTVerts, nFaces, vertAnim; {Vec3 pos (cm), Vec3 normal}[nVerts];
{i32 v0, v1, v2, matId, smoothGroup}[nFaces]; Vec2 uv[nTVerts]; {i32 t0, t1, t2}[nFaces]
(if nTVerts); {i32 n; {i32 bone; Vec3 offset; f32 weight}[n]}[nVerts] (if bones); u8 rgb[3][nVerts]
(if colours)`. The reader splits vertices per (position, uv) pair, groups faces by `matId` into
subsets, keeps the 4 strongest weights (normalised) and computes tangents
(`mesh::compute_tangents`, also usable for generated meshes). Bone ids index the legacy skeleton.

### Skeletons

CompiledBones (0xACDC0000) v0x801: 32 reserved bytes, then 324-byte records:
`BonePhysics phys[2]` (104 bytes each), `u32 controllerId` (CRC-32 of the case-sensitive name,
as zlib; = CAF controller id), `u32 limbId`, `char name[48]`, `i32 parentOffset` (relative
index, 0 = root), `u32 nChildren`, `i32 childOffset`, `Matrix34 boneToModel` (row-major 3x4,
metres). Parents precede children. The elf female base skeleton has 157 bones.

* 15,812 of 19,684 `.chr` have bones. 3,795 `_lodN.chr` have none: they use their LOD0's
  bone list (`Skeleton::load` does that).
* Skeleton files (`*_base.chr`, 17) hold a placeholder mesh: one 10 cm triangle at the origin,
  no `.mtl`. `load_model` does not draw single-triangle meshes of files with bones.
* Body parts (skin attachments) carry a subset of the base skeleton's bones in their own order
  (matched by controller id, then name: `Skeleton::map_to`), plus extra bones (hair, cloth).
  Their own bind poses can differ from the base (hair / face parts), and skinning them with
  their own inverse binds misplaces them. Like CryEngine, bound parts use the **base
  skeleton's** inverse bind poses (`ModelOptions::skeleton`); unmatched extra bones get
  their own joints under their parent.
* CompiledIntSkinVertices (0xACDC0005 v0x801: 32 bytes, then 40-byte `{Vec3 pos; u16 bone[4];
  f32 weight[4]; u32 colour}`) / IntFaces / Ext2IntMap are the physics/software-skinning copy;
  its positions are rotated 180 degrees about Z relative to the render streams. Not used.
* Legacy (97 files): BoneAnim (v0x290, 152-byte entries), BoneNameList (v0x745, `u32 count` +
  NUL-terminated names), BoneInitialPos (v1, `float[4][3]` per bone, translation in cm).

### Materials (`.mtl`)

Plain XML (38,294 files, all parse; 3 are Wavefront `.mtl` exports and parse too). Root
`<Material>` with `<SubMaterials>` (multi) or a single material; a subset's `matId` indexes the
sub-materials. Attributes: `Shader`, `MtlFlags` (bits in `material::flags`: 0x2 two-sided,
0x4 additive, 0x400 no-draw, 0x80000 64-bit gen mask), `GenMask` (hex with the 0x80000 flag,
else decimal), `Diffuse`/`Specular`/`Emissive` (linear floats), `Shininess`, `SpecularLevel`,
`Opacity`, `AlphaTest`, `GlowAmount`, `vertModifType`; `<Textures><Texture Map=... File=...>`
(`Diffuse`, `Specular`, `Normalmap`, `Bumpmap`, `Environment`, `SubSurface`, `Opacity`,
`Detail`, `Decal`, `Custom`, `[1] Custom`) with `<TexMod TileU/V OffsetU/V RotateW>`;
`<PublicParams>` (shader-specific, e.g. `HairColor`). Texture paths: lowercase, `/`, no
`game/` prefix, `.tif/.tga/...` -> `.dds` (97.3% of references resolve; the rest are missing
in the client itself, mostly other races' hair textures).

Material file lookup (`scene::find_mtl`): `.chr` -> `<name>.mtl` first; `.cgf` -> the MtlName
chunk's name (`<dir>/<name>.mtl`), then `<name>.mtl`. CDFs override it (`Material=`).

### DDS

36,817 files (header survey of the whole pak): DXT1 27,613 (33 cubemaps), DXT5 5,595,
A8R8G8B8 2,353 (1 cube, 1 volume), ATI2 1,125, DXT3 59, R8G8B8 38, X8R8G8B8 22 (1 cube), L8 7
(1 volume), A8 2, G16R16F 2, R32F 1. No split `.dds.N` files. `_ddn` normal maps are DXT1
(6,377), ATI2 (1,051), DXT5 (15), R8G8B8 (5) or A8R8G8B8 (1); `_sp` specular, `_df` diffuse.

* CryEngine appends `"CExt"` + tagged chunks + `"CEnd"` after the image (`{fourcc, u32 size,
  data}` each): `AvgC` (average colour, 1,211 files), `Flgs` (298 files, after `AvgC`; skipped),
  `AttC` (a complete attached DDS, 159 files: half-size A8 gloss for 124 `_ddn` maps, A8 masks
  for 35 others, e.g. puddles; `Dds::attached`). 23,569 files have an empty block, 11,975 none.
  The A8 of attached images is stored as fourcc 28 (`D3DFMT_A8`), which the parser accepts.
* **ATI2 stores Y in the first BC4 block and X in the second** (BC5 is X, Y). The loader
  swaps the halves of every block (lossless) so red = X, green = Y.
* BC1/2/3/5 stay compressed on the GPU (sRGB variants for colour maps); BGRA8 uploads as is;
  BGR8 / BGRX8 / L8 / A8 expand to RGBA8; G16R16F / R32F upload as float formats. The three
  BC textures that are not multiples of 4 (256x1, 128x1, 2x512 defaults) and GPUs without BC
  support use the CPU decoder (`Dds::decode_rgba8`). Truncated mip chains are cut to what fits.
* Images use a repeat sampler with trilinear + 4x anisotropic filtering.

## Shader -> StandardMaterial mapping (`shade::to_standard_material`)

Derived from the shader sources in the pak (`shaders/hwscripts/cryfx/{illum,humanskin,hair,eye,
fraglib,shadelib}.cfx`) and calibrated against the captures in `reference/`
(`cargo run -p aa-anim --example compare`, see `docs/aa-anim.md`).

What the numbers mean:

* `Diffuse` (`MatDifColor`) multiplies `texture * (ambient + Emissive + sum N.L * light)`. It is a
  **lighting scale in engine light units** (costumes 0.216, hair 0.067), not a dark albedo. The
  base colour stays `Diffuse * DiffuseModifierInHDR * texture`; `stage` gives lights in the same
  units (one unit shows a white Lambert surface as 1.0 before tone mapping).
* `Emissive` is not a glow: it is added to the *ambient accumulation* that the texture then
  multiplies. Converted as `emissive = Emissive * base` with the diffuse texture as emissive
  texture. (Bevy multiplies the emissive colour by the *sharpened alpha* of alpha-to-coverage
  materials, thousands in flat areas, so hair clears it.)
* `Specular` is the final specular colour, **already multiplied by `SpecularLevel`** (the face's
  `3.0, 2.82, 1.94` is `17,16,11 / 255 * 45`, bows `7.6` = grey 0.87 x level 8.8); it multiplies
  the specular (gloss) map, which is black for wood and leather and bright (often tinted: gold)
  for metal. `Shininess` is the Phong exponent of the sun highlight.
  `reflection = cube * ReflectAmount * fresnel(FresnelBias, FresnelScale) * (ambient + diffuse) *
  gloss`; the result is `max(spec, reflection)`. `ReflectAmount` is 15 on 12,000 costume
  materials and 0.25 on 9,000 props. `DiffuseModifierInHDR` / `SpecularModifierInHDR` scale the
  terms.
* `BSkinColor` / `MSkinColor` (HumanSkin) are stored **blue first** (`BSkinColor="0.52,0.61,0.71"`
  is the warm `0.71,0.61,0.52`; the `skin_colors` table too); `HairColor` is plain RGB
  (colour-picker values: linearised with exponent 1.5).
* `Cut` sub-materials are a coarse closed hull in costume models: the engine does not draw the
  nude body triangles inside it (implemented in aa-anim, `aa_anim::cut`).

Common: `base_color` as above; `normal_map_texture` = `Normalmap` (or a `_ddn` `Bumpmap`), linear,
no Y flip; `perceptual_roughness` from `Shininess` (`(2/(n+2))^0.25`); `reflectance` =
`min(luma(Specular) * SpecularModifierInHDR / 2, 0.5)` (0.35 above a gain of 3); `MtlFlags & 2`
-> double sided; `Opacity < 1` -> blend; `AlphaTest > 0` -> mask, except when the diffuse map has
no alpha (mean alpha of the smallest mip >= 0.98: costume materials all carry `AlphaTest 0.5` on
BC1 maps); additive flag -> add; `TexMod` of the diffuse slot -> `uv_transform`.

| shader | mapping |
|---|---|
| `Illum`, `Cloth`, ... | lit opaque, dielectric (`Shading::Basic`); with `Shading::Full` and a `Specular` map, three images are baked from the map (`g = luma(map) * min(Specular * modifier, 1.2)`): metallic / roughness (`metal = smoothstep(0.2, 0.75, g) * f(ReflectAmount)`; matte roughness from `Shininess`, glossy dielectric texels down to `SHARP_ROUGHNESS` 0.42 = sharper leather highlights, metal 0.6 x matte) and Bevy's `specular_texture` (alpha = `sqrt(SPEC_STRENGTH * g)`, the highlight strength of the dielectric part, `reflectance` 2.0) and `specular_tint_texture` (the map's colour, `SPEC_TINT_AMOUNT` 0.3 of it, only for warm maps: gold, bronze, leather; the greenish maps of plate armour stay white because the plates are silver in the captures) |
| `Vegetation`, `VegetationLeaves`, `Grass` | double sided, alpha mask (`AlphaTest`, default 0.5) |
| `Hair` | double sided, alpha mask 0.5 on `alpha * 2` (the shader draws an alpha-tested pass and a blended `alpha * 2` pass), colour `HairColor^1.5 * 2 * Diffuse`, roughness from `PrimarySpecPow` (0.3-0.5), reflectance and tint from `PrimarySpecColor` (dark hair 0.13 has a dark highlight, blond 0.55), no emissive, **no anisotropy** (see below); the colour map is baked again without white in the cut texels (`bake_hair_colour`); with `Shading::Full` the highlight strength follows the strand pattern of the hair map, cut where the colour map is transparent (`specular_texture` baked from the `Specular` map, `bake_hair_specular`): streaks along the strands |
| `HumanSkin` / `Humanskin` | `luma(Diffuse) * chroma * luma(lit) * SKIN_GAIN` with `lit = BSkinColor^1.5 * 1.5` (the shader's lit colour) and `chroma` pulled `SKIN_PINK_WEIGHT` towards pink (the SSS map's colour); `diffuse_transmission` 0.1, roughness >= 0.5, reflectance 0.4. aa-anim replaces `BSkinColor` / `Diffuse` by the chosen `skin_colors` row (`shade::skin_base_color`) |
| `Eye` | roughness 0.12, reflectance 0.2 (more mirrors the sky over the iris: grey eyes; aa-anim composes the iris colour into the texture) |
| `Metal` | metallic 0.6 |
| `Glass`, `Liquid`, `Ice` | blend, alpha min(opacity, 0.6), roughness 0.1 |
| `Water*`, `Ocean*` | blend, dark blue-green, roughness 0.05 |
| particles, `FX_*`, `Light*`, beams, `Sky*`, clouds, `SpecialEffects`, `Hologram` | unlit, additive, double sided |
| `NoDraw`/`Nodraw` (or flag 0x400), `Cut` | not drawn |

Calibration constants (in `shade.rs` / `stage.rs`, with the reason next to them): `SHARP_ROUGHNESS`,
`SPEC_STRENGTH`, `SPEC_TINT_AMOUNT`, `HAIR_SPEC_REFLECTANCE` (specular maps), `SKIN_PINK`,
`SKIN_PINK_WEIGHT`, `SKIN_GAIN` (pale skins show pink in the captures whatever their
`BSkinColor`), `HAIR_GAMMA`, `stage::LOGIN_EXPOSURE` (3: the client's HDR exposure; the nominal
light values make costumes three times too dark) and `login_color_grading` (saturation 1.6,
contrast 1.2: the client's colour grading).

Effects on callers (aa-world, aa-client): `Emissive` is now tinted by the albedo and small (it
used to be an additive grey glow); `HumanSkin` is no longer a flat 0.22 grey but the skin colour;
hair is an alpha mask on the doubled alpha (not alpha-to-coverage, see below); `Illum` materials with
`AlphaTest` on opaque (BC1) maps are no longer alpha tested; `Specular > 1` no longer makes
metal. Costume `Diffuse` stays 0.2: show characters with `StageLighting` (engine units) or about
3x the nominal sun, not with a plain 10,000 lux light.

Hair and the prepasses (Bevy 0.20). The world renders with a depth prepass and, with SSAO, a
normal prepass without MSAA (`LOAD_PREPASS_NORMALS`). Two things break hair there, both tested
with `cargo run -p aa-anim --example lineup -- --only 3 --ssao 1` (also `--prepass 1`,
`--msaa off`) and the aa-client pond shot with `AA_TUNE=ssao=1`:

* **Anisotropy turns the hair white.** `pbr_fragment` sets `anisotropy_strength`, `anisotropy_T`
  and `anisotropy_B` only in the branch that computes the normal itself; with
  `LOAD_PREPASS_NORMALS` they stay zero while the lighting still takes the anisotropic path
  (`STANDARD_MATERIAL_ANISOTROPY`), whose GGX term (`D` with `ToH = BoH = 0`) explodes. It is not
  about mesh tangents and not fixable from the material, so the hair has no `anisotropy_strength`;
  `bake_hair_specular` gives the streaks instead.
* **Alpha-to-coverage turns the transparent fringe of the hair mesh black** with any depth
  prepass (the prepass treats it as opaque and writes the depth of the whole mesh, the main pass
  covers nothing there and the head behind fails the depth test), so hair is `Mask(0.5)` on the
  doubled alpha. The unit test `hair_is_safe_in_the_depth_and_normal_prepasses` keeps both.

Hair edges. The hair maps hold white (or any) colour in their fully transparent texels. With the
mask, bilinear and mip filtering pull it into the visible edge pixels: a white halo along the
hairline, and the strand-map highlight lit those texels too. `bake_hair_colour` spreads the colour
of the visible texels (alpha >= 64) into the cut ones (8 texels) and rebuilds the mips weighted by
alpha; the highlight map is multiplied by the colour map's coverage. A dark hair also needs a dark
highlight: before it was a fixed strength and black hair turned silver (`PrimarySpecColor`).

Not mapped (no `StandardMaterial` equivalent): the SSS map, `Environment` cube maps per
material (scene-level: use `stage::environment_light`), detail maps and blend layers (surveyed:
they occur on terrain and a few props only; no character material uses them), vertex bending,
Kajiya-Kay's strand direction (the streaks come from the strand map, not from a tangent).

## Tests

* `cargo test -p aa-cry`: unit tests plus real-pak checks (skipped without the client):
  skeletons, models, a `.mtl` sample, 400 DDS and 150 cgf + 50 chr robustness samples.
* `cargo test -p aa-cry --test pak_sample -- --ignored --nocapture`: 1,000 cgf + 300 chr and
  5,000 DDS with failure causes and timings.
* `cargo test -p aa-cry parses_every_mtl -- --ignored --nocapture`: all 38,294 `.mtl`.

Results (test profile, one thread, CPU side: parse + skeleton + `.mtl` + Bevy `Mesh` per subset;
DDS: parse + `Image` + CPU decode of the last mip):

| sample | result | cold file cache | warm |
|---|---|---|---|
| 1,000 cgf + 300 chr (2,261 subsets) | 1,300 ok, 0 failed, 0 panics | 1.50 s (1.15 ms/file, 151 MB/s) | 0.39 s (0.30 ms/file) |
| 5,000 DDS | 5,000 ok, 0 failed, 0 panics | 4.27 s (0.85 ms/file, 285 MB/s) | 0.67 s (0.13 ms/file) |
| all 38,294 `.mtl` | all parse | | |

In the viewer (debug build of the app), the first model costs ~400 ms including the pak index;
later models take 2-10 ms plus GPU upload. Texture references: 97.3% resolve.

## Viewer

```text
cargo run -p aa-cry --example viewer -- --model objects/<...>.cgf [--model ...] [--attach part.chr [--material x.mtl]]
    [--lod n] [--yaw deg] [--pitch deg] [--dist m] [--focus x,y,z] [--bones] [--shot shots/aa-cry/x.png]
cargo run -p aa-cry --example dds_dump -- <pak path.dds> <out.tga> [mip] [face]
cargo run -p aa-cry --example mtl_dump -- <out.tsv> [pak prefix]   # every sub-material as a TSV row
cargo run -p aa-cry --example cut_info -- <costume.chr> [x.mtl]    # the `Cut` hull of a costume model
```

`--attach` binds a part to the previous `--model`'s skeleton (a character = base `.chr` +
`--attach` face, hair and costume parts). `--focus` is in Bevy space (metres, Y up).
Screenshots: `shots/aa-cry/` (`inn.png`, `castle.png`, `tree.png`, `elf_login001.png`,
`rock_ati2.png`, `legacy_helmet.png`).

## Known gaps

* The SSS map, per-material environment maps, detail and blend layers are not mapped (see above);
  vegetation bending and wind are not applied. Eye iris colours are composed by aa-anim; a
  generic `IrisColor` bake is missing.
* Silver armour is still duller than the captures (metal `F0` is `0.216 * texture`; a boost would
  fight the dye textures).
* `Shading::Basic` cannot tell a wooden bow (`Specular 7.6` on a dark map) from a steel one: it
  stays a dielectric; use `Shading::Full` for characters, weapons and close-ups.
* Skin colour is one pink pull (`SKIN_PINK`) plus a per model multiplier fitted by aa-anim
  (`aa_anim::skin_tune`, see `docs/aa-anim.md`); the hue still differs per face texture (a per-texel
  bake of the SSS map would remove the remaining differences).
* Character assembly (the `Cut` hulls, CDF/DB customisation colours for `HumanSkin` / `Hair`)
  belongs to aa-anim; some costume "skin" subsets use UVs that do not match `el_f_nude_df`, and
  use the costume's own skin texture in the game.
* Legacy helmets have no `.mtl` and draw with the default material; the v orientation of
  legacy UVs is unverified for lack of a textured sample.
* Some buildings are open shells (e.g. the inner sides of `castle_nuia_001`'s towers): the
  winding matches the normals everywhere, so this is content and not a mirrored transform.
* Bevy culls by winding only, ignoring mirrored transforms. `load_model` flips the triangles of
  static nodes whose `scene::render_transform` has a negative determinant (1 node in 2,989
  sampled files, e.g. `beach_rock_med_e.cgf`; meshes are never shared between nodes).
  Placement code that mirrors whole instances (negative scale) must do the same, with
  `scene::subset_mesh_with(.., flip_winding: true)`.
