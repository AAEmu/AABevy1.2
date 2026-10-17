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
* **Worker pool** (`aa_cry::workers`). `Workers::global().spawn(priority, job)` runs `job` on a third of the hardware threads (2 to 5,
  `AA_WORKERS=N` overrides), lowest priority value first (use the distance to the camera); `pending()` counts queued and running jobs; a panicking job does not kill
  its worker. aa-world prepares object models on it and the in-game client prepares units and doodads (database rows, models,
  composed faces, clips); the main thread only calls `finish_model` (about 0.03 ms) and spawns entities.
* **Profiling** (`aa_cry::prof`). `prof::scope("name")` times a scope (wall time and, on Windows, the thread's CPU time from its
  cycle counter, which stays comparable on a busy machine); off (one atomic load) until `prof::enable(true)`, which
  `aa_world::profile::ProfilePlugin` does for `AA_PROFILE=1`. `prof::take()` returns the sums per name. The loader reports
  `cry.prepare_model`, `cry.parse_model`, `cry.material`, `cry.texture`, `cry.mesh`, `cry.finish_model`. `prof::sample_threads`
  is a sampling profiler of named threads (see `docs/aa-world.md`, Performance).
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

### Collision geometry (`physics.rs`)

`physics::collision_mesh(bytes, visible_fallback)` / `physics::load(path, ..)` return the collision triangles of a static `.cgf` in model space
(`CollisionMesh { positions, indices, source, materials, material_name }`): the slot 0 (default, colliding) geometry of every node's `MeshPhysicsData` chunk (0xCCCC0018, the
four physics chunk ids of a mesh chunk follow its 16 stream ids) transformed by the node chain, on `proxy` nodes (which `Model` does not read a mesh
for) and on render nodes alike. `parse_blob` reads the CryPhysics blob: boxes (`basis` rows = axes, centre, half sizes; 12 triangles), trimeshes
(`u32 nVerts, u32 nTris`, 16 bit indices; 341 models share the render mesh), spheres, cylinders and capsules (centre, axis, radius, half height;
approximated by prisms and a coarse sphere). 93 % of the 162k placed brushes have such geometry; models without any (ivy, flags, straw,
seedlings) have none in the real client either, `visible_fallback` returns their coarsest visible level of detail instead. All 3826 distinct
models of the main world's brushes parse. Used by `aa_world::collide`, which documents the format and the survey in `docs/aa-world.md`.

`materials` has one sub-material id per triangle: a primitive takes its default id from the geometry header (`+52`, after the version),
and a trimesh uses its stored `u8` ids when present. Geometry that shares the render triangles takes their subset ids instead. A truncated
material tail preserves the valid triangles and uses the default surface. Resolve `material_name` with `physics::material_paths(model, name)`
and read the sub-material's `SurfaceType` (`mat_wood`, `mat_rock`, ...); this lookup is also used by `scene::find_mtl` so drawing and collision
agree. It normalizes slashes/case before removing `.mtl`, handles root-level models, and falls back from `_lodN` to the base material.
For `.chr`, the model's own `.mtl` takes precedence over the stored name.

The installed pak's 6,441 environment models with collision all produce one material id per triangle and valid vertex indices, with no
model parse errors or collision/render material-name differences. 74 reference sub-material ids absent from their `.mtl` (including older
exports), and 32 have no material; callers should use the default surface for missing/out-of-range ids. Run the CPU-only audit with
`cargo run -p aa-cry --example surface_audit`. Real-pak tests verify wooden bridges/shops and rock walls/terrain pieces; synthetic tests
distinguish per-triangle ids and truncated tails.

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
BC1 maps); additive flag -> add, but only for translucent materials and effect textures (`p_effects`,
`sprites`, none): some 330 opaque `Illum` materials (costume skins, the Hariharan rail sets and
detect machines, Nuian tents) carry the flag and are drawn opaque by the real client; added they
were white ghosts (`additive_applies`); `TexMod` of the diffuse slot -> `uv_transform`.

