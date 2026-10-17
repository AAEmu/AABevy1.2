#!/bin/bash
# usage: run_create.sh LOG args...   (builds with retries, copies the exe, runs the copy so cargo can rebuild meanwhile)
cd "$(dirname "$0")/../../../../../.."   # repo root
log=$1; shift
for i in 1 2 3 4 5 6 7 8 9 10 11 12; do
  out=$(cargo build -p aa-client 2>&1)
  if echo "$out" | grep -q "^error"; then
    if echo "$out" | grep -q "Access is denied"; then echo "exe locked, retry $i"; sleep 10; continue; fi
    echo "$out" | grep -E "^error" -A12; exit 1
  fi
  break
done
cp target/debug/aa-client.exe target/aa-client-create.exe || exit 1
RUST_LOG=${RUST_LOG:-warn,aa_client=info} timeout 300 ./target/aa-client-create.exe "$@" > "$log" 2>&1
