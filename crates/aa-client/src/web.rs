//! Browser entry (wasm32): `web/client/index.html` calls [`start`], which loads the pak index, the
//! game database and the login screen's data over HTTP, then runs the app on the `#game` canvas.
//! Anything not prefetched here is fetched on first use (see `aa_pak::web`).
use wasm_bindgen::prelude::*;

/// Name of the trimmed game database inside the browser pak (`aa-bake repack --extra`).
const PAK_DB: &str = "aaweb/compact.sqlite3";
/// Folders fetched before the first frame, so the login screen does not stall on block requests.
const PREFETCH: [&str; 3] = ["scriptsbin/", "ui/login_stage/", "ui/common/"];

/// Shows `text` under the canvas through the page's `window.aaStatus`.
fn status(text: &str) {
    let Some(w) = web_sys::window() else { return };
    if let Ok(f) = js_sys::Reflect::get(&w, &"aaStatus".into()).and_then(|f| f.dyn_into::<js_sys::Function>()) {
        let _ = f.call1(&JsValue::NULL, &text.into());
    }
}

fn js_err(e: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&e.to_string())
}

#[wasm_bindgen]
pub async fn start() -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    status("Loading the game index…");
    let pak = aa_pak::web::open("/pak/manifest.json").await?;

    status("Loading the game database…");
    aa_pak::web::prefetch(PAK_DB).await?;
    let db = pak.read(PAK_DB).map_err(js_err)?;
    // the memory VFS is SQLite's default in the browser; crates open the database by its native path
    sqlite_wasm_rs::MemVfsUtil::<sqlite_wasm_rs::WasmOsCallback>::new()
        .import_db(aa_anim::db::DEFAULT_DB, &db)
        .map_err(js_err)?;
    drop(db);

    for (i, dir) in PREFETCH.iter().enumerate() {
        status(&format!("Loading the login screen ({}/{})…", i + 1, PREFETCH.len()));
        aa_pak::web::prefetch(dir).await?;
    }
    // the blocks a session needs next, into the HTTP cache while the player logs in
    wasm_bindgen_futures::spawn_local(async {
        if let Err(e) = aa_pak::web::warm("/client/warm.txt").await {
            web_sys::console::warn_1(&e);
        }
    });
    status("Starting…");
    crate::run();
    Ok(())
}
