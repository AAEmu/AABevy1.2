//! `aa-web`: the web host for the browser build.
//!
//! * `/pak/manifest.json`: the current baked pak (v2) version; never cached.
//! * `/pak/<version>/index`, `/pak/<version>/b/<n>`, `/pak/<version>/stats.json`,
//!   `/pak/<version>/prefetch/<prefix>`: the zstd index, one solid block (byte ranges allowed),
//!   per-folder sizes, and the blocks holding a folder. Versioned URLs are immutable, so browsers
//!   and CDNs keep them forever; a new pak gets a new version.
//! * `/net/<name>`: WebSocket bridged to a configured TCP game server (`--net login=127.0.0.1:1237`).
//! * everything else: the site (`--web`), brotli/gzip compressed once at startup.
//!
//! Every response carries the cross-origin isolation headers wasm threads need.
//!
//! ```text
//! aa-web --pak <baked>/game-v2.aapk --web web --listen 0.0.0.0:8080 \
//!        --net login=127.0.0.1:1237 --net game=127.0.0.1:1239
//! ```
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::extract::Request;
use axum::http::HeaderValue;
use axum::middleware::Next;
use axum::response::Response;
use axum::routing::get;

mod net;
mod pak;
mod site;

pub struct App {
    pub pak: Option<pak::PakHost>,
    pub site: site::Site,
    pub net: HashMap<String, SocketAddr>,
    pub net_slots: Arc<tokio::sync::Semaphore>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "aa_web=info".into()))
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mut listen, mut pak_path, mut web, mut net) = ("0.0.0.0:8080".to_string(), None, "web".to_string(), HashMap::new());
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = || it.next().cloned().ok_or_else(|| anyhow::anyhow!("{a} needs a value"));
        match a.as_str() {
            "--listen" => listen = val()?,
            "--pak" => pak_path = Some(val()?),
            "--web" => web = val()?,
            "--net" => {
                let v = val()?;
                let (name, addr) = v.split_once('=').ok_or_else(|| anyhow::anyhow!("--net name=host:port"))?;
                net.insert(name.to_string(), addr.parse()?);
            }
            _ => anyhow::bail!(
                "usage: aa-web [--listen 0.0.0.0:8080] [--pak game.aapk] [--web web] [--net name=host:port]..."
            ),
        }
    }

    let pak = match &pak_path {
        Some(p) => Some(pak::PakHost::open(p)?),
        None => None,
    };
    let site = site::Site::load(&web)?;
    let app = Arc::new(App { pak, site, net, net_slots: Arc::new(tokio::sync::Semaphore::new(512)) });
    if let Some(p) = &app.pak {
        tracing::info!("pak {} version {}: {} files, {} blocks", pak_path.unwrap(), p.version, p.files(), p.blocks());
    }
    tracing::info!("site {web}: {} files; net targets {:?}", app.site.len(), app.net);

    let router = Router::new()
        .route("/pak/manifest.json", get(pak::manifest))
        .route("/pak/{version}/index", get(pak::index))
        .route("/pak/{version}/stats.json", get(pak::stats))
        .route("/pak/{version}/b/{n}", get(pak::block))
        .route("/pak/{version}/prefetch/{*prefix}", get(pak::prefetch))
        .route("/net/{name}", get(net::bridge))
        .fallback(site::serve)
        .layer(axum::middleware::from_fn(isolation_headers))
        .with_state(app);
    let listener = tokio::net::TcpListener::bind(&listen).await?;
    tracing::info!("listening on http://{listen}");
    axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>()).await?;
    Ok(())
}

/// Cross-origin isolation (SharedArrayBuffer for wasm threads) and a few safe defaults.
async fn isolation_headers(req: Request, next: Next) -> Response {
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    for (k, v) in [
        ("cross-origin-opener-policy", "same-origin"),
        ("cross-origin-embedder-policy", "require-corp"),
        ("cross-origin-resource-policy", "same-origin"),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
    ] {
        h.insert(k, HeaderValue::from_static(v));
    }
    res
}
