//! Move-struct codecs from the 1.2 `x2game.dll` decompile.
//!
//! * `FUN_3961e2c0` (0x3961e2c0) — 9-byte packed 24-bit position codec.
//! * `FUN_397ad5a0` / `FUN_397ad8c0` / `FUN_397ada60` — 3×i16 velocity codecs,
//!   scales 1/60, 1/30 and 1/50.
//! * `FUN_397ad740` — 3×i8 delta-movement codec, scale 1/127.
//! * `FUN_397adc00` / `FUN_397ae020` — quaternion i16 read/write, with the
//!   `w = sqrt(1 - x² - y² - z²)` reconstruction limit 0.9975.
//! * `FUN_397adca0` / `FUN_3938b460` — 3×i8 axis-angle quaternion codec.
//!
//! The writers reproduce the original's MSVC float→int rounding sequence
//! (`floor(x + 0.5)`): the SSE build uses a ±2^23 magic and the double build a
//! ±2^52 magic with a one-unit correction. Both are implemented literally so
//! boundary values stay bit-exact.

use super::acosf::libm_sse2_acosf;
use super::sinf::libm_sse2_sinf;

pub const I16_SCALE: f32 = f32::from_bits(0x3800_0100); // 1/32767 as in .data
pub const QUAT_W_LIMIT: f32 = f32::from_bits(0x3f7f_5c29);
pub const VEL_TYPE0: f32 = 50.0;
pub const VEL_TYPE1: f32 = 60.0;
pub const VEL_VEHICLE: f32 = 30.0;
pub const VEL_SCALE_TYPE0: f32 = f32::from_bits(0x3ca3_d70a); // 1/50
pub const VEL_SCALE_TYPE1: f32 = f32::from_bits(0x3c88_8889); // 1/60
pub const VEL_SCALE_VEHICLE: f32 = f32::from_bits(0x3d08_8889); // 1/30
pub const DELTA_SCALE: f32 = f32::from_bits(0x3c01_0204); // 1/127
pub const POS_Z_SCALE: f32 = 2.384_185_8e-7; // 1/2^22
pub const POS_Z_BIAS: f32 = 100.0;
pub const POS_Z_MIN: f32 = -100.0;
pub const POS_Z_MAX: f32 = 4096.0;
pub const DEFAULT_CELL_SIZE: f32 = 4196.0;
pub const TWO_PI_F64: f64 = core::f64::consts::TAU;
pub const ANGLE_ZERO: f32 = f32::from_bits(0x38d1_b717);
pub const AXIS_LEN_MIN: f32 = f32::from_bits(0x3882_0610);
pub const AXIS_LEN_MAX: f32 = f32::from_bits(0x3f7f_fbf0);
pub const FALL_VEL_SCALE: f32 = 0.007_812_5; // 1/128
pub const FALL_VEL_MAX: f32 = 128.0;

/// The engine's 64-bit fixed position: two 64-bit axes and a float z.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FixedPos {
    pub x: i64,
    pub y: i64,
    pub z: f32,
}

/// `rsqrtss` followed by one Newton–Raphson step, as `FUN_397ae020` and
/// `FUN_397adca0` use it.
///
/// The `rsqrtss` approximation itself is implementation-defined in its last bits
/// and differs between CPU vendors (and between SSE versions), so the golden
/// values recorded by these tests come from one machine; a few last-bit
/// differences on another CPU are not a logic change.
#[inline]
fn rsqrt_newton(v: f32) -> f32 {
    #[cfg(target_arch = "x86_64")]
    {
        use core::arch::x86_64::{_mm_cvtss_f32, _mm_rsqrt_ss, _mm_set_ss};
        let n = unsafe { _mm_cvtss_f32(_mm_rsqrt_ss(_mm_set_ss(v))) };
        (1.5 - n * v * n * 0.5) * n
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        (1.5 - (1.0 / v.sqrt()) * v * (1.0 / v.sqrt()) * 0.5) * (1.0 / v.sqrt())
    }
}

/// The writer's `(int)floor(x + 0.5)` for a value already offset by 0.5.
///
/// `FUN_397ad5a0`/`FUN_397ad8c0`/`FUN_397ada60`/`FUN_397ad740` use the SSE
/// sequence with a ±2^23 magic; reproduce it operation-for-operation.
#[inline]
pub fn round_half_floor_f32(x: f32) -> i32 {
    let sign = if x < 0.0 { -0.0f32 } else { 0.0f32 };
    let abs_bits = x.to_bits() & 0x7fff_ffff;
    let magic = if abs_bits < 0x4b00_0000 {
        f32::from_bits(sign.to_bits() | 0x4b00_0000)
    } else {
        sign
    };
    let rounded = (x + magic) - magic;
    let err = rounded - x;
    let corr = if err > sign { 1.0 } else { 0.0 };
    (rounded - corr) as i32
}

