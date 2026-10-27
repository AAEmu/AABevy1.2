//! Lossy texture re-encoding (`bake --textures <floor>`): each BC1/BC3/BC5 DDS is tried as
//! Basis ETC1S at two qualities, then as rate-distortion optimised BC (bc7enc_rdo) at two
//! lambdas; the first candidate whose PSNR against the original (top mip, after the transcode the
//! reader does) reaches the floor is kept if it is smaller than the lossless blob.
//!
//! Needs the encoders: `basisu.exe` (Basis Universal 2.5) and `bc7enc.exe` (bc7enc_rdo), found
//! in the folder `AA_BAKE_TOOLS` points at.
use std::path::{Path, PathBuf};
use std::process::Command;

use aa_pak::baked;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Bc1,
    Bc3,
    Bc5,
}

impl Kind {
    fn block(self) -> usize {
        if self == Kind::Bc1 { 8 } else { 16 }
    }
}

pub struct Tools {
    basisu: PathBuf,
    bc7enc: PathBuf,
    tmp: PathBuf,
}

impl Tools {
    pub fn find() -> anyhow::Result<Tools> {
        let dir = PathBuf::from(std::env::var("AA_BAKE_TOOLS").map_err(|_| anyhow::anyhow!("set AA_BAKE_TOOLS to the encoders' folder"))?);
        let tools = Tools {
            basisu: dir.join("basisu-src/bin/basisu.exe"),
            bc7enc: dir.join("bc7enc_rdo/bc7enc.exe"),
            tmp: dir.join("tmp"),
        };
        anyhow::ensure!(tools.basisu.is_file(), "missing {}", tools.basisu.display());
        anyhow::ensure!(tools.bc7enc.is_file(), "missing {}", tools.bc7enc.display());
        std::fs::create_dir_all(&tools.tmp)?;
        Ok(tools)
    }
}

/// One step of the ladder, cheapest output first.
#[derive(Clone, Copy)]
enum Step {
    Etc1s(u32),
    Rdo(f32),
}

const LADDER: [Step; 4] = [Step::Etc1s(50), Step::Etc1s(90), Step::Rdo(3.0), Step::Rdo(1.0)];
/// Below this size only the (fast) ETC1S steps are tried; such textures are a small share of bytes.
const RDO_MIN: u32 = 256;

pub struct Choice {
    pub label: &'static str,
    pub codec: u8,
    pub blob: Vec<u8>,
    pub psnr: f64,
}

/// Plain 2D BC1/BC3/BC5 DDS: kind, size, mip count and where the mips end (nothing or a
/// CryEngine `CExt` trailer may follow).
fn parse(d: &[u8]) -> Option<(Kind, u32, u32, u32, usize)> {
    if d.len() < 128 || &d[0..4] != b"DDS " {
        return None;
    }
    let u32_at = |o: usize| u32::from_le_bytes(d[o..o + 4].try_into().unwrap());
    let kind = match (u32_at(80) & 4 != 0).then(|| &d[84..88])? {
        b"DXT1" => Kind::Bc1,
        b"DXT5" => Kind::Bc3,
        b"ATI2" => Kind::Bc5,
        _ => return None,
    };
    let (h, w, mips) = (u32_at(12), u32_at(16), u32_at(28).max(1));
    if u32_at(112) & (0x200 | 0x20_0000) != 0 || w % 4 != 0 || h % 4 != 0 || w < 8 || h < 8 {
        return None;
    }
    let total: usize = (0..mips).map(|l| level_size(kind, w >> l, h >> l)).sum();
    let end = 128 + total;
    (end == d.len() || d.get(end..end + 4) == Some(b"CExt")).then_some((kind, w, h, mips, end))
}

fn level_size(kind: Kind, w: u32, h: u32) -> usize {
    w.max(1).div_ceil(4) as usize * h.max(1).div_ceil(4) as usize * kind.block()
}

/// Top mip of BC data as RGBA. `bc1_alpha`: decode BC1 3-colour blocks' index 3 as transparent.
fn decode(kind: Kind, w: u32, h: u32, blocks: &[u8], bc1_alpha: bool) -> Vec<[u8; 4]> {
    let (w, h) = (w as usize, h as usize);
    let mut out = vec![[0u8; 4]; w * h];
    let mut px = [0u32; 16];
    for (i, b) in blocks.chunks_exact(kind.block()).take((w / 4) * (h / 4)).enumerate() {
        match kind {
            Kind::Bc1 if bc1_alpha => texture2ddecoder::decode_bc1a_block(b, &mut px),
            Kind::Bc1 => texture2ddecoder::decode_bc1_block(b, &mut px),
            Kind::Bc3 => texture2ddecoder::decode_bc3_block(b, &mut px),
            Kind::Bc5 => texture2ddecoder::decode_bc5_block(b, &mut px),
        }
        let (bx, by) = (i % (w / 4) * 4, i / (w / 4) * 4);
        for (j, p) in px.iter().enumerate() {
            let [b, g, r, a] = p.to_le_bytes();
            out[(by + j / 4) * w + bx + j % 4] = [r, g, b, a];
        }
    }
    out
}

