//! The sound library: FEV projects and FSB banks read lazily from the pak, event path resolution,
//! and the plan for what one trigger of an event plays.
//!
//! # Event paths
//!
//! The game names an event `sounds/<project>:<group>:<event>` (any case, `/` or `\`), where
//! `<project>` is the FMOD project (`game/sounds/<project>/<project>.fev`), `<group>` the event group
//! (nested groups are joined with `/`) and `<event>` the event. Variants seen in the data:
//!
//! * `Sounds/x2skills:wild:fire_arrow_form` (the folder is `Sounds` or `sounds`, or missing)
//! * `sounds/x2weapons:x2weapons:shield_flesh` (the group can repeat the project name)
//! * `sounds/x2skills/monster_skill:pino_effect.mon.mon_heal_hit` (group after the project, one colon)
//! * `x2skills/wild/fire_arrow_form` (FMOD's own path, no colons)
//!
//! Voice projects (`x2voice_*`) have one FEV per language, `sounds/<project>/<lang>/<project>.fev`,
//! with the banks next to it; the library prefers the configured language and falls back to the
//! project root.

use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use aa_pak::Pak;
use bevy::math::Vec3;

use crate::decode::{Pcm, decode_data};
use crate::fev::{Body, EnvPoint, Envelope, Event, Fev, Rolloff, Wave};
use crate::fsb5::{Error, Fsb5, Result};

/// A parsed FMOD project and where its banks are.
pub struct Project {
    pub name: String,
    /// Folder of the FEV and its banks, e.g. `sounds/x2voice_combat/en_us`.
    pub dir: String,
    pub fev: Fev,
}

/// An event of a project.
#[derive(Clone)]
pub struct EventRef {
    pub project: Arc<Project>,
    pub index: usize,
}

impl EventRef {
    pub fn event(&self) -> &Event {
        &self.project.fev.events[self.index]
    }

    /// `project:group:event`.
    pub fn name(&self) -> String {
        let e = self.event();
        format!("{}:{}:{}", self.project.name, e.group, e.name)
    }

    /// Identity of the event across projects (for counting playbacks).
    pub fn key(&self) -> (usize, usize) {
        (Arc::as_ptr(&self.project) as usize, self.index)
    }
}

/// A bank file: its place in the pak and its parsed header.
pub struct BankFile {
    pub path: String,
    pub offset: u64,
    pub size: u64,
    pub fsb: Fsb5,
}

/// The parts of an event path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventPath {
    pub project: String,
    /// `a/b`, empty for none.
    pub group: String,
    pub event: String,
}

impl EventPath {
    /// `group/event`, the key inside a project.
    pub fn local(&self) -> String {
        if self.group.is_empty() { self.event.clone() } else { format!("{}/{}", self.group, self.event) }
    }
}

/// Splits an event path (see the module docs). Everything is lower case.
pub fn parse_path(path: &str) -> Option<EventPath> {
    let p = path.trim().replace('\\', "/").to_ascii_lowercase();
    let p = p.trim_start_matches('/');
    let p = p.strip_prefix("game/").unwrap_or(p);
    let p = p.strip_prefix("sounds/").unwrap_or(p);
    if p.is_empty() {
        return None;
    }
    let (head, rest): (Vec<&str>, Vec<&str>) = if p.contains(':') {
        let parts: Vec<&str> = p.split(':').collect();
        let (first, others) = parts.split_first()?;
        (first.split('/').filter(|s| !s.is_empty()).collect(), others.to_vec())
    } else {
        let parts: Vec<&str> = p.split('/').filter(|s| !s.is_empty()).collect();
        let (first, others) = parts.split_first()?;
        (vec![*first], others.to_vec())
    };
    let project = head.first()?.to_string();
    let (event, groups) = if p.contains(':') {
        let event = rest.last()?;
        let mut groups: Vec<&str> = head[1..].to_vec();
        groups.extend(&rest[..rest.len() - 1]);
        (event.to_string(), groups)
    } else {
        let event = rest.last()?;
        (event.to_string(), rest[..rest.len() - 1].to_vec())
    };
    let group = groups.iter().filter(|g| !g.is_empty()).copied().collect::<Vec<_>>().join("/");
    if event.is_empty() {
        return None;
    }
    Some(EventPath { project, group, event })
}

/// The pak, the projects and the banks. One per process: [`Library::global`].
pub struct Library {
    pak: &'static Pak,
    file: File,
    language: RwLock<String>,
    projects: Mutex<HashMap<String, Option<Arc<Project>>>>,
    banks: Mutex<HashMap<String, Option<Arc<BankFile>>>>,
}

fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        let mut done = 0;
        while done < buf.len() {
            let n = file.seek_read(&mut buf[done..], offset + done as u64)?;
            if n == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            done += n;
        }
        Ok(())
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        file.read_exact_at(buf, offset)
    }
}

