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
/// Edge length of a cell (m) and the most cells on a side.
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
/// `cells` is the count on a side (see [`cells_for`]).
pub fn drape_quad(centre: Vec3, right: Vec3, up: Vec3, ground: &dyn Fn(Vec3) -> Option<f32>, lift: f32, cells: usize) -> Draped {
    let n = cells.max(1);
    let mut out = Draped { cells: n, ..default() };
    out.positions.reserve((n + 1) * (n + 1));
    let mut last = centre.y;
    for j in 0..=n {
        let t = j as f32 / n as f32;
        for i in 0..=n {
            let s = i as f32 / n as f32;
            let mut p = centre + right * (2.0 * s - 1.0) + up * (1.0 - 2.0 * t);
            if let Some(y) = ground(p) {
                last = y;
            }
            p.y = last + lift;
            out.positions.push(p.to_array());
            out.uvs.push([s, t]);
        }
    }
    let w = (n + 1) as u32;
    for j in 0..n as u32 {
        for i in 0..n as u32 {
            let a = j * w + i;
            out.indices.extend_from_slice(&[a, a + 1, a + w + 1, a, a + w + 1, a + w]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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
