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
* the rider sits on the seat of its attach point in `model_attach_point_strings` (a creature's actor bone or a vehicle
  body's prefab helper, see "Vehicle and creature seats" below), faces the mount's facing, and
  plays the original player's mounted states (`player_full_body.xml`; the graph `Vehicle`/`VehicleSeat` comes from
  `FUN_39153b60`'s pose index and seat, selected by `FUN_39571f10`):
  * **get-on:** the attach reason 1..4 picks `fist_ac_horse_l_geton` (`Horse_Enter_l`, :1517), `_r_geton` (:1521),
    `_b_geton` (:1509) or `_f_geton` (:1513) — reason 1 left, 2 right, 3 back, 4 front; every other value maps to 1
    (`FUN_390eb520`). The one-shot runs once, then `Horse_DriverIdles_NullNode` (:1476) leads to the idle. The local player's
    reason is the one `hud/pets/mount_reason.rs` computes when `CSMountMate` is sent; a remote rider plays the reason of its
    `SCUnitAttached` reply (`aa_net::model::Attachment::reason`). The 1.2 client reads that byte (`FUN_397c60c0`) and its
    attach model maps 2/3/4 to those enter variants and every other value to 1 (`FUN_390eb520:187315-187327`); AAEmu
    echoes the `CSMountMate` reason (`mates.rs:340-344`), but the original server's value is unverified.
    The get-on is only for a **fresh** attach: the local player when a reason is pending, a remote unit when the attach
    arrives after this client saw it on foot. A remote rider that streams in already riding, one that leaves the drawing
    range mounted and returns on the same seat, and a local player entity built again while riding start in the riding
    loops; waiting for the rider's animator or the mount's data keeps a real on-foot-to-attach transition fresh, while a
    unit first seen attached never becomes fresh (a conservative rule; the real client's per-entity attach event is not
    in the 1.2 notes);
  * **riding:** `Horse_DriverIdle` (:1464), `Horse_DriverWalk` (:1490) and `Horse_DriverRun` (:1479) by the graph's
    PseudoSpeed bands 0..0.1, 0.15..0.6 and 0.7..2, with the `_FIST_MO_RIDER_HORSE_IDLETORUN` (:1474) and
    `horse_mo_relaxed_runtoidle_f` (:1488) one-shots between idle and run. Their `transition_time` 0.2 s is the blend; the
    one-shots play their own clip lengths (0.6-2.7 s, the `leave_at_stop` end) and the gait is chosen again from the speed
    at their end, so a speed change during one is taken up then (the graph also links the idle-to-run state to the
    run-to-idle one, :14555; whether the real client leaves early is unverified). PseudoSpeed is not m/s: 1.2 sets it to 1.0
    at an actor's full run (the Dash controller, `docs/research/re-skills.md`), so `gait()` divides the mount's speed by its
    `MountSpeed::run`; the 0.1..0.15 and 0.6..0.7 gaps keep the lower state. The scale is unverified;
  * **get-off:** `SCUnitDetached` plays `fist_ac_horse_l_getoff_start` / `_loop` / `_end` (:11733-11747; every kind's
    `jump_*` states use the horse's left clips, links :17356-17359), then restores the on-foot locomotion. The clips' own
    lengths pace it (the research notes 0.67 / 0.83 s are unverified);
  * the other kinds' states are in the same file: lion (:1601-1655), bear (:1285-1351), wildboar (:1839-1899), pangolin
    (:1684-1745) and elephant (:1383-1427). Bear and elephant share the horse idle/walk, wildboar the lion's; the get-on
    clips are `fist_ac_lion_*` (lion, wildboar), `fist_ac_bear_*` (bear) and `fist_ac_horse_*` (horse, pangolin, elephant).
    Pose 0 (donkeys, elks, yatas) keeps the horse set; the boat/car poses 5..8 have no `Enter`/`Exit` states in the graph
    and keep the old loop-only rider;
  * the `_FIST_MO_RIDER_*_RUN` / `_IDLETORUN` graph names are `.cal` locomotion groups; `bind_rider_clip` takes the
    group's `_f` example nearest the forward direction (0,1), not the first one (the horse run group lists the idle-to-run
    clip first), e.g. `horse_mo_normal_run_f`. The elephant's `Elephant_*` group (:1383-1427) has no idle-to-run / run-to-idle
    states of its own, but the lowercase `elephant_DriverIdleToRun` / `elephant_DriverRunToIdle` (:5266-5271) are reachable
    (`Elephant_DriverIdles_NullNode` -> `elephant_DriverIdleToRun` :14476, `Elephant_DriverRun` -> `elephant_DriverRunToIdle`
    :14479). Their clips do not load from the 1.2 data (the `.cal` points the idle-to-run at the misspelt `elehpant/` folder
    and has no run-to-idle), so the elephant crossfades directly instead of playing the horse's clips;
* the seat is the bone's place on the mount's last drawn pose carried to where the mount stands in this frame, so a rider does
  not trail a moving mount;
* the rider's state machine waits while its `Animator` is frame-held (`combat::framehold`, ice/stone): the get-on, gait and
  get-off timers and clip records stop with the animator's clock, so a hold cannot finish the get-on behind the freeze or
  take a getting-off rider off before the get-off clips played (they resume where they stopped);
* the rider's on-foot `Locomotion` is stashed and given back after the get-off clips.

`AA_FAKE_MOUNT=1` checks the remote riders without a server: a horse (model 139) and a rider that looks like the local player
appear next to the player and run a circle at 5 m/s.
The offset between the seat bone and the rider's origin is a first guess, not checked against the real client.

### Owned mounts (the local player)

* **Summon / dismiss:** using the mount's summon item from the bag (`items` with an `item_summon_mates` row, e.g. 4177 Lilyut
  Horse) is the normal item use (`CSStartSkill` with the item); the server's `SpawnMount` effect spawns the mate 3 m in front
  of the player (`SCMateSpawned`, then the unit). Using the item again despawns it. The original pet frame and pet bar
  (`hud/pets`) follow the unit: HP, MP, level, buffs (`SPAWN_PET` / `DISMISS_PET`).
* **Mount / dismount:** the pet bar's ride button (`x2ui/hud/pet_action_bar/pet.lua`, `PET_CMD_MOUNT`) calls
  `X2Mate:MountMate()` / `UnMountMate()`; `DismissPet()` sends the pet away. `CanMount()` gates the button: an alive own pet
  whose model has a driver's seat (`model_bindings` attach point 1), the player not riding, the pet within 25 m (the native
  squared distance 625), two seconds since the last ride request. `MountMate` sends `CSMountMate` (driver point 1) with the
  `FUN_3909ae90` reason 1..4 from how the player comes up to the mount; that reason is kept for the get-on state above and
  used only by a driver-seat attach of a mount with the graph's rider states. A pending reason is dropped when a passenger
  seat or a boat/car attaches instead, or when the player gets off, so a refused request does not play on another unit.
  `UnMountMate` sends `CSUnMountMate` from the seat the player is on with reason 9; `DismissPet` while riding sends
  `CSUnMountMate` reason 10 and then `CSRemoveMate`. `KickPassenger` stays a stub (the lane only lets a character leave its own
  seat).
* **Riding** (`mount::track_local_mount`, `LocalMount`): `SCUnitAttached` for the own object id seats the local player like a
  remote rider (same `Rider`, same states) and puts its `Mover` where the mount is, facing its way; `SCUnitDetached` plays
  the get-off state and then lets it walk on from there (the server puts the character where the mate stood);
  `control_player` ignores the movement keys until the get-off clips are over. Attaching
  again while the get-off plays unseats at once and starts the riding loops again (also for a remote rider). The camera's
  pivot rises with the seat (its height less 0.85 m, the seated rider's lower head).
* **Driving:** on the driver's seat the keys steer the mount: `control_player` moves the mover at the mount's speed and
  sends the movement as `Command::MoveUnit` with the mate's object id (`CSMoveUnit`; the server ignores a riding character's
  own movement). The speed is the mount's relaxed stance run speed (`game_stances.ai_move_speed_run`, stance 2: 9 m/s for the
  Lilyut horse, 7.2 for a donkey; `Db::mount_speed`) times the `MoveSpeedMul` of the mate's buffs (`Sprint::mount_speed_mul`),
  with the actor model's walk and backward factors; no jumping and no swimming while mounted. The mate's unit is marked
  `Driven`: `move_units` leaves it alone (its echoed movement packets are ignored) and `mount::drive_mount` puts it where the
  mover is with the mover's speed for its gait clips; when the player gets off, its track starts over where it stands.
