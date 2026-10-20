//! vegetation.cfx403/780: normalized vertex alpha multiplies final lit RGB, not opacity.
use aa_cry::{
    CryLoader,
    material::Material,
    mesh::{MeshData, Subset},
    scene::LoadedModel,
};
use bevy::{
    asset::{AssetId, RenderAssetUsages},
    prelude::*,
};
use std::collections::HashMap;

/// Only parts with a nonneutral authored alpha stream receive a shared AO mesh mirror.
#[derive(Component, Clone, Copy, Debug)]
pub struct VegetationAoPart {
    pub cell: (i32, i32),
    pub min_alpha: u8,
}
pub(crate) struct Prepared {
    source: AssetId<Mesh>,
    mesh: Mesh,
    min_alpha: u8,
}
pub(crate) struct Shared {
    pub mesh: Handle<Mesh>,
    pub min_alpha: u8,
}
pub(crate) type Meshes = HashMap<AssetId<Mesh>, Shared>;
pub(crate) type Mirrors = HashMap<AssetId<Mesh>, (AssetId<Mesh>, u8)>;

// Bounds additional worker memory; oversized meshes keep the existing unmodified path.
const MAX_VERTICES: usize = 262_144;
const MAX_INDICES: usize = 1_572_864;

fn supported(m: &Material) -> bool {
    m.shader.eq_ignore_ascii_case("Vegetation") && m.gen_mask & 0x4000_0000 == 0
}

/// Exact subset compaction order used by aa-cry, including mirrored triangle winding:
/// the source vertex index of every vertex of the built subset mesh.
fn compact_order(m: &MeshData, subset: &Subset, flip: bool) -> Option<Vec<usize>> {
    if m.colors.len() != m.positions.len() || m.positions.len() > MAX_VERTICES {
        return None;
    }
    let end = subset.first_index.checked_add(subset.index_count)? as usize;
    let indices = m.indices.get(subset.first_index as usize..end)?;
    if indices.is_empty() || indices.len() > MAX_INDICES {
        return None;
    }
    let mut seen = vec![false; m.positions.len()];
    let mut order = Vec::new();
    for tri in indices.chunks_exact(3) {
        let tri = if flip {
            [tri[0], tri[2], tri[1]]
        } else {
            [tri[0], tri[1], tri[2]]
        };
        for i in tri {
            let i = i as usize;
            if !*seen.get(i)? {
                seen[i] = true;
                order.push(i);
            }
        }
    }
    Some(order)
}

/// Vertex alpha as UV1.x in compaction order, with the minimum; `None` when every alpha is neutral.
fn alpha_uvs(m: &MeshData, subset: &Subset, flip: bool) -> Option<(Vec<[f32; 2]>, u8)> {
    let order = compact_order(m, subset, flip)?;
    let minimum = order.iter().map(|&i| m.colors[i][3]).min()?;
    let uv = order.iter().map(|&i| [f32::from(m.colors[i][3]) / 255.0, 0.0]).collect();
    (minimum < 255).then_some((uv, minimum))
}

/// Detail-bending vertex data (R, G, B, quantised normal xy) in compaction order, see
/// `vegetation_wind::pack_detail`. Stored as COLOR, which the vegetation vertex shader never forwards.
fn detail_colors(m: &MeshData, order: &[usize]) -> Vec<[f32; 4]> {
    order
        .iter()
        .map(|&i| {
            let c = m.colors[i];
            crate::vegetation_wind::pack_detail([c[0], c[1], c[2]], m.normals.get(i).copied().unwrap_or(Vec3::ZERO))
        })
        .collect()
}

/// Runs inside the existing bounded model-worker queue. No source mesh/material is mutated.
pub(crate) fn prepare(loader: &CryLoader, loaded: &LoadedModel) -> Vec<Prepared> {
    let Some(mtl) = loaded.material_path.as_deref().and_then(|p| loader.mtl(p)) else {
        return Vec::new();
    };
    prepare_for_material(loaded, &mtl)
}

