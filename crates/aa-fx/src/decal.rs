//! Ground decals: a card that lies on the terrain and follows its height.
//!
//! The engine projects `Facing = Decal` / `Terrain` particles (the bases of the area skills: Freezing Earth, Crippling Mire, the
//! healing circles) and the ground target reticle (`materials/Decals/target/target_correct`) onto whatever is below them. Here the
//! card is cut into cells whose corners sit on [`crate::FxSettings::ground`], which needs no depth buffer and works with any ground
//! query. Emitters with `Facing = Decal` are draped by the simulation itself; [`drape_quad`] is the same cut for a card of your own
//! (a reticle): build the mesh from the returned arrays and move it with the cursor.

use bevy::prelude::*;

/// How far above the ground a decal lies (m): enough not to z-fight.
pub const LIFT: f32 = 0.04;
/// Coarse cell edge length (m) and maximum coarse cells on a side.
/// Adaptive refinement can split each coarse span three ways.
pub const CELL: f32 = 1.5;
pub const MAX_CELLS: usize = 12;

/// A card of `(cells + 1)^2` vertices with two triangles per cell.
#[derive(Clone, Debug, Default)]
pub struct Draped {
    pub positions: Vec<[f32; 3]>,
    /// 0..1 across the card, `v` growing along `-up` (an image's top row is at `+up`).
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    pub cells: usize,
}

/// Cells on a side for a card with the half extents `half_x` and `half_y` (m).
pub fn cells_for(half_x: f32, half_y: f32) -> usize {
    ((half_x.max(half_y) * 2.0 / CELL).ceil() as usize).clamp(1, MAX_CELLS)
}

/// The card centred on `centre` spanned by the half extents `right` and `up` (world vectors), every vertex on `ground` plus
/// `lift`. `ground` returns the height of the terrain below a world position (`None`: keep the height of the previous vertex).
/// `cells` is the starting count on a side (see [`cells_for`]); curved spans may be split three ways.
/// The returned `Draped::cells` is the actual count. Planar ground keeps the starting grid.
pub fn drape_quad(centre: Vec3, right: Vec3, up: Vec3, ground: &dyn Fn(Vec3) -> Option<f32>, lift: f32, cells: usize) -> Draped {
    let mut out = Draped::default();
    drape_into(&mut out, centre, right, up, ground, lift, cells);
    out
}