* A passenger seat (point 2, someone else's mate) is followed, not steered: the mover goes with the mount.
* **Mount skills:** the pet bar's first five `ISLOT_PET_ACTION` slots (`hud/pet_action_bar/pet.alb`, capped by
  `MAX_PET_ACTION_SLOT` 5) map to the mate's displayed mount rows (`owned::displayed`; `GetMountSkill` is
  `FUN_3906aa90` -> `FUN_3933c340`) in the received `npc_mount_skills` order. A click (`AAUI.UseSlot` ->
  `HUD.request("use_pet_action", n)`) or a `pet_action_bar_button` hotkey runs the 1.2 gate `FUN_3933e630`: the
  slot must be shown, the mate level (`+0x4b4`, `MateSkillAccessor::Update` `FUN_3920a320`) must reach
  `skills.ability_level`, and the first `mount_attached_skills.attach_point_id` must equal the riding flag
  (`player+0x3af4`); an empty attach vector is any state. Live capture 2026-10-28 (Heir's Horse, item 27264,
  npc 13022, level 40): the five received rows `mount_skills` 83, 1, 13, 6, 3 map to skills 17092, 11016,
  11369, 11037, 11309 and display in exactly that order; on the level-5 Brown Lilyut Horse slots 2..5 report
  `learned` false (`skills.ability_level` 10/15/20/25), so learned is `ability_level <= mate level`.
  `GetMountSkill` `skillUsable` is 1 only while riding (0 on foot) for the captured target_type_id 0 rows; the
  decompile's target_type_id 6 display bypass stays ready in either state, and the server's cooldowns clear a
  usable row.
  A refusal (a not-learned slot, or any slot while not riding) is skill result `NOT_NOW` (`0x2c`:
  `FUN_39390900` -> `FUN_39897990`) with no packet and the local "Can't be used right now." text. While riding
  this mate (`pet.driving`) the client sends `Command::UseMateSkill` with the mate object as the kind-3 caster,
  `mount_skills.id` and the row's `skills.skill_id` (`FUN_39399630` -> `FUN_393989e0`), one packet; nothing is
  sent for the linked `mount_attached_skills` rider skill (the server casts it). Live capture 2026-10-28: the
  self rows (17092/11037/11309) were cast with no current target and carry target kind 0 / object 0; a targeted
  row with no current target also carries kind 0 / object 0
  (skill 11016, mount_skill 1: `08 2b 00 00 03 81 55 00 01 00 00 00 00 00 00 00 00`); with the Farm Wagon
  (0x6703) as the current target 11016 and 11369 carry kind 0 / object 0x6703. On foot 1.2 has no
  `pet_manual` pending mode: a click sends nothing, shows `0x2c`, and the next click/movement sends nothing.
  The wire body is in `docs/aa-net.md`.
* Getting thrown off is the server's: the owner's death despawns the mate (its riders get `SCUnitDetached` first), a stun,
  sleep or root and the skills marked `unmount` detach the rider, a blink unmounts it. The client also lets go of a mount that
  is no longer in view; a new spawn (`PlayerSpawn`, a world switch) or `Resurrected` clears the attachment, and leaving the
  game resets `LocalMount`.

### Ship and vehicle summon items (2026-10-30 capture)

Using a slave summon item (`item_summon_slaves`; e.g. Eznan Cutter 5223 -> slave 21) first runs the
1.2 client's own space search (`x2game` `FUN_3942c0e0` -> `FUN_3934a340` -> `FUN_39349a10` ->
`FUN_39347fb0`), before any packet. `hud/summon.rs::find_spot`:

* 40 steps at `slaves.spawn_x_offset + i` metres along the player's facing, and at each step three
  candidates sideways `{0, +slaves.spawn_y_offset, -spawn_y_offset}` (Eznan Cutter 30 / 20, carts
  5 / 5). The first candidate that passes wins.
* a ship (`models.sub_type` `ShipModel`, the client's `iVar3 == 4`; not the slave kind, so the
  carriage 715 and the other kind-1 `VehicleModel` rows are land vehicles) needs water there with
  terrain at or below the surface; a land vehicle needs terrain no more than 10 m over the player
  (`DAT_3999cce8`). A ship's spot is the water surface, a land vehicle's the ground.
* a ray from the player 1 m up (`_DAT_3999c488`) towards the spot must hit nothing. The port stops
  it 0.2 m over the spot: the native endpoint is the ground for a land candidate, where
  `WorldCollision::terrain_ray` samples a zero clearance and float noise makes it a hit at large
  x/y.
* the slave model's box at the spot, turned to the player's yaw + pi / 2, must overlap nothing
  (ships also test the terrain, land vehicles do not). The box is the LOD 0 model bounds of the
  prefab / CGF parts (`vehicle::bounds`), or for a ship without a readable model the
  `ship_models.mass_box_size` fallback; collision is the `WorldCollision` triangle soup and the
  terrain is sampled at the box centre and footprint corners. Bathymetry is exact only up to those
  samples.
* the 3D distance must be at most 80 m (`DAT_39ac740c` 6400).

The client's masks (`0xf`, `0x10f`, `0x107`) include rigid and living objects, so the original also
sees other ships, carts, players and NPCs; this port's ray and box only see static `WorldCollision`
and terrain, and the server's placement check still validates the spot. `FUN_39347e20` passes the
model box's `(min + max) / 2` unrotated, while the port rotates that model-space centre to the
player's yaw + pi / 2 (unverified; up to about 3 m on the galleon).

No spot shows `SLAVE_SPAWN_SHIP_NEED_MORE_SPACE` (error 100, "Summoning requires a larger space.")
and sends nothing (`hud::ShowError` -> `alerts::show` -> `SYSMSG`). `ground_target::route` runs the
check for every press of a mapped item, before the skill routing. When the search passes the item's
skill goes to `CSStartSkill` with an item caster and a `Position` target at the chosen spot, rot =
player yaw + pi / 2. The 2026-10-30 group-A2 capture pins the 57-byte body: skill 15802, caster_obj
31221, item 16777310 / template 5223, kind 1, x/y 12000/9030, z 99.8727646, rot 1.5707964, two zero
object ids and flag 0 (`captured_summon_cs_start_skill_carries_the_chosen_position_and_rotation`
in `aa-net/src/skill.rs`). An owned slave of the same item toggles the item into a cancel: within
`slaves.spawn_valid_area_range` (50 m in the 1.2 data) it sends `CSDespawnSlave`, farther away it
shows `SLAVE_DESPAWN_NEAR_THE_SLAVE` (error 312, "Too far away to cancel summoning.") and sends
nothing.

Pending live checks:

* the exact native cancel range and the state it reads (the port uses the database's
  `spawn_valid_area_range` 50 m and `SCMySlave`'s slave template id; the decompile path was not
  recovered);
* the real engine bbox and physics query masks against a capture at a dock, a beach and a narrow
  channel; the port approximates the box overlap with SAT against `WorldCollision` triangles and
  corner-sampled terrain, and treats missing collision cells as free;
* the mates' equivalent (`MATE_CANNOT_SPAWN_NO_SPACE` 108) uses the same text but was not searched
  here.

### Dismount placement (S2, 2026-10-28)

The local player's `SCUnitDetached` (`FUN_390a2e90`, `FUN_390a6110` / `FUN_390aaff0`) moves the character off the seat
instead of leaving it where the mount stood; a remote rider still unseats in place (pending). The reason comes from the
detach packet (AAEmu echoes the request: 9 for `UnMountMate`, 10 for `DismissPet` while mounted).

* **Side (`FUN_3909b1a0`):** `forward` is the mount's local Y flattened; the candidate is
  `mount + forward * 1.0 + left * 1.5` (`DAT_39ac3888` / `DAT_39ac388c`). The left candidate's capsule is tested first,
  then the right (mirrored); if both are blocked the character stands at the mount's position on the ground.
  `FUN_390a2e90` takes this path for reasons 9 / 10 / 0xb / 0xd, and stands at the mount for 3 / 8 / 0xc; the `(1,6)`
  call of every other reason is not modelled (it also stands at the mount).
  `FUN_3933be30` sweeps the capsule along the unit direction (`local_28`), not just its endpoint; the remake samples
  `WorldCollision::push_out` with the player's capsule (radius 0.35, height 1.8, step 0.5) at four points along
  mount -> candidate. It approximates the original's ray / `PrimitiveWorldIntersection` sweep and only knows the static
  collision (brushes, trees), not the living units around the mount or the exact primitive the original uses.
