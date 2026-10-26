//! The files a model is about to read, named ahead of the read so the browser can download their blocks in the
//! background first ([`crate::workers::Workers::spawn_after`]). A good guess, not a contract: anything missed is
//! still read, the blocking way.

use crate::material::Material;

/// The model file, its sibling `.mtl` and a material override (when given), as they exist in the pak.
pub fn model_files(path: &str, material: Option<&str>) -> Vec<String> {
    let mut out = vec![path.to_string()];
    if let Some((stem, _)) = path.rsplit_once('.') {
        out.push(format!("{stem}.mtl"));
    }
    if let Some(m) = material {
        out.push(if m.to_ascii_lowercase().ends_with(".mtl") { m.to_string() } else { format!("{m}.mtl") });
    }
    let pak = aa_pak::Pak::global();
    out.retain(|p| pak.exists(p));
    out
}

/// The textures of the `.mtl` files among `files` (sub-materials included); read them only once
/// [`model_files`] are local.
pub fn texture_files(files: &[String]) -> Vec<String> {
    fn walk(m: &Material, out: &mut Vec<String>) {
        out.extend(m.textures.iter().map(|t| t.pak_path()).filter(|p| !p.is_empty()));
        for sub in &m.sub_materials {
            walk(sub, out);
        }
    }
    let mut out = Vec::new();
    for f in files.iter().filter(|f| f.to_ascii_lowercase().ends_with(".mtl")) {
        if let Ok(m) = Material::load(f) {
            walk(&m, &mut out);
        }
    }
    out.sort();
    out.dedup();
    out
}
