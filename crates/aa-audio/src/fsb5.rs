//! FSB5: the FMOD sound bank container (`.fsb`).
//!
//! Layout (little endian), as written by FMOD Ex 4.4x / Designer and read by python-fsb5 and
//! vgmstream:
//!
//! ```text
//! header (0x3C bytes, version 0 has 4 more)
//!   char[4] "FSB5"   u32 version   u32 sample count   u32 sample header bytes
//!   u32 name table bytes   u32 data bytes   u32 mode (codec)   u32 codec version   u32 flags
//!   u8[16] hash   u8[8] dummy
//! sample headers: one u64 each, optionally followed by chunks
//!   bit 0 more chunks follow   bits 1..4 frequency index   bit 5 channels - 1
//!   bits 6..33 data offset / 16   bits 34..63 sample count
//!   chunk: u32 { bit 0 more chunks, bits 1..24 size, bits 25..31 type } + data
//!     type 1 channels (u8)   2 frequency (u32)   3 loop (u32 start, u32 end)   11 Vorbis (u32 CRC32 of the
//!     setup header, then a seek table)
//! name table: u32 offsets (from the table start), then NUL terminated names
//! data: the samples, 16 byte aligned
//! ```
//!
//! A Vorbis subsound is a run of `u16 length, packet` records without the three Vorbis headers
//! (see [`crate::vorbis`]).

use thiserror::Error;

/// Everything that can go wrong reading or decoding a bank.
#[derive(Debug, Error)]
pub enum Error {
    #[error("not an FSB5 bank")]
    NotFsb5,
    #[error("truncated bank ({0})")]
    Truncated(&'static str),
    #[error("subsound {0} does not exist")]
    NoSubsound(usize),
    #[error("unsupported codec {0:?}")]
    Codec(Codec),
    #[error("no known Vorbis setup header with CRC32 {0:#010x}")]
    UnknownSetup(u32),
    #[error("vorbis: {0}")]
    Vorbis(String),
    #[error("{0}")]
    Other(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// The codec of a whole bank (`mode` in the header).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Codec {
    None,
    Pcm8,
    Pcm16,
    Pcm24,
    Pcm32,
    PcmFloat,
    GcAdpcm,
    ImaAdpcm,
    Vag,
    HeVag,
    Xma,
    Mpeg,
    Celt,
    At9,
    XWma,
    Vorbis,
    Unknown(u32),
}

impl Codec {
    pub fn from_mode(mode: u32) -> Codec {
        match mode {
            0 => Codec::None,
            1 => Codec::Pcm8,
            2 => Codec::Pcm16,
            3 => Codec::Pcm24,
            4 => Codec::Pcm32,
            5 => Codec::PcmFloat,
            6 => Codec::GcAdpcm,
            7 => Codec::ImaAdpcm,
            8 => Codec::Vag,
            9 => Codec::HeVag,
            10 => Codec::Xma,
            11 => Codec::Mpeg,
            12 => Codec::Celt,
            13 => Codec::At9,
            14 => Codec::XWma,
            15 => Codec::Vorbis,
            n => Codec::Unknown(n),
        }
    }
}

/// An extra chunk of a sample header that this reader does not interpret further.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
    pub kind: u8,
    pub data: Vec<u8>,
}

/// One sound of a bank.
#[derive(Clone, Debug)]
pub struct Subsound {
    /// From the name table, when the bank has one.
    pub name: Option<String>,
    pub frequency: u32,
    pub channels: u8,
    /// Length in samples per channel (what the header promises; decoders may deliver a little more).
    pub samples: u32,
    /// Start of the encoded data inside the data section.
    pub data_start: u32,
    pub data_len: u32,
    /// Loop start and end (inclusive) in samples, from the loop chunk.
    pub loop_points: Option<(u32, u32)>,
    /// CRC32 of the Vorbis setup header (chunk 11): selects the header in [`crate::vorbis`].
    pub vorbis_crc: Option<u32>,
    /// Every chunk in file order (including the ones above).
    pub chunks: Vec<Chunk>,
}

/// A parsed bank header. The audio stays in the caller's buffer: [`Fsb5::data`] slices it.
#[derive(Clone, Debug)]
pub struct Fsb5 {
    pub version: u32,
    pub codec: Codec,
    pub subsounds: Vec<Subsound>,
    /// Offset of the data section in the file.
    pub data_offset: usize,
    pub data_size: usize,
}

const FREQUENCIES: [u32; 10] = [0, 8000, 11000, 11025, 16000, 22050, 24000, 32000, 44100, 48000];

struct Cursor<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize, what: &'static str) -> Result<&'a [u8]> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.b.len()).ok_or(Error::Truncated(what))?;
        let s = &self.b[self.at..end];
        self.at = end;
        Ok(s)
    }
    fn u32(&mut self, what: &'static str) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4, what)?.try_into().unwrap()))
    }
    fn u64(&mut self, what: &'static str) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8, what)?.try_into().unwrap()))
    }
}

