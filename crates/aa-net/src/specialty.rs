//! Pure native specialty arithmetic; context and static-row admission belong to the caller.

#[derive(Clone, Copy, Debug)]
pub struct KnownBundleQuote {
    pub refund: i32,
    pub refund_multiplier: i32,
    pub profit: i32,
    pub profit_ratio: i32,
    pub received_ratio: i32,
    pub npc_ratio: f32,
    pub coin_divisor: Option<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpecialtyQuote {
    pub refund: i32,
    pub coin_amount: Option<i32>,
}

/// Executes the known-bundle numeric branch with engine-thread x87 PC64 precision.
/// This establishes neither NPC eligibility nor permission to submit a sale.
pub fn quote_known_bundle(input: KnownBundleQuote) -> SpecialtyQuote {
    let base = native_round(input.refund.wrapping_mul(input.refund_multiplier) as f32 * 0.01f32);
    let bundle =
        native_round(input.profit.wrapping_mul(input.profit_ratio) as f32 * 0.001f32 + base as f32);
    let ratio = extended_product(input.received_ratio, &[0.01f32]);
    let amount = extended_product(bundle, &[input.npc_ratio, 0.01f32, ratio]);
    let refund = native_round(amount);
    SpecialtyQuote {
        refund,
        coin_amount: input
            .coin_divisor
            .map(|d| native_round(refund as f32 / d as f32)),
    }
}

fn native_round(value: f32) -> i32 {
    let value = (value + 0.5f32).floor();
    if !value.is_finite() || value >= 2147483648.0 || value < -2147483648.0 {
        i32::MIN
    } else {
        value as i32
    }
}

fn rounded(n: u128, bits: u32) -> (u128, i32) {
    let shift = (128 - n.leading_zeros()).saturating_sub(bits);
    if shift == 0 {
        return (n, 0);
    }
    let q = n >> shift;
    let tail = n & ((1u128 << shift) - 1);
    let half = 1u128 << (shift - 1);
    (
        q + u128::from(tail > half || (tail == half && q & 1 != 0)),
        shift as i32,
    )
}

// Each native multiplication is rounded to the x87 64-bit significand. All
// operands here are binary32 or i32, so a single product fits in u128.
fn extended_product(integer: i32, factors: &[f32]) -> f32 {
    let mut sign = integer < 0;
    let mut n = 1u128;
    let mut exponent = 0i32;
    for &value in factors {
        let bits = value.to_bits();
        sign ^= bits >> 31 != 0;
        let e = ((bits >> 23) & 255) as i32;
        if e == 255 {
            return f32::NAN;
        }
        let m = if e == 0 {
            bits & 0x7fffff
        } else {
            (bits & 0x7fffff) | 0x800000
        };
        n *= u128::from(m);
        exponent += if e == 0 { -149 } else { e - 150 };
        let (next, shift) = rounded(n, 64);
        n = next;
        exponent += shift;
    }
    n *= u128::from(integer.unsigned_abs());
    let (n, shift) = rounded(n, 64);
    to_f32(sign, n, exponent + shift)
}

fn to_f32(sign: bool, n: u128, mut exponent: i32) -> f32 {
    let sign = u32::from(sign) << 31;
    if n == 0 {
        return f32::from_bits(sign);
    }
    let bits = 128 - n.leading_zeros();
    if exponent + bits as i32 - 1 < -126 {
        let shift = -149 - exponent;
        let mantissa = if shift <= 0 {
            n << (-shift as u32)
        } else if shift >= 128 {
            0
        } else {
            let q = n >> shift;
            let tail = n & ((1u128 << shift) - 1);
            let half = 1u128 << (shift - 1);
            q + u128::from(tail > half || (tail == half && q & 1 != 0))
        };
        return f32::from_bits(sign | mantissa as u32);
    }
    let target_bits = 24;
    let (mut n, shift) = rounded(n, target_bits);
    exponent += shift;
    if n == 0 {
        return f32::from_bits(sign);
    }
    let mut bits = 128 - n.leading_zeros();
    if bits > 24 {
        n >>= 1;
        exponent += 1;
        bits -= 1;
    }
    let top = exponent + bits as i32 - 1;
    if top > 127 {
        return f32::from_bits(sign | 0x7f800000);
    }
    f32::from_bits(sign | (((top + 127) as u32) << 23) | (((n << (24 - bits)) as u32) & 0x7fffff))
}

#[cfg(test)]
mod tests;
