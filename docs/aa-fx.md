# aa-fx: CryEngine particle effects

`crates/aa-fx` plays the particle libraries of the original client (`game/libs/particles/*.xml`, CryEngine 3
`ParticleParams`) in Bevy: parsing, a CPU simulation in compact per-emitter buffers, one batched mesh per emitter.

```rust
app.add_plugins(aa_fx::FxPlugin);
commands.spawn((aa_fx::Effect::new("X2_EFFECTS.fire_smoke.torchlight_b"), Transform::from_xyz(0.0, 0.0, -3.0)));
```

## API

* `Effect { name, scale, speed_scale, count_scale, alpha_scale, brightness, tint, ground_y, prime, looping, despawn_when_finished }`
  (`Effect::new(name).with_scale(s)`). `name` is `library.group.name`, case does not matter; the library is everything
  before the first dot (`X2_EFFECTS`, `abillity_skill_table_m`, ...). The entity's transform places, rotates and scales it
  (the average of the global scale axes multiplies `Effect::scale`). Put it under another entity (`ChildOf`) to attach it.
  `RenderLayers` of the effect entity are copied to its meshes. `alpha_scale` multiplies the opacity of this effect, `brightness`
  and `tint` (a colour) the light on its particles (on top of `FxSettings::light`). `ground_y` is the height of a flat ground
  for this effect (collisions, see below).
