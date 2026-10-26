//! `bake` and `verify`: the original pak -> the compressed pak ([`aa_pak::baked`]).
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::Mutex;
use std::time::Instant;

use aa_pak::Pak;
use aa_pak::baked::{self, Record};

use crate::{ext, par_each};

/// Files the remake never reads, with the reason. Everything else is kept.
fn dropped(name: &str) -> Option<&'static str> {
    let n = aa_pak::normalize(name);
    if n.starts_with("bin32/") {
        Some("bin32 (exe/dll)")
    } else if n.starts_with("game/shaders/cache/") {
        Some("CryEngine shader cache")
    } else if n == "game/db/compact.sqlite3" {
        Some("encrypted client db (server copy is used)")
    } else {
        match ext(&n).as_str() {
            "avi" => Some("machinima videos"),
            "bai" => Some("AI paths (server side)"),
            _ => None,
        }
    }
}

/// Keep compressed data only if it saves at least this much.
const MIN_GAIN: f64 = 0.03;
/// Files compressed in parallel, then written in pak order (keeps related files together).
const BATCH: usize = 2048;

enum Packed {
    /// Same content as a blob already written.
    Dup([u8; 32], u64),
    New { hash: [u8; 32], size: u64, codec: u8, data: Vec<u8> },
}

#[derive(Default)]
struct Totals {
    files: usize,
    raw: u64,
    packed: u64,
}

/// What `bake` does beyond lossless compression.
#[derive(Clone, Copy, Default)]
pub struct Options {
    /// zstd level.
    pub level: i32,
    /// Lossy: textures outside `game/ui/` larger than this lose their top mips (0 = off).
    pub max_texture: u32,
    /// Lossy: re-encode textures outside `game/ui/` down to this PSNR floor in dB (0 = off).
    pub textures: f64,
}

