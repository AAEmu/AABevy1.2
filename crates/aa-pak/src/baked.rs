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
/// A DDS whose BC blocks were regrouped by field ([`bc_split`]) before zstd.
pub const ZSTD_BC_SPLIT: u8 = 2;

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

/// Header size and bytes per field of one BC block, for block-compressed DDS files.
fn bc_fields(d: &[u8]) -> Option<(usize, &'static [usize])> {
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
    ((d.len() - 128) % block == 0).then_some((128, fields))
}

/// Lossless: regroups the BC blocks of a DDS so each field (endpoints, indices) forms one run,
/// which zstd compresses ~6% better. `None` if `d` is not a block-compressed DDS.
pub fn bc_split(d: &[u8]) -> Option<Vec<u8>> {
    let (header, fields) = bc_fields(d)?;
    let block: usize = fields.iter().sum();
    let mut out = Vec::with_capacity(d.len());
    out.extend_from_slice(&d[..header]);
    let mut at = 0;
    for &f in fields {
        for b in d[header..].chunks_exact(block) {
            out.extend_from_slice(&b[at..at + f]);
        }
        at += f;
    }
    Some(out)
}

/// Inverse of [`bc_split`] (the header is unchanged, so the format is read from it).
pub fn bc_join(s: &[u8]) -> io::Result<Vec<u8>> {
    let (header, fields) =
        bc_fields(s).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bc_join: not a BC DDS"))?;
    let block: usize = fields.iter().sum();
    let blocks = (s.len() - header) / block;
    let mut out = vec![0u8; s.len()];
    out[..header].copy_from_slice(&s[..header]);
    let (mut src, mut at) = (header, 0);
    for &f in fields {
        for i in 0..blocks {
            let dst = header + i * block + at;
            out[dst..dst + f].copy_from_slice(&s[src..src + f]);
            src += f;
        }
        at += f;
    }
    Ok(out)
}

/// Decodes a blob as stored with `codec` into the original `size` bytes.
pub fn unpack(codec: u8, packed: &[u8], size: usize) -> io::Result<Vec<u8>> {
    match codec {
        STORED => Ok(packed.to_vec()),
        ZSTD => zstd::bulk::decompress(packed, size),
        ZSTD_BC_SPLIT => bc_join(&zstd::bulk::decompress(packed, size)?),
        c => Err(io::Error::new(io::ErrorKind::InvalidData, format!("unknown baked codec {c}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bc_split_round_trips() {
        let mut dds = vec![0u8; 128 + 16 * 5];
        dds[0..4].copy_from_slice(b"DDS ");
        dds[80] = 4;
        dds[84..88].copy_from_slice(b"DXT5");
        for (i, b) in dds[128..].iter_mut().enumerate() {
            *b = i as u8;
        }
        let split = bc_split(&dds).unwrap();
        assert_ne!(split, dds);
        assert_eq!(bc_join(&split).unwrap(), dds);
    }

    #[test]
    fn index_round_trips() {
        let r = vec![Record { name: "game/a.dds".into(), offset: 1, packed: 2, size: 3, codec: ZSTD }];
        assert_eq!(decode_index(&encode_index(&r)).unwrap(), r);
    }
}
