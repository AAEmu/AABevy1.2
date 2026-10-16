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
`lighting.xml`: `SunRotation` is the compass bearing the light travels towards at midday (188 = the sun stands north-ish), `SunHeight` the
peak elevation (70, capped at 75 here).

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
it reads all 1205 `leveldata.xml`, the 315 `.mtl` and copies the DDS blocks of 240 diffuse (BC1, 512 px), 234 normal (BC5, 512 px) and 71 detail
normal (BC5, 256 px) maps, mip chains down to 4 px, into three texture arrays (130 MB of VRAM; DXT1 normal maps are transcoded to BC5 (two
BC4 blocks from the decoded red and green), normal maps that are only 256 px go to the detail array (slot + 1000 in the table), other sizes
or formats are skipped, and a texture array may have 256 layers, so 255 per array at most; larger sources use their lower mips). Per cell, the background load adds
an `R8Uint` image of the 513 x 513 surface nibbles and a 3 KB uniform table: for each of the 16 surface types and 3 projections the array
slots, the uv scale, a uv offset (`fract(cell origin * scale)`, so that tiles continue across cell borders, unit tested), the detail strength
and the detail map's tiling. The shader reads the four surrounding nibbles (bilinear weights, equal types merged), the dominant axis of the
normal gives the projection weights, and each layer is sampled with explicit gradients (`textureSampleGrad`, the loops are not uniform
control flow): up to about 4 types x 1 to 2 axes x 3 fetches, usually 3. No per-frame CPU work, no extra draw calls (the draw count is the
cover texture's, one material per cell). `WorldConfig::terrain_detail = false` (`fly --no-terrain-detail`) shades the decoded cover colour only.
`WorldConfig::terrain_tint` (linear 1.2, 1.2, 1.3, was 0.30 / 0.32 / 0.45 for the wrongly read cover) scales the result: the engine's exposure and
film curve are not reproduced, this brings the ground to the real client's brightness (sunlit rock and hill grass of
`reference/ingame/spawn/real.png`; the real picture's cooler grade, shadow depth and SSAO are not reproduced). Re-tune it when the lighting
calibration (`sun_lux_scale`, `ambient_scale`) changes.

### Objects (`objects.rs`, `placement.rs`)

Placement files are parsed on the task pool (max 2 cells in flight) and binned per 64 m sector. The main thread spawns sectors nearest first
under a time budget (`object_budget_ms`, 3 ms per frame) through `aa_cry::CryAssets::load_model`; a sector fills in over a few frames. A
part is one entity with the node transform baked, sharing the mesh and material handles of every instance of the model, so Bevy batches
them into instanced draws. Every part has `VisibilityRange::abrupt(0, view)` (its own `maxViewDist`, capped by `brush_distance` 1000 m /
`vegetation_distance` 220 m / `tree_distance` 600 m); a sector is despawned when the camera is farther than that cap. Vegetation is split by its
own view distance: instances up to 220 m are small plants (`Kind::Vegetation`), larger ones (trees, up to 1 km and more in the data) are
`Kind::Trees` and are drawn up to 600 m. Level of detail: `_lodN` / `$lodN` by distance band (brush 140 / 380 m, vegetation 55 / 130 m, trees
150 / 350 m, with hysteresis); a sector is respawned when its band changes. Models that fail to load
are remembered (`ObjectStats::models_failed`). aa-cry binds some `Map="Environment"` cube maps as the base colour of 2D materials, which is
a wgpu validation error that kills the app (the renderer prepares every material asset, used or not): `strip_cube_textures` drops such
textures when a model is first loaded.

Water: the type 11 volumes of a cell become flat meshes at the surface height (`water.rs`: quad strips reordered into rings, ear clipping
for lakes), one mesh per `WaterStyle` (material path + the volume's own fog colour / density), spawned with the cell's other data
(`water_distance` 1400 m) with the style's `WaterMaterial` (`water_material.rs`, see Water below). The terrain pokes through where the banks
are higher, which is the shore line.

### Environment (`env.rs`, `sky.rs`, `sky_material.rs`)

`WorldClock { hour, speed, zone, layer }` (default hour 10, layer 0) selects the sample of the zone under the camera (`zone_at`), smoothed over
about 0.3 s. It drives: a sun / moon `DirectionalLight` (illuminance = `sun_lux_scale x luminance(sun color x multiplier)`, cascaded shadows,
3 cascades over `shadow_distance`), `GlobalAmbientLight` (`ambient_scale x` the sky / ground colour luminance), the sky dome, the post pass
(fog, film curve, grading) and the water materials. The sun rises at 7 h and sets at 18 h on the arc of `lighting.xml` (bearing from
`SunRotation`, elevation `min(SunHeight, 75)`), at night the light comes from the moon side.

All HDR colours are `data value x AtmosphereTuning::radiance (540) x the view's exposure` (the material shaders output pre-exposed colours, the post
pass does the film curve). `AtmosphereTuning` is the calibration against the capture; every field can be overridden for experiments with
`AA_TUNE="sky_gain=3,fog_ext=0.05"` (names: `radiance fog_gain fog_ext glow sky_gain sky_exp sun_sky disc box_gain box_yaw fog_sky_mix saturation
exposure water_body water_absorb water_refl water_sun ssr refraction water_rough sea_body`).

**The time of day of the capture is not recoverable**: `e_time_of_day` says 19.17 for the spawn and 6.17 for create step 3, 13 h apart, while both
pictures show full daylight, and the data is night blue at 19.17. So `hour 10, layer 0` is a calibration choice: sun 57 degrees up, pale blue
horizon, grey blue fog; `AtmosphereTuning` was fitted so that region means of `shots/atmosphere/spawn.png` match the capture within about 10 %
(sky 170 / 190 / 203 against 171 / 184 / 203, hills 81 / 102 / 120 against 91 / 109 / 120, water 54 / 77 / 79 against 45 / 64 / 47 ..).

**Sky** (`sky_material.rs`, `sky.wesl`): a dome of radius `sky_radius` following the camera, a `Material` without prepass. Gradient from
`Night sky: Horizon color` to `Zenith color` (x multipliers x `sky_gain`), sun glow and disc, covered by the zone's sky box the way the engine's
`Sky` shader (`sky.cfx`) blends it: colour = `tex.rgb x lerp(SkyColorMultiplier x ShadeColorFromSky, SunColorMultiplier x ShadeColorFromSun,
exp2(-Attenuation x 8 r))`, alpha = `tex.a x Opacity x r ^ AlphaSaturation` with `r` the red channel. The box is `box_12.dds`, `box_34.dds` and
`box_5.dds` next to the material's diffuse map (`Materials/sky/sky_solzreed_cbt1.mtl`, `Shader=Sky`); layout found by matching the face edges:
four side faces are the upper halves of cube faces (1024 x 512), clockwise north, east, south, west; `box_12` stacks north (top) and east
(bottom, flipped vertically), `box_34` south and west, `box_5` is the top face with north at its bottom edge. `Angle` of `env.xml` rotates
it. The stormy photo is only faintly visible (`box_gain`), as the real picture shows a pale overcast sky with a few bright patches; there are no
separate cloud sprites (`clouds.xml` lists distant cloud billboards: not drawn).

### Post (`post.rs`, `post.wesl`)

`Post { bloom, fog, exposure_ev }` on a `Camera3d` (added automatically to the focus entity, else the first camera, when
`WorldConfig::post`; aa-client's camera needs nothing) makes it an HDR camera (`Hdr`, `Tonemapping::None`, `DepthPrepass` with `TEXTURE_BINDING`
usage, Bevy `Bloom::NATURAL` at `bloom` intensity 0.1) and adds `PostUniform`. One fullscreen pass after the bloom replaces the tone mapping:

* **fog** from the depth buffer: height exponential layer (`Fog height (bottom / top)`, densities, optical depth integrated along the ray), starting
  at `Volumetric fog: Ramp start`, extinction `fog_ext` (0.04 / m at density 1) x `Global density` x layer density, colour from the layer colours
  (`Fog color`, `Fog color (top)`, mixed 60 % towards the sky horizon colour: the data fog colour is more saturated than the real haze) plus a
  sun glow; skipped beyond 0.9 `sky_radius` (the dome has its own haze),
* **grade**: exposure, contrast around 0.18, colour balance, saturation (data `Saturation` x 0.85),
* the engine's **film curve** (`postprocess.cfx`: `x (a 6.2 x + b 0.5) / (x (a 6.2 x + 1.7) + c 0.06)` with shoulder / midtones / toe scales from the
  data, divided by the same at the white point; its output is gamma encoded, so the pass raises it to 2.2 because the swap chain re-encodes), `Levels: min input`.

`aa_world::post::{Post, PostUniform}` is the reusable setup; a second camera only needs the `Post` component. `--no-post` / `post: false`
leaves the camera alone (Bevy tone mapping, no fog, the water falls back to a constant thickness and no reflections).

### Water (`water.rs`, `water_material.rs`, `water.wesl`)

A `WaterMaterial` per style (material path + fog): constants from the `.mtl` public params (see object.dat), environment part refreshed each frame
by `update_water`. The shader runs in the transmissive phase (`reads_view_transmission_texture`: the frame's opaque colour is copied once) and
reads the depth prepass:

* normal: the four layers of the bump map the engine adds (big waves x2 and x4, small waves x8 and x16 tiles, drifting at `WavesSpeed`), world
  space coordinates, `BumpScale`, flattened beyond 60 to 260 m,
* body: `mix(water colour, refracted scene, exp(-absorption x thickness))`; thickness is the depth prepass minus the surface depth along the view
  ray (`SoftIntersectionFactor` makes the shore transparent), water colour = volume fog colour x `water_body` x light, absorption = volume density
  x `water_absorb`; refraction offsets the lookup by the bump, rejected where the sample is in front of the surface,
* reflection: Schlick Fresnel (reflectance `FresnelBias x 0.25`, x `water_refl`) of a screen space ray march (2 rays spread vertically by
  `water_rough`, 16 steps growing 1.3x + 0.3 m, thin object test, one refinement step, within 160 m, fades at the screen edge) against the depth
  prepass, colour from the transmission copy; the analytic sky (horizon / zenith gradient + glow) where nothing is hit,
* sun glint `pow(dot(reflect, sun), SunShinePow) x sun colour x SunMultiplier`.

The sea is the same material (style `<sea>`, bigger and faster waves) on a plane at `ocean_level` following the camera; its colour is the zone's
`<Ocean FogColor>` x `sea_body`. Not done: foam (`FoamMultiplier`, no foam texture used), flow along rivers, caustics, waterfalls (`WATERFALL` flag),
underwater view. The engine's own shader (`watersurface.cfx`) mixes `refractColor` with only `ReflectionAmount x TransparencyRatio x fresnel` of the
reflection (about 6 %); the real picture shows stronger reflections than that (probably planar reflection render targets), so `water_refl` is a fudge.

### Streaming (`stream.rs`)

`stream_cells` picks cells whose square is within `load_radius` (3000 m) of the focus (the entity with `WorldFocus`, else the first
`Camera3d`) and starts `load_cell` on `AsyncComputeTaskPool` (max 3 at a time): read `heightmap.dat` and `cover.ctc`, build the atlas
and all 16 patch meshes at the likely level of detail. A channel returns them; `receive_cells` moves at most `spawns_per_frame` cells into
the world per frame. Cells farther than `unload_radius` (3600 m) are despawned. Failed loads (no terrain, e.g. empty ocean) are remembered.

## 4. API

```rust
app.add_plugins(aa_world::AaWorldPlugin::new("main_world"));   // adds aa_cry::CryPlugin when absent
// the camera and window are the application's; the first Camera3d (or the entity marked WorldFocus) streams the world
```

* `AaWorldPlugin { config: WorldConfig }`; `WorldConfig` fields: `world`, `load_radius`, `unload_radius`, `max_loads`, `spawns_per_frame`,
  `lod_updates_per_frame`, `objects`, `vegetation`, `brush_distance`, `vegetation_distance`, `tree_distance`, `water_distance`, `object_budget_ms`,
  `environment`, `post`, `sky_radius` (the camera far plane must exceed it), `shadows`, `shadow_distance`, `sun_lux_scale` (1700), `ambient_scale`
  (1700), `fog_visibility`, `terrain_tint`, `terrain_detail`.
* Resources: `WorldClock` (time of day: `hour`, `speed` in game hours per second, `zone` to force one, `layer` of the zone files), `Environment` (the
  applied sample, `ocean_fog`), `AtmosphereTuning` (`AA_TUNE`), `WaterStyles` (materials by style), `TodCache`,
  `WorldInfo(Arc<World>)`, `Streamer::loaded_cells()`, `ObjectState` (`stats`), `WorldTimings` (per frame main thread ms
  of the streaming systems), `TerrainLayers` (the layer library of the world, loads on a task at startup; cells wait for it),
  `TerrainQuery` (ground queries, see below). Markers: `WorldFocus`, `SunLight`, `SkyDome`, `OceanPlane`, `TerrainPatch`, `CellRoot`,
  `ObjectSector`.
* Coordinates: `cry_to_bevy`, `bevy_to_cry`, `cell_origin`, `cell_of`, `CELL_SIZE`.
* Parsers (no Bevy world needed, all tested against the pak): `heightmap::CellHeights::parse`, `cover::Cover::parse` (+ `atlas`),
  `placement::{parse_brush_dat, parse_vegetation_dat, parse_vegetation_sector, parse_object_dat, parse_water, load_cell_objects}`,
  `vegetation::VegGroups`, `world::World` (`load`, `zone_at`), `env::{TimeOfDay (parse, parse_layer), Lighting, sun_direction, EnvSample::lerp}`, `sky_material::{ZoneSky, load_sky_box}`, `post::{Post, PostUniform}`,
  `cinema::CinemaCamera` (`load`, `pose(t)`), `water::{water_mesh, water_meshes, WaterStyle, triangulate}`, `water_material::{WaterMaterial, WaterStyles}`,
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
* Not included: terrain holes (`hasHoles` is parsed over, none known in 1.2 data), water (`World::ocean_level`, `placement::parse_water`),
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
run while another session loaded the GPU). Start up: the layer library takes 0.3 s on a task (cells wait for it) and 130 MB of VRAM (240 + 234 + 71
BC textures); each cell adds a 263 KB surface map and a 3 KB table, no per frame CPU work, and the draw count is unchanged (one material per cell).

Atmosphere (sky dome, post pass with depth prepass + HDR + bloom, water shader with screen space reflections), `fly --camera-json
reference/ingame/spawn/camera.json --shift 11264,11264,0 --bench 20 --bench-speed 0` (standing at the pond, 224 sectors, 4600 part entities, 7 water
cells), the build of the commit before against this one, alternating runs: 128 / 128 / 115 / 114 fps before, 113 / 112 / 111 / 97 fps after on quiet
rounds (about -12 %); noisy rounds with another session on the GPU gave 67 to 97 before and 61 to 93 after. The scene is CPU bound (about 8.5 ms
per frame), so the GPU passes (HDR target, depth prepass, bloom, fog / film pass) cost almost nothing by themselves; the water's ray march is the
visible part (`AA_TUNE=ssr=0`: about +10 %) and shrinks with the screen area of water (it is limited to 160 m and to two 16 step rays).

The numbers move by a factor of two with the load of the machine: while another session's real client rendered on the same GPU the same scene
ran at 55 to 65 fps. Main thread cost of the streaming systems in slow frames (`WorldTimings`) is below 6 ms (`objects`); the rest of a slow
frame is render world work (uploading 3 MB cover atlases, extraction of the part entities). A release build of this crate would cut the CPU
share further.

## 6. Known gaps

* Terrain detail, differences to the real client (`shots/terrain/spawn.png` against `reference/ingame/spawn/real.png`): the real picture has a
  cooler, less saturated grade (photofilter, film curve, exposure) and deeper shadows under bushes (SSAO); the layers have no specular /
  gloss (`_sp` maps: the ground is fully rough); diffuse maps below 512 px and normal maps below 256 px are skipped (the layer then
  has no colour / the vertex normal; 4 diffuse textures are missing from the pak); the weights are bilinear
  over the 2 m samples at every distance while the engine interpolates over its (LOD) triangles; the roads and decals of `object.dat`
  (types 13 and 9, `aa_terrain/roads`, `aa_terrain/decals`) are not drawn; `terrain_tint` is a fudge for the missing exposure.
* Water, differences to `reference/ingame/spawn/real.png`: no foam at the shore and around the player, no flow along rivers, the reflection is a
  two ray screen space march (no reflection of things outside the screen, grainy ripples where the foliage depth has holes, bank trees only
  where they are on screen), the body colour and reflection strength are fudged (`water_body`, `water_refl`), the real pond is a little darker
  and greener in the middle, the real surface has bigger soft ripples near the camera. Caustics, waterfalls and the underwater view are not done.
  The sea has no foam / white caps or beach gradient; its colour is the zone's `Ocean FogColor` x `sea_body` and it is hazy at distance.
* Not rendered: object.dat decals (type 9) and roads (13), types 6 and 14, `entities.xml` (lights, particle effects, `GeomEntity` props,
  the login stage's training dummies and grass, `CameraSource` positions), `clouds.xml` cloud billboards (the sky box has clouds, the
  real picture has more, brighter ones), sun shafts, the photofilter / blueshift of the zone's time of day, SSAO (deeper shadows under bushes in
  the real picture), eye adaptation, moon / stars (the night sky is the gradient only), procedural grass blades, sprites for far vegetation (HLOD).
* Ambient light is too dark in shade (trees 19 / 35 / 17 against 28 / 40 / 30 in the capture, the grass in shadow 19 / 33 / 6 against 29 / 41 / 18) while
  the lit surfaces match; raising `ambient_scale` over-saturates the foliage, the engine's ambient is less saturated and has sky brightening
  from terrain occlusion.
* The real client's time of day: `e_time_of_day` in the capture says 19.17 (the spawn) and 6.17 (create step 3) while both pictures show
  full daylight, yet zone 179's `Sun color` spline is night blue at 19.17 in both of its layers. The clock here is a free parameter (default
  hour 10, layer 0, see Environment); matching the capture exactly needs the engine's own time mapping.
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
