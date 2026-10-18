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
`actability_info_notify.lua`: `ACTABILITY_REFRESH_ALL()` after the snapshot completes,
`ACTABILITY_EXPERT_CHANGED(id)` for point changes,
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

The bounded constant audit found only reads of ten language-ID globals39acb624..648,
excluded Enum group39ac99b8 and special count-exclusion39acb620. Their image contents are
protected/nonplaintext; no direct static initializer assignment was found. Runtime or
protected-data initialization tracing is still needed. Numeric IDs must not be guessed
from catalog language names or enum ordering.

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
