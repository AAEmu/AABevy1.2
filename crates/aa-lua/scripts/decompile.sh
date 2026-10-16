#!/bin/sh
# Decompile the client's .alb scripts to readable Lua with unluac.
#
#   crates/aa-lua/scripts/decompile.sh [src_dir] [out_dir] [unluac.jar]
#
# src_dir  extracted scriptsbin tree (default data/extract/game/scriptsbin; refresh it with
#          `cargo run -p aa-pak --bin aa-pak -- x scriptsbin/ data/extract/game`)
# out_dir  default data/extract/aa-lua/decompiled  (mirrors src_dir, .alb -> .lua)
# jar      default data/extract/aa-lua/tools/unluac.jar
#          (https://sourceforge.net/projects/unluac/, tested with 1.2.3.569)
#
# unluac reads the client's float-number 5.1 header directly, so no conversion is needed.
# Failures are listed in <out_dir>/_failed.txt.
set -u
ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
SRC=${1:-$ROOT/data/extract/game/scriptsbin}
OUT=${2:-$ROOT/data/extract/aa-lua/decompiled}
JAR=${3:-$ROOT/data/extract/aa-lua/tools/unluac.jar}
mkdir -p "$OUT"
: > "$OUT/_failed.txt"
export SRC OUT JAR
cd "$SRC" && find . -name '*.alb' | sed 's#^\./##' | xargs -P 4 -I{} sh -c '
  f="{}"; dst="$OUT/${f%.alb}.lua"; mkdir -p "$(dirname "$dst")"
  if ! java -jar "$JAR" "$SRC/$f" > "$dst" 2> "$dst.err"; then echo "$f" >> "$OUT/_failed.txt"; fi
  [ -s "$dst.err" ] || rm -f "$dst.err"'
echo "decompiled into $OUT; failures: $(wc -l < "$OUT/_failed.txt")"
