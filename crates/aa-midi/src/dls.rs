//! A DLS level 1 sound bank (`sounds/default.dls`: the Roland GM set, 235 instruments, 3.4 MB of 16 bit PCM).
//!
//! The file is a RIFF `DLS ` with the instruments in `lins` (each: `insh` the bank and program, `lrgn` the regions with
//! key and velocity ranges, `lart` the articulation) and the samples in `wvpl`, found through the `ptbl` cue table.
//! Only what the synthesizer uses is read: key and velocity ranges, the sample, its root note, tuning, loop and gain,
//! and the connections of the volume envelope (EG1), the pitch envelope (EG2) and the low frequency oscillator.

use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not a DLS file")]
    NotDls,
    #[error("the file is cut short in {0}")]
    Truncated(&'static str),
    #[error("a region points at wave {0}, which the pool does not have")]
    NoWave(u32),
    #[error("invalid DLS: {0}")]
    Invalid(&'static str),
    #[error("unsupported DLS: {0}")]
    Unsupported(&'static str),
    #[error("DLS exceeds the reader resource limit")]
    Limit,
}

/// A sample of the wave pool.
#[derive(Clone, Debug)]
pub struct Wave {
    pub rate: u32,
    pub pcm: Arc<[i16]>,
    /// Root note, cents of fine tuning and gain in dB of the wave's own `wsmp` (a region's overrides them).
    pub unity: u8,
    pub fine_tune: i16,
    pub gain_db: f32,
    /// Loop start and length in samples, when the wave loops.
    pub looped: Option<(u32, u32)>,
}

/// What the connection blocks of one articulation say, in plain units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Articulation {
    /// Volume envelope, seconds and a level between 0 and 1.
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
    /// How much the velocity (0 to 1) lengthens the attack, in timecents, and the key (0 to 1) the decay.
    pub velocity_to_attack: f32,
    pub key_to_decay: f32,
    /// Pitch envelope: its times, level and the cents it moves the pitch by.
    pub pitch_attack: f32,
    pub pitch_decay: f32,
    pub pitch_sustain: f32,
    pub pitch_release: f32,
    pub pitch_cents: f32,
    /// Oscillator: Hz, seconds before it starts, cents of vibrato and dB of tremolo.
    pub lfo_hz: f32,
    pub lfo_delay: f32,
    pub lfo_pitch_cents: f32,
    pub lfo_gain_db: f32,
    /// Pan, -1 left to 1 right.
    pub pan: f32,
}

impl Default for Articulation {
    fn default() -> Self {
        Articulation {
            attack: 0.0,
            decay: 0.0,
            sustain: 1.0,
            release: 0.0,
            velocity_to_attack: 0.0,
            key_to_decay: 0.0,
            pitch_attack: 0.0,
            pitch_decay: 0.0,
            pitch_sustain: 1.0,
            pitch_release: 0.0,
            pitch_cents: 0.0,
            lfo_hz: 5.0,
            lfo_delay: 0.0,
            lfo_pitch_cents: 0.0,
            lfo_gain_db: 0.0,
            pan: 0.0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Region {
    pub key: (u8, u8),
    pub velocity: (u8, u8),
    pub wave: usize,
    pub unity: u8,
    pub fine_tune: i16,
    pub gain_db: f32,
    pub looped: Option<(u32, u32)>,
    /// Voices of one key group (the hi-hats) cut each other off.
    pub key_group: u16,
    /// Its own articulation, the instrument's otherwise.
    pub articulation: Option<Articulation>,
}

#[derive(Clone, Debug)]
pub struct Instrument {
    pub program: u8,
    pub bank: u32,
    /// A drum kit (the channel 10 sounds).
    pub drum: bool,
    pub articulation: Articulation,
    pub regions: Vec<Region>,
}

#[derive(Clone, Debug)]
pub struct Dls {
    pub instruments: Vec<Instrument>,
    pub waves: Vec<Wave>,
}

/// Limits for untrusted bank input; the original bank is 3.4 MB / 495 waves.
pub const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_CHUNKS: usize = 16_384;
const MAX_WAVES: usize = 4096;
const MAX_INSTRUMENTS: usize = 1024;
const MAX_REGIONS: usize = 16_384;

struct Chunk<'a> {
    id: &'a [u8],
    list: Option<&'a [u8]>,
    data: &'a [u8],
    offset: u32,
}

fn chunks(d: &[u8]) -> Result<Vec<Chunk<'_>>, Error> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < d.len() {
        if out.len() == MAX_CHUNKS {
            return Err(Error::Limit);
        }
        let h = d
            .get(at..at + 8)
            .ok_or(Error::Truncated("a chunk header"))?;
        let size = u32_at(h, 4)? as usize;
        let end = at.checked_add(8 + size).ok_or(Error::Limit)?;
        let body = d.get(at + 8..end).ok_or(Error::Truncated("a chunk body"))?;
        let list = if &h[..4] == b"LIST" {
            Some(body.get(..4).ok_or(Error::Truncated("a list type"))?)
        } else {
            None
        };
        out.push(Chunk {
            id: &h[..4],
            list,
            data: if list.is_some() { &body[4..] } else { body },
            offset: at as u32,
        });
        at = end + (size & 1);
        if at > d.len() {
            return Err(Error::Truncated("chunk padding"));
        }
    }
    Ok(out)
}

fn u16_at(d: &[u8], i: usize) -> Result<u16, Error> {
    Ok(u16::from_le_bytes(
        d.get(i..i + 2)
            .ok_or(Error::Truncated("a field"))?
            .try_into()
            .unwrap(),
    ))
}
fn u32_at(d: &[u8], i: usize) -> Result<u32, Error> {
    Ok(u32::from_le_bytes(
        d.get(i..i + 4)
            .ok_or(Error::Truncated("a field"))?
            .try_into()
            .unwrap(),
    ))
}
fn required<'a, 'b>(parts: &'a [Chunk<'b>], id: &[u8]) -> Result<&'a Chunk<'b>, Error> {
    parts
        .iter()
        .find(|c| c.id == id)
        .ok_or(Error::Invalid("missing required chunk"))
}
fn seconds(scale: i32) -> f32 {
    if scale == i32::MIN {
        0.0
    } else {
        2f32.powf(scale as f32 / 65536.0 / 1200.0)
    }
}