* **Controller and motion (`FUN_390aaff0`, `FUN_3909b4a0`):** the controller lives 1000 ms, 500 ms for reason 9; after
  100 ms it hands the rider to the normal state. `FUN_3909b4a0` puts the side probe xy and z = `DAT_39ac3890` 4.0 into a
  type-10 request. Modelled as a velocity: xy = the probe vector per second, z = 4.0 up, for the controller's window,
  under the player's gravity (25 m/s²) and collision (`WorldCollision::slide`, then the floor); the rider lands on the
  floor. The S3 get-off clips play during the move; the move owns the mover, then normal control resumes.
  The second phase after the window (0.5-0.8 s) is not the request: it is modelled as the get-off clip's root motion,
  0.61 m forward and 0.19 m right of the mount over 0.3 s (the capture's 0.5-0.8 s displacement); this is a documented
  stand-in, not a proven clip root motion.
* **Capture `c_dismount`** (`tools/evidence/mount_animations/native.json`, Heir's Horse, 2026-10-28): the mount's last
  `CSMoveUnit` before the detach is at (4366.246, 4148.893, 98.669) with direction byte 44 (123.75 deg); the seat start
  is (4366.231, 4148.880, 100.166). The probe vector `1.0 fwd + 1.5 left` = (0.002, -1.803) is world -Y, and the rider's
  x stays at 4366.23 while y drops 0.77 m in the first 0.5 s (about 1.5 m/s after a ramp): the first phase moves along the
  probe. z rises 0.375 m by 0.2 s (4.0 up under gravity 25 gives a 0.32 m peak). The rider's last sample is
  (4365.625, 4147.931, 98.500), 1.05 m forward and 0.45 m left (1.145 m out); the last 0.3 s adds (-0.61, -0.18).
  Simulated: at 0.5 s it is 0.90 m along the probe (0.13 m short of the capture in xy; z is 0.82 m lower because 25
  gravity drops the hop faster) and the end is 0.13 m from the captured end; the hop peak is 0.32 m vs 0.375 m
  (0.055 m). The capture's decompile probe is 1.803 m out; the character stops 0.66 m short of it, which the root-motion
  stand-in covers, not the request.

### Live capture 2026-10-28

The S3 run (`captures/2026-10-28/s3/native.csv`, Heir's Horse 27264) recorded:

* `SCUnitAttached` 0x076 is a 14-byte frame: 6-byte header and 8 body bytes. The S->C log records only opcode and
  byte count, so the capture does not show the reply's reason byte. The 1.2 reader names the 8 body bytes:
  `childUnitId` (`FUN_397c60c0:1721015`), `point` + parent (`FUN_397c03b0:1716932-1716940`), `reason` at +0x18
  (`FUN_397c60c0:1721019-1721024`). `aa-net` keeps the reason in `Attachment::reason`; the 1.2 client passes it to
  the get-on state: `FUN_390eb520:187315-187327` maps 2/3/4 else 1 and calls `FUN_390bb4d0`, the thunk that calls
  `FUN_39153b60` (`x2game.dll.c:154663`).
* `CSMountMate` 0x0a7 body is `50 00 01 <reason>` (tl 0x50, attach point 1); the four sides, walked to at
  4.11-4.50 m, sent `01` / `02` / `03` / `04`, each followed by `SCUnitAttached` (size only in the capture).
* Dismount: `CSUnMountMate` 0x0a8 body `50 00 01 09` (reason 9); `SCUnitDetached` 0x077 arrived 2063 ms after the
  request, measured with the old timing code (its total included the 1.5 s sampling window), so it is an upper bound
  and was not re-measured. The rider's z then left the seat height (100.17 at 0 ms, 97.97 at ~800 ms, settled at 98.43 by
  ~1300 ms; 100 ms samples). The client keeps its get-off chain and the placement above moves the local character; the
  server also puts its own copy where the mate stood.
* Context only, mounted ground speeds measured from standstill: walk 1.25 m/s mean (max 2.61), run 1.42 m/s mean
  (max 2.50); the short run leg only reaches the horse's run gait at its end.

### Pending live checks (mount animations)

* The get-off animation state the client reports while the clips play (key `0x103`, `DAT_39ac3f28`; probably Action
  `jump_horse`) and what a seat >= 5 (the graph's exit) plays: the client currently uses get-off only. Not readable
  by the 2026-10-28 driver.
* The one-shot lengths: the get-off clips are 0.667 / 0.833 / 0.833 s in the pak (the research notes 0.67 / 0.83), and
  the code uses their own lengths; whether `_getoff_loop` repeats until the dismount controller's 1000 ms (500 ms for
  reason 9) ends. The gait transition one-shots now play their own clip lengths with the graph's 0.2 s blend; whether the
  real client lets a speed change leave them before their end (the graph links `Horse_DriverIdleToRun` -> `Horse_DriverRunToIdle`,
  :14555) is not proven. The capture had no animation accessor; only the packet timing above was recorded.
* Whether a rider that streams in already attached plays the get-on (the code starts it in the riding loops). Not
  captured (one client only).
* The detach reply latency: the capture's 2063 ms was measured with the old probe timing (its total included the
  sampling window) and is an upper bound. A later capture with the fixed probe must measure `SCUnitDetached - request`.
* The PseudoSpeed scale: the code treats PseudoSpeed 1.0 as the mount's `MountSpeed::run` (the Dash controller's value)
  and keeps the lower state in the 0.1..0.15 and 0.6..0.7 gaps. The real client may compute it from the mount graph;
  not captured.
* Whether the elephant idle-to-run really fails in the real client: the `.lmg` exists under `lmg_files/` and the pak has
  `animations/nuian/male/elephant/elephant_mo_relaxed_idletorun_f.caf`, but the `.cal` path is the misspelt `elehpant/`
  folder; the code uses that path and therefore crossfades directly. Not captured.
* Whether pose 0 mounts (donkeys, elks, yatas), the boat/car poses and the other kinds really use these states; the
  graph's own `Enter` states were followed where present. Not captured (only a horse was mounted).
* The mounted jump clips (`horse_mo_jump_f_start` / `_land`, :1498-1502) are not played yet. Not captured.
* The `SCUnitAttached` reason byte from the original server, and for slave bonds (`bond_kind` reasons 5/6/7/10);
  AAEmu echoes the request reason or the bond kind, the original server is unverified.
* Getting off a vehicle seat (a `$` helper: the wagon's bench, a carriage's back seat): the remake skips the 1.5 m
  side probe, which is fitted to a horse, and starts the hop from the drawn seat, so the rider lands under it. Whether
  1.2 uses the vehicle's own exit points is not captured.
* A remote rider's `SCUnitDetached` placement: the code still unseats it in place because the 2026-10-28 run had one
  client. A two-client capture must record the remote rider's landing and the reason it gets.
* The capsule test `FUN_3933be30` against the remake's four-point `push_out` sweep: a dismount beside a wall, a fence
  and the mount itself, on a slope and on a bridge, with the right side chosen. The original may test the mount's
  dynamic body and a different capsule; the remake sees only static collision.
* The type-10 request of `FUN_3909b4a0`: the velocity hypothesis (xy = the probe vector per second, z = 4.0 up for
  the window, gravity 25) fits the first 0.5 s to 0.13 m but not the second phase (modelled as root motion) or the z
  after 0.3 s. A right-side and a reason-10 dismount must show whether the probe velocity, the 0.61 / 0.19 m root
  motion and the 0.45 m end are general or specific to this spot.
* The 100 ms hand-off and the 1000 / 500 ms finish against the get-off clip chain: whether the character can move
  before the clips end, and whether the z hop / settle (100.17 -> 97.97 -> 98.43) is the animation's root motion or
  the physics.

## Vehicles (`vehicle.rs`)

Vehicles and ships are drawn from the `normal` look of their model (`models` -> `vehicle_models` / `ship_models`): the
prefab's body and animated parts, plus the wheel models on its `wheelN` / `wheel_type2N` mounts. The 1.2 farm wagon
(model 1008, slave template 60) is prefab `prefabs/vehicle_tractor.xml/vehicle_tractor.tractor_body` ->
`objects/env/06_unit/05_carriage/transfers_trailer_a_body.chr`; its `transfers_trailer_a_body.mtl` sub-material `metal`
is `Shader="Illum"`, `GenMask=0x1000010` (GLOSS_MAP), Diffuse 0.37 grey, Specular (2.22, 1.75, 1.41), SpecularLevel 5.2,
Shininess 5.9, textures `transfers_trailer_a01_df/_sp/_ddn`. The copper boiler's colour comes from that Illum gloss, so
vehicle parts are prepared with `Shading::Full` (native Illum, `aa-cry/src/shade/illum.rs`) instead of the default Basic
world-prop path.

### Vehicle and creature seats

Riders resolve their seat through `model_attach_point_strings` (`Db::attach_point_strings`) by attach point, not by a
hard-coded bone. Creatures use the actor bone; vehicle models use the prefab helper. Each point keeps its own row
(3..8 are `$passenger1`..`$passenger6`, not `bone_spine_passenger`):

| Point | Actor (creature) | Prefab (vehicle) |
|---|---|---|
| 1 | `bone_spine_driver` | `$driver` |
| 2 | `bone_spine_passenger` | `$passenger0` |
| 3..8 | `bone_spine_passenger1`..`6` | `$passenger1`..`6` |

A vehicle root has no rider `Animator`; `vehicle.rs` collects the body's `$` helpers from its skeleton bones and model
nodes (the same source `mounts()` uses for wheels), composes them with the prefab part transform, and stores them in
`SeatHelpers` on the root (the body's own helpers win over a same-named wheel part's). `mount::seat_world` uses a
creature's animated bone when there is one and these helpers otherwise. The real wagon body
(`transfers_trailer_a_body.chr`) has `$driver` at (-0.0172, -0.4972, 1.3532) model (Cry) space; the client converts it
to Bevy space through the root. For the local player `pose_local_rider` sets `LocalSeat.lift` from that world seat
position (its height over the mover), and `camera` raises the pivot by `LocalMount::lift()` (that height less
`SEATED_DROP`), so the camera follows the helper seat.

