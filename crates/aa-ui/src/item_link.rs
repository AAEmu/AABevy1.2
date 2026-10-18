//! Native 1.2 item-link transport (394df700 encode, 394df7f0 decode, 394e4ef0 format).
//! This transports a caller-supplied native item record; it does not invent instance fields.

pub const NATIVE_ITEM_BYTES: usize = 192;
pub const ITEM_LINK_CHARS: usize = 256;
const ALPHABET: &[u8; 64] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz*+";

/// Encode the exact 192-byte native item record, preserving every supplied byte.
pub fn encode_payload(item: &[u8; NATIVE_ITEM_BYTES]) -> String {
    let mut out = String::with_capacity(ITEM_LINK_CHARS);
    for b in item.chunks_exact(3) {
        let bits = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for shift in [18, 12, 6, 0] {
            out.push(ALPHABET[((bits >> shift) & 63) as usize] as char);
        }
    }
    out
}

/// Decode a canonical 1.2 payload. Invalid alphabet, padding and non-192-byte forms fail.
pub fn decode_payload(encoded: &str) -> Option<[u8; NATIVE_ITEM_BYTES]> {
    if encoded.len() != ITEM_LINK_CHARS {
        return None;
    }
    let mut out = [0u8; NATIVE_ITEM_BYTES];
    for (src, dst) in encoded
        .as_bytes()
        .chunks_exact(4)
        .zip(out.chunks_exact_mut(3))
    {
        let mut bits = 0u32;
        for c in src {
            bits = (bits << 6) | ALPHABET.iter().position(|v| v == c)? as u32;
        }
        dst.copy_from_slice(&[(bits >> 16) as u8, (bits >> 8) as u8, bits as u8]);
    }
    Some(out)
}

/// Native chat formatting: zero the instance ID and set count to one before transport.
/// Item type is the little-endian u32 at +8; grade is the byte at +12.
pub fn format(item: &[u8; NATIVE_ITEM_BYTES]) -> String {
    let mut item = *item;
    item[..8].fill(0);
    item[16..20].copy_from_slice(&1u32.to_le_bytes());
    let item_type = u32::from_le_bytes(item[8..12].try_into().unwrap());
    format!("|i{item_type},{},{};", item[12], encode_payload(&item))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_alphabet_and_msb_bit_order_are_preserved() {
        let mut bytes = [0u8; NATIVE_ITEM_BYTES];
        bytes[..3].copy_from_slice(&[0, 0x10, 0x83]);
        bytes[3..6].copy_from_slice(&[0xfb, 0xff, 0xff]);
        let text = encode_payload(&bytes);
        assert_eq!(&text[..8], "0123*+++");
        assert_eq!(decode_payload(&text), Some(bytes));
    }
    #[test]
    fn transport_round_trips_all_record_bytes_and_formatter_normalizes_only_native_fields() {
        let bytes = std::array::from_fn(|i| i as u8);
        assert_eq!(decode_payload(&encode_payload(&bytes)), Some(bytes));
        let text = format(&bytes);
        let body = text.strip_prefix("|i").unwrap().strip_suffix(';').unwrap();
        let mut parts = body.splitn(3, ',');
        assert_eq!(
            parts.next().unwrap(),
            u32::from_le_bytes([8, 9, 10, 11]).to_string()
        );
        assert_eq!(parts.next(), Some("12"));
        let actual = decode_payload(parts.next().unwrap()).unwrap();
        let mut expected = bytes;
        expected[..8].fill(0);
        expected[16..20].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(actual, expected);
    }
    #[test]
    fn truncated_legacy_and_bad_alphabet_fail_without_padding_or_fabricated_bytes() {
        assert!(decode_payload(&"0".repeat(235)).is_none());
        assert!(decode_payload(&"=".repeat(256)).is_none());
        assert!(decode_payload(&"0".repeat(257)).is_none());
        assert!(decode_payload(&"é".repeat(128)).is_none());
    }
}
