//! Label3929f180 byte grouping and TextModel3922b680 storage/conversion.

use crate::tree::LabelData;

/// Apply a native label string. Refused or identical input leaves the model untouched.
pub(super) fn set(data: &mut LabelData, previous: &str, input: &[u8]) -> Option<String> {
    let input = &input[..input.iter().position(|&c| c == 0).unwrap_or(input.len())];
    if !data.number_only && data.raw.as_deref().unwrap_or(previous.as_bytes()) == input {
        return None;
    }
    let mut raw = Vec::with_capacity(input.len());
    for (i, &byte) in input.iter().enumerate() {
        if data.number_only && i != 0 && (input.len() - i) % 3 == 0 {
            raw.push(b',');
        }
        raw.push(byte);
    }
    if data.raw.as_deref().unwrap_or(previous.as_bytes()) == raw {
        return None;
    }
    // XlStringLen3301b000 validates only the byte structure of the prefix.
    let mut count = 0;
    let mut at = 0;
    while let Some(&lead) = raw.get(at) {
        let length = match lead {
            0..=0x7f => 1,
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => break,
        };
        let Some(bytes) = raw.get(at..at + length) else {
            break;
        };
        if bytes[1..].iter().any(|&c| c & 0xc0 != 0x80) {
            break;
        }
        at += length;
        count += 1;
    }
    if count >= 512 {
        return None;
    }
    if count == 0 {
        data.raw = None;
        return Some(String::new());
    }
    // XlMultiToWide3301a100 skips one BOM, but its limit still includes that BOM.
    // Its byte decoder intentionally differs from Rust's strict UTF-8 decoder.
    let mut at = if raw.starts_with(&[0xef, 0xbb, 0xbf]) {
        3
    } else {
        0
    };
    let mut text = String::new();
    for _ in 0..count {
        let lead = raw.get(at).copied().unwrap_or(0);
        if lead == 0 {
            break;
        }
        at += 1;
        let (mut value, extra) = match lead {
            0..=0x7f => (u32::from(lead), 0),
            0x80..=0xdf => (u32::from(lead & 0x1f), 1),
            0xe0..=0xef => (u32::from(lead & 0x0f), 2),
            _ => (u32::from(lead & 7), 3),
        };
        for _ in 0..extra {
            // The C-string terminator is readable; reading past it is outside our backend contract.
            let next = if at == raw.len() { 0 } else { *raw.get(at)? };
            value = (value << 6) | u32::from(next & 0x3f);
            at += 1;
        }
        if value == 0 {
            break;
        }
        // Native UTF-32 also permits surrogate/out-of-Unicode values. Refuse those inputs;
        // our font backend requires Rust scalar values and has no source-backed substitute.
        text.push(char::from_u32(value)?);
    }
    raw.truncate(2048); // XlCopyString(..., 0x801, input), independently of the UTF-32 model.
    data.raw = (raw != text.as_bytes()).then_some(raw);
    Some(text)
}
