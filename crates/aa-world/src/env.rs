//! Environment data: `zone/<id>/time_of_day.xml`, `env.xml`, `lighting.xml`.
//!
//! `time_of_day.xml` is CryEngine 3's time of day: `<TimeOfDay ...>` blocks (one per relic / weather
//! layer; the first is the default) of `<Variable Name=.. Value=..|Color="r,g,b"><Spline Keys="t:v:flags,.."/>`.
//! Keys are `time:value:tangentFlags` with `time` in `0..1` of the day (`hours / 24`), colours are
//! `time:(r:g:b):flags`. Colours are sRGB encoded; `* multiplier` variables carry the HDR scale
//! (sun ~ 6..8, sky ~ 2..6 at the engine's exposure).
//!
//! `lighting.xml` gives the sun path (see [`sun_direction`], the engine's own formula): `SunRotation`, `Latitude` and
//! `PlanetRotationAxis`; the editor's `SunHeight` and `SunVector` are not used by the engine.

use std::collections::HashMap;

use bevy::math::{Mat3, Vec2, Vec3, Vec4};

#[path = "env_defaults.rs"]
mod defaults;

/// One XML Bézier key. Low three flag bits select the incoming tangent, the next
/// three the outgoing tangent. Higher bits are preserved editor flags.
#[derive(Clone, Copy, Debug, Default)]
pub struct SplineKey {
    pub time: f32,
    pub value: [f32; 3],
    pub flags: u32,
}

/// A time of day spline (scalar or colour).
#[derive(Clone, Debug, Default)]
pub struct Spline {
    /// Native keys may extend slightly beyond 0..1; scalar values use `x`.
    pub keys: Vec<SplineKey>,
    pub is_color: bool,
    /// The editor's current value. It does not replace initialized default keys.
    pub value: [f32; 3],
    /// Native scalar metadata limits; RGB variables clamp to 0..1 after evaluation.
    pub range: Option<[f32; 2]>,
}

impl Spline {
    /// Native CBezierSplineFloat/Vec3 evaluation. Endpoint clamping, binary segment
    /// lookup and at most four neighboring keys; no allocations or curve rebuild.
    pub fn eval(&self, t: f32) -> [f32; 3] {
        let k = &self.keys;
        let Some(first) = k.first() else { return self.value };
        if t < first.time || t.is_nan() {
            return first.value;
        }
        let next = k.partition_point(|key| key.time <= t);
        if next >= k.len() {
            return k.last().unwrap().value;
        }
        let (a, b) = (&k[next - 1], &k[next]);
        if (a.flags >> 3) & 7 == 3 {
            return b.value;
        }
        if b.flags & 7 == 3 {
            return a.value;
        }
        let u = (t - a.time) / (b.time - a.time);
        let (u2, u3) = (u * u, u * u * u);
        let b0 = 3.0 * u2 - u3 - 3.0 * u + 1.0;
        let b1 = 3.0 * u3 - 6.0 * u2 + 3.0 * u;
        let b2 = 3.0 * u2 - 3.0 * u3;
        let outgoing = self.tangent(next - 1, false);
        let incoming = self.tangent(next, true);
        [0, 1, 2].map(|i| a.value[i] * b0 + (a.value[i] + outgoing[i]) * b1
            + (b.value[i] - incoming[i]) * b2 + b.value[i] * u3)
    }