impl Fsb5 {
    /// Parses the header, the sample headers and the name table of `bytes` (the whole file, or at
    /// least everything before the data section).
    pub fn parse(bytes: &[u8]) -> Result<Fsb5> {
        if bytes.len() < 4 || &bytes[..4] != b"FSB5" {
            return Err(Error::NotFsb5);
        }
        let mut c = Cursor { b: bytes, at: 4 };
        let version = c.u32("header")?;
        let count = c.u32("header")? as usize;
        let sample_header_size = c.u32("header")? as usize;
        let name_table_size = c.u32("header")? as usize;
        let data_size = c.u32("header")? as usize;
        let mode = c.u32("header")?;
        let codec_version = c.u32("codec version")?;
        c.u32("header flags")?;
        if mode == 12 && (version != 1 || codec_version != crate::celt::BITSTREAM_VERSION) {
            return Err(Error::Other(format!("unsupported CELT container {version} / bitstream {codec_version:#010x}")));
        }
        c.take(16 + 8, "header")?;
        if version == 0 {
            c.u32("header")?;
        }
        let headers_at = c.at;
        let names_at = headers_at.checked_add(sample_header_size).ok_or(Error::Truncated("sample headers"))?;
        let data_offset = names_at.checked_add(name_table_size).ok_or(Error::Truncated("name table"))?;
        if count > 1 << 20 || names_at > bytes.len() {
            return Err(Error::Truncated("sample headers"));
        }

        let mut subsounds = Vec::with_capacity(count);
        for _ in 0..count {
            let raw = c.u64("sample header")?;
            let mut more = raw & 1 != 0;
            let freq_index = ((raw >> 1) & 0xf) as usize;
            let mut sub = Subsound {
                name: None,
                frequency: FREQUENCIES.get(freq_index).copied().unwrap_or(0),
                channels: ((raw >> 5) & 1) as u8 + 1,
                samples: (raw >> 34) as u32,
                data_start: (((raw >> 6) & 0x0fff_ffff) * 16) as u32,
                data_len: 0,
                loop_points: None,
                vorbis_crc: None,
                chunks: Vec::new(),
            };
            while more {
                let h = c.u32("sample chunk")?;
                more = h & 1 != 0;
                let size = ((h >> 1) & 0x00ff_ffff) as usize;
                let kind = (h >> 25) as u8;
                let data = c.take(size, "sample chunk data")?;
                let u32_at = |o: usize| data.get(o..o + 4).map(|d| u32::from_le_bytes(d.try_into().unwrap()));
                match kind {
                    1 => {
                        if let Some(&ch) = data.first() {
                            sub.channels = ch;
                        }
                    }
                    2 => {
                        if let Some(f) = u32_at(0) {
                            sub.frequency = f;
                        }
                    }
                    3 => {
                        if let (Some(a), Some(b)) = (u32_at(0), u32_at(4)) {
                            sub.loop_points = Some((a, b));
                        }
                    }
                    11 => sub.vorbis_crc = u32_at(0),
                    _ => {}
                }
                sub.chunks.push(Chunk { kind, data: data.to_vec() });
            }
            subsounds.push(sub);
        }
        if c.at > names_at {
            return Err(Error::Truncated("sample headers overrun"));
        }

        if name_table_size > 0 {
            let table = bytes.get(names_at..data_offset.min(bytes.len())).ok_or(Error::Truncated("name table"))?;
            for (i, sub) in subsounds.iter_mut().enumerate() {
                let Some(off) = table.get(i * 4..i * 4 + 4).map(|d| u32::from_le_bytes(d.try_into().unwrap()) as usize) else {
                    break;
                };
                if let Some(s) = table.get(off..) {
                    let end = s.iter().position(|&b| b == 0).unwrap_or(s.len());
                    sub.name = Some(String::from_utf8_lossy(&s[..end]).into_owned());
                }
            }
        }

        // The data of a sound runs up to the next sound's start (sounds are stored in order, but do not
        // rely on it) and the last one up to the end of the data section.
        let mut starts: Vec<u32> = subsounds.iter().map(|s| s.data_start).collect();
        starts.sort_unstable();
        for sub in &mut subsounds {
            let next = starts.iter().copied().find(|&s| s > sub.data_start).unwrap_or(data_size as u32);
            sub.data_len = next.saturating_sub(sub.data_start);
        }

        Ok(Fsb5 { version, codec: Codec::from_mode(mode), subsounds, data_offset, data_size })
    }