A rider on a vehicle model (its own or someone else's) also takes its pose from the 1.2 `attach_anims` table
(`Db::attach_anim`: `owner_type` `VehicleModel`, `owner_id` the model's `sub_id`, `attach_point_id` the seat): the
Farm Wagon's slave 60 uses model 1008, whose `sub_id` 38 has point 1 -> `anim_actions` 12
`fist_ac_steer_sit_idle` (43 rows over the vehicle models: `fist_ac_cannon_idle`, `fist_ba_siegeweapon_idle`,
`fist_ac_ballista_idle`, `fist_ac_trailer_sit_idle`, `speedcar`, `fist_ac_sit_down`, `music_co_sk_drum_s`, `idle`).
The rider plays that clip as its idle loop instead of the mount's rider set (`Rider::step` ignores the mount speed
while posed), so the wagon driver sits with the hands on the wheel. A vehicle seat with no row keeps the mount's rider
set (the wagon's passenger seats fall back to the pose-0 horse loops); creatures never have a row.

## Pending live checks (farm wagon)

* Lit copper boiler: `Shading::Full` is source-verified (Illum + gloss map), but the final look is view/sun dependent and
  has not been compared with a live 1.2 capture.
* Crate placement: the four storage boxes use the server's `slave_attach_points.json` points (9..12), which differ from
  the prefab's cannon markers by roughly 0.08 m in X and 0.03 m in Y (see `docs/aa-client-doodads.md`).
* Rider pose on a cart: seat height/pose from the `$driver` helper has not been compared with a 1.2 capture (the
  1.2 screenshot `shots/cart-12/cart12-03-driver-side.png` shows the driver on the bench, hands on the wheel, which
  the `fist_ac_steer_sit_idle` loop is meant to match).
* The vehicle seat's get-on and get-off: `attach_anims` only names the seated loop, so no get-on one-shot plays and a
  detach unseats at once; whether the 1.2 client plays another clip there is not in the data.

### Pending live checks (mount skills)

- Passenger seat (point 2) riding flag: whether `player+0x3af0` naming a point-2 seat also counts as riding
  (the remake stays driver-only); live capture 2026-10-28 had one client only.
- A `target_type_id` 0 self row (e.g. 17092 "Run!") cast with a target selected: the capture's self rows had
  no target, so whether 1.2 sends the current target (the remake does) or object 0 is unverified.
- `GetAutoStartMountSkill`: source and meaning (the function exists as `X2Mate:GetAutoStartMountSkill`; it was
  not read); the remake stays false.
- Error ordering against a server `SCErrorMsg` (`0x10A` in 1.2; the older `0x0A0` did not appear in S4):
  the local `0x2c` refusal is confirmed and no `SCErrorMsg` was received in the S4 slice, so whether a server
  error can arrive before/with/after the local one is still open.

### Slave and cart boarding (S5, 2026-10-28)

Carts and other vehicles are slave units (`UnitKind::Slave`, unit kind byte 2). The original Lua
namespace is `X2SiegeWeapon` (`strings.txt:10298-10314`), not `X2Slave`. The client side is
`hud/slave.rs`; `aa-net`'s `SlaveCommand::{Bind,Unbind}` already writes `CSBindSlave` 0x031 and
`CSDiscardSlave` 0x032 as a `u16` timeline id (`docs/aa-net.md`, "Native slave position reports
and explicit controls").

* **Right-click interaction (corrected 2026-10-28).** A right click on a slave (the pick in
  `hud/dialog.rs` accepts unit kind 2) sends `CSChangeTarget` 0x02c when the slave is not the
  target yet, then `CSStartInteraction` 0x068 with the slave as `npc_obj`, `obj` 0, `extra_info`
  1, `pick_id` 0, mouse button 3 and modifiers 0 (`FUN_390f3780` -> `FUN_393be030`). The
  2026-10-28 capture sent exactly `3c 5f 00 00 00 00 01 00 00 00 00 00 00 00 03 00 00 00 00`
  for cart bc 0x5f3c. The earlier claim that slaves never send 0x068 is **corrected** by that
  capture; the click does not send 0x031. While already bound to or seated on the clicked slave,
  a right click sends only `CSChangeTarget` 0x02c (capture case e: four sweep points, no 0x068,
  no 0x031, no message); this port did the same after 2026-10-28.
* **Boarding.** `SCNpcInteractionSkillList` 0x07b for a slave acts on the FIRST skill at once
  (`FUN_393be850` -> `FUN_393be280`). 12076 ("occupy") checks the interaction reach first
  (`FUN_393bf270` / `FUN_39093860`: `DAT_39ac79fc` 9.0, or `DAT_39ac7a00` 12.25 when the player
  unit's +0x3c26 mounted byte is set, square-rooted to 3.0 / 3.5 m), then the bind checks of
  `FUN_39346290` / `FUN_393464a0`, in this order: the slave is dead ->
  `SLAVE_CANNOT_BIND_WHILE_IS_DEAD` 0x144; our own unit is dead -> `CANT_DO_WHILE_YOU_DEAD`
  0x14c (the native bind path shows `FUN_39390900(2)`, a different message table, so the exact
  text is unverified); we are bound to another slave -> `SLAVE_UNBIND_FIRST` 0x66; we are bound
  to this one -> silent; a bind is already pending -> `ALREADY_REQUESTED` 0x13; the bind reach
  (the same predicate, `DAT_39ac7408` 16.0 -> 4.0 m) -> `TOO_FAR_AWAY` 0x9a. The reach is the 3D
  **surface gap** of the collision shapes (`FUN_3989bf90` -> `FUN_398b1ab0` ->
  `FUN_398b1af0`): the minimum over the target's colliders of
  `|centre_player - centre_slave| - (r_player + r_slave)`, not clamped at 0, compared strictly
  (`gap < limit`). The exact per-collider shapes are not recovered; the port uses one scaled
  actor-model radius per unit (the same `GameData::model_size` as the server's `Unit.ModelSize`
  and `api::distance_to`), an approximation (unverified). The 2026-10-28 capture bound at a
  3.1022 m centre-to-centre 3D distance with no walking; the capture's Farm Wagon (slave
  template 60, model 1008) has no actor-model radius (it is a `VehicleModel`), so the player
  radius alone (0.4-0.55 m for the 1.2 races) leaves a 2.55-2.70 m gap. Then `CSBindSlave` 0x031
  goes out (`FUN_393bf310` -> `FUN_39362f50` -> `FUN_393464a0`, `SlaveCommand::Bind`). Out of
  reach the native client auto-walks (`FUN_390a6af0`); this port has no walk-to-unit, so it
  refuses with 0x9a. The mounted 3.5 m variant is modelled from the player's received
  attachment. A refusal shows the same `ERROR_MSG` text as the server's `SCErrorMsg`
  (`hud/alerts.rs`) as `SYSMSG`. Other first skills (13863 passenger, 19165 destroy, 0, unknown)
  do nothing: 1.2 sends `CSStartSkill` for the two vehicle skills, and this client has no
  generic interaction skill-cast path yet.
* **Seat position.** The checks measure from the player when on foot and from the attached
  slave's replicated position when seated: the server moves only the slave, while the local
  player's transform can stay at the boarding point (a point-1 seat marks the cart `Driven`, so
  `drive_mount` moves the cart from the stale `Mover`, and a cart has no `Animator` for
  `pose_local_rider`). The 2026-10-28 replay refused a while-bound click from that stale point
  ~7 m away; the fix is `hud/slave.rs::player_centre`.
* **Interact key.** F sends nothing for a cart (the 2026-10-28 capture with a cart targeted and
  an NPC at 1.24 m produced no packet); NPC talk keeps the path it had.
* **Bound state.** `SCSlaveBound` 0x064 (`FUN_391d4760` -> `FUN_39345d00`) with the local
  character as master stores the slave object; `hud/mounted.rs` keeps it with the slave's timeline
  from the loaded unit. `SCUnitAttached` 0x076 remains the authoritative seat state
  (`FUN_39362770`), and a seat attachment to a slave wins over the SCSlaveBound fallback for the
  checks above.
* **Pending bind.** The pending timeline is cleared by `SCSlaveBound`, a local
  `SlaveCommand::validate` rejection (not a server reply), the server's `SCErrorMsg` bind
  refusals 0x60/0x61/0x66, our own unit's death, detach or resurrection, and `SCSlaveRemoved` /
  `SCSlaveDespawn` / `SCEscapeSlave` / `UnitsRemoved` for the slave. A 5 s timeout is a safety
  bound, **not a 1.2 value**: the native client waits for its reply, but a lost or silent one
  must not make every later click show 0x13.
* **Stale bound.** `SCSlaveDespawn`, `SCEscapeSlave`, `SCSlaveRemoved` and `UnitsRemoved` clear
  the bound slave when they carry its object id, so a despawned or escaped cart cannot keep
  refusing other slaves with `SLAVE_UNBIND_FIRST`. The player's own `Resurrected` also ends the
  seat: `Player::apply` clears `attached_to` (`aa-net/src/state.rs`), and `hud/mounted.rs` clears
  the cached seat link and the bound fallback with it (no separate `SCUnitDetached` is needed).
* **Leaving.** `X2SiegeWeapon:DiscardSiegeWeapon` (native `FUN_394aa090` -> `FUN_39345aa0`) sends
  `SlaveCommand::Unbind` 0x032 for the timeline of the seated slave, else the SCSlaveBound one. While the slave's speed squared is above
  `_DAT_3999f85c` 25.0 (5 m/s) it is refused with `SLAVE_SHUT_OFF_TO_UNBIND` 0x67 instead. The
  speed is the latest replicated `SCOneUnitMovement` velocity (`Movement.velocity`: 1/1024 m/s,
  30/32767 m/s for a type-2 vehicle body), or the controller speed for the cart this client drives
  (`cart::CartDrive`, which no `UnitMoved` reports), treated as 0 after 500 ms without a new report (a port bound, **not a 1.2 value**); the native
  getter `vtable+0xa4` is an engine speed, so this source is **unverified**.
* **Passenger walk-off.** With the own attachment point above 1 on a slave, movement input
  (`moveforward`, `moveback`, `moveleft`, `moveright`, `turnleft`, `turnright`, `autorun`, and
  the world mouse buttons through the same `resolve_keys` mapping `control_player` uses: both
  buttons run forward, one button makes A / D strafe) sends the parent's 0x032 through
  `FUN_39362170`, throttled to one request per 1500 ms (`_DAT_3a169948`). That native path has no
  speed check, unlike the driver's `FUN_39345aa0`. Like `talk_input` it is skipped while the HUD
  is suspended or loading, while a widget has the HUD focus, and while the UI owns the keyboard
  (typing in a field); mouse buttons held over a HUD widget are ignored like they are on foot.
* **Not ported here:** ships, and the despawn/destroy lifecycle (0x02f/0x030).
  Cart driving and the wheel visuals are in the next section. The player flags at +0x3a70..+0x3a73/+0x3a7b (native 0x13)
  and the parent unit type 0xe (native silent refusal) are also not ported: their meaning is
  unverified.
  The current UI has no mounted-HUD release action that calls `DiscardSiegeWeapon` (the original
  scripts list it as unused), so the Lua entry point and the passenger walk-off are the two ways
  a client action sends the unbind.

#### Pending live checks

* `CSBindSlave` 0x031 body is exactly a `u16` timeline id (the decompile only shows a 32-bit
  setter; the 2026-10-28 capture never reached 0x031).
* The pick suppression distance: 1.2 sent nothing for a cart picked at 4.57 m and 0x068 at
  2.46 m; the exact check is unknown. The port keeps its view-ray pick (`PICK_RADIUS`) and adds no
  range test.
* The exact collision shapes behind the 3.0 / 3.5 / 4.0 m surface gaps are not recovered; the
  port's per-unit scaled actor-model radii are an approximation. The mounted 3.5 m variant is
  modelled from the attachment, but the native +0x3c26 byte needs a live mounted pick to confirm.
* The 3.5 m and 4.5 m interaction picks could not be measured: the native pick never landed in
  the 2026-10-28 capture (44 sweep points, no 0x02c/0x068), so only the 3.1022 m and 2.9008 m
  binds are live evidence. A later capture must land the pick to check whether 1.2 acts at 3.5 m
  (shifted target flag) and refuses at 4.5 m.
* `FUN_393bf270` skips the reach check when `param_4 == 0` or the skill is `DAT_39ac7978` /
  `DAT_39ac796c`.
* The auto-walk to a slave out of reach (`FUN_390a6af0`) and its re-entry into the interaction.
* F / dynamic action bar 12076 path (`FUN_393b1de0`): the capture with a cart targeted and an NPC
  at 1.24 m sent nothing; the port sends nothing for a cart on F.
* 13863 (passenger) and 19165 (destroy) `CSStartSkill` targets and fields; the port ignores them.
* Server side: the lane's `CSStartInteraction068` only answers NPCs/mates; a target server must
  answer a slave with `[12076]`.
* The server's reply to a refused/dropped bind: which `SCErrorMsg` codes arrive (the port assumes
  0x60/0x61/0x66), whether silence happens, and whether 5 s is a safe pending timeout (not 1.2).
* The exact native bind-path player-dead message (`FUN_39390900(2)`) and whether 0x14c
  (`CANT_DO_WHILE_YOU_DEAD`) shows the same text.
* The player flags +0x3a70..+0x3a73/+0x3a7b -> 0x13 and the parent unit type 0xe silent refusal,
  both not ported.
* Leaves and stale state: 0x032 followed by `SCUnitDetached` 0x077, 0x67 above 5 m/s, and the
  native speed source and staleness (the port ages the last velocity after 500 ms, not 1.2);
  `SCSlaveDespawn`/`SCEscapeSlave`/`SCSlaveRemoved`/visibility loss clearing the bound slave.
* The 1500 ms agent throttle, the driver/passenger seat difference, a non-owner passenger, and
  whether any mounted-HUD button or key (not just `DiscardSiegeWeapon` and movement) leaves.
* A slave with a zero timeline (the port stays silent; not a 1.2 check), and whether movement
  while typing in a field or on a loading screen leaves a passenger seat (the port ignores it).
* Owner and a second player, dead, combat and trade-pack cases; the S5 capture is
  `tools/evidence/cart_boarding/`.

### Passenger seats (S11)

The 1.2 slave interaction offers two seat skills on the target unit. `hud/slave.rs` acts on the FIRST
`SCNpcInteractionSkillList` 0x07b skill (`FUN_393be850` -> `FUN_393be280`):

* **12076 "occupy"** is the driver's seat: the existing bind checks and `CSBindSlave` 0x031 (attach
  point 1).
