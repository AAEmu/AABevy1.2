//! Where 1.2's follow camera stands and how far it tilts, in plain numbers (game axes: x east, y north, z up), no engine;
//! `camera.rs` puts the result on the Bevy camera.
//!
//! What 1.2 does (read in `x2game`, then measured in a running client with `tools/camera_probe.py`,
//! `tools/evidence/camera/`):
//!
//! * **The point followed** T is the model's slot 51 (`actor_model::camera_target` x2game+0xdcb60): for a character the `camera`
//!   bone (fixed bone slot 45, `skeleton::find_fixed_bones` cryanimation+0x11ce60), straight over the feet at its **bind-pose**
//!   height. A Hariharan woman's T stood 1.194 m over her feet in every sample of the run, standing, zooming and in first person.
//!   No idle clip has a track for that bone, so an animated pose that keeps `Bip01` on the ground would put it 0.34 m over the
//!   feet: the bind height is the one to use.
//! * **The lift** z is the character's `.cdf` `zOffsetList` (`<zOffset dist="1" height="0.1"/>`), a curve over the distance now
//!   (camera +0x48, `camera::set_start_dist` x2game+0x17bc90 and `camera::update_distance` x2game+0x16c2b0): before the first
//!   key its height, after the last its height, between two keys a cubic Hermite with monotone slopes
//!   (`spline::monotone_slopes` x2game+0x1779a0, `spline::hermite` x2game+0x177b90).
//! * **The camera** stands at `T - f d + (0, 0, z)` and looks along `f` (the arm in `camera::CollideWithWorld` x2game+0x175420),
//!   so it looks at `T + z` from `d` away: an orbit around [`pivot_height`]. At pitch 0 that is also the camera's own height over
//!   the feet.
//! * **The pitch** (`camera::rotate_by_mouse` x2game+0x16c040): down to -88 degrees; up, the arm to 50 degrees and the rest, to
//!   88 in all, turns the view while the camera stays where 50 put it ([`split_pitch`]).

/// `camera::tw_pitch_min` x2game+0xb0e1d8: the lowest the camera looks (radians, -88 degrees).
pub const PITCH_DOWN: f32 = -88.0 * std::f32::consts::PI / 180.0;
/// `camera::tw_pitch_arm_max` x2game+0xb0e1e0: the highest the arm turns up (radians, 50 degrees); the rest of the pitch turns the
/// view.
pub const PITCH_ARM_UP: f32 = 50.0 * std::f32::consts::PI / 180.0;
/// `camera::tw_pitch_max` x2game+0xb0e1dc: the highest the view looks (radians, 88 degrees).
pub const PITCH_UP: f32 = 88.0 * std::f32::consts::PI / 180.0;

/// The `camera` bone's height over the feet in the bind pose (m), by the model's prefix (`ha_f`), read from each player skeleton
/// (`objects/characters/<race>/<gender>/nude/<prefix>_base.chr`): what the pivot stands on until the player's own rig is at hand.
pub fn bind_camera_bone_height(prefix: &str) -> Option<f32> {
    Some(match prefix {
        "nu_m" | "el_m" => 1.463,
        "nu_f" => 1.346,
        "el_f" => 1.414,
        "ha_m" => 1.269,
        "ha_f" => 1.194,
        "fe_m" => 1.479,
        "fe_f" => 1.412,
        _ => return None,
    })
}

/// The `zOffsetList` of a character's `.cdf`: the keys `(distance, height)` by distance, a repeated distance keeping its first key
/// (two keys at one distance would make a span of 0).
pub fn parse_z_offsets(xml: &str) -> Vec<(f32, f32)> {
    let mut keys = Vec::new();
    let mut rest = xml;
    while let Some(at) = rest.find("<zOffset ") {
        rest = &rest[at + "<zOffset ".len()..];
        let tag = &rest[..rest.find('>').unwrap_or(rest.len())];
        let attr = |name: &str| {
            let from = tag.find(&format!("{name}=\""))? + name.len() + 2;
            let value = &tag[from..];
            value[..value.find('"')?].trim().parse::<f32>().ok()
        };
        if let (Some(dist), Some(height)) = (attr("dist"), attr("height")) {
            keys.push((dist, height));
        }
    }
    keys.sort_by(|a, b| a.0.total_cmp(&b.0));
    keys.dedup_by(|b, a| a.0 == b.0);
    keys
}