    /// The encoded bytes of subsound `index` out of the file `bytes`.
    pub fn data<'a>(&self, bytes: &'a [u8], index: usize) -> Result<&'a [u8]> {
        let sub = self.subsounds.get(index).ok_or(Error::NoSubsound(index))?;
        let start = self.data_offset + sub.data_start as usize;
        let end = start + sub.data_len as usize;
        bytes.get(start..end.min(bytes.len())).filter(|d| !d.is_empty() || sub.data_len == 0).ok_or(Error::Truncated("sample data"))
    }

    /// Index of the subsound called `name` (case-insensitive).
    pub fn find(&self, name: &str) -> Option<usize> {
        self.subsounds.iter().position(|s| s.name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(name)))
    }

    /// Duration of subsound `index` in seconds.
    pub fn seconds(&self, index: usize) -> f32 {
        self.subsounds.get(index).map_or(0.0, |s| if s.frequency == 0 { 0.0 } else { s.samples as f32 / s.frequency as f32 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hand-built bank: two mono Vorbis-looking subsounds with names, one with a loop chunk.
    pub(crate) fn tiny_bank() -> Vec<u8> {
        let mut headers = Vec::new();
        // sample 0: more chunks, freq index 8 (44100), mono, offset 0, 1000 samples; loop chunk
        let raw0: u64 = 1 | (8 << 1) | (1000u64 << 34);
        headers.extend_from_slice(&raw0.to_le_bytes());
        let loop_chunk: u32 = (8 << 1) | (3 << 25);
        headers.extend_from_slice(&loop_chunk.to_le_bytes());
        headers.extend_from_slice(&10u32.to_le_bytes());
        headers.extend_from_slice(&900u32.to_le_bytes());
        // sample 1: 48000 Hz stereo, data at 32, 500 samples
        let raw1: u64 = (9 << 1) | (1 << 5) | (2 << 6) | (500u64 << 34);
        headers.extend_from_slice(&raw1.to_le_bytes());
        let names = {
            let mut t = Vec::new();
            t.extend_from_slice(&8u32.to_le_bytes());
            t.extend_from_slice(&14u32.to_le_bytes());
            t.extend_from_slice(b"alpha\0");
            t.extend_from_slice(b"beta_2\0");
            while t.len() % 4 != 0 {
                t.push(0);
            }
            t
        };
        let data = vec![0xAB; 64];
        let mut out = Vec::new();
        out.extend_from_slice(b"FSB5");
        for v in [1u32, 2, headers.len() as u32, names.len() as u32, data.len() as u32, 15] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&[0u8; 32]);
        out.extend_from_slice(&headers);
        out.extend_from_slice(&names);
        out.extend_from_slice(&data);
        out
    }

    #[test]
    fn parses_header_samples_and_names() {
        let bank = tiny_bank();
        let fsb = Fsb5::parse(&bank).unwrap();
        assert_eq!(fsb.codec, Codec::Vorbis);
        assert_eq!(fsb.subsounds.len(), 2);
        let (a, b) = (&fsb.subsounds[0], &fsb.subsounds[1]);
        assert_eq!((a.frequency, a.channels, a.samples), (44100, 1, 1000));
        assert_eq!(a.loop_points, Some((10, 900)));
        assert_eq!((b.frequency, b.channels, b.samples), (48000, 2, 500));
        assert_eq!(a.name.as_deref(), Some("alpha"));
        assert_eq!(fsb.find("BETA_2"), Some(1));
        assert_eq!((a.data_start, a.data_len), (0, 32));
        assert_eq!((b.data_start, b.data_len), (32, 32));
        assert_eq!(fsb.data(&bank, 1).unwrap().len(), 32);
        assert!(fsb.data(&bank, 2).is_err());
    }

    #[test]
    fn rejects_garbage() {
        assert!(matches!(Fsb5::parse(b"RIFFxxxx"), Err(Error::NotFsb5)));
        let bank = tiny_bank();
        assert!(Fsb5::parse(&bank[..40]).is_err());
    }
}
