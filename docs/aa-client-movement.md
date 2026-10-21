# aa-client: the local player's movement and camera

Files: `crates/aa-client/src/ingame/player.rs` (controller, packets), `swim.rs` (swimming), `camera.rs` (orbit camera, wall clipping).

## Keys (the client's `default_binding.g`)

| key | action |
|---|---|
| W / S | forward / back |
| Q / E | strafe left / right |
| A / D | turn left / right at 90 degrees a second (the camera turns with the character); with either mouse button held, strafe instead |
| Space | jump; swim up in the water (a tap near the surface jumps out) |
| X | swim down |
| `.` | toggle walking (Shift held also walks, a convenience) |
| NumLock | auto-run; once W, S and both mouse buttons were let go, the next press of any of them ends it |
| both mouse buttons | run forward (S cancels it, Q / E and A / D strafe) |
| right mouse button | the character faces where the camera looks; left button orbits only, also on the run (free look: the heading stays, also after the release) |
| W twice within 300 ms | Dash (skill 16287) when option 55 is on; letting go of W, S, Q and E ends it (`sprint.rs`) |

1.2's rules, read from its input code (`resolve_keys`, `AutoRun` in `player.rs`): either world mouse button turns A / D into strafes, and
with both A and D held the one pressed last wins; both buttons add forward without dropping S, Q or E; keyboard turning is pi x
`keyboard_rotate_speed` (0.5) radians a second by the frame time capped at 33 ms; auto-run's cancel is armed only after W, S and both
buttons are all up, so turning it on while W or S is held does not end it.

`--walk START:END:KEYS` scripts the same keys (`W S A D Q E`, `J` Space, `X` swim down, `H` Shift). Note that `A` / `D` now turn and `Q` / `E`
strafe, as in the real client (they were the other way round).

## Speeds and jumping

`actor_models` of the player's model (`ActorParams`, read at spawn): run 5.4 m/s, walk x `game_walk_multiplier` (0.45), backward x 0.65 (walk 0.2925),
jump `sqrt(2 g h)` with `game_jump_height` 1.3 and `physics_living_gravity` 25 (8.06 m/s). Wading slows down from 0.3 m of water (none) to 1 m (x 0.6):
`cl_shallowWaterDepthLo / Hi / SpeedMulPlayer`, registered by `x2game.dll` with those defaults.

The factor scales the whole pressed direction (`direction_factor`), as 1.2 does: a backward diagonal runs at 3.51 m/s along the diagonal, not
with only its backward part slowed. 1.2 interpolates one factor over eight directions; the keys give only those eight and every player row has
its diagonal columns equal to the straight ones, so the straight, strafe and backward factors are enough. A body in the air moves by the frame's
mean vertical speed (`air_step`): a standing jump peaks at v^2 / 2g = 1.30 m at any frame rate (a speed-then-position step peaked at 1.23 m at
60 fps).

