//! Native far-plane dissolve: objects fade out over the last metres of their view distance instead of popping.
//!
//! Cry3DEngine 1.2 (`CVegetation::Render` 300F36E0, the render-node path 30111AE0 and the brush path): with
//! `e_dissolve 1` (the client's default `option_shader_quality` group) an object at camera distance `d` with
//! maximum view distance `mvd` gets the alpha-test noise reference
//! `clamp((d - mvd) / band + 1, 0, 1) * 255` (30112410), where
//! `band = clamp(e_DissolveDistFactor * mvd, e_DissolveDistMin, e_DissolveDistMax)` = `clamp(0.025 mvd, 2, 8)`
//! (defaults registered by 3000C800). Reference 0 draws normally, 255 skips the object, anything between draws
//! it dissolved. Bevy's [`VisibilityRange`] crossfade is the same idea: a screen-space dither pattern over a
//! distance margin, so the native band becomes the range's end margin.

use bevy::camera::visibility::VisibilityRange;

/// `e_DissolveDistFactor`.
const FACTOR: f32 = 0.025;
/// `e_DissolveDistMin` / `e_DissolveDistMax` (m).
const MIN_BAND: f32 = 2.0;
const MAX_BAND: f32 = 8.0;

/// Metres over which an object with view distance `view` dissolves (native clamp order: min first, then max).
pub fn band(view: f32) -> f32 {
    let band = FACTOR * view;
    if MIN_BAND > band {
        MIN_BAND
    } else if MAX_BAND <= band {
        MAX_BAND
    } else {
        band
    }
}

/// Rounds a distance to whole metres. Bevy keeps one GPU slot per distinct range for the whole run (never
/// freed, at most 65535), so per-instance float distances must not each make a new one.
fn quantize(distance: f32) -> f32 {
    distance.round().max(1.0)
}

/// Drawn up to `view`, dissolving over the native band before it.
pub fn range(view: f32) -> VisibilityRange {
    let view = quantize(view);
    VisibilityRange {
        start_margin: 0.0..0.0,
        end_margin: (view - band(view)).max(0.0)..view,
        use_aabb: false,
    }
}

/// Drawn up to `until` (where something else takes over, e.g. a vegetation sprite), crossfading over the
/// native band before it; [`appear`] with the same `until` is the exact complement.
pub fn hand_over(until: f32) -> VisibilityRange {
    range(until)
}

/// Fade-in margin of whatever takes over from [`hand_over`]`(at)`: the same band, so the two dither patterns
/// complement each other pixel for pixel.
pub fn appear(at: f32) -> std::ops::Range<f32> {
    let at = quantize(at);
    (at - band(at)).max(0.0)..at
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_band_clamps_two_and_a_half_percent_rule() {
        assert_eq!(band(40.0), 2.0);
        assert_eq!(band(80.0), 2.0);
        assert_eq!(band(200.0), 5.0);
        assert_eq!(band(320.0), 8.0);
        assert_eq!(band(2000.0), 8.0);
    }

    #[test]
    fn ranges_end_at_the_view_distance_and_share_slots() {
        let r = range(600.4);
        assert_eq!(r.end_margin, 592.0..600.0);
        assert_eq!(r.start_margin, 0.0..0.0);
        assert!(!r.use_aabb && !r.is_abrupt());
        assert_eq!(range(600.2).end_margin, range(599.6).end_margin);
        // a tiny prop still gets a margin, never a negative one
        assert_eq!(range(1.2).end_margin, 0.0..1.0);
        assert_eq!(hand_over(150.0).end_margin, appear(150.0));
    }
}
