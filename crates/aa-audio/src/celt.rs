//! The original game's one CELT subsound: pre-Opus 0.11, 44.1 kHz mono, 512-frame packets.
//! Only this module crosses the private, statically linked C bridge. A decoder owns its mode
//! and history for one complete subsound; no FMOD DLL, device, encoder or concealment is used.

use std::{ffi::c_void, ptr::NonNull};

use crate::fsb5::{Error, Result, Subsound};

pub(crate) const BITSTREAM_VERSION: u32 = 0x8000_0010;
const FRAME: usize = 512;
const MAX_PACKET: usize = 512;
const MAGIC: [u8; 4] = [0x17, 0xc3, 0x0d, 0xf3];

unsafe extern "C" {
    fn aa_celt011_create() -> *mut c_void;
    fn aa_celt011_destroy(decoder: *mut c_void);
    fn aa_celt011_decode(
        decoder: *mut c_void,
        data: *const u8,
        length: i32,
        pcm: *mut i16,
        pcm_samples: usize,
    ) -> i32;
}

fn bad(message: impl std::fmt::Display) -> Error {
    Error::Other(format!("celt 0.11: {message}"))
}

struct Decoder(NonNull<c_void>);

impl Decoder {
    fn new() -> Result<Self> {
        // SAFETY: no arguments; the bridge returns null or one uniquely owned mode/decoder.
        NonNull::new(unsafe { aa_celt011_create() })
            .map(Self)
            .ok_or_else(|| bad("cannot create 44100/512 mono decoder"))
    }

    fn packet(&mut self, packet: &[u8], pcm: &mut [i16; FRAME]) -> Result<()> {
        if !(2..=MAX_PACKET).contains(&packet.len()) {
            return Err(bad("packet length outside 2..=512"));
        }
        // SAFETY: self owns a live decoder, exclusively borrowed for this call. The input is
        // readable for the checked i32 length, and pcm is 512 initialized, writable i16s.
        // C keeps neither pointer and writes exactly one mono 512-sample frame. The bridge
        // rechecks both lengths and rejects null/short data before upstream concealment.
        let status = unsafe {
            aa_celt011_decode(
                self.0.as_ptr(),
                packet.as_ptr(),
                packet.len() as i32,
                pcm.as_mut_ptr(),
                pcm.len(),
            )
        };
        if status != 0 {
            return Err(bad(format_args!("packet decode returned {status}")));
        }
        Ok(())
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        // SAFETY: the handle came from create, remains uniquely owned, and is freed once.
        unsafe { aa_celt011_destroy(self.0.as_ptr()) };
    }
}

/// Validate every record before entering C or reserving the decoded output. The declared sample
/// count bounds the number of packets; only up to 31 terminal zero alignment bytes may remain.
fn packets(data: &[u8], frames: usize) -> Result<Vec<&[u8]>> {
    if frames == 0 || !frames.is_multiple_of(FRAME) || frames / FRAME > data.len() / 10 {
        return Err(bad("sample count does not fit complete 512-sample packets"));
    }
    let mut out = Vec::with_capacity(frames / FRAME);
    let mut at = 0usize;
    for i in 0..frames / FRAME {
        let header = data
            .get(at..at + 8)
            .ok_or_else(|| bad(format_args!("truncated packet {i} header")))?;
        if header[..4] != MAGIC {
            return Err(bad(format_args!("bad packet {i} marker at {at}")));
        }
        let len = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
        if !(2..=MAX_PACKET).contains(&len) {
            return Err(bad(format_args!("packet {i} length {len} outside 2..=512")));
        }
        at += 8;
        let end = at
            .checked_add(len)
            .ok_or_else(|| bad("packet offset overflow"))?;
        out.push(
            data.get(at..end)
                .ok_or_else(|| bad(format_args!("truncated packet {i} payload")))?,
        );
        at = end;
    }
    let tail = &data[at..];
    if tail.len() > 31 || tail.iter().any(|&v| v != 0) {
        return Err(bad("extra packets or invalid terminal alignment padding"));
    }
    Ok(out)
}

