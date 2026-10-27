# aa-web: hosting the browser build

`aa-web` is one binary that serves everything the browser build needs: the baked game data, the
site, and a bridge to the game servers. It is on the `bake-pak` branch, next to `aa-bake`
([`docs/aa-bake.md`](aa-bake.md)).

```text
cargo build --release -p aa-web
aa-web --pak <baked>/game-v2.aapk --web web --listen 0.0.0.0:8080 \
       --net login=127.0.0.1:1237 --net game=127.0.0.1:1239
```

## What it serves

| URL | What | Caching |
|---|---|---|
| `/pak/manifest.json` | current pak version, index URL, sizes | `no-cache` (tiny; decides the version) |
| `/pak/<version>/index` | the v2 index, zstd (2.5 MB for 180,672 files) | immutable |
| `/pak/<version>/b/<n>` | one block: a zstd frame of one folder's files, or a stored sound bank; `Range` works | immutable |
| `/pak/<version>/stats.json` | per folder: files, download size | immutable |
| `/pak/<version>/prefetch/<prefix>` | the blocks holding a folder (`ui/login_stage/`, `scriptsbin/`) | immutable |
| `/net/<name>` | WebSocket relayed to a `--net` TCP target (only those) | – |
| everything else | `web/` | brotli/gzip; hashed names immutable, the rest revalidate by ETag |

The `<version>` is a hash of the pak index, so a new pak gets new URLs: browsers and any CDN can
keep blocks forever, and an old version simply returns 404 (the page re-reads the manifest).
Blocks are read straight from the one `.aapk` file; deploying data is copying one file.

Every response sets `Cross-Origin-Opener-Policy: same-origin` and
`Cross-Origin-Embedder-Policy: require-corp`, so the page is cross-origin isolated
(`SharedArrayBuffer`, which a multithreaded wasm build of the client needs).

## The site (`web/`)

No framework, no build step. `index.html` (CSS inline, 1.3 KB brotli), `js/app.js`, `sw.js`.

* The launcher shows the pak version and size, local storage use, whether the login server is
  reachable through the bridge, and per-folder "download for offline" with progress.
* `sw.js` keeps blocks in a Cache Storage bucket per pak version (cache-first, ranges cut from the
  cached block) and deletes old versions when the manifest changes; the shell is
  stale-while-revalidate.
* Play opens `/client/` once the browser build of the client is published there.

`deploy/build-web-client.sh` builds that client into `web/client/pkg`. It needs `AA_BAKE_TOOLS`, a folder with
the WASI SDK (`wasi-sdk/`, for Lua and SQLite's C code), wasm-bindgen and binaryen's wasm-opt; `WASI_SDK`,
`WASM_BINDGEN` and `WASM_OPT` override the single paths. `aa-bake --textures` finds its encoders in the same folder.

Measured locally: the 44 blocks of `ui/login_stage` (99.4 MB, lossless pak) download at
~140 MB/s over loopback; `scriptsbin/` (all UI scripts) is 12 blocks, 2.2 MB.

## Deploying to a Windows host

```text
AA_DEPLOY_HOST=my-server deploy/windows-host/deploy.sh path/to/game.aapk 8080
```

builds `aa-web`, copies it, `web/` and `install.ps1` to `C:\aa-web` on the ssh host, uploads the
pak only if its size changed, then runs `install.ps1` there: a startup scheduled task (SYSTEM,
restarts on failure, log in `C:\aa-web\aa-web.log`), a firewall rule for the port, and a health
check against `/pak/manifest.json`. The bridge targets default to the AAEmu login (1237) and game
(1239) servers on the same machine.

For players outside the LAN, put it behind a Cloudflare Tunnel (or any TLS proxy): HTTPS is
required for service workers off localhost, HTTP/2 or HTTP/3 multiplexes the block requests, and
the immutable block URLs are cached at the edge (blocks are 1-8 MB, well under CDN object limits).
WebSockets pass through tunnels unchanged.

## Not done yet

* The browser build of the client itself (wasm): `aa-pak` needs an HTTP block source instead of
  file reads, plus the Lua, SQLite and networking work listed in the browser-port notes.
* TLS in `aa-web` itself (left to the tunnel/proxy).