fn prepare_for_material(loaded: &LoadedModel, mtl: &Material) -> Vec<Prepared> {
    let mut result = Vec::new();
    let mut vertices = 0;
    let mut indices = 0;
    for (node, parts) in &loaded.parts {
        let Some(mi) = loaded.model.nodes[*node].mesh else {
            continue;
        };
        let source = &loaded.model.meshes[mi];
        // Match the exact current CryLoader subset filtering before mapping compact meshes.
        let drawn: Vec<_> = source
            .subsets
            .iter()
            .filter(|s| {
                let m = mtl.sub(s.material_id as usize).unwrap_or(mtl);
                !m.no_draw() && !m.shader.eq_ignore_ascii_case("Cut") && s.index_count != 0
            })
            .collect();
        if drawn.len() != parts.len() {
            continue;
        }
        let flip = aa_cry::scene::render_transform(&loaded.model, *node)
            .matrix3
            .determinant()
            < 0.0;
        for (subset, part) in drawn.into_iter().zip(parts) {
            if part.skinned || !supported(mtl.sub(subset.material_id as usize).unwrap_or(mtl)) {
                continue;
            }
            let sub = mtl.sub(subset.material_id as usize).unwrap_or(mtl);
            let Some(order) = compact_order(source, subset, flip) else {
                continue;
            };
            // Neutral alpha without detail bending needs no mirror.
            let (uv, min_alpha) = alpha_uvs(source, subset, flip)
                .unwrap_or_else(|| (vec![[1.0, 0.0]; order.len()], 255));
            let detail = crate::vegetation_wind::bends(sub).then(|| detail_colors(source, &order));
            if min_alpha == 255 && detail.is_none() {
                continue;
            }
            if vertices + uv.len() > MAX_VERTICES
                || indices + subset.index_count as usize > MAX_INDICES
            {
                continue;
            }
            let Some((mut mesh, _)) = aa_cry::scene::subset_mesh_with(
                source,
                subset,
                false,
                RenderAssetUsages::RENDER_WORLD,
                flip,
            ) else {
                continue;
            };
            if mesh.count_vertices() != uv.len() {
                continue;
            }
            vertices += uv.len();
            indices += subset.index_count as usize;
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, uv);
            if let Some(detail) = detail {
                mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, detail);
            }
            result.push(Prepared {
                source: part.mesh.id(),
                mesh,
                min_alpha,
            });
        }
    }
    result
}

