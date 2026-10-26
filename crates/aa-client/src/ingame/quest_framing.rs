//! The quest talk's close-up as 1.2 frames it, in plain numbers (game axes: x east, y north, z up; a yaw is counter-clockwise from
//! north, so a unit at yaw `a` faces `(-sin a, cos a)` and has `(cos a, sin a)` on its right), no engine; `quest_camera.rs` reads the
//! speaker and puts the result on the Bevy camera.
//!
//! What 1.2 does (read in `x2game`, then measured in a running client: the camera object's mode, speaker, facing, offsets, field of
//! view and target height sampled read-only during two talks):
//!
//! * **When it is on** (`quest::try_context` x2game+0x389c70): the quest's `quest_contexts.use_quest_camera` is set and bit
//!   `detail_id - 1` of the console variable `quest_camera_use` (x2game+0xb0e2a0, default 1023, set to 1023 or 0 by the Options'
//!   close-up camera box) is set ([`close_up_on`]).
//! * **The row** (`quest_camera::frame_speaker` x2game+0x17b440, for each quest bubble): the bubble's `camera_id`, else the row of the
//!   speaker's model in `model_quest_cameras`; row 17 ("previous camera") keeps the framing in hand; none leaves the close-up
//!   ([`pick_row`]).
//! * **The eye and the aim** (`quest_camera::update` x2game+0x17a260, every frame, with P the speaker's feet and q its facing): the eye
//!   at `P + q·camera_offset + (0, 0, 1.5)`, the aim at `P + q·npc_offset + (0, 0, h)`, the view from one to the other with yaw and
//!   pitch only ([`view`], [`look_angles`]). h is the speaker's `camera` bone (fixed bone slot 45) at its **bind-pose** height, 1.5 for a
//!   model without a skeleton ([`aim_height`]): in the run h read 1.1936 for a Hariharan woman and 1.2686 for a Hariharan man in every
//!   frame of their talks, the bind heights of their skeletons.
//! * **The lens**: the row's `fov` (30 degrees on every row) as the vertical field, with no aspect factor.
//! * **The move**: a cut at the first line and at each change of speaker (`interpolate` is false on every row); a cut back to the
//!   follow camera, as it was left, at the end.
//! * **Hiding**: the player and its mount are hidden while a row with `invisible` frames (all 51 rows).

/// `cvar::quest_camera_use`'s default (x2game+0xad44c0): one bit for each `quest_contexts.detail_id` 1 to 10. `hud/quest.lua` holds
/// the option and passes it in.
#[cfg_attr(not(test), allow(dead_code))]
pub const OPTION_ON: u32 = 1023;

/// `camera::tw_quest_eye_height` (x2game+0xb0e2a4): how far above the speaker's feet the eye stands.
pub const EYE_HEIGHT: f32 = 1.5;

/// The aim's height over the feet for a model with no skeleton to read (`quest_camera::frame_speaker`: 1.5 for other models).
pub const OTHER_MODEL_HEIGHT: f32 = 1.5;

/// The row `quest_cameras.id` 17, whose comment is "previous camera": a bubble that names it keeps the framing it had.
pub const PREVIOUS_CAMERA_ROW: u32 = 17;

/// The height over the speaker's feet the aim is taken from (h, camera +0x1e4): the `camera` bone at its bind-pose height, `bind`
/// `None` for a model without a skeleton (1.5). Not the bone as the clips leave it: no clip has a track for `camera`, so in an animated
/// pose it rides on `Bip01`, which the clips keep near the ground, about 0.85 m under the bind height.
pub fn aim_height(bind: Option<f32>) -> f32 {
    bind.unwrap_or(OTHER_MODEL_HEIGHT)
}

/// Whether the close-up is on for a quest: its `use_quest_camera` is set and the bit `detail_id - 1` of the option's value is set.
/// A `detail_id` of 0 or past the 32 bits never gets it.
pub fn close_up_on(use_quest_camera: bool, detail_id: u32, option: u32) -> bool {
    use_quest_camera && (1..=32).contains(&detail_id) && option & (1u32 << (detail_id - 1)) != 0
}