| shader | mapping |
|---|---|
| `Illum`, `Cloth`, ... | lit opaque, dielectric (`Shading::Basic`; its `reflectance` is scaled by `ReflectAmount`, `reflect_scale`: 0.25 on props gives 0.43, costumes 10-15 keep 1: the Hariharan detect machine, Specular 3.4, mirrored the whole sky at a grazing view and was pale grey where the captures have dark rust); with `Shading::Full` and a `Specular` map, four images are baked from the map (`g = luma(map) * min(Specular * modifier, METAL_GAIN_MAX 5)`: saint armour is 7.4 x 4.9 on a map that never exceeds 0.19, it was a matte grey at the old limit of 1.5; the albedo of the metal texels is multiplied by up to `METAL_ALBEDO_GAIN` 8, metal reflects in the colour of its albedo and the diffuse maps are dark): metallic / roughness (`metal = smoothstep(0.2, 0.75, g) * f(ReflectAmount)`; matte roughness from `Shininess`, glossy dielectric texels down to `SHARP_ROUGHNESS` 0.42 = sharper leather highlights, metal 0.6 x matte) and Bevy's `specular_texture` (alpha = `sqrt(SPEC_STRENGTH * g)`, the highlight strength of the dielectric part, `reflectance` 2.0) and `specular_tint_texture` (the map's colour, `SPEC_TINT_AMOUNT` 0.3 of it, only for warm maps: gold, bronze, leather; the greenish maps of plate armour stay white because the plates are silver in the captures) |
| `Vegetation`, `VegetationLeaves`, `Grass` | double sided, alpha mask (`AlphaTest`, default 0.5); `GenMask` bit 0x2000 (`%BLEND_WITH_TERRAIN_COLOR`, set exactly on the 276 materials with a `blendWithTerrainAmount`, default 0.5) moves the colour map towards the ground hue at the same brightness (`bake_terrain_blend`, `TERRAIN_BLEND_HUE`): the marigolds of the login fields are yellow in the captures, orange in the map |
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

Not mapped (no `StandardMaterial` equivalent): the skin SSS map, `Environment` cube maps per
material (scene-level: use `stage::environment_light`), detail maps and blend layers (surveyed:
they occur on terrain and a few props only; no character material uses them), vertex bending,
Kajiya-Kay's strand direction (the streaks come from the strand map, not from a tangent).

### Static UCC cloth (`shade/ucc.rs`)

`Prep::convert` bakes the static `Cloth` UCC colour composition in both shading modes for a
UV square. Model preparation checks each UCC subset's referenced vertices after vertex hooks;
missing/nonfinite/out-of-range UVs retain the original material. The material cache key includes
this eligibility, so an unsafe subset cannot reuse a composed material loaded for a safe one.
Direct `CryAssets::material` / `Prep::convert` callers supply UVs in 0..1.
The original pak sources define the mapping:

* `shaders/cloth.ext`: `%X2_UCC_MASK=0x100000`, `%X2_UCC_BACKGROUND_MASK=0x200000`,
  `%X2_IGNORE_UCC_BACKGROUND_MASK=0x400000`; the galleon's remaining `0x40000000` is wind.
* `shaders/hwscripts/cryfx/cloth.cfx:384–467`: default mask colours are the RGB basis, scale is
  one, offsets zero. `532–539` transforms mesh UV by the SubSurface TexMod, then computes crest
  UV `tc * uccScale - uccOffset` and background UV `(tc-uccAddOffset)/(1-uccAddOffset)`.
* `cloth.cfx:735–753` remaps background channels to `saturate(r*red+g*green+b*blue)`, only when
  both UCC/background flags are set and ignore-background is clear. It alpha blends the
  SubSurface crest only at strict `0 < crestUV.x,y < 1`. Original Diffuse alpha survives both.
* `cloth.cfx:90` and `FXSamplerDefs.cfi:4,14`: Custom and SubSurface have **no sRGB lookup**;
  ordinary Diffuse does. The baked image is linear `Rgba8Unorm`, with linear colour mips and
  each original Diffuse alpha mip. Base colour, normals, culling and opacity remain material data.

The dock's `objects/env/06_unit/01_ship/galleon/galleon_mahadevi.mtl`, submaterial 3 (`ucc`),
uses `ucc_pattern04.dds`, red `(0.21568629,0.21568629,0.21568629)`, green/blue white, and
SubSurface `ucc_logo32.dds` at scale `(3,3)`, offset `(1,1)`. This turns the raw rainbow into
neutral patterned cloth with the authored crest in the middle third. All UCC triangles of
`galleon_deco_sail01.cgf`, including its LODs, have UVs in 0..1 (verified from the pak).

The native `$CustomMap` is a runtime binding whose default was **not recovered**. This static
path uses an explicit material Custom texture when present, otherwise the authored Diffuse
pattern; that fallback is supported here, not claimed as a recovered native default. Dynamic
player crest download/binding is not implemented. The bake covers one UV square; repeated UVs
outside it, animated TexMods, rotated SubSurface TexMods and nonidentity Diffuse TexMods need
a shader path. Unhandled TexMod attributes are retained as `TexMod::unhandled` and reject the
static bake, as do missing Diffuse/SubSurface inputs. Unsupported static transforms keep the existing material conversion. Cloth
lighting, fuzz and wind remain separate from this colour composition.