pub(crate) fn finish(
    prepared: Vec<Prepared>,
    meshes: &mut Assets<Mesh>,
    mirrors: &mut Mirrors,
) -> Meshes {
    prepared
        .into_iter()
        .map(|p| {
            let mesh = mirrors
                .get(&p.source)
                .and_then(|(id, _)| meshes.get_strong_handle(*id))
                .unwrap_or_else(|| meshes.add(p.mesh));
            mirrors.insert(p.source, (mesh.id(), p.min_alpha));
            (
                p.source,
                Shared {
                    mesh,
                    min_alpha: p.min_alpha,
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compact_alpha_tracks_mirrored_positions_without_changing_opacity() {
        let m = MeshData {
            positions: vec![Vec3::X, Vec3::Y, Vec3::Z, Vec3::ONE],
            normals: vec![Vec3::Z; 4],
            colors: vec![
                [10, 20, 30, 64],
                [40, 50, 60, 128],
                [70, 80, 90, 255],
                [1, 2, 3, 0],
            ],
            indices: vec![2, 0, 1, 1, 0, 2],
            ..default()
        };
        let s = Subset {
            first_index: 0,
            index_count: 6,
            first_vertex: 0,
            vertex_count: 3,
            material_id: 0,
            radius: 1.0,
            center: Vec3::ZERO,
        };
        for flip in [false, true] {
            let (uv, min) = alpha_uvs(&m, &s, flip).unwrap();
            assert_eq!(min, 64); // unreferenced alpha0 must not influence this subset.
            let (built, _) =
                aa_cry::scene::subset_mesh_with(&m, &s, false, RenderAssetUsages::MAIN_WORLD, flip)
                    .unwrap();
            let bevy::mesh::VertexAttributeValues::Float32x3(pos) =
                built.attribute(Mesh::ATTRIBUTE_POSITION).unwrap()
            else {
                panic!()
            };
            for (p, u) in pos.iter().zip(&uv) {
                let original = m.positions.iter().position(|v| v.to_array() == *p).unwrap();
                assert_eq!(u[0], f32::from(m.colors[original][3]) / 255.0);
            }
            assert!(built.attribute(Mesh::ATTRIBUTE_COLOR).is_none());
        }
        let mut invalid = m.clone();
        invalid.colors.clear();
        assert!(alpha_uvs(&invalid, &s, false).is_none());
        invalid.colors = vec![[0, 0, 0, 255]; 4];
        assert!(alpha_uvs(&invalid, &s, false).is_none());
        let mut out = s;
        out.first_index = u32::MAX;
        assert!(alpha_uvs(&m, &out, false).is_none());
        let mut oversized = m.clone();
        oversized.positions.resize(MAX_VERTICES + 1, Vec3::ZERO);
        oversized.colors.resize(MAX_VERTICES + 1, [0, 0, 0, 128]);
        assert!(alpha_uvs(&oversized, &s, false).is_none());
        assert!(supported(&Material {
            shader: "Vegetation".into(),
            ..default()
        }));
        assert!(!supported(&Material {
            shader: "Vegetation".into(),
            gen_mask: 0x4000_0000,
            ..default()
        }));
        assert!(!supported(&Material {
            shader: "Illum".into(),
            ..default()
        }));
    }
    #[test]
    fn nodraw_cut_glow_and_other_submaterials_cannot_shift_the_part_mapping() {
        use aa_cry::{
            mesh::{Model, Node, NodeKind},
            scene::Part,
        };
        use bevy::{camera::primitives::Aabb, math::Affine3A};
        use std::sync::Arc;
        let mut assets = Assets::<Mesh>::default();
        let handles: Vec<_> = (0..3).map(|_| assets.add(Rectangle::default())).collect();
        let mesh = MeshData {
            positions: vec![Vec3::X, Vec3::Y, Vec3::Z],
            normals: vec![Vec3::Z; 3],
            colors: vec![[10, 20, 30, 64], [40, 50, 60, 128], [70, 80, 90, 255]],
            indices: vec![2, 0, 1],
            subsets: (0..5)
                .map(|id| Subset {
                    first_index: 0,
                    index_count: 3,
                    first_vertex: 0,
                    vertex_count: 3,
                    material_id: id,
                    radius: 1.0,
                    center: Vec3::ZERO,
                })
                .collect(),
            ..default()
        };
        let mtl = Material {
            sub_materials: vec![
                Material {
                    shader: "NoDraw".into(),
                    ..default()
                },
                Material {
                    shader: "Illum".into(),
                    ..default()
                },
                Material {
                    shader: "Vegetation".into(),
                    ..default()
                },
                Material {
                    shader: "Cut".into(),
                    ..default()
                },
                Material {
                    shader: "Vegetation".into(),
                    gen_mask: 0x4000_0000,
                    ..default()
                },
            ],
            ..default()
        };
        let model = Model {
            nodes: vec![Node {
                name: "mirror".into(),
                chunk_id: 1,
                parent: None,
                local: Affine3A::from_scale(Vec3::new(-1.0, 1.0, 1.0)),
                kind: NodeKind::Mesh,
                mesh: Some(0),
                material_name: None,
                properties: String::new(),
            }],
            meshes: vec![mesh],
            ..default()
        };
        let mut loaded = LoadedModel {
            model: Arc::new(model),
            skeleton: None,
            inverse_bindposes: None,
            material_path: None,
            parts: vec![(
                0,
                handles
                    .iter()
                    .map(|h| Part {
                        mesh: h.clone(),
                        material: Handle::default(),
                        aabb: Aabb::default(),
                        skinned: false,
                    })
                    .collect(),
            )],
        };
        let prepared = prepare_for_material(&loaded, &mtl);
        assert_eq!(prepared.len(), 1);
        assert_eq!(prepared[0].source, handles[1].id());
        let bevy::mesh::VertexAttributeValues::Float32x2(uv) =
            prepared[0].mesh.attribute(Mesh::ATTRIBUTE_UV_1).unwrap()
        else {
            panic!()
        };
        assert_eq!(
            uv,
            &vec![[1.0, 0.0], [128.0 / 255.0, 0.0], [64.0 / 255.0, 0.0]]
        );
        let mut mirrors = Mirrors::default();
        let first = finish(prepared, &mut assets, &mut mirrors);
        let again = finish(
            prepare_for_material(&loaded, &mtl),
            &mut assets,
            &mut mirrors,
        );
        assert_eq!(
            first[&handles[1].id()].mesh,
            again[&handles[1].id()].mesh,
            "cached subset mirrors are shared"
        );
        loaded.parts[0].1.pop();
        assert!(
            prepare_for_material(&loaded, &mtl).is_empty(),
            "unknown loader filtering must decline a node"
        );
    }
    #[test]
    fn original_spawn_alpha_is_ao_not_rgb_tint_or_record_brightness() {
        if !crate::pak_available() {
            return;
        }
        for (name, expected) in [
            (
                "bushes/cliffbush/cliff_bush_green_mini",
                (1534, 1534, 153, 1),
            ),
            (
                "ground_plants/small_leaves/small_leaves_bunch_big_c",
                (859, 653, 116, 1),
            ),
        ] {
            let path = format!("objects/env/05_nature/08_cry_source/{name}.cgf");
            let model = aa_cry::Model::load(&path).unwrap();
            let path = aa_cry::scene::find_mtl(&path, &model).unwrap();
            let mtl = Material::load(&path).unwrap();
            let mut verts = 0;
            let mut shaded = 0;
            let mut minimum = 255;
            let mut qualified = 0;
            for n in model.lod_nodes(0) {
                let Some(mi) = model.nodes[n].mesh else {
                    continue;
                };
                let mesh = &model.meshes[mi];
                for subset in &mesh.subsets {
                    if !supported(mtl.sub(subset.material_id as usize).unwrap_or(&mtl)) {
                        continue;
                    }
                    if let Some((uv, min)) = alpha_uvs(
                        mesh,
                        subset,
                        aa_cry::scene::render_transform(&model, n)
                            .matrix3
                            .determinant()
                            < 0.0,
                    ) {
                        verts += uv.len();
                        shaded += uv.iter().filter(|v| v[0] < 1.0).count();
                        minimum = minimum.min(min);
                        qualified += 1;
                    }
                }
            }
            assert_eq!((verts, shaded, minimum, qualified), expected);
        }
        let b = aa_pak::Pak::global()
            .read("worlds/main_world/cells/015_015/client/vegetation.dat")
            .unwrap();
        let plants = crate::placement::parse_vegetation_dat(&b).unwrap();
        let xml = aa_pak::Pak::global()
            .read_string("worlds/main_world/vegetation.xml")
            .unwrap();
        let doc = roxmltree::Document::parse(&xml).unwrap();
        for (group, count) in [(107, 28), (1197, 11)] {
            let found: Vec<_> = plants.iter().filter(|v| v.group == group).collect();
            assert_eq!(found.len(), count);
            assert_eq!(found[0].bright, 0);
            let id = group.to_string();
            let g = doc
                .descendants()
                .find(|n| n.has_tag_name("group") && n.attribute("id") == Some(id.as_str()))
                .unwrap();
            assert_eq!(g.attribute("fBrightness"), Some("1"));
            assert_eq!(g.attribute("bUseTerrainColor"), Some("0"));
        }
    }
}
