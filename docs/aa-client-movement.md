# aa-client: the local player's movement and camera

Files: `crates/aa-client/src/ingame/player.rs` (controller, packets), `swim.rs` (swimming), `camera.rs` (orbit camera, wall clipping), `camera_pivot.rs` (1.2's pivot, pitch and far limit, plain Rust), `camera_log.rs` (`AA_CAMERA_LOG`), `quest_camera.rs` and `quest_framing.rs` (the quest talk's close-up; the rules in plain Rust).

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

## Gliders (`glide.rs`, S14)

A glider is an `item_backpacks.backpack_type_id = 2` item whose `items.use_skill_id` applies a `buffs.gliding = 't'` buff
(the Experimental Glider of quest 1065 is item 14677, skill 12373, buff 1029: move 5/6/7, fall 1.0/1.5/2.5, turn 30 deg/s,
startup 0.7 s / 3.0, smooth 1.0, 2 lifts 1.5 m / 3.0 m/s / 0.5 s / after 5 s, land 1.0, slide 1.0). The flight starts and
ends with that buff. The 1.2 decompile's chain is `ClientBuffMan::ApplyAttrs` (`FUN_39389440`) -> `FUN_39357920` (build the
`0x50` block) -> `FUN_3911fc80(1, block)` (stance 4), and the buff's removal -> `FUN_39389d60` -> `FUN_3911fc80(0, 0)` (the
stop probe: stance 5 within `land_height + 0.2` of the ground, else stance 0). A backpack change (`FUN_393220a0`) is the
second start path. The block and the controller (`FUN_3912d860`, `FUN_3912b280`, the sliding timer in `FUN_3911feb0`) are
ported as `GlideParams` / `Flight`; the tests pin buff 1029 against the database and the platform's row 3583.

* Startup (`elapsed <= startup_time`): normal speed and **+**`startup_speed` as the target. `FUN_3912d860` flips the
  startup word twice (the block copy then the velocity write), so the opening climbs at 3 m/s for 0.7 s (about 2.1 m);
  `smoothed_v` starts at the same `+startup_speed` (`FUN_3911fc80` writes `block+0x14` to `actor+0x1b8`). Then the vertical
  input (`controller+0x10`) below -0.1 takes slow, above +0.1 fast, else normal. **S14-C2:** the keys are not named in the
  decompile; this client maps W to the + axis and S to the - axis.
* Speeds: `FUN_39357920` reads unit attributes 7/8/9 (`GlidingMoveSpeed*`, x 0.001), not the buff row. The client has no
  unit-attribute 7/8/9 table, so it uses the row's `gliding_move_speed_*` columns; **S14-C6** captures the live attributes
  (base + buff modifiers) to check that they agree.
* Lift: `lift_count > 0`, `elapsed > lift_valid_time`, the same input below -0.1, no lift running and a downward
  `lift_height` ray hit. The timer is `lift_count * lift_duration` after `lift_duration / lift_count` (the row's 0.5 s for
  the first of two), the vertical target `+lift_speed`, and the smoothing response `smooth_time * 0.2`.
* Vertical smoothing (`FUN_3912d860`): `x = 2 dt / smooth_time`, `1 / (1 + x + 0.48x^2 + 0.235x^3)`; it acts only when the
  target differs by 0.01 m/s. Sliding (`FUN_3912b280`) moves at `move_normal`, no gravity, no input, for `sliding_time`.
