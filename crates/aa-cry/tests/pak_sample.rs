//! Robustness and performance over random samples of the real pak (skipped when the client is
//! missing). Default runs use small samples; the full-size runs are `#[ignore]`d:
//!
//! ```text
//! cargo test -p aa-cry --test pak_sample -- --nocapture                 # quick samples
//! cargo test -p aa-cry --test pak_sample -- --ignored --nocapture       # 1,000 cgf + 300 chr, 5,000 dds
//! ```

use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant};

use aa_cry::texture::{Dds, DdsOptions};
use bevy::asset::RenderAssetUsages;

/// Deterministic sample of pak names with one of the extensions.
fn sample(exts: &[&str], n: usize, seed: u64) -> Vec<String> {
    let mut names: Vec<String> = aa_pak::Pak::global()
        .names()
        .map(aa_pak::normalize)
        .filter(|p| exts.iter().any(|e| p.ends_with(e)))
        .collect();
    names.sort();
    let mut state = seed;
    let mut next = || {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (state >> 33) as usize
    };
    // partial Fisher-Yates
    let n = n.min(names.len());
    for i in 0..n {
        let j = i + next() % (names.len() - i);
        names.swap(i, j);
    }
    names.truncate(n);
    names
}

/// Error message -> short cause (drop numbers so similar errors group).
fn cause(e: &str) -> String {
    let s: String = e.chars().map(|c| if c.is_ascii_digit() { '#' } else { c }).collect();
    s.split(" at ").next().unwrap_or(&s).chars().take(90).collect()
}

fn report(title: &str, total: usize, failures: &BTreeMap<String, Vec<String>>, elapsed: Duration, bytes: u64) {
    let failed: usize = failures.values().map(Vec::len).sum();
    println!(
        "{title}: {} ok / {total}, {failed} failed, {:.2} s ({:.2} ms/file, {:.1} MB/s)",
        total - failed,
        elapsed.as_secs_f64(),
        elapsed.as_secs_f64() * 1000.0 / total.max(1) as f64,
        bytes as f64 / 1e6 / elapsed.as_secs_f64().max(1e-9)
    );
    for (c, files) in failures {
        println!("  {:5} x {c}   e.g. {}", files.len(), files[0]);
    }
}

/// Loads models like `spawn_model` does on the CPU side: parse, skeleton (LOD0 fallback),
/// material file, Bevy meshes per subset. Returns (bytes read, subsets built).
fn load_model_cpu(path: &str) -> aa_cry::Result<(u64, usize)> {
    let bytes = aa_cry::read(path)?;
    let model = aa_cry::Model::from_bytes(&bytes)?;
    let skeleton = match &model.skeleton {
        Some(_) => None,
        None if model.is_skinned() => Some(aa_cry::Skeleton::load(path)?),
        None => None,
    };
    let n_bones = model.skeleton.as_ref().or(skeleton.as_ref()).map(|s| s.len()).unwrap_or(0);
    if let Some(mtl) = aa_cry::scene::find_mtl(path, &model) {
        aa_cry::material::Material::load(&mtl)?;
    }
    let mut built = 0;
    for ni in model.lod_nodes(0) {
        let mesh = &model.meshes[model.nodes[ni].mesh.unwrap()];
        if let Some(skin) = &mesh.skin {
            if n_bones == 0 {
                return Err(aa_cry::Error::Invalid("skinned mesh without skeleton".into()));
            }
            if let Some(j) = skin.joints.iter().flatten().find(|&&j| j as usize >= n_bones) {
                return Err(aa_cry::Error::Invalid(format!("joint {j} >= {n_bones} bones")));
            }
        }
        for s in &mesh.subsets {
            if aa_cry::scene::subset_mesh(mesh, s, mesh.skin.is_some(), RenderAssetUsages::RENDER_WORLD).is_some() {
                built += 1;
            }
        }
    }
    Ok((bytes.len() as u64, built))
}

fn run_models(n_cgf: usize, n_chr: usize) {
    if !aa_cry::pak_available() {
        return;
    }
    let mut files = sample(&[".cgf"], n_cgf, 1);
    files.extend(sample(&[".chr", ".skin"], n_chr, 2));
    let mut failures: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut panics = Vec::new();
    let (mut bytes, mut subsets) = (0u64, 0usize);
    let t0 = Instant::now();
    for f in &files {
        match catch_unwind(AssertUnwindSafe(|| load_model_cpu(f))) {
            Ok(Ok((b, s))) => {
                bytes += b;
                subsets += s;
            }
            Ok(Err(e)) => failures.entry(cause(&e.to_string())).or_default().push(f.clone()),
            Err(_) => panics.push(f.clone()),
        }
    }
    let elapsed = t0.elapsed();
    report(&format!("models ({n_cgf} cgf + {n_chr} chr, {subsets} subsets)"), files.len(), &failures, elapsed, bytes);
    assert!(panics.is_empty(), "panics: {panics:?}");
    let failed: usize = failures.values().map(Vec::len).sum();
    // Known: ~0.3% legacy uncompiled meshes (Mesh 0x744) and broken files.
    assert!(failed * 100 <= files.len(), "{failed} of {} failed", files.len());
}

fn run_dds(n: usize) {
    if !aa_cry::pak_available() {
        return;
    }
    let files = sample(&[".dds"], n, 3);
    let mut formats: BTreeMap<String, usize> = BTreeMap::new();
    let mut failures: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut panics = Vec::new();
    let mut bytes = 0u64;
    let t0 = Instant::now();
    for f in &files {
        let r = catch_unwind(AssertUnwindSafe(|| -> aa_cry::Result<String> {
            let b = aa_cry::read(f)?;
            bytes += b.len() as u64;
            let dds = Dds::parse(&b)?;
            let img = aa_cry::texture::dds_to_image(&dds, DdsOptions::default())?;
            // also exercise the CPU decoder on the smallest mip
            let _ = dds.decode_rgba8(0, dds.mips - 1);
            let mut kind = format!("{:?}", dds.format);
            if dds.swapped_rg {
                kind += " (ATI2)";
            }
            if dds.is_cubemap() {
                kind += " cube";
            }
            if dds.depth > 1 {
                kind += " volume";
            }
            if dds.attached.is_some() {
                kind += " +attached";
            }
            assert!(img.data.as_ref().is_some_and(|d| !d.is_empty()));
            Ok(kind)
        }));
        match r {
            Ok(Ok(k)) => *formats.entry(k).or_default() += 1,
            Ok(Err(e)) => failures.entry(cause(&e.to_string())).or_default().push(f.clone()),
            Err(_) => panics.push(f.clone()),
        }
    }
    let elapsed = t0.elapsed();
    report(&format!("dds ({n} sampled)"), files.len(), &failures, elapsed, bytes);
    for (k, v) in &formats {
        println!("  {v:6}  {k}");
    }
    assert!(panics.is_empty(), "panics: {panics:?}");
    assert!(failures.is_empty(), "failures: {failures:?}");
}

#[test]
fn models_quick_sample() {
    run_models(150, 50);
}

#[test]
#[ignore]
fn models_1000_cgf_300_chr() {
    run_models(1000, 300);
}

#[test]
fn dds_quick_sample() {
    run_dds(400);
}

#[test]
#[ignore]
fn dds_large_sample() {
    run_dds(5000);
}