fn articulation(d: &[u8]) -> Result<Articulation, Error> {
    let mut a = Articulation::default();
    let cb = u32_at(d, 0)? as usize;
    let n = u32_at(d, 4)? as usize;
    if cb < 8 || cb > d.len() || n > d.len().saturating_sub(cb) / 12 {
        return Err(Error::Invalid("articulation size"));
    }
    for i in 0..n {
        let at = cb + i * 12;
        let (src, ctrl, dst, transform, scale) = (
            u16_at(d, at)?,
            u16_at(d, at + 2)?,
            u16_at(d, at + 4)?,
            u16_at(d, at + 6)?,
            u32_at(d, at + 8)? as i32,
        );
        // Other routing/transforms remain unsupported by this approximate renderer.
        if transform != 0 {
            continue;
        }
        let cents = scale as f32 / 65536.0;
        match (src, ctrl, dst) {
            (0, 0, 0x206) => a.attack = seconds(scale),
            (0, 0, 0x207) => a.decay = seconds(scale),
            (0, 0, 0x209) => a.release = seconds(scale),
            (0, 0, 0x20a) => a.sustain = (cents / 1000.0).clamp(0.0, 1.0),
            (0, 0, 0x30a) => a.pitch_attack = seconds(scale),
            (0, 0, 0x30b) => a.pitch_decay = seconds(scale),
            (0, 0, 0x30d) => a.pitch_release = seconds(scale),
            (0, 0, 0x30e) => a.pitch_sustain = (cents / 1000.0).clamp(0.0, 1.0),
            (0, 0, 0x104) => a.lfo_hz = 8.175_8 * 2f32.powf(cents / 1200.0),
            (0, 0, 0x105) => a.lfo_delay = seconds(scale),
            (0, 0, 4) => a.pan = (cents / 500.0).clamp(-1.0, 1.0),
            (1, 0, 3) => a.lfo_pitch_cents = cents,
            (1, 0, 1) => a.lfo_gain_db = cents / 10.0,
            (5, 0, 3) => a.pitch_cents = cents,
            (2, 0, 0x206) => a.velocity_to_attack = cents,
            (3, 0, 0x207) => a.key_to_decay = cents,
            _ => {}
        }
    }
    Ok(a)
}