* **13863 "passenger seat"** is a plain `CSStartSkill` 0x052 with the **slave** as the unit target and
  the player as the unit caster (`FUN_393bf310` -> `FUN_39362f50`). The server's `AttachTo` effect
  (special type 53, `value1 = 2`) attaches point 2 (`$passenger0`); the client never sends a seat
  number. `CSStartSkill` bytes: `u32 13863, u8 caster kind 0 + bc player, u8 target kind 0 + bc slave,
  u8 0` (13 bytes), checked against the server reader. `FUN_39362f50` refuses a dead target with
  0x144 for 13863/19165, and the port applies the same dead check to the passenger skill.
* The common interaction reach (`FUN_393bf270`) applies to both: the 3D surface gap must be strictly
  under 3.0 m (3.5 m on a mate seat). The 4.0 m bind check belongs to 12076 only. Riding a mate is not
  a refusal on this path: only the reach widens (capture-pending S11-C5).
* **Switching** has no packet: a character leaves with `CSDiscardSlave` 0x032 and boards the other
  seat afterwards. The occupy skill on the slave already occupied is silent. `FUN_39362f50` has no
  seated refusal, so the port does not invent one; a right click while seated sends only
  `CSChangeTarget`, so the interaction request does not normally arise on a second seat (capture-pending
  S11-C2).
* **Seat pose.** A vehicle seat takes its loop from `attach_anims` (`fist_ac_trailer_sit_idle` for the
  red ox cart point 2, `fist_ac_sit_down` for the cars, `fist_ac_cannon_idle` for the tanks). A vehicle
  **passenger** seat (point 2 and above) with no row has no seated pose: the ship passengers stand on
  the deck in the normal idle; they no longer fall back to the mount's horse/car rider loops. Point 1
  with no row keeps the mount's rider set (the ship helm), because the evidence covers passenger
  points only (capture-pending S11-C1).
* **Helper names.** The attach strings name `$passenger0`, but some bodies (the red ox cart's
  `transfers_trailer_c.chr`) name the bones `passenger0` / `driver` without `$`. `vehicle::helper_key`
  accepts both spellings; before that the cart passenger fell back to the cart origin (capture-pending
  S11-C3).
* **Remote passengers.** `SCUnitAttached` point 2..8 sets `Unit::attached_to`; the seated unit gets no
  movement of its own. `mount::start_stop_riding` seats it and `mount::pose_riders` puts it at its own
  `$passengerN` helper on the vehicle entity's **drawn `Transform`** (the S8 branch replaces that same
  input with the smoothed hull). `mount.rs` tests cover two remote passengers at different helpers and
  a moved cart transform (capture-pending S11-C4).
* **Leaving.** The local driver uses `DiscardSiegeWeapon`/the speed-guarded 0x032 (S5); a passenger
  walks off on movement input (1500 ms throttle) or sends the same 0x032. On `SCUnitDetached` the local
  seat clears and the mover starts from the seat helper, as before.

Capture-pending (S11):

* **S11-C1** ship/Farm Wagon passenger pose and the row-less helm. Two clients; `/slave spawn 21` (or
  a clipper) and `/slave spawn 60`; B boards point 2 with 13863 and A captures B's drawn clip and
  relative offset for ≥ 5 s. Expected: standing idle, not a rider loop. Then B takes the helm (point 1)
  and A captures its row-less pose, which the port currently leaves as the mount rider set.
* **S11-C2** switching and seat-taken feedback. B on the driver seat (12076), then attempt 13863 on the
  same slave; with a third client C holding point 2, capture the 0x07b list, any `SCErrorMsg`, the
  order of 0x032/0x052 and whether 1.2 ever offers or acts on a second seat while seated (expected:
  none).
* **S11-C3** helper spelling. With B on the red ox cart's passenger seat, read B's attached-entity
  relative transform (or the rendered pelvis) in the original and compare it with the model's
  `driver` / `passenger0` bones; confirms the `$`-less matching.
* **S11-C4** remote passenger. A drives the red ox cart; B rides point 2 and C observes both the
  SCUnitAttached bytes and B's drawn per-frame pose. Compare with the Bevy replay (the cart transform
  moves in `examples/passenger_seats`). Also confirm that point 3+ is never offered: the only
  `AttachTo` seat skill in the data has value1 2.
* **S11-C5** passenger seat while riding a mate. B mounts a mate within 3.5 m of the cart's passenger
  point, presses the 13863 interaction and records whether the server detaches the mate, refuses the
  seat, or attaches both; the port currently sends 13863 (the interaction reach is the only mounted
  difference).

### Cart driving (S6/S7, 2026-10-29 capture)