impl Library {
    pub fn new(pak: &'static Pak) -> io::Result<Library> {
        Ok(Library {
            pak,
            file: File::open(pak.path())?,
            language: RwLock::new("en_us".into()),
            projects: Mutex::default(),
            banks: Mutex::default(),
        })
    }

    /// The library over [`Pak::global`].
    pub fn global() -> &'static Library {
        static LIB: OnceLock<Library> = OnceLock::new();
        LIB.get_or_init(|| Library::new(Pak::global()).unwrap_or_else(|e| panic!("cannot open the game pak: {e}")))
    }

    pub fn pak(&self) -> &'static Pak {
        self.pak
    }

    /// Language folder used for the voice projects (`en_us`, `fr`, `de`). Call before the first
    /// lookup; projects loaded earlier keep the previous language.
    pub fn set_language(&self, lang: &str) {
        *self.language.write().unwrap_or_else(|e| e.into_inner()) = lang.to_ascii_lowercase();
    }

    fn load_project(&self, name: &str, dir: &str) -> Option<Arc<Project>> {
        let path = format!("{dir}/{name}.fev");
        let bytes = self.pak.read(&path).ok()?;
        match Fev::parse(&bytes) {
            Ok(fev) => Some(Arc::new(Project { name: name.to_string(), dir: dir.to_string(), fev })),
            Err(e) => {
                tracing::warn!("aa-audio: {path}: {e}");
                None
            }
        }
    }

    fn cached_project(&self, key: &str, name: &str, dir: &str) -> Option<Arc<Project>> {
        if let Some(p) = self.projects.lock().unwrap_or_else(|e| e.into_inner()).get(key) {
            return p.clone();
        }
        // Load outside the lock: parsing a big FEV takes a few milliseconds.
        let loaded = self.load_project(name, dir);
        self.projects.lock().unwrap_or_else(|e| e.into_inner()).entry(key.to_string()).or_insert(loaded).clone()
    }

    /// The FMOD project `name` (`x2skills`) in the configured language, if the pak has it.
    pub fn project(&self, name: &str) -> Option<Arc<Project>> {
        let name = name.to_ascii_lowercase();
        let lang = self.language.read().unwrap_or_else(|e| e.into_inner()).clone();
        let lang_dir = format!("sounds/{name}/{lang}");
        if self.pak.exists(&format!("{lang_dir}/{name}.fev")) {
            return self.cached_project(&format!("{name}/{lang}"), &name, &lang_dir);
        }
        self.root_project(&name)
    }

    /// The project from the folder without language.
    pub fn root_project(&self, name: &str) -> Option<Arc<Project>> {
        let name = name.to_ascii_lowercase();
        self.cached_project(&name, &name, &format!("sounds/{name}"))
    }

    /// The event for a path, `None` when the path is malformed, the project is not in the pak or
    /// the project does not have the event.
    pub fn event(&self, path: &str) -> Option<EventRef> {
        let p = parse_path(path)?;
        let local = p.local();
        let project = self.project(&p.project)?;
        if let Some(index) = project.fev.event_index(&local) {
            return Some(EventRef { project, index });
        }
        // A language project can lack what the root one has.
        let root = self.root_project(&p.project)?;
        if Arc::ptr_eq(&root, &project) {
            return None;
        }
        root.fev.event_index(&local).map(|index| EventRef { project: root, index })
    }

    /// The bank `bank` (index in the project's list) with its header read.
    pub fn bank(&self, project: &Project, bank: u32) -> Result<Arc<BankFile>> {
        let name = &project.fev.banks.get(bank as usize).ok_or_else(|| Error::Other(format!("bank {bank} of {}", project.name)))?.name;
        let path = format!("{}/{}.fsb", project.dir, name.to_ascii_lowercase());
        if let Some(b) = self.banks.lock().unwrap_or_else(|e| e.into_inner()).get(&path) {
            return b.clone().ok_or_else(|| Error::Other(format!("{path}: not in the pak")));
        }
        let loaded = self.open_bank(&path);
        let mut banks = self.banks.lock().unwrap_or_else(|e| e.into_inner());
        let slot = banks.entry(path.clone()).or_insert_with(|| loaded.as_ref().ok().cloned());
        match (slot.clone(), loaded) {
            (Some(b), _) => Ok(b),
            (None, Err(e)) => Err(e),
            (None, Ok(_)) => unreachable!(),
        }
    }

    fn open_bank(&self, path: &str) -> Result<Arc<BankFile>> {
        let entry = self.pak.entry(path).ok_or_else(|| Error::Other(format!("{path}: not in the pak")))?;
        let mut head = [0u8; 64];
        let first = head.len().min(entry.size as usize);
        read_at(&self.file, &mut head[..first], entry.offset)?;
        if &head[..4] != b"FSB5" {
            return Err(Error::NotFsb5);
        }
        let u32_at = |o: usize| u32::from_le_bytes(head[o..o + 4].try_into().unwrap()) as u64;
        let fixed = if u32_at(4) == 0 { 64 } else { 60 };
        let header_len = (fixed + u32_at(12) + u32_at(16)).min(entry.size);
        let mut header = vec![0u8; header_len as usize];
        read_at(&self.file, &mut header, entry.offset)?;
        let fsb = Fsb5::parse(&header)?;
        Ok(Arc::new(BankFile { path: path.to_string(), offset: entry.offset, size: entry.size, fsb }))
    }

    /// The encoded bytes of one subsound, read from the pak.
    pub fn read_subsound(&self, bank: &BankFile, index: usize) -> Result<Vec<u8>> {
        let sub = bank.fsb.subsounds.get(index).ok_or(Error::NoSubsound(index))?;
        let start = bank.fsb.data_offset as u64 + sub.data_start as u64;
        let len = (sub.data_len as u64).min(bank.size.saturating_sub(start));
        let mut buf = vec![0u8; len as usize];
        read_at(&self.file, &mut buf, bank.offset + start)?;
        Ok(buf)
    }

    /// Decodes one subsound completely.
    pub fn decode(&self, bank: &BankFile, index: usize) -> Result<Pcm> {
        let data = self.read_subsound(bank, index)?;
        decode_data(&bank.fsb, index, &data)
    }
}

