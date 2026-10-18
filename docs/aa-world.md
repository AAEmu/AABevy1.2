# aa-world: ArcheAge 1.2 worlds in Bevy

`crates/aa-world` renders the two worlds of the 1.2 client from `game_pak`: `main_world` (the game) and `login2` (the 3D scene behind
character select / create). Terrain, static objects, vegetation, water, sky / sun / fog from the time-of-day data, all streamed around a
focus (the camera) on background tasks.

```text
cargo run -p aa-world --example fly -- --world main_world --spawn nuian --shot shots/aa-world/nuian_start.png
cargo run -p aa-world --example fly -- --world main_world --pos 15579.5,15385.9,128 --yaw 201 --shot shots/aa-world/nuian_start.png
cargo run -p aa-world --example fly -- --world login2      # camera of cinemas/01_world_select_e1.xml
cargo test -p aa-world
```

Status: terrain (cover colour + detail layers with normal maps), brushes, vegetation, water (reflective, depth tinted, rippled), ocean, sky dome
with the zone's sky box, sun / ambient / height fog, an HDR post pass (film curve, grading, bloom) and streaming work and are committed. Gaps are
listed at the end.

## 1. Coordinates

* Game world: CryEngine, right handed, **Z up**, metres, X east, Y north. World position = `cell * 1024 + cell-local`. A cell is a 1024 m
  square in folder `cells/XXX_YYY` (`XXX = floor(x / 1024)`, X first). A cell has 16 x 16 sectors of 64 m; the heightmap sample is 2 m
  (513 x 513 samples per cell, the last row / column is shared with the neighbour).
