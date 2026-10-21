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
  and `tint` (a colour) the light on its particles (on top of `FxSettings::light`). `no_refraction` leaves out the refraction emitters of this effect (Dash). `once` names emitters (a name part) that emit their first cycle only (`EmitterDef::first_cycle`: a continuous emitter with no `EmitterLifeTime` gets its particle's life as its own; Dash's repeating shell `speed_full_pc_meshe_*`). `ground_y` is the height of a flat ground
  for this effect (collisions, see below).
* `FxSettings` (resource): `max_particles` (20000, the hard global budget), `max_distance` (160 m activation radius cap),
  `gain` (colour multiplier; a world with pre-exposed HDR colours sets `radiance * exposure`, aa-world does), `light`
  (0.8, how much of `DiffuseLighting` shows), `emissive`, `hdr_base` (2: `EmissiveHDRDynamic` and `LightHDRDynamic` are powers of it), `max_lights` (8) and `light_lux` (1700), `size_scale`, `sprite_half` (1: a sprite's `Size` is a radius), `alpha_scale`, `quality` (0..3, `ConfigMin/Max`),
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
radius (the quad runs from -Size to +Size, see "Round 4"), a mesh's scales the model. A camera-facing sprite's long axis follows
the velocity with `OrientToVelocity` or `Stretch` (`Stretch` = seconds of travel); otherwise it spins with `RotationRate`.
Free sprites put their up (SizeY) axis on the velocity with `OrientToVelocity` after the angles roll them (see below).

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

Not supported (ignored): sounds, `DecalMode` / `DecalSortPriority` (decals are draped on the ground, see round 7), collisions with objects and entities
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
* `shock_form2` is a mesh particle that lives 0.03 s and is continuous with the default count, so it flickers (3 % of the time).
  `st1000b` / `st1000c`, the Shoot Arrow and Bow Attack effect, draw the mesh `fx_arrowhead_tehgin.cgf` with `fx_arrow_a1`
  (Illum, `Diffuse` black, fresnel only, texture `refract_tile_09`), so it shows nothing here; with a black diffuse the real
  engine may show only a rim of fresnel light. A capture of Shoot Arrow in the real client would settle it.
* No data defines a loot sparkle: no effect in the libraries or the database (`fx_items`, `doodad_*`, `skills`) is named for it
  (`x2_sounds.particles` has pickup sounds only), so it is the client's or the doodad's.

Open question, not changed: the direction of camera-facing `OrientToVelocity` sprites. This crate puts the texture's top (`v = 0`) in front, along
the velocity. Cloud, flame and dome textures (`cloud_type_f_velocity`, `fire_animated_06a_velocity`, `glow_big_velocity`, `sand_flowup`)
have their content in the upper half of the sprite and fit that, while `ef_glowline01_r` (a comet with its bright head at the bottom)
and the beam textures `glow_big_velocity2/3` would fit the opposite. A client capture of a velocity beam (Thwart) would decide
(`data/extract/aa-fx/r5/velside.py` lists the textures).

## Round 6: skill audit

Every effect of the shown player skills rendered alone and the suspicious ones compared with the real 1.2 client (Lua
`Particle.SpawnEffect` through the client driver). The full table, the open problems and what the spawn side owes the effects are
in `docs/research/skill-audit-fx.md`. The engine was read in the decompile of `cry3dengine.dll` (IDA, emit function `sub_30059310`,
sprite writer `sub_3009CFD0`, `ResourceParticleParams`) and `particles.cfi`; the facts below are from there unless marked "real".

* **FocusAngle** tilts the emitter axis (+Y) about X toward +Z, then `FocusAzimuth` (default 0, `base + rand * var * 360`)
  spins the tilt about the axis. The old code used a random azimuth: `FocusAngle="90"` (4427 emitters) flew sideways. Real:
  `el_wind0004_hit` rises like ours now. The emitter's orientation in the real client comes from the spawn direction (`dir` is cry +Y).
* **Tile aspect.** Width over height of one tile (`ComputeAspect`): if >= 1 the height is divided by it, otherwise the width is
  multiplied by it; the longer side keeps the authored `Size`. (Was: height times h/w, clamped to 4.) Real: `hitup`.
* **Camera facing sprites** are a 2x2 matrix over the camera right / up: roll, `OrientToVelocity` turn (the roll is added to the
  alignment), stretch grows each row along the screen velocity by `velocity * stretch * (row . dir) / |row|` and moves the centre by
  `StretchOffsetRatio * stretch * velocity` (world velocity). A sprite with no screen velocity is a plain billboard.
* **Facing = Velocity** is a 3D card: the engine rotates the card's +Y (normal) the shortest way onto the direction of travel
  (`Particle::dir`: the emit direction until the particle moves faster than 1 mm/s, then its velocity).
* **Lit meshes.** `Illum` material: colour = texture * Diffuse * (`LIT_SUN` * N.L + `LIT_SKY`), 12 and 2.9 (the engine's 7.6 and 1.8
  noon values times about 1.6 of exposure, fitted on the rock of `fight.ground_launch_standalone` against the real client). Other mesh
  materials keep the old emitted colour.
* **Material before Texture** when both are set. **AnimBlend** (`TextureTiling` field 7): two quads weighted by the fraction of the
  frame. **Colour gamma**: `pow(colour, 2.2)` before the texture (`FxSettings::colour_gamma`, `AA_FX_GAMMA=1` is the old look; the
  first Step 3 mist fits were made with 1; round 7 fitted the mist again at 2.2).
* **Endless tails** show one second of path at most.
* Tools: `effects --list <file> --cols n --times a,b,c --out <prefix>` (contact sheets), `--real <dist>` (the camera of the real client
  captures, 1.68 m above the floor, vertical fov 42.75 degrees), `--stagger`; scratch scripts and the decompile under
  `data/extract/aa-fx/r6/` (git-ignored: `real/cmp.py <effect> <tag>` spawns the effect in the real client and renders both).
* Open: `white0001_hit` rings are 3D in the real client, camera facing here (lace look done, round 8 below).

## Round 7: lights, decals, HDR, children that ride, mist, brightness

* **Particle lights** (`LightSourceIntensity`, `LightSourceRadius`, `LightHDRDynamic`; 2307 emitters carry them, typically intensity 1,
  radius 2-5 m, HDR 1). The engine (`CParticleContainer::RenderLights`, `sub_3009A9A0`) makes one dynamic light of every particle:
  colour = the particle's colour times its age curve (not gamma raised, not lit), times `intensity * rand * ageCurve`, times
  `pow(hdr multiplier, hdr * rand * ageCurve)`; radius = `LightSourceRadius * rand * ageCurve` in metres (the entity scale does not
  scale it); the light is skipped when `intensity * radius` is below 0.001 and by the camera frustum / distance tests. aa-fx collects
  those samples per effect in the simulation step, `update_lights` picks the brightest and nearest `FxSettings::max_lights` (score
  `luminance * r^2 / (d^2 + 1 + r^2 / 4)`) of all active effects and puts pooled Bevy `PointLight`s (no shadows, `range` = radius) on
  them: `FxLight` entities, spawned once, hidden when unused, copying the effect's `RenderLayers`. Lumens are chosen so that an
  intensity 1 light gives `light_lux` lux at half its radius (pi times the world's radiance for a pre-exposed world; 1700 is
  pi * 540, aa-world's value). **Cost**: 0.01 ms of main thread time for 100 candidates and 8 lights (100 torches, release build),
  nothing measurable in frame time up to 32 lights on this machine (the example's 70-90 fps wandered with the load of other
  sessions); clustered forward shading costs per light, 8 is the default cap. `AA_FX_LIGHTS=0` switches them off. Stats:
  `FxStats::lights`, `light_candidates`, `light_ms`. Test: `torchlight_a`, `candle_a`, `e0004` in `effects --max-lights 0|8`
  (`shots/aa-fx/r7/lights_*.png`).
* **Decals** (`Facing = Decal` 661 emitters, `Terrain` 13). The engine projects them on what is below; the Free / Bottom / Water cards
  stay flat. aa-fx drapes them: when `FxSettings::ground` knows the height under the card, the card is cut into cells of about
  1.5 m (at most 12 x 12) whose corners lie 4 cm above the ground (`sim.rs`, `decal.rs`); without it a flat card as before.
  `DecalAlphaMultiply` multiplies the opacity of the decal emitter (0 or absent: 1). `aa_fx::decal::drape_quad(centre, right, up,
  ground, lift, cells)` is the same cut for any card of your own and returns positions / uvs / indices: the ground target reticle
  (`materials/Decals/target/target_correct` is a decal in the client; `ingame/ground_target.rs` drapes its own mesh the same way) can
  call it. `effects --bumps <m>` draws a rolling ground that the effects know; Freezing Earth (`ice_piece_launch`) and Crippling
  Mire follow it (`shots/aa-fx/r7/decal_*.png`). Not done: projection onto objects (only the ground function is asked), `DecalMode`
  (all are draped), the sort priority.
* **EmissiveHDRDynamic** (1602 emitters, values 0.1-4, mostly 1-2): the engine's mesh path multiplies the colour by
  `DiffuseLighting term + EmissiveLighting * pow(S, EmissiveHDRDynamic)` when the renderer draws in HDR (`sub_3009C0B0`), `S` being
  the 3D engine's HDR multiplier (its value is not in the code: `FxSettings::hdr_base`, 2). aa-fx applies the factor to the emissive
  term of sprites too (the writers show no other place for it). `AA_FX_HDR=1` switches it off.
* **Children that ride on a parent particle.** A `SpawnIndirection = ParentStart` child with an `EmitterLifeTime` is attached to its
  parent particle while its life lasts (`CParticleContainer::UpdateChildEmitters`): `Count` over the life, emitted wherever the parent
  is. `firewall_launch`: the parent particle starts 3 m up and falls at 15 m/s, the fire puffs are emitted for 0.2 s along that path,
  a 3 m column; aa-fx emitted them all at the birth point (a ball, half as tall). Fixed (`Follower` in `sim.rs`); the column is now as
  tall as the real one (`shots/aa-fx/r7/fw_cmp.png`, top ours, bottom real; the real one is wider and brighter).
* **Colour keys with two components** (418 keys): the missing blue is 0, not the default 1 (`(0.282,0.09)` is dark red,
  `(0.035,0.027)` black; with 1 the end of the fire columns was violet and the black smoke blue). Base colours keep the default
  (`(0.14,0.4)` is ice blue).
* **Material `AlphaTest` (lace), round 8.** 219 sprite materials of the libraries have an `AlphaTest > 0` (1087 emitters, 685 top-level effects);
  175 of the materials also have the deforming shader (`GenMask & 0x10`, `PerturbationScale`...). `FxAssets::lace(mtl)` reads the cutoff, the
  noise map (`Custom`, else `Normalmap`) and `AnimSpeed` / `HorizonAnimSpeed` / `PerturbationScale` / `PerturbationStrenght` (defaults 0.2 / 0 / 0.5 / 0.01)
  into a `LaceKey` of the `MatKey`. `fx.wesl` does what `ParticlePS` does: the diffuse map is sampled again at `uv + n * strength * alpha` where `n` is
  the noise map sampled in screen space (two taps, `(proj - 0.5) * scale * clip_w * 0.05 + 0.5 + time * speed`, the engine's `* 2 - 2` bias kept)
  and then `clip(tex.a * vertex.a - cutoff)`. The threshold is therefore animated by the particle's alpha over its life: the ring is eaten into
  lace as it fades, which is the "dissolve" of `white0001_hit_exp_ssd` (`hit_pattern`: 0.366, scale 8.8, strength 2.195). A deforming material
  repeats its diffuse map (the engine's default sampler; the others stay clamped). Sprite materials only (mesh materials keep their `.mtl`
  cutoff, the per-particle `AlphaTest` dissolve is separate). The soft-particle fade is applied after the clip. Checked against the real
  client (`bcmp.py w1`): the lace look matches (sparse chains with ragged edges that thin out over the life); the other 5 real comparisons
  that use such materials (`d6`, `gl`, `js`, `rv`, `ws`) do not visibly change; a 24-effect contact sheet (`shots/aa-fx/r7/atest_*`) shows
  rings (`ring_fire4`, `ring_ssd`, `base_pattern*`) now as chain lace, as in the real hit. Only `white0001_hit` has a real-client capture of a
  lace material, the rest is judged by the shader. Still different there: the real rings are one flat and two vertical (3D), camera facing here
  (nothing in the engine code orients them, round 7 notes: spawn path at line 65700 of `data/extract/aa-fx/r6/ida/cry3dengine.c`).
* **Brightness against the real client.** The 15 captures of round 1 (`data/extract/aa-fx/bright2.py`: excess over the empty scene, top 2 %
  of the effect pixels, ours / real): the effects that are clearly visible in both (`fw`, `gr`, `hp`, `ip`, `js`) come out at 0.5-1.3 on the
  peak, medians 1.1, 1.0, 1.1 (r, g, b) with the HDR factor and the gamma in; `gl` (the rock) and `rv` (the red armour) are 2.5 times too
  bright and the rest cannot be measured (a persistent ice effect of my own test hides the real scene in half of the captures, and shape
  differences dominate). No global change is justified: `gain`, `light` and `emissive` stay.
* **Step 3 mist re-fit** (colour gamma 2.2, FocusAngle, tile aspect, radius sized sprites): table in the mist section. Display scaling:
  Windows was set to 125 % between two of the fits; aa-dev windows then came out 1536 x 864 for `--size 1920x1080` and every region of the
  fit moved: pass `--size 2400x1350` to get 1920 x 1080 shots (the fit script does).

## Round 9: oversized or wrong geometry in the after reel

Sunder Earth, Whirlwind Slash, Ollo's Hammer, Freezing Earth and Hell Spear, rendered with `effects` and read against the reel frames and
the reel logs (`AA_FX_LOG=2` run of the battlerage reel with the `after` exe, tag `fx9`). Tools added: `effects --fly <m/s> --fly-len <m>`
(with `--list`: the effects fly along their forward axis like a projectile), `AA_FX_EMITTER_SKIP=a,b` (emitters that run but do not draw, so
that their children still start; `AA_FX_EMITTER` stops the children), `AA_FX_PROBE=1` (logs `aa-fx-probe <effect> quads= extent=` as the
maxima of an effect grow: an effect that draws nothing has no line). `data/extract/aa-fx/r6/real/fly.py <effect> <tag> [speed] [len]` flies an
effect on a script-moved entity in my own real client (`System.SpawnEntity` + `LoadParticleEffect` + a timer; the entity has no physics, so the
emitter velocity of the real engine is 0 there).

| Skill | Cause | Side |
|---|---|---|
| Ollo's Hammer | the wide cyan band is `haje_hammer_proj5` (`form_blue1`, `Stretch 1`, `InheritVelocity 1`): the sprites inherit the projectile's 14 m/s and the stretch (seconds of travel) made them 14 m long, flat to the camera. Real (hammer on a moved entity): tumbling cubes with blue smoke wisps, no streak. | aa-fx, fixed |
| Sunder Earth | the projectiles 139 / 140 / 141 (`ground_launch_standalone*`) are `proj_physic_id` 3. The 257 projectiles of that physic are all named "effect area" (`..._효과지역`, `효과지역5m`, `마법진_효과지역`): a zone effect that stands on the ground at the target. `Projectile::falls()` makes it a thing that drops from 30 m in 0.25 s, so the smoke, the rocks and the debris of the effect start in the air above the caster (the reel log: effect at 137.6 m, ground 131.1) and trail down. The effect itself is right (`effects`: grey spikes, dust, cracks, as the real `gl` capture). | combat (spawn) |
| Whirlwind Slash | the long streak is the equipped two-hander (mainhand 5660) in its horizontal pose, not a particle: the swirl rings are 1 to 3 m and the swoosh ribbons (`swirl_slash_swoosh_level*`, `sword_trail_test1`, 1.9 m) draw only while the bone moves (they need `prime: false`). The blade looks 2.4 m long next to a 1.8 m character: check the weapon scale. | animation / weapon |
| Freezing Earth | by data: the skill radius is 8 m; the emitters are a 10 m refraction ripple (`skill_refraction_00`, camera facing, bump 0.035), the `decal_ice_pattern` decal (`Size 10`) and a 13 m ring decal (`ring13b`), all on the caster (`fx_location_id` 1). The real capture of the same effect (`r_ip_*`) shows the same wide shimmer over the lake. The bright band across the frame at waist height is the ring decal seen at 1.5 m camera height. | none |
| Hell Spear | by data: radius 6 m, decals `Size 9` (`decal_death_pattern_black`) and 7, red spears over 8 m, on the caster (`fx_location_id` 1). The circle covers a stage of 4 m. A real capture of the decal was not possible (it did not show on a spawned effect over the lake bank at night). | none |

* **Stretch is not drawn over the inherited velocity** (`Particle::inh`). A sprite stretches and orients along `vel - inh`, where `inh` is
  the part of its velocity that came from the emitter's own motion (`InheritVelocity`, decays with the air resistance). 794 emitters of the
  libraries combine `InheritVelocity` with `Stretch` or `OrientToVelocity` (the projectile trails, `el_fire0001_proj_*`...): with the old rule a
  hammer at 14 m/s had 14 m streaks. Contact sheet of 8 projectile effects flying at 12 m/s before / after: the heads are rounder, the
  streaks are shorter, nothing else moved (`shots/aa-fx/r9/proj_cmp.png`). Not verified against a real moving projectile (the entity of my real
  test has no physics velocity); the evidence is that the real hammer shows wisps and no 14 m band. The defense reel recorded again with this build
  (`shots/reel/fx9/03_defense.mp4`, 960x540, 20 fps) shows the hammer cubes, the petals and the burst and no band (the stage in that run had
  lost its doll: the hit landed on another unit, run `run_reel.py setup`).
* **"No visible effect" list of the reel, fx side.** The listed skills have 136 effects (skills' fx group, visual groups and projectile
  groups). All of them draw (`AA_FX_PROBE`, sheet): 126 with 3 to 560 quads and 1 to 40 m extent. The ones that draw nothing are the sound
  emitters (`x2_sounds.particles.*`, `*_sound`), the trails that need a moving bone (`critical_damage_sword`, `sword_trail_test1`,
  `nanta_swoosh`) and two variants whose emitters are all `Enabled="false"` (`buble_form2`, `fireball_form2`). `Boneyard` and `Flame Barrier`
  have no fx item at all. So an effect that is missing in the reel is not started (spawn side) or is subtle by data and the daylight
  (additive glows over bright grass: Terrifying Roar is a faint orange ground glow and a ring, as in `effects --day`), not a draw failure.
  The skills' effect lists are in `data/extract/aa-fx/skill_fx.json` (`skill_fx_list.py`).
* **Spawn side, for the combat agent**: physic 3 projectiles are zones on the ground, not falls (above); everything listed in the table as
  "none" is placed where the data puts it (location 1 = the caster, 2 = the target).

## Free sprite velocity alignment

`Facing = Free` honors `OrientToVelocity` for sprites without a tail, as the engine does (read in cry3dengine, `particle::write_sprite`
`+0x9cfd0`): the card's SizeX is on its
local X, SizeY on its local Z (up), its normal is local Y. `InitAngles` and `RandomAngles` roll the card first (x, then y, then z, in
the particle's own frame); then the card spins about its normal until its up axis lies along the velocity projected into the card's plane. A
velocity along the normal leaves the card alone. (An earlier round put the texture's X axis on the velocity, from the public CryEngine
reference; that drew Dash's `speed_full_pc2` streaks, `SizeY` 10 with `InitAngles` 90, as 6 m bars across the flight.) The card stays in
3D space as the camera moves. The `Facing = Camera` convention discussed above is unchanged.

The velocity is converted out of the bound emitter frame first. Small nonzero velocities still align: Provoke and the housing
construction effects use `Speed = 0.001` or `0.01`, below the camera sprite's 0.02 m/s threshold. Zero velocity preserves the
authored free orientation. This corrects the ignored flag on resurrection's `p_skill_line03`, Provoke, construction light lines,
and moving debris; it requires no client API change. The independently authored sword ribbons retain their blade-width axes.

Regression tests cover slow upward and downward travel, rotated and bound emitters, a card flying along its normal, Dash's streaks, and
stationary cards. All 27 crate tests pass. There is no real-client resurrection/Provoke capture in `reference/`; this behavior is
checked against the engine documentation and particle-library data rather than a claimed visual match.

Viewed example captures are in `shots/aa-fx/r7/`: `resurrection_free_before.png` / `resurrection_free_after.png` isolate
`p_skill_line03` in five staggered copies (0.12 s apart). The formerly parallel columns now turn with each particle's travel.
`provoke_free_before.png` / `provoke_free_after.png` check the slow ring emitters remain flat with their authored angles, and
`resurrection_full.png` checks the complete effect at scale 0.35. The copied dev example logged 398 fps for the isolated
resurrection strip (27 particles, simulation 0.029 ms, upload 0.003 ms), 389 fps for Provoke, and 362 fps for the complete effect.

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

Final values (`mist()` in `scenery.rs`, flowing mist opacity x, count x, light RGB gain; the haze `fog_01` only takes the light). Round 7,
colour gamma 2.2 (`pow(colour, 2.2)` darkens the sprites, so the light gains came down by 1.3 to 2.3 times from the round 4 table):

| race | alpha | count | light (r, g, b) | peak colour ours / capture | band excess ours / capture |
|---|---|---|---|---|---|
| Nuian | 1.0 | 1 | 1.85, 1.8, 1.3 | 1.00, 1.02, 1.01 | n/a (see below) |
| Elf | 1.0 | 0.5 | 2.75, 2.75, 1.95 | 1.04, 1.02, 1.03 | n/a (see below) |
| Hariharan | 3.0 | 3 | 2.0, 1.98, 1.77 | 0.97, 1.00, 1.00 | 54, 57, 59 against 55, 60, 51 |
| Ferre | 1.0 | 0.6 | 5.8, 3.4, 2.6 | 1.06, 1.14, 0.98 (two runs) | 53, 53, 59 against 11, 19, 51 (the capture's control is not clear) |

(The peak pick has a noise of 4 to 8 % between runs; the Ferre green does not go down with its gain, the pick takes the least saturated
tiles. Round 4 values with gamma 1: Nuian 1.0 / 1 / 2.6, 2.7, 1.55, Elf 1.0 / 0.5 / 5.5, 5.4, 2.7, Hariharan 3.0 / 3 / 3.35, 3.0, 1.95,
Ferre 1.0 / 0.6 / 7.9, 6.8, 4.6.)

Peak colour: the mean of the 12 brightest, least saturated, least varying 24 x 14 px tiles of the mist band (the same pick on
both images). Band excess: the mist's added light in the band (ours: with minus without the mist; the capture: the band minus
the clear control scaled by our grass's band/control ratio), in 0..255 steps; 5 % is out of reach, between two runs the same
setting moves the excess by 10-20 % (random particles). Nuian and Elf have no usable excess: the capture's grass is much brighter
than ours, so its band minus control comes out negative, and in Nuian our `fog_01` haze also covers the control region. The near
grass under the band gets +6..+9 of mist in Nuian, +1..+8 in Ferre, +18..+22 in Elf (round 4), and none in Hariharan.

Known gaps: `elf_fog_01` (placed in the Elf and Ferre fields) does not exist in any library of the pak, so that mist is missing (the
Elf capture shows more mist than the two flowing sources make, so it may have existed); the brightness of lit particles is not tied
to the time of day (`FxSettings::light` is a constant).

## Round 10: stretched beam anchors, faint effects

**Beams (`proj_physic_id` 2).** The combat code (`combat.rs`, `update_beams`) puts the effect on an anchor at the launch bone, turns it to
the target and scales its Z by the link length (`beam_test.cgf` is one unit long, the effect's forward is -Z); `(lx + ly + lz) / 3`
averaged that into a uniform scale, so the beam never reached the doll. Now `StepCtx` carries `scale` (the middle of the three axis
lengths, the effect's uniform size) and `axis` (the three lengths over it, `(1, 1, L)` for a beam, exactly 1 for a uniform scale):

* offsets (`PositionOffset`, random offsets, attach points) and the initial velocity are stretched along the entity's own axes
  (`StepCtx::stretch`: `rot * (axis * (rot^T * v))`), so sprite emitters (spark and flare streams), ribbons and tails (they follow the
  particle path) cover the whole link;
* mesh particles (`beam_test_water.cgf`, `lightning.cgf`...) are drawn as `rot * diag(axis) * turn * diag(size)`: the anchor's stretch
  is applied after the particle's own orientation, so a mesh one unit long on cry Y spans the link while its width stays the authored one;
* sizes (sprite radius, tail width, mesh x/z) and orbit radii stay at `scale`, so a longer link does not fatten the effect.

`effects --beam <m>` stretches the effect entity like the anchor (`--fx-yaw 90 --loop --no-prime`); `shots/aa-fx/r10/l1.png` (Arc
Lightning, 4 m) and `sheet_beams.png` (Absorb Lifeforce, Telekinesis, Lasso, Mana Force, Crippling Mire's `psychokinesis_launch`, 4 m, in this
order) show every beam spanning the whole 4 m. The launch effects are 0.05 to 0.2 s particles: a screenshot needs `--loop`. Test:
`a_stretched_anchor_stretches_offsets_and_velocities_but_not_sizes`.

**Summon Wraith (`ghost_summon_launch`), Mana Force, Telekinesis in daylight (against the real client, my own account, 5 m / 8 m, day).**

* `ghost_summon_launch` was a real bug: `ghost_summon_launch4` has `RotateLength 50` (and `el_fire0001_proj_trail_smoke` 180) with no
  `RotateSpeed` and no `RotatePosition`; we put every particle on a 50 m circle, so the red smoke was 50 m away and only the grey decal
  showed (the faint ring). The real client draws a dense red cloud rising from the spot, low over the water, 3 to 5 m wide. An orbit now needs a
  `RotateSpeed` or an authored `RotatePosition` (22 emitters of the libraries were radius-only; the 100 with an axis keep their ring, e.g.
  `mon_blaze_launch_a1`). The decal is not visible over water in the real client (the pond), that is by the engine. Test: `a_radius_alone_does_not_orbit`.
* Remaining gap, measured and not closed: the real cloud is more opaque and more saturated than ours (core about 157, 5, 5 on screen against
  our 93; green of the water behind goes 70 to 32 against 44). The cause is not a missing term of the particle: I tried gamma 1 (110
  against 165), `FxSettings::light` 2.5 and `alpha_scale` 3 to 5 (these make the carpet, with the right red but too wide), and an
  unmultiplied premultiplied blend (`One, OneMinusSrcAlpha`, rgb not scaled by the vertex alpha). The last matches the real ghost and
  blood clouds and the fountain of `vital_absorb_dot` (the dark `fireball_fire` smoke of ours vanishes, as in the real client), but it makes
  squares of straight textures (`zz_flare`: constant rgb) and over-brightens `ice_piece_launch`, `judge_spear_hit`, `hammer_proj` (real comparisons,
  seven of the round 6 captures), so it is not the engine's rule and was removed. The textures are the clue: 225 of the 630 textures of
  default-blend emitters are authored premultiplied (rgb = 1.0 to 1.3 x alpha, e.g. `smoke_massive_smoke_gren2_additive.dds`, `exp_piece_21`,
  `fire_flame_8x8`); `particles.cfi` multiplies `tex.rgb * vertexColour.rgb` and blends with `SRC_ALPHA` in the alpha-blend path, which for such
  textures gives rgb x alpha squared; the real picture needs about 3 to 5 times more. What is not known: whether the real
  blend state or the texture loader treats them differently. The mist fit (colour gamma 2.2 and light gains of 1.85 to 5.8) points the same
  way: lit smoke wants far more than `light` 0.8; in the game `Effect::tint` (the scene's light) already carries that, the `effects` example does not.
* `spell_move_form` / `vital_absorb_form` / `spell_move_launch_form` (additive glows, alpha 0.17 to 0.93): in the real client they are small,
  clearly visible flares at 8 m (a red glow with a halo about 2.5 times our radius, a violet star with spikes, a pink flare), partly because the
  client blooms bright additive pixels (`aa-world/post.wesl` has no counterpart in the `effects` example: the halos are the missing "glow"; the
  particle data is right). `vital_absorb_dot` is a rising wisp of violet light in the real client; ours has the same shapes but the grey
  smoke layer (`Color 0.118`) darkens them (the alpha-blend question above).
* The beams' own effects are short and faint by data (`lightning_magic_launch` 0.2 s, `vital_absorb_launch` additive streaks), and the
  lights: none of these has a `Light*` attribute, so there are no lights to add.

## Round 11: effect names, character effects, checks

* **Effect lookup.** The real client (`sub_30092A00`) finds an effect by its exact `library.group` name and logs "Particle effect not
  found" otherwise: no last-part fallback. Missile Rain's `wild.long_distance_shot_launch` is such a miss (the library has
  `test.long_distance_shot_launch`), so the missing arrows are also missing in the real client. `FxSettings::name_fallback` (default off,
  `AA_FX_NAME_FALLBACK=1`) uses the single effect of the library whose last name part matches (`Library::get_by_last_part`): it
  resolves 3 of the 50 unresolved names, the other 47 exist nowhere. aa-client can switch it on if it wants the arrows.
* **Character effects (`FxChr`).** `aa_fx::ChrEffect { path, scale, life, ending }` (module `chr`): a skinned `.chr` that plays the
  `start` animation, then `loop`, then `end` and despawns (`life` seconds or `ending = true`). Built from aa-anim's public API
  (`shared_rig`, `spawn_bones`, `Animator`, `Cal`) and aa-cry's `spawn_loaded`; `FxPlugin` adds `AnimPlugin` if missing. Effects example:
  `--effect game/objects/effects/crow/crow.chr` (`--chr-life <s>`); `shots/aa-fx/r11/chr_crow_anim.png` shows the two crows flying.
  Crow, golem hand and the fepee birds play; the pet net draws nothing (not looked into). Needed from aa-anim (not edited): the `.caf`
  controller chunks of version 0x827 (crow start/loop, pet net, golem hand: 399 clips, skipped by `Clip::parse`) are decoded here in
  `chr.rs` `decode_827`: `u32 key count, u32 controller id`, then per key `i32 ticks, f32 x3 position in centimetres, f32 x3 rotation log`
  (`exp`: axis times half angle), values absolute like v0x829. aa-anim could take that decoder over; then `skipped_controllers`
  stays 0 and the local one is not reached. The wiring is aa-client's (`skills.rs` only reads `FxParticle`, `FxSound`, `FxVoice`).
* **Checks without code.** Decal rings of Stillness, Magic Circle, Banshee Wail and Vicious Implosion draw (see the audit, round 11).
  The mesh beams with `ForceGeneration="Target"` (Telekinesis, Mana Force) draw when the entity is stretched; `ForceGeneration` is not read and
  not needed. The effects example needs `--list` (the scene is only ready after about 1.5 s) or an effect that outlives the load for a
  one-off shot. Frigid Tracks' orange flames are Searing Rain's lingering `fire_rain_form`.
* Known gap (unchanged): lit alpha-blended smoke is thinner in the `effects` example than in the real client; in the game
  `Effect::tint` and the world exposure (`aa-world` `update_gain`) are the compensation and were not changed here.

## Round 12: Liquid bubbles, the black ring of Crippling Mire

* **Bubble Trap's aura** (`illusion.buble_dot`, buff 96) drew a 4.5 m opaque white-grey sphere. Cause: its mesh emitter `buble_meshes_1`
  (`high_sphere2.cgf`, Size 2.1) has the material `water_buble_dot.mtl`: shader `Liquid`, Opacity 1, no textures. The engine draws a Liquid
  surface as clear refracting water; what shows of it is the `DualMaterial` (`water_buble_dot_second`: Illum, Opacity 0.297, a foam alpha map,
  teal diffuse). aa-fx has no liquid pass and took the first material as an opaque unlit mesh. `Geom::load` now follows a Liquid material to its
  `DualMaterial` and draws that (alpha blend, its textures and UV scroll): a translucent foamy bubble. Against the real client (my account,
  `Particle.SpawnEffect`, `shots/aa-fx/r12/bubble_real.png`): a translucent dome about 2 m wide with white foam patches, the ground seen
  through it; in the game with the buff on the caster (`T:buff@self`, `bubble_trap_game.png`) the same. Test:
  `a_liquid_bubble_draws_its_dual_materials_translucent_foam`.
  Other sphere and shell meshes of the libraries (scan of every `Geometry` emitter): the `Glass` and `Illum` ones already blend (opacity under 1 or
  Glass); the other Liquid meshes are `water_buble3_b1` (dual material `water_buble4`, handled by the same path) and `water_plane` /
  `water_shape` (flat water discs with a texture, unchanged).
* **Crippling Mire's black ring** (`death.psychokinesis_launch_form_end`): not an opaque or lost-alpha bug, the same code draws it in the example and
  the game. The ring is 5 sprites (`psychokinesis_launch_form2`, `mimic_ice_hole.tif`, `Color (0,0,0)`, Alpha 0.7, `Facing=Velocity`, 1.8 to 4 m)
  and a black ring mesh (`form3`, `pattern_ring_wind3`, 0.12 s, grows to 7 to 11 m); the texture is sparse (alpha mean 9 %) black cracks, so
  they are black by data. The example shows them only for the first 0.1 to 0.45 s and from a low camera over the ground (a half ring: the
  `psychokinesis_launch_form_end` rows of `crippling_mire_example.png`), the game holds the effect looped from THROW to END on the hand with a
  camera face-on to the ring (`crippling_mire_game.png`: a black smoky ring around the caster). The difference is the orientation of the
  emitter: `Facing=Velocity` cards lie perpendicular to the emission axis. The real client with `Particle.SpawnEffect(.., dir up)` draws a flat,
  faint dark ellipse on the ground (`form_end_real.png`, 1st frame), with a horizontal direction nothing visible. So the combat side hangs the
  effect on the hand bone with an axis that stands the ring up; which axis the real client uses for this fx item (`offset_axis_id`) is for the
  combat agent to settle against a real cast. Nothing in aa-fx to fix.

## Round 13: loop flag, loop budget, Render-attached emitters on the unit

* **Loop flag.** The real client restarts an effect from the root's `Continuous` byte (params byte +1), not from what the caller says. New
  `EffectDef::loops()` (= root `bContinuous`); `Effect.looping` is now ignored (kept so callers compile). `Effect.force_loop` is a dev override
  (the example's `--loop`). Test `the_loop_flag_is_the_roots_continuous`.
* **Loop budget.** `FxParticle::UpdateLoopParticles` keeps emitters only for the 50 closest looping fx-item effects. `Effect.item = true` marks
  an effect as an fx item; `classify` ranks the looping ones by camera distance and deactivates the rest (`FxSettings::loop_budget`, default 50).
  Test `only_the_closest_looping_effects_keep_their_emitters`. Combat must set `item = true` on the effects it spawns for fx items.
* **Arc Lightning's missing orb.** The blue swirl disc at chest height is `lightning_magic_throw` (a `flare_11` sprite of Size 1.2 plus lightning
  bolts whose emitters are `AttachType="Render"`, i.e. they emit from the unit's render mesh). aa-fx had no unit, so those bolts sat on a capsule
  at the origin (above the hand). New `Effect.body: Option<Entity>` (the caster; attached emitters then use a human capsule standing on that
  entity's transform) and `Effect.no_body` (no unit: the origin, like `Particle.SpawnEffect`); example flag `--no-body`. With `--no-body` the
  example reproduces the real structure (orb with bolts around it), about 2.5 to 3 times dimmer than the real client, which blooms it.
  **Combat side:** set `Effect.body = Some(caster)` for fx items and `item = true`.
* **Absorb Lifeforce.** Unstretched, the beam (`beam_test5`) has the width of the real strands; peak pixel colours measured against the real
  client are the same (real 166/198/164, ours 189/207/192, both pale green-teal), so no colour change was made. Under the (1, L, 1) stretch the
  beam is a thin pale strand in the example. In-game verification was inconclusive (the beam was not in the reel frames either before or after
  this round).

### Round 13 follow-up: the beams that flickered (frames longer than the particle life)

The combat agent saw the Absorb Lifeforce beam flicker after round 13 (visible at 2.0 s and 6.2 s, not at 3.5 s and 5.0 s) while the real client holds it for
the whole 6.5 s. It was none of the four suspects: the root of `death.life_absorb_launch` is `Continuous` (so are `psychokinesis_launch`, `spell_move_launch`'s
mesh emitter, `vital_absorb_launch`, `full_launch`; only `lightning_magic_launch` is not, and that one is meant to play out in 0.2 to 0.5 s), `loops()`
is true and the effect never even finishes. The cause is in the simulation: the beam is one mesh particle with `ParticleLifeTime 0.05` (Telekinesis' 0.1), and
`emit` placed each birth `age0` into the step but `update` then aged it by the whole step again, so with a frame longer than the life (the reel runs 10 frames a
second, a slow client 20 to 30) every particle was dead before it was ever drawn; only a frame with a short step happened to show one (a temporary
log: the beam emitter had 0 alive in every sampled frame, 1 after the fix). Fix in `sim.rs`: (1) a particle born in a step is not aged again in that step
(`EmitterState.fresh`); (2) the births of a continuous emitter get their real age (`dt - time of the j-th birth`, from the spawn accumulator) instead of a random
one, and a birth already older than the life at the end of the step is skipped, so one particle of life 0.05 s is alive in every frame at 10, 30 and 60 frames
a second, exactly one on average (test `a_particle_shorter_than_the_frame_is_still_drawn_every_frame`). Checked in game (`AA_COMBAT_FAKE`, 10 fps reel):
Absorb Lifeforce (12 consecutive frames, all with the beam), Telekinesis (dark beam in every frame from 4.5 s to 12 s), Crippling Mire (the beam in every
sampled frame of the 4.5 s); buff auras (Bubble Trap, `life_absorb_form`, `spell_move_form`, `lightning_magic_form` in the example) unchanged.

## Round 14: the after3 reel (effects read against my real client)

Real captures: `Particle.SpawnEffect` on my account at 9 m, frames at 0.25 / 0.5 / 0.75 / 1.0 s, next to the `effects --real 9 --no-body` render
(`shots/aa-fx/r14/cmp_*.png`: real left, ours right, made before the fixes below; `real_*.png` real only).

* **Summon Wraith** (`death.ghost_summon_launch`): not the round 10 orbit change (`RotateLength 50` without a speed is still no orbit) and not the age
  fix (the same cloud with the old simulation). The cloud is the data's: 400 red smoke puffs (`ghost_summon_launch3` x `launch4`) spreading 8 m wide and
  2.5 m high in 1 s, in the real client too (`real: bright saturated red`, `ws_game.png` ours in game: the same size). What differs is the colour: real
  bright red (about 230 / 30 / 30), ours dark maroon that fades to black grey, because the `_additive` smoke texture (RGB mean 45) and the lit factor make it
  about 2 to 3 times too dark. The `FxSettings::light` default went from 0.8 to 1.6 (see Boneyard); the red is still dimmer than real. Left open:
  147 emitters have no `BlendType` and a texture called `..._additive`; making them additive is wrong (tried: far too faint).
* **Multiplicative blend** (350 emitters): a sprite whose texture has no alpha (DXT1 glow / flare maps on black) multiplied the scene by its black corners,
  which drew dark 3.5 m square slabs (Frenzy's `reckless_cast3`, `flare_09`). The real client shows none. `fx.wesl` now takes the strength of such a sprite
  from the texture (`alpha * max(r, g, b)`): black leaves the scene alone, a coloured flare darkens it a little.
* **Illum sprite materials**: the engine draws `texture * (diffuse * lighting + emissive)`. `decal_death_pattern_black` (Hell Spear) has a diffuse of 0.004,
  `decal_pattern1` / `decal_ground3` 0: dark decals, not the white and red texture aa-fx drew, which hid Hell Spear's red glow (`hell_spears_range_glow`, a
  5 m `glow_big`, the sphere of the real capture) under a lattice. `FxAssets::illum_tint` (diffuse + emissive, at most 1) is now a colour factor of the emitter
  (`EmitterState::mat_tint`). In game (`hs_game.png`) the Hell Spear circle is a dark pattern with the red glow.
* **`FxSettings::light` 0.8 -> 1.6**: Boneyard's dust (`bone_wall_a_throw`, colour 0.74 0.66 0.6, `DiffuseLighting 0.72`) is a white cloud in the real client
  (232 / 225 / 215), brown in ours. This lightens every lit alpha smoke by 2x; checked on a sheet of six smoke and dust effects, nothing blows out.
* **Protective Wings** (`spell_shield`, skill 10714): the skill's own fx group is only the hand glow (`spell_shield_form`, `_form_end`), which the real
  client barely shows too (`cmp_implosion_wings_boneyard.png`, row 2). The blue sparkle aura and the ring of light lines are the channel's doodad 879
  ("magic protection", `channeling_doodad_id`): its `DoodadFuncClout` 28 applies buff 258 (fx `p_skill_will.spell_shield_dot`: the aura on the buffed
  allies) and projectile 51 (`spell_shield_proj`, physic 3: the ground ring). aa-fx renders both (`pw_ours.png`: ring of light lines 12 m wide, star
  sparkles, blue aura). **Combat / aa-client** must spawn the doodad's clout (the buff on the allies in 5 m, the projectile at the caster's feet).
* The rest of item 2 (what the data spawns, real against ours): see `docs/research/skill-audit-fx.md` round 14.

## Round 15: the portal gates (`shots/aa-fx/r15/`)

Player-opened portals spawn `portal_gate_dot3_indivisual` (the entrance, green materials) and `portal_gate_exit_dot` (the exit, the same emitters with the
`_red` materials), plus their `_end` versions.

* **The white / grey blob** is the centre of the crack: `portal_gate_dot3_indivisual_test_stone_center` draws `world_crack_center.cgf`, whose material
  (`world_crack_center.mtl`, red variant `_red`) is a `Liquid` shader with no texture and no dual material, only a `LiquidColor` (0.45 0.62 0.54) and a fog
  density: in the engine a clear, tinted water that refracts what is behind it. aa-fx read no texture, so the surface became an opaque white quad.
  `Geom::load` now draws a Liquid surface without dual material or texture as a multiplicative tint of `LiquidColor` (unlit, no emissive), a dark,
  translucent teal (red for the exit) centre through which the world shows. Test `the_liquid_centre_of_the_portal_gate_is_tinted_water_not_a_white_slab`.
* **`objects/effects/portal_gate.cgf` could not be loaded**: the file does not exist in the pak (`tools/aa-pak.exe ls objects/effects/portal_gate`: only
  `portal_gate2.cgf`). It is referenced by the casting effect `portal_gate_end` (`portal_gate_end4`, `portal_caster1`), which the real client skips as
  well. Harmless; no fallback is needed.
* **Colours**: the entrance is green (`portal_gate_green2/3`, `world_crack`, `shield_a ... _blue`), the exit red/orange (`portal_gate_red3/4`,
  `world_crack_red`, `shield_a ... type8`). The orange ring in `r4__33.png` and in `gate_game2.png` is the exit gate, correct per the data; the entrance
  is green in the `effects` example and in the real client (`gate_real.png`).
* Shots: `gate_before_1.png` (white blob), `gate_after_1.png` (tinted centre), `gate_real.png` (my real client, `Particle.SpawnEffect`), `gate_game2.png`
  (aa-client, `--portal-lua`, the camera is inside the exit gate).