/// The writer's double variant (`FUN_397ae020`/`FUN_397adca0`): ±2^52 magic
/// and a one-unit correction, with `1.0` selected by the comparison.
#[inline]
pub fn round_half_floor_f64(x: f32) -> i32 {
    let x = x as f64;
    let sign_bits = if x < 0.0 { 0x8000_0000_0000_0000u64 } else { 0 };
    let sign = f64::from_bits(sign_bits);
    let abs = x.abs();
    let magic = if abs < 4_503_599_627_370_496.0 {
        f64::from_bits(sign_bits | 0x4330_0000_0000_0000)
    } else {
        sign
    };
    let rounded = (x + magic) - magic;
    let err = rounded - x;
    let corr = if err > sign { 1.0 } else { 0.0 };
    (rounded - corr) as i32
}

#[inline]
fn clamp_unit(x: f32) -> f32 {
    if x < -1.0 {
        -1.0
    } else if x < 1.0 {
        x
    } else {
        1.0
    }
}

/// `FUN_397ad5a0` / `FUN_397ad8c0` / `FUN_397ada60` (write side).
pub fn write_i16_vec3(v: [f32; 3], scale: f32) -> [i16; 3] {
    let mut out = [0i16; 3];
    for (o, x) in out.iter_mut().zip(v) {
        let clamped = clamp_unit(x * scale);
        *o = round_half_floor_f32(clamped * 32767.0 + 0.5) as i16;
    }
    out
}

/// Read side of the same three codecs (inlined in `FUN_397c0470`).
pub fn read_i16_vec3(raw: [i16; 3], scale: f32) -> [f32; 3] {
    [
        raw[0] as f32 * I16_SCALE * scale,
        raw[1] as f32 * I16_SCALE * scale,
        raw[2] as f32 * I16_SCALE * scale,
    ]
}

/// `FUN_397ad740` (write side). The input is already a unit delta; the
/// function clamps it and multiplies by 127 (no pre-scale).
pub fn write_delta_i8_vec3(v: [f32; 3]) -> [i8; 3] {
    let mut out = [0i8; 3];
    for (o, x) in out.iter_mut().zip(v) {
        let clamped = clamp_unit(x);
        *o = round_half_floor_f32(clamped * 127.0 + 0.5) as i8;
    }
    out
}

/// Read side of `FUN_397ad740` (inlined in `FUN_397c0470`).
pub fn read_delta_i8_vec3(raw: [i8; 3]) -> [f32; 3] {
    [
        raw[0] as f32 * DELTA_SCALE,
        raw[1] as f32 * DELTA_SCALE,
        raw[2] as f32 * DELTA_SCALE,
    ]
}

/// `FUN_397ae020` (write side): normalise, flip the sign with w, quantise.
pub fn write_quat_i16(q: [f32; 4]) -> [i16; 3] {
    let dot = ((q[0] * q[0] + q[3] * q[3]) + q[1] * q[1]) + q[2] * q[2];
    let inv = rsqrt_newton(dot);
    let mut x = q[0] * inv;
    let mut y = q[1] * inv;
    let mut z = q[2] * inv;
    if q[3] * inv < 0.0 {
        x = -x;
        y = -y;
        z = -z;
    }
    [
        round_half_floor_f64(x * 32767.0 + 0.5) as i16,
        round_half_floor_f64(y * 32767.0 + 0.5) as i16,
        round_half_floor_f64(z * 32767.0 + 0.5) as i16,
    ]
}

/// `FUN_397adc00` (read side).
pub fn read_quat_i16(raw: [i16; 3]) -> [f32; 4] {
    let x = raw[0] as f32 * I16_SCALE;
    let y = raw[1] as f32 * I16_SCALE;
    let z = raw[2] as f32 * I16_SCALE;
    let d = x * x + y * y + z * z;
    let w = if d > QUAT_W_LIMIT {
        0.0
    } else {
        (1.0 - d).sqrt()
    };
    [x, y, z, w]
}

/// `FUN_397adca0` (write side): quaternion -> 3×i8 axis-angle.
pub fn write_quat_i8(q: [f32; 4]) -> [i8; 3] {
    // Two-step clamp of w to [-1, 1], exactly as the original fabs idiom.
    let a = (q[3] + 1.0 - (q[3] - 1.0).abs()) * 0.5;
    let w = ((a + 1.0).abs() + (a - 1.0)) * 0.5;

    let angle = libm_sse2_acosf(w);
    let a2 = angle * 2.0;
    if a2 <= ANGLE_ZERO {
        return [0, 0, 0];
    }

    let s = libm_sse2_sinf(a2 * 0.5);
    let inv = 1.0 / s;
    let mut x = q[0] * inv;
    let mut y = q[1] * inv;
    let mut z = q[2] * inv;

    let len2 = y * y + x * x + z * z;
    if len2 <= f32::EPSILON {
        x = 1.0;
        y = 0.0;
        z = 0.0;
    } else {
        let n = rsqrt_newton(len2);
        x *= n;
        y *= n;
        z *= n;
    }

    let k = (a2 as f64 * 0.159_154_943_091_895_35) as f32;
    [
        round_half_floor_f64(x * k * 127.0 + 0.5) as i8,
        round_half_floor_f64(y * k * 127.0 + 0.5) as i8,
        round_half_floor_f64(z * k * 127.0 + 0.5) as i8,
    ]
}

