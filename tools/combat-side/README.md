# Combat recorder v2 (original 1.2 client)

`record_original.py` attaches to an **already running, hidden** ArcheAge 1.2
client and records the client's own combat state into one JSONL file per
channel.  It is the T1 recorder for the combat goal: client-time stamps, the
cooldown-manager GCD pair, the cast bar, caster/victim animation state,
`UnitBuff`/`UnitDeBuff`, `client_unit_state` regen samples and the chat /
system-message log.

The client clocks are sampled inside a recorder bracket taken under the MCP
lock (``wall_ms`` before and after the request); the recorder fits the offset
per clock over the whole run and remaps every row through that final fit.  The
slope is held at 1 unless the samples span more than 60 s (the unconstrained
slope and its standard error are still reported).  Memory rows are on a
different clock edge than the Lua rows: the client answers Lua only on a frame
and returns that frame's start time, so the recorder measures the memory-to-UI
offset from matched GCD rises and cast starts in each run and shifts the memory
rows (`gcd`, `anim_controller`) by it.  That shift **assumes the GCD is written
in the same frame the cast starts**; the measured offset therefore mixes the
UI's frame-start stamping (about half a 52 ms frame), the 5 ms memory cadence
and any real 1.2 ordering.  Each shifted row keeps its unshifted time in
`client_ms_raw`, and the post-shift GCD-rise-vs-cast difference must not be
read as 1.2 evidence.  Each rise and each cast start is linked to the input
press just before it (within 500 ms); an unpaired rise is an ordering mismatch
only when its own press also produced an unpaired cast start.  A start from
another press, or one with no press, never counts, so an instant pressed near
an interrupted cast stays exempt.  The offset is confirmed only with two pairs.
Wall-stamped rows (`unit_state`, `meta`) are never shifted.  A read that fails (`ReadProcessMemory` error) is retried for 5 s
with marked ``gap`` rows; if the actor never returns, a ``read_failed`` row
stops the sampler with an error instead of silently producing zeros.  The Lua
buffers are drained every 5 s, so a disconnect loses at most a few seconds; the
chat hooks and ``_G.CS`` timers are removed at the end in both Lua states, also
with ``--keep-client``.

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
empty, the GCD never completes a press-linked rise, the memory-to-UI check
fails (an unpaired rise whose own press produced an unpaired cast start, i.e.
an ordering mismatch, or a residual above 25 ms after the median), or a channel's
`client_ms` runs backwards, `1` on a setup failure (including a failed memory
read or an interrupt).  A `KeyboardInterrupt` is recorded in `self_check.json` as a
failure, cleanup runs, and the interrupt is re-raised.
`clock.*.drift_warning` (an applied drift beyond 200 ppm) is reported but does
not gate the self-check; see the `clock` bullet under Self-check.

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
| `actions` | optional (`--action-slots 1-10`): per client frame, `X2Action` `GetActionCooldownEx` / `GetActionCooldown`, type, repeat/current/empty flags for each named slot |
| `sounds` | optional (`--hook-ui-sounds`): every `X2Sound.PlayUISound` / `PlaySound` call with its client time |
| `steps` | optional (`--script`): one row per scenario op with the client-time bracket and its result |

## Scenario scripts

`--script <json>` replaces `--fight-keys` with a list of ops, so a capture does
not need a new Python branch for each scenario.  Supported ops:

* `mark` (name), `sleep` (ms), `key` (key, duration_ms), `chat` (message,
  settle_ms), `lua` (target, code), `snapshot` (target, code);
* `spawn` (template, settle_ms): `/despawn npc <template> 30` then `/spawn`;
* `findtarget` (template, name, approach): `/findobject` + `client_target`,
  optionally walking `approach_ms` towards the target when it is farther than
  `approach_if` metres;
* `clear_target` (`X2Unit:TargetUnit('')`), `self_target`;
* `set_slots` (page, map): `X2Action:ClearAction` + `SetActionSpell` and a
  per-slot readback of type/name/ok;
* `buff` (buff, level, self): GM `/buff astarget`;
* `wait_alive`, `wait_combat` (timeout, poll_ms): poll `UnitIsDead` /
  `UnitCombatState('player')`;
* `wait_target_dead` (timeout, poll_ms): poll `client_unit_state` and return
  when the target is absent or has 0 HP.

Every op writes a `steps` row with `client_ms`, `wall_ms`, `result` and
`error`; an optional op (`"optional": true`) records the error and continues.
The committed K2/K3 scripts are in `tools/combat-side/scenarios/`.

### Common row fields

