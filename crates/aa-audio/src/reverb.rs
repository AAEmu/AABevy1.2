//! Reverb: the area presets of the game and the effect that plays them.
//!
//! **Where the original gets it.** The level places `ReverbVolume` entities (`reverbpresetReverbPreset="x2_cave_1"`, an
//! `OuterRadius`, `bFullEffectWhenInside`; 587 of them in the world cells) and the entity script
//! (`entities/sound/reverbvolume.lua`) registers the preset with the sound system while the player is near it
//! (`Sound.RegisterWeightedEaxEnvironment(preset, id, ...)`, `UpdateWeightedEaxEnvironment(preset, id, weight)`), the weight
//! being 1 inside and falling to 0 at `OuterRadius`. The presets are the 45 rows of `libs/reverbpresets/reverbpresets.xml`
//! (`ReverbPresetDB`), each an `FMOD_REVERB_PROPERTIES` (EAX / I3DL2 style: levels in millibels, a decay time with a high
//! frequency ratio, early reflection and late reverb level and delay). FMOD has one global reverb that every event
//! sends to by its own **reverb wet level** (`Event::reverb_wet`, a dB value: -8 for most events, -60 for the UI, the
//! login music and the tutorial voices, which means none).
//!
//! **What is done here.** [`Presets`] reads the file out of the pak; [`Params::of`] turns a preset and a weight into the
//! values the effect uses; [`ReverbCore`] is the effect (an input diffuser, six early reflection taps and an eight line
//! feedback delay network with a decay time per line, a high frequency damping that reaches the preset's decay
//! ratio at its reference frequency, and the wet high cut). [`ReverbBuilder`] wraps the core as a mixer effect for a send
//! track; the engine routes each sound to it by the event's wet level and sets the preset and its weight as the player
//! moves between volumes. Levels follow the usual convention: `Room + Reverb` millibels is the total energy of the late
//! reverb relative to the sound it is fed with (0 dB = as much energy in the tail as in the sound).
//!
//! Not modelled: echo and modulation (`fEchoDepth`, `fModulationDepth`: three presets), the low frequency decay ratio and
//! room low cut, `RoomRolloffFactor`, air absorption, a decay high ratio above 1 (treated as 1).

use std::collections::HashMap;
use std::f32::consts::PI;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use kira::Frame;
use kira::effect::{Effect, EffectBuilder};
use kira::info::Info;

/// Path of the preset file in the pak.
pub const PRESET_FILE: &str = "game/libs/reverbpresets/reverbpresets.xml";

/// One `FMOD_REVERB_PROPERTIES` row of `reverbpresets.xml`.
#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    pub name: String,
    /// Master level of the room, millibels (hundredths of a dB), -10000 = off.
    pub room: i32,
    /// High frequency attenuation of the room at `hf_reference`, millibels.
    pub room_hf: i32,
    /// Seconds the late reverb takes to fall by 60 dB.
    pub decay_time: f32,
    /// Decay time at `hf_reference` relative to `decay_time` (above 1: the highs last longer).
    pub decay_hf_ratio: f32,
    /// Early reflections level relative to `room`, millibels.
    pub reflections: i32,
    /// Seconds from the sound to the first reflection.
    pub reflections_delay: f32,
    /// Late reverb level relative to `room`, millibels.
    pub reverb: i32,
    /// Seconds from the first reflection to the start of the late reverb.
    pub reverb_delay: f32,
    pub hf_reference: f32,
    /// 0 to 100: how smooth the tail is.
    pub diffusion: f32,
    pub density: f32,
}

fn value(node: roxmltree::Node, tag: &str, default: f32) -> f32 {
    node.children().find(|c| c.has_tag_name(tag)).and_then(|c| c.attribute("value")).and_then(|v| v.trim().parse().ok()).unwrap_or(default)
}

