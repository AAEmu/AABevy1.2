#!/bin/bash
# Frame time benchmarks of release builds, alternating between builds so machine noise hits both.
#
#   tools/perf.sh <scenario> <rounds> <label>=<dir> [<label>=<dir> ...]
#
# <dir> holds fly.exe and aa-client.exe (copy target/release/examples/fly.exe and target/release/aa-client.exe
# into target/perf/<label>/ to keep a build). Scenarios:
#   fly-full    fly --spawn nuian --bench 30            flying 80 m/s over the Nuian start area
#   fly-stand   fly --spawn nuian --bench 20 --bench-speed 0
#   fly-elf     fly --spawn elf --bench 20 --bench-speed 0         dense forest
#   game-spawn  aa-client --enter-world at the Nuian spawn, standing
#   game-run    aa-client --enter-world at the Nuian spawn, running forward (W held)
#   game-town   aa-client --enter-world in the 130 NPC town (13650, 10450), standing
# Env: ACCOUNT (default perfbench), EXTRA (extra flags), AA_PROFILE=1 adds the aa-profile blocks to the log.
# Logs go to shots/perf/<scenario>-<label>-<round>.log; one summary line per run is printed.
set -u
cd "$(dirname "$0")/.."
scenario=$1
rounds=$2
shift 2
account=${ACCOUNT:-perfbench}
mkdir -p shots/perf
for round in $(seq 1 "$rounds"); do
  for spec in "$@"; do
    label=${spec%%=*}
    dir=${spec#*=}
    log=shots/perf/$scenario-$label-$round.log
    case $scenario in
      fly-full) cmd=("$dir/fly.exe" --spawn nuian --bench 30) ;;
      fly-stand) cmd=("$dir/fly.exe" --spawn nuian --bench 20 --bench-speed 0) ;;
      fly-elf) cmd=("$dir/fly.exe" --spawn elf --bench 20 --bench-speed 0) ;;
      game-spawn) cmd=("$dir/aa-client.exe" --enter-world --account "$account" --pos 15578.042,15382.122 --yaw 158.74 --bench 20) ;;
      game-run) cmd=("$dir/aa-client.exe" --enter-world --account "$account" --pos 15578.042,15382.122 --yaw 158.74 --walk 6:200:W --bench 20 --bench-after 8) ;;
      game-town) cmd=("$dir/aa-client.exe" --enter-world --account "$account" --pos 13650,10450 --yaw 0 --bench 20 --bench-after 14) ;;
      *) echo "unknown scenario $scenario"; exit 1 ;;
    esac
    # shellcheck disable=SC2086
    "${cmd[@]}" ${EXTRA:-} > "$log" 2>&1
    summary=$(grep -a "BENCH frames" "$log" | tail -1 | sed 's/\x1b\[[0-9;]*m//g' | sed 's/.*BENCH/BENCH/')
    echo "$scenario $label round $round: ${summary:-no BENCH line (see $log)}"
  done
done