/// The keys' slopes as `spline::monotone_slopes` makes them: each end key the secant of its span, an inner key the mean of its two
/// secants; then, span by span in order, a flat secant or a slope of the wrong sign sets both its slopes to 0, and `a^2 + b^2 > 9`
/// scales them by `3 / sqrt(a^2 + b^2)` (Fritsch-Carlson). In the running client the Hariharan woman's keys held -0.1, 0, 0.
fn slopes(keys: &[(f32, f32)]) -> Vec<f32> {
    let n = keys.len();
    let secant = |i: usize| (keys[i + 1].1 - keys[i].1) / (keys[i + 1].0 - keys[i].0);
    let mut m: Vec<f32> = (0..n)
        .map(|i| match i {
            0 => secant(0),
            i if i == n - 1 => secant(n - 2),
            i => 0.5 * (secant(i - 1) + secant(i)),
        })
        .collect();
    for i in 0..n - 1 {
        let s = secant(i);
        if s == 0.0 {
            m[i] = 0.0;
            m[i + 1] = 0.0;
            continue;
        }
        let (a, b) = (m[i] / s, m[i + 1] / s);
        if a < 0.0 || b < 0.0 {
            m[i] = 0.0;
            m[i + 1] = 0.0;
        } else if a * a + b * b > 9.0 {
            let k = 3.0 / (a * a + b * b).sqrt();
            m[i] = k * a * s;
            m[i + 1] = k * b * s;
        }
    }
    m
}

/// The `.cdf` height at the distance now (`spline::eval` x2game+0x179170): before the first key its height, after the last its
/// height, between two keys a cubic Hermite with the monotone slopes of [`slopes`] (each times the span). No keys: 0.
pub fn z_offset(keys: &[(f32, f32)], distance: f32) -> f32 {
    let Some(first) = keys.first() else { return 0.0 };
    let last = keys[keys.len() - 1];
    if keys.len() == 1 || distance <= first.0 {
        return first.1;
    }
    if distance >= last.0 {
        return last.1;
    }
    let m = slopes(keys);
    let i = keys.windows(2).position(|w| distance < w[1].0).unwrap_or(keys.len() - 2);
    let ((x0, y0), (x1, y1)) = (keys[i], keys[i + 1]);
    let span = x1 - x0;
    let t = (distance - x0) / span;
    let (t2, t3) = (t * t, t * t * t);
    (2.0 * t3 - 3.0 * t2 + 1.0) * y0 + (t3 - 2.0 * t2 + t) * span * m[i] + (-2.0 * t3 + 3.0 * t2) * y1 + (t3 - t2) * span * m[i + 1]
}

/// The orbit's pivot over the feet (m): the `camera` bone's bind height plus the `.cdf` height at the distance now.
pub fn pivot_height(bone: f32, keys: &[(f32, f32)], distance: f32) -> f32 {
    bone + z_offset(keys, distance)
}

