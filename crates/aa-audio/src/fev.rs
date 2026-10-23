//! FEV: FMOD Designer event files (`.fev`), format version `0x450000` (FMOD 4.44), the only one the
//! game ships (all 69 files).
//!
//! ```text
//! RIFF "FEV "
//!   FMT   u32 version (0x450000)
//!   LIST "PROJ"
//!     OBCT  object counts (u32 count, then { u32 type, u32 count })
//!     PROP  project name
//!     LGCY  the project body, described below (the older "legacy" layout is the one in use)
//!     EPRP  the table of envelope points (see below)
//!     STRR  string table: u32 n, n x u32 offsets, NUL terminated strings; names in LGCY are indices
//!     LANG  languages
//! ```
//!
//! `LGCY` (little endian; `str` = u32 length including the NUL + bytes, `s` = u32 index into `STRR`):
//!
//! ```text
//! u32 x2                      memory pool sizes
//! str  project name
//! u32 bank count, u32 hashes per bank (1)
//! bank*      u32 mode (0x80 stream, 0x100 decompress to memory, 0x200 keep compressed), u32 max streams,
//!            { u8[8] hash, u32 0 } x hashes, str name        (file: <fev dir>/<name>.fsb)
//! category   str name, f32 volume, f32 pitch, u32 max playbacks, u32 behaviour, u32 n, category x n
//! u32 group count, group*
//! u32 template count, template* (74 bytes, see [`Template`])
//! u32 sound definition count, sound definition*
//! u32 reverb count, { str name, u32 x 33 } x count
//! music: { u32 size, u32 tag } chunks, the first is "comp" (size 8 when the project has no music)
//!
//! group      s name, u32 user property count, property*, u32 group count, u32 event count, group*, event*
//! property   str name, u32 type (0 int, 1 float, 2 string), value (4 bytes, or str)
//! event      u32 flags (0x10 simple, 0x08 complex), s name, u8[16] guid,
//!            f32 volume, f32 pitch, f32 pitch randomization, f32 volume randomization,
//!            u32 priority, u32 max playbacks, u32, u32 mode, f32 3D min, f32 3D max, u32, f32,
//!            u32 flags2, f32 x 8 speaker levels, f32 cone inside, outside, outside volume,
//!            u32 playback behaviour, f32 doppler, f32 reverb dry, f32 reverb wet, f32 speaker spread,
//!            u32 fade in ms, u32 fade out ms, f32 spawn intensity, f32 spawn intensity randomization,
//!            f32 3D pan level, u32, u32, body, u32 category count, { str name } x count
//! simple     u32, instance
//! complex    u32 layer count, layer*, u32 parameter count, parameter*, u32 property count, property*
//! layer      u16 flags, i16 priority, u16 control parameter, u16 instance count, u16 envelope count,
//!            instance*, envelope*
//! instance   u16 sound def, f32 start, f32 length, u32 autopitch, u32 loop word, u32 x 10
//! envelope   i32 parent, str dsp name, u32, u32 flags, u32 flags2, u32 point count, u32 point id x n,
//!            u32 parameter index, u32
//! parameter  s name, f32 velocity, f32 min, f32 max, u32 flags, f32 seek speed, u32, u32 count, f32 x count
//! template   u32 mode word (4 * kind + flags), u32 spawn min ms, u32 spawn max ms, u32 max spawned,
//!            f32 volume, u32, u32, u32, f32 volume randomization min, f32 pitch, u32, u32, u32,
//!            f32 pitch randomization, u32, u32, u32, u16 fade in ms, u16 fade out ms, u16
//! sound def  s name, u32 template, u32 wave count, wave*
//! wave       u32 type (0 bank sound, 1 oscillator, 2 silence, 3 programmer), u32 weight percent, then
//!            for a bank sound: str name, u32 bank, u32 subsound index, u32 length ms
//! ```
//!
//! The `EPRP` chunk is the table of envelope points, `u32 count, { f32 x, f32 y, u32 curve } x count`;
//! an envelope lists point ids into it. Both x and y are 0 to 1 (x along the range of the parameter).
//!
//! The layout was recovered from the game's own `fmod_eventl.dll` (the loader functions
//! `EventGroupI::load`, `EventSystemI::load` and the complex/simple event loaders) and checked
//! against the 69 files: every file parses to exactly its last byte.

use std::collections::HashMap;

use crate::fsb5::{Error, Result};

/// The only FEV version the game ships.
pub const VERSION: u32 = 0x45_0000;