/// PSNR over the channels the format carries (BC3: the worse of colour and alpha).
fn psnr(kind: Kind, a: &[[u8; 4]], b: &[[u8; 4]]) -> f64 {
    let mse = |chans: &[usize]| {
        let mut sum = 0f64;
        for (p, q) in a.iter().zip(b) {
            for &c in chans {
                let d = p[c] as f64 - q[c] as f64;
                sum += d * d;
            }
        }
        sum / (a.len() * chans.len()) as f64
    };
    let db = |m: f64| if m == 0.0 { 99.0 } else { 10.0 * (255.0f64 * 255.0 / m).log10() };
    match kind {
        Kind::Bc1 => db(mse(&[0, 1, 2])),
        Kind::Bc3 => db(mse(&[0, 1, 2])).min(db(mse(&[3]))),
        Kind::Bc5 => db(mse(&[0, 1])),
    }
}

fn write_png(path: &Path, w: u32, h: u32, px: &[[u8; 4]]) -> anyhow::Result<()> {
    let raw: Vec<u8> = px.iter().flatten().copied().collect();
    image::save_buffer(path, &raw, w, h, image::ExtendedColorType::Rgba8)?;
    Ok(())
}

/// 2x2 box filter.
fn half(w: u32, h: u32, px: &[[u8; 4]]) -> (u32, u32, Vec<[u8; 4]>) {
    let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
    let mut out = Vec::with_capacity((nw * nh) as usize);
    for y in 0..nh {
        for x in 0..nw {
            let mut acc = [0u32; 4];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let sx = (x * 2 + dx).min(w - 1);
                let sy = (y * 2 + dy).min(h - 1);
                let p = px[(sy * w + sx) as usize];
                for c in 0..4 {
                    acc[c] += p[c] as u32;
                }
            }
            out.push(acc.map(|v| ((v + 2) / 4) as u8));
        }
    }
    (nw, nh, out)
}

fn run(cmd: &mut Command) -> bool {
    cmd.env("OMP_NUM_THREADS", "1").output().map(|o| o.status.success()).unwrap_or(false)
}

const LABELS: [&str; 4] = ["etc1s q50", "etc1s q90", "rdo bc l3", "rdo bc l1"];

/// [`reencode`] through a disk cache (`AA_BAKE_CACHE`, default `tex-cache` in the working directory) keyed by
/// the texture's hash, the floor and the lossless size, so an interrupted bake picks up where it
/// stopped. Entry: label index (255 = keep lossless), codec, PSNR (f64 LE), blob.
pub fn reencode_cached(tools: &Tools, job: usize, data: &[u8], floor: f64, lossless: usize) -> Option<Choice> {
    let dir = PathBuf::from(std::env::var("AA_BAKE_CACHE").unwrap_or_else(|_| "tex-cache".into()));
    let hash = blake3::hash(data).to_hex();
    let path = dir.join(&hash[..2]).join(format!("{}-{floor}-{lossless}", &hash[2..32]));
    if let Ok(e) = std::fs::read(&path)
        && e.len() >= 10
    {
        return LABELS.get(e[0] as usize).map(|&label| Choice {
            label,
            codec: e[1],
            psnr: f64::from_le_bytes(e[2..10].try_into().unwrap()),
            blob: e[10..].to_vec(),
        });
    }
    let c = reencode(tools, job, data, floor, lossless);
    let mut e = match &c {
        Some(c) => vec![LABELS.iter().position(|&l| l == c.label).unwrap() as u8, c.codec],
        None => vec![255, 0],
    };
    e.extend_from_slice(&c.as_ref().map_or(0.0, |c| c.psnr).to_le_bytes());
    if let Some(c) = &c {
        e.extend_from_slice(&c.blob);
    }
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, &e).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
    c
}