* Wire: the ordinary `CSMoveUnit` (type 1), no glider packet. The real flight rows (2026-10-31 recapture rows
  5137-5287, buff index 20) are **stance 1, actor flags 0**; move flags are **2 with `delta_movement` 0x7f while W is held**
  (row 5124), 0 otherwise. A/D/S flag bytes are capture-pending; this client keeps flags 2 for W only. `0x04` in the earlier
  (walking) rows is STANDING_ON_SOLID and would remove the buff on the first flight packet. At contact the server removes the
  remove-on-land buff: **`CSRemoveBuff` with the glider buff index and reason 9** (`FUN_393888b0`; log rows 168455 and
  177786), and the capture sends it **before** the first grounded packet. The ground end is then **stance 1, flags 6, actor
  flags 0x14** (later grounded packets return to flags 4 / actor 0x04). Water is **stance 2, flags 0, actor flags 0** and
  then the same remove (row 5288 / 13:39:15), not a fold stop. Velocity at 1/1024 m/s. Deploy is `CSStartSkill` 12373 cast
  as the equipped item 14677 (item id 16777259). **S14-C1 settled for flight; S14-C3 settled; S14-C2 key flags partly open.**
* Look: the stowed glider is the item's `asset2_id` on `item_back_c` (drawn by `weapons.rs`); the opened model is the
  item's `asset_id` (`glider_bx.chr` for 14677), attached to the character's **`item_glider`** bone with an identity
  attachment (`FUN_394e4e80`) and the graph's
  `fist_ac_gliding_{start,idle,front,back,left,right,fl,fr,bl,br,sliding,end}` clips and the glider `.cal`'s
  `start/loop/front/back/left/right/sliding/endloop`. **S14-C5 settled.**
* Deploy by the action bar's item use is the ordinary item skill. Option 57 (`OPTION_ITEM_USE_GLIDE_WITH_DOUBLE_JUMP`,
  default 0) makes a second jump in the air ask for the equipped item's skill. **S14-C4 settled:** `FUN_39404d50` line
  843762 calls `FUN_39399690` on the jump edge only when the actor jump-state byte (`+0x161`) is set and the
  `glider_start_with_double_jump` cvar (`+0x2ac`) is nonzero; there is no height or freefall gate.
* Mouse look: with the look button held (taken as the right mouse button), yaw/pitch deltas beyond `gliding_mouse_ad` /
  `gliding_mouse_ws` (both default 8.0; `FUN_390f7b40` lines 196255-196257) become the A/D and W/S input
  (`FUN_39404d50` lines 843209-843230). The exact look action is still capture-pending.
* Remote gliders: a unit under a gliding buff gets the deployed model, the stowed one is hidden, and its pose is chosen from
  the reported ground speed (`remote_pose`); turn poses need the keys, which are not on the wire.
* Landing and obstacles: the landing/lift floor also asks the deck floor `board_deck` uses (`DeckGround`), so a moving
  hull is not ignored. A touchdown on a deck inserts the same `DeckRide` a walker gets: the touched wait and the 1 s slide
  run in the hull frame (`DeckContact`), so the ship can sail 5 m under the glider without leaving the player behind.
  Every deck slide step goes through `slide_step`: the same world capsule collision, hull-floor `walking_step` and
  riser/edge checks as `deck_control`. A wall/riser stops the slide and leaves the ride there; an edge calls `leave_deck`
  and falls; a lower deck beyond the step-down tolerance starts a real `DeckRide` fall (`DeckRide::falling`), so
  `deck_control` applies gravity, the landing and the fall-damage report. A ship that leaves view during the wait or slide
  ends the glide into a fall at once and stops sending deck moves. `deck_control` pauses while the flight owns the player
  and resumes at the slide's final local point. The contact carries the player's facing in the hull frame: the wait and
  slide compose it with the current hull yaw, so a ship's turn carries the player without steering the slide. Starting a
  flight drops a `DeckRide`, and boarding is refused while flying. The slide branch also sets `Pose::Slide` (the graph's
  `fist_ac_gliding_sliding` / glider `sliding` clips). A buff lost within `land_height + 0.2` above a deck starts that
  slide on the hull (`contact_within`), not in world space. A rise stops at the first surface above the head with a
  `WorldCollision` ray: x2game has no upward ray (the capsule is cryphysics), so this is **S14-C7**, conservative and
  capture-pending. `BUFF_GRACE` 0.2 s and `TOUCH_TIMEOUT` 1.0 s are client tolerances (**S14-C8**); `END_CLIP` 0.5 s is
  the graph's `gliding_end forceInStateUntil`.