```json
{"schema":"aabevy.combat-recorder/v2","channel":"gcd","seq":123,
 "wall_ms":1790000000000.0,"client_ms":53721,"t_ms":1240,"clock":"estimate",
 ...channel fields...}
```

* `wall_ms` is the recorder's monotonic `perf_counter` clock in milliseconds
  (not epoch time); it never jumps with the system clock.
* `client_ms` is the unified client clock: the UI clock (`X2Time:GetUiMsec`)
  for UI rows, the system clock (`System.GetCurrTime()*1000`) mapped through
  the fitted UI/system clock pair for system rows, and the fitted clock model
  for memory and driver-template rows.  The clock used for the row is in
  `clock` (`ui`, `system` or `estimate`).  After the run every row is rewritten
  with the final fit; the memory rows (`gcd`, `anim_controller`) are then
  shifted by the measured memory-to-UI offset (27 ms in run5) so they align
  with the UI cast clock.  `self_check.json`'s `memory_ui` block reports that
  offset, its spread and the residual, together with the same-frame assumption;
  the clock residuals are fit quality only.
* `client_ms_raw` is the unshifted memory time on `gcd` and `anim_controller`
  rows that the shift moved; `client_ms` for those rows is the shifted value.
  The shift never overwrites an existing `client_ms_raw`, so running it twice
  is safe, and an offset of 0 restores the raw time.  `t_ms` follows the
  shifted `client_ms`.
* `native_ms` is the raw value of that row's own clock when it has one (UI or
  system); it is absent for estimated rows.
* `t_ms` is `client_ms` minus the fitted client time at the first clock
  sample; it is the cross-channel capture time and follows the shifted
  `client_ms` on memory rows (subtract `start_client_ms` from `client_ms_raw`
  for the unshifted relative time).

### `gcd` row fields

`gcd_remaining`, `gcd_total`, `cast_normal`, `cast_delay`, `cast_channel`,
`cast_channel_delay` (the four cast slots at `unit+0x3ac4`), `stance_perm`,
`combat_deadline`, `combat_in`, `latch_a`, `latch_b`, `weapon_active`,
`weapon_slots`, `visual_byte`, `player_unit`, `caster_ctrl`, `victim_ctrl`.
A failed read row instead carries `read_failed` and `error`; a row written while
the actor is unavailable carries `gap` and `reason` and has no GCD fields.

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

`<re-notes>/port-combat/structs.md` §3 (`CooldownManager`) records the same
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
* `gcd.changed`: true when a recorded key press is followed within 500 ms by a
  rise of `gcd_remaining` that reaches about `gcd_total` and then falls to 0
  within `gcd_total` plus 250 ms; countdown-only changes, short rises, slow
  falls and rising counters do not count (`gcd.countdown_ticks_rejected`,
  `gcd.incomplete_rises` and `gcd.short_or_slow_rises` count the rejects);
  each press starts at most one rise, so two rises cannot both claim it;
* `gcd.changed` also requires most complete rises to follow a key press
  (`linked_complete_rises` must be more than half of `complete_rises`; a tie is
  not a majority);
* `gcd.pressed_changes`: the press-linked complete runs with their start/end,
  peak, total and duration;
* `quality.monotonic`: per channel, whether `client_ms` never runs backwards
  (count, largest backstep and the first offender); a backstep fails the check;
* `quality.rates`: per channel, the measured step between rows (median, p95 and
  max wall delta), not the requested timer interval;
* `memory_ui`: the memory-to-UI offset (median of GCD-rise minus matched
  cast-start), its spread/stdev, the pairs and the residual after the median.
  It is `measured`/`measurable` only with at least one matched pair; with no
  pairs it says so (`reason`: `no-cast-starts`, `no-gcd-rises`,
  `no-gcd-rises-or-cast-starts` or `no-pairs`) and, if no rise is an ordering
  mismatch, does not fail the run.  The offset is `confirmed` only with two or
  more pairs; a single pair is reported as `status=low-confidence`,
  `low_confidence=true` and logged, but it does not gate the self-check (the
  offset is still the best estimate and the report says it is unconfirmed).
  Both a rise and a cast start are linked to the input press just before them
  (within 500 ms, the same link the GCD check uses).  An unpaired rise is an
  ordering mismatch only when that same press also produced an unpaired cast
  start within `near_window_ms` (1000 ms); the start is then listed in
  `unpaired_near_start` with its delta and the rise's key, and
  `ordering_mismatch` fails the run (e.g. a GCD written 300 ms after its cast
  start).  Two rises cannot claim one start, and a start from another press or
  one with no press never counts, so an instant pressed near an interrupted
  cast stays exempt.  Its `assumption` is the same-frame GCD rule, and
  `raw_field` names `client_ms_raw`.  Do not use the post-shift difference as
  1.2 ordering
  evidence: the shift makes it zero by construction, and only the spread and
  the residual maximum (25 ms gate) carry information about the pairs;
