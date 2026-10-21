#!/usr/bin/env bash
# Copies the site (web/) into deploy/cloudflare/site for Workers static assets, without the client
# wasm (over the 25 MiB asset limit: upload.mjs --wasm puts it in R2). Run from the repo root after
# deploy/build-web-client.sh.
set -euo pipefail
out=deploy/cloudflare/site
rm -rf "$out" && mkdir -p "$out"
cp -r web/. "$out/"
rm -f "$out/client/pkg/aa-client_bg.wasm"
# every response is cross-origin isolated, as aa-web sends it; the wasm-bindgen glue changes with
# each build, so it revalidates (static assets send an ETag)
cat > "$out/_headers" <<'H'
/*
  Cross-Origin-Opener-Policy: same-origin
  Cross-Origin-Embedder-Policy: require-corp
  Cross-Origin-Resource-Policy: same-origin
  X-Content-Type-Options: nosniff
  Referrer-Policy: no-referrer
H
find "$out" -type f | wc -l | xargs echo "staged files:"
du -sh "$out"
