//! Raw RFC 1951 compression used inside DD04. No zlib/gzip wrapper is present.
use crate::wire::{Result, WireError};
use flate2::{Decompress, FlushDecompress, Status};

/// Application memory budget, not a length field in the 1.2 protocol.
pub const MAX_INFLATED_BATCH_BYTES: usize = 16 * 1024 * 1024;

pub(crate) fn inflate_batch(data: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = Decompress::new(false);
    let mut out = Vec::new();
    let mut chunk = [0u8; 16 * 1024];
    loop {
        let input_before = decoder.total_in();
        let output_before = decoder.total_out();
        let status = decoder
            .decompress(
                &data[input_before as usize..],
                &mut chunk,
                FlushDecompress::None,
            )
            .map_err(|e| WireError::Invalid(format!("invalid DD04 raw DEFLATE: {e}")))?;
        let written = (decoder.total_out() - output_before) as usize;
        if written > MAX_INFLATED_BATCH_BYTES - out.len() {
            return Err(WireError::Invalid(
                "DD04 exceeds 16 MiB inflated limit".into(),
            ));
        }
        out.extend_from_slice(&chunk[..written]);
        if status == Status::StreamEnd {
            if decoder.total_in() as usize != data.len() {
                return Err(WireError::Invalid(
                    "trailing bytes after DD04 DEFLATE stream".into(),
                ));
            }
            return Ok(out);
        }
        if decoder.total_in() == input_before && written == 0 {
            return Err(WireError::Invalid("truncated DD04 DEFLATE stream".into()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{LEVEL_COMPRESSED, MAGIC, ServerMsg};

    const FIXED: &[u8] = include_bytes!("../tests/fixtures/deflate/fixed.deflate");
    const DYNAMIC: &[u8] = include_bytes!("../tests/fixtures/deflate/dynamic.deflate");
    const MIXED: &[u8] = include_bytes!("../tests/fixtures/deflate/mixed.deflate");
    const FIXED_RAW: &[u8] = include_bytes!("../tests/fixtures/deflate/fixed.raw");
    const DYNAMIC_RAW: &[u8] = include_bytes!("../tests/fixtures/deflate/dynamic.raw");
    const MIXED_RAW: &[u8] = include_bytes!("../tests/fixtures/deflate/mixed.raw");

    fn frame(count: u16, data: &[u8]) -> Vec<u8> {
        let mut frame = vec![MAGIC, LEVEL_COMPRESSED];
        frame.extend(count.to_le_bytes());
        frame.extend_from_slice(data);
        frame
    }
    fn batch(count: u16, data: &[u8]) -> Vec<ServerMsg> {
        let ServerMsg::Batch(messages) = ServerMsg::decode(&frame(count, data)).unwrap() else {
            panic!("not a decoded batch")
        };
        messages
    }
    fn points(index: i32) -> ServerMsg {
        ServerMsg::UnitPoints {
            obj_id: index as u32 + 1,
            hp: 300 + index * 31,
            mp: 20 + index % 71,
        }
    }

    #[test]
    fn independent_python_zlib_fixtures_cover_fixed_dynamic_and_mixed_blocks() {
        assert_eq!((FIXED[0] >> 1) & 3, 1);
        assert_eq!((DYNAMIC[0] >> 1) & 3, 2);
        assert_eq!((MIXED[0] >> 1) & 3, 0);
        assert_eq!(&MIXED[5 + FIXED_RAW.len()..], DYNAMIC);
        for (compressed, expected) in [
            (FIXED, FIXED_RAW),
            (DYNAMIC, DYNAMIC_RAW),
            (MIXED, MIXED_RAW),
        ] {
            assert_eq!(inflate_batch(compressed).unwrap(), expected);
        }
        assert!(
            inflate_batch(include_bytes!("../tests/fixtures/deflate/empty.deflate"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn every_stream_truncation_and_missing_final_block_fails_without_partial_packets() {
        for data in [FIXED, DYNAMIC, MIXED] {
            for cut in 0..data.len() {
                assert!(inflate_batch(&data[..cut]).is_err(), "cut {cut}");
                assert!(ServerMsg::decode(&frame(260, &data[..cut])).is_err());
            }
        }
        // The first mixed stored block itself is complete, but it is not the final block.
        assert!(inflate_batch(&MIXED[..5 + FIXED_RAW.len()]).is_err());
        assert!(inflate_batch(&[0, 0, 0, 255, 255]).is_err());
    }

    #[test]
    fn reserved_blocks_bad_lengths_wrappers_and_trailing_streams_fail_strictly() {
        for invalid in [
            &[][..],
            &[0x07][..],                               // Reserved BTYPE=11.
            &[1, 3, 0, 0, 0, 1, 2, 3][..],             // Bad stored LEN/NLEN.
            &[0x05, 0, 0, 0][..],                      // Incomplete dynamic tree.
            &[0x78, 0x9c, 0x03, 0, 0, 0, 0, 1][..],    // A zlib-wrapped empty stream.
            &[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3][..], // A gzip header.
        ] {
            assert!(inflate_batch(invalid).is_err(), "{invalid:?}");
            assert!(ServerMsg::decode(&frame(1, invalid)).is_err());
        }
        for data in [FIXED, DYNAMIC, MIXED] {
            for suffix in [&[0][..], &[3, 0][..], FIXED] {
                let mut trailing = data.to_vec();
                trailing.extend_from_slice(suffix);
                assert!(inflate_batch(&trailing).is_err());
                assert!(ServerMsg::decode(&frame(3, &trailing)).is_err());
            }
        }
    }

    #[test]
    fn exactly_sixteen_mib_is_accepted_and_one_byte_over_budget_is_rejected() {
        let at_limit = include_bytes!("../tests/fixtures/deflate/at-limit.deflate");
        let over_limit = include_bytes!("../tests/fixtures/deflate/over-limit.deflate");
        let raw = inflate_batch(at_limit).unwrap();
        assert_eq!(raw.len(), MAX_INFLATED_BATCH_BYTES);
        assert!(raw.iter().all(|v| *v == 0));
        let error = inflate_batch(over_limit).unwrap_err().to_string();
        assert!(error.contains("16 MiB"));
        assert!(ServerMsg::decode(&frame(1, over_limit)).is_err());
    }

    #[test]
    fn huffman_frames_decode_the_complete_ordered_packet_sequence() {
        let fixed = vec![points(0), ServerMsg::Snow(true), points(1)];
        assert_eq!(batch(3, FIXED), fixed);
        let mut dynamic: Vec<_> = (0..256).map(points).collect();
        dynamic.push(ServerMsg::Snow(true));
        assert_eq!(batch(257, DYNAMIC), dynamic);
        let mut expected = fixed;
        expected.extend(dynamic);
        assert_eq!(batch(260, MIXED), expected);
        assert!(batch(0, include_bytes!("../tests/fixtures/deflate/empty.deflate")).is_empty());
    }

    #[test]
    fn compressed_unknown_packet_is_skipped_and_a_truncated_one_ends_the_batch() {
        let unknown = include_bytes!("../tests/fixtures/deflate/unknown-opcode.deflate");
        let messages = batch(3, unknown);
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0], points(0));
        assert!(matches!(
            messages[1],
            ServerMsg::Unhandled {
                level: LEVEL_COMPRESSED,
                opcode: 0x7fff,
                len: 3,
            }
        ));
        assert_eq!(messages[2], ServerMsg::Snow(true));
        // Compression is complete and valid here; its final game packet is truncated.
        let truncated = include_bytes!("../tests/fixtures/deflate/truncated-packet.deflate");
        assert!(matches!(batch(2,truncated).as_slice(),[ServerMsg::Snow(true),ServerMsg::DecodeFailed{..}]));
    }
}
