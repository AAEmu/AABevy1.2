//! The static site, loaded into memory at startup with brotli and gzip variants precomputed.
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
        let mut files = HashMap::new();
        let root = Path::new(dir);
        let mut stack = vec![root.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d)? {
                let p = e?.path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                let rel = p.strip_prefix(root)?.to_string_lossy().replace('\\', "/");
                let raw = std::fs::read(&p)?;
                let ct = content_type(&rel);
                let (br, gz) = if compressible(ct) && raw.len() > 256 {
                    let mut br = Vec::new();
                    // quality 11 takes minutes on the client's wasm (tens of MB); 9 is close in size
                    let big = raw.len() > 4 << 20;
                    brotli::BrotliCompress(&mut &raw[..], &mut br, &brotli::enc::BrotliEncoderParams {
                        quality: if big { 9 } else { 11 },
                        lgwin: if big { 24 } else { 22 },
                        ..Default::default()
                    })?;
                    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
                    gz.write_all(&raw)?;
                    let gz = gz.finish()?;
                    ((br.len() < raw.len()).then_some(br), (gz.len() < raw.len()).then_some(gz))
                } else {
                    (None, None)
                };
                let etag = format!("\"{}\"", &blake3::hash(&raw).to_hex()[..16]);
                files.insert(rel.clone(), Asset { content_type: ct, etag, immutable: hashed_name(&rel), raw, br, gz });
            }
        }
        Ok(Site { files })
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
