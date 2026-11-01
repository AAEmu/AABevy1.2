# Combat recorder v2 (original 1.2 client)

`record_original.py` attaches to an **already running, hidden** ArcheAge 1.2
client and records the client's own combat state into one JSONL file per
channel.  It is the T1 recorder for the combat goal: client-time stamps, the
cooldown-manager GCD pair, the cast bar, caster/victim animation lists,
`UnitBuff`/`UnitDeBuff`, `client_unit_state` regen samples and the chat /
system-message log.

The client is started by the lane (`Scripts/live-stack.sh client`), not by this
tool.  The recorder hides it, verifies `hidden=1` in `client_status`, and stops
it by pid at the end unless `--keep-client` is passed.  It never sends desktop
input and never calls `X2World:LeaveWorld`.

## Run

```bash
python tools/combat-side/record_original.py \
  --pid <client-pid> \
  --run-dir D:/aaemu-client/handoff/2026-11-01-combat-recorder/run \
  --driver D:/aaemu-target/combatcap/live/aaemu-clientdriver.exe \
  --dll D:/aaemu-target/combatcap/aadriver/aadriver.dll \
  --launcher D:/aaemu-target/combatcap/live/aaemu-launcher.exe \
  --client D:/AA1.2/bin32/archeage.exe \
  --login 127.0.0.1:35237 \
  --password-file D:/aaemu-live/cfg/client-test-password.txt \
  --character CombatT1 --character-ability 2 --create-character \
  --spawn-npc 6409 --npc-name "rat_brown " \
  --fight-keys 1,1,2,1,3,2,1,1 --server-log D:/aaemu-live-combatcap/logs/rs-game.log
```

