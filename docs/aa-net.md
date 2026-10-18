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
| client → game | `CSInteractNPC`, `CSInteractNPCEnd`, `CSStartInteraction`, `CSStartQuestContext`, `CSCompleteQuestContext`, `CSDropQuestContext`, `CSQuestTalkMade` (see "Quests and NPC dialogs"), `CSLootOpenBag`, `CSLootItem`, `CSLootCloseBag`, `CSLootDice`, `X2EnterWorld`, `FinishState`, `Ping`, `CSListCharacter`, `CSCreateCharacter`, `CSDeleteCharacter`, `CSCancelCharacterDelete`, `CSSelectCharacter`, `CSSpawnCharacter`, `CSNotifyInGame`, `CSNotifyInGameCompleted`, `CSMoveUnit`, `CSSendChatMessage`, `CSChangeTarget`, `CSUpdateActionSlot`, `CSSwapItems`, `CSSplitBagItem`, `CSDestroyItem` (see "Moving items"), `CSStartSkill` (skills and items), `CSStopCasting`, `CSResurrectCharacter`, `CSTeleportEnded`, `CSLeaveWorld`, `CSCancelLeaveWorld` |
| game → client | `X2EnterWorldResponse`, `ChangeState`, `Pong`, `SCReconnectAuth`, `SCGetSlotCount`, `SCAccountInfo`, `SCUpdatePremiumPoint`, `SCRaceCongestion`, `SCCharacterList`, `SCCreateCharacterResponse`, `SCCharacterCreationFailed`, `SCDeleteCharacterResponse`, `SCCancelCharacterDeleteResponse`, `SCCharacterDeleted`, `SCCharacterState`, `SCUnitState` (all unit kinds), `SCUnitsRemoved`, `SCOneUnitMovement`, `SCUnitPoints`, `SCUnitDeath`, `SCTeleportUnit`, `SCDoodadCreated/sCreated`, `SCDoodadRemoved/sRemoved`, `SCDoodadPhaseChanged`, `SCChatMessage`, `SCErrorMsg`, `SCKicked`, `SCPrepareLeaveWorld`, `SCLeaveWorldGranted`, `SCLeaveWorldCanceled`; for the HUD (see "In-game HUD data"): `SCExpChanged`, `SCAbilityExpChanged`, `SCRecoverableExp`, `SCLevelChanged`, `SCAbilitySwapped`, `SCCharacterLaborPowerChanged`, `SCCharacterGamePoints`, `SCGamePointChanged`, `SCBmPoint`, `SCCombatEngaged/Cleared`, `SCUnitFactionChanged`, `SCCharacterResurrected`, `SCBuffCreated/Removed/Learned`, `SCActionSlots`, `SCCharacterInvenInit/Contents`, `SCItemTaskSuccess`, `SCUnitEquipmentsChanged`, `SCInvenExpanded`, `SCTimeOfDay`, `SCDetailedTimeOfDay`, `SCOnOffSnow`, `SCConflictZoneState`, `SCJoinedChatChannel`, `SCLeavedChatChannel`, `SCTargetChanged`, `SCSkillStarted/Fired/Ended/Stopped`, `SCCastingStopped`, `SCLootableState`, `SCUnitLootingState`, `SCLootBagData`, `SCLootItemTook`, `SCLootItemFailed`, `SCLootDice/Notify/Summary`, `SCCooldowns`, `SCSkillCooldownReset`, `SCSkillLearned/Upgraded`, `SCSkillsReset`, `SCUnitDamaged`, `SCUnitHealed`, `SCCombatText`, `SCPlotEvent/Ended/CastingStopped/ChannelingStopped`, `SCQuests`, `SCCompletedQuests`, `SCQuestContextStarted/Updated/Completed/Reset`, `SCNpcInteractionSkillList`, `SCDoodadQuestAccept`, `SCQuestRewardedByMail`, `SCNpcChatMessage`, `CompressedGamePackets` |
| client ↔ stream | `CTJoin`, `TCJoinResponse` (joined and kept open, other traffic ignored) |

