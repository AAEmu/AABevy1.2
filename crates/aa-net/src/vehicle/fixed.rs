//! Engine 64-bit fixed positions and the world-metre scale (U13).
//!
//! `FUN_3961de90` converts a `FixedPos` (`i64 x, i64 y, f32 z`, 20 bytes) to
//! world doubles by multiplying the integer axes by `DAT_3a5e1098`, which the
//! live image holds as `5.684341886080802e-14` = `2^-44`.  The move wire codec
//! keeps the high 36 bits (`high_dword >> 3`), so one wire step is
//! `8 * 2^32 * 2^-44 = 2^-9 m` (1.953125 mm).

/// `DAT_3a5e1098` from the 1.2 image.
pub const FIXED_SCALE: f64 = f64::from_bits(0x3d30_0000_0000_0000);
/// `2^-44`, the exact value of `FIXED_SCALE`.
pub const FIXED_SCALE_EXACT: f64 = 1.0 / 17_592_186_044_416.0;
/// One step of the 24-bit wire position (`8` fixed units of the high dword).
pub const WIRE_STEP_M: f64 = 0.001_953_125;

use super::codec::FixedPos;

/// World-space position with the original's double precision.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WorldPos {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// `FUN_3961de90`: `out[i] = fixed[i] * 2^-44`, z widened from `f32`.
pub fn fixed_to_world(p: &FixedPos) -> WorldPos {
    WorldPos {
        x: p.x as f64 * FIXED_SCALE,
        y: p.y as f64 * FIXED_SCALE,
        z: p.z as f64,
    }
}

/// The exact scale measured live (the image stores the rounded `f64` constant).
pub fn fixed_to_world_exact(p: &FixedPos) -> WorldPos {
    WorldPos {
        x: p.x as f64 * FIXED_SCALE_EXACT,
        y: p.y as f64 * FIXED_SCALE_EXACT,
        z: p.z as f64,
    }
}
