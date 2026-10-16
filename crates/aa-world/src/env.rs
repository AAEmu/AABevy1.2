//! Environment data: `zone/<id>/time_of_day.xml`, `env.xml`, `lighting.xml`.
//!
//! `time_of_day.xml` is CryEngine 3's time of day: `<TimeOfDay ...>` blocks (one per relic / weather
//! layer; the first is the default) of `<Variable Name=.. Value=..|Color="r,g,b"><Spline Keys="t:v:flags,.."/>`.
//! Keys are `time:value:tangentFlags` with `time` in `0..1` of the day (`hours / 24`), colours are
//! `time:(r:g:b):flags`. Colours are sRGB encoded; `* multiplier` variables carry the HDR scale
//! (sun ~ 6..8, sky ~ 2..6 at the engine's exposure).
//!
//! `lighting.xml` gives the sun path: `SunRotation` is the compass bearing (clockwise from north) the sun's
//! light *travels towards* at midday, `SunHeight` the highest elevation (degrees).

use std::collections::HashMap;

use bevy::math::Vec3;

/// A time of day spline (scalar or colour).
#[derive(Clone, Debug, Default)]
pub struct Spline {
    /// `(time 0..1, value)`; scalars use `x`.
    pub keys: Vec<(f32, [f32; 3])>,
    pub is_color: bool,
    /// The editor's current value (used when there are no keys).
    pub value: [f32; 3],
}

