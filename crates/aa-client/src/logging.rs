//! The client's file log: everything the console shows, also written to a file.
//!
//! The path is `%LOCALAPPDATA%/AABevy1.2/logs/aa-client-YYYYMMDD-HHMMSS.log` (UTC), or `logs/` next to the
//! executable when `LOCALAPPDATA` is missing. `AA_LOG_FILE=<file>` picks an exact file and
//! `AA_LOG_FILE=off` disables the file (the console log stays). The last [`KEEP`] files are kept.
//!
//! [`init`] opens the file before [`bevy::log::LogPlugin`] is built, [`layer`] is the plugin's custom layer:
//! it wraps the dev harness' Lua-error capture layer ([`aa_dev::capture::layer`]) and adds the file writer.
//! A panic hook writes the panic message and location into the same file before the default hook runs.

use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bevy::log::tracing_subscriber::fmt::{self, writer::MakeWriter};
use bevy::log::{BoxedLayer, info};
use bevy::prelude::App;

/// Environment variable with the exact log file, or `off`.
const ENV_LOG_FILE: &str = "AA_LOG_FILE";
/// Log files kept at startup (including the one being started).
const KEEP: usize = 10;
/// A log written to within this is never pruned: another running client may own it.
const RECENT: Duration = Duration::from_secs(60 * 60);
/// Folder under `%LOCALAPPDATA%`.
const APP_DIR: &str = "AABevy1.2";

struct OpenLog {
    path: PathBuf,
    file: Arc<Mutex<File>>,
}

static LOG: RwLock<Option<OpenLog>> = RwLock::new(None);

/// Open the log file, prune old logs and install the panic hook. Call once, before the app is built.
pub fn init() {
    // The browser build has no file system (and `SystemTime::now` panics there): console only.
    if cfg!(target_arch = "wasm32") {
        return;
    }
    let stamp = timestamp(SystemTime::now());
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    let local_app_data = std::env::var_os("LOCALAPPDATA");
    let override_var = std::env::var_os(ENV_LOG_FILE);
    let Some(choice) = choose(
        override_var.as_deref(),
        local_app_data.as_deref(),
        &exe_dir,
        &stamp,
    ) else {
        install_panic_hook();
        return;
    };

    let (path, file, prune_logs) = match open(&choice.path) {
        Ok(file) => (choice.path, file, choice.prune),
        Err(error) => {
            // Users may not have rights to the chosen folder; still write a log next to the executable.
            let fallback = exe_dir.join("logs").join(format!("aa-client-{stamp}.log"));
            eprintln!(
                "aa-client: cannot open log file {}: {error}; using {}",
                choice.path.display(),
                fallback.display()
            );
            match open(&fallback) {
                Ok(file) => (fallback, file, true),
                Err(error) => {
                    eprintln!(
                        "aa-client: cannot open log file {}: {error}",
                        fallback.display()
                    );
                    install_panic_hook();
                    return;
                }
            }
        }
    };

    if prune_logs
        && let Some(dir) = path.parent()
    {
        prune(dir, KEEP, RECENT);
    }
    if let Ok(mut slot) = LOG.write() {
        *slot = Some(OpenLog { path, file: Arc::new(Mutex::new(file)) });
    }
    install_panic_hook();
}

/// The path of the active log file, if file logging is on.
pub fn path() -> Option<PathBuf> {
    LOG.read().ok()?.as_ref().map(|log| log.path.clone())
}

/// Log where the file is; called once the tracing subscriber is up.
pub fn startup_line() {
    match path() {
        Some(path) => info!(target: "aa_client::log", "log file: {}", path.display()),
        None => info!(target: "aa_client::log", "file log off (AA_LOG_FILE=off)"),
    }
}