* Bevy: Y up. The one transform used everywhere is the -90 degrees rotation about X, [`coords::CRY_TO_BEVY`]:

  ```text
  bevy = (x, z, -y)         bevy -> cry: (x, -z, y)
  ```

  Cry east is bevy +X, cry north is bevy -Z (the camera's forward), cry up is bevy +Y. A rotation keeps handedness, so meshes keep their
  winding. Vertices of object parts stay in cry space and carry `R4 * M_instance * node_world` as entity transform (negative
  determinants, i.e. mirrored brushes, are handled by Bevy's winding flip).
* Compass heading for cameras: 0 = north, 90 = east. CryEngine angles `(pitch, roll, yaw)` have yaw counter-clockwise from north, so
  `compass = -yaw`; a yaw of 158.74 degrees looks towards (-0.363, -0.932).
* **Verified against the real client** (`reference/ingame/spawn/camera.json`): the player entity stands at `(4314.041, 4118.121)` and its
  `terrain_height` is `126.245697`. The server spawn is `(15578.042, 15382.122)`. The difference is exactly `11264 = 11 x 1024` on
  both axes (the client renders the seamless world relative to a moving origin). Our API uses **game / server coordinates**; cell files are
  named by game cells: `CellHeights` of `015_015` sampled at the cell-local `(218.04, 22.12)` returns `126.2457`, the client's value to
  six digits. A client-local position becomes ours by adding the shift (`fly --camera-json ... --shift 11264,11264,0`).
* The default third person camera is 4 m behind the player and 1.73 m above its feet (`fly --spawn <race>`).

## 2. Data formats

Everything is little endian. Paths are relative to `worlds/<world>/`.

### world.g, world.xml

`world.g` (text): `name`, `cellX`, `cellY` (35 x 38 cells for main_world, 2 x 2 for login2), `oceanLevel` (100 for main_world, 0 for login2),
`zones ( name, id )`, `cells ( x, y, zones )`. `world.xml` (4.6 MB) lists the 64 m sectors each zone owns; `World::zone_at(pos)` answers
which zone (and thus which `zone/<id>/time_of_day.xml`) a position is in. It is parsed on a thread at startup.

### terrain/heightmap.dat

Header (version, flags, chunk size, 4096 units, unit 2 m, sector 64 m, `zRatio`, `oceanLevel`), then a depth first quadtree of 341 nodes
(1 + 4 + 16 + 64 + 256). Only the 256 leaves (one per 64 m sector) hold data: `size x size` u16 samples, `size` 33 (sometimes 2, 3, 5, 9
or 17 for flat sectors: 6051 such leaves exist, resampled bilinearly). A sample is `height << 4 | surfaceType`; the height is
`offset + (raw & 0xFFF0) * range` per node (node versions < 7, all of 1.2). Node boxes use the engine's 8 x 8 cell window coordinates, so
positions are taken relative to the root box minimum. `CellHeights` has `heights` (`iy * 513 + ix`), `surface` (the 4 bit surface type per
sample, an index into the `SurfaceTypes` of `leveldata.xml`, see "client/leveldata.xml" below), `ocean_level`, `min_z`, `max_z`, bilinear
`sample`, `normal`.

### terrain/cover.ctc

The baked terrain colour map: a quadtree of 128 x 128 px DXT tiles, one tile set per level. Header `"CRY\0"`, version, layer table (DXT1 = 22
or DXT5 = 24, 8 or 16 bytes per 4 x 4 block), then `nodeCount` u16 tile indices (0xFFFF = none) in depth first order (a node with a tile
is followed by its four children `x0y0 x1y0 x0y1 x1y1`, so `nodeCount = 1 + 4 x tiles`), then the tiles. The depth varies per cell: 5 levels
(0.5 m/px at the finest) for most, 2 or 3 for ocean cells, and coastal cells only have fine tiles on the land. Level `n` is mip
`maxLevel - n` of the cell, so `Cover::atlas` just rearranges the blocks into one BC texture with a complete mip chain and never decodes a
pixel; a missing fine tile is re-encoded from its parent (BC1 bilinear upsample + encoder). Image rows run along world **X**, columns along
**Y**: sample with `uv = (y / 1024, x / 1024)`. All 1205 cells have exactly one layer, BC1, `sunShadowIntensity` 1.

**The texels are not colours.** The engine reads them with `tex2DTerrain` (`shaders/hwscripts/cryfx/common.cfi`, the shader sources are in
the pak): after the bilinear fetch, `r' = (r + 0.001012) * 31 / 30`, `g' = g`, `b' = 1 - r' - g'` is a chromaticity (`r + g + b = 1`), the
blue channel `B` of the texel the brightness, and `colour = saturate(chroma * 3 * B)` in gamma space (`pow(., 2.2)` for lighting). Read as
RGB the picture is a yellow green with blue seas and no grey rock (this crate did that first, with `terrain_tint` as a fudge). The image
must therefore be a non-sRGB texture and the shader decodes it (`terrain_layers.wesl`).

### client/leveldata.xml and the terrain layer materials

`leveldata.xml` is Sandbox's level data. `<SurfaceTypes>` lists 21 `<SurfaceType Name, DetailMaterial, DetailScaleX, DetailScaleY, ProjAxis="Z",
Id>` (optional `<VegetationGroup Id>` children). The 4 bit surface nibble of `heightmap.dat` is the **position** in this list (only the first 16
can be referenced; unused slots repeat `__default__` = `beach_white_sand_dry`), `Id` is the global surface type number (up to about 900).
Checked on the spawn: cell `015_015` nibble 4 (`tr_field_dirt_001c`) is exactly the flat dirt square of the spawn (`terrain_height` 126.2),
nibble 1 / 3 (rock) have mean slopes of 37 / 45 degrees. Ocean cells are all nibble 0 (`tr_snow_field_ground_708a`, the seabed). 306 different
`DetailMaterial`s are used by the 1205 cells (`materials/aa_terrain/{field,canyon,mix,wasteland,wet,...}`, a few `materials/terrain/`), all
with the shader `Terrain.Layer`.

A layer `.mtl` is either one material (only the ground, z, projection exists) or **three sub-materials x, y, z** (84 of the 304: rock,
cliffs; the same textures with a different `TexMod` tiling, e.g. 0.05 for x / y and 0.14 for z). Textures: `Diffuse` (BC1, 512 px, some 1024),
`Bumpmap` or `Normalmap` (`_ddn`: ATI2 = BC5, 512 px, some 1024 or 256; 22 are DXT1 with plain RGB = xyz, same orientation as
the BC5 ones: checked by the correlation with the diffuse luma gradient), `Specular` (`_sp`, BC1 256 px), `Detail` (a second normal map, mostly
BC5 256 or 512 px). The DDS files stop at the 4 x 4 mip. `PublicParams`: `DetailTextureStrength`, `DetailBumpScale`, `DetailBumpTillingU / V`;
`GenMask` bit `0x4` bump map, `0x8000` detail bump (`shaders/terrain.ext`). The layer's uv is `world * DetailScale * TexMod tile`.

What the engine does with them (`terrain.cfx`, `fraglib.cfi`; verified on the spawn capture): the base pass draws the decoded cover colour; one
alpha blended pass per surface type follows, with the weight `sqrt(w * axis weight * (1 - (d / 1024)^4))` where `w` is 1 at a vertex of that
type, 0 at one of another type (linear over the triangle), and the axis weight selects the sub-material by the dominant axis of the normal
(a single material is only drawn on the z axis, i.e. below about 45 degrees; steeper ground shows the cover colour alone). The layer colour is
`pow(saturate(cover + (diffuse - 0.5) * DetailTextureStrength), 2.2)`, lit with the layer's normal map (ATI2: the first block is Y, the second X;
`xy` = `2 * texel - 1`, z reconstructed, tangent = direction of growing u, bitangent = direction of growing v, i.e. world x / y for the z axis,
y / z for x, x / z for y) plus the `Detail` map sampled at `DetailBumpTilling x uv` and added to the xy with `DetailBumpScale`. Checked: the
decoded normal correlates 0.5 with the gradient of the diffuse luma (dark = low) for this swap and 0 for the other one.

`materials.dat` / `material_list.dat` of a cell list object materials (3 entries in `015_015`: `lake_tank`, `cliff_rock_a_med`, ...), not terrain.

### client/brush.dat, statobjs.dat, material_list.dat

`statobjs.dat`: `u32 n`, `n x char[256]` model paths (the brush "type" table). `material_list.dat`: `u32 n`, `n x { u32, char[256] }`.
`brush.dat`: `u32 version (1)`, `u32 offset[256]`, `u32 size[256]`, then 128 byte records per 64 m sector (`sx * 16 + sy`):

```text
f32 bbox[6] @0, f32 maxViewDist @28, u32 rndFlags @32, u8 viewDistRatio, lodRatio, internalFlags @36,
f32 m[12] @67 (row major 3 x 4: rotation*scale | translation), i32 materialId @115, i32 editorId @119, i32 statObjId @123, u8 layers @127
```

Matrix and box are sector-local in `brush.dat`, cell-local in `object.dat`. login2 has no `brush.dat`; its brushes only exist in `object.dat`.

`materialId` of a `brush.dat` record indexes the material table of the cell's own `object.dat` (the octree stores the same brushes with
the same indices), **not** `material_list.dat`, which is a different and longer list (749 entries against 149 in cell 10 / 14, in another order for 1808 of 2110
brushes). `placement.rs` therefore takes the materials from `object.dat` and uses `material_list.dat` only for a cell without one. With the wrong
table the Memoria vines (`gw_memo_etc_f02/f03/f04`) were drawn with `remain_divided_road`, whose physics `proxy` sub-material
(`Shader=Illum GenMask 16777216`, pale green, opacity 0.3) is not drawn by the engine but showed up as flat pale green sheets, and the canal
tiles got the stair material's untextured proxy slab (overexposed white under the sun). Test: `brush_materials_come_from_the_object_dat_table`.

### client/visareas.dat

Interior brushes are additional placement, not another exterior `object.dat` export.
`placement/visareas.rs` reads their independent model/material tables and appends the
supported objects to `load_cell_objects`, remapping nonnegative indices by the existing table
lengths. Negative native indices remain null/fallback; cell origin is applied once
by the ordinary render/collision path. This restores rooms, walls, floors and props
that were absent when only the exterior files were read.

Exact 1.2 evidence is the original `data/extract/aa-cry/ida/cry3dengine.dll` /
`cry3d.i64`, audited through the disposable `night/moon-native/engine.i64` copy:
`3017A020` (`CVisAreaManager::SetCompiledData`) validates version 4 and total compiled
size, then loads area, portal and occlusion records in that order through `30170860`.
`301FC140` reads each 33-byte version-2 octree node and `301FA0B0` walks its typed
object block. `301F8420` advances 128 bytes for a brush; instructions at `301F84D2`
and `301F84F3` read model `+0x7B` and material `+0x73` into separate tables through
`301F2B00`/`301F2B40` (negative index returns null). `301F2CA0` copies matrix `+67`,
bounds and view/flags, adding `(cell_x*1024,cell_y*1024,0)` to translation and bounds.
No deduplication occurs in that native construction path.

```text
u32 models; models * { u32, char path[256] }
u32 materials; materials * { u32, char path[256] }
u32 version=4, compiledBytesIncludingThis20ByteHeader, areas, portals, occlusionAreas
for each record:
  264-byte header: u32 version=2|3; bbox[6]@4; bbox[6]@28;
                   char name[32]@52; u32 octreeBytes@84; connections/flags/area properties
  if version==3: 12 extra bytes
  u32 polygonPointCount; polygonPointCount * float3
  octreeBytes bytes: version-2 octree when length>4
```

All table/count/block/tree reads are bounded by their container; malformed records
fail atomically. A 64-level defensive stack limit rejects pathological mask chains.
The supported slice imports brushes, vegetation, decals, roads and water through the
existing renderers, plus supported compiled voxel base/projected diffuse meshes and cell collision.
Voxel/distant-cloud records have exact boundaries; unknown-sized
kinds still end their object block while retaining child trees. It does **not** render
voxel projected normal/specular detail/atlases or distant clouds, traverse portals for occlusion, apply area sky/light/fog
flags or implement visarea light confinement.
Repeated instances inside an authored area are preserved. Exterior `brush.dat` versus
`object.dat` selection is unchanged, so their duplicate export is still selected once.

Native fixture `instance_library` cell (0,4): 4,242,562 bytes, 116 models, 75 materials,
152 areas, no portals/occlusion areas, 4,402 octree nodes, **30,286 type-1 records** and
no unknown blocks. There are 153 byte-identical repeats within their own area; all
remain present. With the one exterior book, the cell contains **30,287 brushes**.
The missing `library_floor_a.cgf` / `library3_floor_a` in `visarea_293_pidesa_7` has
translation `(896,128,255.9970245)`, +90-degree Z rotation, and floor top `256.55066`.
The outdoor cell (18,7) contributes another 234 interior brushes; (10,14) and login2
(0,1) have empty valid containers. Pak-backed regressions check those counts,
transforms/bounds, repeated-instance count and every merged model/material mapping.

Work stays in the existing two placement jobs and 64 m sector bins, with the normal
48-model queue and main-thread spawn/finish budgets. No world-wide visarea scan or
per-frame parsing is added. The library view settles at 20 nearby sectors / 1,981
parts, zero pending/failed models; distant sectors are not spawned. Its full-cell
collision build sees 28,398 colliding brushes, 663,798 proxy triangles and 30,381,788
bytes (29.0 MiB), built in 291 ms. The fixture's `--collision-check` ray from global
Cry `(872,4200,258)` hits the actual floor at `256.55066`, not terrain, then releases
the collision cell. Physics retains the existing per-cell build policy and shared
model-proxy cache; sector-level collision streaming remains future work.

Validation: **144 aa-world library tests pass**, nine pre-existing ignored, and all
examples check. `projected_lights --world` now defaults to a clear view at global Cry
`(872,4211,262)`, looking at `(872,4200,256.55066)`; `--pos`/`--look` allow explicit
coordinates. Inspected 1920x1080 `shots/aa-world/lights/library_visarea_{before,on,off,unload}.png`
show the previously missing room and the original stained-glass projector on its
authored floor. Same-view light on/off changes 635,914 pixels, including 293,663 in
the floor rectangle `(430,440)..(1450,1000)`; maximum RGB increase there is (22,87,69).
Authored light parameters and the existing fixture exposure/material conversion are unchanged. On logs 210-220 FPS,
off 198-253 FPS (shared GPU, separate runs). Actual camera movement releases all
sectors/parts, authored lights, RGB children, cookies/pending work and collision
cells; the unload frame has zero object entities. All runs exit zero without GPU
errors and close their own windows. There is no matching original-client library
capture under `reference/`, so absolute native appearance remains uncalibrated;
projector falloff remains the explicit approximation described under Local lights.

**Mixed interior records and native boundaries.** A bounded read-only survey of 13
cells (28,014,280 bytes read through `aa-pak`, metadata plus one targeted CGF sample retained) covers the six
largest main-world interior exports, main-world (18,7)/(10,14)/(20,10)/(15,15)/(9,14),
login2 (0,1) and library (0,4). Its complete trees contain 36,076 brushes, 40 vegetation
records, 98 decals, seven water volumes and **77 voxel records**, with no unknown-sized
blocks after the following native boundaries were recovered:

* `301F5E30` is `_LoadVoxelObject`, not a fog/light record. It requires voxel version 5
  at +39 and consumes the **198,775-byte** `SVoxelObjectChunkVer5` header. Nested
  `SVoxelChunkVer5` begins at +39: flags +28 (overall +67), compiled-mesh-present bit
  `0x20`, volume `u16[32768]` +32, surface names `char[32][64]` +65568, colours +67616.
  The object matrix is at **198727**, 48 bytes. With compiled data present, `3019D570`
  reads `u32 blockCount`, then repeated `u32 byteCount; bytes[byteCount]`;
  `30194B70` confirms total byte length `4 + sum(4+byteCount)`. The parser retains
  only the first compiled LOD for collision; later LODs are consumed without copies.
* `301F6AB0` is `_LoadWaterVolume`: **119 fixed bytes**, then two float3 arrays whose
  counts are at +103/+115. Even its disabled branch advances both counts. The old
  whole-object-block assumption lost later records; the existing water parser now
  receives exactly that bounded record.
* `301F7680` is `_LoadDistanceCloud` and advances **79 bytes** before its enabled/quality
  checks. Rendering that object type remains unsupported.

The same checked object decoder serves exterior `object.dat` and interior trees;
known records cannot read into children. Count multiplication, voxel blob sizes,
water arrays and stack depth are bounded. Original six-cell regressions assert:

| main-world cell | interior brushes | vegetation | decals | water |
|---|---:|---:|---:|---:|
| 17,8 | 2157 | 28 | 0 | 3 |
| 8,13 | 588 | 2 | 22 | 2 |
| 12,15 | 625 | 9 | 72 | 0 |
| 21,12 | 289 | 0 | 0 | 2 |
| 22,25 | 1311 | 0 | 0 | 0 |
| 20,8 | 115 | 1 | 4 | 0 |

Exact skips recover **69 additional brushes** beyond the initial brush-only slice.
Interior decals/roads/water remap their own material table; vegetation keeps the
native world vegetation-group index. Merge regressions compare actual source paths,
projection/depth/basis/sort fields, positions, groups and water rings after remapping.
No authoring transform or light value is changed. Work/lifetime remain the existing
two placement jobs, sector/model budgets, decal receiver limits and cell unload.
CPU fixtures prepare 18/45/4 supported decal meshes in (8,13)/(12,15)/(20,8), all
finite and within the existing per-mesh triangle bound. Missing voxel receivers
explain why not every authored interior decal can currently render.

Voxel projected detail/atlas rendering is a concrete remaining gap:
the first surveyed compiled blob is a length-prefixed zlib stream containing a valid
`CryTek` 0x745 CGF (2,901 vertices / 5,642 triangles). Native `301A0790` calls
`301A0D30` for compressed memory blocks, then `30192D40`/`3019FFE0` load that CGF.
The original `voxterrain.cfx` samples generated diffuse/normal atlases with per-atlas
range restoration. The CGF also contains `COLORS2` and `VERT_MATS` streams, which
`aa-cry::Model` currently does not retain. Native palette-to-atlas generation,
surface blending and render flags need their own supported path. The supported base
pass below instead uses the exact original packed vertex colours and visible CGF,
without guessed UVs or a generic rock material. Read-only audit outputs are under ignored `lights/visarea-*` and
`lights/voxel-*`; the original shader is extracted under ignored `voxel/`.

**Native compiled voxel collision.** `placement/voxels.rs` retains the cell-local
matrix, 32 surface names and first compiled memory block; `collide/voxels.rs` adds
its serialized triangles to the existing structure soup/grid. Native `301A0790`
(`CVoxelMesh::MakeRenderMeshsFromMemBlocks`) physicalizes the first LOD through
`3019BFA0`, using `301A0D30` decompression when the memory block does not start with
`CryTek`. The supported path accepts raw CGF or the surveyed zlib stream prefixed by
its decoded length, one identity `Merged` static node, and one serialized triangle proxy
with explicit per-face palette IDs. Native bit `0x80` is not a physics-disable bit:
the loader's vtable +344 call reaches `30194220`, which sets byte +448; the audited
physicalization path does not gate on that byte. No such guessed exclusion is used.

The first original (20,8) voxel's CGF has 2,901 vertices / 5,642 triangles. Every
serialized proxy triangle's material matches native `3019BFA0`'s majority rule on
the **first byte of the four-byte `VERT_MATS` stream (type 11)**: choose vertex 1
when vertices 1 and 2 agree and differ from vertex 0, otherwise vertex 0. The proxy
reorders triangle indices; it is not replaced with the render mesh. Palette counts
are 5,262 / 362 / 18 for IDs 0 / 1 / 2. `COLORS2.r` is zero in this sample and is
not the native physics material selector.

Native `30196930` resolves local palette names case-insensitively against the cell's
32 surface entries, falling back to its first entry with a material, otherwise entry
zero. `300C66F0` maps authored capital `ProjAxis` X/Y/Z to 1/2/3. `301227A0` assigns
the first six material children projection IDs by X/XP, Y/YP, Z/ZP, XN, YN, ZN name;
`300C9120` selects the matching child (or root when fewer than three children).
Physicalization passes the entry's default projection axis, not the triangle normal.
Thus the sample rock's authored Z projection resolves to `mat_dirt` even on its wall;
no material or normal-based tuning is applied. Empty local slots remain absent;
unknown physical surface names use the existing default surface table.

Cell origin is applied once, then the serialized matrix; reflected transforms flip
winding. Proxy triangles participate in ground, capsule/camera and structure rays
through the same cell/grid lifecycle as brushes. Parsing retains at most 128 slots
and 16 MiB of first-LOD compressed bytes per placement file; exterior/interior data
can each contribute that bound. Individual decoded CGFs are limited to 16 MiB / 64
chunks / 65,536 vertices per mesh/stream/proxy / one million mesh indices / 32 subsets.
Collision attempts at most 64 MiB decoded bytes and one million contributed voxel triangles
per cell, one slot at a time on the existing shared cell worker. A conservative XY
grid-reference estimate caps added voxel index storage at 8,388,608 IDs (32 MiB);
an entire slot is rejected before insertion if it exceeds the remaining budget.
Exact decoded size, zlib checksum/end/trailing bytes, proxy indices/material IDs and finite transforms
are checked. There are no added threads, per-frame voxel scans or world-wide startup
allocations. `CellStats::{voxels,voxels_colliding,voxel_triangles,voxels_unsupported}`
make skipped variants visible.

Runtime volume meshing, missing/foreign-index proxies without explicit material IDs,
multiple/deformed nodes, other compression formats and voxel projected normal/specular detail/atlas rendering
remain unsupported. Collision alone restores physical cave floors/walls;
the later base/diffuse slices make supported surfaces visible, while decal receivers
remain separate work. Native point/area evidence also
remains separate: `3017B3E0` uses min-inclusive/max-exclusive AABB tests followed by
`3017B2F0`'s XY crossing test, while `3017BEF0` conditionally multiplies area ambient
by native sky colour or the renderer HDR multiplier. Bevy light `RenderLayers`
filter camera clusters/shadow passes, not each receiver's polygon/portal membership;
no camera-global sky, fog or ambient approximation has been substituted.

Collision QA: **154 library tests pass**, nine existing ignored, and all examples
check. Compressed-length/checksum/truncation/trailing-byte regressions, malformed
CGF/proxy count/transform cases (including an extreme-scale grid-budget regression),
actual CGF material-majority verification and authored projection-name/fallback tests
pass. Original cell (20,8) contributes seven supported
slots / **45,413 voxel triangles**, no unsupported slots. Three native proxy triangle
centroid/normal probes hit their original floors/wall within 0.3 mm; the floor at
Cry `(21354.6667,9055.3333,454.91738)` also resolves through `ground_below` and reports
`mat_dirt`. The fixed cell origin is `(20480,8192,0)` and first object translation is
`(832,832,434.29898)`, with explicit tests against accidental double addition.

`voxel_collision` uses `aa_dev` and draws only the real first proxy, with explicitly
diagnostic colours/edges and gold hit markers. Inspected 1920x1080
`shots/aa-world/voxel/native_proxy.png` logs **279-396 FPS** in the final rerun
(earlier 382-437 FPS, shared GPU); this is a diagnostic mesh
fixture, not full-world rendering performance or a claim of original surface parity.
Its complete cell collision builds in 488/457 ms in the final two runs, 883,049
structure/prop triangles total and 38,549,190 bytes (vegetation stays instanced).
`--unload-check` moves the actual camera outside the authored world, follows it with
the existing release path, and asserts zero cells, pending jobs, triangles, cell bytes
and diagnostic entities; the prior floor structure ray becomes absent. Inspected
`native_proxy_unload.png` confirms that state, 431-455 FPS after unload. Both runs
exit zero and close their own windows. No matching native cave reference capture is
available under `reference/`; serialized geometry/numeric ray oracles are the native
comparison for this collision-only slice.

**Visible native voxel base pass.** A bounded follow-up trace identifies the active
default material contract, rather than assuming `voxterrain.cfx` is sufficient.
`3019A740` (`CVoxelObject::Render`) draws the base through `3019EB80`, then projected
surface layers through `3019EEC0`. Base getter `30127D50` returns `CTerrain +3560`;
constructor `300CEAE0` loads **`Terrain`** into that field. `3019FFE0` prepares the
separate projected meshes through `300DF410` / `300DFA60`, using the authored cell
surface indices and projection names. The compiled original sample has no UV stream;
the base pass does not need invented atlas UVs.

The original `terrain.cfx` `TerrainVS` writes `Color1 = (Color.r, Color.b, Normal.w,
Normal.w)` and selects vertex-colour RGB when `Color.g >= 0.5`. Exact renderer
`38206540` / `382065A0` preserve the first colour stream with D3D channel swizzling;
`382051A0` writes **`COLORS2` byte 2 to packed `Normal.w`**. `TerrainPS` then selects
that RGB directly, and the layer shader explicitly documents that voxel vertex
colour is **linear**, converting to gamma only for its detail blend. Therefore the
supported base colour is exactly `(COLORS.r, COLORS.b, COLORS2.b) /255`; byte 128
is included in the flag threshold. `COLORS.g &0x7f` is a global cell surface index,
not a green tint or the local `VERT_MATS` physics ID. The first original vertex's
streams `(21,140,14,255)` / `(0,0,11,0)` decode to linear `(21,14,11)/255`.

`PlacedVoxel::compiled_base_mesh` reads the already retained first LOD with the same
CGF size/count preflight as collision, one identity `Merged` static node, complete
finite normals and exact four-byte `COLORS2` records. The initial baked-only slice
rejected `g <128`; the source-backed cover follow-up below now retains its native
selector and draws the original cover texture.
The native CGF's visible indices/normals are used, not its reordered physics proxy.
`objects/voxels.rs` applies the authored matrix to positions and inverse-transpose
to normals, converts axes once, flips reflected winding and leaves cell origin on
the entity. Vertex colours remain linear in the GPU buffer. Meshes use the existing
Bevy PBR/fog/light/shadow path, without terrain-tint or brightness adjustments;
native Terrain BRDF, baked/dynamic terrain AO and projected normal/gloss detail
remain explicit differences.

The same two existing placement jobs prepare at most 32 voxel meshes /64 MiB decoded
attempt bytes /32 MiB vertex+index bytes per cell. Each slot is complete or skipped;
overflow and unsupported variants increment `ObjectStats::voxels_unsupported`.
`voxels` / `voxel_triangles` report visible meshes/triangles, and `VoxelSurface`
identifies their cell/source slot. GPU uploads happen when that placement result
arrives; visibility uses the existing brush-distance cap and authored maximum view
distance. There are no added threads, independent cell requests or per-frame rebuilds.
Cell unload explicitly despawns the entities and removes their owned mesh/material
assets. Materials and original cover tiles are shared within a cell, with no
persistent voxel material/texture cache.

Original (20,8) has seven supported base meshes / **45,963 visible CGF triangles**;
the independent raw CGF header probe reads index counts
`16926,15288,7674,23034,23079,21459,30429`. Its 45,413 colliding triangles are a
different native stream and remain unchanged. Unit tests verify the original packed
colour, `g=127` cover / `g=128` baked selection, linear GPU attribute values, transform
units, all seven visible meshes and the 32-slot bound. `aa-cry` is unchanged: raw
chunk access supplies `COLORS2`; a future general loader could add
`MeshData.colors2: Vec<[u8;4]>` from data-stream type 4 without changing the original
`colors` field or conflating it with `VERT_MATS`.

Base-pass QA: **157 library tests pass**, nine existing ignored, and all examples
check. `voxel_surfaces --colour-check` renders the first original decoded vertex
beside an independent numeric linear-colour material; both GPU samples are exactly
**`[81,66,59]` sRGB** (the expected display conversion of linear `(21,14,11)/255`).
Inspected 1920x1080 `shots/aa-world/voxel/native_base_colour.png` logs 371-434 FPS.
The threshold fixtures also include two real exterior slots from cell (21,8):
their `COLORS.g` ranges are **1-148 /1-20**, so both require the cover-projection
branch; these were rejected at the baked-only milestone and are supported by the
follow-up below. The seven interior (20,8) slots have ranges 139-145 and remain
supported, with unchanged geometry/physics.

Inspected `native_base_cave_on.png` / `native_base_cave_off.png` use the same original
floor camera at Cry `(21354,9055,456.7)` looking at `(21335,9044,458.5)`, hour 10.
On restores the cave floor/walls around the authored bridge, cart and torches; off
hides only the voxel base meshes, exposing floating props and outside sky. The
original floor collision remains `454.91745` in both. Final settled on logs 79-94 FPS
(first interval 104), off 73-97 FPS; animated particles and shared GPU work make
these separate runs unsuitable for a precise performance delta. No authored light,
environment/exposure, material palette or inherited calibration value is tuned.
`--unload-check` moves the actual camera away and verifies zero voxel entities,
cell collision/loading jobs, plus absence of **all seven owned mesh asset IDs and
the cell's material asset ID** from their asset collections. Inspected
`native_base_unload.png` confirms the empty world view, 178-234 FPS after release.
All four runs exit zero without GPU errors and close their own windows. There is
no matching native cave image under `reference/`; exact original geometry, packed
colour math and on/off receiver alignment are the verified comparison boundaries.

The initial base-pass slice left projected `Terrain.Layer` open. The follow-up below
adds its diffuse projection, native weights/fade and pass order; normal/gloss detail,
volumetric mesh generation, lower LOD selection and custom/animated voxel materials
remain unsupported. Portal/area
receiver lighting and native atlas generation remain open; the base-pass rendering
does not claim those features or complete original cave appearance.

**Native cover-projection follow-up.** `300C2870` / `300D70B0` find the smallest
terrain node containing the serialized voxel box in XY, using the original 0.01 m
containment tolerance and child order. Texture setup `300D9340` chooses an available
ancestor and supplies scale `(1 - 1/resolution) / nodeSize` plus half-texel offsets.
`300CDD50` reads that resolution from the original `cover.ctc` layer header, not from
a guessed atlas size. The supported near/full-quality path selects the original
128x128 BC1/BC3 tile, preserving its raw non-sRGB encoded values, clamped linear
filtering and native Y/X texture orientation. The selected world node is at least
64 m in this static compiled-terrain path; missing finer tiles use their available
ancestor. Dynamic texture-streaming/quality LOD selection remains open.

`compiled_base_mesh` now preserves linear RGB for all vertices and puts the exact
per-vertex `g >=128` baked flag in colour alpha. The opaque `VoxelMaterial` shader
uses it only as an interpolated selector, never as surface transparency. Original
`common.cfi::tex2DTerrain` decodes the filtered cover texel as
`r=(raw.r+.001012)*31/30`, `gamma=saturate((r,raw.g,1-r-raw.g)*3*raw.b)`;
`TerrainPS` raises it to 2.2 and blends with interpolated packed linear RGB. A CPU
choice of cover/baked per vertex would give a different result at mixed triangles
and is deliberately avoided. Source evidence is retained in ignored
`lights/voxel-cover-{mapping,bindings,file}.txt`.

Both original (21,8) boxes, XY `(832,0)..(896,64)`, select level 4 node `(13,0)` and
share one 8192-byte BC1 tile/material. UVs run from `.5/128` to `127.5/128`; midpoint,
axis/sign, crossing-box common-ancestor and invalid-box checks use actual original
data. Missing cover declines the entire mixed slot, without a flat fallback.
The existing two placement jobs retain the same decoded/mesh/count limits; UVs add
eight bytes per vertex. Cover images are bounded by those mesh attempts and deduplicated
by node within each cell. Unload explicitly removes their images as well as meshes
and materials. No additional per-frame parsing, geometry work or threads are added.

`voxel_cover` checks an original (21,8) tile's encoded first texel `(93,102,21)/255`
with selector 0, 0.5 and 1 against independent CPU decode/linear blend controls.
The GPU returns exactly **`[17,18,6]`, `[59,49,41]`, `[81,66,59]` sRGB**, identical
to the controls. Inspected `shots/aa-world/voxel/native_cover_numeric.png` logs
407-408 FPS. **159 library tests pass**, nine existing ignored.
All examples check. Inspected `native_cover_exterior_on.png` / `_off.png` use Cry
`(22310,8180,560)` looking at `(22368,8224,510)`, hour 10, with unchanged authored
lighting/material/environment. The on image restores the native low-detail voxel
faces over the terrain opening; the off image hides only voxel surfaces. The view
also streams the adjacent cell: four supported meshes /3590 triangles, two shared
materials/cover images, zero unsupported slots and zero pending/failed models when
settled. On logs 130-177 FPS, off 153-174 FPS; separate runs and shared GPU work
preclude a precise cost comparison. `native_cover_exterior_unload.png` verifies
zero voxel entities, collision cells/loading jobs and absence of all captured four
mesh IDs, two material IDs and two image IDs after moving the actual camera away;
194-206 FPS after release. All runs close their own windows. No matching native
reference image exists here: exact original geometry/cover mapping and numeric
shader controls are the checked boundary, not complete terrain appearance. The
separate projected-diffuse follow-up below adds the next native layer.

**Native projected voxel diffuse layers.** Static original inputs also suffice for
a bounded `Terrain.Layer` diffuse pass; the heightmap's existing 16-entry array path
is unchanged. `3019FFE0` / `300DF410` use `COLORS.g &0x7f` cell surface indices,
not the physics palette IDs. The cell XML has up to 32 entries. Native palette
resolution matches the first case-insensitive `Name`; duplicate names do not
authorize another entry. The supported slice requires that exact named cell entry
in the voxel's local palette. Unknown-name fallback remains declined for rendering. Original compiled meshes
overlap their serialized Z boxes by up to .46 m in this neighbour fixture; the
resource guard measures actual geometry extent and does not clip/reject that
source-authored overlap (`lights/voxel-detail-bounds.log`).
`301227A0` / `300C9120` choose named X/XP, Y/YP and Z/ZP submaterials among the first
six children, or the authored `ProjAxis` for a single material.

Renderer `38205AE0` packs each normal component as `trunc(n*127.5+128)`; native
index/vertex axis selection decodes around **127.5** and breaks ties X, then Y,
then Z. The supported high-quality path keeps the original per-vertex binary
surface/dominant-axis masks, one draw per supported surface/projection. Single
materials use all three axis weights, not an invented slope threshold. Native
index generation makes a superset of these draw lists; triangles with all three
weights zero are omitted because their fragment alpha is identically zero.
Translation-only volumes are supported, with a defensive 1024 m actual mesh extent
bound; rotated/scaled detail is declined while its base/collision stay independent.

`300C6260` stores only **DetailScaleX** at surface +76; the serialized DetailScaleY
is not used by `300DC9B0`. Its signed projection is X `(Y,-Z)`, Y `(X,-Z)`, Z
`(X,-Y)`, followed by the original diffuse texture's static TileU/V. Whole texture
periods of the cell origin are removed in f64 before GPU sampling, preserving the
world phase. Nonzero TexMod offset/rotation, animated/unknown modifiers, missing
textures, non-`Terrain.Layer` shaders, one/two-child parent layouts, non-opaque/alpha-test
variants and unknown palette names are declined. Original DDS dimensions and mip
chains are retained and sampled as repeat, non-sRGB data; no atlas reconstruction,
flat substitutes or invented texture colours are introduced.

The original shader blends in gamma: `baseGamma = mix(coverGamma,
pow(interpolatedBakedRGB,1/2.2), interpolatedBakedFlag)`, then
`layerLinear = pow(saturate(baseGamma + (diffuseGamma-.5)*DetailTextureStrength),2.2)`.
The authored linear material diffuse colour multiplies that layer. Native
`TerrainLayerVS` multiplies the binary vertex mask by
`1-min(distance/max(.001,limit),1)^4`; **after interpolation**, `TerrainLayerPS`
takes `sqrt(saturate(weight))`. The original layer technique explicitly uses
SrcAlpha/InvSrcAlpha and disables depth writes. Bevy's overlay uses the same
receiver geometry with Equal depth and no raster displacement; its large CPU
sort offsets preserve ascending surface/projection order between sky and deferred
decals, and the specialization clears the coupled GPU depth bias. This keeps the
existing PBR/fog/light integration, not the complete native terrain BRDF.

The two distances are source-proved rather than shared with heightmap's existing
1024 m approximation: cvar registration `3000978D` / `300097CE` loads **Z=128 m**,
**XY=1024 m** from `3024E7AC` / `3024E75C`. Engine vtable `30263718` slots +1208 /
+1212 resolve to `30121AF0` / `30121B10`; their ratios are initialized to 1 by
`30131190`. `3019FFE0` selects Z for single-material surfaces and XY for
multi-projection ones. The supported path uses these native defaults; dynamic
quality/view-distance ratios and the low-quality normal threshold remain open.
Ignored exact-source notes: `lights/voxel-layer-{contract,params,build}.txt`,
`voxel-detail-cvars.txt`, `voxel-normal-pack.txt`, plus original `terrain.cfx`.

Each active cell retains at most **96 diffuse draws /32 MiB layer vertex+index
bytes /32 MiB decoded diffuse/normal/detail texture bytes**, separate from the existing bounded
base meshes. It shares materials by cover node/surface/projection and textures by
path, uploads only textures actually referenced by accepted draws and drops the
worker copies afterwards. There are no new threads, distant cell requests or
per-frame geometry/material work. Cell unload removes all owned layer entities,
meshes, materials and diffuse/cover/normal/detail images. `ObjectStats::voxel_layers` and
`voxel_layers_unsupported` are additive; base mesh/triangle statistics keep their
meaning. The `VoxelDetail` marker allows isolated on/off controls.

**161 library tests pass**, nine existing ignored. Original fixtures verify cell
surface 12's rock texture/material colour and `.2 * .15 = .03` UV scale, both
native fade limits, per-vertex weights, the normal-pack tie asymmetry, signed
matrices, exact original indices and exhaustion of the draw/byte budgets. They
also reject rotated detail and test first-match duplicate/unknown palette names.
`voxel_cover --layer-check` uses the original rock diffuse texel `(161,165,172)`,
strength `.6`, authored material colour `(0.32503697,0.26735809,0.25084025)` and
linear baked `(21,14,11)/255`. Its 96 m camera against the native 128 m limit
checks zero, mixed and full surface masks, including interpolation of vertex fade
before the square root. Independent CPU weights are `0`, `.34167242`, `.68317294`
and alphas `0`, `.5845275`, `.82654274`. Inspected
`shots/aa-world/voxel/native_layers_numeric.png` returns **`[81,66,59]`,
`[69,55,50]`, `[63,50,46]` sRGB**, within one display value of the CPU controls;
348-398 FPS. The zero-weight receiver stays intact.
The cover-only regression remains byte-exact at `[17,18,6]`, `[59,49,41]`,
`[81,66,59]` (`native_layers_cover_regression.png`, 342-359 FPS).

Inspected `native_layers_exterior_on.png` / `_off.png` use the same original
neighbour camera as the cover follow-up. `--detail-off` hides only projected
diffuse layers; the cover/baked receiver meshes, collision, objects and lighting
remain active. Four base meshes /3590 triangles now have **28 projected diffuse
draws**, with zero unsupported layers, pending models or model failures. The on
image restores the original rock/ground texture patterns over those surfaces;
the off image shows the flat low-detail base. Final settled intervals are
135-174 FPS on /152-168 FPS off, with no reliable delta between separate runs on
the shared GPU. `native_layers_exterior_unload.png` verifies zero base/detail
entities, collision cells/jobs, and absence of all **32 captured mesh IDs,
18 material IDs and all captured cover/diffuse image IDs** after real camera
movement; 251-260 FPS after release.

`native_layers_cave_on.png` uses the original cave-floor camera from the base-pass
fixture. Rock walls, ceiling and ground now have authored projected diffuse detail
around the original bridge, ore cart and torch fixtures. Nine streamed base meshes
/47347 triangles (seven interior plus nearby exterior slots) have **50 diffuse
draws**, zero unsupported slots/layers and zero pending/failed models. The original
floor remains `454.91745` m; final intervals are 71-81 FPS. No brightness, material,
light or exposure value was tuned. All six final visual runs exit zero, produce no
GPU errors and close their own windows. The last five focused voxel regressions
pass after the native seam-overlap bound correction; the earlier full library
suite is 161 passed /9 ignored, and all examples check. A matching native cave
capture is still absent, so this does not claim full original appearance.

The next normal/detail-normal follow-up is described below. Native specular/gloss,
AO/rim/environment accumulation, runtime atlas generation, deformation, further
voxel LODs, clipped/procedural materials and area/portal receiver lighting remain
explicit limits.

#### Projected voxel normals and detail normals

The original `terrain.ext` enables `BUMP_MAP` with mask `0x4` and
`DETAIL_BUMP_MAPPING` with `0x8000`. `TerrainLayerVS` in `terrain.cfx` normalizes
LayerTexGen U and V, projects each independently with `t -= dot(t,N)*N`, then
normalizes each **at vertices before interpolation**. They generally are not
orthogonal; a Mikk tangent and handedness would lose that native frame. The voxel
vertex/fragment pair now carries both independent vectors. Renderer `38205AE0`
packs normals as `trunc(n*127.5+128)`; `ModificatorVT.cfi::streamPos_FromGeneral`
expands the bytes for `TEMP_TERRAIN` without normalizing them, and `Pos_VS_Terrain`
does not change that normal. Layer meshes retain that exact nonunit input for
frame projection and fragment interpolation. Base mesh geometry/depth are unchanged.

`Common.cfi::GetNormalMap` expands **BC1 XYZ directly**, retaining its authored Z.
On D3D10 the ATI2 path reads **raw YX**, expands XY and reconstructs
`sqrt(saturate(1-dot(XY,XY)))`. By contrast, `FragLib.cfi` reads the detail sampler's
**raw XY**, expands it and adds `XY*DetailBumpScale` to the base normal **after**
base Z reconstruction. It does not reconstruct Z again. The transformed world
normal is normalized after combining the interpolated U/V/N vectors. This
preserves a native channel-order quirk rather than applying the base normal's
semantic XY swap to every texture.

The own-crate loader retains original BC1/ATI2 blocks, dimensions and mip chains
as non-sRGB repeat data and disables aa-cry's automatic ATI2 block swap only for
these explicitly raw inputs. Semantic diffuse and raw normal cache keys are
separate. Normal UVs use their own supported static bump TexMod tiling; detail UVs
use the diffuse projection times authored `DetailBumpTillingU/V`. Fractional
world-cell phase is resolved after the detail tiling, so noninteger tiling does
not reset at cell boundaries. The shader does not apply a TS_DETAIL TexMod, matching
the original direct `detailMapSampler` sample. Default detail scale/tiling are 1,
from `terrain.cfx` declarations.

The same **32 MiB total cell image budget** now covers diffuse, normal and detail
textures; existing diffuse inputs load first. Unsupported normal formats, displaced
POM/offset-bump variants, unsupported bump TexMods or exhausted normal budget retain
the already supported diffuse receiver and its vertex normal. Layer draw/mesh
budgets and the two placement workers are unchanged. Images are shared by raw
path and uploaded only for accepted draws; cell unload drops all normal/detail
handles with the existing owned assets. No aa-cry fields or aa-client hook is
required, and no normal material work is added per frame.

This is a projected normal-input slice within the existing Bevy lighting path.
Native terrain BRDF/Phong or HQ specular, gloss, deferred normal/specular MRT
accumulation, AO/rim/environment lighting and quality-dependent normal buffers
remain unsupported. No shininess-to-roughness guess or lighting/brightness tuning
is introduced. `voxel_surfaces --normal-off` and `--detail-normal-off` preserve the
same diffuse/depth receiver for controlled comparisons; the unload fixture tracks
all cover/diffuse/normal/detail image IDs.

**162 library tests pass /9 existing ignored**, and all examples check. Native
fixtures verify original ATI2 block identity, exact signed UV scales/detail tiling,
packed nonunit normals, native reconstruction/addition order and declined offset/
POM/exhausted-budget inputs. `voxel_normals` uses the original rock ATI2 first texel
`(86,98)` in raw order, original detail first texel `(113,123)`, and original dirt
BC1 texel `(10,311)=(173,57,247)`. That BC1 texel deliberately separates authored Z
from reconstructed Z. Its varying packed vertex normals exercise projection before
interpolation. Inspected `native_normals_numeric.png` gives panel-centre GPU sRGB
**`[159,238,234]`, `[149,237,231]`, `[207,245,226]`**, within one display value of
independent CPU native vectors; 327-382 FPS. The lower flat patches are those
panel-centre controls, not an interpolation image reference.

Inspected `native_normals_exterior_on.png` / `_off.png` use the previous original
neighbour camera and unchanged authored lighting. Four base meshes /3590 triangles
and 28 layer draws have **16 normal /11 detail-normal materials** and 35 captured
image IDs. Settled intervals are 119-161 FPS on /116-164 off; separate shared-GPU
runs do not establish a reliable cost delta. Original cave `native_normals_cave_on.png`
/ `_off.png` preserve the floor ray at `454.91745` m, nine bases /47347 triangles,
50 layer draws and **12 normal /8 detail-normal materials**, with zero unsupported
slots/layers, pending models or failures. Final settled intervals are 68-76 FPS on
(with a shared-load dip to 51 earlier) /64-88 off. Normal lighting differences are
subtle in these mostly ambient-lit views; the numeric oracle isolates the actual
normal math. Native cave reference/portal-lighting and complete BRDF parity are
still unavailable, so these captures are controlled own-renderer comparisons.

`native_normals_unload.png` verifies actual camera movement removes all **32 mesh
IDs /18 material IDs /35 cover/diffuse/normal/detail image IDs**, all base/detail
entities and collision cells/jobs; 214-256 FPS after release. The old diffuse/fade
oracle remains `[81,66,59]`, `[69,55,50]`, `[63,50,46]` (`native_normals_diffuse_regression.png`,
153-407 FPS), and the cover-only oracle remains `[17,18,6]`, `[59,49,41]`,
`[81,66,59]` (`native_normals_cover_regression.png`, 284-403 FPS). All eight runs
exit zero, produce no GPU errors and close their windows.
Original source inputs are retained in the ignored voxel shader extracts; numeric
DDS audit is `lights/voxel_normal_inputs.py`, and tests/build/capture logs are
`lights/voxel-normals-*.log`.


Area flags are a separate unresolved rendering contract. Exact `30170860` maps
header +208 bit `0x8` to area +72 (`bAffectedByOutLights`, getter `30181120`), `0x10`
to +73 (`bSkyOnly`), `0x20` to +75 (`bDoubleSide`), `0x40` to +77 (`bUseInIndoors`),
`0x80` to +81 and `0x4` to +82. Bit `0x2` or an `ignoresky` name sets +79;
`vAmbColor` +244 is copied to area +140. The packed flags supersede the legacy bool
bytes at +256..259. Their spatial/portal visibility, ambient conversion and per-light
receiver behavior remain unimplemented; the audit does not justify hiding the sky
or changing global light intensity merely because a camera is inside a polygon.

Mixed-record QA: **148 library tests pass**, nine existing ignored; the eight
visarea source/remap tests and the final malicious voxel/water-count test pass,
and all examples check. `decals --interior --no-vegetation` uses the original cell
(20,8) first dirt decal at local Cry `(718.37158,894.16431,444.93076)`, radius 1.46,
material `tr_decal_dirt_600a`. Inspected 1920x1080
`shots/aa-world/decals/interior_close_{on,off}.png` show its real stone/campfire
receiver, with only decal visibility changed for `--off`. The missing voxel cave
floor is conspicuous in both images; this is receiver validation, not a complete
cave or original-client appearance match. Original particles animate between runs.
The streamed view has 84 decal meshes (70 deferred), 12 roads, 588 object parts,
65 sectors and zero pending/failed models. Final on logs 84-150 FPS, off 108-175 FPS;
earlier wider views log 147-175 FPS (shared GPU and simultaneous builds, not a
controlled performance comparison). `--stream-check` moves the actual camera
outside the world and asserts zero sectors, object parts, decals, roads, water,
pending models and collision cells/loading jobs. `interior_unload.png` is inspected;
all three runs exit zero, no GPU errors, and every fixture window closes.

### client/vegetation.dat, vegetation/NN_NN.dat

`vegetation.dat`: `u32 version (1)`, `u32 offset[16]`, `u32 size[16]` (4 x 4 sectors of 256 m, `sx * 4 + sy`), 64 byte records:
`bbox @0, maxViewDist @28, rndFlags @32, ratios @36, f32 pos[3] @39, f32 scale @51, u16 group @55, u8 angleX, angleY @57..58, u8 bright @59,
u8 angleZ @60` (256 steps per turn). `group` is a `<group id>` of `worlds/<world>/vegetation.xml` (2771 groups: `modelFileName`, `matName`,
`bUseSprites`, `fMaxViewDistRatio`, `bCastShadow`). Dense forests (976 cells have an empty `vegetation.dat`) keep their trees in
`vegetation/SX_SY.dat` (`u32 version`, then records, positions sector-local, `SX_SY` = the 256 m sector). The octree of `object.dat` carries
vegetation too; all three sources are disjoint and are concatenated.

### client/object.dat

`u32 n` + `n x { u32, char[256] }` models, `u32 m` + `m x { u32, char[256] }` materials, then a depth first octree:
`node { u32 version (2), f32 box[6], u32 objectsBlockSize, u8 childMask }`, an objects block (`repeated { u32 type, payload }`), then one
child node per mask bit. Item types (all 1208 cells walk exactly to EOF):

| type | payload |
|---|---|
| 1 | brush, 128 B |
| 2 | vegetation, 64 B |
| 9 | decal, 111 B; planar / forward projections plus deferred diffuse/opacity on terrain and opaque brushes |
| 13 | road, `63 + 12 n` bytes (`n` u32 at payload offset 39); parsed and projected onto terrain |
| 11 | water volume, 119 B + 12 times the two counts at +103/+115 |
| 6 | voxel v5, 198775 B plus optional length-prefixed compiled CGF blocks; first-LOD native cover/baked base, projected diffuse/normal/detail-normal and serialized collision supported; specular/atlases open |
| 14 | distant cloud, 79 B; skipped |

Roads (`placement::Road`, `roads.rs`, `roads.wesl`): the 1.2 engine's `SRoadChunk_NEW` is a
63-byte header: common render-node data through byte 38, `u32 vertexCount` at 39,
`f32 uvBegin, uvEnd` at 43/47, `f32 globalUvBegin, globalUvEnd` at 51/55, and a packed
material/sort word at 59 (material index in the low 24 bits, sort priority in the high byte).
Then come `vertexCount` float3 points, alternating left/right edges along the spline. These
are spline boundaries, not mesh vertices to connect directly. Source evidence:
`D:/aa-re/zone/out/cry3dengine_dedicate.c`, `sub_302207D0` (reflection offsets, size 63)
and `sub_30222AA0` (load, material mask and sort byte). The corresponding public
[engine road implementation](https://github.com/aws/lumberyard/blob/master/dev/Code/CryEngine/Cry3DEngine/RoadRenderNode.cpp)
documents terrain-triangle clipping and plane-distance UV generation.

Each pair of adjacent left/right edges defines four vertical clipping planes. The worker
clips the full-resolution two-metre terrain triangles against them, retains interpolated
terrain normals, and raises the result by the engine's 0.06 m road offset. Roads that cross
a cell boundary read the neighbouring heightmap; they are not clamped to their owner's cell.
UV U spans the object's stored range uniformly over its trapezoids, V spans the road's width;
only the global spline ends fade through vertex alpha. The source material's diffuse/normal
maps, colour, opacity, roughness and UV modifier use the existing `aa-cry` conversion.
An extended material samples `Map="Opacity"` independently and multiplies alpha by
`saturate(dot(opacityChannelMask, opacityMap))`, as `illum.cfx` does. This matters: of the
88 original `aa_terrain/roads` materials, 15 have a separate opacity map and 11 of those
give it different tiling from diffuse. DDS reads and meshes are prepared on placement workers,
materials/textures are cached across cells, and road entities despawn with their placement
cell. Per-road visibility uses the stored view distance capped by `brush_distance`.

Focused checks: `cargo test -p aa-world --lib road` covers malformed payload bounds,
clipped area/winding and sloped heights, UV continuity and global end fades, independent
opacity transforms/channel masks, and all 11 roads of (14,11) plus 35 Memoria (10,14) roads,
including cross-cell geometry. Adjacent original heightmaps can disagree by centimetres at
their shared samples; each side retains its own rendered terrain triangle. The `roads`
example inspects cell (14,11) by default, `--memoria` the forest path, `--reference` the Nuian
spawn reference, and `--stream-check --exit-after 18` asserts that unloading leaves no road
entities. Remaining road limits: coarse terrain LODs can diverge from these fine projected
triangles at distance, and the original per-texture specular/gloss behavior uses the existing
material approximation. Forward type 9 decals are now rendered as described below.

Visual checks (2026-10-17, shared machine, RX 9070 XT):
`shots/aa-world/roads/memoria.png` shows the original soft-banked dirt-road junction, 91
streamed road sections / 20,543 object parts, 100–120 FPS at 1600x900 after loading;
`cell14_11.png` has 12 road sections / 6,645 object parts, 126–165 FPS (the zone's orange
fog remains); `spawn-reference.png` uses the same camera/hour as `reference/ingame/spawn/real.png`,
107–122 FPS at 1920x1080. Terrain/vegetation framing aligns with that reference; the
pre-existing water, lighting and missing gameplay entities are outside this road change.
The unload smoke asserts that the road count and actual road entities become zero after
leaving the loaded world. All three screenshot runs and the unload run complete successfully, no
road shader/runtime errors. Cell (14,11) still logs the existing water `blue_sky.dds` not-a-cube
warning. All five focused road tests and `cargo check -p aa-world --examples` pass.

Decals (`placement::Decal`, `decals.rs`, sharing the road material cache/shader): the 111-byte
`SDecalChunk` stores a packed projection word at 39 (projection byte 39, deferred byte 40,
front/back depth bytes 41/42), position at 43, authoring normal at 55, row-major right/up/front
basis at 67, radius at 103, and packed material-low24 / sort-high8 at 107. Depth is
`1 - packed / 255`; 255 is a sentinel read as 254. The normal can be scaled by radius;
the explicit basis supplies the projection direction. Raw bytes remain available.
Original 1.2 evidence: `sub_30220FA0` (reflection layout), `sub_30222F10` / `_LoadDecal`
(packed loads), and `sub_3005DD50` (projection dispatch) in
`D:/aa-re/zone/out/cry3dengine_dedicate.c`. Shader evidence is the pak's `modificatortc.cfi`
(`_TCModifyDecal`) and `illum.cfx` (radial attenuation); the corresponding public
[decal implementation](https://github.com/aws/lumberyard/blob/master/dev/Code/CryEngine/Cry3DEngine/Decal.cpp)
also gives the UV equations.

Forward projection types: 0 is a planar quad, 1 projects onto static receivers, 2 onto terrain,
3 onto both. UVs are `S = .5 - dot(up, relative)/(2r)`,
`T = .5 + dot(right, relative)/(2r)`. Terrain projection follows the native centre-height
gate (`-.5 < authoredZ - groundZ < radius`), moves its centre onto the ground, and subtracts
that delta from the radius. It clips the actual two-metre terrain triangles, including
neighbour cells. Static projection clips visible LOD0 CGF triangles of overlapping brushes
(including neighbouring placement cells), excluding back faces, helpers/proxies and NoDraw/Cut
sub-materials. It deliberately uses render meshes rather than collision proxies. Native static
receiver enumeration excludes vegetation (the original code removes render-node type 2).
Combined projections can produce a separate terrain and receiver mesh.

The original spherical fade `saturate(1 - dot(relative/r, relative/r))` runs per fragment;
baking it only at sparse terrain vertices would erase small decals. Planar decals retain their
authored plane and texture alpha. DDS reads, receiver model parsing and clipping happen on the
same at-most-two pending placement workers; models are parsed once per placement task and
materials/textures are shared across cells. Decal textures clamp at their authored footprint,
including an independent opacity map when present. A 5 mm normal offset avoids depth fighting
with separately transformed receiver meshes. `ObjectStats.decals` counts the drawn meshes;
all decal entities disappear on cell unload. The `decals` example provides `--straw`, `--planar`,
`--no-vegetation`, `--pos`, `--look`, and an asserting `--stream-check`.

Verification: four focused decal tests cover the packed fields/basis, planar UV coverage,
rotated wall clipping/back-face rejection, and real cell (18,7): its 44 records produce exactly
43 terrain and five visible-brush receiver meshes. Five road regression tests still pass.
`shots/aa-world/decals/straw.png` shows the original straw around the mangers in (14,11),
125 streamed decal meshes + 102 roads, about 120–165 FPS at 1600x900. `receivers.png` inspects
the (18,7) receiver area with vegetation disabled, 72 streamed decal meshes / 2,573 object parts,
161–188 FPS after loading. `planar-floor.png` shows the original dirt stain inside a Ferre tent,
39 streamed decal meshes, about 180–215 FPS. The unload run starts with 72 decal meshes and
asserts that both the count and actual entities become zero. No decal shader errors; the
existing non-cube `blue_sky.dds` water warning remains at (14,11). These exact sites have no
real-client reference capture, so their material lighting is source-backed and visually
inspected, not a pixel fit to a reference.

Deferred diffuse/opacity (`decal_projection.rs`): the original 32-bit client and renderer now
provide both halves of the matrix setup. These are the matching original 1.2 binaries/IDBs
listed under Moons below; only disposable copies of the databases were modified. Native
`cry3dengine.dll::30056FD0` scales the authored basis columns XY by radius and Z by
`radius * max(frontDepth, backDepth)`. `300585A0` copies both depth fractions unchanged;
`30058040` maps modes 4/5/6/7 to flags 0x12/0x32/0x52/0x72. Renderer
`cryrenderd3d10.dll::380444C0` independently normalizes those columns and takes their lengths
as half extents `rx, ry, rz`. Its world-to-decal transform returns physical local distances,
without reciprocal size. For `local = axes.transpose() * (worldPosition - centre)`:

* `front = authoredFront * rz`, `back = authoredBack * rz`: the extra max-depth scaling is
  intentional, not normalized away.
* Projected UV is `(.5 + local.x/(2*rx), .5 - local.y/(2*ry))` and
  `projZ = (front - local.z)/(front + back)`.
* The original `deferredshading.cfx::DecalPassPS` clips UV outside 0..1 and negative `projZ`.
  Its alpha factor is `saturate(1 - ((projZ-.2) * (projZ<.2 ? 5 : 1))^2)`.
  Native volume draw `38044070` inverts this projection; renderer initialization `38039700`
  uses `381672A0`'s eight XYZ corners 0..1. Thus the actual receiver volume also bounds
  `projZ <= 1`: its physical local depth interval is `[-back, front]`, centred at
  `(front-back)/2`, rather than the symmetric producer Z column. Geometry uses the inverse
  native distance transform, preserving this interval even for independently normalized axes.
* Wrapped modes use `offset = local.z * decalNormal.xy`; fake-wrapped modes additionally
  multiply by `pow(sqrt(1-dot(nativeZAxis, decalNormal)^2), 8)`, retaining the shader's exact
  axis/local-normal dot product. UV becomes
  `saturate((local.xy-offset) * (1/ry,-1/rx) * .5 + .5)`; the swapped extents are native
  `g_decalSize.zw`, not a typo.
* An authored opacity texture replaces diffuse alpha with its alpha channel, using projected
  UV directly. Alpha then includes material opacity, depth attenuation and the alpha reference.
  The forward material's independent opacity UV/channel-mask behavior remains unchanged.

These formulas run per fragment on clipped real finest-terrain / visible LOD0 opaque-brush
triangles, retaining receiver vertex normals, winding, mirrored transforms and two-sided
surfaces. No box or floating quad substitutes for a receiver. Deferred projection does not use
the forward terrain centre-height gate, radius reduction or spherical fade. It currently
rejects alpha-cutout/translucent brush subsets rather than drawing across their missing masks.
`e_decals_force_deferred` / `e_decals_deffered_static` / dynamic counterparts are all registered
with default 0 in original client `30009E92` onward; non-default forced conversions are not
reproduced.

Workers and cell unload reuse the existing at-most-two placement tasks. Source textures are
shared, while each native receiver mesh gets its own projection uniform/material handle;
these handles are not retained in the priority-only road cache. Geometry limits are 65,536
triangles per deferred receiver mesh, 262,144 triangles and 256 meshes/material handles per
cell, in addition to the existing 128 m radius / one-cell-width bounds. A receiver over budget
is skipped with a diagnostic, not truncated. Native overlays sort before world translucency
and after the sky; a native-only pipeline specialization clears the raster depth bias so
their CPU sort offset does not pull them through opaque geometry. World translucency tests
ordinary receiver depth. This remains Bevy's distance-based overlay sorting, rather than a
separate native priority-sorted deferred render pass.

Remaining decal limits: this is a diffuse/opacity overlay through the existing PBR shader,
not native MRT albedo/normal/specular accumulation. In particular native square-root encoded
buffer blending, receiver normal-map perturbations for wrapping, decal emissive/specular and
normal-buffer changes are not reproduced; normal/specular modifications are disabled rather
than invented. The original mode-0 deferred fixture in (2,29) uses
`materials/decals/siegefield/o_air_deca.mtl` and its real `textures/Decal/o_air_decal.dds`.
The initial bounded 48-cell sample missed wrapped records; a targeted nine-cell audit around
the forward regression fixture found a mode-4 spiderweb in (23,10), local
`(497.16797,995.0664,387.44232)`, radius 2.12895, using
`materials/aa_terrain/decals/tr_decal_spider_400b`. It produces terrain and opaque-brush
receiver meshes; the on/off captures show the authored web on the inside floor of a wooden
hut. Mode-5/7 fake wrapping still has numeric/source checks rather than a real captured fixture.
Most Memoria (10,14) decal records name an empty material slot and correctly produce nothing
(all 346 in the sampled cell). All projected geometry uses LOD0 receivers and finest terrain,
so coarse distant LODs can diverge; `entities.xml` static props, animated/skinned entities,
vegetation and non-depth/stencil receiver semantics remain outside this slice.

Deferred verification: eight focused decal tests cover packed fields, forward regressions,
native depth units, transformed volume-to-fragment agreement, wrapped/fake-wrapped UVs and
original receiver/material preparation. The siegefield's 81 records produce 81 native receiver
meshes / 97,541 triangles, all inside the runtime budget. Distinct placement centres keep
distinct material uniforms while texture handles are shared; no decal normal map or specular
lobe is enabled. `decals --siege` visits the fixture; `AA_NO_DEFERRED_DECALS=1` is its disabled
control, and `--glass` adds an explicit near translucent test pane. The asserting unload run
starts at 81 and verifies both `ObjectStats.decals == 0` and no actual decal entities after
leaving the world. No shader/runtime errors in on, off, glass or unload runs.
`shots/aa-world/decals/deferred_floor_on.png` / `_off.png` use Cry camera
`2524,30195,398` looking at `2519.18,30199.78,390.37`; 81 decals / 1,100 object parts,
about 125–142 FPS enabled and 118–148 disabled on the shared RX 9070 XT at 1920x1080.
`deferred_glass.png` confirms the soil overlay stays under the green pane, about 129–143 FPS.
`wrapped_spider_on.png` / `_off.png` use camera `24049.17,11231.06,391.44` looking at
`24049.17,11235.06,387.44`, about 127–143 FPS enabled. `planar_deferred_regression.png`
retains the original floor stain, with 39 forward + four streamed native meshes, 143–170 FPS.
No exact real-client capture exists for either deferred site under `reference/`; these are
asset-backed on/off and lifecycle checks, not a material-lighting fit to a reference image.

Water volume (type 11), cell-local, cry space: `f32 bbox[6]` @0 (z range = `[surface - depth, surface]`), `f32 maxViewDist` @28, `u8 viewDistRatio,
lodRatio` @36, `u32` volume type @39 (low 2 bits: 2 = area, 3 = river; 515 / 1027 have flags above), an editor guid @43, `u32` material index @51 (into the
cell's material list: `game/materials/ocean/lilyut_lake01`, `.../watervolumes/mp/shore_watervol_loka_800a`, ...; empty for 223), `f32` fog
density @55, `f32[3]` fog colour @59 (linear, tiny: `0.003 0.0047 0.0063`), header to @71, then 12 byte records: plane normal `(0, 0, 1)`, `(-surface, ?, ?)`, `(uv scale x, y, 0)`,
`(depth, 0, 0)`, then the polygon points `(x, y, z)`. Rivers are chains of 4 point quads written as a strip (`0 1 2 3` zig-zags
between the banks); ponds and lakes are rings of 8 to 30 points, often listed twice. Some variants append more data after the points, so
points are read while they stay inside the box (+12 m in x and y, +16 m in z). The Nuian spawn pond (`015_015`, surface 127.0, 8
points, material `lake_tank`, density 0.6) is a unit test. 3182 type 11 items exist in main_world; the materials are `Watersurface` /
`Water` shader materials (`MtlFlags 526336`) with public params `BumpScale`, `BigWavesScale`, `SmallWavesScale`, `WavesSpeed`, `FresnelBias`,
`ReflectionAmount`, `TransparencyRatio`, `SunMultiplier`, `SunShinePow`, `SoftIntersectionFactor`, `FoamMultiplier` and the bump map
`textures/defaults/oceanwaves_ddn.dds` (BC5: x and y in the red / green channels).

### zone/<id>/time_of_day.xml, lighting.xml

CryEngine 3 time of day: `<TimeOfDay>` with `<Variable Name=".." Value=".."|Color="r,g,b"><Spline Keys="t:v:flags,..."/>` (`t` in `0..1` of the
day, colours `t:(r:g:b):flags`, sRGB; `* multiplier` variables carry the HDR scale: sun about 6 to 8, sky 2 to 6). Evaluated with native Bezier
controls, preserved tangent flags and clamped endpoints (`env::Spline`); legacy XML is migrated at load (see Environment). Used: `Sun color` x `Sun color multiplier`, `Sky color`, `Ambient ground color`, `Fog color` x
multiplier, `Volumetric fog: Global density / Ramp start / Ramp influence` and the fog layer (`Fog height (bottom / top)`, `Fog layer density
(bottom / top)`, `Fog color (top)`), `Night sky: Horizon / Zenith color` (these are the sky's gradient colours all day long: the data has
no separate day sky, the engine adds scattering), `Cloud shading: Sun / Sky light multiplier`, `Skybox multiplier`, and the post processing:
`Film curve shoulder / midtones / toe scale / whitepoint`, `Saturation`, `Contrast`, `Color balance`, `Levels: min input`. Not used:
`Filters: photofilter` (a green filter, density 0.2 to 0.4), `Blueshift`, `Sun shafts`, SSDO and the char / BG light multipliers. A zone file
has two `<TimeOfDay relic_layer="0|1">` blocks (`TimeOfDay::parse_layer`; layer 0 is the one used); layer 1 has a bluer, darker ambient.
`zone/<id>/env.xml`: `<Zone SkyBoxMaterial="Materials/sky/sky_solzreed_cbt1" Angle="130"> <Ocean FogColor="21,74,155" FogDensity="0.025"/>`
(`sky_material::ZoneSky`).
`worlds/<world>/lighting.xml` (one per world, not per zone): `SunRotation` (188), `Latitude` (154) and `PlanetRotationAxis` (25) are what the engine
reads for the sun path (`env::sun_direction`, see Environment), `DawnTime` / `DuskTime` (119 and 599: sunrise 1:59, sunset 21:59) and the durations
for the sun's fade in and out. `SunHeight` (80) and `SunVector` (the direction the light travels, z -0.8) are the editor's, the engine ignores them.

### cinemas/*.xml (login2)

Track view sequences. The camera node (`Type="3"`) has tracks `ParamId 1` = position (cry world metres), `2` = rotation quaternion written
**`w,x,y,z`** (forward = local +Y, up = local +Z), `0` = field of view (degrees, vertical); keys `<Key time value/>`. Checked: the character
select camera looks at the `select_character` spot 3.6 m away (`cinema::CinemaCamera`, two unit tests). `01_world_select_e1` starts with a
roll (a Dutch angle) and eases to the select camera.

**The real client does not use these cameras** (`reference/README.md`): world select is a flat painting, character select sits at
`(725.8, 568.0, 298.1)` looking south with a 30 degree FOV, create step 1 at `(725.2, 625.5, 298.3)`, step 3 (training field, one scene per
race) at `(621.4, 407.4, 166.0)`. `fly --camera-json reference/<screen>/camera.json` reproduces them.

## 3. Design

### Terrain (`terrain.rs`, `stream.rs`)

A cell is 4 x 4 patches of 256 m. A patch is a regular grid over the 2 m samples at one of five levels of detail (steps 1, 2, 4, 8, 16
samples = 2 to 32 m), with a skirt (depth 3 steps) around its border so neighbours of different detail never show cracks. Normals always come
from the full resolution heights (over the LOD step) so lighting does not pop. Level of detail by distance to the patch box: 350 / 750 /
1500 / 2600 m with 12 % hysteresis; mesh rebuilds run on the task pool. One BC1 / BC3 texture per cell (`cover_image`, mip chain from the
quadtree, anisotropy 8, **non-sRGB**: see the cover texel encoding), `uv = (y / 1024, x / 1024)`.

**Detail layers** (`terrain_layers.rs`, `terrain_layers.wesl`): one `TerrainMaterial` per cell = `StandardMaterial` (cover texel as its base colour
texture, lighting, shadows, fog) extended with a fragment shader that decodes the cover colour and blends the surface type layers in gamma
space like the engine (see "client/leveldata.xml"). `LayerLibrary::build` runs once per world on a task (0.3 s for main_world, cells wait for it):
it reads all 1205 `leveldata.xml`, the 315 `.mtl` and copies the DDS blocks into five texture arrays (`LAYER_ARRAYS`, `BuiltLayers::images`, built
by `TerrainExt::new`; 163 MB of VRAM): 186 diffuse maps at 512 px (BC1), 53 at 1024 px, 1 at 2048 px, 255 normal maps (BC5, 512 px; the normal
maps and the `Detail` normal maps share the arrays) and 4 more normal maps in a second normal array, mip chains down to 4 px. DXT1 normal maps are
transcoded to BC5 (two BC4 blocks from the decoded red and green). A diffuse map goes to the array of its size (the 2048 px array holds only the
maps that big, 2.8 MB each); a table texture is `slot + 1000 * flags`: diffuse flags 0 / 1 / 2 pick the 512 / 1024 / 2048 px array, normal
flag bit 1 the second normal array. A 256 px normal map is stored as 2 x 2 copies in the 512 px array (flag bit 0, the shader samples it at half the
uv, which gives the original back with its own texel density and mips, and keeps the tiles continuous across cell borders because the content has
period 1 in the layer uv). Other sizes or formats are skipped (diffuse maps below 512 px, normal maps below 256 px), a texture array may have 256
layers, so 255 per array at most (the first normal array overflows into the second, a full diffuse array drops); larger sources use their lower
mips (diffuse above 2048 px, normal maps above 512 px). The big arrays exist because 59 diffuse maps of 1024 px (58) and 2048 px (1) were read from
their 512 px mip: Austera's court yard ground (`cliff_grey_with_dry_bushes`, a 2048 px map over a 50 m period, 41 texels per metre) had 10 texels per
metre and showed blocks the size of a hand right at the camera. The mean luma gradient of the ground in front of the camera (`fly` at the gallery
camera of `30_austera`, objects on, `shots/terrain/aus_gal_before.png`) is, in the lower third / the middle, 38 / 20 % higher with the 1024 px array
(`aus_gal_after.png`) and 61 / 31 % higher with the 2048 px one (`aus_gal_huge.png`). The second normal array was checked by forcing every normal map
past the 100th into it: the render is identical to the pixel. Per cell, the background load adds
an `R8Uint` image of the 513 x 513 surface nibbles and a 3 KB uniform table: for each of the 16 surface types and 3 projections the array
slots, the uv scale, a uv offset (`fract(cell origin * scale)`, so that tiles continue across cell borders, unit tested), the detail strength
and the detail map's tiling. The shader reads the four surrounding nibbles (bilinear weights, equal types merged), the dominant axis of the
normal gives the projection weights, and each layer is sampled with explicit gradients (`textureSampleGrad`, the loops are not uniform
control flow): up to about 4 types x 1 to 2 axes x 3 fetches, usually 3. No per-frame CPU work, no extra draw calls (the draw count is the
cover texture's, one material per cell). `WorldConfig::terrain_detail = false` (`fly --no-terrain-detail`) shades the decoded cover colour only.
The material's own `Diffuse` colour (`MatDifColor`, linear, 1 / 1 / 1 for 251 of the 306 layer materials, 0.24 to 0.97 for rock layers) multiplies
the lit layer, as `frag_custom_end` of `terrain.cfx` does for objects too (the court yard's `tr_af_pavingstone_001b` is 0.82 / 0.82 / 0.78); it is
packed into the free slots of the table entry, the faded-out far terrain (cover colour only) is not multiplied.
`WorldConfig::terrain_tint` (linear 1.75, 1.75, 1.65; 1.2 / 1.2 / 1.3 before the Royal Palace fit, 0.30 / 0.32 / 0.45 for the wrongly read cover)
scales the result: the engine's exposure and film curve are not reproduced, so the tint brings the ground to the real client's brightness.
It is a least squares fit of display values over the shaded and sunlit ground of three captures at the time of the capture (`fly --camera-json`,
`--hour 9.563 / 9.998 / 9.987` for the palace views, mean ratio real / ours per channel 1.00 / 1.00 / 0.99 over 13 regions): palace floor
(shade) 30 / 31 / 29 against 28 / 30 / 28 (real / ours luma, first view, turned, wide), sunlit patch 83 against 72, spawn rock 87 / 63 against
83 / 64, hill grass 75 against 68, dirt path 82 against 85. The grass in the shade at the spawn (24 / 38 / 17 against 34 / 57 / 24) is mostly
vegetation objects and does not follow the terrain. The sunlit patch stays 15 % (display) low: it asks for a stronger sun relative to the
ambient, which is the sun / ambient calibration's job (`sun_lux_scale`). The engine does not darken the cover colour or clamp it to a
minimum: `tex2DTerrain` (see the cover texel encoding) is the whole decode, the layer pass adds `cover + (diffuse - 0.5) * strength`
(0.27 for the cobbles, so the cobbles' look is the cover's) and the lighting adds the vertex ambient occlusion (`Color.a`, only dimming).
A very dark cover (luma 0.11 to 0.16 in the court yard, decoded) is right: the floor looks pale in the capture because of the lighting.
Re-fit the tint when the lighting calibration (`sun_lux_scale`, `ambient_scale`, the `ambient_*` tuning) changes.

### Objects (`objects.rs`, `placement.rs`)

Placement files are parsed on the `aa_cry::workers` threads (max 2 cells in flight, ahead of the models in the queue) and binned per 64 m sector. Models are prepared (parsed, textures
decoded and baked, materials and meshes built) on the `aa_cry::workers` threads, nearest sector first (at most 48 jobs queued, the rest
wait as `Wanted`), and inserted into the asset collections by the main thread with `CryAssets::finish_model` (1 ms budget per frame). An
instance whose model is not ready waits in its sector (`SectorState::waiting`) and is spawned when the model arrives; the instance to model
lookup is a `(cell, kind, model or group, material, lod)` key, no strings per instance. The main thread spawns sectors nearest first
under a time budget (`object_budget_ms`, 3 ms per frame). A level of detail change builds the new sector hidden next to the old one and swaps
them when it is complete, so nothing vanishes while the models of the new level load. `ObjectStats::entities` carries a counter that
changes every frame while models are still being prepared, so the loading screen (which waits for it to settle) waits for them; `models_pending`
is the number of models in flight. A part is one entity with the node transform baked, sharing the mesh and material handles of every instance of the model, so Bevy batches
them into instanced draws. Every part has `VisibilityRange::abrupt(0, view)` (its own `maxViewDist`, capped by `brush_distance` 1000 m /
`vegetation_distance` 220 m / `tree_distance` 600 m); a sector is despawned when the camera is farther than that cap. Vegetation is split by its
own view distance: instances up to 220 m are small plants (`Kind::Vegetation`), larger ones (trees, up to 1 km and more in the data) are
`Kind::Trees` and are drawn up to 600 m. Level of detail: `_lodN` / `$lodN` by distance band (brush 140 / 380 m, vegetation 55 / 130 m, trees
150 / 350 m, with hysteresis). Models that fail to load
are remembered (`ObjectStats::models_failed`). aa-cry binds some `Map="Environment"` cube maps as the base colour of 2D materials, which is
a wgpu validation error that kills the app (the renderer prepares every material asset, used or not): `strip_cube_textures` drops such
textures when a model is first loaded.

Static entity slots (`entities.rs`): `CellObjects::entity_props` reads the direct mission entities,
separately from brush/vegetation and the already-supported particles and sounds. The source is the
1.2 client's `BasicEntity.lua` / `AnimObject.lua` plus `CEntityLoader::LoadEntity`
(`sub_32097000` in `D:/aa-re/zone/out/cryentitysystem_dedicate.c`): `GeomEntity.Geometry`,
or `Properties.object_Model` for `BasicEntity` / `AnimObject`, top-level `Material`,
cell-local `Pos`, `Rotate="w,x,y,z"`, three-axis `Scale`, `ParentGUID`, `CastShadow` (default true),
and `HiddenInGame` (default false). Parent transforms are composed before the cell origin is added;
unresolved parents, cycles and non-finite/singular transforms are skipped. Hidden static slots remain
plain placement data and never enter the draw bins. This preserves the hidden training targets and
does not spawn the CDF/CHR cinematic characters handled by aa-client's login stage.

Visible CGF slots and stopped CGA slots (`Animation.bPlaying=0`, native `ResetAnimation(0,-1)`)
use `Kind::Entity` bins in the existing 64 m sector streamer. They share model/material assets,
worker limits, spawn/finish budgets, brush LOD distances and `brush_distance` visibility cap.
An identical brush model/material/transform suppresses its duplicate entity slot once on the
placement worker. Authored names and GUIDs remain on the drawn parts (`StaticEntityPart`),
`CastShadow=0` adds `NotShadowCaster`, and `ObjectStats::entity_props` counts these parts (also
included in the existing `entities` total). Both the active and retired LOD roots are removed
when their cell unloads. There is no new per-frame geometry or XML parsing.

Real fixtures cover the `ncom_city_streetlamp01.cgf` BasicEntity in main_world cell (18,7),
and login2 cell (1,0): five map CGFs, one miniature boat CGF and ten stopped miniature CGAs are
authored visible; twelve training-target CGFs remain hidden. Fifteen of those visible slots refer
to obsolete `objects/env/loginstage/` paths absent from this pak; they are retained as placement data
and ordinary missing model loads, with no substitutes. The boat model exists. Tests load original
streetlamp, miniature boat and CGA rest geometry,
check parent/nonuniform scale/material semantics, reject cycles/actors/playing CGAs, and verify
hidden/duplicate bin filtering. The `entity_props` aa-dev example inspects the streetlamp;
`--elevator` visits the airport lift at its authored initial position, and `--stream-check`
asserts the actual `StaticEntityPart` mesh entities disappear on unload.

Validation: `cargo test -p aa-world --lib entity` (three tests), example build/check, and aa-dev
captures inspected at 1920x1080. `shots/aa-world/entities/streetlamp.png` clearly shows the
original lamp on the dock: two entity parts / 976 total parts, 173–190 FPS after loading.
The unload smoke starts with two parts and asserts zero actual mesh entities after leaving the
cell. The elevator run loads four parts / 589 total parts, 123–128 FPS late in the run; its
platform is occluded by the tower shell from the exterior QA camera, so this is a load/lifetime
check, not a clear elevator visual comparison. No exact real-client captures of these two sites
are available in `reference/`; no pixel-match claim. All own example windows exit automatically.

Remaining entity limits: unsupported CGA curves / named clips, skinned actors, FlowGraph movement/events, rigid-body
simulation are not implemented by this static-prop path. Local lights have their own path below.
The elevator is visible at its initial position and does not execute its move/wait graph.
Visibility uses the configured brush cap; the XML `ViewDistRatio` has no recovered native bound
conversion here. As with existing brush parts, compound matrix shear is not represented by Bevy's
decomposed `Transform`.

### Playing rigid entities (`objects/rigid.rs`)

`CellObjects.animated_props` separately retains authored playing `AnimObject` CGAs, with the same
parent/material/hidden/shadow transform semantics as static slots. `EntityAnimation` records the
clip name, `bLoop`, nonnegative `Speed`, `bAlwaysUpdate` and `bPhysicalizeAfterAnimation`. Hidden
slots never enter the `Kind::Animated` sector bins. The original `AnimObject.lua` starts exactly
the authored animation, uses its loop flag, then applies Speed. The supported path does the same
for the embedded `Default` clip; it does not turn unsupported playing objects into frozen props.

The near CGA node hierarchy / render parts come from aa-cry. Node v0x823 references the position,
rotation and scale controller chunk IDs at payload 188 / 192 / 196. Controller v0x826 has type,
key count, flags and exporter ID, followed by typed keys. Some real files retain stale offsets in
their copied controller headers after repacking: the chunk table remains authoritative, and this
reader recognizes that header by type/version/ID. The Mari windmill's two near controllers are
TCBQ rotation (type 10, 40-byte keys) and Bezier3 scale (type 6, 40-byte keys).

Playback currently accepts constant vector curves (Linear3 / Bezier3 with zero tangents / TCB3)
and constant-rate relative axis-angle TCBQ rotations with zero tension/continuity/bias/ease and
default controller out-of-range mode. This is a checked subset, not an approximation for arbitrary
TCB curves. Relative rotations accumulate and the result is conjugated; position tracks use
centimetres, scale is unchanged. The matching public Cry
[`LoaderCGA.cpp`](https://github.com/aws/lumberyard/blob/master/dev/Gems/CryLegacy/Code/Source/CryAnimation/LoaderCGA.cpp),
[`ControllerTCB.cpp`](https://github.com/aws/lumberyard/blob/master/dev/Gems/CryLegacy/Code/Source/CryAnimation/ControllerTCB.cpp)
and [`TCBSpline.h`](https://github.com/aws/lumberyard/blob/master/dev/Gems/CryLegacy/Code/Source/CryAnimation/TCBSpline.h)
establish the controller binding, timing, relative rotation and conjugation. In this constant-rate
subset the native quaternion spline reduces to constant angular velocity, including complete turns.
Varying vectors, shaped rotation splines, named external ANM clips and negative playback rates
remain unsupported. Physics / FlowGraph events and animation sounds are not executed.

Models, controller decoding, textures and materials use the existing at-most-48 queued model jobs,
spawn / finish budgets and shared asset cache. Animated instances stay at LOD0 within the configured
brush visibility cap. The runtime touches only spawned controlled node transforms, with no XML,
mesh rebuilding or asset reads per frame. `RigidEntity { cell, guid }` marks each instance root;
sector / cell unload despawns its whole hierarchy. `ObjectStats.animated_props` counts drawn parts,
also included in `entities`. `RigidSettings.enabled=false` / `AA_NO_RIGID_ANIMATION=1` pauses poses.

The original (11,11) `AnimObject30` is the Mari windmill at cell-local (386.02881,733.87646,110.51576),
looping Default at Speed .1. Its 100-frame / 30 FPS clip turns the wing 360 degrees; authored speed
therefore makes a complete turn take 33.333 seconds. The real-asset test checks quarter-turn
orientation, conjugation, constant scale, loop / end timing and runtime Speed; a complex Beroe
wheel fixture is rejected rather than frozen. `rigid_entities` is the aa-dev visual fixture;
`--freeze` holds the native first pose and `--stream-check --exit-after 18` checks unload.
Inspected 1920 x 1080 captures `shots/aa-world/rigid/windmill.png` and `rigid/frozen.png` show the
original blades at different orientations, with the tower / placement unchanged. Three authored
instances / six animated parts / 1,299 total parts stream at this camera. Active FPS 167-197,
frozen 169-197 on the shared RX 9070 XT; separate runs are not a controlled benchmark. The unload
run verifies zero actual `RigidEntity` roots and zero `RigidPart` mesh entities after leaving.
Both focused rigid tests, the three existing entity tests and `cargo check -p aa-world --examples`
pass. No renderer errors appeared. There is no exact real-client windmill capture under
`reference/`; this validates real geometry / motion / lifetime, not a lighting pixel match.
An audit of the 64 largest main-world entities files found 83 playing CGAs in 24 cells; none of
their model paths also appeared in the matching brush model tables. No unparented static-slot
origins lay within a forward decal's radius plus 3 m in that sample; this did not test larger
receiver meshes or parented slots. The static-entity decal receiver extension remains open.

### Local lights (`entities/lights.rs`, `objects/lights.rs`)

`CellObjects::lights` preserves `Light` and `SimpleLight` slots from the original XML, including
inactive/hidden records. The two scripts use different property names: Light's
`clrDiffuse * fDiffuseMultiplier`, specular multiplier divided by the diffuse multiplier;
SimpleLight's `clrColor * fColorMultiplier`, specular percentage divided by 100. Both retain
radius, HDR dynamic value, ambient/style/area/projector/negative/fake flags and shadow settings.
The native light shadow request is `Properties.Options.bCastShadow`, independent of the entity
render proxy's top-level `CastShadow`. Positions use the native parent transform and one cell
origin; entity scale changes a child slot's position but does not multiply the authored light radius.

Every cell owns its `LocalLight` data entities, and unload removes them together with any render
components. `LocalLightSettings` defaults to 64 nearest light components, two nearest authored shadow
requests, and a 400 m distance to the influence sphere. Selection runs at 5 Hz; point-light
components only change when the selected set or parameters change. Inactive/hidden records do
not render. Unsupported projector variants, fake/negative lights and deferred clip bounds are retained as data
and counted as unsupported, without substituting omnidirectional lights. Runtime `enabled=false`
or `AA_NO_LOCAL_LIGHTS=1` disables rendering while retaining the cell data. `LocalLightStats`
reports authored/rendered/shadowed/unsupported counts. No additional aa-client wiring is needed.

Point sources use Bevy's clustered `PointLight` path, so photometry is an approximation:
the original `ShadeLib.cfi::GetAttenuation` and `DeferredShading.cfx::GetDeferredLightAttenuation`
give `(1-d²/r²)²`; Bevy uses `(1-(d²/r²)²)²/d²`. Luminous intensity is therefore
`0.16 * radius² * WorldConfig.sun_lux_scale` (lumens multiply by `4*pi`), matching diffuse
illuminance at half-radius. RGB is the authored diffuse product, without arbitrary source colors.
The exact Cry falloff, legacy `fHDRDynamic`, per-light specular multiplier, ambient-light shader
mode, style animation, vis-area restrictions and legacy shadow-bias units are not reproduced.
Those fields remain typed data; shadows use Bevy's bias defaults and the bounded shadow budget.
IndirectLight and cubemap lighting remain separate renderer work. Ordinary RGB 2D projectors
have a bounded supported path described below.

Validation: `cargo test -p aa-world --lib light` passes five tests, including the real dock
Light_116 (radius 18, warm diffuse times two, HDR 1) and parented airport Light129; the runtime
headless point-budget test excludes inactive/hidden/projector/fake slots and releases all render components
when disabled. `cargo check -p aa-world --examples` passes. The aa-dev `local_lights` example
captures at 1920x1080, hour 0: `shots/aa-world/lights/on.png` and `off.png` show the same dock
with and without its 32 original light slots. With lights, 127–149 FPS late; disabled, 184–190 FPS.
The lamp and nearby wood become visibly lit (a fixed dock sample rises from luma 1.86 to 9.42).
The unload smoke asserts 32 rendered lights become zero, with no authored data entities left.
All own windows exit automatically; no exact real-client night reference exists for this dock.

Read-only material finding for aa-cry: the bright RGB sail in this dock is brush model #71,
`galleon_deco_sail01.cgf`, with cell material #54 `galleon_mahadevi.mtl`, submaterial #3 `ucc`
(`Shader=Cloth`, `GenMask=40300000`). Its Diffuse is `objects/env/07_ucc/ucc_pattern04.dds`;
decoded, it exactly matches the visible red/green/blue pattern. The material carries
`uccRedMask=(0.2156863,...)`, green/blue masks `(1,1,1)`, and SubSurface `ucc_logo32.dds`.
Native `cloth.cfx` lines 735–753 perform conditional UCC background-mask channel remapping and
logo compositing with the authored UV controls. `aa-cry::loader::Prep::material` calls
`shade::convert_material` / `to_standard_material`, which currently binds Diffuse as ordinary
sRGB and has no UCC remap/composite. This is a material-conversion gap, not bad placement;
no aa-cry files were changed.

**Ordinary RGB projectors** (`objects/lights/projectors.rs`): original `Light.lua` feeds
`projector_texture`, `proj_fov` and `proj_nearplane` to native entity parser **32043600**.
It halves the full authored FOV into `SLight+316`, preserves near plane at `+320`, and
loads ordinary 2D cookies with renderer flags zero. Renderer **381A9CB0** calls
**381A8AD0** in its projective-texgen basis mode, then multiplies view, projection and
the `(0.5,-0.5)` texture bias. For normalized authored columns X/Y/Z and local point
`(x,y,z)` relative to the light, depth is **+x**, and
`u=.5-z/(2*x*tan(FOV/2))`, `v=.5-y/(2*x*tan(FOV/2))`.
Entity scale is removed independently from each basis column. Native near is
`max(authoredNear,.01)`, far `max(radius,near+.01)`; the matrix oracle also checks
behind-light, near and far samples. The imported SSE `tanf` alone is replaced with
host libm in the x86 Unicorn oracle; all matrix instructions run from original bytes.

Native **38045CA0** samples the cookie at texture slot 2 with sampler byte 18 zero;
**380FB9C0** normally selects `CTexture+132` and only selects `+136` when that byte is
nonzero and dual-view flag `0x10` is present. The ordinary flags-zero load chain
**38176D00 → 381C5E00 → 381C49C0/381C5980 → 381C4EA0/381C39D0** does not request that
dual view. DDS loader **381D6740** checks the `CRYF` marker at file+124 and maps
header flag `0x800` at file+36 into ImageFile+24 bit `0x1000`; normal-map inputs do
not set it. **381C39D0** copies that image bit into CTexture+60 bit `0x8000`.
Format setup **380F4380/380F41A0** marks DXT1/3/5 as sRGB-capable; **38100230**
preserves the existing image bit only for supported ordinary formats and selects
the corresponding default view, using **380F8B20**'s DXGI mappings 71→72, 74→75
and 77→78. Thus channel images preserve both encoded bytes and the native CRYF
color-space flag; untagged cookies remain linear. Hardware decodes tagged channels
before bilinear filtering. The original shader
`DeferredShading.cfx::LightPassPS` multiplies `cookie.rgb * g_LightDiffuse * NdotL *
attenuation`. Its pass state `0x22`, decoded by **38056EC0**, is ONE/ONE ADD. Three
axis-colored Bevy lights with the same full intensity therefore reproduce the RGB
filter without a 3x gain or a division by three. Bevy's spot and point extractors
both divide lumens by `4*pi` (`bevy_pbr 0.20 render/light.rs`).

Bevy `SpotLightTexture` reads red only. The adapter splits an original cookie into
three red-channel images with the original color space and mirrors U to reconcile the native texgen basis
with a proper Bevy rotation. It circumscribes the native square with the spot's
constant inner cone, sets the outer cosine to `cos(inner)-1e-4` (Bevy's denominator
guard), and scales local XY by `tan(nativeHalfFov)/tan(outer)`. The texture footprint
and all square corners retain native UVs and full angular intensity. A fully black
outer pixel border is required so clamp addressing contributes zero outside the
square. Nonblack borders are rejected rather than repeated or clipped to a circle.

Supported: active visible ordinary non-shadowed BC1/2/3 2D cookies, positive RGB,
orthogonal non-reflected transforms, HDR/style zero, authored near plane zero.
Shadowed RGB, ambient/fake/negative/cubemap/clip-bound variants, positive/negative
authored near planes, HDR/style animation, normal-map cookies, shear/reflection and nonblack borders stay
unsupported. The native effective .01 m near clip is not implemented in the Bevy
receiver. Vis-area restrictions and the separate native specular multiplier retain
the existing local-light limitations. Projector attenuation in the original shader
is **`1-d²/r²`**, unlike the squared point-light falloff. The Bevy inverse-square
receiver remains an approximation: each component uses luminous intensity
`(16/75)*radius²*sun_lux_scale`, matching native diffuse only at half-radius.

Selection remains 5 Hz. Each RGB source consumes **three** of the 64 GPU-light slots
atomically; no partial color and no RGB shadows are allowed. Child lights belong to
the authored cell root. Two nearby cookie assets (six texture views), each at most
262,144 pixels and dimension 1024, bound resident cached channel images to 6 MiB;
at most two transient worker results can coexist with that cache. Only mip zero is
needed by the original explicit-LOD-zero shader. Two shared-worker jobs maximum
remain counted across world changes until drained; obsolete results never upload.
Cookies release when no nearby eligible source references them. The feature requires
both Bevy `pbr_light_textures` and `pbr_clustered_decals`; the plugin checks Bevy's
actual GPU capability predicate and leaves projectors unsupported on other devices.
Bevy shares one sampler across clustered decal/light textures; this path consistently
uses linear clamp. Other consumers must observe that Bevy restriction.
`LocalLightStats.rendered` counts actual GPU components, with new `projected_sources`,
`cookie_assets` and `pending_cookies` fields. No aa-client hook is needed.

A bounded survey of the 22 largest cell-entity XML files (10,648,396 bytes) found 467
projector records; 128 active ordinary non-shadowed candidates all use original
`objects/env/08_dungeon/05_libray/light_a_df.dds`. That 256×512 DXT1 stained-glass cookie
is strongly RGB (maximum channel difference 177) with an exactly black border.
Its original `CRYF` flag is `0x800`, so it uses sRGB sampling.
Fixture `projected_lights` uses original instance-library cell (0,4) **Light_587**,
GUID `491a724ff2eb4d01`, FOV 62.2°, radius 15 m, diffuse
`(.0196078,.705882,.54902)*31.3`, near zero and its authored rotation. Its neutral
receiver is an explicit rendering control, not original library geometry. `--world`
uses normal cell streaming; `--off`, individual RGB controls, `--budget-two` and
`--unload-check` isolate filtering, additive power, atomic budgeting and lifetime.
Read-only source exports and numeric oracles live under ignored
`data/extract/aa-world/lights/projector-*.txt` / `projector-native-goldens.json`.

Validation: `cargo test -p aa-world --lib` passes **139 tests**, with nine pre-existing
ignored tests; current `cargo check -p aa-world --examples` and the fixture build pass.
The source-backed tests include native identity/scaled/authored-rotation UV goldens,
black-border and linear/sRGB image rules, the half-radius photometry equation, atomic
mixed point/RGB budgeting, root/child unload and stale-world worker completion.
Inspected 1920×1080 captures under `shots/aa-world/lights/` are `projector_rgb.png`,
`projector_{red,green,blue,off,budget_two,unload}.png` and
`projector_library_{world,off,unload}.png`. Across 457,942 lit receiver pixels, the full
RGB capture equals the sum of the separate-channel captures exactly: zero byte error
and zero linear-RGB error. Original texel (232,436), outside a circular cone footprint,
remains visible near screen (645,812), with a 5×5 peak RGB (1,10,119). Off, two-slot
budget and root-unload captures contain no nonzero RGB pixels. The final RGB fixture
logs 294–414 FPS, the individual controls 278–451 FPS; separate runs on a shared GPU
are not a controlled benchmark. Its harmless no-effect dithering warning was removed
by disabling dithering with Tonemapping::None, then rebuilding and recapturing.

Actual library streaming retains 2,013 authored slots and selects 64 GPU components:
11 RGB sources/33 spots plus 31 points, one cookie, zero shadows (208–252 FPS). Moving
away in `--world --unload-check` verifies zero authored slots, components, RGB children,
cookie assets and pending cookie jobs. Every run exits with process status zero and
no renderer errors. Those initial in-scene captures did not establish a useful
authored-receiver visual comparison: interior `visareas.dat` geometry was missing.
The later `library_visarea_on/off` captures above restore that original receiver and
show the projected stained glass. No same-camera library reference is available under
`reference/`; absolute appearance is still uncalibrated. World runs also
report an existing aa-fx material-load warning for
`materials/effects/velo_particle_volumelights2.mtl`; no lighting values were tuned to
hide either scene limitation. All own windows close; the user's client is untouched.

Water: the type 11 volumes of a cell become flat meshes at the surface height (`water.rs`: quad strips reordered into rings, ear clipping
for lakes), one mesh per `WaterStyle` (material path + the volume's own fog colour / density), spawned with the cell's other data
(`water_distance` 1400 m) with the style's `WaterMaterial` (`water_material.rs`, see Water below). The terrain pokes through where the banks
are higher, which is the shore line.

### Placed sounds (`objects/sounds.rs`)

The `AmbientVolume`, `RandomSoundVolume`, `SoundEventSpot` and `ReverbVolume` entities of a cell's `entities.xml` (`CellObjects::sounds`,
parsed by `placement::load_cell_objects` next to the effects) with the `AreaShape` outlines that own them (an area lists its entities by GUID
in `<Entities>`; the outline is a polygon prism: the entity's `Pos` / `Rotate` / `Scale` applied to the points, from the lowest point to the highest plus
`Height`). They become entities in `ObjectState::sounds` (spawned with the cell, despawned when it unloads, as the effects are) and
`update_sounds` evaluates them every 0.1 s against the listener (`aa_audio::SpatialListener`, else the `WorldFocus`); nothing happens without an
`aa_audio::Engine` resource (no audio plugin in the app) and `AA_NO_SOUND_ENTITIES=1` leaves them out.

* Ambient: plays (an `aa_audio::Sound` component on the entity, 2D loops with the `daylight` parameter from `Environment`) while the listener
  is in the area or within `OuterRadius` of it; volume 1 inside, linear to 0 at `OuterRadius`; the day and night events are placed in the same
  area and the parameter cross fades them. An area's `<TimeOfDay>` window switches it off outside it. A volume no area lists plays as a point
  of radius `OuterRadius` (a guess; 2 of the 50 around the Nuian spawn).
* Random: only inside its area; `MinWaitTime` after entering, then every `MinWaitTime` to `MaxWaitTime` s, a one-shot child entity at a random
  place on a `DiscRadius` circle around the listener (`bRandomPosition`) or at the entity; leaving stops it.
* Spot: a 3D point sound within 100 m of the listener, when `bPlay=1`.
* Reverb: only the strongest `ReverbVolume` around the listener, as the `ListenerReverb` resource (preset name and weight, 1 inside, `1 - d / OuterRadius`
  outside); nothing applies it.

`placed sounds` counts are in the `PlacedSoundStats` resource; `AA_AUDIO_LOG=1` logs every start with the event and position. At the Nuian spawn
(see `docs/aa-client-sound.md`): 3 ambient playing, 7 random volumes inside, 0 spots, reverb `x2_outdoor_1`. The evaluation costs 40 to 60 us per
tick. `objects::sounds::spawn_cell(commands, cell, &CellObjects::sounds)` is public for scenes that load cells themselves (the create screens' `login2`).

### Environment (`env.rs`, `sky.rs`, `sky_material.rs`)

`WorldClock { hour, speed, zone, layer }` (default hour 10, layer 0) selects the sample of the zone under the camera (`zone_at`), smoothed over
about 0.3 s. `hour` is the server's game hour (the clients' `e_time_of_day`); the zone's splines are read at `env::spline_hour(hour, dawn_hour)`
(see Time of day warp below), everything else (sun path, `Environment.hour`) takes the raw hour. It drives: a sun / moon `DirectionalLight` (illuminance = `sun_lux_scale x luminance(sun color x multiplier)`, cascaded shadows,
3 cascades over `shadow_distance`), the ambient light (see below), the sky dome, the cloud sprites, the post pass (fog, film curve, grading) and the
water materials. The sun follows `env::sun_direction` (see Sun path below); at night the light comes from the moon side.

All HDR colours are `data value x AtmosphereTuning::radiance (540) x the view's exposure` (the material shaders output pre-exposed colours, the post
pass does the film curve). `AtmosphereTuning` is the calibration against the capture; every field can be overridden for experiments with
`AA_TUNE="sky_gain=3,fog_ext=0.05"` (names: `radiance fog_gain fog_ext glow sky_gain sky_exp sun_sky disc box_gain box_yaw fog_sky_mix saturation
exposure water_body water_lit water_absorb water_refl water_sun water_foam water_env water_ssr ssr refraction water_rough sea_body sea_absorb
sea_refl sea_sun under_absorb under_scatter ssao cloud_gain ambient_env ambient_gain ambient_sat ambient_dome ambient_spec ambient_height sky_tau
sky_phase sky_hue sky_mie sky_green bloom_level bloom_threshold bloom_offset bloom_gain sun_az sun_el`; `sun_az` / `sun_el` (degrees, bearing
and elevation) pin the sun direction instead of the sun path).

**Original stars** (`night_sky.rs`, `stars.wesl`): `libs/sky/stars.dat` is 106,956 bytes: `STAR`, version `0x00010001`,
count 8,912, then 12-byte records (`f32` right ascension in hours, `f32` declination in degrees, four bytes RGB / magnitude).
Original 1.2 catalogue reader `381B7150` confirms the Cry direction
`(-cos(dec) sin(15 RA), cos(dec) cos(15 RA), sin(dec))`. The original `shaders/hwscripts/cryfx/stars.cfx` supplies the magnitude
conversion, eight-entry flicker table, Gaussian colour / hot-core falloff and additive blend. Native draw `380BCF50` computes
sprite diameter **`max(3, floor(min(viewportWidth, viewportHeight)/96)) * r_stars_size`**, with native cvar default 1,
and half-speed flicker time. This direct 1.2 evidence supersedes the earlier public Lumberyard five-pixel fallback.
The shader's `StarSize` is diameter / viewport, expanded by +/- that value in NDC: the actual rasterized diameter is in
render-viewport pixels, not Windows logical pixels or UI scale units. Bevy's physical `view.viewport.zw` supplies those dimensions.
One bounded asset-worker request builds a single cached mesh (35,648 vertices, 53,472 indices), with no per-frame geometry or
per-star entities. Camera translation is excluded; reverse-depth testing keeps terrain and opaque props in front. The sky
background now leaves depth unwritten so stars at the far plane remain visible. `StarSettings.enabled` or `AA_NO_STARS=1`
disables them. `StarStats` exposes the catalogue count and current intensity. `EnvSample.star_intensity` samples the authored
`Night sky: Star intensity` spline at the same warped hour as the rest of the environment and blends between zones. The material
uses the existing radiance / exposure calibration; intensities below the native `1e-3` threshold hide the draw entirely.

`cargo run -p aa-world --example stars -- --shot shots/aa-world/night/stars.png --shot-after 14` captures the night sky above
the main-world hills. Same-camera controls are `--off` (`night/off.png`) and `--day` (`night/day.png`). Inspected at 1920 x 1080:
coloured stars with no dark sprite rectangles, correct hill occlusion, no stars when disabled, and no visible stars at noon
(authored intensity 0.0012 versus 3 at midnight). Night FPS 181-253, disabled 228-253, daytime 166-206 on the shared RX 9070 XT;
these separate runs are not a controlled GPU benchmark. No matching real-client night capture exists under `reference/`, so
the sky brightness has not been calibrated against a nighttime reference. The real-catalogue test validates Sirius / Arcturus,
native magnitude bytes, normalized directions, mesh counts, truncation rejection and nighttime / daytime spline interpolation.

`--example star_size` is a GPU pixel regression using the actual Sirius record, ungraded black background, MSAA off and high
test-only intensity to make the complete sprite footprint visible. Its screenshot observer asserts independent fixed answers:
1920 x 1080 -> **11 x 11 / 121 pixels**, 1280 x 720 -> **7 x 7 / 49 pixels**, 320 x 180 -> **3 x 3 / 9 pixels**.
These exercise the actual shader/rasterizer rather than a CPU copy of its sizing math. Captures are
`night/star_size_1920x1080.png`, `star_size_1280x720.png`, `star_size_320x180.png`; aa-dev fixes window scale to 1.0.
All three screenshot assertions passed. These short pixel-only runs logged 422-423 / 42-289 / 221-228 FPS respectively,
including capture/readback work; they are not world-performance measurements. All-example check passes.
The real-catalogue regression still passes. Updated normal-intensity scene `night/moons_native_stars.png` was inspected:
larger native stellar footprints, both textured moons still occlude stars, no rectangular sprite borders (160-261 FPS).
No matching real-client night reference exists, so no additional brightness calibration was introduced.

**Original moon discs / coronas** (`moons.rs`, `moons.wesl`): the original 1.2 renderer now resolves the CPU sizing gap.
Read-only inputs are `data/extract/aa-cry/ida/render/cryrenderd3d10.dll`, `render.i64` and
`data/extract/aa-cry/ida/cry3dengine.dll` / `cry3d.i64`; IDA scripts used disposable copies under the ignored
`data/extract/aa-world/night/moon-native/`. The renderer is x86, image base `0x38000000`, version `0.5.1.101406`,
timestamp 2014-10-14, SHA256 `ebf051bba5d349a6b3bf6d4eb676908115b73bb22d35d5a0cc917c325426589d` (matches its IDB).
The bounded decompilations and scripts remain under `data/extract/aa-world/night/moon-*.txt` / `.py`.

Native `381B8A50` reads primary `Moon` plus enabled `MoonN` children of the native spelling `SecondayMoons`, retaining each
time-of-day slot. `381B9080` sorts descending `Order`. Its size remap is `s = .04 + .96 * clamp(Size, 0, 10)`:
main-world `big_moon_df.dds` size .22 becomes **.2512**, and `small_moon_df.dds` size .025 becomes **.064**. Native
`381B89F0` / `381B8550` expand the quad by outer-corona size `c`: physical half width `(s+c)/sqrt(3)`, UV half extent
`e = (c/s+1)/2`, centred at UV .5. The direction-plane basis is `right = normalize(direction cross Z)`,
`up = normalize(right cross direction)`, with native X/Y basis at either pole. This preserves the disc's angular size while
extending its UVs outside the texture to draw the corona. The real zone 133's .959728 corona gives the primary quad half width
.699130 and UV half extent 2.410287. Native sampler addressing clamps the original sRGB DDS textures.

Native draw `380BDAC0` substitutes the first successfully loaded moon's direction with 3DEngine `GetMoonDir`, leaving later
moon directions authored at default `r_stars_rotate=0`. The client's `CTimeOfDay::Update` (`300A5DD0`) calls moon setter
`30123A60` with the sun's orbital matrix advanced by pi: **`sun_direction(hour+12, lighting)`**, retaining the planet-axis
offset; negating the sun is incorrect when that offset is nonzero. Main-world secondary latitude 140 / longitude 35 gives
Cry direction `(.627507,-.526541,.573576)`. Its own colour spline leaves it visible at noon, while primary disc colour is zero.

The same native update retains the sun vector for the sky (`301238C0`) but switches the global directional-light vector
to that moon orbit when **sun.z <= 0**: its `v244`/`v237` selection feeds vtable +632, resolved through the original
C3DEngine vtable at `30263718` to setter `30126C30`. That setter normalizes the vector and stores the 7500-unit light
position. `sky::directional_light_dir` now uses this exact threshold and orbit, with the existing calibrated horizon fade,
colour and intensity retained. A focused boundary check covers negative/zero/positive sun elevation, midnight and noon,
including the main world's nonzero planet-axis offset. Evidence: `night/moon-tod.txt` and `night/moon-light-setter.txt`.

Client `UpdateEngineMoonParameters` (`300A5380`) passes sampled float RGB directly times colour multiplier times
`2^(HDR dynamic power factor)`; `SetGlobalParameter` (`30128050`, cases 30/32/34) copies that RGB unchanged. HDR base getter
`30121B70` returns the value initialized to 2 by `30131C74`. These moon parameters therefore omit an extra sRGB-to-linear
conversion. Corona shader scale is **`1+1000*authoredScale`**, from `380BDAC0`. Original `x2moon.cfx` supplies straight-alpha
blending, alpha `max(textureAlpha, saturate(1-length(uv-.5)/e)^2)`, and the two colour falloffs
`coronaRGB/(1.05+(1-dot(normalize(skyDirection),moonDirection))*scale)`. The port follows the native HDR branch and existing
sky exposure, leaves depth unwritten at the far plane, and draws after additive stars while respecting moon order.

One bounded worker request reads each world's XML and up to four textures once; one cached unit quad and at most four
entities/materials serve the world. Per-frame work only updates those uniforms. World changes remove the old entities and
materials; disabling the environment or `MoonSettings.enabled` hides the draws. `AA_NO_MOONS=1` disables by default.
`MoonStats` reports loaded/authored and rendered counts, and `MoonDisc.slot` identifies the authored slot.
Moon color/shape curves participate in the raw zone blend before HDR multiplication; `moon_dir` follows the world's raw-hour orbit. Remaining limits: optional non-default
`r_stars_rotate>=2`, reflection-pass moon boost / planar moon reflections, and the native non-HDR shader branch.

`cargo run -p aa-world --example moons -- --shot shots/aa-world/night/moons.png --shot-after 14` frames both original
discs and the main-world hills. The fixture pins the real zone 133 curves; `--natural` restores the camera's local zone,
whose midnight Moon RGB keys yield nearly black discs (`night/moons_natural.png`). Same-camera `--off` and `--day`
captures are `night/moons_off.png` and `night/moons_day.png`. Inspected at 1920 x 1080: both original textured planets,
correct relative sizes / orientation, alpha-edged corona with no rectangular border, stars hidden behind each opaque disc,
no discs when disabled, and only the authored secondary visible at noon. Night FPS 185-282, disabled 183-229, noon 183-260;
separate shared-GPU runs are not a controlled benchmark. No matching real-client moon capture exists under `reference/`,
so nighttime brightness remains on the existing atmosphere calibration. All windows close after capture.
Focused moon tests pass 4/4 (real textures, numeric quad / UV / scale units, colour HDR factor, primary phase / planet offset,
daylight secondary, disable / world-change cleanup); the 12 environment regression tests and all-example check pass.

**Sky transparency order**: original sky draw `380C3E40` invokes the stars' draw slot (`380BCF50`) before moons
`380BDAC0` at default `r_stars_rotate=0` (`night/moon-native-order.txt` / `moon-native-order-detail.txt`). The port preserves
that sequence ahead of world transparency. Bevy 0.20's `TransparentSortingInfo3d::Sorted` uses projected mesh centre plus
material bias, but our sky shaders ignore translation: an origin behind the camera previously sorted stars / moons over nearby
alpha surfaces. Stars now use a CPU sort-only bias of -1,000,000 (outside these worlds' bounds; main is 35 x 38 cells), and
moon ranks add 1..4 so the intra-sky order remains representable in f32. This changes neither geometry nor depth testing.
The regression uses Bevy's real rangefinder / transparent sort key at cameras on both sides of the origin; all sky keys stay
before a pane 50 m away, with stars before ordered moons. `--behind-origin --glass` reproduces the issue with a green 75%-alpha
pane in the existing moon harness. `night/sky_glass_before.png` shows untinted discs / stars over that pane.
`night/sky_glass_after.png` correctly tints and attenuates both planets and the stars through it (211-241 FPS, no shader
errors). The rangefinder regression passes; native relative sky order is preserved. The later night-direction capture
`night/moons_light.png` was also inspected (171-206 FPS). All capture windows close automatically.

**Sun path** (`env::sun_direction`, `Lighting { sun_rotation, latitude, planet_axis }`): the engine's own formula, taken from the decompiled
`CTimeOfDay::Update` (`sub_300AECE0` in `D:\aa-re\zone\out\cry3dengine_dedicate.c`, around line 121800): the matrices
`T(0, 0, -sin axis) x Rz((hour + 12) / 24 turn) x Rx(latitude - 90) x Ry(-rotation)` are multiplied and inverted and the point (0, 1, 0) is
transformed by the result: `Ry(rotation) Rx(90 - latitude) Rz(-angle) (0, 1, sin axis)`, as (x, up, z south). The sun turns 15 degrees an hour,
highest at 12:00. With the main world's 188 / 154 / 25: rises at 2:00 in the north-east (`DawnTime` 119 min), azimuth 172 / elevation 49 at 12:00,
sets at 22:00 in the north-west (`DuskTime` 599 min + 12 h); with login2's 260 / 40 / 0 it is a great circle, 40 degrees up in the west at noon.
At/below the horizon the light uses the moon's separate half-day orbit. **Cross-check with the real client**: its capture of the Royal Palace
(`reference/ingame/royal_palace/real*.png`, zone 186, game time 9.56 h and 10.0 h) has one patch of sunlight on the floor of the court yard.
Rendering the yard with the sun at every azimuth / elevation (`sun_az` / `sun_el`, 1 to 5 degree steps) and comparing the lit area (floor
pixels brighter than luma 55 in a box, intersection over union) gave the best matches azimuth 123 / elevation 42 at 9.56 h (IoU 0.45) and 132 / 44
at 9.998 h (0.46, the wide view 0.38); the formula gives 124.0 / 42.4 and 131.8 / 44.4. (The first path in this crate, an arc from 7 to 18 h
with the sun in the north at noon, lit none of the yard.) The sun is therefore in front of the camera, to the right, above the picture in the
first palace view. Test `env::tests::sun_path`.

**Time of day warp** (`env::spline_hour`, `Lighting::dawn_hour`; `docs/research/time-of-day.md`): the server hour is the capture's hour (`e_time_of_day`
19.17 at the spawn, 2.7 to 4.3 at the combat frames, 6.17 and 18.03 at create step 3), and the engine's time of day manager hands every zone
layer a warped hour before the layer reads its splines at `warped / 24`: a quadratic per half day that moves the world's real dawn and dusk
(`lighting.xml` `DawnTime / 60`, main world 119 min = 1.98 h, so 2:00 and 22:00) onto the splines' authored 6:00 and 18:00 (the second half is clamped
at 0: on the main world every hour from 12:00 to 19:05 reads the splines at 12.0, 19.17 h reads 12.17, 2.7 h reads 6.7, 22 h reads 17.96).
`Lighting::dawn_hour` is `DawnTime / 60` (default 355 / 60 for a file without it); 6 (login worlds and almost every instance, `DawnTime 360`) is the
identity, so the login screens are unchanged. `TimeOfDay::sample(hour, lighting)` does the warp itself; the sun path and the HUD clock keep the
raw hour. Tests: `env::tests::spline_hour_is_the_engines_warp` (the research test vectors), `splines_are_read_at_the_warped_hour_but_the_sun_at_the_raw_one`,
`lighting_reads_the_dawn_time`. The palace views (zone 186, 9.56 to 10.0 h read at 10.9 to 11.1 now) changed by under 1 display value of mean abs
difference: the data is flat between 9 and 13. `AtmosphereTuning` was fitted to the spawn picture at "hour 10" (spline hour 10, a sun 44 degrees up
in the south-east); at the real hour 19.166 (spline 12.17, sun at azimuth 288 / elevation 13) the same tuning is close but not fitted: region means
of the spawn picture (rock, grass slope, far bank, left trees) are 6 to 20 display values bright in green and blue, the sky (hills) within 10, the pond water within 5
at the left and 20 dark at the right, where the real pond has a pale sheet that looks like the sky's mirror image (the cube map reflection is dim at
that angle). Not re-fitted: it is one picture, and the real client's own shade (AO, the engine's film curve) is not reproduced.

