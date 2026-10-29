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
  the get-off state and then lets it walk on from there (the server puts the character where the mate stood). Attaching
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
* Getting thrown off is the server's: the owner's death despawns the mate (its riders get `SCUnitDetached` first), a stun,
  sleep or root and the skills marked `unmount` detach the rider, a blink unmounts it. The client also lets go of a mount that
  is no longer in view; a new spawn (`PlayerSpawn`, a world switch) or `Resurrected` clears the attachment, and leaving the
  game resets `LocalMount`.

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
  ~1300 ms; 100 ms samples). The client keeps its get-off chain and the server places the character (slice S2
  owns the placement).
* Context only, mounted ground speeds measured from standstill: walk 1.25 m/s mean (max 2.61), run 1.42 m/s mean
  (max 2.50); the short run leg only reaches the horse's run gait at its end.

### Pending live checks

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
