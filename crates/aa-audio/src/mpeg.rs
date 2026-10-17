//! MPEG audio subsounds (FSB5 mode 11).
//!
//! The data of a subsound is a plain run of MPEG frames without padding between them (the voice
//! banks are MPEG-2 layer II, 24 kHz mono, 96 kbit/s: 576 byte frames of 1152 samples), followed by
//! zeros up to the next 16 byte boundary. The frames are cut here and decoded one by one with
//! symphonia's layer 1/2/3 decoder.

use symphonia_bundle_mp3::MpaDecoder;
use symphonia_core::codecs::audio::well_known::{CODEC_ID_MP1, CODEC_ID_MP2, CODEC_ID_MP3};
use symphonia_core::codecs::audio::{AudioCodecParameters, AudioDecoder, AudioDecoderOptions};
use symphonia_core::packet::PacketRef;
use symphonia_core::units::{Duration, Timestamp};

use crate::fsb5::{Error, Result, Subsound};

/// What a frame header says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameHeader {
    /// 1, 2 or 3.
    pub layer: u8,
    pub rate: u32,
    pub channels: u8,
    /// Bytes, header included.
    pub size: usize,
    /// Samples per channel.
    pub samples: usize,
}

const BITRATES: [[[u16; 14]; 3]; 2] = [
    // MPEG-1: layers I, II, III
    [
        [32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448],
        [32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384],
        [32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320],
    ],
    // MPEG-2 and 2.5
    [
        [32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256],
        [8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],
        [8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],
    ],
];

/// Parses the 4 byte header at the start of `b`.
pub fn frame_header(b: &[u8]) -> Option<FrameHeader> {
    if b.len() < 4 || b[0] != 0xff || b[1] & 0xe0 != 0xe0 {
        return None;
    }
    let version = (b[1] >> 3) & 3; // 3 = MPEG-1, 2 = MPEG-2, 0 = MPEG-2.5
    let layer = match (b[1] >> 1) & 3 {
        3 => 1,
        2 => 2,
        1 => 3,
        _ => return None,
    };
    let bitrate_index = (b[2] >> 4) as usize;
    let rate_index = ((b[2] >> 2) & 3) as usize;
    let padding = ((b[2] >> 1) & 1) as usize;
    if version == 1 || bitrate_index == 0 || bitrate_index == 15 || rate_index == 3 {
        return None;
    }
    let mpeg1 = version == 3;
    let rates = match version {
        3 => [44100, 48000, 32000],
        2 => [22050, 24000, 16000],
        _ => [11025, 12000, 8000],
    };
    let rate = rates[rate_index];
    let bitrate = BITRATES[if mpeg1 { 0 } else { 1 }][layer as usize - 1][bitrate_index - 1] as usize * 1000;
    let (size, samples) = match layer {
        1 => ((12 * bitrate / rate as usize + padding) * 4, 384),
        2 => (144 * bitrate / rate as usize + padding, 1152),
        _ if mpeg1 => (144 * bitrate / rate as usize + padding, 1152),
        _ => (72 * bitrate / rate as usize + padding, 576),
    };
    Some(FrameHeader { layer, rate, channels: if (b[3] >> 6) == 3 { 1 } else { 2 }, size, samples })
}

/// Decodes a run of MPEG frames to interleaved 16 bit samples, cut to `sub.samples` frames.
pub fn decode(sub: &Subsound, data: &[u8]) -> Result<Vec<i16>> {
    let first = frame_header(data).ok_or_else(|| Error::Other("MPEG subsound does not start with a frame header".into()))?;
    let codec = match first.layer {
        1 => CODEC_ID_MP1,
        2 => CODEC_ID_MP2,
        _ => CODEC_ID_MP3,
    };
    let mut params = AudioCodecParameters::new();
    params.codec = codec;
    params.sample_rate = Some(first.rate);
    let mut dec = MpaDecoder::try_new(&params, &AudioDecoderOptions::default()).map_err(|e| Error::Other(format!("mpeg decoder: {e}")))?;

    let channels = sub.channels as usize;
    let want = sub.samples as usize * channels;
    let mut out: Vec<i16> = Vec::with_capacity(want + 1152 * channels);
    let mut tmp: Vec<i16> = Vec::new();
    let mut at = 0;
    let mut ts = 0i64;
    while out.len() < want {
        let Some(h) = data.get(at..).and_then(frame_header) else { break };
        let Some(frame) = data.get(at..at + h.size) else { break };
        let packet = PacketRef::new(0, Timestamp::new(ts), Duration::new(h.samples as u64), frame);
        let buf = dec.decode_ref(&packet).map_err(|e| Error::Other(format!("mpeg frame at {at}: {e}")))?;
        buf.copy_to_vec_interleaved::<i16>(&mut tmp);
        if h.channels as usize == channels {
            out.extend_from_slice(&tmp);
        } else if h.channels == 1 {
            // mono frames in a stereo subsound: duplicate
            out.extend(tmp.iter().flat_map(|&s| [s, s]));
        } else {
            // stereo frames in a mono subsound: average
            out.extend(tmp.as_chunks::<2>().0.iter().map(|p| ((p[0] as i32 + p[1] as i32) / 2) as i16));
        }
        at += h.size;
        ts += h.samples as i64;
    }
    out.truncate(want);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_voice_bank_header() {
        // 0xFFF5A4C4: MPEG-2 layer II, 96 kbit/s, 24 kHz, mono (the first frame of x2voice_combat_bank02)
        let h = frame_header(&[0xff, 0xf5, 0xa4, 0xc4]).unwrap();
        assert_eq!(h, FrameHeader { layer: 2, rate: 24000, channels: 1, size: 576, samples: 1152 });
        // MPEG-1 layer III, 128 kbit/s, 44.1 kHz, stereo: 417 bytes
        let h = frame_header(&[0xff, 0xfb, 0x90, 0x00]).unwrap();
        assert_eq!((h.layer, h.rate, h.channels, h.size, h.samples), (3, 44100, 2, 417, 1152));
        assert!(frame_header(&[0x12, 0x34, 0x56, 0x78]).is_none());
        assert!(frame_header(&[0xff, 0xfb, 0xf0, 0x00]).is_none());
    }
}