Everything else the server sends (houses, guilds, mount-owner packets ...) is decoded as
`game::ServerMsg::Unhandled { level, opcode, len }` and skipped: frames are length delimited, so an
unknown or undecodable packet never desynchronises the stream.

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
helper are removed. Once compression is valid, existing packet-body semantics remain:
an unknown opcode reports `Unhandled` and ends that batch; a malformed packet body ends
it after already decoded packets, because DD04 bodies have no individual lengths.
The header's count is retained as a diagnostic hint rather than invented per-packet
lengths or permission to scan past an unknown body.

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
the table is `model::exp`), `ability_level(id)`, `money_parts() -> (gold, silver, copper)`,
`hp_fraction()`, `mp_fraction()`, `LABOR_MAX` (2000; 5000 for premium). The title in the unit frame
("Battlerage Novice") comes from `active_abilities` and `ability_level`.

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
| `SCSkillStarted` (`result` 0) | `SkillStarted(SkillStarted)` | `casting[caster]`, and for the player `last_used[skill]`, `last_cast` and `last_cast_skill` (the skill the global cooldown comes from) |
| `SCSkillStarted` (`result` != 0, `tl` 0) | `SkillStarted` with `failed()` | `last_failure` (`skill::result::name`) |
| `SCSkillFired` | `SkillFired` | `casting[caster].fired`; starts the skill when no `SkillStarted` came first |
| `SCPlotEvent` | `PlotEvent { tl, event_id, skill_id, caster_obj_id, target_obj_id, caster_pos, target_pos, casting_ms }` (`event_id` is the `plots` step; the positions are the ones a step aimed at the ground carries) | the first step of a plot skill starts it (`casting`, `last_used`); steps with `casting_ms` > 0 set the cast time |
| `SCSkillEnded`, `SCCastingStopped`, `SCSkillStopped`, `SCPlotEnded`, `SCPlotCastingStopped`, `SCPlotChannelingStopped` | `SkillEnded`, `CastingStopped`, `SkillStopped`, `PlotEnded`, `PlotStopped` | `casting` entry removed |
| `SCCooldowns` | `Cooldowns(SkillCooldowns)` | received skill/tag timers appended for an existing local player (see native cooldown lists below) |
| `SCSkillCooldownReset` | `SkillCooldownReset { obj_id, skill_id, tag_id, gcd }` | `last_used[skill]` removed, `reset_tags`, first matching received entries reset, GCD cleared when requested |
| `SCUnitDamaged` | `UnitDamaged(UnitDamage { action, caster_obj_id, target_obj_id, damage, absorbed, mana_burn, hit_type })` | event only (HP follows in `UnitPoints`) |
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
The server only runs skills the character learned (`Skills::known`) or default skills (the melee
auto attack, id 2). A refused skill comes back as `SkillStarted` with `result` != 0 and `tl` 0.

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
| `Start { npc_obj_id, obj_id, extra_info, pick_id, mouse_button, modifier_keys }` | `CSStartInteraction` 0x068: `bc bc i32 i32 u8 i32` | `SCNpcInteractionSkillList` (the NPC's function skills) |
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
| `SCUnitAttached` 0x076 | `UnitAttached` | `bc rider`, `u8 attach point` (1 driver, 2.. passengers), `bc mount`, `u8 reason`; `Unit::attached_to` |
| `SCUnitDetached` 0x077 | `UnitDetached` | `bc rider`, `u8 reason` |
| `SCUnitModelPostureChanged` 0x103 | `UnitPosture` | `bc id`, `Unit.ModelPosture` (posture byte, loot flag, per posture: 4 = `u32 anim_actions.id` + `bool activate`); `Unit::anim_action` is the pose an NPC stands in (`SCUnitState` carries it too) |
| `SCCombatFirstHit` 0x089 | `CombatFirstHit` | `bc victim`, `bc hitter`, `u32 hit type` |
| `SCAiAggro` 0x1c4 | `AiAggro` | `bc npc`, `i32 count`, per hostile unit `bc`, three `i32` (the first is the damage it did) and a flag byte (135) |

`Movement::read_typed(type, r)` reads all movement types: unit (1; `Movement::support` holds the object the unit stands on or
hangs from, with its world position, when the actor flags say so), vehicle (2) and ship (3) (rotation shorts as a quaternion
without the scalar -> `vehicle_yaw`, angular velocity, steering ...), ship request (4, no position) and transfer (5). The
state resource puts a unit that stands on an object at its world position and sets the yaw of vehicles from the rotation.

NPC movements: the server sends a step every 100 ms with `flags` 0, `actor_flags` 4 (running) or 5 (walking), `delta_movement`
`[0, 127 | 63, 0]`, the stance and alertness of the NPC (`alertness` 2 = in combat) and a **velocity of a fixed 4000** (3.9 m/s
whatever the speed): the speed has to be measured from the positions. Players send their own velocity (1/1024 m/s), the
pressed direction in `delta_movement` and, in the air, no `STANDING_ON_SOLID` (a jump: `JUMPING` and flags `Moving | Stopping`).
Mounts move like any unit; the rider gets no movement of its own (the server puts its local position at 0, 0, 0 under the mount).
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
It creates a uniquely named throw-away account and Nuian character on login port 27237, moves a
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

`Doodads` applies changed data and the stored `owner_id` only to known objects.
The owner is a full u32 stored character ID, not the separate three-byte owner object ID;
the reducer does not infer parent, owner type, permissions, phase or faction state.
Zero owner/data values clear those fields. Unknown and removed objects stay absent.
Faction and sound IDs remain raw events for consumers to interpret; a sound does not
mutate the doodad or invent its position. Audio consumers can resolve a currently known
object through `Doodads`.

Three additional offline tests check the exact source goldens, every truncation, all
three updates followed by phase and another DD04 opcode, signed/full-width event
forwarding, known-only updates, clearing and preservation of unrelated fields.
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
replicated by sending extra guessed packets.

**KickPassenger is a lane gap:** native 390697d0 /39361fc0 sends Unmount with passenger
point 2 and reason 11, but the lane's mates::unmount_mate accepts only a seat occupied by
the requesting character. An owner trying to eject a different passenger is silently
rejected. No functional kick method or fake successful detach is exposed by aa-net.

`Command::UseMateSkill { skill_id, mate_obj_id, mount_skill_id, target_obj_id }` sends the
existing CSStartSkill 052 with explicit mount caster. `skill_id` and `mount_skill_id` must
be nonzero; caster and target are explicit nonzero 24-bit unit IDs. No target 0 self alias,
automatic target selection, learned-skill check or mount-slot resolution is imposed.
For a pet self buff the caller explicitly supplies mate_obj_id as target. The exact
17-byte body is u32 combatSkillId, u8 casterKind 3, bc mateObjId, u32 mountSkillId,
u8 targetKind 0, bc targetObjId, u8 flag 0. The low-level
`skill::StartCaster::Mount { obj_id, mount_skill_id }` is also available.
The player's own object ID is not substituted into the mount caster.

Local `MateEvent::SkillRequested { skill_id, mate_obj_id, mount_skill_id, target_obj_id }`
and `SkillRejected { same fields, message }` do not create a cast or consume resources.
Actual SkillStarted/Fired/Ended and costs remain authoritative. The lane's mount authority
permits an unmounted owner or a passenger with a matching occupied seat, riding flag and
parent relation; it does not impose a static learned-skill allowlist (temporary grants are
supported). Its on_mount_skill_used separately resolves a linked rider skill by attach
point; the client must not duplicate that server-side extra cast.

Native pet action slots are established through `ISLOT_PET_ACTION` rather than a guessed
X2Mate:StartMountSkill method. Static mount_skills/mount_attached_skills mapping, native
slot eligibility, target planning, automatic-use toggles and per-pet cooldown presentation
remain aa-client integration work; raw dynamic triples are not guessed cooldowns.

Three new offline tests check every control's source-backed body/full frame, TL/point/state
and target bounds, zero target clearing, raw reason preservation, unchanged owned state
after requests/rejections, distinct combat/mount/actor IDs, and an ephemeral loopback
session proving invalid/out-of-world requests send nothing and successful sends wait for
real server errors / mount-caster SkillStarted. Full `cargo test -p aa-net` passed 200
library and 2 scripted-session tests, with 9 live tests ignored;
`cargo check -p aa-net --features bevy --examples` passed. No live lane traffic was used.

## Known gaps

* Korean auth flows (challenge, OTP, ARS) and Mail.Ru / Tencent logins; only the Trion ticket.
* Server → client packets not decoded (skipped): houses in the character list
  (`SCLoginCharInfoHouse`, so `GetLoginCharacterInfoHouse`), premium grade of the lobby, guilds,
  slave lifecycle and mate equipment owner packets
  (`SCSlaveCreated`, `SCMateEquipmentChanged` ...; the units themselves and riding are decoded, see
  "Other units' motion"), `SCBuffUpdated` (the lane server never sends it and has no writer for it, so its
  layout is unknown; a buff's time is refreshed by `SCBuffCreated` with the same index). Add a
  variant to `game::ServerMsg` + `Event` for each as the UI needs it. A packet in a `DD04` batch
  that is not decoded ends that batch.
* Client → server: skills, items, target, action bar, resurrect, chat, logout, NPC talk and quests,
  shops (buy/sell/buyback), bank money/expansion, item moves (swap, split, destroy), player trade, auctions.
  Coin/doodad shops, priest buffs and the other NPC functions still need their own requests.
* No maximum HP / MP on the wire: `PlayerStats::max_hp` is the highest value seen (a character that
  spawns dead or hurt shows too small a maximum until it has been at full health once) unless the
  application sets the formula value with `PlayerStats::set_max` (the HUD does).
* Quests: `SCQuestContextFailed`, `SCQuestUnitReqFailed` and the quest error notices are not decoded
  (the lane server never sends them). The objective texts are not on the wire: the HUD makes them from
  the game database.
* The sub zone name of the minimap header is not sent: derive it from the position and the
  `sub_zones` table. The zone group follows the zone chat channel (Shout).
* NPC names are empty on the wire (the client resolves them from the NPC template); `Unit.kind` has the
  template id. Slaves / transfers / shipyards only decode their core fields.
* Movement: we only write a plain unit movement (ground, jump, fall with landing velocity); vehicles, ships,
  standing on / hanging from objects are read (`Movement::read_typed`) but not written. The server does not
  echo our own movement back.
* Reconnect after a dropped game connection is not attempted (the session reports `Disconnected` and
  returns to idle).


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
automatic response. Unknown opcodes still stop a DD04 batch: this is a focused coverage milestone,
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

Bounded native audit of `D:/aa-re/client12/x2game_unpacked.dll`:

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

Native exact wire audit against `D:/aa-re/client12/x2game_unpacked.dll`:

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

Native 1.2 reader/consumer evidence (`D:/aa-re/client12/x2game_unpacked.dll`):

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

Original `D:/aa-re/client12/x2game_unpacked.dll` (image base 0x38ff0000): wrapper3944c8a0
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

### Received house snapshots and native/lane model boundary (2026-10-18)

`housing::{HouseSnapshot, HousingEvent, Houses}` adds the missing read-only house metadata
resource. The active SC0bc dispatch follows this project's bevy lane
`models/house.rs::House::write` exactly. A separate explicit `HouseSnapshot::read_native`
reads the original client format; there is no payload heuristic or transport profile.

Native factory391e3e48 installs vtable399b765c and opcode0xbc. Its serializer397ab210
passes packet+10 to39897460 (`NetHouse`). The field sequence is:

| Field | Lane wire | Native wire |
| --- | --- | --- |
| timeline, database ID, runtime object ID, housing template | u16, u32, BC, u32 | same |
| model | PISC(model_id, second value) | one fixed32-bit value (`ht`, virtual+4c at39897501) |
| co-owner ID, owner character ID, owner name, account ID | u32, u32, string, u32 | same; native owner string capped128 bytes |
| permission, total/current build actions, tax | u8, i32, i32, i32 | same |
| position | i64 X, i64 Y, f32 Z | same;3961e270 |
| house name, recover flag | string, byte boolean | same; native name capped128 bytes |
| sale price, designated buyer ID/name | 32-bit money, u32, string | same; native buyer name capped128 bytes |

The model mismatch is concrete:39897501 uses virtual+4c, exactly the same method used by
`allstep`398975ef and `curstep`39897607. The lane instead calls `write_pisc(&[model_id,0])`.
The active reader preserves both PISC values (`lane_model_aux` is Some); the native reader
has no auxiliary value. Native signed money bits remain i32; the lane's unsigned sale
price is preserved bit-for-bit. Raw permission values and raw long XY are retained rather
than converted into guessed permission policy or quantized spatial state.

SC0bd build progress (u16 TL, u32 model, i32 total/current actions) and SC0be permission
(u16 TL, u8) update known snapshots only. Source handler391d7890 resolves TL through
3932b510 and calls39327990 with the three progress fields;391d7920 similarly resolves TL
before permission setter39325df0. Unknown TL updates never construct a house/unit.
SC0c0 demolition carries u16 TL. Handler391d7a40 calls3932a300, which marks native+148
rather than immediately erasing the object. `House::demolished` therefore records the
received notice; it does not fabricate a `UnitsRemoved` event. Genuine unit removals
remove matching snapshot object IDs. Character selection, world leave and disconnect
clear the resource; a fresh full snapshot replaces its TL record and resets the notice.
All received actions, including repeated snapshots, remain forwarded session events even
when the resource's change detection sees identical state.

Native OnHouseState391d77f0 first requires an existing runtime object at packet+18,
then passes NetHouse to3932fa00 and emits native event0x104. `Houses` records received
metadata separately and does not fabricate an ordinary unit, world placement, current
interaction or housing UI event from the snapshot alone. Existing `UnitKind::Housing`
provides the independent received unit model/build-step shell.

SC0bc contains no decoration list or precomputed proficiency bonus. Existing received
`Doodad` records preserve owner/parent object IDs, item_template_id and owner metadata,
and existing removal events remove them. The prior source-proven `housing_decorations`
static aggregation and house permission/enclosure predicates remain required before an
app can calculate actual local proficiency modifiers. This milestone adds neither a
synthetic decoration packet nor an enclosure/bonus calculation. My-house/login lists,
sale/tax/recovery deltas and construction commands remain separate follow-ups.

Validation: `cargo check -p aa-net --features bevy` passes; the full library suite is
282 passed. Four new tests cover independent lane/native goldens, exact DD04 tail
alignment, every payload truncation, native128-byte name bounds, unknown permission
values, repeated reports, delta-before-snapshot, object/TL replacement, demolition versus
actual removal, lifecycle resets and ordered session forwarding. No live housing command
or server mutation was attempted.

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
needed for the authoritative setting API. Playerflag1000 producer remains unresolved
and must not be replaced with Unit.flags or a guessed SCInitialConfig feature bit.

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
both known prior unit history and the matching static row; missing either leaves None.
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

### Received GM mode bytes and snapshot hydration (2026-10-18)

SC203 `UnitGmModeChanged` retains BC runtime object ID, signed i32 mode, and raw u8 value. Lane `packets/g2c_misc.rs::SCUnitGmModeChangedPacket` writes exactly those fields. Native handler39186420 registration factory391b7b50/vtable399b6144 identifies opcode203; serializer397c7270 uses397b27d0 for BC ID, packet+10 mode and+14 value. Native writes raw actor[3af8+mode]; the safe state fold bounds mutation to indices0..3, preserving every unknown signed mode as a received event. Noncanonical byte values such as255 remain exact.

`Unit.gm_modes:[u8;4]` hydrates from the little-endian bytes of the FIRST integer in the common four-value pack immediately after raceGender, not from the later u16 `Unit.flags`. Native397c9882?397b21d0 decodes this pack to UnitState+2020/+2024/+2028/+202c; the reverse397c9a81..ad8 appends the four integers via397ad500 and emits the named pish. The existing lane parser calls this integer-pack codec PISC: lane snapshot builders currently emit first integer0 (both character and net-unit paths), while third integer remains appellation. Native default flags word3a5e136c and reset398a1aa0 are zero. No extra four bytes are added to lane framing.

Player and Units folds update only a matching existing runtime unit, never create an actor from SC203. Full snapshots replace old bytes; removal/lifecycle removes the owning state. Session forwards repeated/unknown reports without synthetic changes or responses. Mode1 is source-used as a cooldown insertion suppression byte: the cooldown manager owner pointer is RAWACTOR+8, so its owner+3af1 test refers to raw actor+3af9. This milestone retains the producer only and adds no cooldown timing/sidecar. UI source closure distinguishes these bytes from attachment IDs and model flags. Tests cover all modes0..3, unknown negative/out-of-range indices, raw255, each truncation, following DD04 packet alignment, snapshot word byte order, independent u16flags/appellation, local Player+Units mutation, missing/removed/reused objects, duplicate Session reports, and Bevy resource lifecycle.

### Native received stop-duration cooldown fold (2026-10-18)

`Skills.apply` now consumes the actual nonzero duration in `CastingStopped` and `PlotStopped` before ending the tracked cast. Source391d5cf0?39396000 and391d5af0/391d5b70?3938c410/3938d970 resolve the tracked skill/actor and invoke39899f90 before teardown. This path does not evaluate A74, database cooldown lengths or Started/Fired timing. `set_cooldown_catalog(HashMap<u32,u32>)` accepts `SELECT id,cooldown_tag_id FROM skills`; present zero routes to a direct skill row, nonzero to the primary tag, and a missing catalog entry stays unknown. The catalog survives PlayerSpawn, selection, disconnect, kick and leave resets; dynamic timer/context state does not.

The existing `received_skill_cooldowns`, `received_tag_cooldowns` and `received_cooldown[_at]` remain compatible original SCCooldowns views. Separate ordered `native_skill_cooldowns`/`native_tag_cooldowns` in the same Skills resource hold mutable manager rows with origin Snapshot, CastingStopped{tl} or PlotStopped{tl}; snapshots seed both views, stop extensions never rewrite raw receipt rows. `native_cooldown[_at](skill,tags)` selects tags in supplied order then direct rows, greatest remaining with strict greater replacement, preserving first ties/total0. Timers compare elapsed whole monotonic milliseconds. Native39899f90 extends only the FIRST matching ID when the supplied duration is strictly larger, replacing both remaining and total; expired/reset matching rows are reused in place. A missing ID at capacity150 compacts zero-remaining rows in original order, then appends only if space exists. Genuine reset reports affect the first matching runtime row as well as the original receipt view.

Explicit conservative boundary: current cast storage is keyed by actor, not the native timeline tree. Insertion requires exactly one tracked matching timeline, owned by current local object ID; ambiguous/missing/foreign timelines produce no timer. This does not claim native first-TL traversal parity or a complete remote actor cooldown manager. Native GM suppression byte1 must be source-known zero from PlayerSpawn.gm_modes or a matching SC203 report; unknown/nonzero blocks insertion. Local object removal invalidates that context until fresh PlayerSpawn/SC203. Zero duration is a no-op for timers; existing cast teardown remains unchanged. Every genuine raw stop event remains available to app consumers even if no timer is admitted.

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
CS00d reply is u32 inviter ID, u32 expedition ID, u8 join. Native constructors
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
then394245f0 decomposes365-day years/30-day months/days/hours/minutes/seconds;
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

Non-gate deltas (tax/member-limit/member-list/alerts) remain outside this slice;
rows expose the last explicitly provided fields for that metadata. Native SC0eb
392f3670 only changes defender member limit; SC0ec392f5f60 only edits participant
member lists; SC0ed391d955a emits an alert. These do not write phase or participant
expedition IDs. Normal SC024/025 remain conservatively unintegrated and invalidate
gate certainty. Known five mutators never fabricate metadata or success notices.

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
f32 Z, f32 rotation and two BC object IDs; Position2 retains raw position plus
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
Its source is `D:/aa-re/client12/ida/xlcommon.c`330023e0..33004be0.

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
