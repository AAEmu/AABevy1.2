# aa-net: network client for the AAEmu 1.2 server

Client side of the ArcheAge 1.2 (r208022) protocol, ported from the headless bot in the server repo
(`aaemu-testclient`, `aaemu-commons`, `aaemu-game`/`aaemu-login` packet definitions). It takes the
client from the login screen to standing in the world: authenticate, world list, enter a world,
character list, create / delete, select, spawn, units and doodads around, movement, chat, keep-alive
and logout.

It does not depend on any server crate (no MySQL / sqlx); the opcodes and layouts it uses are copied
into `src/opcodes.rs`, `src/login.rs`, `src/game.rs`, `src/stream.rs` and `src/model.rs`.

## Run it

An AAEmu 1.2 server must be up. The examples and live tests log in at `$AA_LOGIN` (default AAEmu's
`127.0.0.1:1237`; the game and stream servers come from the login server's world list).

```bash
cargo run -p aa-net --example walkthrough                       # full flow, ~25 s
cargo run -p aa-net --example walkthrough -- --roundtrip        # + back to character select / world select / in again
cargo run -p aa-net --example walkthrough -- --delete-test      # + create, delete and list a throw-away character
cargo run -p aa-net --example walkthrough -- --move-secs 120    # + walk a circle for 2 minutes at 10 Hz (keep-alive soak test)
cargo run -p aa-net --features bevy --example bevy_walkthrough  # same through NetPlugin in a headless Bevy app
cargo test -p aa-net                                            # unit + scripted-server tests; live lane check ignored
RUST_LOG=aa_net=debug cargo run -p aa-net --example walkthrough # protocol trace (trace = every game packet)
```

Options of `walkthrough`: `--login HOST:PORT`, `--user NAME` (default `bevytest`), `--password-file
PATH`, `--world ID`, `--character NAME`, `--watch-secs N`, `--roundtrip`, `--delete-test`,
`--move-secs N` / `--move-hz 10` / `--scenario circle|far|underground|jump|fall` (movement soak test),
`--no-ping`, `--ping-interval-ms N`, `--first-ping-delay-ms N` (keep-alive experiments, see
"Keep-alive and the server's dead connection sweep"). HUD data: the walkthrough prints the whole player
state (stats, buffs, abilities, action bar with potion counts, bag, gear, zone, clock, chat channels,
target; `print_player_state`). `--cmd TEXT` (repeatable) sends chat / GM commands such as
`/teleport cinderstone`, `--target-nearest` selects the nearest unit, `--skill ID` /
`--skill-target self|nearest|hostile|OBJ` / `--item TEMPLATE:SKILL` cast skills and use items, and
`--resurrect in-place|shrine` stands the character up; each prints the events that follow. Use a throw-away account (`--user NAME
--character NAME`, created on first use) when another client is signed in as `bevytest`: the server
refuses a second game connection for an account that is already online ("enter world denied (1)").

**Accounts.** The login server runs with `AutoAccount`, so an unknown account is created on its first
login with whatever password is sent. The example uses the account `bevytest` (never `clienttest`,
that is the real client's) and reads the password from `--password-file`, `$AA_PASSWORD_FILE`, else
`<server-cfg>\client-test-password.txt`, else the user name. It is never printed or logged
(`Secret` redacts its `Debug`).

## Flow

```text
login server (27237)                          game server (27239)               stream (27250)
CARequestAuthTrion  ─► ACJoinResponse, ACAuthResponse  (account id, max characters)
CAListWorld         ─► ACWorldList  (worlds, congestion, race congestion, account's characters)
CAEnterWorld(id)    ─► ACWorldQueue* ─► ACWorldCookie (cookie, game server address)
                                              X2EnterWorld(account, cookie) ─► X2EnterWorldResponse (stream token+port)
                                                                                ChangeState(0)
                                              FinishState(n) ◄► ChangeState(n+1)   n = 0..6
                                              FinishState(7) ─► SCUpdatePremiumPoint   = lobby ready
                                                                                      CTJoin(account, token) ─► TCJoinResponse
                                              CSListCharacter ─► SCGetSlotCount, SCAccountInfo, SCRaceCongestion, SCCharacterList(last=true)
                                              CSCreateCharacter ─► SCCreateCharacterResponse | SCCharacterCreationFailed
                                              CSSelectCharacter ─► SCCharacterState (+ inventory, quests ... ignored)
                                              CSSpawnCharacter  ─► SCUnitState (own unit: obj id, position, model, equipment)
                                              CSNotifyInGame, CSNotifyInGameCompleted
                                                 ─► SCUnitState* / SCDoodad(s)Created / SCOneUnitMovement* / SCChatMessage ...
                                              CSMoveUnit, CSSendChatMessage, PingPacket every 3 s (the server drops silent clients after 30 s)
                                              CSLeaveWorld(0) ─► SCPrepareLeaveWorld(10 s) ─► SCLeaveWorldGranted
```

* **Framing.** `[u16 length][payload]`, little endian. Login / stream payload: `[u16 opcode][body]`.
  Game payload: `[0xDD][level]…`; level 1 = `[hash][count][u16 opcode][body]` (the server verifies neither
  hash nor count; the client sends 0 and a counter), level 2 = the CryNetwork proxy channel
  `[u16 opcode][body]`, level 4 = a compressed batch of skill results (`[u16 count][deflate
  stream of [u16 0][u16 opcode][body]]`; stored, fixed Huffman and dynamic Huffman blocks are
  unpacked into `ServerMsg::Batch`). 1.2 traffic is not encrypted.
* **Login = Trion ticket.** The simplest flow the server accepts, and what the launcher does: an XML
  ticket with `<username>` and `<password>` = hex SHA-256 of the password in `CARequestAuthTrion`
  (`login::trion_ticket`). The Korean challenge / OTP flows are not implemented.
* **CryNetwork handshake.** The server drives it with `ChangeState(n)`; the client answers
  `FinishState(n)`. After state 7 the server sends `SCUpdatePremiumPoint`: the lobby is ready. The
  same handshake runs again after leaving to character select. The session does this by itself.
* **Return paths.** `CSLeaveWorld(1)` goes back to the character list on the same connection;
  `CSLeaveWorld(2)` from the lobby makes the server send `SCReconnectAuth(token)`, the client then logs
  in again on the login server with `CARequestReconnect` and gets the world list (`Command::ReturnToWorldSelect`).
* **Positions** are the 9 byte packed form (24-bit fixed point x / y with sign bits, 22-bit z) in
  packets and `i64 << 32` fixed point in the character list. Both decode to meters (`wire::Pos`;
  x east, y north, z up). Facing is radians, 0 = +y, positive turns left; the movement direction byte
  is `2π/128` per step, the unit-state rotation bytes `2π/127`.

## Messages supported

| Direction | Messages |
|---|---|
| client → login | `CARequestAuthTrion`, `CAListWorld`, `CAEnterWorld`, `CACancelEnterWorld`, `CARequestReconnect` |
| login → client | `ACJoinResponse`, `ACAuthResponse`, `ACWorldList`, `ACWorldQueue`, `ACWorldCookie`, `ACEnterWorldDenied`, `ACLoginDenied`, `ACAccountWarned` |
| client → game | `CSInteractNPC`, `CSInteractNPCEnd`, `CSStartInteraction`, `CSStartQuestContext`, `CSCompleteQuestContext`, `CSDropQuestContext`, `CSQuestTalkMade` (see "Quests and NPC dialogs"), `CSLootOpenBag`, `CSLootItem`, `CSLootCloseBag`, `CSLootDice`, `X2EnterWorld`, `FinishState`, `Ping`, `CSListCharacter`, `CSCreateCharacter`, `CSDeleteCharacter`, `CSCancelCharacterDelete`, `CSSelectCharacter`, `CSSpawnCharacter`, `CSNotifyInGame`, `CSNotifyInGameCompleted`, `CSMoveUnit`, `CSSendChatMessage`, `CSExpressEmotion`, `CSChangeTarget`, `CSUpdateActionSlot`, `CSSwapItems`, `CSSplitBagItem`, `CSDestroyItem` (see "Moving items"), `CSStartSkill` (skills and items), `CSStopCasting`, `CSResurrectCharacter`, `CSTeleportEnded`, `CSLeaveWorld`, `CSCancelLeaveWorld`, `CSChangeAppellation` (see "Titles"), `CSConstructHouseTax`, `CSCreateHouse`, `CSDecorateHouse`, `CSChangeHouseName`, `CSChangeHousePermission`, `CSRequestHouseTax`, `CSAllowHousingRecover`, `CSSellHouse`, `CSSellHouseCancel`, `CSBuyHouse`, `CSCreateDoodad`, `CSRequestCommonFarmList` (see "Housing and public farms") |
| game → client | `X2EnterWorldResponse`, `ChangeState`, `Pong`, `SCReconnectAuth`, `SCGetSlotCount`, `SCAccountInfo`, `SCUpdatePremiumPoint`, `SCRaceCongestion`, `SCCharacterList`, `SCCreateCharacterResponse`, `SCCharacterCreationFailed`, `SCDeleteCharacterResponse`, `SCCancelCharacterDeleteResponse`, `SCCharacterDeleted`, `SCCharacterState`, `SCUnitState` (all unit kinds), `SCUnitsRemoved`, `SCOneUnitMovement`, `SCUnitPoints`, `SCUnitDeath`, `SCTeleportUnit`, `SCDoodadCreated/sCreated`, `SCDoodadRemoved/sRemoved`, `SCDoodadPhaseChanged`, `SCChatMessage`, `SCEmotionExpressed`, `SCChatBubble`, `SCAreaChatBubble`, `SCErrorMsg`, `SCKicked`, `SCPrepareLeaveWorld`, `SCLeaveWorldGranted`, `SCLeaveWorldCanceled`; for the HUD (see "In-game HUD data"): `SCExpChanged`, `SCAbilityExpChanged`, `SCRecoverableExp`, `SCLevelChanged`, `SCAbilitySwapped`, `SCCharacterLaborPowerChanged`, `SCCharacterGamePoints`, `SCGamePointChanged`, `SCBmPoint`, `SCCombatEngaged/Cleared`, `SCUnitFactionChanged`, `SCCharacterResurrected`, `SCBuffCreated/Removed/Learned`, `SCActionSlots`, `SCCharacterInvenInit/Contents`, `SCItemTaskSuccess`, `SCUnitEquipmentsChanged`, `SCInvenExpanded`, `SCTimeOfDay`, `SCDetailedTimeOfDay`, `SCOnOffSnow`, `SCConflictZoneState`, `SCJoinedChatChannel`, `SCLeavedChatChannel`, `SCTargetChanged`, `SCSkillStarted/Fired/Ended/Stopped`, `SCCastingStopped`, `SCLootableState`, `SCUnitLootingState`, `SCLootBagData`, `SCLootItemTook`, `SCLootItemFailed`, `SCLootDice/Notify/Summary`, `SCCooldowns`, `SCSkillCooldownReset`, `SCSkillLearned/Upgraded`, `SCSkillsReset`, `SCUnitDamaged`, `SCUnitHealed`, `SCCombatText`, `SCPlotEvent/Ended/CastingStopped/ChannelingStopped`, `SCQuests`, `SCCompletedQuests`, `SCQuestContextStarted/Updated/Completed/Reset`, `SCNpcInteractionSkillList`, `SCDoodadQuestAccept`, `SCQuestRewardedByMail`, `SCNpcChatMessage`, `SCAppellations`, `SCAppellationGained`, `SCAppellationChanged` (see "Titles"), `SCHouseState`, `SCHouseBuildProgress`, `SCHouseBuildPayChanged`, `SCHousePermissionChanged`, `SCHouseDemolished`, `SCMyHouse`, `SCMyHouseRemoved`, `SCHouseFarm`, `SCHouseTaxInfo`, `SCConstructHouseTax`, `SCHousingRecoverToggle`, `SCHouseSetForSale`, `SCHouseResetForSale`, `SCHouseSold`, `SCHouseOwnerNameChanged`, `SCLoginCharInfoHouse`, `SCResponseCommonFarmList`, `SCShowCommonFarm`, `SCIsUnitInFarm` (see "Housing and public farms"), `CompressedGamePackets` |
| client ↔ stream | `CTJoin`, `TCJoinResponse` (joined and kept open, other traffic ignored) |

Everything else the server sends is decoded as `game::ServerMsg::Unhandled { level, opcode, len }`
and skipped: frames are length delimited, so an unknown or undecodable packet never desynchronises
the stream (inside a `DD04` batch see "Compressed packet batches"). The packets the lane can still
send undecoded are listed in "Small server messages" at the end, with who owns them.

## Vehicles (CSMoveUnit types)

`CSMoveUnit` (0x089) is shared by every moving object; the first body byte is the move type
(`FUN_397c0470`, types 0-5):

| type | owner | body after the type byte |
|---|---|---|
| 1 | player / NPC / mate | time, flags, 9-byte position, velocity `i16×3` (1/1024 m/s), direction / tilt, delta movement, stance, alertness, actor flags (+ fall velocity) |
| 2 | land vehicle (cart, car, tractor) | time, flags, position, velocity `i16×3` (1.2 client scale `30/32767`, i.e. m/s × 1092), rotation `i16×3` (the scalar is rebuilt), angular velocity `f32×3`, steering `f32`, wheel count `u8`, wheel angular velocities `f32 × count` |
| 3 | ship | time, flags, position, velocity, rotation `i16×3`, angular velocity `f32×3`, steering `i8`, throttle `i8`, zone `u16`, stuck `u8` |
| 4 | ship request | time, flags, throttle `i8`, steering `i8` (no position) |
| 5 | transfer | time, flags, position, velocity (50/32767), rotation `i16×3`, angular velocity, steering `i32`, path point `i32`, speed `f32`, reverse `bool` |

A gliding character sends the same type-1 body: stance 1, actor flags 0 while flying; move flags 2 with delta 0x7f while W
is held (2026-10-31 recapture rows 5137-5287, 5124), 0 otherwise. At contact the glider sends `CSRemoveBuff` (buff index,
reason 9) first, then the captured touchdown packet: stance 1, flags 6 (Moving|Stopping), actor flags 0x14
(Jumping|StandingOnSolid); later grounded packets fall back to flags 4 / actor 0x04. Water sends stance 2 / flags 0 /
actor 0 before the remove. The wire override keeps the IN_COMBAT bit.
`model::tests::gliding_type_1_bodies_match_the_server_unit_move_reader` pins the flight and landing bodies against the
server's `UnitMove.Read` field walk (`aaemu-game/src/models/movement.rs:418-438`).

The type-2 and type-4 codecs are the 1.2 serializer `FUN_397c0470` (case 2 and case 4) in
`aa_net::vehicle`; `FUN_3961e2c0` is the 9-byte position codec, `FUN_397ad8c0` the 30/32767
velocity codec and `FUN_397ae020` the quaternion writer. The goldens under
`crates/aa-net/src/vehicle/goldens/` were recorded from the running 1.2 client and are byte /
bits exact; they do not establish the runtime scales below.

* `Command::MoveVehicle(VehicleMoveRequest)` writes `CSMoveUnit` type 2 for the controlled object.
  `VehicleMoveRequest::from_parts` takes metres, a quaternion `[x, y, z, w]`, m/s, rad/s,
  steering and wheel angular velocities; it checks the BC3 id, type 2, at most 18 wheels and
  finite values. The driver admission, input, send pump and kinematic controller live in
  `aa-client` (`ingame/cart.rs`): it writes the cart's type-2 body on a 100 ms gate checked
  once per frame (also when still) and seeds the time in the session.
* `ShipControlRequest` (type 4) writes through the same serializer (`MoveUnit::write_to`), so the
  type-4 layout exists once.
* Received type 2 is decoded into `model::Movement::vehicle`
  (`VehicleMove { rotation, angular_velocity, steering, wheel_ang_vel }`) and type 3 into
  `model::Movement::ship` (`ShipMove { rotation, angular_velocity, steering, throttle, zone_id,
  stuck }`). Both keep the three quaternion shorts; `Movement::rotation_quat()` rebuilds
  `[x, y, z, w]` as the 1.2 receiver does (`FUN_397c0470` cases 2 and 3) and `vehicle_yaw` keeps
  the yaw derived from it. The full rotation is what S8 draws (a ship's bank/pitch, a cart's
  tilt); the unit tests mirror the server's `ShipMove::write` / `VehicleMove::write` bodies byte
  for byte.

### Live checks

The 2026-10-29 drive of a Farm Wagon (slave 60, model 1008) confirmed, over 857 type-2
bodies:

- the pump's 100 ms gate is checked once per frame and has no change test: a still cart is
  resent, 36 bodies in 5 s at the hidden client's 22.5 FPS (143 ms median, once per 3 frames);
- the velocity scale is the 1.2 client's `30/32767` = m/s × 1092 (1/2048 would be 1.86× off);
- wheel count 0: the Farm Wagon sends no wheel angular velocities (the body ends after the
  steering float);
- steering is the key input through a frame-quantised ramp (0.484 two frames in, then 1.0;
  0.269/-0.752 on release) and `angVel.z = 0.4 * steering` rad/s; flags are 0 on every body,
  with no `scType`/`phase` block (bit 0 was only seen on on-foot bodies);
- the position is the cart's world position, the velocity is world-space 3D, and the
  (x,y,z)-short quaternion's yaw matches the client's rendered yaw.

Still pending:

- the server's handling and echo of type-2 bodies (the capture has no S->C lines): whether it
  corrects the driver's pose, and whether the driver's own broadcast is applied;
- wheel angular velocity units/sign for a vehicle whose wheel flag is set;
- the `camera_distance` consumer and the `Bone_spindle`/wheel update formulas.

## Compressed packet batches

`DD04` carries a raw RFC 1951 DEFLATE stream without a zlib/gzip header or checksum.
The lane writer in `systems/skills/packets.rs:874` uses stored blocks; its C# source
contract uses `DeflateStream(Optimal)`. The decoder accepts stored, fixed-Huffman,
dynamic-Huffman and mixed streams through `flate2`, then uses the same ordered game
packet parser as before. No new application command or event is required.

Inflation proceeds in 16 KiB chunks with a 16 MiB output budget, exposed as
`game::MAX_INFLATED_BATCH_BYTES`. The budget is an application memory limit, **not** a
protocol length or a verified native client limit; the native budget remains unknown.
Exactly 16 MiB is accepted. Oversized output, malformed trees/blocks, bad stored lengths,
missing final blocks, truncation, trailing bytes and concatenated streams return
`WireError::Invalid` before **any** game packet from that frame is emitted. A stream
must finish and consume every input byte. RFC-permitted unused bits in its final byte
are not an additional byte stream. Empty final streams are accepted.

The session logs a wire error and continues with the next length-delimited TCP frame.
The old `ServerMsg::Compressed` skip variant and stored-only `game::inflate_stored`
helper are removed. DD04 bodies have no individual lengths: the `u16` before each opcode is
always 0 (the lane's `CompressedGamePackets::raw_payload` and the C# it ports), not a size. A
packet that cannot be decoded (unknown opcode: `Unhandled` with `len` = the skipped body; bad
body: `DecodeFailed`) is skipped by resynchronising (`ServerMsg::batch_resync`): the batch resumes
at the one later `[00 00][opcode]` from which exactly the header's remaining count of packets
decodes and ends the stream. Two such starts (a body can contain bytes that look like a packet)
are not guessed between, session-control packets (enter world, reconnect, character list or state,
kick, leave) never count in a resynchronised tail, at most 512 candidate starts are tried, and
without a unique start the batch ends there as before (`warn` with the number of lost packets).

`crates/aa-net/tests/fixtures/deflate/generate.py` independently produces the checked-in
goldens with Python zlib 1.2.13 (`wbits=-15`), validates them using zlib, and records
compressed/raw lengths and SHA-256 in `metadata.json`. Fixtures include fixed, dynamic,
mixed stored+dynamic, empty, unknown/truncated game bodies, exactly-at-budget output
and output one byte over budget; no fixture is made with the Rust decoder's library.
Regenerate offline with `python crates/aa-net/tests/fixtures/deflate/generate.py`.
Native/C# captured compressed-packet QA remains pending; no lane connection is needed
for these checks or made while the lane is stalled.

Validation: six decoder tests plus two session tests cover all fixtures, every truncation,
exact stream termination, the output boundary, packet order through the real session and
WorldState, and recovery on the next frame after a compression error. Full
`cargo test -p aa-net` passed 177 library and 2 scripted-session tests, with 9 live tests
ignored. All six compression tests also passed with `--features flate2/zlib-rs`, covering
the alternate backend selected when Bevy features are unified.
`cargo check -p aa-net --features bevy --examples` also passed.

## API

### Protocol layer (plain Rust, no I/O)

| Module | Contents |
|---|---|
| `wire` | `Reader` / `Writer` (little endian, `bc` 24-bit ids, `u16`-prefixed strings and blobs, positions), `Pos` |
| `opcodes` | `ca`, `ac`, `cs`, `sc`, `pp`, `ct`, `tc` constants with `name(op)` |
| `login` | `ClientMsg::{auth, ListWorlds, EnterWorld, ...}` / `ServerMsg`, `World`, `WorldList`, `WorldQueue`, `WorldCookie`, `AccountInfo`, `trion_ticket`, `Secret` |
| `game` | `ClientMsg` / `ServerMsg` (`encode(counter)` / `decode(frame)`), `LeaveTarget`, `VisualOptions` |
| `stream` | `ClientMsg::Join` / `ServerMsg` |
| `model` | `Character`, `CharacterState`, `CreateCharacter`, `ModelParams` + `FaceModel` (full customization), `Equipment` / `Item` (28 slots, `model::slot`), `Unit` / `UnitKind`, `Movement`, `VehicleMove`, `Doodad`, `ChatMessage`, `race`, `gender`, `chat_type`, `stance` |
| `vehicle` | 1.2 move codecs: `FUN_3961e2c0` position, `FUN_397ad8c0` velocity, `FUN_397ae020` quaternion, `FUN_397c0470` `MoveUnit` types 0-5, `VehicleMoveRequest` (CS089 type 2) |

### Transport

`net::{LoginConn, GameConn, StreamConn}`: `connect(addr)`, `send(&msg)`, `recv()` (cancel safe) for code
that wants to drive the protocol step by step.

### Session: `Client`

```rust
let mut client = aa_net::Client::spawn_on(&tokio::runtime::Handle::current()); // or Client::spawn_thread()
client.send(Command::Login { addr, user, password: "...".into() });
while let Some(event) = client.recv().await { /* or client.try_recv() from a game loop */ }
```

`Client::spawn_thread()` runs the session on its own thread (what the Bevy plugin uses), so the app
never blocks. `Client` is the only handle; dropping it closes the connections. `spawn_on_with` /
`spawn_thread_with(ClientOptions)` take the keep-alive tunables (defaults are right for apps).

`Move(MoveCommand)` sends `CSMoveUnit` for the local player (ignored until `InWorld`). `MoveCommand`
is a plain struct of public fields with a `Default`; build it with `MoveCommand::new(pos, yaw)`,
`MoveCommand::running(pos, yaw, speed_mps)` (velocity along the facing + full forward input),
`MoveCommand::stopped(pos, yaw)`, `.with_state(..)`, `.landing(fall_speed_mps)` or struct update
syntax:

| field | wire | notes |
|---|---|---|
| `pos`, `yaw` | packed position, direction byte | yaw 0 faces +y, positive turns left |
| `velocity: [i16; 3]` | velocity, 1/1024 m/s, world axes | |
| `delta_movement: [i8; 3]` | delta movement | pressed direction `[right, forward, up]`, -127..=127; forward is `[0, 127, 0]` as the server's own movers send it |
| `moving` | move flag `0x02` moving, else `0x04` stopping | send one `moving: false` packet when the movement ends |
| `state: MoveState` | actor flags | `Ground` = `0x04` standing on solid, `Jump` = `0x10` jumping (move flags `0x06`), `Fall` = no ground flag |
| `fall_vel: u16` | actor flag `0x80` + `u16` | set once, in the packet of the landing; `FALL_VELOCITY_PER_MPS` = 506 per m/s; the server applies fall damage from it (nothing below 20 m/s) |
| `stance`, `alertness`, `in_combat` | stance byte, alertness byte, move flag `0x08` | `model::stance`, `model::alertness` |

Walking vs running is not a flag: it is the velocity (and the animation the other clients pick from
it). The `0x20` (standing on an object) and `0x40` (hanging) actor blocks are not written. The server
needs no sequence number: `time` is milliseconds since the session started and only has to grow.

Commands (`session::Command`):
`Login`, `RefreshWorldList`, `EnterWorld { world_id }`, `CancelEnterWorld`, `ConnectToWorld`,
`RefreshCharacters`, `CreateCharacter(CreateCharacter)`, `DeleteCharacter`, `CancelDeleteCharacter`,
`EnterGame { character_id }` (select, spawn, notify in game), `Move(MoveCommand)`,
`MoveVehicle(VehicleMoveRequest)`, `Chat`,
`LeaveWorld(Quit | CharacterSelect)`, `CancelLeave`, `ReturnToWorldSelect`, `Disconnect`.

Events (`session::Event`), in the order of a normal run:
`PhaseChanged(Phase)`, `LoginSucceeded(AccountInfo)`, `WorldList`, `WorldQueue`, `ReadyToConnect`,
`LobbyReady`, `RaceCongestion`, `CharacterList`, `CharacterCreated`, `CharacterSelected`, `PlayerSpawn`,
`EnteredWorld`, then `UnitSpawned`, `UnitMoved`, `UnitsRemoved`, `UnitAttached`, `UnitDetached`, `UnitPosture`,
`UnitPoints`, `UnitDeath`, `CombatFirstHit`, `AiAggro`,
`DoodadsSpawned`, `DoodadsRemoved`, `DoodadPhaseChanged`, `Teleported`, `Chat`, `ServerError`, `Kicked`,
and for leaving `LeavePending`, `LeftWorld`, `Disconnected`. Failures: `LoginFailed`, `EnterWorldDenied`,
`ConnectFailed`, `CharacterCreateFailed`, `RequestTimedOut`.

`Phase`: `Idle → Authenticating → WorldSelect → (Queued) → ReadyToConnect → Connecting → CharacterSelect
→ EnteringWorld → InWorld → (Leaving)`.

`PlayerSpawn` carries `obj_id`, the full `CharacterState` (id, name, race, gender, level, zone id,
instance id, rotation, equipment, `ModelParams`, abilities, money, labor, experience ...) and the player's own `Unit`
(position, model id, equipment templates, appearance). Teleports are answered automatically
(`CSTeleportEnded`) and reported as `Teleported`.

### State tables (`state`)

`Worlds { list, queue, account }`, `Characters { list, race_congestion }` (a cancelled delete clears the
delete timer of its character), `Player { spawn }`,
`Units { by_id }`, `Doodads { by_id }`, `Status { phase, last_error }`, and for the HUD `PlayerStats`,
`Buffs`, `ActionBar`, `Inventory`, `WorldInfo`, `Chat`, `Target`, `Skills`, `Quests`, `Loot` (see "In-game HUD data");
each has `apply(&Event) -> bool`, `WorldState` bundles them. They clear themselves on `LeftWorld` /
`Disconnected`.

### Bevy plugin (`--features bevy`, `aa_net::NetPlugin`)

```rust
app.add_plugins(aa_net::NetPlugin);
// send:    MessageWriter<NetCommand>  -> out.write(NetCommand(Command::Login { .. }))
// receive: MessageReader<NetEvent>    -> for NetEvent(event) in events.read() { .. }
// state:   Res<Worlds>, Res<Characters>, Res<Player>, Res<Units>, Res<Doodads>, Res<Status>,
//          Res<PlayerStats>, Res<Buffs>, Res<ActionBar>, Res<Inventory>, Res<WorldInfo>,
//          Res<Chat>, Res<Target>, Res<Skills>, Res<Quests>
```

The plugin starts the session thread in `build`, forwards `NetCommand` messages in `PreUpdate`, drains the
events, applies them to the resources (change detection only fires for resources an event changed)
and republishes them as `NetEvent` messages. Everything happens before `Update`.
`cargo run -p aa-net --features bevy --example bevy_walkthrough` is a complete example.

## In-game HUD data (packet to event to state)

Every server packet below is decoded in `game::ServerMsg`, forwarded by the session as an
`Event` and folded into a `state` resource. All of them are Bevy resources under `NetPlugin`
(and fields of `WorldState` without Bevy). Resources clear themselves on `LeftWorld` /
`Disconnected`; the per-character ones restart at `CharacterSelected`.

**Local player: `PlayerStats`** (`aa_net::state::PlayerStats`)

| packet | event | state |
|---|---|---|
| `SCCharacterState` (select) | `CharacterSelected` | `character_id`, `name`, `level`, `hp`, `mp`, `exp`, `recoverable_exp`, `labor` (+ `labor_updated`), `money`, `bank_money`, `loyalty`, `active_abilities`, `ability_exp` |
| `SCUnitState` (own, spawn) | `PlayerSpawn` | `obj_id`, `level`, `hp` / `mp` and `max_hp` / `max_mp` |
| `SCUnitPoints` | `UnitPoints { obj_id, hp, mp }` | `hp`, `mp`, raises `max_hp` / `max_mp` |
| `SCExpChanged` | `ExpChanged { obj_id, delta, add_ability_exp }` | `exp += delta` |
| `SCRecoverableExp` | `RecoverableExp` | `recoverable_exp` |
| `SCLevelChanged` | `LevelChanged { obj_id, level }` | `level` (and the unit's level in `Units`) |
| `SCAbilityExpChanged` | `AbilityExpChanged { obj_id, ability, exp }` | `ability_exp[ability]` |
| `SCAbilitySwapped` | `AbilitySwapped` | `active_abilities` |
| `SCCharacterLaborPowerChanged` | `LaborChanged { amount, action, point, step }` | `labor += amount`; `labor_log` keeps `(amount, labor after)` of the session's changes (last 16, cleared with the stats) for the chat line "Gained 10 Labor Point(s). (Total LP: 105)" the client prints, also for the ones that came while the screen was loading |
| `SCCharacterGamePoints`, `SCGamePointChanged` | `GamePoints`, `GamePointChanged { kind, amount }` | `honor`, `vocation` |
| `SCBmPoint` | `LoyaltyChanged { bm_point }` | `loyalty` (loyalty tokens) |
| `SCCombatEngaged` / `SCCombatCleared` | `CombatChanged { obj_id, engaged }` | `in_combat` |
| `SCUnitDeath` | `UnitDeath { obj_id }` then `DeathDetails { kill_reason, resurrect_wait_ms, lost_exp, killer }` | `dead`, `hp = 0`; in `Units` also `Unit::dead = true` |
| `SCNotifyResurrection` | `ResurrectionOffered { caster: BuffCaster }` | event only; the HUD retains the offer until the player is dead |
| `SCCharacterResurrected` | `Resurrected { obj_id, pos, yaw }` | `dead = false` (and `Unit::dead = false`) |
| `SCLootableState` | `Lootable { owner_type, obj_id, has_loot }` | `Loot::lootable` (corpses with loot for the player; also cleared by `UnitsRemoved`) |
| `SCLootBagData` | `LootBag { items, loot_all }` | `Loot::bag` (what is left in the corpse that was opened), `bag_seq` + 1 |
| `SCLootItemTook` | `LootItemTook { template_id, item_index, owner_type, owner_obj_id, count }` | the item leaves `Loot::bag`, `taken_seq` + 1 |
| `SCLootItemFailed` | `LootItemFailed { error, item_index, owner_type, owner_obj_id, item_template_id }` | `Loot::failure` |
| `SCUnitLootingState` | `UnitLooting { obj_id, looting }` | none |

Loot items carry their owner and index in the item id (the server makes up `(owner object id << 32) + (owner type <<
16) + index`): `state::item_index(&item)` and `state::item_owner(&item)` read them, which is what `Command::LootItem` needs.
The server only sends `SCLootableState` when the NPC's loot packs rolled something (the Angry Rat's one item has drop rate 1
in 10 million, so it practically never does: the HUD has `--hud-fake-loot` to see the icons and the window).

Helpers: `exp_progress() -> (earned, needed)` (bar text `earned / needed`, `needed` is 0 at level 55;
the table is `model::exp`, installed once by the app from the game database's `levels.total_exp` with
`exp::install`), `ability_level(id)`, `money_parts() -> (gold, silver, copper)`,
`hp_fraction()`, `mp_fraction()`, `LABOR_MAX` (2000; 5000 for premium). The title in the unit frame
("Battlerage Novice") comes from `active_abilities` and `ability_level`. Without an installed table
(`AA_DB` missing, unit tests) `exp::FALLBACK` keeps the original 1.2 curve, including level 55 at
179,307,360; a server that patches `levels` with the 3.0 retail values for 51-55 therefore needs no
client change.

The server never sends maximum HP / MP: the real client computes them from the database's unit
formulas (`unit_formulas`, see `docs/aa-client-hud.md`, "Max HP / MP"). aa-net has no database, so
`max_hp` / `max_mp` are the highest value seen since the spawn (exact for a player who spawns at full
health) until the application calls `PlayerStats::set_max(max_hp, max_mp)` with the computed values (never
below the current points); `Unit::max_hp` is the same for other units (NPCs are announced at full health).

**Buffs: `Buffs`** (`aa_net::state::Buffs`, `Buffs::of(obj_id) -> &[BuffEntry]`)

| packet | event | state |
|---|---|---|
| `SCUnitState` (any unit, its buff lists) | `PlayerSpawn`, `UnitSpawned` | the unit's list is replaced (`Unit::buffs` keeps the snapshot) |
| `SCBuffCreated` | `BuffCreated { obj_id, buff }` | pushed, or replaces the entry with the same `index` |
| `SCBuffRemoved` | `BuffRemoved { obj_id, index }` | removed |
| `SCBuffLearned` | `PassiveBuffLearned` | event only |

`BuffEntry { buff, received }`: `buff.buff_id` (`buffs.id`; `icon_id` and `kind_id` in the game
database), `buff.index`, `buff.stacks`, `buff.duration_ms` (0 = permanent), `buff.elapsed_ms` at
arrival, `buff.kind` (`Good` / `Bad` / `Hidden` from the unit-state list, `Unknown` for later
`BuffCreated`: use `buffs.kind_id`), caster; `remaining_ms()` counts down from `received`. The
unit frame shows the first four `Good` and `Bad` ones.

**Action bar: `ActionBar`** (`ActionBar::slots[i]`, i = slot number - 1, 85 slots)

| packet | event | state |
|---|---|---|
| `SCActionSlots` (select) | `ActionSlots(Vec<ActionSlot>)` | `slots` |
| `CSUpdateActionSlot` (`Command::SetActionSlot { slot, action }`) | `ActionSlotChanged { slot, action }` (raised by the session, the server does not echo) | `slots[slot]` |

`ActionSlot { kind: ActionKind, id }`: `Spell` = skill id (`skills.id`), `ItemType` = item template
(`items.id`, shows the bag total), `ItemId` = one item's database id, `Macro`, `RidePetSpell`,
`None`. `skill_id()`, `item_template_id()`, `item_id()` pick the right one;
`ActionBar::item_count(slot, &Inventory)` is the number on a potion slot.

**Items: `Inventory`**

| packet | event | state |
|---|---|---|
| `SCCharacterState` | `CharacterSelected` | `equipment` (28 slots, `model::slot`), sizes; bag and bank start empty |
| `SCCharacterInvenInit` | `InventoryInit { bag_slots, bank_slots }` | `bag_slots`, `bank_slots` |
| `SCCharacterInvenContents` (chunks of 10) | `InventoryChunk(InvenChunk)` | `bag[n]` / `bank[n]` = `Option<Item>` |
| `SCItemTaskSuccess` | `ItemsChanged(ItemTasks { reason, tasks, force_remove })` | `bag`, `bank`, `equipment` follow `ItemTask::{Add, Count, Remove, Move, Grade, Flags, Update ...}`; `Money` / `BankMoney` / `GamePoint` tasks update `PlayerStats::money` / `bank_money` / `honor` / `vocation` |
| `SCInvenExpanded` | `InventoryExpanded` | sizes |
| `SCUnitEquipmentsChanged` | `EquipmentChanged { obj_id, items }` | `Inventory::equipment` for the player, `Unit::equipment` for other units |

`Item { template_id, id, count, grade, flags, detail, .. }`. Helpers: `bag_items()`, `count_of(template)`,
`find_template(template)`, `find_by_id(id)`, `free_bag_slots()`, `worn(model::slot::MAINHAND)`.
`ItemTasks::reason` is the server's `ItemTaskType` (9 loot, 13 skill source consumed, 53 sold ...)
for chat lines such as "You acquired ...". Containers: `items::container::{EQUIPMENT 1, INVENTORY 2, BANK 3}`.

**World: `WorldInfo`** (zone, game clock, weather)

| packet | event | state |
|---|---|---|
| `SCCharacterState` | `CharacterSelected` | `zone_key` (`character.zone_id`, = `zones.zone_key`), `instance_id` |
| `SCJoinedChatChannel` for Shout (type 1) | `ChatChannelJoined { chat_type: 1, sub_type, .. }` | `zone_group_id = Some(sub_type)` (= `zones.group_id` = `zone_groups.id`) |
| `SCLeavedChatChannel` for Shout | `ChatChannelLeft` | `zone_group_id = None` if it was the current one |
| `SCDetailedTimeOfDay` | `TimeOfDay { hour, speed: Some }` | `time = TimeOfDay { hour, speed, received }` |
| `SCTimeOfDay` | `TimeOfDay { hour, speed: None }` | `time` (keeps the speed) |
| `SCOnOffSnow` | `Snow(on)` | `snow` |

The clock: `hour` is 0..24 game hours, `speed` is game hours per real second (the server sends
0.0016666: 6 game seconds per real second, a day lasts 4 real hours). `WorldInfo::hour_now() ->
Option<f32>` is the hour now (extrapolated from the packet). That is the `WorldClock { hour, speed }`
of aa-world. The server sends the detailed time once at spawn and a plain `SCTimeOfDay` again after
every teleport. The zone name of the minimap header: `zones.zone_key = zone_key` gives `group_id`;
`zone_group_id` (from the zone channel) tells the same after a zone border crossing, when the
character state's `zone_key` is stale; `zone_groups.display_text` is the name ("Solzreed Peninsula").
The server does **not** send the sub zone ("Desiree Peak"): resolve it on the client from the player
position against the `sub_zones` rectangles (`x`, `y`, `w`, `h` in metres, `linked_zone_group_id`,
`parent_sub_zone_id`). `zone_key` is only read when the character is selected; after a teleport or a
border crossing `zone_group_id` is the current zone. The server streams the surroundings of a new
position (units, doodads, the zone chat channel switch) only after the client sends movement from
there: after `Event::Teleported` send a `Command::Move` or two (the real client moves all the
time; `walkthrough --cmd "/teleport ..."` does it).

**Chat: `Chat`**

| packet | event | state |
|---|---|---|
| `SCJoinedChatChannel` | `ChatChannelJoined { chat_type, sub_type, faction_id, name }` (the zone channel, type 1, as three events: types 1, 2 and 3) | `channels` |
| `SCLeavedChatChannel` | `ChatChannelLeft { chat_type, sub_type, faction_id }` (also three for the zone channel) | `channels` |
| `SCChatMessage` | `Chat(ChatMessage)` | `log` (the last `CHAT_LOG_LIMIT` = 500 lines, oldest first) |
| `CSSendChatMessage` | `Command::Chat { chat_type, target, message }` | |

At spawn the server joins the player to Shout (1, sub type = the zone group id), Nation (6, faction
148) and Ally (14, faction 148). The zone channel carries shout, trade (2) and party search (3) lines
(the server routes `chat_type` 1 / 2 / 3 messages through it with the same sub type), and the real client
announces all three when it joins ("Entering Chat: 1.Shout.Solzreed Peninsula", "2.Trade.Solzreed
Peninsula", "3.Party Search.Solzreed Peninsula", then 6 and 14: `reference/ingame/spawn`,
`royal_palace`), in the city as in the field. aa-net does the same: one `SCJoinedChatChannel` /
`SCLeavedChatChannel` of type 1 raises `ChatChannelJoined` / `ChatChannelLeft` for types 1, 2 and 3
(`session::channel_types`), so `Chat.channels` lists all three and the zone name comes from
`zone_groups` for each. `ChatMessage { chat_type, sub_type, channel_faction,
sender, message, .. }` carries the channel it came through; `model::chat_type` has the ids (-2 system
text such as the "Welcome to AAEmu!" the server sends after entering, -1 notice, 0 say, 1 shout ...)
and `chat_type::name(id)`. Game messages such as "Gained 10 Labor Point(s)" are not chat lines on the
wire: the client builds them from `LaborChanged`, `ItemsChanged` (loot), `ExpChanged` and so on.
A GM command is a chat line that starts with `/` (`Insufficient privileges.` comes back as a
type -2 line for accounts without GM rights).

**Emotes and speech bubbles (`emote.rs`)**

| packet | event / command | body |
|---|---|---|
| `CSExpressEmotion` 0x0ad | `Command::ExpressEmotion { emotion_id, target_obj_id }` | `bc own_obj, bc target_obj (0: nobody), u32 emotion_id` |
| `SCEmotionExpressed` 0x12b | `Emote(Expressed { obj_id, target_obj_id, emotion_id })` | the same three fields |
| `SCChatBubble` 0x154 | `Emote(ChatBubble { obj_id, kind, how, body })` | `bc obj, u8 kind, u8 how, then string text (how 1) or u32 bubble id` |
| `SCAreaChatBubble` 0x153 | `Emote(AreaBubble { enter, obj_id, bubble_type })` | `bool enter, bc obj, u32 type` |

Oracle for the layouts: the server's `systems/services/express_text.rs` and `systems/skills/packets_special.rs` (byte-exact
tests there), and the C# originals in git history (`a2ab80abd^`: `CSExpressEmotionPacket.cs`, `SCEmotionExpressedPacket.cs`,
`SCChatBubblePacket.cs`, `SCAreaChatBubblePacket.cs`). The server answers `CSExpressEmotion` by broadcasting
`SCEmotionExpressed` to everything in range, the sender included (no check, no cooldown), so the sender's own clip and chat line
also come from the packet. `emotion_id` is an `express_texts` row. The server sends `SCChatBubble` only for the skill effect
`BubbleEffect` (`how` 2, a `bubbles` id) and `SCAreaChatBubble` (543) only in the C# duel manager (not in the Rust server). The
requests are sent in world only. There is no `SCUserChatMessage` in the 1.2 opcode tables of the server or of the C# project:
say-chat bubbles are not a packet, the client draws them from `SCChatMessage` (see `docs/aa-client-hud.md`, "Emotes and speech
bubbles").

**Target: `Target`**

| packet | event | state |
|---|---|---|
| `CSChangeTarget` (`Command::Target { obj_id }`, 0 clears) | | |
| `SCTargetChanged` (also echoed to the sender) | `TargetChanged { obj_id, target_obj_id }` | `Target::obj_id` when `obj_id` is the player |
| `SCUnitsRemoved` | `UnitsRemoved` | cleared when the target left the view |

`Target::unit(&Units) -> Option<&Unit>` gives the unit for the target frame (`name`, `level`, `hp` /
`max_hp`, `faction_id`, `buffs`). NPC names are empty on the wire: show the template id from
`Unit::kind` (`Npc { template_id, .. }`, `npcs.name` in the game database). `Event::TargetChanged`
for other units is event only.

**Skills: `Skills`** (`aa_net::state::Skills`)

| packet | event | state |
|---|---|---|
| `SCUnitState` (own, spawn) | `PlayerSpawn` | `known` (`KnownSkill { id, level }`), `passive_buffs` |
| `SCSkillLearned`, `SCSkillUpgraded` | `SkillLearned`, `SkillUpgraded { skill_id, level }` | `known` |
| `SCSkillsReset` | `SkillsReset { obj_id, ability }` | event only (drop the skills of `skills.ability_id`) |
| `SCBuffLearned` | `PassiveBuffLearned` | `passive_buffs` |
| `SCSkillStarted` (`result` 0) | `SkillStarted(SkillStarted)` | `casting[caster]`, and for the player `last_used[skill]`, `last_cast` and `last_cast_skill` (the skill the global cooldown comes from); a `Mount` caster stamps only `mount_last_cast[caster]` (the slave the player drives; acceptance starts its global cooldown, the server sends it no `SCCooldowns`) |
| `SCSkillStarted` (`result` != 0, `tl` 0) | `SkillStarted` with `failed()` | `last_failure` (`skill::result::name`) |
| `SCSkillFired` | `SkillFired` | `casting[caster].fired`; a `Mount` caster stamps `mount_last_used[(caster, skill)]` (the server starts the skill's own cooldown when the cast ends, so an interrupted cast stamps none); starts the skill when no `SkillStarted` came first, with the same player / `Mount` stamps (there it also stamps the global cooldown) |
| `SCPlotEvent` | `PlotEvent { tl, event_id, skill_id, caster_obj_id, target_obj_id, caster_pos, target_pos, casting_ms }` (`event_id` is the `plots` step; the positions are the ones a step aimed at the ground carries) | the first step of a plot skill starts it (`casting`, `last_used`); steps with `casting_ms` > 0 set the cast time |
| `SCSkillEnded`, `SCCastingStopped`, `SCSkillStopped`, `SCPlotEnded`, `SCPlotCastingStopped`, `SCPlotChannelingStopped` | `SkillEnded`, `CastingStopped`, `SkillStopped`, `PlotEnded`, `PlotStopped` | `casting` entry removed |
| `SCCooldowns` | `Cooldowns(SkillCooldowns)` | received skill/tag timers appended for an existing local player (see native cooldown lists below) |
| `SCSkillCooldownReset` | `SkillCooldownReset { obj_id, skill_id, tag_id, gcd }` | `last_used[skill]` removed, `reset_tags`, first matching received entries reset, GCD cleared when requested |
| `SCUnitsRemoved` | `UnitsRemoved` | `casting` entries removed; a removed slave's `mount_last_used`/`mount_last_cast` timers are pruned so a reused object id starts clean |
| `SCUnitDamaged` | `UnitDamaged(UnitDamage { action, caster_obj_id, target_obj_id, damage, absorbed, mana_burn, holdable_id, raw_hit, hit_type })` | event only (HP follows in `UnitPoints`) |
| `SCUnitHealed` | `UnitHealed(UnitHeal { target_obj_id, heal_type, hit_type, value })` | event only |
| `SCCombatText` | `CombatText { source, target, text_type }` | event only |

**Combat events for visuals** (what `ingame/combat.rs` draws, see `docs/aa-client-combat.md`): for any unit a cast starts as
`SkillStarted { tl, skill_id, caster, target, cast_ms }`, fires as `SkillFired { tl, skill_id, caster, target, delay_ms, fire_anim_id }`
(`delay_ms` is how long the effects take to land: an arrow in flight), and every plot step is a `PlotEvent`. Damage is
`UnitDamaged` (`action.tl` ties it to its cast), then `UnitDeath { obj_id }`. The server sends **no projectile packet**: the client
derives the flight from `SkillFired::delay_ms`. `Unit::dead` is set for a unit that spawns with `hp <= 0` (a corpse in view),
by `UnitDeath`, and cleared by `UnitPoints` with `hp > 0` or `Resurrected`. A skill the player aims badly is refused with
`SkillStarted` `result::OUTOF_ANGLE` (0x10): the client has to face the target first.

The lane uses cast observations and DB durations; the native `SCCooldowns` receive contract also
allows explicit duration/remaining timers (see below). The HUD
reads `Skills::cooldown_left(skill_id, cooldown_tag_id, cooldown_ms)` (`skills.cooldown_time` and
`cooldown_tag_id` come from the game database), `gcd_left(ms)`, `since_used(skill)` and
`casting_of(obj_id) -> Option<&Casting>` (`progress()` is the cast bar, 0.0 to 1.0; the target frame
shows the target's cast with the same call). `UnitDamage::action` says which skill (or buff tick,
`kind` 2) dealt it; `hit_type` is `skill::hit_type::*` (`is_critical`, `is_miss`). The server's
spell results are messages in `skill::result`.

Skill results arrive in `DD04` batches (raw DEFLATE blocks of `[u16 0][u16 opcode][body]`). The
session unpacks them and reports every packet as if it had come alone, in order: `Event::UnitDamaged`
after `PlotEvent`s and so on. A packet in a batch this client cannot decode ends the batch (the
rest cannot be found: the bodies have no length). Stored and Huffman-compressed streams are
supported with bounded inflation and complete-stream validation (see "Compressed packet batches").

**Quests: `Quests`** (`aa_net::state::Quests`, `aa_net::quest`)

| packet | event | state |
|---|---|---|
| `SCQuests` (sent after the action slots on entering; up to 20 per packet) | `QuestsLoaded(Vec<Quest>)` | `active` |
| `SCCompletedQuests` (up to 200 blocks per packet) | `CompletedQuests(Vec<CompletedBlock>)` | `completed` |
| `SCQuestContextStarted` (0x147) | `QuestStarted(Quest)` | `active` gets the quest |
| `SCQuestContextUpdated` (0x149) | `QuestUpdated(Quest)` | the quest is replaced; status dropped / completed / failed removes it |
| `SCQuestContextCompleted` (0x14b) | `QuestCompleted { quest_id, block }` | removed from `active`, the block replaces its `completed` entry |
| `SCQuestContextReset` (0x14c) | `QuestReset { quest_id, block }` | the same: the bit of the quest is cleared |

`Quest { id, template_id, status, objectives: [i32; 5], time_left_ms, component_id, accept_time,
acceptor_type, acceptor_id }` follows the server's `Quest.Write`; `template_id` is `quest_contexts.id`,
`status` is `quest::status::*` (1 progress, 2 failed, 3 ready, 4 dropped, 5 completed). A completed quest
is one bit of a 64 quest `CompletedBlock { id, bits }`: block `quest_id / 64`, bit `quest_id % 64`, the
first byte of the packet being the lowest. `Quests::active_quest(template_id)`, `is_completed(template_id)`.
The state clears on `CharacterSelected`, `LeftWorld` and `Disconnected`. The server never says which NPC
starts a quest: the client works it out from the game database (`docs/aa-client-hud.md`, "Quest marks").
Quests can be changed on the lane server with the GM chat commands `/quest add <id>` and
`/quest remove <id>` to watch the updates.

**Commands**

| command | wire | answer |
|---|---|---|
| `Command::Target { obj_id }` (0 clears) | `CSChangeTarget` | `TargetChanged` |
| `Command::UseSkill { skill_id, target_obj_id }` (0 = self) | `CSStartSkill`, unit caster | `SkillStarted` (refusal) or `PlotEvent` / `SkillFired`, `UnitDamaged`, `UnitPoints`, `BuffCreated` ... |
| `Command::UseSkillAt { skill_id, pos }` | `CSStartSkill`, position target | as above |
| `Command::StopCasting { tl, plot_tl }` | `CSStopCasting` (`u16 tl`, `u16 plot_tl`, `bc` the player's unit; `tl` for a plain skill, `plot_tl` for a plot, the other 0) | `SkillEnded` + `CastingStopped` (`PlotStopped` for a plot) if the skill still ran; the server logs and ignores it otherwise |
| `Command::UseItem { item_id, template_id, skill_id, target_obj_id }` | `CSStartSkill`, item caster | `SkillFired`, `UnitHealed`, `ItemsChanged` (`Count` delta -1), `SkillEnded` |
| `Command::UseItemAt { item_id, template_id, skill_id, pos, rot }` | `CSStartSkill`, item caster, `POSITION` target with `rot` (the 2026-10-30 summon capture: skill 15802, item 5223, x/y 12000/9030, z 99.8727646, rot 1.5707964 = player yaw + pi/2) | as `UseItem` |
| `Command::SetActionSlot { slot, action }` | `CSUpdateActionSlot` | `ActionSlotChanged` (raised by the session) |
| `Command::SwapItems(ItemMove)` | `CSSwapItems` (0x03a) | `ItemsChanged` reason 5 (`Move`s), or `ServerError` |
| `Command::SplitItem { mv, count }` | `CSSplitBagItem` (0x039) | `ItemsChanged` reason 6 (`Count` on the source, `Add` on the target) |
| `Command::DestroyItem { item_id, container, slot, count }` | `CSDestroyItem` (0x038) | `ItemsChanged` reason 1 (`Remove` or `Count`) |
| `Command::Chat { chat_type, target, message }` | `CSSendChatMessage` | `Chat` echo |
| `Command::Resurrect { in_place }` | `CSResurrectCharacter` | `Resurrected`, `UnitPoints`, weakening `BuffCreated`s |
| `Command::LootOpen { obj_id, loot_all }` | `CSLootOpenBag` (`bc` corpse, `bc` 0, `bool` loot all) | `LootItemTook` per item taken (+ `ItemsChanged`), `LootBag` with what is left |
| `Command::LootItem { owner_type, owner_obj_id, item_index }` | `CSLootItem` (`u16` index, `u16` owner type, `bc` owner, 2 x `u16` 0) | `LootItemTook` + `ItemsChanged`, or `LootItemFailed` (bag full, no right to the item) |
| `Command::LootClose { owner_type, owner_obj_id }` | `CSLootCloseBag` | none |
| `Command::LeaveWorld(LeaveTarget::Quit)` | `CSLeaveWorld` | `LeavePending`, `LeftWorld`, `Disconnected` |

`SCNotifyResurrection` (0x041) contains one `SkillCaster`, decoded with `BuffCaster::read` (kind,
object id and the extra fields of item/mount casters). Its `ResurrectionOffered` event preserves
wire order: the lane server's free in-place offer for a character below level 10, unit caster 0,
arrives before `UnitDeath` / `DeathDetails`. The HUD must retain that offer and raise its revival
prompt once dead. The focused `cargo test -p aa-net --lib resurrection` checks caster layouts,
truncation, compressed-batch alignment and event order (3 matching tests passed).

For `UseItem`, `item_id` is `Item::id` of the bag item, `skill_id` its `items.use_skill_id` (the
server refuses any other skill for the item), `Inventory::find_template(template)` gives the item.
The lane server does not check that the skill was learned or that its ability is active (verified with
`cargo run -p aa-net --example skilltest -- --no-learn`: unlearned skills run). It checks the target, range,
the cooldown (`result` 8, no `SCCooldowns`: the client estimates cooldowns from the database), mana,
movement and the buffs that forbid casting. A refused skill comes back as `SkillStarted` with `result` != 0
and `tl` 0. Every skill of the ten ability trees, the default skills, emotes and ground casts were cast
against the lane with `skilltest`: `docs/research/skill-functional.md` has the packet trace per skill.

## Quests and NPC dialogs

`aa_net::interaction` holds what the client sends when the player talks to an NPC and takes, hands in or
drops a quest (`Command::Interact(Interact)`), and the server's answers that are not quest state
(`Event::Interaction(InteractEvent)`). The quest state itself is `Quests` (above). The layouts come from
the server's `systems/npcs/npc/packets.rs` and `systems/quests/packets.rs`. The unit tests check them
byte for byte against the server's own test vectors. `bc` is the 3-byte object id.

| `Interact` | wire | server answer |
|---|---|---|
| `Npc { obj_id, target_changed }` | `CSInteractNPC` 0x065: `bc` npc, `bool` | sets the interaction object (and the target), answers an empty `SCAiAggro` (skipped) |
| `NpcEnd { obj_id }` | `CSInteractNPCEnd` 0x066: `bc` | none |
| `Start { npc_obj_id, obj_id, extra_info, pick_id, mouse_button, modifier_keys }` | `CSStartInteraction` 0x068: `bc bc i32 i32 u8 i32` | `SCNpcInteractionSkillList` (the NPC's or slave's interaction skills) |
| `QuestStart { quest_id, npc_obj_id, doodad_obj_id, sphere_id }` | `CSStartQuestContext` 0x0d5: `u32 bc bc u32` | `SCQuestContextStarted` (+ `Updated`), nothing when refused |
| `QuestComplete { quest_id, npc_obj_id, doodad_obj_id, selected }` | `CSCompleteQuestContext` 0x0d6: `u32 bc bc i32` | `SCQuestContextCompleted`, `SCExpChanged`, items / money |
| `QuestDrop { quest_id }` | `CSDropQuestContext` 0x0d7: `u32` | `SCQuestContextUpdated` with status 4 (dropped) |
| `QuestTalk { npc_obj_id, quest_id, component_id, act_id }` | `CSQuestTalkMade` 0x0da: `bc u32 u32 u32` | `SCQuestContextUpdated` |

| `InteractEvent` | packet |
|---|---|
| `SkillList { npc_obj_id, obj_id, extra_info, pick_id, mouse_button, skills, interactable, modifier_keys }` | `SCNpcInteractionSkillList` 0x07b (the request's values echoed, `i32` count + `u32` skills, `bool`, `i32`) |
| `DoodadQuestAccept { doodad_obj_id, quest_id }` | `SCDoodadQuestAccept` 0x111 (a doodad's quest function offers a quest: the client opens the dialog) |
| `RewardedByMail { quests }` | `SCQuestRewardedByMail` 0x14e (`u8` count + `u32` ids) |
| `NpcChat { chat_type, faction_id, npc_obj_id, npc_name, character_obj_id, kind, text_id, message }` | `SCNpcChatMessage` 0x0cf (`kind` 1: a text id, else a string) |

`SCQuestContextStarted` and `SCQuestContextUpdated` are read by `quest::read_context`. After the quest,
they carry the component the quest is at as a trailing `u32` (`Updated` then has four more values). The
server's `Quest.Write` only fills its own component field for timed quests, so `Quest::component_id` is
set from the trailing one. This is how the HUD knows which step's objectives to show.

The client side of the dialog, which NPC offers what and the quest windows, is the HUD's
(`docs/aa-client-hud.md`, "Quests and NPC dialogs"). There the sequence is: right click or F →
`CSChangeTarget` + `CSInteractNPC`, the dialog's confirm button → `CSStartQuestContext` /
`CSCompleteQuestContext` / `CSQuestTalkMade`, the dialog's end → `CSInteractNPCEnd`.

What the lane server (`bevy`) does not do:

* It sends no refusal: `SCQuestContextFailed` and `SCQuestUnitReqFailed` are never sent, so a refused
  accept or hand-in gets no answer (the HUD's dialog stays open until Escape).
* `CSQuestStartWith` and `CSRestartMainQuest` are only logged.
* No distance check on talk, accept or hand-in: any NPC in view can be talked to.
* `CSInteractNPC` only gets an empty `SCAiAggro`: no greeting, no `SCNpcChatMessage`.

## What the UI scripts need

The login-stage scripts (`x2ui/loginstage/world_select`, `character_select`, `character_create`) call:

| Lua | aa-net |
|---|---|
| `X2World:GetWorldListInfo()` / `GetWorldInfo(id)` | `Res<Worlds>.list.worlds`: `id`, `name`, `online`, `congestion` (0 low, 1 middle, 2 high), `race_congestion[race]` (race ids 1..8, `login::congestion::*`) |
| `GetWorldCharacterCount(s)` | `Worlds.list.character_count(world_id)`; `characters_of(world_id)` has name / race / gender |
| `X2World:EnterWorld(world, zone)` | `Command::EnterWorld { world_id }` |
| `OPEN_WORLD_QUEUE`, `REFRESH_WORLD_QUEUE` | `Event::WorldQueue(q)`: `my_turn`, `normal_length`, `premium_length`, `premium` (`GetWorldQueuePosition`, `GetWorldNormalQueueLength`, `GetWorldPremiumQueueLength`, `IsInPremiumQueue`). The server sends no expected wait time (`GetWorldQueueExpectedTime` is the client's own estimate) |
| `CancelWorldQueue` | `Command::CancelEnterWorld` |
| `READY_TO_CONNECT_WORLD`, `X2World:ConnectToWorld()` | `Event::ReadyToConnect` then `Command::ConnectToWorld` |
| `ENTER_WORLD_CANCELLED`, `DISCONNECT_FROM_AUTH` | `Event::EnterWorldCancelled`, `Event::Disconnected` / `LoginFailed` |
| `RequestWorldListRefresh` | `Command::RefreshWorldList` |
| `IsPreSelectCharacterPeriod`, `GetTotalCharactersDefaultLimit`, `GetTotalCharactersExpandableLimit` | `Worlds.account`: `pre_select_period`, `max_characters`, `extra_characters_per_world` |
| `RACE_CONGESTION` of the creation screen | `Characters.race_congestion` (`Event::RaceCongestion`) |
| `X2LoginCharacter:GetNumLoginCharacters`, `GetLoginCharacterName/Level/Race/Gender/Abilities/Zone/Faction/FactionName/Money/BmPoint/LaborPower` | `Characters.list[i]`: `name`, `level`, `race`, `gender`, `abilities`, `zone_id`, `faction_id`, `faction_name`, `money`, `bm_point`, `labor_power` (+ `labor_power_modified`; the max is derived by the client) |
| `IsDeleteRequestedCharacter`, `GetCharacterDeleteWaitingTime`, `IsTransferRequestedCharacter` | `delete_request_time != 0`, `delete_time`, `transfer_request_time` |
| `GetLastPlayedCharacterIndex` | index of the highest `leave_time` |
| `SelectCharacter`, `CreateCharacter`, `DeleteCharacter`, `CancelCharacterDelete`, `RequestCharacterListRefresh` | `Command::EnterGame`, `CreateCharacter`, `DeleteCharacter`, `CancelDeleteCharacter`, `RefreshCharacters` (the cancel answer is `Event::CharacterDeleteCancelled`) |
| `LeaveWorld` / `EXIT_TO_WORLD_LIST` (server button) | `Command::LeaveWorld(CharacterSelect)` then `Command::ReturnToWorldSelect` |

Character creation data: `CreateCharacter { name, race, gender, body_items: [u32; 7], model_params, abilities, level }`.
`body_items` are the template ids of face, hair, glasses, reserved, tail, body and beard (0 = the
server's default for the race / gender). `ModelParams` carries hair color, skin color, model id and
the `FaceModel` (decals, maps, colors and the 128 byte customizer `modifier`). `CreateCharacter::new(name,
race, gender)` is a valid default. The name regex is `[a-zA-Z0-9]{1,18}`; the server normalizes the
case (`Bevytest`). `CharacterCreateFailed { reason }`: 3 failed / pending deletion, 4 name exists,
5 invalid characters.
Deleting is delayed: `CharacterDeleteResponse { status: 2, delete_request_time, delete_time }`
(`delete_time` = request + 60 s at level 1, the character stays listed with `delete_requested()` until
then); `CancelDeleteCharacter` answers `SCCancelCharacterDeleteResponse { id, status }` (opcode `0x03c`, body
`u32 id, u8 status`): status 3 (restored) becomes `Event::CharacterDeleteCancelled { character_id, restored:
true }`, which clears the character's delete timer in `Characters`; status 4 (unknown character) gives
`restored: false`. The session refreshes the character list after either answer.

## Evidence (lane `bevy`, `walkthrough`)

```text
login:  15:09:17 [INFO] login - bevytest connected.
game:   name - AddCharacterName, Registered character name Bevytest (1)
        session - CSSpawnCharacterPacket
        session - NotifyInGame: Bevytest (1)
        session - NotifyInGameCompleted SubZoneId 0, Bevytest (1)
        db_character - Saved 1 active buff(s) for character 1      (clean logout)
        listener - Client from 127.0.0.1 disconnected
```

No `ERROR` lines caused by the client; the only warnings are the server's own tick timings. The saved
position after the move survives the next login (`--roundtrip` spawns at the moved position).

### HUD data on the lane server (`walkthrough`, account `aanet2`, character `Aanetwalker`)

```text
spawn      TimeOfDay 4.8 h speed 0.0016666 | ChatChannelJoined Shout 1/5 (zone group 5), Nation 6 faction 148,
           Ally 14 faction 148 | Snow(false) | 5 own buffs (2423 logged on, 8000011/8000012 premium, 8298 and
           8226 hidden / equipment) | 85 action slots, 4 bag items, 46 of 50 slots free
Cross Plains  88 NPCs: faction 1 (16, lvl 33-50), 115 monsters (21, lvl 31), 162, 164 ...; 53 with buffs
           (713 / 715 "wearing cloth / leather", hidden 7109), hp 18610 for a lvl 50, all via SCUnitState
/teleport  Teleported + TimeOfDay; the new area arrives after the first movement packets; zone channel
           Shout 1/5 left, 1/20 joined (group 5 -> 20)
CSChangeTarget  TargetChanged { player, npc } echoed to the sender; cleared with 0
potion     UseItem 18791:25202 -> SkillFired (item caster), UnitHealed 180, ItemsChanged Count -1 (bag 3 -> 2),
           SkillEnded; no SkillStarted for an instant item skill
melee      UseSkill 18132 at a lvl 1 monster: PlotEvent x n, UnitDamaged 27 / 28 / 33 per slash (hit_type 1,
           critical 3), BuffCreated 3761 on the target, mp 484 -> 454 per cast, PlotEnded; the monster
           answers with SkillFired 2 / SkillEnded; after 4 casts ExpChanged +95 (exp 95 / 400),
           UnitDeath + DeathDetails { killer Aanetwalker }, CombatChanged; a cast at the corpse is
           refused: SkillStarted { tl 0, result 4 "target died" }
death      lvl 31 monsters killed the lvl 1 character; it spawns dead after a login (hp 0, dead);
           Command::Resurrect { in_place: false } -> Resurrected, UnitPoints 10 %, BuffCreated 1128
           (weakened body) and 2385 (respawn cooldown)
```

## Keep-alive and the server's dead connection sweep

The game server closes a game connection **silently** (trace level log only, no error) when its
`remove_dead_connections` sweep, which runs every 30 s, decides the client does not ping. The client
sends `PingPacket` every 3 s; what matters is the start: **the first ping has to go out as soon as the
game connection opens**. The measured rule on the lane server (`walkthrough --first-ping-delay-ms`,
`--no-ping`, sweep phase measured from the kill times, `:02.6` / `:32.6` UTC and creeping 12 ms per
sweep):

| client | sweep lands this long after the connect | result |
|---|---|---|
| never pings | 1.0 .. 4.98 s | survives |
| never pings | 5.07 s and later (6.4, 8.2, 19.3, 31.4 s) | **closed** |
| first ping 1.0 .. 3.5 s after connect (the old client) | 5.2, 5.6, 5.6, 5.7, 6.1, 7.35 s | **closed** (6 of 6) |
| first ping 1.5 .. 3.5 s after connect | 8.7, 12, 20 s | survives |
| ping at connect (now; then every 3 s, or no more pings) | 5.4, 5.6, 5.9, 6.8, 7.1, 11.5, 19.7, 27.5 s and a 120 s soak | survives (every run) |

So a connection is spared while it is younger than 5 s, and after that only if it has been pinging for
5 s or more; a client that starts pinging a few seconds late is closed when a sweep falls in the
window between 5 s and (first ping + 5 s) after its connect. That is a 5 to 8 s window per 30 s
cycle, about one run in ten; the old session also started the 3 s timer when the *session*
began and not when the game connection opened. The lane log of the in-game client runs shows it: two
runs started exactly 60 s apart (17:08:56 and 17:09:56) both ended about 5 s after connecting, the two
`Client ... disconnected` lines logged at the sweep time and no error.

The fix (`session.rs`): the ping timer starts when the game connection opens, and the first ping is
sent before the enter world request. `ClientOptions { ping_interval, first_ping_delay }` exist only to
reproduce the old behaviour in tests.

Not causes (checked): the movement rate (10 Hz for 2 minutes, 1200 packets, no disconnect), the
`time` field, flags and actor flags, packets far from the spawn, below the terrain, jump / fall
sequences with a 25 m/s landing; the server's move verifier is off by default and only logs.

## Other units' motion (`unit_msg`, `model::Movement`)

What decides how NPCs, mobs, other players and mounts move on screen, from the server's writers (`models/movement.rs`,
`systems/skills/packets.rs`, `systems/vehicles/packets.rs`, `systems/npcs/npc/packets.rs`) and their byte tests, which the
unit tests here repeat:

| Packet | Event | Layout |
|---|---|---|
| `SCOneUnitMovement` 0x06c | `UnitMoved` | `bc id`, type byte, `MoveType.Write` |
| `SCUnitMovements` 0x06b | one `UnitMoved` per entry | `u16 count` (at most 400), per entry `bc id` and `MoveType.Write` **without** the type byte: the real client takes the type from the object, here every entry is read as a unit movement (players, NPCs, mounts). The lane server has the writer but never sends it (every NPC step is a `SCOneUnitMovement`, 10 per second per moving NPC); it is decoded for the real servers |
| `SCUnitAttached` 0x076 | `UnitAttached` | `bc rider`, `u8 attach point` (1 driver, 2.. passengers), `bc mount`, `u8 reason`; `Unit::attached_to` keeps all four fields. Live capture 2026-10-28: the whole frame is 14 bytes (8 body bytes); the S->C log has only opcode + byte count, so the reply's reason value is not in the capture. AAEmu echoes `CSMountMate`'s reason (`mates.rs:340-344`) and sends `bond_kind` for slave seats (`slaves.rs:1383-1387`); the original server is unverified. The 1.2 reader names the byte `reason` (`FUN_397c60c0`) and the attach model maps 2/3/4 else 1 before `FUN_390bb4d0` calls `FUN_39153b60` |
| `SCUnitDetached` 0x077 | `UnitDetached` | `bc rider`, `u8 reason` |
| `SCUnitModelPostureChanged` 0x103 | `UnitPosture` | `bc id`, `Unit.ModelPosture` (posture byte, loot flag, per posture: 4 = `u32 anim_actions.id` + `bool activate`); `Unit::anim_action` is the pose an NPC stands in (`SCUnitState` carries it too) |
| `SCCombatFirstHit` 0x089 | `CombatFirstHit` | `bc victim`, `bc hitter`, `u32 hit type` |
| `SCAiAggro` 0x1c4 | `AiAggro` | `bc npc`, `i32 count`, per hostile unit `bc`, three `i32` (the first is the damage it did) and a flag byte (135) |

`Movement::read_typed(type, r)` reads all movement types: unit (1; `Movement::support` holds the object the unit stands on or
hangs from, with its world position, when the actor flags say so), vehicle (2) and ship (3) (the rotation shorts are kept in
`vehicle` / `ship`; `rotation_quat()` rebuilds the full quaternion and `vehicle_yaw` its yaw), ship request (4, no position)
and transfer (5). The state resource puts a unit that stands on an object at its world position and sets the yaw of vehicles
from the rotation; the drawn pose of remote vehicles uses the full rotation (`aa-client` `ingame/vehicle_track.rs`).

NPC movements: the server sends a step every 100 ms with `flags` 0, `actor_flags` 4 (running) or 5 (walking), `delta_movement`
`[0, 127 | 63, 0]`, the stance and alertness of the NPC (`alertness` 2 = in combat) and a **velocity of a fixed 4000** (3.9 m/s
whatever the speed): the speed has to be measured from the positions. Players send their own velocity (1/1024 m/s), the
pressed direction in `delta_movement` and, in the air, no `STANDING_ON_SOLID` (a jump: `JUMPING` and flags `Moving | Stopping`).
Mounts move like any unit; the rider gets no movement of its own (the server puts its local position at 0, 0, 0 under the mount).
The local player's own attachment is in `Player::spawn.unit.attached_to` (`UnitAttached` / `UnitDetached` with its object id,
cleared by `Resurrected`). The driver of an owned mate moves it with `Command::MoveUnit { obj_id, movement }`: the same
`CSMoveUnit` as `Command::Move`, with the mate's object id (the lane accepts unit movement for the character and its own mate
only, and ignores a riding character's own movement).
An announced player is not announced again to itself: a later `SCUnitState` with our own object id is dropped (it would draw a
second figure on top of the player).

## Moving items

The session does not track the inventory, so the item commands are plain data and carry the item ids; the
application builds them from the `Inventory` resource: `plan_move(from, to)` (bag to bag, bag to bank, worn to bag,
worn to worn), `plan_equip(bag_slot, equip_slot)`, `plan_unequip(equip_slot, bag_slot)`, with `item_at(container, i)` and
`first_free_bag_slot()`. All three client packets start with the same block (`items::ItemMove`): `u64` item id of
the first slot, `u64` item id of the second (0 for an empty slot), then container, slot of the first and container,
slot of the second as `u8`s (`items::container`: equipment 1, bag 2, bank 3; slots are the indexes of
`Inventory::bag` / `bank` and the `model::slot` of `Inventory::equipment`). `CSSplitBagItem` adds an `i32` count after
it, `CSDestroyItem` is `u64` item id, container, slot, `i32` count. Moving to an empty slot, swapping, merging two
stacks, wearing (`from` the worn slot with its item, or 0 when it is empty, `to` the bag item) and taking off are all
`CSSwapItems`; the server decides and answers with `SCItemTaskSuccess` (`ItemsChanged`: reason 5 swap, 6 split, 1
destroy) that `Inventory::apply_task` folds, or with an error message (`ServerError`) and no change. Nothing is applied
before the answer.

### First-session checks and unit lifecycle (2026-10-17)

The item planners reject unknown containers and slots outside the current bag, bank or equipment
capacity before sending a request. A ship request (movement type 4, throttle and steering only)
does not produce a position update or replace the last movement report. Remote death clears old
motion; resurrection applies the server's position and facing and clears old motion and the mount
attachment. Removing a unit also removes its outstanding cast, so a despawned caster cannot keep
showing a cast bar.

Aggro lists are consumed in full instead of being truncated at 64 entries. Their announced count
is checked against the available bytes; an oversized list used to leave entry bytes in place of
the following opcode in a compressed packet batch.

`cargo test -p aa-net --test live_first_session -- --ignored --nocapture` is an opt-in lane check.
It creates a uniquely named throw-away account and Nuian character on `$AA_LOGIN`, moves a
bag item to an empty slot and back, takes off and re-equips the starter weapon, splits and merges
a stack while checking its total count, and resets Battlerage and relearns Triple Slash. It logs
out even when one of those checks fails. The normal test run skips this server-dependent check.

Skill reset packets carry an ability id rather than the removed skill ids. The application must
use its database to remove the tree's skills/passives and replay any following learn packets in
order before taking a HUD snapshot. `NetPlugin` folds all incoming events before `Update`; simply
clearing a tree afterward can otherwise erase a skill learned later in the same frame.

Verified on 2026-10-17: 111 library tests and 2 scripted login-session tests pass;
`cargo check -p aa-net --features bevy --examples` passes. The opt-in live first-session check
passed on the bevy lane in 10.39 s, including a clean logout, with a fresh timestamped account
(the shared `bevytest` account was not used).

## Parties and raids (2026-10-17)

`Command::Party(PartyCommand)` sends `Invite { team_id, is_party, target_name }`,
`Reply { invite, accept }`, `Leave { team_id, character_id }`,
`Kick { team_id, member_id }`, `Dismiss { team_id }`, and
`MakeOwner { team_id, member_id }`. A new invitation uses `team_id: 0, is_party: true`.
Replies retain the complete `PartyInvite` received in `PartyEvent::Invited`; `accept: false`
rejects it. The session fills the reply's character-name field with the local selected
character (native join task `393075a0`, not the inviter's name). A successful send emits local `PartyEvent::Replied` to close that prompt; only
the server's `Joined` changes membership. The server's direct `CSLeaveTeam`,
`CSKickTeamMember` and `CSDismissTeam` handlers only log, so the commands use the functional
`CSAskRiskyTeamAction` with action 0, 1, or 2 instead.

`Party` is a `NetPlugin` resource and `WorldState::party`. `id == 0` means no membership.
`owner_id`, `PartyMember::id` and invitation `sender_id` are **character IDs**, while
`PartyMember::obj_id` is the current unit used for targeting. The member list is ordered by
`slot` (0..49, groups of five for a raid). It tracks names, race/gender, roles, online state,
level, abilities, zone/instance, positions, death state, and HP/MP including their real maxima when the
server sends the 50-byte remote status (`has_status`). Initial nearby vitals and positions
are seeded from the existing `Units`/`Player` snapshots, then updated by unit packets or
remote-member status. Before a remote status, maxima have the same highest-seen limitation
as `Units`. The local player's own HUD should continue using `PlayerStats` and the app's
current movement position. Logout/disconnect clears party state; login reannounces it.

All twenty server party packet types are decoded, including offline/reconnect snapshots,
member/leader/role changes, raid conversion and slot moves, loot rules, markers and map pings.
The joined packet's ten group counts must match its fifty fixed slots; there is no separate
member count after the loot rule. Negative, over-fifty and truncated remote-member counts
are rejected. A test places each packet before a second opcode in a `DD04` batch and checks
both decode, preventing a partially consumed record from corrupting later combat packets.

`cargo test -p aa-net --test live_party -- --ignored --nocapture` creates two fresh timestamped
accounts and characters on the bevy lane. Passed in 10.32 s: reject an invitation; invite and
accept; compare both rosters and leader; move the second player and receive position/vitals
(720/720 HP, 670/670 MP); send party chat; transfer leadership twice; kick with pair
auto-disband; reform, leave voluntarily and receive auto-disband; cleanly log out both clients.
The shared user account was not touched. Library tests: 120 pass; the Bevy/examples check passes.
Offline/reconnect and raid transitions have protocol/state coverage but were not exercised live
in this two-person test. The later raid milestone below adds conversion, slot/role/loot-rule edits, markers and pings
with separate fresh paired live verification.

For an integrated HUD check, `cargo run -p aa-net --example party_peer -- --invite CharacterName
--seconds 90` remains compatible: a fresh letter-only character prints READY, invites the
supplied character once, automatically replies to direct invitations, reports authoritative
party events, sends stationary movement every2s, then leaves and logs out. No area invitation
is sent; incoming area invitations are declined. Default lifetime90s, configurable10..3600s.
`--account FreshAccount --name LettersOnly` selects an unused test account/name (ASCII name
2..18 letters); password is the disposable account name. Caller must provide an unused account,
never an existing user's login. Setup refuses a nonempty character list after authenticating.
`--accept-from Character` restricts automatic acceptance to that name. `--position x,y,z`
places/stationary-refreshes the peer, and `--role 0..3` selects its own role after joining.

`--ping-after 5 --clear-after 8` schedules one ping relative to confirmed membership, waiting
for exact lane owner/marked authority. Optional `--ping-position x,y,z` and `--ping-instance ID`
override its actual stationary position/current instance. It starts the clear timer only after
a matching server Ping ACK, clears only if membership/authority/current ping still match, and
reports a10s missing-ACK timeout without fabricating success. A replacement root ping is not
cleared. REQUEST logs are outgoing; ACK/STATE log server-confirmed roles, slots, owner, loot,
markers and pings; LOCAL identifies local reply/validation events. READY names the peer for the
root client to invite. Coordinate the lifetime with the visual client before launch.

Paired executable verification used fresh chars45/46, team8: filtered autoaccept, both self
roles, owner ping and clear echoed identically on both peers; unmarked nonowner waited for
permission and sent no unauthorized ping. Both logged DONE after clean logout. Rebuilt helper
copy is `data/extract/aa-net/party-peer-qa.exe` (ignored); the version controlled source is
`examples/party_peer.rs`. Existing raid live test additionally covers convert, sparse move,
loot rules, marker-permitted member ping and clear. Native permission source is
`systems/social/team.rs::set_ping_pos`/`Team::is_marked`; the latter compares stored marker IDs
without a marker-type check. New joins can receive the lane's initial true ping at(0,0,0), even
when nobody placed one; the client retains the authoritative packet rather than inventing a
presence bit the lane does not store.

The original native invitation handler creates `DLG_TASK_JOIN_TEAM` (2), rather than firing a
Lua invitation event. The task's fill function (`x2game_unpacked.dll` VA `393096c0`) sets
`title`, `content`, `btnOkText` and `btnCancelText` for the default dialog handler:
`LocalizeUiText(MSG_BOX_TITLE_TEXT, key)`, `LocalizeUiText(MSG_BOX_BODY_TEXT, key, senderName)`,
and button keys `invite` / `reject` in `MSG_BOX_BTN_TEXT`; `key` is `ask_party_invite` or
`ask_raidteam_invite`. English body text is `$1 invited you to a party. Would you like to join?`
(or `raid`); buttons are Join / Decline. The original default dialog invokes
`X2DialogManager:OnOK(windowId)` / `OnCancel(windowId)` to reply through the native task.

Native party event contracts (all indices are global one-based slots, 1..50):

* `TEAM_MEMBERS_CHANGED("joined_by_self", isParty, ownerName)`.
* `TEAM_MEMBERS_CHANGED("joined" | "leaved" | "kicked", isParty, memberName, memberIndex)`.
* `TEAM_MEMBERS_CHANGED("owner_changed", isParty, newOwnerName, oldOwnerIndex, newOwnerIndex)`.
* `TEAM_MEMBERS_CHANGED("leaved_by_self" | "kicked_by_self" | "dismissed", isParty)`.
* `TEAM_MEMBER_DISCONNECTED(isParty, oldObjectIdString, memberIndex)`.
* `TEAM_MEMBER_UNIT_ID_CHANGED(memberIndex, oldObjectIdString, newObjectIdString)`.
* `TEAM_ROLE_CHANGED(memberIndex, role)`.

Raid commands use the same `Command::Party` path:

* `ConvertToRaid { team_id }` (0x080).
* `SetRole { team_id, member_id, role }` (0x085): 0 undecided, 1 tank, 2 healer, 3 attacker.
  The lane accepts only the sending member's own role. Original Lua calls `X2Team:SetRole(role)`.
* `MoveMember { team_id, id_from, id_to, from, to }` (0x081): **zero-based** slots 0..49;
  `id_to: 0` means an empty destination. `Party::plan_move(from,to)` resolves both IDs from
  the latest slots. Moves swap slots, preserving sparse groups and member vitals/roles.
* `ChangeLootingRule { team_id, rule: LootingRule, flags }` (0x083): flags 1 method, 2 grade,
  4 master character ID, 8 roll for bind-on-pickup. Methods are 0 free, 1 rotate, 2 master,
  255 public; switching away from master clears the master ID in the server response.
* `SetMarker { team_id, index, kind, id }` (0x086): index0..11, kind0 clear / 1 character ID /
  2 object ID. Reset all uses index-2, kind0, id0. Object IDs use three bytes; character IDs
  use four. The server echoes clears as kind1/id0; the reducer normalizes them to `(0,0)`.
* `SetPing { team_id, has_ping, pos, instance_id }` (0x087): plain f32 world coordinates.
  `has_ping:false` clears the current ping. Marker/ping requests with team0 are solo echoes.
* `InviteArea { team_id, is_party }` (0x07b): exact request only; see the server gap below.

Role, conversion, move, loot, marker and ping acknowledgements are existing `PartyEvent`
variants and fold into `Party`; commands never modify confirmed state. Invalid roles, IDs,
slots, flags, grades, marker types/indices or non-finite ping coordinates emit local
`PartyEvent::RequestRejected(message)` without sending. This validates the wire domain;
the application still checks current membership and authority. The lane silently ignores
unauthorized commands. Conversion, member moves and looting rules require the owner;
raid markers require the owner, while party markers allow members. Pings require the
owner or a character ID present in the marks. Reset/leave/disconnect clears these resources.

Original Lua uses `MoveTeamMember(fromIndex,toIndex)` and
`MoveTeamMemberToParty(memberIndex,partyIndex)` with **one-based** member and group indices.
Native wrappers `394b07a0`/`394b07d0` subtract one; the latter picks the destination group's
first free slot. `X2Unit:SetOverHeadMarker(unitToken,index)` likewise converts Lua1..12 to
wire0..11 (`394bb9b0`). Lua consumes `SET_OVERHEAD_MARK(unitIdString,index,visible)` and
`TEAM_MEMBERS_CHANGED("moved",isParty,fromIndex,toIndex)`. Loot UI calls
`ChangeLootingRule(method,masterName,minimumGrade,rollForBop)`; notices are
`LOOTING_RULE_METHOD_CHANGED(method)`, `LOOTING_RULE_MASTER_CHANGED(name)`,
`LOOTING_RULE_GRADE_CHANGED(grade)` and `LOOTING_RULE_BOP_CHANGED(0|1)`.

Server limits: officer assignment has no 1.2 opcode (`CSSetTeamOfficer=0xfff`), conversion
back to party has no supported request, and dungeon raid conversion remains a server TODO.
Area invitations select friendly/neutral players within100m, but the lane's accepted
`is_area` branch removes the invitation without joining (TODO); no successful area join is
fabricated. Area broadcasts were not live-tested. Loot dice packets remain a separate gap.

`cargo test -p aa-net --test live_party raid_roles_sparse_moves_loot_markers_and_pings --
--ignored --nocapture` passed in10.40s using two fresh characters (IDs33/34, team6):
self roles1/2, conversion, slot1->9 then0<->9, all loot fields, automatic master clearing,
character/object markers, a marked-member ping, single/all clear, ping removal and raid
dismissal. Both clients observed matching acknowledged state and logged out cleanly.

The packet handlers and `393077xx` onward in the native client confirm the membership arguments;
the original `x2ui/unitframe/party.lua` and `raidteammanager/manager.lua` consume them. A generic
roster refresh alone loses the original localized notices and specific offline/unit-ID handling.

## NPC shops and banks (2026-10-17)

`Command::Shop(ShopCommand)` exposes `ListSold { npc_obj_id }`,
`Sell { npc_obj_id, items: Vec<SellEntry { container, slot, item_id }> }`, and
`Buy { npc_obj_id, doodad_obj_id, buy: Vec<BuyEntry { item_id, grade, count, currency }>,
buy_back: Vec<i32>, use_aa_point }`. A sale moves the entire stack; split beforehand for a
partial sale. Buyback entries are zero-based positions in the latest sold list. Batch counts
are bytes (at most 255); non-positive purchase counts, invalid buyback slots, duplicate
sale/buyback entries and invalid containers are rejected locally before sending.
`Event::Shop(ShopEvent::RequestRejected(message))` reports that local failure.

`Shop` is installed by `NetPlugin` and lives at `WorldState::shop`. `sold: Vec<Item>` is the
server's `SCSoldItemList` response, limited to twelve real items; negative, over-twelve and
truncated counts are rejected. `loaded` distinguishes an empty response from no response;
`dirty` becomes true after store item tasks (reason 54 buy / 53 sell), so request `ListSold`
again. `Event::Shop(ShopEvent::SoldItems(items))` replaces that list and clears `dirty`.
Transactions update `Inventory` and `PlayerStats` only through the server's existing item tasks.

The catalog is not a server packet. Resolve the NPC template through `merchants` to
`merchant_packs`, then `merchant_goods` (encounter/id order; first duplicate item ID wins).
`merchant_packs.kind_id` maps to the original Lua/checkout currency as 0 -> gold(0),
1/2 -> honor(1), 3 -> vocation(2). Native `GetStoreCurrency` (VA `394ae1d0`, helper `39572470`)
confirms that siege kind2 also spends honor. Buy prices use `items.price`, `honor_price` or
`living_point_price` times quantity, without a grade multiplier or NPC discount in the lane.
`merchant_price_ratios` exists but the server checkout never reads it. The requested catalog
grade is parsed but currently ignored by the server's item acquisition; show the received grade.

Both wire `Item.grade` and `merchant_goods.grade_id` are raw `item_grades.id` values.
Lua display order is a separate `grade_order` (raw ID0 = order1 Basic; raw ID1 = order0 Crude).
The refund lookup uses raw ID: per-unit refund multiplies `items.refund * refund_multiplier`
in i64, converts that product to f32 and divides by100, truncates, then multiplies by stack count.
A zero refund remains zero; there is
no fallback to the purchase price. Buyback charges this same refund. Native
`GetSellItemsMaxCount` returns 16 (method `394ae840`, float constant `3999f964`), making the
original cart ten cells (`16 - 6`) and sell list sixteen.

`Command::Bank(BankCommand)` provides `DepositMoney { amount, aa_point }`,
`WithdrawMoney { amount, aa_point }`, `Expand { container, auto_use_aa_point }` (2 bag,
3 bank; native context zero), and `ExpandWithContext { container, auto_use_aa_point,
raw_context_obj_id }`. Expansion now writes the complete native eight-byte body; see the
service re-audit below. Negative amounts, invalid containers and over-24-bit contexts are refused locally. Bank slots and contents
already arrive at login in `Inventory.bank_slots` / `bank`; read them with `bank_items()` /
`first_free_bank_slot()`. Moves and splits use the existing `plan_move` and `SwapItems` /
`SplitItem` commands with container3. Balances are `PlayerStats.money` / `bank_money`, updated
by the money tasks; expansion is acknowledged by `InventoryExpanded` and preserves slot items.

Opening services uses `Interact::Npc`, then `Interact::Start`: its `SkillList` contains 12087
for a merchant or 13238 for a banker. Original `ShowInteraction` categories are STORE_NPC=3,
BANKER_NPC=10. `SelectNpcInteraction(category,0)` is a native no-op (method `3947da30` only
returns from the script handler), so no learned skill cast is needed. There is no bank-open
or bank-list packet. Buy enforces merchant/catalog and three metres; sell/list enforce a
merchant. The lane bank handlers currently do not enforce an interaction object. AA-point
fields and auto-AA expansion are parsed but ignored. Expansion deducts its configured money
price without emitting the corresponding MoneyChange task (an explicit C# carry-over in
`inventory::expand_slot`); the capacity is confirmed but wire balance stays stale until refreshed.

`SCItemTaskSuccess` now consumes its trailing `u32 type(id)` and refuses a partial transaction
with unknown item tasks. `SCCharacterInvenContents` reads exactly `numChunks * 10` slots;
reading to end-of-frame previously consumed the next packet inside `DD04`. Boundary tests
place sold lists, money/empty tasks and bank chunks before another opcode and verify alignment.

`cargo test -p aa-net --test live_shop_bank -- --ignored --nocapture` uses a fresh account,
no grants and only its own starter equipment. Verified merchant12547 at
(14366.27,15504.10,152.86171), pack171, and banker12544 fifteen metres away at
(14365.36,15519.09,152.86171): service12087/13238; sell starter item23387 for33 copper;
list and buy back the same item ID/count for33; deposit/withdraw5 copper restoring both balances;
move the item into bank and back preserving identity; sell it again and buy item15659 for25
copper with a confirmed +1 item / -25 copper. Both bank and normal inventory start at50 slots.
Expansion success was not live-tested because this fresh character has no expansion certificate;
capacity decoding/reduction is already covered by the inventory tests.

## Friends and blocked users (2026-10-17)

`Command::Friend(FriendCommand)` sends `Add { name }` (0x104), `Remove { name }` (0x105),
`Block { name }` (0x107), or `Unblock { name }` (0x108). Each body is one length-prefixed
string. Empty/whitespace-only, over128-byte or NUL-containing names are rejected locally.
`Event::Friend(FriendEvent)` retains all acknowledged IDs, canonical names, success flags
and signed16-bit server error codes: `Added`, `Removed`, `Blocked`, `Unblocked`,
`StatusChanged`, `List` and `BlockedList`. Local `Requested(command)` follows a successful
send; `RequestRejected { command, message }` means no packet was sent.

The `Friends` Bevy resource and `WorldState::friends` contain `friends: Vec<FriendInfo>`,
`blocked: Vec<BlockedUser>`, `total` / `blocked_total`, `loaded` / `blocked_loaded`,
`pending: Vec<FriendCommand>` and `last_error: Option<FriendFailure>`. Only successful server
acknowledgements change membership; failed replies retain lists and record the original
error code. Status changes update existing friends only. Matching acknowledgements remove
pending requests by operation/name (ASCII case-insensitive). Logout/disconnect/character
selection resets all state. Login lists arrive **before** PlayerSpawn and must survive it.
The lane omits empty list packets; EnteredWorld confirms that both initial lists are loaded.

`FriendInfo` includes character `id`, `name`, `race`, `level`, `hp`, three `abilities`,
`pos`, `zone_id`, `type_id`, `in_party`, `online` and raw `last_world_leave_time`.
Its position is two long fixed-point coordinates plus float height, followed by the zone,
trailing type ID, two booleans and i64 leave time. It is not a packed9-byte position.
`BlockedUser` contains `id` and `name`. Lists have separate i32 total/count fields; negative,
inconsistent, truncated or duplicate-ID lists are rejected without consuming a following
DD04 packet. Helpers are `friend(id)`, `is_blocked(name)` and
`page(show_offline,start_zero_based,count)` (preserves server order). Native UI sorts
level descending with stable ties:39304770?39302f90 compares FriendInfo byte+0x8c,
confirmed as the level formatted into Lua row2 at393040ba. GetFriendsPerPage wrapper
39475710?39300c70 reads global39ac72d0; its exact runtime initialization is unresolved.
Do not infer its page constant from the ten visible rows. Offline row4 uses elapsed Unix
time clamped at0, then394245f0?39429b20 locale formatting; exact localized strings remain
an app/native-formatting concern.

Original `X2Friend` calls: `AddFriend(name)`, `DeleteFriend(name)`, `BlockUser(name)`,
`UnblockUser(name)`, `GetBlockList()` returning row tables plus total,
`GetFriendCount()` returning online then total, and
`RequestFriendList(showOffline,start_one_based)`. The last is local cache filtering,
sorting and pagination (native wrapper39475330 calls393049e0); it has no lane packet.
Lua `FRIENDLIST_INFO(totalFiltered,start_zero_based,rows)` rows use columns
1name,2level,3abilities-table,4position/last-online display,5race,6online,7inParty;
column8 is the Lua selection checkbox. `FRIENDLIST_UPDATE("insert"|"update",row)`
uses the same row formatter39303f30 via39304400; deletion instead sends
`FRIENDLIST_UPDATE("delete",friendNameString)` (native393038e0). `BLOCKED_USER_UPDATE`
takes no arguments.
`FRIENDLIST(message)` and `BLOCKED_USER_LIST(message)` carry localized chat notices.

Lane limits: add/remove friend supports offline stored characters. Block/unblock requires
the target to be online. Duplicate friend/block additions, absent names and removals of
missing entries silently return with no acknowledgement; pending requests remain unresolved
until the application times out or resets its session. No success or server rejection is
invented for those cases. The lane currently permits self-add/self-block; the original
native client provides its own local self/duplicate-name checks. Status broadcasts cover
online/offline and in-party transitions, not a continuous position/level feed.
The lane does not suppress chat from blocked users; the application can filter displayed
sender names through `Friends::is_blocked`. Raw `Event::Chat` remains available for inspection.

`cargo test -p aa-net --test live_friends -- --ignored --nocapture` passed in40.67s with
fresh characters36/37. It verified mutual adds, party join/disband presence, block/unblock,
logout/offline status, saved offline friend/blocked lists after reconnect, login/online
broadcast, removals on both peers and persisted empty lists after a second reconnect.
Both peers logged out cleanly. Six unit tests cover every incoming packet in DD04 batches,
long coordinates/time, malformed list counts/duplicates, failures/pending and spawn ordering.

## Repair, sorting and expansion workflow (2026-10-17)

`Command::Repair(RepairCommand)` sends `Single { container, slot, auto_use_aa_point }`
(0x03c; u8/u8/bool) or `All { auto_use_aa_point }` (0x03d; bool). Single targets the bag or
equipment with physical zero-based slots; bank targets are rejected locally. All repairs
equipped items only, in server container order, and may skip unaffordable entries. The AA
flag is parsed but ignored. Original Lua calls `X2Item:RepairAll()`, `RepairAllCost()`,
`EnterRepairMode()` / `LeaveRepairMode()` and `X2Equipment:ItemRepair(slot)`.

`Repair` is a Bevy resource and `WorldState::repair`. `pending: Vec<RepairCommand>` tracks
successful sends (`Event::Repair(RepairEvent::Requested(command))`); malformed requests use
`RequestRejected { command, message }` and `last_error`. `last_report: Option<RepairReport>`
is cleared when sending and filled only by a complete `ItemsChanged` reason3 (repair).
Truncated item tasks do not complete a request; the packet decoder already rejects malformed
frames, and the resource now enforces this independently for replay/manual events.
Its `items: Vec<RepairedItem { container, slot, item_id, durability }>` lists confirmed
detail updates. `Some(empty)` means a response with no repairs; `None` means no response yet.
Ordinary reason4 durability loss does not complete a repair. Inventory folds full item details
and preserves identity, dyes, gems and temper values. Repair costs arrive separately as
reason83 money tasks before the repair detail packet; PlayerStats applies that confirmed delta.

The lane requires `Interact::Npc` to a **blacksmith** within5m. It advertises repair skill12086
for `npc.repairman`, but the transaction instead checks `npc.blacksmith`: the current database
has331 blacksmith-only and3 repairman-only templates, with none having both flags. This is a
real server mismatch. Blacksmith8233 at(13573.98,15907.689,175.212) is also a merchant and
advertises12087 due to option precedence; repair itself needs no UseSkill cast. Wrong NPC,
healthy/unrepairable item and insufficient funds produce an empty reason3 report; distance
also emits a server error. No successful repair is synthesized from a request or an empty report.

`repair::repair_cost(price,raw_grade_refund_multiplier,factor,current,maximum)` implements
the lane's f32 formula then ceil. The factor is `item_configs.durability_repair_cost_factor`
(25.0 in current data); maximum durability comes from equipment data, not a network quote.
The result is `ceil(factor * .01f32 * (1-current/maximum) * price * refund_multiplier * .01f32)`
with the server's ordered f32 operations. Invalid/missing maximum returns None.

`bank::ExpansionRule` mirrors `bag_expands` fields `is_bank,step,price,item_id,item_count,
currency_id`. `bank::expansion_quote(&inventory,container,money,&rules)` returns the first
matching row for `(slots-50)/10`, current/new capacity, price/certificate metadata and owned
certificates **in the bag**, including bank expansion. Quotes remain available when
unaffordable; `quote.check()` returns typed `NotEnoughMoney { required, available }` or
`NotEnoughCertificates { item_id, required, available }`. Invalid container/capacity,
missing row and malformed negative requirements have separate `ExpansionFailure` variants.
These are local preflight results: the lane silently ignores missing money/items, so they
must not be presented as received server errors. Immediately before confirmation use
`quote.recheck(&inventory,current_money)` to reject stale capacity and refresh requirements.
Confirmation sends `BankCommand::Expand` or `ExpandWithContext`; `InventoryExpanded` alone confirms capacity. Current rows expand by10,
require certificate8000025x1 at price0, and stop at bag150 / bank130. Native dialog info uses
`kind="bag"|"bank"`, `dialog="money"|"item"|"item_money"`, `currency,cost,itemType,itemCount`;
the original `DLG_TASK_EXPAND_INVENTORY` handler formats this supplied metadata.

Inventory sorting has **no packet** in the 1.2 opcode table. Native X2Bag/X2Bank ManualSort
wrappers3944c720/39452130 call394dc6d0, which sorts the current local tab, emits TAB_SORTED
and persists/refreshes local tab data. The default tab's manual sorter392024d0 sorts an array
of real slot indices (container2 or3); filtered tabs use their own item-ID lists. It never
rewrites the server inventory. Keep a UI-owned virtual-to-physical mapping; Lua
`SlotByIdx(virtual_one_based)` returns physical_slot+1 (wrapper3944ef90). Use the physical
slot for item lookup and every repair/sell/swap request. Sort should update this view mapping
and fire BAG_TAB_SORTED/BANK_TAB_SORTED; confirmed item changes rebuild the view. The default
sort keys are `item_categories.category_order` ascending, `item_grades.grade_order` descending,
localized item name ascending, physical slot ascending; empty/missing items sort last.
Native comparators391f8640/391f8720 call391f7c50/391f7ce0; category loader396c9c53 stores
category_order as record field0, grade_order is grade record+0x2c, name is item template+0x70,
and the final tie compares the physical slot byte. This ordering belongs to the client view;
aa-net does not invent a server sort or optimistically change `Inventory`.

`cargo test -p aa-net --test live_repair -- --ignored --nocapture` passed in10.41s with
fresh character38. Verified no-interaction and healthy-blacksmith empty acknowledgements,
real merchant funds from its own starter weapon, and local bank-target rejection. Ordinary
nonlethal falls did not yield damaged armor, so a positive **paid** repair remains unverified
live; no item/grant fixture was injected. Unit coverage checks full repair details plus
separate money tasks before the next DD04 opcode, empty reports, wear/pending behavior and
certificate/money/capacity preflight. The test account logged out cleanly.

## Mailbox protocol and state

`Command::Mail(MailCommand)` and `Event::Mail(MailEvent)` expose the original mailbox
lane. `Mailbox` is both `WorldState.mailbox` and a Bevy resource. It owns `received`/`sent`
header vectors, `bodies: Vec<CachedMailBody { is_sent, body }>`, `unread: UnreadMail`
(sent/received/mia_received/commercial_received), `unread_loaded`, `loaded`, `listing`,
`dirty`, the last snapshot's `total_headers`/`total_bodies`, `pending`, and `last_error`.
Use `header(is_sent,id)`, `headers(is_sent)` and `body(is_sent,id)` for lookup. IDs and
Unix timestamps are i64; original Lua mail IDs are decimal strings. All mail categories
remain available; regular/commercial/MIA filtering belongs to the original UI.

Commands are `List`, `ListContinue`, `Read { is_sent, mail_id }`,
`Delete { is_sent, mail_id }`, `Send(SendMail)`, `TakeMoney { mail_id }`,
`TakeItem { mail_id, attachment: MailAttachment }`, `TakeAll { mail_id }`,
`TakeAllAndDelete { mail_id }`, and `PayHousingTax { mail_id, auto_use_aa_point }`.
Read writes bool then i64; Delete writes i64 then bool. The list snapshot arrives in
multiple `MailEvent::List` chunks, independently marking sent/received; append/upsert
until `ListEnd`. Only locally sending `List` clears old headers/bodies. An empty completed
snapshot is meaningful. ListContinue is currently a lane log-only handler; List sends
all headers. Login's unread packet survives PlayerSpawn; character selection, leave and
disconnect clear the resource. Requested means the request was sent, never success.

`SendMail { mail_type, receiver, title, text, money, items:Vec<MailSlot>, doodad_obj_id }`
accepts normal1/express2/registered3. `money` is copper, `MailSlot { container:2, slot }`
is a zero-based physical bag address. The entire stack is attached; split first for a
partial stack. At most ten item attachments; the first item or money attachment is free.
`postage()` mirrors lane50c+(additional attachments)*30c for normal/registered,
100c+additional*80c for express. Normal delivery is delayed1800s; registered and express
are immediate. Native normal UI uses type strings `mail_normal` / `mail_express`.
Sending requires a real doodad within5m2D whose current funcs include
DoodadFuncNaviOpenMailbox. List/read/claim have no mailbox gate. General SCErrorMsg155
is MAIL_SUCCESS after the Sent ACK;176 means mailbox missing/out of range and has no
MailFailed ACK. The existing `Event::ServerError` also carries these informational codes.

Headers preserve type, status0unread/1read/2unpaid, sender/receiver, attachment count
(including money/billing), open date, returned and extra. Bodies preserve receiver,
title/text, three money fields, send/receive/open dates, and exactly ten optional item
slots. `MailAttachment.item` is the normal parsed Item; its private original Item.Write
bytes preserve skipped detail types for an exact native claim. Clone the chosen cached
attachment into TakeItem. Native3948d150 converts Lua1-based index to0-based;
393cc6e0 builds opcode09d via397ae5f0 and397c5390 serializes full Item.Write, followed by
two zero target address bytes. The lane shortened reader ignores every description field
after the item ID, so full equipment details still work (verified live). No metadata is
invented. Claims update inventory only through authoritative reason47 item tasks;
mail-finalization Remove may already name container5(MAIL), so the inventory reducer
removes that exact unique ID from visible containers. Money is reason83, separately.

Every inbound mail packet115..11f/121..123 is decoded. Failed and Sent contain exactly
ten slot-address pairs, without a count. Body contains ten full Item.Write records,
without a count. AttachmentTaken contains a count0..10 plus **ten** fixed ID/address
records; nonempty padding, duplicate IDs and invalid counts are rejected. Deleted always
contains unread counts even when its modified flag is false. All shapes have strict
DD04 next-opcode boundary coverage, including full equipment and skipped detail bytes.

`AttachmentTaken` can arrive once for money and once per item. Its money flag may be
true again on item ACKs; cached body values, not repeated subtraction, determine the
remaining count. Item locations are the old mail-escrow address, not the new bag slot.
With no cached body, the resource marks `dirty` and retains an unresolved claim until a
refresh; it does not invent the remaining attachments or a success. Load/read a body
before driving the native sequential claim UI. TakeAll claims without deletion;
TakeAllAndDelete uses opcode139 and waits for the authoritative Deleted ACK. Direct
Delete silently refuses sent mail and nonempty incoming mail in the lane. Return's
server opcodefff does not exist in1.2; there is no fake return command. PayHousingTax is
housing-tax payment only; generic COD and commercial/MIA sending are not implemented.
ReceiverOpened is decoded but unused by this lane; sent read status requires a list/detail
refresh rather than an invented live notification. Subject/text result codes are exposed,
but lane does not currently enforce their limits;
wire/string/frame sizes and malformed local requests are validated here. Silent refusal,
bag-full partial claims and general server errors need a UI timeout/retry; pending does
not fabricate a failed/successful response. `mail::result` exports raw codes0..30.

Original Lua contracts: TOGGLE_MAIL(doodadId,receiver_bool),
MAIL_INBOX_UPDATE(read,mailListKind), MAIL_SENTBOX_UPDATE(read,mailListKind),
MAIL_SENT_SUCCESS(), MAIL_RETURNED(), MAIL_INBOX_ITEM_TAKEN(index1..10),
MAIL_INBOX_MONEY_TAKEN(), MAIL_INBOX_ATTACHMENT_TAKEN_ALL(mailId),
MAIL_INBOX_TAX_PAID(). `X2Mail:SendMail` takes a table containing receiver/title/text,
gold/silver/copper strings, doodadId decimal string, type string, withReceiver string0/1.
Title-info tables use mail_id/mail_type/title/sender/receiver/is_read/hasItem/extra;
current-body adds text/sendDate/recvDate and synthesizes sender/extra from its header.
Physical attachment indices are one-based in Lua. List end is native MAIL_LIST_END3,
continue2; read native registration394914xx before adding other list-kind constants.

`cargo test -p aa-net --test live_mail -- --ignored --nocapture` passed32.40s with fresh
owned chars43/44 only. Sender sold its own starter gear for165c, paid exactly135c to send
5c and two real equipment attachments. Verified failures15/1 leave money/state intact,
registered immediate delivery, inbox/outbox list+detail, unread updates, individual full
native equipment claim, sequential claim-only, direct delete, claim-and-delete, and
recipient reconnect persistence both before and after claims. Both peers logged out.
A real mailbox fixture is template320 at(14367.94,15517.02,152.8604), beside merchant12547.
No grants, server/database writes outside normal gameplay, or existing accounts were used.
Normal delayed delivery, express100c, paid housing tax, commercial/MIA/system mail claims
and full-bag partial delivery remain source/unit audited rather than live exercised.

Native ping placement research (read-only follow-up): worldmap392d7a54 and roadmap392c0680
call392ac150?392ac030. Rect X is origin+u*extent, Y origin+(1-v)*extent; subtract origin
cell coordinates<<10 before local engine queries. Z is max(two engine height queries)+2;
the engine calls are vtable0x110(pos,0,0.0,0) and0x108(pos,FLT_MAX), likely surface/terrain
but exact symbols are unproven.392ac150 then transforms through world-service vtable0x28.
Worldmap uses own current instance only when the displayed map level/id matches own map
context, otherwise0; roadmap always uses own instance. Native does not use player Z.

## Player trade protocol and state

`Command::Trade(TradeCommand)` / `Event::Trade(TradeEvent)` expose all nine lane requests
and thirteen responses. `Trade` is `WorldState.trade` and a Bevy resource. Commands are
`Invite { obj_id }`, `Accept { obj_id }`, `Decline { obj_id, reason }`, `Cancel { reason }`,
`OfferItem { slot }`, `RemoveItem { slot }`, `OfferMoney { amount }`, `Lock { locked }`,
and `Confirm`. IDs here are24-bit **character object IDs**, not stored character IDs.
Offers use zero-based physical bag slots and copper. OfferItem always writes container2,
physical slot, amount0 (native full-stack sentinel). **The current lane transfers whole
stacks even if a positive smaller amount was requested.** There is no partial-stack API;
use existing SplitItem first and offer the resulting whole stack. Zero money withdraws
an offer; negative money and invalid object IDs are locally rejected without sending.

`Trade` holds active/partner_obj_id, incoming_invite/outgoing_invite, mine:Vec<TradeOffer>,
other:Vec<Item>, both money/lock/OK flags, pending, last_error, last_canceled(reason,by_me),
and last_settlement:Option<ItemTasks>. Own ACKs contain container/slot/amount but no item;
WorldState/NetPlugin seed each `TradeOffer.item:Option<Item>` from authoritative inventory
at the ACK, preserving exact item ID/count/grade/details. Missing inventory stays None,
not invented. `plan_offer(&Inventory,physical_slot)` rejects inactive/locked or previously confirmed trade, missing
or bound items, duplicates and the native ten-item cart limit; it yields a whole-stack
OfferItem. The raw wire server has fewer guards; use the helper for normal UI requests.
`can_confirm()` requires active, both locks, and own OK false, matching original UI.

No offer or balance changes optimistically. Requested identifies successful packet send;
only server offer ACKs update cart values. Changes to an offer while either side is locked
cause lane unlock_trade to clear both locks and both OK flags. The lane only sends
LockUpdate(false,false), so the reducer remembers the preceding authoritative edit and
uses that source-backed sequence to invalidate both confirmations. Explicit Lock(false)
is a narrower lane quirk: it clears both locks but leaves server OK flags unchanged; an
edit after both locks were explicitly cleared also skips unlock_trade's early-return reset.
The resource retains the last observed confirmation flags in that case. Do not pretend a
new confirmation round exists; cancel/reopen if the UI needs fresh consent after that path.
The lane's Confirm guard permits one locked side; the UI helper deliberately requires both,
as the original Lua does. Cancel, Started and Made reset the current cart/locks/pending;
last_canceled/last_settlement preserve the corresponding authoritative terminal report.
Character selection, leave and disconnect clear all trade state. Late offer/lock packets
outside an active trade are ignored. Invitation lifetime is60s server-side; silent refusal
or expiry requires UI timeout, not fabricated success. Decline has no ACK to the declining
client; local Requested closes its invitation and is not retained as an awaiting request.

SCTradeMade16a has the same reason/tasks/forced-removals/trailing-u32 shape as item tasks;
reason48 is TRADE. It is decoded strictly and the session emits `Event::ItemsChanged(tasks)`
**before** `Event::Trade(TradeEvent::Made(tasks))`, so inventory and balances are settled when
the UI receives completion. Made itself does not apply tasks twice. Full Item.Write is used
for remote offer/add/remove; settlement Add uses existing padded item task details. Unit
coverage checks all thirteen responses in one DD04 batch followed by another opcode,
malformed/truncated counts, command golden bytes, exact own/other item IDs and full counts,
edit/explicit-unlock behavior, lifecycle, and session settlement-notification order.

Original `trade/trade.lua` and `logic/trade.lua` contracts:

* `TRADE_CAN_START(unitIdStr)` calls `X2Trade:StartTrade(unitIdStr)` unless an exclusive
  window blocks it, then `CannotStartTrade(unitIdStr,TCR_OPEND_MONEY_WINDOW)`.
* `TRADE_STARTED(targetName)`, `TRADE_CANCELED()`, `TRADE_LOCKED()`,
  `TRADE_OTHER_LOCKED()`, `TRADE_UNLOCKED()`, `TRADE_OK()`, `TRADE_OTHER_OK()`, `TRADE_MADE()`.
* `TRADE_ITEM_PUTUP(inventoryIdx_one_based,amount)` and
  `TRADE_ITEM_TOOKDOWN(inventoryIdx_one_based)`; amount0 displays the bag's whole count.
* `TRADE_OTHER_ITEM_PUTUP(otherIdx_zero_based,template_id,stackCount,tooltip)`;
  `TRADE_OTHER_ITEM_TOOKDOWN(otherIdx_zero_based)` then compacts later indices.
  Native394ff4f0 emits the zero-based pre-insertion vector count returned by394ff490;
  type comes from Item+8(template), count from Item+0x10. The full tooltip is built there.
* Money events carry a decimal-string amount: TRADE_MONEY_PUTUP(money),
  TRADE_OTHER_MONEY_PUTUP(money). `PutupTradeMoneyByStr(gold,silver,copper)` converts to copper.

`X2Trade:PutupTradeItem(bagIndex_one_based,amount)` wrapper394b41b0 subtracts1 before
394febc0, which checks lock, ten entries, count and bound flag then writes0f0.
`TakeDownTradeItemByInventoryIdx` is called with inventoryIdx-1 (zero-based) in original Lua.
Other calls: CanStartTrade/StartTrade/CannotStartTrade, CancelTrade(reason), IsTradeLocked/IsOtherTradeLocked, LockTrade(),
OkTrade(), GetCurrencyForUserTrade(). The lane uses copper only, no AA-point trade currency.
`LockTrade()` is a toggle of the authoritative own lock, not an unconditional true. Native
registration394b4cd5 binds wrapper394b40d0 -> sender394fd5b0; it compares byte3a5ba538 with zero,
uses `sete` and writes CS0f3 with that bool. `IsTradeLocked` wrapper394b4280 reads the same manager
base3a5ba508+0x30; the other-lock wrapper394b4300 reads+0x18. Bind zero-argument Lua LockTrade to
`TradeCommand::Lock(!trade.my_locked)` and wait for a received lock update. The sender does not
flip its local byte; absent an active trade it emits native error0xec and sends nothing.
All original item lookup/tooltip addresses must respect bag sorting's virtual-to-physical map.

Lane limitations are explicit: faction/distance eligibility is TODO; several silent refusals
have only generic cancel reason0; bound/duplicate/cart/template tradeability guards are mostly
native-side; amount is not a split; offer/settlement failures are not an atomic rollback of
previous moves. Completion is only the actual task list, never a promise every offered item
was transferred. No server fixes, grants or alternate settlement behavior are hidden here.

`cargo test -p aa-net --test live_trade -- --ignored --nocapture` passed20.29s with fresh
chars53/54 only. Each sold its own starter shirt+pants for59c, bought two25c unbound supplies
and retained9c. Verified invite/decline/start, exact full stacks2/2, remove/reoffer,
edit-driven lock invalidation, cancel retaining all balances/items, both locks, first OK
without settlement, second OK moving exact IDs16777949/16777950 and A-2/B+2 net copper.
Partner logout canceled an active trade; reconnect preserved recipient item ID/count/money
and cleared the trade. Both clients logged out cleanly. Unit tests cover larger stacks,
force-remove/truncation, and explicit-unlock/OK quirk; full-bag/partial-failure settlement
remains source-audited, without grant fixtures.

## Auction protocol and escrow state

`Command::Auction(AuctionCommand)` / `Event::Auction(AuctionEvent)` expose all six lane
requests and five implemented responses. `Auction` is `WorldState.auction` and a Bevy
resource. Requests: `Search { auctioneer, query: AuctionSearch }`,
`Post { auctioneer, item_id, start_price, buyout_price, duration }`,
`Bid { auctioneer, lot: AuctionLot, bid: AuctionBid }`,
`Cancel { auctioneer, lot }`, `MyBids { auctioneer, page }`, and
`LowestPrice { auctioneer, item_template_id, grade }`. `AuctioneerIds` has obj_id/obj_id2,
both24-bit native object IDs; **every current lane handler ignores both IDs**, including
range/service eligibility. Zero IDs are accepted here for that documented lane behavior.
The app should still retain the actual active auctioneer context for the original UI.
Money is copper. Post duration0/1/2/3 means6/12/24/48hours. Prices are whole-stack totals,
not a per-item amount. Buyout0 disables direct purchase. No local inventory or money is
changed by a request or auction ACK; regular item tasks remain authoritative.

`AuctionSearch` preserves keyword/exact_match, raw grade, category_a/b/c, zero-based page,
seller stored-character client_id(0all), filter, world_id, min/max item levels(0unbounded),
sort_kind and sort_order. Sort kinds0default/1evaluated/2name/3level/4expiry/5bid/6buyout;
order0ascending/1descending. Evaluated/name sorts1/2 currently retain default ID order in
the lane (not implemented); expiry sort uses post_date. Search filter and
world_id are read but ignored. Grade0 is the lane wildcard, so raw Basic0 cannot be
specifically selected. LowestPrice also ignores grade while choosing its lot, merely
returning the requested grade; a zero-buyout lot may yield a zero quote. These responses
must not be presented as grade-specific or universally purchasable price guarantees.
Own listings use Search.client_id = PlayerStats.character_id; MyBids filters own bidder ID.
No other users' lots need to be bid on to exercise these APIs.

`Auction` holds last page/reported_count/lots/loaded/server_time/query, dirty, quote cache,
last_posted/last_bid/last_canceled, pending requests and last_error. Search and MyBids share
one response opcode; pending query requests are correlated FIFO. The response's count is
**the number of records in this page (0..9), not total matches or page count**. Empty results
return page0 even for another requested page; an out-of-range nonempty query returns its
requested page with count0. The raw header count remains reported_count even after later
ACKs remove a cached lot; lots.len() gives the current cache length. Do not infer total
matches. The reducer preserves the requested query separately from the returned page.
Post marks a query dirty rather than appending a lot to an arbitrary filtered result.
Normal bid ACK updates cached bidder fields only; cancellation removes the cached lot.
Incoming auction mail types13..17 mark auction data dirty. Selection/leave/disconnect reset
all auction state. No unsolicited bid-change broadcast exists; refresh for other players.

An `AuctionLot` contains exact lot ID, full typed item, seller/bidder IDs/names/worlds,
start/buyout/bid prices, duration, Unix-seconds post_date and extra. end_time() derives
expiry from duration. It retains original Item.Write bytes privately, including skipped
item details and reserved equipment bytes, so Bid/Cancel echo the **complete native lot**.
Use a cached lot clone and lot.bid(amount,world_id,stored_character_id,name); do not rebuild
an item from a template or discard details. from_wire() accepts a complete lot for recorded
fixtures. Local validation rejects changed typed item data that disagrees with the retained
wire bytes, invalid IDs/prices/durations/pages, mismatching bid lot ID, non-increasing bids,
invalid sort/grade/level ranges, signed string limits and oversized native request frames.
SC page counts, item descriptions and tails are decoded strictly, preserving DD04 alignment.
Undefined SCAuctionMessage133 has no lane writer and remains unsupported; no guessed body.

Post moves an owned, unbound physical bag stack into auction escrow(container6), task
reason46(AUCTION), preserving its original item ID/details. The listing deposit is
min(buyout * .01 * (duration+1),1000000), truncated to integer copper; listing_fee() exposes
this exact lane calculation. Money changes use reason83(DEPOSIT_MONEY). A zero-buyout lot
has zero deposit in this lane. Cancellation requires seller ownership and no bids; it sends
received AUC_OFF_CANCEL13 mail containing the **original** escrow item, without refunding
the deposit. Claim through the existing Mail commands; cancellation itself does not add it
back to the bag. Regular bids escrow the full new amount, then refund the previous high bid
by mail, including when a bidder increases its own bid. Canonical bidder identity is read
from the active character by the server, never trusted from the request copy.

A buyout charges the lot's actual buyout price, refunds any previous bid through received
mail type15(AUC_OFF_FAIL, an inherited lane/C# naming quirk), sends the original item in
AUC_BID_WIN16 mail, and sends the seller AUC_OFF_SUCCESS14 mail with floor(price*.9)copper.
The listing deposit is **not** returned. Seller money claims also require one labor point.
The lane sends no SCAuctionBid(is_buyout=true) ACK. `Auction.last_purchase_mail` records
lot/mail/item IDs and clears the pending buyout only when a received win-mail **body**
contains the exact pending escrow ItemID, uniquely identifying a pending lot. Multiple
stale requests for different lots of the same relisted ItemID remain unresolved: the mail
has no LotID and cannot distinguish them. A header, debit, empty search result or sent-mail
body is not purchase confirmation. Actual possession still waits for authoritative mail
claim item tasks. Unknown/general errors and silent refusals remain pending for UI timeout
and refresh; they are never turned into invented success or attributed to an unrelated
auction command. Expiry uses the server's periodic auction task and eventual mail; the
client does not run an optimistic expiry settlement.

Original `x2ui/auction` contracts and native x2game12 trace:

* SearchAuctionArticle(page,minLevel,maxLevel,itemGrade,itemCategory,isMatchWord,itemName)
  wrapper39449b20 subtracts1 from Lua page; manager394cec90 clamps negatives to0 and writes
  grade=itemGrade-1. Original GetItemGrade() increments selector indices greater than1.
  Categories split A=category/10000,B=(category/100)%100,C=category%100.
* SearchAuctionArticleByPage39449bc0 / SearchMyAuctionArticles39449bf0 subtract1; managers
 394ceec0/394cedf0 clamp negative pages. The original initial SearchMyAuctionArticles(0)
  resets and requests the first page. SearchMyBidList39449c20 likewise subtracts1.
* Native GetSearchedItemPage returns wire page+1. GetSearchedItemCount3944a2b0 returns cached
  record count. GetSearchedItemTotalCount3944a340 returns literal50.0(.rdata3999ddc8), the
  native maximum-page bound also checked at394ce9ab; it is not a total from this lane.
  common.lua derives navigation from that bound, current page and returned record count.
* GetSearchedItemInfo(idx1based)3944a3c0 returns the normal item-tip table plus idx,
  articleId(decimal u64 string), stackCount, seller, selWorldId, bidder, bidWorldId,
  leftTime(seconds), bidPriceStr, depositStr, directPriceStr, extra.
* BidAuctionArticle(articleIdStr,moneyStr)39449d80 parses the ID with39424890 and amount
  with atoi, then394cfcb0 copies the complete cached lot and bid into397b0f30. DirectPurchase-
  AuctionArticle39449dd0/394cff70 looks up that lot's buyout and calls the same bid path.
  SetDuration39449cb0 converts Lua1..4 to wire0..3. Monetary UI fields are decimal strings.
* AUCTION_ITEM_PUT_UP(itemName), AUCTION_ITEM_SEARCH(), AUCTION_ITEM_SEARCHED(),
  AUCTION_BIDDED(itemName,moneyStr), AUCTION_CANCELED(itemName),
  AUCTION_LOWEST_PRICE(itemType,grade,price). No lane-backed bought notification is
  fabricated where its auction ACK is absent. The app can refresh mail on a win header,
  then show completion after the exact body proof.

Auction validation: seven source-backed unit tests cover all six CS requests, all five SC
responses and every truncation in a DD04 sequence followed by another opcode, invalid page
counts, exact equipment/reserved/skipped item-detail echoes, query FIFO/count semantics,
local rejection, no optimistic escrow or purchase, wrong-item/outbox mail rejection and
lifecycle reset. Full cargo test -p aa-net passed161 library and2 scripted-session tests,
with9 opt-in live tests ignored; cargo check -p aa-net --features bevy --examples passed.
The ignored live_auction test additionally requires the environment flag
AA_NET_RUN_AUCTION_MONEY_CLAIMS=1; do not enable it before resolving the lane issue below.

Partial live run (2026-10-17 19:18 EDT, fresh owned characters56/57,
netauction1792279107856a/b) funded181c/207c solely by their starter gear. It verified listing,
exact full equipment ItemID16777962/details, own/peer searches, lowest-price response,
cancel ACK, full native cancellation-mail claim returning the same item and retaining the
1c listing deposit, repost, normal10c bid ACK, my-bids bidder identity,100c buyout debit,
win/refund mail delivery and refund body with10c. It then **timed out on the refund's
TakeAllAndDelete request**, followed by both LeaveWorld ACK timeouts. TCP clients were
dropped; reconnect of the first owned account was denied as still connected. Root's
independent clients47/55 subsequently also timed out entering this lane. No further claims
or fresh accounts were attempted; the recovery login issued no game commands. No server
process was restarted or modified. Combined refund claim/delete, buyer win-item claim, seller90c claim,
post-buyout search, saved balances/item identity and graceful final logout are **not live
verified**. The complete ignored fixture retains those assertions for a later functional
lane; no success is fabricated here.

Read-only diagnosis: every claim starts ItemTxn::begin and takes economy System.ops, a
lane-wide mutex (inventory.rs:67, economy/mod.rs:129), so a stuck operation can affect all
economy work. SaveManager.capture_financial_snapshot takes auction/housing/economy locks,
and timed_rewards::do_add_labor takes the save transaction lock before game entry. These
paths explain a possible broader login/save cascade; they do not prove the exact held
lock. WARN-level logs show the normal bid and buyout request but no refund-handler trace.
The refund code has no obvious recursive lock in its straight-line claim/delete path.
A read-only query of private MySQL3317 filtered to lane_bevy_* found no active/waiting SQL
or InnoDB transaction. Exact runtime blocking call and current in-memory lot state remain
unproven; no server fix, new risky probe, grant, alternate settlement or restart is hidden.

## Party loot rolls

`Command::LootRoll(LootRollCommand { key, roll })` sends the native loot-dice request;
`Event::LootRoll(LootRollEvent)` reports offers, notifications, summaries, successful local
send (`Requested`) and local validation failures (`RequestRejected`). `LootRolls` is both
`WorldState.loot_rolls` and a Bevy resource. Its public rounds retain the offered full item,
latest notifications per character name, raw summary character IDs/dice, requested choice,
pending status, own acknowledged roll, reported own take and matching loot failure.

`LootRollKey { owner_type, owner_obj_id, item_index }` packs to the temporary 64-bit corpse
ItemID: `(owner_obj_id << 32) | (owner_type << 16) | item_index`. NPC type 1 and doodad type 2
are supported; owner IDs must be nonzero 24-bit IDs and indices nonzero 16-bit values. This
key is **not an inventory ItemID**. Server acquisition can create or stack real inventory
items, whose identity and quantity come only from ordinary authoritative item tasks.

Exact native/lane wire, in `systems/economy/loot_packets.rs`:

| Packet | Body |
|---|---|
| CS091 LootDice | packed `u64` key, `bool roll`; exactly 9 bytes |
| SC0fb LootDice | full non-null `Item.Write` |
| SC0fc LootDiceNotify | string characterName, full non-null `Item.Write`, signed `i8 dice` |
| SC0fd LootDiceSummary | packed `u64` key, signed `i32 count`, count rows of `u32 characterId, i8 dice` |

The count is bounded by available bytes before allocation; invalid/duplicate character IDs,
invalid owners, null items and truncated details fail decoding. There is no invented party
size limit: the lane can append rollers outside its original eligible list. Signed dice
values are preserved; the normal lane generates 1..99 for a roll, -1 for a pass and uses 0
internally for no answer. An empty summary is valid. All three SC bodies consume only their
own fields in a lengthless DD04 batch, so the next packet remains aligned.

Original `x2ui/loot/loot_dice.lua` and native x2game12 contract:

* `TRY_LOOT_DICE(key, timeStamp, itemLink)` creates `LootDiceWindow(%d)` with an opaque
  local integer key. `itemLink` preserves the offered item's full details for
  `X2Item:InfoFromLink`; it must not be replaced with a template-only link. Native
  RequestNext394fad50 calls394f94d0, which passes the queued item at row+8 to the same
  chat-link formatter394e4ef0 (call394f9502), then emits event0x85. This uses that formatter's
  normalized id0/count1 representation; the raw offered item/key remain separate for
  the network request and settlement.
* `X2Loot:DoDiceAction(key, roll)` takes that local key, not the packed 64-bit corpse ID.
  True rolls; false passes. Roll, Give Up, close and the 60-second deadline all use this
  function. The Lua then schedules `X2Loot:RequestNextDiceItem()` after 500ms and animates
  away the prompt. RequestNextDiceItem is a local queue operation, with no CS packet.
* `LootRolls.next_prompt_at(now)` / `next_prompt()` return and mark one unshown local
  prompt among the first `max_concurrent` queued rows. Default 3 matches native
  manager 394fb250. `set_max_concurrent(i32)` clamps to at least 1, matching wrapper 3948a610.
  Its exact Lua name is `X2Loot:SetMaxConcurrentNotifyDiceItem(count)` (native registration
  3948b130 links string399df518 to that wrapper). The setter itself neither advances the
  queue nor sends a packet.
* On `Offered`, call next_prompt and emit TRY_LOOT_DICE with its `ui_key`, full item link
  from the matching round and `prompt.ui_timestamp_ms(X2Time:GetUiMsec(), now)`.
  This preserves receipt time while using the application's UI clock origin.
  RequestNextDiceItem repeats that operation. `plan_action(ui_key, roll)` translates to
  the exact network command and rejects finished, submitted or unknown local keys.
* Native DoDiceAction 3948a5c0 -> 394fa770 removes its queue row after sending; request
  writer 397ba120 serializes only the full 8-byte key and bool. `Requested` likewise removes
  the local prompt and records pending, without inventing a server response.
* RequestNext manager 394fad50 prunes prompts only when older than 60000ms, then exposes
  one eligible unshown row. The resource follows this timing and does **not** automatically
  pass: the original Lua owns its deadline action. `plan_action` permits that deadline pass
  until queue pruning or an authoritative lifecycle event invalidates the key.
* `LOOT_DICE(characterName, itemLink, dice)` is emitted for SC0fc (native 391d9a10).
  `x2ui/chat/chat_msg_event.lua` formats every negative dice as a pass, otherwise a roll.
  Native summary handler 391d9b50 clears the matching loot-bag request bit so ordinary loot
  can retry; it emits no Lua summary/winner event.

Active duplicate offers are deliberately deduplicated by packed key, preserving the first
receipt time; native would enqueue each repeated packet. A new offer after a completed
round starts a fresh opaque local key, allowing an all-pass retry. The resource also
invalidates prompts on summary, recipient take, typed owner removal, no-loot state,
character selection, world leave and disconnect. Unknown server errors do not silently
acknowledge a roll. A matching loot failure is retained without assuming it acknowledges
the roll or changes inventory. Name-only notifications are never guessed to map to another
character's numeric summary ID.

Lane limits from `looting.rs`/`loot.rs`: no distance, eligible-member or duplicate-choice
guard is enforced by do_player_roll; a second raw request can replace a roll and an
outsider can be appended. The public UI planner prevents repeat local submissions. A
container's 180-second make-public deadline forces unanswered 0 entries to pass; this is
separate from the original Lua's 60-second prompt. **Summary is not winner proof**: the
lane currently takes the minimum positive value and includes every roll greater/equal to
that minimum before random choice, effectively choosing among all positive rollers.
No highest-roll winner is fabricated. All-pass summaries leave the item available for a
new attempt. BagFull keeps the reservation for ordinary loot retry. SCLootItemTook is sent
only to the actual recipient; it marks `taken_by_me`, while normal item tasks carry the
resulting inventory identities. No packet here proves another player's actual acquisition.

Validation: eight offline tests cover the exact CS bytes and rejection rules, all three SC
packets followed by another loot packet in DD04, every truncation, variable item detail,
count bombs/duplicate IDs, signed results, prompt concurrency/timestamps/deadlines,
all-pass reopening, typed owner loss, lifecycle reset, silent errors and separation of
summary/take/actual inventory tasks. Full `cargo test -p aa-net` passed 169 library tests and 2
scripted-session tests, with 9 opt-in live tests ignored. The Bevy/example check
`cargo check -p aa-net --features bevy --examples` also passed.
Natural-loot paired live QA remains **pending** while
the lane is stalled; this milestone makes no new lane connections or claims.

## Crafting and gathering proficiency

`Command::Actability(ActabilityCommand)` supports `Upgrade { id, auto_use_aa_point }`,
`Downgrade { id, auto_use_aa_point }` and `Expand`. `Event::Actability(ActabilityEvent)`
reports `Snapshot { last, entries }`, `TierChanged { is_upgrade, id, step }`,
`Expanded { count }`, successful local sends (`Requested`) and local validation failures
(`RequestRejected`). `Actabilities` is both `WorldState.actabilities` and a Bevy resource;
it exposes `entries`, `loaded`, `expanded_expert: Option<u8>`, `pending`, `last_error`,
`get(id)` and `count_at_step(step)`. An entry has `id`, `point: Option<i32>` and `step`.
This proficiency is separate from combat ability experience.

Exact lane packets from `packets/world.rs`, `systems/services/expands.rs` and the active
transactional expansion handler in `systems/economy/store.rs`:

| Packet | Body |
|---|---|
| CS0ff UpgradeExpertLimit / CS100 DowngradeExpertLimit | u32 actabilityId, bool autoUseAAPoint |
| CS101 ExpandExpert | empty |
| SC136 Actability | bool last, u8 count, count rows of u32 id, i32 absolutePoint, u8 step |
| SC1c0 ExpertLimitModified | bool isUpgrade, u32 id, u8 step |
| SC1c1 ExpertExpanded | u8 expandedCount |

Snapshots replace the old list on the first chunk, accumulate through `last=true` and
mark `loaded` only then; an empty completed list clears old entries. Counts are bounded
before allocation, zero/duplicate IDs within a packet fail decoding, and all signed
points / u8 tiers are preserved without an invented maximum tier. New characters get
one zero-point row per `actability_groups` entry (34 IDs in the supplied lane database).
`CharacterState.expanded_expert` now exposes the previously skipped final byte and seeds
the expansion count. Character selection, world leave and disconnect reset stale state.

**`LaborChanged.point` is a delta**, not the new total. Active
`economy/inventory.rs::change_labor` sends `a.point - previous` after the server caps the
gain, with `action=actabilityId` and the current `step`. The resource adds it to a known
snapshot base. A missing base stays unknown, rather than pretending a gain is a total.
Normal labor regeneration (`action=0`) creates no proficiency row.

Expert changes wait for matching authoritative ACKs; sending never changes tier, points,
currency, inventory or expansion count. Upgrade preserves known points. Downgrade changes
the tier but sets points to `None`: the lane resets them to
`expert_limits ORDER BY up_limit ASC[newStep].up_limit` and **does not send that value**.
A client with the matching game-data table can derive it; subsequent deltas alone cannot
recover an unknown base. Generic/silent server errors leave requests pending, and the
resource never assigns unrelated errors to a specific request. The wire's auto-use flag
is preserved although the lane ignores it when regrading. The active expand handler
requires vocation and any certificate but currently **adds** its vocation cost instead
of subtracting it (inherited C# bug). Only the real game-point/item tasks update those
resources; the expansion ACK changes only the returned count.

Original Lua contracts are in `x2ui/skill/tab_actability.lua` and
`actability_info_notify.lua`: `ACTABILITY_REFRESH_ALL()` for each received snapshot packet,
`ACTABILITY_EXPERT_CHANGED(id, localizedName, actualCappedDelta, total)` for point changes,
`ACTABILITY_EXPERT_GRADE_CHANGED(id)` for regrading and
`ACTABILITY_EXPERT_EXPANDED()` for capacity changes. `X2Ability:UpgradeExpert(id)` and
`:DowngradeExpert(id)` take one Lua argument; native wrappers39438840/39438870 obtain
the wire bool via settings getter397ccd50 (`+0xc340 == 1`), then call393101c0/39310080.
`X2Ability:ExpandExpert()` takes no argument. Names/icons/categories and grade/cap/cost
tables are client data (`actability_groups`, `actability_categories`, `expert_limits`,
`expand_expert_limits`); they are not fabricated from these packets.

Six new offline tests cover all three CS commands, all three SC responses followed by
another DD04 opcode, every truncation, bad counts/IDs, chunk and empty snapshots,
selection/logout reset, signed deltas, unknown downgraded totals, matching ACK correlation,
silent errors and no optimistic currency/tier changes. The existing character-state test
now verifies a nonzero expanded-expert byte. Full `cargo test -p aa-net` passed 183 library
and 2 scripted-session tests, with 9 live tests ignored;
`cargo check -p aa-net --features bevy --examples` passed. No live lane traffic was used.

## Doodad, mailbox and recipe source audit

Original `x2ui/interaction.lua` builds a local menu through
`X2Interaction:GetInteractionSkillCount()`, `GetInteractionSkillId(k)` /
`GetInteractionSkillName(k)` (Lua indices start at1), then
`SelectInteraction(interactionId)` or `CancelInteraction()`. `INTERACTION_START()` and
`INTERACTION_END()` take no arguments. The session exposes unit, position and explicit
doodad-target skill casts (see "Doodad skill targets and phase state"); the native local
menu planner remains a gap.
Many gathering interactions run `CSStartSkill` against the doodad, then ordinary skill,
labor, item, loot and phase packets; they do not use the NPC service-list request.
Lane `CSStartInteraction068` only handles NPCs/mates. SC07e/07f world-interaction list /
cancel exist only as opcode declarations here, with no lane writer to justify a guessed
decoder or an invented successful doodad interaction ACK.

Mailbox Lua `mailbox/mail/mailbox.lua` listens to `TOGGLE_MAIL(doodadId, receiver)` and
stores `MAIL_DOODADID` / `MAIL_WITHRECEIVER`. Native local handler393b1040 supplies
**numeric doodad object ID, boolean true** (not a receiver name): target+0x7c is converted
to a Lua number; it calls393b7b10(4, objectId), emits event0xce via39424260 with those two
arguments, then393ccdd0 cleans up the local action. There is no send in this handler.
393b7b10 stores active interaction kind/object and ends a previous one if needed.
The old-context end path393b7aa0 emits INTERACTION_END (event0xd2) with no arguments
and clears the interaction manager's local selection.
The complete dispatch-table index/skill path reaching this handler remains untraced.
`INTERACTION_END()` clears both globals and closes the window. `write_mail.lua` serializes
the bool as `withReceiver="1"` or `"0"`. This local UI context does not prove server-side
send permission; existing mail send/claim commands remain authoritative. The lane's
`DoodadFuncNaviOpenMailbox` and `CraftStart` belong to its logging-only TODO group.

Recipe catalogs are static `crafts`, `craft_materials`, `craft_products` and related
group/category tables, not a server recipe-list message. `economy/craft.rs` defines
CSExecuteCraft0f8 as u32 craftId, bc doodadObjId, i32 count (11 bytes). The server casts
the recipe's skill on the doodad; its CraftEffect consumes materials/produces items,
charges labor through the skill system and schedules another cast while count remains.
SCCraftFailed1bf has a writer (u32 id, i32 count, count u32 craftIds) but is never sent;
there is no normal batch-completion ACK. Original Lua uses
`CRAFTING_START(doodadId,count)`, `CRAFT_STARTED(leftCount)` and `CRAFT_ENDED(leftCount)`;
these cannot be mapped to fabricated server completion events. The bounded request and
native cast audit are described under "Recipe batch requests and native presentation";
proficiency support does not grant or pretend a completed craft. Natural gathering/crafting
and expert-change live QA remain pending while the lane is stalled.

## Doodad skill targets and phase state

`Command::UseDoodadSkill { skill_id, doodad_obj_id }` uses the existing CSStartSkill (0x052)
flow with a unit caster and **target kind 4**, rather than the kind 0 written by UseSkill.
Its exact 13-byte body is u32 skillId, u8 casterKind=0, bc playerObjId, u8 targetKind=4,
bc doodadObjId, u8 flag=0 (no skill object). The checked helper
`SkillTarget::doodad(id) -> Result<SkillTarget, &'static str>` rejects 0 and IDs above
0xffffff before a three-byte write can truncate them. The session also rejects skill 0,
reporting local `DoodadSkillRejected { skill_id, doodad_obj_id, message }`; this is not
a server SkillStarted result. A valid request outside InWorld is ignored like other
skill commands. Target 0 never means self for this command.

There is no client learned-skill filter for gathering/service interactions. The server
must validate the target, distance, permission, labor, item and skill requirements.
Sending does not create a cast, consume items/labor, change a phase or announce successful
interaction/crafting. Existing authoritative SkillStarted/Fired/Ended, skill failures,
LaborChanged, item tasks and doodad/loot events remain the only state changes. This command
does not claim the still-untraced native mailbox/interaction menu dispatch is complete.

SCDoodadPhaseChanged (0x10f) has the exact 19-byte lane layout from
`systems/npcs/doodad/packets.rs`: bc objectId, u32 funcGroupId, u32 timeLeft,
i32 puzzleGroup, u32 itemTemplateId. The old reader consumed only the first 7 bytes,
leaving 12 bytes to be mistaken for another opcode in DD04. It now consumes and exposes
all fields in `ServerMsg::DoodadPhaseChanged` and `Event::DoodadPhaseChanged`
(`obj_id, func_group_id, time_left, puzzle_group, item_template_id`). The lane currently
writes puzzleGroup=-1; the decoder preserves the signed wire value instead of guessing.

`Doodads` updates those four fields only on an already-known object, preserving identity,
position, owner and plant time. Zero item/time values clear previous values. Unknown or
removed objects do not reappear from a phase update. `time_left` is the last announced
remaining milliseconds, not an automatically decremented timer; elapsed client time
does not fabricate a server growth phase or product.

SCVegetationCutdowning (0x081) is the server's Cutdown interaction broadcast: bc unitObjId,
bc doodadObjId, exactly 6 bytes. AAEmu's encoder (`systems/npcs/doodad/interactions.rs`,
CUTDOWN) and its `cutdown_broadcasts_cutdowning` test pin unit 0x000100/doodad 0x000200 to
`00 01 00 00 02 00`. `ServerMsg::VegetationCutdown` and `Event::VegetationCutdown` preserve
both ids and never mutate `Doodads`; the client's `ingame/cutdown.rs` consumes the event for
the fall (see docs/aa-client-doodads.md). The server sends the phase change produced by the
same use before this broadcast; the 2026-10-30 original-client capture observed, in one batch and
in this order, `SCDoodadPhaseChanged` to 6251, `SCErrorMsg` 181 (`BAG_INVALID_ITEM`, "Invalid
item."; the capture does not say why the lane sent it) and then the 6-byte body (unit 25178,
doodad 2543); `SCDoodadRemoved` followed 2367 ms after the cut.

Four new offline tests cover exact kind 4 bytes and ID boundaries, a scripted local game
connection proving invalid/out-of-world commands send no packet and successful sends
wait for the real SkillStarted response, the complete lane phase golden followed by
another DD04 opcode, every phase truncation, and reducer updates/clears/removal behavior.
The scripted connection uses an ephemeral loopback port, never the live lane.
Full `cargo test -p aa-net` passed 187 library and 2 scripted-session tests, with 9 live
tests ignored; `cargo check -p aa-net --features bevy --examples` passed.
Natural gathering/interaction live QA remains pending while the lane is stalled.

Adjacent doodad packets now preserve their complete lane layouts in both `ServerMsg`
and `Event`:

* `DoodadChanged { obj_id, data }` (SC10e): bc objectId, i32 data (7 bytes).
* `DoodadOriginator { obj_id, new_owner_id, new_faction }` (SC114): bc objectId,
  u32 stored ownerId, u32 faction (11 bytes).
* `DoodadSound { obj_id, sound_id }` (SC156): bc objectId, u32 soundId (7 bytes).
* `DoodadPhaseMsg { obj_id, message_id }` (SC157): bc objectId, i32 message id (7 bytes).
  The 10.0 serializer names the second field `type` (`SCDoodadPhaseMsgPacket.cs`); the 1.2
  handler 391dc720 passes it to 3902e0f0, which resolves the phase's `phase_msg`, so this
  client reads it as the `doodad_func_groups.id` candidate. The lane server declares the
  opcode but never writes it: no captured doodad sent it (2026-10-30, the arena countdown
  doodad 4540 forced into phase 11127 sent only `SCDoodadPhaseChanged`), so the meaning is
  still capture-pending. No reducer changes: the message is transient and only
  `Event::DoodadPhaseMsg` is forwarded.

`Doodads` applies changed data and the stored `owner_id` only to known objects.
The owner is a full u32 stored character ID, not the separate three-byte owner object ID;
the reducer does not infer parent, owner type, permissions, phase or faction state.
Zero owner/data values clear those fields. Unknown and removed objects stay absent.
Faction and sound IDs remain raw events for consumers to interpret; a sound does not
mutate the doodad or invent its position. Audio consumers can resolve a currently known
object through `Doodads`.

Three additional offline tests check the exact source goldens, every truncation, all
three updates followed by phase and another DD04 opcode, signed/full-width event
forwarding, known-only updates, clearing and preservation of unrelated fields. The
SCDoodadPhaseMsg golden (bc + i32, `i32::MIN` as well) is part of the mutation packet
test and the session batch test.
Full `cargo test -p aa-net` passed 190 library and 2 scripted-session tests, with 9 live
tests ignored; `cargo check -p aa-net --features bevy --examples` passed.
No live lane traffic was used for these updates.

## Recipe batch requests and native presentation

`Command::ExecuteCraft { craft_id: u32, doodad_obj_id: u32, count: i32 }` submits one
CSExecuteCraft (0x0f8): u32 recipeId, bc doodadObjId, i32 count, exactly 11 body bytes.
The session rejects recipe 0, doodad 0 / IDs above 0xffffff, and nonpositive counts before
anything reaches the socket. Positive counts retain their full i32 value, including the
maximum; the server owns material, bench, range, permission, proficiency and labor checks.
Like interaction casts, valid requests outside InWorld are ignored. The low-level
`game::ClientMsg::ExecuteCraft` writer is unchecked like the other low-level messages.

`Event::CraftRequested { craft_id, doodad_obj_id, count }` reports only a successful local
socket write. `CraftRejected { craft_id, doodad_obj_id, count, message }` reports only
local validation, inactive-character or missing/failed transport write failure. No ACK or
completion event is fabricated for a generic server error or silence. There is no new authoritative batch resource, completed-product
counter or optimistic material/labor mutation. Ordinary SkillStarted/Fired/Ended, errors,
item tasks and LaborChanged continue to carry the real server observations.

The command does **not** send a second CSStartSkill. Native
`X2Craft:ExecuteBatchCraftByType(recipeId, doodadObjId, count)` wrapper39460930 calls
manager394d7650, validator394d6dc0 and sender394d1340. Sender394d1340 builds opcode0xf8
with all three original arguments; constructor397ac9e0 and serializer397bbfc0 confirm
the complete count is sent once. The lane's `economy/craft.rs::craft` starts the recipe
skill with a doodad target through EconomyComposite::start_craft_cast; its CraftEffect
then performs material/product settlement and schedules the next batch cast itself.

The original `x2ui/crafting/crafting_renewal.lua` presentation contract is local:

* `CRAFTING_START(doodadId, count)` opens the bench UI. Its `SetDoodad` treats nil / zero
  context or count as recipe-book mode; this does not constitute a server execution ACK.
* `GetLeftBatchCount()` reads native manager+8; `IsWorkingCraft()` tests whether it is >0.
* Native callback394d53e0 listens to internal Started0x14, Fired0x15, Ended0x16 and
  Stopped0x17. It filters the local unit and a skill containing CraftEffect (type0x0e).
  The first successful Started emits `CRAFT_STARTED(initialLeftCount)` and clears a local
  fired flag. A successful Fired sets the flag; Ended decrements the local count only
  after Fired was seen. It emits `CRAFT_ENDED(remaining)` while positive, otherwise
  `CRAFT_ENDED()` with no argument. Failed Started and Stopped also end with no argument.
  These UI counters describe observed skill sequencing, not proof of received products.

`X2Craft:StopBatchCrafting()` wrapper39460870 invokes manager394d1870(false).
It calls393966d0 /393967c0 to cancel active cast slots (+0x3ac4 /+0x3acc, including a
mounted unit when present) using existing CSStopCasting054; their exact slot labels are
not inferred here. The false argument does not immediately zero the batch counter;
native callback/update processing resets it. The session's existing `StopCasting` names
plain and plot timelines explicitly, from authoritative skill events. There is no separate
guessed craft-cancel packet.

The lane's skills/handlers.rs::stop_casting cancels the matching current skill task and
skills/skill.rs::stop reports casting stopped / ends that skill. It does not call economy
`cancel_craft` or invalidate an already scheduled `CraftTask`. The task captures recipe,
doodad and count, and its execute checks only captured count >0 before invoking craft
again. Consequently canceling a current cast cannot promise cancellation of a queued next
cast; ordinary SkillEnded also cannot be promoted to a whole-batch completion ACK.

SCCraftFailed1bf now has the separate receive-only `CraftFailureNotice` contract documented
below. It retains the native raw header and bounded u32 list without treating the lane's
misleading repeated `craft_id` field as proven recipe semantics. No lane call site currently
sends it. Static recipe catalogs and native UI presentation counters remain aa-client work.

Two new offline tests check exact native min/full-width/max-count bodies and complete
frames, then an ephemeral loopback session proves invalid/out-of-world requests send
nothing, no extra CSStartSkill precedes a batch, a local send emits only CraftRequested,
and server refusal/silence never fabricates acceptance, products, labor or completion.
Full `cargo test -p aa-net` passed 192 library and 2 scripted-session tests, with 9 live
tests ignored; `cargo check -p aa-net --features bevy --examples` passed.
No live lane traffic was used; natural crafting and queued-cancel QA remain pending while
the lane is stalled.

## Owned mate lifecycle and transport state boundaries

`Event::Mate(MateEvent)` and the corresponding `ServerMsg::Mate` now decode three packets
which previously stopped DD04 before following unit state/points:

* `Spawned(MateSpawn)` (SC125), owner-only: u16 tlId, u32 stored mateId, u64 summonItemId,
  u8 userState, i32 experience, i32 mileage, u32 spawnDelayTime, **ten u32 skill slots**.
  This is exactly 67 bytes, with no skill count and no object ID. Native serializer397b54e0
  calls3989a910, whose loop reads exactly ten slots. Preserve zero/sparse slots.
* `State { obj_id, dynamic }` (SC1ac): bc objectId plus the shared dynamic state below.
* `SlaveState(SlaveState)` (SC1ab): bc objectId, u16 tlId, u64 rawType, shared dynamic
  state, string creatorName, **u32** ownerObjId, u32 dbId. The lane writes rawType0 and
  zero dynamic counts; all fields remain available without inferring local slave ownership.

`DynamicUnitState { skills: Vec<[u32;3]>, tags: Vec<[u32;3]> }` preserves both lists as
raw triples. Native397b7790 /397b7670 share397b2350, which reads i32 skillCount, that many
three-u32 entries, i32 tagCount and that many three-u32 entries. Each inline native array
holds 150 entries (second count offset0x70c). Counts must be nonnegative, <=150 and fit the
remaining bytes before allocation. Triple field meanings remain unverified; no guessed
skill levels, cooldowns, learned skills or active tags are created from them.

`Mates` is available as a Bevy resource and `WorldState.mates`. Its public
`own: Option<OwnedMate>` holds `spawn: MateSpawn`, `obj_id: Option<u32>` and
`dynamic: Option<DynamicUnitState>`. The spawn packet establishes owner-only metadata;
an actual UnitSpawned must then match both the timeline and the selected stored character
ID through `UnitKind::Mate.owner_id` before obj_id is set. The source assigns that owner
field from `owner.data().id`, not the player's runtime object ID. Either arrival order is
supported. A visible mate alone does not prove ownership. Ambiguous matches remain unset;
unknown dynamic packets never create a unit or binding.

Matched removal, object-ID reuse for another kind/owner/timeline, a new character selection,
logout, disconnect and kick clear stale owned state. A removal from a previous spawn does
not erase a replacement still awaiting its object. Matching repeated UnitSpawned preserves
its dynamic metadata. Ordinary unit HP/MP, equipment, buffs and movement remain in their
existing resources; `spawn` values are the last owner spawn announcement, not invented live
XP/state counters. SlaveState is forwarded as raw visibility metadata and does not create
an owned-slave resource or a Unit.

The lane fills spawn skill slots with distinct `npc_mount_skills.mount_skill_id` in table
order. These are `mount_skills.id`, **not combat skill IDs**: the matching compact database
joins mount skill209 to skill21172, for example. Activation still needs the native slot,
attachment and caster rules; the receive layer makes no such choices. The current compact
catalog has at most nine distinct entries for an NPC. The lane writer nevertheless emits
every vector entry if there are more than ten, with no count; that unsupported overflow
cannot be safely distinguished from a following DD04 opcode. The decoder follows the
native fixed-ten format rather than guessing an extra slot count.

Original Lua contracts from `x2ui/hud/pet_action_bar/pet.lua` and `unitframe/pet.lua`:
`X2Mate:IsPlayerPetExists()` gates the bar/frame; `playerpet` is the unit alias.
`GetNumMountSkills()` supplies the action slots; `PetState()` returns the mode used by the
Lua aggressive/protective/passive/idle mapping. `DismissPet()`, `MountMate()`,
`UnMountMate()` and `KickPassenger()` take no Lua arguments. `SPAWN_PET()` and
`DISMISS_PET()` have no arguments (native emits0x110 at3933e523 and0x111 at3933f0c5 /
3933fa28). Owner packet handler391db180 only inserts metadata through3933fd00; later
visible unit binding emits SPAWN_PET. UI integration should therefore wait for a resolved
obj_id. `MOUNT_PET(isMyPet)` / `UNMOUNT_PET(isMyPet)` take a bool; passenger mount/unmount
events have no arguments. No UI success event is synthesized by this receive layer.

Source audit of the remaining actions: summon uses the existing item-caster CSStartSkill
and SpawnPet/SpawnSlave special effects, not a new mate-summon request. CSRemoveMate0a4
is u16 tl; CSChangeMateTarget0a5 is u16 tl +bc target; CSChangeMateName0a6 is u16 tl +string;
CSMountMate0a7 /CSUnMountMate0a8 are u16 tl +u8 attachPoint +u8 reason;
CSChangeMateUserState0aa is u16 tl +u8 state and has no lane ACK. Mate equipment0a9 is a
separate full-item transaction contract. Slave info/equipment Lua uses **X2SiegeWeapon**
(`GetMountedSiegeWeaponInfo`, `GetVehicleInfo`, `GetSlaveEquipSlotInfo`,
`PickEquipSlotOfSlave`, `UninstallEquipSlotOfSlave`); this is not an invented X2Slave API.
Lane slave equipment/repair requests are logging-only stubs, and slave target has no
registered 1.2 opcode. Slave created/removed/despawn/bound/my-slave/escape packets and
owned-slave state remain subsequent bounded work.

Five new offline tests cover the source spawn golden, every packet truncation, native
positive dynamic lists, both count limits and 150-entry lists, full-width raw fields,
following DD04 UnitPoints, event/resource forwarding, either ownership packet order,
remote/mismatched mates, unknown updates, replacement, ambiguous TLs, object-ID reuse and
session cleanup. `cargo test -p aa-net` passed 197 library and 2 scripted-session tests,
with 9 live tests ignored; `cargo check -p aa-net --features bevy --examples` passed.
No live lane traffic was used; natural pet summon/ride/gear and vehicle QA remain pending
while the lane is stalled.

## Explicit mate controls and mount-caster skills

`Command::Mate(MateCommand)` sends the proven controls below. Each validates a nonzero u16
timeline. Mount/unmount accept the lane's mate seats 1 (driver) / 2 (passenger); reasons remain
raw u8 values. The session sends valid commands only while InWorld.

| MateCommand | Opcode | Exact body |
|---|---|---|
| `Dismiss { tl_id }` | 0a4 | u16 tl |
| `Mount { tl_id, point, reason }` | 0a7 | u16 tl, u8 point, u8 reason |
| `Unmount { tl_id, point, reason }` | 0a8 | u16 tl, u8 point, u8 reason |
| `Target { tl_id, target_obj_id }` | 0a5 | u16 tl, bc target; 0 clears |
| `UserState { tl_id, state }` | 0aa | u16 tl, u8 state; valid 0..3 |

Target values above 0xffffff are rejected before a three-byte write. Native wrappers
39069bb0 /39069ac0 /39069ca0 /39069d60 map stand/passive/protective/aggressive to 0/1/2/3
and call 3933def0, which updates native local presentation and sends 0aa. The lane updates
its stored mode with **no ACK**; `Mates.own.spawn.user_state` remains the last server spawn
snapshot. aa-client may implement the native local mode presentation separately; no successful
server state is invented here.

`MateEvent::Requested(command)` is only a successful local socket write;
`RequestRejected { command, message }` is local validation failure. Neither mutates Mates,
attachments, target or ownership. Server `UnitAttached`, `UnitDetached`, `UnitsRemoved`,
`TargetChanged` and errors continue independently. There is no invented separate mount or
dismiss ACK, and no successful local-send event without an actual connection/write.

Native mount/dismiss sequencing is distinct from the packet API. X2Mate:MountMate wrapper
390697f0 selects the contextual mate then 393642f0 queues an action with driver 1 for an
owned mate, passenger 2 otherwise. Action 3909b070 sends 0a7 with reason computed by 3909ae90
from approach geometry/seat occupancy (return values 1..4). A fixed reason 0 is not claimed
as the native planner. X2Mate:UnMountMate wrapper390697b0 invokes39361e80 with reason 9;
it uses the actual current attach point. Dismiss 39362070 first attempts passenger removal,
uses reason 10 to dismount the owner when mounted, and otherwise queues the remove action;
final senders 3933b230 /3933f040 write 0a4. The lane's direct owner-authorized remove handler
also detaches occupants before deleting the mate. The UI action/animation planner is not
replicated by sending extra guessed packets. The 1.2 capture in `docs/aa-client-hud.md`
("Unmount/dismiss reasons, ride cooldown and CanMount reach") proves reason bytes 9/10, the shared
2000 ms `_DAT_3a169944` cooldown and the 625 (25 m) `FUN_3935aa10` reach; a mounted dismiss sends
only `0xa8` reason 10 first and `CSRemoveMate` about 1.1 s later.

**KickPassenger is a lane gap:** native 390697d0 /39361fc0 sends Unmount with passenger
point 2 and reason 11, but the lane's mates::unmount_mate accepts only a seat occupied by
the requesting character. An owner trying to eject a different passenger is silently
rejected. No functional kick method or fake successful detach is exposed by aa-net.

`Command::UseMateSkill { skill_id, mate_obj_id, mount_skill_id, target_obj_id }` sends the
existing CSStartSkill 052 with explicit mount caster. `skill_id` and `mount_skill_id` must
be nonzero; the caster must be a nonzero 24-bit unit ID. `target_obj_id` may be 0: live
capture 2026-10-28 shows 1.2 still sends the cast with target kind 0 / object 0 when there
is no current target (skill 11016, mount_skill 1: `08 2b 00 00 03 81 55 00 01 00 00 00 00
00 00 00 00`); a nonzero target must fit 24 bits. aa-net imposes no automatic target
selection, learned-skill check or mount-slot resolution; the aa-client HUD resolves those
before it queues the command (below). For a pet self buff the caller supplies the target
explicitly (the decompile's `skills.target_type_id` 6 rows name the mate; the captured five
rows are all target_type_id 0 and sent object 0). The exact 17-byte body is u32
combatSkillId, u8 casterKind 3, bc mateObjId, u32 mountSkillId, u8 targetKind 0, bc
targetObjId, u8 flag 0. The low-level `skill::StartCaster::Mount { obj_id, mount_skill_id }`
is also available. The player's own object ID is not substituted into the mount caster.

Local `MateEvent::SkillRequested { skill_id, mate_obj_id, mount_skill_id, target_obj_id }`
and `SkillRejected { same fields, message }` do not create a cast or consume resources.
Actual SkillStarted/Fired/Ended and costs remain authoritative. The lane's mount authority
permits an unmounted owner or a passenger with a matching occupied seat, riding flag and
parent relation; it does not impose a static learned-skill allowlist (temporary grants are
supported). Its on_mount_skill_used separately resolves a linked rider skill by attach
point; the client must not duplicate that server-side extra cast.

Native pet action slots are established through `ISLOT_PET_ACTION` rather than a guessed
X2Mate:StartMountSkill method (`hud/pet_action_bar/pet.alb`, `MAX_PET_ACTION_SLOT` 5). The
aa-client HUD resolves a click or `pet_action_bar_button` hotkey through the received
mate's displayed mount rows (`owned::displayed`; `GetMountSkill` = `FUN_3906aa90` ->
`FUN_3933c340`) in the received `npc_mount_skills` order and applies the 1.2 gate
`FUN_3933e630`: slot shown, mate level (`+0x4b4`, `MateSkillAccessor::Update`
`FUN_3920a320`) >= `skills.ability_level`, and the first
`mount_attached_skills.attach_point_id` equal to the riding flag (`player+0x3af4`; an empty
vector is any state). A refusal (a not-learned slot, or any slot while not riding) is skill
result `NOT_NOW` (`0x2c`, `FUN_39390900` -> `FUN_39897990`), not a packet, and the HUD shows
the local "Can't be used right now." text. Live capture 2026-10-28 (Heir's Horse, item
27264): the rows display in the received order `mount_skills` 83, 1, 13, 6, 3 -> skills
17092, 11016, 11369, 11037, 11309; `GetMountSkill` `skillUsable` is 1 only while riding
(0 on foot) for the captured target_type_id 0 rows (the decompile's target_type_id 6 bypass stays
ready in either state); learned is `skills.ability_level <= mate level` (the level-5 horse's slots 2..5
report `learned` false). While riding the mate (driver seat) it queues `UseMateSkill` with
`mount_skills.id`, the row's `skills.skill_id` and the mate object as the kind-3 caster, one
packet; nothing is sent for the linked `mount_attached_skills` rider skill (the server casts
it). The captured wire bodies are `c4 42 00 00 03 81 55 00 53 00 00 00 00 00
00 00 00` (skill 17092, mount_skill 83, no target), `08 2b 00 00 03 81 55 00 01 00 00 00 00
03 67 00 00` (skill 11016, mount_skill 1, current target 0x6703) and `08 2b 00 00 03 81 55
00 01 00 00 00 00 00 00 00 00` (the same row with no current target, still sent). On foot
1.2 has no `pet_manual` pending mode: the click sends nothing and shows `0x2c`, and the next
click/movement sends nothing. Cooldowns are the server's; `GetAutoStartMountSkill` stays
false and raw dynamic triples are not guessed cooldowns.

The offline client tests build the 17-byte mount-caster body from the queued `Command`
(`Command::mate_skill_message`) with the mate as the kind-3 caster, check the mate-level and
attach refusals (`NOT_NOW`, no packet), the silent hotkey with no mate or game tables, the
on-foot `NOT_NOW` refusal, the `target_type_id` 6 mate target, the three captured byte-exact
bodies (self, targeted 0x6703 and targeted with no target), and the happy
`ISLOT_PET_ACTION` click / `pet_action_bar_button` hotkey path through `handle_requests` and
`commands`.
`cargo test -p aa-net -j 4` passes 777 library, 2 login_session and 3 recovered_packets
tests, with 9 live tests ignored; `cargo check -p aa-net --features bevy --examples`
passed. No live lane traffic was used.

Pending live checks for mount skills: whether a passenger seat (point 2) also counts as riding, which
1.2 computes as `player+0x3af4 == 1 && player+0x3af0 == this mate` (the remake stays driver-only;
live capture 2026-10-28 had one client only); what a `target_type_id` 0 self row sends with a
target selected (the captured self rows had none); the source/meaning of `GetAutoStartMountSkill`, which
stays false; and the error ordering against a server `SCErrorMsg` (`0x10A` in 1.2; the older `0x0A0`
did not appear in S4). The local `0x2c` refusal is confirmed, and no `SCErrorMsg` was received in the
S4 slice, so whether a server error can arrive before/with/after the local one is still open.

## Known gaps

Refreshed 2026-10-23 (the list before named slave, mate equipment, buff update and guild packets as
undecoded; they are decoded, see their sections).

* Korean auth flows (challenge, OTP, ARS) and Mail.Ru / Tencent logins; only the Trion ticket.
* Server → client: the packets the lane can still send undecoded are listed in "Small server
  messages" below (mount / ship / glider and doodad packets other work owns, `SCCharDetail`, the
  chat spam configuration). A packet in a `DD04` batch that is not decoded is skipped when the batch
  can be resynchronised (see "Compressed packet batches").
* Client → server: requests exist for the systems the HUD drives (skills, items, chat, NPC talk,
  quests, shops, bank, mail, trade, auction, party, family, expedition, duel, salon, lobby refresh /
  labor power manager ...). Not written: the cash shop (`CSICS*`), `CSCharDetail` (inspect), user
  music, vehicle / ship / hanging movement (N4).
* No maximum HP / MP on the wire: `PlayerStats::max_hp` is the highest value seen (a character that
  spawns dead or hurt shows too small a maximum until it has been at full health once) unless the
  application sets the formula value with `PlayerStats::set_max` (the HUD does).
* Quests: `SCQuestContextFailed`, `SCQuestUnitReqFailed` and the quest error notices are not decoded
  (the lane server never sends them). The objective texts are not on the wire: the HUD makes them from
  the game database. Quest cinemas are reported watched, not rendered (see "Small server messages").
* The sub zone name of the minimap header is not sent: derive it from the position and the
  `sub_zones` table. The zone group follows the zone chat channel (Shout).
* NPC names are empty on the wire (the client resolves them from the NPC template); `Unit.kind` has the
  template id.
* Movement: we only write a plain unit movement (ground, jump, fall with landing velocity); vehicles, ships,
  standing on / hanging from objects are read (`Movement::read_typed`) but not written. The server does not
  echo our own movement back.
* A dropped game connection is not reconnected, as in the 1.2 client: the session reports
  `Disconnected` and returns to idle, the HUD shows the original disconnect dialog (see "Small server
  messages", reconnect).

### Exact item detail payloads (2026-10-17)

`model::Item.detail_payload: Option<Vec<u8>>` retains the exact bytes after its detail tag,
without fixed-block padding. `ItemDetail::kind()` returns that original tag, while the existing
`None` / `Equip` / `Mate` / `Other(u8)` enum remains unchanged. Real decoded tag-zero items carry
`Some(vec![])`; template-only / synthetic items have `None`. Equipment payloads include the two
previously skipped reserved bytes after durability. Consumers can call the native item-link
`ItemDetail::from_wire(item.detail.kind(), payload)` without inventing opaque union fields.

Both ordinary `Item.Write` and padded task-created items preserve this payload. `ItemTask::Update`
also carries `detail_payload`; the inventory reducer replaces parsed detail and raw payload together
(including repair updates). A malformed or unknown-size padded block keeps its existing tolerant
`Other(tag)` behavior with payload `None`; it does not advertise a guessed partial payload. Padding
is never part of the retained payload. Native 3961edf0 proves payload widths 0/55/29/6/9/24/16/16/8/4
for tags 0 through 9; tags 7 and 9 now preserve DD04 alignment as opaque details too.

Lane discrepancy: `models/item.rs::write_details_to` encodes slave repair time as i32 zero when unset,
but as i64 when set, producing tag-2 payloads of 29 or 33 bytes. The reader preserves whichever was
actually received. The native item-link union reader expects 29 bytes; do not truncate or normalize
an observed 33-byte lane payload into an apparently supported native record. Exact nonzero repair-time
translation remains unresolved. The lane writer's normal 29-byte form maps directly.

Offline tests cover every native fixed tag, both lane slave-time forms, all item truncation cuts,
nonzero equipment reserved bytes, task padding exclusion and a DD04 loot notification followed by
loot settlement and snow. No stalled-lane traffic was used.


### Mate equipment receive transactions (2026-10-17)

`MateEvent::EquipmentChanged(MateEquipmentChange)` decodes `SCMateEquipmentChanged` (0x126):
`u32 character_id, u16 mate_tl_id, u32 passenger_id, bool bts, u8 count`, then `count` entries
of two full optional items followed by `u8 first.container, first.slot, second.container,
second.slot`, and one final `bool success`. The stored character ID must not be confused with
its runtime object ID. `MateEquipmentEntry::{first,second}` each hold `MateItemLocation` with
`item: Option<Item>, container: u8, slot: u8`; exact raw detail payloads survive on both items.

Native packet vtable 399b5e2c -> 397af340 -> 397d1340, entry reader 397d12a0, proves the full
items, header widths, final flag and inline maximum of two entries (constructor 391cfaff also
allocates two). Native clamps counts to two; this decoder rejects larger counts rather than
misidentify unread transaction bytes as a following DD04 opcode. Empty lists are legal.
The lane's economy/item_effect_packets.rs writer currently sends exactly one entry.

`Mates.last_equipment_change` caches only notifications matching the selected stored character
ID and current owned mate timeline. A replacement summon, matched removal/object reuse or session
teardown clears it. It records a notification, not successful settlement. No inventory swap or
phantom mate is inferred from it. Actual visible mate gear still follows the existing authoritative
`SCUnitEquipmentsChanged` / `Event::EquipmentChanged` path.

Known lane discrepancy: the trailing success is unconditionally true even when `change_mate_equipment`
rejects a move. Its successful enter/leave effect-generated locations also cannot be treated as a
universal crossed swap: native OnMateEquipmentChanged (391db200 -> 394e1c10) copies only the second
item into the second location when that location resolves in the bag cache, then releases pending
locations through 397d2370. The source's successful effect packets commonly mark that second location
as mate equipment (252). Retaining the exact notification is safe; inventing missing bag settlement
would hide this server/native mismatch. Outbound mate equipment and reconciliation remain a separate
milestone; slave equipment handlers are still logging stubs in this lane.

Offline tests cover zero/one/two entries, both success flags, opaque full-item identities and detail
bytes, raw passenger ID/BTS, every truncation, invalid counts, DD04 continuation, ownership/timeline
matching and the absence of inferred inventory/Unit changes. Live equipment QA remains deferred.


### Slave creation and owned timeline lifecycle (2026-10-17)

`ServerMsg::Slave` / `Event::Slave` carry `SlaveEvent::{Created(SlaveCreated), Removed { obj_id,
tl_id }, Despawn { obj_id }}`. `SlaveCreated` contains `owner_obj_id: u32, tl_id: u16,
obj_id: u32, hide_spawn_effect: bool, raw_id: i64, creator_name: String`. Exact receive bodies:

- SC061: `bc ownerObj, u16 TL, bc slaveObj, bool hide, i64 rawId, string creator`.
- SC062: `bc objId, u16 TL`; the ID's role depends on the lane call site, so it stays neutral.
- SC063: `bc slaveObj`; this announces vehicle/effect despawn, not a full Unit removal packet.

Source: vehicles/packets.rs writers and slaves.rs create_spawn/delete/riders_escape; native packet
readers 397b34d0 / 397b3610 / 397b3690. SC061 is broadcast around the owner before the actual slave
is constructed/spawned, including nearby peers. Raw i64 is zero in the lane; no item/database ID
meaning is inferred. Native OnSlaveCreated 391d45d0 -> 3934a6c0 compares ownerObj with the local
runtime player object, remembers its own timeline, and emits `SLAVE_SPAWN()` with no Lua arguments
(event 0x66), before its remaining creation/effect metadata registration. A creation packet alone
never supplies a template, position, model, equipment or a visible Unit.

`Slaves` is a plain resource registered by NetPlugin and available as `WorldState.slaves`:
`own: Option<SlaveCreated>` is the latest locally owned announcement, while
`created: HashMap<u32, SlaveCreated>` records creation/effect metadata keyed by slave object ID.
It correlates owner runtime ID with `PlayerSpawn.obj_id`, in either packet order; it never compares
that field with the stored character ID. Per the lane CreateForOwner replacement rule, a newer
announcement replaces older creation metadata for that same nonzero owner. Other owners' packets
cannot claim local ownership. UnitSpawned of another kind/timeline at a remembered object ID drops
stale metadata/ownership; selection, logout, disconnect and kick reset it.

Keep despawn and owned-timeline cleanup distinct. Native OnSlaveRemoved 391d4660 -> 39345cb0
clears the owned timeline only when `objId == localPlayerObj` AND `TL == currentOwnedTL`.
Normal lane delete sends SC063(slaveObj), then SC062(ownerObj, TL). Rider's Escape instead sends
SC063(slaveObj), SC062(slaveObj, TL), and respawns it; that SC062 is not the native owner ACK.
The resource clears matching creation metadata for either source-backed role, but clears `own`
only on the native owner-ID/timeline match. SC063 / generic visibility removal clear the creation
cache while retaining the owned announcement: **`own.is_some()` does not assert active visibility**.
Use the actual Units resource for visibility; these notifications do not invent or immediately
remove Unit instances. An old timeline removal cannot clear a newer replacement.

Existing `MateEvent::SlaveState` (SC1ab) retains its independent exact dynamic state fields; it is
not ownership proof. SlaveBound064 / MySlave066 / Escape067 and explicit controls are now implemented
under the next receive/control milestone below. Owned equipment, native HP/position UI planning and
presentation effects remain later work. No completed summon, dismissal or equipment transaction
is fabricated while the live lane is unavailable.

Offline coverage: all three bodies and every truncation, raw i64 / Unicode creator, combined DD04
continuation, runtime-vs-stored owner identity, both arrival orders, peer announcements, replacement,
stale removal, Rider's Escape distinction, object reuse, visibility loss and disconnect. Session
bridging also verifies creation and mate-equipment notifications arrive before subsequent points,
without creating a Unit or claiming an unknown local owner. Live summon/despawn QA remains pending.


Validation for these receive milestones: `cargo test -p aa-net` passes 207 library tests and two
scripted loopback login tests; all nine live-lane tests remain ignored. The Unicode/DD04 slave
fixture also passes its targeted rerun. `cargo check -p aa-net --features bevy --examples` passes.
Only aa-net and this document changed; no lane connections or server changes were used.


### Native bounded craft notice SC1bf (2026-10-17)

`CraftFailureNotice { raw_id: u32, raw_count: i32, entries: Vec<u32> }` is carried by
`ServerMsg::CraftFailureNotice` and `Event::CraftFailureNotice`. Native reader 397b8110 first
reads u32 rawId and i32 rawCount, then consumes exactly `max(0, min(rawCount, 20))` u32 values.
It retains the original raw count, including negative and greater-than-twenty values. The decoder
follows this native cap rather than allocating from the raw count, rejecting it or consuming
unbounded lane entries. Every consumed u32 survives unchanged, including zero and duplicates.

Native OnCraftFailed 391e0840 -> 394d2370 independently clamps the handler loop to twenty,
resolves each entry through item-template lookup 39695dc0 and formats a template item link through
394e4ef0 for `CRAFT_FAILED(itemLink)` (event0x77), once per resolved entry.
That function does not use rawId. The network API keeps these
fields neutral: no recipe ID, craft acceptance/completion, material settlement or queued-batch
cancellation is inferred; authoritative skill/item/labor events remain independent.

Lane `economy/craft.rs::SCCraftFailedPacket` is unused and marked TODO: it writes u32 id,
i32 count, then `max(count,0)` copies of its field named craft_id, with no native twenty-entry cap.
Count0..20 bodies match the native layout; positive count>20 lane bodies do not. The native reader
wins: excess lane bytes cannot be relabeled valid fields or silently swallowed at the cost of the
next DD04 packet. Such a server/native mismatch still needs a lane fix before that unused writer
is activated. No server files were changed.

Tests preserve signed minimum/negative/zero counts and native twenty-entry caps even for i32::MAX,
compare the lane's existing count-two golden, cut every body byte and append a real following DD04
packet. Session bridging emits the raw notice in order without mutating inventory or craft state.


### Native slave position reports and explicit controls (2026-10-17)

Receive `SlaveEvent::Bound { master_id: u32, obj_id: u32 }` is SC064: full u32 stored master
character ID plus bc slave object. Native vtable399b32c4 ->397c5c60 and handler391d4760 use that
master ID independently of the runtime owner/summoner ID; driving another unmarked ship does not
prove local ownership. Existing authoritative UnitAttached/Detached events remain the attachment
state. Bound is retained as an exact event without fabricating ownership or an attachment.

`SlaveEvent::Position(SlavePositionInfo)` is SC066 / SCMySlave: bc object, u16 timeline,
string name, u32 template, i32 HP/maxHP, i64 X/Y and f32 Z. `SlavePositionInfo` exposes all those
fields, including raw X/Y low bits, and `position() -> Pos` provides the existing coordinate
conversion for presentation. Native vtable399b32d0 ->397ca080 uses position helper3961e270.
Handler391d4800 ->39345e10 accepts a report only for its already remembered owned timeline;
therefore `Slaves.position: Option<SlavePositionInfo>` updates only when that TL matches `own`.
It neither proves ownership nor creates/moves a Unit. The reported runtime object can change
while the owned TL stays the same; the raw creation announcement remains unchanged. Position
reports persist through visibility/effect despawn, and clear with a matched owner/TL removal,
a replacement timeline, object reuse or session teardown. The lane sends reports to the actual
summoner after create, on death and every five seconds; stale reports for another TL are ignored.

`SlaveEvent::Escaped(SlaveEscape { obj_id, x: i64, y: i64, z: f32, yaw: f32 })` is SC067:
bc object, native long position, f32 yaw. Native vtable399b32dc ->397c5cc0 and handler391d48a0
->39345ea0 apply it only to an existing slave actor. The plain Units reducer likewise updates
position/yaw only on an existing UnitKind::Slave. It never creates an object, moves a character
or changes owned metadata; raw received coordinates remain available in the event. It is an
authoritative incoming transform, not a local movement-request ACK. The lane defines but does
not currently emit SC067; its Rider's Escape uses despawn/removal/respawn instead.

`Command::Slave(SlaveCommand)` supports exact native requests:

- `Bind { tl_id }`: CS031, u16 TL (native399cfc6c ->397cacc0).
- `Unbind { tl_id }`: CS032, u16 TL (399cfc78 ->397b9520).
- `Dismiss { tl_id }`: CS030, u16 TL (399cfc60 ->397b94e0).
- `Despawn { obj_id }`: CS02f, bc object (399cfc54 ->397c7600).
- `Rename { tl_id, name }`: CS034, u16 TL then string (399e2470 ->397b9560).
- `Spawn(SlaveSpawnRequest)`: CS02e, u32 template, i64 X/Y, f32 Z/yaw, u64 item ID,
  conditional u8 container/u8 slot when item ID is nonzero, then bool hide (399cfc48 ->397cabf0).

High-level session validation requires nonzero TL, a nonzero 24-bit object ID for Despawn,
nonempty/NUL-free names within the native 128-byte name buffer, nonzero spawn template,
finite spawn Z/yaw and `item_location: Option<SlaveItemLocation { container, slot }>` present
exactly when item_id !=0. X/Y are already-native i64 coordinates: the caller owns the position
planner. Name regex/normalization, ownership, seat availability, loaded-item warnings and actual
permission remain server decisions; no guessed client whitelist or automatic target/seat selection.
The native Lua `X2SiegeWeapon:SetSiegeWeaponName(name)` wrapper394aa0c0 normalizes through its
name helper and sends034; `DiscardSiegeWeapon()` wrapper394aa090 ->39345aa0 chooses an unbind
path based on current attachment. Dismiss39345bf0 sends030 and clears local native presentation;
the network resource still waits for actual server lifecycle events. Native UI planning is root work.

Native slave boarding (corrected 2026-10-28; the earlier "a slave bind never sends 0x068" claim
was wrong): the click dispatcher `FUN_390f3780` -> `FUN_393be030` sends `CSChangeTarget` 0x02c
(when the slave is not the target) and `CSStartInteraction` 0x068 (`extra_info`1, mouse3) for the
slave. `SCNpcInteractionSkillList` 0x07b -> `FUN_393be850` -> `FUN_393be280` acts on the FIRST
skill immediately: 12076 -> the 3.0 m reach `FUN_393bf270` (`DAT_39ac79fc`9.0; 3.5 m with target
flag+0x3c26) -> `FUN_393bf310` -> `FUN_39362f50` -> `FUN_393464a0` -> `CSBindSlave`031 (u16 TL);
13863/19165 -> `CSStartSkill`. The 2026-10-28 capture recorded the 0x068 body
`3c 5f 00 00 00 00 01 00 00 00 00 00 00 00 03 00 00 00 00` for cart bc 0x5f3c. The decision
belongs to the app (`hud/slave.rs`, `docs/aa-client-units.md` S5); `aa-net` carries the exact
packets.

Only valid InWorld requests write the socket; success there emits `SlaveEvent::Requested(command)`.
Invalid fields emit `RequestRejected { command, message }`, and no connection/server silence cannot
produce a completion. Neither event mutates owned state, attachments, names or transforms. Native
incoming SC070 / `Event::UnitNameChanged { obj_id, name }` (bc object + string; reader397b36f0,
handler391d5060) now updates only known Units, closing the actual Rename receive path and DD04
boundary. SC061 creator_name remains the summoner name, not the vehicle's mutable label.

Important lane gap: CSSpawnSlave is logging-only, and its reader incorrectly always expects four
item-location bytes (including two skipped bytes). Native sends only container/slot for nonzero
item and none for zero item; native body lengths are39 /37 versus the lane's41. The native writer
is preserved verbatim and is explicitly not a functional lane summon. Existing item-skill summon
flow remains independent. No padding bytes, fake acknowledgement or spawned Unit are invented to
hide the mismatch. Slave equipment handlers remain stubs; original skill eligibility/planning and
map/vehicle UI integration are still root work.

Offline checks cover all request goldens, native conditional spawn locations, strict local
validation, all four receive boundaries including rename, every truncation, DD04 continuation,
raw position bits, full master IDs, ownership/TL gating and known-unit-only transform/name changes.
A fresh ephemeral loopback session proves rejected/out-of-world requests never reach the socket,
valid requests emit only Requested, silence leaves ownership/name/position unchanged and a received
SC070 alone supplies the authoritative rename. No stalled-lane traffic or server edits were used.


Validation for the bounded SC1bf and slave control/position milestones: `cargo test -p aa-net`
passes all215 library tests and two scripted login tests; all nine live tests remain ignored.
`cargo check -p aa-net --features bevy --examples` passes on the final production sources.
Only aa-net and this document changed; the added session fixture uses ephemeral loopback only.


## Lane-emitted receive boundary audit (2026-10-17)

The next three small receive gaps were selected by comparing the lane's actual packet
construction/send paths against `ServerMsg::decode_game`, rather than counting opcode declarations
as implemented gameplay. Ranking here is by transition/inventory impact and a bounded, provable
body, not measured traffic frequency:

| Priority | Packet | Exact body bytes | Why this boundary matters |
|---|---|---:|---|
| 1 | `SCLoadInstance` `0x197` | 32 | Portal, dungeon, main-world return, instant-game and trial transitions. Portal source can follow it with `SCTeleportUnit`; that tail must remain readable. |
| 2 | `SCBlinkUnit` `0x073` | 31 | Actual blink skills. NPC movement sends blink followed by regular unit movement, while player blink requires a separate client movement plan. |
| 3 | `SCSyncItemLifespan` `0x098` | 21 | Initial bag/bank/equipment timers, timer changes and equipment unpack/charge. It can appear beside ordinary inventory state. |

All three now have exact body consumption, typed `ServerMsg` and `session::Event` variants,
and public plain data. None changes settled world, player, unit or inventory resources or sends an
automatic response. (Unknown opcodes stopped a DD04 batch then; they are skipped since 2026-10-23.) This was a focused coverage milestone,
not a claim that all lane packets are decoded. Gimmick elevator/raft lifecycle `0x13c/0x13d/0x13e`
was the next coordinated family and is covered below together with its required `0x13f` boundary. Login configuration/faction snapshots, expedition state,
shipyard state, district/portal lists, duel/crime state and notices remain further audit candidates.
`SCCooldowns 0x04a` has an empty-only writer and no runtime send sites; its full native reader
is now covered below. `SCBuffUpdated 0x0b9` has an opcode declaration but no writer/send site.
Neither was ranked as a currently emitted receive gap in this audit.

### Loading and blink directives

`world_msg::InstanceLoad { raw_id: u32, zone_id: u32, pos: Pos, angles: [f32; 3] }` is exposed
as `ServerMsg::LoadInstance` / `Event::LoadInstance`. Wire order is `u32 raw_id, u32 zone_id`,
then six `f32`: XYZ and the three angles. The lane's `SCLoadInstancePacket` writer is in
`systems/skills/packets_special.rs`. Senders in `systems/instances/{worlds,dungeon,instant_game}.rs`,
`systems/npcs/portal/mod.rs` and `systems/social/trial.rs` use the first field inconsistently:
main-world return and portal use a world template ID, whereas dungeon uses the selected runtime
world's ID. Therefore `raw_id` is not substituted for `WorldInfo.instance_id`. Lane senders convert
stored rotations to radians. The entire angle triple is retained, including pitch/roll and float
bit patterns. No packet field reports loading completion. `CSInstanceLoaded` and transitions of
scene/position/loading state still require a separate application flow; this receive handler does
not send either that response or `CSTeleportEnded`.

`world_msg::BlinkUnit { obj_id: u32, distance: f32, degree: f32, x: i64, y: i64, z: f32 }` is
exposed as `ServerMsg::BlinkUnit` / `Event::BlinkUnit`. Wire order is BC object ID, distance,
degree, two native fixed-point longs, and `f32` height. `position()` converts XY using the existing
`long_to_coord`; the original longs retain even the low 32 bits. The degree value remains raw.
`systems/skills/effects/special/movement.rs::blink` unmounts a player's mates and sends the
computed endpoint without first settling the player's transform. Its NPC branch applies the
position before sending blink and a normal movement report. Consequently this event alone neither
teleports a unit nor represents completed player motion, and does not share the existing
`SCTeleportUnit` automatic-reply path. The application owns the actual motion/instance planner.

Bounded native audit of `<client12-unpacked>/x2game_unpacked.dll`:

- SC `0x197` constructor `391b43d0` sets opcode at `391b43eb`, vtable `399b5fe8`, serializer
  `397afb40`: `iid` u32, `zoneId` i32 (retained as the same u32 bits), position Vec3 and angles Ang3.
  `OnLoadInstance 391de970` forwards both IDs and both triples to `3936f1c0`.
- SC `0x073` constructor `391acc20` sets opcode at `391acc3b`, vtable `399b333c`, serializer
  `397c5d00`: BC `unitId` via `397b27d0`, two floats (`distance`, `degree`), then position helper
  `3961e270` reading i64 X, i64 Y and f32 Z. `OnBlinkUnit` starts at `391d7050`.

These native field layouts match the lane bodies. This audit establishes the receive contract,
not the complete native loading or blink execution algorithm. Inbound float values are not silently
sanitized, clamped or treated as validation errors by the body readers.

### Item lifespan metadata

`items::ItemLifespan { added: bool, item_id: u64, template_id: u32, raw_time: i64 }` is exposed
as `ServerMsg::ItemLifespan` / `Event::ItemLifespan`; the type is also reexported from `aa_net`.
Wire order is exactly the field order above. Both add and clear notifications preserve the exact
item identity, template ID and signed time. No template-wide update or synthetic item is created.
There is deliberately no inferred timer cache/countdown, item timestamp rewrite or item deletion.
An actual item removal continues to require authoritative item tasks.

Lane evidence: `systems/economy/item_packets.rs::SCSyncItemLifespanPacket` writes the 21-byte
body. `item_timers.rs::initial_item_expiration_packets` sends an expiration endpoint for ordinary
and online-minute timers. `models/container.rs` builds add/clear `ItemFx::Lifespan` notifications;
`systems/economy/fx.rs` forwards them unchanged. In contrast,
`systems/economy/item_handlers.rs` sends `item.unpack_time` (the charge start) for unpacked charged
equipment. Treating every `raw_time` as an absolute expiry would be incorrect for this lane.

Native SC `0x098` constructor `391a9240` sets opcode at `391a925b`, vtable `399b339c`, serializer
`397aec40`: boolean `added`, nested item-time payload via `397d03b0` (u64 item ID, u32 template ID,
i64 time). This matches the lane's byte layout. `OnSyncItemLifespan 391d35f0` calls manager
`394ef9e0` on add and `394ea940` on clear; this receive slice does not claim to reconstruct that
manager's timer categories or the charged-item UI.

Validation includes independent little-endian body goldens, every truncation point for all three
bodies and their DD01 dispatch, both lifespan flags and full-width signed values, raw blink XY and
float bits, DD04 tails, and an ephemeral loopback session batch with all three directives followed
by real HP/weather updates. The session test verifies wire order, unchanged settled world/player/
unit/item state for the directives, and no automatic instance/teleport reply. No live lane traffic
was used; end-to-end instance loading, physical blink motion and timer UI remain application/live
QA work.


## Gimmick elevator/raft receive family (2026-10-17)

`gimmick` now decodes the coordinated gimmick announcements/motion family and the joints notice
that the lane sends immediately after creation. All four route through `ServerMsg::Gimmick` and
`Event::Gimmick(GimmickEvent)`. `GimmickSpawn`, `GimmickMovement`, `GimmickJoint`, `Gimmick`,
`GimmickEvent` and the `Gimmicks` resource are reexported from `aa_net`.

| Packet | Body and native bound |
|---|---|
| `SCGimmicksCreated 0x13c` | u8 raw count, then `min(count,30)` complete `GimmickSpawn` bodies |
| `SCGimmicksRemoved 0x13d` | u16 raw count, then `min(count,500)` BC object IDs |
| `SCGimmickMovement 0x13e` | 72 bytes: u32 object ID, u32 time, i64 XY, f32 Z, four rotation floats, three velocity floats, three angular-velocity floats, f32 scale |
| `SCGimmickJointsBroken 0x13f` | u8 raw count, then `min(count,200)` triples of u32 object ID, i32 joint ID and i32 epicenter |

`GimmickEvent::{Created { raw_count, gimmicks }, Removed { raw_count, obj_ids }, Moved(report),
JointsBroken { raw_count, entries }}` retains original counts while consuming exactly the native
bounded inline lists. IDs/duplicates/zero entries are not filtered or silently remapped. The lane's
visibility batcher chunks creates by 30 and removals by 500. Its public writers themselves do not
cap arbitrary oversize input; such an overlong body would differ from the native bounded contract.
This decoder follows native consumption rather than treating excess bodies as extra native list
members. The joints notice is currently always sent empty by lane visibility. Its unused nonempty
writer fills both signed fields with zero; the receiver still preserves the complete native fields.
`SCGimmickResetJoints 0x140` and `SCGimmickGrasped 0x141` have opcode declarations but no lane
writers/send sites found, and remain deferred.

`GimmickSpawn` has full u32 `obj_id` (not BC), `template_id`, raw i64 `entity_guid`, u32 `faction_id`,
`spawner_obj_id`, `grasper_obj_id`, and `zone_id`, then a length-prefixed `model_path`, i64 X/Y,
f32 Z, four rotation floats, f32 scale, velocity/angular velocity triples and f32 scale velocity.
The fixed portion including the two-byte string length is 102 bytes. The i64 GUID preserves the
same eight bytes as native's u64, and `zone_id` preserves native's signed i32 as u32 bits. Both spawn
and movement have `position()` helpers; raw XY and all original float fields remain available.
The four rotation values are retained in wire order without normalization or a component conversion.
The lane writer uses quaternion X/Y/Z/W. No faction relationship, summoner ownership, local physics
control, attachment or acknowledgement is inferred from the metadata IDs.

Native's model-path reader has a 99-byte payload limit (`0x63` plus its terminator). This client
rejects negative or over-99-byte lengths rather than guessing native overflow behavior. Valid UTF-8
payloads are kept verbatim (including encoded NULs); invalid UTF-8 is rejected, without asset-path
normalization or replacement characters. The lane
writer accepts an arbitrary string, so that is an explicit compatibility bound; the read-only
compact database has 76 gimmick templates and maximum model-path byte length 87 in this audit.
No database or lane data was changed.

`Gimmicks { by_id: HashMap<u32, Gimmick> }` is registered by `NetPlugin` and available as
`WorldState.gimmicks`. Each `Gimmick` contains the original `spawn` and optional `last_movement`.
Creation registers an announced ID; a duplicate registered ID leaves the original metadata/report
unchanged, matching native's existing-object guard. Removal deletes matching entries. A new
character selection, leaving the world or disconnect clears the table. Movement is retained only
for an already announced ID, and is the last report in wire arrival order. Decreasing timestamps,
midnight/wrap values and raw transforms are not interpreted as a new chronology. The spawn snapshot
is not rewritten, and no ordinary `Unit`, player motion, loaded entity or physics transform is
created or changed. Joints notices remain events without fabricated breakage/physics state.
`LoadInstance` alone does not clear or settle this table: scene transition/reset planning remains
application work, alongside asset loading, interpolation, riders/support transforms and rendering.

Lane evidence: `models/gimmick.rs::Gimmick::write`, `packets/world_objects.rs`,
`systems/npcs/gimmick/packets.rs`, and `world/{visibility,mod}.rs`. Individual and region visibility
both send `0x13f` after `0x13c`; skipping that empty notification would still stop trailing DD04
state. Actual movement packets are emitted by `systems/npcs/gimmick/manager.rs`.

Native evidence (`x2game_unpacked.dll`): SC `0x13c` constructor `391b0fe0` sets opcode at `391b1020`,
vtable `399b5eb0`, serializer `397c3660` caps the byte count to 30. Entry reader `397c1330` matches
all lane fields and applies the `0x63` model-path limit. SC `0x13d` constructor `391b1120` sets
opcode at `391b1140`, vtable `399b3a08`, serializer `397caa00` caps to 500 and reads BC IDs.
SC `0x13e` constructor `391d0230` sets opcode at `391d024b`, vtable `399b7494`, serializer
`397b5bc0` plus nested `397b24c0` matches the 72-byte report. SC `0x13f` opcode assignment
`391b11ce`, vtable `399b3a14`, serializer `397b5bf0` establishes the 200-entry raw-triple cap.
`OnGimmicksCreated 391db870` calls manager `39179910` (existing ID is ignored),
`OnGimmicksRemoved 391db920` calls `39179a40` to delete a matching registered ID, and
`OnGimmickMovement 391db9d0` calls `3917a1d0`, which looks up an existing object and queues motion
for native processing. Reproducing that engine's asset-failure queues, locally controlled-object
filter, smoothing and actual physics application is outside this metadata receive slice.

Tests cover independent byte goldens/full-width IDs, every truncation point, native count boundaries
and over-cap inline consumption, string byte limits, following DD04 entries for every message,
duplicate/remove/recreate/reset behavior, no phantom units, and an ephemeral loopback session
checking ordered events/resource updates, later HP/weather state and no automatic reply.
Live elevator/raft visibility, riding and scene-transition QA remain pending while the lane is stalled.


## Inventory services native/lane re-audit (2026-10-17)

The NPC repair and bag/bank expansion request/receive APIs already existed. This follow-up fixes
the native expansion tail, bank event routing, service send gating and confirmation preflight;
it does not invent additional successful repairs, sorts or expansions.

Native expansion `CSExpandSlots 0x041` is **eight bytes**: i32 container, bool auto-use-AA,
then a BC context object ID. The earlier five-byte implementation matched the lane reader only.
`BankCommand::Expand { container, auto_use_aa_point }` keeps its Rust shape and now appends an
explicit unset context zero. `ExpandWithContext { container, auto_use_aa_point,
raw_context_obj_id }` writes the caller's exact 24-bit context, with zero allowed and values above
`0xffffff` rejected before send. Container remains 2 bag or 3 bank. The context is not inferred
from a target, current NPC, owner or local character.

Native sender `394e0980` sets CS `0x041` at `394e098f`, constructs via `397aba40`, and sends at
`394e09b9`. Its vtable `399e5ea8` points to serializer `397b97b0`: signed i32 `slotType`, bool
`autoUseAAPoint`, then BC data at packet+0x14. The sender obtains that last argument from
`393b7b80(0xf)`, which reads a navigation-context value or a fallback global. The exact role and
lifecycle of those globals remain unresolved; the public API therefore exposes raw context only.
The lane `systems/economy/item_handlers.rs::CSExpandSlotsPacket` reads its original first five
bytes and ignores the native tail. `net/registry.rs::dispatch` does not require stream exhaustion,
so the complete native request remains source-compatible with this lane's individually framed
requests. That compatibility is source-audited, not newly live-verified while the lane is stalled.

Native NPC repair request shapes match the existing code: `CSRepairSingleEquipment 0x03c`
is u8 container/u8 physical slot/bool AA (three bytes), and `CSRepairAllEquipments 0x03d` is one
bool. Sender `394f10e0` chooses single at `394f1298` and all at `394f12d5`, then sends at
`394f12c8` / `394f12fd`. Constructors `397abb10` / `397abb30`, vtables `399e5efc` / `399e5f08`
and serializers `397b99d0` / `397b0930` confirm those fields. Native bag wrapper `3944ccc0`
converts the Lua one-based physical slot to zero-based before reaching this sender; sorting still
requires the UI's virtual-to-physical mapping first. Repair-all may skip individual unaffordable
items in the lane. Complete reason3 item-task packets acknowledge only the detail updates they
contain; an empty report is a response with no repaired items. Money changes remain separate
reason83 tasks, and unrelated errors/wear/capacity packets do not finish a repair.

`BankEvent::{Requested(BankCommand), RequestRejected { command, message }}` is exposed through
`Event::Bank` and reexported from `aa_net`. Requested means only a successful local socket write.
Invalid bank commands previously used `ShopEvent::RequestRejected`; they now stay in the bank
family. Bank and repair session commands require `Phase::InWorld`; phase and field validation
failures send no bytes and emit their own local rejection. A missing connection is not a successful
send. No additional bank resource was added: real balances remain `PlayerStats`, capacity and
items remain `Inventory`, and repair reports remain `Repair`.

`ExpansionQuote::check()` now also checks a manually constructed quote's container, representable
capacity, expected next capacity, rule step/bank flag and nonnegative requirements.
`quote.recheck(&inventory,current_money)` returns a fresh checked quote with current money and
certificates counted only in the bag, or `ExpansionFailure::CapacityChanged { expected, actual }`
when the quoted container has changed. This matters because CS `0x041` contains neither expected
capacity nor rule step: sending an old confirmation could expand a different tier. The helper is
local preflight, not a reservation, packet send, ACK or capacity/currency/item mutation.

Native expansion ACK `SCInvenExpanded 0x04d` remains exactly u8 container/u8 slots: constructor
`391a9850` sets the opcode at `391a986b`, vtable `399b3204`, reader `397aeaf0`. It confirms only
returned capacity. Certificate consumption must arrive in its actual item tasks. The lane parses
but ignores auto-AA and native context; `inventory::expand_slot` also ignores rule `currency_id`
and checks/debits character gold. It can silently refuse missing requirements or the final tier.
For a positive configured money price it deducts gold without sending its MoneyChange task, so
the client must retain the last received balance rather than synthesizing a debit from capacity.
Current database tiers have price zero and require a bag certificate, with bag150/bank130 limits.
Expansion success remains unavailable in fresh-account live QA without that naturally acquired
certificate; no grant or fake ACK is used.

Sorting remains local native tab ordering, with no inventory-sort request or server rearrangement
ACK. The original default-tab comparator and physical-slot mapping are documented above; tab
persistence/refresh and BAG_TAB_SORTED/BANK_TAB_SORTED belong to the app's original Lua bridge.
The lane has generic per-character UI-data request/save packets, but their option IDs/blob mapping
to these tabs has not been proved here and is not fabricated. `CSRepairSlaveItems 0x035` is a
three-byte NPC-ID request whose lane handler only logs, so no functional vehicle-repair API or
success is claimed. `CSRepairPetItems 0x0b5` is stablehand revival of dead mate summon items,
not inventory equipment repair, and remains the separate owned-mate service scope.

Focused tests cover independent native expansion bytes for zero/nonzero/maximal contexts,
validation of public/stale quotes and fresh money/bag certificates, all ACK truncation points,
partial repair report rejection, and an ephemeral loopback session checking service phases,
request rejection families, exact frame bytes, ordered real ACKs and unchanged unacknowledged
money/items/repair requests. No live lane connections or server changes were made.


## Opaque character UI data and native persistence boundary (2026-10-17)

`Command::UiData(UiDataCommand::Request { character_id, data_type })` sends CS117;
`Save { character_id, data_type, payload: String }` sends CS118. Type IDs remain raw u16;
no blob schema, Lua table encoding, account synchronization or successful save is inferred.
Both are available as low-level `game::ClientMsg::UiData` too; call `validate` before
encoding directly. The session validates payload/phase, requires a nonzero character ID,
and permits saves only in `InWorld` with the exact active selected ID. Requests can run in
the character lobby, while entering, or in world; the lane alone checks connection ownership.

Native exact wire audit against `<client12-unpacked>/x2game_unpacked.dll`:

- Request sender392175f0 sets opcode117 at39217610, constructor397acb90;
  vtable399bf264 -> body397bc460. Body is u16 type then full u32 character ID (six bytes).
- Save sender39217640 sets opcode118 at39217734, constructor397acbb0;
  vtable399bf270 -> body397bc4d0. Body is u16 type, u32 ID, signed-i16 UTF-8 byte length,
  exact bytes, without a trailing length field or a terminator byte. The native constructor
  copies into a 0x2000-byte buffer, the reader limit is0x1fff, and the writer uses
  `xlcommon::XlStringSize(char*)` (330199a0 = strlen). aa-net enforces8191 **bytes** and rejects
  embedded NUL saves rather than silently truncating them. Empty payload is valid.
- Response factory391b6070 (opcode assignment391b608e, allocation0x2018),
  vtable399b4120 -> reader397b81a0: u32 character ID, u16 type, signed-i16 byte length,
  opaque UTF-8 bytes (native cap8191), then signed-i32 raw length. aa-net preserves all text,
  including received NULs, and the exact raw signed trailing value. Negative/oversized byte
  lengths, invalid UTF-8 and every truncated field are rejected; a trailer never drives an
  allocation or controls packet consumption. DD04 consumes exactly this body before its tail.

`ServerMsg::UiData(UiDataResponse)` becomes `Event::UiData(UiDataEvent::Response(value))`.
Response fields are `character_id`, `data_type`, `payload`, `raw_length`. The separate
`matches_lane_utf16_length()` checks exactly `payload.encode_utf16().count()+1`, including
supplementary characters as two code units. It is deliberately **not** a decoder rejection
rule: native generic reader preserves that i32, and native consumers use it as a byte count.
Specifically handler391e57e0 routes types1/2 to lobby storage391e4320 which calls memcpy
(398b5446 -> msvcr memcpy) with the raw length, and other types to39217910 ->
`xlcommon::Data::Load(char*,int)` (33004af0), which also memcpy-copies exactly that many bytes.
The lane `packets/world.rs::SCResponseUIDataPacket` computes UTF-16 code units+1. These
rules coincide for ASCII but can truncate native consumption of non-ASCII text; no lane or
native semantics were silently rewritten. The native server-side producer remains unavailable.

Lane ownership and lifetime are explicit. `systems/services/character_packets.rs` request
handler responds only for the active matching character or an ID in that connection's
character list; unowned IDs produce **no reply and no error**. Missing option returns an
empty response. The save handler ignores the supplied ID and inserts type/payload into its
active character's `options`; with no active character it does nothing. It emits **no save ACK**.
`models/character.rs::Character::get_option`, and `db_character.rs` load/transaction-save
code store these values by character ID in `options(owner,key,value)`. A socket send does not
prove transaction persistence, and a later request response has no revision/request token
which could prove it corresponds to a particular save. No live persistence claim was made.

`UiDataEvent::Requested(command)` means a successful local socket write only;
`RequestRejected { command, message }` reports local validation only. There is no pending
request correlation, synthetic server error, optimistic cache update or saved event.
`UiData` (also `WorldState.ui_data` and a NetPlugin resource) is a received-only cache:
`get(character_id,data_type)` and `entries()` expose the last response per key in arrival
order. It holds at most64 entries (an explicit application budget, at most about512KiB of
payload), evicting oldest arrival; repeated keys replace/move to the end. Character selection
retains only that character's previously received lobby values; new login/world connection,
connect failure, `LeftWorld` and disconnect clear it. Unknown raw type IDs remain supported.

The native name tables399bf178/39a20510 label IDs0..6 respectively `system_option`,
`character_option`, `key_binding`, `quest_notifier_list`, `quest_context_state_values`,
`common_server_data_1`, `common_local_data_1`. These labels do not establish that an arbitrary
X2 Lua getter uses a particular wire type. Lua `ui_management.lua` uses
`GetCharacterUiData/SetCharacterUiData` for `uiOptionDatas`, `screenResolution`, saved window
bounds and other tables, but the complete Lua-to-manager ownership/serialization mapping is
still unresolved. ManualSort394dc6d0 remains a local tab operation; its persistence is not
assigned a guessed UIData type or network acknowledgement.

Bounded keybinding audit for later UI integration:

- SaveHotKey393ff1f0 first commits the draft via393ff190, obtains manager type2 with39217510,
  clears/rebuilds an `xlcommon::Data` tree, then393ff7fb calls `Data::Save("hotkey.g",true)`.
  It does not directly emit CS118. Generic manager39217640 separately uses `Data::SaveToBuffer`
  (xlcommon33002ce0) into8192 bytes and then the native CS118 constructor/send.
- Tables39ac9054/39ac9020/39ac9008 prove primary/second; modes `normal_mode`,
  `mode_action_mode`, `builder_rotate_mode`, `screen_shot_mode`, `quest_directing_mode`;
  and kinds `builtin`, `spell`, `button`, `macro`, `item`, `builtin_multi`. Save constructs
  primary/second -> mode -> kind -> action-name; normal action leaves contain the key string,
  while builtin_multi adds key-string -> integer binding-index beneath the action.
- Key builder393fc400 prepends `CTRL-`, then `ALT-`, then `SHIFT-`, and the native input name
  from39436d90. Save skips flymode/godmode and entries marked removed. Explicit unbinding is
  separately represented in a `removed` subtree; omission alone must not erase defaults.
- User load39400e80 loads default bindings first, requires a positive `version` in the type2
  Data tree, then overlays via39400a70. That parser has different traversal for version0/1
  versus later versions. Action matcher393ffc20 compares action name, kind, primary/second
  and, for builtin_multi, binding index; key collision handling is separate. The complete
  version emission/merge policy and exact Data text grammar remain unresolved, so aa-net
  supplies **no purportedly compatible binding encoder/decoder**. Root's application-local
  account settings are independent from this character-owned wire storage.

Offline checks cover request/save goldens, opaque Unicode round trip, lane UTF-16 count
(including supplementary text), exact response/trailer preservation, empty/max payload,
negative/oversized lengths, NUL handling, invalid UTF-8, every truncation, multiple responses
and subsequent state in a DD04 batch, bounded received-only cache/lifecycle, and scripted
loopback session preflight/identity/request order without a fake save ACK. Natural lane
round trip, reconnect durability and native UI application remain unverified while stalled.

Validation: `cargo test -p aa-net --features bevy --quiet` passed235 library tests and2
scripted loopback integration tests;9 live lane tests remained ignored. No lane traffic,
server mutations, dependency changes or other-crate edits were made for this milestone.


## Environmental reports and authoritative combat flags (2026-10-17)

Four previously unhandled receives now have exact boundaries in DD01/DD04:

| Opcode | `ServerMsg` / session `Event` | Exact body |
|---|---|---|
|0b3|`EnvDamage(EnvironmentDamage)`|u8 raw source, BC target, u32 amount; full u32 gimmick ID only for source1|
|1bc|`SetBreath { breath: u32 }`|one unsigned u32 for the local character|
|043|`ForceAttackSet { obj_id, on }`|BC object ID, bool|
|078|`UnitInvisible { obj_id, invisible }`|BC object ID, bool|

`EnvironmentDamage { source, target_obj_id, amount, gimmick_id: Option<u32> }` preserves
unsigned values and unknown source bytes. Exactly source1 has the additional field; all other
source values have an eight-byte body rather than twelve. The source1 gimmick ID is a full
u32 (including high bits), unlike the target's 24-bit ID. No raw source is coerced to an enum,
no target is spawned, and the report never subtracts HP or invents death/combat engagement.
The existing points/death events remain authoritative. In particular the lane's reported fall
amount can differ from the applied HP delta when the fall cap/stun path is used.

`CombatStatus` (root export, NetPlugin resource and `WorldState.combat_status`) records only
received metadata. `get(obj_id)` / `by_unit` expose `UnitCombatFlags { force_attack:
Option<bool>, invisible: Option<bool> }`. A player/unit announcement seeds force attack from
its received native flags bit3 (proved in the next section); invisibility remains unknown.
Explicit flags update **known objects only**, and duplicate announcements retain a later
explicit report. No true/false is inferred from buffs or movement. Removal drops that
object's flags; reuse starts from its new snapshot. Character selection, logout and
disconnect clear the resource. Receiving invisibility does not delete a unit or decide
how it should render. There are no new required `Unit` or `PlayerStats` literal fields.

`CombatStatus.breath: Option<u32>` is the last local report, including0 and u32::MAX, without
a guessed maximum, percentage, countdown, underwater state or automatic reset. This packet
contains no object ID. It is scoped to the selected-character/connection lifecycle rather
than assigned to a nearby target, and elapsed time never changes this raw received value.

Native 1.2 reader/consumer evidence (`<client12-unpacked>/x2game_unpacked.dll`):

- EnvDamage factory opcode assignment391ac0bb, vtable399b399c -> reader397c3560 ->
  shared damage record397c01b0. The record reads source via byte slot44, target through
  BC slotscc/d0 with width3, amount via unsigned32 slot3c, then `source==1` gates another
  unsigned32 at397c0250. Handler391d60e0 ->39391720 builds combat presentation for native
  source0 (fall) and2 (drowning); no matching native presentation for the lane's collision3
  was inferred. aa-net exposes every source neutrally.
- SetBreath assignment391b3b6b, vtable399b40b4 -> reader397afd50: unsigned32 slot3c.
  Handler391ddf30 targets the current player and calls3930b050. That native method adjusts
  fields at+0xb24/+0xb28 using the engine clock; reproducing its full breathing model would
  require the surrounding native state/GetBreathTime/UnitBreath contract. This receive cache
  does not mistake the adjusted native field for a directly usable wire countdown.
  Original `unitframe/player_breath_bar.lua` polls `X2Player:GetBreathTime()` for
  `(totalTime,leftTime)` each OnUpdate and uses `DIVE_START`, `DIVE_END`, `LEFT_LOADING` to
  manage visibility. GetBreathTime registration39499367 points to wrapper39495d10. SetBreath
  alone proves neither a dive event nor the total/max value required by this Lua getter.
  Getter3930b660 starts with0/0 when stored start is zero or the actor is absent. It queries
  actor attribute0x5b through390b3a80 for capacity. With unchanged capacity its remaining value
  is `capacity - now + stored_start`; SetBreath sets `stored_start = now - old_capacity + raw`.
  If capacity changes, it rescales elapsed time proportionally before returning total/left.
  This identifies the raw value as remaining at receipt, but the cache deliberately adds no
  timer: establishing dive start/end, actor capacity and its changes belongs to client planning.
- ForceAttack assignment391a99eb, vtable399b3228 -> reader397b31e0: BC then bool slot78.
  Handler391d3c60 looks up the existing object via39360020, stores its flag at+0x3bbc, then
  formats the looked-up object's+8 through39424820 and emits
  `FORCE_ATTACK_CHANGED(stringId, on)`. Original Lua handlers agree: `unitframe/player.lua`,
  `target.lua`, `target_to_target.lua`, `party.lua` take `(stringId,on)`;
  `centermessage/center_message_manager.lua` takes `(id,isNewState)` and filters the player.
  aa-client owns translation to this Lua identifier and presentation event.
- Invisible assignment391aa62b, vtable399b3360 -> reader397c61b0: the BC helper397b27d0
  then bool slot78. Handler391d4b10 looks up an existing unit and calls its virtual setter
  at+4c; it ignores a missing unit. No direct Lua invisibility event name/shape was proved.

All four layouts match the Rust lane writers in `systems/skills/packets.rs`.
`combat::set_force_attack` updates the unit, adds/removes Bloodlust and broadcasts043;
`set_invisible` updates the unit and broadcasts078. Trial paths also send043. Regen
`breath_tick` decrements positive breath by1000 and sends1bc, or applies drowning HP changes
then emits0b3/source2 at zero. `combat::do_fall_damage` applies its points/stun logic and then
broadcasts0b3/source0. `systems/ai/physics::apply_collision_damage` applies damage, computes
actual dealt HP and broadcasts0b3/source3. These are real emitters rather than declarations
or fabricated acknowledgements. No corresponding outbound command was added in this slice.

Offline tests cover independent exact-byte goldens for all four, every truncation, conditional
full-u32 gimmick identity, all256 raw source bytes in one DD04 batch with following flag,
breath and snow packets, known-only flags/false-vs-unknown/reuse/lifecycle, raw breath extrema,
no HP double-subtraction or phantom units, and session forwarding/order with an authoritative
points tail. Natural fall/drowning/force-attack/visibility live QA remains pending while the
lane is stalled; no lane traffic or server changes were made.

Validation: `cargo test -p aa-net combat_status --quiet` passed all5 new targeted tests.
`cargo test -p aa-net --features bevy --quiet` passed240 library +2 scripted loopback
integration tests;9 live lane tests stayed ignored. No dependencies or other crates changed.


## Explicit Bloodlust request and initial flag source (2026-10-17)

`Command::SetForceAttack { on: bool }` / `ClientMsg::SetForceAttack` send CS04f with exactly
one bool and **no object ID**. Both true and false are explicit values. The session requires
an active player (`InWorld`, nonzero self object); otherwise it emits
`ForceAttackRejected { on, message }`. `ForceAttackRequested { on }` means only that the local
socket write succeeded. Neither event changes `CombatStatus`, HP, combat engagement, buffs
or PvP UI. Send failure follows the existing disconnect path. Server SC043 remains the
received flag report, without a request token or a fabricated success/matching revision.

Native action/current state proof:

- Native action-name lookup393f8d20 scans104 entries at39ac8e68; `toggle_force_attack` is
  index39 (string pointer39ac8f04 ->399d5a98). Invocation393f8d70 calls callback array+13c;
  initializer39401d13 installs393f8430 at+1d8 (=13c+39*4), which jumps to39390ec0.
- Sender39390ec0 checks the network/current-player pointers, reads the player's byte+3bbc,
  sends its **logical inverse**, and does not mutate that byte. Packet opcode assignment
 39390ef2 is04f, constructor397abb70 copies one bool at packet+c, vtable399d1c00 points to
  body397b09b0 (bool slot78), common send39390f08. Repeated native toggles before the server
  response therefore request the same value, rather than alternate a predicted sent state.
- That exact current byte is updated by native SC043 handler391d3c60. Initial state also
  comes from SCUnitState: shared reader397c992b/397c995b extracts bit3 of its u16 flags into
  record+2052. Conversion398a222a ->398a2231 copies it into destination+3bb4. Caller3936b114
  passes unit+8 to conversion398a1f90 at3936b122, so the field lands at actual unit+3bbc.
  Initializer398a1aa0 clears the bit's storage at398a1b2f before received snapshot data is
  applied. Other same-offset dword references are different struct fields; this mapping uses
  the proven unit+8 offset rather than conflating those references.

`Unit.flags` already retains the u16 snapshot field, so no new required model literal or
packet read field is needed. `combat_status::FORCE_ATTACK_FLAG = 0x0008` names the proven
bit. New `CombatStatus` entries now use `Some(unit.flags & 0x0008 != 0)` for force attack;
invisibility remains `None` until SC078. Duplicate announcements preserve a later explicit
SC043 report; removal/reuse or a new character starts with its new snapshot. This is received
initial state, never an optimistic acknowledgement. Initial seeding emits no synthetic
`ForceAttackSet` or Lua event. The pre-existing best-effort Unit decoder can leave raw flags0
if its trailing details fail; it has no per-field completeness marker, a separate parser
limitation. This feature does not claim to repair that broader decoder behavior.

The current lane `world/unit_state.rs` character writer emits flags0 and NPC writer emits
0x2000, without serializing runtime `unit.force_attack`. Thus a newly visible lane character
reports false even if the lane's runtime flag was previously true; later real SC043 corrects
the cache. The limitation is preserved and documented, not hidden by guessing a buff-based
value. Native unit flags can carry either value and are decoded accordingly. Lane
`CSSetForceAttackPacket` in `systems/skills/handlers.rs` reads one bool, applies it only to
`active_char`, and calls `combat::set_force_attack`, which updates runtime state,
adds/removes Bloodlust and broadcasts SC043. No active character means no reply. The lane
adds no separate duel/battlefield rejection in this handler; aa-net does not invent one.

Original Lua getter/event boundary for root integration:

- `X2Unit:UnitIsForceAttack(target)` registration394c4d97 points to wrapper394bd710.
  Resolver3945fcd0 must return category1; otherwise the wrapper pushes **boolean false**,
  not nil/no values. It also returns false for kind field+410 !=0 (non-character). Only a
  character kind0 returns its byte+3bbc as a bool. `CombatStatus` retains raw unit reports;
  aa-client applies that character-only/missing-unit behavior for the Lua getter.
- `unitframe/unitframe.lua::UpdatePVPIcon` first calls `GetUnitId(self.target)` then
  `UnitIsForceAttack(self.target)`, so initial frames use the snapshot/cache through the
  getter. Subsequent native SC043 emits `FORCE_ATTACK_CHANGED(stringId,on)` as documented
  above. `ShowPVPIcon` verifies stringId matches that frame's target before showing the icon.
- Every SC043 event, including repeated identical values, is forwarded in wire order. A
  resource's value may already be the last DD04 transition when app Update runs, so Lua
  transition events must use each event's `on` argument. A send alone must not emit the
  acknowledgement event, change cached flags, alternate a guessed pending toggle or alter
  combat engagement. Root owns hotkey/Lua dispatch; aa-net intentionally exposes explicit
  SetForceAttack rather than a second local toggle state machine.

Offline tests add exact false/true frame goldens and lane-reader consumption, isolated bit3
seeding with other raw bits, duplicate/reused-object precedence, active-player preflight,
failed-send behavior, repeated queued values without optimistic mutation, and authoritative
true/true/false batch order followed by later state. Live combat/Bloodlust verification remains
pending while the lane is stalled; no lane traffic or server modifications were used.

Validation: `cargo test -p aa-net --features bevy --quiet` passed242 library tests and2
scripted loopback integration tests;9 live lane tests remained ignored. No dependency,
server or other-crate changes were needed.


## Native cooldown lists (`SCCooldowns 0x04a`)

The exact original receive body is `i32 skillCount`, that many triples
`u32 id, u32 total_ms, u32 remaining_ms`, `i32 tagCount`, then the same triples for tags.
`skill::{SkillCooldowns, Cooldown, COOLDOWN_LIMIT}` are also reexported at crate root.
`ServerMsg::Cooldowns` and `Event::Cooldowns` retain both lists, arrival order, duplicate IDs,
zeros and all original u32 values. A remaining value greater than its total is not clamped.
There is no object/character ID, acknowledgement, global cooldown or learned-skill list here.

Source proof: factory `391a9eb0` assigns opcode at `391a9ece`, allocates `0xe24` bytes and
uses vtable `399b3b94`. Body reader `397b64d0` calls record reader `397b2350`. It reads signed
counts with stream slot `+0x4c` and three u32 words per entry with slot `+0x3c`, with no 10 ms
conversion. Its first fixed array starts record+4, the second count is record+`0x70c`, and its
array starts +`0x710`: 150 triples per list. The original reader has no safe count clamp;
aa-net rejects negative counts and counts above 150, rather than reproducing an out-of-bounds
native write or guessing a DD04 tail boundary. Empty bodies are eight zero bytes; empty and
nonempty lists each consume only their exact payload.

Native `OnCooldowns` (`391d43c0`, name at `399b7d28`) only calls into an existing current
player's manager (+`0x2c48`). `39899d40` appends skill entries to manager+`0x720` (count+`0x71c`)
and tag entries to +`0x14` (count+`0x10`), each capped at 150. It does not clear or deduplicate;
an empty packet is not a reset. It transforms each wire `[id,total,remaining]` into internal
`[id,remaining,total]`. `39899e50`, called by unit update `3936a280` with clock delta from
`3936a7f8`/`3936a836`, subtracts elapsed time from remaining with a zero floor. These are the
same millisecond counters used for cast/DB cooldown presentation, not server epoch times.
`3989a290` queries all ordered skill tags followed by direct skill entries and chooses the
strictly greatest remaining time; a tie keeps the first candidate and its total.

`state::Skills` retains received-only entries as
`received_skill_cooldowns` and `received_tag_cooldowns`, each
`Vec<state::ReceivedCooldown { cooldown: Cooldown, received_at: Instant, reset: bool }>`.
`received_cooldown(skill_id, tags: &[u32]) -> Option<(Duration, Duration)>` returns
**remaining then total**, using the native maximum/tie order. `received_cooldown_at` accepts
an explicit `Instant`; `ReceivedCooldown::left_at` exposes one entry's timer. The helpers
subtract monotonic elapsed time from the received remaining value, never invent a DB duration,
rewrite original wire triples, create a cast or infer a global cooldown. Timing begins when the
app folds the event; network/app-pump latency compensation is not guessed.

`cooldown_with_total(skill_id, tag, cooldown_ms)` returns `(remaining, total)` from the greatest
received remaining or the previous cast/DB estimate, using one clock sample. Equal remaining
keeps the received winner and its native tie order. Received total zero is preserved.
Existing `cooldown_left` wraps that query for HUD eligibility, so consumers do not duplicate
last-used/reset logic or guess the winning total. Its
single tag argument retains its existing meaning (zero = none); callers with the native full
ordered tag list can use `received_cooldown`. Observed casts are still separate from the
received-only vectors. This milestone does not reconstruct the full native cooldown manager's
DB tag generation or its compaction during later local casts.

`SkillCooldownReset` only changes an own player's timers. Native helpers `39899ec0` and
`39899f00` stop at the **first** matching skill/tag entry, including a previously reset first
entry; aa-net marks only that received entry `reset=true` and retains the original triple.
The existing use-derived reset behavior remains. `gcd=true` now also clears `last_cast` and
`last_cast_skill`, matching handler `391d64b2` -> manager `39899d00` (global remaining/total
set to zero). A tag reset is not guessed to clear unrelated direct received skill entries.

The cache requires an existing own `PlayerSpawn` (`obj_id != 0`); earlier packets are still
forwarded as events but are not assigned to a speculative character. `PlayerSpawn`,
`CharacterSelected`, `LeftWorld` and `Disconnected` clear it. No snapshot is copied across
characters or reconstructed on re-entry. The exact native current-player creation timing before
the app's PlayerSpawn event is not claimed here.

Lane `systems/skills/packets.rs:395..407` declares only `SCCooldownsPacket` with two i32 zero
counts. A source-wide search finds no runtime emitter, only definition/test/opcode references.
Thus parsing nonempty native data and received-only timers are verified offline; the lane does
not currently demonstrate persisted cooldown restoration. No lane sockets were opened while
it remains stalled. Tests cover native golden triples, all truncated prefixes, independent
empty/full lists, invalid signed counts, capacity 150, exact DD04 tails, session event order,
append/no-clear semantics, expiry, duplicate/tie precedence, first-match resets and lifecycle.

Validation: `cargo test -p aa-net --features bevy --quiet` passed 252 library tests and two
loopback integration tests; nine lane-dependent tests stayed ignored.
`cargo check -p aa-net --features bevy --examples --quiet` and `git diff --check` passed.

### Environment presentation follow-up (read-only native contract)

`39391720` handles only source 0 (fall) and 2 (drown): it builds a generic record whose source
and target IDs both equal the environment packet target, and passes it to `3940a3e0`. That
function resolves both names using `393905b0`/`39390660` (literal `unknown` on missing unit),
formats the target ID using `39424820`, and dispatches `COMBAT_MSG` through
`39424300(0x17, args)` with twelve arguments:

```text
(targetIdString, "ENVIRONMENTAL_DAMAGE", targetName, targetName,
 "FALLING" | "DROWNING", amount, "HEALTH", "HIT", 0, "idle", 0, false)
```

`amount == 0` suppresses this event (`3940a9cf..d4`). The presentation amount is native
signed-i32 -> f32 -> Lua number (`3940a9dd` uses `cvtsi2ss xmm0,eax` without an unsigned
correction), while `EnvironmentDamage.amount` retains the original unsigned u32. Source-name
table `39ac93f8` maps index0=`DROWNING`, index1=`FALLING`; event-name table entry
`39ac93c0` is `ENVIRONMENTAL_DAMAGE`. Zero damage type goes through `3940a1a0(0)` = `idle`.
There is no `COMBAT_TEXT` call in this handler; original
`x2ui/combattext/combat_text.lua` consumes this COMBAT_MSG event and presents environment text.
Sources 1 and other values return without this presentation. This is an aa-client integration
contract, with no inferred HP subtraction or extra network traffic.


## Conflict-zone state (`SCConflictZoneState 0x0ee`)

`conflict::ConflictZoneState { zone_group_id: u16, state: u8, end_time: u64 }` is also
reexported at crate root. Its body is exactly eleven bytes: `u16 group, u8 state, u64 endTime`.
`ServerMsg::ConflictZoneState` / `Event::ConflictZoneState` forward every report in wire order,
including duplicates, zero/max group IDs, unknown state bytes and all timestamp bits. The raw
native timestamp is not interpreted as a local deadline. `lane_end_time_seconds() -> i64`
explicitly reinterprets its bits for the current lane writer, which uses signed Unix seconds;
zero there means no scheduled transition. Negative/high-bit values are retained, never clamped.

Native factory assignment `391ae37b` uses vtable `399b369c`, whose body reader is `397c2cc0`.
It reads group through unsigned-short slot `+0x40`, expands the unsigned-byte state into an
internal enum-sized field via slot `+0x44`, then reads unsigned 64-bit endTime through `+0x38`.
There is no BC object ID, instance ID, extra conditional tail, count or time scaling. Native
`OnConflictZoneState` (`391d9600`, string `399b877c`) forwards those three values to `3902a000`.
That helper inserts/updates the global map `39adb0f8`, keyed by unsigned-short zone-group ID,
retaining state and endTime even when the local player/UI presentation context is absent.
No unit spawn or attack-permission acknowledgement is proved by this packet.

The lane agrees on byte order and size in `packets/world.rs:327..347`; its signed i64 writer
preserves the same 64 bits as the native unsigned reader. `world/session.rs:514` sends all
initial conflict records, and `systems/social/zone_conflict.rs:303` broadcasts authoritative
switches. That system stores an internal Unix-millisecond scheduler separately, but writes
`next_state_time` in **seconds** (`Entry::set_next` uses `ms.div_euclid(1000)`), with zero for no
scheduled switch. This lane interpretation does not establish a broader native time encoding or
justify extrapolating another state after the reported time passes.

`ConflictZones` is a plain/Bevy resource, reexported through `state` and crate root, and stored
as `WorldState.conflict_zones`. Its public `by_group: HashMap<u16, ConflictZoneState>` and
`get(group: u16)` retain the latest received value for each group. The protocol's u16 key space
bounds storage to 65,536 groups. A duplicate exact report does not flag the resource changed,
but the session still forwards its event. Changed state or timestamp replaces only that group;
other groups stay untouched. The cache has no timer tick, local state transitions, zone lookup,
permission boolean or UI acknowledgement.

This state belongs to the world, rather than one character. It accepts reports without a local
player and preserves them through `CharacterSelected`, `PlayerSpawn`, `EnteredWorld`, lobby
notifications, map movement and ordinary world-state events. It clears on `LoginSucceeded`,
`ReadyToConnect`, `LeftWorld` and `Disconnected` to scope it to the current account/world
connection; this is the app cache lifecycle, not an invented native packet. In particular,
initial records arriving before character/spawn events are never discarded by those events.
Native zone/peace permission predicates still require separate source audit; a state byte does
not automatically permit or prohibit a skill under Bloodlust.

Offline tests cover native golden bytes, every truncated prefix, all 256 state bytes, raw
unsigned/signed timestamp boundaries, DD04 following breath/unit-points packets, duplicate and
updated-group ordering, pre-spawn session forwarding, no phantom units/entry/permission events,
state retention across character/spawn notifications and account/world cleanup. No stalled-lane
sockets, grants or server changes were used.

Validation: `cargo test -p aa-net --features bevy --quiet` passed 258 library tests and two
loopback integration tests; nine lane-dependent tests stayed ignored.
`cargo check -p aa-net --features bevy --examples --quiet` and `git diff --check` passed.


## Local underwater reports (`SCUnderWater 0x187`)

`ServerMsg::UnderWater { start: bool }` / `Event::UnderWater { start: bool }` consume exactly
one bool, with no object/character ID or extra tail. Native factory assignment `391b3aeb` uses
vtable `399b3d74`; body reader `397afb00` reads bool through stream slot `+0x78`. Lane
`packets/g2c_misc.rs` writes the same single bool. Actual sender
`systems/skills/water.rs::set_player_under_water` reports changes of the local player's
underwater state. The lane derives that from its water/position checks, including ship-deck
probe and surface hysteresis; aa-net does not manufacture the flag from an arbitrary Z value.

`CombatStatus.under_water: Option<bool>` retains the latest local report independently of the
raw `breath` counter. `None` is unknown, `Some(false)` is an explicitly reported surface state.
It accepts reports even before a unit is known and does not create a unit. Repeated values leave
resource change detection alone, but **every repeated network event is forwarded**. No timer,
capacity, refill, dive Lua event or HP change is synthesized. A zero `SetBreath` does not end a
dive; a surface report does not overwrite the raw breath counter. The field clears together
with existing local CombatStatus state on `CharacterSelected`, `LeftWorld` and `Disconnected`.
No new Unit fields, command or default capacity are needed by this receive boundary.

Native `OnUnderWater` (`391dde80`, string `399b9360`) calls manager `3930b600(start)` and then
fires **zero-argument** `DIVE_START` for true (event index `0x151`) or `DIVE_END` for false
(`0x152`) through `39424220`, even for repeated reports. It does not test a previous cached
boolean before dispatch. `SCSetBreath` (`391ddf30`) is a separate handler and does not emit
these Lua events. Net forwards the two packet families separately; aa-client owns that exact
Lua bridge and presentation controller.

### Read-only native breath/capacity contract

Original `x2ui/unitframe/player_breath_bar.lua` calls **`X2Player:GetBreathTime()`**, with no
target argument, receiving `(totalTime, leftTime)`. `DIVE_START` calls ShowAll, `DIVE_END` calls
HideAll, and `LEFT_LOADING` hides then shows if total is nonzero. OnUpdate sets the bar range
`0..total` and its value to `left`. Registration `39499367` binds wrapper `39495d10`, which
calls manager `3930b660` and returns total then remaining via `3905aca0`. Both numbers are
native unsigned-u32 -> f32 -> Lua number, including f32 rounding, not integer seconds.

The manager at current connection+`0x40` holds start/reference counter S at+`0xb24` and cached
capacity C at+`0xb28`. `3930b600(true)` requires an existing current actor, then records S=now
and C=actor attribute `0x5b` (91); false clears S, retaining the cached C. With no current
actor the helper does not alter the timer, although its caller still dispatches the dive event.
`3930b660` returns `(0,0)` when S is zero or there is no actor. Otherwise it re-queries current
capacity. If capacity changed, it preserves the **elapsed fraction** by rebasing
`S = now - trunc(((now-S) as u32 / oldC) * newC)` and records newC. Its remaining counter is
`(C - now + S) as u32`, with total C. Native `SCSetBreath` handling calls `3930b050`, which
separately rebases `S = now - cachedC + receivedBreath`; it does not itself prove diving.

This getter contains no visible lower/upper clamp on the final arithmetic and no explicit
zero-old-capacity division guard. These edge behaviors remain explicit; this milestone does
not claim an independently safe native countdown or insert a guessed timer into aa-net.
Native timer start/rebase/rescale and Lua event delivery belong to later aa-client presentation.

Native mapping `3956e961..976` identifies `lung_capacity` as attribute 91. The unit getter
`390b3a80` calls attribute component `398a0bc0`. For characters, dispatcher `398b4e80` case91
falls through to generic `398a0d60`; table `398a11a4[91]` selects branch `398a10e5`, returning
`0xea60` = **60,000 ms base capacity**. Outer `398a0bc0` then applies additive and percentage
bonuses and calls final attribute-limit helper `395729f0`. Thus 60,000 is a proved native base,
not a universal final capacity. The read-only compact database has no explicit
`unit_attribute_limits` row91; native default limit initialization was not inferred here.

The lane agrees on attribute 91 and base: `skills/stats.rs` computes
`cwb(60000, LungCapacity) as u32`. `skills/regen.rs::breath_tick` runs each second, subtracts
1000 while underwater and alive, then sends `SCSetBreath`; at zero it sends drowning damage
separately. Surfacing refills its internal breath to calculated capacity and sends UnderWater
false, without manufacturing a SetBreath refill packet. `AddBreath` updates the server's
counter under water without an immediate counter packet; a later regular tick reports it.
These server behaviors do not justify inferred client HP changes or a fake maximum in net.

Offline tests cover true/false one-byte native bodies, truncation and DD04 tails, repeated
reports, unknown versus explicit false, independent breath values, local lifecycle cleanup and
session order without synthetic entry/unit/capacity/Lua events. No lane sockets were opened.

Validation: `cargo test -p aa-net --features bevy --quiet` passed 261 library tests and two
loopback integration tests; nine lane-dependent tests stayed ignored.
`cargo check -p aa-net --features bevy --examples --quiet` and `git diff --check` passed.

### Received auto-use AA Points setting (2026-10-17)

`Command::SetAutoUseAaPoint(bool)` sends native CS0x133 with exactly one byte.
`Event::AutoUseAaPointRequested` is emitted only after a successful transport write;
`AutoUseAaPointRequestRejected` covers inactive-character requests. Neither is an ACK.
There is no matching handler registered in the current read-only bevy lane source, so
server persistence/live acceptance remains unverified; no success is synthesized.

`Inventory.auto_use_aa_point_raw` hydrates from the existing selected character's
`character.auto_use_aa_point`. `auto_use_aa_point()` uses raw equality to one. Received
`ItemTask::AutoUseAaPoint(u8)` (SC item action 18) preserves every byte and clears
`pending_auto_use_aa_point` after folding the whole task batch; repeats still return a changed/report notification. The app
must fire the original `CHANGED_AUTO_USE_AAPOINT` from received actions, not the request.
`can_set_auto_use_aa_point` implements the raw-value/single-pending UI guard. The command
is transport-level and does not enforce this app guard. Failed/absent connections produce a local rejected event and do
not produce a requested event or latch pending. Selection resets pending and hydrates;
left-world/disconnect clear both values, including inventories with no allocated slots.

Original `<client12-unpacked>/x2game_unpacked.dll` (image base 0x38ff0000): wrapper3944c8a0
calls394e98d0. Guard394e96d0 compares raw inventory+C340 with requested integer, checks
pending key3a5b9d38 in map3a5b9cd8, then marks it with397d2200(...,true). Rejection calls
397d2200(...,false) and emits error0x14. Accepted construction sets vtable399e5f14,
opcode0x133, ctor397acee0 stores the argument at packet+C, and39186810 sends it.
Vtable serializer397bc8f0 reads/writes one byte at packet+C; getter397ccd50 returns true
only when inventory+C340 equals one. Native action dispatch branch397d1222 clears its
+1A8 member with3924cd30 then calls397cd940 on action+8. That function calls its sink's
virtual slot zero with global3a5e11d0 and then assigns the action raw dword into
inventory+C340 at397cd95e. Sink timing is now proven: caller394eff00 constructs vtable399e6310; virtual[0]
394e8600 only inserts the packed location into a map. After Apply returns and C340 is
written,394f007f calls394e7140. Action18 branch394e758c reads397ccd50 and emits Lua
0x24a at394e75af through39424240. Registration392165d6 names event0x24a
`CHANGED_AUTO_USE_AAPOINT`. Thus app dispatch observes the received getter after write.

SC0f5 `CraftItemUnlock { kind, slot }` / `Event::CraftItemUnlocked` is a received inventory
lock release, not recipe completion or AA-point value acceptance. Native packet factory
391ae560 sets opcode0xf5 and vtable399b3738; serializer397b4c60 zeroes packed bytes+C/+E
and reads bytes+D/+F, producing `[0,kind,0,slot]`. Handler391d9700 (`OnCraftItemUnlock`)
calls inventory397d2200(packed_key,false). Static initializer39991ec0 proves the AA-point
key3a5b9d38 is `[0,0xf0,0,0]` (0xF000). Only that explicit unlock key clears the pending AA-point
request; unrelated slots/kinds preserve it, and no unlock changes the received getter
or justifies a `CHANGED_AUTO_USE_AAPOINT` event. Repeated received unlock events remain
visible to applications even when no local request is pending.

The regular received value action also releases pending at the end of the item-task batch:
394eff00 gathers the sink map, and394f021b passes it with false to397d22b0; its loop calls
397d2200(key,false) at397d22ce. Action18 had inserted0xF000. Native per-action Lua change
callbacks precede that final map release; aa-net's event fold exposes the finished batch.

Validation: `cargo test -p aa-net --features bevy` passes 267 library tests and two local
login loopbacks; nine lane-dependent tests remain ignored. Unlock tests cover raw packed
keys, wrong kind/slot isolation, truncated payloads, following DD04 packet alignment,
repeated notification ordering, getter immutability, and no-send rejection notifications.
No lane transaction was attempted during the stalled game-session condition.

### Crafting and proficiency request outcomes (2026-10-17)

`ExecuteCraft` and `Actability` now report their existing local rejected events when no
character is active or the transport write cannot occur. `CraftRequested` /
`ActabilityEvent::Requested` still mean only a successful socket write. Proficiency
requests are gated to `Phase::InWorld`, matching crafting; inactive requests cannot
reach an available game socket. Callers can release a local pre-send queue latch on
either requested or rejected notifications without fabricating a recipe result, earned
points, a tier change, or expert capacity. Received tier/expand ACKs remain the only
corresponding authoritative state changes. Generic server errors are still generic;
no association with an arbitrary pending craft/proficiency request is invented.

The local crafting loopback verifies invalid fields, inactive phase and missing
connection all reject, while successful batches keep their exact native wire bytes and
only report a local send. A proficiency loopback checks all three commands across idle,
lobby and missing-transport phases, then verifies rejected lobby requests never enter
the socket and successful requests preserve frame order and leave points/capacity unknown.

Validation for this request-outcome followup: `cargo test -p aa-net --features bevy --lib`
passes 268 tests. The preceding full crate run passed both login loopbacks, with nine
lane-dependent tests ignored. No live crafting/proficiency transaction was attempted.

### Owned mate rename request (2026-10-17)

`Command::Mate(MateCommand::Rename { tl_id, name })` sends CS0a6: u16 timeline ID,
u16 UTF-8 byte length, then the exact name bytes. `MateCommand` is now `Clone` rather
than `Copy`; its validate/opcode/write methods borrow the command. Existing controls
retain their bytes and callers clone when retaining a sent request. A local Requested
notification never changes a name. Existing `UnitNameChanged` updates only an existing
unit by runtime object ID; no mate/unit or rename acceptance is invented.

Native `SetMateName` binding3906b6f7 names wrapper39069860. It performs the game's imported
name check (3906988f), then uses a 0x81-byte C buffer. At3906995c it sets vtable399a0738
and opcode0xa6, obtains the mate timeline at object+14, calls ctor397ac250, then sends
39186810. Ctor397ac250 copies at most0x81 bytes including termination. Serializer397ba7e0
uses virtual+40 for TL, string length at397ba824 and virtual+dc for string data; its read
branch caps the string at0x80 bytes. The lane `vehicles/packets.rs::CSChangeMateNamePacket`
confirms u16 TL/string, and `mates.rs::rename_mount` checks the current owned mate and
server name rules before broadcasting `SCUnitNameChangedPacket`. The transport API
rejects zero TL, embedded NUL and names exceeding128 UTF-8 bytes to avoid native
truncation. It does not duplicate or guess the imported language/name policy: whitespace,
empty names and exact bytes remain the caller/server's policy decision.

Tests cover exact multibyte UTF-8 frame bytes,128-byte boundaries, NUL/oversize rejection,
unchanged existing mate controls, local send without optimistic name mutation, received
name forwarding, and ignoring names for unknown units. No natural mate rename was
attempted against the stalled game-session lane. Mate equipment remains a separate
larger scope: native full-item request echo and received bag/mate reconciliation need
source proof beyond the lane's known mismatching settlement representation.

Validation: `cargo check -p aa-net --features bevy` and the full270-test library suite pass
for mate rename; the earlier two login loopbacks remain green. The source tests use Rust
Unicode escapes so PowerShell's pipeline encoding cannot change the multibyte goldens.

### Stablehand summon-item revival (2026-10-17)

`MateCommand::ReviveItems { npc_obj_id }` sends CS0b5 with a nonzero three-byte BC NPC
object ID. This is a separate companion service from `RepairCommand`; it supplies no
local quote, item mutation, revive completion or special acknowledgement. A successful
send uses the existing `MateEvent::Requested`. Existing authoritative `ItemsChanged`
detail updates change `ItemDetail::Mate { exp, dead, level }` in the bag/bank. Public
`mate::REPAIR_PETS_TASK = 58` names the lane's reason value without inventing a new ACK.

Native `RepairPetItems` binding3944873e points to wrapper39440d80. It resolves the current
interaction object, requires NPC kind1 at+410, resolves NPC data, and checks the stabler
flag at+e4 before setting vtable399d9cb0 / opcode0xb5. It passes the NPC's runtime ID at+418
to ctor397ac5f0, which stores packet+C, and sends39186810. Serializer397c79d0 delegates to
397a7710; that helper passes length3 at397a7749 to its raw read/write virtual method,
proving BC width rather than a u32. The lane `economy/mate_equipment.rs` reads `read_bc`,
and `vehicles/mates.rs::revive_pet_items` checks current interaction, owned dead summon
items, stabler role, range, unsummoned state and funds. It debits through actual money
tasks and sends SCItemTaskSuccess with reason58 and item-detail updates; the existing
generic MATE_RESURRECTED server message is not remapped to an arbitrary request ACK.

Tests prove the exact request bytes and bounds, no dead-flag changes from a local send,
and received reason58 padded detail updates for both bag and bank while preserving the
reported experience/level. The lane's floating-point revive-cost function is not exposed
as a native quote: original UI rounding/order needs separate proof before a quote API.
No pet was killed, revived, granted, or transacted against the stalled lane for this work.

Validation: the built aa-net test executable was copied to the ignored aa-net extraction
directory and run directly, avoiding another shared cargo lock; all271 library tests pass.
The focused `cargo test -p aa-net --features bevy --lib stablehand` also passes.
Earlier full crate verification covered both local login loopbacks.

### Raw initial configuration and received feature bytes (2026-10-17)

`ServerMsg::InitialConfig` / `Event::InitialConfig(InitialConfig)` decode SC006.
`WorldState.game_config` and the Bevy `GameConfig` resource retain an optional received
snapshot. There are no synthetic feature/service defaults. Host bytes and all feature
bytes are preserved, including NUL/non-UTF-8 host bytes and unknown feature bits; service
flags remain seven raw bytes rather than inferred application permissions. Missing
feature bytes stay missing. `player_level_limit()` / `mate_level_limit()` expose optional
received bytes1/8 according to the lane `FeatureSet` schema, retaining explicit zero.
The resource resets on a new login/account/game connection, kick or disconnection. It
retains the config across character selection, spawn and return to the connected lobby.
Identical received reports are forwarded as events even when the snapshot is unchanged.

Native factory391849b0 sets opcode6/vtable399b3024, whose serializer is397b2a00. It reads
host string with a0x103-byte limit, then the fset blob with0xb-byte limit. Both are
u16-length-prefixed fields, matching the lane `packets/g2c.rs::SCInitialConfigPacket`.
Nested397b1c20 reads signed candidate count and clamps values>=100 to the static100 at
39a12604; negative counts read zero entries. Each code uses the u32 primitive. The raw
signed count is retained alongside exactly max(0,min(count,100)) code values. Nested
397b1fb0 reads signed initLp and seven bool-width bytes, named by the original strings:
canPlaceHouse, canPayTax, canUseAuction, canTrade, canSendMail, canUseBank, canUseCopper.
Serializer397b2ab6 then reads secondPriceType(u8), secondPasswordMaxFailCount(u8), and
idleKickTime(u16). Oversize host/fset payloads reject before exposing a snapshot. Native
constructor defaults are not silently inserted for short received feature arrays.

The lane `data/features.rs` sets FeatureSet byte1 from player max level and byte8 from
mate max level, supplied by `data/experience.rs`. This does not yet prove how the native
runtime field is populated: `OnInitialConfig`391d1d20 contains a protected body between
391d1d69 and391d1f5a, guarded by39181fb0/39181fc0. The unprotected serializer proves the
complete packet boundaries independently, but the protected handler's copy into runtime
client+1C remains unresolved. No cap-aware pet XP fold or guessed50-level limit is added.

The downstream original pet XP path is concrete: `OnExpChanged`391d9bd0 resolves the
announced unit and calls39356f40. Its mate-kind5 branch requires a current owned mate and
matching object ID, then3933a890 updates manager+3C using3989a480 and the limit obtained
by3989a4d0(runtime client+1C). For delta<=0,3989a480 performs wrapping addition; for positive
delta, a value already>=cap is retained, otherwise wrapping addition is signed-clamped
to cap. Lookup3989a4d0 calls39620cc0(level) and reads record+8. The catalog mapping is also proven: loader39620b7c uses SQL399fa2ec,
`SELECT id, skill_points, total_exp, total_mate_exp FROM levels`;39620c1b reads column3
and39620c39 stores it at record+8. Thus the XP cap is `levels.total_mate_exp` at the
runtime max-mate-level. The remaining proof is the protected initial-config runtime-level
assignment. A naive accumulation would overcount at the cap because
the lane sends the original unclipped positive delta. `Mates.spawn.experience` therefore
remains spawn metadata pending the source-proven cap-aware integration, rather than
silently claiming a correct current XP total.

Validation: `cargo test -p aa-net --features bevy --lib` passes all275 tests, including
raw config count/buffer bounds, every payload truncation, DD04 continuation, repeated
session event ordering, optional feature limits, snapshot replacement and account/game
connection lifecycle. This includes the plugin/resource integration under the Bevy feature.

### Local outcomes for lost repair/bank/mate transport (2026-10-17)

The existing Bank and Repair command branches already rejected inactive-character use
but silently dropped a missing/failed transport write. Mate commands also silently
ignored inactive phases. These paths now emit their existing typed `RequestRejected`
notifications with the original command. A successful write still emits only Requested;
no wire bytes or received-state/ACK semantics changed. This includes separate stablehand
revival and mate rename, so application queue feedback cannot disappear on an unsent
request. Generic later server errors retain their existing unassociated event behavior.

The offline phase/missing-transport regression covers all three command families across
idle, lobby and in-world phases, ensuring rejected requests do not create pending repair
success, bags, or owned mates. Existing connected repair/bank and mate control loopbacks
continue to assert exact native frames, wrong-phase requests absent from the socket,
and genuine received-state mutation only.

Validation: `cargo test -p aa-net --features bevy --lib` passes276 tests for the final
repair/bank/mate rejection followup; `git diff --check` is clean. No lane mutation or
new wire/result semantics were introduced.

### Native crafting manager and proficiency bridge audit (2026-10-18)

Read-only source: client12 `x2game_unpacked.dll`, image base38ff0000. No new wire API or
completion result was added. The existing ExecuteCraft request carries the entire batch
count once: wrapper39460930 rejects count<=0, manager394d7650 refuses an already-working
batch, stores recipe/count/doodad, registers craft-skill callbacks and invokes validator
394d6dc0. Sender394d1340 builds CS0f8 with that complete count. Update394d32b0 only manages
local readiness/waiting flags; it never sends the next craft. The material-warning
confirmation394d6ab0 can send the same original full-count request after acceptance.

Manager39affa18 stores remaining signed count at+8. IsWorking394d0bb0 is precisely
count>0; GetLeftBatchCount394d0bc0 returns that count. Callback394d53e0 accepts only the
local player's skill carrying CraftEffect kind0e, with nonempty skill metadata/caster guards.
Started success emits CRAFT_STARTED(remaining) only for the first accepted start. Fired
success latches completion. Ended decrements when Fired succeeded; positive
remaining emits CRAFT_ENDED(remaining), exhausted/non-fired emits CRAFT_ENDED() with no
argument. Non-fired/Stopped readiness resets the batch on the next update. Started
failure resets immediately and emits CRAFT_ENDED(). Native event IDs are1ef/1f0.
StopBatchCrafting39460870 calls manager394d1870(false), issuing existing stop-casting
requests for active cast slots; it does not immediately decrement/reset or emit a
separate cancel event. Do not resend ExecuteCraft in response to each CRAFT_ENDED.

SC1bf handler391e0840 invokes394d2370: each resolvable item in its bounded20-entry list
is independently formatted through394e4ef0 and emits CRAFT_FAILED(formattedString),
event77, inside the loop. This is neither a joined item list nor a batch completion
notification. The exact item-link markup produced by the formatter remains untraced.

GetCraftMaterialInfo39462600 walks every material of a known recipe without a12-row
cap or grade-alternative filter. A missing item descriptor logs and returns nil for the
entire getter. Each returned
row has amount/count/mainGrade/item_info; required grade-1 accepts all grades, otherwise
matching is exact. Counts sum physical eligible bag stacks. Predicate397cc350 excludes
an item only when item flag04 and received feature bit45 (feature byte5 bit20) are both
set. A representative matching item supplies item_info, otherwise the native getter
constructs a synthesized item descriptor with count0. Catalog orphan craft_id0 rows
are not children of any real recipe; the largest known valid recipe has8 materials.

GetCraftBaseInfo39461860 reports ac_type from crafts.ac_id (recipe+4), independently
of the skill's proficiency-group field+C. Labor uses skill consume_lp(+78), received
proficiency points plus unit_attr_id bonus and spatial/context modifier3932a190, and
received grade. Helper397a9420 selects current grade only when effective points reach
that row's up_limit; otherwise it selects previous u8 grade. It computes native f32
floor((1-f32(advantage)*0.01)*f32(baseLP)+0.5), minimum1, with zero only for the invalid
group sentinel. Native x86 Unicorn execution (only exact static expert lookup and
security-cookie check stubbed) confirms (baseLP,points,grade)->neededLP:
(100,10000,1)->100; (100,19999,1)->100; (100,20000,1)->95;
(10,20000,1)->10; (0,20000,1)->1; (100,30000,2)->90.
The ignored reproducible helper is data/extract/aa-net/craft_lp_oracle.py.

Expert loader395b45c0 queries expert_limits ORDER BY up_limit ASC. At395b475b it
recognizes SHOW as T/t/1, skips hidden rows at395b4789, and assigns sequential u8 grade
keys starting0 only to inserted visible rows. The current catalog IDs1..7 therefore
map to grades0..6; hidden IDs8..31 are absent from this native map. GetMaxGrade397a7d00
returns map count-1, so6 here. GetGradeInfo3943b170 keys are grade/name/expertLimit/
upLimit/advantage/castAdvantage/colorString/gaugeColorString/upCurrency/upPrice/
downCurrency/downPrice. expertLimit adds received expanded-expert count; colors use
uppercase %08X. Playerflags1000 controls DB currencies/prices versus currency0/string0.
Missing grade uses the native fallback record (upLimit10000, other numeric values0,
currency-1) rather than nil; transport state may still legitimately be unknown.

GetMyActabilityInfo3930c730 returns nil for an unknown static group. point is the
received proficiency point; modifyPoint is the additional unit_attr_id bonus plus
3932a190 spatial/context modifier, not another copy of received points. Native
missing received map values default0, while aa-net deliberately retains loaded/None
information for an integration to distinguish absent authoritative state.
CanUpgrade397a8e60 requires the current row up_limit, a next visible grade, then checks
next-grade expert_limit plus received expansion against a count of eligible groups
whose received grade is >= the target. Count397a8760 excludes special groups through
static IDs and descriptor flags; those encrypted static ID values remain unresolved.
Do not substitute equality-at-grade or all-group counts. CanDowngrade397a8f90 requires
an existing received entry with step>0. These getters do not acknowledge a request.
GetExpandExpertInfo3943ba20 looks up expansion key received_count+1; missing yields
nil/error1fd. It returns requireLP, optional requireItemType/requireItemCount, and
remainExpandCnt by scanning subsequent consecutive entries; no affordability check.

GetActabilityCategoryCraftTypesByTypes394612c0->394d84b0 sorts copied recipe IDs by
visible_order ascending, then unsigned recipe ID ascending (comparator394d1380).
Category comparator394d13c0 likewise sorts category visible_order then unsigned ID.
The named EnumActabilityCategoryTypesFromCurrentDoodad and EnumCraftTypesFromCurrentDoodad
exports are no-op wrappers39460830/39460850 in this build, returning no values. Their
Get* counterparts have separate implementations; no enumeration behavior is inferred
from the export names. Remaining audit boundaries: full current-doodad category
population/filtering, special proficiency exclusion IDs, exact failure link markup.

Failure-link followup: formatter394e4ef0 uses literal399e6134 `|i%d,%d,%s;`.
Its arguments are copied descriptor item-type at+8, grade byte+C and the encoded
192-byte item descriptor. Before native six-bit encoding394df700 it clears the first
8 bytes and forces descriptor count+10 to1. The outer syntax is proven; faithful
payload construction still depends on the complete native item-descriptor layout.

### Proficiency list and natural workbench entry followup (2026-10-18)

GetAllMyActabilityInfos3943a720 enumerates static groups, filtering only record+C.
Loader3982e080 proves this is skill_page_visible (SQL column3); visible (column5)
is a separate record+14 field. Results form a compact1-based table, retaining the
single-info name/point/modifyPoint/type/grade shapes. This getter does not exclude
language groups. IsLanguageActability3943afd0 calls397a7c60, comparing exactly the ten
globals39acb624..648; their runtime ID values remain unproven.

GetActabilityCountByGrade3943b6f0 calls397a86b0 and counts EXACT received grade equality
at397a8700, using the same visible/special-group exclusions as the different upgrade
count397a8760 (which counts >= target grade). GetMinActabilityPoint3943ab70 returns0
for received grade0, otherwise previous visible grade up_limit. GetMaxActabilityPoint
3943ac40 returns current visible grade up_limit. Missing expert lookup397a8d40 returns
10000. CanExpandExpert3943b940 checks only that the next expansion row exists through
397a9540(received expanded count); it performs no LP/item affordability test. Received
state availability remains an independent aa-net concern, not an invented zero value.

Workbench entry394dc360 takes a real doodad objectID and resolves it through39035110.
If absent it emits native error3 and does not create a context. Otherwise it installs
interaction(kind3,objectID) via393b7b10, then394dc210 clears the prior CURRENT recipe/
category maps and reads the actual object's static doodad template via3963b250.
It enumerates template functions+24..28: kind0c contributes its CraftStart recipe list;
kind3a resolves a CraftPack via397f2390 and contributes that pack's recipes. Valid
resolved recipes are inserted uniquely and grouped by recipe ac_id and that category's
group. Finally394dc360 emits CRAFTING_START event78(objectID,uniqueRecipeCount).

This opening is local, source-proven interaction dispatch:393ba9c0 maps a special
interaction skill/action to callback393b7ba0, which invokes394dc360 and returns true.
Callers393b40e4/393b76ee skip their normal skill-cast fallback when handled. No new
network ACK or command is implied. The map's craft action key global39ac7868 remains
unresolved, so arbitrary recipe selection or guessed skill IDs must not impersonate
a natural workbench interaction.

GetActabilityCategoryTypesFromCurrentDoodad39461480 takes a group ID and consults the
CURRENT map3a5ad874 through394d7f30; it sorts its category IDs by visible_order then ID.
GetCraftTypesFromCurrentDoodad39461540 takes group/category IDs and consults that same
CURRENT map through394d7da0, sorting available recipe IDs by visible_order then ID.
Missing group/category, invalid sentinel arguments, or empty output returns nil. These
getters therefore reflect the selected real workbench's contributed recipes, not every
catalog recipe or a filter guessed solely from crafts.req_doodad_id. Lifetime closure
and upstream action selection still require integration with the actual interaction.

Existing aa_ui::item_link::format directly matches394e4ef0, including its alphabet,
192-byte descriptor, zeroed instance ID and count1 normalization. CRAFT_FAILED394d2370
constructs synthetic flags1/world_idff/zero detail and timing before formatting. Its
grade3a5ad89c and maker3a5ad898 are zero-filled image values with direct read-only xrefs;
indirect initialization was not excluded, so those defaults remain a stated boundary.

Material/lifetime precision followup:39462600 replaces the representative descriptor
while accumulated eligible stack count is zero, then retains the first positive eligible
stack descriptor. If final count is zero, it always replaces any prior physical descriptor
with synthesized required-grade item_info. Missing any material descriptor logs and
returns nil for the entire getter at39462a2c. Shared interaction close393b7b50(kind3)
emits INTERACTION_END eventd2 and clears active kind/object, but393b7aa0 has no craft-map
clear branch; CURRENT maps are rebuilt on next394dc210. Closing must not fabricate a
batch completion or assume native getters necessarily become empty.

Modifier refinement:3932a190 queries enclosing spatial object39328630 with the player
position390b3850 and radius0, checks permission393257e0, then reads matching group bonus
from object vector+1a8..1ac (20-byte entries, value+4). The prior equipped-state wording
was too strong; no equipment-only assumption is justified. Exact spatial object type and
its authoritative bonus population remain untraced. Missing player/object/permission/
group/bonus yields native0, independently of received proficiency-point availability.

### Native crafting catalog getter shapes (2026-10-18)

EnumActabilityGroupTypes39460cf0 walks all static groups in native catalog order and
skips only global39ac99b8 (runtime ID unresolved); it does not test skill_page_visible
or visible. No output returns nil. GetCategoryType39463a30 returns category IDs with
visible_ui true, sorting visible_order then unsigned ID, and permits an empty table.
GetActabilityCategoryTypesGroupType394610d0->394d8200 reads the ALL-catalog category
map3a5ad8a0, filters visible_ui, sorts the same way and returns nil on unknown/empty.
Its special all-groups path compares global3a5ad8b0. In contrast the CurrentDoodad
getters read the selected bench map3a5ad874. GetActabilityCategoryCraftTypesByTypes
394612c0->394d84b0 reads the ALL catalog group/category map and sorts recipe IDs by
visible_order then unsigned ID; it imposes no labor/proficiency/workbench/range gate.

GetCraftProductInfo39462a70 returns array rows with exactly itemType,item_name,amount,
success_rate,useGrade,productGrade. success_rate is the raw product rate converted to
f32, without division by100. useGrade is product byte+15; productGrade is grade byte+8.
It supplies no item_info table. Known recipe with zero products returns an empty table;
unknown recipe or any product with missing item descriptor logs/returns nil. Material
rows remain amount,count,mainGrade,item_info, using exact required grade or the native
static default for required grade-1 when no eligible physical material is available.

GetExecutableCraftCount39461ee0 calls394d3cc0. Unknown recipe or zero material amount
returns-1 internally and nil to Lua. The count starts INT_MAX. Labor is considered ONLY
when the material vector is empty, the recipe skill resolves and its base labor is
positive: native discounted neededLP divides current labor, if neededLP is nonzero.
Otherwise material recipes skip this labor branch and take the minimum eligible item
count divided by each required amount. A special material item ID uses64-bit current
copper division, invalid item gives0, normal inventory count3961fbc0 receives the required
grade and playerflags2000 policy. This getter itself does not test proficiency, range,
workbench compatibility, inventory space or material-row widget limits. Its no-limit
path may retain INT_MAX; an integration must not invent a smaller native result.

GetUpperCraftsInfo39462df0(craftID,categoryFilter) selects the LAST source product item
ID, then walks global craft_materials catalog397de0d0 in native order. For each matching
material item ID it resolves the consuming recipe, requires show_upper_crafts and at
least one product, then resolves firstproduct.craft_id; positive categoryFilter must
match that recipe.ac_id. Output rows have craft_Type,ac_type,itemType,useGrade,itemGrade,
mainGrade and optional item_impl. Product fields come from the consuming recipe's FIRST
product; mainGrade comes from the matching material. There is no deduplication/sort in
this getter; it appends per matching material. Unknown source returns nil, known source
with no matches returns an empty table. These shapes were read from original wrappers;
no new protocol state or fabricated crafting results were introduced.

Executable-count secure policy is source-specific:394d3e39 passes playerflags2000
as3961fbc0's last bool argument, which skips item flag04 only when true. MaterialInfo
instead checks feature byte5 bit20 plus item flag04. Their counts must not be assumed
identical when these independent received/config flags differ.

### Base information, search and raw grade boundaries (2026-10-18)

GetCraftBaseInfo39461860 exact keys: craft_type,ac_type,title,desc,tool_type,skill_type,
cast_delay,recommend_level. title (not name) is recipe+28. doodad_name is included only
when req_doodad_id resolves3963b290 and comes from that record+3c. A resolved skill adds
consume_lp,needed_lp,laborpower_satisfied; missing skill omits all labor/proficiency
keys. A resolved skill group adds required_actability_type/name/point and compares
effective received+attribute+spatial points against crafts.actability_limit. Missing
group omits those required fields but sets actability_satisfied=true. Labor satisfaction
is current labor>=native rounded neededLP, without an additional workbench/range gate.

GetCraftTypeByItemType39462010 returns the FIRST global craft_products397de110 record
in native catalog order whose item_id matches and show_lower_crafts(+14) is true.
It returns product.craft_id without checking grade/rate/current bench/proficiency or
resolving that recipe; no match returns nil.

All/CurrentDoodadCraftTypesInGroupByName39461640/39461700 call394d3ea0 with different
source selections. They require XlStringLen(name)1..30, convert multibyte to wide through
XlMultiToWide, filter group/category sentinel rules, then match recipe.title with
XlFindStringI (case-insensitive substring, no trimming/tokenization). All scans native
recipe catalog order; Current scans selected-bench recipe-ID tree order, without a
visible_order sort. Material-name variants39461d60/39461e20 call394d24d0 with the same
length/conversion/matching rules on material item.name. They append per matching
material and retain duplicate consuming recipes. Current material search resolves
active kind3 doodad and compares recipe.req_doodad_id to its actual templateID, rather
than testing current-map membership. Invalid search/empty output returns nil.

XlStringLen implementation is read directly from read-only client12/bin/xlcommon.dll,
export3301b000. It counts UTF-8 sequences, not bytes: ASCII advances1, lead c2..df2,
e0..ef3,f0..f44 and validates continuation bytes, incrementing once per sequence.
Malformed input stops counting; the routine does not fully enforce Unicode scalar
validity beyond these byte checks. For valid UTF-8, Unicode scalar count matches this
1..30 search gate. The imported XlFindStringI locale/casefold implementation was not
replaced with an assumed Rust Unicode casefold policy.

Material required_grade compares directly with received raw grade byte; products export
raw grade byte without grade_order conversion. If no eligible material and require_grade
is-1,3946280f reads zero-filled static byte3a5aa2d0 and passes raw grade0 to39421e20.
It does not substitute item.fixed_grade. Synthetic item_info exports itemGrade directly
from that argument at39421fb0; fixedGrade is separate metadata itemdesc+3c, exported only
if nonnegative. Display tooltip grade_order conversion must not change raw comparisons.

GetExpandExpertInfo3943ba20 uses next-row lookup397a9540(received expansionCount),
which queries key count+1. Missing returns nil/error1fd. Fields: requireLP=row life_point;
requireItemType and requireItemCount only when itemID differs from invalid3a5a9e2c;
remainExpandCnt excludes the immediate next row. At3943bb20 it starts i1 and queries
lookup(count+i), itself adding1, until missing, returning i-1. For contiguous keys1..10,
received count0 therefore returns9, count9 returns0 while key10 is still available.
No LP/item affordability is checked by this getter or CanExpandExpert.
Native3301b000 Unicorn oracle (no stubs) confirms ASCII30bytes->30, Korean30chars/
90bytes->30, Korean31/93bytes->31, emoji30/120bytes->30, and a+ff+z->1. Ignored helper:
data/extract/aa-net/search_length_oracle.py.

### Housing proficiency modifier source and protected constant boundary (2026-10-18)

Spatial3932a190 modifier class is conclusively Housing: OnHouseState391d784f invokes
manager3932fa00, constructs the object queried by39328630 and calls3932d0f0, whose native
literal is `X2::GameClient::Housing::InitFromNetHouse`. It derives bounds from static
house metadata and received house state. Access393257e0 depends on received owner/
permission/current-player relationship, then the matched proficiency bonus is read from
the house's vector+1a8. This is neither a direct proficiency packet nor an equipment-only
bonus. Complete house receive/decoration lifecycle is not yet an aa-net typed resource.

3932fd00 populates/aggregates that vector using housing_decorations: loader39677f..
queries SQL399fec78 with actability_group_id and actability_up stored at record0/+4.
Matching existing group adds actability_up (3932fe1f); new group gets a20-byte entry.
Decoration add/remove paths3903896d/39038ad1 call39330270/39330310 and emit native
ACTABILITY_MODIFIER_UPDATE234 for the current house. Faithful hydration requires actual
received house/doodad decoration association plus the static decoration catalog; absent
full state must not be represented as a proven zero modifier.

The static constant audit found only reads of ten language-ID globals39acb624..648,
excluded Enum group39ac99b8 and special count-exclusion39acb620. Their image contents are
protected/nonplaintext; no direct static initializer assignment was found. The later
read-only runtime check below closes the language and count-exclusion IDs; the separate
Enum exclusion39ac99b8 was subsequently read as20 in the same verified process. No value is inferred from catalog names.

### Stablehand quote getter source audit (2026-10-18)

GetTotalRepairsForPetItems39447a40 invokes397d3130 with argument0b, which is item
implementation11, NOT a container mask. That helper enumerates physical BAG(kind2) and
BANK(kind3) slots and selects items whose static implementation is11. The quote skips
invalid templates, pending packed slot keys in inventory+c348, absent/invalid pet-item
mapping396a6f00 and nondead items. Native detail deadbyte+1d must be nonzero; level is
byte+1e. Each eligible item contributes once, independently of stack count. Getter sums
signed i32 costs and returns the total converted to f32 Lua number. It does not require
a stablehand, inspect affordability/currency policy or change any received dead flag.

Per-item arithmetic39447b08..b96: level converts to f32, then double; imported MSVCR100
__libm_sse2_pow uses double exponent399da2e8=2.299999952316284. Result converts to f32;
separate f32 operations add level*0.5 and0.5; the native SSE floor sequence converts to
i32. This differs from the lane's entirely-double `revive_cost` using exponent2.3.
A formula comparison across byte levels finds one-copper differences at144,164,219,226,
244; this comparison is not a live/native pow oracle and assumes no arbitrary level cap.

RepairPetItems39440d80 independently reads current interaction, requires NPC kind1,
resolved NPC data and stabler flag+e4, and sends existing CS0b5 actual NPC objectID.
It adds no optimistic mutation/acceptance. The existing authoritative reason58 item
updates remain the source of revival. A faithful UI quote still needs current per-slot
pending eligibility plus pet static mapping; no new quote API was fabricated from only
dead flags or from the lane's different floating-point implementation.

### Explicit physical-slot pending state (2026-10-18)

Inventory now retains explicit physical pending slots with mark_item_slot_pending,
item_slot_pending, clear_item_slot_pending and pending_item_slot_keys. Native397d2200
uses packed keys[0,container,0,slot] and excludes trade kind4. This API accepts physical
kinds1..3; setting keys remain separate. Marking changes no item contents, dead flags,
quote, money or server success. Outgoing automatic lock producers remain unwired:
the integration must register only its source-proven accepted native operations and
release explicit local rejection/cancellation. An empty set must not be advertised as
complete native pending coverage while additional command producers are unresolved.

Genuine SC0f5 releases its exact physical key; successfully applied slot-bearing item
tasks release reported keys after the complete batch, preserving per-action ordering.
Unknown/unapplied item updates do not unlock slots. Character selection, LeftWorld and
disconnection clear physical locks with the existing Inventory lifecycle. Tests cover
unrelated/repeated unlock, an unmatched item identity, received mate revival retaining
experience/level, AA-setting independence and lifecycle. Native Mate detail layout is
+18 tag,+19 signed exp,+1d raw dead,+1e raw level; dead is tested nonzero, not only1.

Category-map clarification: initializer394dbe90 calls394dbd50 to make static group
entries,394daa10 inserts ALL static categories with empty recipe vectors into valid
groups, then394d8120 appends recipes to existing category vectors. Consequently
GetActabilityCategoryTypesGroupType includes visible_ui categories with zero recipes;
it must not filter them merely because the recipe list is empty.

Pet quote mapping is item_summon_mates, not npcs.pet_item_id:396a6f00 reads DB map+3f34;
loader396bec80 uses SQL39a01eec `SELECT item_id, npc_id FROM item_summon_mates`, storing
item keys and NPC values. Quote requires a present mapping and noninvalid NPC value.
This closes the static eligibility lookup needed alongside impl11, actual bag/bank mate
details and explicit pending slots; capture-drop NPC pet_item_id is a separate field.

### Original active-pet UI contract audit (2026-10-18)

The original `unitframe/pet.lua` and `hud/pet_action_bar/pet.lua` address one current
`playerpet`, rather than an owned-pet roster. They create/show the frame on `SPAWN_PET`
and hydrate it at load only when `X2Mate:IsPlayerPetExists()` is true. Native wrapper
39069fa0 checks manager3933f5e0 for a current pet. Inventory summon items are a separate
ownership source; do not turn the single `Mates::own` correlation into a fabricated list.
The existing received spawn metadata plus uniquely correlated visible mate unit provide
the current-pet identity. `UNIT_NAME_CHANGED(unitId)` refreshes the frame only when the
ID matches `X2Unit:GetUnitId("playerpet")`.

Native `PetState`3906a070 returns the strings `idle`, `passive`, `protective`, or
`aggressive` from current mate+34 (indices0..3). It returns nil without a player/current
mate, and while the player is attached to that same mate. Returning the wire state's
numeric byte would violate the Lua contract. `CanDismiss`3906a560 returns false without
a current mate; otherwise39357830 supplies a reason whose zero value permits dismissal.
That helper rejects active casting slots with reason7. Its additional runtime virtual+8c
predicate remains unidentified, so this audit does not replace it with a guessed health
or death check. Original command buttons poll this predicate for their red overlay.

`ShowChangeMateName` uses a target only to populate dialog text. Its callback invokes
`X2Mate:SetMateName(newName)` without an ID. Wrapper39069860 resolves the current mate
first, and39069970 sends that object's timeline at+14. Keep the existing explicit-TL
network rename API, but choose that TL from the current owned mate at the original Lua
boundary; received `UnitNameChanged` remains the name authority.

`GetNumMountSkills`3906a9b0 returns current mate+ac, or0 when absent. `GetMountSkill`
3906aa90 accepts a one-based displayed index, converts it to zero-based, and calls
3933c340 for a context-dependent lookup; a missing mate/failed lookup returns nil.
3933ad90 filters the ten stored mount-skill records through static mount/combat-skill
records and attachment context. Therefore neither the raw ten-element length nor a
combat-skill ID alone is a faithful displayed slot mapping. Helper3933b670 starts with
the generic skill tooltip, adds `learned` as a BOOL from runtime pet level versus required
skill level, `skillUsable` as numeric0/1, and the exact native typo `needdiver` as a BOOL.
Original `SetPetBarSkillTooltip` sets `showReq` and `displayPetSkillLearnLevel` only when
`learned == false`. Action widgets establish `ISLOT_PET_ACTION` using displayed slots.

`GetAutoStartMountSkill`3906ab50 also maps the displayed index through3933ae60 before
reading the raw record's byte+60 via3933a950. With a current pet its Lua result is BOOL.
`PET_AUTO_SKILL_CHANGED` refreshes each non-driver button from that getter;
`FAILED_TO_SET_PET_AUTO_SKILL` chooses mountable/nonmountable localized error text.
The original widgets route actions through `ISLOT_PET_ACTION`; this audit has not proved
a separate network auto-use command or its success authority. None is invented here.

Stablehand Lua invokes the global `X2:GetTotalRepairsForPetItems()` and
`X2:RepairPetItems()`, rather than X2Mate/X2Bag methods. Zero quote shows the need-pet
notice; a positive quote opens the money confirmation and its OK invokes the request.
The source-proven physical-item quote/pending/mapping contract above applies. Existing
`MateCommand::ReviveItems` sends the contextual NPC request, and genuine reason58 item
updates supply resurrection state. A local request outcome does not mean revival.

This read-only audit adds no protocol/state API: current mate identity, explicit control
requests, attachment events, authoritative rename, physical inventory details and pending
slots already cover the proven boundaries. Natural summon/ride/service QA, exact action-slot
automatic-use control, and the protected max-mate-level producer remain unresolved.

### Housing and public farms (2026-10-20)

`housing::{HouseSnapshot, HousingEvent, Houses, HouseCommand}` decodes every `SC*House*`
packet and writes the 1.2 `CS*House*` requests. The AAEmu server's
`models/house.rs::House::write` is the wire oracle: one fixed `u32` model (not a PISC
pair), `i16`-length strings and a `u32` sale price. The earlier "lane PISC" assumption was
wrong for both `<home>/AAEmu` and the `bevy-srv` server; the reader now follows
the writer exactly and the native client's 128-byte name buffers are not imposed as wire
limits.

| packet | body | event / state |
|---|---|---|
| `SCHouseState` 0x0bc | house body (below) | `HousingEvent::State`; upserts `Houses.by_tl_id` |
| `SCMyHouse` 0x0c1 | house body | `HousingEvent::MyHouse`; same upsert |
| `SCLoginCharInfoHouse` 0x057 | `u32 owner id` + house body | `HousingEvent::LoginCharInfoHouse`; same upsert |
| `SCHouseBuildProgress` 0x0bd | `u16 tl, u32 model, i32 all steps, i32 current steps` | `BuildProgress`; known TL only |
| `SCHouseBuildPayChanged` 0x0bf | `u16 tl, i32 money` | `BuildPay`; known TL only |
| `SCHousePermissionChanged` 0x0be | `u16 tl, u8 permission` | `PermissionChanged`; raw enum, known TL only |
| `SCHouseDemolished` 0x0c0, `SCMyHouseRemoved` 0x0c2 | `u16 tl` | `Demolished` / `MyHouseRemoved`; marks `House::demolished`, never invents `UnitsRemoved` |
| `SCHouseFarm` 0x0c3 | `string name, i32 total, i32 harvestable` | `Farm`; stored as `Houses.last_farm` (no timeline on the wire) |
| `SCHouseTaxInfo` 0x0c4 | `u16 tl, i32 dominion rate, i32 money, i32 money2, i64 due, bool paid, i32 weeks, bool heavy` | `TaxInfo`; stored on the known TL |
| `SCConstructHouseTax` 0x0c5 | `u32 design, i32 heavy count, i32 normal count, bool heavy, i32 base, i32 deposit, i32 total` | `ConstructHouseTax`; `Houses.construct_quote` |
| `SCHousingRecoverToggle` 0x0c6 | `u16 tl, bool allow` | `RecoverToggle`; updates the snapshot |
| `SCHouseSetForSale` 0x0c8 | `u16 tl, u32 money, u32 buyer id, string buyer, string house` | `SetForSale`; stored on the TL |
| `SCHouseResetForSale` 0x0c9 | `u16 tl, string house` | `ResetForSale`; clears the listing |
| `SCHouseSold` 0x0ca | `u16 tl, u32 previous owner, u32 new owner, u32 new account, string owner, string house` | `Sold`; updates owner/account/name and clears the listing |
| `SCHouseOwnerNameChanged` 0x0cb | `u16 tl, string new name` | `OwnerNameChanged`; updates `snapshot.owner_name` |

The house body is `u16 timeline, u32 database id, BC object id, u32 housing template,
u32 model, u32 co-owner, u32 owner id, string owner name, u32 account id, u8 permission,
i32 all actions, i32 current actions, i32 tax, i64 X, i64 Y, f32 Z, string house name,
bool allow recover, u32 sale price, u32 buyer id, string buyer name`. The server writes
zero `all/current` actions for a finished house (`current_step == -1`) and
`base_action + num_action` otherwise. Raw permission values, signed sale bits and raw
fixed-point XY are retained.

`Houses` applies deltas only to a known timeline. A full body replaces the TL record,
resets `demolished`, and drops any other TL with the same runtime object id. Character
selection, world leave and disconnect clear the table and the quote. `UnitsRemoved`
removes snapshots by object id. `SCMyHouse`/`SCLoginCharInfoHouse` update the same table;
no unit, placement or UI event is fabricated from a snapshot.

`HouseCommand` writes the server's readers byte for byte:

| command | opcode | body |
|---|---|---|
| `ConstructHouseTax { design_id, x, y, z }` | 0x056 | `u32 design, i64 X, i64 Y, f32 Z` |
| `CreateHouse { design_id, x, y, z, z_rot, item_id, money_amount, ht, auto_use_aa_point }` | 0x057 | `u32 design, i64 X, i64 Y, f32 Z, f32 rot, u64 item, i32 money, i32 ht, bool auto` |
| `DecorateHouse { house_tl, design_id, x, y, z, quat, parent_obj_id, item_id }` | 0x058 | `u16 tl, u32 design, f32 x/y/z, f32 quat[4], BC parent, u64 item` |
| `ChangeName { tl, name }` | 0x059 | `u16 tl, string name` |
| `ChangePermission { tl, permission }` | 0x05a | `u16 tl, u8 permission` |
| `RequestTax { tl }` | 0x05c | `u16 tl` |
| `AllowRecover { tl }` | 0x05d | `u16 tl` |
| `Sell { tl, money_amount, sell_to }` | 0x05e | `u16 tl, u32 money, string buyer` |
| `CancelSell { tl }` | 0x05f | `u16 tl` |
| `Buy { tl, money_amount }` | 0x060 | `u16 tl, u32 money` |

There is no 1.2 demolish request: the server demolishes through a doodad function, and
`CSChangeHousePay` is 0xfff and unregistered. Build steps are skill casts handled by the
existing `CSStartSkill`, not a separate house command.

`farm::{FarmEvent, Farms, CreateDoodad}` covers public farms and planting. It follows
`systems/npcs/public_farm.rs` and `systems/npcs/doodad/packets.rs`:

| packet | body | event / state |
|---|---|---|
| `SCResponseCommonFarmList` 0x1b1 | `i32 max count, i32 count`, then per entry `u32 farm type, u32 template, u32 time left, u32 func group, position (9 bytes), i64 plant time` | `Farm::List`; `Farms.list` |
| `SCShowCommonFarm` 0x1b0 | `i32 farm type, i32 count, i64 X, i64 Y, f32 Z` | `Farm::Show`; `Farms.show` |
| `SCIsUnitInFarm` 0x1c9 | `i32 type` (C# 10.0.2.13 serializer; the Rust server has no writer) | `Farm::IsUnitInFarm`; `Farms.unit_farm_type` |
| `Command::CreateDoodad` → `CSCreateDoodad` 0x0e6 | `u32 doodad id, i64 X, i64 Y, f32 Z, f32 rot, f32 scale, u64 item id` | sent in world only |
| `Command::RequestCommonFarmList` → `CSRequestCommonFarmList` 0x10b | empty | sent in world only |

Negative or oversized farm-list counts are rejected before allocating. Farm state is
received-only and clears on character selection, world leave and disconnect. There is no
HUD consumer yet.

Validation: `cargo test -p aa-net --lib` passes all 634 tests (housing and farm goldens
cover every truncation and DD04 tail alignment); `cargo test -p aa-client` loads the real
`AA_DB` housing/farm tables. No live placement, construction, sale or tax transaction was
attempted.

The decoration mutation source also distinguishes attachment from a house snapshot:
39038941 checks the parent unit's kind+410==3, resolves its timeline+418 through3932b510,
and passes doodad+7c (runtime object ID) and+2a8 (item template) to39330270. Removal
39038a97 resolves the doodad's attached unit+88 and passes the same pair to39330310.
This supports later use of received attachment and item identity; it does not prove that
all world doodads merely owned by the same player are decorations of that house.

### Explicit housing attachment identities (2026-10-18)

`housing_attachments(&Doodads, &Units)` yields raw `HousingAttachment` identities only
when a doodad's received `parent_obj_id` resolves to an existing received
`UnitKind::Housing`. The four fields are house runtime object ID, its received timeline,
doodad runtime object ID and raw item template ID. The join is computed from the two
existing tables, so either packet order works and ordinary unit/doodad removal immediately
invalidates membership without another cache. There is no ordering promise, ownership or
proximity fallback, static item classification, permission decision or modifier total.
A zero/unknown item template remains a received identity for the caller to resolve.

This adds no wire fields: Doodad::read already retained parent and item IDs, and Unit::read
already retained housing kind3 plus TL/template/build-step. The lane
`models/doodad.rs::write` writes owner-object and parent-object BCs separately;
`housing/manager.rs::attach_loaded_doodad` explicitly sets parent_obj_id=house.obj_id.
That is consistent with the native parent-kind3/TL check above. Unit and doodad view tables
now also clear on `CharacterSelected`, matching the per-character housing cache and
preventing old attachment joins when a new character state arrives without a prior leave.

The regression uses an independent full doodad wire fixture with different owner-object,
parent-object, character-owner and owner-database IDs. It checks every truncation, parent
kind, both packet orders, same-owner unattached rejection, raw zero item IDs, removals and
character/world/disconnect resets. No placement, furnishing or server command is added.

Lane `world/visibility.rs` announces a house as SCUnitState, then SCHouseState, then
SCDoodadsCreated batches from its attached_doodads IDs. The joined view intentionally
needs no guessed completion marker for those batches and yields only attachments already
received at each point in that stream.

Validation after the attachment/lifecycle follow-up: full aa-net library suite283 passed;
`git diff --check` passes. No live placement or furnishing transaction was performed.

### Housing unit-tail alignment and raw door/window posture (2026-10-18)

The complete lane `SCUnitState` housing tail is now consumed rather than returning after
its first attachment byte. Previously kind3 hit the early return for unsupported kinds,
leaving the second attach point, posture, weapon/counts, rotation, faction and buff lists
at the next compressed-packet boundary. The active kind3 parser now follows
`world/unit_state.rs::{house_unit_state,write_net_unit}`: System attach255, one-byte
unbound marker (or the remaining two bytes of a BC bond), posture, active weapon,
zero learned skill/buff counts, three signed rotation bytes, race/gender, appellation/
faction/honor PISC groups, flags and three complete buff lists. Housing truncation,
unsupported first attachment, unknown posture and nonzero lane-only learned counts fail
instead of exposing an apparently complete partial house. Other unit-kind behavior is
not broadened by this change.

Native shared posture serializer397bda20 reads type and isLooted; type1 selects
397b1d90, which loops exactly two `door` bytes then six `window` bytes using boolean
virtual+78. It is called by397c6750 and397c96b6. The lane writes the same eight fields in
`write_model_posture` (currently all true). `unit_msg::HousePosture` retains the two and
six raw byte arrays plus the loot bool; `Unit.house_posture` is None until a received full
unit tail or posture update supplies it. Raw byte values are not normalized away.

`Posture.house` carries this payload for SC103. The existing `Event::UnitPosture`
remains unchanged; the session additionally forwards
`HousingEvent::UnitPosture { obj_id, posture }` after it when the payload exists. The
Units reducer changes an existing unit only, and a later non-house posture clears the
house payload. There is no fabricated unit, door animation, interaction or outgoing ACK.
Two explicit aa-client test fixtures require their new optional Unit field initialized to
None; root handles those owned integration paths.

The independent lane housing-unit fixture now proves complete SCUnitState followed by
SCHouseState and a third DD04 packet remains aligned. It checks every unit truncation,
raw house posture byte preservation, delta ordering and unknown-unit rejection. Rotation
64 decodes using the existing source-backed TAU/127 quantization, rather than incorrectly
assuming its rounded byte is exactly pi. The earlier focused test expectation was
corrected accordingly; no rotation decoder change was needed.

Additional workbench source clarification:394dc210 enumerates all static template
function groups at+24..28, not only the received phase func_group. Its shared active
interaction monitor393b7e40 resolves the active doodad and calls39093800 with player,
doodad, global39aefb98+2a0 float and false; missing object or a false result closes through
393b7aa0 and clears active kind/object. The float's producer and exact reach predicate
remain pending, so this records no guessed numerical interaction range.

Validation: housing-tail/posture check and full aa-net library suite pass (286 tests).
No live housing placement, demolition, furnishing or interaction transaction was issued.

### Native world-interaction action lists and cancellation (2026-10-18)

`InteractEvent::WorldSkillList(WorldInteractionSkillList)` now receives SC0x07e, distinct
from the existing NPC SC0x07b. Native factory391aa870 installs vtable399b36e4/opcode7e;
serializer397c2d70 calls inner serializer397bef50. Exact wire order is:
two outer BC IDs, inner BC context object, BC doodad object, u32 interaction type,
BC pick object, i32 announced count, i32 extra info, up to10 u32 skill IDs, u8 mouse button,
i32 modifier keys. The two outer IDs retain neutral names because handler391d9080 ignores
them and passes only the inner context, mouse and modifiers to393b7440. There is no
NPC-style interactable bool in this packet. Unknown type/ID/skill/mouse values and duplicate
skill IDs remain raw. Native397bf067 caps transferred skill IDs at10; negative counts
transfer none. The typed payload preserves raw_count alongside the bounded vector rather
than allocating from or silently rewriting an untrusted count.

`InteractEvent::WorldCanceled` receives SC0x07f. Factory/vtable399b36f0 uses serializer
397c6540: interactedUnitId is BC via397b27d0, but interactedDoodadId is FULL u32 via
virtual+3c. Handler391d9100 selects unit or doodad cancellation using the native invalid-ID
sentinels. The transport forwards exact IDs without guessing those sentinels, selecting an
arbitrary current target, clearing an unrelated context, opening crafting or casting a skill.

Native393b7440 uses the received inner doodad ID at+4, validates reach and player state,
then dispatches the selected received skill through393ba9c0 before its ordinary cast
fallback. Original interaction.lua populates the choice UI through
GetInteractionSkillCount/GetInteractionSkillId. This is a genuine action-ID source; merely
finding CraftStart/Pack static template functions is not equivalent to receiving a selected
action. The bevy lane declares both opcodes but currently has no matching packet emitter/
handler implementation (`CLIENT-DB-GAP-AUDIT.md` also records the gap). This client milestone
adds receive infrastructure only, with no fabricated lane response or new outgoing command.

The compact database's CraftStart rows have func_skill_id NULL (18) or0 (7), and all304
CraftPack rows have NULL. A skill_effects→effects(actual_type=InteractionEffect)→
interaction_effects(wi_id21) join finds only skill22678: effect31656→actual3916→wi21, with
no doodad override. Native effect loading identifies InteractionEffect as kind0d
(397309a6/3989592e), independently of CraftEffect kind0e. That proves a DB classifier for
that one skill's declared effect; it does not prove equivalence to every native local
craft callback or resolve protected action key39ac7868. No global-ID guess is added.

Interaction reach has an unprotected producer:390fa0a1 registers the float at config+2a0
as `max_interaction_doodad_distance`, default f32 2.0999999046325684 from399a7660, description
`offset distance (meter)`. Initial393b3fc5, queued393b7561, craft validator394d6e8e and active
monitor393b7e40 all use it. Caller393b60ed runs the monitor when its timer reaches1000ms.
Predicate39093800 requires offset strictly greater than computed geometry distance.
The distance is the minimum geometry-pair result398b1ab0 over doodad virtual+18's shape
vector and the player's physics shape from390b3ae0; this is not a center-distance<=2.1
contract. Exact shape dispatch and local action eligibility remain separate app/physics
integration work, not a guessed net proximity gate.

Batch callback precision:394d53e0 checks resolvable CraftEffect0e and local unit caster,
and rejects the pair only when BOTH skill/TL equal invalid sentinels. It does not require
that each callback pair equals manager's saved+14/+18. Successful Started replaces that
pair, clears Fired latch+12, and emits CRAFT_STARTED(remaining) only while first-start+13
was true (then clears it). Successful Fired sets+12. Ended without Fired emits
CRAFT_ENDED() with NO arguments immediately, keeps count unchanged temporarily, clears
ready+10/waiting+11, then next Update394d32b0 calls reset394d0bf0 and makes count0.
Stopped similarly emits noarg ENDED immediately and resets on next Update. Failed Started
calls reset first, then emits noarg ENDED, so count is already0 during the callback.
Ended with Fired decrements and emits positive remaining or noargs when exhausted;
Fired latch is cleared by next Started, not by Ended, so no extra duplicate-Ended
idempotence guard is claimed. Update only toggles waiting for a positive ready batch;
it never resends ExecuteCraft.

SC0x07e consumer precision:393b75db reads manager+10, the first supplied skill, for its
immediate special-callback/cast path. This does not prove that the packet itself populates
the original UI choice manager; the net layer exposes the whole bounded payload without
choosing an action. Native player-state eligibility393b3d50 remains separate from template
recipe population, including state checks and mounted-context handling. Its broader
housing permission predicate393b3510 has a protected prefix and unresolved special action
keys; this audit does not replace those with zero permission or a guessed generic gate.

Validation: `cargo check -p aa-net --features bevy`, full289 library tests and diff check
pass. Three new tests cover native field goldens, raw count clipping at0/10 with exact
following DD04 boundaries, every truncation, FULL32-bit canceled doodad IDs and repeated
ordered receive forwarding without casting or inventing a workbench.

### Complete lane slave/transfer/shipyard unit tails (2026-10-18)

`Unit::read` now consumes complete active-lane SCUnitState packets for Slave2,
Transfer4 and Shipyard6. Previously Slave returned immediately after its first attachment
byte, and Transfer/Shipyard skipped their equipment/model-parameter/tail bytes entirely;
a following DD04 opcode could therefore be read from the unit body. Transfer and Shipyard
now retain exactly seven bare body-part template IDs in slots19..25, then read model
parameters. Slave retains its full28-slot equipment format. All three consume the exact
lane posture/weapon/zero learned counts/rotation/race/PISC/flags/buff suffix, rejecting
truncation, unknown postures and impossible nonzero lane learned counts instead of
reporting partial success. Character/NPC/mate tail compatibility is not broadened here.

`Unit.state_attachments: Option<UnitStateAttachments>` retains received snapshot identity
separately from `attached_to` and actual UnitAttached/Detached notifications. It is Some
for the strict lane kinds2/3/4/6, including System/unbound snapshots. Fields are raw
first_point, optional first_obj_id and optional bonding_obj_id. The exact active-lane
rules from `world/unit_state.rs::write_net_unit` are:

| Kind | First attachment | Second attachment |
| --- | --- | --- |
| Slave2 | signed i8 point; BC owner object only if point>-1 | BC bond when present, else byteff |
| Housing3 / Shipyard6 | System byteff | BC bond when present, else byteff |
| Transfer4 | u8 point; BC bonding object unless point==ff | BC bond when present, else byteff |

Lane model comments identify Slave owner_obj_id as the unit to which the slave is attached;
the snapshot preserves that role but does not issue a synthetic rider event or guess a
passenger list. Existing real attachments remain authoritative for the live rider-seat
view. Root adds state_attachments:None to its two explicit Unit fixture literals.

A newly verified native boundary is retained explicitly: NetUnit calls397c03b0 at397c9698
for its first attachment and3983bf10 at397c96a7 for its second, then397bda20 posture.
Native first point!=ff always reads a BC via391b8780; it does not use the lane's signed
Slave point>-1 condition. Native second non-ff form is point byte +BC via397b25d0 +kind
byte +i32 space +i32 spot (13 bytes), whereas the lane writes plain optionalBC for these
noncharacter bonds. These are concrete transport differences; the active lane parser does
not guess a native profile or call this raw lane DTO a complete native attachment object.

Three new regressions independently construct exact source field fixtures across every
first-attachment branch and absent/present second bonds. They preserve seven body parts,
distinct owner/bond IDs, signed edge bytes and complete HP/faction/rotation state, reject
every truncation and malformed tails, and decode Slave→Transfer→Shipyard→Snow in one
DD04 batch without desynchronization or fabricated UnitAttached state.

Validation: strict object-tail `cargo check -p aa-net --features bevy` and full292
library tests pass; diff check is clean. No live summon, vehicle, attachment or server
transaction was issued during this protocol-only work.

### Vehicle equipment reports and pet expiry notices (2026-10-18)

`SlaveEvent::EquipmentChanged(SlaveEquipmentChange)` receives SC0x068 with exact wire
ordering: stored character u32, slave TL u16, database slave u32, bts boolean, two full
optional Items, first container/slot bytes, second container/slot bytes, success boolean.
Names deliberately preserve first/second order rather than claiming accepted destinations.
Native factory391e3590/vtable399b73b0→397aec10 invokes397d1410, whose header and common
transaction397d12a0 match the lane `economy/item_effect_packets.rs` writer. This packet has
no entry count or expireTime. The lane defines it but has no sender. The transport exposes
the raw result without fabricating an owned slave, inventory swap or equipment settlement.

`MateEvent::EquipmentExpired { mate_tl_id, container, slot }` receives SC0x127 as exactly
u16 TL plus two bytes. Native factory391b04c0/vtable399b390c→397b5530 proves that layout.
Handler391db280→394e5a90 requires containerfc, the current owned mate's matching TL and
an existing visible unit. It selects the current equipped descriptor (slot<=27) and calls
394e5410, which formats the descriptor through394e4ef0 and emits native EXPIRED_ITEM
(event0x89) with one item-link string. It does not delete gear, mutate item flags or
produce an inventory task. Original chat/chat_msg_event.lua receives EXPIRED_ITEM text.
Consequently this typed expiry report is notification-only: neither Mates nor Units nor
Inventory is changed from this packet alone, and unknown container/slot bytes remain raw.
A future app bridge must use the matched actual descriptor, not a synthetic expired item.

Three regressions cover independent headers, full opaque item details/timers, raw locations,
false/true success flags, all truncations, following DD04 alignment, repeated receive order
and absence of phantom owned units/equipment removal or fabricated completion events.

Fresh original pet auto-use source also closes a previous wire uncertainty: setter3933c750
is called by the ISLOT path39209032. After3933af60 eligibility and required-level checks,
userstate<2 emits FAILED_TO_SET_PET_AUTO_SKILL (0x11b). Otherwise3933c80f writes the local
raw-slot auto flag at mate+60+slot*8 and calls3933bbf0. That function rebuilds local
AI.skills.combat from enabled mount-skill→combat-skill mappings and emits
PET_AUTO_SKILL_CHANGED (0x11a) at3933bdcd. These functions have no separate configuration
packet send. This is local AI/action-slot state, not a justification to add a guessed
CS auto-use opcode or mark a network setting ACK. Anim/root owns the remaining original
pet Lua/control integration and exact eligibility predicates.

Validation: cargo check -p aa-net --features bevy and all 295 library tests passed.


### Current owned pet equipment flags (2026-10-18)

SC0x128 is `MateEvent::EquipmentFlagsChanged { mate_tl_id, container, slot, flags }`.
Native factory391b0540/vtable399b43fc uses serializer397b8e50..397b8ece: u16 TL,
u8 container, u8 slot, u8 flags. The lane declares this as SCUnkPacket4 and has no
emitter. No outgoing configuration command or acknowledgement is inferred.
Handler391db300 forwards to394e1d20. It requires container0xfc, the current owned
mate with matching TL and an existing runtime unit, selects slot0..27 at unit+4b8
(stride0xc0), writes descriptor+0x0d, then calls394e0d70 to refresh that location.

`Units::apply_mate_equipment_flags` changes only flags on an existing item in the
uniquely correlated `Mates.own` unit; it rechecks the actual unit's Mate kind, TL and
selected stored owner ID. It neither creates a unit/item nor changes Inventory,
Mates ownership or expiry state. Invalid slots safely remain raw notifications.
Both WorldState and the Bevy resource fold invoke this same helper after Mates
correlation. Repeated raw events remain observable even when the flags are unchanged.
Native SC127 expiry remains notification-only and cannot stand in for this update.

SC128 validation: cargo check and all 297 aa-net library tests passed, including the Bevy resource fold.


### Initial unit versus local doodad interaction (2026-10-18)

Native input dispatch390f3780 has distinct selected-object paths. At390f37b3 it
passes selected unit3a5a4dbc to393be030, which resolves39360020, checks local-player
state and393c01d0 eligibility, constructs CS068 and sends39186810. Constructor
397ac0a0/serializer397ba470 prove BC target, BC second object, i32 extra, i32 pick,
u8 mouse and i32 modifiers. The second object is initialized from static3a5e1158;
this does not prove it is the player object. No sentinel default is invented.
The lane systems/npcs/npc/packets.rs handler resolves only NPCs and Mates, returns
SC07b skills and explicitly leaves distance checking TODO. It has no SC07e emitter.

At390f37cc selected doodad3a5a4dc0 instead goes to393b79a0 and resolves39035110.
It creates local action data393b7320 and builds393bd290; empty action vectors return.
Otherwise it takes the first action, looks up its associated group393b5b30, and
calls393b3ec0 at393b7a70 with action, group, actual doodad and mode1. This local path
does not send CS068 or require a fabricated SC07e. The existing world manager then
performs eligibility, geometry range and special callback handling before casting.

Builder393bd290 requires runtime doodad+218 and static template3963b250, applies
player/faction/permission and other local predicates, then iterates all template
function groups (+24..28) at393bd421. Candidates are gated by393b3300(group+20),
3983b4e0,393b2c40 and other predicates. Accepted action/group pairs enter a map and
ordered action vector through393bd240;393b9a80 later filters map entries too.
Available natural interaction cannot be inferred from static CraftStart/Pack alone.
393b3300 is a permission switch, not proven phase equality. Protected keys remain
unresolved; this audit adds no guessed policy. Received SC07e/07f remain useful but
do not prove every natural workbench interaction starts with a network request.

SC128 refresh follow-up: containerfc dispatch394e0d70 to394e111d extracts slot
via394e0240, adds one and emits event0xd5 with one numeric one-based slot.
Registration39214ffb at39d2303c names it UNIT_NPC_EQUIPMENT_CHANGED
(string399be0f8). This occurs after the received descriptor flags assignment.


### Public transport telescope receive state (2026-10-18)

`RadarEvent::TransferToggled(RadarToggle)` receives SC1b4 as bool on + raw f32 range.
`TransferContacts { last, contacts }` receives SC1b5 as bool last, u8 count, then each
BC object ID, u32 template ID, packed9-byte position and string name. Lane source is
systems/services/radar.rs, used by the actual range-service producer. Its chunks are
ten contacts, with last on the final chunk; an empty scan emits no packet. Silence
therefore cannot clear old contacts or imply completion.

Native factory391b5380/vtable399b3fb8/397afc80 matches the toggle. SC1b5 factory
391b5410/vtable399b609c/397c4240 delegates entries to397bf320 (BC ID, u32 type,
3961e2c0 position and string bounded1024). Native consumes min(count,10); the active
lane decoder follows its writer's full advertised u8 count, preserving following
DD04 alignment even for count11. No payload guessing or automatic native profile.

Native toggle391df930?39366aa0 writes on/range and clears published and pending
buffers on every toggle. List391df9b0?393691e0 ignores data while off, appends
entries, and swaps pending into published when last is true. `TransferRadar` exposes
unknown toggle and unknown completed snapshot as None, retains raw ordering and
repeated IDs, and follows those received buffer rules. Character selection, leave,
kick and disconnect reset it. It never creates Units, assigns ownership or moves
existing world objects. WorldState and the Bevy plugin register the same resource.
An unfinished snapshot is locally bounded at4096 contacts: overflow discards that
whole in-progress snapshot through its last marker, retaining the prior completed
snapshot. This is a memory safety bound, not a claimed native/server contact limit.
Ship radar SC1b2/1b3 and fish contacts are separate formats and remain pending.

Transfer radar validation: all 302 aa-net library tests passed, including session receive order and Bevy resource publication.


### Ship telescope receive state (2026-10-18)

SC1b2/1b3 now use `RadarEvent::ShipToggled` and `ShipContacts`, with distinct
`ShipRadar` publication/pending buffers. SC1b2 native factory391b52f0/vtable399b3f60
uses397afc40 (bool on, f32 range). SC1b3 factory391d0d40/vtable399b7518 uses397c41c0
and397bf170: bool last/u8 count, each BC object ID, u32 summoner ID, i32 raw unit
kind, u32 template, packed position, u32 faction, u32 expedition, string name,
string summoner name, i32 HP/maxHP and u32 raw tail. Native name bounds1024/128
and min(count,10) differ from the lane's generic string/full-count writer; active
reading follows exact lane bytes without guessing a compatibility profile.
The lane radar service writes kind2 and tail0, but both fields remain received raw
values. Contacts never create Units, grant ownership or correct visible HP/motion.

Toggle handler391df830?393669c0 clears published/pending ship contacts even on a
repeated same toggle. List391df8b0?393690f0 appends only while enabled, swaps on
last, and leaves the published list intact between incomplete chunks. `ShipRadar`
follows those rules with the same explicit local4096-contact overflow/discard
boundary as transport radar. Transfer and ship channels do not clear each other.
Lifecycle resets, duplicates, unknown kind/tail, signed HP, truncations, count11
DD04 alignment, session forwarding and Bevy registration are covered independently.
The lane's empty-scan silence is retained as an explicit producer limitation.
Fish/school-of-fish radar remains a separate format.

Ship radar validation: cargo check and all 305 aa-net library tests passed.


### Fishfinder contacts and original map consumers (2026-10-18)

SC1b6/1b7 use `RadarEvent::FishToggled` / `FishContacts` and the independent
`FishRadar` resource. Native factories391b5500/vtable399b3fc4/397afcc0 and
391b5590/vtable399b60a8/397c42c0 prove bool+f32 toggle and bool last/u8 count,
then BC object ID, u32 template and packed position. Lane fish_school.rs writes
the same entry layout. Active decoding consumes its entire advertised count;
native caps10. Toggle39366b40 clears published/pending buffers; list39369350
ignores disabled data, appends and swaps on last. No Doodad/Unit is created or
fish population/state inferred. Lifecycle, independent channels and the explicit
local4096-contact partial-snapshot memory bound match the other radar resources.

Original world_map.lua346..356/441..460 and road_map.lua44..54/134..153 consume
UPDATE_TELESCOPE_INFO, UPDATE_TRANSFER_TELESCOPE_INFO and UPDATE_FISH_SCHOOL_INFO
with no arguments, forwarding to Map widget UpdateTelescopeInfo,
UpdateTransferTelescopeInfo and UpdateFishSchoolInfo. Corresponding UPDATE_*_AREA
and CLEAR_*_INFO handlers also take no arguments. REMOVE_*_INFO(arg) forwards one
argument. These scripts do not fetch a Lua table of radar rows: native Map widget
methods consume the C++ published vectors directly. An app bridge must provide
those methods using these received contacts, not synthesize visible actors.

Toggle calls directly emit noargument area events: ship0x18f, transfer0x190,
fish0x191 (registration39215ae1, names UPDATE_TELESCOPE_AREA,
UPDATE_TRANSFER_TELESCOPE_AREA and UPDATE_FISH_SCHOOL_AREA). Completed lists notify
the native map category observer via39202fb0/39202db0: category0x30 ship,0x31
transfer,0x39 fish. This is not yet proof of a direct synchronous Lua UPDATE_INFO
emission at packet decode; observer dispatch remains an explicit integration edge.

`X2Map:GetTelescopeUnitHealth(objId NUMBER)` registration3906962e selects390685b0
and39068210, then getter39068460. It scans published ship contacts39354c60 by
runtime objectID (entry+4). Absent returns zero Lua values; found returns TWO
NUMBERs HP then maxHP (entry+4bc/+4c0), both CVTSI2SS f32 values. It does not fall
back to arbitrary nearby Units or the owner's ship metadata.


Map observer closure:39202db0 dispatches category subscribers, and39051ba0 switch
routes categories0x30/0x31/0x39 to3904bd90/3904c270/3904c030. When enabled, these
compare cached map object IDs against the new published list, emit one numeric
runtime-object ID REMOVE event for each vanished entry, update their caches, then
emit noargument UPDATE_INFO. Ship event0x178, transfer0x179, fish0x17b; ship REMOVE
0x17c, fish REMOVE0x17f (transfer counterpart at3904c2dd). Disabled routes emit
CLEAR events instead (ship0x180 at3904bfd3, transfer0x181 at3904c4b3), with no UPDATE.
Thus the map-side diff is after publication, not a packet-level fabricated removal.
Area centering/render scale remains an aa-ui native Map widget contract to trace.

Fishfinder validation: cargo check and all 308 aa-net library tests passed, including
session receive ordering and publication through the registered Bevy resources.


### Pet displayed skills, XP edges and invocation boundary (2026-10-18)

Combat descriptor+8 is definitively `skills.ability_level`. SQL39a06a90 selects
id,ability_id,ability_level,... FROM skills.39731963..3973197a reads column2 to
temporary descriptor base(ebp-2c8)+8;39732d82..39732db2 copies through39719df0,
whose39719e04..07 preserves+8. Range index+26d8 is the396f6390 lookup. Example
compact rows: mountskill1/combat11016 requires10, mountskill3/combat11309 requires25.
Descriptor+1b0 is `skills.target_type_id`, SQL column132/0x84:397327a6..397327bd
stores it at temporarybase+1b0. Calling it channel6 is incorrect.3933b7a7 skips the
general displayed-usability validator only when target_type_id==6. The resulting
skillUsable is NUMBER1; learned remains independently compared to runtime mate level.
Other contexts use393a0a80, an obfuscated general validator; resource/cooldown policy
is not proven by a generic ready default. Context failure uses native result0x2c.

Attached records preserve cursor order, not an explicit native ID sort. SQL399ff650
SELECT id,attach_point_id,mount_skill_id,skill_id FROM mount_attached_skills has no
ORDER BY;3967a87f..a8df stores cursor sequence and3967ab30 iterates that raw array
before appending39644670 into parent vectors. Current compact schema id int PRIMARY
KEY is not a rowid alias; full-table scan starts IDs105,106,107,118,123,12. Only one
current parent has multiple records and that parent already happens to have ID order.
Thus no current first-row regression is claimed, but an ID-order source contract
would be false. First attached-point matching and unknown entries stay explicit.

XP-derived level3989a760 starts1 and advances on signed XP >= next level threshold,
including equality. It scans through record101 then caps return at100. The getter
394425f0 does not consult configured max-mate-level.3989a530 returns next-current
threshold delta, zero only if a required record is missing. Compact levels100/101
have total_mate_exp16417500/16917500, so level100 still has delta500000: XP exactly
16417500 yields0 percent; XP>=16917500 yields100 percent while level stays100.
Configured cap50 does not make the getter invent100 percent at threshold50.
Percent3989a620 performs f32 ratio*100 then arithmetic clamp0..100; the four Lua
returns are percent,totalXP,currentThreshold,nextDelta, with integer values converted
through f32 by394411f0. Missing-threshold zero denominators are not a guessed full bar.

Received delta cap producer remains unresolved at protected initial-config handler
391d1d69..391d1f5a.39357022 reads runtime client+1c and passes it to3933a890 only for
the matching current owned mate. That function caches ceiling3a168328 ONCE under
static guard3a16832c, using levels.total_mate_exp at the first supplied max level.
Later deltas use that cached ceiling.3989a480 wraps nonpositive additions, and for
positive delta preserves already-at/above-cap values, otherwise signed-clamps the
wrapped sum. Raw SCInitialConfig byte8 is not yet proven assigned to client+1c;
no optimistic received-XP accumulator or arbitrary cap is supplied by aa-net.

IsPlayerMounting3906a880 (registered3906b666..b6aa) tests local actor+3af0 resolving
to actual UnitKind5. It does not require ownership or a particular seat. PetState
3906a070 suppresses state only when that attached object equals current own mate+20,
without a seat test. Any seat on the same pet suppresses; riding another mate does
not suppress the current pet's mode state.

Pet action vtable399bb0e0 click39208ff0 button1 invokes39209040?3933e630 using raw
mountskillID at slot+0c; button3 toggles local AI auto-use3933c750.3933e630 requires
that raw ID in the ten received slots, learned level and matching attached context.
Driver context1 builds casterkind3 with pet objectID and RAW mountskillID, and passes
default mount_skills.skill_id to39399630?393989e0. No attached override substitution
is performed at this point; general target planning remains separate. Unmounted use
requires attackability and follows protected validation plus local AI/script commands,
not an unconditional direct CSStartSkill. Displayed target_type6 usability bypass is
therefore not permission to invent self-targeted casts or successful completion.
At3933eb0b..11 the ORIGINAL raw mountskill argument is stored at mate+58;3933ec0b
looks it up with3963b5d0. This is active commanded mountskill identity, not a target
object ID, and must not be folded from Unit.target_obj_id for mode duplicate gates.


Manual-use target input is independently closed:3933e7a8 queries local actor+1aa0
property4 and writes a target-kind0 objectID payload (+8), copied by39094150.
Native UnitTarget setter394270a0 (diagnostic399d875c) reads old property4 at
394270f5..39427103, resolves39360020, then writes its new target argument to that
same property at39427130..3942713d. Thus this is the LOCAL PLAYER selected-target
object ID, not mana/HP. Subsequent393989e0 still performs genuine target planning,
validation and39396e20, so knowing this input does not establish an always-send API.


### Proficiency controls and lane downgrade reconciliation (2026-10-18)

The existing API already covers rank/slot controls: Command::Actability with
Upgrade/Downgrade{id,auto_use_aa_point} and Expand; Requested/RequestRejected are
local send outcomes, TierChanged/Expanded are received-only. Original confirmation
OkProc invokes X2Ability:UpgradeExpert(type) or DowngradeExpert(type), one NUMBER;
ExpandExpert() has no arguments. Native39438840/39438870 read auto-AA via397ccd50,
then393101c0/39310080 check eligibility and optional playerflag1000 currency gates
before sending. No duplicate new wire command or speculative expansion ACK is added.

The fresh native downgrade comparison is important: SC1c0 handler391d3f60 calls
3930c0f0.397a93a0 decrements its existing local grade and clamps point to the lesser
of old point and the new visible grade's up_limit (fallback10000). Upgrade397a9370
increments grade. The native handler does not assign the packet step to its local
map, although it uses that step for grade text. Active lane services/expands.rs
regrade instead assigns exact newStep.up_limit on downgrade and sends that step.
Its character_templates.rs loader uses ALL expert_limits ORDER BY up_limit ASC,
including SHOW=false rows, unlike the original visible-grade map. aa-net follows
this lane boundary and retains the received step; it does not emulate local drift.

`Actabilities::resolve_downgrade_points(event, up_limits)` reconciles the omitted
lane point total ONLY after apply received an exact matching downgrade id/step.
The supplied slice is the complete lane up_limit sequence, ordered ASC, indexed
from step0, not DB IDs or the SHOW-filtered native grade list. Missing/unsorted
limits, upgrades, requests, wrong profession/step and already-resolved evidence
return false. Point deltas, another tier report, a new snapshot or lifecycle reset
invalidate stale evidence; an intervening delta cannot be silently dropped by
restoring an old base. Unavailable catalogs leave point None. Successful resolution
sets exact lane up_limit, including the source-proven difference when old points
were below that limit. No currency/item state or completion event is invented.
Kicked now clears proficiency/pending/reset evidence alongside select/leave/disconnect.

Native grade callback3930c0f0 mutates state first, then emits event0xc4
ACTABILITY_EXPERT_GRADE_CHANGED(id NUMBER,isUpgrade BOOL,groupName STRING,gradeName
STRING), gated by static group+0c. tab_actability.lua uses id; chat uses all four.
SC1c1 handler391d3fe0?3930c250 writes count first, then event0xc5
ACTABILITY_EXPERT_EXPANDED() with no args. Generic lane errors/silent returns do not
identify an owned request and remain uncorrelated. Existing expansion cost-sign bug
and ignored auto-AA wire flag remain lane limitations, not fabricated client fixes.

Proficiency reconciliation validation: cargo check and all 312 aa-net library tests passed.


### Proficiency eligibility, payment and runtime ID closure (2026-10-18)

CanUpgradeExpert wraps native error-code predicate397a8e60 (zero means true).
It compares received base points, without modifyPoint bonuses, against the current
SHOW-visible expert row up_limit (missing row fallback10000), else error1fc. Current
step must be below GetMaxGrade (visible row count minus1), else1fd. The next row's
expert_limit0 succeeds without a capacity count. Otherwise the count of eligible
professions at-or-above the next grade must be strictly less than next expert_limit
plus the received expanded-expert byte, else1fe. This predicate does not check money,
AA Points or certificates. Missing received entries are treated as point0/grade0 by
the native lookup; an integration with an incomplete snapshot must retain unknown
state instead of presenting that fallback as an authoritative received profession.

CanDowngradeExpert397a8f90 requires an existing received entry with step>0; absent or
step0 returns1ff. It checks neither points nor payment. GetActabilityCountByGrade
3943b6f0 calls397a86b0 and counts EXACT equality (397a8700); upgrade capacity instead
uses397a8760 and counts grade>=requested (397a87b0). Both require a resolved static
group whose +0c flag is nonzero, and exclude globals3a5e112c,39acb620 plus all ten
language IDs accepted by397a7c60. This is not a count of every received map entry.

A bounded read-only runtime check closes those formerly protected ID values:
original PID48924, verified module string StandMode at399a08dc, has3a5e112c=0,
39acb620=19, and39acb624..39acb648 consecutively21..30. Thus these count predicates
exclude IDs0,19,21..30. IsLanguageActability397a7c60 recognizes exactly21..30.
Source artifact data/extract/aa-ui/native-proficiency-runtime-20261018.txt records
PROCESS_VM_READ-only reads, with no writes or UI input. A subsequent same-process read resolves the separate Enum exclusion global39ac99b8
as20. Enum filtering must not reuse capacity count exclusion19.

CanExpandExpert3943b940 only tests whether397a9540 finds key(receivedCount+1), with
native byte arithmetic. Neither funds nor certificates are checked. Expansion ends
at the first missing next key, not at a row-count-derived cap. GetExpandExpertInfo
3943ba20 uses the same offered row; requireLP is life_point, optional requireItemType
and requireItemCount appear only for a non-sentinel item ID. remainExpandCnt counts
consecutive rows AFTER the offered row. Current catalog keys1..14 therefore give
count0: offered1/remain13, count13: offered14/remain0, count14: no offer. These are
current data values, not protocol constants. Actual ExpandExpert3930c280 separately
checks nonnegative life_point against character vocation points (+63c), error269,
then physical certificate count via397ccda0, error1b, before sending CS101.

GetGradeInfo3943b170 always adds received expansion count to expertLimit, even when
the base is0; CanUpgradeExpert has its separate base0 unlimited shortcut. Native
SHOW-filtered rows, ordered by up_limit, define the grade index and GetMaxGrade;
current IDs1..7 yield grades0..6. Hidden rows8..31 remain relevant to the lane's
unfiltered downgrade sequence, not the native visible grade map. Missing grade
fallback397a8dd0 has upLimit10000 and other numeric defaults0 (currency-1 before the
playerflag override). Colors use uppercase eight-digit hex. With playerflag1000,
upCurrency/downCurrency are raw DB numbers and upPrice/downPrice decimal strings;
without it, currencies are number0 and prices string"0". Prices are not Lua numbers.

UpgradeExpert/DowngradeExpert action wrappers39438840/39438870 read the current
inventory auto-AA getter (raw received value==1), then run their action predicates.
When playerflag1000 is set, payment helper394f0eb0 uses the current grade row's cost:
raw currency0 checks cash (397d0b60); raw3 checks AA Points (397d0be0, inventory+C330);
raw4 checks cash first, then cash-plus-AA (397cf400) and the auto-AA/dialog path.
The C330 storage is source-named AA Points by GetMyAAPointString394c9580 registration
394c9dad. Raw4 can return a local confirmation/dialog result rather than send success;
this must not become a fabricated ACK. Other currency IDs fall through this helper,
which is not evidence of a complete general currency validator. Current visible
expert rows have zero raw0 costs. Getter eligibility and action payment are separate
contracts; received tier/expansion events remain the only authoritative settlement.


### Mate maximum pools and modifier producers (2026-10-18)

UnitMaxHealth registration394c4526 selects394bc180; UnitMaxMana394c4624 selects394bc430.
Both resolve the actual unit and call390b3a80 with attributes6/7. That reads the common
attribute component via398a0bc0. Mate kind5 is NOT evaluated as an NPC-grade formula:
398a0bc0 selects generic398b4e80, whose fallback398a0d60 maps attributes6/7 at398a0e08/
398a0e12 to unit formula kinds14/15. Evaluator39567110 indexes the formula by the actual
actor kind+408 (Mate5). For kind5,3956722b resolves the actor's NPC template(+414) through
396962c0, reads template+74 (mate_kind_id), and chooses the corresponding per-formula
coefficient. It does not use NPC template/grade/kind formula variables for this branch.

Current DB owner5 formula99(kind14) is `(level * 140 + sta * 12) * mate_kind`; formula100
(kind15) is `level * 15 + int * 10 * mate_kind`. Primary kinds22..27 each use
`10 + (level - 1) * 4`. Variable kind5 supplies separate HP/MP coefficients: mate kind3
uses1.1/0.9, for example. Kind1 at level5 with no modifiers has primary26, HP1012/MP335,
matching the lane's recorded independent original-client horse fixture. These are
catalog formulas, not constants to replace future template data. Inputs/coefficient are
stored as f32;395672b3 explicitly rounds the evaluated result to f32 before398b56d0
truncates to i32. A generic f64-to-i32 evaluation can differ near integer boundaries.

The common component additionally applies modifiers.398a0060 visits actual buffs
through3989e5c0, passive/static sources through3989e6a0, and optional attached provider
virtual+24. Buff callback398a0960 resolves the received buff, folds its unit modifiers
at descriptor+2b4 through3989fde0 using received stack/level arguments, and includes
conditional modifier path398a07c0 plus duration-dependent path3989fee0. Merely loading
Buff-owned flat rows is not a complete implementation of every buff predicate.
Native3989ffb0 separately applies the mate NPC template's vector+184 using received
level. Therefore Npc-owned template modifiers still contribute even though the base
formula owner is Mate5. Missing or unresolved source state must remain an explicit gap.

For kind0/5,398a0bc0 also includes cached gear contributions from398a0a50. It iterates
ordinary physical equipment slots0..18 (stride0xc0, offsets through0xd80), excludes
missing/ineligible item states, adds generated
primary-stat contributions through3989fa80 for attributes0..4, static item modifier
vector+dc, secondary item+20 modifiers, and socket-related item arrays+30. It also folds
slot27 through398a0350. Required inputs include actual received item template/raw grade,
condition/detail/socket state and catalog eligibility; equipment is not merely cosmetic.
Item-owned modifiers can directly change pools (current DB has272 attribute6 and229
attribute7 rows), beyond gear STA/INT changing the underlying formulas. Existing client
Buff/Npc-only modifier loading does not cover those direct Item contributions.

All applicable native flat/percent contributions aggregate before final pool arithmetic:
initial flat0/percent100;398a0d0d uses integer base+flat when percent==100, otherwise
398a0d15..35 computes trunc_i32(f32(base+flat)*f32(percent)*f32(0.01)), in that order.
It then clamps through395729f0 against unit_attribute_limits (current HP6 maximum10M).
This differs from sequentially applying each percent bonus in lane vehicles::stats::
int_bonuses. Lane uses the same Mate formula owner/variables but its bonus arithmetic
must not be advertised as an exact native oracle. No new receive packet is required:
this is derived state from existing received unit/equipment/buffs plus static data.
Read-only disassembly artifact: data/extract/aa-net/mate-stats-native/contracts-disassembly.txt.


Original pet_view.lua UpdateInfoView explicitly calls UnitInfo("playerpet") and
UnitModifierInfo("playerpet"). Its labels consume melee_dps, melee_critical_rate,
dodge_rate, armor/armor_percentage, magic_resist/magic_resist_percentage, health_regen
and mana_regen; health/mana labels use dedicated current/max getters. Native UnitInfo
registration394c4fc2 selects394bdc10, resolves the supplied unit via3945fcd0, and builds
that actual actor's stats with398aa290. This repeatedly queries its own398a0bc0 component;
substituting player attributes for a mate tooltip is not the native contract. Full
combat-derived percentage formulas and conditional modifier eligibility remain a
separate integration task; the pool/formula producer audit does not claim all tooltip
fields have been reconstructed.


### Battlefield initial queue getter and received producer (2026-10-18)

Original battlefield/entrance.lua ENTERED_WORLD calls X2BattleField:GetJoinedInfos(),
not GetParticipationCount/GetEntranceCount. It returns immediately only when curCount
and maxCount both equal0; otherwise FillParticipationInfo multiplies maxCount by2.
The native method always returns THREE numeric values, so a nil/no-result placeholder
causes an original UI error even when there is no queue. Registration394573b9 selects
394562f0 and three-number return helper39455e80. With M=manager+28 and queue counts
C0/C1/C2 at+2c/+30/+34, returns are:
`min(min(C0,M)+min(C1,M)+min(C2,2*M),2*M), M, manager+8(myCorps)`.
Comparisons are unsigned; outputs become Lua f32 numeric values, with myCorps signed.

Singleton3936ce50 points3a16efd8; static initializer39990650 invokes3936d0d0, which
explicitly sets state0, max/counts0, myCorps-1. Thus native initial getter is exactly
(0,0,-1), without requiring a server queue packet. This source-backed local default
must not be described as an authoritative received roster. IsInInstantGame3936d0a0
returns true only for manager state5,6,7,8; initialstate0 returns false.

Received queue producer3936e030 accepts only appliedstate2 and matching battlefieldID
(manager+4), writes maximum/counts, then emits UPDATE_INSTANT_GAME_STATE(event160) if
the static battlefield resolves397f2210. Applied reply3936dfb0 requires requestingstate1
and matching battlefieldID+corps; success becomesstate2, error resets3936d0d0. Cancel/
local reset3936d1b0/3936d260 call that constructor; no optimistic population is implied.

Active lane SC18b SCAppliedToInstantGamePacket is u32 battlefield,u8 corps,u16 error.
SC18d SCInstantGameStatePacket is u32 battlefield,u8 per-corps size,u16 corps0 count,
u16 corps1 count,u16 any count. Server instances/instant_game.rs broadcasts only to
actual applicants, deriving size from rule_set.corps_size; no world-entry initial
queue snapshot is sent. The typed Battlefield queue resource below now exposes these received reports.
Unknown later game transitions must not be replaced by invented success or capacity values.


Battlefield typed queue milestone: Command::Battlefield and BattlefieldEvent cover
CSApply0e1(u32 battlefield,u8 corps0/1/2), CSCancel0e2(empty), SC18b..18f. Native apply
3936f816 requires idle state0, records request identity/state1, builds vtable399d0ce0
through397ac7f0; serializer397bb9b0 writes exactly u32/u8. Cancel3936d1b0 resets local
manager before empty send. The aa-net resource deliberately keeps successful-write
Requested state separate from received admission/population, and retains its received
snapshot until cancellation report/lifecycle reset; sending Cancel is not a received ACK.
Static content/interaction/entry eligibility remains the original client's responsibility.

Battlefield::joined_infos reproduces the native capped received tuple and initial(0,0,-1).
During pending Apply, native locally exposes requested corps before ACK; this received
getter instead leaves corps-1 and exposes that known local input separately in requesting.
This distinction does not fabricate admission or counts and must not be mislabeled as
a fully emulated native pending getter.
requesting is only a successful write; applied requires matching SC18b battlefield+corps
and error0. Matching error resets state and retains raw u16 error bits. Queue reports
are ignored until received admission or for another battlefield. A successful Cancel
write blocks late admission/queue/invitation mutations without fabricating a clear;
only the cancellation report resets the retained received snapshot. Genuine accepted
SC18d provides optional queue snapshot; no admission creates artificial counts. All
raw reports still forward through session Event, even when state correlation rejects
one or the values repeat. WorldState and Bevy resource fold the same reducer and reset
on CharacterSelected/LeftWorld/Disconnected/Kicked. No units, currency or inventory
are created/changed by this queue slice.

SC18e invitation body is u32 zone,u32 instance,u32 ruleset,u8 assigned corps,u64
qualifiedID; SC18f timeout is the same without qualifiedID. Native factories391b3ed0/
391b3f60 install399b3dc8/399b3dd4, serializers397b6ec0/397b6f50. Invitation consumer
3936d810 requires appliedstate2 and corps0/1 exact or requestedANY2 with assigned0/1;
there is no battlefieldID in this message to compare. It enters invitedstate3, retaining
existing queue counts. The typed resource likewise retains the snapshot but blocks
new queue updates while invited. Timeout3936d2a0 requires matching zone,instance,
ruleset and assigned corps and returns to appliedstate2; wrong/stale contexts do nothing.
Native counters remain capped for display rather than being interpreted as actual
participant entities. Join/world-enter/gameplay state controls and results SC190+
remain outside this bounded queue slice; no original join/completion event is fabricated.

Critical SC18c transport distinction: native factory391b3dd0/vtable399b3db0 serializer
397c3bf0 transfers ONLY signed i16 (stream virtual+50), then signextends into its DWORD
struct field. Handler391de380 passes that field to3936d260, which ignores its argument
and resets native state<=4. The lane writer instead deliberately sends FOUR bytes
(u16 error,u8 branch,u8padding), based on older handler-width inference. Active dispatch
reads exactly lane four bytes and preserves all as Cancelled.raw, necessary for DD04
alignment. BattlefieldEvent::read_native_cancel_error is a separate explicit two-byte
source decoder; never choose a layout heuristically from remaining payload bytes.
SC18b error is likewise signed16 natively; public u16 preserves its exact wire bits.
No cancellation packet contains a context ID, so a delayed cancellation after reapply
cannot honestly be distinguished using an invented request token.

Tests cover command goldens, every prefix truncation for all five receipts, each receipt
followed by another DD04 packet, explicit native/lane cancel boundaries, initial getter,
request versus received admission, wrong queue IDs/corps,
capacity caps, exact invitation/timeout matching, lifecycle, WorldState/Bevy folding,
local inactive/missing transport outcomes and session event ordering. No live queue
transaction or server mutation was used.


### Pet equipment request and original method boundary (2026-10-18)

MateCommand::ChangeEquipment(Box<MateEquipmentRequest>) adds CSChangeMateEquipment0a9.
Construct with MateEquipmentRequest::new(stored character ID,mate TL,passenger ID,bts,
Vec<MateEquipmentEntry>); header fields are public, entries() exposes an immutable slice.
Each request has1..2 entries. First location must be physical BAG2; second must be mate
EQUIPMENTfc raw slot0..27. At least one side must contain an existing full equipment
item. The immutable entry echoes require nonzero template/database identity and an
exact retained55-byte Equip detail agreeing with parsed fields; synthetic/missing or
mismatched detail bytes are rejected rather than padded/invented. Full raw grade/flags,
reserved detail bytes, timing and ownership fields survive serialization. Native
3961f500 item encoding and lane read_wire_equip_item agree for these equipment items.
This is a request codec, not a static gear eligibility/ownership/range planner.

Wire header is u32 stored owner,u16 mate TL,u32 passenger,bool bts,u8 entry count.
Each entry writes first full optional Item, second full optional Item, then first
container/slot and second container/slot bytes. Empty item is just u32 template0.
There is NO success byte on the request. Native packet constructor394e50c0 installs
vtable399e6140 and opcodea9;397ac2c0/397cc9a0 initialize header,397cdfc0 appends inline
entry copies,397ac2e0?397d1340 serializes header and at most two entries,397d12a0 writes
both items then four location bytes. Native sender394e8b30 marks packed slot pending
keys397d23b0 before send. aa-net does not fabricate pending unlock/settlement or update
bag/mate equipment on Requested/RequestRejected. Existing SC126 is an ordered raw
notice, not an unconditional crossed swap; actual received UnitEquipmentsChanged and
item tasks remain authoritative. Lane can write success=true even on refused moves.

Original pet.lua uses X2Equipment:PickupMateEquippedItem("playerpet",slotNo) for left
click/drag and MateUnequipItem("playerpet",slotNo) for right click. Authored slots
1(helmet),3(armor),4(saddle),7(shoes) become raw0,2,3,6. Pickup wrapper3946e950 verifies
owned mate through3933b1f0 and current mate state+bc, decrements Lua index, constructs
containerfc, and uses the actual cursor. Bag cursor kind2 invokes394eb6c0?394e8b30;
empty cursor selects a local item/cursor operation rather than instantly unequipping.
Native sender also gates actor byte+3a76/current mate state and actual packed pending
slots, with error143/d0; those planner predicates are not reconstructed as always true.
The raw command cannot stand in for all cursor/bind-confirmation behavior.

MateUnequipItem wrapper3946f710 finds an actual empty bag slot397d19d0; none emits
error19 and BOOLfalse. Otherwise it invokes394eb6c0 and returns BOOLtrue, which is
local invocation, not ACK/successful inventory settlement. Pickup returns no Lua values.
GetEquippedItemTooltipText3946f1b0 resolves actual supplied unit and slot, then formats
its actual item through39423560; no valid unit returns no values. The original panel
refreshes on UNIT_NPC_EQUIPMENT_CHANGED and uses this actual mate item tooltip, not the
player's worn item. MAX_PET_BAG_COUNT is0 in the authored pet_view.lua, so dormant mount
bag loops are not evidence to invent working bag commands. CanMount/CanDismiss local
planner guards remain recorded separately in docs/aa-ui.md Original pet command gates.

Focused tests verify native/lane one/two-entry goldens for equip/unequip/swap, full item
reserved bytes and timing, exact four-location ordering, empty sides, malformed echo/
container/count rejection, and that requests/rejections/SC126 do not predict any slot
swap; actual UnitEquipmentsChanged alone changes the fixture's mate gear.

### Additional source closures for current integrations (2026-10-18)

Battlefield NPC mapping is exact: SQL39a01098 column52 recruiting_battle_field_id;
loader396d03e8 reads column34hex through3953a390,396d03f6 stores temp[ebp-15c], relative
to record baseebp-20c gives+b0. Copy396ba8f0 preserves+b0 at396baa9e..aaa4. RequestInstanceGame
wrapper39455e00 resolves NPC type, rejects invalid+b0, requires battle_fields397f2210,
clamps corps>2 to2, then sends via3936f780. CancelInstanceGame39455dc0 takes no arguments
and returns no values. Native manager state0 is required for Apply; no level/time/range
predicate appears in3936f780 before sending. It additionally searches actual player
buffs using3936e950: eight globals39ac7510..752c or any static buff tag1253 block apply.
Read-only PID48924, fixed-base StandMode verification, resolves those globals to
631,2028,3619,3623,3621,4868,6295,4876 in address order. Artifact
data/extract/aa-net/battlefield-native/runtime-blocker-ids-20261018.json records access
PROCESS_VM_READ|PROCESS_QUERY_LIMITED_INFORMATION only; no writes/input/control.
A matched blocker shows native text3936f3b0 and cancels/reset state<=4; missing local
player also prevents sending. UPDATE_INSTANT_GAME_STATE160 and INSTANT_GAME_JOIN_CANCEL156
are noargs. Invitation opens native confirmation dialog3936d620; timeout3936d2a0 closes
it and returns to appliedstate2, without a separate fabricated Lua timeout event.

Radar toggle timing is AREA first, immediate category observer second, after both
buffers are cleared: ship39366a1e then39366a2b..36(category30), transfer39366adc then
39366ae9..af4(category31), fish39366b9e then39366bab..b6(category39). All invoke
39202fb0(category,0)?39202db0. Enabled observer diffs cached IDs and emits REMOVE per
vanished contact then INFO for empty published data; disabled emits CLEAR. It does not
wait for list completion. Transfer removal is specifically17d at3904c2fa..30a.

Auto-AA default is source-backed constructor state:3961eb10 clears EBX at3961eb45,
then3961ebe1 writes inventory+C340=0. Getter397ccd50 returns that field==1. Character
state/action18 later hydrate it; no OS/Lua persistence key has been established or is
needed for the authoritative setting API. The later enchant feature-storage closure below proves native clientflag1000 is FeatureSet byte5 mask10; it is not Unit.flags. Actual received bytes remain required.

Mate modifier3989fde0 no-stub Unicorn oracle confirms per-level value arithmetic is
NOT banker's rounding. For nonzero linear coefficient it computes wrapping i32
linear*level, converts to f32, multiplies0.01f, adds0.5f, floors with SSE, adds base value
as f32 and truncates to i32, then multiplies by stack/multiplier and accumulates flat
or percent. Base10: linear1/level50 gives11,1/150 gives12,-1/50 gives10,-1/150 gives9;
stack3 turns11 into33, stack0 contributes0. Npc modifier caller passes actual received
unit level, not0. Oracle data/extract/aa-net/mate-stats-native/modifier_oracle.py.
Gear slot is wholly skipped when max durability397ce6b0(template,rawGrade)!=0 AND
actual durability byte+1d==0; max0 non-durable items are not treated broken. This
excludes generated attributes, direct Item modifiers, rune and gems together.

Buff modifier callback398a0960 uses runtime+40 stack multiplier and unsigned word+2e
ability level. However SCBuffCreated391d5e00?3938a3c0?3989e080 initializes NEW stack
count1 at3989e1e8, while incoming charge goes to runtime+44 at3989e1ef..f2. Repeat
stack_rule4 increments+40; descriptor max_stack+114 caps it. Current aa-net Created
charge.max(1)?stacks is an explicitly incomplete lane convenience, not native stack
proof. Unit snapshot NetBuff has genuine separate stacks/charged fields. A subsequent
catalog-aware native stack reconciliation is needed before claiming all buff stat
multipliers exact; gear request work does not silently reinterpret charge.


### Catalog-aware native buff stacks (2026-10-18)

`Buffs.native` is an additive `NativeBuffs` sidecar. Install actual `buffs` rows with
`set_catalog(HashMap<u32, NativeBuffRule>)`: `kind` maps kind_id1/2/3 to Good/Bad/Hidden;
remaining public fields are per_unit_creation, stack_rule_id, max_stack and max_charge.
`native.of(objectID)` returns `Option<&[BuffEntry]>`. A genuine unit snapshot seeds its
separate stack/charge fields even before catalog installation. Created packets require
known prior unit history and descriptor knowledge to mutate the view. Missing rules before a complete catalog installation leave None; a complete catalog proving a missing descriptor instead gives the native no-op.
Unknown history is not repaired by installing a catalog later. UnitsRemoved and session
selection/kick/leave/disconnect discard received views; static rules survive. Raw lane
`Buffs.of` remains index-based and retains the incoming charge convenience count.

Native SCBuffCreated handler391d5e00 passes detail, caster, casterCharID and index to
3938a3c0. Its call to3989e080 passes detail, static template, caster and casterCharID:
runtime+28 is **casterCharID, not index or caster objectID**. Template loader SQL39a07550
and stores397342d9/397343b2/397343bd/39734552/39734d4f establish kind_id?+f4,
max_charge?+110, max_stack?+114, per_unit_creation?+12b and stack_rule_id?+194.
The +f4 field is not group_id. Manager vtable399d1990 +4?39387770 finds first matching
(kind,buffID); +8?39387710 additionally matches casterCharID. Rule6 always allocates.
New records start stacks1, charge=received charge (3989e1e8..f2). Existing records retain
caster/ability/skill metadata: rule4 increments stacks; rule2 replaces charge; rule3 adds
charge. Every result caps stacks at max_stack; only rule3 caps charge at max_charge.
Rule5 adds duration; others replace it. Rule5 selects greater elapsed, other refreshes
select lesser elapsed. The sidecar retains packet-time elapsed baselines; it does not
reproduce the native per-frame tick scheduler. Modifier398a0960 consumes runtime+40
stack count and unsigned runtime+2e ability level. It must not consume raw charge.

A new incoming index rekeys the merged record when it was not previously present
(3938a517..20); removal follows that index. An index colliding with another record marks
the sidecar unknown rather than guessing the manager replacement path. Snapshot resets
restore known state. The small-detail four PISC values are charge, duration/10, elapsed/10
and tick/10 (397c9070); the decoder now retains the third value instead of discarding it.

This is explicitly native presentation/state folding, not a claim that the lane's server
buff model matches native stacking. Lane models/buff.rs writes snapshot stack1/charged0,
and its refresh/charge/extend replacement algorithm differs. Raw values and lane list
remain available. No wire format, command, modifier policy or server state was changed.
Regressions cover independent vs per-caster grouping, separate charge rules, stack caps,
index/removal identity, missing catalog/history, snapshot preservation, session resets,
and a trailing-byte golden for nonzero received elapsed.

Additional proficiency event producer proof: SC136 handler391db5e0?3930bc60 ignores
packet bool+c, rebuilds rows and emits0x55 ACTABILITY_REFRESH_ALL() at3930be5a on every
packet. Labor handler391d3db8 invokes3930be90 only for nonzero proficiency delta. After
capped update, a resolved skill_page_visible group emits0xc3 ACTABILITY_EXPERT_CHANGED
(id NUMBER, localizedName STRING, actualCappedDelta NUMBER, postChangeTotal NUMBER)
via3930bf95?394242a0. Total includes recognized unit-attribute modifiers but excludes
GetMyActabilityInfo's separate enclosing-house bonus. There is no ACTABILITY_CHANGED
notification in this path. Native event producer chunk behavior and the lane resource's
accumulated snapshot completion are distinct contracts.

Direct Item modifier level closure: items SQL39a00d90 column22 is `level` (not
level_limit/level_requirement). Loader396cb701 reads it,396cb70c stores low byte at
temporary?f4; temporary base?140 is passed396cbb4d..54 to396ba730, so descriptor+4c
is that byte. Call398a0af2 sign-extends it and supplies multiplier1. Rune runtime+20
and each gem runtime+30 DWORD are resolved directly by the same item-ID lookup39695dc0,
then contribute their own descriptor+dc modifiers with multiplier1 and signed own level.
Seven gem entries are visited, stopping at first sentinel. Base-slot eligibility rejection
at397cc080 skips all those contributions; it is a descriptor impl_id switch with static
subtype/equip-slot mask lookups, not a proven simple use_as_stat predicate.


### Mate equipment eligibility and count helper follow-up

397cc080's jump table at397cc160 accepts only impl1(weapon),2(armor),22(backpack),
24(accessory). Weapon resolves396a6d40 then holdable39695ab0, takes holdable+74
slot_type; armor resolves39695e00+30; accessory396a6d00+20. Backpack396a6e40+8 must
be type2 or7 and uses slot_type30. All other impls reject. Nonzero slot_type must map
through3956f2e0 primary or3956f430 secondary to the actual raw equipped slot.
Primary mappings:1..9?0..8,10?9,11?11,12?13,13?14,14?15,15?16,16/17?15,
18?17,19invalid,20?16,21?18,22invalid,23..31?19..27. Secondary only10?10,
11?12,17?16; otherwise?1. The separate slot27 helper398a0350 applies the same
broken-item/static-slot checks and Item/rune/gem modifier calls as slots0..18.

Neither outer helper nor generated-primary3989fa80 checks use_as_stat. Generated
primaries require attr0..4, a resolved subtype mod_set, selected nonzero weight and
nonzero weight count. Count397cc670 tests nonzero DWORDs, not positive-only; selected
weight397cc610 and total397cc6b0 are consumed as low unsigned bytes. Main arithmetic
uses x87/f64 intermediates, unsigned config constants, signed item level byte, a narrowed
f32 power result, f64 exact0.01 (39a1fe78), signed grade stat_multiplier, then f64?f32
and floor(x+0.5f). Weapon applies a second equivalent round after holdable stat multiplier.
Power exponent uses unsigned stat_value_const times f32 0.01 (3999cf90), reciprocal;
bases are3 for one weight,1.5 for two,1 otherwise. Existing client tooltip/stat f32-chain
helpers are not proven exact substitutes. No use_as_stat eligibility was inferred.

CountBagItemByItemType registration39450cfe points to3944de50. It uses bag global
394e0960?3a5ad9a0 and3961faf0(template,nullFilter), summing physical available bag slots
with matching template+8 and count+10. It has no bank, flag4, secured-item or pending-slot
filter. Native expansion3930c2e8..f4 instead calls397ccda0(template,false,true): same bag
count but with inventory+C348 pending-location map exclusion; no secured-item flag test.
Thus the ordinary Lua count getter and actual expansion preflight differ for pending slots.


Native3989fa80 oracle (ignored helper `mate-stats-native/item_stat_oracle.py`) executes
its full x87/SSE arithmetic and397cc610/670/6b0. Only lookup return pointers are mocked
to current SQLite descriptors and external MSVCR100 `_CIpow` is mocked with Python
libm double after spilling native x87 arguments. Thus this is not a no-stub claim for
the external pow implementation. Real raw grades0/1/6, attributes STA/INT respectively:

| Template | Level | Grade0 | Grade1 | Grade6 |
|---|---:|---|---|---|
| armor31432 |62|29,23|23,19|41,33|
| accessory32920 |45|5,4|4,3|7,6|
| weapon28905 |62|32,63|26,50|44,89|

Config item_stat350, holdable_stat10, wearable_stat90, stat_value110. Weapon modset64
weights STA1/INT2; two-weight pow base1.5, exponent0.9090909294106748, narrowed result
1.4457159042358398. Its base double is31.372035121917726, first pre-round STA values
20.914690081278483/16.731752065022786/29.280566113789877; first rounded values21/17/29.
Holdable multiplier150 is then applied and separately rounded to32/26/44. The JSON
artifact records every row's actual config/weights/coverage/grade and intermediate values.
This evidence supports a distinct native helper rather than reusing f32 tooltip formulas.


Previously unnamed Buff descriptor+128 is `buffs.one_time`: SQL39a07550 column0x58,
loader397344ce..f4 stores parsed T/t/1 boolean at temp?218, record base?340. Native
398a097d..84 skips the entire Buff modifier callback when this is nonzero, including
base, conditional and duration-dependent vectors. This is distinct from charge/stack
rules and must be checked before applying the resource's native stack multiplier.


Conditional/duration Buff presence is identified by child rows, not a scalar `buffs` flag.
Conditional vector+234: buff_unit_modifiers(owner_type='Buff',owner_id=buffID), each
row's id joining unit_modifiers(owner_type='BuffUnitModifier',owner_id=row.id). SQL
39a0c158 loader3973d14a builds gameDB+11a0; buff loader39735483..3973555a selects
'Buff'/owner and appends into temporary?10c (=record+234). Consumer398a07c0 tests
current unit buff identity/tag predicates before adding the linked unit modifier.
Duration vector+254: dynamic_unit_modifiers(buff_id=buffID), SQL39a08e00 loader3972a800,
postlink39735b80..bc8. Rows directly name unit_attribute_id and unit_modifier_type_id,
with LinearFunc or ManualFunc references. Consumer3989fee0 is called only when runtime
tick_ms AND duration_ms are nonzero (398a09bb..ca), and uses tick index / total tick count
with runtime stack multiplier. A base-only stat adapter can identify unsupported relevant
rows explicitly rather than silently dropping these contributions.

Current compact DB conditional joins contain no attributes0..7. Relevant dynamic rows:
HP6 buffs68,698,699,700,701,711; MP7 buffs712,1892..1895; primaries2591(STA/SPI),
6663(STR/DEX),6664(STA/INT),6665(STA/SPI),6666(STR/SPI). These are observations of the
current rows, not protocol constants; load child associations and test actual attributes.


### Native mount/dismiss local model-action boundary

Local actor+3be8 is source-named ActorUnitModel, not a network combat-state component:
constructor390e53bd uses vtable399a4f68; destructor diagnostic399a521c names
`X2::ActorUnitModel::~ActorUnitModel`. Getter390b3750 returns it. CanDismiss39357861
calls virtual+8c?390e2f90, which returns model+58. That field holds a pending model-action
object: mount390ca4d0 constructs390a5f00/vtable399a30b8, dismiss390ca560 constructs
390a6110/vtable399a3108, and other model actions also occupy it. This closes its meaning
as a local action pointer; no received Unit.flags substitute or guessed combat flag is valid.

Lua DismissPet39069790?39362070 first validates39357830 and clears pet target through
39361fc0(2). If local player is attached to the SAME owned pet, it invokes39361e80(reason10)
and returns without directly sending CSa4. Otherwise it checks shared2s cooldown3a169944,
queues ActorUnitModel390ca560(targetPetObject,reason10,0), sets mate+bc pending and emits
Lua event0x111 DISMISS_PET() (registered3921537f..89, string399bdc0c).
This is local initiation, before removal send/ACK, and does not remove received ownership. The action finalizer virtual+40=390961f0
resolves target and current owned mate, requires reason10 and targetobject==ownedmate+20,
then3909626e..277 calls3933b230 with owned timeline+14 to send exact existing CSa4.

Update390aaff0 marks done after elapsed>1000ms for reason10 (>500ms reason9), alongside
model/fade updates. However this is NOT permission to emulate dismissal with a fixed timer:
ActorUnitModel virtual+98=390ca040 calls action virtual+40 at390ca120 before destruction
and clearing+58, and action replacement invokes this common finalizer too. Mounted
reason10 unmount continuation is detailed below. Existing raw
MateCommand controls remain exact protocol operations, not full native local planner APIs.
No additional receive packet or synthetic completion was added on this evidence.

Formula precision cross-check:395672b3 explicitly FSTP DWORD[ebp+8], then395672b6 FLD
DWORD[ebp+8], narrowing formula result to f32 before downstream398b56e2 FSTP QWORD and
398b56e5 CVTTSD2SI. A398a0bc0 initializes flat0/percent100, folds Buff398a0060 and
Npc3989ffb0 into that same pair, then398a0cf6/0cfb adds cached gear flat/percentage deltas.
Only after all sources combine does398a0d08..35 apply one percentage multiplication
(or integer add when percent100), then395729f0 clamps to attribute limits.

Mounted continuation is present at ActorUnitModel virtual+280=390cea80 (slot399a51e8).
The detach-model callback clears seat bookkeeping, resolves actor/controller objects and
static3963b490(second argument, model_attach_point_strings.id); only successful controller/static resolution reaches
390cec02..0e, queuing390ca560(parentModel+10 target, third-argument reason, true) on
the detached child model. A reason10 finalizer can then send CSa4 with matching owned
identity. String399a2d44 names `UnMountHorseController`. Loader39678963 uses gameDB+98d0 and SQL399fa26c
`SELECT id, actor, prefab FROM model_attach_point_strings` (label399fef2c).
Receive handler391d4a70 (`OnUnitDetached`, diagnostic399b7e34) resolves packet child+c
and passes reason+10 to39355bd0?390b4000. The latter clears actor attachment+3af0/3af4,
then390dca30 passes saved child model/seat/reason to parentModel virtual+280. Thus the
existing UnitDetached child/reason event has sufficient wire data, but model/controller
prerequisites and action finalization remain real local work, not synthetic completion.


### Battlefield invitation response and joined snapshot

`BattlefieldCommand::RespondToInvitation { invitation, accept }` retains the captured
received BattlefieldInvitation identity. `Battlefield::respond_to_invitation` and
`validate_command` require an exact current invitation with no prior response, pending
cancel or joined snapshot. Session mirrors the same received/request/lifecycle fold and
rechecks immediately before writing, preventing stale UI replies from reaching the lane.
Successful writes set only response_pending; invitation remains a received snapshot.
No acceptance is fabricated from a write. Rejected/inactive/failed sends use the existing
RequestRejected event and permit retry while the invitation is still current.

Native invitation object vtable399d0d1c callback+14=3936f890 forwards captured
zone/instance/ruleset/corps/qualifiedID to3936f540. It requires state3, exact identities,
requested/assigned corps compatibility, local actor and the same blocker predicate.
Choice1 sends CSe3(true,qualifiedID), then localstate4; choice2 sends CSe3(false,0),
then localstate2; choice3 invokes Cancel(true). Packet constructor397ac810 and serializer
397b1090 prove bool then u64. Lane CSJoinInstantGamePacket reader matches, but
player_invite_response ignores qualified_id, making the client identity guard essential.
Lane decline withdraws/reapplies the same battlefield as corpsANY2; the resource clears
reply/invitation only upon that matching genuine Applied response, not the local write.
This is an explicit lane behavior difference from native localstate2 after choice2.

SC190 Joined factory391b401c/vtable399b3de0 serializer397b6fd0 and lane packets.rs
SCInstantGameJoinedPacket agree on fixed135bytes: zoneu32,instanceu32,corpsu8,
victoryScorei32,victoryKillCountu16,victoryKillCorpsHeadu32,threei32 phase durations,
twou32 factionIDs,24u32 killstreak skills. BattlefieldEvent::Joined preserves all raw
fields. Resource hydration requires an accepted pending reply and matching zone/instance/
assignedcorps, mirroring3936d890's state4 and identity gates. Native3936e0c0 emits0x157
ENTERED_INSTANT_GAME_ZONE() only after a matching Joined is applied (name registration
392157b7, string399bd688). Original dialog_handler.lua44..66 uses
DLG_TASK_JOIN_BATTLE_FIELD with three buttons enter/next-room/reject and infoTable keys
name/corpsName. Native fill3936d6b0 resolves ruleset?battlefield name and assigned corps
faction localized name (ANY_CORPS for2). Invitation opens a dialog task; no invented
INVITED Lua event is added. Accepted replies ignore old invitation timeout
(state4); unaccepted matching timeout can clear invitation/reply. Selection/kick/leave/
disconnect reset all received and local pending state. Wire carries no qualifiedID in
Joined, so an old identical zone/instance/corps packet cannot be distinguished by one.
Joined is a received snapshot, not a complete match-phase state machine: start/end/scoreboard/leave packets and timers remain separate follow-up work;
explicit CSe4 continuation is detailed below.

Root Lua/source boundaries: CancelInstanceGame39455dc0 calls3936d1b0(true) synchronously.
For state<=4 it resets/defaults and emits INSTANT_GAME_JOIN_CANCEL() BEFORE sendinge2;
state>4 is a no-op. Apply3936f780 sets localstate1 before send and does not roll back a
failed send; repeats nonidle reporterror106. Blocker refusal resets/cancels state<=4.
Native RequestInstanceGame39455e30 clamps signed values>2 to2 but leaves negative i32
unchanged locally (eventual wire lowbyte); bounded typed command deliberately permits
only0..2. Fractional/nonfinite generic Lua conversion has not been established here.
Applied3936dfb0 only processes exact pending field/corps: nonzeroerror resets/defaults
then reports39428370(error), with NO cancel Lua event; zeroerror changesstate1?2 with
NO Lua event. UPDATE_INSTANT_GAME_STATE follows later genuine queue-state notification.


CSe4 continuation: `Battlefield::world_ready(capturedInvitation)` produces additive
`BattlefieldCommand::EnteredWorld { invitation }` only for the exact retained invitation
and a successfully written accepted response, with no cancel or joined state. Session
rechecks the same identity immediately before socket write. The app alone invokes this
after its actual world-entered continuation; net never auto-emits it from SCLoadInstance,
invitation or acceptance. Native3936d2f0 state4 emits UPDATE_INSTANT_GAME_STATE() then
constructsCSe4 through397ac830 with stored qualifiedID; serializer397b10d0 and lane
CSEnteredInstantGameWorldPacket reader agree on u64 only. Native remains state4 afterward,
so explicit retry is permitted, with no invented pending timer or received join mutation.
A stale qualifier/zone/instance/ruleset/corps, declined reply, pending cancel or absent invite
rejects the command before sending. Only matching genuine SC190 populates joined.


Invitation close/ESC is not a custom choice: original default dialog OnCancel invokes
X2DialogManager:OnCancel(id). Native3905eb60?3902d1c0?3902cf60 uses helper3902b2f0,
which calls task virtual+18 with boolean(buttonType==1). Invitation vtable399d0d1c+18
is3936cf20 RET4, a no-op. The task closes without a queue reply; native state3 remains
until timeout. Only OnCustom invokes virtual+14?3936f890 choice1/2/3. Therefore an
adapter must not turn ESC into choice3/Cancel or choice0/decline. Genuine timeout closes
the task through3902cef0 without invoking a response callback.

Recruiter skill producer:393c1310 maps skillglobal39ac79b0 to 'recruiter'; a module-verified
PROCESS_VM_READ read of existing original PID48924 (StandMode marker, original image)
finds12660. Artifact battlefield-native/runtime-recruiter-id-20261018.json records it.
Native393bf3bf compares that exact skill, resolves targetNPC+41c?3936d5f0 static recruiting
battlefield, then398a99c0 obtains last loaded ruleset minimum level. If positive and player
levelbyte+4b4 is lower, it emits restriction/error and does not open. Otherwise393bf45f
emits NPC_INTERACTION_START('recruiter') (event0x6b name399be9b4, registration392149c5).
CurrentDB skill12660 has no skill_effects or npc_interactions child rows: this is a local
special service path, not an invented WorldInteractionEffect. Lane npc/packets.rs
interaction_option selects12660 for recruitingfield>0 only after banker, ability changer,
auctioneer, priest, repairman, merchant, stabler and expedition are false; before blacksmith.
StartInteraction sends singleton[option] for an actual NPC in the player's current instance;
distance check is explicitly TODO. Static field alone is not a received interaction context.


### Battlefield later phases and leave (original 1.2 / active lane)

The additive `BattlefieldPhase` resource states correspond to native manager states 5
(Joined), 6 (Opening), 7 (Playing), and 8 (Ending). `in_instant_game()` includes all
four, matching `3936cfd0` / `3936d0a0`; receiving results does not mean the player
has left. `joined_ruleset_id` retains the matched invitation's ruleset across the
SC190 transition because SC190 does not repeat it.

SC191 is exactly `u32 zone, u32 instance, u32 now, u32 start` in both transports:
native serializer `397b7100`, handler `391de630`, and lane
`systems/instances/packets.rs::SCInstantGameStartPacket`. Native `3936d9b0`
requires state5 and matching zone/instance, writes state6, stores engine-mapped
clock+6000 at manager+3ce8 and received start at +3cec, then emits event0x159().
The received `now` argument is passed but **not read** by this function. Wrapper
`3936e110` immediately enters7/emits0x15a() if start<=engine-mapped now; frame
update `3936cf50` instead enters7 when unsigned wrapping `(now+6000)>=start`.
`advance_server_time(mapped_now_ms)` exposes that frame rule explicitly. The
engine timer virtual+10/+20 clock mapping is now supplied by the source-proven
Pong/TimeKeeper integration below. No SC191-now-to-Instant conversion or Session
clock command is invented; before an actual clock sample, later leave stays gated.

SC192 native factory `391b4110` uses vtable399b3df8, serializer `397c3c50`, and
team serializer `398a97b0`. Active lane writer `SCInstantGameEndPacket` agrees:
`u32 zone,u32 instance,u8 reason`, then two teams, each
`u8 state,u32 faction,i32 score,i16 kills,i16 deaths,u8 memberCount`; each member
is `u32 character,string name,i32 score,i32 bonus,u32 bonusSet,u16 killstreak,
u16 kills,u16 assists,u16 deaths`. Native inline storage clamps memberCount to50
before consuming rows (`398a986d`); the active lane writes the full u8 count and
all rows. The active decoder deliberately consumes all lane rows (up to255),
without a payload heuristic or native clamp that would corrupt the next DD04
entry. Native names use a128-byte destination, while active lane strings retain
the normal u16 wire bound. Native handler391de6b0?3936f310?3936e9f0 admits
only state7 and matching identity, enters8, stores reason+38 and mapped-clock
ending baseline+3cf0, copies both result teams into+58, then emits0x15c().
`last_end` retains every genuine raw receipt separately from phase-admitted
`result`; missing clock integration cannot silently discard the received data.

`BattlefieldCommand::Leave` is empty CSe5, native3936d210 (state8 only), and does
not clear received state or invent completion. Native3936f1c0 (called from
SCLoadInstance handler391de970) resets the manager and emits0x158() for states
5..8 before world loading; accepted pre-join state4 is preserved so CSe4 remains
possible. The received `Event::LoadInstance` now performs that same state reset.
Lane `leave_instant_game` returns through the existing SCLoadInstance flow; no
separate leave acknowledgement is fabricated.

Verification: 341 aa-net library tests passed with Bevy, including all Start/End
truncations and DD04 trailing-packet alignment, non-ASCII result names, the lane
255-member boundary, stale identity/phase refusal, explicit clock threshold,
received-only leave/reset, preservation of accepted invitation world-ready, and
Session loopback proof of the clock-gated no-send path followed by exact empty
CSe5 and received LoadInstance reset.

UI-agent independent runtime/module trace closes the timer transform itself:
CrySystem timer virtual+10=3656f740 converts timer[index0] i64 microseconds to
milliseconds; virtual+20=3656ea30 forwards timer+2a8 to33025600, returning
wrapping `input + mapper[2c] - mapper[30]`. The mapper-offset producer is the native Pong/TimeKeeper path documented
below; received SC191 now is not used as that native timer mapping. Source artifact:
`data/extract/aa-net/battlefield-native/later-phases-20261018.txt`.

Native callback names are confirmed by event-name registration39215799..392157ee
against table base39d22ce8: state5 entry is ENTERED_INSTANT_GAME_ZONE() (0x157),
opening is INSTANT_GAME_WAIT() (0x159), playing is INSTANT_GAME_START() (0x15a),
results is INSTANT_GAME_END() (0x15c), and received-load reset is
LEAVED_INSTANT_GAME_ZONE() (0x158). Each has no arguments. Applications must
use admitted transitions, rather than forwarding an unmatched packet as one of
these native UI events.


### Battlefield received score and skill-enable updates

SC193 `BattlefieldPoint` matches native397b7150 and lane
`SCInstantGameAddPointPacket`: u32 zone,u32 instance,u8 corps,i32 point delta,
two i32 cumulative scores,string character name. Native3936d370 accepts only
states6/7 and matching zone/instance, **overwrites** both manager scores+3c48,
and emits INSTANT_GAME_ADD_POINT(corpsName,characterName,points as f32), event
0x15b. CorpsName is resolved through retained ruleset ID and received corps.
`Battlefield::accepts_point` exposes the pre-fold admission guard so repeated
genuine notifications need not be suppressed just because scores repeat.

SC195 `BattlefieldSkillEnable` matches native397b7410 and lane
`SCInstantGameKillstreakPacket`: u32 zone,u32 instance,**signed i8** index,u32
skillID,bool enable. Native3936d4a0 requires state7 and matching identity,
compares skillID against the SC190 24-slot table, then writes the enable byte
only if it changes and emits UPDATE_INSTANT_GAME_KILLSTREAK(), event0x15e.
The safe decoder retains all signed index values; the fold rejects indices
outside0..23 rather than reproducing native unchecked array indexing.
`accepts_skill_change` additionally checks the old enable value for exact
notification admission. No cooldown, learned state, or skill execution is
invented. Joined explicitly initializes scores[2] to0 and enables[24] tofalse
(native3936d890); received load and full session lifecycle reset them.
`last_point`/`last_skill_enable` preserve raw receipts independently of guards.

SC194 kill notifications and their per-corps kill totals / local killstreak
producer remain a separate bounded follow-up. Native3936e180 updates local
killstreak when killer or victim matches the local character **before** checking
phase/instance, then only emits the main kill notification in states6/7 with
matching instance. This order must not be approximated as the score guard.


### Battlefield SC194 kill totals and local streak ordering

`BattlefieldKill` is the exact native397b7250 / active lane
`SCInstantGameKillPacket` layout: u32 zone,u32 instance,u32 killerCharacterID,
u32 victimCharacterID,u8 killerCorps,u8 victimCorps,signed i8 killerKillstreak,
string killerName,string victimName,i32 killerCorpsKills,i32 victimCorpsDeaths.
Native handler391de7c0 passes these to3936e180. Before any phase or instance
check, it compares actual local character ID against killer first, then victim.
Both matches store the received signed killerKillstreak in manager+3cd0. Killer
emits UPDATE_INSTANT_GAME_KILLSTREAK_COUNT(received as f32); victim emits the
same callback with0f32 **without** storing zero in the getter. This asymmetry
is explicit in3936e1ab..3936e228. `local_killstreak_notification` exposes that
notification value independently from stored `my_killstreak_point`.

Afterward, only states6/7 and exact zone/instance admit the main kill update.
Native3936e48d overwrites `kill_scores[killerCorps]` (+3c50/54) with cumulative
killerCorpsKills; victimCorpsDeaths is retained notification data, not a write
to the other kill-score slot. Safe folding refuses corps indices outside0..1
instead of reproducing unchecked native array indexing. The whole raw packet
is retained in `last_kill` even when phase/identity gates refuse the team write.
`accepts_kill` exposes the pre-fold native main-notification guard.

INSTANT_GAME_KILL(table), event0x15d, contains native string keys `killerCorps`,
`killer`, `victimCorps`, `victim`, `killerKillstreak`, `killerCorpsKill`, and
`victimCorpsDeath`; corps values are localized corps names from retained
ruleset ID. Numeric table values use native i32?f32 conversion. Native local
streak-count callback precedes that table callback, including when the main
callback is refused. Killer-invalid sentinel naming remains separate from raw
ID preservation and is not inferred here.

The resource obtains actual local ID from received CharacterSelected and
PlayerSpawn state, never from the kill packet or ownership proximity. Match-only
reset (cancel, application error, received load) retains that selected identity;
full selection/disconnect/kick/leave-world resets it. Native3936d0d0 zeros
myKillStreakPoint and team result arrays; Joined3936d890 zeros cumulative score
and kill-score arrays but does **not** zero myKillStreakPoint. The fold preserves
that exact Joined distinction. No result team totals or victory state are
fabricated from kill notifications; they remain in genuine SC192 results.


### Typed proxy Pong receipt; clock synchronization boundary

Proxy0x13 now decodes to `game::ServerMsg::Pong(proxy::ProxyPong)` and forwards
`session::Event::Pong` instead of discarding its body. The exact40-byte payload
is `i64 tm,i64 when,i64 elapsed,i64 remote,u32 local,u32 world`. Native
CryNetwork constructor1626c606 sets opcode0x13/vtable1629b190; registration
1626bbff?16270d7e installs callback1626b6e6?1626af9b; serializer16267a47
uses packet offsets+8,+10,+18,+20,+28,+2c. UI-agent module/runtime proof is
independent of the active lane writer in `packets/proxy.rs::PongPacket`, whose
golden has tm1,when2,elapsed0,remote4000,local3,world4.

Native1626b0e1..1626b119 calls TimeKeeper::AddSample only when connection+2c
is nonzero, using received local, received world, and a freshly read timer0
local receive time. It does not use the four i64 fields for that sample. The
connection starts with+2c zero; the guard is a non-null listener pointer, with
the active Session event listener providing the corresponding client context.
Raw values, including signed i64 extremes and u32 wrap, remain preserved. The
separate ClockSample path below owns the proven filter and phase timing.

Lane Ping echoes tm/when/local. It generates elapsed0 and remote=world*1000,
where world comes from `packets/proxy.rs::tick_count`. Existing aa-net Ping
already sends Session elapsed milliseconds in both i64 fields and low32 local.
Crucially, lane `systems/instances/instant_game.rs::tick_count` has a **separate**
function-local OnceLock<Instant> origin from proxy tick_count. SC191 start/now
are generated in that separate domain. The original native shared remote-clock
assumption is therefore not established for the lane. Neither typed Pong nor
SC191 now is silently used to manufacture a common clock or completion.

Wire tests preserve all40bytes, reject every truncation, and leave a trailing
sentinel aligned. A Session test verifies exact forwarding without changing
Battlefield state or emitting any extra acknowledgement.


### Native TimeKeeper and backend receive-time clock samples

`ServerClock` (exported at crate root, Clone and a Bevy resource) implements
`apply(&Event)` and `mapped_now()->Option<u32>`. It returns None before any actual
Pong sample. Session captures a single fresh std::time::Instant and low32 elapsed
milliseconds at game receive dispatch, then emits
`Event::ClockSample(ClockSample { pong, local_receive_ms, received_at, local_origin })` before
raw Event::Pong. Native CryNetwork1626b0e1..b119 calls timer AddSample with the
same three inputs: echoed local send S, remote world R, fresh local receive L.
The i64 Pong fields are retained but not substituted for these inputs.

Native xlcommon33032040 computes signed wrapping RTT=L-S and f32 halfRTT. If
remote anchor is0 it stores R and fills all5 pairs with RTT and
CVTTSS2SI(f32(i32(L))-halfRTT), leaving ring index unchanged. Otherwise it writes
one pair at index, local=CVTTSS2SI(f32(i32(anchor-R+L))-halfRTT), then increments
index modulo5. Conversion reproduces x86 indefinite i32::MIN for out-of-range
values rather than Rust's saturating float cast. Remote0 consequently repeats
the initialization branch on the next sample, as in the original.

The exact MSVCR100 five-pointer qsort/shortsort63194a80?63194993 selects a maximum
using signed wrapping RTT subtraction, swaps it to the current tail, and repeats
for tail4,3,2,1. Equal candidates do not replace the maximum; tail swaps remain
unstable. This explicit five-entry implementation also reproduces nontransitive
wrapping comparisons without using Rust's total-order sort. Let m be sorted
RTT[2]. Squares are signed wrapping (RTT[i]-m)^2; threshold is signed truncating
(square0+square1+square3 wrapping)/5. The fifth square is deliberately omitted
from the threshold sum but still checked for inclusion. Include every pair whose
signed square<=threshold, sum local values with wrapping i32 arithmetic, and
signed-divide by count; count0 gives mean0. GetRemoteTime33025600 returns wrapping
localNow+anchor-mean. Native Init33032010 clears only anchor/index; constructor
330321d0 clears all storage. Both are independently covered.

`ServerClock` retains the actual backend local timer origin as well as the capture
Instant, so delayed UI delivery reads the same fresh timer without accumulating
a sub-millisecond re-anchoring error or measuring a new UI receipt. It resets on a
new game connection phase, failed connection, disconnect or kick; character
selection preserves the connection clock. The backend owns the same filter and
advances Battlefield before game messages/actions. WorldState and Bevy resources
fold samples and advance before battlefield updates; Bevy also advances on frames
without new events. There is no no-wire clock command, fabricated sample, or
SC191.now substitution. The separate active-lane proxy/instant-game timer-origin
mismatch remains a server boundary, despite this source-correct client mapping.

`Battlefield::apply_with_time(event, mapped_now)` retains matching SC191
`opening_at_ms=now+6000` and matching SC192 `ending_at_ms=now`; unknown clock
leaves these None. It preserves SC191's immediate now>=start branch independently
of the subsequent frame's now+6000>=start rule. Shared Session/WorldState/Bevy
folds use this API, and HUD replay can use it too. Native callbacks remain
WAIT() before any immediate START(); the resource does not emit invented wire
acknowledgements. Received Joined clears native timestamp slots. The captured
fields survive ordinary resource clones/HUD reconstruction.

Independent oracle artifacts are
`data/extract/aa-ui/native-timekeeper-oracle-20261018.json` and its helper, executing
original AddSample, comparator and actual MSVCR100 qsort locally in Unicorn.
Committed goldens cover all7 sequences: odd RTT/outlier, mixed threshold, f32
precision boundary, unsigned wrap, zero anchor, signed-square overflow/count0,
and nontransitive RTT. Additional tests cover delayed delivery, lifecycle,
WorldState/Bevy integration and genuine Pong -> backend phase -> empty Leave wire.


### SC196 unearned-win remaining-time notice

Native factory391b4350 sets opcode0x196/vtable399b3e28, whose serializer397afb20
reads one u32 at packet+c. Handler391de900 calls3936d4f0, which computes unsigned
quotient milliseconds/1000 and increments when remainder>499, then emits
INSTANT_GAME_UNEARNED_WIN_REMAIN_TIME(seconds as f32), event0x162. There is no
phase/identity guard, persistent native countdown write, or phase transition.
The lane `SCInstantGameRemainTimePacket` writes the same u32 using its
SCInstantGameUnkPacket0x196 constant, but its server code never emits it.

`BattlefieldEvent::UnearnedWinRemaining { milliseconds }` preserves every genuine
notice; `last_remaining_time_ms` retains the latest raw value without decrementing
it. `unearned_win_remaining_seconds` implements the exact half-up conversion for
UI callbacks. Initial state is None and lifecycle reset clears the receipt. Wire
goldens cover u32 extremes, all truncations and DD04 trailing entry alignment;
rounding tests cover both sides of499/500 and1499/1500. No simulated countdown,
completion acknowledgement, or lane transaction is introduced.


### Complete original ScheduledEventStarted and battlefield ratio producer

SCScheduledEventStarted0x1ec is source-proven by native factory391b6e90 (packet
vtable399b4270), serializer397b01f0, shared descriptor serializers398a7ed0 and
398a7d80. The lane defines this opcode only; no writer/handler implementation
exists there. The default decoder uses this explicitly named original-source
reader, not a guessed lane layout or payload profile heuristic.

The complete wire order is u32 id;21 i32 values;u32 changes;u8 priority,u8
startDate,u8 periodDate,u8 hour,u8 min;i32 periodMin;u8 weekDay;length-prefixed
openMsg bytes. The fixed prefix before the message length is102 bytes. Native
expands low21 changes bits into booleans, but `ScheduledEventStarted` retains all
raw bits. Native field-name literals are id3999d470, v399d8ee8,
changes399e79c4, priority39a20504, startDate3999fc70, periodDate39a204f8,
hour3999fc8c, min399e4cb0, periodMin39a204ec, weekDay39a204e4,
openMsg39a12934. No date/priority policy is inferred from these raw fields.

Native openMsg read requests a127-byte destination plus terminator
(397b0244..024e). The decoder applies this supported byte bound before publishing
a receipt; rejection of overlong input is a safety boundary, not a claim about
native malformed-input recovery. The normal signed-i16 blob framing is retained;
raw message bytes, including NUL/non-UTF8, are preserved. `open_message_text()`
uses the crate's existing lossy UTF8/NUL-trim convention only on demand. Tests
cover126/127/128-byte boundaries independently of text character counts.

Native handler391e1e10 copies the whole descriptor into the player scheduled
manager through3930b900, then unconditionally passes packet+4c to3936cec0.
That field is descriptor.values[10] (descriptor base packet+c, array base+18).
3936cec0 stores it directly into battlefield manager+3cd4. Consequently
`Battlefield.my_corps_kill_death_ratio` is an i32 initial/reset0 and updates from
every genuine complete ScheduledEventStarted receipt, regardless of phase,
changes-bit10, event ID, or priority. Joined does not reset it; match reset does.
No local kill/score arithmetic is substituted for this received ratio.

`ScheduledEvents.starts` retains the latest complete raw start per ID and is
available in WorldState/Bevy. It is explicitly receipt history; the native
scheduled-manager priority/active merge policy is not invented. Its record can
survive a match-only LoadInstance reset while the separate battlefield ratio
resets to0; callers must read the actual battlefield fold rather than replaying
an old record to manufacture a ratio update. Selection/kick/leave/disconnect
clear receipt history. Native optional open-message event0x256 is separately
gated by global3a5a4db8; no unconditional Lua announcement is fabricated here.

Goldens cover every field, high change bits, negative values/raw date bytes,
all truncations, DD04 trailing-packet alignment, raw message bounds and lifecycle.
WorldState/Bevy tests prove the complete record and ratio reach their resources.
Source artifact: `data/extract/aa-net/battlefield-native/scheduled-event-20261018.txt`.


### Ordinary SkillAccessor / battlefield slot244 planner boundary

UI source has closed slot244 to raw SkillAccessor kind15, requiring tag204 and
received first-matching skill enable. It invokes the ordinary path
3920a220/3920a170?39209d80?393989e0; this is not a special direct packet sender.
The ordinary display CanUse39209a70 is protected/obfuscated. No ready default or
plain UseSkill/UseSkillAt replacement is established by slot enable alone.

39209d80 requires the current actor at client39b02294+10c. It builds initial
target-kind0/objectID from selected-target property4 at actor+1aa0, with a
fallback kind4/raw ID from actor+5f90 when the former is the invalid sentinel.
Input modifiers are masked with0x44; any remaining bit replaces the initial
target with caster self/objectID and sets options byte+1. The mask is retained
as raw source evidence; it is not renamed Alt/Shift without input-enum proof.
Raw accessor kind18 has separate invocation guards, while battlefield kind15
uses the normal branch. Accessor byte+17 can refuse invocation through39016c40;
its complete producer meaning is not guessed from a visual enable state.

393989e0 looks up the combat skill descriptor396f6390, copies the caller target,
and unless options+1 forces it, dispatches descriptor.target_type_id:1 through
393910e0,4 through39393a20,5 through39391090. The simple type5 helper only fills
an invalid unit target with self when CVar3a1776c0 exists/GetIVal>0; it converts
kind4 to an invalid kind0 before that fill. Type1 additionally checks the
selected unit's raw relation mask through39028240, so not every selected unit
survives unchanged. Hostile auto-selection uses the protected type4 helper and
39391170; companion CVar3a1776c4 is named auto_enemy_targeting by static row
39ac44d8/44dc. These are conditional planner steps, not blanket self/target casts.

Next,393925c0 can replace the skill ID and caller caster metadata from an equipped
item. Descriptor+e8 is skills.link_equip_slot_id and+e4 is link_backpack_type_id:
SQL39a06a90 columns67/66, loader39731fc0..39731fdc. Index-1 bypasses that entire
substitution. The current read-only compact tag204 catalog has36 skills, all
link_equip_slot_id=-1;32 have target_type0, three have type6 (21010,12614,22527),
and one has type4 (21558). This closes the item-link bypass for those catalog
rows, but does not establish generic target planning or permission to cast.
Native target_type+1b0 mapping remains separately proven by397327a6..397327bd.

The final shared planner39396e20 returns a structured error/result, used by
393989e0 for local error reporting39390900 and conditional model-action retry
39393920 (including result0x0f with its additional CVar/controller gates).
Knowing the initial target does not prove its final target/caster/skill-object
or when it sends. The lane's existing CSStartSkill0x052 codec is distinct from
this missing local planner. No new command, speculative packet route, movement
retry, completion, or always-success callback is introduced by this audit.

A bounded read-only live source check verified StandMode at399a08dc in original
PID48924 before reading: entry stubs match the static DLL, but VM sections at
3ac11fde/3ab63fb7/3a6775ec contain decrypted threaded x86 rather than static
ciphertext. This is useful source evidence, not foreign-process execution.
One local Unicorn attempt with copied source pages and a fake local packet
buffer stopped at100000 instructions/69 pages in protector setup; it did not
return or reach the trapped native sender and provides no constructor/packet
route oracle. That boundary is explicit; unbounded VM exploration is deferred.

Artifacts under `data/extract/aa-net/skill-planner-native/` include the exact
entry map, read-only tag204 rows, module-verified runtime snippets and bounded
local-attempt result/unique instruction record. No original process input,
control, injection, write, network action or live lane transaction was performed.


### Shared native formula recursion and pet detail audit (2026-10-18)

The common formula interpreter is shared by owner5 unit formulas, wearable formulas and
holdable formulas. Its24-entry dispatch table at3a5bd4c0 was read through the verified
original module using PROCESS_VM_READ only; the captured table is
`data/extract/aa-net/mate-stats-native/formula-dispatch-runtime.json`.
Node0 (39566d00) loads a literal f32 at node+8. Node1 (39567fb0) loads a context f32
at context+10+index*4, accepting variable indices0..34. ADD395676c0, SUB39567740,
MUL395677c0, DIV39567840 and POW395678d0 explicitly spill EACH evaluated child to
f32 before combining it. Their own output remains x87 until a parent or caller spills
it. Thus a whole-expression f64 evaluator does not reproduce the original recursion.
NEG39567a70 also narrows its child before FCHS. FLOOR39567e70 narrows its child,
reloads/spills it as f64 and calls MSVCR100 floor through398b57c4/IAT38ff6814.
POW uses _CIpow398b57ca/IAT38ff681c with the narrowed operands. Division's zero
denominator path reports a local error and returns FLDZ, rather than infinity; its
UCOMISS/LAHF parity branch admits unordered NaN to arithmetic. This
bounded audit does not define all24 node kinds or authorize guessing unsupported ones.

`equipment_formula_oracle.py` / `.json` under that directory execute the ORIGINAL
397cc440/4c0/6d0/750 wrappers,39568560/68610 dispatch and native recursive operators
locally in Unicorn, using AST nodes and grade fields reconstructed from real read-only
DB records. Static DB/basis-point lookup returns are mocked. External CRT _CIpow uses
Python libm double and floor returns an integral double; those CRT bodies are NOT
executed. Formula results and recursive operator arithmetic are not mocked. These
limitations are distinct from a fully executed CRT oracle. Raw grades0/1/6 and positive
117-percent overrides are retained in the JSON. Representative no-override outputs:

| Item | Quantity | Grade0 | Grade1 | Grade6 |
|---|---|---:|---:|---:|
|31432|wearable armor|647|517|809|
|19326|wearable armor|109|87|136|
|19326|wearable resistance|547|437|683|
|28905|holdable DPS|389.04461669921875|311.23569799714824|486.30577087402344|
|18420|holdable armor|747|598|934|

The independent owner5 level5/STR26 AST run gives kind21 x87 result spilled to double
29200.000762939453, then mandatory unit-formula f32=29200 and i32=29200. Kind20 gives
401032.8369140625, then f32=401032.84375 and i32=401032. These outputs distinguish
recursive f32 inputs, an extended final expression and the caller's final f32 narrowing.

Generic/Mate BASE attributes10 and77 both select398b526d in398b4e80, then the default
398a10ee in398a0d60, which returns zero. Their FINAL attributes may still include all
modifier pools. This zero-base proof does not justify returning final zero.

GetSpeedInfo registration3906b899 selects3906ac40. It resolves the current owned mate,
its object ID+20, actor39360020 and controller390b3750 (actor+3be8). The controller must
exist and virtual+4 return1; its virtual+204 must return a component, whose virtual+74
returns the speed float. The wrapper spills f32 and390699b0 returns ONE numeric Lua
value (ScriptAnyValue type4). A missing mate/actor/controller/component returns NO Lua
values. There is no zero fallback or table in this getter. Concrete controller vtable
399a4f68 has virtual+4=390e54d0 and+204=390e5510 (returns controller+1f0).
Candidate component vtables399ac3d8/399ac698/399ac918 have+74=391138b0; the exact
controller+1f0 producer/class mapping remains open, so its formula is not yet claimed
as the complete getter contract.391138b0 multiplies motion-state scale by a clamped
`1 + FINAL(attribute10)*0.001f` factor, and has additional conditional attribute66/139
and status-scale branches. The0.001f constant3999cf30 differs from0.01f3999cf90.
Do not replace this runtime movement component with a static NPC speed or attribute10.
Exact excerpts are retained in `speed-and-base-attributes-20261018.txt`.

Tooltip modifier394bb740 has its own precision chain: baseline SSE converts B, static
flat and percent to f32, then ADDSS, MULSS percent, MULSS f32(0.01), storing baselinef32.
After collecting buff/gear T, the helper returns exact FLDZ if Tflat==0 and Tpercent==100.
Otherwise its x87 chain is `((baseline + Tflat) * Tpercent) * f32(0.01) - baseline`,
retaining x87 precision until caller storage. It does not divide by100 and does not
subtract static baseline from the already-clamped final A attribute. This formula
is independent of the generated-primary3989fa80 chain recorded earlier.

`tooltip_modifier_oracle.py` / `.json` independently execute394bb740 itself with explicit
mocked B/S/T source lookups and identity gear pairs; NONE of its arithmetic is mocked.
Seven fixtures include exact no-modifier shortcut and large-integer f32 boundaries.
B29200,S(3,101) gives baseline29495.029296875; T(1,100) returns x87-spilled-double
0.9993407123693032 (f32=0.9993407130241394), while T(0,101) gives294.94962711081826
(f32=294.9496154785156). T(0,100) returns exact zero regardless of nonidentity S.
B401032,S(11,127),T(-37,88) yields -61151.52126744902 (f32=-61151.51953125).


### Pet equipment cursor and confirmation follow-up (2026-10-18)

Original pet_view.lua creates ordinary CreateItemIconButton widgets, not an established
native equipment-accessor slot. Authored physical Lua slots are1/3/4/7. The actual
IsMateEquipableSlot3946ef30 first requires resolved actor kind1 and3933b1f0's kind5 /
local stored-owner equality.3933a9c0 resolves NPC descriptor+70; SQL39a00298 and loader
396ce710 identify mate_equip_slot_packs(chest,feet,head,name,waist), represented as
bytes0/1/2, name pointer4 and waistbyte8. Raw slots0/2/3/6 select head/chest/waist/feet;
missing pack and every other slot return false.

GetEquippedItemTooltipText3946f1b0 requires a resolved kind1 actor but does not require
owned Mate for the read. It formats actual physical item at actor+3f8+LuaSlot*c0
(=actor+4b8+(LuaSlot-1)*c0) through39423560 and returns its table. Invalid unit returns
no values; a valid empty equipment item still goes through the formatter. Packed
location defaults to the global invalid location for nonlocal actors; only the local
player branch builds container1. Do not label a mate tooltip as player-worn or infer
its formatter input location is containerfc. The pet UI uses itemType0 to show empty
slot help and refreshes on UNIT_NPC_EQUIPMENT_CHANGED.

PickupMateEquippedItem3946e950 returns no Lua values. Empty cursor is raw mode0 at
3a5a4e68. It invokes39429460/394293b0 with current mate physical item and packedfc
location; nonstack equipment394292f0 creates cursor mode4, copies item database ID
low/high into cursor+8/+c, source location+10 and count+14. This is local cursor state,
not item removal. Item-like modes1/4 are accepted by39424550; dropping onto a pet slot
requires their source container2 and routes to394eb6c0, then clears the cursor. Current
mate+bc is a pending-action latch, not a dead flag. MateUnequipItem3946f710 chooses
the first ascending empty physical BAG2 slot via397d1920/19d0, excluding packed pending
keys; none emits error19 and BOOLfalse. Otherwise it invokes the shared planner and
returns BOOLtrue even if that downstream planner refuses. The target bag slot is not
invented as zero or an overflow slot.

The shared planner has TWO distinct dialogs. Static item field+10==4/5 with received
flags bit0x10 clear creates task47 ITEM_UNPACK, vtable399e62e8; accepted394ea8e0 sends
CS134 CSThisTimeUnpackItemPacket, not equipment0a9. Original DialogUnpackItemHandler
uses isSoulBound from received flags bit0. The lane registers134 in systems/economy/item_handlers.rs77 and implements
CSThisTimeUnpackItemPacket::read_body/unwrap_item at525+. It validates actual owner,
item DBID and exact container/slot, then reports reason114 action11; a wider source
search corrected the initial opcode-only conclusion. Unpack is separate from equip. Otherwise397cc300 can create
task16 SOUL_BIND_ITEM(vtable399e6268); accepted394ea430 invokes394e8b30 with bts=true
(or394e88e0 for the invalid-owner sentinel branch). Rejection sends neither request.
Both constructors mark a captured packed slot pending394e8590; completion clears it
394e8520 before accepting/rejecting. Their preflight re-reads CURRENT item at captured
slot and refuses missing/empty items, or397cc350's itemflag4 AND playerflag0x2000
(error298). It does NOT compare captured database IDs with current items. No native
stale-item identity guarantee is claimed for these dialogs. Actual0a9 sender separately
rechecks owner, actor+3a76/current mate pending, source/target pending and eligibility.
Its existing public codec does not reproduce those UI/planner gates.

The actual pet-to-bag destination route is HandleClick, NOT PickupBagFromSlot3944cb90
(the latter excludesfc). Original sort_inventory.lua OnDragReceive requires nonnil
cursor then calls adapter:HandleClick(virtualSlot). Native3944c760 decrements the Lua
virtual index, calls bagmanager[3a5ad900] virtual50 ->394dc7e0 ->selected subbag virtual20.
A module-verified PROCESS_VM_READ captured main bag vtable399baa60, click391ff2f0 and
SlotByIdx391f2da0; bagmanager vtable399e5a70. Main bag maps raw virtual index through
subbag+8+index*4 to the actual packed physical destination, reads its current item,
checks its pending key, then dispatches391f46c0. Cursor containerfc at391f47ca resolves
playerpet with the owned-Mate guard, falls back to current mate object+20 if absent,
then calls394eb6c0(mateObject,carriedFcSource,actualBagDestination,currentBagItem) and
clears the cursor. It does NOT choose a first empty slot; right-click unequip does.
Preceding main-bag action-mode391f2a70, quantity/dialog391f4560 and391f3480 predicates
remain separate source gates and are not reconstructed as always true. No extra packet
was invented. Exact cursor/dialog excerpts and read-only helper are in
`data/extract/aa-net/mate-equipment-native/`.


### Received Family roster infrastructure (2026-10-18)

`family::{FamilyEvent,FamilyRoster,FamilyMember,Families}` adds received-only SC02d..037.
Event::Family forwards every genuine receipt unchanged, including duplicates and unknown
family/member IDs; no request, invitation acceptance or optimistic roster update exists.
Families.rosters is keyed by family ID, matching original global manager392ff2d0 and
398a8df0's keyed full-descriptor copy. It is NOT a guessed single local family. Full
Created/Described/MemberAdded rosters replace their matching key; added_index remains
raw in the event. Known member updates change only that matching family/member. Remove
and full-family removal remain separate actual receipts. Native398a8860 owner change
sets the matching row role1, every other row role0, and clears the new owner's title;
an absent new-owner row still clears old roles. No missing member is manufactured.
last_invitation/last_title retain notices, not invented active-dialog/expiry state.
Selection, Kicked, LeftWorld and Disconnected clear all session-owned family state.
WorldState and Bevy NetState expose/fold the same resource; Session only forwards wire.

Active-lane source is systems/social/packets/family.rs and family.rs::FamilyInfo::write:
u32 family ID, signed i32 member count, then each u32 character ID, string name, u8 raw
role, bool online, string title. Strings retain exact blob bytes (including non-UTF8 and
NUL); text conversion helpers are explicit presentation conveniences. Negative and
payload-impossible member counts are rejected BEFORE allocation; framing has no invented
8-row clamp. Native398a85b0 clamps its inline array to8, with name128/title104 byte
read limits. That unsafe leftover-wire behavior is not imitated by the lane decoder.
Complete lane rows and long raw text survive; later native UI visibility can apply its
source cap separately. Login emits FamilyDesc via social/session.rs and family login.
No live family transaction/session was performed against the stalled game lane.

Native factory/serializer pairs were independently traced:

|SC opcode|Packet|Factory header|Serializer|
|---|---|---|---|
|02d|Invitation|391a886e|397c5810|
|02e|Created|391a88ee|397ab110 ->398a85b0|
|02f|Removed|391a896b|397ae900|
|030|Desc|391a89ee|397ab130 ->398a85b0|
|031|MemberAdded|391a8a6e|397ae920|
|032|MemberRemoved|391a8aeb|397c5910|
|033|OwnerChanged|391a8b6b|397c5980|
|034|TitleChanged|391a8bee|397c59e0|
|035|MemberNameChanged|391a8c6e|397c5a90|
|036|Title|391a8cee|397c5b40|
|037|MemberOnline|391a8d6b|397b2f00|

All field ordering agrees with the lane writers:032 writes familyID,memberID,kicked;
036 uses BC unit ID, raw role byte, owner string and title string;031 appends signed
added_index AFTER the complete roster. Lane notes035/036 are never emitted by its
original1.2 source flow, but their typed receives preserve genuine traffic if supplied.
Native handlers391d2a60/2b60/2ae0 apply Created/Desc/MemberAdded snapshots;398a8900
and398a8990 require a matching cached family/member for title/name changes. Source
artifact `data/extract/aa-net/family-native/serializers-20261018.txt` preserves the
complete serializer excerpts. No family/community mutation API is implied by this
received-only milestone; authored X2Family getters/actions/events still need root UI
integration and their separate native contracts.

Validation: seven new Family tests cover all eleven lane packet layouts, every field
truncation, impossible/negative counts, raw roles/NUL/non-UTF8 text, a nine-member roster
followed by an actual DD04 packet, native owner/title changes, wrong family/member IDs,
repeated Session forwarding, both WorldState/Bevy folds and lifecycle resets. Full
`cargo test -p aa-net --features bevy --lib`:374 passed; own feature check passed.


### Guarded unpack request and received security fields (2026-10-18)

UnpackItemRequest::new(container,rawSlot,itemDatabaseID) rejects zero identity and
encodes original CS134 as EXACTLY u8 container,u8 slot,u64 item ID. Fields have read-only
accessors; no caller can mutate a validated ID. Native task47 constructor394ea7c0
captures packed source location, item DBID low/high and isSoulBound for presentation;
callback394ea8e0 clears the dialog pending key and sends134 only on acceptance.
Serializer397bc950 reads packedlocation byte+0d and byte+0f, then item ID at+10.
The request contains NO mate, target equipment slot or second item. Neither native
callback nor server unpack flow can automatically continue a remembered equipment
move from these fields. No automatic0a9 is sent by aa-net.

Command::UnpackItem rechecks the CURRENT received owned inventory identity/location
at socket-write time. Session now mirrors Inventory from genuine events for this check;
selection/leave/disconnect and Kicked discard that mirror. Its physical bag/bank/player
equipment lookup never creates items. Unknown containers, changed DBID/location, missing
items and inactive selection reject locally. This exact-ID app preflight is stronger
than native task47's captured-slot-only preflight and is explicitly not native proof.
The encoder still preserves any u8 wire location; stronger admitted-location checks
belong to the app/session. UnpackItemRequested means a successful transport write;
UnpackItemRequestRejected includes inactive, stale, missing-connection and failed-write
paths. Neither is an ACK, reservation, flags update or local equipment change.

Actual lane unwrap_item sets unpack_time from the server clock, ORs UNPACKED0x10 and
also SOUL_BOUND0x01 for BIND_ON_UNPACK, then emits SCItemTaskSuccess reason114 with
ItemUpdateSecurity action11. Optional received ItemLifespan remains a separate existing
notice. The previous reader collapsed actions10/11 into Flags and discarded action11's
three native bool bytes and both timestamps. Additive ItemTask::Security now preserves
container/slot/DBID/bits, raw u8 is_unsecure_excess/is_unsecure_set/is_unpack, signed i64
unsecure_time/unpack_time. Only an exact EXISTING owned physical item receives the full
flags and both timestamps; unknown/moved/reused identities do not manufacture slots or
items. All three raw flag bytes remain in the receipt without normalization.

Native action11 dispatch397d1168->397cdd00 unconditionally assigns flags at+d and both
i64 times at+b0/+b8, independent of those bool bytes. It also has an optional is_unpack
nonzero/detailkind1/catalog397cd100 charge-time path; catalog-dependent EquipDetail
charge_time and its retained payload are NOT guessed by this timestamp milestone.
Native action10 dispatch397d114b->397cdc90 ORs bits at397cdce7, rather than replacing
all flags. The existing Flags fold is corrected to OR; Security does full replacement.
The source flag/time bytes remain the only authority for the unpack result.

Lane mate binding has a separate limitation: change_mate_equipment reads/logs bts but
does not use it to bind; Container::apply_bind_rules_task binds BIND_ON_EQUIP only for
player EQUIPMENT1, excluding MateFC. Thus task16 acceptance/0a9 bts=true is not evidence
of a received soul-bound pet item. No server change or fabricated flag repairs this.
Native397cc300 with an unbound item returns true for static bindtypes2/5, the caller's
isEquip for3, and caller's final flag for4; the shared equipment planner passes1/0.
Unpack-type4/5 handling occurs before that check when received UNPACKED is clear.

Validation: six additional tests verify native/lane CS134 golden bytes, every action11
truncation, raw noncanonical bool bytes and signed times, following DD04 alignment,
action10 OR versus action11 replacement, pending unlock, missing/reused/moved items
without vector growth, WorldState/Bevy folding, and Session loopback stale/current
identity plus missing/failed writes. Actual unpack receipts update timestamps without
sending equipment continuation. `cargo test -p aa-net --features bevy --lib`:380 passed;
feature check passed. Exact native excerpts are retained in
`data/extract/aa-net/mate-equipment-native/unpack-security-source-20261018.txt`.

### SkillFired final wire byte (2026-10-18)

`SkillFired.raw_final_flag` preserves the final u8 after `fire_anim_id`, including noncanonical nonzero values. The active lane `systems/skills/packets.rs::SCSkillFiredPacket::write` writes zero; the preceding byte is the variable SkillResult header described below. Native serializer397ca4aa..bd decodes final-wire bit1 into packet+160 (bit0 goes to+14c); handler391d58dd passes the decoded bit1 as the final argument to3939b010. Nonzero decoded bit1 at3939b35b bypasses one ordinary own/tag cooldown insertion path39368f90. Raw byte1 does not suppress; raw byte2 does. Zero alone does not imply insertion: native also checks toggle_buff_id and timing_id. This field therefore has a neutral raw name and does not generate estimated cooldowns. Source: `data/extract/aa-ui/native-cooldown-fired-body-20261018.txt` and `native-cooldown-started-fired-20261018.txt`. Tests preserve 1/2/255, reject every truncated prefix, and retain nonzero0x82 followed by three other packets in a DD04 batch.

### Family member removal cache correction (2026-10-18)

Native398a8d40 calls398a8160 to remove only the first matching persistent character ID and compact the ordered roster. After an actual removal, a resulting count below two clears the remaining member's family association and deletes that keyed roster (398a8da1..c5). Unknown family/member is a no-op, including an already received singleton. `Families` now follows this cache lifetime and first-match behavior without truncating the received lane roster or creating Unit identity state. Regression coverage includes duplicate IDs with distinct names, preserved survivor order, two-to-one dissolution, and unknown-member singleton retention. UI source: `data/extract/aa-ui/native-family-cache-callback-source-20261018.txt` and `native-family-unit-id-callbacks-20261018.txt`.

### Family request controls: native/lane boundary (2026-10-18)

`FamilyCommand` supplies Invite `{name,title}`, Reply `{inviter_id,join,role}`, Leave, Kick `{member_id}`, ChangeTitle `{member_id,title}`, and ChangeOwner `{member_id}`. Active lane `systems/social/packets/family.rs` registers all six handlers; CS opcodes1a..1f respectively carry string+string, u32+bool+string, empty, u32, u32+string, u32. Native packet vtables399cd2f4/399cd350/399cd300/399cd30c/399cd318/399cd324 select serializers397b0610/397c7340/397ab5e0/397c73f0/397c7440/397b9310 with this same order. Reply constructor stores role before join in memory, but serializer emits inviter ID, join, then role. Exact copied disassembly: `data/extract/aa-net/family-native/command-serializers-20261018.txt`.

The protocol API preserves binary name/title/role bytes without NUL trimming or UTF-8 conversion. `validate` rejects zero persistent target/inviter IDs and oversized name128/title104/role104 byte strings, an explicit app preflight based on native constructors'129/105 byte destination capacities; it does not claim native Lua rejects all these inputs. Native397ab560/397ab5a0/397ab600 copy C strings, and serializer uses strlen. App Lua integration therefore owns first-NUL lookup/copy behavior. Lane readers convert strings to their existing UTF-8 String representation; no normalization is added in aa-net.

`Command::Family { character_id, command }` captures the persistent acting character ID locally. Session rejects inactive, zero, or changed selected identity immediately before writing, validates payload bounds, and emits `FamilyRequested` only on an actual successful write. Inactive/stale/missing/failed transport emits `FamilyRequestRejected`; neither event changes `Families` or creates invitations/ACKs. The captured acting ID is never included in the wire payload. UI-level cached role/name/member/dialog identity checks remain app-owned: native Invite392ff770 permits familyID0 or a nonzero own cached role, Leave392ff8b0 requires familyID!=0, Kick392ffa90 and Title392ffb60 resolve a persistent member ID by strcmp in the own cached roster and require own role!=0, and Owner392ffc30 additionally rejects self. Source details: UI `native-family-command-eligibility-20261018.txt` and `native-family-controls-final-20261018.txt`. Native invitation task4 vtable399cd330 captures inviter ID, name and title; callback392ff590 sends captured ID/title plus the dialog result through397ab5a0. Net never automatically accepts an invitation.

Tests cover all six native/lane byte layouts, both reply booleans, embedded NUL/non-UTF8 strings, exact bounds, zero IDs, and a real local TCP Session test rejecting queued old-character requests and invalid payloads without writing. Successful Leave is only a request; failed writes cannot emit success. No live social transaction is used.

### Received GM mode bytes and legacy snapshot projection (2026-10-18)

SC203 `UnitGmModeChanged` retains BC runtime object ID, signed i32 mode, and raw u8 value. Lane `packets/g2c_misc.rs::SCUnitGmModeChangedPacket` writes exactly those fields. Native handler39186420 registration factory391b7b50/vtable399b6144 identifies opcode203; serializer397c7270 uses397b27d0 for BC ID, packet+10 mode and+14 value. Native writes raw actor[3af8+mode]; the safe state fold bounds mutation to indices0..3, preserving every unknown signed mode as a received event. Noncanonical byte values such as255 remain exact.

`Unit.gm_modes:[u8;4]` preserves its legacy little-endian FIRST common-integer projection immediately after raceGender; this is NOT proof of effective native GM hydration. Native397c9882/397b21d0 decodes that pack to UnitState+2020/+2024/+2028/+202c, and reverse397c9a81..ad8 emits pish; lane PISC builders currently emit first integer0, third appellation. The earlier hydration inference missed the raw actor+8 destination: UnitState+2020 lands at raw actor+3b00, whereas SC203 writes raw actor+3af8+mode. Raw API/bytes and lane framing remain unchanged. Effective cooldown mode1 is known only from matching SC203; native actor-constructor GM defaults remain unproved.

Player and Units folds update only a matching existing runtime unit, never create an actor from SC203. Full snapshots replace old bytes; removal/lifecycle removes the owning state. Session forwards repeated/unknown reports without synthetic changes or responses. Mode1 is source-used as a cooldown insertion suppression byte: the cooldown manager owner pointer is RAWACTOR+8, so its owner+3af1 test refers to raw actor+3af9. This milestone retains the producer only and adds no cooldown timing/sidecar. UI source closure distinguishes these bytes from attachment IDs and model flags. Tests cover all modes0..3, unknown negative/out-of-range indices, raw255, each truncation, following DD04 packet alignment, snapshot word byte order, independent u16flags/appellation, local Player+Units mutation, missing/removed/reused objects, duplicate Session reports, and Bevy resource lifecycle.

### Native received stop-duration cooldown fold (2026-10-18)

`Skills.apply` now consumes the actual nonzero duration in `CastingStopped` and `PlotStopped` before ending the tracked cast. Source391d5cf0?39396000 and391d5af0/391d5b70?3938c410/3938d970 resolve the tracked skill/actor and invoke39899f90 before teardown. This path does not evaluate A74, database cooldown lengths or Started/Fired timing. `set_cooldown_catalog(HashMap<u32,u32>)` accepts `SELECT id,cooldown_tag_id FROM skills`; present zero routes to a direct skill row, nonzero to the primary tag, and a missing catalog entry stays unknown. The catalog survives PlayerSpawn, selection, disconnect, kick and leave resets; dynamic timer/context state does not.

The existing `received_skill_cooldowns`, `received_tag_cooldowns` and `received_cooldown[_at]` remain compatible original SCCooldowns views. Separate ordered `native_skill_cooldowns`/`native_tag_cooldowns` in the same Skills resource hold mutable manager rows with origin Snapshot, CastingStopped{tl} or PlotStopped{tl}; snapshots seed both views, stop extensions never rewrite raw receipt rows. `native_cooldown[_at](skill,tags)` selects tags in supplied order then direct rows, greatest remaining with strict greater replacement, preserving first ties/total0. Timers compare elapsed whole monotonic milliseconds. Native39899f90 extends only the FIRST matching ID when the supplied duration is strictly larger, replacing both remaining and total; expired/reset matching rows are reused in place. A missing ID at capacity150 compacts zero-remaining rows in original order, then appends only if space exists. Genuine reset reports affect the first matching runtime row as well as the original receipt view.

Explicit conservative boundary: current cast storage is keyed by actor, not the native timeline tree. Insertion requires exactly one tracked matching timeline, owned by current local object ID; ambiguous/missing/foreign timelines produce no timer. This does not claim native first-TL traversal parity or a complete remote actor cooldown manager. Native GM suppression byte1 must be source-known zero from a matching SC203 report; PlayerSpawn starts unknown regardless of its legacy gm_modes bytes, and unknown/nonzero blocks insertion. Local object removal invalidates that context; subsequent genuine actor context and SC203 are required. Zero duration is a no-op for timers; existing cast teardown remains unchanged. Every genuine raw stop event remains available to app consumers even if no timer is admitted.

Independent original x86 outputs are embedded in regressions: first duplicate5000/10000 extended to7000/7000 leaves later9000/11000 unchanged; first8000/10000 is not replaced by7000; first expired row is reused250/250; full150-row cache compacts to survivors100/150/200 then appends17, while all-active capacity rejects. Oracle source: UI `native-cooldown-producer-oracle-20261018.json` (original39899f90 executed locally with only memory/logging/cookie/actor-query stubs). Additional tests exercise actual apply order, raw-history preservation, source-hydrated suppression, ambiguity/foreign/missing catalog, plot primary-tag routing, resets, lifecycle and WorldState propagation. Started/Fired GCD/ordinary cooldown production and skill-specific modifiers remain separate integration work.

### Family feature gate and cooldown A74 prerequisites (read-only closure)

Family native common game+14 mask2 is FeatureSet byte0 bit1, not InWorld. GetFeatureSet39496b67 uses that same feature base; lane Feature::AllowFamilyChanges=1 is explicitly enabled by initialized(). Invite39474840, Leave39474870, Kick394748a0 and Title394748d0 check it; ChangeOwner394747e0 and Task4 Reply392ff590 do not. GetMembers/Count/IsOwner/IsMyFamily/IsFamily have no such gate. GetMembers adapter39474d80 requests/validates ScriptAnyValue type2; shared GetParamAny numeric coercion remains untraced. With a valid bool, false omits offline rows and true includes all, densely indexed [name,title,role-as-f32,online]. Count returns online,total. OwnerChanged398a8860 deliberately marks ALL duplicate matching IDs owner and clears their titles, returning the LAST matching name; this differs from first-match removal. Option74 invitation admission and automatic false reply on rejected task are closed in UI native-family-invitation-closure-20261018.txt. Source summary: `data/extract/aa-net/native-family-feature-summary-20261018.md`.

A74 base is exactly0 through Character/generic/Npc/Mate dispatch, but final398a0bc0 still aggregates static template, passive/static, runtime Buff and equipment providers, then395729f0 clamps to catalog unit_attribute_limits (current A74[-800,4000]). Both attribute and cooldown managers are bound to owner rawActor+8 and provider rawActor+3e80 (3936621f..37/3936b13a..5b); module-verified provider vtable399d1990 virtual24=39387a50 enumerates three runtime Buff record lists. Separate passive collection3989e5c0(selector1) resolves learned PASSIVE IDs via3976ada0, accepts active=false, maps buff_id to BuffDesc, then callback398a0a00 uses stack1/level0. Loader3979ee53 SQL selects id,ability_id,active,buff_id,level,req_points. Current A74 passive IDs106/107/157/178 map buffs3063/3064/3506/3680; 3506's linear24 does not imply player-level scaling on this passive path. No racial character_buffs direct A74 rows or conditional BuffUnitModifier A74 rows exist in this catalog;20 dynamic A74 rows do. Absence of active buffs alone therefore does not prove A74zero. Source/catalog/runtime artifacts: `data/extract/aa-net/native-cooldown-a74-*` and `native-cooldown-provider-runtime-20261018.json`. No A74 approximation is introduced by the stop-duration fold.


### Expedition received foundation (2026-10-18)

`Event::Expedition` / `Expeditions` now retain the active lane's faction metadata (`SC008`), role-policy lists (`SC00b`), member-list chunks (`SC016`) and member-status receipts (`SC017`). The resource is public through the Bevy plugin and `WorldState`; Session forwards every genuine receipt, including repeats and empty lists. No commands, own-expedition inference or local success events are added.

Framing is source-labelled `read_lane`: faction/policy lists begin with u8 count; member lists are total u32, count u8, context expedition ID u32, then rows; status is one complete row then raw flag u8. Faction fields are ID u32, raw aggro byte, mother ID u32, binary name, owner ID u32, binary owner name, raw signed owner kind byte, political-system byte, created i64, raw diplomacy byte, allow-name byte. Role-policy rows are expedition ID u32, role u8, binary name, nine raw permission bytes in order dominion-declare/invite/expel/promote/dismiss/chat/manager-chat/siege-master/join-siege.

Member rows preserve expedition ID and persistent character ID (u32 each), raw party/online bytes, last-world-leave i64, binary name, level u8, zone and unnamed `id3` u32, three ability bytes, role byte, two raw fixed-position i64 coordinates and f32 height, binary memo, transfer-request time i64. No lossy text conversion, coordinate re-encoding, canonicalization of boolean bytes, sorting or deduplication is applied to the received batches. Names/memo can contain NUL or invalid UTF-8. Native397bd700 has the same field order, with inline name buffer129 and memo64; native397bd2a0 faction and397bd570 policy serializers likewise use inline names129. These native presentation/storage limits do not truncate the active lane frame.

Lane evidence: `src/packets/world.rs` writers `write_system_faction`, `SCExpeditionRolePolicyListPacket`, `SCExpeditionMemberListPacket`, `SCExpeditionMemberStatusChangedPacket`; `src/systems/social/expedition.rs` row Marshal and `send_expedition_info` send chunks of20. `send_expeditions` sends faction chunks then an empty policy list. Native list readers397c1a90/397c1cf0 clamp inline policy/member counts20. Our lane decoder consumes all declared rows, including over20, to preserve following DD04 packet alignment; it does not emulate unsafe native inline-array handling. The legacy lane `SCFactionRelationListEmptyPacket` misleadingly uses opcode008 with two zero bytes, whereas actual faction-list opcode008 has one count byte: this patch does not guess a second schema or reinterpret relation packets as factions.

Native cache folding is source-backed:391d2010/391d2110 dispatch lists through392f7fb0/392f7ff0.398a4cd0 upserts faction IDs,398a4ee0 upserts composite(expedition ID,role) keys,398a4ff0 upserts members by persistent character ID alone,398a50e0 status replaces that keyed row (and creates it if missing). Empty lists do nothing to these caches. Later duplicate keys replace earlier cached values, but the ordered `Event::Expedition` channel retains all original batches/order/duplicates; `Expeditions.last_receipt` holds only the last complete batch and `totals` retains the last declared list total per context without claiming completeness. Character selection, kick, left-world and disconnect clear all runtime caches/last receipt.

Original X2Faction boundaries for integration: GetMyExpeditionId39472e70?392f80e0 returns manager39d44be8, not first cached member/faction. IsExpedInfoLoaded39472d00?392f8ad0 compares39d44b90==39d44be8. Member-list handler392fb160 hydrates own ID/role and loaded context only when packet total equals keyed cache count, policy-tree count39d44bd0 is nonzero and actual local persistent-character row exists. The received resource does not manufacture own membership; reconciliation requires the actual local persistent character identity. GetExpeditionMemberCount39473370?392fb780 returns online count (raw online byte nonzero across keyed rows) then cache row count, not declared total. GetAllExpeditionRolePolicies39473090?392fb610 filters sorted composite tree by own expedition and creates dense rows via392faa00; Lua role array begins role,name,invite,expel,promote. Native status392f9630 emits event1b4(existing),1b5(missing),or1b8(existing and flag bit0), after upsert. Registration39215cfd..39215d52 identifies these as `EXPEDITION_MANAGEMENT_MEMBER_STATUS_CHANGED()`, `EXPEDITION_MANAGEMENT_UPDATED()` and `EXPEDITION_MANAGEMENT_MEMBER_NAME_CHANGED()` respectively. Local RequestExpeditionMembers wrapper394727d0 subtracts1 from Lua start index (negative returns without work), then392fef20 filters cached rows (all when true, otherwise online nonzero), sorts locally and emits1b6 `EXPEDITION_MANAGEMENT_MEMBERS_INFO(totalCount,zeroBasedStartIndex,memberInfos)` at392ff1fb; no wire request is sent on that path. Detailed sorting and row conversion remain open. In particular MemberList does not directly emit `EXPEDITION_MANAGEMENT_MEMBERS_INFO`; the original paging/request path owns that callback.

Source artifact: `data/extract/aa-net/native-expedition-received-20261018.txt`. Tests cover independent row/metadata/policy wire goldens, every row truncation, impossible count failure, binary strings/noncanonical boolean bytes, full lane over-inline-cap DD04 alignment, chunk and duplicate event forwarding/last-receipt retention, online/context replacement, empty policy markers, Session repeat forwarding and WorldState/Bevy lifecycle reset. Remaining: management deltas, membership actor fold, page/loaded identity reconciliation, original getter table completion and outgoing actions. No live server transactions were attempted.


Stopped-timer app estimate invalidation follow-up: once `fold_stop_timer` passes the existing unique-local timeline, nonzero duration, received GM-byte0 and known primary-tag catalog gates, it removes only the resolved skill's prior `last_used` cast/DB estimate. This applies even when the first existing native timer is longer and native insertion is a no-op; that row's original timestamp/remaining/total/origin and all raw receipt history remain unchanged. Expiry therefore cannot resurrect an older app estimate. Rejected/unknown/ambiguous/foreign/zero-duration stops retain observations, and a later genuine Started can create a fresh one. This is app observation invalidation, not a new native ACK, duration estimate or expanded timeline-lifetime claim.

The static cooldown routing catalog now uses `Arc<HashMap<u32,u32>>`; `set_cooldown_catalog` still accepts a plain map. Per-frame `Skills` snapshots clone the allocation handle rather than all descriptor rows. PlayerSpawn and character/connection resets retain the same allocation; explicit catalog replacement installs a new one, while equality remains by value.

Expedition cache storage follow-up: the resource retains only `last_receipt: Option<ExpeditionEvent>`, not an unbounded status/event history. Complete raw row order, duplicate receipts and empty list markers remain available in the forwarded `Event::Expedition` stream; keyed caches still upsert exactly as before. This is bounded latest-receipt storage without an invented native history count policy.


### Native expedition identity and bounded cache admission (2026-10-18)

Character mapping is now closed: native Character serializer39620030 visits faction ID at+a0, faction name+a4, then the u32 expedition ID at+128 (3962021f..2b), then family ID+12c. This is the same field order as `Character::read`. Actual OnCharacterState391d3420?391b9640 loads that+128 at391b98c3 and calls392f8080. The latter resets member/policy view, own-role and loaded-ID, retains faction metadata, then stores its received argument as own-ID. Constructor3998fbb0/3998fbf0 copy invalid sentinel39d44b8c (static and verified runtime0) to loaded-ID/own-ID; role39d44b70 is zero-initialized. Pure reset392f8040 has own-dismissal392fde66 and local membership-change392fe381 callers. It is distinct from392f8080 received identity hydration. Source: `native-expedition-character-mapping-20261018.txt`, `native-expedition-ui-extra-source-20261018.txt` under ignored aa-net extraction.

`Expeditions` exposes actual `local_character_id`, `own_id`, `own_role`, `loaded_id`, plus `info_loaded()` (native equality, including0==0), `my_policy()` and ascending `own_policies()`. CharacterSelected captures the actual received persistent ID and explicit expedition field, resets member/policy/totals/latest receipt, and preserves faction metadata. Matching Character-kind PlayerSpawn validates the same actual persistent ID and preserves already received roster/view; a changed actual identity hydrates/reset view. It does not replay a Members callback retroactively. Native Members callback392fb160 alone reconciles after its cache upsert when total equals admitted keyed-member count, policy cache count is nonzero, and the actual local persistent-character row exists: own-ID=row.expedition, own-role=row.role, loaded-ID=packet context. It deliberately does not require context==row expedition; mismatch leaves info-loaded false. Policies and MemberStatus callbacks never retroactively perform this reconciliation. No first-row membership inference is used. Disconnect/kick/left-world retain the stronger app policy of clearing all server-scoped caches including faction metadata, rather than claiming equivalence with native dismissal reset.

Native list readers397c1820/397c1a90/397c1cf0 cap inline faction/policy/member arrays20; downstream callbacks retain the original count byte and would read beyond inline records for an over20 packet. This has no safe defined native cache semantics. Active lane writers send chunks20. The client now safely admits at most20 rows from each such packet to keyed cache, while `read_lane`, Event::Expedition and last_receipt retain every declared row in full and keep DD04 alignment. This over20 admission cap is an explicit app boundary around malformed-for-native packets, not a claim that native callbacks safely clamp. Cached faction/member/policy names and faction owner names are capped128 bytes; member memo is capped63. Single MemberStatus uses the same member caps. Embedded NUL bytes remain through the cap; original Lua text still uses the first NUL. Raw complete receipts remain untouched. Own count/reconciliation uses this admitted cache.

Getter follow-up: GetMyExpeditionName39472ef0 returns one empty string for invalid own-ID or missing faction metadata; an existing name is a C-string. GetExpeditionRolePolicy39473180 via39473900?394736f0 requires successful ScriptAnyValue numeric type4, narrows to f32 and CVTTSS2SI signed i32, then exact composite-key query392fcea0; invalid/non-numeric/missing rows return zero Lua values. There is no u8 role wrap. GetExpeditionMyRolePolicy39473280?392fcf10 queries the same converter with native own-role. GetExpeditionMembersPerPage binding39474714..758?39473430?392f8330 returns the same39ac729c used for paging division392ff097 and upper bound392ff149; verified original runtime value50, distinct from the original view's10 visible rows. Protected static bytes do not prove a literal50 initializer.

Regression scope includes constructor defaults, explicit CharacterSelected hydration, same-ID spawn ordering, changed identity reset/faction retention, count and policy prerequisites, no retroactive policy/status reconciliation, actual local-row lookup, mismatch contexts, full raw versus safely capped cache rows/text, and connection reset. Outgoing expedition commands and positionInfo formatting remain outside this change. The original Lua sets its derived MEMO permission false; no lane memo request handler was found, so no memo wire contract is invented.


### Expedition outgoing controls and invitation receipt (2026-10-18)

`ExpeditionCommand` adds Invite, Reply, Leave, Kick, ChangeMemberRole,
ChangeOwner, ChangePolicy and Dismiss. `Command::Expedition { character_id,
command }` requires InWorld and the same nonzero selected persistent character.
`ExpeditionRequested` means an actual socket write; `ExpeditionRequestRejected`
covers inactive, stale-character, malformed and failed/missing transport paths.
Neither notification is a server ACK, and neither changes received membership.
The UI remains responsible for native permission checks and captured task identity.

Native/lane payload agreement: CS006 is a full policy (u32 expedition, u8 role,
u16-length name, nine boolean bytes in received policy order); CS007 is **u8
role then u32 member ID**; CS008 owner and CS00f kick are u32 member ID;
CS00b dismiss and CS00e leave are empty; CS00c invite is a u16-length name;
CS00d reply is u32 expedition ID, u32 inviter ID, u8 join (the retail field
order, confirmed against the original client). Native constructors
397ab3d0/3f0/410/450/470/490, serializers397c4b60/80,397b90e0,
397b04f0,397b9190/1e0 and empty3901c170 agree with the active lane
`systems/social/packets/expedition.rs` readers. Request validation adds explicit
nonzero identities, <=128-byte names, and canonical policy boolean bytes. These
are application bounds, not a recreation of locale-sensitive native Lua parsing.
Binary names remain exact on the wire; native Lua works on first-NUL C strings.

SC01a now decodes `Invitation { inviter_id, inviter_name, expedition_id,
expedition_name }` as u32/string/u32/string. Factory391a7f6e sets opcode1a and
vtable399b30e4; serializer397b2d90 caps each native inline string to128 bytes.
The lane writer has the same order and complete dynamic strings. Raw events and
last_receipt preserve all bytes. Receiving an invitation creates no faction,
member, local membership, automatic reply or speculative timer.

Native wrapper39472700..394727c9 returns no Lua values for actions; no common
feature-bit gate exists in these bodies. Target lookup398a3480 walks persistent-ID
ordered cache and returns the FIRST name matched by **XlCompareStringI**
(IAT38ff6c54), not strcmp. Missing Kick/role target emits135hex; missing Owner
target emits148hex. Native Invite does not perform this target-cache lookup.
Admission/error order (all error numbers hexadecimal, passed to39425f50):

* Invite392fdf10 null name returns without send;398a4260 requires own faction
  (31), then own policy/invite permission (33); accepted name is UTF8-copied.
* Kick392f91b0 then398a4370: own faction (3e), non-invalid mother_id (3f),
  policy/expel permission (40), target not self (41), ownRole >= targetRole (40),
  and if political_system==0 target must not equal cached owner_id (42).
* Role392f8c50/398a4440: policy/promote, targetRole<ownRole,
  newRole<ownRole and newRole!=255, otherwise46. Equal-rank kick and role-change
  eligibility differ. Lua signed numeric conversion must not silently wrap here.
* Owner392f8cf0/398a3910: own faction exists and ownRole255, otherwise20;
  then392f4ec0 must return null, otherwise238. This scans a separate context tree
  for state4 and39839600(expedition,0); its full received producer remains open.
* Policy392f8bb0: ownRole255 else46; XlIsValidName(name,ValidNameType2)
  (IAT38ff6e98) else128. The validator dispatches by locale in xlcommon330095c0;
  a guessed ASCII-only name rule would not reproduce it.
* Leave392fa4b0: ownID0 emits137.398a38c0 requires own faction (3b) and
  non-owner role (136). If that helper fails and native member-cache count==1,
  it invokes Dismiss instead of reporting the Leave error.
* Dismiss392fa2f0/398a3870 requires own faction (2e), then role255 (30).

Accepted Leave opens Task26 (vtable399ccdf0); accepted Dismiss opens Task27
(vtable399ccdd0). Title/content are dialogTitle/dialogText keys
`ask_leave_expedition` and `ask_dismiss_expedition`. Positive callbacks392f9180
and392f8ff0 send the corresponding empty request; result0 sends nothing.
Join Task3 ctor392f88b0/vtable399ccd24 captures expedition ID+8, inviter ID+c,
and inviter name+10 (128 bytes plus NUL). Admission392f8940 rejects a queued
Task3 or option index2==1. Initial packet callback392f9590 also requires a local
actor and cached invited faction. Info392f9a00 uses title/text
`ask_expedition_invite`; text substitutes inviterName and cached factionName;
buttons common.invite/common.reject. Callback392f8970 rejects any ownID!=0
with132 even for decline, otherwise sends captured inviter/expedition IDs and the
result. It does not optimistically populate membership. Source archive:
`data/extract/aa-net/native-expedition-controls-20261018.txt` (ignored).

PositionInfo source correction:39303db0 receives **id3 first, zone_id second**.
It appends sub_zones.name from3976ade0(id3), then localized zones.display_text
from395779f0(zone_id). zones.group_id (row+10 u16) resolves zone_groups;
zone_groups.target_id (row+30) resolves world_groups.name (row+1c). `none` and
`main_world` are skipped; other names localize with kind92 and raw-name fallback.
The +1019c vector is directly indexed; read-only runtime entries correlate with
zones.zone_key, not zones.id (index0?id31/key0,2?id32/key2,3?id33/key3).
Its static fill producer remains unclosed; this is recorded runtime evidence,
not a fabricated worlds table. zones loader3982c100 SQL39a14830 explicitly loads
both zone_key and display_text; zone_groups3982c350 and world_groups3982f110
close the rest of the chain.

Offline last_world_leave_time is **Unix epoch seconds**, despite legacy helper
names: xlcommon XlGetCurrentFileTime3301ca70 calls _time64(NULL), and
XlDiffTime3301ca80 calls _difftime64. There is no Windows1601/100ns conversion.
39300c10 compares timestamp/current local CRT wallclock as unsigned u64, clamps
future values (including negative wire i64 interpreted unsigned) to zero age,
then394245f0 decomposes360-day years/30-day months/days/hours/minutes/seconds;
39429b20 exposes all six keyed numeric components. This clock is independent of
mapped monotonic ServerClock.


### Expedition authoritative mutation settlement (2026-10-18)

Received SC00c PolicyChanged, SC00d RoleChanged, SC00e OwnerChanged, SC015
Dismissed and SC01c MembershipChanged preserve complete binary names and raw
boolean bytes. Native factories391a77ee/786e/78ee/7e6b/806e and vtables399b3060,
306c,3078,30cc,30fc resolve serializers397c1b00,397c1b30,397b2b50,
397b2c70,397c56f0. These match the active lane social expedition writers.
SC01c order is BC object ID, u32 persistent ID, string kicker, string name,
u32 old expedition, u32 new expedition, u8 expel. Raw receipts are forwarded
regardless of native cache admission; no outgoing request fabricates these events.

`Expeditions::apply_with_notices(&Event) -> (bool, Vec<ExpeditionNotice>)`
returns source-ordered original no-argument callback names through `.event_name()`.
It is an ALTERNATIVE to `.apply`, whose compatibility implementation discards
notices. Never call both for the same receipt. Cache lifecycle and source guards:

* PolicyChanged success0 emits POLICY_CHANGED without modifying cache. Nonzero
  success requires an existing matching policy and policy.expedition_id==own_id;
  then replace that row (128-byte cached name) and emit POLICY_CHANGED.
* RoleChanged requires the new role's policy for own_id. Actual local persistent
  target updates own_role and emits ROLE_CHANGED; then the target keyed member
  receives role and UPDATED emits. Native operator[] creates a zero ROW if missing;
  persistent ID belongs to the tree key, so no name/identity metadata is invented.
* OwnerChanged requires own cached faction and exact current owner==old_owner_id.
  It updates faction owner/name FIRST. Only when role255 and role0 policies both
  exist does it set existing old/new member roles0/255, update actual local role
  with ROLE_CHANGED, and emit UPDATED. It creates no missing member rows.
* Dismissed requires nonzero success, an existing faction and nonzero mother_id.
  It deletes that faction. Matching own_id resets policies/members/role/loadedID
  and emits UPDATE_MAIN_MENU_EXPEDITION, retaining other faction metadata and
  actual selected/spawned identity.
* MembershipChanged native UI context chooses old expedition when nonzero,
  otherwise new expedition; that faction must be cached. This implementation
  also requires actual PlayerSpawn object AND persistent character identity
  (persistent guard is a stronger stale-object application boundary). It resets
  native member/policy view; joining from0 sets own_id=new, leaving to0 retains
  own_id0, both emit UPDATE_MAIN_MENU_EXPEDITION. Direct nonzero?nonzero resets
  but emits neither branch's menu callback, matching native control flow.
  Nonlocal old_id==own_id!=0?new_id0 removes the persistent target member and
  emits UPDATED, even for an offline actor (object0) or already-missing member.
  It still requires the cached context; before actual local spawn identity, a
  packet naming the selected local persistent character is not treated as remote.

`local_object_id` hydrates only actual matched PlayerSpawn and survives native
view reset; session lifecycle clears it. SC01c independently updates already
present matching Character Units/Player and the Player character snapshot. It
requires a nonzero matching persistent ID, never creates a missing actor and
never changes another object's reused identity. Native broader SC01c actor updates remain distinct from these guarded Character
folds; packet receipt alone does not establish an unknown actor's identity.
Dismiss bulk clearing is integrated BEFORE cache folding in both WorldState and
the Bevy plugin: `admitted_dismissal` checks genuine success/cached faction/mother
first, then Player/Units clear only already-present actors with the matching
nonzero expedition ID, before the reducer removes faction metadata. Failure,
missing faction and mother0 leave actor state intact.

MemberStatus notice correction:392f9630 selects EXACTLY ONE callback after row
upsert: missing row?UPDATED, existing with flag&1?MEMBER_NAME_CHANGED, otherwise
MEMBER_STATUS_CHANGED. There is no preceding status callback on rename.
Members batch392fb160 reconciles manager state but emits no management callback.
Source artifact `data/extract/aa-net/native-expedition-settlement-20261018.txt`.

Additional source closures: mother sentinel3a5e1388 is static0 and read-only
module-verified runtime0.390fa7fa binds g_ignore_expedition_invite to store+32c
(324+index2*4), default0;39182860 tests exactly1. This is option73's underlying
index, not Lua option2. Task admission3902d570 rejected-admission calls callback
with virtual+8()==1; Task3's +8 returns0, hence auto-decline, except callback's
already-member guard still blocks send with132. Policy table converter392fa6f0
maps Lua3/4/5/6/7/8/9/10 to invite/expel/promote/join_siege/siege_master/
dominion/chat/manager_chat. It NEVER writes dismiss+8d, and wrapper39472840 uses
uninitialized stack row storage; there is no proven native defaultfalse.
Preserving a known received dismiss value is an explicit safer application policy.

Owner guard392f4ec0 scans Dominion tree39d44aec for row.state(+d8)==4 and calls
39839600(own_id,null), matching expedition against row+e0 OR row+298. It is not
an any-siege-worldwide predicate. The received producer is now closed: SC01d factory391ce1ca/vtable399b7524,
serializer397afd00?3983a470?39839f40 on row+a8; the latter reads signed i8
siege_period into timers+30 (=row+d8), then participant structs at timers+38/
+1f0 (=row+e0/+298) via39839c80. Active lane DominionSiegeTimers agrees.
A typed full Dominion received cache/snapshot-completeness contract is still
needed before the client can safely return a negative blocker result; no
DB-only or missing-receipt 'no siege' context is substituted. Continued source:
`data/extract/aa-net/native-expedition-owner-dominion-20261018.txt`.


### Dominion received owner-eligibility state and observable parser gaps (2026-10-18)

`aa_net::Dominion` (singular) is a plain Clone/Default Bevy resource, also
`WorldState.dominion`. `.apply(&Event)` folds genuine `Event::Dominion` receipts;
`.owner_change_blocked(expedition_id) -> Option<bool>` reproduces the original
392f4ec0/39839600 gate from received rows. It checks exactly siege_period4 and
either defender/offense expedition ID. Zero is the verified invalid participant
sentinel and returns Some(false). Static constructor3998fa10 creates an empty
sentinel tree, so an empty cache legitimately returns Some(false), rather than
inventing a territory/phase. Coverage gaps return None for nonzero identities.

The five gate-mutating opcodes are source-closed against the native serializer
and the active lane social writers:

* SC01d Data: complete DominionData, u8 newly_declared, u8 final_data_by_request.
  Factory391ce1ca/vtable399b7524?397afd00?3983a470. Callback392f7890
  unconditionally upserts via3983a820; both flags control side effects. Neither
  flag clears rows nor proves a complete snapshot or repairs lost coverage.
* SC01e Deleted: u16 ID, removes that keyed row (392f7a10).
* SC01f OwnerChanged: u16 ID,u32 expedition,i64 reign timestamp,u8 bestowed.
  Existing-row helper392f7940 changes owner AND defender expedition ID, reign
  time, national tax rate500 and both tax-rate-change timestamps0. No new row.
* SC0e9 SiegeState: u16 ID, complete territory/timers,u32 new_owner, then ONLY
  if new_owner!=0 an i64 reign timestamp (397b4840). Existing-row callback
  392f7db0 runs owner helper only if the previous received phase==4 and new owner
  is nonzero, then replaces territory and timers. Missing rows are ignored.
* SC0ea SiegeDeclared: u16 ID, complete offense participant.392f7cc0 replaces
  the existing row's offense; it does not create a row or advance phase.

All complete raw fields are typed and retained: fixed XY i64 and Z f32; integer
Unix timestamps; five duration i32s; raw signed-byte-compatible siege_period;
participant object ID/position/ni/nr/limit and ordered duplicate member IDs;
territory dimensions; tax/national-monument/non-PvP metadata; both Data flags.
There is one bounded last_receipt, not a receipt history. Native timers
39839f40 place phase at row+d8 and participant structs at row+e0/+298. Participant
39839c10 reads a count byte and that many IDs with no capacity guard; native
inline storage has100 IDs. The safe lane reader consumes all declared rows;
>100 preserves raw data but poisons native gate certainty rather than emulating
native memory corruption. No clock estimate synthesizes phase changes.

That gate slice initially left tax/member/non-PvP deltas outside its reducer.
The accepted territory-management T1 extension below adds fixed-field receipts and
folds the existing tax/member Notices. SC0eb changes only defender member limit;
SC0ec edits participant members; SC0ed remains a presentation Notice. These do not
write phase or participant expedition IDs. The five gate mutators never fabricate
metadata or success notices; later partial/full data still cannot repair lost coverage.

The existing decoder cannot find the next inner DD04 packet after an unknown
opcode because inner records have no body lengths. It abandons that tail.
Session now forwards factual `UnhandledGamePacket { level, opcode, len }` for
previously trace-only unknown packets. Dominion poisons coverage for ANY abandoned
DD04 tail, which may conceal a later gate mutation, and for unsupported relevant
normal-game opcodes; an unrelated proxy opcode does not poison it.

A malformed inner packet similarly used to log and silently abandon its tail;
`ServerMsg::DecodeFailed` now survives as `Event::GameDecodeFailed { message }`.
Top-level game wire-decode failures also emit that event instead of only logging.
Dominion treats either failure as sticky loss of certainty. Neither malformed
nor unknown packets create partial territory state or invented receipts. This
makes the existing general compressed-tail coverage gap observable; extending
actual packet parsers is still required for wider parity.

CharacterSelected/LeftWorld/Kicked/Disconnected clear received Dominion state
and coverage flags as explicit app session isolation. This is not a claim that
native static territory managers are recreated on every corresponding callback.
No Data flag or later partial row clears a coverage gap. The resource and plugin
use the same reducer; clone into a HUD view, then apply each event once.

Goldens exercise all five layouts, every byte truncation, conditional owner time,
raw values/duplicate IDs, following DD04 alignment, all five gate transitions,
missing-row behavior, constructor/invalid-ID values, sticky decode/unknown gaps,
inline-overflow handling, plugin and Session forwarding. Native source artifact:
`data/extract/aa-net/native-dominion-received-20261018.txt` (ignored).


### Expedition creation, rename, and actual faction context (2026-10-18)

`ExpeditionCommand::Create { name, sponsor_id }` writes CS004 blob/u32;
`Rename { expedition_id, name }` writes CS009 u32/blob/byte1. Both use the
existing captured-character Session preflight and real-write Requested/Rejected
notifications. Name bytes are preserved, with the existing explicit app cap of
128 bytes; no name policy, permission success, roster mutation, or receipt is
invented by a send. Native direct Rename accepts the current own ID even if zero
and returns BOOL true on invocation, not server acceptance. The lane rename
handler only logs the request, so this client cannot claim rename settlement on
that lane without an actual SC011.

Native Create39472810/392fa240 first applies ValidNameType2 (failure0x22), then
398a3820 requires ownID0 and cached current faction (failure0x21). Only afterward
it overrides the caller sponsor with current faction when current faction is at
least39ac72b0. That protected writable global is verified1000 in the read-only
original process; its initializer/producer remains unresolved. It is an audited
original-build runtime compatibility value, **not** a proven GameConfig field or
static PE literal. The lane parses the sponsor u32 but currently ignores it and
uses the actual character faction. Native adapter39473970?3905ef50 requires a
string first argument and numeric second argument;39058d10 converts f32 using
signed CVTTSS2SI and preserves its bits as u32 (including negative and x86
indefinite conversion results). Wrong types produce no action/zero Lua results.

`Expeditions.current_faction_id` starts from actual selected Character.faction_id,
matching391b98b7?392f8030/39d44b94. SC01b retains BC runtime ID, binary name,
u32 old/new factions and raw temporary byte. Only a matching hydrated local
actor and its current old faction update the manager field; missing actor,
wrong object, stale old faction, or removed actor do not. Existing Units and
Player apply the same old-faction check without creating actors. The raw
Temporary byte is retained on the event; no additional actor-mode meaning is
inferred. Lifecycle isolation clears this context. Native392fe0e0?390b9b20 does
also retain a temporary byte in the actor, beyond the current Unit representation.

`top_level_faction()` follows the received current faction through cached
mother_id links, returning the last reached ID if a row is missing or its mother
is0. Source39472c70?392f80d0?392f8180?398a37a0 is recursive, unlike the
single-parent398a3740 helper. The app uses at most cache.len()+1 iterations and
returns None on a malformed cycle; native has no cycle guard. No race-based or
static148 fallback is used.

SC00f is `ShowRenameUi`, empty:391eba93?391ba8e0?391a7950 allocates opcode0x0f
with vtable399b3084; serializer39184540 immediately returns. Handler391d2310
calls392f8db0(nullptr). That planner silently refuses ownID equal to manager
invalid sentinel39d44b8c or398a36e0 failure (missing own faction, political_system
nonzero, or cached owner not actual persistent local character). It captures
cached faction name into Task35's128-byte buffer. Task vtable399ccd90 reports
buttonType0, isModal=false, admission only if no queued Task35, and info key
`expeditionName`. Its result callback392f8a70 is a no-op; the original Lua
OkProc alone calls RenameExpedition. There is no invented ShowRename Lua method.
Other calls to this planner are391e5585 and392f7c0e, outside this empty-receipt
path's completion claim.

SC011 `Renamed` is u32 faction ID/blob name/raw by_gm byte. Native392fdce0
requires existing faction metadata, replaces its capped name, and clears
allow_change_name only if by_gm==0. It does not change own ID, policies or loaded
state, and emits no expedition-management notice in this callback. Raw receipt
bytes remain available even when metadata is absent.

SC013 `Created` carries a complete Faction, BC owner runtime ID, countu8, then
interleaved BC member object/u32 persistent ID/blob name rows. Native397c1910
clamps its inline count to4 before reading; the active lane writes the declared
full vector. The lane decoder consumes **all** rows to keep subsequent DD04
packets aligned, while the native state view admits only the first4. Raw event
and last_receipt retain every row. Native398a5280 inserts absent metadata but
leaves existing metadata unchanged on duplicate creation.392febb0 requires
political_system0 and a nonzero result from the single-parent398a3740 lookup.
The app additionally refuses membership effects for invalid factionID0 (a stronger
malformed-identity boundary); valid IDs satisfy that native parent check. It emits
an initial menu notice when actual
local persistent ID is the incoming owner and incoming mother!=0, and invokes
the ordinary membership reducer for owner then each admitted row. This may emit
a second genuine menu notice for the matching local actor; neither is deduped.
Existing actor updates require matching persistent character identity; no first
row, proximity, ownership guess or new actor supplies membership. The raw Created
receipt remains distinct from its internal source-ordered membership effects.

Source excerpts: ignored `data/extract/aa-net/native-expedition-create-rename-20261018.txt`.
Lane sources: `systems/social/packets/{expedition,faction}.rs`. Regression coverage
includes binary strings/raw boolean bytes, every truncation, full count5 framing
past the native4 boundary, following-packet alignment, duplicate creation,
actual actor/old-faction guards, recursive lookup cycles, existing-only rename,
Session create/rename writes without fabricated receipts, and Bevy resource folds.

The previous SC01b `ServerMsg::UnitFactionChanged` / `Event::UnitFactionChanged`
(String/bool) variants remain compatibility-only. Actual wire dispatch now uses
raw `Event::Expedition(UnitFactionChanged { .. })`, avoiding their lossy string
and bool conversion and the former old-ID assignment bug. The compatibility
Units fold now also requires the old faction and assigns the new faction.

Lane creation currently constructs political_system=1, then sends SC013,
broadcasts SC008, sends SC01c(0?newID), and sends policy/member info. The native
political_system0 Created planner therefore deliberately skips that SC013's
membership/menu effects; the subsequent genuine SC01c supplies membership.
The same political-system boundary prevents native Task35 admission for that
lane-created row. No client workaround changes the received political system.

Validation: cargo test -p aa-net --features bevy --lib: 433 passed, 0 failed, 0 ignored (2026-10-18).


### Native skill-result framing prerequisite and duration oracle (2026-10-18)

Plain `SkillResult { raw_flags:u8, code:u8, extra:u16, value:u32 }` follows actual
reader397b1ef0. It first clears all three decoded fields, then reads the header:
bit0 selects codeu8, bit1 selects extrau16, bit2 selects valueu32. Fields are
independent, including simultaneous extra/value. Unknown header bits are retained
and consume no additional bytes. `SkillFired.result` now decodes this full record
before fire_anim_id; the previous one-byte skip was sufficient only for the
active lane's zero-header success writer and desynchronized native nonzero results.
`SkillStarted.raw_result` retains the same complete record; legacy result/code and
result_value stay compatible (valueu32 takes precedence over extrau16).

`SkillFired.raw_final_flag` remains the complete wire byte.
`native_cooldown_suppressed()` tests **bit1**, not nonzero raw byte. Native
397ca390?397ca4aa..bd splits bit0 to packet+14c and bit1 to packet+160, the latter
being the gate passed by391d58dd to3939b010. This is only one producer predicate;
no Started/Fired duration or GCD timer is introduced by this parser milestone.

Actual x86 reader397b1ef0 and the packed flag decoder were executed locally in
Unicorn, mocking only the abstract reader's primitive accessors. Nine result
header fixtures and all256 packed flag values are archived in ignored
`data/extract/aa-net/native-cooldown-fired-result-oracle-20261018.json`, with helper
`fired_result_oracle.py`. Inlined Rust goldens preserve the executed outputs;
all256 result-header combinations also run truncation and three-packet DD04
alignment regressions (Started?Fired?Ended). Native clearing from an initially
0xcc-filled destination verifies zero defaults, not an invented success receipt.
The lane's zero result/zero packed flag encoding remains unchanged.

The separate read-only duration oracle executes original3989f900, runtime
three-list enumeration39387a50, callbacks398a05b0/398a05f0, matching398a0480,
learned-passive filtering3989e5c0, and racial registry3989e6a0/39682160. Only native
BuffDesc/PassiveDesc lookup functions are mocked with explicit fixture tables;
provider loops and arithmetic are original instructions. Runtime records use
native stack+40, inactive learned passives use stack1, and the first matching
racial/gender record supplies its static Buffs with stack1. Unlike unit-attribute
callbacks, this skill-modifier path does **not** skip one_time Buffs and does not
apply ability-level/conditional/dynamic unit-modifier logic.

Catalog SQL3973d460 selects owner_type,owner_id,skill_attribute_id,skill_id,
synergy,tag_id,unit_modifier_type_id,value from skill_modifiers. Owner Buff rows
become BuffDesc+274. Runtime-verified invalid skill/tag sentinels3a5e131c/1320 are0;
native SQLite integer getter3953a390?3952a830 returns0 for NULL, independently
executed in the oracle. Category1 matching is universal(skill0/tag0), exact skill,
or any matching descriptor tag, once per row; synergy is an output flag rather
than an eligibility gate. Type0 adds wrapping value*stack to flat; type1 to
percent, initially100. Base+flat wraps before the optional f32 percent multiply
and0.01f32 multiply/CVTTSS2SI; signed nonpositive results clamp0. Percent100 skips
the float path completely.

Executed examples: base16777217/percent100 retains16777217; percent101 gives
16944988. A mixed runtime/passive/racial fixture gives6825 and synergytrue,
including a one_time Buff; actual compact Buff1853 stack2/base10000 gives4000,
2323 gives0,402 gives6000,403 gives5000,404 gives4000. Signed overflow and out-of-
range CVTT produce the native zero-clamped outcomes. Full descriptors, mocks,
callback order and results are in `native-cooldown-duration-oracle-20261018.json`
and `cooldown_duration_oracle.py` under the same ignored directory.

Ordinary timeline lifetime remains a separate admission boundary:39394e70 and
39399e90 prove a unique u32TL tree, but39397ff0 ends/decrements a record without
immediately erasing its node. Fired3939b010 validates retained raw skill/caster/
target using39391da0/39391e30; on mismatch it ends the record then invokes
3939a810, and an equal-key insert may still return the old record. Physical erase
occurs later through3939c540/3939c780?3939a0e0 and reference counts+304/+308/+30c.
Consequently incoming Fired skill/caster alone cannot prove the native producer's
retained identity. Existing bounded stop timers and estimates are unchanged;
no immediate replacement model or GCD/A74 approximation is claimed.

Validation: cargo test -p aa-net --features bevy --lib: 435 passed, 0 failed, 0 ignored (2026-10-18).


### Received skill caster/target identities (2026-10-18)

`SkillStarted` and `SkillFired` now expose additive `raw_caster:SkillCasterData`
and `raw_target:SkillTargetData`. Their existing `caster:BuffCaster` and
`target:SkillTarget` projections retain their prior shapes and values. This is a
received-data prerequisite, not a new cast/timer producer or retained-TL model.
Other BuffCaster/SkillTarget consumers remain their existing compact projections;
this milestone does not claim complete preservation of every SkillObject/Buff.

Caster kinds0..4 preserve Unit, Unk1, Item, Mount and Doodad respectively. All
carry the BC runtime ID; Item additionally retains u64 item ID, u32 template ID,
u8 type1, u32 type2; Mount retains its u32 mount skill ID. Target kinds0..5 retain
Unit, Position, Position2, Item, Doodad, Position3. Position retains raw i64 X/Y,
f32 Z, f32 rotation and two BC object IDs; `SkillTarget::position_rot` writes that
rotation (`SkillTarget::position` keeps 0.0 for the ground casts). Position2 retains raw position plus
three-f32 end and normal vectors; Item retains BC object, u64 item ID, u32 type1,
u8 type2; Position3 retains raw position and f32 pitch. `SkillPosition` retains
**all** fixed-point bits; `.projection()` performs only the preexisting display
conversion. Unsupported discriminants are rejected, an explicit app framing guard
rather than guessed extension lengths.

Native397c85b0 and397bfa90 match the active lane's SkillCaster/SkillCastTarget
read/write schemas field-for-field. Native helper39391da0 compares caster kind
and object for0/1/3/4; Item compares item ID/template/owner object/type1, deliberately
ignoring type2. Mount skill ID is also deliberately ignored by that identity
comparison. Native39391e30 compares all retained target identity fields except
Position.rotation. Position object IDs, Item identity/types, Position2 end/normal,
and Position3 pitch are compared. All compared floats use exact equality with
NaN rejection and +0==-0; raw X/Y compare exactly, including bits lost by the
legacy f32 projection. `.native_eq()` implements these complete known-kind helpers;
derived PartialEq instead compares full retained data, so the two intentionally
differ for ignored fields. Neither helper establishes timeline ownership,
recreation success, planner eligibility or a cooldown receipt.

The original two x86 comparators were executed locally without hooks/mocks for
75 kind/field/float fixtures. Inlined Rust tests contain those outputs, including
ignored Itemcaster.type2, Mount.mount_skill_id, Position.rotation (even NaN),
compared NaNs, signed zero and low raw-coordinate bits. Additional tests cover
all five caster and six target wire layouts, unknown kinds, every truncated
prefix, legacy projections, and every30 Started/Fired identity combination followed
by a genuine Ended packet in DD04. No generated artifact is required at test time.
Ignored source/oracle artifacts: `data/extract/aa-net/native-skill-identity-source-20261018.txt`,
`native-skill-identity-oracle-20261018.json`, and `skill_identity_oracle.py`.

Validation: cargo test -p aa-net --features bevy --lib passed all 438 tests (0 failures), including native identity goldens and DD04 alignment coverage. No timer production or retained-timeline policy changed in this slice.

### Native cooldown frame clock and timeline lifetime audit (2026-10-18)

Read-only closure, with no clock or Fired timer implementation: actor update
3936a750 samples Cry timer0 integer milliseconds through timer virtual+10 at
3936a7f6, computes wrapping u32 `now - previous` (global3a16e194), substitutes
zero delta when `now == 0`, then stores now. For every current actor it calls
3936a0b0(delta), which passes that same delta to cooldown manager39899e50.
The latter saturating-subtracts unsigned min(delta, remaining) from every ordered
tag/direct row and global remaining; totals, IDs, counts and ordering are unchanged.
Direct E8 caller coverage:39173bcb ->3936a750;3936a836 ->3936a0b0;
3936a280 ->39899e50. This is local frame time, not mapped ServerClock or SC191.now.
Local Unicorn execution of original39899e50 and39899d10, without hooks/mocks,
confirms six delta cases including zero, equality and unsigned extremes.

Cooldown initialization39899d10 sets owner/provider pointers, clears global
remaining/total and both list counts, without clearing stale backing row bytes.
Its only direct E8 callers are actor construction39366237 and actor hydration
3936b15b. This actor-manager initialization does not prove global timeline reset.
Zero computed duration is an explicit no-op in39899f90 at39899fce..fe6.

Global ordinary timeline manager39aff9ac is allocated at391746bb..df and
constructed by3939c210 with an empty tree/count0. Destructor3939bd20 destroys
the tree;39175050 then frees the manager and clears the global pointer.
Explicit tree clear3939a2d0 has one direct E8 caller39171966 in39171800(mode),
which deliberately skips this clear for mode3. All direct reset callers found:
390f2b69(mode1),3915bd04(mode2),3917221a(mode0),3930b468(mode3),3930cb4b(mode3).
Mode-to-app-event semantics and indirect/protected callers remain unclosed:
CharacterSelected, PlayerSpawn, disconnect or HUD reconstruction must not be
assumed to establish fresh native timeline history.

Fired3939b12b..151 extracts the actor ID from the retained caster and resolves
it again through current registry39360020. Its cooldown path uses that current
actor, not a retained actor pointer. A reused object ID nevertheless may combine
an old retained skill identity with a newly hydrated actor/provider set; broader
effect references and timeline lifetimes remain distinct. Native frame391739e0
runs timeline update through39aff9ac before actor update3936a750. The enclosing
packet-dispatch ordering is not yet closed, so existing Instant-based timers
remain unchanged and are not claimed to reproduce native frame quantization.
GMmode1 admission uses the actual received snapshot/SC203 producer documented
above; missing context is not assumed zero. No Fired producer is added.

Ignored source/oracle artifacts: `native-cooldown-frame-lifetime-20261018.txt`,
`native-cooldown-frame-oracle-20261018.json`, `cooldown_frame_oracle.py` under
`data/extract/aa-net`. Direct E8 scans are caller evidence, not proof against
indirect/protected paths. No foreign process execution or writes were performed.

### Received achievements: SC1fc..200 (2026-10-18)

`AchievementEvent` retains full lane List raw i32 count/ordered rows, Changed
(id/u32,amount/i32), Completed (id,raw i64 timestamp), Reset (id,amount,timestamp),
and ItemSent (id,raw by-mail byte). Every event is forwarded, including duplicates.
`AchievementProgress` preserves signed amount bit patterns and raw timestamps.
Lane `aaemu-commons::PacketStream::write_date_time` writes i64 nonnegative Unix
seconds; no timestamp normalization, local-time conversion, or success inference
occurs in aa-net. Native serializers use the ordinary qword primitive. Original
GetAchievementInfo later calls394245c0 -> xlcommon XlGetLocalTime3301ca30 ->
MSVCR100 `_localtime64_s`, adding1900 to year and1 to month. Negative/extreme
wire timestamps are preserved even though the native calendar conversion can fail.

Factories391b77d0/7850/78d0/7950/79d0 establish opcodes1fc/1fd/1fe/1ff/200,
vtables399b4318/4324/4330/433c/4348. Serializers are397b8f30/89f0/8a30/8a70/8ac0.
The list serializer reads count then interleaved id/amount/qword timestamp, only
min(count,50) rows into native inline arrays. The active lane writer emits every
declared row. Its decoder therefore consumes every lane row for DD04 alignment,
rejecting counts exceeding available bytes before allocation. Negative counts
retain the raw count with no rows. Cache admission is separately bounded to the
first50 declared rows. The native batch callback3900e700 unsafely iterates the
unclamped declaration; >50 cache admission is an app safety boundary, not a claim
that native out-of-bounds behavior has a defined result.

`Achievements` is a plain Bevy/WorldState resource with a keyed `entries` map,
bounded `last_receipt`, and explicit `native_enabled:Option<bool>`. Full receipt
ordering remains on Event::Achievement rather than an accumulated status history.
Batch cache inserts are first-wins, including across chunks/repeated snapshots;
empty batches do not clear prior entries. Batch writes require neither metadata
nor the scalar feature gate and emit no native update/completion notice.

Scalar callbacks39186250/2a0/2f0 call3900ea70. Native admission requires a known
AchievementDesc and game+18 bit31 set, except its absent-game-object special case.
The app does not assume that exception. Actual InitialConfig feature byte7 bit7
hydrates the gate; missing byte remains unknown and blocks scalar cache writes.
GetFeatureSet39496f4d..7c reads the SAME bit from feature-storage+4 and exposes
`achievement`; it is not a second UI availability flag. Static rules are supplied
through `set_catalog(HashMap<u32,AchievementRule>)`, stored in Arc so HUD clones
share catalog storage. Rule fields are `complete_num:i32` and
`objective_count:u32`. Native3975208e loads ALL achievement_objectives rows;
39752ad0..b6f links each to a known parent regardless of missing record metadata.
Thus objective_count counts loaded rows for that parent, not only valid record IDs.
No retroactive scalar replay occurs when a rule/config later arrives.

For timestamp0, scalar cache writes use the received signed amount (Completed's
native constructor argument is0). For ANY nonzero timestamp, including negative
raw values, amount becomes positive complete_num, or objective_count when
complete_num<=0. `apply_with_notices(&Event)` returns `(changed,notices)` with
Completed(id) then Updated, after the received cache write, per admitted completion
receipt even when repeated. Timestamp0 emits only Updated. `apply` delegates and
discards notices; UI consumers must use one fold or the other, not both.
These represent original COMPLETE_ACHIEVEMENT(id) and ACHIEVEMENT_UPDATE().
ItemSent391e2490 calls3900e3f0: known metadata emits
ACHIEVEMENT_ITEM_SENT(localized achievement name), ignoring byMail for that Lua
callback. The resource returns ItemSent(id) so root can resolve the genuine
localized name, without fabricating a progress/reward mutation.

Native manager39173e50 constructs an empty progress tree and empty recent-complete
vector;3900ee50 destroys both at manager teardown39175491. Native completion also
appends its ID to recent-complete vector+4a8 WITHOUT deduplication, and removes the
first matching tracing slot among six slots+490 (invalid sentinel0), persisting
local UI data. Removing a recent ID3900e7a0 removes the first matching entry and
emits Updated; batch hydration does not mark entries new. This local recent/tracing
adapter remains root work, separate from this received-progress cache. App
CharacterSelected clears received cache/last receipt while retaining known feature
configuration; login/reconnect/disconnect/kick clear received runtime and gate.
Static catalog survives all these clears. These are explicit character-isolation
boundaries, not claims that every event invokes native global-manager teardown.

Original getter entry points: GetAchievementInfo390598f0 (numeric adapter39059450),
GetAchievementList390595d0, GetSubcategoryInfo39059720. Registrations3905a231/28d/2ee
and tracing/recent methods3905a34a..5ab are archived. Native missing progress lookup
3900df80 returns zero row39ada858. Info emits current as signed-i32-to-f32,
complete=(timestamp!=0), and completeDate only when nonzero. Static/filter/category
and local tracing presentation remain aa-client integration; no outgoing reward
claim or optimistic completion is introduced.

Validation: all444 aa-net Bevy library tests pass. Coverage includes all five
layouts/all truncations, signed amount/timestamp extremes, noncanonical raw booleans,
negative/oversized counts, >50 lane rows followed by a real DD04 packet, first-wins
chunk/cache behavior, catalog/feature unknown gates, completion normalization and
repeat notice order, Arc sharing/lifecycle, WorldState/Bevy integration, and Session
raw receipt forwarding. Existing compression unknown-opcode goldens moved from now
implemented0x1ff to unsupported0x7fff; the independent Python-zlib generator changed
only that raw/compressed fixture and its hashes. Ignored native source archive:
`data/extract/aa-net/native-achievement-source-20261018.txt`. Lane sources:
`systems/social/packets/misc.rs` and `aaemu-commons/src/packet_stream.rs`.

#### Achievement list and local presentation source follow-up

GetMainCategoryCount39058f50 returns literal5.0; GetSubCategoryCount39058fd0 ->
3900c6f0 maps main1/2 to4, main3 to5, main4/5 to3, others0. List helper3900ef30
requires those positive subcategory ranges. Main0 requires sub0: filter2 returns
all completed cached IDs, filter5 returns occupied tracing slots, other filters
empty. Normal category/sub matches require is_active; hidden entries are excluded
only until completed. Filter1=all,2=nonzero completion timestamp,3=zero timestamp,
4=signed amount>0 and zero timestamp; other filter IDs return empty normal lists.

Filters1/3 sort via3900c9b0: same parent ID uses signed own priority then original
ID; differing parents resolve nonzero parent descriptors and compare their signed
priorities then original ID. Filter2 sorts timestamp as unsigned u64 descending,
then ID ascending. Filter4 sorts f32 signed-amount/completion-threshold descending,
then3900c9b0. Native zero-threshold comparator has a non-strict special case;
malformed zero-threshold progressing rows are not claimed to have a stable order.
GetAchievementList emits dense1-based IDs as u32-to-f32 in a table (empty table
for no results). GetSubcategoryInfo counts filter2 and filter1 into complete/all.

GetTracingAchievementCount39059160 returns TWO f32 numbers: occupied count and
max6 (literal3999ecf4). AddTracing3900cf60 rejects duplicates, including invalid0,
then fills the first empty of six slots; full returns false. Success persists
local UI data and emits Update. RemoveTracing3900cfc0 clears the first matching
slot without compaction, persists and emits Update; absent ID does nothing.
RemoveNewComplete3900e7a0 removes only the first duplicate marker and emits Update
if found. RemoveAllNewComplete3900e900 clears recent markers and emits Update even
when already empty. GetNewCompleteCount39059210 counts duplicates; Info.isNew
3900db40 means any matching marker. These are local UI state, not wire ACKs.

Info.highRank tests parent ID against39addc08; child icon uses own icon unless it
matches39addc0c, then parent icon. Both sentinels and tracing invalid39ada854 were
verified0 by PROCESS_VM_READ in existing PID48924 after image/StandMode checks;
no writes/input/injection were performed, and the handle was closed. Main literal5
and tracing max6 are separate constants. Runtime artifact:
`data/extract/aa-net/native-achievement-runtime-20261018.json`.

Info.objective39059db9..eb3 iterates objective descriptors, resolves record_id via
39744560, and emits only records with kind_id==9; each dense1-based table value is
record.value1 converted signed-i32-to-f32. Zero qualifying records yields nil,
not an empty table. This differs deliberately from completion objective_count,
which counts all loaded linked rows regardless of record validity/kind. Native
CharRecord SQL399f91e0 selects id,kind_id,value1,value2. Full source excerpts and
jump-table entries are appended to native-achievement-source-20261018.txt.

Twelve local executed-original comparator fixtures are archived in
`native-achievement-sort-oracle-20261018.json` with `achievement_sort_oracle.py`.
Original3900c9b0/3900e1f0/3900e2a0 execute; only static descriptor lookup397445a0
and keyed-cache lookup3900db70 are mocked. They verify parent/signed priority,
unsigned negative timestamp ordering, f32 ratio tie/priority fallback, objective
count fallback and both-zero denominator non-strict behavior. No full sort output
is claimed from pairwise comparator execution.

### Optional native UI-data text tree (bounded supported grammar)

`ui_data::{NativeData, NativeDataNode, NativeDataError}` is an opt-in codec; the received
`UiDataResponse.payload` and raw trailer remain opaque and unchanged. Nodes hold byte values
and ordered children, preserving duplicate names and unrelated ordinary subtrees. Parsing is
atomic. The initial supported forms are unquoted/single-/double-quoted values, the native
quote/backslash escapes, CRLF/LF quoted values, chains, parenthesis/comma groups, aligned
space-indented trees and the UTF-8 BOM. Comments, root metadata (`#?`), backslash block
strings, tabs and unmatched indentation levels are explicitly refused; callers must retain
the original payload and decline edits on refusal. This is a supported subset, not a claim
that every refused input is invalid to the native parser. No partial tree is returned.

The writer follows `xlcommon::Data::SaveToBuffer`33002ce0 through33002bc0/33002a90:
compact trees use a raw-value length budget below78 and depth below3, single children use
a space, siblings use ` ( ... , ... )`, expanded children indent four spaces, and every
root line ends CRLF. Empty values and signed-byte whitespace/delimiters are quoted; native
quote/backslash escaping and multiline CRLF output are retained. NULs, leading comment/block
tokens and non-roundtrippable leading quotes are refused rather than emitted ambiguously.
The application8191-byte bound is checked independently of native output-stream truncation.
`encode` returns text bytes without a trailing NUL; converting arbitrary byte values to the
existing UTF-8 `UiDataCommand::Save.payload` remains an explicit caller validation.

Ignored `achievement_trace_save_oracle.py` executes ORIGINAL33002ce0 and all native stream/
formatting helpers without mocked imports. Its six goldens cover trace6+version, empty,
nested compact, quoted/non-ASCII/escape values, long expanded and multiline trees. Those
expected bytes are embedded in the crate tests. `achievement_data_load_oracle.py` executes
ORIGINAL33004af0/all lexer/parser helpers with only allocation/reallocation/free and
memcpy/memset primitives mocked; it confirms the ordinary tree and quote handling. Native
Load accepts further comments/metadata/block forms; these remain outside the initial codec.
Its source is `<client12-unpacked>/ida/xlcommon.c`330023e0..33004be0.

Tracing3900cac0 edits type5 Data in memory; it does not immediately send CS118. Runtime
storage descriptors establish type5 network-backed/version1. `SaveAll`392178d0's sole direct
caller390f455d is inside leave-world action390f4420, before it constructs/sends CS001 via
397ab390. Adjacent390f45d0 constructs cancel-leave CS002. Generic save39217640 captures
`[game+40]+18`, serializes the entire type5 root (including unrelated subtrees/version),
then sends CS118; no save acknowledgement exists. Further identity-field producer closure
and root integration are separate from codec success. UI's source audit establishes
session+7b as selected world ID, not account ID; local key scope must preserve that distinction.

Codec validation: focused3 tests passed; the same freshly built aa-net library binary then
passed all447 tests (0 failed/ignored). No network traffic or persistence claim was involved.

Type5 hydration/identity follow-up: native CS117 helper392175f0 has only391eb233/242 callers,
which request types1/2 for character-list rows. There is no source-proven type5 request timing;
the active lane `world/session.rs` pushes types1/2/5 for the actual selected snapshot ID during
initial world hydration. OnCharacterState391b9640 allocates/constructs object3930fa80, stores
it in client+40 at391b988f, then passes object+10 and received CharacterState to39089d50.
That helper copies the received FIRSTu32 Character.id into header+8, hence object+18.
Consequently CS118's `[client+40]+18` is the actual selected persistent character ID.

SCResponseUIData391e57e0 ignores its character-ID field for types other than1/2, calls
39217910, then publishes observer0x2c(type) regardless of load success. The application must
retain its stronger active-character receipt guard. Native Data::Load(Size<=0) leaves the
existing tree intact. An executed native length oracle confirms that short positive lengths
can succeed while silently losing siblings or truncating later values; excessive padded
lengths can fail with a partial tree. Exact payload byte count or byte count+1 (implied NUL)
retains the complete tested tree. Lane UTF16units+1 is not sufficient: a30-byte UTF8 fixture
with four non-ASCII characters supplies27, truncating later `12345` to `1234` while succeeding.
For atomic app editing, refuse positive lengths other than full UTF8 byte length or +1, and
leave the current tree unchanged for nonpositive lengths. These safety checks are caller
policy; the optional raw text codec deliberately does not reinterpret a response trailer.

Native trace integer loading uses XlAtoI32(base10)3301bcb0/3301ac60: skip ASCII space/9..13,
consume optional +/- and the initial decimal digit run, wrapping multiply/add/negation in32
bits; no digits means0. Twelve no-mock x86 oracle cases include trailing text and overflow.
`ChildWithValue`33002dc0 returns the FIRST case-sensitive strcmp match. Save's version1
insertion uses SetAttribute33004190/33003fa0: replace first `version` node's children/meta
with sole child `1`, preserving sibling position/name/comments; append if absent, leave later
duplicates untouched. The root caller owns these operations and must preserve unrelated nodes.
The empty-string/raw_length1 case is an additional boundary: executed native Load reports
failure and leaves a partial0xff-valued node, with no tracing subtree; the observer still
reloads zero trace slots. Treating it as an application empty state or refusing it must be
explicit, not described as a successful native empty parse. raw_length0 retains prior state.

### Native snapshot/stop frame clock (2026-10-18)

`Skills::advance_native_frame(now_ms)` replaces continuous Instant projection for mutable
snapshot/stop manager rows. Call ONCE after queued received-event folds and before HUD
queries, using one wrapping u32 timer0 millisecond clock. `native_last_frame_ms` starts0;
nonzero now uses wrapping(now-last), zero now uses delta0, and both store now. Every direct
and tag row saturating-subtracts the unsigned delta; totals, order, expired rows, origins and
receipt timestamps remain unchanged. Last-frame time survives existing character/connection
resets and PlayerSpawn, although runtime rows still follow the existing resource resets.
`native_cooldown_at`/`remaining_ms_at` retain their signatures but ignore Instant: queries
observe stored frame remainder. `updated_at` is receipt/insertion metadata, not a timebase.
Raw SCCooldowns receipt history and app DB/cast estimates retain their separate APIs.
No automatic tick occurs in Session/Bevy receipt reducers: the outer app owns frame order.

Source order is now closed for ordinary Fired: MainGame390f6a20 calls framework306f61b0,
which calls CSystem365459b0 and Network SyncWithGame1622ae8d before391739e0. Network stage0
post1622a4a4 calls proxy16264c7e?16269b63?actual client listener391e63e0?FIFO drain391e5b60.
Fired0xa2 registration391ec0dd uses immediate=false; wrapper399b6678 invokes391d58a0.
Only after returning does391739e0 update TL manager39173aeb and actors3936a750 at39173bcb.
The latter samples timer0, reads/writes process global3a16e194, and39899e50 subtracts delta.
Targeted verified runtime artifacts are `native-outer-frame-runtime-20261018.json` and
`native-network-{sync-workers,proxy-dispatch}-runtime-20261018.txt` under ignored aa-net data.
Six no-hook original39899e50 Unicorn cases (0,1,999,1000,INT_MAX,UINT_MAX) are embedded tests;
frame-zero/wrap, newly received rows, no query projection and lifecycle clock retention are
also tested. Native outer activity gates still belong to the app, not this explicit tick API.

This does not close generic Fired admission/TL lifetime:39397ff0 ends/decrements rather than
physically erasing a TL; equal-key reconstruction can retain its old node until frame cleanup.
39171800 modes0/1/2 clear ordinary TLs; mode3 skips tree clearing. TeleportUnit391d6ed0 and
NaviTeleport391dc8c0 reach mode3 through3930c9e0. Loading-start callback390f1e80?391721f0
clears actors, but invokes mode0 only when levelInfo.GetType()==2; the type producer and
remaining mode1/2 engine callback identities remain open. No new Fired/GCD timers or guessed
TL replacement/reset events are introduced by the frame-clock change.

### Retained TL references and movie reset correction (2026-10-18)

The primary object at39afe2b8 is constructed by3916ad80 with vtable399b0a90; its embedded
member at+8 has399b0a38. Thus3915b960 is PRIMARY virtual+4, not primary+5c.3916bbd0 registers
the primary object with gEnv+3c movie system virtual+4. Read-only verified runtime setter
340207a0 stores it at movie+8; movie-play3400e42b..445 obtains sequence flags through
sequence virtual+2c and invokes listener virtual+4(sequence,flags). Resetmode2 therefore
comes from this movie-sequence-begin callback, only first nested invocation and flags&8.
It is not a generic world/session reset. Mode1 MainGame virtual+1e8 remains unmapped; a
CryAction executable-section scan found no virtual+1e8 call and does not prove an event.

Original ref helpers393953e0/39395440/393954a0 independently decrement304/308/30c with wrapping
arithmetic. A no-hook Unicorn oracle executes the actual map lookup39394e70 and helpers on
a synthetic one-node native tree: each1?0?ffffffff, root/count unchanged throughout six calls.
The constructor primary reference becomes1 at3939a50d. Visual queue acquisitions391574a4 and
391579ea increment308; their release paths include3915606d. Fired3939b342 increments30c after
projectile setup;393f0b17 decrements it when projectile+2ac==2 before projectile destruction.
These owners cannot be represented by network End/Stop alone.

Physical removal3939a0e0 has one direct caller3939c785 (TL frame cleanup); bulk clear3939a2d0
has one39171966 (reset). Actual3939a0e0/map erase executes in a second oracle: count1?0,
root?sentinel, exact node freed, even when actor resolution returns0. External projectile/effect
cleanup, actor resolver and allocator are explicit mocks in that oracle; tree instructions are
original. Cleanup resolves retained caster ID FRESH at3939a17c before modifying current actor
cast/channel/effect state. A missing/replaced actor neither proves TL removal nor preserves an
old actor pointer. Artifacts: `timeline_reference_oracle.py`, `timeline_erase_oracle.py`, their
`native-timeline-*-oracle-20261018.json` outputs and `native-movie-begin-runtime-20261018.txt`.
Generic Fired production still requires full visual/projectile reference lifetime, retained-node
admission and remaining reset caller closure; no replacement-on-Started or erase-on-End is added.

### Crime reputation and jury-number receipts (2026-10-18)

`CrimeEvent` decodes three native/lane-identical bodies: SC16f `Changed` is i32 points,
i16 crime_points, i32 infamy_points, i16 crime_state (12bytes); SC18a `JuryPointChanged`
and SC182 `JuryWaitingNumber` each contain one i32. All signed extremes and raw points
are retained. Native factories391b2ec0/391b3cd0/391b3850 prove those opcodes; readers are
397b6800/397afe10/397afa10. Lane source is systems/social/packets/trial.rs. There is no
count field, timer conversion, inferred queue position or action acknowledgment in these receipts.

`Event::Crime` forwards every receipt, including duplicates. Plain `Crimes` is integrated
with WorldState and the Bevy resource reducer. It stores actual selected persistent ID,
optional reputation/jury points and one last raw receipt; the forwarded events carry the
complete ordered receipt stream. CharacterSelected hydrates actual header values without
notifications; a matching PlayerSpawn preserves later receipts, a changed actual identity
hydrates its own snapshot. LeftWorld/connection/authentication lifecycle clears this view as
explicit app isolation. Pre-header point receipts remain raw observations without inventing
a zero baseline. Waiting-number receipts produce notices without a persistent native queue
cache claim. Existing Player snapshot fields update only when a player already exists.
`Character.crime_state` now preserves the previously skipped two bytes after infamy points.

`apply_with_notices` replaces `apply` when dispatching Lua callbacks (never fold twice).
Native391dd190 copies reputation into selected header61c/620/624 BEFORE event13c
`CRIME_REPORTED(crimePointDelta,crimeRecordDelta,crimeScoreDelta)`. Subtractions wrap at
16/32/16 bits respectively, then signed values convert to f32; packet `points` is not an
argument. Every duplicate emits three zero deltas.391de280 calls3930afd0 to assign header660,
then emits143 `PLAYER_JURY_POINT()` even unchanged.391ddc20?3907ca10 emits149
`JURY_WAITING_NUMBER(number)` with signed i32?f32, without a cache assignment.
X2Player.GetCrimeInfo39496420 returns ONE table with signed numeric fields `crimePoint`,
`crimeRecord`, `crimeScore`; GetJuryPoint394969c0 returns ONE signed numeric header660
value, or numeric0 if the client pointer is absent. Resource unknown values remain explicit;
the app owns the native absent-client presentation. Event-name registrations39215604/3921566d/392156c7
close the names/IDs. Disassembly artifact: native-crime-reputation-source-20261018.txt.
No-hook local x86 delta oracle crime_delta_oracle.py executes391dd1cb..391dd25d; four
cases verify assignment-before-notice, signed wrap and duplicate zero deltas.
Active trial state, seats, invitations, records, verdicts and outgoing choices remain separate
unimplemented protocol slices; no action success or jury eligibility is fabricated.

The sole outgoing addition is `Command::RequestJuryWaitingNumber { character_id }`.
X2Trial.RequestJuryWaitingNumber binding39078f4d/39078f92 calls39078410 with no Lua args;
its wrapper returns zero values.3907c770 builds opcode079/vtable399a1ae4 and sends39186810;
serializer3901c170 is exactly ret4 (empty body). Lane CSRequestJuryWaitingNumberPacket is
also empty and queries the connection's actual active character. Session requires InWorld,
matching nonzero captured persistent ID and a successful write. These are stronger app stale
identity/transport guards, not additional native jury eligibility. `JuryWaitingNumberRequested`
and `JuryWaitingNumberRequestRejected` are local transport observations, never native/UI ACKs.
Only genuine SC182 can supply the displayed number; no request changes Crimes/cache/notices.


### Received trial wait cache and defendant callback gates (2026-10-18)

`TrialEvent`, `Event::Trial`, and the Bevy/plain `Trials` resource cover native/lane
SC173 SummonJury,175 SummonDefendant,17b TrialWaiting,17c JuryWaiting,17e RulingClosed,
17f AudienceJoined,180 AudienceLeft and183 TrialCanceled. The exact bodies are respectively
(u32 trial,u32 court,i32 jury number), u32 trial, (u32 order,i32 sentence),
(i32 count,i32 total,i32 sentence), empty, (u32 trial,BC object,raw blob name),
(BC object,raw blob name), and u32 trial. All signed fields preserve their original bits;
complete names, duplicates and order survive the forwarded receipt, including embedded NUL
and invalid UTF-8. `last_receipt` is bounded to one raw receipt, not a growing history.
Lane source: systems/social/packets/trial.rs. Native factories391b2fd0/30d0/3450/34d0/
35d0/3750/37d0/38d0 allocate those exact opcodes. Readers397af830/8e0/980/9c0,
39184540,397b6ba0/6c60,397afa30 establish framing independently of the lane writer.
Native audience name buffers cap128; safe cache admission refuses longer names while
retaining the complete raw event. Native malformed overlength behavior is not claimed.

`Trials` records actual PlayerSpawn object identity, optional waiting order/counts/sentence,
and the source-closed `defendant_context`. The follow-up below adds the independently
closed general trial ID/isJury/phase fields. Some(0) is a true defendant flag. Native
manager3907f570 initializes role flags and trial ID to zero but does not initialize the four
wait fields, so absence stays None until a receipt. Actor removal clears actual object identity,
not the defendant flag. Character/session lifecycle resets are stronger app isolation. Native
ruling/cancel/audience resets preserve initialized wait fields.

SC17b handler391dd830 calls3907c6a0/3907c320, stores order at manager+fc, clears counts
+100/+104, writes sentence+108, then emits event141 SHOW_DEPENDANT_WAIT_TRIAL.
SC17c391dd8b0 calls3907c6c0/3907c3f0, always writes counts/sentence, but emits event142
SHOW_DEPENDANT_WAIT_JURY only when manager byte2 is nonzero. `apply_with_notices`
returns those notices once per genuine admitted receipt, even unchanged; use it instead of
apply, not in addition. Convert order as signed i32 then f32, counts as signed i32 then f32,
and sentence bits as unsigned u32 then f32. GetCrimeData390786d0/3907e830 returns
one table whose defaultSentenceTime uses that same unsigned conversion at3907eb00;
order/counts are callback inputs, not fields of that getter. The spelling DEPENDANT is native.

The complete active-defendant gate producers are retained:175 sets byte2 even for trial0;
173 clears it;17e and183 clear unconditionally (183 ignores the received trial ID).
AudienceJoined39080850 clears only for the actual local object and a changed trial;
AudienceLeft3907c940 clears for the actual local object. Native missing-actor sentinel
39ade6a8 was read-only verified zero. Other callers of3907f660, phase3908003e and
crime records390811e7, call only while both role flags are already clear, so they cannot
invalidate an active defendant context. No stale-ID guard is invented for cancellation.

Source and executable evidence live in ignored data/extract/aa-net/native-trial-wait-*
and trial_wait_oracle.py. The oracle executes original3907c320/3907c3f0; only Lua dispatch
is mocked to record arguments and cache state. It verifies UINT_MAX sentence rounds to
4294967296.0f, signed order/count extremes, cache-before-callback and duplicate delivery.
Tests cover every truncation, all eight exact bodies, DD04 batch plus following packet,
actual/foreign/missing object gates, zero context, lifecycle and Session/plugin/WorldState.

Other trial callbacks, invitations/seats/records/verdicts, ReloadUI replay and general trial
getter state remain separate gaps. Native SummonJury additionally sends CS072; the
automatic reply follow-up below implements its bounded handshake without a jury choice.
No timers, eligibility, automatic response or successful outcome is fabricated.


### Trial defendant data, evidence batches and phase receipts (2026-10-18)

The received family additionally covers SC176/177/178 as
TrialEvent::DefendantData(TrialDefendantData), Records(TrialRecords), and
PhaseChanged { trial_id, phase:u8, current_jury:i32, remaining_time_ms:u32 }.
Native factories391b31d0/3150/32d0 use vtables399b3ca8/3cb4/3ccc and readers
397c3940/397c3aa0/397b6a70. Exact lane bodies match the native field sequence.
SC176 is u32 defendant,blob name,u8 race,u32 faction,u32 trial,i32 sentence,BC judge.
SC177 is u32 trial,u32 unknown,i32 total,i32 count then count full evidence rows.
SC178 is u32 trial,u8 phase,i32 current jury,u32 remaining milliseconds.

TrialCrimeRecord preserves id/victim/reporter/faction/expedition/doodad/skill IDs,
raw victim/reporter names, u8 crime kind, original i64 X/Y fixed-point values, f32 Z bits,
raw message and original i64 report timestamp. No DateTime/Unix conversion is performed.
Native record reader397be730 caps names128 and message200 bytes. Native inline packet
holds FIVE rows:397c3b01 selects min(count,5), but callback390811d0 still uses original
count. Complete lane rows therefore parse and remain in forwarded Event/last_receipt;
the native cache safely refuses counts>5 or oversized strings instead of copying invalid
native memory or truncating framing. Count is checked against minimum63-byte row body
before allocation. Negative count preserves its bits and reads no rows, as the native loop.
The lane's reporter_name=None writes no string length and is malformed relative to the
native schema; no heuristic attempts to repair such a body or fabricate an absent name.

Trials now exposes trial_id(default0),is_jury(false),phase(0),remaining_time_ms(0),
current_jury:Option<i32>, defendant:Option<TrialDefendantData>, ordered records and
optional records_received/records_total/records_raw_unknown. These defaults follow
3907f570 exactly; uninitialized header fields stay optional. Existing defendant_context
and wait APIs remain compatible. current_jury can become known before total, so the
pair jury_counts stays unknown until both are known. No elapsed clock is inferred.

Native3907f660 adopts a changed general trial ID, clears role flags/evidence/counters,
invalidates defendant identity and emits UPDATE_INGAME_BEAUTYSHOP_STATUS(), event23c.
The coherent optional defendant projection is invalidated rather than exposing mixed
stale name/race fields after native ID invalidation. Sentence/count wait fields and phase
are retained across this adoption. RulingClosed/Canceled/local AudienceLeft instead reset
roles/trial/phase/raw remaining/judge; they retain evidence and wait fields. App session
isolation still clears all runtime state. AudienceJoined compares actual local object then
adopts the received general trial, including zero. SummonJury establishes is_jury and
SummonDefendant establishes defendant_context; neither uses first-row inference.

SC177 callback391dd590/390812d0/390811d0 adopts trial only while BOTH role flags are
clear; trial0 in that branch returns before batch headers. An active juror/defendant still
appends records even for another raw trial ID. It stores unknown/total, wrapping-adds
count into received count and appends rows in receipt order, retaining duplicates. There
is no invented snapshot-complete or clearing marker. Unknown initial received count
stays unknown until a genuine changed-context reset establishes0.

SC176391dd4f0/3907dea0/3907dd70 IGNORES its received trial ID, stores actual defendant
details/judge/sentence, recounts crime kinds over existing evidence and emits
SHOW_CRIME_RECORDS(current phase), event13d. The juror-only replay3907bec0 then emits
TRIAL_STATUS(phase,current jury),147, and another SHOW_CRIME_RECORDS if general trial
is nonzero. The app omits numeric callback arguments that depend on genuinely unknown
uninitialized native fields. No placeholder totals or sentence are invented.

SC178391dd6b0/390809e0 first emits TRIAL_STATUS(received phase,current jury) BEFORE
39080000 checks context or updates cache. A mismatched active-role trial then refuses
phase/cache updates but retains that first callback. An inactive role adopts the trial.
Admitted phases always store phase/current jury/raw remaining; phase2 additionally emits
SHOW_DEPENDANT_WAIT_JURY only for defendant with known total/sentence, then
SHOW_CRIME_RECORDS. Phases3/4 emit SHOW_CRIME_RECORDS. Phase5 emits SHOW_VERDICTS
only for jurors with known sentence. Notices are returned in this exact source order;
BeautyshopStatus represents the actual event23c name, not a trial-window close.

Verdicts.minutes:[i32;5] contains INTEGRAL minute values for sentence factors
20%,50%,80%,100%,120%. Native39080201..303 converts unsigned sentence to f32, multiplies
each factor then1/60000f32, CVTTSS2SI then CVTSI2SS. The100% branch instead performs
unsigned integer /60000 then CVTSI2SS. Lua receives those integers converted to f32;
the factors themselves are not the callback arguments. UINT_MAX yields
[14316,35791,57266,71582,85899], verified by executed original x86.

Ignored native-trial-phase-data-source-20261018.txt archives readers/callbacks.
trial_phase_oracle.py/native-trial-phase-oracle-20261018.json execute original
390809e0,39080000,3907f660 over42 role/trial/phase cases. Only Lua dispatch recording,
a fixed time import and task-call observations are mocked; no task behavior or real clock
is claimed. Tests embed those callback/cache results, signed extremes, every truncation,
complete lane6-row preservation/native refusal and DD04 following-packet alignment.
Phase-triggered tasks, jury handshakes/choices, records UI formatting/locations, elapsed
trial timers and full GetCrimeData table remain explicit client/protocol gaps.

Crime getter domain refinement: GetCrimeInfo39496420 always dereferences the selected
character HEADER at client+40 and returns its three signed fields; actual world actor
absence does not select zeros. There is no native missing-header return path. A safe
startup empty table is an app accommodation for that undefined native domain, not a
native default. GetJuryPoint394969c0 explicitly returns one0 only if global client pointer
is absent; otherwise it reads header+660 without a separate missing-header guard.


### Received trial counters, ruling and history (2026-10-18)

SC179/17a are two signed i32s (count,total);17d is i32 count,total,u8 sentence type,
i32 sentence bits;181 is u32 player ID followed by ten i32 values. Their native factory
writes391b336b/33eb/356b/326b select vtables399b3cd8/3ce4/3d08/3cc0 and readers
397af900/af940,397b6b10/6970. Lane trial.rs has the same full bodies.
TrialEvent::JuryOkCount/VerdictCount, RulingStatus(TrialRulingStatus) and
History(TrialHistory) retain all fields and reach existing Session/plugin/WorldState folds.

179 handler391dd730 calls3907c660/3907c1a0, stores manager+f4/+f8 BEFORE
JURY_OK_COUNT(count,total), event14c.17a391dd7b0 calls3907c680/3907c260, stores
+110/+114 BEFORE VERDICT_COUNT(count,total),14d. Both arguments are signed i32
converted to f32; repeated receipts emit repeated notices. No equality/role guard exists.

17d stores count/total into the same +110/+114, type+118, sentence+11c and sets byte3true
in3907c6e0, then3907c4f0 emits RULING_STATUS(count,total,type,sentence),13f. Count/total
and the zero-extended u8 type convert to numeric f32; sentence converts UNSIGNEDu32
bits to f32. Native count>=total can also open a localized task12 (with additional defendant
and sentence-type conditions). This receive-only API does not synthesize that task, user
response or result. Even when no task opens, the genuine ruling callback still fires.

181391dd610/3907bb20 ignores the packet player ID and writes ten shared cache stats:
crimePoint,arrest,acceptGuilty,acceptTrial,notGuilty,guilty,totalSeconds,curSeconds,
crimeRecordCount,botReport. It emits no Lua callback and does not reconcile ownership.
TrialHistory.player_id remains raw metadata; do not interpret it as an admitted own identity.
All signed bits remain intact. No cap, nonnegative clamp or success is invented.

Trials exposes separate optional jury_ok_count/jury_ok_total, verdict_counts, ruling,
has_ruling and history. The native constructor does not initialize these fields, including
byte3, so initial unknown stays None. Changed general trial3907f660 initializes ONLY
jury OK count0, verdict count/total0 and byte3false; it retains jury OK total, ruling
sentence/type and history. A coherent ruling struct keeps its count/total synchronized
with subsequent17a updates and changed-context resets. Role resets/ruling close/cancel
retain all these fields; explicit app character/session isolation clears them.

trial_counts_oracle.py/native-trial-count-ruling-history-oracle-20261018.json execute
original3907c1a0/3907c260/3907c6e0+3907c4f0/3907bb20 in local Unicorn. Lua dispatch
only records actual ScriptAnyValue/cache; locale lookup is mocked missing so no modal UI
is executed. Goldens prove assignment-before-dispatch, signed extrema, duplicate delivery,
UINT_MAX sentence4294967296f32 and ignored history player ID/no callback. Tests also
cover exact bytes/every truncation/DD04 following-packet alignment and runtime lifecycle.

Original getter follow-up source is native-trial-original-getters-source-20261018.txt.
GetTrialType registration390790cf calls39078790: manager+8 signed i32 to f32, i.e.
records_raw_unknown, NOT phase or role. GetCrimeRecordsByPage registration39079242
calls390788a0, subtracts1 from page, then390803c0 scans actual append order using
manager+188 page size (changed-context default10); negative internal page returns an
empty table. Full record strings/localized lookups and full GetCrimeData projection
remain client adapter source work, not fabricated from raw IDs.

Trial elapsed time is independent of Battlefield's mapped clock. Native import38ff6f1c
XlGetCurrentFileTime is xlcommon3301ca70 -> MSVCR100 _time64(NULL), despite its name;
38ff6ef4 XlDiffTime ->3301ca80 -> _difftime64. Accepted178 stores local epoch seconds
and expiry=start+(remaining_ms/1000). Frame3907ce50 emits TRIAL_TIMER(phase,periodTable)
only for nonzero expiry and a nonzero truncated second difference from its last update;
it then stores fresh local epoch seconds. The period helper394245f0/39429b20 is the
existing six-key year/month/day/hour/minute/second formatter. No elapsed state/tick API
is added here, and raw phase duration is never treated as a completion ACK.


Trials.crime_kind_counts is an optional [murder,assault,theft,etc] cache. It becomes
known zero on changed-context adoption, and is recomputed ONLY during an admitted176
receipt over current evidence (kind3/2/4/other respectively). Appending177 does not update
it; deriving counts each UI frame would diverge from native receipt ordering.
Original GetCrimeData.total uses records_total, not the sum of kind counters.
totalRecords selects history.crime_record_count when trial type(records_raw_unknown)0,
otherwise history.bot_report. Details race/faction getter outputs are localized strings.

Read-only original PID48924 with verified StandMode module marker established faction-name
threshold39ac373c=100 and invalid expedition39ade6c4=0; see
native-trial-name-threshold-runtime-20261018.json. Both details faction and record column6
only ask392f8120(id,true) when ID>=100. Record6 prefers nonzero victim expedition ID,
otherwise victim faction. This threshold is distinct from guild creation's1000 constant.
GetCrimeRecordsByPage column1 uses XlI32toA and is a signed decimal ID STRING, not numeric
u32. Columns2/3/4/5 are reporter string/kind numeric/victim string/raw report message.
Column10 calls3907e6e0/XlGetLocalTime then localized category102 year_to_minute with
five decimal strings (year+1900,month+1,day,hour,minute). It is local calendar time, not
UTC/ISO or an invented fixed format. Invalid timestamp behavior and column9 position
localization remain adapter boundaries; raw packet fields are preserved independently.


### Original trial getter execution and static/spatial boundaries (2026-10-18)

trial_crime_data_oracle.py executes3907e830 and records all19 typed keys in native order:
isJury BOOL; defendant/race/faction STRING; murder,assault,theft,etc,total,
defaultSentenceTime,crimePoint,arrest,acceptGuilty,acceptTrial,notGuilty,guilty,
totalSeconds,curSeconds,totalRecords numeric f32. All numeric fields use signed i32
conversion except unsigned defaultSentenceTime. Defendant name is first-NUL terminated.
The oracle mocks script table insertion and labeled race/faction static lookups; actual
getter instructions/conversions execute. It supplies read-only verified threshold100.
Its JSON records trial types0/1/UINT_MAX and signed extrema independently of Rust code.

Race3907e640 calls395701e0: IDs0..8 map none,nuian,fairy,dwarf,elf,hariharan,ferre,
returned,warborn; other IDs map literal invalid char_race. Localize category47 using
that key into initially empty std::string. The lookup result bool is ignored: missing
translation has NO raw-key fallback and yields an empty race string. XlFormatString
then copies into a16-byte static buffer39ade594. Normal en_us race names fit; arbitrary
UTF-8/overlong localized-byte truncation is not claimed closed by the lookup oracle.

trial_record_page_oracle.py executes390803c0 over17 synthetic records. Internal zero-based
pages -1,0,1,2 return0,10,7,0 rows in insertion order. Every row has exactly ten entries:
1 signed decimal ID string;2 reporter string;3 kind numeric f32;4 victim string;
5 report message;6 faction string;7 doodad descriptor name(+3c via3963b290);
8 skill descriptor name(+104 via396f6390);9 position string;10 report-time string.
Script interfaces, memset/ID formatter and explicitly labeled static/location/date
lookups are mocked; native selection/order/index/type assembly executes. This does not
claim localization/calendar mocks as native formatter goldens. Preserve native7/8 even
where pseudo-decompiled consumer local names appear reversed.

Position3907f940 first3961de30 subtracts origin3a5e1080 and converts signed fixed44-bit
XY to f32 (scale2^-44), subtracting origin.z. Actual current-world geometry391eef00
finds subzone ID ->3976ade0 sub_zones.name. A separate main_world layer point query
finds zone ->395779f0 (zone_key index), then zone display text/group, zone_groups target,
world_groups name localized kind92 with raw fallback. Complete output is
subzone >> zone >> worldgroup; missing subzone gives zone >> worldgroup; missing zone
gives worldgroup only. This path does not skip none/main_world like expedition position.
If worldgroup lookup fails, native stack buffer is uninitialized: refuse an unknown safe
projection rather than inventing a location. Received records have no zone field; full
rendering needs the actual spatial query, not an ownership/proximity/protocol heuristic.
Date3907e6e0 uses XlGetLocalTime -> xlcommon3301ca30 -> _localtime64_s, then category102
key year_to_minute with decimal strings year/month/day/hour/minute. Invalid CRT date
outputs and long locale truncation remain explicit formatter boundaries.


### Genuine SummonJury automatic CS072 reply (2026-10-18)

Native39080900 calls UI-data helper39508950(11), calls3907f660 with the received trial, stores court/seat,
sets its transient byte0 and immediately consumes it at39080931..46. It constructs
opcode072/vtable399a1aa8 at39080952..65, constructor397ac160 copies trial/court/seat,
then sends39186810. Serializer397b0e80 is exactly u32 trial+i32 court+i32 jury number.
There is no user choice, nonzero-trial/range/level/court validation or duplicate suppression
in this path. The final jury role flags update independently of sender success.
Native raw u32 court bits from SC173 are copied to the signed wire field unchanged.

ClientMsg::JurySummoned has those three exact fields. Session forwards each genuine
TrialEvent::SummonJury unchanged, then sends the automatic response only in InWorld
with nonzero selected persistent ID and matching actual selected character header.
These are stronger app stale-session guards, not native jury eligibility rules. No public
Lua choice/Command is added. Every genuine duplicate173 sends its own response;
trial0 and all signed court/seat bit patterns remain valid raw inputs.

JurySummonReplyRequested and JurySummonReplyRejected retain captured character ID and
raw summon fields. Requested means an actual transport write only, never invitation
acceptance, seating, verdict or authoritative ACK. Rejected also covers inactive/stale
header/missing connection/failed write. The raw summon is still forwarded on all those
paths. No receipt cache is changed by either local transport observation, no retry timer
is introduced, and no server response is fabricated.

The active lane CSJurySummonedPacket reader matches the12-byte schema, but its handler
only logs the receipt and performs no seating/progress transition. This is an explicit
server boundary; the client does not synthesize SC174 or advance trial state afterward.
Tests cover full native/lane wire golden, real loopback duplicate writes, zero/raw extrema,
actual header refusal/no bytes, missing/failed connections and raw event preservation.

The executable jury_summon_reply_oracle.py records eight same/new-trial, duplicate and
sender-return0/1 cases through original39080900+3907f660+397ac160. Only task removal,
Lua dispatch and the sender are mocked. Final received roles are independent of sender
return. A changed-context BeautyshopStatus callback precedes role assignment, while a
same-trial existing defendant flag clears after the send (no intervening Lua callback).
Future UI dispatch should preserve the callback-time pre-role view for that23c notice;
local transport observations are not replacements for any native Lua callback.


### Callback-time trial getter views (2026-10-18)

Trials::apply_with_dispatch(&Event) returns (changed, Vec<(TrialNotice, Trials)>).
Each owned snapshot is captured at the actual native callback point while the canonical
resource finishes folding the receipt. Present a snapshot only during that Lua callback;
do not replace final backend state or interpret a snapshot as a new wire receipt.
apply() and apply_with_notices() use the same private fold without collecting snapshots;
unrelated events return before cloning evidence. No extra temporal mode/framework exists.

TRIAL_STATUS snapshots precede phase admission/mutation, even when an active-role trial
mismatch later refuses the phase. Changed-context UPDATE_INGAME_BEAUTYSHOP_STATUS
sees adopted trial ID/cleared roles/evidence/counters, with old retained phase/wait data,
BEFORE SummonJury/SummonDefendant establishes its final role. Records adoption likewise
emits this callback before new record-type/header/rows are written. Its GetTrialType and
GetCrimeData.totalRecords branch can therefore differ from the final cache. Later wait,
record and verdict callbacks see their assigned native fields. No callback is reconstructed
by merely replaying final state across a notice list.

Tests query source-equivalent GetCrimeData role/first-NUL name/unsigned sentence/kind
counts/totalRecords and ordered record IDs from each view: old Status -> adopted
Beautyshop -> updated ShowCrimeRecords; wrong-trial active refusal; same-trial wait;
pre-role jury transition; records pre-header transition; duplicate method equivalence and
owned snapshot isolation. Native executable phase/summon/getter oracles provide the
independent callback/cache contracts. Lua integration and reentrant action admission
remain aa-client responsibilities; these snapshots supply factual getter inputs only.


Shared native period correction from executed original394245f0 (NO hooked callees):
the year divisor is0x1da9c00=31,104,000 seconds=360days, not365. Months are30days.
MSVC helper398b5810 divides unsigned64; each quotient's low32 bits becomes a signed
field, and subtraction occurs only when that stored signed field ispositive. Thus raw
negative i64 inputs are not a conventional negative period: -1bits yields
{year361130748,month1,day11,hour7,minute0,second15}. Caller-specific clamping must remain
separate. period_time_oracle.py/native-period-time-oracle-20261018.json archive executed
positive/zero/negative cases and the360-year boundary. Prior365-day wording is corrected;
this source milestone still adds no trial timer behavior.


### Original trial confirmation and cancellation controls

TrialCommand::ConfirmCrimeRecords { trial_id, jury_number } writes CS073 u32/i32;
CancelTrial { trial_id } writes CS074 u32. Original zero-argument Lua wrappers
39078390/390783d0 call3907c8a0/3907c8f0 and return zero values. Confirmation checks only
native jury-role byte+1; cancellation checks only current-jury dword+100==0. There is
no native phase, defendant, nonzero-trial, or positive-seat requirement. Neither sender
updates the cache. Native constructors397ac180/397ac1a0 and packet serializers agree
with the active lane's actual jury_end_testimony/cancel_trial handlers.

Trials now retains received SummonJury court and signed jury_number after its changed-
context Beautyshop callback. Those fields survive native context/role resets; application
session isolation clears them. Command::Trial captures the selected persistent character
ID and native context. Session checks active InWorld/header identity, matching trial and
seat, and the native weak gate at actual send time. These stale-identity checks are
stronger application guards. Unknown current_jury refuses cancellation rather than
inventing zero. Duplicate valid requests remain possible. TrialRequested and
TrialRequestRejected are local transport observations only, never completion ACKs or
optimistic state changes; failed/missing writes reject explicitly.

Ignored native-trial-controls-source-20261018.txt and trial_controls_oracle.py /
native-trial-controls-oracle-20261018.json execute the original control bodies and
constructors locally, with only socket sender39186810 mocked. All twelve role/count
cases preserve cache bytes, including zero trial and negative seat. Golden bodies/full
frames plus real loopback session tests cover duplicate sends, unknown count, actual
waiting admission, stale character/context/seat, lifecycle and failed transport.


### Original verdict selection CS075

TrialCommand::ChooseVerdict { trial_id, jury_number, choice:u8 } captures the same
received context and seat, writes u32/i32/u8, and uses the existing guarded Session
transport path. Native390783b0 returns zero Lua results; its common39078ab0 adapter
accepts ScriptAnyValue type4 only and CVTTSS2SI truncates the numeric f32 argument.
3907c720 admits only unsigned(choice-1)<=5, so1..6; it does not require jury role,
phase, defendant status, or nonzero trial. Packet constructor397ac1b0 stores a dword
choice; serializer397ba580 emits its low byte. Active lane CSJuryVerdictPacket agrees
and invokes the real jury_verdict handler. No local cache changes or successful-verdict
receipt is fabricated. Known matching trial/seat and selected header remain stronger
application stale-context guards; unknown seat refuses safely.

native-trial-verdict-source-20261018.txt and trial_verdict_oracle.py /
native-trial-verdict-oracle-20261018.json execute original3907c720/397ac1b0 locally
with only sender mocked:16 cases cover roles0/1, choice0/1/2/5/6/7/MAX/INTMIN,
trial0 and negative seat; all cache bytes remain unchanged. Protocol tests retain
raw choice byte, native admitted range, full frame, stale/unknown seat/context;
loopback tests send duplicate valid requests and reject choice0 without optimism.


### Native trial close and audience callback snapshots

TrialNotice now additionally forwards AudienceJoined { name }, AudienceLeft { name },
RulingClosed, Closed, and Canceled. Names preserve raw bytes; the Lua adapter applies
native C-string first-NUL conversion. This necessarily makes TrialNotice Clone rather
than Copy. Audience safe-cache admission still refuses names longer than128bytes while
retaining the full raw Event and last_receipt; native oversized inline-buffer behavior
is not defined as a safe contract.

Original39080850 emits145 AUDIENCE_JOINED(name) before comparing actual local object
and adopting a changed trial (23c Beautyshop callback). Original3907c940, for matching
local object only, emits148 TRIAL_CLOSED before resetting if old trial!=0, then reset
23c, then146 AUDIENCE_LEFT(name). Nonlocal receipts emit only the named callback;
local trial0 still resets/emits23c but no148. Missing-local sentinel0 behavior is
unchanged from the received cache slice.

Original3907bb90 resets before23c, closes native tasks6/7, then emits140 RULING_CLOSED
and148 TRIAL_CLOSED.3907bc30 resets before23c/tasks and emits148 then14a TRIAL_CANCELED;
it ignores the packet trial ID and does not require active role or nonzero context.
These notices add no task operations or clock implementation. apply_with_dispatch
captures the old context at pre-reset callbacks and reset context afterward; ordinary
apply/apply_with_notices share the same fold without extra snapshot copies.

trial_close_audience_oracle.py/native-trial-close-audience-oracle-20261018.json execute
those four original bodies and actual reset/adopt helpers locally across16 local/nonlocal
and zero/nonzero-context cases. Only Lua dispatch, task lookup/close/removal, and current
Unix clock are mocked; no network/process execution. Tests verify callback order,
getter evidence/context at every boundary, nonlocal repeated notices, unconditional
wrong-ID cancellation, and preserved oversized raw receipts.


### Explicit native trial Unix clock

Trials exposes last_epoch_seconds and expires_at_epoch_seconds as Option<i64>.
Constructor expiry is known0 (disabled); the native constructor samples local time,
so last is unknown until an explicit sample is supplied. apply_with_time and the
apply_with_notices_at/apply_with_dispatch_at variants accept Option<i64> sampled Unix
seconds. An admitted phase sets last=sample, expiry=sample.wrapping_add(u32ms/1000)
before its inner notices. Unknown sample marks both unknown, without guessing a start.
Wrong active-role trial leaves clocks unchanged; changed-context adoption retains them.
Actual role reset sets expiry0 and last=the supplied sample before Beautyshop callback.
Application session isolation restores constructor-shaped unknown/disabled state.

advance_time(epoch_seconds) returns optional TrialNotice::Timer { phase,
remaining_seconds } after assigning last, so callback snapshots see the fresh value.
It does nothing for unknown/zero expiry or unknown last. Actual MSVCR100 _difftime64
returns0 if either input is negative; otherwise it subtracts integer64 first and FILDs
the result. Original398b5706 converts directly from x87 without a f64 store, retaining
exact valid integer differences above2^53. If now/last is negative or unchanged, no
callback occurs. A negative expiry with otherwise valid elapsed time yields remaining0.
Elapsed expiry remains enabled; negative remaining periods are not clamped or reset.

Original queued outer frame39173ab9 first calls client packet pump391e72b0, then TL
frame3938edb0, then trial tick3907ce50 at39173b1f. Trial phase39080056 samples
XlGetCurrentFileTime, whose actual implementation is _time64 Unix SECONDS, unrelated
to battlefield remote time or Windows FILETIME ticks.3907cf5b/61 updates last BEFORE
period construction and14b TRIAL_TIMER(phase,periodTable). The root HUD bridge owns
explicit receipt/frame SystemTime samples; WorldState/plugin plain apply deliberately
introduces no hidden clock, so other consumers must use the clocked API themselves.

trial_timer_oracle.py/native-trial-timer-oracle-20261018.json execute original frame,
conversion and period bodies plus actual MSVCR100 difftime across13 cases. Read-only
PID48924 verification confirms the CRT bytes match the local SysWOW64 image; only
clock return, errno pointer, Lua/table interfaces are mocked. Separate phase-clock
oracle executes20 cases including0/negative/MAX epochs, unsigned durationMAX, floor
milliseconds, addition wrap and zero expiry. Embedded tests cover both oracles,
unknown sampling, reset/adoption/lifecycle, stale-ID refusal and callback-time fields.


### Initial trial tasks and exact response packets

TrialEvent adds ImprisonOrTrialOffered (SC171:twoi32), JuryInvited (SC172:raw blob,
u32trial), and JurySeated (SC174:raw west byte,u32trial,i32court,i32jury). Native factories
391b3650/391b36d0/391b3050 and serializers397af770/397af7b0/397af880 agree with active
lane framing. Lane names crime_value u32, but native i32 accessor and Task7 signed-f32
conversion establish the signed projection with unchanged bits. Names retain full binary
bytes; native Task6 input cap128 is an app admission boundary, not framing truncation.
These receipts update last_receipt only: no inferred jury role, seat, current trial,
acceptance, or automatic response. Native174 handler3907bcc0 ignores trial, requires
client and unsigned seat<=4, resolves a static west/court/seat actor then requests its
interaction; it does not assign trial-manager membership. Rendering/planning stays open.

TrialCommand adds ReplyImprisonOrTrial { request_trial } CS06f bool,
ReplyJuryInvitation { accept,trial_id } CS071 bool/u32, and SkipFinalStatement { trial_id }
CS070 u32. These reuse selected-header Session transport guards/local outcomes, without
requiring the manager's current trial to equal a captured task trial. Root owns genuine
task token/lifecycle/admission and source world gate. Native true Task8 choice sends;
false/ESC sends no CS070. Root must not queue a skip command for cancellation. No received
state changes on any transport observation. All three lane handlers perform real trial
operations. Source serializers397b0e00/20/40 establish the exact1/4/5byte bodies.

Task7 creation3907d690 first refuses an existing Task7 silently. vt399a1b34 returns
buttonType0,isModaltrue, info crimeValue/jailMinutes signed i32?f32. Task8 vt399a1b54
is buttonType0/isModaltrue, captures raw phase packet trial, and supplies localized
title/category28 and content/category29 using skip_final_statement. Task6 vt399a1b74
is buttonType0/isModalfalse, captures defendantName (CString) and trial, and admits only
if no queued6 and game option index4!=1.390fa85e binds index4/store+334 to
 g_ignore_jury_invite default0;39182860 compares exactly1. Generic admission refusal
3902d570 invokes false callback for buttonType0, hence sends a jury decline. Common
Cancel3902b2f0 likewise calls false: Task7 sends CS06f(false), Task6 CS071(false),
Task8 sends nothing. Task manager/editor suppression branches remain root lifecycle scope.

Native Task6 true reply3907c820 queries ISystem virtual144?CrySystem3654d650?object+ac;
its virtual8=365f2170 returns byte10. Producer365efa10 sets this byte iff client env
byte150 and current loaded layer name exactly main_world; constructorzero. False
forces decline and emits local error544 CANNOT_JOIN_TRIAL_FROM_INSTANT_ZONE before
CS071. This is not Battlefield membership. Unknown app world context must refuse safely;
no true-by-default gate is supplied in aa-net. Original callbacks/constructors execute
in trial_initial_choice_oracle.py/native-trial-initial-choice-oracle-20261018.json,
12cases with main_world query/error/sender mocked only; native forced decline and no
cache mutations are independently observed. Read-only runtime method-pointer proof and
full source are archived in native-trial-jury-accept-runtime-20261018.txt and
native-trial-initial-tasks-source-20261018.txt.

TrialNotice::SkipFinalStatementTask { trial_id } and CloseSkipFinalStatementTask are
explicit task actions, NOT Lua events. Original outer390809e0 runs them after inner
phase callbacks/admission, only while defendant byte2 is true. Phase4 captures raw
packet trial even if an active wrong-trial inner update was refused. Phase5 checks
actual queued8 before closing; root must retain that task-manager guard. Tests exercise
pre/post snapshots, wrong-ID refusal followed by task request, raw receipts/truncation/
DD04 following-packet alignment, native weak command gates, and real loopback transport
with no automatic reply or optimistic state mutation.


Task closure precision: TrialNotice::CloseTrialTasks is an explicit non-Lua request to
close native Tasks6/7 only, emitted after reset/Beautyshop and before terminal ruling/
closed/canceled notices. Context adoption does not close them. Neither173 nor175 closes
Task7: both call39508950(11) before adoption. Earlier 'remove task11' wording was an
incorrect mock label, now corrected.39508950 writes UI manager3a5bb120, with?6 sentinel
skipping active edit update; downstream395107d0 restores chat input method (method11
is Trial), independently source-closed by aa-ui. RulingClosed calls?6 before reset;
local AudienceLeft calls?6 before old-context Closed/reset. Canceled and AudienceJoined
do not call it. This milestone does not invent a Task11 or synthesize chat behavior.


### Executed task removal callbacks and jury main-world flag producer

CloseTrialTasks means the actual native3902cef0 removal path, not no-choice closure.
3902cef0 pushes fourth argument0 into3902ce30; the latter invokes virtual18 with
bool(buttonType==1) BEFORE dialog/node removal. Trial Tasks6/7 have buttonType0, so
if genuinely queued, close6 sends captured CS071(false,trial), then close7 sends
CS06f(false). Native3902cf10 instead passes fourth argument1 and suppresses callback;
trial reset handlers do not use that variant. Root must not invalidate both tokens
before executing the native close callbacks. An absent task sends nothing.

trial_task_close_oracle.py/native-trial-task-close-oracle-20261018.json execute actual
3907bb90/3907bc30,3902cef0/3902cf10/3902ce30,predicate,Task6/7callbacks and packet
constructors against a synthetic two-node pending list. Only UI chat mode, clock,
Lua/visual close, free allocator and sender are mocked. Observed reset order is
Beautyshop(state0), CS071(false,captured999), remove6, CS06f(false), remove7, then
ruling/closed or closed/canceled events. Directcef0 emits decline; cf10 emits none.
3902db50 is the cdecl manager singleton, not a type-consuming lookup; preceding pushed
6/7 remains on stack for the following method. Older mock labels did not establish
no-choice behavior and must not be used for that conclusion.

The jury-accept gate's gEnv byte150 now has a concrete original producer. Original
X2:ConnectToServer39440bd0, registered39447daa as ConnectToServer(serverIP,userId,password),
sets gEnv+96hex to1 at39440c1f BEFORE session393d4860 starts its connection. Original
login.lua241 calls this method. X2Login:OnEndLoginState39484cb0, registration39489ddb,
also sets1 at39484d6b after its GAME_RATING_SHOW path. A read-only module-verified
PID48924 observation sees1. The root adapter may track that actual login action,
separately from the currently loaded engine layer main_world; it is not an InWorld,
received feature bit, or faction inference. Unknown producer/world context still
refuses acceptance safely; decline remains supported. The nominal global flag name
is not required to preserve its concrete producer/consumer contract.


### Received bounty bulletin and settlement boundary (2026-10-18)

Native factories/serializers prove SC0bb (397b3b50), SC1d9 (397c4720),
SC1da (397c6fc0), SC1db (397c7080), and SC1dc (397c7180). Active lane
`systems/social/packets/trial.rs` writes matching framing; `crime.rs::BountyDescription`
writes matching row widths. One semantic mismatch is preserved explicitly: native row
397bebf0 reads not-guilty then guilty, while the lane writes guilty then not-guilty.
`BountyRow.first_verdict_count` / `second_verdict_count` retain those actual wire positions.
No field swap, payment request, reward credit, or optimistic settlement is synthesized.
Names remain binary, positions retain i64 coordinates and raw f32 z bits, amounts retain
all 64 bits, and bool/race bytes and signed counters remain raw.

`Event::Bounty` forwards every complete receipt, including duplicates. `Bounties` keeps
only `last_receipt` plus the native ordered unsigned persistent-ID map. Native39080b00
assigns each row in order, so the last duplicate ID wins; chunks do not clear earlier
rows. After all rows, equality of packet total and map size emits
`TOGGLE_BOUNTY_BULLETIN()` (0xd0), including repeated/empty matching receipts.
The native inline reader stores at most twenty rows but its callback trusts the original
count: behavior above twenty is unsafe, not a valid native truncation contract. Our
reader consumes every lane-declared row for alignment and preserves the raw event;
the safe cache refuses count>20, inconsistent row counts, or names>128 bytes atomically.
Names within the admitted cache preserve NUL tails; Lua conversion uses first NUL.

The native map39ade6cc has static construction/destruction and no character/world-leave
clear in its direct-reference coverage. Accordingly CharacterSelected and LeftWorld
retain the map here. Authentication/connection, successful login, disconnect and kick
clear it as explicit app transport isolation, not a claim of native cache reset equivalence.
`apply_with_notices` folds once and returns callbacks after assignment; these handlers
have no intermediate map callbacks requiring staged copies.

SC1da false emits native local error584, true emits `SET_BOUNTY(kind,name,money)`
(0x14e). SC1db nonzero signed i16 error emits that error; zero emits
`SET_BOUNTY_DONE(kind,name,money,1)` (0x14f). SC1dc emits
`BOUNTY_PAID(hunter,arrested,money,1)` (0x150). Notices preserve full received amounts;
`Bounties::displayed_money` separately reproduces native **signed low32 to f32** conversion.
Original executed getter39080c00 confirms 0x100000001 displays1 and0x80000000 displays
-2147483648. These callbacks do not prove wallet changes or update the bulletin map.

SC0bb handler391d6570 resolves an existing actor, writes its i64 bounty at+3ad8 via
39354260/390b63d0, updates its label, then emits the shared0x14f callback with runtime
objectID, actor name, signed-low32 money, and numeric local0/1. `Units` and `Player`
update only actual matching existing actors; no actor is created. `Unit.bounty_money`
is None until this report, and actor replacement starts unknown again. The separate
`Bounties::unit_money_notice` requires the actual matching actor already holding the
received amount; callers supply actual local object identity. It does not maintain a
second actor cache or infer local identity from persistent IDs.

CharacterState39620856..d3 proves the previously skipped `jail_seconds:i32`,
`bounty_money:i64`, and `bounty_time:u64`. They are now retained without changing
framing. Native39089e33 copies header bounty money into the selected-character object;
SC0bb updates the runtime actor independently. No cross-hydration is inferred. Native
time units/epoch remain unclosed, and the current lane character-state writer sends
zero for all three fields rather than rehydrating live escrow.

`X2Trial:GetBountyData` registration39079137, wrapper39078810?39080c00 returns
(table,count), ordered by unsigned persistent ID. Its fifteen row columns are ID,
name, level, zoneID, low32-signed money, crime points, position string, race string,
faction string, arrest count, accepted-guilty count, accepted-trial count,
not-guilty count, guilty count, seconds. Position3907f940 uses actual current-world
spatial lookup, race3907e640 uses locale47, faction392f8120 requires factionID>=100;
these providers must not be replaced by invented received-zone/proximity mappings.
Source: ignored `native-bounty-source-20261018.txt`; actual local x86 getter oracle
`bounty_get_data_oracle.py` / `native-bounty-get-data-oracle-20261018.json`.
The oracle executes the original getter and ordered-map iteration; static spatial,
race, faction and Script interface providers are explicitly mocked/labeled.

QA: all501 aa-net Bevy library tests pass (0failed/0ignored); the final signed-header
edge assertion also passes independently. Regression coverage includes all five packet
bodies, every truncated prefix, DD04 following-packet alignment, duplicate ordered-map
receipts, malformed-cap refusal, transport isolation versus world-leave retention,
existing/local actor guards, reused actor IDs, unchanged selected header, Bevy folding,
and duplicate Session forwarding without automatic payments.


### Explicit original bounty request transport (2026-10-18)

`BountyCommand::RequestSetMoney{name}` writes CS11f blob(name), original
RequestSetBountyMoney39078470?3907cce0, ctor397accb0, serializer397b13a0.
`BountyCommand::Update{kind,name,money:i32}` writes CS120 u32/blob/i32, original
SendBountyUpdate390784b0?3907cd30, ctor397accd0, serializer397bc6b0. Native money
is signed32 here, despite signed64 received settlement amounts. Native wrappers return
zero Lua results and perform no role, trial, phase, cash, or pending prechecks before
sending. Strict adapters39078b60/39078960 require actual string/numeric types;
39058d10 converts money using CVTTSS2SI, as for kind (then u32 bitcast). The
`update_from_lua` numeric adapter mirrors NaN/out-of-range?INTMIN and truncation,
not Rust's saturating float cast. The caller still validates actual Lua argument types.
Names use first-NUL C-string semantics, preserve arbitrary bytes, and are safely refused
above128 prefix bytes as an explicit app guard around the native129-byte copy buffer.

`Command::Bounty{character_id,command}` additionally requires InWorld, nonzero captured
persistent identity, matching selected ID, and a genuine matching selected header.
Missing/failed sends emit local BountyRequestRejected; a successful transport write
emits local BountyRequested only. Neither is a native Lua event, server ACK, permission,
wallet update, or settlement. No command is automatically invoked from received data.
The current bevy lane has no CS11f/120 registration/handler: these source-backed requests
will not complete there until that separate server gap is addressed. CS0ea navigation
exists but currently returns only an empty bulletin. No server change or live payment
was used for validation.

Ignored `native-bounty-outgoing-source-20261018.txt` records the exact native bodies;
`bounty_outgoing_oracle.py` / `native-bounty-outgoing-oracle-20261018.json` execute
original send helpers and constructors locally (six cases). Imported bounded C-string
copy and socket sender are explicitly mocked; there is no process injection. They
confirm first-NUL and non-UTF8 names and untouched kindMAX/moneyINTMIN bits. Wire
serialization goldens and local TCP loopback cover transport only, not lane success.

QA: cargo check passes; nine focused bounty tests and all504 Bevy library tests pass
(0failed/0ignored). Loopback requests use only an ephemeral local TCP peer and verify
both exact bodies, stale captured/header identity, phase/name refusal, missing/failed
writes, and absence of synthesized bounty receipts or selected-header changes.


### Native bulletin interaction request (2026-10-18)

`BountyCommand::OpenBulletin{doodad_obj_id}` writes CS0ea with a three-byte BC object
ID, native handler393b1140?constructor397ac8c0, vtable399d2c84 reader397bbb50.
Unlike CS11f/120, the active lane registers CSNaviOpenBountyPacket; it currently logs
and returns only an empty SC1d9 list, without populating real bulletin rows.

Native interaction dispatcher393ba9c0 looks up a special handler before ordinary
fallback (callers393b40df..ee and393b76e6..f8). The final registration393baffc..b021
maps runtime global39ac7908 to singleton39ac7920, vtable399d3114[0]=393b7bc0.
That delegates393b1a90, which resolves an actual existing doodad through39035110 and
returns if missing; its separate handler tree maps39ac7788 to39ac77a0, whose vtable
399d2c68[0]=393b1140 reads doodad+7c (runtime objectID) and sends CS0ea. Module-verified
PROCESS_VM_READ of original PID48924 proves both key globals equal20558; the static
skills row20558 is the bounty-bulletin-view skill. This is an interaction-dispatch path,
**not a received SkillFired producer**. Earlier interaction eligibility remains outside
this packet helper; no automatic route, invented range check or always-ready board
predicate is introduced.

The typed request uses the existing actual selected-header transport guards and local
Requested/Rejected observations. Its additional nonzero24-bit object-ID refusal is an
explicit app guard: the inner native handler itself does not reject zero. The caller
must capture/admit the actual current doodad interaction before requesting. The three
new local Unicorn cases execute original393b1140/397ac8c0 against synthetic doodad
objects (sender mocked), confirming0/0x123456/0xffffff unchanged at the packet constructor.
The packet writer then emits only the low24 bits, as separately proven by397bbb50.
Wire tests and the existing local TCP test include the third actual request and rejection
of out-of-range identity without fake bulletin/settlement receipts.

Ignored provenance: native-bounty-navigation-source-20261018.txt,
native-bounty-navigation-runtime-20261018.txt, read_bounty_navigation.py, and the final
three cases in native-bounty-outgoing-oracle-20261018.json. The latter extends the prior
six original bounty-control cases to nine; imported string copy and sender mocks remain
explicit. Runtime module marker and process image were verified before readonly reads.

QA: cargo check, all10 focused bounty tests and all505 Bevy library tests pass
(0failed/0ignored). The local TCP test sends all three bounty commands and confirms
invalid bulletin object IDs produce no wire write or fabricated bulletin receipt.


### Original ranking received foundation (2026-10-18)

The active bevy lane declares ranking_packets/manager/runtime and registers its request
handlers (the packet module's old introductory 'not declared' comment is stale).
Native SC1f9 factory391b75b0?vtable399b42f4?397b89d0 reads u32 rankID;
SC1fa391b7630?399b4300?397c4820?3961f360/3961f1a0 reads outertype u32,
error i16, raw time64, count i32 and personal rows. SC1fb391b7700?399b430c?
397c48b0?3961f3b0/3961f2c0 reads rankID u32, error i16, boardID i64,
two raw time64 fields, signed entry count/rows and signed scope count/rows.
SC205391b7c50?399b4384?397c4940 reads rankID u32 and enum u8. Current lane
writers agree on these exact widths/order. Timestamp bits are retained as raw u64;
no local timezone or clock projection is assumed by this received cache.

Personal rows retain accountID u32, binary account string, characterID/rankID u32,
worldID u8, signed v1/v2, two raw sum bytes, and raw recorded time64. Snapshot entries
retain characterID/rankID, worldID, signed v1/v2, raw updated time64 and signed ranking;
scopes retain signed scopeID/v1/v2. All raw counts, flags, names, duplicates and order
remain in Event::Ranking and bounded last_receipt. Count feasibility is checked against
remaining bytes before allocating, including deliberately impossible i32MAX counts.
Negative native signed counts consume zero rows, as native signed loops do.

The reader consumes full lane-declared framing; native inline readers cap personal
storage at16 and snapshot entries/scopes at100, yet their callbacks trust the original
counts. Out-of-cap malformed inputs therefore have unsafe/non-equivalent native behavior,
not a valid truncation contract. The safe cache atomically refuses positive over-cap
counts, mismatched vectors or personal account names exceeding16 bytes; complete raw
receipts remain observable. Names within accepted caches preserve NUL tails for the
later first-NUL Lua boundary.

`Rankings.by_id` follows native unsigned board-key ordering. Handler39182340?39076980
ignores outertype for cache keying and assigns each row under its own rankID; last duplicate
wins. Personal assignment only replaces the personal row, preserving an existing board
snapshot. Accepted zero-error receipts assign acknowledgement time and emit
`RANK_PLAYER_RECORDS()` (0x258), even repeated/empty receipts; if the cache is empty,
local error792 precedes that callback. Nonzero signed i16 error silently refuses the
cache/callback. The native reader sign-extends the i16 into a DWORD object member:
this is **not a 32-bit wire overread**, despite the lane packet comment's wording.

Handler39182370?39076df0 accepts a zero-error snapshot only for an already cached
rankID, replaces its ordered entries/scopes (including duplicates), and emits no immediate
Lua event. Native frame39074fb0 emits `RANK_SNAPSHOTS()` (0x259) only when the selected
board exists and every entry's character name resolves through390192e0?39018560.
That provider first checks actual units by persistent characterID, then its separate
name cache/request queue. Account strings and runtime object names must not be invented
as substitutes. Ranking readiness, selected board, name availability, lock/unlock timers,
and static-derived refresh timestamps remain explicit adapter/provider gaps here.

Alarm39182410?39076b80 requires a static rank descriptor397f2950; it selects a static
message string according to raw kind==1 versus other, then emits RANK_ALARM_MSG(0x25c)
with the descriptor's signed kind and that string. RewardMail handler391e58c0 also
requires the static descriptor, formats localized key RANK_REWARD_SENT with its descriptor
name, and only on formatter success emits SYSTEM_MESSAGE(0xfd). RANK_REWARD_SENT is a
localization key, not a Lua event. Both remain complete raw RankingEvent receipts until
the app supplies those actual static/localized projections. No reward/item/mail success,
name resolution or snapshot-ready callback is fabricated by the reducer.

Native reset39076f60 empties the map and UI selection/time/ready globals. A tail-jump
scan closes wrapper39213830, called at39213576 inside destructor39213430; the actual
owner/lifecycle event mapping remains unclosed. The plain resource explicitly isolates on actual
selected persistent identity replacement (CharacterSelected/PlayerSpawn) and transport
login/connect/disconnect/kick boundaries as **app policy**, not native reset equivalence.
A matching PlayerSpawn retains already received data, and LeftWorld retains it. Own
identity is never inferred from any received row. No outgoing ranking requests or automatic
name/reward operations are introduced by this foundation.

Ignored source native-ranking-source-20261018.txt records exact bodies;
ranking_cache_oracle.py / native-ranking-cache-oracle-20261018.json execute original
39076980/39076df0, actual tree lookup and preallocated native vector appends in seven cases.
UI visible-board builder, name-cache/static descriptor lookups and Lua/error sinks are
explicitly mocked. Results: duplicate scores11?22; nonzero errors refuse; personal updates
preserve vectors; known-key snapshots retain ordered IDs42/43 and duplicate scopes5/5;
unknown-key snapshots refuse; snapshot receipts emit no immediate callbacks. New-key
allocation and native reset caller are not claimed by those executed cases.

The seventh executed case proves empty personal-map error792 observes the old
acknowledgement, followed by RANK_PLAYER_RECORDS observing the assigned123.
`Rankings::apply_with_dispatch` therefore supplies ordered (notice,Rankings snapshot)
callback views; plain apply and apply_with_notices share the same fold without deep
cache clones. Use the staged variant when local error presentation can query ranking state.

QA: cargo check passes, seven focused ranking tests pass and all512 Bevy library
tests pass (0failed/0ignored). Coverage includes every truncated prefix, all four
raw bodies and DD04 following-packet alignment, impossible counts before allocation,
atomic cache-cap refusal with complete raw receipt retention, duplicate assignments,
known-key snapshots, callback-time acknowledgement views, actual identity isolation,
matching-spawn/world-leave retention, Bevy folding and ordered Session forwarding
without automatic name queries or rewards.


### Persistent character names on the stream channel (2026-10-18)

`CharacterNames` is a plain received keyed cache, exposed through `WorldState.character_names` and the Bevy resource. `Event::CharacterName(CharacterName { character_id, name })` preserves every raw TC009 receipt, including duplicates, binary bytes, embedded NUL and oversized payloads. The bounded cache accepts a maximum 128-byte name and projects the first NUL as the native CString boundary; an unsafe oversized value remains in `last_receipt` without changing cached names or pending state. There is no unbounded receipt history, inferred name, automatic ranking callback, expiry, or retry.

The active lane stream `c2s.rs` implements CT00a as a persistent character ID u32, and `s2c.rs` TC009 as u32 plus the ordinary byte-length string/blob. A missing server name sends no reply. These are separate from CT009/TC008 UCC names. `Command::QueryCharacterName { character_id, requested_id }` captures the actual selected persistent identity. Session admission requires InWorld, the matching actual selected header, nonzero requested ID, an accepted stream join and no outstanding query for that ID. The raw request contains only opcode00a and requested u32. Local `CharacterNameRequested` / `CharacterNameRequestRejected` report transport outcomes, never a native acknowledgement or invented cache success. Internal pending insertion precedes the write; a failed write removes its new pending entry as explicit application safety. Duplicate rejection does not remove an earlier pending query. Stream loss clears outstanding queries while retaining names; a new transport/login lifecycle clears the cache as application isolation, not a claimed native world-reset rule. Character selection and LeftWorld alone retain it.

Original source: `39018500` inserts the ID in the pending set before constructing opcode00a (vtable3999d000, constructor397ad380) and sending through393d8e90?393d5fe0. TC009 factory393d7770/vtable399d448c, serializer397bcc40 reads u32 and a maximum128-byte string into its129-byte buffer; handler393d5e70 calls39019180. That callback samples `xlcommon::XlGetCurrentFileTime` (IAT38ff6f1c), removes pending, then writes the timestamp and received CString into the keyed cache; it emits no Lua notification. This slice does not manufacture a timestamp from UI/frame time.

Lookup39018560 rejects invalidID39adac90 (module-verified read-only original PID48924 value0), checks pending BEFORE actual in-view actor lookup3935e370, then cached value, otherwise requests the name and returns null. Thus pending can hide even a currently available actor name; this resource deliberately does not pretend a simple cached lookup implements that whole native resolver. The executed original-byte five-case oracle confirms sentinel/pending/actor/cache/miss order with the tree lookup, actor resolver and send boundary explicitly mocked.

Lifetime coverage: executable E8/E9 scans find cache node erase39018990 only in range erase39018f30; its only direct callers are destructor39019280 and static finalizer39992c80. Pending erase39017fc0 is only reached through39018460, including receipt removal and static destruction. Lookup never reads the stored timestamp. No native expiry/retry or CharacterSelected/LeftWorld reset was found in this bounded caller audit; indirect/unidentified callers are not declared impossible. Source and exact caller coverage are in ignored `data/extract/aa-net/native-character-names-source-20261018.txt`; executed lookup provenance/results are `character_name_lookup_oracle.py` and `native-character-name-lookup-oracle-20261018.json`.


### Explicit ranking requests (2026-10-18)

`RankingCommand::{RequestPlayerRecords, RequestSnapshots { rank_id }}` and `Command::Ranking { character_id, command }` send only source-backed CS12f (empty) / CS130 (one u32) through the existing game transport. The active lane implements both handlers in `systems/social/ranking_manager.rs`; unknown board IDs produce no snapshot. Session requires an actual matching selected header in InWorld; snapshots additionally require that board in its received `Rankings` cache bound to the same persistent character. No request changes a received board, acknowledgement time, score, name, or readiness. `RankingRequested` / `RankingRequestRejected` are local transport observations, with failed/missing connection writes rejected explicitly. Captured identity/phase checks are stronger app stale-request protection, not invented native Lua success gates.

Native `RequestPlayerRecords39073cc0` validates a zero-based static tab index against397de490, stores selected tab, constructs vtable399a12ac/opcode12f (empty serializer3901c170), sends, emits RANK_LOCK(25a), then samples Cry timer seconds. Tab validity and local lock/timer policy stay in the app adapter; the wire contains no tab argument. `RequestRankSnapshots39075070` requires an existing selected board cache key, sets its local readiness flag, then sends opcode130 using constructor397acea0/vtable399a12b8. Serializer397c79f0 enters the named `rankType` variant and writes only its u32 `type` member at object+c: the variant selector is not an extra wire byte. This matches the lane's exact four-byte reader and archived original live-body source note. Neither explicit request automatically invokes stream name queries or claims that RANK_SNAPSHOTS can fire before names resolve.

Executed original serializer goldens (empty, u32 zero/one/UINT_MAX) are archived in `data/extract/aa-net/native-ranking-request-oracle-20261018.json` with `ranking_request_oracle.py`. The named archive begin/end and leaf-u32 output interface are explicitly mocked; original branching/member-address instructions execute locally, and no variant selector is synthesized. Source excerpts are `native-ranking-request-source-20261018.txt`.


### Original ranking Lua view contracts and executed frame policy (2026-10-18)

This is source evidence for the future app adapter, not an automatic ranking presentation manager or new packet acknowledgement. The loaded static span397de490 reads GameDB+f6b4/count, +f6c0/ID array and +f6c4/124-byte descriptor array. Loader39836b90 uses `SELECT id, day_of_week_id, ed_day, ed_hour, ed_min, ed_month, ed_year, end_time, end_time_alarm, end_time_alarm_msg, name, rank_kind_id, reset_week, st_day, st_hour, st_min, st_month, st_year, start_time, start_time_alarm, start_time_alarm_msg, v1, v2, zone_group_id FROM ranks` without ORDER BY. Read-only original PID48924, verified image/StandMode module marker, confirms four loaded IDs `[1,2,3,4]`. The native count includes all descriptors, independently of the lane's unsupported racing board.

* GetRankCount39073c20 returns the entire static count as numeric f32. Builder39075ee0 retains all static IDs and a received-board-present bool in loaded order, with native storage16, and sets default to the FIRST present board's ONE-based ordinal; no present board gives0. It can produce default17 with17 descriptors even while retaining only16 tab rows. GetDefaultTabIndex39074520 and GetSelectedTabIndex390745a0 return raw signed globals as f32; reset39076f60 clears both to0. RequestPlayerRecords's >=0/<staticCount admission and unchanged selected assignment are preserved as an observed index asymmetry: personal receipt later interprets selected as ONE-based, subtracts1, and falls back to default when invalid/missing or the selected tab rank ID equals the invalid sentinel. Do not smooth these layers into one inferred index convention.
* GetRankInfo39073d60 takes ONE-based static ordinal and returns exactly `{name:string,v1:string,v2:string}` from descriptor+24/+50/+54. Invalid0/out-of-range gives zero Lua results.
* GetPlayerRecord39075320 uses the selected board and returns signed-f32 v1/v2 plus numeric year/month/day/hour/minute/second. Missing selected board gives zero results.
* GetSnapshot390754a0 returns a table keyed by each raw received ranking value, NOT arrival ordinal; repeated ranking keys overwrite earlier rows. Each row has signed-f32 v1/v2 and the six date keys; name is included only when native39018560 returns a pointer, worldName only when current login-world lookup393ce840 finds that raw world byte. Missing name/world does not omit the row or synthesize text. Existing empty vector yields an empty table; missing selected board yields zero results.
* GetSnapshotScope39075780 returns `{snapshotScopes,rankScopes}`. Received snapshotScopes are ordinal1-based rows `{scope,v1,v2}` with signed-f32 numbers, preserving duplicate scope IDs. Static rankScopes are ordinal1-based authored LINK rows `{name,from,to}`; an unresolved child skips its row without compacting later indices (native oracle yields indices1 and3). Missing selected board or static rank descriptor gives zero results.
* GetSnapshotUpdatedTime39075cd0 returns `{prev:date,next:date}` from native cached+48/+50. On genuine snapshot receipt the next timestamp is recomputed as updated plus1800 seconds when max(0,resolved signed scope_to)<1001, otherwise plus3600, ONLY if raw updated>0 and static descriptor exists; zero updated leaves the previous cached next value intact. This computed cache field is not another wire timestamp.
* GetResetPeriod390750f0 returns `{periodStart:date,periodEnd:date,periodReward:date}` using received reset raw: reset+3600, reset+signed(reset_week)*604800?3600, reset+signed(reset_week)*604800 respectively, with native wrapping64 arithmetic. Missing board/descriptor gives zero results. The executed week2/reset10000 arguments are13600/1216000/1219600.
* GetReward39073f70 returns ordinal1-based authored reward-link rows `{scopeName,scopeFrom,scopeTo,name}` plus optional `appellation={name,buffType}` and `item={type,grade,count}` only when their static descriptors resolve. Grade is unsigned-u8?f32; count signed-i32?f32; type/buffType use unsigned-u32?f32. buffType is the resolved appellation descriptor's first DWORD, not an inferred copy of the reward link ID. Empty links give an empty table. A missing required scope/reward descriptor aborts the WHOLE getter with zero results, including any rows already constructed. No claim/grant is implied.

Name-ready frame39074fb0 runs independently of raw snapshot receipt: when ready flag39ac3310 isfalse and selected board exists, it scans received entries in order, invokes the actual name resolver and stops at FIRST null. An empty entry vector is ready. All names available sets flagtrue BEFORE RANK_SNAPSHOTS(259); a true flag or missing selected board emits nothing. Snapshot admission39076df0 clears ready and invokes name lookup for each received row but does not immediately emit259; RequestRankSnapshots sets readytrue while waiting. A non-null empty CString counts available. Pending-name precedence must be preserved before actor/cache fallback; the received CharacterNames map alone is not that entire native resolver.

Local RANK_LOCK(25a) is emitted by the personal request before it stores the Cry timer's f32 seconds. On each ranking frame, recorded>0 and STRICT freshSeconds>recorded+3 emits RANK_UNLOCK(25b) while the callback still sees the old timestamp, then zeroes it. At recorded1/now4 no event; next f32 above4 does unlock. Unlock is processed before name readiness, so a frame can emit25b then259. These are local clock/view notifications, never fabricated server replies.

IsOnGoingRank39074620 is only ONE-based tab present&&rankID!=invalid39addca0; it is not a time check. IsActivatedPeriod390743a0 uses RECEIVED personal acknowledgement time, CRT local tm, and first rejects Monday local hour5<=h<7 (helper3961bf00). It then requires calendar comparator3961bdf0 result1, authored day8 or tm_wday+1, and start_hour==end_hour or start<=hour<end. The unsigned calendar comparator treats both zero bounds as unbounded and includes exact start AND end; before-start gives2, after-end0. The wrapper has no overnight hour branch. These native rules differ the active lane's UTC/half-open/overnight scheduling and must not be silently replaced by server policy. Full calendar-construct/localtime invalid-date admission remains a separate source boundary, not a guessed true result.

Import chain precision: x2game IAT38ff6f1c is xlcommon XlGetCurrentFileTime3301ca70, which despite its name calls CRT `_time64(NULL)`; it is Unix seconds, not Windows FILETIME units. XlGetLocalTime3301ca30 forwards the raw64 value to `_localtime64_s` (IAT3305d2f0). x2game394245c0 then adds1900/year and1/month, and39429b20 writes six signed-f32 keys. Native calls ignore the CRT return code; this audit does not invent safe date defaults on invalid input. A consumer should distinguish supported conversion from an invalid-date refusal. Name receipt timestamps use that same Unix-seconds wrapper but remain unexposed by the received name cache.

Executed provenance: `native-ranking-tabs-oracle-20261018.json` (4cases, original builder/static span/getters, synthetic static IDs and mocked Lua return sink), `native-ranking-frame-oracle-20261018.json` (9cases, real frame/key lookup, mocked name availability/Lua sink, actual x87 timer-return stub), `native-ranking-getter-oracle-20261018.json` (8cases), `native-ranking-more-getters-oracle-20261018.json` (6cases), and `native-ranking-reward-getter-oracle-20261018.json` (3cases) preserve complete return arity/types/keys/order/rounding. Getter Script interfaces, named static providers and localtime fields are explicitly mocked; these are not date/timezone conversion goldens. `native-ranking-period-helpers-oracle-20261018.json` executes both helper bodies without logic mocks over16 synthetic cases. Source/runtime observations are under the same ignored aa-net directory. Module-verified runtime invalid globals39addca0/39addcb0/39addcb4/39addcb8 all read0; these are not inferred from static BSS.


Ranking source follow-up: `native-ranking-next-oracle-20261018.json` executes the full admitted snapshot callback plus3961f4c0/3961bec0 over7 cases. Next is NOT updated+reset_week*604800: it is updated+1800 when max(0,resolved signed scope_to)<1001, otherwise updated+3600. Missing children are ignored, zero updated/missing rank descriptor retains prior next, and addition wraps64 (UINT64_MAX+1800=>1799). Current four compact rank scope maxima are3000/5000/5000/5000, therefore3600 intervals. The week multiplication remains only the independent reset-period getter.

Exact child/static SELECTs have no ORDER BY: `SELECT id, rank_scope_id, rank_id FROM rank_scope_links`; `SELECT id, rank_reward_id, rank_scope_id, rank_id FROM rank_reward_links`; `SELECT id, appellation_id, item_count, item_grade_id, item_id, name, weeks FROM rank_rewards`; `SELECT id, name, scope_from, scope_to FROM rank_scopes`. Association pass39836f40/39836fb0 walks raw loaded link records and appends each resolved parent vector. Finalizers398288b0/39828a50 sort a TEMPORARY ID copy to construct lookup ranges, not original record arrays. Original runtime parent vectors (module-verified read-only PID48924, `native-ranking-runtime-links-20261018.json`) are ascending linkID: scope links1..10/11..20/21..30/31..40 for ranks1..4. Reward linkIDs are [1,2,3,35,36,37,38,39,40,41], [4,5,6,7,17,18,19,20,21,22], [8,9,10,11,23,24,25,26,27,28], [12,13,14,15,29,30,31,32,33,34]. The compact copy's full-query physical order is lexical10-before1 and differs the original loaded arrays. Reconstructing this captured catalog with ascending linkID is an explicit original-input compatibility choice, not a general claim of native SQL sorting.

Calendar constructor3961e020 checks zero year/month/day before clamping: any zero returns raw0. Nonzero values clamp year>=1900, month1..12, day1..31, hour0..23 and minute0..59; it builds seconds0, month?1, year?1900 and tm_isdst0. XlMakeFileTime3301ca60 forwards to CRT `_mktime64`, allowing CRT normalization and forcing standard-time interpretation. `native-ranking-calendar-constructor-oracle-20261018.json` executes6 cases with only the CRT sink and unrelated canary checker mocked; it is not an actual timezone conversion oracle. `native-ranking-getters-source-20261018.txt` archives the owning getters, callback/frame/builder, association pass, SQL strings and xlcommon import chain. No extra native timer/ready policy is installed in aa-net by these source notes.


The full `IsActivatedPeriod` wrapper now has20 executed controlled cases in `native-ranking-activated-wrapper-oracle-20261018.json`: original wrapper, maintenance helper, calendar constructor and range comparison all execute. Only selected static descriptor/Script interfaces and CRT-facing localtime/mktime providers are mocked. Missing descriptor returns one booleanfalse without conversion; unbounded any-day succeeds, Monday5/6 refuses but4/7 succeeds, day7 matches only Saturday, hour20 is inclusive and22 exclusive, start8/end4 refuses at0/3/8/23, and exact calendar endpoints1000/2000 both succeed while999/2001 refuse. Every supported path returns one bool, not zero results. This establishes policy/argument flow without claiming execution of CRT timezone normalization.

Numeric argument admission for RequestPlayerRecords/GetRankInfo/IsOnGoingRank is shared registration adapter39074f90?39074d00: fetch argument1, require successful ScriptAnyValue type4, then f32 CVTTSS2SI. Wrong/missing/string/bool arguments return zero Lua results before the method. NaN/out-of-range follow x86 INT_MIN conversion. No-argument getters use39073b40 dispatch. The original en_us runtime strings in `native-ranking-runtime-labels-20261018.json` are Arena Ranks/Score/Wins, Sport Fisherman/Record Length/empty, Heavyweight Fish/Total Weight/empty, Auto Racing/Best Record/empty. These verified descriptor observations are a cross-check for authored localization, not permission to invent missing translations from Korean compact text.


Appellation reward source refinement: loader3982d650 uses `SELECT id, buff_id, name FROM appellations` (SQL39a149dc);3982d733 reads column1 and3982d775 stores it at resolved descriptor DWORD0, while localized name is+4. GetReward's nested buffType is therefore appellations.buff_id, not appellations.id. Original ranking Lua also preserves the request-index asymmetry: extracted `survey/dec2/x2ui/ranking/ranking.lua` initializes frame.tabSelected1 and forwards it unchanged to RequestPlayerRecords, later selecting GetSelectedTabIndex unchanged; no compensating minus1 appears in those call sites. The native >=0/<count request check remains observed behavior rather than an adapter correction.


Initial personal board storage is explicit, not uninitialized: new-key path39076a0a allocates0x80 and calls3961e120 for the personal row, then writes both vector triplets+60/64/68 and+70/74/78 to0 and updated+48/4c, computed next+50/54, reset+58/5c to0 (EDI was zeroed3907699f). BoardID+40/44 copies39addca8/cac, module-verified runtime64 value0. Thus an existing personal board before any snapshot has known empty snapshot rows and zero raw timestamp inputs, while no board still yields zero Lua results. These raw zeros must pass through actual localtime conversion; they are not invented UTC date tables. GetRankCount reads the static span even before a personal receipt; builder39075ee0 has only personal-callback caller39076adc, so local tab count/default/selected reset0 before that callback independently of static count4.


### Auction raw names and captured-character requests (2026-10-18)

`AuctionLot.raw_client_name`/`raw_bidder_name` and `AuctionBid.raw_bidder_name` retain complete received bytes as `Some(Vec<u8>)`; existing String fields retain the legacy lossy UTF-8/edge-NUL-trim projection for compatibility. App-created records may leave raw fields None. `native_client_name`/`native_bidder_name` expose the prefix before the FIRST NUL, refusing full received names longer than128 bytes as an explicit safe-app boundary. Mutating request validation uses this boundary. Writers prefer retained raw bytes and emit their first-NUL prefix without re-encoding. This does not alter received item descriptor bytes or inventory/mail settlement.

Native lot serializer398a6de0 reads seller/bidder names into129-byte buffers with archive max128, appending NUL. Its write paths call imported XlStringSize330199a0, a byte scan ending at first NUL (not Unicode scalar counting). Bid serializer398a6880 uses the same name contract. SC12e/132 serializers397ab2b0/397ab2d0 delegate to the lot serializer; SC12f397c3200 uses398a6ff0?398a6de0 for rows, and SC131397b56a0 uses398a6880 before buyout/template fields. Source archive: ignored `native-auction-name-source-20261018.txt`.

`Command::AuctionForCharacter { character_id, command }` requires InWorld and exact nonzero captured persistent ID matching both selected identity and the actual selected character header. The tuple `Command::Auction(command)` remains an explicitly unscoped legacy compatibility route. Both report failed/missing connection writes through existing local RequestRejected, and report Requested only after successful write; these are transport observations, not native ACKs. No roster/item/money state is advanced optimistically. Auctioneer service eligibility remains an app/native-planner boundary; active lane handlers ignore both auctioneer IDs. No new live transactions were attempted against the stalled game lane.

`auction_name_writer_oracle.py` executes original398a6de0/398a6880 plus actual xlcommon330199a0 locally in Unicorn. Archive primitive/string sinks are mocked and the null-item3961f500 branch uses invalid template0; this is not a full equipment-descriptor oracle. Five recorded cases cover ASCII, binary+embedded NUL, leading NUL, Korean UTF-8 and128 bytes. Four complete original writer goldens are embedded in Rust tests with no ignored-artifact dependency. Additional regressions cover full raw receipt retention,129-byte refusal, raw bid cache updates, exact captured-session loopback and failed-write rejection. Existing active-lane search count safety (0..9) and transaction settlement boundaries remain unchanged.


### Received buff tick-index retention (2026-10-18)

`Buff.raw_tick_index: Option<i32>` preserves the exact signed snapshot tickIndex, including negative/high-bit patterns. NetBuff serializer397c9230 writes duration+30, elapsed+34, tick interval+38, then signed tickIndex+3c via archive virtual4c at397c9326..39 (name39a13fe0); stack+40 and charged+44 follow. Lane models/buff.rs write_net emits this same i32 field, currently0. Snapshot decoding no longer discards it. NativeBuffs snapshot hydration preserves it; a subsequent Created receipt replaces it with None because Created has no explicit tick-index field. No elapsed-time extrapolation or timer/cooldown producer is added. Native398a0960 consumes the same+3c in duration-dependent modifier evaluation, only when tick interval and duration are nonzero. A future constructor-derived Created projection is separate from raw receipt retention and ongoing tick advancement. Tests cover signed extremes, every truncated NetBuff prefix, following-byte alignment, snapshot sidecar preservation and Created invalidation.


Created follow-up: `Buff.native_tick_index` is a separate optional native constructor projection. Snapshot decode copies raw_tick_index; Created decode derives tick==0?0:unsigned elapsed/tick, then preserves resulting i32 bits. Default/app-uninitialized remains None. NativeBuffs repeated Created first performs existing source-backed duration/elapsed reconciliation (rule5 wrapping duration addition and unsigned max elapsed; others duration replacement and unsigned min elapsed), then recomputes from reconciled elapsed and incoming tick. raw_tick_index remains None. Actual x86 fragments3989e1c4..1e8 (7cases) and3989e0d5..127 (10cases) execute without instruction/provider mocks in `buff_created_tick_oracle.py`; independent goldens cover division, zero interval, unsigned comparison/high bits and no duration clamp. Only synthetic input/descriptor/runtime records are supplied. No actor-frame tick advancement or dynamic-modifier/GCD evaluation is claimed.


Source archive `native-buff-tick-index-source-20261018.txt` also captures the advancement boundary39388a48..39388b28: frame delta adds to elapsed, then a due tick increments index by one after tick-effect work, rather than assigning elapsed/tick each frame. Duration/tick equality can suppress further ticks. This source is why constructor projection must not be extrapolated using elapsed wall time; actor-frame tick effects, invalidation and record lifetime remain outside this slice.


### Explicit self-buff cancellation (2026-10-18)

`Command::RemoveBuff { character_id, obj_id, buff_index }` submits only explicit self cancellation with native reason0. Session checks InWorld, nonzero captured persistent ID matching selected/header, nonzero current actual self runtime ID and BC24 range. Success/failure is reported by local `BuffRemovalRequested`/`BuffRemovalRequestRejected`; neither is an ACK or removes cached buffs. Session has no duplicate Buffs cache: original ordinal/descriptor admission and delayed exact-buff freshness belong to the app adapter. No automatic frame cancellation or mate/slave cancellation is enabled by this command. Raw `game::ClientMsg::RemoveBuff { obj_id, buff_index, reason }` exposes exact protocol fields.

`Command::RemoveOwnBuff { buff_index, reason }` is the same `CSRemoveBuff` addressed to the current self runtime ID with the caller's reason. The glider end uses `{ buff_index, reason: 9 }` (`FUN_393888b0`, decompile line 735655; capture log rows 168455 and 177786). Session checks only InWorld and a nonzero current self runtime ID, sends the exact `game::ClientMsg::RemoveBuff`, and reports `BuffRemovalRequested` / `BuffRemovalRequestRejected` without fabricating an acknowledgement. The loopback test also pins opcode `0x55`, reason 9, index 18/20/`u32::MAX`, and failure when the connection or self ID is missing.

Original `X2Ability:CancelPlayerBuff(i)` registration3943d316 names399d9818, invokes39438790 through3943bd30: strict successful numeric ScriptAnyValue4 then f32 CVTTSS2SI; wrong types return zero results. It subtracts1 with native integer wrap, resolves listKind1 (Good) via39386d70, refuses negative/out-of-range silently, resolves descriptor by actual buff template, and silently returns if absent. Descriptor byte+1a5 true emits error0xe0; otherwise sends current actor runtimeID and the selected record's instance index with reason0. That byte is `buffs.system`: loader39734dd1 reads column0x96/150 of full SELECT39a07550, parses T/t/1 into temporary[-19b] relative to record[-340]. No online/phase/clock gate is invented inside the native wrapper. Original unitframe/player.lua right-button callback calls this method with captured ONE-based Good icon ordinal, not the per-unit instance index. All native branches return zero Lua results.

Constructor397abc30 and serializer397c7840 use BC runtimeID, u32 instance index, u8 reason; the named buffId archive variant emits no selector byte. Active lane registers CS055 in skills/mod.rs and handlers.rs432 reads the same fields. It admits actual self or owned mate/slave, removes only kind1 and otherwise does nothing; reason is not consulted. Native conditional actor-frame path39389148 also uses this packet with reason9, but that planner remains unsupported here. Receipts continue through genuine SCBuffRemoved only. `native-remove-buff-source-20261018.txt` archives source. Wire goldens preserve BC edge/index0/u32MAX/reason bytes; Session loopback tests reject stale header/character/actor/inactive/missing and failed writes, validate exact successful frames and ensure no fabricated BuffRemoved.


`remove_buff_oracle.py` independently executes original39438790, actual list lookup39386d70 and constructor397abc30 over7 cases: ordinal1/systemfalse sends actual actor/index/reason0; ordinals0/2/-1/INT_MIN silently send nothing; systemtrue emits224 without send; missing descriptor silently sends nothing. Every case reaches Lua end exactly once. Descriptor resolver, Lua/error and transport sinks are explicit mocks; record/list/branch/constructor instructions are original. Output is `native-remove-buff-oracle-20261018.json`; this validates native control admission without claiming real server completion.


### Buff cooldown-skill input retention (2026-10-18)

`Buff.raw_cooldown_skill: Option<u32>` retains the formerly skipped trailing NetBuff cooldownSkill. Native397c936e..938e uses named variant cooldownSkill39a13fd0, u32 record+48; active lane models/buff.rs write_net emits0 in the same position. Snapshot skill_id retains its legacy0 projection; the new optional field distinguishes actual received0 from app-default unknown. Created serializer397ca670?397c9070 writes input+18 before the four PISC values, which the existing decoder calls skill_id; new raw_cooldown_skill copies that same received u32. Constructor3989e1f5 copies input+18 to runtime+48. Native repeated Created3989e0d5..1fb skips that assignment, so NativeBuffs retains the original constructor/snapshot value rather than overwriting it with a later receipt. Full raw Created Event still carries the new value. Snapshot seed replaces it authoritatively. Source archive: `native-buff-cooldown-skill-source-20261018.txt`. No skill resolution, cooldown insertion or lifetime inference is added. Tests preserve0/high-bit/u32MAX, every truncation and trailing-byte alignment, Created field mapping and native first-constructor retention across repeat/rekey/snapshot.


### Complete received buff caster identities (2026-10-18)

`Buff.raw_caster: Option<skill::SkillCasterData>` retains complete source-closed caster variants0..4 for both snapshots and Created receipts. Snapshot397c92a3 and Created397ca67f call the same397c85b0 caster serializer already proven for Started/Fired. Item id/template/type1/type2 and mount-skill fields are no longer skipped. Existing BuffCaster kind/object projection remains unchanged. Legacy unknown discriminator acceptance remains kind+BC with raw_caster None; this preserves old framing without inventing a typed unknown identity. Default is None. NativeBuffs preserves the original raw caster on repeated constructor updates, just as it retains the original legacy caster fields; snapshot seed replaces authoritatively. No new caster-based eligibility/timer inference is added.

Per-unit matching was explicitly audited rather than strengthened speculatively: actual BuffManager vtable399d1990+8?3938a340?39387710 compares only buffID(record+4) and persistent caster characterID(record+28), first match in selected group. Non-per-unit+4?3938a350?39387770 compares only buffID. Source archive: `native-buff-caster-source-20261018.txt`. Full item/mount caster identity does NOT participate. Thus different item/mount casters with persistent caster0 merge natively; skill::SkillCasterData::native_eq must not be inserted here. `buff_per_unit_oracle.py` executes both actual lookups plus iterator39387340 without instruction/provider mocks: two differing synthetic caster blobs/char0 select first; wanted11 selects second; missing22 returnsnull; non-per-unit ignores22 and selects first. Typed Rust regression verifies differing item/mount receipts merge then distinct persistent caster separates. Parser regressions cover every supported variant plus legacy unknown255, all truncated prefixes and following-byte alignment in both packet shapes.


### Native Good-list order, capacities and descriptor admission (2026-10-18)

`NativeBuffs::set_catalog` installs the COMPLETE actual buffs catalog and sets `catalog_loaded()` true, even for a genuinely empty catalog. This distinguishes native missing descriptor from app data not yet installed; static rules/loaded status survive received-world lifecycle clearing. Snapshots remain known before catalog installation: actor initializer3936b135?39388440 copies every admitted record directly, descriptor-independent, Good/Bad/Hidden in their own original orders. NetBuff serializer397c93a0 clamps counts to32/24/24 before reading rows. Active-lane parsing continues consuming complete declared rows; an over-cap snapshot makes only the native sidecar unknown as an explicit safe-app malformed-input boundary. It does not emulate native truncated/desynchronized framing or remove raw receipts. A later valid snapshot, including empty, recovers known history.

New-record allocator39386e00 appends below Good32/Bad24/Hidden24 and returnsnull at capacity without changing old records. NativeBuffs mirrors this no-op, while existing-record reconciliation still works at full capacity. Invalid static kind cannot allocate. Created with an actually missing descriptor is also a no-op (3938a465..493). When no complete catalog is installed and no matching row is known, the sidecar instead becomes unknown because descriptor admission cannot be decided; unknown history is never rebuilt by a Created receipt.

Removal39389d60 calls first-index lookup39387440 scanning Good, then Bad, then Hidden, and requires the found record's static descriptor before erasure. Complete-catalog missing descriptor preserves the row; unknown descriptor presence invalidates the native view. Actual erase39387000 memmoves following records left, preserving group order. Sidecar removal now removes only that FIRST matching physical-group record, preserving duplicates/raw order elsewhere; raw lane list projection remains separate. Source: `native-buff-cache-admission-source-20261018.txt`. `buff_capacity_order_oracle.py` executes actual allocator/lookup/erase bodies over6 capacity boundaries and duplicate groups; only CRT memmove primitive is mocked. Output `native-buff-capacity-order-oracle-20261018.json` records native removal order Good100,Good101,Bad200,Hidden300,null. Regressions cover unloaded/missing descriptor, known-empty snapshot, persistent loaded status, duplicate indices across groups, full-capacity new versus repeat/replacement, and complete raw overflow framing.

Minimal original CancelPlayerBuff adapter uses `native.of(actualLocalObj)` only when Some, filters Good without sorting and maps ONE-based icon ordinal to actual instance index/template. Displayed and actionable lists must share this source. Raw Buffs.of can contain duplicate historical rows after native stack rekey and is not a safe native ordinal mapping. Root's existing BuffSnap retains template only, so UI capture needs the received instance index and current actual character/actor identity as well; revalidate the mapping before queuing CS055. Static missing descriptor is silent, system true emits224, all paths return zero Lua values. Do not reject instance index0 merely by value: source validates the ordinal, not a nonzero wire index. None native view disables cancellation rather than guessing raw ordinal. No optimistic erase, frame ticking or automatic cancel is added.


### Native SCBuffUpdated and optional UnitBuff stack field (2026-10-18)

`SCBuffUpdated` (`0xb9`) now forwards `Event::BuffUpdated { obj_id, index, stacks, charged }`, preserving signed stack/charge bits and duplicate receipts. Native registration391ec1e1/handler391d5f20, factory391ac2a0 (opcode b9), serializer397c63f0 establish BC3 object ID + u32 instance index + i32 stacks + i32 charged (15 bytes). The named buffId archive section has no wire discriminant. No writer/producer was found in the active bevy lane; this is explicitly native-source receive coverage. The original serializer executes locally in `buff_updated_codec_oracle.py` with only archive primitives mocked; three read/write goldens are in `native-buff-updated-codec-oracle-20261018.json`.

Native393886c0 resolves the first index in Good/Bad/Hidden order, requires an existing static descriptor, and writes charged+44/stacks+40 directly at3938877e..89. The cache mirrors these guards, does not clamp, create rows, reset receipt time, or alter caster/timing/tick/cooldown inputs. An unloaded descriptor catalog invalidates certainty; a complete catalog proving a missing descriptor leaves the record unchanged. Raw legacy index projection remains separate. No native stats/Lua callback sequence or server generation is inferred by this additive event. Tests cover signed extremes, all truncations, duplicate DD04 receipts with a following packet, descriptor/history guards, group priority, and actual WorldState folding.

For a later original getter integration, simply removing the current UI stack clamp is insufficient. `UnitBuff`394c0230 defaults omitted flags to -1; present arg3 initializes flags0 and ignores failed numeric conversion (39058d10), while numeric values use f32 CVTTSS2SI. It resolves an actual actor and one-based Good ordinal, then formatter39421ca0 requires a static descriptor; missing actor/row/descriptor returns zero Lua results. Its stack section39421dcf..39421e17 emits key `stack` only for flags bit0x10. Static stack rules2/3 use wrapping signed abs(charged), emitting only positive results (zero/INT_MIN omit); rule4 emits positive runtime stacks only; all other rules emit literal1. Actual390309f0 converts signed integer to f32, so INT_MAX emits2147483648.0. `buff_stack_getter_oracle.py` executes the original fragment and conversion with only table SetValue mocked:13 cases in `native-buff-stack-getter-oracle-20261018.json`. This closes the optional count field, not the remaining tooltip metadata sections. Source excerpts are archived in `native-buff-updated-getter-source-20261018.txt`, all under ignored data/extract/aa-net. Root retains the earlier tooltip shape until a separate complete getter slice.


### Buff native-view certainty after factual decoder loss (2026-10-18)

An actual `GameDecodeFailed`, or `UnhandledGamePacket` at compressed level4 with positive remaining-byte length, now discards only native buff histories. DD04 inner packets lack lengths; that positive tail was demonstrably abandoned and can contain unobserved buff/unit mutations. Ordinary unsupported framed packets and empty compressed tails do not trigger this policy. Raw legacy received rows and the complete static rule catalog remain intact. Created/Updated receipts cannot reconstruct missing history; a genuine later unit snapshot recovers it. This is an explicit app certainty boundary, not a claim about native reset behavior or unidentified packet semantics. The original ordinal cancel adapter already refuses unknown native views. A WorldState regression verifies both diagnostics, non-loss controls, preserved raw/catalog, and snapshot-only recovery.


### Further UnitBuff projection boundaries (2026-10-18)

Executed `buff_time_getter_oracle.py` runs39421d8e..dcf plus real unsigned converter3901c950, mocking only table insertion (10 cases in native-buff-time-getter-oracle-20261018.json). Flags4 exposes duration including0; flags8 exposes timeLeft only when duration!=0, using unsigned saturating duration−elapsed. Values remain milliseconds; UINT_MAX rounds to4294967296f32. Neither getter extrapolates elapsed from receipt Instant. Flags2 exposes mine from actual local persistent ID versus caster_char_id, not runtime object equality. UnitBuffCount394c0360 counts physical Good rows without descriptor filtering; missing-descriptor rows still occupy ordinal positions although UnitBuff returns no table. UnitDebuff394c0410 uses the same formatter/flags but Bad(kind2) ordinal lookup. These facts must not compress the cancellation ordinal.

Base formatter394219f0 exposes description for flags1; flags exactly-1 additionally exposes localized category (buff/debuff), literal tipType, static name, and only ScriptSystem type1 buff_id/icon. Keys/source are archived in native-unitbuff-field-keys-20261018.json and native-unitbuff-full-formatter-source-20261018.txt. This does not yet authorize replacing the current app tooltip wholesale. Description is generated even when not exposed: provider vtable399d77dc gets template text from BuffDesc+60 through3941a910; generic3941a920 expands `#{...}` via3941ed20. Tokens include duration/tick, damage/heal/mana ranges, and ua_* (not just literal text). For ua_*,3941f049 passes static descriptor+2b4 modifiers plus ability level to3941a700, which invokes3989fde0 with multiplier1; runtime stack/charge/tick does not scale this static UA tooltip branch. Attribute-specific formatting, dynamic damage/heal helpers, and duration override still need source-complete app projection. Native-description examples/tokens/provider transcript are under ignored data/extract/aa-net; no new runtime tooltip implementation is claimed.


The static UA token formatting dispatcher3941a700 is now archived independently. It resolves the attribute name, calls3989fde0 with abilitylevel/multiplier1, then prefers nonzero percent over flat. Percent58/71/90 uses wrapping absolute; other percent emits signed `%d`. Flat dispatch table3941a85c maps attributes to /5 (67/68), /10 (including10), /1000 variants, absolute branches, two reciprocal percentage helpers, doubling then /10, or plain integer. Complete numeric-ID groups are in native-unitbuff-ua-format-dispatch-20261018.json rather than an invented generic unit conversion. Numeric helpers first select exact divisible integer paths; decimal paths explicitly narrow signed input to f32, multiply the original f32 constant, then promote to f64 for printf. Executed buff_number_arguments_oracle.py runs78 actual helper cases including signed extremes, divisibility and reciprocal unknown sentinels(-1000/-100). JSON records selected `%d`, `%.1f`, `%.2f`, or `?` plus actual numeric argument; the XlFormatString endpoint is mocked, so these are NOT executed CRT rendered-string goldens. Exact localized number_value_format wrapping and the damage/heal token providers remain pending.


Follow-up executed-byte closure: buff_number_full_oracle.py now runs all78 helpers through actual xlcommon XlFormatString/core, using the original process module file <client-copy>/bin32/MSVCR100.dll (SHA25660c06e0fa4449314da3a0a87c1a9d9577df99226f943637e06f61188e5862efa), only memcpy mocked. Read-only PID48924 module inventory and relocated qsort64/modf128 comparisons identify the native CRT; no process writes or execution occurred. Full rendered numeric results are in native-unitbuff-number-full-oracle-20261018.json; this improves the numeric endpoint evidence without closing localized provider assembly. A separate16-case buff_description_parser_full_oracle.py executes3941a920 and actual XlStringSize/XlCopyString, mocking only named dynamic expansions/memcpy. Raw output bytes show unknown substitution empty, unmatched marker dropping its tail, no recursive expansion, and a native buffer-boundary defect: a substitution filling cap−1 then followed by literals can overrun the nominal capacity (`a#{x}b`, x=VALUE, cap7 yields7 characters plus NUL). Literal truncation can split UTF8. A future app projection must explicitly refuse unsafe/inconsistent output rather than reproduce memory corruption; no production parser or localized dynamic-provider approximation is added.


### Native/lane lunagem and lunastone result receipts (2026-10-18)

`SCItemSocketingLunagemResult`0x09d and `SCItemSocketingLunastoneResult`0x09e now decode/forward raw result receipts. Public Event/ServerMsg shapes are `LunagemSocketingResult { raw_result:u8,item_id:u64,template_id:u32,raw_install:u8 }` and `LunastoneSocketingResult { raw_result:u8,item_id:u64,template_id:u32 }`. Native allocators391a95f0/391a9670 assign opcodes9d/9e and serializer397b39b0/397b3a40; the wire walks are14/13bytes. Canonical archive bool0/1 matches actual lane item_effect_packets.rs writers. Arbitrary raw bytes remain available without claiming native noncanonical bool normalization. Executed socket_results_codec_oracle.py runs both original serializers read/write across6 independent canonical cases (archive primitives mocked); source/JSON under ignored data/extract/aa-net. Tests cover literal original goldens, all truncations, noncanonical bytes, ordered repeated DD04 packets with a following ordinary packet, and actual Session forwarding without invented items/actor/ACK.

Registration391ebd99/da6 selects callbacks39185d70/39185f00. Both require active client Game, target persistent itemID existing in current inventory, and successful item-link format394e4ef0. Otherwise they silently skip Lua and the subsequent internal notification. Admitted lunagem callback emits `ITEM_SOCKETING_RESULT(result==1,itemLink,templateType as unsignedf32,installBool)` (event0x99); lunastone emits `ITEM_ENCHANT_MAGICAL_RESULT(resultBool,itemLink,templateType)` (event0x9a). Registration39214c77/86 maps exact names. AFTER callback they dispatch internal event69/70 with argument0 through39202fb0/39202db0. These helpers implement an ordered callback dispatcher, not directly task removal; the registered receiver must be traced before claiming any task-close semantics. Net forwards raw receipts irrespective of frontend lookup, leaving these actual UI gates and task context to the app. There is no new long-lived success resource, inventory mutation, request/choice or optimistic grant. Active lane item_effects.rs sends genuine item-task update before lunagem result, but lunastone result BEFORE its item-task update; consumers must preserve receipt order rather than fabricate an updated item at callback time. Grade-enchant result/broadcast0x09c/09f remain separate pending readers.

The intervening read-only lobby audit is archived as lobby-timeout-readonly-diagnosis-20261018.md plus an unapplied diagnostic-only server diff. Actual timeout is pre-lobby-ready, not character-list decoding; source offline-labor catchup waits on the shared save barrier before EnterWorldResponse, while recent-account initialization can bypass that path. Existing INFO logs and idleSQL/zero-InnoDB-transaction observations do not identify the current barrier holder/downstream cycle. No server/client behavioral workaround, restart, timestamp change or financial mutation was made. Exact owner requires owner-authorized instrumentation; the artifact is not claimed to fix the deadlock.


### Native/lane grade-enchant result receipts (SC09c / SC09f)

`item_effect::GradeEnchantResult` retains raw result/grade bytes, decoded optional inline Item and every consumed `item_wire` byte. `GradeEnchantBroadcast` additionally retains the full counted `raw_name`; `native_name()` projects the first NUL only for lengths <=128. Longer names remain fully framed/forwarded but this safe adapter refuses the original CString destination's unsafe domain. ServerMsg/Event tuple variants forward each genuine receipt/repeat without inventory mutation or fabricated success.

Native factories391a94a0/391a96f0 assign09c/09f. Serializers397c2070/397c2100 read u8 result, inline Item3961f500, before/after u8;09f prefixes counted CString. Original serializers + null-item writer + actual XlStringSize executed locally in `data/extract/aa-net/grade_results_writer_oracle.py`, output `native-grade-results-writer-oracle-20261018.json`:15 paired cases, result0/4/255, empty/non-UTF8/UTF8/128-byte names, explicit archive primitive/string mocks. Non-null item tests reuse the established Item reader and preserve its complete input. No native unsafe-memory behavior is reproduced.

Handler39185bd0 admits result0..4, formats the RECEIVED inline item with394e4ef0 and, only if formatting succeeds, emits GRADE_ENCHANT_RESULT(event98: numeric result,itemLink,before,after), then dispatches internal event68(argument0) through39202fb0/39202db0. Broadcast39186070 has no0..4 gate: formatter success emits GRADE_ENCHANT_BROADCAST(event9b:nameCString,numeric result,itemLink,before,after), with no corresponding internal notification. No current inventory lookup/Game gate is added to these handlers. The active lane `systems/economy/item_effect_packets.rs` matches these fields; `item_effects.rs` sends actual item/money task updates before local result, then SkillEnded, and broadcasts successful grade>=8 afterward. Receipt-only decoding does not claim a frontend formatter/task implementation.

Current-item link helper clarification for09d/e:397ccfb0 searches equipment0..27 before inventory3961e9a0. Formatter394e4ef0 rejects null/invalid-template items, copies192 bytes, clears copied ID, sets count1 and base64-encodes394df700; it does not require a static descriptor/name lookup. Event callbacks precede the internal numeric notification.39202fb0 returns singleton39d1f4f0;39202db0 copies/iterates the selected ordered callback set and invokes each callback with (eventID,arg). Numeric68/69/70 must not be equated with task IDs without their registered receiver.


Result internal receiver closure:39046631..65 registers39046430 for internal68/69/70.68 requires controller39adcea4 kind1;69 requires kind2 or current390447a0 kind3;70 requires current kind4. Admitted active controller(+10!=0) gets+14=1, and only when+11 already set tailcalls39046020. These are item-operation rendezvous notifications, NOT task IDs. Final39046020 effects remain a separate source boundary. Source archived in `data/extract/aa-net/item-result-internal-receiver.txt`. Inventory target helper3961e9a0 searches ONLY its passed container in ascending physical slot order. Constructor3961eb10 sets inventory+10 container kind2 (at+18), and separate inventory+61a0 kind3. Thus09d/e lookup is equipment0..27 then BAG2, not bank3. `item-result-inventory-lookup.txt` archives this exact constructor/lookup proof.


NativeBuffs static catalog storage: `rules` is now `Arc<HashMap<u32,NativeBuffRule>>`; public `set_catalog(HashMap)` and received queries/folds are unchanged. HUD resource clones share the immutable catalog rather than deep-copying thousands of descriptors each frame. Runtime lifecycle resets still clear only unit history, retaining loaded catalog and shared allocation. Replacing the catalog creates a fresh allocation without mutating prior snapshots. Pointer-sharing/reset/replacement regression accompanies the change.


### Item-operation controller and equipment-change custom GCD follow-up (source only)

Actual invalid-item-template global3a5ad970 was read as0 from original PID48924 with StandMode module-marker verification and PROCESS_VM_READ only (`item-result-invalid-template-runtime.txt`). Native noncanonical archive-bool decoding for09d install/09e result remains unclosed; applications may explicitly refuse raw values outside0/1, while preserving receipts. No static item-name gate applies to the raw link formatter.

Internal result receiver39046430's continuation39046020 copies controller state, clears active/rendezvous fields before continuing, releases three stored pending inventory keys via394e8520?397d2200(false), and processes associated item effects394e2d40 before conditional BAG item-operation continuation. It is not generic TaskDialog closure. Actual helper source is archived in `item-result-controller-complete.txt`; a receipt-only Lua adapter does not claim this controller/pending workflow implemented.

Separate received item-task custom GCD source:394eff00 processes all actions first. A successful action8 with source container1 and nonempty item captures its packed slot; only final captured physicalslot15..18 triggers394f0142?39899df0. Duration uses global39005b20()+c8 f32 multiplied by constant3999d1b0, x87 trunc_i64 then low32.39899df0 skips when native GMmode1 byte>0; otherwise evaluates effective attribute74, clamps signed value to at least?999, then writes BOTH GCD remaining/total to wrapping_u32(duration*1000)/wrapping_u32(A74+1000). It overwrites rather than extends. `custom_gcd_oracle.py` executes the original39899df0 with ONLY effective-attribute callback mocked:10 cases in `native-custom-gcd-oracle-20261018.json` cover suppression,zero,negative clamp and unsigned overflow. Example duration1500/A74?1000?1500000; UINT_MAX duration/A740?4294966. These are conditional arithmetic goldens, not proof that runtime A74 providers or global+c8 input are currently supplied. No producer/timer behavior is enabled by this source milestone.


Custom GCD duration input closed:395681c0 iterates numeric constant IDs, getter39856460 resolves gameDB+c880, and stores f32 values at global+ac+id*4. Thus +c8 is constantID7. Loader39894652 SQL is exactly `SELECT id, value FROM constants`;398946d3 rounds the SQL numeric value to f32,39894759 builds the+c87c map. Read-only compact constants row7 is1.0. Multiplier3999d1b0 isf32 1000.0, producing native configured1000ms for current data. This closes static input, not final effective A74 or actor-lifetime admission. Source archived in `global-item-gcd-config-source.txt` / `native-constants-loader-source.txt`.


### UCC imprint/cleanup result receipt (SC15d)

ServerMsg/Event `ItemUccDataChanged { ucc_id:u64, player_id:u32, target_item_id:u64 }` preserve each genuine20-byte receipt/repeat, including zero UCC cleanup and all-ones IDs. No item/UCC/appearance cache or controller mutation is inferred. Factory391b25a0 writes15d/vtable399b3f3c; serializer397b78f0 visits UCC+18 (archive+48,u64), persistent playerID+c (+3c,u32), targetItemID+10 (+38,u64). Registration391ece80?391c6f00 installs handler391dc950, which delegates to native UCCmanager394f88f0; this is not interpreted as generic Lua/task success. Source transcript `native-ucc-result-source.txt` and original read/write serializer oracle `ucc_result_codec_oracle.py` / `native-ucc-result-codec-oracle-20261018.json` are under ignored data/extract/aa-net. The oracle mocks only three archive primitives and covers asymmetric/zero/max fields; independent literal tests cover all truncations, DD04 following-packet alignment and session duplicate forwarding without mutation.

Active lane `systems/economy/item_effect_packets.rs` writes exactu64/u32/u64. Actual ImprintUcc/CleanupUcc producers in `item_effects.rs` queue this receipt BEFORE the genuine GAIN_ITEM_WITH_UCC item-task update; cleanup uses UCC0. The later actual item task remains the authoritative inventory producer. No live imprint/bleach transaction was performed.

Empty-item identity lookup clarification: ctor3901cb10 explicitly initializes Item persistentID+0/+4 to0; destructor3901cc60 likewise clears both.397ccfb0 scans equipment rawIDs with NO template-validity gate, so requestedID0 returns the FIRST empty equipment slot and does not continue to BAG; link formatter then refuses invalid-template0.3961e9a0 BAG loop checks template!=invalid3a5e10a8 before ID comparison, excluding empty BAG rows. An Option<Item> mirror must preserve that early empty-equipment match/refusal rather than skip None and select a later bag row.


Adjacent result-UI getter prerequisite (source only): X2Item GradeColor registration394831ad?39480f20, GradeName39483209?39480fc0; shared39481d20?39481900 strictly requires first argument ScriptAnyValue numeric type4, then CVTTSS2SI fromf32. Missing/wrongtype returns ZERO Lua results; no default.39695c30 takes only the lowbyte and looks up actual item_grades.id. There is NO raw?script/display grade conversion in these wrappers. Missing descriptor returns zero results. Color emits ONE STRING from descriptor+0 (color_argb); Name emits ONE STRING from+34 (localized item_grades.name via395b1350). Loader396c908a reads SQLcol0 into lowbyte key,396c91af resolves tableitem_grades/fieldname. Display-keyed application maps must explicitly undo their existing raw0/1 mapping before this lookup. Source `native-grade-lua-getter-source.txt` and `native-grade-getter-loader.txt`.


### Schedule reward raw receipts (151 / 1eb; explicit lane framing)

`schedule::ScheduleItem { schedule_type:u32, raw_gave:u8, accumulated:i32, raw_updated:i64 }` retains all fields/bits; ServerMsg/Event `ScheduleItems { declared_count, items }` and `ScheduleItemSent { schedule_type, raw_by_mail }` forward ordered rows/duplicates and every sent receipt. No timestamp epoch/localization, claim success, local schedule cache, timer, automatic query or outgoing claim is inferred.

Native151 factory391b1a30/vtable399b3ac8 serializer397b5f20 reads u32type+archiveboolbyte. Native1eb factory391b6da0/vtable399b4264 allocates16 inline rows. Serializer397b0190 consumes bytecount then ONLYmin(count,16) rows; row3961f100 is exactlyu32/u8/i32/i64 (17bytes). Active lane `packets/g2c_misc.rs` instead writes min(list,32) count and ALL declared rows, per its explicit source comments and writer. Active read_lane therefore consumes every declared row, preserving DD04 alignment; it must not pretend this is the native16 wire behavior. No native over16 cache/row eligibility is claimed. The u8 count bounds framing allocation at255; truncated rows fail rather than produce partial receipts.

`schedule_codec_oracle.py` / `native-schedule-codec-oracle-20261018.json` execute original397b0190+3961f100 and397b5f20 with archive primitives explicitly mocked. Six read/write cases include native0/1/16/17 counts (declared17 consumes16), canonical sent0/1 and signed/time extremes. Independent lane32 literal golden and following DD04 sent/removal, full truncation and duplicate/session-no-mutation tests establish active transport behavior separately. Timestamp remains rawi64 despite lane naming updated:UnixTime, until native DateTime producer/projection is independently closed.


Schedule callback source for future UI integration:1eb handler391e1d80 calls3930b7a0 before SCHEDULE_ITEM_UPDATED()(155). Manager clears16x24-byte slots, then loops RAW declaredcount and copies each row into first sentinel-empty slot. More than16 source rows is an unsafe native callback domain because serializer admitted only16; lane32 must not be called a native cache snapshot.151 handler391dc240?3937a760 first requires static schedule descriptor397443e0. With a matching client cached row it increments gave wrappingu8, clears accumulated and stamps fresh current-time import38ff6f1c. Nonzero decoded bymail tries localized DOODAD_PHASE_MSG(text)(fd), then emits SCHEDULE_ITEM_SENT()(fc); missing descriptor emits neither. Names verified read-only original PID48924 with StandMode marker in `native-schedule-event-names-runtime.txt`. No corresponding timestamp/cache fold is added here: it needs complete static admission and explicit native clock sampling, not an inferred receipt time or raw_updated epoch.


### Explicit captured item-effect requests (CSStartSkill052)

`item_effect::{CapturedBagItem,ItemEffectMode,ItemEffectRequest}` adds a separate052 transport path. ModeGrade retains optional support and captured received auto-AA; Socket accepts source-proven skill23728/23729, Gem11285 (original PID48924 read-only StandMode-verified globals39ac3214/18/10). Source/target/support are captured full BAG2 Item snapshots with slots. A nonzero skillID for Grade is caller-resolved from the genuine static use-skill path; the transport does not invent an effect mapping or claim the protected ordinary planner is implemented. Generic Unit/Item/Mount skill writes remain unchanged.

Native X2ItemEnchant.Execute39065100 reads397ccd50 then controller39044f20. Grade390447d0 resolves target/source/support from BAG2, checks source static descriptor, local Game byte+4b4 against source descriptor bounds+54/+50,394e1a90 and use-skill helper394e1880/395a72a0, then calls protected39396e20. Socket39044a30/gem39044ba0 use39397e50; their local/ordinary predicates and native controller continuation are not replaced with always-true claims. Actual caster helper39044360 captures itemgrade+0xc and MADE_UNIT_ID+0x14 (not a detail discriminator); serializer3961f5be confirms+14 belongs to made-unit wire field. Target carries actual targetid/template/grade. Grade object7 contains supporttemplate/id (both0 without support) and auto-AA. No support object is appended for socket/gem.

Actual397cad20 +397c85b0/397bfa90/397bfd50 execute in `item_effect_start_oracle.py`, with only archive/BC primitives mocked:3 complete independent payload goldens cover socket flag0, grade flag7 with absent support and nonzero support+autoAA. `native-item-effect-start-oracle-20261018.json` preserves expected bytes. Lane `systems/skills/handlers.rs` reads skill/caster/target/flag then low-nibble object7; `types.rs` reads its exactu32/u64/bool. Original caller validation is source-only, not mocked-oracle execution.

`Command::ItemEffect { character_id, request }` requires matching actual selected CharacterState, InWorld and nonzero24-bit self runtimeID. Session owns an actual receipt-folded inventory mirror. A plain known-BAG-slot set admits ONLY slots covered by genuine complete InventoryChunk snapshots; it clears on Init/character/world/disconnect/kick, actual decodefailure, demonstrated nonempty compressed-tail loss and truncated inventory chunks. Ordinary unknown framed packets do not invalidate it. All captured Item fields and Grade auto-AA are rechecked before write. These stronger application stale-input/freshness guards do not claim native planner equivalence. Root callers must also resolve static skills/permissions and validate fresh original UI/controller context. This initial transport intentionally supports BAG2 captures only.

Local `ItemEffectRequested` means an actual connection write succeeded; `ItemEffectRejected` covers missing/inactive/stale/unknown/failed-write paths. Neither is native success/ACK or Lua result. There are no automatic sends, costs, grants, pending-key registrations, controller continuations or optimistic inventory edits. Loopback validation only; no lane financial transaction was attempted.


Grade source-item level guard refinement: Item loader SQL39a00d90 col23level_limit?descriptor+50;col24level_requirement?+54 (396cb712/723).390447d0 rejects requirement>local Game levelbyte+4b4 with error206, or nonzero limit<level with212. These are source-item use-level conditions, not target-item level bounds; caller metadata admission can preserve this exact ordering.

### Native enchant mode and selected-slot source contracts

The six public mode wrappers39064b20..39064da0 ignore Lua arguments and return zero results. They call39426af0 before39046740. Crucially,39426af0 only handles action-cursor kind4 at3a5a4ea0, dispatching a mapped action through3920fa60 and resetting that kind; it does **not** call picked-item clear3942e790. These mode wrappers must not unconditionally erase a held BAG item. The application currently has no equivalent kind4 action-cursor provider. X2Item:IsInEnchantMode394808c0 returns one BOOL(mode!=0); no public IsActivated registration was found.

Same-mode39046740 does nothing after that wrapper helper. Changed mode releases old pending/effect keys, clears target/source/support, writes the new mode, then emits only ENTER_ENCHANT_ITEM_MODE(modeString)(207), SWITCH_ENCHANT_ITEM_MODE(modeString)(209), or LEAVE_ENCHANT_ITEM_MODE()(20a). It emits no UPDATE during switching and preserves active/input/result latches. Strings are grade/socketing/unsocket/gem. Enter also registers internal result observers and calls393be180(1), whose application meaning remains unclosed. Leave alone can invoke ordinary cast-stop when an existing casting timeline skill matches the old mode; this is not a generic unconditional cancel. `enchant_mode_oracle.py` executes original39046740/39046600 for six transitions with explicitly absent Game and mocked external endpoints; its JSON records callback-time slots/mode and preserved nonzero latches.

GetTargetItemInfo390656a0, GetEnchantItemInfo39065980 and GetSupportItemInfo39065c60 ignore arguments and call39044d30(kind1/2/3). Empty packed location, missing item or invalid template yields **zero Lua results**. Success yields one ordinary39423560 item-info table plus total from3961fbc0(template,grade,0,0). Grade-mode target additionally requires its static descriptor and adds grade_enchant_cost via397cc850 and grade_enchant_currency (default0; raw-grade descriptor+8 only under native clientflags0x1000). Missing that descriptor returns zero results, even after constructing the intermediate table. These cost/provider helpers are not replaced by arbitrary application prices.

Target/Enchant/Support FromPick wrappers first require cursor predicate39424550 (raw picked kind1 or4 at3a5a4e68); false returns zero Lua results. They capture packed3a5a4e78 and clear the actual picked cursor3942e790 **before** validation. Successful selection emits UPDATE_ENCHANT_ITEM_MODE(target-and-source-present,false)(208), then returns BOOLtrue; error emits39428370(error), then BOOLfalse. Presence390446b0 is only packed-location presence, not Execute readiness. Target/source setters refuse an already occupied respective slot (error1); successful replacement preserves the opposite slot and clears support. `enchant_selection_oracle.py` executes the actual setters with explicitly mocked eligibility validators to prove mutation/refusal/order only; it does not prove admission.

Target validator39045850 requires currentGame. A zero casting pair admits further checks; otherwise its timeline must exist and its skill must not match current enchant mode. Missing timeline or matching enchant cast returns1. Then it requires BAG2, absent pending key, actual nonzero-template item, and397cc350 false. The latter is precisely (item.flags & 0x04)!=0 AND native clientflags0x2000; true returns0x298. Its flag is now mapped to actual FeatureSet byte5 mask20 below, not Unit.flags; absent received features remain unknown. Grade requires items.grade_enchantable (descriptor+3e: SQL39a00d90 column18 through396cb6a1..c1 and descriptor copy396cbb54). Gem requires Equip detailtag1. Unsocket also requires397cd1d0, otherwise0x2e7. Socket's static compatibility helper390444c0/397cc290/395aca00 and target/source compatibility390455d0 remain separate prerequisites; no generic always-ready planner is claimed. Source archives: `native-item-enchant-ui-controller-source.txt`, `native-item-enchant-selection-source.txt`, and the two dated enchant oracle JSONs under data/extract/aa-net.

ClearTargetItemSlot39064e20 ignores arguments, calls39044160(0) clearing ALL target/source/support, then unconditionally UPDATE_ENCHANT_ITEM_MODE(false,false), zero Lua results. ClearEnchantItemSlot39064f00 calls39044160(2), clearing source/support while preserving target, then UPDATE(false,false), zero results. ClearSupportItemSlot39064fe0 calls39044160(3), clears only support and emits UPDATE(presence,false), zero results. Empty repeated clears still emit. These wrappers have no direct active/input/result lock/reset and do not clear the picked cursor.

Further selection/source closure: SetItemSlotFromBag390652c0 tries target, source, support setters in that order; first zero result emits UPDATE(presence,false), then BOOLtrue. All three failures return BOOLfalse without a system error or UPDATE. It uses wrapping low8(input-1) BAG2 packing, with no direct nonpositive branch; its strict numeric adapter is closed below. X2Item:IsEnchantable39480970 uses wrapping i32(input-1)<0 => UI_ERROR_MESSAGE("invalid item slot index") plus BOOLfalse. Positive input queries39045ef0: no target/source tries target OR source validation; one missing compares candidate with the opposite selected item; both selected/support empty tries support; all occupied returnsfalse. It is not a simple static grade_enchantable getter.

Support39045cd0 requires actual target/source, Grade mode, BAG2/no pending, actual support item and the same security gate.39044420 rejects target30803/source30804 before support lookup, then requires item_grade_enchanting_supports keyed item_id. Loader SQL39a036c8 cols10/11 map max/min to record+24/+28. Raw min>max rejects even before -1 sentinels; non--1 bounds compare inclusive native grade-list indices39581240, with missing grade returning list length. Verified original read-only constants are archived in native-item-enchant-support-constants.txt. Native39044420/39581240 ten-case execution is in native-enchant-support-oracle-20261018.json (support lookup mocked; grade list explicitly supplied, not claimed as runtime ordering).

Socket helper397cd1d0 counts only a PREFIX of up to seven gem IDs, stopping at first invalid-template sentinel0 (read-only verified3a5e1178). Holes do not count later entries. native-item-socket-prefix-oracle-20261018.json executes the actual helper without function mocks for seven cases.395aca00 returns configured num_socket lowbyte from slot/grade map, not a normalized BOOL; target-only validation requires nonzero, while install compatibility397ceb80 requires unsigned prefixcount<configured count. Compatibility grade39045300/socket39045470 also resolves source skill, scans ordered effects for type0x18 and special-effect type0x5c/0x6a respectively (compactDB SpecialEffect/special_effects, not interaction_effects.wi_id), evaluates3936c500 application conditions, and requires zero result from393a0130/3939ecf0. No descriptor-only replacement is source equivalent.

Execute39044f20 has no direct active-latch guard. Missing target/source emits the invalid-slot UI error; helper rejection emits no UPDATE. Helper acceptance calls390442e0, captures current slots/source/support identity fields, sets active1/result0/input_latch=(sourceDesc.use_skill_as_reagent==0), then UPDATE(false,true). Public Execute still returns zero Lua results. The ordinary planner39396e20 remains protected and is not replaced by these local compatibility checks. Additional source is archived in native-item-enchant-compatibility-source.txt.

Special-effect catalog correction and executed preview: native SQL39a09898 loads id,special_effect_type_id,value1,value2,value3,value4 from special_effects;3972c453..c4ca stores the five-value record at offsets0/4/8/c/10. The type0x18 compatibility branches refer to SpecialEffect types92/106, **not** interaction_effects.wi_id. Native393a0130 grade preview requires target grade_enchantable, target level_requirement>=SpecialEffect.value2, and value3==-1 or target impl_id. Raw current grade11 alone fails with extra0x299; this check is not a generic >=11 clamp. Event source30804 and target30803 must be paired; neither is the normal path. Preview options39ac3224=2273(0x8e1) skips its currency-check bit0x1000. native-grade-preview-oracle-20261018.json executes12 actual393a0130 cases with descriptor/current-item/security endpoints explicitly mocked and no support/currency/effect-condition/ordinary-planner claim. No first-stage proof is promoted to Execute permission.

Selected target cost397cc850 resolves the item equip slot, looks up equip_slot_enchanting_costs through397445e0, then395684c0 sets native formula variables level(+88, sign-extended item.level i8), raw-grade grade_enchant_cost(+8c), and slot cost(+98), evaluates formula kind22 and narrows f32 before CVTTSS2SI. Static lookup misses return0. Formula/catalog arithmetic and currency-provider integration need independent closure before exposing a quote; no price is invented here.

Numeric adapter closure: SetItemSlotFromBag registration39066253 uses39065f70/39065ec0; IsEnchantable registration39482f64 uses39481d20/39481900 (EBX established39482d1f). Both require successful arg1 ScriptAnyValue type4, then CVTTSS2SI fromf32; missing/nil/bool/string return zero Lua results without invoking the typed method. NaN/overflow produces INT_MIN. native-enchant-numeric-adapter-oracle-20261018.json executes18 actual adapter cases with script-frame and final typed-method endpoints explicitly mocked. This does not execute selection validators.

Grade-cost oracle native-grade-cost-oracle-20261018.json executes actual395684c0 and native AST recursion/final conversion for17 cases, plus actual397cc850 routing for16 cases (12 real item/grade and4 missing-cost returns0). Static equipment/holdable/slot-cost lookups and AST reconstruction use actual compactDB descriptors/formulas; external CRT _CIpow is explicitly Python libm double, as the earlier gear oracle, not executed native CRT. Formula is formulas.id22, not unit_formulas.kind22; item_grade variable is item_grades.grade_enchant_cost, not rawgrade. Native impl1 is weapon: item_weapons+20 holdable_id to holdables+74 slot_type_id; impl2 armor uses item_armors+30 slot_type_id. Raw grades0/1/6 produce costs31432:[163517,157537,506607],19326:[140464,135017,320177],28905:[425146,409598,1317179],18420:[235366,225013,428586]. Synthetic signed/edge cases are marked. Source precision and lookup mocks remain explicit rather than claiming a protected Execute oracle.

#### Enchant integration checklist (source versus application policy)

| Stage | Source-closed behavior / required data | Remaining boundary |
|---|---|---|
| Mode | Six no-arg/zero-result wrappers; kind4 action-cursor helper only; same mode emits nothing; changed mode clears slots then ENTER/SWITCH/LEAVE, never initial UPDATE; latches preserved | Action-cursor provider, matching-cast Leave stop and393be180 meaning are unimplemented; do not erase picked BAG or reset working as substitutes |
| Numeric methods | SetItemSlotFromBag and IsEnchantable strictly accept arg1 numericf32/CVTTi32; wrong/missing type returns no values; SetBag low8 wrapping decrement; IsEnchantable tests wrappingi32 decrement sign | Do not silently change negative/overflow conversion to Rust saturating casts |
| Selection routing | FromPick clears genuine picked cursor before validation, returns no values without supported cursor; setter success UPDATE(presence,false) then true, error system message then false; SetBag tries target/source/support, all-failure false without error/update | Genuine picked location/item identity must come from current application inventory, not Lua floating itemID |
| Shared validation | CurrentGame exists; zero cast pair or existing nonmatching enchant timeline; BAG2; actual nonzero-template item; native pending key absent; security mask(item.flags,04) AND native clientflags2000 must be false | A cleared app cast map is not proof of the native pair/TL; actual received FeatureSet byte5 mask20 supplies the flag; absent features remain unknown. Unflagged item04 branch itself does not require the flag. App pending/reentrancy policy can be explicit, not called a native receipt |
| Target / source | Grade target static grade_enchantable; Gem Equip detail; Unsocket Equip plus positive prefix gem count; Socket configured slot/grade count nonzero plus signed itemlevel>=native threshold gameDB+f8. Source static/use-level/use-skill checks and mode-specific skill validation | Socket threshold is actual content_configs.id62; source requirement0/invalidgroup0 bypasses394e1a90, nonzero requirements need known base plus effective recognized attribute. Resolve the actual use-skill path; do not replace it with a generic presence predicate or only raw item category |
| Pair compatibility | Grade SpecialEffect92; Socket/Unsocket106 with install value1 matching mode; effect order and3936c500 conditions; local validators393a0130/3939ecf0 zero result. Gem uses source11285 and397cd820 | Four all-zero tag conditions are supported with complete actual zero-free tagged_buffs catalog and union-only passive producer; nonzero conditions require known actual tag state. Other static parameters still apply. Other application preflight may be useful but must state its narrower policy |
| Grade preview | Target grade_enchantable; level_requirement>=effect.value2; effect.value3==-1 or impl match; event source30804/target30803 paired; rawgrade11 rejected; preview skips currencybit1000 | Descriptor/caster/item identity still required; this is not outer Execute approval |
| Support | Only Grade, both actual target/source; support slot initially empty; BAG2/no pending/security; keyed support catalog, min/max native grade-list order, -1 bounds, event restrictions | Do not infer support eligibility from icon/type alone |
| Getters | Empty/missing/invalid selection returns no values; success ordinary item-info plus total; Grade target adds signed cost and currency. Rawgrade keys retained | Ordinary item-info formatter and total/count providers must stay truthful; flags1000 controls currency selection, not raw grade conversion |
| Execute / transport | Zero-result wrapper; missing slots UI error; successful native helper captures/latches then UPDATE(false,true). Typed052 already rechecks captured owner/header, known BAG snapshots, full items and captured Grade auto-AA before actual write | Ordinary39396e20 protected. A write is not native planner acceptance or server success; app can use explicit known preflight/server authority without claiming exact native admission. No automatic actions, grants or invented pending/result completion |
| Results | Existing09c..09f receipt order, current-item lookup and Lua notices are closed separately | Internal enchant controller continuation/pending reconciliation remains separate from the raw receipt and must not be fabricated as Task68/69/70 removal |

All mode/selection/getter source addresses and executed-oracle qualifications are in the preceding paragraphs and ignored data/extract/aa-net artifacts. This checklist consolidates current proof; open providers are not defaulted to ready, zero or false.

Common supported branches and received feature providers:394e1a90 immediately admits actability_group_id0 or actability_requirement0 (original invalidgroup3a5b9d2c read-only verified0). Otherwise it needs static group, currentGame, received base proficiency plus final effective group.unit_attr_id only for recognized attrs99..118,138,158..163; unrecognized attrs contribute0. This final attribute uses390b3a80/398a0bc0, not assumed base0. Socket minimum level is content_configs.id62: SQL399f73d8 SELECT value,id stores atGameDB[id*4] in39572050..6c;395720e0(62)/GameDB+f8 therefore read actual value20 in both current compactDB and verified original runtime. It compares sign-extended item.level i8, not player level.

3936c500 checks ONLY source required/forbidden tags(record+34/+38) and target required/forbidden tags(+44/+48). Native skill_effects SQL39a0b5b0 cols16/17/20/21 map those four fields through39731159..1c0 and recordcopy397311fc. Preview supplies local actor for both. Required invalidtag0 skips lookup; forbidden0 still queries passive tags, while runtime3989eb90 explicitly rejects invalid0. Passive rebuild3989f320 clears/collects inactive passive BuffDesc+294 tags; loader395d1a8a..1b50 appends every raw tagged_buffs.tag_id (no native zero filter). Actual complete compact tagged_buffs has10822 rows and no zero tags, so its copied static catalog proves passive0 absent without requiring every active passive identity. This is an actual-catalog invariant, not a universal malformed-data bypass. Nine native3936c500/3989e590/3989eb90 executed cases with no function mocks are in native-enchant-effect-condition-oracle-20261018.json, including intentional passive0 refusal.

Native clientflag1000/2000 closure:39496b38 loads global[39b02294],39496b67 establishes GetFeatureSet base=client+14. Enchant reads DWORD[client+18], exactly FeatureSet DWORD+4; mask1000 is feature_bytes[5]&0x10, mask2000 is feature_bytes[5]&0x20. GetFeatureSet39496bc4..bf3 explicitly emits the latter as itemSecure(CString399e0de8). Active lane data/features.rs names bit44 AaPoint and45 ItemSecure, and initialized() clears ItemSecure; the application must use actual received GameConfig bytes, never those initialization defaults. Unknown/missing byte5 stays unknown. This pointer/bit mapping closes the earlier flag uncertainty without mapping Unit.flags or inventing a different configuration field. Existing InitialConfig native protected-producer/raw-lane boundary remains documented separately.

Grade skill classifier395a72a0 checks nativeGameDB+108b4 set. Read-only StandMode-verified original tree has exactly22520,22522,22523,22524,24290,24998,24999,25503; current compactDB distinct SpecialEffect92 skill association gives the same eight. Producer insertion remains unclosed, so native-grade-skill-set-runtime.txt is an audited1.2/current-catalog compatibility observation, not a claim that every future92 association is automatically inserted. Other common source impl0 with nonzero use_skill_id uses that actual field directly in394e1880(selector-1); uncommon impl-specific routing stays separate.

Nine actual394e1a90/398a8fe0 source-use cases are archived in native-enchant-source-use-oracle-20261018.json. Static group/base proficiency/final effective-attribute/logger endpoints are mocked explicitly. Invalidgroup0 and requirement0 bypass even missing actor/catalog; unknown group/actor otherwise refuses, unrecognized attribute ignores the supplied modifier, recognized attributes add the supplied final value with native wrappingi32 before signed comparison. This validates branching/arithmetic without claiming dynamic attribute-provider parity. native-enchant-common-provider-source.txt contains exact source fragments; native-item-enchant-use-constants.txt verifies invalid group/tag sentinels0 and runtime content_configs62=20 read-only.

Gem static compatibility397cd820 requires source descriptor, nonzero target template/static descriptor, target equip-slot nonzero, and item_enchanting_gems keyed SOURCE template. Native SQL39a033a0/loader396cc5c3..639 maps equip_level(+4), equip_slot_group_id(+8), low8 item_grade_id(+10). It compares target RAW grade unsigned>=gem.item_grade_id (not grade_order) and target level_requirement>=gem.equip_level, then397cd200 group permission. Group0 admits; nonzero missing group refuses; existing group with no members admits; otherwise equip_slot_group_maps.equip_slot_type_id must match. Source skill11285 and Equip-detail target checks remain separate. This helper has no installed-gem prohibition.

GetRatioInfos39065d70 ignores args and returns zero results unless390450b0 succeeds: Grade mode, both target/source packed locations present, actual valid target and raw-grade descriptor; present support additionally needs actual valid item/static support row. Success returns one table with base BOOL success/break/downgrade (corresponding grade base ratio!=0). Without support there are ONLY these3 keys, not numeric odds. Support adds signed-i32-to-f32 success_var_ratio/break_var_ratio/downgrade_var_ratio: adjusted=clamp(CVTT_f32((1+f32(mul)*0.01f)*f32(wrapping(base+flat))),0,10000).39044120 then computes base!=0 => CVTT_f64(f64(wrapping(adjusted-base))/f64(base)*100.0), or base0 => CVTT_f32(f32(adjusted)*0.01f). These are relative percentage CHANGES. Base bools do not change with support; support grade bounds are not validated by this getter. native-enchant-ratio-oracle-20261018.json executes39 actual390450b0/39044120 cases (3 grades x no support/all12 real support rows), with static/item lookup and Lua table-write endpoints explicitly mocked.

Selected item-info total is BAG-only3961fbc0(template,exact rawgrade,NULL,false): global BAG3a5ad9a0 from394e0960, actual received slot count, wrappingi32 sum of physical stack counts. No bank/equipment, no pending exclusion(NULL), no secure04 exclusion(false). Total/cost390309f0 and currency390446f0 all use signedi32->f32; there is no unsigned correction for malformed extreme currency IDs.

Controller result/inventory rendezvous39046430: internal44/45/46 notifications from09c/d/e set result_latch1 only for active matching mode1/(2or3)/4, completing only when input_latch is alreadytrue. This latch means a receipt, not raw-result success. Internal1c task payload requires reason40(SKILL_REAGENTS), active nonzero mode and task-item TEMPLATE matching captured source template; it sets input_latch then completes if result_latch!=0. Native matching is not persistent-ID matching. Matching internal18 skill-end forces completion while active even with result/input latches0; internal15 with payload byte+c nonzero sets result_latch2 and waits input latch. Exact notification-envelope projections remain a separate mapping prerequisite. native-enchant-rendezvous-oracle-20261018.json executes17 actual observer/mode-validator cases; completion is explicitly mocked to record entry snapshots, grade-set lookup uses the separately captured eight IDs, and payload virtual endpoints are mocked.

Completion39046020 snapshots captures, resets active/input/result/captured metadata FIRST, releases current pending/effect keys, clears selected slots, then looks up the old target PACKED SLOT in current BAG. Missing/invalid => UPDATE(false,false); valid => rerun target setter (result ignored). ModeGem plus saved result_latch1 stops after target selection with UPDATE(false,false), regardless of raw result success. Other cases try original source slot; if missing, first BAG same captured template/rawgrade excluding pending; then similarly support. Final UPDATE(currentpresence,false). Native slot restoration is not a full persistent-ID guarantee; an application can require stronger captured identities but must label that safeguard. No controller behavior here is Task68/69/70 removal, invented ACK or automatic resend.

Enchant rendezvous envelope closure: internal0x1c is dispatched at394f0087..94 only after397d1030 admits the individual item-task row and394e7140 applies it. Envelope vtable399e5f38+14 points to394e02e0, which returns self: +4 is reason, +8 points to the task row, +c points to its output-item at row+1a8. Count action4 copies after count mutation; Remove action7 copies the resolved item before clearing its live slot. Controller39046430 therefore tests the operation-output template, not a generic ItemTasks reason/template projection. Internal0x15 is Fired, dispatched3939b7d8 from3939b010; it is NOT Started. Envelope399d1c18+10 points to39392060 returning self, with raw skill at+4 and result code byte at+c. Internal0x18 is dispatched3939612a from39396000 CastingStopped only after tracked-TL and actor lookup succeed; it is NOT every received SkillEnded. Its skill comes from the tracked record. The prior abstract observer oracle verifies envelope gates, not these admission/caller projections. Source: native-enchant-itemtask-envelope-source.txt and native-enchant-skill-envelope-source.txt.

Actual compact catalog source-reagent query (use_skill_id grouped with use_skill_as_reagent): grade22520/22522/22523/22524/24998/24999/25503 each one source=true; grade24290 has no direct source item. Socket23728 has108 sources=true and Gem11285 has249=true. Unsocket23729 has one source, item327=false, so its native initial input_latch is true without awaiting a reagent task. Complete raw ID/name rows are archived in native-enchant-source-reagent-catalog.json. This is current static catalog evidence, not a server receipt or inferred future family default.

Executed native Remove task helper397cdad0: matching nonzero-ID case copies output template8001 then clears live template0 and returns true; declared-slot mismatch still copies/clears but returns false, therefore outer handler emits no0x1c. Zero requested ID returns false without mutation. native-enchant-itemtask-output-oracle.json executes the actual helper with explicit mocked inventory lookup, copy/reset, and observer endpoints; it proves control-flow/ordering, not those providers. An app final-inventory diff cannot alone reconstruct native notification admission.

Selected getter/provider follow-up: native support-grade order is explicitly SQL399f7ee8 `SELECT id FROM item_grades ORDER BY grade_order ASC`, loaded395b2b80 with each ID narrowed to low8 and appended in query order (395b2c20..5c).39581240 returns first matching raw-byte grade position, or list length on miss. No inferred ID tie-break. Impl24 is item_accessories: SQL39a01890 column10 slot_type_id loads396bd8f3 into local?54 and copies396bdb16..19 into descriptor+20;396a6d00 returns that item-keyed descriptor. This closes the accessory equip-slot/cost branch.

Socket catalog contracts: SQL399f9448 `SELECT slot_id,grade_id,num_socket FROM item_socket_num_limits` feeds395aca00, keyed raw slot/raw grade; return is low8 configured count, not bool. SQL399f8198 `SELECT item_id,level FROM item_socket_level_limits` feeds source-level threshold; SQL39a03470 `SELECT item_id,equip_slot_group_id FROM item_sockets` supplies install source descriptor.397ceb80 errors: missing target/static18; signed item-level below config62 or source socket-level above target level_requirement0x30d; zero equip-slot/group refusal/missing-or-zero socket count18; full prefix0x2e6; uninstall with empty prefix0x2e7.3939ecf0 install missing source gives result33, wrong target kind/missing target gives34, noGame1; helper failure34 with extra=error. Security is checked only after helper success, giving62/extra0x298. Uninstall does not require source item_sockets row.

Common selector error distinctions: target/source noGame or blocked cast1; wrong BAG/pending/missing item/invalid template0xb5; secure0x298. Source390443a0 static missing/local-level or nonzero actability failure1; source skill mode mismatch1. Native cast predicate tests actual local actor+3ac4 and+3ac8 against zero sentinels: both zero bypass; otherwise393953a0 lookup by+3ac4 must exist and its raw skill must differ from current mode. Ordinary Started writes+3ac4 only for positive casting time; plot code also writes+3ac8, and matching stop/Fired paths clear pairs. Existing received Skills maps do not establish this full pair/lifetime; a conservative observed-cast refusal must remain explicitly app policy. Source archived in native-enchant-selection-provider-closure.txt; no new cast or planner approximation introduced.

Actual397ceb80 executed12-case oracle native-socket-validation-oracle.json independently confirms branch/error order and output-byte writes: install success0/prefix1; missing source1; missing target/static18; low item or source-level mismatch781; invalid slot/group/limit18; full742; empty removal743; removal success0/prefix2. Every failure leaves optional output byte unchanged. Removal still checks target static/minimum signed item level, but never queries source level/equip slot/group/socket limit. Static/config/count/slot/group/logging endpoints are explicitly mocked; no planner or live lookup equivalence claimed.

Grade/socket effect-scan asymmetry:39045400 picks the first type24/Special92 row whose conditions pass, calls393a0130 once and immediately returns code!=0 as generic1. It never tries later rows after local failure.39045550 Socket instead continues after3939ecf0 failure, returning0 on first local success or1 after exhaustion. Eight executed original-wrapper cases in native-enchant-effect-scan-oracle.json prove this with explicit source/static/condition/local-validator mocks. Current compact catalog has exactly one skill_effects row for each of the eleven grade/socket/gem source skill IDs, so these ordinary cases do not depend on unresolved general descriptor association order. Native full24-column skill_effects SQL39a0b5b0 has no ORDERBY; do not infer general ID sorting.

Original bag right-click routing is Lua pre-use admission: sort_inventory.lua main3/0 requires ItemIdentifier!=0, calls injector.PreUseSlot(realSlot), and invokes adapter.HandleUse(virtualSlot) only if not consumed. sort_bag.lua main44 returns false immediately for nonnil cursor; otherwise priority is sell?trade?mail?bank?coffer?auction?item-converter?enchant, stopping at firsttrue. enchant_window.lua main2 AttachItemToEnchant requires existing visible enchant window, calls SetItemSlotFromBag(realSlot), and returns true regardless of setter result. Native X2Bag.HandleUse3944c7f0 forwards virtualIndex?1 through manager virtual+5c and returns zero Lua values; it is not an unconditional enchant override. Source native-enchant-effect-scan-bag-route.txt plus original dec2 inventory scripts.

Source-skill resolver394e1880 selector?1 first reads items.use_skill_id(+98). Nonzero skills are returned directly except impl1/2/22/24 (require equipment container1), impl11 special branch and impl14 two-special-skill restriction. Impl15 Gem and21 Socket use the direct branch just like impl0. native-enchant-source-skill-oracle.json executes original helper WITHOUT function mocks on all365 actual source descriptors (7 grade,1 unsocket,108 socket,249 gem); every result equals the real catalog skill and error output remains unchanged. Missing source socket-level key395aca70 returns imported MSVCP100 std::numeric_limits<int>::max(), INT_MAX, not zero/throw; refusing missing catalog remains a stronger explicit app boundary.

Count4 native397cd970 mutation differs from legacy Inventory: adds wrapping signed i32 delta at the declared physical slot, copies resulting item into output and returns true without persistent-ID verification or count<=0 removal. Original helper oracle cases retain template8001 with count0,?1,MIN after wrapping; zero delta also admits. Remove7 copies output before clearing; a declared-location mismatch still mutates but returns false, suppressing0x1c. The eight combined cases in native-enchant-itemtask-output-oracle.json qualify mocked lookup/copy/reset/observer providers. Any pure output projection must require explicit known PRE-row physical item/resolved first-lookup context; legacy postfold state and raw reason/template alone cannot establish exact native admission or later history.

Pure item_task_projection API exposes native Count/Remove outputs and mutation intent only, with explicit Unknown/Missing/Found PRE-row context. Found is the declared physical slot for Count or the source-exact first equipment/BAG/bank ID lookup for Remove; an empty/unhydrated legacy Option slot does not establish Missing. Unknown/inconsistent/unsupported inputs return None. Container4 admits an untouched empty output without lookup; Remove ID0 otherwise refuses. Caller must apply/order any native mutation intent itself and retain certainty across rows; legacy Inventory/Event/Session behavior is unchanged and no observer is automatically emitted. Five focused regressions cover executed wrapping/nonpositive counts, copy-before-clear with failed admission, invalid-template clear, bypass and unknown/inconsistent contexts.

Seven actual39046020 completion cases in native-enchant-complete-oracle.json confirm reset-before-endpoints, clearing selected slots, valid target restoration, Gem receipt-latch1 early exit, and source restoration rules. A valid old source slot is tried even with a different template from the capture; setter failure does not trigger fallback. Only missing/invalid old source invokes captured-template/raw-grade fallback. Final UPDATE is (current presence,false), except explicit early false/false branches. Inventory lookup, setters, pending/effect release, fallback and Lua emission are qualified mocks; original completion control flow executes. No runtime controller integration or exact lookup certainty is claimed.

Actor-base correction executed in native-unit-actor-base-oracle.json:398a1f90 receives rawActor+8 (3936b11a); actual client39187247 and registry3936b5f3 retain rawActor. UnitState first TL pair1b5c/1b60 hydrates effective cast3ac4/3ac8, second pair1b64/1b68 channel3acc/3ad0. UnitState2020 writesraw3b00, leavesraw3af8 unchanged; actualSC203 mode1 changesraw3af9 only (registry endpoint mocked to the same proven base). Cooldown ctor39899d10 storesrawActor+8 and39899f90 checksowner+3af1=raw3af9. Skills now leaves its existing native_gm_mode1 unknown on spawn, preserving Unit.gm_modes/rawsnapshot API and all received cooldown lists; only actual matching SC203 establishes readiness. No constructor-zero or general Fired/GCD admission assumption added.


Effective GM ownership correction: OnUnitsRemoved391d4d20 loops received IDs into39366480;39366749..6c erases the actual actor registry tree3a169998 before virtual destruction3936677a. Skills therefore retires its private active obj_id on own removal, rejecting late SC203 until a genuine fresh PlayerSpawn. Spawn PISH remains raw-only and never establishes the effective cooldown GM byte. Regressions cover raw PISH0/2/255, foreign/wrong-mode receipts, own removal/late receipt/fresh spawn, and preserved estimates. Full aa-net Bevy suite562passed; Bevy check passed.

Socket formatter prerequisite:39422d42..39422e57 emits socketInfo only for a nonzero resolved equipment slot and nonzero low-byte395aca00 limit; maxSocket is numeric, and actual Equip detail supplies socketItem[1..maxSocket] as raw u32 IDs converted to f32, including zero holes. Template-only/non-Equip detail omits socketItem. SOCKET_MODIFIER=1024 (399b089c).394224c6..eb calls3941dc60 for (flags&1024 && impl21) OR impl15. That helper first installs modifier={} even if empty. It emits attrs0..4 first using3989fa80 plus the FIRST matching static row397a7790; then ItemDesc+dc..e0 rows with attribute>4 in descriptor order. Attributes54/55/74/119 share one first-row-only group. Direct rows are {name=3956dbd0(attribute),value=CVTSI2SS(raw signed i32 value),type=CVTSI2SS(raw signed i32 modifier type)}; no linear-level scaling occurs in this direct display section. Loader query39a0c078 is SELECT owner_type,owner_id,linear_level_bonus,unit_attribute_id,unit_modifier_type_id,value FROM unit_modifiers, without ORDER BY. Exact Item owner attachment is closed below; do not substitute buff modifier tables. The current31003 direct attr7/value200/type0 path is max_mana200.

Primary modifier display order is fixed attribute0..4, not SQL order: generated3989fa80 plus FIRST matching static value using integer ADD, omit resulting zero, force type0; e.g24632 dex2 (attr1) precedes sta3 (attr2). Direct attrs>4 preserve vector order and are emitted even when value0. Static source artifact: data/extract/aa-net/native-item-socket-modifier-source.txt (static original-byte disassembly, not an executed formatter oracle).

Executed generated-primary boundary: source_generated_primary_oracle.py executes original3989fa80 for all357 current impl15/21 items times five primary attributes (1785cases), all0. Only armor/weapon/accessory lookups are mocked NULL, independently proven by zero matching rows in each actual table. All three NULL leaves the initial invalid mod-set sentinel unchanged, so3989faf1..fb33 returns0 before any formula or grade access. This closes raw first-static primaries for current socket/gem sources without assuming generic non-equipment zero. native-source-generated-primary-oracle.json records all cases and endpoint qualifications.

Item modifier attachment closure:396cb95c..98d looks up literal Item (399ac0a0) in owner map+1190;396cba4a..55 equal_range uses current itemID. The matching native tree rows append pointer node+10 to local vector ebp-64. Local ItemDesc begins ebp-140, so vector offset is dc.396cbb54 calls396ba730;396ba8cf..dc copies that vector to final ItemDesc+dc. Thus formatter provider is precisely unit_modifiers owner_type=Item, owner_id=itemID, with native association order retained. No independent SQL ORDER BY is introduced.

Current-pair source continuation:39395b50 first resolves the native first matching TL tree record39394e70, then caster kind0/3 uses record+10, kind2 record+1c, through CURRENT registry39360020; missing record/actor returns without pair mutation.39395bc0 compares castTL first, ELSE channelTL; matching clears both fields of that one pair. Both TLs equal clears only cast pair, not both. current_pair_clear_oracle.py executes actual fragment for five cases, including zeroTL, with no function mocks; it explicitly supplies a resolved actor and does not prove prior lookup/lifetime admission. Clear globals3a177710/72c and plot3a177690/69c were verified0 by marker-checked PROCESS_VM_READ of originalPID48924. Source runtime artifact native-enchant-cast-pair-sentinels.txt. Existing Skills.casting cannot serve as native presence: app Started inserts even cast_ms0, native3939ac15 writes only positive; app Fired can synthesize instant cast rows, native3939b379 clearing depends on descriptor+138; app SkillEnded erasure is not pair-clear evidence. Hydration optional flag7 absence can prove initial zero fields, but preserving that truth after receipts requires complete native timeline/actor admission, not generic event erasure. Completion restore remains exact actual post-task oldslot/current-item and pure PRE-row projection boundary from prior sections; no postfold relocation or timer shortcut added.

Mode-entry external helper393be180 is now identified as NPC-interaction cancellation: native3947da90 is registered CancelNPCInteraction at3947e941 (399de4bc), calling same global39aff9b4/helper as39046698. Helper first resolves stored actor ID+4, replaces it with invalid sentinel, UNCONDITIONALLY emits Lua NPC_INTERACTION_END() via39424220(event0x6c), then cleans effect handles when actor exists. Only argumenttrue AND resolved actor+410==1 sends CS066 with actor+418 payload (actual ctor397ab760); missing actor/falsearg/otherkind never sends. Event name was verified via marker-checked READONLY PID48924 event table39d22ce8[0x6c]. Four actual-helper cases in enchant_npc_cancel_oracle.py/native-enchant-npc-cancel-oracle.json qualify mocked external registry/event/effect/sender/view endpoints, actual packetconstructor, test-marker sentinels, and no live sends. Exact mode order390467ff..46813: clearedslots/newmode FIRST, old0?nonzero invokes39046600 registerobservers?NPC_INTERACTION_END?ENTER_ENCHANT_ITEM_MODE. Nonzero switches directly emit SWITCH without NPCcancel; same mode and leave do not invoke that helper. NPC_END sees NEW mode and cleared selection. Actor-specific cleanup and CS066 may only be reproduced with actual received interaction context, not unconditional wire.

GetItemInfoByType optional flags:39481ec0 registers3947f2f0 via required-itemID adapter39481d20/39481900.3947f318 initializes flags=-1; parameter count>2 initializes an output0 then reads third USER argument39058d10(index3), ignoring failure. Only numeric ScriptAnyValue4 converts f32 CVTTSS2SI. Missing flags therefore=-1; explicitnil/string/table/wrongtype=0 without zero-result refusal. This is the fourth Lua argument including self. Optional gradeindex2 only replaces defaultglobalbyte after successful numeric read, taking low8. Do not conflate missing with explicitnil.

Pure PRE-row lookup API: NativeItemTaskStorage::{Unknown,Absent,Present{records,extent,container}}, NativeItemTaskInventory{equipment,bag,bank}, lookup_native_item_task(task,&inventory). No owning mirror or automatic mutation/observer. None record means UNKNOWN; Some(empty Item) is actual record. Absent requires genuine native null storage, not missing app data. Equipment extent must be28; container extents must be explicitly known min(nativeconfigured header+9,10*enabled chunks header+a), never inferred from legacy Option vectors. Count physical lookup397cce50 includes equipment1/fc/f2 aliases, BAG2/64/65/66 routing with actual header container equality, BANK3; empty template still resolves. Unsafe equipment index>27 returns Unknown as app safety refusal (native helper unchecked). Remove397cdad0 uses397ccf40 equipment0..27 first, ID-only without template filter; BAG3961e9a0 then BANK3961e8f0, both skip template0, scan ascending physical slots and return first match. Earlier unknown blocks certainty in later matches; known complete miss is Missing. Container4 and RemoveID0 do not perform lookup. Six actual native lookup cases execute397cce50/397ccf40 with all called container helpers, NO function mocks; explicit synthetic records/headers, runtime-verified invalidtemplate/packed0. Artifacts item_task_lookup_oracle.py/native-item-task-lookup-oracle.json. Actual source inputs/extent/certainty remain caller responsibility, not proved by that oracle.

Header/physical storage provenance: OnCharacterInvenInit391d34c0 takes LOWBYTE of received u32 BAG+0c/BANK+10;3961bcc0 stores header+a=floor(u8input/10), header+9=10*that. This is enabled count, NOT allocation. OnInvenExpanded391d3680 container2/3 calls same helper on receivedslotbyte. Contents391d3550/3961df90 copies declared blocks of10records into start_chunk position without changing header or clearing other rows. SC04c factory391a918a constructs only5packetblocks: counts>5 are native-unsafe and must not silently qualify an app native view despite full raw lane retention. Static manager ctor3961eb40..c1 constructs BOTH BAG(manager+10) and BANK(+61a0) as13blocks=130physicalrecords, independently memsets each0x6180 block. OriginalPID48924 both actual staticcontainer vtables verified399f9bbc. Therefore even genuine bank_slots200 enables a native unsafe header: pure lookup allows known early hits below130, but returns Unknown for a search requiring continuation into index130 or a header-admitted physical index>=130. This is app safety refusal, not native truncation or Missing. Raw header/rows remain untouched.
Packet chunk ctor3961e690 constructs10items then memsets ENTIRE0x780 block0; Item serializer3961f500 template0 returns immediately at3961f524?614, preserving that zero record. Thus only a genuine SC04c empty row has source-proven allzero normalized Item metadata (world0). Native Remove/Clear3924cd30 instead resets worldbyte+a8=ff. Do not reconstruct either origin from generic legacy None/postfold emptiness; snapshot provenance and successful source framing matter.

Normalized empty reset mapping is fully sourced by3924cd30 and serializer3961f500: ID/template/grade/flags/count/detail/create_time/lifespan/made_unit_id/unsecure/unpack zero, world_id255. Grade/template/made-unit sentinels39d2cba0/ba4/b9c were runtime-verified0 via READONLY originalPID48924. A genuine source Clear can therefore be represented as Item{world_id:255,..default()}, while genuineSC04c empty is fullzero. Neither is inferred from an arbitrary legacy None. Header oracle inventory_header_oracle.py executes3961bcc0 with12edgecases/no function mocks. Source archive native-item-task-lookup-header-source.txt preserves exact constructor/receipt/lookup/flags/NPCcancel bytes. Lookup milestone full aa-net Bevy suite567passed; no new persistent state, observers or automatic commands.

Equipment receipt provenance continuation (read-only, no new state): SC040factory391e99bd?391e8850?391aeb90 constructs28Items atCharacter+140 using3901cb10; SC069factory391e4533?391d0c90 constructs28 atUnitState+568 withsamector. Constructor normalized defaults are zero fields exceptworldFF (separate ctor sentinels39adacf8/f4/f0 runtime-verified0). Full-item serializer3961f500 leaves those defaults for template0. Native Character39620255..a6 reads slots19..25 as bare templates then3924cdb0; other slots full3961f500. Unit equipment3961fa30 similarly uses bare19..25 except format2. Template setter3924cdb0 produces ID0,grade0,flags1,count1,detail/metadata0,worldFF even when template0. Thus app template_only rows are appearance projections; their normalized defaults are not automatically full native records. IDs0 on source-reconstructed bodyparts suffice for nonzero-ID Remove prefix scans, but no generic full snapshot claim for NPC compact equipment.

UpdateDetail task9 continuation:397d1030 table9 selects397d112e?397cdc10. It performs declared physical397cce50 lookup, ignores packetID and template validity; missing physical returnsfalse. Found copies task+10's0x80 union bytes toitem+18, then copies whole outputitem andreturnstrue. ID/template/grade/count/othermetadataunchanged. Unsupported native PRE-row mirrors can invalidate ONLY that declaredphysicalrow, not legacy ID-relocation matches. Current parsed ItemTask::Update.detail_payload preserves parsed-prefix bytes and tolerates malformed lengths; it does NOT retain full128-byte union/padding. Full-union transport retention is a separately bounded prerequisite before claiming generic raw-native Update fidelity; no Update projection added in this lookup milestone.

Active lane consumption source (read-only): skills/skill.rs1529 queues source consumption after appliedeffect when consume_source_item OR items.use_skill_as_reagent;1695+ calls EconomyForSkills.consume_item_by_id. economy/bridges.rs849 resolvesholdingcontainer and calls consume_item(SKILL_REAGENTS40,template,amount,preferredID). models/container.rs762 prioritizes requestedID thenascending physicalslots; consume_one uses Count4 for remaining>0, otherwise emits a separate immediate Remove7 packet via remove_item_task, and finalaggregate40batch may beempty. These are actual lane receipts, not proof of native callback admission from final Inventory state. PRE-row lookup/projection and output-copy timing remain necessary.


### Received Update9 union and qualified pure detail/grade projection

ItemTask::Update now additionally retains raw_detail_block:Option<Vec<u8>>. Actual parsing supplies Some(exact consumed declared block), including tag and padding; synthetic constructors use None. Existing tolerant detail and parsed-prefix detail_payload behavior remains unchanged, including malformed/unknown/empty blocks and outer framing. Other padded task readers borrow/drop the block without introducing an extra raw-block allocation. Native serializer397cfd40 passes a fixed0x80 capacity to inputstream virtual+d8/output+dc at397cfd96..c3. Raw retention does not claim native safety for non128 lengths or unknown tags.

lookup_native_item_task additionally routes Grade14 and Update9 through the same source-exact declared physical lookup as Count. project_native_item_task supports Grade14 original397cde80: ignore packetID/templatevalidity after knownphysicallookup, copy OLD whole output before assigning NEW rawgradebyte, mutation intent Replace; no container4 bypass. Update9 original397cdc10: knownphysicallookup, ignore packetID/templatevalidity, replace detailunion then copy NEW output. The pure normalized Update projection requires received128-byte raw block, supported detailtag0/1/3, and successful reparsing whose detail AND exact parsed prefix match the supplied legacy fields. It refuses synthetic/unknown/malformed/inconsistent blocks or unknown/stale context. Output remains normalized Item (the raw complete union remains preserved on the receipt), without inventing reserved-padding metadata in Item. No automatic Inventory mutation, native observer, pending/controller continuation or timer is added; legacy reducer still ignores the new raw field and preserves its existing ID relocation/guard behavior.

Independent native-item-task-detail-update-oracle.json and native-item-task-grade-update-oracle.json execute actual helpers plus actual397cce50/3961e850/3961e700 physical lookup. Registry/actor admission is not inferred: explicit synthetic manager rows/header supplied; observer/output-copy and standardmemcpy endpoints are qualified mocks. Deliberate packetID999 versus declared physical recordID123 is admitted in both actual bodies. Grade live6?8 but output6; Update output reflects newunion while ID/template/count stayunchanged. Missing physical emits noobserver/mutation. native-item-empty-origin-oracle.json executes original ctor/reset/template setter with onlystandardmemset mocked and runtimeverified zero sentinels, independently confirming bodypart flags1/count1/worldFF and fresh/reset emptyworldFF. Source/archive bytes are native-item-task-lookup-header-source.txt plus the oracle scripts under data/extract/aa-net.

Literal wire tests preserve a nonzero128-byte padding tail, raw signed mateXP/noncanonical dead/levelbytes, and a following Money task; every truncated prefix refuses at the individual reader. Tolerant malformed/unknown/oversized lengths retain exact consumed bytes without alignment changes. Pure tests distinguish Grade output-before from mutation-after, Update output-after, actual physical ownership despite packetID mismatch, zero/unsupported/synthetic/inconsistent raw blocks, supported None/Mate/Equip normalized details, and missing/unknown physical context. Root aa-client's four synthetic Update literals require None; the fifth production search hit already uses a rest pattern.

Update/Grade milestone verification:14focused pure-projection tests and full573 aa-net Bevy lib tests passed (0.20s full), Bevy cargo check passed. Exact owned scope items.rs/item_task_projection.rs/state/gear.rs/docs/aa-net.md; no legacy reducer behavior change. Complete source excerpt native-item-task-detail-grade-source.txt includes serializer397cfd40 and both original mutation bodies.


### Native Add5 / Move8 pure PRE-row projection and equipment provenance

`ItemTask::Add.raw_detail_block: Option<Vec<u8>>` retains exactly the consumed declared padded detail bytes on real Add/Take/ChangeOwner receipts (`Some`); synthetic tasks use `None`. Legacy normalized Item/prefix parsing and reducers are unchanged. Original serialized Item body397d0a00 reads template first and, for nonzero template, ID/grade/flags/count, fixed-capacity128 detail then metadata. Active lane `models/item_task.rs::write_task_item_details` always writes the full body with declared128. Template0 original early-return versus lane full-body behavior remains a boundary: pure Add projection refuses template0 rather than inventing constructor metadata. No native clamp is imposed on raw parser retention.

`lookup_native_item_task` and `project_native_item_task` now support **action5 only**, using actual physical PRE-row context. Actual397cda20 bypasses container4 OR5 with true/untouched output; otherwise missing physical record is false. Existing record ID/template do not gate replacement. Observer methods run before assignment397ad440; assignment copies all serialized Item fields and entire detail union, then output copies the NEW Item. The normalized projection requires raw128, supported detail tags0/1/3 and reparsed detail/prefix equality. Unknown/mismatched context or unsupported/inconsistent detail refuses projection; metadata is copied from the received Item, never reconstructed from final Inventory. `output` is post-write; `mutation` is full Replace. This does not dispatch observers or change legacy inventory semantics.

`lookup_native_item_task_move(task,inventory)` returns the pair of explicit PRE contexts. `project_native_item_task_move(task,first,second)` returns `Option<NativeItemTaskMoveProjection { admitted, first, second }>` with two ordered `NativeItemTaskMutation` intents. Actual Move8 helper397cfb00 resolves FIRST physical endpoint then its observers, SECOND endpoint then its observers; both observer sets precede any swap. Missing first returns false before second; missing second returns false after first observers but before mutation. Packet persistent IDs are ignored. Container4 skips only its own lookup/observers and disables swapping, while the other endpoint must still resolve; both virtual endpoints return true with no writes. Real endpoints swap full records with first-write then second-write; same physical slot remains unchanged and contradictory same-slot PRE records are refused (including equipment route aliases1/fc/f2). Dispatcher397d10ee clears the output before calling Move; **neither moved Item is a consumed-input output**. This pure helper deliberately exposes no moved-item output, and implements no observers/controller/history.

Independent local Unicorn oracles `data/extract/aa-net/item_task_add_oracle.py` / `native-item-task-add-oracle.json` (four cases) execute actual397cda20, assignment397ad440 and physical lookup/container helpers. `item_task_move_oracle.py` / `native-item-task-move-oracle.json` (six cases) execute actual397cfb00 and actual lookup. Only observer vtable endpoints and standard full-Item copy/temporary-copy/memcpy endpoints are mocked, explicitly recorded. Move traces sample BOTH live records at every observer, proving pre-swap order; wrong packet IDs, missing second, same slot and virtual endpoints are included. Static caller/serializer bytes are archived in `native-item-task-add-move-equipment-source.txt`. Rust tests independently cover literal Add framing/all truncations/nonzero padding/following-task alignment, full overwrite/output order, raw qualification, unknown/missing/virtual and ordered swap cases.

Local equipment certainty can be maintained per genuine SC094 row, without retiring all equipment: opcode factory391ab010 constructs28 full Item records using3901cb10 (empty normalized record worldFF, not SC04c world0). Serializer397c6230 consumes min(declared count,28), reads signed i8 slot into i32, and full3961f500 Item for EVERY row, including body slots. Handler391d5430 resolves actual registry actor and calls394efae0 or3936b850; **both** copy each received full Item into actor+4b8+slot*0xc0 before local event0xd5(slot+1). The second branch additionally invokes3989f890(1) before that callback; it is not deferred. Duplicate slots apply in receipt order. Safe native projection requires count<=28 and slots0..27; native out-of-range writes and lane over-count framing are not defined safe behavior. Existing raw event rows stay untruncated. Empty `Option<Item>::None` may only become constructor-derived worldFF when this actual receipt provenance is established, never from arbitrary missing legacy state.

SC069 equipment mode is actual wire actor kind, not an independent local/full flag:397c9523 passes UnitState+438 into397be0c0, which zero-extends the received kind byte to that field, then397c95f4 passes it unchanged to3961fa20. Local Character kind0 therefore reads bare body templates19..25 and reconstructs via3924cdb0 (flags1/count1/worldFF even template0); the mode2 full-body exception is Slave kind2. SC040 Character body-template handling remains separate. Current app `EquipFormat::Full` is not proof that every native actor kind has identical body framing. No new SC069 decoder behavior is claimed here.


The same pure lookup/projection now covers additional source-closed task cases without changing legacy reducers:

* Take6 uses exactly Add5 dispatch branch397d10d1 and helper397cda20, including container4/5 true/no-write bypass and the same raw128/nonzero-template qualification.
* ChangeOwner15 uses397cdef0: actual inventory manager must resolve BAG header routes2/64/65/66 or BANK3 through3961e850; equipment and virtual4/5 return false. It then invokes the same full397ad440 assignment and copies output AFTER mutation. There is no incoming-ID removal/search, equipment route, or virtual-success rule. Qualified raw128 and explicit PRE context remain required.
* Flags10 uses397cdc90: physical397cce50 lookup, no packet-ID/template check, observers before `flags |= bits`, full output AFTER. Zero bits still admits a found record. Security11 uses397cdd00: same physical lookup, replaces flags and both raw signed timestamp bit patterns, output AFTER. **Only is_unpack==0 is projected.** Nonzero is_unpack can also mutate Equip detail timestamp through static item resolution39695dc0 and397cd100; without those providers the pure API refuses that branch rather than preserving stale exact detail. The other two raw bool bytes do not gate the admitted zero-unpack assignment.
* Seize13 uses397cdde0: physical lookup, ignores packet ID/template, copies full output BEFORE3924cd30 clear, returns true even for a known empty record. It has no Remove7 ID search, ID0 refusal, or resolved-slot/template admission check. Clear intent retains the prior explicit caller responsibility to construct actual reset-worldFF records.
* RemoveCrafting12 maps directly to false397d1242 in the actual397d124c jump table: no native Item mutation or output/observer production. Its pure result is admittedfalse/no mutation even with unknown context. This is a native/lane distinction, not permission to replace lane authoritative receipts or change legacy cache behavior. Money1/2, GamePoint3, AA16/17 and AutoAA18 mutate separate stores rather than equipment/BAG/BANK Item records.

`native-item-task-flags-security-seize-oracle.json` contains seven actual helper cases (wrong packet ID, OR0, signed -7/-9 times, missing physical, known-empty Seize). Actual397cdc90/397cdd00/397cdde0, physical helpers and clear3924cd30 execute; only observers, standardmemset and full output copy are mocked. `native-item-task-owner-oracle.json` contains five actual397cdef0/397ad440 cases: replacement, missing, virtual4/5 and equipment refusal. Source scripts and `native-item-task-flags-security-seize-owner-source.txt` archive exact bytes/table. These tests prove conditional field mutations and output order, not controller continuation or dynamic static-dependent unpack eligibility. All functions remain pure, with unchanged public signatures, no observer implementation, manager or automatic reducer.


### Forced-removal tail and controller notification admission correction

SC090 factory391e3760/vtable399b7644 delegates397ab1f0 to body serializer397d1d00. It reads named `forceRemoveCount` into body+4838 (packet+4848), up to30 named `forceRemoves` u64 values at body+4840 (packet+4850), then trailing typeID body+4930 (packet+4940). The forced-ID loop397d1db1..1de2 performs only archive primitive reads, with no Item/cache mutation. Actual ordinary handler391d52f0 passes only reason, taskcount, taskrows and trailing typeID to394eff00, never the forced-ID count/array. Thus native ordinary SC090 does not produce forced-ID Item erasure or0x1c notifications. An application may deliberately retire certainty for matching received forced IDs after all task rows as a stronger lane-mutation safety policy, but must not call that native removal/output or synthesize a controller input latch. Declared raw lane rows/IDs remain preserved; native reader count30 behavior is not a new parser truncation policy.

`item_result_controller_dispatch_oracle.py` executes actual receipt handlers39185bd0/39185d70/39185f00 in22 cases (`native-item-result-controller-dispatch-oracle.json`). Explicit mocks are manager getter, current Item lookup, link formatter, Lua dispatch, internal dispatcher and log endpoints. Grade09c admits signed decoded result0..4 plus successful inline Item link, then emits Lua0x98 BEFORE internal0x44(arg0); result5/255/-1 or link failure emits no internal notification. Grade has no active-Game gate. Gem09d and Stone09e require active Game, found current Item and successful link; admitted receipts emit Lua0x99/9a BEFORE internal0x45/46(arg0), independently of result success. Gem decoded result2/255 gives false success yet still notifies. Direct decoded install/bool255 does not gate dispatch. These executed handler inputs are NOT proof of arbitrary noncanonical wire archive-bool normalization: frontend canonical0/1 refusal remains an explicit stronger safety boundary. Neither result notification is generic TaskDialog removal.

**Producer/receiver correction:** the earlier conditional39046430 observer oracle showed what an internal0x15 envelope with code!=0 would do; it did not prove that ordinary Fired can produce that envelope. Actual3939b010 at3939b1a6 tests low byte arg+28, decoded SkillResult.code from packet+154. NONZERO calls39395b50(TL) then exits through3939b8a5 BEFORE internal0x15 at3939b7d8. This guard is reached independently of successful actor resolution. `fired_result_exit_oracle.py` executes that actual fragment in six cases, with only timeline-prep endpoint mocked; code0 continues, code1/2/255 exits. Artificial dword256/257 demonstrate low-byte arithmetic only, not wire inputs. A static direct-call/immediate-push0x15 scan finds the ordinary Fired emitter; other near matches394d7692/76a5 are registrations, not emissions. This does not claim exhaustive indirect/dynamic caller absence. **Do not synthesize the controller failure latch from every received Fired error.** Full retained-record/earlier Fired admission is still unimplemented.

Actual CastingStopped39396000 likewise needs FIRST existing native timeline record via39394e70, then resolves CURRENT registry actor using tracked caster kind0/3 object+8 or kind2 object+14; missing record/actor exits. It runs39395b50, optional duration insertion,394e2650 then builds internal0x18 using TRACKED raw skill. There is **no explicit local-player comparison** in that producer. Receiver39046573..5a7 checks valid payload,39044640 skill/mode match and active controller, without inspecting caster/local ID. A local-only app policy can be stronger but is not native admission. Ordinary `Skills.casting`, generic received SkillEnded/SkillStopped, unique receipt TL or current-cast-pair presence does not prove retained native first-record/lifetime identity. No controller completion is newly enabled by this audit.

Mode validator39044640 accepts mode1 only via grade set395a72a0; modes2/3/4 compare source-verified runtime globals39ac3214=23728,39ac3218=23729,39ac3210=11285. Other modesfalse; no caster/TL/actor check occurs inside this validator. Exact source ranges including serializers and receipt/stop gates are archived in `native-enchant-receipt-stop-admission-followup.txt`. Actual-byte tests qualify external endpoints and do not replace the unresolved retained timeline lifecycle model.


Retained TL audit continuation (source-only): `timeline_equal_insert_oracle.py` executes actual39399e90 insertion and39394e70 lookup on a valid synthetic one-node tree. Three attempted equal-TL replacements with caster kinds0/2/3 and different raw skills/objects reject the new node and retain original skill/actor/ref/count. Only rejected-new-record destructor39399780 and allocator free are mocked. It is a UNIQUE TL map, not a duplicate map ordered by actor or receipt. `timeline_frame_retirement_oracle.py` executes actual3939c5a3 and3939c649 comparison fragments with NO function mocks over13 cases: refs304/308/30c sum wrapping32 and signed<=0 queues deletion; otherwise wrapping_u32(now-start) UNSIGNED strictly greater than timeout begins cleanup, equality keeps. Source default timeout10000, or wrapping record138+13c+140+10000 with descriptor. Actual physical tree erasure remains3939a0e0, separately executed in native-timeline-erase-oracle-20261018.json. These comparisons do not provide renderer/projectile reference increments/releases, retained object identity or unclosed mode1 reset caller. Those inputs/lifetimes remain the explicit blocker to a network-only generic timeline provider, not an invitation to reset on every End/Stopped or actor ID reuse. No timeline manager or controller notifications added.

### Explicit static recharge context for Security11 unpack

`project_native_item_task_with_recharge(task,context,known_recharge_buff:Option<u32>)` extends only the existing pure projection. The old `project_native_item_task` delegates `None` and retains its nonzero-unpack unknown default. `Some(0)` means a genuinely complete static lookup found no eligible recharge (including missing Item descriptor/class row); `Some(nonzero)` is the resolved recharge buff ID; `None` is unavailable static knowledge. The value must belong to the actual physical PRE-row Item template, not the packet's unchecked persistent ID. No catalog/manager is owned or auto-populated here.

Exact root catalog input: use actual `items.impl_id`; impl1 resolves `SELECT item_id, COALESCE(recharge_buff_id,0) FROM item_weapons`, impl2 `item_armors`, impl24 `item_accessories`. Other impls or genuinely absent descriptor/class row are known0 only after complete catalog loading. Current compact copy has5421/8024/491 rows respectively, all unique item_id, with5/17/0 nonzero recharge IDs (accessory values NULL). Native397cd100 calls lookup396a6d40/39695e00/396a6d00 using Item descriptor+0 item ID, and reads recharge ID from class+2c/+24/+18. Loader SQL columns14/13/8 are `recharge_buff_id`; weapon396bde0f stores local-68 then396be068 copies descriptor beginning-94 through396a7590 (source+2c?dest+2c); armor396cc1de stores-6c then396cc442 copies base-90 through396a7500 (+24 unchanged); accessory396bd8bb stores-5c then396bdb09 copies to+18. Native invalid-buff sentinel3a5e11a8 is0, verified from original PID48924 with StandMode marker and PROCESS_VM_READ only (`native-unpack-recharge-sentinel.txt`). Raw source/SQL is `native-unpack-recharge-catalog-source.txt`.

Actual397cdd00 nonzero unpack always replaces flags/security timestamps, then ONLY actual union kind1 plus found static Item plus nonzero397cd100 recharge changes equipment charge_time to received unpack_time before output. Projection updates BOTH typed EquipDetail.charge_time and exact parsed detail_payload bytes11..19, requiring a complete55-byte payload reparsing to the existing typed Equip fields; reserved bytes are preserved. Unknown/malformed/type1-as-Other payload refuses eligible charge projection. Known0 leaves the detail untouched, and known non-Equip types skip charge assignment. Output remains AFTER all admitted changes; no automatic reducer/observer/callback or fabricated charge timer is added.

`item_task_unpack_recharge_oracle.py` executes actual397cdd00,397cd100 and physical lookup over nine cases (`native-item-task-unpack-recharge-oracle.json`): weapon/armor/accessory, zero buff, missing class, missing Item, other impl, non-Equip and zero unpack. Only static lookup endpoints, observers, standard primitives and Itemcopy are mocked, with explicit synthetic descriptor fields. Noncanonical decoded is_unpack255 takes the nonzero native branch. The raw charge102 becomes signed-9 only in three eligible Equip cases; output contains the new charge. This is conditional field/control-flow proof, not live-server charge acquisition or catalog completeness inferred from missing app rows. Rust regressions cover raw reserved bytes, known0 versus unknown, missing/malformed/type-mismatched payload, extreme signed timestamps and unchanged original records.


### Original SC093 detail replacement: raw receipt and pure BAG mutation

`SCItemDetailUpdatedPacket`0x093 now decodes/forwards `ServerMsg::ItemDetailUpdated(items::ItemDetailUpdate)` / `Event::ItemDetailUpdated(...)`. Public fields are `item_id:u64, container:u8, slot:u8, raw_detail:Vec<u8>`; raw_detail includes tag and exactly its received prefix. Wire is u64 ID, u8 container, u8 slot, tagged detail with NO length prefix/padding. Original serializer397b3840 delegates3961edf0: payload lengths tags1..9 are55/29/6/9/24/16/16/8/4; tag0 and other byte values consume tag only. In particular native tag2 here is FIXED29, not generic lane Item reader's conditional repair-time walk. All bytes/order remain raw; no unsupported tag semantics are invented. Current active lane has opcode/name093 but no writer found, so this is original-source compatibility, not a claimed observed lane producer.

Original registrar391bea80 assigns this factory at dispatch+250 (opcode93; following94 is+254), callback registration391ec081 chooses391d53b0. Factory391aaf60 explicitly memset-zeroes128-byte detail union before serializer. `native_detail_block()->Option<[u8;128]>` therefore DERIVES a ctor-zero tail after exact received prefix; it does not claim128 wire bytes. `detail()->Option<ItemDetail>` validates exact prefix size and decodes0/1/3, retaining other tags as Other with original prefix bytes. Malformed synthetic prefixes return None atomically; truncated received fields fail the decoder. Unknown native tag consumes one byte as the original reader does, preserving following compressed packet alignment. No changes to generic Item, padded Update9 parsing or legacy Inventory reducer.

Actual callback391d53b0?394e47b0 has no active-Game lookup: it resolves ONLY static BAG through3961e850, checks FULL item ID equality and valid/nonzero template, then copies128 bytes to its detail union BEFORE releasing packed pending key through397d2200(key,false). Wrong ID, empty template, absent/disabled slot and BANK/equipment routing do nothing. ID0 with valid template is admitted; there is no blanket ID0 rejection. Pending false branch397d2281 erases its map entry then invokes installed inventory+C358 callback(key,false); this audit does not invent a named Lua event or task/controller completion. No ItemTask output or internal0x1c consumption notification is produced.

`lookup_native_item_detail_update(update,inventory)->NativeItemTaskContext` uses explicit PRE BAG storage/certainty and extents. `project_native_item_detail_update(update,context)->Option<NativeItemTaskMutation>` returns None for unknown/malformed/inconsistent context, Some(None) for a proved native no-op, or Replace for exact admitted detail replacement. Typed detail and raw parsed payload change together; ID/template/count/flags/times/world remain from the genuine PRE row. Caller owns applying mutation, pending provider and source ordering; this is not an observer, manager, optimistic receipt or consumed-input output.

Session's separate typed052 legacy-inventory guard conservatively retires ONLY the affected BAG slot's known snapshot coverage on every genuine093, restoring it only from actual InventoryChunk. This is an explicitly stronger app transport-safety policy while its private legacy Inventory does not fold093; raw events still forward in order, duplicates included, and unrelated BAG/BANK coverage is unchanged. Root's native Records can use the pure per-row projection immediately, but that does not silently certify Session's stale legacy detail for a new effect request. No new long-lived receipt history or lifecycle-reset equivalence is introduced.

`item_detail_update_codec_oracle.py` executes original397b3840+3961edf0 read/write for14 tags, with only explicit archive primitive/read-flag mocks. Expected literal prefix lengths are independently asserted; constructor zeros supplied from separately proven factory, not inferred received padding. `item_detail_update_mutation_oracle.py` executes actual394e47b0 plus actual physical lookup and original repmovsd128 for six matching/wrong-ID/empty/missing/bank/zero-ID cases; ONLY pending-release endpoint mocked. JSON goldens and exact source are `native-item-detail-update-{codec,mutation}-oracle.json` / `native-item-detail-update-source.txt` under ignored data/extract/aa-net. Self-contained Rust goldens cover every tag/all field truncations, malformed synthetic prefixes, constructor tail, repeated DD04 with following SkillEnded, raw Session forwarding/targeted coverage retirement and actual mutation gates/metadata preservation. No live packet send or server mutation performed.

### SC093 installed pending callback and native SC098 consumer follow-up

The installed inventory callback is source-closed: static initializer39991ef0 passes394e1370 into397d2c10, which stores it at inventory+C358; only a null supplied callback installs the empty397cc270 fallback.394e1370 calls394e12b0. That callback ignores the supplied pending boolean and emits exactly BAG_LOCK_UPDATE(slot+1) for packed container2, BANK_LOCK_UPDATE(slot+1) for3, and nothing for other containers. It does NOT emit BAG_UPDATE or UPDATE_ENCHANT_ITEM. Numeric event IDs2c/2d are resolved by dispatcher3920fd88's39d22ce8[index] and initializers39214623/632 (39d22d98/9c; strings399bee4c/399bee38). False pending397d2281 calls map erase397d1e00, then unconditionally invokes C358 at397d2296 even when no map key existed; only invalid packed sentinel/container4 exits before it. Consequently a qualified SC093 replacement precedes release and one original BAG_LOCK_UPDATE(slot+1), independently of whether an app pending set reported an existing key. This callback alone does not synchronize a separate legacy Item copy or implement controller completion.

True SC098 remains the existing raw ItemLifespan receipt, not SC093: factory391a9240/opcode391a925b/vtable399b339c, serializer397aec40/397d03b0, registrar391bcae0 and callback391d35f0. The handler's nonzero decoded added byte calls394ef9e0 on singleton3a5ad97c; zero calls394ea940. Native manager contains an ordered24-byte-row vector. Add scans full u64 item ID and appends only if absent; a duplicate add does NOT refresh template/time. Clear removes FIRST matching ID, compacts preserving order. Getter397d0400 returns the first matching raw i64 timestamp, or zero. Item ID0 is not rejected. Actual original add/vector-copy/clear/getter bodies execute without hooks in five cache oracle cases; synthetic preallocated vector avoids allocator calls. This is a source/oracle contract, not a newly installed aa-net cache.

Tooltip3942360d first requires a known item descriptor with any of +28(exp_abs_lifetime), +30i64(exp_date), +38(exp_online_lifetime) nonzero. Loader396cb627..65d maps SELECT39a00d90 columns13/14/15 to those fields; copy396ba78b..7a0 preserves them. It queries actual Item u64 ID, falls back to static exp_date only when cache timestamp is exactly zero, and emits expireTime only for a positive signed endpoint. Current time comes from Cry ITimer+6c, the separate Unix-seconds-plus-offset domain, NOT local elapsed milliseconds/TimeKeeper. Emitted integer is low32(endpoint-now); at an expired endpoint, packed container1/2/3/fc substitutes59, other containers0. No newly inferred countdown, charge semantic, expiry deletion or arbitrary timestamp fallback is implemented. Another caller394fecef uses the same cache for a600-second admission check, so this cache is not exclusively a tooltip timer. Existing lane can send charge-start raw_time; retained timestamp must not be relabeled universally as expiry.

Native construction397d2990 zeroes vector begin/end/capacity before reserve.391e9c50 clears its end to begin; direct caller391eadd0 invokes that reset before character-selection request construction.39418294/f7 invoke391eadd0 from selected-character paths. Exact reset-to-app-event association and static exp_date parser/time-domain integration remain pending before production cache addition. Do not equate every lifecycle event with this reset without caller evidence.

Evidence: data/extract/aa-net/native-item-lifespan-pending-consumers-source.txt, item_lifespan_cache_oracle.py and native-item-lifespan-cache-oracle.json. SC093 code checkpoint9073942 passed all586 aa-net Bevy library tests plus Bevy check; this source follow-up adds no timer/controller behavior.

Executed tooltip follow-up adds11 original3942360d..b1 cases in item_lifespan_expire_oracle.py/native-item-lifespan-expire-oracle.json. Only explicit CryTimer+6c sample and Lua integer-field sink are mocked; cache lookup remains original. Cases include equal/expired endpoint59, other-container0, negative cached value suppressing positive static fallback, zero cache fallback, static-all-zero suppression and signed low32 output wrap. Reset provenance is now exact:391eadd0 calls clear391e9c50 at391eae1e BEFORE constructing/sending CS025 SelectCharacter at391eae4a..7d. Root/app should use actual admitted outgoing selection if installing this cache; CharacterSelected receipt is not the native reset producer. Static exp_date parser3967cc00 calls sscanf_s with `%d-%d-%d %d:%d:%d.000`, adjusts year-1900/month-1, then XlMakeFileTime3301ca60; xlcommon.c confirms that endpoint is mktime64 (host LOCAL timezone), not UTC or Win32 FILETIME. A portable adapter must explicitly supply parsed static timestamp/timezone policy. These facts do not convert the charged-equipment lane's unpack-time receipt into an expiry endpoint.

### Item-effect transport guard accepts qualified SC093 refreshes

The private Session item-effect guard now consumes the already-proven SC093 projection before its legacy Inventory event fold. Only an already snapshot-qualified physical BAG slot with a known full PRE Item can supply NativeItemTaskContext::Found. A qualified Replace updates that private guard Item's exact detail/payload while preserving its other fields and known coverage; known native no-op leaves it intact. Malformed/unknown projection or absent full PRE record retires only the target's coverage. A raw093 receipt can never qualify previously unknown history. Raw events retain duplicate/order forwarding; no public legacy Inventory reducer, task observer, pending key or Lua notice is added here. Root owns the separate received Records/pending/LOCK callback integration.

This requires a stricter application provenance fence: any item-mutating ItemTasks receipt, a truncated task list, or nonempty force-removal tail clears all private BAG capture coverage before the legacy reducer runs. Proven currency-only rows and AutoUseAaPoint preserve it. BAG expansion also retires coverage. These are explicitly stronger app transport safety policies; they do not assert native rejection, native force-ID mutation or native cache reset. Legacy task reducers keep their previous behavior for other consumers, but their post-state cannot promote a queued request to known native history. A new genuine safe SC04c snapshot is needed after such mutation; sharing a complete native Records provider with Session may remove this deliberate limitation later.

SC04c requalification now requires the complete declared ten-slot rows, chunks<=5, end<=130 and end within floor((received enabled header low byte)/10)*10. Unsafe/truncated/partial snapshots retire coverage, never synthesize empty rows. Actual received empty rows remain known empty and cannot satisfy an existing-item capture. This bounds native packet/static-storage safety without truncating raw lane parser receipts. The public ItemEffectRequest/Command APIs are unchanged. Loopback tests verify old capture refusal after a qualified093 refresh, freshly captured full-Item acceptance and exact052 bytes; regression cases cover unknown/malformed PRE history, native no-op, duplicate forwarding, task/currency separation, lowbyte header wrap and native snapshot bounds.

### Native SC0A0 item-operation error and ordered slot unlock receipt

items::ItemOperationError { container:u8,slot:u8,second_container:u8,second_slot:u8,error_code:i16 } is forwarded as ServerMsg/Event::ItemOperationError. Original factory391ab480 allocates24 bytes, sets opcodeA0/vtable399b3390 and initial error0; serializer397c1ec0 reads/writes exactly six bytes in that field order, reconstructing packed keys[0,container,0,slot] and sign-extending the last i16 to native i32. Registrar391bedc0 binds callback391d56c0 at dispatch+284; actual callsite391ec0b6 confirms that callback. Active lane calls the same packet SCUnknownPacket in packets/g2c_misc.rs, with independent literal[2,3,1,4,ff,ff]. No current production sender was found in that lane; parser support does not claim a live emitted response or server mutation.

Handler391d56c0 does nothing when error_code==0. Any nonzero signed error first reaches39428370(error), then397d2200(first,false), then397d2200(second,false), with no active-Game or Item-content lookup. pending_releases() exposes precisely those ordered admitted keys; equal keys repeat. Native invalid packed sentinel0 and container4 skip release, while other container bytes remain raw/admitted. Original397d2200(false) invokes the installed C358 callback even if no map key existed; that callback emits BAG_LOCK_UPDATE/BANK_LOCK_UPDATE(slot+1) only for containers2/3. No Item replacement, ItemTask0x1c, request ACK, grade/socket completion latch or TaskDialog continuation is inferred. Session only forwards receipts and does not automatically clear private pending keys or present errors; the app dispatches the source-ordered effects in its canonical context.

Actual original codec read/write executes four signed-edge vectors in item_operation_error_codec_oracle.py/native-item-operation-error-codec-oracle.json with only archive primitives mocked. Five handler cases in item_operation_error_dispatch_oracle.py/native-item-operation-error-dispatch-oracle.json execute actual391d56c0, actual397d2200 admission and actual394e1370/394e12b0 callback. Explicit mocked endpoints are profiling, error presentation, manager getter, tree erase(no existing key), and Lua sink. Cases prove zero silence, error-before-first/second unlock, duplicate same-slot callbacks, invalid0/container4 skip and signedMIN/MAX handling. READONLY PID48924 StandMode-verified artifact native-item-pending-invalid-sentinel.txt establishes3a5e11a4..a7 allzero. Native source transcript is native-item-operation-error-source.txt. Rust tests retain every field, every truncation, repeated DD04 alignment and raw Session forwarding without automatic content/pending/controller mutation.

SC0A0 milestone validation: three focused tests and full591 aa-net Bevy library tests pass; Bevy check also passes. No live request/payment/server mutation was used. The nearby native091 callback391d5370 is empty;095 writes existing actor equipment IDs,096 targets separate coffer state,097 acquisition presentation. None was promoted to a BAG-detail mutation by this audit.

### Shared native Item records for UI dispatch and transport revalidation

The already-used aa-client enchant Records implementation is now owned by aa-net as plain NativeItemRecords (public native_item_records module, root reexport), with NativeItemRowOutput { admitted, item }. No Bevy/UI dependency or generic observer framework is introduced. Public observe(&Event), apply_row(&ItemTask), apply_detail(&ItemDetailUpdate), finish(&ItemTasks), item(container,slot), current(item_id), owner(), complete_bag(), bag_items() and bag_extent() preserve the frozen root implementation. Whole-event apply(&Event) delegates those same operations for Session; it emits no Lua, pending-key, consumed-input or controller callbacks. Original UI integration continues explicit per-row dispatch, so callbacks see each actual PRE/output/mutation boundary before later rows.

The port retains exact unknown versus native empty distinctions: equipment ctor/reset world255, SC04c whole-block empty world0, Character body slots19..25 template setter flags1/count1, and SC094 full Item replacement including body slots/received duplicate order. Header lowbyte ten-slot enabled extent remains independent of130 physical rows and SC04c five-chunk packet capacity. Unknown earlier equipment/BAG/BANK entries block first-ID lookups; current-result lookup is equipment0..27 before BAG only, accepts first empty equipment ID0, and skips template0 in BAG. Count preserves zero/negative/wrapping output-after; Grade output precedes mutation; Remove uses first native ID lookup before declared-location admission; Move resolves both PRE contexts and never yields an input-consumption Item. Qualified Add/Update retain raw128 admission; unknown mutations retire affected records, never adopt a relocated/final legacy Item.

Root's existing app certainty boundaries remain explicit: force-remove IDs retire matching candidates although native SC090 does not dispatch them; unsafe/truncated snapshot, factual decode loss, raw093 loss and current-actor removal lose global certainty. Same-owner respawn/Init/rows do not silently heal global lost history. Genuine selection/new identity resets the provider; a known row retired by a specifically unsupported mutation can be repaired by an actual full row snapshot. Lifecycle isolation remains the same accepted root policy, not an invented native reset/constructor claim. Static recharge context remains unprovided in this provider, so a relevant nonzero-unpack row stays unknown; the previously exposed pure recharge overload remains available for a future actual catalog consumer.

Session replaces its separate known-BAG set/private093 guard logic with this SAME provider. The legacy private Inventory remains for existing unrelated APIs and explicit received AutoAA setting, but cannot supply item-effect full-record certainty. ItemEffectRequest::recheck_native(&NativeItemRecords,auto_use_aa_point_raw:u8) compares every complete captured Item at its actual BAG slot, then checks the explicit grade AutoAA value (exactly1 is true); owner/active selected header/InWorld/BC/self actor and actual write guards remain in Session. Original recheck(&Inventory) is retained for compatibility. This supersedes the temporary1c97bc6 blanket task-reuse refusal: supported native task projections update the shared records and permit a newly captured request without another snapshot, while old captures reject even when legacy Inventory stayed unchanged. Unknown/malformed/actor-lost contexts still refuse and no protected planner admission is fabricated.

Nine frozen root record tests were ported into self-contained aa-net fixtures, with additional whole-event-versus-staged output and lifecycle tests (11 provider tests). Loopback transport regressions prove native Count ignoring mismatched packet IDs while legacy refuses, stale-capture rejection, exact052 fresh retry without a snapshot, qualified093 refresh without mutating legacy contents, target-only unsupported row retirement, and refusal after global decode loss. Source provenance hash snapshot is data/extract/aa-net/native-item-records-port-source.json. Full601 Bevy library tests and plain/no-Bevy cargo check pass; no live item action or server changes were used.

Retained timeline continuation remains source-only: ordinary physical erase3939a0e0 has direct frame caller3939c785; recursive39399e10 is bulk reset/destruction. Existing Started equal-key rejection and wrapping reference/time comparisons remain covered by prior original-byte oracles. Local cancel helpers393966d0/393967c0 read actual cast/channel pairs, sendCS054, then immediately call39396000(TL,0), including the attached actor path. Reset39171800 invokes those before bulk-clear (except mode3 skips tree clear); these are genuine non-receipt internal18 producers. Enchant mode helper39046740 only invokes local cancel when changing to mode0, actual local castTL resolves through393953a0/first native record, and tracked skill39044640 matches current mode; nonzero-mode switching does not run that branch. Current frontend Combat.casts uses app20-second/cap256 retention and Skills.casting is actor-keyed: neither provides native304/308/30c visual/projectile references or physical node lifetime/reset1 producer. No new TL manager, generic stop shortcut or invented local-caster guard is added.


Security11 non-equipment ordering correction: actual397cdd86 tests nonzero is_unpack, then397cdd8d tests PRE Item detail kind1 BEFORE397cdd97 static descriptor lookup. Known non-equipment kinds bypass every recharge provider and still receive flags/unsecure/unpack fields plus output-after mutation; known missing physical record returns false without static lookup. Unknown PRE remains unknown, and equipment kind1 still requires explicit complete recharge context. This corrects the former unnecessarily broad refusal in the default pure projection/shared provider, without inventing a catalog default. Expanded actual397cdd00/397cd100 oracle now has12cases, including kinds0/3/4/255 and absent static endpoints (native-item-task-unpack-recharge-oracle.json); endpoint mocks remain qualified in that artifact. Pure branch and shared-record regressions verify full-record certainty survives the supported non-equipment row.


### Native equipment persistent-ID receipt SC095

SCUnitEquipmentIdsPacket0x095 is now decoded/forwarded as EquipmentIdsChanged { obj_id:u32, ids:[u64;28] }. Factory391ab210/vtable399b3384 and serializer397b38b0 prove BC3 followed by all28 unsigned64 IDs (227 bytes), without count/optional rows. Lane bevy defines the opcode/name but this audit found no packet writer or production sender; this is original1.2 source compatibility, not a claimed lane producer. Original callback391d54f0 resolves the existing registry actor via39360020; missing actor does nothing.391d5545..5567 copies each ID to actor+4b8 at stridec0 and leaves every other Item field intact; no Lua/pending/controller callback is present.

NativeItemRecords observes only matching current actor receipts. It changes ID in already-known equipment Items, including template0/ID0 and repeated IDs, retaining first-equipment-match order; unknown full records remain unknown and global lost certainty remains sticky. No constructor/default Item, legacy Inventory mutation, BAG update or metadata promotion is inferred. Actual original serializer read/write runs three vectors (zero/asymmetric/max obj IDs and IDs) in equipment_ids_codec_oracle.py/native-equipment-ids-codec-oracle.json, with only archive primitives mocked. The actual28-record copy loop runs without hooks in equipment_ids_handler_oracle.py/native-equipment-ids-handler-oracle.json, verifying every other actor byte unchanged; registry guard outside that executed fragment remains source-proven. Rust regressions cover every truncation, duplicate compressed-batch alignment, raw Session forwarding, empty-template first-hit/ID0, wrong actor, unknown metadata and lost-history boundaries.


SC069 duplicate actor audit (no production change): existing Session own-object duplicate suppression is not an equipment refresh gap. Registrar391ebf8d passes callback391d4c30 to391e43d0 (opcode69); callback delegates received state to391b90a0 outside the engine special-mode branch. Its visible391b90bf..90cf first resolves received objectID with39360020; any existing registry actor jumps directly to epilogue391b950c before construction/hydration/equipment copy. Actual original branch fragment executes both existing/absent cases in unit_state_duplicate_admission_oracle.py/native-unit-state-duplicate-admission-oracle.json, with only registry lookup mocked. The protected entry jump391b90a3 and complete constructor remain outside that executed claim. Thus no additive refresh event, invented second actor or blanket retirement is justified by this receipt; actual094/095 remain the admitted existing-equipment mutation paths audited here.


### Shared complete recharge catalog for actual equipment-unpack projections

NativeItemRecords::set_recharge_catalog(Arc<HashMap<u32,u32>>) supplies a COMPLETE successful static lookup result keyed by Item template. Missing map entries mean known recharge0 (including missing item/class rows); missing catalog remains unknown for actual Equip kind1 nonzero-unpack. It calls the existing pure project_native_item_task_with_recharge with the actual PRE Item template and preserves all raw-prefix/normalized-detail qualification. It never repairs already-unknown records when a catalog arrives. Arc clones share static storage across HUD copies and every selection/lifecycle reset while runtime records retain their existing reset rules.

ClientOptions.item_recharge_catalog:Option<Arc<HashMap<u32,u32>>> defaultsNone and seeds the Session NativeItemRecords from exactly the same supplied Arc. Thus the UI staged provider and actual transport recheck can agree after a supported Security11 task; no one-sided HUD-only certainty or fallback to legacy postfold Inventory is introduced. Root owns successful readonly catalog loading and supplies the shared Arc to BOTH ClientOptions and HUD defaults; no network command, UI callback, automatic item action or static-query implementation is added in aa-net.

Complete input must resolve actual items.impl_id1/2/24 through item_weapons/item_armors/item_accessories by child.item_id=items.id, then COALESCE(recharge_buff_id,0); other implementations/missing complete descriptors resolve0. Readonly compactDB has22 nonzero child rows (5weapon/17armor/0accessory), but ONLY12 have matching actual Item descriptors (2weapon/10armor). Orphan/mismatched child rows are not actual template providers. Captured source rows/qualification are native-recharge-catalog-actual-rows.json. Self-contained regressions use all12 actual nonzero pairs, known absent map entry, unknown catalog, reserved prefix preservation, no retroactive repair, pointer sharing/reset preservation, plus Session selected-reset then Security11 then fresh captured052 loopback retry without a snapshot. This remains transport/native row projection, not protected Execute/planner admission.


The actual Bevy connection consumes this input: ClientOptions derives Resource only under the existing bevy feature. NetPlugin reads an already-inserted ClientOptions resource (or defaults when absent) and passes it to the existing Client::spawn_thread_with entry point. Unit NetPlugin construction/call sites remain unchanged, no second plugin/command is introduced. Root must insert options before installing the network plugin; CLI walkthrough explicitly leaves the optional catalogNone. Both standalone/no-Bevy Session and actual Bevy transport therefore use the same contract.

Recharge milestone validation: full609 Bevy library tests, own examples with Bevy, and fresh plain/no-Bevy check pass. No live item request or server mutation was used.


### Native/lane specialty ratio receipt SC099

ServerMsg::SpecialtyRatio(i32) and Event::SpecialtyRatio(i32) retain the exact signed four-byte receipt, without inventing item/cache/transaction mutation in aa-net. Original factory391a92c0 assigns opcode99/vtable399b33a8; serializer397aec70 transfers one signed-i32 field. Lane systems/economy/specialty.rs SCSpecialtyRatioPacket writes the same i32 (independent craft_store_tests literal117=75 00 00 00). Actual original serializer read/write five signed-edge vectors execute in specialty_ratio_codec_oracle.py/native-specialty-ratio-codec-oracle.json with only the scalar archive endpoint mocked. Rust covers every truncation, repeated compressed-batch alignment and ungated/duplicate raw Session forwarding.

Registrar391ebd5e passes callback39181fd0 through391bcb80. That callback unconditionally calls394fb610(received ratio), with NO activeGame/actor/interaction gate. Nonzero signed values assign raw cache3a5ba4e0 BEFORE Lua event6b NPC_INTERACTION_START('specialty'); zero first calls error225 while getters still see PRE cache, THEN resets cache from39ac9b74, with no specialty event. Actual394fb610 executes six zero/signed-edge cases in specialty_ratio_handler_oracle.py/native-specialty-ratio-handler-oracle.json: only error/Lua sinks mocked, recording cache at each call. Audited original1.2 runtime constant39ac9b74=100 is supplied explicitly; protected initialization is not claimed. Readonly StandMode-verified PID48924 confirms both constant and cache100 in native-specialty-ratio-runtime.txt. Cache constructor39991f50 copies this constant; no arbitrary world/character receipt reset is inferred. Other write394fbadd resets100 AFTER explicit sale packet sender, not on general task/result/leave receipt.

X2Store:GetSpecialtyRatio registration394b032f..0374 maps394aeb80, ignores arguments and always returns ONE signed-i32-to-f32 numeric cache value via394fb2f0. GetSpecialtyRatioRefund394ae8c0 and actual sale price394aeaa7 consume the same cache and actual current-interaction NPC; full price/provider integration remains separate. Root owns persistent used getter state, error-before-reset and assign-before-original-callback staging; no unused net sidecar is added.

Actual request boundary is distinct: CS043 CSSpecialtyRatioPacket in the lane reads u32id, ignores that field in run, computes ratio from actual active player/backpack and sends099. The lane NPC interaction_option chain has NO specialty flag/service-skill branch, so no SkillList ID or automatic request is inferred. Original specialization callback393c05f0 (named specialty_trader by399d3460/initializer39990dc0) sends CS043 via397aba70 at393c0718 with CURRENT BACKPACK ITEM TEMPLATE ID, not NPC ID. Its native admission includes actual local backpack+1840 nonzero, static item_backpacks kind3, ItemDesc+84 nonzero, specialty_zone_id u16+8e!=0, region price helper398a9b00 and optional skill18458 actability gate3930c020. Original current-NPC identity must remain live for later Refund; no acceptance, sale action, request command or optimistic ratio is implemented by this receipt slice.

SC099 milestone validation: two focused tests, full611 Bevy library tests, and plain/no-Bevy check pass. No live request or sale was sent.


### Specialty quote/request source follow-up (no transaction implementation)

Field correction: ItemDesc+84 is price, +88 refund, and +8e specialty_zone_id, NOT category_id. Exact native items SELECT39a00d90 indexes37/38/40 and loader396cb889..8e3 establish these fields. Request393c05f0 requires nonzero origin specialty zone. Optional skill18458 gate is effective LABOR affordability: SkillDesc+78 is consume_lp (39461b37..42 writes literal consume_lp), +c actability_group_id; actualDB18458 is group31/consume_lp60. No generic skill-point/zero-requirement assumption is valid. Request current-zone helper391826e0 takes NO arguments: it resolves actual local position through gEnv+84 slot50 and region slot38, missing returnsu32MAX; preceding stack grade/output args belong to the following price helper. Exact native current-zone/labor providers remain request prerequisites.

Refund admission includes a separate binding gate: GetSpecialtyRatioRefund registration394b03b7/03d2 maps method394ae8c0 through394af410/394af190. FIRST argument must be actual ScriptAny numeric4, f32 then CVTTSS2SI; missing/nil/bool/string returns zero Lua results. The converted integer is ignored by the actual method, which recomputes from actual current-NPC and current physical backpack. Executed eight-case specialty_refund_adapter_oracle.py/native-specialty-refund-adapter-oracle.json covers missing/nil/bool/string and numeric0/fraction/NaN/infinity: accepted numeric arguments reach the actual missing-current-actor branch and return ONE false. Mocks are ScriptAny endpoints, encrypted interaction-ID endpoint and registry lookup only. No absent-Game dereference semantics are invented.

The actual method returns ONE false for missing current registry NPC, missing NPC-manager row/staticNPC descriptor, missing backpack/class kind3, missing Item descriptor or zero price, or failed static bundle validation. Validation error is presented before false. Static validator395ac920 resolves currentNPC template to bundle, then FIRST child row matching Item template; map-missing with normal_specialty true allows regional fallback, whereas mapped bundle lacking the Item returns error225. Success with coinID0 returns TWO values(true,signed refund convertedf32); nonzero coin returns THREE(true,signed coin amount convertedf32,unsigned coinType convertedf32). Original equipped-item getter remains table(itemType0/lookType0) for VALID empty slots, as previously source-closed; no nil workaround is justified for an artificially positive ratio receipt lacking its ordinary backpack request domain.

Useful fully known common branch: when bundle row exists,398a9bf0 uses only static refund/grade and row profit/ratio; current-zone argument is unused. Base397cd2c0 rounds f32(wrapping_i32(refund*rawGrade.refund_multiplier))*0.01f. Bundle stage rounds f32(wrapping_i32(profit*ratio))*0.001f +f32(base), preserving each SSE f32 operation. Final wrapper394aeaa7 narrows receivedRatio*0.01f to f32, then evaluates NPCfactor*0.01f*thatFactor*bundle in x87 before mandatory FSTPf32 and original3908c8c0. Native rounding is floor(f32(value+0.5f)) then CVTTSS2SI, not generic Rust ties-away rounding. Coin398a9cb0 divides final signed refund convertedf32 by content_config22 convertedf32, then same rounding. Config22 currentDB is10000. Extended x87 intermediate precision must remain explicit when implementing beyond executed input goldens.

Readonly StandMode-verified original DataManager snapshot native-specialty-static-runtime-maps.json closes current input-copy compatibility: NPCfactor map+10964 EMPTY, so actual395ac7c0 default100 for every currentNPC; this is captured1.2 input, not protectedloader/future guarantee. NPC-to-bundle map+109f4 has37 entries exactly matching SQLite specialty_npcs.35 actualNPC descriptors have +e0 exactly COALESCE(npcs.specialty_coin_id,0);14230/14231 lack descriptors in BOTH inputs. All29 referenced bundle vectors/3311(item,profit,ratio) values exactly match DB, with no(bundle,item) duplicate keys. Original vector order is retained in artifact; currentSQL fullscan differs for12 vectors, so malformed/synthetic duplicate first-match behavior must not be silently sorted. Native SQLs:39a03118 SELECT id,item_id,profit,ratio,specialty_bundle_id FROM specialty_bundle_items;399f8124 SELECT npc_id,specialty_bundle_id FROM specialty_npcs;39a01d00 SELECT item_id,asset2_id,asset_id,backpack_type_id,declare_siege_zone_group_id,heavy,normal_specialty,skin_kind_id,use_as_stat FROM item_backpacks.

Twelve full native quote cases execute actual adapter/method,397cd2c0/398a9bf0, empty-tree NPCratio getter, original rounding/coin conversion and typed Lua output helpers in specialty_refund_quote_oracle.py/native-specialty-refund-quote-oracle.json. Actual item20090, grades0/1/6, NPC4473/14766/4482 and ratios100/117/130/-1 produce respective refunds[112280,102118,308464,-1123], [75927,59585,261205,-759], and coin32103 amounts[13,12,33,0]. Explicit mocks are context/registry/static descriptor/grade endpoints, successful bundle-row lookup, actualDB config22 and UNKNOWN current-zone (proved unused in that known-bundle branch); no protected provider completeness or sale action is claimed. Root may use this source-backed common quote with captured currentNPC/full backpack, while regional fallback, effective labor admission and actual explicit request/sale remain separately qualified.

### Specialty known-bundle numeric projection (2026-10-18)

`specialty::quote_known_bundle(KnownBundleQuote) -> SpecialtyQuote` is a pure numeric projection for a caller-proven matching bundle row. It preserves wrapping signed products, binary32 SSE stages, original round/CVTT results (including INT_MIN on invalid conversion), and optional coin division including a known zero divisor. It supplies no NPC, backpack, zone, labor, or sale admission and sends nothing. Inputs are raw descriptor refund/multiplier, first matching bundle profit/ratio, received ratio, actual NPC factor and optional actual coin divisor. Context failures remain the caller's responsibility.

The final original x87 chain is NOT replaced by a binary64 expression. A private bounded u128 significand calculation rounds each multiplication to 64 significant bits, then performs the mandatory binary32 store. Source CSystem::Init36556770 calls _controlfp(new=0,mask=0x30000) at36558783..78a: PC64 on that engine initialization thread. Actual original PID byte identity is archived in native-specialty-fpcw-runtime-byte-identity.json. This is source calling-thread qualification, not a sampled live FPCW register or proof of all worker-thread settings. Earlier quote oracle files with default Unicorn FPCW0 establish only PC24 behavior.

Important oracle correction: the first standalone final-fragment sweep used Unicorn's end address alone, which did not stop execution at a cached translation-block interior. Unrelated later output code overwrote EAX. Those initial sweep conclusions are invalid. specialty_quote_precision_oracle.py now stops explicitly in a code hook at394aeada. Corrected native execution discriminates PC24 from PC64: cost210075, received ratio90, NPC100 gives189068 versus189067; ratio130 gives273098 versus273097. The corrected native-specialty-quote-precision-oracle.json records twelve discriminating cases. Full-method twelve-case adapter oracles used their separate real return endpoint and were unaffected.

Self-contained Rust goldens retain twelve full-method cases and two hundred corrected executed final-fragment extreme/seeded cases from native-specialty-exact-pc64-oracle.json. Executed bytes include3908c8c0 round and the actual empty native NPC-factor tree getter; provider mocks and captured factor100 compatibility remain explicitly qualified in the artifacts. The public numeric API accepts supplied NPC factors without asserting availability or completeness of their static provider. NativeData, item records, timers, requests and legacy reducers are unchanged.

The static backpack gate is specifically item_backpacks.backpack_type_id==3, not a physical ItemDetail tag or guessed implementation.396a6e40(templateID) looks up DataManager+3c68. LoaderSQL39a01d00 index3 is read at396be69c..6a7 into ebp-24; row copy starts ebp-2c, placing it at row+8 (396be74f..754). The actual physical raw-grade byte and resolved ItemDesc.price!=0 remain separate prerequisites. Additional executed corpus covers48 supplied NPC-factor boundary chains (explicit getter endpoint, no provider claim) and16 direct3908c8c0 binary32 round boundaries.
Six further full-method synthetic overflow/wrapping vectors capture raw EAX before Lua binary32 projection (native-specialty-full-wrapping-oracle.json), keeping descriptor lookup endpoints explicit. They verify base and bundle signed wrapping independently of final-chain-only tests.

Coin divisor key proof: nativeSQL399f73d8 is SELECT value,id FROM content_configs. Loader39572050..56 reads column0 into ESI;39572060..63 reads column1 (ID) into EAX;3957206c writes destination[EAX*4]=ESI. Getter395720e0 directly reads DataManager[index*4]. StandMode-verified read-only PID48924 confirms index22=10000 (index3=100), archived in native-specialty-config-runtime.json. kind_id is not the key; SQLite row with kind_id22 is unrelated. Thus the existing twelve-case coin divisor10000 input is qualified rather than inferred from a potentially ambiguous column.

Validation: scoped specialty Cargo run8 passed; full aa-net Bevy library suite617 passed. No live request/sale or server mutation was performed.

### Specialty explicit sale remains a separate admission domain

Read-only follow-up native-specialty-sale-source.txt closes X2Store:SellBackPackGoods registration394b02c4 to wrapper394ade40: all Lua arguments ignored, invokes394fb8a0, zero results. The sale selects interaction-manager39aff9b4+4 (not Refund's encrypted game+1aa0 context), resolves current NPC, checks range39093860 and static specialty/trader flags, current physical backpack type3/price, then branch-specific bundle/origin/refund conditions. NPC+dc is SQL39a01098 index64 npcs.specialty to local-130; +ed is index70 npcs.trader to local-11f; descriptor rowbase-20c and copy396d1039 to396ba8f0 prove offsets. The original branch-specific comparisons use3a5ba4e4, actual runtimeu16=0, an audited1.2 sentinel input, NOT the spatial current zone. Specialty branch requires origin!=that sentinel and successful bundle/quote; trader branch requires origin==sentinel and positive descriptor refund. Both flags are not interchangeable. CurrentDB matching specialty NPC rows include29 specialty=true/trader=false and6 bothfalse; quote eligibility must not be conflated with actual sale eligibility.

Actual accepted changed-actor interaction path393be280 sets manager+4 at393be48d only after its registry/path gates;393be180 clears it at393be19d BEFORE NPC_INTERACTION_END6c. Exact earlier callback/provider lifetime remains a separate prerequisite. Sale constructor397aba60 fills current NPC object runtime ID, packet opcode42/vtable399e6438, serializer397b9830 writes BC3 matching active lane economy/store CSSellBackpackGoods. Native resets ratio100 only AFTER sender394fbad8. No command, automatic sale, optimistic item/payout, or reuse of Refund-only context is implemented.

The separate ratio-request callback393c05f0 still needs effective labor admission3930c020: group proficiency combines397a9330, optional effective unit-attribute390b3a80 and3932a190; rank397a8d10.397a9420 loads rank row397a8dd0, may downgrade one rank when proficiency falls below its threshold, rounds cost*(1-reduction*.01) in SSE with minimum1 for nonzero group, then compares signed with actual player+5fc. Received labor>=60 alone does not establish this provider. This does not affect the pure known-bundle quote API.
Refund target context is now distinct from the sale interaction manager:39429e40 decodes game+1aa0 at39429f25..32, validates changed object, calls39361bf0, then encodes supplied object ID back at39429f8c..99. Its callers include ordinary target paths390f4ad4/391d5193/3921b21b and interaction393be39e. Missing-target branch3942a02b invokes interaction end393be180; valid target changes need not directly clear manager+4. Thus a dialog NPC alone cannot establish Refund's current target. A consumer must supply the actual current target or explicitly conservatively require current-target==dialog-NPC. This is not a claim that arbitrary received SC099 carries target context.

SC084 source chain is now closed independently of the local setter: factory391aae30 writes opcode84/vtable399b3378; serializer397c61f0 reads BC object at packet+c and BC target at+10. Registration391ec05b pushes callback391d5270 through391be9e0. Callback391d52b0..bc calls394270a0(object,target), NOT39429e40. Helper394270c1 resolves object through registry;394270f6..103 decodes resolved actor+1aa0;39427130..13d writes the received target into the SAME encrypted field read by Refund.39427168..176 compares resolved actor pointer to actual game+10c for local-only notifications. Existing Event::TargetChanged and state::Target matching actual PlayerSpawn object therefore form a genuine received-local-target subset; they do not represent all native local setters. Six executed helper vectors (native-specialty-target-receipt-oracle.json) cover local/foreign/unknown registry objects and target0/BCmax, with registry/encryption/component/logger endpoints explicitly mocked and execution stopped immediately after the field write. No later callback/lifetime behavior is inferred.

LP provider correction: actability group descriptor+10 is unit_attr_id, not combat ability. SQL39a14b00 selects id,icon_path,name,skill_page_visible,unit_attr_id,visible; actual group31 has unit_attr_id118.3930c020 filters the attribute through398a8fe0 then reads its effective value through390b3a80 (unit attribute cache+1aa8), adding it to received proficiency.3932a190 is a separate current-house/spatial/deco bonus provider; its static candidate vector397de050 contains group31 in the actual34-key runtime catalog, so absence cannot bypass that provider. Effective A118 and truthful current-house/deco bonus remain unprovided request-admission dependencies; received points/labor/rank alone are insufficient. No partial LP sidecar/calculator is exposed.

### Native labor delta and sufficient specialty affordability bound

SC045 factory391a9a50/opcode45/vtable399b3234 registers callback391d3d60 via391ebded/391bd2c0. Nonzero amount calls3930b4d0 on client+40. Its3930b4f2 instruction ADDs raw signed32 amount into character header+5fc with wrapping; it does not clamp to zero. Negative deltas additionally subtract from consumed-LP+664. Lua eventC2 receives delta then newtotal, each signed32 converted to binary32. native-labor-delta-oracle.json executes seven mutation/event cases through the pre-controller stop, with only Lua dispatch mocked:50-60=-10; INT_MAX+1=INT_MIN; INT_MIN-1=INT_MAX; subsequent recovery-10+20=10. PlayerStats now uses wrapping_add for this actual received field, removing its previous invented max(0) and debug-overflow panic. Public API, bounded app receipt history, timestamps and zero-receipt observation policy remain unchanged; native SC045 skips the LP helper entirely for a zero amount, so the app history is not a claim of native zero callbacks.

A useful sufficient affordability proof does not require an invented A118 or house value. Actual skill18458 cost60, every captured original expert rank reduction lies0..23 and every compact expert_limits reduction0..30; missing-rank397a8dd0 explicitly sets reduction0 at397a8e30. All possible selected/downgraded ranks therefore cost at most60. An exact current native labor value>=60 guarantees the native affordability outcome; below60 remains unknown without the effective proficiency providers. native-specialty-labor-bound-oracle.json executes actual397a9420 and397a8dd0 (including tree lookup/default), with all256 rank bytes at three signed proficiency edges against the captured7-row original map:768 cases, costs46..60. Only missing-row name formatting is mocked. This is conditional arithmetic/source proof, not a new request API or current-labor/actor certainty provider.

Initial LP owner/base closure:391b9884 pushes the actual Character packet,391b9885 passes method-this=new RAW header+0x10,391b988f stores RAW header at client+40,391b9892 calls39089d50. That method39089d96 MOVZXs raw u16 Character+1740 and stores method-this+5ec at39089d9d, which is RAW header+5fc. It does not sign-extend the wire short. native-labor-header-base-oracle.json executes the actual caller and method through this store for wire bits0/50/32767/32768/65535; only name formatting is mocked. PlayerStats now projects the existing bit-preserving Character.labor_power:i16 through u16 into i32 at selection; no raw protocol field/type is changed. PlayerSpawn continues its existing point/object fold and does not invent a fresh labor header. This correction makes initial high-bit values and later wrapping deltas consistent with the native field.

## Duels (`SCDuel* 0x08b..0x08f`, `CSChallengeDuel 0x050`, `CSStartDuel 0x051`) (2026-10-20)

`duel.rs` decodes the five server packets into `DuelEvent` (`ServerMsg::Duel` / `Event::Duel`) and
folds them into the `Duels` resource: the unanswered challenge (challenger character id), the running
duel (both object ids), the combat flag and the character's own id. The layouts are the server's
(`systems/social/packets/duel.rs`) and are pinned by byte tests with every truncation rejected:

| packet | body |
|---|---|
| `SCDuelChallenged` | `u32 challenger_character_id` |
| `SCDuelStartCountdown` | empty |
| `SCDuelStarted` | `bc challenger_obj, bc challenged_obj` |
| `SCDuelEnded` | `u32 winner_id, u32 loser_id, bc winner_obj, bc loser_obj, u8 det` |
| `SCDuelState` | `bc challenger_obj, bc flag_obj` (0: the flag is gone) |
| `CSChallengeDuel` | `u32 challenged_character_id` |
| `CSStartDuel` | `u32 challenger_character_id, i16 error` (0 accepts, 507 refuses) |

Commands: `Command::ChallengeDuel { challenged_id }` and `Command::AnswerDuel { challenger_id, accept }`
(in world only; the answers are `Event::DuelRequested` / `DuelAnswered` / `DuelRequestRejected`).
`DuelNotice` (`Challenged`, `Countdown`, `Started`, `Ended(DuelResult)`) is what the UI reports, in
order. The result is worked out from the side: `det` 3 is a draw, otherwise the first id won; a `det` 2
(surrender) sent to the second id is "ran away" (`result_runaway`), to the first id "opponent surrendered"
(`result_remained`). A `SCDuelStartCountdown` that arrives after the start is dropped (no notice).

The server sends Started and StartCountdown together (it waits 3 s after the acceptance itself), and
puts both characters on the temporary Red / Blue team factions with `SCUnitFactionChanged` (temp flag)
before and restores them after; the existing faction path applies both to units and to the player
(tested in `duel.rs`). Not known without a retail capture: what the original shows on the countdown
packet (the 1.2 text table has "Duel will begin in %d sec.", the 10.x server notes the client runs it for a fixed
3000 ms).

## Titles (`SCAppellations 0x1a4`, `SCAppellationGained 0x1a5`, `SCAppellationChanged 0x1a6`, `CSChangeAppellation 0x0f9`)

`appellation.rs` decodes the three server packets into `AppellationEvent` (`ServerMsg::Appellation` /
`Event::Appellation`) and folds them into the `Appellations` resource (`state.rs`): the owned title ids and the
character's own shown (active) title. The layouts are the server's (`packets/world.rs`,
`systems/quests/packets.rs`, `systems/services/character_packets.rs`), pinned by byte tests:

| packet | body |
|---|---|
| `SCAppellations` | `i32 count`, then per title `u32 id, bool active`; sent while the character is being selected, in chunks of 512 |
| `SCAppellationGained` | `u32 id`: added to the owned list, it does not become the shown one |
| `SCAppellationChanged` | `bc obj_id, u32 id`, broadcast to the players around and to the owner (0: no title) |
| `CSChangeAppellation` (`Command::ChangeAppellation`) | `u32 id`; 0 takes the title off. The server refuses an id the character does not own and still answers with the current title |

* The owned list arrives before the own spawn (character selection) and survives it, so it is not cleared on spawn.
* The title other players show comes with the unit spawn (`Unit.appellation`) and later `SCAppellationChanged`;
  the own title is `Appellations.active`.
* `Appellations` has a revision that the HUD compares to raise its events.

Tests: `cargo test -p aa-net appellation` (6).

## Small server messages, lobby replies and quest cinemas (2026-10-23)

### Coverage method

Every opcode 0..0x230 was fed to `ServerMsg::decode` with an empty body; the ones answering
`Unhandled` were intersected with the lane's sent list (`rust/tools/opcode_coverage/sc_usage.py`,
regenerated against the lane source by the interrupted agent: 348 sent packets). The recovered
four modules decode 37 opcodes, with layouts taken from the lane writer and checked against
the 1.2 client's reader (`x2game_unpacked.dll`; the opcode-to-handler table is the registration
function 391eb9d0, each `OnX` handler stores into `this + 4 + 4 * opcode`):

| module | packets |
|---|---|
| `notice` (`NoticeEvent`) | `SCDominionTaxRate` 0x020, `SCNoticeMessage` 0x0d0, `SCSiegeMember` 0x0ec, `SCSiegeAlert` 0x0ed, `SCConflictZoneHonorPointSum` 0x0ef, `SCTowerDefList/Start/End/WaveStart` 0x16b..0x16e, `SCCriminalArrested` 0x170, `SCBotSuspectReported` 0x184, `SCProcessingInstance` 0x198, `SCTutorialSaved` 0x1a7, `SCPlaySound` 0x1de, `SCSendUserMusic` / `SCUserNoteLoaded` / `SCPauseUserMusic` 0x1df..0x1e1, `SCSpecialtyCurrent` 0x1f4 |
| `unit_fx` (`UnitFxEvent`) | `SCActiveWeaponChanged` 0x06f, `SCUnitImpulse` 0x079, `SCUnitVisualOptions` 0x1cb, `SCUnitIdleStatus` 0x1f5 |
| `account` (`AccountEvent`, `AccountCommand`) | `SCTrionConfig` 0x007, `SCFactionRelationList` 0x009, `SCLpManaged` 0x048, `SCAccountWarned` 0x1a1, `SCRefreshInCharacterList` 0x1cd, `SCResultRestrictCheck` 0x1ce, `SCAccountAttributeConfig` 0x215, `SCLevelRestrictionConfig` 0x217; requests `CSRefreshInCharacterList` 0x021, `CSSetLpManageCharacter` 0x0fe |
| `cash_shop` (`CashShopEvent`) | `SCICSMenuList` .. `SCICSSyncGood` 0x1cf..0x1d4, `SCPremiumServiceList` 0x1d8 (replies to requests this client does not send) |

Fixed transport receipts 0x06d/0x07a/0x137/0x138 now use `TransportReceipt`;
0x139/0x13a already use the separate complete doodad-bond carrier. Transport
control, movement and physical/visual integration remain separate work.
Still deferred: `SCShipyardState` 0x1aa (concrete blob boundary remains open), and
`SCCharDetail` 0x053 (only the answer of `CSCharDetail`, which is not sent; its body is the full
character with every equipment item). `SCDiceValue` 0x134 and the salon packets 0x1f1 / 0x1f3 are
decoded by `chat_command` / `salon`.

Lane layouts that differ from the client's reader (decoded here as the client reads them; fixed by
the server PR "fix/net-wire-layouts"):

* `SCCriminalArrested`: the lane writes the criminal's obj id as `u32`, the client (397b6870) reads a
  3-byte bc, so the real client read both names one byte off.
* `SCTowerDefList`: the lane writes the count as `i32`, the client (397caad0) reads a `u8` (at most 100).
* `SCPlaySound` kind 2: the lane writes the NPC as `u32`, the client (397bea30) a bc (one stray byte).
* `SCCharacterGenderAndModelModified` ends with `bool genderTransfer` in the client (397c47c0); the lane
  writes `u32 0` (three stray bytes, the value is right). Left to the salon work.
* `SCUnitImpulse`: the lane writes `f32::from_bits(obj_id << 8)` after the obj id; the client reads it
  as its target union (`u8` kind 0, bc unit id), which those four bytes are. Decoded as the union.

### What the HUD shows (`aa-client` `hud/notices.rs`)

The client's handlers raise these UI events, which the original scripts handle:
`NOTICE_MESSAGE(kind, alpha, visibleMs, text)` (391d8d60 -> 393c2780),
`DOMINION_GUARD_TOWER_STATE_NOTICE(alert, name)` for alerts 0..6 (391d9520),
`SHOW_ACCUMULATE_HONOR_POINT_DURING_HPW(zone group name, sum)` (39028ca0),
`TOWER_DEF_MSG(text, title, "start" | "end" | "progress")` (3936f8c0 / 3936f9d0 / 3936fae0: the
`tower_defs` start / end message or the wave's `tower_def_progs.msg`, `$1` = the event zone name),
`SYSMSG(TRIAL_TEXT criminal_arrested(criminal, arrestor))` (3907ee60), `BOT_SUSPECT_REPORTED(reporter,
suspect)` (3907ca80) and the `waiting_instance` notice dialog of `SCProcessingInstance` (3936d000).
Not shown yet: `SCPlaySound` (the bubble's voice on the NPC, 393f4f70), the impulse (the client pushes
the unit's physical entity, 391d7750), the idle flag (unit + 0x3ae0 and a name tag refresh, 391e23a0),
the siege member list (`DOMINION_SIEGE_PARTICIPANT_COUNT_CHANGED` only for the player's own siege side,
392f5f60). `SCUnitPvPPointsChanged` 0x204 was already decoded and folded into `Unit::pvp_honor` /
`pvp_kills` (391e2500 updates the unit's points the same way).

### Lobby (`aa-client` `login/`)

* `X2LoginCharacter:RequestCharacterListRefresh` (39484860) sends `CSRefreshInCharacterList` 0x021,
  not a new character list; the answer `SCRefreshInCharacterList` (391d2ff0) stores the nine race
  congestion values and raises `LOGIN_CHARACTER_UPDATED("refreshed")`. The lobby now does the same
  (`AccountCommand::RefreshInCharacterList`); the full list still comes after creating / deleting.
* `RequestLpManageCharacter(i)` sends `CSSetLpManageCharacter`; `SCLpManaged` (394d33e0) raises
  `LP_MANAGE_CHARACTER_CHANGED(character, index)` and `GetLpManagedCharacterIdx` returns that index
  (`Characters::lp_managed`).
* `GetLoginCharacterPremiumGrade(i)` (39485a00 -> 397a7d10): the highest `premium_grades.grade_id`
  whose `point` the character's premium points reach (the character entry's `point` at + 0x17e4, after
  `prevPoint`), at least 1.
* Houses in the character list: `SCLoginCharInfoHouse` arrives once per house after the list (the
  handler 391e9480 only stores it); `GetLoginCharacterInfo(i).houseCount` and
  `GetLoginCharacterInfoHouse(i, n)` (39485c20: `owner`, `name`, `x`, `y`, `curstep`, `allstep`,
  `type` = `house` / `farm` / `fishfarm`, the last two from `housings.category_id` 16 / 17 and 7)
  read them. The house's `zone` key (the zone under the house) is not filled.

### Quest cinemas (`aa-client` `hud/cinema.rs`)

The server never starts a cinema: a step with an `ObjCinema` act (12 story steps, `cinemas.name`
`id_300_*`) is done when the client reports `CSStartedCinema` 0x0cf then `CSCompletedCinema` 0x0ce
(`Interact::Cinema`). These empty request codecs are available, but receiving a quest does **not**
send either request or report a cinema watched/skipped. Native track-view playback and the user's
explicit skip flow remain unimplemented; `X2Quest:UseQuestCamera` stays false.

### Reconnect (N2)

The 1.2 client does not reconnect a dropped world connection. Its network game object raises
`DISCONNECTED_BY_WORLD(title, body)` (CGame vtable 399a6290 slot 3, 390f33c0; it also stops the game
state unless already leaving), and `baselib/disconnect.lua` answers OK with
`X2World:LeaveWorld(EXIT_CLIENT)`, which quits. aa-net therefore keeps reporting `Disconnected` and
going idle; the HUD shows the same dialog and, instead of quitting, returns to the server list with a
fresh login (`hud/session.rs`), a deliberate convenience of this client.

Run this crate's complete recovery checks with `cargo test -p aa-net --features bevy`.

### Packet and session recovery follow-up (2026-10-23)

All inherited staged/unstaged `aa-net` changes were recovered together, including `chat_command`,
`coffer`, the priest blessing request, the vegetation receipt, and the DD04 shop test expectation
that now preserves the snow packet after a malformed item task. Existing command/event variants
and consumer field names remain available. No mount implementation or client consumer is changed.

Additional decoded packets:

* `SCDiceValue` 0x134 (`ServerMsg::Dice` / `Event::Dice`), plus requests `ChatCommand::RollDice`,
  `JoinChannel` and `LeaveChannel`. Channel identity packs the received faction/subtype/type bits;
  join passwords are redacted through all command debug wrappers.
* `SCCofferContentsUpdate` 0x096 (`CofferEvent::Contents`), and `CofferCommand::Interact`, `Swap`,
  `Split` / `moving`. Lane `systems/npcs/doodad/coffer.rs` and `economy/item_handlers.rs` match the
  codecs: bc doodad, u64 container, u8 slot count, then u8 slot plus full unpadded `Item` (u32 zero
  for empty); requests append the u64 container after the normal item-move block, with i32 count
  first for a split. Native factory 391ab290 stores opcode 096/vtable 399b5c34; reader 397ca1e0
  confirms the bc/u64/count/slot/item sequence and its 30-entry inline item array.
* `SCVegetationCutdowning` 0x081: bc unit, bc doodad, forwarded as `Event::VegetationCutdown`; the AAEmu
  encoder golden (unit 0x000100, doodad 0x000200 -> `00 01 00 00 02 00`) is a byte-exact test
  (`game::tests::vegetation_cutdowning_matches_the_server_encoder_bytes`).
* `SCCharBrief` 0x058: u32 character, string name, u8 race. It becomes
  `HousingEvent::CharacterBrief`, cached by character ID in `Houses::character_briefs` as
  `(name, race)`; it is never treated as a unit/house spawn. Native opcode store 391b4f2e and
  vtable 399b3f18 select 397cab60 -> 397c7e40, which reads these exact fields.
* `SCChatSpamDelay` 0x0d3: u8 version, fifteen u8 chat groups, fifteen f32 delays, apply/detect
  strings. `Event::ChatSpamDelay` updates `Chat::spam_delay`; no local throttling/retry is invented.
  Native factory 391adfd0/opcode store 391ae018/vtable 399b5d00 selects 397b45d0 -> 397b20c0;
  helper 397b2060 performs the two fifteen-entry loops. This closes the root's observed L1 0x0d3.
* Proxy `SetGameType` 0x00f: string level, u64 checksum, raw u8 immersive, matching the lane's
  `packets/proxy.rs` writer. `proxy::GameType`, `ServerMsg::GameType`, and `Event::GameType`
  preserve that descriptor without pretending a world has loaded or changing the handshake.
  This closes the root's observed L2 0x00f; its codec evidence is the lane writer.

`Inventory::coffer` folds paged slots and authoritative item tasks using container 4. Identical
pages leave change detection alone; changing either doodad or container identity drops old slots.
Character selection, leaving/disconnecting, and removal of that doodad clear its state, including
the previously missed coffer-only inventory. Closing the window still requires the consumer's
existing explicit `coffer.close()` and close command: the server sends no close acknowledgement.
Wire-shape validation refuses zero/overflow doodad IDs, zero container IDs, empty/self moves and
invalid splits before transmission; permissions, range, restrictions and outcomes remain server-owned.
The normal UI uses bag/coffer endpoints; the codec also retains the lane's equipment/bank endpoints.

Housing character briefs clear with housing state on selection/leave/disconnect. Chat-delay
configuration clears on leave/disconnect. Lobby LP/congestion replies survive own character spawn
and return to character select, but clear on a new authentication/world-select phase so another
world/account cannot inherit them.

Dropped-game handling stays idle and reports `Disconnected`; it never auto-reconnects. Teardown
now clears selected/own object IDs, buffered character state and ping deadlines. Previously an old
nonzero own object ID could make an explicit fresh login miss its own `PlayerSpawn`. The existing
server-authorized `SCReconnectAuth` path back to world select is retained.

`tests/recovered_packets.rs` covers all 42 recovered/new L1 opcodes in standalone frames and DD04
batches with a following sentinel, and checks every truncated nonempty body. The lane's empty
unknown-song `SCUserNoteLoaded` reply is accepted standalone; inside a lengthless DD04 batch it
remains ambiguous and is reported as `DecodeFailed` while the next packet is recovered. Session
tests exercise guarded requests, ordered event forwarding, idle-on-loss and explicit-login spawn.
These are protocol/state changes with no visual deliverable; screenshot/FPS validation belongs to
the root's integrated client run. Final validation: `cargo test -p aa-net --features bevy -- --quiet`
passed 685 unit tests, 2 scripted-login tests and all 3 recovery integration tests (690 total,
zero failures; 9 live tests ignored). The unit suite includes the three new session recovery tests.
The recovery integration test exercises 42 L1 opcodes, the proxy descriptor, every body truncation,
DD04 sentinel alignment and scoped-cache lifecycle. No live account or server was modified here.

Integration needs: consume the new character-brief and proxy descriptor events where useful;
unit impulse/idle/visibility rendering and actual cinema playback/skip remain client work. The
current lane still has the three native layout mismatches above (tower-list i32 count, criminal
u32 object ID, bubble NPC u32); the stopped agent's server fix was PR 41 on `NickMesser/AAEmu`.
Do not paper over these with ambiguous client decoding. Root's live salon apply receipt `hair: 0`
is also preserved exactly: the missing-original-hair server bug is separate from the correct salon
wire codec and requires a server fix.

## Native outbound player music (2026-10-23)

`aa_net::MusicCommand` and `Command::Music { character_id, command }` now expose
the four original requests. `game::ClientMsg::Music` is the low-level encoder;
call `MusicCommand::validate()` first when bypassing the session. The session
requires InWorld, a nonzero own actor, the same selected persistent character ID,
and its actual captured character header for **all four** variants. It validates
before writing. `Event::MusicRequested { character_id, command }` means only a
successful local socket write; `MusicRequestRejected` reports validation, stale
identity, missing-connection or failed-write rejection. Neither is a server ACK.
There is no new score/MIDI cache, pending success state, automatic skill request,
timer, synthetic music notice, or automatic playback completion in aa-net.

| Command | Opcode | Exact body, in order |
| --- | --- | --- |
| `CacheMidi { midi }` | CS121 | u32 byte count; u16 blob byte count; exact MIDI bytes, **no added NUL** |
| `UploadNotes { item_id, slot_type, slot, title, notes }` | CS122 | u32 notes UTF-8 byte count **excluding NUL**; u64 item ID; u8 bag SlotType; u8 physical slot; u16 title byte count + title **without NUL**; u16 (notes byte count + 1) + notes + **one NUL** |
| `RequestNotes { note_id, is_tooltip, show_buff_only }` | CS123 | u64 note ID; bool isTooltip; bool showBuffOnly |
| `End` | CS124 | empty |

Upload requires a nonzero source item in the main bag: `slot_type` is wire
`items::container::INVENTORY` **2**, not Lua's bag selector. `slot` is the
zero-based physical bag position, not a sorted visual index or a one-based UI
ordinal. Native lookup `3961e9a0` returns location bytes packed at offsets1/3 of
its u32: constructor `397acd20` copies that u32; serializer `397bc740` writes
only its bag/index bytes at packet+19/+1b. It zeros the other two location bytes.
The existing lane enum is `aaemu-game/src/models/container.rs::slot_type`.

Packet limits are title1..96 UTF-8 bytes, notes1..12000 UTF-8 bytes, and
MIDI1..32768 bytes. Title/notes embedded NULs are rejected rather than silently
cutting the text at the native C-string boundary. The byte encoder does not
parse MIDI, count editor characters, enforce proficiency, select equipment or
compile MML; aa-client owns those checks using aa-midi. Native Lua's title12 /
score1000 character limits and proficiency200/400/600/800/1000 limits remain
separate. All u8 physical slot values are representable; the app must resolve an
actual received item. Request note ID0 and both raw flags remain representable:
no invented “song exists” or flag-combination validation is performed by net.

Original flow evidence is `data/extract/ghidra/out/music{2,3,9,10,11,12,13}.c`
and extracted `x2ui/music/{music,user_music,user_music_view,percussion_view}.alb`
(read-only Lua decompilations in `data/extract/aa-lua/decompiled/x2ui/music/`).
Original DLL image base38ff0000, SHA256
`851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e`.

- `PreSaveNotes390799e0`: checks item/title/text, actual bag lookup and proficiency
  length, then `39083c60` stages the item/location/title/notes and validates them
  by compiling. It returns a local boolean and sends **no packet**. Lua opens
  the confirmation dialog only on that boolean.
- `SaveNotes39079640`: repeats those checks and staging, constructs an item
  caster through `39044360`, sets the composition-grade parameter (`39081f80`),
  and starts the existing sheet-item skill. It does not directly upload notes.
  In skill path `3939b010`, the own-player special-action97 branch calls
  `39081e60` at `3939b5fa`; that routine constructs/sends CS122 from staged data.
  Constructor `397acd20` uses actual `xlcommon!XlStringSize330199a0` (UTF-8 byte
  scan), and serializer `397bc740` adds the notes NUL but not a title NUL.
- Native score lookup `39082550` uses an existing local cache when possible.
  A miss sends CS123 `(isTooltip, false)`; a cache-hit playing request sends
  `(false, true)` and copies cached notes for the coming performance. Tooltip
  hits need no request. Cache ownership/lifetime and deduplication stay in client.
  Executed follow-up below corrects a key assumption: the DB key formats only
  low32, while pending equality and outbound requests preserve the full u64.
- Play/pause are existing skills, not additional music wire opcodes. In skill
  path `3939a810`, own-player special action93 calls `39083ba0` at `3939ae37`
  to compile staged notes and send CS121 (`397acd00` / serializer`397b1400`).
  Action94 calls `39081e30` at `3939ae12` to send empty CS124. Natural own voice
  disappearance separately sends CS124 in `390844a0`. The application must use
  its actual playback lifecycle; aa-net does not infer that a song has finished.
- `TestPlayNotes39079b90` compiles and starts a **local preview**, while
  `StopPlayNotes390797f0` calls `390840f0` to stop that local voice. Neither is
  a network play/pause command. The composition OnHide invokes StopPlayNotes.
  This describes entry/explicit stop only: natural preview completion **does**
  send CS124 through the shared manager, as executed in the follow-up below.
- Request constructor`397acd70` and serializer`397b1480` prove independent bool
  flag bytes. `End` uses empty serializer`3901c170`. Original SC1e0 handler
  `391e169b` widens its received signed32 note ID before the local cache callback;
  this does not make the SC1e0 field a u64.

Read-only lane references (under `<aaemu-src>/rust/crates/aaemu-game/src/`):
`systems/services/music.rs` implements all four request readers/handlers and the
note writer; `systems/services/mod.rs` registers CS121..124; `opcodes.rs` names
the offsets; `systems/skills/effects/special/items.rs` actions93/94/97 consume the
cached performance/upload and broadcast genuine notices; `systems/skills/packets_special.rs`
writes SC1df/1e1. The native upload callback is a received-fire callback, not a
pre-CS052 hook; the distinction and exact handoff are detailed below.
No server source or running lane was edited, built, started or stopped here.

Known original/server differences, deliberately preserved:

- The lane merges the two CS123 bool bytes into `value2:u16`. It handles only1
  (tooltip request: load notes) and256 (showBuffOnly: add ScoreMemorized6010).
  **Native playing cache miss `(false,false)=0` is currently ignored by the lane**,
  as is257. It truncates the u64 request ID to u32 for lookup and hardcodes the
  returned tooltip flag to true. Net does not rewrite these native request flags.
  This is the inspected pre-fix bevy checkout. PR44 fixes the reply/flag branch;
  the executed follow-up below distinguishes it from native pending/key behavior.
- The lane ignores CS122 declared byte count and bag/slot (`value2`) and queues
  by item ID. It performs actual sheet creation in the item-skill effect; upload
  alone is not a save receipt. CS121 only caches bytes; actual PlayUserMusic
  broadcasts them and applies the instrument buff. PauseUserMusic broadcasts
  SC1e1; Close the Score22217 also removes play buffs. CS124 removes play buffs
  only when ScoreMemorized is absent and is not a direct pause broadcast.
- SC1e0's lane length field counts UTF-16 code units; native outbound CS122
  counts UTF-8 bytes. The existing inbound note length remains uninterpreted.
  The lane's notes blob includes one NUL, stripped after consuming the whole blob.
  Its empty unknown-song reply is accepted as None standalone (or at a batch
  end); nonfinal empty SC1e0 in lengthless DD04 remains ambiguous and is reported
  as DecodeFailed with safe following-packet recovery, not a fabricated note.

SC1df now reads its MIDI blob length as unsigned u16 and checks repeated counts
against the32768-byte cap. Generic `Reader::blob` treats `0x8000` as negative,
which previously dropped the maximum-size performance and lost DD04 alignment.
Zero-byte received performances remain valid receipts (the lane can broadcast
an empty cache), while outbound compile results must be nonempty. Malformed
negative/oversize/mismatched counts fail explicitly before MIDI allocation.

Retained evidence: `crates/aa-net/tests/music_oracle.py` runs original x86
constructors/serializers and the actual xlcommon StringSize loop in Unicorn.
Only archive primitive/blob sinks, CopyString and memcpy are mocked; it does not
load a DLL into a game process. `tests/fixtures/music/native.tsv` retains11 small
goldens (UTF-8, flag combinations, native empty/C-string cases and empty End),
plus binary maximum-size MIDI/notes writer fixtures. `tests/fixtures/music/lane.rs`
contains verbatim actual server reader/writer method bodies with original file
SHA256s, in a minimal std-only scaffold. Cargo tests execute those methods;
this is not a live server/DB/handler test. Regenerate on the RE machine with
`python crates/aa-net/tests/music_oracle.py` (pefile/unicorn are RE-only packages).

Validation:

```
python crates/aa-net/tests/music_oracle.py
cargo test --locked -p aa-net --lib music -- --nocapture
cargo test --locked -p aa-net --features bevy -- --quiet
```

Original oracle regeneration passed. Focused music selection:9 passed (8 new
tests plus the existing music notice decoder). Full Bevy-feature suite:693 unit
tests,2 scripted-login tests and3 recovery integration tests passed (**698 total**,
zero failures;9 live-server tests ignored). The new tests execute the retained
native byte fixtures and actual lane readers/writers, check every truncated MIDI /
nonempty note body prefix, maximum-size payloads, both UTF-8 and UTF-16 length
semantics, exact bag/slot bytes, DD04 following-packet alignment, malformed-count
recovery, all four variants' identity/phase/header/actor gates, missing and failed
socket writes, exact session forwarding and absence of synthetic success or
automatic End. `git -c core.whitespace=cr-at-eol diff --check` passed (the existing
session source uses CRLF). This protocol task creates no window; FPS/screenshots
and live composition/audio verification remain root's client integration work.
No Cargo.lock, other-crate, server, README or game-asset edits, Ghidra project
copies, scratch executables, alternate targets, workspace runs or process kills
were used. Native fixtures and the reproducible oracle remain as evidence.

## Captured native composition cast and received-fire handoff (2026-10-23)

Public API:

```rust
let request = aa_net::MusicComposeRequest::new(
    aa_net::item_effect::CapturedBagItem { slot: physical_bag_slot, item: received_item },
    grade, // u8, selected from actual music_note_limits rows
)?;
let command = aa_net::Command::ComposeMusic { character_id, request };
```

`MusicComposeRequest` retains the complete source capture and grade with full
`Clone + Eq` semantics. Its accessors are `source()`, `grade()`, `skill_id()`
(always22215); `recheck_native(&NativeItemRecords)` checks the actual main-bag
physical slot. `write(&mut Writer, self_obj_id)` and
`game::ClientMsg::ComposeMusic { self_obj_id, request }` are the typed low-level
CS052 writer. Existing no-object casts and all four `MusicCommand` variants are
unchanged. There is no generic arbitrary SkillObject/packet escape hatch.

`Command::ComposeMusic` uses the ItemEffect request boundaries: InWorld,
nonzero matching selected persistent character ID, actual selected header,
nonzero BC3-representable own actor, matching native record owner, and equality
of every field of the captured Item at its enabled main-bag slot. Unknown,
invalidated, empty, moved, bank-only or changed records reject before any
write; the legacy Inventory is never a fallback. The complete item UID stays
u64, including values above2^53. `MusicComposeRequested { character_id,
request }` means **only a successful local CS052 socket write**.
`MusicComposeRejected { character_id, request, message }` covers guard,
missing-connection and write failures, preserving the exact request for caller
matching. No receipt, saved success, item mutation, score cache, automatic
upload, MIDI or End packet is produced.

Native SaveNotes39079640 checks nonempty item/title/text, own actor, actual
main-bag item lookup3961e9a0, proficiency length39082130, and successful local
stage/compile39083c60. It builds the complete item caster39044360, self unit
target, and SkillObject local147=1, local144=5, local140[0]=39081f80(notes),
then calls the ordinary skill planner thunk3ab63fb7. That planner is not
implemented by this transport. The application owns compilation, actual table
lookup, proficiency, confirmation and local staging. Grade selector39081f80
uses **XlStringLen**, walks the ordered music_note_limits table, and returns
the first row step whose note_length is at least that character length (or0
if none); this is separate from the CS122 UTF-8 byte counts. Restricting the
transport to Music Paper27902/use_skill22215 and supplied grade0..10 is a
deliberate boundary for the currently verified original rows. Native SaveNotes
does not itself impose this numeric grade range or compare all captured Item
fields; the full-record/owner/phase guards are defensive stale-input checks.
No speculative musical skill variants are admitted.

The exact CS052 body is u32(22215), item-caster type2, BC3 own actor,
u64 source Item.id, u32 source template27902, **u8 source Item.grade**,
u32 source Item.made_unit_id, self-target type0 + BC3 own actor, then object
flag5 + **u32 composition grade**. Item grade and composition grade are
independent. Original397cad20 invokes397c85b0/397bfa90/397bfd50;
type5 branches to397bff73 and archive primitive+4c for the four-byte grade.
Executing those original x86 bytes, including39044360, supplies22 retained
full-body goldens across grades0..10, a nonzero item grade/maker, maximum item
grade/maker, large and maximum u64 UID, and BC3 maximum actor. The actual
read-only lane SkillObject::read maps flag5 to Unk5(read_i32); its extracted
reader and exact primitive methods run against these goldens in Rust tests.
This cross-check proves framing, not server save semantics.

**Timing: CS052 first, actual received SCSkillFired, then CS122.** The retained
registration391eb9d0 passes handler391d58a0 to391bf000; packet factory391ab7c0
sets opcode0xa2. That handler
reads received packet members and calls3939b010 at391d5931. Its own-player
special-action97 branch at3939b5e1..3939b5fa calls39081e60 to upload the staged
score. It is not a pre-send planner hook. The later own callback is the call
at3939b77d (return address3939b782) to3939ce90, the existing cooldown producer;
it does **not** send CS052. A nonzero received result code exits3939b010 before
the action loop; upload requires the successful received-fire path. By
comparison, received-start handler391d57f0 calls
3939a810 at391d586e; that path runs action93 compiled CS121 and action94 End.
Function names alone do not establish this ordering: received packet member
loads, handler registration, exact calls and own-actor/action conditions are
retained in tests/fixtures/music/native-compose-source.txt.

Consequently this API sends only the type5 CS052 cast. Do not bundle CS122
before CS052 or upload in response to MusicComposeRequested/SkillStarted.
The application retains its staged score and sends existing
Command::Music(UploadNotes) at the matching actual own item-caster SkillFired
boundary with result.code0. Native3939b010 can also synthesize missing local *skill presentation*
through3939a810 while handling a received fire; this is still after the server
receipt, never a reason to pre-send a score. Matching casts, pending UI state,
and reacting to actual save/inventory receipts remain application-owned. A
server that consumes the uploaded notes synchronously before this callback
needs a server timing fix; changing the native client sequence would hide the
mismatch. No server/lane source, process or runtime was changed in this task.

Retained reproduction: `python crates/aa-net/tests/music_compose_oracle.py`
(reads the original DLL only; pefile/unicorn/capstone required for RE, not
normal builds). Fixtures include native-compose.tsv, native-compose-source.txt
and lane-skill-object.rs under crates/aa-net/tests/fixtures/music/. The oracle
mocks archive primitives and BC3 only; it does not execute the protected skill
planner, proficiency/table catalog, compiler, server or whole live callbacks.
Loopback tests assert exact frames and sequence counters, received-start vs
received-fire forwarding with no automatic output, explicit later CS122,
complete stale-item/physical-slot/main-bag guards, all non-world phases,
identity/header/owner/actor guards, missing/closed writes, full rejection
request equality and absence of synthetic success. No live save transaction or
visual deliverable is claimed. Validation: `cargo test -p aa-net` passed 682 unit
and 5 integration tests (nine live-lane tests ignored), including all seven new
compose tests; the executed-x86 oracle passed 22 cases.
`cargo check -p aa-net --features bevy` and owned-path whitespace validation
also passed. No clippy run or additional build target directory was needed.

## Executed stored-score, preview End and Started dispatch corrections (2026-10-23)

Research only: no net API, application caller, server or audio-device changes. Run
`python -B tools/user_music_lifecycle_oracle.py` and
`python -B tools/user_music_catalog_oracle.py`. Retained results and original
disassembly are in `tools/evidence/user_music_lifecycle/`. The lifecycle oracle
passes **142 original-instruction cases**; the catalog oracle executes four real
music rows through each of three original loaders and the actual attachment loop.
The x2game input is SHA256
`851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e`.
SDK, LevelDB, SQLite, allocation/index construction, presentation endpoints and
the compiler body are explicit fixtures. These results assert client control
flow, memory mutation and packet-sink arguments, not successful live wire/audio,
protected initialization, a complete timeline manager, or server settlement.

**Correction: the disk cache key is low32, not full64.** Actual
`xlcommon!XlFormatString3301b6a0` and its native formatter execute, with only CRT
memcpy/memset substituted. Both `39082550` and `39082260` use
`user_music_%02x_%016lx`; Windows `long` consumes the low32 argument and ignores
the separately pushed high32. For world255 and note`fedcba9876543210`, the actual
key is `user_music_ff_0000000076543210`. This is observed at the DB endpoint,
not generated by a substitute formatter. An earlier exploratory fixture that
formatted both words as a u64 was wrong and is superseded by these goldens.

Pending equality in `39082550`/`39082200` still compares **both** words, and
actual constructor`397acd70` retains both for CS123. Consequently high64 aliases
share DB contents but remain distinct pending requests. SC1e0 callback`391e1640`
uses CDQ at`391e1691` to sign-extend raw32. A zero-extended request`0000000080000000`
and reply`ffffffff80000000` therefore share the same DB key, but receipt does not
remove that original pending request. A later hit succeeds and emits buff-only
with the original full request ID; it does not repair pending. The pending list
also has no world byte in its comparison. **Claims that IDs>=2^31 cannot hit the
native cache because of sign extension are incorrect.** Earlier client notes or
PR discussions describing that as a general cache limit must be read with this
correction; the pending-cleanup mismatch and low32 aliasing are the real limits.

**Receipt does not retry use or create a buff/skill.** Cache misses send
CS123`(isTooltip,false)` once per full pending ID, including deduplication across
tooltip/use overlap. Complete executed `391e1640 ->39082260` receipts format the
key, zero scratch, copy title/notes, save, unlink the exact pending ID and copy
notes into manager+28. The `isTooltip` and declared noteLen arguments are unused.
Failed DB saves still unlink/copy, with no follow-up CS123, CS052, MIDI, End or
Lua callback. Actual CopyString byte limits are96/12000, stopping at NUL; a
boundary can split a UTF-8 sequence. The persisted value is97 fixed title bytes
plus notes bytes excluding the notes NUL. Later tooltip hits are quiet; later
use hits send`(false,true)` and stage the cached notes. Full-u64 pending and
low32 DB alias cases are retained separately in `native-cases.json`.

The inspected old C# and pre-fix bevy handlers only handle packed1/256, ignoring
the native miss0. [PR44](https://github.com/NickMesser/AAEmu/pull/44) fixes independent
flags and returns notes for miss0, but its handler still adds ScoreMemorized6010
only for showBuffOnly. Thus that fixed server's miss response alone does not
grant the score skills; a later native use hit or a separate authoritative buff
receipt is still needed. The actual compact `buff_skills` rows476/477/478 bind
6010 to22213/22214/22217. This establishes the remaining first-use behavior of
that server implementation. It does **not** establish an official-server rule
that justifies adding an automatic client retry or an invented local buff.
No running server was contacted, changed or restarted for this investigation.

**Natural preview completion does send End.** Preview`39079b90` and received
music`390824e0` both load singleton`39ae4674` and call the same `390846d0`.
Constructor`39084890` stores this singleton at`3908496b`; its caller stores the
same returned pointer into game+44 at`390f758d`. MainGame entry`390f6a20` begins
with a protected jump; vtable slot`399a646c` identifies it independently of the
sparse sweep index. Its plaintext continuation calls game+44's `390844a0` at
`390f6c96`. First, framework+20 must return true at`390f6bf1`, or the frame
returns before music update. Then engine virtual+a8 mode must differ from2.
Both gates execute in the oracle. The protected prologue is not emulated. Earlier
sweep-based labels390f5800 and an exploratory390f5910 label are incorrect;
390f5910 is the unrelated CustomizeEntityToNpc function.

Actual preview entry, manager admission, native tree helpers and update execute
together. When an admitted own voice is absent at SDK lookup, update sends one
CS124 then stops/removes that actor's entry; the next frame sends none. There is
no preview marker or preview exception. Both current-player and fallback identity
comparisons remain native. Actual StopPlayNotes`390797f0 ->390840f0` stops/removes
directly, so later update sends none. Replacement removes the old entry before
trying the new create; failed creation or missing actor leaves no entry and no
End. Successful replacement sends no End for the old voice. Mere actor registry
loss while a sound still exists leaves its tracked entry alive. `native-cases.json`
includes received/preview entry cases, own/other removal, replacement failures
and both framework admission outcomes with caller modes0/1/2/3. A blanket application rule that preview
completion never generates wire output is not native fidelity. The application
owns that distinction; aa-audio must continue to emit no network commands.

**Catalog dispatch comes from skill_effects, without an application-method filter.**
Read-only Ghidra exports `catalog-decompiled.c` and
`catalog-binding-decompiled.c` accompany the executed `native-catalog.json`:

- `39730ee0` loads `skill_effects`. SELECT column2 application_method_id becomes
  row+4, column8 effect_id resolves the generic effect pointer at row+1c, and
  column15 skill_id becomes row+30. Rows have stride54.
- `39730620` loads `effects(id,actual_type,actual_id)`. Its literal
  SpecialEffect branch`39730b74..99` writes category24 and resolves actual_id
  through`396e8d40`. `3972c390` loads special_effects, whose20-byte payload is
  special_effect_type_id followed by value1..4.
- `39731870`'s actual loop`39732e50..39732f0c` walks every loaded skill-effect
  row, resolves row+30 through the skill catalog and appends that row pointer
  into SkillDesc+1e8..1ec. No application_method filter is present.

The source-backed SQL join is therefore `skill_effects.skill_id = receivedSkillId`,
`effects.id = skill_effects.effect_id AND effects.actual_type='SpecialEffect'`,
then `special_effects.id = effects.actual_id`. It does not select a skills.effect_id
or skill_fx_group. Current rows are24560/22213/action93/method3,
24561/22214/action94/method3,24817/22217/action94/method3, and
24814/22215/action97/method1. Native Started's loop itself only checks category24
and action93/94. Executed method0/1/2/3/4/5 variants reach the same callbacks.
The C# and Rust application-method enums call3 **SourceOnce**, a targeting rule,
not OnStart. It must not determine received Started versus Fired timing.

Complete `391d57f0 ->3939a810` execution verifies result.code0, valid skill
descriptor, ordinary nonzero timeline and successful local timeline construction
before the action loop. Raw caster0/3 resolves object+8; type2 resolves object+14;
types1/4 use sentinel0. The resolved pointer must equal actual game+10c for93/94.
An item/mount raw caster resolving to that own actor can pass; a legacy projected
own object ID from type1/4 cannot. Failed results, foreign/missing actors, invalid
effect category and failed descriptor/timeline fixtures send none. Timeline
construction is a fixture here; this does not prove duplicate/admission semantics
of a complete timeline manager. Action97 sends no upload on Started: preserve
the existing successful received-Fired CS122 handoff.

Normal action93 calls original `39083ba0` synchronously at`3939ae37`. It reads
the **current** manager+28 staged notes, checks XlStringLen>0, calls`39083960`
with MIDI capacity32768 and parameter`[39ac37a0]=33`, constructs397acd00/CS121
and sends. Action94 calls39081e30/CS124 at`3939ae12`; this also covers Close the
Score22217, not just Pause22214. The parameter33, preview0, bypass0 and fallback
instrument constants80/81 are retained from the pre-existing read-only
`<re-notes>/housing-audit/native12-data-readonly.bin` (.data base39ac3000,
SHA256`7082573df48d7d5229e1f909845733f0cf9ded6ea1a4e3c41a5e276fccde8c9e`).
Every researched x2game instruction range also matches that capture's text;
no live process access was required.

Instrument fallback follow-up is executed in ten cases in `native-instruments.json`.
`39695dc0` resolves the **general ItemDesc** by equipped template, not an
ItemWeapon/holdable descriptor. Original items SQL`39a00d90` column9 is
`items.category_id`; `396cb5b3..b8` writes local-128, and copy constructor
`396ba730` copies rowbase-140+18 into ItemDesc+18 at`396ba772..75`. The native
wrapper compares this field to categories80/81. It does not read holdable_id or
holdables.slot_type_id for fallback. Read-only decompilation is retained in
`instrument-decompiled.c`; the column/store/copy/general-item lookup and actual
instrument-map lookup all execute before the wrapper's compiler sink.

Actual instrument_sounds lookup uses ItemDesc.id, loaded from SQL`39a08240`
`SELECT item_id,midi FROM instrument_sounds`. A nonzero mapped MIDI overrides
the fallback; missing mapping or exactly0 falls back. The static native table
`399a1e94..399a1ea0` is program106/bank0 for category80, program73/bank0 for81;
other categories report error1 and return0. It is a nonzero check, not a native
signed-positive/range-validation check. Native compiler-body admissibility for
arbitrary invalid program integers is not asserted by this wrapper oracle.

Actual DB examples34598/category80/holdable21 and34599/category81/holdable23
have no instrument_sounds rows and execute those106/73 fallbacks. Category96
examples6183/holdable21 and14047/holdable23 fail with error1. Item6177 has MIDI24,
which takes precedence over its category80 fallback. Holdable21 maps to111
category80 items **and18 category96 items**; holdable23 maps to90 category81
and12 category96 items. Therefore the old21/23 fallback shortcut is not the
native catalog rule, even though it agrees for ordinary instrument categories.

Inside39083960, actual `xlcommon!XlMultiToWide3301a100` receives3001 **u32** slots.
It strips one leading UTF-8 BOM, decodes at most3000 code points, appends u32 NUL
and returns storedCount+1. Supplementary characters occupy one slot. The wrapper
ignores its return and compiles the converted prefix; cached notes can exceed
that prefix's byte/character length. It does not perform UTF-8 validity checks.
Nine direct conversion cases and five normal-entry/wrapper cases retain empty,
BOM, embedded-NUL, Unicode and truncation boundaries. Compiler-body fixtures
only establish arguments/order; original MML synthesis fidelity remains the
separate aa-midi oracle's responsibility. No Cargo build, clippy, window or
visual/FPS claim applies to this research-only change.


## Placed piano/drum doodad bonds (2026-10-23)

This is the missing placed-instrument seating transport and received history. It
does not add mounts, piano-specific requests, guessed cameras, animation attachment
transforms, playback, or complete placement/music parity. Existing
`UseDoodadSkill` remains the start request: retained read-only catalog evidence
`data/extract/aa-client/placed-music/catalog.json` joins piano 20847 / buff 5068 and
drum 26592 / buff 8079 to attachment point 2, space 1, bond kind 2. That catalog also
contains other attachment rows; these values are not parser defaults.

### Exact wire/model and original code

`DoodadBond { owner_obj_id:u32, attach_point:u8, kind:u8, space:i32, spot:i32 }`
preserves the received doodad object identity and every field, without translating
space/spot into unsigned counts or defaulting a spot. Wire order is **point/u8,
owner/BC3, kind/u8, space/i32 LE, spot/i32 LE**, thirteen bytes. Character
`Unit::doodad_bond:Option<DoodadBond>` now retains its previously skipped second
attachment. `None` is the one-byte0xff snapshot sentinel. `CharacterState`,
mount/slave and skill APIs remain unchanged; `Unit::attached_to` remains separate.

| Packet | Exact body | Bytes |
|---|---|---|
| SCBondDoodad0x139 | actor/BC3 + full bond |16|
| Native empty SCBondDoodad0x139 | actor/BC3 +0xff |4|
| SCUnbondDoodad0x13a | actor/BC3 + persistent character/u32 + doodad/BC3 |10|
| CSUnbondDoodad0x0cd | actor/BC3 + doodad/BC3 |6|

Native DLL SHA256 is
`851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e`.
Factories391b0e10/391b0ee0 identify SC139/13a and packet vtables 399b39e4/399b39f0;
archives397b5ab0/397b5b10 give the actor and unbond character/doodad fields.
Bond archive3983bf10 reads/writes its point sentinel, BC3 owner through397b25d0,
kind, then independent four-byte space/spot. Runtime sentinel value is pinned
by the existing read-only native-data SHA in the oracle.

Received handlers391db770->393b2b90 and391db7f0->393b2bd0 retain actual packet
member dispatch. Native393566c0 checks existing bond/mount state before applying a
bond; net history rejects overwriting an occupied actor except for the next own
receipt after a matching successful local unbond send (see below), and never
materializes an unknown actor. Root still owns the mount gate and presentation.
Native3935a520 sends opcode0xcd, initializes actor/doodad through397ac640,
serializes BC3/BC3 through397bb470, then calls local detach393566f0. Its local
presentation change is not an SC13a server acknowledgement.

Actual lane sources were read only: `rust/crates/aaemu-game/src/models/doodad.rs`,
`systems/npcs/doodad/packets.rs`, `world/unit_state.rs`, and
`aaemu-commons/src/packet_stream.rs` beneath `<aaemu-src>/`. Extracted
writer/reader bodies and source hashes are committed in
`crates/aa-net/tests/fixtures/doodad-bond/lane.rs`; full character writer evidence
is in `lane-snapshot-source.txt`. The lane always writes a full SC139 bond; the
empty native SC139 variant is handled separately as `BondAbsent`, which does not
assert detach or known-free state.

### Public integration contract

New exports are `DoodadBond`, `DoodadBondEvent`, `DoodadBonds`, and
`UnbondDoodadRequest { character_id:u32, actor_obj_id:u32, bond:DoodadBond }`.
`Event::DoodadBond` wraps `Bonded {unit_obj_id,bond}`,
`BondAbsent {unit_obj_id}`,
`Unbonded {character_obj_id,character_id,doodad_obj_id}`,
`UnbondRequested(request)`, and `UnbondRejected {request,message}`.
`Command::UnbondDoodad(request)` captures the selected character, own actor and
**entire received bond** when queuing; only the two BC3 object IDs are serialized.
InWorld phase, matching selected header/actor, nonzero BC3 bounds, an attached
game socket, and exact current received bond history are mandatory. A socket
write emits only UnbondRequested; it does not delete the bond or invent a
server receipt. A write failure follows existing connection-loss handling.

`WorldState::doodad_bonds` and the Bevy `DoodadBonds` resource fold the same
receipts as the session's private request validator. `by_unit` records known
bonds; `own()` returns the current own bond when identity matches.
**`own()==None` proves free only when `own_known()==true`.** An initial own
PlayerSpawn or a complete matched own snapshot establishes that distinction.
SC139 for the known actor can reestablish a present bond after history loss. A
matched SC13a removes only the current actor/character/doodad bond and preserves
known history. Foreign IDs, stale doodads, absent-sentinel receipts and local
request/rejection events cannot turn unknown history into known-free.

Actor/doodad removal and object reuse invalidate affected bonds; character
selection, kick, leave, disconnect and lobby/session reset clear them.
`GameDecodeFailed` and **nonempty** compressed Unhandled invalidate bond
history, including known-free history, while retaining the actor identity needed
to correlate a later real receipt. A boundary-known ordinary unsupported packet
or a zero-byte compressed Unhandled does not alone prove data loss.

`Event::OwnUnitSnapshot(Box<Unit>)` preserves repeated own SCUnitState receipts.
The session admits it only in InWorld for `obj_id==self_obj`,
`UnitKind::Character {char_id:selected}` and a matching selected CharacterState
header. It emits neither UnitSpawned nor PlayerSpawn. Only `DoodadBonds` and
`Buffs.native` consume its snapshot; player stats, transform, raw HUD buff list,
3D spawn and other UI state stay untouched. `NativeBuffs` keeps the own
actor/character tuple from PlayerSpawn, requires that tuple on refresh, retains
it across decode gaps and clears it on teardown/removal. Bevy marks the buff
resource changed for the own snapshot so dependent native-mode logic sees it.

Character parsing now requires a complete tail. The former guessed9/12-ability
fallback could accept a truncated packet as an empty buff snapshot. Standalone
10/11-ability compatibility parsing must end exactly at the frame boundary;
DD04 character entries use the actual lane's ten-ability schema, established by
CharacterAbilities::default 1..11 exclusive and its writer loop. NPC/mate
best-effort tail behavior is unchanged. Supporting another compressed character
schema requires source-backed framing rather than guessing over the next opcode.

### Fresh own SC139 after a local unbond send

Native `3935a520` sends CS0xcd and immediately clears its local bond through
`393566f0`. Its next receive gate `393566c0` is therefore open before SC13a,
for an identical **or different** fresh bond. The original fd66072 reducer kept
received history correctly but used that retained occupancy to refuse the
different bond. A single private `own_rebond_eligible` flag now separates this
receive eligibility from received history; no public API, wire or command changes.

Only `UnbondRequested` whose full capture still passes `matches_request` opens
eligibility. That event remains socket-write status: `by_unit`, `own()` and
`own_known()` stay unchanged, and no `Unbonded` is synthesized. The next valid
own SC139 may replace the retained full bond and consumes eligibility even when
its fields are identical. The occupied gate then rejects further duplicates or
conflicts until another matching successful send or actual freeing receipt.
Remote actors cannot use the own eligibility. Existing outbound identity,
full-capture, phase, BC3 and socket guards are unchanged; after B is replaced by
different C, capture B rejects and exact capture C can send the same six bytes.

Matched own snapshots (present or absent), matched own SC13a, affected actor or
owner invalidation/replacement, decode loss and session teardown reset the flag.
An invalid completion, rejection, absent SC139 sentinel, zero owner, unknown
actor, unrelated event or unmatched SC13a neither opens nor consumes it.
`apply()` reports a private eligibility change, including an identical accepted
SC139, so Bevy change detection stays consistent with the complete reducer state.
That change flag is not a received free-state assertion or physical attachment.
The session's private validator, WorldState and Bevy all fold the same events.
Root still owns presentation serials/suppression, request sequencing, input and
the separate native mount gate; polling retained history cannot establish a
fresh receipt or replace those controls.

SC13a keeps the stronger existing actor/character/current-owner checks despite
the original handler's broader detach behavior. Wrong identities and an old
different-owner ACK remain rejected. **An old same-owner ACK is inherently
indistinguishable** after a fresh bond, including changed spot/kind/space: the
packet has no generation or full bond fields. A matching ACK still clears the
current bond; local counters cannot claim to identify its server-side epoch.

`python -B crates/aa-net/tests/doodad_rebond_oracle.py` independently executes
eight original-instruction sequences using the existing pinned read-only game
DLL and runtime constants. `tests/fixtures/doodad-bond/rebond-native.json` retains
observations, `rebond-native.tsv` supplies three Rust-consumed received-bond
goldens (identical, different owner, same-owner changed signed fields), and
`rebond-source.txt` records disassembly and per-range SHA256. The oracle is
self-contained within tracked code and does not import ignored aa-client probes.
Original gate, store, send-constructor and clear instructions execute;
actor/owner lookup, render/attach, packet/action sinks and callback registration
or cancellation are fixtures. Missing-owner cases prove accepted storage and
readiness wait, not live seating or interpretation of signed space/spot.
The native mount refusal is evidence for root's separate gate, not a new aa-net
mount/presentation implementation. Root's earlier 50 executed lifecycle cases
and 18 schema-only consumer vectors were inspected separately, not rerun here.

Regressions cover eligibility consumption for equal/different bonds and every
captured field, invalid/rejected completions, absent/invalid/unrelated receipts,
remote isolation, snapshots, decode gaps, removal/reuse and teardown. They pin
different-owner ACK rejection and same-owner ambiguity. Actual decoded DD01/DD04
and scripted loopback writes verify unchanged CS0xcd frames, no synthetic
receipts, stale-capture refusal, fresh C captures, and snapshot supersession.
Session emissions replay through WorldState and the Bevy fold with complete
state equality and resource change detection checked after each event.

Follow-up validation: all eight native oracle cases and all 19 focused bond
tests passed. `cargo test --locked -p aa-net --features bevy` passed 736 tests
(731 library, two scripted-login, three recovered-packet); the same nine live
lane tests remained ignored. The single final
`cargo clippy --locked -p aa-net --features bevy --all-targets` completed
successfully with existing warnings and none in the added code/tests. Both
Cargo runs used the normal shared target; production source was frozen and the
unchanged API announced before the full run. Logs are under
`data/extract/aa-net/seat-eligibility-followup/`. No aa-client, server, manifest,
lockfile, live process or original asset changes are part of this follow-up.

### Retained verification and limits

`python -B crates/aa-net/tests/doodad_bond_oracle.py` executes original packet and
bond instructions in isolated Unicorn memory, substitutes only primitive archive
operations, and retains ten goldens from twenty original read/write executions.
These include point0/2/254, BC3 maxima, distinct space/spot, i32 extremes, and the
native empty-bond sentinel. No live game process, audio device, socket, lane
server, or database write is involved. `native-source.txt`, `native.tsv`, the
actual lane methods and snapshot source are retained alongside the oracle.
The prior research's 62 executed mode/bond cases remain in the ignored
`data/extract/aa-client/placed-music/` proof directory.

Rust coverage compares native goldens with actual lane writers/reader, every
packet-body prefix, complete character prefixes and buff/bond snapshot retention,
DD01/DD04 alignment and a following packet, exact loopback CS0xcd frames without
synthetic replies, all captured-field/identity/phase/socket rejections,
stale/foreign unbonds, decode-loss readiness, own-snapshot recovery,
removal/object reuse, session kick/disconnect/reselect/leave and the Bevy fold.
These are protocol/reducer/scripted-loopback checks, not a live placement,
instrument interaction, sound or visual parity claim. Root owns aa-client
seat/render/unbond controls and any local detach presentation.

Validation: `cargo test -p aa-net --features bevy` passed 717 tests
(712 unit, 2 scripted login, 3 recovered-packet); 9 live-lane tests stayed ignored.
The single `cargo clippy -p aa-net --features bevy --all-targets` run completed
successfully with existing crate/example/test warnings. Its two new style
suggestions (character-tail let-chain and the golden hex decoder) were applied;
Clippy was not run a second time. Focused bond/Bevy tests were rerun afterward.

## Strict native MusicSheet detail8 task projection (2026-10-23)

The newly composed-sheet gap was the strict `qualified_detail` allowlist: Add5,
Take6, Owner15 and Update9 already decode and retain tag8, but projection refused
it and `NativeItemRecords::apply_row` retired the physical record. A later full
snapshot admitted the same item through `Item::read`, explaining recovery after
relogin. This change adds **only tag8** to the existing tags0/1/3 guard. No public
types, receipt framing, lookup, row ordering, pending or observer paths change.

Original 1.2 evidence is retained in
`crates/aa-net/tests/fixtures/music-sheet-detail/source.txt`:

* Tagged archive3961edf0 reads the tag at3961edfe..ee06. Tag8's actual switch
  entry3961eea8 selects3961ee42 (`eax=9`), then3961ee52 subtracts one and the
  archive copies exactly8 bytes beginning at union+1. Original item use at
  394eb108 checks tag8, loads both dwords at Item+19/+1d and forwards them to
  39082550. The projection keeps these as eight opaque bytes/`Other(8)`; it
  introduces no ID validity, range or float conversion policy.
* Task-item constructor/reset397ccbf0 zeros all128 union bytes at397ccc01..21;
  normal Item constructor3901cb10 does the same at3901cb24..45. Both execute
  against a nonzero-filled synthetic record in the oracle. Captured original
  runtime sentinel values come from a SHA-pinned read-only RE data image.
* Add/Take slot archive397d0f90 and Owner archive397d0fe0 call397d0a00. That
  serializer passes fixed128 to blob archive input+d8/output+dc at397d0a8e..ab.
  Update archive397cfd40 likewise passes128 at397cfda0..c3. This is the padded
  task union, distinct from `Item::read`'s unpadded tag+8-byte snapshot prefix.
* Dispatcher397d1030 table entries5 AND6 select397d10d1->397cda20; entry15
  selects397d11bf->397cdef0. Their existing physical lookup/virtual-container
  guards remain in force. Assignment397ad440 copies the complete128 union at
  397ad472..87 plus all serialized Item metadata, then actual3901c0a0 copies
  the post-assignment output (including another full128 union copy).
* Entry9 selects397d112e->397cdc10. After physical lookup/observers it copies
  task+10's complete128 into Item+18 at397cdc67..74, then calls actual output
  copy3901c0a0 at397cdc80. Packet itemID is ignored; all other Item fields stay
  unchanged. All these bodies execute in the oracle, including original
  physical/container helpers and full output copy; only primitive archive,
  memcpy/memset and observer sinks are substituted.

Qualification still requires an exactly128-byte received block, successful
reparse, equal parsed detail and an **exactly matching eight-byte payload** for
tag8. Missing/synthetic raw data, non128 lengths, wrong tag, absent/short/long or
different parsed payload, and unknown/inconsistent PRE-row context still refuse.
Other opaque tags2/4/5/6/7/9 and unknown tags remain unsupported by this strict
projection. Native Add template0 refusal, Add/Take virtual4/5 bypass, Owner's
container restrictions and Update's physical lookup/output timing are unchanged.
The complete raw128, including nonzero reserved tail, remains on the original
ItemTask receipt; normalized Item retains only its exact parsed prefix, as before.
There is no new raw-padding field or reconstruction from legacy Inventory.

Reproduce with `python -B crates/aa-net/tests/music_sheet_detail_oracle.py`.
The pinned original DLL SHA is
`851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e`.
The oracle passes2 constructors,4 tag8 read/write values (0,7,high64,UINT64_MAX)
and7 padded task read/write/dispatch cases. `native.tsv` retains exact original-
serialized task bytes and independently serialized post-write Item goldens; `native.json`
records full128 blocks and observer/copy ordering. Rust tests need neither the
RE binaries nor Python packages.

Regressions compare all Add/Take/Owner/Update goldens and metadata/output order,
all1116 task wire truncation prefixes, following-Money alignment, every raw-union
length0..127 plus129/256, tag/payload disagreements and still-unsupported tags.
The actual shared `NativeItemRecords` test starts from a fresh known-empty
snapshot, applies Add sheet28051/note7, then immediately resolves the exact same
Item by physical slot and persistent ID without a relogin. Subsequent Count and
Update see the previous row's mutation (including count0), and malformed rows
retire only the declared slot without recovering certainty from a later Add.
Root owns the aa-client integration regression and actual OwnServer rerun; this
CPU-only change performs no live connection, server/process or client edits.

Validation: `cargo test -p aa-net --features bevy` passed722 tests (717 unit,
2 scripted login,3 recovered-packet), preserving all prior717 passing cases;
the same9 live-lane tests stayed ignored. The single
`cargo clippy -p aa-net --features bevy --all-targets` run succeeded with the
existing warnings and one new test-only hex-decoder `as_chunks` suggestion.
That suggestion was applied without a second Clippy run.
All5 focused `--lib music_sheet` tests passed again after that test-helper change.
Root separately reported its actual `aa-client` dispatcher regression passed:
`cargo test --locked -p aa-client --bin aa-client newly_created_sheet_is_usable_before_relogin -- --include-ignored`
(1passed,0failed; `data/local/resume/fresh-sheet-hud-regression-tests.log`).
It covers Add5 raw128/tag8 -> strict captured-item equality -> cold use(false,false),
an actual SC1e0 fixture without automatic retry, then use(false,true). That
consumer test and the fresh-sheet live6010/mode/MIDI rerun remain root-owned.

## Received combat holdable and packed hit word (2026-10-23)

`skill::UnitDamage` now retains `holdable_id: u8` and `raw_hit: u16` exactly as
received in SCUnitDamaged0x0a7. `pierce_selector() -> u8` returns `raw_hit >> 12`.
The existing `hit_type` remains `raw_hit & 0x1f`; the generic critical/miss helpers
keep their existing classifications. These are primitive fields, with no DB
lookup, range rejection, enum conversion, sound policy, or equipment inference.
All256 holdable bytes and all65536 packed words remain representable.

Original packet archive397ca630 calls context archive397c8760. Its received
holdable byte goes to context+118. The packed word goes to +48 (low5 hit), +44
(bits5..7), +4c (bits8..11), and +64 (high4 pierce selector). The middle groups
remain in `raw_hit` without assigning unproven semantic names. This includes the
group consulted by the native plot damage timing path. The reader still consumes
exactly one holdable byte and one little-endian word at their original positions;
the subsequent flag/result handling is unchanged.

`tests/unit_damage_oracle.py` executes the pinned original DLL in place, hooking
only primitive archive calls. It retains63 hol/de/flag suffix write/read pairs in
`tests/fixtures/unit-damage/native-suffix.tsv`, including holdables0/3/255,
selectors0/1/2/15, independent middle groups, unknown hit31, and flags0/1/15.
Original write397c8a4e..397c8ad1 and read397c88ae..397c894c execute against
nonzero-filled synthetic contexts. `source.txt` retains disassembly and the
original SHA256851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e.
Reproduce with `python -B crates/aa-net/tests/unit_damage_oracle.py` (pefile,
Unicorn and Capstone required only for evidence regeneration).

The same oracle extracts actual lane SCUnitDamaged, CastAction, SkillCaster and
PacketStream writer methods verbatim into the executable `lane.rs` fixture,
with input file hashes. The current lane writes `holdable_id`, then
`0x0120 | hit_type`, then flag1/result1: its pierce selector is zero. No server
change is required for retention. Any future writer must preserve the same
one-byte/two-byte widths and keep the four packed groups independent. aa-net
does not add an outgoing UnitDamage writer.

Rust regressions compare a fixed full-body lane golden,200 actual lane writer
cases (all5 cast actions, all5 caster forms, all4 PISC widths), every isolated
truncation prefix and an untouched following sentinel. They exhaust the65536
words and256 bytes independently, verify legacy helper classifications, and
check200 adjacent distinct damage contexts plus following messages through
DD04 and Session. The existing fire/hit/death/end batch also checks retained
fields. Ordinary tests need neither the DLL, lane sources, DB nor Python.

Scope: original suffix evidence does not establish the full native optional
tail; Rust goldens embed hol/de in the unchanged lane flag1/result1 tail.
No live packet/animation ordering or impact callback is claimed. Root owns the
three aa-client struct literal updates and carrying each received context to
impact. Native sound source material comes from the packet holdable's
`sound_material_id`; sound `twohand` separately uses the current mainhand's
holdable `slot_type_id == 16`. A zero selector is received data, not a missing
value; selectors1/2 map to native sound values1/0.3 outside aa-net. Existing
generic hit helpers are broader than the native weapon-audio admission rules.

Validation: the original suffix oracle passed63 write/read cases. The focused
Bevy run passed all7 new tests. `cargo test --locked -p aa-net --features bevy`
passed729 tests (724 unit,2 scripted login,3 recovered-packet), with the same9
live-lane tests ignored. The single
`cargo clippy --locked -p aa-net --features bevy --all-targets` run succeeded;
its warnings are in existing code, with none in the added API, damage tests or
retained lane writer. Logs are under `data/extract/aa-net/unit-damage-audit/`.
No server, process, aa-client, manifest or lockfile changes are part of this work.

## Appearance conversion requests (2026-10-23)

`look_conversion::{ItemLookOperation, ItemLookRequest}` adds the approved separate
transport surface for the original appearance UI. `ItemEffectMode` and its
Grade/Socket/Gem variants are unchanged. Constructors are
`convert(base, image)`, `cleanup(skill_id, source, target)` and
`skinize(skill_id, source, target)`, using the existing
`item_effect::CapturedBagItem { slot, item }`. IDs/templates must be nonzero and
skill requests require an explicit nonzero catalog-resolved use skill. The
transport does not choose powder quantities, infer permission or combination
rules, replace ordinary planner admission, or modify inventory on submission.

Accessors are `operation()`, `skill_id()`, `source()`, `target()`, `captures()`
and `recheck_native(&NativeItemRecords)`. For conversion, target is the base and
source is the image; its skill ID is None. For skills, source is the powder and
target is the recipient. `recheck_native` compares both complete captured Items
at their exact physical BAG slots, including normalized detail and retained
detail prefix. Unknown, changed, disabled or out-of-bounds records refuse.
There is no legacy Inventory fallback. This full-record equality is a stronger
application stale-input boundary: original converter39047650 instead rereads
the selected physical slots' current IDs when invoked.

Send `Command::ItemLook { character_id, request }`. Session checks the same
active received character and socket conditions as ItemEffect: InWorld,
nonzero selected persistent character matching the captured character,
matching received selected-state identity, nonzero24-bit self object,
matching NativeItemRecords owner, and both exact current captures. Then
`ClientMsg::ItemLook { self_obj_id, request }` uses this wire contract:

| Operation | Level/opcode | Body, in order |
|---|---|---|
| Convert | 1 / CSConvertItemLook0x049 | baseID:u64, imageID:u64 |
| Cleanup/Skinize | 1 / CSStartSkill0x052 | skill:u32, caster kind2:u8, self:bc24, sourceID:u64, sourceTemplate:u32, sourceGrade:u8, sourceMadeUnit:u32, target kind3:u8, self:bc24, targetID:u64, targetTemplate:u32, targetGrade:u8, skillObjectFlag0:u8 |

All multibyte values are LE. Body lengths are16 and43 bytes. Skills reuse the
existing private `write_item_effect_start` with no support or AA object. Current
read-only catalog joins map cleanup23457/source29704 to special13681/type103,
and Skinize23741/source29880 to special14162/type108. Root still owns catalog,
ordinary planner, permission and UI mode gates. No source item, target or u64
identity is recovered from a later legacy slot or a Lua floating-point number.

`Event::ItemLookRequested { character_id, request }` reports a completed local
send only. `ItemLookRejected { character_id, request, message }` reports local
guard/write failure. These events do not grant success, change records, lock or
unlock slots, create skill/timeline events or automatically retry. Received
SC090 `ItemsChanged`, existing skill events and server errors remain the inputs
to the caller's state machine. No new success packet or receipt projection was
introduced.

Current lane and original C# agree: applying an image consumes configured
reagents as reason40, writes the image item's own template onto base equipment,
emits reason111/UpdateDetail9, then removes the image as reason40. Cleanup sets
image template0 through reason111/UpdateDetail9. Skinize ORs Skinized0x08 through
reason113/SetFlagsBits10. Both skill effects separately consume powder under
their reason, while the generic skill pipeline can attempt another source
consumption under reason40. Clients neither compensate for this nor assume a
fixed receipt count. Source and query provenance are committed with fixtures.

All existing native projection bounds remain: Update9 needs the actual128-byte
received block, supported tag0/1/3/8 and matching reparsed detail/prefix. Its
physical PRE-row lookup ignores the packet ID, replaces details, then copies
output. Flags10 ORs bits on that physical record. Reserved raw tail remains on
the receipt; it is not invented as normalized Item metadata. Equipment extent28
and BAG/BANK safe bounds130 plus known configured/enabled extent, unknown-versus-
missing distinctions, decode-gap history loss and forced-ID certainty retirement
are unchanged. SC093's BAG-only/full-ID guards are not transplanted onto Update9.

Evidence is retained in `crates/aa-net/tests/fixtures/look-conversion/`.
`native_oracle.py` asserts pinned x2game SHA256
`851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e` and executes
three CS049 constructor/serializer read+write cases, four selected-slot caller
cases and two cursor-input/serializer cases in isolated Unicorn. Native
390474a0 emits header0x49 and397b0950 serializes base/image IDs. For skills,
3942fb50 constructs full target and empty object and hands them to protected
39396e20; the oracle explicitly intercepts that ordinary planner and separately
executes397cad20/397c85b0/397bfa90/397bfd50 on captured inputs. This establishes
input/wire mapping, not execution of protected admission or live success.
Converter validation, archive/BC/memset and send endpoints and all runtime
fixtures are explicitly qualified in `native-goldens.json`; exact instruction
bytes, immutable C# and lane source hashes and88 read-only catalog rows remain
alongside it. Normal Rust tests use committed bytes without the PE, DB or Python.

Regressions cover literal frames/high-bit u64 IDs, all codec and receipt
truncation prefixes, received detail/flag projection, loopback writes, both full
captures' metadata, native-versus-legacy divergence, every inactive phase,
selected/header/owner/actor/socket guards, physical BAG versus BANK and slot129,
explicit recapture after received changes, targeted snapshot repair, Init,
decode gaps, malformed snapshots, actor loss, duplicate receipts and absence of
optimistic success or retries.

Validation: all9 isolated native cases and10 focused Bevy regressions passed.
`cargo test --locked -p aa-net --features bevy` passed746 tests (741 unit,
2 scripted login and3 recovered-packet), with9 live-lane tests ignored.
The single final `cargo clippy --locked -p aa-net --features bevy --all-targets`
completed successfully; its warnings concern existing code, with none in the
added API or tests. Logs are under the ignored
`data/extract/aa-net/look-conversion-20261023/` directory. No server, live client,
foreign process, aa-client, manifest, dependency or lockfile was modified.

### Original in-game shop menu transport (2026-10-24)

`aa_net::cash_shop::NativeInGameShopMenu` preserves one complete SCICSMenuList
receipt as `result:u8`, `main_tabs:[u8;6]`, `sub_tabs:[[u8;7];6]`. Opcode0x1cf
now emits exactly one `CashShopEvent::NativeMenuList(menu)` through the existing
ServerMsg::CashShop / Event::CashShop envelopes. The legacy
`MenuList { enabled, tabs }` literal and log name remain available for migration,
but decoding no longer emits that lossy variant. Exhaustive CashShopEvent matches
must handle the added variant; no damage/look/bond API or other cash-shop decoder
is changed.

The original archive397b8390 ->397b22f0 reads result:u8 followed by six groups of
main:u8 and seven sub:u8, exactly49 mandatory body bytes, even for status0.
Receiver391e10d0 uses **result!=0**, not the wire Reader::bool's exactly1 rule.
All raw IDs are retained, including0,255, repeated/nonidentity IDs and nonzero
sub IDs under main0. Lua tab indices are positions, not these request IDs.
Native memory holds six contiguous mains followed by42 subs, while the body
interleaves one main and its seven subs. The decoder does not install an active
menu cache, grant admission or synthesize a second legacy event.

`Command::RequestInGameShopMenu { character_id }` carries a captured persistent
identity used only for local validation. The session requires InWorld, matching
nonzero selected and captured identity, matching received selected_state header,
nonzero24-bit own object ID, and an existing game socket. Successful local
`send_game` writes emit `Event::InGameShopMenuRequested { character_id }`;
validation/write failures emit `InGameShopMenuRequestRejected { character_id,
message }`. A failed write also follows the existing connection-loss event path.
These outcomes describe transport only, not a received menu, remote acceptance,
storefront availability or a financial result. Client::send merely enqueues and
is not the Requested outcome.

Low-level `game::ClientMsg::RequestInGameShopMenu` is CSICSMenuListPacket0x11b,
level1, with an **empty body**. Native producer391876d0 selects vtable399b2f64;
its body archive3901c170 is just ret4. No character/object/account ID or
correlation token is serialized. No goods, cart, wallet, purchase, gift, charge,
browser or modelview request API is introduced.

Feature admission is independent of menu status. GetFeatureSet39496b20 sets
its feature base to game+14, then emits ingamecashshop from base+4 bit40;
IsInGameShopEnable39062360 and hotkey393f91f0 read the same game+18 bit40.
The actual native menu receiver/apply/reset does not write that bit. Existing
GameConfig.received retains the separate received fset bytes: no config or
fewer than five bytes means unknown; byte4 mask40 clear/set distinguishes the
received disabled/enabled values. A new short snapshot must not inherit an old
byte4. Menu result0/nonzero cannot replace that input or fallback true.
The protected original InitialConfig391d1d69..391d1f5a packet-to-runtime copy
remains unexecuted/unverified; this change adds neither hydration nor a new
feature authority. Native shared-bit reads and menu non-authority are proved
separately from that remaining producer boundary.

Root owns feature unknown/short/clear/set policy, single-flight/native waiting,
active cache versus raw receipt, and UI admission. Native status0 preserves
the old active tab cache and emits checkTime then waiting(false). Nonzero
replaces it, emits maintab then waiting(false), and may select a sub/page,
request uncached goods and return to waiting(true). Native CheckTime marks
refresh pending and clears internal waiting, emits checkTime without a waiting
event, and delays the menu request until subsequent selection. CheckWaitingServer
only re-emits the current bool; SyncGood is not a menu acknowledgement. The
transport deliberately performs none of these root-owned UI/cache transitions
or automatic requests. No response produces no fake receipt, retry or success;
no native timeout/recovery policy is claimed.

Replies have no owner/request token. On leaving, character change/re-entry or
connection replacement, root must retire its menu/wait/queued state and fence
old events. Clearing A's pending owner, sending B and assigning the next reply
to B is unsafe on the same connection: retain a retired outstanding tombstone
until drained, or use a proven connection/receive fence. Same-character re-entry
also needs an owner generation. Session rejects stale captured commands but
forwards raw untagged receipts without inventing correlation. Connection-scoped
GameConfig lifetime remains distinct from character-owned menu state.

Committed `tests/fixtures/ingame-shop-menu/native.tsv` contains exact original
archive outputs from retained x86 instructions under bounded Unicorn, with
provenance/endpoints in source.txt. Module SHA256 is
851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e.
Inputs are synthetic instruction fixtures, **not actual server menu snapshots
or storefront/admission availability**. No native process, service or device
was used. The full prior UI/menu audit evidence remains immutable under
data/extract/aa-ui/ingame-shop-audit-20261024 and
data/extract/aa-net/ingame-shop-menu-audit-20261024.

Regressions cover all256 raw status values, four original archive goldens,
all196 strict prefixes, suffix cursor position49, normal/DD04 following-packet
alignment (including header-looking IDs), all256 empty-request frame counters,
every inactive phase, selected/header/identity/object/socket guards, successful
and failed local writes, no-response/no-auto-retry behavior, one raw event,
truncated receipt recovery, feature snapshot non-mutation and stale captured
commands/untagged late receipts. DD04 bodies have no per-entry lengths; no
impossible arbitrary truncated-body-plus-next-packet boundary is inferred.

Validation: all10 focused menu regressions passed. The full
`cargo test --locked -p aa-net --features bevy` passed756 tests (751 unit,
2 scripted-login and3 recovered-packet), with9 live-lane tests ignored.
The single final `cargo clippy --locked -p aa-net --features bevy --all-targets`
exited0; warnings concern existing code, with none in the new menu API/decoder,
command handling or tests. Prior UI/menu evidence and all other frozen aa-net
sources were hash-verified unchanged, including manifests and Cargo.lock.
Logs and the exact owned freeze are recorded under
data/extract/aa-net/ingame-shop-menu-implementation-20261024.

### Original territory management T1 (2026-10-24)

`DominionCommand` adds `ChangeTaxRate { zone_group_id: u16, tax_rate: i32 }`
(CS012), `ChangeNationalTaxRate` with the same fields (native CS013), and
`RequestCharacterBrief { requested_id: u32 }` (CS02d). The tax bodies are u16 then
i32; the brief body is one persistent u32. `game::ClientMsg::Dominion` encodes
them. `Command::Dominion { character_id, command }` requires InWorld, nonzero
captured persistent ID equal to selected and the actual received character header,
nonzero zone/requested IDs, rate0..50 for Dominion and0..1000 for national tax.
The national native scale1000 means100 percent; original Lua conversion and its
confirmation remain caller-owned. CS013 is recovered from the original positive
Task40 callback, despite the old C# unknown0xfff placeholder; the lane does not
register it, so wire submission promises no service or settlement.

`DominionRequested { character_id, command }` is emitted only after a successful
socket write; `DominionRequestRejected { character_id, command, message }` covers
invalid/inactive/stale/missing-header and missing/failed transport cases. Neither
is an ACK, changes received state or starts an invented request timeout. Native
permission, selected zone/house context, dialog capture and same-character new
session generation are Root/caller responsibilities. No extra session cache,
automatic request, list/details refresh, collect API or UI callback is added.

Six added `DominionEvent` variants preserve all fixed fields:

| Opcode / variant | Body in wire order |
|---|---|
| 021 `NationalTaxRate` | `u16 id, i32 tax_rate, i32 previous_tax_rate, i64 changed_time` |
| 022 `NationalMonumentChanged` | `u16 id, i64 monument_db_id, i64 x, i64 y, f32 z` |
| 023 `TaxBalanced` | `u16 id, i32 peace_tax_money, i32 peace_tax_aa_point` |
| 024 `NonPvpStarted` | `u16 id, i64 started, u16 duration_minutes` |
| 025 `NonPvpStopped` | `u16 id` |
| 0eb `SiegeReinforced` | `u16 id` |

All six update only an existing received row and retain the complete receipt,
including repeats/missing-row deltas. National rate caches its low16 bits and
changed time; previous rate remains in the receipt. Monument replaces only its
ID/position. Balanced zeros current house money, hunt money and house AA, then
replaces both peace balances without changing paid/rate timestamps. Reinforce sets
defender.limit=min(defender.nr,100) without changing phase, team IDs or members.

`DominionData.non_pvp_end_time: i64` is a derived cache field, with no extra wire
bytes. Full SC01d derives it by wrapping start+raw u16 duration in **seconds**.
SC024 stores start and wrapping start+duration_minutes*60; SC025 zeroes only end.
The existing `non_pvp_duration` retains its raw latest SC01d suffix. These raw
absolute i64 time bits are not mapped through ServerClock's u32 Pong clock; no
estimated timer or phase transition is synthesized.

The same reducer now consumes genuine existing `NoticeEvent::DominionTaxRate`
(020) and canonical `SiegeMember` (0ec). Tax replaces only the rate. Member folding
selects defender first, else offense by expedition ID; append retains duplicates
and requires count<limit, removal swap-removes the first matching ID. Unknown
row/side does nothing. If the selected participant has count or limit>100, its full
row stays unchanged and coverage becomes uncertain: an explicit safe application
boundary around native inline storage. Notice folds do not invent a second
Dominion receipt, change last_receipt or emit native UI callbacks.

Decode failures/unknown DD04 tails remain sticky coverage gaps, including when a
later packet is recovered; no Data flag or later complete row repairs them.
Successful newly decoded024/025 no longer take an unknown-packet path. Existing
character/leave/kick/disconnect resets clear all rows, derived end and last receipt.
WorldState and the Bevy resource use this same reducer.

Original X2Dominion territory open/list/details and peace-tax getters read cache;
the loaded territory window has no collect request. House CS05c queries a bill.
Native siege auction search is representable by existing Auction Search with
filter1/world255, but the lane ignores that filter. CS02d differs from stream
CT00a name lookup and does not guarantee a reply for every ID. Existing Notice
bool and Notice/Housing String carriers remain unchanged and cannot claim full
raw member/alert/brief fidelity. Concrete CryNetwork bool/blob primitives and the
shop prerequisite remain open; fixed-field execution is independent evidence.

Native provenance, exact API approval and prior immutable oracle are under
`data/extract/aa-net/territory-management-20261024/`. Checked-in fixtures at
`crates/aa-net/tests/fixtures/dominion/` pin the original packet field call sites
with explicitly synthetic typed scalar sinks, not concrete archive primitives;
Python independently produces the repeated DD04 alignment input. Focused tests
cover literal bodies, all short prefixes, suffix/DD04 alignment, exact reducer
writes and ordering, raw vectors, sticky gaps/resets, actual local socket frames,
identity/wire guards, failed/missing writes and no optimistic or duplicate events.

Validation: `cargo test --locked -p aa-net --features bevy --lib territory_`
passed all19 focused tests. The full `cargo test --locked -p aa-net --features bevy`
passed775 tests (770 unit,2 scripted-login,3 recovered-packet); all9 live-lane
tests remained ignored. The single `cargo clippy --locked -p aa-net --features bevy
--all-targets` exited0 with existing warnings and one new test-only constant-chunk
style warning. Its suggested one-line test change was applied, then the focused
and full suites reran; Clippy was not repeated under the one-pass limit. Production
source/API stayed frozen throughout. Both the original and final18-file SHA-256
manifests, that exact test-only amendment and logs are under the audit's
`implementation/` subdirectory. Native primitive limitations remain as stated above.

### SC080 raw NPC interaction status (2026-10-24)

`InteractEvent::NpcStatusChanged { npc_obj_id:u32, status_wire_byte:u8 }` now
decodes `SCNpcInteractionStatusChangedPacket` (0x080): BC3 actor followed by one
unchanged wire byte. Existing `ServerMsg::Interaction` and `Event::Interaction`
forward it once in receive order. There is no new resource, top-level event,
outbound request, status/default/enabled projection or optimistic state update.
Root owns the ordered NPC availability/membership fold. SC07b's
`SkillList.interactable` remains a separate receipt and does not supply this value.

Original factory391aa9a0/table399b36fc pins opcode080 and serializer397b4b00.
The serializer clears the actor word on read, transfers three raw bytes through
archive+cc/+d0, then supplies the byte at packet+10 to **BOOL archive+78**, not
u8 archive+44. Root explicitly accepted only the existing Rust/lane one-byte
bool wire convention for the observed carrier. All256 wire byte values remain
unchanged. They are not a claim about post-archive native normalization;
concrete CryNetwork bool implementations/vptrs/construction remain open,
including2/255. Global `Reader::bool` is unchanged.

Original callback391d91a0 calls393be250, which resolves an existing actor before
dirtying NPC metadata and storing its archive-produced status at actor+5fd4.
The native actor constructor/default and zero/nonzero consumer gate belong to
Root's separate native contract. This carrier does not turn a noncanonical wire
byte into that native stored value or manufacture construction/availability state.
The current bevy source at3e0f74498929695d325353b76ffb644be2958a40 defines the
opcode/name but has no typed writer or production sender for it.

Immutable native evidence and the exact accepted proposal remain unchanged under
`data/extract/aa-net/transport-nonmount-20261024/sc080/`. Its96 original-code
invocations use modeled direction/raw/bool leaves: they prove serializer field
order under that model, not concrete bool normalization, native truncation policy
or factory behavior. Three small literal frame/inner fixtures are copied unchanged
to `tests/fixtures/npc-status/`. Focused Rust regressions cover all256 statuses
across four BC3 IDs, mandatory prefixes, suffix preservation, stripping the ordinary
frame's two-byte length, adjacent DD04 statuses followed by fixed SC0ba points,
exact cursor/count, unchanged Session forwarding and no synthetic state/events.
No broader transport API or frozen CharacterState/Unit/OwnUnitSnapshot/Skill/
Slave/Mount/damage/bond/itemlook/territory shape changes accompany this receipt.

Validation: `cargo test --locked -p aa-net --features bevy --lib npc_status`
passed all5 focused tests. Full `cargo test --locked -p aa-net --features bevy`
passed780 tests (775 unit,2 scripted-login,3 recovered-packet); all9 live-lane
tests remained ignored. The single `cargo clippy --locked -p aa-net --features
bevy --all-targets` exited0 with existing warnings and no SC080 warning. No
baseline warnings were changed. Logs, source hashes and prior-evidence
preservation checks are in the new `sc080/implementation/` subdirectory;
the accepted native goldens, fixtures and pins remain unchanged.

### Fixed transport receive carriers (2026-10-24)

The receive-only `TransportReceipt` and `ReceivedRopeState` types are exported
from `transport`. `ServerMsg::Transport` forwards to `Event::Transport` once,
in receive order, through the existing Session and Bevy event bridges. There is
no transport resource, reducer, outbound command/message, predicted success or
new interpretation of existing Unit/Character/Slave/Mount/Skill/bond state.

| Receipt | Exact body under the accepted one-byte bool wire convention |
|---|---|
| SC06d ControllerState | BC3 object, u8 sc_type; **only type0:** f32 length, raw teared byte, raw cutouted byte. Type0 totals10bytes; any nonzero type totals4 and has `rope:None`. |
| SC07a FlyingState | BC3 object, raw flying byte;4bytes. |
| SC137 Hung | BC3 unit, BC3 target;6bytes. |
| SC138 Unhung | BC3 unit, BC3 target, signed i32 reason;10bytes. |

Unknown nonzero controller types are retained without reading a guessed suffix.
Lengths retain their f32 bits, including signed zero, infinities and NaNs;
reasons retain signed extremes. Zero and maximal BC3 receipt IDs remain raw
observations. Missing mandatory bytes error before a receipt is published.
Nothing creates or attaches a unit, starts a controller, sets flying state or
changes position merely because one of these packets was received.

Native field order is pinned by original397c1ea0/397c11d0,397b37a0,
397b5960 and397b5a00 in x2game_unpacked.dll SHA256
851ae243d161c1ac7e6242982d12698ec35163a7913f16292a312df9e85eb27e.
Root accepted only the existing Rust/lane **one-byte BOOL wire convention**
for these four fixed carriers. All256 observed byte values are retained, with
no bool/default/enabled projection and no change to global `Reader::bool`.
The actual native bool normalization remains open, including0/1 and2/255;
native handler gates do not establish that archive behavior. No native
truncation policy, factory behavior, controller admission or physics parity is
claimed by the Rust checks.

The immutable original73-case/140-invocation serializer replay was rerun into
the new ignored `transport-nonmount-20261024/fixed-receive-implementation/`
directory and matches the original golden file byte for byte. Its archive
direction/group/scalar/raw/BOOL methods remain explicit modeled leaves. The
original goldens, source pins and earlier SC080 evidence were not rewritten.
Thirty fixed receive rows from that evidence are checked in under
`tests/fixtures/transport-fixed/`, with a literal DD04 input containing adjacent
type0/nonzero controller states, raw flying bytes0/1/2/255, Hung/Unhung and a
following SC0ba points packet. Tests verify exact cursors/count/order and every
mandatory short prefix, all256 status bytes, float/reason bits, no fallback
suffix, and actual Session-to-Bevy forwarding with unchanged resource data and
change detection. No network thread, server, GPU or device is started by these
new tests.

Shipyard metadata/types/decoding remain excluded because the blob primitive is
unresolved. Outbound transport/controller creation, Movement/MoveCommand,
native/lane movement dialect/math, existing frozen APIs and every existing
literal remain unchanged. Root owns any later controller, glider, hanging,
physical or visual fold using these received observations.

Validation: the focused `cargo test --locked -p aa-net --features bevy --lib
transport_` selection passed15 tests, including the six new fixed-receive
regressions. Full `cargo test --locked -p aa-net --features bevy` passed786 tests
(781 unit,2 scripted-login,3 recovered-packet); all9 live-lane tests stayed ignored.
The original140-invocation replay matched byte for byte with all26 prior audit
artifacts unchanged. This does not close the concrete native archive boundaries.
The single `cargo clippy --locked -p aa-net --features bevy --all-targets`
pass exited0 with existing warnings and no warning in the new receive/test
modules. No baseline warning was changed. Logs and per-file source hashes are
in `fixed-receive-implementation/validation.json` beside the replay evidence.

### Checked hanging request codec (2026-10-25)

`HangingRequest::hang(unit_obj_id, target_obj_id)` and
`HangingRequest::unhang(unit_obj_id, reason)` construct fixed outbound wire
requests. Both reject zero or greater-than-24-bit actor IDs; Hang applies the
same check to its target. Private fields prevent subsequent truncating edits.
These checks are an application boundary, not native eligibility or ownership
proof. Equal representable actor/target IDs are not rejected by this codec.
The accessors `unit_obj_id()`, `target_obj_id()` and `reason()` preserve the
construction inputs; absent fields return None rather than a guessed default.

`game::ClientMsg::Hanging(request)` encodes level1 CS0cb `CSHangPacket` as
BC3 actor then BC3 target (six body bytes), or CS0cc `CSUnhangPacket` as
BC3 actor then signed i32 reason (seven body bytes). Unhang serializes **no
target**. Every reason bit is retained, including negative values and signed
extremes; no reason enum/default or bool/blob convention is introduced.
Original producers393b9012..393b9037/39187350 and serializers397bb360/397bb400
pin those contracts. The native serializer span397bb360..397bb466 has SHA256
16d46374d051207895c725e94a4c1cb8a1da3389bd38d883ca6a28fe3bd4c0ad.

This is packet construction only: there is no new `Command`, Session write
path, Requested/rejected event, received-state reducer, pending action or
predicted hanging/controller state. Existing receive carriers remain unchanged.
Native Hang eligibility and sentinel inputs remain unresolved. A future
session adapter needs captured
active character/own-actor identity, a source-backed live candidate and native
eligibility/cooldown inputs, plus lifecycle generations. **Unhang must capture
and recheck its expected current target before writing**, so a queued stale
request cannot unhang a replacement target. This local guard adds no wire field.

The read-only bevy lane at3d6ec02e62b4e6c25102dfee11dd4b65cebeecec registers
both requests and requires the active character's actor. It reads Unhang's
current sticky target server-side and broadcasts both receipts excluding the
sender; no own-client confirmation can be assumed. Its u32 reason preserves
the same bits. Lane support does not supply the missing native admission,
prediction or physical controller contract. No lane request was sent.

`tests/fixtures/hanging-outbound/native.tsv` contains33 exact original x86
archive outputs with provenance in `source.txt`. Six retained audit cases were
replayed identically, including traces/storage, and27 additional cases cover
three BC3 actors, four targets and five signed reasons. All66 original
serializer invocations used bounded Unicorn and explicit modeled archive
direction/group/raw/scalar leaves; no concrete CryNetwork primitive, native
truncation policy, BOOL/blob, admission, live controller or solver was executed.
These are synthetic scalar inputs through original instructions, not traffic
captures. All44 pre-existing transport audit files remained unchanged.
Replay inputs/traces/instruction pins and preservation hashes are retained
under `data/extract/aa-net/hanging-outbound-20261025/`.

Three new focused regressions verify all33 literal bodies, all256 frame counters
per fixture, exact names/levels/opcodes/lengths, signed reason/accessor fidelity,
and rejection of zero/high-bit IDs without truncation. A local loopback peer
also verifies both literal length-prefixed GameConn frames and no extra request
when the peer supplies no reply. The full `cargo test --locked -p aa-net
--features bevy` passed789 tests (784 unit,2 scripted-login,3 recovered-packet),
with all9 live-lane tests ignored. No GPU or live-lane session was used.
The single final `cargo clippy --locked -p aa-net --features bevy --all-targets`
pass exited0 with existing warnings and none in the new hanging codec/tests.
Logs, owned source hashes and prior-evidence preservation checks are retained
in `hanging-outbound-20261025/validation.json` beside the replay evidence.

### Checked ship-control wire codec (2026-10-25)

`ShipControlRequest::new(slave_obj_id: u32, time: u32, flags: u8,
controller: Option<(u32, u8)>, throttle: i8, steering: i8)` returns a checked
fixed packet carrier. It rejects zero or greater-than-24-bit slave object IDs
and requires the controller tuple exactly when flags bit0x10 is set. Private
fields prevent later invalid edits. Accessors preserve every supplied value;
there is no Default, controller substitution, time conversion or control clamp.
The tuple is full-width `(sc_type, phase)`; explicit `(0, 0)` remains distinct
from no block. Unknown flag bits and the complete signed byte domain are kept.
These are wire/application checks, not native driving admission.

`game::ClientMsg::ShipControl(request)` encodes level1 CS089 `CSMoveUnitPacket`:
BC3 **controlled slave object**, type4, time u32, flags u8, optional controller
type u32/phase u8 for bit0x10, **throttle i8 then steering i8**. Bodies are
exactly11/16 bytes (DD01 payloads17/22 bytes, before the transport length prefix).
No position, velocity, physical normalization, BOOL or blob is serialized.

The existing `Movement::write` always emits the unit body regardless of its
public type byte; changing its type to4 does not produce ship control.
`MoveCommand` constructs type1 only, and the existing receive projection skips
type4 controls/controller fields. This separate checked carrier fills the
outbound gap without modifying either public struct, its literals, the legacy
writer/reader, movement math or native/lane SC06b dialect. There is no conversion
through the lossy received `Movement` representation or a new inbound decoder.

Original serializers397c52f0 ->397c0470 pin this field order. Header branch
397c04e5 checks bit0x10;397c0de9..397c0e24 emits only the two signed controls.
`tests/fixtures/ship-control/native.tsv` retains18 exact earlier golden outputs:
three complete CS089 bodies and15 movement-only outputs, including controller
typefedcba98, phase255, flags0/2/4/10/ff and signed controls. Movement-only rows
are tested after explicit composition with the separately known BC3 prefix;
they are not claimed as additional original full-packet executions.
`source.txt` records the module/source hashes and precise qualification. The
source golden SHA256 is
f329fd17a095bab5d27354c3e284b52bb10bcc27444e8d327a48720c4c0cb92d.
These36 earlier original serializer invocations used modeled scalar/raw archive
leaves, not concrete CryNetwork primitives or traffic captures. No original
replay or controller/physical solver execution is newly claimed here; retained
native inputs, traces and all earlier fixture files remain untouched.

There is no new `Command`, Session write path, event, received-state mutation,
controller creation, current-caster inference or synthetic success. Future
activation requires the actual current own driver's received attachment at
point1, slave incarnation, connection/world generation and any existing native
controller/type/phase provenance, rechecked immediately before writing. Slave
ownership alone is insufficient; a borrowed ship can have a valid driver and
an owned ship can have none. Hanging's native sentinels/cast-pair/physical
admission remain open as detailed in
`data/extract/aa-net/hanging-admission-20261025/REPORT.md`.

Validation: `cargo check --locked --offline -p aa-net --features bevy` passed.
The focused `cargo test --locked --offline -p aa-net --features bevy --lib
ship_control` passed all6 new regressions. They cover the18 retained outputs,
literal DD01 frames/all256 counters, BC3 limits, all256 flags and mismatched
controller presence, all65536 signed control pairs with/without the block, and
time/controller bit patterns/all256 phases. Fixture bytes and both original
serializer span hashes were independently checked against the retained source.

The broader `cargo test --locked --offline -p aa-net --features bevy --lib
--test recovered_packets -- --skip session:: --skip net::tests::
--skip hanging_game_connection` passed666 unit tests plus3 recovered-packet
tests. Its124 filtered unit tests and the unselected login/live integration
targets were deliberately excluded to honor the no-network constraint, including
loopback; this is not a new full-suite/789-test claim. No GPU, live client,
server or socket test ran. The single final `cargo clippy --locked --offline
-p aa-net --features bevy --all-targets` exited0 with existing warnings and no
diagnostic in the new codec/tests. Cargo.lock and all foreign paths were left
to their owners; no dependencies were added.

Full validation follow-up (2026-10-25): after clarification that isolated local
loopback/socket fixtures are authorized, the exact unfiltered command
`cargo test --locked -p aa-net --features bevy` exited 0. It passed **790 unit
tests, 2 scripted-login integration tests and 3 recovered-packet integration
tests: 795 passed total**, with 0 failures and 0 filtered tests. Only the 9
pre-existing live-lane integration tests remained ignored; doc-tests had 0
tests. This includes all 6 ship-control regressions and the existing local
GameConn/Session/socket tests omitted from the earlier offline selection.
No external/live game endpoint or running-server action was exercised.

This follow-up changes validation documentation only. Before/after SHA256
tree checks matched all 229 files under `crates/aa-net` and all 244 retained
evidence files across `transport-nonmount-20261024/`,
`hanging-admission-20261025/` and `hanging-outbound-20261025/`. No source,
fixture, native evidence, root path or Cargo.lock edit was made. Ship-control
Session activation and native driver/controller admission remain disabled;
the full test pass adds no native archive or live-feature completeness claim.

## Lobby header receipts and payment-tail alignment (2026-10-25)

The requested house-list audit found the complete existing path:
`HousingEvent::LoginCharInfoHouse { owner_id, house }` is decoded and forwarded;
`aa-client/login/character_select.rs` passes it into `Bridge::add_house`, which
deduplicates by database house ID and resets on a new character list. The bridge
already exposes character `houseCount`, `GetLoginCharacterInfoHouse`, and premium
grade derived from each received `Character::point`. No second house cache or
decoder was added. The wrapper character ID and embedded owner ID are distinct
wire fields; the current bridge filters the embedded owner, while the existing
event preserves both. Neither the packet nor its list sequence has an explicit
end-of-houses marker, so absence alone cannot prove a complete empty house set.

The bounded missing receive path was the lobby header: the session discarded
`SCGetSlotCount`/`SCAccountInfo`, and used `SCUpdatePremiumPoint` only to advance
the initial handshake. These now reach the app and the existing `Characters`
resource, available through both `WorldState::characters` and `NetPlugin`:

| Producer | Session event | Cached field |
|---|---|---|
| `SCGetSlotCount` 0x1e2, `u8` extra slots | `Event::CharacterSlotCount(u8)` | `Characters::extra_slots: Option<u8>` |
| `SCAccountInfo` 0x1c2, `i32 payMethod, i32 payLocation, i64 payStart, i64 payEnd, i64 realPayTime` | `Event::AccountPayment(AccountPayment)` | `Characters::payment: Option<AccountPayment>` |
| `SCUpdatePremiumPoint` 0x201, `i32 point, u8 oldPg, u8 pg` | `Event::PremiumPoints(PremiumPoints)` | `Characters::premium_points: Option<PremiumPoints>` |

`AccountPayment` and `PremiumPoints` are re-exported plain Copy structs. Their
field names are `pay_method`, `pay_location`, `pay_start`, `pay_end`,
`real_pay_time`, and `point`, `old_grade`, `grade`, respectively. Start/end are
the lane's Unix seconds; `real_pay_time` remains raw signed bits with its meaning
and unit unresolved (the lane writes zero). Payment enums, signed values and
unknown grade bytes are preserved. These receipts establish neither an active
subscription nor permission to create characters. Slot count is extra slots,
not a replacement for the login server's base character limit. The premium
packet has no character ID, and never rewrites `Character::point`/`prev_point`
or substitutes its grade for the character-list grade.

`None` means unreceived, distinct from a received zero. Duplicate values leave
Bevy change detection alone; every actual receipt still has its own event.
Headers survive list refresh, selection, spawn and return to the same connected
lobby (`LeftWorld(1)` followed by `Connecting`). Authentication, world select,
ready-to-connect for a new/retried connection, disconnect, login failure and
kick clear these headers; the existing list/LP/congestion reset behavior is
preserved. The initial premium event
precedes the existing `PhaseChanged(CharacterSelect)`/`LobbyReady` events and
the normal list request. Later premium receipts do not re-enter the lobby or
send a new list request. Truncated packets report the existing decode-failure
event without replacing previously received metadata.

The `game::ServerMsg::AccountInfo` struct variant gains `real_pay_time: i64`.
The decoder now consumes that previously unread trailing word, fixing alignment
of following DD04 packets. Sources are lane `packets/g2c.rs` (including its exact
`lobby_packet_frames_are_byte_exact` test), `packets/c2g.rs::send_account_info`
and the retained original 1.2 archives 397aeb90 and 397b8b00. The source/module
hashes and archive span hashes are pinned in
`tests/fixtures/lobby_headers/metadata.json`.

Five small standalone/batch fixtures are generated offline using Python struct
and raw zlib; the three lane fixtures equal its literal writer goldens. The
varied payment preserves signed extremes and nonzero `realPayTime` containing
packet-like bytes. `native.tsv` records four independently matching original
archive outputs: eight read/write executions with modeled scalar/date archive
leaves, **not** concrete CryNetwork primitives or traffic captures. Native
inspection, the isolated oracle and traces live only in the new ignored
`data/extract/aa-net/lobby-header-20261025/` directory. Prior native artifacts
were left untouched.

Root wiring remains in `aa-client`: extend `Bridge::sync`/the account or premium
API consumers to read these optional `Characters` fields and dispatch the
source-confirmed original UI events where appropriate. The already implemented
house and per-character premium-grade getters need no new aa-net producer.
No UI, server, entitlement policy, mount or movement controller was changed.

Validation: `cargo test --locked --offline -p aa-net --features bevy` passed
795 unit tests plus 2 scripted-login and 3 recovered-packet integration tests
(800 passed, 0 failures, 0 filtered); only the 9 existing live-lane tests were
ignored. The five new tests cover exact lane/native goldens, every standalone
truncation, signed/raw values, the compressed tail, unknown versus zero,
duplicate change detection and connection/character lifecycle. Two isolated
loopback tests cover initial handshake ordering, later lobby/in-world updates,
decode-failure recovery and actual Bevy receive-bridge forwarding. No live
endpoint, running server, external network, UI window or GPU was used.

The single final `cargo clippy --locked --offline -p aa-net --features bevy
--all-targets` exited 0 with existing warnings; no new header or lobby-test
line had a diagnostic. Final source-frozen full tests repeated the same 800
passes/9 live ignores, and scoped `git diff --check` passed. No dependency,
manifest, Cargo.lock or foreign source path was edited for this change.

## Speech-bubble semantic errata (2026-10-25)

This dated update supersedes the earlier **Emotes and speech bubbles**
paragraph's `bubbles` ID interpretation and claim that only the C# duel manager
sends 0x153. That paragraph's server/C# layout provenance remains historical;
the packet table still describes the existing aa-net codec. The retained
reports below establish the following consumer semantics, with their explicit
native execution boundaries. Earlier evidence files remain unchanged.

* **0x153 / SCAreaChatBubble:** native handler `391dc340` ignores `enter`; true
  and false take the same action, never start versus stop. The object is the
  recipient: it must resolve to the local actor before `39370230` looks up the
  type in `sphere_bubbles` / `sphere_chat_bubbles`. The active Rust lane sends
  type 543 at duel start to both participants with the challenger's object ID;
  native therefore admits only that recipient. No active false/exit producer
  was found. Type 543 is the three-row NPC conversation 1173 -> 1174 -> 1175,
  using NPC templates 11996/11997/11541 in the supplied server DB. It does not
  authorize invented duel-start text or a bubble over the challenger.
* **Sphere prerequisites remain root-owned:** an explicit NPC selector must
  find the real speaker, not substitute the recipient. Template selection
  uses nearest matching center distance strictly <15 m; line emission has a
  separate <=30 m center gate and actor-byte check. Queue range cancellation
  instead uses native minimum geometry distance <15 m: actor centers/camera
  distance cannot establish that physical boundary. Speaker availability,
  concrete geometry, game-time scheduling, stop/cancel and actor/world/HUD
  lifecycle authority are required. Root still does not implement this sphere
  path. Its source-backed queue uses native show+fade timing, not 3.5 s gaps.
* **0x154 / SCChatBubble:** an accepted ID resolves `bubble_effects.speech`
  through `396f6350`, not a `bubbles` / `bubble_chats` conversation group. Missing
  effect IDs produce no UI/hook action. Packet kind remains authoritative;
  the DB kind never replaces it. Inline text and resolved effect text enter
  the same direct handler `39371140`. It requires nonempty native C-string
  prefix length, loaded speaker/local actors and their current world-center
  distance <=30 m. It does not inherit the AREA recipient/actor-byte gates.
  Raw kind 4 emits exactly `CHAT_MSG_ALARM(text)`. Every other raw kind emits
  exactly six values:

  ```text
  CHAT_MSG_QUEST(text, actor_name, lowercase_hex_object_id_string,
                 raw_kind_number, show_ms_number, fractional_fade_ms_number)
  ```

  The native tags are three strings then three f32 numbers; no next/qtype/
  forceFinished values, START/END event, dialogue queue or next-row chain is
  appended. The initialized speed wrapper stores integer 100, whose actual
  x87 getter/arithmetic yields **89 ms per accepted code point**; null or
  nonpositive controlled providers use fallback 90. Direct fade is fractional
  SSE f32, not the sphere path's truncated integer. Length 46 gives show 4094
  and fade317.9921875 ms. The direct path has neither sphere's locale 3
  one-hour override nor the separate origin 2 +950 ms branch. Original Lua
  owns its additional 1000 ms visual fade and ignores the supplied fade value.
* **Live-unit strings:** actual `39424820` formatting and `39424840` parsing use
  radix 16. This supersedes the AREA report's explicitly modeled decimal
  formatter, including its nine-value sphere event's speaker-ID spelling.
  `GetUnitId` accepts a unit token and returns lowercase unpadded hex;
  `GetUnitInfoById` / `GetUnitScreenNameTagOffset` require string arguments
  parsed as hex prefixes, not numeric Lua IDs or decimal fallback. For example,
  `"64"` means object 100 and `"100"` object 256. Internal/network numeric IDs
  stay numeric. This does not justify a global rewrite of persistent IDs.
* **Native ID voice hook:** after accepted ID UI delivery (including ALARM),
  re-resolve the loaded actor, then use the enabled bootstrap flag 1 and
  SoundManager's shared voice slot: pack 370 (`voice`), key
  `bubble_effect_<id>`. Direct text has no ID hook. The supplied server-data
  cache misses IDs1/3; a miss preserves any prior voice. This is not proof of
  absence in a live original-client cache. Sound research identifies engine
  virtual +620 as a CRC-generator accessor, superseding the earlier incomplete
  lookup description. Raw stop flag 1 reaches ISound virtual +36, but its exact
  immediate/fade meaning and real CrySoundSystem/device behavior remain open.

The protected concrete CryNetwork archive primitive and wire-variant
normalization remain unproven. Native decoded variants 0(text)/2(ID) do not
establish wire 1/unknown mapping. **The existing aa-net codec remains unchanged:**
`how == 1 -> Text`, otherwise `u32 Id`. Both packets already reach the app via
`EmoteEvent` / `Event::Emote` / `NetEvent`; these findings justify no new packet
API, command, resource or Session state. Root's aa-client integration and final validation are recorded separately in [the client HUD documentation](aa-client-hud.md). This documentation update establishes no published/live implementation or whole-feature, sphere, geometry or audio parity.

Retained proof provenance (read here, not rerun):

* [AREA report](../data/extract/aa-net/area-bubble-research-20261025/REPORT.md):
  26 bounded original-byte cases with modeled archive/registry/UI/formatter/
  ASCII-length/locale leaves. Scheduler entry is captured; the full queue
  update/cancellation lifecycle and geometry remain source-backed boundaries.
* [Direct/ID report](../data/extract/aa-net/chat-bubble-native-20261025/REPORT.md):
  67 bootstrap/direct/ID/gate scenarios; separate 13 conditional archive and 9
  localization controls, 4 enable/key controls, one loader/sort/index scenario
  with 4 lookups, 6 formatter and 4 min/whitespace/NULL controls, plus actor-byte
  and locale 3/kind 2 controls. Registry/transforms/UI/SQL/archive leaves remain
  fixtures. Concrete CryNetwork primitive goldens are not claimed.
* [Unit-ID follow-up](../data/extract/aa-net/chat-bubble-native-20261025/unit-id-followup/REPORT.md):
  36 original binding/type/parser controls plus 12 hex-parser controls, using
  original x2game/xlcommon/CryScriptSystem/CRT with synthetic Lua/world state;
  no full Lua UI, projection renderer or every-token equivalence is claimed.
* [Root speed handoff](../data/extract/aa-client/speech-speed-20261025/ROOT-HANDOFF.txt):
  308 timing cases (44 retained text/locale fixtures x 7 provider states), real
  getter/setter and bounded teardown/reinitialization execution. These close
  initialized 100 -> 89, superseding the AREA report's ordinary 90 timing model;
  controlled raw storage is not proof of a mutable profile/CVar setting.
* [Sound report](../data/extract/aa-sound/chat-bubble-macro-20261025/REPORT.md):
  20 bounded original-byte controls, including cache construction over all 2,409
  supplied voice descriptors; separate 1,429 original decoder blocks establish
  pack 370, and two FEV metadata parses identify the positive asset family.
  SQL descriptors/CRC accessor/engine endpoints remain fixtures; the actual
  create boundary deliberately returns failure 0. No playback success, live
  cache acquisition or exact native stop-fade mapping is claimed.


## Browser (wasm32-unknown-unknown)

`cargo check --target wasm32-unknown-unknown -p aa-net --features bevy` compiles the whole crate.
Native behaviour is unchanged; only the transport, timers and session start differ:

* **Transport.** Browsers have no TCP. `Link::connect(addr)` opens a `web_sys::WebSocket`
  (`binary_type = arraybuffer`) to `ws(s)://<location.host>/net/<addr>` (`wss` on an https page) and
  waits for `open` (error / close first, or 10 s, is a connect error). The `aa-web` server relays the
  binary messages to that TCP address as a plain byte stream, so a message may hold part of a frame or
  several. Incoming messages go through an mpsc channel into the same frame buffer as the TCP path, so
  `recv_payload` stays cancel safe. `send_payload` sends each length-prefixed frame as one message.
  Dropping the link closes the socket. `LoginConn` / `GameConn` / `StreamConn` are unchanged.
* **Runtime.** On wasm tokio is used only for `sync` and `macros`. `Client::spawn_thread*` (what
  `NetPlugin` calls) starts the session with `wasm_bindgen_futures::spawn_local` instead of a thread;
  `spawn_on*` is native only.
* **Timers and time.** `src/timer.rs` holds `timeout` and the 500 ms `Ticker` (tokio natively,
  `gloo-timers` on wasm; no burst catch-up). Non-test code uses `web_time::Instant`, which is
  `std::time::Instant` natively.

## S9: ship steering and type-3 hull movement (2026-10-29)

Live capture: original 1.2 r208022, rowboat (slave 15, `ShipModel` 129/smallboat), `captures/2026-10-29/s910`.
The driver at attach point 1 sent 342 `CSMoveUnit` type-4 requests for the ship object (8911, then 24593);
the server broadcast the type-3 hull and the driver received it too.

* **Type 4 (`ShipControlRequest`, `transport/ship_control.rs`).** Body: `bc` ship id, type 4, `u32` time,
  `u8` flags, `i8` throttle, `i8` steering (`FUN_397c0470` case 4). Every captured request had flags 0
  (the decompile's bit0x01 "local control" was not set). The eight fixtures in
  `tests/fixtures/ship-control/s910-native.tsv` are exact captured bodies: idle 0/0, W 127, S -127,
  A -98 then -127, release -62, D 65, release 94.
* **Type 3 (`Movement::ship`, `model.rs`).** `FUN_397c0470` case 3: position, three velocity shorts,
  three quaternion shorts, three `f32` angular velocities, `i8` steering, `i8` throttle, `u16` zone,
  `u8` stuck. Main skipped steering/throttle/zone/stuck; they are now kept in `ShipMove` and
  `Movement::velocity_mps()` converts the shorts at 30/32767 (the capture's steady segments measured
  1104.5 wire units per m/s; 32767/30 = 1092.2). The five rows in
  `tests/fixtures/ship-move/s910.tsv` are those captured fields in the native layout (S->C bodies were
  not retained by the capture, so the bytes are rebuilt from the decoded values).
* **Driver gate.** Only the local player's attachment point 1 of a `ShipModel` slave steers; point 2
  and mates do not. `Session::send_ship_control` fills the send time and refuses outside the world.
  The steering rate is the ship model's `ship_models.steer_vel` (rowboat 6.0, galleon 1.0); a model
  without a row falls back to 6.0.
* **Cadence.** The non-actor pump (`FUN_394045f0`) is the cart's 100 ms gate, with no change test, so the
  driver resends 0/0 as well. The capture's 127 ms median is the hidden client's frame spacing, not a
  separate interval. The driver receives the type-3 echo (4007 packets for the second rowboat). At the
  leave the capture's last type-4 is 0/0 on the same wall tick as `CSDiscardSlave`, with no type-4
  after it; the HUD slave leave writes that final 0/0 just before the discard request and stops
  the helm sends until `SCUnitDetached`.

Pending live checks: the uncaptured game-option factor in `FUN_3909d8c0` (folded into the rowboat's
fitted `steer_vel`), the steering ramp on a sailboat/clipper, whether flags stay 0
for another account/seat, and the server velocity multiplier (the capture supports 30/32767; the server
code seen offline uses 2048, so a fresh server build may differ).

### S10 type-1 deck standing bodies

`Movement::write` no longer masks `actor_flags::STANDING_ON_OBJECT` (0x20): it
writes the block in the capture's order, `gcFlags` u8, `gcPartId` u16, the
auxiliary position, rotation x/y/z i8, then `gcId` u32 (`FUN_3961e2c0`).
`move_flags::STANDING_ON_OBJECT` (0x40) is the matching header flag; the
capture's walking and standing deck packets are 0x40 and only its two jumps 0x44.
`MoveCommand::support` sets both flags and keeps the ground/jump actor bits
(0x24 walking, 0x34 on a deck jump); header-only 0x40 without the actor flag
still writes no block. The six exact C->S bodies of the 2026-10-29 rowboat
capture are in `tests/fixtures/deck-standing/s910-native.tsv`; the test zeroes
the captured time before re-encoding, so it is byte-exact otherwise.

The rowboat capture confirms the primary `pos` is ship-local in the
(right, forward, up) frame and the auxiliary position is world (median residual
0.581 m for the rowboat). The rotation bytes are resolved: `x` and `y` are 0,
the primary direction is the player's facing relative to the hull, and the
auxiliary `z` is the player's world facing. Both are truncated
`2 pi / 128` direction bytes: the `d_leave` row has ship yaw 1.3977 and player
yaw -1.7344, so the primary is -63 (-3.132 rad relative) and aux `z` is -35;
the `a_idle` row has both facings at pi/2, so primary 0 and aux `z` 32.
