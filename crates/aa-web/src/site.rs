//! The static site, loaded into memory at startup with brotli and gzip variants precomputed (in
//! parallel, cached on disk across restarts).
//! Files named with a content hash (`app.1a2b3c4d.wasm`) are cached forever; the rest revalidate
//! with their ETag (a 304 costs one round trip and no body).
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::App;

struct Asset {
    content_type: &'static str,
    etag: String,
    immutable: bool,
    raw: Vec<u8>,
    br: Option<Vec<u8>>,
    gz: Option<Vec<u8>>,
}

pub struct Site {
    files: HashMap<String, Asset>,
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "webmanifest" => "application/json",
        "wasm" => "application/wasm",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "txt" | "md" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn compressible(ct: &str) -> bool {
    ct.starts_with("text/") || ct.starts_with("application/json") || ct == "application/wasm" || ct == "image/svg+xml"
}

/// `name.<8+ hex>.ext`
fn hashed_name(path: &str) -> bool {
    let file = path.rsplit('/').next().unwrap_or(path);
    let parts: Vec<&str> = file.split('.').collect();
    parts.len() >= 3 && parts[parts.len() - 2].len() >= 8 && parts[parts.len() - 2].chars().all(|c| c.is_ascii_hexdigit())
}

impl Site {
    pub fn load(dir: &str) -> anyhow::Result<Site> {
        let root = Path::new(dir);
        let mut paths = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d)? {
                let p = e?.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    paths.push(p);
                }
            }
        }
        // Compressed variants are cached on disk by content hash: the client's wasm (tens of MB)
        // takes minutes at the best brotli level, so only the first start after a build pays it.
        let cache = std::env::temp_dir().join("aa-web-site-cache");
        std::fs::create_dir_all(&cache)?;
        let assets = std::thread::scope(|s| {
            let jobs: Vec<_> = paths
                .iter()
                .map(|p| {
                    let cache = &cache;
                    s.spawn(move || -> anyhow::Result<(String, Asset)> {
                        let rel = p.strip_prefix(root)?.to_string_lossy().replace('\\', "/");
                        let raw = std::fs::read(p)?;
                        let ct = content_type(&rel);
                        let hash = blake3::hash(&raw).to_hex()[..16].to_string();
                        let (br, gz) = if compressible(ct) && raw.len() > 256 {
                            (cached(cache, &hash, &format!("br{}", brotli_quality()), || brotli(&raw))?, cached(cache, &hash, "gz", || gzip(&raw))?)
                        } else {
                            (Vec::new(), Vec::new())
                        };
                        let smaller = |v: Vec<u8>| (!v.is_empty() && v.len() < raw.len()).then_some(v);
                        let (br, gz) = (smaller(br), smaller(gz));
                        let asset = Asset { content_type: ct, etag: format!("\"{hash}\""), immutable: hashed_name(&rel), raw, br, gz };
                        Ok((rel, asset))
                    })
                })
                .collect();
            jobs.into_iter().map(|j| j.join().expect("site compression thread")).collect::<anyhow::Result<Vec<_>>>()
        })?;
        Ok(Site { files: assets.into_iter().collect() })
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }
}

pub async fn serve(State(app): State<Arc<App>>, req: Request) -> Response {
    let path = req.uri().path().trim_start_matches('/');
    let path = if path.is_empty() || path.ends_with('/') { format!("{path}index.html") } else { path.to_string() };
    let Some(a) = app.site.files.get(&path) else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };
    let h = req.headers();
    let mut res = if h.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()) == Some(a.etag.as_str()) {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        let accept = h.get(header::ACCEPT_ENCODING).and_then(|v| v.to_str().ok()).unwrap_or("");
        let (body, enc) = match (&a.br, &a.gz) {
            (Some(b), _) if accept.contains("br") => (b.clone(), Some("br")),
            (_, Some(g)) if accept.contains("gzip") => (g.clone(), Some("gzip")),
            _ => (a.raw.clone(), None),
        };
        let mut res = Response::new(Body::from(body));
        res.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static(a.content_type));
        if let Some(e) = enc {
            res.headers_mut().insert(header::CONTENT_ENCODING, HeaderValue::from_static(e));
        }
        res
    };
    let rh = res.headers_mut();
    rh.insert(header::ETAG, HeaderValue::from_str(&a.etag).unwrap());
    if a.br.is_some() || a.gz.is_some() {
        rh.insert(header::VARY, HeaderValue::from_static("accept-encoding"));
    }
    let cache = if a.immutable { "public, max-age=31536000, immutable" } else { "no-cache" };
    rh.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    res
}

fn brotli_quality() -> i32 {
    std::env::var("AA_WEB_BROTLI").ok().and_then(|q| q.parse().ok()).unwrap_or(11)
}

fn brotli(raw: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::new();
    // `AA_WEB_BROTLI=5` while iterating on the client: seconds instead of minutes, ~15% larger
    let params = brotli::enc::BrotliEncoderParams { quality: brotli_quality(), lgwin: 24, ..Default::default() };
    brotli::BrotliCompress(&mut &raw[..], &mut out, &params)?;
    Ok(out)
}

fn gzip(raw: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    gz.write_all(raw)?;
    gz.finish()
}

/// `make()`'s output, read from `<cache>/<hash>.<ext>` when an earlier start wrote it.
fn cached(cache: &Path, hash: &str, ext: &str, make: impl FnOnce() -> std::io::Result<Vec<u8>>) -> std::io::Result<Vec<u8>> {
    let path = cache.join(format!("{hash}.{ext}"));
    if let Ok(v) = std::fs::read(&path) {
        return Ok(v);
    }
    let v = make()?;
    let tmp = path.with_extension(format!("{ext}.tmp"));
    std::fs::write(&tmp, &v)?;
    std::fs::rename(&tmp, &path)?;
    Ok(v)
}
