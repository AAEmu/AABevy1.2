//! FMOD's Vorbis streams inside FSB5.
//!
//! FSB5 stores only the audio packets of a Vorbis stream, each as `u16 length, bytes`, and no
//! headers. FMOD always encodes with block sizes 256 and 2048, so the identification header is
//! rebuilt from the channel count and rate; the comment header is not needed for decoding; the
//! setup header (codebooks, floors, residues, modes) is one of a few dozen that FMOD's encoder
//! writes, selected by the CRC32 in the sample's Vorbis chunk. [`setup_header`] looks it up in
//! `data/vorbis_setup.bin` (see `data/vorbis_setup.LICENSE`). The packets then go to lewton.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use lewton::audio::{PreviousWindowRight, read_audio_packet_generic};
use lewton::header::{IdentHeader, SetupHeader, read_header_ident, read_header_setup};
use lewton::samples::InterleavedSamples;

use crate::fsb5::{Error, Result, Subsound};

/// log2 of FMOD's short and long block sizes (256 and 2048).
const BLOCKSIZES: (u8, u8) = (8, 11);

static TABLE: &[u8] = include_bytes!("../data/vorbis_setup.bin");

fn table() -> &'static HashMap<u32, &'static [u8]> {
    static MAP: OnceLock<HashMap<u32, &'static [u8]>> = OnceLock::new();
    MAP.get_or_init(|| {
        let u32_at = |o: usize| u32::from_le_bytes(TABLE[o..o + 4].try_into().unwrap());
        let mut map = HashMap::new();
        let mut at = 4;
        for _ in 0..u32_at(0) {
            let (crc, len) = (u32_at(at), u32_at(at + 4) as usize);
            map.insert(crc, &TABLE[at + 8..at + 8 + len]);
            at += 8 + len;
        }
        map
    })
}

/// The Vorbis setup header packet for a sample's CRC32, if FMOD's encoder is known to write it.
pub fn setup_header(crc: u32) -> Option<&'static [u8]> {
    table().get(&crc).copied()
}

/// How many setup headers the crate carries.
pub fn known_setups() -> usize {
    table().len()
}

/// The Vorbis identification header packet (30 bytes) for a stream.
pub fn ident_header(channels: u8, rate: u32) -> Vec<u8> {
    let mut p = Vec::with_capacity(30);
    p.push(1);
    p.extend_from_slice(b"vorbis");
    p.extend_from_slice(&0u32.to_le_bytes()); // version
    p.push(channels);
    p.extend_from_slice(&rate.to_le_bytes());
    p.extend_from_slice(&[0u8; 12]); // bitrate max, nominal, min
    p.push(BLOCKSIZES.0 | (BLOCKSIZES.1 << 4));
    p.push(1); // framing
    p
}

struct Headers {
    ident: IdentHeader,
    setup: SetupHeader,
}

/// Parsing a setup header builds its Huffman tables (milliseconds), and a handful of them serve
/// thousands of sounds: keep the parsed headers.
fn headers(crc: u32, channels: u8, rate: u32) -> Result<Arc<Headers>> {
    static CACHE: OnceLock<Mutex<HashMap<(u32, u8, u32), Arc<Headers>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(h) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&(crc, channels, rate)) {
        return Ok(h.clone());
    }
    let setup_bytes = setup_header(crc).ok_or(Error::UnknownSetup(crc))?;
    let ident = read_header_ident(&ident_header(channels, rate)).map_err(|e| Error::Vorbis(format!("identification header: {e:?}")))?;
    let setup = read_header_setup(setup_bytes, channels, BLOCKSIZES).map_err(|e| Error::Vorbis(format!("setup header: {e:?}")))?;
    let h = Arc::new(Headers { ident, setup });
    cache.lock().unwrap_or_else(|e| e.into_inner()).insert((crc, channels, rate), h.clone());
    Ok(h)
}

/// Splits `data` into its packets: `u16 length, packet` until the end or a zero length.
pub fn packets(data: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut at = 0;
    std::iter::from_fn(move || {
        let len = u16::from_le_bytes(data.get(at..at + 2)?.try_into().ok()?) as usize;
        if len == 0 {
            return None;
        }
        let p = data.get(at + 2..at + 2 + len)?;
        at += 2 + len;
        Some(p)
    })
}

/// Decodes a Vorbis subsound to interleaved 16 bit samples, cut to the length the header promises.
pub fn decode(sub: &Subsound, data: &[u8]) -> Result<Vec<i16>> {
    let crc = sub.vorbis_crc.ok_or_else(|| Error::Other("Vorbis subsound without a Vorbis chunk".into()))?;
    let h = headers(crc, sub.channels, sub.frequency)?;
    let channels = sub.channels as usize;
    let mut pwr = PreviousWindowRight::new();
    let mut out: Vec<i16> = Vec::with_capacity((sub.samples as usize + 2048) * channels);
    for (i, packet) in packets(data).enumerate() {
        let block: InterleavedSamples<i16> = read_audio_packet_generic(&h.ident, &h.setup, packet, &mut pwr)
            .map_err(|e| Error::Vorbis(format!("packet {i}: {e:?}")))?;
        out.extend_from_slice(&block.samples);
    }
    out.truncate(sub.samples as usize * channels);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_loads_and_headers_parse() {
        assert_eq!(known_setups(), 23);
        for (&crc, bytes) in table() {
            assert_eq!(bytes[0], 5, "setup packet type");
            assert_eq!(&bytes[1..7], b"vorbis");
            // a setup header belongs to a channel count (stereo ones use channel coupling, which a mono
            // stream cannot): at least stereo must parse
            headers(crc, 2, 44100).unwrap_or_else(|e| panic!("crc {crc:#x}: {e}"));
        }
    }

    #[test]
    fn ident_header_is_valid() {
        let h = read_header_ident(&ident_header(2, 22050)).unwrap();
        assert_eq!((h.audio_channels, h.audio_sample_rate, h.blocksize_0, h.blocksize_1), (2, 22050, 8, 11));
    }

    #[test]
    fn splits_packets() {
        let data = [2, 0, 1, 2, 1, 0, 9, 0, 0, 5, 0, 1];
        let got: Vec<&[u8]> = packets(&data).collect();
        assert_eq!(got, [&[1u8, 2][..], &[9][..]]);
        // a length that runs past the end stops the stream instead of panicking
        assert_eq!(packets(&[9, 0, 1]).count(), 0);
    }
}