pub(crate) fn decode(sub: &Subsound, data: &[u8]) -> Result<Vec<i16>> {
    if sub.channels != 1 || sub.frequency != 44_100 {
        return Err(bad(format_args!(
            "unsupported {} Hz / {} channels; expected 44100 Hz mono",
            sub.frequency, sub.channels
        )));
    }
    let frames = sub.samples as usize;
    let packets = packets(data, frames)?;
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(frames)
        .map_err(|e| bad(format_args!("PCM allocation: {e}")))?;
    samples.resize(frames, 0);
    let mut decoder = Decoder::new()?;
    for (packet, output) in packets.into_iter().zip(samples.as_chunks_mut::<FRAME>().0) {
        decoder.packet(packet, output)?;
    }
    Ok(samples)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn framed(payload: &[u8]) -> Vec<u8> {
        let mut data = MAGIC.to_vec();
        data.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        data.extend_from_slice(payload);
        data
    }

    #[test]
    fn framing_bounds_complete_records_and_terminal_alignment() {
        let data = framed(&[0x66, 1, 2, 3]);
        assert_eq!(packets(&data, FRAME).unwrap(), [&[0x66, 1, 2, 3][..]]);
        let mut padded = data.clone();
        padded.extend_from_slice(&[0; 31]);
        assert_eq!(packets(&padded, FRAME).unwrap().len(), 1);
        padded.push(0);
        assert!(packets(&padded, FRAME).is_err());
        let mut nonzero = data.clone();
        nonzero.push(1);
        assert!(packets(&nonzero, FRAME).is_err());
        for length in 0..data.len() {
            assert!(
                packets(&data[..length], FRAME).is_err(),
                "truncation {length}"
            );
        }
        for count in [0, 1, 511, 513, 1024, u32::MAX as usize] {
            assert!(packets(&data, count).is_err(), "sample count {count}");
        }
        let mut bad_magic = data.clone();
        bad_magic[0] ^= 1;
        assert!(packets(&bad_magic, FRAME).is_err());
        for n in [0u32, 1, 513, u32::MAX] {
            let mut bad_length = data.clone();
            bad_length[4..8].copy_from_slice(&n.to_le_bytes());
            assert!(packets(&bad_length, FRAME).is_err(), "length {n}");
        }
        let mut extra = data.clone();
        extra.extend_from_slice(&data);
        assert!(packets(&extra, FRAME).is_err());
        assert_eq!(packets(&extra, 2 * FRAME).unwrap().len(), 2);
    }

    #[test]
    fn unsupported_metadata_and_short_packets_do_not_enter_the_decoder() {
        let mut sub = Subsound {
            name: None,
            frequency: 24_000,
            channels: 1,
            samples: 512,
            data_start: 0,
            data_len: 10,
            loop_points: None,
            vorbis_crc: None,
            chunks: vec![],
        };
        let data = framed(&[0, 0]);
        assert!(decode(&sub, &data).is_err());
        sub.frequency = 44_100;
        sub.channels = 2;
        assert!(decode(&sub, &data).is_err());
        let mut decoder = Decoder::new().unwrap();
        for payload in [&[][..], &[0][..], &[0; 513][..]] {
            assert!(decoder.packet(payload, &mut [0; FRAME]).is_err());
        }
    }

    #[test]
    fn unsupported_packet_modes_propagate_decoder_errors() {
        let mut errors = 0;
        for first in 0..=255 {
            let mut decoder = Decoder::new().unwrap();
            let mut payload = [0; 186];
            payload[0] = first;
            if decoder.packet(&payload, &mut [0; FRAME]).is_err() {
                errors += 1;
            }
        }
        assert!(errors > 0, "upstream packet errors must propagate to Rust");
    }
}
