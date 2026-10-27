//! The depth of field as 1.2's renderer draws it, in plain numbers (no engine): the time of day's blur at rest, the focus the game asks
//! for in a quest talk, how the renderer eases to it, and how a pixel is blended with the blurred copy of the scene. `sky.rs` feeds it the
//! time of day, the application the talk (`aa-client`'s `ingame/quest_depth_of_field.rs`), and `post_dof.rs` with `post.wesl` draw it.
//!
//! What 1.2 does (read in `x2game` and `cryrenderd3d9`, the frame from a D3D11 capture of open land by day):
//!
//! * **At rest** the time of day hands the renderer `Dof: focus range` (default 1000) and `Dof: blur amount` (default 0) each update
//!   (`TimeOfDayManager::UpdateEngine` cry3dengine+0xa5dd0); with `r_DepthOfField` 2 (the Options' box on) and no user focus the effect's
//!   `focus_params` (cryrenderd3d9+0x12f5d0) takes focus distance 0, range 2 x the focus range and that blur ([`rest_focus`]). 114 of the
//!   121 zones of the main world set a blur at 12:30 (mean 0.17, the most 0.30).
//! * **In a quest talk** the game (`camera::set_user_dof` x2game+0x16a9e0, from `quest_camera::frame_speaker`) sets the user values:
//!   distance `d = camera_offset_y` of the row, range `2.5 d` (`camera::tw_quest_dof_range_scale`), blur 1 (`camera::tw_quest_dof_blur`);
//!   the eight ghost rows have no `dof` ([`asked`], [`Focus::of_row`]).
//! * **The easing**: `focus_params` moves its own copy of the user values towards them by `min(3 x the frame time, 1)` of the way **every
//!   frame, whether the user focus is on or not**; the user values start at 3.5 m, 5 m, 1 and nothing resets them when a talk ends. So the
//!   copy has long settled when a talk starts and the blur is full from its first frame; only distance and range glide from the last
//!   talk's row to this one's ([`Eased`]). With the user focus off it draws the time of day's at once.
//! * **The frame**: the lit HDR scene is box-filtered to half resolution, the merged pass blurs that copy into a second half resolution
//!   target, and the final composite (`HDRToneMapSample`) blends each full resolution pixel towards the bilinear copy by the pixel's own
//!   depth: `lerp(scene, copy, saturate(far + near))` with far = `saturate((z - d) / (r / 2))` and near = `saturate(2 b saturate((d - z)
//!   / (r / 2)))`, nothing at all when `2 b < 0.1` ([`Focus::composite_weight`]). One blurred image blended in: no per-pixel kernel.
//! * **How far the copy is blurred**: at rest, the merged pass's ring (8 bilinear taps at `min(4 b, 2)` half resolution texels, the centre
//!   weighted 1/3; [`rest_sigma_texels`]). Behind a talk 1.2 on D3D11 leaves that pass for NVIDIA's `Nv_Dof` path (every row but the ghost
//!   rows sets `nv_dof`), whose kernel is not read: its width here is a fit to a native 1080 p screenshot ([`TALK_SIGMA_PX`]).

/// `camera::tw_quest_dof_range_scale` (x2game+0xb0e2ac): the focus range is this times the row's `camera_offset_y`.
pub const RANGE_SCALE: f32 = 2.5;

/// `camera::tw_quest_dof_blur` (x2game+0xb0e2b0): the blur amount the game sets.
pub const BLUR_AMOUNT: f32 = 1.0;

/// The merged pass's ring is at most this many half resolution texels wide (`min(4 b (near + far), 2)`).
pub const MAX_BLUR: f32 = 2.0;

/// The renderer moves each focus value this many times the frame time of the way to its target in a frame (`min(3 x dt, 1)`).
pub const EASE_RATE: f32 = 3.0;

/// The renderer's user values before any talk sets them (`Dof_User_FocusDistance` 3.5, `Dof_User_FocusRange` 5,
/// `Dof_User_BlurAmount` 1); its eased copy settles on them in the first seconds of the game.
pub const USER_DEFAULT: Focus = Focus { distance: 3.5, range: 5.0, blur: 1.0 };