/// Small xorshift generator: variation does not need a dependency.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed | 1)
    }
    pub fn from_time() -> Rng {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
        Rng::new(t ^ 0x9E37_79B9_7F4A_7C15)
    }
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// 0 to 1.
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
    /// -1 to 1.
    pub fn signed(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next_u64() % n as u64) as usize }
    }
}

/// Where sequential and shuffled sound definitions are in their order.
#[derive(Default)]
pub struct PickState {
    seen: HashMap<(usize, usize), (usize, Vec<bool>)>,
}

/// One sound an event starts: an instance of a layer with the wave picked for it.
#[derive(Clone, Debug)]
pub struct Planned {
    pub layer: usize,
    /// Index of the instance in the layer (0 for a simple event).
    pub instance: usize,
    /// Index of the bank in the project.
    pub bank: u32,
    pub index: u32,
    /// Name from the FEV (`wild/new_22k/wild_fire_arrow_form1.wav`).
    pub name: String,
    /// Linear amplitude: event, sound definition and their randomization (not the envelopes, which
    /// follow the parameters: see [`layer_gain`]).
    pub gain: f32,
    /// Playback rate (frequency ratio) after pitch and randomization.
    pub rate: f32,
    pub looping: bool,
    /// The sound definition has several waves: a looping instance plays them one after the other (the next pick of the play
    /// mode each time round) instead of repeating one.
    pub playlist: bool,
    /// Length of the picked wave.
    pub length_ms: u32,
    pub fade_in_ms: u32,
    pub fade_out_ms: u32,
    /// While the instance is active a new sound of the definition starts every `min..=max` ms, with at most `max_spawned` of
    /// them playing at once.
    pub spawn_ms: Option<(u32, u32)>,
    pub max_spawned: u32,
}

fn pick_wave<'a>(state: &mut PickState, rng: &mut Rng, key: (usize, usize), def: &'a crate::fev::SoundDef, mode: crate::fev::PlayMode) -> Option<&'a Wave> {
    use crate::fev::PlayMode;
    let n = def.waves.len();
    if n == 0 {
        return None;
    }
    let entry = state.seen.entry(key).or_insert_with(|| (0, vec![false; n]));
    if entry.1.len() != n {
        *entry = (0, vec![false; n]);
    }
    let weighted = |rng: &mut Rng, allowed: &dyn Fn(usize) -> bool| -> usize {
        let total: u32 = def.waves.iter().enumerate().filter(|(i, _)| allowed(*i)).map(|(_, w)| w.weight().max(1)).sum();
        if total == 0 {
            return 0;
        }
        let mut r = (rng.unit() * total as f32) as u32;
        for (i, w) in def.waves.iter().enumerate().filter(|(i, _)| allowed(*i)) {
            let w = w.weight().max(1);
            if r < w {
                return i;
            }
            r -= w;
        }
        0
    };
    let i = match mode {
        PlayMode::Sequential => {
            let i = entry.0 % n;
            entry.0 += 1;
            i
        }
        PlayMode::Random => weighted(rng, &|_| true),
        PlayMode::Shuffle => {
            if entry.1.iter().all(|&p| p) {
                // A new round: do not repeat the last one first.
                let last = entry.0 % n;
                entry.1.iter_mut().for_each(|p| *p = false);
                if n > 1 {
                    entry.1[last] = true;
                }
            }
            let played = entry.1.clone();
            let i = weighted(rng, &|i| !played[i]);
            entry.1[i] = true;
            entry.0 = i;
            i
        }
        PlayMode::Programmer => 0,
    };
    def.waves.get(i)
}

