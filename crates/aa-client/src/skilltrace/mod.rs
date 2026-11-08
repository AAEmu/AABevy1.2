//! `AA_SKILL_TRACE=<path.jsonl>`: a structured trace of what the client does during skills, one JSON object per line, in the
//! format both clients write for the skill parity audit (`tools/skillparity/README.md`, schema `skillparity/v1`). The notes are
//! in `docs/aa-client-combat.md`, "Skill trace".
//!
//! Off unless the variable is set. Unset, [`SkillTracePlugin`] adds nothing and every hook is one relaxed atomic load
//! ([`enabled`]). Set, it opens the file (replacing it), turns on the opt-in records of aa-anim (clip starts) and aa-audio
//! (sounds asked for), and adds the systems that turn the game's state into rows:
//!
//! | row | hook |
//! |---|---|
//! | `anim` | `aa_anim::Animator` clip starts, drained after the animation pass ([`world::drain_anims`]) |
//! | `fx` | an `aa_fx::Effect` / `ChrEffect` that appears ([`world::fx_started`]) |
//! | `sound` | `aa_audio::Engine::play_with` ([`world::sounds`]) |
//! | `net` | `NetCommand` / `NetEvent` messages ([`world::net_rows`], [`net`]) |
//! | `castbar`, `cooldown` | the HUD's state ([`crate::hud::skill_trace`]) |
//! | `press` | `hud::panels::action_keys`, for a key and for a scripted slot ([`press`]) |
//! | `pos` | the player's and the target's world transform ([`world::positions`]) |
//! | `frame` | the real time of the frame ([`world::begin_frame`]) |
//! | `mark` | the script runner's `trace TEXT` and `echo TEXT` ([`mark`]) |
//!
//! Rows are buffered and written every 250 ms and when the app exits. Rows of one frame are not sorted by time across kinds
//! (`anim` and `sound` carry the moment they were asked for, the others the moment they were noticed); sort by `wall_us`.

mod net;
pub mod rows;
mod world;

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use bevy::app::AppExit;
use bevy::prelude::*;
use web_time::{Instant, SystemTime, UNIX_EPOCH};

pub use rows::Row;

static ON: AtomicBool = AtomicBool::new(false);
static FRAME: AtomicU32 = AtomicU32::new(0);
static SINK: Mutex<Option<Sink>> = Mutex::new(None);

/// How often the buffered rows are written out.
const FLUSH_EVERY: Duration = Duration::from_millis(250);

struct Sink {
    out: BufWriter<File>,
    flushed: Instant,
}

/// Whether the trace is on (the variable was set and the file opened).
#[inline]
pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Unix time in microseconds from the OS wall clock, the clock every row uses.
pub fn wall_us() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_micros() as u64)
}

/// Writes a row of `kind` stamped with now and the current frame; `build` adds the fields. Does nothing when the trace is off.
pub fn emit(kind: &str, build: impl FnOnce(Row) -> Row) {
    if enabled() {
        emit_at(kind, wall_us(), build);
    }
}

/// [`emit`] with the moment the event happened, for rows noticed a little after.
pub fn emit_at(kind: &str, wall_us: u64, build: impl FnOnce(Row) -> Row) {
    if enabled() {
        write(&build(Row::new(kind, wall_us, FRAME.load(Ordering::Relaxed))).line());
    }
}

fn write(line: &str) {
    let mut sink = SINK.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(s) = sink.as_mut() {
        // a full disk must not stop the game: the rest of the trace is lost
        let _ = s.out.write_all(line.as_bytes()).and_then(|()| s.out.write_all(b"\n"));
    }
}

/// Writes the buffered rows out.
pub fn flush() {
    if let Some(s) = SINK.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        let _ = s.out.flush();
        s.flushed = Instant::now();
    }
}

fn flush_due() {
    let due = SINK.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|s| s.flushed.elapsed() >= FLUSH_EVERY);
    if due {
        flush();
    }
}

/// The action bar key `slot` (1..=12, the display slot of the current page) was pressed and the HUD takes it. `key` is the
/// default binding of that slot: `"1"`..`"9"`, `"0"`, `"-"`, `"="`.
pub fn press(slot: usize) {
    const KEYS: [&str; 12] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0", "-", "="];
    if enabled() && (1..=KEYS.len()).contains(&slot) {
        emit("press", |r| r.str("key", KEYS[slot - 1]).int("slot", slot as i64));
    }
}