/// The focus distance of the time of day's blur: everything from the eye on is "far".
pub const REST_DISTANCE: f32 = 0.0;

/// The range is this times `Dof: focus range` (`Dof_Tod_FocusRange`) at `r_DepthOfField` 2.
pub const REST_RANGE_SCALE: f32 = 2.0;

/// Below this blur amount the composite draws nothing (`HDRDofParams` is zeroed when `2 b < 0.1`).
pub const MIN_BLUR: f32 = 0.05;

/// The standard deviation, in pixels of a 1080 pixel high picture, of the blur behind a quest talk. A fit, not read: the D3D11
/// `Nv_Dof` kernel is not read. A native 1080 p screenshot of a talk has the background's sharpest edges (a dark window frame against a
/// lit wall, 5 m or more behind the speaker) rising over 20 to 22 pixels (10 to 90 per cent, linear light); a gaussian turns an edge in
/// 2.56 sigma: 8 pixels.
pub const TALK_SIGMA_PX: f32 = 8.0;

/// The merged pass's colour: the centre at weight 1/3 and 8 taps on a ring of `blur` texels; its spread along an axis is this times
/// the ring's radius (`sqrt(8 / (8 + 1/3) / 2)`).
pub const RING_SIGMA_PER_RADIUS: f32 = 0.692_820_3;

/// What the renderer draws at rest, from the time of day's `Dof: blur amount` and `Dof: focus range`: focus 0, range 2 x the focus
/// range, blur b, when the depth of field is on and the blur is one the composite draws. `None` otherwise.
pub fn rest_focus(blur_amount: f32, focus_range: f32, on: bool) -> Option<Focus> {
    // a range of 0 (or less) would take the whole picture, the player too, into the blurred copy
    if !on || !blur_amount.is_finite() || !focus_range.is_finite() || focus_range <= 0.0 {
        return None;
    }
    let focus = Focus { distance: REST_DISTANCE, range: focus_range.max(0.0) * REST_RANGE_SCALE, blur: blur_amount };
    (!focus.is_off()).then_some(focus)
}

/// What the game asks of the renderer for the framing in hand: the row's focus when the close-up holds, the row has a `dof` (not the
/// eight ghost rows), the depth of field is on and the row has a distance to focus at (a `camera_offset_y` of 0 would be a range of 0);
/// `None` otherwise.
pub fn asked(close_up: bool, row_dof: bool, on: bool, camera_y: f32) -> Option<Focus> {
    (close_up && row_dof && on && camera_y > 0.0).then(|| Focus::of_row(camera_y))
}

/// The blur of the half resolution copy at rest, as a gaussian's standard deviation in half resolution texels: the merged pass's ring at
/// its widest (`min(4 b, 2)` texels; its lens offsets are one texel of the half resolution target at any picture size).
pub fn rest_sigma_texels(blur: f32) -> f32 {
    RING_SIGMA_PER_RADIUS * (4.0 * blur.max(0.0)).min(MAX_BLUR)
}

/// The blur of the half resolution copy behind a quest talk, in half resolution texels, for a picture `height` pixels high
/// ([`TALK_SIGMA_PX`] at 1080 p, kept in proportion at any size).
pub fn talk_sigma_texels(height: f32) -> f32 {
    TALK_SIGMA_PX * 0.5 * (height / 1080.0).clamp(0.25, 8.0)
}

/// The focus of a frame: the renderer's focus distance, focus range and blur amount in effect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Focus {
    /// Metres from the eye, along the view, that are sharp.
    pub distance: f32,
    /// The range: the composite takes all of the blurred copy at half of it beyond the distance.
    pub range: f32,
    /// The blur amount `b`.
    pub blur: f32,
}

impl Focus {
    /// What the game asks for a `quest_cameras` row whose `camera_offset_y` is `camera_y`.
    pub fn of_row(camera_y: f32) -> Focus {
        Focus { distance: camera_y, range: camera_y * RANGE_SCALE, blur: BLUR_AMOUNT }
    }

