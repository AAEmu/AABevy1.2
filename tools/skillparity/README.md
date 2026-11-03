# Skill parity: original 1.2 client vs aa-client, event by event

Goal: every player skill must look, sound and feel the same in `aa-client` as in the original 1.2 client. The audit
casts the same skills, from the same character, on the same target, through the same server, in both clients, records
what each client does as a timeline of events, and compares the timelines skill by skill. A side-by-side video of each
skill backs every finding.

```
tools/skillparity/
  README.md            this file: the trace format both clients write, and the workflow
  plan.py              which skills to cast, in which order, on which slot, with which weapon and how long to wait
  run_original.py      drives the original client through aaemu-clientdriver (MCP) and writes orig/trace.jsonl (+ video)
  run_bevy.py          writes an aa-client --script for the same plan, runs it with AA_SKILL_TRACE, writes bevy/trace.jsonl (+ video)
  server_log.py        the lane server's packet trace -> net rows for both runs (ms timestamps, AAEMU_LOG_MS=1)
  diff.py              per-skill comparison of the two timelines -> report.json + report.md
  sidebyside.py        aligned side-by-side video per skill (original left, aa-client right), captioned
  common.py            shared: trace reading and writing, name rules, tolerances, the role rule
  mcp_driver.py        a small MCP (JSON-RPC over stdio) client for aaemu-clientdriver, used by run_original.py
  tests/               offline tests on small fixture traces (python -m unittest discover -s tools/skillparity/tests)
```

## The trace format (schema `skillparity/v1`)

One JSON object per line. Both clients write the same rows; a field a client cannot know is left out, never guessed.

Common fields on every row:

| field | type | meaning |
|---|---|---|
| `src` | `"orig"` / `"bevy"` | which client wrote the row |
| `wall_us` | integer | Unix time in microseconds, from the OS's precise wall clock (`GetSystemTimePreciseAsFileTime` / `SystemTime::now`). All sources (both clients, the server log, the runners) share this clock on one machine; it is the only clock used to align them |
| `frame` | integer | the client's frame counter when the row was made (original: the driver's `Present` count; aa-client: `FrameCount`) |
| `kind` | string | one of the kinds below |

Kinds and their fields:

| kind | fields | original source | aa-client source |
|---|---|---|---|
| `mark` | `label` (e.g. `skill:14376:start`), `skill_id` | runner | runner (script `lua`/`echo` line, or the trace's own mark) |
| `press` | `key` (`"1"`..`"0"`, `"-"`, `"="`), `slot` (1..12) | driver input injection | the scripted key press as the HUD consumes it |
| `anim` | `inst` (string: the original's skeleton pointer as hex, aa-client's entity bits), `role` (`player` / `target` / `other`, when known), `clip` (the CAL animation name, lowercase), `file` (pak path, when known), `layer` (0 = full body; the original's layer index), `speed` (playback speed), `blend` (transition time, s), `loop` (bool), `flags` (original only: `CA_*` bits) | `CSkeletonAnim::StartAnimation` / `StartAnimationById` hooks | `aa_anim::Animator` starts (base clip and layer) |
| `fx` | `name` (full `library.group.effect`, lowercase), `phase` (`spawn`; the original also logs `find` for lookups), `pos` ([x, y, z], game coordinates, when known), `inst` / `role` / `bone` (attach, when known), `scale` | `CParticleManager::FindEffect` + emitter creation hooks | the effect start in aa-client combat / aa-fx |
| `sound` | `name` (the FMOD event or sound name, lowercase, `group:event` or path as the client names it), `phase` (`create` / `play`), `pos` (when known) | `CSoundSystem::CreateSound`, `CSound::Play` hooks | aa-audio voice start (event path) |
| `net` | `dir` (`out` = client to server, `in`), `op` (packet name, e.g. `CSStartSkill`, `SCSkillStarted`), `skill_id`, `tl` (time-line / cast id when the packet has one), `obj` (the unit), `fields` (small object with the other decoded fields, optional) | the lane server's log (`server_log.py`; the same server rows serve both runs) | aa-net events and commands |
| `castbar` | `unit` (`player` / `target`), `state` (`start` / `stop`), `name`, `total_ms`, `elapsed_ms`, `channel` (bool) | `X2Unit:UnitCastingInfo` sampled per frame (change rows only) | the same API, from the same Rust state the Lua sees |
| `cooldown` | `slot`, `remain_ms`, `total_ms`, `gcd` (bool: the global cooldown pair) | `X2Action:GetActionCooldown` / the GCD pair (change rows only) | the same API |
| `pos` | `unit` (`player` / `target`), `x`, `y`, `z`, `yaw` (degrees, 0 = north, counter-clockwise) | `client_unit_state` / entity world position (seamless origin added) | `Transform` in game coordinates |
| `frame` | `dt_ms` | `Present` interval | frame delta |

Rules:

* **Time**: `wall_us` only. Runners record the moment they inject a key as a `press` row; the client's own time of seeing it is
  the first `frame` after it. Per-skill timings are measured from the `press` row of that skill.
* **Rows are events, not samples**: `castbar`, `cooldown` and `pos` are written when a value changes (positions: moved more than
  1 cm or turned more than 0.1 degree), so a still client writes nothing.
* **Names are compared lowercase**, with `\` turned into `/` and no `game/` prefix on paths.
* **Roles**: aa-client knows its player and target. The original labels skeletons by pointer; `run_original.py` learns the
  player's skeleton from the first skill clip after the first press (and the target's from its hit reactions) and rewrites `role`.
* A capture is one directory: `<capture-root>/<run-id>/{orig,bevy}/trace.jsonl`, `video.mp4`, `video.json`
  (`{"t0_wall_us": ..., "fps": 30}`: frame `k` of the video shows wall time `t0 + k / fps`), `meta.json` (plan, client build, lane,
  server commit, character, start and end).

## Workflow

1. One lane server (`AAEmu-rust-1.2`, `Scripts/lane.sh new ...`, `live-stack.sh start`) with `AAEMU_LOG_MS=1` and the packet field trace.
2. `plan.py` picks the skills (default: the 110 shown skills of the ten trees, plus the basic attack), from `compact.sqlite3`.
3. The GM character is prepared once (level 50, the tree learned, `/ignoreskillcds false`, the weapon of the tree, the action bar,
   the training doll at a fixed spot in a quiet place). With `--stage`, both runners reset each skill the same way before its press:
   target self (ends the auto-attack), `/clearcombat`, `/rcd` (real cooldowns, none left over), the move to the spot, a settle wait.
   The ignore flag stays off: with it on, the server answers every cast with `SCSkillCooldownReset` and the cooldowns are not real.
4. `run_original.py` casts the plan in the original client with the driver's trace and video; `run_bevy.py` casts the same plan in
   aa-client with `AA_SKILL_TRACE` (wall clock, `--require-wall-clock`) and its own video (aa-dev's `--video`: screenshots taken
   inside the client on a wall-clock tick grid, so frame `k` is at `t0 + k / fps` like the original's; `docs/aa-dev.md`).
5. `diff.py` aligns both on each skill's `press` and reports, per skill: time from press to `CSStartSkill`, to the first player clip,
   to each effect and sound, to the server's answer and the cast bar; clip names, layers, speeds and blends; the effect and sound sets;
   cast bar and cooldown durations; movement (dashes, leaps, knock-backs); and anything one client did that the other did not.
6. `sidebyside.py` cuts both videos at each press and stacks them.

Tolerances (the report flags anything outside them): timings 1 frame of the slower client (about 17 ms at 60 fps, 33 ms at 30 fps),
playback speed 2 %, blend 0.05 s, positions 0.1 m, names exact.

## Usage

Python 3, standard library only. Nothing machine-specific lives in the repository: paths come from options or these variables.

| variable | used by | meaning |
|---|---|---|
| `AA_DB` | `plan.py` | the server's `compact.sqlite3` (`--db`). Without it `plan.py` exits with code 2 and a message |
| `AA_PARITY_ROOT` | the runners | the capture root (`--capture-root`), outside the repository |
| `AA_DRIVER`, `AA_DRIVER_DLL`, `AA_LAUNCHER`, `AA_CLIENT`, `AA_LOGIN`, `AA_USER`, `AA_PASSWORD_FILE` | `run_original.py` | the driver executable, its DLL, launcher and client, and the login (`--driver --dll --launcher --client --login --user --password-file`) |
| `AA_CLIENT_EXE`, `AA_ACCOUNT`, `AA_PASSWORD_FILE`, `AA_CHARACTER`, `AA_PAK`, `AA_DB` | `run_bevy.py` | aa-client and its login (`--exe --account --password-file --character --pak --db`) |
| `FFMPEG`, `SKILLPARITY_FONT` | `sidebyside.py` | ffmpeg (else the one on `PATH`) and a `.ttf` for the captions |

```
# 1. the plan (110 shown skills of the ten trees, plus the basic attack batch); batches of at most 10 slots, re-slotted between batches
python tools/skillparity/plan.py --db "$AA_DB" --trees 1,2 --out plan.json          # --skills ids, --defaults none|basic|all, --slots-per-page 10|12

# 2. the original client (started by you or the lane; this tool only attaches to its pid and never stops it)
python tools/skillparity/run_original.py setup --pid PID --plan plan.json --move "x y z" --spawn-doll   # GM commands + the doll
python tools/skillparity/run_original.py bar   --pid PID --plan plan.json --batch 1                     # puts one batch on the bar, prints it
python tools/skillparity/run_original.py run   --pid PID --plan plan.json --capture-root ROOT --run-id RUN [--video] [--ground-click X,Y]
python tools/skillparity/run_original.py roles ROOT/RUN/orig/trace.jsonl                                  # the role rule on a trace

# 3. aa-client
python tools/skillparity/run_bevy.py --plan plan.json --exe AA_CLIENT_EXE --account A --login H:P --password-file P --character C     --capture-root ROOT --run-id RUN [--video --ffmpeg FFMPEG] [--ground-click X,Y] [--dry-run]

# 4. the server's side (once per run, both clients share it), then the comparison and the videos
python tools/skillparity/server_log.py rs-game.log --date YYYY-MM-DD --trace ROOT/RUN/orig/trace.jsonl --trace ROOT/RUN/bevy/trace.jsonl     --out ROOT/RUN/server.jsonl
python tools/skillparity/diff.py ROOT/RUN [--skills ids] [--tol-ms N]                      # report.json + report.md in the run directory
python tools/skillparity/sidebyside.py ROOT/RUN [--skills ids] [--ffmpeg FFMPEG]           # ROOT/RUN/sidebyside/<skill>.mp4 + all.mp4
```

Details a reader of the reports should know:

* **What the tools assume of the drivers.** `run_original.py` needs the driver tools `client_trace {pid, action: start|drain|stop|status,
  kinds?}` (drain returns this format's rows with `src: "orig"`, `wall_us` and `frame`, and the driver logs the `press` rows of the keys
  it injects) and, with `--video`, `client_video {pid, action: start|stop|status, path, fps, ffmpeg?}` (writes `path` and `path.json`
  with `{t0_wall_us, fps, width, height, frames, dropped}`). It checks `tools/list` before it writes anything and exits with code 3
  naming the missing tools. `run_bevy.py` assumes the aa-client script command `trace TEXT` (a `mark` row whose label is TEXT);
  `--mark-command echo` falls back to `echo` lines, in which case `diff.py` cuts the trace at the `press` rows. aa-client writes
  its rows to the file named by `AA_SKILL_TRACE`, and takes `--require-wall-clock` (the flag this tool passes so that `wall_us` is the OS wall clock).
* **Server log.** `server_log.py` reads the `HH:MM:SS[.frac] [LEVEL] target - message` lines of the game log (the date comes from
  `--date` or the file name; `--tz` tells which time zone the server logs in, default local) and turns `StartSkill` handler lines,
  the field trace (`C->S level= opcode=0x.. NAMEPacket len= body=`) and `GamePacket: C->S / S->C` lines into `net` rows with
  `dir`, `op`, `skill_id`, `tl`, `obj`. Start the lane with `AAEMU_LOG_MS=1` (microsecond fractions; without it the log has whole seconds and
  the server rows are only that exact). Decoded server-to-client fields need the `aaemu_game::net::connection` trace target in
  addition to `net::fields=trace`.
* **Roles in the original.** `run_original.py` and `diff.py` give the original's `anim` rows their `role` by the rule above:
  the skeleton of the first non-idle, non-locomotion clip within 1 s after the first press is the player; the first other skeleton
  that plays a hit-reaction clip within 3 s of that press is the target.
* **Plan timing.** A skill's wait is its cast time + the channel time (capped at 6 s, then the runner stops the channel) + the flight time
  (effect delay + distance / projectile speed) + the longest chain of the skill's plot events + a 2.5 s tail, at least 3 s.
  Ground-targeted skills (target types 6, 11, 15, 18) need `--ground-click X,Y` in both runners; without it they are left out and the
  runner says which (`meta.json` lists them too).
* **What a PASS means.** A metric a client never records is NO-DATA, not PASS. The `pos` yaw tolerance is 2 degrees (not in the table above:
  an assumption of the tool). Segments whose press is missing in either client are NO-DATA.

Never commit captures: the original client's frames and traces stay under the capture root on D: (git-ignored `data/local/` or an
`AA_PARITY_ROOT` outside the repository).
