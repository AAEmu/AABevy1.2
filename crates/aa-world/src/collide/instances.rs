//! Vegetation collision: tree trunks and rocks placed as vegetation, as instances of shared models.
//!
//! A forest cell has tens of thousands of instances of a few hundred models; expanding every instance to world
//! space triangles (as [`Soup`](super::soup::Soup) does for brushes) would cost hundreds of MB. Here a model's
//! collision triangles are kept once (model space) and an instance is 24 bytes: position, scale, three angles.
//! A uniform grid over the XY plane lists the instances near each cell (an instance is in every cell its bounding
//! sphere touches); a query takes the instances near it, moves their triangles to world space on the fly (a trunk
//! is 12 to 400 triangles) and runs the same triangle tests as for brushes.

use std::sync::Arc;

use bevy::math::{Mat4, Vec2, Vec3};

use super::geom::{Tri, cross_normal, plan_height, ray_tri, segment_near_box, tri_max, tri_min};
use crate::placement::Vegetation;

/// Most grid cells per side.
const MAX_DIM: usize = 128;

/// The collision triangles of one vegetation model (model space).
pub struct VegModel {
    tris: Vec<Tri>,
    surf: Vec<u16>,
    /// Bounding sphere of the triangles.
    centre: Vec3,
    radius: f32,
    /// Extent of the bounding box on the horizontal axes (m, unscaled).
    width: Vec2,
}

impl VegModel {
    /// `triangles` with the surface id of each; `None` when there are none.
    pub fn new(tris: Vec<Tri>, surf: Vec<u16>) -> Option<VegModel> {
        if tris.is_empty() || tris.len() != surf.len() {
            return None;
        }
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for t in &tris {
            lo = lo.min(tri_min(t));
            hi = hi.max(tri_max(t));
        }
        if !(lo.is_finite() && hi.is_finite()) {
            return None;
        }
        let centre = (lo + hi) * 0.5;
        Some(VegModel { tris, surf, centre, radius: (hi - lo).length() * 0.5 + 1e-3, width: (hi - lo).truncate() })
    }

    #[cfg(test)]
    pub fn triangles(&self) -> usize {
        self.tris.len()
    }

    /// The smaller horizontal extent of the model (m).
    pub fn min_width(&self) -> f32 {
        self.width.min_element()
    }

}

/// One placed instance, in game space.
#[derive(Clone, Copy, Debug)]
pub struct Item {
    pos: Vec3,
    scale: f32,
    model: u32,
    /// `angle_x`, `angle_y`, `angle_z` of the placement record.
    angles: [u8; 3],
    /// Wide enough to stop the camera too.
    big: bool,
}

impl Item {
    /// `v` is a record of the cell at `origin` (game space of the cell's corner); `model` indexes the models given to
    /// [`Instances::build`].
    pub fn new(model: u32, v: &Vegetation, origin: Vec3, big: bool) -> Item {
        Item { pos: origin + v.pos, scale: v.scale, model, angles: [v.angle_x, v.angle_y, v.angle_z], big }
    }

    fn scale(&self) -> f32 {
        self.scale.clamp(0.1, 5.0)
    }

    /// The renderer's transform ([`Vegetation::mat4`]) with the cell origin in the translation.
    fn matrix(&self) -> Mat4 {
        Vegetation { pos: self.pos, scale: self.scale, group: 0, angle_x: self.angles[0], angle_y: self.angles[1], bright: 0, angle_z: self.angles[2], flags: 0, max_view_dist: 0.0 }.mat4()
    }
}

/// Which instances a query looks at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    All,
    /// Only the ones flagged big (they stop the camera).
    Big,
}

/// The vegetation instances of one cell.
#[derive(Default)]
pub struct Instances {
    models: Vec<Arc<VegModel>>,
    items: Vec<Item>,
    min: Vec2,
    cell: f32,
    dim: [usize; 2],
    starts: Vec<u32>,
    ids: Vec<u32>,
}

