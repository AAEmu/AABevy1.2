//! Browser backend: the pak is a v2 baked pak served by `aa-web` (`/pak/manifest.json`, the
//! index, one URL per block). [`open`] fetches the manifest and index before the app starts; after
//! that [`Pak::read`] stays synchronous: blocks already in memory are used directly, a missing one
//! is fetched with a synchronous request (block URLs are immutable, so the browser's HTTP cache
//! answers repeats). [`prefetch`] fetches the blocks of a folder ahead of time, in parallel.
use std::collections::{HashMap, VecDeque};
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use futures_util::StreamExt;
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

use crate::{Pak, normalize};

static PAK: OnceLock<Pak> = OnceLock::new();

/// Packed blocks kept in memory (with the decompressed ones of `Pak::block` this is most of the heap). At 768 MB the
/// warm list pushed out what the character list's prewarm had read, and loading fetched it again, blocking.
const STORE_BUDGET: usize = 1 << 30;
/// Requests in flight while prefetching: each costs a round trip plus the store's first-byte time (about 0.35 s from
/// R2), so it takes many to fill the line; HTTP/2 carries them on one connection (HTTP/1.1 queues past six).
const PARALLEL: usize = 32;
/// [`warm`] waits while the blocks it fetched and nobody read yet take more than this.
const WARM_AHEAD: usize = 512 << 20;

pub(crate) struct Remote {
    block_url: String,
    store: Mutex<Store>,
}

#[derive(Default)]
struct Store {
    blocks: HashMap<u32, Arc<Vec<u8>>>,
    order: VecDeque<u32>,
    bytes: usize,
    /// Blocks [`warm`] fetched that were not read yet, and their bytes.
    unread: std::collections::HashSet<u32>,
    unread_bytes: usize,
}

impl Store {
    fn insert(&mut self, b: u32, data: Arc<Vec<u8>>) {
        if self.blocks.contains_key(&b) {
            return;
        }
        self.bytes += data.len();
        self.blocks.insert(b, data);
        self.order.push_back(b);
        // oldest first, but blocks already read go before warmed ones nobody has read yet
        while self.bytes > STORE_BUDGET && self.order.len() > 1 {
            let at = self.order.iter().position(|o| !self.unread.contains(o)).unwrap_or(0);
            if let Some(o) = self.order.remove(at)
                && let Some(old) = self.blocks.remove(&o)
            {
                self.bytes -= old.len();
                if self.unread.remove(&o) {
                    self.unread_bytes -= old.len();
                }
            }
        }
    }
}

impl Remote {
    /// The packed bytes of block `b`, fetched synchronously when not in memory yet.
    pub(crate) fn get(&self, b: u32) -> io::Result<Arc<Vec<u8>>> {
        touch(b);
        {
            let mut store = self.store.lock().unwrap();
            if let Some(d) = store.blocks.get(&b).cloned() {
                if store.unread.remove(&b) {
                    store.unread_bytes -= d.len();
                }
                return Ok(d);
            }
        }
        let t0 = js_sys::Date::now();
        let data = Arc::new(fetch_sync(&format!("{}{b}?sync", self.block_url))?);
        let file = READING.with(|r| r.borrow().clone());
        SYNC_LOG.lock().unwrap().push((b, data.len(), js_sys::Date::now() - t0, file));
        self.store.lock().unwrap().insert(b, data.clone());
        Ok(data)
    }

    fn has(&self, b: u32) -> bool {
        self.store.lock().unwrap().blocks.contains_key(&b)
    }
}

/// The pak opened by [`open`].
pub fn global() -> &'static Pak {
    PAK.get().expect("aa_pak::web::open must finish before Pak::global is used")
}