fn bad(what: impl Into<String>) -> Error {
    Error::Other(format!("fev: {}", what.into()))
}

struct Rd<'a> {
    b: &'a [u8],
    o: usize,
}

impl<'a> Rd<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.o.checked_add(n).filter(|&e| e <= self.b.len()).ok_or(Error::Truncated("fev"))?;
        let s = &self.b[self.o..end];
        self.o = end;
        Ok(s)
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }
    fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }
    /// A count that must fit in what is left of the data (each element takes at least one byte).
    fn count(&mut self) -> Result<usize> {
        let n = self.u32()? as usize;
        if n > self.b.len() - self.o {
            return Err(bad(format!("count {n} at {:#x} exceeds the data", self.o - 4)));
        }
        Ok(n)
    }
    fn string(&mut self) -> Result<String> {
        let n = self.u32()? as usize;
        if n > 0x1000 {
            return Err(bad(format!("string of {n} bytes at {:#x}", self.o - 4)));
        }
        let s = self.take(n)?;
        let s = s.strip_suffix(&[0]).unwrap_or(s);
        Ok(s.iter().map(|&c| c as char).collect())
    }
}

/// One wave bank the project plays from.
#[derive(Clone, Debug)]
pub struct Bank {
    /// File stem: `<fev dir>/<name>.fsb`.
    pub name: String,
    /// 0x80 stream from disk, 0x100 decompress into memory, 0x200 keep compressed in memory.
    pub mode: u32,
    pub max_streams: u32,
}

/// A mixer category (a tree, root `master`).
#[derive(Clone, Debug)]
pub struct Category {
    pub name: String,
    pub volume: f32,
    pub pitch: f32,
    pub max_playbacks: u32,
    pub children: Vec<Category>,
}