Native probe: `D:/aa-re/client12/x2game_unpacked.dll`, `sub_390E3ED0`, changes the
`%X2_UCC_BACKGROUND_MASK` flag depending on whether a background texture was supplied
(`0x390E4074–0x390E41A0`), writes that texture to copied resource-array entry 10
(`0x390E4085–0x390E40C6`; array starts at frame `-0x424`, stride `0x3c`, entry at `-0x1cc`),
and supplies the crest to texture slot 9 (`0x390E4391–0x390E43B3`, non-HumanSkin case).
These are disassembly observations of dynamic UCC replacement, not evidence of the renderer's
absent-Custom default. The bounded `D:/aa-re` inventory contains only the dedicated-server
`cry3dengine` binary and no client renderer, so that default remains unresolved.

`cargo run -p aa-cry --example ucc_audit -- data/extract/aa-cry/ucc/audit` writes material and
source-linked geometry TSVs. The installed pak has 38,294 MTL files / 67,885 material slots,
622 Cloth materials, and 218 UCC materials (184 character cloaks, 34 environment):

| GenMask | UCC materials |
|---|---:|
| `100000` | 184 |
| `300000` | 5 |
| `700000` | 9 |
| `40100000` | 3 |
| `40104000` | 1 |
| `40300000` | 12 |
| `40340000` | 4 |

Twenty-one enable background remapping; nine explicitly ignore it. Seventeen provide Custom
(eleven with active background remapping); the authored Custom is used, with separate Diffuse
opacity. Nine omit Diffuse and remain unsupported; all 218 authored TexMods are empty.
The audit checks 1,340 same-directory model candidates plus explicit cloak CDF bindings and the
dock override: 479 source-linked bindings / 302 models, with no parse/load failures. It links
198 UCC materials to 247 UV-square subsets and 16 out-of-range subsets. The latter have small
authored overshoots: flag minimum `(-0.0020723,-0.0006968)` and siege meshes minimum U
`-0.0000079`; the strict gate conservatively bypasses them. Twenty materials have no proven
model binding in this bounded audit; this does not establish their absence from world overrides.

`cargo run -p aa-cry --example ucc_cloth -- --shot shots/aa-cry/ucc_cloth.png --shot-after 8`
shows the original sail mesh with raw Diffuse/control and the composition side by side, plus the
source mask, composed texture and crest. It uses white, unlit factors to isolate this operation.
The installed `reference/ingame/spawn/real.png` contains no dock sail, so there is no matching
real-client sail capture for a pixel comparison; `shots/aa-world/lights/on.png` is the previous
Bevy dock control showing the raw rainbow. Focused tests cover the flags/defaults, linear
palette, strict crest clipping/alpha, authored UV controls, alpha mips, cache/conversion and the
real galleon recipe/UV domain.

Add `--assembled --shot shots/aa-cry/ucc_assembled.png` for the original fourteen-part dock
galleon, including hull, sails, masts, six cannons, helm/rudder and anchor hinge. The diagnostic
reads the original cell `(18,7)` brush/model/material tables through aa-pak and preserves each
part's relative matrix, centered and rotated together for comparison. The inspected image shows
both authored sails corrected on the complete brush assembly; aa-dev reports 279.8–353.9 FPS
(2.85–3.59 ms) at 1920x1080 while the independent CPU audit was also running.

Validation (2026-10-17): `cargo test -p aa-cry --lib` passes 61 tests, with one exhaustive
material audit intentionally ignored; ten of the passing tests cover UCC/domain handling. The fixture build
passes, and the inspected 1920x1080 capture shows neutral braided cloth and the centered sun
crest alongside the raw RGB control. `aa-dev` reports 356.7–379.4 FPS (2.66–2.82 ms) in this
unlit diagnostic scene. The window exits after capture. No aa-client integration is required:
world material preparation already uses this conversion path.

## Character back lights (`shade::backlight`)

The time of day has a `Char BackLight` group: `InShadowIntensity` (0.5) and, per light `i` = 1, 2, `"i Color"`,
`"i Diffuse Multiplier"`, `"i HDR dynamic"` and `"i Specular Multiplier"` (zone 186: light 1 = 0.486 x 5, light 2 = 1.0 x 0.7,
constant over the day). What the client does with them, from `cry3dengine.dll`, `cryanimation.dll`, the renderer's parameter
table and the shader scripts in the pak:

