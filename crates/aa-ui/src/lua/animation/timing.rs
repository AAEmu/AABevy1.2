//! The two native Effect phase clocks (3928e6c0/3928f460), in f32 seconds.

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Timing {
    pub velocity: f32,
    pub acceleration: f32,
}

impl Timing {
    pub fn coefficients(self, delta: f32) -> [f32; 2] {
        let v = if self.velocity != 0. {
            delta / self.velocity
        } else {
            0.
        };
        let a = if self.acceleration != 0. {
            ((delta - v * self.acceleration) * 2.) / (self.acceleration * self.acceleration)
        } else {
            0.
        };
        [v, a]
    }
}

#[derive(Clone, Debug)]
pub(super) struct Clock {
    pub phase: usize,
    pub running: bool,
    pub finished: bool,
    pub elapsed: f32,
    pub carry: f32,
    pub delay_elapsed: f32,
    pub hold_elapsed: f32,
    pub counter: u32,
    pub coefficients: [f32; 2],
    // A repaint optimization only; native running flags do not admit the draw clock.
    pub terminal: bool,
}

impl Default for Clock {
    fn default() -> Self {
        Self {
            phase: 0,
            running: false,
            finished: false,
            elapsed: 0.,
            carry: 0.,
            delay_elapsed: 0.,
            hold_elapsed: 0.,
            counter: 1,
            coefficients: [0.; 2],
            terminal: false,
        }
    }
}

impl Clock {
    pub fn initialize(&mut self, phase: usize, delta: f32, timing: Timing) {
        self.phase = phase;
        self.finished = false;
        self.elapsed = 0.;
        self.hold_elapsed = 0.;
        self.coefficients = timing.coefficients(delta);
        self.terminal = false;
    }

    pub fn start(&mut self) {
        self.delay_elapsed = 0.;
        self.counter = 1;
        self.carry = 0.;
        self.running = true;
        self.terminal = false;
    }

    /// Delay admission returns the leftover frame time, retaining native add/sub order.
    pub fn time(&mut self, dt: f32, delay: f32) -> Option<f32> {
        let elapsed = self.delay_elapsed + dt;
        if delay != 0. && delay > elapsed {
            self.delay_elapsed = elapsed;
            return None;
        }
        self.delay_elapsed = delay;
        Some(elapsed - delay)
    }

    pub fn displacement(&mut self, dt: f32) -> f32 {
        self.elapsed += dt;
        let [v, mut a] = self.coefficients;
        let t = self.elapsed;
        if t == 0. || (v == 0. && a == 0.) {
            return 0.;
        }
        let end_velocity = a * t + v;
        if end_velocity == 0. || v / end_velocity < 0. {
            a = 0.;
        }
        //3928a8d0 evaluates the f32 inputs in x87, then the caller spills to f32.
        let t = f64::from(t);
        (f64::from(v) * t + (f64::from(a) * 0.5) * (t * t)) as f32
    }

    pub fn finish(&mut self, timing: Timing) {
        self.finished = true;
        self.carry = self.elapsed
            - if timing.acceleration != 0. {
                timing.acceleration
            } else {
                timing.velocity
            };
    }

    /// At most one handoff per draw. The new phase keeps its initial sample this frame.
    pub fn following(
        &mut self,
        mut dt: f32,
        hold: f32,
        count: usize,
        repeat: u32,
    ) -> Option<(usize, f32)> {
        if self.carry != 0. {
            dt += self.carry;
            self.carry = 0.;
        }
        if hold != 0. {
            let elapsed = self.hold_elapsed + dt;
            if hold > elapsed {
                self.hold_elapsed = elapsed;
                return None;
            }
            dt = elapsed - hold;
        }
        if self.phase + 1 < count {
            return Some((self.phase + 1, dt));
        }
        if self.phase + 1 == count && (repeat == 0 || repeat > self.counter) {
            if repeat != 0 {
                self.counter = self.counter.wrapping_add(1);
            }
            return Some((0, dt));
        }
        self.running = false;
        self.terminal = true;
        None
    }
}

pub(super) fn crossed(initial: f32, final_value: f32, value: f32) -> bool {
    (final_value >= initial && value >= final_value)
        || (initial >= final_value && final_value >= value)
}

#[cfg(test)]
mod tests {
    use super::{Clock, Timing};
    #[test]
    fn executed_native_acceleration_and_overshoot_samples() {
        let t = Timing {
            velocity: 0.5,
            acceleration: 0.4,
        };
        let mut clock = Clock::default();
        clock.initialize(0, 1., t);
        assert_eq!(
            clock.displacement(0.1).to_bits(),
            0.21250000596046448_f32.to_bits()
        );
        clock.displacement(0.35);
        clock.finish(t);
        assert_eq!(clock.carry.to_bits(), 0.04999998211860657_f32.to_bits());
        assert_eq!(clock.following(0.05, 0.15, 2, 1), None);
        assert_eq!(
            clock.hold_elapsed.to_bits(),
            0.09999997913837433_f32.to_bits()
        );
    }
}
