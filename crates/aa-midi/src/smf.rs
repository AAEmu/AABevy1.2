//! A bounded Standard MIDI File (format 0 or 1) reader with a shared tempo map.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    NoteOn {
        note: u8,
        velocity: u8,
    },
    NoteOff {
        note: u8,
    },
    ProgramChange {
        program: u8,
    },
    Controller {
        number: u8,
        value: u8,
    },
    /// 14 bit pitch bend, 8192 is the middle.
    PitchBend {
        value: u16,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    pub ms: u32,
    pub channel: u8,
    pub kind: Kind,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Song {
    pub tracks: usize,
    /// Stable file order for simultaneous events, ordered by time.
    pub events: Vec<Event>,
    /// Includes end-of-track silence.
    pub duration_ms: u32,
}

impl Song {
    pub fn length_ms(&self) -> u32 {
        self.duration_ms.max(self.events.last().map_or(0, |e| e.ms))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("not a MIDI file")]
    NotMidi,
    #[error("the file ends in the middle of {0}")]
    Truncated(&'static str),
    #[error("invalid MIDI: {0}")]
    Invalid(&'static str),
    #[error("MIDI exceeds the reader's resource or duration limit")]
    Limit,
    #[error("SMPTE time is not supported")]
    Smpte,
}

/// Reader limits, independent of the native composition payload's smaller 32 KiB cap.
pub const MAX_BYTES: usize = 1024 * 1024;
pub const MAX_EVENTS: usize = 65_536;
pub const MAX_TRACKS: usize = 64;

struct Cursor<'a> {
    d: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, len: usize, what: &'static str) -> Result<&'a [u8], Error> {
        let end = self.at.checked_add(len).ok_or(Error::Truncated(what))?;
        let bytes = self.d.get(self.at..end).ok_or(Error::Truncated(what))?;
        self.at = end;
        Ok(bytes)
    }

    fn u8(&mut self, what: &'static str) -> Result<u8, Error> {
        Ok(self.take(1, what)?[0])
    }

    fn data(&mut self) -> Result<u8, Error> {
        let b = self.u8("event data")?;
        if b >= 128 {
            return Err(Error::Invalid("status in event data"));
        }
        Ok(b)
    }

    fn vlq(&mut self) -> Result<u32, Error> {
        let mut v = 0;
        for _ in 0..4 {
            let b = self.u8("a variable length quantity")?;
            v = v << 7 | u32::from(b & 127);
            if b & 128 == 0 {
                return Ok(v);
            }
        }
        Err(Error::Invalid(
            "variable length quantity exceeds four bytes",
        ))
    }
}

#[derive(Clone, Copy)]
enum Raw {
    Event(u8, Kind),
    Tempo(u32),
    End,
}

/// Parse format 0/1 PPQN MIDI. Unsupported timing/formats and malformed lengths,
/// statuses and data return errors. Tempo integration uses integer rational time
/// and one sorted pass, with no per-event scan of the tempo map.
pub fn parse(d: &[u8]) -> Result<Song, Error> {
    if d.len() > MAX_BYTES {
        return Err(Error::Limit);
    }
    if d.len() < 14 || &d[..4] != b"MThd" {
        return Err(Error::NotMidi);
    }
    let mut file = Cursor { d, at: 4 };
    let len = u32::from_be_bytes(file.take(4, "header length")?.try_into().unwrap()) as usize;
    if len < 6 {
        return Err(Error::Invalid("short header"));
    }
    let h = file.take(len, "header")?;
    let format = u16::from_be_bytes([h[0], h[1]]);
    let tracks = usize::from(u16::from_be_bytes([h[2], h[3]]));
    let division = u16::from_be_bytes([h[4], h[5]]);
    if format > 1 || tracks == 0 || (format == 0 && tracks != 1) {
        return Err(Error::Invalid("format or track count"));
    }
    if tracks > MAX_TRACKS {
        return Err(Error::Limit);
    }
    if division & 0x8000 != 0 {
        return Err(Error::Smpte);
    }
    if division == 0 {
        return Err(Error::Invalid("zero division"));
    }
    let mut raw = Vec::new();
    let mut event_count = 0;
    for _ in 0..tracks {
        if file.take(4, "track header")? != b"MTrk" {
            return Err(Error::Invalid("track header"));
        }
        let len = u32::from_be_bytes(file.take(4, "track length")?.try_into().unwrap()) as usize;
        let mut c = Cursor {
            d: file.take(len, "track")?,
            at: 0,
        };
        let mut tick = 0u64;
        let mut running = 0;
        let mut ended = false;
        while c.at < c.d.len() {
            event_count += 1;
            if event_count > MAX_EVENTS {
                return Err(Error::Limit);
            }
            tick += u64::from(c.vlq()?);
            let b = c.u8("event status")?;
            let status = if b < 128 {
                if running == 0 {
                    return Err(Error::Invalid("missing running status"));
                }
                c.at -= 1;
                running
            } else {
                running = if b < 0xf0 { b } else { 0 };
                b
            };
            let channel = status & 15;
            let event = match status >> 4 {
                8 => {
                    let note = c.data()?;
                    c.data()?;
                    Some(Kind::NoteOff { note })
                }
                9 => {
                    let note = c.data()?;
                    let velocity = c.data()?;
                    Some(if velocity == 0 {
                        Kind::NoteOff { note }
                    } else {
                        Kind::NoteOn { note, velocity }
                    })
                }
                10 => {
                    c.data()?;
                    c.data()?;
                    None
                }
                11 => Some(Kind::Controller {
                    number: c.data()?,
                    value: c.data()?,
                }),
                12 => Some(Kind::ProgramChange { program: c.data()? }),
                13 => {
                    c.data()?;
                    None
                }
                14 => Some(Kind::PitchBend {
                    value: u16::from(c.data()?) | u16::from(c.data()?) << 7,
                }),
                _ => {
                    match status {
                        0xff => {
                            let kind = c.u8("meta type")?;
                            let len = c.vlq()? as usize;
                            let bytes = c.take(len, "meta data")?;
                            match kind {
                                0x51 => {
                                    if bytes.len() != 3 {
                                        return Err(Error::Invalid("tempo size"));
                                    }
                                    let tempo =
                                        u32::from_be_bytes([0, bytes[0], bytes[1], bytes[2]]);
                                    if tempo == 0 {
                                        return Err(Error::Invalid("zero tempo"));
                                    }
                                    raw.push((tick, Raw::Tempo(tempo)));
                                }
                                0x2f => {
                                    if len != 0 {
                                        return Err(Error::Invalid("end-of-track size"));
                                    }
                                    raw.push((tick, Raw::End));
                                    ended = true;
                                    break;
                                }
                                _ => {}
                            }
                        }
                        0xf0 | 0xf7 => {
                            let len = c.vlq()? as usize;
                            c.take(len, "system exclusive data")?;
                        }
                        _ => return Err(Error::Invalid("system status")),
                    }
                    None
                }
            };
            if let Some(kind) = event {
                raw.push((tick, Raw::Event(channel, kind)));
            }
        }
        if !ended {
            return Err(Error::Invalid("missing end-of-track"));
        }
    }
    // Stable sorting preserves track/file order for simultaneous events and tempo changes.
    raw.sort_by_key(|e| e.0);
    let mut song = Song {
        tracks,
        ..Default::default()
    };
    let (mut last_tick, mut tempo, mut scaled_us) = (0, 500_000u32, 0u128);
    let denominator = u128::from(division) * 1000;
    for (tick, kind) in raw {
        scaled_us += u128::from(tick - last_tick) * u128::from(tempo);
        last_tick = tick;
        let ms =
            u32::try_from((scaled_us + denominator / 2) / denominator).map_err(|_| Error::Limit)?;
        song.duration_ms = ms;
        match kind {
            Raw::Event(channel, kind) => song.events.push(Event { ms, channel, kind }),
            Raw::Tempo(t) => tempo = t,
            Raw::End => {}
        }
    }
    Ok(song)
}
