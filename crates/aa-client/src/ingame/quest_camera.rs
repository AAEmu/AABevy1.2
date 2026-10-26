//! The quest talk's close-up camera, as 1.2's `quest_camera` has it ([`super::quest_framing`] holds the rules and what was read).
//!
//! While a quest talk is on and its quest allows the close-up, each line frames its speaker from the speaker's `quest_cameras` row:
//! the eye in front of the speaker and 1.5 m up, the aim at its `camera` bone's bind-pose height plus the row's offset, the row's 30
//! degrees for the lens, a cut at each new speaker. The talk (`hud::dialog`) writes what it asks into [`QuestTalkCamera`]; [`drive`]
//! finds the speaker's unit and puts the camera's pose into [`QuestShot`], which `camera::follow_player` takes over the follow camera
//! (the orbit stays as it was left, so the cut back out is to the view the player had), `cam_fov::drive_cam_fov` takes the lens from
//! and `units::cull_units` the mount's hiding from.
//!
//! Not done: the eased move of a row with `interpolate` (no row has it). The facing the speaker has at its first line is kept until
//! the speaker changes, as 1.2 keeps its quaternion (+0x1b8).

use aa_anim::Animator;
use aa_world::cry_to_bevy;
use bevy::prelude::*;

use super::quest_framing as rule;
use super::units::RemoteUnit;

/// What the talk asks of the camera.
#[derive(Resource, Default, Debug)]
pub struct QuestTalkCamera {
    /// The close-up is on for the talk in hand (`quest_camera::set_active`).
    pub on: bool,
    /// The line being said frames this speaker; `None` leaves the follow camera (`quest_camera::leave`).
    pub frame: Option<Frame>,
}

/// One framing: the speaker (an object id), the row and the facing taken at the first frame of this speaker.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub speaker: u32,
    pub row: rule::Row,
    pub facing: Option<f32>,
}

/// What the close-up puts on the camera now.
#[derive(Resource, Default, Debug)]
pub struct QuestShot {
    /// The camera's pose; `None` while the follow camera is up.
    pub pose: Option<Transform>,
    /// The vertical field of view (degrees), the row's, with no aspect factor.
    pub fov: f32,
    /// The row hides the player and its mount (`invisible`).
    pub hide: bool,
    /// The speaker framed and its feet (game axes), and the point looked at: what the camera log measures in this mode.
    pub speaker: Option<(Entity, Vec3)>,
    pub aim: Vec3,
}

/// Run condition: no close-up holds the camera (the mouse and the wheel move the follow camera).
pub fn off(shot: Option<Res<QuestShot>>) -> bool {
    shot.is_none_or(|s| s.pose.is_none())
}

/// Finds the speaker and puts the pose of its row into [`QuestShot`] each frame.
pub fn drive(
    mut talk: ResMut<QuestTalkCamera>,
    mut shot: ResMut<QuestShot>,
    units: Query<(Entity, &RemoteUnit, Option<&Animator>)>,
    mut bind: Local<Option<(u32, u64, Option<f32>)>>,
) {
    let framed = talk.frame.filter(|_| talk.on);
    let found = framed.and_then(|f| units.iter().find(|(_, u, _)| u.obj_id == f.speaker).map(|unit| (f, unit)));
    let Some((frame, (entity, unit, animator))) = found else {
        if shot.pose.is_some() {
            *shot = QuestShot::default();
        }
        return;
    };
    // the facing is taken once for a speaker and kept (1.2 makes the quaternion again only when it is the identity or the speaker changed)
    let facing = frame.facing.unwrap_or(unit.facing);
    if frame.facing.is_none()
        && let Some(f) = talk.frame.as_mut()
    {
        f.facing = Some(facing);
    }
    // the speaker's `camera` bone at its bind-pose height (kept per speaker and rig), as 1.2 stands it for an actor; 1.5 for a model
    // without a skeleton or before its rig is in
    if let Some(a) = animator
        && bind.is_none_or(|(speaker, rig, _)| speaker != frame.speaker || rig != a.rig.id)
    {
        *bind = Some((frame.speaker, a.rig.id, super::camera::bind_camera_bone(&a.rig)));
    }
    let height = rule::aim_height(bind.filter(|(speaker, ..)| *speaker == frame.speaker).and_then(|(.., h)| h));
    let view = rule::view(&frame.row, unit.pos.to_array(), facing, height);
    let (yaw, pitch) = rule::look_angles(&view);
    shot.pose = Some(Transform {
        translation: cry_to_bevy(Vec3::from_array(view.eye)),
        rotation: Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch),
        scale: Vec3::ONE,
    });
    shot.fov = frame.row.fov;
    shot.hide = frame.row.invisible;
    shot.speaker = Some((entity, unit.pos));
    shot.aim = Vec3::from_array(view.target);
}
