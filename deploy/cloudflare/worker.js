// aa-web on Cloudflare: the site comes from static assets (edge cached), the pak from R2, and
// `/net/*` WebSockets are bridged to the game servers' TCP ports (see README.md).

import { connect } from 'cloudflare:sockets';
//
// R2 layout (written by upload.mjs):
//   pak/current.json               the manifest, as aa-web serves it
//   pak/<version>/index            the zstd index
//   pak/<version>/stats.json       per-folder sizes
//   pak/<version>/prefetch/<dir>   the blocks of a folder (two levels under game/)
//   pak/<version>/table            12 bytes a block: content key (8, hex order) + packed size (4, LE)
//   blocks/<key>                   one block, shared by every version that has it
//   site/aa-client_bg.wasm.br      the client, brotli 11 (too big for static assets)

const IMMUTABLE = 'public, max-age=31536000, immutable';
const ISOLATION = {
  'cross-origin-opener-policy': 'same-origin',
  'cross-origin-embedder-policy': 'require-corp',
  'cross-origin-resource-policy': 'same-origin',
  'x-content-type-options': 'nosniff',
  'referrer-policy': 'no-referrer',
};

export default {
  async fetch(req, env, ctx) {
    const url = new URL(req.url);
    const p = url.pathname;
    try {
      if (p.startsWith('/net/')) return net(req, env, url);
      if (p.startsWith('/admin/')) return admin(req, env, url);
      if (req.method !== 'GET' && req.method !== 'HEAD') return text(405, 'method not allowed');
      if (p === '/pak/manifest.json') return object(env, 'pak/current.json', 'application/json', 'no-cache');
      if (p === '/client/pkg/aa-client_bg.wasm') return wasm(req, env);
      const m = p.match(/^\/pak\/([0-9a-f]{16})\/(index|stats\.json|prefetch\/.+|b\/(\d+))$/);
      if (m) return cached(req, ctx, () => pak(req, env, url, m[1], m[2], m[3]));
      return env.ASSETS.fetch(req);
    } catch (e) {
      return text(500, String(e?.stack ?? e));
    }
  },
};

function text(status, body) {
  return new Response(body, { status, headers: { 'content-type': 'text/plain; charset=utf-8', ...ISOLATION } });
}

// R2 reads fail now and then (a 500 for one block once stopped a whole session's prefetch): try three times.
async function r2get(env, key, options) {
  for (let i = 0; ; i++) {
    try {
      return await env.PAK.get(key, options);
    } catch (e) {
      if (i >= 2) throw e;
      await new Promise((r) => setTimeout(r, 100 * 2 ** i));
    }
  }
}

async function object(env, key, type, cache) {
  const o = await r2get(env, key);
  if (!o) return text(404, `no ${key}`);
  return new Response(o.body, { headers: { 'content-type': type, 'cache-control': cache, etag: o.httpEtag, ...ISOLATION } });
}

// The edge cache keeps immutable pak responses (on a custom domain; workers.dev has no cache, then this is a pass-through).
async function cached(req, ctx, make) {
  const cache = caches.default;
  const plain = !req.headers.has('range');
  if (plain) {
    const hit = await cache.match(req);
    if (hit) return hit;
  }
  const res = await make();
  if (plain && res.status === 200) ctx.waitUntil(cache.put(req, res.clone()));
  return res;
}

// The client wasm: stored brotli compressed, sent as is to browsers that take brotli (all of them).
async function wasm(req, env) {
  const o = await r2get(env, 'site/aa-client_bg.wasm.br', { onlyIf: req.headers });
  if (!o) return text(404, 'no client');
  const headers = { 'content-type': 'application/wasm', 'cache-control': 'no-cache', etag: o.httpEtag, vary: 'accept-encoding', ...ISOLATION };
  if (!('body' in o)) return new Response(null, { status: 304, headers });
  if (!/\bbr\b/.test(req.headers.get('accept-encoding') ?? '')) {
    const raw = o.body.pipeThrough(new DecompressionStream('brotli'));
    return new Response(raw, { headers });
  }
  return new Response(o.body, { headers: { ...headers, 'content-encoding': 'br' }, encodeBody: 'manual' });
}

// Block tables, kept per isolate: 150 KB a version.
const tables = new Map();
async function table(env, version) {
  let t = tables.get(version);
  if (!t) {
    const o = await r2get(env, `pak/${version}/table`);
    if (!o) return null;
    t = new DataView(await o.arrayBuffer());
    tables.set(version, t);
  }
  return t;
}

