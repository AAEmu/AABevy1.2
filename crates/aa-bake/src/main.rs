//! Offline baker for the game pak. Run with `--release`.
//!
//! * `bake <out> [level]`: writes the compressed pak ([`aa_pak::baked`]): unused files dropped,
//!   identical files stored once, DDS blocks regrouped, every file zstd compressed on its own.
//! * `verify <baked>`: every file of the baked pak equals the original.
//! * `stats`, `dedup`, `dds`, `dict`, `ddsinfo`: the measurements that chose the above.
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use aa_pak::Pak;

mod bake;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["bake", out] => bake::bake(out, 19),
        ["bake", out, level] => bake::bake(out, level.parse()?),
        ["verify", path] => bake::verify(path),
        ["stats"] => stats(64),
        ["stats", mb] => stats(mb.parse()?),
        ["dedup"] => dedup(),
        ["dds", mb] => dds_split_trial(mb.parse()?),
        ["dict", e, mb] => dict_trial(e, mb.parse()?),
        ["ddsinfo"] => dds_info(),
        _ => {
            eprintln!("usage: aa-bake bake <out> [level] | verify <baked> | stats [sample_mb_per_ext] | dedup | dds <mb> | dict <ext> <mb>");
            Ok(())
        }
    }
}

pub(crate) fn ext(name: &str) -> String {
    let file = name.rsplit('/').next().unwrap_or(name);
    match file.rsplit_once('.') {
        Some((_, e)) => e.to_ascii_lowercase(),
        None => "(none)".into(),
    }
}

/// Runs `f` over `items` on all cores.
pub(crate) fn par_each<T: Sync>(items: &[T], f: impl Fn(&T) + Sync) {
    let next = AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                while let Some(item) = items.get(next.fetch_add(1, Ordering::Relaxed)) {
                    f(item);
                }
            });
        }
    });
}

#[derive(Default, Clone)]
struct Row {
    files: usize,
    total: u64,
    sampled: u64,
    z3: u64,
    z19: u64,
    dec_secs: f64,
}

fn stats(sample_mb: u64) -> anyhow::Result<()> {
    let pak = Pak::global();
    let mut by_ext: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for n in pak.names() {
        by_ext.entry(ext(n)).or_default().push(n);
    }

    // Deterministic sample per extension: files ordered by name hash, up to `sample_mb`.
    let mut rows: BTreeMap<String, Row> = BTreeMap::new();
    let mut work: Vec<(String, &str)> = Vec::new();
    for (e, names) in &by_ext {
        let row = rows.entry(e.clone()).or_default();
        row.files = names.len();
        row.total = names.iter().map(|n| pak.entry(n).unwrap().size).sum();
        let mut shuffled = names.clone();
        shuffled.sort_by_key(|n| *blake3::hash(n.as_bytes()).as_bytes());
        let mut taken = 0;
        for n in shuffled {
            if taken >= sample_mb << 20 {
                break;
            }
            taken += pak.entry(n).unwrap().size;
            work.push((e.clone(), n));
        }
    }

    let t = Instant::now();
    let rows = Mutex::new(rows);
    let dds = Mutex::new(BTreeMap::<String, (usize, u64)>::new());
    par_each(&work, |(e, n)| {
        let data = pak.read(n).unwrap();
        let z3 = zstd::bulk::compress(&data, 3).unwrap();
        let z19 = zstd::bulk::compress(&data, 19).unwrap();
        let d = Instant::now();
        let mut out = Vec::with_capacity(data.len());
        ruzstd::decoding::StreamingDecoder::new(&z19[..])
            .map(|mut r| std::io::copy(&mut r, &mut out))
            .unwrap()
            .unwrap();
        let dec = d.elapsed().as_secs_f64();
        assert_eq!(out, data, "{n}");
        let mut rows = rows.lock().unwrap();
        let r = rows.get_mut(e).unwrap();
        r.sampled += data.len() as u64;
        r.z3 += z3.len() as u64;
        r.z19 += z19.len() as u64;
        r.dec_secs += dec;
        drop(rows);
        if e == "dds" {
            let mut dds = dds.lock().unwrap();
            let v = dds.entry(dds_format(&data)).or_default();
            v.0 += 1;
            v.1 += data.len() as u64;
        }
    });
    let rows = rows.into_inner().unwrap();
    eprintln!("sampled {} files in {:.0}s", work.len(), t.elapsed().as_secs_f64());

    let gb = |b: f64| b / 1e9;
    let (mut total, mut est3, mut est19) = (0.0, 0.0, 0.0);
    let mut sorted: Vec<_> = rows.iter().collect();
    sorted.sort_by(|a, b| b.1.total.cmp(&a.1.total));
    println!("{:<8} {:>7} {:>9} {:>8} {:>8} {:>9} {:>10}", "ext", "files", "GB", "zstd3", "zstd19", "est19 GB", "ruzstd MB/s");
    for (e, r) in sorted {
        let (f3, f19) = if r.sampled > 0 {
            (r.z3 as f64 / r.sampled as f64, r.z19 as f64 / r.sampled as f64)
        } else {
            (1.0, 1.0)
        };
        total += r.total as f64;
        est3 += r.total as f64 * f3;
        est19 += r.total as f64 * f19;
        if r.total < 5_000_000 {
            continue;
        }
        println!(
            "{e:<8} {:>7} {:>9.2} {:>7.1}% {:>7.1}% {:>9.2} {:>10.0}",
            r.files,
            gb(r.total as f64),
            f3 * 100.0,
            f19 * 100.0,
            gb(r.total as f64 * f19),
            r.sampled as f64 / 1e6 / r.dec_secs.max(1e-9),
        );
    }
    println!("total {:.2} GB -> zstd3 {:.2} GB, zstd19 {:.2} GB", gb(total), gb(est3), gb(est19));

    println!("\nDDS formats in the sample:");
    let dds = dds.into_inner().unwrap();
    let mut dds: Vec<_> = dds.into_iter().collect();
    dds.sort_by(|a, b| b.1.1.cmp(&a.1.1));
    for (f, (c, b)) in dds {
        println!("  {f:<24} {c:>6} files {:>8.1} MB", b as f64 / 1e6);
    }
    Ok(())
}

