//! Environment data: `zone/<id>/time_of_day.xml`, `env.xml`, `lighting.xml`.
//!
//! `time_of_day.xml` is CryEngine 3's time of day: `<TimeOfDay ...>` blocks (one per relic / weather
//! layer; the first is the default) of `<Variable Name=.. Value=..|Color="r,g,b"><Spline Keys="t:v:flags,.."/>`.
//! Keys are `time:value:tangentFlags` with `time` in `0..1` of the day (`hours / 24`), colours are
//! `time:(r:g:b):flags`. Colours are sRGB encoded; `* multiplier` variables carry the HDR scale
//! (sun ~ 6..8, sky ~ 2..6 at the engine's exposure).
//!
//! `lighting.xml` gives the sun path (see [`sun_direction`]): `SunRotation` is the compass bearing of the sun at its highest
//! (the editor's `SunVector` points there), `SunHeight` that elevation in percent (`asin(SunHeight / 100)`: 80 is 53 degrees,
//! the `SunVector`'s z of 0.8), `PlanetRotationAxis` the tilt of the axis the sun turns about (degrees from the vertical).

use std::collections::HashMap;

use bevy::math::{Quat, Vec2, Vec3, Vec4};

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
    s.split([',', ':', '(', ')'])
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
    /// Layer 0 (the default `relic_layer` / `weather_layer`).
    pub fn parse(xml: &str) -> Option<TimeOfDay> {
        Self::parse_layer(xml, 0)
    }

    /// A zone file carries several `<TimeOfDay>` blocks (`relic_layer` 0 and 1); `layer` picks one, falling back to
    /// the first block.
    pub fn parse_layer(xml: &str, layer: u32) -> Option<TimeOfDay> {
        let doc = roxmltree::Document::parse(xml).ok()?;
        let want = layer.to_string();
        let block = doc
            .descendants()
            .filter(|n| n.has_tag_name("TimeOfDay"))
            .find(|n| n.attribute("relic_layer").unwrap_or("0") == want && n.attribute("weather_layer").unwrap_or("0") == "0")
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
    /// `PlanetRotationAxis`: the tilt (degrees from the vertical) of the axis the sun turns about.
    pub planet_axis: f32,
    /// The hour at which the sun is highest (see [`HIGHEST_HOUR`]; `AA_TUNE=sun_noon`).
    pub highest_hour: f32,
}

impl Default for Lighting {
    fn default() -> Self {
        Lighting { sun_rotation: 188.0, sun_height: 80.0, planet_axis: 25.0, highest_hour: HIGHEST_HOUR }
    }
}

/// The hour of the day at which the sun is highest, fitted so that the sun patch on the floor of the Royal Palace court yard
/// falls where the real client's does at game time 9.56 and 10.0 (`reference/ingame/royal_palace/`).
pub const HIGHEST_HOUR: f32 = 12.85;