A slave at attach point 1 is driven by `ingame/cart.rs`: the movement keys go to the cart
(W/S throttle, A/D steering) and `player::control_player` sends no on-foot movement while it
is driven. The original client runs a CryPhysics wheeled simulation (`FUN_390c1b80`,
`FUN_390abe10`); this port uses a kinematic stand-in. The keys are read like
`control_player`'s (bound keys plus the script runner's held keys), so the script `hold`
command drives an unattended replay. The cart's unit, `RemoteUnit` and
transform follow the controller and the driver's mover stays on the cart (the seat state is
the S5 attachment). A driven cart is marked `Driven`, so `units::move_units` does not fight
the controller for its transform; remote carts keep their received type-2 path
(`docs/aa-net.md`). Leaving is the S5 `DiscardSiegeWeapon` path below: the captured
leave-while-moving case (last cart velocity 3.54 m/s, measured 3.63) is below the 5 m/s
refusal bound and sends `CSDiscardSlave` 0x032 (`hud/slave/tests.rs`).

The send pump (`FUN_394045f0`) has a 100 ms gate checked once per frame and no change test,
so a still cart is resent. At the captured 22.5 FPS that is one send every 3 frames (143 ms
median for every case); the body is `CSMoveUnit` type 2 with flags 0, no scType/phase block
and wheel count 0. The capture (Farm Wagon, slave template 60, model 1008) pinned the rest:
velocity scale 30/32767 (m/s × 1092), world-space 3D velocity, the (x,y,z)-short quaternion,
`angVel.z = 0.4 * steering` rad/s, and no wheel angular velocities on this vehicle.

Stand-in values fitted to `tools/evidence/cart_driving/native.json`: top speed 4.0 m/s both
ways, acceleration and braking 2.5 m/s² (0 to 4.0 m/s in 1.73 s including the input delay,
4.0 to 0 in 1.77 s), yaw rate `0.4 * steering` rad/s, and a steering ramp of 5.5/s (0.245
per 22.5 FPS frame, full lock in ~4 frames) fitted to the captured intermediates 0.484
(two frames in), -0.752 (one frame after release) and 0.269 (three frames after release).
`TOP_SPEED`, `ACCEL`, `YAW_RATE_PER_STEER` and `STEER_RATE` are in `cart.rs`. `drive_cart`
stops the controller and send pump while the driver is dead, and when the server's cart pose
differs from the last written pose by more than `RESYNC_DISTANCE` / `RESYNC_YAW` (2 m /
0.35 rad) it resyncs the controller pose instead of overwriting the server's move every frame.
The stand-in fades its turn out below `TURN_FULL_SPEED` (0.5 m/s) and mirrors it in reverse, so
a still cart does not turn on the spot; the wire `angVel.z` keeps the captured
`0.4 * steering` (**not captured**: a turn at a standstill or in reverse). The captured
quaternions show the yaw is applied about the cart's own up axis (`step` post-multiplies the
turn when the ground is missing).

#### Terrain following (C8a)

The Farm Wagon is not a wheeled-sim vehicle (`vehicle_models.use_wheeled_vehicle_simulation`
is false for model 1008), so `CartController::step_grounded` keeps the kinematic motion and
samples the terrain under its four wheel mounts. `WHEEL_CONTACTS` in `cart.rs` is read from
`transfers_trailer_a_body.chr`: front `wheel1`/`wheel0` at y 1.6104 m, x +0.8350/-0.8431,
rear `wheel_type21`/`wheel_type20` at y -2.5453 m, x +1.5295/-1.5212 (wheelbase 4.1557 m,
mean track 2.3644 m). `pitch = atan2(front - rear, WHEELBASE)`,
`roll = atan2(left - right, TRACK)` and the plane is evaluated at the body origin, not at the
four-wheel centroid `WHEEL_CENTROID` (y -0.467 m): `z = mean + REST_OFFSET - gx*cx - gy*cy`,
with `gx = -tan(roll)`, `gy = tan(pitch)` (0.024 m REST_OFFSET, the flat `b_forward` capture
exactly). Each wheel probes the highest static surface within a 0.5 m step through
`WorldCollision`/`player::floor_at` (bridge decks, floors, then the terrain; terrain alone
when collision is absent), and a probe that misses keeps its last height (`resync` clears them
for a server teleport). The full rotation
`Rz(yaw) * Ry(roll) * Rx(pitch)` goes in the type-2 body and the cart's `Transform`, and the
driver's own mesh takes the mount's rotation (`mount::pose_local_rider`), so it tilts with the
cart like the native driver quaternion; attached doodads follow the root. `vel.z = dz/dt`
keeps the 1/1092 m/s wire scale and is negative downhill. `Unit.rotation` is `[roll, pitch,
yaw]` (aa-net), and `drive_cart` writes that order.

The capture's tilt does not snap: the `e_slope` samples still carry the previous slope's
-0.068/+0.069 rad on flat ground and relax over 2-3 s, so tilt uses `TILT_TAU = 2.5 s`
(mean replay error 0.007/0.010 rad) while height uses `HEIGHT_TAU = 0.15 s` (mean z error
0.029 m, max 0.043 m after the body-origin fix). A 0.15 s tilt filter would lag the captured
roll by 0.064 rad. Unit tests cover the plane fit and its body-origin height, both time
constants, a terrain miss, the downhill `vel.z`, flat ground (`pitch 0`, `z = ground + 0.024`)
and the full quaternion on the wire; the fixture test replays the 58 captured `e_slope` poses
with the terrain read at run time from the user's pak (skipped without `AA_PAK`, no terrain
data in the repository) and checks the capture's own `terrain` samples against it (tolerance:
mean <= 0.02 rad / 0.035 m, max <= 0.03 rad / 0.05 m). The evidence is in
`tools/evidence/cart_terrain/`: the 2026-10-29 22:43 replay gives flat `z - terrain` +0.024 m
(native +0.024) and e_slope +0.029 m (native +0.028), with a matched mean z error of 0.024 m.
A synthetic deck test keeps the wagon on the deck, and a pak-backed test takes the Lutesong
bridge deck over its river bed.

Out of scope: collision with rocks and bushes (the `g3` case's 0.3 m bumps) and full wheeled
physics for the nine car models. Remote carts' tilt now comes from the received type-2
quaternion (S8 below). Pending live checks: the server's echo/correction of a tilted type-2
pose, `vel.z` on the wire over the slope, and the cart's tilt while turning on a slope.

#### Wheels and spindle (S6/S7 step 3)

`ingame/vehicle.rs` draws the wheels as before and now turns them from the cart's motion:

* `vehicle::track_motion` measures each vehicle root's horizontal velocity and yaw rate from its
  transform every frame (a step over 2 m is a spawn/teleport and gives no motion). This covers
  the driven cart (`cart::drive_cart` places it) and remote carts (`units::move_units` plays back
  their type-2 reports); no wire change is needed.
* `vehicle::spin_wheels` ports `FUN_390d6b10` (0x390d6b10, non-physics branch): each wheel
  stores an angle and advances it by `-(v_forward / radius) * dt`, wrapped to 2π. The radius is
  taken from the wheel model's bounds (`transfers_trailer_a_wheel01.cgf` 0.518 m,
  `..._wheel02.cgf` 0.885 m: the largest half extent, the axle is model-space Y). The wheel
  models are held on the `wheelN` / `wheel_type2N` mounts of the body `.chr`, so the spin is
  applied about the holder's local -Z (the model's Y axle after `CRY_TO_BEVY`). The real
  wheel-record radius field (record+0x74) is not in the client data we read; the bounds are the
  documented stand-in. Axis and sign are inferred from the mount matrices and the formula, not
  seen in a capture.
* `vehicle::steer_spindles` ports `FUN_390cffd0` (0x390cffd0) for the body's `Bone_spindle`
  (`transfers_trailer_a_body.chr`). With `R = |v_forward| / |yaw_rate|` it wants
  `sign * mean(atan(front / R), atan(rear / R))`, clamped to
  `SPINDLE_MAX_ANGLE = 0.52359879` rad (30°, `_DAT_399a440c` at 0x399a440c), where front/rear are
  the distances from the spindle bone to the front/rear wheel mounts along model-space Y
  (Farm Wagon: 2.389 m / 1.766 m). Without yaw rate it is 0; at a standstill it falls back to
  `clamp(yaw_rate / limit)` times the maximum (limit is the first float of the model's
  runtime-built vehicle-physics block at `vtable+0x1e0` + 0x34, which cannot be read from the
  unpacked dll, so `SPINDLE_YAW_LIMIT` uses the captured full-lock 0.4 rad/s). The stored angle
  is smoothed by `min(dt, 0.1)` (`DAT_3999ccf0` at 0x3999ccf0) and the bone is rotated by the
  half-angle quaternion about its local Z (up); the sign flips when forward speed and yaw rate
  disagree, i.e. in reverse. `SPINDLE_MAX_ANGLE`, `SPINDLE_LERP` and `SPINDLE_YAW_LIMIT` are in
  `vehicle.rs`.

Unit tests cover the wheel advance, reverse sign, the turn-radius spindle angle and its reverse
sign, the 30° clamp (including the standstill fallback) and the `min(dt, 0.1)` smoothing. The
ignored data test also checks the Farm Wagon spindle arms and both wheel radii.

Pending live checks:

* server handling/echo of type-2 (the capture has no S->C lines): whether the server
  corrects the driver's pose and whether the driver's own echo is applied;
