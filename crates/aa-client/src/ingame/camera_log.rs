//! Dev: `AA_CAMERA_LOG=1` writes one CSV line a frame of the follow camera to `camera-log-<seconds>.csv` in the working folder (any
//! other value but `0` is the file's path): the feet, the camera's place, the point followed, the view's forward, the distances,
//! the lift and the lens, in the columns `tools/camera_probe.py` writes for the running 1.2 client, so `tools/camera_compare.py`
//! sets the two side by side per zoom step. It also places the drawn body (head, pelvis and lowest toe bone) as heights over the
//! feet and as shares of the view's height, which the native probe leaves empty (its frames show the body). While the quest
//! talk's close-up holds, the line is mode 3 and its feet and body are the speaker's.
//!
//! The line itself ([`CameraSample`], [`LOG_COLUMNS`], [`look_above_feet`]) is plain Rust; [`log_camera`] reads the frame as it is
//! drawn (it runs in `PostUpdate` after the transforms are propagated and the projection is made).

use aa_world::bevy_to_cry;
use bevy::prelude::*;

use super::camera::{CameraFollowed, OrbitCamera};
use super::player::{LocalPlayer, Mover};

/// The log's columns, the same names and order `tools/camera_probe.py` writes.
pub const LOG_COLUMNS: &str = "time,mode,feet_x,feet_y,feet_z,cam_x,cam_y,cam_z,target_x,target_y,target_z,fwd_x,fwd_y,fwd_z,yaw_deg,pitch_deg,\
distance,wanted,zoffset,fov_deg,cam_above_feet,target_above_feet,look_above_feet,arm_pitch_deg,view_extra_deg,arm_len,lag_x,lag_y,lag_z,feet_src,\
head_above_feet,pelvis_above_feet,toe_above_feet,head_screen,pelvis_screen,toe_screen";

/// Where the drawn body is: its head, pelvis and lowest toe bones as heights over the feet the camera uses (m), and where each lands
/// on the screen, as a share of the view's height from its top (0 the top edge, 1 the bottom; past 1 below the frame).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BodyOnScreen {
    pub head_above_feet: f32,
    pub pelvis_above_feet: f32,
    pub toe_above_feet: f32,
    pub head_screen: f32,
    pub pelvis_screen: f32,
    pub toe_screen: f32,
}

/// The share of the view's height from its top for a point at normalized device y (`-1` the bottom edge, `1` the top).
pub fn screen_share(ndc_y: f32) -> f32 {
    (1.0 - ndc_y) * 0.5
}

/// One frame of the follow camera for the log (game axes, metres, degrees).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CameraSample {
    /// Seconds since the log began.
    pub time: f32,
    /// 1 the follow camera; 3 the quest talk's close-up (`feet` and `body` are then the speaker's); 0 another pose (the combat chase).
    pub mode: i32,
    pub feet: [f32; 3],
    pub cam: [f32; 3],
    /// The point followed without the lift (1.2's T, camera +0x3c).
    pub target: [f32; 3],
    /// The view's forward, a unit vector.
    pub forward: [f32; 3],
    pub yaw_deg: f32,
    pub distance: f32,
    pub wanted: f32,
    /// The `.cdf` lift (1.2's camera +0x48).
    pub zoffset: f32,
    pub fov_deg: f32,
    /// The arm's length after the walls: the camera's distance from the target.
    pub arm_len: f32,
    /// The drawn body, when the player's model is at hand.
    pub body: Option<BodyOnScreen>,
}

/// Where the view's centre ray passes over the feet: the camera's height plus the forward's rise over the level distance from the
/// camera to the feet, as a height above the feet. `None` when the view looks straight up or down.
pub fn look_above_feet(cam: [f32; 3], forward: [f32; 3], feet: [f32; 3]) -> Option<f32> {
    let level = forward[0].hypot(forward[1]);
    if level < 1e-6 {
        return None;
    }
    let reach = (feet[0] - cam[0]).hypot(feet[1] - cam[1]);
    Some(cam[2] + forward[2] / level * reach - feet[2])
}