/// The values of a `quest_cameras` row that frame a line.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Row {
    pub id: u32,
    /// The vertical field of view, degrees (30 on every row).
    pub fov: f32,
    /// `camera_offset`: x to the speaker's right, y in front of it, z up (the rows hold `(0, y, 0)`).
    pub camera: [f32; 3],
    /// `npc_offset`: the same axes, from the speaker's place (and its bone's height) to the point looked at.
    pub aim: [f32; 3],
    /// Eased over a second from the old view when set, else a cut (false on every row).
    pub interpolate: bool,
    /// The player and its mount are hidden while the row frames (true on every row).
    pub invisible: bool,
    /// The renderer blurs what lies behind while the row frames (`dof`: false only on the eight ghost rows 39 to 44, 50 and 51).
    pub dof: bool,
}

/// The eye and the point looked at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub eye: [f32; 3],
    pub target: [f32; 3],
}

/// The unit's frame: its right, forward and up on the world axes.
fn frame(yaw: f32) -> ([f32; 3], [f32; 3], [f32; 3]) {
    ([yaw.cos(), yaw.sin(), 0.0], [-yaw.sin(), yaw.cos(), 0.0], [0.0, 0.0, 1.0])
}

fn along(at: [f32; 3], yaw: f32, offset: [f32; 3]) -> [f32; 3] {
    let (r, f, u) = frame(yaw);
    std::array::from_fn(|i| at[i] + r[i] * offset[0] + f[i] * offset[1] + u[i] * offset[2])
}

/// The view of a row for a speaker at `at` (the feet) facing `yaw`, whose aim height is `aim_height`: the eye at
/// `at + q·camera_offset + (0, 0, 1.5)`, the target at `at + q·npc_offset + (0, 0, aim_height)`.
pub fn view(row: &Row, at: [f32; 3], yaw: f32, aim_height: f32) -> View {
    let mut eye = along(at, yaw, row.camera);
    eye[2] += EYE_HEIGHT;
    let mut target = along(at, yaw, row.aim);
    target[2] += aim_height;
    View { eye, target }
}

/// The yaw and the pitch (radians; yaw counter-clockwise from north, pitch up) of the look from the eye to the target: no roll.
pub fn look_angles(v: &View) -> (f32, f32) {
    let d = [v.target[0] - v.eye[0], v.target[1] - v.eye[1], v.target[2] - v.eye[2]];
    let flat = d[0].hypot(d[1]);
    (f32::atan2(-d[0], d[1]), f32::atan2(d[2], flat))
}

