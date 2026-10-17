//! Native effect kinematics, recovered from x2game_unpacked.dll 3928a8d0/3928df00.
//! Times are milliseconds here; the Lua/native API uses seconds.

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Timing {
    pub velocity: f32,
    pub acceleration: f32,
}

#[cfg(test)]
mod tests {
    use super::Timing;

    #[test]
    fn native_velocity_and_acceleration_times_control_progress() {
        let t = Timing {
            velocity: 500.0,
            acceleration: 400.0,
        };
        assert!((t.fraction(250.0) - 0.578125).abs() < 0.00001);
        assert!((t.duration() - 400.0).abs() < 0.00001);
        let t = Timing {
            velocity: 1500.0,
            acceleration: 0.0,
        };
        assert_eq!(t.fraction(375.0), 0.25);
        let t = Timing {
            velocity: 500.0,
            acceleration: 1500.0,
        };
        assert!((t.duration() - 750.0).abs() < 0.00001);
    }
}

impl Timing {
    pub(super) fn coefficients(self) -> (f64, f64) {
        let velocity = if self.velocity > 0.0 {
            1.0 / self.velocity as f64
        } else {
            0.0
        };
        let acceleration = if self.acceleration > 0.0 {
            let t = self.acceleration as f64;
            2.0 * (1.0 - velocity * t) / (t * t)
        } else {
            0.0
        };
        (velocity, acceleration)
    }

    pub fn duration(self) -> f64 {
        let (v, a) = self.coefficients();
        if a.abs() < 1e-18 {
            return if v > 0.0 { 1.0 / v } else { 0.0 };
        }
        // First crossing of the final value. Acceleration time can exceed velocity time.
        let discriminant = (v * v + 2.0 * a).max(0.0).sqrt();
        let roots = [(-v + discriminant) / a, (-v - discriminant) / a];
        roots
            .into_iter()
            .filter(|t| *t >= 0.0 && t.is_finite())
            .fold(f64::INFINITY, f64::min)
    }

    pub fn fraction(self, elapsed: f64) -> f32 {
        let duration = self.duration();
        if duration <= 0.0 || elapsed >= duration {
            return 1.0;
        }
        let (v, a) = self.coefficients();
        (v * elapsed + 0.5 * a * elapsed * elapsed).clamp(0.0, 1.0) as f32
    }
}