**Native curves and defaults** (`env.rs`, `env_defaults.rs`): original 1.2 initialization
**300A9450**, **300A8EE0** (Moon1-3) and **300A92B0** (character lights) define 147 variable
slots. The catalog stores exact f32 bits: scalar default/minimum/maximum or RGB defaults.
**300A8C30** creates CBezierSplineFloat / CBezierSplineVec3 and inserts constant default
keys at 0 and 1. Missing variables retain these keys; a Variable's editor Value/Color
attribute alone does not replace them. Unknown names are ignored, and the native slot-100
alias `HDR blueshift` resolves to `Blueshift`. The two relic layers are allocated from
defaults before loading (`crysystem` **365F2CE0**): a missing layer in a TimeOfDayList keeps
defaults, while a standalone TimeOfDay block is loaded into both. This replaces the old
zero-for-missing-variable and first-layer fallback behavior.

Native **300ABDD0** (float) / **300AC0A0** (RGB) calculate cubic Bezier controls. At an
interior key `i`, the automatic incoming control delta is
`(value[i+1]-value[i-1])/3 * (time[i]-time[i-1])/(time[i+1]-time[i-1])`;
the outgoing control uses the complementary time fraction. The first outgoing and last
incoming controls are one third of their adjacent value difference; the unused outer
controls are zero. Low flag bits 0..2 select incoming mode and bits 3..5 outgoing mode:
0 automatic, 1 stored/free (zero after XML InsertKey), 2 flat, 3 step, 4 linear. Higher editor
flags are retained and do not change tangents. Interior flat/step controls are zero; linear
uses the corresponding adjacent difference / 3. Native endpoint rules run before the
interior mode switch, including their treatment of flat endpoints. A left outgoing step
returns the right value, while a right incoming step holds the left value.