/// Fetches `manifest_url` (e.g. `/pak/manifest.json`) and the index it names, and makes the pak
/// [`Pak::global`]. Call once, before the app starts.
pub async fn open(manifest_url: &str) -> Result<&'static Pak, JsValue> {
    let manifest = fetch_bytes(manifest_url).await?;
    let m: serde_json::Value =
        serde_json::from_slice(&manifest).map_err(|e| JsValue::from_str(&format!("pak manifest: {e}")))?;
    let field = |k: &str| m.get(k).ok_or_else(|| JsValue::from_str(&format!("pak manifest without {k}")));
    let index_url = field("index")?.as_str().unwrap_or_default().to_string();
    let block_url = field("blockUrl")?.as_str().unwrap_or_default().to_string();
    let raw_len = field("indexRawBytes")?.as_u64().unwrap_or(0) as usize;
    let index = fetch_bytes(&index_url).await?;
    let remote = Remote { block_url, store: Mutex::default() };
    let pak = Pak::from_index(PathBuf::from(manifest_url), None, Some(remote), &index, raw_len, true)
        .map_err(|e| JsValue::from_str(&format!("pak index: {e}")))?;
    let _ = PAK.set(pak);
    Ok(global())
}

/// Fetches every block holding a file under `prefix` (with or without `game/`) that is not in
/// memory yet, `PARALLEL` at a time. Returns the bytes downloaded.
pub async fn prefetch(prefix: &str) -> Result<u64, JsValue> {
    let pak = global();
    let remote = pak.remote.as_ref().expect("web pak");
    let want = normalize(prefix);
    let with_game = format!("game/{want}");
    let mut ids: Vec<u32> = pak
        .entries
        .iter()
        .filter(|(n, _)| n.starts_with(&want) || n.starts_with(&with_game))
        .filter_map(|(_, e)| e.block)
        .filter(|&b| !remote.has(b))
        .collect();
    ids.sort_unstable();
    ids.dedup();
    let mut total = 0u64;
    let mut fetches = futures_util::stream::iter(ids.into_iter().map(|b| {
        let url = format!("{}{b}", remote.block_url);
        async move { (b, fetch_bytes(&url).await) }
    }))
    .buffer_unordered(PARALLEL);
    while let Some((b, data)) = fetches.next().await {
        let data = data?;
        total += data.len() as u64;
        remote.store.lock().unwrap().insert(b, Arc::new(data));
    }
    Ok(total)
}

/// Fetches, in the background, the blocks of the files listed in `list_url` (one path per line, in
/// the order a session needs them; `web/client/warm.txt`) into memory, so the blocking reads later
/// find them there (a blocking read costs a text-to-bytes conversion on top of the transfer).
/// Stays at most [`WARM_AHEAD`] ahead of what was read. Returns the bytes fetched.
pub async fn warm(list_url: &str) -> Result<u64, JsValue> {
    let pak = global();
    let remote = pak.remote.as_ref().expect("web pak");
    let list = String::from_utf8_lossy(&fetch_bytes(list_url).await?).into_owned();
    let mut seen = std::collections::HashSet::new();
    let ids: Vec<u32> = list.lines().filter_map(|n| pak.entry(n.trim())?.block).filter(|&b| seen.insert(b)).collect();
    let mut total = 0u64;
    let fetches = futures_util::stream::iter(ids.iter().copied())
        .then(|b| async move {
            // a sliding window: each request starts once the one before it has been let through
            while remote.store.lock().unwrap().unread_bytes > WARM_AHEAD {
                sleep_ms(250).await;
            }
            b
        })
        .filter(|&b| std::future::ready(!remote.has(b)))
        .map(|b| {
            let url = format!("{}{b}", remote.block_url);
            async move { (b, fetch_bytes(&url).await) }
        })
        .buffer_unordered(PARALLEL);
    let mut fetches = std::pin::pin!(fetches);
    while let Some((b, data)) = fetches.next().await {
        let data = data?;
        total += data.len() as u64;
        let mut store = remote.store.lock().unwrap();
        if !store.blocks.contains_key(&b) {
            store.unread.insert(b);
            store.unread_bytes += data.len();
            store.insert(b, Arc::new(data));
        }
    }
    web_sys::console::log_1(&format!("aa-pak: warmed {} blocks, {:.1} MB", ids.len(), total as f64 / 1e6).into());
    Ok(total)
}

