//! Triangle and box tests for the collision queries (game space, f32, no allocation).

use bevy::math::{Vec2, Vec3};

pub type Tri = [Vec3; 3];

/// Unnormalized normal (twice the area).
#[inline]
pub fn cross_normal(t: &Tri) -> Vec3 {
    (t[1] - t[0]).cross(t[2] - t[0])
}

#[inline]
pub fn tri_min(t: &Tri) -> Vec3 {
    t[0].min(t[1]).min(t[2])
}

#[inline]
pub fn tri_max(t: &Tri) -> Vec3 {
    t[0].max(t[1]).max(t[2])
}

/// Ray / triangle, two sided (Moller-Trumbore, a little tolerance on the edges so rays do not slip between
/// neighbouring triangles). `t` of the hit in `0..=max`.
#[inline]
pub fn ray_tri(origin: Vec3, dir: Vec3, tri: &Tri, max: f32) -> Option<f32> {
    const EDGE: f32 = 1e-5;
    let e1 = tri[1] - tri[0];
    let e2 = tri[2] - tri[0];
    let p = dir.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = origin - tri[0];
    let u = s.dot(p) * inv;
    if !(-EDGE..=1.0 + EDGE).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = dir.dot(q) * inv;
    if v < -EDGE || u + v > 1.0 + EDGE {
        return None;
    }
    let t = e2.dot(q) * inv;
    (0.0..=max).contains(&t).then_some(t)
}

/// Height of the triangle's plane over `(x, y)` when the point is inside the triangle seen from above (a hair of
/// tolerance for the shared edges), with the unit normal facing up. Vertical triangles never contain a point.
#[inline]
pub fn plan_height(t: &Tri, x: f32, y: f32) -> Option<(f32, Vec3)> {
    let p = Vec2::new(x, y);
    let (a, b, c) = (t[0].truncate(), t[1].truncate(), t[2].truncate());
    let area = (b - a).perp_dot(c - a);
    let u = (c - b).perp_dot(p - b) / area;
    let v = (a - c).perp_dot(p - c) / area;
    let w = (b - a).perp_dot(p - a) / area;
    const EDGE: f32 = -1e-5;
    if u < EDGE || v < EDGE || w < EDGE {
        return None;
    }
    let n = cross_normal(t);
    let z = t[0].z - (n.x * (x - t[0].x) + n.y * (y - t[0].y)) / n.z;
    let up = if n.z > 0.0 { n } else { -n };
    z.is_finite().then(|| (z, up.normalize()))
}

/// The point of the triangle closest to `p` (Ericson, Real-Time Collision Detection 5.1.5).
pub fn closest_point(p: Vec3, tri: &Tri) -> Vec3 {
    let [a, b, c] = *tri;
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return a + ab * v;
    }
    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return a + ac * w;
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return b + (c - b) * w;
    }
    let denom = 1.0 / (va + vb + vc);
    a + ab * (vb * denom) + ac * (vc * denom)
}

/// Whether the segment `origin + dir * t`, `t` in `0..=max` (`dir` unit length), passes within `margin` of the
/// box `lo..hi` (slab test on the box grown by `margin`).
#[inline]
pub fn segment_near_box(origin: Vec3, dir: Vec3, max: f32, lo: Vec3, hi: Vec3, margin: f32) -> bool {
    let (lo, hi) = (lo - margin, hi + margin);
    let (mut t0, mut t1) = (0.0f32, max);
    for k in 0..3 {
        let (o, d) = (origin[k], dir[k]);
        if d.abs() < 1e-9 {
            if o < lo[k] || o > hi[k] {
                return false;
            }
        } else {
            let inv = 1.0 / d;
            let (mut a, mut b) = ((lo[k] - o) * inv, (hi[k] - o) * inv);
            if a > b {
                std::mem::swap(&mut a, &mut b);
            }
            t0 = t0.max(a);
            t1 = t1.min(b);
            if t0 > t1 {
                return false;
            }
        }
    }
    true
}

