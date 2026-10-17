# aa-fx: CryEngine particle effects

`crates/aa-fx` plays the particle libraries of the original client (`game/libs/particles/*.xml`, CryEngine 3
`ParticleParams`) in Bevy: parsing, a CPU simulation in compact per-emitter buffers, one batched mesh per emitter.

```rust
app.add_plugins(aa_fx::FxPlugin);
commands.spawn((aa_fx::Effect::new("X2_EFFECTS.fire_smoke.torchlight_b"), Transform::from_xyz(0.0, 0.0, -3.0)));
```

## API

* `Effect { name, scale, speed_scale, count_scale, alpha_scale, brightness, tint, prime, looping, despawn_when_finished }`
  (`Effect::new(name).with_scale(s)`). `name` is `library.group.name`, case does not matter; the library is everything
  before the first dot (`X2_EFFECTS`, `abillity_skill_table_m`, ...). The entity's transform places, rotates and scales it
  (the average of the global scale axes multiplies `Effect::scale`). Put it under another entity (`ChildOf`) to attach it.
  `RenderLayers` of the effect entity are copied to its meshes. `alpha_scale` multiplies the opacity of this effect, `brightness`
  and `tint` (a colour) the light on its particles (on top of `FxSettings::light`).
* `FxSettings` (resource): `max_particles` (20000, the hard global budget), `max_distance` (160 m activation radius cap),
  `gain` (colour multiplier; a world with pre-exposed HDR colours sets `radiance * exposure`, aa-world does), `light`
  (0.8, how much of `DiffuseLighting` shows), `emissive`, `size_scale`, `alpha_scale`, `quality` (0..3, `ConfigMin/Max`),
  `count_over_life`, `soft_distance` (0.6 m, needs `DepthPrepass` on the camera, otherwise particles are not softened),
  `activations_per_frame`, `wind`, `enabled`.
* `FxStats` (resource): effects, active, emitters, particles, quads, `sim_ms`, `upload_ms`, `manage_ms`.
* `library::Library` / `EffectDef` / `params::EmitterDef`: the parsed data, usable without Bevy
  (`Library::load("x2_effects")`, `lib.get("fire_smoke.torchlight_b")`).
* Env: `AA_FX_LOG=1` logs the counters every 2 s (`=2` also lists every active emitter with its bounds); `AA_NO_FX=1` leaves
  the world's placed effects out (aa-world); `AA_FX_COUNT_OLD=1` goes back to `Count` per particle life (see below);
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
  quad bounds, `NotShadowCaster`, depth tested, no depth write, transparent phase.
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
A non continuous emitter bursts `Count`; `PulsePeriod` restarts it; `EmitterLifeTime` ends it;
`Enabled=false` emitters still hand their children to `SpawnIndirection` triggers (`ParentStart`, `ParentDeath`).
The focus axis is the emitter's cry +Y (up with `FocusGravityDir`); `Acceleration` is in world space; Size is the full width;
with `OrientToVelocity` or `Stretch` the sprite's long axis follows the velocity (`Stretch` = seconds of travel), otherwise
the quad faces the camera and spins with `RotationRate`.

Supported: `Texture`/`Material` (the diffuse map of a `.mtl`), `TextureTiling` (tiles, first tile, variants, frame animation),
`BlendType`, `Facing` (camera, free, bottom, flat), `OrientToVelocity`, `Size`/`SizeX`/`SizeY` with age curves, `Stretch`,
`StretchOffsetRatio`, `Alpha`, `Color` (with curves), `DiffuseLighting`, `DiffuseBacklighting`, `EmissiveLighting`, `Count`,
`Continuous`, `EmitterLifeTime`, `SpawnDelay`, `PulsePeriod`, `ParticleLifeTime`, `PositionOffset`, `RandomOffset`,
`PosRandomOffset`, `Speed`, `AirResistance`, `GravityScale`, `Acceleration`, `WindInfluence`, `EmitAngle`, `FocusAngle`,
`FocusAzimuth`, `FocusGravityDir`, `InitAngles`, `RandomAngles`, `RotationRate`, `RandomRotationRate`, `Turbulence3DSpeed`,
`TurbulenceSize`, `TurbulenceSpeed`, `SpawnIndirection`, `SoftParticle`, `DrawLast`, `SortOldestFirst`, `CameraMaxDistance`,
`ConfigMin`/`ConfigMax`, `MoveRelEmitter`/`BindToEmitter`.