/// Linear gain of the log, linear and linear squared rolloffs at `distance`.
pub fn rolloff_gain(kind: Rolloff, min: f32, max: f32, distance: f32) -> f32 {
    let min = min.max(0.01);
    let max = max.max(min + 0.01);
    if distance <= min {
        return 1.0;
    }
    let base = match kind {
        Rolloff::Linear => 1.0 - (distance - min) / (max - min),
        Rolloff::LinearSquare => (1.0 - (distance - min) / (max - min)).powi(2),
        Rolloff::Log | Rolloff::Custom => min / distance,
    };
    // Past the maximum distance FMOD events are silent; fade the last tenth so it does not click.
    let edge = ((max - distance) / (0.1 * max)).clamp(0.0, 1.0);
    base.clamp(0.0, 1.0) * edge
}

/// Parameters the player sets from the listener distance of a 3D event.
fn is_distance(name: &str) -> bool {
    name.eq_ignore_ascii_case("distance") || name.eq_ignore_ascii_case("camera_distance") || name.eq_ignore_ascii_case("(distance)")
}

/// Indices of the parameters that follow the listener distance.
pub fn distance_parameters(ev: &Event) -> Vec<usize> {
    match &ev.body {
        Body::Complex { parameters, .. } => parameters.iter().enumerate().filter(|(_, p)| is_distance(&p.name)).map(|(i, _)| i).collect(),
        Body::Simple(_) => Vec::new(),
    }
}

/// Values for the parameters of `ev`: the minimum of each, the listener `distance` for the distance
/// ones, then the named `user` values (case-insensitive, clamped to the range).
pub fn initial_values(ev: &Event, distance: Option<f32>, user: &[(String, f32)]) -> Vec<f32> {
    let Body::Complex { parameters, .. } = &ev.body else { return Vec::new() };
    let mut values: Vec<f32> = parameters
        .iter()
        .map(|p| match (is_distance(&p.name), distance) {
            (true, Some(d)) => d.clamp(p.min.min(p.max), p.max.max(p.min)),
            _ => p.min,
        })
        .collect();
    for (name, v) in user {
        set_value(ev, &mut values, name, *v);
    }
    values
}

/// Sets the parameter called `name`; `false` when the event has none.
pub fn set_value(ev: &Event, values: &mut [f32], name: &str, v: f32) -> bool {
    let Body::Complex { parameters, .. } = &ev.body else { return false };
    let mut found = false;
    for (p, slot) in parameters.iter().zip(values.iter_mut()) {
        if p.name.eq_ignore_ascii_case(name) {
            *slot = v.clamp(p.min.min(p.max), p.max.max(p.min));
            found = true;
        }
    }
    found
}

/// Position of parameter `i` in its range, 0 to 1.
pub(crate) fn normalized(ev: &Event, values: &[f32], i: usize) -> f32 {
    let Body::Complex { parameters, .. } = &ev.body else { return 0.0 };
    match (parameters.get(i), values.get(i)) {
        (Some(p), Some(v)) if p.max > p.min => ((v - p.min) / (p.max - p.min)).clamp(0.0, 1.0),
        _ => 0.0,
    }
}

/// The `(layer, instance)` pairs that play at these parameter values: those whose range, along the
/// layer's control parameter, holds the current position.
pub fn active_instances(ev: &Event, values: &[f32]) -> Vec<(usize, usize)> {
    match &ev.body {
        Body::Simple(_) => vec![(0, 0)],
        Body::Complex { layers, parameters, .. } => {
            let mut out = Vec::new();
            for (li, l) in layers.iter().enumerate() {
                let control = (l.control_parameter as usize).min(parameters.len().saturating_sub(1));
                let at = normalized(ev, values, control);
                for (ii, inst) in l.instances.iter().enumerate() {
                    // the range includes its end only at the top of the parameter
                    let end = inst.start + inst.length;
                    if at >= inst.start - 1e-4 && (at < end || (at >= 1.0 - 1e-4 && end >= 1.0 - 1e-4)) {
                        out.push((li, ii));
                    }
                }
            }
            out
        }
    }
}

