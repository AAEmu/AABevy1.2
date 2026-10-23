//! The score text of the composition window as a standard MIDI file: a port of the client's compiler
//! (`x2game.dll` `FUN_390838a0`, `FUN_39081860` the tokenizer, `FUN_39082e60` the note writer).
//!
//! ## The language
//!
//! Case does not matter and every character the compiler does not know is skipped. Up to three tracks are separated by
//! commas (what comes after the third comma is ignored); the tracks play together.
//!
//! | code | meaning |
//! |---|---|
//! | `C D E F G A B` | a note; the syllables `도 레 미 파 솔 라 시`, `ド レ ミ ファ ソ ラ シ` and `哆 来 咪 发 梭 拉 西` work too |
//! | `+` or `#` / `-` after a note | sharp / flat |
//! | `n` after a note, `1` to `64` | its length, `n` th of a whole note (`C4`); without one the default length is used |
//! | `.` | a dot: the length times 1.5, once per dot |
//! | `&` after a note | tie: the note joins the next one when both have the same pitch |
//! | `R`, `Rn` | a rest of `n` th of a whole note (dots do not apply to rests) |
//! | `On` | the octave, `0` to `10`, default 5 (3 for percussion); middle C is `O5 C`, MIDI note 60 |
//! | `>` / `<` | one octave up / down |
//! | `Vn` | the velocity, `0` to `127`, default 100 |
//! | `Ln` | the default note length, `1` to `64`, default `4`; dots apply |
//! | `Tn` | the tempo in quarter notes a minute, `32` to `255`; default 120 (the file's own 500 ms a quarter) |
//!
//! A number out of range is an error ([`Error::Code`]); a score without a single note is one too ([`Error::Empty`]).
//!
//! ## The file
//!
//! Format 0 for one track, 1 for more, 500 ticks to a quarter note and the default MIDI tempo (500000 microseconds a
//! quarter), so a tick is a millisecond. Each track starts with a program change on channel 0 and its notes use the
//! voice's channel (9 for percussion). The state (octave, velocity, tempo, default length) carries on from one track to
//! the next, as in the client.
//!
//! ## Wrong notes
//!
//! A player whose proficiency only just reaches the song's length plays it with a `quality` below 1. The native
//! random threshold then decides whether to move each note by `rand() % 15 - 7` semitones. Pass 1.0 for perfect notes.

/// The instrument a score is compiled for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Voice {
    /// General MIDI program (`instrument_sounds.midi`).
    pub program: u8,
    /// MIDI channel, 9 is the drum kit.
    pub channel: u8,
    /// The octave a score starts in.
    pub octave: i32,
}

impl Voice {
    /// A melodic instrument of this program.
    pub const fn melodic(program: u8) -> Voice {
        Voice {
            program,
            channel: 0,
            octave: 5,
        }
    }

    /// The percussion instruments: the drum kit, written one octave step of 12 notes from note 36.
    pub const PERCUSSION: Voice = Voice {
        program: 35,
        channel: 9,
        octave: 3,
    };
}

/// Why a score does not compile.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// A number out of range; the text is the 14 characters from where the code starts (the client names them in its
    /// "code error" message).
    #[error("code error near {0:?}")]
    Code(String),
    /// Not a single note.
    #[error("the score has no notes")]
    Empty,
    /// Score text, event delta or output bytes exceed defensive limits.
    #[error("the score is too long")]
    TooLong,
    #[error("invalid compiler parameter: {0}")]
    InvalidParameter(&'static str),
}

/// The longest file the client accepts (`FUN_39083960`: 0x8000).
pub const MAX_BYTES: usize = 0x8000;
/// The original composition editor's hard character limit (proficiency may impose a smaller one).
/// This is a caller-side editing/saving limit, not the compiler's total input limit.
pub const MAX_CHARACTERS: usize = 1000;
// XlMultiToWide3301a100 receives capacity 3001, including its terminating u32 zero.
const MAX_INPUT_CHARACTERS: usize = 3000;
// FUN_39081d90 advances by 4000 bytes per track and also writes a terminating u32.
const TRACK_CHARACTERS: usize = 1000;
const MAX_DELTA: i32 = 0x0fff_ffff;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// A note, 0 for C ... 11 for B.
    Note(u8),
    Rest,
    Octave,
    Velocity,
    /// `N`: read and ignored.
    Number,
    Length,
    Tempo,
    OctaveUp,
    OctaveDown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Token {
    kind: Kind,
    /// The number after the code, -1 when there is none.
    value: i32,
    /// +1 sharp, -1 flat.
    accidental: i8,
    tie: bool,
    dots: u32,
}