`--character-ability 2` selects the ability whose level-1 starter
(`14376`, Earth's Grasp) has `default_gcd='t'`, so the first key press produces
a real global cooldown.  Ability 1's starter (`18132`) has `default_gcd='f'`
and would leave the GCD pair at zero.

The account must have GM before the client logs in (`access_level = 100` in
the lane's game database `accounts` row for the account id).  The optional
`prepare_account.py` helper fills that row and sets the launcher password.

Exit status: `0` when the self-check passes, `2` when a required channel is
empty or the GCD pair never changes, `1` on a setup failure.

## Channels

Every file is `<channel>.jsonl` in `--run-dir`; each line is one JSON object.
`server_slice.log` (raw game-server log from the start offset) and
`self_check.json` are written beside them.

| channel | contents |
|---|---|
| `meta` | one start row and one end row (character, ids, counts, self-check result) |
| `clock_sync` | once per second: `System.GetCurrTime` and `X2Time:GetUiMsec` beside `wall_ms` |
| `anim_state` | every 16 ms: caster/victim `GetCurrentAnimationState`, `GetAnimationTime`, actor pointer, world position (system state) |
| `anim_controller` | optional (`--controller-lists`): the old caster/victim scheduler lists A/B; off by default |
| `gcd` | every 5 ms: cooldown-manager global pair and the unit combat fields |
| `castbar` | every 50 ms: `X2Unit:UnitCastingInfo` for `player` and `target` plus HP/MP/combat state |
| `buffs` | once per second: one `list`/`count` row and one row per `UnitBuff`/`UnitDeBuff` entry for player and target |
| `unit_state` | once per second: the driver's `client_unit_state` for `player` and `target` (name, id, distance, HP/MP, buffs) |
| `chat` | every client chat/system dispatch, with a `likely_refusal` flag |
| `inputs` | every driver key press, with client clock before/after and the driver reply |

### Common row fields

```json
{"schema":"aabevy.combat-recorder/v2","channel":"gcd","seq":123,
 "wall_ms":1790000000000.0,"client_ms":53721,"t_ms":1240,"clock":"estimate",
 ...channel fields...}
```

* `wall_ms` is the recorder's `time.time()` clock.
* `client_ms` is the unified client clock: the UI clock (`X2Time:GetUiMsec`)
  for UI rows, the system clock (`System.GetCurrTime()*1000`) mapped through
  the last sync for system rows, and the last synced offset for memory and
  driver-template rows.  The clock used for the row is in `clock`
  (`ui`, `system` or `estimate`).
* `native_ms` is the raw value of that row's own clock when it has one (UI or
  system); it is absent for estimated rows.
* `t_ms` is `client_ms` minus the first synced client time; it is the
  cross-channel capture time.

### `gcd` row fields

`gcd_remaining`, `gcd_total`, `cast_normal`, `cast_delay`, `cast_channel`,
`cast_channel_delay` (the four cast slots at `unit+0x3ac4`), `stance_perm`,
`combat_deadline`, `combat_in`, `latch_a`, `latch_b`, `weapon_active`,
`weapon_slots`, `visual_byte`, `player_unit`, `caster_ctrl`, `victim_ctrl`.

## The GCD pair

The manager is inline in the unit at `+0x2c48`; the global pair is at `+0x08`
(remaining ms) and `+0x0c` (total ms).  The evidence, from the 1.2 decompile
`D:/aa-re/client12/out-x2game/x2game.dll.c`:

* `x2game.dll.c:1931135` `FUN_3989a290` — when its mode byte is 0 it writes the
  caller's two out-pointers from `*(uint *)(param_1 + 8)` and
  `*(uint *)(param_1 + 0xc)` (lines 1931141-1931150), then scans the skill and
  tag lists for a larger remaining value.
* `x2game.dll.c:829989` `FUN_393f61c0` — the four-value export:
  `FUN_3989a3e0(manager, out1, out2)` (mode read from `manager+0xdc`) and
  `FUN_3989a290(manager, 1, out3, out4)` (lines 830008-830025).
* `x2game.dll.c:886364` `FUN_3943f420` — the UI-facing wrapper that calls
  `FUN_393f61c0(param_2, &local_10, &local_14, &local_8, &local_c)` and passes
  the four values to `FUN_3943ecb0` (lines 886370-886379).
* `x2game.dll.c:1930785` `FUN_39899d00` — clears `+8` and `+0xc`;
  `x2game.dll.c:1930855` `FUN_39899df0` — writes both; `x2game.dll.c:1930880`
  `FUN_39899e50` — subtracts elapsed ms from `+8` with a zero floor.  These
  prove `+8` is remaining and `+0xc` is total.

`docs/research/port-combat/structs.md` §3 (`CooldownManager`) records the same
layout and the owning unit offset.

## Animation lists

`anim_state` has a row per system Lua tick with the actor state string for both
the caster and the victim (``GetCurrentAnimationState``), their world positions
and the actor handle.  The clip time field is empty when the client's actor
table does not expose a time getter; this build has no ``GetAnimationTime``
method, so the recorder does not invent one.

The old recorder's controller scheduler lists are **optional** and off by
default (``--controller-lists``).  They need the PlayAnim pass-through hook,
which uses the driver's ``alloc``/``write``/``call`` memory commands; the lane's
standard ``aadriver.dll`` does not implement them.  Without the hook the
unit/model pointer chain did not expose the old ``ctrl+0x24``/``ctrl+0x30``
lists: the candidate that looked plausible produced entry words that were not
an animation entry, so the default recorder does not write that channel.  The
lane dry run therefore reads the caster/victim animation list through
``anim_state`` and reports the scheduler channel as not read.

## Self-check

`self_check.json` reports:

* `channels`: rows per channel;
* `missing`: required channels with zero rows (default:
  `clock_sync,anim_state,gcd,castbar,unit_state,chat,inputs`);
* `gcd.changed`: true when a recorded key press is followed within 3 s by a
  change of the cooldown pair;
* `gcd.pressed_changes`: the key/change pairs that prove a press started the
  GCD;
* `animations.controller_lists`: whether the opt-in scheduler lists were
  sampled; `caster_ctrl` / `victim_ctrl` and `tried` name the resolved
  addresses when they were.

The default fight plan presses the first slot twice (`1,1`) before the other
keys, so a cast/GCD refusal (if any) is also captured in `chat`.  A fast plan
(`--fight-keys 1,1,1,2,1,3,1,2 --fight-interval 0.45`) presses the GCD skill
during its cast and during its cooldown to provoke the refusal text.

## Limits

* The client clock was read from `X2Time:GetUiMsec` (UI) and
  `System.GetCurrTime` (system).  If a future client build does not expose
  them, the affected rows fall back to `clock="estimate"`.
* Damage, death and server replies in `server_slice.log` are the lane server's
  behaviour; they are not 1.2 client evidence.  The recorder's channels only
  contain what the client itself exposes.
* `unit_state` uses the driver's `client_unit_state`; its `unit_id` is the
  native opaque id (`X2Unit:GetUnitId`), not the engine entity id.
* The optional controller lists are not part of the default dry run: the lane
  `aadriver.dll` has no `alloc`/`write`/`call` commands for the PlayAnim hook,
  and the unit/model pointer chain did not expose the old scheduler lists.  The
  dry run reads the caster and victim animation list through `anim_state` and
  reports the scheduler list as not read.
