//! Which stubbed or faked game calls the original scripts hit (`AA_STUB_REPORT=1`).
//!
//! Two things stand in for natives the client has not implemented: aa-ui's permissive stubs (a known
//! method that does nothing, counted in [`UiRuntime::stub_calls`]) and the **FALLBACK** fakes
//! (`aa-ui/examples/fake/fake_*.lua`, installed first and overridden function by function by the real
//! bindings). With the environment variable set, every such method logs one `stub-report:` line the
//! first time a script calls it, and a summary with the call counts follows at exit. Without it nothing
//! is wrapped and nothing is polled.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Mutex;

use aa_ui::UiRuntime;
use bevy::app::AppExit;
use bevy::prelude::*;
use mlua::{Function, Lua, Table, Value};

/// `AA_STUB_REPORT` is set (and not `0`).
pub fn enabled() -> bool {
    std::env::var("AA_STUB_REPORT").is_ok_and(|v| !v.is_empty() && v != "0")
}

/// (global table, method) -> the function there, for the `X2*` objects.
pub type Methods = HashMap<(String, String), usize>;

/// Calls of the faked methods, by `X2Unit:GetFoo`.
static FAKE_HITS: Mutex<BTreeMap<String, u32>> = Mutex::new(BTreeMap::new());

fn api_tables(lua: &Lua) -> Vec<(String, Table)> {
    let mut out = Vec::new();
    for (k, v) in lua.globals().pairs::<String, Value>().flatten() {
        if let Value::Table(t) = v
            && k.starts_with("X2")
        {
            out.push((k, t));
        }
    }
    out
}

/// The functions of the `X2*` objects now.
pub fn snapshot(lua: &Lua) -> Methods {
    if !enabled() {
        return Methods::new();
    }
    methods(lua)
}

fn methods(lua: &Lua) -> Methods {
    let mut out = Methods::new();
    for (name, table) in api_tables(lua) {
        for (k, v) in table.pairs::<String, Value>().flatten() {
            if let Value::Function(f) = v {
                out.insert((name.clone(), k), f.to_pointer() as usize);
            }
        }
    }
    out
}

/// Wrap every method the fake files installed (`fakes` differs from `before`) that nothing replaced since, so its
/// first call is logged. `before`: the objects before the fake files, `fakes`: right after them.
pub fn instrument(lua: &Lua, before: &Methods, fakes: &Methods) -> mlua::Result<()> {
    if !enabled() {
        return Ok(());
    }
    wrap_fakes(lua, before, fakes)
}

fn wrap_fakes(lua: &Lua, before: &Methods, fakes: &Methods) -> mlua::Result<()> {
    let hit = lua.create_function(|_, name: String| {
        let mut hits = FAKE_HITS.lock().unwrap_or_else(|e| e.into_inner());
        let n = hits.entry(name.clone()).or_insert(0);
        if *n == 0 {
            info!("stub-report: fake {name}");
        }
        *n += 1;
        Ok(())
    })?;
    let wrap: Function = lua.load("local hit, orig, name = ... return function(...) hit(name) return orig(...) end").into_function()?;
    let mut live = 0;
    for (name, table) in api_tables(lua) {
        for (k, v) in table.pairs::<String, Value>().flatten() {
            let Value::Function(f) = v else { continue };
            let key = (name.clone(), k.clone());
            let now = f.to_pointer() as usize;
            if fakes.get(&key) == Some(&now) && before.get(&key) != Some(&now) {
                let wrapped: Function = wrap.call((hit.clone(), f, format!("{name}:{k}")))?;
                table.raw_set(k, wrapped)?;
                live += 1;
            }
        }
    }
    info!("stub-report: {live} faked methods are still live (FALLBACK fake_*.lua), logged on their first call");
    Ok(())
}

/// A summary of what was hit so far, one method per line.
pub fn summary(native: &[(String, u32)]) -> String {
    let hits = FAKE_HITS.lock().unwrap_or_else(|e| e.into_inner());
    let mut out = format!("stub-report: {} faked methods hit, {} native stubs hit", hits.len(), native.len());
    for (name, n) in hits.iter() {
        out.push_str(&format!("\n  fake   {name} x{n}"));
    }
    for (name, n) in native {
        out.push_str(&format!("\n  native {name} x{n}"));
    }
    out
}

/// Logs the native stubs as they are first hit (every second) and the summary at exit.
fn poll(
    rt: Option<NonSend<UiRuntime>>,
    time: Res<Time>,
    mut seen: Local<HashSet<String>>,
    mut last: Local<(f32, Vec<(String, u32)>)>,
    mut exit: MessageReader<AppExit>,
) {
    let Some(rt) = rt else { return };
    if time.elapsed_secs() - last.0 >= 1.0 {
        last.0 = time.elapsed_secs();
        last.1 = rt.stub_calls();
        for (name, _) in &last.1 {
            if seen.insert(name.clone()) {
                info!("stub-report: native stub {name}");
            }
        }
    }
    if exit.read().next().is_some() {
        info!("{}", summary(&rt.stub_calls()));
    }
}

pub fn plugin(app: &mut App) {
    if enabled() {
        app.add_systems(Update, poll);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapped_fake_logs_through_the_hit_table() -> mlua::Result<()> {
        let lua = Lua::new();
        lua.load("X2Fake = { Real = function() return 1 end }").exec()?;
        let before = methods(&lua);
        lua.load("X2Fake.Faked = function(self, a) return a + 1 end; X2Fake.Real = function() return 2 end").exec()?;
        let fakes = methods(&lua);
        lua.load("X2Fake.Real = function() return 3 end").exec()?;
        wrap_fakes(&lua, &before, &fakes)?;
        let (faked, real): (i64, i64) = lua.load("return X2Fake:Faked(4), X2Fake:Real()").eval()?;
        assert_eq!((faked, real), (5, 3));
        let hits = FAKE_HITS.lock().unwrap();
        assert_eq!(hits.get("X2Fake:Faked"), Some(&1));
        assert!(!hits.contains_key("X2Fake:Real"));
        Ok(())
    }
}