impl Category {
    /// All categories below and including this one, as `master/music/...` paths.
    pub fn paths(&self) -> Vec<String> {
        fn walk(c: &Category, prefix: &str, out: &mut Vec<String>) {
            let p = if prefix.is_empty() { c.name.clone() } else { format!("{prefix}/{}", c.name) };
            out.push(p.clone());
            for k in &c.children {
                walk(k, &p, out);
            }
        }
        let mut out = Vec::new();
        walk(self, "", &mut out);
        out
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PropValue {
    Int(i32),
    Float(f32),
    Text(String),
}

#[derive(Clone, Debug)]
pub struct UserProp {
    pub name: String,
    pub value: PropValue,
}

/// How a sound definition picks among its waves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayMode {
    Sequential,
    Random,
    Shuffle,
    Programmer,
}

/// A sound definition template: play mode, spawning and the per definition volume and pitch.
/// 74 bytes in the file; identified from the values in all 69 files (see the docs).
#[derive(Clone, Debug)]
pub struct Template {
    /// `4 * kind + flags` as stored.
    pub mode_word: u32,
    /// While the event plays, a new sound is started every `min..=max` ms (0 when unused).
    pub spawn_time_min_ms: u32,
    pub spawn_time_max_ms: u32,
    /// Most sounds of this definition playing at once from spawning.
    pub max_spawned: u32,
    /// Linear amplitude.
    pub volume: f32,
    /// Lowest random volume factor (1.0 for none).
    pub volume_random_min: f32,
    /// As stored: octaves = `pitch * 4`.
    pub pitch: f32,
    /// As stored: octaves = `pitch_random * 4`, +- around the pitch.
    pub pitch_random: f32,
    pub fade_in_ms: u16,
    pub fade_out_ms: u16,
    /// Words not identified (always the same value in the game data): +20, +24, +36, +40, +48, +52.
    pub raw: [u32; 6],
}

impl Template {
    pub fn play_mode(&self) -> PlayMode {
        match (self.mode_word >> 2) & 3 {
            0 => PlayMode::Sequential,
            1 => PlayMode::Random,
            2 => PlayMode::Shuffle,
            _ => PlayMode::Programmer,
        }
    }
}

/// One entry of a sound definition.
#[derive(Clone, Debug)]
pub enum Wave {
    /// A sound in a wave bank of the project.
    Bank {
        /// Selection weight in percent.
        weight: u32,
        /// Name of the sound in the bank (the FSB name table, without the extension).
        name: String,
        /// Index in [`Fev::banks`].
        bank: u32,
        /// Index of the subsound in the bank.
        index: u32,
        /// Length in milliseconds.
        length_ms: u32,
    },
    /// An oscillator (test tone).
    Oscillator { weight: u32, kind: u32, rate: u32 },
    /// Silence.
    Null { weight: u32 },
    /// Picked by the game at run time (unused by the game data).
    Programmer { weight: u32 },
}

impl Wave {
    pub fn weight(&self) -> u32 {
        match *self {
            Wave::Bank { weight, .. } | Wave::Oscillator { weight, .. } | Wave::Null { weight } | Wave::Programmer { weight } => weight,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SoundDef {
    pub name: String,
    /// Index in [`Fev::templates`].
    pub template: usize,
    pub waves: Vec<Wave>,
}

/// A sound placed on a layer: what plays when the layer's parameter is in `start..start + length`.
#[derive(Clone, Debug)]
pub struct Instance {
    /// Index in [`Fev::sound_defs`].
    pub sound_def: usize,
    pub start: f32,
    pub length: f32,
    pub autopitch: bool,
    /// The loop word as stored: 1 plays once, 0 loops and is cut off when the event stops, 2 loops
    /// and finishes the pass (checked on the data: footsteps, UI and voices are all 1, ambience,
    /// music and projectile loops 0).
    pub loop_word: u32,
    /// The raw words that follow (autopitch parameter, flags, fade values ...), see the source.
    pub raw: [u32; 10],
}

impl Instance {
    /// The sound repeats until the event is stopped.
    pub fn loops(&self) -> bool {
        self.loop_word & 1 == 0
    }
}

/// A point of an envelope curve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvPoint {
    /// Normalized parameter position (0 to 1, including for distance parameters).
    pub x: f32,
    /// Level, 0 to 1 for volume.
    pub y: f32,
    /// Shape bits of the segment ending here: 1 smooth, 2 linear, 4 exponential, 8 inverse cubic.
    /// Recovered from `fmod_eventl.dll!sub_10010AD0`; these are flags, not an ordinal.
    pub curve: u32,
}

/// A curve on a layer: what a layer property does as the parameter `parameter` moves.
#[derive(Clone, Debug)]
pub struct Envelope {
    pub parent: i32,
    pub dsp: String,
    /// Which parameter of the DSP effect the curve drives (0 is the cutoff of a low-pass); for an occlusion curve (0x2000)
    /// 0 is the direct path and 1 the reverb path.
    pub dsp_param: u32,
    /// What the curve drives: 0x8 volume, 0x10 pitch, 0x20 pan, 0x40 time offset, 0x80 surround pan,
    /// 0x100 3D speaker spread, 0x200 reverb level, 0x400 3D pan level, 0x800 reverb balance,
    /// 0x1000 spawn intensity (plus effect parameters on other bits).
    pub flags: u32,
    pub flags2: u32,
    pub points: Vec<EnvPoint>,
    /// Index in the event's parameter list.
    pub parameter: u32,
}

impl Envelope {
    /// The curve value at `x`, held outside the first and last point.
    pub fn at(&self, x: f32) -> f32 {
        let p = &self.points;
        let (Some(first), Some(last)) = (p.first(), p.last()) else { return 1.0 };
        if x <= first.x {
            return first.y;
        }
        if x >= last.x {
            return last.y;
        }
        let i = p.windows(2).position(|w| x < w[1].x).unwrap_or(0);
        let (a, b) = (p[i], p[i + 1]);
        let t = if b.x > a.x { (x - a.x) / (b.x - a.x) } else { 1.0 };
        // FMOD's evaluator reads the destination point's flags. In particular most original
        // distance curves end in 2 (linear); using the first point or an ordinal changes rolloff.
        let shape = b.curve;
        if shape & 1 != 0 {
            a.y + (b.y - a.y) * t * t * (3.0 - 2.0 * t)
        } else if shape & 2 != 0 {
            a.y + (b.y - a.y) * t
        } else if shape & 4 != 0 {
            let t = if a.y >= b.y { 1.0 - t } else { t };
            a.y.min(b.y) + (b.y - a.y).abs() * (1025.0f32.powf(t) - 1.0) / 1024.0
        } else if shape & 8 != 0 {
            a.y + (b.y - a.y) * (0.5 + 4.0 * (t - 0.5).powi(3))
        } else {
            0.0
        }
    }
}

#[derive(Clone, Debug)]
pub struct Layer {
    pub flags: u16,
    pub priority: i16,
    pub control_parameter: u16,
    pub instances: Vec<Instance>,
    pub envelopes: Vec<Envelope>,
}

#[derive(Clone, Debug)]
pub struct Parameter {
    pub name: String,
    pub velocity: f32,
    pub min: f32,
    pub max: f32,
    pub flags: u32,
    pub seek_speed: f32,
    pub raw_44: u32,
    pub sustain_points: Vec<f32>,
}

#[derive(Clone, Debug)]
pub enum Body {
    /// One sound instance.
    Simple(Instance),
    Complex { layers: Vec<Layer>, parameters: Vec<Parameter>, props: Vec<UserProp> },
}

/// How loudness falls off with distance in a 3D event (FMOD_3D_*ROLLOFF).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rolloff {
    /// Inverse distance: quiet beyond `min`, halving per doubling (the FMOD default).
    Log,
    Linear,
    LinearSquare,
    /// A curve drawn in the project (stored with the envelope points, not read).
    Custom,
}

#[derive(Clone, Debug)]
pub struct Event {
    pub name: String,
    /// `group/subgroup`, empty for events directly in the project.
    pub group: String,
    pub flags: u32,
    pub guid: [u8; 16],
    /// Linear amplitude, 1.0 unchanged.
    pub volume: f32,
    /// As stored: octaves = `pitch * 4`.
    pub pitch: f32,
    /// As stored: octaves = `pitch_random * 4`, applied as +- around the pitch.
    pub pitch_random: f32,
    /// 0 to 1: the volume is scaled by a random factor in `1 - volume_random ..= 1`.
    pub volume_random: f32,
    /// 0 is the most important, 256 the least.
    pub priority: u32,
    pub max_playbacks: u32,
    /// What happens when `max_playbacks` is reached: 0 or 1 stop the oldest, 2 the newest, 3 the
    /// quietest, 4 refuse the new one, 5 refuse it if it is the quietest.
    pub playback_behaviour: u32,
    /// FMOD mode bits: 0x8 2D, 0x10 3D, 0x100000 log, 0x200000 linear, 0x400000 linear square and
    /// 0x4000000 custom rolloff, 0x40000 head relative, 0x80000 world relative.
    pub mode: u32,
    pub min_distance: f32,
    pub max_distance: f32,
    pub flags2: u32,
    pub speaker_levels: [f32; 8],
    pub cone_inside: f32,
    pub cone_outside: f32,
    pub cone_outside_volume: f32,
    pub doppler: f32,
    pub reverb_dry: f32,
    pub reverb_wet: f32,
    pub speaker_spread: f32,
    pub pan_level: f32,
    pub fade_in_ms: u32,
    pub fade_out_ms: u32,
    pub spawn_intensity: f32,
    pub spawn_intensity_random: f32,
    /// Fields whose meaning is not known: file offsets (in the event record) 152, 156.. kept raw.
    pub raw: [u32; 4],
    /// Mixer category path, e.g. `master/skill`.
    pub category: String,
    pub body: Body,
}

impl Event {
    /// `true` for events positioned in the world; otherwise the event plays 2D.
    pub fn is_3d(&self) -> bool {
        self.mode & 0x10 != 0
    }
    pub fn rolloff(&self) -> Rolloff {
        if self.mode & 0x400_0000 != 0 {
            Rolloff::Custom
        } else if self.mode & 0x20_0000 != 0 {
            Rolloff::Linear
        } else if self.mode & 0x40_0000 != 0 {
            Rolloff::LinearSquare
        } else {
            Rolloff::Log
        }
    }
    /// The event stops by itself after one pass of its sounds.
    pub fn one_shot(&self) -> bool {
        self.flags2 & 0x8_0000 != 0
    }
    /// Pitch as a frequency ratio, before randomization.
    pub fn pitch_ratio(&self) -> f32 {
        (self.pitch * 4.0).exp2()
    }
    pub fn instances(&self) -> Vec<&Instance> {
        match &self.body {
            Body::Simple(i) => vec![i],
            Body::Complex { layers, .. } => layers.iter().flat_map(|l| l.instances.iter()).collect(),
        }
    }
    pub fn is_simple(&self) -> bool {
        matches!(self.body, Body::Simple(_))
    }
    /// `group/name`.
    pub fn path(&self) -> String {
        if self.group.is_empty() { self.name.clone() } else { format!("{}/{}", self.group, self.name) }
    }
}

/// A parsed project.
#[derive(Clone, Debug)]
pub struct Fev {
    pub name: String,
    pub banks: Vec<Bank>,
    pub categories: Category,
    /// All groups, as `a/b` paths, parents before children.
    pub groups: Vec<String>,
    pub events: Vec<Event>,
    pub templates: Vec<Template>,
    pub sound_defs: Vec<SoundDef>,
    pub reverbs: Vec<String>,
    /// Bytes of interactive music data after the sound definitions (not read).
    pub music_bytes: usize,
    index: HashMap<String, usize>,
}

fn chunks(b: &[u8]) -> Result<HashMap<[u8; 4], &[u8]>> {
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"FEV " {
        return Err(bad("not a FEV file"));
    }
    let mut out = HashMap::new();
    let mut stack: Vec<(usize, usize)> = vec![(12, (8 + u32::from_le_bytes(b[4..8].try_into().unwrap()) as usize).min(b.len()))];
    while let Some((mut o, end)) = stack.pop() {
        while o + 8 <= end {
            let tag: [u8; 4] = b[o..o + 4].try_into().unwrap();
            let size = u32::from_le_bytes(b[o + 4..o + 8].try_into().unwrap()) as usize;
            let body_end = (o + 8 + size).min(end);
            if &tag == b"LIST" {
                if o + 12 <= body_end {
                    stack.push((o + 12, body_end));
                }
            } else {
                out.insert(tag, &b[o + 8..body_end]);
            }
            o += 8 + size + (size & 1);
        }
    }
    Ok(out)
}

fn string_table(b: &[u8]) -> Result<Vec<String>> {
    let mut r = Rd { b, o: 0 };
    let n = r.count()?;
    let offsets: Vec<usize> = (0..n).map(|_| r.u32().map(|v| v as usize)).collect::<Result<_>>()?;
    let base = r.o;
    offsets
        .iter()
        .map(|&o| {
            let s = b.get(base + o..).ok_or(Error::Truncated("fev strings"))?;
            let end = s.iter().position(|&c| c == 0).ok_or(Error::Truncated("fev string"))?;
            Ok(s[..end].iter().map(|&c| c as char).collect())
        })
        .collect()
}

struct Ctx<'a> {
    strings: &'a [String],
    points: Vec<EnvPoint>,
    categories: Vec<String>,
    events: Vec<Event>,
    groups: Vec<String>,
}

impl Ctx<'_> {
    fn s(&self, i: u32) -> Result<String> {
        self.strings.get(i as usize).cloned().ok_or_else(|| bad(format!("string index {i} out of {}", self.strings.len())))
    }
}

