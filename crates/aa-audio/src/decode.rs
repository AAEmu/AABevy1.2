//! Decoded sound: the format the player and the cache work with.

use crate::fsb5::{Codec, Error, Fsb5, Result};
use crate::{celt, mpeg, vorbis};

/// Decoded audio: interleaved signed 16 bit samples, the common denominator of every codec in the
/// banks (the sources are 16 bit already) at half the memory of floats.
#[derive(Clone, Debug)]
pub struct Pcm {
    pub rate: u32,
    pub channels: u8,
    pub samples: Vec<i16>,
    /// Loop start and end (inclusive) in frames, from the bank.
    pub loop_points: Option<(u32, u32)>,
}

impl Pcm {
    /// Samples per channel.
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1) as usize
    }

    pub fn seconds(&self) -> f32 {
        self.frames() as f32 / self.rate.max(1) as f32
    }

    /// Memory held by the samples.
    pub fn bytes(&self) -> usize {
        self.samples.len() * 2
    }

    /// Root mean square of all samples, 0 to 1.
    pub fn rms(&self) -> f32 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let sum: f64 = self.samples.iter().map(|&s| (s as f64 / 32768.0).powi(2)).sum();
        (sum / self.samples.len() as f64).sqrt() as f32
    }

    /// A 16 bit PCM `.wav` file.
    pub fn to_wav(&self) -> Vec<u8> {
        let data_len = self.samples.len() * 2;
        let mut w = Vec::with_capacity(44 + data_len);
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&(36 + data_len as u32).to_le_bytes());
        w.extend_from_slice(b"WAVEfmt ");
        w.extend_from_slice(&16u32.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes());
        w.extend_from_slice(&(self.channels as u16).to_le_bytes());
        w.extend_from_slice(&self.rate.to_le_bytes());
        w.extend_from_slice(&(self.rate * self.channels as u32 * 2).to_le_bytes());
        w.extend_from_slice(&(self.channels as u16 * 2).to_le_bytes());
        w.extend_from_slice(&16u16.to_le_bytes());
        w.extend_from_slice(b"data");
        w.extend_from_slice(&(data_len as u32).to_le_bytes());
        for s in &self.samples {
            w.extend_from_slice(&s.to_le_bytes());
        }
        w
    }
}

/// Decodes subsound `index` of the bank whose file is `bytes`.
pub fn decode(fsb: &Fsb5, bytes: &[u8], index: usize) -> Result<Pcm> {
    decode_data(fsb, index, fsb.data(bytes, index)?)
}

/// Decodes subsound `index` from its encoded bytes alone (`data` is what [`Fsb5::data`] returns), for
/// callers that read just that part of the bank.
pub fn decode_data(fsb: &Fsb5, index: usize, data: &[u8]) -> Result<Pcm> {
    let sub = fsb.subsounds.get(index).ok_or(Error::NoSubsound(index))?;
    let want = sub.samples as usize * sub.channels as usize;
    let samples = match fsb.codec {
        Codec::Vorbis => vorbis::decode(sub, data)?,
        Codec::Mpeg => mpeg::decode(sub, data)?,
        Codec::Celt => celt::decode(sub, data)?,
        Codec::Pcm16 => data.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).take(want).collect(),
        Codec::Pcm8 => data.iter().map(|&b| ((b as i16) - 128) << 8).take(want).collect(),
        other => return Err(Error::Codec(other)),
    };
    Ok(Pcm { rate: sub.frequency, channels: sub.channels, samples, loop_points: sub.loop_points })
}