impl Spline {
    /// Linear interpolation over the (wrapping) day.
    pub fn eval(&self, t: f32) -> [f32; 3] {
        let t = t.rem_euclid(1.0);
        let k = &self.keys;
        match k.len() {
            0 => self.value,
            1 => k[0].1,
            _ => {
                // find the segment [i, i+1] containing t, wrapping from the last key to the first
                let next = k.iter().position(|(kt, _)| *kt > t);
                let (a, b, span, local) = match next {
                    Some(0) => {
                        let (a, b) = (k.last().unwrap(), &k[0]);
                        let span = b.0 + 1.0 - a.0;
                        (a, b, span, t + 1.0 - a.0)
                    }
                    Some(i) => (&k[i - 1], &k[i], k[i].0 - k[i - 1].0, t - k[i - 1].0),
                    None => {
                        let (a, b) = (k.last().unwrap(), &k[0]);
                        let span = b.0 + 1.0 - a.0;
                        (a, b, span, t - a.0)
                    }
                };
                let u = if span > 1e-6 { (local / span).clamp(0.0, 1.0) } else { 0.0 };
                [0, 1, 2].map(|i| a.1[i] + (b.1[i] - a.1[i]) * u)
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct TimeOfDay {
    pub variables: HashMap<String, Spline>,
    /// `Time` attribute: the editor's current hour.
    pub default_hour: f32,
}

fn parse_floats(s: &str) -> Vec<f32> {
    s.split(|c: char| c == ',' || c == ':' || c == '(' || c == ')')
        .filter(|p| !p.trim().is_empty())
        .filter_map(|p| p.trim().parse().ok())
        .collect()
}

fn parse_spline(keys: &str, color: bool) -> Vec<(f32, [f32; 3])> {
    let mut out = Vec::new();
    // keys are separated by ',' but colour values contain ':' only, so split on ',' at depth 0
    for key in keys.split(',').filter(|k| !k.trim().is_empty()) {
        let v = parse_floats(key);
        if color && v.len() >= 4 {
            out.push((v[0], [v[1], v[2], v[3]]));
        } else if !color && v.len() >= 2 {
            out.push((v[0], [v[1], 0.0, 0.0]));
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out
}

impl TimeOfDay {
    pub fn parse(xml: &str) -> Option<TimeOfDay> {
        let doc = roxmltree::Document::parse(xml).ok()?;
        let block = doc
            .descendants()
            .filter(|n| n.has_tag_name("TimeOfDay"))
            .find(|n| n.attribute("relic_layer").unwrap_or("0") == "0" && n.attribute("weather_layer").unwrap_or("0") == "0")
            .or_else(|| doc.descendants().find(|n| n.has_tag_name("TimeOfDay")))?;
        let default_hour = block.attribute("Time").and_then(|v| v.parse().ok()).unwrap_or(12.0);
        let mut variables = HashMap::new();
        for v in block.children().filter(|n| n.has_tag_name("Variable")) {
            let Some(name) = v.attribute("Name") else { continue };
            let is_color = v.attribute("Color").is_some();
            let value = if let Some(c) = v.attribute("Color") {
                let f = parse_floats(c);
                [f.first().copied().unwrap_or(0.0), f.get(1).copied().unwrap_or(0.0), f.get(2).copied().unwrap_or(0.0)]
            } else {
                [v.attribute("Value").and_then(|x| x.parse().ok()).unwrap_or(0.0), 0.0, 0.0]
            };
            let keys = v
                .children()
                .find(|n| n.has_tag_name("Spline"))
                .and_then(|s| s.attribute("Keys"))
                .map(|k| parse_spline(k, is_color))
                .unwrap_or_default();
            variables.insert(name.to_string(), Spline { keys, is_color, value });
        }
        Some(TimeOfDay { variables, default_hour })
    }

    pub fn scalar(&self, name: &str, t: f32) -> f32 {
        self.variables.get(name).map_or(0.0, |s| s.eval(t)[0])
    }

    pub fn color(&self, name: &str, t: f32) -> Vec3 {
        self.variables.get(name).map_or(Vec3::ZERO, |s| Vec3::from(s.eval(t)))
    }
}

/// `lighting.xml` sun path settings.
#[derive(Clone, Copy, Debug)]
pub struct Lighting {
    pub sun_rotation: f32,
    pub sun_height: f32,
}

impl Default for Lighting {
    fn default() -> Self {
        Lighting { sun_rotation: 188.0, sun_height: 80.0 }
    }
}

impl Lighting {
    pub fn parse(xml: &str) -> Option<Lighting> {
        let doc = roxmltree::Document::parse(xml).ok()?;
        let l = doc.descendants().find(|n| n.has_tag_name("Lighting"))?;
        Some(Lighting {
            sun_rotation: l.attribute("SunRotation")?.parse().ok()?,
            sun_height: l.attribute("SunHeight").and_then(|v| v.parse().ok()).unwrap_or(80.0),
        })
    }
}

/// One evaluated frame of the environment (linear colours, `Vec3` rgb).
#[derive(Clone, Debug)]
pub struct EnvSample {
    /// Direction *towards* the sun (cry space, Z up).
    pub sun_dir: Vec3,
    /// `Sun color * multiplier`, linear.
    pub sun: Vec3,
    pub sky: Vec3,
    pub ambient_ground: Vec3,
    pub fog: Vec3,
    pub fog_density: f32,
    pub ocean_fog: Vec3,
    /// Raw sRGB colours (before multipliers), for sky gradients.
    pub sky_srgb: Vec3,
    pub fog_srgb: Vec3,
    pub night_horizon: Vec3,
    pub night_zenith: Vec3,
    /// 0 at night .. 1 at day: how much the sun contributes.
    pub day: f32,
}

fn srgb_to_linear(c: Vec3) -> Vec3 {
    let f = |x: f32| if x <= 0.04045 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) };
    Vec3::new(f(c.x), f(c.y), f(c.z))
}

impl TimeOfDay {
    /// Evaluates the environment at `hour` (0..24).
    pub fn sample(&self, hour: f32, lighting: Lighting) -> EnvSample {
        let t = hour / 24.0;
        let col = |name: &str, mul: &str| {
            let c = self.color(name, t);
            let m = if mul.is_empty() { 1.0 } else { self.scalar(mul, t) };
            (c, srgb_to_linear(c) * m)
        };
        let (_, sun) = col("Sun color", "Sun color multiplier");
        let (sky_srgb, sky) = col("Sky color", "Sky color multiplier");
        let (_, ambient_ground) = col("Ambient ground color", "Ambient ground color multiplier");
        let (fog_srgb, fog) = col("Fog color", "Fog color multiplier");
        let (_, ocean_fog) = col("Ocean fog color", "Ocean fog color multiplier");
        let (nh, _) = col("Night sky: Horizon color", "");
        let (nz, _) = col("Night sky: Zenith color", "");
        // The sun is up between the dawn and dusk colour keys: brightness of the sun colour decides.
        let lum = |c: Vec3| c.dot(Vec3::new(0.2126, 0.7152, 0.0722));
        let sun_srgb = self.color("Sun color", t);
        let day = ((lum(sun_srgb) - 0.25) / 0.5).clamp(0.0, 1.0);
        EnvSample {
            sun_dir: sun_direction(hour, lighting),
            sun,
            sky,
            ambient_ground,
            fog,
            fog_density: self.scalar("Volumetric fog: Global density", t),
            ocean_fog,
            sky_srgb,
            fog_srgb,
            night_horizon: nh,
            night_zenith: nz,
            day,
        }
    }
}

/// Direction towards the sun (cry space) at `hour`: rises in the east around 06:00, peaks at
/// `sun_height` on the midday bearing and sets in the west around 18:00; below the horizon (moon side)
/// otherwise.
pub fn sun_direction(hour: f32, l: Lighting) -> Vec3 {
    let u = (hour - 6.0) / 12.0; // 0 at sunrise .. 1 at sunset
    let elev = if (0.0..=1.0).contains(&u) {
        (l.sun_height.min(75.0)) * (std::f32::consts::PI * u).sin()
    } else {
        // night: continue the arc below the horizon
        let n = if u > 1.0 { (u - 1.0) / 1.0 } else { (u + 1.0) / 1.0 };
        -(l.sun_height.min(75.0)) * (std::f32::consts::PI * n.clamp(0.0, 1.0)).sin()
    };
    // bearing of the sun (clockwise from north): midday = rotation - 180
    let noon = (l.sun_rotation - 180.0).to_radians();
    let az = noon + (0.5 - u) * std::f32::consts::PI;
    let e = elev.to_radians();
    // bearing -> cry axes: north = +Y, east = +X
    Vec3::new(az.sin() * e.cos(), az.cos() * e.cos(), e.sin()).normalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spline_eval_wraps() {
        let s = Spline { keys: vec![(0.0, [1.0, 0.0, 0.0]), (0.5, [3.0, 0.0, 0.0]), (1.0, [1.0, 0.0, 0.0])], is_color: false, value: [0.0; 3] };
        assert!((s.eval(0.25)[0] - 2.0).abs() < 1e-5);
        assert!((s.eval(0.75)[0] - 2.0).abs() < 1e-5);
        assert!((s.eval(1.25)[0] - 2.0).abs() < 1e-5);
    }

    #[test]
    fn parse_variables() {
        let xml = r#"<TimeOfDayList><TimeOfDay Time="12">
            <Variable Name="Sun color multiplier" Value="8"><Spline Keys="0:8:0,0.5:4:0,1:8:0,"/></Variable>
            <Variable Name="Sun color" Color="0.5,0.5,0.5"><Spline Keys="0:(0.1:0.2:0.3):0,1:(0.1:0.2:0.3):0,"/></Variable>
            </TimeOfDay></TimeOfDayList>"#;
        let t = TimeOfDay::parse(xml).unwrap();
        assert!((t.scalar("Sun color multiplier", 0.5) - 4.0).abs() < 1e-5);
        let c = t.color("Sun color", 0.3);
        assert!((c.y - 0.2).abs() < 1e-5);
    }

    #[test]
    fn sun_path() {
        let l = Lighting { sun_rotation: 188.0, sun_height: 70.0 };
        let noon = sun_direction(12.0, l);
        assert!(noon.z > 0.9);
        let rise = sun_direction(6.5, l);
        assert!(rise.x > 0.5 && rise.z > 0.0, "{rise}");
        let set = sun_direction(17.5, l);
        assert!(set.x < -0.5 && set.z > 0.0, "{set}");
        assert!(sun_direction(0.0, l).z < 0.0);
    }

    #[test]
    fn real_zone() {
        if !crate::pak_available() {
            return;
        }
        let xml = aa_pak::Pak::global().read_string("worlds/main_world/zone/129/time_of_day.xml").unwrap();
        let t = TimeOfDay::parse(&xml).unwrap();
        let noon = t.sample(13.0, Lighting::default());
        let midnight = t.sample(0.0, Lighting::default());
        assert!(noon.day > 0.9 && midnight.day < 0.1, "{} {}", noon.day, midnight.day);
        assert!(noon.sun.length() > 3.0);
    }
}