/// Product of the volume curves of a layer at these parameter values.
pub fn layer_gain(ev: &Event, values: &[f32], layer: usize) -> f32 {
    let Body::Complex { layers, .. } = &ev.body else { return 1.0 };
    let Some(l) = layers.get(layer) else { return 1.0 };
    l.envelopes
        .iter()
        .filter(|e| e.flags & 0x8 != 0 && !e.points.is_empty())
        .map(|e| e.at(normalized(ev, values, e.parameter as usize)).clamp(0.0, 1.0))
        .product()
}

/// Pitch factor of the layer's pitch curves (flag 0x10) at these parameter values: a curve value `y` of 0 to 1 is 2^(8y - 4), so
/// 0.5 leaves the pitch alone and the ends are four octaves down and up (`EventLayer` in `fmod_eventl.dll`).
pub fn layer_pitch(ev: &Event, values: &[f32], layer: usize) -> f32 {
    let Body::Complex { layers, .. } = &ev.body else { return 1.0 };
    let Some(l) = layers.get(layer) else { return 1.0 };
    l.envelopes.iter().filter(|e| e.flags & 0x10 != 0 && !e.points.is_empty()).map(|e| (e.at(normalized(ev, values, e.parameter as usize)).clamp(0.0, 1.0) * 8.0 - 4.0).exp2()).product()
}

/// Native sub_1001CB90 multiplies the event's pan level by the first 0x400
/// envelope. A disabled first match falls back to the event, without scanning on.
/// Kira accepts spatialization strength in 0..1; zero must remain undirectional.
pub(crate) fn layer_pan(ev: &Event, values: &[f32], layer: usize) -> f32 {
    let mut pan = ev.pan_level;
    if let Body::Complex { layers, .. } = &ev.body
        && let Some(env) = layers.get(layer).and_then(|l| l.envelopes.iter().find(|e| e.flags & 0x400 != 0))
        && env.flags & 1 == 0
    {
        // The native evaluator returns zero for an empty curve (the pak has none).
        pan *= if env.points.is_empty() { 0.0 } else { env.at(normalized(ev, values, env.parameter as usize)) };
    }
    pan.clamp(0.0, 1.0)
}

/// Cutoffs at or above this (Hz) are not worth a filter.
pub const OPEN_CUTOFF: f32 = 18000.0;

/// The cutoff in Hz of a low-pass DSP curve value `y` (0 to 1): the project's DSP parameter is 10 to 22000 Hz on a log scale
/// (`EventLayer::updateEnvelope` in `fmod_eventl.dll` maps the curve through `exp2(lerp(log2 min, log2 max, y))` for the filter DSPs).
pub fn dsp_cutoff(y: f32) -> f32 {
    10.0 * (22000.0f32 / 10.0).powf(y.clamp(0.0, 1.0))
}

/// The cutoff in Hz of FMOD's direct-path occlusion `occ` (0 clear, 1 shut). FMOD 4 does not document how the occlusion
/// filters; this is a smooth guess that leaves 0.3 (the largest value the project's distance curves reach) at 11 kHz.
pub fn occlusion_cutoff(occ: f32) -> f32 {
    22000.0 * (1.0 - occ.clamp(0.0, 1.0)).powi(2)
}

fn lowpass_curve(e: &Envelope) -> Option<fn(f32) -> f32> {
    if e.flags & 0x2000 != 0 {
        // occlusion: 0 is the direct path, 1 the reverb path (not modelled)
        (e.dsp_param == 0).then_some(occlusion_cutoff as fn(f32) -> f32)
    } else if e.flags & 0x4 != 0 && e.dsp.starts_with("FMOD Lowpass") && e.dsp_param == 0 {
        Some(dsp_cutoff)
    } else {
        None
    }
}

/// Whether the layer has a low-pass curve that ever closes the filter (so a sound of the layer needs one).
pub fn layer_filtered(ev: &Event, layer: usize) -> bool {
    let Body::Complex { layers, .. } = &ev.body else { return false };
    let Some(l) = layers.get(layer) else { return false };
    l.envelopes.iter().any(|e| lowpass_curve(e).is_some_and(|f| e.points.iter().any(|p| f(p.y) < OPEN_CUTOFF)))
}

/// The cutoff the layer's low-pass curves (the DSP filters and the distance occlusion) ask for at these parameter values,
/// `None` when open.
pub fn layer_cutoff(ev: &Event, values: &[f32], layer: usize) -> Option<f32> {
    let Body::Complex { layers, .. } = &ev.body else { return None };
    let l = layers.get(layer)?;
    l.envelopes
        .iter()
        .filter(|e| !e.points.is_empty())
        .filter_map(|e| lowpass_curve(e).map(|f| f(e.at(normalized(ev, values, e.parameter as usize)))))
        .fold(None, |m: Option<f32>, c| Some(m.map_or(c, |m| m.min(c))))
        .filter(|&c| c < OPEN_CUTOFF)
}

