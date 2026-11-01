# Parity harness: scenario format and original-client runner (step 1)

One scenario file drives a capture on the original 1.2 client.  A later step
adds the Bevy runner and a differ on the same format.  This directory currently
contains the format, the original-client runner, the packet-clock alignment and
offline tests.

```
tools/parity/
  scenario.py              YAML-subset/JSON parser and validator
  run_original.py          the original-client runner
  packet_clock.py          server log -> client-clock packet rows
  scenarios/wagon_bar.yaml
  scenarios/auto_attack.yaml
  tests/                   offline tests and MCP fixtures
```

## Scenario format

YAML (a small subset: mappings, lists, quoted strings, numbers, booleans,
`[a, b, c]`, comments) or JSON.  Top level:

```yaml
name: auto_attack            # required, file-safe, also the capture folder
version: 1                   # only 1 is accepted
description: ...             # optional
tags: [combat]               # optional
defaults: {}                 # reserved for the later compiler
preconditions: []            # optional steps, run first, same step shape
steps: []                    # required, non-empty
```

Every precondition and step has a unique `id` (letters, digits, `_`, `.`, `-`),
exactly one verb, and optional `note`, `optional` (record the error and
continue), and `server_tag` (`client`, `server` or `script`).  A failed step
without `optional: true` aborts the run; the row is still written.

### Verbs

| Verb | Value | What the original runner does |
|---|---|---|
| `gm` | command string | `client_chat` (a Say message or GM command) |
| `wait` | milliseconds or a condition | sleep, or poll a condition (below) |
| `key` | key name, `hold_ms` | `client_input`; keys `w a s d q e r t y u i o p space f g 1..0` |
| `click-ui` | widget Lua path | resolve `GetEffectiveOffset`/`Extent`, `client_pointer click` at the centre |
| `target` | name/template mapping | `/findobject npc <template>` + `client_target`, or a right-click screen pick by entity name |
| `use-skill` | id / slot / mapping | scan the action bar for the id, press its slot key; `bar: mode` uses `client_mode_input` or a click on the mode-bar button |
| `interact` | `true` or mapping | press `f` (or `client_action` with a named action) |
| `move-to` | `[x, y, z]` or `"x y z"` | GM `/move` |
| `mount` | `true` or `{kind: pet\|vehicle}` | `X2Mate:MountMate()`, or `f` + wait for `vehicle.mounted` |
| `dismount` | `true` | `client_vehicle action=discard` or `X2Mate:UnMountMate()` |
| `snapshot` | label | screenshot + UI dump + entity/unit/vehicle/mode-action state |
| `mark` | label | a marker row in `steps.jsonl` |
| `lua` | `{target, code}` | arbitrary Lua on `ui`, `system` or `third` (escape hatch) |
| `set-slots` | `{page, slots}` | `X2Action` page + `SetActionSpell` setup |

All verbs map to the Bevy script runner's existing primitives later; the
compiler is a separate step.  `LeaveWorld`, `System.Quit` and similar are
rejected by the validator: the runner only stops the client by pid.

### Conditions

`wait` either sleeps or waits for client state:

```yaml
- id: wait_mounted
  wait:
    until: "vehicle.mounted == true"
    timeout_ms: 8000
    poll_ms: 250
    source: server
```

`source` is required: `client`, `server` or `script`.  The runner records it as
`condition_source` (and as `server_tag` when none is given) so a report can
tell a server-driven timing from a client rule or a scripted step.  Paths come
from the state read for each poll: `player.*`, `target.*`, `vehicle.*` and
`chat.last`.  Operators are `== != < <= > >= contains`; literals are numbers,
booleans, `null` or strings.

## Capture layout

```
<capture-root>/<scenario>/<run-id>/
  steps.jsonl
  ui/<step>.json
  entities/<step>.json
  shots/<step>.png
  client_packets.jsonl
  clock_sync.jsonl
  meta.json
```

* `steps.jsonl`: one row per step with `id`, `verb`, `client_ms`, `t_ms`,
  `phase` (`precondition` or `step`), `server_tag`, `result`, `error` and the
  files a snapshot wrote.
* `ui/<step>.json`: `reference/tools/snapshot.lua` output (the existing
  `reference/**/ui.json` format).
* `entities/<step>.json`: `client_entities` (up to 200, client-local), the
  player/target `client_unit_state`, `X2Mate`/actor animation state,
  `client_vehicle` and `client_mode_actions`.  Animation state is best effort:
  the original build exposes the actor state string, not always a clip time.
* `shots/<step>.png`: the rendered frame.  Local only; never commit PNGs from
  the original client (`reference/README.md` and AGENTS.md).