/// The note a character names, and how many characters it takes.
fn note_of(c: char, next: char) -> Option<(Kind, usize)> {
    let n = |s: u8| Some((Kind::Note(s), 1));
    match c {
        'C' | '도' | 'ド' | '哆' => n(0),
        'D' | '레' | 'レ' | '来' => n(2),
        'E' | '미' | 'ミ' | '咪' => n(4),
        'F' | '파' | '发' => n(5),
        'フ' if next == 'ァ' => Some((Kind::Note(5), 2)),
        'G' | '솔' | 'ソ' | '梭' => n(7),
        'A' | '라' | 'ラ' | '拉' => n(9),
        'B' | '시' | 'シ' | '西' => n(11),
        _ => None,
    }
}

fn command_of(c: char) -> Option<Kind> {
    Some(match c {
        'R' => Kind::Rest,
        'O' => Kind::Octave,
        'V' => Kind::Velocity,
        'N' => Kind::Number,
        'L' => Kind::Length,
        'T' => Kind::Tempo,
        _ => return None,
    })
}

/// Whether `n` is allowed after this code (`FUN_390817d0`).
fn number_ok(kind: Kind, n: i32) -> bool {
    match kind {
        Kind::Note(_) | Kind::Rest | Kind::Length => (1..=64).contains(&n),
        Kind::Octave => (0..=10).contains(&n),
        Kind::Velocity | Kind::Number => (0..128).contains(&n),
        Kind::Tempo => (32..256).contains(&n),
        Kind::OctaveUp | Kind::OctaveDown => false,
    }
}

fn upper(c: char) -> char {
    // xlcommon!XlToUpper(uint) leaves codepoints above U+07FF unchanged.
    if c as u32 > 0x7ff {
        c
    } else {
        c.to_uppercase().next().unwrap_or(c)
    }
}

/// One token from the start of `s`: how many characters it took and the token (none for a skipped character), or the
/// error of a number out of range (`FUN_39081860`).
fn next_token(s: &[char]) -> Result<(usize, Option<Token>), ()> {
    let mut i = 0;
    while i < s.len() && matches!(s[i], ' ' | '\t' | '\r' | '\n' | '\u{b}' | '\u{c}') {
        i += 1;
    }
    let mut token: Option<Token> = None;
    while i < s.len() {
        let c = upper(s[i]);
        let next = s.get(i + 1).copied().map_or('\0', upper);
        let Some(t) = token.as_mut() else {
            let kind = match c {
                '<' => {
                    return Ok((
                        i + 1,
                        Some(Token {
                            kind: Kind::OctaveDown,
                            value: -1,
                            accidental: 0,
                            tie: false,
                            dots: 0,
                        }),
                    ));
                }
                '>' => {
                    return Ok((
                        i + 1,
                        Some(Token {
                            kind: Kind::OctaveUp,
                            value: -1,
                            accidental: 0,
                            tie: false,
                            dots: 0,
                        }),
                    ));
                }
                _ => match note_of(c, next) {
                    Some((kind, used)) => {
                        i += used - 1;
                        kind
                    }
                    None => match command_of(c) {
                        Some(kind) => kind,
                        // a character that starts nothing is skipped
                        None => return Ok((i + 1, None)),
                    },
                },
            };
            token = Some(Token {
                kind,
                value: -1,
                accidental: 0,
                tie: false,
                dots: 0,
            });
            i += 1;
            continue;
        };
        if c == '.' {
            t.dots += 1;
            i += 1;
            continue;
        }
        // after a dot nothing else belongs to the token
        if t.dots != 0 {
            return Ok((i, token));
        }
        match c {
            '#' | '+' => t.accidental = 1,
            '-' => t.accidental = -1,
            '&' => t.tie = true,
            '0'..='9' => {
                let mut n = 0i32;
                while i < s.len() && s[i].is_ascii_digit() {
                    // The native x86 tokenizer wraps the accumulator, then validates the result.
                    n = n.wrapping_mul(10).wrapping_add(s[i] as i32 - '0' as i32);
                    i += 1;
                }
                if !number_ok(t.kind, n) {
                    return Err(());
                }
                t.value = n;
                continue;
            }
            // the start of the next code ends this one
            '<' | '>' | 'A'..='G' | 'R' | 'O' | 'V' | 'N' | 'L' | 'T' => return Ok((i, token)),
            _ if note_of(c, next).is_some() => return Ok((i, token)),
            _ => return Ok((i + 1, token)),
        }
        i += 1;
    }
    Ok((i, token))
}

