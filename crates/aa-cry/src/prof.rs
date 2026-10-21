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
use web_time::Instant;

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

/// A sampling profiler for hot spots without a profiler on the machine (Windows): every `interval` it suspends each thread whose
/// name starts with one of `threads`, reads its instruction pointer and resumes it; after `seconds` it prints the functions
/// most often caught (self time, flat), with their crate prefixes summed, through `log`. Needs the PDB next to the exe for
/// names; the Rust symbols are shortened by dropping the hash.
#[cfg(windows)]
pub fn sample_threads(threads: Vec<String>, delay: std::time::Duration, seconds: f32, interval: std::time::Duration, log: impl Fn(String) + Send + 'static) {
    use std::collections::HashMap;

    #[repr(C, align(16))]
    struct Context([u8; 1232]);

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
        fn GetLastError() -> u32;
        fn GetModuleHandleA(name: *const u8) -> isize;
        fn GetModuleFileNameA(module: isize, name: *mut u8, size: u32) -> u32;
        fn GetCurrentProcess() -> isize;
        fn CreateToolhelp32Snapshot(flags: u32, process: u32) -> isize;
        fn Thread32First(snapshot: isize, entry: *mut ThreadEntry) -> i32;
        fn Thread32Next(snapshot: isize, entry: *mut ThreadEntry) -> i32;
        fn OpenThread(access: u32, inherit: i32, thread_id: u32) -> isize;
        fn GetThreadDescription(thread: isize, description: *mut *mut u16) -> i32;
        fn LocalFree(mem: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
        fn SuspendThread(thread: isize) -> u32;
        fn ResumeThread(thread: isize) -> u32;
        fn GetThreadContext(thread: isize, context: *mut Context) -> i32;
        fn CloseHandle(handle: isize) -> i32;
    }

    #[link(name = "dbghelp")]
    unsafe extern "system" {
        fn SymInitialize(process: isize, search_path: *const u8, invade: i32) -> i32;
        fn SymSetOptions(options: u32) -> u32;
        fn SymFromAddr(process: isize, address: u64, displacement: *mut u64, symbol: *mut u8) -> i32;
    }

    /// `_ZN4core3fmt5write17h0123456789abcdefE` -> `core::fmt::write`; other names unchanged.
    fn shorten(name: &str) -> String {
        let Some(rest) = name.strip_prefix("_ZN") else { return name.to_string() };
        let mut out: Vec<&str> = Vec::new();
        let mut i = 0;
        let bytes = rest.as_bytes();
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            let mut n = 0usize;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                n = n * 10 + (bytes[i] - b'0') as usize;
                i += 1;
            }
            if i + n > rest.len() {
                break;
            }
            let seg = &rest[i..i + n];
            i += n;
            if !(seg.starts_with('h') && seg.len() == 17) {
                out.push(seg);
            }
        }
        if out.is_empty() { name.to_string() } else { out.join("::") }
    }

    // SAFETY (whole thread): Win32 calls with correctly sized structures; a suspended thread is resumed before anything that
    // could take a lock it holds (the sample buffer is preallocated)
    std::thread::Builder::new()
        .name("aa-sampler".into())
        .spawn(move || unsafe {
            std::thread::sleep(delay);
            let pid = GetCurrentProcessId();
            // the target threads
            let mut targets: Vec<(isize, String)> = Vec::new();
            let snapshot = CreateToolhelp32Snapshot(4, 0);
            let mut entry = ThreadEntry { size: std::mem::size_of::<ThreadEntry>() as u32, usage: 0, thread_id: 0, owner_process_id: 0, base_priority: 0, delta_priority: 0, flags: 0 };
            let mut ok = if snapshot == -1 { 0 } else { Thread32First(snapshot, &mut entry) };
            while ok != 0 {
                if entry.owner_process_id == pid {
                    // THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT | THREAD_QUERY_INFORMATION
                    let handle = OpenThread(0x0002 | 0x0008 | 0x0040, 0, entry.thread_id);
                    if handle != 0 {
                        let mut description: *mut u16 = std::ptr::null_mut();
                        let mut name = String::new();
                        if GetThreadDescription(handle, &mut description) >= 0 && !description.is_null() {
                            let mut len = 0;
                            while *description.add(len) != 0 {
                                len += 1;
                            }
                            name = String::from_utf16_lossy(std::slice::from_raw_parts(description, len));
                            LocalFree(description.cast());
                        }
                        if threads.iter().any(|t| name.starts_with(t.as_str())) {
                            targets.push((handle, name));
                        } else {
                            CloseHandle(handle);
                        }
                    }
                }
                entry.size = std::mem::size_of::<ThreadEntry>() as u32;
                ok = Thread32Next(snapshot, &mut entry);
            }
            if snapshot != -1 {
                CloseHandle(snapshot);
            }
            if targets.is_empty() {
                log(format!("aa-sampler: no thread named like {threads:?}"));
                return;
            }
            let rounds = (seconds / interval.as_secs_f32()).max(1.0) as usize;
            let mut samples: Vec<u64> = Vec::with_capacity(rounds * targets.len() + 16);
            let mut ctx = Context([0; 1232]);
            for _ in 0..rounds {
                for (handle, _) in &targets {
                    if SuspendThread(*handle) == u32::MAX {
                        continue;
                    }
                    ctx.0[0x30..0x34].copy_from_slice(&0x0010_0001u32.to_le_bytes()); // CONTEXT_CONTROL
                    if GetThreadContext(*handle, &mut ctx) != 0 {
                        let rip = u64::from_le_bytes(ctx.0[0xF8..0x100].try_into().unwrap());
                        samples.push(rip);
                    }
                    ResumeThread(*handle);
                }
                std::thread::sleep(interval);
            }
            for (handle, _) in &targets {
                CloseHandle(*handle);
            }
            // resolve the addresses
            let process = GetCurrentProcess();
            SymSetOptions(0x0000_0002 | 0x0000_0010); // SYMOPT_UNDNAME | SYMOPT_DEFERRED_LOADS
            // the executable's directory holds its PDB
            let exe = GetModuleHandleA(std::ptr::null());
            let mut path = [0u8; 520];
            let n = GetModuleFileNameA(exe, path.as_mut_ptr(), 519) as usize;
            let dir_end = path[..n].iter().rposition(|&c| c == b'\\').unwrap_or(0);
            let mut search = path[..dir_end].to_vec();
            search.push(0);
            if SymInitialize(process, search.as_ptr(), 1) == 0 {
                log(format!("aa-sampler: SymInitialize failed, error {}", GetLastError()));
            }
            let mut by_addr: HashMap<u64, u32> = HashMap::new();
            for s in &samples {
                *by_addr.entry(*s).or_default() += 1;
            }
            let mut by_name: HashMap<String, u32> = HashMap::new();
            let mut buffer = vec![0u8; 88 + 1024];
            let mut logged_error = false;
            for (addr, count) in by_addr {
                buffer.fill(0);
                buffer[0..4].copy_from_slice(&88u32.to_le_bytes());
                buffer[80..84].copy_from_slice(&1024u32.to_le_bytes());
                let mut displacement = 0u64;
                let name = if SymFromAddr(process, addr, &mut displacement, buffer.as_mut_ptr()) != 0 {
                    let len = u32::from_le_bytes(buffer[76..80].try_into().unwrap()).min(1023) as usize;
                    shorten(&String::from_utf8_lossy(&buffer[84..84 + len]))
                } else {
                    if !logged_error {
                        logged_error = true;
                        log(format!("aa-sampler: SymFromAddr({addr:#x}) failed, error {}", GetLastError()));
                    }
                    format!("{addr:#x}")
                };
                *by_name.entry(name).or_default() += count;
            }
            let total = samples.len().max(1) as f32;
            let mut rows: Vec<_> = by_name.into_iter().collect();
            rows.sort_by(|a, b| b.1.cmp(&a.1));
            log(format!("aa-sampler: {} samples of {:?} over {seconds} s; hottest functions (self):", samples.len(), targets.iter().map(|t| t.1.as_str()).collect::<Vec<_>>()));
            for (name, count) in rows.iter().take(45) {
                log(format!("aa-sampler:   {:5.1}%  {name}", *count as f32 * 100.0 / total));
            }
            let mut by_crate: HashMap<String, u32> = HashMap::new();
            for (name, count) in &rows {
                let krate = name.split("::").next().unwrap_or(name).to_string();
                *by_crate.entry(krate).or_default() += count;
            }
            let mut crates: Vec<_> = by_crate.into_iter().collect();
            crates.sort_by(|a, b| b.1.cmp(&a.1));
            let summary: Vec<String> = crates.iter().take(14).map(|(n, c)| format!("{n} {:.1}%", *c as f32 * 100.0 / total)).collect();
            log(format!("aa-sampler: by crate: {}", summary.join(", ")));
        })
        .ok();
}

#[cfg(not(windows))]
pub fn sample_threads(_threads: Vec<String>, _delay: std::time::Duration, _seconds: f32, _interval: std::time::Duration, _log: impl Fn(String) + Send + 'static) {}