thread_local! {
    /// Blocks [`want`] is downloading, and the ones it could not get (their readers fetch them the blocking way).
    static WANTED: std::cell::RefCell<(std::collections::HashSet<u32>, std::collections::HashSet<u32>)> = Default::default();
}

/// Whether every block in `blocks` is in memory; starts downloading the missing ones in the background (the page
/// keeps running), so work that needs them can wait for this instead of blocking on each read.
pub fn want(blocks: &[u32]) -> bool {
    let Some(remote) = global().remote.as_ref() else { return true };
    let mut all = true;
    for &b in blocks {
        if remote.has(b) || WANTED.with(|w| w.borrow().1.contains(&b)) {
            continue;
        }
        all = false;
        if !WANTED.with(|w| w.borrow_mut().0.insert(b)) {
            continue;
        }
        let url = format!("{}{b}", remote.block_url);
        wasm_bindgen_futures::spawn_local(async move {
            let remote = global().remote.as_ref().expect("web pak");
            match fetch_bytes(&url).await {
                Ok(data) => {
                    let mut store = remote.store.lock().unwrap();
                    if !store.blocks.contains_key(&b) {
                        // unread: eviction takes these last, they are about to be read
                        store.unread.insert(b);
                        store.unread_bytes += data.len();
                        store.insert(b, Arc::new(data));
                    }
                }
                Err(e) => {
                    web_sys::console::warn_1(&e);
                    WANTED.with(|w| w.borrow_mut().1.insert(b));
                }
            }
            WANTED.with(|w| w.borrow_mut().0.remove(&b));
        });
    }
    all
}

async fn sleep_ms(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(w) = web_sys::window() {
            let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    let _ = JsFuture::from(promise).await;
}

async fn fetch_bytes(url: &str) -> Result<Vec<u8>, JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
    let res: web_sys::Response = JsFuture::from(window.fetch_with_str(url)).await?.dyn_into()?;
    if !res.ok() {
        return Err(JsValue::from_str(&format!("GET {url}: HTTP {}", res.status())));
    }
    let buf = JsFuture::from(res.array_buffer()?).await?;
    Ok(js_sys::Uint8Array::new(&buf).to_vec())
}

/// Synchronous GET of binary data. Window-context sync requests cannot ask for an ArrayBuffer, so
/// the body comes back as a string with the `x-user-defined` charset, where each byte maps to one
/// char (0x80..=0xFF as U+F780..=U+F7FF): the low byte of each char is the data. The conversion
/// runs in JS (one call, JIT-compiled loop) rather than one wasm/JS crossing per byte.
#[wasm_bindgen(inline_js = r#"
export function aa_pak_sync_get(url) {
  const x = new XMLHttpRequest();
  // base64 (`?b64`) where the browser decodes it natively: an ASCII body is a one-byte string, twice as fast to
  // read as `x-user-defined` text and decoded at GB/s (aa-web zstd-encodes it back to the block's size on the wire)
  const b64 = typeof Uint8Array.fromBase64 === 'function';
  x.open('GET', b64 ? url.replace('?sync', '?b64') : url, false);
  if (!b64) x.overrideMimeType('text/plain; charset=x-user-defined');
  x.send();
  if (x.status !== 200) throw new Error(`GET ${url}: HTTP ${x.status}`);
  if (b64) return Uint8Array.fromBase64(x.responseText);
  // `?sync` bodies start with one pad byte (aa-web), so a byte-order mark is never sniffed
  const s = x.responseText, b = new Uint8Array(s.length - 1);
  for (let i = 1; i < s.length; i++) b[i - 1] = s.charCodeAt(i);
  return b;
}
"#)]
extern "C" {
    #[wasm_bindgen(catch)]
    fn aa_pak_sync_get(url: &str) -> Result<js_sys::Uint8Array, JsValue>;
}

