#!/usr/bin/env bash
# Builds the browser client into web/client/pkg (served by aa-web at /client/).
#   deploy/build-web-client.sh [--dev]
# Needs: the wasm32-unknown-unknown target, the WASI SDK (Lua), wasm-bindgen 0.2.x matching
# Cargo.lock, and optionally binaryen's wasm-opt (paths below, override with env vars).
set -euo pipefail
cd "$(dirname "$0")/.."
TOOLS=${AA_BAKE_TOOLS:-D:/aa-baked/tools}
WBG=${WASM_BINDGEN:-$TOOLS/wasm-bindgen/wasm-bindgen.exe}
OPT=${WASM_OPT:-$TOOLS/binaryen-version_123/bin/wasm-opt.exe}
profile=release; flag=--release
[[ ${1:-} == --dev ]] && { profile=debug; flag=; }

cargo build $flag --target wasm32-unknown-unknown -p aa-client --bin aa-client
target=$(cargo metadata --format-version 1 --no-deps | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p' | sed 's/\\/\//g')
wasm=$target/wasm32-unknown-unknown/$profile/aa-client.wasm
rm -rf web/client/pkg
"$WBG" --target web --no-typescript --out-dir web/client/pkg "$wasm"
if [[ $profile == release && -x $OPT ]]; then
  # -O2 keeps build times sane on a 100 MB module; exceptions are wasm EH (Lua's longjmp)
  "$OPT" -O2 --strip-debug --strip-producers --enable-exception-handling --enable-bulk-memory --enable-nontrapping-float-to-int \
    --enable-sign-ext --enable-mutable-globals --enable-reference-types --enable-multivalue \
    web/client/pkg/aa-client_bg.wasm -o web/client/pkg/aa-client_bg.wasm
fi
# the login screen's art (crates/aa-client/assets/login), read over HTTP in the browser
rm -rf web/client/login && mkdir -p web/client/login
cp crates/aa-client/assets/login/{manifest.json,*.png} web/client/login/
rm -f web/client/login/preview_still.png
ls -la web/client/pkg
