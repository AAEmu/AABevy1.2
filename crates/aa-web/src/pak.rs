//! Serving a v2 baked pak ([`aa_pak::baked`]) to browsers, block by block, straight from the file.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::File;
use std::sync::Arc;

use aa_pak::baked::{self, Block, Record2};
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::App;

pub struct PakHost {
    file: Arc<File>,
    /// First 16 hex digits of the BLAKE3 of the packed index: changes with any content change.
    pub version: String,
    index: Vec<u8>,
    blocks: Vec<Block>,
    records: Vec<Record2>,
    manifest: Vec<u8>,
    stats: Vec<u8>,
}

impl PakHost {
    pub fn open(path: &str) -> anyhow::Result<PakHost> {
        let file = File::open(path)?;
        let len = file.metadata()?.len();
        let mut footer = [0u8; baked::FOOTER_SIZE as usize];
        read_at(&file, &mut footer, len - baked::FOOTER_SIZE)?;
        anyhow::ensure!(&footer[..8] == baked::MAGIC2, "{path} is not a v2 baked pak (run aa-bake repack)");
        let u64_at = |o: usize| u64::from_le_bytes(footer[o..o + 8].try_into().unwrap());
        let mut index = vec![0u8; u64_at(16) as usize];
        read_at(&file, &mut index, u64_at(8))?;
        let raw_len = u64_at(24) as usize;
        let (blocks, records) = baked::decode_index2(&zstd::bulk::decompress(&index, raw_len)?)?;
        let version = blake3::hash(&index).to_hex()[..16].to_string();

        let manifest = serde_json::json!({
            "version": version,
            "index": format!("/pak/{version}/index"),
            "indexBytes": index.len(),
            "indexRawBytes": raw_len,
            "blockUrl": format!("/pak/{version}/b/"),
            "blocks": blocks.len(),
            "files": records.len(),
            "bytes": len,
        });

        // Per folder (two levels under game/): files, original bytes, and the blocks they touch.
        let mut folders: BTreeMap<String, (usize, u64, BTreeSet<u32>)> = BTreeMap::new();
        for r in &records {
            let n = aa_pak::normalize(&r.name);
            let key: Vec<&str> = n.trim_start_matches("game/").split('/').take(2).collect();
            let key = if key.len() == 2 { key.join("/") } else { "(root)".to_string() };
            let f = folders.entry(key).or_default();
            f.0 += 1;
            f.1 += r.size;
            f.2.insert(r.block);
        }
        let mut folders: Vec<_> = folders
            .into_iter()
            .map(|(name, (files, bytes, b))| {
                let download: u64 = b.iter().map(|&i| blocks[i as usize].packed).sum();
                serde_json::json!({ "name": name, "files": files, "bytes": bytes, "download": download })
            })
            .collect();
        folders.sort_by_key(|f| std::cmp::Reverse(f["download"].as_u64().unwrap_or(0)));
        let stats = serde_json::json!({ "version": version, "folders": folders });

        Ok(PakHost {
            file: Arc::new(file),
            manifest: serde_json::to_vec(&manifest)?,
            stats: serde_json::to_vec(&stats)?,
            version,
            index,
            blocks,
            records,
        })
    }

    pub fn files(&self) -> usize {
        self.records.len()
    }

    pub fn blocks(&self) -> usize {
        self.blocks.len()
    }
}

fn read_at(file: &File, mut buf: &mut [u8], mut offset: u64) -> std::io::Result<()> {
    while !buf.is_empty() {
        #[cfg(windows)]
        let n = std::os::windows::fs::FileExt::seek_read(file, buf, offset)?;
        #[cfg(unix)]
        let n = std::os::unix::fs::FileExt::read_at(file, buf, offset)?;
        if n == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        buf = &mut buf[n..];
        offset += n as u64;
    }
    Ok(())
}

const IMMUTABLE: &str = "public, max-age=31536000, immutable";

fn host<'a>(app: &'a App, version: &str) -> Result<&'a PakHost, Response> {
    match &app.pak {
        Some(p) if p.version == version => Ok(p),
        Some(_) => Err((StatusCode::NOT_FOUND, "stale pak version, reload /pak/manifest.json").into_response()),
        None => Err((StatusCode::NOT_FOUND, "no pak").into_response()),
    }
}

fn bytes(body: Vec<u8>, content_type: &'static str, cache: &'static str, etag: Option<String>) -> Response {
    let mut res = Response::new(Body::from(body));
    let h = res.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    if let Some(e) = etag {
        h.insert(header::ETAG, HeaderValue::from_str(&e).unwrap());
    }
    res
}

pub async fn manifest(State(app): State<Arc<App>>) -> Response {
    match &app.pak {
        Some(p) => bytes(p.manifest.clone(), "application/json", "no-cache", Some(format!("\"{}\"", p.version))),
        None => (StatusCode::NOT_FOUND, "no pak").into_response(),
    }
}