* Remote glider models are spawned through `Commands` and exist a frame later; their clip is started on that frame (the
  character's own clip is tracked separately), so the opening is not skipped. A pose whose clip the model lacks is
  remembered and not retried every frame.

### Capture-pending steps

1. **S14-C1**: settled for flight 2026-10-31 (stance 1 / actor flags 0 while flying; move flags 2 with delta 0x7f while
   W is held, 0 otherwise; deploy is item cast 12373 with item 16777259/14677). Still capture A/D/S flag bytes.
2. **S14-C2**: capture the exact look action; W/S speed tiers, A/D turn and the mouse cvar thresholds are settled.
3. **S14-C3**: settled: `CSRemoveBuff` index + reason 9, then ground stance 1 / flags 6 / actor 0x14, later packets flags 4 /
   actor 0x04; water stance 2 / flags 0 / actor 0 then the remove. The old "fold stop" (stance 2) was that water entry.
4. **S14-C4**: settled: jump edge + actor jump-state byte + `glider_start_with_double_jump`; no height/freefall gate.
5. **S14-C5**: settled: `item_glider` identity attachment for the opened model; `item_back_c` for the stowed one.
6. **S14-C6**: read the live unit's attributes 7/8/9 (base + the buff's modifiers) and compare them with buff row 1029's
   `gliding_move_speed_*`.
7. **S14-C7**: glide up under a bridge or a ship's deck: does the rise stop before the head passes through, and where?
8. **S14-C8**: with a capture of the buff removal, measure the delay to the first slide packet and the landing packet;
   confirm whether the 0.2 s / 1.0 s client fallbacks are ever needed.

## Camera

The zoom is 1.2's `camera::update_distance` (`zoom_notch`, `approach_distance`, `start_distance` in `camera.rs`):

* **Start**: at world entry, by the race and gender the server sent (Nuian 4.0 m for a man, 3.2 for a woman; Elf 3.8 / 3.4; Hariharan 3.6 / 3.2;
  Ferre 4.0 / 3.4; any other 10), set at once. The log says `camera start distance 4.00 m (race 1, gender 1)`. `--cam-dist` overrides it.
* **Notch**: the wanted distance moves by 0.5 x clamp(0.3 x wanted, 1, 20), so 0.5 m under 3.33 m and 0.15 x the distance above; it is held
  between 0 and the camera mode's far limit: `camera_max_dist` of the same `option_camera_fov_set.cfg` level as its `cl_fov`, 10 m in the action
  mode (level 1, 1.2's group default) and 18 m in the classic mode (level 2, this client's default). Under 1 m it snaps to 0, first person, or to 1 m when zooming out. From a Nuian man's
  start the 6th notch in is first person; 14 notches take first person out to 10 m in the action mode. A touchpad's fractions add up to whole notches.
* **Settle**: the drawn distance follows the wanted one at 5 + gap^1.8 m/s, never past it, by the frame time capped at 33 ms (1.2's camera
  update caps it so): a 0.5 m notch settles in about 0.1 s.
* **First person**: while the wanted distance is under 1 m the own character (and what it carries) is hidden; the camera stands on the pivot,
  where there is no arm to sweep.

The pivot is the point 1.2 follows (`camera_pivot.rs`, measured in a running 1.2 with `tools/camera_probe.py`, `tools/evidence/camera/`):

* **The bone**: the `camera` bone's height over the feet in the **bind pose**, from the player's own rig, before it is loaded from a table by
  race and gender (Nuian man 1.463, Hariharan woman 1.194). Not the animated bone: no idle clip drives it, and in a pose that keeps `Bip01` on
  the ground it rides 0.34 m over the feet. 1.2's point stood at the bind height in every sample.