/// The C runtime's `rand()` (the client seeds it with the time): the only randomness is the wrong notes.
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> i32 {
        self.0 = self.0.wrapping_mul(214013).wrapping_add(2531011);
        ((self.0 >> 16) & 0x7fff) as i32
    }
}

struct Writer<'a> {
    voice: Voice,
    quality: f32,
    rng: &'a mut Rng,
    octave: i32,
    velocity: i32,
    /// The default note length in ticks (a whole note is 2000).
    length: i32,
    /// Milliseconds a quarter note takes; 500 is the file's own tempo.
    tempo: i32,
    /// Rest ticks to put before the next note.
    rest: i32,
    pending: Vec<Token>,
    events: Vec<u8>,
    limited: bool,
}

const BASE_TEMPO: i32 = 500;

/// A variable length quantity.
fn vlq(out: &mut Vec<u8>, v: i32) {
    let v = v.max(0) as u32;
    let mut stack = [0u8; 5];
    let mut n = 0;
    let mut rest = v;
    loop {
        stack[n] = (rest & 0x7f) as u8;
        n += 1;
        rest >>= 7;
        if rest == 0 {
            break;
        }
    }
    for i in (0..n).rev() {
        out.push(stack[i] | if i > 0 { 0x80 } else { 0 });
    }
}

impl Writer<'_> {
    fn ratio(&self) -> f32 {
        self.tempo as f32 / BASE_TEMPO as f32
    }

    fn pitch(&self, t: &Token) -> i32 {
        let base = match t.kind {
            Kind::Note(s) => i32::from(s) + self.octave * 12,
            _ => -1,
        };
        base + i32::from(t.accidental)
    }

    /// Ticks of a token: `n`th of a whole note or the default, times 1.5 for every dot.
    fn length_of(&self, t: &Token) -> i32 {
        let mut ticks = if (1..=64).contains(&t.value) {
            (2000.0 / t.value as f32) as i32
        } else {
            self.length
        };
        for _ in 0..t.dots {
            ticks = (ticks as f32 * 1.5) as i32;
        }
        ticks
    }

    /// One note on the track: the rest before it and its length, both scaled by the tempo.
    fn put(&mut self, mut note: i32, ticks: i32) {
        let ratio = self.ratio();
        let delay = self.rest as f32 * ratio;
        let duration = ticks as f32 * ratio;
        if self.rest > MAX_DELTA
            || ticks > MAX_DELTA
            || delay >= 268_435_456.0
            || duration >= 268_435_456.0
            || self.events.len() + 14 > MAX_BYTES
        {
            self.limited = true;
            return;
        }
        if self.quality < 1.0 {
            let r = self.rng.next();
            let off = self.rng.next();
            if self.quality < (r % 10000) as f32 * 0.0001 {
                note += off % 15 - 7;
            }
        }
        let note = note.clamp(0, 127) as u8;
        vlq(&mut self.events, delay as i32);
        self.events
            .extend([0x90 + self.voice.channel, note, self.velocity as u8]);
        vlq(&mut self.events, duration as i32);
        self.events.extend([0x80 + self.voice.channel, note, 0]);
        self.rest = 0;
    }

    /// Writes out the tied notes: runs of one pitch become one note. Returns the pitch and ticks of the last run, which
    /// the caller either writes or joins with the note that ended the tie.
    fn drain_ties(&mut self) -> Option<(i32, i32)> {
        let pending = std::mem::take(&mut self.pending);
        let first = pending.first()?;
        let mut current = self.pitch(first);
        let mut ticks = 0i32;
        for t in &pending {
            let p = self.pitch(t);
            if p == current {
                ticks = ticks.saturating_add(self.length_of(t));
            } else {
                self.put(current, ticks);
                current = p;
                ticks = self.length_of(t);
            }
        }
        Some((current, ticks))
    }

    /// Ties that are not followed by a note are written as they are (every code but a note does this first).
    fn flush(&mut self) {
        if let Some((pitch, ticks)) = self.drain_ties() {
            self.put(pitch, ticks);
        }
    }

    fn token(&mut self, t: Token) {
        match t.kind {
            Kind::Note(_) => {
                if t.tie {
                    self.pending.push(t);
                    return;
                }
                let mut ticks = self.length_of(&t);
                if let Some((pitch, tied)) = self.drain_ties() {
                    if pitch == self.pitch(&t) {
                        ticks = ticks.saturating_add(tied);
                    } else {
                        self.put(pitch, tied);
                    }
                }
                let pitch = self.pitch(&t);
                self.put(pitch, ticks);
            }
            Kind::Rest => {
                self.flush();
                self.rest = self.rest.saturating_add(if (1..=64).contains(&t.value) {
                    (2000.0 / t.value as f32) as i32
                } else {
                    self.length
                });
            }
            Kind::Octave => {
                self.flush();
                self.octave = t.value;
            }
            Kind::Velocity => {
                self.flush();
                if t.value != -1 {
                    self.velocity = t.value;
                }
            }
            Kind::Number => {}
            Kind::Length => {
                self.flush();
                if (1..=64).contains(&t.value) {
                    let mut ticks = (2000.0 / t.value as f32) as i32;
                    for _ in 0..t.dots {
                        ticks = (ticks as f32 * 1.5) as i32;
                    }
                    self.length = ticks;
                }
            }
            Kind::Tempo => {
                self.flush();
                if (32..256).contains(&t.value) {
                    self.tempo = (60000.0 / t.value as f32) as i32;
                }
            }
            Kind::OctaveUp => {
                self.flush();
                self.octave = (self.octave + 1).min(10);
            }
            Kind::OctaveDown => {
                self.flush();
                self.octave = (self.octave - 1).max(0);
            }
        }
    }
}