pub async fn index(State(app): State<Arc<App>>, Path(version): Path<String>) -> Response {
    match host(&app, &version) {
        Ok(p) => bytes(p.index.clone(), "application/zstd", IMMUTABLE, None),
        Err(r) => r,
    }
}

pub async fn stats(State(app): State<Arc<App>>, Path(version): Path<String>) -> Response {
    match host(&app, &version) {
        Ok(p) => bytes(p.stats.clone(), "application/json", IMMUTABLE, None),
        Err(r) => r,
    }
}

/// The blocks holding every file under `prefix` (with or without `game/`), smallest set first.
pub async fn prefetch(State(app): State<Arc<App>>, Path((version, prefix)): Path<(String, String)>) -> Response {
    let p = match host(&app, &version) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let want = aa_pak::normalize(&prefix);
    let with_game = format!("game/{want}");
    let mut ids = BTreeSet::new();
    let mut files = 0usize;
    for r in &p.records {
        let n = aa_pak::normalize(&r.name);
        if n.starts_with(&want) || n.starts_with(&with_game) {
            ids.insert(r.block);
            files += 1;
        }
    }
    let download: u64 = ids.iter().map(|&i| p.blocks[i as usize].packed).sum();
    let body = serde_json::json!({ "prefix": want, "files": files, "download": download, "blocks": ids });
    bytes(serde_json::to_vec(&body).unwrap(), "application/json", IMMUTABLE, None)
}

/// One block as stored: a zstd frame (solid block) or the raw bytes (stored block, e.g. a sound
/// bank). A single `Range: bytes=a-b` is honoured.
///
/// `?sync`: the body gets one leading zero byte. Browsers read blocks synchronously as
/// `x-user-defined` text, and text decoding drops a leading byte-order mark whatever the charset,
/// so a block starting with one (a UTF-8 script stored as is) would lose bytes.
pub async fn block(
    State(app): State<Arc<App>>,
    Path((version, n)): Path<(String, String)>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let p = match host(&app, &version) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let Some(b) = n.parse::<usize>().ok().and_then(|i| p.blocks.get(i)).copied() else {
        return (StatusCode::NOT_FOUND, "no such block").into_response();
    };
    let etag = format!("\"{}-{n}{}\"", p.version, if query.contains_key("sync") { "s" } else { "" });
    if headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()) == Some(etag.as_str()) {
        let mut res = StatusCode::NOT_MODIFIED.into_response();
        res.headers_mut().insert(header::ETAG, HeaderValue::from_str(&etag).unwrap());
        return res;
    }
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok()).and_then(|v| parse_range(v, b.packed));
    let (start, len) = range.unwrap_or((0, b.packed));
    let file = p.file.clone();
    let pad = usize::from(query.contains_key("sync") && range.is_none());
    let data = tokio::task::spawn_blocking(move || {
        let mut buf = vec![0u8; pad + len as usize];
        read_at(&file, &mut buf[pad..], b.offset + start).map(|_| buf)
    })
    .await;
    let Ok(Ok(data)) = data else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "read failed").into_response();
    };
    let kind = if b.kind == baked::BLOCK_ZSTD { "application/zstd" } else { "application/octet-stream" };
    let mut res = bytes(data, kind, IMMUTABLE, Some(etag));
    let h = res.headers_mut();
    h.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    h.insert("x-block-raw", HeaderValue::from(b.raw));
    if range.is_some() {
        *res.status_mut() = StatusCode::PARTIAL_CONTENT;
        let cr = format!("bytes {start}-{}/{}", start + len - 1, b.packed);
        res.headers_mut().insert(header::CONTENT_RANGE, HeaderValue::from_str(&cr).unwrap());
    }
    res
}

/// `bytes=a-b`, `bytes=a-` or `bytes=-n` within `size` -> (start, len).
fn parse_range(v: &str, size: u64) -> Option<(u64, u64)> {
    let spec = v.strip_prefix("bytes=")?;
    if spec.contains(',') || size == 0 {
        return None;
    }
    let (a, b) = spec.split_once('-')?;
    let (start, end) = match (a.trim(), b.trim()) {
        ("", n) => {
            let n: u64 = n.parse().ok()?;
            (size.saturating_sub(n), size - 1)
        }
        (a, "") => (a.parse().ok()?, size - 1),
        (a, b) => (a.parse().ok()?, b.parse::<u64>().ok()?.min(size - 1)),
    };
    (start <= end && end < size).then(|| (start, end - start + 1))
}

#[cfg(test)]
mod tests {
    use super::parse_range;

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-9", 100), Some((0, 10)));
        assert_eq!(parse_range("bytes=90-", 100), Some((90, 10)));
        assert_eq!(parse_range("bytes=-5", 100), Some((95, 5)));
        assert_eq!(parse_range("bytes=50-500", 100), Some((50, 50)));
        assert_eq!(parse_range("bytes=100-", 100), None);
        assert_eq!(parse_range("bytes=0-1,4-5", 100), None);
    }
}
