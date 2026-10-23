//! Renders a MIDI file with a DLS bank to mono PCM: a small wavetable synthesizer.
//!
//! Renders region samples with root note, tuning, loops and gain, approximate
//! volume/pitch envelopes and LFO vibrato/tremolo. This is a subset of DLS synthesis,
//! not a reproduction of the original FMOD renderer. Channel 10 plays the
//! drum kit, the others the program set by the file; channel volume (7), expression (11) and the pitch bend are
//! followed. Pan is not: the result is a single source that the game places in the world.

use crate::dls::{Articulation, Dls, Instrument};
use crate::smf::{Kind, Song};

/// The rate of the rendered audio: the bank's samples are 22.05 kHz and 24 kHz.
pub const RATE: u32 = 24_000;
/// Offline render duration limit, including the requested release tail.
pub const MAX_SECONDS: f32 = 240.0;

/// Samples between recalculations of pitch and envelope-driven values.
const CONTROL: usize = 32;
/// What every voice is multiplied by before the sum is limited.
const MASTER: f32 = 0.42;
/// Voices at once; the oldest is dropped for a new one.
const MAX_VOICES: usize = 48;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("song plus tail exceeds the 240 second render limit")]
    TooLong,
    #[error("invalid render input: {0}")]
    Invalid(&'static str),
}

#[derive(Clone, Copy, PartialEq)]
enum Stage {
    Attack,
    Decay,
    Sustain,
    Release,
}

/// An envelope with the DLS shape: a linear attack, a decay towards the sustain level, a release to silence.
#[derive(Clone, Copy)]
struct Envelope {
    stage: Stage,
    level: f32,
    attack_step: f32,
    decay_coef: f32,
    sustain: f32,
    release_coef: f32,
}

impl Envelope {
    fn new(attack: f32, decay: f32, sustain: f32, release: f32) -> Envelope {
        let rate = RATE as f32;
        // 60 dB over the release time, and over the decay time from the peak down to the sustain level
        let release = release.max(0.004);
        let coef = |secs: f32, span: f32| -> f32 {
            if secs <= 0.0 {
                0.0
            } else {
                (-(1000f32.ln()) * span / (secs * rate)).exp()
            }
        };
        Envelope {
            stage: if attack > 0.0 {
                Stage::Attack
            } else {
                Stage::Decay
            },
            level: if attack > 0.0 { 0.0 } else { 1.0 },
            attack_step: if attack > 0.0 {
                1.0 / (attack * rate)
            } else {
                1.0
            },
            // time-constant so that the level falls from 1 to `sustain` in `decay` seconds
            decay_coef: if decay <= 0.0 {
                0.0
            } else {
                (sustain.max(0.001).ln() / (decay * rate)).exp()
            },
            sustain,
            release_coef: coef(release, 1.0),
        }
    }

    fn release(&mut self) {
        self.stage = Stage::Release;
    }

