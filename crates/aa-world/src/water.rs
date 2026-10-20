//! Water volumes (rivers, lakes, ponds) of a cell as one flat mesh per cell.
//!
//! The data is the polygon outline of each `Water` volume placed at the volume's surface height; the terrain pokes
//! through where the banks are higher, which gives the shore line. The sea is the separate ocean plane
//! (`WorldInfo::ocean_level`), see `sky.rs`.

use bevy::asset::RenderAssetUsages;
use bevy::math::{Vec2, Vec3};
use bevy::mesh::{Indices, Mesh, PrimitiveTopology};

use crate::placement::Water;

mod authored;
pub mod ocean;

/// Streamed authored water mesh; the ocean is a separate renderer.
#[derive(bevy::prelude::Component)]
pub struct WaterSurface { pub cell: (i32,i32) }

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

/// What makes two volumes look different: the material of the volume and its own fog (colour, density).
/// Volumes of one cell with the same style share one mesh.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct WaterStyle {
    /// Material path (`game/materials/ocean/lilyut_lake01`), empty when the volume names none.
    pub material: String,
    /// `placement::Water::kind` (low 2 bits: 2 = area, 3 = river).
    pub kind: u32,
    /// Fog colour and density as bit patterns (so the style can be a map key).
    fog: [u32; 4],
}

impl WaterStyle {
    pub fn new(material: &str, kind: u32, fog_color: [f32; 3], fog_density: f32) -> WaterStyle {
        let q = |v: f32| if v.is_finite() { v.to_bits() } else { 0 };
        WaterStyle {
            material: material.to_string(),
            kind: kind & 3,
            fog: [q(fog_color[0]), q(fog_color[1]), q(fog_color[2]), q(fog_density)],
        }
    }

    /// The style of a volume; `materials` is the cell's material list.
    pub fn of(w: &Water, materials: &[String]) -> WaterStyle {
        let name = materials.get(w.material as usize).map(String::as_str).unwrap_or("");
        WaterStyle::new(name, w.kind, w.fog_color, w.fog_density)
    }

    pub fn fog_color(&self) -> [f32; 3] {
        [f32::from_bits(self.fog[0]), f32::from_bits(self.fog[1]), f32::from_bits(self.fog[2])]
    }

    pub fn fog_density(&self) -> f32 {
        f32::from_bits(self.fog[3])
    }

    /// Rivers (flowing) rather than ponds and lakes.
    pub fn is_river(&self) -> bool {
        self.kind == 3
    }
}

/// The outline of a volume (cell-local game coordinates) in drawing order, see [`ring_order`].
pub(crate) fn ring(points: &[Vec3]) -> Vec<Vec2> {
    let pts: Vec<Vec2> = points.iter().map(|p| Vec2::new(p.x, p.y)).collect();
    ring_order(&pts).into_iter().map(|i| pts[i]).collect()
}

fn push_volume(w: &Water, positions: &mut Vec<[f32; 3]>, uvs: &mut Vec<[f32; 2]>, colors: &mut Vec<[f32;4]>, indices: &mut Vec<u32>) {
    let base = positions.len() as u32;
    if w.kind & 3 == 3 {
        let Some((points,coordinates))=authored::river(w) else {return};
        for ((point,uv),alpha) in points.iter().zip(coordinates).zip(authored::river_alpha(w.kind)) {
            positions.push([point.x,w.surface,-point.y]);
            uvs.push(uv.to_array());
            colors.push([1.0,1.0,1.0,alpha]);
        }
        // Native centre fan: all four bank segments retain their own UV interpolation.
        indices.extend([0,1,4,1,3,4,3,2,4,0,4,2].map(|i|base+i));
    } else {
        let ring = ring(&w.points);
        let Some(origin)=w.points.first() else {return};
        if !w.uv_scale.iter().all(|v|v.is_finite()) {return}
        for q in &ring {
            positions.push([q.x,w.surface,-q.y]);
            colors.push([1.0;4]);
            // Native CreateArea anchors both independent scales at its first outline vertex.
            uvs.push([(q.x-origin.x)*w.uv_scale[0],(q.y-origin.y)*w.uv_scale[1]]);
        }
        for t in triangulate(&ring) {indices.extend(t.iter().map(|&i|base+i as u32));}
    }
}

fn build_mesh(positions: Vec<[f32; 3]>, uvs: Vec<[f32; 2]>, colors: Vec<[f32;4]>, indices: Vec<u32>) -> Option<Mesh> {
    if indices.is_empty() {
        return None;
    }
    let n = positions.len();
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; n]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
    Some(mesh)
}

/// One mesh with all volumes of a cell, in bevy space relative to the cell origin
/// (`cry (x, y, z) -> bevy (x, z, -y)`).
pub fn water_mesh(waters: &[Water]) -> Option<Mesh> {
    let (mut positions, mut uvs, mut colors, mut indices) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for w in waters {
        push_volume(w, &mut positions, &mut uvs, &mut colors, &mut indices);
    }
    build_mesh(positions, uvs, colors, indices)
}

/// The volumes of a cell grouped by [`WaterStyle`]: one mesh per style (same space as [`water_mesh`]).
pub fn water_meshes(waters: &[Water], materials: &[String]) -> Vec<(WaterStyle, Mesh)> {
    let mut groups: Vec<(WaterStyle, Vec<&Water>)> = Vec::new();
    for w in waters {
        let style = WaterStyle::of(w, materials);
        match groups.iter_mut().find(|(s, _)| *s == style) {
            Some((_, list)) => list.push(w),
            None => groups.push((style, vec![w])),
        }
    }
    groups
        .into_iter()
        .filter_map(|(style, list)| {
            let (mut positions, mut uvs, mut colors, mut indices) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            for w in list {
                push_volume(w, &mut positions, &mut uvs, &mut colors, &mut indices);
            }
            build_mesh(positions, uvs, colors, indices).map(|m| (style, m))
        })
        .collect()
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

    fn volume(x: f32, material: u32, density: f32) -> Water {
        Water {
            bbox: [x, 0.0, 9.0, x + 10.0, 10.0, 10.0],
            surface: 10.0,
            depth: 1.0,
            uv_scale: [0.2, 0.2],
            river_u: [0.0,1.0],
            points: vec![Vec3::new(x, 0.0, 10.0), Vec3::new(x + 10.0, 0.0, 10.0), Vec3::new(x + 10.0, 10.0, 10.0), Vec3::new(x, 10.0, 10.0)],
            kind: 2,
            material,
            fog_density: density,
            fog_color: [0.01, 0.02, 0.03],
        }
    }

    #[test]
    fn meshes_are_grouped_by_style() {
        let names = vec!["game/materials/ocean/a".to_string(), "game/materials/ocean/b".to_string()];
        let waters = [volume(0.0, 0, 0.1), volume(20.0, 1, 0.1), volume(40.0, 0, 0.1), volume(60.0, 0, 0.5)];
        let groups = water_meshes(&waters, &names);
        // a / 0.1 (two volumes), b / 0.1, a / 0.5
        assert_eq!(groups.len(), 3);
        let first = groups.iter().find(|(s, _)| s.material.ends_with("/a") && s.fog_density() == 0.1).unwrap();
        assert_eq!(first.1.count_vertices(), 8);
        assert_eq!(first.0.fog_color(), [0.01, 0.02, 0.03]);
        // a material index outside the list is "no material"
        let g = water_meshes(&[volume(0.0, 9, 0.1)], &names);
        assert_eq!(g[0].0.material, "");
        // the single mesh variant still has everything
        assert_eq!(water_mesh(&waters).unwrap().count_vertices(), 16);
    }

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