**300ABFA0** / **300AC540** evaluate Bezier `(A, A+outA, B-inB, B)` at the normalized
segment time. Lookup **300AE830** / **3006C250** clamps outside the first/last key in the
ToD spline's default range mode; it does not invent a periodic last-to-first segment.
The world clock still wraps hours normally. aa-world uses binary key lookup and four
neighboring keys at most, without per-frame allocations or rebuilding all curve controls.
**300A4C10** clamps each evaluated scalar to its native catalog range (values below 1e-10
in magnitude become zero) and each RGB channel to 0..1 before the zone blend. Native white
Color balance defaults also remove the former black-to-white missing-value fallback.

Numeric validation uses the original x86 bytes, not just a second formula transcription:
the read-only client image was mapped into a bounded Unicorn emulator and its native
tangent/evaluation routines executed for 35 scalar/RGB cases. Rust regressions retain the
results for real zone-179 Sun/Sky/Fog/HDR curves and nonuniform automatic/free/flat/linear/
step/editor flags, within 4e-6. Evidence/scripts and the 147-slot extraction are under ignored
`data/extract/aa-world/environment/`. The `tod_curves --hour H` example fixes the original
spawn camera plus its world-origin shift and forced zone 179; baseline linear and native
curve executables are kept separately for controlled dawn/dusk captures.