/// The presets of a `reverbpresets.xml`, in file order.
pub fn parse_presets(xml: &str) -> Vec<Preset> {
    let Ok(doc) = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')) else { return Vec::new() };
    let mut out = Vec::new();
    for n in doc.root_element().children().filter(|c| c.is_element()) {
        let v = |tag: &str, d: f32| value(n, tag, d);
        out.push(Preset {
            name: n.tag_name().name().to_string(),
            room: v("nRoom", -10000.0) as i32,
            room_hf: v("nRoomHF", 0.0) as i32,
            decay_time: v("fDecayTime", 1.0),
            decay_hf_ratio: v("fDecayHFRatio", 1.0),
            reflections: v("nReflections", -10000.0) as i32,
            reflections_delay: v("fReflectionsDelay", 0.02),
            reverb: v("nReverb", -10000.0) as i32,
            reverb_delay: v("fReverbDelay", 0.04),
            hf_reference: v("fHFReference", 5000.0),
            diffusion: v("fDiffusion", 100.0),
            density: v("fDensity", 100.0),
        });
    }
    out
}

/// All the presets of the game by name (case-insensitive).
#[derive(Clone, Debug, Default)]
pub struct Presets {
    by_name: HashMap<String, Preset>,
}

impl Presets {
    pub fn parse(xml: &str) -> Presets {
        Presets { by_name: parse_presets(xml).into_iter().map(|p| (p.name.to_ascii_lowercase(), p)).collect() }
    }

    /// Reads `libs/reverbpresets/reverbpresets.xml` from the pak (empty when it is not there).
    pub fn load(pak: &aa_pak::Pak) -> Presets {
        pak.read(PRESET_FILE).map(|b| Presets::parse(&String::from_utf8_lossy(&b))).unwrap_or_default()
    }

    pub fn get(&self, name: &str) -> Option<&Preset> {
        self.by_name.get(&name.to_ascii_lowercase())
    }

    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }
}

fn mb_to_amp(mb: f32) -> f32 {
    if mb <= -10000.0 { 0.0 } else { 10f32.powf(mb / 2000.0) }
}

/// What the effect is told: linear levels and times, no more millibels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    /// Amplitude of the early reflections (`Room + Reflections`) and of the late reverb (`Room + Reverb`) times the weight.
    pub early: f32,
    pub late: f32,
    pub decay: f32,
    pub hf_ratio: f32,
    pub hf_ref: f32,
    /// Gain of the wet signal at `hf_ref` relative to its lows (`RoomHF`), 0 to 1.
    pub room_hf: f32,
    pub early_delay: f32,
    pub late_delay: f32,
    /// 0 to 1.
    pub diffusion: f32,
}

impl Params {
    /// No reverb.
    pub const OFF: Params = Params { early: 0.0, late: 0.0, decay: 0.3, hf_ratio: 1.0, hf_ref: 5000.0, room_hf: 1.0, early_delay: 0.01, late_delay: 0.02, diffusion: 1.0 };

    /// A preset at `weight` (0 to 1): the levels scale with the weight, the character stays.
    pub fn of(p: &Preset, weight: f32) -> Params {
        let w = weight.clamp(0.0, 1.0);
        Params {
            early: mb_to_amp((p.room + p.reflections) as f32) * w,
            late: mb_to_amp((p.room + p.reverb) as f32) * w,
            decay: p.decay_time.clamp(0.1, 20.0),
            hf_ratio: p.decay_hf_ratio.clamp(0.1, 1.0),
            hf_ref: p.hf_reference.clamp(1000.0, 20000.0),
            room_hf: mb_to_amp(p.room_hf as f32).clamp(0.0, 1.0),
            early_delay: p.reflections_delay.clamp(0.0, 0.3),
            late_delay: (p.reflections_delay + p.reverb_delay).clamp(0.0, 0.4),
            diffusion: (p.diffusion / 100.0).clamp(0.0, 1.0),
        }
    }