* **The lift**: the character's `.cdf` `zOffsetList`, a monotone cubic over the distance now (Hariharan woman: 0.20 at 0 m, 0.1 at 1 m, 0.8 at
  15 m). 1.2 stands the camera at T - f d + (0, 0, z) and looks along f, which is an orbit around T + z: over a Hariharan woman's feet 1.393 m
  in first person, 1.294 at 1 m, 1.340 at her 3.2 m start, 1.790 at 10 m, 1.994 at 18 m. The log says `camera pivot 1.340 m over the feet at 3.20 m`.
* **Pitch**: -88 to +88 degrees; past 50 degrees up the arm stays and the view turns the rest (`camera_pose`).
* **A rider**: 1.754 m over the seat (1.2's rider point is read only in part).

**A driven vehicle or ship.** 1.2's `FUN_3915c8a0` gives the driver (attach point 1, never a passenger or a mate) the
model's camera distance: `models.camera_distance`, or `camera_distance_for_wide_angle` when option 65 (the camera mode)
is 1; only `VehicleModel` / `ShipModel` rows count. `FUN_3915aed0` sets the far limit (+0x144) and `FUN_3915aef0` the
wanted distance (+0x20) in camera mode 1; the drawn arm eases like the zoom. `ingame/camera.rs::vehicle_camera` reads
the model of the local driver's mount (`Db::camera_distance`), uses the wide value in the action mode (16 m for the
Farm Wagon's model 1008, 25 m for the Eznan Cutter's model 393) and the normal one otherwise (20 / 40 m), snaps the
wanted distance in camera mode 1 and holds it within the new limit in every mode. On leaving, the mode's own limit
returns and the wanted distance is clamped to it, which is what the captures show (16 m while driving the Farm Wagon
and 25 m at the Eznan Cutter helm, 10 m after leaving the wagon, with the same 10 m before boarding). The captures had
not measured a frame-by-frame ease, so the drawn distance follows the existing `approach_distance` curve.
`cargo run -p aa-client --example vehicle_camera -- --mute --offscreen-shot shots/aa-client/vehicle_camera.png`
drives the Farm Wagon through the received events (16 m in camera mode 1, 10 m after leaving) and writes the picture.

The arm from the pivot to the wanted camera position is swept with a sphere of 0.3 m against the terrain and the structures
(`clip_camera`, `Mask::CAMERA`: barrels and fences do not stop it). The camera stops short of the hit at once and comes back out smoothly
(`ARM_RETURN`, an exponential approach at 5 / s) when the hit clears; the wall acts on the drawn distance, after the zoom's settle. `AA_CAMERA_NO_CLIP=1` turns the clip off for
before / after shots. The log has one line when a wall takes hold of the arm.

`AA_CAMERA_LOG=1` writes one CSV line a frame of the camera (`camera_log.rs`: `camera-log-<seconds>.csv` in the working folder, any other value
but `0` is the path), in the columns `tools/camera_probe.py` writes for the running 1.2 client; `tools/camera_compare.py` sets the two side by
side per zoom step. With a script, `zoom N` and `pitch DEG` (docs/aa-client-script.md) drive the same sequence as a hand on the wheel.
While the quest talk's close-up holds the log writes mode 3, with the speaker's feet and body instead of the player's.

### Pending live checks (vehicle camera)

The Group-A2 capture (2026-10-30) gave only settled values (the driver read after 2 s): camera mode 1, wanted = max =
16 m on the Farm Wagon, arm 16.02, and 10 m after leaving; the normal 20 m value was not reachable (option 65 = 0 is a
no-op there). The doodads capture (2026-10-30) read the same fields at the Eznan Cutter's helm: camera mode 1,
wanted = max 25.0, arm 25.01 (its `ShipModel` row 393 is normal 40 / wide 25). A later capture must confirm: whether
the wanted distance snaps on boarding (this client snaps it) or eases; whether leaving restores the pre-board wanted
distance or only clamps it to the mode limit (both readings were 10 m); the normal-distance path with option 65 = 2; a
driver on another ship model or a siege weapon (their own `ShipModel` / `VehicleModel` rows); and whether the wheel may
zoom below/above the vehicle distance (this client keeps 0..the model value).