Not supported (ignored): collisions and everything that depends on them (`Bounciness`, `MaxCollisionEvents`, spawn on
collision), geometry particles (`Geometry`; `EffectDef::geometry()` returns the model and material an effect names so that an
application can draw it: the combat code draws the arrow body of `st1000b` that way), tails/trails, `Rotate*` (orbiting), lights (`LightSource*`), sounds, refraction
materials, `BindEmitterToCamera`/`SpaceLoop`, `AttachType`/`AttachForm` emission shapes, `ViewDistRatio`, `CameraMinDistance`
(the data has values like 270 that would hide the quest beams), `VisibleUnderwater`/`VisibleIndoors`, per frame texture
blending, terrain-facing nuances, `FillRateCost`.

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
cargo run -p aa-fx --release --example effects -- --grid 100 --exit-after 14     # 100 torches, logs the cost
```

Flags: `--effect` (repeatable), `--grid n`, `--spacing`, `--dist --height --yaw --orbit`, `--gain --light --size-scale
--alpha-scale --no-soft --max-particles`, `--scale`, `--fx-yaw`, `--day`.

Release, 100 torches (600 emitters, ~2350 quads, 1080p): simulation 0.15-0.27 ms, mesh upload 0.10-0.15 ms, management
0.01-0.13 ms on the main thread (about 0.3-0.5 ms together), ~100 fps on the dev box with everything else idle.

## Step 3 mist (fit on the captures)

The library's plain values draw almost nothing in the character create fields (`flow_fog_01`: grey sprites, Alpha 0.4 x a curve
that peaks at 0.6, a texture whose alpha averages 0.15), while the captures show a pale, warm veil. What was tried, against
`reference/character_create/step3_*` (`data/extract/aa-fx/fit.py`, `evalfit.py`; ours rendered 6 times per race, 2 genders x 3
shots, against a run without the mist, `AA_CREATE_NO_FX=1`):

* Blend mode: `BlendType` is absent, i.e. alpha blended, which is what is drawn (not the reason).
* Particle colour in gamma space: would make it darker (not the reason). Entity `Scale`, `CountScale`, `bPrime`: 0.334..0.646,
  1, primed. Soft particles: off in the stage (no depth prepass), they could only thin the mist. Unscaled effects (scale 1):
  1.5-3x more mist but also wrong sizes and speeds. `Size` as a radius (sprites twice as wide): the fog_01 haze of the Nuian
  field becomes a blanket, no.
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
| Nuian | 1.0 | 1 | 3.0, 3.15, 1.9 | 0.97, 1.00, 1.00 | n/a (see below) |
| Elf | 3.0 | 1 | 6.0, 6.0, 3.4 | 0.70, 0.72, 0.91 (tile pick hits grass) | n/a, looks alike |
| Hariharan | 3.0 | 3 | 4.0, 3.6, 2.35 | 1.05, 1.07, 1.07 | 0.77, 0.75, 0.82 |
| Ferre | 1.3 | 1 | 7.6, 6.6, 4.1 | 0.97, 0.97, 0.93 | 1.32, 1.07, 0.86 |

Peak colour: the mean of the 12 brightest, least saturated, least varying 24 x 14 px tiles of the mist band (the same pick on
both images). Band excess: the mist's added light in the band (ours: with minus without the mist; the capture: the band minus
the clear control scaled by our grass's band/control ratio), in 0..255 steps; 5 % is out of reach, between two runs the same
setting moves the excess by 10-20 % (random particles). Nuian and Elf have no usable excess: the capture's grass is much brighter
than ours, so its band minus control comes out negative, and in Nuian our `fog_01` haze also covers the control region. The near
grass under the band gets +5..+9 of mist in Nuian, Elf and Ferre, 0 in Hariharan.

Known gaps: `elf_fog_01` (placed in the Elf and Ferre fields) does not exist in any library of the pak, so that mist is missing (the
Elf capture shows more mist than the two flowing sources make, so it may have existed); the brightness of lit particles is not tied
to the time of day (`FxSettings::light` is a constant).
