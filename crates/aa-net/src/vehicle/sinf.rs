//! MSVCR100 `__libm_sse2_sinf` (export ordinal 384, VA `0x78b431a0`, RVA `0xa31a0`), the
//! float sine `FUN_3912ebc0` calls at `0x3912ecc2` / `0x3912ed5c` (import thunk
//! `x2game!0x398b57e2`).
//!
//! Ported instruction by instruction: the exponent gate, the `32/pi` quadrant reduction with
//! the magic-constant rounding, the sign/quadrant XOR masks, the 64-entry `(sin,cos)` table at
//! `0x78b432f0` and the two degree-4 minimax polynomials.  The reduction is `f64` throughout,
//! exactly as the original (`cvtss2sd`/`mulsd`), and only the final result is rounded to
//! `f32`.
//!
//! The original's fallback for `|x| >= 2^13` calls the CRT double `sin` (`0x78aa39a0`); that
//! path is outside the movement call sites (all angles are bounded) and is delegated to the
//! platform `f64::sin`, which is documented as a tolerance in `docs/research/port-movement`.
//! `±inf`/NaN and the subnormal path are reproduced exactly.

const TWO_OVER_PI_32: f32 = f32::from_bits(0x4122_f983); // 32/pi
const MAGIC: f32 = 12582912.0;
const SPLIT_A: u64 = 0x77b9_21fb_5444_4000;
const SPLIT_B: u64 = 0x7532_e7b9_6767_33af;
const TINY_SCALE: f64 = f64::from_bits(0x3fef_ffff_ff80_0000);

static TABLE: [(u64, u64); 64] = [
    (0x0000000000000000, 0x3ff0000000000000),
    (0x3fb917a6bc29b42c, 0x3fefd88da3d12526),
    (0x3fc8f8b83c69a60b, 0x3fef6297cff75cb0),
    (0x3fd294062ed59f06, 0x3fee9f4156c62dda),
    (0x3fd87de2a6aea963, 0x3fed906bcf328d46),
    (0x3fde2b5d3806f63b, 0x3fec38b2f180bdb1),
    (0x3fe1c73b39ae68c8, 0x3fea9b66290ea1a3),
    (0x3fe44cf325091dd6, 0x3fe8bc806b151741),
    (0x3fe6a09e667f3bcd, 0x3fe6a09e667f3bcd),
    (0x3fe8bc806b151741, 0x3fe44cf325091dd6),
    (0x3fea9b66290ea1a3, 0x3fe1c73b39ae68c8),
    (0x3fec38b2f180bdb1, 0x3fde2b5d3806f63b),
    (0x3fed906bcf328d46, 0x3fd87de2a6aea963),
    (0x3fee9f4156c62dda, 0x3fd294062ed59f06),
    (0x3fef6297cff75cb0, 0x3fc8f8b83c69a60b),
    (0x3fefd88da3d12526, 0x3fb917a6bc29b42c),
    (0x3ff0000000000000, 0x0000000000000000),
    (0x3fefd88da3d12526, 0xbfb917a6bc29b42c),
    (0x3fef6297cff75cb0, 0xbfc8f8b83c69a60b),
    (0x3fee9f4156c62dda, 0xbfd294062ed59f06),
    (0x3fed906bcf328d46, 0xbfd87de2a6aea963),
    (0x3fec38b2f180bdb1, 0xbfde2b5d3806f63b),
    (0x3fea9b66290ea1a3, 0xbfe1c73b39ae68c8),
    (0x3fe8bc806b151741, 0xbfe44cf325091dd6),
    (0x3fe6a09e667f3bcd, 0xbfe6a09e667f3bcd),
    (0x3fe44cf325091dd6, 0xbfe8bc806b151741),
    (0x3fe1c73b39ae68c8, 0xbfea9b66290ea1a3),
    (0x3fde2b5d3806f63b, 0xbfec38b2f180bdb1),
    (0x3fd87de2a6aea963, 0xbfed906bcf328d46),
    (0x3fd294062ed59f06, 0xbfee9f4156c62dda),
    (0x3fc8f8b83c69a60b, 0xbfef6297cff75cb0),
    (0x3fb917a6bc29b42c, 0xbfefd88da3d12526),
    (0x0000000000000000, 0xbff0000000000000),
    (0xbfb917a6bc29b42c, 0xbfefd88da3d12526),
    (0xbfc8f8b83c69a60b, 0xbfef6297cff75cb0),
    (0xbfd294062ed59f06, 0xbfee9f4156c62dda),
    (0xbfd87de2a6aea963, 0xbfed906bcf328d46),
    (0xbfde2b5d3806f63b, 0xbfec38b2f180bdb1),
    (0xbfe1c73b39ae68c8, 0xbfea9b66290ea1a3),
    (0xbfe44cf325091dd6, 0xbfe8bc806b151741),
    (0xbfe6a09e667f3bcd, 0xbfe6a09e667f3bcd),
    (0xbfe8bc806b151741, 0xbfe44cf325091dd6),
    (0xbfea9b66290ea1a3, 0xbfe1c73b39ae68c8),
    (0xbfec38b2f180bdb1, 0xbfde2b5d3806f63b),
    (0xbfed906bcf328d46, 0xbfd87de2a6aea963),
    (0xbfee9f4156c62dda, 0xbfd294062ed59f06),
    (0xbfef6297cff75cb0, 0xbfc8f8b83c69a60b),
    (0xbfefd88da3d12526, 0xbfb917a6bc29b42c),
    (0xbff0000000000000, 0x0000000000000000),
    (0xbfefd88da3d12526, 0x3fb917a6bc29b42c),
    (0xbfef6297cff75cb0, 0x3fc8f8b83c69a60b),
    (0xbfee9f4156c62dda, 0x3fd294062ed59f06),
    (0xbfed906bcf328d46, 0x3fd87de2a6aea963),
    (0xbfec38b2f180bdb1, 0x3fde2b5d3806f63b),
    (0xbfea9b66290ea1a3, 0x3fe1c73b39ae68c8),
    (0xbfe8bc806b151741, 0x3fe44cf325091dd6),
    (0xbfe6a09e667f3bcd, 0x3fe6a09e667f3bcd),
    (0xbfe44cf325091dd6, 0x3fe8bc806b151741),
    (0xbfe1c73b39ae68c8, 0x3fea9b66290ea1a3),
    (0xbfde2b5d3806f63b, 0x3fec38b2f180bdb1),
    (0xbfd87de2a6aea963, 0x3fed906bcf328d46),
    (0xbfd294062ed59f06, 0x3fee9f4156c62dda),
    (0xbfc8f8b83c69a60b, 0x3fef6297cff75cb0),
    (0xbfb917a6bc29b42c, 0x3fefd88da3d12526),
];