### The quest talk's close-up

While a quest talk is on and its quest allows it (`quest_contexts.use_quest_camera` and the bit `detail_id - 1` of the Options' close-up
camera value, 1023 or 0: `quest_framing::close_up_on`), each line frames its speaker from the speaker's `quest_cameras` row (the bubble's
`camera_id`, else the row of its model in `model_quest_cameras`; row 17 "previous camera" keeps the framing in hand). The eye stands
`camera_offset` in front of the speaker and 1.5 m up; the aim is `npc_offset` from the speaker's feet plus its `camera` bone's **bind-pose**
height (`quest_framing::aim_height`, the same bone and the same bind height the follow camera's pivot uses; 1.5 m for a model without a
skeleton). No clip drives that bone, so in an animated pose it rides on `Bip01` near the ground: aimed there, the close-up looks at the
speaker's feet. The lens is the row's 30 degrees vertical with no aspect factor; each new speaker is a cut, and so is the way back to the
follow camera, whose orbit is kept as it was left. The player and its mount are hidden meanwhile (the row's `invisible`).

`hud/quest.lua` asks per line (`HUD.quest_camera`, `HUD.quest_frame`), `hud/dialog.rs` finds the speaker and writes `QuestTalkCamera`,
`quest_camera::drive` turns it into `QuestShot`, which `follow_player` takes over the follow camera (the mouse and the wheel do nothing
meanwhile), `cam_fov::drive_cam_fov` takes the lens from and `units::cull_units` the mount's hiding from. Not done: the eased move of a row
with `interpolate` (no row has it).

The blur behind the speaker is `quest_depth_of_field.rs`: while a row with `dof` frames (all but the eight ghost rows) and the camera's
`Post.dof` is on, the focus is the row's (distance `camera_offset_y`, range 2.5 times it, blur 1), eased as 1.2's renderer eases its copy of
the user focus (`min(3 dt, 1)` of the way every frame, talk or none, from 3.5 m, 5 m, 1), so the blur is full from the talk's first frame;
it is off at once when the close-up ends. aa-world draws it (`aa_world::depth_of_field`, docs/aa-world.md "Post").

## Checked

* Wall: `--pos 21140.0,10596.2,111.17 --yaw 116 --cam-dist 7 --cam-pitch 14` (`shots/aa-client/move_wall_p14_clip.png`, `_noclip.png`): the arm stops at
  3.12 m of 7; with pitch -14 at 3.42 m.
* Bridge: `--pos 21055.7,10544.3 --yaw 281 --walk 2:9:W` climbs the stone arch from the quay (104.0) to the crest (113.74) at 5.4 m/s
  (`move_bridge3.png`); a quay railing at x 21060 stops the walk into the harbour (slides along it).
* Swimming: `--pos 15630,15352 --yaw 240 --walk 2:14:W --walk 15:16.5:X --walk 18:18.2:J`: wades (depth 0.69, 4.3 m/s), swims at 3.0 m/s from 1.3 m of
  water (`move_swim_14.png`, `move_swim_27.png`), dives (`move_swim_dive.png`), floats back up at 2 m/s, jumps out and falls back in.
* Saved heading: a walk at 281 degrees, then `--enter-world` without `--yaw` spawns with `facing -1.37` (281 degrees) at the saved position: the
  client sends its facing in every movement packet and applies the saved one.
* Glider: `cargo run -p aa-client --example glider_flight -- --mute --offscreen-shot shots/aa-client/glider_flight.png --shot-after 15`:
  the real `track_flight`/`control_glide`/`control_player` over `main_world` climb 2.05 m in the 0.70 s opening, glide 4 s at
  6.00 m/s forward and -1.45 m/s down, land and slide 0.98 s, then fold in the air and fall at -10.07 m/s. Mutations fail:
  the startup sign (1.32 m), the stop phase guard (20 s budget in `Sliding`), the fold (budget in `FoldFall`) and a
  disabled slide (budget in `SlideWait`).