impl CameraSample {
    /// The sample as a line of [`LOG_COLUMNS`] (no line end); the native-only columns (the arm's own pitch, the view's turn past
    /// it, the lag) are empty, `feet_src` is `source`.
    pub fn csv(&self, source: &str) -> String {
        let f = |x: f32| format!("{x:.4}");
        let pitch = self.forward[2].clamp(-1.0, 1.0).asin().to_degrees();
        let look = look_above_feet(self.cam, self.forward, self.feet).map_or(String::new(), f);
        let mut v = vec![format!("{:.3}", self.time), self.mode.to_string()];
        v.extend(self.feet.iter().chain(&self.cam).chain(&self.target).chain(&self.forward).map(|x| f(*x)));
        v.extend([f(self.yaw_deg), f(pitch), f(self.distance), f(self.wanted), f(self.zoffset), f(self.fov_deg)]);
        v.extend([f(self.cam[2] - self.feet[2]), f(self.target[2] - self.feet[2]), look]);
        v.extend([String::new(), String::new(), f(self.arm_len), String::new(), String::new(), String::new(), source.to_string()]);
        match self.body {
            Some(b) => v.extend([b.head_above_feet, b.pelvis_above_feet, b.toe_above_feet, b.head_screen, b.pelvis_screen, b.toe_screen].map(f)),
            None => v.extend(std::iter::repeat_n(String::new(), 6)),
        }
        v.join(",")
    }
}

/// The log's file, opened on the first frame when `AA_CAMERA_LOG` is set.
#[derive(Default)]
pub struct CameraLog {
    checked: bool,
    out: Option<std::io::BufWriter<std::fs::File>>,
    start: f64,
    lines: u32,
}

impl CameraLog {
    fn open(&mut self, now: f64) -> Option<&mut std::io::BufWriter<std::fs::File>> {
        use std::io::Write;
        if !self.checked {
            self.checked = true;
            let path = match std::env::var("AA_CAMERA_LOG") {
                Ok(v) if v == "1" => {
                    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
                    format!("camera-log-{secs}.csv")
                }
                Ok(v) if !v.is_empty() && v != "0" => v,
                _ => return None,
            };
            match std::fs::File::create(&path) {
                Ok(file) => {
                    let mut out = std::io::BufWriter::new(file);
                    let _ = writeln!(out, "# aa-client {} camera log", env!("CARGO_PKG_VERSION"));
                    let _ = writeln!(out, "{LOG_COLUMNS}");
                    info!("camera log: {path}");
                    self.out = Some(out);
                    self.start = now;
                }
                Err(e) => warn!("camera log: cannot write {path}: {e}"),
            }
        }
        self.out.as_mut()
    }
}