/// `LogPlugin::custom_layer`: the dev harness' capture layer plus this run's file layer.
pub fn layer(app: &mut App) -> Option<BoxedLayer> {
    let capture = aa_dev::capture::layer(app);
    let file = LOG
        .read()
        .ok()
        .and_then(|log| log.as_ref().map(|log| file_layer(log.file.clone())));
    match (capture, file) {
        (Some(capture), Some(file)) => Some(Box::new(vec![capture, file])),
        (Some(capture), None) => Some(capture),
        (None, Some(file)) => Some(file),
        (None, None) => None,
    }
}

fn file_layer(file: Arc<Mutex<File>>) -> BoxedLayer {
    Box::new(
        fmt::Layer::default()
            .with_ansi(false)
            .with_writer(FileWriter(file)),
    )
}

/// A warning that the file could not be opened still goes somewhere.
fn open(path: &Path) -> io::Result<File> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    OpenOptions::new().create(true).append(true).open(path)
}

/// The file to write and (for the default location) the folder to prune.
struct Choice {
    path: PathBuf,
    prune: bool,
}

/// Path selection, separated from the environment for the tests.
fn choose(
    override_var: Option<&OsStr>,
    local_app_data: Option<&OsStr>,
    exe_dir: &Path,
    stamp: &str,
) -> Option<Choice> {
    if let Some(value) = override_var.filter(|v| !v.is_empty()) {
        if value.to_str().is_some_and(|v| v.eq_ignore_ascii_case("off")) {
            return None;
        }
        return Some(Choice { path: PathBuf::from(value), prune: false });
    }
    let dir = match local_app_data.filter(|base| !base.is_empty()) {
        Some(base) => Path::new(base).join(APP_DIR).join("logs"),
        None => exe_dir.join("logs"),
    };
    Some(Choice { path: dir.join(format!("aa-client-{stamp}.log")), prune: true })
}

/// `YYYYMMDD-HHMMSS` in UTC.
fn timestamp(now: SystemTime) -> String {
    let secs = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        rem / 3600,
        (rem / 60) % 60,
        rem % 60
    )
}

/// Days since 1970-01-01 -> (year, month, day). Howard Hinnant's `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// Delete all but the newest `keep` `aa-client-*.log` files. Names carry the timestamp, so they sort by age.
fn prune(dir: &Path, keep: usize, recent: Duration) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let mut logs: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter(|entry| is_log_name(&entry.file_name()))
        // another client started within `recent` may still be writing to it
        .filter(|entry| !written_within(entry, recent))
        .map(|entry| entry.path())
        .collect();
    if logs.len() <= keep {
        return;
    }
    logs.sort();
    for old in &logs[..logs.len() - keep] {
        let _ = fs::remove_file(old);
    }
}

fn written_within(entry: &fs::DirEntry, age: Duration) -> bool {
    entry
        .metadata()
        .and_then(|meta| meta.modified())
        .is_ok_and(|modified| modified.elapsed().is_ok_and(|elapsed| elapsed < age))
}

fn is_log_name(name: &OsStr) -> bool {
    let name = name.to_string_lossy();
    name.starts_with("aa-client-") && name.ends_with(".log")
}

fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        write_panic(panic);
        previous(panic);
    }));
}

fn write_panic(panic: &std::panic::PanicHookInfo<'_>) {
    let Some(log) = LOG.read().ok().and_then(|log| log.as_ref().map(|log| log.file.clone())) else {
        return;
    };
    // A panic while an event is being formatted holds the lock; don't deadlock the hook.
    let Ok(mut file) = log.try_lock() else { return };
    let message = panic
        .payload()
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.payload().downcast_ref::<String>().map(String::as_str))
        .unwrap_or("<non-string panic payload>");
    let location = panic
        .location()
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_else(|| "unknown location".to_string());
    let thread = std::thread::current();
    let thread = thread.name().unwrap_or("<unnamed>");
    let _ = writeln!(file, "[panic] {thread} at {location}: {message}");
    let _ = file.flush();
}

#[derive(Clone)]
struct FileWriter(Arc<Mutex<File>>);

impl<'a> MakeWriter<'a> for FileWriter {
    type Writer = FileGuard<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        FileGuard(self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
    }
}