Validation: all 122 aa-world library tests pass (nine existing ignored tests), and all
examples pass `cargo check`. The recovered engine and targeted original-pak DLL have the
same SHA256 `34d6b73690d1a9d8d0ea0f5eb743fd0624107cfda28c1302826b19a3bc9546de`.
Inspected fixed-camera captures under `shots/aa-world/environment/` are
`curves_linear_dawn.png`, `curves_native_dawn.png`, `curves_linear_dusk.png` and
`curves_native_dusk.png`. Linear/native dawn logged 221–256 / 168–233 FPS; dusk logged
266–295 / 234–293 FPS, with other shared-machine GPU activity during the native dawn run.
Mean absolute RGB differences are 4.80 / 4.17 / 2.10 at dawn and 1.72 / 1.45 / 2.03 at
dusk. These are controlled comparisons against the old sampler; no matching original-client
dawn/dusk frame was available. Original native numeric outputs provide the correctness
check, without fitting new exposure or color constants. All four windows exited normally.

**Legacy XML migration**: native **300A8190** treats absent `sRGB`, or an integer other
than 1, as the older format. Only the stored spline keys of `Sun color`, `Sun color
multiplier`, `Sun specular multiplier`, `Sky color`, `Sky color multiplier` and
`Global Rim Amount` are raised to **2.200000047683716** (the double at **3025B3F8**).
The native code promotes each f32 key value to double, calls pow, then rounds back to f32
before updating tangent controls. Editor Value/Color metadata and other variable keys are
unchanged. Serialized `HDR dynamic power factor` and any recognized variable containing
`Film` are ignored for legacy input, retaining the native initialized defaults. Modern
`sRGB="1"` curves bypass all these migrations. This is load-time work only.

The real `login2` cell `001_000` AreaShape `login_224_oktop_2` contains a nonempty legacy
`Area/TimeOfDay`. Original x86 spline evaluation on its source-proved migrated keys supplies
numeric regressions for all six variable families. Noon Sun RGB is
`(.7817514,.7817514,.76674455)`, multiplier `6.8622217`; Sky RGB is
`(.35865393,.5859734,.86713576)`, multiplier `1.2332863`. Legacy HDR dynamic stays zero.
The `legacy_tod` aa-dev example renders these actual raw curves as controlled RGB/scalar
strips. `--old-parser-control` marks the same input sRGB=1 to reproduce the previous
parser's behavior; it is a numerical/visual control, not a production area override.
Original sRGB=1 zone-179 goldens remain unchanged.

Validation: all 127 library tests pass (nine existing ignored), including 18 environment
tests, and all examples pass `cargo check`. Inspected `legacy_area_before.png` /
`legacy_area_after.png` under `shots/aa-world/environment/` show the source-specified
Sun/Sky and HDR changes. The Fog and Film-whitepoint strips are pixel-identical controls.
The viewer uses raw spline time, before the world's clock warp; it does not activate an
area in production or claim a world-render/reference match. The 2D controls logged
368–387 / 477–540 FPS under shared-machine activity; these are fixture rates, not a world
performance comparison. Both windows exited normally. No brightness calibration changed.

Further producer audit: original `cryentitysystem.dll` **32084070** reads the serialized
Area attributes, polygon and nested ToD. **3208ECB0** creates/loads that ToD only for
area Group 14; **32083A10** transforms its points with the entity matrix. Proxy
**32085470** returns the master weight at CArea +244 (initialized to 1). Seamless native
slot +148 (**365F1290**) accepts area registration inputs, then **365EEC20** resolves
priorities and normalized area/zone weights. The producer is now recovered, as detailed
below. Read-only exports are under ignored `data/extract/aa-world/environment/area-*.txt`.

#### Native polygon area inputs

`area_tod` supports authored Group-14 polygons, their nested legacy/modern curves and
cell lifetime. The native bridge is **32099ED0**. **3209CD40 CAreaManager::Update**
calls **3209BFD0 CAreaManager::UpdatePlayer**, which reaches the bridge.
It passes the owner entity ID, inward boundary distance, serialized `value1` and
`Priority` to Seamless slot +148 (**365F1290**). Leaving calls +156 (**365EEB10**).
Other area groups and flag-bit-1 zone effects are not turned into ToD layers.

The subject is an **IEntity carrying flags bit 0x20**, checked through slot +24
(**32016AA0**, returns entity +24). The area manager samples its world position
(slot +152, **32016BA0**) plus **0.01 m in Cry Z**. This is not the orbit camera.
Owner hidden state is slot +200 (**32016BD0**, entity +8 bit 0x10); hidden owners
do not register. `AreaToDSubject` explicitly designates this qualified subject's
`GlobalTransform`. Zero or multiple markers disable area inputs; there is no camera
fallback. **The gameplay producer assigning bit 0x20 is still unresolved.** A bounded
read-only game/entity operand audit did not establish which local player/login actor
gets it, so aa-client must not add the marker merely because an entity is local.
The new automatic evaluator stays inactive in the integrated client until that hook
is proved. No global login-area override is installed.
`CEntity::OnRellocate` **32010A90** queues area work for mask `0x40004000`;
script binding **3204A280** separately confirms `ENTITY_FLAG_TRIGGER_AREAS=0x4000`.
That generic area-trigger flag does not establish the additional ToD bit 0x20.

Qualification recheck: original instructions **32099F16** test area owner ID
`[ESI+48h]`, then **32099F1A** loads the moving subject from handler argument
`[EBP+1Ch]`. **32099F21/26** call its vtable +18h (`GetFlags`), **32099F28** is
`test al,20h`, and **32099F2A** is **`jne 32099F49`**, entering registration.
Without the bit, ordinary runtime takes **32099F38 je 32099FC4**, skipping it;
only the separate editor gate can bypass qualification. Both UpdatePlayer call
sites (**3209C1BB-C1D0**, **3209C4DC-C4F0**) forward their moving-entity argument
`[EBP+0Ch]` into the handler's final argument slot. Thus the bit is a positive subject test,
not an owner flag or exclusion. Original **3204A280** explicitly registers
`NOT_REGISTER_IN_SECTORS=0x40`, `TRIGGER_AREAS=0x4000`, `CLIENT_ONLY=0x100`;
historical header values must not replace this 1.2 table.

CryAction **306ADEE0** sets `g_localActorId`/`g_localActor`; **306AF9A0** resolves
that IEntity, calls **AddFlags +28** with **0x40004000**, and emits GameObject
event **10**, named `BecomeLocalPlayer` by **306F7200**. Native AddFlags
**32016AB0** only ORs that mask into entity +24 via SetFlags, without adding
0x20. **305017E0 ActorSystem::CreateActor** initializes spawn flags at +32 to
zero or 0x400000; **32013BC0 CEntity constructor** copies them into entity +24.
This proves the local-actor area-update route, but does not prove its additional
ToD qualification. The remaining bit assignment in actor extension/event handling
was not recovered in the bounded x2game/CryAction pass. Raw exports and operand
scans are under `environment/area-{raw-caller,actor-source,client-actor-setter,
actor-system,flag-names}.txt`; the original actor is still not marked by aa-client.

**32083A10** transforms the polygon, then **32094AD0 SetPoints** uses its minimum Z as the base;
serialized positive `Height` remains unscaled. **3208F090** uses the native half-open
XY crossings and inclusive height bounds; nonpositive height is vertically unbounded.
**32094360** returns the closest inward XY segment distance, additionally bounded by
unobstructed floor/ceiling for positive height. The slice uses the native unobstructed
defaults; external obstruction setters and sphere/box/Bezier area geometry remain open.
Parent transforms/hidden state reuse the entity parser and cell origin is added once.

**365EEC20** computes `clamp(distance / denominator, 0, 1)`, where only an exactly-zero
`value1` selects the native **30 m** denominator. Master weight is CArea +244 (default
1; getter **32085470**, setter **32085450**). Runtime master-weight producers remain
unwired. Highest priority starts at zero. If total exceeds one, only positive weights
below the highest priority lose their proportional share of the excess, clamped to
0..1; highest-priority weights can still sum above one. Areas enter the manager first.
**300A4270** replaces an existing ToD pointer's weight and admits at most **ten unique
inputs**. The producer's total still includes valid areas rejected by this cap.
Every spatial zone then uses `clamp(sectorWeight - areaTotal, 0, 1)`, followed by
raw-curve normalization in **300A5010**. No contributing sector falls back to the
current zone at weight **1**, rather than multiplying it by the remainder.

The optional input from `gEnv +60` slot +308 is a **movie-system ToD**, not server
ZoneData: **3654D3D0** (CSystem vtable **3660C588** +20) returns `CSystem +40`, and
**36552EE0 InitMovieSystem** stores `CreateMovieSystem` at `CSystem +100`.
When present, it supplies the remaining area weight and bypasses spatial zones.
This movie producer is not implemented. The inspected login sequence has empty
`TimeOfDayName`. Read-only server ZoneData contains zone/conflict state; existing
`SCTimeOfDay`/`SCDetailedTimeOfDay` supply the clock, not an area or movie-ToD pointer.
No network layer/override is inferred from them or `ConflictZoneState`.

Area XML reads/parses run on the shared workers, at most **two outstanding jobs**,
independently of `WorldConfig.objects`. Curves reside only with loaded terrain cells.
Leaving a polygon removes its registration; cell unload, environment-off, missing
subject and world changes discard resident curves and active inputs. Outstanding
workers remain counted until drained, so repeated resets cannot flood the pool;
late results are accepted only for the active world/cell/subject. An unchanged point
and unchanged resident cells perform no polygon evaluation or curve work.
Existing registrations retain encounter order. In an overfull overlap, simultaneous
cross-cell encounter order has not been calibrated against native streaming.

Validation: original x86 **3208F090/32094360** executed for **15** real login/boundary
samples, and original **365EEC20 +300A4270** for **eight** denominator/priority/master/
capacity cases in a bounded Unicorn sandbox. Ignored scratch oracles and numeric
outputs are `environment/native_area_{geometry,oracle}.py` and `native-area-*.json`.
Committed regressions cover those values, parent transforms, hidden/group exclusions,
negative priorities, ten slots, area-first raw blending, spatial subtraction and the
current-zone fallback. Original entity DLL SHA-256:
`d1f0623a45c491888b9a2f3de54270d142d17afe2a63dd4c4ae95a3ea2a7377e`.

`cargo run -p aa-world --example area_tod -- --shot shots/aa-world/environment/area_inside.png --shot-after 16`
uses the real `login_224_oktop_2` polygon and original `03_character_select_s` camera.
`--outside` / `--no-subject` are controls, `--motion-check` crosses its authored 3 m
falloff, `--unload-check` disables the environment and changes worlds, and
`--cell-unload-check` moves the streaming camera away while leaving the subject inside.
`--no-objects` verifies area loading without rendered placements. These are world
fixture controls; the real character-select reference uses the separate authored
map stage and cannot provide a same-camera lighting comparison for this cinema.
No brightness constants were tuned to match the area screenshots.

Final validation (2026-10-17): **133 passed / nine existing ignored**, all examples
check clean. Six inspected captures are `area_{inside,outside,motion,no_subject,
unload,cell_unload}.png` under `shots/aa-world/environment/`. Motion logged native
weights **0 -> 0.127584 -> 0.949127 -> 1** at a fixed camera; no subject loaded zero
area cells. Both environment-off/world-change and actual terrain-cell unload cleared
all resident curves/registrations; the latter ran with objects disabled and the subject
still inside the original polygon. Jobs stayed <=2 and accepted inputs <=10. Scene
controls logged roughly **180-226 FPS**; the no-object control logged **201-217 FPS**
before unload. These are shared-machine fixture rates, not a native-client benchmark.
All fixture windows exited with process code 0.

Weighted `env.xml` LDR loading/blending is also proved by **365F8DD0**, **365F0210** and
engine **30122CC0**. However, **30128050** bypasses Sun/Sky LDR multipliers when renderer
query 19 reports HDR enabled. They must not be applied indiscriminately to HDR lighting;
the sampled original zone LDR inputs were also all 1. No new brightness multiplier is
introduced from this audit.

Limits: XML contains no custom control values
to restore for free tangents. Malformed/empty key lists retain initialized defaults rather
than reproducing the native empty-spline undefined output. No subjective color/exposure
calibration is part of the curve correction.

**Spatial zone transitions** (`World::environment_weights`, `TimeOfDay::sample_blended`,
`sky::TodCache`): the original 1.2 producer is recovered in `crysystem.dll`,
`SeamlessSystem::UpdateTimeOfDay` at **365EEC20**. The read-only decompile is
`D:/aa-re/client12/ida/crysystem.c`; its DLL matches a targeted original-pak extraction,
SHA256 `04b994394c24d2c31033ff196003d71913a20ecf7c7c2443520ac158c7798f23`.
With no area-ToD override, it scans offsets -2 through +2 in both axes of the 64 m sector
grid. Each sector contributes `clamp(1 - nearestDistanceToSectorAabb / 90, 0, 1)` to its
zone, keeping the greatest weight for repeated sectors of one zone. The containing sector
has weight 1; diagonals use Euclidean distance to the sector rectangle, not its center.
Only ten zone inputs enter the producer map. The native lookup operates in a moving
8-cell / 128-sector local grid (`365ED070` subtracts the current origin); aa-world applies the translation-invariant distance formula
to its global sector ownership table. Until that table arrives, the existing cell-zone
fallback remains. A `WorldClock.zone` override selects exactly that single-zone input.

Zone virtual slot 172 (**365F2520**) multiplies the spatial weight by each active relic
layer's weight before registering it with engine **300A4270**. Engine **300A5010** then
evaluates the layers and adds every raw scalar/RGB value times `weight / sum(weights)`.
Only afterward does **300A5DD0** apply the existing renderer color/HDR operations. Thus
interpolating already-lit `EnvSample`s is incorrect: RGB conversion, color multipliers and
moon `exp2(HDR dynamic power factor)` must operate on blended raw values. The new sampler
does this without building per-frame spline maps. Sun and moon orbital directions still
use the raw clock hour and world lighting configuration; curve lookup keeps the existing
warped hour. Native **300A4C10** invokes each layer's spline evaluator at warped hour / 24.
Native Bezier evaluation and the exact default table are now implemented as described above.

There is no native previous-frame interpolation in this path. The old frame-rate-dependent
exponential filter / 0.02 minimum step has been removed; continuous movement now blends
by distance, and explicit clock/zone overrides apply immediately. The ToD cache retains
only the active (at most ten) zone/layer inputs. A world change invalidates curves, lighting,
HDR and skybox cache keys; a late sector-map result from another world is discarded.
Disabling the environment clears its sample and active curve references, which also lets
dependent cloud/moon systems retire or hide their draws. Existing sky/ocean entity
ownership remains with the world plugin; this is not an independent full-world teardown.

Supported slice: the spatial blend of the selected `WorldClock.layer` (normal by default).
Polygon area-ToD priorities/weights are supported for an explicitly qualified subject;
the client flag producer, automatic per-zone relic activation and `env.xml` parameter
blending remain open. Native discrete skybox/cloud selection and fades are now
implemented below. No sky/cloud color calibration was changed.
`environment_border` exercises the actual Solzreed 179 / Silent Sea 210 border
at Cry `(15712,15808)`, with `--offset`, `--zone179`, `--zone210`, `--motion-check` and
`--unload-check` controls.

**Discrete skybox/cloud transitions** (`sky_transition::SkySelection`): native
`SeamlessSystem::UpdateEnv` **365F0210** uses the same sector-distance weights, picks their
strongest zone, and computes `clamp(2 * maximumWeight - sumWeights, 0, 1)`. The weights are
not normalized for this opacity. Equal strongest weights make the assets invisible, so
native pointer-map tie ordering cannot produce a visible material difference. A new zone
loads its sky assets with layer weight zero. While that zone and active relic/weather
layer remain selected, layer weight rises by `frameTime * 0.5`, clamped to 0..1. A same-zone
layer change lowers it at the same rate, retains the previous assets until zero, then
loads the new assets and fades in on later frames. Final opacity is spatial opacity times
layer weight. This fade uses frame time, independently of the time-of-day clock hour.

Exact original engine setters were recovered from C3DEngine vtable **30263718**:
**3012BF50** (+1048) sets the zone sky material/angle; **3012E8A0** (+1052) reapplies the
zone's shared distant-cloud list; **30124A90** (+1056) writes sky opacity and calls each
cloud node's slot +60 (**300B4DE0**, node +116). Cloud Render **300B4410** passes that value
as render-object alpha. Thus the same opacity multiplies both skybox and cloud alpha.
Slot +1060 (**3012CE90**) sets a zone effect, and +1064 (**30127FA0**) controls that effect's
spawn parameters: the extra native area/weather factor there is not a cloud multiplier.
Evidence is retained in ignored `data/extract/aa-world/environment/sky-setters.txt`.

The implementation consumes the already computed ten-or-fewer spatial inputs without
new per-frame allocations. `Environment.zone` still reports the containing sector, while
`SkySelection.zone` reports the strongest asset input. The selected authored layer reads
`Zone` / `Zone_Dominion_Normal`; **365F3830** leaves absent list layers initialized rather
than falling back to the first block, and loads a standalone `Zone` into both. Cloud XML
is one list per zone; native reapplies it when the selected asset layer changes. Only the
active sky material, at most two active sky metadata entries and the active cloud list
stay cached. World change or environment off resets the selection; off also drops skybox
texture references and despawns clouds.

`environment_border --fade-check --motion-check` verifies real border movement, shared
cloud alpha and zero-opacity asset changes. `--zone179 --layer-check --fade-check` switches
the real Solzreed sky to its authored dominion variant; `--unload-check` verifies retirement.
This implements the selected `WorldClock.layer` fade. Automatic per-zone relic activation,
weather-layer inputs, zone-effect spawning/opacity, automatic area subject qualification and weighted `env.xml`
multipliers remain explicit gaps. Existing sky/cloud brightness and shader calibration
are unchanged.

Fade validation (2026-10-17): all 125 library tests pass (nine existing ignored), including
the real zone-179 dominion material, scalar shared alpha, interrupted/reversed layer fades,
world reset, bounded sky caches and texture/entity retirement. All examples pass `cargo
check`. Inspected captures in `shots/aa-world/environment/` are `sky_border_no_fade.png`
(old discrete selection, 217–312 FPS), `sky_border_fade.png` (exact border, 235–290),
`sky_border_motion.png` (202–264), `sky_layer_dominion.png` (184–295) and `sky_unload.png`
(183–241). The GPU motion fixture crossed the real border in a 0.086914 m frame step with
sun/fog RGB deltas 0.000077 / 0.000063; attached cloud uniforms matched selection opacity,
and every asset change occurred at zero. The relic fixture faded to the authored
`Materials/sky/sky_salpi_nuimari_cbt4` variant; the unload fixture asserted no active sample,
selection, skybox texture handles or cloud entities. All windows exited normally. There
is no original-client border or relic transition sequence in `reference/`, so these compare
the recovered contract against the old sampler and real authored asset controls.

Validation (2026-10-17): 15 native regression tests passed, along with all 13 environment
tests and the real Solzreed/sea 2 cm border continuity test. The GPU motion fixture crossed
the same boundary in a **0.117188 m** frame step with sun RGB delta **0.000105** and fog
RGB delta **0.000085**, rather than a full-zone step. Inspected captures under
`shots/aa-world/environment/` are `border_blend.png`, `border_zone179.png`,
`border_zone210.png` and `border_motion.png`. The exact-border inputs are `(179,1)` and
`(210,1)`; blended sun is `(6.048904,5.651661,5.054752)` and fog
`(.156384,.277799,.467424)`, after the existing conversions. Single-zone controls verify
the changed fog coloration and expose the still-discrete skybox selection. Captures ran at
176-298 FPS (blend), 182-249 (forced 179), 182-212 (forced 210) and 182-232 (moving).
The GPU unload assertion passed (no active sample or cloud entities), and
`cargo check -p aa-world --examples` passed. All fixture windows exited normally.
No original-client still or sequence of this border is available in `reference/`; visual
comparison here is against the two original-data zone controls, with the blend contract
proved independently from native code. The old spawn reference's sky calibration gap
remains unchanged; there was no new brightness fit.

**Sun direction at the spawn hour**: the raw formula (`sun_direction`) puts the sun at azimuth 288 / elevation 13 (west-north-west, low) at 19.166 h.
An earlier reading of the diffuse picture ("lit from the north-north-east") was wrong: `fly --camera-json reference/ingame/spawn/camera.json --shift 11264,11264,0
--hour 19.166` with `AA_TUNE=sun_az=A,sun_el=E` for 28 directions (azimuth 20 .. 340, elevation 5 .. 45), correlating the luminance of the left half
(grass bank and rock, where shadows show) with the real picture: the best Pearson r (0.705, high pass 0.431) is the unpinned engine formula
(287.5 / 13.2), the neighbours 270 / 13 and 305 / 13 give 0.61 and 0.56, 20 / 13 and 340 / 13 (north) 0.52 to 0.57, 160 to 200 (the sun ahead) 0.60 to 0.63;
the peak is sharp and at the formula's direction, so the sun path needs no change.

