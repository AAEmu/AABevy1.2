//! Exact authored vertex correspondence; no surface fitting or interpolated deltas.
use crate::{Error, MorphSet, Result};
use bevy::prelude::Vec3;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

/// Matches neutral position/UV classes and verifies oriented triangles. Exported
/// tangent seams can split equivalent vertices differently; delta equality is checked separately.
pub(crate) fn vertex_map(target: &aa_cry::Model, source: &aa_cry::Model) -> Result<Vec<Vec<usize>>> {
    if target.meshes.len() != 1 || source.meshes.len() != 1 {
        return Err(Error::Format("facial overlay needs one mesh".into()));
    }
    let (a, b) = (&target.meshes[0], &source.meshes[0]);
    if a.uvs.len() != a.positions.len() || b.uvs.len() != b.positions.len() {
        return Err(Error::Format("facial vertex/UV counts differ".into()));
    }
    let matches =
        |i: usize, j: usize| a.positions[i].distance_squared(b.positions[j]) < 1e-10 && a.uvs[i].distance_squared(b.uvs[j]) < 1e-10;
    let mut candidates: Vec<Vec<usize>> = (0..a.positions.len())
        .map(|i| (0..b.positions.len()).filter(|&j| matches(i, j)).collect())
        .collect();
    if candidates.iter().any(Vec::is_empty) {
        return Err(Error::Format("facial neutral position/UV differs".into()));
    }
    let labels: Vec<u32> = (0..a.positions.len())
        .map(|i| {
            (0..=i)
                .find(|&k| a.positions[i].distance_squared(a.positions[k]) < 1e-10 && a.uvs[i].distance_squared(a.uvs[k]) < 1e-10)
                .unwrap() as u32
        })
        .collect();
    let source_labels: Vec<u32> = (0..b.positions.len())
        .map(|j| {
            (0..a.positions.len())
                .find(|&i| matches(i, j))
                .map(|i| labels[i])
                .ok_or_else(|| Error::Format(format!("no facial position/UV match for vertex {j}")))
        })
        .collect::<Result<_>>()?;
    let triangles = |mesh: &aa_cry::MeshData, mapping: &[u32]| {
        let mut triangles = HashMap::new();
        for subset in &mesh.subsets {
            let range = subset.first_index as usize..(subset.first_index + subset.index_count) as usize;
            for triangle in mesh.indices[range].chunks_exact(3) {
                let v = [
                    mapping[triangle[0] as usize],
                    mapping[triangle[1] as usize],
                    mapping[triangle[2] as usize],
                ];
                let rotations = [v, [v[1], v[2], v[0]], [v[2], v[0], v[1]]];
                *triangles.entry(*rotations.iter().min().unwrap()).or_insert(0usize) += 1;
            }
        }
        triangles
    };
    let (a_triangles, b_triangles) = (triangles(a, &labels), triangles(b, &source_labels));
    equivalent_patches(&a_triangles, &b_triangles)?;
    // Coincident tongue/lip vertices can share position AND UV but have different authored
    // deltas. An unchanged oriented incident-triangle ring identifies which surface owns one.
    // Never use this shortcut at a retriangulated patch; unresolved candidates must agree.
    let rings = |mesh: &aa_cry::MeshData, labels: &[u32], own: &Triangles, other: &Triangles| {
        let mut rings = vec![Vec::new(); labels.len()];
        let mut stable = vec![true; labels.len()];
        for t in mesh.indices.chunks_exact(3) {
            let v = [labels[t[0] as usize], labels[t[1] as usize], labels[t[2] as usize]];
            let key = *[v, [v[1], v[2], v[0]], [v[2], v[0], v[1]]].iter().min().unwrap();
            for j in 0..3 {
                rings[t[j] as usize].push([v[(j + 1) % 3], v[(j + 2) % 3]]);
                stable[t[j] as usize] &= own.get(&key) == other.get(&key);
            }
        }
        for ring in &mut rings {
            ring.sort_unstable();
        }
        (rings, stable)
    };
    let (a_rings, a_stable) = rings(a, &labels, &a_triangles, &b_triangles);
    let (b_rings, b_stable) = rings(b, &source_labels, &b_triangles, &a_triangles);
    for (i, choices) in candidates.iter_mut().enumerate() {
        if choices.len() > 1 && a_stable[i] && choices.iter().all(|&j| b_stable[j]) {
            let exact: Vec<_> = choices.iter().copied().filter(|&j| a_rings[i] == b_rings[j]).collect();
            if !exact.is_empty() {
                *choices = exact;
            }
        }
    }
    Ok(candidates)
}

