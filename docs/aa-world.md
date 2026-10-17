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
| 9 | decal, 111 B (kept raw) |
| 13 | road, `63 + 12 n` bytes (`n` u32 at payload offset 39); skipped |
| 11 | water volume, the rest of the block |
| 6, 14 | the rest of the block, not decoded (fog volumes / auto cubemaps, probably) |

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
it reads all 1205 `leveldata.xml`, the 315 `.mtl` and copies the DDS blocks of 186 diffuse (BC1, 512 px), 54 large diffuse (BC1, 1024 px) and
255 normal (BC5, 512 px; the normal maps and the `Detail` normal maps share the array) maps, mip chains down to 4 px, into three texture arrays
(159 MB of VRAM). DXT1 normal maps are transcoded to BC5 (two BC4 blocks from the decoded red and green). A diffuse map of 1024 px or more
goes to the large array (the diffuse array holds the 512 px ones, `TerrainExt::details` is the large array), its slot is + 1000 in the table;
a 256 px normal map is stored as 2 x 2 copies in the 512 px array (slot + 1000, the shader samples it at half the uv, which gives the original back
with its own texel density and mips, and keeps the tiles continuous across cell borders because the content has period 1 in the layer uv).
Other sizes or formats are skipped (diffuse maps below 512 px, normal maps below 256 px), a texture array may have 256 layers, so 255 per array at
most (the least used normal maps are dropped: 4 of 259 in main_world, one cell each); larger sources use their lower mips (diffuse above 1024 px,
normal maps above 512 px). The large array exists because 59 diffuse maps of 1024 px (58) and 2048 px (1) were read from their 512 px mip: Austera's
court yard ground (`cliff_grey_with_dry_bushes`, a 2048 px map over a 50 m period, 41 texels per metre) had 10 texels per metre and showed blocks the
size of a hand right at the camera. At 1024 px (20 texels per metre) the mean luma gradient of the ground in front of the camera is 38 % higher in the
lower third and 20 % in the middle (`shots/terrain/aus_gal_before.png` against `aus_gal_after.png`, `fly` at the gallery camera of `30_austera`, objects on),
at the native 2048 px it would be 54 / 63 % (x / y) against 32 / 39 % for 1024 px in a closer view (`aus_before` / `aus_after` / `aus_2048`,
experiment, not kept: it needs 165 MB for the array, or a fourth array that `TerrainExt` does not have). Per cell, the background load adds
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
about 0.3 s. It drives: a sun / moon `DirectionalLight` (illuminance = `sun_lux_scale x luminance(sun color x multiplier)`, cascaded shadows,
3 cascades over `shadow_distance`), the ambient light (see below), the sky dome, the cloud sprites, the post pass (fog, film curve, grading) and the
water materials. The sun follows `env::sun_direction` (see Sun path below); at night the light comes from the moon side.

All HDR colours are `data value x AtmosphereTuning::radiance (540) x the view's exposure` (the material shaders output pre-exposed colours, the post
pass does the film curve). `AtmosphereTuning` is the calibration against the capture; every field can be overridden for experiments with
`AA_TUNE="sky_gain=3,fog_ext=0.05"` (names: `radiance fog_gain fog_ext glow sky_gain sky_exp sun_sky disc box_gain box_yaw fog_sky_mix saturation
exposure water_body water_lit water_absorb water_refl water_sun water_foam water_env water_ssr ssr refraction water_rough sea_body sea_absorb
sea_refl sea_sun under_absorb under_scatter ssao cloud_gain ambient_env ambient_gain ambient_sat ambient_dome ambient_spec ambient_height sky_tau
sky_phase sky_hue sky_mie sky_green sun_az sun_el`; `sun_az` / `sun_el` (degrees, bearing
and elevation) pin the sun direction instead of the sun path).

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

