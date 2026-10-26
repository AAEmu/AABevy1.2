// Service worker: pak blocks are cache-first (versioned URLs never change), the site shell is
// served from cache and refreshed in the background, and old pak versions are dropped.
const SHELL = 'shell-v3';
const SHELL_FILES = ['/', '/js/app.js'];

self.addEventListener('install', (e) => {
  e.waitUntil(caches.open(SHELL).then((c) => c.addAll(SHELL_FILES)).then(() => self.skipWaiting()));
});

self.addEventListener('activate', (e) => {
  e.waitUntil(self.clients.claim());
});

const PAK = /^\/pak\/([0-9a-f]{16})\//;

self.addEventListener('fetch', (e) => {
  const req = e.request;
  if (req.method !== 'GET') return;
  const url = new URL(req.url);
  if (url.origin !== location.origin || url.pathname.startsWith('/net/')) return;

  if (url.pathname === '/pak/manifest.json') {
    e.respondWith(manifest(req));
    return;
  }
  const m = url.pathname.match(PAK);
  if (m) {
    e.respondWith(pak(req, `pak-${m[1]}`));
    return;
  }
  e.respondWith(shell(req, e));
});

// Network first (it is tiny and decides the version); on a new version, drop the old caches.
async function manifest(req) {
  try {
    const res = await fetch(req);
    if (res.ok) {
      const { version } = await res.clone().json();
      for (const k of await caches.keys()) {
        if (k.startsWith('pak-') && k !== `pak-${version}`) await caches.delete(k);
      }
      (await caches.open(SHELL)).put(req, res.clone());
    }
    return res;
  } catch {
    return (await caches.match(req)) ?? Response.error();
  }
}

// Cache first; byte ranges are cut from the cached block.
// `?sync` (the client's blocking reads) shares the cached block and gets the one pad byte aa-web
// would add; `?b64` gets it as base64 text (see aa-web `pak::block`).
async function pak(req, name) {
  const cache = await caches.open(name);
  const url = new URL(req.url);
  const sync = url.searchParams.has('sync');
  const b64 = url.searchParams.has('b64');
  url.search = '';
  const key = new Request(url);
  let res = await cache.match(key);
  if (!res) {
    res = await fetch(key);
    if (res.ok && res.status === 200) await cache.put(key, res.clone());
  }
  if (b64 && res.ok) {
    const body = new Uint8Array(await res.arrayBuffer());
    if (typeof body.toBase64 !== 'function') return fetch(req);
    return new Response(body.toBase64(), { status: 200, headers: { 'content-type': 'text/plain; charset=us-ascii' } });
  }
  if (sync && res.ok) {
    const body = new Uint8Array(await res.arrayBuffer());
    const padded = new Uint8Array(body.length + 1);
    padded.set(body, 1);
    return new Response(padded, { status: 200, headers: res.headers });
  }
  const range = req.headers.get('range');
  if (!range || !res.ok) return res;
  const buf = await res.arrayBuffer();
  const m = /^bytes=(\d*)-(\d*)$/.exec(range);
  if (!m) return new Response(buf, { status: 200, headers: res.headers });
  let start = m[1] === '' ? buf.byteLength - Number(m[2]) : Number(m[1]);
  let end = m[1] === '' || m[2] === '' ? buf.byteLength - 1 : Math.min(Number(m[2]), buf.byteLength - 1);
  start = Math.max(0, start);
  const headers = new Headers(res.headers);
  headers.set('content-range', `bytes ${start}-${end}/${buf.byteLength}`);
  return new Response(buf.slice(start, end + 1), { status: 206, headers });
}

// Stale while revalidate.
async function shell(req, e) {
  const cache = await caches.open(SHELL);
  const hit = await cache.match(req);
  const fresh = fetch(req).then((res) => {
    if (res.ok && res.status === 200) cache.put(req, res.clone());
    return res;
  });
  if (hit) {
    e.waitUntil(fresh.catch(() => {}));
    return hit;
  }
  return fresh;
}