    /// Whether this makes any sound.
    pub fn audible(&self) -> bool {
        self.early > 1e-4 || self.late > 1e-4
    }

    fn to_array(self) -> [f32; 9] {
        [self.early, self.late, self.decay, self.hf_ratio, self.hf_ref, self.room_hf, self.early_delay, self.late_delay, self.diffusion]
    }

    fn from_array(a: [f32; 9]) -> Params {
        Params { early: a[0], late: a[1], decay: a[2], hf_ratio: a[3], hf_ref: a[4], room_hf: a[5], early_delay: a[6], late_delay: a[7], diffusion: a[8] }
    }
}

// ---- the core ------------------------------------------------------------------------------------------------------

const LINES: usize = 8;
/// Delay line lengths in samples at 44.1 kHz: primes between 30 and 65 ms.
const LINE_LEN: [usize; LINES] = [1327, 1523, 1709, 1913, 2131, 2347, 2617, 2879];
/// Early reflection tap times (s after the first) and gains.
const TAPS: [(f32, f32); 6] = [(0.0, 0.9), (0.0113, 0.75), (0.0197, 0.65), (0.0311, 0.5), (0.0437, 0.4), (0.0571, 0.3)];
/// Input diffusers (allpass) in samples at 44.1 kHz.
const DIFFUSERS: [usize; 2] = [211, 307];

/// Pole of a one-pole low-pass `y = (1 - p) x + p y'` whose gain at `freq` is `ratio` (0 < ratio <= 1).
fn damping_pole(ratio: f32, freq: f32, rate: f32) -> f32 {
    if ratio >= 0.999 {
        return 0.0;
    }
    let r2 = ratio.max(0.001).powi(2);
    let c = (2.0 * PI * freq.min(rate * 0.45) / rate).cos();
    let a = 1.0 - r2 * c;
    let disc = (a * a - (1.0 - r2) * (1.0 - r2)).max(0.0);
    ((a - disc.sqrt()) / (1.0 - r2)).clamp(0.0, 0.999)
}

struct Delay {
    buf: Vec<f32>,
    pos: usize,
}

impl Delay {
    fn new(len: usize) -> Delay {
        Delay { buf: vec![0.0; len.max(1)], pos: 0 }
    }
    fn read(&self) -> f32 {
        self.buf[self.pos]
    }
    fn write_advance(&mut self, v: f32) {
        self.buf[self.pos] = v;
        self.pos += 1;
        if self.pos == self.buf.len() {
            self.pos = 0;
        }
    }
    fn clear(&mut self) {
        self.buf.fill(0.0);
    }
}

/// The reverb of one send, mono in and stereo out, fully wet. Pure DSP: usable offline (the `sound` example renders with it).
pub struct ReverbCore {
    rate: f32,
    cur: Params,
    target: Params,
    /// Input history for the early taps and the late reverb's pre-delay, one slot per sample.
    hist: Vec<f32>,
    hist_pos: usize,
    diffusers: [Delay; 2],
    lines: Vec<Delay>,
    /// Low-pass state of each line.
    damp: [f32; LINES],
    damp_pole: [f32; LINES],
    line_gain: [f32; LINES],
    norm: f32,
    out_lp: [f32; 2],
    out_pole: f32,
    ap_gain: f32,
    early_taps: [(usize, f32); 6],
    late_delay: usize,
    /// Samples of tail left after the last input before the core may rest.
    ring: usize,
    cleared: bool,
}

impl ReverbCore {
    pub fn new(rate: u32) -> ReverbCore {
        let rate = rate.max(8000) as f32;
        let scale = rate / 44100.0;
        let mut core = ReverbCore {
            rate,
            cur: Params::OFF,
            target: Params::OFF,
            hist: vec![0.0; (rate * 0.6) as usize + 1],
            hist_pos: 0,
            diffusers: DIFFUSERS.map(|n| Delay::new((n as f32 * scale) as usize)),
            lines: LINE_LEN.iter().map(|&n| Delay::new((n as f32 * scale) as usize)).collect(),
            damp: [0.0; LINES],
            damp_pole: [0.0; LINES],
            line_gain: [0.0; LINES],
            norm: 1.0,
            out_lp: [0.0; 2],
            out_pole: 0.0,
            ap_gain: 0.5,
            early_taps: [(0, 0.0); 6],
            late_delay: 0,
            ring: 0,
            cleared: true,
        };
        core.update_coefficients();
        core
    }