**Sky** (`sky_material.rs`, `sky.wesl`): a dome of radius `sky_radius` following the camera, a `Material` without prepass. Gradient from
`Night sky: Horizon color` to `Zenith color` (x multipliers x `sky_gain`, 2.4), sun glow and disc, covered by the zone's sky box the way the engine's
`Sky` shader (`sky.cfx`) blends it: colour = `tex.rgb x lerp(SkyColorMultiplier x ShadeColorFromSky, SunColorMultiplier x ShadeColorFromSun,
exp2(-Attenuation x 8 r))`, alpha = `tex.a x Opacity x r ^ AlphaSaturation` with `r` the red channel. The box is `box_12.dds`, `box_34.dds` and
`box_5.dds` next to the material's diffuse map (`Materials/sky/sky_solzreed_cbt1.mtl`, `Shader=Sky`); layout found by matching the face edges:
four side faces are the upper halves of cube faces (1024 x 512), clockwise north, east, south, west; `box_12` stacks north (top) and east
(bottom, flipped vertically), `box_34` south and west, `box_5` is the top face with north at its bottom edge. `Angle` of `env.xml` rotates
it. The stormy photo is only faintly visible (`box_gain`), as the real picture shows a pale overcast sky with a few bright patches.

The sky hue (`update_sky`): the gradient colours keep the data's luminance, their hue comes from the scattering variables, as the engine's sky
light model does: `Sky light: Wavelength (R / G / B)` give the Rayleigh hue `(1, (R/G)^4, (R/B)^4)` x `Rayleigh scattering` x `Sky light: Sun
intensity`, plus the `Mie scattering` term (`sky_mie` 0.5, green boosted by `sky_green` 1.2), mixed in by `sky_hue` (1 = fully). Without it the
data's `Night sky` colours give a nearly white sky (blue over red 1.14 at the palace, the capture has 1.3 to 1.75). The shader blends horizon to
zenith by `exp(-elevation / sky_tau)` (16.5 degrees; the old blend was an exponent of the cosine) and brightens towards the sun by the Rayleigh
phase, `1 + sky_phase x ((1 + cos^2 g) / 1.03 - 1)` (`sky_phase` 2); the sun disc fades out below the horizon with the time of day's `day`.

**Dusk** (zone 179, `fly --pos 15578,15382,128.5 --yaw 263 --pitch 12 --fov 70 --hour 17.3`): the scattering hue is the calibration of the noon
captures (a bluish sun intensity colour); at 17.3 h the same variable is orange (and the gradient colours are orange at the horizon, lilac at the
zenith), so forcing it on the whole dome made a flat tan wall, at 18.6 h (the gradient is deep blue, the sky light's `Sun intensity multiplier`
has dropped from 10 to 1) a maroon one. The hue weight (`sky_hue`) is therefore scaled by `smoothstep(20, 40, sun elevation in degrees)`: full
at the palace and spawn calibration hours (the sun at 42 to 44 degrees), none below 20 degrees, where the zone's own gradient takes over (17.3 h:
orange red horizon blending to lilac, 18 h purple, 18.6 h deep blue). The sun's glow and disc (and the fog glow of the post pass) are shown
by `sun_shine`: the `day` factor or, for the warm colours of a sunset (red above blue in the linear `Sun color`), the colour itself; a sunset
sun has a low sRGB luminance, which `day` alone read as night (the disc and glow were hidden at 17.3 h). The blue "sun colour" after 18 h is the
moon: it keeps the dome's disc dim (`day` 0.2). Shots: `shots/water/dk_c16.png`, `dk_c173.png`, `dk_c18.png`, `dk_c186.png` (before:
`shots/gallery/24b_spawn_golden_hour.png`).