fn user_prop(r: &mut Rd) -> Result<UserProp> {
    let name = r.string()?;
    let kind = r.u32()?;
    let value = match kind {
        0 => PropValue::Int(r.i32()?),
        1 => PropValue::Float(r.f32()?),
        2 => PropValue::Text(r.string()?),
        k => return Err(bad(format!("user property type {k}"))),
    };
    Ok(UserProp { name, value })
}

fn category(r: &mut Rd) -> Result<Category> {
    let name = r.string()?;
    let volume = r.f32()?;
    let pitch = r.f32()?;
    let max_playbacks = r.u32()?;
    let _behaviour = r.u32()?;
    let n = r.count()?;
    let children = (0..n).map(|_| category(r)).collect::<Result<_>>()?;
    Ok(Category { name, volume, pitch, max_playbacks, children })
}

fn instance(r: &mut Rd) -> Result<Instance> {
    let sound_def = r.u16()? as usize;
    let start = r.f32()?;
    let length = r.f32()?;
    let autopitch = r.u32()? != 0;
    let loop_word = r.u32()?;
    // autopitch parameter, flag, 40, 44, 48, 28, 32, 36, flags, flags
    let mut raw = [0u32; 10];
    for v in raw.iter_mut() {
        *v = r.u32()?;
    }
    Ok(Instance { sound_def, start, length, autopitch, loop_word, raw })
}