    // Exact 300ABDD0 / 300AC0A0 controls. XML serializes flags, not custom tangent
    // values: free (1) retains the zero controls established by native InsertKey.
    fn tangent(&self, i: usize, incoming: bool) -> [f32; 3] {
        let k = &self.keys;
        let mode = (k[i].flags >> if incoming { 0 } else { 3 }) & 7;
        if mode == 1 || k.len() < 2 {
            return [0.0; 3];
        }
        let delta = |a: usize, b: usize| [0, 1, 2].map(|c| (k[b].value[c] - k[a].value[c]) / 3.0);
        // Native endpoint rules precede the interior tangent-mode switch.
        if i == 0 {
            return if incoming { [0.0; 3] } else { delta(0, 1) };
        }
        if i == k.len() - 1 {
            return if incoming { delta(i - 1, i) } else { [0.0; 3] };
        }
        match mode {
            2 | 3 => [0.0; 3],
            4 => if incoming { delta(i - 1, i) } else { delta(i, i + 1) },
            _ => {
                let span = k[i + 1].time - k[i - 1].time;
                if span <= 0.0 { return [0.0; 3] }
                let weight = (k[i].time - k[i - 1].time) / span;
                delta(i - 1, i + 1).map(|v| v * if incoming { weight } else { 1.0 - weight })
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct TimeOfDay {
    pub variables: HashMap<String, Spline>,
    /// `Time` attribute: the editor's current hour.
    pub default_hour: f32,
}

impl Default for TimeOfDay {
    fn default() -> Self {
        let variables = defaults::VARIABLES.iter().map(|(name, color, bits)| {
            let v = bits.map(f32::from_bits);
            let value = if *color { v } else { [v[0], 0.0, 0.0] };
            let keys = [0.0, 1.0].map(|time| SplineKey { time, value, flags: 0 }).to_vec();
            ((*name).to_string(), Spline { keys, is_color: *color, value,
                range: (!*color).then_some([v[1], v[2]]) })
        }).collect();
        Self { variables, default_hour: 12.0 }
    }
}

fn parse_floats(s: &str) -> Vec<f32> {
    s.split([',', ':', '(', ')'])
        .filter(|p| !p.trim().is_empty())
        .filter_map(|p| p.trim().parse().ok())
        .collect()
}

fn parse_spline(keys: &str, color: bool) -> Vec<SplineKey> {
    let mut out = Vec::new();
    // keys are separated by ',' but colour values contain ':' only, so split on ',' at depth 0
    for key in keys.split(',').filter(|k| !k.trim().is_empty()) {
        let v = parse_floats(key);
        let count = if color { 4 } else { 2 };
        if v.len() >= count && v[..count].iter().all(|v| v.is_finite()) {
            let value = if color { [v[1], v[2], v[3]] } else { [v[1], 0.0, 0.0] };
            let flags = if v.len() > count { key.rsplit(':').next().and_then(|s| s.trim().parse().ok()).unwrap_or(0) } else { 0 };
            out.push(SplineKey { time: v[0], value, flags });
        }
    }
    out.sort_by(|a, b| a.time.total_cmp(&b.time));
    out
}

impl TimeOfDay {
    /// Layer 0 (the default `relic_layer` / `weather_layer`).
    pub fn parse(xml: &str) -> Option<TimeOfDay> {
        Self::parse_layer(xml, 0)
    }

    /// A zone list initializes two native default layers, then loads matching blocks.
    /// An absent layer keeps its defaults. A standalone TimeOfDay block applies to both.
    pub fn parse_layer(xml: &str, layer: u32) -> Option<TimeOfDay> {
        let doc = roxmltree::Document::parse(xml).ok()?;
        if layer > 1 { return None }
        let mut tod = Self::default();
        let want = layer.to_string();
        let root = doc.root_element();
        let block = if root.has_tag_name("TimeOfDay") { Some(root) } else if root.has_tag_name("TimeOfDayList") {
            root.children().filter(|n| n.has_tag_name("TimeOfDay"))
                .find(|n| n.attribute("relic_layer").unwrap_or("0") == want && n.attribute("weather_layer").unwrap_or("0") == "0")
        } else { return None };
        let Some(block) = block else { return Some(tod) };
        tod.default_hour = block.attribute("Time").and_then(|v| v.parse().ok()).unwrap_or(12.0);
        let legacy = block.attribute("sRGB").and_then(|value| value.parse::<i32>().ok()).unwrap_or(0) != 1;
        for v in block.children().filter(|n| n.has_tag_name("Variable")) {
            let Some(name) = v.attribute("Name") else { continue };
            let name = if name == "HDR blueshift" { "Blueshift" } else { name };
            let Some(spline) = tod.variables.get_mut(name) else { continue };
            // Native 300A8190 leaves these initialized when migrating pre-sRGB XML.
            if legacy && (name.eq_ignore_ascii_case("HDR dynamic power factor") || name.to_ascii_lowercase().contains("film")) {
                continue;
            }
            if spline.is_color && let Some(c) = v.attribute("Color") {
                let f = parse_floats(c);
                if f.len() >= 3 { spline.value = [f[0], f[1], f[2]]; }
            } else if !spline.is_color && let Some(value) = v.attribute("Value").and_then(|x| x.parse().ok()) {
                spline.value[0] = value;
            }
            if let Some(mut keys) = v
                .children()
                .find(|n| n.has_tag_name("Spline"))
                .and_then(|s| s.attribute("Keys"))
                .map(|k| parse_spline(k, spline.is_color))
                && !keys.is_empty()
            {
                if legacy && ["Sun color", "Sun color multiplier", "Sun specular multiplier",
                    "Sky color", "Sky color multiplier", "Global Rim Amount"].contains(&name)
                {
                    // Native converts each stored key via double pow, then rounds to
                    // float, with the f32 exponent at 3025B3F8 promoted to double.
                    for key in &mut keys {
                        key.value = key.value.map(|value| f64::from(value).powf(f64::from(2.2_f32)) as f32);
                    }
                }
                spline.keys = keys;
            }
        }
        Some(tod)
    }

    pub fn scalar(&self, name: &str, t: f32) -> f32 {
        let name = if name == "HDR blueshift" { "Blueshift" } else { name };
        self.variables.get(name).map_or(0.0, |s| {
            let mut value = s.eval(t)[0];
            if let Some([min, max]) = s.range { value = value.clamp(min, max); }
            if value.abs() < 1e-10 { 0.0 } else { value }
        })
    }

    pub fn color(&self, name: &str, t: f32) -> Vec3 {
        let name = if name == "HDR blueshift" { "Blueshift" } else { name };
        self.variables.get(name).map_or(Vec3::ZERO, |s| Vec3::from(s.eval(t)).clamp(Vec3::ZERO, Vec3::ONE))
    }
}

/// `lighting.xml` sun path settings, as the engine's `CTimeOfDay` reads them (with its defaults for missing attributes).
#[derive(Clone, Copy, Debug)]
pub struct Lighting {
    /// `SunRotation` (default 240): turns the whole path about the vertical.
    pub sun_rotation: f32,
    /// `Latitude` (default 90): tilts the plane of the path about the east-west axis (`90 - Latitude`).
    pub latitude: f32,
    /// `PlanetRotationAxis` (default 0): offsets the circle along its axis by `sin(PlanetRotationAxis)`, a small circle
    /// instead of a great one.
    pub planet_axis: f32,
    /// `DawnTime / 60` (hours; default 355 / 60): where the world's real dawn is, which the engine's time of day manager moves
    /// onto the splines' authored 6:00 (and, mirrored as `24 - dawn_hour`, the dusk onto 18:00); see [`spline_hour`].
    /// 6.0 (login worlds, most instances) leaves the hour as it is.
    pub dawn_hour: f32,
}

impl Default for Lighting {
    /// The main world's.
    fn default() -> Self {
        Lighting { sun_rotation: 188.0, latitude: 154.0, planet_axis: 25.0, dawn_hour: 119.0 / 60.0 }
    }
}

impl Lighting {
    pub fn parse(xml: &str) -> Option<Lighting> {
        let doc = roxmltree::Document::parse(xml).ok()?;
        let l = doc.descendants().find(|n| n.has_tag_name("Lighting"))?;
        let number = |name: &str, default: f32| l.attribute(name).and_then(|v| v.parse().ok()).unwrap_or(default);
        Some(Lighting {
            sun_rotation: number("SunRotation", 240.0),
            latitude: number("Latitude", 90.0),
            planet_axis: number("PlanetRotationAxis", 0.0),
            dawn_hour: number("DawnTime", 355.0) / 60.0,
        })
    }
}

/// The `<HDRSetup>` node of a world's `env.xml` (the engine's `HDRSetup` of `cry3dengine.dll`): the colour and the multiplier of
/// the bloom, which `FilmicMapping` adds to the exposed scene. The 1.2 main world has `BloomColor="204,244,255" BloomMul="1.8"` (a
/// cool white), login2 `255,255,255` and 1.5; `FlaresMul` and `StreaksMul` are 0 (login2: flares 0.1), so there are no lens flares or
/// streaks to draw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HdrSetup {
    /// `BloomColor`, 0..1 (the file has 0..255).
    pub bloom_color: Vec3,
    /// `BloomMul`, clamped to 0..10 like the engine does.
    pub bloom_mul: f32,
}

impl Default for HdrSetup {
    /// The engine's defaults for a file without the node.
    fn default() -> Self {
        HdrSetup { bloom_color: Vec3::ONE, bloom_mul: 1.0 }
    }
}

impl HdrSetup {
    pub fn parse(xml: &str) -> Option<HdrSetup> {
        let doc = roxmltree::Document::parse(xml).ok()?;
        let n = doc.descendants().find(|n| n.has_tag_name("HDRSetup"))?;
        let mut h = HdrSetup::default();
        if let Some(c) = n.attribute("BloomColor") {
            let v: Vec<f32> = c.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            if v.len() == 3 {
                h.bloom_color = Vec3::new(v[0], v[1], v[2]) / 255.0;
            }
        }
        if let Some(m) = n.attribute("BloomMul").and_then(|m| m.trim().parse::<f32>().ok()) {
            h.bloom_mul = m.clamp(0.0, 10.0);
        }
        Some(h)
    }
}

/// One evaluated frame of the environment (linear colours, `Vec3` rgb).
#[derive(Clone, Debug)]
pub struct EnvSample {
    /// Direction *towards* the sun (cry space, Z up).
    pub sun_dir: Vec3,
    /// Native primary-moon path: the same orbital matrix, phase advanced by half a day.
    pub moon_dir: Vec3,
    /// `Sun color * multiplier`, linear.
    pub sun: Vec3,
    pub sky: Vec3,
    pub ambient_ground: Vec3,
    pub fog: Vec3,
    /// Native SetFogColor RGB: raw sampled colour * (multiplier * 2^HDR power).
    /// Kept separate from the calibrated, IEC-decoded `fog` surface/post sample.
    pub native_fog: Vec3,
    /// Native ordinary-game SunColor + SkyColor RGB, before the existing renderer-unit
    /// conversion: raw colours * (their multipliers * 2^HDR power), then summed.
    /// Kept separate from the calibrated IEC-decoded surface/post colours.
    pub native_sun_sky: Vec3,
    pub fog_density: f32,
    pub ocean_fog: Vec3,
    /// Raw sRGB colours (before multipliers), for sky gradients.
    pub sky_srgb: Vec3,
    pub fog_srgb: Vec3,
    pub night_horizon: Vec3,
    pub night_zenith: Vec3,
    /// Authored `Night sky: Star intensity`, evaluated at the same warped hour as the sky.
    pub star_intensity: f32,
    pub moons: [MoonSample; 4],
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
    /// `Levels: min input`, in the engine's `AdjustLevels` input units (0..255 of the sRGB range, not a 0..1 fraction).
    pub levels_min: f32,
    /// `Filters: photofilter` colour (sRGB) and density.
    pub photofilter: Vec4,
    /// `HDR dynamic power factor`: a stop offset of the colours' HDR multipliers (not an eye adaptation range).
    pub hdr_dynamic: f32,
    /// `Sun shafts visibility`.
    pub sun_shafts: f32,
    /// `Sun rays visibility` (length of the rays) and `Sun rays attenuation` (fall off with the distance from the sun).
    pub sun_rays: f32,
    pub sun_rays_atten: f32,
    pub sun_specular: f32,
    /// The sky light's Rayleigh scattering: `Rayleigh scattering x (wavelength R / wavelength)^4 x Sky light: Sun intensity`
    /// per channel (linear); blue scatters about five times more than red at 500 nm.
    pub sky_rayleigh: Vec3,
    /// The sky light's Mie scattering (white haze): `Mie scattering x Sky light: Sun intensity`, linear.
    pub sky_mie: Vec3,
    /// `Ambient min height` / `Ambient max height` (metres): below the first the ambient light has the `Ambient ground color`,
    /// above the second the `Sky color`, in between a blend.
    pub ambient_heights: Vec2,
    /// The two `Char BackLight` lights, which the engine applies to characters only (`illum` / `humanskin` / `hair` /
    /// `cloth` / `metal` under `%_RT_OBJECT_BACKLIGHT`, direction from the character's `.cdf`): `rgb` = `"i Color"` (linear)
    /// `x "i Diffuse Multiplier"` (engine renderer parameter 71, `"i HDR dynamic"` is 0 in the main world and not applied),
    /// `w` = `"i Specular Multiplier"` (parameter 72). Engine light units; the engine adds them to the ambient light's sum, see
    /// `crate::char_light` for the conversion to lux. A file without the group (an old one) has no back light: colour 0.
    pub char_back: [Vec4; 2],
    /// `Char BackLight: InShadowIntensity`.
    pub char_back_shadow: f32,
    /// `Ocean fog density under water`: the sea's extinction per metre (base 2) seen from below the surface.
    pub ocean_under: f32,
}

/// Renderer parameters 30 / 32..36: native sampled float RGB times multipliers / HDR scale.
/// Unlike the calibrated surface-light samples, these retain the engine's RGB values unchanged.
#[derive(Clone, Copy, Debug, Default)]
pub struct MoonSample {
    pub color: Vec3,
    pub inner: Vec3,
    pub outer: Vec3,
    pub inner_scale: f32,
    pub outer_scale: f32,
    pub outer_size: f32,
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

impl<T: Mix, const N: usize> Mix for [T; N] {
    fn mix(self, o: [T; N], t: f32) -> [T; N] {
        let mut i = 0;
        self.map(|a| {
            i += 1;
            a.mix(o[i - 1], t)
        })
    }
}

impl Mix for MoonSample {
    fn mix(self, o: Self, t: f32) -> Self {
        Self { color: self.color.mix(o.color, t), inner: self.inner.mix(o.inner, t), outer: self.outer.mix(o.outer, t),
            inner_scale: self.inner_scale.mix(o.inner_scale, t), outer_scale: self.outer_scale.mix(o.outer_scale, t),
            outer_size: self.outer_size.mix(o.outer_size, t) }
    }
}

impl EnvSample {
    /// Component-wise interpolation utility for evaluated samples. Native zone blending uses
    /// [`TimeOfDay::sample_blended`] before color conversion and HDR multiplication instead.
    pub fn lerp(&self, other: &EnvSample, t: f32) -> EnvSample {
        macro_rules! mix {
            ($($f:ident),*) => {
                EnvSample { sun_dir: self.sun_dir.mix(other.sun_dir, t).normalize_or(other.sun_dir),
                    moon_dir: self.moon_dir.mix(other.moon_dir, t).normalize_or(other.moon_dir), $($f: self.$f.mix(other.$f, t)),* }
            };
        }
        mix!(
            sun, sky, ambient_ground, fog, native_fog, native_sun_sky, fog_density, ocean_fog, sky_srgb, fog_srgb, night_horizon, night_zenith, star_intensity, moons, day, fog_top,
            fog_layer, fog_ramp, horizon, zenith, zenith_shift, cloud_sun, cloud_sky, skybox_mul, film, saturation, contrast,
            color_balance, levels_min, photofilter, hdr_dynamic, sun_shafts, sun_rays, sun_rays_atten, sun_specular, sky_rayleigh, sky_mie, ambient_heights,
            char_back, char_back_shadow, ocean_under
        )
    }
}

pub(crate) fn srgb_to_linear(c: Vec3) -> Vec3 {
    let f = |x: f32| if x <= 0.04045 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) };
    Vec3::new(f(c.x), f(c.y), f(c.z))
}

impl TimeOfDay {
    /// Evaluates the environment at the game hour `hour` (0..24): the splines are read at [`spline_hour`] of it, the sun path
    /// and everything else take the raw hour.
    pub fn sample(&self, hour: f32, lighting: Lighting) -> EnvSample {
        let t = spline_hour(hour, lighting.dawn_hour) / 24.0;
        Self::sample_values(hour, lighting, |name| self.scalar(name, t), |name| self.color(name, t))
    }

    /// Native 300A5010: evaluate each layer, normalize its weight, blend raw scalar/RGB
    /// values, then apply color-space conversion and HDR multipliers in UpdateEngine.
    /// Blending already-lit EnvSamples would change both color and multiplier behavior.
    pub fn sample_blended(layers: &[(&TimeOfDay, f32)], hour: f32, lighting: Lighting) -> EnvSample {
        let total: f32 = layers.iter().filter(|(_, w)| w.is_finite() && *w > 0.0).map(|(_, w)| w).sum();
        if total <= 0.0 {
            static EMPTY: std::sync::OnceLock<TimeOfDay> = std::sync::OnceLock::new();
            return EMPTY.get_or_init(Self::default).sample(hour, lighting);
        }
        let t = spline_hour(hour, lighting.dawn_hour) / 24.0;
        let scalar = |name: &str| layers.iter().filter(|(_, w)| w.is_finite() && *w > 0.0)
            .map(|(tod, weight)| tod.scalar(name, t) * (*weight / total)).sum();
        let color = |name: &str| layers.iter().filter(|(_, w)| w.is_finite() && *w > 0.0)
            .fold(Vec3::ZERO, |sum, (tod, weight)| sum + tod.color(name, t) * (*weight / total));
        Self::sample_values(hour, lighting, scalar, color)
    }

    fn sample_values(
        hour: f32,
        lighting: Lighting,
        scalar: impl Fn(&str) -> f32,
        color: impl Fn(&str) -> Vec3,
    ) -> EnvSample {
        let col = |name: &str, mul: &str| {
            let c = color(name);
            let m = if mul.is_empty() { 1.0 } else { scalar(mul) };
            (c, srgb_to_linear(c) * m)
        };
        let (sun_raw, sun) = col("Sun color", "Sun color multiplier");
        let (sky_srgb, sky) = col("Sky color", "Sky color multiplier");
        let (_, ambient_ground) = col("Ambient ground color", "Ambient ground color multiplier");
        let (fog_srgb, fog) = col("Fog color", "Fog color multiplier");
        // 300A5DD0 computes this product after raw layer blending, then 301235E0
        // stores all three floats unchanged. HDR renderer base (30121B70) is 2.
        let native_fog = fog_srgb * (scalar("Fog color multiplier") * scalar("HDR dynamic power factor").exp2());
        // 300A729D /300A7362 -> setters30123410 /30123340 -> getters30123860
        // /30123810 -> renderer38077AA8 /3806FD90 preserve this RGB without IEC decode.
        let native_hdr = scalar("HDR dynamic power factor").exp2();
        let native_sun_sky = sun_raw * (scalar("Sun color multiplier") * native_hdr)
            + sky_srgb * (scalar("Sky color multiplier") * native_hdr);
        let (_, ocean_fog) = col("Ocean fog color", "Ocean fog color multiplier");
        let (nh, horizon) = col("Night sky: Horizon color", "Night sky: Horizon color multiplier");
        let (nz, zenith) = col("Night sky: Zenith color", "Night sky: Zenith color multiplier");
        let (_, fog_top) = col("Fog color (top)", "Fog color (top) multiplier");
        let cloud_sun = srgb_to_linear(color("Sun color")) * scalar("Cloud shading: Sun light multiplier");
        let cloud_sky = srgb_to_linear(color("Sky color")) * scalar("Cloud shading: Sky light multiplier");
        // The sun is up between the dawn and dusk colour keys: brightness of the sun colour decides.
        let lum = |c: Vec3| c.dot(Vec3::new(0.2126, 0.7152, 0.0722));
        let sun_srgb = color("Sun color");
        let day = ((lum(sun_srgb) - 0.25) / 0.5).clamp(0.0, 1.0);
        let balance = color("Color balance");
        let photo = color("Filters: photofilter color");
        let wavelength = |n: &str| scalar(n).max(1.0);
        let (red, green, blue) = (wavelength("Sky light: Wavelength (R)"), wavelength("Sky light: Wavelength (G)"), wavelength("Sky light: Wavelength (B)"));
        let scatter = Vec3::new(1.0, (red / green).powi(4), (red / blue).powi(4));
        let sky_sun = srgb_to_linear(color("Sky light: Sun intensity"));
        let back = |i: u32| {
            let (_, c) = col(&format!("{i} Color"), &format!("{i} Diffuse Multiplier"));
            c.extend(scalar(&format!("{i} Specular Multiplier")))
        };
        EnvSample {
            sun_dir: sun_direction(hour, lighting),
            moon_dir: sun_direction(hour + 12.0, lighting),
            sun,
            sky,
            ambient_ground,
            fog,
            native_fog,
            native_sun_sky,
            fog_density: scalar("Volumetric fog: Global density"),
            ocean_fog,
            sky_srgb,
            fog_srgb,
            night_horizon: nh,
            night_zenith: nz,
            star_intensity: scalar("Night sky: Star intensity"),
            moons: std::array::from_fn(|i| {
                let prefix = if i == 0 { "Night sky: Moon".to_string() } else { format!("Night sky: Moon{i}") };
                // Native UpdateEngineMoonParameters multiplies the sampled float RGB directly;
                // SetGlobalParameter copies it unchanged. The engine HDR base defaults to 2.
                let hdr = scalar("HDR dynamic power factor").exp2();
                let color = |suffix: &str| color(&format!("{prefix}{suffix} color"))
                    * scalar(&format!("{prefix}{suffix} color multiplier")) * hdr;
                MoonSample { color: color(""), inner: color(" inner corona"), outer: color(" outer corona"),
                    inner_scale: scalar(&format!("{prefix} inner corona scale")),
                    outer_scale: scalar(&format!("{prefix} outer corona scale")),
                    outer_size: scalar(&format!("{prefix} outer corona size")) }
            }),
            day,
            fog_top,
            fog_layer: Vec4::new(
                scalar("Fog height (bottom)"),
                scalar("Fog height (top)"),
                scalar("Fog layer density (bottom)"),
                scalar("Fog layer density (top)"),
            ),
            fog_ramp: Vec3::new(
                scalar("Volumetric fog: Ramp start"),
                scalar("Volumetric fog: Ramp end"),
                scalar("Volumetric fog: Ramp influence"),
            ),
            horizon,
            zenith,
            zenith_shift: scalar("Night sky: Zenith shift"),
            cloud_sun,
            cloud_sky,
            skybox_mul: scalar("Skybox multiplier"),
            film: Vec4::new(
                scalar("Film curve shoulder scale"),
                scalar("Film curve midtones scale"),
                scalar("Film curve toe scale"),
                scalar("Film curve whitepoint"),
            ),
            saturation: scalar("Saturation"),
            contrast: scalar("Contrast"),
            color_balance: balance,
            levels_min: scalar("Levels: min input"),
            photofilter: Vec4::new(photo.x, photo.y, photo.z, scalar("Filters: photofilter density")),
            hdr_dynamic: scalar("HDR dynamic power factor"),
            sun_shafts: scalar("Sun shafts visibility"),
            sun_rays: scalar("Sun rays visibility"),
            sun_rays_atten: scalar("Sun rays attenuation"),
            sun_specular: scalar("Sun specular multiplier"),
            sky_rayleigh: scatter * scalar("Sky light: Rayleigh scattering") * sky_sun,
            sky_mie: Vec3::splat(scalar("Sky light: Mie scattering")) * sky_sun,
            ambient_heights: Vec2::new(scalar("Ambient min height"), scalar("Ambient max height")),
            char_back: [back(1), back(2)],
            char_back_shadow: scalar("InShadowIntensity"),
            ocean_under: scalar("Ocean fog density under water"),
        }
    }
}

/// The hour the zone's time of day splines are read at for the game hour `hour`: the engine's time of day manager (`SetTime`
/// -> vtable slot 14, `cry3dengine.dll` 0x300A49D0) hands every zone layer a warped hour, a quadratic per half day that moves
/// the world's real dawn and dusk (`lighting.xml` `DawnTime`, `dawn` = `DawnTime / 60`; main world 119 -> 1.9833, so 2:00 and
/// 22:00) onto the 6:00 and 18:00 the splines were authored for. The second half's quadratic is clamped at 0, so on the main
/// world every hour from 12:00 to 19:05 reads the splines at 12.0 (a long afternoon of noon light; 19.17 h reads 12.17), and
/// 2.7 h reads 6.7 (morning). `dawn` = 6 (login worlds and most instances) is the identity. The sun path (`sun_direction`),
/// `e_time_of_day`, the HUD clock all keep the raw hour. `docs/research/time-of-day.md`.
pub fn spline_hour(hour: f32, dawn: f32) -> f32 {
    // bend `k` of the half day quadratic `y = k u^2 + (1 - k) u`; `anchor` in hours
    fn bend(anchor: f32) -> f32 {
        let v = anchor / 12.0;
        if v >= 1.0 {
            let w = v - 1.0;
            (2.0 - 4.0 * w) / (4.0 * w * w - 4.0 * w)
        } else {
            (v - 0.5) * -4.0
        }
    }
    let h = hour.rem_euclid(24.0);
    if h < 12.0 {
        let (u, k) = (h / 12.0, bend(dawn));
        let y = if k.abs() < 1e-6 { u } else { (((1.0 - k).powi(2) + 4.0 * k * u).sqrt() + k - 1.0) * 0.5 / k };
        y.max(0.0) * 12.0
    } else {
        let (u, k) = ((h - 12.0) / 12.0, bend(24.0 - dawn));
        let y = if k.abs() < 1e-6 { u } else { u * u * k + (1.0 - k) * u };
        (y.max(0.0) + 1.0) * 12.0
    }
}

/// Direction towards the sun (cry space) at `hour`: `CTimeOfDay::Update` of `cry3dengine.dll`, in closed form.
///
/// The engine multiplies four matrices, `T(0, 0, -sin axis) x Rz(angle) x Rx(latitude - 90) x Ry(-rotation)`, inverts the product
/// and transforms the point (0, 1, 0) by it; the result, taken as (x, y up, z south), is the sun vector. `angle` is
/// `(hour + 12) / 24` of a turn, so the sun is highest at 12:00 and turns 15 degrees an hour. With the main world's 188 / 154 / 25
/// the sun rises at 2:00 in the north-east, is at its highest at 12:00 (azimuth 172, elevation 49), and sets at 22:00 in the
/// north-west (`DawnTime` 119 and `DuskTime` 599 of the file say the same). The time of day data's own colours make the dawn
/// and dusk; at/below the horizon the directional light follows the separate half-day moon orbit.
///
/// Cross-checked against the real client: at game time 9.56 h and 10.0 h the court yard of the Royal Palace (zone 186) has one
/// patch of sun on its floor, which is where the sun shines through a gap of the buildings from azimuth 123 / elevation 42 and
/// 132 / 44 (rendering the yard with the sun at every direction and comparing the patch, `docs/aa-world.md`); this gives
/// 124.0 / 42.4 and 131.8 / 44.4.
pub fn sun_direction(hour: f32, l: Lighting) -> Vec3 {
    let (rotation, latitude, axis) = (l.sun_rotation.to_radians(), l.latitude.to_radians(), l.planet_axis.to_radians());
    let angle = (hour + 12.0) / 24.0 * std::f32::consts::TAU;
    // M^-1 (0, 1, 0) = Ry(rotation) Rx(90 - latitude) Rz(-angle) (0, 1, sin axis)
    let p = Mat3::from_rotation_y(rotation)
        * Mat3::from_rotation_x(std::f32::consts::FRAC_PI_2 - latitude)
        * Mat3::from_rotation_z(-angle)
        * Vec3::new(0.0, 1.0, axis.sin());
    Vec3::new(p.x, -p.z, p.y).normalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_spline_clamps_endpoints_and_uses_bezier_controls() {
        let s = Spline { keys: parse_spline("0:1:0,0.5:3:0,1:1:0,", false), ..Default::default() };
        assert!((s.eval(0.25)[0] - 2.25).abs() < 1e-5);
        assert!((s.eval(0.75)[0] - 2.25).abs() < 1e-5);
        assert_eq!(s.eval(-0.25)[0], 1.0);
        assert_eq!(s.eval(1.25)[0], 1.0);
    }

    #[test]
    fn native_x86_nonuniform_tangent_and_step_goldens() {
        // Actual original x86 functions executed in bounded Unicorn: 300ABDD0 +
        // 300AF720/300AE830, not a second implementation of the cubic formula.
        for (flags, left, right) in [
            (0, 1.15, 2.3648438), (1, 1.25, 2.3648438), (2, 1.25, 2.3648438),
            (3, 0.0, 2.3648438), (4, 1.0, 2.3648438), (8, 1.15, 2.0585938),
            (16, 1.15, 2.0585938), (24, 1.15, 4.0), (32, 1.15, 2.25),
            (65536, 1.15, 2.3648438),
        ] {
            let s = Spline { keys: parse_spline(&format!("0:0:0,0.2:2:{flags},1:4:0,"), false), ..Default::default() };
            assert!((s.eval(0.1)[0] - left).abs() < 2e-6, "incoming flags={flags}");
            assert!((s.eval(0.3)[0] - right).abs() < 2e-6, "outgoing flags={flags}");
            assert_eq!(s.keys[1].flags, flags);
        }
    }

    #[test]
    fn native_defaults_catalog_missing_layers_and_editor_values() {
        let tod = TimeOfDay::default();
        assert_eq!(tod.variables.len(), 147);
        assert_eq!(tod.scalar("Saturation", 0.0), 1.0);
        assert_eq!(tod.scalar("Film curve whitepoint", 0.0), 1.0);
        assert_eq!(tod.scalar("Skybox multiplier", 0.0), 1.0);
        assert_eq!(tod.scalar("Sun color multiplier", 0.0), 2.4);
        assert_eq!(tod.scalar("InShadowIntensity", 0.0), 0.5);
        assert_eq!(tod.scalar("Ocean fog density under water", 0.0), 0.04);
        assert_eq!(tod.color("Sun color", 0.0).to_array().map(f32::to_bits), [0x3f57d7d9, 0x3f48c8ca, 0x3f2aaaab]);
        assert_eq!(tod.color("HDR blueshift", 0.37), tod.color("Blueshift", 0.37));
        // Native allocated defaults retain their initialized keys when XML has only
        // an editor Value/Color attribute, with no serialized Spline child.
        let xml = r#"<TimeOfDayList><TimeOfDay relic_layer="0">
            <Variable Name="Saturation" Value="8"/>
            <Variable Name="Sun color" Color="0,0,0"/>
            <Variable Name="Not a native variable" Value="42"/>
            </TimeOfDay></TimeOfDayList>"#;
        let loaded = TimeOfDay::parse(xml).unwrap();
        assert_eq!(loaded.variables["Saturation"].value[0], 8.0);
        assert_eq!(loaded.scalar("Saturation", 0.37), tod.scalar("Saturation", 0.37));
        assert_eq!(loaded.color("Sun color", 0.37), tod.color("Sun color", 0.37));
        assert!(!loaded.variables.contains_key("Not a native variable"));
        let absent = TimeOfDay::parse_layer(xml, 1).unwrap();
        assert_eq!(absent.scalar("Sun color multiplier", 0.0), 2.4);
        assert!(TimeOfDay::parse_layer(xml, 2).is_none());
        let limited = TimeOfDay::parse(r#"<TimeOfDay sRGB="1">
            <Variable Name="Saturation"><Spline Keys="0:99:0,1:99:0,"/></Variable>
            <Variable Name="Sun color"><Spline Keys="0:(-1:2:0.5):0,1:(-1:2:0.5):0,"/></Variable>
            <Variable Name="Color balance"><Spline Keys="0:(0:0:0):0,1:(0:0:0):0,"/></Variable>
            </TimeOfDay>"#).unwrap();
        assert_eq!(limited.scalar("Saturation", 0.2), 10.0);
        assert_eq!(limited.color("Sun color", 0.2), Vec3::new(0.0, 1.0, 0.5));
        assert_eq!(limited.sample(10.0, Lighting::default()).color_balance, Vec3::ZERO);
    }

    #[test]
    fn real_solzreed_curves_match_original_x86() {
        if !crate::pak_available() { return }
        let xml = aa_pak::Pak::global().read_string("worlds/main_world/zone/179/time_of_day.xml").unwrap();
        let tod = TimeOfDay::parse(&xml).unwrap();
        // Direct native RGB tangent/evaluation results (300AC0A0, 300AF640).
        for (name, t, expected) in [
            ("Sun color", 0.26, [0.73387384, 0.6581154, 0.5029779]),
            ("Sun color", 0.278, [0.9459861, 0.84307146, 0.598666]),
            ("Sun color", 0.63, [0.9899174, 0.9050442, 0.7649168]),
            ("Sun color", 0.72, [0.7406572, 0.18789124, 0.02058377]),
            ("Sky color", 0.275, [0.30772263, 0.4202817, 0.49660057]),
            ("Sky color", 0.4, [0.67518896, 0.79674417, 0.56262916]),
            ("Sky color", 0.7, [0.66330814, 0.24146298, 0.10244907]),
            ("Fog color", 0.25, [0.27412897, 0.2534206, 0.24647745]),
            ("Fog color", 0.7, [0.4693937, 0.4673743, 0.6565209]),
            ("Fog color", 0.75, [0.01540425, 0.03778355, 0.074057065]),
        ] {
            assert!((tod.color(name, t) - Vec3::from(expected)).abs().max_element() < 3e-6, "{name} at {t}");
        }
        for (name, t, expected) in [
            ("Sun color multiplier", 0.137, 9.016159), ("Sun color multiplier", 0.26, 6.942363),
            ("Sun color multiplier", 0.71, 6.460512), ("HDR dynamic power factor", 0.3, 0.45384666),
            ("HDR dynamic power factor", 0.65, 0.36715248),
        ] {
            assert!((tod.scalar(name, t) - expected).abs() < 4e-6, "{name} at {t}");
        }
        assert!(tod.variables["Sun color"].keys.iter().any(|key| key.flags == 458752));
    }

    #[test]
    fn legacy_xml_migrates_only_native_six_key_sets() {
        let names = ["Sun color", "Sun color multiplier", "Sun specular multiplier",
            "Sky color", "Sky color multiplier", "Global Rim Amount"];
        let mut variables = String::new();
        for name in names {
            let color = name == "Sun color" || name == "Sky color";
            let value = if color { "(0.5:0.5:0.5)" } else { "0.5" };
            variables.push_str(&format!(r#"<Variable Name="{name}" Value="0.8" Color="0.8,0.8,0.8"><Spline Keys="0:{value}:65536,1:{value}:0,"/></Variable>"#));
        }
        variables.push_str(r#"<Variable Name="Fog color"><Spline Keys="0:(0.5:0.5:0.5):0,1:(0.5:0.5:0.5):0,"/></Variable>
            <Variable Name="HDR dynamic power factor"><Spline Keys="0:2:0,1:2:0,"/></Variable>
            <Variable Name="Film curve whitepoint"><Spline Keys="0:3:0,1:3:0,"/></Variable>"#);
        let legacy = TimeOfDay::parse(&format!("<TimeOfDay>{variables}</TimeOfDay>")).unwrap();
        let modern = TimeOfDay::parse(&format!("<TimeOfDay sRGB=\"1\">{variables}</TimeOfDay>")).unwrap();
        for name in names {
            assert_eq!(legacy.variables[name].keys[0].value[0].to_bits(), 0.21763763_f32.to_bits());
            assert_eq!(modern.variables[name].keys[0].value[0], 0.5);
            assert_eq!(legacy.variables[name].keys[0].flags, 65536);
            assert_eq!(legacy.variables[name].value[0], 0.8, "editor metadata is not migrated");
        }
        assert_eq!(legacy.color("Fog color", 0.0), Vec3::splat(0.5));
        assert_eq!(legacy.scalar("HDR dynamic power factor", 0.0), 0.0);
        assert_eq!(legacy.scalar("Film curve whitepoint", 0.0), 1.0);
        assert_eq!(modern.scalar("HDR dynamic power factor", 0.0), 2.0);
        assert_eq!(modern.scalar("Film curve whitepoint", 0.0), 3.0);
    }

    #[test]
    fn real_login_area_legacy_keys_match_native_curve_evaluation() {
        if !crate::pak_available() { return }
        let xml = aa_pak::Pak::global().read_string("worlds/login2/cells/001_000/client/entities.xml").unwrap();
        let doc = roxmltree::Document::parse(&xml).unwrap();
        let entity = doc.descendants().find(|n| n.has_tag_name("Entity") && n.attribute("Name") == Some("login_224_oktop_2")).unwrap();
        let block = entity.descendants().find(|n| n.has_tag_name("TimeOfDay")).unwrap();
        assert_eq!(block.attribute("sRGB"), None);
        let tod = TimeOfDay::parse(&xml[block.range()]).unwrap();
        // Original x86 spline routines run on the source-proved migrated f32 keys.
        for (name, t, expected) in [
            ("Sun color", 0.25, [0.74623096, 0.7083347, 0.6329001]),
            ("Sun color", 0.5, [0.7817514, 0.7817514, 0.76674455]),
            ("Sun color", 0.7, [0.7575028, 0.73163223, 0.67537344]),
            ("Sky color", 0.25, [0.35865396, 0.5859734, 0.87013614]),
            ("Sky color", 0.5, [0.35865393, 0.5859734, 0.86713576]),
            ("Sky color", 0.7, [0.35865393, 0.5859734, 0.8691841]),
        ] {
            assert!((tod.color(name, t) - Vec3::from(expected)).abs().max_element() < 3e-6, "{name} at {t}");
        }
        for (name, expected) in [("Sun color multiplier", 6.8622217), ("Sun specular multiplier", 1.0),
            ("Sky color multiplier", 1.2332863), ("Global Rim Amount", 0.0)]
        {
            assert!((tod.scalar(name, 0.5) - expected).abs() < 3e-6, "{name}");
        }
        assert_eq!(tod.scalar("HDR dynamic power factor", 0.5), 0.0);
        assert!((tod.scalar("Film curve whitepoint", 0.5) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn parse_variables() {
        let xml = r#"<TimeOfDayList><TimeOfDay Time="12" sRGB="1">
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
    fn character_back_lights_are_colour_times_multiplier() {
        // zone 186: light 1 is grey 0.486 (= 0.2 linear) x 5, light 2 white x 0.7 with a specular multiplier of 0.7
        let xml = r#"<TimeOfDayList><TimeOfDay>
            <Variable Name="1 Color" Color="0.486275,0.486275,0.486275"><Spline Keys="0:(0.486275:0.486275:0.486275):0,1:(0.486275:0.486275:0.486275):0,"/></Variable>
            <Variable Name="1 Diffuse Multiplier" Value="5"><Spline Keys="0:5:0,1:5:0,"/></Variable>
            <Variable Name="1 Specular Multiplier" Value="1"><Spline Keys="0:1:0,1:1:0,"/></Variable>
            <Variable Name="2 Color" Color="1,1,1"><Spline Keys="0:(1:1:1):0,1:(1:1:1):0,"/></Variable>
            <Variable Name="2 Diffuse Multiplier" Value="0.7"><Spline Keys="0:0.7:0,1:0.7:0,"/></Variable>
            <Variable Name="2 Specular Multiplier" Value="0.7"><Spline Keys="0:0.7:0,1:0.7:0,"/></Variable>
            </TimeOfDay></TimeOfDayList>"#;
        let s = TimeOfDay::parse(xml).unwrap().sample(9.6, Lighting::default());
        assert!((s.char_back[0] - Vec4::new(1.0, 1.0, 1.0, 1.0)).abs().max_element() < 0.01, "{:?}", s.char_back[0]);
        assert!((s.char_back[1] - Vec4::new(0.7, 0.7, 0.7, 0.7)).abs().max_element() < 1e-5);
        let blended = s.lerp(&TimeOfDay::default().sample(9.6, Lighting::default()), 0.5);
        assert!((blended.char_back[1].x - 0.35).abs() < 1e-5);
        // a file without the group has no back light
        assert_eq!(TimeOfDay::default().sample(9.6, Lighting::default()).char_back, [Vec4::ZERO; 2]);
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
    fn native_zone_blending_precedes_color_conversion_and_hdr_products() {
        let tod = |color: f32, multiplier: f32, hdr: f32| {
            TimeOfDay::parse(&format!(
                r#"<TimeOfDay sRGB="1">
            <Variable Name="Sun color" Color="{color},{color},{color}"><Spline Keys="0:({color}:{color}:{color}):0,1:({color}:{color}:{color}):0,"/></Variable>
            <Variable Name="Sun color multiplier" Value="{multiplier}"><Spline Keys="0:{multiplier}:0,1:{multiplier}:0,"/></Variable>
            <Variable Name="Night sky: Moon color" Color="{color},{color},{color}"><Spline Keys="0:({color}:{color}:{color}):0,1:({color}:{color}:{color}):0,"/></Variable>
            <Variable Name="Night sky: Moon color multiplier" Value="{multiplier}"><Spline Keys="0:{multiplier}:0,1:{multiplier}:0,"/></Variable>
            <Variable Name="HDR dynamic power factor" Value="{hdr}"><Spline Keys="0:{hdr}:0,1:{hdr}:0,"/></Variable>
            </TimeOfDay>"#
            ))
            .unwrap()
        };
        let a = tod(0.2, 2.0, 1.0);
        let b = tod(0.8, 6.0, 3.0);
        let lighting = Lighting::default();
        let mixed = TimeOfDay::sample_blended(&[(&a, 1.0), (&b, 3.0)], 3.8, lighting);
        let raw = Vec3::splat(0.65);
        assert!((mixed.sun - srgb_to_linear(raw) * 5.0).length() < 1e-5);
        assert!((mixed.moons[0].color - raw * 5.0 * 2.5_f32.exp2()).length() < 1e-5);
        assert_eq!(mixed.hdr_dynamic, 2.5);
        assert_eq!(mixed.sun_dir, sun_direction(3.8, lighting));
        let after_conversion = a.sample(3.8, lighting).lerp(&b.sample(3.8, lighting), 0.75);
        assert!((after_conversion.sun - mixed.sun).length() > 0.5);
        assert!((after_conversion.moons[0].color - mixed.moons[0].color).length() > 10.0);
        let single = TimeOfDay::sample_blended(&[(&a, 0.2)], 3.8, lighting);
        assert_eq!(single.sun, a.sample(3.8, lighting).sun);
        assert_eq!(single.moons[0].color, a.sample(3.8, lighting).moons[0].color);
    }

    #[test]
    fn native_sun_sky_rgb_upload_goldens_and_raw_layer_order() {
        let tod = |sun: &str, sky: &str, sm: f32, am: f32, hdr: f32| TimeOfDay::parse(&format!(
            r#"<TimeOfDay sRGB="1">
            <Variable Name="Sun color"><Spline Keys="0:({sun}):0,1:({sun}):0,"/></Variable>
            <Variable Name="Sky color"><Spline Keys="0:({sky}):0,1:({sky}):0,"/></Variable>
            <Variable Name="Sun color multiplier"><Spline Keys="0:{sm}:0,1:{sm}:0,"/></Variable>
            <Variable Name="Sky color multiplier"><Spline Keys="0:{am}:0,1:{am}:0,"/></Variable>
            <Variable Name="HDR dynamic power factor"><Spline Keys="0:{hdr}:0,1:{hdr}:0,"/></Variable>
            </TimeOfDay>"#)).unwrap();
        let lighting = Lighting::default();
        // Original ToD SSE -> native setters/getters -> renderer PF/PB upload,
        // executed against the two original DLLs. No IEC decode occurs there.
        for (sm, hdr, expected) in [
            (0.4, -1.0, [0.34, 0.2875, 0.23500001]),
            (0.4, 0.0, [0.68, 0.575, 0.47000003]),
            (0.4, 0.6, [1.0306873, 0.87153703, 0.7123868]),
            (2.4, -1.0, [1.34, 1.2875, 1.235]),
            (2.4, 0.0, [2.68, 2.575, 2.47]),
            (2.4, 0.6, [4.0621204, 3.9029703, 3.7438202]),
        ] {
            let sample = tod("0.2:0.5:0.8", "0.8:0.5:0.2", sm, sm + 0.35, hdr).sample(10.0, lighting);
            assert!(sample.native_sun_sky.abs_diff_eq(Vec3::from_array(expected), 1e-6));
            assert!(sample.sun.abs_diff_eq(srgb_to_linear(Vec3::new(0.2, 0.5, 0.8)) * sm, 1e-6));
        }
        let a = tod("0.2:0.4:0.6", "0.1:0.3:0.5", 2.0, 1.0, 1.0);
        let b = tod("0.8:0.6:0.4", "0.7:0.5:0.3", 6.0, 3.0, 3.0);
        let mixed = TimeOfDay::sample_blended(&[(&a, 1.0), (&b, 3.0)], 10.0, lighting);
        let expected = Vec3::new(0.65, 0.55, 0.45) * (5.0 * 2.5_f32.exp2())
            + Vec3::new(0.55, 0.45, 0.35) * (2.5 * 2.5_f32.exp2());
        assert!(mixed.native_sun_sky.abs_diff_eq(expected, 5e-6));
        let sa = a.sample(10.0, lighting);
        let sb = b.sample(10.0, lighting);
        let late = sa.lerp(&sb, 0.75);
        assert_eq!(late.native_sun_sky, sa.native_sun_sky.mix(sb.native_sun_sky, 0.75));
        assert!((late.native_sun_sky - mixed.native_sun_sky).length() > 5.0);
        let default = TimeOfDay::default();
        let d = default.sample(10.0, lighting);
        let t = spline_hour(10.0, lighting.dawn_hour) / 24.0;
        let expected = default.color("Sun color", t) * default.scalar("Sun color multiplier", t)
            + default.color("Sky color", t) * default.scalar("Sky color multiplier", t);
        assert!(d.native_sun_sky.abs_diff_eq(expected, 1e-6));
    }

    #[test]
    fn native_global_fog_raw_rgb_multiplier_hdr_and_raw_zone_blend() {
        let tod = |color: &str, multiplier: f32, hdr: f32| TimeOfDay::parse(&format!(
            r#"<TimeOfDay sRGB="1">
            <Variable Name="Fog color"><Spline Keys="0:({color}):0,1:({color}):0,"/></Variable>
            <Variable Name="Fog color multiplier"><Spline Keys="0:{multiplier}:0,1:{multiplier}:0,"/></Variable>
            <Variable Name="HDR dynamic power factor"><Spline Keys="0:{hdr}:0,1:{hdr}:0,"/></Variable>
            </TimeOfDay>"#)).unwrap();
        let a = tod("0.1:0.3:0.5", 2.0, 1.0);
        let b = tod("0.5:0.7:0.9", 6.0, 3.0);
        let lighting = Lighting::default();
        let single = a.sample(10.0, lighting);
        assert!((single.native_fog - Vec3::new(0.4, 1.2, 2.0)).length() < 1e-6);
        // The existing calibrated fog is deliberately a separate adapter.
        assert!((single.fog - srgb_to_linear(Vec3::new(0.1, 0.3, 0.5)) * 2.0).length() < 1e-6);
        let mixed = TimeOfDay::sample_blended(&[(&a, 1.0), (&b, 3.0)], 10.0, lighting);
        let raw = Vec3::new(0.4, 0.6, 0.8);
        assert!((mixed.native_fog - raw * (5.0 * 2.5_f32.exp2())).length() < 5e-6);
        let late = a.sample(10.0, lighting).lerp(&b.sample(10.0, lighting), 0.75);
        assert!((mixed.native_fog - late.native_fog).length() > 10.0);
        // Old XML ignores its serialized HDR power; Fog colour is not one of
        // the six legacy key sets migrated with exponent 2.2.
        let legacy = tod("0.1:0.3:0.5", 2.0, 1.0);
        let xml = r#"<TimeOfDay><Variable Name="Fog color"><Spline Keys="0:(0.1:0.3:0.5):0,1:(0.1:0.3:0.5):0,"/></Variable>
            <Variable Name="Fog color multiplier"><Spline Keys="0:2:0,1:2:0,"/></Variable>
            <Variable Name="HDR dynamic power factor"><Spline Keys="0:3:0,1:3:0,"/></Variable></TimeOfDay>"#;
        let old = TimeOfDay::parse(xml).unwrap().sample(10.0, lighting);
        assert!((old.native_fog - legacy.sample(10.0, lighting).native_fog * 0.5).length() < 1e-6);
        let defaults = TimeOfDay::default();
        let sample = defaults.sample(10.0, lighting);
        let t = spline_hour(10.0, lighting.dawn_hour) / 24.0;
        assert_eq!(sample.native_fog, defaults.color("Fog color", t)
            * (defaults.scalar("Fog color multiplier", t) * defaults.scalar("HDR dynamic power factor", t).exp2()));
    }

    #[test]
    fn real_global_fog_zones_match_native_curve_numeric_controls() {
        if !crate::pak_available() { return }
        // Raw curves executed by original 300AC0A0/300AF640 and
        // 300ABDD0/300AF720 in bounded Unicorn, then native f32 product order.
        for (zone, expected) in [
            (149, [[0.0018018803, 0.004504689, 0.009910349], [0.20698118, 0.6002463, 0.9893724]]),
            (179, [[0.0088735, 0.026620477, 0.057677742], [0.3472341, 0.49274158, 0.7539938]]),
        ] {
            let xml = aa_pak::Pak::global().read_string(&format!("worlds/main_world/zone/{zone}/time_of_day.xml")).unwrap();
            let tod = TimeOfDay::parse(&xml).unwrap();
            for (hour, rgb) in [0., 12.].into_iter().zip(expected) {
                // Isolate native spline t=0/.5 from the separate main-world hour warp.
                let native = tod.sample(hour, Lighting { dawn_hour: 6.0, ..Lighting::default() }).native_fog;
                assert!((native - Vec3::from(rgb)).abs().max_element() < 3e-7, "zone{zone} hour{hour}: {native:?}");
            }
        }
    }

    #[test]
    fn moon_raw_rgb_hdr_factor_and_zone_blending() {
        let xml = r#"<TimeOfDayList><TimeOfDay sRGB="1">
            <Variable Name="Night sky: Moon color" Color="0.5,0.4,0.3"><Spline Keys="0:(0.5:0.4:0.3):0,1:(0.5:0.4:0.3):0,"/></Variable>
            <Variable Name="Night sky: Moon color multiplier" Value="3"><Spline Keys="0:3:0,1:3:0,"/></Variable>
            <Variable Name="HDR dynamic power factor" Value="1"><Spline Keys="0:1:0,1:1:0,"/></Variable>
            </TimeOfDay></TimeOfDayList>"#;
        let a = TimeOfDay::parse(xml).unwrap().sample(0.0, Lighting::default());
        assert!((a.moons[0].color - Vec3::new(3.0, 2.4, 1.8)).length() < 1e-6);
        let mut b = a.clone();
        b.moons[0].color = Vec3::ZERO;
        b.moons[0].outer_size = 2.0;
        b.moon_dir = Vec3::X;
        let middle = a.lerp(&b, 0.5);
        assert!((middle.moons[0].color - a.moons[0].color * 0.5).length() < 1e-6);
        assert!((middle.moons[0].outer_size - (a.moons[0].outer_size + 2.0) * 0.5).abs() < 1e-6);
        assert!((middle.moon_dir.length() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn sun_path() {
        let l = Lighting::default();
        let degrees = |v: Vec3| ((v.x.atan2(v.y).to_degrees() + 360.0) % 360.0, v.z.asin().to_degrees());
        // the real client at the Royal Palace: the sun patch of the court yard is where 123 / 42 (9.56 h) and 132 / 44 (10.0 h) put it
        let (az, el) = degrees(sun_direction(9.563, l));
        assert!((az - 124.0).abs() < 0.3 && (el - 42.4).abs() < 0.3, "{az} {el}");
        let (az, el) = degrees(sun_direction(9.998, l));
        assert!((az - 131.8).abs() < 0.3 && (el - 44.4).abs() < 0.3, "{az} {el}");
        // highest at noon, south-south-west and 49 degrees up
        let (az, el) = degrees(sun_direction(12.0, l));
        assert!((az - 172.0).abs() < 0.3 && (el - 48.9).abs() < 0.3, "{az} {el}");
        // up from 2:00 (DawnTime 119) to 22:00 (DuskTime 599), east in the morning, west in the evening
        assert!(sun_direction(1.9, l).z < 0.01 && sun_direction(2.1, l).z > -0.01);
        assert!(sun_direction(21.9, l).z > -0.01 && sun_direction(22.1, l).z < 0.01);
        assert!(sun_direction(0.0, l).z < 0.0 && sun_direction(23.0, l).z < 0.0);
        assert!(sun_direction(6.0, l).x > 0.5 && sun_direction(18.0, l).x < -0.5);
        // the login stage's file (260 / 40 / 0): a great circle, the sun at 40 degrees in the west at noon
        let login = Lighting { sun_rotation: 260.0, latitude: 40.0, planet_axis: 0.0, dawn_hour: 6.0 };
        let (az, el) = degrees(sun_direction(12.0, login));
        assert!((az - 280.0).abs() < 0.3 && (el - 40.0).abs() < 0.3, "{az} {el}");
        assert!(sun_direction(6.0, login).z.abs() < 1e-3 && sun_direction(18.0, login).z.abs() < 1e-3);
    }

    #[test]
    fn spline_hour_is_the_engines_warp() {
        // main world, DawnTime 119: the test vectors of docs/research/time-of-day.md
        let dawn = 119.0 / 60.0;
        for (hour, spline) in [(0.0, 3.04), (2.0, 6.02), (2.7, 6.67), (3.8, 7.55), (9.56, 10.90), (12.0, 12.0), (16.68, 12.0), (19.0, 12.0), (19.17, 12.17), (20.0, 13.53), (22.0, 17.96), (23.0, 20.78)] {
            let got = spline_hour(hour, dawn);
            assert!((got - spline).abs() < 0.03, "{hour}: {got} != {spline}");
        }
        // the real dawn and dusk land on the authored 6:00 and 18:00, and the half days join at noon
        assert!((spline_hour(dawn, dawn) - 6.0).abs() < 0.05 && (spline_hour(24.0 - dawn, dawn) - 18.0).abs() < 0.05);
        assert!((spline_hour(11.999, dawn) - 12.0).abs() < 0.01);
        // login worlds and most instances (DawnTime 360) are the identity, at every hour
        for i in 0..240 {
            let h = i as f32 * 0.1;
            assert!((spline_hour(h, 6.0) - h).abs() < 1e-3, "{h}");
        }
        // an hour outside 0..24 wraps like the clock
        assert!((spline_hour(25.0, dawn) - spline_hour(1.0, dawn)).abs() < 1e-4);
    }

    #[test]
    fn splines_are_read_at_the_warped_hour_but_the_sun_at_the_raw_one() {
        // a ramp over the day: the scalar at spline hour `h` is `h`
        let xml = r#"<TimeOfDayList><TimeOfDay>
            <Variable Name="Volumetric fog: Global density" Value="1"><Spline Keys="0:0:0,1:24:0,"/></Variable>
            </TimeOfDay></TimeOfDayList>"#;
        let t = TimeOfDay::parse(xml).unwrap();
        let main = Lighting::default();
        let tt = |h: f32, l: Lighting| t.scalar("Volumetric fog: Global density", spline_hour(h, l.dawn_hour) / 24.0);
        assert!((tt(19.17, main) - 12.17).abs() < 0.03 && (tt(2.7, main) - 6.67).abs() < 0.03);
        let login = Lighting { dawn_hour: 6.0, ..main };
        assert!((tt(19.17, login) - 19.17).abs() < 1e-3);
        // the sun path does not see the warp
        assert_eq!(t.sample(19.17, main).sun_dir, sun_direction(19.17, main));
    }

    #[test]
    fn hdr_setup_is_the_worlds_bloom() {
        let main = HdrSetup::parse(r#"<Environment><HDRSetup BloomColor="204,244,255" BloomMul="1.8" FlaresMul="0"/></Environment>"#).unwrap();
        assert!((main.bloom_color - Vec3::new(0.8, 0.957, 1.0)).abs().max_element() < 1e-3 && (main.bloom_mul - 1.8).abs() < 1e-6);
        assert_eq!(HdrSetup::parse(r#"<Environment><HDRSetup BloomMul="50"/></Environment>"#).unwrap(), HdrSetup { bloom_color: Vec3::ONE, bloom_mul: 10.0 });
        assert!(HdrSetup::parse("<Environment/>").is_none());
        if crate::pak_available() {
            let xml = aa_pak::Pak::global().read_string("worlds/main_world/env.xml").unwrap();
            assert!((HdrSetup::parse(&xml).unwrap().bloom_mul - 1.8).abs() < 1e-6);
        }
    }

    #[test]
    fn lighting_reads_the_dawn_time() {
        let main = Lighting::parse(r#"<sun><Lighting SunRotation="188" Latitude="154" PlanetRotationAxis="25" DawnTime="119" DawnDuration="15"/></sun>"#).unwrap();
        assert!((main.dawn_hour - 119.0 / 60.0).abs() < 1e-6);
        let login = Lighting::parse(r#"<sun><Lighting SunRotation="260" Latitude="40" DawnTime="360"/></sun>"#).unwrap();
        assert!((login.dawn_hour - 6.0).abs() < 1e-6);
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
