//! Coarse scope timers for profiling (`AA_PROFILE=1`, see `aa_world::profile`).
//!
//! Off unless [`enable`] was called: a scope then costs one relaxed atomic load. Scopes may nest and run on any thread;
//! each name collects the summed wall time, the call count, the longest call and the summed thread CPU time (Windows: the
//! thread's cycle counter, which does not grow while the thread is descheduled, so it stays comparable between runs on a
//! busy machine).
//!
//! ```
//! aa_cry::prof::enable(true);
//! {
//!     let _t = aa_cry::prof::scope("example");
//! }
//! assert_eq!(aa_cry::prof::take()["example"].calls, 1);
//! # aa_cry::prof::enable(false);
//! ```

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

static ENABLED: AtomicBool = AtomicBool::new(false);
/// Thread cycles per millisecond (f64 bits), measured by [`enable`].
static CYCLES_PER_MS: AtomicU64 = AtomicU64::new(0);
static TIMES: Mutex<BTreeMap<&'static str, Stat>> = Mutex::new(BTreeMap::new());

/// What one name collected.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stat {
    /// Summed wall time (ms).
    pub wall: f64,
    pub calls: u32,
    /// Longest call, wall time (ms).
    pub max: f64,
    /// Summed thread CPU time (ms).
    pub cpu: f64,
}

/// CPU cycles the calling thread has run (0 off Windows).
#[cfg(windows)]
pub fn thread_cycles() -> u64 {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThread() -> isize;
        fn QueryThreadCycleTime(thread: isize, cycles: *mut u64) -> i32;
    }
    let mut cycles = 0u64;
    // SAFETY: plain Win32 calls with a valid out pointer
    unsafe { QueryThreadCycleTime(GetCurrentThread(), &mut cycles) };
    cycles
}

#[cfg(not(windows))]
pub fn thread_cycles() -> u64 {
    0
}

/// CPU cycles this process has used on all its threads (0 off Windows). Unlike wall time it does not grow when other
/// programs compete for the cores.
#[cfg(windows)]
pub fn process_cycles() -> u64 {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> isize;
        fn QueryProcessCycleTime(process: isize, cycles: *mut u64) -> i32;
    }
    let mut cycles = 0u64;
    // SAFETY: plain Win32 calls with a valid out pointer
    unsafe { QueryProcessCycleTime(GetCurrentProcess(), &mut cycles) };
    cycles
}

#[cfg(not(windows))]
pub fn process_cycles() -> u64 {
    0
}

