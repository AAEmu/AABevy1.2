//! The baked pak written by `aa-bake`: a plain, unencrypted archive whose files are zstd
//! compressed one by one (so any file is still one positioned read away).
//!
//! Layout (little endian): `[file data][index, zstd][footer 32 bytes]`.
//! Footer: `MAGIC`, index offset `u64`, packed index size `u64`, raw index size `u64`.
//! Raw index: file count `u32`, then per file in pak order: name length `u16`, name (UTF-8),
//! offset `u64`, packed size `u64`, size `u64`, codec `u8`. Identical files share one blob.

use std::io;

pub const MAGIC: &[u8; 8] = b"AAPKv1\0\0";
pub const FOOTER_SIZE: u64 = 32;

/// Stored as is (range reads stay cheap; used for data zstd cannot shrink, like FSB banks).
pub const STORED: u8 = 0;
pub const ZSTD: u8 = 1;
/// A DDS whose blocks were all regrouped by field before zstd (first baker; reads only).
pub const ZSTD_BC_SPLIT: u8 = 2;
/// A DDS re-encoded as Basis Universal (lossy): its original 128-byte header, the length (`u32`)
/// and bytes of whatever followed its mips (CryEngine's `CExt` trailer), then a KTX2 file that
/// is transcoded back to the header's BC format and mip count on read ([`basis_to_dds`]).
pub const BASIS: u8 = 3;

/// One index record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub name: String,
    pub offset: u64,
    pub packed: u64,
    pub size: u64,
    pub codec: u8,
}

pub fn encode_index(records: &[Record]) -> Vec<u8> {
    let mut out = Vec::with_capacity(records.len() * 96);
    out.extend_from_slice(&(records.len() as u32).to_le_bytes());
    for r in records {
        out.extend_from_slice(&(r.name.len() as u16).to_le_bytes());
        out.extend_from_slice(r.name.as_bytes());
        for v in [r.offset, r.packed, r.size] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.push(r.codec);
    }
    out
}

pub fn decode_index(mut b: &[u8]) -> io::Result<Vec<Record>> {
    fn take<'a>(b: &mut &'a [u8], n: usize) -> io::Result<&'a [u8]> {
        if b.len() < n {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "truncated baked pak index"));
        }
        let (head, tail) = b.split_at(n);
        *b = tail;
        Ok(head)
    }
    let u64_of = |s: &[u8]| u64::from_le_bytes(s.try_into().unwrap());
    let count = u32::from_le_bytes(take(&mut b, 4)?.try_into().unwrap()) as usize;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let len = u16::from_le_bytes(take(&mut b, 2)?.try_into().unwrap()) as usize;
        let name = String::from_utf8_lossy(take(&mut b, len)?).into_owned();
        let offset = u64_of(take(&mut b, 8)?);
        let packed = u64_of(take(&mut b, 8)?);
        let size = u64_of(take(&mut b, 8)?);
        let codec = take(&mut b, 1)?[0];
        out.push(Record { name, offset, packed, size, codec });
    }
    Ok(out)
}

/// Bytes per field of one BC block and the byte range of `d` that is BC blocks. `mips_only`: just
/// the mip chain the header describes (what follows, like CryEngine's `CExt` trailer, is left
/// alone); otherwise everything after the header, if it is whole blocks (the [`ZSTD_BC_SPLIT`] form).
fn bc_range(d: &[u8], mips_only: bool) -> Option<(&'static [usize], usize, usize)> {
    if d.len() < 128 || &d[0..4] != b"DDS " || u32::from_le_bytes(d[80..84].try_into().unwrap()) & 4 == 0 {
        return None;
    }
    let fields: &[usize] = match &d[84..88] {
        b"DXT1" => &[4, 4],
        b"DXT3" => &[8, 4, 4],
        b"DXT5" => &[2, 6, 4, 4],
        b"ATI2" | b"BC5U" => &[2, 6, 2, 6],
        b"ATI1" | b"BC4U" => &[2, 6],
        _ => return None,
    };
    let block: usize = fields.iter().sum();
    if !mips_only {
        return ((d.len() - 128) % block == 0).then_some((fields, 128, d.len()));
    }
    let u32_at = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
    let (h, w, mips) = (u32_at(12), u32_at(16), u32_at(28).max(1));
    let mut end = 128usize;
    for l in 0..mips.min(16) {
        end = end.checked_add((w >> l).max(1).div_ceil(4) as usize * (h >> l).max(1).div_ceil(4) as usize * block)?;
    }
    (end <= d.len()).then_some((fields, 128, end))
}

fn split_with(d: &[u8], mips_only: bool) -> Option<Vec<u8>> {
    let (fields, start, end) = bc_range(d, mips_only)?;
    let block: usize = fields.iter().sum();
    let mut out = Vec::with_capacity(d.len());
    out.extend_from_slice(&d[..start]);
    let mut at = 0;
    for &f in fields {
        for b in d[start..end].chunks_exact(block) {
            out.extend_from_slice(&b[at..at + f]);
        }
        at += f;
    }
    out.extend_from_slice(&d[end..]);
    Some(out)
}