The own buffs' `MoveSpeedMul` rows scale the run and walk speed (`sprint::speed_multiplier`: Dash at level 1 is x 1.384, 7.47 m/s); the run clip
plays faster with it. A buff with `sprint_motion` (Dash) runs with the second tier of the run group instead: 1.2's `fist_mo_normal_run.lmg`
lists the run and then `fist_mo_sprint_run_f`, `_fl`, `_fr`, `_l`, `_r` (and the run's own back clips), read from the model's own group
(`anim::sprint_tier`, `SprintClips`; a combat stance's `<prefix>_mo_combat_run.lmg` the same way), paced by the sprint clip's own speed.
Dash's group (`fx_group` 1034) is played by the buff visuals (`combat::buffs`): its held effect and its `FxSound` (`skill_rush_speed`, kept and despawned with the buff) live as long as the buff, the end burst plays when it goes; the refraction spheres of its two effects are not drawn (`aa_fx::Effect::no_refraction`), the repeating shell `speed_full_pc_meshe_*` emits its first cycle only (`aa_fx::Effect::once`, `combat::DASH_ONCE`), the double tap and the release need option 55, which is on until a player turns it off (`sprint::celerity_on`, `settings::native_default`), and the streaks of `speed_full_pc2` are free sprites turned as the engine turns them (docs/aa-fx.md, "Free sprite velocity alignment").

**Jump clips** (`jump.rs`, played by `anim::drive_locomotion`): 1.2's player graph picks a take-off (`fist_mo_relaxed_jump_[dir]_start`, 0.667 s,
held to 0.98 of its length, about the 0.645 s a flat jump is in the air), an air clip (`_[dir]_loop`, played once and held) and a landing by the
keys at touchdown: none held gives the standing `_s_end` (1.0 s, held to 0.8 unless a key is pressed), a direction `_[dir]_land` (0.6 s).
Walking lands with `_walk_[dir]_land`, Dash jumps with `fist_mo_jump_sprint_*`, a drawn weapon with `[item]_mo_combat_jump_*`. Diagonals take
the forward or backward clip (relaxed) or the side clip (combat); 1.2's 45 degree pelvis turn is not drawn. Only the local player knows a
take-off from a fall (`anim::LocalJump`); others fall into the air clip. The relaxed, walking and Dash clips bind when the clip set loads.

**Dash** (`sprint.rs`): double tap of W, end on letting go of every direction key (`Command::RemoveBuff`, 1.2's toggle-off), speed from the
buff, the sprint run (above) and the wider view (`cam_fov.rs`): the buff's `FxCamFov` item (`fx_cam_fovs` 1: +23 degrees, in 1 s, out 1 s)
adds to the camera's 42.75 degrees along 1.2's two curves, the `SizeX` and `SizeY` age curves of `camera_fov_curve` in
`abillity_skill_table_m` (in: 0.704 at 0.19 s, full at 1 s; out: opens at 0.75 and is 0.112 at 0.29 s). Only the own unit's buffs widen
it. The start burst, the streaks and the end burst are the buff's particle items, played by `combat::buffs`. A light radial blur of the
screen's edges in `aa_world`'s post pass (`PostUniform::motion`) can fade in and out with it; it is off, since 1.2's Dash shows no blur
(`AA_MOTION_BLUR=1` or `"s:AoAMotionBlur": 1` under `options` in the settings file turns it on). The other ends of Dash are the server's.

## Collision (`aa_world::WorldCollision`)

Every step: `slide` the capsule (radius 0.35, height 1.8, step 0.5) along walls, then stand on the highest surface under it (`ground_below`: terrain or brush,
bridges and stairs) within the step height; further down is a fall. Terrain that rises faster than tan 50 degrees (`physics_living_max_climb_angle`) over the
next half metre is not walked up (the mesh facets are rougher than that, so a facet normal is no measure). The first step waits (at most 5 s) until the
collision cells within 24 m are built; after the wait or a teleport a landing is not reported for 1.5 s (the server applies fall damage from it). The
saved height of the server is kept at spawn (a deck, not the river bed under it).

Dead players start in their corpse pose and cannot move, jump or auto-run. A local resurrection applies the server's
position and facing and restarts the collision wait, just as a teleport does. Loading honours the same `--pos` override
as the player instead of waiting for cells around a different saved spawn.

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

The zoom is 1.2's `camera::update_distance` (`zoom_notch`, `approach_distance`, `start_distance` in `camera.rs`):

* **Start**: at world entry, by the race and gender the server sent (Nuian 4.0 m for a man, 3.2 for a woman; Elf 3.8 / 3.4; Hariharan 3.6 / 3.2;
  Ferre 4.0 / 3.4; any other 10), set at once. The log says `camera start distance 4.00 m (race 1, gender 1)`. `--cam-dist` overrides it.
* **Notch**: the wanted distance moves by 0.5 x clamp(0.3 x wanted, 1, 20), so 0.5 m under 3.33 m and 0.15 x the distance above; it is held
  between 0 and 10 m (the far limit a running 1.2 shows). Under 1 m it snaps to 0, first person, or to 1 m when zooming out. From a Nuian man's
  start the 6th notch in is first person; 14 notches take first person out to 10 m. A touchpad's fractions add up to whole notches.
* **Settle**: the drawn distance follows the wanted one at 5 + gap^1.8 m/s, never past it, by the frame time capped at 33 ms (1.2's camera
  update caps it so): a 0.5 m notch settles in about 0.1 s.
* **First person**: while the wanted distance is under 1 m the own character (and what it carries) is hidden; the camera stands on the pivot,
  where there is no arm to sweep.

The arm from the pivot (1.754 m over the feet) to the wanted camera position is swept with a sphere of 0.3 m against the terrain and the structures
(`clip_camera`, `Mask::CAMERA`: barrels and fences do not stop it). The camera stops short of the hit at once and comes back out smoothly
(`ARM_RETURN`, an exponential approach at 5 / s) when the hit clears; the wall acts on the drawn distance, after the zoom's settle. `AA_CAMERA_NO_CLIP=1` turns the clip off for
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
