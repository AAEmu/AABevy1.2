//! Water volumes (rivers, lakes, ponds) of a cell as one flat mesh per cell.
//!
//! The data is the polygon outline of each `Water` volume placed at the volume's surface height; the terrain pokes
//! through where the banks are higher, which gives the shore line. The sea is the separate ocean plane
//! (`WorldInfo::ocean_level`), see `sky.rs`.

use bevy::asset::RenderAssetUsages;
use bevy::math::{Vec2, Vec3};
use bevy::mesh::{Indices, Mesh, PrimitiveTopology};

use crate::placement::Water;

fn cross(a: Vec2, b: Vec2, c: Vec2) -> f32 {
    (b - a).perp_dot(c - a)
}

fn segments_cross(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> bool {
    let (d1, d2) = (cross(a, b, c), cross(a, b, d));
    let (d3, d4) = (cross(c, d, a), cross(c, d, b));
    d1 * d2 < 0.0 && d3 * d4 < 0.0
}

/// Four-point volumes are river quads written as a strip (`0 1 2 3` zig-zags between the banks): pick the order
/// that does not cross itself. Longer outlines are rings already.
fn ring_order(p: &[Vec2]) -> Vec<usize> {
    if p.len() == 4 {
        let bow_tie = segments_cross(p[0], p[1], p[2], p[3]) || segments_cross(p[1], p[2], p[3], p[0]);
        if bow_tie {
            return vec![0, 1, 3, 2];
        }
    }
    (0..p.len()).collect()
}

fn signed_area(p: &[Vec2]) -> f32 {
    (0..p.len()).map(|i| p[i].perp_dot(p[(i + 1) % p.len()])).sum::<f32>() * 0.5
}

/// Ear clipping of a simple polygon (counter-clockwise). Falls back to a fan when no ear is found
/// (self-touching outlines).
pub fn triangulate(p: &[Vec2]) -> Vec<[usize; 3]> {
    let n = p.len();
    if n < 3 {
        return Vec::new();
    }
    let ccw = signed_area(p) >= 0.0;
    let mut idx: Vec<usize> = if ccw { (0..n).collect() } else { (0..n).rev().collect() };
    let mut out = Vec::with_capacity(n - 2);
    let inside = |t: [Vec2; 3], q: Vec2| cross(t[0], t[1], q) >= 0.0 && cross(t[1], t[2], q) >= 0.0 && cross(t[2], t[0], q) >= 0.0;
    while idx.len() > 3 {
        let m = idx.len();
        let mut clipped = false;
        for i in 0..m {
            let (a, b, c) = (idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]);
            let tri = [p[a], p[b], p[c]];
            if cross(tri[0], tri[1], tri[2]) <= 1e-6 {
                continue;
            }
            if idx.iter().any(|&o| o != a && o != b && o != c && inside(tri, p[o])) {
                continue;
            }
            out.push([a, b, c]);
            idx.remove(i);
            clipped = true;
            break;
        }
        if !clipped {
            for i in 1..idx.len() - 1 {
                out.push([idx[0], idx[i], idx[i + 1]]);
            }
            return out;
        }
    }
    out.push([idx[0], idx[1], idx[2]]);
    out
}

/// One mesh with all volumes of a cell, in bevy space relative to the cell origin
/// (`cry (x, y, z) -> bevy (x, z, -y)`).
pub fn water_mesh(waters: &[Water]) -> Option<Mesh> {
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    for w in waters {
        let pts: Vec<Vec2> = w.points.iter().map(|p| Vec2::new(p.x, p.y)).collect();
        let order = ring_order(&pts);
        let ring: Vec<Vec2> = order.iter().map(|&i| pts[i]).collect();
        let base = positions.len() as u32;
        let scale = if w.uv_scale[0].abs() > 1e-3 { w.uv_scale[0].abs() } else { 0.2 };
        for q in &ring {
            positions.push([q.x, w.surface, -q.y]);
            uvs.push([q.x * scale, q.y * scale]);
        }
        for t in triangulate(&ring) {
            indices.extend(t.iter().map(|&i| base + i as u32));
        }
    }
    if indices.is_empty() {
        return None;
    }
    let n = positions.len();
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; n]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    Some(mesh)
}

/// Axis aligned bounds of a polygon list, bevy space relative to the cell (for tests and culling).
pub fn bounds(waters: &[Water]) -> Option<(Vec3, Vec3)> {
    let mut it = waters.iter().flat_map(|w| w.points.iter().map(|p| Vec3::new(p.x, w.surface, -p.y)));
    let first = it.next()?;
    Some(it.fold((first, first), |(lo, hi), p| (lo.min(p), hi.max(p))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bow_tie_quad_is_reordered() {
        // river quad as a strip: left bank 0, right bank 1, left 2, right 3
        let p = [Vec2::new(0.0, 0.0), Vec2::new(4.0, 0.0), Vec2::new(0.0, 10.0), Vec2::new(4.0, 10.0)];
        assert_eq!(ring_order(&p), vec![0, 1, 3, 2]);
        let ring: Vec<Vec2> = ring_order(&p).iter().map(|&i| p[i]).collect();
        let tris = triangulate(&ring);
        assert_eq!(tris.len(), 2);
        let area: f32 = tris.iter().map(|t| cross(ring[t[0]], ring[t[1]], ring[t[2]]).abs() * 0.5).sum();
        assert!((area - 40.0).abs() < 1e-3);
    }

    #[test]
    fn concave_polygon_triangulates_to_its_area() {
        // an L shape, area 3
        let p = [
            Vec2::new(0.0, 0.0),
            Vec2::new(2.0, 0.0),
            Vec2::new(2.0, 1.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(1.0, 2.0),
            Vec2::new(0.0, 2.0),
        ];
        let tris = triangulate(&p);
        assert_eq!(tris.len(), 4);
        let area: f32 = tris.iter().map(|t| cross(p[t[0]], p[t[1]], p[t[2]]).abs() * 0.5).sum();
        assert!((area - 3.0).abs() < 1e-4);
    }
}