* `FxSettings` (resource): `max_particles` (20000, the hard global budget), `max_distance` (160 m activation radius cap),
  `gain` (colour multiplier; a world with pre-exposed HDR colours sets `radiance * exposure`, aa-world does), `light`
  (0.8, how much of `DiffuseLighting` shows), `emissive`, `size_scale`, `sprite_half` (1: a sprite's `Size` is a radius), `alpha_scale`, `quality` (0..3, `ConfigMin/Max`),
  `count_over_life`, `soft_distance` (0.6 m, needs `DepthPrepass` on the camera, otherwise particles are not softened),
  `activations_per_frame`, `wind`, `sun` (world direction to the sun, shades opaque mesh particles), `refraction` (distort the
  scene behind refraction materials, default on, `AA_FX_NO_REFRACTION=1` switches it off), `ground` (`Some(Ground(Arc<dyn Fn(Vec3)
  -> Option<f32>>))`: ground height below a world position, for collisions), `enabled`.
* `FxStats` (resource): effects, active, emitters, particles, `load` (budget use: a mesh particle counts its vertices / 4), quads
  (sprites, ribbon segments and mesh copies drawn), `sim_ms`, `upload_ms`, `manage_ms`.
* `library::Library` / `EffectDef` / `params::EmitterDef`: the parsed data, usable without Bevy
  (`Library::load("x2_effects")`, `lib.get("fire_smoke.torchlight_b")`).
* Env: `AA_FX_LOG=1` logs the counters every 2 s (`=2` also lists every active emitter with its bounds); `AA_NO_FX=1` leaves
  the world's placed effects out (aa-world); `AA_FX_COUNT_OLD=1` goes back to `Count` per particle life (see below); `AA_FX_SPRITE=0.5` reads a sprite's `Size` as its full width
  again (before round 4); `AA_FX_ONLY=a,b` / `AA_FX_SKIP=a,b` run only the effects whose name contains one of the words / all but those
  (debugging, case insensitive, matched against `lib.group.effect`);
  `AA_CREATE_NO_FX=1` leaves the step 3 mist out, `AA_MIST=alpha,count,r,g,b` overrides its fit (aa-client).

## How it runs

Per frame, in `PostUpdate` after transform propagation: camera view, register, resolve library, classify (parallel distance
test), activate/deactivate (a few per frame), simulate (`par_iter_mut`), upload (swap the vertex vectors into the mesh).

* An effect outside its radius costs one distance test and holds nothing. The radius follows the effect's extent
  (`CameraMaxDistance` and the size/speed/lifetime of its emitters), capped by `FxSettings::max_distance` (x3 for big ones).
* Beyond 45% of the radius the spawn rate thins (down to 40%) and the update rate halves; the effect fades out near the radius.
* On activation an effect is primed (about 10 coarse steps) so that it looks established at once; `prime=false` skips it.
* The budget is an atomic counter: spawning stops when it is used up, so a crowd cannot run away.
* Meshes are children of the effect entity with the quads in effect-local space (no per-frame transform work), `Aabb` from the
  quad bounds, `NotShadowCaster`, depth tested, no depth write, transparent phase (refraction materials: transmissive phase).
  One dynamic mesh per drawing emitter, whatever it draws (sprites, ribbons, mesh copies).
* `FxMaterial`: unlit, texture x vertex colour x gain, additive (`rgb*a`, alpha 0), alpha, premultiplied (`ColorBased`) and
  multiply modes, optional soft fade against the depth prepass, `DrawLast` as a depth bias.

## Parameter semantics

A variable parameter reads `"base,variation,emitterStrengthCurve,particleAgeCurve"`; curves are `(key;key;...)` with keys
`time,value[,flags]` (an omitted time or value is 0, a single number is a time, colours default to 1). The variation is
one-sided: `value = base * (1 - variation * rand)`. Defaults: Speed, ParticleLifeTime, Count, Size, Alpha, DiffuseLighting,
SizeX/Y/Z = 1; everything else 0 (`ParticleLifeTime=0` is endless). A continuous emitter that never ends spawns
`Count / ParticleLifeTime` per second (`Count` alive at once). One with an `EmitterLifeTime` spawns `Count` over that life
(`Count / EmitterLifeTime` per second), and a `PulsePeriod` shorter than the life restarts the emitter while the last emission
still runs, `ceil(life / pulse)` emissions at a time (`flow_fog_01`: Count 10, life 1, pulse 0.5, particle life 3.5 s = 20
per second, 70 alive; reading it as 10 alive made the mist 7 times too thin, see "Step 3 mist"). This is inferred from the
captures, not read from the engine; `FxSettings::count_over_life = false` (or `AA_FX_COUNT_OLD=1`) restores the old reading.
A continuous emitter emits its first particle at once when it starts (not one period later: a short arrow flight is
visible from its first frame). A non continuous emitter bursts `Count`; `PulsePeriod` restarts it; `EmitterLifeTime` ends it;
`Enabled=false` emitters still hand their children to `SpawnIndirection` triggers (`ParentStart`, `ParentDeath`).
The focus axis is the emitter's cry +Y (up with `FocusGravityDir`); `Acceleration` is in world space; a sprite's `Size` is its
radius (the quad runs from -Size to +Size, see "Round 4"), a mesh's scales the model; with `OrientToVelocity` or `Stretch` the sprite's long axis follows the velocity (`Stretch` = seconds of travel), otherwise
the quad faces the camera and spins with `RotationRate`.

Supported: `Texture`/`Material` (the diffuse map of a `.mtl`), `TextureTiling` (tiles, first tile, variants, frame animation),
`BlendType`, `Facing` (camera, free, flat for Bottom / Water / Decal / Horizontal / Terrain), `OrientToVelocity`, `Size`/`SizeX`/`SizeY`/`SizeZ` with age curves, `Stretch`,
`StretchOffsetRatio`, `Alpha`, `AlphaTest`, `Color` (with curves), `DiffuseLighting`, `DiffuseBacklighting`, `EmissiveLighting`,
`Count`, `Continuous`, `EmitterLifeTime`, `SpawnDelay`, `PulsePeriod`, `ParticleLifeTime`, `PositionOffset`, `RandomOffset`,
`PosRandomOffset`, `Speed`, `AirResistance`, `GravityScale`, `Acceleration`, `WindInfluence`, `InheritVelocity`, `EmitAngle`,
`FocusAngle`, `FocusAzimuth`, `FocusGravityDir`, `InitAngles`, `RandomAngles`, `RotationRate`, `RandomRotationRate`,
`Turbulence3DSpeed`, `TurbulenceSize`, `TurbulenceSpeed`, `SpawnIndirection` (`ParentStart`, `ParentDeath`, `ParentCollide`),
`SoftParticle`, `DrawLast`, `SortOldestFirst`, `CameraMaxDistance`, `ConfigMin`/`ConfigMax`, `MoveRelEmitter`/`BindToEmitter`,
and the round 3 features below: `Geometry` (mesh particles) with `NoOffset`/`NoMaterialAlpha`/`IndependentUVTimer`, `Rotate*`
(orbiting), `Tail*`, refraction materials, `AttachType`/`AttachForm`, ground collisions (`PhysicsType`, `CollideTerrain`,
`Bounciness`, `DynamicFriction`).

Not supported (ignored): lights (`LightSource*`, `LightHDR*`), sounds, decals (`Decal*`), collisions with objects and entities
(`CollideStaticObjects`, `CollideDynamicObjects`; only the ground is hit), `MaxCollisionEvents` (a particle reports its first
ground hit only), `GeometryInPieces`, `ForceGeneration`, `ERotateMode`/local rotation modes and `VirtualAge`,
`BindEmitterToCamera`/`SpaceLoop`, `ViewDistRatio`, `CameraMinDistance` (the data has values like 270 that would hide the
quest beams), `VisibleUnderwater`/`VisibleIndoors`, per frame texture blending, terrain-facing nuances, `FillRateCost`.

## Round 3: what the player skills need

Each feature was checked on real player-skill effects (`data/extract/combat/skill_coverage.tsv`, `cargo run -p aa-client
--example skill_coverage`) with the effects example (`--loop --no-prime`, one shot of a single effect, plus `--stagger` film
strips, `--sweep` for moving effects, `--backdrop` for refraction, `--ground` for collisions).

* **Mesh particles** (`Geometry`, `geom.rs`). The `.cgf` loads on an async task through `aa_cry::mesh::Model::load` (LOD0 nodes,
  `aa_cry::scene::render_transform`, vertices in Bevy axes; the sub-material the most triangles use). Until it arrives the
  emitter draws nothing. The material is the emitter's `Material` `.mtl` (empty: the model's own): diffuse and opacity-map
  textures, `Diffuse` tint, `Opacity`, `Emissive`; blend = `ADDITIVE` flag: additive, opacity < 1 or an opacity map or `Glass`:
  alpha, otherwise opaque with the alpha-test cutoff. `TexMod` UV animation of the `.mtl` runs (tile/offset, constant scroll,
  sine waves, flip books at 10 fps by default; the clock is the effect's). One dynamic mesh per emitter holds every copy: the
  template is transformed per particle on the CPU (scale `SizeX`, `SizeZ`, `SizeY` times `Size`, rotation from
  `InitAngles`/`RandomAngles`/`RotationRate` or `OrientToVelocity`, centred on the bounding box unless `NoOffset`), size, alpha
  and colour curves apply as for sprites. Opaque debris is shaded per vertex by `FxSettings::sun`. A mesh particle counts its
  vertices / 4 against the particle budget. The effect's forward is -Z in Bevy: a model that points along cry +Y flies along
  -Z, so a projectile effect moves its entity along its own -Z (see "What the client should do"). Glass and ice materials look
  pale (no fresnel, no environment reflection).
* **Orbiting** (`Rotate*`, simulation only). A particle with `RotateLength` circles on a path about `RotatePosition` (the axis,
  cry axes, default up; the emitter's rotation turns it unless `RotateWorld`): radius = `RotateLength` x its age curve, angle =
  `RotateInitAngle` + the integral of `RotateSpeed` (radians per second, times its age curve). The orbit offset is added to the
  stored path, so tails and child emitters follow the visible position.
* **Tails** (`TailLength`, `TailSteps`). A particle with a tail keeps a ring of its past world positions (at most 48 samples, one
  per `min(TailLength, life) / steps` seconds, never finer than 1/90 s); it draws as a camera-facing quad strip along that
  path, tapering and fading towards the old end, texture u along the length (the sword and arrow trails of the skills). The
  sprite size is the ribbon's width; the particle's own quad is not drawn.
* **Refraction** (`.mtl` with `RefrBumpScale` and a normal map). Such sprites were plain glow blobs; now they draw as a distortion:
  the material reads the scene behind it (`ScreenSpaceTransmission`, which the plugin adds to the camera when a refraction
  material shows up; Bevy copies the opaque scene once and the sprites draw in the transmissive phase) and shifts the lookup
  by the normal map times `RefrBumpScale` times the sprite's alpha (`REFRACTION_SCALE` = 0.2 of the viewport height per unit).
  The strength was tuned by eye on the heat shimmer and the shock rings (checker backdrop), there is no reference capture.
  `FxSettings::refraction = false` (`AA_FX_NO_REFRACTION=1`) does not draw them at all.
* **Emission from the body** (`AttachType` `BoundingBox`/`Physics`/`Render`, `AttachForm` `Surface`/`Edges`/`Vertices`/`Volume`).
  The effect does not know the mesh of the entity it is on, so the shape is generic: a 0.3 m radius capsule, ankle to above
  the head, for `Physics`/`Render`, a 0.8 x 0.6 x 1.8 m box for `BoundingBox`; particles with a speed leave the surface along
  its normal. Edges and vertices fall back to the surface.
* **`InheritVelocity`**: a new particle adds that fraction of the effect entity's velocity (from its world position, smoothed,
  teleports clamped at 40 m/s), unless it is bound to the emitter.
* **`AlphaTest`**: a sprite with a (curved) alpha test dissolves: the texture's alpha has to exceed the particle's threshold,
  soft edge 0.08. It is carried per vertex in the second UV set.
* **Collisions.** `CollideTerrain` with a `PhysicsType` other than `None` (the data has `CollideTerrain=true` on 500 emitters
  without a `PhysicsType`; as far as known CryEngine ignores it there, so do we): the particle is put on the ground when its
  path crosses `FxSettings::ground(pos)` (or `Effect::ground_y` when the settings have none; neither: nothing collides), the
  vertical speed is reflected times `Bounciness` (never above 1; below 0 the particle dies on impact; a bounce below 0.3 m/s
  stops), the horizontal speed slows with `DynamicFriction`. The first hit of a particle starts the emitters under it with
  `SpawnIndirection=ParentCollide` at that point. Not done: bouncing off objects and characters.

### What the client should do

* Projectiles: spawn the projectile effect as an `aa_fx::Effect` (`prime: false`, `looping: false`) and move/rotate its entity
  along the flight; the mesh is drawn with its own `.mtl` (the combat code's `EffectDef::geometry()` special case for the arrow
  of `st1000b` is no longer needed; `EffectDef::geometry()` stays for anyone who wants the model alone). The effect's forward
  is Bevy -Z.
* `FxSettings::sun`: the direction to the sun, for the debris shading (aa-world knows it).
* `FxSettings::ground`: a terrain (and floor) height query; without it only effects with `Effect::ground_y` collide.
* Effects that follow a bone (the weapon): put the effect entity under the bone, tails and `InheritVelocity` follow its world
  motion.
* The camera needs nothing else: `ScreenSpaceTransmission` is added by the plugin.

## Round 4: size convention, first particle, the slash trail

* **Sprite `Size` is a radius.** The Return cast orb (`reference/ingame/combat_return`, around the left hand) is about 0.39 m across
  in the real client; its emitter has `Size = 0.2`: the quad runs from -Size to +Size (before round 4 this crate drew `Size` as the
  full width, half of that, nearly invisible). The ratio real / old is 1.95-2.2 over the frames. The other checks agree: torches
  and campfires look right with it (compared by eye in the effects example), and the Hariharan mist excess needs no change.
  Mesh particles are unchanged (`Size` 1 is the model's native size). `FxSettings::sprite_half` (default 1,
  `AA_FX_SPRITE`, `--sprite` in the example) is the one switch. Ribbons (`TailLength`) take their width from the same rule.
  The earlier rejection ("`Size` as a radius turns the fog_01 haze of the Nuian field into a blanket") only held for the old light
  gains: a sprite twice as wide covers four times the area, so the mist's light, opacity and count come down instead
  (refit below). Nothing else in the client scales a sprite: the entity `Scale` (0.334 on the step 3 mist, already applied) is the
  only factor, and `fx_particles.scale` of the database is 1 on 2229 of its 2253 rows (the others, 2 to 8, are the combat
  code's, it spawns the skill effects).
* **Refraction and glow.** `GenMask` bit 4 is refraction; a material that also has bit 0x10 (the glow bit: it draws its own colour)
  is drawn as a plain sprite (the Return orb's `love_book_skill1`: with the distortion it was a barely visible lens). The real
  client shows that orb as a bloomed white disc with a purple halo: ours has the halo and the petals but no bloom (a camera
  effect, not the particle's).
* **First particle at time 0.** `EmitterState::begun`: the first update of a continuous emitter spawns one particle at once and
  then goes on with the rate. Test `a_continuous_emitter_emits_its_first_particle_at_once`.
* **Triple Slash (18132) trail.** The plot's effects with a trail (`skill_coverage`): `X2_EFFECTS.sword_trail_test1`
  in steps 1 and 2 and `abillity_skill_table_m.fight.3stage_slash_step3_trail` in step 3, both on bone 9 (the weapon).
  `sword_trail_test1` has, among others, a refraction ribbon `_ref` with
  `sword_line_ref_skill`, `TailLength 20`, and `sword_trail_test16`, the blue-white one: `Color (0.408, 0.616, 1)`, texture
  `sword_line_13` with the material `sword_line_critical`, `TailLength 100`, life 0.5 s, alpha 1 only between 7 and 13 % of the
  life, 0.067 at half of it). They are drawn (the ribbon follows
  the entity), but faint: the texture is an edge lit line, so the ribbon is a thin pale blue arc (`shots/aa-fx/r3/trail6_sheet.png`); the real client adds its bloom to it, and at the 0.3 s spacing of the
  capture the real trail is not clearly visible either (only the blade itself glows white-blue, `real_09.png`). Round 5 made the
  `Facing = Free` ribbon take its width from the emitter's axis (below). The client must move the effect entity with the weapon
  bone every frame (`prime: false`).
* **Debugging.** `AA_FX_ONLY` / `AA_FX_SKIP` (above), `AA_FX_LOG=2`. They showed that the pale green sheets and white blocks of the
  Memoria screenshot (`shots/gallery/29_memoria.png`) are not particles: the screenshot is identical with `AA_NO_FX=1`, with
  `AA_FX_SKIP=waterfall` and with `AA_FX_ONLY` of the only geometry waterfall near it (`X2_EFFECTS_K.waterfall.waterfall_sm_new01`,
  80 m away, its material has Opacity 0.08 and is invisible in the example). The cell's object table
  (`010_014`) places `game/materials/ocean/water_volume_gw01` (a water volume), which is aa-world's.

## Round 5: blade ribbons, flat Bottom, refraction, the first 10 minutes

* **`Facing = Free` ribbons (the Triple Slash trail).** A tailed particle with `Facing = Free` keeps, next to its position history,
  a width axis per sample (`EmitterState::trail_axis`): the emitter's rotation applied to the particle's cry +Y. Sword models run
  along +Y and the hand bone (`item_hand_r`, fx bone 9) holds them unrotated, so that axis is the blade: the ribbon is a sheet
  spanned by the blade and the tip's path, which is what a blade sweep looks like, from any camera. Its edge `v = 0` is the
  tip side (the `sword_line_*` textures are lit there), it does not taper (a blade keeps its length), and it falls back to the
  camera facing strip when the axis is degenerate. The camera facing ribbons (`Facing` Camera) are unchanged. `u` now runs from the head
  (`u = 0`) to the tail: the sword line textures are bright at `u = 0`, and with the old direction the whole trail was very dim.
  Test `a_free_ribbon_takes_its_width_from_the_emitter_axis`; `effects --swing <s>` swings the entity like a hand with a blade
  (0.3 s is a fast cut) and shows it. What the combat code has to do: the entity's rotation is the weapon bone's world rotation
  (so that the model's +Y, the blade, is the effect's cry +Y, Bevy `-Z`), it moves every frame, and the effect is not primed.
* **`Facing = Bottom` is a flat ground quad**, like Water, Decal, Horizontal and Terrain (`Facing::Flat`, 2 cm above the surface
  against z fighting). It used to be a camera facing sprite with a vertical offset. The evidence: the click marker stacks a Decal
  ring and a Bottom ring of the same `ui_target_ring` texture, and the Bottom textures are rings, ripples, cracks and ground fire,
  with `Size` 10 bases of the area skills.
* **`TextureTiling` field 6 is `AnimCycle`** (the animation loops), field 7 `AnimBlend` (blending between frames: not done). Torches,
  campfires and the other flipbooks were compared before and after (`shots/aa-fx/r5/th/default_cmp.png`): unchanged.
* **Refraction no longer paints a rectangle.** A refraction particle replaced the scene behind it by the shifted copy of the scene
  (alpha blended). The copy the camera keeps for it is about 1.4 times brighter than the target (a 72 grey checker square came out as
  101 or 121, and as 255 where quads overlapped), so every `skill_refraction_*` quad was a pale trapezoid (the resurrection and
  Thwart effects). It now adds the difference between the shifted and the unshifted copy (`AlphaMode::Add`, colour `moved - here`,
  alpha 0, `refract_scene` in `fx.wesl`): a flat normal map changes nothing, brightness does not matter, and what was drawn after
  the copy stays. Checked on `shots/aa-fx/r5/th/rl6_n.png` (the checker is exact outside the wobble at the wall's foot).
* **Debugging.** `AA_FX_EMITTER=a,b` draws only the emitters whose name contains a word (next to `AA_FX_ONLY` / `AA_FX_SKIP`, which
  select whole effects): it isolates one emitter of a skill effect.
* **Memoria (`waterfall_splash_smooth01/02/03`).** With the material table fixed in aa-world the cell shows no fog wall: the 40-50 m
  haze sprites are a faint pale veil at the foot of the falls (`shots/aa-fx/r5/mem_a_fx.png` against `mem_a_nofx.png`, the
  world's `fly` example at the cell, 2 s). The standalone effect (`wf_waterfall_splash_smooth01.png`) is the same: a soft haze
  of low alpha, no sheet edges. The check through `aa-client` was blocked by another agent's compile error in `hud/api.rs`.

### First 10 minutes audit

`skill_coverage` now lists the effect names of every skill (`fx_names` column, `data/extract/aa-fx/r5/skill_coverage.tsv`). Film strips
(5 copies, 0.2 s apart, `effects --grid 5 --stagger 0.2 --no-prime --loop`) of the effects below are in `shots/aa-fx/r5/audit/`;
this is what they show and what was changed.

| what the player sees | effect | result |
|---|---|---|
| quest NPC beam | `quest_mark_addon_yellow_loop` | compared with `reference/ingame/spawn/real.png`: the same tall pale column; `CameraMinDistance 270` does not hide it at 36 m in the capture, so it stays ignored |
| level up | `X2_EFFECT_O.com.level_up_new` | good once the main emitters start: they have `SpawnDelay 2.5`, so the first 2.5 s show only the small sparks (a strip needs a longer `--shot-after`) |
| Return / Recall | `return_skill_form`, `return_skill_launch`, `return_hit_launch` | the orb is right (round 4); `love_book_skill1` has bit 0x10 and is drawn as a sprite |
| resurrection | `resurrection_form`, `resurrection_launch` | green light pillars, rune circle and honeycomb disc are right; the pale trapezoid from `skill_refraction_01` was the refraction bug above (fixed) |
| portal | `portal_main`, `portal_idle` | green arcs and lightning veins, no problem found |
| water splash | `water_splash`, `doodad_water_splash` | foam flecks as in the combat captures' pond |
| critical hit | `fight.critical_damage_hit`, `_level2`, `_level3` | particle effects (flare, sparks, blood); they look right (the HUD `critical.dds` burst is not touched) |
| starter skills of the classes | the `skill_coverage` list (about 50 effects) | no systematic fault left after the Bottom and ribbon fixes |

Things that look odd and are, as far as the data goes, authored (not changed):

* Thwart (`p_skill_will.sprit_charge_launch`): `dissonance_hit1` shoots ten `glow_big_velocity3` beams, `SizeY 5`, `Speed 25`; they overlap into bright white
  bars when five copies are stacked in a strip (one copy is a starburst), and `dissonance_hit2/3` are 20 m wide rings (`Size` 11 and 10).
* `hill_attack_form` is made of tall box-like light pillars; `ground_restriction_hit` is a large crack burst with rocks.
* `p_skill_love.resurrection_launch/p_skill_line03` (`Facing = Free` with `OrientToVelocity`): the sheets have random angles here
  and are cut by the ground plane (diagonal edges at their feet). The engine probably aligns them with the velocity; only 38
  emitters combine the two without a tail (provoke, hurbal_remedy, line03, ...), so it is not done.
* `shock_form2` is a mesh particle that lives 0.03 s and is continuous with the default count, so it flickers (3 % of the time).
  `st1000b` / `st1000c`, the Shoot Arrow and Bow Attack effect, draw the mesh `fx_arrowhead_tehgin.cgf` with `fx_arrow_a1`
  (Illum, `Diffuse` black, fresnel only, texture `refract_tile_09`), so it shows nothing here; with a black diffuse the real
  engine may show only a rim of fresnel light. A capture of Shoot Arrow in the real client would settle it.
* No data defines a loot sparkle: no effect in the libraries or the database (`fx_items`, `doodad_*`, `skills`) is named for it
  (`x2_sounds.particles` has pickup sounds only), so it is the client's or the doodad's.

Open question, not changed: the direction of `OrientToVelocity` sprites. This crate puts the texture's top (`v = 0`) in front, along
the velocity. Cloud, flame and dome textures (`cloud_type_f_velocity`, `fire_animated_06a_velocity`, `glow_big_velocity`, `sand_flowup`)
have their content in the upper half of the sprite and fit that, while `ef_glowline01_r` (a comet with its bright head at the bottom)
and the beam textures `glow_big_velocity2/3` would fit the opposite. A client capture of a velocity beam (Thwart) would decide
(`data/extract/aa-fx/r5/velside.py` lists the textures).

## Coordinates

`aa_fx::c2b` maps a cry vector into Bevy (`(x, y, z) -> (x, z, -y)`). Entities of a cell's `entities.xml` carry `Rotate` as
`w,x,y,z` in cry space: place them with `rotation = CRY_TO_BEVY * q * CRY_TO_BEVY.inverse()` (aa-world's `CRY_TO_BEVY`). Effect
scale = entity `Scale` x `Properties Scale`; `bActive=0` effects are not spawned.

## Where it is used

* aa-world: `placement::parse_effect_entities` / `CellObjects.effects` read the `ParticleEffect` entities of a cell;
  `objects/effects.rs` spawns one `Effect` per placed effect (and despawns them with the cell), and keeps `FxSettings::gain` on
  the world exposure. 738 effects around the starting town, about 30 active at a time (100-150 particles, ~0.05 ms).
* aa-client create step 3 (`login/create/scenery.rs`): the ground mist (`flow_fog_01`, `fog_01`) and leaves of the field, with
  a per-race fit of the mist (see below).
* aa-client hud (`hud/marks.rs`): the light beams of the quest marks, `X2_EFFECTS.doodad_effect.quest_mark_addon_{yellow,white}_loop`
  (the `.cdf` of every `objects/ui/quest_mark/*` model binds one to `Bone01`; red/green/blue variants exist).

## Example and numbers

```
cargo run -p aa-fx --example effects -- --shot shots/aa-fx/effects.png
cargo run -p aa-fx --example effects -- --effect X2_EFFECTS.fire_smoke.torchlight_b --effect X2_EFFECTS.e_effect.smoke
cargo run -p aa-fx --release --example effects -- --effect X2_EFFECTS.fire_smoke.torchlight_b --grid 100 --exit-after 14   # 100 torches, logs the cost
cargo run -p aa-fx --example effects -- --effect abillity_skill_table_m.fight.slash_trail_level1 --loop --no-prime --sweep 3 --shot shots/aa-fx/x.png
```

Flags: `--effect` (repeatable), `--grid n` (n copies, cycling through the `--effect` list), `--spacing`, `--dist --height --yaw
--orbit`, `--gain --light --size-scale --alpha-scale --no-soft --max-particles`, `--scale`, `--fx-yaw`, `--day`, and for skill
effects (they are one shots that start from nothing): `--loop`, `--no-prime`, `--stagger s` (a film strip of one effect),
`--sweep r` (the effect moves on a circle: tails, `InheritVelocity`), `--backdrop` (a checker wall: refraction), `--ground`
(collisions), `--lift m` (the effect's height above the floor, for effects that hang in the air), `--sprite h` (`FxSettings::sprite_half`). A single shot of a skill effect needs `--shot-after 4` or more (the models and textures load on tasks).

Release, 100 torches (600 emitters, ~2350 quads, 1080p): simulation 0.15-0.27 ms, mesh upload 0.10-0.15 ms, management
0.01-0.13 ms on the main thread (about 0.3-0.5 ms together), ~100 fps on the dev box with everything else idle.

After round 3 (same machine, but with other builds running, so the numbers jump around by 2x; compared back to back with the
build before round 3 the torches cost the same: simulation 0.2-0.3 ms, upload 0.2-0.4 ms for both). Per particle the cost is
about 60-90 ns simulation + 15-25 ns upload, for sprites, ribbons and mesh copies alike. 100 *skill* effects alive at once are
much heavier than 100 torches because a skill effect has 80-180 particles, not 23: a mixed set of 12 player-skill effects
(ice arrow, slash and trail, hammer, judge spear, ice wall, ...) x 100 copies, all running (980 emitters, ~8000 particles):
simulation 0.56-0.74 ms + upload 0.19-0.22 ms; the worst single ones, 100 x `judge_spear_form` (17,700 particles) 1.3-1.5 + 0.3
ms, 100 x `icearrow_proj` (13,900 particles) 0.8 + 0.15 ms. In a fight there are 10-20 skill effects on screen, which is
0.1-0.2 ms. 12 effects moving on a circle (`--sweep`) with tails and meshes: 0.11-0.15 + 0.02 ms (1000 particles).

## Step 3 mist (fit on the captures)

The library's plain values draw almost nothing in the character create fields (`flow_fog_01`: grey sprites, Alpha 0.4 x a curve
that peaks at 0.6, a texture whose alpha averages 0.15), while the captures show a pale, warm veil. What was tried, against
`reference/character_create/step3_*` (`data/extract/aa-fx/fit.py`, `evalfit.py`; ours rendered 6 times per race, 2 genders x 3
shots, against a run without the mist, `AA_CREATE_NO_FX=1`):

* Blend mode: `BlendType` is absent, i.e. alpha blended, which is what is drawn (not the reason).
* Particle colour in gamma space: would make it darker (not the reason). Entity `Scale`, `CountScale`, `bPrime`: 0.334..0.646,
  1, primed. Soft particles: off in the stage (no depth prepass), they could only thin the mist. Unscaled effects (scale 1):
  1.5-3x more mist but also wrong sizes and speeds. `Size` as a radius (sprites twice as wide): the fog_01 haze of the Nuian
  field becomes a blanket at the old light gains (the fit of round 2 and 3); round 4 uses the radius (the Return orb needs it) and
  fits the light, opacity and count again (table below).
* Switching on the `bActive=0` mist and haze effects of the cell: the near `flow_fog_01` of the Hariharan field washes the near
  grass (the capture has none there), `fog_01` makes the Elf and Hariharan scenes hazy over their whole width. No.
* Count: reading `Count` with `EmitterLifeTime` / `PulsePeriod` as above (x7 for the flowing mist, no change for `fog_01`
  and every emitter without a finite life) fits the Nuian capture without any factor. Kept.
* What is left is the light: the captures' mist is lit (pale, warm: peak colour 190, 176, 138 in Hariharan) where the library
  colour times the texture is a mid grey (about 125, 122, 110 on screen even at a light gain of 3). So the colour gain is a stage light, fitted per
  race, plus some more opacity (and in Hariharan more particles) in the three fields whose capture shows more mist than one source
  makes. Unexplained: the same effect, `flow_fog_01` at scale 0.4, is a clear veil in the Hariharan and Ferre captures and a thin
  one in Nuian.

Final values (`mist()` in `scenery.rs`, flowing mist opacity x, count x, light RGB gain; the haze `fog_01` only takes the light):

| race | alpha | count | light (r, g, b) | peak colour ours / capture | band excess ours / capture |
|---|---|---|---|---|---|
| Nuian | 1.0 | 1 | 2.6, 2.7, 1.55 | 1.00, 1.02, 1.07 | n/a (see below) |
| Elf | 1.0 | 0.5 | 5.5, 5.4, 2.7 | 0.91, 0.91, 1.02 | n/a (see below) |
| Hariharan | 3.0 | 3 | 3.35, 3.0, 1.95 | 1.02, 1.04, 1.04 | 0.86, 0.85, 0.91 (ours 48, 51, 50 against 56, 60, 55) |
| Ferre | 1.0 | 0.6 | 7.9, 6.8, 4.6 | 0.94, 0.95, 0.91 | 1.52, 1.32, 1.02 (ours 38, 37, 52 against 25, 28, 51) |

(Round 4, with `Size` as a radius; the round 2 and 3 values, sprites as full width, were Nuian 1.0 / 1 / 3.0, 3.15, 1.9, Elf 3.0 / 1
/ 6.0, 6.0, 3.4, Hariharan 3.0 / 3 / 4.0, 3.6, 2.35, Ferre 1.3 / 1 / 7.6, 6.6, 4.1. A sprite twice as wide covers four times the
area: the light comes down 13-20 % (Nuian, Hariharan), the Elf opacity and count down to 1 / 0.5, the Ferre ones to 1 / 0.6.
The peak pick has a noise of about 4 % between runs: a second Elf / Ferre set with other count and light products came out
the same within that. The grass under the Elf band still gets +18..+22 of mist where the capture has +5..+9: the Elf haze
and the flowing mist cannot be told apart by one gain, and lowering it takes the peak with it; kept.)

Peak colour: the mean of the 12 brightest, least saturated, least varying 24 x 14 px tiles of the mist band (the same pick on
both images). Band excess: the mist's added light in the band (ours: with minus without the mist; the capture: the band minus
the clear control scaled by our grass's band/control ratio), in 0..255 steps; 5 % is out of reach, between two runs the same
setting moves the excess by 10-20 % (random particles). Nuian and Elf have no usable excess: the capture's grass is much brighter
than ours, so its band minus control comes out negative, and in Nuian our `fog_01` haze also covers the control region. The near
grass under the band gets +6..+9 of mist in Nuian, +1..+8 in Ferre, +18..+22 in Elf (round 4), and none in Hariharan.

Known gaps: `elf_fog_01` (placed in the Elf and Ferre fields) does not exist in any library of the pak, so that mist is missing (the
Elf capture shows more mist than the two flowing sources make, so it may have existed); the brightness of lit particles is not tied
to the time of day (`FxSettings::light` is a constant).