fn envelope(r: &mut Rd, ctx: &Ctx) -> Result<Envelope> {
    let parent = r.i32()?;
    let dsp = r.string()?;
    let dsp_param = r.u32()?;
    let flags = r.u32()?;
    let flags2 = r.u32()?;
    let n = r.count()?;
    let mut points = Vec::with_capacity(n);
    for _ in 0..n {
        let id = r.u32()? as usize;
        points.push(*ctx.points.get(id).ok_or_else(|| bad(format!("envelope point {id} of {}", ctx.points.len())))?);
    }
    let parameter = r.u32()?;
    let _flag = r.u32()?;
    Ok(Envelope { parent, dsp, dsp_param, flags, flags2, points, parameter })
}

fn parameter(r: &mut Rd, ctx: &Ctx) -> Result<Parameter> {
    let name = ctx.s(r.u32()?)?;
    let velocity = r.f32()?;
    let min = r.f32()?;
    let max = r.f32()?;
    let flags = r.u32()?;
    let seek_speed = r.f32()?;
    let raw_44 = r.u32()?;
    let n = r.count()?;
    let sustain_points = (0..n).map(|_| r.f32()).collect::<Result<_>>()?;
    Ok(Parameter { name, velocity, min, max, flags, seek_speed, raw_44, sustain_points })
}

