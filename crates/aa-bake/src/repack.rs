//! `repack`: a v1 baked pak (one blob per file) -> v2 (solid blocks, see [`aa_pak::baked`]).
//! The expensive per-file work of `bake` (texture re-encoding) is reused as is.
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::Mutex;
use std::time::Instant;

use aa_pak::Pak;
use aa_pak::baked::{self, Block, Record2};

use crate::{ext, par_each};

/// Chunk files whose float streams `--meshes` may round.
const MESH_EXTS: [&str; 4] = ["chr", "cgf", "cga", "skin"];
/// Bits kept: positions, normals, texcoords (see `aa_pak::mesh::quantize`).
const MESH_BITS: [u32; 3] = [16, 12, 16];
/// Blocks closed at a folder change once they hold this much.
const MIN_BLOCK: u64 = 1 << 20;

struct Plan {
    zstd: bool,
    /// Indices into the source names.
    members: Vec<usize>,
    raw: u64,
}

/// `extras`: `(name, path)` of loose files to add (e.g. the game database for the browser build),
/// each in its own block.
pub fn repack(input: &str, out: &str, level: i32, block_mb: u64, meshes: bool, extras: &[(String, String)]) -> anyhow::Result<()> {
    let src = Pak::open(input)?;
    let mut names: Vec<&str> = src.names().collect();
    let mut entries: Vec<aa_pak::Entry> = names.iter().map(|n| src.entry(n).unwrap()).collect();
    anyhow::ensure!(entries.iter().all(|e| e.block.is_none()), "{input} is not a v1 baked pak");
    // Extras get fake v1 entries (offsets past the input, so they never alias a real blob).
    let mut extra_paths: HashMap<usize, &str> = HashMap::new();
    for (k, (name, path)) in extras.iter().enumerate() {
        anyhow::ensure!(src.entry(name).is_none(), "{name} is already in {input}");
        let size = std::fs::metadata(path)?.len();
        extra_paths.insert(names.len(), path);
        names.push(name);
        entries.push(aa_pak::Entry { offset: u64::MAX - k as u64, size, packed: size, codec: baked::ZSTD, block: None });
    }
    let max_block = block_mb << 20;
    let inner_len = |e: &aa_pak::Entry| if e.codec == baked::BASIS { e.packed } else { e.size };
    let inner_codec = |c: u8| if c == baked::ZSTD { baked::STORED } else { c };

    // Folders in name order, files inside by extension so similar data sits together.
    let mut folders: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, n) in names.iter().enumerate().filter(|(i, _)| !extra_paths.contains_key(i)) {
        let n = aa_pak::normalize(n);
        let dir = n.rsplit_once('/').map_or("", |d| d.0).to_string();
        folders.entry(dir).or_default().push(i);
    }
    let mut plans: Vec<Plan> = Vec::new();
    let mut owner: HashMap<u64, usize> = HashMap::new(); // v1 blob offset -> first file using it
    let mut current = Plan { zstd: true, members: Vec::new(), raw: 0 };
    for (&i, _) in &extra_paths {
        owner.insert(entries[i].offset, i);
        plans.push(Plan { zstd: true, members: vec![i], raw: entries[i].size });
    }
    for files in folders.values_mut() {
        files.sort_by_key(|&i| (ext(names[i]), aa_pak::normalize(names[i])));
        for &i in files.iter() {
            let e = &entries[i];
            if owner.contains_key(&e.offset) {
                continue;
            }
            owner.insert(e.offset, i);
            if e.codec == baked::STORED {
                plans.push(Plan { zstd: false, members: vec![i], raw: e.size });
                continue;
            }
            let len = inner_len(e);
            if current.raw > 0 && current.raw + len > max_block {
                plans.push(std::mem::replace(&mut current, Plan { zstd: true, members: Vec::new(), raw: 0 }));
            }
            current.members.push(i);
            current.raw += len;
        }
        if current.raw >= MIN_BLOCK {
            plans.push(std::mem::replace(&mut current, Plan { zstd: true, members: Vec::new(), raw: 0 }));
        }
    }
    if !current.members.is_empty() {
        plans.push(current);
    }

    let mut w = BufWriter::with_capacity(8 << 20, File::create(out)?);
    let mut offset = 0u64;
    let mut blocks: Vec<Block> = Vec::with_capacity(plans.len());
    let mut placed: HashMap<usize, (u32, u64, u64, u8)> = HashMap::new(); // owner -> place
    let t = Instant::now();
    let rounded = Mutex::new(0usize);
    for (b, batch) in plans.chunks(48).enumerate() {
        let built: Vec<Mutex<Option<Vec<u8>>>> = batch.iter().map(|_| Mutex::new(None)).collect();
        let idx: Vec<usize> = (0..batch.len()).collect();
        par_each(&idx, |&p| {
            let plan = &batch[p];
            let mut raw = Vec::with_capacity(plan.raw as usize);
            for &i in &plan.members {
                let e = &entries[i];
                if let Some(path) = extra_paths.get(&i) {
                    raw.extend_from_slice(&std::fs::read(path).expect("read extra file"));
                    continue;
                }
                let blob = src.read_packed(names[i]).expect("read v1 blob");
                let mut inner = match e.codec {
                    baked::ZSTD | baked::ZSTD_BC_SPLIT | baked::ZSTD_BC_MIPS => {
                        zstd::bulk::decompress(&blob, e.size as usize).expect("v1 blob")
                    }
                    _ => blob,
                };
                if meshes && e.codec == baked::ZSTD && MESH_EXTS.contains(&ext(names[i]).as_str()) {
                    if aa_pak::mesh::quantize(&mut inner, MESH_BITS) > 0 {
                        *rounded.lock().unwrap() += 1;
                    }
                }
                raw.extend_from_slice(&inner);
            }
            let data = if plan.zstd {
                let mut c = zstd::bulk::Compressor::new(level).unwrap();
                let window = (64 - (raw.len().max(1) as u64 - 1).leading_zeros()).clamp(10, 30);
                c.set_parameter(zstd::zstd_safe::CParameter::WindowLog(window)).unwrap();
                c.set_parameter(zstd::zstd_safe::CParameter::EnableLongDistanceMatching(true)).unwrap();
                let mut packed = c.compress(&raw).unwrap();
                packed.extend_from_slice(&(raw.len() as u64).to_le_bytes()); // raw size, popped below
                packed
            } else {
                raw
            };
            *built[p].lock().unwrap() = Some(data);
        });
        for (plan, data) in batch.iter().zip(built) {
            let mut data = data.into_inner().unwrap().unwrap();
            let raw = if plan.zstd {
                let n = data.len() - 8;
                let raw = u64::from_le_bytes(data[n..].try_into().unwrap());
                data.truncate(n);
                raw
            } else {
                data.len() as u64
            };
            let id = blocks.len() as u32;
            let kind = if plan.zstd { baked::BLOCK_ZSTD } else { baked::BLOCK_STORED };
            blocks.push(Block { offset, packed: data.len() as u64, raw, kind });
            w.write_all(&data)?;
            offset += data.len() as u64;
            let mut at = 0;
            for &i in &plan.members {
                let len = inner_len(&entries[i]);
                placed.insert(i, (id, at, len, inner_codec(entries[i].codec)));
                at += len;
            }
        }
        eprint!("\r{}/{} blocks, {:.2} GB, {:.0}s   ", ((b + 1) * 48).min(plans.len()), plans.len(), offset as f64 / 1e9, t.elapsed().as_secs_f64());
    }
    eprintln!();

    let records: Vec<Record2> = names
        .iter()
        .zip(&entries)
        .map(|(n, e)| {
            let (block, off, len, codec) = placed[&owner[&e.offset]];
            Record2 { name: n.to_string(), block, offset: off, len, size: e.size, codec }
        })
        .collect();
    let index = baked::encode_index2(&blocks, &records);
    let packed_index = zstd::bulk::compress(&index, 19)?;
    w.write_all(&packed_index)?;
    w.write_all(baked::MAGIC2)?;
    for v in [offset, packed_index.len() as u64, index.len() as u64] {
        w.write_all(&v.to_le_bytes())?;
    }
    w.flush()?;
    let total = offset + packed_index.len() as u64 + baked::FOOTER_SIZE;
    let input_size = std::fs::metadata(input)?.len();
    let zb: Vec<&Block> = blocks.iter().filter(|b| b.kind == baked::BLOCK_ZSTD).collect();
    println!(
        "repacked {} files into {} blocks ({} solid, mean {:.1} MB packed) in {:.0}s: {:.2} GB -> {:.2} GB{}",
        records.len(),
        blocks.len(),
        zb.len(),
        zb.iter().map(|b| b.packed).sum::<u64>() as f64 / zb.len().max(1) as f64 / 1e6,
        t.elapsed().as_secs_f64(),
        input_size as f64 / 1e9,
        total as f64 / 1e9,
        if meshes { format!(", {} meshes rounded", rounded.into_inner().unwrap()) } else { String::new() }
    );
    Ok(())
}