* `clock`: per client clock, the fitted offset, the applied drift (0 unless the
  span exceeds 60 s), the unconstrained fitted drift and its standard error,
  the sample count and round trips, and the residual min/median/max/RMS.  The
  residual describes how well the wall-to-client line reproduces the clock
  reads, not the cross-channel accuracy.  `drift_warning` marks an applied
  drift beyond 200 ppm; `drift_warning_policy` says it is reported but does not
  gate the self-check, because the UI clock is stamped at frame start and a
  long run can fit such a slope by chance;
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

The memory-to-UI alignment measured on run5: offset **27 ms**, spread 6 ms
(stdev 2.5 ms) over 3 matched Flamebolt casts (confirmed), residual maximum
4 ms.  That 27 ms is the same-frame shift: it contains the UI frame-start
stamping (about half of the 52 ms frame), the 5 ms memory cadence and any real
1.2 ordering, so it is not evidence of when the client writes the GCD.  After
the shift the GCD rises coincide with their cast starts by construction; only
the spread and the residual describe the pair jitter.  The unshifted time is in
`client_ms_raw`, and the recomputed run is in `run5-recomputed/`; the raw run is
untouched.

## Tests

```bash
python -m unittest discover -s tools/combat-side -p "test_*.py" -v
```

The offline tests cover the drift gate and warning policy, the completed-rise
GCD rule (including an incrementing-counter negative case, the "most rises
follow a press" majority and one press starting at most one rise), the
memory-to-UI measurement and shift (an instant near an unrelated start, two
unpaired rises claiming one start once, same-press outliers at +300 ms and
-100 ms, an outlier pair, a single low-confidence pair, idempotent and
offset-0 shifting, and the shift touching only the memory channels),
stale-reply skipping, the KeyboardInterrupt self-check, sampler gap recovery
and a null actor that must enter recovery instead of writing stale rows.

## Evidence classes (K2/K3 summary)

`tools/evidence/combat_core/native.json` labels every K2/K3 value with one of these classes:

* **client** - the 1.2 client's own send, memory/UI byte or animation, with no server reply or scripted step in
  the causal path (the GCD pair, the `CSStartSkill 2` send, the stance byte).
* **server-dependent** - the value includes our server's reply (`SCSkillStarted`, `SCSkillFired`,
  `SCCombatCleared`, the auto-attack repeat timer, the range check) or a scripted step (the swing cadence, the
  cast-bar total, the target loss).
* **inferred** - not separable from our server's reply at localhost, or a one-sample ordering: stated as
  inferred, never as a 1.2 rule (the GCD exemption, the queue window, the refusal order).

The stance rule is decompile-proven (client): `FUN_391d6a60` (`OnCombatCleared`) clears the `+0x3bba` latch
through vtable `+0x48` = `FUN_39355ec0`, and `FUN_39355ee0` refreshes a 10 s deadline while the latch is set
(`<re-notes>/port-combat/side-by-side.md:52-68`). The captured stance drop is timed by our server's
`SCCombatCleared`, not by a client timer.

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
* A zone change or respawn longer than the 5 s recovery grace still stops the
  sampler.  The victim pointer is refreshed by its object id only; a respawn
  that gives it a new id is not re-targeted.
* The memory-to-UI offset is measurable only when at least one GCD rise pairs
  with a cast-bar start within the window.  Instant skills without a cast bar
  still count for the GCD check but cannot contribute a cross-channel pair; a
  run with no casts (or with starts and rises that never pair) leaves
  `memory_ui.measurable=false` with a `reason`.  It still fails when an
  unpaired rise's own input press also produced an unpaired cast start within
  1000 ms (`ordering_mismatch`), because that is exactly the ordering
  difference the tight window would hide; a rise from another press, or one
  with no press, cannot be an ordering mismatch.  A true instant pressed near
  an interrupted cast is therefore exempt.  One pair is only `low-confidence`,
  not `confirmed`; it does not fail the run but does not prove the offset
  either.  The measured 27 ms shift absorbs frame stamping and cadence, so the
  post-shift difference is not 1.2 ordering evidence; use `client_ms_raw`.
* Drift is not applied on runs shorter than 60 s; the slope reported there is
  the unconstrained estimate with its standard error, not a correction.
* Gap rows are written while the actor is unavailable, so the `gcd` channel can
  contain rows with `gap=true` and no GCD fields; they are skipped by the GCD
  checks and included in the monotonic/rate reports.
