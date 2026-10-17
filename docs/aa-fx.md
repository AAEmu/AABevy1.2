# aa-fx: CryEngine particle effects

`crates/aa-fx` plays the particle libraries of the original client (`game/libs/particles/*.xml`, CryEngine 3
`ParticleParams`) in Bevy: parsing, a CPU simulation in compact per-emitter buffers, one batched mesh per emitter.

```rust
app.add_plugins(aa_fx::FxPlugin);
commands.spawn((aa_fx::Effect::new("X2_EFFECTS.fire_smoke.torchlight_b"), Transform::from_xyz(0.0, 0.0, -3.0)));
```

## API

* `Effect { name, scale, speed_scale, count_scale, alpha_scale, brightness, prime, looping, despawn_when_finished }`
  (`Effect::new(name).with_scale(s)`). `name` is `library.group.name`, case does not matter; the library is everything
  before the first dot (`X2_EFFECTS`, `abillity_skill_table_m`, ...). The entity's transform places, rotates and scales it
  (the average of the global scale axes multiplies `Effect::scale`). Put it under another entity (`ChildOf`) to attach it.
  `RenderLayers` of the effect entity are copied to its meshes.
* `FxSettings` (resource): `max_particles` (20000, the hard global budget), `max_distance` (160 m activation radius cap),
  `gain` (colour multiplier; a world with pre-exposed HDR colours sets `radiance * exposure`, aa-world does), `light`
  (0.8, how much of `DiffuseLighting` shows), `emissive`, `size_scale`, `alpha_scale`, `quality` (0..3, `ConfigMin/Max`),
  `soft_distance` (0.6 m, needs `DepthPrepass` on the camera, otherwise particles are not softened), `activations_per_frame`,
  `wind`, `enabled`.
* `FxStats` (resource): effects, active, emitters, particles, quads, `sim_ms`, `upload_ms`, `manage_ms`.
* `library::Library` / `EffectDef` / `params::EmitterDef`: the parsed data, usable without Bevy
  (`Library::load("x2_effects")`, `lib.get("fire_smoke.torchlight_b")`).
* Env: `AA_FX_LOG=1` logs the counters every 2 s (`=2` also lists every active emitter with its bounds); `AA_NO_FX=1` leaves
  the world's placed effects out (aa-world).

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
SizeX/Y/Z = 1; everything else 0 (`ParticleLifeTime=0` is endless). A continuous emitter spawns `Count / ParticleLifeTime`
per second; a non continuous one bursts `Count`; `PulsePeriod` restarts the emitter; `EmitterLifeTime` ends it;
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
collision), geometry particles (`Geometry`), tails/trails, `Rotate*` (orbiting), lights (`LightSource*`), sounds, refraction
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
* aa-client create step 3 (`login/create/scenery.rs`): the ground mist (`flow_fog_01`, `fog_01`) and leaves of the field, the
  mist with an opacity/brightness fit (`alpha_scale` 3, `brightness` 2).
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

Known gaps: `elf_fog_01` (placed in the elf training field) does not exist in any library of the pak, so that mist is missing;
the brightness of lit particles is not tied to the time of day (`FxSettings::light` is a constant).