/// Reuses an emitter's scratch grid, sampling each shared vertex once. Refinement
/// splits whole rows/columns, so adjacent cells have matching edges and UVs.
#[allow(clippy::too_many_arguments)]
pub(crate) fn drape_into(out: &mut Draped, centre: Vec3, right: Vec3, up: Vec3, ground: &dyn Fn(Vec3) -> Option<f32>, lift: f32, cells: usize) {
    let base = cells.max(1);
    let sample = |out: &mut Draped, xs: &[f32], ys: &[f32]| {
        out.positions.clear();
        out.uvs.clear();
        let mut last = centre.y;
        for &t in ys {
            for &s in xs {
                let mut p = centre + right * (2.0 * s - 1.0) + up * (1.0 - 2.0 * t);
                if let Some(y) = ground(p) {
                    last = y;
                }
                p.y = last + lift;
                out.positions.push(p.to_array());
                out.uvs.push([s, t]);
            }
        }
    };
    // Callers can request a larger explicit grid; adapt only the bounded coarse
    // grids returned by cells_for, up to 36 x 36 cells.
    let mut xs = [0.0; MAX_CELLS * 3 + 1];
    let mut ys = xs;
    let mut n = base;
    if base <= MAX_CELLS {
        for i in 0..=base {
            xs[i] = i as f32 / base as f32;
            ys[i] = xs[i];
        }
        sample(out, &xs[..=base], &ys[..=base]);
        let w = base + 1;
        let at = |i: usize, j: usize| Vec3::from(out.positions[j * w + i]);
        let (a, b, c) = (at(0, 0), at(base, 0), at(0, base));
        let plane_y = |s: f32, t: f32| a.y + (b.y - a.y) * s + (c.y - a.y) * t;
        let planar = (0..=base).all(|j| (0..=base).all(|i| (at(i, j).y - plane_y(xs[i], ys[j])).abs() < 1e-4))
            && [[0.5, 0.5], [0.5, 0.0], [0.5, 1.0], [0.0, 0.5], [1.0, 0.5]].into_iter().all(|[s, t]| {
                let p = centre + right * (2.0 * s - 1.0) + up * (1.0 - 2.0 * t);
                ground(p).is_none_or(|y| (y + lift - plane_y(s, t)).abs() < 1e-4)
            });
        if !planar {
            let mut split_x = [1usize; MAX_CELLS];
            let mut split_y = split_x;
            let intersects = |p: Vec3| ground(p).is_some_and(|y| p.y - y < lift * 0.5);
            for j in 0..=base {
                for (i, split) in split_x[..base].iter_mut().enumerate() {
                    if intersects((at(i, j) + at(i + 1, j)) * 0.5) { *split = 3; }
                }
            }
            for (j, split) in split_y[..base].iter_mut().enumerate() {
                for i in 0..=base {
                    if intersects((at(i, j) + at(i, j + 1)) * 0.5) { *split = 3; }
                }
            }
            for (j, row) in split_y[..base].iter_mut().enumerate() {
                for (i, col) in split_x[..base].iter_mut().enumerate() {
                    // Centre of the shared triangle diagonal: its interpolated
                    // height must clear the ground too (including saddle shapes).
                    if *row == 1 && *col == 1 && intersects((at(i, j) + at(i + 1, j + 1)) * 0.5) {
                        *row = 3;
                        *col = 3;
                    }
                }
            }
            n = split_x[..base].iter().sum::<usize>().max(split_y[..base].iter().sum());
            let knots = |splits: &mut [usize], out: &mut [f32]| {
                // Keep a square grid and the existing (cells+1)^2 public layout.
                let mut extra = n - splits.iter().sum::<usize>();
                for split in &mut *splits {
                    let add = extra.min(3 - *split);
                    *split += add;
                    extra -= add;
                }
                let mut k = 0;
                out[0] = 0.0;
                for (i, &split) in splits.iter().enumerate() {
                    for j in 1..=split {
                        k += 1;
                        out[k] = (i as f32 + j as f32 / split as f32) / base as f32;
                    }
                }
            };
            if n > base {
                knots(&mut split_x[..base], &mut xs);
                knots(&mut split_y[..base], &mut ys);
                sample(out, &xs[..=n], &ys[..=n]);
            }
        }
    } else {
        let knots: Vec<f32> = (0..=base).map(|i| i as f32 / base as f32).collect();
        sample(out, &knots, &knots);
    }
    if out.cells != n || out.indices.is_empty() {
        out.indices.clear();
        let w = (n + 1) as u32;
        for j in 0..n as u32 {
            for i in 0..n as u32 {
                let a = j * w + i;
                out.indices.extend_from_slice(&[a, a + 1, a + w + 1, a, a + w + 1, a + w]);
            }
        }
    }
    out.cells = n;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_large_card_stays_above_curved_ground_between_its_vertices() {
        // Same surface as `effects --bumps 0.5`: Freezing Earth's 26 m card used
        // to cut into the floor between the 12 x 12 coarse cells.
        let ground = |p: Vec3| Some(0.25 * ((p.x * 0.7).sin() + (p.z * 0.9).cos() - 1.0));
        for centre in [Vec3::ZERO, Vec3::new(6.0, 0.0, -6.0), Vec3::new(-12.0, 0.0, 9.0)] {
            for yaw in [0.0, 0.7] {
                let rotation = Quat::from_rotation_y(yaw);
                let d = drape_quad(centre, rotation * Vec3::X * 13.0, rotation * Vec3::Z * 13.0, &ground, LIFT, cells_for(13.0, 13.0));
                assert!(d.cells > MAX_CELLS && d.cells <= MAX_CELLS * 3);
                for triangle in d.indices.as_chunks::<3>().0 {
                    let [a, b, c] = triangle.map(|i| Vec3::from(d.positions[i as usize]));
                    for i in 0..=4 {
                        for j in 0..=4 - i {
                            let p = a + (b - a) * (i as f32 * 0.25) + (c - a) * (j as f32 * 0.25);
                            assert!(p.y >= ground(p).unwrap(), "triangle cuts through ground: {p:?}, centre {centre:?}, yaw {yaw}");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn flat_ground_keeps_the_coarse_budget_and_uvs() {
        let calls = std::cell::Cell::new(0);
        let flat = |_: Vec3| {
            calls.set(calls.get() + 1);
            Some(2.0)
        };
        let d = drape_quad(Vec3::ZERO, Vec3::X * 13.0, Vec3::Z * 13.0, &flat, LIFT, cells_for(13.0, 13.0));
        assert_eq!(d.cells, 12);
        assert_eq!(d.positions.len(), 169);
        assert_eq!(calls.get(), 169 + 5);
        for (p, uv) in d.positions.iter().zip(&d.uvs) {
            assert!((p[0] - (uv[0] * 2.0 - 1.0) * 13.0).abs() < 1e-5);
            assert!((p[2] - (1.0 - uv[1] * 2.0) * 13.0).abs() < 1e-5);
        }
    }

    #[test]
    fn refined_spans_keep_their_uvs_and_reused_grids_can_shrink() {
        let ground = |p: Vec3| Some(0.25 * ((p.x * 0.7).sin() + (p.z * 0.9).cos() - 1.0));
        let mut d = drape_quad(Vec3::ZERO, Vec3::X * 13.0, Vec3::Z * 13.0, &ground, LIFT, 12);
        assert!(d.cells > 12);
        for (p, uv) in d.positions.iter().zip(&d.uvs) {
            assert!((p[0] - (uv[0] * 2.0 - 1.0) * 13.0).abs() < 1e-5);
            assert!((p[2] - (1.0 - uv[1] * 2.0) * 13.0).abs() < 1e-5);
        }
        let capacity = d.positions.capacity();
        drape_into(&mut d, Vec3::ZERO, Vec3::X * 13.0, Vec3::Z * 13.0, &|_| Some(0.0), LIFT, 12);
        assert_eq!(d.positions.len(), 169);
        assert_eq!(d.indices.len(), 12 * 12 * 6);
        assert_eq!(d.positions.capacity(), capacity);
    }

    #[test]
    fn a_draped_card_lies_on_the_slope() {
        let slope = |p: Vec3| Some(0.5 * p.x);
        let d = drape_quad(Vec3::new(10.0, 3.0, 0.0), Vec3::X * 3.0, Vec3::Z * 3.0, &slope, LIFT, cells_for(3.0, 3.0));
        assert_eq!(d.cells, 4);
        assert_eq!(d.positions.len(), 25);
        assert_eq!(d.indices.len(), 4 * 4 * 6);
        for p in &d.positions {
            assert!((p[1] - (0.5 * p[0] + LIFT)).abs() < 1e-4, "{p:?}");
        }
        // the corners are at the ends of the half extents
        assert!((d.positions[0][0] - 7.0).abs() < 1e-4 && (d.positions[24][0] - 13.0).abs() < 1e-4);
        assert_eq!(d.uvs[0], [0.0, 0.0]);
        assert_eq!(d.uvs[24], [1.0, 1.0]);
    }
}