/// A pitch as the mouse asks for it, held to the limits and split as 1.2 does: `(the arm's pitch, the view's turn up past the arm)`.
pub fn split_pitch(pitch: f32) -> (f32, f32) {
    let held = pitch.clamp(PITCH_DOWN, PITCH_UP);
    let arm = held.min(PITCH_ARM_UP);
    (arm, held - arm)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32, tolerance: f32) -> bool {
        (a - b).abs() <= tolerance
    }

    #[test]
    fn the_pivot_stands_where_the_native_client_put_a_hariharan_woman_at_every_zoom_step() {
        // ha_f.cdf's keys and her camera bone's bind height
        let keys = parse_z_offsets("<zOffsetList>\r\n  <zOffset dist=\"0\" height=\"0.20\"/>\r\n  <zOffset dist=\"1\" height=\"0.1\"/>\r\n  <zOffset dist=\"15\" height=\"0.8\"/>\r\n </zOffsetList>");
        assert_eq!(keys, vec![(0.0, 0.2), (1.0, 0.1), (15.0, 0.8)]);
        let bone = bind_camera_bone_height("ha_f").unwrap();
        // the running 1.2 client (tools/evidence/camera/native-ye-take1.csv): the camera's height over her feet, settled at each
        // wheel notch in from the 3.2 m start to first person and out to the 10 m limit, pitch 0; within 5 mm
        let native = [
            (3.2, 1.340), (2.7, 1.322), (2.2, 1.308), (1.7, 1.299), (1.2, 1.294), (0.0, 1.393), (1.0, 1.294), (1.5, 1.296), (2.0, 1.304),
            (2.5, 1.316), (3.0, 1.332), (3.5, 1.353), (4.025, 1.377), (4.629, 1.410), (5.323, 1.453), (6.122, 1.506), (7.040, 1.572),
            (8.096, 1.651), (9.310, 1.741), (10.0, 1.790),
        ];
        for (distance, height) in native {
            let got = pivot_height(bone, &keys, distance);
            assert!(near(got, height, 0.005), "at {distance} m: {got} against 1.2's {height}");
        }
        // past the last key the height holds (18 m, the classic mode's far limit): 1.194 + 0.8
        assert!(near(pivot_height(bone, &keys, 18.0), 1.994, 1e-4));
        assert_eq!(z_offset(&[], 3.0), 0.0);
    }

    #[test]
    fn each_race_starts_where_its_bone_and_cdf_put_the_camera() {
        // (prefix, the .cdf keys at 0 / 1 / 15 m, start distance, height at the start, at 1 m); the Nuian man's 1.734 at 4 m is
        // also the camera height of the spawn capture `reference/ingame/spawn/camera.json`
        let rows = [
            ("nu_m", [0.28, 0.2, 0.8], 4.0, 1.734, 1.663),
            ("nu_f", [0.25, 0.15, 0.8], 3.2, 1.539, 1.496),
            ("el_m", [0.32, 0.2, 0.8], 3.8, 1.725, 1.663),
            ("el_f", [0.27, 0.15, 0.8], 3.4, 1.615, 1.564),
            ("ha_m", [0.22, 0.15, 0.8], 3.6, 1.478, 1.419),
            ("ha_f", [0.20, 0.1, 0.8], 3.2, 1.340, 1.294),
            ("fe_m", [0.32, 0.2, 0.8], 4.0, 1.750, 1.679),
            ("fe_f", [0.15, 0.08, 0.8], 3.4, 1.548, 1.492),
        ];
        for (prefix, h, start, at_start, at_one) in rows {
            let keys = [(0.0, h[0]), (1.0, h[1]), (15.0, h[2])];
            let bone = bind_camera_bone_height(prefix).unwrap();
            assert!(near(pivot_height(bone, &keys, start), at_start, 2e-3), "{prefix} at {start} m");
            assert!(near(pivot_height(bone, &keys, 1.0), at_one, 2e-3), "{prefix} at 1 m");
        }
        assert_eq!(bind_camera_bone_height("dw_m"), None);
    }

    #[test]
    fn the_pitch_is_held_between_88_down_and_88_up_and_the_arm_stops_at_50() {
        // the native drag held at -88 (the arm), and at the top the arm at 50 with the view 38 more
        let deg = |d: f32| d.to_radians();
        assert_eq!(split_pitch(deg(-100.0)), (PITCH_DOWN, 0.0));
        assert_eq!(split_pitch(deg(-30.0)), (deg(-30.0), 0.0));
        assert_eq!(split_pitch(deg(50.0)), (PITCH_ARM_UP, 0.0));
        let (arm, turn) = split_pitch(deg(70.0));
        assert!(near(arm, deg(50.0), 1e-6) && near(turn, deg(20.0), 1e-6));
        let (arm, turn) = split_pitch(deg(120.0));
        assert!(near(arm, deg(50.0), 1e-6) && near(turn, deg(38.0), 1e-6));
    }
}