fn join_with(s: &[u8], mips_only: bool) -> io::Result<Vec<u8>> {
    let (fields, start, end) =
        bc_range(s, mips_only).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bc_join: not a BC DDS"))?;
    let block: usize = fields.iter().sum();
    let blocks = (end - start) / block;
    let mut out = s.to_vec();
    let (mut src, mut at) = (start, 0);
    for &f in fields {
        for i in 0..blocks {
            let dst = start + i * block + at;
            out[dst..dst + f].copy_from_slice(&s[src..src + f]);
            src += f;
        }
        at += f;
    }
    Ok(out)
}

/// Lossless: regroups the BC blocks of a DDS's mips so each field (endpoints, indices) forms one
/// run, which zstd compresses ~6% better ([`ZSTD_BC_MIPS`]). `None` if `d` is not a BC DDS.
pub fn bc_split(d: &[u8]) -> Option<Vec<u8>> {
    split_with(d, true)
}

/// Inverse of [`bc_split`] (the header is unchanged, so the layout is read from it).
pub fn bc_join(s: &[u8]) -> io::Result<Vec<u8>> {
    join_with(s, true)
}

/// A DDS whose mip blocks were regrouped by field ([`bc_split`]) before zstd.
pub const ZSTD_BC_MIPS: u8 = 4;

/// Decodes a blob as stored with `codec` into the original `size` bytes.
pub fn unpack(codec: u8, packed: &[u8], size: usize) -> io::Result<Vec<u8>> {
    match codec {
        STORED => Ok(packed.to_vec()),
        ZSTD => zstd::bulk::decompress(packed, size),
        ZSTD_BC_SPLIT => join_with(&zstd::bulk::decompress(packed, size)?, false),
        ZSTD_BC_MIPS => bc_join(&zstd::bulk::decompress(packed, size)?),
        BASIS => basis_to_dds(packed),
        c => Err(io::Error::new(io::ErrorKind::InvalidData, format!("unknown baked codec {c}"))),
    }
}

/// [`BASIS`] blob -> DDS: the KTX2's mips transcoded to the BC format named in the header.
pub fn basis_to_dds(packed: &[u8]) -> io::Result<Vec<u8>> {
    use basisu::{DecodeFlags, TargetFormat, Transcoder};
    let bad = |e: String| io::Error::new(io::ErrorKind::InvalidData, e);
    if packed.len() < 132 {
        return Err(bad("basis blob too short".into()));
    }
    let (header, rest) = packed.split_at(128);
    let tail_len = u32::from_le_bytes(rest[..4].try_into().unwrap()) as usize;
    let (tail, ktx2) = rest[4..].split_at_checked(tail_len).ok_or_else(|| bad("basis blob tail".into()))?;
    let target = match &header[84..88] {
        b"DXT1" => TargetFormat::Bc1Rgb,
        b"DXT5" => TargetFormat::Bc3Rgba,
        b"ATI2" | b"BC5U" => TargetFormat::Bc5Rg,
        b"ATI1" | b"BC4U" => TargetFormat::Bc4R,
        f => return Err(bad(format!("basis blob for unsupported DDS format {f:?}"))),
    };
    let tex = Transcoder::new(ktx2).map_err(|e| bad(format!("ktx2: {e:?}")))?;
    let mips = u32::from_le_bytes(header[28..32].try_into().unwrap()).clamp(1, tex.level_count());
    let mut out = header.to_vec();
    out[28..32].copy_from_slice(&mips.to_le_bytes());
    for level in 0..mips {
        let data = tex.transcode(level, target, DecodeFlags::NONE).map_err(|e| bad(format!("transcode: {e:?}")))?;
        out.extend_from_slice(&data);
    }
    out.extend_from_slice(tail);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bc_split_round_trips() {
        // 8x4 DXT5, 2 mips (2 + 1 blocks), then a 7-byte trailer.
        let mut dds = vec![0u8; 128 + 16 * 3 + 7];
        dds[0..4].copy_from_slice(b"DDS ");
        dds[12] = 4;
        dds[16] = 8;
        dds[28] = 2;
        dds[80] = 4;
        dds[84..88].copy_from_slice(b"DXT5");
        for (i, b) in dds[128..].iter_mut().enumerate() {
            *b = i as u8;
        }
        let split = bc_split(&dds).unwrap();
        assert_ne!(split, dds);
        assert_eq!(bc_join(&split).unwrap(), dds);
        assert_eq!(&split[split.len() - 7..], &dds[dds.len() - 7..]);
    }

    #[test]
    fn index_round_trips() {
        let r = vec![Record { name: "game/a.dds".into(), offset: 1, packed: 2, size: 3, codec: ZSTD }];
        assert_eq!(decode_index(&encode_index(&r)).unwrap(), r);
    }
}