thread_local! {
    /// The file being read (for [`sync_log`]).
    static READING: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

thread_local! {
    /// Blocks read so far, and the file that read each one first, in order.
    static TOUCHED: std::cell::RefCell<(std::collections::HashSet<u32>, Vec<String>)> = Default::default();
}

fn touch(b: u32) {
    TOUCHED.with(|t| {
        let mut t = t.borrow_mut();
        if t.0.insert(b) {
            let file = READING.with(|r| r.borrow().clone());
            t.1.push(file);
        }
    });
}

/// The file that first read each block, in the order of the reads, one a line: a session's
/// `web/client/warm.txt` (file names outlive block numbers, which change with every pak).
#[wasm_bindgen]
pub fn aa_pak_touch_log() -> String {
    TOUCHED.with(|t| t.borrow().1.iter().map(|f| format!("{f}
")).collect())
}

/// Every blocking fetch: `(block, bytes, ms, the file that needed it)`.
static SYNC_LOG: Mutex<Vec<(u32, usize, f64, String)>> = Mutex::new(Vec::new());

pub(crate) fn reading(path: &str) {
    READING.with(|r| {
        let mut r = r.borrow_mut();
        r.clear();
        r.push_str(path);
    });
}

/// The blocking fetches so far as tab-separated lines (`block, bytes, ms, file`): which files
/// stall the page, to group them into shared blocks (`aa-bake repack`) or prefetch them.
#[wasm_bindgen]
pub fn aa_pak_sync_log() -> String {
    let log = SYNC_LOG.lock().unwrap();
    log.iter().map(|(b, n, ms, f)| format!("{b}	{n}	{ms:.0}	{f}
")).collect()
}

/// Block decompressions so far: count, distinct blocks, raw bytes, milliseconds.
static UNZSTD: Mutex<(u32, Option<std::collections::HashSet<u32>>, u64, f64)> = Mutex::new((0, None, 0, 0.0));

/// Counts a decompressed block (logged every 100: a block decompressed again means the cache
/// of decompressed blocks is too small for the working set).
pub(crate) fn note_unzstd(b: u32, raw: usize, ms: f64) {
    let mut u = UNZSTD.lock().unwrap();
    u.0 += 1;
    u.1.get_or_insert_with(Default::default).insert(b);
    u.2 += raw as u64;
    u.3 += ms;
    if u.0 % 100 == 0 {
        let distinct = u.1.as_ref().map_or(0, |s| s.len());
        web_sys::console::log_1(&format!("aa-pak: {} block decompressions ({distinct} distinct), {:.0} MB, {:.0} ms", u.0, u.2 as f64 / 1e6, u.3).into());
    }
}

/// Sync fetches so far: count, bytes, milliseconds (logged now and then, to find stalls).
static SYNC_STATS: Mutex<(u32, u64, f64)> = Mutex::new((0, 0, 0.0));

fn fetch_sync(url: &str) -> io::Result<Vec<u8>> {
    let t0 = js_sys::Date::now();
    let data = aa_pak_sync_get(url).map_err(|e| io::Error::other(format!("GET {url}: {e:?}")))?.to_vec();
    let ms = js_sys::Date::now() - t0;
    let mut st = SYNC_STATS.lock().unwrap();
    st.0 += 1;
    st.1 += data.len() as u64;
    st.2 += ms;
    if st.0 % 25 == 0 || ms > 100.0 {
        web_sys::console::log_1(&format!("aa-pak: {} blocking block fetches, {:.1} MB, {:.0} ms (last {url}: {:.0} ms)", st.0, st.1 as f64 / 1e6, st.2, ms).into());
    }
    Ok(data)
}