/// Tries the ladder; `lossless` is the size of the blob the file gets otherwise.
pub fn reencode(tools: &Tools, job: usize, data: &[u8], floor: f64, lossless: usize) -> Option<Choice> {
    let (kind, w, h, mips, end) = parse(data)?;
    if kind == Kind::Bc1 && decode(kind, w, h, &data[128..], true).iter().any(|p| p[3] == 0) {
        return None; // cut-out alpha: the BC1 paths here would lose it
    }
    let orig = decode(kind, w, h, &data[128..], false);
    let dir = tools.tmp.join(format!("j{job}"));
    std::fs::create_dir_all(&dir).ok()?;
    let png = dir.join("top.png");
    write_png(&png, w, h, &orig).ok()?;

    let mut result = None;
    for step in LADDER {
        let candidate = match step {
            Step::Etc1s(q) => etc1s(tools, &dir, &png, kind, q, data, end),
            Step::Rdo(_) if w.max(h) < RDO_MIN => None,
            Step::Rdo(l) => rdo(tools, &dir, kind, (w, h, mips), &orig, l, data, end),
        };
        let Some((codec, blob, dds)) = candidate else { continue };
        let q = psnr(kind, &orig, &decode(kind, w, h, &dds[128..], false));
        if q >= floor {
            if blob.len() < lossless {
                let label = match step {
                    Step::Etc1s(50) => LABELS[0],
                    Step::Etc1s(_) => LABELS[1],
                    Step::Rdo(l) if l > 2.0 => LABELS[2],
                    Step::Rdo(_) => LABELS[3],
                };
                result = Some(Choice { label, codec, blob, psnr: q });
            }
            break;
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// (codec, blob, the DDS the reader will produce).
fn etc1s(tools: &Tools, dir: &Path, png: &Path, kind: Kind, q: u32, data: &[u8], end: usize) -> Option<(u8, Vec<u8>, Vec<u8>)> {
    let out = dir.join("e.ktx2");
    let mut cmd = Command::new(&tools.basisu);
    cmd.arg(png).args(["-etc1s", "-quality", &q.to_string(), "-mipmap", "-no_multithreading", "-output_file"]).arg(&out);
    if kind == Kind::Bc5 {
        cmd.args(["-linear", "-normal_map", "-separate_rg_to_color_alpha"]);
    }
    if !run(&mut cmd) {
        return None;
    }
    let mut blob = data[..128].to_vec();
    blob.extend_from_slice(&((data.len() - end) as u32).to_le_bytes());
    blob.extend_from_slice(&data[end..]);
    blob.extend_from_slice(&std::fs::read(&out).ok()?);
    let dds = baked::basis_to_dds(&blob).ok()?;
    Some((baked::BASIS, blob, dds))
}

#[allow(clippy::too_many_arguments)]
fn rdo(
    tools: &Tools,
    dir: &Path,
    kind: Kind,
    (w, h, mips): (u32, u32, u32),
    top: &[[u8; 4]],
    lambda: f32,
    data: &[u8],
    end: usize,
) -> Option<(u8, Vec<u8>, Vec<u8>)> {
    let mut dds = data[..128].to_vec();
    let (mut lw, mut lh, mut px) = (w, h, top.to_vec());
    for level in 0..mips {
        let png = dir.join(format!("m{level}.png"));
        write_png(&png, lw, lh, &px).ok()?;
        let mut cmd = Command::new(&tools.bc7enc);
        cmd.arg(&png).args([
            match kind {
                Kind::Bc1 => "-1",
                Kind::Bc3 => "-3",
                Kind::Bc5 => "-5",
            },
            "-g",
            "-q",
            "-o",
            "-zt",
        ]);
        if kind == Kind::Bc5 {
            cmd.args(["-X0", "-Y1"]);
        }
        // Mips under 16 px are a few bytes: plain re-encode.
        if lw.max(lh) >= 16 {
            cmd.args([format!("-z{lambda}"), "-zc1024".into(), "-zn".into()]);
        }
        if !run(&mut cmd) {
            return None;
        }
        let d = std::fs::read(png.with_extension("dds")).ok()?;
        let payload = if &d[84..88] == b"DX10" { &d[148..] } else { &d[128..] };
        if payload.len() != level_size(kind, lw, lh) {
            return None;
        }
        dds.extend_from_slice(payload);
        (lw, lh, px) = half(lw, lh, &px);
    }
    dds.extend_from_slice(&data[end..]);
    let split = baked::bc_split(&dds)?;
    let blob = zstd::bulk::compress(&split, 19).ok()?;
    Some((baked::ZSTD_BC_MIPS, blob, dds))
}
