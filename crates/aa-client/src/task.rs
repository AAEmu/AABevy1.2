//! Background work: a thread natively, inline in the browser (a single thread, `std::thread` is not available there).

/// Runs `work` on a new thread named `name` (fire and forget). In the browser it runs right here, before `spawn` returns.
pub fn spawn(name: &str, work: impl FnOnce() + Send + 'static) {
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::Builder::new().name(name.into()).spawn(work).ok();
    #[cfg(target_arch = "wasm32")]
    {
        let _ = name;
        work();
    }
}
