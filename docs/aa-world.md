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
| 9 | decal, 111 B; planar / forward terrain and static-brush projections drawn |
| 13 | road, `63 + 12 n` bytes (`n` u32 at payload offset 39); parsed and projected onto terrain |
| 11 | water volume, the rest of the block |
| 6, 14 | the rest of the block, not decoded (fog volumes / auto cubemaps, probably) |

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

Remaining decal limits: deferred volumes and wrapped modes (deferred flag / projection 4..7)
remain parsed but undrawn; their renderer-side projection matrix, asymmetric depth fade and
normal/specular accumulation need their own implementation. Most Memoria (10,14) decal records
name an empty material slot and correctly produce nothing (all 346 in the sampled cell).
Forward geometry currently uses LOD0 receivers and finest terrain, so coarse distant LODs can
diverge. Radii above 128 m or projection bounds above a cell width are rejected to bound work.
Specular/gloss/cubemap effects retain the existing material approximation, and `entities.xml`
static render props are not yet decal receivers.

Follow-up native audit: `sub_3005CA30` scales the deferred projection basis XY by radius and Z by
`radius * max(frontDepth, backDepth)`; `sub_3005DE50` maps projection 4..7 to flags 0x12 / 0x32 /
0x52 / 0x72. The original `deferredshading.cfx` contains the wrapped/fake-wrapped UV correction
and the front/back asymmetric depth attenuation. The missing native renderer CPU matrix setup
must still connect the two authored depths to `g_mLightProj`, `g_worldToDecal` and `g_decalSize`.
The newer public single-depth implementation differs, so it has not been used as a substitute.
A bounded 48-cell sample found 81 mode-0 deferred siegefield decals in (2,29), confirming this is
real authored content rather than an unused enum. No guessed deferred volumes are emitted.

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
day, colours `t:(r:g:b):flags`, sRGB; `* multiplier` variables carry the HDR scale: sun about 6 to 8, sky 2 to 6). Evaluated with linear
interpolation and day wrap-around (`env::Spline`). Used: `Sun color` x `Sun color multiplier`, `Sky color`, `Ambient ground color`, `Fog color` x
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
components. `LocalLightSettings` defaults to 64 nearest lights, two nearest authored shadow
requests, and a 400 m distance to the influence sphere. Selection runs at 5 Hz; point-light
components only change when the selected set or parameters change. Inactive/hidden records do
not render. Projector textures, fake/negative lights and deferred clip bounds are retained as data
and counted as unsupported, without substituting omnidirectional lights. Runtime `enabled=false`
or `AA_NO_LOCAL_LIGHTS=1` disables rendering while retaining the cell data. `LocalLightStats`
reports authored/rendered/shadowed/unsupported counts. No additional aa-client wiring is needed.

The current renderer uses Bevy's clustered `PointLight` path, so photometry is an approximation:
the original `ShadeLib.cfi::GetAttenuation` and `DeferredShading.cfx::GetDeferredLightAttenuation`
give `(1-d²/r²)²`; Bevy uses `(1-(d²/r²)²)²/d²`. Luminous intensity is therefore
`0.16 * radius² * WorldConfig.sun_lux_scale` (lumens multiply by `4*pi`), matching diffuse
illuminance at half-radius. RGB is the authored diffuse product, without arbitrary source colors.
The exact Cry falloff, legacy `fHDRDynamic`, per-light specular multiplier, ambient-light shader
mode, style animation, vis-area restrictions and legacy shadow-bias units are not reproduced.
Those fields remain typed data; shadows use Bevy's bias defaults and the bounded shadow budget.
IndirectLight and projected/cubemap lighting remain separate renderer work.

