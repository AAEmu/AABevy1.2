//! What the log said, for unattended runs that must fail on a problem.
//!
//! [`layer`] is the `LogPlugin` custom layer: every WARN / ERROR record is looked at. Those that are a Lua runtime
//! error of the original UI scripts (the toolkit and the HUD log them as warnings, the game goes on) are kept in
//! [`lua_errors`], unless `--allow-lua-error <text>` (repeatable) names a text the message contains. The script
//! runner (`aa-client`'s `script`) and `--headless` runs use this for their exit code and summary.

use std::fmt::Write as _;
use std::sync::Mutex;

use bevy::log::tracing::{Event, Level, Subscriber, field::{Field, Visit}};
use bevy::log::tracing_subscriber::{Layer, layer::Context};
use bevy::log::BoxedLayer;
use bevy::prelude::App;

#[derive(Default)]
struct Captured {
    lua: Vec<String>,
    allowed: Vec<String>,
    allowed_seen: usize,
    warnings: usize,
    errors: usize,
}

static CAPTURED: Mutex<Captured> = Mutex::new(Captured { lua: Vec::new(), allowed: Vec::new(), allowed_seen: 0, warnings: 0, errors: 0 });

/// Text fragments that make a WARN / ERROR record a Lua error: the message of a failed script call.
const LUA_MARKERS: [&str; 4] = ["runtime error", "stack traceback", "scriptsbin/", "attempt to "];

pub fn layer(_app: &mut App) -> Option<BoxedLayer> {
    let args: Vec<String> = std::env::args().collect();
    let allowed = args.iter().enumerate().filter(|(_, a)| *a == "--allow-lua-error").filter_map(|(i, _)| args.get(i + 1).cloned()).collect();
    CAPTURED.lock().ok()?.allowed = allowed;
    Some(Box::new(CaptureLayer))
}

struct CaptureLayer;

struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.0, "{value:?}");
        }
    }
}

impl<S: Subscriber> Layer<S> for CaptureLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let level = *event.metadata().level();
        if level > Level::WARN {
            return;
        }
        let mut message = Message(String::new());
        event.record(&mut message);
        let text = message.0;
        let Ok(mut c) = CAPTURED.lock() else { return };
        if level == Level::ERROR {
            c.errors += 1;
        } else {
            c.warnings += 1;
        }
        if LUA_MARKERS.iter().any(|m| text.contains(m)) {
            if c.allowed.iter().any(|a| text.contains(a.as_str())) {
                c.allowed_seen += 1;
            } else if c.lua.len() < 200 {
                c.lua.push(text);
            }
        }
    }
}

/// The Lua errors logged so far (not allowed by `--allow-lua-error`).
pub fn lua_errors() -> Vec<String> {
    CAPTURED.lock().map(|c| c.lua.clone()).unwrap_or_default()
}

/// How many WARN and ERROR records there were, and how many Lua errors `--allow-lua-error` let through.
pub fn counts() -> (usize, usize, usize) {
    CAPTURED.lock().map(|c| (c.warnings, c.errors, c.allowed_seen)).unwrap_or_default()
}
