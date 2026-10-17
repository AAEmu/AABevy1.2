#!/bin/bash
# One profiled run, printed compactly: tools/perf-profile.sh <exe> <args...>
# AA_PROFILE=1 AA_OFFSCREEN=1 (no swap chain, uncapped): the last two profile blocks of the log and the BENCH line.
cd "$(dirname "$0")/.."
exe=$1
shift
log=shots/perf/profile.log
AA_PROFILE=1 AA_OFFSCREEN=${AA_OFFSCREEN-1} "$exe" "$@" > "$log" 2>&1
sed 's/\x1b\[[0-9;]*m//g' "$log" | grep -a "BENCH frames" | sed 's/.*BENCH/BENCH/'
sed 's/\x1b\[[0-9;]*m//g' "$log" | grep -a "aa-profile: frame\|aa-profile: render thread\|aa-profile: timers\|aa-profile: passes\|aa-profile: fps" | tail -${TAILN:-5} | sed 's/.*aa-profile: //'
