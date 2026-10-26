//! Timers: tokio natively, browser timers on wasm32 (tokio's `time` driver does not exist there).

use std::future::Future;
use std::time::Duration;

#[cfg(not(target_arch = "wasm32"))]
pub use native::*;
#[cfg(target_arch = "wasm32")]
pub use web::*;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;

    /// `Err(())` when `fut` did not finish within `dur`.
    pub async fn timeout<F: Future>(dur: Duration, fut: F) -> Result<F::Output, ()> {
        tokio::time::timeout(dur, fut).await.map_err(|_| ())
    }

    /// Fixed period ticker that never bursts to catch up.
    pub struct Ticker(tokio::time::Interval);

    impl Ticker {
        pub fn new(period: Duration) -> Self {
            let mut i = tokio::time::interval(period);
            i.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            Self(i)
        }

        /// Cancel safe.
        pub async fn tick(&mut self) {
            self.0.tick().await;
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use super::*;
    use gloo_timers::future::sleep;
    use web_time::Instant;

    /// `Err(())` when `fut` did not finish within `dur`.
    pub async fn timeout<F: Future>(dur: Duration, fut: F) -> Result<F::Output, ()> {
        tokio::select! {
            out = fut => Ok(out),
            _ = sleep(dur) => Err(()),
        }
    }

    /// Fixed period ticker that never bursts to catch up.
    pub struct Ticker {
        period: Duration,
        next: Instant,
    }

    impl Ticker {
        pub fn new(period: Duration) -> Self {
            Self { period, next: Instant::now() }
        }

        /// Cancel safe: the deadline only moves once a tick has completed.
        pub async fn tick(&mut self) {
            let now = Instant::now();
            if self.next > now {
                sleep(self.next - now).await;
            }
            self.next = Instant::now() + self.period;
        }
    }
}
