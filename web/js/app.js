// Launcher: pak manifest and stats, block prefetch through the service worker's cache, storage.
const $ = (id) => document.getElementById(id);
const fmt = (b) => {
  if (b == null) return '–';
  const u = ['B', 'KB', 'MB', 'GB'];
  let i = 0;
  while (b >= 1000 && i < u.length - 1) { b /= 1000; i++; }
  return `${b.toFixed(i ? 1 : 0)} ${u[i]}`;
};

if ('serviceWorker' in navigator) navigator.serviceWorker.register('/sw.js');

async function storage() {
  if (!navigator.storage?.estimate) return;
  const { usage, quota } = await navigator.storage.estimate();
  const persisted = (await navigator.storage.persisted?.()) ?? false;
  $('cached').textContent = `${fmt(usage)} of ${fmt(quota)}${persisted ? ' (kept)' : ''}`;
  $('persist').disabled = persisted;
}

function server() {
  // One short WebSocket through the bridge to the login server.
  const proto = location.protocol === 'https:' ? 'wss' : 'ws';
  const ws = new WebSocket(`${proto}://${location.host}/net/login`);
  const done = (text, cls) => { $('server').textContent = text; $('server').className = cls; ws.close(); };
  const timer = setTimeout(() => done('no answer', 'bad'), 5000);
  ws.onopen = () => { clearTimeout(timer); done('reachable', 'ok'); };
  ws.onerror = () => { clearTimeout(timer); done('unreachable', 'bad'); };
}

async function play() {
  // The browser build of the client is published at /client/ once it exists.
  const r = await fetch('/client/index.html', { method: 'HEAD' }).catch(() => null);
  if (r?.ok) {
    $('play').disabled = false;
    $('play').onclick = () => (location.href = '/client/?login=login');
    $('play-note').textContent = '';
  } else {
    $('play-note').textContent = 'The browser build of the client is not published yet.';
  }
}

async function prefetch(manifest, prefixes) {
  const ids = new Set();
  for (const p of prefixes) {
    const r = await fetch(`/pak/${manifest.version}/prefetch/${p}`).then((r) => r.json());
    for (const b of r.blocks) ids.add(b);
  }
  const queue = [...ids];
  const total = queue.length;
  let done = 0, bytes = 0;
  const start = performance.now();
  const step = () => {
    $('bar').style.width = `${(done / total) * 100}%`;
    const secs = (performance.now() - start) / 1000;
    $('progress').textContent = `${done}/${total} blocks, ${fmt(bytes)} (${fmt(bytes / Math.max(secs, 0.1))}/s)`;
  };
  // Six requests in flight is what browsers allow per host on HTTP/1.1; HTTP/2 copes too.
  const worker = async () => {
    while (queue.length) {
      const id = queue.pop();
      const r = await fetch(`${manifest.blockUrl}${id}`);
      const n = (await r.arrayBuffer()).byteLength; // read before adding: workers interleave
      bytes += n;
      done++;
      step();
    }
  };
  step();
  await Promise.all(Array.from({ length: Math.min(6, total) }, worker));
  storage();
}

async function main() {
  play();
  server();
  storage();
  $('persist').onclick = async () => { await navigator.storage?.persist?.(); storage(); };

  const manifest = await fetch('/pak/manifest.json').then((r) => (r.ok ? r.json() : null)).catch(() => null);
  if (!manifest) {
    $('pak-version').textContent = 'no game data on this server';
    return;
  }
  $('pak-version').textContent = manifest.version;
  $('pak-size').textContent = `${fmt(manifest.bytes)} in ${manifest.blocks.toLocaleString()} blocks`;
  $('pak-files').textContent = manifest.files.toLocaleString();

  const stats = await fetch(`/pak/${manifest.version}/stats.json`).then((r) => r.json());
  const body = $('folders').tBodies[0];
  const frag = document.createDocumentFragment();
  for (const f of stats.folders) {
    const tr = document.createElement('tr');
    tr.innerHTML = `<td><input type="checkbox" value="${f.name}"></td><td><label>${f.name}</label></td>`
      + `<td class="n">${f.files.toLocaleString()}</td><td class="n">${fmt(f.download)}</td>`;
    tr.querySelector('label').onclick = () => tr.querySelector('input').click();
    frag.append(tr);
  }
  body.append(frag);
  body.onchange = () => {
    const picked = body.querySelectorAll('input:checked');
    let size = 0;
    picked.forEach((i) => (size += stats.folders.find((f) => f.name === i.value).download));
    $('fetch').disabled = !picked.length;
    $('fetch').textContent = picked.length ? `Download selected (${fmt(size)})` : 'Download selected';
  };
  $('fetch').onclick = async () => {
    $('fetch').disabled = true;
    const prefixes = [...body.querySelectorAll('input:checked')].map((i) => i.value);
    await prefetch(manifest, prefixes);
    $('fetch').disabled = false;
  };
}

main();