/// `DXT1 1024x1024 m11` style summary of a DDS header.
fn dds_format(d: &[u8]) -> String {
    if d.len() < 128 || &d[0..4] != b"DDS " {
        return "not-dds".into();
    }
    let u32_at = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
    let (h, w, mips) = (u32_at(12), u32_at(16), u32_at(28));
    let pf_flags = u32_at(80);
    let fmt = if pf_flags & 4 != 0 {
        String::from_utf8_lossy(&d[84..88]).into_owned()
    } else {
        format!("rgb{}bpp", u32_at(88))
    };
    let size = match w.max(h) {
        0..=256 => "<=256",
        257..=512 => "512",
        513..=1024 => "1k",
        1025..=2048 => "2k",
        _ => "4k+",
    };
    format!("{fmt} {size} mips={}", mips.max(1).min(20) > 1)
}

/// Deterministic sample of `names`: ordered by name hash, up to `mb` megabytes.
fn sample<'a>(pak: &Pak, names: impl Iterator<Item = &'a str>, mb: u64) -> Vec<&'a str> {
    let mut v: Vec<&str> = names.collect();
    v.sort_by_key(|n| *blake3::hash(n.as_bytes()).as_bytes());
    let mut taken = 0;
    v.retain(|n| {
        let keep = taken < mb << 20;
        taken += pak.entry(n).unwrap().size;
        keep
    });
    v
}

/// Content-identical files over the whole pak, by extension.
fn dedup() -> anyhow::Result<()> {
    let pak = Pak::global();
    let names: Vec<&str> = pak.names().collect();
    let seen = Mutex::new(std::collections::HashMap::<[u8; 32], u64>::new());
    let dups = Mutex::new(BTreeMap::<String, (usize, u64)>::new());
    let t = Instant::now();
    par_each(&names, |n| {
        let data = pak.read(n).unwrap();
        let h = *blake3::hash(&data).as_bytes();
        if seen.lock().unwrap().insert(h, data.len() as u64).is_some() {
            let mut d = dups.lock().unwrap();
            let v = d.entry(ext(n)).or_default();
            v.0 += 1;
            v.1 += data.len() as u64;
        }
    });
    let dups = dups.into_inner().unwrap();
    let total: u64 = dups.values().map(|v| v.1).sum();
    println!("read all in {:.0}s; duplicate content: {:.2} GB", t.elapsed().as_secs_f64(), total as f64 / 1e9);
    let mut v: Vec<_> = dups.into_iter().collect();
    v.sort_by(|a, b| b.1.1.cmp(&a.1.1));
    for (e, (c, b)) in v.into_iter().take(15) {
        println!("  {e:<10} {c:>6} files {:>8.1} MB", b as f64 / 1e6);
    }
    Ok(())
}

