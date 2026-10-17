#!/bin/bash
# The way into the game through the normal login flow, timed: world select -> character select -> Start -> loading -> in game.
#
#   tools/perf-flow.sh <rounds> <label>=<dir>[|<flags>] [<label>=<dir>[|<flags>] ...]
#
# <dir> holds aa-client.exe (a copy of target/release/aa-client.exe). Builds run alternately; each run logs in as ACCOUNT
# (default bevycreate: one login per account at a time, never share it with a run of someone else), presses Start through
# the original scripts (`--goto loading`) and stays in game for 12 s. One summary line per run, from the `flow:` log lines
# (ingame/flow.rs): the loading screen's length and worst frame, the time from pressing Start to the first in-game frame,
# and the worst frame of the first 5 s in game. Logs go to shots/perf/flow-<label>-<round>.log.
# Env: ACCOUNT, EXTRA (extra flags, e.g. --no-hud, --no-prewarm, --start-after 8), GOTO (default loading: presses Start after 2 s; character-select with
# --start-after S presses it S s after the list came up), CYCLES=1 adds `--leave-world 6 --again` and a longer run (a round trip).
set -u
cd "$(dirname "$0")/.."
rounds=$1
shift
account=${ACCOUNT:-bevycreate}
mkdir -p shots/perf
for round in $(seq 1 "$rounds"); do
  for spec in "$@"; do
    label=${spec%%=*}
    dir=${spec#*=}
    flags=""
    # `label=dir|--some-flag`: the same build with its own flags
    if [[ $dir == *"|"* ]]; then flags=${dir#*|}; dir=${dir%%|*}; fi
    log=shots/perf/flow-$label-$round.log
    cycle=(--exit-after 22)
    if [ "${CYCLES:-0}" = 1 ]; then cycle=(--leave-world 6 --again --exit-after 75); fi
    # shellcheck disable=SC2086
    "$dir/aa-client.exe" --account "$account" --goto "${GOTO:-loading}" "${cycle[@]}" ${EXTRA:-} $flags > "$log" 2>&1
    sed 's/\x1b\[[0-9;]*m//g' "$log" | grep -a "flow: \(character list\|loading took\|first in-game\|in game: worst\)\|prewarm: \(pipelines settled\|cut short\|gave up\|worst\)\|loading: done\|Validation Error" | sed 's/.*INFO //; s/aa_client::[a-z:_]*: //; s/.*ERROR.*/CRASH (validation error)/' | tr '\n' '|' | sed "s/^/$label round $round: /"
    echo
  done
done