/// Every thread of the process with its name and the cycles it has run (Windows; empty elsewhere).
#[cfg(windows)]
pub fn all_thread_cycles() -> Vec<(u32, String, u64)> {
    use std::ffi::c_void;

    #[repr(C)]
    struct ThreadEntry {
        size: u32,
        usage: u32,
        thread_id: u32,
        owner_process_id: u32,
        base_priority: i32,
        delta_priority: i32,
        flags: u32,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcessId() -> u32;
        fn CreateToolhelp32Snapshot(flags: u32, process: u32) -> isize;
        fn Thread32First(snapshot: isize, entry: *mut ThreadEntry) -> i32;
        fn Thread32Next(snapshot: isize, entry: *mut ThreadEntry) -> i32;
        fn OpenThread(access: u32, inherit: i32, thread_id: u32) -> isize;
        fn QueryThreadCycleTime(thread: isize, cycles: *mut u64) -> i32;
        fn GetThreadDescription(thread: isize, description: *mut *mut u16) -> i32;
        fn LocalFree(mem: *mut c_void) -> *mut c_void;
        fn CloseHandle(handle: isize) -> i32;
    }

    let mut out = Vec::new();
    // SAFETY: Win32 enumeration with correctly sized structs; every handle and the description buffer are released
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(4, 0);
        if snapshot == -1 {
            return out;
        }
        let pid = GetCurrentProcessId();
        let mut entry = ThreadEntry { size: std::mem::size_of::<ThreadEntry>() as u32, usage: 0, thread_id: 0, owner_process_id: 0, base_priority: 0, delta_priority: 0, flags: 0 };
        let mut ok = Thread32First(snapshot, &mut entry);
        while ok != 0 {
            if entry.owner_process_id == pid {
                // THREAD_QUERY_LIMITED_INFORMATION
                let handle = OpenThread(0x0800, 0, entry.thread_id);
                if handle != 0 {
                    let mut cycles = 0u64;
                    QueryThreadCycleTime(handle, &mut cycles);
                    let mut name = String::new();
                    let mut description: *mut u16 = std::ptr::null_mut();
                    if GetThreadDescription(handle, &mut description) >= 0 && !description.is_null() {
                        let mut len = 0;
                        while *description.add(len) != 0 {
                            len += 1;
                        }
                        name = String::from_utf16_lossy(std::slice::from_raw_parts(description, len));
                        LocalFree(description.cast());
                    }
                    CloseHandle(handle);
                    out.push((entry.thread_id, name, cycles));
                }
            }
            entry.size = std::mem::size_of::<ThreadEntry>() as u32;
            ok = Thread32Next(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
    }
    out
}

#[cfg(not(windows))]
pub fn all_thread_cycles() -> Vec<(u32, String, u64)> {
    Vec::new()
}

/// Turns the timers on or off. Turning them on measures the clock rate (about 40 ms).
pub fn enable(on: bool) {
    if on && CYCLES_PER_MS.load(Ordering::Relaxed) == 0 {
        // the best of a few short spins: the one the scheduler did not interrupt
        let mut best = 0.0f64;
        for _ in 0..5 {
            let (t0, c0) = (Instant::now(), thread_cycles());
            while t0.elapsed().as_micros() < 8000 {
                std::hint::spin_loop();
            }
            let ms = t0.elapsed().as_secs_f64() * 1000.0;
            best = best.max((thread_cycles().saturating_sub(c0)) as f64 / ms);
        }
        CYCLES_PER_MS.store(best.to_bits(), Ordering::Relaxed);
    }
    ENABLED.store(on, Ordering::Relaxed);
}

/// Millions of cycles per second the clock calibration measured (0 off Windows).
pub fn mhz() -> f64 {
    f64::from_bits(CYCLES_PER_MS.load(Ordering::Relaxed)) / 1000.0
}

/// Times until dropped.
pub struct Scope {
    name: &'static str,
    start: Option<(Instant, u64)>,
}

/// Starts timing `name` (ends when the guard is dropped).
pub fn scope(name: &'static str) -> Scope {
    Scope { name, start: ENABLED.load(Ordering::Relaxed).then(|| (Instant::now(), thread_cycles())) }
}

/// Adds a measurement made elsewhere (`wall_ms`, thread CPU `cpu_ms`).
pub fn add(name: &'static str, wall_ms: f64, cpu_ms: f64) {
    let mut times = TIMES.lock().unwrap_or_else(|e| e.into_inner());
    let e = times.entry(name).or_default();
    e.wall += wall_ms;
    e.calls += 1;
    e.max = e.max.max(wall_ms);
    e.cpu += cpu_ms;
}

impl Drop for Scope {
    fn drop(&mut self) {
        if let Some((start, c0)) = self.start {
            let wall = start.elapsed().as_secs_f64() * 1000.0;
            let per_ms = f64::from_bits(CYCLES_PER_MS.load(Ordering::Relaxed));
            let cpu = if per_ms > 0.0 { thread_cycles().saturating_sub(c0) as f64 / per_ms } else { wall };
            add(self.name, wall, cpu);
        }
    }
}

/// Everything collected since the last call.
pub fn take() -> BTreeMap<&'static str, Stat> {
    std::mem::take(&mut *TIMES.lock().unwrap_or_else(|e| e.into_inner()))
}