fn complex(r: &mut Rd, ctx: &Ctx) -> Result<Body> {
    let nlayers = r.count()?;
    let mut layers = Vec::with_capacity(nlayers);
    for _ in 0..nlayers {
        let flags = r.u16()?;
        let priority = r.u16()? as i16;
        let control_parameter = r.u16()?;
        let ninst = r.u16()? as usize;
        let nenv = r.u16()? as usize;
        let instances = (0..ninst).map(|_| instance(r)).collect::<Result<_>>()?;
        let envelopes = (0..nenv).map(|_| envelope(r, ctx)).collect::<Result<_>>()?;
        layers.push(Layer { flags, priority, control_parameter, instances, envelopes });
    }
    let nparams = r.count()?;
    let parameters = (0..nparams).map(|_| parameter(r, ctx)).collect::<Result<_>>()?;
    let nprops = r.count()?;
    let props = (0..nprops).map(|_| user_prop(r)).collect::<Result<_>>()?;
    Ok(Body::Complex { layers, parameters, props })
}

fn event(r: &mut Rd, ctx: &mut Ctx, group: &str) -> Result<()> {
    let flags = r.u32()?;
    let name = ctx.s(r.u32()?)?;
    let guid: [u8; 16] = r.take(16)?.try_into().unwrap();
    let volume = r.f32()?;
    let pitch = r.f32()?;
    let pitch_random = r.f32()?;
    let volume_random = r.f32()?;
    let priority = r.u32()?;
    let max_playbacks = r.u32()?.max(1);
    let raw152 = r.u32()?;
    let mode = r.u32()?;
    let min_distance = r.f32()?;
    let max_distance = r.f32()?;
    let raw300 = r.u32()?;
    let raw304 = r.u32()?;
    let flags2 = r.u32()?;
    let mut speaker_levels = [0f32; 8];
    for v in speaker_levels.iter_mut() {
        *v = r.f32()?;
    }
    let cone_inside = r.f32()?;
    let cone_outside = r.f32()?;
    let cone_outside_volume = r.f32()?;
    let playback_behaviour = r.u32()?;
    let doppler = r.f32()?.clamp(0.0, 5.0);
    let reverb_dry = r.f32()?;
    let reverb_wet = r.f32()?;
    let speaker_spread = r.f32()?;
    let fade_in_ms = r.u32()? & 0xFFFF;
    let fade_out_ms = r.u32()? & 0xFFFF;
    let spawn_intensity = r.f32()?;
    let spawn_intensity_random = r.f32()?;
    let pan_level = r.f32()?;
    let raw260 = r.u32()?;
    let raw264 = r.u32()?;
    let body = if flags & 0x10 != 0 {
        let _ = r.u32()?;
        Body::Simple(instance(r)?)
    } else {
        complex(r, ctx)?
    };
    let ncat = r.count()?;
    let mut category = String::new();
    for i in 0..ncat {
        let c = r.string()?;
        if i == 0 {
            category = c;
        }
    }
    if category.is_empty() {
        category = "master".into();
    }
    let _ = (raw300, raw304);
    ctx.events.push(Event {
        name,
        group: group.to_string(),
        flags,
        guid,
        volume,
        pitch,
        pitch_random,
        volume_random,
        priority,
        max_playbacks,
        playback_behaviour,
        mode,
        min_distance,
        max_distance,
        flags2,
        speaker_levels,
        cone_inside,
        cone_outside,
        cone_outside_volume,
        doppler,
        reverb_dry,
        reverb_wet,
        speaker_spread,
        pan_level,
        fade_in_ms,
        fade_out_ms,
        spawn_intensity,
        spawn_intensity_random,
        raw: [raw152, raw300, raw260, raw264],
        category,
        body,
    });
    Ok(())
}

fn group(r: &mut Rd, ctx: &mut Ctx, parent: &str) -> Result<()> {
    let name = ctx.s(r.u32()?)?;
    let path = if parent.is_empty() { name } else { format!("{parent}/{name}") };
    let nprops = r.count()?;
    for _ in 0..nprops {
        user_prop(r)?;
    }
    let nsub = r.count()?;
    let nev = r.count()?;
    ctx.groups.push(path.clone());
    for _ in 0..nsub {
        group(r, ctx, &path)?;
    }
    for _ in 0..nev {
        event(r, ctx, &path)?;
    }
    Ok(())
}

