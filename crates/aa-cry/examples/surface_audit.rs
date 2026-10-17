//! CPU-only collision surface audit against the installed game pak.
fn main() {
    let pak = aa_pak::Pak::global();
    let mut names: Vec<_> =
        pak.names().map(aa_pak::normalize).filter(|n| n.starts_with("game/objects/env/") && n.ends_with(".cgf") && !n.contains("_lod")).collect();
    names.sort();
    let mut meshes = 0;
    let mut bad = 0;
    let mut different = 0;
    let mut unreadable = 0;
    let mut missing_material = 0;
    for path in names {
        let mesh = match aa_cry::physics::load(&path, false) {
            Ok(Some(mesh)) => mesh,
            Ok(None) => continue,
            Err(error) => {
                unreadable += 1;
                eprintln!("{path}: {error}");
                continue;
            }
        };
        meshes += 1;
        if let Ok(render) = aa_cry::Model::load(&path) {
            if render.material_name != mesh.material_name {
                different += 1;
                if different <= 20 {
                    println!("different material for {path}: render {:?}, collision {:?}", render.material_name, mesh.material_name);
                }
            }
        }
        assert_eq!(mesh.materials.len(), mesh.indices.len() / 3, "{path}");
        assert!(mesh.indices.iter().all(|&id| (id as usize) < mesh.positions.len()), "{path}: invalid vertex index");
        let material = aa_cry::physics::material_paths(&path, mesh.material_name.as_deref())
            .into_iter()
            .find(|p| pak.exists(p))
            .and_then(|p| aa_cry::material::Material::load(&p).ok());
        if let Some(material) = material {
            let invalid: Vec<_> = mesh.materials.iter().copied().filter(|&id| material.sub(id as usize).is_none()).collect();
            if !invalid.is_empty() {
                bad += 1;
                if bad <= 20 {
                    println!(
                        "{path}: {} invalid of {}, ids {:?}, name {:?}",
                        invalid.len(),
                        mesh.materials.len(),
                        &invalid[..invalid.len().min(8)],
                        mesh.material_name
                    );
                }
            }
        } else {
            missing_material += 1;
        }
    }
    println!("{meshes} collision meshes; {bad} with invalid surface references; {different} differ from rendering");
    println!("{unreadable} unreadable models; {missing_material} missing materials");
}
