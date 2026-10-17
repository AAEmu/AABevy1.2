//! A triangle soup indexed by a uniform grid over the XY plane (game space, Z up).
//!
//! Each grid cell lists the triangles that touch it (compressed rows: `starts` + `ids`). Rays walk the cells
//! they cross; vertical queries and box queries read the cells under them. Immutable once built, so queries
//! need no locks.

use bevy::math::{Vec2, Vec3};

use super::geom::{Tri, cross_normal, ray_tri, segment_near_box, tri_max, tri_min, tri_overlaps_square};

/// Most grid cells per side (a 1024 m cell at 8 m is 128; stray far away triangles must not blow this up).
const MAX_DIM: usize = 256;

#[derive(Default)]
pub struct Soup {
    min: Vec2,
    cell: f32,
    dim: [usize; 2],
    tris: Vec<Tri>,
    starts: Vec<u32>,
    ids: Vec<u32>,
    /// Per grid cell: how many of its first ids are triangles that can be ground (`|n.z| >= min_nz`).
    flat: Vec<u32>,
    lo: Vec3,
    hi: Vec3,
}

impl Soup {
    /// Indexes `tris` with grid cells `cell` metres wide. Degenerate triangles are dropped. Triangles whose unit
    /// normal has `|n.z| >= min_nz` are the ones [`Soup::highest_below`] looks at.
    pub fn build(tris: Vec<Tri>, cell: f32, min_nz: f32) -> Soup {
        let tris: Vec<Tri> = tris.into_iter().filter(|t| cross_normal(t).length_squared() > 1e-10 && t.iter().all(|v| v.is_finite())).collect();
        if tris.is_empty() {
            return Soup::default();
        }
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for t in &tris {
            lo = lo.min(tri_min(t));
            hi = hi.max(tri_max(t));
        }
        // very large soups get coarser cells instead of clamped ones
        let cell = cell.max((hi.x - lo.x).max(hi.y - lo.y) / (MAX_DIM - 1) as f32);
        let dim = [0, 1].map(|k| (((hi[k] - lo[k]) / cell).floor() as usize + 1).min(MAX_DIM));
        let mut soup = Soup { min: lo.truncate(), cell, dim, tris, starts: Vec::new(), ids: Vec::new(), flat: Vec::new(), lo, hi };
        let mut counts = vec![0u32; dim[0] * dim[1] + 1];
        for t in &soup.tris {
            soup.for_cells(t, |c| counts[c] += 1);
        }
        let mut sum = 0u32;
        for c in counts.iter_mut() {
            let n = *c;
            *c = sum;
            sum += n;
        }
        let mut fill = counts.clone();
        let mut ids = vec![0u32; sum as usize];
        for (i, t) in soup.tris.iter().enumerate() {
            soup.for_cells(t, |c| {
                ids[fill[c] as usize] = i as u32;
                fill[c] += 1;
            });
        }
        // ground candidates first in every row
        let is_flat = |t: &Tri| {
            let n = cross_normal(t);
            n.z.abs() >= min_nz * n.length()
        };
        let mut flat = vec![0u32; dim[0] * dim[1]];
        for c in 0..dim[0] * dim[1] {
            let row = &mut ids[counts[c] as usize..counts[c + 1] as usize];
            let mut front = 0;
            for i in 0..row.len() {
                if is_flat(&soup.tris[row[i] as usize]) {
                    row.swap(front, i);
                    front += 1;
                }
            }
            flat[c] = front as u32;
        }
        soup.starts = counts;
        soup.ids = ids;
        soup.flat = flat;
        soup
    }

    pub fn is_empty(&self) -> bool {
        self.tris.is_empty()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.tris.len()
    }

    /// Heap bytes of the soup.
    pub fn bytes(&self) -> usize {
        self.tris.len() * std::mem::size_of::<Tri>() + (self.starts.len() + self.ids.len() + self.flat.len()) * 4
    }

    pub fn tri(&self, id: u32) -> &Tri {
        &self.tris[id as usize]
    }

    #[cfg(test)]
    pub fn triangles(&self) -> &[Tri] {
        &self.tris
    }