async function pak(req, env, url, version, what, n) {
  if (n === undefined) {
    const type = what === 'index' ? 'application/zstd' : 'application/json';
    return object(env, `pak/${version}/${what}`, type, IMMUTABLE);
  }
  const t = await table(env, version);
  if (!t) return text(404, 'stale pak version, reload /pak/manifest.json');
  const i = Number(n);
  if (i * 12 + 12 > t.byteLength) return text(404, 'no such block');
  const key = t.getBigUint64(i * 12).toString(16).padStart(16, '0'); // the key's bytes in order
  const size = t.getUint32(i * 12 + 8, true);
  const b64 = url.searchParams.has('b64');
  const sync = url.searchParams.has('sync');
  const headers = { 'cache-control': IMMUTABLE, etag: `"${version}-${n}${b64 ? 'b' : sync ? 's' : ''}"`, ...ISOLATION };

  // the blocking reads of a page the service worker does not control yet (it answers them itself otherwise)
  if (b64 || sync) {
    const o = await r2get(env, `blocks/${key}`);
    if (!o) return text(404, 'missing block');
    const body = new Uint8Array(await o.arrayBuffer());
    if (b64) return new Response(toBase64(body), { headers: { ...headers, 'content-type': 'text/plain; charset=us-ascii' } });
    const padded = new Uint8Array(body.length + 1);
    padded.set(body, 1);
    return new Response(padded, { headers: { ...headers, 'content-type': 'application/octet-stream' } });
  }

  const range = parseRange(req.headers.get('range'), size);
  const o = await r2get(env, `blocks/${key}`, range ? { range: { offset: range[0], length: range[1] } } : {});
  if (!o) return text(404, 'missing block');
  Object.assign(headers, { 'content-type': 'application/octet-stream', 'accept-ranges': 'bytes' });
  if (!range) return new Response(o.body, { headers: { ...headers, 'content-length': String(size) } });
  headers['content-range'] = `bytes ${range[0]}-${range[0] + range[1] - 1}/${size}`;
  return new Response(o.body, { status: 206, headers });
}

function toBase64(bytes) {
  if (typeof bytes.toBase64 === 'function') return bytes.toBase64();
  let s = '';
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
  return btoa(s);
}

// `bytes=a-b`, `bytes=a-`, `bytes=-n` -> [offset, length]
function parseRange(v, size) {
  const m = v && /^bytes=(\d*)-(\d*)$/.exec(v.trim());
  if (!m || size === 0) return null;
  let start, end;
  if (m[1] === '') {
    start = Math.max(0, size - Number(m[2]));
    end = size - 1;
  } else {
    start = Number(m[1]);
    end = m[2] === '' ? size - 1 : Math.min(Number(m[2]), size - 1);
  }
  return start <= end && end < size ? [start, end - start + 1] : null;
}

// `/net/server`: the address the login screen starts with (NET_SERVER, `host:port` of a login server).
// `/net/<host:port>`: a WebSocket bridged to that TCP address: the server the player typed, then the game server it
// announces. Only the AAEmu ports in NET_PORTS ("1237,1239,1250") are reachable, so this is not a relay to anything
// else. `/net/login` is NET_SERVER. Without NET_PORTS, WebSockets are passed on to an aa-web behind a tunnel
// (R2 `net/origin` or NET_ORIGIN).
async function net(req, env, url) {
  const name = decodeURIComponent(url.pathname.slice('/net/'.length));
  if (name === 'server') {
    if (!env.NET_SERVER) return text(404, 'no default server');
    return new Response(env.NET_SERVER, { headers: { 'content-type': 'text/plain', 'cache-control': 'no-cache', ...ISOLATION } });
  }
  if (req.headers.get('upgrade')?.toLowerCase() !== 'websocket') return text(426, 'websocket only');
  if (env.NET_PORTS) {
    const ports = new Set(String(env.NET_PORTS).split(',').map(Number));
    const [host, port] = splitHostPort(name === 'login' ? env.NET_SERVER ?? '' : name);
    if (!host || !ports.has(port)) return text(403, `only the game ports ${env.NET_PORTS} can be reached`);
    return bridge(host, port, url.searchParams.has('debug'));
  }
  const o = await r2get(env, 'net/origin');
  const base = (o && (await o.text()).trim()) || env.NET_ORIGIN;
  if (!base) return text(503, 'no game server configured');
  return fetch(new URL(url.pathname, base), req);
}

