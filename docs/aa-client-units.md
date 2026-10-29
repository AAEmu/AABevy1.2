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
  plays the rider clips of the mount's kind: `models.mount_pose_id` 2 lion, 4 wildboar, 10 pangolin give `lion_ba_relaxed_idle`,
  `lion_mo_normal_walk_f`, `lion_mo_normal_run_f` (and the other prefixes); every other pose (1 horse, and 0, which most
  mountable models have: donkeys, elks, yatas) takes the `horse_` set. The gait is picked from the mount's speed (idle below
  0.3 m/s, walk below 3 m/s, run above) and the tempo copied from the mount's animator;
* the seat is the bone's place on the mount's last drawn pose carried to where the mount stands in this frame, so a rider does
  not trail a moving mount;
* the rider's on-foot `Locomotion` is stashed and given back on `SCUnitDetached`.

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
  squared distance 625), two seconds since the last ride request. `MountMate` sends `CSMountMate` (driver point 1, reason 1:
  the native client picks 1..4 from how the player comes up to the mount, for its mounting animation, which is not played
  here); `UnMountMate` sends `CSUnMountMate` from the seat the player is on with reason 9; `DismissPet` while riding sends
  `CSUnMountMate` reason 10 and then `CSRemoveMate`. `KickPassenger` stays a stub (the lane only lets a character leave its own
  seat).
* **Riding** (`mount::track_local_mount`, `LocalMount`): `SCUnitAttached` for the own object id seats the local player like a
  remote rider (same `Rider`, same clips) and puts its `Mover` where the mount is, facing its way; `SCUnitDetached` lets it walk
  on from there (the server puts the character where the mate stood). The camera's pivot rises with the seat (its height less 0.85 m, the seated rider's lower head).
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
  speed is the latest replicated `SCOneUnitMovement` velocity (`Movement.velocity`, 1/1024 m/s),
  treated as 0 after 500 ms without a new report (a port bound, **not a 1.2 value**); the native
  getter `vtable+0xa4` is an engine speed, so this source is **unverified**.
* **Passenger walk-off.** With the own attachment point above 1 on a slave, movement input
  (`moveforward`, `moveback`, `moveleft`, `moveright`, `turnleft`, `turnright`, `autorun`, and
  the world mouse buttons through the same `resolve_keys` mapping `control_player` uses: both
  buttons run forward, one button makes A / D strafe) sends the parent's 0x032 through
  `FUN_39362170`, throttled to one request per 1500 ms (`_DAT_3a169948`). That native path has no
  speed check, unlike the driver's `FUN_39345aa0`. Like `talk_input` it is skipped while the HUD
  is suspended or loading, while a widget has the HUD focus, and while the UI owns the keyboard
  (typing in a field); mouse buttons held over a HUD widget are ignored like they are on foot.
* **Not ported here:** cart driving/movement, wheel visuals, ships, and the despawn/destroy
  lifecycle (0x02f/0x030). The player flags at +0x3a70..+0x3a73/+0x3a7b (native 0x13) and the
  parent unit type 0xe (native silent refusal) are also not ported: their meaning is unverified.
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
* Owner and a second player, dead, combat and trade-pack cases; the capture plan in
  `handoff/2026-10-27/research/s5-cart-boarding.md` has the exact packet list.

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