    /// Grid index range `(ix0, iy0, ix1, iy1)` of the cells that overlap an XY box, `None` when it misses the grid.
    fn range(&self, lo: Vec2, hi: Vec2) -> Option<[usize; 4]> {
        if self.is_empty() {
            return None;
        }
        let f = |v: f32, k: usize| ((v / self.cell).floor() as isize).clamp(-1, self.dim[k] as isize);
        let (a, b) = ((lo - self.min), (hi - self.min));
        let (x0, y0, x1, y1) = (f(a.x, 0), f(a.y, 1), f(b.x, 0), f(b.y, 1));
        if x1 < 0 || y1 < 0 || x0 >= self.dim[0] as isize || y0 >= self.dim[1] as isize {
            return None;
        }
        Some([x0.max(0) as usize, y0.max(0) as usize, x1.min(self.dim[0] as isize - 1) as usize, y1.min(self.dim[1] as isize - 1) as usize])
    }

    /// Calls `f(cell index)` for the cells a triangle touches.
    fn for_cells(&self, t: &Tri, mut f: impl FnMut(usize)) {
        let Some([x0, y0, x1, y1]) = self.range(tri_min(t).truncate(), tri_max(t).truncate()) else { return };
        let exact = (x1 - x0 + 1) * (y1 - y0 + 1) > 4;
        for iy in y0..=y1 {
            for ix in x0..=x1 {
                if exact {
                    let origin = self.min + Vec2::new(ix as f32, iy as f32) * self.cell;
                    if !tri_overlaps_square(t, origin, self.cell) {
                        continue;
                    }
                }
                f(iy * self.dim[0] + ix);
            }
        }
    }

    #[inline]
    fn row(&self, cell: usize) -> &[u32] {
        &self.ids[self.starts[cell] as usize..self.starts[cell + 1] as usize]
    }

    /// Nearest hit of the ray `origin + dir * t`, `t` in `0..=max` (`dir` unit length): `(t, triangle id)`.
    pub fn ray(&self, origin: Vec3, dir: Vec3, max: f32) -> Option<(f32, u32)> {
        if self.is_empty() {
            return None;
        }
        // clip to the bounds of the soup
        let (mut t0, mut t1) = (0.0f32, max);
        for k in 0..3 {
            let (lo, hi) = (self.lo[k] - 1e-3, self.hi[k] + 1e-3);
            if dir[k].abs() < 1e-9 {
                if origin[k] < lo || origin[k] > hi {
                    return None;
                }
            } else {
                let inv = 1.0 / dir[k];
                let (mut a, mut b) = ((lo - origin[k]) * inv, (hi - origin[k]) * inv);
                if a > b {
                    std::mem::swap(&mut a, &mut b);
                }
                t0 = t0.max(a);
                t1 = t1.min(b);
                if t0 > t1 {
                    return None;
                }
            }
        }
        let p = origin + dir * t0;
        let mut ix = (((p.x - self.min.x) / self.cell).floor() as isize).clamp(0, self.dim[0] as isize - 1);
        let mut iy = (((p.y - self.min.y) / self.cell).floor() as isize).clamp(0, self.dim[1] as isize - 1);
        let axis = |k: usize, i: isize| -> (isize, f32, f32) {
            if dir[k].abs() < 1e-9 {
                return (0, f32::INFINITY, f32::INFINITY);
            }
            let step = if dir[k] > 0.0 { 1 } else { -1 };
            let edge = self.min[k] + (i + (step > 0) as isize) as f32 * self.cell;
            (step, (edge - origin[k]) / dir[k], self.cell / dir[k].abs())
        };
        let ((sx, mut tx, dx), (sy, mut ty, dy)) = (axis(0, ix), axis(1, iy));
        let mut best: Option<(f32, u32)> = None;
        let mut limit = t1;
        loop {
            for &id in self.row(iy as usize * self.dim[0] + ix as usize) {
                if let Some(t) = ray_tri(origin, dir, &self.tris[id as usize], limit) {
                    limit = t;
                    best = Some((t, id));
                }
            }
            let exit = tx.min(ty);
            if exit >= limit || exit.is_infinite() {
                break;
            }
            if tx < ty {
                ix += sx;
                tx += dx;
            } else {
                iy += sy;
                ty += dy;
            }
            if ix < 0 || iy < 0 || ix >= self.dim[0] as isize || iy >= self.dim[1] as isize {
                break;
            }
        }
        best
    }