fn dds_split_trial(mb: u64) -> anyhow::Result<()> {
    let pak = Pak::global();
    let names = sample(pak, pak.names().filter(|n| ext(n) == "dds"), mb);
    let acc = Mutex::new([0u64; 4]); // raw, zstd19, split raw, split zstd19
    par_each(&names, |n| {
        let data = pak.read(n).unwrap();
        let plain = zstd::bulk::compress(&data, 19).unwrap().len() as u64;
        let split = aa_pak::baked::bc_split(&data).map(|s| zstd::bulk::compress(&s, 19).unwrap().len() as u64);
        let mut a = acc.lock().unwrap();
        a[0] += data.len() as u64;
        a[1] += plain;
        if let Some(s) = split {
            a[2] += plain;
            a[3] += s;
        } 
    });
    let a = acc.into_inner().unwrap();
    println!(
        "dds sample {:.0} MB: zstd19 {:.1}%; BC files: zstd19 {:.0} MB -> split+zstd19 {:.0} MB ({:.1}% smaller)",
        a[0] as f64 / 1e6,
        a[1] as f64 / a[0] as f64 * 100.0,
        a[2] as f64 / 1e6,
        a[3] as f64 / 1e6,
        (1.0 - a[3] as f64 / a[2] as f64) * 100.0
    );
    Ok(())
}

/// Per-extension zstd dictionary: train on half the sample, measure on the other half.
fn dict_trial(e: &str, mb: u64) -> anyhow::Result<()> {
    let pak = Pak::global();
    let names = sample(pak, pak.names().filter(|n| ext(n) == e), mb);
    let data: Vec<Vec<u8>> = names.iter().map(|n| pak.read(n).unwrap()).collect();
    let (train, test): (Vec<_>, Vec<_>) = data.iter().enumerate().partition(|(i, _)| i % 2 == 0);
    let train: Vec<&[u8]> = train.into_iter().map(|(_, d)| d.as_slice()).collect();
    let test: Vec<&Vec<u8>> = test.into_iter().map(|(_, d)| d).collect();
    let raw: usize = test.iter().map(|d| d.len()).sum();
    let plain: usize = test.iter().map(|d| zstd::bulk::compress(d, 19).unwrap().len()).sum();
    for size in [112 << 10, 1 << 20] {
        let dict = zstd::dict::from_samples(&train, size)?;
        let mut c = zstd::bulk::Compressor::with_dictionary(19, &dict)?;
        let with: usize = test.iter().map(|d| c.compress(d).unwrap().len()).sum();
        println!(
            "{e}: {} test files {:.1} MB: zstd19 {:.1}%, dict {} KB {:.1}%",
            test.len(),
            raw as f64 / 1e6,
            plain as f64 / raw as f64 * 100.0,
            size >> 10,
            with as f64 / raw as f64 * 100.0
        );
    }
    Ok(())
}

/// Every DDS header: raw bytes by format / size class and by folder.
fn dds_info() -> anyhow::Result<()> {
    let pak = Pak::global();
    let file = std::fs::File::open(pak.path())?;
    let mut fmt = BTreeMap::<String, (usize, u64)>::new();
    let mut dir = BTreeMap::<String, (usize, u64)>::new();
    for n in pak.names().filter(|n| ext(n) == "dds") {
        let e = pak.entry(n).unwrap();
        let mut h = [0u8; 128];
        let len = (e.size as usize).min(128);
        std::os::windows::fs::FileExt::seek_read(&file, &mut h[..len], e.offset)?;
        let f = dds_format(&h);
        let v = fmt.entry(f).or_default();
        v.0 += 1;
        v.1 += e.size;
        let d: String = n.to_ascii_lowercase().split('/').take(3).collect::<Vec<_>>().join("/");
        let v = dir.entry(d).or_default();
        v.0 += 1;
        v.1 += e.size;
    }
    for (title, map) in [("format", fmt), ("folder", dir)] {
        println!("by {title}:");
        let mut v: Vec<_> = map.into_iter().collect();
        v.sort_by(|a, b| b.1.1.cmp(&a.1.1));
        for (k, (c, b)) in v.into_iter().take(20) {
            println!("  {k:<36} {c:>6} files {:>8.1} MB", b as f64 / 1e6);
        }
    }
    Ok(())
}
