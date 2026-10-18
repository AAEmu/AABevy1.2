use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Key {
    pub time: f32,
    pub value: f32,
    pub flags: u32,
    pub incoming: f32,
    pub outgoing: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Curve {
    pub keys: Vec<Key>,
}

impl Curve {
    /// Cry scalar spline serialization: comma-separated `time:value[:flags[:in:out]]`.
    pub fn parse(text: &str) -> Result<Self> {
        let mut keys = Vec::new();
        for raw in text.split(',').filter(|s| !s.trim().is_empty()) {
            let fields: Vec<_> = raw.split(':').collect();
            if fields.len() < 2 || fields.len() > 5 {
                return Err(Error::Format(format!("facial spline key {raw}")));
            }
            let f = |i: usize| -> Result<f32> {
                fields.get(i).map_or(Ok(0.0), |v| {
                    v.parse::<f32>()
                        .ok()
                        .filter(|x| x.is_finite())
                        .ok_or_else(|| Error::Format(format!("facial spline key {raw}")))
                })
            };
            let flags = fields.get(2).map_or(Ok(0), |v| {
                v.parse::<u32>().map_err(|_| Error::Format(format!("facial spline flags {raw}")))
            })?;
            keys.push(Key {
                time: f(0)?,
                value: f(1)?,
                flags,
                incoming: f(3)?,
                outgoing: f(4)?,
            });
        }
        keys.sort_by(|a, b| a.time.total_cmp(&b.time));
        Ok(Self { keys })
    }

    /// Sequence Hermite curve. Low three flag bits select the incoming tangent; next three,
    /// outgoing. 0 smooth, 1 custom, 2 zero, 3 step, 4 linear. UI bits are ignored.
    pub fn sample(&self, time: f32) -> f32 {
        self.interpolate(time, false)
    }

    /// FXL's controller spline uses Catmull–Rom slopes, not the FSQ tangent modes.
    pub fn sample_catmull(&self, input: f32) -> f32 {
        self.interpolate(input, true)
    }

    fn interpolate(&self, time: f32, catmull: bool) -> f32 {
        let Some(first) = self.keys.first() else { return 0.0 };
        if time < first.time {
            return first.value;
        }
        let i = self.keys.partition_point(|k| k.time <= time).saturating_sub(1);
        let a = self.keys[i];
        let Some(b) = self.keys.get(i + 1).copied() else { return a.value };
        let span = b.time - a.time;
        if span <= 0.0 {
            return b.value;
        }
        if !catmull && (((a.flags >> 3) & 7) == 3 || (b.flags & 7) == 3) {
            return a.value;
        }
        let tangent = |index: usize, outgoing: bool| {
            let key = self.keys[index];
            let previous = index.checked_sub(1).map(|p| self.keys[p].value).unwrap_or(key.value);
            let next = self.keys.get(index + 1).map_or(key.value, |k| k.value);
            let smooth = 0.5 * (next - previous);
            if catmull {
                return smooth;
            }
            match (key.flags >> if outgoing { 3 } else { 0 }) & 7 {
                1 => {
                    if outgoing {
                        key.outgoing
                    } else {
                        key.incoming
                    }
                }
                2 | 3 => 0.0,
                4 => {
                    if outgoing {
                        next - key.value
                    } else {
                        key.value - previous
                    }
                }
                _ => smooth,
            }
        };
        let t = ((time - a.time) / span).clamp(0.0, 1.0);
        let t2 = t * t;
        let t3 = t2 * t;
        (2.0 * t3 - 3.0 * t2 + 1.0) * a.value
            + (-2.0 * t3 + 3.0 * t2) * b.value
            + (t3 - 2.0 * t2 + t) * tangent(i, true)
            + (t3 - t2) * tangent(i + 1, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoints_empty_duplicate_and_negative() {
        let c = Curve::parse("-1:-2,0:0,1:1,1:1,").unwrap();
        assert_eq!(c.sample(-2.0), -2.0);
        assert_eq!(c.sample(0.0), 0.0);
        assert_eq!(c.sample(1.0), 1.0);
        assert_eq!(c.sample(2.0), 1.0);
        assert_eq!(Curve::default().sample(2.0), 0.0);
        assert!(Curve::parse("0:NaN:0").is_err());
    }
    #[test]
    fn tangent_flags_and_hermite() {
        assert!((Curve::parse("0:0:32,1:1:4").unwrap().sample(0.25) - 0.25).abs() < 1e-6);
        assert!((Curve::parse("0:0:16,1:1:2").unwrap().sample(0.25) - 0.15625).abs() < 1e-6);
        assert_eq!(Curve::parse("0:0:24,1:1:0").unwrap().sample(0.99), 0.0);
        assert_eq!(Curve::parse("0:0:24,1:1:0").unwrap().sample(1.0), 1.0);
        assert!((Curve::parse("0:0:8:0:2,1:1:1:2:0").unwrap().sample(0.25) - 0.34375).abs() < 1e-6);
    }
}
