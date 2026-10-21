//! A few background threads for asset preparation (parsing models, decoding textures, building meshes).
//!
//! Bevy's task pools are shared with terrain streaming and the ECS executor; model preparation is
//! long, bursty and wants a nearest-first order, so it gets its own small pool: jobs carry a
//! priority (lower runs first, e.g. the distance to the camera) and run on [`Workers::global`].
//!
//! ```no_run
//! let workers = aa_cry::workers::Workers::global();
//! let (tx, rx) = std::sync::mpsc::channel();
//! workers.spawn(12.5, move || { let _ = tx.send(expensive()); });
//! # fn expensive() -> u32 { 1 }
//! // later, on the main thread: rx.try_recv()
//! ```

// the queue is native only: the browser runs jobs inline
#![cfg_attr(target_arch = "wasm32", allow(dead_code, unused_imports))]

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering as AtomicOrdering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

struct Job {
    priority: f32,
    seq: u64,
    run: Box<dyn FnOnce() + Send>,
}

impl PartialEq for Job {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Job {}

impl PartialOrd for Job {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Job {
    /// `BinaryHeap` pops the greatest: the smallest priority value (then the oldest job) is the greatest.
    fn cmp(&self, other: &Self) -> Ordering {
        other.priority.total_cmp(&self.priority).then(other.seq.cmp(&self.seq))
    }
}

struct Inner {
    queue: Mutex<BinaryHeap<Job>>,
    wake: Condvar,
    seq: AtomicU64,
    /// Jobs queued or running.
    pending: AtomicUsize,
}

/// A pool of worker threads that run jobs in priority order.
#[derive(Clone)]
pub struct Workers {
    inner: Arc<Inner>,
}

impl Workers {
    /// A pool with `threads` workers (at least one). They live as long as the process.
    pub fn new(threads: usize) -> Workers {
        let inner = Arc::new(Inner { queue: Mutex::new(BinaryHeap::new()), wake: Condvar::new(), seq: AtomicU64::new(0), pending: AtomicUsize::new(0) });
        #[cfg(target_arch = "wasm32")]
        let _ = threads; // the browser build is single threaded: `spawn` runs the job inline
        #[cfg(not(target_arch = "wasm32"))]
        for i in 0..threads.max(1) {
            let inner = inner.clone();
            std::thread::Builder::new()
                .name(format!("aa-workers-{i}"))
                .spawn(move || loop {
                    let job = {
                        let mut queue = inner.queue.lock().unwrap_or_else(|e| e.into_inner());
                        loop {
                            if let Some(job) = queue.pop() {
                                break job;
                            }
                            queue = inner.wake.wait(queue).unwrap_or_else(|e| e.into_inner());
                        }
                    };
                    // a panicking job must not take the worker down (its result channel just closes)
                    let _ = catch_unwind(AssertUnwindSafe(job.run));
                    inner.pending.fetch_sub(1, AtomicOrdering::Relaxed);
                })
                .expect("cannot start a worker thread");
        }
        Workers { inner }
    }

    /// The process wide pool: a third of the hardware threads, 2 to 5 (terrain cells, placement files, models and units all go
    /// through it; the order is by priority, so more threads only shorten the queue).
    pub fn global() -> &'static Workers {
        static GLOBAL: OnceLock<Workers> = OnceLock::new();
        GLOBAL.get_or_init(|| {
            #[cfg(not(target_arch = "wasm32"))]
            let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
            #[cfg(target_arch = "wasm32")]
            let cores = 1;
            // `AA_WORKERS=N` for experiments
            let n = std::env::var("AA_WORKERS").ok().and_then(|v| v.parse().ok()).unwrap_or((cores / 3).clamp(2, 5));
            Workers::new(n)
        })
    }

    /// Queues `job`; jobs with a smaller `priority` run first.
    pub fn spawn(&self, priority: f32, job: impl FnOnce() + Send + 'static) {
        // no threads in the browser: run the job right here (a panic there aborts the module anyway)
        #[cfg(target_arch = "wasm32")]
        {
            let _ = priority;
            job();
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.queue_job(priority, job);
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn queue_job(&self, priority: f32, job: impl FnOnce() + Send + 'static) {
        let seq = self.inner.seq.fetch_add(1, AtomicOrdering::Relaxed);
        self.inner.pending.fetch_add(1, AtomicOrdering::Relaxed);
        self.inner.queue.lock().unwrap_or_else(|e| e.into_inner()).push(Job { priority, seq, run: Box::new(job) });
        self.inner.wake.notify_one();
    }

    /// Jobs queued or running right now.
    pub fn pending(&self) -> usize {
        self.inner.pending.load(AtomicOrdering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use std::time::Duration;

    #[test]
    fn runs_in_priority_order() {
        let workers = Workers::new(1);
        let (gate_tx, gate_rx) = channel::<()>();
        let (tx, rx) = channel();
        // occupy the only worker so the rest queue up
        workers.spawn(0.0, move || {
            let _ = gate_rx.recv();
        });
        for (p, name) in [(5.0, "c"), (1.0, "a"), (3.0, "b"), (1.0, "a2")] {
            let tx = tx.clone();
            workers.spawn(p, move || {
                let _ = tx.send(name);
            });
        }
        gate_tx.send(()).unwrap();
        let got: Vec<&str> = (0..4).map(|_| rx.recv_timeout(Duration::from_secs(5)).unwrap()).collect();
        assert_eq!(got, ["a", "a2", "b", "c"]);
        // the counter drops just after the last job returns
        let start = web_time::Instant::now();
        while workers.pending() > 0 && start.elapsed() < Duration::from_secs(2) {
            std::thread::yield_now();
        }
        assert_eq!(workers.pending(), 0);
    }

    #[test]
    fn survives_a_panic() {
        let workers = Workers::new(1);
        workers.spawn(0.0, || panic!("boom"));
        let (tx, rx) = channel();
        workers.spawn(1.0, move || {
            let _ = tx.send(7);
        });
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), 7);
    }
}