#[inline]
fn d(bits: u64) -> f64 {
    f64::from_bits(bits)
}

/// `__libm_sse2_sinf(x)` from MSVCR100 `0x78b431a0`.
pub fn libm_sse2_sinf(x: f32) -> f32 {
    let bits = x.to_bits();
    let gate = (((bits >> 16) & 0x7fff).wrapping_sub(0x80)) & 0xffff;

    if gate > 0x457f {
        if (gate as i16) <= 0x457f {
            // |x| < 2^-126 (or zero): one f64 multiply by the odd reflection constant.
            let scaled = ((x as f64) * TINY_SCALE) as f32;
            // The client runs with MXCSR FTZ set, so a subnormal f32 result flushes to zero.
            return if scaled.is_subnormal() {
                0.0f32
            } else {
                scaled
            };
        }
        if bits & 0x7f80_0000 == 0x7f80_0000 {
            return if x.is_nan() {
                x
            } else {
                f32::from_bits(0xffc0_0000)
            };
        }
        return (x as f64).sin() as f32;
    }

    let t = x * TWO_OVER_PI_32;
    let n = t.round_ties_even() as i32;
    let nf = (t + MAGIC) - MAGIC;
    let sign = (bits & 0x8000_0000) as u64;
    let flip = (sign << 32) ^ (sign << 29);
    let index = (n.wrapping_add(0x1c_7600) & 0x3f) as usize;

    let c2 = d(SPLIT_A ^ flip);
    let c3 = d(SPLIT_B ^ flip);
    let scale = d((nf.to_bits() as u64) << 29);
    let y = (x as f64) - c2 * scale;
    let z = c3 * scale + y;

    let y2 = y * y;
    let y4 = y2 * y2;
    let cos_poly = (1.0 + (-0.5) * y2) + (1.0 / 24.0) * y4;
    let sin_poly = (1.0 + (-1.0 / 6.0) * y2) + (1.0 / 120.0) * y4;

    let (ts, tc) = TABLE[index];
    let out = d(ts) * cos_poly + (z * d(tc)) * sin_poly;
    out as f32
}
