//! Browser backend: the pak is a v2 baked pak served by `aa-web` (`/pak/manifest.json`, the
//! index, one URL per block). [`open`] fetches the manifest and index before the app starts; after
//! that [`Pak::read`] stays synchronous: blocks already in memory are used directly, a missing one
//! is fetched with a synchronous request (block URLs are immutable, so the browser's HTTP cache
//! answers repeats). [`prefetch`] fetches the blocks of a folder ahead of time, in parallel.
use std::collections::{HashMap, VecDeque};
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

use crate::{Pak, normalize};

static PAK: OnceLock<Pak> = OnceLock::new();

/// Packed blocks kept in memory (`?pak_store_mb=` is not read; the budget is fixed).
const STORE_BUDGET: usize = 1 << 30;
/// Parallel requests while prefetching.
const PARALLEL: usize = 6;

pub(crate) struct Remote {
    block_url: String,
    store: Mutex<Store>,
}

#[derive(Default)]
struct Store {
    blocks: HashMap<u32, Arc<Vec<u8>>>,
    order: VecDeque<u32>,
    bytes: usize,
}

impl Store {
    fn insert(&mut self, b: u32, data: Arc<Vec<u8>>) {
        if self.blocks.contains_key(&b) {
            return;
        }
        self.bytes += data.len();
        self.blocks.insert(b, data);
        self.order.push_back(b);
        while self.bytes > STORE_BUDGET && self.order.len() > 1 {
            if let Some(old) = self.order.pop_front().and_then(|o| self.blocks.remove(&o)) {
                self.bytes -= old.len();
            }
        }
    }
}

impl Remote {
    /// The packed bytes of block `b`, fetched synchronously when not in memory yet.
    pub(crate) fn get(&self, b: u32) -> io::Result<Arc<Vec<u8>>> {
        if let Some(d) = self.store.lock().unwrap().blocks.get(&b) {
            return Ok(d.clone());
        }
        let data = Arc::new(fetch_sync(&format!("{}{b}", self.block_url))?);
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
    for chunk in ids.chunks(PARALLEL) {
        let fetches = chunk.iter().map(|&b| {
            let url = format!("{}{b}", remote.block_url);
            async move { (b, fetch_bytes(&url).await) }
        });
        for (b, data) in futures_util::future::join_all(fetches).await {
            let data = data?;
            total += data.len() as u64;
            remote.store.lock().unwrap().insert(b, Arc::new(data));
        }
    }
    Ok(total)
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
/// char (0x80..=0xFF as U+F780..=U+F7FF): the low byte of each char is the data.
fn fetch_sync(url: &str) -> io::Result<Vec<u8>> {
    let err = |e: JsValue| io::Error::other(format!("GET {url}: {e:?}"));
    let xhr = web_sys::XmlHttpRequest::new().map_err(err)?;
    xhr.open_with_async("GET", url, false).map_err(err)?;
    xhr.override_mime_type("text/plain; charset=x-user-defined").map_err(err)?;
    xhr.send().map_err(err)?;
    let status = xhr.status().map_err(err)?;
    if status != 200 {
        return Err(io::Error::other(format!("GET {url}: HTTP {status}")));
    }
    let text: js_sys::JsString = xhr.response().map_err(err)?.dyn_into().map_err(err)?;
    let mut out = Vec::with_capacity(text.length() as usize);
    for unit in text.iter() {
        out.push(unit as u8);
    }
    Ok(out)
}
