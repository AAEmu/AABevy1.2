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
* **Shading level.** `ModelOptions::shading`: `Shading::Basic` (default) uses StandardMaterial
  lighting, plus the native post-lighting RGB-squared term on Illum `VERTCOLORS` brushes.
  `Shading::Full` uses `shade::illum::IllumMaterial` for opaque/masked Illum: native Phong,
  gloss channels and light-modulated material reflections. Skin and hair keep their dedicated
  materials. The previous metallic/roughness bake remains an explicit comparison fallback.
* **Stage lighting** (`aa_cry::stage`): `StageLighting::login().spawn(&mut commands, &mut cry,
  camera, frame)` spawns the three directional lights (key with shadows, fill, rim), the
  environment map (`GeneratedEnvironmentMapLight` from the `beach_blur2.dds` cube the costumes
  name) and the ambient light on the camera; `aa_cry::stage::login_color_grading()` is the
  matching `ColorGrading` component. Lights are in engine light units (`LIGHT_UNIT_LUX`,
  `LOGIN_EXPOSURE`); `environment_light(cry, cubemap, intensity)` and
  `material_environment(&Material)` are the generic helpers. Directions are in the figure's frame:
  its front is **-Z**, +Z points away from the camera, +X is the viewer's left (the callers pass
  `from_rotation_arc(NEG_Z, to_camera)`). The first calibration read +Z as "to the camera", which put the
  key light behind the figure and the rim in front; fixed. The key stands at `(-0.621, 0.616, -0.485)` since the stage rig
  refit (below).
  `StageShadows` sets the key's cascades (2, 1 - 30 m, first bound 10 m instead of Bevy's 4 out to 150 m).
* **The stage rig refit** (`StageLighting::login`). The step 1 / 2 / 3 and character select captures show the
  key light on the viewer's right (centroid azimuth 38 degrees, 20 - 35 up; the first rig had it 35 degrees to the
  left, 37 up, which lit every face frontally and from the wrong side). Fitted with the new `SkinMaterial` /
  `HairMaterial` / eye shaders: a non-negative least squares fit of key-only basis renders (a 12 x 5 grid of azimuth /
  elevation, `compare --key-from x,y,z --fill 0 --rim 0 --env 0 --amb 0 --tonemap none`) plus environment and
  ambient renders to the skin pixels of the captures (elf female / male, nuian male, linear display assumed), then
  trials of the face asymmetry (right minus left luminance over the sum), the coefficient of variation and the
  10 / 90 percentile ratio of the blurred skin luminance against the eight `step2_*` captures. Result: the key
  at azimuth 52 degrees to the viewer's right, 38 degrees up (`from` `(-0.621, 0.616, -0.485)`, the figure's frame),
  intensity, fill, rim, environment and ambient unchanged. The asymmetry goes from -0.13 to +0.07 (captures +0.16),
  and the brow ridge, the nose and the lips now throw shadows (the old key lit the eye sockets). The up-facing
  irradiance is nearly unchanged (`key.y x intensity` 1.29 against 1.27) so the step 3 field and the costumes'
  mean brightness do not move. What the rig does *not* change: the face contrast (cv 0.42 against the captures'
  0.62). `skin.wesl` makes the key's N.L, the shadow and the constant shadow fill (`0.3 x dark / middle colour`)
  the only terms of the face, so key / fill / rim / environment / ambient intensities were varied over a factor
  of 5 each with no change of the face's contrast (trials `mH` .. `mN`); a deeper face needs a lower fill floor or
  a steeper wrap in the skin shader. `compare --key-from / --fill-from / --rim-from x,y,z` (stage frame) set the
  directions for such trials.
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

`material::Texture::attributes: Vec<(String, String)>` retains texture-level attributes
other than `Map`, `File` and `Amount`, including `IsTileU/V`, `Filter`, `TexType` and
unknown names. Original attribute spelling and XML-decoded string values are kept;
absent, empty and malformed values remain distinct. `Clone` retains this data.
Public `Texture` literals must supply `attributes: Vec::new()` when no raw authoring
exists; Wavefront/OBJ fallback textures use an empty vector. This is data preservation
only: rendering, sampler defaults/descriptors, texture caches and pass routing still
use their existing behavior. Native sampler and renderer integration remain pending.

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
| `Illum`, `Cloth`, ... | Basic: lit dielectric with uniform factors; Illum VERTCOLORS adds post-lighting interpolated RGB squared. Full opaque/masked Illum: native Phong, gloss masks and light-modulated material reflection (see Native Illum below). Cloth and unsupported/blended Illum retain StandardMaterial; `AA_ILLUM_LEGACY=1` selects the previous Full gloss-to-metallic/roughness bake. |
| `Vegetation`, `VegetationLeaves`, `Grass` | double sided, alpha mask (`AlphaTest`, default 0.5); `GenMask` bit 0x2000 (`%BLEND_WITH_TERRAIN_COLOR`, set exactly on the 276 materials with a `blendWithTerrainAmount`, default 0.5) moves the colour map towards the ground hue at the same brightness (`bake_terrain_blend`, `TERRAIN_BLEND_HUE`): the marigolds of the login fields are yellow in the captures, orange in the map |
| `Hair` | double sided, alpha mask 0.5 on `alpha * 2` (the shader draws an alpha-tested pass and a blended `alpha * 2` pass), colour `hair_colour(HairColor) * 2 * Diffuse` (hue as written, brightness `^HAIR_GAMMA` 1.5), no emissive, **no anisotropy** (see below); the colour map is baked again without white in the cut texels (`bake_hair_colour`). This `StandardMaterial` keeps only a faint broad sheen (roughness 0.65); with `Shading::Full` the material is tagged and swapped for the hair shader, see "Hair" below |
| `HumanSkin` / `Humanskin` | `luma(Diffuse) * chroma * luma(lit) * SKIN_GAIN` with `lit = BSkinColor^1.5 * 1.5` (the shader's lit colour) and `chroma` pulled `SKIN_PINK_WEIGHT` towards pink (the SSS map's colour); `diffuse_transmission` 0.1, roughness >= 0.5, reflectance 0.4. aa-anim replaces `BSkinColor` / `Diffuse` by the chosen `skin_colors` row (`shade::skin_base_color`). With `Shading::Full` the `StandardMaterial` is only the base of `skin::SkinMaterial` (below): no GGX, no environment specular |
| `Eye` | roughness 0.45, reflectance 0.08 (`eye.cfx` has no GGX or Fresnel: a small Phong glint inside the iris, an environment term of about 0.04; 0.12 / 0.2 put a wet sheen over the whole eyeball); static `IrisColor` multiplies Diffuse under the sRGB-decoded Specular red mask when both IRIS/GLOSS_MAP flags are enabled (see below); aa-anim can replace the final face texture |
| `Metal` | metallic 0.6 |
| `Glass`, `Liquid`, `Ice` | blend, alpha min(opacity, 0.6), roughness 0.1 |
| `Water*`, `Ocean*` | blend, dark blue-green, roughness 0.05 |
| particles, `FX_*`, `Light*`, beams, `Sky*`, clouds, `SpecialEffects`, `Hologram` | unlit, additive, double sided |
| `NoDraw`/`Nodraw` (or flag 0x400), `Cut` | not drawn |