pub fn bake(out: &str, opt: Options) -> anyhow::Result<()> {
    let Options { level, max_texture, textures } = opt;
    let tools = if textures > 0.0 { Some(crate::tex::Tools::find()?) } else { None };
    let choices = Mutex::new(BTreeMap::<&'static str, (usize, u64, u64, f64)>::new());
    let pak = Pak::global();
    let mut kept = Vec::new();
    let mut drops: BTreeMap<&str, Totals> = BTreeMap::new();
    for n in pak.names() {
        match dropped(n) {
            Some(why) => {
                let t = drops.entry(why).or_default();
                t.files += 1;
                t.raw += pak.entry(n).unwrap().size;
            }
            None => kept.push(n),
        }
    }

    if let Some(dir) = std::path::Path::new(out).parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut w = BufWriter::with_capacity(8 << 20, File::create(out)?);
    let mut offset = 0u64;
    let mut blobs: HashMap<[u8; 32], (u64, u64, u8)> = HashMap::new();
    let mut records = Vec::with_capacity(kept.len());
    let mut by_ext: BTreeMap<String, Totals> = BTreeMap::new();
    let (mut dup_files, mut dup_bytes) = (0usize, 0u64);
    let shrunk = Mutex::new((0usize, 0u64));
    let t = Instant::now();

    for (b, batch) in kept.chunks(BATCH).enumerate() {
        let known: std::collections::HashSet<[u8; 32]> = blobs.keys().copied().collect();
        let results: Vec<Mutex<Option<Packed>>> = batch.iter().map(|_| Mutex::new(None)).collect();
        let idx: Vec<usize> = (0..batch.len()).collect();
        par_each(&idx, |&i| {
            let mut data = pak.read(batch[i]).expect("read original pak");
            let lossy_ok = ext(batch[i]) == "dds" && !aa_pak::normalize(batch[i]).starts_with("game/ui/");
            if max_texture > 0 && lossy_ok {
                if let Some(small) = shrink_dds(&data, max_texture) {
                    let mut s = shrunk.lock().unwrap();
                    s.0 += 1;
                    s.1 += (data.len() - small.len()) as u64;
                    data = small;
                }
            }
            let hash = *blake3::hash(&data).as_bytes();
            let mut packed = if known.contains(&hash) {
                Packed::Dup(hash, data.len() as u64)
            } else {
                pack(hash, data.clone(), level)
            };
            if let (Some(tools), Packed::New { codec, data: blob, .. }) = (&tools, &mut packed)
                && lossy_ok
            {
                if let Some(c) = crate::tex::reencode_cached(tools, b * BATCH + i, &data, textures, blob.len()) {
                    let mut ch = choices.lock().unwrap();
                    let e = ch.entry(c.label).or_default();
                    e.0 += 1;
                    e.1 += blob.len() as u64;
                    e.2 += c.blob.len() as u64;
                    e.3 += c.psnr;
                    *codec = c.codec;
                    *blob = c.blob;
                }
            }
            *results[i].lock().unwrap() = Some(packed);
        });

        for (name, r) in batch.iter().zip(results) {
            let (hash, size, fresh) = match r.into_inner().unwrap().unwrap() {
                Packed::Dup(hash, size) => (hash, size, None),
                Packed::New { hash, size, codec, data } => (hash, size, Some((codec, data))),
            };
            let t = by_ext.entry(ext(name)).or_default();
            t.files += 1;
            t.raw += size;
            let (at, packed, codec) = match (blobs.get(&hash), fresh) {
                (Some(&blob), _) => {
                    dup_files += 1;
                    dup_bytes += size;
                    blob
                }
                (None, Some((codec, data))) => {
                    w.write_all(&data)?;
                    let blob = (offset, data.len() as u64, codec);
                    offset += data.len() as u64;
                    t.packed += data.len() as u64;
                    blobs.insert(hash, blob);
                    blob
                }
                (None, None) => unreachable!("duplicate of a blob that was never written"),
            };
            records.push(Record { name: name.to_string(), offset: at, packed, size, codec });
        }
        let done = ((b + 1) * BATCH).min(kept.len());
        eprint!(
            "\r{done}/{} files, {:.2} GB written, {:.0}s   ",
            kept.len(),
            offset as f64 / 1e9,
            t.elapsed().as_secs_f64()
        );
    }
    eprintln!();

    let index = baked::encode_index(&records);
    let packed_index = zstd::bulk::compress(&index, 19)?;
    w.write_all(&packed_index)?;
    w.write_all(baked::MAGIC)?;
    for v in [offset, packed_index.len() as u64, index.len() as u64] {
        w.write_all(&v.to_le_bytes())?;
    }
    w.flush()?;
    let total = offset + packed_index.len() as u64 + baked::FOOTER_SIZE;

    let gb = |b: u64| b as f64 / 1e9;
    let original: u64 = pak.names().map(|n| pak.entry(n).unwrap().size).sum();
    println!("baked {} files in {:.0}s: {:.2} GB -> {:.2} GB", records.len(), t.elapsed().as_secs_f64(), gb(original), gb(total));
    println!("dropped:");
    for (why, t) in &drops {
        println!("  {why:<44} {:>6} files {:>7.2} GB", t.files, gb(t.raw));
    }
    let shrunk = shrunk.into_inner().unwrap();
    if textures > 0.0 {
        println!("textures re-encoded (PSNR floor {textures} dB), lossless baked size -> new:");
        for (label, (n, before, after, q)) in choices.into_inner().unwrap() {
            println!("  {label:<10} {n:>6} files {:>6.2} GB -> {:>6.2} GB  mean PSNR {:.1}", gb(before), gb(after), q / n as f64);
        }
    }
    if max_texture > 0 {
        println!("textures capped at {max_texture} px: {} files, {:.2} GB of mips dropped", shrunk.0, gb(shrunk.1));
    }
    println!("duplicates stored once: {dup_files} files, {:.2} GB", gb(dup_bytes));
    println!("{:<10} {:>7} {:>8} {:>8} {:>6}", "ext", "files", "raw GB", "baked GB", "ratio");
    let mut v: Vec<_> = by_ext.into_iter().filter(|(_, t)| t.raw > 10_000_000).collect();
    v.sort_by(|a, b| b.1.raw.cmp(&a.1.raw));
    for (e, t) in v {
        println!("{e:<10} {:>7} {:>8.2} {:>8.2} {:>5.1}%", t.files, gb(t.raw), gb(t.packed), t.packed as f64 / t.raw as f64 * 100.0);
    }
    Ok(())
}

/// Drops the top mip levels of a block-compressed 2D DDS until it is at most `max` pixels wide and
/// high. `None` if there is nothing to drop or the file is not a plain mipmapped BC texture.
fn shrink_dds(d: &[u8], max: u32) -> Option<Vec<u8>> {
    if d.len() < 128 || &d[0..4] != b"DDS " {
        return None;
    }
    let u32_at = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
    let block = match (u32_at(80) & 4 != 0).then(|| &d[84..88])? {
        b"DXT1" | b"ATI1" | b"BC4U" => 8,
        b"DXT3" | b"DXT5" | b"ATI2" | b"BC5U" => 16,
        _ => return None,
    };
    if u32_at(112) & (0x200 | 0x20_0000) != 0 {
        return None; // cube map or volume
    }
    let (mut h, mut w, mut mips) = (u32_at(12), u32_at(16), u32_at(28));
    let level = |w: u32, h: u32| w.div_ceil(4).max(1) as usize * h.div_ceil(4).max(1) as usize * block;
    let mut skip = 0;
    while w.max(h) > max && mips > 1 {
        skip += level(w, h);
        w = (w / 2).max(1);
        h = (h / 2).max(1);
        mips -= 1;
    }
    if skip == 0 || 128 + skip >= d.len() {
        return None;
    }
    let mut out = Vec::with_capacity(d.len() - skip);
    out.extend_from_slice(&d[..128]);
    out[12..16].copy_from_slice(&h.to_le_bytes());
    out[16..20].copy_from_slice(&w.to_le_bytes());
    out[20..24].copy_from_slice(&(level(w, h) as u32).to_le_bytes());
    out[28..32].copy_from_slice(&mips.to_le_bytes());
    out.extend_from_slice(&d[128 + skip..]);
    Some(out)
}

fn pack(hash: [u8; 32], data: Vec<u8>, level: i32) -> Packed {
    let size = data.len() as u64;
    let (codec, packed) = match baked::bc_split(&data) {
        Some(split) => (baked::ZSTD_BC_MIPS, zstd::bulk::compress(&split, level).unwrap()),
        None => (baked::ZSTD, zstd::bulk::compress(&data, level).unwrap()),
    };
    if (packed.len() as f64) < size as f64 * (1.0 - MIN_GAIN) {
        Packed::New { hash, size, codec, data: packed }
    } else {
        Packed::New { hash, size, codec: baked::STORED, data }
    }
}

/// Every file of the baked pak decodes to the original bytes, except re-encoded textures (Basis
/// or rate-distortion optimised BC), which must keep the original's size and header; reports decode
/// throughput.
pub fn verify(path: &str) -> anyhow::Result<()> {
    let original = Pak::global();
    let baked = Pak::open(path)?;
    let names: Vec<&str> = baked.names().collect();
    let bad = Mutex::new(Vec::new());
    let acc = Mutex::new((0u64, 0f64));
    let t = Instant::now();
    par_each(&names, |n| {
        let d = Instant::now();
        let got = baked.read(n);
        let secs = d.elapsed().as_secs_f64();
        let want = original.read(n).unwrap();
        let codec = baked.entry(n).unwrap().codec;
        // Re-encoded textures keep size and header; rounded meshes keep size and file header.
        let lossy = |g: &[u8]| {
            let e = ext(n);
            g.len() == want.len()
                && ((e == "dds" && matches!(codec, baked::BASIS | baked::ZSTD_BC_MIPS) && g[..128.min(g.len())] == want[..128.min(want.len())])
                    || (["chr", "cgf", "cga", "skin"].contains(&e.as_str()) && g[..20.min(g.len())] == want[..20.min(want.len())]))
        };
        match got {
            Ok(got) if got == want || lossy(&got) => {
                let mut a = acc.lock().unwrap();
                a.0 += want.len() as u64;
                a.1 += secs;
            }
            other => bad.lock().unwrap().push(format!("{n}: {:?}", other.map(|g| g.len()))),
        }
    });
    let bad = bad.into_inner().unwrap();
    let (bytes, secs) = acc.into_inner().unwrap();
    let missing = original.names().filter(|n| dropped(n).is_none() && !baked.exists(n)).count();
    println!(
        "verified {} files ({:.2} GB) in {:.0}s: {} mismatches, {missing} missing; read+decode {:.0} MB/s per thread",
        names.len(),
        bytes as f64 / 1e9,
        t.elapsed().as_secs_f64(),
        bad.len(),
        bytes as f64 / 1e6 / secs
    );
    for b in bad.iter().take(20) {
        println!("  {b}");
    }
    anyhow::ensure!(bad.is_empty() && missing == 0, "baked pak differs from the original");
    Ok(())
}
