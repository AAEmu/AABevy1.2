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
  stream of [u16 0][u16 opcode][body]]`; the server writes stored blocks, which are unpacked into
  `ServerMsg::Batch`). 1.2 traffic is not encrypted.
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
| client → game | `CSInteractNPC`, `CSInteractNPCEnd`, `CSStartInteraction`, `CSStartQuestContext`, `CSCompleteQuestContext`, `CSDropQuestContext`, `CSQuestTalkMade` (see "Quests and NPC dialogs"), `CSLootOpenBag`, `CSLootItem`, `CSLootCloseBag`, `X2EnterWorld`, `FinishState`, `Ping`, `CSListCharacter`, `CSCreateCharacter`, `CSDeleteCharacter`, `CSCancelCharacterDelete`, `CSSelectCharacter`, `CSSpawnCharacter`, `CSNotifyInGame`, `CSNotifyInGameCompleted`, `CSMoveUnit`, `CSSendChatMessage`, `CSChangeTarget`, `CSUpdateActionSlot`, `CSSwapItems`, `CSSplitBagItem`, `CSDestroyItem` (see "Moving items"), `CSStartSkill` (skills and items), `CSStopCasting`, `CSResurrectCharacter`, `CSTeleportEnded`, `CSLeaveWorld`, `CSCancelLeaveWorld` |
| game → client | `X2EnterWorldResponse`, `ChangeState`, `Pong`, `SCReconnectAuth`, `SCGetSlotCount`, `SCAccountInfo`, `SCUpdatePremiumPoint`, `SCRaceCongestion`, `SCCharacterList`, `SCCreateCharacterResponse`, `SCCharacterCreationFailed`, `SCDeleteCharacterResponse`, `SCCancelCharacterDeleteResponse`, `SCCharacterDeleted`, `SCCharacterState`, `SCUnitState` (all unit kinds), `SCUnitsRemoved`, `SCOneUnitMovement`, `SCUnitPoints`, `SCUnitDeath`, `SCTeleportUnit`, `SCDoodadCreated/sCreated`, `SCDoodadRemoved/sRemoved`, `SCDoodadPhaseChanged`, `SCChatMessage`, `SCErrorMsg`, `SCKicked`, `SCPrepareLeaveWorld`, `SCLeaveWorldGranted`, `SCLeaveWorldCanceled`; for the HUD (see "In-game HUD data"): `SCExpChanged`, `SCAbilityExpChanged`, `SCRecoverableExp`, `SCLevelChanged`, `SCAbilitySwapped`, `SCCharacterLaborPowerChanged`, `SCCharacterGamePoints`, `SCGamePointChanged`, `SCBmPoint`, `SCCombatEngaged/Cleared`, `SCUnitFactionChanged`, `SCCharacterResurrected`, `SCBuffCreated/Removed/Learned`, `SCActionSlots`, `SCCharacterInvenInit/Contents`, `SCItemTaskSuccess`, `SCUnitEquipmentsChanged`, `SCInvenExpanded`, `SCTimeOfDay`, `SCDetailedTimeOfDay`, `SCOnOffSnow`, `SCJoinedChatChannel`, `SCLeavedChatChannel`, `SCTargetChanged`, `SCSkillStarted/Fired/Ended/Stopped`, `SCCastingStopped`, `SCLootableState`, `SCUnitLootingState`, `SCLootBagData`, `SCLootItemTook`, `SCLootItemFailed`, `SCSkillCooldownReset`, `SCSkillLearned/Upgraded`, `SCSkillsReset`, `SCUnitDamaged`, `SCUnitHealed`, `SCCombatText`, `SCPlotEvent/Ended/CastingStopped/ChannelingStopped`, `SCQuests`, `SCCompletedQuests`, `SCQuestContextStarted/Updated/Completed/Reset`, `SCNpcInteractionSkillList`, `SCDoodadQuestAccept`, `SCQuestRewardedByMail`, `SCNpcChatMessage`, `CompressedGamePackets` |
| client ↔ stream | `CTJoin`, `TCJoinResponse` (joined and kept open, other traffic ignored) |

Everything else the server sends (mail, houses, guilds, trading, looting windows ...) is decoded as
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
| `SCSkillCooldownReset` | `SkillCooldownReset { obj_id, skill_id, tag_id, gcd }` | `last_used[skill]` removed, `reset_tags` |
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

The server sends no cooldown lengths or timers; the client times from the last use. The HUD
reads `Skills::cooldown_left(skill_id, cooldown_tag_id, cooldown_ms)` (`skills.cooldown_time` and
`cooldown_tag_id` come from the game database), `gcd_left(ms)`, `since_used(skill)` and
`casting_of(obj_id) -> Option<&Casting>` (`progress()` is the cast bar, 0.0 to 1.0; the target frame
shows the target's cast with the same call). `UnitDamage::action` says which skill (or buff tick,
`kind` 2) dealt it; `hit_type` is `skill::hit_type::*` (`is_critical`, `is_miss`). The server's
spell results are messages in `skill::result`.

Skill results arrive in `DD04` batches (stored deflate blocks of `[u16 0][u16 opcode][body]`). The
session unpacks them and reports every packet as if it had come alone, in order: `Event::UnitDamaged`
after `PlotEvent`s and so on. A packet in a batch this client cannot decode ends the batch (the
rest cannot be found: the bodies have no length); real Huffman compressed batches (the C# server)
are reported as `ServerMsg::Compressed` and skipped.

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
in this two-person test. The API does not yet send raid conversion, slot/role/loot-rule edits,
markers, or pings; those incoming changes are exposed for the HUD.

For an integrated HUD check, `cargo run -p aa-net --example party_peer -- --invite CharacterName
--seconds 90` creates a fresh letter-only character and prints `READY account=... character=...`.
It invites the supplied character once, accepts invitations received by its own throw-away
character, sends a stationary movement every two seconds to refresh party vitals, then leaves
the party and logs out. Coordinate with the visual client before starting its lifetime.

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
`WithdrawMoney { amount, aa_point }`, and `Expand { container, auto_use_aa_point }` (2 bag,
3 bank). Negative amounts and invalid containers are refused locally. Bank slots and contents
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
`page(show_offline,start_zero_based,count)` (preserves server order).

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
is cleared when sending and filled only by the server's `ItemsChanged` reason3 (repair).
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
must not be presented as received server errors. Confirmation sends existing
`BankCommand::Expand`; `InventoryExpanded` alone confirms capacity. Current rows expand by10,
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

## Known gaps

* Korean auth flows (challenge, OTP, ARS) and Mail.Ru / Tencent logins; only the Trion ticket.
* Server → client packets not decoded (skipped): mail, houses in the character list
  (`SCLoginCharInfoHouse`, so `GetLoginCharacterInfoHouse`), premium grade of the lobby, guilds,
  trading, loot dice (`SCLootDice*`, rolls for group loot), the mount owner's packets
  (`SCMateSpawned`, `SCMateState`, `SCSlaveCreated` ...; the units themselves and riding are decoded, see
  "Other units' motion"), `SCBuffUpdated` (the lane server never sends it and has no writer for it, so its
  layout is unknown; a buff's time is refreshed by `SCBuffCreated` with the same index). Add a
  variant to `game::ServerMsg` + `Event` for each as the UI needs it. A packet in a `DD04` batch
  that is not decoded ends that batch.
* Client → server: skills, items, target, action bar, resurrect, chat, logout, NPC talk and quests,
  shops (buy/sell/buyback), bank money/expansion, item moves (swap, split, destroy). Trade, mail,
  coin/doodad shops, priest buffs and the other NPC functions still need their own requests.
* No maximum HP / MP on the wire: `PlayerStats::max_hp` is the highest value seen (a character that
  spawns dead or hurt shows too small a maximum until it has been at full health once) unless the
  application sets the formula value with `PlayerStats::set_max` (the HUD does).
* Quests: `SCQuestContextFailed`, `SCQuestUnitReqFailed` and the quest error notices are not decoded
  (the lane server never sends them). The objective texts are not on the wire: the HUD makes them from
  the game database.
* The sub zone name of the minimap header is not sent: derive it from the position and the
  `sub_zones` table. The zone group follows the zone chat channel (Shout).
* `DD04` batches with real Huffman compression (the C# server) are skipped; the Rust server writes
  stored blocks.
* NPC names are empty on the wire (the client resolves them from the NPC template); `Unit.kind` has the
  template id. Slaves / transfers / shipyards only decode their core fields.
* Movement: we only write a plain unit movement (ground, jump, fall with landing velocity); vehicles, ships,
  standing on / hanging from objects are read (`Movement::read_typed`) but not written. The server does not
  echo our own movement back.
* Reconnect after a dropped game connection is not attempted (the session reports `Disconnected` and
  returns to idle).