Calibration constants (in `shade.rs` / `stage.rs`, with the reason next to them): `SHARP_ROUGHNESS`,
`SPEC_STRENGTH`, `SPEC_TINT_AMOUNT` (specular maps), `SKIN_PINK`,
`SKIN_PINK_WEIGHT`, `SKIN_GAIN` (pale skins show pink in the captures whatever their
`BSkinColor`), `HAIR_GAMMA`, `stage::LOGIN_EXPOSURE` (3: the client's HDR exposure; the nominal
light values make costumes three times too dark) and `login_color_grading` (saturation 1.6,
contrast 1.2: the client's colour grading).

Effects on callers (aa-world, aa-client): `Emissive` is now tinted by the albedo and small (it
used to be an additive grey glow); `HumanSkin` is no longer a flat 0.22 grey but the skin colour;
hair is an alpha mask on the doubled alpha (not alpha-to-coverage, see below); materials with
`AlphaTest` on proven fully opaque maps can skip the redundant test; `Specular > 1` no longer makes
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
  the strand highlights come from `hair.wesl` instead, which builds its frame from the vertex tangent.
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

### Authored alpha cutouts

An opaque smallest mip does **not** prove a texture is opaque. The previous conversion removed
`AlphaMode::Mask` when the smallest authored mip's mean alpha was at least 0.98. Real BC1
textures can lose all their cutouts in those tiny mips: BC1 has one-bit transparency and is
not intrinsically opaque. In the native sources, `humanskin.cfx:509–521` multiplies opacity by
Diffuse alpha, `commonZPass.cfi:470–475` tests the sampled Diffuse alpha against the per-material
reference, and `fragLib.cfi:292–295` clips against it when no Z pass supplies that coverage.
`commonShadowGenPass.cfi:193–203` also clips the sampled alpha in shadows.

`Dds::alpha_is_opaque` now scans alpha across **every authored mip**, directly in compressed
blocks or explicit alpha bytes without decoding colour or allocating an image. It distinguishes
BC1 three/four-colour selectors, BC2 nibbles and both BC3 alpha palettes; formats with implicit
alpha one are opaque. Transparent padding in partial blocks conservatively declines the proof.
`Dds::mips_truncated` records dropped/invalid requested mips, which cannot prove opacity; missing,
unreadable, cube and volume inputs cannot prove it either. `CryLoader::alpha_is_opaque` caches
the result by normalized texture path, independently of colour space.

Only an existing Mask with a finite threshold that the base alpha can reach may be elided, and
only after that full-opacity proof. Otherwise the original threshold is retained. Blend/no-draw
decisions, vegetation exclusions, texture data and existing hair approximations are preserved.
This restores native cutout coverage; it adds no guessed colour, lighting or alpha threshold.

`cargo run -p aa-cry --example alpha_audit -- data/extract/aa-cry/mtl_all.tsv data/extract/aa-cry/alpha/audit.tsv`
checks the inventory produced by `mtl_dump`, using the same conversion's Mask/no-draw/blend
decisions and effective texture threshold. The installed audit successfully checks 27,853 masked
material slots, considering 4,449 unique texture paths. The old mean shortcut elided 12,630 slots; only 10,945 prove
full opacity. Of the old elisions, 1,605 have authored texels below their actual threshold
(1,241 character slots): 1,276 Illum, 196 Cloth, 113 HumanSkin, 10 Metal, nine Frozenlayerwip and
one Hair. Another 80 old elisions are conservatively retained by the stricter full-opacity proof.
These are material/texture counts; the audit does not establish use of every cutout by every mesh.

The original `cat_baby_black_pet.cdf` explicitly pairs `cat_baby_pet.chr` with
`cat_baby_black_pet.mtl`: HumanSkin subset 0, AlphaTest 0.5, BC1 Diffuse 256x256 with seven mips.
Its smallest mip is alpha 255 everywhere, while 2,192 base texels are below the threshold;
authored mip minima are `[0,0,0,0,0,0,255]`.
`cargo run -p aa-cry --example alpha_cutout -- --shot shots/aa-cry/alpha_cutout.png --shot-after 8`
reads that CDF binding and compares the complete original mesh with the old opaque elision and
the retained authored mask. An orthographic camera gives both meshes the same viewing angle;
white, unlit factors isolate coverage. The inspected 1920x1080 capture removes
the black rectangular whisker cards and restores the ear cutouts. There is no matching real-client
cat capture under `reference/`, so this demonstrates the source-backed coverage correction rather
than a real-client pixel comparison. `aa-dev` reports 336.0–398.4 FPS (2.59–3.01 ms); the fixture
window exits after capture.

Validation (2026-10-17): 71 library tests pass, one exhaustive audit is ignored. Five focused tests
cover compressed alpha modes, every authored mip, padding/truncation, explicit/implicit alpha,
real cat conversion, normalized caching, blend/no-draw exclusions and an unreachable alpha
reference. Genuine opaque inputs still skip their redundant mask.

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
uses Diffuse `ucc_pattern04.dds`, red `(0.21568629,0.21568629,0.21568629)`, green/blue white, and
SubSurface `ucc_logo32.dds` at scale `(3,3)`, offset `(1,1)`. It has no Custom texture, so the native
white default supplies the background mask: the palette sum saturates to plain white cloth, with
the authored crest in the middle third. Diffuse still supplies opacity. All UCC triangles of
`galleon_deco_sail01.cgf`, including its LODs, have UVs in 0..1 (verified from the pak).

The native `$CustomMap` uses an explicit material Custom texture when present, otherwise
`textures/defaults/white.dds`, **not the Diffuse texture**. The original default asset is 4x4,
with every RGBA channel 255, verified through aa-pak and in the real-galleon regression.
Dynamic player crest download/binding is not implemented. The bake covers one UV square; repeated UVs
outside it, animated TexMods, rotated SubSurface TexMods and nonidentity Diffuse TexMods need
a shader path. Unhandled TexMod attributes are retained as `TexMod::unhandled` and reject the
static bake, as do missing Diffuse/SubSurface inputs. Unsupported static transforms keep the existing material conversion. Cloth
lighting, fuzz and wind remain separate from this colour composition.

Native probe: `D:/aa-re/client12/x2game_unpacked.dll`, `sub_390E3ED0`, changes the
`%X2_UCC_BACKGROUND_MASK` flag depending on whether a background texture was supplied
(`0x390E4074–0x390E41A0`), writes that texture to copied resource-array entry 10
(`0x390E4085–0x390E40C6`; array starts at frame `-0x424`, stride `0x3c`, entry at `-0x1cc`),
and supplies the crest to texture slot 9 (`0x390E4391–0x390E43B3`, non-HumanSkin case).
These are disassembly observations of dynamic UCC replacement; that supplied texture overrides
the static absent-Custom binding.

Renderer proof: the existing `data/extract/aa-cry/ida/render/cryrenderd3d10.dll` is PE x86,
image base `0x38000000`, file/product version `0.5.1.101406`, timestamp
`2014-10-14 05:46:04 UTC` (the related cry3dengine/client12 x2game builds are 05:45:00/05:44:21).
SHA256 `ebf051bba5d349a6b3bf6d4eb676908115b73bb22d35d5a0cc917c325426589d` and MD5
`f5f22e8b45bcbc7652fa424047a56a5c` match its existing IDA database input hashes. The renderer is
also the 1.2 source used for the post defaults in `docs/aa-world.md`. Only disposable copies
were opened for this bounded probe, with automatic analysis disabled.

* `0x38257BE8–0x38257C03`: `$CustomMap` resolves to the dummy array plus `0x9b0`, index 10
  at stride 248. `mfCreateShaderResources` (`0x38259E70`, case 10) loads the explicit Custom
  filename; the resource constructor (`0x38259400`) leaves missing entries null.
* `CHWShader_D3D::mfSetSamplers` (`0x38078EB0`), `0x38079025–0x3807902F`, checks
  `resources[slot+16]`. When absent, `0x3807907B–0x380790B4` selects White global
  `0x383DDB68` for slot 10 (only slots 0, 7, 1 and 5 have different fallbacks).
* `0x381C68FE–0x381C6945` initializes that global from `Textures/Defaults/White.dds`.

The disassembly and bounded decompilations are retained under the ignored
`data/extract/aa-cry/ucc/renderer_probe/` directory. This proves the renderer's absent-slot
default; it does not assert what a live gameplay customization override will supply.

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
opacity. The other ten active-background slots use the native white default: six galleon
materials, three siege-weapon materials and the prologue ship. Nine omit Diffuse and remain
unsupported; all 218 authored TexMods are empty.
The audit checks 1,340 same-directory model candidates plus explicit cloak CDF bindings and the
dock override: 479 source-linked bindings / 302 models, with no parse/load failures. It links
198 UCC materials to 247 UV-square subsets and 16 out-of-range subsets. The latter have small
authored overshoots: flag minimum `(-0.0020723,-0.0006968)` and siege meshes minimum U
`-0.0000079`; the strict gate conservatively bypasses them. Twenty materials have no proven
model binding in this bounded audit; this does not establish their absence from world overrides.

`cargo run -p aa-cry --example ucc_cloth -- --shot shots/aa-cry/ucc_cloth.png --shot-after 8`
shows the original sail mesh with raw Diffuse/control and the composition side by side, plus the
raw Diffuse, native default mask, composed texture and crest. It uses white, unlit factors to isolate this operation.
The installed `reference/ingame/spawn/real.png` contains no dock sail, so there is no matching
real-client sail capture for a pixel comparison; `shots/aa-world/lights/on.png` is the previous
Bevy dock control showing the raw rainbow. Focused tests cover the flags/defaults, linear
palette, strict crest clipping/alpha, authored UV controls, alpha mips, cache/conversion and the
real galleon recipe/UV domain.

Add `--assembled --shot shots/aa-cry/ucc_assembled_native.png` for the original fourteen-part dock
galleon, including hull, sails, masts, six cannons, helm/rudder and anchor hinge. The diagnostic
reads the original cell `(18,7)` brush/model/material tables through aa-pak and preserves each
part's relative matrix, centered and rotated together for comparison. The inspected image shows
both sails with the native white background and authored sun on the complete brush assembly.
The 1920x1080 capture records 147.8–305.0 FPS (3.29–6.89 ms), with the window exiting after capture.

Validation (2026-10-17): `cargo test -p aa-cry --lib` passes 72 tests, with one exhaustive audit
ignored; all nine focused UCC tests pass, including the actual default DDS,
white output outside the crest, explicit Custom/default cache separation, every Diffuse alpha
mip and the original galleon recipe. Domain/cache tests also preserve unsupported subset bypass.
No aa-client integration is required: world material preparation already uses this conversion path.

### Static Eye iris (`shade/eye.rs`)

The original `shaders/eye.ext` defines `%IRIS=0x1` and `%GLOSS_MAP=0x4`.
`shaders/hwscripts/cryfx/eye.cfx:134–140` applies, above `QUALITY_LOW`,
`Diffuse.rgb *= lerp(1, IrisColor, gloss.r)`. `IrisColor` defaults to white
(`eye.cfx:86–93`). Both texture reads use `baseTC.xy`; shared `FXSamplerDefs.cfi:4,7`
sets `sRGBLookup=true` for Diffuse and Gloss. The mask red channel is decoded before
bilinear filtering. Other mask channels and alpha do not affect this albedo operation.
The installed material inventory has 117 Eye slots: 83 enable both flags, and 76 of those
provide both authored textures. Missing inputs and identity palettes keep the existing conversion.

`Prep::convert` performs this static operation in both shading modes, with a cache key containing
both input paths and the exact palette. The composed image is `Rgba8UnormSrgb`; each original
Diffuse mip keeps its alpha. Base colour, opacity, culling and lighting parameters remain as
before. Any nonidentity or unhandled TexMod on either input declines the bake, including animated
or generated mapping. Ordinary repeated UVs are supported because both inputs share periodic UVs.
Only uniform power-of-two texture size ratios are supported; nonuniform domains are declined.

Real Ferre and Elf materials have 512-square Diffuse with 256-square Gloss. For each Diffuse mip,
the bake chooses Gloss mip `clamp(diffuseMip + log2(glossWidth / diffuseWidth))` and resamples
at Diffuse texel centres with linear bilinear repeat sampling. The ratio supplies an integer LOD
bias. This is a finite precomposed texture approximation: runtime screen derivatives, fractional
LOD/trilinear filtering, and filtering a product of two independent samples cannot be exactly
represented by these baked mips. Eye reflection/cornea and other native lighting terms are
separate gaps; this does not claim full Eye shader parity.

`cargo run -p aa-cry --example eye_iris -- --shot shots/aa-cry/eye_iris.png --shot-after 8`
uses the actual `objects/characters/ferre/female/face/face01/fe_f_face01.chr` and matching MTL,
Eye subset 3, with authored `IrisColor=(0.38039219,0.50196081,0.627451)`. It shows two complete
heads with raw/composed Eye albedo and the three source/output atlases. White, unlit factors
isolate the albedo operation. The inspected 1920x1080 capture shows the authored darker blue
iris without changing skin or eyelash subsets. `aa-dev` reports 298.1–355.4 FPS (2.83–3.44 ms)
and exits after capture. The real-client `reference/character_create/step1_ferre_female/real.png`
also has blue eyes; its customized face and stage lighting differ, so this is a qualitative
comparison, not a matching-preset pixel comparison.

Custom face hooks can still replace the static composition with their own face atlas; applying
the native static palette again would double-tint a customized iris. When an Eye hook replaces
base albedo and leaves the inherited albedo-linked emissive texture untouched, the loader follows
the new base for that emissive texture too: Eye emissive represents extra ambient times albedo.
An explicitly changed emissive texture remains the hook's choice. Custom face/picker composition
continues to belong to aa-anim.

Validation (2026-10-17): `cargo test -p aa-cry --lib` passes 66 tests, one exhaustive audit
ignored. Five focused tests cover flags/defaults, modified/animated mapping rejection, sRGB mask
filtering, resolution/mip selection, every Diffuse alpha mip, cache/palette separation, real
512/256 assets, and unchanged/base-replaced/explicit-emissive hook behaviour. The fixture builds
and its inspected window closes after capture.

## Native Illum composition and vertex colours (2026-10-23)

Recovered the interrupted `loader.rs`, `scene.rs`, `shade.rs`, `shade/illum.rs` and `illum.wesl`
work against this client's pak sources: `shaders/illum.ext`, `hwscripts/cryfx/illum.cfx`,
`fraglib.cfi`, `shadelib.cfi`, `fxsamplerdefs.cfi`, `fxsetupenvvars.cfi` and `common.cfi`.
Targeted native decompilation output is retained under ignored
`data/extract/aa-cry/illum-source/color.strings.txt` and `color-upload.decomp.txt`.

* `VERTCOLORS` is **0x400000**; `VERTALPHA` is the separate **0x400** feature. Illum enables
  `bVertexColors` in `frag_unify_parameters`; `fraglib.cfi` 766-768 multiplies final composed
  RGB by **interpolated RGB squared**, before post-balance, additive alpha glow and fog.
  CPU squaring before interpolation is wrong: the midpoint of black/white becomes 0.5,
  whereas native composition gives 0.25. RGB never changes opacity under `VERTCOLORS`.
* Native D3D10 `CRenderMesh::SetMesh_IntImpl` (0x381F4EA0) reads the colour stream at CMesh
  offset 40. Helpers **0x38206540** (16-byte vertices, colour offset 8) and **0x382065A0**
  (24-byte vertices, colour offset 12) write bytes **2,1,0,3**. `COLOR_SWAP=1` in
  `fxsetupenvvars.cfi` and `GetInputColor(Color.zyxw)` swap R/B back in the vertex shader.
  Therefore effective colour is the source RGB order; no R/B swap belongs in the Bevy bridge.
* `scene::insert_illum_vertex_colors(mesh, mesh_data, subset, flip_winding)` compacts unsquared
  source RGB/255 in the exact subset/mirrored index order. Attribute alpha is 1. Source alpha
  stays in `MeshData.colors`; this helper deliberately does not implement `VERTALPHA`.
  `shade::illum::uses_vertex_color(&Material)` checks the shader name and generation mask.
  The loader applies the helper to static **and skinned** flagged subsets, including material
  overrides and hooked loads. Other shaders' streams (notably vegetation bending) are untouched.
* `IllumMaterial = ExtendedMaterial<StandardMaterial, IllumExt>` neutralizes Bevy's automatic
  albedo-times-vertex-colour operation, then applies RGB squared after lighting and before
  fog/tone mapping. Basic brushes retain their existing PBR factors, emission and textures;
  they never receive character gain or back lights. The fragment also calls Bevy's
  `visibility_range_dither`, so the world's new dissolve margins work with this material.
* Full opaque/masked Illum uses the original unnormalized **Phong(R,L,Shininess)**, shadow
  factor `0.8 * shadow + 0.2`, `max(highlight, reflection)` and the ambient hemisphere/sun
  weighting. `HDR_MODIFIER` 0x4 enables the diffuse/specular HDR multipliers; it does not scale
  reflection. Gloss is sRGB (`GLOSSMAP` explicitly sets `sRGBLookup=true`); `GLOSS_MAP` 0x10
  gates it, `SPLIT_GLOSS_MAP` 0x8 uses red for highlight and green for reflection, and
  `GLOSSNESS_GLOSSZ` 0x800000000 uses blue for the exponent only when all three flags and the
  map are present. No-map gloss is white. Reflection requires `ENVCMSPEC` 0x80 and a cube map;
  missing `ReflectAmount` defaults to **0.25**, Fresnel bias/scale to 0/1. It is multiplied by
  ambient plus direct/back light, the gloss mask and Fresnel, independently of `Specular`.
  The native cube sampler has **no** sRGB lookup (`ENVIRONMENTCUBEMAP`, and the two-argument
  `GetEnvironmentCMap` returns the raw sample), unlike the gloss sampler. The cube cache now
  keys by path **and encoding**: native reflection uses the raw BC1 view; stage IBL keeps its
  existing sRGB view. The public `CryAssets::cube_texture` API retains its stage behaviour.
* Native `MatEmissiveColor` joins the ambient accumulator before the sun weighting/composition,
  rather than being appended as a final glow. Parameters convert engine light units with
  `LIGHT_UNIT_LUX / pi`, then the fragment applies view exposure. Full materials retain the
  existing world gain 3 and rotating `CharBackLight`; uploads stay bindless, shared by source
  material, and limited to 64 changed materials per frame. The registry uses the existing
  lightmap-exposure tag mechanism, range 3e6..4e6 (separate from skin/hair/backlight tags).

Integration: `CryPlugin` adds `IllumPlugin` when PBR exists. `Part`, `LoadedModel`, loader and
spawn signatures are unchanged; world instances retaining their part handles need no new call.
Code replacing meshes directly must preserve the colour attribute with the helper above.
Code enumerating material components must include `MeshMaterial3d<shade::illum::IllumMaterial>`:
in aa-client this includes **stealth save/swap/restore**, **combat material-flash selection**
and **housing's invalid-placement red override**. Housing uses Basic; the material survey has
153 VERTCOLORS Illum rows among 414 housing Illum sub-materials (including `hari_housing.mtl`).
Do not layer `CharMaterial` over Illum; its shader already adds the back lights. No world,
anim or client files were edited here.

Limits: Full still uses the shadow-casting directional light as sun; the stage's synthetic
fill/rim remain in Bevy's diffuse/ambient estimate. The follow-up below moves punctual lights
to recovered forward composition. The estimate subtracts a Lambert sun term from white-albedo
PBR, so it is not an exact replacement for the native deferred ambient/SSDO buffers. Native
actor balance, rim/decal accumulation, specular UV animation, anisotropic Illum and blend layers
remain separate gaps. Ordinary `VERTALPHA` / `ALPHAGLOW`, including authored masks, are covered
by the bounded follow-up below; compound permutations remain excluded. Skin shader/face
calibration was not changed.

Targeted waterfall follow-up: `objects/effects/waterfall/waterfall_size_15.mtl` is blended
Illum, opacity 0.7, diffuse TileV 0.3 / V oscillator type 2 / rate 0.25 / amplitude -3;
its Custom texture has a separate type-2 V oscillator (rate 1, amplitude -0.4), an alpha
opacity map, a `_second` dual material and type-1 vertex deformation. The water subset in
`objects/env/02_harihara/01_ma/ma_waterfall_a.mtl` similarly uses opacity 0.34 and type-2
V animation (rate 0.25, amplitude -0.7). The narrow type-2 motion/opacity slice below now
animates these materials. Dual-material passes, object-space perturbation and vertex deformation
remain separate; this does not establish complete waterfall or material parity.

Visual controls: `viewer --full --stage --yaw 180` uses the character-stage rig. Set
`AA_ILLUM_LEGACY=1` for the previous Full metallic/roughness approximation, then unset it for
native Illum. `AA_ILLUM_TUNE="spec=1,env=1,ambient=1,debug=0"` changes diagnostic factors;
defaults are all 1, debug 0 (1 reflection only, 2 highlight only). `illum_colors` uses aa-dev
and needs no pak: black/white and red/blue gradients plus constant 128/255, all with source
alpha 7/255 and an alpha-test material. `--before` disables RGB multiplication, `--dissolve`
exercises visibility-range dithering, `--emission` tests native composition with emissive 0.5
and no scene lights.

Inspected aa-dev captures (Vulkan, RX 9070 XT, copied dev executables; all windows exited):

| Capture in `shots/aa-cry/illum/` | aa-dev FPS range | Result |
|---|---:|---|
| `colors-before.png` / `colors-after.png` | 291-389 / 326-416 | Before: white. After: grey midpoint 138, red/blue midpoint (136,0,138), constant 128/255 RGB -> 138; alpha-7 source remains visible. |
| `emission-after.png` | 294-332 | Native no-light emissive-0.5 composition: constant/midpoint 96, matching the source ambient weighting and RGB squared. |
| `colors-dissolve.png` | 272-364 | 41.67% of fixture pixels survive the range dither; surviving RGB is pixel-identical to the undissolved render. |
| `palace-before.png` / `palace-after.png` | 218-256 / 362-387 | Same castle/twocrowns material and camera: 54,609 pixels change, maximum darkening 26 sRGB steps; average changed RGB -6.53. Local masonry occlusion is restored. |
| `armour-before.png` / `armour-after.png` | 226-267 / 188-220 | Same Saint model and stage, legacy bake vs native: stronger chrome light/shadow contrast, dark undersides and bright edges. |

Compared with `reference/ingame/royal_palace/real.png` and
`reference/character_create/step1_nuian_female/real.png` (the same Saint costume). These isolated
fixtures verify the restored terms; they do not reproduce the reference's complete scene/pose.
The shared machine was running other builds/world captures, so the FPS ranges are observations,
not a controlled performance comparison; native Full adds shader work and needs integrated profiling.

Validation: `cargo check -p aa-cry` and
`cargo build -p aa-cry --example illum_colors --example viewer` pass.
`cargo test -p aa-cry --lib --test loader`: **87 unit tests + 6 loader tests pass**, one
exhaustive material survey ignored. This includes bindless descriptors, native masks/defaults,
material sharing and exclusion of Basic brushes from character lighting, mirrored compaction,
real Palace material overrides and distinct cube encodings. `git diff --check` is clean.

## Punctual Illum forward composition (2026-10-23)

`illum.wesl` now visits Bevy's clustered point and spot ranges (not rect lights/probes).
Full Illum removes their white-albedo PBR contribution from the ambient estimate, then adds
the original **forward** `illum.cfx:frag_custom_per_light` diffuse and highlight:
`saturate(N.L) * shadow * light * falloff * filter` and
`pow(saturate(R.L), Shininess) * (0.8*shadow+0.2) * light * falloff * filter`.
Both join the existing material/gloss/HDR composition and the light-modulated cube reflection.
There is no additional N.L, BRDF normalization, inverse-square falloff, bulb-size broadening
or fitted brightness on this direct term. Basic Illum's PBR path is unchanged.

The radial function is `shadelib.cfi:GetAttenuation`: `saturate(1-d²/r²)`. The existing caller
bridge supplies Bevy luminous intensity I and range r. Bevy's illuminance at r/2 is
`I/r² * 4*(15/16)²`; dividing by native forward falloff 3/4 gives amplitude
`I/r² * 75/16`, then `/pi` for the material's Lambert units. This **preserves the existing
half-radius anchor**, without a tuned gain or caller changes. In particular aa-world's
point conversion `I=.16*r²*lux` yields forward amplitude `.75*lux`; its projector conversion
`I=(16/75)*r²*lux` yields amplitude `lux`. The fx bridge `I=.25*r²*lux` yields
`75/64*lux`. These are parameter bridges, not recovery of each source light's native amplitude.

Spot lights use Bevy's packed direction/inner-outer cone and scalar cookie atlas. Existing
aa-world projectors split their RGB cookie into three coloured lights, so multiplying each
channel preserves that reconstruction. Projected UVs behind/outside the frustum contribute
zero. Point and spot shadow maps (and optional contact shadows) feed the original occlusion
composition, including the 20% highlight floor. These are **Bevy shadows and cone geometry**,
not the native shadow-mask/NVSSAO buffers or original square-projector parameter setup.

Public `IllumParams`, `IllumExt`, `IllumMaterial`, material/loader APIs, existing bindless indices
and the `fragment` entrypoint are unchanged. The aa-world dissolve wrappers keep importing that
entrypoint. Private light-table texture binding/index 55 is appended (index range 50..56).
No caller, root manifest or lockfile was edited. aa-cry's Bevy **dev dependency** enables
`pbr_light_textures` and `pbr_clustered_decals` for standalone GPU checks, matching aa-world's
existing feature selection; otherwise attaching a cookie to the harness silently omits it.

### Source oracle and pass limits

`python crates/aa-cry/tools/illum_source_oracle.py --check` executes the extracted original
`GetAttenuation`, `Phong`, `frag_custom_per_light` and `frag_custom_end` bodies using a small
HLSL-to-Python translation and numpy vector arithmetic. It checks **189** radial/filter/shadow
cases, the shadow floor without an N.L specular mask, **nine** algebraic intensity mappings,
and executes the distinct deferred attenuation/Blinn functions. Source hashes, flags and
results are retained in `tools/illum_source_golden.json`; translated executed bodies remain
under ignored `data/extract/aa-cry/punctual/`. This is execution of original **source**, not
of the shipped GPU bytecode or renderer DLL. The composition oracle selects non-low quality,
split gloss, HDR modifier, no anisotropy/actor balance/rim/decal accumulation.

Native opaque deferred parity remains open. Illum sets `bDeferredSpecularShading=true`;
`fraglib.cfi:frag_get_light_buffers` imports the deferred accumulators. The separate
`deferredshading.cfx:LightPassPS` uses **normalized Blinn**, N.L/shadow on specular and squared
point falloff `(1-d²/r²)²` (projectors linear). Its `LightingTiledPS` has a different clipped
polynomial highlight. At half range the source oracle distinguishes forward/projector .75
from deferred point .5625. Porting forward Phong to this Bevy material does not establish
the original opaque client's pass choice or reproduce either deferred implementation.
Native forward `fraglib.cfi` also declares a four-entry light-type table; the renderer's
per-object light selection/priority is not recovered here. This bridge evaluates the
current Bevy cluster's point/spot set. The 32-light fixtures stress the bridge and cost,
not native forward light-admission parity.

### Authored specular-multiplier audit: release limitation

`python crates/aa-cry/tools/audit_light_specular.py --check` reads ten targeted original XML
cells through `aa-pak`: dock/airport neighbours, three known lamp cells, and login2. The
hashes and all **291** rows are retained in `tools/punctual_evidence/authored_light_audit.json`.
This is a risk sample, not a world census or a visible-light count.

Original `Light.lua` defaults `fDiffuseMultiplier=1`, `fSpecularMultiplier=1`, and
`Options.bDeferredLight=1`. It sends `specular_multiplier=specular/diffuse` when diffuse is
nonzero, else 1. `SimpleLight.lua` defaults `fSpecularPercentage=100` and sends percentage/100.
The 290 sampled Light rows request deferred: 280 explicitly, ten by default. The remaining
SimpleLight has no deferred property in that Lua schema. Thus these isolated forward captures
do **not** validate the pass requested by ordinary authored opaque world lights.

Of 291 sampled lights, **268 have non-one specular ratios**, 23 have ratio one, none default
the authored specular field. Ratios 1/3 and 1/2 each occur 85 times; five are .02. A static
active/nonhidden/positive-diffuse/supported-flag approximation admits 289, of which 266 are
non-one; distance, light budget, cookies and runtime visibility are not evaluated. All 32
dock-cell 018_007 lights are non-one: 30 have ratio 1/3, `Light_116` has ratio .5
(`fSpecularMultiplier=1`, diffuse multiplier 2). The one zero is active `Light_268`,
GUID `4ff59a69200592a5`, cell 021_008, raw specular 0/diffuse -1; its entirely negative diffuse
is excluded by current aa-world's `p.diffuse.max_element() <= 0` admission check. The sample
does not prove the absence of eligible zero multipliers elsewhere.

**Bevy's ordinary light record drops this multiplier. Assuming 1 for authored world lights
is a material release limitation:** the sampled dock highlights would be roughly 2–3 times
too strong before other composition. Root approved the additive bridge below; aa-world's
separate attachment/ratio-only repair landed in `ab918ae` / `b690462`. Authored-light
publication needs both halves. There is no full opaque-lighting completion claim.

`aa_cry::shade::illum::IllumLightSpecular(pub f32)` now attaches to actual PointLight/SpotLight
entities, absent = 1 for existing programmatic/fx lights. aa-world must attach/update
`PlacedLight.specular_multiplier` on each point and each of the three projector channels.
The component uses Bevy extraction/sync so removal also removes its render-world copy.
aa-cry builds a private GPU lookup indexed by the current **render-entity**
`GlobalClusterableObjectMeta.entity_to_index`, in `RenderSystems::PrepareResources` explicitly
**after `prepare_lights`** (which clears/sorts/rebuilds that map in CreateViews). Only local
highlight is scaled; diffuse, ambient and material reflection are independent.

Every update fills the current/high-water prefix with 1 before applying component values;
missing, dead, newly mapped and reused entities cannot inherit a stale zero/scale. Finite
values, including exact zero, are retained without clamping; nonfinite input falls back to 1.
One stable 256 KiB linear RGBA8 texture holds the exact four bytes of each of 65,536
f32 slots, shared by all Illum materials. textureLoad and byte reconstruction avoid filtering,
sRGB conversion and half-float rounding; only changed row-aligned prefixes are uploaded.
IDs beyond that capacity explicitly return 1, not an out-of-bounds fetch. GPU resource initialization follows RenderStartup/device recovery.

A private AsBindGroup adapter supplies the shared texture while preserving all public
`IllumExt` fields. Existing indices 50..54 stay put. The extension needs the same texture-array capability as StandardMaterial for bindless,
and its ordinary-binding fallback uses the same table. It does not add a buffer-array
requirement: the initial raw-buffer design exposed Bevy's mixed-capability ExtendedMaterial
layout bug (retained buffer-array-layout-failure.txt), so that private design was replaced.
There is no overloading of intensity, range, colour, or Bevy flag bits. Optional
`AA_ILLUM_LIGHT_TRACE=1` logs main entity bits, current GPU index and effective multiplier
when mapping/values change, for checking extraction/reindex timing during a running fixture.

### GPU checks, matched captures and cost

`illum_lights` is an aa-dev example with a fixed orthographic plane, optional original
castle/twocrowns or Saint armour model, and separate point/spot/directional controls. It
supports range, position, projected aim, black/checker cookies, shadows, highlight/diffuse/
reflection isolation, explicit specular ratios, 32-light stress cases and a live churn case.
All authored-material captures use **explicit ratio .5**, taken from `Light_116`; there is no
assumed-one authored-light comparison. The lamp is an analytic fixture, not the full native
world/stage rig. The original Palace and character-create references were inspected again;
these isolated dark/no-ambient fixtures do not reproduce those references' scene or exposure.

Reproduce current captures with `cargo build -p aa-cry --example illum_lights`, copy the exe
to `data/extract/aa-cry/punctual/illum-after.exe`, then
`python crates/aa-cry/tools/capture_illum.py after`. The script closes every window. The
pre-change executable's captured PNGs/logs are retained, but its scratch exe is not delivered.
`tools/punctual_evidence/provenance.json` records its hash and predecessor shader ref, the
final shader/binary hashes, exact arguments and baseline projector-feature limitation.

`python crates/aa-cry/tools/validate_illum_gpu.py` passes **6,240** pixel samples against
executed original forward bodies: maximum error **0.5744 sRGB step**. This includes
point and spot ratios **0, .02, 1/3, .5, 1**, near/range changes, and cone/aim controls.
Zero highlight is exactly black. Missing component equals explicit one pixel for pixel;
changing specular leaves isolated diffuse and material reflection pixel-identical. Black
projectors contribute zero; the checker controls the projected footprint. No-light and
directional-only renders are pixel-identical to before. Point and spot shadow fixtures
contain 223 / 118 pixels at the original 20% highlight floor (within quantization tolerance).

The live GPU churn trace records initial scales `0,.5,1/3`, then reindexed `.5,1,0`
(including component removal/default reset), then `1/3,.02,.5` after despawn/reuse. Its final
PNG is pixel-identical to a separately built static final state. Three compilation/binding
permutations agree pixel for pixel: normal bindless, buffer arrays disabled while texture
arrays stay enabled (**still bindless**), and both array features disabled (**ordinary
bindings**). Public material fields and the imported fragment entrypoint stay unchanged.

Inspected matched PNGs and all logs live in `crates/aa-cry/tools/punctual_evidence/`:
`point-{before,after}.png`, `palace-{point,spot}-{before,after}.png`,
`armour-{point,spot}-{before,after}.png`, plus the authored `*-point-highlight-*` isolates.
The latter show the restored armour/masonry highlight directly; non-Illum skin remains
visible in the armour's before isolate. `validation.json` contains pixel counts and FPS
ranges; the whole source/capture/filter/shadow/default/churn matrix is retained.

Observed aa-dev FPS (1280x720, Vulkan RX 9070 XT, copied dev exe): one point **286–346 before /
302–340 after**, one spot **378–466 / 271–431**; 32 points **305–457 / 247–302**, 32 spots
**264–316 / 292–332**. Palace with one point: **282–336 / 303–473**; Saint armour with one
point: **285–356 / 249–335**. CPU builds and other sessions ran throughout; these are
observations, not a controlled performance comparison. The shader adds direct-light work;
integrated world profiling remains necessary.

Final validation: crate build/check pass; **88 unit tests + 6 real-pak loader tests pass**, one
exhaustive survey ignored. The scoped all-target Clippy run completed once with existing
warnings retained in `clippy.log`; its new trace-tuple complexity warning was factored into
a type alias and checked, without running Clippy a second time. Source oracle, ten-cell
authored audit, GPU validator and `git diff --check` pass. No push or foreign-path edits.

## Costume gloss and stage lighting (previous approximation)

The bake below is now the `AA_ILLUM_LEGACY` comparison path, not the default Full Illum shader.

* `bake_specular` (Illum, `Shading::Full`) treats a material as costume by `ReflectAmount` (7 - 15 on costumes, 0.25 on props;
  factor `ReflectAmount / 8`, clamped): `COSTUME_GLOSS_BOOST` 8 on the glossiness of the specular map, sharpest roughness 0.3,
  warm highlight tint 0.1, `COSTUME_DIFFUSE` 0.85 on the diffuse. Only `Illum` is touched: hair, skin and eyes are not.
  The real costume leather is lacquer-glossy (the shader multiplies a dim map by `ReflectAmount`); the trims shine white-silver.
* Measured on the step 2 captures (geometric mean real / ours of the figure's luma): mean 1.06 -> 1.00, q90 1.36 -> 1.35,
  q99 1.36 -> 1.21 (frame fix and gloss together); real still has darker darks (q25 0.25).
* Shadows: casters and receivers are the default for every character / creature mesh (nothing sets `NotShadowReceiver`); in
  game `ingame/units.rs` drops the caster flag beyond a radius. The key's shadow is small on the stage because the fill and
  environment light the shaded side. SSAO changes almost nothing (it only dims ambient light).
* Step 1 "much darker in the real capture": the figure's mean luma matches (real / ours 0.9 - 1.1 for most races); the real
  has deeper midtones and brighter highlights (gloss, shadow), and the costume leather is darker than ours while the face
  matches. Not an exposure difference. A light-basis fit (`--key-from`, NNLS over 24 directions) puts most of the light near
  the camera direction at 15 - 45 degrees elevation.

## Hair (`shade::hair`, `hair.wesl`)

The client's `Hair` shader (`shaders/hwscripts/cryfx/hair.cfx` in the pak, see `docs/research/hair.md`) ported as
`HairMaterial = ExtendedMaterial<StandardMaterial, HairExt>` (bindless like `CharMaterial`):

* **Strands.** Two Kajiya-Kay lobes `sqrt(max(1 - (T.H)^2, 0.01))^Pow` along the binormal (the texture's V axis:
  checked on the hair meshes, Bevy's bitangent `cross(N, T) * w` follows `dP/dv` with a dot of 0.9 - 0.95), shifted
  along the normal, not normalised: `T = B + N * w * |dot(V, N) * 0.8 + 0.2| * (noise + Shift)`, the noise
  `((2a - 1) * 2 - 1) * BumpScale` from the Specular map's alpha (the source applies the remap twice). Primary lobe
  `PrimarySpec{Color,Pow,Shift}`, secondary `SecondarySpec*`, each light's highlight times its wrapped `N.L`; the
  "shadow area" highlight `0.2 * KK(T2, (V.x, 1, V.z), Pow * 0.3)` where a directional light is shadowed; the ambient
  highlight (the view as half vector) times `AmbientSpecMult` and the light received. All of it times the Specular
  map's rgb and `Specular`. No GGX, no environment reflection: those were the plastic sheen of the old
  `StandardMaterial` (an isotropic GGX lobe with reflectance up to 1.8 over a baked strand mask, plus Bevy's grazing
  `F_ab.y` environment term).
* **Diffuse.** Bevy's PBR lighting runs once on a white albedo with every specular term off (reflectance 0,
  `specular_occlusion` 0): it gives the light received (ambient, environment, point lights, shadows). Its Lambert part
  of the directional lights is replaced by the engine's: wrapped `saturate(N.L * DiffuseWrap + 1 - DiffuseWrap)` plus
  the back rim `(1 - N.V)^3 * RimMultiplier * (1 - L.V)`. Light colours: Bevy lux / pi = the engine's units.
* **Back lights.** The two `Char BackLight`s (`CharBackLight`, when the resource exists) go through the same per
  light terms, in the character's space; hair is not swapped to `CharMaterial`.
* **World.** While the back lights are on (in the world) hair gets `WORLD_DIFFUSE_GAIN` (3, `mode.z`) on everything
  but the back lights, like skin and costumes: its colours are calibrated under the login stage's lights (three
  times the engine's units), and without the gain blond came out a dark khaki where `reference/hair/ingame_*` is a
  pale bright blond.
* **Colour parameters** go through `hair_colour` (hue as written, brightness `^1.5`): plain `^1.5` per channel made
  blond a saturated gold, plain values made dark hair olive grey; the captures have ash blond and near black.
* **Calibration** `DIFFUSE_GAIN` 0.55, `SPEC_GAIN` 1.3: fitted to the lit top of the hair in the `step2_*` captures
  (at 1 / 1 the blond came out about 1.7 times too bright and too flat; the stage rig is brighter than the engine's
  for hair, the skins carry per-model tunes for the same reason).
* **Alpha.** The hair mesh stays the opaque pass: `Mask(0.5)` on the doubled alpha, safe in the depth and normal
  prepasses (SSAO checked: `compare --ssao medium`). A child entity `HairFringe` (same mesh, `SkinnedMesh` copied,
  `NotShadowCaster`) draws the engine's blended passes: `AlphaMode::Blend`, only the texels the mask cuts away and
  above `clip(a - 0.05)`, alpha `saturate(2a)`. Blend materials are not in the prepasses, so nothing changes there;
  the hairline is soft like in the captures instead of spiky.

Wiring: `convert_material` (`Shading::Full`) calls `hair::prepare`, which registers the parameters and the Specular
map and stores the index in `lightmap_exposure` (`4 + index`; the back light tags are below 3). `HairPlugin`, added by
`CryPlugin` when `PbrPlugin` is present, swaps every entity whose `MeshMaterial3d<StandardMaterial>` carries the tag
for a `MeshMaterial3d<HairMaterial>` and spawns the fringe child; `follow_meshes` keeps the child on the parent's
`Mesh3d`. Without the plugin (or with `Shading::Basic`) the plain `StandardMaterial` stays.

To draw hair differently (stealth, fades): the hair entity has `MeshMaterial3d<HairMaterial>` and a `HairFringe(child)`;
the child is a descendant with `Mesh3d` and `MeshMaterial3d<HairMaterial>` too. Swap both like `CharMaterial`
(remove `MeshMaterial3d<HairMaterial>`, insert the other material; to restore, remove that one and insert the saved
`HairMaterial` handle), or hide the child with `Visibility::Hidden`. Inserting a `MeshMaterial3d<StandardMaterial>`
without the hair tag is left alone by `HairPlugin`. (`aa-client` `ingame/stealth.rs` does this: the hair turns into the ghost, its fringe is hidden.)

Shots: `shots/aa-cry/hair/compare_<target>.png` (real client | before | after),
`cargo run -p aa-anim --example compare -- --target step2_elf_female --shot shots/aa-cry/hair/after_step2_elf_female.png`.
`Illum` hair (some ferre styles, astra, dwarf, warborn) keeps the `Illum` path: matte in the captures and here.

## Skin (`shade::skin`, `skin.wesl`)

The client's `HumanSkin` shader (`shaders/hwscripts/cryfx/humanskin.cfx` with `fraglib.cfi` / `shadelib.cfi`, extracted to
`data/extract/aa-cry/skin/`) ported as `SkinMaterial = ExtendedMaterial<StandardMaterial, SkinExt>` (bindless, index
table 50..55: data, gloss map + sampler, SSS map + sampler). Why the old skin looked oily: Bevy's normalised GGX lobe
(reflectance 0.4 at roughness 0.5 on faces, aa-anim's 0.7 at roughness 0.3 on bodies) spread a sheen over the whole
face, and Bevy's environment specular mirrored the bright sky at every grazing angle (in the world on top of the
x3 world gain). The engine has neither:

* **Specular** per light: plain, unnormalised Blinn-Phong `pow(N.H, Shininess * gloss.a)` times the light colour and
  its shadow, no Fresnel, no `N.L` (here faded out behind the surface). The Specular ("gloss") map masks it:
  `x gloss.r x lerp(Specular, 1, gloss.b)` with `GLOSS_MAP_IGNORE_SPECULAR_COLOR` (0x2000000000, bodies),
  `x gloss.r x Specular` with `SPLIT_GLOSS_MAP` (0x8, where `gloss.g` also triples the exponent) or `FUZZYNESS`,
  else `x gloss.rgb x Specular`; without `GLOSS_MAP` (0x2) the diffuse map's alpha is the gloss. `Shininess` 25
  (bodies) to 65 (nuian face), the maps about 0.25: a small highlight on nose and forehead. Only the shadow casting
  light (the sun) makes it: the login stage's fill and rim lights stand for the engine's sky light.
  An environment reflection (`pow(env, 3) x light x gloss.g`, the "lip" gloss) exists only with `ENVIRONMENT_MAP`
  (0x100); it is not ported (the nuian faces and bodies do not have the bit, some elf and hariharan faces do).
* **Diffuse** per light (the sun): `SkinfNdotL2`, `T = smoothstep(-DiffusionAmount * dark, 1, N.L)` (unclamped `N.L`;
  `dark = lerp(sss.r, sss.rgb, SSSMapSat)` from the `SubSurface` map, read linear),
  `lerp(dark, lerp(MSkinColor * (SubsurfaceStrenght + 1), BSkinColor, N.L^2), T) * T`, `^1.5 * 1.5`, `x 0.8` towards a
  light behind the camera, times `luma(light) * 0.9 + light * 0.1`; shadows take `reShadow * 0.8 + 0.2` with
  `reShadow = saturate(shadow * max(N.L * 0.7 + 0.3, L.V) * 0.7 + shadow * 0.3)`, and add the colourless shadow rim
  `(1 - |N.V|) * (1 - reShadow) * RimPow * (N.up * 0.5 + 0.5) * (1 - N.up)`. The base colour stays the calibrated
  colour of fully lit skin (`skin_base_color`, aa-anim's `skin_colors` row and `skin_tune`), so the fall-off is used
  relative to full light (`SkinfNdotL2(N.L)^1.5 / BSkinColor^1.5` with the material's own colours): 1 in the sun, a
  soft wrap with the sub-surface colour into the terminator.
* **Ambient**: Bevy's PBR lighting on a white albedo with every specular term off (ambient, environment diffuse,
  SSAO, the stage's fill lights) minus its Lambert part of the sun, times the hemisphere factor `N.up * 0.25 + 0.75`
  (`bHemisphereLighting`: half the ambient under the chin, nose and lips), plus `0.3 x reColorInShadow` (a skin
  coloured fill, `lerp(dark * 0.7, lerp(M, (B + M) * 0.6, N.V^1.3), N.V)`) in engine units. The shader sets
  `bDontUseAmbientOcc` (no AO from a map or the diffuse alpha); SSAO is Bevy's (on in the world, `post.rs`).
* **Detail**: the face normal map is applied at full strength (CryEngine's `GetNormalMap` has no Y flip, the mesh
  binormals are kept, see the CGF section; checked with `AA_SKIN_TUNE=debug=1|2|3`: shading normal, mesh normal,
  flipped Y). `HumanSkin` has no tiling detail map; its only extra layer is `WRINKLE_BLENDING` (animated wrinkle
  normals from per-vertex weights), not ported. Stubble, brows and makeup are decals of the composed face texture
  (aa-anim `face.rs`).
* **Back lights** (`X2BackLight`): plain `N.L` on `texture x Diffuse` (as `CharMaterial`) and a Phong highlight
  `pow(R.L, Shininess)` x the specular multiplier, masked like the sun's.
* **World**: while `CharBackLight` is active the material carries `WORLD_DIFFUSE_GAIN` (3) on everything but the back
  lights, like `backlight::world_look` does for costumes; skins are not swapped to `CharMaterial` any more (their
  tag is `2e6 + index`, outside the back light and hair tags).
* **Calibration** (`TUNE` in `skin.rs`, `AA_SKIN_TUNE="spec=..,diffuse=..,ambient=..,fill=..,sss_srgb=0|1"` for
  experiments): `Specular x 0.35` (at 1 the stage's key light, 3x the engine's units, made a broad bluish sheen on
  the forehead that the captures do not have), ambient x 0.5 (the engine multiplies it by `texture x Diffuse`, the
  base colour carries the lit skin colour). aa-anim's `skin_tune` table was refitted for the new lighting (the old
  rows cut blue, against Bevy's sky-blue environment specular): `data/extract/aa-cry/scripts/skintune.py`, the
  `step1_*` faces, mean linear colour within 1 % of the captures.

Wiring like hair: `convert_material` (`Shading::Full`) calls `skin::prepare` (parameters, gloss and SSS maps into a
registry, the index into `lightmap_exposure`), `SkinPlugin` (added by `CryPlugin` with `PbrPlugin`) swaps tagged
`StandardMaterial`s for `SkinMaterial`s. To draw skin differently (stealth) swap `MeshMaterial3d<SkinMaterial>` like
`HairMaterial` (`aa-client` `ingame/stealth.rs` does).

Second round (`shots/aa-cry/skin2/`, before | after | real, kept): hemisphere ambient, multiplied brow and stubble
decals, linear iris tint, matte eyeball, hair world gain, skin tunes refitted again. The login stage's grading
(`login_color_grading`, post-saturation 1.6) is what keeps irises bluer than the captures.

Face contrast (open; measured, nothing changed). The stage refit left faces flatter than the captures. The measurement
uses a hand-placed face box per `step2_*` target, intersected with the skin pixels of both images. The luma is linear
and blurred. The coefficient of variation (cv) is 0.48 for us and 0.63 for the captures. The p10 / p90 ratio is
0.26 against 0.15.
* **humanskin.cfx, reread.** The shadow side is lit by three things only:
  * the wrap `SkinfNdotL2`: `smoothstep(-DiffusionAmount * dark, 1, N.L)`, coloured from the SSS map, then `^1.5 * 1.5`;
  * the rim term;
  * the constant `reColorInShadow * 0.3`, added to `cDiffuseAcc` in `frag_custom_begin`.

  The shader also sets `bHemisphereLighting` (ambient times `N.z * 0.25 + 0.75`) and `bDontUseAmbientOcc`.
  `fraglib.cfi` adds two more terms: D3D9 SSDO scales `fNdotL` by the bent-normal occlusion, and D3D10 NVSSAO scales
  `fOcclShadow`. For actors, `X2CustomBalancePreEnd` rescales the lights above the ambient and fill by
  `Char Light Multiplier`, but that is 1 in `game/studio_tod.xml`, so it is not the cause.
* **The gap is mostly on the lit side, not in the shadows.** These are face-luma quantiles over the median for
  elf male, nuian male / female and hariharan male:
  * captures: p10 0.23 - 0.36, p75 1.6 - 1.9, p90 2.1 - 2.5;
  * ours: p10 0.35 - 0.40, p75 1.2 - 1.6, p90 1.4 - 2.0.

  With the skin fill off (`AA_SKIN_TUNE=fill=0`), the dark tail already matches (p10 / p90 0.13 against 0.13), but
  cv only reaches 0.58 against 0.69. With the key alone (fill, rim, environment, ambient and skin fill all 0), cv is
  0.58 - 0.61.
* **The tonemapper accounts for part of it.** `--tonemap none` alone raises cv from 0.48 to 0.56 and lifts p90 to
  1.7 - 2.1. TonyMcMapface's highlight shoulder squeezes the lit cheek, which the captures (CryEngine's HDR curve)
  keep at about 2.3 x the median.
* **`fill=0.5`** (the shadow fill halved) gives cv 0.51 and p5 / p50 0.30 against the captures' 0.24, with the mean
  5 % darker. It was not adopted: the gain is small and it is not checked in game.
* **Next steps:**
  * a stage tonemapper with a softer shoulder (aa-client / stage);
  * a broad lit-side term: the captures' cheek and nose-bridge sheen is part of the highlight, and the spec tune
    0.35 removes it. That has to be weighed against the shine complaint.
  * then refit the fill (about 0.5) and the skin tunes.
* **Iris.** The stage-only eye desaturation against the 1.6 grading is not done.

Shots: `shots/aa-cry/skin3/luma_{nuian_male,nuian_female,elf_male}.png`. Each panel shows HEAD | `fill=0.5` |
`--tonemap none` | real, colour on top and median-normalised luma (0 - 3x) below.

Shots (kept): `shots/aa-cry/skin/compare_<target>.png` (before | after | real, the eight `step2_*` and `step1_nuian_female`),
`compare_step2_sheet_{a,b}.png`, `compare_ingame.png` / `compare_ingame_close.png` (refhair / Hairrefb at noon and in the
morning against `reference/hair/ingame_*`).

## Characters under the world's light

Skin and costume colours are calibrated on the login stage, whose lights are `LOGIN_EXPOSURE` (3) times the engine's nominal
units; the world's are nominal, so in game faces were grey-brown and costumes dim, and the gloss (a boosted dim map, i.e.
dielectrics promoted to metals) mirrored the bright sky: a grey-purple sheen on leather. `shade::backlight::world_look`
fixes the copy of the material that `swap_materials` makes (only where the plugin is active, i.e. in the world; the login
stages never swap): the colour of skin and costumes (not hair and eyes, `NEUTRAL_TAG`) times `WORLD_DIFFUSE_GAIN` 3, the
reflectance of costumes times 0.3; the back light keeps its calibration (its albedo factor divides the gain out).
In `bake_specular` the metal mask uses `COSTUME_METAL_BOOST` 2 instead of the highlight boost 8, so leather and cloth stay
dielectrics. Checked against `reference/hair/ingame_{noon_sun,morning_backlit,shade}_front.png`: pale pink face, matte tan
jacket; the palace shade (`reference/ingame/royal_palace`) is unchanged within the back light's calibration.

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

* Detail and blend layers remain approximate. Full Illum now uses the material's environment map;
  HumanSkin uses its SSS map (above), and aa-world owns vegetation bending/wind. Static Eye iris composition is supported above;
  custom-face replacement and native Eye reflection/cornea lighting remain separate.
* The legacy silver-armour approximation was dull (`F0 = 0.216 * texture`). Full Illum replaces
  that mapping; integrated character/stage parity still needs comparison beyond the isolated fixtures.
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

## Waterfall type-2 texture motion and opacity (2026-10-23)

This is a material slice in `aa-cry`, automatically used by `CryPlugin`/the normal loader.
The actual original `objects/effects/waterfall/waterfall_size_15.cgf` and its same-name Illum
material were static. Its diffuse map authors TileV .3, V oscillator type 2, rate .25,
amplitude -3; `objects/env/02_harihara/01_ma/ma_waterfall_a.mtl`'s water subset authors
TileV 1.5, rate .25, amplitude -.7. Neither is a lake/river caustic or an invented foam effect.

`tools/waterfall_source_oracle.py` executes the **shipped original x86 machine code** at
`cryrenderd3d10.dll:0x3822BEB0` (14,267 bytes), including its original matrix multiplication
routine, under Unicorn. The DLL SHA256 is retained in `tools/waterfall_evidence/native_oracle.json`.
The material loader `cry3dengine.dll:0x301A6230` maps oscillator types to modifier offsets 4/5,
rates to 88/92, amplitudes to 96/100 and phases to 104/108. The renderer's type-2 cases write
`amplitude * (rate * seconds)` to matrix translation, then multiply by static tiling and offset.
Phase is not read for type 2. Thus size15's final V translation at 1/2/4 seconds is
-.225/-.45/-.9, **not** -.75/-1.5/-3. The retained oracle passes 36 original matrices,
including signed two-axis/phase controls and 3599.75/3600.25/3601/7201 seconds, with zero
translation error. This is offline matrix execution with explicit native time argument `a4`,
default coordinate generation and zero rotation, not execution of the native GPU draw.

Original shader ownership is explicit:

* `VertexLib.cfi:vs_shared_output` transforms diffuse through `_TCModify`, but initializes
  `opacSpecTC` from **raw `IN.baseTC`**. Ordinary `OPACITY_MAP` leaves those opacity coordinates
  untouched, even when the Opacity slot has a TexMod. `X2_PARTICLE_DARK_MAP` is a distinct path.
* `fraglib.cfi:frag_unify` sets `bumpTC=baseTC`; ordinary gloss also samples baseTC. Consequently
  the diffuse input shift correctly moves the existing normal/gloss inputs together.
  Separate `SPECULAR_UV_ANIMATION` and diffuse-blend permutations decline this slice.
* `illum.cfx:frag_custom_begin` multiplies alpha by
  `saturate(dot(opacityChannelMask, opacityMap))`, in addition to diffuse alpha and authored
  material opacity. `fxsamplerdefs.cfi:OPACITYMAP` has no sRGB lookup. The opacity image is
  therefore loaded linear and sampled at raw mesh UV, independent of moving diffuse UV.
  Missing channel mask uses the source default (0,0,0,1).

Only blended Illum with effective nonzero type-2 diffuse motion is opted in. Types 0/2 are
accepted per axis; other oscillator types, nonzero rotation, nondefault coordinate generation,
unknown modifier attributes and the conflicting shader permutations decline. Attributes stay
in `TexMod::unhandled`, so existing static Eye/UCC bake rejection and the public parser layout
are unchanged. Basic/Full blended surfaces keep their existing PBR lighting approximation;
this does not opt them into Full Illum's native forward lighting.

Public `IllumParams`, `IllumExt`, `IllumLightBuffer`, `IllumLightSpecular`, loader/part APIs and
`fragment` are unchanged. The previously unused `params.vertex.w` is a private registry token;
all existing programmatic constructors use zero. Token zero and invalid/fractional/nonfinite
values resolve to disabled motion/opacity. No new material component or caller integration is
required, including the world's Illum dissolve wrapper. Private GPU data adds the oscillator,
mask and enable flags. Existing binding/index 55 and the light table's encoding/mapping are
unchanged; 56/57 add opacity texture/sampler and 58 adds a **separate** shared 1x1 RGBA8 clock
texture, with the exact four bytes of unwrapped `Time::elapsed_secs()`. No clock is packed into
the light lookup. The native negative-time fallback reads the render-thread's current time;
this bridge uses elapsed Bevy render time with an independent startup epoch, no fitted speed
or phase. `globals.time` is deliberately avoided because its default one-hour wrap would
jump nonperiodic authored motion. Parameters/material assets are not rewritten every frame;
one four-byte clock upload serves every material. Both bindless and ordinary bindings use it.

The `waterfall` aa-dev example fixes only render-world time with `--time`; main-world time
continues for screenshot-and-exit and FPS logging. `--quad --unlit` shows the **original
material/maps** on a diagnostic UV square, with no substitute waterfall texture. The mesh
fixture uses the original geometry and `--yaw -90`; camera, source, time, binary and screenshot
hashes plus FPS samples are retained in `tools/waterfall_evidence/captures.json`.
`AA_ILLUM_MOTION_LEGACY=1` retains the pre-slice conversion; `--token-zero` explicitly exercises
the existing-material path in the same final binary. Reproduce via:

```text
cargo build -p aa-cry --example waterfall --example illum_lights --example illum_colors
# copy the three executables to data/extract/aa-cry/waterfall/{waterfall,lights,colors}-after.exe
python crates/aa-cry/tools/capture_waterfall.py after
python crates/aa-cry/tools/capture_waterfall.py after colors light-point light-spot
python crates/aa-cry/tools/waterfall_source_oracle.py --check
python crates/aa-cry/tools/validate_waterfall_gpu.py
```

Limits: size15 also authors independently animated Custom perturbation (type-2 V rate 1,
amplitude -.4, TileV .8), `OBJECT_SPACE_DEFORMATION`, type-1 `VertexDeform` (WaveX amplitude .6)
and `waterfall_size_15_second` as DualMaterial. Ma's water has WaveX amplitude .2 and
`si_rock_01_mossy_water_dual`. Those passes/deformations are not implemented here. The simpler
`waterfall_second.mtl` diagnostic (TileV 1.3, rate .32, amplitude -1) has neither a dual pass
nor vertex deformation, and provides the nonperiodic one-hour clock control. Waterfall particles
and placements/streaming remain `aa-fx`/`aa-world` work. No matched real-client waterfall scene
capture exists in `reference/`; native comparison here is the executable timing oracle and
original shader composition evidence. Native opaque deferred/tiled lighting remains open.

Validation and inspected evidence:

* `cargo check -p aa-cry --examples` and the three-example build pass.
  `cargo test -p aa-cry --lib --test loader`: **92 unit + 6 loader tests pass**;
  one exhaustive 38k-material survey is ignored. No Clippy run was needed.
* `validate_waterfall_gpu.py` passes **728** primary original-map pixel samples at
  t=0/1/2/4: maximum error **1.696 sRGB steps**, mean per time .497-.527. Timing offsets
  are obtained by executing the original x86 matrix routine; colour/alpha expectations
  use the original DDS, native opacity law and the fixed unlit camera. BC3 decode/filter
  rounding remains in the error, rather than being fitted away.
* Moving opacity with diffuse is explicitly rejected: mean error **19.452** versus
  **.497** for raw opacity UV at t=2. Static tiling alone fails: the original-mesh
  t=0/t=2 baseline changes **0 pixels**, final motion changes **168,483**; the UV square
  changes **233,122** pixels. The opacity fade stays attached to the mesh.
* Bindless and ordinary captures are pixel-identical at t=2 and **3600.25 seconds**.
  Token-zero equals the original pre-change UV-square capture exactly. Existing
  `illum_colors` and point/spot fixtures (explicit specular .5) are pixel-identical
  before/after, preserving the punctual-light work and no-motion path.
* The nonperiodic second material is captured at .25, 3599.75, 3600.25 and 3601.25 seconds.
  At 3600.25 the unwrapped/native base-mip model has mean error **1.505**, whereas the
  incorrect one-hour-wrapped model has **36.981**. This is a clock discriminator, not
  an exact anisotropic minification oracle (maximum correct-model error 8.539).
* Actual mesh captures `tools/waterfall_evidence/mesh-yaw-90-t{0,2}-{before,after}.png`
  were inspected, as were the opacity/ordinary-binding and >3600-second controls.
  aa-dev observed **285.4-347.8 FPS before**, **329.7-401.2 after** on the original mesh,
  Vulkan/RX 9070 XT, copied dev executables, 960x720. Other sessions/builds make this
  unsuitable as a controlled performance comparison. All capture processes exited;
  copied executables, temporary IDA databases and scratch scripts were removed.
  PNGs, commands/FPS/hashes, source provenance and native/GPU validation JSON remain
  under `crates/aa-cry/tools/waterfall_evidence/`; small extracted source/assembly stays
  under ignored `data/extract/aa-cry/waterfall/`. `git diff --check` is clean.


## Ordinary Illum vertex alpha and diffuse-alpha glow

Original `game/shaders/illum.ext` assigns `VERTALPHA=0x400`, `ALPHAGLOW=0x2000`,
`VERTCOLORS=0x400000`. `illum.cfx:873-899` proves the output-alpha precedence:
start with diffuse-map alpha; `VERTALPHA` multiplies interpolated vertex alpha;
`ALPHAGLOW` replaces that result with vertex alpha outside `DIFFUSE_TEX_BLEND`;
material opacity follows. `fraglib.cfi:764-777` multiplies composed lighting by
interpolated RGB squared, then **in the ambient pass only** adds raw diffuse-map
RGB times raw diffuse-map alpha times `AmbientMultiplier` (default 1). This is
independent of material Diffuse/HDR factors, vertex RGB and GlowAmount. The combined
forward material adds it once before fog, in the existing engine light-unit bridge
(`LIGHT_UNIT_LUX / pi = 998`, multiplied by view exposure).

The loader now supplies original normalized RGBA for admitted Illum subsets in both
Basic and Full shading. Absent streams retain white vertex defaults. The public
`insert_illum_vertex_colors` stays RGB-only with alpha 1; `uses_vertex_color` still
means only VERTCOLORS. Private registry data uses the existing loader token in
`IllumParams.vertex.w`; zero/invalid tokens disable alpha/glow and motion. Public
`IllumParams`, `IllumExt`, light types, material factories and `fragment` signatures
are unchanged. The live specular lookup, private waterfall clock and bindings 55-58
are unchanged. Basic keeps PBR lighting; Full keeps its existing forward composition.

A private `illum_bindings.wesl::compose_alpha` is shared by the colour fragment and
`illum_prepass.wesl`, including shadow alpha discard. The latter retains Bevy 0.20's
normal-map and motion-vector outputs and calls the stock prepass for disabled tokens.
It uses Bevy's alpha-test thresholds; it does not attempt native shadow filtering or
native deferred lighting. Diffuse-only all-mip opacity proofs no longer remove masks
from admitted materials because vertex alpha can still cut them out. The shader library
is loaded permanently in rendered apps; CPU-only CryPlugin conversion remains usable.

The world `LodDither` material needs no API change: its default prepass fragment delegates
to the embedded Illum material. Its existing prepass vertex forwards colour, UV and
instance index. Root must verify the nested `LodIllumMaterial` after rebuilding; no world
files were edited here. To obtain an original tagged material for that test, load
`objects/effects/haje_hammer.cgf` with material override
`objects/effects/haje_hammer_second1.mtl` and Full shading, take the first loaded part's
StandardMaterial handle, and let CryPlugin swap it to IllumMaterial. Preserve the complete
extension (including vertex.w) when wrapping it. The authored material has ALPHAGLOW and
AlphaTest .059; a rectangle with an X vertex-alpha gradient is a useful controlled mesh.

The full pak-derived material inventory contains **446** flagged Illum submaterials:
188 VERTALPHA, 262 ALPHAGLOW, with four overlaps. Original candidate `.mtl` files were
re-read using `tools/aa-pak.exe` and retained with hashes. This slice admits **169**
(29 with VERTALPHA, 142 with ALPHAGLOW, two overlaps), including **15 authored alpha tests**.
The remaining **277** keep the previous path: 269 have unaudited generation bits and
eight have unaudited diffuse TexMod attributes. The explicit ordinary mask allows only
HDR/gloss/reflection, these three vertex/alpha flags, DISABLE_RAIN_PASS and inactive
ALLOW_POM. OPACITY_MAP, diffuse blending, outline/dark/UCC/dye/decal layers, active
POM/tessellation, anisotropy, unknown bits, rotated/animated diffuse coordinates and
missing diffuse slots decline. `tools/illum_alpha_evidence/inventory.json` lists every
flagged row, admission/exclusion reason, alpha test, opacity and original source hash.
This is bounded progress, not closure of every authored Illum permutation.

Reproduce from copied executables (normal shared target, no alternative build target):

```text
cargo build -p aa-cry --example illum_alpha --example illum_colors --example illum_lights --example waterfall
# copy target/debug/examples/{illum_alpha,illum_colors,illum_lights,waterfall}.exe
# to data/extract/aa-cry/illum-alpha/<name>-after.exe
python crates/aa-cry/tools/illum_alpha_source.py
python crates/aa-cry/tools/audit_illum_alpha.py
python crates/aa-cry/tools/capture_illum_alpha.py after
python crates/aa-cry/tools/validate_illum_alpha.py
cargo run -p aa-cry --example illum_alpha -- --asset hammer --quad --unlit --prepass --shot shots/aa-cry/illum-alpha-mask.png --shot-after 5
```

The aa-dev example fixes render time at 2 seconds while main-world screenshot/FPS time
continues. It displays the original bow/hammer/coral meshes or original maps on a UV
square. The plant trunk uses the original combined VERTCOLORS+VERTALPHA+ALPHAGLOW material
with AmbientMultiplier 2.13; the road isolates authored opacity .723 and VERTALPHA;
the hammer isolates ALPHAGLOW with AlphaTest .059; coral isolates VERTALPHA with AlphaTest
.5. `--uv-window .25` magnifies original mip0 for tighter numeric tests. `--prepass` adds
depth, normal and motion prepasses; `--shadows`, `--dissolve`, `--no-colors`, `--token-zero`
and `--no-bindless` provide bounded controls. The capture tool rejects shader warnings
containing unresolved imports or lowercase errors as well as ordinary errors/panics.

Source/GPU validation results and before/after picture/FPS evidence are retained under
`crates/aa-cry/tools/illum_alpha_evidence/`; exact commands, executable/PNG hashes and
all aa-dev FPS samples are in `captures.json`. No matched real-client scene for these
assets exists in `reference/`; native comparison executes selected original HLSL alpha
and composition statements (144 cases), with original DDS samples for the fixed-camera
GPU checks. It does not execute compiled native GPU shaders or reproduce deferred buffers,
lighting, dual-material passes or native shadow filtering.


Validated source freeze: `133712d7d6f7e8f5bd025878e10e49ef1f3b853e` (no push).
The 144 original-source statement cases and **3,360** fixed-camera original-DDS samples
pass: maximum **2.373 / 255** sRGB error, per-fixture means **.074-.504**. The .25-window
road, plant and coral checks have max errors .786, 1.724 and 1.202 respectively. A wrong
material-factored glow model is retained as a numerical discriminator, not fitted away.

Exact pixel-equality controls pass for both mask flags (hammer ALPHAGLOW, coral VERTALPHA)
with/without depth+normal+motion prepasses, bindless/ordinary prepasses, masked visibility
dissolve with/without prepasses, and token-zero masked prepasses. Existing vertex RGB,
point-light specular .5 and waterfall time 3600.25 captures remain pixel-identical to the
pre-change executable. Original bow/hammer mesh captures with the final zero token are
also pixel-identical to their immutable pre-change binary captures. The side-view original
bow pair (same binary, disabled token versus authored glow) changes **16,261** pixels;
full-UV bow/road/hammer diagnostic before/after pairs change 105,268 / 222,480 / 216,702.
The imported library was initially only embedded; invalid unresolved-import captures were
rejected, overwritten and replaced with clean captures after permanent shader-library loading.
No retained capture log contains a shader error or unresolved import.

Observed aa-dev FPS on original geometry, Vulkan/RX 9070 XT, 960x720 dev executables:
bow **190.7-247.8 before**, **254.9-324.7 after**; hammer **214.2-477.0 before**,
**263.3-293.9 after**. The clearer bow side-view control observes 286.0-309.6 with token
zero and 244.1-302.9 with authored glow. Other sessions/builds share the machine, so these
are observations, not evidence of a speed improvement/regression. Actual screenshots,
including both mask fixtures, were inspected. The self-contained `comparison.html` preserves
the side-view comparison; per-capture commands, binary/image hashes and FPS remain in JSON.

Checks: `cargo check -p aa-cry --examples` passes; final `cargo test -p aa-cry --lib --tests`
passes **94 unit + 7 loader + 2 quick pak** tests, with three exhaustive tests ignored;
the five doctests pass on retry (the first full run hit a missing shared `.rlib` extern).
The single strict `cargo clippy -p aa-cry --all-targets -- -D warnings` attempt fails on
existing aa-dev `headless.rs:68` unnecessary ownership and broad aa-cry lint diagnostics
(50 library errors, including old enum/type complexity and constant chunk iteration).
No unrelated lint cleanup or second Clippy run was performed. No dependencies, manifests,
lock file, client/world sources or other owner paths were changed. All capture processes
exited; copied executables and scratch were removed while source/PNG/numeric evidence remains.

## Private renderer mechanism probe (historical 507b6546, 2026-10-24)

`cargo build -p aa-cry --example native_pass_probe --locked` builds the isolated
[native_pass_probe](../crates/aa-cry/examples/native_pass_probe/README.md). It uses
the standard aa-dev screenshot/exit harness and explicitly supplied rigid coverage
controls. **This is renderer mechanism proof, not native Illum material completion.**
Production aa-cry sources/shaders, public APIs, manifests, dependencies and lockfile
remain unchanged, including raw `Texture.attributes` preservation at `0bd4e470`.

The executed Bevy 0.20 hooks prove a trailing record command after actual DrawMesh,
Skip short-circuiting despite `Draw::draw()==Ok`, current-view/current-frame Z
tickets, unavailable paired-pipeline withholding, initial composed clipping,
post-Z Equal with no depth writes, automatic indirect three-instance batching
partitioned by paired identities, and separate motion with Load/no depth writes.
Depth-only, normal-only, motion-only, explicit no-Z+motion, multiple views/targets,
frame resets, stock early/late culling and independent explicitly eligible Blend/Add
motion are exercised. A moving stock backing proves that .5 discard preserves real
prior velocities and does not leak an occluded producer. Actual attachment readbacks
prove motion preserves depth, normals and forward color. Stock StandardMaterial and
actual zero-token IllumMaterial remain controls; Shadow registration is retained.

All **5,496 assertions across 15 clean GPU runs pass**. Bindless/ordinary attachment
bytes match exactly. Directional/point shadow screenshots match stock-renderer
controls exactly, with shadows-off discriminators changing **5,788 / 11,006** pixels.
Unmarked views retain the stock prepass functions. The earlier proposal's arbitrary
`.in_set(stock_function.into_system_set())` replacement panics on 0.20; this example
replaces public schedule graph nodes before initialization to retain their original
edges. Persistent bins also require explicit dirty/requeue notification when paired
pipeline identities change. Both findings are implemented and executed here.

At 640x480, shared RX 9070 XT/Vulkan: default bindless **235.1–239.0 FPS**, ordinary
**251.3–282.5**; default pair planning **19.44 µs/frame** for two supplied views.
Recorded Z spans average **22.14–22.29 µs CPU / 21.21–21.27 µs GPU** and motion
**19.38–20.18 µs CPU / 19.84–19.85 µs GPU**. These are small-probe observations,
not a world-scale cost or speed comparison. The README gives all control FPS ranges,
exact reproduction, measured limits and the proposed production file/API/shader
contract. It identifies the unresolved per-instance GPU input channel and invariant
skin/morph/LOD/MSAA integration; no native caller defaults are invented.

Evidence is retained under `data/extract/aa-cry/native-pass-probe-20261024/verified/`
with commands, executable/source/PNG/attachment hashes, shader logs, timestamps,
CSV submission records and lossless raw-readback archives. Executable SHA256:
`9d48813466402e90e8e50ff99836a9ee3225780afb6e3102e0afbf397be9ae3d`.
Both owned executable copies were removed; no foreign process was stopped. The crate
check and example build pass. One Clippy run completed with the existing 50 library
warnings and 15 probe suggestions; the latter were addressed without rerunning
Clippy or changing baseline sources. **1,578 frozen input/production files compare
byte-identically**, including all accepted routing/scheduling/opacity evidence.

## Renderer probe review-01 fix (2026-10-24)

Independent review found that a stock base unavailable during planning could
become ready before draw and enter the marked-view original-pipeline fallback.
That unsplit pipeline could write motion early and replay depth writes. Marked
stock draws now Skip until their planned split variant exists and is ready in
each stage. The unmarked-view original fallback remains intact. Motion binds its
depth attachment read-only, adding wgpu validation against a depth-writing replay.

The private `readiness.rs` control introduces a fresh real stock pipeline ID,
asserts unavailable-at-plan, then finishes actual compilation after queue/before
draw. Both binding modes capture the first availability frame with no split plan,
selection or submission, and a following frame whose retained queue uses the
split Z ID and whose Z/motion DrawMesh submissions execute. Before-motion retains
the entire supplied sentinel; deferred stock motion stays untouched, then resumes
with the planned pipeline. Its ordinary stock main draw may provide depth during
deferral, allowed only within that control's exact footprint by validation.

Bounded no-attachment controls also prove that ViewPrepassTextures and both camera
prepass phases are actually absent, stock main depth/color remain present, and
the lit/shadow screenshot equals stock rendering. This does not establish native
camera policy. EarlyPostProcess ordering, controlled late disocclusion,
deformation/binding-generation/MSAA and actual positive-token Illum shadow
coverage remain production prerequisites.

All **8,105 assertions across 19 final-source GPU runs pass**, including **388
logical attachment hashes**. The prior 15 configurations were rerun for the shared
dispatcher/attachment change. New evidence and the detailed review response are in
the [probe README](../crates/aa-cry/examples/native_pass_probe/README.md) and
`data/extract/aa-cry/native-pass-probe-review01-fix-20261024/verified/`.
Executable SHA256:
`005a93a43a42acea1bad3f57748f3f88bf3cba80d1d70eaec727c4441b7138cd`.
All **538** historical proof files and **1,578** earlier frozen production/input
files remain byte-identical. Capture defaults use a new directory; historical
validation is read-only. Focused example check/build pass. No additional Clippy,
production edit, manifest/lock/dependency change, push or foreign-process action
occurred. Owned executable copies are removed. Default final-run FPS was
**254.0-291.9**; planning **20.36 us/frame**, Z **13.31-15.18 us GPU**, motion
**12.22-14.01 us GPU**, with no performance comparison inferred.