fn template(r: &mut Rd) -> Result<Template> {
    let mode_word = r.u32()?;
    let spawn_time_min_ms = r.u32()?;
    let spawn_time_max_ms = r.u32()?;
    let max_spawned = r.u32()?;
    let volume = r.f32()?;
    let _flag_d = r.u32()?;
    let r20 = r.u32()?;
    let r24 = r.u32()?;
    let volume_random_min = r.f32()?;
    let pitch = r.f32()?;
    let _flag_i = r.u32()?;
    let r36 = r.u32()?;
    let r40 = r.u32()?;
    let pitch_random = r.f32()?;
    let _m = r.u32()?;
    let r48 = r.u32()?;
    let r52 = r.u32()?;
    let fade_in_ms = r.u16()?;
    let fade_out_ms = r.u16()?;
    let _crossfade = r.u16()?;
    Ok(Template {
        mode_word,
        spawn_time_min_ms,
        spawn_time_max_ms,
        max_spawned,
        volume,
        volume_random_min,
        pitch,
        pitch_random,
        fade_in_ms,
        fade_out_ms,
        raw: [r20, r24, r36, r40, r48, r52],
    })
}

fn sound_def(r: &mut Rd, ctx: &Ctx, banks: usize, templates: usize) -> Result<SoundDef> {
    let name = ctx.s(r.u32()?)?;
    let template = r.u32()? as usize;
    if template >= templates {
        return Err(bad(format!("sound def {name}: template {template} of {templates}")));
    }
    let n = r.count()?;
    let mut waves = Vec::with_capacity(n);
    for _ in 0..n {
        let kind = r.u32()?;
        let weight = r.u32()?;
        waves.push(match kind {
            0 => {
                let wave_name = r.string()?;
                let bank = r.u32()?;
                let index = r.u32()?;
                let length_ms = r.u32()?;
                if bank as usize >= banks {
                    return Err(bad(format!("sound def {name}: bank {bank} of {banks}")));
                }
                Wave::Bank { weight, name: wave_name, bank, index, length_ms }
            }
            1 => {
                let kind = r.u32()?;
                let rate = r.u32()?;
                Wave::Oscillator { weight, kind, rate }
            }
            2 => Wave::Null { weight },
            3 => Wave::Programmer { weight },
            k => return Err(bad(format!("sound def {name}: wave type {k}"))),
        });
    }
    Ok(SoundDef { name, template, waves })
}

impl Fev {
    /// Parses a project. Fails on any other FEV version or on data that does not end exactly where
    /// the format says.
    pub fn parse(bytes: &[u8]) -> Result<Fev> {
        let ch = chunks(bytes)?;
        let version = ch.get(b"FMT ").filter(|c| c.len() >= 4).map(|c| u32::from_le_bytes(c[0..4].try_into().unwrap()));
        if version != Some(VERSION) {
            return Err(bad(format!("version {version:x?}, only {VERSION:#x} is supported")));
        }
        let strings = string_table(ch.get(b"STRR").ok_or_else(|| bad("no STRR chunk"))?)?;
        let points = match ch.get(b"EPRP") {
            Some(e) => {
                let mut r = Rd { b: e, o: 0 };
                let n = r.count()?;
                let pts = (0..n).map(|_| Ok(EnvPoint { x: r.f32()?, y: r.f32()?, curve: r.u32()? })).collect::<Result<Vec<_>>>()?;
                if r.o != e.len() {
                    return Err(bad("EPRP size"));
                }
                pts
            }
            None => Vec::new(),
        };
        let lgcy = *ch.get(b"LGCY").ok_or_else(|| bad("no LGCY chunk"))?;
        let mut r = Rd { b: lgcy, o: 8 };
        let name = r.string()?;
        let nbanks = r.count()?;
        let hashes = r.count()?;
        let mut banks = Vec::with_capacity(nbanks);
        for _ in 0..nbanks {
            let mode = r.u32()?;
            let max_streams = r.u32()?;
            for _ in 0..hashes {
                r.take(8)?;
                r.u32()?;
            }
            banks.push(Bank { name: r.string()?, mode, max_streams });
        }
        let categories = category(&mut r)?;
        let mut ctx = Ctx { strings: &strings, points, categories: categories.paths(), events: Vec::new(), groups: Vec::new() };
        let ngroups = r.count()?;
        for _ in 0..ngroups {
            group(&mut r, &mut ctx, "")?;
        }
        let ntemplates = r.count()?;
        let templates = (0..ntemplates).map(|_| template(&mut r)).collect::<Result<Vec<_>>>()?;
        let nsounds = r.count()?;
        let sound_defs = (0..nsounds).map(|_| sound_def(&mut r, &ctx, banks.len(), templates.len())).collect::<Result<Vec<_>>>()?;
        let nreverbs = r.count()?;
        let mut reverbs = Vec::with_capacity(nreverbs);
        for _ in 0..nreverbs {
            reverbs.push(r.string()?);
            r.take(33 * 4)?;
        }
        // Music chunks: skip them, they are only measured.
        let music_start = r.o;
        let mut music_end = r.o;
        loop {
            let size = r.u32()? as usize;
            let tag = r.take(4)?;
            if tag == b"comp" {
                music_end = r.o + size.saturating_sub(8);
            } else {
                r.take(size.saturating_sub(8))?;
            }
            if r.o >= music_end {
                break;
            }
        }
        r.take(music_end.saturating_sub(r.o))?;
        if r.o != lgcy.len() {
            return Err(bad(format!("{} bytes left after the music data", lgcy.len() - r.o)));
        }
        let music_bytes = r.o - music_start - 8;

        // Event categories were read as names; resolve them to the category tree paths.
        let mut events = ctx.events;
        for e in &mut events {
            if let Some(p) = ctx.categories.iter().find(|p| p.rsplit('/').next() == Some(&e.category)) {
                e.category = p.clone();
            }
        }
        let mut index = HashMap::new();
        for (i, e) in events.iter().enumerate() {
            index.entry(e.path().to_ascii_lowercase()).or_insert(i);
        }
        Ok(Fev { name, banks, categories, groups: ctx.groups, events, templates, sound_defs, reverbs, music_bytes, index })
    }