    /// The highest surface under `(x, y)` that is not above `limit_z` among the ground candidates (`|n.z| >= min_nz`
    /// of [`Soup::build`]; two sided, the normal returned faces up): `(height, normal)`.
    pub fn highest_below(&self, x: f32, y: f32, limit_z: f32) -> Option<(f32, Vec3)> {
        let [ix, iy, _, _] = self.range(Vec2::new(x, y), Vec2::new(x, y))?;
        let cell = iy * self.dim[0] + ix;
        let row = &self.ids[self.starts[cell] as usize..][..self.flat[cell] as usize];
        let mut best: Option<(f32, Vec3)> = None;
        let p = Vec2::new(x, y);
        for &id in row {
            let t = &self.tris[id as usize];
            // barycentric test in plan, with a hair of tolerance for the shared edges
            let (a, b, c) = (t[0].truncate(), t[1].truncate(), t[2].truncate());
            let area = (b - a).perp_dot(c - a);
            let u = (c - b).perp_dot(p - b) / area;
            let v = (a - c).perp_dot(p - c) / area;
            let w = (b - a).perp_dot(p - a) / area;
            const EDGE: f32 = -1e-5;
            if u < EDGE || v < EDGE || w < EDGE {
                continue;
            }
            let n = cross_normal(t);
            let z = t[0].z - (n.x * (x - t[0].x) + n.y * (y - t[0].y)) / n.z;
            if z <= limit_z && best.is_none_or(|(h, _)| z > h) {
                let up = if n.z > 0.0 { n } else { -n };
                best = Some((z, up.normalize()));
            }
        }
        best
    }

    /// Appends the ids of the triangles whose bounding box overlaps `lo..hi` and that touch a grid cell under it
    /// (each id once).
    pub fn gather(&self, lo: Vec3, hi: Vec3, out: &mut Vec<u32>) {
        let Some([x0, y0, x1, y1]) = self.range(lo.truncate(), hi.truncate()) else { return };
        let start = out.len();
        for iy in y0..=y1 {
            for ix in x0..=x1 {
                for &id in self.row(iy * self.dim[0] + ix) {
                    let t = &self.tris[id as usize];
                    let (a, b) = (tri_min(t), tri_max(t));
                    if a.x <= hi.x && b.x >= lo.x && a.y <= hi.y && b.y >= lo.y && a.z <= hi.z && b.z >= lo.z {
                        out.push(id);
                    }
                }
            }
        }
        if (x1 - x0 + 1) * (y1 - y0 + 1) > 1 {
            out[start..].sort_unstable();
            let mut keep = start;
            for i in start..out.len() {
                if i == start || out[i] != out[keep - 1] {
                    out[keep] = out[i];
                    keep += 1;
                }
            }
            out.truncate(keep);
        }
    }