* the send gate's first-send delay and its behaviour when the frame time is below 100 ms;
* wheel angular velocity units/sign for a vehicle whose wheel flag is set;
* the `camera_distance` consumer;
* the wheel spin's visual axis and sign on the Farm Wagon and on a rear-wheel model, and the
  `Bone_spindle` rotation (axis, sign, and whether its skinned axle mesh matches the real
  client) against `shots/drive-12/`;
* remote carts' wheels: whether deriving speed from the played-back position matches the
  captured type-2 `velocity` (the port does not use the wire velocity for visuals);
* whether the leave speed check should read the wire velocity (30/32767 m/s for a type-2
  body, the controller speed for the driven cart) or the engine speed `vtable+0xa4` reads.

### Remote vehicles and ships (S8)

`ingame/vehicle_track.rs` is the 1.2 vehicle controller for other players' carts and ships; a
`RemoteUnit` switches to it on its first type-2 or type-3 movement and keeps the walking
`remote::Track` for every other unit. The receive chain was read in the 1.2 decompile
(`<client12-decompile>/x2game.dll.c`): `FUN_390b2f30` (ships) and `FUN_390c63c0` (land
vehicles) queue a sample per packet (`FUN_390b2980` / `FUN_390b2840`), the playback clock is
kept by the offset of `FUN_390986a0`/`FUN_39098780`, and `FUN_390b2420` -> `FUN_390b0040` ->
`FUN_390ad150`/`FUN_390a16b0` applies the pose. The rules, and what backs them:

* **Interpolation, not extrapolation.** The pose is the position lerp and quaternion nlerp
  (short arc, normalised) of the two samples around the playback time; past the newest sample
  it is held, and a single remaining sample is held at `t = 0` (`FUN_390ad150`). Unlike the
  living-unit mode 3, the vehicle path has no velocity extrapolation.
* **Fixed delay clock, corrected every frame.** The first sample sets
  `offset = (server - local) - delay`; later samples compute
  `deviation = (server - local) - offset`. A negative deviation or one over
  `delay * 20.0` re-bases the offset and erases the queue. Otherwise `FUN_390ad150` calls
  `FUN_39098780` once per frame: inside the dead band the offset moves 1 ms
  (`DAT_399a28a8` = 0.001, mainly the ms-to-seconds factor), outside it by
  `max(1 ms, 0.2 * excess)` (`_DAT_399a2ec0` = 0.2). `_DAT_399a2eb8` = 20.0 and the 0.2/1 ms
  constants were read from the unpacked image.
* **Hold after silence.** `FUN_390b2420` stops updating 5000 ms after the last sample arrived;
  the drawn pose stays where it was. No sample ever keeps the announced (level) pose.
* **20 m snap, checked before sampling.** `FUN_390b0040` compares the drawn entity with the
  controller's model (newest sample) position before it samples; a larger distance
  (`DAT_399a3364` = 400.0 squared) is a teleport and snaps at once, even while the playback
  clock is still behind that sample. **S8-C5** (inferred): the port snaps to the newest sample
  and re-bases the clock; the original resets its smoothing state (`FUN_390ae4a0`), and a
  later capture must record the drawn poses across a >20 m ship move to settle the exact pose.
* **Ship-only rules.** `FUN_390b2f30` drops a packet older than the newest sample by less than
  1000 ms, and restarts the sync when the zone changes. **S8-C3** (inferred): the original
  compares the packet's zone with the controller model's zone; the port compares it with the
  last sample's and clears the queue and re-bases instead of shifting the queued times
  (`FUN_390a0f10`).
* **Full rotation, no terrain snap.** The packet's quaternion is applied to the root transform
  and `facing` is its yaw; `move_units` does not snap a vehicle's z to the terrain (the packet
  already carries the pose, and a ship may be over water or a shore). The first sample seeds
  the track with its own position and rotation (not the announced level/yaw pose), and a pending
  actor admitted later keeps that rotation (`View::admit` uses `drawn()`), so a cart is never
  drawn flat for the playback delay.
* **Deck riders walk in the hull frame.** A movement whose support block names a unit and
  carries a world point has its primary position and relative facing in the hull frame (S10
  deck standing). Each `DeckRider` keeps a walking `Track` in that frame (the same delay and
  rules as an on-foot remote unit), and `place_deck_riders` samples it every frame and composes
  `hull.pos + hull.rotation * local` with the smoothed hull's full quaternion. A walker
  therefore moves continuously between packets (no 0.5-1 m packet steps), turns smoothly, and
  stays on a pitching/moving deck instead of sliding with its old variable-delay world track.
  `place_deck_riders` also writes `Motion` from the local track (speed and heading), so the
  walk clip plays; `move_units` skips `DeckRider` units and `place_deck_riders` runs before
  `drive_locomotion` so a starved world track cannot leave an idle slide. Pending actors sample
  their deck track too, so admission draws the current composed pose. Without a drawn hull the
  wire hull pose is used, else the last drawn pose is kept. On leaving the deck the world track
  is seeded from the last composed pose, so the first on-foot frame does not jump; stepping onto
  another hull seeds the new local track at the drawn point and blends (a large step is not
  snapped).
* **Deck standing uses the same hull rotation.** `DeckHull::pose`/`ship_to_world` carry
  the full quaternion into `board_deck`, `deck_control` and the deck landing
  (`deck_landing`). The shared `DeckHull::pose`, `ship_to_world`, `world_to_ship` and
  `deck_landing` now all carry the full game rotation, so PR #49's `DeckContact`/`DeckGround`
  callers follow pitch and roll: `pick` returns `(root, rotation)` and `ship_to_world` takes
  that rotation. PR #49's `DeckContact.yaw: f32` and `DeckGround::pose` return type become
  `Quat` at merge (its `let (root, yaw)` / `ship_to_world(root, yaw, local)` expressions then
  compile unchanged). A ship-local walk step is rotated
  into a horizontal collision step scaled so its inverse keeps the full ship-local length
  (`deck_world_step`); dropping the rotated z made a pitched deck cos² slower.
* **A local controller ignores its own echo.** `FUN_390c63c0` is a no-op while the unit has a
  local vehicle controller; `receive_moves` therefore does not feed type-2/3 movements of a
  `Driven` unit (the cart this client drives) into the track.
* **Steering/throttle/wheel data are kept** on `Movement` (`VehicleMove` / `ShipMove`) as the
  seams for the S6/S9 rudder, wheel and propeller visuals; the track interpolates
  steering/throttle for its pose but this port has no ship rudder or wheel-speed consumer yet.

`DELAY` is the one value the decompile cannot give: it comes from a runtime getter
(`DAT_39ae4e90`, integer ms capped at 10000 in `NetActorMovementController::PrePhysicsUpdate`).
The only default in the image is **300 ms** when that getter is null (`x2game.dll.c:756212`),
so the port uses **0.3 s (S8-C1)** and keeps the re-base window at 20 × delay = 6 s. The
minimum step and the dead band are **S8-C2**: `FUN_39098780` takes both from its caller
(`DAT_39ae4e94` at runtime) and neither is read; the port uses the 1 ms nudge as the minimum
step and a 10 ms band.

Capture steps for the pending items (two clients, same cell; B drives a Farm Wagon and a
rowboat; A is the hidden original as observer): record every `SCOneUnitMovement` type-2/3 body
with its arrival time, and the drawn A-side entity pose per frame; compare the drawn pose with
`(packet time) - delay` to measure `DAT_39ae4e90` (S8-C1), start A after B and record a long
drive to see how the offset is corrected (S8-C2), drive a ship across a zone border (S8-C3) and
make B teleport or sail more than 20 m between packets (S8-C5); for the deck paths, stand A on
B's hull while it pitches and record A's drawn offset. The example below replays the recorded
S6/S7 cart drive (`tests/fixtures/remote-vehicle/cart_c_left.tsv`: the original client's own
type-2 bodies, which the server rebroadcasts unchanged) through `receive_moves` and `move_units`
twice, pending and rendered: 44 samples, drawn path 13.80 m, no stall or jump, final pose within
5 cm of the recording, and a cut stream (stops while moving) held at its last packet. The delay
check uses the decompile's 300 ms literal, not the code constant. It fails non-zero when the
interpolation, the delay, the clock offset, the hold, the rotation, the first-sample seed or the
rendered path is broken:
`cargo run -p aa-client --example remote_vehicle_motion -- --headless`.

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
* `AA_FAKE_MOUNT=1`: the remote rider check above.
* `AA_UNIT_SHADOW_RADIUS`, `AA_ANIM_LOD`: shadow radius and animation levels of detail of units (performance pass).

## Gaps

* `SCBuffUpdated` is decoded (`crates/aa-net/src/game.rs`, layout in `docs/aa-net.md`), but the AAEmu server never sends it, so live stack/charge updates do not arrive; a buff refresh comes as `SCBuffCreated` with the same index.
* Combat idle is for creatures only; humanoid NPCs fight with the combat agent's clips.
* Mount and rider placement is unverified against the real client; mount skills cast only from the driver's seat
  (live capture 2026-10-28: any slot on foot is a local `0x2c` refusal, no `pet_manual` mode), and there is no mount
  dismount animation.

* The 2026-10-28 capture sampled the seat height (left get-on: rider z 100.25 over mount origin z 98.11); the get-off clips are played but the dismount controller's placement is not ported (S2), and there are no mount skills.
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

