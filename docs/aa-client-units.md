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

* `SCBuffUpdated` is not decoded: the lane server never sends it and its layout is unknown.
* Combat idle is for creatures only; humanoid NPCs fight with the combat agent's clips.
* Mount and rider placement is unverified against the real client; there are no mount skills, no mount dismount animation.
* `SCUnitMovements` entries are all read as unit movements (the real client takes the type from the object).
* Units beyond 140 m are not drawn.
