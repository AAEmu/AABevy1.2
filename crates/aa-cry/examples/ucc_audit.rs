//! CPU-only UCC material and source-linked geometry audit. Reads the pak; writes TSV reports.
//! `cargo run -p aa-cry --example ucc_audit -- data/extract/aa-cry/ucc/audit`

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Write;
use std::sync::Arc;

use aa_cry::Model;
use aa_cry::material::{Material, TexMod, TextureMap};

fn canonical(path: &str) -> String {
    let p = aa_pak::normalize(path);
    if p.starts_with("game/") {
        p
    } else {
        format!("game/{p}")
    }
}

fn main() {
    let out = std::env::args().nth(1).expect("output prefix");
    let pak = aa_pak::Pak::global();
    let mut flagged = BTreeMap::<String, Arc<Material>>::new();
    let mut mtl_files = 0;
    let mut slots = 0;
    let mut cloth = 0;
    let mut masks = BTreeMap::<u64, usize>::new();
    for path in pak.names().filter(|p| p.ends_with(".mtl")) {
        mtl_files += 1;
        let m = Material::load(path).expect(path);
        let subs: Vec<_> = if m.sub_materials.is_empty() {
            vec![&m]
        } else {
            m.sub_materials.iter().collect()
        };
        for sub in subs {
            slots += 1;
            if sub.shader.eq_ignore_ascii_case("cloth") {
                cloth += 1;
                if sub.gen_mask & 0x10_0000 != 0 {
                    *masks.entry(sub.gen_mask).or_default() += 1;
                }
            }
        }
        if is_ucc(&m) || m.sub_materials.iter().any(is_ucc) {
            flagged.insert(canonical(path), Arc::new(m));
        }
    }
    let dirs: BTreeSet<_> = flagged
        .keys()
        .filter_map(|p| p.rsplit_once('/').map(|(d, _)| d.to_string()))
        .collect();
    let mut models = HashMap::<String, Arc<Model>>::new();
    let mut bindings = BTreeMap::<(String, String), &'static str>::new();
    let mut candidates = 0;
    let mut failed = 0;
    for path in pak
        .names()
        .filter(|p| p.ends_with(".cgf") || p.ends_with(".chr") || p.ends_with(".cga"))
    {
        let path = canonical(path);
        if !path.rsplit_once('/').is_some_and(|(d, _)| dirs.contains(d)) {
            continue;
        }
        candidates += 1;
        let Ok(model) = Model::load(&path) else {
            failed += 1;
            continue;
        };
        if let Some(mtl) = aa_cry::scene::find_mtl(&path, &model).map(|p| canonical(&p)) {
            if flagged.contains_key(&mtl) {
                bindings.insert((path.clone(), mtl), "model");
                models.insert(path, Arc::new(model));
            }
        }
    }
    // Cloak CDFs explicitly bind models from a separate obj/ directory to these materials.
    for mtl in flagged.keys() {
        let cdf = format!("{}.cdf", mtl.strip_suffix(".mtl").unwrap());
        if !pak.exists(&cdf) {
            continue;
        }
        let bytes = pak.read(&cdf).unwrap();
        let xml = String::from_utf8_lossy(&bytes);
        let doc = roxmltree::Document::parse(&xml).unwrap();
        for node in doc.descendants().filter(|n| n.has_tag_name("Model")) {
            let Some(path) = node.attribute("File").map(canonical) else {
                continue;
            };
            let Some(material) = node.attribute("Material").map(canonical) else {
                continue;
            };
            if material != *mtl {
                continue;
            }
            if !models.contains_key(&path) {
                let Ok(model) = Model::load(&path) else {
                    failed += 1;
                    continue;
                };
                models.insert(path.clone(), Arc::new(model));
            }
            bindings.insert((path, material), "cdf");
        }
    }
    // The dock cell's explicit override, verified in brush.dat/model#71/material#54.
    let sail = canonical("objects/env/06_unit/01_ship/galleon/galleon_deco_sail01.cgf");
    models
        .entry(sail.clone())
        .or_insert_with(|| Arc::new(Model::load(&sail).unwrap()));
    bindings.insert(
        (
            sail,
            canonical("objects/env/06_unit/01_ship/galleon/galleon_mahadevi.mtl"),
        ),
        "dock override",
    );

    let mut material_report =
        std::io::BufWriter::new(std::fs::File::create(format!("{out}_materials.tsv")).unwrap());
    let mut geometry_report =
        std::io::BufWriter::new(std::fs::File::create(format!("{out}_geometry.tsv")).unwrap());
    writeln!(material_report, "material\tsub\tgenmask\tbackground\tcustom\tinputs_present\ttransforms_supported\tlinked_subsets\tuv_square\toutside_or_missing_uv").unwrap();
    writeln!(
        geometry_report,
        "model\tmaterial\tsub\tbinding\tmesh\tfirst_index\tindices\tuv_square\tuv_min\tuv_max"
    )
    .unwrap();
    let mut totals = [0; 9];
    for (mtl, material) in &flagged {
        let subs: Vec<_> = if material.sub_materials.is_empty() {
            vec![material.as_ref()]
        } else {
            material.sub_materials.iter().collect()
        };
        for (sub, m) in subs.into_iter().enumerate().filter(|(_, m)| is_ucc(m)) {
            let custom = m.texture(&TextureMap::Custom).is_some();
            let background = m.gen_mask & 0x20_0000 != 0 && m.gen_mask & 0x40_0000 == 0;
            let inputs = m.texture(&TextureMap::Diffuse).is_some()
                && m.texture(&TextureMap::SubSurface).is_some();
            let supported = m
                .texture(&TextureMap::Diffuse)
                .is_none_or(|t| t.tex_mod.as_ref().is_none_or(|t| *t == TexMod::default()))
                && m.texture(&TextureMap::SubSurface).is_none_or(|t| {
                    t.tex_mod
                        .as_ref()
                        .is_none_or(|t| t.rotate_w == 0.0 && t.unhandled.is_empty())
                });
            let (mut good, mut bad) = (0, 0);
            for ((path, used_mtl), origin) in &bindings {
                if used_mtl != mtl {
                    continue;
                }
                for (mi, mesh) in models[path].meshes.iter().enumerate() {
                    for subset in mesh
                        .subsets
                        .iter()
                        .filter(|s| s.material_id as usize == sub)
                    {
                        let square = mesh.subset_uvs_in_unit_square(subset);
                        let mut uv_min = glam::Vec2::splat(f32::MAX);
                        let mut uv_max = glam::Vec2::splat(f32::MIN);
                        for &idx in &mesh.indices[subset.first_index as usize
                            ..(subset.first_index + subset.index_count) as usize]
                        {
                            if let Some(uv) = mesh.uvs.get(idx as usize) {
                                uv_min = uv_min.min(*uv);
                                uv_max = uv_max.max(*uv);
                            }
                        }
                        if square {
                            good += 1;
                        } else {
                            bad += 1;
                        }
                        writeln!(
                            geometry_report,
                            "{path}\t{mtl}\t{sub}\t{origin}\t{mi}\t{}\t{}\t{square}\t{},{}\t{},{}",
                            subset.first_index,
                            subset.index_count,
                            uv_min.x,
                            uv_min.y,
                            uv_max.x,
                            uv_max.y
                        )
                        .unwrap();
                    }
                }
            }
            writeln!(
                material_report,
                "{mtl}\t{sub}\t{:x}\t{background}\t{custom}\t{inputs}\t{supported}\t{}\t{good}\t{bad}",
                m.gen_mask,
                good + bad
            )
            .unwrap();
            totals[0] += 1;
            totals[1] += usize::from(background);
            totals[2] += usize::from(custom);
            totals[3] += usize::from(!supported);
            totals[4] += good;
            totals[5] += bad;
            totals[6] += usize::from(good + bad > 0);
            totals[7] += usize::from(background && custom);
            totals[8] += usize::from(!inputs);
        }
    }
    println!(
        "{mtl_files} material files, {slots} slots, {cloth} Cloth, {} UCC; masks={}",
        totals[0],
        masks
            .iter()
            .map(|(mask, count)| format!("0x{mask:x}={count}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "{} active background, {} Custom ({} active background), {} unsupported transforms, {} missing inputs",
        totals[1], totals[2], totals[7], totals[3], totals[8]
    );
    println!(
        "{candidates} same-directory model candidates; {} source-linked bindings / {} models; {failed} parse/load failures",
        bindings.len(),
        models.len()
    );
    println!(
        "{} UCC materials linked: {} UV-square subsets, {} unsupported subsets; {} without proven model binding",
        totals[6],
        totals[4],
        totals[5],
        totals[0] - totals[6]
    );
}

fn is_ucc(m: &Material) -> bool {
    m.shader.eq_ignore_ascii_case("cloth") && m.gen_mask & 0x10_0000 != 0
}