* `client_packets.jsonl`: the client's sent packets from the server field trace.
* `clock_sync.jsonl`: the bracketed clock samples behind the fit.
* `meta.json`: scenario hash, client pid/status, lane, server commit, packet
  alignment summary and the `reply_delay` hint.

Snapshots are named by step id.  A `snapshot` step writes all three files; a
`mark` step only writes a marker row.

## Packet clock

The server log timestamp has one-second resolution, and the recorder's read time
is not a send time (evidence audit F1).  The client's own clock is inside
`CSMoveUnit` bytes 4-7 and `Ping` bytes 16-19 (`Ping.local`, echoed in
`Pong.local`).  Those are the two fields the audit established; the trace also
shows a clock-looking value in FastPing's 4-byte body, but it is not trusted
here.  `packet_clock.py`:

* reports `client_ms` exactly for those packets (`clock: packet`);
* interpolates a packet without a clock between the nearest anchors in log
  order (`clock: interpolated`) and records the anchor gap;
* uses the nearest anchor when only one side exists (`nearest_anchor`);
* maps the client system clock to the UI clock through the run's clock fit and
  writes `ui_ms`/`t_ms` (the same time base as `steps.jsonl`).

Nothing without an embedded clock is labelled an exact send time.  The K2
capture (`handoff/2026-11-01-combat-k2k3/run`) is the regression: the aligned
rows reproduce the audit's corrected K2 times (target clear ~58.5 s, re-press
~68.6 s, out-of-range press ~117.5 s; `SCCombatCleared` ~59.3 s).

## Running on the original client

Prerequisites, per the lane rules:

1. Start the lane's server with the field trace, for example
   `RUST_LOG=info,fields=trace,connection=debug Scripts/live-stack.sh start`
   (without `--mysql` for stop; never touch port 3306).
2. The account needs GM (`access_level = 100`) for the scenario setup.
3. The lane's database and configuration already have the character/skills the
   scenario expects.  `wagon_bar` needs `/slave spawn 60` and `auto_attack`
   needs skill 2 on action slot 3 (the K2 setup).

Run one scenario:

```bash
python tools/parity/run_original.py tools/parity/scenarios/auto_attack.yaml \
  --lane combatcap --capture-root <capture-root> \
  --character Combatt1 --create-character \
  --driver <lane-target>/live/aaemu-clientdriver.exe \
  --dll <lane-target>/aadriver/aadriver.dll \
  --launcher <lane-target>/live/aaemu-launcher.exe \
  --client <client>/bin32/archeage.exe \
  --login 127.0.0.1:35237 --password-file <password-file> \
  --reply-delay 600
```

`--lane` starts the client through that lane's `Scripts/live-stack.sh client`
once; a second start is attempted only when the first looks like a startup
crash.  The runner hides the client immediately and fails (stopping the pid) if
`hidden=1` does not come back.  At the end it stops the client by pid unless
`--keep-client`.  To attach instead, pass `--pid <pid>` and omit `--lane`; the
same stop policy applies, so pass `--keep-client` when the client is not yours.

`--reply-delay` records the lane's reply-delay setting in `meta.json`
(`reply_delay.hint_ms`); it does not change the server.  The server switch lives
on the server's `feat/reply-delay` branch and is recorded for the later run.

`--fake-mcp <fixture>` replays recorded MCP replies for offline tests.  It does
not start or attach a client.

## Offline tests

```bash
python -m unittest discover -s tools/parity/tests -p "test_*.py" -v
```

The tests parse and validate both scenarios, reject bad ids/verbs/untagged
conditions/`LeaveWorld`, run the mini scenario through a recorded MCP fixture
and assert the capture files, and align the K2 capture to reproduce the audit's
corrected client times.  The K2 test skips unless `AA_PARITY_K2_RUN` points at
`<handoff>/2026-11-01-combat-k2k3/run`.

## Open for the first live run

* The lane and the client build path are machine-local; no game files or PNGs
  are written into this repository.
* `click-ui` needs a plain Lua path (`window.child`, `window.slot[2]`); closure
  paths (`UpdateList^window`) are not supported.
* Animation state is the best the script API exposes today; a clip name/time is
  not guaranteed.  The research step for the CryAnimation structures is
  separate.
* `target` by name uses a right-click screen pick and a short sweep because
  `X2Util:ConvertWorldToScreen` does not always agree with the pointer space;
  the template path uses `/findobject` and is the more deterministic one.
* The reward of this step is the capture, not a verdict: server replies and
  scripted steps are still tagged, not interpreted.