/// Speed of sound (m/s) in FMOD's doppler model.
pub const SPEED_OF_SOUND: f32 = 340.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Highpass {
    pub cutoff: f32,
    pub enabled: bool,
}

/// The proven original topology: one log-mapped cutoff root and its default-Q
/// (constant-zero) resonance child. Other mapping/topology variants stay unsupported.
pub(crate) fn layer_highpass(event: &EventRef, values: &[f32], layer: usize) -> Option<Highpass> {
    let ev = event.event();
    let Body::Complex { layers, .. } = &ev.body else { return None };
    let envelopes = &layers.get(layer)?.envelopes;
    let mut roots = envelopes.iter().enumerate().filter(|(_, e)| e.dsp == "FMOD Highpass");
    let (ei, root) = roots.next()?;
    if roots.next().is_some() || root.parent != -1 || root.dsp_param != 0 || root.flags & !3 != 4
        || root.points.iter().any(|p| !p.y.is_finite() || !(0.0..=1.0).contains(&p.y))
        || event.project.fev.highpass_mapping(event.index, layer, ei) != Some(1)
    {
        return None;
    }
    let mut children = envelopes.iter().filter(|e| e.parent == ei as i32);
    if let Some(child) = children.next()
        && (children.next().is_some() || !child.dsp.is_empty() || child.dsp_param != 1
            || child.flags & 5 != 4 || child.points.is_empty() || child.points.iter().any(|p| p.y != 0.0))
    {
        return None;
    }
    let y = if root.points.is_empty() { 0.0 } else { root.at(normalized(ev, values, root.parameter as usize)) };
    Some(Highpass { cutoff: dsp_cutoff(y), enabled: root.flags & 1 == 0 })
}

/// The playback rate factor of the doppler effect: the source moves with velocity `source`, the listener with `listener`,
/// `to_source` is the vector from the listener to the source, `level` the event's doppler scale (0 off).
pub fn doppler_factor(level: f32, to_source: Vec3, source: Vec3, listener: Vec3) -> f32 {
    let d = to_source.length();
    if level <= 0.0 || d < 0.5 {
        return 1.0;
    }
    let u = to_source / d;
    // along `u`: a source moving away and a listener moving towards the source
    let (vs, vl) = (source.dot(u) * level, listener.dot(u) * level);
    ((SPEED_OF_SOUND + vl) / (SPEED_OF_SOUND + vs).max(SPEED_OF_SOUND * 0.2)).clamp(0.5, 2.0)
}

/// `true` when the event has a volume curve on a distance parameter (a custom rolloff).
pub fn has_distance_curve(ev: &Event) -> bool {
    let dist = distance_parameters(ev);
    match &ev.body {
        Body::Complex { layers, .. } => layers.iter().flat_map(|l| l.envelopes.iter()).any(|e| e.flags & 0x8 != 0 && !e.points.is_empty() && dist.contains(&(e.parameter as usize))),
        Body::Simple(_) => false,
    }
}

/// Loudness of a 3D event at `distance` from the listener, from its rolloff alone: the project's own
/// curve for a custom rolloff (see [`layer_gain`]; a custom rolloff without a curve falls back to the
/// logarithmic one).
pub fn distance_gain(ev: &Event, distance: f32) -> f32 {
    if ev.rolloff() == Rolloff::Custom && has_distance_curve(ev) { 1.0 } else { rolloff_gain(ev.rolloff(), ev.min_distance, ev.max_distance, distance) }
}

/// Picks the wave for instance `instance` of `layer` and applies the random variation: what starting
/// that sound plays. `None` for silent and unplayable definitions.
pub fn start_part(event: &EventRef, layer: usize, instance: usize, state: &mut PickState, rng: &mut Rng) -> Option<Planned> {
    let project = &event.project;
    let fev = &project.fev;
    let ev = event.event();
    let inst = match &ev.body {
        Body::Simple(i) => i,
        Body::Complex { layers, .. } => layers.get(layer)?.instances.get(instance)?,
    };
    let def = fev.sound_defs.get(inst.sound_def)?;
    let template = fev.templates.get(def.template)?;
    let key = (Arc::as_ptr(project) as usize, inst.sound_def);
    let Wave::Bank { name, bank, index, length_ms, .. } = pick_wave(state, rng, key, def, template.play_mode())? else { return None };
    let vol_random = 1.0 - ev.volume_random * rng.unit();
    let def_random = template.volume_random_min + (1.0 - template.volume_random_min) * rng.unit();
    let octaves = (ev.pitch + template.pitch) * 4.0 + ev.pitch_random * 4.0 * rng.signed() + template.pitch_random * 4.0 * rng.signed();
    Some(Planned {
        layer,
        instance,
        bank: *bank,
        index: *index,
        name: name.clone(),
        gain: ev.volume * template.volume * vol_random * def_random,
        rate: octaves.exp2(),
        looping: inst.loops(),
        playlist: def.waves.len() > 1,
        length_ms: *length_ms,
        fade_in_ms: ev.fade_in_ms.max(template.fade_in_ms as u32),
        fade_out_ms: ev.fade_out_ms.max(template.fade_out_ms as u32),
        spawn_ms: (template.spawn_time_max_ms > 0).then_some((template.spawn_time_min_ms, template.spawn_time_max_ms)),
        max_spawned: template.max_spawned.max(1),
    })
}