    /// An event by `group/subgroup/name` (case-insensitive).
    pub fn event(&self, path: &str) -> Option<&Event> {
        let key = path.replace('\\', "/").to_ascii_lowercase();
        self.index.get(key.trim_matches('/')).map(|&i| &self.events[i])
    }

    pub fn event_index(&self, path: &str) -> Option<usize> {
        let key = path.replace('\\', "/").to_ascii_lowercase();
        self.index.get(key.trim_matches('/')).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_other_data() {
        assert!(Fev::parse(b"RIFF\0\0\0\0WAVEfmt ").is_err());
        assert!(Fev::parse(b"").is_err());
    }

    fn curve(shape: u32) -> Envelope {
        Envelope {
            parent: -1,
            dsp: String::new(),
            dsp_param: 0,
            flags: 8,
            flags2: 0,
            // Distinct shape flags make a source/destination mixup visible.
            points: vec![EnvPoint { x: 0.0, y: 0.0, curve: 0 }, EnvPoint { x: 1.0, y: 1.0, curve: shape }],
            parameter: 0,
        }
    }

    #[test]
    fn segment_uses_the_destination_shape_flags() {
        // Golden values from the FMOD evaluator, not an ordinal interpretation of the flags.
        let samples = [(1, 0.15625), (2, 0.25), (4, 0.0045490574), (8, 0.4375), (3, 0.15625)];
        for (shape, want) in samples {
            let env = curve(shape);
            assert!((env.at(0.25) - want).abs() < 1e-6, "shape {shape}: {} vs {want}", env.at(0.25));
            assert_eq!(env.at(0.0), 0.0);
            assert_eq!(env.at(1.0), 1.0);
        }
    }

    #[test]
    fn exponential_segments_reverse_with_falling_levels() {
        let mut env = curve(4);
        env.points[0].y = 1.0;
        env.points[1].y = 0.0;
        assert!((env.at(0.75) - 0.0045490574).abs() < 1e-6);
        assert!((env.at(0.25) - 0.17592958).abs() < 1e-6);
    }

    #[test]
    fn curve_knots_and_constant_envelopes_keep_their_authored_levels() {
        let mut env = curve(2);
        env.points.insert(1, EnvPoint { x: 0.5, y: 0.8, curve: 1 });
        assert_eq!(env.at(-1.0), 0.0);
        assert_eq!(env.at(0.5), 0.8);
        assert!((env.at(0.75) - 0.9).abs() < 1e-6);
        assert_eq!(env.at(2.0), 1.0);
        env.points.truncate(1);
        env.points[0].y = 0.47;
        assert_eq!(env.at(0.75), 0.47);
    }
}