/// Writes a line of the follow camera each frame while `AA_CAMERA_LOG` is set (system, `PostUpdate`).
#[allow(clippy::type_complexity)]
pub fn log_camera(
    time: Res<Time>,
    mut log: Local<CameraLog>,
    followed: Option<Res<CameraFollowed>>,
    player: Query<(Entity, &Mover), With<LocalPlayer>>,
    camera: Query<(&OrbitCamera, &GlobalTransform, &Projection, &Camera)>,
    models: Query<&aa_anim::Animator>,
    globals: Query<&GlobalTransform, Without<OrbitCamera>>,
) {
    use std::io::Write;
    let now = time.elapsed_secs_f64();
    if log.open(now).is_none() {
        return;
    }
    let (Ok((me, mover)), Ok((orbit, global, projection, view)), Some(followed)) = (player.single(), camera.single(), followed) else { return };
    // the close-up measures its speaker, the follow camera the player
    let (body_of, feet) = followed.speaker.unwrap_or((me, mover.pos));
    let transform = global.compute_transform();
    let cam = bevy_to_cry(transform.translation);
    // the drawn body: the bones' world places as they are drawn this frame
    let body = models.get(body_of).ok().and_then(|animator| {
        let bone = |names: &[&str]| -> Option<Vec3> {
            names
                .iter()
                .filter_map(|n| animator.rig.skeleton.bones.iter().position(|b| b.name.eq_ignore_ascii_case(n)))
                .filter_map(|i| globals.get(*animator.bones.get(i)?).ok().map(|g| g.translation()))
                .min_by(|a, b| a.y.total_cmp(&b.y))
        };
        let (head, pelvis, toe) = (bone(&["Bip01 Head"])?, bone(&["Bip01 Pelvis"])?, bone(&["Bip01 L Toe0Nub", "Bip01 R Toe0Nub", "Bip01 L Toe0", "Bip01 R Toe0"])?);
        let on_screen = |p: Vec3| view.world_to_ndc(global, p).map_or(f32::NAN, |ndc| screen_share(ndc.y));
        let above = |p: Vec3| bevy_to_cry(p).z - feet.z;
        Some(BodyOnScreen {
            head_above_feet: above(head),
            pelvis_above_feet: above(pelvis),
            toe_above_feet: above(toe),
            head_screen: on_screen(head),
            pelvis_screen: on_screen(pelvis),
            toe_screen: on_screen(toe),
        })
    });
    let fov_deg = match projection {
        Projection::Perspective(p) => p.fov.to_degrees(),
        _ => 0.0,
    };
    let sample = CameraSample {
        time: (now - log.start) as f32,
        mode: followed.mode,
        feet: feet.to_array(),
        cam: cam.to_array(),
        target: followed.target.to_array(),
        forward: bevy_to_cry(transform.rotation * Vec3::NEG_Z).to_array(),
        yaw_deg: orbit.yaw.to_degrees(),
        distance: orbit.distance,
        wanted: orbit.wanted,
        zoffset: followed.zoffset,
        fov_deg,
        arm_len: cam.distance(followed.target),
        body,
    };
    log.lines += 1;
    let flush = log.lines.is_multiple_of(30);
    if let Some(out) = log.out.as_mut() {
        let _ = writeln!(out, "{}", sample.csv("aa-client"));
        if flush {
            let _ = out.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_look_height_is_where_the_view_centre_passes_over_the_feet() {
        let feet = [10.0, 20.0, 5.0];
        // level, 4 m south of the feet at 1.34 m: the centre passes at the camera's own height
        assert!((look_above_feet([10.0, 16.0, 6.34], [0.0, 1.0, 0.0], feet).unwrap() - 1.34).abs() < 1e-5);
        // looking at a point 1.754 m above the feet from 3.2 m behind and 1 m over it: that point's height
        let cam = [10.0, 16.8, 5.0 + 1.754 + 1.0];
        let n = (3.2f32 * 3.2 + 1.0).sqrt();
        let fwd = [0.0, 3.2 / n, -1.0 / n];
        assert!((look_above_feet(cam, fwd, feet).unwrap() - 1.754).abs() < 1e-4);
        assert_eq!(look_above_feet(cam, [0.0, 0.0, -1.0], feet), None, "straight down");
        // a line has every column, with or without the body
        let line = CameraSample { forward: [0.0, 1.0, 0.0], ..CameraSample::default() }.csv("aa-client");
        assert_eq!(line.split(',').count(), LOG_COLUMNS.split(',').count());
        assert!(line.contains(",aa-client,"));
        let body = BodyOnScreen { head_screen: screen_share(0.2), ..BodyOnScreen::default() };
        let line = CameraSample { body: Some(body), ..CameraSample::default() }.csv("aa-client");
        assert_eq!(line.split(',').count(), LOG_COLUMNS.split(',').count());
        assert_eq!((screen_share(1.0), screen_share(-1.0), screen_share(0.0)), (0.0, 1.0, 0.5));
    }
}