impl Instances {
    /// Indexes `items` (their `model` indexes `models`) with grid cells `cell` metres wide.
    pub fn build(models: Vec<Arc<VegModel>>, items: Vec<Item>, cell: f32) -> Instances {
        let items: Vec<Item> = items.into_iter().filter(|i| (i.model as usize) < models.len() && i.pos.is_finite()).collect();
        if items.is_empty() {
            return Instances::default();
        }
        let spheres: Vec<(Vec3, f32)> = items
            .iter()
            .map(|i| {
                let m = &models[i.model as usize];
                (i.matrix().transform_point3(m.centre), m.radius * i.scale())
            })
            .collect();
        let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for (c, r) in &spheres {
            lo = lo.min(c.truncate() - *r);
            hi = hi.max(c.truncate() + *r);
        }
        let cell = cell.max((hi - lo).max_element() / (MAX_DIM - 1) as f32);
        let dim = [0, 1].map(|k| (((hi[k] - lo[k]) / cell).floor() as usize + 1).min(MAX_DIM));
        let mut out = Instances { models, items, min: lo, cell, dim, starts: Vec::new(), ids: Vec::new() };
        let cells_of = |c: Vec3, r: f32| out.range(c.truncate() - r, c.truncate() + r);
        let mut counts = vec![0u32; dim[0] * dim[1] + 1];
        for (c, r) in &spheres {
            if let Some([x0, y0, x1, y1]) = cells_of(*c, *r) {
                for iy in y0..=y1 {
                    for ix in x0..=x1 {
                        counts[iy * dim[0] + ix] += 1;
                    }
                }
            }
        }
        let mut sum = 0u32;
        for c in counts.iter_mut() {
            let n = *c;
            *c = sum;
            sum += n;
        }
        let mut fill = counts.clone();
        let mut ids = vec![0u32; sum as usize];
        for (i, (c, r)) in spheres.iter().enumerate() {
            if let Some([x0, y0, x1, y1]) = cells_of(*c, *r) {
                for iy in y0..=y1 {
                    for ix in x0..=x1 {
                        let cell = iy * dim[0] + ix;
                        ids[fill[cell] as usize] = i as u32;
                        fill[cell] += 1;
                    }
                }
            }
        }
        out.starts = counts;
        out.ids = ids;
        out
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Number of instances.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// How many instances are big.
    #[cfg(test)]
    pub fn big(&self) -> usize {
        self.items.iter().filter(|i| i.big).count()
    }

    /// Heap bytes of the instances and their grid (the models are shared).
    pub fn bytes(&self) -> usize {
        self.items.len() * std::mem::size_of::<Item>() + (self.starts.len() + self.ids.len()) * 4
    }

    fn range(&self, lo: Vec2, hi: Vec2) -> Option<[usize; 4]> {
        if self.items.is_empty() {
            return None;
        }
        let f = |v: f32, k: usize| ((v / self.cell).floor() as isize).clamp(-1, self.dim[k] as isize);
        let (a, b) = (lo - self.min, hi - self.min);
        let (x0, y0, x1, y1) = (f(a.x, 0), f(a.y, 1), f(b.x, 0), f(b.y, 1));
        if x1 < 0 || y1 < 0 || x0 >= self.dim[0] as isize || y0 >= self.dim[1] as isize {
            return None;
        }
        Some([x0.max(0) as usize, y0.max(0) as usize, x1.min(self.dim[0] as isize - 1) as usize, y1.min(self.dim[1] as isize - 1) as usize])
    }

    /// Calls `f(model, matrix)` for the instances whose bounding sphere touches the box `lo..hi`.
    fn for_items(&self, lo: Vec3, hi: Vec3, which: Which, mut f: impl FnMut(&VegModel, &Mat4, &Item)) {
        let Some([x0, y0, x1, y1]) = self.range(lo.truncate(), hi.truncate()) else { return };
        let mut ids: Vec<u32> = Vec::new();
        for iy in y0..=y1 {
            for ix in x0..=x1 {
                let c = iy * self.dim[0] + ix;
                ids.extend_from_slice(&self.ids[self.starts[c] as usize..self.starts[c + 1] as usize]);
            }
        }
        if (x1 - x0 + 1) * (y1 - y0 + 1) > 1 {
            ids.sort_unstable();
            ids.dedup();
        }
        for id in ids {
            let item = &self.items[id as usize];
            if which == Which::Big && !item.big {
                continue;
            }
            let model = &self.models[item.model as usize];
            let m = item.matrix();
            let c = m.transform_point3(model.centre);
            let r = model.radius * item.scale();
            let outside = (lo - c).max(c - hi).max(Vec3::ZERO);
            if outside.length_squared() <= r * r {
                f(model, &m, item);
            }
        }
    }

    /// Appends the triangles (world space) whose bounding box overlaps `lo..hi`.
    pub fn tris_in(&self, lo: Vec3, hi: Vec3, which: Which, out: &mut Vec<Tri>) {
        self.for_items(lo, hi, which, |model, m, _| {
            for t in &model.tris {
                let w = world(m, t);
                let (a, b) = (tri_min(&w), tri_max(&w));
                if a.x <= hi.x && b.x >= lo.x && a.y <= hi.y && b.y >= lo.y && a.z <= hi.z && b.z >= lo.z {
                    out.push(w);
                }
            }
        });
    }

    /// Appends the triangles (world space) whose bounding box the segment passes within `margin` of.
    pub fn tris_along(&self, origin: Vec3, dir: Vec3, max: f32, margin: f32, which: Which, out: &mut Vec<Tri>) {
        for_chunks(origin, dir, max, margin, |lo, hi| {
            self.for_items(lo, hi, which, |model, m, _| {
                for t in &model.tris {
                    let w = world(m, t);
                    if segment_near_box(origin, dir, max, tri_min(&w), tri_max(&w), margin) {
                        out.push(w);
                    }
                }
            });
        });
    }

    /// Nearest hit of the ray `origin + dir * t`, `t` in `0..=max` (`dir` unit length): `(t, triangle, surface id)`.
    pub fn ray(&self, origin: Vec3, dir: Vec3, max: f32, which: Which) -> Option<(f32, Tri, u16)> {
        if self.items.is_empty() {
            return None;
        }
        let mut best: Option<(f32, Tri, u16)> = None;
        for_chunks(origin, dir, max, 0.0, |lo, hi| {
            self.for_items(lo, hi, which, |model, m, _| {
                for (i, t) in model.tris.iter().enumerate() {
                    let w = world(m, t);
                    let limit = best.map_or(max, |b| b.0);
                    if let Some(hit) = ray_tri(origin, dir, &w, limit) {
                        best = Some((hit, w, model.surf[i]));
                    }
                }
            });
        });
        best
    }

    /// The highest surface under `(x, y)` that is not above `limit_z` among the triangles whose unit normal has
    /// `|n.z| >= min_nz`: `(height, normal facing up, surface id)`.
    pub fn highest_below(&self, x: f32, y: f32, limit_z: f32, which: Which, min_nz: f32) -> Option<(f32, Vec3, u16)> {
        let mut best: Option<(f32, Vec3, u16)> = None;
        self.for_items(Vec3::new(x, y, -1.0e5), Vec3::new(x, y, limit_z), which, |model, m, _| {
            for (i, t) in model.tris.iter().enumerate() {
                let w = world(m, t);
                let n = cross_normal(&w);
                if n.z.abs() < min_nz * n.length() {
                    continue;
                }
                if let Some((z, up)) = plan_height(&w, x, y)
                    && z <= limit_z
                    && best.is_none_or(|(h, _, _)| z > h)
                {
                    best = Some((z, up, model.surf[i]));
                }
            }
        });
        best
    }

    /// Triangles of all instances in the world space of one cell, for tests.
    #[cfg(test)]
    pub fn all_triangles(&self) -> Vec<Tri> {
        let mut out = Vec::new();
        for item in &self.items {
            let m = item.matrix();
            out.extend(self.models[item.model as usize].tris.iter().map(|t| world(&m, t)));
        }
        out
    }
}

#[inline]
fn world(m: &Mat4, t: &Tri) -> Tri {
    [m.transform_point3(t[0]), m.transform_point3(t[1]), m.transform_point3(t[2])]
}

/// Boxes along a segment, at most [`CHUNK`] metres of it each (a long ray is not one huge box), grown by `margin`.
fn for_chunks(origin: Vec3, dir: Vec3, max: f32, margin: f32, mut f: impl FnMut(Vec3, Vec3)) {
    const CHUNK: f32 = 24.0;
    let mut t = 0.0f32;
    loop {
        let t1 = (t + CHUNK).min(max);
        let (a, b) = (origin + dir * t, origin + dir * t1);
        f(a.min(b) - margin, a.max(b) + margin);
        if t1 >= max {
            break;
        }
        t = t1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trunk: a 1 m wide, 6 m tall box centred on the model origin's XY, standing on it.
    fn trunk() -> Arc<VegModel> {
        let (a, b) = (Vec3::new(-0.5, -0.5, 0.0), Vec3::new(0.5, 0.5, 6.0));
        let corner = |i: u32| Vec3::new(if i & 1 == 0 { a.x } else { b.x }, if i & 2 == 0 { a.y } else { b.y }, if i & 4 == 0 { a.z } else { b.z });
        let quads: [[u32; 4]; 6] = [[1, 3, 7, 5], [0, 4, 6, 2], [2, 6, 7, 3], [0, 1, 5, 4], [4, 5, 7, 6], [0, 2, 3, 1]];
        let mut tris = Vec::new();
        for q in quads {
            tris.push([corner(q[0]), corner(q[1]), corner(q[2])]);
            tris.push([corner(q[0]), corner(q[2]), corner(q[3])]);
        }
        let surf = (0..tris.len() as u16).map(|i| i % 2).collect();
        Arc::new(VegModel::new(tris, surf).unwrap())
    }

    fn veg(x: f32, y: f32, z: f32, scale: f32) -> Item {
        let v = Vegetation { pos: Vec3::new(x, y, z), scale, group: 1, angle_x: 0, angle_y: 0, bright: 0, angle_z: 0, flags: 0, max_view_dist: 0.0 };
        Item::new(0, &v, Vec3::new(1024.0, 0.0, 0.0), scale > 1.5)
    }

    #[test]
    fn instances_are_placed_in_game_space() {
        // two trunks in the cell at x = 1024: at 10, 10 (scale 1) and 50, 10 (scale 2, big)
        let inst = Instances::build(vec![trunk()], vec![veg(10.0, 10.0, 5.0, 1.0), veg(50.0, 10.0, 5.0, 2.0)], 16.0);
        assert_eq!((inst.len(), inst.big()), (2, 1));
        // the ray along +x at z = 8 meets the first trunk's side at x = 1034 - 0.5, the second at 1074 - 1
        let hit = inst.ray(Vec3::new(1000.0, 10.0, 8.0), Vec3::X, 200.0, Which::All).unwrap();
        assert!((hit.0 - 33.5).abs() < 1e-3, "{}", hit.0);
        // only the big one when asked for the big ones
        let hit = inst.ray(Vec3::new(1000.0, 10.0, 8.0), Vec3::X, 200.0, Which::Big).unwrap();
        assert!((hit.0 - 73.0).abs() < 1e-3, "{}", hit.0);
        // too short, over the top (the scaled one is 12 m), beside
        assert!(inst.ray(Vec3::new(1000.0, 10.0, 8.0), Vec3::X, 30.0, Which::All).is_none());
        assert!(inst.ray(Vec3::new(1000.0, 10.0, 18.0), Vec3::X, 200.0, Which::Big).is_none());
        assert!(inst.ray(Vec3::new(1000.0, 14.0, 8.0), Vec3::X, 200.0, Which::All).is_none());
        // a ray across the whole cell is chunked and still finds the far trunk
        assert!(inst.ray(Vec3::new(900.0, 10.0, 8.0), Vec3::X, 400.0, Which::Big).is_some());
    }

    #[test]
    fn the_top_of_a_trunk_is_ground() {
        let inst = Instances::build(vec![trunk()], vec![veg(10.0, 10.0, 5.0, 1.0)], 16.0);
        // the trunk stands from z = 5 to 11
        let (z, n, _) = inst.highest_below(1034.2, 10.1, 20.0, Which::All, 0.5).unwrap();
        assert!((z - 11.0).abs() < 1e-3 && n.z > 0.99, "{z} {n}");
        // a limit below its top: the bottom face (two sided, facing up)
        let (z, _, _) = inst.highest_below(1034.2, 10.1, 8.0, Which::All, 0.5).unwrap();
        assert!((z - 5.0).abs() < 1e-3, "{z}");
        assert!(inst.highest_below(1040.0, 10.1, 20.0, Which::All, 0.5).is_none());
        assert!(inst.highest_below(1034.2, 10.1, 20.0, Which::Big, 0.5).is_none());
    }

    #[test]
    fn triangles_are_gathered_by_box_and_segment() {
        let inst = Instances::build(vec![trunk()], vec![veg(10.0, 10.0, 5.0, 1.0), veg(50.0, 10.0, 5.0, 1.0)], 16.0);
        let mut near = Vec::new();
        inst.tris_in(Vec3::new(1033.0, 9.0, 6.0), Vec3::new(1035.0, 11.0, 7.0), Which::All, &mut near);
        // the sides of the first trunk reach the box, the second is far away
        assert!(!near.is_empty() && near.len() <= 12 && near.iter().all(|t| t.iter().all(|v| v.x < 1040.0)));
        let mut along = Vec::new();
        inst.tris_along(Vec3::new(1000.0, 10.0, 8.0), Vec3::X, 80.0, 0.3, Which::All, &mut along);
        assert!(along.iter().any(|t| t[0].x < 1040.0) && along.iter().any(|t| t[0].x > 1060.0));
        assert_eq!(inst.all_triangles().len(), 24);
    }

    #[test]
    fn models_without_triangles_are_none() {
        assert!(VegModel::new(Vec::new(), Vec::new()).is_none());
        assert!(VegModel::new(vec![[Vec3::ZERO, Vec3::X, Vec3::Y]], Vec::new()).is_none());
        assert_eq!(Instances::build(Vec::new(), Vec::new(), 16.0).len(), 0);
    }
}