Validation: `cargo test -p aa-world --lib light` passes five tests, including the real dock
Light_116 (radius 18, warm diffuse times two, HDR 1) and parented airport Light129; the runtime
budget test excludes inactive/hidden/projector/fake slots and releases all render components
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
The matching catalogue reader in public Lumberyard [`CRESky.cpp`](https://github.com/aws/lumberyard/blob/master/dev/Code/CryEngine/RenderDll/Common/RendElements/CRESky.cpp) confirms the Cry direction
`(-cos(dec) sin(15 RA), cos(dec) cos(15 RA), sin(dec))`. The original `shaders/hwscripts/cryfx/stars.cfx` supplies the magnitude
conversion, eight-entry flicker table, Gaussian colour / hot-core falloff and additive blend. Its geometry shader expands
the screen-facing quads; the matching [`CStars::Render`](https://github.com/aws/lumberyard/blob/master/dev/Code/CryEngine/RenderDll/XRenderD3D9/D3DRenderRE.cpp) uses five pixels, reduced below 1280 x 720, and half-speed flicker time.
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
`EnvSample.moon_dir` and `moons` participate in the existing zone blend. Remaining limits: optional non-default
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

**Sun path** (`env::sun_direction`, `Lighting { sun_rotation, latitude, planet_axis }`): the engine's own formula, taken from the decompiled
`CTimeOfDay::Update` (`sub_300AECE0` in `D:\aa-re\zone\out\cry3dengine_dedicate.c`, around line 121800): the matrices
`T(0, 0, -sin axis) x Rz((hour + 12) / 24 turn) x Rx(latitude - 90) x Ry(-rotation)` are multiplied and inverted and the point (0, 1, 0) is
transformed by the result: `Ry(rotation) Rx(90 - latitude) Rz(-angle) (0, 1, sin axis)`, as (x, up, z south). The sun turns 15 degrees an hour,
highest at 12:00. With the main world's 188 / 154 / 25: rises at 2:00 in the north-east (`DawnTime` 119 min), azimuth 172 / elevation 49 at 12:00,
sets at 22:00 in the north-west (`DuskTime` 599 min + 12 h); with login2's 260 / 40 / 0 it is a great circle, 40 degrees up in the west at noon.
Below the horizon the light is the moon (opposite direction here). **Cross-check with the real client**: its capture of the Royal Palace
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
`size`, `speed`, `material`). Each becomes a camera facing quad (4 per zone in the spawn zone) placed at `camera + compass direction x distance +
height` and shaded as the engine's `DistanceClouds` material (`distanceclouds.cfx`) does: density = the red channel of the sprite texture, the
colour lerps from the sky shading colour to the sun shading colour by `exp2(-Attenuation x density)` summed over 8 taps towards the sun in the
sprite plane, alpha = `density ^ AlphaSaturation x Opacity`, faded into the haze at the horizon. `AlphaMode::Blend`, no prepass, no shadows, so
the cost is 4 draws. The clouds are subtle against the capture's sky (`cloud_gain` scales their brightness, `speed` is not animated yet).

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

* normal: the four layers of the bump map the engine adds (big waves x2 and x4, small waves x8 and x16 tiles, drifting at `WavesSpeed`), world
  space coordinates, `BumpScale`, flattened beyond 60 to 260 m,
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

**Fit** to the real combat frames (`reference/ingame/combat_triple_slash/real_05.png`, `combat_return/real_10.png`, the Nuian pond; `fly
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
  `placement::{parse_brush_dat, parse_vegetation_dat, parse_vegetation_sector, parse_object_dat, parse_water, load_cell_objects}`,
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
`load_cell_objects`), each distinct model's proxy loaded once and shared by all cells (a `OnceLock` per model, kept for the process: 3.2k models,
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
  over the 2 m samples at every distance while the engine interpolates over its (LOD) triangles; deferred `object.dat` decals
  remain undrawn; `terrain_tint` is a fudge for the missing exposure. Roads and forward decals are rendered (see placement formats).
* Water, differences to the real captures: the pond matches the combat frames within 7 display values (see Water), but the fit is on one pond
  at two hours (2.81 and 3.76, sun low in the north-east); other lakes and rivers only follow the engine's formulas with their own `.mtl` parameters
  (no reference capture: the river of cell (9, 12) is dark and nearly clear, the old build was milky white). The bed of the pond is the terrain
  owner's (the cover colour is a flat teal where the real bed is darker and greener), no flow along rivers, the sun glint of the real frames
  is not visible (the sun is low and to the left there), the sea has no foam / white caps or beach gradient (its colour is the zone's `Ocean
  FogColor` x `sea_body`, hazy at distance), no caustics, waterfalls, light shafts under water, refraction of the surface window seen from
  below or the transition at the surface line. The underwater colours are plausible, not fitted (no reference capture).
* Not rendered: deferred object.dat decals (type 9), types 6 and 14, unsupported CGA curves / named clips and skinned `entities.xml` render slots, indirect/projected local lights.
  Hidden cinematic actors/targets remain aa-client's screen-flow responsibility. Also missing: cloud animation (`speed` of `clouds.xml`; the sprites are subtle, the
  real picture has more, brighter clouds), sun shafts, the glow buffer (see Post), the photofilter / blueshift of the zone's time of day, SSAO by default (hair material
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
* The real client's time of day is solved (see Environment, Time of day warp): the splines are read at the engine's warped hour. Not modelled: the
  engine blends two zone layers by `layer weight` across zone borders (aa-world reads layer 0 of the zone under the camera), and the plateau of
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