/// A label from the script runner (`trace TEXT`, `echo TEXT`); `skill:<id>:...` labels also carry the skill id.
pub fn mark(label: &str) {
    if enabled() {
        emit("mark", |r| r.str("label", label).opt_int("skill_id", rows::skill_of_label(label).map(i64::from)));
    }
}

/// Adds the trace when `AA_SKILL_TRACE` names a file. Add once.
pub struct SkillTracePlugin;

/// Opens `path` (replacing it) and turns the records and the writer on.
fn start(path: &Path) -> std::io::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let file = File::create(path)?;
    *SINK.lock().unwrap_or_else(|e| e.into_inner()) = Some(Sink { out: BufWriter::with_capacity(64 * 1024, file), flushed: Instant::now() });
    aa_anim::trace::enable();
    aa_audio::trace::enable();
    ON.store(true, Ordering::Relaxed);
    Ok(())
}

impl Plugin for SkillTracePlugin {
    fn build(&self, app: &mut App) {
        let Some(path) = std::env::var_os("AA_SKILL_TRACE").filter(|p| !p.is_empty()) else { return };
        let path = PathBuf::from(path);
        if let Err(e) = start(&path) {
            warn!("skilltrace: cannot create {}: {e}", path.display());
            return;
        }
        info!("skilltrace: writing {}", path.display());
        app.init_resource::<world::Roles>()
            .add_systems(PreUpdate, world::begin_frame)
            .add_systems(Update, world::net_rows)
            .add_systems(PostUpdate, world::drain_anims.after(aa_anim::AnimSet))
            .add_systems(
                Last,
                (
                    world::fx_started,
                    world::sounds,
                    world::positions,
                    crate::hud::skill_trace::castbar,
                    crate::hud::skill_trace::cooldowns,
                    flush_due,
                    flush.run_if(on_message::<AppExit>),
                )
                    .chain(),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The writer end to end: the variable's file is created, rows from the hooks land in it as JSON lines, and a flush writes them.
    /// (Turns the trace on for the rest of this test binary; the other tests only emit into the file.)
    #[test]
    fn the_hooks_write_json_lines_to_the_file() {
        let dir = std::env::temp_dir().join(format!("aa-skilltrace-test-{}", std::process::id()));
        let path = dir.join("nested").join("trace.jsonl");
        start(&path).expect("the file and its folder are created");
        assert!(enabled() && aa_anim::trace::enabled() && aa_audio::trace::enabled());
        FRAME.store(7, Ordering::Relaxed);
        mark("skill:14376:start");
        mark("plain label");
        press(3);
        press(12);
        press(0);
        press(13);
        emit("frame", |r| r.num("dt_ms", 16.667, 2));
        flush();
        let text = std::fs::read_to_string(&path).expect("the trace file");
        let rows: Vec<serde_json::Value> = text.lines().map(|l| serde_json::from_str(l).expect("a JSON object per line")).collect();
        let of = |kind: &str| rows.iter().filter(|r| r["kind"] == kind).collect::<Vec<_>>();
        for r in &rows {
            assert_eq!(r["src"], "bevy");
            assert!(r["wall_us"].as_u64().is_some_and(|t| t > 1_600_000_000_000_000), "Unix microseconds: {r}");
            assert!(r["frame"].is_u64());
        }
        let skill = of("mark").into_iter().find(|r| r["label"] == "skill:14376:start").expect("the skill mark");
        assert_eq!(skill["skill_id"], 14376);
        assert!(of("mark").into_iter().find(|r| r["label"] == "plain label").expect("the plain mark").get("skill_id").is_none());
        let presses: Vec<_> = of("press").into_iter().map(|r| (r["slot"].as_u64().unwrap(), r["key"].as_str().unwrap().to_string())).collect();
        assert!(presses.contains(&(3, "3".into())) && presses.contains(&(12, "=".into())), "{presses:?}");
        assert!(presses.iter().all(|(slot, _)| (1..=12).contains(slot)), "only the twelve bar keys press: {presses:?}");
        assert_eq!(of("frame").into_iter().find(|r| r["dt_ms"] == 16.67).expect("the frame row")["frame"], 7);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