* **A shader term, not scene lights.** `illum`, `humanskin`, `hair`, `cloth` and `metal` have, under
  `%_RT_OBJECT_BACKLIGHT` (a per-object flag, set for actor models and switched by the cvar `e_character_back_light`), a loop over
  two `fragLightPass` directional lights: `cDiffuse = BackLightColor[i].xyz`, `cSpecular = BackLightColor[i].xyz *
  BackLightColor[i].w`, `vLight = BackLightDir[i].xyz`, `fNdotL = dot(BackLightDir[i].xyz, N)`, `fFallOff = BackLightDir[i].w`,
  no shadow, no filter. `X2BackLight` (`shadelib.cfi`: `saturate(N.L)` diffuse and `Phong(R, L, specPow)`) adds the result to
  `cDiffuseAcc` / `cSpecularAcc`, i.e. to the light that is then multiplied by the diffuse map and `MatDifColor`
  (`hair` uses its own per-light function, `humanskin` the same `X2BackLight` as `illum`, so skin gets plain
  `colour * N.L`, not the skin shader's `pow(skin, 1.5) * 1.5` colour). The two per-instance parameters are
  `PI_BackLightDir` and `PI_BackLightColor` (renderer parameters 0x86 / 0x87, read from the render object's extra data).
* **Colour.** `TimeOfDayManager::UpdateEngineBackLightParameters` (`sub_300A5630`) sends, for index 0 and 1, renderer parameter 71 =
  `Color.rgb * (powf(hdr, "i HDR dynamic") * "i Diffuse Multiplier")` and 72 = `"i Specular Multiplier"` (the power is 1 where
  `HDR dynamic` is 0, as in the main world). The colour is gamma encoded like every time of day colour: `0.486` decodes to 0.2 and
  `0.2 x 5` is exactly 1.0, which is how light 1 reads as "1.0".
* **Direction: a property of the character, not of the camera.** Every player `.cdf` (`nu_m.cdf`, `el_f.cdf`, ...) has
  `<BackLight><Light Index="1" Position="0.45,-1.1,0.25"/><Light Index="2" Position="-0.35,0.35,0.1"/></BackLight>` in the
  character's own space (CryEngine: +Y forward, +X to its right, Z up), read by `cryanimation.dll`'s `CharacterDefinition`
  loader and saved again by its writer; only the elf female (`0.55,-1.1,0.25` / `-0.35,0.5,-0.2`) and the ferre female
  (`0.65,-1.1,0.25`) differ from the nuian and male values. Taken as directions: light 1 shines from behind, a little from the
  right and above (it lights the back of a character seen by the default third person camera, and the rim of one seen from the
  front); light 2, 0.7 times as strong, from the front left. They turn with the character. (The step from `Position` to
  `BackLightDir.xyz` is in the render object setup, not found in the dumps; this reading, `normalize(Position)` rotated into the
  world by the character, reproduces the Royal Palace capture, whose camera is behind the player: the back is evenly lit, and the
  right hand brighter than the left. The real captures have the camera behind the player only, so the front lit by light 2 is
  not checked.)
* The zones of the login stage define only light 1 (`login2/zone/177`: white x 1.0, `zone/224`: a coloured, animated x 0 .. 3,
  `login2` itself: animated x 5 with `HDR dynamic` 1.13); light 2 is missing from those files (colour 0).

In Bevy a light cannot be restricted to some meshes (`RenderLayers` only select the views), so the term is a material extension:

* `shade::backlight::CharMaterial` = `ExtendedMaterial<StandardMaterial, CharBackExt>`, shader `backlight.wesl`: the
  `StandardMaterial` lighting, then for each light `diffuse_colour / pi * lux * saturate(N.L) + F0 * Blinn-Phong(roughness) * lux *
  specular multiplier`, times the view exposure, added before fog / tone mapping. The direction is the `.cdf` position
  (`CDF_POSITIONS`) turned by the mesh's `world_from_local` (the root of a character is its orientation; rigid attachments such as
  swords use their own, which is only approximately the character's).
* `shade::backlight::CharBackLight` (resource, `dirs` + `lights` = rgb lux, specular multiplier) is what the world feeds;
  `CharLightPlugin` registers the material and two `PostUpdate` systems. **Nothing changes unless the plugin is added and a light has
  a colour**: the material of an entity is swapped (one `CharMaterial` per `StandardMaterial`, shared) only while a light is on, and
  the lights are uploaded to the `CharMaterial`s when they moved by more than 0.4 %, at most every 0.5 s and 64 materials per frame
  (every changed material asset is re-extracted and re-prepared by the render world, about 10 us each: the ~1000 materials of a town
  in one frame were a 10 ms hitch).
* **The extension must be bindless.** An `ExtendedMaterial` is only bindless if both halves are, and `StandardMaterial` is. With a
  plain `#[uniform]` extension every `CharMaterial` got its own bind group and uniform buffer, the draws stopped batching (one batch
  set per material) and the 90-character town cost 100 ms per frame instead of 14 (render thread: `write-buffers` 27 ms,
  `bind-groups` 26 ms, GPU `uniform_allocation` 9 ms and `bin_unpacking` 6 ms instead of about 1 ms each). `CharBackExt` is
  `#[data(50, BackData, binding_array(101))]` + `#[bindless(index_table(range(50..51), binding(100)))]` (the standard material owns
  the indices 0..31 and the binding 0 / 10); the shader reads `back_array[back_indices[slot].data]` with
  `slot = mesh[instance_index].material_and_lightmap_bind_group_slot & 0xffffu`, or a uniform at binding 50 without bindless
  (`@if(BINDLESS)`). A test (`the_character_material_is_bindless_like_the_standard_material`) keeps it so.
* The direction uses `get_world_from_local(instance_index)`: for a skinned mesh that is the mesh entity's transform (the skin moves the
  vertices with the joint matrices, not this), i.e. the character root's facing; palace renders from the front, the side and the
  back keep light 1 behind the character.
* Which materials: `convert_material` marks every `Shading::Full` material by `StandardMaterial::lightmap_exposure` (a field that is
  only read with a lightmap, which nothing here has): `backlight::TAG`, or `2 + Diffuse luma` for `humanskin`, whose
  base colour carries the lit skin colour that the engine puts into the sun and ambient light instead (the swap then multiplies
  the base colour by `Diffuse / base colour`, i.e. the term lights `texture * Diffuse`). `Shading::Basic` (world props) is never swapped.
* `aa_world::char_light` (added by `sky::build`) turns `EnvSample::char_back` (`"i Color"` x `"i Diffuse Multiplier"`, linear, and the
  specular multiplier; the sample follows the time of day and the zone) into lux with **`pi * ambient_scale`** per engine unit
  (`AA_CHAR_BACKLIGHT=<gain>` scales it, 0 turns it off). The term joins the ambient light in the engine's light sum, whose
  units `ambient_scale` converts (a radiance `L`; an illuminance `pi L` gives a diffuse surface the same radiance); the sun's
  `sun_lux_scale` (1000, fitted to sunlit ground) gave back lights 5.4 times too dark.
* The login stages are **unchanged**: they do not add the plugin, and their rig (`StageLighting::login`, a fit to the captures with
  its own "rim from behind", about half of whose chest brightness comes from it) stands in for the engine's lights.
  `cargo run -p aa-anim --example compare -- --target select --backlight K` adds light 1 = K engine units to a stage render (from the
  front it changes nothing, light 1 being behind the character).

Numbers (mean sRGB luma, real / ours before / ours with the term, `--hour 9.6`, the player in the shade of the Royal Palace
court yard seen from behind, `reference/ingame/royal_palace/real.png`; the player is Aanetwalker, the capture Refone, so the dye of the
jacket and the hair differ, the white fur does not): collar 114 / 51 / 122, hem 47 / 13 / 61, jacket 31 / 7 / 36, trousers 13 / 5 / 16,
hands 56 and 84 / 47 and 52 / 94 and 103 (before the skin albedo fix 117 and 114). At the spawn (`--hour 10`, sunlit, the same
equipment difference): collar 135 / 101 / 129, jacket 36 / 30 / 47, hem 38 / 26 / 43, hands 68 and 90 / 39 and 65 / 61 and 83. The
character select render is pixel identical to before (0 of 2,073,600 pixels differ).

Cost (release, `--enter-world --account perfbench`, `tools/perf.sh` alternating, medians of the quiet rounds; other sessions make the
rest too noisy): at the Nuian spawn 12.4 / 12.6 / 12.8 / 13.0 ms with the term against 12.2 / 12.4 / 12.4 / 12.5 ms with
`AA_CHAR_BACKLIGHT=0` (about +0.3 ms, the extra lighting in the pixel shader of the characters; it was +2.7 ms with the non-bindless
extension), the town crowd (`game-crowd`, ~90 animated characters) 11.6 against 11.7 ms in a profiled run.

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

* The skin SSS map, per-material environment maps, detail and blend layers are not mapped (see above);
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