/// Splits the text into its tracks at the commas: three at most, and a comma at the very end starts none.
fn tracks(chars: &[char]) -> Vec<&[char]> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, c) in chars.iter().enumerate() {
        if *c == ',' {
            out.push(&chars[start..i]);
            start = i + 1;
            if out.len() == 3 {
                return out;
            }
        }
    }
    if start < chars.len() || out.is_empty() {
        out.push(&chars[start..]);
    }
    out
}

/// Compiles a score into a standard MIDI file. `seed` only matters for a `quality` below 1
/// and is restarted per track. Accepts up to 3000 codepoints before the first NUL, matching
/// the native UTF-8 wrapper's output capacity. The caller owns the wrapper's initial BOM
/// removal/truncation and editor/proficiency save limits. Each track is bounded to 999
/// codepoints; a final first/second track may have 1000. Larger inputs and layouts that
/// overwrite native terminators/pointers return [`Error::TooLong`]. Empty input is rejected
/// instead of emitting the native compiler's invalid zero-track header.
pub fn compile(text: &str, voice: Voice, quality: f32, seed: u32) -> Result<Vec<u8>, Error> {
    if voice.program > 127 || voice.channel > 15 || !(0..=10).contains(&voice.octave) {
        return Err(Error::InvalidParameter("voice"));
    }
    if !quality.is_finite() || !(0.0..=1.0).contains(&quality) {
        return Err(Error::InvalidParameter("quality"));
    }
    // Bound the converted, NUL-terminated input before allocating or tokenizing.
    let chars: Vec<char> = text
        .chars()
        .take(MAX_INPUT_CHARACTERS + 1)
        .take_while(|c| *c != '\0')
        .collect();
    if chars.len() > MAX_INPUT_CHARACTERS {
        return Err(Error::TooLong);
    }
    let parts = tracks(&chars);
    // Native splitting has no bounds checks and tokenization reads until NUL, not the
    // stored length. A saturated track aliases a following nonempty one. The third
    // track's terminator would overwrite pointer storage. Keep the working 1000-char
    // final first/second-track cases; reject overflow even after an empty track since
    // native splitting happens before determining how many tracks will be compiled.
    for (i, part) in parts.iter().enumerate() {
        if part.len() > TRACK_CHARACTERS
            || (part.len() == TRACK_CHARACTERS
                && (i == 2 || parts.get(i + 1).is_some_and(|next| !next.is_empty())))
        {
            return Err(Error::TooLong);
        }
    }
    // the tracks that count are the leading non-empty ones
    let parts: Vec<&[char]> = parts.into_iter().take_while(|t| !t.is_empty()).collect();
    if parts.is_empty() {
        return Err(Error::Empty);
    }
    let mut rng = Rng(seed);
    let mut w = Writer {
        voice,
        quality,
        rng: &mut rng,
        octave: voice.octave,
        velocity: 100,
        length: BASE_TEMPO,
        tempo: BASE_TEMPO,
        rest: 0,
        pending: Vec::new(),
        events: Vec::new(),
        limited: false,
    };
    let mut out = Vec::new();
    out.extend(b"MThd");
    out.extend(6u32.to_be_bytes());
    out.extend((if parts.len() < 2 { 0u16 } else { 1 }).to_be_bytes());
    out.extend((parts.len() as u16).to_be_bytes());
    out.extend(500u16.to_be_bytes());
    for part in parts {
        // FUN_39083480 calls srand(time(NULL)) at the beginning of every track.
        w.rng.0 = seed;
        let mut tokens = Vec::new();
        let mut at = 0;
        while at < part.len() {
            let (used, token) = next_token(&part[at..])
                .map_err(|()| Error::Code(part[at..].iter().take(14).collect()))?;
            at += used.max(1);
            tokens.extend(token);
        }
        w.events.clear();
        for t in tokens {
            w.token(t);
        }
        w.flush();
        if w.limited {
            return Err(Error::TooLong);
        }
        w.rest = 0;
        if w.events.is_empty() {
            return Err(Error::Empty);
        }
        let mut body = vec![0x00, 0xc0, voice.program];
        body.append(&mut w.events);
        body.extend([0x00, 0xff, 0x2f, 0x00]);
        out.extend(b"MTrk");
        out.extend((body.len() as u32).to_be_bytes());
        out.extend(body);
    }
    if out.len() > MAX_BYTES {
        return Err(Error::TooLong);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::smf;

    fn notes(midi: &[u8]) -> Vec<(u32, u8, u8, bool)> {
        smf::parse(midi)
            .unwrap()
            .events
            .iter()
            .filter_map(|e| match e.kind {
                smf::Kind::NoteOn { note, velocity } => Some((e.ms, note, velocity, true)),
                smf::Kind::NoteOff { note } => Some((e.ms, note, 0, false)),
                _ => None,
            })
            .collect()
    }

    fn build(text: &str) -> Vec<u8> {
        compile(text, Voice::melodic(73), 1.0, 1).unwrap()
    }

    #[test]
    fn header_and_program() {
        let m = build("C");
        assert_eq!(&m[..14], b"MThd\0\0\0\x06\0\0\0\x01\x01\xf4");
        assert_eq!(&m[14..18], b"MTrk");
        // delta 0, program change 73 on channel 0, then the note
        assert_eq!(&m[22..25], &[0x00, 0xc0, 73]);
        assert_eq!(&m[25..29], &[0x00, 0x90, 60, 100]);
        assert!(m.ends_with(&[0x00, 0xff, 0x2f, 0x00]));
    }

    #[test]
    fn default_note_is_a_quarter_in_octave_five() {
        // 500 ticks, a quarter at 120 BPM
        assert_eq!(
            notes(&build("C")),
            vec![(0, 60, 100, true), (500, 60, 0, false)]
        );
    }

    #[test]
    fn lengths_dots_and_rests() {
        let n = notes(&build("C8 R4 D16. E2"));
        // 8th = 250, rest 500, dotted 16th = 125 * 1.5 = 187 (truncated), then a half = 1000
        assert_eq!(n[0], (0, 60, 100, true));
        assert_eq!(n[1], (250, 60, 0, false));
        assert_eq!(n[2], (750, 62, 100, true));
        assert_eq!(n[3], (937, 62, 0, false));
        assert_eq!(n[4], (937, 64, 100, true));
        assert_eq!(n[5], (1937, 64, 0, false));
    }

    #[test]
    fn octave_volume_default_length_and_tempo() {
        let n = notes(&build("O4 V80 L8 T240 C"));
        // T240: 250 ms a quarter, so ratio 0.5: an 8th of 250 ticks lasts 125 ms
        assert_eq!(n, vec![(0, 48, 80, true), (125, 48, 0, false)]);
        let n = notes(&build("> C < < C"));
        assert_eq!((n[0].1, n[2].1), (72, 48));
    }

    #[test]
    fn accidentals_and_syllables() {
        let n = notes(&build("C# D- 미 파+ ソ"));
        let pitches: Vec<u8> = n.iter().filter(|e| e.3).map(|e| e.1).collect();
        assert_eq!(pitches, vec![61, 61, 64, 66, 67]);
    }

    #[test]
    fn ties_join_notes_of_one_pitch() {
        let n = notes(&build("C&C"));
        assert_eq!(n, vec![(0, 60, 100, true), (1000, 60, 0, false)]);
        // different pitches are written one after the other
        let n = notes(&build("C&D"));
        assert_eq!(n.len(), 4);
        assert_eq!((n[1].0, n[2].0, n[3].0), (500, 500, 1000));
        // a tie before a rest is written at the rest
        let n = notes(&build("C&R4 D"));
        assert_eq!(n[1], (500, 60, 0, false));
        assert_eq!(n[2], (1000, 62, 100, true));
    }

    #[test]
    fn commas_make_tracks_and_state_carries_over() {
        let m = compile("C, E", Voice::melodic(24), 1.0, 1).unwrap();
        let song = smf::parse(&m).unwrap();
        assert_eq!(song.tracks, 2);
        assert_eq!(m[9], 1);
        // the second track starts in the octave the first left
        let m = compile("O3 C, C", Voice::melodic(24), 1.0, 1).unwrap();
        let on: Vec<u8> = notes(&m).iter().filter(|e| e.3).map(|e| e.1).collect();
        assert_eq!(on, vec![36, 36]);
        // the fourth track is dropped
        let m = compile("C, D, E, F", Voice::melodic(24), 1.0, 1).unwrap();
        assert_eq!(smf::parse(&m).unwrap().tracks, 3);
    }

    #[test]
    fn percussion_uses_the_drum_channel_from_octave_three() {
        let m = compile("C E F+", Voice::PERCUSSION, 1.0, 1).unwrap();
        let on: Vec<(u8, u8)> = smf::parse(&m)
            .unwrap()
            .events
            .iter()
            .filter_map(|e| match e.kind {
                smf::Kind::NoteOn { note, .. } => Some((e.channel, note)),
                _ => None,
            })
            .collect();
        // bass drum, snare and the closed hi-hat of the GM kit
        assert_eq!(on, vec![(9, 36), (9, 40), (9, 42)]);
    }

    #[test]
    fn unknown_characters_are_skipped_and_numbers_checked() {
        assert_eq!(notes(&build("C!?x D")).len(), 4);
        assert!(
            matches!(compile("C65", Voice::melodic(0), 1.0, 1), Err(Error::Code(s)) if s == "C65")
        );
        assert!(matches!(
            compile("O11 C", Voice::melodic(0), 1.0, 1),
            Err(Error::Code(_))
        ));
        assert!(matches!(
            compile("T20 C", Voice::melodic(0), 1.0, 1),
            Err(Error::Code(_))
        ));
        assert_eq!(compile("", Voice::melodic(0), 1.0, 1), Err(Error::Empty));
        assert_eq!(
            compile("T120 O4", Voice::melodic(0), 1.0, 1),
            Err(Error::Empty)
        );
        assert_eq!(compile("   ", Voice::melodic(0), 1.0, 1), Err(Error::Empty));
    }

    #[test]
    fn a_lower_quality_moves_notes_and_one_keeps_them() {
        let text = "C D E F G A B ".repeat(40);
        let exact = notes(&compile(&text, Voice::melodic(0), 1.0, 7).unwrap());
        let sloppy = notes(&compile(&text, Voice::melodic(0), 0.5, 7).unwrap());
        assert_eq!(exact.len(), sloppy.len());
        assert_ne!(exact, sloppy);
        assert_eq!(
            exact,
            notes(&compile(&text, Voice::melodic(0), 1.0, 99).unwrap())
        );
    }

    #[test]
    fn the_vlq_is_the_standard_one() {
        let mut v = Vec::new();
        vlq(&mut v, 0);
        vlq(&mut v, 127);
        vlq(&mut v, 128);
        vlq(&mut v, 2000);
        vlq(&mut v, 0x0fffffff);
        assert_eq!(
            v,
            [0x00, 0x7f, 0x81, 0x00, 0x8f, 0x50, 0xff, 0xff, 0xff, 0x7f]
        );
    }

    #[test]
    fn a_three_part_score_compiles() {
        // Synthetic score; the help page's example is an image, not a Lua string.
        let m = build("CDEFG, GFEDC, r4CDEr4");
        assert_eq!(smf::parse(&m).unwrap().tracks, 3);
    }
}
