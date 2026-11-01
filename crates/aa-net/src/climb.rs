//! `actor.climbData` of a hanging unit movement (actor flag `0x40`).
//!
//! The 1.2 writer is `FUN_390cbee0` (the local player's unit model vtable slot `+0x60`, called from the
//! `CSMoveUnit` sender `FUN_394046f0`): with the player hanging (`Player+0x1db != 0`) and not in the
//! top exit (`Player+0x1dd == 0`) it packs three numbers into a `u32`:
//!
//! ```text
//! climbData = angle8 << 24 | y10 << 14 | x14
//! x14      = trunc(clamp(P+0x244 * 0.01, 0, 1) * 16383)   ; metres along the climb, 0..100 m
//! y10      = trunc(clamp(P+0x248 * 0.05, 0, 1) * 1023)    ; lateral offset, 0..20 m
//! angle8   = trunc(clamp((atan2(...) + pi) / 2pi, 0, 1) * 255)
//! ```
//!
//! The values are copied from the ladder frame by `Player::GrabOnLadder` `FUN_391388f0` and do not
//! change while the hang lasts. The kinds 1, 4, 5, 8 and 9 carry an identity relative rotation, so
//! their angle is 127 (`trunc(127.5)`); see `2026-10-31-decompile-pass2.md` §1.2-1.3.
//!
//! The remote decoder is `FUN_3909db50`: `x = bits/16383 * 100`, `y = bits/1023 * 20`,
//! `angle = bits/255 * 2pi - pi`.

use std::f32::consts::{PI, TAU};

/// The low 14 bits (`x14`): 0..=100 m.
pub const X_MAX_M: f32 = 100.0;
/// The next 10 bits (`y10`): 0..=20 m.
pub const Y_MAX_M: f32 = 20.0;

const X_SCALE: u32 = 0x3fff;
const Y_SCALE: u32 = 0x3ff;
const ANGLE_SCALE: u32 = 0xff;

/// Packs the three climb fields exactly as `FUN_390cbee0` does: clamp to the wire ranges, scale,
/// then truncate toward zero (the writer's FPU control word is `0xC00`, round toward zero).
///
/// `x_m` is the distance along the climb axis in metres, `y_m` the lateral offset and
/// `angle_rad` the relative facing (ignored by the game for kinds 1, 4, 5, 8 and 9).
pub fn pack(x_m: f32, y_m: f32, angle_rad: f32) -> u32 {
    // The writer works in f32: `mulss` by 0.01 / 0.05 / 1/(2pi), then `fmul` by the field's
    // full-scale, then an x87 `fistp` with round-toward-zero.
    let x = ((x_m * 0.01).clamp(0.0, 1.0) * X_SCALE as f32).trunc() as u32;
    let y = ((y_m * 0.05).clamp(0.0, 1.0) * Y_SCALE as f32).trunc() as u32;
    let a = (((angle_rad + PI) / TAU).clamp(0.0, 1.0) * ANGLE_SCALE as f32).trunc() as u32;
    (a << 24) | (y << 14) | x
}

/// The `climbData` of a hang whose relative angle is the identity: kinds 1, 4, 5, 8 and 9 in 1.2
/// (`FUN_3916c520` returns `(0, 0, 0, 1)` for them, so the writer stores `trunc(127.5) = 127`).
pub fn pack_ladder(x_m: f32, y_m: f32) -> u32 {
    pack(x_m, y_m, 0.0)
}