impl Lighting {
    pub fn parse(xml: &str) -> Option<Lighting> {
        let doc = roxmltree::Document::parse(xml).ok()?;
        let l = doc.descendants().find(|n| n.has_tag_name("Lighting"))?;
        let number = |name: &str, default: f32| l.attribute(name).and_then(|v| v.parse().ok()).unwrap_or(default);
        Some(Lighting {
            sun_rotation: l.attribute("SunRotation")?.parse().ok()?,
            sun_height: number("SunHeight", 80.0),
            planet_axis: number("PlanetRotationAxis", 0.0),
            highest_hour: HIGHEST_HOUR,
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
    /// `Fog color (top) * multiplier`, linear: the colour of the thin fog high up.
    pub fog_top: Vec3,
    /// `(bottom height, top height, bottom density, top density)` of the fog layer.
    pub fog_layer: Vec4,
    /// `(ramp start, ramp end, ramp influence)` of the volumetric fog (metres).
    pub fog_ramp: Vec3,
    /// Sky gradient, linear: `Night sky: Horizon color * multiplier` (it is the sky's horizon colour all day long).
    pub horizon: Vec3,
    /// `Night sky: Zenith color * multiplier`, linear.
    pub zenith: Vec3,
    pub zenith_shift: f32,
    /// Shading colours of clouds: `Sun color * Cloud shading: Sun light multiplier` / `Sky color * ... Sky ...`, linear.
    pub cloud_sun: Vec3,
    pub cloud_sky: Vec3,
    /// How much of the sky box textures shows through.
    pub skybox_mul: f32,
    /// Film curve `(shoulder scale, midtones scale, toe scale, whitepoint)`.
    pub film: Vec4,
    pub saturation: f32,
    pub contrast: f32,
    pub color_balance: Vec3,
    /// `Levels: min input` (0..1 of the sRGB range).
    pub levels_min: f32,
    /// `Filters: photofilter` colour (sRGB) and density.
    pub photofilter: Vec4,
    /// HDR eye adaptation strength (`HDR dynamic power factor`).
    pub hdr_dynamic: f32,
    /// `Sun shafts visibility`.
    pub sun_shafts: f32,
    pub sun_specular: f32,
    /// The sky light's Rayleigh scattering: `Rayleigh scattering x (wavelength R / wavelength)^4 x Sky light: Sun intensity`
    /// per channel (linear); blue scatters about five times more than red at 500 nm.
    pub sky_rayleigh: Vec3,
    /// The sky light's Mie scattering (white haze): `Mie scattering x Sky light: Sun intensity`, linear.
    pub sky_mie: Vec3,
    /// `Ambient min height` / `Ambient max height` (metres): below the first the ambient light has the `Ambient ground color`,
    /// above the second the `Sky color`, in between a blend.
    pub ambient_heights: Vec2,
}

/// Linear interpolation of the sample's field types.
trait Mix: Copy {
    fn mix(self, other: Self, t: f32) -> Self;
}

impl Mix for f32 {
    fn mix(self, o: f32, t: f32) -> f32 {
        self + (o - self) * t
    }
}

impl Mix for Vec3 {
    fn mix(self, o: Vec3, t: f32) -> Vec3 {
        self + (o - self) * t
    }
}

impl Mix for Vec2 {
    fn mix(self, o: Vec2, t: f32) -> Vec2 {
        self + (o - self) * t
    }
}

impl Mix for Vec4 {
    fn mix(self, o: Vec4, t: f32) -> Vec4 {
        self + (o - self) * t
    }
}

impl EnvSample {
    /// Component-wise interpolation between two samples (zone changes, time steps).
    pub fn lerp(&self, other: &EnvSample, t: f32) -> EnvSample {
        macro_rules! mix {
            ($($f:ident),*) => {
                EnvSample { sun_dir: self.sun_dir.mix(other.sun_dir, t).normalize_or(other.sun_dir), $($f: self.$f.mix(other.$f, t)),* }
            };
        }
        mix!(
            sun, sky, ambient_ground, fog, fog_density, ocean_fog, sky_srgb, fog_srgb, night_horizon, night_zenith, day, fog_top,
            fog_layer, fog_ramp, horizon, zenith, zenith_shift, cloud_sun, cloud_sky, skybox_mul, film, saturation, contrast,
            color_balance, levels_min, photofilter, hdr_dynamic, sun_shafts, sun_specular, sky_rayleigh, sky_mie, ambient_heights
        )
    }
}

pub(crate) fn srgb_to_linear(c: Vec3) -> Vec3 {
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
        let (nh, horizon) = col("Night sky: Horizon color", "Night sky: Horizon color multiplier");
        let (nz, zenith) = col("Night sky: Zenith color", "Night sky: Zenith color multiplier");
        let (_, fog_top) = col("Fog color (top)", "Fog color (top) multiplier");
        let cloud_sun = srgb_to_linear(self.color("Sun color", t)) * self.scalar("Cloud shading: Sun light multiplier", t);
        let cloud_sky = srgb_to_linear(self.color("Sky color", t)) * self.scalar("Cloud shading: Sky light multiplier", t);
        // The sun is up between the dawn and dusk colour keys: brightness of the sun colour decides.
        let lum = |c: Vec3| c.dot(Vec3::new(0.2126, 0.7152, 0.0722));
        let sun_srgb = self.color("Sun color", t);
        let day = ((lum(sun_srgb) - 0.25) / 0.5).clamp(0.0, 1.0);
        let balance = self.color("Color balance", t);
        let photo = self.color("Filters: photofilter color", t);
        let wavelength = |n: &str| self.scalar(n, t).max(1.0);
        let (red, green, blue) = (wavelength("Sky light: Wavelength (R)"), wavelength("Sky light: Wavelength (G)"), wavelength("Sky light: Wavelength (B)"));
        let scatter = Vec3::new(1.0, (red / green).powi(4), (red / blue).powi(4));
        let sky_sun = srgb_to_linear(self.color("Sky light: Sun intensity", t));
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
            fog_top,
            fog_layer: Vec4::new(
                self.scalar("Fog height (bottom)", t),
                self.scalar("Fog height (top)", t),
                self.scalar("Fog layer density (bottom)", t),
                self.scalar("Fog layer density (top)", t),
            ),
            fog_ramp: Vec3::new(
                self.scalar("Volumetric fog: Ramp start", t),
                self.scalar("Volumetric fog: Ramp end", t),
                self.scalar("Volumetric fog: Ramp influence", t),
            ),
            horizon,
            zenith,
            zenith_shift: self.scalar("Night sky: Zenith shift", t),
            cloud_sun,
            cloud_sky,
            skybox_mul: self.scalar("Skybox multiplier", t),
            film: Vec4::new(
                self.scalar("Film curve shoulder scale", t),
                self.scalar("Film curve midtones scale", t),
                self.scalar("Film curve toe scale", t),
                self.scalar("Film curve whitepoint", t),
            ),
            saturation: self.scalar("Saturation", t),
            contrast: self.scalar("Contrast", t),
            color_balance: if balance == Vec3::ZERO { Vec3::ONE } else { balance },
            levels_min: self.scalar("Levels: min input", t),
            photofilter: Vec4::new(photo.x, photo.y, photo.z, self.scalar("Filters: photofilter density", t)),
            hdr_dynamic: self.scalar("HDR dynamic power factor", t),
            sun_shafts: self.scalar("Sun shafts visibility", t),
            sun_specular: self.scalar("Sun specular multiplier", t),
            sky_rayleigh: scatter * self.scalar("Sky light: Rayleigh scattering", t) * sky_sun,
            sky_mie: Vec3::splat(self.scalar("Sky light: Mie scattering", t)) * sky_sun,
            ambient_heights: Vec2::new(self.scalar("Ambient min height", t), self.scalar("Ambient max height", t)),
        }
    }
}

/// Direction towards the sun (cry space) at `hour`.
///
/// The sun turns at 15 degrees an hour about an axis that is tilted `PlanetRotationAxis` degrees from the vertical, away from
/// the bearing `SunRotation`; at its highest ([`HIGHEST_HOUR`]) it stands at that bearing and at the elevation
/// `asin(SunHeight / 100)`, in the morning it is east of that. With the main world's 188 / 80 / 25 the sun never sets: at
/// midnight it skims the northern horizon at 3 degrees (the time of day colours make the night). The time of day data's own
/// dawn and dusk (6:00 and 17:30) do not move it.
///
/// Measured: in the real client at game time 9.56 to 10.0 the court yard of the Royal Palace (zone 186) has one patch of sun on
/// its floor; it is where the sun, at an azimuth of 123 degrees (9.56 h) to 132 degrees (10.0 h) and an elevation of 42 to 44
/// degrees, shines through a gap of the buildings (found by rendering the court yard with the sun at every direction and
/// comparing the patch, `docs/aa-world.md`). The path above, with the highest point at 12.85 h, passes through both
/// (123 / 42 and 131 / 45); the two data points leave the culmination hour and the 15 degrees an hour
/// uncertain by about half an hour. The older path (a 11 hour arc with the sun in the north at noon) lit none of the court yard.
pub fn sun_direction(hour: f32, l: Lighting) -> Vec3 {
    let direction = |bearing: f32, elevation: f32| {
        let (b, e) = (bearing.to_radians(), elevation.to_radians());
        // bearing -> cry axes: north = +Y, east = +X
        Vec3::new(b.sin() * e.cos(), b.cos() * e.cos(), e.sin())
    };
    let highest = direction(l.sun_rotation, (l.sun_height / 100.0).clamp(0.05, 1.0).asin().to_degrees());
    // the axis points up and towards the north (away from the sun at its highest)
    let axis = direction(l.sun_rotation + 180.0, 90.0 - l.planet_axis);
    // clockwise seen from above: the bearing grows with the hour
    let angle = -(hour - l.highest_hour) * 15f32.to_radians();
    Quat::from_axis_angle(axis, angle).mul_vec3(highest).normalize()
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
    fn layers_are_separate() {
        let xml = r#"<TimeOfDayList>
            <TimeOfDay relic_layer="0"><Variable Name="Saturation" Value="1"><Spline Keys="0:1:0,1:1:0,"/></Variable></TimeOfDay>
            <TimeOfDay relic_layer="1"><Variable Name="Saturation" Value="1"><Spline Keys="0:0.5:0,1:0.5:0,"/></Variable></TimeOfDay>
            </TimeOfDayList>"#;
        assert!((TimeOfDay::parse(xml).unwrap().scalar("Saturation", 0.5) - 1.0).abs() < 1e-5);
        assert!((TimeOfDay::parse_layer(xml, 0).unwrap().scalar("Saturation", 0.5) - 1.0).abs() < 1e-5);
        assert!((TimeOfDay::parse_layer(xml, 1).unwrap().scalar("Saturation", 0.5) - 0.5).abs() < 1e-5);
    }

    #[test]
    fn samples_blend() {
        let xml = r#"<TimeOfDayList><TimeOfDay>
            <Variable Name="Volumetric fog: Global density" Value="1"><Spline Keys="0:0.2:0,1:0.2:0,"/></Variable>
            </TimeOfDay></TimeOfDayList>"#;
        let t = TimeOfDay::parse(xml).unwrap();
        let a = t.sample(10.0, Lighting::default());
        let mut b = a.clone();
        b.fog_density = 0.6;
        assert!((a.lerp(&b, 0.5).fog_density - 0.4).abs() < 1e-5);
    }

    #[test]
    fn sun_path() {
        let l = Lighting { sun_rotation: 188.0, sun_height: 80.0, planet_axis: 25.0, highest_hour: HIGHEST_HOUR };
        let degrees = |v: Vec3| ((v.x.atan2(v.y).to_degrees() + 360.0) % 360.0, v.z.asin().to_degrees());
        // at its highest: the bearing SunRotation, asin(0.8) up
        let (az, el) = degrees(sun_direction(HIGHEST_HOUR, l));
        assert!((az - 188.0).abs() < 0.1 && (el - 53.13).abs() < 0.1, "{az} {el}");
        // the real client at the Royal Palace: 123 / 42 at 9.56 h, 132 / 44 at 10.0 h
        let (az, el) = degrees(sun_direction(9.563, l));
        assert!((az - 123.0).abs() < 4.0 && (el - 42.0).abs() < 3.0, "{az} {el}");
        let (az, el) = degrees(sun_direction(9.998, l));
        assert!((az - 132.0).abs() < 4.0 && (el - 44.0).abs() < 3.0, "{az} {el}");
        // east in the morning, west in the evening, never far below the horizon
        assert!(sun_direction(6.0, l).x > 0.5 && sun_direction(18.0, l).x < -0.5);
        for h in 0..24 {
            assert!(sun_direction(h as f32, l).z > 0.0, "hour {h}");
        }
        // no axis tilt: the sun circles at the same height
        let flat = Lighting { planet_axis: 0.0, ..l };
        assert!((sun_direction(0.0, flat).z - 0.8).abs() < 1e-4 && (sun_direction(15.0, flat).z - 0.8).abs() < 1e-4);
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