    pub fn sample_rate(&self) -> u32 {
        self.rate as u32
    }

    /// The values the core is moving towards.
    pub fn set_target(&mut self, p: Params) {
        self.target = p;
    }

    /// Jumps to the target (offline rendering, tests).
    pub fn snap(&mut self) {
        self.cur = self.target;
        self.update_coefficients();
    }

    /// Whether the core makes no sound and can be skipped until it is fed again.
    pub fn resting(&self) -> bool {
        !self.target.audible() && !self.cur.audible()
    }

    fn update_coefficients(&mut self) {
        let p = self.cur;
        let sr = self.rate;
        let mut mean = 0.0;
        for (i, line) in self.lines.iter().enumerate() {
            let d = line.buf.len() as f32 / sr;
            mean += d;
            let g = 10f32.powf(-3.0 * d / p.decay.max(0.05));
            // the line's gain at the reference frequency is the one a decay of decay * hf_ratio gives
            let ratio = 10f32.powf(-3.0 * d / p.decay.max(0.05) * (1.0 / p.hf_ratio.max(0.05) - 1.0));
            self.line_gain[i] = g;
            self.damp_pole[i] = damping_pole(ratio, p.hf_ref, sr);
        }
        mean /= LINES as f32;
        // a lossless eight line network sends all of its energy through the lines once per mean delay; with the decay the
        // rounds add up to 1 / (1 - q): scale that to one
        self.norm = (1.0 - 10f32.powf(-6.0 * mean / p.decay.max(0.05))).sqrt();
        self.out_pole = damping_pole(p.room_hf, p.hf_ref, sr);
        self.ap_gain = 0.2 + 0.4 * p.diffusion;
        let early = (p.early_delay * sr) as usize;
        let n = self.hist.len();
        let norm: f32 = TAPS.iter().map(|t| t.1 * t.1).sum::<f32>().sqrt();
        for (i, (t, g)) in TAPS.iter().enumerate() {
            self.early_taps[i] = (((early + (t * sr) as usize) + 1).min(n - 1), g / norm);
        }
        self.late_delay = ((p.late_delay * sr) as usize + 1).min(n - 1);
    }

    /// Moves the current values towards the target; `seconds` is the time since the last call. Call once per block.
    pub fn glide(&mut self, seconds: f32) {
        let k = 1.0 - (-seconds / 0.35).exp();
        let (c, t) = (self.cur.to_array(), self.target.to_array());
        let mut n = [0.0; 9];
        for i in 0..9 {
            n[i] = c[i] + (t[i] - c[i]) * k;
            if (n[i] - t[i]).abs() < 1e-4 * t[i].abs().max(1.0) {
                n[i] = t[i];
            }
        }
        let next = Params::from_array(n);
        if next != self.cur {
            self.cur = next;
            self.update_coefficients();
        }
    }

