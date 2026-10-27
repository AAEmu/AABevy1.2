#!/usr/bin/env bash
# Deploys aa-web, the site and a v2 baked pak to a Windows host over ssh (AA_DEPLOY_HOST, an ssh host or
# alias), then runs install.ps1 there. Usage, from the repo root:
#   AA_DEPLOY_HOST=my-server deploy/windows-host/deploy.sh path/to/game.aapk [port]
# The pak is only uploaded when its size differs from the one already there.
set -euo pipefail
PAK=${1:?usage: deploy.sh <v2 baked pak> [port]}
PORT=${2:-8080}
HOST=${AA_DEPLOY_HOST:?set AA_DEPLOY_HOST to the ssh host}
cargo build --release -p aa-web
EXE="$(cargo metadata --format-version 1 --no-deps | python -I -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')/release/aa-web.exe"

ssh "$HOST" 'New-Item -ItemType Directory -Force C:\aa-web\web | Out-Null'
scp -q "$EXE" "$HOST:C:/aa-web/aa-web.new.exe"
scp -q -r web/* "$HOST:C:/aa-web/web/"
scp -q deploy/windows-host/install.ps1 "$HOST:C:/aa-web/install.ps1"
REMOTE_SIZE=$(ssh "$HOST" "if (Test-Path C:\aa-web\game.aapk) { (Get-Item C:\aa-web\game.aapk).Length } else { 0 }" | tr -d '\r')
LOCAL_SIZE=$(stat -c %s "$PAK")
if [ "$REMOTE_SIZE" != "$LOCAL_SIZE" ]; then
  echo "uploading pak ($((LOCAL_SIZE / 1000000)) MB)..."
  scp -q "$PAK" "$HOST:C:/aa-web/game.new.aapk"
fi
ssh "$HOST" "& C:\aa-web\install.ps1 -Port $PORT"
