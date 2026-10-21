//! Original TimeKeeper arithmetic with explicit genuine network receive anchors.
use crate::{
    proxy::ProxyPong,
    session::{Event, Phase},
};
use web_time::Instant;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TimeSamplePair {
    pub rtt: i32,
    pub local: i32,
}
/// Exact five-entry native filter, including its wrapping arithmetic and omitted fifth square.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NativeTimeKeeper {
    pub pairs: [TimeSamplePair; 5],
    pub index: usize,
    pub anchor: u32,
    pub local_mean: i32,
}
fn cvtt(value: f32) -> i32 {
    if !(-2147483648.0..2147483648.0).contains(&value) {
        i32::MIN
    } else {
        value as i32
    }
}
impl NativeTimeKeeper {
    /// Native Init resets only anchor and ring index, not pairs/mean.
    pub fn init(&mut self) {
        self.anchor = 0;
        self.index = 0;
    }
    pub fn mapped_time(&self, local: u32) -> u32 {
        local.wrapping_add(self.anchor).wrapping_sub(self.local_mean as u32)
    }
    pub fn add_sample(&mut self, sent: u32, remote: u32, received: u32) {
        let rtt = received.wrapping_sub(sent) as i32;
        let half = (rtt as f32) * 0.5;
        if self.anchor == 0 {
            self.anchor = remote;
            self.pairs = [TimeSamplePair {
                rtt,
                local: cvtt(received as i32 as f32 - half),
            }; 5];
        } else {
            let local = cvtt(self.anchor.wrapping_sub(remote).wrapping_add(received) as i32 as f32 - half);
            self.pairs[self.index] = TimeSamplePair { rtt, local };
            self.index = (self.index + 1) % 5;
        }
        // Actual MSVCR100 qsort five-element shortsort, not a total-order Rust sort.
        let mut order = [0usize, 1, 2, 3, 4];
        for last in (1..5).rev() {
            let mut max = 0;
            for candidate in 1..=last {
                if self.pairs[order[candidate]].rtt.wrapping_sub(self.pairs[order[max]].rtt) > 0 {
                    max = candidate;
                }
            }
            order.swap(max, last);
        }
        let median = self.pairs[order[2]].rtt;
        let squares = order.map(|i| {
            let d = self.pairs[i].rtt.wrapping_sub(median);
            d.wrapping_mul(d)
        });
        let threshold = squares[0].wrapping_add(squares[1]).wrapping_add(squares[3]) / 5;
        let mut total = 0i32;
        let mut count = 0;
        for i in 0..5 {
            if squares[i] <= threshold {
                total = total.wrapping_add(self.pairs[order[i]].local);
                count += 1;
            }
        }
        self.local_mean = if count == 0 { 0 } else { total / count };
    }
}
/// Backend capture. `received_at` and low32 local receive time are taken together before UI delivery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockSample {
    pub pong: ProxyPong,
    pub local_receive_ms: u32,
    pub received_at: Instant,
    /// Exact backend timer origin; avoids re-anchoring and sub-millisecond rounding loss.
    pub local_origin: Instant,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct ServerClock {
    pub keeper: NativeTimeKeeper,
    pub last_sample: Option<ClockSample>,
}
impl ServerClock {
    /// Unknown until a genuine sample. No constructor clock is invented.
    pub fn mapped_now(&self) -> Option<u32> {
        self.mapped_at(Instant::now())
    }
    pub fn mapped_at(&self, now: Instant) -> Option<u32> {
        let sample = self.last_sample?;
        let local = now.saturating_duration_since(sample.local_origin).as_millis() as u32;
        Some(self.keeper.mapped_time(local))
    }
    pub fn apply(&mut self, event: &Event) -> bool {
        match event {
            Event::ClockSample(sample) => {
                self.keeper.add_sample(sample.pong.local, sample.pong.world, sample.local_receive_ms);
                self.last_sample = Some(*sample);
                true
            }
            Event::PhaseChanged(Phase::Connecting) | Event::ConnectFailed { .. } | Event::Disconnected { .. } | Event::Kicked { .. } => {
                let changed = self.last_sample.is_some() || self.keeper != NativeTimeKeeper::default();
                *self = Self::default();
                changed
            }
            _ => false,
        }
    }
}
#[cfg(test)]
mod tests;
