# aa-client: the local player's movement and camera

Files: `crates/aa-client/src/ingame/player.rs` (controller, packets), `swim.rs` (swimming), `camera.rs` (orbit camera, wall clipping).

## Keys (the client's `default_binding.g`)

| key | action |
|---|---|
| W / S | forward / back (S stops auto-run) |
| Q / E | strafe left / right |
| A / D | turn left / right (the camera turns with the character) |
| Space | jump; swim up in the water (a tap near the surface jumps out) |
| X | swim down |
| `.` | toggle walking (Shift held also walks, a convenience) |
| NumLock | auto-run |
| both mouse buttons | run forward |
| right mouse button | the character faces where the camera looks; left button orbits only |

`--walk START:END:KEYS` scripts the same keys (`W S A D Q E`, `J` Space, `X` swim down, `H` Shift). Note that `A` / `D` now turn and `Q` / `E`
strafe, as in the real client (they were the other way round).

## Speeds and jumping

`actor_models` of the player's model (`ActorParams`, read at spawn): run 5.4 m/s, walk x `game_walk_multiplier` (0.45), backward x 0.65 (walk 0.2925),
jump `sqrt(2 g h)` with `game_jump_height` 1.3 and `physics_living_gravity` 25 (8.06 m/s). Wading slows down from 0.3 m of water (none) to 1 m (x 0.6):
`cl_shallowWaterDepthLo / Hi / SpeedMulPlayer`, registered by `x2game.dll` with those defaults.

## Collision (`aa_world::WorldCollision`)

Every step: `slide` the capsule (radius 0.35, height 1.8, step 0.5) along walls, then stand on the highest surface under it (`ground_below`: terrain or brush,
bridges and stairs) within the step height; further down is a fall. Terrain that rises faster than tan 50 degrees (`physics_living_max_climb_angle`) over the
next half metre is not walked up (the mesh facets are rougher than that, so a facet normal is no measure). The first step waits (at most 5 s) until the
collision cells within 24 m are built; after the wait or a teleport a landing is not reported for 1.5 s (the server applies fall damage from it). The
saved height of the server is kept at spawn (a deck, not the river bed under it).

`AA_MOVE_TRACE=1` logs the position, the ground, the state and the water twice a second.

## Swimming (`swim.rs`)

* Starts when the water over the ground is deeper than `height - swim_height` = 1.3 m (a swimmer's origin hangs that far under the surface: the swim
  clips are authored with the body 1.3 m over the origin, `fist_mo_swim_f` lies horizontally with the pelvis at 1.34 m, `fist_ba_idle_swim` treads water
  upright with the head at 1.83 m). Stops below 1.1 m (`LEAVE_MARGIN` hysteresis). The depth threshold is the part not traced in the binary
  (`CActor::UpdateSwimStats` keeps the same `relativeWaterLevel`): change `ActorParams::swim_depth` when a capture of the real client says otherwise.
* Speeds: 3.0 m/s forward and to the side (the `.lmg` MoveDir), x 0.8 backwards (`swim_back_speed_mul`), up x 0.7, down x 0.5 (`swim_up/down_speed_mul`),
  a diver floats up at 2.0 m/s (`swim_buoy_speed`); Space within 0.2 m of the surface jumps out (`swim_jump_permission_range`).
* A body that falls in is stopped by the water (35 m/s^2); gravity is x 0.3 in water and a splash is no landing (no fall packet).
* Clips: `fist_ba_idle_swim`, `fist_mo_swim_<dir>` (8 ways), `_deep` when diving (more than 0.5 m under the floating height), `fist_mo_swim_up / down`
  while only Space / X is pressed. `drive_swim` runs after `anim::drive_locomotion` (the player's `Motion` is empty in the water, so locomotion
  leaves the animator alone) and calls `Locomotion::invalidate` when the swim ends. `anim.rs` is not touched: a combat action on the whole body
  (`Acting`, not layered) takes the clip over.
* Packets: stance `SWIM` (2), no ground flag (`MoveState::Fall`), the velocity with its vertical part, `delta_movement` `[right, forward, up]`, one packet at
  once when swimming starts or stops, and one per 0.1 s while moving, diving or bobbing.

## Camera

The arm from the pivot (1.754 m over the feet) to the wanted camera position is swept with a sphere of 0.3 m against the terrain and the structures
(`clip_camera`, `Mask::CAMERA`: barrels and fences do not stop it). The camera stops short of the hit at once and comes back out smoothly
(`ARM_RETURN`, an exponential approach at 5 / s) when the hit clears; the zoom itself is not delayed. `AA_CAMERA_NO_CLIP=1` turns the clip off for
before / after shots. The log has one line when a wall takes hold of the arm.

## Checked

* Wall: `--pos 21140.0,10596.2,111.17 --yaw 116 --cam-dist 7 --cam-pitch 14` (`shots/aa-client/move_wall_p14_clip.png`, `_noclip.png`): the arm stops at
  3.12 m of 7; with pitch -14 at 3.42 m.
* Bridge: `--pos 21055.7,10544.3 --yaw 281 --walk 2:9:W` climbs the stone arch from the quay (104.0) to the crest (113.74) at 5.4 m/s
  (`move_bridge3.png`); a quay railing at x 21060 stops the walk into the harbour (slides along it).
* Swimming: `--pos 15630,15352 --yaw 240 --walk 2:14:W --walk 15:16.5:X --walk 18:18.2:J`: wades (depth 0.69, 4.3 m/s), swims at 3.0 m/s from 1.3 m of
  water (`move_swim_14.png`, `move_swim_27.png`), dives (`move_swim_dive.png`), floats back up at 2 m/s, jumps out and falls back in.
* Saved heading: a walk at 281 degrees, then `--enter-world` without `--yaw` spawns with `facing -1.37` (281 degrees) at the saved position: the
  client sends its facing in every movement packet and applies the saved one.