/// Which `quest_cameras` row frames a bubble: the bubble's own `camera_id` when it has one, else its model's row; the "previous
/// camera" row keeps `previous` (the row framing now), or the model's row when nothing frames yet. `None` leaves the close-up.
pub fn pick_row(camera_id: u32, model_row: Option<u32>, previous: Option<u32>) -> Option<u32> {
    match camera_id {
        0 => model_row,
        PREVIOUS_CAMERA_ROW => previous.or(model_row),
        id => Some(id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where a point falls across the screen, in pixels, for the view's camera with a vertical `fov` over a `w` x `h` screen.
    fn screen_x(v: &View, point: [f32; 3], fov: f32, w: f32, h: f32) -> f32 {
        let (yaw, pitch) = look_angles(v);
        let forward = [-yaw.sin() * pitch.cos(), yaw.cos() * pitch.cos(), pitch.sin()];
        let right = [yaw.cos(), yaw.sin(), 0.0];
        let d: Vec<f32> = (0..3).map(|i| point[i] - v.eye[i]).collect();
        let depth: f32 = (0..3).map(|i| d[i] * forward[i]).sum();
        let side: f32 = (0..3).map(|i| d[i] * right[i]).sum();
        let focal = (h / 2.0) / (fov.to_radians() / 2.0).tan();
        w / 2.0 + side / depth * focal
    }

    /// The share of the screen's height from its top at which a point falls (0 the top edge, 1 the bottom).
    fn screen_y(v: &View, point: [f32; 3], fov: f32) -> f32 {
        let (yaw, pitch) = look_angles(v);
        let forward = [-yaw.sin() * pitch.cos(), yaw.cos() * pitch.cos(), pitch.sin()];
        let up = [yaw.sin() * pitch.sin(), -yaw.cos() * pitch.sin(), pitch.cos()];
        let d: Vec<f32> = (0..3).map(|i| point[i] - v.eye[i]).collect();
        let depth: f32 = (0..3).map(|i| d[i] * forward[i]).sum();
        let rise: f32 = (0..3).map(|i| d[i] * up[i]).sum();
        0.5 - 0.5 * rise / depth / (fov.to_radians() / 2.0).tan()
    }

    /// Row 7 `hariharan_male_base` (model 18): camera (0, 2.2, 0), aim (-0.4, 0, 0.05), 30 degrees. Camp Administrator Hassan
    /// (NPC 2472), quest 1113.
    const HASSAN: Row = Row { id: 7, fov: 30.0, camera: [0.0, 2.2, 0.0], aim: [-0.4, 0.0, 0.05], interpolate: false, invisible: true, dof: true };
    /// The Hariharan man's `camera` bone in the bind pose (`ha_m_base.chr`).
    const HA_M_CAMERA: f32 = 1.269;

    /// Row 8 `hariharan_female_base` (model 19): Yuelli (NPC 7682), quest 3509 "Shady Dealings".
    const YUELLI: Row = Row { id: 8, fov: 30.0, camera: [0.0, 2.2, 0.0], aim: [-0.4, 0.0, 0.05], interpolate: false, invisible: true, dof: true };
    /// The Hariharan woman's bind pose (`ha_f_base.chr`): the `camera` bone, `Bip01` (the parent the undriven bone rides on in an
    /// animated pose), `Bip01 Head`, `Bip01 Pelvis`.
    const HA_F_CAMERA: f32 = 1.194;
    const HA_F_BIP01: f32 = 0.855;
    const HA_F_HEAD: f32 = 1.399;
    const HA_F_PELVIS: f32 = 0.855;

    #[test]
    fn yuelli_is_framed_head_to_waist_as_in_1_2() {
        let (at, yaw) = ([19981.95, 7576.92, 179.49], -0.39);
        let v = view(&YUELLI, at, yaw, aim_height(Some(HA_F_CAMERA)));
        let share = |z: f32| screen_y(&v, [at[0], at[1], at[2] + z], YUELLI.fov);
        // 1.2's frame at 1080 p: her mouth, where the head bone sits, at y 408 (37.8 %), the pelvis low in the frame, the feet below it
        assert!((share(HA_F_HEAD) - 408.0 / 1080.0).abs() < 0.03, "head at {}", share(HA_F_HEAD));
        assert!(share(HA_F_PELVIS) > 0.75 && share(HA_F_PELVIS) < 1.0, "pelvis at {}", share(HA_F_PELVIS));
        assert!(share(0.0) > 1.0, "feet at {}", share(0.0));
    }

    #[test]
    fn the_bone_as_the_clips_leave_it_frames_the_feet() {
        // the undriven `camera` bone riding on `Bip01` at the ground stands 0.339 m over the feet: aimed there, the feet fall at 77 % of
        // the height and the head is off the top of the frame; `aim_height` takes the bind height instead
        let low = view(&YUELLI, [0.0; 3], 0.0, HA_F_CAMERA - HA_F_BIP01);
        assert!((screen_y(&low, [0.0, 0.0, 0.0], 30.0) - 0.77).abs() < 0.02);
        assert!(screen_y(&low, [0.0, 0.0, HA_F_HEAD], 30.0) < 0.0);
        assert_eq!(aim_height(Some(HA_F_CAMERA)), HA_F_CAMERA);
        assert_eq!(aim_height(None), OTHER_MODEL_HEIGHT);
    }

    #[test]
    fn the_close_up_needs_the_quest_and_the_options_bit() {
        // quest 1113 is detail 1 with `use_quest_camera`
        assert!(close_up_on(true, 1, OPTION_ON));
        assert!(close_up_on(true, 10, OPTION_ON));
        // detail 12 (one quest) never gets it: its bit is past the 1023
        assert!(!close_up_on(true, 12, OPTION_ON));
        assert!(!close_up_on(false, 1, OPTION_ON));
        // the Options' box cleared: 0
        assert!(!close_up_on(true, 1, 0));
        assert!(!close_up_on(true, 0, OPTION_ON));
        assert!(!close_up_on(true, 40, u32::MAX));
    }

    #[test]
    fn hassan_is_framed_from_2_2_m_in_front_and_1_5_m_up_looking_past_the_left_shoulder() {
        // facing north at the origin: in front of the speaker is +y
        let v = view(&HASSAN, [0.0; 3], 0.0, HA_M_CAMERA);
        assert_eq!(v.eye, [0.0, 2.2, 1.5]);
        assert!((v.target[0] + 0.4).abs() < 1e-6 && v.target[1].abs() < 1e-6 && (v.target[2] - 1.319).abs() < 1e-5, "{:?}", v.target);
        // turned left a quarter: in front of the speaker is -x, and the speaker's left (-0.4 to the right) is south
        let v = view(&HASSAN, [10.0, 20.0, 5.0], std::f32::consts::FRAC_PI_2, HA_M_CAMERA);
        assert!((v.eye[0] - 7.8).abs() < 1e-5 && (v.eye[1] - 20.0).abs() < 1e-5 && (v.eye[2] - 6.5).abs() < 1e-5, "{:?}", v.eye);
        assert!((v.target[0] - 10.0).abs() < 1e-5 && (v.target[1] - 19.6).abs() < 1e-5, "{:?}", v.target);
    }

    #[test]
    fn hassan_stands_366_px_left_of_the_centre_at_1080p() {
        // 1.2's frame of quest 1113 has Hassan's face at x 590 of 1920; the rule gives 2015 x 0.4 / 2.2 = 366 px left of 960
        let v = view(&HASSAN, [100.0, 200.0, 10.0], 0.7, HA_M_CAMERA);
        let x = screen_x(&v, [100.0, 200.0, 10.0 + HA_M_CAMERA], HASSAN.fov, 1920.0, 1080.0);
        assert!((960.0 - x - 366.0).abs() < 6.0, "the speaker's axis falls at {x}");
        // the aim is the screen's centre
        let centre = screen_x(&v, v.target, HASSAN.fov, 1920.0, 1080.0);
        assert!((centre - 960.0).abs() < 0.01, "{centre}");
    }

    #[test]
    fn the_look_has_yaw_and_pitch_and_the_target_is_dead_ahead() {
        let v = view(&HASSAN, [0.0; 3], 0.0, HA_M_CAMERA);
        let (yaw, pitch) = look_angles(&v);
        // from (0, 2.2, 1.5) to (-0.4, 0, 1.319): south and a little to the west, a little down
        assert!((yaw - 2.9604).abs() < 0.01, "{yaw}");
        assert!(pitch < 0.0 && pitch > -0.1, "{pitch}");
    }

    #[test]
    fn a_bubble_names_its_row_or_the_models() {
        assert_eq!(pick_row(0, Some(7), None), Some(7));
        assert_eq!(pick_row(0, None, Some(3)), None);
        assert_eq!(pick_row(12, Some(7), Some(3)), Some(12));
        assert_eq!(pick_row(PREVIOUS_CAMERA_ROW, Some(7), Some(3)), Some(3));
        assert_eq!(pick_row(PREVIOUS_CAMERA_ROW, Some(7), None), Some(7));
    }
}
