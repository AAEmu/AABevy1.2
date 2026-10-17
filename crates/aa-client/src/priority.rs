//! The client runs at above normal priority (Windows).
//!
//! A frame hands work between the main thread, the render thread and the task pools dozens of times. When every core is
//! busy (on this machine: other sessions' builds and clients, `docs/research/perf.md`), each hand-off waits for a scheduler
//! quantum at normal priority, and the frames go from 13 ms to 100-400 ms for seconds at a time while the process does
//! the same work per frame. At `ABOVE_NORMAL_PRIORITY_CLASS` the game's threads get the cores first: at the Nuian spawn
//! with the CPU at 100 % (3 alternating rounds) 41 / 28 / 57 fps at normal priority, 68 / 76 / 82 above it, 1 % lows
//! 4-6 against 17-59 fps. `AA_PRIORITY=normal` keeps the normal priority.

use bevy::prelude::*;

/// Startup: raises the process' priority class (once).
pub fn raise() {
    if std::env::var("AA_PRIORITY").is_ok_and(|v| v.eq_ignore_ascii_case("normal")) {
        info!("priority: normal (AA_PRIORITY=normal)");
        return;
    }
    match above_normal() {
        Ok(()) => info!("priority: above normal (AA_PRIORITY=normal keeps the default)"),
        Err(code) => warn!("priority: could not raise the priority class (error {code})"),
    }
}

#[cfg(windows)]
fn above_normal() -> Result<(), u32> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> isize;
        fn SetPriorityClass(process: isize, class: u32) -> i32;
        fn GetLastError() -> u32;
    }
    const ABOVE_NORMAL_PRIORITY_CLASS: u32 = 0x8000;
    // SAFETY: Win32 calls on the pseudo handle of the current process, no pointers involved
    unsafe {
        if SetPriorityClass(GetCurrentProcess(), ABOVE_NORMAL_PRIORITY_CLASS) != 0 { Ok(()) } else { Err(GetLastError()) }
    }
}

#[cfg(not(windows))]
fn above_normal() -> Result<(), u32> {
    Ok(())
}
