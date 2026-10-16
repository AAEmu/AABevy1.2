//! Coordinate systems.
//!
//! The game world is CryEngine's: right handed, **Z up**, metres, X east, Y north. A world position is
//! `cell * 1024 + cell-local`, cells being 1024 m squares named `XXX_YYY` after their grid
//! position. Bevy is Y up. Everything this crate puts into the Bevy world uses
//!
//! ```text
//! bevy = (x, z, -y)        (cry east -> +X, cry north -> -Z (forward), cry up -> +Y)
//! ```
//!
//! which is the rotation by -90 degrees about X ([`CRY_TO_BEVY`]); it keeps handedness, so no mesh
//! winding changes are needed.

use bevy::math::{Mat4, Quat, Vec3};

/// Width of a cell in metres.
pub const CELL_SIZE: f32 = 1024.0;
/// Width of a terrain / brush sector in metres (a cell is 16 x 16 sectors).
pub const SECTOR_SIZE: f32 = 64.0;
/// Heightmap sample spacing in metres.
pub const UNIT_SIZE: f32 = 2.0;
/// Heightmap samples per cell side (the shared edge sample makes 513 vertices).
pub const CELL_UNITS: usize = 512;

/// Rotation CryEngine (Z up) -> Bevy (Y up): `(x, y, z)` -> `(x, z, -y)`.
pub const CRY_TO_BEVY: Quat =
    Quat::from_xyzw(-std::f32::consts::FRAC_1_SQRT_2, 0.0, 0.0, std::f32::consts::FRAC_1_SQRT_2);

/// A cry-space point or direction as a Bevy-space one.
#[inline]
pub fn cry_to_bevy(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.z, -v.y)
}

/// A Bevy-space point or direction as a cry-space one.
#[inline]
pub fn bevy_to_cry(v: Vec3) -> Vec3 {
    Vec3::new(v.x, -v.z, v.y)
}

/// A cry-space rotation as a Bevy-space rotation of the same physical turn.
#[inline]
pub fn cry_rot_to_bevy(q: Quat) -> Quat {
    CRY_TO_BEVY * q * CRY_TO_BEVY.inverse()
}

/// A cry-space transform matrix (object -> world, both in cry space) re-expressed so it maps Bevy-space
/// object coordinates to Bevy-space world coordinates: `R * M * R^-1`.
pub fn cry_matrix_to_bevy(m: Mat4) -> Mat4 {
    let r = Mat4::from_quat(CRY_TO_BEVY);
    r * m * r.inverse()
}

/// Cry-space origin (cell corner, z = 0) of cell `(x, y)`.
#[inline]
pub fn cell_origin(x: i32, y: i32) -> Vec3 {
    Vec3::new(x as f32 * CELL_SIZE, y as f32 * CELL_SIZE, 0.0)
}

/// Cell containing a cry-space position, with the cell-local position.
pub fn cell_of(pos: Vec3) -> ((i32, i32), Vec3) {
    let cx = (pos.x / CELL_SIZE).floor() as i32;
    let cy = (pos.y / CELL_SIZE).floor() as i32;
    ((cx, cy), Vec3::new(pos.x - cx as f32 * CELL_SIZE, pos.y - cy as f32 * CELL_SIZE, pos.z))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_and_rotation_agree() {
        let v = Vec3::new(1.0, 2.0, 3.0);
        let b = cry_to_bevy(v);
        assert!((CRY_TO_BEVY * v - b).length() < 1e-5);
        assert_eq!(bevy_to_cry(b), v);
        // a turn about cry Z (up) is a turn about bevy Y (up)
        let q = Quat::from_rotation_z(0.7);
        let qb = cry_rot_to_bevy(q);
        let (axis, angle) = qb.to_axis_angle();
        assert!((axis - Vec3::Y).length() < 1e-5 && (angle - 0.7).abs() < 1e-5);
        // a rotated, translated object lands at the same place either way
        let m = Mat4::from_rotation_translation(q, Vec3::new(10.0, 20.0, 30.0));
        let p = Vec3::new(1.0, 0.5, 0.25);
        let cry = m.transform_point3(p);
        let bevy = cry_matrix_to_bevy(m).transform_point3(cry_to_bevy(p));
        assert!((cry_to_bevy(cry) - bevy).length() < 1e-4);
    }

    #[test]
    fn cells() {
        let ((cx, cy), local) = cell_of(Vec3::new(9300.5, 14400.0, 12.0));
        assert_eq!((cx, cy), (9, 14));
        assert!((local.x - 84.5).abs() < 1e-3 && (local.y - 64.0).abs() < 1e-3);
    }
}