/// What one trigger of `event` plays at these parameter values (see [`initial_values`]).
pub fn plan(event: &EventRef, values: &[f32], state: &mut PickState, rng: &mut Rng) -> Vec<Planned> {
    active_instances(event.event(), values).into_iter().filter_map(|(l, i)| start_part(event, l, i, state, rng)).collect()
}

/// Points of an envelope as `(x, y)` pairs (for tests and the docs).
pub fn points(e: &Envelope) -> Vec<(f32, f32)> {
    e.points.iter().map(|EnvPoint { x, y, .. }| (*x, *y)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> (String, String, String) {
        let e = parse_path(s).unwrap_or_else(|| panic!("{s}"));
        (e.project, e.group, e.event)
    }

    #[test]
    fn parses_every_path_form() {
        let want = |a: &str, b: &str, c: &str| (a.to_string(), b.to_string(), c.to_string());
        assert_eq!(p("sounds/x2skills:wild:fire_arrow_form"), want("x2skills", "wild", "fire_arrow_form"));
        assert_eq!(p("Sounds/X2Monster_Footstep:move:Dirt_Lots_Mid"), want("x2monster_footstep", "move", "dirt_lots_mid"));
        assert_eq!(p("sounds/x2weapons:x2weapons:shield_flesh"), want("x2weapons", "x2weapons", "shield_flesh"));
        assert_eq!(p("sounds/x2skills/monster_skill:pino_effect.mon.mon_heal_hit"), want("x2skills", "monster_skill", "pino_effect.mon.mon_heal_hit"));
        assert_eq!(p("x2skills/wild/fire_arrow_form"), want("x2skills", "wild", "fire_arrow_form"));
        assert_eq!(p("sounds\\x2ui:a:b"), want("x2ui", "a", "b"));
        assert_eq!(p("sounds/x2a:g1:g2:ev"), want("x2a", "g1/g2", "ev"));
        assert_eq!(p("sounds/x2a:ev"), want("x2a", "", "ev"));
        assert!(parse_path("").is_none());
        assert!(parse_path("sounds/").is_none());
        assert!(parse_path("x2skills").is_none());
    }

    #[test]
    fn rolloffs() {
        assert_eq!(rolloff_gain(Rolloff::Log, 2.0, 50.0, 1.0), 1.0);
        assert!((rolloff_gain(Rolloff::Log, 2.0, 50.0, 10.0) - 0.2).abs() < 1e-6);
        assert_eq!(rolloff_gain(Rolloff::Log, 2.0, 50.0, 60.0), 0.0);
        assert!((rolloff_gain(Rolloff::Linear, 0.0, 100.0, 50.0) - 0.5).abs() < 0.02);
        assert!(rolloff_gain(Rolloff::LinearSquare, 0.0, 100.0, 50.0) < 0.3);
    }

    #[test]
    fn dsp_cutoffs_are_on_a_log_scale() {
        assert!((dsp_cutoff(0.0) - 10.0).abs() < 1e-3);
        assert!((dsp_cutoff(1.0) - 22000.0).abs() < 1.0);
        // 0.8 is 4.7 kHz, the lowest curve value in the ambience beds 0.5 is 470 Hz
        assert!((dsp_cutoff(0.8) - 4720.0).abs() < 30.0, "{}", dsp_cutoff(0.8));
        assert!((dsp_cutoff(0.5) - 10.0 * 2200f32.sqrt()).abs() < 1.0);
    }

    #[test]
    fn occlusion_closes_the_filter() {
        assert_eq!(occlusion_cutoff(0.0), 22000.0);
        assert!((occlusion_cutoff(0.3) - 10780.0).abs() < 10.0);
        assert!(occlusion_cutoff(1.0) < 1.0);
    }

    #[test]
    fn doppler_shifts_by_the_closing_speed() {
        let ahead = Vec3::new(0.0, 0.0, 50.0);
        // a source coming towards the listener at 34 m/s (a tenth of the speed of sound): about 11 % up
        let up = doppler_factor(1.0, ahead, Vec3::new(0.0, 0.0, -34.0), Vec3::ZERO);
        assert!((up - 340.0 / 306.0).abs() < 1e-4, "{up}");
        // going away: down; the same by the listener moving towards it
        let down = doppler_factor(1.0, ahead, Vec3::new(0.0, 0.0, 34.0), Vec3::ZERO);
        assert!((down - 340.0 / 374.0).abs() < 1e-4, "{down}");
        let walker = doppler_factor(1.0, ahead, Vec3::ZERO, Vec3::new(0.0, 0.0, 34.0));
        assert!((walker - 374.0 / 340.0).abs() < 1e-4, "{walker}");
        // sideways motion, no doppler level, and a source on top of the listener do nothing
        assert!((doppler_factor(1.0, ahead, Vec3::new(30.0, 0.0, 0.0), Vec3::ZERO) - 1.0).abs() < 1e-6);
        assert_eq!(doppler_factor(0.0, ahead, Vec3::new(0.0, 0.0, -34.0), Vec3::ZERO), 1.0);
        assert_eq!(doppler_factor(1.0, Vec3::new(0.0, 0.0, 0.1), Vec3::new(0.0, 0.0, -34.0), Vec3::ZERO), 1.0);
        // the level scales the velocities: 2 doubles the shift; a bullet cannot pass the clamp
        let double = doppler_factor(2.0, ahead, Vec3::new(0.0, 0.0, -34.0), Vec3::ZERO);
        assert!((double - 340.0 / 272.0).abs() < 1e-4, "{double}");
        assert_eq!(doppler_factor(5.0, ahead, Vec3::new(0.0, 0.0, -600.0), Vec3::ZERO), 2.0);
    }

    #[test]
    fn layer_pan_matches_original_instruction_goldens() {
        use crate::fev::{Layer, Parameter};
        let mut ev = Event {
            name: String::new(), group: String::new(), flags: 0, guid: [0; 16], volume: 1.0,
            pitch: 0.0, pitch_random: 0.0, volume_random: 0.0, priority: 128, max_playbacks: 1,
            playback_behaviour: 4, mode: 0x10, min_distance: 0.0, max_distance: 35.0, flags2: 0,
            speaker_levels: [1.0; 8], cone_inside: 360.0, cone_outside: 360.0, cone_outside_volume: 1.0,
            doppler: 0.0, reverb_dry: 0.0, reverb_wet: 0.0, speaker_spread: 0.0, pan_level: 1.0,
            fade_in_ms: 0, fade_out_ms: 0, spawn_intensity: 0.0, spawn_intensity_random: 0.0,
            raw: [0; 4], category: String::new(), body: Body::Complex { layers: Vec::new(), parameters: Vec::new(), props: Vec::new() },
        };
        let mut cases = 0;
        for row in include_str!("../tests/native_pan_cases.tsv").lines().filter(|r| !r.starts_with('#')) {
            let fields: Vec<_> = row.split('\t').collect();
            ev.pan_level = fields[1].parse().unwrap();
            let x: f32 = fields[2].parse().unwrap();
            let mut envelopes = Vec::new();
            for encoded in fields[3].split(';').filter(|s| !s.is_empty()) {
                let (flags, points) = encoded.split_once('@').unwrap();
                let points = points.split(',').filter(|s| !s.is_empty()).map(|p| {
                    let p: Vec<_> = p.split(':').collect();
                    EnvPoint { x: p[0].parse().unwrap(), y: p[1].parse().unwrap(), curve: p[2].parse().unwrap() }
                }).collect();
                envelopes.push(Envelope { parent: -1, dsp: String::new(), dsp_param: 0, flags: flags.parse().unwrap(), flags2: 0, parameter: 0, points });
            }
            ev.body = Body::Complex {
                layers: vec![Layer { flags: 0, priority: 0, control_parameter: 0, instances: Vec::new(), envelopes }],
                // Use a non-normalized range to check parameter normalization too.
                parameters: vec![Parameter { name: "distance".into(), min: 10.0, max: 45.0, velocity: 0.0,
                    flags: 0, seek_speed: 0.0, raw_44: 0, sustain_points: Vec::new() }], props: Vec::new(),
            };
            let expected: f32 = fields[4].parse().unwrap();
            assert!((layer_pan(&ev, &[10.0 + 35.0 * x], 0) - expected).abs() < 1e-6, "{}", fields[0]);
            assert_eq!(layer_pan(&ev, &[], 99), ev.pan_level);
            cases += 1;
        }
        assert_eq!(cases, 11);
    }

    #[test]
    fn rng_is_spread() {
        let mut r = Rng::new(7);
        let v: Vec<f32> = (0..1000).map(|_| r.unit()).collect();
        assert!(v.iter().all(|x| (0.0..1.0).contains(x)));
        let mean = v.iter().sum::<f32>() / 1000.0;
        assert!((0.4..0.6).contains(&mean), "{mean}");
    }
}