    /// One frame: the sum of the sends in, the wet signal out.
    #[inline]
    pub fn tick(&mut self, input: Frame) -> Frame {
        let x = 0.5 * (input.left + input.right);
        self.hist[self.hist_pos] = x;
        let n = self.hist.len();
        let at = |d: usize| self.hist[(self.hist_pos + n - d) % n];
        // early reflections: taps alternate between the ears
        let (mut el, mut er) = (0.0f32, 0.0f32);
        for (i, &(d, g)) in self.early_taps.iter().enumerate() {
            let v = at(d) * g;
            if i % 2 == 0 {
                el += v;
            } else {
                er += v;
            }
        }
        el *= self.cur.early;
        er *= self.cur.early;

        // late reverb: pre-delayed input through two allpass diffusers into the network
        let mut f = at(self.late_delay);
        for d in self.diffusers.iter_mut() {
            let b = d.read();
            let v = f - self.ap_gain * b;
            d.write_advance(v);
            f = b + self.ap_gain * v;
        }
        let mut s = [0.0f32; LINES];
        let mut sum = 0.0;
        let (mut ll, mut lr) = (0.0f32, 0.0f32);
        for i in 0..LINES {
            let o = self.lines[i].read();
            // loss, then the damping low-pass
            let p = self.damp_pole[i];
            self.damp[i] = (1.0 - p) * o * self.line_gain[i] + p * self.damp[i];
            s[i] = self.damp[i];
            sum += s[i];
            let sign_l = if i % 2 == 0 { 1.0 } else { -1.0 };
            let sign_r = if (i / 2) % 2 == 0 { 1.0 } else { -1.0 };
            ll += sign_l * o;
            lr += sign_r * o;
        }
        let feed = f * (1.0 / (LINES as f32).sqrt());
        let mean = sum * (2.0 / LINES as f32);
        for i in 0..LINES {
            self.lines[i].write_advance(feed + s[i] - mean);
        }
        // two channels with unit coefficients each carry half of the network's energy
        let scale = self.norm * self.cur.late * std::f32::consts::FRAC_1_SQRT_2;
        let (ll, lr) = (ll * scale, lr * scale);

        self.hist_pos = (self.hist_pos + 1) % n;

        // the room's high cut on the wet sum
        let (l, r) = (el + ll, er + lr);
        let p = self.out_pole;
        self.out_lp[0] = (1.0 - p) * l + p * self.out_lp[0];
        self.out_lp[1] = (1.0 - p) * r + p * self.out_lp[1];
        Frame { left: self.out_lp[0], right: self.out_lp[1] }
    }

    /// Processes a block in place (the input is replaced by the wet signal). Does nothing while the reverb is off or has
    /// nothing to ring on.
    pub fn process(&mut self, block: &mut [Frame], seconds_per_frame: f64) {
        self.glide((block.len() as f64 * seconds_per_frame) as f32);
        if self.resting() {
            if !self.cleared {
                self.clear_state();
                self.cleared = true;
            }
            block.fill(Frame::ZERO);
            return;
        }
        self.cleared = false;
        let fed = block.iter().any(|f| f.left.abs() > 1e-6 || f.right.abs() > 1e-6);
        if fed {
            // the tail lasts the decay time (and the pre-delay) after the last input
            self.ring = (self.cur.decay.min(20.0) * self.rate) as usize + self.hist.len();
        } else if self.ring == 0 {
            block.fill(Frame::ZERO);
            return;
        } else {
            self.ring = self.ring.saturating_sub(block.len());
        }
        for f in block.iter_mut() {
            *f = self.tick(*f);
        }
    }

    fn clear_state(&mut self) {
        self.hist.fill(0.0);
        for d in self.diffusers.iter_mut() {
            d.clear();
        }
        for l in self.lines.iter_mut() {
            l.clear();
        }
        self.damp = [0.0; LINES];
        self.out_lp = [0.0; 2];
        self.ring = 0;
    }
}

// ---- the mixer effect ----------------------------------------------------------------------------------------------

struct Shared {
    params: [AtomicU32; 9],
}

/// Sets the reverb of a [`ReverbBuilder`] effect from any thread.
#[derive(Clone)]
pub struct ReverbHandle {
    shared: Arc<Shared>,
}

impl ReverbHandle {
    pub fn set(&self, p: Params) {
        for (slot, v) in self.shared.params.iter().zip(p.to_array()) {
            slot.store(v.to_bits(), Ordering::Relaxed);
        }
    }
}