/// The `MutexGuard` is held while the whole event is formatted, so lines from different threads don't mix.
struct FileGuard<'a>(MutexGuard<'a, File>);

impl Write for FileGuard<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.0.write_all(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use bevy::log::tracing;
    use bevy::log::tracing_subscriber::{self, layer::SubscriberExt};

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "aa-client-log-{}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn timestamp_is_utc_yyyymmdd_hhmmss() {
        let at = UNIX_EPOCH + Duration::from_secs(1_704_164_645); // 2024-01-02 03:04:05 UTC
        assert_eq!(timestamp(at), "20240102-030405");
    }

    #[test]
    fn chooses_the_local_app_data_folder() {
        let choice = choose(
            None,
            Some(OsStr::new(r"C:\Users\me\AppData\Local")),
            Path::new(r"C:\game"),
            "20240102-030405",
        )
        .unwrap();
        assert_eq!(
            choice.path,
            PathBuf::from(r"C:\Users\me\AppData\Local\AABevy1.2\logs\aa-client-20240102-030405.log")
        );
        assert!(choice.prune);
    }

    #[test]
    fn falls_back_to_the_folder_next_to_the_exe() {
        for local in [None, Some(OsStr::new(""))] {
            let choice = choose(None, local, Path::new(r"C:\game\bin"), "20240102-030405");
            let choice = choice.unwrap();
            assert_eq!(
                choice.path,
                PathBuf::from(r"C:\game\bin\logs\aa-client-20240102-030405.log")
            );
            assert!(choice.prune);
        }
    }

    #[test]
    fn env_var_picks_an_exact_file() {
        let choice = choose(
            Some(OsStr::new(r"D:\logs\player.log")),
            Some(OsStr::new(r"C:\Users\me\AppData\Local")),
            Path::new(r"C:\game"),
            "20240102-030405",
        )
        .unwrap();
        assert_eq!(choice.path, PathBuf::from(r"D:\logs\player.log"));
        assert!(!choice.prune);
    }

    #[test]
    fn env_var_off_disables_the_file() {
        assert!(choose(Some(OsStr::new("off")), None, Path::new("."), "x").is_none());
        assert!(choose(Some(OsStr::new("OFF")), None, Path::new("."), "x").is_none());
    }

    #[test]
    fn empty_env_var_uses_the_default() {
        let choice = choose(
            Some(OsStr::new("")),
            Some(OsStr::new(r"C:\Users\me\AppData\Local")),
            Path::new(r"C:\game"),
            "20240102-030405",
        )
        .unwrap();
        assert!(choice.path.ends_with("AABevy1.2/logs/aa-client-20240102-030405.log"));
    }

    #[test]
    fn prunes_all_but_the_newest_logs() {
        let dir = temp_dir("prune");
        for day in 1..=12 {
            fs::write(dir.join(format!("aa-client-202401{day:02}-000000.log")), b"x").unwrap();
        }
        fs::write(dir.join("notes.log"), b"keep me").unwrap();
        fs::write(dir.join("aa-client-20240113-000000.txt"), b"keep me too").unwrap();

        prune(&dir, KEEP, Duration::ZERO);

        let mut logs: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("aa-client-") && name.ends_with(".log"))
            .collect();
        logs.sort();
        assert_eq!(logs.len(), KEEP);
        assert_eq!(logs.first().unwrap(), "aa-client-20240103-000000.log");
        assert!(dir.join("notes.log").exists());
        assert!(dir.join("aa-client-20240113-000000.txt").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_tracing_event_lands_in_the_file() {
        let dir = temp_dir("event");
        let path = dir.join("aa-client-20240102-030405.log");
        let file = open(&path).unwrap();
        let subscriber = tracing_subscriber::registry().with(file_layer(Arc::new(Mutex::new(file))));

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: "aa_client::log_test", "hello from the file log test");
        });

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("hello from the file log test"), "{text}");
        assert!(text.contains("aa_client::log_test"), "{text}");
        let _ = fs::remove_dir_all(&dir);
    }
}
