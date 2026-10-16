# aa-net: network client for the AAEmu 1.2 server

Client side of the ArcheAge 1.2 (r208022) protocol, ported from the headless bot in the server repo
(`aaemu-testclient`, `aaemu-commons`, `aaemu-game`/`aaemu-login` packet definitions). It takes the
client from the login screen to standing in the world: authenticate, world list, enter a world,
character list, create / delete, select, spawn, units and doodads around, movement, chat, keep-alive
and logout.

It does not depend on any server crate (no MySQL / sqlx); the opcodes and layouts it uses are copied
into `src/opcodes.rs`, `src/login.rs`, `src/game.rs`, `src/stream.rs` and `src/model.rs`.

## Run it

The lane `bevy` stack must be up (`cd D:/aaemu-wt/bevy && Scripts/live-stack.sh status`): login
`127.0.0.1:27237`, game 27239, stream 27250, world "Lane-bevy". Never stop it; another agent uses it.

```bash
cargo run -p aa-net --example walkthrough                       # full flow, ~25 s
cargo run -p aa-net --example walkthrough -- --roundtrip        # + back to character select / world select / in again
cargo run -p aa-net --example walkthrough -- --delete-test      # + create, delete and list a throw-away character
cargo run -p aa-net --example walkthrough -- --move-secs 120    # + walk a circle for 2 minutes at 10 Hz (keep-alive soak test)
cargo run -p aa-net --features bevy --example bevy_walkthrough  # same through NetPlugin in a headless Bevy app
cargo test -p aa-net                                            # 41 unit tests + 2 scripted-server tests
RUST_LOG=aa_net=debug cargo run -p aa-net --example walkthrough # protocol trace (trace = every game packet)
```

Options of `walkthrough`: `--login HOST:PORT`, `--user NAME` (default `bevytest`), `--password-file
PATH`, `--world ID`, `--character NAME`, `--watch-secs N`, `--roundtrip`, `--delete-test`,
`--move-secs N` / `--move-hz 10` / `--scenario circle|far|underground|jump|fall` (movement soak test),
`--no-ping`, `--ping-interval-ms N`, `--first-ping-delay-ms N` (keep-alive experiments, see
"Keep-alive and the server's dead connection sweep"). Use a throw-away account (`--user NAME
--character NAME`, created on first use) when another client is signed in as `bevytest`: the server
refuses a second game connection for an account that is already online ("enter world denied (1)").

