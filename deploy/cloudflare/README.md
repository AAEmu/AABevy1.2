# Hosting the browser build on Cloudflare

| What | Where | Why |
|---|---|---|
| Site (`web/`: launcher, game page, JS glue, login art) | Workers static assets | served and cached at the edge |
| Client wasm | R2 `site/aa-client_bg.wasm.br`, brotli 11 | over the 25 MiB asset limit; sent pre-compressed |
| Pak | R2 `blocks/<content key>` + `pak/<version>/…` | one object per block; a new pak uploads only changed blocks |
| `/net/*` (login and game servers) | the Worker passes the WebSocket on to an aa-web next to the servers, through a Cloudflare Tunnel | browsers cannot open TCP sockets |

`worker.js` serves the same URLs as `crates/aa-web`, so the client and its service worker need no
changes. The service worker turns plain blocks into the `?b64`/`?sync` forms itself; the Worker
does it only for a page the service worker does not control yet.

On a custom domain, the Worker also keeps pak responses in the edge cache. `workers.dev` has no
edge cache, so on `workers.dev` R2 serves every block.

## Deploy

From the repo root, after `deploy/build-web-client.sh` (without `AA_WASM_NAMES`):

```sh
deploy/cloudflare/stage.sh                          # web/ -> deploy/cloudflare/site
cd deploy/cloudflare && npm install
cf deploy --secrets-file <secrets.json>             # {"ADMIN_TOKEN": "<random>"}, kept out of the repo
AA_CF_TOKEN_FILE=<file with the token> node upload.mjs https://aa-web.<account>.workers.dev \
    --wasm ../../web/client/pkg/aa-client_bg.wasm --pak <game.aapk> --net <tunnel url>
```

Each step can run on its own:
- `--wasm` after every client build;
- `--pak` for a new bake. It goes live when its manifest is written last; `AA_CF_PARALLEL` sets the number of parallel uploads (12 by default);
- `--net` when the tunnel changes.

## Game servers

Next to the login and game servers, run an aa-web that only bridges `/net/*`, plus a tunnel to it:

```sh
aa-web --web <any dir> --listen 127.0.0.1:8091 --net login=127.0.0.1:1237 --net game=127.0.0.1:1239
cloudflared tunnel --url http://127.0.0.1:8091      # prints https://<random>.trycloudflare.com
```

A quick tunnel gets a new URL on every start, so run `upload.mjs --net` again after each restart.
For a fixed setup, use a named tunnel (`cloudflared tunnel create`) on a domain in Cloudflare.