    /// Appends the ids of the triangles whose bounding box the segment passes within `margin` of.
    pub fn gather_along(&self, origin: Vec3, dir: Vec3, max: f32, margin: f32, out: &mut Vec<u32>) {
        let end = origin + dir * max;
        let (lo, hi) = (origin.min(end) - margin, origin.max(end) + margin);
        let start = out.len();
        self.gather(lo, hi, out);
        let mut keep = start;
        for i in start..out.len() {
            let t = &self.tris[out[i] as usize];
            if segment_near_box(origin, dir, max, tri_min(t), tri_max(t), margin) {
                out[keep] = out[i];
                keep += 1;
            }
        }
        out.truncate(keep);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 100 x 100 m floor at z = 2 made of 25 m squares (2 triangles each) and a wall at x = 50.
    fn scene() -> Vec<Tri> {
        let mut tris = Vec::new();
        for i in 0..4 {
            for j in 0..4 {
                let (x, y) = (i as f32 * 25.0, j as f32 * 25.0);
                let (a, b, c, d) = (Vec3::new(x, y, 2.0), Vec3::new(x + 25.0, y, 2.0), Vec3::new(x + 25.0, y + 25.0, 2.0), Vec3::new(x, y + 25.0, 2.0));
                tris.push([a, b, c]);
                tris.push([a, c, d]);
            }
        }
        let wall = |y0: f32, y1: f32| {
            let (a, b, c, d) = (Vec3::new(50.0, y0, 2.0), Vec3::new(50.0, y1, 2.0), Vec3::new(50.0, y1, 6.0), Vec3::new(50.0, y0, 6.0));
            [[a, b, c], [a, c, d]]
        };
        tris.extend(wall(0.0, 100.0));
        tris
    }

    #[test]
    fn rays_cross_cells() {
        let s = Soup::build(scene(), 8.0, 0.5);
        assert_eq!(s.len(), 34);
        // down onto the floor
        let (t, _) = s.ray(Vec3::new(10.0, 10.0, 12.0), Vec3::NEG_Z, 100.0).unwrap();
        assert!((t - 10.0).abs() < 1e-4);
        // along the floor's height into the wall, through many cells
        let dir = Vec3::new(1.0, 0.1, 0.0).normalize();
        let (t, id) = s.ray(Vec3::new(1.0, 5.0, 4.0), dir, 100.0).unwrap();
        let hit = Vec3::new(1.0, 5.0, 4.0) + dir * t;
        assert!((hit.x - 50.0).abs() < 1e-3, "{hit}");
        assert!(s.tri(id).iter().all(|v| v.x == 50.0));
        // too short, and parallel above everything
        assert!(s.ray(Vec3::new(1.0, 5.0, 4.0), dir, 40.0).is_none());
        assert!(s.ray(Vec3::new(1.0, 5.0, 40.0), Vec3::X, 100.0).is_none());
        // a ray starting outside the grid walks in
        assert!(s.ray(Vec3::new(-30.0, 40.0, 5.0), Vec3::X, 200.0).is_some());
        // against a brute force check on random rays
        let mut seed = 12345u32;
        let mut rnd = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as f32 / 16777216.0
        };
        for _ in 0..500 {
            let o = Vec3::new(rnd() * 120.0 - 10.0, rnd() * 120.0 - 10.0, rnd() * 10.0 - 1.0);
            let d = Vec3::new(rnd() - 0.5, rnd() - 0.5, rnd() - 0.5).normalize();
            let brute = s.triangles().iter().filter_map(|t| ray_tri(o, d, t, 150.0)).fold(f32::INFINITY, f32::min);
            let fast = s.ray(o, d, 150.0).map_or(f32::INFINITY, |h| h.0);
            assert!((brute - fast).abs() < 1e-3 || (brute.is_infinite() && fast.is_infinite()), "{o} {d}: {brute} / {fast}");
        }
    }

    #[test]
    fn ground_under_a_point() {
        let s = Soup::build(scene(), 8.0, 0.5);
        let (z, n) = s.highest_below(30.0, 30.0, 10.0).unwrap();
        assert!((z - 2.0).abs() < 1e-4 && n.z > 0.99);
        assert!(s.highest_below(30.0, 30.0, 1.0).is_none());
        assert!(s.highest_below(300.0, 30.0, 10.0).is_none());
        // a vertical wall is no ground
        assert!(s.highest_below(50.0, 30.0, 10.0).map(|(z, _)| z) == Some(2.0));
    }

    #[test]
    fn gathering() {
        let s = Soup::build(scene(), 8.0, 0.5);
        let mut ids = Vec::new();
        s.gather(Vec3::new(45.0, 10.0, 1.0), Vec3::new(55.0, 12.0, 7.0), &mut ids);
        // the floor squares on both sides of x = 50 and the wall pieces, each once
        let mut sorted = ids.clone();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len());
        assert!(ids.len() >= 4 && ids.len() <= 8, "{}", ids.len());
        let mut along = Vec::new();
        s.gather_along(Vec3::new(0.0, 5.0, 4.0), Vec3::X, 60.0, 0.3, &mut along);
        // the wall at x = 50, not the floor two metres below
        assert!(!along.is_empty() && along.iter().all(|&id| s.tri(id).iter().all(|v| v.x == 50.0)), "{along:?}");
        along.clear();
        s.gather_along(Vec3::new(0.0, 5.0, 4.0), Vec3::X, 40.0, 0.3, &mut along);
        assert!(along.is_empty());
    }
}
