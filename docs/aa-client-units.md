# aa-client: other units (NPCs, mobs, players, mounts)

How the units the server announces move and animate in game. Code: `crates/aa-client/src/ingame/units.rs` (spawn, per-frame
movement), `remote.rs` (the pure interpolation, unit-tested), `creature.rs` (creature rigs and clips), `idle.rs` (combat stance
and NPC poses), `mount.rs` (riders). The wire side (what each packet carries) is in `docs/aa-net.md`, "Other units' motion".

## What the server sends

* Every moving NPC or mob sends a `SCOneUnitMovement` every 100 ms. Its `velocity` is a fixed 4000 (3.9 m/s) whatever the pace,
  so the **speed is measured from positions**, not read from the packet. `alertness` 2 or the IN_COMBAT flag means the unit
  fights. `actor_flags` 4 = running on the ground, 5 = walking.
* Players send their real velocity and the pressed direction (`delta_movement`); a jump or fall has no solid-ground flag.
* A rider gets no movement of its own: the server moves it with the mount (`SCUnitAttached`).
* `SCUnitMovements` is flattened by `aa-net` into one `Event::UnitMoved` per entry, so everything below sees a single stream.

## Movement (`units::receive_moves`, `move_units`)

`receive_moves` turns each `UnitMoved` into a `remote::Report` (position, yaw, the sender's `server_ms`, airborne, combat) and
pushes it into the unit's `remote::Track`. `move_units` samples the track every frame:

Death freezes the displayed track at the corpse; resurrection clears it and uses the packet's position and facing
immediately, even when no movement follows. Ship steering requests carry no position and never enter interpolation.

* **Snapshot interpolation.** The track keeps the last 8 reports and plays them back `delay` seconds behind the newest
  one: `delay = clamp(1.25 * report gap, 0.08, 0.35)`, the gap measured with the sender's `server_ms`, so the arrival jitter of
  the network never shows. A playback clock runs at `clamp(1 + error, 0.5, 1.5)` times real time to stay near that delay, and
  jumps when it is off by more than 0.6 s.
* **Extrapolation** carries on for 0.3 s when a report is late, then holds still.
* A silence longer than 0.5 s inserts a hold snapshot (the unit stood; it does not glide to the next position). A move of more
  than 12 m clears the track (teleport, respawn).
* **Heading.** The unit turns toward its direction of travel at 8 rad/s (`TURN_RATE`) when it moves, and takes the server's yaw
  when it stands. Players strafe: `remote::local_dir(heading, yaw)` is the direction of travel in the unit's own frame, which
  picks the forward, backward and strafe clips of `drive_locomotion`; NPCs always run forward.
* **Ground.** The server's z is replaced by `TerrainQuery::height_at` when it is within 1 m of it and the unit is not airborne.
  A jumping or falling player keeps the server's z.
* `Motion { speed, local_dir, airborne }` is written for `anim::drive_locomotion`, which owns the walk, run, jump and fall clips.
  The transform is touched only when the pose changed (a standing unit costs nothing).

## Clips

Creatures (`creature.rs`) use the `.cal` of their model; the sets are the `fist_*` ones (`fist_ba_relaxed_idle`,
`fist_mo_normal_walk_f`, `fist_mo_normal_run_f`, strafes through `global_monster_strafe`). Humanoid NPCs and players use the
character `.cal` (`ba_relaxed_idle`, `mo_normal_*`, jump and fall clips for players).

* **Combat stance** (`idle::IdleClips::combat`): a creature that stands while the server reports it in combat plays
  `fist_ba_combat_idle`; moving, it keeps the walk or run clip of locomotion.
* **NPC poses** (`IdleClips::pose`): `Unit::anim_action` (from `SCUnitState` or `SCUnitModelPostureChanged`) names an
  `anim_actions` row; its `anim_name` is the idle the NPC stands in (`fist_pos_stn_armor_dealer_idle`,
  `fist_pos_sit_chair_rest_idle`, ...). `idle::posture_changes` rebinds the clip when the server changes it.
* No turn-in-place clips exist for humanoids and most creatures in the 1.2 data, so a unit turns without one.

## Mounts (`mount.rs`)

A unit with `Unit::attached_to` set (`SCUnitAttached`) becomes a `Rider`:

* the mount is another unit, drawn like any creature (model, gait clips by its own speed);
* the rider sits on the mount's `bone_spine_driver` (attach point 1) or `bone_spine_passenger`, faces the mount's facing, and
  plays the rider clips of the mount's kind: `models.mount_pose_id` 1 horse, 2 lion, 4 wildboar, 10 pangolin give
  `horse_ba_relaxed_idle`, `horse_mo_normal_walk_f`, `horse_mo_normal_run_f` (and the other prefixes), with the gait picked from
  the mount's speed (idle below 0.3 m/s, walk below 3 m/s, run above) and the tempo copied from the mount's animator;
* the rider's on-foot `Locomotion` is stashed and given back on `SCUnitDetached`.

The server has no command that summons a mount, so the riders are checked with `AA_FAKE_MOUNT=1`: a horse (model 139) and a
rider that looks like the local player appear next to the player and run a circle at 5 m/s.
The offset between the seat bone and the rider's origin is a first guess, not checked against the real client.

## What is drawn (the census)

Units are spawned within `UNIT_RADIUS` (140 m) of the player, in a time budget per frame (models are prepared on the task pool,
`prepare.rs`). The `in game:` log line ends with
`not drawn: N beyond 140 m, M without a model, K waiting`, and a unit whose model cannot be built logs
`warn: unit <id> (<name>) is not drawn: <reason>` once. At the Nuian spawn the "13 of 18" was all of the 5 units beyond 140 m;
no model failed. The set of units grows as the player walks, as the server announces units around it.

## The second figure in `shots/gallery/25_spawn_night.png`

It is a real other player (another agent session logged in on the same lane server standing at the spawn), announced with its
own object id; it is not a duplicate of the local player and not a ghost. The reproduction with a second account showed the
same picture. `aa-net` also drops an `SCUnitState` that carries the local player's own object id, so that one could never draw a
second figure.

## Env switches (dev)

* `AA_UNIT_SMOOTHNESS=1`: logs how the displayed speed of remote units compares with their path speed (mean and p5/p95 per
  group, npcs and players).
* `AA_FAKE_MOUNT=1`: the mount check above.
* `AA_UNIT_SHADOW_RADIUS`, `AA_ANIM_LOD`: shadow radius and animation levels of detail of units (performance pass).

## Gaps

* `SCBuffUpdated` is decoded (`crates/aa-net/src/game.rs`, layout in `docs/aa-net.md`), but the AAEmu server never sends it, so live stack/charge updates do not arrive; a buff refresh comes as `SCBuffCreated` with the same index.
* Combat idle is for creatures only; humanoid NPCs fight with the combat agent's clips.
* Mount and rider placement is unverified against the real client; there are no mount skills, no mount dismount animation.
* `SCUnitMovements` entries are all read as unit movements (the real client takes the type from the object).
* Units beyond 140 m are not drawn.

## Authored NPC facial playback (2026-10-18)

`creature::spawn` now prepares a supported CDF head before spawning its skin subsets and
puts the returned facial state and morph weights on the same root as the body Animator.
The head's own CAL must declare a facial library; this inexpensive shared-CAL check avoids
parsing every body/clothing part in a crowd as expression geometry. Unsupported or missing
head data keeps the existing ordinary skin path. Cached models remain unchanged, and each
prepared instance receives private mesh handles so its weights cannot affect another NPC.
The existing FacialPlugin follows the body's authored CAL-to-FSQ mapping.

`cargo build -p aa-client --example facial_creature` passed. The unattended example exercises
the actual client CDF loader with `npc_chef01_base.cdf` (bound clothing/face/hair), not the
blank customization slots in `npc_chef01.cdf`. Three independent instances use the original
`fist_ac_talk_11` CAF at 0, 1.7218 and 3 seconds. Runtime assertions verify 23 targets, the
body clock, private mesh handles, correct owner references, and one facial versus three
ordinary skin subsets per instance. Speech peaks at 0.6935022; the authored return retains
its small signed values, with positive peak 0.040279. The inspected capture is
`shots/aa-client/facial-creature/cdf-body-clock.png`, with 266–277 FPS after startup and no
GPU errors; the screenshot process exited normally.

This adds playback to supported bound NPC heads. The following milestone also handles rigid
bone attachments and explicit received body items. PC-shaped remote units and voice-driven
sequence selection remain separate work. No procedural mouth movement or voice lip sync is claimed.

## Received NPC body slots and rigid attachments (2026-10-18)

The creature preparation path applies explicit received Equipment slots19..25 to the original
actor CDF's matching declared skin slots. Each item must resolve to the same model and its
matching native body slot23..29. Empty or incompatible items preserve the authored definition.
The worker's model list and main-thread spawn share this effective CDF; neither substitutes
an `_base.cdf` or infers an outfit. This fixes blank actor CDF slots previously ignoring received
face/hair/body items. Initial spawn is covered; later appearance-change rebuilding is still open.

Declared visible CA_BONE bindings now load alongside skin parts, including material overrides
and CDF wrappers, and parent to the existing animated joints using aa-anim's source-proven
alignment helper. Bone-aligned attachments ignore XML absolute offsets; model-aligned ones
use inverse joint bind transforms. Skin-aligned projection remains unsupported. Missing joints
or unsupported bindings skip that attachment without breaking the body. The loader preserves
authored eye-path choices, including a duplicate left-eye path in the chef's actor CDF.

`facial_creature --packet` exercises the actual blank chef actor CDF with explicit original
face14478/hair14475/body14479 items, the same configuration helper and spawn path as production.
The inspected `shots/aa-client/facial-creature/packet-cdf-eyes.png` shows seated eyes, clothing,
hair and authored neutral/A/return poses. Each figure has one independent facial subset and
five ordinary subsets; body-clock and owner assertions pass. It measured223–256 FPS while the
HUD fixture also used the shared GPU, with no GPU errors and normal screenshot exit.
The client suite passes150 tests /24 ignored, including model/slot incompatibility and worker
model-list checks. This is offline visual/source verification; live lane game connections remain stalled.

## Selected remote character faces (2026-10-18)

The server-confirmed Target now opts one visible, awake remote Character into original facial
playback after normal unit culling. A target change, hidden/out-of-view target, parked Animator,
or removed entity detaches the previous private face before enabling the next. The queued mutation
rechecks visibility/awake identity. Unsupported heads are attempted only when eligibility changes,
not every frame. The local player's existing spawn-time face is independent. This is a bounded
client memory policy, not a claim of identical native crowd facial LOD.

The aa-anim in-place API preserves the actual customized face, material/skin components, bones,
movement/entity identity and cached ordinary mesh handles. It handles selection AFTER GPU upload
has released ordinary CPU vertices by rebuilding only the selected head with the same source model,
face_morph and aa-cry subset converter. Disabling restores ordinary handles and underlying body bone
poses, including a parked body; private meshes are released through normal asset lifetime.

The supporting aa-anim regression/capture enables at3s, disables at5s after private GPU upload and
captures at8s. It passed73 tests /1 ignored; facial_attach_toggle_final_20261018.png was inspected
by the owner at252-339 FPS, with no window left open. Root's main client check passes, and a
read-only integration review found no lifecycle blocker. Natural full-world target switching is
still awaiting the lane game-connection unblock. Same-entity appearance replacement will need an
explicit re-enable trigger once that pipeline is implemented.

Measured Nuian male morph payload is4,049,472 bytes per enabled head (about386 MiB for100), with
zero extra image/material/bindpose assets. The crowd audit therefore did not add a CPU cache that
would leave per-mesh GPU duplication intact. Its expression-update microbench excludes full ECS,
body and facial-bone costs; original32-head captures check independent poses and real GPU use.

## CDF particle and object attachments (2026-10-18)

The client now distinguishes object paths, named particle effects and entity bindings before
prewarming or spawning. Original CA_BONE objects and effects use the authored bone/local
transform. Creature CDFs and their skin-part CDFs retain these bindings; assembled Characters
use the attachments retained by aa-anim on each resolved costume/hair part. Named effects are
never mistaken for mesh filenames. The three source entity bindings remain unsupported.

Effect anchors follow the existing actor bones while retaining their actual source part as
owner. A hidden actor or independently hidden part releases the emitter; unhide lazily creates
it again. Removing a part releases its bone-parented attachment, and actor unload removes all
descendants. Rigid object attachments also follow source-part visibility and lifetime.

Native constructor31515900 uses scale1; lazy spawn31517A40 and update31517CC0 retain the
emitter pointer until hidden/destructed. The integration therefore uses scale1, no priming,
and no forced repeat. Native flag0x20000 passed through315170B0 remains unclassified and is
not represented as an invented priming/loop option. This does not claim full native particle
rendering/material parity.

The unattended cdf_attachments example uses the actual horse_lord.cdf four hoof effects and
the Elf female white-day costume head ring. Inspected native-bindings.png shows all five;
native-unload.png and its assertions confirm zero Effect/FxMesh entities after despawn.
The baseline measured218-298 FPS, and the hide/show/unload run279-412 FPS on the shared GPU,
with no render errors. Captures/logs are in shots/aa-client/cdf-attachments/. These are original
asset/lifetime checks in a controlled scene; same-scene real-client lighting comparison and
live full-world attachment validation remain open.

The fresh client suite passes156 tests /26 ignored (4.34s), including part/actor visibility,
emitter recreation, source-part destruction and named-effect exclusion from mesh prewarm.