**The time of day of the capture is not recoverable**: `e_time_of_day` says 19.17 for the spawn and 6.17 for create step 3, 13 h apart, while both
pictures show full daylight, and the data is night blue at 19.17. So `hour 10, layer 0` is a calibration choice: sun 44 degrees up in the south-east, pale blue
horizon, grey blue fog; `AtmosphereTuning` was fitted so that region means of `shots/atmosphere/spawn.png` match the capture within about 10 %
(sky 170 / 190 / 203 against 171 / 184 / 203, hills 81 / 102 / 120 against 91 / 109 / 120, water 54 / 77 / 79 against 45 / 64 / 47 ..).

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
usage, Bevy `Bloom::NATURAL` at `bloom` intensity 0.1) and adds `PostUniform`. One fullscreen pass after the bloom replaces the tone mapping:

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
--camera-json cam_c05.json --shift 11264,11264,0 --hour 10` with `AA_TUNE=sun_az=290,sun_el=15`, the sun behind the camera as the real
picture's, see "Known gaps" for the clock): mean colour of 12 water regions (near / mid / far, left / right, both frames), mean abs error over
R, G, B in display values: 38.5 before, 6.8 after (the real region means are all within (38..50, 51..60, 33..47), a luminance of 47 to
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
  `TerrainQuery` (ground queries, see below), `WaterQuery` (water height and kind, see Water). Markers: `WorldFocus`, `SunLight`, `SkyDome`, `OceanPlane`, `TerrainPatch`, `CellRoot`,
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

**Collision with brushes (buildings, rocks, bridges): how it would be exposed** (not implemented). The pieces exist: `placement::parse_brush_dat`
gives every brush of a cell (model, transform; 162k in the world), `aa-cry` parses the `.cgf` models and marks the physics proxy nodes
(`proxy*`, `$collision`, `$physics_proxy`, `NodeKind::Proxy`, the `MeshPhysicsData` chunk; materials with `COLLISION_PROXY` / `RAYCAST_PROXY`
flags) that the renderer drops, falling back to the lowest level of detail of the visible mesh for models without a proxy. A
`BrushColliders` next to `TerrainQuery` would build, per cell on a task, a static triangle soup in game space (proxy triangles transformed by the
brush matrices, only brushes whose model has a proxy or a walkable surface), indexed by a 16 m grid, and drop it with the cell; then
`TerrainQuery::ground_below(x, y, z, step_up) -> Option<Ground>` returns the highest surface not above `z + step_up` (the terrain triangle or a
brush triangle whose normal is walkable), plus `raycast(origin, dir, max) -> Option<Hit>` and a capsule sweep for walls. The server stays
authoritative for positions; the client only needs this for standing on stairs and bridges and for not walking through walls. Cost would be one
CGF proxy load per distinct model (cached by `aa-cry`) and a few hundred thousand triangles near the player.

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
| `--hour h` `--speed h/s` `--zone id` `--layer 0/1` | time of day (default hour 10, layer 0) |
| `--sun-scale` `--ambient-scale` `--terrain-tint r,g,b` `--fog-visibility m` `--tree-distance m` | calibration |
| `--radius m` `--no-objects` `--no-shadows` `--no-env` `--no-post` `--no-terrain-detail` | |
| env `AA_TUNE="key=value,.."` | `AtmosphereTuning` overrides, see Environment |
| `--bench s [--bench-speed m/s]` | fly forward and print frame statistics, scene sizes and slow frames, then exit |

WASD / Space / Ctrl fly (Shift fast), right mouse button looks.

### Tests

`cargo test -p aa-world`: 36 unit tests (coordinates, heightmap / cover / brush / vegetation / object / water parsers against the real pak
incl. the spawn pond, BC1 round trip, env splines, sun path, cinema cameras, triangulation, the terrain layer library of the spawn cells and
the uv continuity of layers across cell borders, the BC1 to BC5 normal map transcoder, ground queries against the grid, the rendered
triangulation, cell borders, threads, the cache bound and prefetch; they skip when the pak is missing).
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
run while another session loaded the GPU). Start up: the layer library takes 0.3 s on a task (cells wait for it) and 159 MB of VRAM (186 + 54 + 255
BC textures; 130 MB before the large diffuse array); each cell adds a 263 KB surface map and a 3 KB table, no per frame CPU work, and the draw count is
unchanged (one material per cell). The large diffuse array does not cost frame time: `fly --pos 16776,8926.1,137.2 --pitch -10 --bench 8 --bench-speed 0`
at Austera, old and new build alternating, 116 / 122 / 119 against 114 / 124 / 120 fps (the machine was shared, single runs swing from 24 to 140).

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
  has no colour / the vertex normal; 4 diffuse textures are missing from the pak); the 2048 px ground map of Austera is used at 1024 px (half the
  engine's texel density, see Detail layers); the weights are bilinear
  over the 2 m samples at every distance while the engine interpolates over its (LOD) triangles; the roads and decals of `object.dat`
  (types 13 and 9, `aa_terrain/roads`, `aa_terrain/decals`) are not drawn; `terrain_tint` is a fudge for the missing exposure.
* Water, differences to the real captures: the pond matches the combat frames within 7 display values (see Water), but the fit is on one pond
  at one hour with the sun pinned behind the camera; other lakes and rivers only follow the engine's formulas with their own `.mtl` parameters
  (no reference capture: the river of cell (9, 12) is dark and nearly clear, the old build was milky white). The bed of the pond is the terrain
  owner's (the cover colour is a flat teal where the real bed is darker and greener), no flow along rivers, the sun glint of the real frames
  is not visible (the sun is behind the camera there), the sea has no foam / white caps or beach gradient (its colour is the zone's `Ocean
  FogColor` x `sea_body`, hazy at distance), no caustics, waterfalls, light shafts under water, refraction of the surface window seen from
  below or the transition at the surface line. The underwater colours are plausible, not fitted (no reference capture).
* Not rendered: object.dat decals (type 9) and roads (13), types 6 and 14, `entities.xml` (lights, particle effects, `GeomEntity` props,
  the login stage's training dummies and grass, `CameraSource` positions), cloud animation (`speed` of `clouds.xml`; the sprites are subtle, the
  real picture has more, brighter clouds), sun shafts, the photofilter / blueshift of the zone's time of day, SSAO by default (hair material
  conflict, see Post), eye adaptation, moon / stars (the night sky is the gradient only), procedural grass blades, sprites for far vegetation (HLOD).
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
* The real client's time of day: `e_time_of_day` in the capture says 19.17 (the spawn) and 6.17 (create step 3) while both pictures show
  full daylight, yet zone 179's `Sun color` spline is night blue at 19.17 in both of its layers. The clock here is a free parameter (default
  hour 10, layer 0, see Environment); matching the capture exactly needs the engine's own time mapping. The engine's sun at 19.17 h would be
  low in the west-north-west (azimuth 288, elevation 13), the capture's shadows point the other way (it is lit from the north-north-east);
  the palace times (the game clock of the capture) are reliable (the sun path matches), the spawn one is not.
* login2: cinema cameras and the real cameras are supported, but the race training fields of create step 3 are not part of the cell
  brush data (entities / cinemas); the scene shows terrain and the stage props of `object.dat`.
* Far objects (beyond 1000 m brushes / 220 m small vegetation / 600 m trees) vanish; there is no HLOD or impostor.

## 7. Requests to other crates

* **aa-cry**: `CryAssets::load_model` is main thread only and its cost (parse, mesh build, DDS decode) is the largest main thread item here;
  a prepare / finish split (parse + mesh + decode on a task, the main thread only adds assets) would remove the per-frame time budget.
  A seedable cache would let aa-world keep one model set per race across screens.
* **aa-cry**: some `.mtl` files bind `Map="Environment"` cube maps in a 2D slot (base colour of `hdeco_table_h.cgf` is a cube image),
  a wgpu validation error that quits the app. aa-world strips such textures after loading; the proper fix is to ignore the Environment map or
  use it as `EnvironmentMapLight` only.