fn read_art(parts: &[Chunk<'_>]) -> Result<Option<Articulation>, Error> {
    if parts.iter().any(|c| c.list == Some(b"lar2")) {
        return Err(Error::Unsupported("DLS2 articulation"));
    }
    if let Some(lart) = parts.iter().find(|c| c.list == Some(b"lart")) {
        let a = chunks(lart.data)?;
        return Ok(Some(articulation(required(&a, b"art1")?.data)?));
    }
    Ok(None)
}

#[derive(Clone, Copy)]
struct Wsmp {
    unity: u8,
    fine_tune: i16,
    gain_db: f32,
    looped: Option<(u32, u32)>,
}
impl Default for Wsmp {
    fn default() -> Self {
        Self {
            unity: 60,
            fine_tune: 0,
            gain_db: 0.0,
            looped: None,
        }
    }
}
fn wsmp(d: &[u8]) -> Result<Wsmp, Error> {
    let cb = u32_at(d, 0)? as usize;
    let unity = u16_at(d, 4)?;
    let fine_tune = u16_at(d, 6)? as i16;
    let gain_db = (u32_at(d, 8)? as i32) as f32 / 655_360.0;
    let loops = u32_at(d, 16)?;
    if cb < 20 || cb > d.len() || unity > 127 {
        return Err(Error::Invalid("sample header"));
    }
    if loops > 1 {
        return Err(Error::Unsupported("multiple sample loops"));
    }
    let looped = if loops == 1 {
        let size = u32_at(d, cb)? as usize;
        if size < 16 || size > d.len() - cb {
            return Err(Error::Invalid("sample loop size"));
        }
        if u32_at(d, cb + 4)? != 0 {
            return Err(Error::Unsupported("sample loop type"));
        }
        let start = u32_at(d, cb + 8)?;
        let length = u32_at(d, cb + 12)?;
        if length == 0 {
            return Err(Error::Invalid("zero sample loop length"));
        }
        Some((start, length))
    } else {
        None
    };
    Ok(Wsmp {
        unity: unity as u8,
        fine_tune,
        gain_db,
        looped,
    })
}
fn read_wsmp(parts: &[Chunk<'_>]) -> Result<Option<Wsmp>, Error> {
    parts
        .iter()
        .find(|c| c.id == b"wsmp")
        .map(|c| wsmp(c.data))
        .transpose()
}
fn check_loop(looped: Option<(u32, u32)>, samples: usize) -> Result<(), Error> {
    if let Some((start, len)) = looped
        && (len == 0 || u64::from(start) + u64::from(len) > samples as u64)
    {
        return Err(Error::Invalid("sample loop outside wave"));
    }
    Ok(())
}

impl Dls {
    /// Parse mono 8/16 bit PCM DLS1 banks. Unsupported wave encodings and loop
    /// types fail explicitly; every chunk, cue and loop is checked before use.
    pub fn parse(d: &[u8]) -> Result<Dls, Error> {
        if d.len() > MAX_BYTES {
            return Err(Error::Limit);
        }
        if d.len() < 12 || &d[..4] != b"RIFF" || &d[8..12] != b"DLS " {
            return Err(Error::NotDls);
        }
        let end = u32_at(d, 4)? as usize + 8;
        let body = d.get(12..end).ok_or(Error::Truncated("RIFF body"))?;
        let top = chunks(body)?;
        let pool = top
            .iter()
            .find(|c| c.list == Some(b"wvpl"))
            .ok_or(Error::Invalid("missing wave pool"))?;
        let mut waves = Vec::new();
        let mut offsets = Vec::new();
        for wave in chunks(pool.data)?
            .iter()
            .filter(|c| c.list == Some(b"wave"))
        {
            if waves.len() == MAX_WAVES {
                return Err(Error::Limit);
            }
            let parts = chunks(wave.data)?;
            let fmt = required(&parts, b"fmt ")?.data;
            let (encoding, channels, rate, align, bits) = (
                u16_at(fmt, 0)?,
                u16_at(fmt, 2)?,
                u32_at(fmt, 4)?,
                u16_at(fmt, 12)?,
                u16_at(fmt, 14)?,
            );
            if encoding != 1 || channels != 1 || ![8, 16].contains(&bits) {
                return Err(Error::Unsupported("wave encoding (requires mono PCM8/16)"));
            }
            if !(8000..=192_000).contains(&rate) || align != bits / 8 {
                return Err(Error::Invalid("wave format"));
            }
            let data = required(&parts, b"data")?.data;
            if data.is_empty() || data.len() % usize::from(align) != 0 {
                return Err(Error::Invalid("wave data size"));
            }
            let pcm: Vec<i16> = if bits == 8 {
                data.iter().map(|b| (i16::from(*b) - 128) << 8).collect()
            } else {
                data.as_chunks::<2>()
                    .0
                    .iter()
                    .map(|b| i16::from_le_bytes([b[0], b[1]]))
                    .collect()
            };
            let s = read_wsmp(&parts)?.unwrap_or_default();
            check_loop(s.looped, pcm.len())?;
            offsets.push(wave.offset);
            waves.push(Wave {
                rate,
                pcm: pcm.into(),
                unity: s.unity,
                fine_tune: s.fine_tune,
                gain_db: s.gain_db,
                looped: s.looped,
            });
        }
        let ptbl = required(&top, b"ptbl")?.data;
        let cb = u32_at(ptbl, 0)? as usize;
        let n = u32_at(ptbl, 4)? as usize;
        if n > MAX_WAVES {
            return Err(Error::Limit);
        }
        if cb < 8 || cb > ptbl.len() || n > (ptbl.len() - cb) / 4 {
            return Err(Error::Invalid("pool table size"));
        }
        let mut cues = Vec::new();
        for i in 0..n {
            let offset = u32_at(ptbl, cb + i * 4)?;
            cues.push(
                offsets
                    .binary_search(&offset)
                    .map_err(|_| Error::NoWave(i as u32))?,
            );
        }
        let lins = top
            .iter()
            .find(|c| c.list == Some(b"lins"))
            .ok_or(Error::Invalid("missing instruments"))?;
        let mut instruments = Vec::new();
        let mut region_count = 0;
        for ins in chunks(lins.data)?
            .iter()
            .filter(|c| c.list == Some(b"ins "))
        {
            if instruments.len() == MAX_INSTRUMENTS {
                return Err(Error::Limit);
            }
            let parts = chunks(ins.data)?;
            let header = required(&parts, b"insh")?.data;
            let bank = u32_at(header, 4)?;
            let program = u32_at(header, 8)?;
            if program > 127 {
                return Err(Error::Invalid("instrument program"));
            }
            let art = read_art(&parts)?.unwrap_or_default();
            let lrgn = parts
                .iter()
                .find(|c| c.list == Some(b"lrgn"))
                .ok_or(Error::Invalid("missing regions"))?;
            let mut regions = Vec::new();
            for rgn in chunks(lrgn.data)?
                .iter()
                .filter(|c| c.list == Some(b"rgn ") || c.list == Some(b"rgn2"))
            {
                if rgn.list == Some(b"rgn2") {
                    return Err(Error::Unsupported("DLS2 region"));
                }
                region_count += 1;
                if region_count > MAX_REGIONS {
                    return Err(Error::Limit);
                }
                let rp = chunks(rgn.data)?;
                let h = required(&rp, b"rgnh")?.data;
                let key = (u16_at(h, 0)?, u16_at(h, 2)?);
                let velocity = (u16_at(h, 4)?, u16_at(h, 6)?);
                if key.0 > key.1 || key.1 > 127 || velocity.0 > velocity.1 || velocity.1 > 127 {
                    return Err(Error::Invalid("region ranges"));
                }
                let link = required(&rp, b"wlnk")?.data;
                let cue = u32_at(link, 8)?;
                let wave = *cues.get(cue as usize).ok_or(Error::NoWave(cue))?;
                let w = &waves[wave];
                let s = read_wsmp(&rp)?.unwrap_or(Wsmp {
                    unity: w.unity,
                    fine_tune: w.fine_tune,
                    gain_db: w.gain_db,
                    looped: w.looped,
                });
                check_loop(s.looped, w.pcm.len())?;
                regions.push(Region {
                    key: (key.0 as u8, key.1 as u8),
                    velocity: (velocity.0 as u8, velocity.1 as u8),
                    wave,
                    unity: s.unity,
                    fine_tune: s.fine_tune,
                    gain_db: s.gain_db,
                    looped: s.looped,
                    key_group: u16_at(h, 10)?,
                    articulation: read_art(&rp)?,
                });
            }
            if regions.len() != u32_at(header, 0)? as usize {
                return Err(Error::Invalid("region count"));
            }
            instruments.push(Instrument {
                program: program as u8,
                bank,
                drum: bank & 0x8000_0000 != 0,
                articulation: art,
                regions,
            });
        }
        if let Some(colh) = top.iter().find(|c| c.id == b"colh")
            && u32_at(colh.data, 0)? as usize != instruments.len()
        {
            return Err(Error::Invalid("instrument count"));
        }
        Ok(Dls { instruments, waves })
    }

    /// GM bank zero for melodic channels; bank zero drum kits with the nearest
    /// lower kit as fallback. Bank variations require explicit selection by the caller.
    pub fn instrument(&self, program: u8, drum: bool) -> Option<&Instrument> {
        let bank = if drum { 0x8000_0000 } else { 0 };
        self.instruments
            .iter()
            .find(|i| i.bank == bank && i.program == program)
            .or_else(|| {
                if drum {
                    self.instruments
                        .iter()
                        .filter(|i| i.bank == bank && i.program <= program)
                        .max_by_key(|i| i.program)
                        .or_else(|| self.instruments.iter().find(|i| i.bank == bank))
                } else {
                    None
                }
            })
    }
}