## S9: steering a ship (2026-10-29)

`ingame/ship.rs` drives the ship whose `LocalSeat` has `ship` set (a `UnitKind::Slave` whose
`models.sub_type` is `ShipModel`). The server owns the hull: the client sends only CSMoveUnit type 4
and never moves the hull locally. `mount::LocalMount::ship_driver()` is the point-1 gate; a ship is no
longer classified as a cart, so `cart::drive_cart` leaves it alone, and `player::control_player`
stops on-foot movement at the helm. `mount::drive_mount` copies the hull's displayed pose (or its
`Units` pose when the hull is not drawn) into the driver's `Mover` each frame, with zero velocity and
grounded, so the camera follows the sailing hull; the hull's own transform is still placed only by
its type-3 echo (the hull has no `Driven` marker). Leaving queues the final 0/0 just before the
`CSDiscardSlave` request; the existing `SCUnitDetached` path then clears the seat and stops the pump
(see the pump bullet).

* **Controller (`FUN_3909d8c0` / `FUN_3909da60`).** W/S set throttle immediately to +127 / -127;
  A/D ramp steering at the ship model's `ship_models.steer_vel` (axis/s) with the float clamped
  to -1..=1, released steering decays at the same rate. The wire signs are A -127, D +127.
  `Db::ship_steer_vel` maps the slave's `models` row to `ship_models` (`m.sub_type =
  'ShipModel'`, `s.id = m.sub_id`); the rowboat's smallboat is 6.0, the galleon 1.0, and a model
  without a ship row falls back to 6.0 (`STEER_RATE`). The 2026-10-29 rowboat capture's
  intermediate levels (D 65, A -98, release 62/94 then 0) are frame samples of that ramp; the
  fitted rate equals the rowboat's row, with the game-option factor the decompile multiplies in
  folded into the fit (the factor itself was not captured).
* **Send pump.** The cart's 100 ms `Pump` (`FUN_394045f0`) is reused, with no change test, so a still
  helm resends 0/0 (capture: 87 idle requests, all 0/0, median 127 ms wall spacing at the hidden
  client's frame rate). At the leave the capture's last type-4 is one 0/0 on the same wall tick,
  logged just before `CSDiscardSlave`, with no type-4 after it. The HUD slave leave
  (`handle_requests`) writes that final 0/0 immediately before the unbind request; `ShipDrive`
  then sends nothing until `SCUnitDetached` clears the seat (and nothing after it). A teleport's
  loading screen skips `drive_ship`, so `reset_on_teleport` clears the ramp and pump meanwhile.
* **Type 3 receive.** `aa-net` now keeps the type-3 steering, throttle, zone and stuck fields and the
  30/32767 m/s velocity scale. The hull continues through the normal vehicle interpolation
  (`remote::Track`); this slice does not add a ship-specific predictor (S8 may refine it).
* **Visuals.** The rowboat prefab (`prefabs/smallboat.xml`, smallboat model 129) has only the
  `$helms`, `$driver`, `$passenger0`, `$ladder_left/right`, `$aimpoint`, `$heal_point0` and
  `$lamp_rear` helper nodes plus the static `smallboat_helms.cgf` brush. It has **no** rudder part and
  no steering-wheel/helm bone: `FUN_390c7410` needs a child model named `rudder`, and
  `FUN_390d2990` returns early for `smallboat` / `soloship` / `coupleduckship`. No visual change is
  therefore possible or native for this hull; sailboat/clipper rendering is still blocked by the
  server-hull placement issue (deck standing is S10).

Pending live checks: the uncaptured game-option factor (folded into the rowboat's fitted
`steer_vel`), the ramp on a sailboat or clipper (galleon row 1.0), and a second
character's view of the type-3 hull (interpolation and velocity scale).

## S10: standing and walking on a ship deck (2026-10-29)

`ingame/ship/deck.rs` puts the local player on the hull floor of any
`UnitKind::Slave` whose `models.sub_type` is `ShipModel`. The floor is the
salvaged older own-ship work (`test/ships` deck floor): `ship/floor.rs` takes
the upward-facing triangles of the hull brush (`*_body.cgf`) of the model's
`ship_models.normal` prefab, in the ship frame (cry: x right, y forward, z up).
Boarding is automatic while the player stands within `DECK_STEP` (1.5 m) of
that floor; a dead (the `Dead` component or, as on foot, the stats' death
flag that arrives first), skill-moved, seated or mounted player is left to
those paths, and mounting drops an existing ride.

* **Range gate and preload.** `board_deck` first transforms the feet into the
  ship frame and skips a ship whose point is outside the cached hull floor
  bounds plus 2 m, so it does not ask for the floor of every ship in view.
  Unit preparation (`prepare::build_unit`) warms both the floor and its bounds
  on the worker thread; `board_deck` only reads the caches.
* **Ship frame.** `ship_to_world(root, rotation, local)` is
  world = ship + rotation * local, with local in the captured (right, forward,
  up) frame and `rotation` the drawn hull's full game-space quaternion
  (`DeckHull::pose`; S8 adds the server's bank and wave pitch);
  `world_to_ship` is its inverse. PR #49's `DeckContact.yaw: f32` /
  `DeckGround::pose` need the `Quat` at merge; its
  `ship_to_world(root, yaw, local)` call then follows pitch and roll. Each frame the player follows
  the authoritative type-3 hull, so a moving or pitching hull carries the
  same deck point without the client simulating the ship. Synthetic-deck tests
  (no pak) board a hull pitched 4 degrees with the player 4 m forward, land a
  get-off on it, and walk off it; the stander and the exit point follow
  `rotation * local`, not the yaw-only point. `deck_world_step` keeps the full
  ship-local step length on a pitched/rolled deck.
* **Input.** The deck reuses the on-foot rules (`resolve_keys`, `face_camera`,
  the mouse look): W / S and Q / E move in the character's frame, the first
  direction snaps the facing to the camera, the right button keeps it there,
  and A / D turn the character and the camera together, moving or not. `.`,
  Shift, auto-run and the Dash/buff multiplier use the same `player::
  ground_velocity` / `walking_mode` as `control_player`. `PlayerInput::moving`
  (what cancels a cast) is the step actually taken in the ship frame, on the
  floor and in a deck jump, as on foot; the packet's moving flag stays the
  captured grounded one.
* **Packet (rowboat capture, 2026-10-29).** Header flags 0x40
  (`move_flags::STANDING_ON_OBJECT`, the captured walking and standing value; only
  the jumps are 0x44), actor flags 0x20 plus the ground bits (0x24 walking, 0x34 on a deck
  jump), `gcFlags` 1, `gcPartId` 1, `gcId` 8911, primary `pos` = ship-local
  (right, forward, up) and the auxiliary position = world. The primary
  direction is the player's facing relative to the hull and the auxiliary `z`
  is the player's world facing, both truncated direction bytes (`d_leave`: -63
  and -35; `a_idle`: 0 and 32).
* **Jump.** A jump keeps `DeckRide`: the feet rise and fall in the ship frame
  with the actor's jump speed and gravity, and every packet keeps the support
  block (the captured 0x44 / 0x34 body). Landing returns to the hull floor.
* **Leaving.** Stepping or jumping off the floor removes `DeckRide`: the feet
  become a world point and the fall starts with the step's world velocity plus
  the ship's type-3 velocity. An airborne step blocked by a higher floor (a
  riser, hatch or rail brush) keeps the ride and the vertical motion instead.
  The deck controller takes the exit frame's step (and a jump-off's air step)
  once and `SendState::hand_over_from_deck` makes `player::control_player`
  skip that frame; from the next frame it continues the fall (the ship's speed
  carried from its first frame, also after a walk-off that starts grounded at
  the rail), and its first packet, sent at once, is an ordinary on-foot movement
  without 0x40/0x20 (the captured transition). A teleport also removes the
  component.
* **Remote deck riders (S8).** Another player standing on a deck reports the
  ship-local point and relative facing in the primary fields and the world
  point in the support block; their own walking track is a variable delay (and
  extrapolates), so drawing the world point slides them 1-2 m against the
  smoothed hull. A `DeckRider` keeps a walking `Track` in the hull frame
  (position and relative facing, same delay/rules), sampled every frame and
  composed as `hull.pos + hull.rotation * local` with the drawn hull's full
  quaternion (`place_deck_riders`); without a drawn hull the wire pose is used,
  else the last drawn pose. Leaving the deck seeds the world track from the
  composed pose. Tests drive the real `receive_moves`/`place_deck_riders` with
  a walking rider on a moving pitched hull (no per-packet steps, smooth turn, `Motion` speed
  and heading), a pending rider before admission, a leave frame that does not jump, and a
  switch between two hulls that blends from the drawn point.
* **Transport.** `aa-net` now writes the 0x20 support block from `MoveSupport`
  instead of masking it; `MoveCommand::support` carries the block. The six
  captured bodies (idle, two walking, jump, header-only 0x40, and an off-deck
  packet) are in `tests/fixtures/deck-standing/s910-native.tsv` and re-encode
  byte for byte after zeroing the time.

Pending live checks: a sailboat/clipper deck (only the rowboat was walkable in
the capture), whether walking packets (the fixture's `walk-support` rows are
standing rows) keep header 0x40 on another hull, and whether the support
block's velocity/rotation follow the hull when the ship turns.
