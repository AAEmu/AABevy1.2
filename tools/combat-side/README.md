# Combat recorder v2 (original 1.2 client)

`record_original.py` attaches to an **already running, hidden** ArcheAge 1.2
client and records the client's own combat state into one JSONL file per
channel.  It is the T1 recorder for the combat goal: client-time stamps, the
cooldown-manager GCD pair, the cast bar, caster/victim animation state,
`UnitBuff`/`UnitDeBuff`, `client_unit_state` regen samples and the chat /
system-message log.

The client clocks are sampled inside a recorder bracket (``wall_ms`` before and
after the read); the recorder fits an offset and drift line per clock over the
whole run and remaps every row through that final fit.  A read that fails
(`ReadProcessMemory` error) is written as a marked ``read_failed`` row and stops
the sampler with an error instead of silently producing zeros.  The Lua buffers
are drained every 5 s, so a disconnect loses at most a few seconds; the chat
hooks and ``_G.CS`` timers are removed at the end in both Lua states, also with
``--keep-client``.

The client is started by the lane (`Scripts/live-stack.sh client`), not by this
tool.  The recorder hides it, verifies `hidden=1` in `client_status`, and stops
it by pid at the end unless `--keep-client` is passed.  It never sends desktop
input and never calls `X2World:LeaveWorld`.

## Run

```bash
python tools/combat-side/record_original.py \
  --pid <client-pid> \
  --run-dir <capture-root>/2026-11-01-combat-recorder/run \
  --driver <lane-target>/live/aaemu-clientdriver.exe \
  --dll <lane-target>/aadriver/aadriver.dll \
  --launcher <lane-target>/live/aaemu-launcher.exe \
  --client <client>/bin32/archeage.exe \
  --login <login-host:port> \
  --password-file <password-file> \
  --character CombatT1 --character-ability 2 --create-character \
  --spawn-npc 6409 --npc-name "rat_brown " \
  --fight-keys 1,1,2,1,3,2,1,1 --server-log <lane-live>/logs/rs-game.log
```

`--character-ability` is the index passed to the creation UI's
`SelectAbility`; it is not the database ability id.  In the 1.2 dry run index 2
produced ability 7 and its level-1 starter `10752` (Flamebolt,
`default_gcd='t'`), so the first key press produced a 1000 ms global cooldown.
Ability 1's starter `18132` has `default_gcd='f'` and would leave the GCD pair
at zero.  Check `self_check.json`'s `gcd` block if you change the choice.

The account must have GM before the client logs in (`access_level = 100` in
the lane's game database `accounts` row for the account id).  The optional
`prepare_account.py` helper fills that row and sets the launcher password.

The recorder refuses a target farther than `--target-range` metres (default
4.0) unless `--move` is given; raise the range only when the selected skill's
own range is the point of the capture.  `--keep-client` leaves the client
running (the hooks and timers are still removed) so a failed run can be retried
without starting the client again; stop it by pid afterwards.

Exit status: `0` when the self-check passes, `2` when a required channel is
empty, the GCD pair never rises after a press, or a channel's `client_ms` runs
backwards, `1` on a setup failure (including a failed memory read).

## Channels

Every file is `<channel>.jsonl` in `--run-dir`; each line is one JSON object.
`server_slice.log` (raw game-server log from the start offset) and
`self_check.json` are written beside them.

| channel | contents |
|---|---|
| `meta` | one start row and one end row (character, ids, counts, self-check result) |
| `clock_sync` | once per second: `System.GetCurrTime` and `X2Time:GetUiMsec`, each with its bracket round trip |
| `anim_state` | per client frame: caster/victim `GetCurrentAnimationState`, `GetAnimationTime`, actor pointer, world position, seamless origin |
| `anim_controller` | optional (`--controller-lists`): the old caster/victim scheduler lists A/B; off by default |
| `gcd` | every 5 ms (busy-wait): cooldown-manager global pair and the unit combat fields |
| `castbar` | per client frame: `X2Unit:UnitCastingInfo` for `player` and `target`, HP/MP/combat state, seamless origin |
| `buffs` | once per second: one `list`/`count` row and one row per `UnitBuff`/`UnitDeBuff` entry for player and target |
| `unit_state` | once per second (player and target, alternating): the driver's `client_unit_state` (name, id, distance, HP/MP, buffs) |
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
  the fitted UI/system clock pair for system rows, and the fitted clock model
  for memory and driver-template rows.  The clock used for the row is in
  `clock` (`ui`, `system` or `estimate`).  After the run every row is rewritten
  with the final fit, so `client_ms` is monotonic per channel and comparable
  across channels.
* `native_ms` is the raw value of that row's own clock when it has one (UI or
  system); it is absent for estimated rows.