type Triangles = HashMap<[u32; 3], usize>;

#[derive(PartialEq, Eq, Hash)]
struct Patch {
    boundary: Vec<(u32, u32, i32)>,
    vertices: Vec<u32>,
    triangles: usize,
}

/// The Ferre exports change five local triangle fans/quad diagonals. Exact vertex identity is
/// already proved above; require each differing patch to have the same vertices and oriented
/// boundary. Reversed, non-manifold, closed or missing patches fail instead of fitting a surface.
fn equivalent_patches(a: &Triangles, b: &Triangles) -> Result<()> {
    let difference = |a: &Triangles, b: &Triangles| {
        a.iter()
            .flat_map(|(triangle, count)| std::iter::repeat_n(*triangle, count.saturating_sub(*b.get(triangle).unwrap_or(&0))))
            .collect::<Vec<_>>()
    };
    if patches(&difference(a, b))? != patches(&difference(b, a))? {
        return Err(Error::Format("facial mapped patch boundary/vertices differ".into()));
    }
    Ok(())
}

fn patches(triangles: &[[u32; 3]]) -> Result<HashMap<Patch, usize>> {
    let edge = |a: u32, b: u32| if a < b { (a, b) } else { (b, a) };
    let mut owners: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (i, t) in triangles.iter().enumerate() {
        for j in 0..3 {
            if t[j] == t[(j + 1) % 3] {
                return Err(Error::Format("degenerate facial patch".into()));
            }
            owners.entry(edge(t[j], t[(j + 1) % 3])).or_default().push(i);
        }
    }
    if owners.values().any(|v| v.len() > 2) {
        return Err(Error::Format("non-manifold facial patch".into()));
    }
    let mut seen = vec![false; triangles.len()];
    let mut result = HashMap::new();
    for start in 0..triangles.len() {
        if seen[start] {
            continue;
        }
        let mut todo = vec![start];
        let mut boundary: HashMap<(u32, u32), i32> = HashMap::new();
        let mut vertices = HashSet::new();
        let mut count = 0;
        while let Some(i) = todo.pop() {
            if std::mem::replace(&mut seen[i], true) {
                continue;
            }
            count += 1;
            let t = triangles[i];
            vertices.extend(t);
            for j in 0..3 {
                let (a, b) = (t[j], t[(j + 1) % 3]);
                let e = edge(a, b);
                *boundary.entry(e).or_default() += if a < b { 1 } else { -1 };
                todo.extend(&owners[&e]);
            }
        }
        let mut boundary: Vec<_> = boundary
            .into_iter()
            .filter(|(_, count)| *count != 0)
            .map(|((a, b), count)| (a, b, count))
            .collect();
        if boundary.is_empty() || boundary.iter().any(|(_, _, c)| c.abs() != 1) {
            return Err(Error::Format("ambiguous facial patch boundary".into()));
        }
        let mut vertices: Vec<_> = vertices.into_iter().collect();
        vertices.sort_unstable();
        boundary.sort_unstable();
        *result
            .entry(Patch {
                boundary,
                vertices,
                triangles: count,
            })
            .or_default() += 1;
    }
    Ok(result)
}