**Ambient light** (`update_ambient` in `sky.rs`): the data's flat ambient (a constant `GlobalAmbientLight`) left the shade too dark and too
saturated, so the camera carries a hemispherical `EnvironmentMapLight` instead (`GlobalAmbientLight.brightness` is 0; `AA_TUNE=ambient_env=0`
brings the old flat ambient back). The top is the zone's `Sky color`, the bottom `Ambient ground color`, the middle their mean; the hue of each is
pulled towards the sky dome (`ambient_dome`, the mix of the horizon and zenith colours: this is the "sky brightening" of the engine) and
desaturated (`ambient_sat`, 0.75 keeps 75 % of the saturation), the level is `ambient_scale x ambient_gain (1.3) x luminance`. The bottom colour
(the ground bounce) rises towards the sky colour with the camera height between `Ambient min height` and `Ambient max height` of the time of day
(`ambient_height`, 1 = fully: high above the ground the shade sees more sky). The cube map has
1 x 1 faces, so it costs nothing per pixel beyond the normal lighting; it is rebuilt only when the colours move by more than 1.5 % of the peak
(a time of day change), the diffuse map is the lit one and the specular map is `ambient_spec` (0.2) times it: the full strength made the aa-cry hair
and the water speckle. `sun_lux_scale` went from 1700 to 1000 with this change (the sun and the ambient share the brightness of the lit
ground; the terrain tint is unchanged). Shade means of `spawn.png` against the capture: trees 28 / 45 / 24 against 28 / 40 / 30, grass in shadow
28 / 43 / 11 against 29 / 41 / 18 (it was 19 / 35 / 17 and 19 / 33 / 6), sunlit trees 41 / 60 / 28 against 48 / 61 / 38, rock 84 / 83 / 77 against
75 / 78 / 67. With the sun path, sky hue and ambient of the Royal Palace fit (below): trees 31 / 54 / 33, grass 24 / 41 / 10, sunlit trees
38 / 64 / 33, rock 59 / 69 / 72 (the sun now stands south-east at 10 h, the capture's rock is lit from the north-north-east), spawn mean
63 / 84 / 79 against the capture's 68 / 80 / 70.

Royal Palace (zone 186, `reference/ingame/royal_palace/real*.png`, region means of the `fly` example at the capture's game time, ours before
-> after (capture)), turned view: sky 193 / 206 / 218 -> 151 / 180 / 222 (173 / 203 / 232; blue over red 1.13 -> 1.47, capture 1.34), stair roof
65 / 65 / 69 -> 38 / 46 / 62 (34 / 47 / 59), tower 48 / 48 / 48 -> 44 / 48 / 54 (42 / 53 / 53), shaded cobbles 21 / 21 / 20 -> 25 / 29 / 33
(26 / 34 / 33), sunlit patch 18 -> 72 / 65 / 57 (87 / 83 / 72). Wide view sky 143 / 152 / 161 -> 93 / 114 / 152 (81 / 110 / 142). First view sky
196 / 211 / 223 -> 188 / 208 / 237 (162 / 212 / 242).

**Clouds** (`sky_clouds.rs`, `clouds.wesl`): `clouds.xml` of the zone lists the distant cloud billboards (`bearing`, `distance`, `height`,
`size`, `speed`, `material`). The original client `cry3dengine.dll` now proves the native parameter contract:
`3012E8A0` (`C3DEngine::SetSkyBoxCloud`) copies bearing / distance / height / size / material /
speed from input-record offsets 8 / 12 / 16 / 20 / 24 / 28. Size goes to both quad half extents;
`300B42A0` sets the node's current bearing from its authored bearing. Native `300B4410`
(`CDistanceCloudRenderNode::Render`) computes `camera + Rz(bearing) * (0,distance,0) + (0,0,height)`
in Cry coordinates, then advances bearing by `frameSeconds * speed`, wrapping once at +/-360.
Thus native bearing 90 points towards negative Cry X; speed is degrees per frame-second,
independent of the world clock's game-hour speed. A bearing above 360 remains usable for its
first pose, then wraps as the native render call does. The radial plane uses
`normal = normalize(offset)`, `right = normalize(normal cross worldUp)`,
`up = right cross normal`; it ignores camera pitch/roll. Its full width and height are both
`2*size`, rather than a screen-aligned texture-aspect-scaled rectangle.

The original XML adapter is now recovered in `crysystem.dll` **365F9000**: bearing,
distance, height, size and speed load directly into record offsets +8/+12/+16/+20/+28.
Getter **365F9960** returns the zone's one shared cloud list. Engine **3012E8A0** maps
those fields directly into its node parameters; **300B42A0** initializes the current bearing
from the authored bearing. There is no XML-unit conversion in this path, confirming the
parser's unchanged degrees/metres fields. This supersedes the earlier upstream-adapter
boundary; the original source was in the zone producer rather than the game DLL.

Shading follows the engine's `DistanceClouds` material (`distanceclouds.cfx`): density = the red channel of the sprite texture, the
colour lerps from the sky shading colour to the sun shading colour by `exp2(-Attenuation x density)` summed over 8 taps towards the sun in the
sprite plane, alpha = `density ^ AlphaSaturation x Opacity`. UVs stay fixed; motion is physical
angular advection of the node, not guessed UV scrolling. The shader's packed tangent/binormal
are radial right / negative up, so the eight light taps now use that frame rather than camera
right/up. Lighting selects the same native sun-or-moon direction as the directional light.
The original vertex shader forces far depth (`z=w`); `clouds.wesl` uses Bevy reverse-Z zero,
read-only depth and alpha blending. CPU sorting keeps clouds behind world translucency and
above the opaque sky/catalogue, including when the camera is behind the world origin. The old
extra horizon `smoothstep` fade has been removed: it does not occur in the original shader.
Existing HDR/exposure/brightness calibration remains (`cloud_gain` scales brightness); no
reference-based retuning is part of this motion change.

`CloudSettings.enabled` / `AA_NO_CLOUDS` implement the native `e_clouds` on/off behavior: hide
sprites and freeze their current angles, then resume. `Time` pause likewise stops motion.
Zone/world changes reset authored angles and retire old entities/material references; only
active-zone textures stay cached. An environment unload clears all cloud entities and cache
references. Work is O(active sprites/materials), with one shared quad, shared per-material
uniforms and at most 128 sprites per zone; no per-frame mesh generation or asset reads.
`sky_clouds::CloudSprite::{definition,bearing}` expose the authored data and next-frame angle
for diagnostics. The `clouds` example provides day, `--night`, `--off`,
`--glass --behind-origin`, `--reference`, `--motion-check` and asserting `--unload-check`
fixtures. `--reference` uses the position, direction, FOV and hour from
`reference/ingame/spawn/camera.json`; this isolates sky comparison, with no claim of matching
the old capture's terrain/props or unknown elapsed cloud phase.

Validation (2026-10-17): all five cloud tests passed, including real zone-179/186 materials,
60-second native angular motion, pause / switch resume, both sides of world-origin alpha
sorting, zone/world changes and complete entity/cache unload. `cargo check -p aa-world
--examples` passed. GPU captures are under `shots/aa-world/clouds/`: `day_8s.png`,
`day_28s.png`, `night.png`, `off.png`, `glass.png` and `reference.png`. The real authored
speed-0.1 cloud advanced **1.000381 degrees in 10.003 frame-seconds** in the GPU fixture.
Day captures ran at 165-252 FPS under shared builds (the first capture 200-220), night at
218-296, off at 235-314, behind-origin glass at 209-234 and reference pose at 192-222.
The glass fixture shows clouds below foreground alpha and blocked by opaque geometry;
the day/night controls preserve terrain occlusion and catalogue-star attenuation. The GPU
`--unload-check --exit-after 18` run asserted four live sprites before environment teardown
and zero afterward, then exited normally. The
disabled control still contains clouds baked into the original skybox: `e_clouds` controls
the separate distant nodes. The reference comparison retains the existing visibly bluer /
less hazy sky and brighter-cloud calibration gap; no phase or pixel match can be asserted
from its single still image and unknown elapsed cloud phase.

### Post (`post.rs`, `post.wesl`)

`Post { bloom, fog, exposure_ev, ssao }` on a `Camera3d` (added automatically to the focus entity, else the first camera, when
`WorldConfig::post`; aa-client's camera needs nothing) makes it an HDR camera (`Hdr`, `Tonemapping::None`, `DepthPrepass` with `TEXTURE_BINDING`
usage; `bloom` is the gain of the bloom below, 1 = the engine's) and adds `PostUniform`. Bevy's own `Bloom` is not used any more. The passes (`post_pass`
after `bloom_passes`, both before the tone mapping slot) are:

* **bloom** (`post_bloom.rs`, `post_bloom.wesl`, `bright_fragment` of `post.wesl`): the engine's `HDRBrightPassFilterPS` + `FilmicMapping`
  (`postprocess.cfx`, `r_HDRRendering 3`), with the 1.2 renderer's cvar defaults (read from `cryrenderd3d10.dll`): `v = c x r_HDRBrightLevel / 0.18 -
  r_HDRBrightThreshold` (1.25 / 6, `c` the exposed scene colour), `max(v, 0)`, `v / (r_HDRBrightOffset 5 + v)`: a knee that saturates, so the sun disc and
  white-hot particles bloom like a white wall and a lit wall does not. It runs on the fogged scene (the same `scene_color` as the final pass) at half
  resolution (the mean of the 4 pixels of a block), into a 4 level `Rg11b10Ufloat` chain (1/2 .. 1/16, 13 tap downsample, 3x3 tent upsample added back
  up to level 0; the engine blurs three levels of its own and sums them). The final pass adds level 0 times `BloomColor x BloomMul x gain` to the exposed
  scene before contrast and film curve, as `FilmicMapping` does (`exposure x scene + bloom x 0.5`). The colour is the world's `env.xml` `<HDRSetup>`
  (`env::HdrSetup`: main_world `BloomColor="204,244,255" BloomMul="1.8"`, login2 white / 1.5; `FlaresMul` 0, `StreaksMul` 0, login2 flares 0.1: no flares or
  streaks to draw) and `bloom_gain` 0.3 (`r_HDRBloomMul` 0.2 x 3 x 0.5), so the strength is about 0.43 / 0.52 / 0.54 on the main world. One deviation: the
  threshold acts on the **luminance** and keeps the pixel's hue (the engine thresholds every channel; the blue channel of a sky is far above the threshold
  while its luminance is not, and painted a blue fringe round the foliage). Cost (1080p, RTX class GPU, the `aa-dev` pass timers): `aa_bloom` 0.17 ms +
  `aa_post` 0.10 ms, against Bevy's bloom 0.27 + 0.06 before. Tuning keys `bloom_level`, `bloom_threshold`, `bloom_offset`, `bloom_gain`.
  The bloom is not visible in the spawn and combat captures (the sun is off screen and nothing there reaches the threshold), as in the real ones; it shows
  with the sun in view (a halo that thickens towards the disc), on the sun's glitter on water and on HDR particles.
  What is **not** here: the engine's **glow buffer** (`r_Glow 1`): glow-flagged materials (`GlowAmount` is set in 2682 of the 67885 materials, nearly all
  effects and particles) write `GlowParams x diffuse x MatDifColor` into a separate target, which the engine blurs (3 levels, weights 2.0 / 1.15 / 0.45)
  and adds. That needs a pass in the material shaders (aa-cry's, aa-fx's); the cheap substitute is for those materials to emit HDR (colour x
  `GlowAmount`) so that the bright pass above finds them. The real Return orb (a white-hot disc with a violet halo about 55 px across) and the blade of
  Triple Slash rely on it; with the current aa-fx particles (GAIN 1.0, no fill) the orb does not reach the threshold. Sun shafts (`sunshafts.cfx`, 8 tap radial
  blur of the sky mask towards the sun, `Sun shafts visibility` about 0.83 in the day, the strength set from C++) are not done.

One fullscreen pass then replaces the tone mapping:

* **fog** from the depth buffer: height exponential layer (`Fog height (bottom / top)`, densities, optical depth integrated along the ray), starting
  at `Volumetric fog: Ramp start`, extinction `fog_ext` (0.04 / m at density 1) x `Global density` x layer density, colour from the layer colours
  (`Fog color`, `Fog color (top)`, mixed 60 % towards the sky horizon colour: the data fog colour is more saturated than the real haze) plus a
  sun glow; skipped beyond 0.9 `sky_radius` (the dome has its own haze),
* **under water**, instead of the fog, when the camera is below the surface of the sea or of a water volume (`update_post` asks
  `WaterQuery::body_at` for the camera position each frame; `PostUniform::under` / `under_color`): the engine's `OceanOutof` /
  `WaterVolumeOutof` fog. Along the view ray the scene is dimmed by `exp(-k (path + depth of the thing seen))` and the water's own colour is
  scattered towards the eye, `colour x exp(-k depth of the eye) x (exp(kt path) - 1) / kt x k`, `kt = k (dir.y - 1)`, so the light dims with
  the depth and the sky above looks brighter. A ray that reaches the surface before the scene sees what is above only inside Snell's window
  (`dir.y` above 0.62 to 0.72, 49 degrees around the vertical: the sky through a bright soft edged disc), outside it the surface is a mirror of
  the water below (the pure water colour, as at the horizon). `k` is the zone's `Ocean fog density under water` (0.043, base 2) for the sea, the
  volume's `density x water_absorb` for ponds (lake_tank: murky green within a few metres); the colour is the one the surface shows from above
  (`sea_color` / `volume_color`). The water shader does not draw the surface from below (`discard`). Knobs `under_absorb` (extinction
  multiplier), `under_scatter` (gain of the water colour). Not done: light shafts, caustics on the bed, refraction of the window, red absorbed
  first, the transition at the surface line (the engine fades through the surface). Shots: `shots/water/uw1_0.png` (sea floor, 53 m deep),
  `uw1_35.png` (looking up), `uw1_s.png` (8 m deep), `uw2_a.png` and `uw2_b.png` (the Nuian pond), `cli_uw.png` (the client on the sea floor).
* **grade**: exposure, contrast around 0.18, colour balance, saturation (data `Saturation` x 0.85),
* the engine's **film curve** (`postprocess.cfx`: `x (a 6.2 x + b 0.5) / (x (a 6.2 x + 1.7) + c 0.06)` with shoulder / midtones / toe scales from the
  data, divided by the same at the white point; its output is gamma encoded, so the pass raises it to 2.2 because the swap chain re-encodes), `Levels: min input` (in 0..255 as in the engine's `AdjustLevels`, so the data's 0.07 is a black point of 0.07 / 255, not 7 % of the range).

**SSAO** is on by default (`Post.ssao`; set it to `false` to turn it off): Bevy's `ScreenSpaceAmbientOcclusion` (low quality, radius 1.2 m,
thickness 0.25), which needs `Msaa::Off` (SMAA High replaces it) and a normal prepass. It deepens the shade under bushes and trees the way the
capture shows, for about 8 to 10 % of the frame time. It was opt-in until aa-cry's hair material became prepass-safe (`9751975`).

`aa_world::post::{Post, PostUniform}` is the reusable setup; a second camera only needs the `Post` component. `--no-post` / `post: false`
leaves the camera alone (Bevy tone mapping, no fog, the water falls back to a constant thickness and no reflections).

### Water (`water.rs`, `water_material.rs`, `water.wesl`, `water_query.rs`)

A `WaterMaterial` per style (material path + fog): constants from the `.mtl` public params (see object.dat), environment part refreshed each frame
by `update_water`. The shader runs in the transmissive phase (`reads_view_transmission_texture`: the frame's opaque colour is copied once) and
reads the depth prepass. It follows the engine's own two shaders, `watersurface.cfx` and `watervolume.cfx` (read from the pak,
`game/shaders/hwscripts/cryfx/`), for ponds, lakes and rivers:

* normal: native authored volume UVs and the material's flow/NO_FLOW tap branch (see follow-up below), `WavesSpeed` and
  `BumpScale`, flattened beyond 60 to 260 m; the sea retains its existing world-coordinate approximation,
* **volume fog** (`WaterVolumeInto`): `mix(fog colour, refracted scene, T)` with `T = exp(-k thickness)`, `k = density x water_absorb` and
  `water_absorb` = ln 2 (the engine's `exp2(-density x depth)`); thickness is the depth prepass minus the surface depth along the view ray. The
  old absorption was `water_absorb` 6, nine times the engine's: legs, the rat and the bed vanished a metre down (lake_tank: density 0.6, 0.66
  per metre). Refraction offsets the lookup by the bump, rejected where the sample is in front of the surface. The fog colour of the data is
  tiny (lake_tank 0.002 / 0.005 / 0.004), the real pond is far brighter: it is lit by `sky + water_lit (0.25) x sun` and scaled by
  `water_body` (6, fitted),
* **surface** (`WaterSurfacePS`): `final = lerp(refract, ReflectionAmount x reflect, TransparencyRatio x fresnel)`, `fresnel = bias + (1 - bias)
  (1 - N.V)^5` with `bias` = `FresnelBias`, the normal flattened as the engine does (bump x 0.15); `TransparencyRatio` is weighted by
  `water_refl` (1). The reflected colour is the material's **environment cube map** (`Environment` texture of the `.mtl`, nearly always
  `textures/cubemaps/camera_river_cubemap.dds`: a dark overcast panorama with trees, rock and a sun, sampled with the Cry space reflection
  vector, +Z is up) times the engine's `sun + sky` colour, and `water_env` (0.08, fitted) for the unit conversion. There are no screen space
  reflections on ponds by default (the engine has none; `water_ssr=1` brings the two ray march back on top of the cube map),
* sun glint `pow(dot(reflect(unclamped normal), sun), SunShinePow) x sun colour x SunMultiplier x water_sun`,
* **foam** (`FOAM`): `textures/defaults/water_foam.dds` (256 x 256 DXT5, linear, wrapping) sampled twice, `world.xy x 0.3197 - a` and
  `world.yx x 0.2797 + a` with `a = fract(time x 0.025 x WavesSpeed)` plus the bump x 0.075, `lum = saturate((rgb1 + rgb2 summed over channels
  x 0.333) - 1)`; `foam = 1 - saturate(FoamIntersectionFactor x (0.2 / lum) x depth)`, masked by `smoothstep(0.2, 0.7, soft x foam)`, added as
  `foam x lum x (sun + sky) x FoamMultiplier x water_foam (0.7)`. These are the white flecks of the real frames: sparse over shallow water,
  dense where the water is thin (around legs and plants, at the shore).

**Historical material fit, before the authored UV/motion follow-up below**, to the real combat frames (`reference/ingame/combat_triple_slash/real_05.png`, `combat_return/real_10.png`, the Nuian pond; `fly
--camera-json cam_c05.json --shift 11264,11264,0` at the frames' real game hours 2.81 and 3.76, no pinned sun: they read the splines at 6.7 and
7.5, the sun is low in the north-east; before the time of day warp the fit used `--hour 10` with `AA_TUNE=sun_az=290,sun_el=15`, which gave 6.8 and
`water_body` 6 / `water_lit` 0.25, the real hours score 11.9 with those and 7.2 with the current 7 / 0.35): mean colour of 12 water regions (near / mid / far, left / right, both frames), mean abs error over
R, G, B in display values: 38.5 before, 7.2 after (the real region means are all within (38..50, 51..60, 33..47), a luminance of 47 to
56 that varies little with distance: the fog colour dominates beyond two metres, the dim cube map reflection gives the green hue). The
standard deviation of the regions (ripples, flecks) is 3 to 14 against the real 2 to 14. Fleck coverage above luma 85: 4.4 % against 3.8 %
(c05 bottom right), 7.4 against 7.9, 4.8 against 5.4. The player's legs are seen through the surface in the client
(`shots/water/cli_m1.png`, before: `cli_base.png`, `cli_a1.png`). The body and the cube map scale (`water_body`, `water_env`) trade against each
other (the fit is flat along the line body 4 / env 0.12 to body 8 / env 0.06); the fitted pair keeps the near and far regions both within a few
display values.

**Sea**: the same material (style `<sea>`, bigger and faster waves) on a plane at `ocean_level` following the camera; its colour is the zone's
`<Ocean FogColor>` x `sea_body`, its own knobs (`sea_absorb` 6, `sea_refl` 0.5, `sea_sun` 3) keep it exactly as before the pond fit (the
analytic sky and the screen space reflections, no foam; mean abs pixel difference of `fly --pos 15000,10000,200` before / after: 0.007).

### Native authored volume UVs and wave motion

Exact 1.2 `_LoadWaterVolume` **301F6AB0** passes serialized UV scales at offsets
95/99 and river U begin/end at 87/91 into `CWaterVolumeRenderNode` vtable
**3025F730**. Slot +312 is **300E85A0 CreateArea**: each outline vertex uses
`(point.xy-firstPoint.xy)*UVScale.xy`. Both scales and the first-point anchor
matter; the spawn pond's `.03/.02` was previously replaced by global `.2` world UVs.
Slot +316 is **300E9320 CreateRiver**: four bank vertices plus their centre,
native indices `0,1,4 /1,3,4 /3,2,4 /0,4,2`. It builds inward vertical planes for
opposite end/bank pairs. The UV ratios are signed plane distances
`a/(a+b)` when `abs(a+b)>=FLT_EPSILON`, otherwise zero. U interpolates
**absolute** serialized begin/end then multiplies scale X; V multiplies scale Y.
The centre retains the native plane-ratio result and its own four-triangle
interpolation, rather than using two guessed rectangular triangles. Degenerate,
nonfinite or non-four-point river surfaces are declined. Existing polygon
triangulation and horizontal-plane support remain the boundary for area geometry;
the native long-edge/triangle subdivision is not reproduced.

Original `WaterSurfaceVS` uses the mesh UV directly. `OceanGenParams.x` is explicitly
`PB_time .0125`; drift is `(seconds*.0125*WavesSpeed,0)`. With `d` as this drift,
the default tap coordinates are `4uv+2d`, `2uv+3d`, `8uv+2d`, `16uv+3d`.
`watersurface.ext` sets **NO_FLOW=0x200**: that branch instead uses `2uv+2d`,
`3uv-2d`, `4uv+2d`, `5uv-2d`. NO_FLOW still animates the counter-drifting pairs;
it is the material generation flag, not the volume's river/lake kind. Original
`lake_tank` enables it; `valley_river_watervolume1` does not.

`WaterSurfacePS` samples bump texture **raw XY**, not `Common.cfi::GetNormalMap`.
Qualifying WaterSurface/BUMP_MAP styles now retain the original ATI2 blocks without
aa-cry's semantic XY swap. Normal texture cache keys distinguish that raw input
from the sea's existing semantic input. Big/small scales and BumpScale remain the
authored values. Unknown/new-water shaders and the sea keep their existing sampling
path. Fog, reflection, exposure, foam, body/light gains and inherited calibration
hunks were not adjusted. Sloped-water
geometry, physical flow/current input and the complete native water shading/composite
contract remain open; this slice implements authored UV/wave inputs only.

The existing two placement jobs prepare these meshes. Rivers add only their one
native centre vertex; the view streams six water-style meshes /1297 vertices over
the original river/neighbour cells. No per-frame mesh work or per-cell texture
allocations are added. `WaterSurface { cell }` is an additive inspection marker;
`WaterMaterialPlugin` exposes the existing shader to the isolated aa_dev fixture.
Actual unload drops water entities/mesh handles; the shared material/normal/foam/cube
cache retains its existing world-resource lifetime, explicitly not a claim that
all shared style images are freed per cell.

**164 library tests pass /9 existing ignored**, and all examples check. Original
fixtures verify the >100 river records in cell9,12, serialized U range/scales,
endpoint UVs, native five-vertex fan, degenerate decline and the spawn pond's
independent first-point-relative scales. Material tests distinguish NO_FLOW from
volume kind and preserve the sea path. `water_motion` compares four production GPU
cases against independent bilinear sampling of original DDS bytes: river flow /
pond NO_FLOW at seconds0/4. Inspected `native_uv_motion_numeric.png` yields
`[189,255,196]`, `[200,254,193]`, `[181,254,172]`, `[176,254,186]`, within one display
value of CPU controls; 238-300 FPS.

Inspected `native_river_uv_on.png` / `_legacy.png` show the same source-backed river
camera, six style meshes /1297 vertices, zero pending/failed models; 158-179 FPS
native /161-191 legacy control. `native_pond_uv_on.png` / `_legacy.png` show the
original spawn pond, one mesh /8 vertices; 149-192 FPS native /154-194 control.
The legacy control changes only UV/motion sampling and retains the new raw normal
texture, geometry and current lighting; it is not a full old-binary comparison.
The river is mostly volume-fog dominated in this view, so its still-image difference
is subtle. `native_water_unload.png` verifies all **six mesh asset IDs and all
water entities** absent after actual camera movement; 180-233 FPS after release.

`native_pond_reference05.png` / `reference10.png` use the original combat camera
positions/yaws/FOV and recorded hours2.80975842/3.76261711, shifted to main-world
coordinates. Both native references were inspected. Twelve fixed 48px clear-water
patches at X550/1500/1650/1800 and Y720/850/940 gave mean RGB patch errors
**6.8 /9.4 display values** at that milestone; this is a different sampling grid from the historical
fit, not a before/after improvement metric. Wave phase, full scene/vegetation,
refraction and reflection shading still differ. No values were tuned to hide those
differences. Reference captures run145-201 /132-199 FPS. All eight visual runs
exit zero without GPU errors and close their windows.

Ignored source evidence is `lights/water-create-native.txt`, `water-vtable.txt`,
`terrain-water-next-audit.txt`, `water-next-inputs.txt` and the original
`watersurface.cfx` / `watersurface.ext`. `water-native-*.log` holds checks/captures;
`water-native-reference-patch-means.txt` retains every comparison patch mean.

### Native river endpoint opacity and counted outlines

Exact `_LoadWaterVolume` **301F6AB0** reads byte40 bits2/4, equivalently
`kind & 0x200/0x400`, and passes begin/end booleans to **300E9320 CreateRiver**.
The native 24-byte vertex is position / RGBA8 / UV. Both vertices at the selected
end receive alpha zero; other bank vertices and the centre remain255. The four
five-vertex alpha patterns are therefore `[1,1,1,1,1]`, `[0,0,1,1,1]`,
`[1,1,0,0,1]`, `[0,0,0,0,1]`. The centre matters: fading is not a single linear
ramp across the entire quad. The existing style grouping remains shared; endpoint
flags live in each mesh's vertex colors, not extra per-volume materials.

The source also exposed an actual omitted-geometry bug: `parse_water` previously
walked the render array into the separate physics outline until an approximate
position check stopped it. Some real four-bank rivers became larger outlines and
were declined by the exact five-vertex producer. Parsing now obeys the **render
count@103**; the separate physics count@115 only bounds the complete serialized
record. Checked arithmetic and lengths reject truncated or malicious counts.
This restores three river sections /15 vertices in the chosen streamed view.
It also prevents physical-outline vertices from polluting water queries.

Original `WaterSurfaceVS` copies authored alpha `a` to RGB and separately writes
`a*(1-saturate(1/clipW))` to alpha. These are interpolated **before** the PS reads
`sqrt(saturate(alpha))*red`; the new static-water vertex stage follows that order.
The near fade uses projection W, not Euclidean distance. Independently,
`watervolume.cfx::WaterVolumeIntoVS` passes the original alpha unchanged and the PS
writes `1-a+T*a`, where `T=exp2(-density*rayDepth)`, with InvSrcAlpha/SrcAlpha
blending. Fog therefore fades linearly; surface opacity has the nonlinear and
near-camera factor. Multiplying the entire old composite by one alpha would lose
that distinction.

The current combined fog/surface material folds both source factors into its
existing opaque-scene composite: linear alpha changes fog transmission, while
surface alpha blends from that unshifted fogged background to the existing
refracted/reflected/foam result. It retains the same render phase and does not
add blended sorting or draw calls. Qualifying WaterSurface/BUMP_MAP styles use the
native channels; the sea and unknown shaders retain their prior path. Native
soft-intersection/full BRDF and separate native render-pass behavior remain
existing approximations; this is not a claim of complete water compositing parity.
Material gains, exposure, inherited mip edits, sloped geometry and physical
currents were not changed. There is no per-frame mesh work, no new images or
materials, and one16-byte vertex color per water vertex. The same two placement
jobs and cell asset/entity removal own the lifetime.

**166 library tests pass /9 existing ignored**; all examples check. Actual cell9,12
has149 water records, including four flagged river ends. Tests verify actual begin/
end bank colors and the unchanged centre, both-ends controls, independent counted
physics/render arrays, truncated payloads and maximum hostile counts. Production
GPU controls use alpha0/.25/.5/1 and projection W=.5/1/2/8; all eight RGB triplets
match independent CPU references exactly. The full real depth-prepass + scene-copy
fog/reflection composite yields `[63,89,124]`, `[107,117,120]`, `[125,151,122]`,
`[99,213,146]`, within one display value of CPU controls. Numeric captures run
283-400 FPS; the existing original-DDS wave regression remains exact within one
value, 376-422 FPS.

Inspected `native_endpoint_river_on.png` / `_opaque.png` use original cell9,12
at Cry camera `(9268,12755,205)` looking at `(9239,12785,189.4)`, hour10. The
authored endpoint blends into the waterfall lip; the all-one-alpha control exposes
its hard rectangular edge. Both retain six style meshes /1312 vertices and zero
pending/failed models; native156-204 FPS /control140-205 FPS on the shared GPU.
No original-client river capture is available, so this comparison proves the
source-backed control difference rather than a calibrated whole-scene match.
`native_endpoint_unload.png` verifies all six mesh IDs and all water entities gone
following camera movement; after release215-246 FPS. Shared style images retain
their existing cache lifetime.

The isolated example now explicitly enables Bevy0.20 `ScreenSpaceTransmission`,
as the integrated in-game camera already does; older isolated world captures used
the shader's fallback body color. `native_endpoint_reference05.png` / `reference10.png`
repeat both original pond camera/hour captures with that actual scene copy and the
new native near fade. Both originals were inspected. The same twelve48px water
patches give **6.2 /9.4** mean RGB errors; reference runs174-217 /167-209 FPS.
Full vegetation/actors/reflection/refraction and wave phase still differ, and
neither exposure nor authored lighting was tuned. All nine final runs exit zero,
close their windows, and report no GPU errors. Evidence: ignored
`lights/water-endpoint-source.txt`, `water-endpoint-inputs.txt`,
`water-endpoint-reference-means.txt`, and `water-endpoints-*.log`.

**Water queries** (`water_query.rs`): `aa_world::WaterQuery` (a `Resource` the plugin inserts next to `TerrainQuery`, a clonable `Arc` handle
like it, `&self`, `Send + Sync`) answers "is there water here":

```rust
let water = WaterQuery::new("main_world");                          // or Res<WaterQuery>
water.water_at(Vec3::new(x, y, z))   // Option<(f32, WaterKind)>: surface height (m) and Sea / Lake / River, game coordinates
water.body_at(pos)                   // Option<WaterBody { height, kind, fog_color, fog_density }>, the volume's own fog too
```

A volume (`object.dat` type 11: outline, surface, depth) applies when the outline contains `(x, y)` (even-odd test of the same ring the mesh is
built from) and `z` is not below its floor (`surface - depth`); the highest surface wins, a volume beats the sea. The sea is the plane at the
world's `oceanLevel` (100; none in login2) wherever the terrain is lower (a cell without terrain inside the map is open sea). `z` may be above
the surface (a jump, a bridge: compare it with the height to see whether the feet are wet). Cells (`object.dat`, a few ms) are parsed on first
use and cached (64, least recently used). `None` on dry land and outside the map. Examples (tests): the Nuian pond `(15577.3, 15379.9, 126.25)`
-> `(127.0, Lake)`, a river in cell (9, 12), the map corner `(100, 100)` -> `(100.0, Sea)`.

Not done: flow along rivers, caustics, waterfalls (`WATERFALL` flag), planar reflections (some real materials use realtime reflections), the
`Foam`-less materials (gen mask bits are not read: every volume with foam parameters gets foam), `TransparencyRatio` above 1.

### Streaming (`stream.rs`)

`stream_cells` picks cells whose square is within `load_radius` (3000 m) of the focus (the entity with `WorldFocus`, else the first
`Camera3d`) and starts `load_cell` on the `aa_cry::workers` threads (max 3 at a time, nearest first, ahead of everything else the workers
do; Bevy's async compute pool is where the render pipelines compile, and behind a first frame full of shaders it was busy for seconds): read `heightmap.dat` and `cover.ctc`, build the atlas
and all 16 patch meshes at the likely level of detail. A channel returns them; `receive_cells` moves at most `spawns_per_frame` cells into
the world per frame. Cells farther than `unload_radius` (3600 m) are despawned. Failed loads (no terrain, e.g. empty ocean) are remembered.

## 4. API

```rust
app.add_plugins(aa_world::AaWorldPlugin::new("main_world"));   // adds aa_cry::CryPlugin when absent
// the camera and window are the application's; the first Camera3d (or the entity marked WorldFocus) streams the world
```

* `AaWorldPlugin { config: WorldConfig }`; `WorldConfig` fields: `world`, `load_radius`, `unload_radius`, `max_loads`, `spawns_per_frame`,
  `lod_updates_per_frame`, `objects`, `vegetation`, `brush_distance`, `vegetation_distance`, `tree_distance`, `water_distance`, `object_budget_ms`,
  `finish_budget_ms` (main thread time per frame for inserting prepared models, 1 ms; the loading screen raises it), `environment`, `post`, `sky_radius` (the camera far plane must exceed it), `shadows`, `shadow_distance`, `sun_lux_scale` (1000), `ambient_scale`
  (1700), `fog_visibility`, `terrain_tint`, `terrain_detail`.
* Resources: `WorldClock` (time of day: `hour`, `speed` in game hours per second, `zone` to force one, `layer` of the zone files), `Environment` (the
  applied sample, `ocean_fog`), `AtmosphereTuning` (`AA_TUNE`), `WaterStyles` (materials by style), `TodCache`,
  `WorldInfo(Arc<World>)`, `Streamer::loaded_cells()`, `ObjectState` (`stats`), `WorldTimings` (per frame main thread ms
  of the streaming systems), `pipelines::PipelineStatus` (render pipelines waiting / requested, frames of the render world; for loading screens), `TerrainLayers` (the layer library of the world, loads on a task at startup; cells wait for it),
  `TerrainQuery` (ground queries, see below), `WaterQuery` (water height and kind, see Water), `WorldCollision` (ground, rays, camera clipping and wall sliding against the brushes and the terrain, see Collision with brushes). Markers: `WorldFocus`, `SunLight`, `SkyDome`, `OceanPlane`, `TerrainPatch`, `CellRoot`,
  `ObjectSector`.
* Coordinates: `cry_to_bevy`, `bevy_to_cry`, `cell_origin`, `cell_of`, `CELL_SIZE`.
* Parsers (no Bevy world needed, all tested against the pak): `heightmap::CellHeights::parse`, `cover::Cover::parse` (+ `atlas`),
  `placement::{parse_brush_dat, parse_vegetation_dat, parse_vegetation_sector, parse_object_dat, parse_visareas_dat, parse_water, load_cell_objects}`,
  `vegetation::VegGroups`, `world::World` (`load`, `zone_at`), `env::{TimeOfDay (parse, parse_layer), Lighting, sun_direction, EnvSample::lerp}`, `sky_material::{ZoneSky, load_sky_box}`, `sky_clouds::{parse_clouds, CloudDef, CloudMaterial}`, `post::{Post, PostUniform}`,
  `cinema::CinemaCamera` (`load`, `pose(t)`), `water::{water_mesh, water_meshes, WaterStyle, triangulate}`, `water_material::{WaterMaterial, WaterStyles}`, `water_query::{WaterQuery, WaterBody, WaterKind}`,
  `terrain_layers::{LayerLibrary::build, parse_surface_types, TerrainMaterial}`.

### Ground queries (`terrain_query.rs`)

`aa_world::TerrainQuery` answers "where is the ground" for gameplay code (the player controller of `aa-client`, which keeps its own
`heightmap.dat` cache today, should use it instead). It is independent of render streaming: it reads the `heightmap.dat` files itself, needs
no window and no `AaWorldPlugin`, and is a clonable `Arc` handle (`&self` methods, `Send + Sync`, usable from tasks) and a `Resource`
(`AaWorldPlugin` inserts one for its world unless the app has one: `Res<TerrainQuery>`).

```rust
let terrain = TerrainQuery::new("main_world");                    // or Res<TerrainQuery>
terrain.height_at(x, y)         // Option<f32>   game metres: x east, y north, as the server sends them
terrain.ground_at(x, y)         // Option<Ground { height, normal }>, one cell lookup (normal: unit, up = +Z)
terrain.normal_at(x, y)         // Option<Vec3>   terrain.slope_at(x, y)  // Option<f32> radians, 0 flat, pi / 2 wall
terrain.prefetch(x, y, 600.0)   // load the cells around a position on a background thread (call when entering a cell)
terrain.cell(cx, cy)            // Option<Arc<CellHeights>>: the raw grid, e.g. for surface type nibbles
```

* The surface is the finest mesh the renderer draws (`CellHeights::mesh_height` / `mesh_normal`: 2 m quads split from the south west to the north
  east corner, linear over the triangles), so feet and ground agree where you look. It differs from a bilinear interpolation of the four
  corners (what `CellHeights::sample` and the client's old cache return) by up to 0.8 m on steep faces and a few cm on meadows. `normal_at` is
  the face normal of that triangle, not the smoothed shading normal.
* `None` outside the map: negative or non-finite coordinates, a cell that does not exist or fails to parse (`warn!` for the latter).
* Cost: a cell is loaded on first use (2.6 ms: pak read + parse; the very first query of a process also opens the pak, 0.3 s if nothing else
  did), then a query is about 0.1 microseconds. At most `TerrainQuery::DEFAULT_CACHE_CELLS` (48, 1.3 MB each) parsed cells are kept (least
  recently used dropped; `with_capacity`); concurrent first queries of a cell load it once. Missing cells are remembered as `None`.
* Not included: terrain holes (`hasHoles` is parsed over, none known in 1.2 data), water (use `WaterQuery`, see Water),
  and the surface type (the nibble is in `CellHeights::surface`, its meaning in the cell's `leveldata.xml`, see `terrain_layers::parse_surface_types`).

### Collision with brushes and vegetation (`collide.rs`, `collide/`, `aa_cry::physics`)

`aa_world::WorldCollision` (`aa_world::collide`) is the collision of the world's brushes (buildings, rocks, bridges, walls, stairs), vegetation
(tree trunks, rocks and plants with physics), and terrain, for the camera and the player controller of `aa-client`. Like `TerrainQuery` it is a clonable `Arc` handle (`&self` methods,
`Send + Sync`) and a `Resource` (`AaWorldPlugin` inserts one for its world unless the app has one; build your own with
`WorldCollision::with_config(world, terrain, CollisionConfig { .. })` and insert it before the plugin to change the settings), in game
coordinates (metres, x east, y north, z up).

```rust
let world = Res<WorldCollision>;                                   // or WorldCollision::new("main_world", TerrainQuery::new("main_world"))
world.request_around(x, y, 150.0);                                 // the plugin does this for the camera / WorldFocus (follow(), 4 Hz)
world.ready_around(x, y, 60.0)                                     // bool: every cell in that radius is built (loading screens, the first step)

// camera: the arm from the pivot above the head towards the wanted camera position
world.clip_camera(pivot, wanted, 0.3) -> Vec3                      // wanted, or short of the wall / ground (Mask::CAMERA, sphere of 0.3 m)
world.sweep_sphere(origin, dir, max, radius, mask) -> Option<Hit>  // t = how far the sphere centre gets, point / normal = contact
world.raycast(origin, dir, max) -> Option<Hit>                     // terrain + all brushes; raycast_mask(.., Mask::CAMERA | ALL | STRUCTURES | PROPS | TERRAIN)
// Hit { t, point, normal (faces the origin), terrain: bool }

// player: stand on bridges and stairs, slide along walls
world.ground_below(x, y, z, step_up) -> Option<Ground>             // the highest walkable surface not above z + step_up, terrain or brush
world.slide(from, Vec2 delta, radius, height, step_up) -> Pushed   // horizontal move of a standing capsule, sub-stepped, slides along walls
world.push_out(feet, radius, height, step_up) -> Pushed            // the same without the move: Pushed { pos, blocked, normal }
world.surface_below(x, y, z, step_up) -> Option<Standing>         // ground + Surface + terrain flag, for footsteps / dust
world.terrain_surface(x, y) -> Surface                           // terrain layer alone; mat_default until the cell is built
world.raycast_surface(origin, dir, max, mask) -> Option<(Hit, Surface)>
world.triangles_near(center, half_size, mask) -> Vec<[Vec3; 3]>    // for debug drawing (examples/collide.rs)
```

A controller frame is `let m = world.slide(pos, wish_dir * speed * dt, 0.35, 1.8, 0.5);` then
`let z = world.ground_below(m.pos.x, m.pos.y, pos.z, 0.5).map_or(<fall>, |g| g.height);` and `pos = (m.pos.xy, z)`; the camera is
`world.clip_camera(pivot, wanted, 0.3)` every frame.

* **Semantics.** `ground_below` finds the highest surface under the point that is not above `z + step_up` (the feet height and the tallest
  step), so a bridge deck 20 m over the water is invisible to someone on the shore and found by someone on the deck; a surface counts as
  ground up to `CollisionConfig::max_slope` (55 degrees), both faces of a triangle; the terrain counts at any slope. The feet are a disc
  (`foot_radius` 0.4): where nothing is under the centre the rim is probed, because the proxies of neighbouring pieces leave gaps (0.7 m
  between a quay and the bridge deck at Lutesong Harbor) that would drop the character through; `None` when everything under the point is above
  the limit (inside the ground: use `world.terrain().height_at`) or outside the map. `push_out` / `slide` use a vertical capsule as a
  chain of spheres at most one radius apart, starting `step_up + radius` above the feet (so curbs and steps below `step_up` never push), and
  only change x and y; terrain slopes are the controller's rule (the terrain is not a wall here). Thin walls cannot be crossed: `slide` pushes out
  after every half radius of the move. A capsule that starts inside a model is pushed out by at most `2 * radius + 0.5`.
* **Masks.** Brushes are split by the size of their bounding box: `Mask::STRUCTURES` (houses, walls, rocks, bridges, boats: 4 m^3 and
  more, `prop_volume`) and `Mask::PROPS` (barrels, crates, fences, lamp posts). Vegetation is `Mask::VEGETATION`; instances whose proxy is at
  least `vegetation_camera_width` (2 m) wide on both horizontal axes also count as structures. `Mask::CAMERA` = terrain + structures:
  the camera passes small trunks and barrels, stops at boulders and wide trunks; the player collides with `Mask::ALL`.
* **Physical surfaces.** Every proxy triangle carries its sub-material id from `aa_cry::physics::CollisionMesh.materials`; its model `.mtl`
  and the placement's material override supply `SurfaceType` (`mat_wood`, `mat_rock`, `mat_sand_dry`, ...). Terrain uses the nearest
  heightmap sample's surface nibble and the cell's `leveldata.xml` detail material. `Standing.surface.name()` is the column name in
  `materialeffects.xml`, and `.kind()` is a coarse class for effects. Unknown materials return `mat_default`. `nocollide` materials and
  the `no_collide` surface types in `surfacetypes.xml` (canopy, grass, leaves) do not stop the player; `mat_obstruct` is AI-only.
* **Not loaded yet.** A cell that is not built answers with the terrain only. `WorldCollision::follow(x, y)` (the plugin calls it every 0.25 s
  for the `WorldFocus` entity, else the first `Camera3d`) requests the cells within `radius` (150 m) and drops those beyond `keep` (400 m);
  `radius <= 0` switches it off (character select), `AA_NO_COLLISION=1` leaves the plugin's system out. The first cell of a process takes
  0.1 to 0.4 s (pak open, models), so wait for `ready_around` before putting the player into a town.

**Where the collision is.** `.cgf` files keep it as serialized CryPhysics geometry, not as triangles: `MeshPhysicsData` chunks (0xCCCC0018),
referenced from the four `physicsChunkId` of a mesh chunk (after its 16 stream ids). Slot 0 collides, slot 1 is "no collide" (the `$picking`
cylinders), slot 2 "obstruct" (a few models; not used). The geometry hangs on the node of a `proxy` (an empty mesh with only the physics, as in
`si_woodbridge_01`: one box) or on the render node (`Merged`, the model's own name: `si_shop_g`, 206 triangles), and a model can have several
(`daru_house`: 14). `aa_cry::physics` parses the blob (format in its module documentation; `D:\aa-re\zone\agent_geom\sub_cgf.md` has the engine side) and
returns every node's slot 0 geometry transformed by its node chain as one triangle list: trimeshes as they are, boxes as 12 triangles
(`basis` rows are the box axes, the sizes are along them), cylinders as 12 sided prisms, spheres as 8 x 3 meshes, capsules as prisms. Facts found
on the way: the trimesh header is `u32 nVerts, u32 nTris` (vertex arrays are often longer than the triangles use); 341 trimeshes share the
render mesh instead of storing their own (`foreign` index without flag 0x400000); box `basis` is a row matrix. Of the 162,386 brushes
of the main world, 151,004 (93 %, 3446 models) have physics geometry, 11,339 (367 models) have none (ivy, flags, laundry, straw, seedlings,
moss, waterfalls, spiderwebs, the floating cube parts of the air library; the real client walks through them too), 35 an empty one, 8 a missing
file; none of the 3826 models fails to parse. `CollisionConfig::visible_fallback` collides such models as their coarsest visible mesh
(`aa_cry::physics::collision_mesh(bytes, true)`); off by default. The physics triangles of all distinct models together are 1.1 M.

**Build.** Per cell, on the `aa_cry::workers` threads (`WorldCollision::request`): `brush.dat` + `statobjs.dat` (else the `object.dat` octree, as
`load_cell_objects`), plus the interior `visareas.dat` brushes and supported first-LOD voxel proxies. Each distinct model's proxy is loaded once and shared by all cells (a `OnceLock` per model, kept for the process: 3.2k models,
tens of MB), the brush matrix and the cell origin applied (mirrored matrices flip the winding), the triangles split into structures and props
and each indexed by an 8 m grid over XY (`collide/soup.rs`: CSR rows of triangle ids, big triangles only in the cells they touch, ground
candidates first in every row). Vegetation stays instanced: `collide/instances.rs` shares model-space triangles, keeps each placement in 24 bytes,
and indexes the placements in a 16 m grid. Queries transform nearby trunks' triangles as needed, so a forest does not expand every tree to a
triangle soup. In Lutesong Harbor 1,013 of 10,957 vegetation placements collide (265 wide enough for the camera), with 0.05 MB of instance data;
Marianople has 1,348 of 10,439, 429 wide, 0.06 MB; forest cell (12,14) has 732 of 29,770, 660 wide, 0.06 MB. Small grass/bushes without proxies
are traversable. `CollisionConfig::vegetation = false` omits vegetation. Measured before vegetation on the 140 cells with brushes of the middle of the
world (dev profile, one thread, models cached): build median 29 ms, 90 % 112 ms, worst 221 ms; memory median 6.8 MB, 90 % 24.5 MB, worst 69 MB
(cell 14,11: 1.7 M triangles, 1400 instances of two rock cover pieces of 800 to 960 triangles, `cross_plains_102b/c`; a dense town cell such as Lutesong Harbor or Marianople is 21 to 27 MB for 500 to 640 k
triangles at 42 bytes each); the first cell of a process 100 to 400 ms with cold models and pak. A cell is dropped with `release_outside`, so a
walk keeps one to four cells (a few tens of MB).

**Queries** (Lutesong Harbor and Marianople, 2000 random calls near the player, dev profile with `opt-level = 1` for this crate; release is the same
within noise): `raycast` of 10 m (terrain + brushes) 6 to 7 microseconds, with `Mask::CAMERA` 5 to 6, `sweep_sphere` of 10 m (r = 0.3) 6 to 9,
`ground_below` 3, `push_out` 2.4, `slide` of 0.2 m 4 to 5. Rays walk the grid cells they cross (2D DDA) and test the triangles
(Moller-Trumbore, two sided); the terrain is marched in 1 m steps and bisected (the same triangles as `TerrainQuery`); the sphere sweep is a
conservative advancement against the few triangles near the ray (point to triangle distance), refined by bisection; the terrain under a sphere is
a plane (`z = h + r / n.z`). With the plugin following the camera the main thread cost is the 4 Hz `follow` (microseconds) and the cell builds
run on the workers. fps: `fly --bench 15 --bench-speed 5 --no-shadows` at Lutesong Harbor (14.5 k part entities), alternating runs with the cells built and
with `AA_NO_COLLISION=1`: 128, 136, 151, 141, 137, 152 fps (and one run of 63 while another session loaded the machine) against 132, 152, 148, 150, the
slow frames are not in the streaming systems (`WorldTimings` under 1 ms); the `collide` example at 1280 x 720 with 20 k wireframe triangles drawn over the
scene (retained gizmo): 188 and 190 fps against 184 and 195 without.

**Verification.** `cargo test -p aa-world --lib collide` (geometry, vegetation placement/rotation/masks, soup against brute force, terrain rays,
Lutesong cell, camera at a wall, a walker over the arch bridge in both directions, the two embankment stair ramps, wood/stone/terrain surfaces,
an exposed tree trunk approached from eight sides, and 120 000 walking steps in two towns with two detected crossings), `cargo test -p aa-cry --lib physics` (primitives, a blob, real
models); `-- --ignored --nocapture` for `survey_world` (the counts above), `survey_cells`, `query_costs`, `find_spots`, `find_camera_spots`,
`climb_the_stairs` (an exploratory survey of routes inferred from placement centroids, not proof of spiral/ladder traversal), `survey_vegetation`,
`survey_surfaces`. The embankment proxies' low vertices include their underside, so the survey previously started below the slab and reported
false failures; both actual ramp routes climb with a 0.5 m step, radius 0.35, and less than 0.3 m rise per 0.1 m walking step. Visual: `cargo run -p aa-world --example collide -- --player 21140,10596.2 --yaw 244 --arm 7 --pitch 14
--shot shots/aa-world/collide_a_clip.png --shot-after 14` draws the collision triangles near the player as a wireframe over the scene (cyan
structures, orange props, pink vegetation) and the camera arm (yellow wanted, green clipped, red contact; `--no-clip` shows the camera inside the wall, `--cam x,y,z
--look x,y,z` a free camera, `--xray` the wireframe over everything, `--no-objects` collision only, `--bench n` times the queries). The wireframes
sit on the visible walls, roofs, awnings, railing posts, hulls of boats and carts (`shots/aa-world/collide_a_clip.png`, `collide_a_noclip.png`,
`collide_bridge.png`, `collide_stairs.png`).

Completion audit: the collision group has **22 passing tests, 9 ignored surveys, 0 failures**. The real exposed birch at
`9265.508,13333.971,261.663` stops eight walking approaches 0.69 to 1.15 m from its centre; narrow-trunk camera rays pass it. The fixture finds
the trunk's chest-height cross section because some models' pivots are offset and many vegetation placements are buried in slopes.
`shots/aa-world/exposed_tree_collision.png` shows its pink proxy following the visible trunk and leaving the leaves clear (aa-dev 173 to 196 FPS
after loading, final screenshot interval 167 FPS); `shots/aa-world/embankment_stairs_audit.png` shows the stair ramp proxy following the steps
(150 to 161 FPS after loading). There is no matching real-client capture at these coordinates; these checks compare collision against the
rendered original models. The broad town walk survey still detects two wall crossings in 120,000 steps and 2,222 residual pushes after
the controller raises its feet to a new support height; it is not a proof that the controller handles every contact or corner.

**Test coordinates (main_world, game coordinates).** Lutesong Harbor's `sub_zones` box (x 20510 to 21752, y 10222 to 10967) has its centre at
21131, 10594 (the gallery's `--pos 21130,10594`). A camera that ends inside a house wall: the player at 21140.0, 10596.2 (feet 111.17) with the arm
7 m long at 14 degrees pointing 26 degrees from +X (compass yaw 244 for `--yaw`): the arm is clipped at 1.26 m by a house wall; unclipped, the camera is inside
the building. More: player 21123.4, 10622.7 (feet 103.17, wall at 1.97 m), 21139.3, 10535.1 (107.34, 2.55 m), 21016.4, 10620.2 (82.51, 1.80 m).
Bridges: the stone arch bridge `si_bridge_b01` at 21091.0, 10551.2 (water at 89.7, deck 113.8 at the crest, 104.0 at the ends; axis along 0.98, 0.19;
walk from 21055.7, 10544.3 to 21126.3, 10558.1), the wooden bridges at 21015.3, 10544.9 (deck 121.5 over terrain 118.7) and 21109.4, 10591.8 (brush z 101.0), the Marianople drain bridge at
11023.1, 11751.5 (deck 120.5 over terrain 112.3). Stairs:
`si_woodbridge_stairs02` at 20999.4, 10586.3 (rise 1.7 m over 1.8 m), `mari_stair_a` at 10855.9, 12053.0, `ncom_city_old_stair06` at 10946.3, 11991.6
(rise 6.7 m over 10 m), `ncom_city_old_stair07` at 11078.3, 12083.0 (rise 9.3 m over 12.9 m), the dock stairs `si_seahouse_dockstair01` at 20567.5, 10296.4.

**Not done.** Terrain steepness as a wall in `push_out`; doors, gates and the
moving brushes (ships, tram) which are not in `brush.dat`; physics material flags (`RAYCAST_PROXY`, `COLLISION_PROXY`, `NO_PHYSICALIZE`;
the surveyed proxy sub-materials have none); gaps wider than 0.7 m
between pieces drop the character (as they would a real capsule); proxies that are a solid hull keep the player out of buildings whose interior
the real client lets you enter only through a separate interior model.

### Example `fly`

`cargo run -p aa-world --example fly -- <flags>` (plus the aa-dev flags `--shot <png> --shot-after <s> --exit-after <s> --size WxH`; the
window is forced to 1 physical pixel per pixel):

| flag | |
|---|---|
| `--world main_world / login2` | |
| `--pos x,y,z` `--yaw deg` (compass) `--pitch deg` `--look x,y,z` | camera in game coordinates |
| `--spawn nuian / elf / hariharan / ferre` | the new character spawn, default third person camera, FOV 42.75 |
| `--camera-json reference/<screen>/camera.json [--shift 11264,11264,0]` | the real client's camera (in game: client local, hence the shift) |
| `--cinema <name> [--t s] [--play]` | camera of `cinemas/<name>.xml` (default for login2: `01_world_select_e1`) |
| `--hour h` `--speed h/s` `--zone id` `--layer 0/1` | time of day: `h` is the game hour as the server sends it (default 10, layer 0); the splines are read at `spline_hour(h)` |
| `--sun-scale` `--ambient-scale` `--terrain-tint r,g,b` `--fog-visibility m` `--tree-distance m` | calibration |
| `--fx lib.group.name [--fx-dist m] [--fx-drop m] [--fx-scale x]` | an `aa-fx` effect in front of the camera (e.g. `abillity_skill_table_m.system_skill.return_skill_form`, the Return orb) |
| `--radius m` `--no-objects` `--no-shadows` `--no-env` `--no-post` `--no-terrain-detail` | |
| env `AA_TUNE="key=value,.."` | `AtmosphereTuning` overrides, see Environment |
| `--bench s [--bench-speed m/s]` | fly forward and print frame statistics, scene sizes and slow frames, then exit |

WASD / Space / Ctrl fly (Shift fast), right mouse button looks.

### Tests

`cargo test -p aa-world`: 36 unit tests (coordinates, heightmap / cover / brush / vegetation / object / water parsers against the real pak
incl. the spawn pond, BC1 round trip, env splines, sun path, cinema cameras, triangulation, the terrain layer library of the spawn cells and
the uv continuity of layers across cell borders, the BC1 to BC5 normal map transcoder, ground queries against the grid, the rendered
triangulation, cell borders, threads, the cache bound and prefetch; they skip when the pak is missing), and 12 collision tests (see Collision with brushes).
`cargo test -p aa-world --test all_cells -- --ignored --nocapture` parses every cell of both worlds (1205 + 3 cells, 162k brushes,
1.75 M vegetation instances, 3181 water volumes, about 5 s) and builds every cover atlas and water mesh.

Screenshots: `shots/aa-world/{nuian,elf,hariharan,ferre}_start.png` (`fly --spawn <race>`), `ingame_h12d.png` (the real in-game camera of
`reference/ingame/spawn`, compare with its `real.png`), `coast.png` (ocean at level 100), `login2_*.png` (cinema camera, real character
select / create cameras). Atmosphere: `shots/atmosphere/spawn.png` (`fly --camera-json ... --shift ...`, default hour 10) and `ingame.png` (`aa-client
--enter-world --pos 15578.042,15382.122 --yaw 158.74`, the player in the pond) against `reference/ingame/spawn/real.png`, `coast.png` (`fly --pos 15000,10000,200
--yaw 180 --pitch -8`, the sea). Terrain detail: `shots/terrain/spawn_before.png` (cover colour only, read as RGB) and `spawn.png` (layers) are
the camera of `reference/ingame/spawn` (`fly --camera-json reference/ingame/spawn/camera.json --shift 11264,11264,0`, compare with
`real.png`), `close1.png` (ground level, no objects), `coast.png`, `{elf,hariharan,ferre}_start.png`, `login2.png`.

## 5. Performance

Windows 11, Ryzen 5 5600X, Radeon RX 9070 XT, 1920 x 1080, workspace `dev` profile (`opt-level = 1` for this crate, 3 for dependencies),
shadows on (3 cascades, 300 m), `fly --bench`. Scene at the Nuian spawn: 36 terrain cells, 136 object sectors, about 4000 part entities, 7 water
cells.

| case | avg fps | 1 % low | worst frame |
|---|---|---|---|
| terrain only, no shadows, standing | 332 (3.0 ms) | 159 | 9.7 ms |
| everything, standing at the spawn | 120 to 145 | 50 to 83 | 16 to 26 ms |
| everything, flying 60 m/s | 173 to 179 | 88 to 94 | 16 to 23 ms |
| everything, flying 150 m/s (540 km/h) | 147 | 41 | 45 ms |
| elf spawn (dense forest, 12.8k part entities, 475 sectors), standing | 120 | 82 | 18 ms |

Terrain detail layers (`terrain_layers`), alternating A / B runs of the same machine state, `fly --spawn nuian --bench 15 --bench-speed 0
--no-objects --no-shadows` (terrain, water and sky only, standing; 3 runs each): cover colour only 195 / 195 / 209 fps (5.0 ms), with the
layers 185 / 178 / 199 fps (5.35 ms), i.e. the layer shader costs 0.35 ms per frame at 1080p (-6 %). The full scene (`--bench 30`, flying 80 m/s
from the spawn, shadows and objects on, 4 runs each) went from 132 / 120 / 120 / 129 to 142 / 89 / 115 / 121 fps (-6 % on average; the 89 was a
run while another session loaded the GPU). Start up: the layer library takes 0.3 s on a task (cells wait for it) and 163 MB of VRAM (186 + 53 + 1 diffuse
+ 255 + 4 normal BC textures; 130 MB before the big diffuse arrays); each cell adds a 263 KB surface map and a 3 KB table, no per frame CPU work, and the
draw count is unchanged (one material per cell). The big diffuse arrays do not cost frame time: `fly --pos 16776,8926.1,137.2 --pitch -10 --bench 8
--bench-speed 0` at Austera, old and new build alternating: 116 / 122 / 119 fps before the 1024 px array, 114 / 124 / 120 with it; the five arrays
against the 1024 px build: 122 against 121 fps (terrain only, `--no-objects --no-shadows`: 179 / 175 / 179 against 200 / 182 / 170 fps; the machine
was shared, single runs swing from 24 to 140).

Atmosphere (sky dome, post pass with depth prepass + HDR + bloom, water shader with screen space reflections), `fly --camera-json
reference/ingame/spawn/camera.json --shift 11264,11264,0 --bench 20 --bench-speed 0` (standing at the pond, 224 sectors, 4600 part entities, 7 water
cells), the build of the commit before against this one, alternating runs: 128 / 128 / 115 / 114 fps before, 113 / 112 / 111 / 97 fps after on quiet
rounds (about -12 %); noisy rounds with another session on the GPU gave 67 to 97 before and 61 to 93 after. The scene is CPU bound (about 8.5 ms
per frame), so the GPU passes (HDR target, depth prepass, bloom, fog / film pass) cost almost nothing by themselves; the water's ray march is the
visible part (`AA_TUNE=ssr=0`: about +10 %) and shrinks with the screen area of water (it is limited to 160 m and to two 16 step rays).

With the hemisphere ambient, the cloud sprites and (optionally) SSAO added, same view and method (best of alternating runs, the machine was
shared): 114 fps before the atmosphere work, 101 fps now (-11 %), 93 fps with `AA_TUNE=ssao=1` (-18 %).

The numbers move by a factor of two with the load of the machine: while another session's real client rendered on the same GPU the same scene
ran at 55 to 65 fps. Main thread cost of the streaming systems in slow frames (`WorldTimings`) is below 6 ms (`objects`); the rest of a slow
frame is render world work (uploading 3 MB cover atlases, extraction of the part entities). A release build of this crate would cut the CPU
share further.

### Release build, hitches and the in-game client (perf pass)

Everything above is the `dev` profile. The real measure is `cargo build --release`: `fly --bench 30` and
`aa-client --enter-world --pos 15578.042,15382.122 --yaw 158.74 --bench 20` (with `--bench-after S` to skip the first S seconds). The
shared machine made single runs useless (other sessions load the CPU and GPU: the same build ranges 70 to 110 fps), so the method is
`tools/perf.sh <scenario> <rounds> <label>=<dir> ...`, which runs the builds alternately (copies of `fly.exe` / `aa-client.exe` in
`target/perf/<label>/`) and prints the BENCH line of each run: average fps, 1 % low, worst frame, frames over 20 ms. Scenarios: `fly-full`
(80 m/s over the Nuian start), `fly-stand`, `fly-elf` (dense forest), `game-spawn`, `game-run` (W held), `game-town` (the 132 NPC town,
looking away from the crowd), `game-crowd` (looking at it: about 90 animated characters in view), `game-fill` (the crowd view measured from
the first frame in game: the town filling in). `EXTRA=--no-hud` removes the HUD (which belongs to another crate) from the numbers.

`ProfilePlugin` (`profile.rs`, `AA_PROFILE=1`, also in `aa-client`) logs every 3 s the main thread slices of a frame (update schedule,
extract gap, wait for the render thread), `cpu N Mcycles/frame` of the whole process (stable under load, unlike milliseconds), the longest
frames, the render thread stages, the `aa_cry::prof` scopes (thread CPU time and wall time per call site, the machine's clock in MHz), the
render passes' CPU and GPU time (Bevy `RenderDiagnosticsPlugin`) and the entity, visible and distinct mesh + material counts. `AA_OFFSCREEN=1`
renders into an image instead of the swap chain (no vsync and no window cap; the unfocused window of a background run is otherwise
limited to 60 Hz by winit unless `WinitSettings::continuous()` is set, which `profile::uncapped` does), `AA_PROFILE_THREADS=1` adds cycles per
thread, `AA_SAMPLE=<thread>` with `AA_SAMPLE_AFTER` / `AA_SAMPLE_SECONDS` runs `aa_cry::prof::sample_threads` (a sampling profiler over
dbghelp, symbols from the PDB next to the exe). `tools/perf-profile.sh <exe> <args>` is one profiled run, printed compactly.

What it showed: the GPU is not the limit (passes sum to 1.4 to 3.6 ms), and the steady state of every scenario is 100 to 130 fps with
the world's own cost small. What hurt was the main thread doing loads and per unit work in bursts:

* Model, texture and material loading ran on the main thread (`load_model` per placed model, `CryAssets::load_model` per unit part). Now
  `aa_cry::workers` (a third of the hardware threads, priority = distance) prepares them (`CryLoader::prepare_model`), `finish_model` (about 0.03 ms) runs
  on the main thread under a 1 ms budget, a sector spawns when its models are ready, a new level of detail is built hidden and swapped.
* Units and doodads of the client (`ingame/prepare.rs`): database rows, models, the composed face (`prewarm_character_face`, the face
  cache of aa-anim) and the clip set are prepared on a worker; the main thread only spawns the entities (about 10 ms instead of 45 per
  character). One shared `Rig` per skeleton file (`pose::shared_rig`), a cached clip binding per `(rig, clip)`, one parsed `.cal`.
* Animation: far characters update at 30 Hz (beyond 25 m) and 15 Hz (beyond 45 m) (`ANIMATION_LOD`, `AA_ANIM_LOD=0` turns it off), sampling
  is parallel; characters beyond 45 m (`AA_UNIT_SHADOW_RADIUS`) do not cast shadows. Looks identical in the screenshots (compare
  `shots/ingame/spawn.png` and `reference/ingame/spawn/real.png`; `shots/perf/vis/` has before / after shots of the spawn and the elf forest).

Release build, standing / running at the spawn and in the town (3 alternating rounds of 20 s, HUD off so that only this work is compared;
the first number is the median of the rounds):

| scenario | before | after |
|---|---|---|
| spawn, standing (avg fps, 1 % low, worst frame) | 109, 65, 25 to 52 ms | 108, 65, 23 to 24 ms |
| spawn, running forward | 115 fps, 1 % low 39, worst 133 to 136 ms | 115 fps, 1 % low 68, worst 21 ms |
| town (132 NPCs), looking away | 132, 84, 13 to 17 ms | 128, 81, 15 to 27 ms (equal within noise) |
| town, looking at the crowd (90 characters in view) | 70 fps, median frame 14.1 ms | 87 fps, median frame 11.4 ms |
| town fill (first 25 s after entering, crowd view) | 67 fps, 1 % low 8, 130 frames over 20 ms | 100 fps, 1 % low 37, 7 to 11 frames over 20 ms |
| `fly --bench 30` (80 m/s) | 151 fps, 1 % low 80, worst 29 to 34 ms | 163 fps, 1 % low 90, worst 17.5 to 20 ms |
| `fly --bench 20`, standing / elf forest | 108 / 99 | 108 / 103 |

With the HUD on (aa-client, `--enter-world`, the HUD agent's numbers are not mine): 98 to 101 fps at the spawn for both builds. The HUD
(`aa-ui` Lua frames) costs 4 to 6 ms of main thread per frame (`--no-hud`: update slice 1 ms instead of 6), the largest single item of the
client now.

Release profile: a `release-lto` profile (thin LTO, 4 codegen units) was built and compared with the plain `release` build, alternating
3 rounds each of `game-spawn` and `game-crowd` with `--no-hud`: spawn 107 / 95 / 97 fps against 110 / 105 / 96 (release), crowd 90 / 91 / 94
against 92 / 90 / 89, i.e. no difference outside the noise, while the link takes more than 20 minutes instead of 2. Not kept: the cost of a
frame is in Bevy's own crates (already optimised with `opt-level = 3`) and in waiting on the render thread, not in cross crate inlining.
The workspace `release` profile stays as it is (`debug = "line-tables-only"`, which the sampling profiler needs for names).

What still costs most, in order: the HUD's per frame Lua; one stall of about 200 ms in the first frames after entering (the first
skinned and water pipeline specialisations; the same in both builds); the render thread's per skinned mesh work in crowds (Bevy's
`extract_skins` walks all 157 joints of every visible part, so a crowd is tens of thousands of joint matrices per frame; compacting
a part's joint set to the bones it uses would cut it, not done: it touches `aa-anim::character` skinning); transform propagation of the
animated bones (0.9 ms for a crowd after the rate limit, 2.1 ms before); 3,000 or more distinct skinned draws in a crowd (merging the parts
of a character into one mesh per material would need the dye and face systems' cooperation). The Compute Task Pool threads of Bevy spin
(74 % of the process cycles, mostly idle polling); fewer threads (`AA_COMPUTE_THREADS` in `fly`) did not change the frame time.

### Entering the world (normal login flow)

The first frames of a world are expensive once: Bevy compiles the render pipelines (80 to 160 of them, about 1 s on this machine
however many threads it gets; they run on the async compute pool), the first skinned meshes, models and textures are uploaded. They used to
happen in the first frames in game (one frame of 90 to 215 ms, 1.3 to 1.8 s with the HUD build, and meshes popping in while the pipelines
compiled). Now the loading screen does them: as soon as the server's spawn is known `ingame::player::spawn_player` builds the character and
`ingame::camera::spawn_camera` the world camera, which renders into an image of the window's size, behind the loading UI;
`login::loading` waits until terrain cells, objects, models, the units near the spawn and the pipelines
(`aa_world::pipelines::PipelineStatus`: waiting and total counts of the render world's `PipelineCache`, shared through atomics) have stopped
changing for 0.4 s, then switches to `Screen::InGame`, where the camera is put on the window as soon as the UI cameras agree with it.
Terrain cells and placement files moved from Bevy's async compute pool (where the shader compilation starves them) to the `aa_cry::workers`
queue, ahead of the models: models were done after 1.4 to 2.2 s before, 0.5 s now. The loading screen raises the per frame streaming budgets
(`object_budget_ms` 10, `finish_budget_ms` 4) and puts them back when it ends.

`tools/perf-flow.sh <rounds> <label>=<dir> ...` runs the real flow through the original scripts (`--account bevycreate --goto loading`) and
prints the `flow:` lines of `ingame/flow.rs`; `CYCLES=1` adds `--leave-world 6 --again` (leave through the server, press Start again).
Release, 1920 x 1080, alternating runs of the build before (`before`: world built on the first game frame) and after:

| | before | after |
|---|---|---|
| no HUD: loading screen | 1.3 s | 2.4 s (4.9 s with a cold file cache) |
| no HUD: Start pressed to first in-game frame | 1.4 to 1.6 s | 2.4 s |
| no HUD: worst frame of the first 5 s in game | 113 to 215 ms (frame 2) | 21 to 45 ms |
| HUD (built during loading by the HUD): Start pressed to first in-game frame | 2.7 to 3.2 s | 2.4 to 2.7 s |
| HUD: worst frame of the first 5 s in game | 1.35 to 1.85 s | 39 to 50 ms |
| HUD: worst frame on the loading screen | 44 ms | 540 to 595 ms (the HUD stage build, 1.2 s of Lua before it moved) |

The machine was loaded by other sessions (300 to 580 frames in 5 s instead of 600), the worst frames in game are the noise floor. What the
loading screen waits for is logged (`loading: waited for (s): terrain, camera frames, models, units, pipelines`): terrain 0.3 to 0.6 s, models
0.5 s, units 0.4 s, pipelines 1.6 to 1.8 s, which is the critical path (more async compute threads do not shorten it: 3, 6 and 10 threads
all settle in 1.0 s in `fly`, which logs `render pipelines settled after ...`). Leaving the game (`OnExit(Screen::InGame)`) restores the UI
cameras the HUD had made HDR without MSAA (`hud::cameras`), which the login stage camera cannot share a window with (a wgpu validation
error that ended the app); five round trips in a row are clean and the character list looks the same as on the first visit
(`shots/perf/flow-charlist-first.png`, `flow-charlist-after-leave.png`: the only difference is the idle pose of the character).

### Prewarm on the character list

The loading screen's critical path was the pipeline compile (1.6 to 1.8 s), which does not need the player or the server, so
`ingame/prewarm.rs` does it while the player sits on the character list. One second after the list is built, the real world is
streamed in around the first listed character at a small footprint (`load_radius` 450 m, objects within 300 m, trees 300 m, water 500 m, 2 ms of
main thread per frame; terrain, objects, vegetation, water, sky and the particle effects through the real systems), the character is
prepared on a worker and spawned where the player will stand, and a camera renders it into a 96 x 54 image. That camera is the in-game
camera in everything that decides a pipeline (the lens, aa-world's `Post` with its HDR, no MSAA, prepasses, SSAO, bloom and tonemapping),
so the combinations compiled are the ones the game asks for; a mock scene with one object of each kind would miss the
material flags and vertex layouts that only the data decides, and the shadow and prepass variants of each. It ends when
`PipelineStatus` has stopped changing for 0.6 s (1.4 to 1.6 s on this machine: 117 pipelines, 72 of them are the login screens'), drops the camera and the
character and moves the anchor away so the world unloads, or at once when the player leaves the list (Start before it settled keeps the
world that is already there and whatever compiled stays in the pipeline cache). It runs once per process. `--no-prewarm` or `AA_PREWARM=0`
turns it off.

`tools/perf-flow.sh` with `GOTO=character-select EXTRA="--start-after S"` and `off=<dir>|--no-prewarm on=<dir>` (3 alternating runs, HUD on,
release; `--start-after S` presses Start S seconds after the list came up):

| Start pressed after | loading screen, off | loading screen, on | Start to first in-game frame, off / on | pipelines compiled on the loading screen, off / on |
|---|---|---|---|---|
| 6 s (list settled; 9 s and 3 s the same) | 2.1 to 2.3 s | 1.3 to 1.4 s | 2.19 to 2.30 s / 1.33 to 1.40 s | 61 / 17 |
| 0.5 s (fast click) | 2.1 to 2.8 s | 2.2 to 2.7 s | 2.1 to 2.8 s / 2.25 to 2.75 s | 61 / 61 |
| 1.8 s (prewarm cut short at 0.8 s) | 2.3 to 3.2 s | 2.7 s (5.8 s cold) | the same plus 0.05 s | 46 to 49 |

The character list's frames: nothing changes in the first second (the list's own build, 260 to 450 ms for one frame, unchanged);
after it, the idle list renders at 9 to 16 ms worst frame, and with the prewarm the worst frame is 25 to 39 ms (the frame that starts it and
the first frames of the new camera; no frame over 33 ms in 5 of 6 runs, two in the sixth), 9 to 14 ms in the second after the world is
dropped. The loading screen still waits 0.65 s of settle and hold, 0.5 s for models and units and 0.7 s for the last 17 pipelines (units:
creatures and NPCs the server announces, which the list cannot know). The first in-game frames are as before (worst frame of the
first 5 s 35 to 49 ms).

Persistent pipeline cache: wgpu 29 has `wgpu::PipelineCache` (Vulkan: `Device::create_pipeline_cache`, `get_data()` to save the driver's blob, `data`
to start from it), which would make the compile on the second start of the client near free. Bevy 0.20's `PipelineCache` creates every
pipeline with `cache: None` (three places in `bevy_render/src/render_resource/pipeline_cache.rs`) and has no hook to change that, so it
needs a patched or vendored `bevy_render`; not built.

What the pipeline key hangs on, found with `AA_PIPELINE_DUMP=<file>` (aa-world's `pipelines.rs` appends one line per requested pipeline,
descriptor included; sort and diff two runs): the camera's `EnvironmentMapLight`. A camera whose environment map is not on the GPU yet
has the view layout without `environment_map`, which is another key for every material, so `update_ambient` creating new cube maps
on every change of the light (1.5 % of a colour, which a moving server clock does all the time) compiled the whole set a second time (19
pipelines on the loading screen with the clock following the server, none with `--hour`, which hid it) and drew the world without
ambient light until they were done. `update_ambient` now writes the pixels of its two cube maps in place (`sky::AmbientCubes`, a
resource) and inserts the component only on a camera that does not have it yet, and it does so when the world focus moves to
another camera (the login flow's anchor camera holds it first; with a frozen clock the game's camera never got the ambient light at
all). The prewarm camera takes its `EnvironmentMapLight` from the same maps.

The world's time of day follows the server's game clock (`ingame::sync_clock`: `aa_net::state::WorldInfo::hour_now()`, extrapolated from its
last packet, in `Screen::Loading` and `Screen::InGame`; nothing until the server has sent it; `--hour H` pins it).

## 6. Known gaps

* Terrain detail, differences to the real client (`shots/terrain/spawn.png` against `reference/ingame/spawn/real.png`): the real picture has a
  cooler, less saturated grade (photofilter, film curve, exposure) and deeper shadows under bushes (SSAO is opt-in, see Post); the layers have no specular /
  gloss (`_sp` maps: the ground is fully rough); diffuse maps below 512 px and normal maps below 256 px are skipped (the layer then
  has no colour / the vertex normal; 4 diffuse textures are missing from the pak); the weights are bilinear
  over the 2 m samples at every distance while the engine interpolates over its (LOD) triangles;
  `terrain_tint` is a fudge for the missing exposure. Roads, forward decals and the deferred diffuse/opacity slice
  are rendered (see placement formats); native MRT decal normal/specular blending remains open.
* Water, differences to the real captures: current authored-UV/endpoint reference-camera patch means differ by about 6-9 display values (see Water);
  calibration still covers one pond at two hours (2.81 and 3.76, sun low in the north-east); other lakes and rivers only follow the engine's formulas with their own `.mtl` parameters
  (no reference capture: the river of cell (9, 12) is dark and nearly clear, the old build was milky white). The bed of the pond is the terrain
  owner's (the cover colour is a flat teal where the real bed is darker and greener), sloped surfaces/physical currents remain open, the sun glint of the real frames
  is not visible (the sun is low and to the left there), the sea has no foam / white caps or beach gradient (its colour is the zone's `Ocean
  FogColor` x `sea_body`, hazy at distance), no caustics, waterfalls, light shafts under water, refraction of the surface window seen from
  below or the transition at the surface line. The underwater colours are plausible, not fitted (no reference capture).
* Not rendered: native MRT normal/specular decal effects, voxel projected specular detail/atlases and type 14, unsupported CGA curves / named clips and skinned `entities.xml` render slots, indirect/cubemap local lights and the unsupported projector variants listed above.
  Interior supported placements render/collide through the existing paths, including the supported voxel base and projected diffuse/normal/detail-normal; voxel specular detail/atlases, distant clouds, portal occlusion, area sky/fog/light flags and light confinement remain unsupported.
  Hidden cinematic actors/targets remain aa-client's screen-flow responsibility. Also missing: original cloud reference brightness calibration,
  volumetric cloud animation, sun shafts, the glow buffer (see Post), the photofilter / blueshift of the zone's time of day, SSAO by default (hair material
  conflict, see Post), eye adaptation, procedural grass blades, sprites for far vegetation (HLOD).
* Ambient light: the shade now matches the capture within about 15 % (see Environment); the dirt paths are brighter and warmer than the capture's, and
  the water shows some reflection speckle where the screen space march hits foliage. One `ambient_gain` serves both calibration zones (the
  spawn zone 179 and the palace zone 186 would like about 1.1 and 1.5).
* Royal Palace lighting differences left (the sun direction itself is the engine's, see Sun path): the west-facing walls far from the camera in the
  first view are lit warm in the capture (luma 60 to 80) although the sun is behind them: this is probably the engine's screen space bounce
  (`SSDO`, `Global illumination multiplier` of the time of day) from the sunlit surfaces, which is not done here (ours: 35 to 48, shade only);
  the near big wall of the turned views is 77 against the capture's 37 and stays so with the sun off (`--sun-scale 0`): ambient on a vertical
  surface in a deep canyon (no occlusion term; the capture's wall is also darker towards the floor and mossy: baked occlusion / vertex colours of
  the building meshes are not applied, an aa-cry / objects question); the court yard floor of the first view is black (3 to 8 against 28): the
  terrain shader decodes the baked cover colour, which is near black along the walls (terrain owner); the sunlit patch is about 20 % too dark
  (terrain tint). Tried and not kept: a warm "sun bounce" on the faces of the ambient cube turned away from the sun (strength 1.5 x the lateral
  ambient) brings the first view's far walls and the trees within 10 luma of the capture, but the same wall seen near in the turned views (dark
  in the capture) and the left block become 30 luma too bright, the total error over the regions grows (261 -> 307 summed luma). The
  characters are lit differently in the engine: the time of day has two extra "Char BackLight" lights (`1 Color` 0.49 x 5.0,
  `2 Color` 1.0 x 0.7, constant over the day) and `Char Light Multiplier`, so the capture's character is a clear brown while ours (shade only)
  is nearly black (aa-client / aa-anim materials).
* The real client's time-of-day hour warp, spatial raw-curve zone blending and discrete skybox/cloud fades are implemented (see Environment). Remaining transition gaps:
  area subject qualification/movie/master-weight producers, per-zone relic layer weights and weighted `env.xml` multipliers. The plateau of
  12:00 to 19:05 (every hour reads as noon) and the compression of 19:00 to 22:00 into spline 12 to 18 come from the 1.2 code as read, not from a day sweep
  of the real client (`time set hour=H` captures at 2, 3, 5, 8, 12, 15, 19, 20, 21, 22, 23 would confirm it). The dusk colours of the pond at 21 to 22 h
  (olive, then dark red) are the zone's fog colour lit by the sun colour (`water_lit`), not checked against a capture.
* login2: cinema cameras and visible static entity slots are supported by the generic world renderer. The race tutorial targets are
  authored hidden; aa-client's separate create scenery activates them and owns its isolated stage rendering.
* Far objects (beyond 1000 m brushes / 220 m small vegetation / 600 m trees) vanish; there is no HLOD or impostor.

## 7. Requests to other crates

* **aa-cry**: `CryAssets::load_model` is main thread only and its cost (parse, mesh build, DDS decode) is the largest main thread item here;
  a prepare / finish split (parse + mesh + decode on a task, the main thread only adds assets) would remove the per-frame time budget.
  A seedable cache would let aa-world keep one model set per race across screens.
* **aa-cry**: some `.mtl` files bind `Map="Environment"` cube maps in a 2D slot (base colour of `hdeco_table_h.cgf` is a cube image),
  a wgpu validation error that quits the app. aa-world strips such textures after loading; the proper fix is to ignore the Environment map or
  use it as `EnvironmentMapLight` only.
