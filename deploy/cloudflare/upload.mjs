// Uploads a v2 baked pak (and the client wasm) to the R2 bucket behind worker.js, through the
// Worker's /admin calls. Blocks are stored by content, so a new pak only uploads the blocks that
// changed; the manifest goes last, so players switch to the new version only once it is complete.
//
//   node upload.mjs <https://worker-url> [--pak game.aapk] [--wasm aa-client_bg.wasm] [--net https://tunnel-url]
//
// The admin token is read from $AA_CF_TOKEN or the file named by $AA_CF_TOKEN_FILE (never printed).
import { openSync, readSync, fstatSync, readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { zstdDecompressSync, brotliCompressSync, constants } from 'node:zlib';

const args = process.argv.slice(2);
const base = args.shift()?.replace(/\/$/, '');
const opt = (name) => { const i = args.indexOf(name); return i >= 0 ? args[i + 1] : undefined; };
if (!base?.startsWith('http')) throw new Error('usage: node upload.mjs <worker url> [--pak f] [--wasm f] [--net url]');
const token = process.env.AA_CF_TOKEN ?? (process.env.AA_CF_TOKEN_FILE && readFileSync(process.env.AA_CF_TOKEN_FILE, 'utf8').trim());
if (!token) throw new Error('set AA_CF_TOKEN or AA_CF_TOKEN_FILE');
const auth = { authorization: `Bearer ${token}` };

async function put(key, body, tries = 6) {
  for (let t = 1; ; t++) {
    try {
      const r = await fetch(`${base}/admin/put/${encodeURIComponent(key)}`, { method: 'PUT', headers: auth, body });
      if (r.ok) return;
      throw new Error(`${r.status} ${(await r.text()).slice(0, 200)}`);
    } catch (e) {
      if (t >= tries) throw new Error(`put ${key}: ${e.message}`);
      await new Promise((r) => setTimeout(r, 1000 * 2 ** t));
    }
  }
}

async function list(prefix) {
  const keys = new Set();
  let cursor = '';
  do {
    const r = await fetch(`${base}/admin/list?prefix=${encodeURIComponent(prefix)}&cursor=${encodeURIComponent(cursor)}`, { headers: auth });
    if (!r.ok) throw new Error(`list: ${r.status} ${await r.text()}`);
    const j = await r.json();
    for (const k of j.keys) keys.add(k);
    cursor = j.cursor;
  } while (cursor);
  return keys;
}

const mb = (n) => `${(n / 1e6).toFixed(0)} MB`;

if (opt('--wasm')) {
  const raw = readFileSync(opt('--wasm'));
  const t = Date.now();
  const br = brotliCompressSync(raw, { params: { [constants.BROTLI_PARAM_QUALITY]: 11, [constants.BROTLI_PARAM_LGWIN]: 24, [constants.BROTLI_PARAM_SIZE_HINT]: raw.length } });
  console.log(`wasm: ${mb(raw.length)} -> brotli ${mb(br.length)} in ${((Date.now() - t) / 1000).toFixed(0)} s`);
  await put('site/aa-client_bg.wasm.br', br);
}

if (opt('--net')) {
  await put('net/origin', opt('--net'));
  console.log('net: game server origin set');
}

if (opt('--pak')) await uploadPak(opt('--pak'));

async function uploadPak(path) {
  const fd = openSync(path, 'r');
  const len = fstatSync(fd).size;
  const read = (n, at) => { const b = Buffer.alloc(n); let o = 0; while (o < n) o += readSync(fd, b, o, n - o, at + o); return b; };
  const foot = read(32, len - 32);
  if (foot.subarray(0, 6).toString('latin1') !== 'AAPKv2') throw new Error(`${path} is not a v2 baked pak`);
  const index = read(Number(foot.readBigUInt64LE(16)), Number(foot.readBigUInt64LE(8)));
  const raw = zstdDecompressSync(index);
  const version = createHash('sha256').update(index).digest('hex').slice(0, 16);

  // blocks: offset, packed, raw (u64 each), kind (u8); records: name, block, offset, len, size, codec
  let o = 4;
  const blocks = [];
  for (let i = 0, n = raw.readUInt32LE(0); i < n; i++, o += 25) {
    blocks.push({ offset: Number(raw.readBigUInt64LE(o)), packed: Number(raw.readBigUInt64LE(o + 8)) });
  }
  const records = [];
  const count = raw.readUInt32LE(o);
  o += 4;
  for (let i = 0; i < count; i++) {
    const l = raw.readUInt16LE(o);
    const name = raw.subarray(o + 2, o + 2 + l).toString('utf8');
    o += 2 + l;
    records.push({ name, block: raw.readUInt32LE(o), size: Number(raw.readBigUInt64LE(o + 20)) });
    o += 29;
  }
  if (o !== raw.length) throw new Error('index did not parse to its end');
  console.log(`pak ${path}: version ${version}, ${blocks.length} blocks, ${records.length} files, ${mb(len)}`);

  const have = await list('blocks/');
  console.log(`r2 has ${have.size} blocks already`);
  const table = Buffer.alloc(blocks.length * 12);
  let sent = 0, sentBytes = 0, skipped = 0, done = 0;
  const start = Date.now();
  const report = setInterval(() => {
    const s = (Date.now() - start) / 1000;
    console.log(`  ${done}/${blocks.length} blocks, uploaded ${sent} (${mb(sentBytes)}, ${(sentBytes / 1e6 / s * 8).toFixed(0)} Mbit/s), reused ${skipped}`);
  }, 15000);
  let next = 0;
  const worker = async () => {
    while (next < blocks.length) {
      const i = next++;
      const b = blocks[i];
      const data = read(b.packed, b.offset);
      const key = createHash('sha256').update(data).digest().subarray(0, 8);
      key.copy(table, i * 12);
      table.writeUInt32LE(b.packed, i * 12 + 8);
      const name = `blocks/${key.toString('hex')}`;
      if (have.has(name)) skipped++;
      else {
        await put(name, data);
        have.add(name);
        sent++;
        sentBytes += b.packed;
      }
      done++;
    }
  };
  await Promise.all(Array.from({ length: Number(process.env.AA_CF_PARALLEL ?? 12) }, worker));
  clearInterval(report);
  console.log(`blocks done: uploaded ${sent} (${mb(sentBytes)}), reused ${skipped}, ${((Date.now() - start) / 1000).toFixed(0)} s`);

  // what aa-web computes at startup: per-folder stats and the prefetch lists of those folders
  const norm = (n) => n.replace(/\\/g, '/').toLowerCase().replace(/^(\.\/|\/)+/, '');
  const folders = new Map();
  for (const r of records) {
    const parts = norm(r.name).replace(/^(game\/)+/, '').split('/').slice(0, 2);
    const key = parts.length === 2 ? parts.join('/') : '(root)';
    const f = folders.get(key) ?? { files: 0, bytes: 0, blocks: new Set() };
    f.files++;
    f.bytes += r.size;
    f.blocks.add(r.block);
    folders.set(key, f);
  }
  const download = (ids) => [...ids].reduce((s, i) => s + blocks[i].packed, 0);
  const stats = [...folders].map(([name, f]) => ({ name, files: f.files, bytes: f.bytes, download: download(f.blocks) }));
  stats.sort((a, b) => b.download - a.download);
  const pre = `pak/${version}`;
  await put(`${pre}/table`, table);
  await put(`${pre}/index`, index);
  await put(`${pre}/stats.json`, JSON.stringify({ version, folders: stats }));
  for (const [name, f] of folders) {
    const ids = [...f.blocks].sort((a, b) => a - b);
    await put(`${pre}/prefetch/${name}`, JSON.stringify({ prefix: name, files: f.files, download: download(ids), blocks: ids }));
  }
  const manifest = {
    version,
    index: `/pak/${version}/index`,
    indexBytes: index.length,
    indexRawBytes: raw.length,
    blockUrl: `/pak/${version}/b/`,
    blocks: blocks.length,
    files: records.length,
    bytes: len,
  };
  await put('pak/current.json', JSON.stringify(manifest));
  console.log(`live: pak ${version}`);
}