pub(crate) fn expression_set(path: &str, neutral: &aa_cry::Model) -> Result<Arc<MorphSet>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<MorphSet>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(set) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(path) {
        return Ok(set.clone());
    }
    // NPC heads can carry their speech/lid morphs directly in face01, unlike the
    // customizable PC heads whose face01 contains only static shape sliders.
    // These indices already address the rendered model: still validate the native
    // internal/external records, but do not require an absent face00 companion.
    // The girl01 export uses its own npc_* names, addressed directly by girl01.fxl.
    if MorphSet::load(path)?.is_some_and(|set| {
        [("m_a", "eye_close_l"), ("npc_a", "npc_eye_close_l")]
            .into_iter().any(|(mouth, lid)| set.find(mouth).is_some() && set.find(lid).is_some())
    }) {
        if neutral.meshes.len() != 1 {
            return Err(Error::Format("facial native head needs one mesh".into()));
        }
        let set = Arc::new(read_expression(path)?);
        return Ok(cache.lock().unwrap_or_else(|e| e.into_inner()).entry(path.into()).or_insert(set).clone());
    }
    let companion = path.replace("_face01.chr", "_face00.chr");
    if companion == path {
        return Err(Error::Missing(format!("expression companion for {path}")));
    }
    let (gender, letter) = if path.contains("/female/") {
        ("female", "f")
    } else {
        ("male", "m")
    };
    let shared = format!("objects/characters/nuian/{gender}/face/nu_{letter}_face00.chr");
    // Prefer the direct authored companion, then the common human expression mesh.
    let direct = [companion, shared].into_iter().find_map(|source| {
        let model = aa_cry::Model::load(&source).ok()?;
        let mapping = vertex_map(neutral, &model).ok()?;
        Some((source, model, mapping))
    });
    let set = if let Some((source, model, mapping)) = direct {
        remap(read_expression(&source)?, model.meshes[0].positions.len(), &mapping)?
    } else if path == "objects/characters/hariharan/female/face/ha_f_face01.chr" {
        // The Hariharan female has the original shared template's race target baked into its
        // neutral positions. Prove both exact maps; no fitting or transformation of deltas.
        let template_path = "objects/characters/nuian/female/face/nu_f_face01.chr";
        let source_path = "objects/characters/nuian/female/face/nu_f_face00.chr";
        let mut template = aa_cry::Model::load(template_path)?;
        let source = aa_cry::Model::load(source_path)?;
        let mapping = vertex_map(&template, &source)?;
        let set = remap(read_expression(source_path)?, source.meshes[0].positions.len(), &mapping)?;
        let static_targets = MorphSet::load(template_path)?.ok_or_else(|| Error::Missing("Hariharan race baseline".into()))?;
        if static_targets.find("hariharan_Morph").is_none() {
            return Err(Error::Missing("hariharan_Morph".into()));
        }
        static_targets.apply(&mut template.meshes[0].positions, &[("hariharan_Morph".into(), 1.0)]);
        let mapping = vertex_map(neutral, &template)?;
        remap(set, template.meshes[0].positions.len(), &mapping)?
    } else {
        return Err(Error::Format(format!("no matching expression topology for {path}")));
    };
    let set = Arc::new(set);
    Ok(cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(path.into())
        .or_insert(set)
        .clone())
}

fn remap(mut set: MorphSet, source_vertices: usize, mapping: &[Vec<usize>]) -> Result<MorphSet> {
    for target in &mut set.targets {
        let mut dense = vec![Vec3::ZERO; source_vertices];
        for &(vertex, delta) in &target.deltas {
            *dense
                .get_mut(vertex as usize)
                .ok_or_else(|| Error::Format("facial morph vertex out of range".into()))? = delta;
        }
        let mut deltas = Vec::new();
        for (vertex, candidates) in mapping.iter().enumerate() {
            let delta = dense[candidates[0]];
            if candidates.iter().any(|&i| dense[i].distance_squared(delta) > 1e-12) {
                return Err(Error::Format(format!(
                    "facial seam deltas disagree for {} vertex {vertex}",
                    target.name
                )));
            }
            if delta != Vec3::ZERO {
                deltas.push((vertex as u32, delta));
            }
        }
        target.deltas = deltas;
    }
    Ok(set)
}

