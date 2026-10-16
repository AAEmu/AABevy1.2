//! Lua 5.1 runtime for the original `.alb` UI bytecode and `toc.g` script loading.
//!
//! ```no_run
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let lua = aa_lua::new_lua()?;
//! // install the native API (widgets, X2*, ...) as globals here, then:
//! aa_lua::run_alb(&lua, aa_lua::stage::INIT_SCRIPT)?;
//! aa_lua::run_toc(&lua, "scriptsbin/x2ui/loginstage/world_select/toc.g")?;
//! // or a whole screen in the client's order:
//! aa_lua::run_stage(&lua, aa_lua::Stage::WorldSelect)?;
//! # Ok(()) }
//! ```
//!
//! `mlua::Error` is not `Send + Sync` (the workspace builds mlua without its `send` feature), so
//! `?` from an `mlua::Result` into `anyhow::Result` needs `.map_err(|e| anyhow::anyhow!("{e}"))`.
//!
//! `.alb` files are Lua 5.1 bytecode for a 32-bit host with `float` numbers; [`load_alb`] rewrites
//! them for the host Lua (see [`chunk`]). `toc.g` files list the scripts of one UI module in load
//! order (see [`toc`]); [`stage`] has the order of whole screens.

pub mod chunk;
pub mod recorder;
pub mod stage;
pub mod toc;

use std::collections::HashSet;

use anyhow::{Context, Result};
use mlua::chunk::ChunkMode;
use mlua::{Function, Lua, StdLib};

pub use stage::Stage;
pub use toc::{alb_path, script_path, toc_files};

/// A script that failed while running a toc or stage: pak path and error.
pub type ScriptError = (String, anyhow::Error);

/// Scripts executed so far (pak paths), for `Script.LoadScript`.
#[derive(Default)]
struct LoadedScripts(HashSet<String>);

/// A Lua 5.1 state with the standard libraries the client's script system opens (base, string,
/// table, math, os, io, debug, package, coroutine) and CryEngine's `Script` loader table
/// (`Script.ReloadScript`, `Script.LoadScript`, `Script.UnloadScript`) reading from the pak.
pub fn new_lua() -> mlua::Result<Lua> {
    // SAFETY: `unsafe_new_with` only means "the debug library and C modules are available". The
    // client opens all standard libraries in its UI state, and we only run its own scripts.
    let lua = unsafe { Lua::unsafe_new_with(StdLib::ALL, mlua::LuaOptions::default()) };
    lua.set_app_data(LoadedScripts::default());
    install_script_table(&lua)?;
    Ok(lua)
}

/// `Script.*` from CryEngine's `CScriptBind_Script` (only the file loading part). Paths like
/// `SCRIPTS/x2ui/chat/option.lua` map to `scriptsbin/x2ui/chat/option.alb`. Like the engine,
/// a failing script is logged (`tracing::warn!`) and reported as `false`, not raised.
fn install_script_table(lua: &Lua) -> mlua::Result<()> {
    let script: mlua::Table = match lua.globals().get::<Option<mlua::Table>>("Script")? {
        Some(t) => t,
        None => lua.create_table()?,
    };
    script.set(
        "ReloadScript",
        lua.create_function(|lua, path: String| Ok(exec_logged(lua, &script_path(&path))))?,
    )?;
    script.set(
        "LoadScript",
        lua.create_function(|lua, (path, reload, _raise): (String, Option<bool>, Option<bool>)| {
            let p = script_path(&path);
            let loaded = lua.app_data_ref::<LoadedScripts>().is_some_and(|l| l.0.contains(&p));
            Ok(if loaded && !reload.unwrap_or(false) { true } else { exec_logged(lua, &p) })
        })?,
    )?;
    script.set(
        "UnloadScript",
        lua.create_function(|lua, path: String| {
            if let Some(mut l) = lua.app_data_mut::<LoadedScripts>() {
                l.0.remove(&script_path(&path));
            }
            Ok(())
        })?,
    )?;
    lua.globals().set("Script", script)
}

fn exec_logged(lua: &Lua, pak_path: &str) -> bool {
    match run_alb(lua, pak_path) {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!("[Lua Error] {e:#}");
            false
        }
    }
}

/// Read one script from the pak, convert it and load it (compile only, not run). `pak_path` may
/// name the `.alb` or the `.lua` that a `toc.g` lists (`scriptsbin/x2ui/hud/clock.lua`).
pub fn load_alb(lua: &Lua, pak_path: &str) -> Result<Function> {
    let path = alb_path(pak_path);
    let bytes = aa_pak::Pak::global().read(&path).with_context(|| format!("reading {path}"))?;
    load_alb_bytes(lua, &bytes, &path)
}

/// Like [`load_alb`] for bytes already in memory; `name` is the pak path used in error messages.
pub fn load_alb_bytes(lua: &Lua, bytes: &[u8], name: &str) -> Result<Function> {
    let host = convert_alb(bytes, name)?;
    lua.load(&host[..])
        .set_name(format!("@{name}"))
        .set_mode(ChunkMode::Binary)
        .into_function()
        .map_err(|e| anyhow::anyhow!("loading {name}: {e}"))
}

/// Convert `.alb` bytes to a binary chunk for the host Lua (see [`chunk::convert`]).
pub fn convert_alb(bytes: &[u8], name: &str) -> Result<Vec<u8>> {
    chunk::convert(bytes, Some(&format!("@{name}"))).with_context(|| format!("converting {name}"))
}

/// Run one script (load + call with no arguments).
pub fn run_alb(lua: &Lua, pak_path: &str) -> Result<()> {
    let path = alb_path(pak_path);
    let f = load_alb(lua, &path)?;
    if let Some(mut l) = lua.app_data_mut::<LoadedScripts>() {
        l.0.insert(aa_pak::normalize(&path));
    }
    f.call::<()>(()).map_err(|e| anyhow::anyhow!("running {path}: {e}"))
}

/// Run every script of a `toc.g` in order. Like the client, a failing script does not stop the
/// rest: all scripts run, and the error (if any) lists every failure.
pub fn run_toc(lua: &Lua, toc_path: &str) -> Result<()> {
    let errors = run_toc_collect(lua, toc_path)?;
    errors_to_result(toc_path, errors)
}

/// Run every script of a `toc.g` in order and return the failures. Fails only if the toc itself
/// cannot be read.
pub fn run_toc_collect(lua: &Lua, toc_path: &str) -> Result<Vec<ScriptError>> {
    Ok(run_files(lua, &toc_files(toc_path)?))
}

/// Run scripts in order, carrying on past failures; returns the failures.
pub fn run_files(lua: &Lua, files: &[String]) -> Vec<ScriptError> {
    files.iter().filter_map(|p| run_alb(lua, p).err().map(|e| (p.clone(), e))).collect()
}

/// Run the scripts of one screen in the client's order ([`Stage::scripts`]), carrying on past
/// failures. Does not run [`stage::INIT_SCRIPT`] (call [`run_alb`] once when creating the state).
pub fn run_stage(lua: &Lua, stage: Stage) -> Result<()> {
    let errors = run_files(lua, &stage.scripts()?);
    errors_to_result(stage.name(), errors)
}

fn errors_to_result(what: &str, errors: Vec<ScriptError>) -> Result<()> {
    if errors.is_empty() {
        return Ok(());
    }
    let mut msg = format!("{} script(s) of {what} failed:", errors.len());
    for (path, e) in &errors {
        msg.push_str(&format!("\n  {path}: {e:#}"));
    }
    Err(anyhow::anyhow!(msg))
}