    /// One frame of the renderer's easing from `self` towards `target`: each value moves `min(3 dt, 1)` of the way.
    pub fn step(self, target: Focus, dt: f32) -> Focus {
        let k = (EASE_RATE * dt.max(0.0)).min(1.0);
        let towards = |from: f32, to: f32| from + (to - from) * k;
        Focus { distance: towards(self.distance, target.distance), range: towards(self.range, target.range), blur: towards(self.blur, target.blur) }
    }

    /// Whether the composite draws nothing (`2 b < 0.1`).
    pub fn is_off(&self) -> bool {
        !(self.blur >= MIN_BLUR)
    }

    /// How much of the blurred copy a pixel whose linear depth is `z` metres takes in the final composite: the far side by the pixel's own
    /// full resolution depth, `saturate((z - d) / (r / 2))`, the near side `saturate(2 b saturate((d - z) / (r / 2)))`, their sum held to
    /// 1; 0 when the blur is off.
    pub fn composite_weight(&self, z: f32) -> f32 {
        if self.is_off() {
            return 0.0;
        }
        let half = (self.range * 0.5).max(1e-3);
        let far = ((z - self.distance) / half).clamp(0.0, 1.0);
        let near = (2.0 * self.blur * ((self.distance - z) / half).clamp(0.0, 1.0)).clamp(0.0, 1.0);
        (far + near).min(1.0)
    }
}

/// The renderer's eased copy of the user focus, kept between frames and between talks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Eased {
    /// The copy `focus_params` eases each frame.
    focus: Focus,
    /// The user values last set; a talk's end clears only the switch, not these.
    user: Focus,
}

impl Default for Eased {
    /// Settled on the renderer's defaults: the copy has had the whole login to ease to them.
    fn default() -> Self {
        Eased { focus: USER_DEFAULT, user: USER_DEFAULT }
    }
}

impl Eased {
    /// One frame: `ask` is the focus the game sets ([`Focus::of_row`]) or `None` when it sets none (no talk, a ghost row, the depth of
    /// field off); `dt` is the frame time. The copy moves towards the last user values every frame, asked or not. Returns the focus the
    /// talk draws, `None` while it asks for none (the time of day's is drawn at once).
    pub fn frame(&mut self, ask: Option<Focus>, dt: f32) -> Option<Focus> {
        if let Some(target) = ask {
            self.user = target;
        }
        self.focus = self.focus.step(self.user, dt);
        ask.map(|_| self.focus).filter(|f| !f.is_off())
    }
}

/// The post passes' four numbers for a focus and the blur of the copy (`sigma` in half resolution texels): x the blur's sigma (0 = off:
/// no pass runs and the composite takes no sample), y the distance, z half the range, w the blur amount.
pub fn shader_params(focus: Option<Focus>, sigma: f32) -> [f32; 4] {
    match focus {
        Some(f) if !f.is_off() => [sigma.max(1e-3), f.distance, f.range * 0.5, f.blur],
        _ => [0.0; 4],
    }
}