    /// The next level.
    fn next(&mut self) -> f32 {
        match self.stage {
            Stage::Attack => {
                self.level += self.attack_step;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => {
                self.level *= self.decay_coef;
                if self.level <= self.sustain || self.decay_coef == 0.0 {
                    self.level = self.sustain;
                    self.stage = Stage::Sustain;
                }
            }
            Stage::Sustain => {}
            Stage::Release => self.level *= self.release_coef,
        }
        self.level
    }

    fn done(&self) -> bool {
        self.stage == Stage::Release && self.level < 1e-4
    }
}

struct Voice {
    channel: u8,
    note: u8,
    group: u16,
    wave: usize,
    pos: f64,
    step: f64,
    base_cents: f32,
    gain: f32,
    looped: Option<(f64, f64)>,
    art: Articulation,
    volume: Envelope,
    pitch: Envelope,
    age: usize,
    held: bool,
    ended: bool,
    control_gain: f32,
}

#[derive(Clone, Copy)]
struct Channel {
    program: u8,
    volume: f32,
    expression: f32,
    bend_cents: f32,
    sustain: bool,
}

impl Default for Channel {
    fn default() -> Self {
        Channel {
            program: 0,
            volume: 100.0 / 127.0,
            expression: 1.0,
            bend_cents: 0.0,
            sustain: false,
        }
    }
}

fn instrument_for(dls: &Dls, channel: u8, program: u8) -> Option<&Instrument> {
    dls.instrument(program, channel == 9)
}

/// Starts the voices a note asks for.
#[allow(clippy::too_many_arguments)]
fn note_on(dls: &Dls, voices: &mut Vec<Voice>, channel: u8, ch: &Channel, note: u8, velocity: u8) {
    let Some(inst) = instrument_for(dls, channel, ch.program) else {
        return;
    };
    for region in &inst.regions {
        if note < region.key.0
            || note > region.key.1
            || velocity < region.velocity.0
            || velocity > region.velocity.1
        {
            continue;
        }
        let wave = &dls.waves[region.wave];
        if wave.pcm.is_empty() {
            continue;
        }
        let art = region.articulation.unwrap_or(inst.articulation);
        let v = f32::from(velocity) / 127.0;
        // velocity lengthens or shortens the attack, the key the decay (timecents)
        let attack = (art.attack * 2f32.powf(art.velocity_to_attack * v / 1200.0)).max(0.0);
        let decay =
            (art.decay * 2f32.powf(art.key_to_decay * f32::from(note) / 127.0 / 1200.0)).max(0.0);
        let base_cents =
            (f32::from(note) - f32::from(region.unity)) * 100.0 + f32::from(region.fine_tune);
        if region.key_group != 0 {
            for other in voices
                .iter_mut()
                .filter(|o| o.channel == channel && o.group == region.key_group)
            {
                other.volume.release();
                other.pitch.release();
            }
        }
        if voices.len() >= MAX_VOICES
            && let Some(oldest) = voices
                .iter()
                .enumerate()
                .max_by_key(|(_, v)| v.age)
                .map(|(i, _)| i)
        {
            voices.swap_remove(oldest);
        }
        voices.push(Voice {
            channel,
            note,
            group: region.key_group,
            wave: region.wave,
            pos: 0.0,
            step: f64::from(wave.rate) / f64::from(RATE),
            base_cents,
            // Approximate velocity response; not the full DLS connection graph.
            gain: v * v * 10f32.powf(region.gain_db.clamp(-96.0, 24.0) / 20.0) * MASTER,
            looped: region.looped.map(|(s, l)| (f64::from(s), f64::from(l))),
            art,
            volume: Envelope::new(attack, decay, art.sustain, art.release),
            pitch: Envelope::new(
                art.pitch_attack,
                art.pitch_decay,
                art.pitch_sustain,
                art.pitch_release,
            ),
            age: 0,
            held: true,
            ended: false,
            control_gain: 1.0,
        });
    }
}

/// Renders the song to mono samples in -1..1 at [`RATE`]; `tail` seconds are kept after the last event for the
/// releases to die away. Long songs return an error instead of silently cutting
/// off. Tail must be finite and nonnegative; events must already be in time order.
pub fn render(dls: &Dls, song: &Song, tail: f32) -> Result<Vec<f32>, Error> {
    if !tail.is_finite() || tail < 0.0 {
        return Err(Error::Invalid("tail"));
    }
    if song.events.len() > crate::smf::MAX_EVENTS {
        return Err(Error::Invalid("event limit"));
    }
    let mut previous = 0;
    for e in &song.events {
        let valid = match e.kind {
            Kind::NoteOn { note, velocity } => note < 128 && velocity < 128,
            Kind::NoteOff { note } => note < 128,
            Kind::ProgramChange { program } => program < 128,
            Kind::Controller { number, value } => number < 128 && value < 128,
            Kind::PitchBend { value } => value < 16384,
        };
        if e.channel > 15 || e.ms < previous || !valid {
            return Err(Error::Invalid("events"));
        }
        previous = e.ms;
    }
    for wave in &dls.waves {
        if wave.rate == 0 || wave.pcm.is_empty() {
            return Err(Error::Invalid("wave"));
        }
    }
    for instrument in &dls.instruments {
        if !valid_art(instrument.articulation) {
            return Err(Error::Invalid("articulation"));
        }
        for r in &instrument.regions {
            let Some(wave) = dls.waves.get(r.wave) else {
                return Err(Error::Invalid("wave reference"));
            };
            if !r.gain_db.is_finite() || r.articulation.is_some_and(|a| !valid_art(a)) {
                return Err(Error::Invalid("region"));
            }
            if let Some((s, l)) = r.looped
                && (l == 0 || u64::from(s) + u64::from(l) > wave.pcm.len() as u64)
            {
                return Err(Error::Invalid("loop"));
            }
        }
    }
    let end_seconds = f64::from(song.length_ms()) / 1000.0 + f64::from(tail);
    if end_seconds > f64::from(MAX_SECONDS) {
        return Err(Error::TooLong);
    }
    let total = (end_seconds * f64::from(RATE)).ceil() as usize;
    let mut out = vec![0f32; total];
    let mut channels = [Channel::default(); 16];
    let mut voices: Vec<Voice> = Vec::new();
    let mut events = song.events.iter().peekable();
    let lfo_step = std::f64::consts::TAU / f64::from(RATE);
    for (n, sample) in out.iter_mut().enumerate() {
        let now_ms = (n as f64 * 1000.0 / f64::from(RATE)) as u32;
        while let Some(e) = events.peek().filter(|e| e.ms <= now_ms) {
            let ch = &mut channels[usize::from(e.channel)];
            match e.kind {
                Kind::NoteOn { note, velocity } => {
                    let ch = *ch;
                    if velocity == 0 {
                        note_off(&mut voices, e.channel, note, ch.sustain);
                    } else {
                        note_on(dls, &mut voices, e.channel, &ch, note, velocity);
                    }
                }
                Kind::NoteOff { note } => {
                    note_off(&mut voices, e.channel, note, ch.sustain);
                }
                Kind::ProgramChange { program } => ch.program = program,
                Kind::Controller { number, value } => match number {
                    7 => ch.volume = f32::from(value) / 127.0,
                    11 => ch.expression = f32::from(value) / 127.0,
                    64 => {
                        ch.sustain = value >= 64;
                        if !ch.sustain {
                            release_unheld(&mut voices, e.channel);
                        }
                    }
                    120 => voices.retain(|v| v.channel != e.channel),
                    121 => {
                        *ch = Channel {
                            program: ch.program,
                            volume: ch.volume,
                            ..Default::default()
                        };
                        release_unheld(&mut voices, e.channel);
                    }
                    123 => {
                        for v in voices.iter_mut().filter(|v| v.channel == e.channel) {
                            v.held = false;
                        }
                        if !ch.sustain {
                            release_unheld(&mut voices, e.channel);
                        }
                    }
                    _ => {}
                },
                // +-2 semitones
                Kind::PitchBend { value } => {
                    ch.bend_cents = (f32::from(value) - 8192.0) / 8192.0 * 200.0
                }
            }
            events.next();
        }
        let mut sum = 0f32;
        for v in voices.iter_mut() {
            let wave = &dls.waves[v.wave];
            if !v.pos.is_finite() || v.pos >= wave.pcm.len() as f64 {
                v.ended = true;
                continue;
            }
            if v.age % CONTROL == 0 {
                let t = v.age as f32 / RATE as f32;
                let lfo = if t >= v.art.lfo_delay {
                    (lfo_step * f64::from(v.art.lfo_hz) * v.age as f64).sin() as f32
                } else {
                    0.0
                };
                let ch = &channels[usize::from(v.channel & 15)];
                let cents = v.base_cents
                    + ch.bend_cents
                    + v.art.lfo_pitch_cents * lfo
                    + v.art.pitch_cents * v.pitch.level;
                v.step =
                    f64::from(wave.rate) / f64::from(RATE) * f64::from(2f32.powf(cents / 1200.0));
                v.control_gain = 10f32.powf((v.art.lfo_gain_db * lfo).clamp(-96.0, 24.0) / 20.0)
                    * ch.volume
                    * ch.expression;
                if !v.step.is_finite() {
                    v.ended = true;
                    continue;
                }
            }
            let i = v.pos as usize;
            let frac = (v.pos - i as f64) as f32;
            let next_index = match v.looped {
                Some((s, l)) if (i + 1) as f64 >= s + l => s as usize,
                _ => i + 1,
            };
            let (Some(&a), b) = (wave.pcm.get(i), wave.pcm.get(next_index).copied()) else {
                v.ended = true;
                continue;
            };
            let a = f32::from(a);
            let s = (a + (f32::from(b.unwrap_or(0)) - a) * frac) / 32768.0;
            let env = v.volume.next();
            v.pitch.next();
            sum += s * v.gain * env * v.control_gain;
            v.pos += v.step;
            if let Some((s, l)) = v.looped
                && v.pos >= s + l
            {
                v.pos = s + (v.pos - s).rem_euclid(l);
            }
            v.age += 1;
            if v.volume.done() {
                v.ended = true;
            }
        }
        voices.retain(|v| !v.ended);
        // limit smoothly instead of clipping
        *sample = (sum * 1.1).tanh() / 1.1;
    }
    Ok(out)
}

fn note_off(voices: &mut [Voice], channel: u8, note: u8, sustain: bool) {
    for v in voices
        .iter_mut()
        .filter(|v| v.channel == channel && v.note == note && v.held)
    {
        v.held = false;
        if !sustain {
            v.volume.release();
            v.pitch.release();
        }
    }
}

fn release_unheld(voices: &mut [Voice], channel: u8) {
    for v in voices
        .iter_mut()
        .filter(|v| v.channel == channel && !v.held)
    {
        v.volume.release();
        v.pitch.release();
    }
}

fn valid_art(a: Articulation) -> bool {
    [
        a.attack,
        a.decay,
        a.sustain,
        a.release,
        a.velocity_to_attack,
        a.key_to_decay,
        a.pitch_attack,
        a.pitch_decay,
        a.pitch_sustain,
        a.pitch_release,
        a.pitch_cents,
        a.lfo_hz,
        a.lfo_delay,
        a.lfo_pitch_cents,
        a.lfo_gain_db,
        a.pan,
    ]
    .iter()
    .all(|v| v.is_finite())
        && [
            a.attack,
            a.decay,
            a.release,
            a.pitch_attack,
            a.pitch_decay,
            a.pitch_release,
            a.lfo_hz,
            a.lfo_delay,
        ]
        .iter()
        .all(|v| *v >= 0.0)
        && (0.0..=1.0).contains(&a.sustain)
        && (0.0..=1.0).contains(&a.pitch_sustain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{mml, smf};

    fn constant_bank() -> Dls {
        use crate::dls::{Instrument, Region, Wave};
        Dls {
            waves: vec![Wave {
                rate: RATE,
                pcm: vec![12000; 4].into(),
                unity: 60,
                fine_tune: 0,
                gain_db: 0.0,
                looped: Some((1, 2)),
            }],
            instruments: vec![Instrument {
                program: 0,
                bank: 0,
                drum: false,
                articulation: Articulation::default(),
                regions: vec![Region {
                    key: (0, 127),
                    velocity: (0, 127),
                    wave: 0,
                    unity: 60,
                    fine_tune: 0,
                    gain_db: 0.0,
                    looped: Some((1, 2)),
                    key_group: 0,
                    articulation: None,
                }],
            }],
        }
    }

    fn song(events: &[(u32, u8, Kind)], duration_ms: u32) -> Song {
        Song {
            tracks: 1,
            events: events
                .iter()
                .map(|&(ms, channel, kind)| smf::Event { ms, channel, kind })
                .collect(),
            duration_ms,
        }
    }

    #[test]
    fn simultaneous_voices_keep_their_own_channel_control() {
        let bank = constant_bank();
        let song = song(
            &[
                (
                    0,
                    1,
                    Kind::Controller {
                        number: 7,
                        value: 0,
                    },
                ),
                (
                    0,
                    0,
                    Kind::NoteOn {
                        note: 60,
                        velocity: 127,
                    },
                ),
                (
                    1,
                    1,
                    Kind::NoteOn {
                        note: 60,
                        velocity: 127,
                    },
                ),
            ],
            20,
        );
        let pcm = render(&bank, &song, 0.0).unwrap();
        // The muted second voice must not overwrite the first voice's cached gain.
        assert!(pcm.iter().all(|s| *s > 0.1));
    }

    #[test]
    fn short_loops_wrap_multiple_periods_and_one_shots_end_safely() {
        let mut bank = constant_bank();
        let s = song(
            &[(
                0,
                0,
                Kind::NoteOn {
                    note: 127,
                    velocity: 127,
                },
            )],
            20,
        );
        assert!(
            render(&bank, &s, 0.0)
                .unwrap()
                .iter()
                .all(|s| s.is_finite() && *s > 0.1)
        );
        bank.instruments[0].regions[0].looped = None;
        bank.instruments[0].regions[0].articulation = Some(Articulation {
            pitch_cents: 100_000.0,
            ..Default::default()
        });
        assert!(
            render(&bank, &s, 0.0).unwrap()[1..]
                .iter()
                .all(|s| *s == 0.0)
        );
    }

    #[test]
    fn sustain_all_notes_off_and_all_sound_off_are_distinct() {
        let bank = constant_bank();
        for end in [
            Kind::NoteOff { note: 60 },
            Kind::Controller {
                number: 123,
                value: 0,
            },
        ] {
            let s = song(
                &[
                    (
                        0,
                        0,
                        Kind::Controller {
                            number: 64,
                            value: 127,
                        },
                    ),
                    (
                        0,
                        0,
                        Kind::NoteOn {
                            note: 60,
                            velocity: 127,
                        },
                    ),
                    (10, 0, end),
                    (
                        30,
                        0,
                        Kind::Controller {
                            number: 64,
                            value: 0,
                        },
                    ),
                ],
                70,
            );
            let pcm = render(&bank, &s, 0.0).unwrap();
            assert!(pcm[480] > 0.1); // still held by pedal at 20 ms
            assert!(pcm[1440].abs() < 0.0001); // released by 60 ms
        }
        let s = song(
            &[
                (
                    0,
                    0,
                    Kind::NoteOn {
                        note: 60,
                        velocity: 127,
                    },
                ),
                (
                    10,
                    0,
                    Kind::Controller {
                        number: 120,
                        value: 0,
                    },
                ),
            ],
            20,
        );
        assert!(
            render(&bank, &s, 0.0).unwrap()[240..]
                .iter()
                .all(|s| *s == 0.0)
        );
    }

    #[test]
    fn invalid_and_excessive_render_inputs_return_errors() {
        let mut bank = constant_bank();
        let s = Song::default();
        for tail in [f32::NAN, f32::INFINITY, -1.0] {
            assert!(render(&bank, &s, tail).is_err());
        }
        assert_eq!(render(&bank, &s, 241.0), Err(Error::TooLong));
        assert!(render(&bank, &song(&[(0, 16, Kind::NoteOff { note: 60 })], 1), 0.0).is_err());
        assert!(
            render(
                &bank,
                &song(
                    &[
                        (2, 0, Kind::NoteOff { note: 60 }),
                        (1, 0, Kind::NoteOff { note: 60 })
                    ],
                    2
                ),
                0.0
            )
            .is_err()
        );
        bank.instruments[0].regions[0].wave = 99;
        assert!(render(&bank, &s, 0.0).is_err());
        bank.instruments[0].regions[0].wave = 0;
        bank.instruments[0].regions[0].looped = Some((0, 0));
        assert!(render(&bank, &s, 0.0).is_err());
        bank.instruments[0].regions[0].looped = None;
        bank.instruments[0].articulation.lfo_hz = f32::NAN;
        assert!(render(&bank, &s, 0.0).is_err());
    }

    #[test]
    fn the_envelope_attacks_decays_and_releases() {
        let mut e = Envelope::new(0.01, 0.1, 0.5, 0.05);
        let mut peak = 0f32;
        for _ in 0..(RATE as usize / 100) {
            peak = peak.max(e.next());
        }
        assert!(peak > 0.98 && peak <= 1.0);
        for _ in 0..(RATE as usize / 2) {
            e.next();
        }
        assert!((e.level - 0.5).abs() < 0.01);
        e.release();
        for _ in 0..(RATE as usize / 2) {
            e.next();
        }
        assert!(e.done());
    }

    #[test]
    fn an_empty_song_renders_nothing_audible() {
        // a bank is not needed to render silence
        let dls = Dls {
            instruments: Vec::new(),
            waves: Vec::new(),
        };
        let song = smf::parse(&mml::compile("C", mml::Voice::melodic(0), 1.0, 1).unwrap()).unwrap();
        let out = render(&dls, &song, 0.5).unwrap();
        assert!(out.iter().all(|s| *s == 0.0));
    }

    /// The real bank: a flute scale has the right pitches and is not silent. Needs the game's pak.
    #[test]
    #[ignore = "reads sounds/default.dls from the game_pak"]
    fn the_game_bank_plays_a_scale() {
        let bytes = aa_pak::Pak::global().read("sounds/default.dls").unwrap();
        let dls = Dls::parse(&bytes).unwrap();
        assert!(dls.instruments.len() > 200);
        for program in [73u8, 24, 106, 0] {
            let midi =
                mml::compile("O5 L8 C E G > C", mml::Voice::melodic(program), 1.0, 1).unwrap();
            let song = smf::parse(&midi).unwrap();
            let out = render(&dls, &song, 1.0).unwrap();
            let peak = out.iter().fold(0f32, |m, s| m.max(s.abs()));
            assert!(peak > 0.05, "program {program} is silent (peak {peak})");
            assert!(out.iter().all(|s| s.is_finite()));
            assert!(peak <= 1.0);
            eprintln!("program {program}: {} samples, peak {peak:.6}", out.len());
        }
        let drums = mml::compile("C E F+ C", mml::Voice::PERCUSSION, 1.0, 1).unwrap();
        let out = render(&dls, &smf::parse(&drums).unwrap(), 1.0).unwrap();
        assert!(out.iter().fold(0f32, |m, s| m.max(s.abs())) > 0.05);
    }

    #[test]
    #[ignore = "reads sounds/default.dls from the game_pak"]
    fn original_flute_has_the_expected_fundamental() {
        let bank = Dls::parse(&aa_pak::Pak::global().read("sounds/default.dls").unwrap()).unwrap();
        for (score, expected_hz) in [("O5 A1", 440.0f64), ("O6 A1", 880.0)] {
            let song =
                smf::parse(&mml::compile(score, mml::Voice::melodic(73), 1.0, 7).unwrap()).unwrap();
            let out = render(&bank, &song, 0.1).unwrap();
            let window = &out[4800..9600];
            let lag = ((f64::from(RATE) / expected_hz * 0.9) as usize
                ..=(f64::from(RATE) / expected_hz * 1.1) as usize)
                .max_by(|&a, &b| {
                    let corr = |lag| {
                        window
                            .iter()
                            .zip(window.iter().skip(lag))
                            .map(|(a, b)| a * b)
                            .sum::<f32>()
                    };
                    corr(a).total_cmp(&corr(b))
                })
                .unwrap();
            let hz = f64::from(RATE) / lag as f64;
            assert!((hz / expected_hz - 1.0).abs() < 0.04, "{score}: {hz} Hz");
            eprintln!("original flute {score}: {hz:.3} Hz (expected {expected_hz:.3})");
        }
    }
}
