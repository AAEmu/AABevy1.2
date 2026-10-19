#!/bin/sh
# usage: tools/skilltest/retry.sh OUT ARGS...  -- runs the driver, retrying a few times while the server still holds the old session
out=$1; shift
for i in 1 2 3 4 5 6; do
  rm -f "$out.jsonl"
  MSYS_NO_PATHCONV=1 ./target/debug/examples/skilltest_run.exe "$@" --out "$out.jsonl" > "$out.log" 2>&1 && exit 0
  if grep -q "server closed the connection\|timed out" "$out.log"; then sleep 20; else exit 1; fi
done
exit 1