#[cfg(target_arch = "x86_64")]
fn fsincos_f32(x: f32) -> (f32, f32) {
    use core::arch::asm;
    let mut sin = 0.0f32;
    let mut cos = 0.0f32;
    unsafe {
        asm!(
            "fld dword ptr [{x}]",
            "fsincos",
            "fstp dword ptr [{cos}]",
            "fstp dword ptr [{sin}]",
            x = in(reg) &x,
            cos = in(reg) &mut cos,
            sin = in(reg) &mut sin,
        );
    }
    (sin, cos)
}

#[cfg(not(target_arch = "x86_64"))]
fn fsincos_f32(x: f32) -> (f32, f32) {
    (x.sin(), x.cos())
}

/// `FUN_3938b460` (read side): 3×i8 axis-angle -> quaternion.
pub fn read_quat_i8(raw: [i8; 3]) -> [f32; 4] {
    let x = raw[0] as f32 * DELTA_SCALE;
    let y = raw[1] as f32 * DELTA_SCALE;
    let z = raw[2] as f32 * DELTA_SCALE;
    let len2 = x * x + y * y + z * z;
    if len2 > AXIS_LEN_MAX || len2 < AXIS_LEN_MIN {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let len = len2.sqrt();
    let inv = 1.0 / len;
    let x = x * inv;
    let y = y * inv;
    let z = z * inv;
    let angle = ((len as f64 * TWO_PI_F64) as f32) * 0.5;
    let (sin, cos) = fsincos_f32(angle);
    [x * sin, y * sin, z * sin, cos]
}

#[inline]
fn u24(b: &[u8]) -> u32 {
    b[0] as u32 | (b[1] as u32) << 8 | (b[2] as u32) << 16
}

/// The write side of `FUN_3961e2c0` takes the **high dword** of the 64-bit
/// fixed axis (the low dword is the fraction), makes it positive with the
/// two's-complement carry of `low != 0`, then shifts the result right by 3.
fn axis_wire(v: i64) -> u32 {
    let hi = (v >> 32) as u32;
    let lo = v as u32;
    let sign = ((hi as i32) >> 31) as u32;
    let (_sum, carry) = sign.overflowing_add(lo);
    let abs_hi = sign.wrapping_add(hi).wrapping_add(carry as u32) ^ sign;
    ((abs_hi as i32) >> 3) as u32
}

/// `FUN_3961e2c0` write direction: `FixedPos` -> 9 wire bytes.
pub fn write_position(pos: FixedPos, cell_size: f32) -> [u8; 9] {
    let xq = axis_wire(pos.x);
    let yq = axis_wire(pos.y);

    let z = if pos.z < POS_Z_MIN {
        POS_Z_MIN
    } else if pos.z < POS_Z_MAX {
        pos.z
    } else {
        POS_Z_MAX
    };
    let zq =
        ((((z as f64 - POS_Z_MIN as f64) / cell_size as f64) * 4_194_304.0 + 0.5).floor()) as i64;

    let zq = zq as u32;
    [
        xq as u8,
        (xq >> 8) as u8,
        (xq >> 16) as u8,
        yq as u8,
        (yq >> 8) as u8,
        (yq >> 16) as u8,
        zq as u8,
        (zq >> 8) as u8,
        ((if pos.x < 0 { 0x80 } else { 0 })
            | (if pos.y < 0 { 0x40 } else { 0 })
            | ((zq >> 16) as u8 & 0x3f)),
    ]
}

/// `FUN_3961e2c0` read direction: 9 wire bytes -> `FixedPos`.
pub fn read_position(bytes: &[u8; 9], cell_size: f32) -> FixedPos {
    // `__allmul((0, u24*8), sign, sign>>31)`: the magnitude lands in the high
    // dword of the 64-bit fixed.
    let x = ((u24(&bytes[0..3]) as i64) * 8) << 32;
    let y = ((u24(&bytes[3..6]) as i64) * 8) << 32;
    let zq = u24(&[bytes[6], bytes[7], bytes[8] & 0x3f]);
    let z = (zq as f64 * POS_Z_SCALE as f64 * cell_size as f64 - POS_Z_BIAS as f64) as f32;
    FixedPos {
        x: if bytes[8] & 0x80 != 0 { -x } else { x },
        y: if bytes[8] & 0x40 != 0 { -y } else { y },
        z,
    }
}