function splitHostPort(s) {
  const i = s.lastIndexOf(':');
  return i > 0 ? [s.slice(0, i).replace(/^\[|\]$/g, ''), Number(s.slice(i + 1))] : [s, 0];
}

// Binary WebSocket messages <-> one TCP connection; either side closing closes the other. `?debug` reports each
// step back as text messages.
function bridge(host, port, debug = false) {
  const [client, server] = Object.values(new WebSocketPair());
  server.accept();
  const dbg = (m) => debug && server.send(`dbg: ${m}`);
  dbg(`bridge to ${host}:${port}`);
  const socket = connect({ hostname: host, port }, { allowHalfOpen: false });
  const writer = socket.writable.getWriter();
  let closed = false;
  const close = (code = 1000, reason = '') => {
    if (closed) return;
    closed = true;
    try { server.close(code, reason); } catch {}
    socket.close().catch(() => {});
  };
  // writes go out in message order; a message may come as text, an ArrayBuffer, a typed array or a Blob
  let queue = Promise.resolve();
  server.addEventListener('message', (e) => {
    queue = queue
      .then(async () => {
        const d = e.data;
        const data = typeof d === 'string' ? new TextEncoder().encode(d)
          : d instanceof ArrayBuffer ? new Uint8Array(d)
          : ArrayBuffer.isView(d) ? new Uint8Array(d.buffer, d.byteOffset, d.byteLength)
          : new Uint8Array(await d.arrayBuffer());
        dbg(`ws -> tcp ${data.byteLength} bytes (${Object.prototype.toString.call(d)})`);
        await writer.write(data);
      })
      .catch((err) => {
        dbg(`write failed: ${err}`);
        close(1011, 'game server write failed');
      });
  });
  server.addEventListener('close', () => close());
  server.addEventListener('error', () => close());
  socket.readable
    .pipeTo(new WritableStream({ write: (chunk) => server.send(chunk) }))
    .then(() => { dbg('tcp closed'); close(); }, (err) => { dbg(`tcp error: ${err}`); close(1011, 'game server unreachable'); });
  return new Response(null, { status: 101, webSocket: client });
}

// Uploads (upload.mjs), with the ADMIN_TOKEN secret:
//   GET  /admin/list?prefix=&cursor=   keys in R2
//   PUT  /admin/put/<key>              body -> R2 <key>
async function admin(req, env, url) {
  const token = env.ADMIN_TOKEN;
  if (!token || req.headers.get('authorization') !== `Bearer ${token}`) return text(403, 'forbidden');
  const p = url.pathname;
  if (p === '/admin/list' && req.method === 'GET') {
    const r = await env.PAK.list({ prefix: url.searchParams.get('prefix') ?? '', cursor: url.searchParams.get('cursor') || undefined, limit: 1000 });
    return Response.json({ keys: r.objects.map((o) => o.key), cursor: r.truncated ? r.cursor : null });
  }
  // GET /admin/tcp?to=host:port&send=text: connect, send, return the first reply (bridge diagnostics)
  if (p === '/admin/tcp') {
    const [host, port] = splitHostPort(url.searchParams.get('to') ?? '');
    const t = Date.now();
    const log = [];
    try {
      const socket = connect({ hostname: host, port });
      const info = await socket.opened;
      log.push(`opened ${Date.now() - t} ms ${JSON.stringify(info)}`);
      const w = socket.writable.getWriter();
      await w.write(new TextEncoder().encode(url.searchParams.get('send') ?? 'hi\n'));
      log.push(`written ${Date.now() - t} ms`);
      const r = socket.readable.getReader();
      const first = await Promise.race([r.read(), new Promise((res) => setTimeout(() => res({ timeout: true }), 5000))]);
      log.push(first.timeout ? 'no reply in 5 s' : `reply ${first.value?.byteLength} bytes: ${JSON.stringify(new TextDecoder().decode(first.value ?? new Uint8Array()))}`);
      socket.close();
    } catch (e) {
      log.push(`error ${Date.now() - t} ms: ${e}`);
    }
    return text(200, log.join('\n'));
  }
  if (p.startsWith('/admin/put/') && req.method === 'PUT') {
    const key = decodeURIComponent(p.slice('/admin/put/'.length));
    const length = Number(req.headers.get('content-length'));
    const body = Number.isFinite(length) && length > 0 ? req.body.pipeThrough(new FixedLengthStream(length)) : await req.arrayBuffer();
    await env.PAK.put(key, body);
    return text(200, 'ok');
  }
  return text(404, 'no such admin call');
}
