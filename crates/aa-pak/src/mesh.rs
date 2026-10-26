//! Lossy shrinking of CryEngine chunk files (`.chr`, `.cgf`, ...): [`quantize`] rounds the floats
//! of position, normal and texcoord streams so zstd finds more matches. The file stays a valid
//! chunk file; equal floats stay equal, so shared seams do not open.
//!
//! (Regrouping chunk bytes into planes was tried as well: it makes zstd ~20% worse, because mesh
//! data repeats whole vertices.)

/// `(start, len, stride)` of each chunk body (DataStream data with its element size); `None` if `d` is not a chunk file with
/// sane, non-overlapping chunks.
fn regions(d: &[u8]) -> Option<Vec<(usize, usize, usize)>> {
    let u32_at = |o: usize| d.get(o..o + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    if d.get(..8)? != b"CryTek\0\0" {
        return None;
    }
    let (version, table) = (u32_at(12)?, u32_at(16)? as usize);
    let entry = match version {
        0x744 => 16,
        0x745 => 20,
        _ => return None,
    };
    let count = u32_at(table)? as usize;
    if count > 100_000 || table + 4 + count * entry > d.len() {
        return None;
    }
    let mut chunks: Vec<(u32, usize, usize)> = Vec::with_capacity(count);
    for i in 0..count {
        let e = table + 4 + i * entry;
        let size = if version == 0x745 { u32_at(e + 16)? as usize } else { 0 };
        chunks.push((u32_at(e)?, u32_at(e + 8)? as usize, size));
    }
    if version == 0x744 {
        // Sizes are the distance to the next chunk (or the table, or the end).
        let mut starts: Vec<usize> = chunks.iter().map(|c| c.1).collect();
        starts.push(d.len());
        if table > 0x14 {
            starts.push(table);
        }
        starts.sort_unstable();
        for c in &mut chunks {
            let next = *starts.iter().find(|&&s| s > c.1)?;
            c.2 = next - c.1;
        }
    }
    let table_range = table..table + 4 + count * entry;
    let mut out = Vec::new();
    for (ty, offset, size) in chunks {
        let end = offset.checked_add(size)?;
        if end > d.len() || offset < 0x14 || (offset < table_range.end && end > table_range.start) {
            return None;
        }
        // Skip the 16-byte {type, version, offset, id} copy when present.
        let mut start = offset;
        if u32_at(offset) == Some(ty) && size >= 16 {
            start += 16;
        }
        let mut stride = 4;
        if ty == 0xCCCC_0016 && end >= start + 24 {
            // DataStream: flags, type, count, element size, reserved[2], data.
            let elem = u32_at(start + 12)? as usize;
            if (1..=64).contains(&elem) {
                stride = elem;
                start += 24;
            }
        }
        let len = (end - start) / stride * stride;
        if len >= stride * 2 {
            out.push((start, len, stride));
        }
    }
    out.sort_unstable();
    out.windows(2).all(|w| w[0].0 + w[0].1 <= w[1].0).then_some(out)
}

/// Lossy: rounds the floats of position, normal and texcoord DataStreams to `bits` mantissa
/// bits (positions, normals, uvs), in place. Returns how many streams were touched.
pub fn quantize(d: &mut [u8], bits: [u32; 3]) -> usize {
    let Some(regions) = regions(d) else { return 0 };
    let mut touched = 0;
    for (start, len, stride) in regions {
        // The DataStream type sits 20 bytes before the data (flags, type, count, elem, res[2]).
        if start < 24 {
            continue;
        }
        let ty = u32::from_le_bytes(d[start - 20..start - 16].try_into().unwrap());
        let keep = match (ty, stride) {
            (0, 12) => bits[0],
            (1, 12) => bits[1],
            (2, 8) => bits[2],
            _ => continue,
        };
        if u32::from_le_bytes(d[start - 12..start - 8].try_into().unwrap()) as usize != stride {
            continue;
        }
        for f in d[start..start + len].chunks_exact_mut(4) {
            let v = u32::from_le_bytes(f.try_into().unwrap());
            f.copy_from_slice(&round_mantissa(v, keep).to_le_bytes());
        }
        touched += 1;
    }
    touched
}

fn round_mantissa(v: u32, keep: u32) -> u32 {
    if keep >= 23 || (v >> 23) & 0xFF == 0xFF {
        return v;
    }
    let drop = 23 - keep;
    let half = 1u32 << (drop - 1);
    // Round half up on the magnitude; a carry into the exponent is the correct rounding.
    ((v & 0x7FFF_FFFF) + half) & !((1u32 << drop) - 1) | (v & 0x8000_0000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_mantissa() {
        let r = |x: f32, k| f32::from_bits(round_mantissa(x.to_bits(), k));
        assert_eq!(r(1.0, 10), 1.0);
        assert!((r(1.2345678, 10) - 1.2345678).abs() < 1e-3);
        assert_eq!(r(-2.5, 4), -2.5);
    }
}