* `t_ms` is `client_ms` minus the fitted client time at the first clock
  sample; it is the cross-channel capture time.

### `gcd` row fields

`gcd_remaining`, `gcd_total`, `cast_normal`, `cast_delay`, `cast_channel`,
`cast_channel_delay` (the four cast slots at `unit+0x3ac4`), `stance_perm`,
`combat_deadline`, `combat_in`, `latch_a`, `latch_b`, `weapon_active`,
`weapon_slots`, `visual_byte`, `player_unit`, `caster_ctrl`, `victim_ctrl`.
A failed read row instead carries `read_failed` and `error`.

`anim_state` and `castbar` carry `seamless_origin`; the client's positions are
seamless-local, so global = local + origin (the origin sampled in the UI camera
state; `anim_state` uses the cast-bar sample nearest its mapped UI time).

## The GCD pair

The manager is inline in the unit at `+0x2c48`; the global pair is at `+0x08`
(remaining ms) and `+0x0c` (total ms).  The evidence, from the 1.2 decompile
`<re-notes>/client12/out-x2game/x2game.dll.c`:

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

`anim_state` has a row per system Lua tick (one per client frame) with the actor state string for both
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
  **rise** of `gcd_remaining` or a change of `gcd_total`; countdown-only changes
  do not count (`gcd.countdown_ticks_rejected` counts those);
* `gcd.pressed_changes`: the key/change pairs that prove a press started the
  GCD;
* `quality.monotonic`: per channel, whether `client_ms` never runs backwards
  (count, largest backstep and the first offender); a backstep fails the check;
* `quality.rates`: per channel, the measured step between rows (median, p95 and
  max wall delta), not the requested timer interval;
* `clock`: per client clock, the fitted offset and drift, the sample count and
  round trips, the residual min/median/max/RMS and the residual of the
  best-round-trip sample;
* `animations.controller_lists`: whether the opt-in scheduler lists were
  sampled; `caster_ctrl` / `victim_ctrl` and `tried` name the resolved
  addresses when they were.

The default fight plan presses the first slot twice (`1,1`) before the other
keys, so a cast/GCD refusal (if any) is also captured in `chat`.  A fast plan
(`--fight-keys 1,1,1,2,1,3,1,2 --fight-interval 0.45`) presses the GCD skill
during its cast and during its cooldown to provoke the refusal text.

## Measured rates (run5, 2026-11-01)

The recorder requests 5 ms for `gcd` and "every frame" for `anim_state` and
`castbar`; the measured rates from the dry run are:

| channel | requested | measured median step |
|---|---|---|
| `gcd` | 5 ms | 5.0 ms (p95 5.6 ms, max 27 ms) |
| `anim_state` | every frame | 52.1 ms (p95 54 ms) |
| `castbar` | every frame | 52.0 ms (p95 56 ms) |
| `buffs` | 1000 ms | 1043.5 ms |
| `unit_state` | 1000 ms | 102 ms between alternating rows (about 1.16 s per player+target pair) |
| `clock_sync` | 1000 ms | 1126 ms (the MCP calls share a serialised driver) |

The Lua channels cannot beat the client's own frame rate: `Script.SetTimer`
re-arms on the next frame, and this hidden client renders at about 19 fps
(52 ms).  A 16 ms or 50 ms request therefore still fires once per frame.  The
Python memory sampler uses `timeBeginPeriod(1)` and a `perf_counter` busy-wait
for its 5 ms cadence; before that it inherited the 15.6 ms Windows timer.

## Limits

* The client clock was read from `X2Time:GetUiMsec` (UI) and
  `System.GetCurrTime` (system).  If a future client build does not expose
  them, the affected rows fall back to `clock="estimate"`.
* The UI clock and `X2Util` only exist in the UI Lua state; `anim_state` is
  sampled in the system state, so its `seamless_origin` comes from the UI
  cast-bar sample nearest the row's mapped UI time.
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
