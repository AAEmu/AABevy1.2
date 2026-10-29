//! MSVCR100 `__libm_sse2_acosf` (export ordinal 367, VA `0x78b33968`, RVA `0x93968`), the
//! function the original `FUN_3912ebc0` calls through the import thunk `x2game!0x398b57fa`.
//!
//! Ported instruction by instruction from the disassembly: the magnitude branches on
//! `abs(x) - 0.0625`, the shared `sqrt`/mantissa-truncation reductions, the polynomial pair
//! tables at `0x78b33d50` / `0x78b34550` (the same tables `asinf` uses) and the final
//! reconstruction.  Only lane 0/1 of the SIMD temporaries is observable; the scalar form
//! below keeps the original operation order.
//!
//! Range: the function has no x87/double fallback for finite `f32`; every branch is covered,
//! including `|x| > 1` (NaN) and `x == ±1`.

use super::asinf_tables::{COEF, POLY};

const NEG_EPS: f32 = f32::from_bits(0xb42e_ef4c); // 0x78b34950 = -1.6292068e-7
const HALF_PI: f32 = f32::from_bits(0x3fc9_0fdc); // 0x78b34954 = 1.5707965
const EXTRA_EPS: f32 = f32::from_bits(0x33bb_bd2e); // 0x78b34960 = +8.742278e-8
const PI: f32 = f32::from_bits(0xc049_0fdb); // 0x78b34964 = -pi (used by the sign add)
const NEG_EXTRA_EPS: f32 = f32::from_bits(0xb3bb_bd2e); // 0x78b34970 = -8.742278e-8
const POS_PI: f32 = f32::from_bits(0x4049_0fdb); // 0x78b34974 = +pi

const SIXTH: f32 = f32::from_bits(0x3e2a_aaab); // 0x3e2aaaab
const NEG_SIXTH: f32 = f32::from_bits(0xbe2a_aaab); // 0xbe2aaaab
const C_0_075: f32 = f32::from_bits(0xbd99_999a);
const C_0_044642858: f32 = f32::from_bits(0xbd36_db6e);

#[inline]
fn poly_pair(index: u32) -> (f32, f32) {
    let i = index as usize;
    (f32::from_bits(POLY[i]), f32::from_bits(POLY[i + 1]))
}

#[inline]
fn coef(index: u32) -> f32 {
    f32::from_bits(COEF[index as usize])
}

/// `__libm_sse2_acosf(x)` from MSVCR100 `0x78b33968`.
pub fn libm_sse2_acosf(x: f32) -> f32 {
    let bits = x.to_bits();
    let sign = (bits & 0x8000_0000) != 0;
    let mut edx = (bits & 0x7fff_ffff).wrapping_sub(0x3d80_0000);

    // 0x78b33994: 0.0625 <= |x| < sqrt(3)/2.
    if edx < 0x01dd_b3d7 {
        let xh = f32::from_bits((bits & 0xfffe_0000) | 0x0001_0000);
        let s = (1.0 - x * x).sqrt();
        edx >>= 16;
        edx &= 0x1fe;
        let t = coef(edx / 2);
        let (tiny, coef) = poly_pair(edx);
        let (eps, rel) = if sign {
            (NEG_EPS + tiny, HALF_PI + coef)
        } else {
            (NEG_EPS - tiny, HALF_PI - coef)
        };
        let ts = x * t;
        let xhs = xh * s;
        let d = xhs - ts;
        let ratio = (x - xh) * (x + xh) / (ts + xhs);
        let poly = (SIXTH * d) * (d * d) + eps;
        let u = rel - ratio;
        let v = rel - u;
        let w = ratio - v;
        return (poly - w) + u;
    }

    // 0x78b33a55: sqrt(3)/2 <= |x| < 0.9921875.
    edx = edx.wrapping_sub(0x01dd_b3d7);
    if edx < 0x0020_4c29 {
        let xh = f32::from_bits(bits & 0xffff_f000);
        let h = x - xh;
        let s = (1.0 - xh * xh - (x + xh) * h).sqrt();
        let s_masked = f32::from_bits(s.to_bits() & 0xfffe_0000);
        let sign_word = if sign { 0x8000_0000 } else { 0 };
        let sm = f32::from_bits(s_masked.to_bits() | sign_word | 0x0001_0000);
        edx = ((s.to_bits() >> 16).wrapping_sub(0x3d80)) & 0xfffe;
        let t = coef(edx / 2);
        let (p0, p1) = poly_pair(edx);
        let (t2, p2) = if sign {
            (p0 + EXTRA_EPS, p1 + PI)
        } else {
            (p0, p1)
        };
        let a = xh * sm - s * t;
        let b = h * sm;
        let c = a + b;
        let mut r = (1.0 - xh * xh) - sm * sm;
        r -= (x + xh) * h;
        let den = (s * t) + (s * t) + c;
        let div = r / den;
        let poly = (NEG_SIXTH * c) * (c * c) + t2;
        let out = ((poly + div) + p2).to_bits() ^ (if sign { 0x8000_0000 } else { 0 });
        return f32::from_bits(out);
    }

    // 0x78b33b53: 0.9921875 <= |x| < 1.0 (plus the tiny-magnitude wrap-around).
    edx = edx.wrapping_add(0x07dd_b3d7);
    if edx < 0x0600_0000 {
        let x2 = x * x;
        let d = (x2 * x2) * (C_0_075 * x) + ((NEG_SIXTH * x) * x2 + NEG_EPS);
        let shifted = HALF_PI + (x - HALF_PI);
        let z = x - shifted;
        return (d - z) - (x - HALF_PI);
    }

    // 0x78b33bc7: |x| in [1.0, 1.0039) (the sqrt-based Newton branch).
    edx = edx.wrapping_sub(0x07fe_0000);
    if edx < 0x0002_0000 {
        let t = 0.5 - f32::from_bits(bits & 0x7fff_ffff) * 0.5;
        let s = t.sqrt();
        let st = f32::from_bits(s.to_bits() & 0xffff_f000);
        let t2 = t * t;
        let poly = NEG_SIXTH + C_0_075 * t;
        let poly = poly + C_0_044642858 * t2;
        let poly = poly * t;
        let ds = s - st;
        let two_s = s + s;
        let y = t - st * st;
        let rest = ds * (two_s - ds);
        let y = (y - rest) / s;
        let num = poly * two_s;
        let (eps, rel) = if sign {
            (NEG_EXTRA_EPS, POS_PI)
        } else {
            (0.0, 0.0)
        };
        let out = (rel + (num + eps - y - two_s)).to_bits() ^ (if sign { 0 } else { 0x8000_0000 });
        return f32::from_bits(out);
    }

    // 0x78b33cb6: special values.
    edx = edx.wrapping_add(0x3f7e_0000);
    if edx < 0x3780_0000 {
        let x1 = x - HALF_PI;
        let x2 = HALF_PI + x1;
        let x2 = x2 - x;
        return (NEG_EPS + x2) - x1;
    }
    if edx == 0x3f80_0000 {
        return if sign { POS_PI + NEG_EXTRA_EPS } else { 0.0 };
    }
    if edx > 0x7f80_0000 {
        return x + 0.0;
    }
    f32::from_bits(0xffc0_0000)
}