/// Builds the reverb as a mixer effect for a send track.
pub struct ReverbBuilder;

impl EffectBuilder for ReverbBuilder {
    type Handle = ReverbHandle;

    fn build(self) -> (Box<dyn Effect>, ReverbHandle) {
        let shared = Arc::new(Shared { params: Params::OFF.to_array().map(|v| AtomicU32::new(v.to_bits())) });
        (Box::new(ReverbEffect { shared: shared.clone(), core: ReverbCore::new(44100) }), ReverbHandle { shared })
    }
}

struct ReverbEffect {
    shared: Arc<Shared>,
    core: ReverbCore,
}

impl Effect for ReverbEffect {
    fn init(&mut self, sample_rate: u32, _internal_buffer_size: usize) {
        self.core = ReverbCore::new(sample_rate);
    }

    fn on_change_sample_rate(&mut self, sample_rate: u32) {
        self.core = ReverbCore::new(sample_rate);
    }

    fn on_start_processing(&mut self) {
        let mut a = [0.0f32; 9];
        for (v, slot) in a.iter_mut().zip(&self.shared.params) {
            *v = f32::from_bits(slot.load(Ordering::Relaxed));
        }
        self.core.set_target(Params::from_array(a));
    }

    fn process(&mut self, input: &mut [Frame], dt: f64, _info: &Info) {
        self.core.process(input, dt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<ReverbPresetDB>
 <x2_cave_1>
  <nEnvironment type="int" min="0" max="25" value="1"/>
  <nRoom type="int" min="-10000" max="0" value="0"/>
  <nRoomHF type="int" min="-10000" max="0" value="-700"/>
  <fDecayTime type="float" min="0.1" max="20.0" value="7"/>
  <fDecayHFRatio type="float" min="0.1" max="2.0" value="0.33"/>
  <nReflections type="int" min="-10000" max="1000" value="-1100"/>
  <fReflectionsDelay type="float" min="0.0" max="0.3" value="0.02"/>
  <nReverb type="int" min="-10000" max="2000" value="16"/>
  <fReverbDelay type="float" min="0.0" max="0.1" value="0.03"/>
  <fHFReference type="float" min="1000.0" max="20000" value="5000"/>
  <fDiffusion type="float" min="0.0" max="100.0" value="100"/>
  <fDensity type="float" min="0.0" max="100.0" value="100"/>
 </x2_cave_1>
 <off><nRoom type="int" value="-10000"/><nRoomHF type="int" value="-10000"/></off>
</ReverbPresetDB>"#;

    fn params(decay: f32, hf_ratio: f32) -> Params {
        Params { early: 0.0, late: 1.0, decay, hf_ratio, hf_ref: 5000.0, room_hf: 1.0, early_delay: 0.01, late_delay: 0.02, diffusion: 1.0 }
    }

    /// Impulse response of the left channel for `seconds`.
    fn impulse(p: Params, seconds: f32) -> Vec<f32> {
        let mut core = ReverbCore::new(44100);
        core.set_target(p);
        core.snap();
        let n = (seconds * 44100.0) as usize;
        (0..n).map(|i| core.tick(if i == 0 { Frame::new(1.0, 1.0) } else { Frame::ZERO }).left).collect()
    }

    #[test]
    fn presets_are_read() {
        let p = Presets::parse(XML);
        assert_eq!(p.len(), 2);
        let cave = p.get("X2_Cave_1").unwrap();
        assert_eq!((cave.room, cave.room_hf, cave.reverb, cave.reflections), (0, -700, 16, -1100));
        assert_eq!((cave.decay_time, cave.decay_hf_ratio), (7.0, 0.33));
        assert!((cave.reverb_delay - 0.03).abs() < 1e-6);
        // -10000 mB is off at any weight
        let off = Params::of(p.get("off").unwrap(), 1.0);
        assert!(!off.audible());
        let half = Params::of(cave, 0.5);
        let full = Params::of(cave, 1.0);
        assert!((half.late * 2.0 - full.late).abs() < 1e-6);
        assert!((full.late - 10f32.powf(16.0 / 2000.0)).abs() < 1e-6);
        assert!((full.late_delay - 0.05).abs() < 1e-6);
    }

    #[test]
    fn decay_time_is_the_time_to_minus_60_db() {
        for t in [0.8f32, 2.0, 4.0] {
            let ir = impulse(params(t, 1.0), (t * 1.6).max(2.0));
            // Schroeder backward integration of the energy; the slope between -5 and -35 dB gives the decay time
            let mut energy = vec![0.0f64; ir.len()];
            let mut acc = 0.0;
            for i in (0..ir.len()).rev() {
                acc += f64::from(ir[i]).powi(2);
                energy[i] = acc;
            }
            let db: Vec<f64> = energy.iter().map(|e| 10.0 * (e / energy[0]).log10()).collect();
            let at = |level: f64| db.iter().position(|&d| d < level).unwrap() as f64 / 44100.0;
            let slope = (at(-35.0) - at(-5.0)) / 30.0 * 60.0;
            assert!((slope / f64::from(t) - 1.0).abs() < 0.25, "decay {t}: measured {slope:.2} s");
        }
    }

    #[test]
    fn the_tail_carries_the_level_of_the_sound() {
        // 0 dB late reverb: about as much energy in the tail as in the input pulse
        for t in [0.5f32, 2.0, 7.0] {
            let ir = impulse(params(t, 1.0), t * 2.0);
            let e: f64 = ir.iter().map(|&v| f64::from(v).powi(2)).sum();
            // one channel carries half of the energy of the pair
            let db = 10.0 * (2.0 * e).log10();
            assert!(db.abs() < 3.0, "decay {t}: {db:.1} dB");
        }
    }

    #[test]
    fn a_low_high_ratio_darkens_the_tail() {
        let bright = impulse(params(2.0, 1.0), 2.0);
        let dark = impulse(params(2.0, 0.2), 2.0);
        // energy of the first difference stands for the highs, taken from the middle of the tail
        let highs = |ir: &[f32]| ir[22050..66150].windows(2).map(|w| f64::from(w[1] - w[0]).powi(2)).sum::<f64>();
        let total = |ir: &[f32]| ir[22050..66150].iter().map(|&v| f64::from(v).powi(2)).sum::<f64>();
        assert!(highs(&dark) / total(&dark) < 0.5 * highs(&bright) / total(&bright));
    }

    #[test]
    fn rests_when_off_and_when_the_tail_has_died() {
        let mut core = ReverbCore::new(44100);
        let mut block = vec![Frame::new(0.5, 0.5); 512];
        core.process(&mut block, 1.0 / 44100.0);
        assert!(block.iter().all(|f| f.left == 0.0), "off makes no sound");
        assert!(core.resting());
        core.set_target(params(0.3, 1.0));
        for _ in 0..40 {
            block.fill(Frame::new(0.5, 0.5));
            core.process(&mut block, 1.0 / 44100.0);
        }
        assert!(block.iter().any(|f| f.left != 0.0), "on makes sound");
        // silence in: the tail rings, then the core stops computing
        let mut quiet = vec![Frame::ZERO; 4410];
        core.process(&mut quiet, 1.0 / 44100.0);
        assert!(quiet.iter().any(|f| f.left != 0.0), "the tail rings after the input stops");
        for _ in 0..60 {
            quiet.fill(Frame::ZERO);
            core.process(&mut quiet, 1.0 / 44100.0);
        }
        assert!(quiet.iter().all(|f| f.left == 0.0), "and ends");
        // off again: the core rests
        core.set_target(Params::OFF);
        for _ in 0..400 {
            quiet.fill(Frame::ZERO);
            core.process(&mut quiet, 1.0 / 44100.0);
        }
        assert!(core.resting());
    }
}
