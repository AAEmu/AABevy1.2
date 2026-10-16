//! `Cut` hulls: the costume models carry a coarse closed hull under a `Cut` sub-material (a
//! handful of triangles around the torso, the legs, the sleeves). The nude body triangles inside
//! such a hull are not drawn, so a costume does not show the body through it (and the costume's
//! own `skin` sub-mesh replaces the exposed skin).
//!
//! The hulls are closed, consistently wound surfaces (the file splits their vertices per face;
//! welded by position they have no open edge), so "inside" is the generalised winding number of the
//! hull around a point.

use aa_cry::MeshData;
use aa_cry::material::Material;
use aa_cry::mesh::Model;
use bevy::math::Vec3;

/// A closed triangle hull in the character's rest space.
#[derive(Clone, Debug)]
pub struct Hull {
    tris: Vec<[Vec3; 3]>,
    min: Vec3,
    max: Vec3,
}

impl Hull {
    fn new(tris: Vec<[Vec3; 3]>) -> Hull {
        let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for p in tris.iter().flatten() {
            min = min.min(*p);
            max = max.max(*p);
        }
        Hull { tris, min, max }
    }

    /// Is `p` inside the hull (winding number of the surface around it is non-zero)?
    pub fn contains(&self, p: Vec3) -> bool {
        if p.cmplt(self.min).any() || p.cmpgt(self.max).any() {
            return false;
        }
        let mut omega = 0.0f32;
        for [a, b, c] in &self.tris {
            omega += solid_angle(*a - p, *b - p, *c - p);
        }
        omega.abs() > 2.0 * std::f32::consts::PI
    }
}

/// Signed solid angle of the triangle `a b c` seen from the origin (Van Oosterom & Strakhov).
fn solid_angle(a: Vec3, b: Vec3, c: Vec3) -> f32 {
    let (la, lb, lc) = (a.length(), b.length(), c.length());
    let denom = la * lb * lc + a.dot(b) * lc + a.dot(c) * lb + b.dot(c) * la;
    2.0 * a.dot(b.cross(c)).atan2(denom)
}

/// The `Cut` hulls of a costume model (`mtl` = its material, for the shader of each sub-material).
pub fn cut_hulls(model: &Model, mtl: &Material) -> Vec<Hull> {
    let mut hulls = Vec::new();
    for mesh in &model.meshes {
        for subset in &mesh.subsets {
            if !mtl.sub(subset.material_id as usize).is_some_and(|s| s.shader.eq_ignore_ascii_case("cut")) {
                continue;
            }
            let (first, count) = (subset.first_index as usize, subset.index_count as usize);
            let Some(indices) = mesh.indices.get(first..first + count) else { continue };
            let tris: Vec<[Vec3; 3]> = indices
                .chunks_exact(3)
                .map(|t| [mesh.positions[t[0] as usize], mesh.positions[t[1] as usize], mesh.positions[t[2] as usize]])
                .collect();
            if !tris.is_empty() {
                hulls.push(Hull::new(tris));
            }
        }
    }
    hulls
}

/// Collapses the triangles of `mesh` whose three vertices lie inside one of the `hulls`
/// (degenerate triangles are not rasterised; subset index ranges stay valid).
pub fn cut_mesh(mesh: &mut MeshData, hulls: &[Hull]) {
    if hulls.is_empty() {
        return;
    }
    let inside: Vec<bool> = mesh.positions.iter().map(|&p| hulls.iter().any(|h| h.contains(p))).collect();
    for t in mesh.indices.chunks_exact_mut(3) {
        if t.iter().all(|&i| inside[i as usize]) {
            t[1] = t[0];
            t[2] = t[0];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit cube from 12 triangles, outward wound.
    fn cube() -> Hull {
        let v = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
        let p = [v(0., 0., 0.), v(1., 0., 0.), v(1., 1., 0.), v(0., 1., 0.), v(0., 0., 1.), v(1., 0., 1.), v(1., 1., 1.), v(0., 1., 1.)];
        let f = [[0, 2, 1], [0, 3, 2], [4, 5, 6], [4, 6, 7], [0, 1, 5], [0, 5, 4], [1, 2, 6], [1, 6, 5], [2, 3, 7], [2, 7, 6], [3, 0, 4], [3, 4, 7]];
        Hull::new(f.iter().map(|t| [p[t[0]], p[t[1]], p[t[2]]]).collect())
    }

    #[test]
    fn winding_number_inside_and_outside() {
        let h = cube();
        assert!(h.contains(Vec3::splat(0.5)));
        assert!(h.contains(Vec3::new(0.1, 0.9, 0.5)));
        assert!(!h.contains(Vec3::new(1.5, 0.5, 0.5)));
        assert!(!h.contains(Vec3::new(0.5, 0.5, -0.2)));
    }

    #[test]
    fn inside_triangles_are_collapsed() {
        let h = cube();
        let mut mesh = MeshData {
            positions: vec![Vec3::splat(0.4), Vec3::new(0.6, 0.4, 0.4), Vec3::new(0.4, 0.6, 0.4), Vec3::new(3.0, 0.0, 0.0)],
            indices: vec![0, 1, 2, 0, 1, 3],
            ..Default::default()
        };
        cut_mesh(&mut mesh, &[h]);
        assert_eq!(mesh.indices, vec![0, 0, 0, 0, 1, 3]);
    }
}