/// Verify the authored external deltas against the file's native external-to-internal map and
/// internal morph records before using them. Those IDs are local to each export, not a stable
/// ID across separately compiled models; the geometry/UV/patch proof above establishes that join.
fn read_expression(path: &str) -> Result<MorphSet> {
    use aa_cry::chunk::{ChunkFile, ty};
    let bytes = aa_cry::read(path)?;
    let file = ChunkFile::parse(&bytes)?;
    let native_map = file
        .first(ty::COMPILED_EXT_TO_INT_MAP)
        .ok_or_else(|| Error::Missing("facial native vertex map".into()))?;
    let vertices = file
        .first(ty::COMPILED_INT_SKIN_VERTICES)
        .ok_or_else(|| Error::Missing("facial native skin vertices".into()))?;
    if native_map.version != 0x800
        || native_map.data.len() % 2 != 0
        || vertices.version != 0x801
        || vertices.data.len() < 32
        || (vertices.data.len() - 32) % 40 != 0
    {
        return Err(Error::Format("unsupported facial native vertex layout".into()));
    }
    let count = (vertices.data.len() - 32) / 40;
    let native_map: Vec<usize> = native_map
        .data
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes(b.try_into().unwrap()) as usize)
        .collect();
    if native_map.iter().any(|&i| i >= count) {
        return Err(Error::Format("facial native internal vertex out of range".into()));
    }
    let chunk = file
        .first(ty::COMPILED_MORPH_TARGETS)
        .ok_or_else(|| Error::Missing("facial morphs".into()))?;
    let mut r = chunk.reader();
    for _ in 0..r.u32()? {
        r.u32()?; // mesh chunk
        let (name_len, num_int, num_ext) = (r.u32()? as usize, r.u32()? as usize, r.u32()? as usize);
        r.skip(name_len)?;
        let mut internal = vec![Vec3::ZERO; count];
        for _ in 0..num_int {
            let i = r.u32()? as usize;
            let delta = Vec3::new(r.f32()?, r.f32()?, r.f32()?);
            *internal
                .get_mut(i)
                .ok_or_else(|| Error::Format("facial internal morph vertex out of range".into()))? = delta;
        }
        for _ in 0..num_ext {
            let i = r.u32()? as usize;
            let delta = Vec3::new(r.f32()?, r.f32()?, r.f32()?);
            let internal_id = *native_map
                .get(i)
                .ok_or_else(|| Error::Format("facial external morph vertex out of range".into()))?;
            if !delta.is_finite() || internal[internal_id] != delta {
                return Err(Error::Format("facial native internal/external morph deltas disagree".into()));
            }
        }
    }
    MorphSet::from_chunks(&file)?.ok_or_else(|| Error::Missing("facial morphs".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triangles(values: &[[u32; 3]]) -> Triangles {
        values.iter().map(|&v| (v, 1)).collect()
    }

    #[test]
    fn native_diagonal_changes_preserve_oriented_boundary_and_vertex_identity() {
        let a = triangles(&[[0, 1, 2], [0, 2, 3]]);
        let b = triangles(&[[0, 1, 3], [1, 2, 3]]);
        equivalent_patches(&a, &b).unwrap();
        assert!(equivalent_patches(&a, &triangles(&[[0, 3, 1], [1, 3, 2]])).is_err());
        assert!(equivalent_patches(&a, &triangles(&[[0, 1, 3]])).is_err());
        assert!(equivalent_patches(&a, &triangles(&[[0, 1, 4], [0, 4, 3]])).is_err());
    }

    #[test]
    fn contradictory_coincident_morph_deltas_are_rejected() {
        use crate::morph::MorphTarget;
        let set = MorphSet {
            targets: vec![MorphTarget {
                name: "mouth".into(),
                mesh_chunk: 1,
                deltas: vec![(0, Vec3::X), (1, Vec3::Y)],
            }],
        };
        assert!(remap(set.clone(), 2, &[vec![0, 1]]).is_err());
        let mapped = remap(set, 2, &[vec![1], vec![0]]).unwrap();
        assert_eq!(mapped.targets[0].deltas, vec![(0, Vec3::Y), (1, Vec3::X)]);
    }

    #[test]
    fn original_ferre_coincident_surfaces_have_distinct_authored_ownership() {
        if !aa_cry::pak_available() {
            return;
        }
        let target = aa_cry::Model::load("objects/characters/ferre/male/face/fe_m_face01.chr").unwrap();
        let source = aa_cry::Model::load("objects/characters/ferre/male/face/fe_m_face00.chr").unwrap();
        let mapping = vertex_map(&target, &source).unwrap();
        // Same neutral position AND UV, different oriented incident rings and native internal
        // IDs (2649 vs 2745). m_f is zero on one, nonzero on the other: never average them.
        assert_eq!(mapping[3397], vec![3397]);
        assert_eq!(mapping[3417], vec![3417]);
        let set = read_expression("objects/characters/ferre/male/face/fe_m_face00.chr").unwrap();
        let target = set.find("m_f").unwrap();
        assert!(!target.deltas.iter().any(|(i, _)| *i == 3397));
        assert!(target.deltas.iter().any(|(i, d)| *i == 3417 && d.length() > 1e-5));
    }
}
