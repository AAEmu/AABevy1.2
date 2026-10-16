//! Statistics of the `Cut` hull of a costume model: `cargo run -p aa-cry --example cut_info -- <model.chr|cgf>`.
use aa_cry::Model;
use aa_cry::material::Material;

fn main() {
    let path = std::env::args().nth(1).expect("model path");
    let model = Model::load(&path).expect("model");
    let mtl_path = std::env::args().nth(2).unwrap_or_else(|| path.rsplit_once('.').map(|(b, _)| format!("{b}.mtl")).unwrap());
    let mtl = Material::load(&mtl_path).expect("mtl");
    for (mi, mesh) in model.meshes.iter().enumerate() {
        println!("mesh {mi}: {} verts {} indices skin={}", mesh.positions.len(), mesh.indices.len(), mesh.skin.is_some());
        for s in &mesh.subsets {
            let shader = mtl.sub(s.material_id as usize).map(|m| m.shader.clone()).unwrap_or_default();
            println!("  subset mat {} ({shader}) first_index {} count {} verts {}+{}", s.material_id, s.first_index, s.index_count, s.first_vertex, s.vertex_count);
            if shader.eq_ignore_ascii_case("cut") {
                let weld = |i: u32| {
                    let p = mesh.positions[i as usize];
                    ((p.x * 1000.0).round() as i32, (p.y * 1000.0).round() as i32, (p.z * 1000.0).round() as i32)
                };
                let mut ids = std::collections::HashMap::new();
                let mut wid = |i: u32| {
                    let n = ids.len() as u32;
                    *ids.entry(weld(i)).or_insert(n)
                };
                let tris: Vec<[u32; 3]> = mesh.indices[s.first_index as usize..(s.first_index + s.index_count) as usize].chunks(3).map(|t| [wid(t[0]), wid(t[1]), wid(t[2])]).collect();
                println!("    welded vertices: {}", ids.len());
                let orig: Vec<[u32; 3]> = mesh.indices[s.first_index as usize..(s.first_index + s.index_count) as usize].chunks(3).map(|t| [t[0], t[1], t[2]]).collect();
                let mut edges = std::collections::HashMap::<(u32, u32), i32>::new();
                for t in &tris {
                    for k in 0..3 {
                        let (a, b) = (t[k], t[(k + 1) % 3]);
                        *edges.entry((a.min(b), a.max(b))).or_default() += if a < b { 1 } else { -1 };
                    }
                }
                let open = edges.values().filter(|&&v| v != 0).count();
                let mut lo = glam::Vec3::splat(1e9);
                let mut hi = glam::Vec3::splat(-1e9);
                for t in &orig {
                    for &i in t {
                        lo = lo.min(mesh.positions[i as usize]);
                        hi = hi.max(mesh.positions[i as usize]);
                    }
                }
                println!("    {} triangles, {} edges, {} edges with inconsistent winding/open, bbox {lo:?} .. {hi:?}", tris.len(), edges.len(), open);
            }
        }
    }
}