/// The inverse of [`pack`], as `FUN_3909db50` decodes it: `(metres along, lateral metres, angle)`.
pub fn unpack(bits: u32) -> (f32, f32, f32) {
    let x = (bits & X_SCALE) as f32 / X_SCALE as f32 * X_MAX_M;
    let y = ((bits >> 14) & Y_SCALE) as f32 / Y_SCALE as f32 * Y_MAX_M;
    let a = (bits >> 24) as f32 / ANGLE_SCALE as f32 * TAU - PI;
    (x, y, a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_packing_uses_the_writer_s_ranges_and_bit_order() {
        assert_eq!(pack_ladder(0.0, 0.0), 127 << 24);
        assert_eq!(pack(100.0, 20.0, 0.0), 0x7fff_ffff);
        assert_eq!(pack(100.0, 20.0, std::f32::consts::PI), 0xffff_ffff);
        assert_eq!(pack(0.0, 0.0, std::f32::consts::PI), 0xff00_0000);
        assert_eq!(pack(0.0, 0.0, -std::f32::consts::PI), 0);
        // x is distance in metres * 0.01; 50 m is half of 16383
        assert_eq!(pack_ladder(50.0, 0.0) & X_SCALE, 8191);
        // y is lateral metres * 0.05; 10 m is half of 1023
        assert_eq!((pack_ladder(0.0, 10.0) >> 14) & Y_SCALE, 511);
        // kinds 1, 4, 5, 8, 9 (identity relative rotation): trunc(127.5) = 127
        for x in [0.0, 0.05, 1.234, 100.0] {
            assert_eq!(pack_ladder(x, 0.0) >> 24, 127);
        }
    }

    #[test]
    fn every_field_truncates_toward_zero_not_rounds() {
        // 0.01 m -> 1.6383 -> 1 (rounding would give 2)
        assert_eq!(pack_ladder(0.01, 0.0) & X_SCALE, 1);
        // 0.05 m -> 2.5575 -> 2 (rounding would give 3)
        assert_eq!((pack_ladder(0.0, 0.05) >> 14) & Y_SCALE, 2);
        // an angle just above 0 -> 127.5... ; just below 0 -> 127.49...
        assert_eq!(pack(1.0, 1.0, 1.0e-4) >> 24, 127);
        assert_eq!(pack(1.0, 1.0, -1.0e-4) >> 24, 127);
        // 0.6 * 255 is not reached by truncation
        let angle = -std::f32::consts::PI + std::f32::consts::TAU * 0.6 / 255.0;
        assert_eq!(pack(0.0, 0.0, angle) >> 24, 0);
    }

    /// The writer and reader are f32 on both sides, so a decoded value that sits on a step
    /// boundary can re-encode one step low. Every field must stay within one step.
    #[test]
    fn the_decoder_recovers_the_writer_s_fields_within_one_step() {
        for bits in [0u32, 0x3fff, 0x4000, 0x80004000, 0x7fff_ffff, 0xffff_ffff] {
            assert_within_one_step(bits);
        }
        let mut state = 0x1234_5678u32;
        for _ in 0..1000 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            assert_within_one_step(state);
        }
    }

    fn assert_within_one_step(bits: u32) {
        let back = pack_from_unpacked(bits);
        let x = (back & X_SCALE) as i32 - (bits & X_SCALE) as i32;
        let y = (((back >> 14) & Y_SCALE) as i32) - (((bits >> 14) & Y_SCALE) as i32);
        let a = (back >> 24) as i32 - (bits >> 24) as i32;
        assert!(
            x.abs() <= 1 && y.abs() <= 1 && a.abs() <= 1,
            "{bits:#010x}: {x} {y} {a}"
        );
    }

    fn pack_from_unpacked(bits: u32) -> u32 {
        let (x, y, a) = unpack(bits);
        pack(x, y, a)
    }

    #[test]
    fn the_decoded_extremes_match_the_native_constants() {
        let (x, y, a) = unpack(0xffff_ffff);
        assert!((x - 100.0).abs() < 1e-3);
        assert!((y - 20.0).abs() < 1e-3);
        assert!((a - std::f32::consts::PI).abs() < 1e-6);
        let (x, y, a) = unpack(0);
        assert_eq!((x, y), (0.0, 0.0));
        assert!((a + std::f32::consts::PI).abs() < 1e-6);
    }
}
