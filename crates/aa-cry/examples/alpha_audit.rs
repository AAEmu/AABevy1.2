//! Audit the alpha-test elision against every decoded mip of its actual source textures.
//! `cargo run -p aa-cry --example alpha_audit -- data/extract/aa-cry/mtl_all.tsv <out.tsv>`
use aa_cry::material::{Material, Texture, TextureMap};
use aa_cry::texture::Dds;
use bevy::prelude::AlphaMode;
use std::collections::HashMap;
use std::io::Write;

struct Alpha {
    width: u32,
    height: u32,
    mean: f32,
    opaque: bool,
    minima: Vec<u8>,
    below: Vec<Vec<u32>>,
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let inventory = std::fs::read_to_string(&args[1]).unwrap();
    let mut out = std::io::BufWriter::new(std::fs::File::create(&args[2]).unwrap());
    writeln!(out, "material\tsub\tshader\ttexture_threshold\ttexture\tsize\tsmallest_mip_mean\tmip_minima\tbase_below_test\tall_mips_below_test").unwrap();
    let mut cache: HashMap<String, Option<Alpha>> = HashMap::new();
    let (mut tested, mut elided, mut unsafe_elisions, mut proven_opaque) = (0, 0, 0, 0);
    for line in inventory.lines().skip(1) {
        let f: Vec<_> = line.split('\t').collect();
        if f[3].to_ascii_lowercase().starts_with("veg") || f[3].eq_ignore_ascii_case("grass") {
            continue;
        }
        let Some(texture) = f[14].split(';').find_map(|s| s.strip_prefix("Diffuse=")) else {
            continue;
        };
        let texture = aa_cry::material::texture_pak_path(texture);
        let material = Material {
            shader: f[3].into(),
            flags: f[4].parse().unwrap(),
            opacity: f[11].parse().unwrap(),
            alpha_test: f[12].parse().unwrap(),
            textures: vec![Texture {
                map: TextureMap::Diffuse,
                file: texture.clone(),
                amount: None,
                tex_mod: None,
                attributes: Vec::new(),
            }],
            ..Default::default()
        };
        let Some(converted) = aa_cry::shade::to_standard_material(&material, |_, _| None) else {
            continue;
        };
        let AlphaMode::Mask(threshold) = converted.alpha_mode else {
            continue;
        };
        let alpha = threshold / converted.base_color.to_linear().alpha;
        let a = cache.entry(texture.clone()).or_insert_with(|| {
            let bytes = aa_cry::read(&texture).ok()?;
            let dds = Dds::parse(&bytes).ok()?;
            if dds.faces != 1 || dds.depth != 1 {
                return None;
            }
            let last = dds.decode_rgba8(0, dds.mips - 1);
            let mean = last.chunks_exact(4).map(|p| p[3] as f32).sum::<f32>()
                / (255.0 * (last.len() / 4) as f32);
            let mut minima = Vec::new();
            let mut below = Vec::new();
            if mean >= 0.98 {
                for mip in 0..dds.mips {
                    let pixels = dds.decode_rgba8(0, mip);
                    let mut histogram = vec![0; 256];
                    for p in pixels.chunks_exact(4) {
                        histogram[p[3] as usize] += 1
                    }
                    minima.push(histogram.iter().position(|n| *n != 0).unwrap_or(255) as u8);
                    below.push(histogram);
                }
            }
            Some(Alpha {
                width: dds.width,
                height: dds.height,
                mean,
                opaque: dds.alpha_is_opaque(),
                minima,
                below,
            })
        });
        let Some(a) = a else { continue };
        tested += 1;
        if a.mean < 0.98 {
            continue;
        }
        elided += 1;
        if a.opaque {
            proven_opaque += 1
        }
        let count = |histogram: &Vec<u32>| {
            histogram
                .iter()
                .enumerate()
                .filter(|(i, _)| (*i as f32 / 255.0) < alpha)
                .map(|(_, n)| *n as u64)
                .sum::<u64>()
        };
        let total: u64 = a.below.iter().map(count).sum();
        if total == 0 {
            continue;
        }
        unsafe_elisions += 1;
        writeln!(
            out,
            "{}\t{}\t{}\t{alpha}\t{texture}\t{}x{}\t{}\t{:?}\t{}\t{total}",
            f[0],
            f[1],
            f[3],
            a.width,
            a.height,
            a.mean,
            a.minima,
            count(&a.below[0])
        )
        .unwrap();
    }
    println!(
        "tested {tested} material slots / {} textures; mean-based elisions {elided}; proven opaque {proven_opaque}; elisions with below-threshold texels {unsafe_elisions}",
        cache.len()
    );
}
