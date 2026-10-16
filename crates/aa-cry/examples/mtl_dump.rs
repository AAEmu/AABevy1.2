//! Dumps every sub-material of every `.mtl` of the pak as TSV (one row per material):
//! `cargo run -p aa-cry --example mtl_dump -- data/extract/aa-cry/mtl_all.tsv [path prefix]`
//!
//! Columns: file, sub, name, shader, flags, genmask, diffuse (r,g,b), specular, emissive,
//! shininess, specular_level, opacity, alpha_test, glow, textures (`Map=file;...`), public
//! params (`k=v;...`). Used to survey which shader parameters occur and with which values.

use std::io::Write;

use aa_cry::material::Material;

fn main() {
    let out = std::env::args().nth(1).expect("output .tsv");
    let prefix = std::env::args().nth(2).unwrap_or_default().to_ascii_lowercase();
    let pak = aa_pak::Pak::global();
    let mut names: Vec<String> = pak.names().map(aa_pak::normalize).filter(|n| n.ends_with(".mtl") && n.starts_with(&prefix)).collect();
    names.sort();
    let mut w = std::io::BufWriter::new(std::fs::File::create(&out).unwrap());
    writeln!(w, "file\tsub\tname\tshader\tflags\tgenmask\tdiffuse\tspecular\temissive\tshininess\tspecular_level\topacity\talpha_test\tglow\ttextures\tparams").unwrap();
    let v3 = |v: glam::Vec3| format!("{:.3},{:.3},{:.3}", v.x, v.y, v.z);
    let mut rows = 0;
    for n in &names {
        let Ok(m) = Material::load(n) else { continue };
        let subs: Vec<&Material> = if m.sub_materials.is_empty() { vec![&m] } else { m.sub_materials.iter().collect() };
        for (i, s) in subs.iter().enumerate() {
            let tex: Vec<String> = s.textures.iter().map(|t| format!("{:?}={}", t.map, t.file)).collect();
            let params: Vec<String> = s.public_params.iter().map(|(k, v)| format!("{k}={v}")).collect();
            writeln!(
                w,
                "{n}\t{i}\t{}\t{}\t{}\t{:x}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                s.name, s.shader, s.flags, s.gen_mask, v3(s.diffuse), v3(s.specular), v3(s.emissive), s.shininess,
                s.specular_level, s.opacity, s.alpha_test, s.glow_amount, tex.join(";"), params.join(";")
            )
            .unwrap();
            rows += 1;
        }
    }
    eprintln!("{} files, {rows} materials -> {out}", names.len());
}
