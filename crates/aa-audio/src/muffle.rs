//! The low-pass filter of a volume group (the `LowPassCutoff` of a sound mood: under water, when dead, in a fall).
//!
//! A second order low-pass whose cutoff glides to what [`MuffleHandle::set`] says; at 20 kHz and above it is off and costs
//! a branch per block. The kira filter does the same but computes a tangent per sample, which is wasted on a filter
//! that is open nearly all the time.

use std::f32::consts::PI;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use kira::Frame;
use kira::effect::{Effect, EffectBuilder};
use kira::info::Info;

/// Cutoffs at or above this are "open".
pub const OPEN: f32 = 19000.0;

struct Shared {
    cutoff: AtomicU32,
}

#[derive(Clone)]
pub struct MuffleHandle {
    shared: Arc<Shared>,
}

impl MuffleHandle {
    /// Hz; `None` opens the filter.
    pub fn set(&self, cutoff: Option<f32>) {
        self.shared.cutoff.store(cutoff.unwrap_or(OPEN * 1.5).to_bits(), Ordering::Relaxed);
    }
}

pub struct MuffleBuilder;

impl EffectBuilder for MuffleBuilder {
    type Handle = MuffleHandle;

    fn build(self) -> (Box<dyn Effect>, MuffleHandle) {
        let shared = Arc::new(Shared { cutoff: AtomicU32::new((OPEN * 1.5).to_bits()) });
        (Box::new(Muffle::new(shared.clone())), MuffleHandle { shared })
    }
}

/// Direct form II transposed biquad coefficients of an RBJ low-pass, Q = 1/sqrt(2).
fn lowpass(cutoff: f32, rate: f32) -> [f32; 5] {
    let w = 2.0 * PI * cutoff.clamp(20.0, rate * 0.45) / rate;
    let (s, c) = w.sin_cos();
    let alpha = s / (2.0 * std::f32::consts::FRAC_1_SQRT_2);
    let a0 = 1.0 + alpha;
    [(1.0 - c) / 2.0 / a0, (1.0 - c) / a0, (1.0 - c) / 2.0 / a0, -2.0 * c / a0, (1.0 - alpha) / a0]
}

struct Muffle {
    shared: Arc<Shared>,
    rate: f32,
    target: f32,
    /// The cutoff in use; glides to `target` (on a log scale).
    cutoff: f32,
    coefficients: [f32; 5],
    z: [[f32; 2]; 2],
}

impl Muffle {
    fn new(shared: Arc<Shared>) -> Muffle {
        let open = OPEN * 1.5;
        Muffle { shared, rate: 44100.0, target: open, cutoff: open, coefficients: lowpass(open, 44100.0), z: [[0.0; 2]; 2] }
    }
}

impl Muffle {
    /// Filters a block of frames `dt` seconds apart, in place.
    fn run(&mut self, input: &mut [Frame], dt: f64) {
        if self.cutoff >= OPEN && self.target >= OPEN {
            self.z = [[0.0; 2]; 2];
            return;
        }
        // about 0.3 s from the open filter to the closed one, on a log scale
        let diff = self.target.ln() - self.cutoff.ln();
        if diff.abs() > 1e-3 {
            let k = 1.0 - (-(input.len() as f64 * dt) / 0.2).exp() as f32;
            self.cutoff = if diff.abs() < 0.03 { self.target } else { self.cutoff * (diff * k).exp() };
            self.coefficients = lowpass(self.cutoff, self.rate);
        }
        let [b0, b1, b2, a1, a2] = self.coefficients;
        for f in input.iter_mut() {
            let (l, r) = (f.left, f.right);
            let ol = b0 * l + self.z[0][0];
            self.z[0][0] = b1 * l - a1 * ol + self.z[0][1];
            self.z[0][1] = b2 * l - a2 * ol;
            let or = b0 * r + self.z[1][0];
            self.z[1][0] = b1 * r - a1 * or + self.z[1][1];
            self.z[1][1] = b2 * r - a2 * or;
            *f = Frame { left: ol, right: or };
        }
    }
}

impl Effect for Muffle {
    fn init(&mut self, sample_rate: u32, _internal_buffer_size: usize) {
        self.rate = sample_rate.max(8000) as f32;
        self.coefficients = lowpass(self.cutoff, self.rate);
    }

    fn on_change_sample_rate(&mut self, sample_rate: u32) {
        self.init(sample_rate, 0);
    }

    fn on_start_processing(&mut self) {
        self.target = f32::from_bits(self.shared.cutoff.load(Ordering::Relaxed));
    }

    fn process(&mut self, input: &mut [Frame], dt: f64, _info: &Info) {
        self.run(input, dt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Peak of a sine of `freq` Hz after the filter with `cutoff` has settled.
    fn peak(cutoff: Option<f32>, freq: f32) -> f32 {
        let (_, h) = MuffleBuilder.build();
        let mut m = Muffle::new(h.shared.clone());
        h.set(cutoff);
        m.init(44100, 128);
        m.on_start_processing();
        m.cutoff = m.target; // no glide in the test
        m.coefficients = lowpass(m.cutoff, m.rate);
        let mut peak = 0.0f32;
        for block in 0..40 {
            let mut buf: Vec<Frame> = (0..256)
                .map(|i| {
                    let v = (2.0 * PI * freq * (block * 256 + i) as f32 / 44100.0).sin();
                    Frame::new(v, v)
                })
                .collect();
            m.run(&mut buf, 1.0 / 44100.0);
            if block > 20 {
                peak = buf.iter().fold(peak, |p, f| p.max(f.left.abs()));
            }
        }
        peak
    }

    #[test]
    fn passes_the_lows_and_cuts_the_highs() {
        assert!(peak(Some(500.0), 100.0) > 0.9, "100 Hz passes a 500 Hz low-pass");
        assert!(peak(Some(500.0), 4000.0) < 0.05, "4 kHz is cut");
        let open = peak(None, 4000.0);
        assert!((open - 1.0).abs() < 0.01, "open: untouched, got {open}");
    }

    #[test]
    fn the_cutoff_glides() {
        let (_, h) = MuffleBuilder.build();
        let mut m = Muffle::new(h.shared.clone());
        m.init(44100, 128);
        h.set(Some(300.0));
        m.on_start_processing();
        let mut buf = vec![Frame::new(0.1, 0.1); 128];
        m.run(&mut buf, 1.0 / 44100.0);
        assert!(m.cutoff < OPEN * 1.5 && m.cutoff > 300.0, "moving, at {}", m.cutoff);
        for _ in 0..400 {
            m.run(&mut buf, 1.0 / 44100.0);
        }
        assert!((m.cutoff - 300.0).abs() < 5.0, "settled at {}", m.cutoff);
    }
}