/// Whether the triangle, projected on the XY plane, touches the square `min..min + size`. Separating axis
/// test; the box axes are covered by the caller's bounding box test.
pub fn tri_overlaps_square(t: &Tri, min: Vec2, size: f32) -> bool {
    let c = min + Vec2::splat(size * 0.5);
    let h = size * 0.5;
    for i in 0..3 {
        let (p, q) = (t[i].truncate(), t[(i + 1) % 3].truncate());
        let n = Vec2::new(-(q.y - p.y), q.x - p.x);
        if n == Vec2::ZERO {
            continue;
        }
        let proj = [n.dot(t[0].truncate()), n.dot(t[1].truncate()), n.dot(t[2].truncate())];
        let (lo, hi) = (proj[0].min(proj[1]).min(proj[2]), proj[0].max(proj[1]).max(proj[2]));
        let centre = n.dot(c);
        let radius = h * (n.x.abs() + n.y.abs());
        if lo > centre + radius || hi < centre - radius {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: Tri = [Vec3::new(0.0, 0.0, 1.0), Vec3::new(4.0, 0.0, 1.0), Vec3::new(0.0, 4.0, 1.0)];

    #[test]
    fn rays_hit_from_both_sides() {
        let down = ray_tri(Vec3::new(1.0, 1.0, 5.0), Vec3::NEG_Z, &T, 10.0).unwrap();
        assert!((down - 4.0).abs() < 1e-5);
        let up = ray_tri(Vec3::new(1.0, 1.0, -3.0), Vec3::Z, &T, 10.0).unwrap();
        assert!((up - 4.0).abs() < 1e-5);
        assert!(ray_tri(Vec3::new(3.0, 3.0, 5.0), Vec3::NEG_Z, &T, 10.0).is_none());
        assert!(ray_tri(Vec3::new(1.0, 1.0, 5.0), Vec3::NEG_Z, &T, 3.0).is_none());
        assert!(ray_tri(Vec3::new(1.0, 1.0, 5.0), Vec3::X, &T, 10.0).is_none());
        // along the shared edge of two triangles one of them catches the ray
        let other: Tri = [Vec3::new(4.0, 0.0, 1.0), Vec3::new(4.0, 4.0, 1.0), Vec3::new(0.0, 4.0, 1.0)];
        assert!(ray_tri(Vec3::new(2.0, 2.0, 5.0), Vec3::NEG_Z, &T, 10.0).is_some() || ray_tri(Vec3::new(2.0, 2.0, 5.0), Vec3::NEG_Z, &other, 10.0).is_some());
    }

    #[test]
    fn closest_points() {
        assert_eq!(closest_point(Vec3::new(1.0, 1.0, 4.0), &T), Vec3::new(1.0, 1.0, 1.0));
        assert_eq!(closest_point(Vec3::new(-2.0, -2.0, 1.0), &T), Vec3::new(0.0, 0.0, 1.0));
        assert_eq!(closest_point(Vec3::new(2.0, -1.0, 1.0), &T), Vec3::new(2.0, 0.0, 1.0));
        let on_hyp = closest_point(Vec3::new(4.0, 4.0, 1.0), &T);
        assert!((on_hyp - Vec3::new(2.0, 2.0, 1.0)).length() < 1e-5);
    }

    #[test]
    fn boxes_and_squares() {
        let (lo, hi) = (Vec3::new(10.0, -1.0, -1.0), Vec3::new(11.0, 1.0, 1.0));
        assert!(segment_near_box(Vec3::ZERO, Vec3::X, 20.0, lo, hi, 0.0));
        assert!(!segment_near_box(Vec3::ZERO, Vec3::X, 5.0, lo, hi, 0.0));
        assert!(segment_near_box(Vec3::new(0.0, 1.5, 0.0), Vec3::X, 20.0, lo, hi, 0.6));
        assert!(!segment_near_box(Vec3::new(0.0, 1.5, 0.0), Vec3::X, 20.0, lo, hi, 0.4));
        // a long diagonal triangle touches the squares along it and not the far corner one
        let diag: Tri = [Vec3::new(0.0, 0.0, 0.0), Vec3::new(40.0, 40.0, 0.0), Vec3::new(40.0, 39.0, 0.0)];
        assert!(tri_overlaps_square(&diag, Vec2::new(10.0, 10.0), 8.0));
        assert!(!tri_overlaps_square(&diag, Vec2::new(0.0, 24.0), 8.0));
        // a vertical wall (degenerate in plan) is a segment
        let wall: Tri = [Vec3::new(0.0, 0.0, 0.0), Vec3::new(30.0, 0.0, 0.0), Vec3::new(30.0, 0.0, 3.0)];
        assert!(tri_overlaps_square(&wall, Vec2::new(8.0, -4.0), 8.0));
        assert!(!tri_overlaps_square(&wall, Vec2::new(8.0, 8.0), 8.0));
    }
}
