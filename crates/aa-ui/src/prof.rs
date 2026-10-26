//! Opt-in timing of the UI frame: `AA_UI_PROFILE=1` logs every 3 s the average milliseconds per
//! frame of each part of [`crate::UiPlugin`]'s frame (input, the Lua tick's parts, the rebuild),
//! how often the meshes were rebuilt, and the per-frame handlers (`OnUpdate`, `OnEffect`) that
//! cost most. `AA_UI_PROFILE=2` also wraps every native method: time and calls per method name,
//! and which ones changed the tree (each change rebuilds the meshes). Off, a scope costs one
//! atomic load.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;
use web_time::Instant;

/// The browser has no environment: its page sets the level (`?ui-profile=1`) before the app starts.
pub static LEVEL_OVERRIDE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

fn level() -> u8 {
    static LEVEL: OnceLock<u8> = OnceLock::new();
    *LEVEL.get_or_init(|| {
        std::env::var("AA_UI_PROFILE")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(LEVEL_OVERRIDE.load(std::sync::atomic::Ordering::Relaxed))
    })
}

fn on() -> bool {
    level() > 0
}

#[derive(Default)]
struct Acc {
    frames: u32,
    since: Option<Instant>,
    sections: Vec<(&'static str, f64)>,
    handlers: HashMap<String, (f64, u32)>,
    counters: HashMap<&'static str, u64>,
    /// Native method -> (ms, calls, calls that changed the tree).
    natives: HashMap<String, (f64, u32, u32)>,
    /// Content changes by cause.
    changes: HashMap<String, u32>,
}

thread_local! {
    static ACC: RefCell<Acc> = RefCell::new(Acc::default());
}

/// Times its lifetime into the section `name`.
pub struct Scope {
    name: &'static str,
    t0: Option<Instant>,
}

impl Drop for Scope {
    fn drop(&mut self) {
        if let Some(t0) = self.t0 {
            add(self.name, t0.elapsed().as_secs_f64() * 1000.0);
        }
    }
}

pub fn scope(name: &'static str) -> Scope {
    Scope {
        name,
        t0: on().then(Instant::now),
    }
}

fn add(name: &'static str, ms: f64) {
    ACC.with(|a| {
        let mut a = a.borrow_mut();
        match a.sections.iter_mut().find(|(n, _)| *n == name) {
            Some(s) => s.1 += ms,
            None => a.sections.push((name, ms)),
        }
    });
}

/// Start time for [`handler_done`], when profiling.
pub fn handler_start() -> Option<Instant> {
    on().then(Instant::now)
}

/// A per-frame handler call took the time since `t0` (`key`: handler and widget name).
pub fn handler_done(t0: Option<Instant>, key: impl FnOnce() -> String) {
    let Some(t0) = t0 else { return };
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    ACC.with(|a| {
        let mut a = a.borrow_mut();
        let e = a.handlers.entry(key()).or_default();
        e.0 += ms;
        e.1 += 1;
    });
}

/// What made the frame's content change (`AA_UI_PROFILE`): a [`crate::tree::Tree::touch`] call
/// site or a changed object.
pub fn change(key: impl FnOnce() -> String) {
    if !on() {
        return;
    }
    let k = key();
    ACC.with(|a| *a.borrow_mut().changes.entry(k).or_default() += 1);
}

/// `AA_UI_PROFILE=3`: also log what changed in each changed object.
pub fn detail() -> bool {
    level() >= 3
}

/// Whether profiling is on.
pub fn enabled() -> bool {
    on()
}

/// Count `n` events at once.
pub fn add_count(name: &'static str, n: u64) {
    if !on() || n == 0 {
        return;
    }
    ACC.with(|a| *a.borrow_mut().counters.entry(name).or_default() += n);
}

/// Count an event (`rebuild`, ...).
pub fn count(name: &'static str) {
    if !on() {
        return;
    }
    ACC.with(|a| *a.borrow_mut().counters.entry(name).or_default() += 1);
}

/// With `AA_UI_PROFILE=2`: replace every native method of the class tables by a wrapper that
/// times it and notes tree changes. The wrappers are Lua closures (one Rust reference per
/// wrapped function would overflow mlua's reference stack).
pub fn wrap_natives(lua: &mlua::Lua) -> mlua::Result<()> {
    if level() < 2 {
        return Ok(());
    }
    let t0 = Instant::now();
    let clock = lua.create_function(move |_, ()| Ok(t0.elapsed().as_secs_f64() * 1000.0))?;
    let generation = lua.create_function(|lua, ()| Ok(crate::lua::ui(lua).tree.generation))?;
    let record = lua.create_function(|_, (name, ms, touched): (String, f64, bool)| {
        ACC.with(|a| {
            let mut a = a.borrow_mut();
            let e = a.natives.entry(name).or_default();
            e.0 += ms;
            e.1 += 1;
            e.2 += touched as u32;
        });
        Ok(())
    })?;
    let tables = lua.create_table()?;
    for mt in crate::lua::reg(lua).mt.values() {
        if let Ok(idx) = mt.raw_get::<mlua::Table>("__index") {
            tables.raw_push(idx)?;
        }
    }
    lua.load(WRAP_LUA)
        .set_name("@aa-ui/prof")
        .call::<()>((tables, clock, generation, record))
}

const WRAP_LUA: &str = r##"
local tables, clock, generation, record = ...
local select, unpack = select, unpack
local function pack(...) return { n = select("#", ...), ... } end
local seen = {}
for _, idx in ipairs(tables) do
  local names = {}
  if not seen[idx] then
    seen[idx] = true
    for name, f in pairs(idx) do
      if type(name) == "string" and type(f) == "function" then names[#names + 1] = name end
    end
  end
  for _, name in ipairs(names) do
    local f = idx[name]
    idx[name] = function(...)
      local g0, t0 = generation(), clock()
      local r = pack(f(...))
      record(name, clock() - t0, generation() ~= g0)
      return unpack(r, 1, r.n)
    end
  end
end
"##;

/// End of a UI frame: log the averages every 3 s.
pub fn frame_end() {
    if !on() {
        return;
    }
    ACC.with(|a| {
        let mut a = a.borrow_mut();
        a.frames += 1;
        let since = *a.since.get_or_insert_with(Instant::now);
        if since.elapsed().as_secs_f32() < 3.0 {
            return;
        }
        let n = a.frames.max(1) as f64;
        let parts: Vec<String> = a
            .sections
            .iter()
            .map(|(k, v)| format!("{k} {:.3}", v / n))
            .collect();
        let mut hs: Vec<(&String, &(f64, u32))> = a.handlers.iter().collect();
        hs.sort_by(|x, y| y.1.0.total_cmp(&x.1.0));
        let top: Vec<String> = hs
            .iter()
            .take(8)
            .map(|(k, (ms, c))| format!("{k} {:.3} ({:.1}/f)", ms / n, *c as f64 / n))
            .collect();
        let mut cs: Vec<String> = a
            .counters
            .iter()
            .map(|(k, v)| format!("{k} {:.2}/f", *v as f64 / n))
            .collect();
        cs.sort();
        bevy::log::info!(
            "aa-ui profile ({} frames, ms per frame): {} | {} | top handlers: {}",
            a.frames,
            parts.join(", "),
            cs.join(", "),
            top.join(", ")
        );
        if !a.changes.is_empty() {
            let mut ch: Vec<(&String, &u32)> = a.changes.iter().collect();
            ch.sort_by(|x, y| y.1.cmp(x.1));
            let list: Vec<String> = ch
                .iter()
                .take(24)
                .map(|(k, c)| format!("{k} {:.2}/f", **c as f64 / n))
                .collect();
            bevy::log::info!("aa-ui content changes: {}", list.join(", "));
        }
        if !a.natives.is_empty() {
            let mut ns: Vec<(&String, &(f64, u32, u32))> = a.natives.iter().collect();
            ns.sort_by(|x, y| y.1.0.total_cmp(&x.1.0));
            let top: Vec<String> = ns
                .iter()
                .take(16)
                .map(|(k, (ms, c, t))| {
                    format!(
                        "{k} {:.3} ({:.1}/f, {:.1} changed)",
                        ms / n,
                        *c as f64 / n,
                        *t as f64 / n
                    )
                })
                .collect();
            let mut ts: Vec<(&String, &(f64, u32, u32))> =
                a.natives.iter().filter(|(_, v)| v.2 > 0).collect();
            ts.sort_by_key(|x| std::cmp::Reverse(x.1.2));
            let touch: Vec<String> = ts
                .iter()
                .take(16)
                .map(|(k, (_, c, t))| format!("{k} {:.1}/{:.1}", *t as f64 / n, *c as f64 / n))
                .collect();
            bevy::log::info!("aa-ui natives (ms per frame): {}", top.join(", "));
            bevy::log::info!(
                "aa-ui natives that changed the tree (per frame / calls): {}",
                touch.join(", ")
            );
        }
        *a = Acc::default();
    });
}