**Accounts.** The login server runs with `AutoAccount`, so an unknown account is created on its first
login with whatever password is sent. The example uses the account `bevytest` (never `clienttest`,
that is the real client's) and reads the password from `--password-file`, `$AA_PASSWORD_FILE`, else
`D:\aaemu-live\cfg\client-test-password.txt`, else the user name. It is never printed or logged
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
  `[u16 opcode][body]`, level 4 = a compressed batch (only used for combat effects; reported as
  `Compressed`, not interpreted). 1.2 traffic is not encrypted.
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
| client → game | `X2EnterWorld`, `FinishState`, `Ping`, `CSListCharacter`, `CSCreateCharacter`, `CSDeleteCharacter`, `CSCancelCharacterDelete`, `CSSelectCharacter`, `CSSpawnCharacter`, `CSNotifyInGame`, `CSNotifyInGameCompleted`, `CSMoveUnit`, `CSSendChatMessage`, `CSChangeTarget`, `CSTeleportEnded`, `CSLeaveWorld`, `CSCancelLeaveWorld` |
| game → client | `X2EnterWorldResponse`, `ChangeState`, `Pong`, `SCReconnectAuth`, `SCGetSlotCount`, `SCAccountInfo`, `SCUpdatePremiumPoint`, `SCRaceCongestion`, `SCCharacterList`, `SCCreateCharacterResponse`, `SCCharacterCreationFailed`, `SCDeleteCharacterResponse`, `SCCancelCharacterDeleteResponse`, `SCCharacterDeleted`, `SCCharacterState`, `SCUnitState` (all unit kinds), `SCUnitsRemoved`, `SCOneUnitMovement`, `SCUnitPoints`, `SCUnitDeath`, `SCTeleportUnit`, `SCDoodadCreated/sCreated`, `SCDoodadRemoved/sRemoved`, `SCDoodadPhaseChanged`, `SCChatMessage`, `SCErrorMsg`, `SCKicked`, `SCPrepareLeaveWorld`, `SCLeaveWorldGranted`, `SCLeaveWorldCanceled` |
| client ↔ stream | `CTJoin`, `TCJoinResponse` (joined and kept open, other traffic ignored) |

Everything else the server sends (inventory, quests, buffs, skills, time of day ...) is decoded as
`game::ServerMsg::Unhandled { level, opcode, len }` and skipped: frames are length delimited, so an
unknown or undecodable packet never desynchronises the stream.

## API

### Protocol layer (plain Rust, no I/O)

| Module | Contents |
|---|---|
| `wire` | `Reader` / `Writer` (little endian, `bc` 24-bit ids, `u16`-prefixed strings and blobs, positions), `Pos` |
| `opcodes` | `ca`, `ac`, `cs`, `sc`, `pp`, `ct`, `tc` constants with `name(op)` |
| `login` | `ClientMsg::{auth, ListWorlds, EnterWorld, ...}` / `ServerMsg`, `World`, `WorldList`, `WorldQueue`, `WorldCookie`, `AccountInfo`, `trion_ticket`, `Secret` |
| `game` | `ClientMsg` / `ServerMsg` (`encode(counter)` / `decode(frame)`), `LeaveTarget`, `VisualOptions` |
| `stream` | `ClientMsg::Join` / `ServerMsg` |
| `model` | `Character`, `CharacterState`, `CreateCharacter`, `ModelParams` + `FaceModel` (full customization), `Equipment` / `Item` (28 slots, `model::slot`), `Unit` / `UnitKind`, `Movement`, `Doodad`, `ChatMessage`, `race`, `gender`, `chat_type`, `stance` |

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
`EnterGame { character_id }` (select, spawn, notify in game), `Move(MoveCommand)`, `Chat`,
`LeaveWorld(Quit | CharacterSelect)`, `CancelLeave`, `ReturnToWorldSelect`, `Disconnect`.

Events (`session::Event`), in the order of a normal run:
`PhaseChanged(Phase)`, `LoginSucceeded(AccountInfo)`, `WorldList`, `WorldQueue`, `ReadyToConnect`,
`LobbyReady`, `RaceCongestion`, `CharacterList`, `CharacterCreated`, `CharacterSelected`, `PlayerSpawn`,
`EnteredWorld`, then `UnitSpawned`, `UnitMoved`, `UnitsRemoved`, `UnitPoints`, `UnitDeath`,
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

`Worlds { list, queue, account }`, `Characters { list, race_congestion }`, `Player { spawn }`,
`Units { by_id }`, `Doodads { by_id }`, `Status { phase, last_error }`; each has `apply(&Event) -> bool`,
`WorldState` bundles them. They clear themselves on `LeftWorld` / `Disconnected`.

### Bevy plugin (`--features bevy`, `aa_net::NetPlugin`)

```rust
app.add_plugins(aa_net::NetPlugin);
// send:    MessageWriter<NetCommand>  -> out.write(NetCommand(Command::Login { .. }))
// receive: MessageReader<NetEvent>    -> for NetEvent(event) in events.read() { .. }
// state:   Res<Worlds>, Res<Characters>, Res<Player>, Res<Units>, Res<Doodads>, Res<Status>
```

The plugin starts the session thread in `build`, forwards `NetCommand` messages in `PreUpdate`, drains the
events, applies them to the resources (change detection only fires for resources an event changed)
and republishes them as `NetEvent` messages. Everything happens before `Update`.
`cargo run -p aa-net --features bevy --example bevy_walkthrough` is a complete example.

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
| `SelectCharacter`, `CreateCharacter`, `DeleteCharacter`, `CancelCharacterDelete`, `RequestCharacterListRefresh` | `Command::EnterGame`, `CreateCharacter`, `DeleteCharacter`, `CancelDeleteCharacter`, `RefreshCharacters` |
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
then); `CancelDeleteCharacter` answers status 3 (restored) or 4 (unknown character).

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

## Known gaps

* Korean auth flows (challenge, OTP, ARS) and Mail.Ru / Tencent logins; only the Trion ticket.
* Server → client packets not decoded (skipped): inventory and equipment changes after spawn, skills and
  combat (`DD04` batches included), quests, buffs, mail, houses in the character list
  (`SCLoginCharInfoHouse`, so `GetLoginCharacterInfoHouse`), premium grade of the lobby, time of day,
  `SCUnitMovements` (batched, no per-entry type byte). Add a variant to `game::ServerMsg` + `Event` for
  each as the UI needs it.
* Client → server: no skills, interaction, inventory or party packets yet.
* NPC names are empty on the wire (the client resolves them from the NPC template); `Unit.kind` has the
  template id. Slaves / transfers / shipyards only decode their core fields.
* Movement: a plain unit movement (ground, jump, fall with landing velocity); vehicles, ships, standing
  on / hanging from objects are not written. The server does not echo our own movement back.
* Reconnect after a dropped game connection is not attempted (the session reports `Disconnected` and
  returns to idle).
