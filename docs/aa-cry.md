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
| `scene` | yes | `CryPlugin`, `CryAssets` (cached conversion), `spawn_model*`, material mapping |

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

* `ModelOptions { material, lod, skeleton: Option<SkeletonBinding>, cry_space, keep_cpu_mesh }`.
  `skeleton` binds a skin (`.chr` body part) to existing joint entities of a base skeleton
  (bones matched by controller id, then name); `cry_space` keeps the root at identity (for
  parents that are already in CryEngine space); `lod` picks `*_lodN` files / `$lodN` nodes.
* Spawned hierarchy: root (`CryModel`, `Transform = CRY_TO_BEVY`) -> per file node (`CryNode`,
  node transform) -> per drawn subset (`Mesh3d`, `MeshMaterial3d<StandardMaterial>`, `Aabb`).
  Skinned models: joint entities (`CryJoint`, local bind pose) under the root, meshes under
  the root with `SkinnedMesh`; the root has `CrySkeleton { skeleton, joints }`.
  `spawn_model_full` / `spawn_loaded_full` also return the joint entities.
* `CryAssets::texture(path, srgb)`, `::mtl(path)`, `::material(mtl_path, sub)` give cached
  handles. Textures whose path moved are found by file name (about 1% of material references).
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

## Shader -> StandardMaterial mapping (`scene::to_standard_material`)

Common: `base_color` = `Diffuse` (linear) x diffuse texture (sRGB); `normal_map_texture` =
`Normalmap` (or a `_ddn` `Bumpmap`), linear, no Y flip; `perceptual_roughness` from
`Shininess` (`(2/(n+2))^0.25`); `reflectance` = luma(`Specular`) x `SpecularLevel` / 2;
`emissive` = `Emissive`; `MtlFlags & 2` -> double sided, no culling; `Opacity < 1` -> blend;
`AlphaTest > 0` -> mask; additive flag -> add; `TexMod` of the diffuse slot -> `uv_transform`.

| shader | mapping |
|---|---|
| `Illum`, `Cloth`, `Eye`, `Frozenlayerwip`, `Cloaklayer`, `Terrain.Layer`, ... | lit opaque (+ the common rules) |
| `Vegetation`, `VegetationLeaves`, `Grass` | double sided, alpha mask (`AlphaTest`, default 0.5) |
| `Hair` | double sided, alpha mask; colour = `HairColor` public param (textures are greyscale) |
| `HumanSkin` / `Humanskin` | neutral albedo `SKIN_ALBEDO` (0.22) x texture; per-file tints differ between face and body and are replaced by customisation |
| `Metal` | metallic 0.6 |
| `Glass`, `Liquid`, `Ice` | blend, alpha min(opacity, 0.6), roughness 0.1 |
| `Water*`, `Ocean*` | blend, dark blue-green, roughness 0.05 |
| particles, `FX_*`, `Light*`, beams, `Sky*`, clouds, `SpecialEffects`, `Hologram` | unlit, additive, double sided |
| `NoDraw`/`Nodraw` (or flag 0x400), `Cut` | not drawn (`Cut` is a coarse hull in costumes) |

Not mapped (no StandardMaterial equivalent without extra Bevy features): specular maps
(`pbr_specular_textures` is not enabled), environment cubemaps (scene-level), sub-surface,
detail maps, vertex bending.

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
```

`--attach` binds a part to the previous `--model`'s skeleton (a character = base `.chr` +
`--attach` face, hair and costume parts). `--focus` is in Bevy space (metres, Y up).
Screenshots: `shots/aa-cry/` (`inn.png`, `castle.png`, `tree.png`, `elf_login001.png`,
`rock_ati2.png`, `legacy_helmet.png`).

## Known gaps

* Specular maps, environment cubemaps, sub-surface, detail and blend layers are not mapped
  (see above); vegetation bending and wind are not applied.
* Character assembly (which parts hide which body pieces, CDF/DB customisation colours for
  `HumanSkin` / `Hair`) belongs to aa-anim. Costume parts replace body regions, so drawing the
  nude body under a costume shows through; some costume "skin" subsets use UVs that do not
  match `el_f_nude_df`, and use the costume's own skin texture in the game.
* Legacy helmets have no `.mtl` and draw with the default material; the v orientation of
  legacy UVs is unverified for lack of a textured sample.
* Some buildings are open shells (e.g. the inner sides of `castle_nuia_001`'s towers): the
  winding matches the normals everywhere, so this is content and not a mirrored transform.
* Bevy culls by winding only, ignoring mirrored transforms. `load_model` flips the triangles of
  static nodes whose `scene::render_transform` has a negative determinant (1 node in 2,989
  sampled files, e.g. `beach_rock_med_e.cgf`; meshes are never shared between nodes).
  Placement code that mirrors whole instances (negative scale) must do the same, with
  `scene::subset_mesh_with(.., flip_winding: true)`.