/// The weights of the separable gaussian the copy is blurred with, as `post.wesl`'s `dof_gauss` takes them: pairs of neighbouring texels
/// merged into one bilinear tap (offset in texels, weight), each side; the centre's weight is the first entry's, at offset 0. The weights
/// sum to 1 over the centre and both sides. Reach: three sigma, at most 24 texels.
pub fn gaussian_taps(sigma: f32) -> Vec<(f32, f32)> {
    let sigma = sigma.max(1e-3);
    let reach = (3.0 * sigma).ceil().clamp(1.0, 24.0) as i32;
    let g = |k: i32| (-((k * k) as f32) / (2.0 * sigma * sigma)).exp();
    let mut taps = vec![(0.0, g(0))];
    let mut k = 1;
    while k <= reach {
        let (a, b) = (g(k), if k < reach { g(k + 1) } else { 0.0 });
        let w = a + b;
        taps.push(((k as f32 * a + (k + 1) as f32 * b) / w, w));
        k += 2;
    }
    let total = taps[0].1 + 2.0 * taps[1..].iter().map(|t| t.1).sum::<f32>();
    taps.iter().map(|&(o, w)| (o, w / total)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Row 7 (`hariharan_male_base`, Camp Administrator Hassan): `camera_offset_y` 2.2.
    const HASSAN_Y: f32 = 2.2;

    #[test]
    fn the_game_asks_distance_y_range_two_and_a_half_y_blur_one() {
        let f = Focus::of_row(HASSAN_Y);
        assert_eq!((f.distance, f.blur), (2.2, 1.0));
        assert!((f.range - 5.5).abs() < 1e-6);
    }

    #[test]
    fn the_composite_takes_the_copy_by_the_pixels_own_depth() {
        // HDRDofParams (2/r, -2d/r): for Hassan sharp at 2.2 m, all of the copy from 4.95 m on, half of it at 3.575 m
        let f = Focus::of_row(HASSAN_Y);
        assert_eq!(f.composite_weight(2.2), 0.0, "the speaker keeps its own pixels");
        assert!((f.composite_weight(2.2 + 1.375) - 0.5).abs() < 1e-5);
        assert!(f.composite_weight(2.2 + 2.75) > 0.9999 && f.composite_weight(5.0) == 1.0);
        assert_eq!(f.composite_weight(1.0e6), 1.0, "the sky");
        // the near side is the merged pass's alpha, 2 b times the ramp: all of the copy a quarter range in front
        assert!((f.composite_weight(2.2 - 0.6875) - 0.5).abs() < 1e-5);
        assert_eq!(f.composite_weight(2.2 - 1.375), 1.0);
    }

    #[test]
    fn at_rest_the_composite_blends_a_tenth_of_the_copy_at_100_m() {
        // the open-land capture: HDRDofParams 0.001, 0 (range 2000), vDofParamsFocus1.w 0.479 (2 b)
        let f = rest_focus(0.2396, 1000.0, true).expect("a blur");
        assert!((f.composite_weight(100.0) - 0.1).abs() < 1e-5);
        assert!((f.composite_weight(500.0) - 0.5).abs() < 1e-5);
        assert_eq!(f.composite_weight(1500.0), 1.0);
        assert!(f.composite_weight(0.5) < 1e-3, "the ground at the feet stays sharp");
        // the ring at its widest: 4 b = 0.96 texel, a spread of 0.66 texel
        assert!((rest_sigma_texels(0.2396) - 0.664).abs() < 0.002);
        assert_eq!(rest_sigma_texels(5.0), RING_SIGMA_PER_RADIUS * MAX_BLUR);
    }

    #[test]
    fn a_blur_under_a_twentieth_draws_nothing() {
        // HDRToneMapSample's constants are zeroed when 2 b < 0.1
        let faint = Focus { blur: 0.04, ..Focus::of_row(HASSAN_Y) };
        assert_eq!(faint.composite_weight(100.0), 0.0);
        assert_eq!(rest_focus(0.04, 1000.0, true), None);
        assert_eq!(shader_params(Some(faint), 4.0), [0.0; 4]);
        assert!(rest_focus(0.05, 1000.0, true).is_some());
    }

    #[test]
    fn the_close_up_blurs_only_for_a_row_with_dof_and_the_switch_on() {
        assert_eq!(asked(true, true, true, HASSAN_Y), Some(Focus::of_row(HASSAN_Y)));
        assert_eq!(asked(true, false, true, 2.3), None, "a ghost row");
        assert_eq!(asked(true, true, false, HASSAN_Y), None, "the depth of field off");
        assert_eq!(asked(false, true, true, HASSAN_Y), None, "no close-up");
        assert_eq!(asked(true, true, true, 0.0), None, "no distance to focus at");
    }

    #[test]
    fn a_talk_blurs_from_its_first_frame() {
        // the copy has eased to the renderer's defaults (3.5 m, 5 m, 1) long before: the first frame of a talk is already a full blur
        // behind the speaker, not a ramp from the time of day's 2000 m
        let mut e = Eased::default();
        let first = e.frame(Some(Focus::of_row(HASSAN_Y)), 1.0 / 60.0).expect("on at once");
        assert!((first.blur - 1.0).abs() < 1e-6);
        assert!(first.range < 5.1 && first.distance > 3.4, "{first:?}");
        assert_eq!(first.composite_weight(10.0), 1.0, "the wall 10 m behind is all copy on the first frame");
        // distance and range glide to the row's: a second later within 5 per cent
        let mut f = first;
        for _ in 1..60 {
            f = e.frame(Some(Focus::of_row(HASSAN_Y)), 1.0 / 60.0).expect("on");
        }
        assert!((f.distance - 2.2).abs() < 0.07 && (f.range - 5.5).abs() < 0.03, "{f:?}");
    }

    #[test]
    fn a_talk_ends_at_once_and_the_next_starts_where_the_last_left() {
        let mut e = Eased::default();
        for _ in 0..120 {
            e.frame(Some(Focus::of_row(HASSAN_Y)), 1.0 / 60.0);
        }
        // the talk ends: nothing drawn from this frame on (the time of day's at once), the user values stay
        assert_eq!(e.frame(None, 1.0 / 60.0), None);
        for _ in 0..600 {
            e.frame(None, 1.0 / 60.0);
        }
        // the next talk with the same row: settled from its first frame
        let again = e.frame(Some(Focus::of_row(HASSAN_Y)), 1.0 / 60.0).expect("on");
        assert!((again.distance - 2.2).abs() < 1e-3 && (again.range - 5.5).abs() < 1e-3);
    }

    #[test]
    fn a_long_frame_jumps_to_the_target() {
        let f = USER_DEFAULT.step(Focus::of_row(HASSAN_Y), 0.5);
        assert_eq!(f, Focus::of_row(HASSAN_Y), "min(3 x 0.5, 1) is the whole way");
    }

    /// Arcum Iris (zone 187) at 12:30: `Dof: blur amount` 0.237, `Dof: focus range` 1000.
    const ARCUM_BLUR: f32 = 0.237;

    #[test]
    fn at_rest_the_time_of_day_blurs_from_the_eye_on_over_twice_its_focus_range() {
        let f = rest_focus(ARCUM_BLUR, 1000.0, true).expect("114 of the 121 zones set a blur");
        assert_eq!((f.distance, f.range, f.blur), (0.0, 2000.0, ARCUM_BLUR));
        assert_eq!(rest_focus(ARCUM_BLUR, 1000.0, false), None, "r_DepthOfField 1");
        assert_eq!(rest_focus(0.0, 1000.0, true), None, "7 of the 121 zones set none");
        assert_eq!(rest_focus(f32::NAN, 1000.0, true), None);
    }

    #[test]
    fn the_pass_numbers_carry_the_sigma_and_are_off_with_no_focus() {
        assert_eq!(shader_params(None, 4.0), [0.0; 4]);
        let p = shader_params(Some(Focus::of_row(HASSAN_Y)), talk_sigma_texels(1080.0));
        assert_eq!(p, [4.0, 2.2, 2.75, 1.0], "8 px at 1080 p is 4 half resolution texels");
        assert_eq!(talk_sigma_texels(2160.0), 8.0);
    }

    #[test]
    fn the_gaussian_taps_sum_to_one_and_merge_pairs() {
        for sigma in [0.3, 0.66, 1.0, 4.0, 8.0, 40.0] {
            let taps = gaussian_taps(sigma);
            let total = taps[0].1 + 2.0 * taps[1..].iter().map(|t| t.1).sum::<f32>();
            assert!((total - 1.0).abs() < 1e-5, "sigma {sigma}: {total}");
            assert!(taps.len() <= 13, "at most 12 bilinear taps a side: {}", taps.len());
            // the spread is the gaussian's
            let var: f32 = 2.0 * taps[1..].iter().map(|&(o, w)| w * o * o).sum::<f32>();
            if (0.66..=8.0).contains(&sigma) {
                assert!((var.sqrt() - sigma).abs() < 0.12 * sigma, "sigma {sigma}: {}", var.sqrt());
            }
        }
        assert!(gaussian_taps(4.0)[1].0 > 1.0, "the first merged tap lies between texels 1 and 2");
    }
}
